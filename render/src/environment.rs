//! Shared outdoor-lighting state used by sky, terrain, fog, and shadow projection.

use glam::Vec3;
use voxels_core::EnclosureSample;
use voxels_world::{AtmosphereSample, CelestialModel, CelestialObservation, SurfaceRegion};

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

    pub fn for_solar_position(solar_elevation: f32, solar_hour_angle_radians: f64) -> Self {
        if solar_elevation < -0.12 {
            Self::Night
        } else if solar_hour_angle_radians < 0.0 && solar_elevation < 0.22 {
            Self::Dawn
        } else if solar_hour_angle_radians >= 0.0 && solar_elevation < -0.02 {
            Self::BlueHour
        } else if solar_hour_angle_radians >= 0.0 && solar_elevation < 0.30 {
            Self::GoldenHour
        } else {
            Self::ClearDay
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

#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum WeatherKind {
    #[default]
    Clear,
    Cloudy,
    Overcast,
    Rain,
    Storm,
    Snow,
}

impl WeatherKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Clear => "CLEAR",
            Self::Cloudy => "CLOUDY",
            Self::Overcast => "OVERCAST",
            Self::Rain => "RAIN",
            Self::Storm => "STORM",
            Self::Snow => "SNOW",
        }
    }
}

/// Stable authored points on the continuous weather curve used by local developer controls. The
/// presets change presentation only; cloud advection, seed, and revision remain server-authored.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum WeatherPreset {
    Clear,
    Cloudy,
    Overcast,
    Rain,
    Storm,
}

