struct ShadowFrame {
  clip_from_world: mat4x4<f32>,
  camera_voxel: vec4<f32>,
};

@group(0) @binding(0) var<uniform> shadow_frame: ShadowFrame;

const CORNERS = array<vec2<i32>, 4>(
  vec2<i32>(0, 0),
  vec2<i32>(1, 0),
  vec2<i32>(1, 1),
  vec2<i32>(0, 1),
);
const STANDARD_DIAGONAL = array<u32, 6>(0u, 1u, 2u, 0u, 2u, 3u);

@vertex
fn vs_main(
  @builtin(vertex_index) vertex_index: u32,
  @location(0) origin: vec3<i32>,
  @location(1) extent_voxels: vec2<u32>,
  @location(2) material_face: u32,
) -> @builtin(position) vec4<f32> {
  let face = (material_face >> 16u) & 7u;
  let extent = vec2<i32>(extent_voxels);
  let uv = CORNERS[STANDARD_DIAGONAL[vertex_index]];
  var local = vec3<i32>(0);
  switch face {
    case 0u: { local = vec3<i32>(1, uv.y * extent.y, uv.x * extent.x); }
    case 1u: { local = vec3<i32>(0, uv.y * extent.y, uv.x * extent.x); }
    case 2u: { local = vec3<i32>(uv.x * extent.x, 1, uv.y * extent.y); }
    case 3u: { local = vec3<i32>(uv.x * extent.x, 0, uv.y * extent.y); }
    case 4u: { local = vec3<i32>(uv.x * extent.x, uv.y * extent.y, 1); }
    default: { local = vec3<i32>(uv.x * extent.x, uv.y * extent.y, 0); }
  }
  let world = vec3<f32>(origin + local) * shadow_frame.camera_voxel.w;
  return shadow_frame.clip_from_world * vec4<f32>(world, 1.0);
}
