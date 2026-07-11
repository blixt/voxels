use crate::arena::{Allocation, ArenaAllocator};
use bytemuck::{Pod, Zeroable};
use std::collections::BTreeMap;
use std::sync::Arc;
use voxels_core::CameraState;
use voxels_world::{
    CHUNK_EDGE, ChunkCoord, FAR_TILE_SPAN_VOXELS, FarTileCoord, Quad, SurfaceQuad,
    VOXEL_SIZE_METRES,
};
use wgpu::util::DeviceExt;
use wgpu::{
    Backends, BindGroup, Buffer, CurrentSurfaceTexture, Device, DeviceDescriptor, Instance,
    InstanceDescriptor, PowerPreference, PresentMode, Queue, RenderPipeline, RequestAdapterOptions,
    Surface, SurfaceConfiguration, TextureFormat, TextureUsages, TextureView,
};

const DEPTH_FORMAT: TextureFormat = TextureFormat::Depth32Float;
const VIEW_DISTANCE_METRES: f32 = 220.0;
const ARENA_PAGE_BYTES: u32 = 4 * 1024 * 1024;
const FAR_MATERIAL_FLAG: u32 = 1 << 31;
type MeshKey = (u8, i32, i32, i32);

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FrameUniform {
    view_projection: [[f32; 4]; 4],
    inverse_view_projection: [[f32; 4]; 4],
    camera_time: [f32; 4],
    viewport_voxel: [f32; 4],
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

struct ChunkMesh {
    allocation: Allocation,
    quad_count: u32,
    bounds_min: glam::Vec3,
    bounds_max: glam::Vec3,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RenderDiagnostics {
    pub resident_chunks: u32,
    pub visible_chunks: u32,
    pub draw_calls: u32,
    pub quads: u32,
    pub arena_pages: u32,
    pub arena_capacity_bytes: u64,
    pub arena_allocated_bytes: u64,
}

pub struct Renderer {
    surface: Surface<'static>,
    device: Device,
    queue: Queue,
    config: SurfaceConfiguration,
    sky_pipeline: RenderPipeline,
    voxel_pipeline: RenderPipeline,
    frame_buffer: Buffer,
    frame_bind_group: BindGroup,
    chunks: BTreeMap<MeshKey, ChunkMesh>,
    arena: ArenaAllocator,
    arena_buffers: Vec<Buffer>,
    depth_view: TextureView,
    time: f32,
    diagnostics: RenderDiagnostics,
}

impl Renderer {
    pub async fn new(
        target: wgpu::SurfaceTarget<'static>,
        width: u32,
        height: u32,
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
                apply_limit_buckets: false,
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
            color_space: wgpu::SurfaceColorSpace::Auto,
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
            &[Some(quad_layout())],
            Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
        );

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
            chunks: BTreeMap::new(),
            arena: ArenaAllocator::new(ARENA_PAGE_BYTES, 4),
            arena_buffers: Vec::new(),
            depth_view,
            time: 0.0,
            diagnostics: RenderDiagnostics::default(),
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

    pub fn quad_count(&self) -> u32 {
        self.chunks.values().map(|chunk| chunk.quad_count).sum()
    }

    pub const fn diagnostics(&self) -> RenderDiagnostics {
        self.diagnostics
    }

    pub fn upload_chunk(&mut self, coord: ChunkCoord, quads: &[Quad]) -> bool {
        let key = (0, coord.x, coord.y, coord.z);
        if quads.is_empty() {
            self.remove_chunk(coord);
            return true;
        }
        let origin = coord.world_origin();
        let gpu_quads: Vec<_> = quads
            .iter()
            .map(|quad| GpuQuad {
                origin: [
                    (origin[0] + i32::from(quad.origin[0])) as f32 * VOXEL_SIZE_METRES,
                    (origin[1] + i32::from(quad.origin[1])) as f32 * VOXEL_SIZE_METRES,
                    (origin[2] + i32::from(quad.origin[2])) as f32 * VOXEL_SIZE_METRES,
                ],
                face: u32::from(quad.face),
                extent: [
                    f32::from(quad.extent[0]) * VOXEL_SIZE_METRES,
                    f32::from(quad.extent[1]) * VOXEL_SIZE_METRES,
                ],
                material: u32::from(quad.material),
            })
            .collect();
        let min = glam::Vec3::from_array(origin.map(|value| value as f32 * VOXEL_SIZE_METRES));
        let max = min + glam::Vec3::splat(CHUNK_EDGE as f32 * VOXEL_SIZE_METRES);
        self.upload_mesh(key, &gpu_quads, min, max)
    }

    pub fn upload_far_tile(&mut self, coord: FarTileCoord, quads: &[SurfaceQuad]) -> bool {
        let key = (1, coord.x, 0, coord.z);
        if quads.is_empty() {
            self.remove_far_tile(coord);
            return true;
        }
        let gpu_quads: Vec<_> = quads
            .iter()
            .map(|quad| GpuQuad {
                origin: quad.origin.map(|value| value as f32 * VOXEL_SIZE_METRES),
                face: u32::from(quad.face),
                extent: [
                    f32::from(quad.extent[0]) * VOXEL_SIZE_METRES,
                    f32::from(quad.extent[1]) * VOXEL_SIZE_METRES,
                ],
                material: u32::from(quad.material.id()) | FAR_MATERIAL_FLAG,
            })
            .collect();
        let [x, z] = coord.voxel_origin();
        let min = glam::Vec3::new(
            x as f32 * VOXEL_SIZE_METRES,
            -6.4,
            z as f32 * VOXEL_SIZE_METRES,
        );
        let max = glam::Vec3::new(
            (x + FAR_TILE_SPAN_VOXELS) as f32 * VOXEL_SIZE_METRES,
            12.8,
            (z + FAR_TILE_SPAN_VOXELS) as f32 * VOXEL_SIZE_METRES,
        );
        self.upload_mesh(key, &gpu_quads, min, max)
    }

    fn upload_mesh(
        &mut self,
        key: MeshKey,
        gpu_quads: &[GpuQuad],
        bounds_min: glam::Vec3,
        bounds_max: glam::Vec3,
    ) -> bool {
        let bytes = bytemuck::cast_slice(gpu_quads);
        let Ok(byte_len) = u32::try_from(bytes.len()) else {
            return false;
        };
        let Some(allocation) = self.arena.allocate(byte_len) else {
            return false;
        };
        while self.arena_buffers.len() <= allocation.page as usize {
            let page = self.arena_buffers.len() as u16;
            let Some(capacity) = self.arena.page_capacity(page) else {
                let _ = self.arena.free(allocation);
                return false;
            };
            self.arena_buffers
                .push(self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("voxel mesh arena page"),
                    size: u64::from(capacity),
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }));
        }
        let Some(buffer) = self.arena_buffers.get(allocation.page as usize) else {
            let _ = self.arena.free(allocation);
            return false;
        };
        self.queue
            .write_buffer(buffer, u64::from(allocation.offset), bytes);
        let old = self.chunks.insert(
            key,
            ChunkMesh {
                allocation,
                quad_count: gpu_quads.len() as u32,
                bounds_min,
                bounds_max,
            },
        );
        if let Some(old) = old {
            let _ = self.arena.free(old.allocation);
        }
        true
    }

    pub fn remove_chunk(&mut self, coord: ChunkCoord) {
        self.remove_mesh((0, coord.x, coord.y, coord.z));
    }

    pub fn remove_far_tile(&mut self, coord: FarTileCoord) {
        self.remove_mesh((1, coord.x, 0, coord.z));
    }

    fn remove_mesh(&mut self, key: MeshKey) {
        if let Some(chunk) = self.chunks.remove(&key) {
            let _ = self.arena.free(chunk.allocation);
        }
    }

    pub fn render(&mut self, dt: f32, camera: &CameraState) {
        self.time += dt.min(0.1);
        let uniform = frame_uniform(&self.config, camera, self.time);
        let view_projection = glam::Mat4::from_cols_array_2d(&uniform.view_projection);
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
            let mut visible_chunks = 0;
            let mut visible_quads = 0;
            for chunk in self.chunks.values() {
                if !aabb_visible(chunk.bounds_min, chunk.bounds_max, view_projection) {
                    continue;
                }
                let Some(buffer) = self.arena_buffers.get(chunk.allocation.page as usize) else {
                    continue;
                };
                let start = u64::from(chunk.allocation.offset);
                let end = start + u64::from(chunk.allocation.size);
                pass.set_vertex_buffer(0, buffer.slice(start..end));
                pass.draw(0..6, 0..chunk.quad_count);
                visible_chunks += 1;
                visible_quads += chunk.quad_count;
            }
            let arena = self.arena.stats();
            self.diagnostics = RenderDiagnostics {
                resident_chunks: self.chunks.len() as u32,
                visible_chunks,
                draw_calls: visible_chunks,
                quads: visible_quads,
                arena_pages: arena.pages as u32,
                arena_capacity_bytes: arena.capacity_bytes,
                arena_allocated_bytes: arena.allocated_bytes,
            };
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
    }
}

