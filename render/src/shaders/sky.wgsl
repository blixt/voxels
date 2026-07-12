struct Frame {
  view_projection: mat4x4<f32>,
  inverse_view_projection: mat4x4<f32>,
  camera_time: vec4<f32>,
  viewport_voxel: vec4<f32>,
  target_voxel: vec4<f32>,
  render_options: vec4<f32>,
  lod_options: vec4<f32>,
  camera_forward: vec4<f32>,
  shadow_splits: vec4<f32>,
  shadow_texel_sizes: vec4<f32>,
  shadow_view_projection: array<mat4x4<f32>, 3>,
  sun_direction: vec4<f32>,
  sun_radiance: vec4<f32>,
  sky_horizon: vec4<f32>,
  sky_zenith: vec4<f32>,
  ground_atmosphere: vec4<f32>,
  fog_exposure: vec4<f32>,
  medium: vec4<f32>,
};

@group(0) @binding(0) var<uniform> frame: Frame;

fn hash21(position: vec2<f32>) -> f32 {
  return fract(sin(dot(position, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn value_noise(position: vec2<f32>) -> f32 {
  let cell = floor(position);
  let fraction = fract(position);
  let blend = fraction * fraction * (3.0 - 2.0 * fraction);
  let a = hash21(cell);
  let b = hash21(cell + vec2<f32>(1.0, 0.0));
  let c = hash21(cell + vec2<f32>(0.0, 1.0));
  let d = hash21(cell + vec2<f32>(1.0, 1.0));
  return mix(mix(a, b, blend.x), mix(c, d, blend.x), blend.y);
}

fn cloud_field(position: vec2<f32>) -> f32 {
  let broad = value_noise(position) * 0.58;
  let billows = value_noise(position * 2.03 + vec2<f32>(17.2, -9.1)) * 0.29;
  let detail = value_noise(position * 4.11 + vec2<f32>(-4.7, 23.4)) * 0.13;
  return broad + billows + detail;
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
  let x = f32((index << 1u) & 2u);
  let y = f32(index & 2u);
  return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let uv = position.xy / frame.viewport_voxel.xy;
  let ndc = vec2<f32>(uv.x * 2.0 - 1.0, (1.0 - uv.y) * 2.0 - 1.0);
  let far_position = frame.inverse_view_projection * vec4<f32>(ndc, 1.0, 1.0);
  let ray = normalize(far_position.xyz / far_position.w - frame.camera_time.xyz);
  let sun_direction = normalize(frame.sun_direction.xyz);
  let elevation = clamp(ray.y * 0.5 + 0.5, 0.0, 1.0);
  let horizon = pow(1.0 - abs(ray.y), 5.0);
  let rayleigh = pow(max(ray.y, 0.0), 0.42);
  let base = mix(frame.sky_horizon.rgb, frame.sky_zenith.rgb, rayleigh);
  let warm_horizon = frame.sun_radiance.rgb * vec3<f32>(1.0, 0.48, 0.24) * horizon * 0.018;
  let sun_amount = max(dot(ray, sun_direction), 0.0);
  let sun_disc = smoothstep(0.99955, 0.99985, sun_amount);
  let sun_glow = pow(sun_amount, 96.0) * 0.16 + pow(sun_amount, 12.0) * 0.022;
  let below_horizon = mix(frame.ground_atmosphere.rgb, base, smoothstep(0.0, 0.12, elevation));
  var color = below_horizon + warm_horizon + frame.sun_radiance.rgb * (sun_disc * 1.15 + sun_glow);
  if ray.y > 0.015 {
    let cloud_height = 480.0;
    let distance_to_layer = (cloud_height - frame.camera_time.y) / ray.y;
    let cloud_world = frame.camera_time.xz + ray.xz * distance_to_layer;
    let wind = vec2<f32>(frame.camera_time.w * 0.55, frame.camera_time.w * 0.16);
    let field = cloud_field(cloud_world * 0.0032 + wind * 0.001);
    let coverage = smoothstep(0.62, 0.77, field)
      * smoothstep(0.015, 0.12, ray.y)
      * (1.0 - smoothstep(0.94, 0.995, sun_amount));
    let cloud_shadow = vec3<f32>(0.16, 0.20, 0.27);
    let cloud_light = cloud_shadow + frame.sun_radiance.rgb * 0.085;
    color = mix(color, cloud_light, coverage * 0.62);
  }
  let wave = sin(ray.x * 21.0 + frame.camera_time.w * 1.3)
    * sin(ray.z * 17.0 - frame.camera_time.w * 0.9) * 0.025;
  let snell_window = smoothstep(0.61, 0.72, ray.y + wave);
  let path_to_surface = frame.medium.y / max(ray.y, 0.08);
  let water_transmittance = exp(-vec3<f32>(0.42, 0.16, 0.075) * path_to_surface);
  let water_scattering = vec3<f32>(0.010, 0.105, 0.145);
  let window_color = color * water_transmittance
    + water_scattering * (vec3<f32>(1.0) - water_transmittance);
  let overhead_glow = frame.sun_radiance.rgb
    * pow(max(dot(ray, sun_direction), 0.0), 18.0)
    * 0.012;
  let underwater_sky = mix(
    water_scattering * mix(0.34, 1.0, max(ray.y, 0.0)) + overhead_glow,
    window_color,
    snell_window,
  );
  color = mix(color, underwater_sky, frame.medium.x);
  return vec4<f32>(max(color * frame.fog_exposure.y, vec3<f32>(0.0)), 1.0);
}
