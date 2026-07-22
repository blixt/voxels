//! Server-authoritative player-presence registry and spatial replication scheduler.
//!
//! World-product WebSockets own player sessions. A second, small-frame presence WebSocket binds
//! with the unguessable session token returned by `WorldOpened`. Authoritative records live in one
//! world, while a spatial hash and receiver-local stream state make replication proportional to
//! nearby relationships instead of the square of the global player count.

use crate::{GameplayConfig, PresenceConfig};
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;
use uuid::Uuid;
use voxels_core::player_intersects_voxel;
use voxels_world::protocol::{
    BrowserUserId, EditVolume, PLAYER_POSE_DISCONTINUITY, PLAYER_POSE_GLIDING,
    PLAYER_POSE_GROUNDED, PLAYER_POSE_SPECTATOR, PLAYER_POSE_SWIMMING, PlayerId, PlayerIdentity,
    PlayerPoseUpdate, PlayerPresenceState, PlayerPresenceUpdate, PlayerResume, PresenceDelta,
    PresenceSessionId, encode_presence_delta,
};
use voxels_world::{VOXEL_SIZE_METRES, VoxelCoord};

const MAX_FUTURE_SAMPLE_MS: u64 = 250;
const MAX_STALE_SAMPLE_MS: u64 = 2_000;

pub(crate) struct PresenceHub {
    started: Instant,
    config: PresenceConfig,
    gameplay: GameplayConfig,
    inner: Mutex<PresenceInner>,
}

struct PresenceInner {
    next_connection_id: u64,
    players: HashMap<PlayerId, PlayerSession>,
    sessions: HashMap<PresenceSessionId, PlayerId>,
    connections: HashMap<u64, PlayerId>,
    cells: HashMap<SpatialCell, BTreeSet<PlayerId>>,
}

struct PlayerSession {
    browser_user_id: BrowserUserId,
    connection_id: u64,
    session_id: PresenceSessionId,
    color_index: u16,
    presence_attached: bool,
    pose: Option<PlayerPoseUpdate>,
    body_pose: PlayerPoseUpdate,
    resume_revision: u64,
    last_pose_receipt_ms: u64,
    last_discontinuity_sequence: u64,
    discontinuity_on_next_accept: bool,
    horizontal_movement_credit_metres: f32,
    vertical_movement_credit_metres: f32,
    movement_credit_updated_ms: u64,
    cell: Option<SpatialCell>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct SpatialCell {
    x: i32,
    z: i32,
}

#[derive(Clone, Copy)]
struct Candidate {
    state: PlayerPresenceState,
    distance_squared: f32,
    last_discontinuity_sequence: u64,
}

#[derive(Clone, Copy)]
struct SentState {
    connection_id: u64,
    pose: PlayerPoseUpdate,
    sent_at_ms: u64,
}

#[derive(Default)]
pub(crate) struct PresenceStreamState {
    stream_sequence: u64,
    sent_initial: bool,
    known: HashMap<PlayerId, SentState>,
}

#[derive(Clone, Copy)]
struct PendingRecord {
    candidate: Candidate,
    enter: bool,
    discontinuity: bool,
    prediction_error: bool,
    overdue_millis: u64,
}

pub(crate) struct WorldPresenceClaim {
    hub: Arc<PresenceHub>,
    pub(crate) connection_id: u64,
    pub(crate) session_id: PresenceSessionId,
    player_id: PlayerId,
}

pub(crate) struct PresenceAttachment {
    hub: Arc<PresenceHub>,
    pub(crate) connection_id: u64,
    session_id: PresenceSessionId,
    player_id: PlayerId,
    browser_user_id: BrowserUserId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PoseAdmission {
    Accepted,
    IgnoredStale,
    IgnoredRateLimit,
    SessionClosed,
    Invalid(&'static str),
}

impl PresenceHub {
    pub(crate) fn new(
        config: PresenceConfig,
        gameplay: GameplayConfig,
    ) -> Result<Arc<Self>, String> {
        Ok(Arc::new(Self {
            started: Instant::now(),
            config,
            gameplay,
            inner: Mutex::new(PresenceInner {
                next_connection_id: 1,
                players: HashMap::new(),
                sessions: HashMap::new(),
                connections: HashMap::new(),
                cells: HashMap::new(),
            }),
        }))
    }

    fn lock(&self) -> MutexGuard<'_, PresenceInner> {
        match self.inner.lock() {
            Ok(inner) => inner,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    pub(crate) fn now_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis())
            .unwrap_or(u64::MAX)
            .max(1)
    }

    pub(crate) const fn config(&self) -> PresenceConfig {
        self.config
    }

    /// Authorizes one canonical-voxel interaction against the same connection's latest accepted
    /// pose. The fixed slack is a hard allowance for ordering skew between the world and presence
    /// sockets; it does not grow with client-reported velocity or pose age.
    pub(crate) fn authorize_interaction(
        &self,
        connection_id: u64,
        target: VoxelCoord,
    ) -> Result<(), &'static str> {
        let now = self.now_ms();
        let inner = self.lock();
        let player_id = inner
            .connections
            .get(&connection_id)
            .ok_or("player connection is not active")?;
        let player = inner
            .players
            .get(player_id)
            .ok_or("player connection is not active")?;
        if player.connection_id != connection_id || !player.presence_attached {
            return Err("player presence is not attached");
        }
        let pose = player.pose.ok_or("player pose is unavailable")?;
        if is_spectator(pose) {
            return Err("spectators cannot edit the world");
        }
        if player.last_pose_receipt_ms == 0
            || now.saturating_sub(player.last_pose_receipt_ms)
                > u64::from(self.gameplay.interaction_pose_max_age_ms)
        {
            return Err("player pose is stale");
        }

        let reach_metres = f64::from(
            self.gameplay
                .interaction_reach_centimetres
                .saturating_add(self.gameplay.interaction_latency_slack_centimetres),
        ) * 0.01;
        let voxel_size = f64::from(VOXEL_SIZE_METRES);
        let mut distance_squared = 0.0_f64;
        for (axis, coordinate) in [target.x, target.y, target.z].into_iter().enumerate() {
            let minimum = f64::from(coordinate) * voxel_size;
            let maximum = minimum + voxel_size;
            let eye = f64::from(pose.eye_position_metres[axis]);
            let nearest = eye.clamp(minimum, maximum);
            let delta = eye - nearest;
            distance_squared += delta * delta;
        }
        if distance_squared > reach_metres * reach_metres {
            return Err("interaction target is out of reach");
        }
        Ok(())
    }

    pub(crate) fn authorize_placement_volume(
        &self,
        connection_id: u64,
        volume: EditVolume,
    ) -> Result<(), &'static str> {
        let inner = self.lock();
        let player_id = inner
            .connections
            .get(&connection_id)
            .ok_or("player connection is not active")?;
        let player = inner
            .players
            .get(player_id)
            .ok_or("player connection is not active")?;
        if player.connection_id != connection_id {
            return Err("player connection is not active");
        }
        let pose = player.pose.ok_or("player pose is unavailable")?;
        if volume.coordinates().any(|coord| {
            player_intersects_voxel(
                pose.eye_position_metres,
                [coord.x, coord.y, coord.z],
                VOXEL_SIZE_METRES,
            )
        }) {
            return Err("placement volume intersects the player");
        }
        Ok(())
    }

