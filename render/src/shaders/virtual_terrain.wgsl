struct CandidatePage {
  // opaque surface, opaque triangle, water surface, water triangle.
  ranges: array<vec2<u32>, 4>,
  additional_ranges: array<array<vec2<u32>, 8>, 4>,
  range_counts: array<u32, 4>,
  // Stream-relative exclusive prefixes computed by the CPU.
  destinations: array<u32, 4>,
};

struct SnapshotCounters {
  element_counts: array<u32, 4>,
  encoded_pages: atomic<u32>,
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
@group(0) @binding(3) var<storage, read_write> page_tokens: array<u32>;

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
const OVERFLOW_STRUCTURE: u32 = 64u;
const HANDLE_MISMATCH: u32 = 128u;

var<workgroup> lane_failures: array<u32, 64>;

fn candidate_range(page: CandidatePage, stream: u32, range: u32) -> vec2<u32> {
  if range == 0u {
    return page.ranges[stream];
  }
  return page.additional_ranges[stream][range - 1u];
}

fn candidate_stream_count(page: CandidatePage, stream: u32) -> u32 {
  var count = 0u;
  for (var range = 0u; range < page.range_counts[stream]; range += 1u) {
    count += candidate_range(page, stream, range).y;
  }
  return count;
}

fn candidate_range_error_flags(
  page: CandidatePage,
  stream: u32,
  range_index: u32,
  destination: u32,
) -> u32 {
  let range = candidate_range(page, stream, range_index);
  let first_handle = range.x;
  let count = range.y;
  let source_segment = first_handle >> 31u;
  let first_element = first_handle & HANDLE_ELEMENT_MASK;
  let source_capacity = select(
    counters.source_element_capacities.x,
    counters.source_element_capacities.y,
    source_segment == 1u,
  );
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

// One unique workgroup validates one descriptor. The token buffer is cleared immediately before
// this pass, so a missing dispatch cannot inherit success from an older generation.
@compute @workgroup_size(1)
fn validate_candidate_structure(@builtin(workgroup_id) workgroup: vec3<u32>) {
  let page_index = workgroup.x;
  if page_index >= counters.selected_count
      || page_index >= arrayLength(&candidates)
      || page_index >= arrayLength(&page_tokens) {
    atomicOr(&counters.overflow_flags, OVERFLOW_DIRECTORY);
    return;
  }

  let page = candidates[page_index];
  var error_flags = 0u;
  for (var stream = 0u; stream < 4u; stream += 1u) {
    if page.range_counts[stream] > 9u {
      error_flags |= OVERFLOW_STRUCTURE;
      continue;
    }
    var range_destination = page.destinations[stream];
    for (var range = 0u; range < page.range_counts[stream]; range += 1u) {
      let geometry_range = candidate_range(page, stream, range);
      if geometry_range.y == 0u {
        error_flags |= OVERFLOW_STRUCTURE;
      }
      error_flags |= candidate_range_error_flags(page, stream, range, range_destination);
      range_destination += geometry_range.y;
    }

    var expected_destination = 0u;
    if page_index > 0u {
      let previous = candidates[page_index - 1u];
      let previous_destination = previous.destinations[stream];
      let previous_count = candidate_stream_count(previous, stream);
      if previous_destination > STREAM_CAPACITIES[stream]
          || previous_count > STREAM_CAPACITIES[stream]
              - min(previous_destination, STREAM_CAPACITIES[stream]) {
        error_flags |= OVERFLOW_STREAM_BASE << stream;
      } else {
        expected_destination = previous_destination + previous_count;
      }
    }
    if page.destinations[stream] != expected_destination {
      error_flags |= OVERFLOW_STRUCTURE;
    }

    if page_index + 1u == counters.selected_count {
      let end = page.destinations[stream] + candidate_stream_count(page, stream);
      if end != counters.element_counts[stream] {
        error_flags |= OVERFLOW_STRUCTURE;
      }
    }
  }

  if error_flags == 0u {
    page_tokens[page_index] = 1u;
  } else {
    atomicOr(&counters.overflow_flags, error_flags);
  }
}

@compute @workgroup_size(64)
fn encode_snapshot(
  @builtin(workgroup_id) workgroup: vec3<u32>,
  @builtin(local_invocation_id) local: vec3<u32>,
) {
  let page_index = workgroup.x;
  if page_index >= counters.selected_count
      || page_index >= arrayLength(&candidates)
      || page_index >= arrayLength(&page_tokens) {
    if local.x == 0u {
      atomicOr(&counters.overflow_flags, OVERFLOW_DIRECTORY);
    }
    return;
  }
  if page_tokens[page_index] != 1u {
    if local.x == 0u {
      atomicOr(&counters.overflow_flags, OVERFLOW_STRUCTURE);
    }
    return;
  }

  let page = candidates[page_index];
  for (var stream = 0u; stream < 4u; stream += 1u) {
    var range_destination = STREAM_OFFSETS[stream] + page.destinations[stream];
    for (var range = 0u; range < page.range_counts[stream]; range += 1u) {
      let geometry_range = candidate_range(page, stream, range);
      let first_handle = geometry_range.x;
      let count = geometry_range.y;
      for (var element = local.x; element < count; element += 64u) {
        handles[range_destination + element] = first_handle + element;
      }
      range_destination += count;
    }
  }
}

// This is intentionally a separate compute pass from encoding. WebGPU guarantees storage writes
// from the first pass are visible here. Each unique page workgroup compares every exact
// destination/value pair and contributes exactly one completion only after every lane agrees.
@compute @workgroup_size(64)
fn validate_snapshot(
  @builtin(workgroup_id) workgroup: vec3<u32>,
  @builtin(local_invocation_id) local: vec3<u32>,
) {
  let page_index = workgroup.x;
  if page_index >= counters.selected_count
      || page_index >= arrayLength(&candidates)
      || page_index >= arrayLength(&page_tokens) {
    if local.x == 0u {
      atomicOr(&counters.overflow_flags, OVERFLOW_DIRECTORY);
    }
    return;
  }
  if page_tokens[page_index] != 1u {
    if local.x == 0u {
      atomicOr(&counters.overflow_flags, OVERFLOW_STRUCTURE);
    }
    return;
  }

  let page = candidates[page_index];
  var lane_failed = 0u;
  for (var stream = 0u; stream < 4u; stream += 1u) {
    var range_destination = STREAM_OFFSETS[stream] + page.destinations[stream];
    for (var range = 0u; range < page.range_counts[stream]; range += 1u) {
      let geometry_range = candidate_range(page, stream, range);
      let first_handle = geometry_range.x;
      let count = geometry_range.y;
      for (var element = local.x; element < count; element += 64u) {
        if handles[range_destination + element] != first_handle + element {
          lane_failed = 1u;
        }
      }
      range_destination += count;
    }
  }
  lane_failures[local.x] = lane_failed;
  workgroupBarrier();

  if local.x == 0u {
    var page_failed = 0u;
    for (var lane = 0u; lane < 64u; lane += 1u) {
      page_failed |= lane_failures[lane];
    }
    if page_failed != 0u {
      atomicOr(&counters.overflow_flags, HANDLE_MISMATCH);
    } else {
      // Exactly one workgroup is dispatched for each page index, so this is bounded to one
      // uncontended completion operation per successfully validated page.
      atomicAdd(&counters.encoded_pages, 1u);
    }
  }
}