fn frame_uniform(config: &SurfaceConfiguration, camera: &CameraState, time: f32) -> FrameUniform {
    let view_projection = view_projection(config, camera);
    FrameUniform {
        view_projection: view_projection.to_cols_array_2d(),
        inverse_view_projection: view_projection.inverse().to_cols_array_2d(),
        camera_time: [
            camera.position.x,
            camera.position.y,
            camera.position.z,
            time,
        ],
        viewport_voxel: [
            config.width as f32,
            config.height as f32,
            VOXEL_SIZE_METRES,
            VIEW_DISTANCE_METRES,
        ],
    }
}

fn view_projection(config: &SurfaceConfiguration, camera: &CameraState) -> glam::Mat4 {
    let aspect = config.width as f32 / config.height.max(1) as f32;
    let projection =
        glam::camera::rh::proj::directx::perspective(68.0f32.to_radians(), aspect, 0.01, 320.0);
    let view =
        glam::camera::rh::view::look_to_mat4(camera.position, camera.forward(), glam::Vec3::Y);
    projection * view
}

fn aabb_visible(min: glam::Vec3, max: glam::Vec3, view_projection: glam::Mat4) -> bool {
    let mut clips = [glam::Vec4::ZERO; 8];
    for (index, clip) in clips.iter_mut().enumerate() {
        let corner = glam::Vec3::new(
            if index & 1 == 0 { min.x } else { max.x },
            if index & 2 == 0 { min.y } else { max.y },
            if index & 4 == 0 { min.z } else { max.z },
        );
        *clip = view_projection * corner.extend(1.0);
    }
    !clips.iter().all(|value| value.x < -value.w)
        && !clips.iter().all(|value| value.x > value.w)
        && !clips.iter().all(|value| value.y < -value.w)
        && !clips.iter().all(|value| value.y > value.w)
        && !clips.iter().all(|value| value.z < 0.0)
        && !clips.iter().all(|value| value.z > value.w)
}

