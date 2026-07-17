struct CloudUniform {
  target_size: vec4<f32>,
  layer: vec4<f32>,
  quality: vec4<u32>,
  shaping: vec4<f32>,
};

@group(0) @binding(0) var<uniform> frame: Frame;
@group(1) @binding(0) var<uniform> clouds: CloudUniform;
@group(1) @binding(1) var cloud_noise: texture_3d<f32>;
@group(1) @binding(2) var cloud_noise_sampler: sampler;
@group(2) @binding(0) var cloud_target: texture_2d<f32>;
@group(2) @binding(1) var cloud_target_sampler: sampler;

fn screen_triangle(index: u32, depth: f32) -> vec4<f32> {
  let x = f32((index << 1u) & 2u);
  let y = f32(index & 2u);
  return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, depth, 1.0);
}

@vertex
fn vs_trace(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
  return screen_triangle(index, 0.0);
}

@vertex
fn vs_composite(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
  return screen_triangle(index, 1.0);
}

fn camera_ray(position: vec2<f32>, viewport: vec2<f32>) -> vec3<f32> {
  let uv = position / viewport;
  let ndc = vec2<f32>(uv.x * 2.0 - 1.0, (1.0 - uv.y) * 2.0 - 1.0);
  let forward = normalize(frame.camera_forward.xyz);
  let right = normalize(cross(forward, vec3<f32>(0.0, 1.0, 0.0)));
  let up = normalize(cross(right, forward));
  let vertical_half_fov_tangent = 0.6745085;
  let aspect = viewport.x / max(viewport.y, 1.0);
  return normalize(
    forward
      + right * ndc.x * aspect * vertical_half_fov_tangent
      + up * ndc.y * vertical_half_fov_tangent,
  );
}

fn cloud_height_profile(height: f32, local_growth: f32) -> f32 {
  // Dense weather-field cores grow lower bases and taller crowns while their edges stay lifted
  // and shallow. Using the unthresholded weather field here gives neighboring cloud cells
  // different vertical extents rather than turning every occupied column into the same pillar.
  let storm_growth = mix(local_growth, 0.78, clouds.shaping.w * 0.55);
  let base_start = mix(0.10, 0.0, storm_growth);
  let base_end = base_start + mix(0.14, 0.07, storm_growth);
  let top_fade_start = mix(0.38, 0.82, pow(storm_growth, 0.72));
  return smoothstep(base_start, base_end, height)
    * (1.0 - smoothstep(top_fade_start, 1.0, height));
}

fn cloud_noise_lod(scale: f32, filter_width_metres: f32) -> f32 {
  return clamp(log2(max(filter_width_metres * scale * 64.0, 1.0)), 0.0, 6.0);
}

