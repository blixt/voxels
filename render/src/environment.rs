//! Shared outdoor-lighting state used by sky, terrain, fog, and shadow projection.

use glam::Vec3;
use voxels_core::EnclosureSample;
use voxels_world::{AtmosphereSample, SurfaceRegion};

#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DaylightPhase {
    Dawn,
    ClearDay,
    #[default]
    GoldenHour,
    BlueHour,
}

impl DaylightPhase {
    pub const ALL: [Self; 4] = [Self::Dawn, Self::ClearDay, Self::GoldenHour, Self::BlueHour];

    pub const fn next(self) -> Self {
        match self {
            Self::Dawn => Self::ClearDay,
            Self::ClearDay => Self::GoldenHour,
            Self::GoldenHour => Self::BlueHour,
            Self::BlueHour => Self::Dawn,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Dawn => "DAWN",
            Self::ClearDay => "CLEAR DAY",
            Self::GoldenHour => "GOLDEN HOUR",
            Self::BlueHour => "BLUE HOUR",
        }
    }
}

pub const fn surface_region_label(region: SurfaceRegion) -> &'static str {
    match region {
        SurfaceRegion::VerdantForest => "VERDANT FOREST",
        SurfaceRegion::WindMoor => "WIND MOOR",
        SurfaceRegion::Alpine => "ALPINE",
        SurfaceRegion::RedBadlands => "RED BADLANDS",
        SurfaceRegion::PaleDunes => "PALE DUNES",
        SurfaceRegion::Volcanic => "VOLCANIC",
    }
}

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
    pub cloud_coverage: f32,
    pub star_visibility: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InteriorEnvironment {
    pub enclosure: f32,
    pub exposure_multiplier: f32,
    pub fog_density: f32,
    pub headlamp_strength: f32,
}

impl Default for InteriorEnvironment {
    fn default() -> Self {
        Self {
            enclosure: 0.0,
            exposure_multiplier: 1.0,
            fog_density: 0.0,
            headlamp_strength: 0.0,
        }
    }
}

impl InteriorEnvironment {
    pub fn for_enclosure(sample: EnclosureSample) -> Self {
        let enclosure = sample.enclosure.clamp(0.0, 1.0);
        Self {
            enclosure,
            exposure_multiplier: 1.0 + enclosure * 1.15,
            fog_density: enclosure * 0.032,
            headlamp_strength: ((enclosure - 0.32) / 0.68).clamp(0.0, 1.0),
        }
    }

    pub fn lerp(self, target: Self, amount: f32, exposure_amount: f32) -> Self {
        let amount = amount.clamp(0.0, 1.0);
        let exposure_amount = exposure_amount.clamp(0.0, 1.0);
        Self {
            enclosure: scalar_lerp(self.enclosure, target.enclosure, amount),
            exposure_multiplier: scalar_lerp(
                self.exposure_multiplier,
                target.exposure_multiplier,
                exposure_amount,
            ),
            fog_density: scalar_lerp(self.fog_density, target.fog_density, amount),
            headlamp_strength: scalar_lerp(
                self.headlamp_strength,
                target.headlamp_strength,
                amount,
            ),
        }
    }
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
            cloud_coverage: 0.42,
            star_visibility: 0.0,
        }
    }
}

