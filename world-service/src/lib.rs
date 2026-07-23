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
    HeightfieldWorldSource, MacroTerrainSource, Material, ProceduralWorldSource, SEA_LEVEL_VOXELS,
    WorldId, WorldSourceEngine, WorldSourceError, WorldSourceIdentityHash,
};
use voxels_world_terrain_diffusion::{
    MODEL_REVISION, TerrainDiffusionError, validate_terrain_generation_parameters,
};
#[cfg(all(feature = "terrain-metal", target_os = "macos"))]
use voxels_world_terrain_diffusion::{TerrainDiffusionConfig, TerrainPrecision};

#[cfg(feature = "automation-fixture")]
pub mod automation_fixture;
mod edits;
mod generation_limiter;
mod presence;
pub mod server;
#[cfg(feature = "automation-fixture")]
pub mod storage_benchmark;
mod traffic;

pub use server::{
    PRESENCE_WEBSOCKET_PATH, WORLD_WEBSOCKET_PATH, WORLD_WEBSOCKET_PROTOCOL, WorldServer,
    WorldServerError, serve_loaded_config,
};

pub const WORLD_SERVICE_CONFIG_SCHEMA_VERSION: u32 = 25;
pub const EDIT_DATABASE_SCHEMA_VERSION: i64 = 13;