fn cloud_density_world(world: vec3<f32>, filter_width_metres: f32) -> f32 {
  if world.y <= clouds.layer.x || world.y >= clouds.layer.y {
    return 0.0;
  }
  let height = (world.y - clouds.layer.x) / max(clouds.layer.y - clouds.layer.x, 1.0);
  let macro_field = atmosphere_cloud_field_world(
    world.xz,
    frame.environment_time.yz,
    frame.environment_time.w,
  );
  let coverage = clamp(frame.fog_exposure.z, 0.0, 1.0);
  let macro_threshold = mix(0.84, 0.45, coverage);
  let envelope = smoothstep(macro_threshold - 0.08, macro_threshold + 0.08, macro_field);
  if envelope <= 0.002 {
    return 0.0;
  }
  let seed = vec3<f32>(
    fract(frame.environment_time.w * 0.1031),
    fract(frame.environment_time.w * 0.11369),
    fract(frame.environment_time.w * 0.13787),
  );
  let advected = vec3<f32>(world.x - frame.environment_time.y, world.y, world.z - frame.environment_time.z);
  let base_position = vec3<f32>(advected.x, advected.y * 2.1, advected.z);
  let detail_position = vec3<f32>(advected.z, advected.x, advected.y * 1.45);
  let base_uvw = fract(base_position * clouds.shaping.x + seed);
  let detail_uvw = fract(
    detail_position * clouds.shaping.y + seed.yzx + vec3<f32>(0.37, 0.11, 0.73),
  );
  let shape_lod = cloud_noise_lod(clouds.shaping.x, filter_width_metres);
  let detail_lod = cloud_noise_lod(clouds.shaping.y, filter_width_metres);
  let shape = textureSampleLevel(cloud_noise, cloud_noise_sampler, base_uvw, shape_lod).r;
  let erosion = textureSampleLevel(cloud_noise, cloud_noise_sampler, detail_uvw, detail_lod).r;
  // Deterministic vertical warping interleaves adjacent integration strata. It reuses the two
  // density samples already paid for, so the effect adds arithmetic but no texture fetches.
  let height_warp = (shape - 0.5) * 0.11 + (erosion - 0.5) * 0.05;
  let shaped_height = clamp(height + height_warp, 0.0, 1.0);
  let local_growth = smoothstep(macro_threshold - 0.04, 0.92, macro_field);
  let profile = cloud_height_profile(shaped_height, local_growth);
  if profile <= 0.0 {
    return 0.0;
  }
  let threshold = mix(0.53, 0.39, clouds.shaping.w);
  let billow = shape * 0.72 + envelope * 0.48 - erosion * 0.16;
  // Erode upper cells progressively so tall formations narrow into crowns instead of extruding
  // the same footprint through the whole layer.
  let crown_taper = pow(smoothstep(0.48, 1.0, shaped_height), 2.0) * mix(0.18, 0.07, local_growth);
  let volume = smoothstep(threshold - 0.10, threshold + 0.12, billow - crown_taper);
  return volume * envelope * profile * clouds.shaping.z;
}

fn henyey_greenstein(cos_angle: f32, eccentricity: f32) -> f32 {
  let g2 = eccentricity * eccentricity;
  return (1.0 - g2)
    / max(4.0 * 3.14159265 * pow(1.0 + g2 - 2.0 * eccentricity * cos_angle, 1.5), 0.001);
}

fn light_transmittance(
  world: vec3<f32>,
  light_direction: vec3<f32>,
  view_filter_width_metres: f32,
) -> f32 {
  let vertical_distance = max(clouds.layer.y - world.y, 0.0);
  let distance_to_top = vertical_distance / max(abs(light_direction.y), 0.14);
  let step_length = distance_to_top / f32(max(clouds.quality.y, 1u));
  var optical_depth = 0.0;
  let filter_width_metres = max(view_filter_width_metres, step_length * 0.35);
  for (var index = 0u; index < 4u; index += 1u) {
    if index >= clouds.quality.y {
      break;
    }
    let sample_world = world + light_direction * step_length * (f32(index) + 0.65);
    optical_depth += cloud_density_world(sample_world, filter_width_metres) * step_length;
  }
  let direct = exp(-optical_depth * clouds.layer.w);
  // Approximate higher-order scattering without flattening every cloud interior to the same
  // brightness. The floor keeps storm cores readable while retaining enough contrast for volume.
  return max(direct, 0.52 * exp(-optical_depth * clouds.layer.w * 0.20));
}