impl OutdoorEnvironment {
    pub fn for_atmosphere(sample: AtmosphereSample, phase: DaylightPhase) -> Self {
        let mut environment = match phase {
            DaylightPhase::Dawn => Self {
                sun_direction: Vec3::new(-0.74, 0.24, 0.43).normalize(),
                sun_radiance: Vec3::new(6.0, 3.05, 1.55),
                sky_horizon: Vec3::new(0.72, 0.39, 0.28),
                sky_zenith: Vec3::new(0.045, 0.11, 0.31),
                ground_irradiance: Vec3::new(0.14, 0.13, 0.12),
                fog_density: 0.014,
                fog_height_falloff: 0.070,
                exposure: 0.98,
                cloud_coverage: 0.44,
                star_visibility: 0.08,
            },
            DaylightPhase::ClearDay => Self::default(),
            DaylightPhase::GoldenHour => Self {
                sun_direction: Vec3::new(0.79, 0.38, 0.28).normalize(),
                sun_radiance: Vec3::new(6.1, 4.05, 2.35),
                sky_horizon: Vec3::new(0.70, 0.52, 0.39),
                sky_zenith: Vec3::new(0.045, 0.15, 0.43),
                ground_irradiance: Vec3::new(0.18, 0.17, 0.13),
                fog_density: 0.013,
                fog_height_falloff: 0.068,
                exposure: 0.98,
                cloud_coverage: 0.38,
                star_visibility: 0.0,
            },
            DaylightPhase::BlueHour => Self {
                sun_direction: Vec3::new(-0.70, 0.18, -0.46).normalize(),
                sun_radiance: Vec3::new(1.45, 1.65, 2.35),
                sky_horizon: Vec3::new(0.18, 0.24, 0.38),
                sky_zenith: Vec3::new(0.012, 0.038, 0.13),
                ground_irradiance: Vec3::new(0.052, 0.066, 0.10),
                fog_density: 0.017,
                fog_height_falloff: 0.062,
                exposure: 1.10,
                cloud_coverage: 0.36,
                star_visibility: 0.62,
            },
        };
        let humidity = sample.humidity.clamp(0.0, 1.0);
        let coldness = sample.coldness.clamp(0.0, 1.0);
        let aerosol = sample.aerosol.clamp(0.0, 1.0);
        let haze = sample.haze.clamp(0.0, 1.0);
        let warmth = sample.horizon_warmth.clamp(0.0, 1.0);
        environment.cloud_coverage = (environment.cloud_coverage * 0.58
            + sample.cloudiness.clamp(0.0, 1.0) * 0.66)
            .clamp(0.08, 0.94);
        environment.fog_density *= 0.62 + haze * 1.16 + humidity * 0.22;
        environment.fog_height_falloff *= 1.12 - humidity * 0.30 + aerosol * 0.16;
        environment.sky_horizon = environment
            .sky_horizon
            .lerp(
                Vec3::new(0.86, 0.39, 0.17),
                warmth * (0.12 + aerosol * 0.12),
            )
            .lerp(Vec3::new(0.54, 0.66, 0.76), humidity * 0.10);
        environment.sky_zenith = environment
            .sky_zenith
            .lerp(Vec3::new(0.12, 0.13, 0.16), aerosol * 0.24)
            .lerp(Vec3::new(0.08, 0.19, 0.42), coldness * 0.10);
        environment.ground_irradiance = environment
            .ground_irradiance
            .lerp(Vec3::new(0.11, 0.19, 0.13), humidity * 0.22)
            .lerp(Vec3::new(0.10, 0.14, 0.22), coldness * 0.18)
            .lerp(Vec3::new(0.12, 0.105, 0.09), aerosol * 0.24);
        environment.sun_radiance *= 1.0 - environment.cloud_coverage * 0.18 - aerosol * 0.16;
        environment.star_visibility *= 1.0 - environment.cloud_coverage * 0.78;
        environment.sanitized()
    }

    pub fn lerp(self, target: Self, amount: f32) -> Self {
        let amount = amount.clamp(0.0, 1.0);
        let direction = self.sun_direction.lerp(target.sun_direction, amount);
        Self {
            sun_direction: if direction.length_squared() > 0.0001 {
                direction.normalize()
            } else {
                target.sun_direction
            },
            sun_radiance: self.sun_radiance.lerp(target.sun_radiance, amount),
            sky_horizon: self.sky_horizon.lerp(target.sky_horizon, amount),
            sky_zenith: self.sky_zenith.lerp(target.sky_zenith, amount),
            ground_irradiance: self
                .ground_irradiance
                .lerp(target.ground_irradiance, amount),
            fog_density: scalar_lerp(self.fog_density, target.fog_density, amount),
            fog_height_falloff: scalar_lerp(
                self.fog_height_falloff,
                target.fog_height_falloff,
                amount,
            ),
            exposure: scalar_lerp(self.exposure, target.exposure, amount),
            cloud_coverage: scalar_lerp(self.cloud_coverage, target.cloud_coverage, amount),
            star_visibility: scalar_lerp(self.star_visibility, target.star_visibility, amount),
        }
        .sanitized()
    }

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
            cloud_coverage: finite_unit_scalar(self.cloud_coverage, fallback.cloud_coverage),
            star_visibility: finite_unit_scalar(self.star_visibility, fallback.star_visibility),
        }
    }
}

