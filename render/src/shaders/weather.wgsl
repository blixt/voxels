@group(0) @binding(0) var<uniform> frame: Frame;

const GRID_SIDE: u32 = 48u;
const GRID_CELLS: u32 = GRID_SIDE * GRID_SIDE;
const PRECIPITATION_RADIUS_METRES: f32 = 36.0;
const PRECIPITATION_HEIGHT_METRES: f32 = 32.0;

struct VertexOutput {
  @builtin(position) position: vec4<f32>,
  @location(0) local: vec2<f32>,
  @location(1) tint_alpha: vec4<f32>,
  @location(2) @interpolate(flat) snow: u32,
  @location(3) @interpolate(flat) flake_phase: f32,
};

fn hash_cell(cell: vec2<f32>, lane: f32, salt: f32) -> f32 {
  return atmosphere_hash21(
    cell
      + vec2<f32>(
        lane * 71.0 + salt * 19.0 + frame.environment_time.w * 0.013,
        lane * 137.0 - salt * 43.0 + frame.environment_time.w * 0.017,
      ),
  );
}

fn quad_coordinates(vertex_index: u32) -> vec2<f32> {
  switch vertex_index {
    case 0u: {
      return vec2<f32>(-1.0, 0.0);
    }
    case 1u: {
      return vec2<f32>(1.0, 0.0);
    }
    case 2u: {
      return vec2<f32>(1.0, 1.0);
    }
    case 3u: {
      return vec2<f32>(-1.0, 0.0);
    }
    case 4u: {
      return vec2<f32>(1.0, 1.0);
    }
    default: {
      return vec2<f32>(-1.0, 1.0);
    }
  }
}

fn hidden_vertex() -> VertexOutput {
  var output: VertexOutput;
  output.position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
  output.local = vec2<f32>(0.0);
  output.tint_alpha = vec4<f32>(0.0);
  output.snow = 0u;
  output.flake_phase = 0.0;
  return output;
}

