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
                "sky_horizon",
                "sky_zenith",
                "ground_atmosphere",
                "fog_exposure",
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
    fn sky_and_terrain_share_one_seeded_world_space_cloud_field() {
        let sky = include_str!("shaders/sky.wgsl");
        let voxels = include_str!("shaders/voxels.wgsl");
        assert!(FRAME_SOURCE.contains("weather_seed: f32"));
        for source in [sky, voxels] {
            assert_eq!(source.matches("atmosphere_cloud_field_world(").count(), 1);
            assert!(source.contains("frame.environment_time.yz"));
            assert!(source.contains("frame.environment_time.w"));
            assert!(!source.contains("camera_time.w * 0.55"));
        }
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
