struct ShadowFrame {
  clip_from_world: mat4x4<f32>,
  camera_voxel: vec4<f32>,
};

@group(0) @binding(0) var<uniform> shadow_frame: ShadowFrame;
@group(1) @binding(0) var<storage, read> virtual_handles: array<u32>;
@group(1) @binding(1) var<storage, read> virtual_geometry_segment_0: array<u32>;
@group(1) @binding(2) var<storage, read> virtual_geometry_segment_1: array<u32>;

const CORNERS = array<vec2<i32>, 4>(
  vec2<i32>(0, 0),
  vec2<i32>(1, 0),
  vec2<i32>(1, 1),
  vec2<i32>(0, 1),
);
const STANDARD_STRIP = array<u32, 4>(1u, 2u, 0u, 3u);
const FLIPPED_STRIP = array<u32, 4>(0u, 1u, 3u, 2u);
const TRIANGLE_STRIP = array<u32, 4>(1u, 2u, 0u, 0u);
const CANONICAL_TRIANGLE_FLAG: u32 = 0x2000u;
const CANONICAL_TRIANGLE_SHADOW_OWNER_FLAG: u32 = 0x4000u;
const VIRTUAL_TRIANGLE_HANDLE_OFFSET: u32 = 2796202u;

fn virtual_geometry_word(handle: u32, word: u32) -> u32 {
  let source_word = (handle & 0x7fffffffu) * 6u + word;
  if (handle & 0x80000000u) == 0u {
    return virtual_geometry_segment_0[source_word];
  }
  return virtual_geometry_segment_1[source_word];
}

fn corner_ao(packed: u32, corner: u32) -> f32 {
  return f32((packed >> (corner * 2u)) & 3u) / 3.0;
}

fn unpack_signed_i3(value: u32) -> f32 {
  let bits = value & 7u;
  return f32(select(i32(bits), i32(bits) - 8, bits >= 4u));
}

fn surface_quad_flip(_face: u32, surface_shape: u32, packed_ao: u32) -> bool {
  if surface_shape != 0u {
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

fn shadow_vertex(
  vertex_index: u32,
  origin: vec3<i32>,
  extent_voxels: vec2<u32>,
  material_face: u32,
  ao: u32,
) -> vec4<f32> {
  let face = (material_face >> 16u) & 7u;
  let packed_material = material_face & 0xfff8ff1fu;
  let surface_shape = ((packed_material >> 8u) & 255u) | (((ao >> 20u) & 15u) << 8u);
  let canonical_triangle = (extent_voxels.x & CANONICAL_TRIANGLE_FLAG) != 0u;
  let canonical_shadow_owner =
    canonical_triangle && (extent_voxels.y & CANONICAL_TRIANGLE_SHADOW_OWNER_FLAG) != 0u;
  let custom_triangle = canonical_triangle;
  let extent = vec2<i32>(vec2<u32>(
    select(extent_voxels.x, 0u, custom_triangle),
    extent_voxels.y,
  ));
  let flip = !custom_triangle && surface_quad_flip(face, surface_shape, ao);
  let quad_corner = select(STANDARD_STRIP[vertex_index], FLIPPED_STRIP[vertex_index], flip);
  let corner = select(
    quad_corner,
    TRIANGLE_STRIP[vertex_index],
    custom_triangle && !canonical_shadow_owner,
  );
  let uv = CORNERS[corner];
  var local = vec3<i32>(0);
  if canonical_shadow_owner {
    let width = i32(
      (((extent_voxels.x >> 6u) & 31u) | ((extent_voxels.x >> 10u) & 32u)) + 1u,
    );
    let height = i32(
      (((extent_voxels.y >> 6u) & 31u) | ((extent_voxels.y >> 10u) & 32u)) + 1u,
    );
    switch face {
      case 0u: { local = vec3<i32>(1, uv.y * height, uv.x * width); }
      case 1u: { local = vec3<i32>(0, uv.y * height, uv.x * width); }
      case 2u: { local = vec3<i32>(uv.x * width, 1, uv.y * height); }
      case 3u: { local = vec3<i32>(uv.x * width, 0, uv.y * height); }
      case 4u: { local = vec3<i32>(uv.x * width, uv.y * height, 1); }
      default: { local = vec3<i32>(uv.x * width, uv.y * height, 0); }
    }
  } else if canonical_triangle {
    local = vec3<i32>(0, 1, 0);
  } else {
    switch face {
      case 0u: { local = vec3<i32>(1, uv.y * extent.y, uv.x * extent.x); }
      case 1u: { local = vec3<i32>(0, uv.y * extent.y, uv.x * extent.x); }
      case 2u: { local = vec3<i32>(uv.x * extent.x, 1, uv.y * extent.y); }
      case 3u: { local = vec3<i32>(uv.x * extent.x, 0, uv.y * extent.y); }
      case 4u: { local = vec3<i32>(uv.x * extent.x, uv.y * extent.y, 1); }
      default: { local = vec3<i32>(uv.x * extent.x, uv.y * extent.y, 0); }
    }
  }
  var world = vec3<f32>(origin + local) * shadow_frame.camera_voxel.w;
  if surface_shape != 0u && !canonical_triangle {
    world.y += unpack_signed_i3(surface_shape >> (corner * 3u))
      * shadow_frame.camera_voxel.w;
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
  );
}

@vertex
fn vs_virtual_surface_handle(
  @builtin(vertex_index) vertex_index: u32,
  @builtin(instance_index) instance_index: u32,
) -> @builtin(position) vec4<f32> {
  let handle = virtual_handles[instance_index];
  let packed_extent = virtual_geometry_word(handle, 3u);
  return shadow_vertex(
    vertex_index,
    vec3<i32>(
      bitcast<i32>(virtual_geometry_word(handle, 0u)),
      bitcast<i32>(virtual_geometry_word(handle, 1u)),
      bitcast<i32>(virtual_geometry_word(handle, 2u)),
    ),
    vec2<u32>(packed_extent & 0xffffu, packed_extent >> 16u),
    virtual_geometry_word(handle, 4u),
    virtual_geometry_word(handle, 5u),
  );
}

@vertex
fn vs_virtual_triangle_handle(
  @builtin(vertex_index) vertex_index: u32,
) -> @builtin(position) vec4<f32> {
  let handle = virtual_handles[VIRTUAL_TRIANGLE_HANDLE_OFFSET + vertex_index];
  let world = vec3<f32>(
    bitcast<f32>(virtual_geometry_word(handle, 0u)),
    bitcast<f32>(virtual_geometry_word(handle, 1u)),
    bitcast<f32>(virtual_geometry_word(handle, 2u)),
  ) * shadow_frame.camera_voxel.w;
  return shadow_frame.clip_from_world * vec4<f32>(world, 1.0);
}
