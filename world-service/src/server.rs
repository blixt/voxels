//! Bounded binary-WebSocket transport for canonical world products.

use crate::{
    EnvironmentConfig, LoadedWorldServiceConfig, WorldServiceConfig, WorldServiceConfigError,
    WorldServiceSourceError,
    edits::{ChunkEditSnapshot, EditAuthority, LoadedPlayer, ProtectedSpawn, SurfaceEditSnapshot},
    generation_limiter::PriorityGenerationLimiter,
    presence::{PoseAdmission, PresenceHub, PresenceStreamState},
    traffic::{ClientTrafficRegistry, ClientTrafficShaper, TrafficPriority},
};
use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::header::{ORIGIN, SEC_WEBSOCKET_PROTOCOL};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::serve::ListenerExt;
use futures_util::stream::{FuturesUnordered, SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::fmt;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::net::TcpListener;
use tokio::sync::{Mutex as AsyncMutex, OwnedSemaphorePermit, Semaphore, mpsc, watch};
#[cfg(test)]
use voxels_world::protocol::DigVolume;
use voxels_world::protocol::{
    ChunkBatchItem, ChunkBatchRequest, EditSessionId, EncodedChunkBatchItem,
    EncodedSurfaceTileBatchItem, FRAME_FRAGMENT_OVERHEAD_BYTES, PlayerIdentity, PlayerResume,
    PresenceOpened, PresencePong, ResyncRequired, SpawnPoint, SurfaceTileBatchItem,
    SurfaceTileBatchRequest, VoxelMutation, WorldCapabilities, WorldEnvironmentSnapshot,
    WorldOpened, cancel_kind, chunk_batch_kind, clone_message_with_request_id, decode_cancel,
    decode_chunk_batch, decode_edit_command, decode_open_presence, decode_open_world,
    decode_player_pose, decode_presence_ping, decode_surface_tile_batch, edit_command_kind,
    encode_chunk_batch_item, encode_chunk_batch_result_from_items, encode_edit_commit,
    encode_error, encode_frame_fragment, encode_presence_opened, encode_presence_pong,
    encode_resync_required, encode_surface_tile_batch_item,
    encode_surface_tile_batch_result_from_items, encode_world_opened, message_kind,
    message_request_id, open_presence_kind, open_world_kind, player_pose_kind, presence_ping_kind,
    surface_tile_batch_kind,
};
use voxels_world::{
    CHUNK_EDGE, ChunkCoord, Material, MeshingHalo, SurfaceSampleBlockRequest, WORLD_SCHEMA_VERSION,
    WorldManifest, WorldManifestError, WorldProduct, WorldProductBatch, WorldProductPriority,
    WorldProductRequest, WorldSourceEngine, WorldSourceError,
};

pub const WORLD_WEBSOCKET_PATH: &str = "/v24/world";
pub const PRESENCE_WEBSOCKET_PATH: &str = "/v24/presence";
pub const WORLD_WEBSOCKET_PROTOCOL: &str = "voxels.world.v24";
const DEFAULT_PLAYER_EYE_HEIGHT_METRES: f32 = 1.62;
const PREFETCH_WORKER_DIVISOR: usize = 4;
const RESPONSE_CACHE_BUDGET_DIVISOR: usize = 4;
const CLOUD_PERIOD_METRES: f64 = 1_280_000.0;

/// Prepared server state. Source construction and spawn coverage validation happen before bind.
pub struct WorldServer {
    router: Router,
}

impl WorldServer {
    pub fn from_loaded_config(loaded: &LoadedWorldServiceConfig) -> Result<Self, WorldServerError> {
        let source = loaded.build_world_source()?;
        let edit_database = loaded.edit_database_path(source.identity().identity_hash());
        Self::build(loaded.config().clone(), source, Some(edit_database))
    }

    pub fn new(
        config: WorldServiceConfig,
        source: Box<dyn WorldSourceEngine>,
    ) -> Result<Self, WorldServerError> {
        Self::build(config, source, None)
    }

    fn build(
        config: WorldServiceConfig,
        source: Box<dyn WorldSourceEngine>,
        edit_database: Option<std::path::PathBuf>,
    ) -> Result<Self, WorldServerError> {
        config.validate()?;
        let source = Arc::<dyn WorldSourceEngine>::from(source);
        let world = prepare_world(&config, source.as_ref())?;
        let edits = match edit_database {
            Some(path) => EditAuthority::open(
                &path,
                world.manifest.world_id,
                source.as_ref(),
                config.edits.change_queue_capacity,
            ),
            None => EditAuthority::in_memory(
                world.manifest.world_id,
                source.as_ref(),
                config.edits.change_queue_capacity,
            ),
        }
        .map_err(|error| WorldServerError::Edits(error.to_string()))?;
        edits.install_protected_spawn(world.protected_spawn.clone());
        let capacity = usize::from(config.transport.global_queue_capacity);
        let (generation_tx, generation_rx) = mpsc::channel(capacity);
        let generation_limiter =
            PriorityGenerationLimiter::new(usize::from(config.transport.generation_workers));
        let prefetch_workers =
            (usize::from(config.transport.generation_workers) / PREFETCH_WORKER_DIVISOR).max(1);
        let prefetch_semaphore = Arc::new(Semaphore::new(prefetch_workers));
        tokio::spawn(run_generation_dispatcher(
            generation_rx,
            Arc::clone(&source),
            generation_limiter,
            prefetch_semaphore,
            config.transport.max_frame_bytes,
            config.transport.product_cache_bytes,
        ));

        let presence = PresenceHub::new(config.presence, config.gameplay)
            .map_err(WorldServerError::Presence)?;
        let environment = EnvironmentAuthority::new(config.environment, presence.now_ms());
        let state = Arc::new(ServerState {
            allowed_origins: config.transport.allowed_origins,
            auth_subprotocol_token: config.transport.auth_subprotocol_token,
            max_frame_bytes: config.transport.max_frame_bytes,
            max_queued_outbound_bytes_per_client: config
                .transport
                .max_queued_outbound_bytes_per_client,
            max_in_flight_batches: config.transport.max_in_flight_batches,
            generation_workers_per_client: config.transport.generation_workers_per_client,
            collision_generation_workers_per_client: config
                .transport
                .collision_generation_workers_per_client,
            connections: Arc::new(Semaphore::new(usize::from(
                config.transport.max_connections,
            ))),
            presence_connections: Arc::new(Semaphore::new(usize::from(
                config.transport.max_connections,
            ))),
            traffic: ClientTrafficRegistry::new(
                config.transport.outbound_bandwidth_floor_bytes_per_second,
                config.transport.outbound_bandwidth_ceiling_bytes_per_second,
                config.transport.outbound_bandwidth_burst_bytes,
                Duration::from_millis(u64::from(config.transport.outbound_queue_delay_target_ms)),
                Duration::from_millis(u64::from(config.transport.outbound_feedback_timeout_ms)),
                config.transport.outbound_max_frame_fragment_bytes,
            ),
            world,
            environment,
            presence,
            source,
            edits,
            generation_tx,
        });
        let router = Router::new()
            .route(WORLD_WEBSOCKET_PATH, get(world_websocket_endpoint))
            .route(PRESENCE_WEBSOCKET_PATH, get(presence_websocket_endpoint))
            .with_state(state);
        Ok(Self { router })
    }

    pub async fn serve(self, listener: TcpListener) -> Result<(), WorldServerError> {
        let address = listener
            .local_addr()
            .map_err(|error| WorldServerError::Listener(error.to_string()))?;
        if !address.ip().is_loopback() {
            return Err(WorldServerError::NonLoopbackListener(address));
        }
        let listener = listener.tap_io(|stream| {
            let _ = stream.set_nodelay(true);
        });
        axum::serve(listener, self.router)
            .await
            .map_err(|error| WorldServerError::Serve(error.to_string()))
    }
}

pub async fn serve_loaded_config(
    loaded: &LoadedWorldServiceConfig,
) -> Result<(), WorldServerError> {
    let address = loaded.config().transport.listen;
    let server = WorldServer::from_loaded_config(loaded)?;
    let listener = TcpListener::bind(address)
        .await
        .map_err(|error| WorldServerError::Bind {
            address,
            reason: error.to_string(),
        })?;
    server.serve(listener).await
}

#[derive(Debug)]
pub enum WorldServerError {
    Config(WorldServiceConfigError),
    Source(WorldServiceSourceError),
    Spawn(WorldSourceError),
    InvalidSpawnProduct,
    Presence(String),
    Edits(String),
    Manifest(WorldManifestError),
    Bind { address: SocketAddr, reason: String },
    Listener(String),
    NonLoopbackListener(SocketAddr),
    Serve(String),
}

impl fmt::Display for WorldServerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(error) => error.fmt(formatter),
            Self::Source(error) => error.fmt(formatter),
            Self::Spawn(error) => write!(formatter, "configured spawn is unavailable: {error}"),
            Self::InvalidSpawnProduct => {
                formatter.write_str("world source returned a mismatched spawn surface product")
            }
            Self::Presence(reason) => write!(formatter, "could not initialize presence: {reason}"),
            Self::Edits(error) => write!(formatter, "could not initialize edit authority: {error}"),
            Self::Manifest(error) => write!(formatter, "invalid world manifest: {error}"),
            Self::Bind { address, reason } => {
                write!(
                    formatter,
                    "could not bind world-service to {address}: {reason}"
                )
            }
            Self::Listener(reason) => write!(formatter, "could not inspect listener: {reason}"),
            Self::NonLoopbackListener(address) => {
                write!(formatter, "refusing non-loopback listener {address}")
            }
            Self::Serve(reason) => write!(formatter, "world-service failed: {reason}"),
        }
    }
}

impl std::error::Error for WorldServerError {}

impl From<WorldServiceConfigError> for WorldServerError {
    fn from(error: WorldServiceConfigError) -> Self {
        Self::Config(error)
    }
}

impl From<WorldServiceSourceError> for WorldServerError {
    fn from(error: WorldServiceSourceError) -> Self {
        Self::Source(error)
    }
}

impl From<WorldManifestError> for WorldServerError {
    fn from(error: WorldManifestError) -> Self {
        Self::Manifest(error)
    }
}

fn prepare_world(
    config: &WorldServiceConfig,
    source: &dyn WorldSourceEngine,
) -> Result<WorldBootstrap, WorldServerError> {
    let pillar_radius = i32::from(config.spawn.pillar_radius_voxels);
    let sample_edge = u32::from(config.spawn.pillar_radius_voxels)
        .checked_mul(2)
        .and_then(|edge| edge.checked_add(1))
        .ok_or(WorldServerError::InvalidSpawnProduct)?;
    let spawn_request = SurfaceSampleBlockRequest {
        origin: [
            config.spawn.xz_voxels[0]
                .checked_sub(pillar_radius)
                .ok_or(WorldServerError::InvalidSpawnProduct)?,
            config.spawn.xz_voxels[1]
                .checked_sub(pillar_radius)
                .ok_or(WorldServerError::InvalidSpawnProduct)?,
        ],
        sample_shape: [sample_edge, sample_edge],
    };
    let result = source
        .generate_batch(WorldProductBatch {
            priority: WorldProductPriority::CollisionCritical,
            requests: vec![WorldProductRequest::SurfaceSampleBlock(spawn_request)],
        })
        .map_err(WorldServerError::Spawn)?;
    if result.source_identity_hash != source.identity().identity_hash() || result.items.len() != 1 {
        return Err(WorldServerError::InvalidSpawnProduct);
    }
    let item = result
        .items
        .into_iter()
        .next()
        .ok_or(WorldServerError::InvalidSpawnProduct)?;
    if item.request != WorldProductRequest::SurfaceSampleBlock(spawn_request) {
        return Err(WorldServerError::InvalidSpawnProduct);
    }
    let product = item.result.map_err(WorldServerError::Spawn)?;
    let WorldProduct::SurfaceSampleBlock(block) = product else {
        return Err(WorldServerError::InvalidSpawnProduct);
    };
    if block.source_identity_hash != source.identity().identity_hash()
        || block.request != spawn_request
    {
        return Err(WorldServerError::InvalidSpawnProduct);
    }
    let sample = block
        .sample(config.spawn.xz_voxels[0], config.spawn.xz_voxels[1])
        .ok_or(WorldServerError::InvalidSpawnProduct)?;
    let pillar_base = block
        .samples()
        .iter()
        .map(|sample| {
            sample
                .water_level
                .unwrap_or(sample.height)
                .max(sample.height)
        })
        .max()
        .ok_or(WorldServerError::InvalidSpawnProduct)?;
    let pillar_top = pillar_base
        .checked_add(i32::from(config.spawn.pillar_height_voxels))
        .ok_or(WorldServerError::InvalidSpawnProduct)?;
    let mut pillar_overrides = Vec::new();
    for z in spawn_request.origin[1]
        ..spawn_request.origin[1]
            .checked_add(
                i32::try_from(sample_edge).map_err(|_| WorldServerError::InvalidSpawnProduct)?,
            )
            .ok_or(WorldServerError::InvalidSpawnProduct)?
    {
        for x in spawn_request.origin[0]
            ..spawn_request.origin[0]
                .checked_add(
                    i32::try_from(sample_edge)
                        .map_err(|_| WorldServerError::InvalidSpawnProduct)?,
                )
                .ok_or(WorldServerError::InvalidSpawnProduct)?
        {
            let dx = i64::from(x) - i64::from(config.spawn.xz_voxels[0]);
            let dz = i64::from(z) - i64::from(config.spawn.xz_voxels[1]);
            if dx * dx + dz * dz > i64::from(pillar_radius) * i64::from(pillar_radius) {
                continue;
            }
            let column = block
                .sample(x, z)
                .ok_or(WorldServerError::InvalidSpawnProduct)?;
            let first_y = column
                .height
                .checked_add(1)
                .ok_or(WorldServerError::InvalidSpawnProduct)?;
            pillar_overrides.extend((first_y..=pillar_top).map(|y| VoxelMutation {
                coord: voxels_world::VoxelCoord::new(x, y, z),
                material: config.spawn.pillar_material,
            }));
        }
    }
    validate_spawn_chunk(config, source, pillar_top, None)?;
    let manifest = WorldManifest {
        world_id: config.canonical_world_id(),
        seed: config.world_seed,
        world_schema_version: WORLD_SCHEMA_VERSION,
        material_schema_version: Material::SCHEMA_VERSION,
        source: source.identity().clone(),
    };
    manifest.validate()?;
    let capabilities = WorldCapabilities::CANONICAL_CHUNKS
        .union(WorldCapabilities::SURFACE_LOD)
        .union(WorldCapabilities::SERVER_EDITS)
        .union(WorldCapabilities::ENVIRONMENT)
        .union(WorldCapabilities::PLAYER_PRESENCE);
    let capabilities = if config.gameplay.allow_gliding {
        capabilities.union(WorldCapabilities::GLIDING)
    } else {
        capabilities
    };
    let capabilities = if config.gameplay.allow_spectator_mode {
        capabilities.union(WorldCapabilities::SPECTATOR_MODE)
    } else {
        capabilities
    };
    Ok(WorldBootstrap {
        manifest,
        capabilities,
        spawn: SpawnPoint {
            x: config.spawn.xz_voxels[0],
            z: config.spawn.xz_voxels[1],
            height: pillar_top,
            water_level: sample.water_level,
            material: config.spawn.pillar_material,
            region: sample.region,
            moisture: sample.moisture,
            temperature: sample.temperature,
            ridge: sample.ridge,
        },
        protected_spawn: ProtectedSpawn::new(
            config.spawn.xz_voxels,
            config.spawn.protection_radius_voxels,
            pillar_overrides,
        ),
    })
}

#[derive(Clone)]
struct WorldBootstrap {
    manifest: WorldManifest,
    capabilities: WorldCapabilities,
    spawn: SpawnPoint,
    protected_spawn: ProtectedSpawn,
}

impl WorldBootstrap {
    fn opened(
        &self,
        identity: PlayerIdentity,
        recommended_in_flight_batches: u16,
        player_claim: &crate::presence::WorldPresenceClaim,
        edit_session_id: EditSessionId,
        player: LoadedPlayer,
        environment: WorldEnvironmentSnapshot,
    ) -> WorldOpened {
        WorldOpened {
            manifest: self.manifest.clone(),
            capabilities: self.capabilities,
            environment,
            recommended_in_flight_batches,
            identity,
            connection_id: player_claim.connection_id,
            presence_session_id: player_claim.session_id,
            edit_session_id,
            spawn: self.spawn,
            player_resume: player.resume,
            inventory: player.inventory,
        }
    }

