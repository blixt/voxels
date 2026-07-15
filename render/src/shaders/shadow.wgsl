struct ShadowFrame {
  clip_from_world: mat4x4<f32>,
  camera_voxel: vec4<f32>,
  lod_options: vec4<f32>,
};

@group(0) @binding(0) var<uniform> shadow_frame: ShadowFrame;

struct VertexOut {
  @builtin(position) position: vec4<f32>,
  @location(0) world: vec3<f32>,
  @location(1) @interpolate(flat) material: u32,
};

const CORNERS = array<vec2<f32>, 4>(
  vec2<f32>(0.0, 0.0),
  vec2<f32>(1.0, 0.0),
  vec2<f32>(1.0, 1.0),
  vec2<f32>(0.0, 1.0),
);
const STANDARD_DIAGONAL = array<u32, 6>(0u, 1u, 2u, 0u, 2u, 3u);

@vertex
fn vs_main(
  @builtin(vertex_index) vertex_index: u32,
  @location(0) origin: vec3<f32>,
  @location(1) extent_voxels: vec2<u32>,
  @location(2) material_face: u32,
) -> VertexOut {
  let face = (material_face >> 16u) & 7u;
  let material = material_face & 0xfff8ffffu;
  let extent = vec2<f32>(extent_voxels) * shadow_frame.camera_voxel.w;
  let uv = CORNERS[STANDARD_DIAGONAL[vertex_index]];
  var local = vec3<f32>(0.0);
  switch face {
    case 0u: { local = vec3<f32>(shadow_frame.camera_voxel.w, uv.y * extent.y, uv.x * extent.x); }
    case 1u: { local = vec3<f32>(0.0, uv.y * extent.y, uv.x * extent.x); }
    case 2u: { local = vec3<f32>(uv.x * extent.x, shadow_frame.camera_voxel.w, uv.y * extent.y); }
    case 3u: { local = vec3<f32>(uv.x * extent.x, 0.0, uv.y * extent.y); }
    case 4u: { local = vec3<f32>(uv.x * extent.x, uv.y * extent.y, shadow_frame.camera_voxel.w); }
    default: { local = vec3<f32>(uv.x * extent.x, uv.y * extent.y, 0.0); }
  }
  let world = origin + local;
  var out: VertexOut;
  out.position = shadow_frame.clip_from_world * vec4<f32>(world, 1.0);
  out.world = world;
  out.material = material;
  return out;
}

fn coarser_owns_boundary(distance_xz: f32, boundary: u32) -> bool {
  var split = 9.5;
  switch boundary {
    case 1u: { split = 23.5; }
    case 2u: { split = 48.0; }
    case 3u: { split = 96.0; }
    case 4u: { split = 192.0; }
    case 5u: { split = 384.0; }
    default: {}
  }
  return distance_xz >= split;
}

fn owns_lod_surface(world: vec3<f32>, packed_material: u32) -> bool {
  let far_surface = (packed_material & 0x80000000u) != 0u;
  let material = packed_material & 0xffffu;
  if !far_surface && shadow_frame.lod_options.x < 0.5 && (material == 8u || material == 9u) {
    return false;
  }
  if shadow_frame.lod_options.w > 0.5 {
    return true;
  }
  if shadow_frame.lod_options.z < 0.5 {
    return true;
  }
  if !far_surface && shadow_frame.lod_options.y < 0.5 {
    return true;
  }
  let distance_xz = distance(world.xz, shadow_frame.camera_voxel.xz);
  if !far_surface {
    return !coarser_owns_boundary(distance_xz, 0u);
  }
  let level = (packed_material >> 27u) & 7u;
  var owns = coarser_owns_boundary(distance_xz, level);
  if level < 5u {
    owns = owns && !coarser_owns_boundary(distance_xz, level + 1u);
  }
  return owns;
}

@fragment
fn fs_main(input: VertexOut) {
  if !owns_lod_surface(input.world, input.material) {
    discard;
  }
}
