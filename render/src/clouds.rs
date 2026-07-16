//! Bounded half-resolution volumetric clouds with full-resolution depth composition.

use bytemuck::{Pod, Zeroable};
use std::mem::size_of;
use wgpu::util::DeviceExt;
use wgpu::{
    AddressMode, BindGroup, BindGroupLayout, BlendComponent, BlendFactor, BlendOperation,
    BlendState, Buffer, BufferBindingType, BufferSize, BufferUsages, Color, ColorTargetState,
    ColorWrites, CommandEncoder, CompareFunction, DepthBiasState, DepthStencilState, Device,
    Extent3d, FilterMode, FragmentState, LoadOp, MipmapFilterMode, MultisampleState, Operations,
    Origin3d, PipelineLayoutDescriptor, PrimitiveState, Queue, RenderPassColorAttachment,
    RenderPassDepthStencilAttachment, RenderPassDescriptor, RenderPassTimestampWrites,
    RenderPipeline, RenderPipelineDescriptor, Sampler, SamplerBindingType, SamplerDescriptor,
    ShaderStages, StoreOp, TexelCopyBufferLayout, TexelCopyTextureInfo, Texture, TextureAspect,
    TextureDescriptor, TextureDimension, TextureFormat, TextureSampleType, TextureUsages,
    TextureView, TextureViewDescriptor, TextureViewDimension, VertexState,
};

use crate::environment::{OutdoorEnvironment, WorldEnvironmentState};

const CLOUD_FORMAT: TextureFormat = TextureFormat::Rgba16Float;
const NOISE_EDGE: u32 = 64;
const HARD_MAX_VIEW_STEPS: u32 = 24;
const HARD_MAX_LIGHT_STEPS: u32 = 4;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VolumetricCloudConfig {
    pub enabled: bool,
    pub resolution_scale: f32,
    pub view_steps: u32,
    pub light_steps: u32,
    pub max_distance_metres: f32,
    pub extinction: f32,
}

impl Default for VolumetricCloudConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            resolution_scale: 0.5,
            view_steps: 14,
            light_steps: 2,
            max_distance_metres: 14_000.0,
            extinction: 0.006,
        }
    }
}

