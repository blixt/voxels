struct VirtualTerrainNode {
  minimum_level: vec4<i32>,
  maximum_flags: vec4<i32>,
  errors: vec4<u32>,
  children_low: vec4<u32>,
  children_high: vec4<u32>,
};

struct VirtualTerrainView {
  camera_near: vec4<f32>,
  forward_far: vec4<f32>,
  right_tangent_horizontal: vec4<f32>,
  up_tangent_vertical: vec4<f32>,
  projection_thresholds: vec4<f32>,
  counts_flags: vec4<u32>,
  options: vec4<u32>,
};

struct TraversalCounters {
  selected_count: atomic<u32>,
  request_count: atomic<u32>,
  ownerless_roots: atomic<u32>,
  visited_nodes: atomic<u32>,
  overflow_flags: atomic<u32>,
  stack_peak: atomic<u32>,
  reserved_0: u32,
  reserved_1: u32,
};

@group(0) @binding(0) var<uniform> view: VirtualTerrainView;
@group(0) @binding(1) var<storage, read> nodes: array<VirtualTerrainNode>;
@group(0) @binding(2) var<storage, read> roots: array<u32>;
@group(0) @binding(3) var<storage, read_write> counters: TraversalCounters;
@group(0) @binding(4) var<storage, read_write> selected_pages: array<u32>;
@group(0) @binding(5) var<storage, read_write> requested_pages: array<u32>;

const NODE_HAS_CHILDREN: u32 = 1u;
const NODE_IS_ROOT: u32 = 2u;
const NODE_RESIDENT: u32 = 4u;
const NODE_REPLACEMENT_COHERENT: u32 = 8u;
const NODE_PRIOR_REFINED: u32 = 16u;
const OVERFLOW_SELECTION: u32 = 1u;
const OVERFLOW_FEEDBACK: u32 = 2u;
const OVERFLOW_TRAVERSAL: u32 = 4u;
const OVERFLOW_STACK: u32 = 8u;
const STACK_CAPACITY: u32 = 192u;

fn node_flags(node: VirtualTerrainNode) -> u32 {
  return bitcast<u32>(node.maximum_flags.w);
}

fn child_index(node: VirtualTerrainNode, child: u32) -> u32 {
  if child < 4u {
    return node.children_low[child];
  }
  return node.children_high[child - 4u];
}

fn page_bounds_metres(node: VirtualTerrainNode) -> mat2x3<f32> {
  return mat2x3<f32>(
    vec3<f32>(node.minimum_level.xyz) * 0.1,
    vec3<f32>(node.maximum_flags.xyz) * 0.1,
  );
}

fn root_visible(node: VirtualTerrainNode) -> bool {
  let bounds = page_bounds_metres(node);
  let center = (bounds[0] + bounds[1]) * 0.5;
  let radius = length(bounds[1] - center);
  let relative = center - view.camera_near.xyz;
  let depth = dot(relative, view.forward_far.xyz);
  if depth + radius < view.camera_near.w || depth - radius > view.forward_far.w {
    return false;
  }
  let tangent_horizontal = view.right_tangent_horizontal.w;
  let tangent_vertical = view.up_tangent_vertical.w;
  let horizontal_radius = radius * sqrt(1.0 + tangent_horizontal * tangent_horizontal);
  let vertical_radius = radius * sqrt(1.0 + tangent_vertical * tangent_vertical);
  return abs(dot(relative, view.right_tangent_horizontal.xyz))
      <= depth * tangent_horizontal + horizontal_radius
    && abs(dot(relative, view.up_tangent_vertical.xyz))
      <= depth * tangent_vertical + vertical_radius;
}

fn distance_to_page(node: VirtualTerrainNode) -> f32 {
  let bounds = page_bounds_metres(node);
  let point = view.camera_near.xyz;
  let delta = max(max(bounds[0] - point, point - bounds[1]), vec3<f32>(0.0));
  return max(length(delta), view.camera_near.w);
}

fn projected_error_pixels(node: VirtualTerrainNode) -> f32 {
  if (node.errors.w & 0x100u) != 0u {
    return 3.402823466e+38;
  }
  let positional_metres = f32(node.errors.x) * 0.0001;
  let positional_pixels =
    positional_metres * view.projection_thresholds.x / distance_to_page(node);
  let normal_pixels = f32(node.errors.y)
    * 0.001
    * 0.25
    * view.projection_thresholds.w;
  return max(positional_pixels, normal_pixels);
}