    /// Returns the latest server-accepted camera state even while the presence socket is detached.
    /// The world-session owner uses this value when durably closing or checkpointing the player.
    pub(crate) fn latest_player_resume(&self, connection_id: u64) -> Option<PlayerResume> {
        let inner = self.lock();
        let player_id = inner.connections.get(&connection_id)?;
        let player = inner.players.get(player_id)?;
        if player.connection_id != connection_id {
            return None;
        }
        let pose = player.body_pose;
        Some(PlayerResume {
            revision: player.resume_revision,
            eye_position_metres: pose.eye_position_metres,
            look_yaw_radians: pose.look_yaw_radians,
            look_pitch_radians: pose.look_pitch_radians,
        })
    }

    /// Returns the union of connections whose current horizontal interest area covers any edited
    /// voxel. A half-metre dig supplies many mutations but few distinct X/Z centers; collect
    /// their spatial cells first so the global presence registry is locked and scanned once.
    pub(crate) fn connections_near_voxels(
        &self,
        coords: impl IntoIterator<Item = VoxelCoord>,
    ) -> BTreeSet<u64> {
        let xz_voxels = coords
            .into_iter()
            .map(|coord| (coord.x, coord.z))
            .collect::<BTreeSet<_>>();
        if xz_voxels.is_empty() {
            return BTreeSet::new();
        }
        let radius = f32::from(self.config.interest_radius_metres);
        let size = f32::from(self.config.spatial_cell_metres);
        let cell_radius = (i32::from(self.config.interest_radius_metres)
            + i32::from(self.config.spatial_cell_metres)
            - 1)
            / i32::from(self.config.spatial_cell_metres);
        let mut cells = BTreeSet::new();
        for (x, z) in &xz_voxels {
            let center = SpatialCell {
                x: (*x as f32 * VOXEL_SIZE_METRES / size).floor() as i32,
                z: (*z as f32 * VOXEL_SIZE_METRES / size).floor() as i32,
            };
            for cell_x in (center.x - cell_radius)..=(center.x + cell_radius) {
                for cell_z in (center.z - cell_radius)..=(center.z + cell_radius) {
                    cells.insert(SpatialCell {
                        x: cell_x,
                        z: cell_z,
                    });
                }
            }
        }
        let inner = self.lock();
        let mut candidate_players = BTreeSet::new();
        for cell in cells {
            if let Some(players) = inner.cells.get(&cell) {
                candidate_players.extend(players);
            }
        }
        let radius_squared = radius * radius;
        candidate_players
            .into_iter()
            .filter_map(|player_id| inner.players.get(player_id))
            .filter_map(|player| {
                let pose = player.pose.filter(|_| player.presence_attached)?;
                xz_voxels
                    .iter()
                    .any(|(x, z)| {
                        let dx = pose.eye_position_metres[0] - *x as f32 * VOXEL_SIZE_METRES;
                        let dz = pose.eye_position_metres[2] - *z as f32 * VOXEL_SIZE_METRES;
                        dx.mul_add(dx, dz * dz) <= radius_squared
                    })
                    .then_some(player.connection_id)
            })
            .collect()
    }

    pub(crate) fn join(
        self: &Arc<Self>,
        identity: &PlayerIdentity,
        resume: PlayerResume,
    ) -> Option<WorldPresenceClaim> {
        if !valid_resume(resume) {
            return None;
        }
        let now = self.now_ms();
        let mut inner = self.lock();
        if inner.players.contains_key(&identity.player_id)
            || inner.players.len() >= usize::from(self.config.max_players)
        {
            return None;
        }
        let connection_id = inner.next_connection_id;
        inner.next_connection_id = inner.next_connection_id.wrapping_add(1).max(1);
        let session_id = loop {
            let candidate = PresenceSessionId::from_bytes(*Uuid::new_v4().as_bytes());
            if !inner.sessions.contains_key(&candidate) {
                break candidate;
            }
        };
        let color_index =
            choose_color(identity.player_id, &inner.players, self.config.max_players)?;
        inner.sessions.insert(session_id, identity.player_id);
        inner.connections.insert(connection_id, identity.player_id);
        let movement_slack = f32::from(self.gameplay.movement_slack_centimetres) * 0.01;
        let body_pose = PlayerPoseUpdate {
            sequence: 0,
            sample_server_time_ms: 0,
            eye_position_metres: resume.eye_position_metres,
            linear_velocity_metres_per_second: [0.0; 3],
            look_yaw_radians: resume.look_yaw_radians,
            look_pitch_radians: resume.look_pitch_radians,
            flags: 0,
        };
        inner.players.insert(
            identity.player_id,
            PlayerSession {
                browser_user_id: identity.browser_user_id,
                connection_id,
                session_id,
                color_index,
                presence_attached: false,
                pose: Some(body_pose),
                body_pose,
                resume_revision: resume.revision,
                last_pose_receipt_ms: 0,
                last_discontinuity_sequence: 0,
                discontinuity_on_next_accept: true,
                horizontal_movement_credit_metres: movement_slack,
                vertical_movement_credit_metres: movement_slack,
                movement_credit_updated_ms: now,
                cell: None,
            },
        );
        Some(WorldPresenceClaim {
            hub: Arc::clone(self),
            connection_id,
            session_id,
            player_id: identity.player_id,
        })
    }

    pub(crate) fn attach(
        self: &Arc<Self>,
        session_id: PresenceSessionId,
    ) -> Option<PresenceAttachment> {
        let mut inner = self.lock();
        let player_id = *inner.sessions.get(&session_id)?;
        let player = inner.players.get_mut(&player_id)?;
        if player.session_id != session_id || player.presence_attached {
            return None;
        }
        player.presence_attached = true;
        player.discontinuity_on_next_accept = true;
        Some(PresenceAttachment {
            hub: Arc::clone(self),
            connection_id: player.connection_id,
            session_id,
            player_id,
            browser_user_id: player.browser_user_id,
        })
    }

