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
    use std::rc::Rc;
    use voxels_core::{CameraState, InputState};
    use voxels_render::renderer::Renderer;
    use voxels_world::{Generator, VOXEL_SIZE_METRES};
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;
    use web_sys::{DedicatedWorkerGlobalScope, OffscreenCanvas};

    const WORLD_SEED: u64 = 0x5eed_cafe;

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
        store: RefCell<Store>,
        scope: DedicatedWorkerGlobalScope,
        callback: RefCell<Option<FrameCallback>>,
        frame_id: Cell<i32>,
        last_time: Cell<f64>,
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
            let mut camera = self.camera.borrow_mut();
            camera.update(&self.input.borrow(), dt, VOXEL_SIZE_METRES, |x, y, z| {
                self.generator.sample(x, y, z).is_solid()
            });
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
            let values = self.engine.as_ref().map_or([0.0; 7], |engine| {
                let camera = engine.camera.borrow();
                [
                    camera.position.x,
                    camera.position.y,
                    camera.position.z,
                    camera.yaw,
                    camera.pitch,
                    if camera.grounded { 1.0 } else { 0.0 },
                    engine.renderer.borrow().quad_count() as f32,
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
        let spawn_z = 5.2;
        let spawn_voxel_z = (spawn_z / VOXEL_SIZE_METRES).floor() as i32;
        let spawn_y = (generator.surface_height(0, spawn_voxel_z) + 1) as f32 * VOXEL_SIZE_METRES
            + voxels_core::PLAYER_EYE_HEIGHT_METRES
            + 0.02;
        let camera = store
            .load_camera()?
            .unwrap_or_else(|| CameraState::spawn(glam::Vec3::new(0.0, spawn_y, spawn_z)));
        let renderer = Renderer::new(
            wgpu::SurfaceTarget::OffscreenCanvas(canvas),
            width,
            height,
            WORLD_SEED,
            log_gpu_error,
        )
        .await
        .map_err(|error| JsValue::from_str(&error))?;
        let scope: DedicatedWorkerGlobalScope = js_sys::global().unchecked_into();
        let engine = Rc::new(Engine {
            renderer: RefCell::new(renderer),
            camera: RefCell::new(camera),
            input: RefCell::new(InputState::default()),
            generator,
            store: RefCell::new(store),
            scope,
            callback: RefCell::new(None),
            frame_id: Cell::new(0),
            last_time: Cell::new(0.0),
            last_persist: Cell::new(0.0),
            stopped: Cell::new(false),
        });
        engine.start()?;
        Ok(EngineHandle {
            engine: Some(engine),
        })
    }
}

#[cfg(target_arch = "wasm32")]
pub use web::*;
