//! Browser/WASM leaf for Voxels. The worker owns the renderer, clock, input semantics, and persistence.

#[cfg(target_arch = "wasm32")]
mod web {
    use bytemuck::{Pod, Zeroable};
    use glam::Vec2;
    use js_sys::Float32Array;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use voxels_core::CameraState;
    use voxels_render::renderer::Renderer;
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;
    use web_sys::{DedicatedWorkerGlobalScope, OffscreenCanvas};

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

    struct Engine {
        renderer: RefCell<Renderer>,
        camera: RefCell<CameraState>,
        scope: DedicatedWorkerGlobalScope,
        callback: RefCell<Option<FrameCallback>>,
        frame_id: Cell<i32>,
        last_time: Cell<f64>,
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
            self.renderer.borrow_mut().render(dt);
            if let Err(error) = self.request_frame() {
                web_sys::console::error_1(&error);
                self.stopped.set(true);
            }
        }

        fn stop(&self) {
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
                if record.kind == KIND_POINTER_MOVE {
                    self.camera
                        .borrow_mut()
                        .look(Vec2::new(record.dx, record.dy));
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
            let values = self.engine.as_ref().map_or([0.0; 5], |engine| {
                let camera = engine.camera.borrow();
                [
                    camera.position.x,
                    camera.position.y,
                    camera.position.z,
                    camera.yaw,
                    camera.pitch,
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
        let renderer = Renderer::new(wgpu::SurfaceTarget::OffscreenCanvas(canvas), width, height)
            .await
            .map_err(|error| JsValue::from_str(&error))?;
        let scope: DedicatedWorkerGlobalScope = js_sys::global().unchecked_into();
        let engine = Rc::new(Engine {
            renderer: RefCell::new(renderer),
            camera: RefCell::new(CameraState::default()),
            scope,
            callback: RefCell::new(None),
            frame_id: Cell::new(0),
            last_time: Cell::new(0.0),
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
