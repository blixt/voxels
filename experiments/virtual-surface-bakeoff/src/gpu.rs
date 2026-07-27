use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use serde_json::{Value, json};
use std::error::Error;
use std::sync::mpsc;
use voxels_world::{BakeoffCamera, BakeoffGpuQuad};
use wgpu::util::DeviceExt;

const WIDTH: u32 = 3_840;
const HEIGHT: u32 = 1_814;
const SHADOW_EDGE: u32 = 2_048;
const WARMUP_FRAMES: u32 = 8;
const SAMPLE_FRAMES: u32 = 40;
const QUERIES_PER_FRAME: u32 = 4;
const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

const SHADER: &str = r#"
struct Camera {
  view_projection: mat4x4<f32>,
  shadow_view_projection: mat4x4<f32>,
  eye_voxels: vec4<f32>,
};

struct Quad {
  address: vec4<i32>,
  extent: vec4<u32>,
};

struct VertexOut {
  @builtin(position) position: vec4<f32>,
  @location(0) @interpolate(flat) material_id: u32,
  @location(1) @interpolate(flat) normal: vec3<f32>,
  @location(2) world: vec3<f32>,
  @location(3) shadow_position: vec4<f32>,
};

@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var<storage, read> quads: array<Quad>;
@group(1) @binding(0) var shadow_depth: texture_depth_2d;
@group(1) @binding(1) var shadow_sampler: sampler_comparison;

const CORNERS = array<vec2<f32>, 6>(
  vec2<f32>(0.0, 0.0),
  vec2<f32>(1.0, 0.0),
  vec2<f32>(1.0, 1.0),
  vec2<f32>(0.0, 0.0),
  vec2<f32>(1.0, 1.0),
  vec2<f32>(0.0, 1.0),
);

@vertex
fn vs_main(
  @builtin(vertex_index) vertex_index: u32,
  @builtin(instance_index) instance_index: u32,
) -> VertexOut {
  let quad = quads[instance_index];
  let corner = CORNERS[vertex_index];
  let plane = f32(quad.address.y);
  let u = f32(quad.address.z) + corner.x * f32(quad.extent.x);
  let v = f32(quad.address.w) + corner.y * f32(quad.extent.y);
  var world = vec3<f32>(0.0);
  var normal = vec3<f32>(0.0);
  let sign = select(-1.0, 1.0, quad.extent.z != 0u);
  if quad.address.x == 0 {
    world = vec3<f32>(plane, u, v);
    normal.x = sign;
  } else if quad.address.x == 1 {
    world = vec3<f32>(u, plane, v);
    normal.y = sign;
  } else {
    world = vec3<f32>(u, v, plane);
    normal.z = sign;
  }
  var out: VertexOut;
  out.position = camera.view_projection * vec4<f32>(world - camera.eye_voxels.xyz, 1.0);
  out.material_id = quad.extent.w;
  out.normal = normal;
  out.world = world;
  out.shadow_position =
    camera.shadow_view_projection * vec4<f32>(world - camera.eye_voxels.xyz, 1.0);
  return out;
}

fn material_color(id: u32) -> vec3<f32> {
  let seed = vec3<u32>(
    id * 1664525u + 1013904223u,
    id * 22695477u + 1u,
    id * 1103515245u + 12345u,
  );
  return vec3<f32>(seed & vec3<u32>(255u)) / 255.0 * 0.55 + 0.08;
}

@fragment
fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
  let sun = normalize(vec3<f32>(-0.42, 0.78, -0.46));
  let view = normalize(camera.eye_voxels.xyz - input.world);
  let half_vector = normalize(sun + view);
  let diffuse = max(dot(input.normal, sun), 0.0);
  let shadow_ndc = input.shadow_position.xyz / input.shadow_position.w;
  let shadow_uv = vec2<f32>(shadow_ndc.x * 0.5 + 0.5, 0.5 - shadow_ndc.y * 0.5);
  let in_shadow_map = all(shadow_uv >= vec2<f32>(0.0))
    && all(shadow_uv <= vec2<f32>(1.0))
    && shadow_ndc.z >= 0.0
    && shadow_ndc.z <= 1.0;
  let shadow_visibility = select(
    1.0,
    textureSampleCompare(shadow_depth, shadow_sampler, shadow_uv, shadow_ndc.z - 0.00015),
    in_shadow_map,
  );
  let wet_base = material_color(input.material_id) * 0.72;
  let specular = pow(max(dot(input.normal, half_vector), 0.0), 48.0) * 0.65;
  let color = wet_base * (0.055 + diffuse * 0.28 * shadow_visibility)
    + vec3<f32>(0.24, 0.31, 0.45) * specular * shadow_visibility;
  return vec4<f32>(color, 1.0);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuCamera {
    view_projection: [f32; 16],
    shadow_view_projection: [f32; 16],
    eye_voxels: [f32; 4],
}