fn pipeline(
    device: &Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    format: TextureFormat,
    buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_view_projection(camera: &CameraState) -> glam::Mat4 {
        glam::camera::rh::proj::directx::perspective(68.0f32.to_radians(), 1.0, 0.01, 80.0)
            * glam::camera::rh::view::look_to_mat4(camera.position, camera.forward(), glam::Vec3::Y)
    }

    #[test]
    fn frustum_rejects_chunks_behind_camera_and_beyond_far_plane() {
        let camera = CameraState::spawn(glam::Vec3::new(0.0, 1.7, 0.0));
        let matrix = test_view_projection(&camera);
        let edge = CHUNK_EDGE as f32 * VOXEL_SIZE_METRES;
        let bounds = |coord: ChunkCoord| {
            let min = glam::Vec3::from_array(
                coord
                    .world_origin()
                    .map(|value| value as f32 * VOXEL_SIZE_METRES),
            );
            (min, min + glam::Vec3::splat(edge))
        };
        let (front_min, front_max) = bounds(ChunkCoord::new(0, 0, -1));
        let (back_min, back_max) = bounds(ChunkCoord::new(0, 0, 2));
        let (far_min, far_max) = bounds(ChunkCoord::new(0, 0, -120));
        assert!(aabb_visible(front_min, front_max, matrix));
        assert!(!aabb_visible(back_min, back_max, matrix));
        assert!(!aabb_visible(far_min, far_max, matrix));
    }
}
