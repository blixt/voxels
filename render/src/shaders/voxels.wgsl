struct LocalLight {
  position_radius: vec4<f32>,
  color_intensity: vec4<f32>,
};

struct LocalLightUniform {
  metadata: vec4<u32>,
  lights: array<LocalLight, 16>,
};

@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var shadow_map: texture_depth_2d_array;
@group(0) @binding(2) var shadow_sampler: sampler_comparison;
@group(0) @binding(3) var material_albedo: texture_2d_array<f32>;
@group(0) @binding(4) var material_surface: texture_2d_array<f32>;
@group(0) @binding(5) var material_sampler: sampler;
@group(0) @binding(6) var<uniform> local_light_uniform: LocalLightUniform;
@group(1) @binding(0) var opaque_scene: texture_2d<f32>;
@group(1) @binding(1) var opaque_scene_sampler: sampler;
@group(1) @binding(2) var opaque_depth: texture_depth_2d;
@group(2) @binding(0) var filtered_spatial_ao: texture_2d<f32>;

struct CutTransitionUniform {
  // x is normalized phase; y is 0 stable, 1 outgoing, or 2 incoming.
  phase_role: vec4<f32>,
  lod_boundary_centres: array<vec4<f32>, 4>,
  lod_boundary_half_extents: array<vec4<f32>, 2>,
};

@group(3) @binding(0) var<uniform> cut_transition: CutTransitionUniform;

override MATERIAL_DETAIL: u32 = 1u;
override CUT_TRANSITION: u32 = 0u;

struct VertexOut {
  @builtin(position) position: vec4<f32>,
  @location(0) world: vec3<f32>,
  @location(1) normal: vec3<f32>,
  @location(2) @interpolate(flat) material: u32,
  @location(3) ao: f32,
  @location(4) @interpolate(flat) terrain_lighting: vec2<f32>,
  @location(5) @interpolate(flat) source: u32,
  @location(6) surface_weather: vec2<f32>,
  // Screenshot-only integer attachment identity. Keeping this as an ordinary flat varying lets
  // the diagnostic pass execute the exact production vertex path, including morphs and cut
  // transitions, without changing the visible color pass or maintaining a second geometry model.
  @location(7) @interpolate(flat) terrain_identity: vec4<u32>,
};

const CORNERS = array<vec2<i32>, 4>(
  vec2<i32>(0, 0),
  vec2<i32>(1, 0),
  vec2<i32>(1, 1),
  vec2<i32>(0, 1),
);
const STANDARD_STRIP = array<u32, 4>(1u, 2u, 0u, 3u);
const FLIPPED_STRIP = array<u32, 4>(0u, 1u, 3u, 2u);
const TRIANGLE_STRIP = array<u32, 4>(1u, 2u, 0u, 0u);
const MORPH_CLOSURE_EXTENT_FLAG: u32 = 0x8000u;
const TRANSITION_TRIANGLE_FLAG: u32 = 0x4000u;
const TRANSITION_TRIANGLE_OFFSET_MASK: u32 = 0x01ffu;
const CANONICAL_TRIANGLE_FLAG: u32 = 0x2000u;
const CANONICAL_TRIANGLE_OFFSET_MASK: u32 = 0x003fu;
const GPU_SOURCE_SHIFT: u32 = 5u;
fn diagnostic_primitive_id(
  origin: vec3<i32>,
  extent_voxels: vec2<u32>,
  material_face: u32,
  ao: u32,
) -> u32 {
  var hash = 2166136261u;
  hash = diagnostic_hash_step(hash, bitcast<u32>(origin.x));
  hash = diagnostic_hash_step(hash, bitcast<u32>(origin.y));
  hash = diagnostic_hash_step(hash, bitcast<u32>(origin.z));
  hash = diagnostic_hash_step(hash, extent_voxels.x | (extent_voxels.y << 16u));
  hash = diagnostic_hash_step(hash, material_face);
  hash = diagnostic_hash_step(hash, ao);
  return select(hash, 1u, hash == 0u);
}

fn diagnostic_hash_step(hash: u32, value: u32) -> u32 {
  return (hash ^ value) * 16777619u;
}

fn diagnostic_descriptor(material: u32, source: u32, face: u32) -> u32 {
  let streamed = (material & 0x80000000u) != 0u;
  let hierarchy_depth = select(0u, ((material >> 27u) & 7u) + 1u, streamed);
  let material_id = material & 0xffffu;
  // bits 0..3 representation/source, 4..7 hierarchy depth, 8..10 face, 11..26 material.
  // bits 27..31 remain available for attachment-level flags without revising the pixel format.
  return (source & 15u)
    | ((hierarchy_depth & 15u) << 4u)
    | ((face & 7u) << 8u)
    | ((material_id & 0xffffu) << 11u);
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

fn surface_morph_corner(morph_heights: vec4<i32>, corner: u32) -> f32 {
  return f32(morph_heights[corner]) * 0.5;
}

fn interpolated_quad_value(
  values: vec4<f32>,
  uv: vec2<f32>,
  flipped: bool,
) -> f32 {
  let point = clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0));
  if flipped {
    if point.x + point.y <= 1.0 {
      return values.x * (1.0 - point.x - point.y)
        + values.y * point.x
        + values.w * point.y;
    }
    return values.y * (1.0 - point.y)
      + values.z * (point.x + point.y - 1.0)
      + values.w * (1.0 - point.x);
  }
  if point.y <= point.x {
    return values.x * (1.0 - point.x)
      + values.y * (point.x - point.y)
      + values.z * point.y;
  }
  return values.x * (1.0 - point.y)
    + values.z * point.x
    + values.w * (point.y - point.x);
}

fn unpack_surface_macro_normal(packed: u32, parent: bool) -> vec3<f32> {
  let shift = select(vec2<u32>(0u, 5u), vec2<u32>(10u, 15u), parent);
  let x = f32((packed >> shift.x) & 31u) * (2.0 / 31.0) - 1.0;
  let z = f32((packed >> shift.y) & 31u) * (2.0 / 31.0) - 1.0;
  let y = sqrt(max(1.0 - x * x - z * z, 0.01));
  return normalize(vec3<f32>(x, y, z));
}

fn unpack_surface_horizon_profile(material: u32, ao: u32) -> u32 {
  let low = (material >> 19u) & 255u;
  let middle = ((material >> 30u) & 1u) << 8u;
  let high = ((ao >> 25u) & 127u) << 9u;
  return low | middle | high;
}

fn decoded_horizon_code(profile: u32, direction: u32, parent: bool) -> u32 {
  let parent_shift = select(0u, 8u, parent);
  return (profile >> (parent_shift + direction * 2u)) & 3u;
}

fn terrain_horizon_lighting(
  profile: u32,
  parent_blend: f32,
  terrain_normal: vec3<f32>,
  light_direction: vec3<f32>,
) -> vec2<f32> {
  let horizon_slopes = array<f32, 4>(0.0, 0.10510424, 0.2867454, 0.70020753);
  // A four-sector, two-bit profile is deliberately compact, but treating each representative
  // angle as an infinitesimal cardinal ray makes all unrepresented diagonal sky look open. These
  // values integrate a conservative angular interval around each code instead, restoring broad
  // valley/ridge definition without screen-space samples or high-frequency crevice darkening.
  let sector_accessibility = array<f32, 4>(1.0, 0.85, 0.60, 0.32);
  var slopes = array<f32, 4>();
  var sky_accessibility = vec2<f32>(0.0);
  for (var direction = 0u; direction < 4u; direction += 1u) {
    let own_code = decoded_horizon_code(profile, direction, false);
    let parent_code = decoded_horizon_code(profile, direction, true);
    slopes[direction] = mix(
      horizon_slopes[own_code],
      horizon_slopes[parent_code],
      parent_blend,
    );
    sky_accessibility += vec2<f32>(
      sector_accessibility[own_code],
      sector_accessibility[parent_code],
    );
  }
  sky_accessibility *= 0.25;
  let horizontal = abs(light_direction.xz);
  let x_horizon = select(slopes[1], slopes[0], light_direction.x >= 0.0);
  let z_horizon = select(slopes[2], slopes[3], light_direction.z >= 0.0);
  let horizon_slope = dot(vec2<f32>(x_horizon, z_horizon), horizontal)
    / max(horizontal.x + horizontal.y, 0.0001);
  let light_slope = max(light_direction.y, 0.0) / max(length(light_direction.xz), 0.0001);
  // tan(a +/- 4deg) is locally tan(a) +/- 0.07 * sec(a)^2. This slope-space form avoids
  // transcendental work per vertex while retaining the same broad angular penumbra.
  let horizon_softness = 0.07 * (1.0 + horizon_slope * horizon_slope);
  let key_visibility = smoothstep(
    horizon_slope - horizon_softness,
    horizon_slope + horizon_softness,
    light_slope,
  );
  let sky_visibility = mix(sky_accessibility.x, sky_accessibility.y, parent_blend);
  // Clear-sky radiance is anisotropic around the key light. Preserve full fill on flat and
  // sun-facing ground, but let broad away-facing slopes receive less of that directional lobe.
  // The light's horizontal magnitude naturally removes this cue when it is near the zenith.
  let directional_sky_visibility = clamp(
    1.0 + dot(terrain_normal.xz, light_direction.xz) * 1.1,
    0.75,
    1.0,
  );
  return vec2<f32>(
    key_visibility,
    mix(1.0, sky_visibility, 0.82) * directional_sky_visibility,
  );
}