    fn default_player_resume(&self) -> PlayerResume {
        let top = self
            .spawn
            .water_level
            .unwrap_or(self.spawn.height)
            .max(self.spawn.height);
        PlayerResume {
            revision: 1,
            eye_position_metres: [
                (self.spawn.x as f32 + 0.5) * voxels_world::VOXEL_SIZE_METRES,
                (top + 1) as f32 * voxels_world::VOXEL_SIZE_METRES
                    + DEFAULT_PLAYER_EYE_HEIGHT_METRES
                    + 0.02,
                (self.spawn.z as f32 + 0.5) * voxels_world::VOXEL_SIZE_METRES,
            ],
            look_yaw_radians: 0.0,
            look_pitch_radians: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct EnvironmentAuthority {
    config: EnvironmentConfig,
    anchor_server_time_ms: u64,
    anchor_unix_seconds: f64,
}

impl EnvironmentAuthority {
    fn new(config: EnvironmentConfig, anchor_server_time_ms: u64) -> Self {
        let anchor_unix_seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0.0, |duration| duration.as_secs_f64());
        Self::from_anchor(config, anchor_server_time_ms, anchor_unix_seconds)
    }

    fn from_anchor(
        config: EnvironmentConfig,
        anchor_server_time_ms: u64,
        anchor_unix_seconds: f64,
    ) -> Self {
        Self {
            config,
            anchor_server_time_ms,
            anchor_unix_seconds,
        }
    }

    fn snapshot(self, sample_server_time_ms: u64) -> WorldEnvironmentSnapshot {
        let elapsed_seconds =
            sample_server_time_ms.saturating_sub(self.anchor_server_time_ms) as f64 / 1_000.0;
        let unix_seconds = self.anchor_unix_seconds + elapsed_seconds;
        let world_days = if self.config.day_length_seconds > 0.0 {
            self.config.world_day_number_at_unix_epoch as f64
                + f64::from(self.config.day_fraction_at_unix_epoch)
                + unix_seconds / f64::from(self.config.day_length_seconds)
        } else {
            self.config.world_day_number_at_unix_epoch as f64
                + f64::from(self.config.day_fraction_at_unix_epoch)
        };
        let world_day_number = world_days.floor() as i64;
        let day_fraction = world_days.rem_euclid(1.0) as f32;
        let weather_fraction = if self.config.weather_cycle_seconds > 0.0 {
            (f64::from(self.config.weather_fraction_at_unix_epoch)
                + unix_seconds / f64::from(self.config.weather_cycle_seconds))
            .rem_euclid(1.0) as f32
        } else {
            self.config.weather_fraction_at_unix_epoch
        };
        let cloud_offset_metres = std::array::from_fn(|axis| {
            (f64::from(self.config.cloud_offset_metres_at_unix_epoch[axis])
                + f64::from(self.config.cloud_velocity_metres_per_second[axis]) * unix_seconds)
                .rem_euclid(CLOUD_PERIOD_METRES) as f32
        });
        WorldEnvironmentSnapshot {
            sample_server_time_ms,
            world_day_number,
            day_fraction,
            day_length_seconds: self.config.day_length_seconds,
            days_per_year: self.config.days_per_year,
            moon_sidereal_orbit_days: self.config.moon_sidereal_orbit_days,
            moon_orbit_phase_at_world_epoch: self.config.moon_orbit_phase_at_world_epoch,
            planet_circumference_metres: self.config.planet_circumference_metres,
            axial_tilt_radians: self.config.axial_tilt_degrees.to_radians(),
            moon_orbit_inclination_radians: self.config.moon_orbit_inclination_degrees.to_radians(),
            celestial_seed: self.config.celestial_seed,
            celestial_revision: self.config.celestial_revision,
            weather_fraction,
            weather_cycle_seconds: self.config.weather_cycle_seconds,
            cloud_offset_metres,
            cloud_velocity_metres_per_second: self.config.cloud_velocity_metres_per_second,
            cloud_coverage: self.config.cloud_coverage,
            cloud_base_metres: self.config.cloud_base_metres,
            cloud_top_metres: self.config.cloud_top_metres,
            weather_seed: self.config.weather_seed,
            weather_revision: self.config.weather_revision,
        }
    }
}

fn validate_spawn_chunk(
    config: &WorldServiceConfig,
    source: &dyn WorldSourceEngine,
    surface_height: i32,
    water_level: Option<i32>,
) -> Result<(), WorldServerError> {
    let top = water_level.unwrap_or(surface_height).max(surface_height);
    let edge = CHUNK_EDGE as i32;
    let coord = ChunkCoord::new(
        config.spawn.xz_voxels[0].div_euclid(edge),
        top.div_euclid(edge),
        config.spawn.xz_voxels[1].div_euclid(edge),
    );
    let request = WorldProductRequest::ChunkWithHalo(coord);
    let result = source
        .generate_batch(WorldProductBatch {
            priority: WorldProductPriority::CollisionCritical,
            requests: vec![request],
        })
        .map_err(WorldServerError::Spawn)?;
    if result.source_identity_hash != source.identity().identity_hash() || result.items.len() != 1 {
        return Err(WorldServerError::InvalidSpawnProduct);
    }
    let item = result
        .items
        .into_iter()
        .next()
        .ok_or(WorldServerError::InvalidSpawnProduct)?;
    let snapshot = match (item.request, item.result) {
        (WorldProductRequest::ChunkWithHalo(returned), Ok(WorldProduct::Chunk(snapshot)))
            if returned == coord =>
        {
            snapshot
        }
        (_, Err(error)) => return Err(WorldServerError::Spawn(error)),
        _ => return Err(WorldServerError::InvalidSpawnProduct),
    };
    if snapshot.source_identity_hash != source.identity().identity_hash()
        || snapshot.chunk.coord() != coord
        || snapshot.meshing_halo.coord() != coord
    {
        return Err(WorldServerError::InvalidSpawnProduct);
    }
    Ok(())
}

struct ServerState {
    allowed_origins: Vec<String>,
    auth_subprotocol_token: String,
    max_frame_bytes: usize,
    max_queued_outbound_bytes_per_client: usize,
    max_in_flight_batches: u16,
    generation_workers_per_client: u16,
    collision_generation_workers_per_client: u16,
    connections: Arc<Semaphore>,
    presence_connections: Arc<Semaphore>,
    traffic: Arc<ClientTrafficRegistry>,
    world: WorldBootstrap,
    environment: EnvironmentAuthority,
    presence: Arc<PresenceHub>,
    source: Arc<dyn WorldSourceEngine>,
    edits: Arc<EditAuthority>,
    generation_tx: mpsc::Sender<GenerationJob>,
}

async fn world_websocket_endpoint(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    websocket: WebSocketUpgrade,
) -> Response {
    let allowed_origin = headers
        .get(ORIGIN)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|origin| {
            state
                .allowed_origins
                .iter()
                .any(|allowed| allowed == origin)
        });
    if !allowed_origin {
        return (StatusCode::FORBIDDEN, "origin is not allowed").into_response();
    }
    if !header_offers_protocol(&headers, WORLD_WEBSOCKET_PROTOCOL)
        || !header_offers_protocol(&headers, &state.auth_subprotocol_token)
    {
        return (
            StatusCode::UNAUTHORIZED,
            "world-service authorization required",
        )
            .into_response();
    }
    let connection_permit = match Arc::clone(&state.connections).try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "world-service is at capacity",
            )
                .into_response();
        }
    };
    let max_frame_bytes = state.max_frame_bytes;
    websocket
        .max_frame_size(max_frame_bytes)
        .max_message_size(max_frame_bytes)
        .protocols([WORLD_WEBSOCKET_PROTOCOL])
        .on_upgrade(move |socket| async move {
            let _connection_permit = connection_permit;
            run_session(socket, state).await;
        })
}

async fn presence_websocket_endpoint(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    websocket: WebSocketUpgrade,
) -> Response {
    if !request_is_authorized(&state, &headers) {
        return (
            StatusCode::UNAUTHORIZED,
            "world-service authorization required",
        )
            .into_response();
    }
    let connection_permit = match Arc::clone(&state.presence_connections).try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "world-service presence is at capacity",
            )
                .into_response();
        }
    };
    let max_frame_bytes = state.max_frame_bytes;
    websocket
        .max_frame_size(max_frame_bytes)
        .max_message_size(max_frame_bytes)
        .protocols([WORLD_WEBSOCKET_PROTOCOL])
        .on_upgrade(move |socket| async move {
            let _connection_permit = connection_permit;
            run_presence_session(socket, state).await;
        })
}

fn request_is_authorized(state: &ServerState, headers: &HeaderMap) -> bool {
    headers
        .get(ORIGIN)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|origin| {
            state
                .allowed_origins
                .iter()
                .any(|allowed| allowed == origin)
        })
        && header_offers_protocol(headers, WORLD_WEBSOCKET_PROTOCOL)
        && header_offers_protocol(headers, &state.auth_subprotocol_token)
}

fn header_offers_protocol(headers: &HeaderMap, expected: &str) -> bool {
    headers
        .get_all(SEC_WEBSOCKET_PROTOCOL)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|protocol| protocol.trim() == expected)
}

struct SessionRequests {
    max_in_flight: usize,
    closed: AtomicBool,
    in_flight: Mutex<HashMap<u64, Arc<AtomicBool>>>,
    generation_permits: Arc<Semaphore>,
    collision_generation_permits: Arc<Semaphore>,
    outbound_bytes: Arc<Semaphore>,
}

impl SessionRequests {
    fn new(
        max_in_flight: u16,
        generation_workers: u16,
        collision_generation_workers: u16,
        max_outbound_bytes: usize,
    ) -> Self {
        Self {
            max_in_flight: usize::from(max_in_flight),
            closed: AtomicBool::new(false),
            in_flight: Mutex::new(HashMap::new()),
            generation_permits: Arc::new(Semaphore::new(usize::from(generation_workers))),
            collision_generation_permits: Arc::new(Semaphore::new(usize::from(
                collision_generation_workers,
            ))),
            outbound_bytes: Arc::new(Semaphore::new(max_outbound_bytes)),
        }
    }

    async fn acquire_generation(
        self: &Arc<Self>,
        priority: WorldProductPriority,
    ) -> Result<OwnedSemaphorePermit, tokio::sync::AcquireError> {
        let permits = if priority == WorldProductPriority::CollisionCritical {
            &self.collision_generation_permits
        } else {
            &self.generation_permits
        };
        Arc::clone(permits).acquire_owned().await
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<u64, Arc<AtomicBool>>> {
        match self.in_flight.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn insert(&self, request_id: u64) -> Result<Arc<AtomicBool>, RequestAdmissionError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(RequestAdmissionError::Closed);
        }
        let mut in_flight = self.lock();
        if in_flight.contains_key(&request_id) {
            return Err(RequestAdmissionError::Duplicate);
        }
        if in_flight.len() >= self.max_in_flight {
            return Err(RequestAdmissionError::WindowFull);
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        in_flight.insert(request_id, Arc::clone(&cancelled));
        Ok(cancelled)
    }

    fn cancel(&self, request_id: u64) -> bool {
        let cancelled = self.lock().get(&request_id).cloned();
        cancelled.is_some_and(|cancelled| {
            cancelled.store(true, Ordering::Release);
            true
        })
    }

    fn finish(&self, request_id: u64, cancelled: &Arc<AtomicBool>) {
        let mut in_flight = self.lock();
        let is_current = in_flight
            .get(&request_id)
            .is_some_and(|current| Arc::ptr_eq(current, cancelled));
        if is_current {
            in_flight.remove(&request_id);
        }
    }

    fn cancel_all(&self) {
        self.closed.store(true, Ordering::Release);
        for cancelled in self.lock().values() {
            cancelled.store(true, Ordering::Release);
        }
    }
}

#[derive(Clone, Copy)]
enum RequestAdmissionError {
    Closed,
    Duplicate,
    WindowFull,
}

struct TrackedRequest {
    request_id: u64,
    cancelled: Arc<AtomicBool>,
    session: Arc<SessionRequests>,
}

impl TrackedRequest {
    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    fn finish(&self) {
        self.session.finish(self.request_id, &self.cancelled);
    }
}

struct OutboundFrame {
    bytes: Vec<u8>,
    offset: usize,
    priority: TrafficPriority,
    tracked: Option<TrackedRequest>,
    _byte_permit: Option<OwnedSemaphorePermit>,
}

impl OutboundFrame {
    fn fragment_bytes(&self, traffic: &ClientTrafficShaper) -> Option<usize> {
        let fragmentable = self.offset > 0
            || message_request_id(&self.bytes).is_ok_and(|request_id| request_id != 0);
        fragmentable
            .then(|| traffic.frame_fragment_bytes(self.bytes.len()))
            .flatten()
    }

    fn next_wire_bytes(&self, fragment_bytes: Option<usize>) -> usize {
        fragment_bytes.map_or(self.bytes.len(), |fragment_bytes| {
            self.bytes
                .len()
                .saturating_sub(self.offset)
                .min(fragment_bytes)
                + FRAME_FRAGMENT_OVERHEAD_BYTES
        })
    }

    fn take_next_wire(
        &mut self,
        fragment_bytes: Option<usize>,
    ) -> Result<(Vec<u8>, bool), voxels_world::protocol::ProtocolError> {
        let Some(fragment_bytes) = fragment_bytes else {
            return Ok((std::mem::take(&mut self.bytes), true));
        };
        let transfer_id = message_request_id(&self.bytes)?;
        let end = self
            .offset
            .saturating_add(fragment_bytes)
            .min(self.bytes.len());
        let wire = encode_frame_fragment(
            transfer_id,
            self.bytes.len(),
            self.offset,
            &self.bytes[self.offset..end],
        )?;
        self.offset = end;
        Ok((wire, self.offset == self.bytes.len()))
    }
}

struct PresenceOutboundFrame {
    bytes: Vec<u8>,
    priority: TrafficPriority,
    _delta_permit: Option<OwnedSemaphorePermit>,
}

struct GenerationJob {
    request: GenerationRequest,
    outbound: mpsc::Sender<OutboundFrame>,
    tracked: TrackedRequest,
}

#[derive(Clone)]
enum GenerationRequest {
    Chunks {
        request: ChunkBatchRequest,
        snapshot: ChunkEditSnapshot,
    },
    SurfaceTiles {
        request: SurfaceTileBatchRequest,
        snapshot: SurfaceEditSnapshot,
    },
}

impl GenerationRequest {
    fn chunks(request: ChunkBatchRequest, edits: &EditAuthority) -> Self {
        let snapshot = edits.snapshot_chunks(&request.coords);
        Self::Chunks { request, snapshot }
    }

    fn surface_tiles(request: SurfaceTileBatchRequest, edits: &EditAuthority) -> Self {
        let snapshot = edits.snapshot_surface(&request.coords);
        Self::SurfaceTiles { request, snapshot }
    }

    fn request_id(&self) -> u64 {
        match self {
            Self::Chunks { request, .. } => request.request_id,
            Self::SurfaceTiles { request, .. } => request.request_id,
        }
    }

    fn priority(&self) -> WorldProductPriority {
        match self {
            Self::Chunks { request, .. } => request.priority,
            Self::SurfaceTiles { request, .. } => request.priority,
        }
    }

    fn product_keys(&self) -> Vec<ProductKey> {
        match self {
            Self::Chunks { request, snapshot } => request
                .coords
                .iter()
                .copied()
                .zip(snapshot.revisions.iter().copied())
                .map(|(coord, edit_revision)| ProductKey::Chunk {
                    coord,
                    edit_revision,
                })
                .collect(),
            Self::SurfaceTiles { request, snapshot } => request
                .coords
                .iter()
                .copied()
                .zip(snapshot.revisions.iter().copied())
                .map(|(coord, edit_revision)| ProductKey::SurfaceTile {
                    coord,
                    edit_revision,
                })
                .collect(),
        }
    }

