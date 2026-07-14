//! Bounded binary-WebSocket transport for canonical world products.

use crate::{
    LoadedWorldServiceConfig, WorldServiceConfig, WorldServiceConfigError, WorldServiceSourceError,
    presence::{PoseAdmission, PresenceHub, PresenceStreamState},
};
use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::header::{ORIGIN, SEC_WEBSOCKET_PROTOCOL};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::serve::ListenerExt;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::fmt;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::net::TcpListener;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};
use voxels_world::protocol::{
    ChunkBatchItem, ChunkBatchRequest, ChunkBatchResult, PlayerIdentity, PresenceOpened,
    PresencePong, PresenceSessionId, SpawnPoint, SurfaceTileBatchItem, SurfaceTileBatchRequest,
    SurfaceTileBatchResult, WorldCapabilities, WorldOpened, cancel_kind, chunk_batch_kind,
    decode_cancel, decode_chunk_batch, decode_open_presence, decode_open_world, decode_player_pose,
    decode_presence_ping, decode_surface_tile_batch, encode_chunk_batch_result, encode_error,
    encode_presence_opened, encode_presence_pong, encode_surface_tile_batch_result,
    encode_world_opened, message_kind, open_presence_kind, open_world_kind, player_pose_kind,
    presence_ping_kind, surface_tile_batch_kind,
};
use voxels_world::{
    CHUNK_EDGE, ChunkCoord, Material, SurfaceSampleBlockRequest, WORLD_SCHEMA_VERSION,
    WorldManifest, WorldManifestError, WorldProduct, WorldProductBatch, WorldProductPriority,
    WorldProductRequest, WorldSourceEngine, WorldSourceError,
};

pub const WORLD_WEBSOCKET_PATH: &str = "/v5/world";
pub const PRESENCE_WEBSOCKET_PATH: &str = "/v5/presence";
pub const WORLD_WEBSOCKET_PROTOCOL: &str = "voxels.world.v5";

/// Prepared server state. Source construction and spawn coverage validation happen before bind.
pub struct WorldServer {
    router: Router,
}

impl WorldServer {
    pub fn from_loaded_config(loaded: &LoadedWorldServiceConfig) -> Result<Self, WorldServerError> {
        let source = loaded.build_world_source()?;
        Self::new(loaded.config().clone(), source)
    }