fn lod_boundary_center(
  boundary: u32,
  boundary_centres: array<vec4<f32>, 4>,
) -> vec2<f32> {
  let packed = boundary_centres[boundary / 2u];
  return select(packed.xy, packed.zw, (boundary & 1u) != 0u);
}

fn lod_boundary_half_extent(
  boundary: u32,
  boundary_half_extents: array<vec4<f32>, 2>,
) -> f32 {
  return boundary_half_extents[boundary / 4u][boundary & 3u];
}

fn cut_transition_parent_blend(spatial_blend: f32, phase_role: vec2<f32>) -> f32 {
  let phase = clamp(phase_role.x, 0.0, 1.0);
  let role = u32(round(phase_role.y));
  if role == 1u {
    // A departing fine patch converges into the current coarse owner.
    return mix(spatial_blend, 1.0, phase);
  }
  if role == 2u {
    // An arriving fine patch unfolds from the previous coarse owner.
    return mix(1.0, spatial_blend, phase);
  }
  return spatial_blend;
}

fn surface_parent_normal_blend(
  world: vec3<f32>,
  material: u32,
  boundary_centres: array<vec4<f32>, 4>,
  boundary_half_extents: array<vec4<f32>, 2>,
  transition_phase_role: vec2<f32>,
) -> f32 {
  if frame.lod_options.w < 0.5 || (material & 0x80000000u) == 0u {
    return 0.0;
  }
  let level = (material >> 27u) & 7u;
  if level >= 7u {
    return 0.0;
  }
  let boundary = level + 1u;
  let half_extent = lod_boundary_half_extent(boundary, boundary_half_extents);
  // Ownership remains on the exact snapped lattice, but its geometry field follows the camera
  // continuously. Hysteresis can hold a cut 5/8 of one snap step behind the camera; finishing the
  // morph before that envelope guarantees every newly owned fine patch is already identical to
  // its parent when the atomic cut moves.
  let snap_step = 3.2 * exp2(f32(max(i32(boundary) - 1, 0)));
  let maximum_snap_lag = snap_step * 0.625;
  let delta = abs(world.xz - frame.camera_time.xz);
  let inside = half_extent - maximum_snap_lag - max(delta.x, delta.y);
  // At sprint speed the nearest 1.6m ramp lasts roughly 200ms.
  let width = max(1.6, half_extent * 0.02);
  return cut_transition_parent_blend(
    1.0 - smoothstep(0.0, width, inside),
    transition_phase_role,
  );
}

fn surface_wall_macro_blend(world: vec3<f32>) -> f32 {
  // The canonical square reaches 12.8m along its axes and 18.1m at its corners. Start close enough
  // that every first coarse wall still uses almost exactly its voxel-face normal, then converge
  // toward the bounded terrain slope over the next LOD rings. Camera distance keeps this lighting
  // invariant when the snapped ownership hierarchy moves around a stationary world point.
  let distance_from_near_field = max(distance(world.xz, frame.camera_time.xz) - 8.0, 0.0);
  return smoothstep(0.0, 48.0, distance_from_near_field) * 0.82;
}

fn quad_local(face: u32, uv: vec2<i32>, extent: vec2<i32>) -> vec3<i32> {
  switch face {
    case 0u: { return vec3<i32>(1, uv.y * extent.y, uv.x * extent.x); }
    case 1u: { return vec3<i32>(0, uv.y * extent.y, uv.x * extent.x); }
    case 2u: { return vec3<i32>(uv.x * extent.x, 1, uv.y * extent.y); }
    case 3u: { return vec3<i32>(uv.x * extent.x, 0, uv.y * extent.y); }
    case 4u: { return vec3<i32>(uv.x * extent.x, uv.y * extent.y, 1); }
    default: { return vec3<i32>(uv.x * extent.x, uv.y * extent.y, 0); }
  }
}

fn transition_triangle_local(
  corner: u32,
  encoded_extent: vec2<u32>,
  material: u32,
) -> vec3<i32> {
  let anchor = (encoded_extent.x >> 9u) & 3u;
  let edge = (encoded_extent.x >> 11u) & 3u;
  let level = (material >> 27u) & 7u;
  let stride = i32(2u << level);
  let anchor_xz = array<vec2<i32>, 4>(
    vec2<i32>(0, 0),
    vec2<i32>(stride, 0),
    vec2<i32>(stride, stride),
    vec2<i32>(0, stride),
  )[anchor];
  let raw_start = i32(encoded_extent.x & TRANSITION_TRIANGLE_OFFSET_MASK);
  let raw_end = i32(encoded_extent.y);
  // Negative-X and positive-Z run opposite the A-B-C-D polygon winding.
  let reverse = edge == 0u || edge == 3u;
  let tangent = select(
    select(raw_start, raw_end, corner == 2u),
    select(raw_end, raw_start, corner == 2u),
    reverse,
  );
  var boundary_xz = vec2<i32>(0);
  switch edge {
    case 0u: { boundary_xz = vec2<i32>(0, tangent); }
    case 1u: { boundary_xz = vec2<i32>(stride, tangent); }
    case 2u: { boundary_xz = vec2<i32>(tangent, 0); }
    default: { boundary_xz = vec2<i32>(tangent, stride); }
  }
  let xz = select(boundary_xz, anchor_xz, corner == 0u);
  return vec3<i32>(xz.x, 1, xz.y);
}

fn canonical_triangle_extent(encoded_extent: vec2<u32>) -> vec2<f32> {
  return vec2<f32>(
    f32((((encoded_extent.x >> 6u) & 31u) | ((encoded_extent.x >> 10u) & 32u)) + 1u),
    f32((((encoded_extent.y >> 6u) & 31u) | ((encoded_extent.y >> 10u) & 32u)) + 1u),
  );
}

fn canonical_triangle_uv(corner: u32, encoded_extent: vec2<u32>) -> vec2<f32> {
  let extent = canonical_triangle_extent(encoded_extent);
  let width = extent.x;
  let height = extent.y;
  let edge = (encoded_extent.x >> 11u) & 3u;
  let anchor_code = (encoded_extent.y >> 11u) & 7u;
  let raw_start = f32(encoded_extent.x & CANONICAL_TRIANGLE_OFFSET_MASK);
  let raw_end = f32(encoded_extent.y & CANONICAL_TRIANGLE_OFFSET_MASK);
  let reverse = edge == 0u || edge == 3u;
  let tangent = select(
    select(raw_start, raw_end, corner == 2u),
    select(raw_end, raw_start, corner == 2u),
    reverse,
  );
  var boundary_uv = vec2<f32>(0.0);
  switch edge {
    case 0u: { boundary_uv = vec2<f32>(0.0, tangent); }
    case 1u: { boundary_uv = vec2<f32>(width, tangent); }
    case 2u: { boundary_uv = vec2<f32>(tangent, 0.0); }
    default: { boundary_uv = vec2<f32>(tangent, height); }
  }
  let corner_anchors = array<vec2<f32>, 4>(
    vec2<f32>(0.0, 0.0),
    vec2<f32>(width, 0.0),
    vec2<f32>(width, height),
    vec2<f32>(0.0, height),
  );
  let anchor_uv = select(
    corner_anchors[min(max(anchor_code, 1u), 4u) - 1u],
    extent * 0.5,
    anchor_code == 0u,
  );
  return select(boundary_uv, anchor_uv, corner == 0u);
}

fn canonical_triangle_local(
  corner: u32,
  encoded_extent: vec2<u32>,
  face: u32,
) -> vec3<f32> {
  let uv = canonical_triangle_uv(corner, encoded_extent);
  switch face {
    case 0u: { return vec3<f32>(1.0, uv.y, uv.x); }
    case 1u: { return vec3<f32>(0.0, uv.y, uv.x); }
    case 2u: { return vec3<f32>(uv.x, 1.0, uv.y); }
    case 3u: { return vec3<f32>(uv.x, 0.0, uv.y); }
    case 4u: { return vec3<f32>(uv.x, uv.y, 1.0); }
    default: { return vec3<f32>(uv.x, uv.y, 0.0); }
  }
}

struct MorphedQuadPosition {
  world: vec3<f32>,
  parent_blend: f32,
};

