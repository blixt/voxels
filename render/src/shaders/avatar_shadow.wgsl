struct ShadowFrame {
  clip_from_world: mat4x4<f32>,
  camera_voxel: vec4<f32>,
};

@group(0) @binding(0) var<uniform> shadow_frame: ShadowFrame;

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
  @location(3) _color: vec4<f32>,
) -> @builtin(position) vec4<f32> {
  let local = CUBE_POSITIONS[vertex_index] * vec3<f32>(center_half_x.w, half_yz.x, half_yz.y);
  let world = center_half_x.xyz + rotate_by_quaternion(local, rotation);
  return shadow_frame.clip_from_world * vec4<f32>(world, 1.0);
}
