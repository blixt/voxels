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
};

@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var shadow_map: texture_depth_2d_array;
@group(0) @binding(2) var shadow_sampler: sampler_comparison;

struct VertexOut {
  @builtin(position) position: vec4<f32>,
  @location(0) world: vec3<f32>,
  @location(1) normal: vec3<f32>,
  @location(2) @interpolate(flat) material: u32,
  @location(3) ao: f32,
};

const CORNERS = array<vec2<f32>, 4>(
  vec2<f32>(0.0, 0.0),
  vec2<f32>(1.0, 0.0),
  vec2<f32>(1.0, 1.0),
  vec2<f32>(0.0, 1.0),
);
const STANDARD_DIAGONAL = array<u32, 6>(0u, 1u, 2u, 0u, 2u, 3u);
const FLIPPED_DIAGONAL = array<u32, 6>(0u, 1u, 3u, 1u, 2u, 3u);

fn corner_ao(packed: u32, corner: u32) -> f32 {
  return f32((packed >> (corner * 2u)) & 3u) / 3.0;
}

@vertex
fn vs_main(
  @builtin(vertex_index) vertex_index: u32,
  @location(0) origin: vec3<f32>,
  @location(1) face: u32,
  @location(2) extent: vec2<f32>,
  @location(3) material: u32,
  @location(4) ao: u32,
) -> VertexOut {
  let flip = corner_ao(ao, 0u) + corner_ao(ao, 2u) > corner_ao(ao, 1u) + corner_ao(ao, 3u);
  let corner = select(STANDARD_DIAGONAL[vertex_index], FLIPPED_DIAGONAL[vertex_index], flip);
  let uv = CORNERS[corner];
  var local = vec3<f32>(0.0);
  var normal = vec3<f32>(0.0);
  switch face {
    case 0u: { local = vec3<f32>(frame.viewport_voxel.z, uv.y * extent.y, uv.x * extent.x); normal.x = 1.0; }
    case 1u: { local = vec3<f32>(0.0, uv.y * extent.y, uv.x * extent.x); normal.x = -1.0; }
    case 2u: { local = vec3<f32>(uv.x * extent.x, frame.viewport_voxel.z, uv.y * extent.y); normal.y = 1.0; }
    case 3u: { local = vec3<f32>(uv.x * extent.x, 0.0, uv.y * extent.y); normal.y = -1.0; }
    case 4u: { local = vec3<f32>(uv.x * extent.x, uv.y * extent.y, frame.viewport_voxel.z); normal.z = 1.0; }
    default: { local = vec3<f32>(uv.x * extent.x, uv.y * extent.y, 0.0); normal.z = -1.0; }
  }
  let world = origin + local;
  var out: VertexOut;
  out.position = frame.view_projection * vec4<f32>(world, 1.0);
  out.world = world;
  out.normal = normal;
  out.material = material;
  out.ao = corner_ao(ao, corner);
  return out;
}

fn material_color(material: u32) -> vec3<f32> {
  switch material {
    case 1u: { return vec3<f32>(0.18, 0.42, 0.12); }
    case 2u: { return vec3<f32>(0.36, 0.20, 0.095); }
    case 3u: { return vec3<f32>(0.34, 0.38, 0.43); }
    case 4u: { return vec3<f32>(0.72, 0.53, 0.25); }
    case 5u: { return vec3<f32>(0.76, 0.86, 0.91); }
    case 6u: { return vec3<f32>(0.56, 0.25, 0.15); }
    case 7u: { return vec3<f32>(0.12, 0.15, 0.20); }
    case 8u: { return vec3<f32>(0.31, 0.15, 0.055); }
    case 9u: { return vec3<f32>(0.08, 0.30, 0.10); }
    case 10u: { return vec3<f32>(0.12, 0.32, 0.14); }
    case 11u: { return vec3<f32>(0.58, 0.55, 0.44); }
    case 12u: { return vec3<f32>(0.62, 0.20, 0.075); }
    default: { return vec3<f32>(1.0, 0.0, 1.0); }
  }
}

fn srgb_to_linear(srgb: vec3<f32>) -> vec3<f32> {
  let low = srgb / 12.92;
  let high = pow((srgb + 0.055) / 1.055, vec3<f32>(2.4));
  return select(high, low, srgb <= vec3<f32>(0.04045));
}

fn material_roughness(material: u32) -> f32 {
  switch material {
    case 3u: { return 0.68; }
    case 5u: { return 0.42; }
    case 7u: { return 0.74; }
    case 9u: { return 0.64; }
    case 11u: { return 0.58; }
    default: { return 0.86; }
  }
}

