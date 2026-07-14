//! Transport-neutral remote-player interpolation and animation state.
//!
//! Complete server rosters are sampled on a deliberately delayed server timeline. Each player has
//! a small ordered pose history; rendering uses bounded Hermite interpolation, then at most a short
//! velocity extrapolation. Animation is distance-driven so gait cadence does not depend on packet
//! or display frequency.

use glam::Vec3;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub const REMOTE_POSE_DISCONTINUITY: u16 = 1 << 2;
const MAX_SAMPLES_PER_PLAYER: usize = 32;
const MOVING_SPEED_METRES_PER_SECOND: f32 = 0.18;
const GAIT_STRIDE_METRES: f32 = 1.30;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RemotePlayerId(pub [u8; 16]);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RemotePoseSample {
    pub player_id: RemotePlayerId,
    pub connection_id: u64,
    pub color_index: u8,
    pub sequence: u64,
    pub sample_server_time_ms: u64,
    pub eye_position_metres: Vec3,
    pub linear_velocity_metres_per_second: Vec3,
    pub look_yaw_radians: f32,
    pub look_pitch_radians: f32,
    pub flags: u16,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RemotePresenceSnapshot {
    pub snapshot_sequence: u64,
    pub server_time_ms: u64,
    pub players: Vec<RemotePoseSample>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RemoteAvatarPose {
    pub player_id: RemotePlayerId,
    pub connection_id: u64,
    pub color_index: u8,
    pub eye_position_metres: Vec3,
    pub linear_velocity_metres_per_second: Vec3,
    pub look_yaw_radians: f32,
    pub look_pitch_radians: f32,
    pub body_yaw_radians: f32,
    pub head_yaw_radians: f32,
    pub gait_phase_radians: f32,
    pub locomotion_speed_metres_per_second: f32,
    pub flags: u16,
    pub extrapolated: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PresenceInterpolationConfig {
    pub initial_delay_ms: u16,
    pub min_delay_ms: u16,
    pub max_delay_ms: u16,
    pub max_extrapolation_ms: u16,
}

impl PresenceInterpolationConfig {
    pub fn validate(self) -> Result<Self, &'static str> {
        if self.min_delay_ms == 0
            || self.min_delay_ms > self.initial_delay_ms
            || self.initial_delay_ms > self.max_delay_ms
            || self.max_delay_ms > 1_000
        {
            return Err("presence interpolation delays are inconsistent");
        }
        if self.max_extrapolation_ms > 250 {
            return Err("presence extrapolation exceeds the 250 ms safety bound");
        }
        Ok(self)
    }
}

impl Default for PresenceInterpolationConfig {
    fn default() -> Self {
        Self {
            initial_delay_ms: 100,
            min_delay_ms: 67,
            max_delay_ms: 200,
            max_extrapolation_ms: 100,
        }
    }
}

pub struct RemotePresenceTimeline {
    config: PresenceInterpolationConfig,
    tracks: BTreeMap<RemotePlayerId, PlayerTrack>,
    latest_snapshot_sequence: u64,
    prior_snapshot_server_time_ms: Option<u64>,
    prior_snapshot_receive_time_ms: Option<f64>,
    jitter_ms: f32,
    interpolation_delay_ms: f32,
}

struct PlayerTrack {
    connection_id: u64,
    color_index: u8,
    samples: VecDeque<TimedPose>,
    body_yaw_radians: Option<f32>,
    smoothed_speed: f32,
}

#[derive(Clone, Copy)]
struct TimedPose {
    pose: RemotePoseSample,
    locomotion_distance_metres: f32,
}

#[derive(Clone, Copy)]
struct SampledPose {
    eye_position_metres: Vec3,
    velocity: Vec3,
    look_yaw_radians: f32,
    look_pitch_radians: f32,
    locomotion_distance_metres: f32,
    flags: u16,
    extrapolated: bool,
}

impl RemotePresenceTimeline {
    pub fn new(config: PresenceInterpolationConfig) -> Result<Self, &'static str> {
        let config = config.validate()?;
        Ok(Self {
            config,
            tracks: BTreeMap::new(),
            latest_snapshot_sequence: 0,
            prior_snapshot_server_time_ms: None,
            prior_snapshot_receive_time_ms: None,
            jitter_ms: 0.0,
            interpolation_delay_ms: f32::from(config.initial_delay_ms),
        })
    }

    pub fn clear(&mut self) {
        self.tracks.clear();
        self.latest_snapshot_sequence = 0;
        self.prior_snapshot_server_time_ms = None;
        self.prior_snapshot_receive_time_ms = None;
        self.jitter_ms = 0.0;
        self.interpolation_delay_ms = f32::from(self.config.initial_delay_ms);
    }

    pub fn interpolation_delay_ms(&self) -> f32 {
        self.interpolation_delay_ms
    }

    /// Ingests one complete server roster. Missing players are removed immediately; old or
    /// reordered snapshots cannot resurrect them.
    pub fn ingest_snapshot(
        &mut self,
        snapshot: RemotePresenceSnapshot,
        receive_time_ms: f64,
    ) -> bool {
        if snapshot.snapshot_sequence <= self.latest_snapshot_sequence
            || snapshot.server_time_ms == 0
            || !receive_time_ms.is_finite()
        {
            return false;
        }
        if let (Some(prior_server), Some(prior_receive)) = (
            self.prior_snapshot_server_time_ms,
            self.prior_snapshot_receive_time_ms,
        ) {
            let server_delta = snapshot.server_time_ms.saturating_sub(prior_server) as f64;
            let receive_delta = (receive_time_ms - prior_receive).max(0.0);
            let jitter_sample = (receive_delta - server_delta).abs().min(1_000.0) as f32;
            self.jitter_ms += (jitter_sample - self.jitter_ms) * 0.10;
            let target = (f32::from(self.config.min_delay_ms) + self.jitter_ms * 2.5).clamp(
                f32::from(self.config.min_delay_ms),
                f32::from(self.config.max_delay_ms),
            );
            self.interpolation_delay_ms += (target - self.interpolation_delay_ms) * 0.08;
        }
        self.latest_snapshot_sequence = snapshot.snapshot_sequence;
        self.prior_snapshot_server_time_ms = Some(snapshot.server_time_ms);
        self.prior_snapshot_receive_time_ms = Some(receive_time_ms);

        let mut roster = BTreeSet::new();
        for pose in snapshot.players {
            if !valid_sample(&pose) || !roster.insert(pose.player_id) {
                continue;
            }
            let track = self
                .tracks
                .entry(pose.player_id)
                .or_insert_with(|| PlayerTrack::new(pose));
            if track.connection_id != pose.connection_id {
                *track = PlayerTrack::new(pose);
                continue;
            }
            track.color_index = pose.color_index;
            track.push(pose);
        }
        self.tracks
            .retain(|player_id, _| roster.contains(player_id));
        true
    }

    /// Samples every remote avatar against one receiver-wide presentation clock.
    pub fn sample(
        &mut self,
        estimated_server_time_ms: f64,
        frame_delta_seconds: f32,
    ) -> Vec<RemoteAvatarPose> {
        if !estimated_server_time_ms.is_finite() {
            return Vec::new();
        }
        let target_time_ms = estimated_server_time_ms - f64::from(self.interpolation_delay_ms);
        let frame_delta_seconds = if frame_delta_seconds.is_finite() {
            frame_delta_seconds.clamp(0.0, 0.1)
        } else {
            0.0
        };
        self.tracks
            .iter_mut()
            .filter_map(|(player_id, track)| {
                let sampled =
                    track.sample(target_time_ms, f64::from(self.config.max_extrapolation_ms))?;
                Some(track.animate(*player_id, sampled, frame_delta_seconds))
            })
            .collect()
    }
}

impl PlayerTrack {
    fn new(pose: RemotePoseSample) -> Self {
        let mut track = Self {
            connection_id: pose.connection_id,
            color_index: pose.color_index,
            samples: VecDeque::new(),
            body_yaw_radians: None,
            smoothed_speed: 0.0,
        };
        track.push(pose);
        track
    }

    fn push(&mut self, pose: RemotePoseSample) {
        if pose.flags & REMOTE_POSE_DISCONTINUITY != 0 {
            self.samples.clear();
            self.body_yaw_radians = Some(pose.look_yaw_radians);
        }
        if self.samples.back().is_some_and(|prior| {
            pose.sequence <= prior.pose.sequence
                || pose.sample_server_time_ms <= prior.pose.sample_server_time_ms
        }) {
            return;
        }
        let distance = self.samples.back().map_or(0.0, |prior| {
            let delta = pose.eye_position_metres - prior.pose.eye_position_metres;
            prior.locomotion_distance_metres + Vec3::new(delta.x, 0.0, delta.z).length()
        });
        self.samples.push_back(TimedPose {
            pose,
            locomotion_distance_metres: distance,
        });
        while self.samples.len() > MAX_SAMPLES_PER_PLAYER {
            self.samples.pop_front();
        }
    }

    fn sample(&self, target_time_ms: f64, max_extrapolation_ms: f64) -> Option<SampledPose> {
        let first = *self.samples.front()?;
        if target_time_ms <= first.pose.sample_server_time_ms as f64 {
            return Some(SampledPose::from_timed(first, false));
        }
        for (first, second) in self.samples.iter().zip(self.samples.iter().skip(1)) {
            let first_time = first.pose.sample_server_time_ms as f64;
            let second_time = second.pose.sample_server_time_ms as f64;
            if target_time_ms <= second_time {
                let duration_seconds = ((second_time - first_time) / 1_000.0) as f32;
                let amount = ((target_time_ms - first_time) / (second_time - first_time)) as f32;
                let eye_position_metres = hermite_position(
                    first.pose.eye_position_metres,
                    first.pose.linear_velocity_metres_per_second,
                    second.pose.eye_position_metres,
                    second.pose.linear_velocity_metres_per_second,
                    duration_seconds,
                    amount,
                );
                return Some(SampledPose {
                    eye_position_metres,
                    velocity: first
                        .pose
                        .linear_velocity_metres_per_second
                        .lerp(second.pose.linear_velocity_metres_per_second, amount),
                    look_yaw_radians: lerp_angle(
                        first.pose.look_yaw_radians,
                        second.pose.look_yaw_radians,
                        amount,
                    ),
                    look_pitch_radians: first.pose.look_pitch_radians
                        + (second.pose.look_pitch_radians - first.pose.look_pitch_radians) * amount,
                    locomotion_distance_metres: first.locomotion_distance_metres
                        + (second.locomotion_distance_metres - first.locomotion_distance_metres)
                            * amount,
                    flags: second.pose.flags,
                    extrapolated: false,
                });
            }
        }
        let last = *self.samples.back()?;
        let elapsed_ms = (target_time_ms - last.pose.sample_server_time_ms as f64).max(0.0);
        let extrapolation_ms = elapsed_ms.min(max_extrapolation_ms);
        let extrapolation_seconds = (extrapolation_ms / 1_000.0) as f32;
        let velocity = last.pose.linear_velocity_metres_per_second;
        let horizontal_speed = Vec3::new(velocity.x, 0.0, velocity.z).length();
        Some(SampledPose {
            eye_position_metres: last.pose.eye_position_metres + velocity * extrapolation_seconds,
            velocity,
            look_yaw_radians: last.pose.look_yaw_radians,
            look_pitch_radians: last.pose.look_pitch_radians,
            locomotion_distance_metres: last.locomotion_distance_metres
                + horizontal_speed * extrapolation_seconds,
            flags: last.pose.flags,
            extrapolated: extrapolation_ms > 0.0 && elapsed_ms <= max_extrapolation_ms,
        })
    }

    fn animate(
        &mut self,
        player_id: RemotePlayerId,
        sampled: SampledPose,
        frame_delta_seconds: f32,
    ) -> RemoteAvatarPose {
        let horizontal_velocity = Vec3::new(sampled.velocity.x, 0.0, sampled.velocity.z);
        let speed = horizontal_velocity.length();
        let speed_alpha = 1.0 - (-10.0 * frame_delta_seconds).exp();
        self.smoothed_speed += (speed - self.smoothed_speed) * speed_alpha;

        let mut body_yaw = self.body_yaw_radians.unwrap_or(sampled.look_yaw_radians);
        let look_delta = angle_delta(body_yaw, sampled.look_yaw_radians);
        let mut target_body_yaw = body_yaw;
        let moving = self.smoothed_speed > MOVING_SPEED_METRES_PER_SECOND;
        if moving && speed > 0.01 {
            let movement_yaw = horizontal_velocity.x.atan2(-horizontal_velocity.z);
            let movement_from_look = angle_delta(sampled.look_yaw_radians, movement_yaw).abs();
            target_body_yaw = if movement_from_look < 1.75 {
                movement_yaw
            } else {
                sampled.look_yaw_radians
            };
        }
        if look_delta.abs() > 0.96 {
            target_body_yaw = sampled.look_yaw_radians - look_delta.signum() * 0.52;
        }
        let follow_rate = if moving { 9.0 } else { 4.5 };
        let follow_alpha = 1.0 - (-follow_rate * frame_delta_seconds).exp();
        body_yaw = lerp_angle(body_yaw, target_body_yaw, follow_alpha);
        self.body_yaw_radians = Some(body_yaw);
        let head_yaw = angle_delta(body_yaw, sampled.look_yaw_radians).clamp(-1.13, 1.13);

        RemoteAvatarPose {
            player_id,
            connection_id: self.connection_id,
            color_index: self.color_index,
            eye_position_metres: sampled.eye_position_metres,
            linear_velocity_metres_per_second: sampled.velocity,
            look_yaw_radians: sampled.look_yaw_radians,
            look_pitch_radians: sampled.look_pitch_radians,
            body_yaw_radians: body_yaw,
            head_yaw_radians: head_yaw,
            gait_phase_radians: (sampled.locomotion_distance_metres / GAIT_STRIDE_METRES)
                * std::f32::consts::TAU,
            locomotion_speed_metres_per_second: self.smoothed_speed,
            flags: sampled.flags,
            extrapolated: sampled.extrapolated,
        }
    }
}

impl SampledPose {
    fn from_timed(timed: TimedPose, extrapolated: bool) -> Self {
        Self {
            eye_position_metres: timed.pose.eye_position_metres,
            velocity: timed.pose.linear_velocity_metres_per_second,
            look_yaw_radians: timed.pose.look_yaw_radians,
            look_pitch_radians: timed.pose.look_pitch_radians,
            locomotion_distance_metres: timed.locomotion_distance_metres,
            flags: timed.pose.flags,
            extrapolated,
        }
    }
}

fn valid_sample(sample: &RemotePoseSample) -> bool {
    sample.connection_id != 0
        && sample.sequence != 0
        && sample.sample_server_time_ms != 0
        && sample.eye_position_metres.is_finite()
        && sample.linear_velocity_metres_per_second.is_finite()
        && sample.look_yaw_radians.is_finite()
        && sample.look_pitch_radians.is_finite()
}

fn hermite_position(
    first_position: Vec3,
    first_velocity: Vec3,
    second_position: Vec3,
    second_velocity: Vec3,
    duration_seconds: f32,
    amount: f32,
) -> Vec3 {
    let amount_squared = amount * amount;
    let amount_cubed = amount_squared * amount;
    let result = first_position * (2.0 * amount_cubed - 3.0 * amount_squared + 1.0)
        + first_velocity * duration_seconds * (amount_cubed - 2.0 * amount_squared + amount)
        + second_position * (-2.0 * amount_cubed + 3.0 * amount_squared)
        + second_velocity * duration_seconds * (amount_cubed - amount_squared);
    result.clamp(
        first_position.min(second_position),
        first_position.max(second_position),
    )
}

fn angle_delta(from: f32, to: f32) -> f32 {
    (to - from + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

fn lerp_angle(from: f32, to: f32, amount: f32) -> f32 {
    from + angle_delta(from, to) * amount
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(sequence: u64, time: u64, x: f32) -> RemotePoseSample {
        RemotePoseSample {
            player_id: RemotePlayerId([1; 16]),
            connection_id: 7,
            color_index: 3,
            sequence,
            sample_server_time_ms: time,
            eye_position_metres: Vec3::new(x, 1.62, 0.0),
            linear_velocity_metres_per_second: Vec3::X * 10.0,
            look_yaw_radians: 3.10,
            look_pitch_radians: 0.2,
            flags: 0,
        }
    }

    fn fixed_timeline() -> RemotePresenceTimeline {
        RemotePresenceTimeline::new(PresenceInterpolationConfig {
            initial_delay_ms: 100,
            min_delay_ms: 100,
            max_delay_ms: 100,
            max_extrapolation_ms: 100,
        })
        .expect("timeline")
    }

    #[test]
    fn delayed_timeline_interpolates_and_never_overshoots() {
        let mut timeline = fixed_timeline();
        assert!(timeline.ingest_snapshot(
            RemotePresenceSnapshot {
                snapshot_sequence: 1,
                server_time_ms: 1_000,
                players: vec![sample(1, 1_000, 0.0)],
            },
            1_010.0,
        ));
        assert!(timeline.ingest_snapshot(
            RemotePresenceSnapshot {
                snapshot_sequence: 2,
                server_time_ms: 1_100,
                players: vec![sample(2, 1_100, 1.0)],
            },
            1_118.0,
        ));
        let midpoint = timeline.sample(1_150.0, 1.0 / 60.0);
        assert_eq!(midpoint.len(), 1);
        assert!((midpoint[0].eye_position_metres.x - 0.5).abs() < 0.001);
        assert!((0.0..=1.0).contains(&midpoint[0].eye_position_metres.x));
    }

    #[test]
    fn extrapolation_stops_at_the_configured_horizon() {
        let mut timeline = fixed_timeline();
        timeline.ingest_snapshot(
            RemotePresenceSnapshot {
                snapshot_sequence: 1,
                server_time_ms: 1_000,
                players: vec![sample(1, 1_000, 0.0)],
            },
            1_000.0,
        );
        let at_horizon = timeline.sample(1_200.0, 1.0 / 60.0)[0];
        let well_after = timeline.sample(2_000.0, 1.0 / 60.0)[0];
        assert!((at_horizon.eye_position_metres.x - 1.0).abs() < 0.001);
        assert_eq!(
            well_after.eye_position_metres,
            at_horizon.eye_position_metres
        );
        assert!(!well_after.extrapolated);
    }

    #[test]
    fn discontinuities_reset_history_and_complete_rosters_remove_players() {
        let mut timeline = fixed_timeline();
        timeline.ingest_snapshot(
            RemotePresenceSnapshot {
                snapshot_sequence: 1,
                server_time_ms: 1_000,
                players: vec![sample(1, 1_000, 0.0)],
            },
            1_000.0,
        );
        let mut teleported = sample(2, 1_050, 100.0);
        teleported.flags = REMOTE_POSE_DISCONTINUITY;
        timeline.ingest_snapshot(
            RemotePresenceSnapshot {
                snapshot_sequence: 2,
                server_time_ms: 1_050,
                players: vec![teleported],
            },
            1_050.0,
        );
        assert_eq!(
            timeline.sample(1_100.0, 1.0 / 60.0)[0]
                .eye_position_metres
                .x,
            100.0
        );
        timeline.ingest_snapshot(
            RemotePresenceSnapshot {
                snapshot_sequence: 3,
                server_time_ms: 1_100,
                players: Vec::new(),
            },
            1_100.0,
        );
        assert!(timeline.sample(1_200.0, 1.0 / 60.0).is_empty());
    }

    #[test]
    fn shortest_arc_head_follow_is_nearly_frame_rate_invariant() {
        fn run(frame_delta: f32) -> RemoteAvatarPose {
            let mut timeline = fixed_timeline();
            let mut pose = sample(1, 1_000, 0.0);
            pose.linear_velocity_metres_per_second = Vec3::ZERO;
            pose.look_yaw_radians = -2.8;
            timeline.ingest_snapshot(
                RemotePresenceSnapshot {
                    snapshot_sequence: 1,
                    server_time_ms: 1_000,
                    players: vec![pose],
                },
                1_000.0,
            );
            let frames = (1.0 / frame_delta).round() as usize;
            let mut result = timeline.sample(1_100.0, frame_delta)[0];
            for _ in 1..frames {
                result = timeline.sample(1_100.0, frame_delta)[0];
            }
            result
        }
        let sixty = run(1.0 / 60.0);
        let one_forty_four = run(1.0 / 144.0);
        assert!(angle_delta(sixty.body_yaw_radians, one_forty_four.body_yaw_radians).abs() < 0.01);
        assert!(sixty.head_yaw_radians.abs() <= 1.13);
    }
}