const DEFAULT_WORLD_ID: [u8; 16] = [
    0x76, 0x6f, 0x78, 0x65, 0x6c, 0x73, 0x40, 0x6c, 0x6f, 0x63, 0x61, 0x6c, 0x00, 0x00, 0x00, 0x01,
];
const MAX_CONFIGURED_IN_FLIGHT_BATCHES: u16 = 1_024;
const EDIT_DATABASE_SCHEMA_TOKEN: &str = "{edit_schema}";
const EDIT_DATABASE_WORLD_TOKEN: &str = "{world_id}";
const EDIT_DATABASE_SOURCE_TOKEN: &str = "{source_hash}";

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
    /// Hard process-wide connection bound. Each accepted WebSocket holds one permit.
    pub max_connections: u16,
    /// Bounded process-wide queue shared by all clients.
    pub global_queue_capacity: u16,
    /// LRU budget for immutable encoded world products shared by overlapping requests.
    pub product_cache_bytes: usize,
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
    #[serde(rename = "procedural-v16")]
    ProceduralV16,
    #[serde(rename = "terrain-diffusion-30m")]
    TerrainDiffusion30m,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TerrainModelPrecision {
    #[default]
    Float16,
    Float32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerrainDiffusionProviderConfig {
    /// Cache root containing the immutable model-revision directory. Relative paths are resolved
    /// from the directory containing the service configuration file.
    pub model_cache: Option<PathBuf>,
    pub precision: TerrainModelPrecision,
    /// Canonical voxel X/Z coordinate where the finite generated tile is placed.
    pub world_origin_voxels: [i32; 2],
    /// Horizontal presentation scale relative to the model's native 30 m sample spacing.
    pub horizontal_scale: u32,
    /// Terrain Diffusion latent-window row/column. One step advances 7.68 km for the 30 m model.
    pub latent_window: [i32; 2],
    /// Five learned terrain-quality logits. The showcase preset favors the two highest bins.
    pub quality_histogram: [f32; 5],
    /// Flood height used by the fidelity-honest macro-heightfield voxel composer.
    pub sea_level_voxels: i32,
}

impl Default for TerrainDiffusionProviderConfig {
    fn default() -> Self {
        Self {
            model_cache: None,
            precision: TerrainModelPrecision::Float16,
            world_origin_voxels: [0, 0],
            horizontal_scale: 1,
            latent_window: [-2, -1],
            quality_histogram: [0.0, 0.0, 0.0, 1.0, 1.5],
            sea_level_voxels: SEA_LEVEL_VOXELS,
        }
    }
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
    pub terrain_diffusion: TerrainDiffusionProviderConfig,
}

impl Default for WorldServiceConfig {
    fn default() -> Self {
        Self {
            schema_version: WORLD_SERVICE_CONFIG_SCHEMA_VERSION,
            world_id: Uuid::from_bytes(DEFAULT_WORLD_ID),
            world_seed: 0x5eed_cafe,
            source: WorldSourceMode::ProceduralV16,
            transport: LoopbackTransportConfig::default(),
            presence: PresenceConfig::default(),
            gameplay: GameplayConfig::default(),
            environment: EnvironmentConfig::default(),
            edits: EditPersistenceConfig::default(),
            spawn: SpawnConfig::default(),
            terrain_diffusion: TerrainDiffusionProviderConfig::default(),
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
        validate_terrain_generation_parameters(
            self.terrain_diffusion.horizontal_scale,
            self.terrain_diffusion.latent_window,
            self.terrain_diffusion.quality_histogram,
        )
        .map_err(WorldServiceConfigError::InvalidTerrainDiffusion)?;
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
    InvalidTerrainDiffusion(TerrainDiffusionError),
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
            Self::InvalidTerrainDiffusion(reason) => {
                write!(formatter, "invalid Terrain Diffusion config: {reason}")
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

    pub fn terrain_model_root(&self) -> Result<PathBuf, WorldServiceSourceError> {
        let cache = match &self.config.terrain_diffusion.model_cache {
            Some(path) if path.is_absolute() => path.clone(),
            Some(path) => self
                .path
                .parent()
                .map_or_else(|| path.clone(), |parent| parent.join(path)),
            None => default_terrain_model_cache()?,
        };
        Ok(cache.join(MODEL_REVISION))
    }

    /// Constructs the configured macro provider entirely inside the service process.
    ///
    /// The returned trait object is identical from the caller's perspective for both modes. A
    /// future canonical composer and transport can therefore remain source-neutral.
    pub fn build_macro_source(
        &self,
    ) -> Result<Box<dyn MacroTerrainSource>, WorldServiceSourceError> {
        match self.config.source {
            WorldSourceMode::ProceduralV16 => {
                Ok(Box::new(ProceduralWorldSource::new(self.config.world_seed)))
            }
            WorldSourceMode::TerrainDiffusion30m => self.build_terrain_diffusion_source(),
        }
    }

    /// Builds the authoritative canonical product engine selected by service configuration.
    ///
    /// Procedural mode intentionally uses the exact current engine. Learned macro terrain is
    /// composed through [`HeightfieldWorldSource`] so both modes expose identical chunk products.
    pub fn build_world_source(
        &self,
    ) -> Result<Box<dyn WorldSourceEngine>, WorldServiceSourceError> {
        match self.config.source {
            WorldSourceMode::ProceduralV16 => {
                Ok(Box::new(ProceduralWorldSource::new(self.config.world_seed)))
            }
            WorldSourceMode::TerrainDiffusion30m => {
                let macro_source = self.build_terrain_diffusion_source()?;
                let source = HeightfieldWorldSource::new(
                    macro_source,
                    self.config.terrain_diffusion.sea_level_voxels,
                )?;
                Ok(Box::new(source))
            }
        }
    }

    #[cfg(all(feature = "terrain-metal", target_os = "macos"))]
    fn build_terrain_diffusion_source(
        &self,
    ) -> Result<Box<dyn MacroTerrainSource>, WorldServiceSourceError> {
        use voxels_world_terrain_diffusion::{
            MetalTerrainDiffusion, TerrainDiffusionMacroTileSource,
        };

        let model_root = self.terrain_model_root()?;
        let precision = match self.config.terrain_diffusion.precision {
            TerrainModelPrecision::Float16 => TerrainPrecision::Float16,
            TerrainModelPrecision::Float32 => TerrainPrecision::Float32,
        };
        let runtime = MetalTerrainDiffusion::load_full(TerrainDiffusionConfig {
            model_root,
            seed: self.config.world_seed,
            precision,
            require_metal: true,
            world_origin_voxels: self.config.terrain_diffusion.world_origin_voxels,
            horizontal_scale: self.config.terrain_diffusion.horizontal_scale,
            latent_window: self.config.terrain_diffusion.latent_window,
            quality_histogram: self.config.terrain_diffusion.quality_histogram,
        })?;
        Ok(Box::new(TerrainDiffusionMacroTileSource::generate(
            &runtime,
        )?))
    }

    #[cfg(not(feature = "terrain-metal"))]
    fn build_terrain_diffusion_source(
        &self,
    ) -> Result<Box<dyn MacroTerrainSource>, WorldServiceSourceError> {
        let _ = self;
        Err(WorldServiceSourceError::TerrainMetalFeatureDisabled)
    }

    #[cfg(all(feature = "terrain-metal", not(target_os = "macos")))]
    fn build_terrain_diffusion_source(
        &self,
    ) -> Result<Box<dyn MacroTerrainSource>, WorldServiceSourceError> {
        let _ = self;
        Err(WorldServiceSourceError::TerrainMetalUnsupportedPlatform)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorldServiceSourceError {
    MissingHomeDirectory,
    TerrainMetalFeatureDisabled,
    TerrainMetalUnsupportedPlatform,
    TerrainDiffusion(TerrainDiffusionError),
    WorldSource(WorldSourceError),
}

impl fmt::Display for WorldServiceSourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingHomeDirectory => {
                formatter.write_str("HOME is unavailable and no Terrain Diffusion cache was set")
            }
            Self::TerrainMetalFeatureDisabled => formatter.write_str(
                "Terrain Diffusion was selected but world-service lacks the `terrain-metal` feature",
            ),
            Self::TerrainMetalUnsupportedPlatform => formatter
                .write_str("Terrain Diffusion was selected but Apple Metal requires macOS"),
            Self::TerrainDiffusion(error) => error.fmt(formatter),
            Self::WorldSource(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for WorldServiceSourceError {}

impl From<TerrainDiffusionError> for WorldServiceSourceError {
    fn from(error: TerrainDiffusionError) -> Self {
        Self::TerrainDiffusion(error)
    }
}

impl From<WorldSourceError> for WorldServiceSourceError {
    fn from(error: WorldSourceError) -> Self {
        Self::WorldSource(error)
    }
}

fn default_terrain_model_cache() -> Result<PathBuf, WorldServiceSourceError> {
    let home = std::env::var_os("HOME").ok_or(WorldServiceSourceError::MissingHomeDirectory)?;
    let home = PathBuf::from(home);
    if cfg!(target_os = "macos") {
        Ok(home.join("Library/Caches/voxels/terrain-diffusion"))
    } else if let Some(cache) = std::env::var_os("XDG_CACHE_HOME") {
        Ok(PathBuf::from(cache).join("voxels/terrain-diffusion"))
    } else {
        Ok(home.join(".cache/voxels/terrain-diffusion"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use voxels_world::{
        MacroBlockBatch, MacroBlockRequest, VOXEL_SIZE_METRES, WorldProductPriority,
        WorldSourceKind,
    };

    const CONFIG_TOML: &str = r#"
schema_version = 25
world_id = "07070707-0707-0707-0707-070707070707"
world_seed = 42
source = "procedural-v16"

[transport]
listen = "127.0.0.1:9777"
allowed_origins = ["http://127.0.0.1:5173"]
auth_subprotocol_token = "test-token"
max_frame_bytes = 16777216
max_queued_outbound_bytes_per_client = 33554432
outbound_bandwidth_floor_bytes_per_second = 98304
outbound_bandwidth_ceiling_bytes_per_second = 4194304
outbound_bandwidth_burst_bytes = 65536
outbound_queue_delay_target_ms = 25
outbound_feedback_timeout_ms = 3000
outbound_max_frame_fragment_bytes = 32768
max_in_flight_batches = 16
max_connections = 512
global_queue_capacity = 128
product_cache_bytes = 268435456
response_cache_bytes = 67108864
generation_workers = 8
generation_workers_per_client = 2
collision_generation_workers_per_client = 1

[presence]
broadcast_interval_ms = 33
max_players = 512
max_pose_updates_per_second = 60
spatial_cell_metres = 64
interest_radius_metres = 256
interest_hysteresis_metres = 32
near_radius_metres = 32
mid_radius_metres = 96
near_update_interval_ms = 50
mid_update_interval_ms = 100
far_update_interval_ms = 250
max_records_per_delta = 64
prediction_error_centimetres = 25
look_error_milliradians = 175

[gameplay]
allow_gliding = true
allow_spectator_mode = false
interaction_reach_centimetres = 500
interaction_latency_slack_centimetres = 100
interaction_pose_max_age_ms = 1000
max_horizontal_speed_centimetres_per_second = 900
max_vertical_speed_centimetres_per_second = 2000
spectator_max_horizontal_speed_centimetres_per_second = 15000
spectator_max_vertical_speed_centimetres_per_second = 15000
movement_slack_centimetres = 100
movement_credit_window_ms = 500
spectator_movement_credit_window_ms = 2000

[environment]
day_length_seconds = 1200.0
world_day_number_at_unix_epoch = 0
day_fraction_at_unix_epoch = 0.72
days_per_year = 365.2422
moon_sidereal_orbit_days = 27.321661
moon_orbit_phase_at_world_epoch = 0.0
planet_circumference_metres = 40075016.0
axial_tilt_degrees = 23.4393
moon_orbit_inclination_degrees = 5.145
celestial_seed = 1470258925
celestial_revision = 1
weather_cycle_seconds = 900.0
weather_fraction_at_unix_epoch = 0.08
cloud_offset_metres_at_unix_epoch = [0.0, 0.0]
cloud_velocity_metres_per_second = [5.5, 1.6]
cloud_coverage = 0.24
cloud_base_metres = 550.0
cloud_top_metres = 1800.0
weather_seed = 1474984685
weather_revision = 1

[edits]
database = "world-state/schema-{edit_schema}/{world_id}-{source_hash}.sqlite3"
change_queue_capacity = 1024

[spawn]
xz_voxels = [0, 0]
pillar_height_voxels = 50
pillar_radius_voxels = 25
protection_radius_voxels = 64
pillar_material = "Stone"

[terrain_diffusion]
precision = "float16"
world_origin_voxels = [1200, -900]
horizontal_scale = 2
latent_window = [-64, 128]
quality_histogram = [0.0, 0.0, 0.0, 1.0, 1.5]
sea_level_voxels = 52
"#;

    fn test_config(source: WorldSourceMode) -> WorldServiceConfig {
        WorldServiceConfig {
            world_seed: 42,
            source,
            ..WorldServiceConfig::default()
        }
    }

    #[test]
    fn config_round_trips_through_human_readable_toml() {
        let config = test_config(WorldSourceMode::ProceduralV16);
        assert_eq!(config.source, WorldSourceMode::ProceduralV16);
        assert_eq!(config.world_seed, 42);
        assert_eq!(
            config.terrain_diffusion.precision,
            TerrainModelPrecision::Float16
        );
        let serialized = config.to_toml().expect("serializable config");
        assert_eq!(WorldServiceConfig::from_toml(&serialized), Ok(config));
    }

    #[test]
    fn checked_in_world_service_config_is_strict_and_valid() {
        let config = WorldServiceConfig::from_toml(include_str!("../../config/world-service.toml"));
        assert!(config.is_ok());
    }

    #[test]
    fn documented_complete_schema_matches_checked_in_development_config() {
        let documentation = include_str!("../../docs/world-service-config.md");
        let (_, documented_schema) = documentation
            .split_once("The complete schema is:\n\n```toml\n")
            .expect("complete schema heading and TOML fence");
        let (documented_schema, _) = documented_schema
            .split_once("\n```")
            .expect("closing TOML fence");

        let documented =
            WorldServiceConfig::from_toml(documented_schema).expect("documented complete schema");
        let checked_in =
            WorldServiceConfig::from_toml(include_str!("../../config/world-service.toml"))
                .expect("checked-in development config");
        assert_eq!(documented, checked_in);
    }

    #[test]
    fn checked_in_production_config_requires_signed_sessions_and_public_https_origins() {
        let config = WorldServiceConfig::from_toml(include_str!(
            "../../config/world-service.production.toml"
        ))
        .expect("production config");
        assert!(config.transport.allow_non_loopback);
        assert_eq!(
            config.transport.auth_session_hmac_key_env.as_deref(),
            Some("VOXELS_SESSION_SIGNING_KEY")
        );
        assert!(
            config
                .transport
                .allowed_origins
                .iter()
                .all(|origin| origin.starts_with("https://"))
        );
        assert_eq!(
            config.edits.database.as_path(),
            std::path::Path::new("/data/world-state.sqlite3")
        );
    }

    #[test]
    fn default_spawn_is_a_five_metre_raised_platform() {
        let spawn = SpawnConfig::default();
        let checked_in =
            WorldServiceConfig::from_toml(include_str!("../../config/world-service.toml"))
                .expect("checked-in config");
        assert_eq!(checked_in.spawn, spawn);
        assert_eq!(spawn.pillar_height_voxels, 50);
        assert_eq!(spawn.pillar_radius_voxels, 25);

        let height_metres = f32::from(spawn.pillar_height_voxels) * VOXEL_SIZE_METRES;
        let diameter_metres =
            f32::from(spawn.pillar_radius_voxels) * 2.0 * VOXEL_SIZE_METRES + VOXEL_SIZE_METRES;
        assert!((height_metres - 5.0).abs() < f32::EPSILON);
        assert!((diameter_metres - 5.0).abs() <= VOXEL_SIZE_METRES + f32::EPSILON);
        assert!(spawn.protection_radius_voxels > u16::from(spawn.pillar_radius_voxels));
    }

    #[test]
    fn toml_contract_parses_typed_terrain_origins() {
        let config = WorldServiceConfig::from_toml(CONFIG_TOML).expect("valid config");
        assert_eq!(config.terrain_diffusion.world_origin_voxels, [1_200, -900]);
        assert_eq!(config.terrain_diffusion.horizontal_scale, 2);
        assert_eq!(config.terrain_diffusion.latent_window, [-64, 128]);
        assert_eq!(
            config.terrain_diffusion.quality_histogram,
            [0.0, 0.0, 0.0, 1.0, 1.5]
        );
    }

    #[test]
    fn typed_fixture_selects_terrain_without_rewriting_toml() {
        let config = test_config(WorldSourceMode::TerrainDiffusion30m);
        assert_eq!(config.source, WorldSourceMode::TerrainDiffusion30m);
    }

    #[test]
    fn schema_and_unknown_fields_are_rejected() {
        let wrong_schema = CONFIG_TOML.replace("schema_version = 25", "schema_version = 24");
        assert_eq!(
            WorldServiceConfig::from_toml(&wrong_schema),
            Err(WorldServiceConfigError::UnsupportedSchema {
                expected: WORLD_SERVICE_CONFIG_SCHEMA_VERSION,
                found: WORLD_SERVICE_CONFIG_SCHEMA_VERSION - 1,
            })
        );
        let unknown = format!("{CONFIG_TOML}\nunknown = true\n");
        assert!(matches!(
            WorldServiceConfig::from_toml(&unknown),
            Err(WorldServiceConfigError::Parse(_))
        ));
        let wrong_origin_shape = CONFIG_TOML.replace(
            "world_origin_voxels = [1200, -900]",
            "world_origin_voxels = [1200]",
        );
        assert!(matches!(
            WorldServiceConfig::from_toml(&wrong_origin_shape),
            Err(WorldServiceConfigError::Parse(_))
        ));

        for missing in [
            "outbound_bandwidth_floor_bytes_per_second = 98304\n",
            "outbound_bandwidth_ceiling_bytes_per_second = 4194304\n",
            "outbound_bandwidth_burst_bytes = 65536\n",
            "outbound_queue_delay_target_ms = 25\n",
            "outbound_feedback_timeout_ms = 3000\n",
            "outbound_max_frame_fragment_bytes = 32768\n",
            "max_connections = 512\n",
            "product_cache_bytes = 268435456\n",
            "response_cache_bytes = 67108864\n",
            "broadcast_interval_ms = 33\n",
            "interaction_reach_centimetres = 500\n",
            "spectator_max_horizontal_speed_centimetres_per_second = 15000\n",
            "spectator_max_vertical_speed_centimetres_per_second = 15000\n",
            "spectator_movement_credit_window_ms = 2000\n",
            "planet_circumference_metres = 40075016.0\n",
            "xz_voxels = [0, 0]\n",
            "precision = \"float16\"\n",
            "horizontal_scale = 2\n",
            "quality_histogram = [0.0, 0.0, 0.0, 1.0, 1.5]\n",
        ] {
            let incomplete = CONFIG_TOML.replace(missing, "");
            assert!(matches!(
                WorldServiceConfig::from_toml(&incomplete),
                Err(WorldServiceConfigError::Parse(_))
            ));
        }
    }

    #[test]
    fn transport_rejects_non_loopback_and_unbounded_inputs() {
        let mut config = test_config(WorldSourceMode::ProceduralV16);
        config.transport.listen = SocketAddr::from(([0, 0, 0, 0], 9_777));
        assert!(matches!(
            config.validate(),
            Err(WorldServiceConfigError::ListenIsNotLoopback(_))
        ));

        config.transport.listen = SocketAddr::from(([127, 0, 0, 1], 9_777));
        config.transport.max_in_flight_batches = 0;
        assert!(matches!(
            config.validate(),
            Err(WorldServiceConfigError::InvalidMaxInFlightBatches { .. })
        ));

        config.transport.max_in_flight_batches = 1;
        config.transport.generation_workers_per_client =
            config.transport.generation_workers.saturating_add(1);
        assert!(matches!(
            config.validate(),
            Err(WorldServiceConfigError::InvalidConcurrency(_))
        ));

        config.transport.generation_workers_per_client = 1;
        config.transport.collision_generation_workers_per_client = 0;
        assert!(matches!(
            config.validate(),
            Err(WorldServiceConfigError::InvalidConcurrency(_))
        ));

        config.transport.collision_generation_workers_per_client = 1;
        config.transport.response_cache_bytes = 256 * 1024 * 1024 + 1;
        assert!(matches!(
            config.validate(),
            Err(WorldServiceConfigError::InvalidConcurrency(_))
        ));

        config.transport.response_cache_bytes = 64 * 1024 * 1024;
        config.transport.auth_subprotocol_token = "invalid token".to_owned();
        assert_eq!(
            config.validate(),
            Err(WorldServiceConfigError::InvalidAuthSubprotocolToken)
        );
    }

    #[test]
    fn environment_clock_and_weather_inputs_are_strictly_bounded() {
        let mut config = WorldServiceConfig::default();
        config.environment.day_fraction_at_unix_epoch = 1.0;
        assert!(matches!(
            config.validate(),
            Err(WorldServiceConfigError::InvalidEnvironment(_))
        ));

        let mut config = WorldServiceConfig::default();
        config.environment.cloud_velocity_metres_per_second = [f32::NAN, 0.0];
        assert!(matches!(
            config.validate(),
            Err(WorldServiceConfigError::InvalidEnvironment(_))
        ));

        let mut config = WorldServiceConfig::default();
        config.environment.weather_revision = 0;
        assert!(matches!(
            config.validate(),
            Err(WorldServiceConfigError::InvalidEnvironment(_))
        ));

        let mut config = WorldServiceConfig::default();
        config.environment.planet_circumference_metres = 0.0;
        assert!(matches!(
            config.validate(),
            Err(WorldServiceConfigError::InvalidEnvironment(_))
        ));

        let mut config = WorldServiceConfig::default();
        config.environment.celestial_revision = 0;
        assert!(matches!(
            config.validate(),
            Err(WorldServiceConfigError::InvalidEnvironment(_))
        ));
    }

    #[test]
    fn terrain_diffusion_scale_is_bounded() {
        let mut config = test_config(WorldSourceMode::TerrainDiffusion30m);
        config.terrain_diffusion.horizontal_scale = 0;
        assert_eq!(
            config.validate(),
            Err(WorldServiceConfigError::InvalidTerrainDiffusion(
                TerrainDiffusionError::InvalidHorizontalScale(0)
            ))
        );
    }

    #[test]
    fn terrain_diffusion_latent_window_keeps_model_coordinates_representable() {
        let mut config = test_config(WorldSourceMode::TerrainDiffusion30m);
        for coordinate in [
            voxels_world_terrain_diffusion::MIN_LATENT_WINDOW_COORDINATE,
            voxels_world_terrain_diffusion::MAX_LATENT_WINDOW_COORDINATE,
        ] {
            config.terrain_diffusion.latent_window = [coordinate; 2];
            config.validate().expect("boundary window is valid");
        }

        let coordinate = voxels_world_terrain_diffusion::MAX_LATENT_WINDOW_COORDINATE + 1;
        config.terrain_diffusion.latent_window = [coordinate, 0];
        assert_eq!(
            config.validate(),
            Err(WorldServiceConfigError::InvalidTerrainDiffusion(
                TerrainDiffusionError::InvalidLatentWindow([coordinate, 0])
            ))
        );
    }

    #[test]
    fn terrain_diffusion_quality_histogram_is_finite_and_bounded() {
        let mut config = test_config(WorldSourceMode::TerrainDiffusion30m);
        config.terrain_diffusion.quality_histogram[2] = f32::NAN;
        assert_eq!(
            config.validate(),
            Err(WorldServiceConfigError::InvalidTerrainDiffusion(
                TerrainDiffusionError::InvalidQualityHistogram
            ))
        );

        config.terrain_diffusion.quality_histogram[2] = 10.01;
        assert_eq!(
            config.validate(),
            Err(WorldServiceConfigError::InvalidTerrainDiffusion(
                TerrainDiffusionError::InvalidQualityHistogram
            ))
        );
    }

    #[test]
    fn relative_model_cache_is_resolved_from_the_config_file() {
        let config = WorldServiceConfig {
            terrain_diffusion: TerrainDiffusionProviderConfig {
                model_cache: Some(PathBuf::from("models")),
                ..TerrainDiffusionProviderConfig::default()
            },
            ..WorldServiceConfig::default()
        };
        let loaded = LoadedWorldServiceConfig::from_config(
            config,
            "test-fixtures/voxels-config/world-service.toml",
        )
        .expect("loaded config");
        assert_eq!(
            loaded.terrain_model_root(),
            Ok(PathBuf::from("test-fixtures/voxels-config/models").join(MODEL_REVISION))
        );
    }

    #[test]
    fn edit_database_path_expands_compatibility_tokens_but_keeps_explicit_paths_strict() {
        let source_hash = WorldSourceIdentityHash::from_bytes([0xab; 32]);
        let mut config = test_config(WorldSourceMode::ProceduralV16);
        config.edits.database =
            PathBuf::from("state/schema-{edit_schema}/{world_id}-{source_hash}.sqlite3");
        let expected_world_id = config.world_id.to_string();
        let loaded = LoadedWorldServiceConfig::from_config(
            config.clone(),
            "test-fixtures/voxels-config/world-service.toml",
        )
        .expect("loaded config");
        assert_eq!(
            loaded.edit_database_path(source_hash),
            PathBuf::from(format!(
                "test-fixtures/voxels-config/state/schema-{EDIT_DATABASE_SCHEMA_VERSION}/{expected_world_id}-{source_hash}.sqlite3"
            ))
        );

        config.edits.database = PathBuf::from("persistent-world.sqlite3");
        let loaded = LoadedWorldServiceConfig::from_config(
            config,
            "test-fixtures/voxels-config/world-service.toml",
        )
        .expect("loaded config");
        assert_eq!(
            loaded.edit_database_path(source_hash),
            PathBuf::from("test-fixtures/voxels-config/persistent-world.sqlite3")
        );
    }

    #[test]
    fn procedural_factory_is_source_neutral_and_generates_macro_fields() {
        let config = test_config(WorldSourceMode::ProceduralV16);
        let loaded = LoadedWorldServiceConfig::from_config(config, "world-service.toml")
            .expect("loaded config");
        let source = loaded.build_macro_source().expect("procedural source");
        assert_eq!(
            source.identity().source_kind,
            WorldSourceKind::ProceduralV16
        );
        let result = source
            .request_blocks(MacroBlockBatch {
                priority: WorldProductPriority::VisibleSurface,
                requests: vec![MacroBlockRequest {
                    origin: [0, 0],
                    sample_shape: [2, 2],
                    stride_voxels: 300,
                }],
            })
            .expect("macro block");
        assert_eq!(result.blocks.len(), 1);
        assert_eq!(result.blocks[0].elevation_voxels.len(), 4);
        assert!(result.blocks[0].validity.iter().all(|valid| *valid));
    }

    #[cfg(not(feature = "terrain-metal"))]
    #[test]
    fn terrain_selection_never_silently_falls_back_to_procedural() {
        let config = test_config(WorldSourceMode::TerrainDiffusion30m);
        let loaded = LoadedWorldServiceConfig::from_config(config, "world-service.toml")
            .expect("loaded config");
        assert!(matches!(
            loaded.build_macro_source(),
            Err(WorldServiceSourceError::TerrainMetalFeatureDisabled)
        ));
    }
}