fn shadow_matrix() -> glam::Mat4 {
    let light_direction = Vec3::new(-0.42, -0.78, -0.46).normalize();
    let light_eye = -light_direction * 640.0;
    let view = glam::camera::rh::view::look_at_mat4(light_eye, Vec3::ZERO, Vec3::Y);
    let projection =
        glam::camera::rh::proj::directx::orthographic(-420.0, 420.0, -420.0, 420.0, 1.0, 1_280.0);
    projection * view
}

fn camera_uniform(camera: BakeoffCamera) -> GpuCamera {
    let eye = Vec3::from_array(camera.eye_voxels.map(|value| value as f32));
    let forward = Vec3::new(
        camera.yaw_radians.sin() as f32 * camera.pitch_radians.cos() as f32,
        camera.pitch_radians.sin() as f32,
        -camera.yaw_radians.cos() as f32 * camera.pitch_radians.cos() as f32,
    )
    .normalize();
    let projection = glam::camera::rh::proj::directx::perspective(
        camera.vertical_fov_radians as f32,
        camera.aspect_ratio as f32,
        0.05,
        4_096.0,
    );
    let view = glam::camera::rh::view::look_to_mat4(Vec3::ZERO, forward, Vec3::Y);
    GpuCamera {
        view_projection: (projection * view).to_cols_array(),
        shadow_view_projection: shadow_matrix().to_cols_array(),
        eye_voxels: [eye.x, eye.y, eye.z, 0.0],
    }
}

fn shadow_uniform(camera: BakeoffCamera) -> GpuCamera {
    let eye = Vec3::from_array(camera.eye_voxels.map(|value| value as f32));
    let matrix = shadow_matrix();
    GpuCamera {
        view_projection: matrix.to_cols_array(),
        shadow_view_projection: matrix.to_cols_array(),
        eye_voxels: [eye.x, eye.y, eye.z, 0.0],
    }
}

fn target(
    device: &wgpu::Device,
    label: &str,
    size: [u32; 2],
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

fn percentile(values: &[f64], quantile: f64) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let index = (quantile * sorted.len() as f64).ceil() as usize;
    sorted[index.saturating_sub(1).min(sorted.len().saturating_sub(1))]
}

