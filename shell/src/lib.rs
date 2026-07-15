//! Browser/WASM leaf for Voxels. The worker owns the renderer, clock, input semantics, and persistence.

#[cfg(any(target_arch = "wasm32", test))]
use voxels_core::CameraState;

#[cfg(any(target_arch = "wasm32", test))]
const INTERACTION_REACH_METRES: f32 = 5.0;
#[cfg(any(target_arch = "wasm32", test))]
const INTERACTION_STREAM_MARGIN_METRES: f32 = 0.7;

/// Canonical chunks intersecting the current view/edit corridor. This bounded secondary interest
/// keeps raycasts, collision, and rendering in agreement below the normal vertical residency span
/// without multiplying the primary cylinder for every direction.
#[cfg(any(target_arch = "wasm32", test))]
fn interaction_stream_interest(camera: &CameraState) -> Vec<voxels_world::ChunkCoord> {
    use std::collections::BTreeSet;
    use voxels_world::{CHUNK_EDGE, ChunkCoord, VOXEL_SIZE_METRES};

    let end = camera.position
        + camera.forward() * (INTERACTION_REACH_METRES + INTERACTION_STREAM_MARGIN_METRES);
    let minimum = camera.position.min(end) - glam::Vec3::splat(INTERACTION_STREAM_MARGIN_METRES);
    let maximum = camera.position.max(end) + glam::Vec3::splat(INTERACTION_STREAM_MARGIN_METRES);
    let chunk_size = CHUNK_EDGE as f32 * VOXEL_SIZE_METRES;
    let minimum = (minimum / chunk_size).floor().as_ivec3();
    let maximum = (maximum / chunk_size).floor().as_ivec3();
    let mut chunks = BTreeSet::new();
    for z in minimum.z..=maximum.z {
        for y in minimum.y..=maximum.y {
            for x in minimum.x..=maximum.x {
                let coord = ChunkCoord::new(x, y, z);
                if coord.is_world_representable() {
                    chunks.insert(coord);
                }
            }
        }
    }
    chunks.into_iter().collect()
}

#[cfg(any(target_arch = "wasm32", test))]
fn camera_persistence_values(camera: &CameraState) -> [f32; 5] {
    [
        camera.position.x,
        camera.position.y,
        camera.position.z,
        camera.yaw,
        camera.pitch,
    ]
}

#[cfg(any(target_arch = "wasm32", test))]
fn camera_from_persisted_values(values: [f32; 5]) -> (CameraState, bool) {
    let camera = CameraState::from_persisted(
        glam::Vec3::new(values[0], values[1], values[2]),
        values[3],
        values[4],
    );
    let canonical = camera_persistence_values(&camera) == values;
    (camera, canonical)
}

#[cfg(target_arch = "wasm32")]
mod persist;
#[cfg(target_arch = "wasm32")]
mod presence_remote;
#[cfg(target_arch = "wasm32")]
pub mod remote;
#[cfg(target_arch = "wasm32")]
mod web {
    use crate::persist::{PersistenceConfig, PersistencePlayer, PersistenceWorld, Store};
    use crate::presence_remote::RemotePresenceClient;
    use crate::remote::{
        RemoteChunkCompletion, RemoteEditEvent, RemoteSurfaceCompletion, RemoteSurfaceTicket,
        RemoteWorldClient, RemoteWorldError,
    };
    use bytemuck::{Pod, Zeroable};
    use glam::{Vec2, Vec3};
    use std::cell::{Cell, RefCell};
    use std::collections::{BTreeMap, BTreeSet, VecDeque};
    use std::rc::Rc;
    use voxels_client_config::{ClientConfig, DaylightConfig};
    use voxels_core::{
        CameraState, EnclosureSample, InputState, ProfileAutomation, ProfileConfig, ProfilePhase,
        VoxelHit, VoxelPhysics, probe_enclosure, raycast_voxels, voxel_segment_is_clear,
    };
    use voxels_render::environment::DaylightPhase;
    use voxels_render::renderer::{
        ChunkActivationReason, LocalLightVisibility, MissionControlConfig, Renderer,
        RendererConfig, RendererFeatureConfig,
    };
    use voxels_render::shadow::DirectionalShadowConfig;
    use voxels_render::ui::LiveStats;
    use voxels_runtime::{
        AuthoritativeEditRevisions, ChunkState, CompletionStatus, FrameBudget, StreamConfig,
        StreamScheduler, SurfaceFocusAction, SurfaceRevisionCache, revision_satisfies,
    };
    use voxels_world::protocol::{
        BrowserUserId, EditAction, MaterialInventory, PlayerId, PlayerIdentity, VoxelFace,
        VoxelMutation,
    };
    use voxels_world::{
        AtmosphereSample, CHUNK_EDGE, CHUNK_VOXEL_BYTES, CINDER_VAULT_PORTAL_COUNT,
        CaveStreamInterest, Chunk, ChunkCoord, EditMap, Material, MeshedChunk, MeshingHalo,
        PortalState, SURFACE_LOD_LEVEL_COUNT, SurfaceLodLevel, SurfaceRegion, SurfaceSample,
        SurfaceTileCoord, VOXEL_SIZE_METRES, VoxelCoord, WorldProductPriority,
        WorldSourceIdentityHash, mesh_chunk,
    };
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;
    use web_sys::{DedicatedWorkerGlobalScope, OffscreenCanvas};

    const FRAME_HISTORY_CAPACITY: usize = 512;
    const SNAPSHOT_SCHEMA_VERSION: f32 = 20.0;
    const INTERACTIVE_SURFACE_LOD_LEVELS: usize = 4;

    #[derive(Clone, Copy, Debug)]
    struct EngineConfig {
        fixed_step_seconds: f32,
        max_steps_per_frame: u32,
        persist_interval_ms: f64,
        max_edit_trackers: usize,
        stream_frame_budget: FrameBudget,
        startup_ready_radius_chunks: i32,
        surface_load_radius_tiles: [i32; SURFACE_LOD_LEVEL_COUNT],
        surface_retain_margin_tiles: i32,
        enclosure_probe_interval_ms: f64,
        enclosure_probe_distance_metres: f32,
    }

    type FrameCallback = Closure<dyn FnMut(f64)>;

    fn resident_material(
        chunks: &BTreeMap<(i32, i32, i32), Chunk>,
        coord: VoxelCoord,
    ) -> Option<Material> {
        let chunk = chunks.get(&coord_key(coord.chunk()))?;
        let [x, y, z] = coord.local();
        Some(chunk.get(x, y, z))
    }

    fn resident_surface_sample(
        chunks: &BTreeMap<(i32, i32, i32), Chunk>,
        x: i32,
        z: i32,
        region: SurfaceRegion,
    ) -> Option<SurfaceSample> {
        let chunk_x = x.div_euclid(CHUNK_EDGE as i32);
        let chunk_z = z.div_euclid(CHUNK_EDGE as i32);
        let local_x = x.rem_euclid(CHUNK_EDGE as i32) as usize;
        let local_z = z.rem_euclid(CHUNK_EDGE as i32) as usize;
        let mut surface = None::<(i32, Material)>;
        let mut water_level = None::<i32>;
        for (&(candidate_x, _, candidate_z), chunk) in chunks {
            if candidate_x != chunk_x || candidate_z != chunk_z {
                continue;
            }
            let origin_y = chunk.coord().world_origin()[1];
            for local_y in 0..CHUNK_EDGE {
                let material = chunk.get(local_x, local_y, local_z);
                let world_y = origin_y + local_y as i32;
                if material.is_collidable() && surface.is_none_or(|(height, _)| world_y > height) {
                    surface = Some((world_y, material));
                }
                if material == Material::Water && water_level.is_none_or(|height| world_y > height)
                {
                    water_level = Some(world_y);
                }
            }
        }
        let (height, material) = surface?;
        Some(SurfaceSample {
            height,
            material,
            water_level,
            region,
            moisture: 0.5,
            temperature: 0.5,
            ridge: 0.0,
            route: None,
        })
    }

    #[derive(Clone, Copy, Default)]
    struct FrameSample {
        interval_ms: f32,
        cpu_ms: f32,
        simulation_ms: f32,
        stream_ms: f32,
        render_ms: f32,
    }

    struct FrameHistory {
        samples: [FrameSample; FRAME_HISTORY_CAPACITY],
        next: usize,
        len: usize,
        dropped: u32,
    }

    impl FrameHistory {
        fn new() -> Self {
            Self {
                samples: [FrameSample::default(); FRAME_HISTORY_CAPACITY],
                next: 0,
                len: 0,
                dropped: 0,
            }
        }

        fn push(&mut self, sample: FrameSample) {
            self.samples[self.next] = sample;
            self.next = (self.next + 1) % FRAME_HISTORY_CAPACITY;
            if self.len < FRAME_HISTORY_CAPACITY {
                self.len += 1;
            } else {
                self.dropped = self.dropped.saturating_add(1);
            }
        }

        fn drain_into(&mut self, values: &mut Vec<f32>) {
            values.push(self.len as f32);
            values.push(self.dropped as f32);
            let first = (self.next + FRAME_HISTORY_CAPACITY - self.len) % FRAME_HISTORY_CAPACITY;
            for offset in 0..self.len {
                let sample = self.samples[(first + offset) % FRAME_HISTORY_CAPACITY];
                values.extend_from_slice(&[
                    sample.interval_ms,
                    sample.cpu_ms,
                    sample.simulation_ms,
                    sample.stream_ms,
                    sample.render_ms,
                ]);
            }
            self.len = 0;
            self.dropped = 0;
        }
    }

    #[derive(Clone, Copy, Debug)]
    struct CanonicalRequirement {
        coord: ChunkCoord,
        revision: u64,
    }

    #[derive(Clone, Copy, Debug)]
    struct SurfaceRequirement {
        coord: SurfaceTileCoord,
        revision: u64,
    }

    #[derive(Default)]
    struct EditRequirements {
        canonical: Vec<CanonicalRequirement>,
        surface: Vec<SurfaceRequirement>,
    }