fn append_selected(node_index: u32) {
  let destination = atomicAdd(&counters.selected_count, 1u);
  if destination < view.counts_flags.y {
    selected_pages[destination] = node_index;
  } else {
    atomicOr(&counters.overflow_flags, OVERFLOW_SELECTION);
  }
}

fn append_request(node_index: u32) {
  let destination = atomicAdd(&counters.request_count, 1u);
  if destination < view.counts_flags.z {
    requested_pages[destination] = node_index;
  } else {
    atomicOr(&counters.overflow_flags, OVERFLOW_FEEDBACK);
  }
}

@compute @workgroup_size(64)
fn traverse(@builtin(global_invocation_id) invocation: vec3<u32>) {
  if invocation.x >= view.counts_flags.x {
    return;
  }
  let root_index = roots[invocation.x];
  if root_index >= view.options.z {
    atomicOr(&counters.overflow_flags, OVERFLOW_TRAVERSAL);
    atomicAdd(&counters.ownerless_roots, 1u);
    return;
  }
  if !root_visible(nodes[root_index]) {
    return;
  }

  var stack: array<u32, 192>;
  var stack_count = 1u;
  stack[0] = root_index;
  atomicMax(&counters.stack_peak, stack_count);
  loop {
    if stack_count == 0u {
      break;
    }
    stack_count -= 1u;
    let node_index = stack[stack_count];
    let visited = atomicAdd(&counters.visited_nodes, 1u);
    if visited >= view.counts_flags.w || node_index >= view.options.z {
      atomicOr(&counters.overflow_flags, OVERFLOW_TRAVERSAL);
      if node_index < view.options.z && (node_flags(nodes[node_index]) & NODE_RESIDENT) != 0u {
        append_selected(node_index);
      } else if node_index == root_index {
        atomicAdd(&counters.ownerless_roots, 1u);
      }
      continue;
    }
    let node = nodes[node_index];
    let flags = node_flags(node);
    if (flags & NODE_RESIDENT) == 0u {
      append_request(node_index);
      if (flags & NODE_IS_ROOT) != 0u {
        atomicAdd(&counters.ownerless_roots, 1u);
      }
      continue;
    }
    let force_exact = view.options.x != 0u;
    let threshold = select(
      view.projection_thresholds.y,
      view.projection_thresholds.z,
      (flags & NODE_PRIOR_REFINED) != 0u,
    );
    let wants_refinement = (flags & NODE_HAS_CHILDREN) != 0u
      && (force_exact || projected_error_pixels(node) > threshold);
    if wants_refinement && (flags & NODE_REPLACEMENT_COHERENT) != 0u {
      if stack_count + 8u <= STACK_CAPACITY {
        for (var child = 0u; child < 8u; child += 1u) {
          stack[stack_count + child] = child_index(node, 7u - child);
        }
        stack_count += 8u;
        atomicMax(&counters.stack_peak, stack_count);
        continue;
      }
      atomicOr(&counters.overflow_flags, OVERFLOW_STACK);
    } else if wants_refinement {
      for (var child = 0u; child < 8u; child += 1u) {
        let child_node = child_index(node, child);
        if child_node < view.options.z
            && (node_flags(nodes[child_node]) & NODE_RESIDENT) == 0u {
          append_request(child_node);
        }
      }
    }
    append_selected(node_index);
  }
}

struct GeometryPage {
  opaque_surface_offset: u32,
  opaque_surface_count: u32,
  opaque_triangle_offset: u32,
  opaque_triangle_count: u32,
  water_surface_offset: u32,
  water_surface_count: u32,
  water_triangle_offset: u32,
  water_triangle_count: u32,
};

struct CompactionCounters {
  surface_elements: atomic<u32>,
  triangle_elements: atomic<u32>,
  water_surface_elements: atomic<u32>,
  water_triangle_elements: atomic<u32>,
  copied_pages: atomic<u32>,
  overflow_flags: atomic<u32>,
  surface_capacity: u32,
  triangle_capacity: u32,
  water_surface_capacity: u32,
  water_triangle_capacity: u32,
  surface_water_word_offset: u32,
  triangle_water_word_offset: u32,
};

