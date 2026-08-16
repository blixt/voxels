@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var shadow_map: texture_depth_2d_array;
@group(0) @binding(2) var shadow_sampler: sampler_comparison;

const SHADOW_VOXEL_NORMAL_BIAS: f32 = 0.22;
const SHADOW_TEXEL_NORMAL_BIAS: f32 = 0.60;

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

@fragment
fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
  let albedo = srgb_to_linear(input.color.rgb);
  let sun = normalize(frame.key_light_direction.xyz);
  let shadow = sun_visibility(input.world, input.normal);
  let sky_visibility = input.normal.y * 0.5 + 0.5;
  let interior_ambient = mix(1.0, 0.06, frame.interior.x);
  let ambient = mix(frame.ground_atmosphere.rgb, frame.sky_horizon.rgb * 0.52, sky_visibility)
    * interior_ambient * 0.92;
  let view_direction = normalize(frame.camera_time.xyz - input.world);
  let roughness = clamp(input.color.a, MIN_PERCEPTUAL_ROUGHNESS, 1.0);
  let no_v = max(dot(input.normal, view_direction), 0.0001);
  let ambient_fresnel = fresnel_schlick_roughness(no_v, DIELECTRIC_F0, roughness);
  let reflection_direction = reflect(-view_direction, input.normal);
  let reflection_radiance = environment_radiance(
    mix(reflection_direction, input.normal, roughness * roughness),
  );
  let ambient_diffuse = albedo * (vec3<f32>(1.0) - ambient_fresnel) * ambient;
  let ambient_specular = reflection_radiance * ambient_fresnel * interior_ambient;
  let direct = frame.key_light_radiance.rgb
    * evaluate_direct_dielectric(albedo, roughness, input.normal, view_direction, sun)
    * shadow
    * frame.key_light_direction.w
    * 0.60;
  var color = ambient_diffuse + ambient_specular + direct;
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
