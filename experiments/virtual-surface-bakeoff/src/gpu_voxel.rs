use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use serde_json::{Value, json};
use std::error::Error;
use std::sync::mpsc;
use voxels_world::{BakeoffCamera, BakeoffVolume};
use wgpu::util::DeviceExt;

const WIDTH: u32 = 3_840;
const HEIGHT: u32 = 1_814;
const SHADOW_EDGE: u32 = 2_048;
const WARMUP_FRAMES: u32 = 8;
const SAMPLE_FRAMES: u32 = 40;
const QUERIES_PER_FRAME: u32 = 4;
const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const SHADOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;

const SHADER: &str = r#"
struct RayUniform {
  eye: vec4<f32>,
  forward_tan_half_fov: vec4<f32>,
  right_aspect: vec4<f32>,
  up: vec4<f32>,
  bounds_min: vec4<i32>,
  shape: vec4<u32>,
  sun_direction: vec4<f32>,
  shadow_center_extent: vec4<f32>,
  shadow_right: vec4<f32>,
  shadow_up: vec4<f32>,
  shadow_ray_direction: vec4<f32>,
};

struct Hit {
  distance: f32,
  material_id: u32,
  found: u32,
  _padding: u32,
  world: vec4<f32>,
  normal: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: RayUniform;
@group(0) @binding(1) var<storage, read> materials: array<u32>;
@group(1) @binding(0) var shadow_output: texture_storage_2d<r32float, write>;
@group(2) @binding(0) var shadow_input: texture_2d<f32>;
@group(2) @binding(1) var color_output: texture_storage_2d<rgba8unorm, write>;

fn empty_hit() -> Hit {
  var hit: Hit;
  hit.distance = 0.0;
  hit.material_id = 0u;
  hit.found = 0u;
  hit._padding = 0u;
  hit.world = vec4<f32>(0.0);
  hit.normal = vec4<f32>(0.0);
  return hit;
}

fn material_at(cell: vec3<i32>, bounds_min: vec3<i32>, shape: vec3<u32>) -> u32 {
  let local = cell - bounds_min;
  if any(local < vec3<i32>(0)) || any(local >= vec3<i32>(shape)) {
    return 0u;
  }
  let index = u32(local.x)
    + u32(local.y) * shape.x
    + u32(local.z) * shape.x * shape.y;
  return materials[index];
}

fn trace_volume(
  origin: vec3<f32>,
  direction: vec3<f32>,
  bounds_min: vec3<i32>,
  shape: vec3<u32>,
) -> Hit {
  let box_min = vec3<f32>(bounds_min);
  let box_max = box_min + vec3<f32>(shape);
  var near_distance = 0.0;
  var far_distance = 1.0e30;
  var entry_axis = 0u;
  for (var axis = 0u; axis < 3u; axis += 1u) {
    let component = direction[axis];
    if abs(component) < 1.0e-7 {
      if origin[axis] < box_min[axis] || origin[axis] >= box_max[axis] {
        return empty_hit();
      }
    } else {
      let first = (box_min[axis] - origin[axis]) / component;
      let second = (box_max[axis] - origin[axis]) / component;
      let axis_near = min(first, second);
      let axis_far = max(first, second);
      if axis_near > near_distance {
        near_distance = axis_near;
        entry_axis = axis;
      }
      far_distance = min(far_distance, axis_far);
      if far_distance < near_distance {
        return empty_hit();
      }
    }
  }
  if far_distance <= 0.0 {
    return empty_hit();
  }
  var distance = max(near_distance, 0.0) + 1.0e-4;
  let start = origin + direction * distance;
  var cell = vec3<i32>(floor(start));
  let step = vec3<i32>(
    select(-1, 1, direction.x >= 0.0),
    select(-1, 1, direction.y >= 0.0),
    select(-1, 1, direction.z >= 0.0),
  );
  let delta = vec3<f32>(
    select(1.0e30, abs(1.0 / direction.x), abs(direction.x) >= 1.0e-7),
    select(1.0e30, abs(1.0 / direction.y), abs(direction.y) >= 1.0e-7),
    select(1.0e30, abs(1.0 / direction.z), abs(direction.z) >= 1.0e-7),
  );
  let next_boundary = vec3<f32>(
    f32(cell.x + select(0, 1, step.x > 0)),
    f32(cell.y + select(0, 1, step.y > 0)),
    f32(cell.z + select(0, 1, step.z > 0)),
  );
  var side = vec3<f32>(
    select(1.0e30, (next_boundary.x - origin.x) / direction.x, abs(direction.x) >= 1.0e-7),
    select(1.0e30, (next_boundary.y - origin.y) / direction.y, abs(direction.y) >= 1.0e-7),
    select(1.0e30, (next_boundary.z - origin.z) / direction.z, abs(direction.z) >= 1.0e-7),
  );
  var last_axis = entry_axis;
  for (var iteration = 0u; iteration < 2048u; iteration += 1u) {
    let local = cell - bounds_min;
    if any(local < vec3<i32>(0)) || any(local >= vec3<i32>(shape)) || distance > far_distance {
      break;
    }
    let material_id = material_at(cell, bounds_min, shape);
    if material_id != 0u {
      var normal = vec3<f32>(0.0);
      normal[last_axis] = -f32(step[last_axis]);
      var hit: Hit;
      hit.distance = distance;
      hit.material_id = material_id;
      hit.found = 1u;
      hit._padding = 0u;
      hit.world = vec4<f32>(origin + direction * distance, 1.0);
      hit.normal = vec4<f32>(normal, 0.0);
      return hit;
    }
    if side.x <= side.y && side.x <= side.z {
      distance = side.x;
      side.x += delta.x;
      cell.x += step.x;
      last_axis = 0u;
    } else if side.y <= side.z {
      distance = side.y;
      side.y += delta.y;
      cell.y += step.y;
      last_axis = 1u;
    } else {
      distance = side.z;
      side.z += delta.z;
      cell.z += step.z;
      last_axis = 2u;
    }
  }
  return empty_hit();
}

fn shadow_ray(pixel: vec2<u32>) -> vec3<f32> {
  let uv = (vec2<f32>(pixel) + 0.5) / f32(textureDimensions(shadow_output).x);
  let offset = (uv * 2.0 - 1.0) * params.shadow_center_extent.w;
  let center = params.shadow_center_extent.xyz;
  let ray_direction = params.shadow_ray_direction.xyz;
  return center
    + params.shadow_right.xyz * offset.x
    + params.shadow_up.xyz * offset.y
    - ray_direction * params.shadow_center_extent.w * 2.0;
}

@compute @workgroup_size(8, 8)
fn shadow_main(@builtin(global_invocation_id) invocation: vec3<u32>) {
  let size = textureDimensions(shadow_output);
  if invocation.x >= size.x || invocation.y >= size.y {
    return;
  }
  let origin = shadow_ray(invocation.xy);
  let hit = trace_volume(
    origin,
    params.shadow_ray_direction.xyz,
    params.bounds_min.xyz,
    params.shape.xyz,
  );
  textureStore(
    shadow_output,
    vec2<i32>(invocation.xy),
    vec4<f32>(select(1.0e30, hit.distance, hit.found != 0u), 0.0, 0.0, 0.0),
  );
}

fn material_color(id: u32) -> vec3<f32> {
  let seed = vec3<u32>(
    id * 1664525u + 1013904223u,
    id * 22695477u + 1u,
    id * 1103515245u + 12345u,
  );
  return vec3<f32>(seed & vec3<u32>(255u)) / 255.0 * 0.55 + 0.08;
}

fn shadow_visibility(world: vec3<f32>) -> f32 {
  let relative = world - params.shadow_center_extent.xyz;
  let extent = params.shadow_center_extent.w;
  let uv = vec2<f32>(
    dot(relative, params.shadow_right.xyz),
    dot(relative, params.shadow_up.xyz),
  ) / (2.0 * extent) + 0.5;
  if any(uv < vec2<f32>(0.0)) || any(uv >= vec2<f32>(1.0)) {
    return 1.0;
  }
  let dimensions = textureDimensions(shadow_input);
  let pixel = vec2<i32>(clamp(uv * vec2<f32>(dimensions), vec2<f32>(0.0), vec2<f32>(dimensions - 1u)));
  let stored_distance = textureLoad(shadow_input, pixel, 0).x;
  let ray_origin = params.shadow_center_extent.xyz
    + params.shadow_right.xyz * ((uv.x * 2.0 - 1.0) * extent)
    + params.shadow_up.xyz * ((uv.y * 2.0 - 1.0) * extent)
    - params.shadow_ray_direction.xyz * extent * 2.0;
  let receiver_distance = dot(world - ray_origin, params.shadow_ray_direction.xyz);
  return select(0.28, 1.0, receiver_distance <= stored_distance + 0.12);
}

@compute @workgroup_size(8, 8)
fn color_main(@builtin(global_invocation_id) invocation: vec3<u32>) {
  let size = textureDimensions(color_output);
  if invocation.x >= size.x || invocation.y >= size.y {
    return;
  }
  let uv = (vec2<f32>(invocation.xy) + 0.5) / vec2<f32>(size);
  let screen = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
  let direction = normalize(
    params.forward_tan_half_fov.xyz
      + params.right_aspect.xyz
        * screen.x
        * params.forward_tan_half_fov.w
        * params.right_aspect.w
      + params.up.xyz * screen.y * params.forward_tan_half_fov.w
  );
  let hit = trace_volume(
    params.eye.xyz,
    direction,
    params.bounds_min.xyz,
    params.shape.xyz,
  );
  if hit.found == 0u {
    textureStore(color_output, vec2<i32>(invocation.xy), vec4<f32>(0.005, 0.008, 0.02, 1.0));
    return;
  }
  let sun = params.sun_direction.xyz;
  let view = normalize(params.eye.xyz - hit.world.xyz);
  let half_vector = normalize(sun + view);
  let diffuse = max(dot(hit.normal.xyz, sun), 0.0);
  let visibility = shadow_visibility(hit.world.xyz + hit.normal.xyz * 0.02);
  let wet_base = material_color(hit.material_id) * 0.72;
  let specular = pow(max(dot(hit.normal.xyz, half_vector), 0.0), 48.0) * 0.65;
  let color = wet_base * (0.055 + diffuse * 0.28 * visibility)
    + vec3<f32>(0.24, 0.31, 0.45) * specular * visibility;
  textureStore(color_output, vec2<i32>(invocation.xy), vec4<f32>(color, 1.0));
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct RayUniform {
    eye: [f32; 4],
    forward_tan_half_fov: [f32; 4],
    right_aspect: [f32; 4],
    up: [f32; 4],
    bounds_min: [i32; 4],
    shape: [u32; 4],
    sun_direction: [f32; 4],
    shadow_center_extent: [f32; 4],
    shadow_right: [f32; 4],
    shadow_up: [f32; 4],
    shadow_ray_direction: [f32; 4],
}

fn uniform(camera: BakeoffCamera, volume: &BakeoffVolume) -> RayUniform {
    let eye = Vec3::from_array(camera.eye_voxels.map(|value| value as f32));
    let forward = Vec3::new(
        camera.yaw_radians.sin() as f32 * camera.pitch_radians.cos() as f32,
        camera.pitch_radians.sin() as f32,
        -camera.yaw_radians.cos() as f32 * camera.pitch_radians.cos() as f32,
    )
    .normalize();
    let right = Vec3::new(
        camera.yaw_radians.cos() as f32,
        0.0,
        camera.yaw_radians.sin() as f32,
    );
    let up = right.cross(forward).normalize();
    let bounds = volume.bounds();
    let shape = volume.shape();
    let minimum = Vec3::new(
        bounds.min.x as f32,
        bounds.min.y as f32,
        bounds.min.z as f32,
    );
    let shape_vector = Vec3::new(shape[0] as f32, shape[1] as f32, shape[2] as f32);
    let center = minimum + shape_vector * 0.5;
    let extent = shape_vector.length() * 0.55;
    let sun = Vec3::new(-0.42, 0.78, -0.46).normalize();
    let shadow_ray_direction = -sun;
    let shadow_right = shadow_ray_direction.cross(Vec3::Y).normalize();
    let shadow_up = shadow_right.cross(shadow_ray_direction).normalize();
    RayUniform {
        eye: eye.extend(0.0).to_array(),
        forward_tan_half_fov: forward
            .extend((camera.vertical_fov_radians as f32 * 0.5).tan())
            .to_array(),
        right_aspect: right.extend(camera.aspect_ratio as f32).to_array(),
        up: up.extend(0.0).to_array(),
        bounds_min: [bounds.min.x, bounds.min.y, bounds.min.z, 0],
        shape: [
            u32::try_from(shape[0]).unwrap_or(u32::MAX),
            u32::try_from(shape[1]).unwrap_or(u32::MAX),
            u32::try_from(shape[2]).unwrap_or(u32::MAX),
            0,
        ],
        sun_direction: sun.extend(0.0).to_array(),
        shadow_center_extent: center.extend(extent).to_array(),
        shadow_right: shadow_right.extend(0.0).to_array(),
        shadow_up: shadow_up.extend(0.0).to_array(),
        shadow_ray_direction: shadow_ray_direction.extend(0.0).to_array(),
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

pub async fn run(camera: BakeoffCamera, volume: &BakeoffVolume) -> Result<Value, Box<dyn Error>> {
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
        label: Some("dense voxel lower-bound shader"),
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });
    let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("dense voxel ray parameters"),
        contents: bytemuck::bytes_of(&uniform(camera, volume)),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let material_ids = volume
        .materials()
        .iter()
        .map(|material| u32::from(material.id()))
        .collect::<Vec<_>>();
    let material_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("dense canonical voxel materials"),
        contents: bytemuck::cast_slice(&material_ids),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let shadow_target = target(
        &device,
        "dense voxel 2K shadow map",
        [SHADOW_EDGE, SHADOW_EDGE],
        SHADOW_FORMAT,
        wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
    );
    let color_target = target(
        &device,
        "dense voxel 4K color",
        [WIDTH, HEIGHT],
        COLOR_FORMAT,
        wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
    );
    let shadow_view = shadow_target.create_view(&Default::default());
    let color_view = color_target.create_view(&Default::default());
    let common_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("dense voxel immutable ray-cast inputs"),
        entries: &[uniform_entry(0), storage_entry(1)],
    });
    let shadow_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("dense voxel shadow output"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format: SHADOW_FORMAT,
                view_dimension: wgpu::TextureViewDimension::D2,
            },
            count: None,
        }],
    });
    let empty_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("dense voxel unused color group"),
        entries: &[],
    });
    let color_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("dense voxel shadow input and color output"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: COLOR_FORMAT,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                count: None,
            },
        ],
    });
    let shadow_pipeline = compute_pipeline(
        &device,
        &shader,
        &[&common_layout, &shadow_layout],
        "shadow_main",
    );
    let color_pipeline = compute_pipeline(
        &device,
        &shader,
        &[&common_layout, &empty_layout, &color_layout],
        "color_main",
    );
    let common_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("dense voxel immutable ray-cast bind group"),
        layout: &common_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: material_buffer.as_entire_binding(),
            },
        ],
    });
    let shadow_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("dense voxel shadow output bind group"),
        layout: &shadow_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(&shadow_view),
        }],
    });
    let color_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("dense voxel color output bind group"),
        layout: &color_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&shadow_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&color_view),
            },
        ],
    });
    let encode_frame = |encoder: &mut wgpu::CommandEncoder,
                        query_set: Option<&wgpu::QuerySet>,
                        first_query: u32| {
        {
            let timestamp_writes = query_set.map(|query_set| wgpu::ComputePassTimestampWrites {
                query_set,
                beginning_of_pass_write_index: Some(first_query),
                end_of_pass_write_index: Some(first_query + 1),
            });
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("dense voxel 2K shadow ray cast"),
                timestamp_writes,
            });
            pass.set_pipeline(&shadow_pipeline);
            pass.set_bind_group(0, &common_bind_group, &[]);
            pass.set_bind_group(1, &shadow_bind_group, &[]);
            pass.dispatch_workgroups(SHADOW_EDGE.div_ceil(8), SHADOW_EDGE.div_ceil(8), 1);
        }
        {
            let timestamp_writes = query_set.map(|query_set| wgpu::ComputePassTimestampWrites {
                query_set,
                beginning_of_pass_write_index: Some(first_query + 2),
                end_of_pass_write_index: Some(first_query + 3),
            });
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("dense voxel 4K shadowed wet material ray cast"),
                timestamp_writes,
            });
            pass.set_pipeline(&color_pipeline);
            pass.set_bind_group(0, &common_bind_group, &[]);
            pass.set_bind_group(2, &color_bind_group, &[]);
            pass.dispatch_workgroups(WIDTH.div_ceil(8), HEIGHT.div_ceil(8), 1);
        }
    };
    for _ in 0..WARMUP_FRAMES {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("dense voxel lower-bound warmup"),
        });
        encode_frame(&mut encoder, None, 0);
        queue.submit([encoder.finish()]);
    }
    device.poll(wgpu::PollType::wait_indefinitely())?;
    let query_count = SAMPLE_FRAMES * QUERIES_PER_FRAME;
    let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
        label: Some("dense voxel lower-bound timestamps"),
        ty: wgpu::QueryType::Timestamp,
        count: query_count,
    });
    let query_bytes = u64::from(query_count) * 8;
    let resolve = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("dense voxel timestamp resolve"),
        size: query_bytes,
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("dense voxel timestamp readback"),
        size: query_bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let color_bytes = u64::from(WIDTH) * u64::from(HEIGHT) * 4;
    let shadow_bytes = u64::from(SHADOW_EDGE) * u64::from(SHADOW_EDGE) * 4;
    let output_readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("dense voxel output verification readback"),
        size: color_bytes + shadow_bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("dense voxel timed lower-bound frames"),
    });
    for frame in 0..SAMPLE_FRAMES {
        encode_frame(&mut encoder, Some(&query_set), frame * QUERIES_PER_FRAME);
    }
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
        shadow_target.as_image_copy(),
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
        .filter(|pixel| pixel[0] != 1 || pixel[1] != 2 || pixel[2] != 5 || pixel[3] != 255)
        .count();
    let mut shadow_hit_pixels = 0usize;
    let mut invalid_shadow_pixels = 0usize;
    for bytes in output[color_end..].chunks_exact(4) {
        let mut raw = [0u8; 4];
        raw.copy_from_slice(bytes);
        let distance = f32::from_le_bytes(raw);
        if !distance.is_finite() {
            invalid_shadow_pixels += 1;
        } else if distance < 1.0e29 {
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
            "dense voxel GPU output is invalid: {primary_hit_pixels} primary hits, \
             {shadow_hit_pixels} shadow hits, {invalid_shadow_pixels} invalid shadow samples"
        )
        .into());
    }
    let mut shadow_ms = Vec::with_capacity(SAMPLE_FRAMES as usize);
    let mut color_ms = Vec::with_capacity(SAMPLE_FRAMES as usize);
    let mut discarded_timestamp_frames = 0u32;
    for frame in 0..SAMPLE_FRAMES as usize {
        let base = frame * QUERIES_PER_FRAME as usize;
        let Some((shadow, color)) = duration_ms(&timestamps, base, base + 1, timestamp_period).zip(
            duration_ms(&timestamps, base + 2, base + 3, timestamp_period),
        ) else {
            discarded_timestamp_frames += 1;
            continue;
        };
        shadow_ms.push(shadow);
        color_ms.push(color);
    }
    if shadow_ms.len() < 32 {
        return Err(format!(
            "Metal supplied only {} valid dense-voxel timestamp frames out of {SAMPLE_FRAMES}",
            shadow_ms.len()
        )
        .into());
    }
    let total_ms = shadow_ms
        .iter()
        .zip(&color_ms)
        .map(|(shadow, color)| shadow + color)
        .collect::<Vec<_>>();
    Ok(json!({
        "schema": "voxels.virtual-surface-dense-voxel-gpu-bakeoff.v1",
        "classification": "optimistic-lower-bound-for-sparse-brick-traversal",
        "adapter": {
            "name": info.name,
            "deviceType": format!("{:?}", info.device_type),
            "backend": format!("{:?}", info.backend),
            "timestampPeriodNanoseconds": timestamp_period,
        },
        "workload": {
            "pixelSize": [WIDTH, HEIGHT],
            "shadowSize": [SHADOW_EDGE, SHADOW_EDGE],
            "samples": SAMPLE_FRAMES,
            "measuredFrames": shadow_ms.len(),
            "discardedTimestampFrames": discarded_timestamp_frames,
            "warmupFrames": WARMUP_FRAMES,
            "canonicalVoxelCount": material_ids.len(),
            "passesPerFrame": 2,
            "state": "night-rain-wet-material-shadow-sampled",
            "traversal": "contiguous-dense-dda",
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
            "canonicalMaterials": std::mem::size_of_val(material_ids.as_slice()),
            "colorTarget": u64::from(WIDTH) * u64::from(HEIGHT) * 4,
            "shadowTarget": u64::from(SHADOW_EDGE) * u64::from(SHADOW_EDGE) * 4,
        },
    }))
}

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn storage_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn compute_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layouts: &[&wgpu::BindGroupLayout],
    entry_point: &str,
) -> wgpu::ComputePipeline {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("dense voxel compute pipeline layout"),
        bind_group_layouts: &layouts.iter().copied().map(Some).collect::<Vec<_>>(),
        immediate_size: 0,
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(entry_point),
        layout: Some(&pipeline_layout),
        module: shader,
        entry_point: Some(entry_point),
        compilation_options: Default::default(),
        cache: None,
    })
}

fn duration_ms(
    timestamps: &[u64],
    first: usize,
    last: usize,
    timestamp_period: f64,
) -> Option<f64> {
    timestamps[last]
        .checked_sub(timestamps[first])
        .filter(|ticks| *ticks != 0)
        .map(|ticks| ticks as f64 * timestamp_period / 1_000_000.0)
}

fn distribution(values: &[f64]) -> Value {
    json!({
        "minimum": values.iter().copied().fold(f64::INFINITY, f64::min),
        "p50": percentile(values, 0.50),
        "p95": percentile(values, 0.95),
        "p99": percentile(values, 0.99),
        "maximum": values.iter().copied().fold(0.0, f64::max),
    })
}