fn quad_world(
  origin: vec3<i32>,
  face: u32,
  corner: u32,
  uv: vec2<i32>,
  extent: vec2<i32>,
  material: u32,
  ao: u32,
  surface_shape: u32,
  morph_heights: vec4<i32>,
  morph_closure: bool,
  morph_geometry: bool,
  transition_triangle: bool,
  canonical_triangle: bool,
  encoded_extent: vec2<u32>,
  boundary_centres: array<vec4<f32>, 4>,
  boundary_half_extents: array<vec4<f32>, 2>,
  transition_phase_role: vec2<f32>,
) -> MorphedQuadPosition {
  let local = select(
    quad_local(face, uv, extent),
    transition_triangle_local(corner, encoded_extent, material),
    transition_triangle,
  );
  var world = vec3<f32>(origin + local) * frame.viewport_voxel.z;
  if canonical_triangle {
    world = (vec3<f32>(origin) + canonical_triangle_local(corner, encoded_extent, face))
      * frame.viewport_voxel.z;
  }
  if surface_shape != 0u && !canonical_triangle {
    world.y += unpack_signed_i3(surface_shape >> (corner * 3u))
      * frame.viewport_voxel.z;
  }
  var parent_blend = 0.0;
  if morph_geometry && (ao & 0x01000000u) != 0u {
    parent_blend = surface_parent_normal_blend(
      world,
      material,
      boundary_centres,
      boundary_half_extents,
      transition_phase_role,
    );
    let morph_blend = select(parent_blend, 1.0 - parent_blend, morph_closure);
    var morph_delta = surface_morph_corner(morph_heights, corner);
    if canonical_triangle {
      let values = vec4<f32>(
        surface_morph_corner(morph_heights, 0u),
        surface_morph_corner(morph_heights, 1u),
        surface_morph_corner(morph_heights, 2u),
        surface_morph_corner(morph_heights, 3u),
      );
      let source_extent = canonical_triangle_extent(encoded_extent);
      morph_delta = interpolated_quad_value(
        values,
        canonical_triangle_uv(corner, encoded_extent) / source_extent,
        surface_quad_flip(face, surface_shape, ao),
      );
    }
    world.y += morph_delta
      * frame.viewport_voxel.z
      * morph_blend;
  }
  return MorphedQuadPosition(world, parent_blend);
}

fn voxel_vertex(
  vertex_index: u32,
  origin: vec3<i32>,
  extent_voxels: vec2<u32>,
  material_face: u32,
  ao: u32,
  encoded_owner_id: vec2<u32>,
  morph_heights: vec4<i32>,
  morph_geometry: bool,
  boundary_centres: array<vec4<f32>, 4>,
  boundary_half_extents: array<vec4<f32>, 2>,
  transition_phase_role: vec2<f32>,
) -> VertexOut {
  let face = (material_face >> 16u) & 7u;
  let encoded_source = (material_face >> GPU_SOURCE_SHIFT) & 7u;
  let packed_material = material_face & 0xfff8ff1fu;
  let surface_shape = ((packed_material >> 8u) & 255u) | (((ao >> 20u) & 15u) << 8u);
  let material = packed_material & 0xffff00ffu;
  let transition_triangle = (extent_voxels.x & TRANSITION_TRIANGLE_FLAG) != 0u;
  let canonical_triangle = (extent_voxels.x & CANONICAL_TRIANGLE_FLAG) != 0u;
  let morph_closure =
    (extent_voxels.x & MORPH_CLOSURE_EXTENT_FLAG) != 0u && !canonical_triangle;
  let custom_triangle = transition_triangle || canonical_triangle;
  let extent = vec2<i32>(vec2<u32>(
    select(
      extent_voxels.x & ~MORPH_CLOSURE_EXTENT_FLAG,
      0u,
      custom_triangle,
    ),
    extent_voxels.y,
  ));
  let flip = !custom_triangle && surface_quad_flip(face, surface_shape, ao);
  let quad_corner = select(STANDARD_STRIP[vertex_index], FLIPPED_STRIP[vertex_index], flip);
  let corner = select(quad_corner, TRIANGLE_STRIP[vertex_index], custom_triangle);
  let uv = CORNERS[corner];
  var normal = vec3<f32>(0.0);
  switch face {
    case 0u: { normal.x = 1.0; }
    case 1u: { normal.x = -1.0; }
    case 2u: { normal.y = 1.0; }
    case 3u: { normal.y = -1.0; }
    case 4u: { normal.z = 1.0; }
    default: { normal.z = -1.0; }
  }
  let morphed_position = quad_world(
    origin,
    face,
    corner,
    uv,
    extent,
    material,
    ao,
    surface_shape,
    morph_heights,
    morph_closure,
    morph_geometry,
    transition_triangle,
    canonical_triangle,
    extent_voxels,
    boundary_centres,
    boundary_half_extents,
    transition_phase_role,
  );
  let world = morphed_position.world;
  let surface_macro_normal = (ao & 0x01000000u) != 0u;
  var terrain_lighting = vec2<f32>(1.0);
  if surface_macro_normal {
    let own_normal = unpack_surface_macro_normal(ao, false);
    let parent_normal = unpack_surface_macro_normal(ao, true);
    let parent_blend = morphed_position.parent_blend;
    let terrain_normal = normalize(
      mix(own_normal, parent_normal, parent_blend),
    );
    normal = select(
      normalize(mix(normal, terrain_normal, surface_wall_macro_blend(world))),
      terrain_normal,
      face == 2u,
    );
    let resolved_horizon_lighting = terrain_horizon_lighting(
      unpack_surface_horizon_profile(material, ao),
      parent_blend,
      terrain_normal,
      normalize(frame.key_light_direction.xyz),
    );
    // Canonical 10cm chunks do not carry a streamed horizon profile. Fade the macro term in over
    // a broad world-distance band so their handoff to Stride2 cannot move a dark contour with the
    // ownership ring. Beyond 32m every coarse level receives the full landscape lighting cue.
    let horizon_strength = smoothstep(8.0, 32.0, distance(world.xz, frame.camera_time.xz));
    terrain_lighting = mix(vec2<f32>(1.0), resolved_horizon_lighting, horizon_strength);
  }
  var out: VertexOut;
  out.position = frame.view_projection * vec4<f32>(world, 1.0);
  out.world = world;
  out.normal = normal;
  out.material = material;
  out.ao = select(corner_ao(ao, corner), 1.0, surface_macro_normal);
  out.terrain_lighting = terrain_lighting;
  // Morph closures are ordinary streamed products in storage but a distinct draw source on the
  // active cut. Keep them visually separable without consuming another packed extent category.
  out.source = select(encoded_source, 5u, morph_closure);
  // Production entry points pass a literal zero owner. Keeping every diagnostic operation inside
  // this branch lets shader specialization eliminate it from ordinary frames; screenshot entry
  // points supply the transient sidecar's non-zero owner.
  if any(encoded_owner_id != vec2<u32>(0u)) {
    out.terrain_identity = vec4<u32>(
      encoded_owner_id.x,
      encoded_owner_id.y,
      diagnostic_primitive_id(origin, extent_voxels, material_face, ao),
      diagnostic_descriptor(material, out.source, face),
    );
  } else {
    out.terrain_identity = vec4<u32>(0u);
  }
  // The finest cloud octave changes over hundreds of metres, so evaluating it at every covered
  // 10 cm terrain fragment only repeats the same signal. Interpolating the exact shared field
  // from surface vertices retains substantially more spatial resolution than the field contains.
  out.surface_weather = cloud_surface_weather(world);
  return out;
}

@vertex
fn vs_main_fixed(
  @builtin(vertex_index) vertex_index: u32,
  @location(0) origin: vec3<i32>,
  @location(1) extent_voxels: vec2<u32>,
  @location(2) material_face: u32,
  @location(3) ao: u32,
) -> VertexOut {
  return voxel_vertex(
    vertex_index,
    origin,
    extent_voxels,
    material_face,
    ao,
    vec2<u32>(0u),
    vec4<i32>(0),
    false,
    frame.lod_boundary_centres,
    frame.lod_boundary_half_extents,
    vec2<f32>(0.0),
  );
}

fn virtual_cluster_surface_weather(world: vec3<f32>) -> vec2<f32> {
  return cloud_surface_weather(world);
}