@group(1) @binding(0) var<storage, read> compact_selected_pages: array<u32>;
@group(1) @binding(1) var<storage, read_write> compact_traversal: TraversalCounters;
@group(1) @binding(2) var<storage, read> geometry_pages: array<GeometryPage>;
@group(1) @binding(3) var<storage, read> geometry_source: array<u32>;
@group(1) @binding(4) var<storage, read_write> compact_surfaces: array<u32>;
@group(1) @binding(5) var<storage, read_write> compact_triangles: array<u32>;
@group(1) @binding(6) var<storage, read_write> compaction: CompactionCounters;
@group(1) @binding(7) var<storage, read_write> indirect_commands: array<u32>;

const GEOMETRY_WORDS_PER_ELEMENT: u32 = 6u;
const COMPACTION_OVERFLOW_SURFACE: u32 = 1u;
const COMPACTION_OVERFLOW_TRIANGLE: u32 = 2u;
const COMPACTION_OVERFLOW_WATER_SURFACE: u32 = 4u;
const COMPACTION_OVERFLOW_WATER_TRIANGLE: u32 = 8u;
const COMPACTION_OVERFLOW_DIRECTORY: u32 = 16u;
const COMPACTION_OVERFLOW_SOURCE: u32 = 32u;

var<workgroup> compact_destinations: array<u32, 4>;
var<workgroup> compact_enabled: array<u32, 4>;

@compute @workgroup_size(1)
fn prepare_compaction() {
  indirect_commands[0] = min(
    atomicLoad(&compact_traversal.selected_count),
    arrayLength(&compact_selected_pages),
  );
  indirect_commands[1] = 1u;
  indirect_commands[2] = 1u;
  indirect_commands[3] = 0u;
}

