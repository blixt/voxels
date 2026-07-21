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
  lod_boundary_half_extents: array<vec4<f32>, 2>,
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
  diagnostic_sky: vec4<f32>,
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

fn primary_rainbow_peak(cosine: f32, centre: f32, half_width: f32) -> f32 {
  return 1.0 - smoothstep(half_width * 0.25, half_width, abs(cosine - centre));
}

fn liquid_precipitation() -> f32 {
  return max(frame.weather.x - frame.weather.w, 0.0);
}

fn primary_rainbow_weather_possible() -> bool {
  let sun_y = normalize(frame.sun_direction.xyz).y;
  return liquid_precipitation() > 0.04
    && sun_y > 0.015
    && sun_y < 0.6691306
    && frame.sun_direction.w > 0.001
    && frame.medium.x < 0.999
    && frame.interior.x < 0.999;
}

fn primary_rainbow_radiance(ray: vec3<f32>) -> vec3<f32> {
  // This condition is uniform for the entire frame. In the overwhelmingly common case it keeps
  // the full-screen sky and cloud-composite invocations out of all angular and weather work.
  if !primary_rainbow_weather_possible() {
    return vec3<f32>(0.0);
  }
  let sun = normalize(frame.sun_direction.xyz);
  // A primary bow is centred on the antisolar point. Its red outer edge is about 42 degrees from
  // that point and the violet inner edge is about 40 degrees. Reject everything outside the
  // narrow cone before evaluating the world-space shower field.
  let antisolar_cosine = dot(ray, -sun);
  if antisolar_cosine < 0.7314 || antisolar_cosine > 0.7716 {
    return vec3<f32>(0.0);
  }
  // The ground hides the cone once the Sun rises above its 42-degree radius. Gentle endpoint
  // fades avoid a temporal switch as the synchronized celestial clock crosses either horizon.
  let low_sun = smoothstep(0.015, 0.055, sun.y)
    * (1.0 - smoothstep(0.635, 0.6691306, sun.y))
    * frame.sun_direction.w;
  if low_sun <= 0.001 {
    return vec3<f32>(0.0);
  }
  let horizontal = ray.xz / max(length(ray.xz), 0.0001);
  let shower_world_xz = frame.camera_time.xz + horizontal * 1800.0;
  let local_rain = atmosphere_cloud_envelope_world(shower_world_xz)
    * liquid_precipitation();
  if local_rain <= 0.025 {
    return vec3<f32>(0.0);
  }

  // Cosines encode increasing angular radius in reverse order. These overlapping narrow lobes
  // approximate wavelength dispersion without acos, a lookup texture, or another render pass.
  let spectrum =
    vec3<f32>(1.00, 0.10, 0.03) * primary_rainbow_peak(antisolar_cosine, 0.7396, 0.0042)
    + vec3<f32>(1.00, 0.34, 0.02) * primary_rainbow_peak(antisolar_cosine, 0.7443, 0.0040)
    + vec3<f32>(1.00, 0.78, 0.04) * primary_rainbow_peak(antisolar_cosine, 0.7484, 0.0038)
    + vec3<f32>(0.18, 0.92, 0.16) * primary_rainbow_peak(antisolar_cosine, 0.7527, 0.0038)
    + vec3<f32>(0.08, 0.36, 1.00) * primary_rainbow_peak(antisolar_cosine, 0.7567, 0.0038)
    + vec3<f32>(0.42, 0.10, 0.86) * primary_rainbow_peak(antisolar_cosine, 0.7604, 0.0038);
  let sunlight = clamp(
    dot(frame.key_light_radiance.rgb, vec3<f32>(0.2126, 0.7152, 0.0722)) * 0.24,
    0.0,
    1.0,
  );
  let clearing = 1.0 - frame.weather.y * 0.82;
  return spectrum
    * low_sun
    * sunlight
    * clearing
    * smoothstep(0.025, 0.55, local_rain)
    * (1.0 - frame.medium.x)
    * (1.0 - frame.interior.x)
    * 0.16;
}
