@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var scene_sampler: sampler;

struct VertexOutput {
  @builtin(position) position: vec4<f32>,
  @location(0) uv: vec2<f32>,
};

fn pbr_neutral(color_in: vec3<f32>) -> vec3<f32> {
  let minimum = min(color_in.r, min(color_in.g, color_in.b));
  let offset = select(0.04, minimum - minimum * minimum / 0.16, minimum < 0.08);
  let color = color_in - vec3<f32>(offset);
  let peak = max(color.r, max(color.g, color.b));
  if peak < 0.76 {
    return color;
  }
  let new_peak = 1.0 - 0.0576 / (peak - 0.52);
  let desaturation = 1.0 / (0.15 * (peak - new_peak) + 1.0);
  return mix(vec3<f32>(new_peak), color * (new_peak / peak), desaturation);
}

fn linear_to_srgb(linear: vec3<f32>) -> vec3<f32> {
  let low = linear * 12.92;
  let high = 1.055 * pow(max(linear, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
  return select(high, low, linear <= vec3<f32>(0.0031308));
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
  let x = f32((index << 1u) & 2u);
  let y = f32(index & 2u);
  var output: VertexOutput;
  output.position = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
  output.uv = vec2<f32>(x, y);
  return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
  let hdr = max(textureSampleLevel(scene, scene_sampler, input.uv, 0.0).rgb, vec3<f32>(0.0));
  return vec4<f32>(linear_to_srgb(pbr_neutral(hdr)), 1.0);
}