@compute @workgroup_size(64)
fn compact_selected(
  @builtin(workgroup_id) workgroup: vec3<u32>,
  @builtin(local_invocation_id) local: vec3<u32>,
) {
  let selected_count = min(
    atomicLoad(&compact_traversal.selected_count),
    arrayLength(&compact_selected_pages),
  );
  if workgroup.x >= selected_count {
    return;
  }
  let page_index = compact_selected_pages[workgroup.x];
  if page_index >= arrayLength(&geometry_pages) {
    if local.x == 0u {
      atomicOr(&compaction.overflow_flags, COMPACTION_OVERFLOW_DIRECTORY);
    }
    return;
  }
  let page = geometry_pages[page_index];

  if local.x == 0u {
    compact_enabled = array<u32, 4>(0u, 0u, 0u, 0u);
    let offsets = array<u32, 4>(
      page.opaque_surface_offset,
      page.opaque_triangle_offset,
      page.water_surface_offset,
      page.water_triangle_offset,
    );
    let counts = array<u32, 4>(
      page.opaque_surface_count,
      page.opaque_triangle_count,
      page.water_surface_count,
      page.water_triangle_count,
    );
    var source_valid = true;
    for (var stream = 0u; stream < 4u; stream += 1u) {
      let source_words = counts[stream] * GEOMETRY_WORDS_PER_ELEMENT;
      if counts[stream] > 0xffffffffu / GEOMETRY_WORDS_PER_ELEMENT
          || offsets[stream] > arrayLength(&geometry_source)
          || source_words > arrayLength(&geometry_source) - offsets[stream] {
        source_valid = false;
      }
    }
    if !source_valid {
      atomicOr(&compaction.overflow_flags, COMPACTION_OVERFLOW_SOURCE);
    } else {
      compact_destinations[0] =
        atomicAdd(&compaction.surface_elements, page.opaque_surface_count);
      compact_destinations[1] =
        atomicAdd(&compaction.triangle_elements, page.opaque_triangle_count);
      compact_destinations[2] =
        atomicAdd(&compaction.water_surface_elements, page.water_surface_count);
      compact_destinations[3] =
        atomicAdd(&compaction.water_triangle_elements, page.water_triangle_count);
      let capacities = array<u32, 4>(
        compaction.surface_capacity,
        compaction.triangle_capacity,
        compaction.water_surface_capacity,
        compaction.water_triangle_capacity,
      );
      let overflow_flags = array<u32, 4>(
        COMPACTION_OVERFLOW_SURFACE,
        COMPACTION_OVERFLOW_TRIANGLE,
        COMPACTION_OVERFLOW_WATER_SURFACE,
        COMPACTION_OVERFLOW_WATER_TRIANGLE,
      );
      var page_valid = true;
      for (var stream = 0u; stream < 4u; stream += 1u) {
        let destination = compact_destinations[stream];
        if destination <= capacities[stream]
            && counts[stream] <= capacities[stream] - destination {
          compact_enabled[stream] = 1u;
        } else {
          page_valid = false;
          atomicOr(&compaction.overflow_flags, overflow_flags[stream]);
        }
      }
      if page_valid {
        atomicAdd(&compaction.copied_pages, 1u);
      }
    }
  }
  workgroupBarrier();
  for (var element = local.x; element < page.opaque_surface_count; element += 64u) {
    let source = page.opaque_surface_offset + element * GEOMETRY_WORDS_PER_ELEMENT;
    let destination = (compact_destinations[0] + element) * GEOMETRY_WORDS_PER_ELEMENT;
    for (var word = 0u; word < GEOMETRY_WORDS_PER_ELEMENT; word += 1u) {
      if compact_enabled[0] != 0u {
        compact_surfaces[destination + word] = geometry_source[source + word];
      }
    }
  }
  for (var element = local.x; element < page.opaque_triangle_count; element += 64u) {
    let source = page.opaque_triangle_offset + element * GEOMETRY_WORDS_PER_ELEMENT;
    let destination = (compact_destinations[1] + element) * GEOMETRY_WORDS_PER_ELEMENT;
    for (var word = 0u; word < GEOMETRY_WORDS_PER_ELEMENT; word += 1u) {
      if compact_enabled[1] != 0u {
        compact_triangles[destination + word] = geometry_source[source + word];
      }
    }
  }
  for (var element = local.x; element < page.water_surface_count; element += 64u) {
    let source = page.water_surface_offset + element * GEOMETRY_WORDS_PER_ELEMENT;
    let destination =
      compaction.surface_water_word_offset
      + (compact_destinations[2] + element) * GEOMETRY_WORDS_PER_ELEMENT;
    for (var word = 0u; word < GEOMETRY_WORDS_PER_ELEMENT; word += 1u) {
      if compact_enabled[2] != 0u {
        compact_surfaces[destination + word] = geometry_source[source + word];
      }
    }
  }
  for (var element = local.x; element < page.water_triangle_count; element += 64u) {
    let source = page.water_triangle_offset + element * GEOMETRY_WORDS_PER_ELEMENT;
    let destination =
      compaction.triangle_water_word_offset
      + (compact_destinations[3] + element) * GEOMETRY_WORDS_PER_ELEMENT;
    for (var word = 0u; word < GEOMETRY_WORDS_PER_ELEMENT; word += 1u) {
      if compact_enabled[3] != 0u {
        compact_triangles[destination + word] = geometry_source[source + word];
      }
    }
  }
}

@compute @workgroup_size(1)
fn finalize_compaction() {
  let overflow = atomicLoad(&compaction.overflow_flags);
  let surface_count = min(
    atomicLoad(&compaction.surface_elements),
    compaction.surface_capacity,
  );
  let triangle_count = min(
    atomicLoad(&compaction.triangle_elements),
    compaction.triangle_capacity,
  );
  let water_surface_count = min(
    atomicLoad(&compaction.water_surface_elements),
    compaction.water_surface_capacity,
  );
  let water_triangle_count = min(
    atomicLoad(&compaction.water_triangle_elements),
    compaction.water_triangle_capacity,
  );

  indirect_commands[4] = 4u;
  indirect_commands[5] = select(surface_count, 0u, overflow != 0u);
  indirect_commands[6] = 0u;
  indirect_commands[7] = 0u;
  indirect_commands[8] = select(triangle_count, 0u, overflow != 0u);
  indirect_commands[9] = 1u;
  indirect_commands[10] = 0u;
  indirect_commands[11] = 0u;
  indirect_commands[12] = 4u;
  indirect_commands[13] = select(water_surface_count, 0u, overflow != 0u);
  indirect_commands[14] = 0u;
  indirect_commands[15] = 0u;
  indirect_commands[16] = select(water_triangle_count, 0u, overflow != 0u);
  indirect_commands[17] = 1u;
  indirect_commands[18] = 0u;
  indirect_commands[19] = 0u;
}
