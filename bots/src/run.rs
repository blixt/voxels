use crate::behavior::{
    BehaviorContext, BehaviorKind, BehaviorState, BotLayout, LeaderPose, ObservedAction,
};
use crate::cache::ChunkCache;
use crate::client::{BotSocket, ConnectedBot, connect_bot};
use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt, future::join_all};
use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::protocol::Message;
use voxels_core::{CameraState, InputState};
use voxels_world::protocol::{
    BrowserUserId, ChunkBatchRequest, EditAction, EditCommand, FrameReassembler, MaterialInventory,
    PLAYER_POSE_GROUNDED, PLAYER_POSE_SWIMMING, PlayerId, PlayerIdentity, PlayerPoseUpdate,
    PresencePing, SurfaceTileBatchRequest, chunk_batch_result_kind, decode_chunk_batch_result,
    decode_edit_commit, decode_error, decode_presence_delta, decode_presence_pong,
    decode_resync_required, decode_surface_tile_batch_result, edit_commit_kind, encode_chunk_batch,
    encode_edit_command, encode_player_pose, encode_presence_ping, encode_surface_tile_batch,
    error_kind, message_kind, presence_delta_kind, presence_pong_kind, resync_required_kind,
    surface_tile_batch_result_kind,
};
use voxels_world::{
    ChunkCoord, Material, SurfaceLodLevel, SurfaceTileCoord, VOXEL_SIZE_METRES, VoxelCoord,
    WorldProductPriority,
};

const REPORT_SCHEMA_VERSION: u32 = 3;
const SIMULATION_HZ: u64 = 60;
const POSE_HZ: u64 = 30;
const PING_INTERVAL: Duration = Duration::from_secs(1);
const SURFACE_REQUEST_INTERVAL: Duration = Duration::from_secs(2);
const CHUNK_CACHE_CAPACITY: usize = 96;
const MAX_TOWER_COLUMN_SCAN_VOXELS: i32 = 48;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BotRunConfig {
    pub world_url: String,
    pub presence_url: String,
    pub origin: String,
    pub subprotocol: String,
    #[serde(skip_serializing)]
    pub auth_token: String,
    pub bots: usize,
    pub duration_seconds: f64,
    pub seed: u64,
    pub layout: BotLayout,
}