    struct EditTracker {
        target: VoxelCoord,
        started_ms: f64,
        requirements: EditRequirements,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Pod, Zeroable)]
    struct InputRecord {
        kind: u8,
        code: u8,
        buttons: u16,
        x: f32,
        y: f32,
        dx: f32,
        dy: f32,
        flags: u32,
    }

    const INPUT_RECORD_SIZE: usize = size_of::<InputRecord>();
    const _: () = assert!(INPUT_RECORD_SIZE == 24);
    const KIND_POINTER_MOVE: u8 = 1;
    const KIND_POINTER_DOWN: u8 = 0;
    const KIND_WHEEL: u8 = 2;
    const KIND_KEY_DOWN: u8 = 4;
    const KIND_KEY_UP: u8 = 5;
    const KIND_CANCEL: u8 = 6;

    fn log_gpu_error(message: &str) {
        web_sys::console::error_1(&JsValue::from_str(message));
    }

    struct Engine {
        config: EngineConfig,
        renderer: RefCell<Renderer>,
        camera: RefCell<CameraState>,
        input: RefCell<InputState>,
        remote: RemoteWorldClient,
        presence: RemotePresenceClient,
        source_identity_hash: WorldSourceIdentityHash,
        remote_environment: (AtmosphereSample, SurfaceRegion),
        edits: RefCell<EditMap>,
        inventory: Cell<MaterialInventory>,
        edit_revisions: RefCell<AuthoritativeEditRevisions>,
        scheduler: RefCell<StreamScheduler>,
        chunks: RefCell<BTreeMap<(i32, i32, i32), Chunk>>,
        chunk_halos: RefCell<BTreeMap<(i32, i32, i32), MeshingHalo>>,
        pending_meshes: RefCell<BTreeMap<(i32, i32, i32), MeshedChunk>>,
        surface_focus: Cell<Option<[SurfaceTileCoord; SURFACE_LOD_LEVEL_COUNT]>>,
        surface_active_focus: Cell<Option<[SurfaceTileCoord; SURFACE_LOD_LEVEL_COUNT]>>,
        surface_resident: RefCell<BTreeSet<SurfaceTileCoord>>,
        surface_revisions: RefCell<SurfaceRevisionCache>,
        surface_accepted_edit_revisions: RefCell<BTreeMap<SurfaceTileCoord, u64>>,
        surface_queue: RefCell<VecDeque<SurfaceTileCoord>>,
        surface_in_flight: RefCell<BTreeSet<SurfaceTileCoord>>,
        surface_dirty: RefCell<BTreeSet<SurfaceTileCoord>>,
        fine_initialized: Cell<bool>,
        all_lods_ready: Cell<bool>,
        interactive_lods_ready: Cell<bool>,
        full_lods_initialized: Cell<bool>,
        startup_ready: Cell<bool>,
        store: RefCell<Store>,
        scope: DedicatedWorkerGlobalScope,
        callback: RefCell<Option<FrameCallback>>,
        frame_id: Cell<i32>,
        last_time: Cell<f64>,
        simulation_accumulator: Cell<f32>,
        frame_milliseconds: Cell<f32>,
        cpu_milliseconds: Cell<f32>,
        simulation_milliseconds: Cell<f32>,
        stream_milliseconds: Cell<f32>,
        render_milliseconds: Cell<f32>,
        frame_history: RefCell<FrameHistory>,
        edit_trackers: RefCell<VecDeque<EditTracker>>,
        edit_last_ms: Cell<f32>,
        enclosure: Cell<EnclosureSample>,
        last_enclosure_probe: Cell<f64>,
        enclosure_probe_microseconds: Cell<f32>,
        cinder_portal_state: Cell<PortalState>,
        cinder_portal_revision: Cell<u32>,
        cinder_stream_interest: Cell<CaveStreamInterest>,
        radial_active_chunks: RefCell<BTreeSet<(i32, i32, i32)>>,
        portal_active_chunks: RefCell<BTreeSet<(i32, i32, i32)>>,
        interaction_active_chunks: RefCell<BTreeSet<(i32, i32, i32)>>,
        profile: RefCell<ProfileAutomation>,
        profile_tracked_high: Cell<usize>,
        profile_surface_high: Cell<usize>,
        profile_pending_high: Cell<usize>,
        profile_pending_mesh_high: Cell<usize>,
        profile_arena_capacity_high: Cell<u64>,
        profile_wasm_high: Cell<u64>,
        profile_start_evictions: Cell<u64>,
        last_persist: Cell<f64>,
        last_persisted_camera: Cell<Option<[f32; 5]>>,
        stopped: Cell<bool>,
    }

    impl Engine {
        fn start(self: &Rc<Self>) -> Result<(), JsValue> {
            let weak = Rc::downgrade(self);
            let callback: FrameCallback = Closure::wrap(Box::new(move |time: f64| {
                if let Some(engine) = weak.upgrade() {
                    engine.frame(time);
                }
            }));
            *self.callback.borrow_mut() = Some(callback);
            self.request_frame()
        }

        fn request_frame(&self) -> Result<(), JsValue> {
            if self.stopped.get() {
                return Ok(());
            }
            let callback = self.callback.borrow();
            let callback = callback
                .as_ref()
                .ok_or_else(|| JsValue::from_str("animation callback is unavailable"))?;
            let id = self
                .scope
                .request_animation_frame(callback.as_ref().unchecked_ref())?;
            self.frame_id.set(id);
            Ok(())
        }

        fn source_identity_hash(&self) -> WorldSourceIdentityHash {
            self.remote
                .source_identity_hash()
                .unwrap_or(self.source_identity_hash)
        }

        fn cached_surface_sample(&self, x: i32, z: i32) -> Result<SurfaceSample, String> {
            resident_surface_sample(&self.chunks.borrow(), x, z, self.remote_environment.1)
                .ok_or_else(|| "native surface column is not resident yet".to_owned())
        }

        fn start_profile(&self, profile_id: u32) -> bool {
            match profile_id {
                1 => self.start_stream_profile(),
                _ => {
                    log_gpu_error("only the provider-neutral streaming profile is available");
                    false
                }
            }
        }

        fn start_stream_profile(&self) -> bool {
            self.input.borrow_mut().clear();
            let position = self.camera.borrow().position;
            self.profile.borrow_mut().start(position);
            self.profile_tracked_high.set(0);
            self.profile_surface_high.set(0);
            self.profile_pending_high.set(0);
            self.profile_pending_mesh_high.set(0);
            self.profile_arena_capacity_high.set(0);
            self.profile_wasm_high.set(wasm_committed_bytes());
            self.profile_start_evictions
                .set(self.scheduler.borrow().diagnostics().total_evictions);
            true
        }

        fn frame(&self, time: f64) {
            let performance = self.scope.performance();
            let cpu_start = performance_now(performance.as_ref());
            self.apply_server_edits();
            let last = self.last_time.replace(time);
            let dt = if last <= 0.0 {
                1.0 / 60.0
            } else {
                ((time - last).max(0.0) / 1000.0) as f32
            };
            let frame_ms = dt * 1_000.0;
            self.frame_milliseconds
                .set(smoothed_ms(self.frame_milliseconds.get(), frame_ms));
            let simulation_start = performance_now(performance.as_ref());
            let mut camera = self.camera.borrow_mut();
            let profiling = self.profile.borrow().running();
            let chunks = self.chunks.borrow();
            let mut accumulator = (self.simulation_accumulator.get() + dt.min(0.1))
                .min(self.config.fixed_step_seconds * self.config.max_steps_per_frame as f32);
            if !self.startup_ready.get() {
                accumulator = 0.0;
            }
            let mut steps = 0;
            while self.startup_ready.get()
                && accumulator >= self.config.fixed_step_seconds
                && steps < self.config.max_steps_per_frame
            {
                if profiling {
                    self.profile.borrow_mut().advance_fixed_step();
                } else {
                    camera.update(
                        &self.input.borrow(),
                        self.config.fixed_step_seconds,
                        VOXEL_SIZE_METRES,
                        |x, y, z| {
                            let coord = VoxelCoord::new(x, y, z);
                            // Missing resident data is a conservative simulation boundary. Source
                            // requests are admitted by the stream scheduler, never from callbacks.
                            let material =
                                resident_material(&chunks, coord).unwrap_or(Material::Stone);
                            VoxelPhysics {
                                collidable: material.is_collidable(),
                                fluid: material.is_fluid(),
                            }
                        },
                    );
                }
                accumulator -= self.config.fixed_step_seconds;
                steps += 1;
            }
            self.simulation_accumulator.set(accumulator);
            drop(chunks);
            if profiling && let Some(pose) = self.profile.borrow().pose() {
                let voxel_x = (pose.position_xz.x / VOXEL_SIZE_METRES).floor() as i32;
                let voxel_z = (pose.position_xz.y / VOXEL_SIZE_METRES).floor() as i32;
                match self.cached_surface_sample(voxel_x, voxel_z) {
                    Ok(surface) => {
                        let top = surface
                            .water_level
                            .unwrap_or(surface.height)
                            .max(surface.height);
                        let position = glam::Vec3::new(
                            pose.position_xz.x,
                            (top + 1) as f32 * VOXEL_SIZE_METRES
                                + voxels_core::PLAYER_EYE_HEIGHT_METRES
                                + 0.8,
                            pose.position_xz.y,
                        );
                        *camera = CameraState::spawn(position);
                        camera.yaw = pose.yaw;
                        camera.pitch = pose.pitch;
                    }
                    Err(error) => {
                        log_gpu_error(&format!("streaming profile surface probe failed: {error}"))
                    }
                }
            }
            if time - self.last_enclosure_probe.get() >= self.config.enclosure_probe_interval_ms {
                let probe_start = performance_now(performance.as_ref());
                let eye_voxel = (camera.position / VOXEL_SIZE_METRES).floor().as_ivec3();
                let underground = self
                    .cached_surface_sample(eye_voxel.x, eye_voxel.z)
                    .map(|surface| eye_voxel.y + 3 < surface.height);
                match underground {
                    Ok(true) => {
                        let chunks = self.chunks.borrow();
                        self.enclosure.set(probe_enclosure(
                            camera.position,
                            self.config.enclosure_probe_distance_metres,
                            VOXEL_SIZE_METRES,
                            |x, y, z| {
                                resident_material(&chunks, VoxelCoord::new(x, y, z))
                                    .unwrap_or(Material::Stone)
                                    .occludes_ambient()
                            },
                        ));
                    }
                    Ok(false) => self.enclosure.set(EnclosureSample::OPEN),
                    Err(error) => web_sys::console::warn_1(&JsValue::from_str(&format!(
                        "enclosure surface probe deferred; retaining prior sample: {error}"
                    ))),
                }
                self.last_enclosure_probe.set(time);
                self.enclosure_probe_microseconds
                    .set(((performance_now(performance.as_ref()) - probe_start) * 1_000.0) as f32);
            }
            let simulation_ms = (performance_now(performance.as_ref()) - simulation_start) as f32;
            self.simulation_milliseconds.set(smoothed_ms(
                self.simulation_milliseconds.get(),
                simulation_ms,
            ));
            let stream_start = performance_now(performance.as_ref());
            self.stream_world(&camera);
            if let Some(opened) = self.remote.world_opened() {
                self.presence.ensure_session(&opened, time);
            }
            // The streaming profiler moves a synthetic camera outside gameplay simulation. It may
            // stream/render that route, but it must never update authoritative player position or
            // gain edit reach from benchmark-only motion.
            let remote_avatars = self.presence.update(&camera, time, dt, !profiling);
            if let Some(error) = self.presence.take_error() {
                log_gpu_error(&format!("player presence: {error}"));
            }
            let stream_ms = (performance_now(performance.as_ref()) - stream_start) as f32;
            self.stream_milliseconds
                .set(smoothed_ms(self.stream_milliseconds.get(), stream_ms));
            let target = self.raycast_target(&camera).map(|hit| hit.voxel);
            let mut renderer = self.renderer.borrow_mut();
            renderer.set_remote_avatars(&remote_avatars);
            renderer.set_target_voxel(target);
            let (atmosphere, region) = self.remote_environment;
            renderer.set_atmosphere(atmosphere, region);
            let enclosure = self.enclosure.get();
            renderer.set_enclosure(enclosure);
            renderer.set_route_status("NATIVE WORLD", 0);
            let stream = self.scheduler.borrow().diagnostics();
            let render = renderer.diagnostics();
            let lod_tiles = self.surface_lod_counts();
            let fine_coverage_ready = stream.generation.queued == 0
                && stream.generation.in_flight == 0
                && stream.meshing.queued == 0
                && stream.meshing.in_flight == 0
                && stream.upload.queued == 0
                && stream.upload.in_flight == 0;
            if fine_coverage_ready {
                self.fine_initialized.set(true);
            }
            let interactive_lods_ready =
                fine_coverage_ready
                    && !self.surface_dirty.borrow().iter().any(|coord| {
                        usize::from(coord.level.index()) < INTERACTIVE_SURFACE_LOD_LEVELS
                    })
                    && !self.surface_in_flight.borrow().iter().any(|coord| {
                        usize::from(coord.level.index()) < INTERACTIVE_SURFACE_LOD_LEVELS
                    })
                    && self.surface_coverage_current_through(INTERACTIVE_SURFACE_LOD_LEVELS);
            let all_lods_ready = interactive_lods_ready
                && self.surface_queue.borrow().is_empty()
                && self.surface_in_flight.borrow().is_empty()
                && self.surface_dirty.borrow().is_empty()
                && self.surface_coverage_current();
            self.all_lods_ready.set(all_lods_ready);
            self.interactive_lods_ready.set(interactive_lods_ready);
            debug_assert!(
                !all_lods_ready || self.surface_coverage_current(),
                "surface coverage became ready with missing or stale revisions"
            );
            renderer.set_lod_coverage_ready(self.fine_initialized.get(), all_lods_ready);
            if all_lods_ready {
                let voxel_x = (camera.position.x / VOXEL_SIZE_METRES).floor() as i32;
                let voxel_z = (camera.position.z / VOXEL_SIZE_METRES).floor() as i32;
                renderer.set_geometric_lod_focus(voxel_x, voxel_z, SURFACE_LOD_LEVEL_COUNT);
                self.surface_active_focus.set(self.surface_focus.get());
                self.full_lods_initialized.set(true);
            } else if interactive_lods_ready && !self.full_lods_initialized.get() {
                let voxel_x = (camera.position.x / VOXEL_SIZE_METRES).floor() as i32;
                let voxel_z = (camera.position.z / VOXEL_SIZE_METRES).floor() as i32;
                renderer.set_geometric_lod_focus(voxel_x, voxel_z, INTERACTIVE_SURFACE_LOD_LEVELS);
                self.surface_active_focus.set(self.surface_focus.get());
            }
            let render_start = performance_now(performance.as_ref());
            let chunks = self.chunks.borrow();
            let submitted = renderer.render(
                dt,
                &camera,
                LiveStats {
                    frames_per_second: if self.frame_milliseconds.get() > 0.0 {
                        1_000.0 / self.frame_milliseconds.get()
                    } else {
                        0.0
                    },
                    frame_ms: self.frame_milliseconds.get(),
                    cpu_ms: self.cpu_milliseconds.get(),
                    gpu_ms: render.gpu_total_ms,
                    gpu_ambient_occlusion_ms: render.gpu_ambient_occlusion_ms,
                    resident_chunks: usize_to_u32(
                        stream.resident + self.surface_resident.borrow().len(),
                    ),
                    visible_chunks: render.visible_chunks,
                    quads: render.quads,
                    water_quads: render.water_quads,
                    draw_calls: render.draw_calls,
                    water_draw_calls: render.water_draw_calls,
                    shadow_draw_calls: render.shadow_draw_calls,
                    shadow_cascades: render.shadow_cascades,
                    load_p95_frames: stream.initial_residency_latency.p95_frames,
                    load_max_frames: stream.initial_residency_latency.max_frames,
                    remesh_p95_frames: stream.remesh_latency.p95_frames,
                    remesh_max_frames: stream.remesh_latency.max_frames,
                    edit_last_ms: self.edit_last_ms.get(),
                    edit_in_flight: usize_to_u32(self.edit_trackers.borrow().len()),
                    lod_tiles,
                    pending_jobs: usize_to_u32(
                        stream.generation.queued
                            + stream.meshing.queued
                            + stream.upload.queued
                            + self.surface_queue.borrow().len()
                            + self.surface_dirty.borrow().len(),
                    ),
                    core_gpu_bytes: render.core_gpu_bytes,
                    water_immersion: camera.fluid_state().immersion,
                    eye_depth_metres: camera.fluid_state().eye_depth_metres,
                    eyes_submerged: camera.fluid_state().eyes_submerged,
                    swimming: camera.fluid_state().swimming,
                    local_light_candidates: render.local_light_candidates,
                    active_local_lights: render.active_local_lights,
                    occluded_local_lights: render.occluded_local_lights,
                    portal_rejected_local_lights: render.portal_rejected_local_lights,
                    open_cinder_portals: self
                        .cinder_portal_state
                        .get()
                        .open_count(CINDER_VAULT_PORTAL_COUNT),
                    cinder_portal_revision: self.cinder_portal_revision.get(),
                    stream_interest_requested: usize_to_u32(stream.secondary_interest_requested),
                    stream_interest_desired: usize_to_u32(stream.secondary_interest_desired),
                    stream_interest_truncated: usize_to_u32(stream.secondary_interest_truncated),
                    portal_active_chunks: usize_to_u32(self.portal_active_chunks.borrow().len()),
                },
                |position, _maximum_geodesic_metres| {
                    if voxel_segment_is_clear(
                        camera.position,
                        Vec3::from_array(position),
                        VOXEL_SIZE_METRES,
                        |x, y, z| {
                            let coord = VoxelCoord::new(x, y, z);
                            let material =
                                resident_material(&chunks, coord).unwrap_or(Material::Stone);
                            material.occludes_ambient() && material.emission().is_none()
                        },
                    ) {
                        LocalLightVisibility::Visible
                    } else {
                        LocalLightVisibility::Occluded
                    }
                },
            );
            if submitted
                && self
                    .scheduler
                    .borrow()
                    .vicinity_readiness(self.config.startup_ready_radius_chunks)
                    .is_ready()
            {
                self.startup_ready.set(true);
            }
            drop(chunks);
            let rendered = renderer.diagnostics();
            drop(renderer);
            self.update_edit_convergence(time, submitted);
            if self.profile.borrow().phase() != ProfilePhase::Idle {
                let pending = stream.generation.queued
                    + stream.meshing.queued
                    + stream.upload.queued
                    + self.surface_queue.borrow().len()
                    + self.surface_dirty.borrow().len();
                self.profile_tracked_high
                    .set(self.profile_tracked_high.get().max(stream.tracked));
                self.profile_surface_high.set(
                    self.profile_surface_high
                        .get()
                        .max(self.surface_resident.borrow().len()),
                );
                self.profile_pending_high
                    .set(self.profile_pending_high.get().max(pending));
                self.profile_pending_mesh_high.set(
                    self.profile_pending_mesh_high
                        .get()
                        .max(self.pending_meshes.borrow().len()),
                );
                self.profile_arena_capacity_high.set(
                    self.profile_arena_capacity_high
                        .get()
                        .max(rendered.arena_capacity_bytes),
                );
                self.profile_wasm_high
                    .set(self.profile_wasm_high.get().max(wasm_committed_bytes()));
                if self.profile.borrow().phase() == ProfilePhase::Drain
                    && all_lods_ready
                    && submitted
                {
                    self.profile.borrow_mut().complete_drain();
                }
            }
            let render_ms = (performance_now(performance.as_ref()) - render_start) as f32;
            self.render_milliseconds
                .set(smoothed_ms(self.render_milliseconds.get(), render_ms));
            if time - self.last_persist.get() >= self.config.persist_interval_ms {
                self.persist_camera_if_changed(&camera);
                self.last_persist.set(time);
            }
            let cpu_ms = (performance_now(performance.as_ref()) - cpu_start) as f32;
            self.cpu_milliseconds
                .set(smoothed_ms(self.cpu_milliseconds.get(), cpu_ms));
            self.frame_history.borrow_mut().push(FrameSample {
                interval_ms: frame_ms,
                cpu_ms,
                simulation_ms,
                stream_ms,
                render_ms,
            });
            if let Err(error) = self.request_frame() {
                web_sys::console::error_1(&error);
                self.stopped.set(true);
            }
        }

        fn stream_world(&self, camera: &CameraState) {
            self.drain_remote_generation();
            let focus = world_to_chunk(camera.position);
            let interest = crate::interaction_stream_interest(camera);
            let (focus_changed, work) = {
                let mut scheduler = self.scheduler.borrow_mut();
                let changed = scheduler.update_focus_with_interest(focus, &interest);
                (
                    changed,
                    scheduler.schedule_frame(self.config.stream_frame_budget),
                )
            };
            let mut uploaded = false;

            let interest_keys: BTreeSet<_> = interest.iter().copied().map(coord_key).collect();
            let mut collision_generation = Vec::new();
            let mut background_generation = Vec::new();
            for ticket in work.generation {
                let dx = i64::from(ticket.coord.x) - i64::from(focus.x);
                let dz = i64::from(ticket.coord.z) - i64::from(focus.z);
                let radius = i64::from(self.config.startup_ready_radius_chunks);
                if interest_keys.contains(&coord_key(ticket.coord))
                    || (!self.startup_ready.get() && dx * dx + dz * dz <= radius * radius)
                {
                    collision_generation.push(ticket);
                } else {
                    background_generation.push(ticket);
                }
            }
            self.submit_generation_batch(
                WorldProductPriority::CollisionCritical,
                collision_generation,
            );
            self.submit_generation_batch(WorldProductPriority::VisibleChunk, background_generation);
            {
                let chunks = self.chunks.borrow();
                let halos = self.chunk_halos.borrow();
                for ticket in work.meshing {
                    let Some(chunk) = chunks.get(&coord_key(ticket.coord)) else {
                        continue;
                    };
                    let Some(halo) = halos.get(&coord_key(ticket.coord)) else {
                        let _ = self.scheduler.borrow_mut().retry(ticket);
                        continue;
                    };
                    let mut halo_contract_valid = true;
                    let mesh = mesh_chunk(chunk, |x, y, z| {
                        let Some(material) = halo.sample_world(x, y, z) else {
                            halo_contract_valid = false;
                            return Material::Stone;
                        };
                        material
                    });
                    if !halo_contract_valid {
                        let _ = self.scheduler.borrow_mut().retry(ticket);
                        web_sys::console::error_1(&JsValue::from_str(
                            "world source meshing halo omitted a required shell coordinate",
                        ));
                        continue;
                    }
                    self.pending_meshes
                        .borrow_mut()
                        .insert(coord_key(ticket.coord), mesh);
                    let _ = self.scheduler.borrow_mut().complete(ticket);
                }
            }
            for ticket in work.upload {
                let mesh = self
                    .pending_meshes
                    .borrow_mut()
                    .remove(&coord_key(ticket.coord));
                let Some(mesh) = mesh else {
                    continue;
                };
                if self.renderer.borrow_mut().upload_chunk(ticket.coord, &mesh) {
                    let _ = self.scheduler.borrow_mut().complete(ticket);
                    uploaded = true;
                } else {
                    self.pending_meshes
                        .borrow_mut()
                        .insert(coord_key(ticket.coord), mesh);
                    let _ = self.scheduler.borrow_mut().retry(ticket);
                    web_sys::console::error_1(&JsValue::from_str(
                        "voxel mesh arena allocation failed; upload requeued",
                    ));
                }
            }
            let evictions = self.scheduler.borrow_mut().drain_evictions();
            let evicted = !evictions.is_empty();
            if !evictions.is_empty() {
                let mut chunks = self.chunks.borrow_mut();
                let mut halos = self.chunk_halos.borrow_mut();
                let mut pending = self.pending_meshes.borrow_mut();
                let mut renderer = self.renderer.borrow_mut();
                for eviction in evictions {
                    chunks.remove(&coord_key(eviction.coord));
                    halos.remove(&coord_key(eviction.coord));
                    pending.remove(&coord_key(eviction.coord));
                    renderer.remove_chunk(eviction.coord);
                }
            }
            if focus_changed || uploaded || evicted {
                self.reconcile_chunk_activation(focus, &interest);
            }
            self.stream_surface_lods(camera.position);
        }

        fn submit_generation_batch(
            &self,
            priority: WorldProductPriority,
            tickets: Vec<voxels_runtime::WorkTicket>,
        ) {
            if tickets.is_empty() {
                return;
            }
            if let Err(error) = self.remote.submit_chunk_batch(priority, tickets.clone()) {
                for ticket in tickets {
                    let _ = self.scheduler.borrow_mut().retry(ticket);
                }
                if !matches!(
                    error,
                    RemoteWorldError::Backpressured
                        | RemoteWorldError::RequestWindowFull
                        | RemoteWorldError::NotOpen
                ) {
                    log_gpu_error(&format!("native world request failed: {error}"));
                }
            }
        }

        fn drain_remote_generation(&self) {
            for completion in self.remote.drain_completions() {
                self.accept_remote_completion(completion);
            }
            for completion in self.remote.drain_surface_completions() {
                self.accept_remote_surface_completion(completion);
            }
        }

        fn accept_remote_completion(&self, completion: RemoteChunkCompletion) {
            let Ok(result) = completion.result else {
                for ticket in completion.tickets {
                    let _ = self.scheduler.borrow_mut().retry(ticket);
                }
                return;
            };
            if result.source_identity_hash != self.source_identity_hash() {
                for ticket in completion.tickets {
                    let _ = self.scheduler.borrow_mut().retry(ticket);
                }
                log_gpu_error("native world response identity changed");
                return;
            }
            let mut items = result.items;
            for ticket in completion.tickets {
                let Some(index) = items.iter().position(|item| item.coord == ticket.coord) else {
                    let _ = self.scheduler.borrow_mut().retry(ticket);
                    continue;
                };
                let item = items.remove(index);
                match item.result {
                    Ok(snapshot) => {
                        self.accept_generated_chunk(ticket, item.edit_revision, snapshot)
                    }
                    Err(voxels_world::WorldSourceError::SourceCoverageUnavailable) => {
                        // This source owns finite coverage. Leaving the exact scheduler capability
                        // in flight forms a conservative collision boundary without retry thrash;
                        // focus eviction releases it normally.
                        log_gpu_error(&format!(
                            "native world has no coverage for chunk {:?}",
                            ticket.coord
                        ));
                    }
                    Err(error) => {
                        let _ = self.scheduler.borrow_mut().retry(ticket);
                        log_gpu_error(&format!(
                            "native world could not generate chunk {:?}: {error}",
                            ticket.coord
                        ));
                    }
                }
            }
        }

        fn accept_remote_surface_completion(&self, completion: RemoteSurfaceCompletion) {
            for ticket in &completion.tickets {
                self.surface_in_flight.borrow_mut().remove(&ticket.coord);
            }
            let Ok(result) = completion.result else {
                for ticket in completion.tickets {
                    self.enqueue_surface_front(ticket.coord);
                }
                return;
            };
            if result.source_identity_hash != self.source_identity_hash() {
                log_gpu_error("world surface response identity changed");
                return;
            }
            let mut items = result.items;
            for ticket in completion.tickets {
                let Some(index) = items.iter().position(|item| item.coord == ticket.coord) else {
                    self.enqueue_surface_front(ticket.coord);
                    continue;
                };
                let item = items.remove(index);
                let edit_revision = item.edit_revision;
                let snapshot = match item.result {
                    Ok(snapshot) => snapshot,
                    Err(voxels_world::WorldSourceError::SourceCoverageUnavailable) => continue,
                    Err(error) => {
                        log_gpu_error(&format!(
                            "world service could not generate surface tile {:?}: {error}",
                            ticket.coord
                        ));
                        self.enqueue_surface_front(ticket.coord);
                        continue;
                    }
                };
                let server_floor = self.edit_revisions.borrow().surface_floor(ticket.coord);
                if !revision_satisfies(edit_revision, server_floor)
                    || !self
                        .surface_revisions
                        .borrow()
                        .accepts(ticket.coord, ticket.revision)
                {
                    self.enqueue_surface_front(ticket.coord);
                    continue;
                }
                if self
                    .renderer
                    .borrow_mut()
                    .upload_surface_tile_meshes(&snapshot.terrain, &snapshot.water)
                {
                    self.surface_resident.borrow_mut().insert(ticket.coord);
                    let committed = self
                        .surface_revisions
                        .borrow_mut()
                        .commit(ticket.coord, ticket.revision);
                    debug_assert!(committed, "uploaded remote surface revision became stale");
                    self.surface_accepted_edit_revisions
                        .borrow_mut()
                        .insert(ticket.coord, edit_revision);
                    self.surface_dirty.borrow_mut().remove(&ticket.coord);
                } else {
                    self.enqueue_surface_front(ticket.coord);
                }
            }
        }

        fn enqueue_surface_front(&self, coord: SurfaceTileCoord) {
            if self.surface_in_flight.borrow().contains(&coord)
                || self.surface_queue.borrow().contains(&coord)
            {
                return;
            }
            if usize::from(coord.level.index()) < INTERACTIVE_SURFACE_LOD_LEVELS {
                self.surface_queue.borrow_mut().push_front(coord);
            } else {
                self.surface_queue.borrow_mut().push_back(coord);
            }
        }

        fn accept_generated_chunk(
            &self,
            ticket: voxels_runtime::WorkTicket,
            edit_revision: u64,
            snapshot: voxels_world::ChunkSnapshot,
        ) {
            if snapshot.source_identity_hash != self.source_identity_hash()
                || snapshot.chunk.coord() != ticket.coord
                || snapshot.meshing_halo.coord() != ticket.coord
            {
                let _ = self.scheduler.borrow_mut().retry(ticket);
                return;
            }
            let server_floor = self.edit_revisions.borrow().chunk_floor(ticket.coord);
            if !revision_satisfies(edit_revision, server_floor) {
                let _ = self.scheduler.borrow_mut().retry(ticket);
                return;
            }
            // Network completions can arrive after focus/edit invalidation. The scheduler
            // capability is the admission check; stale bytes never attach to a newer revision.
            if self.scheduler.borrow_mut().complete(ticket) != CompletionStatus::Accepted {
                return;
            }
            self.chunks
                .borrow_mut()
                .insert(coord_key(ticket.coord), snapshot.chunk);
            self.chunk_halos
                .borrow_mut()
                .insert(coord_key(ticket.coord), snapshot.meshing_halo);
        }

        fn reconcile_chunk_activation(&self, focus: ChunkCoord, interest: &[ChunkCoord]) {
            let scheduler = self.scheduler.borrow();
            let config = scheduler.config();
            let mut radial = BTreeSet::new();
            for dz in -config.load_radius_chunks..=config.load_radius_chunks {
                for dx in -config.load_radius_chunks..=config.load_radius_chunks {
                    if i64::from(dx) * i64::from(dx) + i64::from(dz) * i64::from(dz)
                        > i64::from(config.load_radius_chunks)
                            * i64::from(config.load_radius_chunks)
                    {
                        continue;
                    }
                    let Some(x) = focus.x.checked_add(dx) else {
                        continue;
                    };
                    let Some(z) = focus.z.checked_add(dz) else {
                        continue;
                    };
                    let column: Vec<_> = (-config.vertical_radius_chunks
                        ..=config.vertical_radius_chunks)
                        .filter_map(|dy| focus.y.checked_add(dy).map(|y| ChunkCoord::new(x, y, z)))
                        .collect();
                    if column.iter().all(|coord| {
                        scheduler.status(*coord).is_some_and(|status| {
                            status.desired && status.state == ChunkState::Resident
                        })
                    }) {
                        radial.extend(column.into_iter().map(coord_key));
                    }
                }
            }
            // Preserve the old radial reason for retained resident meshes until the scheduler
            // actually evicts them. This carries visible coverage across small focus moves while
            // new columns become atomically ready, matching the retention hysteresis contract.
            for key in self.radial_active_chunks.borrow().iter().copied() {
                if scheduler
                    .status(ChunkCoord::new(key.0, key.1, key.2))
                    .is_some()
                {
                    radial.insert(key);
                }
            }

            let mut interaction_columns = BTreeMap::<(i32, i32), Vec<ChunkCoord>>::new();
            for coord in interest {
                if scheduler
                    .status(*coord)
                    .is_some_and(|status| status.desired)
                {
                    interaction_columns
                        .entry((coord.x, coord.z))
                        .or_default()
                        .push(*coord);
                }
            }
            let mut interaction = BTreeSet::new();
            for coords in interaction_columns.values() {
                if coords.iter().all(|coord| {
                    scheduler
                        .status(*coord)
                        .is_some_and(|status| status.state == ChunkState::Resident)
                }) {
                    interaction.extend(coords.iter().copied().map(coord_key));
                }
            }
            drop(scheduler);
            self.reconcile_activation_reason(
                &self.radial_active_chunks,
                radial,
                ChunkActivationReason::Radial,
            );
            self.reconcile_activation_reason(
                &self.interaction_active_chunks,
                interaction,
                ChunkActivationReason::Interaction,
            );
        }

        fn reconcile_activation_reason(
            &self,
            current: &RefCell<BTreeSet<(i32, i32, i32)>>,
            next: BTreeSet<(i32, i32, i32)>,
            reason: ChunkActivationReason,
        ) {
            let mut current = current.borrow_mut();
            let removed: Vec<_> = current.difference(&next).copied().collect();
            let added: Vec<_> = next.difference(&current).copied().collect();
            *current = next;
            drop(current);
            if removed.is_empty() && added.is_empty() {
                return;
            }
            let mut renderer = self.renderer.borrow_mut();
            for (x, y, z) in removed {
                renderer.set_chunk_activation(ChunkCoord::new(x, y, z), reason, false);
            }
            for (x, y, z) in added {
                renderer.set_chunk_activation(ChunkCoord::new(x, y, z), reason, true);
            }
        }

        fn surface_lod_counts(&self) -> [u32; SURFACE_LOD_LEVEL_COUNT] {
            let mut counts = [0u32; SURFACE_LOD_LEVEL_COUNT];
            for coord in self.surface_resident.borrow().iter() {
                let count = &mut counts[coord.level.index() as usize];
                *count = count.saturating_add(1);
            }
            counts
        }

        fn stream_surface_lods(&self, position: glam::Vec3) {
            let focus = std::array::from_fn(|index| {
                world_to_surface_tile(position, SurfaceLodLevel::ALL[index])
            });
            if self.surface_focus.get() != Some(focus) {
                self.surface_focus.set(Some(focus));
                let mut desired = BTreeSet::new();
                for (index, level) in SurfaceLodLevel::ALL.into_iter().enumerate() {
                    let radius = self.config.surface_load_radius_tiles[index];
                    let level_focus = focus[index];
                    for dz in -radius..=radius {
                        for dx in -radius..=radius {
                            let coord = SurfaceTileCoord::new(
                                level,
                                level_focus.x + dx,
                                level_focus.z + dz,
                            );
                            if coord.is_world_representable() {
                                desired.insert(coord);
                            }
                        }
                    }
                }
                let evicted: Vec<_> = self
                    .surface_resident
                    .borrow()
                    .iter()
                    .copied()
                    .filter(|coord| {
                        let index = coord.level.index() as usize;
                        let retain = self.config.surface_load_radius_tiles[index]
                            + self.config.surface_retain_margin_tiles;
                        let dx = coord.x - focus[index].x;
                        let dz = coord.z - focus[index].z;
                        let outside_pending = dx.abs().max(dz.abs()) > retain;
                        let outside_active = self.surface_active_focus.get().is_none_or(|active| {
                            let dx = coord.x - active[index].x;
                            let dz = coord.z - active[index].z;
                            dx.abs().max(dz.abs())
                                > self.config.surface_load_radius_tiles[index]
                                    + self.config.surface_retain_margin_tiles
                        });
                        outside_pending && outside_active
                    })
                    .collect();
                if !evicted.is_empty() {
                    let mut resident = self.surface_resident.borrow_mut();
                    let mut revisions = self.surface_revisions.borrow_mut();
                    let mut accepted = self.surface_accepted_edit_revisions.borrow_mut();
                    let mut dirty = self.surface_dirty.borrow_mut();
                    let mut renderer = self.renderer.borrow_mut();
                    for coord in evicted {
                        resident.remove(&coord);
                        revisions.evict(coord);
                        accepted.remove(&coord);
                        dirty.remove(&coord);
                        renderer.remove_surface_tile(coord);
                    }
                }

                // Edits may have dirtied a tile just before a focus jump. Keep replacement work only
                // while the tile is still resident or belongs to the new desired set; a future load
                // samples the authoritative edit map and does not need a stale dirty marker.
                {
                    let resident = self.surface_resident.borrow();
                    self.surface_dirty
                        .borrow_mut()
                        .retain(|coord| resident.contains(coord) || desired.contains(coord));
                }

                let resident = self.surface_resident.borrow();
                let mut revisions = self.surface_revisions.borrow_mut();
                let mut dirty = self.surface_dirty.borrow_mut();
                revisions.retain(|coord| resident.contains(&coord) || desired.contains(&coord));
                let mut candidates = Vec::new();
                for coord in desired {
                    match revisions.prepare_focus(coord) {
                        SurfaceFocusAction::Load { .. } => {
                            debug_assert!(!resident.contains(&coord));
                            candidates.push(coord);
                        }
                        SurfaceFocusAction::Replace { .. } => {
                            debug_assert!(resident.contains(&coord));
                            dirty.insert(coord);
                        }
                        SurfaceFocusAction::Current { .. } => {
                            debug_assert!(resident.contains(&coord));
                        }
                    }
                }
                drop(dirty);
                drop(revisions);
                drop(resident);
                candidates.sort_by_key(|coord| {
                    let index = coord.level.index() as usize;
                    let dx = coord.x - focus[index].x;
                    let dz = coord.z - focus[index].z;
                    (
                        index >= INTERACTIVE_SURFACE_LOD_LEVELS,
                        u8::MAX - coord.level.index(),
                        dx * dx + dz * dz,
                        coord.z,
                        coord.x,
                    )
                });
                let mut queue = self.surface_queue.borrow_mut();
                queue.clear();
                queue.extend(candidates);
            }

            const INTERACTIVE_SURFACE_BATCH: usize = 4;
            const BACKGROUND_SURFACE_BATCH: usize = 16;
            let mut tickets = Vec::with_capacity(BACKGROUND_SURFACE_BATCH);
            let mut priority = WorldProductPriority::VisibleSurface;
            loop {
                let batch_limit = if priority == WorldProductPriority::Prefetch {
                    BACKGROUND_SURFACE_BATCH
                } else {
                    INTERACTIVE_SURFACE_BATCH
                };
                if tickets.len() >= batch_limit {
                    break;
                }
                let Some(coord) = self.surface_queue.borrow_mut().pop_front() else {
                    break;
                };
                if self.surface_in_flight.borrow().contains(&coord)
                    || (self.surface_resident.borrow().contains(&coord)
                        && !self.surface_dirty.borrow().contains(&coord))
                {
                    continue;
                }
                let background = usize::from(coord.level.index()) >= INTERACTIVE_SURFACE_LOD_LEVELS;
                if background {
                    let background_in_flight =
                        self.surface_in_flight.borrow().iter().any(|coord| {
                            usize::from(coord.level.index()) >= INTERACTIVE_SURFACE_LOD_LEVELS
                        });
                    let diagnostics = self.scheduler.borrow().diagnostics();
                    let fine_current = diagnostics.generation.queued == 0
                        && diagnostics.generation.in_flight == 0
                        && diagnostics.meshing.queued == 0
                        && diagnostics.meshing.in_flight == 0
                        && diagnostics.upload.queued == 0
                        && diagnostics.upload.in_flight == 0;
                    let interactive_current = self
                        .surface_coverage_current_through(INTERACTIVE_SURFACE_LOD_LEVELS)
                        && !self.surface_dirty.borrow().iter().any(|coord| {
                            usize::from(coord.level.index()) < INTERACTIVE_SURFACE_LOD_LEVELS
                        })
                        && !self.surface_in_flight.borrow().iter().any(|coord| {
                            usize::from(coord.level.index()) < INTERACTIVE_SURFACE_LOD_LEVELS
                        });
                    if (!tickets.is_empty() && priority != WorldProductPriority::Prefetch)
                        || background_in_flight
                        || !fine_current
                        || !interactive_current
                    {
                        self.surface_queue.borrow_mut().push_front(coord);
                        break;
                    }
                    priority = WorldProductPriority::Prefetch;
                }
                let revision = {
                    let revisions = self.surface_revisions.borrow();
                    revisions
                        .requested_revision(coord)
                        .unwrap_or_else(|| revisions.epoch())
                };
                tickets.push(RemoteSurfaceTicket { coord, revision });
            }
            if tickets.is_empty() {
                return;
            }
            match self.remote.submit_surface_batch(priority, tickets.clone()) {
                Ok(_) => {
                    self.surface_in_flight
                        .borrow_mut()
                        .extend(tickets.into_iter().map(|ticket| ticket.coord));
                }
                Err(RemoteWorldError::Backpressured | RemoteWorldError::RequestWindowFull) => {
                    let mut queue = self.surface_queue.borrow_mut();
                    for ticket in tickets.into_iter().rev() {
                        queue.push_front(ticket.coord);
                    }
                }
                Err(error) => {
                    let mut queue = self.surface_queue.borrow_mut();
                    for ticket in tickets.into_iter().rev() {
                        queue.push_front(ticket.coord);
                    }
                    log_gpu_error(&format!("submit remote surface batch: {error}"));
                }
            }
        }

        async fn stop(&self) {
            self.prepare_stop();
            let camera = *self.camera.borrow();
            self.presence.close_after_final_pose(&camera).await;
            self.remote.close();
            let shutdown = self.store.borrow().shutdown();
            shutdown.await;
        }

        fn stop_now(&self) {
            self.prepare_stop();
            self.presence.close();
            self.remote.close();
            self.store.borrow().shutdown_now();
        }

        fn prepare_stop(&self) {
            self.persist_camera_if_changed(&self.camera.borrow());
            self.stopped.set(true);
            let id = self.frame_id.replace(0);
            if id != 0 {
                let _ = self.scope.cancel_animation_frame(id);
            }
            self.callback.borrow_mut().take();
        }

        fn persist_camera_if_changed(&self, camera: &CameraState) {
            let values = crate::camera_persistence_values(camera);
            if self.last_persisted_camera.get() == Some(values) {
                return;
            }
            match self.store.borrow().save_camera(camera) {
                Ok(()) => self.last_persisted_camera.set(Some(values)),
                Err(error) => web_sys::console::error_1(&error),
            }
        }

        fn feed_input(&self, bytes: &[u8]) -> bool {
            for chunk in bytes.chunks_exact(INPUT_RECORD_SIZE) {
                let record = bytemuck::pod_read_unaligned::<InputRecord>(chunk);
                match record.kind {
                    KIND_POINTER_DOWN => {
                        let was_open = self.renderer.borrow().ui_open();
                        let is_open = self.renderer.borrow_mut().handle_ui_pointer_down(
                            record.x,
                            record.y,
                            record.buttons & 2 != 0,
                        );
                        if !was_open && !is_open {
                            self.edit_target(record.buttons);
                        }
                    }
                    KIND_POINTER_MOVE => {
                        if self.renderer.borrow().ui_open() {
                            self.renderer
                                .borrow_mut()
                                .handle_ui_pointer_move(record.x, record.y);
                        } else {
                            self.camera
                                .borrow_mut()
                                .look(Vec2::new(record.dx, record.dy));
                        }
                    }
                    KIND_WHEEL => {
                        let direction = if record.dy >= 0.0 { 1 } else { -1 };
                        let _ = self
                            .renderer
                            .borrow_mut()
                            .cycle_placement_material(direction);
                    }
                    KIND_KEY_DOWN => {
                        if record.code == 8 {
                            self.renderer.borrow_mut().handle_ui_key(
                                record.code,
                                true,
                                record.flags & 1 != 0,
                            );
                        } else {
                            self.input.borrow_mut().set_key(record.code, true);
                        }
                    }
                    KIND_KEY_UP => {
                        if record.code == 8 {
                            self.renderer
                                .borrow_mut()
                                .handle_ui_key(record.code, false, false);
                        } else {
                            self.input.borrow_mut().set_key(record.code, false);
                        }
                    }
                    KIND_CANCEL => self.input.borrow_mut().clear(),
                    _ => {}
                }
            }
            if self.renderer.borrow_mut().take_coast_teleport_request() {
                self.teleport_to_coast();
            }
            if self
                .renderer
                .borrow_mut()
                .take_underwater_teleport_request()
            {
                self.teleport_underwater();
            }
            if self.renderer.borrow_mut().take_landmark_teleport_request() {
                self.teleport_to_next_landmark();
            }
            if self.renderer.borrow_mut().take_route_teleport_request() {
                self.teleport_to_next_route_mark();
            }
            if self.renderer.borrow_mut().take_cave_teleport_request() {
                self.teleport_to_cinder_vault();
            }
            self.renderer.borrow().ui_open()
        }

        fn teleport_to_cinder_vault(&self) {
            log_gpu_error("Cinder Vault is not advertised by the world service");
        }

        fn teleport_to_next_route_mark(&self) {
            log_gpu_error("authored routes are not advertised by the world service");
        }

        fn teleport_to_next_landmark(&self) {
            log_gpu_error("skyline landmarks are not advertised by the world service");
        }

        fn teleport_to_coast(&self) {
            log_gpu_error("surface search is not available in VXWP v2");
        }

        fn teleport_underwater(&self) {
            log_gpu_error("surface search is not available in VXWP v2");
        }

        fn edit_target(&self, buttons: u16) {
            let camera = *self.camera.borrow();
            let hit = self.raycast_target(&camera);
            let Some(hit) = hit else {
                return;
            };
            let action = if buttons & 1 != 0 {
                let Some(face) = VoxelFace::from_normal(hit.normal) else {
                    return;
                };
                EditAction::Dig {
                    hit: VoxelCoord::new(hit.voxel[0], hit.voxel[1], hit.voxel[2]),
                    face,
                }
            } else if buttons & 2 != 0 {
                if camera.intersects_voxel(hit.adjacent, VOXEL_SIZE_METRES) {
                    return;
                }
                let placement_material = { self.renderer.borrow().placement_material() };
                let Some(placement_material) = placement_material else {
                    self.renderer
                        .borrow_mut()
                        .show_gameplay_toast("Dig material before placing");
                    return;
                };
                EditAction::Place {
                    coord: VoxelCoord::new(hit.adjacent[0], hit.adjacent[1], hit.adjacent[2]),
                    material: placement_material,
                }
            } else {
                return;
            };

            // The server expands the hit face to a fixed inward half-metre cube and atomically owns
            // material yield/debit. The browser never emits 125 independently raceable mutations.
            let _ = self.submit_local_edit(action);
        }

        fn apply_server_edits(&self) {
            for event in self.remote.drain_edit_events() {
                match event {
                    RemoteEditEvent::Commit(commit) => {
                        if let Some(inventory) = commit.editor_inventory {
                            self.inventory.set(inventory);
                            self.renderer
                                .borrow_mut()
                                .set_inventory_counts(inventory.counts);
                        }
                        self.apply_durable_edits(
                            &commit.mutations,
                            commit.revision,
                            &commit.affected_chunks,
                            &commit.affected_surface_tiles,
                        );
                    }
                    RemoteEditEvent::ResyncRequired { revision } => {
                        self.resynchronize_world_products(revision);
                    }
                    RemoteEditEvent::Rejected {
                        operation_id,
                        message,
                    } => {
                        log_gpu_error(&format!(
                            "server rejected edit operation {operation_id}: {message}"
                        ));
                        self.renderer.borrow_mut().show_gameplay_toast(message);
                    }
                }
            }
        }

        fn submit_local_edit(&self, action: EditAction) -> [usize; 2] {
            match self.remote.submit_edit(action) {
                Ok(_) => [1, 0],
                Err(error) => {
                    log_gpu_error(&format!("submit authoritative edit: {error}"));
                    [0, 0]
                }
            }
        }

        fn apply_durable_edits(
            &self,
            mutations: &[VoxelMutation],
            server_revision: u64,
            affected_chunks: &[ChunkCoord],
            affected_surface_tiles: &[SurfaceTileCoord],
        ) -> EditRequirements {
            if mutations.is_empty() {
                return EditRequirements::default();
            }
            let coords = mutations
                .iter()
                .map(|mutation| mutation.coord)
                .collect::<Vec<_>>();
            let apply_values = self.edit_revisions.borrow_mut().observe_commit_batch(
                &coords,
                server_revision,
                affected_chunks,
                affected_surface_tiles,
            );
            let accepted_mutations = mutations
                .iter()
                .copied()
                .zip(apply_values)
                .filter_map(|(mutation, apply)| apply.then_some(mutation))
                .collect::<Vec<_>>();
            {
                let mut edits = self.edits.borrow_mut();
                for mutation in &accepted_mutations {
                    edits.replace_durable_override(mutation.coord, Some(mutation.material));
                }
            }
            voxels_world::apply_resident_mutations(
                &mut self.chunks.borrow_mut(),
                &mut self.chunk_halos.borrow_mut(),
                &accepted_mutations,
            );
            let canonical = {
                let mut scheduler = self.scheduler.borrow_mut();
                let report = scheduler.mark_voxels_edited(&coords);
                report
                    .affected_chunks
                    .into_iter()
                    .filter_map(|coord| {
                        let status = scheduler.status(coord)?;
                        status.desired.then_some(CanonicalRequirement {
                            coord,
                            revision: status.revision,
                        })
                    })
                    .collect()
            };
            let surface_revision = if affected_surface_tiles.is_empty() {
                self.surface_revisions.borrow().epoch()
            } else {
                self.surface_revisions.borrow_mut().begin_edit()
            };
            let mut surface = Vec::new();
            for &coord in affected_surface_tiles {
                if !self.surface_tile_relevant(coord)
                    && !self.surface_resident.borrow().contains(&coord)
                {
                    continue;
                }
                self.surface_revisions
                    .borrow_mut()
                    .request(coord, surface_revision);
                self.surface_dirty.borrow_mut().insert(coord);
                self.enqueue_surface_front(coord);
                surface.push(SurfaceRequirement {
                    coord,
                    revision: surface_revision,
                });
            }
            let performance = self.scope.performance();
            let started_ms = performance_now(performance.as_ref());
            let requirements = EditRequirements { canonical, surface };
            let mut trackers = self.edit_trackers.borrow_mut();
            let target = mutations[0].coord;
            if let Some(index) = trackers.iter().position(|tracker| tracker.target == target) {
                trackers.remove(index);
            }
            if trackers.len() == self.config.max_edit_trackers {
                trackers.pop_front();
            }
            trackers.push_back(EditTracker {
                target,
                started_ms,
                requirements: EditRequirements {
                    canonical: requirements.canonical.clone(),
                    surface: requirements.surface.clone(),
                },
            });
            requirements
        }

        fn resynchronize_world_products(&self, revision: u64) {
            log_gpu_error(&format!(
                "edit stream overflowed at revision {revision}; reconciling retained world products"
            ));
            *self.edits.borrow_mut() = EditMap::default();
            self.edit_revisions.borrow_mut().clear();
            self.surface_accepted_edit_revisions.borrow_mut().clear();
            self.scheduler.borrow_mut().invalidate_all_generation();
            let replacement = self.surface_revisions.borrow_mut().begin_edit();
            let retained = self
                .surface_resident
                .borrow()
                .iter()
                .copied()
                .collect::<Vec<_>>();
            for coord in retained {
                self.surface_revisions
                    .borrow_mut()
                    .request(coord, replacement);
                self.surface_dirty.borrow_mut().insert(coord);
                self.enqueue_surface_front(coord);
            }
        }

        fn surface_tile_relevant(&self, coord: SurfaceTileCoord) -> bool {
            coord.is_world_representable()
                && (surface_tile_in_coverage(
                    coord,
                    self.surface_focus.get(),
                    self.config.surface_load_radius_tiles,
                ) || surface_tile_in_coverage(
                    coord,
                    self.surface_active_focus.get(),
                    self.config.surface_load_radius_tiles,
                ))
        }

        fn surface_coverage_current(&self) -> bool {
            self.surface_coverage_current_through(SURFACE_LOD_LEVEL_COUNT)
        }

        fn surface_coverage_current_through(&self, level_count: usize) -> bool {
            if self.surface_focus.get().is_none() {
                return false;
            }
            let resident = self.surface_resident.borrow();
            let revisions = self.surface_revisions.borrow();
            for focus in [self.surface_focus.get(), self.surface_active_focus.get()]
                .into_iter()
                .flatten()
            {
                for (index, level) in SurfaceLodLevel::ALL
                    .into_iter()
                    .take(level_count)
                    .enumerate()
                {
                    let center = focus[index];
                    let radius = self.config.surface_load_radius_tiles[index];
                    for dz in -radius..=radius {
                        for dx in -radius..=radius {
                            let coord = SurfaceTileCoord::new(level, center.x + dx, center.z + dz);
                            if !coord.is_world_representable() {
                                continue;
                            }
                            if !resident.contains(&coord) || !revisions.is_current(coord) {
                                return false;
                            }
                        }
                    }
                }
            }
            true
        }

        fn update_edit_convergence(&self, now_ms: f64, submitted: bool) {
            if !submitted || self.edit_trackers.borrow().is_empty() {
                return;
            }
            let scheduler = self.scheduler.borrow();
            let surface_revisions = self.surface_revisions.borrow();
            let mut trackers = self.edit_trackers.borrow_mut();
            let mut pending = VecDeque::with_capacity(trackers.len());
            while let Some(tracker) = trackers.pop_front() {
                let canonical_ready = tracker.requirements.canonical.iter().all(|requirement| {
                    scheduler.status(requirement.coord).is_none_or(|status| {
                        !status.desired
                            || (status.state == ChunkState::Resident
                                && revision_satisfies(status.revision, requirement.revision))
                    })
                });
                let surface_ready = tracker.requirements.surface.iter().all(|requirement| {
                    surface_revisions
                        .resident_revision(requirement.coord)
                        .is_some_and(|revision| revision_satisfies(revision, requirement.revision))
                        || !self.surface_tile_relevant(requirement.coord)
                });
                if canonical_ready && surface_ready {
                    let full_ms = (now_ms - tracker.started_ms) as f32;
                    self.edit_last_ms.set(full_ms);
                } else {
                    pending.push_back(tracker);
                }
            }
            *trackers = pending;
        }

        fn raycast_target(&self, camera: &CameraState) -> Option<VoxelHit> {
            let chunks = self.chunks.borrow();
            let camera_voxel = VoxelCoord::new(
                (camera.position.x / VOXEL_SIZE_METRES).floor() as i32,
                (camera.position.y / VOXEL_SIZE_METRES).floor() as i32,
                (camera.position.z / VOXEL_SIZE_METRES).floor() as i32,
            );
            let mut skipping_origin_water =
                resident_material(&chunks, camera_voxel) == Some(Material::Water);
            raycast_voxels(
                camera.position,
                camera.forward(),
                5.0,
                VOXEL_SIZE_METRES,
                |x, y, z| {
                    let coord = VoxelCoord::new(x, y, z);
                    let material = resident_material(&chunks, coord);
                    if skipping_origin_water && material == Some(Material::Water) {
                        false
                    } else {
                        skipping_origin_water = false;
                        material.is_some_and(|material| {
                            material.is_collidable() || material == Material::Water
                        })
                    }
                },
            )
        }
    }

    #[wasm_bindgen]
    pub struct EngineHandle {
        engine: Option<Rc<Engine>>,
    }

    #[wasm_bindgen]
    impl EngineHandle {
        pub fn start_profile(&self, profile_id: u32) -> bool {
            self.engine
                .as_ref()
                .is_some_and(|engine| engine.start_profile(profile_id))
        }

        pub fn feed_input(&self, bytes: &[u8]) -> bool {
            if let Some(engine) = self.engine.as_ref() {
                engine.feed_input(bytes)
            } else {
                false
            }
        }

        pub fn resize(&self, css_width: f32, css_height: f32, dpr: f32) {
            if let Some(engine) = self.engine.as_ref() {
                let width = (css_width * dpr).round().max(1.0) as u32;
                let height = (css_height * dpr).round().max(1.0) as u32;
                engine.renderer.borrow_mut().resize(width, height, dpr);
            }
        }

        pub fn set_reduced_motion(&self, reduced_motion: bool) {
            if let Some(engine) = self.engine.as_ref() {
                engine
                    .renderer
                    .borrow_mut()
                    .set_reduced_motion(reduced_motion);
            }
        }

        /// `[resident, required, playable]` for the browser's canvas-only startup surface.
        pub fn startup_progress(&self) -> Vec<u32> {
            let Some(engine) = self.engine.as_ref() else {
                return vec![0, 0, 0];
            };
            let readiness = engine
                .scheduler
                .borrow()
                .vicinity_readiness(engine.config.startup_ready_radius_chunks);
            vec![
                usize_to_u32(readiness.resident),
                usize_to_u32(readiness.required),
                u32::from(engine.startup_ready.get()),
            ]
        }

        /// Deterministic browser-harness seam that submits through the same server-authoritative
        /// path as pointer input. It does not mutate local world state optimistically.
        pub fn submit_edit(&self, x: i32, y: i32, z: i32, material_id: u16) -> bool {
            let Some(engine) = self.engine.as_ref() else {
                return false;
            };
            let Some(material) = Material::from_id(material_id) else {
                return false;
            };
            engine.submit_local_edit(EditAction::Place {
                coord: VoxelCoord::new(x, y, z),
                material,
            })[0]
                == 1
        }

        /// Deterministic browser-harness seam for the exact gameplay dig action. The server, not
        /// this API, expands the hit voxel into the fixed 5x5x5 volume and validates reach.
        pub fn submit_dig(&self, x: i32, y: i32, z: i32) -> bool {
            self.engine.as_ref().is_some_and(|engine| {
                engine.submit_local_edit(EditAction::Dig {
                    hit: VoxelCoord::new(x, y, z),
                    face: VoxelFace::PositiveY,
                })[0]
                    == 1
            })
        }

        /// `[inventory_revision, air, grass, ..., glow_crystal]` in stable material-ID order.
        pub fn inventory(&self) -> Vec<f64> {
            let Some(engine) = self.engine.as_ref() else {
                return Vec::new();
            };
            let inventory = engine.inventory.get();
            std::iter::once(inventory.revision as f64)
                .chain(inventory.counts.into_iter().map(|count| count as f64))
                .collect()
        }

        /// Returns `[tile_x, tile_z, required_server_revision, accepted_server_revision,
        /// resident, dirty, fingerprint_low32, fingerprint_high32, quad_count, activation_mask]`
        /// for the tile containing one canonical voxel coordinate.
        pub fn surface_edit_state(&self, stride: i32, x: i32, z: i32) -> Vec<f64> {
            let Some(engine) = self.engine.as_ref() else {
                return Vec::new();
            };
            let Some(level) = SurfaceLodLevel::from_stride_voxels(stride) else {
                return Vec::new();
            };
            let coord = SurfaceTileCoord::containing(level, x, z);
            let floor = engine.edit_revisions.borrow().surface_floor(coord);
            let accepted = engine
                .surface_accepted_edit_revisions
                .borrow()
                .get(&coord)
                .copied()
                .unwrap_or(0);
            let diagnostics = engine.renderer.borrow().surface_tile_diagnostics(coord);
            let fingerprint = diagnostics.map_or(0, |value| value.0);
            vec![
                f64::from(coord.x),
                f64::from(coord.z),
                floor as f64,
                accepted as f64,
                if engine.surface_resident.borrow().contains(&coord) {
                    1.0
                } else {
                    0.0
                },
                if engine.surface_dirty.borrow().contains(&coord) {
                    1.0
                } else {
                    0.0
                },
                f64::from(fingerprint as u32),
                f64::from((fingerprint >> 32) as u32),
                f64::from(diagnostics.map_or(0, |value| value.1)),
                f64::from(diagnostics.map_or(0, |value| value.2)),
            ]
        }

        pub fn snapshot(&self) -> Vec<f32> {
            let mut values = Vec::new();
            if let Some(engine) = self.engine.as_ref() {
                let camera = engine.camera.borrow();
                let fluid = camera.fluid_state();
                let diagnostics = engine.scheduler.borrow().diagnostics();
                let render = engine.renderer.borrow().diagnostics();
                let target = engine.renderer.borrow().target_voxel();
                let lod_tiles = engine.surface_lod_counts();
                let canonical_voxel_bytes = engine
                    .chunks
                    .borrow()
                    .len()
                    .saturating_mul(CHUNK_VOXEL_BYTES)
                    .saturating_add(
                        engine
                            .chunk_halos
                            .borrow()
                            .values()
                            .map(MeshingHalo::logical_bytes)
                            .sum::<usize>(),
                    );
                let pending_mesh_bytes = engine
                    .pending_meshes
                    .borrow()
                    .values()
                    .map(MeshedChunk::retained_bytes)
                    .sum::<usize>();
                let edit_logical_bytes = engine.edits.borrow().logical_bytes();
                let profile = *engine.profile.borrow();
                let stream_interest = engine.cinder_stream_interest.get();
                let stream_interest_keys: BTreeSet<_> = stream_interest
                    .as_slice()
                    .iter()
                    .copied()
                    .map(coord_key)
                    .collect();
                let portal_active = engine.portal_active_chunks.borrow();
                let portal_active_columns: BTreeSet<_> =
                    portal_active.iter().map(|(x, _, z)| (*x, *z)).collect();
                let unreachable_portal_active = portal_active
                    .iter()
                    .filter(|key| !stream_interest_keys.contains(key))
                    .count();
                values.extend_from_slice(&[
                    camera.position.x,
                    camera.position.y,
                    camera.position.z,
                    camera.yaw,
                    camera.pitch,
                    if camera.grounded { 1.0 } else { 0.0 },
                    engine.renderer.borrow().quad_count() as f32,
                    engine.edits.borrow().len() as f32,
                    diagnostics.resident as f32,
                    diagnostics.tracked as f32,
                    render.visible_chunks as f32,
                    render.draw_calls as f32,
                    render.arena_pages as f32,
                    render.arena_allocated_bytes as f32 / (1024.0 * 1024.0),
                    render.arena_capacity_bytes as f32 / (1024.0 * 1024.0),
                    (diagnostics.generation.queued
                        + diagnostics.generation.in_flight
                        + diagnostics.meshing.queued
                        + diagnostics.meshing.in_flight
                        + diagnostics.upload.queued
                        + diagnostics.upload.in_flight
                        + engine.surface_queue.borrow().len()
                        + engine.surface_in_flight.borrow().len()
                        + engine.surface_dirty.borrow().len()) as f32,
                    engine.surface_resident.borrow().len() as f32,
                    engine.frame_milliseconds.get(),
                    render.shadow_draw_calls as f32,
                    render.shadow_cascades as f32,
                    diagnostics.initial_residency_latency.p95_frames as f32,
                    diagnostics.initial_residency_latency.max_frames as f32,
                    diagnostics.remesh_latency.p95_frames as f32,
                    diagnostics.remesh_latency.max_frames as f32,
                    lod_tiles[0] as f32,
                    lod_tiles[1] as f32,
                    lod_tiles[2] as f32,
                    lod_tiles[3] as f32,
                    render.water_quads as f32,
                    render.water_draw_calls as f32,
                    render.refraction_copy_bytes as f32 / (1024.0 * 1024.0),
                    fluid.immersion,
                    fluid.eye_depth_metres,
                    if fluid.eyes_submerged { 1.0 } else { 0.0 },
                    if fluid.swimming { 1.0 } else { 0.0 },
                    target.map_or(0.0, |coord| coord[0] as f32),
                    target.map_or(0.0, |coord| coord[1] as f32),
                    target.map_or(0.0, |coord| coord[2] as f32),
                    if target.is_some() { 1.0 } else { 0.0 },
                    render.core_gpu_bytes as f32 / (1024.0 * 1024.0),
                    engine.cpu_milliseconds.get(),
                    engine.simulation_milliseconds.get(),
                    engine.stream_milliseconds.get(),
                    engine.render_milliseconds.get(),
                    render.gpu_sample_id as f32,
                    render.gpu_total_ms.unwrap_or(-1.0),
                    render.gpu_shadow_ms.unwrap_or(-1.0),
                    render.gpu_world_ms.unwrap_or(-1.0),
                    render.gpu_water_ms.unwrap_or(-1.0),
                    render.gpu_ui_ms.unwrap_or(-1.0),
                    wasm_committed_bytes() as f32 / (1024.0 * 1024.0),
                    canonical_voxel_bytes as f32 / (1024.0 * 1024.0),
                    pending_mesh_bytes as f32 / (1024.0 * 1024.0),
                    edit_logical_bytes as f32 / (1024.0 * 1024.0),
                    diagnostics.total_evictions as f32,
                    diagnostics.stale_completions as f32,
                    profile.phase() as u8 as f32,
                    profile.elapsed_seconds(),
                    profile.distance_metres(),
                    if profile.phase() == ProfilePhase::Complete {
                        1.0
                    } else {
                        0.0
                    },
                    engine.profile_tracked_high.get() as f32,
                    engine.profile_surface_high.get() as f32,
                    engine.profile_pending_high.get() as f32,
                    engine.profile_pending_mesh_high.get() as f32,
                    engine.profile_arena_capacity_high.get() as f32 / (1024.0 * 1024.0),
                    engine.profile_wasm_high.get() as f32 / (1024.0 * 1024.0),
                    diagnostics
                        .total_evictions
                        .saturating_sub(engine.profile_start_evictions.get())
                        as f32,
                    if render.material_detail { 1.0 } else { 0.0 },
                    render.daylight_phase as f32,
                    render.surface_region as f32,
                    render.cloud_coverage,
                    if render.screen_space_ambient_occlusion {
                        1.0
                    } else {
                        0.0
                    },
                    render.gpu_depth_prepass_ms.unwrap_or(-1.0),
                    render.gpu_ambient_occlusion_ms.unwrap_or(-1.0),
                    render.ambient_occlusion_bytes as f32 / (1024.0 * 1024.0),
                    render.depth_prepass_draw_calls as f32,
                    render.enclosure,
                    render.interior_exposure,
                    if render.cave_headlamp { 1.0 } else { 0.0 },
                    engine.enclosure_probe_microseconds.get(),
                    render.local_light_candidates as f32,
                    render.active_local_lights as f32,
                    render.clipped_local_lights as f32,
                    render.occluded_local_lights as f32,
                    render.portal_rejected_local_lights as f32,
                    render.local_light_visibility_tests as f32,
                    engine
                        .cinder_portal_state
                        .get()
                        .open_count(CINDER_VAULT_PORTAL_COUNT) as f32,
                    engine.cinder_portal_revision.get() as f32,
                    if render.local_lighting { 1.0 } else { 0.0 },
                    engine
                        .renderer
                        .borrow()
                        .placement_material()
                        .unwrap_or(Material::Air)
                        .id() as f32,
                    diagnostics.secondary_interest_requested as f32,
                    diagnostics.secondary_interest_normalized as f32,
                    diagnostics.secondary_interest_desired as f32,
                    diagnostics.secondary_interest_truncated as f32,
                    if stream_interest.overflowed() {
                        1.0
                    } else {
                        0.0
                    },
                    portal_active.len() as f32,
                    portal_active_columns.len() as f32,
                    unreachable_portal_active as f32,
                    render.remote_avatars as f32,
                    render.avatar_parts as f32,
                    render.avatar_draw_calls as f32,
                    (render.viewport_fingerprint & 0x00ff_ffff) as f32,
                    ((render.viewport_fingerprint >> 24) & 0x00ff_ffff) as f32,
                    if engine.all_lods_ready.get() {
                        1.0
                    } else {
                        0.0
                    },
                    engine.surface_in_flight.borrow().len() as f32,
                    if engine.interactive_lods_ready.get() {
                        1.0
                    } else {
                        0.0
                    },
                    lod_tiles[4] as f32,
                    lod_tiles[5] as f32,
                    SNAPSHOT_SCHEMA_VERSION,
                ]);
                engine.frame_history.borrow_mut().drain_into(&mut values);
            }
            values
        }

        pub async fn destroy(&mut self) {
            if let Some(engine) = self.engine.take() {
                engine.stop().await;
            }
        }
    }

    impl Drop for EngineHandle {
        fn drop(&mut self) {
            if let Some(engine) = self.engine.take() {
                engine.stop_now();
            }
        }
    }

    #[wasm_bindgen]
    pub async fn create_engine(
        canvas: OffscreenCanvas,
        css_width: f32,
        css_height: f32,
        dpr: f32,
        reduced_motion: bool,
        config_toml: String,
        player: js_sys::Array,
    ) -> Result<EngineHandle, JsValue> {
        console_error_panic_hook::set_once();
        if player.length() != 3 {
            return Err(JsValue::from_str(
                "player bootstrap must contain three strings",
            ));
        }
        let player_string = |index: u32, name: &str| {
            player.get(index).as_string().ok_or_else(|| {
                JsValue::from_str(&format!("player bootstrap {name} is not a string"))
            })
        };
        let browser_user_id = player_string(0, "browser user id")?;
        let player_id = player_string(1, "player id")?;
        let player_name = player_string(2, "name")?;
        let identity = PlayerIdentity {
            browser_user_id: BrowserUserId::from_uuid_str(&browser_user_id)
                .ok_or_else(|| JsValue::from_str("browser user id is not a UUID"))?,
            player_id: PlayerId::from_uuid_str(&player_id)
                .ok_or_else(|| JsValue::from_str("player id is not a UUID"))?,
            player_name,
        };
        identity
            .validate()
            .map_err(|error| JsValue::from_str(&format!("player identity: {error}")))?;
        let client_config = ClientConfig::from_toml(&config_toml)
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        let world_transport = client_config.world.clone();
        let runtime = client_config.runtime;
        let streaming = &client_config.streaming;
        let diagnostics = client_config.diagnostics;
        let profiling = client_config.profiling;
        let engine_config = EngineConfig {
            fixed_step_seconds: runtime.fixed_step_seconds,
            max_steps_per_frame: runtime.max_steps_per_frame,
            persist_interval_ms: f64::from(runtime.persist_interval_ms),
            max_edit_trackers: runtime.max_edit_trackers as usize,
            stream_frame_budget: FrameBudget {
                generation: streaming.frame_budget.generation as usize,
                meshing: streaming.frame_budget.meshing as usize,
                upload: streaming.frame_budget.upload as usize,
            },
            startup_ready_radius_chunks: streaming.startup_ready_radius_chunks as i32,
            surface_load_radius_tiles: streaming
                .surface
                .load_radius_tiles
                .map(|radius| radius as i32),
            surface_retain_margin_tiles: streaming.surface.retention_margin_tiles as i32,
            enclosure_probe_interval_ms: f64::from(diagnostics.enclosure_probe_interval_ms),
            enclosure_probe_distance_metres: diagnostics.enclosure_probe_distance_metres,
        };
        let rendering = &client_config.rendering;
        let renderer_config = RendererConfig {
            features: RendererFeatureConfig {
                cascaded_sun_shadows: rendering.features.cascaded_sun_shadows,
                voxel_ambient_occlusion: rendering.features.voxel_ambient_occlusion,
                screen_space_ambient_occlusion: rendering.features.screen_space_ambient_occlusion,
                atmospheric_fog: rendering.features.atmospheric_fog,
                far_terrain: rendering.features.far_terrain,
                water_surface: rendering.features.water_surface,
                target_outline: rendering.features.target_outline,
                material_surface_detail: rendering.features.material_surface_detail,
                cave_headlamp: rendering.features.cave_headlamp,
                voxel_emissive_lights: rendering.features.voxel_emissive_lights,
            },
            mission_control: MissionControlConfig {
                open: rendering.mission_control.open,
                compact: rendering.mission_control.compact,
            },
            view_distance_metres: rendering.view_distance_metres,
            directional_shadows: DirectionalShadowConfig {
                vertical_fov_radians: rendering.shadows.vertical_fov_radians,
                near_plane: rendering.shadows.near_plane,
                far_plane: rendering.shadows.far_plane,
                split_lambda: rendering.shadows.split_lambda,
                shadow_map_resolution: rendering.shadows.shadow_map_resolution,
                caster_depth_expansion: rendering.shadows.caster_depth_expansion,
            },
            initial_daylight_phase: match rendering.daylight {
                DaylightConfig::Dawn => DaylightPhase::Dawn,
                DaylightConfig::ClearDay => DaylightPhase::ClearDay,
                DaylightConfig::GoldenHour => DaylightPhase::GoldenHour,
                DaylightConfig::BlueHour => DaylightPhase::BlueHour,
            },
        };
        let width = (css_width * dpr).round().max(1.0) as u32;
        let height = (css_height * dpr).round().max(1.0) as u32;
        let remote = RemoteWorldClient::connect(world_transport.clone(), identity.clone())
            .await
            .map_err(|error| JsValue::from_str(&format!("connect world service: {error}")))?;
        let opened = remote
            .world_opened()
            .ok_or_else(|| JsValue::from_str("world handshake completed without a manifest"))?;
        let manifest_hash = opened
            .manifest
            .manifest_hash()
            .map_err(|error| JsValue::from_str(&format!("native world manifest: {error}")))?;
        let persistence_world = PersistenceWorld::negotiated(manifest_hash);
        let store = Store::open(
            persistence_world,
            PersistencePlayer {
                player_id: identity.player_id,
            },
            PersistenceConfig {
                request_timeout_ms: client_config.persistence.request_timeout_ms as i32,
                request_retries: client_config.persistence.request_retries as usize,
                vfs_install_attempts: client_config.persistence.vfs_install_attempts as usize,
                vfs_retry_delay_ms: client_config.persistence.vfs_retry_delay_ms as i32,
            },
        )
        .await?;
        let edits = EditMap::default();
        let spawn = opened.spawn;
        // Position and look direction now resume from server-owned player state. OPFS remains a
        // local cache but can no longer turn a modified browser database into a teleport primitive.
        let resume = opened.player_resume;
        let (camera, _) = crate::camera_from_persisted_values([
            resume.eye_position_metres[0],
            resume.eye_position_metres[1],
            resume.eye_position_metres[2],
            resume.look_yaw_radians,
            resume.look_pitch_radians,
        ]);
        let persisted_camera = Some(crate::camera_persistence_values(&camera));
        let presence =
            RemotePresenceClient::start(world_transport, client_config.multiplayer, &opened)
                .map_err(|error| JsValue::from_str(&format!("connect player presence: {error}")))?;
        let renderer = Renderer::new(
            wgpu::SurfaceTarget::OffscreenCanvas(canvas),
            width,
            height,
            dpr,
            log_gpu_error,
            renderer_config,
        )
        .await
        .map_err(|error| JsValue::from_str(&error))?;
        let mut renderer = renderer;
        renderer.set_reduced_motion(reduced_motion);
        renderer.set_inventory_counts(opened.inventory.counts);
        let scheduler = StreamScheduler::new(StreamConfig {
            load_radius_chunks: streaming.load_radius_chunks as i32,
            vertical_radius_chunks: streaming.vertical_radius_chunks as i32,
            retention_margin_chunks: streaming.retention_margin_chunks as i32,
            max_tracked_chunks: streaming.max_tracked_chunks as usize,
            max_secondary_interest_chunks: streaming.max_secondary_interest_chunks as usize,
        })
        .map_err(|error| JsValue::from_str(&format!("stream configuration: {error:?}")))?;
        let remote_environment = (
            AtmosphereSample {
                humidity: spawn.moisture,
                coldness: 1.0 - spawn.temperature,
                aerosol: spawn.ridge,
                cloudiness: (spawn.moisture + spawn.ridge) * 0.5,
                horizon_warmth: spawn.temperature,
                haze: spawn.moisture * 0.5,
            },
            spawn.region,
        );
        let scope: DedicatedWorkerGlobalScope = js_sys::global().unchecked_into();
        let engine = Rc::new(Engine {
            config: engine_config,
            renderer: RefCell::new(renderer),
            camera: RefCell::new(camera),
            input: RefCell::new(InputState::default()),
            remote,
            presence,
            source_identity_hash: opened.manifest.source_identity_hash(),
            remote_environment,
            edits: RefCell::new(edits),
            inventory: Cell::new(opened.inventory),
            edit_revisions: RefCell::new(AuthoritativeEditRevisions::default()),
            scheduler: RefCell::new(scheduler),
            chunks: RefCell::new(BTreeMap::new()),
            chunk_halos: RefCell::new(BTreeMap::new()),
            pending_meshes: RefCell::new(BTreeMap::new()),
            surface_focus: Cell::new(None),
            surface_active_focus: Cell::new(None),
            surface_resident: RefCell::new(BTreeSet::new()),
            surface_revisions: RefCell::new(SurfaceRevisionCache::new()),
            surface_accepted_edit_revisions: RefCell::new(BTreeMap::new()),
            surface_queue: RefCell::new(VecDeque::new()),
            surface_in_flight: RefCell::new(BTreeSet::new()),
            surface_dirty: RefCell::new(BTreeSet::new()),
            fine_initialized: Cell::new(false),
            all_lods_ready: Cell::new(false),
            interactive_lods_ready: Cell::new(false),
            full_lods_initialized: Cell::new(false),
            startup_ready: Cell::new(false),
            store: RefCell::new(store),
            scope,
            callback: RefCell::new(None),
            frame_id: Cell::new(0),
            last_time: Cell::new(0.0),
            simulation_accumulator: Cell::new(0.0),
            frame_milliseconds: Cell::new(0.0),
            cpu_milliseconds: Cell::new(0.0),
            simulation_milliseconds: Cell::new(0.0),
            stream_milliseconds: Cell::new(0.0),
            render_milliseconds: Cell::new(0.0),
            frame_history: RefCell::new(FrameHistory::new()),
            edit_trackers: RefCell::new(VecDeque::new()),
            edit_last_ms: Cell::new(0.0),
            enclosure: Cell::new(EnclosureSample::OPEN),
            last_enclosure_probe: Cell::new(f64::NEG_INFINITY),
            enclosure_probe_microseconds: Cell::new(0.0),
            cinder_portal_state: Cell::new(PortalState::default()),
            cinder_portal_revision: Cell::new(0),
            cinder_stream_interest: Cell::new(CaveStreamInterest::empty()),
            radial_active_chunks: RefCell::new(BTreeSet::new()),
            portal_active_chunks: RefCell::new(BTreeSet::new()),
            interaction_active_chunks: RefCell::new(BTreeSet::new()),
            profile: RefCell::new(ProfileAutomation::with_config(ProfileConfig {
                fixed_step_seconds: engine_config.fixed_step_seconds,
                speed_metres_per_second: profiling.speed_metres_per_second,
                warmup_seconds: profiling.warmup_seconds,
                measure_seconds: profiling.measure_seconds,
            })),
            profile_tracked_high: Cell::new(0),
            profile_surface_high: Cell::new(0),
            profile_pending_high: Cell::new(0),
            profile_pending_mesh_high: Cell::new(0),
            profile_arena_capacity_high: Cell::new(0),
            profile_wasm_high: Cell::new(0),
            profile_start_evictions: Cell::new(0),
            last_persist: Cell::new(0.0),
            last_persisted_camera: Cell::new(persisted_camera),
            stopped: Cell::new(false),
        });
        engine.start()?;
        Ok(EngineHandle {
            engine: Some(engine),
        })
    }

    const fn coord_key(coord: ChunkCoord) -> (i32, i32, i32) {
        (coord.x, coord.y, coord.z)
    }

    fn usize_to_u32(value: usize) -> u32 {
        u32::try_from(value).unwrap_or(u32::MAX)
    }

    fn smoothed_ms(previous: f32, sample: f32) -> f32 {
        if previous <= 0.0 {
            sample
        } else {
            previous * 0.9 + sample * 0.1
        }
    }

    fn performance_now(performance: Option<&web_sys::Performance>) -> f64 {
        performance.map_or(0.0, web_sys::Performance::now)
    }

    fn wasm_committed_bytes() -> u64 {
        let memory: js_sys::WebAssembly::Memory = wasm_bindgen::memory().unchecked_into();
        let buffer: js_sys::ArrayBuffer = memory.buffer().unchecked_into();
        u64::from(buffer.byte_length())
    }

    fn world_to_chunk(position: glam::Vec3) -> ChunkCoord {
        let edge_metres = CHUNK_EDGE as f32 * VOXEL_SIZE_METRES;
        ChunkCoord::new(
            (position.x / edge_metres).floor() as i32,
            (position.y / edge_metres).floor() as i32,
            (position.z / edge_metres).floor() as i32,
        )
    }

    fn world_to_surface_tile(position: glam::Vec3, level: SurfaceLodLevel) -> SurfaceTileCoord {
        let voxel_x = (position.x / VOXEL_SIZE_METRES).floor() as i32;
        let voxel_z = (position.z / VOXEL_SIZE_METRES).floor() as i32;
        SurfaceTileCoord::containing(level, voxel_x, voxel_z)
    }

    fn surface_tile_in_coverage(
        coord: SurfaceTileCoord,
        focus: Option<[SurfaceTileCoord; SURFACE_LOD_LEVEL_COUNT]>,
        load_radius_tiles: [i32; SURFACE_LOD_LEVEL_COUNT],
    ) -> bool {
        let Some(focus) = focus else {
            return false;
        };
        let index = coord.level.index() as usize;
        let center = focus[index];
        let dx = (coord.x - center.x).abs();
        let dz = (coord.z - center.z).abs();
        dx.max(dz) <= load_radius_tiles[index]
    }
}

