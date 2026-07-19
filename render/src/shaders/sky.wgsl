@group(0) @binding(0) var<uniform> frame: Frame;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
  let x = f32((index << 1u) & 2u);
  let y = f32(index & 2u);
  return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 1.0, 1.0);
}

fn moon_surface_radiance(
  ray: vec3<f32>,
  moon_direction: vec3<f32>,
  sun_direction: vec3<f32>,
  visibility: f32,
) -> vec3<f32> {
  // The equatorial north pole anchors lunar texture to the synchronized celestial sphere. It
  // remains stable under camera rotation and transports continuously across either world pole.
  let orbital_normal = normalize(vec3<f32>(
    frame.equatorial_east.z,
    frame.equatorial_up.z,
    -frame.equatorial_north.z,
  ));
  let moon_right = normalize(cross(orbital_normal, moon_direction));
  let moon_up = normalize(cross(moon_direction, moon_right));
  let apparent_radius = 0.01414;
  let local = vec2<f32>(dot(ray, moon_right), dot(ray, moon_up)) / apparent_radius;
  let radius_squared = dot(local, local);
  let coverage = (1.0 - smoothstep(0.94, 1.04, radius_squared)) * visibility;

  // Eight apparent cells span only about two pixels each at 720p. Quantizing the normal at the
  // cell centre creates a cheap voxel-facet cue while the actual silhouette remains round.
  let cell_count = 8.0;
  let texture_uv = local * 0.5 + 0.5;
  let cell = floor(texture_uv * cell_count);
  var cell_local = ((cell + 0.5) / cell_count) * 2.0 - 1.0;
  let cell_radius_squared = dot(cell_local, cell_local);
  cell_local *= min(1.0, 0.98 / sqrt(max(cell_radius_squared, 0.0001)));
  let cell_depth = sqrt(max(1.0 - dot(cell_local, cell_local), 0.001));
  let facet_normal = normalize(
    moon_right * cell_local.x + moon_up * cell_local.y - moon_direction * cell_depth,
  );

  // Genuine sphere/sun illumination makes phases emerge from the independent lunar orbit.
  let sunlight = max(dot(facet_normal, sun_direction), 0.0);
  let phase_light = 0.012 + sunlight * 0.988;
  let fine = atmosphere_hash21(cell + vec2<f32>(43.0, 17.0));
  let coarse = atmosphere_hash21(floor(texture_uv * 4.0) + vec2<f32>(113.0, 71.0));
  let maria = select(1.0, 0.68, coarse < 0.27);
  let crater = select(1.0, 0.72, fine < 0.16);
  let albedo = mix(0.78, 1.10, fine) * maria * crater;
  return vec3<f32>(0.48, 0.54, 0.68) * albedo * phase_light * coverage;
}

fn octahedral_encode(direction: vec3<f32>) -> vec2<f32> {
  var encoded = direction.xy
    / max(abs(direction.x) + abs(direction.y) + abs(direction.z), 0.00001);
  if direction.z < 0.0 {
    let signs = vec2<f32>(
      select(-1.0, 1.0, encoded.x >= 0.0),
      select(-1.0, 1.0, encoded.y >= 0.0),
    );
    encoded = (vec2<f32>(1.0) - abs(encoded.yx)) * signs;
  }
  return encoded * 0.5 + 0.5;
}

