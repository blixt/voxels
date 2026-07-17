//! Familiar celestial coordinates projected onto the world's infinite planar voxel coordinates.
//!
//! Terrain remains an infinite plane. Only the observer frame repeats as if that plane were the
//! universal cover of a sphere: +X travels east and -Z travels north. Continuing through a pole
//! transports the local frame smoothly toward the opposite equator instead of wrapping terrain.

use std::f64::consts::{PI, TAU};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlanetaryCoordinates {
    /// Continuous meridional angle. Its sine is geographic latitude; retaining the raw angle makes
    /// the transported observer frame continuous while crossing either pole.
    pub meridian_angle_radians: f64,
    /// Periodic eastward angle before the diagnostic pole reflection.
    pub base_longitude_radians: f64,
    /// Canonical diagnostic latitude in `-PI/2..=PI/2`.
    pub latitude_radians: f64,
    /// Canonical diagnostic longitude in `-PI..PI`.
    pub longitude_radians: f64,
}

impl PlanetaryCoordinates {
    pub fn from_world_xz_metres(
        world_xz_metres: [f64; 2],
        circumference_metres: f64,
    ) -> Option<Self> {
        if !world_xz_metres.into_iter().all(f64::is_finite)
            || !circumference_metres.is_finite()
            || circumference_metres <= 0.0
        {
            return None;
        }
        let east_turn = (world_xz_metres[0] / circumference_metres).rem_euclid(1.0);
        let north_turn = (-world_xz_metres[1] / circumference_metres).rem_euclid(1.0);
        let base_longitude_radians = east_turn * TAU;
        let meridian_angle_radians = north_turn * TAU;
        let latitude_radians = meridian_angle_radians.sin().asin();
        let pole_reflection = if meridian_angle_radians.cos() < 0.0 {
            PI
        } else {
            0.0
        };
        let longitude_radians = wrap_signed_radians(base_longitude_radians + pole_reflection);
        Some(Self {
            meridian_angle_radians,
            base_longitude_radians,
            latitude_radians,
            longitude_radians,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CelestialModel {
    pub planet_circumference_metres: f64,
    pub axial_tilt_radians: f64,
    pub moon_orbit_inclination_radians: f64,
}

impl CelestialModel {
    pub fn observe(
        self,
        world_xz_metres: [f64; 2],
        day_fraction: f64,
        year_fraction: f64,
        moon_orbit_fraction: f64,
    ) -> Option<CelestialObservation> {
        if !self.axial_tilt_radians.is_finite()
            || !self.moon_orbit_inclination_radians.is_finite()
            || !day_fraction.is_finite()
            || !year_fraction.is_finite()
            || !moon_orbit_fraction.is_finite()
        {
            return None;
        }
        let coordinates = PlanetaryCoordinates::from_world_xz_metres(
            world_xz_metres,
            self.planet_circumference_metres,
        )?;
        let day_fraction = day_fraction.rem_euclid(1.0);
        let year_fraction = year_fraction.rem_euclid(1.0);
        let moon_orbit_fraction = moon_orbit_fraction.rem_euclid(1.0);
        let solar_ecliptic_longitude = TAU * year_fraction;
        let sun_equatorial =
            ecliptic_to_equatorial(solar_ecliptic_longitude, 0.0, self.axial_tilt_radians);
        let (sun_right_ascension, sun_declination) =
            right_ascension_and_declination(sun_equatorial);
        let solar_hour_angle = TAU * (day_fraction - 0.5) + coordinates.base_longitude_radians;
        let local_sidereal_angle = wrap_unsigned_radians(sun_right_ascension + solar_hour_angle);
        let sun_direction = horizon_direction(
            coordinates.meridian_angle_radians,
            solar_hour_angle,
            sun_declination,
        );
        let canonical_solar_hour_angle = solar_hour_angle
            + if coordinates.meridian_angle_radians.cos() < 0.0 {
                PI
            } else {
                0.0
            };

        let lunar_ecliptic_longitude = TAU * moon_orbit_fraction;
        let lunar_ecliptic_latitude =
            self.moon_orbit_inclination_radians * lunar_ecliptic_longitude.sin();
        let moon_equatorial = ecliptic_to_equatorial(
            lunar_ecliptic_longitude,
            lunar_ecliptic_latitude,
            self.axial_tilt_radians,
        );
        let (moon_right_ascension, moon_declination) =
            right_ascension_and_declination(moon_equatorial);
        let lunar_hour_angle = wrap_signed_radians(local_sidereal_angle - moon_right_ascension);
        let moon_direction = horizon_direction(
            coordinates.meridian_angle_radians,
            lunar_hour_angle,
            moon_declination,
        );

        let (sidereal_sine, sidereal_cosine) = local_sidereal_angle.sin_cos();
        let (meridian_sine, meridian_cosine) = coordinates.meridian_angle_radians.sin_cos();
        let equatorial_east = [-sidereal_sine, sidereal_cosine, 0.0];
        let equatorial_up = [
            meridian_cosine * sidereal_cosine,
            meridian_cosine * sidereal_sine,
            meridian_sine,
        ];
        let equatorial_north = [
            -meridian_sine * sidereal_cosine,
            -meridian_sine * sidereal_sine,
            meridian_cosine,
        ];
        let moon_illuminated_fraction =
            ((1.0 - dot(sun_direction, moon_direction)) * 0.5).clamp(0.0, 1.0);

        Some(CelestialObservation {
            coordinates,
            local_solar_day_fraction: (canonical_solar_hour_angle / TAU + 0.5).rem_euclid(1.0),
            year_fraction,
            moon_orbit_fraction,
            solar_hour_angle_radians: wrap_signed_radians(canonical_solar_hour_angle),
            local_sidereal_angle_radians: local_sidereal_angle,
            sun_direction: to_f32(sun_direction),
            moon_direction: to_f32(moon_direction),
            equatorial_east: to_f32(equatorial_east),
            equatorial_up: to_f32(equatorial_up),
            equatorial_north: to_f32(equatorial_north),
            moon_illuminated_fraction: moon_illuminated_fraction as f32,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CelestialObservation {
    pub coordinates: PlanetaryCoordinates,
    pub local_solar_day_fraction: f64,
    pub year_fraction: f64,
    pub moon_orbit_fraction: f64,
    pub solar_hour_angle_radians: f64,
    pub local_sidereal_angle_radians: f64,
    /// World-space directions use +X east, +Y up, and -Z north.
    pub sun_direction: [f32; 3],
    pub moon_direction: [f32; 3],
    /// Equatorial coordinates of the local east, up, and north basis directions.
    pub equatorial_east: [f32; 3],
    pub equatorial_up: [f32; 3],
    pub equatorial_north: [f32; 3],
    pub moon_illuminated_fraction: f32,
}

fn ecliptic_to_equatorial(longitude: f64, latitude: f64, tilt: f64) -> [f64; 3] {
    let (longitude_sine, longitude_cosine) = longitude.sin_cos();
    let (latitude_sine, latitude_cosine) = latitude.sin_cos();
    let (tilt_sine, tilt_cosine) = tilt.sin_cos();
    let x = latitude_cosine * longitude_cosine;
    let ecliptic_y = latitude_cosine * longitude_sine;
    let ecliptic_z = latitude_sine;
    normalize([
        x,
        ecliptic_y * tilt_cosine - ecliptic_z * tilt_sine,
        ecliptic_y * tilt_sine + ecliptic_z * tilt_cosine,
    ])
}

fn right_ascension_and_declination(direction: [f64; 3]) -> (f64, f64) {
    (
        direction[1].atan2(direction[0]),
        direction[2].clamp(-1.0, 1.0).asin(),
    )
}

fn horizon_direction(meridian_angle: f64, hour_angle: f64, declination: f64) -> [f64; 3] {
    let (latitude_sine, latitude_cosine) = meridian_angle.sin_cos();
    let (hour_sine, hour_cosine) = hour_angle.sin_cos();
    let (declination_sine, declination_cosine) = declination.sin_cos();
    let east = -declination_cosine * hour_sine;
    let north =
        latitude_cosine * declination_sine - latitude_sine * declination_cosine * hour_cosine;
    let up = latitude_sine * declination_sine + latitude_cosine * declination_cosine * hour_cosine;
    normalize([east, up, -north])
}

fn normalize(vector: [f64; 3]) -> [f64; 3] {
    let length = dot(vector, vector).sqrt();
    if length > 0.0 && length.is_finite() {
        [vector[0] / length, vector[1] / length, vector[2] / length]
    } else {
        [0.0, 1.0, 0.0]
    }
}

fn dot(left: [f64; 3], right: [f64; 3]) -> f64 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn to_f32(value: [f64; 3]) -> [f32; 3] {
    [value[0] as f32, value[1] as f32, value[2] as f32]
}

fn wrap_unsigned_radians(value: f64) -> f64 {
    value.rem_euclid(TAU)
}

fn wrap_signed_radians(value: f64) -> f64 {
    (value + PI).rem_euclid(TAU) - PI
}

#[cfg(test)]
mod tests {
    use super::*;

    const EARTH_CIRCUMFERENCE: f64 = 40_075_016.686;
    const EARTH_TILT: f64 = 23.439_3_f64.to_radians();
    const LUNAR_INCLINATION: f64 = 5.145_f64.to_radians();

    fn model() -> CelestialModel {
        CelestialModel {
            planet_circumference_metres: EARTH_CIRCUMFERENCE,
            axial_tilt_radians: EARTH_TILT,
            moon_orbit_inclination_radians: LUNAR_INCLINATION,
        }
    }

    fn observation(world_xz: [f64; 2], day: f64, year: f64, moon: f64) -> CelestialObservation {
        model()
            .observe(world_xz, day, year, moon)
            .expect("valid observation")
    }

    fn direction_dot(left: [f32; 3], right: [f32; 3]) -> f32 {
        left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
    }

    #[test]
    fn planar_cover_reflects_at_poles_and_repeats_after_one_circumference() {
        let quarter = EARTH_CIRCUMFERENCE * 0.25;
        let origin =
            PlanetaryCoordinates::from_world_xz_metres([0.0, 0.0], EARTH_CIRCUMFERENCE).unwrap();
        let north =
            PlanetaryCoordinates::from_world_xz_metres([0.0, -quarter], EARTH_CIRCUMFERENCE)
                .unwrap();
        let antipode =
            PlanetaryCoordinates::from_world_xz_metres([0.0, -quarter * 2.0], EARTH_CIRCUMFERENCE)
                .unwrap();
        let south =
            PlanetaryCoordinates::from_world_xz_metres([0.0, -quarter * 3.0], EARTH_CIRCUMFERENCE)
                .unwrap();
        let repeated = PlanetaryCoordinates::from_world_xz_metres(
            [EARTH_CIRCUMFERENCE, -EARTH_CIRCUMFERENCE],
            EARTH_CIRCUMFERENCE,
        )
        .unwrap();
        assert!(origin.latitude_radians.abs() < 1.0e-12);
        assert!((north.latitude_radians - PI * 0.5).abs() < 1.0e-12);
        assert!(antipode.latitude_radians.abs() < 1.0e-12);
        assert!((antipode.longitude_radians.abs() - PI).abs() < 1.0e-12);
        assert!((south.latitude_radians + PI * 0.5).abs() < 1.0e-12);
        assert!(repeated.latitude_radians.abs() < 1.0e-12);
        assert!(repeated.longitude_radians.abs() < 1.0e-12);
    }

    #[test]
    fn reflected_equator_reports_the_opposite_local_solar_time() {
        let antipode = observation([0.0, -EARTH_CIRCUMFERENCE * 0.5], 0.5, 0.0, 0.0);
        assert!(antipode.local_solar_day_fraction.abs() < 1.0e-12);
        assert!(antipode.sun_direction[1] < -0.999);
    }

    #[test]
    fn equinox_sun_uses_familiar_east_up_west_daily_arc() {
        let dawn = observation([0.0, 0.0], 0.25, 0.0, 0.0);
        let noon = observation([0.0, 0.0], 0.5, 0.0, 0.0);
        let dusk = observation([0.0, 0.0], 0.75, 0.0, 0.0);
        let midnight = observation([0.0, 0.0], 0.0, 0.0, 0.0);
        assert!(dawn.sun_direction[0] > 0.999);
        assert!(noon.sun_direction[1] > 0.999);
        assert!(dusk.sun_direction[0] < -0.999);
        assert!(midnight.sun_direction[1] < -0.999);
    }

    #[test]
    fn seasons_change_high_latitude_solar_elevation() {
        let north_metres = EARTH_CIRCUMFERENCE / 6.0;
        let summer = observation([0.0, -north_metres], 0.5, 0.25, 0.0);
        let winter = observation([0.0, -north_metres], 0.5, 0.75, 0.0);
        let summer_altitude = summer.sun_direction[1].asin().to_degrees();
        let winter_altitude = winter.sun_direction[1].asin().to_degrees();
        assert!((summer_altitude - 53.439_3).abs() < 0.01);
        assert!((winter_altitude - 6.560_7).abs() < 0.01);
    }

    #[test]
    fn transported_sky_is_continuous_across_both_poles() {
        let pole = EARTH_CIRCUMFERENCE * 0.25;
        let epsilon = 0.01;
        for z in [-pole, -pole * 3.0] {
            let before = observation([1_000.0, z + epsilon], 0.67, 0.31, 0.52);
            let after = observation([1_000.0, z - epsilon], 0.67, 0.31, 0.52);
            assert!(direction_dot(before.sun_direction, after.sun_direction) > 0.999_999);
            assert!(direction_dot(before.moon_direction, after.moon_direction) > 0.999_999);
            assert!(direction_dot(before.equatorial_up, after.equatorial_up) > 0.999_999);
        }
    }

    #[test]
    fn celestial_basis_is_orthonormal_across_the_planar_cover() {
        for x in [
            -EARTH_CIRCUMFERENCE,
            -1.0,
            0.0,
            7_500_000.0,
            EARTH_CIRCUMFERENCE,
        ] {
            for z in [-EARTH_CIRCUMFERENCE, -10_018_754.0, 0.0, 10_018_754.0] {
                let value = observation([x, z], 0.42, 0.19, 0.63);
                for basis in [
                    value.equatorial_east,
                    value.equatorial_up,
                    value.equatorial_north,
                ] {
                    assert!((direction_dot(basis, basis) - 1.0).abs() < 1.0e-5);
                }
                assert!(direction_dot(value.equatorial_east, value.equatorial_up).abs() < 1.0e-5);
                assert!(
                    direction_dot(value.equatorial_east, value.equatorial_north).abs() < 1.0e-5
                );
                assert!(direction_dot(value.equatorial_up, value.equatorial_north).abs() < 1.0e-5);
            }
        }
    }

    #[test]
    fn lunar_orbit_produces_new_quarter_and_full_phases() {
        let new = observation([0.0, 0.0], 0.5, 0.0, 0.0);
        let quarter = observation([0.0, 0.0], 0.5, 0.0, 0.25);
        let full = observation([0.0, 0.0], 0.5, 0.0, 0.5);
        assert!(new.moon_illuminated_fraction < 0.01);
        assert!((quarter.moon_illuminated_fraction - 0.5).abs() < 0.05);
        assert!(full.moon_illuminated_fraction > 0.99);
    }

    #[test]
    fn invalid_or_extreme_inputs_fail_closed_or_remain_finite() {
        assert!(PlanetaryCoordinates::from_world_xz_metres([0.0, 0.0], 0.0).is_none());
        assert!(
            PlanetaryCoordinates::from_world_xz_metres([f64::NAN, 0.0], EARTH_CIRCUMFERENCE)
                .is_none()
        );
        let edge_metres = f64::from(i32::MAX) * 0.1;
        let edge = observation([edge_metres, -edge_metres], 1.0e12, -1.0e12, 1.0e12);
        assert!(edge.sun_direction.into_iter().all(f32::is_finite));
        assert!(edge.moon_direction.into_iter().all(f32::is_finite));
    }
}
