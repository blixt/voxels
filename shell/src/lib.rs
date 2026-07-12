//! Browser/WASM leaf for Voxels. The worker owns the renderer, clock, input semantics, and persistence.

#[cfg(target_arch = "wasm32")]
mod persist;
#[cfg(target_arch = "wasm32")]
mod web {
    use crate::persist::Store;
    use bytemuck::{Pod, Zeroable};
    use glam::{Vec2, Vec3};
    use js_sys::Float32Array;
    use std::cell::{Cell, RefCell};
    use std::collections::{BTreeMap, BTreeSet, VecDeque};
    use std::rc::Rc;
    use voxels_core::{
        CameraState, EnclosureSample, InputState, ProfileAutomation, ProfilePhase, VoxelHit,
        VoxelPhysics, probe_enclosure, raycast_voxels, voxel_segment_is_clear,
    };
    use voxels_render::renderer::{LocalLightVisibility, Renderer};
    use voxels_render::ui::LiveStats;
    use voxels_runtime::{
        ChunkState, FrameBudget, StreamConfig, StreamScheduler, SurfaceFocusAction,
        SurfaceRevisionCache,
    };
    use voxels_world::{
        CHUNK_EDGE, CHUNK_VOXEL_BYTES, CINDER_VAULT, CINDER_VAULT_NODES, CINDER_VAULT_PORTAL_COUNT,
        Chunk, ChunkCoord, EditMap, Generator, Material, MeshedChunk, PortalState, SkylineFeature,
        SkylineFeatureKind, SurfaceLodLevel, SurfaceTileCoord, VOXEL_SIZE_METRES, VoxelCoord,
        cinder_vault_portal_is_open, cinder_vault_portal_state,
        cinder_vault_portals_affected_by_voxel, cinder_vault_visibility_cell,
        cinder_vault_visibility_graph, first_pilgrim_road_length_voxels,
        first_pilgrim_route_anchor, first_pilgrim_route_anchor_count,
        generate_edited_surface_tile_mesh, generate_edited_water_tile_mesh, mesh_chunk,
        pilgrim_chapter_at_distance, sample_cinder_vault, sample_first_pilgrim_road,
        surface_tiles_affected_by_voxel,
    };
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;
    use web_sys::{DedicatedWorkerGlobalScope, OffscreenCanvas};

    const WORLD_SEED: u64 = 0x5eed_cafe;
    const SURFACE_LOAD_RADIUS_TILES: [i32; 4] = [4, 4, 4, 5];
    const SURFACE_RETAIN_MARGIN_TILES: i32 = 1;
    const SIMULATION_STEP_SECONDS: f32 = 1.0 / 120.0;
    const MAX_SIMULATION_STEPS_PER_FRAME: u32 = 6;
    const FRAME_HISTORY_CAPACITY: usize = 512;
    const EDIT_HISTORY_CAPACITY: usize = 64;
    const SNAPSHOT_SCHEMA_VERSION: f32 = 14.0;
    const MAX_EDIT_TRACKERS: usize = 128;
    const ENCLOSURE_PROBE_INTERVAL_MS: f64 = 100.0;
    const ENCLOSURE_PROBE_DISTANCE_METRES: f32 = 12.0;
    const COAST_WATER_REFERENCE: [i32; 2] = [18_016, 12_896];
    const EDIT_PROFILE_TERRAIN: [i32; 3] = [18_016, -12, 12_896];
    const EDIT_PROFILE_WATER: [i32; 3] = [18_016, 10, 12_896];
    const EDIT_PROFILE_OPERATIONS: u8 = 40;
    const STREAM_FRAME_BUDGET: FrameBudget = FrameBudget {
        generation: 2,
        meshing: 1,
        upload: 3,
    };

