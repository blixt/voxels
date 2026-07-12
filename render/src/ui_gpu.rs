//! WGPU resources for the Rust-owned screen UI.
//!
//! The world is rendered into [`SceneTarget`]. A second pass presents that backdrop, draws every
//! glass surface in one instanced call, then draws the glyph atlas. Layout and input remain in
//! [`crate::ui`]; this module is deliberately only their GPU backend.

use crate::ui::{GlassSurface, SurfaceRole, TextAlign, TextRun, UiDrawList};
use bytemuck::{Pod, Zeroable};
use wgpu::{
    AddressMode, BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout,
    BindGroupLayoutDescriptor, BindGroupLayoutEntry, BindingResource, BindingType, BlendState,
    Buffer, BufferBindingType, BufferDescriptor, BufferUsages, ColorTargetState, ColorWrites,
    CommandEncoder, Device, Extent3d, FilterMode, FragmentState, MipmapFilterMode,
    MultisampleState, PipelineLayoutDescriptor, PrimitiveState, Queue, RenderPass, RenderPipeline,
    RenderPipelineDescriptor, Sampler, SamplerBindingType, SamplerDescriptor, ShaderStages,
    Texture, TextureDescriptor, TextureDimension, TextureFormat, TextureSampleType, TextureUsages,
    TextureView, TextureViewDescriptor, TextureViewDimension, VertexState,
};
use wgpu_text::glyph_brush::ab_glyph::FontArc;
use wgpu_text::glyph_brush::{HorizontalAlign, Layout, Section, Text, VerticalAlign};
use wgpu_text::{BrushBuilder, TextBrush};

const GLASS_CAPACITY: usize = 192;
pub(crate) const SCENE_FORMAT: TextureFormat = TextureFormat::Rgba16Float;

pub struct SceneTarget {
    texture: Texture,
    view: TextureView,
    sampler: Sampler,
    width: u32,
    height: u32,
}

impl SceneTarget {
    fn new(device: &Device, width: u32, height: u32) -> Self {
        let width = width.max(1);
        let height = height.max(1);
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("voxel scene backdrop"),
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: SCENE_FORMAT,
            usage: TextureUsages::RENDER_ATTACHMENT
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_SRC
                | TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&TextureViewDescriptor::default());
        let sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("voxel scene backdrop sampler"),
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            address_mode_w: AddressMode::ClampToEdge,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: MipmapFilterMode::Nearest,
            ..Default::default()
        });
        Self {
            texture,
            view,
            sampler,
            width,
            height,
        }
    }

    pub const fn view(&self) -> &TextureView {
        &self.view
    }

    fn copy_to(&self, encoder: &mut CommandEncoder, destination: &Self) {
        debug_assert_eq!(
            (self.width, self.height),
            (destination.width, destination.height)
        );
        encoder.copy_texture_to_texture(
            self.texture.as_image_copy(),
            destination.texture.as_image_copy(),
            Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
    }

    fn resize(&mut self, device: &Device, width: u32, height: u32) -> bool {
        let size = (width.max(1), height.max(1));
        if size == (self.width, self.height) {
            return false;
        }
        *self = Self::new(device, size.0, size.1);
        true
    }
}

struct PresentPipeline {
    pipeline: RenderPipeline,
    layout: BindGroupLayout,
    bind_group: BindGroup,
}

impl PresentPipeline {
    fn new(device: &Device, format: TextureFormat, target: &SceneTarget) -> Self {
        let layout = texture_sampler_layout(device, "scene present layout");
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("scene present pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/ui_present.wgsl"));
        let pipeline = screen_pipeline(
            device,
            "scene present pipeline",
            &pipeline_layout,
            &shader,
            format,
            None,
        );
        let bind_group = texture_sampler_bind_group(
            device,
            "scene present bind group",
            &layout,
            &target.view,
            &target.sampler,
        );
        Self {
            pipeline,
            layout,
            bind_group,
        }
    }

    fn rebind(&mut self, device: &Device, target: &SceneTarget) {
        self.bind_group = texture_sampler_bind_group(
            device,
            "scene present bind group",
            &self.layout,
            &target.view,
            &target.sampler,
        );
    }

    fn draw<'pass>(&'pass self, pass: &mut RenderPass<'pass>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GlassInstance {
    rect: [f32; 4],
    viewport_radius: [f32; 4],
    fill: [f32; 4],
    border: [f32; 4],
    style: [f32; 4],
}