impl VolumetricCloudConfig {
    pub fn sanitized(self) -> Self {
        let fallback = Self::default();
        Self {
            enabled: self.enabled,
            resolution_scale: finite_or(self.resolution_scale, fallback.resolution_scale)
                .clamp(0.25, 1.0),
            view_steps: self.view_steps.clamp(4, HARD_MAX_VIEW_STEPS),
            light_steps: self.light_steps.clamp(1, HARD_MAX_LIGHT_STEPS),
            max_distance_metres: finite_or(self.max_distance_metres, fallback.max_distance_metres)
                .clamp(1_000.0, 40_000.0),
            extinction: finite_or(self.extinction, fallback.extinction).clamp(0.0001, 0.1),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CloudUniform {
    target_size: [f32; 4],
    layer: [f32; 4],
    quality: [u32; 4],
    shaping: [f32; 4],
}

const _: () = assert!(size_of::<CloudUniform>() == 64);

pub(crate) struct VolumetricCloudGpu {
    target: Texture,
    target_view: TextureView,
    target_sampler: Sampler,
    noise: Texture,
    uniform: Buffer,
    trace_bind_group: BindGroup,
    composite_layout: BindGroupLayout,
    composite_bind_group: BindGroup,
    trace_pipeline: RenderPipeline,
    composite_pipeline: RenderPipeline,
    width: u32,
    height: u32,
    noise_seed: u64,
    config: VolumetricCloudConfig,
}

impl VolumetricCloudGpu {
    pub(crate) fn new(
        device: &Device,
        queue: &Queue,
        frame_layout: &BindGroupLayout,
        scene_format: TextureFormat,
        depth_format: TextureFormat,
        width: u32,
        height: u32,
        config: VolumetricCloudConfig,
    ) -> Self {
        let config = config.sanitized();
        let (target, target_view, width, height) =
            cloud_target(device, width, height, config.resolution_scale);
        let target_sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("volumetric cloud target sampler"),
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            address_mode_w: AddressMode::ClampToEdge,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let noise = noise_texture(device);
        let noise_view = noise.create_view(&TextureViewDescriptor {
            label: Some("volumetric cloud noise view"),
            dimension: Some(TextureViewDimension::D3),
            ..Default::default()
        });
        let noise_sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("volumetric cloud noise sampler"),
            address_mode_u: AddressMode::Repeat,
            address_mode_v: AddressMode::Repeat,
            address_mode_w: AddressMode::Repeat,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: MipmapFilterMode::Nearest,
            ..Default::default()
        });
        write_noise(queue, &noise, 0);
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("volumetric cloud uniform"),
            contents: bytemuck::bytes_of(&CloudUniform {
                target_size: target_size(width, height),
                layer: [550.0, 1_800.0, config.max_distance_metres, config.extinction],
                quality: [
                    config.view_steps,
                    config.light_steps,
                    u32::from(config.enabled),
                    0,
                ],
                shaping: [1.0 / 4_800.0, 1.0 / 1_200.0, 0.4, 0.0],
            }),
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        });
        let trace_layout = trace_layout(device);
        let trace_bind_group =
            trace_bind_group(device, &trace_layout, &uniform, &noise_view, &noise_sampler);
        let composite_layout = composite_layout(device);
        let composite_bind_group =
            composite_bind_group(device, &composite_layout, &target_view, &target_sampler);
        let shader = crate::shader::frame_shader(
            device,
            "volumetric cloud shader",
            include_str!("shaders/clouds.wgsl"),
        );
        let trace_pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("volumetric cloud trace pipeline layout"),
            bind_group_layouts: &[Some(frame_layout), Some(&trace_layout)],
            immediate_size: 0,
        });
        let composite_pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("volumetric cloud composite pipeline layout"),
            bind_group_layouts: &[Some(frame_layout), None, Some(&composite_layout)],
            immediate_size: 0,
        });
        let trace_pipeline = screen_pipeline(
            device,
            "volumetric cloud trace pipeline",
            &trace_pipeline_layout,
            &shader,
            "vs_trace",
            "fs_trace",
            CLOUD_FORMAT,
            None,
            None,
        );
        let composite_pipeline = screen_pipeline(
            device,
            "volumetric cloud composite pipeline",
            &composite_pipeline_layout,
            &shader,
            "vs_composite",
            "fs_composite",
            scene_format,
            Some(BlendState {
                color: BlendComponent {
                    src_factor: BlendFactor::One,
                    dst_factor: BlendFactor::OneMinusSrcAlpha,
                    operation: BlendOperation::Add,
                },
                alpha: BlendComponent {
                    src_factor: BlendFactor::One,
                    dst_factor: BlendFactor::OneMinusSrcAlpha,
                    operation: BlendOperation::Add,
                },
            }),
            Some(DepthStencilState {
                format: depth_format,
                depth_write_enabled: Some(false),
                depth_compare: Some(CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: DepthBiasState::default(),
            }),
        );
        Self {
            target,
            target_view,
            target_sampler,
            noise,
            uniform,
            trace_bind_group,
            composite_layout,
            composite_bind_group,
            trace_pipeline,
            composite_pipeline,
            width,
            height,
            noise_seed: 0,
            config,
        }
    }

    pub(crate) fn resize(&mut self, device: &Device, width: u32, height: u32) {
        let (target, target_view, width, height) =
            cloud_target(device, width, height, self.config.resolution_scale);
        self.target = target;
        self.target_view = target_view;
        self.width = width;
        self.height = height;
        self.composite_bind_group = composite_bind_group(
            device,
            &self.composite_layout,
            &self.target_view,
            &self.target_sampler,
        );
    }

    pub(crate) fn update(
        &mut self,
        queue: &Queue,
        state: WorldEnvironmentState,
        environment: OutdoorEnvironment,
    ) {
        if self.noise_seed != state.weather_seed {
            write_noise(queue, &self.noise, state.weather_seed);
            self.noise_seed = state.weather_seed;
        }
        let uniform = CloudUniform {
            target_size: target_size(self.width, self.height),
            layer: [
                state.cloud_base_metres,
                state.cloud_top_metres,
                self.config.max_distance_metres,
                self.config.extinction,
            ],
            quality: [
                ((10.0 + (self.config.view_steps.saturating_sub(10)) as f32
                    * environment.cloud_density)
                    .round() as u32)
                    .clamp(8, self.config.view_steps),
                if environment.cloud_density > 0.54 {
                    self.config.light_steps
                } else {
                    1
                },
                u32::from(self.config.enabled),
                0,
            ],
            shaping: [
                1.0 / 4_800.0,
                1.0 / 1_200.0,
                environment.cloud_density,
                environment.storminess,
            ],
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));
    }

    pub(crate) fn trace<'query>(
        &self,
        encoder: &mut CommandEncoder,
        frame_bind_group: &BindGroup,
        timestamp_writes: Option<RenderPassTimestampWrites<'query>>,
    ) {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("half-resolution volumetric cloud trace"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &self.target_view,
                resolve_target: None,
                depth_slice: None,
                ops: Operations {
                    load: LoadOp::Clear(Color::TRANSPARENT),
                    store: StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.trace_pipeline);
        pass.set_bind_group(0, frame_bind_group, &[]);
        pass.set_bind_group(1, &self.trace_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    pub(crate) fn composite<'query>(
        &self,
        encoder: &mut CommandEncoder,
        frame_bind_group: &BindGroup,
        scene_view: &TextureView,
        depth_view: &TextureView,
        depth_store: StoreOp,
        timestamp_writes: Option<RenderPassTimestampWrites<'query>>,
    ) {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("full-resolution volumetric cloud composite"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: scene_view,
                resolve_target: None,
                depth_slice: None,
                ops: Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(Operations {
                    load: LoadOp::Load,
                    store: depth_store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.composite_pipeline);
        pass.set_bind_group(0, frame_bind_group, &[]);
        pass.set_bind_group(2, &self.composite_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    pub(crate) const fn enabled(&self) -> bool {
        self.config.enabled
    }

    pub(crate) const fn bytes(&self) -> u64 {
        self.width as u64 * self.height as u64 * 8
            + (NOISE_EDGE as u64 * NOISE_EDGE as u64 * NOISE_EDGE as u64)
            + size_of::<CloudUniform>() as u64
    }

    pub(crate) const fn resolution(&self) -> [u32; 2] {
        [self.width, self.height]
    }

    pub(crate) const fn quality(&self) -> [u32; 2] {
        [self.config.view_steps, self.config.light_steps]
    }
}

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

fn scaled_extent(value: u32, scale: f32) -> u32 {
    ((value.max(1) as f32 * scale).ceil() as u32).max(1)
}

fn target_size(width: u32, height: u32) -> [f32; 4] {
    [
        width as f32,
        height as f32,
        1.0 / width.max(1) as f32,
        1.0 / height.max(1) as f32,
    ]
}

fn cloud_target(
    device: &Device,
    width: u32,
    height: u32,
    scale: f32,
) -> (Texture, TextureView, u32, u32) {
    let width = scaled_extent(width, scale);
    let height = scaled_extent(height, scale);
    let target = device.create_texture(&TextureDescriptor {
        label: Some("half-resolution volumetric cloud target"),
        size: Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: CLOUD_FORMAT,
        usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let target_view = target.create_view(&TextureViewDescriptor::default());
    (target, target_view, width, height)
}

fn noise_texture(device: &Device) -> Texture {
    device.create_texture(&TextureDescriptor {
        label: Some("volumetric cloud periodic 3D noise"),
        size: Extent3d {
            width: NOISE_EDGE,
            height: NOISE_EDGE,
            depth_or_array_layers: NOISE_EDGE,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D3,
        format: TextureFormat::R8Unorm,
        usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn noise_bytes(seed: u64) -> Vec<u8> {
    let mut bytes = vec![0; (NOISE_EDGE * NOISE_EDGE * NOISE_EDGE) as usize];
    let seed = seed as u32 ^ (seed >> 32) as u32;
    for z in 0..NOISE_EDGE {
        for y in 0..NOISE_EDGE {
            for x in 0..NOISE_EDGE {
                let mut hash = x.wrapping_mul(0x9e37_79b1)
                    ^ y.wrapping_mul(0x85eb_ca77)
                    ^ z.wrapping_mul(0xc2b2_ae3d)
                    ^ seed;
                hash ^= hash >> 16;
                hash = hash.wrapping_mul(0x7feb_352d);
                hash ^= hash >> 15;
                hash = hash.wrapping_mul(0x846c_a68b);
                hash ^= hash >> 16;
                bytes[(x + NOISE_EDGE * (y + NOISE_EDGE * z)) as usize] = (hash >> 24) as u8;
            }
        }
    }
    bytes
}

fn write_noise(queue: &Queue, texture: &Texture, seed: u64) {
    queue.write_texture(
        TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: Origin3d::ZERO,
            aspect: TextureAspect::All,
        },
        &noise_bytes(seed),
        TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(NOISE_EDGE),
            rows_per_image: Some(NOISE_EDGE),
        },
        Extent3d {
            width: NOISE_EDGE,
            height: NOISE_EDGE,
            depth_or_array_layers: NOISE_EDGE,
        },
    );
}

fn trace_layout(device: &Device) -> BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("volumetric cloud trace layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: BufferSize::new(size_of::<CloudUniform>() as u64),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: TextureSampleType::Float { filterable: true },
                    view_dimension: TextureViewDimension::D3,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

fn trace_bind_group(
    device: &Device,
    layout: &BindGroupLayout,
    uniform: &Buffer,
    noise_view: &TextureView,
    noise_sampler: &Sampler,
) -> BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("volumetric cloud trace bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(noise_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(noise_sampler),
            },
        ],
    })
}

fn composite_layout(device: &Device) -> BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("volumetric cloud composite layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: TextureSampleType::Float { filterable: true },
                    view_dimension: TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

fn composite_bind_group(
    device: &Device,
    layout: &BindGroupLayout,
    target_view: &TextureView,
    target_sampler: &Sampler,
) -> BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("volumetric cloud composite bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(target_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(target_sampler),
            },
        ],
    })
}

#[allow(clippy::too_many_arguments)]
fn screen_pipeline(
    device: &Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    vertex_entry: &str,
    fragment_entry: &str,
    format: TextureFormat,
    blend: Option<BlendState>,
    depth_stencil: Option<DepthStencilState>,
) -> RenderPipeline {
    device.create_render_pipeline(&RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: VertexState {
            module: shader,
            entry_point: Some(vertex_entry),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(FragmentState {
            module: shader,
            entry_point: Some(fragment_entry),
            targets: &[Some(ColorTargetState {
                format,
                blend,
                write_mask: ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: PrimitiveState::default(),
        depth_stencil,
        multisample: MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaled_cloud_extent_is_bounded_and_rounds_up() {
        assert_eq!(scaled_extent(1, 0.5), 1);
        assert_eq!(scaled_extent(1_279, 0.5), 640);
        assert_eq!(scaled_extent(1_280, 0.5), 640);
    }

    #[test]
    fn seeded_noise_is_deterministic_and_uses_the_full_byte_range() {
        let first = noise_bytes(7);
        let second = noise_bytes(7);
        let different = noise_bytes(8);
        assert_eq!(first, second);
        assert_ne!(first, different);
        assert_eq!(first.iter().copied().min(), Some(0));
        assert_eq!(first.iter().copied().max(), Some(255));
    }

    #[test]
    fn invalid_quality_falls_back_or_clamps_to_shader_limits() {
        let config = VolumetricCloudConfig {
            resolution_scale: f32::NAN,
            view_steps: u32::MAX,
            light_steps: 0,
            max_distance_metres: f32::INFINITY,
            extinction: -1.0,
            ..VolumetricCloudConfig::default()
        }
        .sanitized();
        assert_eq!(config.resolution_scale, 0.5);
        assert_eq!(config.view_steps, HARD_MAX_VIEW_STEPS);
        assert_eq!(config.light_steps, 1);
        assert_eq!(config.max_distance_metres, 14_000.0);
        assert_eq!(config.extinction, 0.0001);
    }
}
