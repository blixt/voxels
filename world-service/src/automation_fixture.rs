//! Typed configuration overlays for isolated automation worlds.
//!
//! Automation starts from the same checked-in configuration as normal play, changes only the
//! requested fields, then serializes through the production client and service config types.

use crate::{
    PRESENCE_WEBSOCKET_PATH, WORLD_WEBSOCKET_PATH, WORLD_WEBSOCKET_PROTOCOL, WorldServiceConfig,
    WorldServiceConfigError, WorldSourceMode,
};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;
use voxels_client_config::{ClientConfig, ClientConfigError};

pub const AUTOMATION_FIXTURE_SCHEMA_VERSION: u32 = 8;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AutomationFixtureOverlay {
    pub schema_version: u32,
    pub browser_port: u16,
    pub backend_port: u16,
    pub client_ports: Vec<u16>,
    pub auth_token: String,
    pub source: WorldSourceMode,
    pub spawn_voxels: Option<[i32; 2]>,
    pub spawn_pillar_height_voxels: Option<u16>,
    pub spawn_pillar_radius_voxels: Option<u8>,
    pub spawn_protection_radius_voxels: Option<u16>,
    pub generation_workers: Option<u16>,
    pub generation_workers_per_client: Option<u16>,
    pub cascaded_shadows: Option<bool>,
    pub screen_space_ambient_occlusion: Option<bool>,
    pub lod_boundary_half_extents_voxels: Option<[u32; 8]>,
    pub diagnostic_sky_rgb: Option<[u8; 3]>,
    pub profiling_warmup_seconds: Option<f32>,
    pub profiling_measure_seconds: Option<f32>,
    pub day_length_seconds: Option<f32>,
    pub world_day_number_at_unix_epoch: Option<i64>,
    pub day_fraction_at_unix_epoch: Option<f32>,
    pub days_per_year: Option<f32>,
    pub moon_sidereal_orbit_days: Option<f32>,
    pub moon_orbit_phase_at_world_epoch: Option<f32>,
    pub planet_circumference_metres: Option<f32>,
    pub axial_tilt_degrees: Option<f32>,
    pub moon_orbit_inclination_degrees: Option<f32>,
    pub celestial_seed: Option<u64>,
    pub celestial_revision: Option<u64>,
    pub weather_cycle_seconds: Option<f32>,
    pub weather_fraction_at_unix_epoch: Option<f32>,
    pub cloud_velocity_metres_per_second: Option<[f32; 2]>,
    pub cloud_coverage: Option<f32>,
    pub cloud_base_metres: Option<f32>,
    pub cloud_top_metres: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AutomationFixtureResolved {
    pub spawn_voxels: [i32; 2],
    pub spawn_pillar_height_voxels: u16,
    pub spawn_pillar_radius_voxels: u8,
    pub spawn_protection_radius_voxels: u16,
    pub generation_workers: u16,
    pub generation_workers_per_client: u16,
    pub cascaded_shadows: bool,
    pub screen_space_ambient_occlusion: bool,
    pub lod_boundary_half_extents_voxels: [u32; 8],
    pub profiling_warmup_seconds: f32,
    pub profiling_measure_seconds: f32,
    pub day_length_seconds: f32,
    pub world_day_number_at_unix_epoch: i64,
    pub day_fraction_at_unix_epoch: f32,
    pub days_per_year: f32,
    pub moon_sidereal_orbit_days: f32,
    pub moon_orbit_phase_at_world_epoch: f32,
    pub planet_circumference_metres: f32,
    pub axial_tilt_degrees: f32,
    pub moon_orbit_inclination_degrees: f32,
    pub celestial_seed: u64,
    pub celestial_revision: u64,
    pub weather_cycle_seconds: f32,
    pub weather_fraction_at_unix_epoch: f32,
    pub cloud_velocity_metres_per_second: [f32; 2],
    pub cloud_coverage: f32,
    pub cloud_base_metres: f32,
    pub cloud_top_metres: f32,
    pub outbound_bandwidth_floor_bytes_per_second: usize,
    pub outbound_bandwidth_ceiling_bytes_per_second: usize,
    pub outbound_bandwidth_burst_bytes: usize,
    pub outbound_queue_delay_target_ms: u16,
    pub outbound_feedback_timeout_ms: u16,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AutomationFixtureConfig {
    pub service_toml: String,
    pub client_toml: String,
    pub routed_client_tomls: Vec<String>,
    pub resolved: AutomationFixtureResolved,
}

#[derive(Debug)]
pub enum AutomationFixtureError {
    UnsupportedSchema { expected: u32, found: u32 },
    Service(WorldServiceConfigError),
    Client(ClientConfigError),
}

impl fmt::Display for AutomationFixtureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchema { expected, found } => {
                write!(
                    formatter,
                    "unsupported automation fixture schema {found}; expected {expected}"
                )
            }
            Self::Service(error) => write!(formatter, "invalid world-service fixture: {error}"),
            Self::Client(error) => write!(formatter, "invalid client fixture: {error}"),
        }
    }
}

