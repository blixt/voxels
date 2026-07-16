//! Half-resolution spatial horizon ambient occlusion with edge-aware denoising.
//!
//! Geometry ownership is established by the renderer's depth prepass. This module then reconstructs
//! positions and geometric normals from that authoritative depth, avoiding a second normal target and
//! keeping canonical chunks, surface LODs, edits, and water independent of browser concerns.

use wgpu::{
    BindGroup, BindGroupLayout, Color, ColorTargetState, ColorWrites, CommandEncoder, Device,
    Extent3d, FragmentState, LoadOp, MultisampleState, Operations, PipelineLayoutDescriptor,
    PrimitiveState, RenderPassColorAttachment, RenderPassDescriptor, RenderPassTimestampWrites,
    RenderPipeline, RenderPipelineDescriptor, ShaderStages, StoreOp, Texture, TextureDescriptor,
    TextureDimension, TextureFormat, TextureSampleType, TextureUsages, TextureView,
    TextureViewDescriptor, TextureViewDimension, VertexState,
};

const AO_FORMAT: TextureFormat = TextureFormat::Rg16Float;

pub(crate) struct AmbientOcclusionGpu {
    _raw_texture: Texture,
    raw_view: TextureView,
    _filtered_texture: Texture,
    filtered_view: TextureView,
    width: u32,
    height: u32,
    depth_layout: BindGroupLayout,
    depth_bind_group: BindGroup,
    denoise_layout: BindGroupLayout,
    denoise_bind_group: BindGroup,
    sample_layout: BindGroupLayout,
    sample_bind_group: BindGroup,
    evaluate_pipeline: RenderPipeline,
    denoise_pipeline: RenderPipeline,
}