    pub fn new(
        config: WorldServiceConfig,
        source: Box<dyn WorldSourceEngine>,
    ) -> Result<Self, WorldServerError> {
        config.validate()?;
        let source = Arc::<dyn WorldSourceEngine>::from(source);
        let world = prepare_world(&config, source.as_ref())?;
        let capacity = usize::from(config.transport.global_queue_capacity);
        let (generation_tx, generation_rx) = mpsc::channel(capacity);
        let semaphore = Arc::new(Semaphore::new(usize::from(
            config.transport.generation_workers,
        )));
        tokio::spawn(run_generation_dispatcher(
            generation_rx,
            Arc::clone(&source),
            semaphore,
            config.transport.max_frame_bytes,
        ));

        let presence = PresenceHub::new(config.presence).map_err(WorldServerError::Presence)?;
        let state = Arc::new(ServerState {
            allowed_origins: config.transport.allowed_origins,
            auth_subprotocol_token: config.transport.auth_subprotocol_token,
            max_frame_bytes: config.transport.max_frame_bytes,
            max_outbound_bytes_per_client: config.transport.max_outbound_bytes_per_client,
            max_in_flight_batches: config.transport.max_in_flight_batches,
            generation_workers_per_client: config.transport.generation_workers_per_client,
            connections: Arc::new(Semaphore::new(usize::from(
                config.transport.max_connections,
            ))),
            presence_connections: Arc::new(Semaphore::new(usize::from(
                config.transport.max_connections,
            ))),
            world,
            presence,
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
    let spawn_request = SurfaceSampleBlockRequest {
        origin: config.spawn.xz_voxels,
        sample_shape: [1, 1],
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
        .samples()
        .first()
        .copied()
        .ok_or(WorldServerError::InvalidSpawnProduct)?;
    validate_spawn_chunk(config, source, sample.height, sample.water_level)?;
    let manifest = WorldManifest {
        world_id: config.canonical_world_id(),
        seed: config.world_seed,
        world_schema_version: WORLD_SCHEMA_VERSION,
        material_schema_version: Material::SCHEMA_VERSION,
        source: source.identity().clone(),
    };
    manifest.validate()?;
    Ok(WorldBootstrap {
        manifest,
        capabilities: WorldCapabilities::CANONICAL_CHUNKS
            .union(WorldCapabilities::SURFACE_LOD)
            .union(WorldCapabilities::PLAYER_PRESENCE),
        spawn: SpawnPoint {
            x: config.spawn.xz_voxels[0],
            z: config.spawn.xz_voxels[1],
            height: sample.height,
            water_level: sample.water_level,
            material: sample.material,
            region: sample.region,
            moisture: sample.moisture,
            temperature: sample.temperature,
            ridge: sample.ridge,
        },
    })
}

#[derive(Clone)]
struct WorldBootstrap {
    manifest: WorldManifest,
    capabilities: WorldCapabilities,
    spawn: SpawnPoint,
}

impl WorldBootstrap {
    fn opened(
        &self,
        identity: PlayerIdentity,
        recommended_in_flight_batches: u16,
        connection_id: u64,
        presence_session_id: PresenceSessionId,
    ) -> WorldOpened {
        WorldOpened {
            manifest: self.manifest.clone(),
            capabilities: self.capabilities,
            recommended_in_flight_batches,
            identity,
            connection_id,
            presence_session_id,
            spawn: self.spawn,
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
    max_outbound_bytes_per_client: usize,
    max_in_flight_batches: u16,
    generation_workers_per_client: u16,
    connections: Arc<Semaphore>,
    presence_connections: Arc<Semaphore>,
    world: WorldBootstrap,
    presence: Arc<PresenceHub>,
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
    outbound_bytes: Arc<Semaphore>,
}

impl SessionRequests {
    fn new(max_in_flight: u16, generation_workers: u16, max_outbound_bytes: usize) -> Self {
        Self {
            max_in_flight: usize::from(max_in_flight),
            closed: AtomicBool::new(false),
            in_flight: Mutex::new(HashMap::new()),
            generation_permits: Arc::new(Semaphore::new(usize::from(generation_workers))),
            outbound_bytes: Arc::new(Semaphore::new(max_outbound_bytes)),
        }
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
    tracked: Option<TrackedRequest>,
    _byte_permit: Option<OwnedSemaphorePermit>,
}

struct GenerationJob {
    request: GenerationRequest,
    outbound: mpsc::Sender<OutboundFrame>,
    tracked: TrackedRequest,
}

enum GenerationRequest {
    Chunks(ChunkBatchRequest),
    SurfaceTiles(SurfaceTileBatchRequest),
}

impl GenerationRequest {
    fn request_id(&self) -> u64 {
        match self {
            Self::Chunks(request) => request.request_id,
            Self::SurfaceTiles(request) => request.request_id,
        }
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
    let Some(player_claim) = state.presence.join(&open.identity) else {
        let _ = socket
            .send(Message::Binary(
                encode_error(0, "player is already connected").into(),
            ))
            .await;
        return;
    };
    let negotiated_window = open.max_in_flight_batches.min(state.max_in_flight_batches);
    let session = Arc::new(SessionRequests::new(
        negotiated_window,
        state.generation_workers_per_client,
        state.max_outbound_bytes_per_client,
    ));
    let (sink, stream) = socket.split();
    let outbound_capacity = usize::from(state.max_in_flight_batches).saturating_add(2);
    let (outbound, outbound_rx) = mpsc::channel(outbound_capacity);
    let (inbound_tx, mut inbound) = mpsc::channel(outbound_capacity);
    let writer_session = Arc::clone(&session);
    let writer = tokio::spawn(write_frames(sink, outbound_rx, writer_session));
    let reader = tokio::spawn(read_frames(stream, inbound_tx, state.max_frame_bytes));

    let opened = state.world.opened(
        open.identity,
        negotiated_window,
        player_claim.connection_id,
        player_claim.session_id,
    );
    if outbound
        .send(OutboundFrame {
            bytes: encode_world_opened(&opened),
            tracked: None,
            _byte_permit: None,
        })
        .await
        .is_err()
    {
        session.cancel_all();
        return;
    }

    loop {
        let bytes = match inbound.recv().await {
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
                request: GenerationRequest::Chunks(request),
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
                request: GenerationRequest::SurfaceTiles(request),
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
    if socket.send(Message::Binary(opened.into())).await.is_err() {
        return;
    }
    let mut stream = PresenceStreamState::default();
    let initial_delta = match state.presence.build_delta(&attachment, &mut stream) {
        Ok(Some(delta)) => delta,
        Ok(None) | Err(_) => return,
    };
    if socket
        .send(Message::Binary(initial_delta.into()))
        .await
        .is_err()
    {
        return;
    }
    let mut replication_tick = tokio::time::interval(tokio::time::Duration::from_millis(
        u64::from(config.broadcast_interval_ms),
    ));
    replication_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    replication_tick.tick().await;

    loop {
        tokio::select! {
            message = socket.recv() => {
                let bytes = match message {
                    Some(Ok(Message::Binary(bytes))) if bytes.len() <= state.max_frame_bytes => {
                        bytes.to_vec()
                    }
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                    Some(Ok(Message::Binary(_))) => {
                        let _ = socket.send(Message::Binary(
                            encode_error(0, "VXWP frame exceeds configured limit").into(),
                        )).await;
                        break;
                    }
                    Some(Ok(Message::Text(_))) => {
                        let _ = socket.send(Message::Binary(
                            encode_error(0, "VXWP accepts binary messages only").into(),
                        )).await;
                        break;
                    }
                };
                let kind = match message_kind(&bytes) {
                    Ok(kind) => kind,
                    Err(error) => {
                        let _ = socket.send(Message::Binary(
                            encode_error(0, &error.to_string()).into(),
                        )).await;
                        break;
                    }
                };
                if kind == player_pose_kind() {
                    let pose = match decode_player_pose(&bytes) {
                        Ok(pose) => pose,
                        Err(error) => {
                            let _ = socket.send(Message::Binary(
                                encode_error(0, &error.to_string()).into(),
                            )).await;
                            break;
                        }
                    };
                    match state.presence.accept_pose(&attachment, pose) {
                        PoseAdmission::Accepted
                        | PoseAdmission::IgnoredStale
                        | PoseAdmission::IgnoredRateLimit => {}
                        PoseAdmission::SessionClosed => break,
                        PoseAdmission::Invalid(message) => {
                            let _ = socket.send(Message::Binary(encode_error(0, message).into())).await;
                            break;
                        }
                    }
                } else if kind == presence_ping_kind() {
                    let ping = match decode_presence_ping(&bytes) {
                        Ok(ping) => ping,
                        Err(error) => {
                            let _ = socket.send(Message::Binary(
                                encode_error(0, &error.to_string()).into(),
                            )).await;
                            break;
                        }
                    };
                    let receive_time = state.presence.now_ms();
                    let pong = PresencePong {
                        sequence: ping.sequence,
                        client_send_time_ms: ping.client_send_time_ms,
                        server_receive_time_ms: receive_time,
                        server_send_time_ms: state.presence.now_ms().max(receive_time),
                    };
                    let Ok(pong) = encode_presence_pong(pong) else {
                        break;
                    };
                    if socket.send(Message::Binary(pong.into())).await.is_err() {
                        break;
                    }
                } else if kind == open_presence_kind() {
                    let _ = socket.send(Message::Binary(
                        encode_error(0, "presence session is already open").into(),
                    )).await;
                    break;
                } else {
                    let _ = socket.send(Message::Binary(
                        encode_error(0, "unexpected presence message kind").into(),
                    )).await;
                    break;
                }
            }
            _ = replication_tick.tick() => {
                match state.presence.build_delta(&attachment, &mut stream) {
                    Ok(Some(bytes)) => {
                        if socket.send(Message::Binary(bytes.into())).await.is_err() {
                            break;
                        }
                    }
                    Ok(None) => {}
                    Err(_) => break,
                }
            }
        }
    }
}

enum InboundFrame {
    Binary(Vec<u8>),
    Rejected(&'static str),
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
            tracked: None,
            _byte_permit: None,
        })
        .await
}

async fn write_frames(
    mut sink: SplitSink<WebSocket, Message>,
    mut outbound: mpsc::Receiver<OutboundFrame>,
    session: Arc<SessionRequests>,
) {
    while let Some(frame) = outbound.recv().await {
        let cancelled = frame
            .tracked
            .as_ref()
            .is_some_and(TrackedRequest::is_cancelled);
        let send_result = if cancelled {
            Ok(())
        } else {
            sink.send(Message::Binary(frame.bytes.into())).await
        };
        if let Some(tracked) = frame.tracked {
            tracked.finish();
        }
        if send_result.is_err() {
            break;
        }
    }
    session.cancel_all();
}

async fn run_generation_dispatcher(
    mut jobs: mpsc::Receiver<GenerationJob>,
    source: Arc<dyn WorldSourceEngine>,
    semaphore: Arc<Semaphore>,
    max_frame_bytes: usize,
) {
    while let Some(job) = jobs.recv().await {
        let source = Arc::clone(&source);
        let semaphore = Arc::clone(&semaphore);
        tokio::spawn(async move {
            let session_semaphore = Arc::clone(&job.tracked.session.generation_permits);
            let Ok(_session_permit) = session_semaphore.acquire_owned().await else {
                job.tracked.finish();
                return;
            };
            let Ok(permit) = semaphore.acquire_owned().await else {
                job.tracked.finish();
                return;
            };
            process_generation_job(job, source, permit, max_frame_bytes).await;
        });
    }
}

async fn process_generation_job(
    job: GenerationJob,
    source: Arc<dyn WorldSourceEngine>,
    _permit: OwnedSemaphorePermit,
    max_frame_bytes: usize,
) {
    if job.tracked.is_cancelled() {
        job.tracked.finish();
        return;
    }
    let request_id = job.request.request_id();
    let request = job.request;
    let generated = tokio::task::spawn_blocking(move || match request {
        GenerationRequest::Chunks(request) => generate_chunk_result(source.as_ref(), request),
        GenerationRequest::SurfaceTiles(request) => {
            generate_surface_tile_result(source.as_ref(), request)
        }
    })
    .await;
    if job.tracked.is_cancelled() {
        job.tracked.finish();
        return;
    }
    let bytes = match generated {
        Ok(Ok(bytes)) if bytes.len() <= max_frame_bytes => bytes,
        Ok(Ok(_)) => encode_error(request_id, "chunk result exceeds configured frame limit"),
        Ok(Err(message)) => encode_error(request_id, &message),
        Err(_) => encode_error(request_id, "world generation task failed"),
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
        tracked: Some(job.tracked),
        _byte_permit: Some(byte_permit),
    };
    if let Err(error) = job.outbound.send(frame).await
        && let Some(tracked) = error.0.tracked
    {
        tracked.finish();
    }
}

fn generate_surface_tile_result(
    source: &dyn WorldSourceEngine,
    request: SurfaceTileBatchRequest,
) -> Result<Vec<u8>, String> {
    let coords = request.coords.clone();
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
    let mut items = Vec::with_capacity(coords.len());
    for (coord, item) in coords.into_iter().zip(result.items) {
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
            result: item_result,
        });
    }
    encode_surface_tile_batch_result(&SurfaceTileBatchResult {
        request_id: request.request_id,
        source_identity_hash: result.source_identity_hash,
        items,
    })
    .map_err(|error| error.to_string())
}

fn generate_chunk_result(
    source: &dyn WorldSourceEngine,
    request: ChunkBatchRequest,
) -> Result<Vec<u8>, String> {
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
    for (coord, item) in coords.into_iter().zip(result.items) {
        if item.request != WorldProductRequest::ChunkWithHalo(coord) {
            return Err("world source returned a mismatched chunk key".to_owned());
        }
        let item_result = match item.result {
            Ok(WorldProduct::Chunk(chunk)) => Ok(chunk),
            Ok(_) => return Err("world source returned a non-chunk product".to_owned()),
            Err(error) => Err(error),
        };
        items.push(ChunkBatchItem {
            coord,
            result: item_result,
        });
    }
    encode_chunk_batch_result(&ChunkBatchResult {
        request_id: request.request_id,
        source_identity_hash: result.source_identity_hash,
        items,
    })
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        LoopbackTransportConfig, PresenceConfig, SpawnConfig, TerrainDiffusionProviderConfig,
        WORLD_SERVICE_CONFIG_SCHEMA_VERSION, WorldSourceMode,
    };
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::connect_async;
    use tokio_tungstenite::tungstenite::Error as ClientError;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::http::HeaderValue;
    use tokio_tungstenite::tungstenite::protocol::Message as ClientMessage;
    use uuid::Uuid;
    use voxels_world::protocol::{
        BrowserUserId, ChunkBatchRequest, OpenPresence, OpenWorld, PLAYER_POSE_GROUNDED, PlayerId,
        PlayerIdentity, PlayerPoseUpdate, PresenceDelta, PresenceOpened, SurfaceTileBatchRequest,
        decode_chunk_batch_result, decode_error, decode_presence_delta, decode_presence_opened,
        decode_surface_tile_batch_result, decode_world_opened, encode_chunk_batch,
        encode_open_presence, encode_open_world, encode_player_pose, encode_surface_tile_batch,
        presence_delta_kind,
    };
    use voxels_world::{
        ChunkCoord, ProceduralWorldSource, SurfaceLodLevel, SurfaceTileCoord, WorldProductPriority,
    };

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
                max_outbound_bytes_per_client: 32 * 1024 * 1024,
                max_in_flight_batches: 2,
                max_connections: 4,
                global_queue_capacity: 8,
                generation_workers: 4,
                generation_workers_per_client: 2,
            },
            presence: PresenceConfig {
                max_players: 4,
                ..PresenceConfig::default()
            },
            spawn: SpawnConfig {
                xz_voxels: [13, -21],
            },
            terrain_diffusion: TerrainDiffusionProviderConfig::default(),
        }
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
        let session = SessionRequests::new(1, 1, 1024);
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
            HeaderValue::from_static("voxels.world.v5, test-local-token"),
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

        let batch = ChunkBatchRequest {
            request_id: 9,
            priority: WorldProductPriority::VisibleChunk,
            coords: vec![ChunkCoord::new(0, 0, 0)],
        };
        socket
            .send(ClientMessage::Binary(encode_chunk_batch(&batch)?.into()))
            .await?;
        let result_bytes = next_client_binary(&mut socket).await?;
        let result = decode_chunk_batch_result(&result_bytes)?;
        assert_eq!(result.request_id, batch.request_id);
        assert_eq!(result.items.len(), 1);
        assert!(result.items[0].result.is_ok());

        socket.close(None).await?;
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

        for (socket, sequence, x) in [
            (&mut first_presence, 1_u64, 1.0_f32),
            (&mut second_presence, 1_u64, 3.0_f32),
        ] {
            socket
                .send(ClientMessage::Binary(
                    encode_player_pose(PlayerPoseUpdate {
                        sequence,
                        sample_server_time_ms: 0,
                        eye_position_metres: [x, 1.62, 2.0],
                        linear_velocity_metres_per_second: [1.0, 0.0, 0.0],
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
            HeaderValue::from_static("voxels.world.v5, test-local-token"),
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
            HeaderValue::from_static("voxels.world.v5, test-local-token"),
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