impl WeatherPreset {
    pub const ALL: [Self; 5] = [
        Self::Clear,
        Self::Cloudy,
        Self::Overcast,
        Self::Rain,
        Self::Storm,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Clear => "CLEAR",
            Self::Cloudy => "CLOUDY",
            Self::Overcast => "OVERCAST",
            Self::Rain => "RAIN",
            Self::Storm => "STORM",
        }
    }

    pub const fn anchor_weather_fraction(self) -> f32 {
        match self {
            Self::Clear => 0.08,
            Self::Cloudy => 0.23,
            Self::Overcast => 0.32,
            Self::Rain => 0.50,
            Self::Storm => 0.68,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DebugEnvironmentOverride {
    pub day_fraction: Option<f32>,
    pub weather_fraction: Option<f32>,
}

impl DebugEnvironmentOverride {
    pub const fn active(self) -> bool {
        self.day_fraction.is_some() || self.weather_fraction.is_some()
    }

    pub fn apply(self, server: WorldEnvironmentState) -> WorldEnvironmentState {
        WorldEnvironmentState {
            day_fraction: self
                .day_fraction
                .filter(|value| value.is_finite())
                .map_or(server.day_fraction, |value| value.rem_euclid(1.0)),
            weather_fraction: self
                .weather_fraction
                .filter(|value| value.is_finite())
                .map_or(server.weather_fraction, |value| value.rem_euclid(1.0)),
            ..server
        }
        .sanitized()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeatherState {
    pub kind: WeatherKind,
    pub coverage: f32,
    pub density: f32,
    pub precipitation: f32,
    pub storminess: f32,
    pub lightning: f32,
    pub snow: f32,
}

impl WeatherState {
    pub fn for_cycle(
        fraction: f32,
        baseline_coverage: f32,
        cycle_seconds: f32,
        weather_seed: u64,
        coldness: f32,
    ) -> Self {
        let fraction = if fraction.is_finite() {
            fraction.rem_euclid(1.0)
        } else {
            0.0
        };
        let overcast = smoothstep(0.16, 0.36, fraction) * (1.0 - smoothstep(0.84, 0.98, fraction));
        let rain = smoothstep(0.34, 0.48, fraction) * (1.0 - smoothstep(0.78, 0.91, fraction));
        let storm = smoothstep(0.56, 0.66, fraction) * (1.0 - smoothstep(0.72, 0.84, fraction));
        let precipitation = rain * scalar_lerp(0.58, 1.0, storm);
        let coverage = scalar_lerp(baseline_coverage.clamp(0.0, 1.0), 0.88, overcast)
            .max(scalar_lerp(0.0, 0.98, storm))
            .clamp(0.0, 1.0);
        let density = (0.28 + overcast * 0.48 + storm * 0.22).clamp(0.0, 1.0);
        let lightning = lightning_flash(fraction, cycle_seconds, weather_seed) * storm;
        let snow = precipitation * smoothstep(0.58, 0.78, coldness);
        let kind = if storm > 0.48 {
            WeatherKind::Storm
        } else if precipitation > 0.12 && coldness > 0.62 {
            WeatherKind::Snow
        } else if precipitation > 0.12 {
            WeatherKind::Rain
        } else if overcast > 0.72 {
            WeatherKind::Overcast
        } else if coverage > baseline_coverage + 0.12 {
            WeatherKind::Cloudy
        } else {
            WeatherKind::Clear
        };
        Self {
            kind,
            coverage,
            density,
            precipitation,
            storminess: storm,
            lightning,
            snow,
        }
    }
}

fn lightning_flash(weather_fraction: f32, weather_cycle_seconds: f32, weather_seed: u64) -> f32 {
    if !weather_cycle_seconds.is_finite() || weather_cycle_seconds <= 0.0 {
        return 0.0;
    }
    let weather_seconds = weather_fraction * weather_cycle_seconds;
    let cell = (weather_seconds / 7.0).floor() as u64;
    let mut value = weather_seed ^ cell.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^= value >> 31;
    let trigger = (value >> 40) as f32 / ((1u32 << 24) - 1) as f32;
    if trigger < 0.91 {
        return 0.0;
    }
    let local = (weather_seconds / 7.0).fract();
    (-local * 34.0).exp() + 0.42 * (-(local - 0.16).abs() * 72.0).exp()
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
    pub cloud_density: f32,
    pub precipitation: f32,
    pub storminess: f32,
    pub lightning: f32,
    pub snow: f32,
    pub star_visibility: f32,
}

/// Dynamic, server-authored environment values evaluated for one render frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldEnvironmentState {
    pub server_time_seconds: f32,
    pub world_days: f64,
    pub day_fraction: f32,
    pub year_fraction: f32,
    pub moon_orbit_fraction: f32,
    pub twinkle_phase: f32,
    pub planet_circumference_metres: f32,
    pub axial_tilt_radians: f32,
    pub moon_orbit_inclination_radians: f32,
    pub celestial_seed: u64,
    pub celestial_revision: u64,
    pub weather_fraction: f32,
    pub weather_cycle_seconds: f32,
    pub cloud_offset_metres: [f32; 2],
    pub cloud_velocity_metres_per_second: [f32; 2],
    pub cloud_coverage: f32,
    pub cloud_base_metres: f32,
    pub cloud_top_metres: f32,
    pub weather_seed: u64,
    pub weather_revision: u64,
}

impl Default for WorldEnvironmentState {
    fn default() -> Self {
        Self {
            server_time_seconds: 0.0,
            world_days: 0.72,
            day_fraction: DaylightPhase::GoldenHour.anchor_day_fraction(),
            year_fraction: 0.0,
            moon_orbit_fraction: 0.5,
            twinkle_phase: 0.0,
            planet_circumference_metres: 40_075_016.0,
            axial_tilt_radians: 23.439_3_f32.to_radians(),
            moon_orbit_inclination_radians: 5.145_f32.to_radians(),
            celestial_seed: 0x57a2_5eed,
            celestial_revision: 1,
            weather_fraction: 0.08,
            weather_cycle_seconds: 900.0,
            cloud_offset_metres: [0.0; 2],
            cloud_velocity_metres_per_second: [0.0; 2],
            cloud_coverage: 0.24,
            cloud_base_metres: 550.0,
            cloud_top_metres: 1_800.0,
            weather_seed: 0,
            weather_revision: 1,
        }
    }
}

impl WorldEnvironmentState {
    pub fn sanitized(self) -> Self {
        let fallback = Self::default();
        let cloud_base_metres =
            finite_positive_scalar(self.cloud_base_metres, fallback.cloud_base_metres);
        let cloud_top_metres =
            finite_positive_scalar(self.cloud_top_metres, fallback.cloud_top_metres)
                .max(cloud_base_metres + 1.0);
        Self {
            server_time_seconds: if self.server_time_seconds.is_finite() {
                self.server_time_seconds.max(0.0)
            } else {
                fallback.server_time_seconds
            },
            world_days: if self.world_days.is_finite() {
                self.world_days
            } else {
                fallback.world_days
            },
            day_fraction: if self.day_fraction.is_finite() {
                self.day_fraction.rem_euclid(1.0)
            } else {
                fallback.day_fraction
            },
            year_fraction: if self.year_fraction.is_finite() {
                self.year_fraction.rem_euclid(1.0)
            } else {
                fallback.year_fraction
            },
            moon_orbit_fraction: if self.moon_orbit_fraction.is_finite() {
                self.moon_orbit_fraction.rem_euclid(1.0)
            } else {
                fallback.moon_orbit_fraction
            },
            twinkle_phase: if self.twinkle_phase.is_finite() {
                self.twinkle_phase.rem_euclid(1.0)
            } else {
                fallback.twinkle_phase
            },
            planet_circumference_metres: finite_positive_scalar(
                self.planet_circumference_metres,
                fallback.planet_circumference_metres,
            ),
            axial_tilt_radians: if self.axial_tilt_radians.is_finite() {
                self.axial_tilt_radians
                    .clamp(0.0, std::f32::consts::FRAC_PI_4)
            } else {
                fallback.axial_tilt_radians
            },
            moon_orbit_inclination_radians: if self.moon_orbit_inclination_radians.is_finite() {
                self.moon_orbit_inclination_radians
                    .clamp(0.0, std::f32::consts::FRAC_PI_6)
            } else {
                fallback.moon_orbit_inclination_radians
            },
            celestial_seed: self.celestial_seed,
            celestial_revision: self.celestial_revision.max(1),
            weather_fraction: if self.weather_fraction.is_finite() {
                self.weather_fraction.rem_euclid(1.0)
            } else {
                fallback.weather_fraction
            },
            weather_cycle_seconds: if self.weather_cycle_seconds.is_finite() {
                self.weather_cycle_seconds.max(0.0)
            } else {
                fallback.weather_cycle_seconds
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
            cloud_base_metres,
            cloud_top_metres,
            weather_seed: self.weather_seed,
            weather_revision: self.weather_revision.max(1),
        }
    }

    pub fn weather(self, coldness: f32) -> WeatherState {
        WeatherState::for_cycle(
            self.weather_fraction,
            self.cloud_coverage,
            self.weather_cycle_seconds,
            self.weather_seed,
            coldness,
        )
    }

    pub fn celestial_observation(self, world_xz_metres: [f64; 2]) -> CelestialObservation {
        let state = self.sanitized();
        CelestialModel {
            planet_circumference_metres: f64::from(state.planet_circumference_metres),
            axial_tilt_radians: f64::from(state.axial_tilt_radians),
            moon_orbit_inclination_radians: f64::from(state.moon_orbit_inclination_radians),
        }
        .observe(
            world_xz_metres,
            f64::from(state.day_fraction),
            f64::from(state.year_fraction),
            f64::from(state.moon_orbit_fraction),
        )
        .expect("sanitized celestial state and finite camera coordinates")
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
        let state = WorldEnvironmentState::default();
        Self::for_celestial(
            AtmosphereSample::default(),
            state.celestial_observation([0.0, 0.0]),
            WeatherState::for_cycle(0.08, 0.24, 900.0, 0, 0.32),
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
            cloud_density: 0.42,
            precipitation: 0.0,
            storminess: 0.0,
            lightning: 0.0,
            snow: 0.0,
            star_visibility: 0.0,
        }
    }

    pub fn for_atmosphere(sample: AtmosphereSample, phase: DaylightPhase) -> Self {
        let state = WorldEnvironmentState {
            day_fraction: phase.anchor_day_fraction(),
            ..WorldEnvironmentState::default()
        };
        Self::for_celestial(
            sample,
            state.celestial_observation([0.0, 0.0]),
            WeatherState::for_cycle(0.08, sample.cloudiness, 900.0, 0, sample.coldness),
        )
    }

    pub fn for_celestial(
        sample: AtmosphereSample,
        celestial: CelestialObservation,
        weather: WeatherState,
    ) -> Self {
        let day_fraction = celestial.local_solar_day_fraction as f32;
        let sun_direction = Vec3::from_array(celestial.sun_direction);
        let moon_direction = Vec3::from_array(celestial.moon_direction);
        let solar_elevation = sun_direction.y;
        let daylight = smoothstep(-0.10, 0.16, solar_elevation);
        let direct_sun = smoothstep(-0.015, 0.10, solar_elevation);
        let night = 1.0 - smoothstep(-0.20, -0.02, solar_elevation);
        let moon_horizon_visibility = smoothstep(-0.01, 0.08, moon_direction.y);
        let moon_day_contrast = scalar_lerp(1.0, 0.20, daylight);
        let moon_visibility = moon_horizon_visibility * moon_day_contrast;
        let horizon_band = 1.0 - smoothstep(0.02, 0.42, solar_elevation.abs());
        let sunset = smoothstep(0.5, 0.8, day_fraction);
        let warm_color = Vec3::new(6.4, 2.65, 1.15).lerp(Vec3::new(6.8, 3.45, 1.55), sunset);
        let daylight_color = warm_color.lerp(
            Vec3::new(4.9, 4.45, 3.85),
            smoothstep(0.08, 0.72, solar_elevation),
        );
        let sun_radiance = daylight_color * direct_sun;
        let moon_radiance =
            Vec3::new(0.10, 0.14, 0.24) * moon_visibility * celestial.moon_illuminated_fraction;
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
            cloud_coverage: weather.coverage,
            cloud_density: weather.density,
            precipitation: weather.precipitation,
            storminess: weather.storminess,
            lightning: weather.lightning,
            snow: weather.snow,
            star_visibility: night,
        };
        let humidity = sample.humidity.clamp(0.0, 1.0);
        let coldness = sample.coldness.clamp(0.0, 1.0);
        let aerosol = sample.aerosol.clamp(0.0, 1.0);
        let haze = sample.haze.clamp(0.0, 1.0);
        let warmth = sample.horizon_warmth.clamp(0.0, 1.0);
        environment.cloud_coverage = (environment.cloud_coverage * 0.95
            + sample.cloudiness.clamp(0.0, 1.0) * 0.05)
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
        let storm_tint = Vec3::new(0.18, 0.22, 0.29);
        environment.sky_horizon = environment
            .sky_horizon
            .lerp(storm_tint * 1.24, weather.storminess * 0.72);
        environment.sky_zenith = environment
            .sky_zenith
            .lerp(storm_tint * 0.54, weather.storminess * 0.78);
        environment.ground_irradiance = environment
            .ground_irradiance
            .lerp(Vec3::new(0.065, 0.075, 0.09), weather.storminess * 0.74);
        let direct_transmittance = (-environment.cloud_coverage * 0.42
            - weather.density * 0.34
            - weather.storminess * 1.18
            - aerosol * 0.16)
            .exp();
        environment.key_light_radiance *= direct_transmittance;
        environment.key_light_radiance += Vec3::splat(weather.lightning * 8.0);
        environment.sky_horizon += Vec3::new(0.46, 0.58, 0.82) * weather.lightning * 1.8;
        environment.sky_zenith += Vec3::new(0.30, 0.38, 0.62) * weather.lightning * 1.2;
        environment.shadow_strength *=
            (1.0 - weather.precipitation * 0.68 - weather.storminess * 0.58).max(0.0);
        environment.fog_density *= 1.0 + weather.precipitation * 1.65 + weather.storminess * 1.25;
        environment.fog_height_falloff *= 1.0 - weather.precipitation * 0.24;
        environment.exposure *= 1.0 + weather.storminess * 0.12;
        environment.star_visibility *=
            (1.0 - environment.cloud_coverage * 0.78) * (1.0 - weather.storminess);
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
            cloud_density: scalar_lerp(self.cloud_density, target.cloud_density, amount),
            precipitation: scalar_lerp(self.precipitation, target.precipitation, amount),
            storminess: scalar_lerp(self.storminess, target.storminess, amount),
            lightning: scalar_lerp(self.lightning, target.lightning, amount),
            snow: scalar_lerp(self.snow, target.snow, amount),
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
            cloud_density: finite_unit_scalar(self.cloud_density, fallback.cloud_density),
            precipitation: finite_unit_scalar(self.precipitation, fallback.precipitation),
            storminess: finite_unit_scalar(self.storminess, fallback.storminess),
            lightning: finite_non_negative_scalar(self.lightning, fallback.lightning),
            snow: finite_unit_scalar(self.snow, fallback.snow),
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

fn finite_positive_scalar(value: f32, fallback: f32) -> f32 {
    if value.is_finite() && value > 0.0 {
        value
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

    fn equatorial_environment(
        sample: AtmosphereSample,
        day_fraction: f32,
        weather: WeatherState,
    ) -> OutdoorEnvironment {
        let state = WorldEnvironmentState {
            day_fraction,
            year_fraction: 0.0,
            ..WorldEnvironmentState::default()
        };
        OutdoorEnvironment::for_celestial(sample, state.celestial_observation([0.0, 0.0]), weather)
    }

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
            cloud_density: f32::NAN,
            precipitation: -1.0,
            storminess: 4.0,
            lightning: f32::NAN,
            snow: f32::NAN,
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
        let weather = WeatherState::for_cycle(0.08, 0.4, 900.0, 7, sample.coldness);
        let midnight = equatorial_environment(sample, 0.0, weather);
        let wrapped = equatorial_environment(sample, 1.0, weather);
        let noon = equatorial_environment(sample, 0.5, weather);
        assert!(midnight.sun_direction.y < 0.0);
        assert!(noon.sun_direction.y > 0.9);
        assert!(midnight.star_visibility > 0.5);
        assert!(noon.star_visibility < 0.01);
        assert!(midnight.sun_direction.dot(wrapped.sun_direction) > 0.999_999);
        assert!((midnight.sky_zenith - wrapped.sky_zenith).length() < 1.0e-5);

        let before = equatorial_environment(sample, 0.9999, weather);
        let after = equatorial_environment(sample, 0.0001, weather);
        assert!(before.sun_direction.dot(after.sun_direction) > 0.999_99);
        assert!((before.sky_horizon - after.sky_horizon).length() < 0.002);

        for minute in 0..1_440 {
            let environment = equatorial_environment(sample, minute as f32 / 1_440.0, weather);
            assert!(environment.key_light_direction.is_finite());
            assert!((environment.key_light_direction.length() - 1.0).abs() < 1.0e-5);
            assert!(environment.key_light_radiance.min_element() >= 0.0);
            assert!((0.0..=1.0).contains(&environment.shadow_strength));
        }
    }

    #[test]
    fn weather_cycle_progresses_continuously_from_clear_through_storm_and_clearing() {
        let clear = WeatherState::for_cycle(0.08, 0.24, 0.0, 7, 0.25);
        let cloudy = WeatherState::for_cycle(0.23, 0.24, 0.0, 7, 0.25);
        let overcast = WeatherState::for_cycle(0.32, 0.24, 0.0, 7, 0.25);
        let rain = WeatherState::for_cycle(0.50, 0.24, 0.0, 7, 0.25);
        let storm = WeatherState::for_cycle(0.68, 0.24, 0.0, 7, 0.25);
        let clearing = WeatherState::for_cycle(0.89, 0.24, 0.0, 7, 0.25);
        assert_eq!(clear.kind, WeatherKind::Clear);
        assert_eq!(cloudy.kind, WeatherKind::Cloudy);
        assert_eq!(overcast.kind, WeatherKind::Overcast);
        assert_eq!(rain.kind, WeatherKind::Rain);
        assert_eq!(storm.kind, WeatherKind::Storm);
        assert!(clearing.coverage < storm.coverage);
        assert!(clear.precipitation < 0.01);
        assert!(rain.precipitation > 0.5);
        assert!(storm.storminess > 0.7);
        assert_eq!(WeatherState::for_cycle(1.08, 0.24, 0.0, 7, 0.25), clear);
    }

    #[test]
    fn debug_override_changes_only_selected_fractions() {
        let server = WorldEnvironmentState {
            server_time_seconds: 91.0,
            day_fraction: 0.14,
            weather_fraction: 0.81,
            weather_cycle_seconds: 900.0,
            cloud_offset_metres: [321.0, -88.0],
            cloud_velocity_metres_per_second: [5.5, 1.6],
            cloud_coverage: 0.24,
            cloud_base_metres: 550.0,
            cloud_top_metres: 1_800.0,
            weather_seed: 77,
            weather_revision: 4,
            ..WorldEnvironmentState::default()
        };
        let effective = DebugEnvironmentOverride {
            day_fraction: Some(DaylightPhase::GoldenHour.anchor_day_fraction()),
            weather_fraction: Some(WeatherPreset::Storm.anchor_weather_fraction()),
        }
        .apply(server);
        assert_eq!(effective.server_time_seconds, server.server_time_seconds);
        assert_eq!(effective.cloud_offset_metres, server.cloud_offset_metres);
        assert_eq!(
            effective.cloud_velocity_metres_per_second,
            server.cloud_velocity_metres_per_second
        );
        assert_eq!(effective.weather_seed, server.weather_seed);
        assert_eq!(effective.weather_revision, server.weather_revision);
        assert_eq!(
            effective.day_fraction,
            DaylightPhase::GoldenHour.anchor_day_fraction()
        );
        assert_eq!(
            effective.weather_fraction,
            WeatherPreset::Storm.anchor_weather_fraction()
        );
        assert_eq!(DebugEnvironmentOverride::default().apply(server), server);
    }

    #[test]
    fn severe_weather_softens_sunlight_and_thickens_air_without_changing_geometry_inputs() {
        let sample = AtmosphereSample::default();
        let clear = equatorial_environment(
            sample,
            0.5,
            WeatherState::for_cycle(0.08, 0.24, 0.0, 7, sample.coldness),
        );
        let storm = equatorial_environment(
            sample,
            0.5,
            WeatherState::for_cycle(0.68, 0.24, 0.0, 7, sample.coldness),
        );
        assert!(storm.key_light_radiance.length() < clear.key_light_radiance.length());
        assert!(storm.shadow_strength < clear.shadow_strength);
        assert!(storm.fog_density > clear.fog_density);
        assert!(storm.cloud_coverage > clear.cloud_coverage);
        assert!(storm.precipitation > clear.precipitation);
    }

    #[test]
    fn cold_precipitation_becomes_snow_without_changing_weather_intensity() {
        let rain = WeatherState::for_cycle(0.50, 0.24, 0.0, 7, 0.2);
        let snow = WeatherState::for_cycle(0.50, 0.24, 0.0, 7, 0.9);
        assert_eq!(rain.kind, WeatherKind::Rain);
        assert_eq!(snow.kind, WeatherKind::Snow);
        assert_eq!(rain.precipitation, snow.precipitation);
        assert!(rain.snow < 0.01);
        assert!(snow.snow > 0.8 * snow.precipitation);
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