#[cfg(target_arch = "wasm32")]
pub use web::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_camera_values_report_when_recovery_changed_the_row() {
        let valid = [12.0, 4.5, -8.0, 0.75, -0.4];
        let (camera, canonical) = camera_from_persisted_values(valid);
        assert!(canonical);
        assert_eq!(camera_persistence_values(&camera), valid);

        for recovered in [
            [f32::NAN, 4.5, -8.0, 0.75, -0.4],
            [12.0, 4.5, -8.0, 1.0e30, -0.4],
            [12.0, 4.5, -8.0, 0.75, 4.0],
        ] {
            let (camera, canonical) = camera_from_persisted_values(recovered);
            assert!(!canonical);
            assert!(camera.position.is_finite());
            assert!(camera.yaw.is_finite());
            assert!(camera.pitch.is_finite());
        }
    }

    #[test]
    fn interaction_interest_reaches_below_a_downward_edit_corridor() {
        let mut camera = CameraState::spawn(glam::Vec3::new(1.6, 3.25, 1.6));
        camera.pitch = -1.5;
        let interest = interaction_stream_interest(&camera);

        assert!(interest.contains(&voxels_world::ChunkCoord::new(0, 1, 0)));
        assert!(
            interest
                .iter()
                .any(|coord| coord.x == 0 && coord.y <= -1 && coord.z == 0)
        );
        assert!(interest.len() <= 36, "lookahead must stay tightly bounded");
        assert!(interest.iter().all(|coord| coord.is_world_representable()));
    }

    #[test]
    fn interaction_interest_is_stable_across_negative_chunk_boundaries() {
        let mut camera = CameraState::spawn(glam::Vec3::new(-3.21, 0.05, -3.21));
        camera.pitch = -1.5;
        let first = interaction_stream_interest(&camera);
        let second = interaction_stream_interest(&camera);

        assert_eq!(first, second);
        assert!(first.contains(&voxels_world::ChunkCoord::new(-2, 0, -2)));
        assert!(first.iter().any(|coord| coord.y < 0));
    }
}