impl std::error::Error for AutomationFixtureError {}

impl From<WorldServiceConfigError> for AutomationFixtureError {
    fn from(error: WorldServiceConfigError) -> Self {
        Self::Service(error)
    }
}

impl From<ClientConfigError> for AutomationFixtureError {
    fn from(error: ClientConfigError) -> Self {
        Self::Client(error)
    }
}

pub fn build_automation_fixture(
    service_toml: &str,
    client_toml: &str,
    overlay: AutomationFixtureOverlay,
) -> Result<AutomationFixtureConfig, AutomationFixtureError> {
    if overlay.schema_version != AUTOMATION_FIXTURE_SCHEMA_VERSION {
        return Err(AutomationFixtureError::UnsupportedSchema {
            expected: AUTOMATION_FIXTURE_SCHEMA_VERSION,
            found: overlay.schema_version,
        });
    }

    let mut service = WorldServiceConfig::from_toml(service_toml)?;
    let mut client = ClientConfig::from_toml(client_toml)?;

    service.source = overlay.source;
    service.transport.listen = SocketAddr::from(([127, 0, 0, 1], overlay.backend_port));
    service.transport.allowed_origins = vec![format!("http://127.0.0.1:{}", overlay.browser_port)];
    service.transport.auth_subprotocol_token = overlay.auth_token.clone();
    service.edits.database = PathBuf::from("world-state.sqlite3");

    if let Some(value) = overlay.spawn_voxels {
        service.spawn.xz_voxels = value;
    }
    if let Some(value) = overlay.spawn_pillar_height_voxels {
        service.spawn.pillar_height_voxels = value;
    }
    if let Some(value) = overlay.spawn_pillar_radius_voxels {
        service.spawn.pillar_radius_voxels = value;
    }
    if let Some(value) = overlay.spawn_protection_radius_voxels {
        service.spawn.protection_radius_voxels = value;
    }
    if let Some(value) = overlay.generation_workers {
        service.transport.generation_workers = value;
    }
    if let Some(value) = overlay.generation_workers_per_client {
        service.transport.generation_workers_per_client = value;
    }

    if let Some(value) = overlay.day_length_seconds {
        service.environment.day_length_seconds = value;
    }
    if let Some(value) = overlay.world_day_number_at_unix_epoch {
        service.environment.world_day_number_at_unix_epoch = value;
    }
    if let Some(value) = overlay.day_fraction_at_unix_epoch {
        service.environment.day_fraction_at_unix_epoch = value;
    }
    if let Some(value) = overlay.days_per_year {
        service.environment.days_per_year = value;
    }
    if let Some(value) = overlay.moon_sidereal_orbit_days {
        service.environment.moon_sidereal_orbit_days = value;
    }
    if let Some(value) = overlay.moon_orbit_phase_at_world_epoch {
        service.environment.moon_orbit_phase_at_world_epoch = value;
    }
    if let Some(value) = overlay.planet_circumference_metres {
        service.environment.planet_circumference_metres = value;
    }
    if let Some(value) = overlay.axial_tilt_degrees {
        service.environment.axial_tilt_degrees = value;
    }
    if let Some(value) = overlay.moon_orbit_inclination_degrees {
        service.environment.moon_orbit_inclination_degrees = value;
    }
    if let Some(value) = overlay.celestial_seed {
        service.environment.celestial_seed = value;
    }
    if let Some(value) = overlay.celestial_revision {
        service.environment.celestial_revision = value;
    }
    if let Some(value) = overlay.weather_cycle_seconds {
        service.environment.weather_cycle_seconds = value;
    }
    if let Some(value) = overlay.weather_fraction_at_unix_epoch {
        service.environment.weather_fraction_at_unix_epoch = value;
    }
    if let Some(value) = overlay.cloud_velocity_metres_per_second {
        service.environment.cloud_velocity_metres_per_second = value;
    }
    if let Some(value) = overlay.cloud_coverage {
        service.environment.cloud_coverage = value;
    }
    if let Some(value) = overlay.cloud_base_metres {
        service.environment.cloud_base_metres = value;
    }
    if let Some(value) = overlay.cloud_top_metres {
        service.environment.cloud_top_metres = value;
    }

    let configure_client = |client: &mut ClientConfig, port: u16| {
        client.world.endpoint = format!("ws://127.0.0.1:{port}{WORLD_WEBSOCKET_PATH}");
        client.world.presence_endpoint = format!("ws://127.0.0.1:{port}{PRESENCE_WEBSOCKET_PATH}");
        client.world.subprotocol = WORLD_WEBSOCKET_PROTOCOL.to_owned();
        client
            .world
            .auth_subprotocol_token
            .clone_from(&overlay.auth_token);
    };
    configure_client(&mut client, overlay.backend_port);
    if let Some(value) = overlay.cascaded_shadows {
        client.rendering.features.cascaded_sun_shadows = value;
    }
    if let Some(value) = overlay.screen_space_ambient_occlusion {
        client.rendering.features.screen_space_ambient_occlusion = value;
    }
    if let Some(value) = overlay.lod_boundary_half_extents_voxels {
        client.rendering.geometry_lod.boundary_half_extents_voxels = value;
    }
    if let Some(value) = overlay.diagnostic_sky_rgb {
        client.rendering.diagnostics.sky_override_rgb = Some(value);
    }
    if let Some(value) = overlay.profiling_warmup_seconds {
        client.profiling.warmup_seconds = value;
    }
    if let Some(value) = overlay.profiling_measure_seconds {
        client.profiling.measure_seconds = value;
    }
    let routed_clients = overlay
        .client_ports
        .iter()
        .map(|port| {
            let mut routed = client.clone();
            configure_client(&mut routed, *port);
            routed
        })
        .collect::<Vec<_>>();

    let resolved = AutomationFixtureResolved {
        spawn_voxels: service.spawn.xz_voxels,
        spawn_pillar_height_voxels: service.spawn.pillar_height_voxels,
        spawn_pillar_radius_voxels: service.spawn.pillar_radius_voxels,
        spawn_protection_radius_voxels: service.spawn.protection_radius_voxels,
        generation_workers: service.transport.generation_workers,
        generation_workers_per_client: service.transport.generation_workers_per_client,
        cascaded_shadows: client.rendering.features.cascaded_sun_shadows,
        screen_space_ambient_occlusion: client.rendering.features.screen_space_ambient_occlusion,
        lod_boundary_half_extents_voxels: client
            .rendering
            .geometry_lod
            .boundary_half_extents_voxels,
        profiling_warmup_seconds: client.profiling.warmup_seconds,
        profiling_measure_seconds: client.profiling.measure_seconds,
        day_length_seconds: service.environment.day_length_seconds,
        world_day_number_at_unix_epoch: service.environment.world_day_number_at_unix_epoch,
        day_fraction_at_unix_epoch: service.environment.day_fraction_at_unix_epoch,
        days_per_year: service.environment.days_per_year,
        moon_sidereal_orbit_days: service.environment.moon_sidereal_orbit_days,
        moon_orbit_phase_at_world_epoch: service.environment.moon_orbit_phase_at_world_epoch,
        planet_circumference_metres: service.environment.planet_circumference_metres,
        axial_tilt_degrees: service.environment.axial_tilt_degrees,
        moon_orbit_inclination_degrees: service.environment.moon_orbit_inclination_degrees,
        celestial_seed: service.environment.celestial_seed,
        celestial_revision: service.environment.celestial_revision,
        weather_cycle_seconds: service.environment.weather_cycle_seconds,
        weather_fraction_at_unix_epoch: service.environment.weather_fraction_at_unix_epoch,
        cloud_velocity_metres_per_second: service.environment.cloud_velocity_metres_per_second,
        cloud_coverage: service.environment.cloud_coverage,
        cloud_base_metres: service.environment.cloud_base_metres,
        cloud_top_metres: service.environment.cloud_top_metres,
        outbound_bandwidth_floor_bytes_per_second: service
            .transport
            .outbound_bandwidth_floor_bytes_per_second,
        outbound_bandwidth_ceiling_bytes_per_second: service
            .transport
            .outbound_bandwidth_ceiling_bytes_per_second,
        outbound_bandwidth_burst_bytes: service.transport.outbound_bandwidth_burst_bytes,
        outbound_queue_delay_target_ms: service.transport.outbound_queue_delay_target_ms,
        outbound_feedback_timeout_ms: service.transport.outbound_feedback_timeout_ms,
    };

    Ok(AutomationFixtureConfig {
        service_toml: service.to_toml()?,
        client_toml: client.to_toml()?,
        routed_client_tomls: routed_clients
            .into_iter()
            .map(|client| client.to_toml())
            .collect::<Result<_, _>>()?,
        resolved,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overlay() -> AutomationFixtureOverlay {
        AutomationFixtureOverlay {
            schema_version: AUTOMATION_FIXTURE_SCHEMA_VERSION,
            browser_port: 41_234,
            backend_port: 41_235,
            client_ports: Vec::new(),
            auth_token: "automation-token".to_owned(),
            source: WorldSourceMode::ProceduralV16,
            spawn_voxels: Some([-12_800, 25_600]),
            spawn_pillar_height_voxels: Some(7),
            spawn_pillar_radius_voxels: Some(2),
            spawn_protection_radius_voxels: Some(3),
            generation_workers: Some(12),
            generation_workers_per_client: Some(5),
            cascaded_shadows: Some(false),
            screen_space_ambient_occlusion: Some(false),
            lod_boundary_half_extents_voxels: Some([
                160, 384, 768, 1_536, 3_072, 6_144, 12_288, 24_576,
            ]),
            diagnostic_sky_rgb: Some([255, 0, 255]),
            profiling_warmup_seconds: Some(40.0),
            profiling_measure_seconds: Some(80.0),
            day_length_seconds: None,
            world_day_number_at_unix_epoch: None,
            day_fraction_at_unix_epoch: None,
            days_per_year: None,
            moon_sidereal_orbit_days: None,
            moon_orbit_phase_at_world_epoch: None,
            planet_circumference_metres: None,
            axial_tilt_degrees: None,
            moon_orbit_inclination_degrees: None,
            celestial_seed: None,
            celestial_revision: None,
            weather_cycle_seconds: Some(36.0),
            weather_fraction_at_unix_epoch: Some(0.62),
            cloud_velocity_metres_per_second: None,
            cloud_coverage: Some(0.31),
            cloud_base_metres: Some(600.0),
            cloud_top_metres: Some(1_400.0),
        }
    }

    #[test]
    fn fixture_round_trips_through_production_config_types() {
        let fixture = build_automation_fixture(
            include_str!("../../config/world-service.toml"),
            include_str!("../../config/client.toml"),
            overlay(),
        )
        .expect("valid fixture");

        let service =
            WorldServiceConfig::from_toml(&fixture.service_toml).expect("service round trip");
        let client = ClientConfig::from_toml(&fixture.client_toml).expect("client round trip");
        assert_eq!(service.transport.listen.port(), 41_235);
        assert_eq!(service.spawn.xz_voxels, [-12_800, 25_600]);
        assert_eq!(service.transport.generation_workers, 12);
        assert_eq!(service.transport.generation_workers_per_client, 5);
        assert_eq!(fixture.resolved.generation_workers, 12);
        assert_eq!(fixture.resolved.generation_workers_per_client, 5);
        assert_eq!(
            client.world.endpoint,
            format!("ws://127.0.0.1:41235{WORLD_WEBSOCKET_PATH}")
        );
        assert!(!fixture.resolved.cascaded_shadows);
        assert_eq!(
            client.rendering.geometry_lod.boundary_half_extents_voxels,
            [160, 384, 768, 1_536, 3_072, 6_144, 12_288, 24_576]
        );
        assert_eq!(
            client.rendering.diagnostics.sky_override_rgb,
            Some([255, 0, 255])
        );
        assert_eq!(client.profiling.warmup_seconds, 40.0);
        assert_eq!(client.profiling.measure_seconds, 80.0);
        assert_eq!(fixture.resolved.cloud_top_metres, 1_400.0);
        assert_eq!(
            fixture.resolved.outbound_bandwidth_ceiling_bytes_per_second,
            4 * 1_024 * 1_024
        );
    }

    #[test]
    fn fixture_rejects_unsupported_overlay_schema() {
        let mut invalid = overlay();
        invalid.schema_version += 1;
        assert!(matches!(
            build_automation_fixture(
                include_str!("../../config/world-service.toml"),
                include_str!("../../config/client.toml"),
                invalid,
            ),
            Err(AutomationFixtureError::UnsupportedSchema { .. })
        ));
    }

    #[test]
    fn fixture_can_route_clients_through_a_separate_transport() {
        let mut shaped = overlay();
        shaped.client_ports = vec![41_236, 41_237];
        let fixture = build_automation_fixture(
            include_str!("../../config/world-service.toml"),
            include_str!("../../config/client.toml"),
            shaped,
        )
        .expect("valid shaped-link fixture");
        let client =
            ClientConfig::from_toml(&fixture.routed_client_tomls[0]).expect("client round trip");

        assert_eq!(
            client.world.endpoint,
            format!("ws://127.0.0.1:41236{WORLD_WEBSOCKET_PATH}")
        );
        assert_eq!(
            client.world.presence_endpoint,
            format!("ws://127.0.0.1:41236{PRESENCE_WEBSOCKET_PATH}")
        );
        assert_eq!(fixture.routed_client_tomls.len(), 2);
    }
}
