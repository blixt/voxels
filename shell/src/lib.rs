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
    use voxels_core::{CameraState, InputState, raycast_voxels};
    use voxels_render::renderer::Renderer;
    use voxels_runtime::{FrameBudget, StreamConfig, StreamScheduler};
    use voxels_world::{
        CHUNK_EDGE, Chunk, ChunkCoord, EditMap, FAR_TILE_SPAN_VOXELS, FarTileCoord, Generator,
        Material, Quad, VOXEL_SIZE_METRES, VoxelCoord, generate_far_tile_with, mesh_chunk,
    };
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;
    use web_sys::{DedicatedWorkerGlobalScope, OffscreenCanvas};

    const WORLD_SEED: u64 = 0x5eed_cafe;
    const FAR_LOAD_RADIUS_TILES: i32 = 5;
    const FAR_RETAIN_RADIUS_TILES: i32 = 6;
    const SIMULATION_STEP_SECONDS: f32 = 1.0 / 120.0;
    const MAX_SIMULATION_STEPS_PER_FRAME: u32 = 6;

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
        pending_meshes: RefCell<BTreeMap<(i32, i32, i32), Vec<Quad>>>,
        far_focus: Cell<Option<FarTileCoord>>,
        far_resident: RefCell<BTreeSet<(i32, i32)>>,
        far_queue: RefCell<VecDeque<FarTileCoord>>,
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
                            .is_solid()
                    },
                );
                accumulator -= SIMULATION_STEP_SECONDS;
                steps += 1;
            }
            self.simulation_accumulator.set(accumulator);
            drop(edits);
            self.stream_world(&camera);
            self.renderer.borrow_mut().render(dt, &camera);
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
                scheduler.schedule_frame(FrameBudget {
                    generation: 1,
                    meshing: 2,
                    upload: 3,
                })
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
                let quads = mesh_chunk(&chunk, |x, y, z| {
                    edits.sample(self.generator, VoxelCoord::new(x, y, z))
                });
                drop(edits);
                self.pending_meshes
                    .borrow_mut()
                    .insert(coord_key(ticket.coord), quads);
                let _ = self.scheduler.borrow_mut().complete(ticket);
            }
            for ticket in work.upload {
                let quads = self
                    .pending_meshes
                    .borrow_mut()
                    .remove(&coord_key(ticket.coord));
                let Some(quads) = quads else {
                    continue;
                };
                if self
                    .renderer
                    .borrow_mut()
                    .upload_chunk(ticket.coord, &quads)
                {
                    let _ = self.scheduler.borrow_mut().complete(ticket);
                } else {
                    self.pending_meshes
                        .borrow_mut()
                        .insert(coord_key(ticket.coord), quads);
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
                    renderer.remove_chunk(eviction.coord);
                }
            }
            self.stream_far(camera.position);
        }

        fn stream_far(&self, position: glam::Vec3) {
            let focus = world_to_far_tile(position);
            if self.far_focus.get() != Some(focus) {
                self.far_focus.set(Some(focus));
                let desired: BTreeSet<_> = (-FAR_LOAD_RADIUS_TILES..=FAR_LOAD_RADIUS_TILES)
                    .flat_map(|dz| {
                        (-FAR_LOAD_RADIUS_TILES..=FAR_LOAD_RADIUS_TILES).map(move |dx| (dx, dz))
                    })
                    .filter(|(dx, dz)| {
                        dx * dx + dz * dz <= FAR_LOAD_RADIUS_TILES * FAR_LOAD_RADIUS_TILES
                    })
                    .map(|(dx, dz)| (focus.x + dx, focus.z + dz))
                    .collect();
                let evicted: Vec<_> = self
                    .far_resident
                    .borrow()
                    .iter()
                    .copied()
                    .filter(|(x, z)| {
                        let dx = x - focus.x;
                        let dz = z - focus.z;
                        dx * dx + dz * dz > FAR_RETAIN_RADIUS_TILES * FAR_RETAIN_RADIUS_TILES
                    })
                    .collect();
                if !evicted.is_empty() {
                    let mut resident = self.far_resident.borrow_mut();
                    let mut renderer = self.renderer.borrow_mut();
                    for (x, z) in evicted {
                        resident.remove(&(x, z));
                        renderer.remove_far_tile(FarTileCoord::new(x, z));
                    }
                }
                let resident = self.far_resident.borrow();
                let mut candidates: Vec<_> = desired
                    .into_iter()
                    .filter(|coord| !resident.contains(coord))
                    .map(|(x, z)| FarTileCoord::new(x, z))
                    .collect();
                drop(resident);
                candidates.sort_by_key(|coord| {
                    let dx = coord.x - focus.x;
                    let dz = coord.z - focus.z;
                    (dx * dx + dz * dz, coord.z, coord.x)
                });
                let mut queue = self.far_queue.borrow_mut();
                queue.clear();
                queue.extend(candidates);
            }

            let next = self.far_queue.borrow_mut().pop_front();
            let Some(coord) = next else {
                return;
            };
            let edits = self.edits.borrow();
            let quads =
                generate_far_tile_with(coord, |x, z| edits.surface_sample(self.generator, x, z));
            drop(edits);
            if self.renderer.borrow_mut().upload_far_tile(coord, &quads) {
                self.far_resident.borrow_mut().insert((coord.x, coord.z));
            } else {
                self.far_queue.borrow_mut().push_front(coord);
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

        fn feed_input(&self, bytes: &[u8]) {
            for chunk in bytes.chunks_exact(INPUT_RECORD_SIZE) {
                let record = bytemuck::pod_read_unaligned::<InputRecord>(chunk);
                match record.kind {
                    KIND_POINTER_DOWN => self.edit_target(record.buttons),
                    KIND_POINTER_MOVE => self
                        .camera
                        .borrow_mut()
                        .look(Vec2::new(record.dx, record.dy)),
                    KIND_KEY_DOWN => self.input.borrow_mut().set_key(record.code, true),
                    KIND_KEY_UP => self.input.borrow_mut().set_key(record.code, false),
                    KIND_CANCEL => self.input.borrow_mut().clear(),
                    _ => {}
                }
            }
        }

        fn edit_target(&self, buttons: u16) {
            let camera = *self.camera.borrow();
            let hit = {
                let edits = self.edits.borrow();
                raycast_voxels(
                    camera.position,
                    camera.forward(),
                    5.0,
                    VOXEL_SIZE_METRES,
                    |x, y, z| {
                        edits
                            .sample(self.generator, VoxelCoord::new(x, y, z))
                            .is_solid()
                    },
                )
            };
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
            let far_coord = FarTileCoord::new(
                target.x.div_euclid(FAR_TILE_SPAN_VOXELS),
                target.z.div_euclid(FAR_TILE_SPAN_VOXELS),
            );
            if self
                .far_resident
                .borrow_mut()
                .remove(&(far_coord.x, far_coord.z))
            {
                self.renderer.borrow_mut().remove_far_tile(far_coord);
            }
            let mut queue = self.far_queue.borrow_mut();
            queue.retain(|queued| *queued != far_coord);
            queue.push_front(far_coord);
        }
    }

    #[wasm_bindgen]
    pub struct EngineHandle {
        engine: Option<Rc<Engine>>,
    }

    #[wasm_bindgen]
    impl EngineHandle {
        pub fn feed_input(&self, bytes: &[u8]) {
            if let Some(engine) = self.engine.as_ref() {
                engine.feed_input(bytes);
            }
        }

        pub fn resize(&self, css_width: f32, css_height: f32, dpr: f32) {
            if let Some(engine) = self.engine.as_ref() {
                let width = (css_width * dpr).round().max(1.0) as u32;
                let height = (css_height * dpr).round().max(1.0) as u32;
                engine.renderer.borrow_mut().resize(width, height);
            }
        }

        pub fn snapshot(&self) -> Float32Array {
            let values = self.engine.as_ref().map_or([0.0; 18], |engine| {
                let camera = engine.camera.borrow();
                let diagnostics = engine.scheduler.borrow().diagnostics();
                let render = engine.renderer.borrow().diagnostics();
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
                        + engine.far_queue.borrow().len()) as f32,
                    engine.far_resident.borrow().len() as f32,
                    engine.frame_milliseconds.get(),
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
    ) -> Result<EngineHandle, JsValue> {
        console_error_panic_hook::set_once();
        let width = (css_width * dpr).round().max(1.0) as u32;
        let height = (css_height * dpr).round().max(1.0) as u32;
        let generator = Generator::new(WORLD_SEED);
        let store = Store::open(WORLD_SEED, voxels_world::generation::GENERATOR_VERSION).await?;
        let edits = store.load_edits()?;
        let spawn_z = 5.2;
        let spawn_voxel_z = (spawn_z / VOXEL_SIZE_METRES).floor() as i32;
        let spawn_y = (generator.surface_height(0, spawn_voxel_z) + 1) as f32 * VOXEL_SIZE_METRES
            + voxels_core::PLAYER_EYE_HEIGHT_METRES
            + 0.02;
        let camera = store
            .load_camera()?
            .filter(|camera| {
                let voxel_x = (camera.position.x / VOXEL_SIZE_METRES).floor() as i32;
                let voxel_z = (camera.position.z / VOXEL_SIZE_METRES).floor() as i32;
                let terrain_top =
                    (generator.surface_height(voxel_x, voxel_z) + 1) as f32 * VOXEL_SIZE_METRES;
                camera.position.y - voxels_core::PLAYER_EYE_HEIGHT_METRES >= terrain_top
            })
            .unwrap_or_else(|| CameraState::spawn(glam::Vec3::new(0.0, spawn_y, spawn_z)));
        let renderer = Renderer::new(
            wgpu::SurfaceTarget::OffscreenCanvas(canvas),
            width,
            height,
            log_gpu_error,
        )
        .await
        .map_err(|error| JsValue::from_str(&error))?;
        let scheduler = StreamScheduler::new(StreamConfig {
            load_radius_chunks: 3,
            vertical_radius_chunks: 1,
            retention_margin_chunks: 1,
            max_tracked_chunks: 128,
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
            far_focus: Cell::new(None),
            far_resident: RefCell::new(BTreeSet::new()),
            far_queue: RefCell::new(VecDeque::new()),
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

    fn world_to_chunk(position: glam::Vec3) -> ChunkCoord {
        let edge_metres = CHUNK_EDGE as f32 * VOXEL_SIZE_METRES;
        ChunkCoord::new(
            (position.x / edge_metres).floor() as i32,
            (position.y / edge_metres).floor() as i32,
            (position.z / edge_metres).floor() as i32,
        )
    }

    fn world_to_far_tile(position: glam::Vec3) -> FarTileCoord {
        let tile_metres = FAR_TILE_SPAN_VOXELS as f32 * VOXEL_SIZE_METRES;
        FarTileCoord::new(
            (position.x / tile_metres).floor() as i32,
            (position.z / tile_metres).floor() as i32,
        )
    }
}

#[cfg(target_arch = "wasm32")]
pub use web::*;