pub async fn run(camera: BakeoffCamera, quads: &[BakeoffGpuQuad]) -> Result<Value, Box<dyn Error>> {
    if quads.is_empty() {
        return Err("GPU bake-off received no exact leaf quads".into());
    }
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::METAL,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            apply_limit_buckets: false,
        })
        .await?;
    if !adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
        return Err("selected Metal adapter has no timestamp queries".into());
    }
    let info = adapter.get_info();
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            required_features: wgpu::Features::TIMESTAMP_QUERY,
            required_limits: wgpu::Limits::default(),
            ..Default::default()
        })
        .await?;
    let timestamp_period = f64::from(queue.get_timestamp_period());
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("virtual surface raster bake-off shader"),
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });
    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("virtual surface raster bindings"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });
    let shadow_sample_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("virtual surface shadow sample bindings"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
            ],
        });
    let color_pipeline_layout =
        device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("virtual surface color pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout), Some(&shadow_sample_layout)],
            immediate_size: 0,
        });
    let shadow_pipeline_layout =
        device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("virtual surface shadow pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
    let depth_state = wgpu::DepthStencilState {
        format: DEPTH_FORMAT,
        depth_write_enabled: Some(true),
        depth_compare: Some(wgpu::CompareFunction::Less),
        stencil: wgpu::StencilState::default(),
        bias: wgpu::DepthBiasState::default(),
    };
    let color_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("virtual surface 4K wet night raster"),
        layout: Some(&color_pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: COLOR_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(depth_state.clone()),
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    });
    let shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("virtual surface shadow raster"),
        layout: Some(&shadow_pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: None,
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(depth_state),
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    });
    let quad_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("virtual surface exact clustered leaf quads"),
        contents: bytemuck::cast_slice(quads),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("virtual surface raster camera"),
        contents: bytemuck::bytes_of(&camera_uniform(camera)),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let shadow_camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("virtual surface shadow camera"),
        contents: bytemuck::bytes_of(&shadow_uniform(camera)),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let make_bind_group = |label, camera_buffer: &wgpu::Buffer| {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: camera_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: quad_buffer.as_entire_binding(),
                },
            ],
        })
    };
    let color_bind_group = make_bind_group("virtual surface raster bind group", &camera_buffer);
    let shadow_bind_group =
        make_bind_group("virtual surface shadow bind group", &shadow_camera_buffer);
    let color_target = target(
        &device,
        "virtual surface 4K color",
        [WIDTH, HEIGHT],
        COLOR_FORMAT,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let color_depth = target(
        &device,
        "virtual surface 4K depth",
        [WIDTH, HEIGHT],
        DEPTH_FORMAT,
        wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    let shadow_depth = target(
        &device,
        "virtual surface shadow depth",
        [SHADOW_EDGE, SHADOW_EDGE],
        DEPTH_FORMAT,
        wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
    );
    let color_view = color_target.create_view(&Default::default());
    let color_depth_view = color_depth.create_view(&Default::default());
    let shadow_depth_view = shadow_depth.create_view(&Default::default());
    let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("virtual surface shadow comparison sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        compare: Some(wgpu::CompareFunction::LessEqual),
        ..Default::default()
    });
    let shadow_sample_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("virtual surface shadow sample bind group"),
        layout: &shadow_sample_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&shadow_depth_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&shadow_sampler),
            },
        ],
    });

    let encode_frame = |encoder: &mut wgpu::CommandEncoder,
                        query_set: Option<&wgpu::QuerySet>,
                        first_query: u32| {
        {
            let timestamp_writes = query_set.map(|query_set| wgpu::RenderPassTimestampWrites {
                query_set,
                beginning_of_pass_write_index: Some(first_query),
                end_of_pass_write_index: Some(first_query + 1),
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("virtual surface shadow pass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &shadow_depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&shadow_pipeline);
            pass.set_bind_group(0, &shadow_bind_group, &[]);
            pass.draw(0..6, 0..u32::try_from(quads.len()).unwrap_or(u32::MAX));
        }
        {
            let timestamp_writes = query_set.map(|query_set| wgpu::RenderPassTimestampWrites {
                query_set,
                beginning_of_pass_write_index: Some(first_query + 2),
                end_of_pass_write_index: Some(first_query + 3),
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("virtual surface 4K wet night color pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &color_depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&color_pipeline);
            pass.set_bind_group(0, &color_bind_group, &[]);
            pass.set_bind_group(1, &shadow_sample_bind_group, &[]);
            pass.draw(0..6, 0..u32::try_from(quads.len()).unwrap_or(u32::MAX));
        }
    };
    for _ in 0..WARMUP_FRAMES {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("virtual surface raster warmup"),
        });
        encode_frame(&mut encoder, None, 0);
        queue.submit([encoder.finish()]);
    }
    device.poll(wgpu::PollType::wait_indefinitely())?;

    let query_count = SAMPLE_FRAMES * QUERIES_PER_FRAME;
    let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
        label: Some("virtual surface GPU timestamps"),
        ty: wgpu::QueryType::Timestamp,
        count: query_count,
    });
    let query_bytes = u64::from(query_count) * 8;
    let resolve = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("virtual surface timestamp resolve"),
        size: query_bytes,
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("virtual surface timestamp readback"),
        size: query_bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let color_bytes = u64::from(WIDTH) * u64::from(HEIGHT) * 4;
    let shadow_bytes = u64::from(SHADOW_EDGE) * u64::from(SHADOW_EDGE) * 4;
    let output_readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("virtual surface raster output verification"),
        size: color_bytes + shadow_bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("virtual surface timed raster frames"),
    });
    for frame in 0..SAMPLE_FRAMES {
        encode_frame(&mut encoder, Some(&query_set), frame * QUERIES_PER_FRAME);
    }
    // Keep the final measured pass from also being the command encoder's final pass.
    // On Metal the final end-of-pass counter sample can otherwise remain zero.
    encode_frame(&mut encoder, None, 0);
    encoder.copy_texture_to_buffer(
        color_target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &output_readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(WIDTH * 4),
                rows_per_image: Some(HEIGHT),
            },
        },
        wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
    );
    encoder.copy_texture_to_buffer(
        shadow_depth.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &output_readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: color_bytes,
                bytes_per_row: Some(SHADOW_EDGE * 4),
                rows_per_image: Some(SHADOW_EDGE),
            },
        },
        wgpu::Extent3d {
            width: SHADOW_EDGE,
            height: SHADOW_EDGE,
            depth_or_array_layers: 1,
        },
    );
    encoder.resolve_query_set(&query_set, 0..query_count, &resolve, 0);
    encoder.copy_buffer_to_buffer(&resolve, 0, &readback, 0, query_bytes);
    queue.submit([encoder.finish()]);
    let slice = readback.slice(..);
    let (sender, receiver) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    device.poll(wgpu::PollType::wait_indefinitely())?;
    receiver.recv()??;
    let mapped = slice.get_mapped_range()?;
    let timestamps = mapped
        .chunks_exact(8)
        .map(|bytes| {
            let mut raw = [0u8; 8];
            raw.copy_from_slice(bytes);
            u64::from_le_bytes(raw)
        })
        .collect::<Vec<_>>();
    drop(mapped);
    readback.unmap();
    let output_slice = output_readback.slice(..);
    let (output_sender, output_receiver) = mpsc::channel();
    output_slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = output_sender.send(result);
    });
    device.poll(wgpu::PollType::wait_indefinitely())?;
    output_receiver.recv()??;
    let output = output_slice.get_mapped_range()?;
    let color_end = usize::try_from(color_bytes)?;
    let primary_hit_pixels = output[..color_end]
        .chunks_exact(4)
        .filter(|pixel| pixel[0] != 0 || pixel[1] != 0 || pixel[2] != 0)
        .count();
    let mut shadow_hit_pixels = 0usize;
    let mut invalid_shadow_pixels = 0usize;
    for bytes in output[color_end..].chunks_exact(4) {
        let mut raw = [0u8; 4];
        raw.copy_from_slice(bytes);
        let depth = f32::from_le_bytes(raw);
        if !depth.is_finite() {
            invalid_shadow_pixels += 1;
        } else if depth < 0.999_999 {
            shadow_hit_pixels += 1;
        }
    }
    let output_hash = output.iter().fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    });
    drop(output);
    output_readback.unmap();
    if primary_hit_pixels == 0 || shadow_hit_pixels == 0 || invalid_shadow_pixels != 0 {
        return Err(format!(
            "cluster raster GPU output is invalid: {primary_hit_pixels} primary hits, \
             {shadow_hit_pixels} shadow hits, {invalid_shadow_pixels} invalid shadow samples"
        )
        .into());
    }
    let mut shadow_ms = Vec::with_capacity(SAMPLE_FRAMES as usize);
    let mut color_ms = Vec::with_capacity(SAMPLE_FRAMES as usize);
    let mut discarded_timestamp_frames = 0u32;
    for frame in 0..SAMPLE_FRAMES as usize {
        let base = frame * QUERIES_PER_FRAME as usize;
        let duration = |first: usize, last: usize| {
            timestamps[last]
                .checked_sub(timestamps[first])
                .filter(|ticks| *ticks != 0)
                .map(|ticks| ticks as f64 * timestamp_period / 1_000_000.0)
        };
        let Some((shadow, color)) = duration(base, base + 1)
            .zip(duration(base + 2, base + 3))
        else {
            discarded_timestamp_frames += 1;
            continue;
        };
        shadow_ms.push(shadow);
        color_ms.push(color);
    }
    if shadow_ms.len() < 32 {
        return Err(format!(
            "Metal supplied only {} valid timestamp frames out of {SAMPLE_FRAMES}",
            shadow_ms.len()
        )
        .into());
    }
    let total_ms = shadow_ms
        .iter()
        .zip(&color_ms)
        .map(|(shadow, color)| shadow + color)
        .collect::<Vec<_>>();
    let distribution = |values: &[f64]| {
        json!({
            "minimum": values.iter().copied().fold(f64::INFINITY, f64::min),
            "p50": percentile(values, 0.50),
            "p95": percentile(values, 0.95),
            "p99": percentile(values, 0.99),
            "maximum": values.iter().copied().fold(0.0, f64::max),
        })
    };
    Ok(json!({
        "schema": "voxels.virtual-surface-gpu-bakeoff.v1",
        "adapter": {
            "name": info.name,
            "deviceType": format!("{:?}", info.device_type),
            "backend": format!("{:?}", info.backend),
            "driver": info.driver,
            "driverInfo": info.driver_info,
            "timestampPeriodNanoseconds": timestamp_period,
        },
        "workload": {
            "pixelSize": [WIDTH, HEIGHT],
            "shadowSize": [SHADOW_EDGE, SHADOW_EDGE],
            "samples": SAMPLE_FRAMES,
            "measuredFrames": shadow_ms.len(),
            "discardedTimestampFrames": discarded_timestamp_frames,
            "warmupFrames": WARMUP_FRAMES,
            "quadCount": quads.len(),
            "triangleCountPerPass": quads.len() * 2,
            "passesPerFrame": 2,
            "state": "night-rain-wet-material-shadow-sampled",
        },
        "gpuMs": {
            "shadow": distribution(&shadow_ms),
            "color": distribution(&color_ms),
            "total": distribution(&total_ms),
        },
        "readbackVerification": {
            "primaryHitPixels": primary_hit_pixels,
            "shadowHitPixels": shadow_hit_pixels,
            "invalidShadowPixels": invalid_shadow_pixels,
            "fnv1a64": format!("{output_hash:016x}"),
        },
        "allocatedBytes": {
            "quadPayload": std::mem::size_of_val(quads),
            "colorTarget": u64::from(WIDTH) * u64::from(HEIGHT) * 4,
            "colorDepth": u64::from(WIDTH) * u64::from(HEIGHT) * 4,
            "shadowDepth": u64::from(SHADOW_EDGE) * u64::from(SHADOW_EDGE) * 4,
        },
    }))
}
