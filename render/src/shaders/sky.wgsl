struct Frame {
  view_projection: mat4x4<f32>,
  camera_time: vec4<f32>,
  viewport: vec2<f32>,
  _pad: vec2<f32>,
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
  let uv = position.xy / frame.viewport;
  let horizon = smoothstep(0.12, 0.82, uv.y);
  let pulse = sin(frame.camera_time.w * 0.12) * 0.018;
  let sky = mix(vec3<f32>(0.42, 0.68, 0.82), vec3<f32>(0.035, 0.08, 0.16), horizon);
  let glow = vec3<f32>(0.96, 0.66, 0.32) * pow(max(0.0, 1.0 - distance(uv, vec2<f32>(0.72, 0.28))), 10.0);
  return vec4<f32>(sky + glow + pulse, 1.0);
}