impl BotRunConfig {
    fn validate(&self) -> Result<()> {
        if self.bots == 0 || self.bots > 1_024 {
            bail!("bot count must be in 1..=1024");
        }
        if !self.duration_seconds.is_finite()
            || self.duration_seconds < 1.0
            || self.duration_seconds > 86_400.0
        {
            bail!("duration must be finite and in 1..=86400 seconds");
        }
        if self.auth_token.is_empty() {
            bail!("auth token must not be empty");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrafficCounters {
    pub sent_payload_bytes: u64,
    pub received_payload_bytes: u64,
    pub sent_frames: u64,
    pub received_frames: u64,
    pub max_sent_frame_bytes: u64,
    pub max_received_frame_bytes: u64,
    pub sent_by_kind: BTreeMap<u16, MessageTraffic>,
    pub received_by_kind: BTreeMap<u16, MessageTraffic>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageTraffic {
    pub frames: u64,
    pub payload_bytes: u64,
}

impl TrafficCounters {
    pub fn sent(&mut self, bytes: &[u8]) -> Result<()> {
        Self::record(
            &mut self.sent_payload_bytes,
            &mut self.sent_frames,
            &mut self.max_sent_frame_bytes,
            &mut self.sent_by_kind,
            bytes,
        )
    }

    pub fn received(&mut self, bytes: &[u8]) -> Result<()> {
        Self::record(
            &mut self.received_payload_bytes,
            &mut self.received_frames,
            &mut self.max_received_frame_bytes,
            &mut self.received_by_kind,
            bytes,
        )
    }

    fn record(
        bytes_total: &mut u64,
        frames_total: &mut u64,
        max_frame_bytes: &mut u64,
        by_kind: &mut BTreeMap<u16, MessageTraffic>,
        bytes: &[u8],
    ) -> Result<()> {
        let kind = message_kind(bytes)?;
        *bytes_total = bytes_total.saturating_add(bytes.len() as u64);
        *frames_total = frames_total.saturating_add(1);
        *max_frame_bytes = (*max_frame_bytes).max(bytes.len() as u64);
        let entry = by_kind.entry(kind).or_default();
        entry.frames = entry.frames.saturating_add(1);
        entry.payload_bytes = entry.payload_bytes.saturating_add(bytes.len() as u64);
        Ok(())
    }

    fn merge(&mut self, other: &Self) {
        self.sent_payload_bytes = self
            .sent_payload_bytes
            .saturating_add(other.sent_payload_bytes);
        self.received_payload_bytes = self
            .received_payload_bytes
            .saturating_add(other.received_payload_bytes);
        self.sent_frames = self.sent_frames.saturating_add(other.sent_frames);
        self.received_frames = self.received_frames.saturating_add(other.received_frames);
        self.max_sent_frame_bytes = self.max_sent_frame_bytes.max(other.max_sent_frame_bytes);
        self.max_received_frame_bytes = self
            .max_received_frame_bytes
            .max(other.max_received_frame_bytes);
        merge_traffic_map(&mut self.sent_by_kind, &other.sent_by_kind);
        merge_traffic_map(&mut self.received_by_kind, &other.received_by_kind);
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LatencySummary {
    pub samples: usize,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BotReport {
    pub index: usize,
    pub name: String,
    pub behavior: BehaviorKind,
    pub connection_id: u64,
    pub handshake_ms: f64,
    pub traffic: TrafficCounters,
    pub poses_sent: u64,
    pub max_visible_players: u16,
    pub presence_enters: u64,
    pub presence_updates: u64,
    pub presence_leaves: u64,
    pub pings: u64,
    pub ping_latency: LatencySummary,
    pub final_outbound_rate_bytes_per_second: u32,
    pub max_outbound_rate_bytes_per_second: u32,
    pub chunk_batches_sent: u64,
    pub chunk_results: u64,
    pub unique_chunks_requested: usize,
    pub chunk_latency: LatencySummary,
    pub surface_batches_sent: u64,
    pub surface_results: u64,
    pub unique_surface_tiles_requested: usize,
    pub surface_latency: LatencySummary,
    pub edits_submitted: u64,
    pub edits_accepted: u64,
    pub edits_rejected: u64,
    pub edit_conflicts: u64,
    pub edits_observed: u64,
    pub no_op_edits: u64,
    pub mutations_committed: u64,
    pub copied_actions: u64,
    pub edit_latency: LatencySummary,
    pub resyncs: u64,
    pub protocol_errors: u64,
    pub error_samples: Vec<String>,
    pub max_distance_from_spawn_metres: f32,
    pub deepest_eye_metres: f32,
    pub final_inventory_total: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BotRunReport {
    pub schema_version: u32,
    pub config: BotRunConfig,
    pub wall_time_ms: f64,
    pub connection_count: usize,
    pub behaviors: BTreeMap<String, usize>,
    pub traffic: TrafficCounters,
    pub poses_sent: u64,
    pub unique_chunks_requested: usize,
    pub unique_surface_tiles_requested: usize,
    pub edits_submitted: u64,
    pub edits_accepted: u64,
    pub edits_rejected: u64,
    pub edit_conflicts: u64,
    pub mutations_committed: u64,
    pub copied_actions: u64,
    pub max_visible_players: u16,
    pub reports: Vec<BotReport>,
}

#[derive(Clone, Copy, Debug, Default)]
struct SharedBotState {
    action: Option<ObservedAction>,
}

struct PendingEdit {
    submitted: Instant,
    action: EditAction,
}

struct PendingChunkBatch {
    submitted: Instant,
    coords: Vec<ChunkCoord>,
}

struct PendingSurfaceTile {
    submitted: Instant,
    coord: SurfaceTileCoord,
}

#[derive(Default)]
struct ChunkRequests {
    requested: HashSet<ChunkCoord>,
    in_flight: HashSet<ChunkCoord>,
    pending: HashMap<u64, PendingChunkBatch>,
}

impl ChunkRequests {
    fn needs(&self, cache: &ChunkCache, coord: ChunkCoord) -> bool {
        !cache.contains(coord) && !self.in_flight.contains(&coord)
    }

    fn begin(&mut self, request_id: u64, coords: Vec<ChunkCoord>, submitted: Instant) {
        self.requested.extend(coords.iter().copied());
        self.in_flight.extend(coords.iter().copied());
        self.pending
            .insert(request_id, PendingChunkBatch { submitted, coords });
    }

    fn finish(&mut self, request_id: u64) -> Option<PendingChunkBatch> {
        let pending = self.pending.remove(&request_id)?;
        for coord in &pending.coords {
            self.in_flight.remove(coord);
        }
        Some(pending)
    }

    fn unique_count(&self) -> usize {
        self.requested.len()
    }
}

#[derive(Default)]
struct SurfaceRequests {
    requested: HashSet<SurfaceTileCoord>,
    completed: HashSet<SurfaceTileCoord>,
    in_flight: HashSet<SurfaceTileCoord>,
    pending: HashMap<u64, PendingSurfaceTile>,
}

impl SurfaceRequests {
    fn needs(&self, coord: SurfaceTileCoord) -> bool {
        !self.completed.contains(&coord) && !self.in_flight.contains(&coord)
    }

    fn begin(&mut self, request_id: u64, coord: SurfaceTileCoord, submitted: Instant) {
        self.requested.insert(coord);
        self.in_flight.insert(coord);
        self.pending
            .insert(request_id, PendingSurfaceTile { submitted, coord });
    }

    fn finish(&mut self, request_id: u64) -> Option<PendingSurfaceTile> {
        let pending = self.pending.remove(&request_id)?;
        self.in_flight.remove(&pending.coord);
        Some(pending)
    }

    fn complete(&mut self, coord: SurfaceTileCoord) {
        self.completed.insert(coord);
    }

    fn unique_count(&self) -> usize {
        self.requested.len()
    }
}

struct BotRuntime {
    index: usize,
    name: String,
    behavior: BehaviorState,
    leader_player_id: Option<PlayerId>,
    world: BotSocket,
    presence: BotSocket,
    connection_id: u64,
    edit_session_id: voxels_world::protocol::EditSessionId,
    camera: CameraState,
    inventory: MaterialInventory,
    traffic: TrafficCounters,
    cache: ChunkCache,
    frame_reassembler: FrameReassembler,
    chunk_requests: ChunkRequests,
    surface_requests: SurfaceRequests,
    pending_edits: HashMap<u64, PendingEdit>,
    pending_pings: HashMap<u32, Instant>,
    leader_connection_id: Option<u64>,
    leader_pose: Option<LeaderPose>,
    next_request_id: u64,
    pose_sequence: u64,
    ping_sequence: u32,
    latest_round_trip_ms: u32,
    latest_outbound_rate_bytes_per_second: u32,
    max_outbound_rate_bytes_per_second: u32,
    next_pose: Instant,
    next_ping: Instant,
    next_surface: Instant,
    start: Instant,
    end: Instant,
    spawn_position: Vec3,
    shared: Arc<RwLock<Vec<SharedBotState>>>,
    handshake_ms: f64,
    report: BotReportAccumulator,
}

#[derive(Default)]
struct BotReportAccumulator {
    poses_sent: u64,
    max_visible_players: u16,
    presence_enters: u64,
    presence_updates: u64,
    presence_leaves: u64,
    pings: u64,
    ping_latency_ms: Vec<f64>,
    chunk_batches_sent: u64,
    chunk_results: u64,
    chunk_latency_ms: Vec<f64>,
    surface_batches_sent: u64,
    surface_results: u64,
    surface_latency_ms: Vec<f64>,
    edits_submitted: u64,
    edits_accepted: u64,
    edits_rejected: u64,
    edit_conflicts: u64,
    edits_observed: u64,
    no_op_edits: u64,
    mutations_committed: u64,
    copied_actions: u64,
    edit_latency_ms: Vec<f64>,
    resyncs: u64,
    protocol_errors: u64,
    error_samples: Vec<String>,
    max_distance_from_spawn_metres: f32,
    deepest_eye_metres: f32,
}

pub async fn run_bots(config: BotRunConfig) -> Result<BotRunReport> {
    config.validate()?;
    let whole_start = Instant::now();
    let identities = (0..config.bots)
        .map(|index| identity_for(index, config.seed))
        .collect::<Vec<_>>();
    let connect_futures = identities.iter().cloned().map(|identity| {
        connect_bot(
            &config.world_url,
            &config.presence_url,
            &config.origin,
            &config.subprotocol,
            &config.auth_token,
            identity,
        )
    });
    let connection_results = join_all(connect_futures).await;
    let connections = connection_results
        .into_iter()
        .enumerate()
        .map(|(index, result)| result.with_context(|| format!("connect bot {index}")))
        .collect::<Result<Vec<_>>>()?;
    let shared = Arc::new(RwLock::new(vec![SharedBotState::default(); config.bots]));
    let duration = Duration::from_secs_f64(config.duration_seconds);
    let start = Instant::now();
    let end = start + duration;
    let tasks = connections
        .into_iter()
        .enumerate()
        .map(|(index, connection)| {
            let leader_player_id = follower_leader(index)
                .and_then(|leader| identities.get(leader))
                .map(|identity| identity.player_id);
            let runtime = BotRuntime::new(
                index,
                connection,
                BehaviorState::new(
                    BehaviorKind::for_index(index),
                    config.layout,
                    index,
                    config.seed,
                ),
                leader_player_id,
                start,
                end,
                Arc::clone(&shared),
            );
            tokio::spawn(runtime.run())
        })
        .collect::<Vec<JoinHandle<Result<BotReport>>>>();
    let reports = join_all(tasks)
        .await
        .into_iter()
        .enumerate()
        .map(|(index, joined)| {
            joined
                .with_context(|| format!("bot {index} task join"))?
                .with_context(|| format!("bot {index} runtime"))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(aggregate_report(config, whole_start.elapsed(), reports))
}

impl BotRuntime {
    #[allow(
        clippy::too_many_arguments,
        reason = "constructor pins one protocol session"
    )]
    fn new(
        index: usize,
        connection: ConnectedBot,
        behavior: BehaviorState,
        leader_player_id: Option<PlayerId>,
        start: Instant,
        end: Instant,
        shared: Arc<RwLock<Vec<SharedBotState>>>,
    ) -> Self {
        let resume = connection.opened.player_resume;
        let camera = CameraState::from_persisted(
            Vec3::from_array(resume.eye_position_metres),
            resume.look_yaw_radians,
            resume.look_pitch_radians,
        );
        let now = Instant::now();
        Self {
            index,
            name: connection.opened.identity.player_name.clone(),
            behavior,
            leader_player_id,
            world: connection.world,
            presence: connection.presence,
            connection_id: connection.opened.connection_id,
            edit_session_id: connection.opened.edit_session_id,
            camera,
            inventory: connection.opened.inventory,
            traffic: connection.traffic,
            cache: ChunkCache::new(CHUNK_CACHE_CAPACITY),
            frame_reassembler: FrameReassembler::default(),
            chunk_requests: ChunkRequests::default(),
            surface_requests: SurfaceRequests::default(),
            pending_edits: HashMap::new(),
            pending_pings: HashMap::new(),
            leader_connection_id: None,
            leader_pose: None,
            next_request_id: 1,
            pose_sequence: 0,
            ping_sequence: 0,
            latest_round_trip_ms: 0,
            latest_outbound_rate_bytes_per_second: 0,
            max_outbound_rate_bytes_per_second: 0,
            next_pose: now,
            next_ping: now + PING_INTERVAL,
            next_surface: now,
            start,
            end,
            spawn_position: camera.position,
            shared,
            handshake_ms: connection.handshake_ms,
            report: BotReportAccumulator {
                deepest_eye_metres: camera.position.y,
                ..BotReportAccumulator::default()
            },
        }
    }

    async fn run(mut self) -> Result<BotReport> {
        self.request_local_chunks().await?;
        self.send_pose().await?;
        let mut simulation =
            tokio::time::interval(Duration::from_secs_f64(1.0 / SIMULATION_HZ as f64));
        simulation.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        while Instant::now() < self.end {
            tokio::select! {
                _ = simulation.tick() => self.simulation_tick().await?,
                message = self.world.next() => self
                    .handle_world_message(message)
                    .await
                    .context("world stream")?,
                message = self.presence.next() => self
                    .handle_presence_message(message)
                    .await
                    .context("presence stream")?,
            }
        }
        self.send_pose().await?;
        let _ = self.presence.close(None).await;
        let _ = self.world.close(None).await;
        Ok(self.finish())
    }

    async fn simulation_tick(&mut self) -> Result<()> {
        let now = Instant::now();
        let leader_action = follower_leader(self.index).and_then(|leader| {
            self.shared
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(leader)
                .and_then(|state| state.action)
        });
        let intent = self.behavior.plan(BehaviorContext {
            elapsed_seconds: now.duration_since(self.start).as_secs_f32(),
            eye_position_metres: self.camera.position.to_array(),
            placeable_material: first_placeable(self.inventory),
            leader_pose: self.leader_pose,
            leader_action,
        });
        self.camera.yaw = intent.yaw_radians;
        self.camera.pitch = intent.pitch_radians;
        let mut input = InputState::default();
        input.set_key(1, intent.forward);
        input.set_key(6, intent.sprint);
        self.camera.update(
            &input,
            1.0 / SIMULATION_HZ as f32,
            VOXEL_SIZE_METRES,
            |x, y, z| self.cache.physics(VoxelCoord::new(x, y, z)),
        );
        let displacement = self.camera.position - self.spawn_position;
        self.report.max_distance_from_spawn_metres = self
            .report
            .max_distance_from_spawn_metres
            .max(displacement.length());
        self.report.deepest_eye_metres = self.report.deepest_eye_metres.min(self.camera.position.y);
        if now >= self.next_pose {
            self.next_pose = now + Duration::from_secs_f64(1.0 / POSE_HZ as f64);
            self.send_pose().await?;
        }
        if now >= self.next_ping {
            self.next_ping = now + PING_INTERVAL;
            self.send_ping().await?;
        }
        if now >= self.next_surface {
            self.next_surface = now + SURFACE_REQUEST_INTERVAL;
            self.request_surface().await?;
        }
        self.request_local_chunks().await?;
        if let Some(action) = intent.edit {
            if intent.copied_leader_action {
                self.report.copied_actions = self.report.copied_actions.saturating_add(1);
            }
            if let Some(action) = prepare_edit(&self.cache, action) {
                self.send_edit(action).await?;
            }
        }
        Ok(())
    }

    async fn send_pose(&mut self) -> Result<()> {
        self.pose_sequence = self.pose_sequence.saturating_add(1);
        let fluid = self.camera.fluid_state();
        let flags = if self.camera.grounded {
            PLAYER_POSE_GROUNDED
        } else if fluid.eyes_submerged {
            PLAYER_POSE_SWIMMING
        } else {
            0
        };
        let bytes = encode_player_pose(PlayerPoseUpdate {
            sequence: self.pose_sequence,
            sample_server_time_ms: 0,
            eye_position_metres: self.camera.position.to_array(),
            linear_velocity_metres_per_second: self.camera.velocity.to_array(),
            look_yaw_radians: self.camera.yaw,
            look_pitch_radians: self.camera.pitch,
            flags,
        })?;
        self.traffic.sent(&bytes)?;
        self.presence.send(Message::Binary(bytes.into())).await?;
        self.report.poses_sent = self.report.poses_sent.saturating_add(1);
        Ok(())
    }

    async fn send_ping(&mut self) -> Result<()> {
        self.ping_sequence = self.ping_sequence.saturating_add(1).max(1);
        let bytes = encode_presence_ping(PresencePing {
            sequence: self.ping_sequence,
            observed_round_trip_ms: self.latest_round_trip_ms,
            client_send_time_ms: elapsed_milliseconds(self.start),
        })?;
        self.traffic.sent(&bytes)?;
        self.presence.send(Message::Binary(bytes.into())).await?;
        self.pending_pings
            .insert(self.ping_sequence, Instant::now());
        self.report.pings = self.report.pings.saturating_add(1);
        Ok(())
    }

    async fn request_local_chunks(&mut self) -> Result<()> {
        let voxel = position_voxel(self.camera.position);
        let center = voxel.chunk();
        let mut coords = Vec::new();
        for y in (center.y - 1)..=(center.y + 1) {
            for z in (center.z - 1)..=(center.z + 1) {
                for x in (center.x - 1)..=(center.x + 1) {
                    let coord = ChunkCoord::new(x, y, z);
                    if coord.is_world_representable()
                        && self.chunk_requests.needs(&self.cache, coord)
                    {
                        coords.push(coord);
                    }
                }
            }
        }
        if coords.is_empty() {
            return Ok(());
        }
        let request_id = self.allocate_request_id();
        let request = ChunkBatchRequest {
            request_id,
            // This 3x3x3 neighborhood is the bot's collision and startup-ready set, matching the
            // browser scheduler rather than treating immediately walkable terrain as background.
            priority: WorldProductPriority::CollisionCritical,
            coords,
        };
        let bytes = encode_chunk_batch(&request)?;
        self.traffic.sent(&bytes)?;
        self.world.send(Message::Binary(bytes.into())).await?;
        self.chunk_requests
            .begin(request_id, request.coords, Instant::now());
        self.report.chunk_batches_sent = self.report.chunk_batches_sent.saturating_add(1);
        Ok(())
    }

    async fn request_surface(&mut self) -> Result<()> {
        let voxel = position_voxel(self.camera.position);
        let coord = SurfaceTileCoord::containing(SurfaceLodLevel::Stride16, voxel.x, voxel.z);
        if !self.surface_requests.needs(coord) {
            return Ok(());
        }
        let request_id = self.allocate_request_id();
        let bytes = encode_surface_tile_batch(&SurfaceTileBatchRequest {
            request_id,
            priority: WorldProductPriority::VisibleSurface,
            coords: vec![coord],
        })?;
        self.traffic.sent(&bytes)?;
        self.world.send(Message::Binary(bytes.into())).await?;
        self.surface_requests
            .begin(request_id, coord, Instant::now());
        self.report.surface_batches_sent = self.report.surface_batches_sent.saturating_add(1);
        Ok(())
    }

    async fn send_edit(&mut self, action: EditAction) -> Result<()> {
        if !self.pending_edits.is_empty() {
            return Ok(());
        }
        let operation_id = self.allocate_request_id();
        let bytes = encode_edit_command(EditCommand {
            operation_id,
            edit_session_id: self.edit_session_id,
            action,
        })?;
        self.traffic.sent(&bytes)?;
        self.world.send(Message::Binary(bytes.into())).await?;
        self.pending_edits.insert(
            operation_id,
            PendingEdit {
                submitted: Instant::now(),
                action,
            },
        );
        self.report.edits_submitted = self.report.edits_submitted.saturating_add(1);
        Ok(())
    }

    async fn handle_world_message(
        &mut self,
        message: Option<Result<Message, tokio_tungstenite::tungstenite::Error>>,
    ) -> Result<()> {
        let Some(message) = message else {
            bail!("world socket closed before the run completed");
        };
        match message? {
            Message::Binary(bytes) => self.handle_world_binary(&bytes).await,
            Message::Close(frame) => bail!("world socket closed early: {frame:?}"),
            _ => Ok(()),
        }
    }

    async fn handle_world_binary(&mut self, bytes: &[u8]) -> Result<()> {
        self.traffic.received(bytes)?;
        let kind = message_kind(bytes)?;
        if kind == voxels_world::protocol::frame_fragment_kind() {
            if let Some(completed) = self.frame_reassembler.accept(bytes)? {
                self.handle_world_payload(&completed).await?;
            }
            return Ok(());
        }
        self.handle_world_payload(bytes).await
    }

    async fn handle_world_payload(&mut self, bytes: &[u8]) -> Result<()> {
        let kind = message_kind(bytes)?;
        if kind == chunk_batch_result_kind() {
            let result = decode_chunk_batch_result(bytes)?;
            if let Some(pending) = self.chunk_requests.finish(result.request_id) {
                self.report
                    .chunk_latency_ms
                    .push(pending.submitted.elapsed().as_secs_f64() * 1_000.0);
            }
            for item in result.items {
                self.report.chunk_results = self.report.chunk_results.saturating_add(1);
                match item.result {
                    Ok(snapshot) => self.cache.insert(snapshot.chunk),
                    Err(error) => self.record_error(format!("chunk {:?}: {error}", item.coord)),
                }
            }
        } else if kind == surface_tile_batch_result_kind() {
            let result = decode_surface_tile_batch_result(bytes)?;
            if let Some(pending) = self.surface_requests.finish(result.request_id) {
                self.report
                    .surface_latency_ms
                    .push(pending.submitted.elapsed().as_secs_f64() * 1_000.0);
            }
            for item in result.items {
                self.report.surface_results = self.report.surface_results.saturating_add(1);
                match item.result {
                    Ok(_) => self.surface_requests.complete(item.coord),
                    Err(error) => self.record_error(format!("surface {:?}: {error}", item.coord)),
                }
            }
        } else if kind == edit_commit_kind() {
            let commit = decode_edit_commit(bytes)?;
            for mutation in &commit.mutations {
                self.cache.apply(mutation.coord, mutation.material);
            }
            self.report.edits_observed = self.report.edits_observed.saturating_add(1);
            if commit.editor_connection_id == self.connection_id {
                self.report.edits_accepted = self.report.edits_accepted.saturating_add(1);
                self.report.mutations_committed = self
                    .report
                    .mutations_committed
                    .saturating_add(commit.mutations.len() as u64);
                if commit.mutations.is_empty() {
                    self.report.no_op_edits = self.report.no_op_edits.saturating_add(1);
                }
                if let Some(inventory) = commit.editor_inventory {
                    self.inventory = inventory;
                }
                if let Some(pending) = self.pending_edits.remove(&commit.operation_id) {
                    self.report
                        .edit_latency_ms
                        .push(pending.submitted.elapsed().as_secs_f64() * 1_000.0);
                    let mut shared = self
                        .shared
                        .write()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    if let Some(state) = shared.get_mut(self.index) {
                        state.action = Some(ObservedAction {
                            serial: commit.operation_id,
                            action: pending.action,
                        });
                    }
                }
            }
        } else if kind == error_kind() {
            let (request_id, message) = decode_error(bytes)?;
            let mut expected_edit_conflict = false;
            if let Some(pending) = self.pending_edits.remove(&request_id) {
                self.report.edits_rejected = self.report.edits_rejected.saturating_add(1);
                if message == "placement target is occupied" {
                    self.report.edit_conflicts = self.report.edit_conflicts.saturating_add(1);
                    expected_edit_conflict = true;
                }
                self.report
                    .edit_latency_ms
                    .push(pending.submitted.elapsed().as_secs_f64() * 1_000.0);
            }
            self.chunk_requests.finish(request_id);
            self.surface_requests.finish(request_id);
            if !expected_edit_conflict {
                self.report.protocol_errors = self.report.protocol_errors.saturating_add(1);
                self.record_error(format!("request {request_id}: {message}"));
            }
        } else if kind == resync_required_kind() {
            let _ = decode_resync_required(bytes)?;
            self.report.resyncs = self.report.resyncs.saturating_add(1);
        } else {
            self.record_error(format!("unexpected world message kind {kind}"));
        }
        Ok(())
    }

    async fn handle_presence_message(
        &mut self,
        message: Option<Result<Message, tokio_tungstenite::tungstenite::Error>>,
    ) -> Result<()> {
        let Some(message) = message else {
            bail!("presence socket closed before the run completed");
        };
        match message? {
            Message::Binary(bytes) => self.handle_presence_binary(&bytes),
            Message::Close(frame) => bail!(
                "presence socket closed early: {frame:?}; recent errors: {:?}",
                self.report.error_samples
            ),
            _ => Ok(()),
        }
    }

    fn handle_presence_binary(&mut self, bytes: &[u8]) -> Result<()> {
        self.traffic.received(bytes)?;
        let kind = message_kind(bytes)?;
        if kind == presence_delta_kind() {
            let delta = decode_presence_delta(bytes)?;
            self.report.max_visible_players = self
                .report
                .max_visible_players
                .max(delta.visible_player_count);
            self.report.presence_enters = self
                .report
                .presence_enters
                .saturating_add(delta.enters.len() as u64);
            self.report.presence_updates = self
                .report
                .presence_updates
                .saturating_add(delta.updates.len() as u64);
            self.report.presence_leaves = self
                .report
                .presence_leaves
                .saturating_add(delta.leaves.len() as u64);
            for entry in delta.enters {
                if Some(entry.player_id) == self.leader_player_id {
                    self.leader_connection_id = Some(entry.connection_id);
                    self.leader_pose = Some(LeaderPose {
                        eye_position_metres: entry.pose.eye_position_metres,
                        yaw_radians: entry.pose.look_yaw_radians,
                    });
                }
            }
            for update in delta.updates {
                if Some(update.connection_id) == self.leader_connection_id {
                    self.leader_pose = Some(LeaderPose {
                        eye_position_metres: update.pose.eye_position_metres,
                        yaw_radians: update.pose.look_yaw_radians,
                    });
                }
            }
            if delta
                .leaves
                .iter()
                .any(|connection| Some(*connection) == self.leader_connection_id)
            {
                self.leader_connection_id = None;
                self.leader_pose = None;
            }
        } else if kind == presence_pong_kind() {
            let pong = decode_presence_pong(bytes)?;
            self.latest_outbound_rate_bytes_per_second = pong.outbound_rate_bytes_per_second;
            self.max_outbound_rate_bytes_per_second = self
                .max_outbound_rate_bytes_per_second
                .max(pong.outbound_rate_bytes_per_second);
            if let Some(started) = self.pending_pings.remove(&pong.sequence) {
                let server_processing_ms =
                    pong.server_send_time_ms
                        .saturating_sub(pong.server_receive_time_ms) as f64;
                let round_trip_ms =
                    (started.elapsed().as_secs_f64() * 1_000.0 - server_processing_ms).max(0.0);
                self.latest_round_trip_ms =
                    round_trip_ms.round().clamp(1.0, u32::MAX as f64) as u32;
                self.report.ping_latency_ms.push(round_trip_ms);
            }
        } else if kind == error_kind() {
            let (_, message) = decode_error(bytes)?;
            self.report.protocol_errors = self.report.protocol_errors.saturating_add(1);
            self.record_error(format!("presence: {message}"));
        } else {
            self.record_error(format!("unexpected presence message kind {kind}"));
        }
        Ok(())
    }

    fn allocate_request_id(&mut self) -> u64 {
        let allocated = self.next_request_id;
        self.next_request_id = self.next_request_id.saturating_add(1).max(1);
        allocated
    }

    fn record_error(&mut self, message: String) {
        if self.report.error_samples.len() < 16 {
            self.report.error_samples.push(message);
        }
    }

    fn finish(self) -> BotReport {
        BotReport {
            index: self.index,
            name: self.name,
            behavior: self.behavior.kind(),
            connection_id: self.connection_id,
            handshake_ms: self.handshake_ms,
            traffic: self.traffic,
            poses_sent: self.report.poses_sent,
            max_visible_players: self.report.max_visible_players,
            presence_enters: self.report.presence_enters,
            presence_updates: self.report.presence_updates,
            presence_leaves: self.report.presence_leaves,
            pings: self.report.pings,
            ping_latency: summarize_latencies(&self.report.ping_latency_ms),
            final_outbound_rate_bytes_per_second: self.latest_outbound_rate_bytes_per_second,
            max_outbound_rate_bytes_per_second: self.max_outbound_rate_bytes_per_second,
            chunk_batches_sent: self.report.chunk_batches_sent,
            chunk_results: self.report.chunk_results,
            unique_chunks_requested: self.chunk_requests.unique_count(),
            chunk_latency: summarize_latencies(&self.report.chunk_latency_ms),
            surface_batches_sent: self.report.surface_batches_sent,
            surface_results: self.report.surface_results,
            unique_surface_tiles_requested: self.surface_requests.unique_count(),
            surface_latency: summarize_latencies(&self.report.surface_latency_ms),
            edits_submitted: self.report.edits_submitted,
            edits_accepted: self.report.edits_accepted,
            edits_rejected: self.report.edits_rejected,
            edit_conflicts: self.report.edit_conflicts,
            edits_observed: self.report.edits_observed,
            no_op_edits: self.report.no_op_edits,
            mutations_committed: self.report.mutations_committed,
            copied_actions: self.report.copied_actions,
            edit_latency: summarize_latencies(&self.report.edit_latency_ms),
            resyncs: self.report.resyncs,
            protocol_errors: self.report.protocol_errors,
            error_samples: self.report.error_samples,
            max_distance_from_spawn_metres: self.report.max_distance_from_spawn_metres,
            deepest_eye_metres: self.report.deepest_eye_metres,
            final_inventory_total: self
                .inventory
                .counts
                .into_iter()
                .fold(0_u64, u64::saturating_add),
        }
    }
}

fn identity_for(index: usize, seed: u64) -> PlayerIdentity {
    let kind = BehaviorKind::for_index(index);
    let name = format!("bot-{}-{index:03}", behavior_name(kind));
    PlayerIdentity {
        browser_user_id: BrowserUserId::from_bytes(stable_id(seed, index, 1)),
        player_id: PlayerId::from_bytes(stable_id(seed, index, 2)),
        player_name: name,
    }
}

fn stable_id(seed: u64, index: usize, domain: u64) -> [u8; 16] {
    let first = splitmix64(seed ^ domain.rotate_left(17) ^ index as u64);
    let second = splitmix64(first ^ 0xa5a5_5a5a_d3c3_b4b4);
    let mut bytes = [0; 16];
    bytes[..8].copy_from_slice(&first.to_le_bytes());
    bytes[8..].copy_from_slice(&second.to_le_bytes());
    if bytes == [0; 16] {
        bytes[15] = 1;
    }
    bytes
}

fn follower_leader(index: usize) -> Option<usize> {
    (BehaviorKind::for_index(index) == BehaviorKind::Follower).then_some(index.saturating_sub(1))
}

fn first_placeable(inventory: MaterialInventory) -> Option<Material> {
    Material::ALL.into_iter().find(|material| {
        !matches!(material, Material::Air | Material::Water) && inventory.count(*material) > 0
    })
}

fn prepare_edit(cache: &ChunkCache, action: EditAction) -> Option<EditAction> {
    let EditAction::Place {
        mut coord,
        material,
    } = action
    else {
        return Some(action);
    };
    for _ in 0..=MAX_TOWER_COLUMN_SCAN_VOXELS {
        match cache.material(coord) {
            Some(Material::Air) => return Some(EditAction::Place { coord, material }),
            Some(_) => coord.y = coord.y.saturating_add(1),
            None => return None,
        }
    }
    None
}

fn position_voxel(position: Vec3) -> VoxelCoord {
    VoxelCoord::new(
        (position.x / VOXEL_SIZE_METRES).floor() as i32,
        (position.y / VOXEL_SIZE_METRES).floor() as i32,
        (position.z / VOXEL_SIZE_METRES).floor() as i32,
    )
}

fn elapsed_milliseconds(start: Instant) -> u64 {
    let value = start.elapsed().as_millis();
    u64::try_from(value).unwrap_or(u64::MAX).max(1)
}

fn summarize_latencies(values: &[f64]) -> LatencySummary {
    if values.is_empty() {
        return LatencySummary::default();
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    LatencySummary {
        samples: sorted.len(),
        p50_ms: percentile(&sorted, 0.50),
        p95_ms: percentile(&sorted, 0.95),
        p99_ms: percentile(&sorted, 0.99),
        max_ms: sorted.last().copied().unwrap_or_default(),
    }
}

fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    let index = ((sorted.len() as f64 * fraction).ceil() as usize)
        .saturating_sub(1)
        .min(sorted.len().saturating_sub(1));
    sorted.get(index).copied().unwrap_or_default()
}

fn aggregate_report(
    config: BotRunConfig,
    wall_time: Duration,
    reports: Vec<BotReport>,
) -> BotRunReport {
    let mut traffic = TrafficCounters::default();
    let mut behaviors = BTreeMap::new();
    let mut poses_sent = 0_u64;
    let mut unique_chunks_requested = 0_usize;
    let mut unique_surface_tiles_requested = 0_usize;
    let mut edits_submitted = 0_u64;
    let mut edits_accepted = 0_u64;
    let mut edits_rejected = 0_u64;
    let mut edit_conflicts = 0_u64;
    let mut mutations_committed = 0_u64;
    let mut copied_actions = 0_u64;
    let mut max_visible_players = 0_u16;
    for report in &reports {
        traffic.merge(&report.traffic);
        *behaviors
            .entry(behavior_name(report.behavior).to_owned())
            .or_default() += 1;
        poses_sent = poses_sent.saturating_add(report.poses_sent);
        unique_chunks_requested =
            unique_chunks_requested.saturating_add(report.unique_chunks_requested);
        unique_surface_tiles_requested =
            unique_surface_tiles_requested.saturating_add(report.unique_surface_tiles_requested);
        edits_submitted = edits_submitted.saturating_add(report.edits_submitted);
        edits_accepted = edits_accepted.saturating_add(report.edits_accepted);
        edits_rejected = edits_rejected.saturating_add(report.edits_rejected);
        edit_conflicts = edit_conflicts.saturating_add(report.edit_conflicts);
        mutations_committed = mutations_committed.saturating_add(report.mutations_committed);
        copied_actions = copied_actions.saturating_add(report.copied_actions);
        max_visible_players = max_visible_players.max(report.max_visible_players);
    }
    BotRunReport {
        schema_version: REPORT_SCHEMA_VERSION,
        connection_count: reports.len(),
        config,
        wall_time_ms: wall_time.as_secs_f64() * 1_000.0,
        behaviors,
        traffic,
        poses_sent,
        unique_chunks_requested,
        unique_surface_tiles_requested,
        edits_submitted,
        edits_accepted,
        edits_rejected,
        edit_conflicts,
        mutations_committed,
        copied_actions,
        max_visible_players,
        reports,
    }
}

fn merge_traffic_map(
    target: &mut BTreeMap<u16, MessageTraffic>,
    source: &BTreeMap<u16, MessageTraffic>,
) {
    for (kind, traffic) in source {
        let entry = target.entry(*kind).or_default();
        entry.frames = entry.frames.saturating_add(traffic.frames);
        entry.payload_bytes = entry.payload_bytes.saturating_add(traffic.payload_bytes);
    }
}

const fn behavior_name(kind: BehaviorKind) -> &'static str {
    match kind {
        BehaviorKind::Explorer => "explorer",
        BehaviorKind::Digger => "digger",
        BehaviorKind::Builder => "builder",
        BehaviorKind::Follower => "follower",
    }
}

const fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;
    use voxels_world::{CHUNK_EDGE, Chunk};

    #[test]
    fn stable_roster_has_valid_unique_ids_and_expected_leaders() {
        let identities = (0..64)
            .map(|index| identity_for(index, 123))
            .collect::<Vec<_>>();
        let players = identities
            .iter()
            .map(|identity| identity.player_id)
            .collect::<HashSet<_>>();
        assert_eq!(players.len(), identities.len());
        assert!(
            identities
                .iter()
                .all(|identity| identity.validate().is_ok())
        );
        assert_eq!(follower_leader(3), Some(2));
        assert_eq!(follower_leader(7), Some(6));
        assert_eq!(follower_leader(2), None);
    }

    #[test]
    fn latency_summary_uses_nearest_rank_percentiles() {
        let values = (1..=100).map(f64::from).collect::<Vec<_>>();
        let summary = summarize_latencies(&values);
        assert_eq!(summary.p50_ms, 50.0);
        assert_eq!(summary.p95_ms, 95.0);
        assert_eq!(summary.p99_ms, 99.0);
        assert_eq!(summary.max_ms, 100.0);
    }

    #[test]
    fn traffic_merge_keeps_message_attribution() -> Result<()> {
        let bytes =
            voxels_world::protocol::encode_open_presence(voxels_world::protocol::OpenPresence {
                session_id: voxels_world::protocol::PresenceSessionId::from_bytes([4; 16]),
            })?;
        let mut first = TrafficCounters::default();
        first.sent(&bytes)?;
        let mut second = TrafficCounters::default();
        second.sent(&bytes)?;
        first.merge(&second);
        assert_eq!(first.sent_frames, 2);
        assert_eq!(first.max_sent_frame_bytes, bytes.len() as u64);
        assert_eq!(
            first
                .sent_by_kind
                .get(&voxels_world::protocol::open_presence_kind())
                .map(|traffic| traffic.frames),
            Some(2)
        );
        Ok(())
    }

    #[test]
    fn placement_advances_to_the_first_authoritative_empty_voxel() {
        let mut cache = ChunkCache::new(1);
        let mut chunk = Chunk::filled(ChunkCoord::new(0, 0, 0), Material::Air);
        for y in 0..4 {
            chunk.set(2, y, 3, Material::Stone);
        }
        cache.insert(chunk);
        let prepared = prepare_edit(
            &cache,
            EditAction::Place {
                coord: VoxelCoord::new(2, 0, 3),
                material: Material::Dirt,
            },
        );
        assert_eq!(
            prepared,
            Some(EditAction::Place {
                coord: VoxelCoord::new(2, 4, 3),
                material: Material::Dirt,
            })
        );

        let missing = VoxelCoord::new(CHUNK_EDGE as i32, 0, 0);
        assert_eq!(
            prepare_edit(
                &cache,
                EditAction::Place {
                    coord: missing,
                    material: Material::Dirt,
                }
            ),
            None
        );
    }

    #[test]
    fn chunk_requests_retry_failures_and_cache_evictions() {
        let first = ChunkCoord::new(0, 0, 0);
        let second = ChunkCoord::new(1, 0, 0);
        let mut cache = ChunkCache::new(1);
        let mut requests = ChunkRequests::default();

        assert!(requests.needs(&cache, first));
        requests.begin(1, vec![first], Instant::now());
        assert!(!requests.needs(&cache, first));

        let failed = requests.finish(1).expect("pending request");
        assert_eq!(failed.coords, vec![first]);
        assert!(requests.needs(&cache, first));

        requests.begin(2, vec![first], Instant::now());
        requests.finish(2);
        cache.insert(Chunk::filled(first, Material::Dirt));
        assert!(!requests.needs(&cache, first));

        cache.insert(Chunk::filled(second, Material::Stone));
        assert!(requests.needs(&cache, first));
        assert_eq!(requests.unique_count(), 1);
    }

    #[test]
    fn surface_requests_retry_failures_but_not_successes() {
        let coord = SurfaceTileCoord::new(SurfaceLodLevel::Stride16, 2, -3);
        let mut requests = SurfaceRequests::default();

        assert!(requests.needs(coord));
        requests.begin(1, coord, Instant::now());
        assert!(!requests.needs(coord));

        let failed = requests.finish(1).expect("pending request");
        assert_eq!(failed.coord, coord);
        assert!(requests.needs(coord));

        requests.begin(2, coord, Instant::now());
        requests.finish(2);
        requests.complete(coord);
        assert!(!requests.needs(coord));
        assert_eq!(requests.unique_count(), 1);
    }
}