fn material_macro_tint(material: u32, world: vec3<f32>) -> vec3<f32> {
  let wave = sin(world.x * 0.17 + sin(world.z * 0.11) * 1.7) * 0.5 + 0.5;
  switch material {
    case 1u: { return mix(vec3<f32>(0.80, 0.94, 0.77), vec3<f32>(1.10, 1.02, 0.72), wave); }
    case 9u: { return mix(vec3<f32>(0.72, 0.92, 0.76), vec3<f32>(1.08, 1.01, 0.78), wave); }
    case 10u: { return mix(vec3<f32>(0.76, 0.96, 0.82), vec3<f32>(1.03, 0.95, 0.72), wave); }
    case 3u: { return mix(vec3<f32>(0.82, 0.88, 0.96), vec3<f32>(1.08, 1.02, 0.91), wave); }
    case 11u: { return mix(vec3<f32>(0.90, 0.94, 1.02), vec3<f32>(1.08, 1.01, 0.86), wave); }
    default: { return vec3<f32>(mix(0.93, 1.06, wave)); }
  }
}

fn hash31(position: vec3<f32>) -> f32 {
  let value = dot(position, vec3<f32>(127.1, 311.7, 74.7));
  return fract(sin(value) * 43758.5453);
}

fn coarser_owns_boundary(distance_xz: f32, boundary: u32) -> bool {
  // Each split sits safely inside the guaranteed coverage of both adjacent rings. A continuous
  // radial cut keeps ownership complementary without turning height disagreement into visible
  // salt-and-pepper holes. The lower coarse mesh remains a crack-hiding underlay at the seam.
  var split = 9.5;
  switch boundary {
    case 1u: { split = 23.5; }
    case 2u: { split = 48.0; }
    case 3u: { split = 96.0; }
    default: {}
  }
  return distance_xz >= split;
}

fn owns_lod_surface(world: vec3<f32>, packed_material: u32) -> bool {
  let far_surface = (packed_material & 0x80000000u) != 0u;
  let material = packed_material & 0xffffu;
  if !far_surface && frame.lod_options.x < 0.5 && (material == 8u || material == 9u) {
    return false;
  }
  if frame.lod_options.w > 0.5 {
    return true;
  }
  if frame.lod_options.z < 0.5 {
    return true;
  }
  if !far_surface && frame.lod_options.y < 0.5 {
    return true;
  }
  let distance_xz = distance(world.xz, frame.camera_time.xz);
  if !far_surface {
    return !coarser_owns_boundary(distance_xz, 0u);
  }
  let level = (packed_material >> 28u) & 3u;
  var owns = coarser_owns_boundary(distance_xz, level);
  if level < 3u {
    owns = owns && !coarser_owns_boundary(distance_xz, level + 1u);
  }
  return owns;
}

fn cascade_shadow(world: vec3<f32>, normal: vec3<f32>, cascade: u32) -> f32 {
  let texel_world_size = frame.shadow_texel_sizes[cascade];
  let normal_offset = normal * (frame.viewport_voxel.z * 0.24 + texel_world_size * 0.65);
  let clip = frame.shadow_view_projection[cascade] * vec4<f32>(world + normal_offset, 1.0);
  let projected = clip.xyz / clip.w;
  let uv = projected.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
  if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) || projected.z <= 0.0 || projected.z >= 1.0 {
    return 1.0;
  }
  let layer = i32(cascade);
  let depth_ref = projected.z - 0.00035;
  var visibility = 0.0;
  visibility += textureSampleCompareLevel(shadow_map, shadow_sampler, uv, layer, depth_ref, vec2<i32>(-1, -1));
  visibility += textureSampleCompareLevel(shadow_map, shadow_sampler, uv, layer, depth_ref, vec2<i32>( 0, -1));
  visibility += textureSampleCompareLevel(shadow_map, shadow_sampler, uv, layer, depth_ref, vec2<i32>( 1, -1));
  visibility += textureSampleCompareLevel(shadow_map, shadow_sampler, uv, layer, depth_ref, vec2<i32>(-1,  0));
  visibility += textureSampleCompareLevel(shadow_map, shadow_sampler, uv, layer, depth_ref, vec2<i32>( 0,  0));
  visibility += textureSampleCompareLevel(shadow_map, shadow_sampler, uv, layer, depth_ref, vec2<i32>( 1,  0));
  visibility += textureSampleCompareLevel(shadow_map, shadow_sampler, uv, layer, depth_ref, vec2<i32>(-1,  1));
  visibility += textureSampleCompareLevel(shadow_map, shadow_sampler, uv, layer, depth_ref, vec2<i32>( 0,  1));
  visibility += textureSampleCompareLevel(shadow_map, shadow_sampler, uv, layer, depth_ref, vec2<i32>( 1,  1));
  return visibility / 9.0;
}