    pub(crate) fn accept_pose(
        &self,
        attachment: &PresenceAttachment,
        mut pose: PlayerPoseUpdate,
    ) -> PoseAdmission {
        let min_interval_ms =
            1_000_u64.div_ceil(u64::from(self.config.max_pose_updates_per_second));
        let spectator = is_spectator(pose);
        let horizontal_speed = f32::from(if spectator {
            self.gameplay
                .spectator_max_horizontal_speed_centimetres_per_second
        } else {
            self.gameplay.max_horizontal_speed_centimetres_per_second
        }) * 0.01;
        let vertical_speed = f32::from(if spectator {
            self.gameplay
                .spectator_max_vertical_speed_centimetres_per_second
        } else {
            self.gameplay.max_vertical_speed_centimetres_per_second
        }) * 0.01;
        let movement_slack = f32::from(self.gameplay.movement_slack_centimetres) * 0.01;
        let credit_window_seconds = f32::from(if spectator {
            self.gameplay.spectator_movement_credit_window_ms
        } else {
            self.gameplay.movement_credit_window_ms
        }) * 0.001;
        let horizontal_credit_limit = movement_slack + horizontal_speed * credit_window_seconds;
        let vertical_credit_limit = movement_slack + vertical_speed * credit_window_seconds;
        if !valid_pose(pose) {
            return PoseAdmission::Invalid("player pose fields are invalid");
        }
        if spectator && !self.gameplay.allow_spectator_mode {
            return PoseAdmission::Invalid("spectator mode is not enabled for this world");
        }
        if pose.flags & PLAYER_POSE_GLIDING != 0 && !self.gameplay.allow_gliding {
            return PoseAdmission::Invalid("gliding is not enabled for this world");
        }
        let horizontal_velocity_squared = pose.linear_velocity_metres_per_second[0].mul_add(
            pose.linear_velocity_metres_per_second[0],
            pose.linear_velocity_metres_per_second[2] * pose.linear_velocity_metres_per_second[2],
        );
        if horizontal_velocity_squared > horizontal_speed * horizontal_speed
            || pose.linear_velocity_metres_per_second[1].abs() > vertical_speed
        {
            return PoseAdmission::Invalid("player velocity exceeds the authoritative limit");
        }
        // Discontinuities are a server-authored presentation hint, never movement permission.
        pose.flags &= !PLAYER_POSE_DISCONTINUITY;
        let mut inner = self.lock();
        // Admission is serialized by the hub lock. Timestamp it here so lock contention cannot
        // make ordinary queued motion look faster than the configured receipt-time budget.
        let now = self.now_ms();
        let (old_cell, new_cell) = {
            let Some(player) = inner.players.get_mut(&attachment.player_id) else {
                return PoseAdmission::SessionClosed;
            };
            if player.connection_id != attachment.connection_id
                || player.session_id != attachment.session_id
                || !player.presence_attached
            {
                return PoseAdmission::SessionClosed;
            }
            if pose.sequence <= player.pose.map_or(0, |prior| prior.sequence) {
                return PoseAdmission::IgnoredStale;
            }
            let role_transition = player
                .pose
                .is_some_and(|prior| is_spectator(prior) != is_spectator(pose));
            if player.last_pose_receipt_ms != 0
                && now.saturating_sub(player.last_pose_receipt_ms) < min_interval_ms
                && !role_transition
            {
                return PoseAdmission::IgnoredRateLimit;
            }
            if pose.sample_server_time_ms == 0 {
                pose.sample_server_time_ms = now.max(
                    player
                        .pose
                        .map_or(1, |prior| prior.sample_server_time_ms.saturating_add(1)),
                );
            } else if pose.sample_server_time_ms > now.saturating_add(MAX_FUTURE_SAMPLE_MS) {
                return PoseAdmission::Invalid("player pose timestamp is too far in the future");
            } else if now.saturating_sub(pose.sample_server_time_ms) > MAX_STALE_SAMPLE_MS
                || player
                    .pose
                    .is_some_and(|prior| pose.sample_server_time_ms <= prior.sample_server_time_ms)
            {
                return PoseAdmission::IgnoredStale;
            }
            let Some(prior) = player.pose else {
                return PoseAdmission::SessionClosed;
            };
            let entering_spectator = !is_spectator(prior) && is_spectator(pose);
            let leaving_spectator = is_spectator(prior) && !is_spectator(pose);
            if leaving_spectator {
                pose.eye_position_metres = player.body_pose.eye_position_metres;
                pose.linear_velocity_metres_per_second =
                    player.body_pose.linear_velocity_metres_per_second;
                pose.look_yaw_radians = player.body_pose.look_yaw_radians;
                pose.look_pitch_radians = player.body_pose.look_pitch_radians;
                pose.flags = player.body_pose.flags;
            }
            let elapsed_ms = now.saturating_sub(player.movement_credit_updated_ms);
            let elapsed_seconds = elapsed_ms as f32 * 0.001;
            let horizontal_credit = replenish_movement_credit(
                player.horizontal_movement_credit_metres,
                horizontal_speed,
                elapsed_seconds,
                horizontal_credit_limit,
            );
            let vertical_credit = replenish_movement_credit(
                player.vertical_movement_credit_metres,
                vertical_speed,
                elapsed_seconds,
                vertical_credit_limit,
            );
            if leaving_spectator {
                player.horizontal_movement_credit_metres = movement_slack;
                player.vertical_movement_credit_metres = movement_slack;
            } else {
                let dx = pose.eye_position_metres[0] - prior.eye_position_metres[0];
                let dz = pose.eye_position_metres[2] - prior.eye_position_metres[2];
                let horizontal_distance = dx.mul_add(dx, dz * dz).sqrt();
                let vertical_distance =
                    (pose.eye_position_metres[1] - prior.eye_position_metres[1]).abs();
                if horizontal_distance > horizontal_credit + f32::EPSILON {
                    return PoseAdmission::Invalid(
                        "player horizontal movement exceeded its budget",
                    );
                }
                if vertical_distance > vertical_credit + f32::EPSILON {
                    return PoseAdmission::Invalid("player vertical movement exceeded its budget");
                }
                player.horizontal_movement_credit_metres =
                    (horizontal_credit - horizontal_distance).max(0.0);
                player.vertical_movement_credit_metres =
                    (vertical_credit - vertical_distance).max(0.0);
            }
            player.movement_credit_updated_ms = now;
            if player.discontinuity_on_next_accept {
                player.last_discontinuity_sequence = pose.sequence;
                player.discontinuity_on_next_accept = false;
            }
            let old_cell = player.cell;
            let new_cell = Some(cell_for_pose(pose, self.config.spatial_cell_metres));
            player.pose = Some(pose);
            if entering_spectator {
                player.body_pose = PlayerPoseUpdate {
                    linear_velocity_metres_per_second: prior.linear_velocity_metres_per_second,
                    flags: prior.flags,
                    ..pose
                };
                player.resume_revision = player.resume_revision.saturating_add(1);
            } else if !is_spectator(pose) {
                player.body_pose = pose;
                player.resume_revision = player.resume_revision.saturating_add(1);
            }
            player.last_pose_receipt_ms = now;
            player.cell = new_cell;
            (old_cell, new_cell)
        };
        move_cell_membership(&mut inner.cells, attachment.player_id, old_cell, new_cell);
        PoseAdmission::Accepted
    }

