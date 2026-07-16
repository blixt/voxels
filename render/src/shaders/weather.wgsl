@group(0) @binding(0) var<uniform> frame: Frame;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
  let x = f32((index << 1u) & 2u);
  let y = f32(index & 2u);
  return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

fn rain_layer(uv: vec2<f32>, layer: f32, wind: vec2<f32>) -> f32 {
  let scale = mix(0.82, 1.42, layer);
  let position = uv * vec2<f32>(68.0, 22.0) * scale
    + vec2<f32>(
      wind.x * frame.camera_time.w * 0.028,
      frame.camera_time.w * mix(15.0, 24.0, layer),
    );
  let cell = floor(position);
  let random = atmosphere_hash21(cell + vec2<f32>(layer * 113.0, layer * 47.0));
  let local = fract(position);
  let centre = fract(random + local.y * (wind.x * 0.018 + 0.12));
  let distance = abs(fract(local.x - centre + 0.5) - 0.5);
  let streak = 1.0 - smoothstep(0.012, 0.045, distance);
  let tail = smoothstep(0.04, 0.14, local.y) * (1.0 - smoothstep(0.62, 0.96, local.y));
  return streak * tail * smoothstep(0.44, 0.88, random);
}

fn snow_layer(uv: vec2<f32>, layer: f32, wind: vec2<f32>) -> f32 {
  let scale = mix(16.0, 34.0, layer);
  let drift = vec2<f32>(
    wind.x * frame.camera_time.w * 0.018 + sin(frame.camera_time.w * 0.7 + layer * 8.0) * 0.12,
    frame.camera_time.w * mix(0.34, 0.72, layer),
  );
  let position = uv * scale + drift;
  let cell = floor(position);
  let random = atmosphere_hash21(cell + vec2<f32>(layer * 71.0, layer * 137.0));
  let centre = vec2<f32>(
    0.18 + atmosphere_hash21(cell + vec2<f32>(9.0, 17.0)) * 0.64,
    0.18 + atmosphere_hash21(cell + vec2<f32>(31.0, 5.0)) * 0.64,
  );
  let distance = length(fract(position) - centre);
  return (1.0 - smoothstep(0.035, 0.11, distance)) * smoothstep(0.42, 0.94, random);
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let precipitation = frame.weather.x * (1.0 - smoothstep(0.20, 0.72, frame.interior.x));
  if precipitation <= 0.002 {
    discard;
  }
  let uv = position.xy / frame.viewport_voxel.xy;
  let aspect_uv = vec2<f32>((uv.x - 0.5) * frame.viewport_voxel.x / frame.viewport_voxel.y, uv.y);
  let wind = frame.cloud_layer.zw;
  var rain = 0.0;
  var snow = 0.0;
  for (var layer = 0u; layer < 3u; layer += 1u) {
    let depth = f32(layer) / 2.0;
    rain += rain_layer(aspect_uv, depth, wind) * mix(0.34, 0.82, depth);
    snow += snow_layer(aspect_uv, depth, wind) * mix(0.30, 0.72, depth);
  }
  let snow_amount = frame.weather.w;
  let weather_shape = mix(rain, snow, snow_amount);
  let alpha = clamp(weather_shape * precipitation * mix(0.11, 0.22, frame.weather.y), 0.0, 0.42);
  let rain_color = mix(vec3<f32>(0.42, 0.55, 0.70), vec3<f32>(0.72, 0.82, 0.94), frame.weather.y);
  let snow_color = vec3<f32>(0.82, 0.88, 0.96);
  let color = mix(rain_color, snow_color, snow_amount) * alpha;
  return vec4<f32>(color, alpha);
}
