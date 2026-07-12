//! Browser/WASM leaf for Voxels. The worker owns the renderer, clock, input semantics, and persistence.

#[cfg(target_arch = "wasm32")]
mod persist;
#[cfg(target_arch = "wasm32")]
mod web {
    use crate::persist::Store;
    use bytemuck::{Pod, Zeroable};
    use glam::Vec2;
    use js_sys::Float32Array;
    use std::cell::{Cell, RefCell};
    use std::collections::{BTreeMap, BTreeSet, VecDeque};
    use std::rc::Rc;
    use voxels_core::{CameraState, InputState, VoxelHit, raycast_voxels};
    use voxels_render::renderer::Renderer;
    use voxels_render::ui::LiveStats;
    use voxels_runtime::{ChunkState, FrameBudget, StreamConfig, StreamScheduler};
    use voxels_world::{
        CHUNK_EDGE, Chunk, ChunkCoord, EditMap, Generator, Material, MeshedChunk, SurfaceLodLevel,
        SurfaceTileCoord, VOXEL_SIZE_METRES, VoxelCoord, generate_edited_surface_tile_mesh,
        generate_edited_water_tile_mesh, mesh_chunk, surface_tiles_affected_by_voxel,
    };
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;
    use web_sys::{DedicatedWorkerGlobalScope, OffscreenCanvas};

    const WORLD_SEED: u64 = 0x5eed_cafe;
    const SURFACE_LOAD_RADIUS_TILES: [i32; 4] = [4, 4, 4, 5];
    const SURFACE_RETAIN_MARGIN_TILES: i32 = 1;
    const SIMULATION_STEP_SECONDS: f32 = 1.0 / 120.0;
    const MAX_SIMULATION_STEPS_PER_FRAME: u32 = 6;
    const STREAM_FRAME_BUDGET: FrameBudget = FrameBudget {
        generation: 2,
        meshing: 2,
        upload: 3,
    };

    type FrameCallback = Closure<dyn FnMut(f64)>;

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