impl AmbientOcclusionGpu {
    pub(crate) fn new(
        device: &Device,
        frame_layout: &BindGroupLayout,
        depth_view: &TextureView,
        width: u32,
        height: u32,
    ) -> Self {
        let depth_layout = depth_layout(device);
        let denoise_layout = ao_layout(device, "spatial AO denoise layout", 1);
        let sample_layout = ao_layout(device, "spatial AO world sample layout", 0);
        let shader = crate::shader::frame_shader(
            device,
            "spatial ambient occlusion shader",
            include_str!("shaders/ambient_occlusion.wgsl"),
        );
        let evaluate_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("spatial AO evaluate pipeline layout"),
            bind_group_layouts: &[Some(frame_layout), Some(&depth_layout)],
            immediate_size: 0,
        });
        let denoise_pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("spatial AO denoise pipeline layout"),
            bind_group_layouts: &[Some(frame_layout), Some(&denoise_layout)],
            immediate_size: 0,
        });
        let evaluate_pipeline = screen_pipeline(
            device,
            "spatial AO evaluate pipeline",
            &evaluate_layout,
            &shader,
            "fs_evaluate",
        );
        let denoise_pipeline = screen_pipeline(
            device,
            "spatial AO denoise pipeline",
            &denoise_pipeline_layout,
            &shader,
            "fs_denoise",
        );
        let targets = targets(device, width, height);
        let depth_bind_group = depth_bind_group(device, &depth_layout, depth_view);
        let denoise_bind_group = ao_bind_group(
            device,
            "spatial AO denoise bind group",
            &denoise_layout,
            1,
            &targets.raw_view,
        );
        let sample_bind_group = ao_bind_group(
            device,
            "spatial AO world sample bind group",
            &sample_layout,
            0,
            &targets.filtered_view,
        );
        Self {
            _raw_texture: targets.raw_texture,
            raw_view: targets.raw_view,
            _filtered_texture: targets.filtered_texture,
            filtered_view: targets.filtered_view,
            width: targets.width,
            height: targets.height,
            depth_layout,
            depth_bind_group,
            denoise_layout,
            denoise_bind_group,
            sample_layout,
            sample_bind_group,
            evaluate_pipeline,
            denoise_pipeline,
        }
    }

    pub(crate) const fn sample_layout(&self) -> &BindGroupLayout {
        &self.sample_layout
    }

    pub(crate) const fn sample_bind_group(&self) -> &BindGroup {
        &self.sample_bind_group
    }

    pub(crate) fn resize(
        &mut self,
        device: &Device,
        depth_view: &TextureView,
        width: u32,
        height: u32,
    ) {
        let targets = targets(device, width, height);
        self._raw_texture = targets.raw_texture;
        self.raw_view = targets.raw_view;
        self._filtered_texture = targets.filtered_texture;
        self.filtered_view = targets.filtered_view;
        self.width = targets.width;
        self.height = targets.height;
        self.depth_bind_group = depth_bind_group(device, &self.depth_layout, depth_view);
        self.denoise_bind_group = ao_bind_group(
            device,
            "spatial AO denoise bind group",
            &self.denoise_layout,
            1,
            &self.raw_view,
        );
        self.sample_bind_group = ao_bind_group(
            device,
            "spatial AO world sample bind group",
            &self.sample_layout,
            0,
            &self.filtered_view,
        );
    }

    pub(crate) const fn bytes(&self) -> u64 {
        self.width as u64 * self.height as u64 * 8
    }

    pub(crate) fn evaluate<'query>(
        &self,
        encoder: &mut CommandEncoder,
        frame_bind_group: &BindGroup,
        timestamp_writes: Option<RenderPassTimestampWrites<'query>>,
    ) {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("half-resolution spatial AO pass"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &self.raw_view,
                resolve_target: None,
                depth_slice: None,
                ops: Operations {
                    load: LoadOp::Clear(Color::WHITE),
                    store: StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.evaluate_pipeline);
        pass.set_bind_group(0, frame_bind_group, &[]);
        pass.set_bind_group(1, &self.depth_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    pub(crate) fn denoise<'query>(
        &self,
        encoder: &mut CommandEncoder,
        frame_bind_group: &BindGroup,
        timestamp_writes: Option<RenderPassTimestampWrites<'query>>,
    ) {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("depth-aware spatial AO denoise pass"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &self.filtered_view,
                resolve_target: None,
                depth_slice: None,
                ops: Operations {
                    load: LoadOp::Clear(Color::WHITE),
                    store: StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.denoise_pipeline);
        pass.set_bind_group(0, frame_bind_group, &[]);
        pass.set_bind_group(1, &self.denoise_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

struct Targets {
    raw_texture: Texture,
    raw_view: TextureView,
    filtered_texture: Texture,
    filtered_view: TextureView,
    width: u32,
    height: u32,
}

const fn half_extent(value: u32) -> u32 {
    if value <= 1 { 1 } else { value / 2 + value % 2 }
}

fn targets(device: &Device, width: u32, height: u32) -> Targets {
    let width = half_extent(width);
    let height = half_extent(height);
    let make = |label| {
        device.create_texture(&TextureDescriptor {
            label: Some(label),
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: AO_FORMAT,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
    };
    let raw_texture = make("half-resolution raw spatial AO");
    let raw_view = raw_texture.create_view(&TextureViewDescriptor::default());
    let filtered_texture = make("half-resolution filtered spatial AO");
    let filtered_view = filtered_texture.create_view(&TextureViewDescriptor::default());
    Targets {
        raw_texture,
        raw_view,
        filtered_texture,
        filtered_view,
        width,
        height,
    }
}

fn depth_layout(device: &Device) -> BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("spatial AO depth layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: TextureSampleType::Depth,
                view_dimension: TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        }],
    })
}

fn ao_layout(device: &Device, label: &str, binding: u32) -> BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: TextureSampleType::Float { filterable: false },
                view_dimension: TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        }],
    })
}

fn depth_bind_group(
    device: &Device,
    layout: &BindGroupLayout,
    depth_view: &TextureView,
) -> BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("spatial AO depth bind group"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(depth_view),
        }],
    })
}

fn ao_bind_group(
    device: &Device,
    label: &str,
    layout: &BindGroupLayout,
    binding: u32,
    ao_view: &TextureView,
) -> BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::TextureView(ao_view),
        }],
    })
}

fn screen_pipeline(
    device: &Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    fragment_entry: &str,
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
            entry_point: Some(fragment_entry),
            targets: &[Some(ColorTargetState {
                format: AO_FORMAT,
                blend: None,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_resolution_rounds_up_and_never_becomes_zero() {
        assert_eq!(half_extent(0), 1);
        assert_eq!(half_extent(1), 1);
        assert_eq!(half_extent(2), 1);
        assert_eq!(half_extent(3), 2);
        assert_eq!(half_extent(1_279), 640);
        assert_eq!(half_extent(1_280), 640);
    }

    #[test]
    fn two_rg16_targets_have_exact_bounded_size() {
        let bytes = u64::from(half_extent(1_280)) * u64::from(half_extent(720)) * 8;
        assert_eq!(bytes, 1_843_200);
    }
}