fn celestial_star_radiance(ray: vec3<f32>, moon_disc: f32) -> vec3<f32> {
  let visibility = smoothstep(0.01, 0.20, ray.y)
    * frame.fog_exposure.w
    * (1.0 - moon_disc);
  if visibility <= 0.001 {
    return vec3<f32>(0.0);
  }
  // Convert the local world ray into one shared equatorial catalog. The basis changes with
  // synchronized sidereal time and observer location; the catalog itself never follows camera
  // translation or orientation.
  let celestial_ray = normalize(
    frame.equatorial_east.xyz * ray.x
      + frame.equatorial_up.xyz * ray.y
      - frame.equatorial_north.xyz * ray.z,
  );
  let grid = 640.0;
  let coordinates = octahedral_encode(celestial_ray) * grid;
  let cell = floor(coordinates);
  let catalog_seed = frame.equatorial_north.w;
  let seeded_cell = cell + vec2<f32>(
    fract(catalog_seed * 0.0000137) * 3072.0,
    fract(catalog_seed * 0.0000211) * 3072.0,
  );
  let identity = atmosphere_hash21(seeded_cell);
  // Fewer than one percent of catalog cells contain a star. Reject the others before evaluating
  // sub-cell placement, color, magnitude, and twinkle hashes.
  if identity <= 0.9935 {
    return vec3<f32>(0.0);
  }
  let offset = vec2<f32>(
    atmosphere_hash21(seeded_cell + vec2<f32>(19.0, 47.0)),
    atmosphere_hash21(seeded_cell + vec2<f32>(73.0, 11.0)),
  );
  let distance_to_star = length(fract(coordinates) - offset);
  let point = 1.0 - smoothstep(0.035, 0.145, distance_to_star);
  let candidate = smoothstep(0.9935, 0.9997, identity);
  let magnitude = mix(
    0.28,
    1.0,
    atmosphere_hash21(seeded_cell + vec2<f32>(131.0, 89.0)),
  );
  let twinkle_identity = atmosphere_hash21(seeded_cell + vec2<f32>(211.0, 157.0));
  let twinkle = 0.88 + 0.12 * sin(
    6.2831853 * (frame.equatorial_east.w * mix(0.72, 1.37, twinkle_identity)
      + twinkle_identity),
  );
  let color = mix(
    vec3<f32>(0.48, 0.62, 1.0),
    vec3<f32>(1.0, 0.82, 0.58),
    identity,
  );
  return color * point * candidate * magnitude * twinkle * visibility * 1.8;
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
  let horizon = pow(1.0 - abs(ray.y), 5.0);
  let sky_elevation = max(ray.y, 0.0);
  let rayleigh = pow(sky_elevation, 0.42);
  let upper_atmosphere = mix(frame.sky_horizon.rgb, frame.sky_zenith.rgb, rayleigh);
  // An infinite planar world has no geometric planet disc behind distant terrain. Extend the
  // atmosphere below the mathematical horizon so a high spectator camera sees one continuous
  // aerial-perspective backdrop rather than a flat horizon color ending above a dead lower half.
  // The ground irradiance only tints the nadir; keeping some horizon scattering also makes sparse
  // ultra-distant terrain disappear into haze instead of exposing its coverage boundary.
  let lower_depth = smoothstep(0.0, 0.78, max(-ray.y, 0.0));
  let lower_atmosphere = mix(
    frame.sky_horizon.rgb,
    frame.ground_atmosphere.rgb,
    lower_depth * 0.72,
  );
  let base = mix(lower_atmosphere, upper_atmosphere, smoothstep(-0.015, 0.025, ray.y));
  let sun_azimuth = normalize(sun_direction.xz + vec2<f32>(0.0001));
  let ray_azimuth = normalize(ray.xz + vec2<f32>(0.0001));
  let horizon_alignment = pow(max(dot(ray_azimuth, sun_azimuth), 0.0), 3.0);
  let sun_visible = frame.sun_direction.w * smoothstep(-0.025, 0.025, sun_direction.y);
  let moon_visible = frame.moon_direction.w * smoothstep(-0.01, 0.04, moon_direction.y);
  let warm_horizon = vec3<f32>(1.0, 0.34, 0.12)
    * horizon * horizon_alignment * sun_visible * 0.48;
  let sun_amount = max(dot(ray, sun_direction), 0.0);
  let sun_disc = smoothstep(0.99985, 0.99997, sun_amount) * sun_visible;
  let sun_glow = pow(sun_amount, 96.0) * 0.16 + pow(sun_amount, 12.0) * 0.022;
  let moon_amount = max(dot(ray, moon_direction), 0.0);
  let moon_disc = smoothstep(0.99990, 0.99998, moon_amount) * moon_visible;
  let moon_glow = pow(moon_amount, 320.0)
    * moon_visible * frame.equatorial_up.w * 0.018;
  let moon_surface = moon_surface_radiance(ray, moon_direction, sun_direction, moon_visible);
  var color = base
    + warm_horizon
    + vec3<f32>(5.8, 4.6, 3.4) * (sun_disc * 1.15 + sun_glow * sun_visible)
    + moon_surface
    + vec3<f32>(0.42, 0.50, 0.68) * moon_glow;
  color += celestial_star_radiance(ray, moon_disc);
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
