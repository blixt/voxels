//! Browser/WASM leaf for Voxels. The worker owns the renderer, clock, input semantics, and persistence.

#[cfg(any(target_arch = "wasm32", test))]
use voxels_core::CameraState;

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
pub mod remote;
#[cfg(target_arch = "wasm32")]
mod web {
    use crate::persist::{PersistenceConfig, PersistencePlayer, PersistenceWorld, Store};
    use crate::remote::{
        RemoteChunkCompletion, RemoteSurfaceCompletion, RemoteSurfaceTicket, RemoteWorldClient,
        RemoteWorldError,
    };
    use bytemuck::{Pod, Zeroable};
    use glam::{Vec2, Vec3};
    use js_sys::Float32Array;
    use std::cell::{Cell, RefCell};
    use std::collections::{BTreeMap, BTreeSet, VecDeque};
    use std::rc::Rc;
    use voxels_client_config::{ClientConfig, DaylightConfig, PlacementMaterialConfig};
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
        ChunkState, CompletionStatus, FrameBudget, StreamConfig, StreamScheduler,
        SurfaceFocusAction, SurfaceRevisionCache, revision_satisfies,
    };
    use voxels_world::protocol::{BrowserUserId, PlayerId, PlayerIdentity};
    use voxels_world::{
        AtmosphereSample, CHUNK_EDGE, CHUNK_VOXEL_BYTES, CINDER_VAULT_PORTAL_COUNT,
        CaveStreamInterest, Chunk, ChunkCoord, EditMap, Material, MeshedChunk, MeshingHalo,
        PortalState, SurfaceLodLevel, SurfaceRegion, SurfaceSample, SurfaceTileCoord,
        VOXEL_SIZE_METRES, VoxelCoord, WorldProductPriority, WorldSourceIdentityHash, mesh_chunk,
    };
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;
    use web_sys::{DedicatedWorkerGlobalScope, OffscreenCanvas};

    const FRAME_HISTORY_CAPACITY: usize = 512;
    const EDIT_HISTORY_CAPACITY: usize = 64;
    const SNAPSHOT_SCHEMA_VERSION: f32 = 15.0;

    #[derive(Clone, Copy, Debug)]
    struct EngineConfig {
        fixed_step_seconds: f32,
        max_steps_per_frame: u32,
        persist_interval_ms: f64,
        max_edit_trackers: usize,
        stream_frame_budget: FrameBudget,
        surface_load_radius_tiles: [i32; 4],
        surface_retain_margin_tiles: i32,
        enclosure_probe_interval_ms: f64,
        enclosure_probe_distance_metres: f32,
        edit_profile_operations: u8,
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
        ordinal: u8,
        class: u8,
        operation: u8,
        started_ms: f64,
        enqueue_ms: f32,
        canonical_ms: Option<f32>,
        requirements: EditRequirements,
    }

    #[derive(Clone, Copy, Default)]
    struct EditSample {
        ordinal: f32,
        class: f32,
        operation: f32,
        enqueue_ms: f32,
        canonical_ms: f32,
        full_ms: f32,
    }

    struct EditHistory {
        samples: [EditSample; EDIT_HISTORY_CAPACITY],
        next: usize,
        len: usize,
        dropped: u32,
    }

    impl EditHistory {
        fn new() -> Self {
            Self {
                samples: [EditSample::default(); EDIT_HISTORY_CAPACITY],
                next: 0,
                len: 0,
                dropped: 0,
            }
        }

        fn push(&mut self, sample: EditSample) {
            self.samples[self.next] = sample;
            self.next = (self.next + 1) % EDIT_HISTORY_CAPACITY;
            if self.len < EDIT_HISTORY_CAPACITY {
                self.len += 1;
            } else {
                self.dropped = self.dropped.saturating_add(1);
            }
        }

        fn drain_into(&mut self, values: &mut Vec<f32>) {
            values.push(self.len as f32);
            values.push(self.dropped as f32);
            let first = (self.next + EDIT_HISTORY_CAPACITY - self.len) % EDIT_HISTORY_CAPACITY;
            for offset in 0..self.len {
                let sample = self.samples[(first + offset) % EDIT_HISTORY_CAPACITY];
                values.extend_from_slice(&[
                    sample.ordinal,
                    sample.class,
                    sample.operation,
                    sample.enqueue_ms,
                    sample.canonical_ms,
                    sample.full_ms,
                ]);
            }
            self.len = 0;
            self.dropped = 0;
        }
    }

    #[repr(u8)]
    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    enum EditProfilePhase {
        #[default]
        Idle = 0,
    }

    #[derive(Clone, Copy, Debug, Default)]
    struct EditProfile {
        phase: EditProfilePhase,
        next_operation: u8,
        baseline_edits: usize,
        restored: bool,
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
        source_identity_hash: WorldSourceIdentityHash,
        remote_environment: (AtmosphereSample, SurfaceRegion),
        edits: RefCell<EditMap>,
        edit_source_materials: RefCell<BTreeMap<VoxelCoord, Material>>,
        scheduler: RefCell<StreamScheduler>,
        chunks: RefCell<BTreeMap<(i32, i32, i32), Chunk>>,
        chunk_halos: RefCell<BTreeMap<(i32, i32, i32), MeshingHalo>>,
        pending_meshes: RefCell<BTreeMap<(i32, i32, i32), MeshedChunk>>,
        surface_focus: Cell<Option<[SurfaceTileCoord; 4]>>,
        surface_active_focus: Cell<Option<[SurfaceTileCoord; 4]>>,
        surface_resident: RefCell<BTreeSet<SurfaceTileCoord>>,
        surface_revisions: RefCell<SurfaceRevisionCache>,
        surface_queue: RefCell<VecDeque<SurfaceTileCoord>>,
        surface_in_flight: RefCell<BTreeSet<SurfaceTileCoord>>,
        surface_dirty: RefCell<BTreeSet<SurfaceTileCoord>>,
        fine_initialized: Cell<bool>,
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
        edit_history: RefCell<EditHistory>,
        edit_completed: Cell<u32>,
        edit_superseded: Cell<u32>,
        edit_last_ms: Cell<f32>,
        edit_profile: Cell<EditProfile>,
        enclosure: Cell<EnclosureSample>,
        last_enclosure_probe: Cell<f64>,
        enclosure_probe_microseconds: Cell<f32>,
        cinder_portal_state: Cell<PortalState>,
        cinder_portal_revision: Cell<u32>,
        cinder_stream_interest: Cell<CaveStreamInterest>,
        radial_active_chunks: RefCell<BTreeSet<(i32, i32, i32)>>,
        portal_active_chunks: RefCell<BTreeSet<(i32, i32, i32)>>,
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
            self.apply_remote_edits();
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
            let mut steps = 0;
            while accumulator >= self.config.fixed_step_seconds
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
                    Err(error) => log_gpu_error(&format!(
                        "enclosure surface probe failed; retaining prior sample: {error}"
                    )),
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
            let stream_ms = (performance_now(performance.as_ref()) - stream_start) as f32;
            self.stream_milliseconds
                .set(smoothed_ms(self.stream_milliseconds.get(), stream_ms));
            let target = self.raycast_target(&camera).map(|hit| hit.voxel);
            let mut renderer = self.renderer.borrow_mut();
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
            let all_lods_ready = fine_coverage_ready
                && self.surface_queue.borrow().is_empty()
                && self.surface_in_flight.borrow().is_empty()
                && self.surface_dirty.borrow().is_empty()
                && self.surface_coverage_current();
            debug_assert!(
                !all_lods_ready || self.surface_coverage_current(),
                "surface coverage became ready with missing or stale revisions"
            );
            renderer.set_lod_coverage_ready(self.fine_initialized.get(), all_lods_ready);
            if all_lods_ready {
                let voxel_x = (camera.position.x / VOXEL_SIZE_METRES).floor() as i32;
                let voxel_z = (camera.position.z / VOXEL_SIZE_METRES).floor() as i32;
                renderer.set_geometric_lod_focus(voxel_x, voxel_z);
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
            let interest = CaveStreamInterest::empty();
            let work = {
                let mut scheduler = self.scheduler.borrow_mut();
                scheduler.update_focus_with_interest(focus, interest.as_slice());
                scheduler.schedule_frame(self.config.stream_frame_budget)
            };
            let uploaded = !work.upload.is_empty();

            let generation_tickets = work.generation;
            if !generation_tickets.is_empty()
                && let Err(error) = self.remote.submit_chunk_batch(
                    WorldProductPriority::VisibleChunk,
                    generation_tickets.clone(),
                )
            {
                for ticket in generation_tickets {
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
            for ticket in work.meshing {
                let chunk = self.chunks.borrow().get(&coord_key(ticket.coord)).cloned();
                let Some(chunk) = chunk else {
                    continue;
                };
                let halo = self
                    .chunk_halos
                    .borrow()
                    .get(&coord_key(ticket.coord))
                    .cloned();
                let Some(halo) = halo else {
                    let _ = self.scheduler.borrow_mut().retry(ticket);
                    continue;
                };
                let edits = self.edits.borrow();
                let mut halo_contract_valid = true;
                let mesh = mesh_chunk(&chunk, |x, y, z| {
                    let coord = VoxelCoord::new(x, y, z);
                    let Some(generated) = halo.sample_world(x, y, z) else {
                        halo_contract_valid = false;
                        return Material::Stone;
                    };
                    edits.resolve_generated(coord, generated)
                });
                drop(edits);
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
            if uploaded || evicted {
                self.reconcile_chunk_activation(focus, interest);
            }
            self.stream_surface_lods(camera.position);
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
                    Ok(snapshot) => self.accept_generated_chunk(ticket, snapshot),
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
                let resident = self.surface_resident.borrow();
                let mut queue = self.surface_queue.borrow_mut();
                for ticket in completion.tickets {
                    if !resident.contains(&ticket.coord) {
                        queue.push_front(ticket.coord);
                    }
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
                    self.surface_queue.borrow_mut().push_front(ticket.coord);
                    continue;
                };
                let item = items.remove(index);
                let snapshot = match item.result {
                    Ok(snapshot) => snapshot,
                    Err(voxels_world::WorldSourceError::SourceCoverageUnavailable) => continue,
                    Err(error) => {
                        log_gpu_error(&format!(
                            "world service could not generate surface tile {:?}: {error}",
                            ticket.coord
                        ));
                        self.surface_queue.borrow_mut().push_front(ticket.coord);
                        continue;
                    }
                };
                if !self
                    .surface_revisions
                    .borrow()
                    .accepts(ticket.coord, ticket.revision)
                {
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
                    self.surface_dirty.borrow_mut().remove(&ticket.coord);
                } else {
                    self.surface_queue.borrow_mut().push_front(ticket.coord);
                }
            }
        }

        fn accept_generated_chunk(
            &self,
            ticket: voxels_runtime::WorkTicket,
            mut snapshot: voxels_world::ChunkSnapshot,
        ) {
            if snapshot.source_identity_hash != self.source_identity_hash()
                || snapshot.chunk.coord() != ticket.coord
                || snapshot.meshing_halo.coord() != ticket.coord
            {
                let _ = self.scheduler.borrow_mut().retry(ticket);
                return;
            }
            let edits = self.edits.borrow();
            self.edit_source_materials
                .borrow_mut()
                .extend(edits.source_values_for_overrides(&snapshot.chunk));
            edits.apply_to_chunk(&mut snapshot.chunk);
            drop(edits);
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

        fn reconcile_chunk_activation(&self, focus: ChunkCoord, interest: CaveStreamInterest) {
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

            let mut portal_columns = BTreeMap::<(i32, i32), Vec<ChunkCoord>>::new();
            for coord in interest.as_slice() {
                if scheduler
                    .status(*coord)
                    .is_some_and(|status| status.desired)
                {
                    portal_columns
                        .entry((coord.x, coord.z))
                        .or_default()
                        .push(*coord);
                }
            }
            let mut portal = BTreeSet::new();
            for coords in portal_columns.values() {
                if coords.iter().all(|coord| {
                    scheduler
                        .status(*coord)
                        .is_some_and(|status| status.state == ChunkState::Resident)
                }) {
                    portal.extend(coords.iter().copied().map(coord_key));
                }
            }
            drop(scheduler);
            self.reconcile_activation_reason(
                &self.radial_active_chunks,
                radial,
                ChunkActivationReason::Radial,
            );
            self.reconcile_activation_reason(
                &self.portal_active_chunks,
                portal,
                ChunkActivationReason::Portal,
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

        fn surface_lod_counts(&self) -> [u32; 4] {
            let mut counts = [0u32; 4];
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
                    let mut dirty = self.surface_dirty.borrow_mut();
                    let mut renderer = self.renderer.borrow_mut();
                    for coord in evicted {
                        resident.remove(&coord);
                        revisions.evict(coord);
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

            const REMOTE_SURFACE_BATCH: usize = 4;
            let mut tickets = Vec::with_capacity(REMOTE_SURFACE_BATCH);
            while tickets.len() < REMOTE_SURFACE_BATCH {
                let Some(coord) = self.surface_queue.borrow_mut().pop_front() else {
                    break;
                };
                if self.surface_in_flight.borrow().contains(&coord)
                    || self.surface_resident.borrow().contains(&coord)
                {
                    continue;
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
            match self
                .remote
                .submit_surface_batch(WorldProductPriority::VisibleSurface, tickets.clone())
            {
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
            let shutdown = self.store.borrow().shutdown();
            shutdown.await;
        }

        fn stop_now(&self) {
            self.prepare_stop();
            self.store.borrow().shutdown_now();
        }

        fn prepare_stop(&self) {
            self.persist_camera_if_changed(&self.camera.borrow());
            self.remote.close();
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
            let placement_material = self.renderer.borrow().placement_material();
            let hit = self.raycast_target(&camera);
            let Some(hit) = hit else {
                return;
            };
            let (target, material) = if buttons & 1 != 0 {
                (
                    VoxelCoord::new(hit.voxel[0], hit.voxel[1], hit.voxel[2]),
                    Material::Air,
                )
            } else if buttons & 2 != 0 {
                if camera.intersects_voxel(hit.adjacent, VOXEL_SIZE_METRES) {
                    return;
                }
                (
                    VoxelCoord::new(hit.adjacent[0], hit.adjacent[1], hit.adjacent[2]),
                    placement_material,
                )
            } else {
                return;
            };

            let Some(generated) = self.source_material_for_edit(target) else {
                log_gpu_error(
                    "edit refused: authoritative source material is not prepared for the target",
                );
                return;
            };
            let durable = (material != generated).then_some(material);
            let _ =
                self.submit_local_edit(target, durable, 0, u8::from(material == Material::Air), 0);
        }

        fn apply_remote_edits(&self) {
            let edits = match self.store.borrow().drain_remote_edits() {
                Ok(edits) => edits,
                Err(error) => {
                    web_sys::console::error_1(&error);
                    return;
                }
            };
            for (coord, material) in edits {
                // The persistence leader echoes every commit in its authoritative order, including
                // this tab's optimistic writes. A matching echo is free; a differing echo means an
                // intervening remote write committed first and must be corrected locally.
                if self.edits.borrow().override_at(coord) == material {
                    continue;
                }
                if let Err(error) = self.apply_durable_edit(coord, material) {
                    log_gpu_error(&format!("remote edit could not be applied: {error}"));
                }
            }
        }

        fn submit_local_edit(
            &self,
            target: VoxelCoord,
            durable: Option<Material>,
            class: u8,
            operation: u8,
            ordinal: u8,
        ) -> [usize; 2] {
            if let Err(error) = self.prepare_edit_source(target, durable) {
                log_gpu_error(&format!("edit refused: {error}"));
                return [0, 0];
            }
            let performance = self.scope.performance();
            let started_ms = performance_now(performance.as_ref());
            if let Err(error) = self.store.borrow().save_edit(target, durable) {
                web_sys::console::error_1(&error);
            }
            let enqueue_ms = (performance_now(performance.as_ref()) - started_ms) as f32;
            let requirements = match self.apply_durable_edit(target, durable) {
                Ok(requirements) => requirements,
                Err(error) => {
                    log_gpu_error(&format!("prepared edit could not be applied: {error}"));
                    return [0, 0];
                }
            };
            let counts = [requirements.canonical.len(), requirements.surface.len()];
            let mut trackers = self.edit_trackers.borrow_mut();
            if let Some(index) = trackers.iter().position(|tracker| tracker.target == target) {
                trackers.remove(index);
                self.edit_superseded
                    .set(self.edit_superseded.get().saturating_add(1));
            }
            if trackers.len() == self.config.max_edit_trackers {
                trackers.pop_front();
                self.edit_superseded
                    .set(self.edit_superseded.get().saturating_add(1));
            }
            trackers.push_back(EditTracker {
                target,
                ordinal,
                class,
                operation,
                started_ms,
                enqueue_ms,
                canonical_ms: None,
                requirements,
            });
            counts
        }

        fn source_material_for_edit(&self, target: VoxelCoord) -> Option<Material> {
            self.edit_source_materials
                .borrow()
                .get(&target)
                .copied()
                .or_else(|| {
                    self.edits
                        .borrow()
                        .override_at(target)
                        .is_none()
                        .then(|| resident_material(&self.chunks.borrow(), target))
                        .flatten()
                })
        }

        fn prepare_edit_source(
            &self,
            target: VoxelCoord,
            durable: Option<Material>,
        ) -> Result<(), String> {
            let previous = self.edits.borrow().override_at(target);
            let resident = resident_material(&self.chunks.borrow(), target);
            let cached = self.edit_source_materials.borrow().contains_key(&target);
            if !cached && previous.is_some() && resident.is_some() {
                return Err(
                    "resident edited voxel has no retained authoritative source value".to_owned(),
                );
            }
            if !cached
                && previous.is_none()
                && durable.is_some()
                && let Some(source_material) = resident
            {
                self.edit_source_materials
                    .borrow_mut()
                    .insert(target, source_material);
            }
            Ok(())
        }

        fn apply_durable_edit(
            &self,
            target: VoxelCoord,
            durable: Option<Material>,
        ) -> Result<EditRequirements, String> {
            self.prepare_edit_source(target, durable)?;
            let previous = self.edits.borrow().override_at(target);
            let resident_source = self
                .edit_source_materials
                .borrow()
                .get(&target)
                .copied()
                .or_else(|| {
                    previous
                        .is_none()
                        .then(|| resident_material(&self.chunks.borrow(), target))
                        .flatten()
                });
            self.edits
                .borrow_mut()
                .replace_durable_override(target, durable);
            let mut chunks = self.chunks.borrow_mut();
            if let Some(chunk) = chunks.get_mut(&coord_key(target.chunk())) {
                let material = durable.or(resident_source).ok_or_else(|| {
                    "resident edit restoration has no authoritative source value".to_owned()
                })?;
                let [x, y, z] = target.local();
                chunk.set(x, y, z, material);
            }
            drop(chunks);
            if durable.is_none() {
                self.edit_source_materials.borrow_mut().remove(&target);
            }
            let canonical = {
                let mut scheduler = self.scheduler.borrow_mut();
                let report = scheduler.mark_voxel_edited(target);
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
            let surface = Vec::new();
            Ok(EditRequirements { canonical, surface })
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
            if self.surface_focus.get().is_none() {
                return false;
            }
            let resident = self.surface_resident.borrow();
            let revisions = self.surface_revisions.borrow();
            for focus in [self.surface_focus.get(), self.surface_active_focus.get()]
                .into_iter()
                .flatten()
            {
                for (index, level) in SurfaceLodLevel::ALL.into_iter().enumerate() {
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
            while let Some(mut tracker) = trackers.pop_front() {
                let canonical_ready = tracker.requirements.canonical.iter().all(|requirement| {
                    scheduler.status(requirement.coord).is_none_or(|status| {
                        !status.desired
                            || (status.state == ChunkState::Resident
                                && revision_satisfies(status.revision, requirement.revision))
                    })
                });
                if canonical_ready && tracker.canonical_ms.is_none() {
                    tracker.canonical_ms = Some((now_ms - tracker.started_ms) as f32);
                }
                let surface_ready = tracker.requirements.surface.iter().all(|requirement| {
                    surface_revisions
                        .resident_revision(requirement.coord)
                        .is_some_and(|revision| revision_satisfies(revision, requirement.revision))
                        || !self.surface_tile_relevant(requirement.coord)
                });
                if canonical_ready && surface_ready {
                    let full_ms = (now_ms - tracker.started_ms) as f32;
                    self.edit_history.borrow_mut().push(EditSample {
                        ordinal: f32::from(tracker.ordinal),
                        class: f32::from(tracker.class),
                        operation: f32::from(tracker.operation),
                        enqueue_ms: tracker.enqueue_ms,
                        canonical_ms: tracker.canonical_ms.unwrap_or_default(),
                        full_ms,
                    });
                    self.edit_last_ms.set(full_ms);
                    self.edit_completed
                        .set(self.edit_completed.get().saturating_add(1));
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

        pub fn snapshot(&self) -> Float32Array {
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
                        + diagnostics.meshing.queued
                        + diagnostics.upload.queued
                        + engine.surface_queue.borrow().len()
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
                    engine.renderer.borrow().placement_material().id() as f32,
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
                    SNAPSHOT_SCHEMA_VERSION,
                ]);
                engine.frame_history.borrow_mut().drain_into(&mut values);
                let edit_profile = engine.edit_profile.get();
                values.extend_from_slice(&[
                    edit_profile.phase as u8 as f32,
                    edit_profile.next_operation as f32,
                    engine.config.edit_profile_operations as f32,
                    if edit_profile.restored { 1.0 } else { 0.0 },
                    edit_profile.baseline_edits as f32,
                    engine.edits.borrow().len() as f32,
                    engine.edit_trackers.borrow().len() as f32,
                    engine.edit_completed.get() as f32,
                    engine.edit_superseded.get() as f32,
                ]);
                engine.edit_history.borrow_mut().drain_into(&mut values);
            }
            Float32Array::from(values.as_slice())
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
            surface_load_radius_tiles: streaming
                .surface
                .load_radius_tiles
                .map(|radius| radius as i32),
            surface_retain_margin_tiles: streaming.surface.retention_margin_tiles as i32,
            enclosure_probe_interval_ms: f64::from(diagnostics.enclosure_probe_interval_ms),
            enclosure_probe_distance_metres: diagnostics.enclosure_probe_distance_metres,
            edit_profile_operations: profiling.edit_operations,
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
            initial_placement_material: match rendering.placement_material {
                PlacementMaterialConfig::Grass => Material::Grass,
                PlacementMaterialConfig::Stone => Material::Stone,
                PlacementMaterialConfig::Basalt => Material::Basalt,
                PlacementMaterialConfig::GlowCrystal => Material::GlowCrystal,
            },
        };
        let width = (css_width * dpr).round().max(1.0) as u32;
        let height = (css_height * dpr).round().max(1.0) as u32;
        let remote = RemoteWorldClient::connect(world_transport, identity.clone())
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
        let mut store = Store::open(
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
        let edits = store.load_edits()?;
        let spawn = opened.spawn;
        let spawn_surface = SurfaceSample {
            height: spawn.height,
            material: spawn.material,
            water_level: spawn.water_level,
            region: spawn.region,
            moisture: spawn.moisture,
            temperature: spawn.temperature,
            ridge: spawn.ridge,
            route: None,
        };
        let spawn_x = (spawn.x as f32 + 0.5) * VOXEL_SIZE_METRES;
        let spawn_z = (spawn.z as f32 + 0.5) * VOXEL_SIZE_METRES;
        let spawn_top = spawn_surface
            .water_level
            .unwrap_or(spawn_surface.height)
            .max(spawn_surface.height);
        let spawn_y = (spawn_top + 1) as f32 * VOXEL_SIZE_METRES
            + voxels_core::PLAYER_EYE_HEIGHT_METRES
            + 0.02;
        let spawn_camera = CameraState::spawn(glam::Vec3::new(spawn_x, spawn_y, spawn_z));
        let mut persisted_camera = None;
        let mut camera = spawn_camera;
        if let Some((candidate, canonical)) = store.load_camera()?
            && candidate.position.is_finite()
            && candidate.yaw.is_finite()
            && candidate.pitch.is_finite()
        {
            // The database is namespaced by the negotiated manifest hash. Exact resident chunks
            // become the runtime collision authority as streaming starts.
            camera = candidate;
            if canonical {
                persisted_camera = Some(crate::camera_persistence_values(&candidate));
            }
        }
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
            source_identity_hash: opened.manifest.source_identity_hash(),
            remote_environment,
            edits: RefCell::new(edits),
            edit_source_materials: RefCell::new(BTreeMap::new()),
            scheduler: RefCell::new(scheduler),
            chunks: RefCell::new(BTreeMap::new()),
            chunk_halos: RefCell::new(BTreeMap::new()),
            pending_meshes: RefCell::new(BTreeMap::new()),
            surface_focus: Cell::new(None),
            surface_active_focus: Cell::new(None),
            surface_resident: RefCell::new(BTreeSet::new()),
            surface_revisions: RefCell::new(SurfaceRevisionCache::new()),
            surface_queue: RefCell::new(VecDeque::new()),
            surface_in_flight: RefCell::new(BTreeSet::new()),
            surface_dirty: RefCell::new(BTreeSet::new()),
            fine_initialized: Cell::new(false),
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
            edit_history: RefCell::new(EditHistory::new()),
            edit_completed: Cell::new(0),
            edit_superseded: Cell::new(0),
            edit_last_ms: Cell::new(0.0),
            edit_profile: Cell::new(EditProfile::default()),
            enclosure: Cell::new(EnclosureSample::OPEN),
            last_enclosure_probe: Cell::new(f64::NEG_INFINITY),
            enclosure_probe_microseconds: Cell::new(0.0),
            cinder_portal_state: Cell::new(PortalState::default()),
            cinder_portal_revision: Cell::new(0),
            cinder_stream_interest: Cell::new(CaveStreamInterest::empty()),
            radial_active_chunks: RefCell::new(BTreeSet::new()),
            portal_active_chunks: RefCell::new(BTreeSet::new()),
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
        focus: Option<[SurfaceTileCoord; 4]>,
        load_radius_tiles: [i32; 4],
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
}
