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

fn cloud_height_profile(world_y: f32) -> f32 {
  let height = clamp(
    (world_y - clouds.layer.x) / max(clouds.layer.y - clouds.layer.x, 1.0),
    0.0,
    1.0,
  );
  let top_fade_start = mix(0.62, 0.82, clouds.shaping.w);
  return smoothstep(0.0, 0.11, height) * (1.0 - smoothstep(top_fade_start, 1.0, height));
}

fn cloud_noise_lod(scale: f32, filter_width_metres: f32) -> f32 {
  return clamp(log2(max(filter_width_metres * scale * 64.0, 1.0)), 0.0, 6.0);
}

fn cloud_density_world(world: vec3<f32>, filter_width_metres: f32) -> f32 {
  let profile = cloud_height_profile(world.y);
  if profile <= 0.0 {
    return 0.0;
  }
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
  let base_uvw = fract(advected * clouds.shaping.x + seed);
  let detail_uvw = fract(advected.zxy * clouds.shaping.y + seed.yzx + vec3<f32>(0.37, 0.11, 0.73));
  let shape_lod = cloud_noise_lod(clouds.shaping.x, filter_width_metres);
  let detail_lod = cloud_noise_lod(clouds.shaping.y, filter_width_metres);
  let shape = textureSampleLevel(cloud_noise, cloud_noise_sampler, base_uvw, shape_lod).r;
  let erosion = textureSampleLevel(cloud_noise, cloud_noise_sampler, detail_uvw, detail_lod).r;
  let threshold = mix(0.53, 0.39, clouds.shaping.w);
  let billow = shape * 0.72 + envelope * 0.48 - erosion * 0.16;
  let volume = smoothstep(threshold - 0.10, threshold + 0.12, billow);
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
  return max(direct, 0.70 * exp(-optical_depth * clouds.layer.w * 0.25));
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
  let step_length = trace_length / f32(max(clouds.quality.x, 1u));
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
  for (var index = 0u; index < 24u; index += 1u) {
    if index >= clouds.quality.x || transmittance < 0.02 {
      break;
    }
    let distance = trace_start + (f32(index) + stable_sample_phase) * step_length;
    if distance >= trace_end {
      break;
    }
    let world = camera + ray * distance;
    let pixel_footprint_metres = distance * 1.349017 / max(clouds.target_size.y, 1.0);
    let filter_width_metres = max(pixel_footprint_metres, step_length * 0.35);
    let density = cloud_density_world(world, filter_width_metres);
    if density <= 0.001 {
      continue;
    }
    let light_visibility = light_transmittance(world, light_direction, filter_width_metres);
    let height = clamp((world.y - clouds.layer.x) / max(clouds.layer.y - clouds.layer.x, 1.0), 0.0, 1.0);
    let base_darkening = mix(0.46, 1.0, smoothstep(0.0, 0.56, height));
    let direct = frame.key_light_radiance.rgb * phase * light_visibility * base_darkening;
    let scattering = ambient + direct;
    let optical_depth = density * step_length * clouds.layer.w;
    let sample_alpha = 1.0 - exp(-optical_depth);
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
  return cloud * (1.0 - frame.medium.x);
}
