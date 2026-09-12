//! Server-owned configuration and source-provider bootstrap.
//!
//! Client and protocol code consume canonical world products and source identities. Only the
//! service reads this configuration or knows which macro-terrain provider is active.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use uuid::Uuid;
use voxels_world::protocol::{
    FRAME_HEADER_BYTES, MAX_PLAYERS_PER_PRESENCE_DELTA, MAX_PROTOCOL_FRAME_BYTES,
};
use voxels_world::{
    MacroTerrainSource, Material, ProceduralWorldSource, WorldId, WorldSourceEngine,
    WorldSourceError, WorldSourceIdentityHash,
};
#[cfg(feature = "automation-fixture")]
pub mod automation_fixture;
mod edits;
mod generation_limiter;
mod presence;
pub mod server;
#[cfg(feature = "automation-fixture")]
pub mod storage_benchmark;
mod traffic;
mod virtual_terrain;

pub use server::{
    PRESENCE_WEBSOCKET_PATH, WORLD_WEBSOCKET_PATH, WORLD_WEBSOCKET_PROTOCOL, WorldServer,
    WorldServerError, serve_loaded_config,
};

pub const WORLD_SERVICE_CONFIG_SCHEMA_VERSION: u32 = 27;
pub const EDIT_DATABASE_SCHEMA_VERSION: i64 = 14;

const DEFAULT_WORLD_ID: [u8; 16] = [
    0x76, 0x6f, 0x78, 0x65, 0x6c, 0x73, 0x40, 0x6c, 0x6f, 0x63, 0x61, 0x6c, 0x00, 0x00, 0x00, 0x01,
];
const MAX_CONFIGURED_IN_FLIGHT_BATCHES: u16 = 1_024;
const EDIT_DATABASE_SCHEMA_TOKEN: &str = "{edit_schema}";
const EDIT_DATABASE_WORLD_TOKEN: &str = "{world_id}";
const EDIT_DATABASE_SOURCE_TOKEN: &str = "{source_hash}";

