//! Shared outdoor-lighting state used by sky, terrain, fog, and shadow projection.

use glam::Vec3;
use voxels_core::EnclosureSample;
use voxels_world::{AtmosphereSample, SurfaceRegion};

#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DaylightPhase {
    Night,
    Dawn,
    ClearDay,
    #[default]
    GoldenHour,
    BlueHour,
}

impl DaylightPhase {
    pub const ALL: [Self; 5] = [
        Self::Night,
        Self::Dawn,
        Self::ClearDay,
        Self::GoldenHour,
        Self::BlueHour,
    ];

    pub const fn next(self) -> Self {
        match self {
            Self::Night => Self::Dawn,
            Self::Dawn => Self::ClearDay,
            Self::ClearDay => Self::GoldenHour,
            Self::GoldenHour => Self::BlueHour,
            Self::BlueHour => Self::Night,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Night => "NIGHT",
            Self::Dawn => "DAWN",
            Self::ClearDay => "CLEAR DAY",
            Self::GoldenHour => "GOLDEN HOUR",
            Self::BlueHour => "BLUE HOUR",
        }
    }

    pub const fn anchor_day_fraction(self) -> f32 {
        match self {
            Self::Night => 0.0,
            Self::Dawn => 0.235,
            Self::ClearDay => 0.5,
            Self::GoldenHour => 0.72,
            Self::BlueHour => 0.80,
        }
    }

