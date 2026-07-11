struct Frame {
  view_projection: mat4x4<f32>,
  camera_time: vec4<f32>,
  viewport_voxel: vec4<f32>,
};

@group(0) @binding(0) var<uniform> frame: Frame;

struct VertexOut {
  @builtin(position) position: vec4<f32>,
  @location(0) world: vec3<f32>,
  @location(1) normal: vec3<f32>,
  @location(2) @interpolate(flat) material: u32,
};

const CORNERS = array<vec2<f32>, 6>(
  vec2<f32>(0.0, 0.0),
  vec2<f32>(1.0, 0.0),
  vec2<f32>(1.0, 1.0),
  vec2<f32>(0.0, 0.0),
  vec2<f32>(1.0, 1.0),
  vec2<f32>(0.0, 1.0),
);

@vertex
fn vs_main(
  @builtin(vertex_index) vertex_index: u32,
  @location(0) origin: vec3<f32>,
  @location(1) face: u32,
  @location(2) extent: vec2<f32>,
  @location(3) material: u32,
) -> VertexOut {
  let uv = CORNERS[vertex_index];
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
  return out;
}

fn material_color(material: u32) -> vec3<f32> {
  switch material {
    case 1u: { return vec3<f32>(0.32, 0.56, 0.22); }
    case 2u: { return vec3<f32>(0.38, 0.25, 0.14); }
    case 3u: { return vec3<f32>(0.42, 0.45, 0.48); }
    case 4u: { return vec3<f32>(0.74, 0.64, 0.38); }
    case 5u: { return vec3<f32>(0.84, 0.91, 0.94); }
    case 6u: { return vec3<f32>(0.57, 0.32, 0.22); }
    case 7u: { return vec3<f32>(0.20, 0.23, 0.27); }
    case 8u: { return vec3<f32>(0.36, 0.22, 0.10); }
    case 9u: { return vec3<f32>(0.20, 0.46, 0.20); }
    default: { return vec3<f32>(1.0, 0.0, 1.0); }
  }
}

@fragment
fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
  let sun = normalize(vec3<f32>(0.55, 0.82, 0.28));
  let diffuse = max(dot(input.normal, sun), 0.0);
  let up = input.normal.y * 0.08;
  let light = 0.38 + diffuse * 0.62 + up;
  var color = material_color(input.material) * light;
  let distance_to_camera = distance(input.world, frame.camera_time.xyz);
  let fog = smoothstep(frame.viewport_voxel.w * 0.62, frame.viewport_voxel.w, distance_to_camera);
  let fog_color = vec3<f32>(0.24, 0.39, 0.55);
  color = mix(color, fog_color, fog);
  return vec4<f32>(color, 1.0);
}
