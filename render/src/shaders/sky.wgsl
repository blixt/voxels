struct Frame {
  view_projection: mat4x4<f32>,
  inverse_view_projection: mat4x4<f32>,
  camera_time: vec4<f32>,
  viewport_voxel: vec4<f32>,
  target_voxel: vec4<f32>,
  render_options: vec4<f32>,
};

@group(0) @binding(0) var<uniform> frame: Frame;

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
  let sun_direction = normalize(vec3<f32>(0.48, 0.72, 0.35));
  let elevation = clamp(ray.y * 0.5 + 0.5, 0.0, 1.0);
  let horizon = pow(1.0 - abs(ray.y), 5.0);
  let rayleigh = pow(max(ray.y, 0.0), 0.42);
  let base = mix(vec3<f32>(0.64, 0.76, 0.82), vec3<f32>(0.10, 0.32, 0.62), rayleigh);
  let warm_horizon = vec3<f32>(0.93, 0.54, 0.29) * horizon * 0.24;
  let sun_amount = max(dot(ray, sun_direction), 0.0);
  let sun_disc = smoothstep(0.99955, 0.99985, sun_amount);
  let sun_glow = pow(sun_amount, 96.0) * 0.42 + pow(sun_amount, 12.0) * 0.09;
  let below_horizon = mix(vec3<f32>(0.20, 0.28, 0.34), base, smoothstep(0.0, 0.12, elevation));
  let color = below_horizon + warm_horizon + vec3<f32>(1.0, 0.83, 0.57) * (sun_disc * 3.0 + sun_glow);
  return vec4<f32>(max(color, vec3<f32>(0.0)), 1.0);
}
