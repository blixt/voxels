use std::borrow::Cow;

const FRAME_SOURCE: &str = include_str!("shaders/frame.wgsl");
const PBR_SOURCE: &str = include_str!("shaders/pbr.wgsl");

pub(crate) fn frame_shader(
    device: &wgpu::Device,
    label: &'static str,
    source: &'static str,
) -> wgpu::ShaderModule {
    shader_from_sources(device, label, &[FRAME_SOURCE, source])
}

pub(crate) fn frame_pbr_shader(
    device: &wgpu::Device,
    label: &'static str,
    source: &'static str,
) -> wgpu::ShaderModule {
    shader_from_sources(device, label, &[FRAME_SOURCE, PBR_SOURCE, source])
}

fn shader_from_sources(
    device: &wgpu::Device,
    label: &'static str,
    sources: &[&str],
) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(Cow::Owned(sources.join("\n"))),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn shared_frame_source_matches_the_host_uniform_order() {
        let fields = FRAME_SOURCE
            .lines()
            .take_while(|line| line.trim() != "};")
            .filter_map(|line| line.trim().split_once(':').map(|(name, _)| name))
            .collect::<Vec<_>>();
        assert_eq!(
            fields,
            [
                "view_projection",
                "inverse_view_projection",
                "camera_time",
                "viewport_voxel",
                "target_voxel",
                "target_voxel_max",
                "render_options",
                "lod_options",
                "lod_boundary_centres",
                "camera_forward",
                "shadow_splits",
                "shadow_texel_sizes",
                "shadow_view_projection",
                "key_light_direction",
                "key_light_radiance",
                "sun_direction",
                "moon_direction",
                "equatorial_east",
                "equatorial_up",
                "equatorial_north",
                "environment_time",
                "atmosphere_motion",
                "sky_horizon",
                "sky_zenith",
                "ground_atmosphere",
                "fog_exposure",
                "weather",
                "cloud_layer",
                "medium",
                "interior",
                "diagnostic_sky",
            ]
        );
    }

    #[test]
    fn voxel_shader_uses_one_energy_conserving_microfacet_model() {
        for required in [
            "fn fresnel_schlick(",
            "fn distribution_ggx(",
            "fn visibility_smith_ggx_correlated_fast(",
            "fn evaluate_direct_dielectric_f0(",
            "fn evaluate_direct_dielectric(",
            "fn specular_ambient_visibility(",
        ] {
            assert!(PBR_SOURCE.contains(required), "missing {required}");
        }
        assert!(!PBR_SOURCE.contains("specular_power"));
        assert_eq!(
            PBR_SOURCE.matches("fn evaluate_direct_dielectric(").count(),
            1
        );
        assert!(include_str!("shaders/voxels.wgsl").contains("evaluate_direct_dielectric_f0("));
        assert!(include_str!("shaders/avatar.wgsl").contains("evaluate_direct_dielectric("));
    }

    #[test]
    fn rain_uses_one_cloud_sample_to_create_a_physical_water_film() {
        let voxels = include_str!("shaders/voxels.wgsl");
        assert!(voxels.contains("fn cloud_surface_weather("));
        assert!(!voxels.contains("fn cloud_sun_visibility("));
        assert!(FRAME_SOURCE.contains("fn liquid_precipitation()"));
        assert!(FRAME_SOURCE.contains("max(frame.weather.x - frame.weather.w, 0.0)"));
        assert_eq!(
            voxels
                .matches("let surface_weather = cloud_surface_weather(input.world);")
                .count(),
            1
        );
        assert!(voxels.contains("let local_precipitation = liquid_precipitation()"));
        assert!(voxels.contains("surface_weather.y"));
        assert!(voxels.contains("surface_detail.roughness * 0.24"));
        assert!(voxels.contains("mix(DIELECTRIC_F0, vec3<f32>(0.02037), wetness)"));
        assert!(voxels.contains("evaluate_direct_dielectric_f0("));

        let dry_albedo = 0.72_f32;
        let dry_roughness = 0.84_f32;
        let wetness = 1.0_f32;
        let wet_albedo = dry_albedo * (1.0 + (0.64 - 1.0) * wetness);
        let wet_roughness = (dry_roughness * 0.24).max(0.089);
        assert!(wet_albedo < dry_albedo);
        assert!(wet_roughness < dry_roughness * 0.3);
        assert!((0.02037_f32 - 1.0 / 49.0).abs() < 0.000_04);
    }

    #[test]
    fn terrain_fog_integrates_physical_distance_without_a_horizon_knee() {
        let voxels = include_str!("shaders/voxels.wgsl");
        assert!(!voxels.contains("fn atmospheric_path_length("));
        assert!(!voxels.contains("fog_knee"));
        assert_eq!(
            voxels
                .matches("let optical_depth = distance_to_camera")
                .count(),
            2
        );
    }

    #[test]
    fn edit_outline_uses_the_authoritative_sphere_or_cube_stencil() {
        let voxels = include_str!("shaders/voxels.wgsl");
        assert!(voxels.contains("let target_center ="));
        assert!(voxels.contains("let target_is_cube = frame.target_voxel.w > 1.5;"));
        assert!(voxels.contains(
            "let inside_target_shape = target_is_cube || dot(target_delta, target_delta) < 39.0;"
        ));
    }

    #[test]
    fn volumetric_clouds_and_terrain_share_one_seeded_world_space_weather_field() {
        let clouds = include_str!("shaders/clouds.wgsl");
        let weather = include_str!("shaders/weather.wgsl");
        let voxels = include_str!("shaders/voxels.wgsl");
        assert!(FRAME_SOURCE.contains("environment_time: vec4<f32>"));
        assert!(FRAME_SOURCE.contains("weather: vec4<f32>"));
        assert!(FRAME_SOURCE.contains("cloud_layer: vec4<f32>"));
        assert!(FRAME_SOURCE.contains("position = (world_xz - cloud_offset_metres) * 0.0008"));
        assert!(FRAME_SOURCE.contains("periodic_gradient_noise"));
        assert!(FRAME_SOURCE.contains("fn atmosphere_branchless_gradient("));
        assert!(FRAME_SOURCE.contains("position * 2.0"));
        assert!(FRAME_SOURCE.contains("position * 4.0"));
        assert!(!FRAME_SOURCE.contains("switch u32(floor(hash * 8.0))"));
        assert!(!FRAME_SOURCE.contains("position * 2.03"));
        assert!(!FRAME_SOURCE.contains("position * 4.11"));
        for source in [clouds, voxels] {
            assert_eq!(source.matches("atmosphere_cloud_field_world(").count(), 1);
            assert!(source.contains("frame.environment_time.yz"));
            assert!(source.contains("frame.environment_time.w"));
            assert!(!source.contains("camera_time.w * 0.55"));
        }
        assert!(FRAME_SOURCE.contains("mix(0.84, 0.45, coverage)"));
        assert!(FRAME_SOURCE.contains("fn atmosphere_cloud_envelope("));
        assert!(FRAME_SOURCE.contains("fn atmosphere_cloud_envelope_world("));
        assert!(
            clouds.contains("let envelope = atmosphere_cloud_envelope(macro_field, coverage);")
        );
        assert!(weather.contains("let rain_cloud = atmosphere_cloud_envelope_world(world.xz);"));
        assert!(weather.contains("if rain_cloud <= 0.08"));
        assert!(FRAME_SOURCE.contains("threshold - 0.08, threshold + 0.08"));
        assert!(voxels.contains("atmosphere_cloud_envelope(field, coverage_control)"));
        assert!(
            clouds.contains("fn cloud_density_world(world: vec3<f32>, filter_width_metres: f32)")
        );
        assert!(clouds.contains("textureSampleLevel(cloud_noise"));
        assert!(clouds.contains("cloud_noise_lod("));
        assert!(!clouds.contains("trace_start += jitter"));
        assert!(clouds.contains("transmittance < 0.02"));
        assert!(clouds.contains("textureLoad(cloud_target"));
        assert!(!clouds.contains("textureSampleLevel(cloud_target"));
        let ordered = clouds
            .split_once("const ORDERED_4X4")
            .expect("ordered reconstruction table")
            .1
            .split_once(");")
            .expect("ordered reconstruction table terminator")
            .0
            .split_once('(')
            .expect("ordered reconstruction values")
            .1;
        let mut ranks = ordered
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.parse::<f32>().expect("ordered rank is numeric") as u32)
            .collect::<Vec<_>>();
        ranks.sort_unstable();
        assert_eq!(ranks, (0..16).collect::<Vec<_>>());
        assert!(!include_str!("shaders/sky.wgsl").contains("cloud_height = 480.0"));

        let base_cells = 1_280_000.0_f32 * 0.0008;
        assert_eq!(base_cells, 1_024.0);
        assert_eq!(base_cells * 2.0, 2_048.0);
        assert_eq!(base_cells * 4.0, 4_096.0);
    }

    #[test]
    fn distant_voxel_walls_fade_to_macro_lighting_instead_of_switching_normals() {
        let voxels = include_str!("shaders/voxels.wgsl");
        assert!(voxels.contains("fn surface_wall_macro_blend("));
        assert!(voxels.contains("smoothstep(0.0, 48.0, distance_from_near_field) * 0.82"));
        assert!(voxels.contains("face == 2u"));
    }

    #[test]
    fn subpixel_distant_surfaces_use_prefiltered_macro_lighting() {
        let voxels = include_str!("shaders/voxels.wgsl");
        assert!(voxels.contains("fn distant_surface_radiance("));
        assert!(voxels.contains("if distance_to_camera >= 144.0"));
        assert!(voxels.contains("smoothstep(96.0, 144.0, distance_to_camera)"));
        assert!(voxels.contains("transport_surface_radiance(distant_radiance"));
        assert!(voxels.contains("let detail_uv_dx = dpdx(continuous_uv);"));
        assert!(!voxels.contains("if distance_to_camera >= 144.0 {\n    discard;"));
    }

    #[test]
    fn terrain_horizon_lighting_tracks_key_direction_and_preserves_sky_access() {
        fn smoothstep(low: f32, high: f32, value: f32) -> f32 {
            let amount = ((value - low) / (high - low)).clamp(0.0, 1.0);
            amount * amount * (3.0 - 2.0 * amount)
        }
        fn lighting(profile: u16, light: Vec3, terrain_normal: Vec3) -> [f32; 2] {
            let angles = [
                0.0_f32,
                6.0_f32.to_radians(),
                16.0_f32.to_radians(),
                35.0_f32.to_radians(),
            ];
            let decoded = std::array::from_fn::<_, 4, _>(|direction| {
                angles[usize::from((profile >> (direction * 2)) & 3)]
            });
            let horizontal = glam::Vec2::new(light.x, light.z).abs();
            let x_horizon = if light.x >= 0.0 {
                decoded[0]
            } else {
                decoded[1]
            };
            let z_horizon = if light.z >= 0.0 {
                decoded[3]
            } else {
                decoded[2]
            };
            let horizon = (x_horizon * horizontal.x + z_horizon * horizontal.y)
                / (horizontal.x + horizontal.y).max(0.0001);
            let elevation = light
                .y
                .max(0.0)
                .atan2(glam::Vec2::new(light.x, light.z).length().max(0.0001));
            let key = smoothstep(
                horizon - 4.0_f32.to_radians(),
                horizon + 4.0_f32.to_radians(),
                elevation,
            );
            let sector_accessibility = [1.0_f32, 0.85, 0.60, 0.32];
            let accessibility = (0..4)
                .map(|direction| {
                    sector_accessibility[usize::from((profile >> (direction * 2)) & 3)]
                })
                .sum::<f32>()
                * 0.25;
            let directional_sky = (1.0
                + (terrain_normal.x * light.x + terrain_normal.z * light.z) * 1.1)
                .clamp(0.75, 1.0);
            [key, (1.0 + (accessibility - 1.0) * 0.82) * directional_sky]
        }

        // East has a 35-degree ridge while the other three sectors are open. Parent matches own.
        let profile = 3_u16 | (3_u16 << 8);
        let low_east = Vec3::new(1.0, 10.0_f32.to_radians().tan(), 0.0).normalize();
        let low_west = Vec3::new(-1.0, 10.0_f32.to_radians().tan(), 0.0).normalize();
        let high_east = Vec3::new(1.0, 60.0_f32.to_radians().tan(), 0.0).normalize();
        let east = lighting(profile, low_east, Vec3::Y);
        let west = lighting(profile, low_west, Vec3::Y);
        let high = lighting(profile, high_east, Vec3::Y);
        assert!(
            east[0] < 0.01,
            "low east light is below the encoded ridge: {east:?}"
        );
        assert!(
            west[0] > 0.99,
            "turning the key light reveals the open west sky: {west:?}"
        );
        assert!(
            high[0] > 0.99,
            "a high key light clears the same ridge: {high:?}"
        );
        assert!(
            (0.82..0.90).contains(&east[1]),
            "one blocked sector softly reduces sky access"
        );
        let sun_facing = lighting(0, low_east, Vec3::new(0.35, 1.0, 0.0).normalize());
        let away_facing = lighting(0, low_east, Vec3::new(-0.35, 1.0, 0.0).normalize());
        assert_eq!(sun_facing[1], 1.0);
        assert!(
            away_facing[1] < 0.80,
            "broad away-facing slopes receive less directional sky fill: {away_facing:?}",
        );

        let voxels = include_str!("shaders/voxels.wgsl");
        assert!(voxels.contains("fn unpack_surface_horizon_profile("));
        assert!(voxels.contains("fn terrain_horizon_lighting("));
        assert!(voxels.contains("normalize(frame.key_light_direction.xyz)"));
        assert!(voxels.contains("* input.terrain_lighting.x"));
        assert!(voxels.contains("* input.terrain_lighting.y"));
    }

    #[test]
    fn cloud_raymarch_stratifies_and_reconstructs_bounded_samples() {
        let clouds = include_str!("shaders/clouds.wgsl");
        assert!(clouds.contains("fract(stable_sample_phase + f32(index) * 0.61803398875)"));
        assert!(clouds.contains("f32(index) + stratified_sample_phase"));
        assert!(clouds.contains("index < 24u"));
        assert!(clouds.contains("let view_steps = clouds.quality.x"));
        assert!(!clouds.contains("grazing_quality"));
        assert!(clouds.contains("index >= view_steps"));
        assert!(clouds.contains("mix(sampled_density, previous_density, 0.32)"));
        assert!(clouds.contains("cloud_height_profile(shaped_height, local_growth)"));
        assert!(clouds.contains("let height_warp ="));
        assert!(clouds.contains("let crown_taper ="));
        assert!(clouds.contains("advected.y * 2.1"));
        assert!(clouds.contains("ambient * ambient_visibility + direct"));
        assert!(clouds.contains("mix(1.10, 0.86, powder)"));
        assert!(clouds.contains("if frame.key_light_radiance.w > 0.02"));
        assert!(clouds.contains("smoothstep(0.02, 0.18, frame.key_light_radiance.w)"));
        assert!(clouds.contains("select(0.5, 1.0, cloud.a > 0.72)"));
        assert!(clouds.contains("cloud.a > 0.18"));
        assert!(!clouds.contains("smoothstep(0.035, 0.965, cloud.a)"));
        assert!(clouds.contains("let radiance_scale ="));
    }

    #[test]
    fn precipitation_is_world_space_depth_tested_geometry_that_falls_downward() {
        let weather = include_str!("shaders/weather.wgsl");
        assert!(weather.contains("@builtin(instance_index) instance_index: u32"));
        assert!(weather.contains("let server_time = frame.atmosphere_motion.x"));
        assert!(weather.contains("mix(16.0, 24.0, random_speed)"));
        assert!(weather.contains("* 128.0"));
        assert!(weather.contains("let vertical_phase = fract("));
        assert!(
            weather.contains("let vertical_cell = round((frame.camera_time.y - vertical_phase)")
        );
        assert!(weather.contains("velocity - frame.atmosphere_motion.yzw"));
        assert!(!weather.contains("frame.camera_time.w / fall_duration"));
        assert!(!weather.contains("frame.camera_time.y +"));
        assert!(weather.contains("frame.view_projection * vec4<f32>(world, 1.0)"));
        assert!(weather.contains("smoothstep(0.85, 1.35, projected_radius_pixels)"));
        assert!(weather.contains("smoothstep(1.5, 3.0, length(segment_pixels))"));
        assert!(!weather.contains("world.xz +="));
        assert!(!weather.contains("world.xz ="));
        assert!(!weather.contains("position.xy / frame.viewport_voxel.xy"));
        assert!(!weather.contains("fn rain_layer("));
    }

    #[test]
    fn voxel_and_shadow_vertices_convert_shared_integer_corners_only_once() {
        let voxels = include_str!("shaders/voxels.wgsl");
        let shadows = include_str!("shaders/shadow.wgsl");
        for shader in [voxels, shadows] {
            assert!(shader.contains("@location(0) origin: vec3<i32>"));
            assert!(shader.contains("vec3<f32>(origin + local)"));
            assert!(!shader.contains("let world = origin + local"));
        }
    }

    #[test]
    fn far_voxel_faces_expand_raster_coverage_without_stretching_world_space() {
        let voxels = include_str!("shaders/voxels.wgsl");
        assert!(voxels.contains("if (material & 0x80000000u) == 0u"));
        assert!(voxels.contains("fn conservative_surface_clip("));
        assert!(voxels.contains("direction * 1.5 / frame.viewport_voxel.xy"));
        assert!(voxels.contains("out.position = conservative_surface_clip(world, face, uv"));
        assert!(voxels.contains("out.world = world"));
    }

    #[test]
    fn precipitation_vertical_lattice_is_camera_invariant_within_a_world_cell() {
        const HEIGHT: f32 = 32.0;
        fn world_y(initial_phase: f32, server_time: f32, fall_duration: f32, camera_y: f32) -> f32 {
            let vertical_phase =
                (initial_phase - server_time / fall_duration).rem_euclid(1.0) * HEIGHT;
            vertical_phase + ((camera_y - vertical_phase) / HEIGHT).round() * HEIGHT
        }

        let first_client = world_y(0.37, 42.0, 2.8, 73.0);
        let second_client = world_y(0.37, 42.0, 2.8, 78.0);
        assert_eq!(first_client, second_client);

        let one_frame_later = world_y(0.37, 42.01, 2.8, 73.0);
        let expected_fall = HEIGHT / 2.8 * 0.01;
        assert!(((first_client - one_frame_later) - expected_fall).abs() < 1.0e-4);
    }

    #[test]
    fn primary_rainbow_obeys_antisolar_geometry_and_weather() {
        fn visible(
            sun_elevation_degrees: f32,
            bow_radius_degrees: f32,
            precipitation: f32,
        ) -> bool {
            let sun_y = sun_elevation_degrees.to_radians().sin();
            let antisolar_cosine = bow_radius_degrees.to_radians().cos();
            (0.015..0.669_130_6).contains(&sun_y)
                && (0.7314..=0.7716).contains(&antisolar_cosine)
                && precipitation > 0.04
        }

        assert!(visible(18.0, 42.0, 0.7));
        assert!(!visible(50.0, 42.0, 0.7));
        assert!(!visible(18.0, 20.0, 0.7));
        assert!(!visible(18.0, 42.0, 0.0));

        let frame = FRAME_SOURCE;
        let sky = include_str!("shaders/sky.wgsl");
        let clouds = include_str!("shaders/clouds.wgsl");
        assert!(frame.contains("fn primary_rainbow_radiance("));
        assert!(frame.contains("fn primary_rainbow_weather_possible()"));
        assert!(frame.contains("liquid_precipitation() > 0.04"));
        assert!(frame.contains("let antisolar_cosine = dot(ray, -sun);"));
        assert!(frame.contains("0.6691306"));
        assert!(frame.contains("atmosphere_cloud_envelope_world(shower_world_xz)"));
        assert!(frame.contains("if antisolar_cosine < 0.7314"));
        let angular_reject = frame.find("if antisolar_cosine < 0.7314").unwrap();
        let cloud_sample = frame
            .find("atmosphere_cloud_envelope_world(shower_world_xz)")
            .unwrap();
        assert!(angular_reject < cloud_sample);
        assert!(sky.contains("color += primary_rainbow_radiance(ray);"));
        assert!(clouds.contains("if primary_rainbow_weather_possible()"));
        assert!(clouds.contains("rainbow * reconstructed_alpha"));
    }

    #[test]
    fn moon_uses_phase_ready_sphere_lighting_and_bounded_apparent_facets() {
        let sky = include_str!("shaders/sky.wgsl");
        assert!(sky.contains("fn moon_surface_radiance("));
        assert!(sky.contains("let cell_count = 8.0"));
        assert!(sky.contains("let facet_normal = normalize("));
        assert!(sky.contains("max(dot(facet_normal, sun_direction), 0.0)"));
        assert!(sky.contains("let phase_light = 0.012 + sunlight * 0.988"));
        assert!(sky.contains("frame.equatorial_up.w"));
        assert!(sky.contains("* (1.0 - moon_disc)"));
        assert!(!sky.contains("moon_disc * 0.82"));
    }

    #[test]
    fn sky_atmosphere_continues_smoothly_below_the_flight_horizon() {
        let sky = include_str!("shaders/sky.wgsl");
        assert!(sky.contains("let lower_depth = smoothstep(0.0, 0.78, max(-ray.y, 0.0));"));
        assert!(sky.contains("lower_depth * 0.72"));
        assert!(sky.contains(
            "let base = mix(lower_atmosphere, upper_atmosphere, smoothstep(-0.015, 0.025, ray.y));"
        ));
        assert!(!sky.contains("ray.y * 0.5 + 0.5"));
        assert!(!sky.contains("let below_horizon ="));
    }

    #[test]
    fn stars_use_the_synchronized_equatorial_catalog_and_bounded_twinkle() {
        let sky = include_str!("shaders/sky.wgsl");
        assert!(sky.contains("fn octahedral_encode("));
        assert!(sky.contains("fn celestial_star_radiance("));
        assert!(sky.contains("frame.equatorial_east.xyz * ray.x"));
        assert!(sky.contains("- frame.equatorial_north.xyz * ray.z"));
        assert!(sky.contains("frame.equatorial_east.w * mix(0.72, 1.37"));
        assert!(sky.contains("let twinkle = 0.88 + 0.12 * sin("));
        assert!(sky.contains("if visibility <= 0.001"));
        assert!(sky.contains("if identity <= 0.9935"));
        assert!(!sky.contains("ray.xz / max(ray.y + 1.08"));
        assert!(!sky.contains("star_coordinates = ray.xz"));
    }

    #[test]
    fn dielectric_fresnel_has_physical_endpoints() {
        let f0 = 0.04;
        assert!((fresnel_schlick(1.0, f0) - f0).abs() < f32::EPSILON);
        assert!((fresnel_schlick(0.0, f0) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn microfacet_brdf_is_reciprocal_finite_and_nonnegative() {
        let normal = Vec3::Y;
        let view = Vec3::new(0.35, 0.91, -0.22).normalize();
        let light = Vec3::new(-0.58, 0.79, 0.20).normalize();
        for roughness in [0.089, 0.25, 0.5, 0.9, 1.0] {
            let forward = dielectric_brdf(normal, view, light, roughness, 0.7);
            let reverse = dielectric_brdf(normal, light, view, roughness, 0.7);
            assert!(forward.is_finite() && forward >= 0.0);
            assert!((forward - reverse).abs() < 0.000_01);
        }
    }

    #[test]
    fn dielectric_white_furnace_reflectance_stays_energy_bounded() {
        let normal = Vec3::Y;
        let view = normal;
        let theta_steps = 128;
        let phi_steps = 256;
        let delta_theta = std::f32::consts::FRAC_PI_2 / theta_steps as f32;
        let delta_phi = std::f32::consts::TAU / phi_steps as f32;
        for roughness in [0.089, 0.25, 0.5, 0.9, 1.0] {
            let mut reflectance = 0.0;
            for theta_index in 0..theta_steps {
                let theta = (theta_index as f32 + 0.5) * delta_theta;
                let sin_theta = theta.sin();
                let cos_theta = theta.cos();
                for phi_index in 0..phi_steps {
                    let phi = (phi_index as f32 + 0.5) * delta_phi;
                    let light = Vec3::new(sin_theta * phi.cos(), cos_theta, sin_theta * phi.sin());
                    reflectance += dielectric_brdf(normal, view, light, roughness, 1.0)
                        * cos_theta
                        * sin_theta
                        * delta_theta
                        * delta_phi;
                }
            }
            assert!(
                reflectance <= 1.01,
                "roughness {roughness} reflected {reflectance}"
            );
            assert!(reflectance > 0.75);
        }
    }

    #[test]
    fn roughness_broadens_and_lowers_the_normal_incidence_peak() {
        let normal = Vec3::Y;
        let smooth = dielectric_brdf(normal, normal, normal, 0.2, 0.5);
        let rough = dielectric_brdf(normal, normal, normal, 0.8, 0.5);
        assert!(smooth > rough * 8.0);
    }

    fn fresnel_schlick(cosine: f32, f0: f32) -> f32 {
        let complement = 1.0 - cosine.clamp(0.0, 1.0);
        f0 + (1.0 - f0) * complement.powi(5)
    }

    fn dielectric_brdf(
        normal: Vec3,
        view: Vec3,
        light: Vec3,
        perceptual_roughness: f32,
        albedo: f32,
    ) -> f32 {
        let no_v = normal.dot(view).max(0.0001);
        let no_l = normal.dot(light).max(0.0);
        let half_direction = (view + light).normalize_or_zero();
        let no_h = normal.dot(half_direction).max(0.0);
        let lo_h = light.dot(half_direction).max(0.0);
        let roughness = perceptual_roughness.max(0.089);
        let alpha = roughness * roughness;
        let alpha_squared = alpha * alpha;
        let denominator = (no_h * alpha_squared - no_h) * no_h + 1.0;
        let distribution =
            alpha_squared / (std::f32::consts::PI * denominator * denominator).max(0.000_001);
        let visibility_v = no_l * (no_v * (1.0 - alpha) + alpha);
        let visibility_l = no_v * (no_l * (1.0 - alpha) + alpha);
        let visibility = 0.5 / (visibility_v + visibility_l).max(0.0001);
        let fresnel = fresnel_schlick(lo_h, 0.04);
        albedo * (1.0 - fresnel) / std::f32::consts::PI + distribution * visibility * fresnel
    }
}
