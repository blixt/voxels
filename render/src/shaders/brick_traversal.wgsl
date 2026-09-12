// Direct voxel traversal prototype. The host supplies a power-of-two hash table whose entries
// point at the fixed-capacity material atlas. `metadata.w` is 1 for a resident empty brick and 2
// for a resident brick containing material. Missing entries are reported as unknown; they are
// never treated as air while the world stream is incomplete. The table is deliberately separate
// from the linear atlas descriptor buffer so hash rebuilds can be measured independently.

struct BrickEntry {
  coord: vec4<i32>,
  metadata: vec4<u32>,
};

struct TraceRay {
  origin: vec4<f32>,
  direction: vec4<f32>,
};

struct TraceResult {
  voxel: vec4<i32>,
  material_distance: vec4<u32>,
};

struct TraceParams {
  hash_mask: u32,
  max_distance_voxels: f32,
  ray_count: u32,
  width: u32,
  height: u32,
  reserved: vec3<u32>,
};

@group(0) @binding(0) var<storage, read> brick_entries: array<BrickEntry>;
@group(0) @binding(1) var<storage, read> brick_materials: array<u32>;
@group(0) @binding(2) var<storage, read> rays: array<TraceRay>;
@group(0) @binding(3) var<storage, read_write> results: array<TraceResult>;
@group(0) @binding(4) var<uniform> params: TraceParams;
@group(0) @binding(5) var output_texture: texture_storage_2d<rgba8unorm, write>;

const BRICK_EDGE: i32 = 8;
const BRICK_WORD_COUNT: u32 = 128u;
const INVALID_SLOT: u32 = 0xffffffffu;
const STATUS_MISS: u32 = 0u;
const STATUS_HIT: u32 = 1u;
const STATUS_UNKNOWN: u32 = 2u;
const MAX_PROBES: u32 = 8u;
const MAX_STEPS: u32 = 1048576u;
const INF: f32 = 3.402823e+38;

fn brick_hash(coord: vec3<i32>) -> u32 {
  var hash = 2166136261u;
  hash = (hash ^ bitcast<u32>(coord.x)) * 16777619u;
  hash = (hash ^ bitcast<u32>(coord.y)) * 16777619u;
  hash = (hash ^ bitcast<u32>(coord.z)) * 16777619u;
  return hash;
}

fn find_brick(coord: vec3<i32>) -> vec2<u32> {
  if arrayLength(&brick_entries) == 0u || (params.hash_mask + 1u) > arrayLength(&brick_entries) {
    return vec2<u32>(INVALID_SLOT, 0u);
  }
  let start = brick_hash(coord) & params.hash_mask;
  for (var probe = 0u; probe < MAX_PROBES; probe += 1u) {
    let index = (start + probe) & params.hash_mask;
    let entry = brick_entries[index];
    let slot = entry.metadata.x;
    if slot == INVALID_SLOT {
      return vec2<u32>(INVALID_SLOT, 0u);
    }
    if all(entry.coord.xyz == coord) {
      return vec2<u32>(slot, entry.metadata.w);
    }
  }
  return vec2<u32>(INVALID_SLOT, 0u);
}

fn read_material(slot: u32, local: vec3<i32>) -> u32 {
  let index = u32(local.x + local.z * BRICK_EDGE + local.y * BRICK_EDGE * BRICK_EDGE);
  let word = brick_materials[slot * BRICK_WORD_COUNT + index / 4u];
  return (word >> ((index & 3u) * 8u)) & 0xffu;
}

fn floor_i(value: f32) -> i32 {
  return i32(floor(value));
}

fn finite_f32(value: f32) -> bool {
  return value == value && abs(value) < 3.402823e+38;
}

fn finite_vec3(value: vec3<f32>) -> bool {
  return finite_f32(value.x) && finite_f32(value.y) && finite_f32(value.z);
}

fn output_pixel(ray_index: u32, color: vec4<f32>) {
  if params.width == 0u || params.height == 0u {
    return;
  }
  let pixel = vec2<u32>(ray_index % params.width, ray_index / params.width);
  if pixel.y < params.height {
    textureStore(output_texture, pixel, color);
  }
}

fn material_color(material: u32) -> vec4<f32> {
  let hue = f32(material & 31u) / 31.0;
  let band = f32((material >> 5u) & 7u) / 7.0;
  let color = vec3<f32>(
    0.22 + 0.58 * hue,
    0.28 + 0.42 * (1.0 - abs(hue - 0.5) * 1.6),
    0.20 + 0.56 * band,
  );
  return vec4<f32>(color, 1.0);
}

