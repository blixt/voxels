struct GlassInstance {
  rect: vec4<f32>,
  viewport_radius: vec4<f32>,
  fill: vec4<f32>,
  border: vec4<f32>,
  style: vec4<f32>,
};

@group(0) @binding(0) var<storage, read> instances: array<GlassInstance>;
@group(1) @binding(0) var backdrop: texture_2d<f32>;
@group(1) @binding(1) var backdrop_sampler: sampler;

struct VertexOutput {
  @builtin(position) position: vec4<f32>,
  @location(0) uv: vec2<f32>,
  @location(1) @interpolate(flat) instance_index: u32,
};

const CORNERS = array<vec2<f32>, 6>(
  vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
  vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
);

fn rounded_rect_distance(position: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
  let corner = abs(position) - (half_size - vec2<f32>(radius));
  return length(max(corner, vec2<f32>(0.0))) + min(max(corner.x, corner.y), 0.0) - radius;
}

fn height_at(position: vec2<f32>, half_size: vec2<f32>, radius: f32, bevel: f32) -> f32 {
  return smoothstep(0.0, bevel, -rounded_rect_distance(position, half_size, radius));
}

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

fn display_color(hdr: vec3<f32>) -> vec3<f32> {
  return linear_to_srgb(pbr_neutral(max(hdr, vec3<f32>(0.0))));
}

@vertex
fn vs_main(
  @builtin(vertex_index) vertex_index: u32,
  @builtin(instance_index) instance_index: u32,
) -> VertexOutput {
  let instance = instances[instance_index];
  let uv = CORNERS[vertex_index];
  let screen_position = instance.rect.xy + uv * instance.rect.zw;
  let viewport = instance.viewport_radius.xy;
  var output: VertexOutput;
  output.position = vec4<f32>(
    screen_position.x / viewport.x * 2.0 - 1.0,
    1.0 - screen_position.y / viewport.y * 2.0,
    0.0,
    1.0,
  );
  output.uv = uv;
  output.instance_index = instance_index;
  return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
  let instance = instances[input.instance_index];
  let size = instance.rect.zw;
  let position = input.uv * size - size * 0.5;
  let half_size = size * 0.5;
  let radius = min(instance.viewport_radius.z, min(half_size.x, half_size.y));
  let distance = rounded_rect_distance(position, half_size, radius);
  let antialias = max(fwidth(distance), 0.0001);
  let mask = 1.0 - smoothstep(-antialias, antialias, distance);
  if mask < 0.002 {
    discard;
  }

  let style = instance.style.x;
  if style > 3.5 {
    let dpr = instance.style.y;
    let ring = 1.0 - smoothstep(0.75 * dpr, 1.75 * dpr, abs(distance + 1.25 * dpr));
    let shadow = 1.0 - smoothstep(1.5 * dpr, 2.8 * dpr, abs(distance + 1.25 * dpr));
    let alpha = max(ring * instance.fill.a, shadow * instance.border.a) * mask;
    let color = mix(instance.border.rgb, instance.fill.rgb, ring);
    return vec4<f32>(color * alpha, alpha);
  }
  if style < 0.5 {
    let alpha = mask * instance.fill.a;
    return vec4<f32>(instance.fill.rgb * alpha, alpha);
  }

  let bevel = max(radius, instance.style.y);
  let epsilon = max(1.25 * instance.style.y, 1.0);
  let height_x = height_at(position + vec2<f32>(epsilon, 0.0), half_size, radius, bevel)
    - height_at(position - vec2<f32>(epsilon, 0.0), half_size, radius, bevel);
  let height_y = height_at(position + vec2<f32>(0.0, epsilon), half_size, radius, bevel)
    - height_at(position - vec2<f32>(0.0, epsilon), half_size, radius, bevel);
  var normal = normalize(vec3<f32>(-height_x, -height_y, 0.18));
  if style > 2.5 {
    let sphere = position / max(half_size, vec2<f32>(0.001));
    normal = normalize(vec3<f32>(sphere, sqrt(max(1.0 - dot(sphere, sphere), 0.0))));
  }

  let viewport = instance.viewport_radius.xy;
  let scene_uv = (instance.rect.xy + input.uv * size) / viewport;
  let lens = -position / max(half_size, vec2<f32>(0.001));
  let short_side = min(size.x, size.y);
  let control = style > 1.5;
  let bend = (normal.xy * select(0.12, 0.42, control) + lens * 0.025) * short_side;
  let offset = bend / viewport;
  var refracted = vec3<f32>(0.0);
  refracted.r = display_color(textureSampleLevel(backdrop, backdrop_sampler, scene_uv + offset * 0.88, 0.0).rgb).r;
  refracted.g = display_color(textureSampleLevel(backdrop, backdrop_sampler, scene_uv + offset, 0.0).rgb).g;
  refracted.b = display_color(textureSampleLevel(backdrop, backdrop_sampler, scene_uv + offset * 1.14, 0.0).rgb).b;

  var color = mix(refracted, instance.fill.rgb, instance.fill.a);
  let edge_depth = max(-distance, 0.0);
  let rim = 1.0 - smoothstep(0.0, max(1.5 * instance.style.y, antialias), edge_depth);
  color += vec3<f32>(0.82, 0.88, 0.98) * rim * select(0.18, 0.09, control);
  let border = rim * instance.border.a;
  color = mix(color, instance.border.rgb, border);
  let alpha = mask * clamp(0.62 + instance.fill.a * 0.42, 0.0, 1.0);
  return vec4<f32>(color * alpha, alpha);
}
