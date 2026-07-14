//! Server-authoritative player-presence registry and spatial replication scheduler.
//!
//! World-product WebSockets own player sessions. A second, small-frame presence WebSocket binds
//! with the unguessable session token returned by `WorldOpened`. Authoritative records live in one
//! world, while a spatial hash and receiver-local stream state make replication proportional to
//! nearby relationships instead of the square of the global player count.

use crate::PresenceConfig;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;
use uuid::Uuid;
use voxels_world::protocol::{
    PLAYER_POSE_DISCONTINUITY, PlayerId, PlayerIdentity, PlayerPoseUpdate, PlayerPresenceState,
    PlayerPresenceUpdate, PresenceDelta, PresenceSessionId, encode_presence_delta,
};
use voxels_world::{VOXEL_SIZE_METRES, VoxelCoord};

const MAX_FUTURE_SAMPLE_MS: u64 = 250;
const MAX_STALE_SAMPLE_MS: u64 = 2_000;

pub(crate) struct PresenceHub {
    started: Instant,
    config: PresenceConfig,
    inner: Mutex<PresenceInner>,
}

struct PresenceInner {
    next_connection_id: u64,
    players: HashMap<PlayerId, PlayerSession>,
    sessions: HashMap<PresenceSessionId, PlayerId>,
    cells: HashMap<SpatialCell, BTreeSet<PlayerId>>,
}

