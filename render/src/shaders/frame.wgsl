struct Frame {
  view_projection: mat4x4<f32>,
  inverse_view_projection: mat4x4<f32>,
  camera_time: vec4<f32>,
  viewport_voxel: vec4<f32>,
  target_voxel: vec4<f32>,
  target_voxel_max: vec4<f32>,
  render_options: vec4<f32>,
  lod_options: vec4<f32>,
  lod_boundary_centres: array<vec4<f32>, 3>,
  camera_forward: vec4<f32>,
  shadow_splits: vec4<f32>,
  shadow_texel_sizes: vec4<f32>,
  shadow_view_projection: array<mat4x4<f32>, 3>,
  key_light_direction: vec4<f32>,
  key_light_radiance: vec4<f32>,
  sun_direction: vec4<f32>,
  moon_direction: vec4<f32>,
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

fn atmosphere_cloud_field_world(
  world_xz: vec2<f32>,
  cloud_offset_metres: vec2<f32>,
  weather_seed: f32,
) -> f32 {
  let seed_offset = vec2<f32>(
    fract(weather_seed * 0.1031),
    fract(weather_seed * 0.11369),
  ) * 4096.0;
  let position = (world_xz - cloud_offset_metres) * 0.00065 + seed_offset;
  let broad = atmosphere_value_noise(position) * 0.58;
  let billows = atmosphere_value_noise(position * 2.03 + vec2<f32>(17.2, -9.1)) * 0.29;
  let detail = atmosphere_value_noise(position * 4.11 + vec2<f32>(-4.7, 23.4)) * 0.13;
  return broad + billows + detail;
}
