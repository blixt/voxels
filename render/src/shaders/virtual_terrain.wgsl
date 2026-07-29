struct CandidatePage {
  // opaque surface, opaque triangle, water surface, water triangle.
  ranges: array<vec2<u32>, 4>,
  // Stream-relative exclusive prefixes computed by the CPU.
  destinations: array<u32, 4>,
};

struct SnapshotCounters {
  element_counts: array<u32, 4>,
  encoded_pages: u32,
  overflow_flags: atomic<u32>,
  generation: vec2<u32>,
  fingerprint: vec2<u32>,
  selected_count: u32,
  ownerless_roots: u32,
  source_element_capacities: vec2<u32>,
  reserved: vec2<u32>,
};

@group(0) @binding(0) var<storage, read> candidates: array<CandidatePage>;
@group(0) @binding(1) var<storage, read_write> handles: array<u32>;
@group(0) @binding(2) var<storage, read_write> counters: SnapshotCounters;

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
const HANDLE_MISMATCH: u32 = 64u;

fn candidate_range_error_flags(page: CandidatePage, stream: u32) -> u32 {
  let first_handle = page.ranges[stream].x;
  let count = page.ranges[stream].y;
  let source_segment = first_handle >> 31u;
  let first_element = first_handle & HANDLE_ELEMENT_MASK;
  let source_capacity = select(
    counters.source_element_capacities.x,
    counters.source_element_capacities.y,
    source_segment == 1u,
  );
  let destination = page.destinations[stream];

  var flags = 0u;
  if first_element > source_capacity
      || count > source_capacity - min(first_element, source_capacity) {
    flags |= OVERFLOW_SOURCE;
  }
  if destination > STREAM_CAPACITIES[stream]
      || count > STREAM_CAPACITIES[stream] - min(destination, STREAM_CAPACITIES[stream]) {
    flags |= OVERFLOW_STREAM_BASE << stream;
  }
  return flags;
}

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
  for (var stream = 0u; stream < 4u; stream += 1u) {
    let error_flags = candidate_range_error_flags(page, stream);
    if error_flags != 0u {
      if local.x == 0u {
        atomicOr(&counters.overflow_flags, error_flags);
      }
      continue;
    }
    let first_handle = page.ranges[stream].x;
    let count = page.ranges[stream].y;
    let destination = STREAM_OFFSETS[stream] + page.destinations[stream];
    for (var element = local.x; element < count; element += 64u) {
      handles[destination + element] = first_handle + element;
    }
  }
}

// This is intentionally a separate compute pass from encoding. WebGPU guarantees storage writes
// from the first pass are visible here, and validation independently re-walks every descriptor and
// compares every exact destination/value pair. The valid path performs no atomic operations.
@compute @workgroup_size(64)
fn validate_snapshot(
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
  for (var stream = 0u; stream < 4u; stream += 1u) {
    let error_flags = candidate_range_error_flags(page, stream);
    if error_flags != 0u {
      if local.x == 0u {
        atomicOr(&counters.overflow_flags, error_flags);
      }
      continue;
    }
    let first_handle = page.ranges[stream].x;
    let count = page.ranges[stream].y;
    let destination = STREAM_OFFSETS[stream] + page.destinations[stream];
    for (var element = local.x; element < count; element += 64u) {
      if handles[destination + element] != first_handle + element {
        atomicOr(&counters.overflow_flags, HANDLE_MISMATCH);
        return;
      }
    }
  }
}
