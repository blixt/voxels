@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var shadow_map: texture_depth_2d_array;
@group(0) @binding(2) var shadow_sampler: sampler_comparison;

struct VertexOut {
  @builtin(position) position: vec4<f32>,
  @location(0) world: vec3<f32>,
  @location(1) normal: vec3<f32>,
  @location(2) @interpolate(flat) color: vec4<f32>,
};

const CUBE_POSITIONS = array<vec3<f32>, 36>(
  vec3<f32>(-1.0,-1.0, 1.0), vec3<f32>( 1.0,-1.0, 1.0), vec3<f32>( 1.0, 1.0, 1.0),
  vec3<f32>(-1.0,-1.0, 1.0), vec3<f32>( 1.0, 1.0, 1.0), vec3<f32>(-1.0, 1.0, 1.0),
  vec3<f32>( 1.0,-1.0,-1.0), vec3<f32>(-1.0,-1.0,-1.0), vec3<f32>(-1.0, 1.0,-1.0),
  vec3<f32>( 1.0,-1.0,-1.0), vec3<f32>(-1.0, 1.0,-1.0), vec3<f32>( 1.0, 1.0,-1.0),
  vec3<f32>( 1.0,-1.0, 1.0), vec3<f32>( 1.0,-1.0,-1.0), vec3<f32>( 1.0, 1.0,-1.0),
  vec3<f32>( 1.0,-1.0, 1.0), vec3<f32>( 1.0, 1.0,-1.0), vec3<f32>( 1.0, 1.0, 1.0),
  vec3<f32>(-1.0,-1.0,-1.0), vec3<f32>(-1.0,-1.0, 1.0), vec3<f32>(-1.0, 1.0, 1.0),
  vec3<f32>(-1.0,-1.0,-1.0), vec3<f32>(-1.0, 1.0, 1.0), vec3<f32>(-1.0, 1.0,-1.0),
  vec3<f32>(-1.0, 1.0, 1.0), vec3<f32>( 1.0, 1.0, 1.0), vec3<f32>( 1.0, 1.0,-1.0),
  vec3<f32>(-1.0, 1.0, 1.0), vec3<f32>( 1.0, 1.0,-1.0), vec3<f32>(-1.0, 1.0,-1.0),
  vec3<f32>(-1.0,-1.0,-1.0), vec3<f32>( 1.0,-1.0,-1.0), vec3<f32>( 1.0,-1.0, 1.0),
  vec3<f32>(-1.0,-1.0,-1.0), vec3<f32>( 1.0,-1.0, 1.0), vec3<f32>(-1.0,-1.0, 1.0),
);

const CUBE_NORMALS = array<vec3<f32>, 6>(
  vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0, 0.0,-1.0),
  vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(-1.0,0.0, 0.0),
  vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(0.0,-1.0, 0.0),
);

fn rotate_by_quaternion(vector: vec3<f32>, quaternion: vec4<f32>) -> vec3<f32> {
  let t = 2.0 * cross(quaternion.xyz, vector);
  return vector + quaternion.w * t + cross(quaternion.xyz, t);
}

@vertex
fn vs_main(
  @builtin(vertex_index) vertex_index: u32,
  @location(0) center_half_x: vec4<f32>,
  @location(1) rotation: vec4<f32>,
  @location(2) half_yz: vec4<f32>,
  @location(3) color: vec4<f32>,
) -> VertexOut {
  let local = CUBE_POSITIONS[vertex_index] * vec3<f32>(center_half_x.w, half_yz.x, half_yz.y);
  let world = center_half_x.xyz + rotate_by_quaternion(local, rotation);
  let normal = normalize(rotate_by_quaternion(CUBE_NORMALS[vertex_index / 6u], rotation));
  var out: VertexOut;
  out.position = frame.view_projection * vec4<f32>(world, 1.0);
  out.world = world;
  out.normal = normal;
  out.color = color;
  return out;
}

fn srgb_to_linear(srgb: vec3<f32>) -> vec3<f32> {
  let low = srgb / 12.92;
  let high = pow((srgb + 0.055) / 1.055, vec3<f32>(2.4));
  return select(high, low, srgb <= vec3<f32>(0.04045));
}

