//! Server-authoritative player-presence registry.
//!
//! World-product WebSockets own player sessions. A second, small-frame presence WebSocket binds
//! with the unguessable session token returned by `WorldOpened`. The hub retains only the latest
//! accepted pose and publishes complete, coalesced rosters, so a slow receiver never accumulates
//! stale movement packets.

use crate::PresenceConfig;
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;
use tokio::sync::watch;
use tokio::time::{Duration, MissedTickBehavior};
use uuid::Uuid;
use voxels_world::protocol::{
    PLAYER_POSE_DISCONTINUITY, PlayerId, PlayerIdentity, PlayerPoseUpdate, PlayerPresenceState,
    PresenceSessionId, PresenceSnapshot, encode_presence_snapshot,
};

const MAX_FUTURE_SAMPLE_MS: u64 = 250;
const MAX_STALE_SAMPLE_MS: u64 = 2_000;

pub(crate) struct PresenceHub {
    started: Instant,
    config: PresenceConfig,
    inner: Mutex<PresenceInner>,
    latest_snapshot: watch::Sender<Arc<Vec<u8>>>,
}

struct PresenceInner {
    next_connection_id: u64,
    snapshot_sequence: u64,
    players: HashMap<PlayerId, PlayerSession>,
    sessions: HashMap<PresenceSessionId, PlayerId>,
}

struct PlayerSession {
    connection_id: u64,
    session_id: PresenceSessionId,
    color_index: u8,
    presence_attached: bool,
    pose: Option<PlayerPoseUpdate>,
    last_pose_receipt_ms: u64,
    discontinuity_pending: bool,
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
    pub(crate) fn new(config: PresenceConfig) -> Arc<Self> {
        let initial = PresenceSnapshot {
            snapshot_sequence: 1,
            server_time_ms: 1,
            players: Vec::new(),
        };
        let initial = encode_presence_snapshot(&initial)
            .expect("empty server presence snapshot is always valid");
        let (latest_snapshot, _) = watch::channel(Arc::new(initial));
        let hub = Arc::new(Self {
            started: Instant::now(),
            config,
            inner: Mutex::new(PresenceInner {
                next_connection_id: 1,
                snapshot_sequence: 1,
                players: HashMap::new(),
                sessions: HashMap::new(),
            }),
            latest_snapshot,
        });
        let broadcaster = Arc::clone(&hub);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(u64::from(
                broadcaster.config.broadcast_interval_ms,
            )));
            interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
            interval.tick().await;
            loop {
                interval.tick().await;
                broadcaster.broadcast_once();
            }
        });
        hub
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
                discontinuity_pending: false,
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

    pub(crate) fn subscribe(&self) -> watch::Receiver<Arc<Vec<u8>>> {
        self.latest_snapshot.subscribe()
    }

    pub(crate) fn is_attached(&self, attachment: &PresenceAttachment) -> bool {
        self.lock()
            .players
            .get(&attachment.player_id)
            .is_some_and(|player| {
                player.connection_id == attachment.connection_id
                    && player.session_id == attachment.session_id
                    && player.presence_attached
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
        } else if now.saturating_sub(pose.sample_server_time_ms) > MAX_STALE_SAMPLE_MS {
            return PoseAdmission::IgnoredStale;
        } else if player
            .pose
            .is_some_and(|prior| pose.sample_server_time_ms <= prior.sample_server_time_ms)
        {
            return PoseAdmission::IgnoredStale;
        }
        if let Some(prior) = player.pose {
            let distance_squared = pose
                .eye_position_metres
                .into_iter()
                .zip(prior.eye_position_metres)
                .map(|(next, prior)| {
                    let delta = next - prior;
                    delta * delta
                })
                .sum::<f32>();
            let teleport = f32::from(self.config.teleport_distance_metres);
            if distance_squared > teleport * teleport && pose.flags & PLAYER_POSE_DISCONTINUITY == 0
            {
                return PoseAdmission::Invalid(
                    "large player position change lacks discontinuity flag",
                );
            }
        }
        player.discontinuity_pending |= pose.flags & PLAYER_POSE_DISCONTINUITY != 0;
        if player.discontinuity_pending {
            pose.flags |= PLAYER_POSE_DISCONTINUITY;
        }
        player.pose = Some(pose);
        player.last_pose_receipt_ms = now;
        PoseAdmission::Accepted
    }

    fn broadcast_once(&self) {
        let now = self.now_ms();
        let snapshot = {
            let mut inner = self.lock();
            inner.snapshot_sequence = inner.snapshot_sequence.wrapping_add(1).max(1);
            let snapshot_sequence = inner.snapshot_sequence;
            let mut players = inner
                .players
                .iter_mut()
                .filter_map(|(player_id, player)| {
                    if !player.presence_attached {
                        return None;
                    }
                    let pose = player.pose?;
                    player.discontinuity_pending = false;
                    if let Some(stored) = player.pose.as_mut() {
                        stored.flags &= !PLAYER_POSE_DISCONTINUITY;
                    }
                    Some(PlayerPresenceState {
                        player_id: *player_id,
                        connection_id: player.connection_id,
                        color_index: player.color_index,
                        pose,
                    })
                })
                .collect::<Vec<_>>();
            players.sort_unstable_by_key(|player| player.player_id);
            PresenceSnapshot {
                snapshot_sequence,
                server_time_ms: now,
                players,
            }
        };
        if let Ok(bytes) = encode_presence_snapshot(&snapshot) {
            self.latest_snapshot.send_replace(Arc::new(bytes));
        }
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
        inner.players.remove(&player_id);
        inner.sessions.remove(&session_id);
    }

    fn detach_presence(
        &self,
        player_id: PlayerId,
        connection_id: u64,
        session_id: PresenceSessionId,
    ) {
        let mut inner = self.lock();
        if let Some(player) = inner.players.get_mut(&player_id)
            && player.connection_id == connection_id
            && player.session_id == session_id
        {
            player.presence_attached = false;
            player.pose = None;
            player.discontinuity_pending = false;
        }
    }
}

