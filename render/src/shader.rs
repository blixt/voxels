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
            ]
        );
    }

    #[test]
    fn voxel_shader_uses_one_energy_conserving_microfacet_model() {
        for required in [
            "fn fresnel_schlick(",
            "fn distribution_ggx(",
            "fn visibility_smith_ggx_correlated_fast(",
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
        assert!(include_str!("shaders/voxels.wgsl").contains("evaluate_direct_dielectric("));
        assert!(include_str!("shaders/avatar.wgsl").contains("evaluate_direct_dielectric("));
    }

    #[test]
    fn volumetric_clouds_and_terrain_share_one_seeded_world_space_weather_field() {
        let clouds = include_str!("shaders/clouds.wgsl");
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
            assert!(source.contains("mix(0.84, 0.45, coverage"));
            assert!(!source.contains("camera_time.w * 0.55"));
        }
        assert!(clouds.contains("macro_threshold - 0.08, macro_threshold + 0.08"));
        assert!(voxels.contains("threshold - 0.08, threshold + 0.08"));
        assert!(
            clouds.contains("fn cloud_density_world(world: vec3<f32>, filter_width_metres: f32)")
        );
        assert!(clouds.contains("textureSampleLevel(cloud_noise"));
        assert!(clouds.contains("cloud_noise_lod("));
        assert!(!clouds.contains("trace_start += jitter"));
        assert!(clouds.contains("transmittance < 0.02"));
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
        assert!(clouds.contains("let extra_capacity = min(6u"));
        assert!(clouds.contains("index >= view_steps"));
        assert!(clouds.contains("mix(sampled_density, previous_density, 0.32)"));
        assert!(clouds.contains("cloud_height_profile(shaped_height, local_growth)"));
        assert!(clouds.contains("let height_warp ="));
        assert!(clouds.contains("let crown_taper ="));
        assert!(clouds.contains("advected.y * 2.1"));
        assert!(clouds.contains("ambient * ambient_visibility + direct"));
        assert!(clouds.contains("mix(1.10, 0.86, powder)"));
        assert!(clouds.contains("let reconstructed_alpha = smoothstep("));
    }

    #[test]
    fn precipitation_is_world_space_depth_tested_geometry_that_falls_downward() {
        let weather = include_str!("shaders/weather.wgsl");
        assert!(weather.contains("@builtin(instance_index) instance_index: u32"));
        assert!(weather.contains("let server_time = frame.atmosphere_motion.x"));
        assert!(weather.contains("let vertical_phase = fract("));
        assert!(
            weather.contains("let vertical_cell = round((frame.camera_time.y - vertical_phase)")
        );
        assert!(weather.contains("velocity - frame.atmosphere_motion.yzw"));
        assert!(!weather.contains("frame.camera_time.w / fall_duration"));
        assert!(!weather.contains("frame.camera_time.y +"));
        assert!(weather.contains("frame.view_projection * vec4<f32>(world, 1.0)"));
        assert!(!weather.contains("world.xz +="));
        assert!(!weather.contains("world.xz ="));
        assert!(!weather.contains("position.xy / frame.viewport_voxel.xy"));
        assert!(!weather.contains("fn rain_layer("));
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
    fn moon_uses_phase_ready_sphere_lighting_and_bounded_apparent_facets() {
        let sky = include_str!("shaders/sky.wgsl");
        assert!(sky.contains("fn moon_surface_radiance("));
        assert!(sky.contains("let cell_count = 8.0"));
        assert!(sky.contains("let facet_normal = normalize("));
        assert!(sky.contains("max(dot(facet_normal, sun_direction), 0.0)"));
        assert!(sky.contains("let phase_light = 0.055 + sunlight * 0.945"));
        assert!(sky.contains("* (1.0 - moon_disc)"));
        assert!(!sky.contains("moon_disc * 0.82"));
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