    type FrameCallback = Closure<dyn FnMut(f64)>;

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
        Settling = 1,
        Running = 2,
        Complete = 3,
        Failed = 4,
    }

    #[derive(Clone, Copy, Debug, Default)]
    struct EditProfile {
        phase: EditProfilePhase,
        next_operation: u8,
        baseline_edits: usize,
        restored: bool,
    }

    impl EditProfile {
        fn active(self) -> bool {
            matches!(
                self.phase,
                EditProfilePhase::Settling | EditProfilePhase::Running
            )
        }
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
        time: f32,
        flags: u32,
    }

    const INPUT_RECORD_SIZE: usize = size_of::<InputRecord>();
    const _: () = assert!(INPUT_RECORD_SIZE == 28);
    const KIND_POINTER_MOVE: u8 = 1;
    const KIND_POINTER_DOWN: u8 = 0;
    const KIND_KEY_DOWN: u8 = 4;
    const KIND_KEY_UP: u8 = 5;
    const KIND_CANCEL: u8 = 6;

    fn log_gpu_error(message: &str) {
        web_sys::console::error_1(&JsValue::from_str(message));
    }

    struct Engine {
        renderer: RefCell<Renderer>,
        camera: RefCell<CameraState>,
        input: RefCell<InputState>,
        generator: Generator,
        edits: RefCell<EditMap>,
        scheduler: RefCell<StreamScheduler>,
        chunks: RefCell<BTreeMap<(i32, i32, i32), Chunk>>,
        pending_meshes: RefCell<BTreeMap<(i32, i32, i32), MeshedChunk>>,
        surface_focus: Cell<Option<[SurfaceTileCoord; 4]>>,
        surface_active_focus: Cell<Option<[SurfaceTileCoord; 4]>>,
        surface_resident: RefCell<BTreeSet<SurfaceTileCoord>>,
        surface_revisions: RefCell<SurfaceRevisionCache>,
        surface_queue: RefCell<VecDeque<SurfaceTileCoord>>,
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
        cinder_portal_dirty: Cell<u8>,
        cinder_portal_revision: Cell<u32>,
        route_tour_index: Cell<u16>,
        cave_tour_index: Cell<u8>,
        landmark_tour_index: Cell<u8>,
        profile: RefCell<ProfileAutomation>,
        profile_tracked_high: Cell<usize>,
        profile_surface_high: Cell<usize>,
        profile_pending_high: Cell<usize>,
        profile_pending_mesh_high: Cell<usize>,
        profile_arena_capacity_high: Cell<u64>,
        profile_wasm_high: Cell<u64>,
        profile_start_evictions: Cell<u64>,
        last_persist: Cell<f64>,
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

        fn start_profile(&self, profile_id: u32) -> bool {
            match profile_id {
                1 => self.start_stream_profile(),
                2 => self.start_edit_profile(),
                _ => false,
            }
        }

        fn start_stream_profile(&self) -> bool {
            if self.edit_profile.get().active() {
                return false;
            }
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

        fn start_edit_profile(&self) -> bool {
            let terrain = voxel_coord(EDIT_PROFILE_TERRAIN);
            let water = voxel_coord(EDIT_PROFILE_WATER);
            if self.profile.borrow().running() {
                return self.refuse_edit_profile(1, "streaming profile is active");
            }
            if !self.store.borrow().owns_persistence() {
                return self.refuse_edit_profile(2, "tab does not own persistence");
            }
            if !self.edit_trackers.borrow().is_empty() {
                return self.refuse_edit_profile(3, "prior edits are still converging");
            }
            if self.generator.sample(terrain.x, terrain.y, terrain.z) != Material::Sand {
                return self.refuse_edit_profile(4, "terrain fixture no longer resolves to sand");
            }
            if self.generator.sample(water.x, water.y, water.z) != Material::Water {
                return self.refuse_edit_profile(5, "water fixture no longer resolves to water");
            }
            let edits = self.edits.borrow();
            if edits.override_at(terrain).is_some() || edits.override_at(water).is_some() {
                drop(edits);
                return self.refuse_edit_profile(6, "fixture has existing local voxel edits");
            }
            let baseline_edits = edits.len();
            drop(edits);

            let mut camera = CameraState::spawn(glam::Vec3::new(
                (EDIT_PROFILE_TERRAIN[0] as f32 + 0.5) * VOXEL_SIZE_METRES,
                2.74,
                (EDIT_PROFILE_TERRAIN[2] as f32 + 0.5) * VOXEL_SIZE_METRES,
            ));
            camera.yaw = 0.35;
            camera.pitch = 0.12;
            *self.camera.borrow_mut() = camera;
            self.input.borrow_mut().clear();
            *self.edit_history.borrow_mut() = EditHistory::new();
            self.edit_completed.set(0);
            self.edit_superseded.set(0);
            self.edit_profile.set(EditProfile {
                phase: EditProfilePhase::Settling,
                next_operation: 0,
                baseline_edits,
                restored: false,
            });
            true
        }

        fn refuse_edit_profile(&self, code: u8, reason: &str) -> bool {
            self.edit_profile.set(EditProfile {
                phase: EditProfilePhase::Failed,
                next_operation: code,
                ..EditProfile::default()
            });
            log_gpu_error(&format!("edit profile refused: {reason}"));
            false
        }

        fn frame(&self, time: f64) {
            let performance = self.scope.performance();
            let cpu_start = performance_now(performance.as_ref());
            self.apply_remote_edits();
            self.refresh_cinder_portals();
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
            let edit_profiling = self.edit_profile.get().active();
            let edits = self.edits.borrow();
            let mut generated_columns = BTreeMap::new();
            let mut accumulator = (self.simulation_accumulator.get() + dt.min(0.1))
                .min(SIMULATION_STEP_SECONDS * MAX_SIMULATION_STEPS_PER_FRAME as f32);
            let mut steps = 0;
            while accumulator >= SIMULATION_STEP_SECONDS && steps < MAX_SIMULATION_STEPS_PER_FRAME {
                if profiling {
                    self.profile.borrow_mut().advance_fixed_step();
                } else if !edit_profiling {
                    camera.update(
                        &self.input.borrow(),
                        SIMULATION_STEP_SECONDS,
                        VOXEL_SIZE_METRES,
                        |x, y, z| {
                            let coord = VoxelCoord::new(x, y, z);
                            let material = edits.override_at(coord).unwrap_or_else(|| {
                                generated_columns
                                    .entry((x, z))
                                    .or_insert_with(|| self.generator.column(x, z))
                                    .sample(y)
                            });
                            VoxelPhysics {
                                collidable: material.is_collidable(),
                                fluid: material.is_fluid(),
                            }
                        },
                    );
                }
                accumulator -= SIMULATION_STEP_SECONDS;
                steps += 1;
            }
            self.simulation_accumulator.set(accumulator);
            drop(edits);
            if profiling && let Some(pose) = self.profile.borrow().pose() {
                let voxel_x = (pose.position_xz.x / VOXEL_SIZE_METRES).floor() as i32;
                let voxel_z = (pose.position_xz.y / VOXEL_SIZE_METRES).floor() as i32;
                let surface = self.generator.surface_sample(voxel_x, voxel_z);
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
            if time - self.last_enclosure_probe.get() >= ENCLOSURE_PROBE_INTERVAL_MS {
                let eye_voxel = (camera.position / VOXEL_SIZE_METRES).floor().as_ivec3();
                let surface_height = self.generator.surface_height(eye_voxel.x, eye_voxel.z);
                let underground = eye_voxel.y + 3 < surface_height
                    || sample_cinder_vault(eye_voxel.x, eye_voxel.y, eye_voxel.z).is_some();
                let probe_start = performance_now(performance.as_ref());
                let sample = if underground {
                    let edits = self.edits.borrow();
                    let chunks = self.chunks.borrow();
                    let mut columns = BTreeMap::new();
                    probe_enclosure(
                        camera.position,
                        ENCLOSURE_PROBE_DISTANCE_METRES,
                        VOXEL_SIZE_METRES,
                        |x, y, z| {
                            let coord = VoxelCoord::new(x, y, z);
                            edits
                                .override_at(coord)
                                .or_else(|| {
                                    chunks.get(&coord_key(coord.chunk())).map(|chunk| {
                                        let [local_x, local_y, local_z] = coord.local();
                                        chunk.get(local_x, local_y, local_z)
                                    })
                                })
                                .unwrap_or_else(|| {
                                    columns
                                        .entry((x, z))
                                        .or_insert_with(|| self.generator.column(x, z))
                                        .sample(y)
                                })
                                .occludes_ambient()
                        },
                    )
                } else {
                    EnclosureSample::OPEN
                };
                self.enclosure.set(sample);
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
            let atmosphere_x = (camera.position.x / VOXEL_SIZE_METRES).floor() as i32;
            let atmosphere_z = (camera.position.z / VOXEL_SIZE_METRES).floor() as i32;
            let (atmosphere, region) = self.generator.atmosphere_sample(atmosphere_x, atmosphere_z);
            renderer.set_atmosphere(atmosphere, region);
            let eye_voxel_y = (camera.position.y / VOXEL_SIZE_METRES).floor() as i32;
            let enclosure = self.enclosure.get();
            renderer.set_enclosure(enclosure);
            if sample_cinder_vault(atmosphere_x, eye_voxel_y, atmosphere_z).is_some() {
                renderer
                    .set_route_status("CINDER VAULT", (enclosure.enclosure * 100.0).round() as u8);
            } else if let Some(route) = sample_first_pilgrim_road(atmosphere_x, atmosphere_z) {
                let chapter = pilgrim_chapter_at_distance(route.distance_along_voxels);
                let progress = (route.distance_along_voxels / first_pilgrim_road_length_voxels()
                    * 100.0)
                    .round() as u8;
                renderer.set_route_status(chapter.id.label(), progress);
            } else {
                renderer.set_route_status("OFF PILGRIM ROAD", 0);
            }
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
                && self.surface_dirty.borrow().is_empty();
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
            let edits = self.edits.borrow();
            let chunks = self.chunks.borrow();
            let mut light_columns = BTreeMap::new();
            let camera_voxel = (camera.position / VOXEL_SIZE_METRES).floor().as_ivec3();
            let camera_visibility_cell =
                cinder_vault_visibility_cell(camera_voxel.x, camera_voxel.y, camera_voxel.z);
            let cinder_graph = cinder_vault_visibility_graph().ok();
            let cinder_portal_state = self.cinder_portal_state.get();
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
                },
                |position, maximum_geodesic_metres| {
                    let light_voxel = (Vec3::from_array(position) / VOXEL_SIZE_METRES)
                        .floor()
                        .as_ivec3();
                    let light_visibility_cell =
                        cinder_vault_visibility_cell(light_voxel.x, light_voxel.y, light_voxel.z);
                    if light_visibility_cell != camera_visibility_cell
                        && let Some(graph) = cinder_graph
                    {
                        let Some(distance) = graph.shortest_open_distance(
                            camera_visibility_cell,
                            light_visibility_cell,
                            cinder_portal_state,
                        ) else {
                            return LocalLightVisibility::PortalRejected;
                        };
                        if distance > maximum_geodesic_metres {
                            return LocalLightVisibility::PortalRejected;
                        }
                        return LocalLightVisibility::Visible;
                    }
                    if voxel_segment_is_clear(
                        camera.position,
                        Vec3::from_array(position),
                        VOXEL_SIZE_METRES,
                        |x, y, z| {
                            let coord = VoxelCoord::new(x, y, z);
                            let material = edits
                                .override_at(coord)
                                .or_else(|| {
                                    chunks.get(&coord_key(coord.chunk())).map(|chunk| {
                                        let [local_x, local_y, local_z] = coord.local();
                                        chunk.get(local_x, local_y, local_z)
                                    })
                                })
                                .unwrap_or_else(|| {
                                    light_columns
                                        .entry((x, z))
                                        .or_insert_with(|| self.generator.column(x, z))
                                        .sample(y)
                                });
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
            drop(edits);
            let rendered = renderer.diagnostics();
            drop(renderer);
            self.update_edit_convergence(time, submitted);
            self.advance_edit_profile(all_lods_ready, submitted);
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
            if time - self.last_persist.get() >= 1_000.0 {
                if let Err(error) = self.store.borrow().save_camera(&camera) {
                    web_sys::console::error_1(&error);
                }
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
            let focus = world_to_chunk(camera.position);
            let work = {
                let mut scheduler = self.scheduler.borrow_mut();
                scheduler.update_focus(focus);
                scheduler.schedule_frame(STREAM_FRAME_BUDGET)
            };

            for ticket in work.generation {
                let mut chunk = self.generator.generate_chunk(ticket.coord);
                self.edits.borrow().apply_to_chunk(&mut chunk);
                self.chunks
                    .borrow_mut()
                    .insert(coord_key(ticket.coord), chunk);
                let _ = self.scheduler.borrow_mut().complete(ticket);
            }
            for ticket in work.meshing {
                let chunk = self.chunks.borrow().get(&coord_key(ticket.coord)).cloned();
                let Some(chunk) = chunk else {
                    continue;
                };
                let edits = self.edits.borrow();
                let origin = ticket.coord.world_origin();
                let generated_region = self.generator.region(
                    origin[0] - 1,
                    origin[2] - 1,
                    CHUNK_EDGE + 2,
                    CHUNK_EDGE + 2,
                );
                let mesh = mesh_chunk(&chunk, |x, y, z| {
                    let coord = VoxelCoord::new(x, y, z);
                    edits
                        .override_at(coord)
                        .unwrap_or_else(|| generated_region.sample(x, y, z))
                });
                drop(edits);
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
                    self.update_render_ready_column(ticket.coord);
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
            if !evictions.is_empty() {
                let mut chunks = self.chunks.borrow_mut();
                let mut pending = self.pending_meshes.borrow_mut();
                let mut renderer = self.renderer.borrow_mut();
                for eviction in evictions {
                    chunks.remove(&coord_key(eviction.coord));
                    pending.remove(&coord_key(eviction.coord));
                    renderer.set_chunk_column_active(eviction.coord.x, eviction.coord.z, false);
                    renderer.remove_chunk(eviction.coord);
                }
            }
            self.stream_surface_lods(camera.position);
        }

        fn update_render_ready_column(&self, coord: ChunkCoord) {
            let scheduler = self.scheduler.borrow();
            let focus = scheduler.focus();
            let vertical = scheduler.config().vertical_radius_chunks;
            let ready = (-vertical..=vertical).all(|dy| {
                scheduler
                    .status(ChunkCoord::new(coord.x, focus.y + dy, coord.z))
                    .is_some_and(|status| status.desired && status.state == ChunkState::Resident)
            });
            drop(scheduler);
            self.renderer
                .borrow_mut()
                .set_chunk_column_active(coord.x, coord.z, ready);
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
                    let radius = SURFACE_LOAD_RADIUS_TILES[index];
                    let level_focus = focus[index];
                    for dz in -radius..=radius {
                        for dx in -radius..=radius {
                            desired.insert(SurfaceTileCoord::new(
                                level,
                                level_focus.x + dx,
                                level_focus.z + dz,
                            ));
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
                        let retain = SURFACE_LOAD_RADIUS_TILES[index] + SURFACE_RETAIN_MARGIN_TILES;
                        let dx = coord.x - focus[index].x;
                        let dz = coord.z - focus[index].z;
                        let outside_pending = dx.abs().max(dz.abs()) > retain;
                        let outside_active = self.surface_active_focus.get().is_none_or(|active| {
                            let dx = coord.x - active[index].x;
                            let dz = coord.z - active[index].z;
                            dx.abs().max(dz.abs())
                                > SURFACE_LOAD_RADIUS_TILES[index] + SURFACE_RETAIN_MARGIN_TILES
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

            let dirty = {
                let resident = self.surface_resident.borrow();
                self.surface_dirty
                    .borrow()
                    .iter()
                    .copied()
                    .find(|coord| resident.contains(coord))
            };
            let next = dirty.or_else(|| self.surface_queue.borrow_mut().pop_front());
            let Some(coord) = next else {
                return;
            };
            let edits = self.edits.borrow();
            let revision = {
                let revisions = self.surface_revisions.borrow();
                revisions
                    .requested_revision(coord)
                    .unwrap_or_else(|| revisions.epoch())
            };
            let mesh = generate_edited_surface_tile_mesh(self.generator, &edits, coord);
            let water = generate_edited_water_tile_mesh(self.generator, &edits, coord);
            drop(edits);
            if !self.surface_revisions.borrow().accepts(coord, revision) {
                if dirty.is_some() {
                    self.surface_dirty.borrow_mut().insert(coord);
                } else {
                    self.surface_queue.borrow_mut().push_front(coord);
                }
                return;
            }
            if self
                .renderer
                .borrow_mut()
                .upload_surface_tile_meshes(&mesh, &water)
            {
                self.surface_resident.borrow_mut().insert(coord);
                let committed = self.surface_revisions.borrow_mut().commit(coord, revision);
                debug_assert!(committed, "uploaded surface revision became stale");
                self.surface_dirty.borrow_mut().remove(&coord);
            } else if dirty.is_none() {
                self.surface_queue.borrow_mut().push_front(coord);
            }
        }

        fn stop(&self) {
            if let Err(error) = self.store.borrow().save_camera(&self.camera.borrow()) {
                web_sys::console::error_1(&error);
            }
            self.store.borrow().shutdown();
            self.stopped.set(true);
            let id = self.frame_id.replace(0);
            if id != 0 {
                let _ = self.scope.cancel_animation_frame(id);
            }
            self.callback.borrow_mut().take();
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
            let index = self.cave_tour_index.get() % 4;
            self.cave_tour_index.set((index + 1) % 4);
            let mut camera = match index {
                0 => {
                    let x = CINDER_VAULT.entrance[0] + 40;
                    let z = CINDER_VAULT.entrance[2];
                    let surface = self.generator.surface_sample(x, z);
                    let mut camera = CameraState::spawn(glam::Vec3::new(
                        (x as f32 + 0.5) * VOXEL_SIZE_METRES,
                        (surface.height + 1) as f32 * VOXEL_SIZE_METRES
                            + voxels_core::PLAYER_EYE_HEIGHT_METRES,
                        (z as f32 + 0.5) * VOXEL_SIZE_METRES,
                    ));
                    camera.yaw = -std::f32::consts::FRAC_PI_2;
                    camera.pitch = -0.08;
                    camera
                }
                1 => {
                    let node = CINDER_VAULT_NODES[2].center;
                    let mut camera = CameraState::spawn(glam::Vec3::new(
                        (node[0] as f32 + 0.5) * VOXEL_SIZE_METRES,
                        (node[1] as f32 + 4.0) * VOXEL_SIZE_METRES,
                        (node[2] as f32 + 0.5) * VOXEL_SIZE_METRES,
                    ));
                    camera.yaw = -2.36;
                    camera.pitch = -0.10;
                    camera
                }
                2 => {
                    let node = CINDER_VAULT.chamber;
                    let mut camera = CameraState::spawn(glam::Vec3::new(
                        (node[0] as f32 + 0.5) * VOXEL_SIZE_METRES,
                        (node[1] as f32 + 0.5) * VOXEL_SIZE_METRES,
                        (node[2] as f32 + 0.5) * VOXEL_SIZE_METRES,
                    ));
                    camera.yaw = -2.13;
                    camera.pitch = -0.08;
                    camera
                }
                _ => {
                    let x = CINDER_VAULT.chamber[0];
                    let z = CINDER_VAULT.chamber[2];
                    let surface = self.generator.surface_sample(x, z);
                    let mut camera = CameraState::spawn(glam::Vec3::new(
                        (x as f32 + 0.5) * VOXEL_SIZE_METRES,
                        (surface.height + 1) as f32 * VOXEL_SIZE_METRES
                            + voxels_core::PLAYER_EYE_HEIGHT_METRES,
                        (z as f32 + 0.5) * VOXEL_SIZE_METRES,
                    ));
                    camera.yaw = 0.35;
                    camera.pitch = 0.28;
                    camera
                }
            };
            camera.velocity = glam::Vec3::ZERO;
            *self.camera.borrow_mut() = camera;
            self.input.borrow_mut().clear();
            self.last_enclosure_probe.set(f64::NEG_INFINITY);
        }

        fn teleport_to_next_route_mark(&self) {
            let count = first_pilgrim_route_anchor_count();
            if count == 0 {
                web_sys::console::error_1(&JsValue::from_str(
                    "pilgrim road has no authored route marks",
                ));
                return;
            }
            let ordinal = self.route_tour_index.get() % count;
            self.route_tour_index.set((ordinal + 1) % count);
            let Some(anchor) = first_pilgrim_route_anchor(ordinal) else {
                web_sys::console::error_1(&JsValue::from_str(
                    "pilgrim road route mark could not be reconstructed",
                ));
                return;
            };
            let Some(feature) = self
                .generator
                .skyline_features_anchored_in([
                    [anchor.anchor[0], anchor.anchor[1]],
                    [anchor.anchor[0] + 1, anchor.anchor[1] + 1],
                ])
                .into_iter()
                .find(|feature| {
                    feature
                        .route_landmark
                        .is_some_and(|landmark| landmark.ordinal == ordinal)
                })
            else {
                web_sys::console::error_1(&JsValue::from_str(
                    "pilgrim road route mark has no canonical landmark",
                ));
                return;
            };
            self.teleport_to_feature(feature);
        }

        fn teleport_to_next_landmark(&self) {
            let index =
                usize::from(self.landmark_tour_index.get()).min(SkylineFeatureKind::ALL.len() - 1);
            let kind = SkylineFeatureKind::ALL[index];
            self.landmark_tour_index
                .set(((index + 1) % SkylineFeatureKind::ALL.len()) as u8);
            let camera = *self.camera.borrow();
            let voxel_x = (camera.position.x / VOXEL_SIZE_METRES).floor() as i32;
            let voxel_z = (camera.position.z / VOXEL_SIZE_METRES).floor() as i32;
            let feature = if kind.is_semantic_hero() {
                self.generator
                    .nearest_prominent_skyline_feature(voxel_x, voxel_z, kind, 192)
            } else {
                self.generator
                    .nearest_skyline_feature(voxel_x, voxel_z, kind, 192)
            };
            let Some(feature) = feature else {
                web_sys::console::error_1(&JsValue::from_str(
                    "landmark tour could not find the requested biome archetype",
                ));
                return;
            };
            self.teleport_to_feature(feature);
        }

        fn teleport_to_feature(&self, feature: SkylineFeature) {
            let [anchor_x, ground, anchor_z] = feature.anchor;
            let offsets = [[48, 36], [-48, 36], [48, -36], [-48, -36]];
            let [view_x, view_z, view_height] = offsets
                .into_iter()
                .find_map(|offset| {
                    let offset = feature.oriented_offset(offset[0], offset[1]);
                    let x = anchor_x + offset[0];
                    let z = anchor_z + offset[1];
                    let surface = self.generator.surface_sample(x, z);
                    surface
                        .water_level
                        .is_none()
                        .then_some([x, z, surface.height])
                })
                .unwrap_or([anchor_x + 48, anchor_z + 36, ground]);
            let mut camera = CameraState::spawn(glam::Vec3::new(
                (view_x as f32 + 0.5) * VOXEL_SIZE_METRES,
                (view_height + 1) as f32 * VOXEL_SIZE_METRES
                    + voxels_core::PLAYER_EYE_HEIGHT_METRES
                    + 0.02,
                (view_z as f32 + 0.5) * VOXEL_SIZE_METRES,
            ));
            let look_x = (anchor_x - view_x) as f32;
            let look_z = (anchor_z - view_z) as f32;
            camera.yaw = look_x.atan2(-look_z);
            let horizontal_metres = look_x.hypot(look_z) * VOXEL_SIZE_METRES;
            let target_y = (ground + (feature.trunk_top - ground) / 2) as f32 * VOXEL_SIZE_METRES;
            camera.pitch = (target_y - camera.position.y).atan2(horizontal_metres);
            *self.camera.borrow_mut() = camera;
            self.input.borrow_mut().clear();
        }

        fn teleport_to_coast(&self) {
            let [water_x, water_z] = COAST_WATER_REFERENCE;
            let mut coast = None;
            'search: for radius in 1..=192 {
                for x in water_x - radius..=water_x + radius {
                    for z in [water_z - radius, water_z + radius] {
                        let surface = self.generator.surface_sample(x, z);
                        if surface.water_level.is_none() {
                            coast = Some((x, z, surface.height));
                            break 'search;
                        }
                    }
                }
                for z in water_z - radius + 1..water_z + radius {
                    for x in [water_x - radius, water_x + radius] {
                        let surface = self.generator.surface_sample(x, z);
                        if surface.water_level.is_none() {
                            coast = Some((x, z, surface.height));
                            break 'search;
                        }
                    }
                }
            }
            let Some((x, z, height)) = coast else {
                web_sys::console::error_1(&JsValue::from_str("coast teleport anchor is invalid"));
                return;
            };
            let mut camera = CameraState::spawn(glam::Vec3::new(
                (x as f32 + 0.5) * VOXEL_SIZE_METRES,
                (height + 1) as f32 * VOXEL_SIZE_METRES
                    + voxels_core::PLAYER_EYE_HEIGHT_METRES
                    + 0.02,
                (z as f32 + 0.5) * VOXEL_SIZE_METRES,
            ));
            let look_x = (water_x - x) as f32;
            let look_z = (water_z - z) as f32;
            camera.yaw = look_x.atan2(-look_z);
            camera.pitch = -0.12;
            *self.camera.borrow_mut() = camera;
            self.input.borrow_mut().clear();
        }

        fn teleport_underwater(&self) {
            let [water_x, water_z] = COAST_WATER_REFERENCE;
            let mut dive = None;
            'search: for radius in 0..=256 {
                for x in water_x - radius..=water_x + radius {
                    for z in [water_z - radius, water_z + radius] {
                        let surface = self.generator.surface_sample(x, z);
                        if let Some(water_level) = surface.water_level {
                            let seabed_top = (surface.height + 1) as f32 * VOXEL_SIZE_METRES;
                            let eye_y = seabed_top + voxels_core::PLAYER_EYE_HEIGHT_METRES + 0.04;
                            let water_top = (water_level + 1) as f32 * VOXEL_SIZE_METRES;
                            if eye_y <= water_top - 0.12 {
                                dive = Some((x, z, eye_y));
                                break 'search;
                            }
                        }
                    }
                }
                for z in water_z - radius + 1..water_z + radius {
                    for x in [water_x - radius, water_x + radius] {
                        let surface = self.generator.surface_sample(x, z);
                        if let Some(water_level) = surface.water_level {
                            let seabed_top = (surface.height + 1) as f32 * VOXEL_SIZE_METRES;
                            let eye_y = seabed_top + voxels_core::PLAYER_EYE_HEIGHT_METRES + 0.04;
                            let water_top = (water_level + 1) as f32 * VOXEL_SIZE_METRES;
                            if eye_y <= water_top - 0.12 {
                                dive = Some((x, z, eye_y));
                                break 'search;
                            }
                        }
                    }
                }
            }
            let Some((x, z, eye_y)) = dive else {
                web_sys::console::error_1(&JsValue::from_str(
                    "underwater teleport could not find a player-deep ocean column",
                ));
                return;
            };
            let mut camera = CameraState::spawn(glam::Vec3::new(
                (x as f32 + 0.5) * VOXEL_SIZE_METRES,
                eye_y,
                (z as f32 + 0.5) * VOXEL_SIZE_METRES,
            ));
            camera.yaw = 0.35;
            camera.pitch = 0.18;
            let edits = self.edits.borrow();
            let mut generated_columns = BTreeMap::new();
            camera.refresh_fluid_state(VOXEL_SIZE_METRES, |x, y, z| {
                let coord = VoxelCoord::new(x, y, z);
                let material = edits.override_at(coord).unwrap_or_else(|| {
                    generated_columns
                        .entry((x, z))
                        .or_insert_with(|| self.generator.column(x, z))
                        .sample(y)
                });
                VoxelPhysics {
                    collidable: material.is_collidable(),
                    fluid: material.is_fluid(),
                }
            });
            drop(edits);
            *self.camera.borrow_mut() = camera;
            self.input.borrow_mut().clear();
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

            let generated = self.generator.sample(target.x, target.y, target.z);
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
                let _ = self.apply_durable_edit(coord, material);
            }
        }

        fn refresh_cinder_portals(&self) {
            let dirty = self.cinder_portal_dirty.replace(0);
            if dirty == 0 {
                return;
            }
            let edits = self.edits.borrow();
            let mut state = self.cinder_portal_state.get();
            let mut changed = false;
            for portal_index in 0..u8::BITS as usize {
                if dirty & (1 << portal_index) == 0 {
                    continue;
                }
                let Some(open) = cinder_vault_portal_is_open(portal_index, |x, y, z| {
                    !edits
                        .sample(self.generator, VoxelCoord::new(x, y, z))
                        .occludes_ambient()
                }) else {
                    continue;
                };
                changed |= state.is_open(portal_index) != open;
                let _ = state.set_open(portal_index, open);
            }
            self.cinder_portal_state.set(state);
            if changed {
                self.cinder_portal_revision
                    .set(self.cinder_portal_revision.get().wrapping_add(1));
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
            let performance = self.scope.performance();
            let started_ms = performance_now(performance.as_ref());
            if let Err(error) = self.store.borrow().save_edit(target, durable) {
                web_sys::console::error_1(&error);
            }
            let enqueue_ms = (performance_now(performance.as_ref()) - started_ms) as f32;
            let requirements = self.apply_durable_edit(target, durable);
            let counts = [requirements.canonical.len(), requirements.surface.len()];
            let mut trackers = self.edit_trackers.borrow_mut();
            if let Some(index) = trackers.iter().position(|tracker| tracker.target == target) {
                trackers.remove(index);
                self.edit_superseded
                    .set(self.edit_superseded.get().saturating_add(1));
            }
            if trackers.len() == MAX_EDIT_TRACKERS {
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

        fn apply_durable_edit(
            &self,
            target: VoxelCoord,
            durable: Option<Material>,
        ) -> EditRequirements {
            let affected_portals =
                cinder_vault_portals_affected_by_voxel(target.x, target.y, target.z);
            self.cinder_portal_dirty
                .set(self.cinder_portal_dirty.get() | affected_portals);
            self.edits
                .borrow_mut()
                .replace_durable_override(target, durable);
            let material =
                durable.unwrap_or_else(|| self.generator.sample(target.x, target.y, target.z));
            if let Some(chunk) = self.chunks.borrow_mut().get_mut(&coord_key(target.chunk())) {
                let [x, y, z] = target.local();
                chunk.set(x, y, z, material);
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
            let revision = self.surface_revisions.borrow_mut().begin_edit();
            let mut surface = Vec::new();
            let resident = self.surface_resident.borrow();
            let mut revisions = self.surface_revisions.borrow_mut();
            let mut dirty = self.surface_dirty.borrow_mut();
            let edits = self.edits.borrow();
            for level in SurfaceLodLevel::ALL {
                for coord in surface_tiles_affected_by_voxel(self.generator, &edits, level, target)
                {
                    let relevant = self.surface_tile_relevant(coord);
                    if !relevant && !resident.contains(&coord) {
                        continue;
                    }
                    revisions.request(coord, revision);
                    if relevant {
                        dirty.insert(coord);
                        surface.push(SurfaceRequirement { coord, revision });
                    }
                }
            }
            EditRequirements { canonical, surface }
        }

        fn surface_tile_relevant(&self, coord: SurfaceTileCoord) -> bool {
            surface_tile_in_coverage(coord, self.surface_focus.get())
                || surface_tile_in_coverage(coord, self.surface_active_focus.get())
        }

        fn surface_coverage_current(&self) -> bool {
            let resident = self.surface_resident.borrow();
            let revisions = self.surface_revisions.borrow();
            for focus in [self.surface_focus.get(), self.surface_active_focus.get()]
                .into_iter()
                .flatten()
            {
                for (index, level) in SurfaceLodLevel::ALL.into_iter().enumerate() {
                    let center = focus[index];
                    let radius = SURFACE_LOAD_RADIUS_TILES[index];
                    for dz in -radius..=radius {
                        for dx in -radius..=radius {
                            let coord = SurfaceTileCoord::new(level, center.x + dx, center.z + dz);
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
                                && status.revision >= requirement.revision)
                    })
                });
                if canonical_ready && tracker.canonical_ms.is_none() {
                    tracker.canonical_ms = Some((now_ms - tracker.started_ms) as f32);
                }
                let surface_ready = tracker.requirements.surface.iter().all(|requirement| {
                    surface_revisions
                        .resident_revision(requirement.coord)
                        .is_some_and(|revision| revision >= requirement.revision)
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

        fn advance_edit_profile(&self, all_lods_ready: bool, submitted: bool) {
            let profile = self.edit_profile.get();
            match profile.phase {
                EditProfilePhase::Idle | EditProfilePhase::Complete | EditProfilePhase::Failed => {
                    return;
                }
                EditProfilePhase::Settling => {
                    if submitted
                        && all_lods_ready
                        && self.surface_focus.get() == self.surface_active_focus.get()
                    {
                        self.edit_profile.set(EditProfile {
                            phase: EditProfilePhase::Running,
                            ..profile
                        });
                    }
                    return;
                }
                EditProfilePhase::Running => {}
            }
            if !submitted || !all_lods_ready || !self.edit_trackers.borrow().is_empty() {
                return;
            }
            if profile.next_operation == EDIT_PROFILE_OPERATIONS {
                let terrain = voxel_coord(EDIT_PROFILE_TERRAIN);
                let water = voxel_coord(EDIT_PROFILE_WATER);
                let restored = self.edits.borrow().len() == profile.baseline_edits
                    && self.edits.borrow().override_at(terrain).is_none()
                    && self.edits.borrow().override_at(water).is_none()
                    && self.edit_completed.get() == u32::from(EDIT_PROFILE_OPERATIONS)
                    && self.edit_superseded.get() == 0;
                self.edit_profile.set(EditProfile {
                    phase: if restored {
                        EditProfilePhase::Complete
                    } else {
                        EditProfilePhase::Failed
                    },
                    restored,
                    ..profile
                });
                return;
            }

            let step = profile.next_operation % 4;
            let (target, class) = if step < 2 {
                (voxel_coord(EDIT_PROFILE_TERRAIN), 1)
            } else {
                (voxel_coord(EDIT_PROFILE_WATER), 2)
            };
            let removing = step == 0 || step == 2;
            let counts = self.submit_local_edit(
                target,
                removing.then_some(Material::Air),
                class,
                if removing { 1 } else { 2 },
                profile.next_operation + 1,
            );
            self.edit_profile.set(EditProfile {
                phase: if counts == [3, 4] {
                    EditProfilePhase::Running
                } else {
                    EditProfilePhase::Failed
                },
                next_operation: profile.next_operation + 1,
                ..profile
            });
        }

        fn raycast_target(&self, camera: &CameraState) -> Option<VoxelHit> {
            let edits = self.edits.borrow();
            let camera_voxel = VoxelCoord::new(
                (camera.position.x / VOXEL_SIZE_METRES).floor() as i32,
                (camera.position.y / VOXEL_SIZE_METRES).floor() as i32,
                (camera.position.z / VOXEL_SIZE_METRES).floor() as i32,
            );
            let mut skipping_origin_water =
                edits.sample(self.generator, camera_voxel) == Material::Water;
            raycast_voxels(
                camera.position,
                camera.forward(),
                5.0,
                VOXEL_SIZE_METRES,
                |x, y, z| {
                    let material = edits.sample(self.generator, VoxelCoord::new(x, y, z));
                    if skipping_origin_water && material == Material::Water {
                        false
                    } else {
                        skipping_origin_water = false;
                        material.is_collidable() || material == Material::Water
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
                    .saturating_mul(CHUNK_VOXEL_BYTES);
                let pending_mesh_bytes = engine
                    .pending_meshes
                    .borrow()
                    .values()
                    .map(MeshedChunk::retained_bytes)
                    .sum::<usize>();
                let edit_logical_bytes = engine.edits.borrow().logical_bytes();
                let profile = *engine.profile.borrow();
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
                    SNAPSHOT_SCHEMA_VERSION,
                ]);
                engine.frame_history.borrow_mut().drain_into(&mut values);
                let edit_profile = engine.edit_profile.get();
                values.extend_from_slice(&[
                    edit_profile.phase as u8 as f32,
                    edit_profile.next_operation as f32,
                    EDIT_PROFILE_OPERATIONS as f32,
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

        pub fn destroy(&mut self) {
            if let Some(engine) = self.engine.take() {
                engine.stop();
            }
        }
    }

    impl Drop for EngineHandle {
        fn drop(&mut self) {
            self.destroy();
        }
    }

    #[wasm_bindgen]
    pub async fn create_engine(
        canvas: OffscreenCanvas,
        css_width: f32,
        css_height: f32,
        dpr: f32,
        reduced_motion: bool,
    ) -> Result<EngineHandle, JsValue> {
        console_error_panic_hook::set_once();
        let width = (css_width * dpr).round().max(1.0) as u32;
        let height = (css_height * dpr).round().max(1.0) as u32;
        let generator = Generator::new(WORLD_SEED);
        let mut store =
            Store::open(WORLD_SEED, voxels_world::generation::GENERATOR_VERSION).await?;
        let edits = store.load_edits()?;
        let spawn_z = 5.2;
        let spawn_voxel_z = (spawn_z / VOXEL_SIZE_METRES).floor() as i32;
        let spawn_surface = generator.surface_sample(0, spawn_voxel_z);
        let spawn_top = spawn_surface
            .water_level
            .unwrap_or(spawn_surface.height)
            .max(spawn_surface.height);
        let spawn_y = (spawn_top + 1) as f32 * VOXEL_SIZE_METRES
            + voxels_core::PLAYER_EYE_HEIGHT_METRES
            + 0.02;
        let mut camera = store
            .load_camera()?
            .filter(|camera| {
                if !camera.position.is_finite()
                    || !camera.yaw.is_finite()
                    || !camera.pitch.is_finite()
                {
                    return false;
                }
                let mut generated_columns = BTreeMap::new();
                !camera.overlaps_collidable(VOXEL_SIZE_METRES, |x, y, z| {
                    let coord = VoxelCoord::new(x, y, z);
                    let material = edits.override_at(coord).unwrap_or_else(|| {
                        generated_columns
                            .entry((x, z))
                            .or_insert_with(|| generator.column(x, z))
                            .sample(y)
                    });
                    VoxelPhysics {
                        collidable: material.is_collidable(),
                        fluid: material.is_fluid(),
                    }
                })
            })
            .unwrap_or_else(|| CameraState::spawn(glam::Vec3::new(0.0, spawn_y, spawn_z)));
        {
            let mut generated_columns = BTreeMap::new();
            camera.refresh_fluid_state(VOXEL_SIZE_METRES, |x, y, z| {
                let coord = VoxelCoord::new(x, y, z);
                let material = edits.override_at(coord).unwrap_or_else(|| {
                    generated_columns
                        .entry((x, z))
                        .or_insert_with(|| generator.column(x, z))
                        .sample(y)
                });
                VoxelPhysics {
                    collidable: material.is_collidable(),
                    fluid: material.is_fluid(),
                }
            });
        }
        let renderer = Renderer::new(
            wgpu::SurfaceTarget::OffscreenCanvas(canvas),
            width,
            height,
            dpr,
            log_gpu_error,
        )
        .await
        .map_err(|error| JsValue::from_str(&error))?;
        let mut renderer = renderer;
        renderer.set_reduced_motion(reduced_motion);
        let scheduler = StreamScheduler::new(StreamConfig {
            load_radius_chunks: 5,
            vertical_radius_chunks: 1,
            retention_margin_chunks: 1,
            max_tracked_chunks: 320,
        })
        .map_err(|error| JsValue::from_str(&format!("stream configuration: {error:?}")))?;
        let cinder_portal_state = cinder_vault_portal_state(|x, y, z| {
            !edits
                .sample(generator, VoxelCoord::new(x, y, z))
                .occludes_ambient()
        });
        let scope: DedicatedWorkerGlobalScope = js_sys::global().unchecked_into();
        let engine = Rc::new(Engine {
            renderer: RefCell::new(renderer),
            camera: RefCell::new(camera),
            input: RefCell::new(InputState::default()),
            generator,
            edits: RefCell::new(edits),
            scheduler: RefCell::new(scheduler),
            chunks: RefCell::new(BTreeMap::new()),
            pending_meshes: RefCell::new(BTreeMap::new()),
            surface_focus: Cell::new(None),
            surface_active_focus: Cell::new(None),
            surface_resident: RefCell::new(BTreeSet::new()),
            surface_revisions: RefCell::new(SurfaceRevisionCache::new()),
            surface_queue: RefCell::new(VecDeque::new()),
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
            cinder_portal_state: Cell::new(cinder_portal_state),
            cinder_portal_dirty: Cell::new(0),
            cinder_portal_revision: Cell::new(0),
            route_tour_index: Cell::new(0),
            cave_tour_index: Cell::new(0),
            landmark_tour_index: Cell::new(0),
            profile: RefCell::new(ProfileAutomation::default()),
            profile_tracked_high: Cell::new(0),
            profile_surface_high: Cell::new(0),
            profile_pending_high: Cell::new(0),
            profile_pending_mesh_high: Cell::new(0),
            profile_arena_capacity_high: Cell::new(0),
            profile_wasm_high: Cell::new(0),
            profile_start_evictions: Cell::new(0),
            last_persist: Cell::new(0.0),
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

    const fn voxel_coord(coord: [i32; 3]) -> VoxelCoord {
        VoxelCoord::new(coord[0], coord[1], coord[2])
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
    ) -> bool {
        let Some(focus) = focus else {
            return false;
        };
        let index = coord.level.index() as usize;
        let center = focus[index];
        let dx = (coord.x - center.x).abs();
        let dz = (coord.z - center.z).abs();
        dx.max(dz) <= SURFACE_LOAD_RADIUS_TILES[index]
    }
}

#[cfg(target_arch = "wasm32")]
pub use web::*;
