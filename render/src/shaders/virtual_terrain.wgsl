struct CandidatePage {
  // opaque surface, opaque triangle, water surface, water triangle.
  ranges: array<vec2<u32>, 4>,
};

struct SnapshotCounters {
  element_counts: array<atomic<u32>, 4>,
  encoded_pages: atomic<u32>,
  overflow_flags: atomic<u32>,
  generation: vec2<u32>,
  fingerprint: vec2<u32>,
  selected_count: u32,
  ownerless_roots: u32,
  source_element_capacities: vec2<u32>,
  reserved: vec2<u32>,
  handle_fingerprint_sums: array<atomic<u32>, 4>,
  handle_fingerprint_squares: array<atomic<u32>, 4>,
};

@group(0) @binding(0) var<storage, read> candidates: array<CandidatePage>;
@group(0) @binding(1) var<storage, read_write> handles: array<u32>;
@group(0) @binding(2) var<storage, read_write> counters: SnapshotCounters;
@group(0) @binding(3) var<storage, read_write> indirect_commands: array<u32>;

const STREAM_CAPACITIES = array<u32, 4>(
  2796202u,
  4194304u,
  699050u,
  699050u,
);
const STREAM_OFFSETS = array<u32, 4>(
  0u,
  2796202u,
  6990506u,
  7689556u,
);
const HANDLE_ELEMENT_MASK: u32 = 0x7fffffffu;
const OVERFLOW_STREAM_BASE: u32 = 1u;
const OVERFLOW_SOURCE: u32 = 16u;
const OVERFLOW_DIRECTORY: u32 = 32u;

var<workgroup> destinations: array<u32, 4>;
var<workgroup> page_valid: u32;

@compute @workgroup_size(64)
fn encode_snapshot(
  @builtin(workgroup_id) workgroup: vec3<u32>,
  @builtin(local_invocation_id) local: vec3<u32>,
) {
  let page_index = workgroup.x;
  if page_index >= counters.selected_count || page_index >= arrayLength(&candidates) {
    if local.x == 0u {
      atomicOr(&counters.overflow_flags, OVERFLOW_DIRECTORY);
    }
    return;
  }
  let page = candidates[page_index];
  if local.x == 0u {
    page_valid = 1u;
    for (var stream = 0u; stream < 4u; stream += 1u) {
      let first_handle = page.ranges[stream].x;
      let count = page.ranges[stream].y;
      let source_segment = first_handle >> 31u;
      let first_element = first_handle & HANDLE_ELEMENT_MASK;
      let source_capacity = select(
        counters.source_element_capacities.x,
        counters.source_element_capacities.y,
        source_segment == 1u,
      );
      if source_segment > 1u
          || first_element > source_capacity
          || count > source_capacity - first_element {
        page_valid = 0u;
        atomicOr(&counters.overflow_flags, OVERFLOW_SOURCE);
      }
      let destination = atomicAdd(&counters.element_counts[stream], count);
      destinations[stream] = destination;
      if destination > STREAM_CAPACITIES[stream]
          || count > STREAM_CAPACITIES[stream] - destination {
        page_valid = 0u;
        atomicOr(&counters.overflow_flags, OVERFLOW_STREAM_BASE << stream);
      }
    }
    if page_valid != 0u {
      atomicAdd(&counters.encoded_pages, 1u);
    }
  }
  workgroupBarrier();
  if page_valid == 0u {
    return;
  }
  for (var stream = 0u; stream < 4u; stream += 1u) {
    let first_handle = page.ranges[stream].x;
    let count = page.ranges[stream].y;
    let destination = STREAM_OFFSETS[stream] + destinations[stream];
    for (var element = local.x; element < count; element += 64u) {
      handles[destination + element] = first_handle + element;
    }
  }
}

@compute @workgroup_size(256)
fn validate_snapshot(@builtin(global_invocation_id) global: vec3<u32>) {
  let compact_index = global.x;
  let surface_end = atomicLoad(&counters.element_counts[0]);
  let triangle_end = surface_end + atomicLoad(&counters.element_counts[1]);
  let water_surface_end = triangle_end + atomicLoad(&counters.element_counts[2]);
  let water_triangle_end = water_surface_end + atomicLoad(&counters.element_counts[3]);
  if compact_index >= water_triangle_end {
    return;
  }
  var stream = 0u;
  var stream_index = compact_index;
  if compact_index >= water_surface_end {
    stream = 3u;
    stream_index -= water_surface_end;
  } else if compact_index >= triangle_end {
    stream = 2u;
    stream_index -= triangle_end;
  } else if compact_index >= surface_end {
    stream = 1u;
    stream_index -= surface_end;
  }
  let handle = handles[STREAM_OFFSETS[stream] + stream_index];
  atomicAdd(&counters.handle_fingerprint_sums[stream], handle);
  atomicAdd(&counters.handle_fingerprint_squares[stream], handle * handle);
}

@compute @workgroup_size(1)
fn finalize_snapshot() {
  let overflow = atomicLoad(&counters.overflow_flags);
  let surface = min(atomicLoad(&counters.element_counts[0]), STREAM_CAPACITIES[0]);
  let triangle = min(atomicLoad(&counters.element_counts[1]), STREAM_CAPACITIES[1]);
  let water_surface = min(atomicLoad(&counters.element_counts[2]), STREAM_CAPACITIES[2]);
  let water_triangle = min(atomicLoad(&counters.element_counts[3]), STREAM_CAPACITIES[3]);

  indirect_commands[0] = 4u;
  indirect_commands[1] = select(surface, 0u, overflow != 0u);
  indirect_commands[2] = 0u;
  indirect_commands[3] = 0u;
  indirect_commands[4] = select(triangle, 0u, overflow != 0u);
  indirect_commands[5] = 1u;
  indirect_commands[6] = 0u;
  indirect_commands[7] = 0u;
  indirect_commands[8] = 4u;
  indirect_commands[9] = select(water_surface, 0u, overflow != 0u);
  indirect_commands[10] = 0u;
  indirect_commands[11] = 0u;
  indirect_commands[12] = select(water_triangle, 0u, overflow != 0u);
  indirect_commands[13] = 1u;
  indirect_commands[14] = 0u;
  indirect_commands[15] = 0u;
}