fn sun_visibility(world: vec3<f32>, normal: vec3<f32>) -> f32 {
  if frame.shadow_splits.w < 0.5 {
    return 1.0;
  }
  let view_depth = max(dot(world - frame.camera_time.xyz, frame.camera_forward.xyz), 0.0);
  var cascade = 0u;
  if view_depth > frame.shadow_splits.x { cascade = 1u; }
  if view_depth > frame.shadow_splits.y { cascade = 2u; }
  if view_depth > frame.shadow_splits.z { return 1.0; }
  let visibility = cascade_shadow(world, normal, cascade);
  if cascade >= 2u {
    return visibility;
  }
  var near_split = 0.0;
  if cascade > 0u {
    near_split = frame.shadow_splits[cascade - 1u];
  }
  let far_split = frame.shadow_splits[cascade];
  let blend = smoothstep(mix(near_split, far_split, 0.88), far_split, view_depth);
  return mix(visibility, cascade_shadow(world, normal, cascade + 1u), blend);
}

fn water_wave_normal(world: vec3<f32>) -> vec3<f32> {
  let time = frame.camera_time.w;
  let phase_a = dot(world.xz, vec2<f32>(1.08, 0.46)) * 2.1 + time * 0.82;
  let phase_b = dot(world.xz, vec2<f32>(-0.38, 1.27)) * 3.4 - time * 0.57;
  let phase_c = dot(world.xz, vec2<f32>(0.82, -1.11)) * 6.8 + time * 1.16;
  let slope = vec2<f32>(1.08, 0.46) * cos(phase_a) * 0.075
    + vec2<f32>(-0.38, 1.27) * cos(phase_b) * 0.048
    + vec2<f32>(0.82, -1.11) * cos(phase_c) * 0.018;
  return normalize(vec3<f32>(-slope.x, 1.0, -slope.y));
}

fn reflected_environment(direction: vec3<f32>) -> vec3<f32> {
  let sky_height = pow(clamp(direction.y * 0.5 + 0.5, 0.0, 1.0), 0.58);
  var radiance = mix(frame.ground_atmosphere.rgb, frame.sky_zenith.rgb, sky_height);
  radiance = mix(radiance, frame.sky_horizon.rgb, exp(-abs(direction.y) * 5.5) * 0.46);
  let sun = normalize(frame.sun_direction.xyz);
  radiance += frame.sun_radiance.rgb * pow(max(dot(direction, sun), 0.0), 420.0) * 0.72;
  return radiance;
}

@fragment
fn fs_water_depth(input: VertexOut) -> @location(0) vec4<f32> {
  if !owns_lod_surface(input.world, input.material) || (input.material & 0xffffu) != 13u {
    discard;
  }
  return vec4<f32>(0.0);
}

@fragment
fn fs_water(input: VertexOut) -> @location(0) vec4<f32> {
  if !owns_lod_surface(input.world, input.material) {
    discard;
  }
  let material = input.material & 0xffffu;
  if material != 13u {
    discard;
  }
  let view_direction = normalize(frame.camera_time.xyz - input.world);
  let normal = select(input.normal, water_wave_normal(input.world), input.normal.y > 0.5);
  let facing = clamp(dot(normal, view_direction), 0.0, 1.0);
  let fresnel = 0.02037 + 0.97963 * pow(1.0 - facing, 5.0);
  let reflection = reflected_environment(reflect(-view_direction, normal));
  let deep = srgb_to_linear(vec3<f32>(0.025, 0.18, 0.24));
  let shallow = srgb_to_linear(vec3<f32>(0.09, 0.39, 0.43));
  let wave_light = sin(input.world.x * 2.7 + frame.camera_time.w * 0.9)
    * sin(input.world.z * 2.2 - frame.camera_time.w * 0.7) * 0.5 + 0.5;
  var color = mix(deep, shallow, 0.28 + wave_light * 0.18);
  color = mix(color, reflection, clamp(fresnel * 0.88 + 0.08, 0.0, 0.92));
  let sun = normalize(frame.sun_direction.xyz);
  let half_direction = normalize(sun + view_direction);
  let visibility = sun_visibility(input.world, normal);
  color += frame.sun_radiance.rgb
    * pow(max(dot(normal, half_direction), 0.0), 260.0)
    * mix(0.18, 1.0, visibility)
    * 0.34;

  let camera_to_surface = input.world - frame.camera_time.xyz;
  let distance_to_camera = length(camera_to_surface);
  let fog_view_direction = camera_to_surface / max(distance_to_camera, 0.0001);
  let average_height = max((input.world.y + frame.camera_time.y) * 0.5, 0.0);
  let height_density = exp(-average_height * frame.fog_exposure.x);
  let optical_depth = distance_to_camera * frame.ground_atmosphere.w * height_density * frame.render_options.y;
  let transmittance = exp(-optical_depth);
  let sky_factor = pow(max(fog_view_direction.y, 0.0), 0.42);
  let fog_radiance = mix(frame.sky_horizon.rgb, frame.sky_zenith.rgb, sky_factor);
  color = color * transmittance + fog_radiance * (1.0 - transmittance);
  color = max(color * frame.fog_exposure.y, vec3<f32>(0.0));
  let alpha = mix(0.68, 0.91, fresnel);
  return vec4<f32>(color * alpha, alpha);
}