fn virtual_cluster_vertex(
  position_voxels: vec3<f32>,
  material: u32,
  packed_normal: vec4<f32>,
  encoded_owner_id: vec2<u32>,
) -> VertexOut {
  let world = position_voxels * frame.viewport_voxel.z;
  var out: VertexOut;
  out.position = frame.view_projection * vec4<f32>(world, 1.0);
  out.world = world;
  out.normal = normalize(packed_normal.xyz);
  out.material = material;
  out.ao = 1.0;
  out.terrain_lighting = vec2<f32>(1.0);
  out.source = 8u;
  out.terrain_identity = vec4<u32>(0u);
  if any(encoded_owner_id != vec2<u32>(0u)) {
    var primitive = 2166136261u;
    primitive = diagnostic_hash_step(primitive, bitcast<u32>(position_voxels.x));
    primitive = diagnostic_hash_step(primitive, bitcast<u32>(position_voxels.y));
    primitive = diagnostic_hash_step(primitive, bitcast<u32>(position_voxels.z));
    primitive = diagnostic_hash_step(primitive, material);
    out.terrain_identity = vec4<u32>(
      encoded_owner_id.x,
      encoded_owner_id.y,
      select(primitive, 1u, primitive == 0u),
      diagnostic_descriptor(material, 8u, 7u),
    );
  }
  out.surface_weather = virtual_cluster_surface_weather(world);
  return out;
}

@vertex
fn vs_virtual_cluster(
  @location(0) position_voxels: vec3<f32>,
  @location(1) material: u32,
  @location(2) packed_normal: vec4<f32>,
) -> VertexOut {
  return virtual_cluster_vertex(
    position_voxels,
    material,
    packed_normal,
    vec2<u32>(0u),
  );
}

@vertex
fn vs_virtual_cluster_diagnostic(
  @location(0) position_voxels: vec3<f32>,
  @location(1) material: u32,
  @location(2) packed_normal: vec4<f32>,
  @location(3) diagnostic_owner: vec2<u32>,
) -> VertexOut {
  return virtual_cluster_vertex(
    position_voxels,
    material,
    packed_normal,
    diagnostic_owner,
  );
}

@vertex
fn vs_main_fixed_diagnostic(
  @builtin(vertex_index) vertex_index: u32,
  @location(0) origin: vec3<i32>,
  @location(1) extent_voxels: vec2<u32>,
  @location(2) material_face: u32,
  @location(3) ao: u32,
  @location(4) diagnostic_owner: vec2<u32>,
) -> VertexOut {
  return voxel_vertex(
    vertex_index,
    origin,
    extent_voxels,
    material_face,
    ao,
    diagnostic_owner,
    vec4<i32>(0),
    false,
    frame.lod_boundary_centres,
    frame.lod_boundary_half_extents,
    vec2<f32>(0.0),
  );
}

@vertex
fn vs_main_morph(
  @builtin(vertex_index) vertex_index: u32,
  @location(0) origin: vec3<i32>,
  @location(1) extent_voxels: vec2<u32>,
  @location(2) material_face: u32,
  @location(3) ao: u32,
  @location(4) morph_heights: vec4<i32>,
) -> VertexOut {
  return voxel_vertex(
    vertex_index,
    origin,
    extent_voxels,
    material_face,
    ao,
    vec2<u32>(0u),
    morph_heights,
    true,
    frame.lod_boundary_centres,
    frame.lod_boundary_half_extents,
    vec2<f32>(0.0),
  );
}

@vertex
fn vs_main_morph_diagnostic(
  @builtin(vertex_index) vertex_index: u32,
  @location(0) origin: vec3<i32>,
  @location(1) extent_voxels: vec2<u32>,
  @location(2) material_face: u32,
  @location(3) ao: u32,
  @location(4) diagnostic_owner: vec2<u32>,
  @location(5) morph_heights: vec4<i32>,
) -> VertexOut {
  return voxel_vertex(
    vertex_index,
    origin,
    extent_voxels,
    material_face,
    ao,
    diagnostic_owner,
    morph_heights,
    true,
    frame.lod_boundary_centres,
    frame.lod_boundary_half_extents,
    vec2<f32>(0.0),
  );
}

fn transition_boundary_centres() -> array<vec4<f32>, 4> {
  if cut_transition.phase_role.y == 1.0 {
    return cut_transition.lod_boundary_centres;
  }
  return frame.lod_boundary_centres;
}

fn transition_boundary_half_extents() -> array<vec4<f32>, 2> {
  if cut_transition.phase_role.y == 1.0 {
    return cut_transition.lod_boundary_half_extents;
  }
  return frame.lod_boundary_half_extents;
}

@vertex
fn vs_transition_fixed(
  @builtin(vertex_index) vertex_index: u32,
  @location(0) origin: vec3<i32>,
  @location(1) extent_voxels: vec2<u32>,
  @location(2) material_face: u32,
  @location(3) ao: u32,
) -> VertexOut {
  return voxel_vertex(
    vertex_index,
    origin,
    extent_voxels,
    material_face,
    ao,
    vec2<u32>(0u),
    vec4<i32>(0),
    false,
    transition_boundary_centres(),
    transition_boundary_half_extents(),
    cut_transition.phase_role.xy,
  );
}

@vertex
fn vs_transition_fixed_diagnostic(
  @builtin(vertex_index) vertex_index: u32,
  @location(0) origin: vec3<i32>,
  @location(1) extent_voxels: vec2<u32>,
  @location(2) material_face: u32,
  @location(3) ao: u32,
  @location(4) diagnostic_owner: vec2<u32>,
) -> VertexOut {
  return voxel_vertex(
    vertex_index,
    origin,
    extent_voxels,
    material_face,
    ao,
    diagnostic_owner,
    vec4<i32>(0),
    false,
    transition_boundary_centres(),
    transition_boundary_half_extents(),
    cut_transition.phase_role.xy,
  );
}

@vertex
fn vs_transition_morph(
  @builtin(vertex_index) vertex_index: u32,
  @location(0) origin: vec3<i32>,
  @location(1) extent_voxels: vec2<u32>,
  @location(2) material_face: u32,
  @location(3) ao: u32,
  @location(4) morph_heights: vec4<i32>,
) -> VertexOut {
  return voxel_vertex(
    vertex_index,
    origin,
    extent_voxels,
    material_face,
    ao,
    vec2<u32>(0u),
    morph_heights,
    true,
    transition_boundary_centres(),
    transition_boundary_half_extents(),
    cut_transition.phase_role.xy,
  );
}

@vertex
fn vs_transition_morph_diagnostic(
  @builtin(vertex_index) vertex_index: u32,
  @location(0) origin: vec3<i32>,
  @location(1) extent_voxels: vec2<u32>,
  @location(2) material_face: u32,
  @location(3) ao: u32,
  @location(4) diagnostic_owner: vec2<u32>,
  @location(5) morph_heights: vec4<i32>,
) -> VertexOut {
  return voxel_vertex(
    vertex_index,
    origin,
    extent_voxels,
    material_face,
    ao,
    diagnostic_owner,
    morph_heights,
    true,
    transition_boundary_centres(),
    transition_boundary_half_extents(),
    cut_transition.phase_role.xy,
  );
}

fn srgb_to_linear(srgb: vec3<f32>) -> vec3<f32> {
  let low = srgb / 12.92;
  let high = pow((srgb + 0.055) / 1.055, vec3<f32>(2.4));
  return select(high, low, srgb <= vec3<f32>(0.04045));
}

struct SurfaceBasis {
  uv: vec2<f32>,
  tangent: vec3<f32>,
  bitangent: vec3<f32>,
};

struct SurfaceDetail {
  albedo: vec3<f32>,
  normal: vec3<f32>,
  roughness: f32,
};

fn surface_uv(world: vec3<f32>, normal: vec3<f32>) -> vec2<f32> {
  let dominant_axis = abs(normal);
  if dominant_axis.x >= dominant_axis.y && dominant_axis.x >= dominant_axis.z {
    return select(vec2<f32>(world.y, -world.z), world.yz, normal.x >= 0.0);
  }
  if dominant_axis.y >= dominant_axis.z {
    return select(world.xz, vec2<f32>(world.x, -world.z), normal.y >= 0.0);
  }
  return select(vec2<f32>(-world.x, world.y), world.xy, normal.z >= 0.0);
}

fn surface_basis(world: vec3<f32>, normal: vec3<f32>) -> SurfaceBasis {
  let n = normalize(normal);
  let dominant_axis = abs(n);
  var basis: SurfaceBasis;
  basis.uv = surface_uv(world, n);
  var tangent_seed = vec3<f32>(1.0, 0.0, 0.0);
  if dominant_axis.x >= dominant_axis.y && dominant_axis.x >= dominant_axis.z {
    tangent_seed = vec3<f32>(0.0, 1.0, 0.0);
  } else if dominant_axis.y < dominant_axis.z {
    tangent_seed = select(
      vec3<f32>(-1.0, 0.0, 0.0),
      vec3<f32>(1.0, 0.0, 0.0),
      n.z >= 0.0,
    );
  }
  // Smoothed distant-terrain normals are not axis aligned. Reproject the chosen world-aligned
  // texture axis so tangent-space normal detail cannot skew or amplify lighting across LOD slopes.
  basis.tangent = normalize(tangent_seed - n * dot(tangent_seed, n));
  basis.bitangent = normalize(cross(n, basis.tangent));
  return basis;
}