const fn default_virtual_terrain_cache_bytes() -> usize {
    256 * 1024 * 1024
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoopbackTransportConfig {
    pub listen: SocketAddr,
    /// Public deployments must opt in explicitly; local and automation configs remain loopback-only.
    #[serde(default)]
    pub allow_non_loopback: bool,
    pub allowed_origins: Vec<String>,
    /// Offered as a second `Sec-WebSocket-Protocol` value so it is not placed in a URL or log.
    pub auth_subprotocol_token: String,
    /// Optional environment variable containing the HMAC key for signed public session tokens.
    /// When set, the static token remains only a syntactically valid local/config placeholder.
    #[serde(default)]
    pub auth_session_hmac_key_env: Option<String>,
    pub max_frame_bytes: usize,
    /// Maximum encoded world-product bytes retained while a client socket is backpressured.
    pub max_queued_outbound_bytes_per_client: usize,
    /// Safe combined VXWP payload floor across one player's world and presence sockets.
    pub outbound_bandwidth_floor_bytes_per_second: usize,
    /// Maximum rate receiver latency feedback may unlock for a healthy connection.
    pub outbound_bandwidth_ceiling_bytes_per_second: usize,
    /// Initial and maximum token-bucket credit. A single larger frame is sent whole and repaid.
    pub outbound_bandwidth_burst_bytes: usize,
    /// Maximum receiver-observed RTT growth tolerated before the adaptive rate is reduced.
    pub outbound_queue_delay_target_ms: u16,
    /// Return to the safe floor when receiver latency feedback is absent for this long.
    pub outbound_feedback_timeout_ms: u16,
    /// Frames above this size are split into pacing-aware VXWP fragments.
    pub outbound_max_frame_fragment_bytes: usize,
    /// Per-connection request window negotiated with each browser.
    pub max_in_flight_batches: u16,
    /// Hard connection bound per world/presence endpoint. Each accepted WebSocket holds one permit.
    pub max_connections: u16,
    /// Bounded process-wide queue shared by all clients.
    pub global_queue_capacity: u16,
    /// LRU budget for immutable encoded world products shared by overlapping requests.
    pub product_cache_bytes: usize,
    /// LRU budget for encoded virtual-terrain directories and page payloads.
    #[serde(default = "default_virtual_terrain_cache_bytes")]
    pub virtual_terrain_cache_bytes: usize,
    /// LRU budget for complete compressed batch responses shared by co-located clients.
    pub response_cache_bytes: usize,
    /// Maximum blocking generation batches executing across all clients.
    pub generation_workers: u16,
    /// Fairness guard: one connection cannot occupy the whole worker pool.
    pub generation_workers_per_client: u16,
    /// Dedicated urgent lane per connection, still bounded by the process-wide worker pool.
    pub collision_generation_workers_per_client: u16,
}

impl Default for LoopbackTransportConfig {
    fn default() -> Self {
        Self {
            listen: SocketAddr::from(([127, 0, 0, 1], 9_777)),
            allow_non_loopback: false,
            allowed_origins: vec![
                "http://127.0.0.1:5173".to_owned(),
                "http://localhost:5173".to_owned(),
            ],
            auth_subprotocol_token: "replace-with-a-random-local-token".to_owned(),
            auth_session_hmac_key_env: None,
            max_frame_bytes: MAX_PROTOCOL_FRAME_BYTES,
            max_queued_outbound_bytes_per_client: 32 * 1024 * 1024,
            outbound_bandwidth_floor_bytes_per_second: 96 * 1024,
            outbound_bandwidth_ceiling_bytes_per_second: 4 * 1024 * 1024,
            outbound_bandwidth_burst_bytes: 64 * 1024,
            outbound_queue_delay_target_ms: 25,
            outbound_feedback_timeout_ms: 3_000,
            outbound_max_frame_fragment_bytes: 32 * 1024,
            max_in_flight_batches: 16,
            max_connections: 1_024,
            global_queue_capacity: 16_384,
            product_cache_bytes: 256 * 1024 * 1024,
            virtual_terrain_cache_bytes: default_virtual_terrain_cache_bytes(),
            response_cache_bytes: 64 * 1024 * 1024,
            generation_workers: 8,
            generation_workers_per_client: 2,
            collision_generation_workers_per_client: 1,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PresenceConfig {
    /// Per-connection replication scheduler cadence.
    pub broadcast_interval_ms: u16,
    /// Hard player bound for this single, unsharded world.
    pub max_players: u16,
    /// Per-player inbound abuse bound. Clients normally send at 30 Hz.
    pub max_pose_updates_per_second: u16,
    /// Width and depth of a spatial-interest grid cell.
    pub spatial_cell_metres: u16,
    /// Players outside this horizontal radius do not enter a receiver's replication set.
    pub interest_radius_metres: u16,
    /// Extra radius retained for known players to avoid boundary enter/leave churn.
    pub interest_hysteresis_metres: u16,
    pub near_radius_metres: u16,
    pub mid_radius_metres: u16,
    pub near_update_interval_ms: u16,
    pub mid_update_interval_ms: u16,
    pub far_update_interval_ms: u16,
    /// Hard dense-region budget shared by enters and dynamic pose updates in one delta.
    pub max_records_per_delta: u16,
    /// Send early when dead-reckoned position differs from authoritative state by this amount.
    pub prediction_error_centimetres: u16,
    /// Send early when predicted look direction differs by this many milliradians.
    pub look_error_milliradians: u16,
}

impl Default for PresenceConfig {
    fn default() -> Self {
        Self {
            broadcast_interval_ms: 33,
            max_players: 1_024,
            max_pose_updates_per_second: 60,
            spatial_cell_metres: 64,
            interest_radius_metres: 256,
            interest_hysteresis_metres: 32,
            near_radius_metres: 32,
            mid_radius_metres: 96,
            near_update_interval_ms: 50,
            mid_update_interval_ms: 100,
            far_update_interval_ms: 250,
            max_records_per_delta: 64,
            prediction_error_centimetres: 25,
            look_error_milliradians: 175,
        }
    }
}

/// Server-authoritative interaction and movement limits.
///
/// Distances and speeds use integer centimetres so configuration equality and TOML round trips do
/// not depend on floating-point spelling. The presence service spends movement from bounded token
/// buckets; a delayed packet can therefore cover a modest accumulated distance without letting a
/// client add the fixed tolerance again on every pose.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameplayConfig {
    /// Advertise and accept the normal airborne glider pose flag.
    pub allow_gliding: bool,
    /// Advertise and accept a bodyless spectator camera. Spectators retain bounded movement and
    /// world-stream interest, but have no avatar or edit authority.
    pub allow_spectator_mode: bool,
    /// Maximum ray interaction distance before bounded latency tolerance is added.
    pub interaction_reach_centimetres: u16,
    /// Hard extra distance allowed for ordering skew between world and presence WebSockets.
    pub interaction_latency_slack_centimetres: u16,
    /// Edits require a recently received pose from the same connection.
    pub interaction_pose_max_age_ms: u16,
    /// Sustained horizontal movement budget.
    pub max_horizontal_speed_centimetres_per_second: u16,
    /// Sustained vertical movement budget, including jumps and falling.
    pub max_vertical_speed_centimetres_per_second: u16,
    /// Sustained horizontal budget for a bodyless, read-only spectator camera.
    pub spectator_max_horizontal_speed_centimetres_per_second: u16,
    /// Sustained vertical budget for a bodyless, read-only spectator camera.
    pub spectator_max_vertical_speed_centimetres_per_second: u16,
    /// Fixed movement credit that absorbs simulation and packet-timing jitter.
    pub movement_slack_centimetres: u16,
    /// Maximum delayed-motion credit that can accumulate while pose packets are absent.
    pub movement_credit_window_ms: u16,
    /// Delayed-motion credit window for high-speed spectators. This remains bounded by the pose
    /// freshness limit, but tolerates an ordinary transient presence-socket stall at cruise speed.
    pub spectator_movement_credit_window_ms: u16,
}

impl Default for GameplayConfig {
    fn default() -> Self {
        Self {
            allow_gliding: true,
            allow_spectator_mode: false,
            interaction_reach_centimetres: 500,
            interaction_latency_slack_centimetres: 100,
            interaction_pose_max_age_ms: 1_000,
            max_horizontal_speed_centimetres_per_second: 900,
            max_vertical_speed_centimetres_per_second: 2_000,
            spectator_max_horizontal_speed_centimetres_per_second: 15_000,
            spectator_max_vertical_speed_centimetres_per_second: 15_000,
            movement_slack_centimetres: 100,
            movement_credit_window_ms: 500,
            spectator_movement_credit_window_ms: 2_000,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpawnConfig {
    /// Canonical voxel X/Z coordinate sampled when constructing `WorldOpened`.
    pub xz_voxels: [i32; 2],
    /// Height added above the highest source surface under the pillar footprint.
    pub pillar_height_voxels: u16,
    /// Circular pillar radius around `xz_voxels`, including the centre voxel.
    pub pillar_radius_voxels: u8,
    /// Horizontal radius in which all player-authored digging and placement is rejected.
    pub protection_radius_voxels: u16,
    /// Collidable material used for the server-authored pillar.
    pub pillar_material: Material,
}

impl Default for SpawnConfig {
    fn default() -> Self {
        Self {
            xz_voxels: [0, 0],
            pillar_height_voxels: 50,
            pillar_radius_voxels: 25,
            protection_radius_voxels: 64,
            pillar_material: Material::Stone,
        }
    }
}

/// Restart-stable server authority for celestial time and the first weather layer.
///
/// `day_fraction_at_unix_epoch` makes an accelerated clock deterministic across daemon restarts.
/// Set `day_length_seconds` to zero to hold an exact time for visual tests or authored events.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentConfig {
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
    pub cloud_offset_metres_at_unix_epoch: [f32; 2],
    pub cloud_velocity_metres_per_second: [f32; 2],
    pub cloud_coverage: f32,
    pub cloud_base_metres: f32,
    pub cloud_top_metres: f32,
    pub weather_seed: u64,
    pub weather_revision: u64,
}

impl Default for EnvironmentConfig {
    fn default() -> Self {
        Self {
            day_length_seconds: 1_200.0,
            world_day_number_at_unix_epoch: 0,
            day_fraction_at_unix_epoch: 0.72,
            days_per_year: 365.242_2,
            moon_sidereal_orbit_days: 27.321_661,
            moon_orbit_phase_at_world_epoch: 0.0,
            planet_circumference_metres: 40_075_016.0,
            axial_tilt_degrees: 23.439_3,
            moon_orbit_inclination_degrees: 5.145,
            celestial_seed: 0x57a2_5eed,
            celestial_revision: 1,
            weather_cycle_seconds: 900.0,
            weather_fraction_at_unix_epoch: 0.08,
            cloud_offset_metres_at_unix_epoch: [0.0, 0.0],
            cloud_velocity_metres_per_second: [5.5, 1.6],
            cloud_coverage: 0.24,
            cloud_base_metres: 550.0,
            cloud_top_metres: 1_800.0,
            weather_seed: 0x57ea_7aed,
            weather_revision: 1,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditPersistenceConfig {
    /// Native SQLite file containing the authoritative sparse edit journal. Relative paths are
    /// resolved from the directory containing the service configuration file. `{edit_schema}`,
    /// `{world_id}`, and `{source_hash}` can scope development state to every compatibility input.
    pub database: PathBuf,
    /// Bounded per-client change queue. Overflow forces an explicit product resynchronization.
    pub change_queue_capacity: u16,
}

impl Default for EditPersistenceConfig {
    fn default() -> Self {
        Self {
            database: PathBuf::from(
                "../tmp/world-state/schema-{edit_schema}/{world_id}-{source_hash}.sqlite3",
            ),
            change_queue_capacity: 1_024,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum WorldSourceMode {
    #[default]
    #[serde(rename = "procedural-v17")]
    ProceduralV17,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldServiceConfig {
    pub schema_version: u32,
    pub world_id: Uuid,
    pub world_seed: u64,
    pub source: WorldSourceMode,
    pub transport: LoopbackTransportConfig,
    pub presence: PresenceConfig,
    pub gameplay: GameplayConfig,
    pub environment: EnvironmentConfig,
    pub edits: EditPersistenceConfig,
    pub spawn: SpawnConfig,
}

impl Default for WorldServiceConfig {
    fn default() -> Self {
        Self {
            schema_version: WORLD_SERVICE_CONFIG_SCHEMA_VERSION,
            world_id: Uuid::from_bytes(DEFAULT_WORLD_ID),
            world_seed: 0x5eed_cafe,
            source: WorldSourceMode::ProceduralV17,
            transport: LoopbackTransportConfig::default(),
            presence: PresenceConfig::default(),
            gameplay: GameplayConfig::default(),
            environment: EnvironmentConfig::default(),
            edits: EditPersistenceConfig::default(),
            spawn: SpawnConfig::default(),
        }
    }
}

impl WorldServiceConfig {
    pub fn from_toml(contents: &str) -> Result<Self, WorldServiceConfigError> {
        let config: Self = toml::from_str(contents)
            .map_err(|error| WorldServiceConfigError::Parse(error.to_string()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn to_toml(&self) -> Result<String, WorldServiceConfigError> {
        self.validate()?;
        toml::to_string_pretty(self)
            .map_err(|error| WorldServiceConfigError::Serialize(error.to_string()))
    }

    pub fn validate(&self) -> Result<(), WorldServiceConfigError> {
        if self.schema_version != WORLD_SERVICE_CONFIG_SCHEMA_VERSION {
            return Err(WorldServiceConfigError::UnsupportedSchema {
                expected: WORLD_SERVICE_CONFIG_SCHEMA_VERSION,
                found: self.schema_version,
            });
        }
        if !self.transport.listen.ip().is_loopback() && !self.transport.allow_non_loopback {
            return Err(WorldServiceConfigError::ListenIsNotLoopback(
                self.transport.listen,
            ));
        }
        if self.transport.allowed_origins.is_empty() {
            return Err(WorldServiceConfigError::EmptyAllowedOrigins);
        }
        for origin in &self.transport.allowed_origins {
            if !valid_http_origin(origin) {
                return Err(WorldServiceConfigError::InvalidAllowedOrigin(
                    origin.clone(),
                ));
            }
        }
        if self.transport.allow_non_loopback
            && self
                .transport
                .allowed_origins
                .iter()
                .any(|origin| !origin.starts_with("https://"))
        {
            return Err(WorldServiceConfigError::InsecurePublicOrigin);
        }
        if !self.transport.listen.ip().is_loopback()
            && self.transport.auth_session_hmac_key_env.is_none()
        {
            return Err(WorldServiceConfigError::MissingPublicSessionAuthorization);
        }
        if !valid_websocket_protocol_token(&self.transport.auth_subprotocol_token) {
            return Err(WorldServiceConfigError::InvalidAuthSubprotocolToken);
        }
        if self
            .transport
            .auth_session_hmac_key_env
            .as_deref()
            .is_some_and(|name| !valid_environment_name(name))
        {
            return Err(WorldServiceConfigError::InvalidAuthSessionEnvironment);
        }
        if !(FRAME_HEADER_BYTES..=MAX_PROTOCOL_FRAME_BYTES)
            .contains(&self.transport.max_frame_bytes)
        {
            return Err(WorldServiceConfigError::InvalidMaxFrameBytes {
                min: FRAME_HEADER_BYTES,
                max: MAX_PROTOCOL_FRAME_BYTES,
                found: self.transport.max_frame_bytes,
            });
        }
        if self.transport.max_queued_outbound_bytes_per_client < self.transport.max_frame_bytes
            || self.transport.max_queued_outbound_bytes_per_client > 256 * 1024 * 1024
        {
            return Err(WorldServiceConfigError::InvalidConcurrency(
                "max_queued_outbound_bytes_per_client must fit at least one frame and stay at most 256 MiB",
            ));
        }
        if !(32 * 1024..=16 * 1024 * 1024)
            .contains(&self.transport.outbound_bandwidth_floor_bytes_per_second)
        {
            return Err(WorldServiceConfigError::InvalidConcurrency(
                "outbound_bandwidth_floor_bytes_per_second must stay in 32 KiB/s..=16 MiB/s",
            ));
        }
        if self.transport.outbound_bandwidth_ceiling_bytes_per_second
            < self.transport.outbound_bandwidth_floor_bytes_per_second
            || self.transport.outbound_bandwidth_ceiling_bytes_per_second > 16 * 1024 * 1024
        {
            return Err(WorldServiceConfigError::InvalidConcurrency(
                "outbound_bandwidth_ceiling_bytes_per_second must be at least the floor and at most 16 MiB/s",
            ));
        }
        if !(FRAME_HEADER_BYTES..=64 * 1024 * 1024)
            .contains(&self.transport.outbound_bandwidth_burst_bytes)
        {
            return Err(WorldServiceConfigError::InvalidConcurrency(
                "outbound_bandwidth_burst_bytes must stay between one header and 64 MiB",
            ));
        }
        if !(5..=500).contains(&self.transport.outbound_queue_delay_target_ms) {
            return Err(WorldServiceConfigError::InvalidConcurrency(
                "outbound_queue_delay_target_ms must stay in 5..=500",
            ));
        }
        if self.transport.outbound_feedback_timeout_ms
            < self
                .transport
                .outbound_queue_delay_target_ms
                .saturating_mul(4)
            || self.transport.outbound_feedback_timeout_ms > 60_000
        {
            return Err(WorldServiceConfigError::InvalidConcurrency(
                "outbound_feedback_timeout_ms must be at least four queue-delay targets and at most 60000",
            ));
        }
        if !(8 * 1024..=voxels_world::protocol::MAX_FRAME_FRAGMENT_DATA_BYTES)
            .contains(&self.transport.outbound_max_frame_fragment_bytes)
        {
            return Err(WorldServiceConfigError::InvalidConcurrency(
                "outbound_max_frame_fragment_bytes must stay in 8 KiB..=the VXWP fragment limit",
            ));
        }
        if self.transport.max_in_flight_batches == 0
            || self.transport.max_in_flight_batches > MAX_CONFIGURED_IN_FLIGHT_BATCHES
        {
            return Err(WorldServiceConfigError::InvalidMaxInFlightBatches {
                max: MAX_CONFIGURED_IN_FLIGHT_BATCHES,
                found: self.transport.max_in_flight_batches,
            });
        }
        if self.transport.max_connections == 0 {
            return Err(WorldServiceConfigError::InvalidConcurrency(
                "max_connections must be nonzero",
            ));
        }
        if self.transport.global_queue_capacity < self.transport.max_in_flight_batches {
            return Err(WorldServiceConfigError::InvalidConcurrency(
                "global_queue_capacity must cover one full client request window",
            ));
        }
        if self.transport.product_cache_bytes > 1024 * 1024 * 1024 {
            return Err(WorldServiceConfigError::InvalidConcurrency(
                "product_cache_bytes must stay at most 1 GiB",
            ));
        }
        if self.transport.virtual_terrain_cache_bytes > 1024 * 1024 * 1024 {
            return Err(WorldServiceConfigError::InvalidConcurrency(
                "virtual_terrain_cache_bytes must stay at most 1 GiB",
            ));
        }
        if self.transport.response_cache_bytes > 256 * 1024 * 1024 {
            return Err(WorldServiceConfigError::InvalidConcurrency(
                "response_cache_bytes must stay at most 256 MiB",
            ));
        }
        if self.transport.generation_workers == 0
            || self.transport.generation_workers_per_client == 0
            || self.transport.collision_generation_workers_per_client == 0
            || self
                .transport
                .generation_workers_per_client
                .checked_add(self.transport.collision_generation_workers_per_client)
                .is_none_or(|per_client| per_client > self.transport.generation_workers)
        {
            return Err(WorldServiceConfigError::InvalidConcurrency(
                "generation worker lanes must be nonzero and their per-client sum must not exceed global",
            ));
        }
        if !self.environment.day_length_seconds.is_finite()
            || !(0.0..=86_400.0).contains(&self.environment.day_length_seconds)
            || !self.environment.day_fraction_at_unix_epoch.is_finite()
            || !(0.0..1.0).contains(&self.environment.day_fraction_at_unix_epoch)
            || self
                .environment
                .world_day_number_at_unix_epoch
                .unsigned_abs()
                > 1_000_000_000
            || !self.environment.days_per_year.is_finite()
            || !(4.0..=4_096.0).contains(&self.environment.days_per_year)
            || !self.environment.moon_sidereal_orbit_days.is_finite()
            || !(0.25..=self.environment.days_per_year)
                .contains(&self.environment.moon_sidereal_orbit_days)
            || !self.environment.moon_orbit_phase_at_world_epoch.is_finite()
            || !(0.0..1.0).contains(&self.environment.moon_orbit_phase_at_world_epoch)
            || !self.environment.planet_circumference_metres.is_finite()
            || !(100_000.0..=100_000_000.0).contains(&self.environment.planet_circumference_metres)
            || !self.environment.axial_tilt_degrees.is_finite()
            || !(0.0..=45.0).contains(&self.environment.axial_tilt_degrees)
            || !self.environment.moon_orbit_inclination_degrees.is_finite()
            || !(0.0..=30.0).contains(&self.environment.moon_orbit_inclination_degrees)
            || self.environment.celestial_revision == 0
            || !self.environment.weather_cycle_seconds.is_finite()
            || !(0.0..=86_400.0).contains(&self.environment.weather_cycle_seconds)
            || !self.environment.weather_fraction_at_unix_epoch.is_finite()
            || !(0.0..1.0).contains(&self.environment.weather_fraction_at_unix_epoch)
            || !self
                .environment
                .cloud_offset_metres_at_unix_epoch
                .into_iter()
                .chain(self.environment.cloud_velocity_metres_per_second)
                .all(|value| value.is_finite())
            || self
                .environment
                .cloud_velocity_metres_per_second
                .into_iter()
                .any(|value| value.abs() > 100.0)
            || !self.environment.cloud_coverage.is_finite()
            || !(0.0..=1.0).contains(&self.environment.cloud_coverage)
            || !self.environment.cloud_base_metres.is_finite()
            || !(100.0..=5_000.0).contains(&self.environment.cloud_base_metres)
            || !self.environment.cloud_top_metres.is_finite()
            || self.environment.cloud_top_metres <= self.environment.cloud_base_metres
            || self.environment.cloud_top_metres > 10_000.0
            || self.environment.weather_revision == 0
        {
            return Err(WorldServiceConfigError::InvalidEnvironment(
                "celestial and weather clocks, planetary mapping, cloud layer, wind, coverage, and revisions must be finite and bounded",
            ));
        }
        if !(16..=1_000).contains(&self.presence.broadcast_interval_ms) {
            return Err(WorldServiceConfigError::InvalidPresence(
                "broadcast_interval_ms must be in 16..=1000",
            ));
        }
        if self.presence.max_players == 0
            || usize::from(self.presence.max_players) > MAX_PLAYERS_PER_PRESENCE_DELTA
            || self.presence.max_players > self.transport.max_connections
        {
            return Err(WorldServiceConfigError::InvalidPresence(
                "max_players must be nonzero and fit both the protocol and connection limit",
            ));
        }
        if !(1..=120).contains(&self.presence.max_pose_updates_per_second) {
            return Err(WorldServiceConfigError::InvalidPresence(
                "max_pose_updates_per_second must be in 1..=120",
            ));
        }
        if !(8..=256).contains(&self.presence.spatial_cell_metres)
            || self.presence.interest_radius_metres < self.presence.spatial_cell_metres
            || self.presence.interest_radius_metres > 2_048
            || self.presence.interest_hysteresis_metres > self.presence.interest_radius_metres
        {
            return Err(WorldServiceConfigError::InvalidPresence(
                "presence spatial cell, interest radius, or hysteresis is invalid",
            ));
        }
        if self.presence.near_radius_metres == 0
            || self.presence.near_radius_metres > self.presence.mid_radius_metres
            || self.presence.mid_radius_metres > self.presence.interest_radius_metres
        {
            return Err(WorldServiceConfigError::InvalidPresence(
                "presence distance tiers must be ordered inside the interest radius",
            ));
        }
        if self.presence.near_update_interval_ms < self.presence.broadcast_interval_ms
            || self.presence.mid_update_interval_ms < self.presence.near_update_interval_ms
            || self.presence.far_update_interval_ms < self.presence.mid_update_interval_ms
            || self.presence.far_update_interval_ms > 2_000
        {
            return Err(WorldServiceConfigError::InvalidPresence(
                "presence update intervals must be ordered and fit 16..=2000 ms",
            ));
        }
        if self.presence.max_records_per_delta == 0
            || self.presence.prediction_error_centimetres == 0
            || self.presence.look_error_milliradians == 0
        {
            return Err(WorldServiceConfigError::InvalidPresence(
                "presence delta budget and prediction-error thresholds must be nonzero",
            ));
        }
        if !(100..=1_000).contains(&self.gameplay.interaction_reach_centimetres)
            || self.gameplay.interaction_latency_slack_centimetres > 300
            || !(100..=5_000).contains(&self.gameplay.interaction_pose_max_age_ms)
        {
            return Err(WorldServiceConfigError::InvalidGameplay(
                "interaction reach, latency slack, or pose age is invalid",
            ));
        }
        if !(100..=2_000).contains(&self.gameplay.max_horizontal_speed_centimetres_per_second)
            || !(100..=5_000).contains(&self.gameplay.max_vertical_speed_centimetres_per_second)
            || !(self.gameplay.max_horizontal_speed_centimetres_per_second..=60_000).contains(
                &self
                    .gameplay
                    .spectator_max_horizontal_speed_centimetres_per_second,
            )
            || !(self.gameplay.max_vertical_speed_centimetres_per_second..=60_000).contains(
                &self
                    .gameplay
                    .spectator_max_vertical_speed_centimetres_per_second,
            )
            || !(1..=300).contains(&self.gameplay.movement_slack_centimetres)
            || !(100..=2_000).contains(&self.gameplay.movement_credit_window_ms)
            || !(self.gameplay.movement_credit_window_ms..=2_000)
                .contains(&self.gameplay.spectator_movement_credit_window_ms)
        {
            return Err(WorldServiceConfigError::InvalidGameplay(
                "player/spectator movement speeds, slack, or credit window is invalid",
            ));
        }
        if self.spawn.pillar_height_voxels == 0
            || self.spawn.pillar_height_voxels > 1_000
            || self.spawn.pillar_radius_voxels == 0
            || self.spawn.pillar_radius_voxels > 32
            || self.spawn.protection_radius_voxels < u16::from(self.spawn.pillar_radius_voxels)
            || self.spawn.protection_radius_voxels > 10_000
            || !self.spawn.pillar_material.is_collidable()
        {
            return Err(WorldServiceConfigError::InvalidSpawn(
                "pillar height/radius, protection radius, or material is invalid",
            ));
        }
        if self.edits.database.as_os_str().is_empty() {
            return Err(WorldServiceConfigError::InvalidEdits(
                "edit database path must not be empty",
            ));
        }
        if !(16..=4_096).contains(&self.edits.change_queue_capacity) {
            return Err(WorldServiceConfigError::InvalidEdits(
                "edit change_queue_capacity must be in 16..=4096",
            ));
        }
        Ok(())
    }

    pub fn canonical_world_id(&self) -> WorldId {
        WorldId::from_bytes(*self.world_id.as_bytes())
    }
}

fn valid_http_origin(origin: &str) -> bool {
    let authority = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"));
    authority.is_some_and(|authority| {
        !authority.is_empty()
            && !authority.ends_with('/')
            && !authority
                .bytes()
                .any(|byte| byte.is_ascii_whitespace() || matches!(byte, b',' | b'/' | b'?' | b'#'))
    })
}

fn valid_websocket_protocol_token(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= 128
        && token.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

fn valid_environment_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name.bytes().enumerate().all(|(index, byte)| {
            byte == b'_' || byte.is_ascii_uppercase() || (index > 0 && byte.is_ascii_digit())
        })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorldServiceConfigError {
    Read {
        path: PathBuf,
        reason: String,
    },
    Parse(String),
    Serialize(String),
    UnsupportedSchema {
        expected: u32,
        found: u32,
    },
    ListenIsNotLoopback(SocketAddr),
    InsecurePublicOrigin,
    MissingPublicSessionAuthorization,
    EmptyAllowedOrigins,
    InvalidAllowedOrigin(String),
    InvalidAuthSubprotocolToken,
    InvalidAuthSessionEnvironment,
    InvalidMaxFrameBytes {
        min: usize,
        max: usize,
        found: usize,
    },
    InvalidMaxInFlightBatches {
        max: u16,
        found: u16,
    },
    InvalidConcurrency(&'static str),
    InvalidPresence(&'static str),
    InvalidGameplay(&'static str),
    InvalidSpawn(&'static str),
    InvalidEnvironment(&'static str),
    InvalidEdits(&'static str),
}

impl fmt::Display for WorldServiceConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, reason } => {
                write!(formatter, "could not read {}: {reason}", path.display())
            }
            Self::Parse(reason) => write!(formatter, "invalid world-service TOML: {reason}"),
            Self::Serialize(reason) => {
                write!(
                    formatter,
                    "could not serialize world-service TOML: {reason}"
                )
            }
            Self::UnsupportedSchema { expected, found } => write!(
                formatter,
                "unsupported world-service config schema {found}; expected {expected}"
            ),
            Self::ListenIsNotLoopback(address) => write!(
                formatter,
                "world-service transport must listen on loopback, not {address}"
            ),
            Self::InsecurePublicOrigin => formatter
                .write_str("public world-service transports require https:// allowed origins"),
            Self::MissingPublicSessionAuthorization => formatter.write_str(
                "public world-service transports require auth_session_hmac_key_env so player identities are signed",
            ),
            Self::EmptyAllowedOrigins => {
                formatter.write_str("world-service transport requires at least one allowed origin")
            }
            Self::InvalidAllowedOrigin(origin) => {
                write!(formatter, "invalid allowed HTTP origin {origin:?}")
            }
            Self::InvalidAuthSubprotocolToken => formatter.write_str(
                "auth_subprotocol_token must be a non-empty RFC WebSocket protocol token of at most 128 bytes",
            ),
            Self::InvalidAuthSessionEnvironment => formatter.write_str(
                "auth_session_hmac_key_env must be an uppercase ASCII environment variable name",
            ),
            Self::InvalidMaxFrameBytes { min, max, found } => write!(
                formatter,
                "max_frame_bytes {found} is outside the supported range {min}..={max}"
            ),
            Self::InvalidMaxInFlightBatches { max, found } => write!(
                formatter,
                "max_in_flight_batches {found} is outside the supported range 1..={max}"
            ),
            Self::InvalidConcurrency(reason) => {
                write!(formatter, "invalid world-service concurrency: {reason}")
            }
            Self::InvalidPresence(reason) => {
                write!(formatter, "invalid world-service presence: {reason}")
            }
            Self::InvalidGameplay(reason) => {
                write!(formatter, "invalid world-service gameplay: {reason}")
            }
            Self::InvalidSpawn(reason) => {
                write!(formatter, "invalid world-service spawn: {reason}")
            }
            Self::InvalidEnvironment(reason) => {
                write!(formatter, "invalid world-service environment: {reason}")
            }
            Self::InvalidEdits(reason) => {
                write!(formatter, "invalid world-service edits: {reason}")
            }
        }
    }
}

impl std::error::Error for WorldServiceConfigError {}

/// A validated configuration retaining its file location for relative-path resolution.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadedWorldServiceConfig {
    config: WorldServiceConfig,
    path: PathBuf,
}

impl LoadedWorldServiceConfig {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, WorldServiceConfigError> {
        let path = path.as_ref().to_owned();
        let contents =
            std::fs::read_to_string(&path).map_err(|error| WorldServiceConfigError::Read {
                path: path.clone(),
                reason: error.to_string(),
            })?;
        let config = WorldServiceConfig::from_toml(&contents)?;
        Ok(Self { config, path })
    }

    pub fn from_config(
        config: WorldServiceConfig,
        path: impl Into<PathBuf>,
    ) -> Result<Self, WorldServiceConfigError> {
        config.validate()?;
        Ok(Self {
            config,
            path: path.into(),
        })
    }

    pub const fn config(&self) -> &WorldServiceConfig {
        &self.config
    }

    pub fn edit_database_path(&self, source_hash: WorldSourceIdentityHash) -> PathBuf {
        let configured = self.config.edits.database.to_str().map_or_else(
            || self.config.edits.database.clone(),
            |database| {
                PathBuf::from(
                    database
                        .replace(
                            EDIT_DATABASE_SCHEMA_TOKEN,
                            &EDIT_DATABASE_SCHEMA_VERSION.to_string(),
                        )
                        .replace(EDIT_DATABASE_WORLD_TOKEN, &self.config.world_id.to_string())
                        .replace(EDIT_DATABASE_SOURCE_TOKEN, &source_hash.to_string()),
                )
            },
        );
        if configured.is_absolute() {
            configured
        } else {
            self.path
                .parent()
                .map_or_else(|| configured.clone(), |parent| parent.join(&configured))
        }
    }

    /// Constructs the configured macro provider entirely inside the service process.
    ///
    /// Returns the configured procedural generator behind the source-neutral trait.
    pub fn build_macro_source(
        &self,
    ) -> Result<Box<dyn MacroTerrainSource>, WorldServiceSourceError> {
        match self.config.source {
            WorldSourceMode::ProceduralV17 => {
                Ok(Box::new(ProceduralWorldSource::new(self.config.world_seed)))
            }
        }
    }

    pub fn build_world_source(
        &self,
    ) -> Result<Box<dyn WorldSourceEngine>, WorldServiceSourceError> {
        match self.config.source {
            WorldSourceMode::ProceduralV17 => {
                Ok(Box::new(ProceduralWorldSource::new(self.config.world_seed)))
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorldServiceSourceError {
    WorldSource(WorldSourceError),
}

impl fmt::Display for WorldServiceSourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WorldSource(error) => error.fmt(formatter),
        }
    }
}
impl std::error::Error for WorldServiceSourceError {}

impl From<WorldSourceError> for WorldServiceSourceError {
    fn from(error: WorldSourceError) -> Self {
        Self::WorldSource(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use voxels_world::{
        MacroBlockBatch, MacroBlockRequest, VOXEL_SIZE_METRES, WorldProductPriority,
        WorldSourceKind,
    };

    fn test_config(source: WorldSourceMode) -> WorldServiceConfig {
        WorldServiceConfig {
            world_seed: 42,
            source,
            ..WorldServiceConfig::default()
        }
    }

    #[test]
    fn config_round_trips_through_human_readable_toml() {
        let config = test_config(WorldSourceMode::ProceduralV17);
        let serialized = config.to_toml().expect("serializable config");
        assert_eq!(WorldServiceConfig::from_toml(&serialized), Ok(config));
    }

    #[test]
    fn checked_in_configs_are_valid() {
        assert!(
            WorldServiceConfig::from_toml(include_str!("../../config/world-service.toml")).is_ok()
        );
        assert!(
            WorldServiceConfig::from_toml(include_str!(
                "../../config/world-service.production.toml"
            ))
            .is_ok()
        );
    }

    #[test]
    fn default_spawn_is_a_five_metre_raised_platform() {
        let spawn = SpawnConfig::default();
        let height_metres = f32::from(spawn.pillar_height_voxels) * VOXEL_SIZE_METRES;
        assert!((height_metres - 5.0).abs() < f32::EPSILON);
    }

    #[test]
    fn edit_database_path_expands_compatibility_tokens() {
        let source_hash = WorldSourceIdentityHash::from_bytes([0xab; 32]);
        let mut config = test_config(WorldSourceMode::ProceduralV17);
        config.edits.database =
            PathBuf::from("state/schema-{edit_schema}/{world_id}-{source_hash}.sqlite3");
        let loaded = LoadedWorldServiceConfig::from_config(
            config.clone(),
            "test-fixtures/world-service.toml",
        )
        .expect("loaded config");
        assert!(
            loaded
                .edit_database_path(source_hash)
                .to_string_lossy()
                .contains(&source_hash.to_string())
        );
    }

    #[test]
    fn procedural_factory_generates_macro_fields() {
        let loaded = LoadedWorldServiceConfig::from_config(
            test_config(WorldSourceMode::ProceduralV17),
            "world-service.toml",
        )
        .expect("loaded config");
        let source = loaded.build_macro_source().expect("procedural source");
        assert_eq!(
            source.identity().source_kind,
            WorldSourceKind::ProceduralV17
        );
        let result = source
            .request_blocks(MacroBlockBatch {
                priority: WorldProductPriority::VisibleChunk,
                requests: vec![MacroBlockRequest {
                    origin: [0, 0],
                    sample_shape: [2, 2],
                    stride_voxels: 300,
                }],
            })
            .expect("macro block");
        assert_eq!(result.blocks.len(), 1);
    }
}
