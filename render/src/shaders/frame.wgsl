struct Frame {
  view_projection: mat4x4<f32>,
  inverse_view_projection: mat4x4<f32>,
  camera_time: vec4<f32>,
  viewport_voxel: vec4<f32>,
  target_voxel: vec4<f32>,
  target_voxel_max: vec4<f32>,
  render_options: vec4<f32>,
  lod_options: vec4<f32>,
  lod_boundary_centres: array<vec4<f32>, 4>,
  camera_forward: vec4<f32>,
  shadow_splits: vec4<f32>,
  shadow_texel_sizes: vec4<f32>,
  shadow_view_projection: array<mat4x4<f32>, 3>,
  key_light_direction: vec4<f32>,
  key_light_radiance: vec4<f32>,
  sun_direction: vec4<f32>,
  moon_direction: vec4<f32>,
  equatorial_east: vec4<f32>,
  equatorial_up: vec4<f32>,
  equatorial_north: vec4<f32>,
  environment_time: vec4<f32>,
  atmosphere_motion: vec4<f32>,
  sky_horizon: vec4<f32>,
  sky_zenith: vec4<f32>,
  ground_atmosphere: vec4<f32>,
  fog_exposure: vec4<f32>,
  weather: vec4<f32>,
  cloud_layer: vec4<f32>,
  medium: vec4<f32>,
  interior: vec4<f32>,
};

// Shared by the sky and world shaders so visible clouds and their terrain attenuation cannot
// drift. Wrapping the lattice makes the 1,280 km CPU offset period seamless.
fn atmosphere_wrap_cell(cell: vec2<f32>) -> vec2<f32> {
  return cell - floor(cell / 4096.0) * 4096.0;
}

fn atmosphere_hash21(position: vec2<f32>) -> f32 {
  let wrapped = atmosphere_wrap_cell(position);
  return fract(sin(dot(wrapped, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn atmosphere_value_noise(position: vec2<f32>) -> f32 {
  let cell = floor(position);
  let fraction = fract(position);
  let blend = fraction * fraction * (3.0 - 2.0 * fraction);
  let a = atmosphere_hash21(cell);
  let b = atmosphere_hash21(cell + vec2<f32>(1.0, 0.0));
  let c = atmosphere_hash21(cell + vec2<f32>(0.0, 1.0));
  let d = atmosphere_hash21(cell + vec2<f32>(1.0, 1.0));
  return mix(mix(a, b, blend.x), mix(c, d, blend.x), blend.y);
}

fn atmosphere_branchless_gradient(hash: f32) -> vec2<f32> {
  let index = u32(floor(hash * 8.0));
  let magnitude = select(1.0, 0.70710678, (index & 1u) == 1u);
  let x_zero = index == 2u || index == 6u;
  let y_zero = index == 0u || index == 4u;
  let x_sign = select(-1.0, 1.0, index <= 1u || index == 7u);
  let y_sign = select(-1.0, 1.0, index <= 3u);
  return vec2<f32>(
    select(magnitude * x_sign, 0.0, x_zero),
    select(magnitude * y_sign, 0.0, y_zero),
  );
}

fn atmosphere_periodic_gradient_noise(position: vec2<f32>, period: f32) -> f32 {
  let cell = floor(position);
  let fraction = fract(position);
  let blend = fraction * fraction * fraction * (fraction * (fraction * 6.0 - 15.0) + 10.0);
  let wrapped = cell - floor(cell / period) * period;
  let next_x = vec2<f32>(wrapped.x + 1.0 - select(0.0, period, wrapped.x + 1.0 >= period), wrapped.y);
  let next_y = vec2<f32>(wrapped.x, wrapped.y + 1.0 - select(0.0, period, wrapped.y + 1.0 >= period));
  let next_xy = vec2<f32>(next_x.x, next_y.y);
  let a = dot(atmosphere_branchless_gradient(atmosphere_hash21(wrapped)), fraction);
  let b = dot(atmosphere_branchless_gradient(atmosphere_hash21(next_x)), fraction - vec2<f32>(1.0, 0.0));
  let c = dot(atmosphere_branchless_gradient(atmosphere_hash21(next_y)), fraction - vec2<f32>(0.0, 1.0));
  let d = dot(atmosphere_branchless_gradient(atmosphere_hash21(next_xy)), fraction - vec2<f32>(1.0, 1.0));
  return clamp(0.5 + mix(mix(a, b, blend.x), mix(c, d, blend.x), blend.y) * 0.70710678, 0.0, 1.0);
}

fn atmosphere_cloud_field_world(
  world_xz: vec2<f32>,
  cloud_offset_metres: vec2<f32>,
  weather_seed: f32,
) -> f32 {
  let seed_offset = vec2<f32>(
    fract(weather_seed * 0.1031),
    fract(weather_seed * 0.11369),
  );
  // 1,280 km is exactly 1,024 base cells. Integer-frequency octaves therefore wrap at
  // 1,024/2,048/4,096 cells with no discontinuity when the server bounds cloud advection.
  let position = (world_xz - cloud_offset_metres) * 0.0008;
  let broad = atmosphere_periodic_gradient_noise(position + seed_offset * 1024.0, 1024.0) * 0.58;
  let billows = atmosphere_periodic_gradient_noise(
    position * 2.0 + seed_offset * 2048.0 + vec2<f32>(17.2, -9.1),
    2048.0,
  ) * 0.29;
  let detail = atmosphere_periodic_gradient_noise(
    position * 4.0 + seed_offset * 4096.0 + vec2<f32>(-4.7, 23.4),
    4096.0,
  ) * 0.13;
  return broad + billows + detail;
}

fn atmosphere_cloud_envelope(field: f32, coverage_control: f32) -> f32 {
  let coverage = clamp(coverage_control, 0.0, 1.0);
  let threshold = mix(0.84, 0.45, coverage);
  return smoothstep(threshold - 0.08, threshold + 0.08, field);
}

fn atmosphere_cloud_envelope_world(world_xz: vec2<f32>) -> f32 {
  let field = atmosphere_cloud_field_world(
    world_xz,
    frame.environment_time.yz,
    frame.environment_time.w,
  );
  return atmosphere_cloud_envelope(field, frame.fog_exposure.z);
}