    /// Builds at most one receiver-specific delta. The hub lock is held only while copying the
    /// viewer's nearby immutable records; ranking and encoding happen after it is released.
    pub(crate) fn build_delta(
        &self,
        attachment: &PresenceAttachment,
        stream: &mut PresenceStreamState,
    ) -> Result<Option<Vec<u8>>, String> {
        let now = self.now_ms();
        let candidates = self.nearby_candidates(attachment, stream)?;
        let visible_player_count = u16::try_from(candidates.len()).unwrap_or(u16::MAX);
        let relevant = candidates
            .iter()
            .map(|candidate| (candidate.state.player_id, candidate.state.connection_id))
            .collect::<HashMap<_, _>>();

        let mut leaves = Vec::new();
        stream.known.retain(|player_id, sent| {
            let keep = relevant
                .get(player_id)
                .is_some_and(|connection_id| *connection_id == sent.connection_id);
            if !keep {
                leaves.push(sent.connection_id);
            }
            keep
        });
        leaves.sort_unstable();

        let mut pending = Vec::new();
        for candidate in candidates {
            let Some(sent) = stream.known.get(&candidate.state.player_id).copied() else {
                pending.push(PendingRecord {
                    candidate,
                    enter: true,
                    discontinuity: true,
                    prediction_error: true,
                    overdue_millis: u64::MAX,
                });
                continue;
            };
            if candidate.state.pose.sequence <= sent.pose.sequence {
                continue;
            }
            let interval = self.target_interval_ms(candidate.distance_squared);
            let age = now.saturating_sub(sent.sent_at_ms);
            let discontinuity = candidate.last_discontinuity_sequence > sent.pose.sequence;
            let prediction_error = self.prediction_error(&candidate, sent.pose);
            if discontinuity || prediction_error || age >= interval {
                pending.push(PendingRecord {
                    candidate,
                    enter: false,
                    discontinuity,
                    prediction_error,
                    overdue_millis: age.saturating_sub(interval),
                });
            }
        }
        let record_limit = usize::from(self.config.max_records_per_delta);
        if pending.len() > record_limit {
            pending.select_nth_unstable_by(record_limit, compare_pending);
            pending.truncate(record_limit);
        }

        let mut enters = Vec::new();
        let mut updates = Vec::new();
        for record in pending {
            let mut state = record.candidate.state;
            if record.discontinuity {
                state.pose.flags |= PLAYER_POSE_DISCONTINUITY;
            }
            stream.known.insert(
                state.player_id,
                SentState {
                    connection_id: state.connection_id,
                    pose: state.pose,
                    sent_at_ms: now,
                },
            );
            if record.enter {
                enters.push(state);
            } else {
                updates.push(PlayerPresenceUpdate {
                    connection_id: state.connection_id,
                    pose: state.pose,
                });
            }
        }
        enters.sort_unstable_by_key(|player| player.player_id);
        updates.sort_unstable_by_key(|update| update.connection_id);

        if stream.sent_initial && enters.is_empty() && updates.is_empty() && leaves.is_empty() {
            return Ok(None);
        }
        stream.stream_sequence = stream.stream_sequence.wrapping_add(1).max(1);
        stream.sent_initial = true;
        encode_presence_delta(&PresenceDelta {
            stream_sequence: stream.stream_sequence,
            server_time_ms: now,
            visible_player_count,
            enters,
            updates,
            leaves,
        })
        .map(Some)
        .map_err(|error| error.to_string())
    }

    fn nearby_candidates(
        &self,
        attachment: &PresenceAttachment,
        stream: &PresenceStreamState,
    ) -> Result<Vec<Candidate>, String> {
        let inner = self.lock();
        let viewer = inner
            .players
            .get(&attachment.player_id)
            .filter(|player| {
                player.connection_id == attachment.connection_id
                    && player.session_id == attachment.session_id
                    && player.presence_attached
            })
            .ok_or_else(|| "presence session closed".to_owned())?;
        let Some(viewer_pose) = viewer.pose else {
            return Ok(Vec::new());
        };
        let viewer_cell = cell_for_pose(viewer_pose, self.config.spatial_cell_metres);
        let outer_radius = self
            .config
            .interest_radius_metres
            .saturating_add(self.config.interest_hysteresis_metres);
        let cell_size = i32::from(self.config.spatial_cell_metres);
        let cell_radius = (i32::from(outer_radius) + cell_size - 1) / cell_size;
        let mut candidates = Vec::new();
        for cell_x in (viewer_cell.x - cell_radius)..=(viewer_cell.x + cell_radius) {
            for cell_z in (viewer_cell.z - cell_radius)..=(viewer_cell.z + cell_radius) {
                let Some(players) = inner.cells.get(&SpatialCell {
                    x: cell_x,
                    z: cell_z,
                }) else {
                    continue;
                };
                for player_id in players {
                    if *player_id == attachment.player_id {
                        continue;
                    }
                    let Some(player) = inner.players.get(player_id) else {
                        continue;
                    };
                    let Some(pose) = player
                        .pose
                        .filter(|pose| player.presence_attached && !is_spectator(*pose))
                    else {
                        continue;
                    };
                    let dx = pose.eye_position_metres[0] - viewer_pose.eye_position_metres[0];
                    let dz = pose.eye_position_metres[2] - viewer_pose.eye_position_metres[2];
                    let distance_squared = dx.mul_add(dx, dz * dz);
                    let radius = if stream.known.contains_key(player_id) {
                        outer_radius
                    } else {
                        self.config.interest_radius_metres
                    };
                    if distance_squared > f32::from(radius).powi(2) {
                        continue;
                    }
                    candidates.push(Candidate {
                        state: PlayerPresenceState {
                            player_id: *player_id,
                            connection_id: player.connection_id,
                            color_index: player.color_index,
                            pose,
                        },
                        distance_squared,
                        last_discontinuity_sequence: player.last_discontinuity_sequence,
                    });
                }
            }
        }
        Ok(candidates)
    }

    fn target_interval_ms(&self, distance_squared: f32) -> u64 {
        if distance_squared <= f32::from(self.config.near_radius_metres).powi(2) {
            u64::from(self.config.near_update_interval_ms)
        } else if distance_squared <= f32::from(self.config.mid_radius_metres).powi(2) {
            u64::from(self.config.mid_update_interval_ms)
        } else {
            u64::from(self.config.far_update_interval_ms)
        }
    }

    fn prediction_error(&self, candidate: &Candidate, sent: PlayerPoseUpdate) -> bool {
        let elapsed_seconds = candidate
            .state
            .pose
            .sample_server_time_ms
            .saturating_sub(sent.sample_server_time_ms) as f32
            / 1_000.0;
        let predicted = std::array::from_fn(|axis| {
            sent.eye_position_metres[axis]
                + sent.linear_velocity_metres_per_second[axis] * elapsed_seconds
        });
        let distance_scale = 1.0
            + candidate.distance_squared.sqrt()
                / f32::from(self.config.interest_radius_metres.max(1));
        let position_threshold =
            f32::from(self.config.prediction_error_centimetres) * 0.01 * distance_scale;
        if squared_distance(predicted, candidate.state.pose.eye_position_metres)
            > position_threshold * position_threshold
        {
            return true;
        }
        let look_threshold =
            f32::from(self.config.look_error_milliradians) * 0.001 * distance_scale;
        angle_delta(sent.look_yaw_radians, candidate.state.pose.look_yaw_radians).abs()
            > look_threshold
            || (sent.look_pitch_radians - candidate.state.pose.look_pitch_radians).abs()
                > look_threshold
    }

    fn leave_world(&self, player_id: PlayerId, connection_id: u64) {
        let mut inner = self.lock();
        let Some(player) = inner.players.get(&player_id) else {
            return;
        };
        if player.connection_id != connection_id {
            return;
        }
        let session_id = player.session_id;
        let old_cell = player.cell;
        inner.players.remove(&player_id);
        inner.sessions.remove(&session_id);
        inner.connections.remove(&connection_id);
        move_cell_membership(&mut inner.cells, player_id, old_cell, None);
    }

    fn detach_presence(
        &self,
        player_id: PlayerId,
        connection_id: u64,
        session_id: PresenceSessionId,
    ) {
        let mut inner = self.lock();
        let old_cell = if let Some(player) = inner.players.get_mut(&player_id)
            && player.connection_id == connection_id
            && player.session_id == session_id
        {
            player.presence_attached = false;
            player.discontinuity_on_next_accept = true;
            player.cell.take()
        } else {
            None
        };
        move_cell_membership(&mut inner.cells, player_id, old_cell, None);
    }
}