@vertex
fn vs_main(
  @builtin(vertex_index) vertex_index: u32,
  @builtin(instance_index) instance_index: u32,
) -> VertexOutput {
  let precipitation = frame.weather.x * (1.0 - smoothstep(0.20, 0.72, frame.interior.x));
  let lane = instance_index / GRID_CELLS;
  let cell_index = instance_index % GRID_CELLS;
  let cell_offset = vec2<i32>(
    i32(cell_index % GRID_SIDE) - i32(GRID_SIDE / 2u),
    i32(cell_index / GRID_SIDE) - i32(GRID_SIDE / 2u),
  );
  let cell_size = PRECIPITATION_RADIUS_METRES * 2.0 / f32(GRID_SIDE);
  let camera_cell = floor(frame.camera_time.xz / cell_size);
  let absolute_cell = camera_cell + vec2<f32>(cell_offset);
  let activation = hash_cell(absolute_cell, f32(lane), 0.0);
  if precipitation <= 0.002 || activation > precipitation {
    return hidden_vertex();
  }

  let random_x = hash_cell(absolute_cell, f32(lane), 1.0);
  let random_z = hash_cell(absolute_cell, f32(lane), 2.0);
  let random_speed = hash_cell(absolute_cell, f32(lane), 3.0);
  let random_shape = hash_cell(absolute_cell, f32(lane), 4.0);
  let snow = u32(hash_cell(absolute_cell, f32(lane), 5.0) < frame.weather.w);
  let wind = frame.cloud_layer.zw;
  let fall_speed = select(mix(9.0, 14.0, random_speed), mix(1.1, 3.0, random_speed), snow == 1u);
  let fall_duration = PRECIPITATION_HEIGHT_METRES / fall_speed;
  let age = fract(
    hash_cell(absolute_cell, f32(lane), 6.0)
      + frame.camera_time.w / fall_duration,
  );
  let age_seconds = age * fall_duration;
  var world = vec3<f32>(
    (absolute_cell.x + 0.10 + random_x * 0.80) * cell_size,
    frame.camera_time.y + 17.0 - age * PRECIPITATION_HEIGHT_METRES,
    (absolute_cell.y + 0.10 + random_z * 0.80) * cell_size,
  );
  if snow == 1u {
    let flutter_phase = random_shape * 6.2831853 + frame.camera_time.w * mix(1.1, 2.4, random_x);
    world.xz += wind * age_seconds * 0.34
      + vec2<f32>(sin(flutter_phase), cos(flutter_phase * 0.83)) * mix(0.18, 0.52, random_z);
  } else {
    world.xz += wind * age_seconds * 0.52;
  }

  let radial_distance = length(world.xz - frame.camera_time.xz);
  let radial_fade = 1.0 - smoothstep(PRECIPITATION_RADIUS_METRES * 0.76, PRECIPITATION_RADIUS_METRES, radial_distance);
  let wrap_fade = smoothstep(0.0, 0.07, age) * (1.0 - smoothstep(0.90, 1.0, age));
  let view_depth = dot(world - frame.camera_time.xyz, frame.camera_forward.xyz);
  let depth_fade = smoothstep(1.5, 4.0, view_depth)
    * (1.0 - smoothstep(PRECIPITATION_RADIUS_METRES * 0.72, PRECIPITATION_RADIUS_METRES, view_depth));
  let alpha = precipitation * radial_fade * wrap_fade * depth_fade;
  if alpha <= 0.001 {
    return hidden_vertex();
  }

  let coordinates = quad_coordinates(vertex_index);
  let head_clip = frame.view_projection * vec4<f32>(world, 1.0);
  if head_clip.w <= 0.01 {
    return hidden_vertex();
  }

  var output: VertexOutput;
  output.snow = snow;
  output.flake_phase = random_shape * 6.2831853 + frame.camera_time.w * 1.7;
  if snow == 1u {
    let head_ndc = head_clip.xy / head_clip.w;
    let focal_pixels = frame.viewport_voxel.y / 1.349017;
    let physical_size = mix(0.045, 0.13, random_shape);
    let radius_pixels = clamp(physical_size * focal_pixels / max(view_depth, 0.1), 0.55, 5.0);
    let local = vec2<f32>(coordinates.x, coordinates.y * 2.0 - 1.0);
    let angle = output.flake_phase;
    let rotated = vec2<f32>(
      local.x * cos(angle) - local.y * sin(angle),
      local.x * sin(angle) + local.y * cos(angle),
    );
    let ndc_offset = rotated * radius_pixels * 2.0 / frame.viewport_voxel.xy;
    output.position = vec4<f32>(
      (head_ndc + ndc_offset) * head_clip.w,
      head_clip.z,
      head_clip.w,
    );
    output.local = local;
    let snow_color = mix(vec3<f32>(0.72, 0.80, 0.90), vec3<f32>(0.94, 0.97, 1.0), random_x);
    output.tint_alpha = vec4<f32>(snow_color, alpha * mix(0.34, 0.72, frame.weather.y));
    return output;
  }

  let velocity = vec3<f32>(wind.x * 0.52, -fall_speed, wind.y * 0.52);
  let streak_length = mix(0.34, 0.92, random_shape) * mix(0.78, 1.16, frame.weather.y);
  let tail = world - normalize(velocity) * streak_length;
  let tail_clip = frame.view_projection * vec4<f32>(tail, 1.0);
  if tail_clip.w <= 0.01 {
    return hidden_vertex();
  }
  let head_ndc = head_clip.xy / head_clip.w;
  let tail_ndc = tail_clip.xy / tail_clip.w;
  let segment_pixels = (head_ndc - tail_ndc) * frame.viewport_voxel.xy * 0.5;
  let perpendicular = normalize(vec2<f32>(-segment_pixels.y, segment_pixels.x) + vec2<f32>(0.0001, 0.0));
  let width_pixels = clamp(mix(0.42, 1.18, random_x) * 7.0 / sqrt(max(view_depth, 1.0)), 0.42, 1.35);
  let endpoint_clip = mix(tail_clip, head_clip, coordinates.y);
  let ndc_offset = perpendicular * coordinates.x * width_pixels * 2.0 / frame.viewport_voxel.xy;
  output.position = vec4<f32>(
    endpoint_clip.xy + ndc_offset * endpoint_clip.w,
    endpoint_clip.z,
    endpoint_clip.w,
  );
  output.local = coordinates;
  let rain_color = mix(
    mix(vec3<f32>(0.30, 0.42, 0.58), frame.sky_horizon.rgb, 0.32),
    vec3<f32>(0.76, 0.86, 0.98),
    frame.weather.y,
  );
  output.tint_alpha = vec4<f32>(rain_color, alpha * mix(0.20, 0.46, frame.weather.y));
  return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
  var coverage: f32;
  if input.snow == 1u {
    let radius = length(input.local);
    let angle = atan2(input.local.y, input.local.x);
    let irregular_edge = 0.78 + cos(angle * 6.0 + input.flake_phase) * 0.10;
    coverage = 1.0 - smoothstep(irregular_edge - 0.18, irregular_edge, radius);
    coverage *= mix(0.72, 1.0, 1.0 - radius);
  } else {
    let cross_section = 1.0 - smoothstep(0.22, 1.0, abs(input.local.x));
    let tail_fade = smoothstep(0.0, 0.12, input.local.y);
    let head_fade = 1.0 - smoothstep(0.90, 1.0, input.local.y);
    let head_glint = mix(0.72, 1.0, smoothstep(0.58, 0.96, input.local.y));
    coverage = cross_section * tail_fade * head_fade * head_glint;
  }
  let alpha = input.tint_alpha.a * coverage;
  return vec4<f32>(input.tint_alpha.rgb * alpha, alpha);
}