        fn frame(&self, time: f64) {
            let last = self.last_time.replace(time);
            let dt = if last <= 0.0 {
                1.0 / 60.0
            } else {
                ((time - last).max(0.0) / 1000.0) as f32
            };
            let frame_ms = dt * 1_000.0;
            let previous_frame_ms = self.frame_milliseconds.get();
            self.frame_milliseconds.set(if previous_frame_ms <= 0.0 {
                frame_ms
            } else {
                previous_frame_ms * 0.9 + frame_ms * 0.1
            });
            let mut camera = self.camera.borrow_mut();
            let edits = self.edits.borrow();
            let mut accumulator = (self.simulation_accumulator.get() + dt.min(0.1))
                .min(SIMULATION_STEP_SECONDS * MAX_SIMULATION_STEPS_PER_FRAME as f32);
            let mut steps = 0;
            while accumulator >= SIMULATION_STEP_SECONDS && steps < MAX_SIMULATION_STEPS_PER_FRAME {
                camera.update(
                    &self.input.borrow(),
                    SIMULATION_STEP_SECONDS,
                    VOXEL_SIZE_METRES,
                    |x, y, z| {
                        edits
                            .sample(self.generator, VoxelCoord::new(x, y, z))
                            .is_collidable()
                    },
                );
                accumulator -= SIMULATION_STEP_SECONDS;
                steps += 1;
            }
            self.simulation_accumulator.set(accumulator);
            drop(edits);
            self.stream_world(&camera);
            let target = self.raycast_target(&camera).map(|hit| hit.voxel);
            let mut renderer = self.renderer.borrow_mut();
            renderer.set_target_voxel(target);
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
            renderer.set_lod_coverage_ready(self.fine_initialized.get(), all_lods_ready);
            if all_lods_ready {
                let voxel_x = (camera.position.x / VOXEL_SIZE_METRES).floor() as i32;
                let voxel_z = (camera.position.z / VOXEL_SIZE_METRES).floor() as i32;
                renderer.set_geometric_lod_focus(voxel_x, voxel_z);
                self.surface_active_focus.set(self.surface_focus.get());
            }
            renderer.render(
                dt,
                &camera,
                LiveStats {
                    frames_per_second: if self.frame_milliseconds.get() > 0.0 {
                        1_000.0 / self.frame_milliseconds.get()
                    } else {
                        0.0
                    },
                    frame_ms: self.frame_milliseconds.get(),
                    cpu_ms: self.frame_milliseconds.get(),
                    gpu_ms: None,
                    resident_chunks: usize_to_u32(
                        stream.resident + self.surface_resident.borrow().len(),
                    ),
                    visible_chunks: render.visible_chunks,
                    quads: render.quads,
                    draw_calls: render.draw_calls,
                    shadow_draw_calls: render.shadow_draw_calls,
                    shadow_cascades: render.shadow_cascades,
                    load_p95_frames: stream.initial_residency_latency.p95_frames,
                    load_max_frames: stream.initial_residency_latency.max_frames,
                    remesh_p95_frames: stream.remesh_latency.p95_frames,
                    remesh_max_frames: stream.remesh_latency.max_frames,
                    lod_tiles,
                    pending_jobs: usize_to_u32(
                        stream.generation.queued
                            + stream.meshing.queued
                            + stream.upload.queued
                            + self.surface_queue.borrow().len()
                            + self.surface_dirty.borrow().len(),
                    ),
                    core_gpu_bytes: render.core_gpu_bytes,
                },
            );
            drop(renderer);
            if time - self.last_persist.get() >= 1_000.0 {
                if let Err(error) = self.store.borrow().save_camera(&camera) {
                    web_sys::console::error_1(&error);
                }
                self.last_persist.set(time);
            }
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
                let mut generated_columns = BTreeMap::new();
                let mesh = mesh_chunk(&chunk, |x, y, z| {
                    let coord = VoxelCoord::new(x, y, z);
                    edits.override_at(coord).unwrap_or_else(|| {
                        generated_columns
                            .entry((x, z))
                            .or_insert_with(|| self.generator.column(x, z))
                            .sample(y)
                    })
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
                    let mut dirty = self.surface_dirty.borrow_mut();
                    let mut renderer = self.renderer.borrow_mut();
                    for coord in evicted {
                        resident.remove(&coord);
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
                let mut candidates: Vec<_> = desired
                    .into_iter()
                    .filter(|coord| !resident.contains(coord))
                    .collect();
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
            let mesh = generate_edited_surface_tile_mesh(self.generator, &edits, coord);
            let water = generate_edited_water_tile_mesh(self.generator, &edits, coord);
            drop(edits);
            if self
                .renderer
                .borrow_mut()
                .upload_surface_tile_meshes(&mesh, &water)
            {
                self.surface_resident.borrow_mut().insert(coord);
                self.surface_dirty.borrow_mut().remove(&coord);
            } else if dirty.is_none() {
                self.surface_queue.borrow_mut().push_front(coord);
            }
        }

        fn stop(&self) {
            if let Err(error) = self.store.borrow().save_camera(&self.camera.borrow()) {
                web_sys::console::error_1(&error);
            }
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
            self.renderer.borrow().ui_open()
        }

        fn edit_target(&self, buttons: u16) {
            let camera = *self.camera.borrow();
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
                    Material::Grass,
                )
            } else {
                return;
            };

            let mut edits = self.edits.borrow_mut();
            edits.set(self.generator, target, material);
            if let Some(chunk) = self.chunks.borrow_mut().get_mut(&coord_key(target.chunk())) {
                let [x, y, z] = target.local();
                chunk.set(x, y, z, material);
            }
            let durable = edits.override_at(target);
            if let Err(error) = self.store.borrow().save_edit(target, durable) {
                web_sys::console::error_1(&error);
            }
            drop(edits);
            let _ = self.scheduler.borrow_mut().mark_voxel_edited(target);
            let resident = self.surface_resident.borrow();
            let queue = self.surface_queue.borrow();
            let mut dirty = self.surface_dirty.borrow_mut();
            for level in SurfaceLodLevel::ALL {
                dirty.extend(
                    surface_tiles_affected_by_voxel(self.generator, level, target)
                        .into_iter()
                        .filter(|coord| resident.contains(coord) || queue.contains(coord)),
                );
            }
        }

        fn raycast_target(&self, camera: &CameraState) -> Option<VoxelHit> {
            let edits = self.edits.borrow();
            let camera_voxel = VoxelCoord::new(
                (camera.position.x / VOXEL_SIZE_METRES).floor() as i32,
                (camera.position.y / VOXEL_SIZE_METRES).floor() as i32,
                (camera.position.z / VOXEL_SIZE_METRES).floor() as i32,
            );
            let ignore_water = edits.sample(self.generator, camera_voxel) == Material::Water;
            raycast_voxels(
                camera.position,
                camera.forward(),
                5.0,
                VOXEL_SIZE_METRES,
                |x, y, z| {
                    let material = edits.sample(self.generator, VoxelCoord::new(x, y, z));
                    material.is_collidable() || (!ignore_water && material == Material::Water)
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
            let values = self.engine.as_ref().map_or([0.0; 28], |engine| {
                let camera = engine.camera.borrow();
                let diagnostics = engine.scheduler.borrow().diagnostics();
                let render = engine.renderer.borrow().diagnostics();
                let lod_tiles = engine.surface_lod_counts();
                [
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
                ]
            });
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
        let store = Store::open(WORLD_SEED, voxels_world::generation::GENERATOR_VERSION).await?;
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
        let camera = store
            .load_camera()?
            .filter(|camera| {
                let voxel_x = (camera.position.x / VOXEL_SIZE_METRES).floor() as i32;
                let voxel_z = (camera.position.z / VOXEL_SIZE_METRES).floor() as i32;
                let surface = generator.surface_sample(voxel_x, voxel_z);
                let walkable_top = surface
                    .water_level
                    .unwrap_or(surface.height)
                    .max(surface.height);
                camera.position.y - voxels_core::PLAYER_EYE_HEIGHT_METRES
                    >= (walkable_top + 1) as f32 * VOXEL_SIZE_METRES
            })
            .unwrap_or_else(|| CameraState::spawn(glam::Vec3::new(0.0, spawn_y, spawn_z)));
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

    fn usize_to_u32(value: usize) -> u32 {
        u32::try_from(value).unwrap_or(u32::MAX)
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
}

#[cfg(target_arch = "wasm32")]
pub use web::*;