    pub fn for_day_fraction(day_fraction: f32) -> Self {
        let day_fraction = day_fraction.rem_euclid(1.0);
        if !(0.18..0.84).contains(&day_fraction) {
            Self::Night
        } else if day_fraction < 0.29 {
            Self::Dawn
        } else if day_fraction < 0.66 {
            Self::ClearDay
        } else if day_fraction < 0.77 {
            Self::GoldenHour
        } else {
            Self::BlueHour
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
    pub key_light_direction: Vec3,
    pub key_light_radiance: Vec3,
    pub sun_direction: Vec3,
    pub moon_direction: Vec3,
    pub sun_visibility: f32,
    pub moon_visibility: f32,
    pub shadow_strength: f32,
    pub sky_horizon: Vec3,
    pub sky_zenith: Vec3,
    pub ground_irradiance: Vec3,
    pub fog_density: f32,
    pub fog_height_falloff: f32,
    pub exposure: f32,
    pub cloud_coverage: f32,
    pub star_visibility: f32,
}

/// Dynamic, server-authored environment values evaluated for one render frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldEnvironmentState {
    pub day_fraction: f32,
    pub cloud_offset_metres: [f32; 2],
    pub cloud_velocity_metres_per_second: [f32; 2],
    pub cloud_coverage: f32,
    pub weather_seed: u64,
    pub weather_revision: u64,
}

impl Default for WorldEnvironmentState {
    fn default() -> Self {
        Self {
            day_fraction: DaylightPhase::GoldenHour.anchor_day_fraction(),
            cloud_offset_metres: [0.0; 2],
            cloud_velocity_metres_per_second: [0.0; 2],
            cloud_coverage: 0.42,
            weather_seed: 0,
            weather_revision: 1,
        }
    }
}

impl WorldEnvironmentState {
    pub fn sanitized(self) -> Self {
        let fallback = Self::default();
        Self {
            day_fraction: if self.day_fraction.is_finite() {
                self.day_fraction.rem_euclid(1.0)
            } else {
                fallback.day_fraction
            },
            cloud_offset_metres: std::array::from_fn(|axis| {
                if self.cloud_offset_metres[axis].is_finite() {
                    self.cloud_offset_metres[axis]
                } else {
                    fallback.cloud_offset_metres[axis]
                }
            }),
            cloud_velocity_metres_per_second: std::array::from_fn(|axis| {
                if self.cloud_velocity_metres_per_second[axis].is_finite() {
                    self.cloud_velocity_metres_per_second[axis]
                } else {
                    fallback.cloud_velocity_metres_per_second[axis]
                }
            }),
            cloud_coverage: finite_unit_scalar(self.cloud_coverage, fallback.cloud_coverage),
            weather_seed: self.weather_seed,
            weather_revision: self.weather_revision.max(1),
        }
    }
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
        // Directional terrain probes can report a small enclosure beside a hill or tree even
        // while the player is plainly outdoors. Applying cave extinction from the first non-zero
        // sample makes distant terrain exponentially darken as that tiny value toggles. Reserve
        // interior exposure and air extinction for meaningful enclosure, with a smooth entrance
        // band so walking into an actual cave still adapts progressively.
        let interior = smoothstep(0.28, 0.72, enclosure);
        Self {
            enclosure,
            exposure_multiplier: 1.0 + interior * 1.15,
            fog_density: interior * 0.032,
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

fn smoothstep(start: f32, end: f32, value: f32) -> f32 {
    let normalized = ((value - start) / (end - start)).clamp(0.0, 1.0);
    normalized * normalized * (3.0 - 2.0 * normalized)
}

impl Default for OutdoorEnvironment {
    fn default() -> Self {
        Self::for_world_time(
            AtmosphereSample::default(),
            DaylightPhase::GoldenHour.anchor_day_fraction(),
            0.42,
        )
    }
}

impl OutdoorEnvironment {
    fn fallback() -> Self {
        let sun_direction = Vec3::new(0.48, 0.72, 0.35).normalize();
        Self {
            key_light_direction: sun_direction,
            key_light_radiance: Vec3::new(4.8, 4.25, 3.55),
            sun_direction,
            moon_direction: -sun_direction,
            sun_visibility: 1.0,
            moon_visibility: 0.0,
            shadow_strength: 1.0,
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

    pub fn for_atmosphere(sample: AtmosphereSample, phase: DaylightPhase) -> Self {
        Self::for_world_time(sample, phase.anchor_day_fraction(), sample.cloudiness)
    }

    pub fn for_world_time(
        sample: AtmosphereSample,
        day_fraction: f32,
        weather_cloud_coverage: f32,
    ) -> Self {
        let day_fraction = if day_fraction.is_finite() {
            day_fraction.rem_euclid(1.0)
        } else {
            0.5
        };
        let orbit = std::f32::consts::TAU * (day_fraction - 0.25);
        // A small northward declination keeps noon shadows legible instead of putting the sun
        // directly overhead, while the analytic orbit remains continuous through midnight.
        let sun_direction =
            Vec3::new(orbit.cos() * 0.86, orbit.sin(), orbit.cos() * 0.51 + 0.22).normalize();
        let moon_direction = (-sun_direction + Vec3::new(0.10, 0.0, -0.06)).normalize();
        let solar_elevation = sun_direction.y;
        let daylight = smoothstep(-0.10, 0.16, solar_elevation);
        let direct_sun = smoothstep(-0.015, 0.10, solar_elevation);
        let night = 1.0 - smoothstep(-0.20, -0.02, solar_elevation);
        let moon_visibility = night * smoothstep(0.03, 0.20, moon_direction.y);
        let horizon_band = 1.0 - smoothstep(0.02, 0.42, solar_elevation.abs());
        let sunset = smoothstep(0.5, 0.8, day_fraction);
        let warm_color = Vec3::new(6.4, 2.65, 1.15).lerp(Vec3::new(6.8, 3.45, 1.55), sunset);
        let daylight_color = warm_color.lerp(
            Vec3::new(4.9, 4.45, 3.85),
            smoothstep(0.08, 0.72, solar_elevation),
        );
        let sun_radiance = daylight_color * direct_sun;
        let moon_radiance = Vec3::new(0.10, 0.14, 0.24) * moon_visibility;
        let (key_light_direction, key_light_radiance, shadow_strength) = if direct_sun > 0.025 {
            (
                sun_direction,
                sun_radiance,
                smoothstep(0.035, 0.14, solar_elevation),
            )
        } else {
            (moon_direction, moon_radiance, 0.0)
        };
        let warm_horizon = Vec3::new(0.78, 0.31, 0.14).lerp(Vec3::new(0.86, 0.48, 0.24), sunset);
        let night_horizon = Vec3::new(0.010, 0.016, 0.036);
        let night_zenith = Vec3::new(0.0015, 0.004, 0.018);
        let day_horizon = Vec3::new(0.58, 0.72, 0.88);
        let day_zenith = Vec3::new(0.055, 0.20, 0.56);
        let twilight_amount = horizon_band * smoothstep(-0.16, 0.10, solar_elevation);
        let mut environment = Self {
            key_light_direction,
            key_light_radiance,
            sun_direction,
            moon_direction,
            sun_visibility: direct_sun,
            moon_visibility,
            shadow_strength,
            sky_horizon: night_horizon
                .lerp(day_horizon, daylight)
                .lerp(warm_horizon, twilight_amount * 0.62),
            sky_zenith: night_zenith
                .lerp(day_zenith, daylight)
                .lerp(Vec3::new(0.018, 0.036, 0.10), twilight_amount * 0.38),
            ground_irradiance: Vec3::new(0.010, 0.014, 0.028)
                .lerp(Vec3::new(0.16, 0.19, 0.16), daylight)
                .lerp(Vec3::new(0.18, 0.105, 0.065), twilight_amount * 0.34),
            fog_density: scalar_lerp(0.018, 0.012, daylight),
            fog_height_falloff: scalar_lerp(0.058, 0.075, daylight),
            exposure: scalar_lerp(1.42, 1.0, daylight),
            cloud_coverage: weather_cloud_coverage.clamp(0.0, 1.0),
            star_visibility: night,
        };
        let humidity = sample.humidity.clamp(0.0, 1.0);
        let coldness = sample.coldness.clamp(0.0, 1.0);
        let aerosol = sample.aerosol.clamp(0.0, 1.0);
        let haze = sample.haze.clamp(0.0, 1.0);
        let warmth = sample.horizon_warmth.clamp(0.0, 1.0);
        environment.cloud_coverage = (environment.cloud_coverage * 0.72
            + sample.cloudiness.clamp(0.0, 1.0) * 0.28)
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
        environment.key_light_radiance *= 1.0 - environment.cloud_coverage * 0.18 - aerosol * 0.16;
        environment.star_visibility *= 1.0 - environment.cloud_coverage * 0.78;
        environment.sanitized()
    }

    pub fn lerp(self, target: Self, amount: f32) -> Self {
        let amount = amount.clamp(0.0, 1.0);
        let key_direction = self
            .key_light_direction
            .lerp(target.key_light_direction, amount);
        let sun_direction = self.sun_direction.lerp(target.sun_direction, amount);
        let moon_direction = self.moon_direction.lerp(target.moon_direction, amount);
        Self {
            key_light_direction: if key_direction.length_squared() > 0.0001 {
                key_direction.normalize()
            } else {
                target.key_light_direction
            },
            key_light_radiance: self
                .key_light_radiance
                .lerp(target.key_light_radiance, amount),
            sun_direction: normalized_or(sun_direction, target.sun_direction),
            moon_direction: normalized_or(moon_direction, target.moon_direction),
            sun_visibility: scalar_lerp(self.sun_visibility, target.sun_visibility, amount),
            moon_visibility: scalar_lerp(self.moon_visibility, target.moon_visibility, amount),
            shadow_strength: scalar_lerp(self.shadow_strength, target.shadow_strength, amount),
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
        let fallback = Self::fallback();
        Self {
            key_light_direction: normalized_or(
                self.key_light_direction,
                fallback.key_light_direction,
            ),
            key_light_radiance: finite_non_negative(
                self.key_light_radiance,
                fallback.key_light_radiance,
            ),
            sun_direction: normalized_or(self.sun_direction, fallback.sun_direction),
            moon_direction: normalized_or(self.moon_direction, fallback.moon_direction),
            sun_visibility: finite_unit_scalar(self.sun_visibility, fallback.sun_visibility),
            moon_visibility: finite_unit_scalar(self.moon_visibility, fallback.moon_visibility),
            shadow_strength: finite_unit_scalar(self.shadow_strength, fallback.shadow_strength),
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

fn normalized_or(value: Vec3, fallback: Vec3) -> Vec3 {
    if value.is_finite() && value.length_squared() > 0.0 {
        value.normalize()
    } else {
        fallback
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
        assert!((environment.key_light_direction.length() - 1.0).abs() < 1.0e-6);
        assert!((environment.sun_direction.length() - 1.0).abs() < 1.0e-6);
        assert!(environment.sun_direction.y > 0.0);
        assert!(environment.key_light_radiance.min_element() > 0.0);
        assert!(environment.sky_horizon.min_element() >= 0.0);
        assert!(environment.sky_zenith.min_element() >= 0.0);
        assert!(environment.fog_density > 0.0);
        assert!(environment.exposure > 0.0);
    }

    #[test]
    fn invalid_parameters_fall_back_without_nan_reaching_the_gpu() {
        let environment = OutdoorEnvironment {
            key_light_direction: Vec3::ZERO,
            key_light_radiance: Vec3::splat(f32::NAN),
            sun_direction: Vec3::ZERO,
            moon_direction: Vec3::splat(f32::NAN),
            sun_visibility: f32::NAN,
            moon_visibility: -1.0,
            shadow_strength: 4.0,
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
        assert!(environment.key_light_direction.is_finite());
        assert!(environment.sun_direction.is_finite());
        assert!(environment.moon_direction.is_finite());
        assert!(environment.key_light_radiance.is_finite());
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
            assert_eq!(
                (0..DaylightPhase::ALL.len()).fold(phase, |value, _| value.next()),
                phase
            );
            for sample in samples {
                let environment = OutdoorEnvironment::for_atmosphere(sample, phase);
                assert!(environment.sun_direction.is_finite());
                assert!((environment.sun_direction.length() - 1.0).abs() < 1.0e-5);
                assert!(environment.fog_density > 0.0);
                assert!((0.0..=1.0).contains(&environment.cloud_coverage));
                assert!((0.0..=1.0).contains(&environment.star_visibility));
                fingerprints.insert([
                    (environment.key_light_radiance.x * 100.0).round() as i32,
                    (environment.sky_horizon.x * 100.0).round() as i32,
                    (environment.sky_zenith.z * 100.0).round() as i32,
                    (environment.fog_density * 10_000.0).round() as i32,
                    (environment.cloud_coverage * 100.0).round() as i32,
                    (environment.star_visibility * 100.0).round() as i32,
                    (environment.sun_direction.y * 100.0).round() as i32,
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
        assert!(
            sixty_hz
                .key_light_direction
                .dot(one_twenty_hz.key_light_direction)
                > 0.9999
        );
        assert!((sixty_hz.fog_density - one_twenty_hz.fog_density).abs() < 0.0001);
        assert!((sixty_hz.cloud_coverage - one_twenty_hz.cloud_coverage).abs() < 0.0001);
    }

    #[test]
    fn full_day_orbit_is_continuous_periodic_and_has_a_real_night() {
        let sample = AtmosphereSample::default();
        let midnight = OutdoorEnvironment::for_world_time(sample, 0.0, 0.4);
        let wrapped = OutdoorEnvironment::for_world_time(sample, 1.0, 0.4);
        let noon = OutdoorEnvironment::for_world_time(sample, 0.5, 0.4);
        assert!(midnight.sun_direction.y < 0.0);
        assert!(noon.sun_direction.y > 0.9);
        assert!(midnight.star_visibility > 0.5);
        assert!(noon.star_visibility < 0.01);
        assert!(midnight.sun_direction.dot(wrapped.sun_direction) > 0.999_999);
        assert!((midnight.sky_zenith - wrapped.sky_zenith).length() < 1.0e-5);

        let before = OutdoorEnvironment::for_world_time(sample, 0.9999, 0.4);
        let after = OutdoorEnvironment::for_world_time(sample, 0.0001, 0.4);
        assert!(before.sun_direction.dot(after.sun_direction) > 0.999_99);
        assert!((before.sky_horizon - after.sky_horizon).length() < 0.002);

        for minute in 0..1_440 {
            let environment =
                OutdoorEnvironment::for_world_time(sample, minute as f32 / 1_440.0, 0.4);
            assert!(environment.key_light_direction.is_finite());
            assert!((environment.key_light_direction.length() - 1.0).abs() < 1.0e-5);
            assert!(environment.key_light_radiance.min_element() >= 0.0);
            assert!((0.0..=1.0).contains(&environment.shadow_strength));
        }
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
    fn one_blocked_outdoor_probe_ray_does_not_create_global_cave_fog() {
        let outdoors = InteriorEnvironment::for_enclosure(EnclosureSample {
            sky_visibility: 8.0 / 9.0,
            enclosure: 1.0 / 9.0,
            ceiling_distance_metres: 12.0,
            escape_direction: Vec3::Y,
            escaped_rays: 8,
            ray_count: 9,
        });
        assert_eq!(outdoors.enclosure, 1.0 / 9.0);
        assert_eq!(outdoors.exposure_multiplier, 1.0);
        assert_eq!(outdoors.fog_density, 0.0);
        assert_eq!(outdoors.headlamp_strength, 0.0);
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