impl GlassInstance {
    fn from_surface(surface: &GlassSurface, viewport: [f32; 2], dpr: f32) -> Self {
        let style = match surface.role {
            SurfaceRole::Hover => 0.0,
            SurfaceRole::Crosshair => 4.0,
            SurfaceRole::Panel | SurfaceRole::ContextMenu => 1.0,
            SurfaceRole::ToggleThumb => 3.0,
            SurfaceRole::Brand
            | SurfaceRole::Launcher
            | SurfaceRole::Toast
            | SurfaceRole::Header
            | SurfaceRole::Button
            | SurfaceRole::StatCard
            | SurfaceRole::FeatureRow
            | SurfaceRole::ToggleTrack
            | SurfaceRole::ContextRow => 2.0,
        };
        Self {
            rect: [
                surface.rect.x * dpr,
                surface.rect.y * dpr,
                surface.rect.width * dpr,
                surface.rect.height * dpr,
            ],
            viewport_radius: [
                viewport[0],
                viewport[1],
                surface.radius * dpr,
                surface.blur_radius * dpr,
            ],
            fill: surface.fill.0,
            border: surface.border.0,
            style: [style, dpr, 0.0, 0.0],
        }
    }
}

struct GlassPipeline {
    pipeline: RenderPipeline,
    instances: Buffer,
    instance_bind_group: BindGroup,
    backdrop_layout: BindGroupLayout,
    backdrop_bind_group: BindGroup,
}

impl GlassPipeline {
    fn new(device: &Device, format: TextureFormat, target: &SceneTarget) -> Self {
        let instance_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("glass instance layout"),
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX_FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let backdrop_layout = texture_sampler_layout(device, "glass backdrop layout");
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("glass pipeline layout"),
            bind_group_layouts: &[Some(&instance_layout), Some(&backdrop_layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/ui_glass.wgsl"));
        let pipeline = screen_pipeline(
            device,
            "glass pipeline",
            &pipeline_layout,
            &shader,
            format,
            Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        );
        let instances = device.create_buffer(&BufferDescriptor {
            label: Some("glass instances"),
            size: (GLASS_CAPACITY * size_of::<GlassInstance>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let instance_bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("glass instance bind group"),
            layout: &instance_layout,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: instances.as_entire_binding(),
            }],
        });
        let backdrop_bind_group = texture_sampler_bind_group(
            device,
            "glass backdrop bind group",
            &backdrop_layout,
            &target.view,
            &target.sampler,
        );
        Self {
            pipeline,
            instances,
            instance_bind_group,
            backdrop_layout,
            backdrop_bind_group,
        }
    }

    fn rebind(&mut self, device: &Device, target: &SceneTarget) {
        self.backdrop_bind_group = texture_sampler_bind_group(
            device,
            "glass backdrop bind group",
            &self.backdrop_layout,
            &target.view,
            &target.sampler,
        );
    }

    fn upload(&self, queue: &Queue, surfaces: &[GlassSurface], viewport: [f32; 2], dpr: f32) {
        let instances = surfaces
            .iter()
            .take(GLASS_CAPACITY)
            .map(|surface| GlassInstance::from_surface(surface, viewport, dpr))
            .collect::<Vec<_>>();
        if !instances.is_empty() {
            queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&instances));
        }
    }

    fn draw<'pass>(&'pass self, pass: &mut RenderPass<'pass>, count: usize) {
        let count = count.min(GLASS_CAPACITY) as u32;
        if count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.instance_bind_group, &[]);
        pass.set_bind_group(1, &self.backdrop_bind_group, &[]);
        pass.draw(0..6, 0..count);
    }
}

struct TextEngine {
    brush: TextBrush<FontArc>,
    dpr: f32,
    width: u32,
    height: u32,
}

impl TextEngine {
    fn new(
        device: &Device,
        format: TextureFormat,
        width: u32,
        height: u32,
        dpr: f32,
    ) -> Result<Self, String> {
        let font = FontArc::try_from_slice(include_bytes!("ui-font.ttf"))
            .map_err(|error| format!("embedded UI font: {error}"))?;
        let brush =
            BrushBuilder::using_font(font).build(device, width.max(1), height.max(1), format);
        Ok(Self {
            brush,
            dpr: valid_dpr(dpr),
            width: width.max(1),
            height: height.max(1),
        })
    }

    fn resize(&mut self, queue: &Queue, width: u32, height: u32, dpr: f32) {
        self.dpr = valid_dpr(dpr);
        let size = (width.max(1), height.max(1));
        if size != (self.width, self.height) {
            self.width = size.0;
            self.height = size.1;
            self.brush
                .resize_view(self.width as f32, self.height as f32, queue);
        }
    }

    fn queue(&mut self, device: &Device, queue: &Queue, runs: &[TextRun]) -> Result<(), String> {
        let dpr = self.dpr;
        let sections = runs
            .iter()
            .map(|run| {
                let horizontal = match run.align {
                    TextAlign::Left => HorizontalAlign::Left,
                    TextAlign::Center => HorizontalAlign::Center,
                    TextAlign::Right => HorizontalAlign::Right,
                };
                Section::default()
                    .with_screen_position((run.position[0] * dpr, run.position[1] * dpr))
                    .with_layout(
                        Layout::default_single_line()
                            .h_align(horizontal)
                            .v_align(VerticalAlign::Center),
                    )
                    .add_text(
                        Text::new(&run.text)
                            .with_scale(run.size * dpr)
                            .with_color(run.color.0),
                    )
            })
            .collect::<Vec<_>>();
        self.brush
            .queue(device, queue, sections)
            .map_err(|error| format!("UI glyph atlas: {error}"))
    }

