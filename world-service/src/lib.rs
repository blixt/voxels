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
    HeightfieldWorldSource, MacroTerrainSource, ProceduralWorldSource, SEA_LEVEL_VOXELS, WorldId,
    WorldSourceEngine, WorldSourceError,
};
use voxels_world_terrain_diffusion::{MODEL_REVISION, TerrainDiffusionError};
#[cfg(all(feature = "terrain-metal", target_os = "macos"))]
use voxels_world_terrain_diffusion::{
    TerrainDiffusionConfig, TerrainPrecision, validate_model_root,
};

mod edits;
mod presence;
pub mod server;

pub use server::{
    PRESENCE_WEBSOCKET_PATH, WORLD_WEBSOCKET_PATH, WORLD_WEBSOCKET_PROTOCOL, WorldServer,
    WorldServerError, serve_loaded_config,
};

pub const WORLD_SERVICE_CONFIG_SCHEMA_VERSION: u32 = 8;

const DEFAULT_WORLD_ID: [u8; 16] = [
    0x76, 0x6f, 0x78, 0x65, 0x6c, 0x73, 0x40, 0x6c, 0x6f, 0x63, 0x61, 0x6c, 0x00, 0x00, 0x00, 0x01,
];
const MAX_CONFIGURED_IN_FLIGHT_BATCHES: u16 = 1_024;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoopbackTransportConfig {
    pub listen: SocketAddr,
    pub allowed_origins: Vec<String>,
    /// Offered as a second `Sec-WebSocket-Protocol` value so it is not placed in a URL or log.
    pub auth_subprotocol_token: String,
    pub max_frame_bytes: usize,
    pub max_outbound_bytes_per_client: usize,
    /// Per-connection request window negotiated with each browser.
    pub max_in_flight_batches: u16,
    /// Hard process-wide connection bound. Each accepted WebSocket holds one permit.
    pub max_connections: u16,
    /// Bounded process-wide queue shared by all clients.
    pub global_queue_capacity: u16,
    /// LRU budget for immutable compressed world-product batch responses.
    pub product_cache_bytes: usize,
    /// Maximum blocking generation batches executing across all clients.
    pub generation_workers: u16,
    /// Fairness guard: one connection cannot occupy the whole worker pool.
    pub generation_workers_per_client: u16,
}

