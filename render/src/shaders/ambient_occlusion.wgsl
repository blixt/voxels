@group(0) @binding(0) var<uniform> frame: Frame;
@group(1) @binding(0) var world_depth: texture_depth_2d;
@group(1) @binding(1) var source_ao: texture_2d<f32>;

struct VertexOutput {
  @builtin(position) position: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
  let x = f32((index << 1u) & 2u);
  let y = f32(index & 2u);
  var output: VertexOutput;
  output.position = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
  return output;
}

fn clamp_pixel(pixel: vec2<i32>, dimensions: vec2<u32>) -> vec2<i32> {
  return clamp(pixel, vec2<i32>(0), vec2<i32>(dimensions) - vec2<i32>(1));
}

fn position_from_depth(pixel: vec2<i32>, depth: f32, dimensions: vec2<u32>) -> vec3<f32> {
  let uv = (vec2<f32>(pixel) + vec2<f32>(0.5)) / vec2<f32>(dimensions);
  let ndc = vec2<f32>(uv.x * 2.0 - 1.0, (1.0 - uv.y) * 2.0 - 1.0);
  let homogeneous = frame.inverse_view_projection * vec4<f32>(ndc, depth, 1.0);
  return homogeneous.xyz / max(abs(homogeneous.w), 0.000001) * sign(homogeneous.w);
}

fn load_position(pixel: vec2<i32>, dimensions: vec2<u32>) -> vec4<f32> {
  let coordinate = clamp_pixel(pixel, dimensions);
  let depth = textureLoad(world_depth, coordinate, 0);
  return vec4<f32>(position_from_depth(coordinate, depth, dimensions), depth);
}

fn view_depth(position: vec3<f32>) -> f32 {
  return dot(position - frame.camera_time.xyz, frame.camera_forward.xyz);
}

fn reconstructed_normal(
  center: vec4<f32>,
  pixel: vec2<i32>,
  dimensions: vec2<u32>,
) -> vec3<f32> {
  let left = load_position(pixel + vec2<i32>(-1, 0), dimensions);
  let right = load_position(pixel + vec2<i32>(1, 0), dimensions);
  let top = load_position(pixel + vec2<i32>(0, -1), dimensions);
  let bottom = load_position(pixel + vec2<i32>(0, 1), dimensions);
  let center_depth = view_depth(center.xyz);
  let horizontal = select(
    left.xyz - center.xyz,
    right.xyz - center.xyz,
    abs(view_depth(right.xyz) - center_depth) < abs(view_depth(left.xyz) - center_depth),
  );
  let vertical = select(
    top.xyz - center.xyz,
    bottom.xyz - center.xyz,
    abs(view_depth(bottom.xyz) - center_depth) < abs(view_depth(top.xyz) - center_depth),
  );
  var normal = normalize(cross(vertical, horizontal));
  if dot(normal, frame.camera_time.xyz - center.xyz) < 0.0 {
    normal = -normal;
  }
  return normal;
}

fn interleaved_rotation(pixel: vec2<i32>) -> f32 {
  let seed = dot(vec2<f32>(pixel), vec2<f32>(0.06711056, 0.00583715));
  return fract(52.9829189 * fract(seed)) * 6.28318530718;
}

@fragment
fn fs_evaluate(@builtin(position) position: vec4<f32>) -> @location(0) vec2<f32> {
  let dimensions = textureDimensions(world_depth);
  let pixel = clamp_pixel(vec2<i32>(position.xy) * 2 + vec2<i32>(1), dimensions);
  let center = load_position(pixel, dimensions);
  if center.w >= 0.999999 {
    return vec2<f32>(1.0, 0.0);
  }
  let normal = reconstructed_normal(center, pixel, dimensions);
  let center_view_depth = max(view_depth(center.xyz), 0.01);
  let projected_radius = clamp(
    1.35 * f32(dimensions.y) / (1.349016 * center_view_depth),
    3.0,
    56.0,
  );
  let distance_fade = 1.0 - smoothstep(78.0, 118.0, center_view_depth);
  let rotation = interleaved_rotation(pixel);
  var horizon_sum = 0.0;
  for (var direction_index = 0u; direction_index < 4u; direction_index += 1u) {
    let angle = rotation + f32(direction_index) * 0.78539816339;
    let direction = vec2<f32>(cos(angle), sin(angle));
    var positive_horizon = 0.0;
    var negative_horizon = 0.0;
    for (var step_index = 1u; step_index <= 3u; step_index += 1u) {
      let step_fraction = f32(step_index) / 3.0;
      let radius = projected_radius * step_fraction * step_fraction;
      for (var side = 0u; side < 2u; side += 1u) {
        let sign_value = select(-1.0, 1.0, side == 1u);
        let sample_pixel = clamp_pixel(
          pixel + vec2<i32>(round(direction * radius * sign_value)),
          dimensions,
        );
        let sample_value = load_position(sample_pixel, dimensions);
        if sample_value.w < 0.999999 {
          let delta = sample_value.xyz - center.xyz;
          let distance_to_sample = length(delta);
          let horizon = max(dot(normal, delta / max(distance_to_sample, 0.0001)) - 0.045, 0.0)
            * (1.0 - smoothstep(0.10, 2.1, distance_to_sample));
          if side == 0u {
            negative_horizon = max(negative_horizon, horizon);
          } else {
            positive_horizon = max(positive_horizon, horizon);
          }
        }
      }
    }
    horizon_sum += positive_horizon + negative_horizon;
  }
  let projected_fade = smoothstep(2.5, 6.0, projected_radius);
  let occlusion = clamp(horizon_sum * 0.235 * distance_fade * projected_fade, 0.0, 0.72);
  return vec2<f32>(1.0 - occlusion, center_view_depth);
}

@fragment
fn fs_denoise(@builtin(position) position: vec4<f32>) -> @location(0) vec2<f32> {
  let ao_dimensions = textureDimensions(source_ao);
  let ao_pixel = clamp(
    vec2<i32>(position.xy),
    vec2<i32>(0),
    vec2<i32>(ao_dimensions) - vec2<i32>(1),
  );
  let center = textureLoad(source_ao, ao_pixel, 0).rg;
  if center.y <= 0.0 {
    return vec2<f32>(1.0, 0.0);
  }
  var weighted_ao = 0.0;
  var total_weight = 0.0;
  for (var y = -1; y <= 1; y += 1) {
    for (var x = -1; x <= 1; x += 1) {
      let sample_pixel = clamp(
        ao_pixel + vec2<i32>(x, y),
        vec2<i32>(0),
        vec2<i32>(ao_dimensions) - vec2<i32>(1),
      );
      let source = textureLoad(source_ao, sample_pixel, 0).rg;
      let relative_depth_delta = abs(source.y - center.y) / max(center.y, 0.01);
      let spatial_weight = exp(-f32(x * x + y * y) * 0.48);
      let depth_weight = select(exp(-relative_depth_delta * 180.0), 0.0, source.y <= 0.0);
      let weight = spatial_weight * depth_weight;
      weighted_ao += source.x * weight;
      total_weight += weight;
    }
  }
  return vec2<f32>(select(1.0, weighted_ao / total_weight, total_weight > 0.0001), center.y);
}