fn material_detail_scale(material: u32) -> f32 {
  switch material {
    case 4u, 12u: { return 0.38; }
    case 8u: { return 0.72; }
    case 9u, 10u: { return 0.82; }
    default: { return 0.55; }
  }
}

const MATERIAL_TEXELS_PER_VOXEL: f32 = 3.0;

fn pixelated_material_uv(surface_metres: vec2<f32>, material_scale: f32) -> vec2<f32> {
  // Quantize in canonical world space before applying the material's atlas frequency. Greedy
  // quads can span many voxels, so this preserves exactly 3x3 visible blocks on every 10 cm face
  // without introducing per-face vertices or abandoning world-aligned material continuity.
  let texels_per_metre = MATERIAL_TEXELS_PER_VOXEL / frame.viewport_voxel.z;
  // Keep mathematically exact voxel boundaries stable when f32 interpolation lands one ULP low.
  let world_texel = floor(surface_metres * texels_per_metre + vec2<f32>(0.0001));
  return ((world_texel + vec2<f32>(0.5)) / texels_per_metre) * material_scale;
}

fn sample_surface_detail(
  world: vec3<f32>,
  geometric_normal: vec3<f32>,
  material: u32,
  basis: SurfaceBasis,
  uv_dx: vec2<f32>,
  uv_dy: vec2<f32>,
  detail_distance: f32,
) -> SurfaceDetail {
  var detail: SurfaceDetail;
  detail.normal = geometric_normal;
  let material_scale = material_detail_scale(material);
  let uv = pixelated_material_uv(basis.uv, material_scale);
  // Past this point even one screen pixel covers many authored material texels at 720p. Sampling
  // two anisotropic atlas layers and rebuilding a tangent frame cannot add visible information;
  // the atlas' terminal mip is the same prefiltered material, without paying that per-fragment
  // cost across kilometre-scale terrain.
  if MATERIAL_DETAIL != 0u && detail_distance < 144.0 {
    // Derive mip selection from the continuous coordinates. Derivatives of the quantized UV are
    // zero inside a block and discontinuous at its edge, which would otherwise force unstable LOD.
    detail.albedo = textureSampleGrad(
      material_albedo,
      material_sampler,
      uv,
      i32(material),
      uv_dx,
      uv_dy,
    ).rgb;
    let packed_surface = textureSampleGrad(
      material_surface,
      material_sampler,
      uv,
      i32(material),
      uv_dx,
      uv_dy,
    );
    let averaged_normal = packed_surface.rgb * 2.0 - vec3<f32>(1.0);
    let normal_length = clamp(length(averaged_normal), 0.001, 1.0);
    let tangent_normal = averaged_normal / normal_length;
    let distance_fade = 1.0 - smoothstep(42.0, 120.0, distance(world, frame.camera_time.xyz));
    let faded_normal = normalize(vec3<f32>(
      tangent_normal.xy * distance_fade,
      max(tangent_normal.z, 0.08),
    ));
    detail.normal = normalize(
      basis.tangent * faded_normal.x
        + basis.bitangent * faded_normal.y
        + geometric_normal * faded_normal.z,
    );
    let normal_variance = 1.0 - normal_length;
    detail.roughness = sqrt(clamp(
      packed_surface.a * packed_surface.a + normal_variance * 0.72,
      0.01,
      1.0,
    ));
  } else {
    let base_mip = i32(textureNumLevels(material_albedo) - 1u);
    // The atlas remains the sole material definition for flat debug mode and distant terrain.
    // Preserve normal-map variance in the terminal mip so the far roughness still matches the
    // fully sampled PBR material even though sub-pixel tangent normals are intentionally omitted.
    detail.albedo = textureLoad(material_albedo, vec2<i32>(0), i32(material), base_mip).rgb;
    let packed_surface = textureLoad(
      material_surface,
      vec2<i32>(0),
      i32(material),
      base_mip,
    );
    let averaged_normal = packed_surface.rgb * 2.0 - vec3<f32>(1.0);
    let normal_variance = 1.0 - clamp(length(averaged_normal), 0.001, 1.0);
    detail.roughness = sqrt(clamp(
      packed_surface.a * packed_surface.a + normal_variance * 0.72,
      0.01,
      1.0,
    ));
  }
  return detail;
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

fn streamed_lod_debug_color(level: u32) -> vec3<f32> {
  let colors = array<vec3<f32>, 8>(
    vec3<f32>(0.12, 1.00, 0.18),
    vec3<f32>(0.62, 1.00, 0.06),
    vec3<f32>(1.00, 0.86, 0.04),
    vec3<f32>(1.00, 0.43, 0.03),
    vec3<f32>(1.00, 0.07, 0.04),
    vec3<f32>(1.00, 0.04, 0.62),
    vec3<f32>(0.62, 0.08, 1.00),
    vec3<f32>(0.10, 0.32, 1.00),
  );
  return colors[min(level, 7u)];
}

fn geometry_source_debug_color(input: VertexOut) -> vec3<f32> {
  let streamed = (input.material & 0x80000000u) != 0u;
  let level = (input.material >> 27u) & 7u;
  var color = select(
    vec3<f32>(0.02, 0.82, 1.00),
    streamed_lod_debug_color(level),
    streamed,
  );
  switch input.source {
    // Temporary opaque exact-volume frontier cap.
    case 1u: { color = vec3<f32>(1.00, 0.02, 0.02); }
    // Height-matched connector generated between two ownership sources.
    case 2u: { color = vec3<f32>(1.00); }
    // Streamed synthetic fallback wall standing in for unavailable exact vertical data.
    case 3u: { color = vec3<f32>(1.00, 0.72, 0.02); }
    // Streamed water keeps the surface level's hue but shifts toward blue.
    case 4u: { color = mix(streamed_lod_debug_color(level), vec3<f32>(0.02, 0.35, 1.00), 0.48); }
    // Streamed skyline proxy (and the legacy morph-closure stream, when present). A neutral
    // checker stays categorical beside every saturated terrain-LOD hue.
    case 5u: {
      let closure_checker = (u32(input.position.x) ^ u32(input.position.y)) & 8u;
      color = select(vec3<f32>(0.08), vec3<f32>(0.92), closure_checker != 0u);
    }
    // Coarse top cells subdivided to the adjacent fine lattice at an exact LOD cut.
    case 6u: { color = vec3<f32>(1.00, 0.08, 0.02); }
    // Connector whose fine and coarse edge curves cross within one segment.
    case 7u: { color = vec3<f32>(0.72, 0.04, 1.00); }
    default: {}
  }
  if CUT_TRANSITION != 0u {
    let checker = (u32(input.position.x) + u32(input.position.y)) & 4u;
    let transition_color = select(
      vec3<f32>(1.00, 0.02, 0.02),
      vec3<f32>(0.02, 1.00, 0.88),
      cut_transition.phase_role.y == 2.0,
    );
    color = select(transition_color, vec3<f32>(1.00), checker != 0u);
  }

  // Draw the source's actual sampling lattice in world space. Canonical cyan therefore carries a
  // 10 cm grid, while every streamed hue exposes its own 20 cm, 40 cm, ... cell alignment.
  let stride_voxels = select(1u, 1u << (level + 1u), streamed || input.source >= 2u);
  let grid = surface_uv(input.world, input.normal)
    / (frame.viewport_voxel.z * f32(stride_voxels));
  let fraction = fract(grid);
  let edge = min(min(fraction.x, 1.0 - fraction.x), min(fraction.y, 1.0 - fraction.y));
  let footprint = max(max(fwidth(grid).x, fwidth(grid).y), 0.0005);
  let resolved = 1.0 - smoothstep(0.28, 0.55, footprint);
  let line = (1.0 - smoothstep(0.0, footprint * 0.75, edge)) * resolved;
  return mix(color, color * 0.12, line * 0.72);
}

fn hash31(position: vec3<f32>) -> f32 {
  let value = dot(position, vec3<f32>(127.1, 311.7, 74.7));
  return fract(sin(value) * 43758.5453);
}

@fragment
fn fs_depth_transition() {}

fn cloud_surface_weather(world: vec3<f32>) -> vec2<f32> {
  let coverage_control = clamp(frame.fog_exposure.z, 0.0, 1.0);
  if coverage_control < 0.08 {
    return vec2<f32>(1.0, 0.0);
  }
  let sun = normalize(frame.key_light_direction.xyz);
  let cloud_height = mix(frame.cloud_layer.x, frame.cloud_layer.y, 0.46);
  let distance_to_layer = max(cloud_height - world.y, 0.0) / max(sun.y, 0.12);
  let cloud_world = world.xz + sun.xz * distance_to_layer;
  let field = atmosphere_cloud_field_world(
    cloud_world,
    frame.environment_time.yz,
    frame.environment_time.w,
  );
  let cloud = atmosphere_cloud_envelope(field, coverage_control);
  let sun_visibility = mix(
    1.0,
    mix(0.62, 0.40, frame.weather.y),
    cloud * coverage_control,
  );
  let local_precipitation = liquid_precipitation() * smoothstep(0.08, 0.42, cloud);
  return vec2<f32>(sun_visibility, local_precipitation);
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
  let view_depth = distance(world, frame.camera_time.xyz);
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

fn scene_sample(uv: vec2<f32>) -> vec4<f32> {
  let dimensions = textureDimensions(opaque_depth);
  let pixel = clamp(
    vec2<i32>(uv * vec2<f32>(dimensions)),
    vec2<i32>(0),
    vec2<i32>(dimensions) - vec2<i32>(1),
  );
  let depth = textureLoad(opaque_depth, pixel, 0);
  if depth <= 0.000001 {
    return vec4<f32>(0.0);
  }
  let ndc = vec2<f32>(uv.x * 2.0 - 1.0, (1.0 - uv.y) * 2.0 - 1.0);
  let homogeneous = frame.inverse_view_projection * vec4<f32>(ndc, depth, 1.0);
  let world = homogeneous.xyz
    / max(abs(homogeneous.w), 0.000001) * sign(homogeneous.w);
  return vec4<f32>(world, depth);
}

fn filtered_wave_slope(
  position: vec2<f32>,
  direction: vec2<f32>,
  wavelength: f32,
  slope_amplitude: f32,
  speed: f32,
  phase_offset: f32,
  pixel_footprint: f32,
) -> vec2<f32> {
  // A projected wavelength needs several pixels to remain a stable normal cue. Smoothly removing
  // unresolved bands prevents distant water from sparkling or collapsing into a repetitive moire.
  let resolved = 1.0 - smoothstep(wavelength * 0.12, wavelength * 0.32, pixel_footprint);
  let wave_number = 6.28318530718 / wavelength;
  let phase = dot(position, direction) * wave_number
    + frame.atmosphere_motion.x * speed + phase_offset;
  return direction * cos(phase) * slope_amplitude * resolved;
}

fn water_wave_normal(world: vec3<f32>, water_depth: f32) -> vec3<f32> {
  let footprint = max(length(fwidth(world.xz)), 0.001);
  let position = world.xz;
  var slope = filtered_wave_slope(
    position, normalize(vec2<f32>(0.91, 0.42)), 17.3, 0.080, 0.39, 0.7, footprint,
  );
  slope += filtered_wave_slope(
    position, normalize(vec2<f32>(-0.34, 0.94)), 9.1, 0.061, -0.57, 2.1, footprint,
  );
  slope += filtered_wave_slope(
    position, normalize(vec2<f32>(0.68, -0.73)), 4.7, 0.043, 0.83, 4.4, footprint,
  );
  slope += filtered_wave_slope(
    position, normalize(vec2<f32>(-0.82, -0.57)), 2.6, 0.028, -1.14, 1.3, footprint,
  );
  slope += filtered_wave_slope(
    position, normalize(vec2<f32>(0.23, 0.97)), 1.37, 0.017, 1.61, 5.2, footprint,
  );
  slope += filtered_wave_slope(
    position, normalize(vec2<f32>(0.99, -0.12)), 0.73, 0.009, -2.05, 3.5, footprint,
  );
  // Shallow water transfers less energy into free-surface waves and converges continuously on the
  // bank instead of intersecting it.
  slope *= smoothstep(0.08, 1.15, water_depth);
  return normalize(vec3<f32>(-slope.x, 1.0, -slope.y));
}

fn environment_radiance(direction: vec3<f32>) -> vec3<f32> {
  let sky_height = pow(clamp(direction.y * 0.5 + 0.5, 0.0, 1.0), 0.58);
  var radiance = mix(frame.ground_atmosphere.rgb, frame.sky_zenith.rgb, sky_height);
  radiance = mix(radiance, frame.sky_horizon.rgb, exp(-abs(direction.y) * 5.5) * 0.46);
  return radiance;
}

fn reflected_environment(direction: vec3<f32>) -> vec3<f32> {
  return environment_radiance(direction);
}

@fragment
fn fs_water(input: VertexOut) -> @location(0) vec4<f32> {
  let material = input.material & 0xffffu;
  if material != 13u {
    discard;
  }
  if frame.lod_options.x > 0.5 {
    return vec4<f32>(geometry_source_debug_color(input), 1.0);
  }
  let base_uv = input.position.xy / frame.viewport_voxel.xy;
  let base_background = scene_sample(base_uv);
  let top_water_depth = select(
    12.0,
    max(input.world.y - base_background.y, 0.0),
    base_background.w > 0.000001,
  );
  let view_direction = normalize(frame.camera_time.xyz - input.world);
  var normal = select(
    input.normal,
    water_wave_normal(input.world, top_water_depth),
    input.normal.y > 0.5,
  );
  if dot(normal, view_direction) < 0.0 {
    normal = -normal;
  }
  let facing = clamp(dot(normal, view_direction), 0.0, 1.0);
  var fresnel = fresnel_schlick(facing, vec3<f32>(0.02037));
  let reflection = reflected_environment(reflect(-view_direction, normal));
  let camera_to_surface = input.world - frame.camera_time.xyz;
  let distance_to_camera = length(camera_to_surface);
  let below_surface = frame.medium.y > 0.0;
  let refraction_ratio = select(1.0 / 1.333, 1.333, below_surface);
  var transmitted_ray = refract(-view_direction, normal, refraction_ratio);
  if dot(transmitted_ray, transmitted_ray) < 0.000001 {
    transmitted_ray = reflect(-view_direction, normal);
    fresnel = vec3<f32>(1.0);
  }
  let sample_world = input.world + transmitted_ray * mix(0.28, 1.25, smoothstep(0.2, 6.0, top_water_depth));
  let sample_clip = frame.view_projection * vec4<f32>(sample_world, 1.0);
  let projected_uv = sample_clip.xy / max(sample_clip.w, 0.0001)
    * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
  let max_refraction_offset = vec2<f32>(9.0) / frame.viewport_voxel.xy;
  let projected_offset = clamp(
    projected_uv - base_uv,
    -max_refraction_offset,
    max_refraction_offset,
  );
  var refraction_uv = clamp(
    base_uv + select(vec2<f32>(0.0), projected_offset, sample_clip.w > 0.0),
    vec2<f32>(0.001),
    vec2<f32>(0.999),
  );
  var refracted_background = scene_sample(refraction_uv);
  // A screen-space offset must never pull foreground geometry through the water. Falling back to
  // the unperturbed ray is stable and preserves the true shoreline silhouette.
  if refracted_background.w > input.position.z + 0.000001 {
    refraction_uv = base_uv;
    refracted_background = base_background;
  }
  let refracted_scene = max(
    textureSampleLevel(opaque_scene, opaque_scene_sampler, refraction_uv, 0.0).rgb,
    vec3<f32>(0.0),
  );
  let optical_path = select(
    28.0,
    clamp(distance(refracted_background.xyz, input.world), 0.0, 80.0),
    refracted_background.w > 0.000001,
  );
  // Single-layer participating medium: Beer-Lambert extinction preserves shallow bottom color,
  // while out-scattering supplies the familiar blue-green body color only as path length grows.
  let absorption_coefficient = vec3<f32>(0.31, 0.075, 0.028);
  let scattering_coefficient = vec3<f32>(0.012, 0.038, 0.052);
  let extinction = absorption_coefficient + scattering_coefficient;
  let water_transmittance = exp(-extinction * optical_path);
  let water_radiance = srgb_to_linear(vec3<f32>(0.055, 0.29, 0.34))
    * (0.34 + max(normalize(frame.key_light_direction.xyz).y, 0.0) * 0.36);
  let in_scattering = water_radiance
    * (scattering_coefficient / extinction)
    * (vec3<f32>(1.0) - water_transmittance);
  let transmitted = refracted_scene * water_transmittance + in_scattering;

  var surface_radiance = reflection * fresnel;
  let sun = normalize(frame.key_light_direction.xyz);
  let visibility = sun_visibility(input.world, normal) * input.surface_weather.x;
  surface_radiance += frame.key_light_radiance.rgb
    * evaluate_direct_dielectric_f0(
      vec3<f32>(0.0),
      0.075,
      vec3<f32>(0.02037),
      normal,
      view_direction,
      sun,
    )
    * visibility;
  var color = surface_radiance + transmitted * (vec3<f32>(1.0) - fresnel);

  let fog_view_direction = camera_to_surface / max(distance_to_camera, 0.0001);
  let average_height = max((input.world.y + frame.camera_time.y) * 0.5, 0.0);
  let height_density = exp(-average_height * frame.fog_exposure.x);
  let optical_depth = distance_to_camera
    * frame.ground_atmosphere.w * height_density * frame.render_options.y;
  let transmittance = exp(-optical_depth);
  let sky_factor = pow(max(fog_view_direction.y, 0.0), 0.42);
  let fog_radiance = mix(frame.sky_horizon.rgb, frame.sky_zenith.rgb, sky_factor);
  color = color * transmittance + fog_radiance * (1.0 - transmittance);
  color = max(color * frame.fog_exposure.y * frame.interior.y, vec3<f32>(0.0));
  return vec4<f32>(color, 1.0);
}

fn screen_space_ambient_visibility(pixel_position: vec2<f32>, world: vec3<f32>) -> f32 {
  if frame.camera_forward.w < 0.5 {
    return 1.0;
  }
  let dimensions = textureDimensions(filtered_spatial_ao);
  let half_position = (pixel_position - vec2<f32>(1.5)) * 0.5;
  let base = vec2<i32>(floor(half_position));
  let fraction = fract(half_position);
  let center_view_depth = dot(world - frame.camera_time.xyz, frame.camera_forward.xyz);
  var weighted_visibility = 0.0;
  var total_weight = 0.0;
  for (var y = 0; y <= 1; y += 1) {
    for (var x = 0; x <= 1; x += 1) {
      let coordinate = clamp(
        base + vec2<i32>(x, y),
        vec2<i32>(0),
        vec2<i32>(dimensions) - vec2<i32>(1),
      );
      let sample_value = textureLoad(filtered_spatial_ao, coordinate, 0).rg;
      let bilinear = (1.0 - abs(f32(x) - fraction.x))
        * (1.0 - abs(f32(y) - fraction.y));
      let relative_depth_delta = abs(sample_value.y - center_view_depth)
        / max(center_view_depth, 0.01);
      let depth_weight = select(exp(-relative_depth_delta * 220.0), 0.0, sample_value.y <= 0.0);
      let weight = bilinear * depth_weight;
      weighted_visibility += sample_value.x * weight;
      total_weight += weight;
    }
  }
  return clamp(select(1.0, weighted_visibility / total_weight, total_weight > 0.0001), 0.30, 1.0);
}

fn distant_surface_radiance(
  input: VertexOut,
  material: u32,
  sun: vec3<f32>,
) -> vec3<f32> {
  // Quad face normals and vertex-produced surface macro normals are already normalized.
  let normal = input.normal;
  let base_mip = i32(textureNumLevels(material_albedo) - 1u);
  let dry_albedo = textureLoad(
    material_albedo,
    vec2<i32>(0),
    i32(material),
    base_mip,
  ).rgb;
  // The distant path deliberately avoids resampling the multi-octave rain footprint for every
  // sub-pixel fragment. Scene-scale dampness changes diffuse reflectance continuously while the
  // exact active-cloud field governs visible drops and nearby surface sheen.
  let retained_wetness = liquid_precipitation()
    * frame.fog_exposure.z
    * smoothstep(-0.15, 0.65, normal.y)
    * (1.0 - frame.interior.x)
    * select(1.0, 0.0, material == 13u || material == 14u);
  let albedo = dry_albedo * mix(1.0, 0.64, retained_wetness);
  let sky_visibility = normal.y * 0.5 + 0.5;
  let interior_ambient = mix(1.0, 0.05, frame.interior.x);
  let sky_irradiance = mix(
    frame.ground_atmosphere.rgb,
    frame.sky_horizon.rgb * 0.48,
    sky_visibility,
  ) * interior_ambient;
  let ambient = albedo * sky_irradiance * input.terrain_lighting.y * 0.96;
  // Cloud cover already modulates the synchronized key radiance. Evaluating the multi-octave
  // local cloud field per sub-pixel terrain fragment is both unstable and visually redundant.
  let key_visibility = input.terrain_lighting.x;
  let diffuse = albedo
    * frame.key_light_radiance.rgb
    * max(dot(normal, sun), 0.0)
    * key_visibility
    * frame.key_light_direction.w
    * 0.197;
  // Full microfacet, local-light, grain, six-metre macro tint, cloud-shadow, and SSAO evaluation
  // cannot contribute stable information at this distance. Sampling their prefiltered material
  // color and broad sky fill avoids turning unresolved surface variation into temporal shimmer.
  return ambient + diffuse;
}

fn transport_surface_radiance(color: vec3<f32>, world: vec3<f32>, sun: vec3<f32>) -> vec3<f32> {
  let camera_to_surface = world - frame.camera_time.xyz;
  let distance_to_camera = length(camera_to_surface);
  let fog_view_direction = camera_to_surface / max(distance_to_camera, 0.0001);
  let average_height = max((world.y + frame.camera_time.y) * 0.5, 0.0);
  let height_density = exp(-average_height * frame.fog_exposure.x);
  let optical_depth = distance_to_camera
    * frame.ground_atmosphere.w * height_density * frame.render_options.y;
  let transmittance = exp(-optical_depth);
  let sky_factor = pow(max(fog_view_direction.y, 0.0), 0.42);
  var fog_radiance = mix(frame.sky_horizon.rgb, frame.sky_zenith.rgb, sky_factor);
  let sun_amount = max(dot(fog_view_direction, sun), 0.0);
  fog_radiance += frame.key_light_radiance.rgb * pow(sun_amount, 32.0) * 0.012;
  var transported = color * transmittance + fog_radiance * (1.0 - transmittance);
  let cave_transmittance = exp(-distance_to_camera * frame.interior.z);
  let cave_air = vec3<f32>(0.010, 0.014, 0.020);
  transported = mix(cave_air, transported, cave_transmittance);
  var underwater_path = 0.0;
  if frame.medium.y > 0.0 {
    let endpoint_depth = frame.medium.w - world.y;
    if endpoint_depth >= 0.0 {
      underwater_path = distance_to_camera;
    } else {
      // The camera-to-fragment ray exits the horizontal free surface. Only attenuate the segment
      // below that plane, so above-water geometry remains clear during partial submersion.
      underwater_path = distance_to_camera
        * frame.medium.y / max(frame.medium.y - endpoint_depth, 0.0001);
    }
  }
  underwater_path *= frame.medium.x;
  let water_transmittance = exp(-vec3<f32>(0.36, 0.14, 0.07) * underwater_path);
  let water_scattering = srgb_to_linear(vec3<f32>(0.018, 0.20, 0.27));
  transported = transported * water_transmittance
    + water_scattering * (vec3<f32>(1.0) - water_transmittance);
  return max(transported * frame.fog_exposure.y * frame.interior.y, vec3<f32>(0.0));
}

@fragment
fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
  let material = input.material & 0xffffu;
  // EnvironmentState normalizes this direction before it enters the frame uniform.
  let sun = frame.key_light_direction.xyz;
  let distance_to_camera = distance(input.world, frame.camera_time.xyz);
  // Derivatives must be evaluated in uniform control flow. Supplying the explicit gradients to
  // the near-field sampler lets distant fragments skip the tangent basis and texture lookups.
  let material_scale = material_detail_scale(material);
  let continuous_uv = surface_uv(input.world, input.normal) * material_scale;
  let detail_uv_dx = dpdx(continuous_uv);
  let detail_uv_dy = dpdy(continuous_uv);
  if frame.lod_options.x > 0.5 {
    return vec4<f32>(geometry_source_debug_color(input), 1.0);
  }
  if distance_to_camera >= 144.0 {
    let distant_radiance = distant_surface_radiance(input, material, sun);
    return vec4<f32>(
      transport_surface_radiance(distant_radiance, input.world, sun),
      1.0,
    );
  }
  let detail_basis = surface_basis(input.world, input.normal);
  let surface_detail = sample_surface_detail(
    input.world,
    input.normal,
    material,
    detail_basis,
    detail_uv_dx,
    detail_uv_dy,
    distance_to_camera,
  );
  let surface_weather = input.surface_weather;
  let shadow = sun_visibility(input.world, input.normal)
    * surface_weather.x
    * input.terrain_lighting.x;
  let sky_visibility = surface_detail.normal.y * 0.5 + 0.5;
  let cell = floor(input.world / frame.viewport_voxel.z);
  let flat_grain = mix(0.88, 1.12, hash31(cell + vec3<f32>(f32(material) * 3.1)));
  let detail_grain = mix(0.96, 1.04, hash31(cell + vec3<f32>(f32(material) * 3.1)));
  let grain = select(flat_grain, detail_grain, MATERIAL_DETAIL != 0u);
  let fine_grain = select(
    mix(0.96, 1.04, hash31(floor(input.world * 28.0))),
    1.0,
    MATERIAL_DETAIL != 0u,
  );
  let voxel_ambient_occlusion = select(1.0, mix(0.52, 1.0, input.ao), frame.render_options.x > 0.5);
  let spatial_ambient_occlusion = screen_space_ambient_visibility(input.position.xy, input.world);
  let ambient_occlusion = min(voxel_ambient_occlusion, spatial_ambient_occlusion)
    * input.terrain_lighting.y;
  let interior_ambient = mix(1.0, 0.05, frame.interior.x);
  let sky_irradiance = mix(frame.ground_atmosphere.rgb, frame.sky_horizon.rgb * 0.48, sky_visibility)
    * interior_ambient;
  let bounce = frame.ground_atmosphere.rgb
    * max(-surface_detail.normal.y, 0.0)
    * 0.35
    * interior_ambient;
  let dry_albedo = surface_detail.albedo
    * material_macro_tint(material, input.world)
    * grain
    * fine_grain;
  // A thin water film darkens the diffuse substrate and narrows its microfacet distribution.
  // Its air/water normal-incidence Fresnel value is 0.02037. The exact shared cloud footprint
  // ensures that exposed nearby surfaces only acquire the live sheen beneath active rain.
  let rain_exposure = smoothstep(-0.15, 0.65, input.normal.y);
  let can_be_wet = select(1.0, 0.0, material == 13u || material == 14u);
  let wetness = clamp(
    surface_weather.y
      * rain_exposure
      * (1.0 - frame.interior.x)
      * (1.0 - frame.medium.x)
      * can_be_wet,
    0.0,
    1.0,
  );
  let albedo = dry_albedo * mix(1.0, 0.64, wetness);
  let view_direction = normalize(frame.camera_time.xyz - input.world);
  let roughness = mix(
    surface_detail.roughness,
    max(MIN_PERCEPTUAL_ROUGHNESS, surface_detail.roughness * 0.24),
    wetness,
  );
  let dielectric_f0 = mix(DIELECTRIC_F0, vec3<f32>(0.02037), wetness);
  let no_v = max(dot(surface_detail.normal, view_direction), 0.0001);
  let ambient_fresnel = fresnel_schlick_roughness(no_v, dielectric_f0, roughness);
  let ambient_diffuse = albedo
    * (vec3<f32>(1.0) - ambient_fresnel)
    * (sky_irradiance + bounce)
    * ambient_occlusion;
  let reflection_direction = reflect(-view_direction, surface_detail.normal);
  let reflection_radiance = environment_radiance(
    mix(
      reflection_direction,
      surface_detail.normal,
      roughness * roughness,
    ),
  );
  let reflection_horizon = smoothstep(
    -0.10,
    0.15,
    dot(reflection_direction, normalize(input.normal)),
  );
  let ambient_specular = reflection_radiance
    * ambient_fresnel
    * specular_ambient_visibility(no_v, ambient_occlusion, roughness)
    * reflection_horizon
    * interior_ambient;
  let direct = frame.key_light_radiance.rgb
    * evaluate_direct_dielectric_f0(
      albedo,
      roughness,
      dielectric_f0,
      surface_detail.normal,
      view_direction,
      sun,
    )
    * shadow
    * frame.key_light_direction.w
    * 0.62;
  var color = ambient_diffuse + ambient_specular + direct;
  for (var light_index = 0u; light_index < 16u; light_index += 1u) {
    if light_index >= local_light_uniform.metadata.x {
      break;
    }
    let light = local_light_uniform.lights[light_index];
    let to_light = light.position_radius.xyz - input.world;
    let distance_squared = dot(to_light, to_light);
    let radius_squared = light.position_radius.w * light.position_radius.w;
    if distance_squared >= radius_squared {
      continue;
    }
    let inverse_distance = inverseSqrt(max(distance_squared, 0.000001));
    let light_direction = to_light * inverse_distance;
    let normalized_squared = distance_squared / radius_squared;
    let window = max(1.0 - normalized_squared * normalized_squared, 0.0);
    let attenuation = window * window / max(distance_squared, 0.15 * 0.15);
    let radiance = light.color_intensity.rgb * light.color_intensity.w * attenuation;
    color += radiance * evaluate_direct_dielectric_f0(
      albedo,
      roughness,
      dielectric_f0,
      surface_detail.normal,
      view_direction,
      light_direction,
    );
  }
  if material == 9u {
    let leaf_scatter = pow(max(dot(-sun, view_direction), 0.0), 3.0) * (1.0 - shadow * 0.55);
    color += albedo * frame.key_light_radiance.rgb * leaf_scatter * 0.035;
  }
  if material == 14u {
    let crystal_pulse = 0.86 + sin(input.world.y * 9.0 + input.world.x * 3.0) * 0.08;
    color += srgb_to_linear(vec3<f32>(0.10, 0.72, 0.96)) * crystal_pulse * 1.45;
  }
  if frame.interior.w > 0.0001 {
    let camera_to_surface = input.world - frame.camera_time.xyz;
    let lamp_distance = length(camera_to_surface);
    let lamp_ray = camera_to_surface / max(lamp_distance, 0.0001);
    let cone = smoothstep(0.76, 0.93, dot(lamp_ray, normalize(frame.camera_forward.xyz)));
    let range = 1.0 - smoothstep(2.0, 13.0, lamp_distance);
    let incidence = max(dot(surface_detail.normal, -lamp_ray), 0.0);
    let lamp = cone * range * range * (0.18 + incidence * 0.82) * frame.interior.w;
    color += albedo * vec3<f32>(3.2, 2.65, 2.15) * lamp * 0.36;
  }
  if frame.medium.x > 0.0001 && input.normal.y > 0.35 {
    let phase_a = sin(input.world.x * 5.1 + frame.camera_time.w * 1.7)
      * sin(input.world.z * 4.3 - frame.camera_time.w * 1.2);
    let phase_b = sin((input.world.x + input.world.z) * 8.7 - frame.camera_time.w * 2.1);
    let caustic = pow(clamp(phase_a * 0.55 + phase_b * 0.25 + 0.55, 0.0, 1.0), 5.0);
    let water_depth = max(frame.medium.w - input.world.y, 0.0);
    let below_surface = smoothstep(0.0, 0.08, frame.medium.w - input.world.y);
    let caustic_fade = exp(-water_depth * 0.32)
      * below_surface
      * smoothstep(0.35, 0.9, input.normal.y);
    color += frame.key_light_radiance.rgb
      * vec3<f32>(0.36, 0.78, 0.84)
      * caustic
      * caustic_fade
      * shadow
      * frame.medium.x
      * 0.08;
  }
  let inside_position = input.world - input.normal * frame.viewport_voxel.z * 0.02;
  let voxel = floor(inside_position / frame.viewport_voxel.z);
  let target_center = (frame.target_voxel.xyz + frame.target_voxel_max.xyz) * 0.5;
  let target_delta = voxel - target_center;
  let target_is_cube = frame.target_voxel.w > 1.5;
  let inside_target_shape = target_is_cube || dot(target_delta, target_delta) < 39.0;
  let targeted = frame.render_options.w > 0.5
    && frame.target_voxel.w > 0.5
    && all(voxel >= frame.target_voxel.xyz)
    && all(voxel <= frame.target_voxel_max.xyz)
    && inside_target_shape;
  if targeted {
    let coordinate = fract(input.world / frame.viewport_voxel.z + vec3<f32>(0.0001));
    var edge = 1.0;
    if abs(input.normal.x) < 0.5 { edge = min(edge, min(coordinate.x, 1.0 - coordinate.x)); }
    if abs(input.normal.y) < 0.5 { edge = min(edge, min(coordinate.y, 1.0 - coordinate.y)); }
    if abs(input.normal.z) < 0.5 { edge = min(edge, min(coordinate.z, 1.0 - coordinate.z)); }
    let outline = 1.0 - smoothstep(0.045, 0.085, edge);
    color = mix(color, vec3<f32>(1.4, 1.08, 0.42), outline * 0.88);
  }
  if distance_to_camera > 96.0 {
    let distant_radiance = distant_surface_radiance(input, material, sun);
    color = mix(
      color,
      distant_radiance,
      smoothstep(96.0, 144.0, distance_to_camera),
    );
  }
  return vec4<f32>(transport_surface_radiance(color, input.world, sun), 1.0);
}

/// One machine-readable identity record for the winning terrain fragment at each screenshot
/// pixel. A second single-channel integer target stores the exact reverse-Z f32 bit pattern.
/// Together with the reproduction package's inverse view-projection matrix and integer pixel
/// coordinate this reconstructs world position without spending three more 32-bit render targets.
struct DiagnosticFragmentOut {
  @location(0) identity: vec4<u32>,
  @location(1) reverse_z_depth: u32,
};

@fragment
fn fs_diagnostic(input: VertexOut) -> DiagnosticFragmentOut {
  var out: DiagnosticFragmentOut;
  out.identity = input.terrain_identity;
  out.reverse_z_depth = bitcast<u32>(input.position.z);
  return out;
}
