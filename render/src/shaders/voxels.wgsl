struct Frame {
  view_projection: mat4x4<f32>,
  inverse_view_projection: mat4x4<f32>,
  camera_time: vec4<f32>,
  viewport_voxel: vec4<f32>,
};

@group(0) @binding(0) var<uniform> frame: Frame;

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
    case 1u: { return vec3<f32>(0.22, 0.52, 0.13); }
    case 2u: { return vec3<f32>(0.36, 0.20, 0.095); }
    case 3u: { return vec3<f32>(0.34, 0.38, 0.43); }
    case 4u: { return vec3<f32>(0.72, 0.53, 0.25); }
    case 5u: { return vec3<f32>(0.76, 0.86, 0.91); }
    case 6u: { return vec3<f32>(0.56, 0.25, 0.15); }
    case 7u: { return vec3<f32>(0.12, 0.15, 0.20); }
    case 8u: { return vec3<f32>(0.31, 0.15, 0.055); }
    case 9u: { return vec3<f32>(0.10, 0.40, 0.13); }
    case 10u: { return vec3<f32>(0.08, 0.34, 0.16); }
    case 11u: { return vec3<f32>(0.58, 0.55, 0.44); }
    case 12u: { return vec3<f32>(0.62, 0.20, 0.075); }
    default: { return vec3<f32>(1.0, 0.0, 1.0); }
  }
}

fn hash31(position: vec3<f32>) -> f32 {
  let value = dot(position, vec3<f32>(127.1, 311.7, 74.7));
  return fract(sin(value) * 43758.5453);
}

@fragment
fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
  let far_surface = (input.material & 0x80000000u) != 0u;
  let material = input.material & 0xffffu;
  let distance_xz = distance(input.world.xz, frame.camera_time.xz);
  let transition_cell = floor(input.world / frame.viewport_voxel.z);
  if far_surface {
    let handoff = smoothstep(6.3, 7.3, distance_xz);
    let threshold = hash31(transition_cell + vec3<f32>(17.0));
    if handoff < threshold {
      discard;
    }
  } else {
    if (material == 8u || material == 9u) && distance_xz > 9.0 {
      discard;
    }
    let ownership = 1.0 - smoothstep(10.0, 12.0, distance_xz);
    let threshold = hash31(transition_cell + vec3<f32>(41.0));
    if ownership < threshold {
      discard;
    }
  }
  let sun = normalize(vec3<f32>(0.48, 0.72, 0.35));
  let diffuse = max(dot(input.normal, sun), 0.0);
  let sky_visibility = input.normal.y * 0.5 + 0.5;
  let bounce = max(-input.normal.y, 0.0) * 0.08;
  let light = 0.46 + sky_visibility * 0.22 + diffuse * 0.48 + bounce;
  let cell = floor(input.world / frame.viewport_voxel.z);
  let grain = mix(0.88, 1.12, hash31(cell + vec3<f32>(f32(material) * 3.1)));
  let fine_grain = mix(0.96, 1.04, hash31(floor(input.world * 28.0)));
  let ambient_occlusion = mix(0.52, 1.0, input.ao);
  var color = material_color(material) * light * grain * fine_grain * ambient_occlusion;
  let distance_to_camera = distance(input.world, frame.camera_time.xyz);
  let distance_fog = smoothstep(frame.viewport_voxel.w * 0.58, frame.viewport_voxel.w, distance_to_camera);
  let height_fog = exp(-max(input.world.y, 0.0) * 0.16) * smoothstep(2.5, frame.viewport_voxel.w, distance_to_camera) * 0.18;
  let fog = clamp(distance_fog + height_fog, 0.0, 1.0);
  let fog_color = vec3<f32>(0.49, 0.62, 0.72);
  color = mix(color, fog_color, fog);
  let mapped = color / (color + vec3<f32>(0.72));
  return vec4<f32>(max(mapped, vec3<f32>(0.0)), 1.0);
}
