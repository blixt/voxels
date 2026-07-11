use bytemuck::{Pod, Zeroable};
use std::sync::Arc;
use voxels_core::CameraState;
use voxels_world::{ChunkCoord, Generator, mesh_chunk};
use wgpu::util::DeviceExt;
use wgpu::{
    Backends, BindGroup, Buffer, CurrentSurfaceTexture, Device, DeviceDescriptor, Instance,
    InstanceDescriptor, PowerPreference, PresentMode, Queue, RenderPipeline, RequestAdapterOptions,
    Surface, SurfaceConfiguration, TextureFormat, TextureUsages, TextureView,
};

const DEPTH_FORMAT: TextureFormat = TextureFormat::Depth32Float;
const WORLD_RADIUS_CHUNKS: i32 = 3;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FrameUniform {
    view_projection: [[f32; 4]; 4],
    camera_time: [f32; 4],
    viewport: [f32; 2],
    _pad: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuQuad {
    origin: [f32; 3],
    face: u32,
    extent: [f32; 2],
    material: u32,
}

const _: () = assert!(size_of::<GpuQuad>() == 28);

pub struct Renderer {
    surface: Surface<'static>,
    device: Device,
    queue: Queue,
    config: SurfaceConfiguration,
    sky_pipeline: RenderPipeline,
    voxel_pipeline: RenderPipeline,
    frame_buffer: Buffer,
    frame_bind_group: BindGroup,
    quad_buffer: Buffer,
    quad_count: u32,
    depth_view: TextureView,
    time: f32,
}

impl Renderer {
    pub async fn new(
        target: wgpu::SurfaceTarget<'static>,
        width: u32,
        height: u32,
        world_seed: u64,
        log_error: fn(&str),
    ) -> Result<Self, String> {
        let instance = Instance::new(InstanceDescriptor {
            backends: Backends::BROWSER_WEBGPU,
            ..InstanceDescriptor::new_without_display_handle()
        });
        let surface = instance
            .create_surface(target)
            .map_err(|error| format!("create_surface: {error:?}"))?;
        let adapter = instance
            .request_adapter(&RequestAdapterOptions {
                power_preference: PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
            })
            .await
            .map_err(|error| format!("request_adapter: {error:?}"))?;
        let (device, queue) = adapter
            .request_device(&DeviceDescriptor {
                required_limits: wgpu::Limits::default(),
                required_features: wgpu::Features::empty(),
                ..Default::default()
            })
            .await
            .map_err(|error| format!("request_device: {error:?}"))?;
        device.on_uncaptured_error(Arc::new(move |error| {
            log_error(&format!("wgpu validation: {error}"));
        }));
        let caps = surface.get_capabilities(&adapter);
        let format = preferred_format(&caps.formats);
        let config = SurfaceConfiguration {
            usage: TextureUsages::RENDER_ATTACHMENT,
            format,
            width: width.max(1),
            height: height.max(1),
            present_mode: PresentMode::Fifo,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let frame = frame_uniform(&config, &CameraState::default(), 0.0);
        let frame_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("frame uniform"),
            contents: bytemuck::bytes_of(&frame),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("frame layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let frame_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("frame bind group"),
            layout: &frame_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: frame_buffer.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("world pipeline layout"),
            bind_group_layouts: &[Some(&frame_layout)],
            immediate_size: 0,
        });
        let sky_shader = device.create_shader_module(wgpu::include_wgsl!("shaders/sky.wgsl"));
        let sky_pipeline = pipeline(
            &device,
            "sky pipeline",
            &pipeline_layout,
            &sky_shader,
            format,
            &[],
            Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
        );
        let voxel_shader = device.create_shader_module(wgpu::include_wgsl!("shaders/voxels.wgsl"));
        let voxel_pipeline = pipeline(
            &device,
            "voxel pipeline",
            &pipeline_layout,
            &voxel_shader,
            format,
            &[quad_layout()],
            Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
        );

        let quads = build_initial_world(world_seed);
        let quad_count = quads.len() as u32;
        let quad_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("world quad instances"),
            contents: bytemuck::cast_slice(&quads),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let depth_view = depth_view(&device, config.width, config.height);

        Ok(Self {
            surface,
            device,
            queue,
            config,
            sky_pipeline,
            voxel_pipeline,
            frame_buffer,
            frame_bind_group,
            quad_buffer,
            quad_count,
            depth_view,
            time: 0.0,
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.depth_view = depth_view(&self.device, width, height);
    }

    pub const fn quad_count(&self) -> u32 {
        self.quad_count
    }

    pub fn render(&mut self, dt: f32, camera: &CameraState) {
        self.time += dt.min(0.1);
        let uniform = frame_uniform(&self.config, camera, self.time);
        self.queue
            .write_buffer(&self.frame_buffer, 0, bytemuck::bytes_of(&uniform));
        let frame = match self.surface.get_current_texture() {
            CurrentSurfaceTexture::Success(frame) | CurrentSurfaceTexture::Suboptimal(frame) => {
                frame
            }
            CurrentSurfaceTexture::Outdated | CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            _ => return,
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("world pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.frame_bind_group, &[]);
            pass.set_pipeline(&self.sky_pipeline);
            pass.draw(0..3, 0..1);
            pass.set_pipeline(&self.voxel_pipeline);
            pass.set_vertex_buffer(0, self.quad_buffer.slice(..));
            pass.draw(0..6, 0..self.quad_count);
        }
        self.queue.submit([encoder.finish()]);
        frame.present();
    }
}

fn build_initial_world(seed: u64) -> Vec<GpuQuad> {
    let generator = Generator::new(seed);
    let mut gpu_quads = Vec::new();
    for chunk_z in -WORLD_RADIUS_CHUNKS..=WORLD_RADIUS_CHUNKS {
        for chunk_x in -WORLD_RADIUS_CHUNKS..=WORLD_RADIUS_CHUNKS {
            for chunk_y in 0..=1 {
                let coord = ChunkCoord::new(chunk_x, chunk_y, chunk_z);
                let chunk = generator.generate_chunk(coord);
                let world_origin = coord.world_origin();
                let quads = mesh_chunk(&chunk, |x, y, z| generator.sample(x, y, z));
                gpu_quads.extend(quads.into_iter().map(|quad| GpuQuad {
                    origin: [
                        (world_origin[0] + i32::from(quad.origin[0])) as f32,
                        (world_origin[1] + i32::from(quad.origin[1])) as f32,
                        (world_origin[2] + i32::from(quad.origin[2])) as f32,
                    ],
                    face: u32::from(quad.face),
                    extent: [f32::from(quad.extent[0]), f32::from(quad.extent[1])],
                    material: u32::from(quad.material),
                }));
            }
        }
    }
    gpu_quads
}

fn frame_uniform(config: &SurfaceConfiguration, camera: &CameraState, time: f32) -> FrameUniform {
    let aspect = config.width as f32 / config.height.max(1) as f32;
    let projection =
        glam::camera::rh::proj::directx::perspective(65.0f32.to_radians(), aspect, 0.1, 800.0);
    let view =
        glam::camera::rh::view::look_to_mat4(camera.position, camera.forward(), glam::Vec3::Y);
    FrameUniform {
        view_projection: (projection * view).to_cols_array_2d(),
        camera_time: [
            camera.position.x,
            camera.position.y,
            camera.position.z,
            time,
        ],
        viewport: [config.width as f32, config.height as f32],
        _pad: [0.0; 2],
    }
}

fn pipeline(
    device: &Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    format: TextureFormat,
    buffers: &[wgpu::VertexBufferLayout<'_>],
    depth_stencil: Option<wgpu::DepthStencilState>,
) -> RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            buffers,
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn quad_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRIBUTES: [wgpu::VertexAttribute; 4] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Uint32, 2 => Float32x2, 3 => Uint32];
    wgpu::VertexBufferLayout {
        array_stride: size_of::<GpuQuad>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &ATTRIBUTES,
    }
}

fn depth_view(device: &Device, width: u32, height: u32) -> TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("world depth"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

fn preferred_format(formats: &[TextureFormat]) -> TextureFormat {
    formats
        .iter()
        .copied()
        .find(|format| *format == TextureFormat::Bgra8Unorm)
        .or_else(|| {
            formats
                .iter()
                .copied()
                .find(|format| *format == TextureFormat::Rgba8Unorm)
        })
        .unwrap_or(formats[0])
}