fn scalar_lerp(from: f32, to: f32, amount: f32) -> f32 {
    from + (to - from) * amount
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

fn finite_unit_scalar(value: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
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
            cloud_coverage: f32::NAN,
            star_visibility: 4.0,
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
        assert!((0.0..=1.0).contains(&environment.cloud_coverage));
        assert_eq!(environment.star_visibility, 1.0);
    }

    #[test]
    fn every_daylight_phase_and_region_profile_is_distinct_and_bounded() {
        let samples = [
            AtmosphereSample {
                humidity: 0.82,
                coldness: 0.30,
                aerosol: 0.06,
                cloudiness: 0.76,
                horizon_warmth: 0.28,
                haze: 0.42,
            },
            AtmosphereSample {
                humidity: 0.30,
                coldness: 0.72,
                aerosol: 0.18,
                cloudiness: 0.42,
                horizon_warmth: 0.20,
                haze: 0.24,
            },
            AtmosphereSample {
                humidity: 0.10,
                coldness: 0.12,
                aerosol: 0.88,
                cloudiness: 0.18,
                horizon_warmth: 0.86,
                haze: 0.78,
            },
        ];
        let mut fingerprints = std::collections::BTreeSet::new();
        for phase in DaylightPhase::ALL {
            assert_eq!(phase.next().next().next().next(), phase);
            for sample in samples {
                let environment = OutdoorEnvironment::for_atmosphere(sample, phase);
                assert!(environment.sun_direction.is_finite());
                assert!((environment.sun_direction.length() - 1.0).abs() < 1.0e-5);
                assert!(environment.sun_direction.y > 0.0);
                assert!(environment.fog_density > 0.0);
                assert!((0.0..=1.0).contains(&environment.cloud_coverage));
                assert!((0.0..=1.0).contains(&environment.star_visibility));
                fingerprints.insert([
                    (environment.sun_radiance.x * 100.0).round() as i32,
                    (environment.sky_horizon.x * 100.0).round() as i32,
                    (environment.sky_zenith.z * 100.0).round() as i32,
                    (environment.fog_density * 10_000.0).round() as i32,
                    (environment.cloud_coverage * 100.0).round() as i32,
                ]);
            }
        }
        assert_eq!(fingerprints.len(), DaylightPhase::ALL.len() * samples.len());
    }

    #[test]
    fn environment_blending_is_frame_rate_independent_and_normalized() {
        let start =
            OutdoorEnvironment::for_atmosphere(AtmosphereSample::default(), DaylightPhase::Dawn);
        let target = OutdoorEnvironment::for_atmosphere(
            AtmosphereSample {
                humidity: 1.0,
                coldness: 1.0,
                aerosol: 1.0,
                cloudiness: 1.0,
                horizon_warmth: 1.0,
                haze: 1.0,
            },
            DaylightPhase::BlueHour,
        );
        let response_seconds = 0.85;
        let advance = |mut environment: OutdoorEnvironment, dt: f32, steps: usize| {
            for _ in 0..steps {
                let amount = 1.0 - (-dt / response_seconds).exp();
                environment = environment.lerp(target, amount);
            }
            environment
        };
        let sixty_hz = advance(start, 1.0 / 60.0, 60);
        let one_twenty_hz = advance(start, 1.0 / 120.0, 120);
        assert!(sixty_hz.sun_direction.dot(one_twenty_hz.sun_direction) > 0.9999);
        assert!((sixty_hz.fog_density - one_twenty_hz.fog_density).abs() < 0.0001);
        assert!((sixty_hz.cloud_coverage - one_twenty_hz.cloud_coverage).abs() < 0.0001);
    }

    #[test]
    fn interior_environment_is_bounded_and_adapts_exposure_separately() {
        let target = InteriorEnvironment::for_enclosure(EnclosureSample {
            sky_visibility: 0.0,
            enclosure: 1.0,
            ceiling_distance_metres: 2.0,
            escape_direction: Vec3::Y,
            escaped_rays: 0,
            ray_count: 9,
        });
        assert_eq!(target.enclosure, 1.0);
        assert!(target.exposure_multiplier > 2.0);
        assert_eq!(target.headlamp_strength, 1.0);
        let advanced = InteriorEnvironment::default().lerp(target, 0.8, 0.1);
        assert!(advanced.enclosure > 0.7);
        assert!(advanced.exposure_multiplier < 1.2);
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