impl Drop for WorldPresenceClaim {
    fn drop(&mut self) {
        self.hub.leave_world(self.player_id, self.connection_id);
    }
}

impl WorldPresenceClaim {
    pub(crate) const fn player_id(&self) -> PlayerId {
        self.player_id
    }

    pub(crate) fn latest_resume(&self) -> Option<PlayerResume> {
        self.hub.latest_player_resume(self.connection_id)
    }
}

impl Drop for PresenceAttachment {
    fn drop(&mut self) {
        self.hub
            .detach_presence(self.player_id, self.connection_id, self.session_id);
    }
}

impl PresenceAttachment {
    pub(crate) const fn browser_user_id(&self) -> BrowserUserId {
        self.browser_user_id
    }

    pub(crate) const fn player_id(&self) -> PlayerId {
        self.player_id
    }
}

fn choose_color(
    player_id: PlayerId,
    players: &HashMap<PlayerId, PlayerSession>,
    max_players: u16,
) -> Option<u16> {
    let used = players
        .values()
        .map(|player| player.color_index)
        .collect::<BTreeSet<_>>();
    let hash = player_id
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        });
    let start = hash % u64::from(max_players);
    (0..max_players)
        .map(|offset| ((start + u64::from(offset)) % u64::from(max_players)) as u16)
        .find(|candidate| !used.contains(candidate))
}

fn cell_for_pose(pose: PlayerPoseUpdate, cell_metres: u16) -> SpatialCell {
    let size = f32::from(cell_metres);
    SpatialCell {
        x: (pose.eye_position_metres[0] / size).floor() as i32,
        z: (pose.eye_position_metres[2] / size).floor() as i32,
    }
}

fn move_cell_membership(
    cells: &mut HashMap<SpatialCell, BTreeSet<PlayerId>>,
    player_id: PlayerId,
    old_cell: Option<SpatialCell>,
    new_cell: Option<SpatialCell>,
) {
    if old_cell == new_cell {
        return;
    }
    if let Some(old_cell) = old_cell {
        let remove_cell = cells.get_mut(&old_cell).is_some_and(|players| {
            players.remove(&player_id);
            players.is_empty()
        });
        if remove_cell {
            cells.remove(&old_cell);
        }
    }
    if let Some(new_cell) = new_cell {
        cells.entry(new_cell).or_default().insert(player_id);
    }
}

fn compare_pending(left: &PendingRecord, right: &PendingRecord) -> Ordering {
    right
        .enter
        .cmp(&left.enter)
        .then_with(|| right.discontinuity.cmp(&left.discontinuity))
        .then_with(|| right.overdue_millis.cmp(&left.overdue_millis))
        .then_with(|| right.prediction_error.cmp(&left.prediction_error))
        .then_with(|| {
            left.candidate
                .distance_squared
                .total_cmp(&right.candidate.distance_squared)
        })
        .then_with(|| {
            left.candidate
                .state
                .player_id
                .cmp(&right.candidate.state.player_id)
        })
}

fn squared_distance(first: [f32; 3], second: [f32; 3]) -> f32 {
    first
        .into_iter()
        .zip(second)
        .map(|(first, second)| {
            let delta = first - second;
            delta * delta
        })
        .sum()
}

fn replenish_movement_credit(
    current_metres: f32,
    speed_metres_per_second: f32,
    elapsed_seconds: f32,
    limit_metres: f32,
) -> f32 {
    (current_metres + speed_metres_per_second * elapsed_seconds).min(limit_metres)
}

fn valid_resume(resume: PlayerResume) -> bool {
    resume.revision != 0
        && valid_position(resume.eye_position_metres)
        && valid_look(resume.look_yaw_radians, resume.look_pitch_radians)
}

fn valid_pose(pose: PlayerPoseUpdate) -> bool {
    const CLIENT_FLAGS: u16 = PLAYER_POSE_GROUNDED
        | PLAYER_POSE_SWIMMING
        | PLAYER_POSE_DISCONTINUITY
        | PLAYER_POSE_SPECTATOR
        | PLAYER_POSE_GLIDING;
    pose.sequence != 0
        && valid_position(pose.eye_position_metres)
        && pose
            .linear_velocity_metres_per_second
            .into_iter()
            .all(f32::is_finite)
        && valid_look(pose.look_yaw_radians, pose.look_pitch_radians)
        && pose.flags & !CLIENT_FLAGS == 0
        && (pose.flags & PLAYER_POSE_GLIDING == 0
            || pose.flags & (PLAYER_POSE_GROUNDED | PLAYER_POSE_SWIMMING | PLAYER_POSE_SPECTATOR)
                == 0)
        && (pose.flags & PLAYER_POSE_SPECTATOR == 0
            || pose.flags & (PLAYER_POSE_GROUNDED | PLAYER_POSE_SWIMMING | PLAYER_POSE_GLIDING)
                == 0)
}

const fn is_spectator(pose: PlayerPoseUpdate) -> bool {
    pose.flags & PLAYER_POSE_SPECTATOR != 0
}

fn valid_position(position: [f32; 3]) -> bool {
    let limit = (i32::MAX as f32 - 64.0) * VOXEL_SIZE_METRES;
    position
        .into_iter()
        .all(|value| value.is_finite() && value.abs() <= limit)
}

fn valid_look(yaw: f32, pitch: f32) -> bool {
    yaw.is_finite()
        && (-std::f32::consts::PI..=std::f32::consts::PI).contains(&yaw)
        && pitch.is_finite()
        && (-1.5..=1.5).contains(&pitch)
}