    fn draw<'pass>(&'pass self, pass: &mut RenderPass<'pass>) {
        self.brush.draw(pass);
    }
}

pub struct UiGpu {
    opaque_scene: SceneTarget,
    scene: SceneTarget,
    present: PresentPipeline,
    glass: GlassPipeline,
    text: TextEngine,
    width: u32,
    height: u32,
    dpr: f32,
    glass_count: usize,
}

impl UiGpu {
    pub fn new(
        device: &Device,
        format: TextureFormat,
        width: u32,
        height: u32,
        dpr: f32,
    ) -> Result<Self, String> {
        let opaque_scene = SceneTarget::new(device, width, height);
        let scene = SceneTarget::new(device, width, height);
        let present = PresentPipeline::new(device, format, &scene);
        let glass = GlassPipeline::new(device, format, &scene);
        let text = TextEngine::new(device, format, width, height, dpr)?;
        Ok(Self {
            opaque_scene,
            scene,
            present,
            glass,
            text,
            width: width.max(1),
            height: height.max(1),
            dpr: valid_dpr(dpr),
            glass_count: 0,
        })
    }

    pub const fn scene_view(&self) -> &TextureView {
        self.scene.view()
    }

    pub const fn opaque_scene_view(&self) -> &TextureView {
        self.opaque_scene.view()
    }

    pub fn copy_opaque_to_scene(&self, encoder: &mut CommandEncoder) {
        self.opaque_scene.copy_to(encoder, &self.scene);
    }

    pub fn refraction_bind_group(&self, device: &Device, layout: &BindGroupLayout) -> BindGroup {
        texture_sampler_bind_group(
            device,
            "water refraction scene bind group",
            layout,
            &self.opaque_scene.view,
            &self.opaque_scene.sampler,
        )
    }

    pub fn resize(
        &mut self,
        device: &Device,
        queue: &Queue,
        _format: TextureFormat,
        width: u32,
        height: u32,
        dpr: f32,
    ) -> bool {
        self.width = width.max(1);
        self.height = height.max(1);
        self.dpr = valid_dpr(dpr);
        let opaque_resized = self.opaque_scene.resize(device, self.width, self.height);
        let scene_resized = self.scene.resize(device, self.width, self.height);
        if scene_resized {
            self.present.rebind(device, &self.scene);
            self.glass.rebind(device, &self.scene);
        }
        self.text.resize(queue, self.width, self.height, self.dpr);
        opaque_resized || scene_resized
    }

    pub fn prepare(
        &mut self,
        device: &Device,
        queue: &Queue,
        draw: &UiDrawList,
    ) -> Result<(), String> {
        self.glass_count = draw.glass.len().min(GLASS_CAPACITY);
        self.glass.upload(
            queue,
            &draw.glass,
            [self.width as f32, self.height as f32],
            self.dpr,
        );
        self.text.queue(device, queue, &draw.text)
    }

    pub fn draw<'pass>(&'pass self, pass: &mut RenderPass<'pass>) {
        self.present.draw(pass);
        self.glass.draw(pass, self.glass_count);
        self.text.draw(pass);
    }
}

fn valid_dpr(dpr: f32) -> f32 {
    if dpr.is_finite() && dpr > 0.0 {
        dpr
    } else {
        1.0
    }
}

pub(crate) fn texture_sampler_layout(device: &Device, label: &str) -> BindGroupLayout {
    device.create_bind_group_layout(&BindGroupLayoutDescriptor {
        label: Some(label),
        entries: &[
            BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Texture {
                    sample_type: TextureSampleType::Float { filterable: true },
                    view_dimension: TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 1,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Sampler(SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

fn texture_sampler_bind_group(
    device: &Device,
    label: &str,
    layout: &BindGroupLayout,
    view: &TextureView,
    sampler: &Sampler,
) -> BindGroup {
    device.create_bind_group(&BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[
            BindGroupEntry {
                binding: 0,
                resource: BindingResource::TextureView(view),
            },
            BindGroupEntry {
                binding: 1,
                resource: BindingResource::Sampler(sampler),
            },
        ],
    })
}

fn screen_pipeline(
    device: &Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    format: TextureFormat,
    blend: Option<BlendState>,
) -> RenderPipeline {
    device.create_render_pipeline(&RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            targets: &[Some(ColorTargetState {
                format,
                blend,
                write_mask: ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: PrimitiveState::default(),
        depth_stencil: None,
        multisample: MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}
