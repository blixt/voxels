//! Shared outdoor-lighting state used by sky, terrain, fog, and shadow projection.

use glam::Vec3;

/// Rust-owned daylight parameters. Keeping this outside WGSL prevents the sky and world pipelines
/// from quietly drifting to different suns or horizon colors as rendering evolves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutdoorEnvironment {
    pub sun_direction: Vec3,
    pub sun_radiance: Vec3,
    pub sky_horizon: Vec3,
    pub sky_zenith: Vec3,
    pub ground_irradiance: Vec3,
    pub fog_density: f32,
    pub fog_height_falloff: f32,
    pub exposure: f32,
}

impl Default for OutdoorEnvironment {
    fn default() -> Self {
        Self {
            sun_direction: Vec3::new(0.48, 0.72, 0.35).normalize(),
            sun_radiance: Vec3::new(4.8, 4.25, 3.55),
            sky_horizon: Vec3::new(0.58, 0.72, 0.88),
            sky_zenith: Vec3::new(0.055, 0.20, 0.56),
            ground_irradiance: Vec3::new(0.16, 0.19, 0.16),
            fog_density: 0.012,
            fog_height_falloff: 0.075,
            exposure: 1.0,
        }
    }
}

impl OutdoorEnvironment {
    pub fn sanitized(self) -> Self {
        let fallback = Self::default();
        let direction =
            if self.sun_direction.is_finite() && self.sun_direction.length_squared() > 0.0 {
                self.sun_direction.normalize()
            } else {
                fallback.sun_direction
            };
        Self {
            sun_direction: direction,
            sun_radiance: finite_non_negative(self.sun_radiance, fallback.sun_radiance),
            sky_horizon: finite_non_negative(self.sky_horizon, fallback.sky_horizon),
            sky_zenith: finite_non_negative(self.sky_zenith, fallback.sky_zenith),
            ground_irradiance: finite_non_negative(
                self.ground_irradiance,
                fallback.ground_irradiance,
            ),
            fog_density: finite_non_negative_scalar(self.fog_density, fallback.fog_density),
            fog_height_falloff: finite_non_negative_scalar(
                self.fog_height_falloff,
                fallback.fog_height_falloff,
            ),
            exposure: finite_non_negative_scalar(self.exposure, fallback.exposure),
        }
    }
}

fn finite_non_negative(value: Vec3, fallback: Vec3) -> Vec3 {
    if value.is_finite() {
        value.max(Vec3::ZERO)
    } else {
        fallback
    }
}

fn finite_non_negative_scalar(value: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        fallback
    }
}

/// CPU reference for the Khronos PBR Neutral tone mapper used by the present and glass shaders.
/// Keeping a host implementation gives regression tests a precise contract for the display curve.
pub fn pbr_neutral(color_in: Vec3) -> Vec3 {
    let color_in = color_in.max(Vec3::ZERO);
    let minimum = color_in.min_element();
    let offset = if minimum < 0.08 {
        minimum - minimum * minimum / 0.16
    } else {
        0.04
    };
    let color = color_in - Vec3::splat(offset);
    let peak = color.max_element();
    if peak < 0.76 {
        return color;
    }
    let new_peak = 1.0 - 0.0576 / (peak - 0.52);
    let desaturation = 1.0 / (0.15 * (peak - new_peak) + 1.0);
    Vec3::splat(new_peak).lerp(color * (new_peak / peak), desaturation)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_daylight_is_finite_normalized_and_positive() {
        let environment = OutdoorEnvironment::default().sanitized();
        assert!((environment.sun_direction.length() - 1.0).abs() < 1.0e-6);
        assert!(environment.sun_direction.y > 0.0);
        assert!(environment.sun_radiance.min_element() > 0.0);
        assert!(environment.sky_horizon.min_element() >= 0.0);
        assert!(environment.sky_zenith.min_element() >= 0.0);
        assert!(environment.fog_density > 0.0);
        assert!(environment.exposure > 0.0);
    }

    #[test]
    fn invalid_parameters_fall_back_without_nan_reaching_the_gpu() {
        let environment = OutdoorEnvironment {
            sun_direction: Vec3::ZERO,
            sun_radiance: Vec3::splat(f32::NAN),
            sky_horizon: Vec3::splat(-1.0),
            sky_zenith: Vec3::splat(f32::INFINITY),
            ground_irradiance: Vec3::splat(-2.0),
            fog_density: f32::NAN,
            fog_height_falloff: -1.0,
            exposure: f32::INFINITY,
        }
        .sanitized();
        assert!(environment.sun_direction.is_finite());
        assert!(environment.sun_radiance.is_finite());
        assert_eq!(environment.sky_horizon, Vec3::ZERO);
        assert!(environment.sky_zenith.is_finite());
        assert_eq!(environment.ground_irradiance, Vec3::ZERO);
        assert!(environment.fog_density.is_finite());
        assert_eq!(environment.fog_height_falloff, 0.0);
        assert!(environment.exposure.is_finite());
    }

    #[test]
    fn pbr_neutral_is_bounded_finite_and_monotonic_for_neutral_radiance() {
        let mut previous = 0.0;
        for step in 0..=512 {
            let input = step as f32 / 16.0;
            let mapped = pbr_neutral(Vec3::splat(input));
            assert!(mapped.is_finite());
            assert!(mapped.min_element() >= 0.0 && mapped.max_element() <= 1.0);
            assert!(mapped.x + f32::EPSILON >= previous);
            assert!((mapped.max_element() - mapped.min_element()).abs() < 1.0e-6);
            previous = mapped.x;
        }
    }

    #[test]
    fn pbr_neutral_preserves_hue_order_under_highlights() {
        let mapped = pbr_neutral(Vec3::new(8.0, 2.0, 0.5));
        assert!(mapped.x > mapped.y && mapped.y > mapped.z);
        assert!(mapped.max_element() <= 1.0);
    }
}