@compute @workgroup_size(64)
fn trace_voxels(@builtin(global_invocation_id) invocation: vec3<u32>) {
  let ray_index = invocation.x;
  if ray_index >= params.ray_count || ray_index >= arrayLength(&rays) || ray_index >= arrayLength(&results) {
    return;
  }
  let ray = rays[ray_index];
  let direction_length = length(ray.direction.xyz);
  if !finite_vec3(ray.origin.xyz) || !finite_vec3(ray.direction.xyz)
      || !finite_f32(direction_length) || direction_length <= 0.000001
      || !finite_f32(params.max_distance_voxels) || params.max_distance_voxels <= 0.0 {
    results[ray_index].material_distance = vec4<u32>(0u, STATUS_MISS, 0u, 0u);
    output_pixel(ray_index, vec4<f32>(0.0));
    return;
  }
  let direction = ray.direction.xyz / direction_length;
  var voxel = vec3<i32>(floor_i(ray.origin.x), floor_i(ray.origin.y), floor_i(ray.origin.z));
  var next = vec3<f32>(INF);
  var delta = vec3<f32>(INF);
  let step = vec3<i32>(select(0, 1, direction.x > 0.0) + select(-1, 0, direction.x >= 0.0),
                       select(0, 1, direction.y > 0.0) + select(-1, 0, direction.y >= 0.0),
                       select(0, 1, direction.z > 0.0) + select(-1, 0, direction.z >= 0.0));
  for (var axis = 0u; axis < 3u; axis += 1u) {
    if step[axis] != 0 {
      let boundary = select(f32(voxel[axis]), f32(voxel[axis] + 1), step[axis] > 0);
      next[axis] = (boundary - ray.origin[axis]) / direction[axis];
      delta[axis] = 1.0 / abs(direction[axis]);
    }
  }

  var distance = 0.0;
  for (var iteration = 0u; iteration < MAX_STEPS; iteration += 1u) {
    let brick = vec3<i32>(
      floor_i(f32(voxel.x) / 8.0),
      floor_i(f32(voxel.y) / 8.0),
      floor_i(f32(voxel.z) / 8.0),
    );
    let found = find_brick(brick);
    if found.x == INVALID_SLOT {
      results[ray_index].voxel = vec4<i32>(voxel, 0);
      results[ray_index].material_distance = vec4<u32>(0u, STATUS_UNKNOWN, bitcast<u32>(distance), 0u);
      output_pixel(ray_index, vec4<f32>(0.0));
      return;
    }
    if found.y == 0u {
      var brick_next = vec3<f32>(INF);
      for (var axis = 0u; axis < 3u; axis += 1u) {
        if step[axis] != 0 {
          let edge = select(f32(brick[axis] * BRICK_EDGE), f32((brick[axis] + 1) * BRICK_EDGE), step[axis] > 0);
          brick_next[axis] = (edge - ray.origin[axis]) / direction[axis];
        }
      }
      let skip = min(brick_next.x, min(brick_next.y, brick_next.z));
      if !finite_f32(skip) || skip > params.max_distance_voxels {
        results[ray_index].material_distance = vec4<u32>(0u, STATUS_MISS, 0u, 0u);
        output_pixel(ray_index, vec4<f32>(0.0));
        return;
      }
      distance = skip;
      let point = ray.origin.xyz + direction * (distance + 0.0001);
      voxel = vec3<i32>(floor_i(point.x), floor_i(point.y), floor_i(point.z));
      for (var axis = 0u; axis < 3u; axis += 1u) {
        if step[axis] != 0 {
          let boundary = select(f32(voxel[axis]), f32(voxel[axis] + 1), step[axis] > 0);
          next[axis] = (boundary - ray.origin[axis]) / direction[axis];
        }
      }
      continue;
    }
    let local = vec3<i32>(
      voxel.x - brick.x * BRICK_EDGE,
      voxel.y - brick.y * BRICK_EDGE,
      voxel.z - brick.z * BRICK_EDGE,
    );
    let material = read_material(found.x, local);
    if material != 0u {
      results[ray_index].voxel = vec4<i32>(voxel, 0);
      results[ray_index].material_distance = vec4<u32>(material, STATUS_HIT, bitcast<u32>(distance), 0u);
      output_pixel(ray_index, material_color(material));
      return;
    }
    let axis = select(0u, 1u, next.y < next.x);
    let chosen_axis = select(axis, 2u, next.z < next[axis]);
    distance = next[chosen_axis];
    if distance > params.max_distance_voxels {
      results[ray_index].material_distance = vec4<u32>(0u, STATUS_MISS, 0u, 0u);
      output_pixel(ray_index, vec4<f32>(0.0));
      return;
    }
    voxel[chosen_axis] += step[chosen_axis];
    next[chosen_axis] += delta[chosen_axis];
  }
  results[ray_index].voxel = vec4<i32>(voxel, 0);
  results[ray_index].material_distance = vec4<u32>(0u, STATUS_UNKNOWN, bitcast<u32>(distance), 0u);
  output_pixel(ray_index, vec4<f32>(0.0));
}