impl Drop for WorldPresenceClaim {
    fn drop(&mut self) {
        self.hub.leave_world(self.player_id, self.connection_id);
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
) -> Option<u8> {
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
        .map(|offset| ((start + u64::from(offset)) % u64::from(max_players)) as u8)
        .find(|candidate| !used.contains(candidate))
}

#[cfg(test)]
mod tests {
    use super::*;
    use voxels_world::protocol::{BrowserUserId, PLAYER_POSE_GROUNDED, decode_presence_snapshot};

    fn identity(seed: u8) -> PlayerIdentity {
        PlayerIdentity {
            browser_user_id: BrowserUserId::from_bytes([seed; 16]),
            player_id: PlayerId::from_bytes([seed.wrapping_add(1); 16]),
            player_name: format!("player-{seed}"),
        }
    }

    fn pose(sequence: u64, x: f32) -> PlayerPoseUpdate {
        PlayerPoseUpdate {
            sequence,
            sample_server_time_ms: 0,
            eye_position_metres: [x, 1.62, 0.0],
            linear_velocity_metres_per_second: [1.0, 0.0, 0.0],
            look_yaw_radians: 0.0,
            look_pitch_radians: 0.0,
            flags: PLAYER_POSE_GROUNDED,
        }
    }

    #[tokio::test]
    async fn complete_snapshots_are_unique_coalesced_and_disconnect_safe() {
        let hub = PresenceHub::new(PresenceConfig {
            max_pose_updates_per_second: 120,
            ..PresenceConfig::default()
        });
        let first_claim = hub.join(&identity(1)).expect("first player");
        let second_claim = hub.join(&identity(2)).expect("second player");
        assert_ne!(first_claim.connection_id, second_claim.connection_id);
        let first = hub.attach(first_claim.session_id).expect("first attach");
        let second = hub.attach(second_claim.session_id).expect("second attach");
        assert_eq!(
            hub.accept_pose(&first, pose(1, 0.0)),
            PoseAdmission::Accepted
        );
        assert_eq!(
            hub.accept_pose(&second, pose(1, 2.0)),
            PoseAdmission::Accepted
        );
        assert_eq!(
            hub.accept_pose(&first, pose(1, 3.0)),
            PoseAdmission::IgnoredStale
        );

        hub.broadcast_once();
        let bytes = hub.latest_snapshot.borrow().clone();
        let snapshot = decode_presence_snapshot(bytes.as_ref()).expect("snapshot");
        assert_eq!(snapshot.players.len(), 2);
        assert_ne!(
            snapshot.players[0].color_index,
            snapshot.players[1].color_index
        );

        drop(second_claim);
        assert!(!hub.is_attached(&second));
        hub.broadcast_once();
        let bytes = hub.latest_snapshot.borrow().clone();
        let snapshot = decode_presence_snapshot(bytes.as_ref()).expect("snapshot after leave");
        assert_eq!(snapshot.players.len(), 1);
        assert_eq!(snapshot.players[0].connection_id, first_claim.connection_id);
    }

    #[tokio::test]
    async fn large_motion_requires_a_discontinuity() {
        let hub = PresenceHub::new(PresenceConfig {
            max_pose_updates_per_second: 120,
            ..PresenceConfig::default()
        });
        let claim = hub.join(&identity(1)).expect("player");
        let attachment = hub.attach(claim.session_id).expect("attach");
        assert_eq!(
            hub.accept_pose(&attachment, pose(1, 0.0)),
            PoseAdmission::Accepted
        );
        // Avoid exercising the rate limiter in this focused semantic test.
        hub.lock()
            .players
            .get_mut(&attachment.player_id)
            .expect("player")
            .last_pose_receipt_ms = 0;
        assert!(matches!(
            hub.accept_pose(&attachment, pose(2, 20.0)),
            PoseAdmission::Invalid(_)
        ));
        let mut teleported = pose(2, 20.0);
        teleported.flags |= PLAYER_POSE_DISCONTINUITY;
        assert_eq!(
            hub.accept_pose(&attachment, teleported),
            PoseAdmission::Accepted
        );
    }
}