struct PlayerSession {
    connection_id: u64,
    session_id: PresenceSessionId,
    color_index: u16,
    presence_attached: bool,
    pose: Option<PlayerPoseUpdate>,
    last_pose_receipt_ms: u64,
    last_discontinuity_sequence: u64,
    cell: Option<SpatialCell>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
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
    known: BTreeMap<PlayerId, SentState>,
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
    pub(crate) fn new(config: PresenceConfig) -> Result<Arc<Self>, String> {
        Ok(Arc::new(Self {
            started: Instant::now(),
            config,
            inner: Mutex::new(PresenceInner {
                next_connection_id: 1,
                players: HashMap::new(),
                sessions: HashMap::new(),
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

    /// Returns only connections whose current horizontal interest area covers an edited voxel.
    /// The existing spatial grid keeps one edit proportional to local players, not world population.
    pub(crate) fn connections_near_voxel(&self, coord: VoxelCoord) -> BTreeSet<u64> {
        let x = coord.x as f32 * VOXEL_SIZE_METRES;
        let z = coord.z as f32 * VOXEL_SIZE_METRES;
        let radius = f32::from(self.config.interest_radius_metres);
        let size = f32::from(self.config.spatial_cell_metres);
        let center = SpatialCell {
            x: (x / size).floor() as i32,
            z: (z / size).floor() as i32,
        };
        let cell_radius = (i32::from(self.config.interest_radius_metres)
            + i32::from(self.config.spatial_cell_metres)
            - 1)
            / i32::from(self.config.spatial_cell_metres);
        let mut connections = BTreeSet::new();
        let inner = self.lock();
        for cell_x in (center.x - cell_radius)..=(center.x + cell_radius) {
            for cell_z in (center.z - cell_radius)..=(center.z + cell_radius) {
                let Some(players) = inner.cells.get(&SpatialCell {
                    x: cell_x,
                    z: cell_z,
                }) else {
                    continue;
                };
                for player_id in players {
                    let Some(player) = inner.players.get(player_id) else {
                        continue;
                    };
                    let Some(pose) = player.pose.filter(|_| player.presence_attached) else {
                        continue;
                    };
                    let dx = pose.eye_position_metres[0] - x;
                    let dz = pose.eye_position_metres[2] - z;
                    if dx.mul_add(dx, dz * dz) <= radius * radius {
                        connections.insert(player.connection_id);
                    }
                }
            }
        }
        connections
    }

    pub(crate) fn join(self: &Arc<Self>, identity: &PlayerIdentity) -> Option<WorldPresenceClaim> {
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
        inner.players.insert(
            identity.player_id,
            PlayerSession {
                connection_id,
                session_id,
                color_index,
                presence_attached: false,
                pose: None,
                last_pose_receipt_ms: 0,
                last_discontinuity_sequence: 0,
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
        Some(PresenceAttachment {
            hub: Arc::clone(self),
            connection_id: player.connection_id,
            session_id,
            player_id,
        })
    }

    pub(crate) fn accept_pose(
        &self,
        attachment: &PresenceAttachment,
        mut pose: PlayerPoseUpdate,
    ) -> PoseAdmission {
        let now = self.now_ms();
        let min_interval_ms =
            1_000_u64.div_ceil(u64::from(self.config.max_pose_updates_per_second));
        let mut inner = self.lock();
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
            if player.last_pose_receipt_ms != 0
                && now.saturating_sub(player.last_pose_receipt_ms) < min_interval_ms
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
            if let Some(prior) = player.pose {
                let distance_squared =
                    squared_distance(pose.eye_position_metres, prior.eye_position_metres);
                let teleport = f32::from(self.config.teleport_distance_metres);
                if distance_squared > teleport * teleport
                    && pose.flags & PLAYER_POSE_DISCONTINUITY == 0
                {
                    return PoseAdmission::Invalid(
                        "large player position change lacks discontinuity flag",
                    );
                }
            }
            if pose.flags & PLAYER_POSE_DISCONTINUITY != 0 {
                player.last_discontinuity_sequence = pose.sequence;
            }
            let old_cell = player.cell;
            let new_cell = Some(cell_for_pose(pose, self.config.spatial_cell_metres));
            player.pose = Some(pose);
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
            .map(|candidate| (candidate.state.player_id, *candidate))
            .collect::<BTreeMap<_, _>>();

        let mut leaves = Vec::new();
        stream.known.retain(|player_id, sent| {
            let keep = relevant
                .get(player_id)
                .is_some_and(|candidate| candidate.state.connection_id == sent.connection_id);
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
        pending.sort_unstable_by(compare_pending);
        pending.truncate(usize::from(self.config.max_records_per_delta));

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
                    let Some(pose) = player.pose.filter(|_| player.presence_attached) else {
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
            player.pose = None;
            player.last_discontinuity_sequence = 0;
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
}

impl Drop for PresenceAttachment {
    fn drop(&mut self) {
        self.hub
            .detach_presence(self.player_id, self.connection_id, self.session_id);
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

fn angle_delta(from: f32, to: f32) -> f32 {
    (to - from + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

#[cfg(test)]
mod tests {
    use super::*;
    use voxels_world::protocol::{BrowserUserId, PLAYER_POSE_GROUNDED, decode_presence_delta};

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

    fn joined_at(
        hub: &Arc<PresenceHub>,
        seed: u16,
        x: f32,
        z: f32,
    ) -> (WorldPresenceClaim, PresenceAttachment) {
        let claim = hub.join(&identity(seed)).expect("join");
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
    fn five_hundred_twelve_player_dense_region_is_bounded_and_starvation_free() {
        let hub = PresenceHub::new(PresenceConfig {
            max_players: 512,
            max_records_per_delta: 64,
            max_pose_updates_per_second: 120,
            ..PresenceConfig::default()
        })
        .expect("hub");
        let (_viewer_claim, viewer) = joined_at(&hub, 0, 0.0, 0.0);
        let mut residents = Vec::new();
        for seed in 1..512 {
            residents.push(joined_at(&hub, seed, f32::from(seed % 10), 0.0));
        }
        let mut stream = PresenceStreamState::default();
        let mut entered = BTreeSet::new();
        let started = Instant::now();
        let mut wire_bytes = 0;
        let mut delta_count = 0;
        for _ in 0..16 {
            let bytes = hub
                .build_delta(&viewer, &mut stream)
                .expect("build")
                .expect("dense delta");
            wire_bytes += bytes.len();
            delta_count += 1;
            let delta = decode_presence_delta(&bytes).expect("decode");
            assert_eq!(delta.visible_player_count, 511);
            assert!(delta.enters.len() + delta.updates.len() <= 64);
            entered.extend(delta.enters.into_iter().map(|player| player.player_id));
            if entered.len() == 511 {
                break;
            }
        }
        let elapsed = started.elapsed();
        assert_eq!(delta_count, 8);
        assert_eq!(wire_bytes, 41_264);
        assert_eq!(entered.len(), 511);
        assert_eq!(stream.known.len(), 511);
        eprintln!(
            "presence-scale dense=512 deltas={delta_count} bytes={wire_bytes} build_us={}",
            elapsed.as_micros()
        );
        drop(residents);
    }

    #[test]
    fn far_players_produce_no_candidates_or_followup_bytes() {
        let hub = PresenceHub::new(PresenceConfig {
            max_players: 512,
            max_pose_updates_per_second: 120,
            ..PresenceConfig::default()
        })
        .expect("hub");
        let (_viewer_claim, viewer) = joined_at(&hub, 0, 0.0, 0.0);
        let mut residents = Vec::new();
        for seed in 1..512 {
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
        eprintln!("presence-scale isolated=512 observer_entity_bytes=0");
        drop(residents);
    }

    #[test]
    fn disconnect_emits_an_explicit_leave() {
        let hub = PresenceHub::new(PresenceConfig {
            max_pose_updates_per_second: 120,
            ..PresenceConfig::default()
        })
        .expect("hub");
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
}