impl Default for LoopbackTransportConfig {
    fn default() -> Self {
        Self {
            listen: SocketAddr::from(([127, 0, 0, 1], 9_777)),
            allowed_origins: vec![
                "http://127.0.0.1:5173".to_owned(),
                "http://localhost:5173".to_owned(),
            ],
            auth_subprotocol_token: "replace-with-a-random-local-token".to_owned(),
            max_frame_bytes: MAX_PROTOCOL_FRAME_BYTES,
            max_outbound_bytes_per_client: 32 * 1024 * 1024,
            max_in_flight_batches: 16,
            max_connections: 512,
            global_queue_capacity: 8_192,
            product_cache_bytes: 256 * 1024 * 1024,
            generation_workers: 8,
            generation_workers_per_client: 2,
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
            max_players: 512,
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
    /// Fixed movement credit that absorbs simulation and packet-timing jitter.
    pub movement_slack_centimetres: u16,
    /// Maximum delayed-motion credit that can accumulate while pose packets are absent.
    pub movement_credit_window_ms: u16,
    /// Initial quantity granted for every non-Air material when a player is first seen.
    pub starting_units_per_material: u32,
}

impl Default for GameplayConfig {
    fn default() -> Self {
        Self {
            interaction_reach_centimetres: 500,
            interaction_latency_slack_centimetres: 100,
            interaction_pose_max_age_ms: 1_000,
            max_horizontal_speed_centimetres_per_second: 900,
            max_vertical_speed_centimetres_per_second: 1_200,
            movement_slack_centimetres: 100,
            movement_credit_window_ms: 500,
            starting_units_per_material: 64,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpawnConfig {
    /// Canonical voxel X/Z coordinate sampled when constructing `WorldOpened`.
    pub xz_voxels: [i32; 2],
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditPersistenceConfig {
    /// Native SQLite file containing the authoritative sparse edit journal. Relative paths are
    /// resolved from the directory containing the service configuration file.
    pub database: PathBuf,
    /// Bounded per-client change queue. Overflow forces an explicit product resynchronization.
    pub change_queue_capacity: u16,
}

impl Default for EditPersistenceConfig {
    fn default() -> Self {
        Self {
            database: PathBuf::from("../tmp/world-state-v3.sqlite3"),
            change_queue_capacity: 256,
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerrainDiffusionProviderConfig {
    /// Cache root containing the immutable model-revision directory. Relative paths are resolved
    /// from the directory containing the service configuration file.
    pub model_cache: Option<PathBuf>,
    pub precision: TerrainModelPrecision,
    /// Canonical voxel X/Z coordinate where the finite generated tile is placed.
    pub world_origin_voxels: [i32; 2],
    /// Terrain Diffusion model-grid row/column used to key spatial sampling and noise.
    pub model_origin: [i32; 2],
    /// Flood height used by the fidelity-honest macro-heightfield voxel composer.
    pub sea_level_voxels: i32,
}

impl Default for TerrainDiffusionProviderConfig {
    fn default() -> Self {
        Self {
            model_cache: None,
            precision: TerrainModelPrecision::Float16,
            world_origin_voxels: [0, 0],
            model_origin: [0, 0],
            sea_level_voxels: SEA_LEVEL_VOXELS,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldServiceConfig {
    pub schema_version: u32,
    pub world_id: Uuid,
    pub world_seed: u64,
    pub source: WorldSourceMode,
    pub transport: LoopbackTransportConfig,
    pub presence: PresenceConfig,
    pub gameplay: GameplayConfig,
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
        if !self.transport.listen.ip().is_loopback() {
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
        if !valid_websocket_protocol_token(&self.transport.auth_subprotocol_token) {
            return Err(WorldServiceConfigError::InvalidAuthSubprotocolToken);
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
        if self.transport.max_outbound_bytes_per_client < self.transport.max_frame_bytes
            || self.transport.max_outbound_bytes_per_client > 256 * 1024 * 1024
        {
            return Err(WorldServiceConfigError::InvalidConcurrency(
                "max_outbound_bytes_per_client must fit at least one frame and stay at most 256 MiB",
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
        if self.transport.generation_workers == 0
            || self.transport.generation_workers_per_client == 0
            || self.transport.generation_workers_per_client > self.transport.generation_workers
        {
            return Err(WorldServiceConfigError::InvalidConcurrency(
                "generation worker limits must be nonzero and per-client must not exceed global",
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
            || !(1..=300).contains(&self.gameplay.movement_slack_centimetres)
            || !(100..=2_000).contains(&self.gameplay.movement_credit_window_ms)
        {
            return Err(WorldServiceConfigError::InvalidGameplay(
                "movement speeds, slack, or credit window is invalid",
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
    EmptyAllowedOrigins,
    InvalidAllowedOrigin(String),
    InvalidAuthSubprotocolToken,
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
            Self::EmptyAllowedOrigins => {
                formatter.write_str("world-service transport requires at least one allowed origin")
            }
            Self::InvalidAllowedOrigin(origin) => {
                write!(formatter, "invalid allowed HTTP origin {origin:?}")
            }
            Self::InvalidAuthSubprotocolToken => formatter.write_str(
                "auth_subprotocol_token must be a non-empty RFC WebSocket protocol token of at most 128 bytes",
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
            Self::InvalidEdits(reason) => {
                write!(formatter, "invalid world-service edits: {reason}")
            }
        }
    }
}

impl std::error::Error for WorldServiceConfigError {}

/// A validated configuration retaining its file location for relative-path resolution.
#[derive(Clone, Debug, Eq, PartialEq)]
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

    pub fn edit_database_path(&self) -> PathBuf {
        if self.config.edits.database.is_absolute() {
            self.config.edits.database.clone()
        } else {
            self.path.parent().map_or_else(
                || self.config.edits.database.clone(),
                |parent| parent.join(&self.config.edits.database),
            )
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
        validate_model_root(&model_root)?;
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
            model_origin: self.config.terrain_diffusion.model_origin,
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
    use voxels_world::{MacroBlockBatch, MacroBlockRequest, WorldProductPriority, WorldSourceKind};

    const CONFIG_TOML: &str = r#"
schema_version = 8
world_id = "07070707-0707-0707-0707-070707070707"
world_seed = 42
source = "procedural-v16"

[transport]
listen = "127.0.0.1:9777"
allowed_origins = ["http://127.0.0.1:5173"]
auth_subprotocol_token = "test-token"
max_frame_bytes = 16777216
max_outbound_bytes_per_client = 33554432
max_in_flight_batches = 16
max_connections = 512
global_queue_capacity = 128
product_cache_bytes = 268435456
generation_workers = 8
generation_workers_per_client = 2

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
interaction_reach_centimetres = 500
interaction_latency_slack_centimetres = 100
interaction_pose_max_age_ms = 1000
max_horizontal_speed_centimetres_per_second = 900
max_vertical_speed_centimetres_per_second = 1200
movement_slack_centimetres = 100
movement_credit_window_ms = 500
starting_units_per_material = 64

[edits]
database = "world-state-v3.sqlite3"
change_queue_capacity = 256

[spawn]
xz_voxels = [0, 0]

[terrain_diffusion]
precision = "float16"
world_origin_voxels = [1200, -900]
model_origin = [-64, 128]
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
    fn toml_contract_parses_typed_terrain_origins() {
        let config = WorldServiceConfig::from_toml(CONFIG_TOML).expect("valid config");
        assert_eq!(config.terrain_diffusion.world_origin_voxels, [1_200, -900]);
        assert_eq!(config.terrain_diffusion.model_origin, [-64, 128]);
    }

    #[test]
    fn typed_fixture_selects_terrain_without_rewriting_toml() {
        let config = test_config(WorldSourceMode::TerrainDiffusion30m);
        assert_eq!(config.source, WorldSourceMode::TerrainDiffusion30m);
    }

    #[test]
    fn schema_and_unknown_fields_are_rejected() {
        let wrong_schema = CONFIG_TOML.replace("schema_version = 8", "schema_version = 7");
        assert_eq!(
            WorldServiceConfig::from_toml(&wrong_schema),
            Err(WorldServiceConfigError::UnsupportedSchema {
                expected: 8,
                found: 7,
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
            "max_connections = 512\n",
            "product_cache_bytes = 268435456\n",
            "broadcast_interval_ms = 33\n",
            "interaction_reach_centimetres = 500\n",
            "xz_voxels = [0, 0]\n",
            "precision = \"float16\"\n",
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
        config.transport.auth_subprotocol_token = "invalid token".to_owned();
        assert_eq!(
            config.validate(),
            Err(WorldServiceConfigError::InvalidAuthSubprotocolToken)
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
