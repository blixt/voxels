@group(0) @binding(0) var<uniform> frame: Frame;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
  let x = f32((index << 1u) & 2u);
  let y = f32(index & 2u);
  return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 1.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let uv = position.xy / frame.viewport_voxel.xy;
  let ndc = vec2<f32>(uv.x * 2.0 - 1.0, (1.0 - uv.y) * 2.0 - 1.0);
  // The sky is infinitely distant, so its view ray must depend on camera rotation but never camera
  // translation. Reconstructing a world-space point from the translated inverse view-projection and
  // subtracting the camera again caused visible cancellation jitter while walking, especially far
  // from the world origin. This matches the renderer's fixed 68-degree vertical field of view.
  let camera_forward = normalize(frame.camera_forward.xyz);
  let camera_right = normalize(cross(camera_forward, vec3<f32>(0.0, 1.0, 0.0)));
  let camera_up = normalize(cross(camera_right, camera_forward));
  let vertical_half_fov_tangent = 0.6745085;
  let aspect = frame.viewport_voxel.x / max(frame.viewport_voxel.y, 1.0);
  let ray = normalize(
    camera_forward
      + camera_right * ndc.x * aspect * vertical_half_fov_tangent
      + camera_up * ndc.y * vertical_half_fov_tangent,
  );
  let sun_direction = normalize(frame.sun_direction.xyz);
  let moon_direction = normalize(frame.moon_direction.xyz);
  let key_light_direction = normalize(frame.key_light_direction.xyz);
  let elevation = clamp(ray.y * 0.5 + 0.5, 0.0, 1.0);
  let horizon = pow(1.0 - abs(ray.y), 5.0);
  let rayleigh = pow(max(ray.y, 0.0), 0.42);
  let base = mix(frame.sky_horizon.rgb, frame.sky_zenith.rgb, rayleigh);
  let sun_azimuth = normalize(sun_direction.xz + vec2<f32>(0.0001));
  let ray_azimuth = normalize(ray.xz + vec2<f32>(0.0001));
  let horizon_alignment = pow(max(dot(ray_azimuth, sun_azimuth), 0.0), 3.0);
  let sun_visible = frame.sun_direction.w * smoothstep(-0.025, 0.025, sun_direction.y);
  let moon_visible = frame.moon_direction.w * smoothstep(-0.01, 0.04, moon_direction.y);
  let warm_horizon = vec3<f32>(1.0, 0.34, 0.12)
    * horizon * horizon_alignment * sun_visible * 0.48;
  let sun_amount = max(dot(ray, sun_direction), 0.0);
  let sun_disc = smoothstep(0.99955, 0.99985, sun_amount) * sun_visible;
  let sun_glow = pow(sun_amount, 96.0) * 0.16 + pow(sun_amount, 12.0) * 0.022;
  let moon_amount = max(dot(ray, moon_direction), 0.0);
  let moon_disc = smoothstep(0.99925, 0.99972, moon_amount) * moon_visible;
  let moon_glow = pow(moon_amount, 72.0) * moon_visible * 0.025;
  let below_horizon = mix(frame.ground_atmosphere.rgb, base, smoothstep(0.0, 0.12, elevation));
  var color = below_horizon
    + warm_horizon
    + vec3<f32>(5.8, 4.6, 3.4) * (sun_disc * 1.15 + sun_glow * sun_visible)
    + vec3<f32>(0.42, 0.50, 0.68) * (moon_disc * 0.82 + moon_glow);
  let star_coordinates = ray.xz / max(ray.y + 1.08, 0.08);
  let star_cell = floor(star_coordinates * 420.0);
  let star_seed = atmosphere_hash21(star_cell);
  let star = smoothstep(0.9968, 0.9995, star_seed)
    * mix(0.35, 1.0, atmosphere_hash21(star_cell + vec2<f32>(19.0, 47.0)))
    * smoothstep(0.04, 0.32, ray.y)
    * frame.fog_exposure.w;
  color += mix(vec3<f32>(0.48, 0.62, 1.0), vec3<f32>(1.0, 0.82, 0.58), star_seed) * star * 1.8;
  if ray.y > 0.015 {
    let cloud_height = 480.0;
    let distance_to_layer = (cloud_height - frame.camera_time.y) / ray.y;
    let cloud_world = frame.camera_time.xz + ray.xz * distance_to_layer;
    let field = atmosphere_cloud_field_world(cloud_world, frame.environment_time.yz);
    let coverage_control = clamp(frame.fog_exposure.z, 0.0, 1.0);
    let threshold = mix(0.76, 0.49, coverage_control);
    // Keep this analytic rather than derivative-driven: the cloud layer is only evaluated for
    // upward rays, while WGSL derivatives must execute in uniform control flow.
    let softness = mix(0.052, 0.035, smoothstep(0.02, 0.28, ray.y));
    let coverage = smoothstep(threshold - softness, threshold + softness, field)
      * smoothstep(0.015, 0.12, ray.y)
      * (1.0 - smoothstep(0.94, 0.995, max(sun_amount * sun_visible, moon_amount * moon_visible)));
    let cloud_shadow = mix(frame.sky_horizon.rgb * 0.24, frame.sky_zenith.rgb * 0.62, elevation);
    let cloud_light = cloud_shadow + frame.key_light_radiance.rgb * 0.085;
    color = mix(color, cloud_light, coverage * mix(0.34, 0.78, coverage_control));
  }
  let interface_distance = max(
    (frame.medium.w - frame.camera_time.y) / max(ray.y, 0.05),
    0.0,
  );
  let interface_xz = frame.camera_time.xz + ray.xz * interface_distance;
  let wave = sin(interface_xz.x * 3.1 + frame.camera_time.w * 1.3)
    * sin(interface_xz.y * 2.7 - frame.camera_time.w * 0.9) * 0.025;
  let snell_window = smoothstep(0.61, 0.72, ray.y + wave);
  let path_to_surface = frame.medium.y / max(ray.y, 0.08);
  let water_transmittance = exp(-vec3<f32>(0.42, 0.16, 0.075) * path_to_surface);
  let water_scattering = vec3<f32>(0.010, 0.105, 0.145);
  let window_color = color * water_transmittance
    + water_scattering * (vec3<f32>(1.0) - water_transmittance);
  let overhead_glow = frame.key_light_radiance.rgb
    * pow(max(dot(ray, key_light_direction), 0.0), 18.0)
    * 0.012;
  let underwater_sky = mix(
    water_scattering * mix(0.34, 1.0, max(ray.y, 0.0)) + overhead_glow,
    window_color,
    snell_window,
  );
  color = mix(color, underwater_sky, frame.medium.x);
  return vec4<f32>(max(color * frame.fog_exposure.y * frame.interior.y, vec3<f32>(0.0)), 1.0);
}
