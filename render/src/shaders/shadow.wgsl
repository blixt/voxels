struct ShadowFrame {
  clip_from_world: mat4x4<f32>,
  camera_voxel: vec4<f32>,
  lod_options: vec4<f32>,
  lod_boundary_centres: array<vec4<f32>, 4>,
  lod_boundary_half_extents: array<vec4<f32>, 2>,
};

@group(0) @binding(0) var<uniform> shadow_frame: ShadowFrame;

const CORNERS = array<vec2<i32>, 4>(
  vec2<i32>(0, 0),
  vec2<i32>(1, 0),
  vec2<i32>(1, 1),
  vec2<i32>(0, 1),
);
const STANDARD_STRIP = array<u32, 4>(1u, 2u, 0u, 3u);
const FLIPPED_STRIP = array<u32, 4>(0u, 1u, 3u, 2u);
const MORPH_CLOSURE_EXTENT_FLAG: u32 = 0x8000u;

fn corner_ao(packed: u32, corner: u32) -> f32 {
  return f32((packed >> (corner * 2u)) & 3u) / 3.0;
}

fn unpack_signed_i16(value: u32) -> f32 {
  let bits = value & 65535u;
  return f32(select(i32(bits), i32(bits) - 65536, bits >= 32768u));
}

fn unpack_signed_i3(value: u32) -> f32 {
  let bits = value & 7u;
  return f32(select(i32(bits), i32(bits) - 8, bits >= 4u));
}

fn surface_quad_flip(face: u32, surface_shape: u32, packed_ao: u32) -> bool {
  if face == 2u && surface_shape != 0u {
    let diagonal_02 = abs(
      unpack_signed_i3(surface_shape) - unpack_signed_i3(surface_shape >> 6u),
    );
    let diagonal_13 = abs(
      unpack_signed_i3(surface_shape >> 3u) - unpack_signed_i3(surface_shape >> 9u),
    );
    return diagonal_02 > diagonal_13;
  }
  return corner_ao(packed_ao, 0u) + corner_ao(packed_ao, 2u)
    > corner_ao(packed_ao, 1u) + corner_ao(packed_ao, 3u);
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

fn lod_boundary_half_extent(boundary: u32) -> f32 {
  return shadow_frame.lod_boundary_half_extents[boundary / 4u][boundary & 3u];
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
  let half_extent = lod_boundary_half_extent(boundary);
  let delta = abs(world.xz - lod_boundary_center(boundary));
  let inside = half_extent - max(delta.x, delta.y);
  let width = max(1.6, half_extent * 0.02);
  return 1.0 - smoothstep(0.0, width, inside);
}

fn surface_shape_blend(world: vec3<f32>, material: u32) -> f32 {
  if shadow_frame.lod_options.w < 0.5 || (material & 0x80000000u) == 0u {
    return 0.0;
  }
  let level = (material >> 27u) & 7u;
  let inner_half_extent = lod_boundary_half_extent(level);
  let inner_delta = abs(world.xz - lod_boundary_center(level));
  let inner_inside = inner_half_extent - max(inner_delta.x, inner_delta.y);
  let inner_width = max(1.6, inner_half_extent * 0.02);
  let inner_blend = smoothstep(0.0, inner_width, -inner_inside);
  if level >= 7u {
    return inner_blend;
  }
  let outer_boundary = level + 1u;
  let outer_half_extent = lod_boundary_half_extent(outer_boundary);
  let outer_delta = abs(world.xz - lod_boundary_center(outer_boundary));
  let outer_inside = outer_half_extent - max(outer_delta.x, outer_delta.y);
  let outer_width = max(1.6, outer_half_extent * 0.02);
  return min(inner_blend, smoothstep(0.0, outer_width, outer_inside));
}

fn shadow_vertex(
  vertex_index: u32,
  origin: vec3<i32>,
  extent_voxels: vec2<u32>,
  material_face: u32,
  ao: u32,
  morph_heights: u32,
  morph_geometry: bool,
) -> vec4<f32> {
  let face = (material_face >> 16u) & 7u;
  let packed_material = material_face & 0xfff8ffffu;
  let surface_shape = ((packed_material >> 8u) & 255u) | (((ao >> 20u) & 15u) << 8u);
  let material = packed_material & 0xffff00ffu;
  let morph_closure = (extent_voxels.x & MORPH_CLOSURE_EXTENT_FLAG) != 0u;
  let extent = vec2<i32>(vec2<u32>(
    extent_voxels.x & ~MORPH_CLOSURE_EXTENT_FLAG,
    extent_voxels.y,
  ));
  let flip = surface_quad_flip(face, surface_shape, ao);
  let corner = select(STANDARD_STRIP[vertex_index], FLIPPED_STRIP[vertex_index], flip);
  let uv = CORNERS[corner];
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
  if surface_shape != 0u {
    world.y += unpack_signed_i3(surface_shape >> (corner * 3u))
      * shadow_frame.camera_voxel.w
      * surface_shape_blend(world, material);
  }
  if morph_geometry {
    let parent_blend = surface_parent_blend(world, material);
    let morph_blend = select(parent_blend, 1.0 - parent_blend, morph_closure);
    world.y += surface_morph_delta(morph_heights, uv.y)
      * shadow_frame.camera_voxel.w
      * morph_blend;
  }
  return shadow_frame.clip_from_world * vec4<f32>(world, 1.0);
}

@vertex
fn vs_main_fixed(
  @builtin(vertex_index) vertex_index: u32,
  @location(0) origin: vec3<i32>,
  @location(1) extent_voxels: vec2<u32>,
  @location(2) material_face: u32,
  @location(3) ao: u32,
) -> @builtin(position) vec4<f32> {
  return shadow_vertex(
    vertex_index,
    origin,
    extent_voxels,
    material_face,
    ao,
    0u,
    false,
  );
}

@vertex
fn vs_main_morph(
  @builtin(vertex_index) vertex_index: u32,
  @location(0) origin: vec3<i32>,
  @location(1) extent_voxels: vec2<u32>,
  @location(2) material_face: u32,
  @location(3) ao: u32,
  @location(4) morph_heights: u32,
) -> @builtin(position) vec4<f32> {
  return shadow_vertex(
    vertex_index,
    origin,
    extent_voxels,
    material_face,
    ao,
    morph_heights,
    true,
  );
}
