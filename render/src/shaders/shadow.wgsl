struct ShadowFrame {
  clip_from_world: mat4x4<f32>,
  camera_voxel: vec4<f32>,
  lod_options: vec4<f32>,
  lod_boundary_centres: array<vec4<f32>, 4>,
};

@group(0) @binding(0) var<uniform> shadow_frame: ShadowFrame;

const CORNERS = array<vec2<i32>, 4>(
  vec2<i32>(0, 0),
  vec2<i32>(1, 0),
  vec2<i32>(1, 1),
  vec2<i32>(0, 1),
);
const STANDARD_STRIP = array<u32, 4>(1u, 2u, 0u, 3u);
const MORPH_CLOSURE_EXTENT_FLAG: u32 = 0x8000u;

fn unpack_signed_i16(value: u32) -> f32 {
  let bits = value & 65535u;
  return f32(select(i32(bits), i32(bits) - 65536, bits >= 32768u));
}

fn surface_morph_delta(morph_heights: u32, vertical_corner: i32) -> f32 {
  let bottom = unpack_signed_i16(morph_heights);
  let top = unpack_signed_i16(morph_heights >> 16u);
  return select(bottom, top, vertical_corner != 0);
}

fn lod_boundary_center(boundary: u32) -> vec2<f32> {
  let packed = shadow_frame.lod_boundary_centres[boundary / 2u];
  return select(packed.xy, packed.zw, (boundary & 1u) != 0u);
}

fn surface_parent_blend(world: vec3<f32>, material: u32) -> f32 {
  if shadow_frame.lod_options.w < 0.5 || (material & 0x80000000u) == 0u {
    return 0.0;
  }
  let level = (material >> 27u) & 7u;
  if level >= 7u {
    return 0.0;
  }
  let boundary = level + 1u;
  var half_extent = 25.6;
  switch boundary {
    case 2u: { half_extent = 51.2; }
    case 3u: { half_extent = 102.4; }
    case 4u: { half_extent = 204.8; }
    case 5u: { half_extent = 409.6; }
    case 6u: { half_extent = 819.2; }
    case 7u: { half_extent = 1638.4; }
    default: {}
  }
  let delta = abs(world.xz - lod_boundary_center(boundary));
  let inside = half_extent - max(delta.x, delta.y);
  let width = max(3.2, half_extent * 0.025);
  return 1.0 - smoothstep(0.0, width, inside);
}

@vertex
fn vs_main(
  @builtin(vertex_index) vertex_index: u32,
  @location(0) origin: vec3<i32>,
  @location(1) extent_voxels: vec2<u32>,
  @location(2) material_face: u32,
  @location(4) morph_heights: u32,
) -> @builtin(position) vec4<f32> {
  let face = (material_face >> 16u) & 7u;
  let material = material_face & 0xfff8ffffu;
  let morph_closure = (extent_voxels.x & MORPH_CLOSURE_EXTENT_FLAG) != 0u;
  let extent = vec2<i32>(vec2<u32>(
    extent_voxels.x & ~MORPH_CLOSURE_EXTENT_FLAG,
    extent_voxels.y,
  ));
  let uv = CORNERS[STANDARD_STRIP[vertex_index]];
  var local = vec3<i32>(0);
  switch face {
    case 0u: { local = vec3<i32>(1, uv.y * extent.y, uv.x * extent.x); }
    case 1u: { local = vec3<i32>(0, uv.y * extent.y, uv.x * extent.x); }
    case 2u: { local = vec3<i32>(uv.x * extent.x, 1, uv.y * extent.y); }
    case 3u: { local = vec3<i32>(uv.x * extent.x, 0, uv.y * extent.y); }
    case 4u: { local = vec3<i32>(uv.x * extent.x, uv.y * extent.y, 1); }
    default: { local = vec3<i32>(uv.x * extent.x, uv.y * extent.y, 0); }
  }
  var world = vec3<f32>(origin + local) * shadow_frame.camera_voxel.w;
  let parent_blend = surface_parent_blend(world, material);
  let morph_blend = select(parent_blend, 1.0 - parent_blend, morph_closure);
  world.y += surface_morph_delta(morph_heights, uv.y)
    * shadow_frame.camera_voxel.w
    * morph_blend;
  return shadow_frame.clip_from_world * vec4<f32>(world, 1.0);
}