fn cascade_shadow(world: vec3<f32>, normal: vec3<f32>, cascade: u32) -> f32 {
  let normal_offset = normal * (frame.viewport_voxel.z * 0.22 + frame.shadow_texel_sizes[cascade] * 0.6);
  let clip = frame.shadow_view_projection[cascade] * vec4<f32>(world + normal_offset, 1.0);
  let projected = clip.xyz / clip.w;
  let uv = projected.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
  if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) || projected.z <= 0.0 || projected.z >= 1.0 {
    return 1.0;
  }
  let depth_ref = projected.z - 0.00035;
  // WebGPU requires every comparison-sampler offset to be a shader-creation-time constant.
  // Spell out the compact 3x3 PCF kernel so Chrome, Metal, and native wgpu validate identically.
  var visibility = textureSampleCompareLevel(
    shadow_map, shadow_sampler, uv, i32(cascade), depth_ref, vec2<i32>(-1, -1),
  );
  visibility += textureSampleCompareLevel(
    shadow_map, shadow_sampler, uv, i32(cascade), depth_ref, vec2<i32>(0, -1),
  );
  visibility += textureSampleCompareLevel(
    shadow_map, shadow_sampler, uv, i32(cascade), depth_ref, vec2<i32>(1, -1),
  );
  visibility += textureSampleCompareLevel(
    shadow_map, shadow_sampler, uv, i32(cascade), depth_ref, vec2<i32>(-1, 0),
  );
  visibility += textureSampleCompareLevel(
    shadow_map, shadow_sampler, uv, i32(cascade), depth_ref, vec2<i32>(0, 0),
  );
  visibility += textureSampleCompareLevel(
    shadow_map, shadow_sampler, uv, i32(cascade), depth_ref, vec2<i32>(1, 0),
  );
  visibility += textureSampleCompareLevel(
    shadow_map, shadow_sampler, uv, i32(cascade), depth_ref, vec2<i32>(-1, 1),
  );
  visibility += textureSampleCompareLevel(
    shadow_map, shadow_sampler, uv, i32(cascade), depth_ref, vec2<i32>(0, 1),
  );
  visibility += textureSampleCompareLevel(
    shadow_map, shadow_sampler, uv, i32(cascade), depth_ref, vec2<i32>(1, 1),
  );
  return visibility / 9.0;
}

fn sun_visibility(world: vec3<f32>, normal: vec3<f32>) -> f32 {
  if frame.shadow_splits.w < 0.5 { return 1.0; }
  let view_depth = distance(world, frame.camera_time.xyz);
  var cascade = 0u;
  if view_depth > frame.shadow_splits.x { cascade = 1u; }
  if view_depth > frame.shadow_splits.y { cascade = 2u; }
  if view_depth > frame.shadow_splits.z { return 1.0; }
  return cascade_shadow(world, normal, cascade);
}

@fragment
fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
  let albedo = srgb_to_linear(input.color.rgb);
  let sun = normalize(frame.sun_direction.xyz);
  let diffuse = max(dot(input.normal, sun), 0.0);
  let shadow = sun_visibility(input.world, input.normal);
  let sky_visibility = input.normal.y * 0.5 + 0.5;
  let interior_ambient = mix(1.0, 0.06, frame.interior.x);
  let ambient = mix(frame.ground_atmosphere.rgb, frame.sky_horizon.rgb * 0.52, sky_visibility)
    * interior_ambient * 0.92;
  let direct = frame.sun_radiance.rgb * diffuse * mix(0.03, 1.0, shadow)
    * mix(1.0, 0.10, frame.interior.x) * 0.18;
  let view_direction = normalize(frame.camera_time.xyz - input.world);
  let half_direction = normalize(sun + view_direction);
  let specular = frame.sun_radiance.rgb
    * pow(max(dot(input.normal, half_direction), 0.0), 18.0)
    * input.color.a * mix(0.04, 1.0, shadow) * 0.055;
  var color = albedo * (ambient + direct) + specular;
  let camera_to_surface = input.world - frame.camera_time.xyz;
  let distance_to_camera = length(camera_to_surface);
  let fog_direction = camera_to_surface / max(distance_to_camera, 0.0001);
  let average_height = max((input.world.y + frame.camera_time.y) * 0.5, 0.0);
  let height_density = exp(-average_height * frame.fog_exposure.x);
  let optical_depth = distance_to_camera * frame.ground_atmosphere.w * height_density * frame.render_options.y;
  let transmittance = exp(-optical_depth);
  let fog_radiance = mix(frame.sky_horizon.rgb, frame.sky_zenith.rgb, pow(max(fog_direction.y, 0.0), 0.42));
  color = color * transmittance + fog_radiance * (1.0 - transmittance);
  color = max(color * frame.fog_exposure.y * frame.interior.y, vec3<f32>(0.0));
  return vec4<f32>(color, 1.0);
}
