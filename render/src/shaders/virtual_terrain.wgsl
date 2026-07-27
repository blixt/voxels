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