    fn select(&self, indices: &[usize]) -> Self {
        match self {
            Self::Chunks { request, snapshot } => Self::Chunks {
                request: ChunkBatchRequest {
                    request_id: request.request_id,
                    priority: request.priority,
                    coords: indices.iter().map(|&index| request.coords[index]).collect(),
                },
                snapshot: ChunkEditSnapshot {
                    edits: snapshot.edits.clone(),
                    revisions: indices
                        .iter()
                        .map(|&index| snapshot.revisions[index])
                        .collect(),
                },
            },
            Self::SurfaceTiles { request, snapshot } => Self::SurfaceTiles {
                request: SurfaceTileBatchRequest {
                    request_id: request.request_id,
                    priority: request.priority,
                    coords: indices.iter().map(|&index| request.coords[index]).collect(),
                },
                snapshot: SurfaceEditSnapshot {
                    edits: snapshot.edits.clone(),
                    revisions: indices
                        .iter()
                        .map(|&index| snapshot.revisions[index])
                        .collect(),
                },
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
enum ProductKey {
    Chunk {
        coord: ChunkCoord,
        edit_revision: u64,
    },
    SurfaceTile {
        coord: voxels_world::SurfaceTileCoord,
        edit_revision: u64,
    },
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ProductFlightKey {
    product: ProductKey,
    priority: WorldProductPriority,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum EncodedProduct {
    Chunk(EncodedChunkBatchItem),
    SurfaceTile(EncodedSurfaceTileBatchItem),
}

impl EncodedProduct {
    fn key(&self) -> ProductKey {
        match self {
            Self::Chunk(item) => ProductKey::Chunk {
                coord: item.coord(),
                edit_revision: item.edit_revision(),
            },
            Self::SurfaceTile(item) => ProductKey::SurfaceTile {
                coord: item.coord(),
                edit_revision: item.edit_revision(),
            },
        }
    }

    fn encoded_len(&self) -> usize {
        match self {
            Self::Chunk(item) => item.encoded_len(),
            Self::SurfaceTile(item) => item.encoded_len(),
        }
    }
}

struct ProductGenerationCompletion {
    keys: Vec<ProductFlightKey>,
    result: Result<Vec<EncodedProduct>, String>,
}

struct CachedProduct {
    product: Arc<EncodedProduct>,
    last_access: u64,
}

struct ProductCache {
    max_bytes: usize,
    retained_bytes: usize,
    entries: HashMap<ProductKey, CachedProduct>,
    lru: BTreeSet<(u64, ProductKey)>,
    next_access: u64,
}

impl ProductCache {
    fn new(max_bytes: usize) -> Self {
        Self {
            max_bytes,
            retained_bytes: 0,
            entries: HashMap::new(),
            lru: BTreeSet::new(),
            next_access: 1,
        }
    }

    fn get(&mut self, key: &ProductKey) -> Option<Arc<EncodedProduct>> {
        let access = self.record_access();
        let entry = self.entries.get_mut(key)?;
        self.lru.remove(&(entry.last_access, *key));
        entry.last_access = access;
        let value = Arc::clone(&entry.product);
        self.lru.insert((access, *key));
        Some(value)
    }

    fn insert(&mut self, key: ProductKey, product: Arc<EncodedProduct>) {
        let encoded_len = product.encoded_len();
        if self.max_bytes == 0 || encoded_len > self.max_bytes {
            return;
        }
        if let Some(replaced) = self.entries.remove(&key) {
            self.retained_bytes = self
                .retained_bytes
                .saturating_sub(replaced.product.encoded_len());
            self.lru.remove(&(replaced.last_access, key));
        }
        while self.retained_bytes.saturating_add(encoded_len) > self.max_bytes {
            let Some((_, oldest)) = self.lru.pop_first() else {
                break;
            };
            if let Some(evicted) = self.entries.remove(&oldest) {
                self.retained_bytes = self
                    .retained_bytes
                    .saturating_sub(evicted.product.encoded_len());
            }
        }
        let access = self.record_access();
        self.retained_bytes = self.retained_bytes.saturating_add(encoded_len);
        self.lru.insert((access, key));
        self.entries.insert(
            key,
            CachedProduct {
                product,
                last_access: access,
            },
        );
    }

    fn record_access(&mut self) -> u64 {
        let access = self.next_access;
        self.next_access = self.next_access.saturating_add(1);
        access
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct BatchResponseKey(Box<[ProductKey]>);

impl BatchResponseKey {
    fn from_products(products: &[Arc<EncodedProduct>]) -> Self {
        Self(
            products
                .iter()
                .map(|product| product.key())
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        )
    }
}

struct CachedBatchResponse {
    bytes: Arc<[u8]>,
    last_access: u64,
}

/// Byte-bounded compressed-response cache with per-key assembly serialization.
///
/// A response key retains product ordering and edit revisions but excludes the connection-scoped
/// request id. Cache hits therefore clone the already-compressed frame and rewrite only its header.
struct BatchResponseCache {
    max_bytes: usize,
    retained_bytes: usize,
    entries: HashMap<BatchResponseKey, CachedBatchResponse>,
    lru: BTreeSet<(u64, BatchResponseKey)>,
    flights: HashMap<BatchResponseKey, Weak<AsyncMutex<()>>>,
    next_access: u64,
}

impl BatchResponseCache {
    fn new(max_bytes: usize) -> Self {
        Self {
            max_bytes,
            retained_bytes: 0,
            entries: HashMap::new(),
            lru: BTreeSet::new(),
            flights: HashMap::new(),
            next_access: 1,
        }
    }

    fn get(&mut self, key: &BatchResponseKey) -> Option<Arc<[u8]>> {
        let access = self.record_access();
        let entry = self.entries.get_mut(key)?;
        self.lru.remove(&(entry.last_access, key.clone()));
        entry.last_access = access;
        let bytes = Arc::clone(&entry.bytes);
        self.lru.insert((access, key.clone()));
        Some(bytes)
    }

    fn insert(&mut self, key: BatchResponseKey, bytes: Arc<[u8]>) {
        let encoded_len = bytes.len();
        if self.max_bytes == 0 || encoded_len > self.max_bytes {
            return;
        }
        if let Some(replaced) = self.entries.remove(&key) {
            self.retained_bytes = self.retained_bytes.saturating_sub(replaced.bytes.len());
            self.lru.remove(&(replaced.last_access, key.clone()));
        }
        while self.retained_bytes.saturating_add(encoded_len) > self.max_bytes {
            let Some((_, oldest)) = self.lru.pop_first() else {
                break;
            };
            if let Some(evicted) = self.entries.remove(&oldest) {
                self.retained_bytes = self.retained_bytes.saturating_sub(evicted.bytes.len());
            }
        }
        let access = self.record_access();
        self.retained_bytes = self.retained_bytes.saturating_add(encoded_len);
        self.lru.insert((access, key.clone()));
        self.entries.insert(
            key,
            CachedBatchResponse {
                bytes,
                last_access: access,
            },
        );
    }

    fn flight_lock(&mut self, key: &BatchResponseKey) -> Arc<AsyncMutex<()>> {
        if let Some(lock) = self.flights.get(key).and_then(Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(AsyncMutex::new(()));
        self.flights.insert(key.clone(), Arc::downgrade(&lock));
        lock
    }

    fn finish_flight(&mut self, key: &BatchResponseKey, lock: &Arc<AsyncMutex<()>>) {
        let matches = self
            .flights
            .get(key)
            .and_then(Weak::upgrade)
            .is_some_and(|current| Arc::ptr_eq(&current, lock));
        if matches {
            self.flights.remove(key);
        }
    }

    fn record_access(&mut self) -> u64 {
        let access = self.next_access;
        self.next_access = self.next_access.saturating_add(1);
        access
    }
}

fn lock_batch_response_cache(
    cache: &Mutex<BatchResponseCache>,
) -> MutexGuard<'_, BatchResponseCache> {
    cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn get_cached_batch_response(
    cache: &Mutex<BatchResponseCache>,
    key: &BatchResponseKey,
    request_id: u64,
) -> Result<Option<Vec<u8>>, String> {
    let bytes = { lock_batch_response_cache(cache).get(key) };
    bytes
        .map(|bytes| {
            clone_message_with_request_id(&bytes, request_id).map_err(|error| error.to_string())
        })
        .transpose()
}

#[derive(Clone, Copy)]
struct ProductWaiter {
    batch_id: u64,
    item_index: usize,
}

struct PendingGenerationBatch {
    job: GenerationJob,
    items: Vec<Option<Result<Arc<EncodedProduct>, String>>>,
    remaining: usize,
}

impl PendingGenerationBatch {
    fn new(job: GenerationJob, item_count: usize) -> Self {
        Self {
            job,
            items: vec![None; item_count],
            remaining: item_count,
        }
    }

    fn fill(&mut self, item_index: usize, result: Result<Arc<EncodedProduct>, String>) -> bool {
        let Some(slot) = self.items.get_mut(item_index) else {
            return false;
        };
        if slot.is_none() {
            *slot = Some(result);
            self.remaining -= 1;
        }
        self.remaining == 0
    }
}

async fn run_session(mut socket: WebSocket, state: Arc<ServerState>) {
    let first = match next_socket_binary(&mut socket).await {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return,
        Err(message) => {
            let _ = socket
                .send(Message::Binary(encode_error(0, message).into()))
                .await;
            return;
        }
    };
    let open = match decode_open_world(&first) {
        Ok(open) => open,
        Err(error) => {
            let _ = socket
                .send(Message::Binary(encode_error(0, &error.to_string()).into()))
                .await;
            return;
        }
    };
    let loaded_player = match state
        .edits
        .load_player(open.identity.player_id, state.world.default_player_resume())
    {
        Ok(player) => player,
        Err(error) => {
            let _ = socket
                .send(Message::Binary(
                    encode_error(0, &format!("open player state: {error}")).into(),
                ))
                .await;
            return;
        }
    };
    let Some(player_claim) = state.presence.join(&open.identity, loaded_player.resume) else {
        let _ = socket
            .send(Message::Binary(
                encode_error(0, "player is already connected").into(),
            ))
            .await;
        return;
    };
    let edit_session_id = match state.edits.begin_player_session(open.identity.player_id) {
        Ok(edit_session_id) => edit_session_id,
        Err(error) => {
            let _ = socket
                .send(Message::Binary(
                    encode_error(0, &format!("begin player edit session: {error}")).into(),
                ))
                .await;
            return;
        }
    };
    let negotiated_window = open.max_in_flight_batches.min(state.max_in_flight_batches);
    let traffic_registration = state.traffic.register(player_claim.connection_id);
    let traffic = traffic_registration.shaper();
    let session = Arc::new(SessionRequests::new(
        negotiated_window,
        state.generation_workers_per_client,
        state.collision_generation_workers_per_client,
        state.max_queued_outbound_bytes_per_client,
    ));
    let (sink, stream) = socket.split();
    let outbound_capacity = usize::from(state.max_in_flight_batches).saturating_add(2);
    let (outbound, outbound_rx) = mpsc::channel(outbound_capacity);
    let (inbound_tx, mut inbound) = mpsc::channel(outbound_capacity);
    let writer_session = Arc::clone(&session);
    let writer = tokio::spawn(write_frames(sink, outbound_rx, writer_session, traffic));
    let reader = tokio::spawn(read_frames(stream, inbound_tx, state.max_frame_bytes));

    let environment_server_time_ms = state.presence.now_ms();
    let opened = state.world.opened(
        open.identity,
        negotiated_window,
        &player_claim,
        edit_session_id,
        loaded_player,
        state.environment.snapshot(environment_server_time_ms),
    );
    if outbound
        .send(OutboundFrame {
            bytes: encode_world_opened(&opened),
            offset: 0,
            priority: TrafficPriority::Critical,
            tracked: None,
            _byte_permit: None,
        })
        .await
        .is_err()
    {
        session.cancel_all();
        return;
    }

    let mut edit_subscription = state.edits.subscribe(player_claim.connection_id);
    let mut edit_subscription_open = true;

    loop {
        let inbound_frame = tokio::select! {
            edit = edit_subscription.receiver.recv(), if edit_subscription_open => {
                let Some(commit) = edit else {
                    edit_subscription_open = false;
                    continue;
                };
                if edit_subscription.discard_stale_after_overflow() {
                    let resync = ResyncRequired {
                        revision: state.edits.revision(),
                    };
                    match encode_resync_required(resync) {
                        Ok(bytes) => {
                            if send_frame(&outbound, bytes, TrafficPriority::Critical)
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                    continue;
                }
                match encode_edit_commit(&commit) {
                    Ok(bytes) => {
                        if send_frame(&outbound, bytes, TrafficPriority::WorldChange)
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(_) => break,
                }
                continue;
            }
            frame = inbound.recv() => frame,
        };
        let bytes = match inbound_frame {
            Some(InboundFrame::Binary(bytes)) => bytes,
            Some(InboundFrame::Rejected(message)) => {
                let _ = send_control_error(&outbound, 0, message).await;
                break;
            }
            None => break,
        };
        let kind = match message_kind(&bytes) {
            Ok(kind) => kind,
            Err(error) => {
                let _ = send_control_error(&outbound, 0, &error.to_string()).await;
                break;
            }
        };
        if kind == chunk_batch_kind() {
            let request = match decode_chunk_batch(&bytes) {
                Ok(request) => request,
                Err(error) => {
                    let _ = send_control_error(&outbound, 0, &error.to_string()).await;
                    break;
                }
            };
            let cancelled = match session.insert(request.request_id) {
                Ok(cancelled) => cancelled,
                Err(error) => {
                    let message = match error {
                        RequestAdmissionError::Closed => "world session is closing",
                        RequestAdmissionError::Duplicate => "request id is already in flight",
                        RequestAdmissionError::WindowFull => "negotiated request window is full",
                    };
                    let _ = send_control_error(&outbound, request.request_id, message).await;
                    continue;
                }
            };
            let request_id = request.request_id;
            let tracked = TrackedRequest {
                request_id,
                cancelled,
                session: Arc::clone(&session),
            };
            let job = GenerationJob {
                request: GenerationRequest::chunks(request, state.edits.as_ref()),
                outbound: outbound.clone(),
                tracked,
            };
            match state.generation_tx.try_send(job) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(job)) => {
                    job.tracked.finish();
                    let _ =
                        send_control_error(&outbound, request_id, "world generator is busy").await;
                }
                Err(mpsc::error::TrySendError::Closed(job)) => {
                    job.tracked.finish();
                    let _ =
                        send_control_error(&outbound, request_id, "world generator stopped").await;
                    break;
                }
            }
        } else if kind == surface_tile_batch_kind() {
            let request = match decode_surface_tile_batch(&bytes) {
                Ok(request) => request,
                Err(error) => {
                    let _ = send_control_error(&outbound, 0, &error.to_string()).await;
                    break;
                }
            };
            let cancelled = match session.insert(request.request_id) {
                Ok(cancelled) => cancelled,
                Err(error) => {
                    let message = match error {
                        RequestAdmissionError::Closed => "world session is closing",
                        RequestAdmissionError::Duplicate => "request id is already in flight",
                        RequestAdmissionError::WindowFull => "negotiated request window is full",
                    };
                    let _ = send_control_error(&outbound, request.request_id, message).await;
                    continue;
                }
            };
            let request_id = request.request_id;
            let tracked = TrackedRequest {
                request_id,
                cancelled,
                session: Arc::clone(&session),
            };
            let job = GenerationJob {
                request: GenerationRequest::surface_tiles(request, state.edits.as_ref()),
                outbound: outbound.clone(),
                tracked,
            };
            match state.generation_tx.try_send(job) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(job)) => {
                    job.tracked.finish();
                    let _ =
                        send_control_error(&outbound, request_id, "world generator is busy").await;
                }
                Err(mpsc::error::TrySendError::Closed(job)) => {
                    job.tracked.finish();
                    let _ =
                        send_control_error(&outbound, request_id, "world generator stopped").await;
                    break;
                }
            }
        } else if kind == edit_command_kind() {
            let command = match decode_edit_command(&bytes) {
                Ok(command) => command,
                Err(error) => {
                    let _ = send_control_error(&outbound, 0, &error.to_string()).await;
                    break;
                }
            };
            if let Err(message) = state
                .presence
                .authorize_interaction(player_claim.connection_id, command.action.target())
            {
                let _ = send_control_error(&outbound, command.operation_id, message).await;
                continue;
            }
            let authority = Arc::clone(&state.edits);
            let source = Arc::clone(&state.source);
            let player_id = player_claim.player_id();
            let editor_connection_id = player_claim.connection_id;
            let applied = tokio::task::spawn_blocking(move || {
                authority.apply(source.as_ref(), player_id, editor_connection_id, command)
            })
            .await;
            let applied = match applied {
                Ok(Ok(applied)) => applied,
                Ok(Err(error)) => {
                    let _ = send_control_error(&outbound, command.operation_id, &error.to_string())
                        .await;
                    continue;
                }
                Err(_) => {
                    let _ = send_control_error(
                        &outbound,
                        command.operation_id,
                        "edit authority task failed",
                    )
                    .await;
                    break;
                }
            };
            let mut recipients = BTreeSet::new();
            if applied.changed {
                recipients = state.presence.connections_near_voxels(
                    applied.commit.mutations.iter().map(|item| item.coord),
                );
            }
            recipients.remove(&player_claim.connection_id);
            state.edits.publish(&applied.commit, &recipients);
            let bytes = match encode_edit_commit(&applied.commit) {
                Ok(bytes) => bytes,
                Err(_) => break,
            };
            if send_frame(&outbound, bytes, TrafficPriority::Critical)
                .await
                .is_err()
            {
                break;
            }
        } else if kind == cancel_kind() {
            match decode_cancel(&bytes) {
                Ok(request_id) => {
                    session.cancel(request_id);
                }
                Err(error) => {
                    let _ = send_control_error(&outbound, 0, &error.to_string()).await;
                    break;
                }
            }
        } else if kind == open_world_kind() {
            let _ = send_control_error(&outbound, 0, "world session is already open").await;
            break;
        } else {
            let _ = send_control_error(&outbound, 0, "unexpected client message kind").await;
            break;
        }
    }
    if let Some(resume) = player_claim.latest_resume() {
        let _ = state
            .edits
            .save_player_resume(player_claim.player_id(), resume);
    }
    state.edits.unsubscribe(player_claim.connection_id);
    session.cancel_all();
    reader.abort();
    let _ = reader.await;
    drop(outbound);
    let _ = writer.await;
}

async fn run_presence_session(mut socket: WebSocket, state: Arc<ServerState>) {
    let first = match next_socket_binary(&mut socket).await {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return,
        Err(message) => {
            let _ = socket
                .send(Message::Binary(encode_error(0, message).into()))
                .await;
            return;
        }
    };
    let open = match decode_open_presence(&first) {
        Ok(open) => open,
        Err(error) => {
            let _ = socket
                .send(Message::Binary(encode_error(0, &error.to_string()).into()))
                .await;
            return;
        }
    };
    let Some(attachment) = state.presence.attach(open.session_id) else {
        let _ = socket
            .send(Message::Binary(
                encode_error(0, "presence session is invalid or already attached").into(),
            ))
            .await;
        return;
    };
    let Some(traffic) = state.traffic.get(attachment.connection_id) else {
        let _ = socket
            .send(Message::Binary(
                encode_error(0, "world traffic session is no longer active").into(),
            ))
            .await;
        return;
    };
    let config = state.presence.config();
    let opened = PresenceOpened {
        connection_id: attachment.connection_id,
        server_time_ms: state.presence.now_ms(),
        broadcast_interval_ms: config.broadcast_interval_ms,
        max_players: config.max_players,
    };
    let opened = match encode_presence_opened(opened) {
        Ok(opened) => opened,
        Err(_) => return,
    };
    if send_presence_frame(&mut socket, &traffic, TrafficPriority::Critical, opened)
        .await
        .is_err()
    {
        return;
    }
    let mut stream = PresenceStreamState::default();
    let initial_delta = match state.presence.build_delta(&attachment, &mut stream) {
        Ok(Some(delta)) => delta,
        Ok(None) | Err(_) => return,
    };
    if send_presence_frame(
        &mut socket,
        &traffic,
        TrafficPriority::RealtimePresence,
        initial_delta,
    )
    .await
    .is_err()
    {
        return;
    }
    let (sink, source) = socket.split();
    let (outbound, outbound_rx) = mpsc::channel(8);
    let (inbound_tx, mut inbound) = mpsc::channel(128);
    let (pose_tx, mut latest_pose) = watch::channel(None);
    let (shutdown, shutdown_rx) = watch::channel(false);
    let reader = tokio::spawn(read_presence_frames(
        source,
        inbound_tx,
        pose_tx,
        state.max_frame_bytes,
    ));
    let writer = tokio::spawn(write_presence_frames(
        sink,
        outbound_rx,
        Arc::clone(&traffic),
        shutdown_rx,
    ));
    let delta_slot = Arc::new(Semaphore::new(1));
    let mut replication_tick = tokio::time::interval(tokio::time::Duration::from_millis(
        u64::from(config.broadcast_interval_ms),
    ));
    replication_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    replication_tick.tick().await;

    let mut peer_closed = false;
    let mut pose_open = true;
    loop {
        tokio::select! {
            message = inbound.recv() => {
                let bytes = match message {
                    Some(InboundFrame::Binary(bytes)) => bytes,
                    Some(InboundFrame::Rejected(message)) => {
                        let _ = queue_presence_frame(
                            &outbound,
                            TrafficPriority::Critical,
                            encode_error(0, message),
                        ).await;
                        break;
                    }
                    None => {
                        peer_closed = true;
                        break;
                    }
                };
                let kind = match message_kind(&bytes) {
                    Ok(kind) => kind,
                    Err(error) => {
                        let _ = queue_presence_frame(
                            &outbound,
                            TrafficPriority::Critical,
                            encode_error(0, &error.to_string()),
                        ).await;
                        break;
                    }
                };
                if kind == player_pose_kind() {
                    let pose = match decode_player_pose(&bytes) {
                        Ok(pose) => pose,
                        Err(error) => {
                            let _ = queue_presence_frame(
                                &outbound,
                                TrafficPriority::Critical,
                                encode_error(0, &error.to_string()),
                            ).await;
                            break;
                        }
                    };
                    if let Some(message) = pose_admission_error(
                        state.presence.accept_pose(&attachment, pose),
                    ) {
                        let _ = queue_presence_frame(
                            &outbound,
                            TrafficPriority::Critical,
                            encode_error(0, message),
                        ).await;
                        break;
                    }
                } else if kind == presence_ping_kind() {
                    let ping = match decode_presence_ping(&bytes) {
                        Ok(ping) => ping,
                        Err(error) => {
                            let _ = queue_presence_frame(
                                &outbound,
                                TrafficPriority::Critical,
                                encode_error(0, &error.to_string()),
                            ).await;
                            break;
                        }
                    };
                    if ping.observed_round_trip_ms > 0 {
                        traffic.observe_round_trip(Duration::from_millis(u64::from(
                            ping.observed_round_trip_ms,
                        )));
                    }
                    let receive_time = state.presence.now_ms();
                    let pong = PresencePong {
                        sequence: ping.sequence,
                        outbound_rate_bytes_per_second: u32::try_from(
                            traffic.current_rate_bytes_per_second(),
                        )
                        .unwrap_or(u32::MAX),
                        client_send_time_ms: ping.client_send_time_ms,
                        server_receive_time_ms: receive_time,
                        server_send_time_ms: state.presence.now_ms().max(receive_time),
                    };
                    let Ok(pong) = encode_presence_pong(pong) else {
                        break;
                    };
                    if queue_presence_frame(&outbound, TrafficPriority::Critical, pong)
                        .await
                        .is_err() {
                        break;
                    }
                } else if kind == open_presence_kind() {
                    let _ = queue_presence_frame(
                        &outbound,
                        TrafficPriority::Critical,
                        encode_error(0, "presence session is already open"),
                    ).await;
                    break;
                } else {
                    let _ = queue_presence_frame(
                        &outbound,
                        TrafficPriority::Critical,
                        encode_error(0, "unexpected presence message kind"),
                    ).await;
                    break;
                }
            }
            changed = latest_pose.changed(), if pose_open => {
                if changed.is_err() {
                    pose_open = false;
                    continue;
                }
                let pose = *latest_pose.borrow_and_update();
                let Some(pose) = pose else {
                    continue;
                };
                if let Some(message) = pose_admission_error(
                    state.presence.accept_pose(&attachment, pose),
                ) {
                    let _ = queue_presence_frame(
                        &outbound,
                        TrafficPriority::Critical,
                        encode_error(0, message),
                    ).await;
                    break;
                }
            }
            _ = replication_tick.tick() => {
                let Ok(delta_permit) = Arc::clone(&delta_slot).try_acquire_owned() else {
                    continue;
                };
                match state.presence.build_delta(&attachment, &mut stream) {
                    Ok(Some(bytes)) => {
                        if outbound.send(PresenceOutboundFrame {
                            bytes,
                            priority: TrafficPriority::RealtimePresence,
                            _delta_permit: Some(delta_permit),
                        }).await.is_err() {
                            break;
                        }
                    }
                    Ok(None) => drop(delta_permit),
                    Err(error) => {
                        let _ = queue_presence_frame(
                            &outbound,
                            TrafficPriority::Critical,
                            encode_error(0, &error),
                        ).await;
                        break;
                    }
                }
            }
        }
    }
    drop(outbound);
    if peer_closed {
        let _ = shutdown.send(true);
    }
    reader.abort();
    let _ = reader.await;
    let _ = writer.await;
}

fn pose_admission_error(admission: PoseAdmission) -> Option<&'static str> {
    match admission {
        PoseAdmission::Accepted | PoseAdmission::IgnoredStale | PoseAdmission::IgnoredRateLimit => {
            None
        }
        PoseAdmission::SessionClosed => Some("presence session closed"),
        PoseAdmission::Invalid(message) => Some(message),
    }
}

enum InboundFrame {
    Binary(Vec<u8>),
    Rejected(&'static str),
}

async fn read_presence_frames(
    mut stream: SplitStream<WebSocket>,
    inbound: mpsc::Sender<InboundFrame>,
    latest_pose: watch::Sender<Option<voxels_world::protocol::PlayerPoseUpdate>>,
    max_frame_bytes: usize,
) {
    while let Some(message) = stream.next().await {
        let frame = match message {
            Ok(Message::Binary(bytes)) if bytes.len() <= max_frame_bytes => {
                let bytes = bytes.to_vec();
                if message_kind(&bytes).ok() == Some(player_pose_kind())
                    && let Ok(pose) = decode_player_pose(&bytes)
                {
                    latest_pose.send_replace(Some(pose));
                    continue;
                }
                InboundFrame::Binary(bytes)
            }
            Ok(Message::Binary(_)) => InboundFrame::Rejected("VXWP frame exceeds configured limit"),
            Ok(Message::Ping(_) | Message::Pong(_)) => continue,
            Ok(Message::Close(_)) | Err(_) => break,
            Ok(Message::Text(_)) => InboundFrame::Rejected("VXWP accepts binary messages only"),
        };
        let rejected = matches!(frame, InboundFrame::Rejected(_));
        if inbound.send(frame).await.is_err() || rejected {
            break;
        }
    }
}

async fn read_frames(
    mut stream: SplitStream<WebSocket>,
    inbound: mpsc::Sender<InboundFrame>,
    max_frame_bytes: usize,
) {
    while let Some(message) = stream.next().await {
        let frame = match message {
            Ok(Message::Binary(bytes)) if bytes.len() <= max_frame_bytes => {
                InboundFrame::Binary(bytes.to_vec())
            }
            Ok(Message::Binary(_)) => InboundFrame::Rejected("VXWP frame exceeds configured limit"),
            Ok(Message::Ping(_) | Message::Pong(_)) => continue,
            Ok(Message::Close(_)) | Err(_) => break,
            Ok(Message::Text(_)) => InboundFrame::Rejected("VXWP accepts binary messages only"),
        };
        let rejected = matches!(frame, InboundFrame::Rejected(_));
        if inbound.send(frame).await.is_err() || rejected {
            break;
        }
    }
}

async fn next_socket_binary(socket: &mut WebSocket) -> Result<Option<Vec<u8>>, &'static str> {
    loop {
        match socket.recv().await {
            Some(Ok(Message::Binary(bytes))) => return Ok(Some(bytes.to_vec())),
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
            Some(Ok(Message::Close(_))) | None => return Ok(None),
            Some(Ok(Message::Text(_))) => return Err("VXWP accepts binary messages only"),
            Some(Err(_)) => return Ok(None),
        }
    }
}

async fn send_control_error(
    outbound: &mpsc::Sender<OutboundFrame>,
    request_id: u64,
    message: &str,
) -> Result<(), mpsc::error::SendError<OutboundFrame>> {
    outbound
        .send(OutboundFrame {
            bytes: encode_error(request_id, message),
            offset: 0,
            priority: TrafficPriority::Critical,
            tracked: None,
            _byte_permit: None,
        })
        .await
}

async fn send_frame(
    outbound: &mpsc::Sender<OutboundFrame>,
    bytes: Vec<u8>,
    priority: TrafficPriority,
) -> Result<(), mpsc::error::SendError<OutboundFrame>> {
    outbound
        .send(OutboundFrame {
            bytes,
            offset: 0,
            priority,
            tracked: None,
            _byte_permit: None,
        })
        .await
}

async fn write_frames(
    mut sink: SplitSink<WebSocket, Message>,
    mut outbound: mpsc::Receiver<OutboundFrame>,
    session: Arc<SessionRequests>,
    traffic: Arc<ClientTrafficShaper>,
) {
    let mut queues: [VecDeque<OutboundFrame>; TrafficPriority::COUNT] =
        std::array::from_fn(|_| VecDeque::new());
    let mut outbound_open = true;
    'writer: loop {
        while let Ok(frame) = outbound.try_recv() {
            queues[frame.priority.index()].push_back(frame);
        }
        for queue in &mut queues {
            while queue
                .front()
                .and_then(|frame| frame.tracked.as_ref())
                .is_some_and(TrackedRequest::is_cancelled)
            {
                if let Some(tracked) = queue.pop_front().and_then(|frame| frame.tracked) {
                    tracked.finish();
                }
            }
        }
        if queues.iter().all(VecDeque::is_empty) {
            if !outbound_open {
                break;
            }
            match outbound.recv().await {
                Some(frame) => {
                    queues[frame.priority.index()].push_back(frame);
                    continue;
                }
                None => {
                    outbound_open = false;
                    continue;
                }
            }
        }

        let mut permits = FuturesUnordered::new();
        let contended = queues.iter().filter(|queue| !queue.is_empty()).count() > 1;
        for priority in TrafficPriority::ALL {
            if let Some(frame) = queues[priority.index()].front() {
                let fragment_bytes = frame.fragment_bytes(&traffic);
                permits.push(acquire_world_traffic(
                    Arc::clone(&traffic),
                    priority,
                    frame.next_wire_bytes(fragment_bytes),
                    fragment_bytes,
                    contended,
                ));
            }
        }
        loop {
            tokio::select! {
                Some(permit) = permits.next() => {
                    let priority = permit.priority;
                    let Some(mut frame) = queues[priority.index()].pop_front() else {
                        break 'writer;
                    };
                    drop(permits);
                    if frame
                        .tracked
                        .as_ref()
                        .is_some_and(TrackedRequest::is_cancelled)
                    {
                        if let Some(tracked) = frame.tracked {
                            tracked.finish();
                        }
                        continue 'writer;
                    }
                    let (wire, complete) = match frame.take_next_wire(permit.fragment_bytes) {
                        Ok(next) => next,
                        Err(_) => {
                            if let Some(tracked) = frame.tracked {
                                tracked.finish();
                            }
                            break 'writer;
                        }
                    };
                    if sink.send(Message::Binary(wire.into())).await.is_err() {
                        if let Some(tracked) = frame.tracked {
                            tracked.finish();
                        }
                        break 'writer;
                    }
                    if complete {
                        if let Some(tracked) = frame.tracked {
                            tracked.finish();
                        }
                    } else {
                        queues[priority.index()].push_front(frame);
                    }
                    continue 'writer;
                }
                incoming = outbound.recv(), if outbound_open => {
                    match incoming {
                        Some(incoming) => {
                            let priority = incoming.priority;
                            let queue = &mut queues[priority.index()];
                            let was_empty = queue.is_empty();
                            let fragment_bytes = incoming.fragment_bytes(&traffic);
                            let next_wire_bytes = incoming.next_wire_bytes(fragment_bytes);
                            queue.push_back(incoming);
                            if was_empty {
                                permits.push(acquire_world_traffic(
                                    Arc::clone(&traffic),
                                    priority,
                                    next_wire_bytes,
                                    fragment_bytes,
                                    true,
                                ));
                            }
                        }
                        None => {
                            outbound_open = false;
                        }
                    }
                }
            }
        }
    }
    session.cancel_all();
}

struct WorldTrafficPermit {
    priority: TrafficPriority,
    fragment_bytes: Option<usize>,
}

async fn acquire_world_traffic(
    traffic: Arc<ClientTrafficShaper>,
    priority: TrafficPriority,
    bytes: usize,
    fragment_bytes: Option<usize>,
    contended: bool,
) -> WorldTrafficPermit {
    if contended {
        traffic.acquire_contended(priority, bytes).await;
    } else {
        traffic.acquire(priority, bytes).await;
    }
    WorldTrafficPermit {
        priority,
        fragment_bytes,
    }
}

async fn acquire_traffic(
    traffic: Arc<ClientTrafficShaper>,
    priority: TrafficPriority,
    bytes: usize,
    contended: bool,
) -> TrafficPriority {
    if contended {
        traffic.acquire_contended(priority, bytes).await;
    } else {
        traffic.acquire(priority, bytes).await;
    }
    priority
}

async fn send_presence_frame(
    socket: &mut WebSocket,
    traffic: &Arc<ClientTrafficShaper>,
    priority: TrafficPriority,
    bytes: Vec<u8>,
) -> Result<(), axum::Error> {
    traffic.acquire(priority, bytes.len()).await;
    socket.send(Message::Binary(bytes.into())).await
}

async fn queue_presence_frame(
    outbound: &mpsc::Sender<PresenceOutboundFrame>,
    priority: TrafficPriority,
    bytes: Vec<u8>,
) -> Result<(), mpsc::error::SendError<PresenceOutboundFrame>> {
    outbound
        .send(PresenceOutboundFrame {
            bytes,
            priority,
            _delta_permit: None,
        })
        .await
}

async fn write_presence_frames(
    mut sink: SplitSink<WebSocket, Message>,
    mut outbound: mpsc::Receiver<PresenceOutboundFrame>,
    traffic: Arc<ClientTrafficShaper>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut queues: [VecDeque<PresenceOutboundFrame>; TrafficPriority::COUNT] =
        std::array::from_fn(|_| VecDeque::new());
    let mut outbound_open = true;
    'writer: loop {
        if *shutdown.borrow() {
            break;
        }
        while let Ok(frame) = outbound.try_recv() {
            queues[frame.priority.index()].push_back(frame);
        }
        if queues.iter().all(VecDeque::is_empty) {
            if !outbound_open {
                break;
            }
            tokio::select! {
                biased;
                changed = shutdown.changed() => {
                    if changed.is_ok() && *shutdown.borrow() {
                        break;
                    }
                }
                incoming = outbound.recv() => {
                    match incoming {
                        Some(incoming) => {
                            queues[incoming.priority.index()].push_back(incoming);
                        }
                        None => outbound_open = false,
                    }
                }
            }
            continue;
        }

        let mut permits = FuturesUnordered::new();
        let contended = queues.iter().filter(|queue| !queue.is_empty()).count() > 1;
        for priority in TrafficPriority::ALL {
            if let Some(frame) = queues[priority.index()].front() {
                permits.push(acquire_traffic(
                    Arc::clone(&traffic),
                    priority,
                    frame.bytes.len(),
                    contended,
                ));
            }
        }
        loop {
            tokio::select! {
                biased;
                changed = shutdown.changed() => {
                    if changed.is_ok() && *shutdown.borrow() {
                        break 'writer;
                    }
                }
                Some(priority) = permits.next() => {
                    let Some(frame) = queues[priority.index()].pop_front() else {
                        break 'writer;
                    };
                    drop(permits);
                    if sink.send(Message::Binary(frame.bytes.into())).await.is_err() {
                        break 'writer;
                    }
                    continue 'writer;
                }
                incoming = outbound.recv(), if outbound_open => {
                    match incoming {
                        Some(incoming) => {
                            let priority = incoming.priority;
                            let queue = &mut queues[priority.index()];
                            let was_empty = queue.is_empty();
                            queue.push_back(incoming);
                            if was_empty {
                                let bytes = queue
                                    .front()
                                    .map(|frame| frame.bytes.len())
                                    .unwrap_or_default();
                                permits.push(acquire_traffic(
                                    Arc::clone(&traffic),
                                    priority,
                                    bytes,
                                    true,
                                ));
                            }
                        }
                        None => outbound_open = false,
                    }
                }
            }
        }
    }
    let _ = sink.close().await;
}

async fn run_generation_dispatcher(
    mut jobs: mpsc::Receiver<GenerationJob>,
    source: Arc<dyn WorldSourceEngine>,
    generation_limiter: Arc<PriorityGenerationLimiter>,
    prefetch_semaphore: Arc<Semaphore>,
    max_frame_bytes: usize,
    product_cache_bytes: usize,
) {
    let (completion_tx, mut completions) = mpsc::unbounded_channel();
    let source_identity_hash = source.identity().identity_hash();
    let response_cache_bytes = product_cache_bytes / RESPONSE_CACHE_BUDGET_DIVISOR;
    let encoded_product_cache_bytes = product_cache_bytes.saturating_sub(response_cache_bytes);
    let mut in_flight = HashMap::<ProductFlightKey, Vec<ProductWaiter>>::new();
    let mut pending = HashMap::<u64, PendingGenerationBatch>::new();
    let mut cache = ProductCache::new(encoded_product_cache_bytes);
    let response_cache = Arc::new(Mutex::new(BatchResponseCache::new(response_cache_bytes)));
    let mut next_batch_id = 1_u64;
    let mut jobs_open = true;
    loop {
        if !jobs_open && in_flight.is_empty() {
            break;
        }
        tokio::select! {
            job = jobs.recv(), if jobs_open => {
                let Some(job) = job else {
                    jobs_open = false;
                    continue;
                };
                let product_keys = job.request.product_keys();
                let priority = job.request.priority();
                let session = Arc::clone(&job.tracked.session);
                let batch_id = next_batch_id;
                next_batch_id = next_batch_id.wrapping_add(1).max(1);
                let mut batch = PendingGenerationBatch::new(job, product_keys.len());
                let mut miss_indices = Vec::new();
                let mut miss_keys = Vec::new();
                for (item_index, product) in product_keys.into_iter().enumerate() {
                    if let Some(cached) = cache.get(&product) {
                        batch.fill(item_index, Ok(cached));
                        continue;
                    }
                    let flight = ProductFlightKey { product, priority };
                    let waiter = ProductWaiter { batch_id, item_index };
                    if let Some(waiters) = in_flight.get_mut(&flight) {
                        waiters.push(waiter);
                    } else {
                        in_flight.insert(flight, vec![waiter]);
                        miss_indices.push(item_index);
                        miss_keys.push(flight);
                    }
                }
                if batch.remaining == 0 {
                    spawn_generation_assembly(
                        batch,
                        source_identity_hash,
                        Arc::clone(&generation_limiter),
                        Arc::clone(&response_cache),
                        max_frame_bytes,
                    );
                    continue;
                }
                let miss_request = batch.job.request.select(&miss_indices);
                pending.insert(batch_id, batch);
                if !miss_indices.is_empty() {
                    let source = Arc::clone(&source);
                    let generation_limiter = Arc::clone(&generation_limiter);
                    let prefetch_semaphore = Arc::clone(&prefetch_semaphore);
                    let completion_tx = completion_tx.clone();
                    tokio::spawn(async move {
                        let result = generate_single_flight_products(
                            miss_request,
                            source,
                            session,
                            generation_limiter,
                            prefetch_semaphore,
                        )
                        .await;
                        let _ = completion_tx.send(ProductGenerationCompletion {
                            keys: miss_keys,
                            result,
                        });
                    });
                }
            }
            completion = completions.recv(), if !in_flight.is_empty() => {
                let Some(completion) = completion else {
                    break;
                };
                let results = match completion.result {
                    Ok(products) if products.len() == completion.keys.len() => products
                        .into_iter()
                        .zip(completion.keys.iter())
                        .map(|(product, expected)| {
                            if product.key() == expected.product {
                                Ok(product)
                            } else {
                                Err("world source returned a mismatched product key".to_owned())
                            }
                        })
                        .collect::<Vec<_>>(),
                    Ok(_) => vec![
                        Err("world source returned a mismatched product count".to_owned());
                        completion.keys.len()
                    ],
                    Err(message) => vec![Err(message); completion.keys.len()],
                };
                let mut ready = BTreeSet::new();
                for (flight, result) in completion.keys.into_iter().zip(results) {
                    let Some(waiters) = in_flight.remove(&flight) else {
                        continue;
                    };
                    let result = result.map(Arc::new);
                    if let Ok(product) = &result {
                        cache.insert(flight.product, Arc::clone(product));
                    }
                    for waiter in waiters {
                        let Some(batch) = pending.get_mut(&waiter.batch_id) else {
                            continue;
                        };
                        if batch.fill(waiter.item_index, result.clone()) {
                            ready.insert(waiter.batch_id);
                        }
                    }
                }
                for batch_id in ready {
                    let Some(batch) = pending.remove(&batch_id) else {
                        continue;
                    };
                    spawn_generation_assembly(
                        batch,
                        source_identity_hash,
                        Arc::clone(&generation_limiter),
                        Arc::clone(&response_cache),
                        max_frame_bytes,
                    );
                }
            }
        }
    }
}

async fn generate_single_flight_products(
    request: GenerationRequest,
    source: Arc<dyn WorldSourceEngine>,
    session: Arc<SessionRequests>,
    generation_limiter: Arc<PriorityGenerationLimiter>,
    prefetch_semaphore: Arc<Semaphore>,
) -> Result<Vec<EncodedProduct>, String> {
    let priority = request.priority();
    let _prefetch_permit = if priority == WorldProductPriority::Prefetch {
        Some(
            prefetch_semaphore
                .acquire_owned()
                .await
                .map_err(|_| "world prefetch generation limiter stopped".to_owned())?,
        )
    } else {
        None
    };
    let _session_permit = session
        .acquire_generation(priority)
        .await
        .map_err(|_| "world session generation limiter stopped".to_owned())?;
    let _global_permit = generation_limiter.acquire(priority).await;
    tokio::task::spawn_blocking(move || match request {
        GenerationRequest::Chunks { request, snapshot } => {
            generate_chunk_products(source.as_ref(), request, snapshot)
        }
        GenerationRequest::SurfaceTiles { request, snapshot } => {
            generate_surface_tile_products(source.as_ref(), request, snapshot)
        }
    })
    .await
    .map_err(|_| "world generation task failed".to_owned())?
}

fn spawn_generation_assembly(
    batch: PendingGenerationBatch,
    source_identity_hash: voxels_world::WorldSourceIdentityHash,
    generation_limiter: Arc<PriorityGenerationLimiter>,
    response_cache: Arc<Mutex<BatchResponseCache>>,
    max_frame_bytes: usize,
) {
    tokio::spawn(async move {
        assemble_and_deliver_generation_batch(
            batch,
            source_identity_hash,
            generation_limiter,
            response_cache,
            max_frame_bytes,
        )
        .await;
    });
}

async fn assemble_and_deliver_generation_batch(
    batch: PendingGenerationBatch,
    source_identity_hash: voxels_world::WorldSourceIdentityHash,
    generation_limiter: Arc<PriorityGenerationLimiter>,
    response_cache: Arc<Mutex<BatchResponseCache>>,
    max_frame_bytes: usize,
) {
    if batch.job.tracked.is_cancelled() {
        batch.job.tracked.finish();
        return;
    }
    let request_id = batch.job.request.request_id();
    let Some(items) = batch.items.into_iter().collect::<Option<Vec<_>>>() else {
        deliver_generation_job(
            batch.job,
            Err("completed generation batch is missing a product".to_owned()),
            max_frame_bytes,
        )
        .await;
        return;
    };
    let products = match items.into_iter().collect::<Result<Vec<_>, _>>() {
        Ok(products) => products,
        Err(message) => {
            deliver_generation_job(batch.job, Err(message), max_frame_bytes).await;
            return;
        }
    };
    let response_key = BatchResponseKey::from_products(&products);
    match get_cached_batch_response(&response_cache, &response_key, request_id) {
        Ok(Some(response)) => {
            deliver_generation_job(batch.job, Ok(response), max_frame_bytes).await;
            return;
        }
        Ok(None) => {}
        Err(message) => {
            deliver_generation_job(batch.job, Err(message), max_frame_bytes).await;
            return;
        }
    }
    let priority = batch.job.request.priority();
    let session = Arc::clone(&batch.job.tracked.session);
    let session_permit = match session.acquire_generation(priority).await {
        Ok(permit) => permit,
        Err(_) => {
            batch.job.tracked.finish();
            return;
        }
    };
    let global_permit = generation_limiter.acquire(priority).await;
    if batch.job.tracked.is_cancelled() {
        batch.job.tracked.finish();
        return;
    }
    match get_cached_batch_response(&response_cache, &response_key, request_id) {
        Ok(Some(response)) => {
            drop(global_permit);
            drop(session_permit);
            deliver_generation_job(batch.job, Ok(response), max_frame_bytes).await;
            return;
        }
        Ok(None) => {}
        Err(message) => {
            drop(global_permit);
            drop(session_permit);
            deliver_generation_job(batch.job, Err(message), max_frame_bytes).await;
            return;
        }
    }
    let response_flight = { lock_batch_response_cache(&response_cache).flight_lock(&response_key) };
    let response_flight_guard = response_flight.lock().await;
    if batch.job.tracked.is_cancelled() {
        lock_batch_response_cache(&response_cache).finish_flight(&response_key, &response_flight);
        batch.job.tracked.finish();
        return;
    }
    match get_cached_batch_response(&response_cache, &response_key, request_id) {
        Ok(Some(response)) => {
            lock_batch_response_cache(&response_cache)
                .finish_flight(&response_key, &response_flight);
            drop(response_flight_guard);
            drop(global_permit);
            drop(session_permit);
            deliver_generation_job(batch.job, Ok(response), max_frame_bytes).await;
            return;
        }
        Ok(None) => {}
        Err(message) => {
            lock_batch_response_cache(&response_cache)
                .finish_flight(&response_key, &response_flight);
            drop(response_flight_guard);
            drop(global_permit);
            drop(session_permit);
            deliver_generation_job(batch.job, Err(message), max_frame_bytes).await;
            return;
        }
    }
    let request = batch.job.request.clone();
    let response = tokio::task::spawn_blocking(move || {
        assemble_generation_response(&request, source_identity_hash, &products)
    })
    .await
    .map_err(|_| "world response assembly task failed".to_owned())
    .and_then(|result| result);
    drop(global_permit);
    drop(session_permit);
    if let Ok(bytes) = &response
        && bytes.len() <= max_frame_bytes
    {
        lock_batch_response_cache(&response_cache)
            .insert(response_key.clone(), Arc::from(bytes.clone()));
    }
    lock_batch_response_cache(&response_cache).finish_flight(&response_key, &response_flight);
    drop(response_flight_guard);
    deliver_generation_job(batch.job, response, max_frame_bytes).await;
}

fn assemble_generation_response(
    request: &GenerationRequest,
    source_identity_hash: voxels_world::WorldSourceIdentityHash,
    products: &[Arc<EncodedProduct>],
) -> Result<Vec<u8>, String> {
    match request {
        GenerationRequest::Chunks { request, .. } => {
            let items = products
                .iter()
                .map(|product| match product.as_ref() {
                    EncodedProduct::Chunk(item) => Ok(item),
                    EncodedProduct::SurfaceTile(_) => {
                        Err("surface product cannot satisfy a chunk batch".to_owned())
                    }
                })
                .collect::<Result<Vec<_>, _>>()?;
            encode_chunk_batch_result_from_items(request.request_id, source_identity_hash, items)
                .map_err(|error| error.to_string())
        }
        GenerationRequest::SurfaceTiles { request, .. } => {
            let items = products
                .iter()
                .map(|product| match product.as_ref() {
                    EncodedProduct::SurfaceTile(item) => Ok(item),
                    EncodedProduct::Chunk(_) => {
                        Err("chunk product cannot satisfy a surface batch".to_owned())
                    }
                })
                .collect::<Result<Vec<_>, _>>()?;
            encode_surface_tile_batch_result_from_items(
                request.request_id,
                source_identity_hash,
                items,
            )
            .map_err(|error| error.to_string())
        }
    }
}

async fn deliver_generation_job(
    job: GenerationJob,
    response: Result<Vec<u8>, String>,
    max_frame_bytes: usize,
) {
    if job.tracked.is_cancelled() {
        job.tracked.finish();
        return;
    }
    let request_id = job.request.request_id();
    let bytes = match response {
        Ok(bytes) if bytes.len() <= max_frame_bytes => bytes,
        Ok(_) => encode_error(request_id, "chunk result exceeds configured frame limit"),
        Err(message) => encode_error(request_id, &message),
    };
    let byte_count = match u32::try_from(bytes.len()) {
        Ok(count) => count,
        Err(_) => {
            job.tracked.finish();
            return;
        }
    };
    let byte_permit = match Arc::clone(&job.tracked.session.outbound_bytes)
        .acquire_many_owned(byte_count)
        .await
    {
        Ok(permit) => permit,
        Err(_) => {
            job.tracked.finish();
            return;
        }
    };
    let frame = OutboundFrame {
        bytes,
        offset: 0,
        priority: traffic_priority(job.request.priority()),
        tracked: Some(job.tracked),
        _byte_permit: Some(byte_permit),
    };
    if let Err(error) = job.outbound.send(frame).await
        && let Some(tracked) = error.0.tracked
    {
        tracked.finish();
    }
}

fn traffic_priority(priority: WorldProductPriority) -> TrafficPriority {
    match priority {
        WorldProductPriority::CollisionCritical => TrafficPriority::Collision,
        WorldProductPriority::VisibleChunk
        | WorldProductPriority::VisibleSurface
        | WorldProductPriority::ReplacementSurface => TrafficPriority::VisibleWorld,
        WorldProductPriority::Prefetch => TrafficPriority::BackgroundWorld,
    }
}

fn generate_surface_tile_products(
    source: &dyn WorldSourceEngine,
    request: SurfaceTileBatchRequest,
    snapshot: SurfaceEditSnapshot,
) -> Result<Vec<EncodedProduct>, String> {
    let coords = request.coords.clone();
    let mut items = Vec::with_capacity(coords.len());
    if snapshot.edits.is_empty() {
        let result = source
            .generate_batch(WorldProductBatch {
                priority: request.priority,
                requests: coords
                    .iter()
                    .copied()
                    .map(WorldProductRequest::SurfaceTile)
                    .collect(),
            })
            .map_err(|error| error.to_string())?;
        if result.source_identity_hash != source.identity().identity_hash()
            || result.items.len() != coords.len()
        {
            return Err("world source returned a mismatched surface tile batch".to_owned());
        }
        for ((coord, edit_revision), item) in coords
            .iter()
            .copied()
            .zip(snapshot.revisions.iter().copied())
            .zip(result.items)
        {
            if item.request != WorldProductRequest::SurfaceTile(coord) {
                return Err("world source returned a mismatched surface tile key".to_owned());
            }
            let item_result = match item.result {
                Ok(WorldProduct::SurfaceTile(snapshot)) => Ok(snapshot),
                Ok(_) => return Err("world source returned a non-surface product".to_owned()),
                Err(error) => Err(error),
            };
            items.push(SurfaceTileBatchItem {
                coord,
                edit_revision,
                result: item_result,
            });
        }
    } else {
        for (coord, edit_revision) in coords
            .iter()
            .copied()
            .zip(snapshot.revisions.iter().copied())
        {
            items.push(SurfaceTileBatchItem {
                coord,
                edit_revision,
                result: source.generate_edited_surface_tile(&snapshot.edits, coord),
            });
        }
    }
    let source_identity_hash = source.identity().identity_hash();
    items
        .iter()
        .map(|item| {
            encode_surface_tile_batch_item(source_identity_hash, item)
                .map(EncodedProduct::SurfaceTile)
                .map_err(|error| error.to_string())
        })
        .collect()
}

fn generate_chunk_products(
    source: &dyn WorldSourceEngine,
    request: ChunkBatchRequest,
    snapshot: ChunkEditSnapshot,
) -> Result<Vec<EncodedProduct>, String> {
    let coords = request.coords.clone();
    let result = source
        .generate_batch(WorldProductBatch {
            priority: request.priority,
            requests: coords
                .iter()
                .copied()
                .map(WorldProductRequest::ChunkWithHalo)
                .collect(),
        })
        .map_err(|error| error.to_string())?;
    if result.source_identity_hash != source.identity().identity_hash()
        || result.items.len() != coords.len()
    {
        return Err("world source returned a mismatched chunk batch".to_owned());
    }
    let mut items = Vec::with_capacity(coords.len());
    for ((coord, edit_revision), item) in
        coords.into_iter().zip(snapshot.revisions).zip(result.items)
    {
        if item.request != WorldProductRequest::ChunkWithHalo(coord) {
            return Err("world source returned a mismatched chunk key".to_owned());
        }
        let item_result = match item.result {
            Ok(WorldProduct::Chunk(mut chunk)) => {
                let pristine_halo = chunk.meshing_halo.clone();
                snapshot.edits.apply_to_chunk(&mut chunk.chunk);
                chunk.meshing_halo = MeshingHalo::from_sampler(coord, |x, y, z| {
                    let voxel = voxels_world::VoxelCoord::new(x, y, z);
                    let pristine = pristine_halo.sample_world(x, y, z).unwrap_or(Material::Air);
                    snapshot.edits.resolve_generated(voxel, pristine)
                });
                Ok(chunk)
            }
            Ok(_) => return Err("world source returned a non-chunk product".to_owned()),
            Err(error) => Err(error),
        };
        items.push(ChunkBatchItem {
            coord,
            edit_revision,
            result: item_result,
        });
    }
    items
        .iter()
        .map(|item| {
            encode_chunk_batch_item(result.source_identity_hash, item)
                .map(EncodedProduct::Chunk)
                .map_err(|error| error.to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        EditPersistenceConfig, LoopbackTransportConfig, PresenceConfig, SpawnConfig,
        TerrainDiffusionProviderConfig, WORLD_SERVICE_CONFIG_SCHEMA_VERSION, WorldSourceMode,
    };
    use futures_util::{SinkExt, StreamExt};
    use std::sync::atomic::AtomicUsize;
    use tokio_tungstenite::connect_async;
    use tokio_tungstenite::tungstenite::Error as ClientError;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::http::HeaderValue;
    use tokio_tungstenite::tungstenite::protocol::Message as ClientMessage;
    use uuid::Uuid;
    use voxels_world::protocol::{
        BrowserUserId, ChunkBatchRequest, EditCommand, EditCommit, OpenPresence, OpenWorld,
        PLAYER_POSE_GROUNDED, PlayerId, PlayerIdentity, PlayerPoseUpdate, PresenceDelta,
        PresenceOpened, PresenceSessionId, SurfaceTileBatchRequest, decode_chunk_batch_result,
        decode_edit_commit, decode_error, decode_presence_delta, decode_presence_opened,
        decode_surface_tile_batch_result, decode_world_opened, edit_commit_kind,
        encode_chunk_batch, encode_edit_command, encode_open_presence, encode_open_world,
        encode_player_pose, encode_surface_tile_batch, presence_delta_kind,
    };
    use voxels_world::{
        ChunkCoord, ProceduralWorldSource, SurfaceLodLevel, SurfaceTileCoord, VoxelCoord,
        WorldProductPriority,
    };

    struct CountingSource {
        inner: ProceduralWorldSource,
        batch_calls: Arc<AtomicUsize>,
        product_calls: Arc<Mutex<HashMap<WorldProductRequest, usize>>>,
    }

    impl WorldSourceEngine for CountingSource {
        fn identity(&self) -> &voxels_world::WorldSourceIdentity {
            self.inner.identity()
        }

        fn generate_batch(
            &self,
            request: WorldProductBatch,
        ) -> Result<voxels_world::WorldProductBatchResult, WorldSourceError> {
            std::thread::sleep(std::time::Duration::from_millis(20));
            self.batch_calls.fetch_add(1, Ordering::Relaxed);
            let mut product_calls = self
                .product_calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            for product in &request.requests {
                *product_calls.entry(*product).or_default() += 1;
            }
            drop(product_calls);
            self.inner.generate_batch(request)
        }

        fn generate_edited_surface_tile(
            &self,
            edits: &voxels_world::EditMap,
            coord: SurfaceTileCoord,
        ) -> Result<voxels_world::SurfaceTileSnapshot, WorldSourceError> {
            self.inner.generate_edited_surface_tile(edits, coord)
        }

        fn surface_tiles_affected_by_voxel(
            &self,
            edits: &voxels_world::EditMap,
            level: SurfaceLodLevel,
            coord: voxels_world::VoxelCoord,
        ) -> Vec<SurfaceTileCoord> {
            self.inner
                .surface_tiles_affected_by_voxel(edits, level, coord)
        }

        fn atmosphere_sample(
            &self,
            x: i32,
            z: i32,
        ) -> (voxels_world::AtmosphereSample, voxels_world::SurfaceRegion) {
            self.inner.atmosphere_sample(x, z)
        }

        fn skyline_features_anchored_in(
            &self,
            bounds: [[i32; 2]; 2],
        ) -> Vec<voxels_world::SkylineFeature> {
            self.inner.skyline_features_anchored_in(bounds)
        }

        fn skyline_features_at(
            &self,
            coord: voxels_world::VoxelCoord,
        ) -> Vec<voxels_world::SkylineFeature> {
            self.inner.skyline_features_at(coord)
        }

        fn nearest_skyline_feature(
            &self,
            x: i32,
            z: i32,
            kind: voxels_world::SkylineFeatureKind,
            max_radius_cells: i32,
        ) -> Option<voxels_world::SkylineFeature> {
            self.inner
                .nearest_skyline_feature(x, z, kind, max_radius_cells)
        }

        fn nearest_prominent_skyline_feature(
            &self,
            x: i32,
            z: i32,
            kind: voxels_world::SkylineFeatureKind,
            max_radius_cells: i32,
        ) -> Option<voxels_world::SkylineFeature> {
            self.inner
                .nearest_prominent_skyline_feature(x, z, kind, max_radius_cells)
        }
    }

    #[tokio::test]
    async fn each_session_has_one_bounded_collision_generation_lane() {
        let session = Arc::new(SessionRequests::new(16, 2, 1, 1_024));
        let _ordinary_a = session
            .acquire_generation(WorldProductPriority::VisibleSurface)
            .await
            .expect("first ordinary permit");
        let _ordinary_b = session
            .acquire_generation(WorldProductPriority::VisibleChunk)
            .await
            .expect("second ordinary permit");
        let collision = session
            .acquire_generation(WorldProductPriority::CollisionCritical)
            .await
            .expect("collision lane must not wait behind ordinary work");

        assert!(
            tokio::time::timeout(
                Duration::from_millis(10),
                session.acquire_generation(WorldProductPriority::CollisionCritical),
            )
            .await
            .is_err(),
            "one session must not occupy unbounded critical generation workers"
        );

        drop(collision);
        let _reopened = tokio::time::timeout(
            Duration::from_secs(1),
            session.acquire_generation(WorldProductPriority::CollisionCritical),
        )
        .await
        .expect("collision lane must reopen")
        .expect("session limiter must remain open");
    }

    fn test_config() -> WorldServiceConfig {
        WorldServiceConfig {
            schema_version: WORLD_SERVICE_CONFIG_SCHEMA_VERSION,
            world_id: Uuid::from_bytes([7; 16]),
            world_seed: 42,
            source: WorldSourceMode::ProceduralV16,
            transport: LoopbackTransportConfig {
                listen: SocketAddr::from(([127, 0, 0, 1], 0)),
                allowed_origins: vec!["http://test.local".to_owned()],
                auth_subprotocol_token: "test-local-token".to_owned(),
                max_frame_bytes: voxels_world::protocol::MAX_PROTOCOL_FRAME_BYTES,
                max_queued_outbound_bytes_per_client: 32 * 1024 * 1024,
                outbound_bandwidth_floor_bytes_per_second: 96 * 1024,
                outbound_bandwidth_ceiling_bytes_per_second: 4 * 1024 * 1024,
                outbound_bandwidth_burst_bytes: 64 * 1024,
                outbound_queue_delay_target_ms: 25,
                outbound_feedback_timeout_ms: 3_000,
                outbound_max_frame_fragment_bytes:
                    voxels_world::protocol::MAX_FRAME_FRAGMENT_DATA_BYTES,
                max_in_flight_batches: 2,
                max_connections: 4,
                global_queue_capacity: 8,
                product_cache_bytes: 4 * 1024 * 1024,
                generation_workers: 4,
                generation_workers_per_client: 2,
                collision_generation_workers_per_client: 1,
            },
            presence: PresenceConfig {
                max_players: 4,
                ..PresenceConfig::default()
            },
            gameplay: crate::GameplayConfig::default(),
            environment: crate::EnvironmentConfig::default(),
            edits: EditPersistenceConfig::default(),
            spawn: SpawnConfig {
                xz_voxels: [13, -21],
                pillar_height_voxels: 1,
                pillar_radius_voxels: 1,
                protection_radius_voxels: 1,
                ..SpawnConfig::default()
            },
            terrain_diffusion: TerrainDiffusionProviderConfig::default(),
        }
    }

    #[test]
    fn environment_authority_advances_sun_and_clouds_from_one_server_clock() {
        let config = crate::EnvironmentConfig {
            day_length_seconds: 100.0,
            day_fraction_at_unix_epoch: 0.25,
            weather_cycle_seconds: 200.0,
            weather_fraction_at_unix_epoch: 0.1,
            cloud_offset_metres_at_unix_epoch: [10.0, 20.0],
            cloud_velocity_metres_per_second: [4.0, -2.0],
            cloud_coverage: 0.6,
            cloud_base_metres: 420.0,
            cloud_top_metres: 780.0,
            weather_seed: 7,
            weather_revision: 3,
            ..crate::EnvironmentConfig::default()
        };
        let authority = EnvironmentAuthority::from_anchor(config, 1_000, 0.0);
        let start = authority.snapshot(1_000);
        let later = authority.snapshot(26_000);
        let next_day = authority.snapshot(126_000);
        assert!((start.day_fraction - 0.25).abs() < f32::EPSILON);
        assert!((later.day_fraction - 0.5).abs() < 1.0e-6);
        assert_eq!(start.world_day_number, 0);
        assert_eq!(later.world_day_number, 0);
        assert_eq!(next_day.world_day_number, 1);
        assert!((next_day.day_fraction - 0.5).abs() < 1.0e-6);
        assert_eq!(later.days_per_year, 365.242_2);
        assert_eq!(later.moon_sidereal_orbit_days, 27.321_661);
        assert_eq!(later.celestial_revision, 1);
        assert!((start.weather_fraction - 0.1).abs() < f32::EPSILON);
        assert!((later.weather_fraction - 0.225).abs() < 1.0e-6);
        assert_eq!(start.cloud_offset_metres, [10.0, 20.0]);
        assert_eq!(later.cloud_offset_metres, [110.0, 1_279_970.0]);
        assert_eq!(later.weather_revision, 3);
    }

    #[test]
    fn zero_cycle_lengths_freeze_the_configured_environment_time() {
        let config = crate::EnvironmentConfig {
            day_length_seconds: 0.0,
            day_fraction_at_unix_epoch: 0.73,
            weather_cycle_seconds: 0.0,
            weather_fraction_at_unix_epoch: 0.68,
            ..crate::EnvironmentConfig::default()
        };
        let authority = EnvironmentAuthority::from_anchor(config, 10, 500.0);
        assert_eq!(authority.snapshot(10).day_fraction, 0.73);
        assert_eq!(authority.snapshot(9_000_010).day_fraction, 0.73);
        assert_eq!(authority.snapshot(10).weather_fraction, 0.68);
        assert_eq!(authority.snapshot(9_000_010).weather_fraction, 0.68);
    }

    fn player_identity(user: u8, player: u8, name: &str) -> PlayerIdentity {
        PlayerIdentity {
            browser_user_id: BrowserUserId::from_bytes([user; 16]),
            player_id: PlayerId::from_bytes([player; 16]),
            player_name: name.to_owned(),
        }
    }

    #[test]
    fn cancellation_keeps_request_id_stale_until_the_worker_finishes() {
        let session = SessionRequests::new(1, 1, 1, 1024);
        let cancelled = session.insert(7).ok();
        assert!(cancelled.is_some());
        assert!(session.cancel(7));
        assert!(matches!(
            session.insert(7),
            Err(RequestAdmissionError::Duplicate)
        ));
        if let Some(cancelled) = cancelled {
            assert!(cancelled.load(Ordering::Acquire));
            session.finish(7, &cancelled);
        }
        assert!(session.insert(7).is_ok());
    }

    #[test]
    fn encoded_product_cache_is_byte_bounded_and_lru() {
        fn product(x: i32) -> (ProductKey, Arc<EncodedProduct>) {
            let source = ProceduralWorldSource::new(42);
            let item = ChunkBatchItem {
                coord: ChunkCoord::new(x, 0, 0),
                edit_revision: 1,
                result: Err(WorldSourceError::InvalidChunkCoordinate),
            };
            let product = Arc::new(EncodedProduct::Chunk(
                encode_chunk_batch_item(source.source_identity_hash(), &item).expect("encode item"),
            ));
            (product.key(), product)
        }

        let (key1, product1) = product(1);
        let (key2, product2) = product(2);
        let (key3, product3) = product(3);
        let item_len = product1.encoded_len();
        let mut cache = ProductCache::new(item_len * 2);
        cache.insert(key1, product1);
        cache.insert(key2, product2);
        assert!(cache.get(&key1).is_some());
        cache.insert(key3, product3);
        assert!(cache.get(&key1).is_some());
        assert!(cache.get(&key2).is_none());
        assert!(cache.get(&key3).is_some());
        assert!(cache.retained_bytes <= cache.max_bytes);
        assert_eq!(cache.lru.len(), cache.entries.len());

        let (key4, product4) = product(4);
        let mut undersized = ProductCache::new(item_len - 1);
        undersized.insert(key4, product4);
        assert!(undersized.get(&key4).is_none());
        assert_eq!(undersized.retained_bytes, 0);
        assert!(undersized.lru.is_empty());
    }

    #[test]
    fn compressed_batch_cache_is_byte_bounded_lru_and_request_id_agnostic() {
        fn key(x: i32) -> BatchResponseKey {
            BatchResponseKey(
                vec![ProductKey::Chunk {
                    coord: ChunkCoord::new(x, 0, 0),
                    edit_revision: 1,
                }]
                .into_boxed_slice(),
            )
        }

        let bytes1 = Arc::<[u8]>::from(encode_error(1, "one"));
        let bytes2 = Arc::<[u8]>::from(encode_error(2, "two"));
        let bytes3 = Arc::<[u8]>::from(encode_error(3, "six"));
        assert_eq!(bytes1.len(), bytes2.len());
        assert_eq!(bytes2.len(), bytes3.len());
        let mut cache = BatchResponseCache::new(bytes1.len() * 2);
        cache.insert(key(1), Arc::clone(&bytes1));
        cache.insert(key(2), bytes2);
        assert!(cache.get(&key(1)).is_some());
        cache.insert(key(3), bytes3);
        assert!(cache.get(&key(1)).is_some());
        assert!(cache.get(&key(2)).is_none());
        assert!(cache.get(&key(3)).is_some());
        assert!(cache.retained_bytes <= cache.max_bytes);
        assert_eq!(cache.lru.len(), cache.entries.len());

        let cache = Mutex::new(cache);
        let cloned = get_cached_batch_response(&cache, &key(1), 99)
            .expect("cached frame remains valid")
            .expect("cached response");
        assert_eq!(
            voxels_world::protocol::decode_error(&cloned),
            Ok((99, "one".to_owned()))
        );

        let old_revision = key(1);
        let new_revision = BatchResponseKey(
            vec![ProductKey::Chunk {
                coord: ChunkCoord::new(1, 0, 0),
                edit_revision: 2,
            }]
            .into_boxed_slice(),
        );
        assert_ne!(old_revision, new_revision);
        assert!(
            get_cached_batch_response(&cache, &new_revision, 100)
                .expect("cache lookup")
                .is_none()
        );
    }

    #[test]
    fn compressed_batch_cache_serializes_assembly_per_product_sequence() {
        let key = BatchResponseKey(
            vec![ProductKey::Chunk {
                coord: ChunkCoord::new(1, 0, 0),
                edit_revision: 1,
            }]
            .into_boxed_slice(),
        );
        let mut cache = BatchResponseCache::new(1024);
        let first = cache.flight_lock(&key);
        let second = cache.flight_lock(&key);
        assert!(Arc::ptr_eq(&first, &second));
        cache.finish_flight(&key, &first);
        let next = cache.flight_lock(&key);
        assert!(!Arc::ptr_eq(&first, &next));
    }

    #[tokio::test]
    async fn authorized_client_opens_world_and_receives_a_chunk_batch()
    -> Result<(), Box<dyn std::error::Error>> {
        let config = test_config();
        let listener = TcpListener::bind(config.transport.listen).await?;
        let address = listener.local_addr()?;
        let server = WorldServer::new(
            config.clone(),
            Box::new(ProceduralWorldSource::new(config.world_seed)),
        )?;
        let server_task = tokio::spawn(server.serve(listener));

        let mut denied = format!("ws://{address}{WORLD_WEBSOCKET_PATH}").into_client_request()?;
        denied
            .headers_mut()
            .insert(ORIGIN, HeaderValue::from_static("http://test.local"));
        match connect_async(denied).await {
            Err(ClientError::Http(response)) => {
                assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            }
            Ok(_) => return Err("unauthenticated WebSocket unexpectedly connected".into()),
            Err(error) => return Err(error.into()),
        }

        let mut denied = format!("ws://{address}{WORLD_WEBSOCKET_PATH}").into_client_request()?;
        denied
            .headers_mut()
            .insert(ORIGIN, HeaderValue::from_static("http://test.local"));
        match connect_async(denied).await {
            Err(ClientError::Http(response)) => {
                assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            }
            Ok(_) => return Err("unauthenticated WebSocket unexpectedly connected".into()),
            Err(error) => return Err(error.into()),
        }

        let mut request = format!("ws://{address}{WORLD_WEBSOCKET_PATH}").into_client_request()?;
        request
            .headers_mut()
            .insert(ORIGIN, HeaderValue::from_static("http://test.local"));
        request.headers_mut().insert(
            SEC_WEBSOCKET_PROTOCOL,
            HeaderValue::from_static("voxels.world.v24, test-local-token"),
        );
        let (mut socket, response) = connect_async(request).await?;
        assert_eq!(
            response.headers().get(SEC_WEBSOCKET_PROTOCOL),
            Some(&HeaderValue::from_static(WORLD_WEBSOCKET_PROTOCOL))
        );
        let identity = player_identity(1, 2, "default");
        socket
            .send(ClientMessage::Binary(
                encode_open_world(&OpenWorld {
                    max_in_flight_batches: 2,
                    identity: identity.clone(),
                })?
                .into(),
            ))
            .await?;
        let opened_bytes = next_client_binary(&mut socket).await?;
        let opened = decode_world_opened(&opened_bytes)?;
        assert_eq!([opened.spawn.x, opened.spawn.z], config.spawn.xz_voxels);
        assert_eq!(opened.manifest.world_id, config.canonical_world_id());
        assert_eq!(opened.recommended_in_flight_batches, 2);
        assert_eq!(opened.identity, identity);
        assert!(opened.capabilities.contains(WorldCapabilities::SURFACE_LOD));
        assert!(opened.capabilities.contains(WorldCapabilities::GLIDING));
        let spawn_top = VoxelCoord::new(opened.spawn.x, opened.spawn.height, opened.spawn.z);
        let expected_eye_y = (opened.spawn.height + 1) as f32 * voxels_world::VOXEL_SIZE_METRES
            + DEFAULT_PLAYER_EYE_HEIGHT_METRES
            + 0.02;
        assert!((opened.player_resume.eye_position_metres[1] - expected_eye_y).abs() < 0.000_1);

        let batch = ChunkBatchRequest {
            request_id: 9,
            priority: WorldProductPriority::VisibleChunk,
            coords: vec![spawn_top.chunk()],
        };
        socket
            .send(ClientMessage::Binary(encode_chunk_batch(&batch)?.into()))
            .await?;
        let result_bytes = next_client_binary(&mut socket).await?;
        let result = decode_chunk_batch_result(&result_bytes)?;
        assert_eq!(result.request_id, batch.request_id);
        assert_eq!(result.items.len(), 1);
        assert!(result.items[0].result.is_ok());
        let chunk = &result.items[0].result.as_ref().unwrap().chunk;
        let [x, y, z] = spawn_top.local();
        assert_eq!(chunk.get(x, y, z), config.spawn.pillar_material);

        let cached_batch = ChunkBatchRequest {
            request_id: 10,
            ..batch
        };
        socket
            .send(ClientMessage::Binary(
                encode_chunk_batch(&cached_batch)?.into(),
            ))
            .await?;
        let cached = decode_chunk_batch_result(&next_client_binary(&mut socket).await?)?;
        assert_eq!(cached.request_id, cached_batch.request_id);
        assert_eq!(cached.items, result.items);

        socket.close(None).await?;
        server_task.abort();
        let _ = server_task.await;
        Ok(())
    }

    #[tokio::test]
    async fn overlapping_batches_single_flight_each_product_across_clients()
    -> Result<(), Box<dyn std::error::Error>> {
        let config = test_config();
        let listener = TcpListener::bind(config.transport.listen).await?;
        let address = listener.local_addr()?;
        let batch_calls = Arc::new(AtomicUsize::new(0));
        let product_calls = Arc::new(Mutex::new(HashMap::new()));
        let source = CountingSource {
            inner: ProceduralWorldSource::new(config.world_seed),
            batch_calls: Arc::clone(&batch_calls),
            product_calls: Arc::clone(&product_calls),
        };
        let server = WorldServer::new(config, Box::new(source))?;
        batch_calls.store(0, Ordering::Relaxed);
        let server_task = tokio::spawn(server.serve(listener));

        let (mut first, _) = connect_test_client(address, player_identity(1, 2, "alice")).await?;
        let (mut second, _) = connect_test_client(address, player_identity(1, 3, "bob")).await?;
        let a = ChunkCoord::new(4, 0, -7);
        let b = ChunkCoord::new(5, 0, -7);
        let c = ChunkCoord::new(6, 0, -7);
        for (socket, request_id, coords) in
            [(&mut first, 41, vec![a, b]), (&mut second, 99, vec![b, c])]
        {
            socket
                .send(ClientMessage::Binary(
                    encode_chunk_batch(&ChunkBatchRequest {
                        request_id,
                        priority: WorldProductPriority::VisibleChunk,
                        coords,
                    })?
                    .into(),
                ))
                .await?;
        }
        let first_result = decode_chunk_batch_result(&next_client_binary(&mut first).await?)?;
        let second_result = decode_chunk_batch_result(&next_client_binary(&mut second).await?)?;
        assert_eq!(first_result.request_id, 41);
        assert_eq!(second_result.request_id, 99);
        assert_eq!(
            first_result
                .items
                .iter()
                .map(|item| item.coord)
                .collect::<Vec<_>>(),
            vec![a, b]
        );
        assert_eq!(
            second_result
                .items
                .iter()
                .map(|item| item.coord)
                .collect::<Vec<_>>(),
            vec![b, c]
        );
        assert_eq!(batch_calls.load(Ordering::Relaxed), 2);
        {
            let counts = product_calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            for coord in [a, b, c] {
                assert_eq!(
                    counts.get(&WorldProductRequest::ChunkWithHalo(coord)),
                    Some(&1)
                );
            }
        }

        first
            .send(ClientMessage::Binary(
                encode_chunk_batch(&ChunkBatchRequest {
                    request_id: 42,
                    priority: WorldProductPriority::Prefetch,
                    coords: vec![c, b, a],
                })?
                .into(),
            ))
            .await?;
        let cached = decode_chunk_batch_result(&next_client_binary(&mut first).await?)?;
        assert_eq!(cached.request_id, 42);
        assert_eq!(
            cached
                .items
                .iter()
                .map(|item| item.coord)
                .collect::<Vec<_>>(),
            vec![c, b, a]
        );
        assert_eq!(batch_calls.load(Ordering::Relaxed), 2);

        let surface_a = SurfaceTileCoord::new(SurfaceLodLevel::Stride16, 20, -12);
        let surface_b = SurfaceTileCoord::new(SurfaceLodLevel::Stride16, 21, -12);
        let surface_c = SurfaceTileCoord::new(SurfaceLodLevel::Stride16, 22, -12);
        for (socket, request_id, coords) in [
            (&mut first, 43, vec![surface_a, surface_b]),
            (&mut second, 100, vec![surface_b, surface_c]),
        ] {
            socket
                .send(ClientMessage::Binary(
                    encode_surface_tile_batch(&SurfaceTileBatchRequest {
                        request_id,
                        priority: WorldProductPriority::VisibleSurface,
                        coords,
                    })?
                    .into(),
                ))
                .await?;
        }
        let first_surface =
            decode_surface_tile_batch_result(&next_client_binary(&mut first).await?)?;
        let second_surface =
            decode_surface_tile_batch_result(&next_client_binary(&mut second).await?)?;
        assert_eq!(
            first_surface
                .items
                .iter()
                .map(|item| item.coord)
                .collect::<Vec<_>>(),
            vec![surface_a, surface_b]
        );
        assert_eq!(
            second_surface
                .items
                .iter()
                .map(|item| item.coord)
                .collect::<Vec<_>>(),
            vec![surface_b, surface_c]
        );
        assert_eq!(batch_calls.load(Ordering::Relaxed), 4);
        {
            let counts = product_calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            for coord in [surface_a, surface_b, surface_c] {
                assert_eq!(
                    counts.get(&WorldProductRequest::SurfaceTile(coord)),
                    Some(&1)
                );
            }
        }

        first
            .send(ClientMessage::Binary(
                encode_surface_tile_batch(&SurfaceTileBatchRequest {
                    request_id: 44,
                    priority: WorldProductPriority::Prefetch,
                    coords: vec![surface_c, surface_b, surface_a],
                })?
                .into(),
            ))
            .await?;
        let cached_surface =
            decode_surface_tile_batch_result(&next_client_binary(&mut first).await?)?;
        assert_eq!(
            cached_surface
                .items
                .iter()
                .map(|item| item.coord)
                .collect::<Vec<_>>(),
            vec![surface_c, surface_b, surface_a]
        );
        assert_eq!(batch_calls.load(Ordering::Relaxed), 4);

        first.close(None).await?;
        second.close(None).await?;
        server_task.abort();
        let _ = server_task.await;
        Ok(())
    }

    #[tokio::test]
    async fn two_clients_stream_independent_locations_with_session_scoped_ids()
    -> Result<(), Box<dyn std::error::Error>> {
        let config = test_config();
        let listener = TcpListener::bind(config.transport.listen).await?;
        let address = listener.local_addr()?;
        let server = WorldServer::new(
            config.clone(),
            Box::new(ProceduralWorldSource::new(config.world_seed)),
        )?;
        let server_task = tokio::spawn(server.serve(listener));

        let first_identity = player_identity(1, 2, "alice");
        let second_identity = player_identity(1, 3, "bob");
        let (mut first, first_opened) =
            connect_test_client(address, first_identity.clone()).await?;
        let (mut second, second_opened) =
            connect_test_client(address, second_identity.clone()).await?;
        assert_eq!(first_opened.identity, first_identity.clone());
        assert_eq!(second_opened.identity, second_identity);

        let (mut duplicate, duplicate_response) =
            connect_test_client_raw(address, first_identity).await?;
        let (request_id, message) = decode_error(&duplicate_response)?;
        assert_eq!(request_id, 0);
        assert_eq!(message, "player is already connected");
        duplicate.close(None).await?;
        let first_coord = SurfaceTileCoord::new(SurfaceLodLevel::Stride16, -400, 200);
        let second_coord = SurfaceTileCoord::new(SurfaceLodLevel::Stride16, 700, -600);
        for (socket, coord) in [(&mut first, first_coord), (&mut second, second_coord)] {
            let request = SurfaceTileBatchRequest {
                request_id: 7,
                priority: WorldProductPriority::VisibleSurface,
                coords: vec![coord],
            };
            socket
                .send(ClientMessage::Binary(
                    encode_surface_tile_batch(&request)?.into(),
                ))
                .await?;
        }
        let first_result =
            decode_surface_tile_batch_result(&next_client_binary(&mut first).await?)?;
        let second_result =
            decode_surface_tile_batch_result(&next_client_binary(&mut second).await?)?;
        assert_eq!(first_result.request_id, 7);
        assert_eq!(second_result.request_id, 7);
        assert_eq!(first_result.items[0].coord, first_coord);
        assert_eq!(second_result.items[0].coord, second_coord);
        assert!(first_result.items[0].result.is_ok());
        assert!(second_result.items[0].result.is_ok());

        first.close(None).await?;
        second.close(None).await?;
        server_task.abort();
        let _ = server_task.await;
        Ok(())
    }

    #[tokio::test]
    async fn two_presence_channels_receive_personalized_enters_and_explicit_disconnect()
    -> Result<(), Box<dyn std::error::Error>> {
        let config = test_config();
        let listener = TcpListener::bind(config.transport.listen).await?;
        let address = listener.local_addr()?;
        let server = WorldServer::new(
            config.clone(),
            Box::new(ProceduralWorldSource::new(config.world_seed)),
        )?;
        let server_task = tokio::spawn(server.serve(listener));

        let (mut first_world, first_opened) =
            connect_test_client(address, player_identity(1, 2, "alice")).await?;
        let (mut second_world, second_opened) =
            connect_test_client(address, player_identity(1, 3, "bob")).await?;
        let (mut first_presence, first_presence_opened) =
            connect_test_presence(address, first_opened.presence_session_id).await?;
        let (mut second_presence, second_presence_opened) =
            connect_test_presence(address, second_opened.presence_session_id).await?;
        assert_eq!(
            first_presence_opened.connection_id,
            first_opened.connection_id
        );
        assert_eq!(
            second_presence_opened.connection_id,
            second_opened.connection_id
        );

        let mut first_eye = first_opened.player_resume.eye_position_metres;
        let mut second_eye = second_opened.player_resume.eye_position_metres;
        first_eye[0] -= 0.25;
        second_eye[0] += 0.25;
        for (socket, sequence, eye) in [
            (&mut first_presence, 1_u64, first_eye),
            (&mut second_presence, 1_u64, second_eye),
        ] {
            socket
                .send(ClientMessage::Binary(
                    encode_player_pose(PlayerPoseUpdate {
                        sequence,
                        sample_server_time_ms: 0,
                        eye_position_metres: eye,
                        linear_velocity_metres_per_second: [0.0, 0.0, 0.0],
                        look_yaw_radians: 0.25,
                        look_pitch_radians: -0.1,
                        flags: PLAYER_POSE_GROUNDED,
                    })?
                    .into(),
                ))
                .await?;
        }

        let first_delta = next_presence_delta(&mut first_presence, 1, 0).await?;
        let second_delta = next_presence_delta(&mut second_presence, 1, 0).await?;
        assert_eq!(
            first_delta.enters[0].player_id,
            second_opened.identity.player_id
        );
        assert_eq!(
            second_delta.enters[0].player_id,
            first_opened.identity.player_id
        );
        assert_ne!(
            first_delta.enters[0].color_index,
            second_delta.enters[0].color_index
        );

        first_world.close(None).await?;
        let remaining = next_presence_delta(&mut second_presence, 0, 1).await?;
        assert_eq!(remaining.leaves, vec![first_opened.connection_id]);

        second_world.close(None).await?;
        first_presence.close(None).await?;
        second_presence.close(None).await?;
        server_task.abort();
        let _ = server_task.await;
        Ok(())
    }

    #[tokio::test]
    async fn five_builders_stream_a_far_lod_tower_only_to_interested_players()
    -> Result<(), Box<dyn std::error::Error>> {
        const BUILDER_COUNT: usize = 5;
        const OBSERVER_INDEX: usize = BUILDER_COUNT;
        const FAR_INDEX: usize = BUILDER_COUNT + 1;

        let mut config = test_config();
        config.transport.max_connections = 8;
        config.presence.max_players = 8;
        config.presence.spatial_cell_metres = 8;
        config.presence.interest_radius_metres = 32;
        config.presence.interest_hysteresis_metres = 4;
        config.presence.near_radius_metres = 8;
        config.presence.mid_radius_metres = 16;
        config.gameplay.max_horizontal_speed_centimetres_per_second = 2_000;
        config.gameplay.movement_slack_centimetres = 300;
        config.gameplay.movement_credit_window_ms = 2_000;
        let listener = TcpListener::bind(config.transport.listen).await?;
        let address = listener.local_addr()?;
        let server = WorldServer::new(
            config.clone(),
            Box::new(ProceduralWorldSource::new(config.world_seed)),
        )?;
        let server_task = tokio::spawn(server.serve(listener));

        let mut worlds = Vec::new();
        let mut presences = Vec::new();
        let mut opened = Vec::new();
        for index in 0..=FAR_INDEX {
            let identity = player_identity(
                40 + index as u8,
                60 + index as u8,
                if index < BUILDER_COUNT {
                    match index {
                        0 => "builder0",
                        1 => "builder1",
                        2 => "builder2",
                        3 => "builder3",
                        _ => "builder4",
                    }
                } else if index == OBSERVER_INDEX {
                    "observer"
                } else {
                    "faraway"
                },
            );
            let (world, world_opened) = connect_test_client(address, identity).await?;
            let (presence, _) =
                connect_test_presence(address, world_opened.presence_session_id).await?;
            worlds.push(world);
            presences.push(presence);
            opened.push(world_opened);
        }

        let spawn = opened[0].spawn;
        tokio::time::sleep(std::time::Duration::from_millis(2_000)).await;
        for (index, presence) in presences.iter_mut().enumerate() {
            let x_offset = if index < BUILDER_COUNT {
                index as f32 * 0.4
            } else if index == OBSERVER_INDEX {
                25.0
            } else {
                -40.0
            };
            let mut eye = opened[index].player_resume.eye_position_metres;
            eye[0] += x_offset;
            presence
                .send(ClientMessage::Binary(
                    encode_player_pose(PlayerPoseUpdate {
                        sequence: 1,
                        sample_server_time_ms: 0,
                        eye_position_metres: eye,
                        linear_velocity_metres_per_second: [0.0, 0.0, 0.0],
                        look_yaw_radians: 0.0,
                        look_pitch_radians: 0.0,
                        flags: PLAYER_POSE_GROUNDED,
                    })?
                    .into(),
                ))
                .await?;
        }
        let expected_builders = opened
            .iter()
            .take(BUILDER_COUNT)
            .map(|opened| opened.connection_id)
            .collect::<BTreeSet<_>>();
        let observer_delta =
            wait_for_visible_connections(&mut presences[OBSERVER_INDEX], &expected_builders)
                .await?;
        assert_eq!(
            observer_delta.visible_player_count as usize, BUILDER_COUNT,
            "the far-tier observer should see all builders but not the out-of-interest bystander"
        );

        let dig_hits = (0..BUILDER_COUNT)
            .map(|index| VoxelCoord::new(spawn.x + 4 + index as i32 * 6, spawn.height - 1, spawn.z))
            .collect::<Vec<_>>();
        for (index, hit) in dig_hits.iter().copied().enumerate() {
            worlds[index]
                .send(ClientMessage::Binary(
                    encode_edit_command(EditCommand {
                        operation_id: 50 + index as u64,
                        edit_session_id: opened[index].edit_session_id,
                        action: voxels_world::protocol::EditAction::Dig { hit },
                    })?
                    .into(),
                ))
                .await?;
        }

        let mut builder_materials = vec![Material::Air; BUILDER_COUNT];
        for (client_index, world) in worlds.iter_mut().take(OBSERVER_INDEX + 1).enumerate() {
            for _ in 0..BUILDER_COUNT {
                let (commit, _) = next_edit_commit(world).await?;
                if client_index < BUILDER_COUNT && commit.operation_id == 50 + client_index as u64 {
                    let inventory = commit
                        .editor_inventory
                        .ok_or("builder dig omitted private inventory")?;
                    builder_materials[client_index] = Material::ALL
                        .into_iter()
                        .find(|material| {
                            !matches!(material, Material::Air | Material::Water)
                                && inventory.count(*material) > 0
                        })
                        .ok_or("builder dig earned no placeable solid material")?;
                }
            }
        }
        assert!(
            builder_materials
                .iter()
                .all(|material| *material != Material::Air)
        );

        let tower_base = spawn.water_level.unwrap_or(spawn.height).max(spawn.height) + 1;
        let tower = (0..BUILDER_COUNT)
            .map(|index| VoxelCoord::new(spawn.x + 4, tower_base + index as i32, spawn.z))
            .collect::<Vec<_>>();
        for (index, coord) in tower.iter().copied().enumerate() {
            worlds[index]
                .send(ClientMessage::Binary(
                    encode_edit_command(EditCommand {
                        operation_id: 100 + index as u64,
                        edit_session_id: opened[index].edit_session_id,
                        action: voxels_world::protocol::EditAction::Place {
                            coord,
                            material: builder_materials[index],
                        },
                    })?
                    .into(),
                ))
                .await?;
        }

        let mut observer_edit_bytes = 0;
        let mut observer_commits = Vec::new();
        let mut first_edit_revision = None;
        for (index, world) in worlds.iter_mut().take(OBSERVER_INDEX + 1).enumerate() {
            let mut commits = Vec::new();
            for _ in 0..BUILDER_COUNT {
                let (commit, encoded_bytes) = next_edit_commit(world).await?;
                if index == OBSERVER_INDEX {
                    observer_edit_bytes += encoded_bytes;
                    observer_commits.push(commit.clone());
                }
                commits.push(commit);
            }
            if index == 0 {
                first_edit_revision = commits
                    .iter()
                    .find(|commit| commit.operation_id == 100)
                    .map(|commit| commit.revision);
            }
            commits.sort_unstable_by_key(|commit| commit.revision);
            assert_eq!(
                commits
                    .iter()
                    .map(|commit| commit.revision)
                    .collect::<Vec<_>>(),
                vec![7, 8, 9, 10, 11]
            );
            let mut committed_coords = commits
                .iter()
                .flat_map(|commit| commit.mutations.iter().map(|mutation| mutation.coord))
                .collect::<Vec<_>>();
            committed_coords.sort_unstable();
            assert_eq!(committed_coords, tower);
        }
        assert!(
            observer_edit_bytes < 16 * 1024,
            "five sparse commits used {observer_edit_bytes} bytes"
        );
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(150),
                next_client_binary(&mut worlds[FAR_INDEX]),
            )
            .await
            .is_err(),
            "an out-of-interest player received an unrelated edit frame"
        );

        let first_edit = EditCommand {
            operation_id: 100,
            edit_session_id: opened[0].edit_session_id,
            action: voxels_world::protocol::EditAction::Place {
                coord: tower[0],
                material: builder_materials[0],
            },
        };
        worlds[0]
            .send(ClientMessage::Binary(
                encode_edit_command(first_edit)?.into(),
            ))
            .await?;
        let (retry_commit, _) = next_edit_commit(&mut worlds[0]).await?;
        assert_eq!(Some(retry_commit.revision), first_edit_revision);
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(150),
                next_client_binary(&mut worlds[OBSERVER_INDEX]),
            )
            .await
            .is_err(),
            "an idempotent retry was redundantly fanned out to the observer"
        );

        let mut chunk_coords = tower.iter().map(|coord| coord.chunk()).collect::<Vec<_>>();
        chunk_coords.sort_unstable();
        chunk_coords.dedup();
        worlds[OBSERVER_INDEX]
            .send(ClientMessage::Binary(
                encode_chunk_batch(&ChunkBatchRequest {
                    request_id: 900,
                    priority: WorldProductPriority::VisibleChunk,
                    coords: chunk_coords,
                })?
                .into(),
            ))
            .await?;
        let chunks = decode_chunk_batch_result(
            &tokio::time::timeout(
                std::time::Duration::from_secs(2),
                next_client_binary(&mut worlds[OBSERVER_INDEX]),
            )
            .await??,
        )?;
        assert_eq!(chunks.request_id, 900);
        assert!(chunks.items.iter().all(|item| item.edit_revision == 11));
        for (coord, material) in tower.iter().zip(&builder_materials) {
            let snapshot = chunks
                .items
                .iter()
                .find(|item| item.coord == coord.chunk())
                .and_then(|item| item.result.as_ref().ok())
                .ok_or("observer did not receive the edited tower chunk")?;
            let [x, y, z] = coord.local();
            assert_eq!(snapshot.chunk.get(x, y, z), *material);
        }

        let coarse_coord =
            SurfaceTileCoord::containing(SurfaceLodLevel::Stride16, tower[0].x, tower[0].z);
        worlds[OBSERVER_INDEX]
            .send(ClientMessage::Binary(
                encode_surface_tile_batch(&SurfaceTileBatchRequest {
                    request_id: 901,
                    priority: WorldProductPriority::VisibleSurface,
                    coords: vec![coarse_coord],
                })?
                .into(),
            ))
            .await?;
        let surface = decode_surface_tile_batch_result(
            &tokio::time::timeout(
                std::time::Duration::from_secs(2),
                next_client_binary(&mut worlds[OBSERVER_INDEX]),
            )
            .await??,
        )?;
        let item = surface.items.first().ok_or("missing coarse surface tile")?;
        assert_eq!(item.coord, coarse_coord);
        let expected_surface_revision = observer_commits
            .iter()
            .filter(|commit| commit.affected_surface_tiles.contains(&coarse_coord))
            .map(|commit| commit.revision)
            .max()
            .ok_or("tower commits did not invalidate their coarse surface tile")?;
        assert_eq!(item.edit_revision, expected_surface_revision);
        let snapshot = item.result.as_ref().map_err(|error| error.to_string())?;
        let tower_top = *tower.last().unwrap();
        assert!(
            snapshot.terrain.quads.iter().any(|quad| {
                quad.material == builder_materials[BUILDER_COUNT - 1]
                    && quad.origin[1] == tower_top.y
                    && quad.origin[0] <= tower_top.x
                    && tower_top.x < quad.origin[0] + i32::from(quad.extent[0])
                    && quad.origin[2] <= tower_top.z
                    && tower_top.z < quad.origin[2] + i32::from(quad.extent[1])
            }),
            "the five-player tower vanished from the 1.6 m coarse LOD seen at 120 m"
        );

        for world in &mut worlds {
            world.close(None).await?;
        }
        for presence in &mut presences {
            presence.close(None).await?;
        }
        server_task.abort();
        let _ = server_task.await;
        Ok(())
    }

    #[tokio::test]
    async fn digging_converges_editor_observer_inventory_chunks_halos_and_surface_lod()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut config = test_config();
        config.transport.max_connections = 3;
        config.presence.max_players = 3;
        let listener = TcpListener::bind(config.transport.listen).await?;
        let address = listener.local_addr()?;
        let server = WorldServer::new(
            config.clone(),
            Box::new(ProceduralWorldSource::new(config.world_seed)),
        )?;
        let server_task = tokio::spawn(server.serve(listener));

        let (mut editor_world, editor_opened) =
            connect_test_client(address, player_identity(80, 80, "digger")).await?;
        let (mut observer_world, observer_opened) =
            connect_test_client(address, player_identity(81, 81, "observer")).await?;
        let (mut editor_presence, _) =
            connect_test_presence(address, editor_opened.presence_session_id).await?;
        let (mut observer_presence, _) =
            connect_test_presence(address, observer_opened.presence_session_id).await?;

        for (socket, opened) in [
            (&mut editor_presence, &editor_opened),
            (&mut observer_presence, &observer_opened),
        ] {
            socket
                .send(ClientMessage::Binary(
                    encode_player_pose(PlayerPoseUpdate {
                        sequence: 1,
                        sample_server_time_ms: 0,
                        eye_position_metres: opened.player_resume.eye_position_metres,
                        linear_velocity_metres_per_second: [0.0, 0.0, 0.0],
                        look_yaw_radians: 0.0,
                        look_pitch_radians: 0.0,
                        flags: PLAYER_POSE_GROUNDED,
                    })?
                    .into(),
                ))
                .await?;
        }

        let hit = VoxelCoord::new(
            editor_opened.spawn.x + 4,
            editor_opened.spawn.height - 1,
            editor_opened.spawn.z,
        );
        let dig_coords = DigVolume::for_hit(hit)
            .expect("bounded server test dig")
            .coordinates()
            .collect::<Vec<_>>();
        let requested_chunks = dig_coords
            .iter()
            .flat_map(|coord| voxels_world::EditMap::affected_chunks(*coord))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        assert!(requested_chunks.len() <= voxels_world::protocol::MAX_EDIT_AFFECTED_CHUNKS);

        observer_world
            .send(ClientMessage::Binary(
                encode_chunk_batch(&ChunkBatchRequest {
                    request_id: 1_000,
                    priority: WorldProductPriority::VisibleChunk,
                    coords: requested_chunks.clone(),
                })?
                .into(),
            ))
            .await?;
        let before = decode_chunk_batch_result(&next_client_binary(&mut observer_world).await?)?;
        assert_eq!(before.request_id, 1_000);
        assert!(before.items.iter().all(|item| item.edit_revision == 1));
        let before_material = |coord: VoxelCoord| {
            let item = before
                .items
                .iter()
                .find(|item| item.coord == coord.chunk())
                .expect("pre-dig batch omitted an owner chunk");
            let chunk = &item
                .result
                .as_ref()
                .expect("pre-dig chunk generation failed")
                .chunk;
            let [x, y, z] = coord.local();
            chunk.get(x, y, z)
        };
        let expected_mutations = dig_coords
            .iter()
            .copied()
            .filter(|coord| before_material(*coord) != Material::Air)
            .collect::<Vec<_>>();
        assert!(!expected_mutations.is_empty());
        assert!(expected_mutations.len() < dig_coords.len());
        let unrelated = VoxelCoord::new(hit.x + 3, hit.y, hit.z);
        let unrelated_before = before_material(unrelated);

        let command = EditCommand {
            operation_id: 1,
            edit_session_id: editor_opened.edit_session_id,
            action: voxels_world::protocol::EditAction::Dig { hit },
        };
        editor_world
            .send(ClientMessage::Binary(encode_edit_command(command)?.into()))
            .await?;
        let (editor_commit, _) = next_edit_commit(&mut editor_world).await?;
        let (observer_commit, _) = next_edit_commit(&mut observer_world).await?;
        assert_eq!(
            observer_commit,
            EditCommit {
                editor_inventory: None,
                ..editor_commit.clone()
            }
        );
        assert_eq!(
            editor_commit
                .mutations
                .iter()
                .map(|mutation| mutation.coord)
                .collect::<Vec<_>>(),
            expected_mutations
        );
        assert!(
            editor_commit
                .mutations
                .iter()
                .all(|mutation| mutation.material == Material::Air)
        );
        let expected_affected = editor_commit
            .mutations
            .iter()
            .flat_map(|mutation| voxels_world::EditMap::affected_chunks(mutation.coord))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        assert_eq!(editor_commit.affected_chunks, expected_affected);

        let inventory = editor_commit.editor_inventory.expect("editor inventory");
        assert_eq!(inventory.revision, editor_opened.inventory.revision + 1);
        for material in Material::ALL {
            let expected_gain = expected_mutations
                .iter()
                .filter(|coord| before_material(**coord) == material)
                .count() as u64;
            assert_eq!(
                inventory.count(material),
                editor_opened.inventory.count(material) + expected_gain,
                "wrong authoritative inventory delta for material {}",
                material.id()
            );
        }

        observer_world
            .send(ClientMessage::Binary(
                encode_chunk_batch(&ChunkBatchRequest {
                    request_id: 1_001,
                    priority: WorldProductPriority::VisibleChunk,
                    coords: requested_chunks,
                })?
                .into(),
            ))
            .await?;
        let after = decode_chunk_batch_result(&next_client_binary(&mut observer_world).await?)?;
        assert_eq!(after.request_id, 1_001);
        for mutation in &editor_commit.mutations {
            let owner = after
                .items
                .iter()
                .find(|item| item.coord == mutation.coord.chunk())
                .and_then(|item| item.result.as_ref().ok())
                .expect("post-dig owner chunk");
            let [x, y, z] = mutation.coord.local();
            assert_eq!(owner.chunk.get(x, y, z), Material::Air);
            for coord in voxels_world::EditMap::affected_chunks(mutation.coord) {
                if coord == mutation.coord.chunk() {
                    continue;
                }
                let neighbor = after
                    .items
                    .iter()
                    .find(|item| item.coord == coord)
                    .and_then(|item| item.result.as_ref().ok())
                    .expect("post-dig halo neighbor");
                assert_eq!(
                    neighbor.meshing_halo.sample_world(
                        mutation.coord.x,
                        mutation.coord.y,
                        mutation.coord.z
                    ),
                    Some(Material::Air)
                );
            }
        }
        let unrelated_item = after
            .items
            .iter()
            .find(|item| item.coord == unrelated.chunk())
            .and_then(|item| item.result.as_ref().ok())
            .expect("post-dig unrelated owner");
        let [x, y, z] = unrelated.local();
        assert_eq!(unrelated_item.chunk.get(x, y, z), unrelated_before);

        assert!(!editor_commit.affected_surface_tiles.is_empty());
        let surface_coord = editor_commit.affected_surface_tiles[0];
        observer_world
            .send(ClientMessage::Binary(
                encode_surface_tile_batch(&SurfaceTileBatchRequest {
                    request_id: 1_002,
                    priority: WorldProductPriority::VisibleSurface,
                    coords: vec![surface_coord],
                })?
                .into(),
            ))
            .await?;
        let surface =
            decode_surface_tile_batch_result(&next_client_binary(&mut observer_world).await?)?;
        assert_eq!(surface.request_id, 1_002);
        assert_eq!(surface.items[0].coord, surface_coord);
        assert_eq!(surface.items[0].edit_revision, editor_commit.revision);
        assert!(surface.items[0].result.is_ok());

        editor_world.close(None).await?;
        observer_world.close(None).await?;
        editor_presence.close(None).await?;
        observer_presence.close(None).await?;
        server_task.abort();
        let _ = server_task.await;
        Ok(())
    }

    async fn connect_test_client(
        address: SocketAddr,
        identity: PlayerIdentity,
    ) -> Result<
        (
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
            WorldOpened,
        ),
        Box<dyn std::error::Error>,
    > {
        let (socket, opened_bytes) = connect_test_client_raw(address, identity).await?;
        let opened = decode_world_opened(&opened_bytes)?;
        Ok((socket, opened))
    }

    async fn connect_test_client_raw(
        address: SocketAddr,
        identity: PlayerIdentity,
    ) -> Result<
        (
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
            Vec<u8>,
        ),
        Box<dyn std::error::Error>,
    > {
        let mut request = format!("ws://{address}{WORLD_WEBSOCKET_PATH}").into_client_request()?;
        request
            .headers_mut()
            .insert(ORIGIN, HeaderValue::from_static("http://test.local"));
        request.headers_mut().insert(
            SEC_WEBSOCKET_PROTOCOL,
            HeaderValue::from_static("voxels.world.v24, test-local-token"),
        );
        let (mut socket, _) = connect_async(request).await?;
        socket
            .send(ClientMessage::Binary(
                encode_open_world(&OpenWorld {
                    max_in_flight_batches: 2,
                    identity,
                })?
                .into(),
            ))
            .await?;
        let response = next_client_binary(&mut socket).await?;
        Ok((socket, response))
    }

    async fn connect_test_presence(
        address: SocketAddr,
        session_id: PresenceSessionId,
    ) -> Result<
        (
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
            PresenceOpened,
        ),
        Box<dyn std::error::Error>,
    > {
        let mut request =
            format!("ws://{address}{PRESENCE_WEBSOCKET_PATH}").into_client_request()?;
        request
            .headers_mut()
            .insert(ORIGIN, HeaderValue::from_static("http://test.local"));
        request.headers_mut().insert(
            SEC_WEBSOCKET_PROTOCOL,
            HeaderValue::from_static("voxels.world.v24, test-local-token"),
        );
        let (mut socket, _) = connect_async(request).await?;
        socket
            .send(ClientMessage::Binary(
                encode_open_presence(OpenPresence { session_id })?.into(),
            ))
            .await?;
        let response = next_client_binary(&mut socket).await?;
        Ok((socket, decode_presence_opened(&response)?))
    }

    async fn next_presence_delta<S>(
        socket: &mut tokio_tungstenite::WebSocketStream<S>,
        enter_count: usize,
        leave_count: usize,
    ) -> Result<PresenceDelta, Box<dyn std::error::Error>>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let bytes = next_client_binary(socket).await?;
                if message_kind(&bytes)? == presence_delta_kind() {
                    let delta = decode_presence_delta(&bytes)?;
                    if delta.enters.len() == enter_count && delta.leaves.len() == leave_count {
                        return Ok(delta);
                    }
                }
            }
        })
        .await?
    }

    async fn wait_for_visible_connections<S>(
        socket: &mut tokio_tungstenite::WebSocketStream<S>,
        expected: &BTreeSet<u64>,
    ) -> Result<PresenceDelta, Box<dyn std::error::Error>>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            let mut visible = BTreeSet::new();
            loop {
                let bytes = next_client_binary(socket).await?;
                if message_kind(&bytes)? != presence_delta_kind() {
                    continue;
                }
                let delta = decode_presence_delta(&bytes)?;
                visible.extend(delta.enters.iter().map(|player| player.connection_id));
                for connection_id in &delta.leaves {
                    visible.remove(connection_id);
                }
                if &visible == expected {
                    return Ok(delta);
                }
            }
        })
        .await?
    }

    async fn next_edit_commit<S>(
        socket: &mut tokio_tungstenite::WebSocketStream<S>,
    ) -> Result<(EditCommit, usize), Box<dyn std::error::Error>>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let bytes = next_client_binary(socket).await?;
                if message_kind(&bytes)? == edit_commit_kind() {
                    let encoded_bytes = bytes.len();
                    return Ok((decode_edit_commit(&bytes)?, encoded_bytes));
                }
            }
        })
        .await?
    }

    async fn next_client_binary<S>(
        socket: &mut tokio_tungstenite::WebSocketStream<S>,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        loop {
            let message = socket
                .next()
                .await
                .ok_or("server closed before sending a binary frame")??;
            if let ClientMessage::Binary(bytes) = message {
                return Ok(bytes.to_vec());
            }
        }
    }
}