@fragment
fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
  if !owns_lod_surface(input.world, input.material) {
    discard;
  }
  let material = input.material & 0xffffu;
  let sun = normalize(frame.sun_direction.xyz);
  let diffuse = max(dot(input.normal, sun), 0.0);
  let shadow = sun_visibility(input.world, input.normal);
  let sky_visibility = input.normal.y * 0.5 + 0.5;
  let cell = floor(input.world / frame.viewport_voxel.z);
  let grain = mix(0.88, 1.12, hash31(cell + vec3<f32>(f32(material) * 3.1)));
  let fine_grain = mix(0.96, 1.04, hash31(floor(input.world * 28.0)));
  let ambient_occlusion = select(1.0, mix(0.52, 1.0, input.ao), frame.render_options.x > 0.5);
  let sky_irradiance = mix(frame.ground_atmosphere.rgb, frame.sky_horizon.rgb * 0.48, sky_visibility);
  let bounce = frame.ground_atmosphere.rgb * max(-input.normal.y, 0.0) * 0.35;
  let direct = frame.sun_radiance.rgb * diffuse * mix(0.16, 1.0, shadow) * 0.19;
  let albedo = srgb_to_linear(material_color(material))
    * material_macro_tint(material, input.world)
    * grain
    * fine_grain;
  let view_direction = normalize(frame.camera_time.xyz - input.world);
  let half_direction = normalize(sun + view_direction);
  let roughness = material_roughness(material);
  let specular_power = mix(110.0, 5.0, roughness * roughness);
  let fresnel = 0.04 + 0.96 * pow(1.0 - max(dot(view_direction, half_direction), 0.0), 5.0);
  let specular = frame.sun_radiance.rgb
    * pow(max(dot(input.normal, half_direction), 0.0), specular_power)
    * fresnel
    * (1.0 - roughness * 0.72)
    * mix(0.16, 1.0, shadow)
    * 0.16;
  var color = albedo * ((sky_irradiance + bounce) * ambient_occlusion + direct) + specular;
  if material == 9u {
    let leaf_scatter = pow(max(dot(-sun, view_direction), 0.0), 3.0) * (1.0 - shadow * 0.55);
    color += albedo * frame.sun_radiance.rgb * leaf_scatter * 0.035;
  }
  let inside_position = input.world - input.normal * frame.viewport_voxel.z * 0.02;
  let voxel = floor(inside_position / frame.viewport_voxel.z);
  let targeted = frame.render_options.w > 0.5 && frame.target_voxel.w > 0.5 && all(abs(voxel - frame.target_voxel.xyz) < vec3<f32>(0.1));
  if targeted {
    let coordinate = fract(input.world / frame.viewport_voxel.z + vec3<f32>(0.0001));
    var edge = 1.0;
    if abs(input.normal.x) < 0.5 { edge = min(edge, min(coordinate.x, 1.0 - coordinate.x)); }
    if abs(input.normal.y) < 0.5 { edge = min(edge, min(coordinate.y, 1.0 - coordinate.y)); }
    if abs(input.normal.z) < 0.5 { edge = min(edge, min(coordinate.z, 1.0 - coordinate.z)); }
    let outline = 1.0 - smoothstep(0.045, 0.085, edge);
    color = mix(color, vec3<f32>(1.4, 1.08, 0.42), outline * 0.88);
  }
  let camera_to_surface = input.world - frame.camera_time.xyz;
  let distance_to_camera = length(camera_to_surface);
  let fog_view_direction = camera_to_surface / max(distance_to_camera, 0.0001);
  let average_height = max((input.world.y + frame.camera_time.y) * 0.5, 0.0);
  let height_density = exp(-average_height * frame.fog_exposure.x);
  let optical_depth = distance_to_camera * frame.ground_atmosphere.w * height_density * frame.render_options.y;
  let transmittance = exp(-optical_depth);
  let sky_factor = pow(max(fog_view_direction.y, 0.0), 0.42);
  var fog_radiance = mix(frame.sky_horizon.rgb, frame.sky_zenith.rgb, sky_factor);
  let sun_amount = max(dot(fog_view_direction, sun), 0.0);
  fog_radiance += frame.sun_radiance.rgb * pow(sun_amount, 32.0) * 0.012;
  color = color * transmittance + fog_radiance * (1.0 - transmittance);
  return vec4<f32>(max(color * frame.fog_exposure.y, vec3<f32>(0.0)), 1.0);
}