fn angle_delta(from: f32, to: f32) -> f32 {
    (to - from + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

#[cfg(test)]
mod tests {
    use super::*;
    use voxels_world::protocol::{
        BrowserUserId, EditShape, PLAYER_POSE_GROUNDED, decode_presence_delta,
    };

    fn identity(seed: u16) -> PlayerIdentity {
        let mut bytes = [0_u8; 16];
        bytes[..2].copy_from_slice(&seed.to_le_bytes());
        let mut player = bytes;
        player[15] = 1;
        PlayerIdentity {
            browser_user_id: BrowserUserId::from_bytes(bytes),
            player_id: PlayerId::from_bytes(player),
            player_name: format!("player-{seed}"),
        }
    }

    fn pose(sequence: u64, x: f32, z: f32) -> PlayerPoseUpdate {
        PlayerPoseUpdate {
            sequence,
            sample_server_time_ms: 0,
            eye_position_metres: [x, 1.62, z],
            linear_velocity_metres_per_second: [1.0, 0.0, 0.0],
            look_yaw_radians: 0.0,
            look_pitch_radians: 0.0,
            flags: PLAYER_POSE_GROUNDED,
        }
    }

    fn resume(x: f32, y: f32, z: f32) -> PlayerResume {
        PlayerResume {
            revision: 1,
            eye_position_metres: [x, y, z],
            look_yaw_radians: 0.0,
            look_pitch_radians: 0.0,
        }
    }

    fn hub(config: PresenceConfig) -> Arc<PresenceHub> {
        PresenceHub::new(config, GameplayConfig::default()).expect("hub")
    }

    fn security_hub(gameplay: GameplayConfig) -> Arc<PresenceHub> {
        PresenceHub::new(
            PresenceConfig {
                max_pose_updates_per_second: 120,
                ..PresenceConfig::default()
            },
            gameplay,
        )
        .expect("hub")
    }

    fn joined_at(
        hub: &Arc<PresenceHub>,
        seed: u16,
        x: f32,
        z: f32,
    ) -> (WorldPresenceClaim, PresenceAttachment) {
        let claim = hub.join(&identity(seed), resume(x, 1.62, z)).expect("join");
        let attachment = hub.attach(claim.session_id).expect("attach");
        assert_eq!(
            hub.accept_pose(&attachment, pose(1, x, z)),
            PoseAdmission::Accepted
        );
        (claim, attachment)
    }

    #[test]
    fn spatial_cells_floor_negative_coordinates() {
        assert_eq!(
            cell_for_pose(pose(1, -0.1, -64.1), 64),
            SpatialCell { x: -1, z: -2 }
        );
    }

    #[test]
    fn edit_interest_batches_voxels_into_one_exact_recipient_union() {
        let hub = hub(PresenceConfig {
            spatial_cell_metres: 4,
            interest_radius_metres: 5,
            ..PresenceConfig::default()
        });
        let (first_claim, _first) = joined_at(&hub, 1, 0.0, 0.0);
        let (second_claim, _second) = joined_at(&hub, 2, 30.0, 0.0);
        let (_far_claim, _far) = joined_at(&hub, 3, 60.0, 0.0);
        let recipients = hub.connections_near_voxels([
            VoxelCoord::new(0, 10, 0),
            VoxelCoord::new(0, 11, 0),
            VoxelCoord::new(300, 10, 0),
        ]);
        assert_eq!(
            recipients,
            BTreeSet::from([first_claim.connection_id, second_claim.connection_id])
        );
    }

    #[test]
    fn one_thousand_player_dense_region_is_bounded_and_starvation_free() {
        let hub = hub(PresenceConfig {
            max_players: 1_024,
            max_records_per_delta: 64,
            max_pose_updates_per_second: 120,
            ..PresenceConfig::default()
        });
        let (_viewer_claim, viewer) = joined_at(&hub, 0, 0.0, 0.0);
        let mut residents = Vec::new();
        for seed in 1..1_000 {
            residents.push(joined_at(&hub, seed, f32::from(seed % 10), 0.0));
        }
        let mut stream = PresenceStreamState::default();
        let mut entered = BTreeSet::new();
        let started = Instant::now();
        let mut wire_bytes = 0;
        let mut delta_count = 0;
        for _ in 0..32 {
            let bytes = hub
                .build_delta(&viewer, &mut stream)
                .expect("build")
                .expect("dense delta");
            wire_bytes += bytes.len();
            delta_count += 1;
            let delta = decode_presence_delta(&bytes).expect("decode");
            assert_eq!(delta.visible_player_count, 999);
            assert!(delta.enters.len() + delta.updates.len() <= 64);
            entered.extend(delta.enters.into_iter().map(|player| player.player_id));
            if entered.len() == 999 {
                break;
            }
        }
        let elapsed = started.elapsed();
        assert_eq!(delta_count, 16);
        assert_eq!(wire_bytes, 80_688);
        assert_eq!(entered.len(), 999);
        assert_eq!(stream.known.len(), 999);
        eprintln!(
            "presence-scale dense=1000 deltas={delta_count} bytes={wire_bytes} build_us={}",
            elapsed.as_micros()
        );
        drop(residents);
    }

    #[test]
    fn far_players_produce_no_candidates_or_followup_bytes() {
        let hub = hub(PresenceConfig {
            max_players: 1_024,
            max_pose_updates_per_second: 120,
            ..PresenceConfig::default()
        });
        let (_viewer_claim, viewer) = joined_at(&hub, 0, 0.0, 0.0);
        let mut residents = Vec::new();
        for seed in 1..1_000 {
            residents.push(joined_at(&hub, seed, 10_000.0 + f32::from(seed), 10_000.0));
        }
        let mut stream = PresenceStreamState::default();
        let initial = hub
            .build_delta(&viewer, &mut stream)
            .expect("build")
            .expect("initial");
        let initial = decode_presence_delta(&initial).expect("decode");
        assert_eq!(initial.visible_player_count, 0);
        assert!(initial.enters.is_empty());
        assert!(
            hub.build_delta(&viewer, &mut stream)
                .expect("build")
                .is_none()
        );
        eprintln!("presence-scale isolated=1000 observer_entity_bytes=0");
        drop(residents);
    }

    #[test]
    fn disconnect_emits_an_explicit_leave() {
        let hub = hub(PresenceConfig {
            max_pose_updates_per_second: 120,
            ..PresenceConfig::default()
        });
        let (_viewer_claim, viewer) = joined_at(&hub, 0, 0.0, 0.0);
        let (other_claim, _other) = joined_at(&hub, 1, 2.0, 0.0);
        let mut stream = PresenceStreamState::default();
        let first = hub
            .build_delta(&viewer, &mut stream)
            .expect("build")
            .expect("enter");
        let first = decode_presence_delta(&first).expect("decode");
        let connection_id = first.enters[0].connection_id;
        drop(other_claim);
        let leave = hub
            .build_delta(&viewer, &mut stream)
            .expect("build")
            .expect("leave");
        assert_eq!(
            decode_presence_delta(&leave).expect("decode").leaves,
            vec![connection_id]
        );
    }

    #[test]
    fn client_discontinuity_cannot_bypass_bounded_movement_credit() {
        let hub = security_hub(GameplayConfig::default());
        let claim = hub
            .join(&identity(10), resume(0.0, 1.62, 0.0))
            .expect("join");
        let attachment = hub.attach(claim.session_id).expect("attach");
        let mut initial = pose(1, 0.0, 0.0);
        initial.flags |= PLAYER_POSE_DISCONTINUITY;
        assert_eq!(
            hub.accept_pose(&attachment, initial),
            PoseAdmission::Accepted
        );

        {
            let mut inner = hub.lock();
            let player = inner
                .players
                .get_mut(&identity(10).player_id)
                .expect("player");
            assert_eq!(
                player.pose.expect("pose").flags & PLAYER_POSE_DISCONTINUITY,
                0
            );
            assert_eq!(player.last_discontinuity_sequence, 1);
            player.last_pose_receipt_ms = 0;
        }
        let mut teleport = pose(2, 20.0, 0.0);
        teleport.flags |= PLAYER_POSE_DISCONTINUITY;
        assert_eq!(
            hub.accept_pose(&attachment, teleport),
            PoseAdmission::Invalid("player horizontal movement exceeded its budget")
        );
        let latest = hub
            .latest_player_resume(claim.connection_id)
            .expect("latest resume");
        assert_eq!(latest.eye_position_metres, [0.0, 1.62, 0.0]);
        assert_eq!(latest.revision, 2);
    }

    #[test]
    fn movement_credit_replenishes_with_receipt_time_and_has_a_hard_window() {
        let gameplay = GameplayConfig {
            max_horizontal_speed_centimetres_per_second: 100,
            max_vertical_speed_centimetres_per_second: 100,
            movement_slack_centimetres: 1,
            movement_credit_window_ms: 100,
            ..GameplayConfig::default()
        };
        let hub = security_hub(gameplay);
        let claim = hub
            .join(&identity(11), resume(0.0, 1.62, 0.0))
            .expect("join");
        let attachment = hub.attach(claim.session_id).expect("attach");
        assert_eq!(
            hub.accept_pose(&attachment, pose(1, 0.0, 0.0)),
            PoseAdmission::Accepted
        );

        std::thread::sleep(std::time::Duration::from_millis(25));
        assert_eq!(
            hub.accept_pose(&attachment, pose(2, 0.025, 0.0)),
            PoseAdmission::Accepted
        );
        assert!((replenish_movement_credit(0.0, 1.0, 10.0, 0.11) - 0.11).abs() < f32::EPSILON);
        let credit_now = hub.now_ms();
        {
            let mut inner = hub.lock();
            let player = inner
                .players
                .get_mut(&identity(11).player_id)
                .expect("player");
            player.last_pose_receipt_ms = 0;
            player.movement_credit_updated_ms = credit_now;
            player.horizontal_movement_credit_metres = 0.0;
        }
        assert_eq!(
            hub.accept_pose(&attachment, pose(3, 0.2, 0.0)),
            PoseAdmission::Invalid("player horizontal movement exceeded its budget")
        );
    }

    #[test]
    fn reported_velocity_is_bounded_independently_per_horizontal_and_vertical_axes() {
        let hub = security_hub(GameplayConfig::default());
        let claim = hub
            .join(&identity(12), resume(0.0, 1.62, 0.0))
            .expect("join");
        let attachment = hub.attach(claim.session_id).expect("attach");

        let mut horizontal = pose(1, 0.0, 0.0);
        horizontal.linear_velocity_metres_per_second = [9.01, 0.0, 0.0];
        assert_eq!(
            hub.accept_pose(&attachment, horizontal),
            PoseAdmission::Invalid("player velocity exceeds the authoritative limit")
        );
        let mut vertical = pose(1, 0.0, 0.0);
        vertical.linear_velocity_metres_per_second = [0.0, -20.01, 0.0];
        assert_eq!(
            hub.accept_pose(&attachment, vertical),
            PoseAdmission::Invalid("player velocity exceeds the authoritative limit")
        );
    }

    #[test]
    fn spectator_mode_requires_world_authority_and_uses_separate_movement_limits() {
        let disabled = security_hub(GameplayConfig::default());
        let disabled_claim = disabled
            .join(&identity(21), resume(0.0, 1.62, 0.0))
            .expect("join");
        let disabled_attachment = disabled.attach(disabled_claim.session_id).expect("attach");
        let mut spectator = pose(1, 0.0, 0.0);
        spectator.flags = PLAYER_POSE_SPECTATOR;
        assert_eq!(
            disabled.accept_pose(&disabled_attachment, spectator),
            PoseAdmission::Invalid("spectator mode is not enabled for this world")
        );

        let enabled = security_hub(GameplayConfig {
            allow_spectator_mode: true,
            ..GameplayConfig::default()
        });
        let enabled_claim = enabled
            .join(&identity(22), resume(0.0, 1.62, 0.0))
            .expect("join");
        let enabled_attachment = enabled.attach(enabled_claim.session_id).expect("attach");
        assert_eq!(
            enabled.accept_pose(&enabled_attachment, spectator),
            PoseAdmission::Accepted
        );
        enabled
            .lock()
            .players
            .get_mut(&identity(22).player_id)
            .expect("spectator")
            .last_pose_receipt_ms = 0;

        let mut fast_spectator = pose(2, 0.0, 0.0);
        fast_spectator.flags = PLAYER_POSE_SPECTATOR;
        fast_spectator.linear_velocity_metres_per_second = [128.0, 0.0, 0.0];
        assert_eq!(
            enabled.accept_pose(&enabled_attachment, fast_spectator),
            PoseAdmission::Accepted
        );

        let credit_now = enabled.now_ms();
        {
            let mut inner = enabled.lock();
            let player = inner
                .players
                .get_mut(&identity(22).player_id)
                .expect("spectator");
            player.last_pose_receipt_ms = 0;
            player.movement_credit_updated_ms = credit_now;
            player.horizontal_movement_credit_metres = 128.0;
        }
        let mut buffered_cruise = pose(3, 128.0, 0.0);
        buffered_cruise.flags = PLAYER_POSE_SPECTATOR;
        buffered_cruise.linear_velocity_metres_per_second = [128.0, 0.0, 0.0];
        assert_eq!(
            enabled.accept_pose(&enabled_attachment, buffered_cruise),
            PoseAdmission::Accepted
        );

        let mut too_fast = pose(4, 128.0, 0.0);
        too_fast.flags = PLAYER_POSE_SPECTATOR;
        too_fast.linear_velocity_metres_per_second = [150.01, 0.0, 0.0];
        assert_eq!(
            enabled.accept_pose(&enabled_attachment, too_fast),
            PoseAdmission::Invalid("player velocity exceeds the authoritative limit")
        );

        let normal = security_hub(GameplayConfig::default());
        let normal_claim = normal
            .join(&identity(23), resume(0.0, 1.62, 0.0))
            .expect("join");
        let normal_attachment = normal.attach(normal_claim.session_id).expect("attach");
        let mut too_fast_player = pose(1, 0.0, 0.0);
        too_fast_player.linear_velocity_metres_per_second = [9.01, 0.0, 0.0];
        assert_eq!(
            normal.accept_pose(&normal_attachment, too_fast_player),
            PoseAdmission::Invalid("player velocity exceeds the authoritative limit")
        );
    }

    #[test]
    fn spectator_camera_is_bodyless_read_only_and_returns_to_its_saved_body() {
        let hub = security_hub(GameplayConfig {
            allow_spectator_mode: true,
            ..GameplayConfig::default()
        });
        let (_viewer_claim, viewer) = joined_at(&hub, 30, 0.0, 0.0);
        let (subject_claim, subject) = joined_at(&hub, 31, 0.5, 0.0);
        let mut viewer_stream = PresenceStreamState::default();
        let initial = decode_presence_delta(
            &hub.build_delta(&viewer, &mut viewer_stream)
                .expect("build viewer delta")
                .expect("subject enter"),
        )
        .expect("decode subject enter");
        assert_eq!(initial.enters.len(), 1);
        let subject_connection = initial.enters[0].connection_id;
        let body_resume = subject_claim.latest_resume().expect("body resume");

        {
            let mut inner = hub.lock();
            inner
                .players
                .get_mut(&identity(31).player_id)
                .expect("subject")
                .last_pose_receipt_ms = 0;
        }
        let mut spectator = pose(2, 0.5, 0.0);
        spectator.flags = PLAYER_POSE_SPECTATOR;
        spectator.linear_velocity_metres_per_second = [0.0; 3];
        assert_eq!(
            hub.accept_pose(&subject, spectator),
            PoseAdmission::Accepted
        );
        assert_eq!(
            hub.authorize_interaction(subject_claim.connection_id, VoxelCoord::new(5, 16, 0)),
            Err("spectators cannot edit the world")
        );
        let spectator_resume = subject_claim
            .latest_resume()
            .expect("spectator body resume");
        assert_eq!(
            spectator_resume.eye_position_metres,
            body_resume.eye_position_metres
        );
        assert_eq!(spectator_resume.revision, body_resume.revision + 1);
        let leave = decode_presence_delta(
            &hub.build_delta(&viewer, &mut viewer_stream)
                .expect("build spectator leave")
                .expect("spectator leave"),
        )
        .expect("decode spectator leave");
        assert_eq!(leave.visible_player_count, 0);
        assert_eq!(leave.leaves, vec![subject_connection]);

        let edit_recipients = hub.connections_near_voxels([VoxelCoord::new(5, 16, 0)]);
        assert!(
            edit_recipients.contains(&subject_claim.connection_id),
            "spectator camera must retain nearby world-edit interest"
        );
        let mut spectator_stream = PresenceStreamState::default();
        let camera_view = decode_presence_delta(
            &hub.build_delta(&subject, &mut spectator_stream)
                .expect("build camera view")
                .expect("viewer enter"),
        )
        .expect("decode camera view");
        assert_eq!(camera_view.enters.len(), 1);

        {
            let mut inner = hub.lock();
            inner
                .players
                .get_mut(&identity(31).player_id)
                .expect("subject")
                .last_pose_receipt_ms = 0;
        }
        let attempted_return = pose(3, 100.0, 100.0);
        assert_eq!(
            hub.accept_pose(&subject, attempted_return),
            PoseAdmission::Accepted
        );
        let returned = decode_presence_delta(
            &hub.build_delta(&viewer, &mut viewer_stream)
                .expect("build returned body")
                .expect("body re-enter"),
        )
        .expect("decode returned body");
        assert_eq!(returned.enters.len(), 1);
        assert_eq!(
            returned.enters[0].pose.eye_position_metres,
            body_resume.eye_position_metres
        );
        let restored_resume = subject_claim.latest_resume().expect("restored resume");
        assert_eq!(
            restored_resume.eye_position_metres,
            body_resume.eye_position_metres
        );
        assert_eq!(restored_resume.revision, spectator_resume.revision + 1);
    }

    #[test]
    fn gliding_requires_world_authority_and_an_exclusive_airborne_pose() {
        let disabled = security_hub(GameplayConfig {
            allow_gliding: false,
            ..GameplayConfig::default()
        });
        let disabled_claim = disabled
            .join(&identity(23), resume(0.0, 1.62, 0.0))
            .expect("join");
        let disabled_attachment = disabled.attach(disabled_claim.session_id).expect("attach");
        let mut gliding = pose(1, 0.0, 0.0);
        gliding.flags = PLAYER_POSE_GLIDING;
        assert_eq!(
            disabled.accept_pose(&disabled_attachment, gliding),
            PoseAdmission::Invalid("gliding is not enabled for this world")
        );

        let enabled = security_hub(GameplayConfig::default());
        let enabled_claim = enabled
            .join(&identity(24), resume(0.0, 1.62, 0.0))
            .expect("join");
        let enabled_attachment = enabled.attach(enabled_claim.session_id).expect("attach");
        assert_eq!(
            enabled.accept_pose(&enabled_attachment, gliding),
            PoseAdmission::Accepted
        );
        let mut conflicting = gliding;
        conflicting.sequence = 2;
        conflicting.flags |= PLAYER_POSE_GROUNDED;
        assert_eq!(
            enabled.accept_pose(&enabled_attachment, conflicting),
            PoseAdmission::Invalid("player pose fields are invalid")
        );
    }

    #[test]
    fn detach_retains_resume_and_next_accepted_pose_is_server_discontinuous() {
        let hub = security_hub(GameplayConfig::default());
        let claim = hub
            .join(&identity(13), resume(3.0, 1.62, -2.0))
            .expect("join");
        let attachment = hub.attach(claim.session_id).expect("attach");
        assert_eq!(
            hub.accept_pose(&attachment, pose(1, 3.0, -2.0)),
            PoseAdmission::Accepted
        );
        drop(attachment);
        assert_eq!(
            hub.latest_player_resume(claim.connection_id)
                .expect("retained")
                .eye_position_metres,
            [3.0, 1.62, -2.0]
        );
        assert_eq!(
            hub.authorize_interaction(claim.connection_id, VoxelCoord::new(30, 16, -20)),
            Err("player presence is not attached")
        );

        let reattached = hub.attach(claim.session_id).expect("reattach");
        {
            let mut inner = hub.lock();
            inner
                .players
                .get_mut(&identity(13).player_id)
                .expect("player")
                .last_pose_receipt_ms = 0;
        }
        assert_eq!(
            hub.accept_pose(&reattached, pose(2, 3.0, -2.0)),
            PoseAdmission::Accepted
        );
        let inner = hub.lock();
        let player = inner.players.get(&identity(13).player_id).expect("player");
        assert_eq!(player.last_discontinuity_sequence, 2);
        assert_eq!(
            player.pose.expect("pose").flags & PLAYER_POSE_DISCONTINUITY,
            0
        );
    }

    #[test]
    fn interaction_authorization_requires_attached_fresh_pose_and_nearest_aabb_reach() {
        let gameplay = GameplayConfig {
            interaction_reach_centimetres: 500,
            interaction_latency_slack_centimetres: 100,
            interaction_pose_max_age_ms: 1,
            ..GameplayConfig::default()
        };
        let hub = security_hub(gameplay);
        let claim = hub
            .join(&identity(14), resume(0.05, 1.65, 0.05))
            .expect("join");
        let attachment = hub.attach(claim.session_id).expect("attach");
        let near = VoxelCoord::new(0, 16, -59);
        assert_eq!(
            hub.authorize_interaction(claim.connection_id, near),
            Err("player pose is stale")
        );
        assert_eq!(
            hub.accept_pose(&attachment, pose(1, 0.05, 0.05)),
            PoseAdmission::Accepted
        );
        assert_eq!(hub.authorize_interaction(claim.connection_id, near), Ok(()));
        assert_eq!(
            hub.authorize_interaction(claim.connection_id, VoxelCoord::new(0, 16, -61)),
            Err("interaction target is out of reach")
        );
        std::thread::sleep(std::time::Duration::from_millis(3));
        assert_eq!(
            hub.authorize_interaction(claim.connection_id, near),
            Err("player pose is stale")
        );
    }

    #[test]
    fn placement_volume_cannot_intersect_the_authoritative_player_capsule() {
        let hub = security_hub(GameplayConfig::default());
        let claim = hub
            .join(&identity(16), resume(0.05, 1.62, 0.05))
            .expect("join");
        let attachment = hub.attach(claim.session_id).expect("attach");
        assert_eq!(
            hub.accept_pose(&attachment, pose(1, 0.05, 0.05)),
            PoseAdmission::Accepted
        );

        let intersecting = EditVolume::for_hit(VoxelCoord::new(0, 10, 0), EditShape::Cube).unwrap();
        assert_eq!(
            hub.authorize_placement_volume(claim.connection_id, intersecting),
            Err("placement volume intersects the player")
        );
        let clear = EditVolume::for_hit(VoxelCoord::new(20, 10, 0), EditShape::Cube).unwrap();
        assert_eq!(
            hub.authorize_placement_volume(claim.connection_id, clear),
            Ok(())
        );
    }

    #[test]
    fn invalid_authoritative_resume_is_rejected_before_session_allocation() {
        let hub = security_hub(GameplayConfig::default());
        let mut invalid = resume(0.0, 1.62, 0.0);
        invalid.revision = 0;
        assert!(hub.join(&identity(15), invalid).is_none());
        invalid.revision = 1;
        invalid.eye_position_metres[0] = f32::NAN;
        assert!(hub.join(&identity(15), invalid).is_none());
        assert!(hub.lock().players.is_empty());
    }
}