@fragment
fn fs_trace(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  if clouds.quality.z == 0u {
    return vec4<f32>(0.0);
  }
  let ray = camera_ray(position.xy, clouds.target_size.xy);
  let camera = frame.camera_time.xyz;
  if abs(ray.y) < 0.001 {
    return vec4<f32>(0.0);
  }
  let lower = (clouds.layer.x - camera.y) / ray.y;
  let upper = (clouds.layer.y - camera.y) / ray.y;
  var trace_start = max(min(lower, upper), 0.0);
  var trace_end = min(max(lower, upper), clouds.layer.z);
  if trace_end <= trace_start {
    return vec4<f32>(0.0);
  }
  let trace_length = trace_end - trace_start;
  // Long grazing rays expose the individual integration strata most strongly. Spend a bounded
  // six extra samples only there; overhead rays retain the configured cost.
  let grazing_quality = 1.0 - smoothstep(0.10, 0.38, abs(ray.y));
  let extra_capacity = min(6u, 24u - min(clouds.quality.x, 24u));
  let view_steps = clouds.quality.x + u32(round(grazing_quality * f32(extra_capacity)));
  let step_length = trace_length / f32(max(view_steps, 1u));
  let entry_world = camera + ray * trace_start;
  let stable_sample_phase = mix(
    0.15,
    0.85,
    atmosphere_value_noise(
      entry_world.xz * 0.12
        + vec2<f32>(
          fract(frame.environment_time.w * 0.173) * 4096.0,
          fract(frame.environment_time.w * 0.197) * 4096.0,
        ),
    ),
  );
  let light_direction = normalize(frame.key_light_direction.xyz);
  let cos_angle = dot(ray, light_direction);
  let phase = henyey_greenstein(cos_angle, 0.62) * 0.82
    + henyey_greenstein(cos_angle, -0.23) * 0.18;
  let ambient = (
    mix(frame.sky_horizon.rgb, frame.sky_zenith.rgb, 0.62) * 0.72
      + frame.ground_atmosphere.rgb * 0.18
  ) * mix(0.86, 0.42, clouds.shaping.w);
  var transmittance = 1.0;
  var radiance = vec3<f32>(0.0);
  var previous_density = 0.0;
  for (var index = 0u; index < 24u; index += 1u) {
    if index >= view_steps || transmittance < 0.02 {
      break;
    }
    // A stable low-discrepancy rotation keeps every sample inside its own stratum while preventing
    // the aligned planes that read as stacked, flat cloud layers at grazing view angles.
    let stratified_sample_phase = mix(
      0.12,
      0.88,
      fract(stable_sample_phase + f32(index) * 0.61803398875),
    );
    let distance = trace_start + (f32(index) + stratified_sample_phase) * step_length;
    if distance >= trace_end {
      break;
    }
    let world = camera + ray * distance;
    let pixel_footprint_metres = distance * 1.349017 / max(clouds.target_size.y, 1.0);
    let filter_width_metres = max(pixel_footprint_metres, step_length * 0.35);
    let sampled_density = cloud_density_world(world, filter_width_metres);
    // Reconstruct a short linear segment between adjacent strata instead of treating every point
    // sample as a constant-density slab. This mixes the visible layers without extra noise reads.
    let density = mix(sampled_density, previous_density, 0.32);
    previous_density = sampled_density;
    if density <= 0.001 {
      continue;
    }
    let light_visibility = light_transmittance(world, light_direction, filter_width_metres);
    let height = clamp((world.y - clouds.layer.x) / max(clouds.layer.y - clouds.layer.x, 1.0), 0.0, 1.0);
    let base_darkening = mix(0.46, 1.0, smoothstep(0.0, 0.56, height));
    let direct = frame.key_light_radiance.rgb * phase * light_visibility * base_darkening;
    let optical_depth = density * step_length * clouds.layer.w;
    let sample_alpha = 1.0 - exp(-optical_depth);
    let ambient_visibility = mix(0.48, 1.0, sqrt(light_visibility));
    let powder = 1.0 - exp(-optical_depth * 2.4);
    let edge_to_core = mix(1.10, 0.86, powder);
    let scattering = (ambient * ambient_visibility + direct) * edge_to_core;
    radiance += transmittance * scattering * sample_alpha;
    transmittance *= 1.0 - sample_alpha;
  }
  // Fade from the distance where the ray first enters the cloud layer. Using the slab exit
  // distance creates a visible horizontal shelf whenever the top of the layer crosses the march
  // budget; entry distance instead dissolves the deck continuously into aerial perspective.
  let horizon_fade = 1.0 - smoothstep(clouds.layer.z * 0.40, clouds.layer.z * 0.96, trace_start);
  let alpha = (1.0 - transmittance) * horizon_fade;
  return vec4<f32>(radiance * horizon_fade, alpha);
}

@fragment
fn fs_composite(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let uv = position.xy / frame.viewport_voxel.xy;
  let cloud = textureSampleLevel(cloud_target, cloud_target_sampler, uv, 0.0);
  // Preserve premultiplied radiance while tightening only the softest part of half-resolution
  // coverage. This removes the oversized blurred base cells without inventing screen-space noise.
  let reconstructed_alpha = smoothstep(0.035, 0.965, cloud.a);
  let radiance_scale = reconstructed_alpha / max(cloud.a, 0.0001);
  return vec4<f32>(cloud.rgb * radiance_scale, reconstructed_alpha) * (1.0 - frame.medium.x);
}
