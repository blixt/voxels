use glam::{Vec2, Vec3};

pub const PROFILE_SPEED_METRES_PER_SECOND: f32 = 12.0;
pub const PROFILE_WARMUP_SECONDS: f32 = 30.0;
pub const PROFILE_MEASURE_SECONDS: f32 = 60.0;
pub const PROFILE_TOTAL_SECONDS: f32 = PROFILE_WARMUP_SECONDS + PROFILE_MEASURE_SECONDS;
pub const PROFILE_LOOP_METRES: f32 = PROFILE_SPEED_METRES_PER_SECOND * PROFILE_WARMUP_SECONDS;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ProfilePhase {
    Idle = 0,
    Warmup = 1,
    Measured = 2,
    Drain = 3,
    Complete = 4,
}

#[derive(Clone, Copy, Debug)]
pub struct ProfilePose {
    pub position_xz: Vec2,
    pub yaw: f32,
    pub pitch: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct ProfileAutomation {
    origin: Vec3,
    ticks: u64,
    phase: ProfilePhase,
}

impl Default for ProfileAutomation {
    fn default() -> Self {
        Self {
            origin: Vec3::ZERO,
            ticks: 0,
            phase: ProfilePhase::Idle,
        }
    }
}

impl ProfileAutomation {
    pub fn start(&mut self, origin: Vec3) {
        self.origin = origin;
        self.ticks = 0;
        self.phase = ProfilePhase::Warmup;
    }

    pub fn advance_fixed_step(&mut self) {
        if matches!(self.phase, ProfilePhase::Warmup | ProfilePhase::Measured) {
            self.ticks = self.ticks.saturating_add(1);
            let elapsed = self.elapsed_seconds();
            self.phase = if elapsed >= PROFILE_TOTAL_SECONDS {
                ProfilePhase::Drain
            } else if elapsed >= PROFILE_WARMUP_SECONDS {
                ProfilePhase::Measured
            } else {
                ProfilePhase::Warmup
            };
        }
    }

    pub fn complete_drain(&mut self) {
        if self.phase == ProfilePhase::Drain {
            self.phase = ProfilePhase::Complete;
        }
    }

    pub const fn phase(self) -> ProfilePhase {
        self.phase
    }

    pub fn elapsed_seconds(self) -> f32 {
        self.ticks as f32 / 120.0
    }

    pub fn distance_metres(self) -> f32 {
        self.elapsed_seconds().min(PROFILE_TOTAL_SECONDS) * PROFILE_SPEED_METRES_PER_SECOND
    }

    pub fn pose(self) -> Option<ProfilePose> {
        if !matches!(self.phase, ProfilePhase::Warmup | ProfilePhase::Measured) {
            return None;
        }
        // Warm the exact route measured by the allocator gate. The first 30-second lap discovers
        // the route's geometry; the next two laps prove that evicting and reloading the same
        // canonical/LOD working set reuses GPU pages instead of growing with distance travelled.
        let radius = PROFILE_LOOP_METRES / std::f32::consts::TAU;
        let angle = (self.distance_metres() % PROFILE_LOOP_METRES) / radius;
        let offset = Vec2::new(radius * angle.sin(), radius * (1.0 - angle.cos()));
        let direction = Vec2::new(angle.cos(), angle.sin());
        Some(ProfilePose {
            position_xz: Vec2::new(self.origin.x, self.origin.z) + offset,
            yaw: direction.x.atan2(-direction.y),
            pitch: -0.22,
        })
    }

    pub fn running(self) -> bool {
        !matches!(self.phase, ProfilePhase::Idle | ProfilePhase::Complete)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_rail_warms_one_lap_then_measures_two_more() {
        let mut profile = ProfileAutomation::default();
        profile.start(Vec3::new(2.0, 3.0, -4.0));
        assert_eq!(profile.phase(), ProfilePhase::Warmup);
        for _ in 0..(120 * 30) {
            profile.advance_fixed_step();
        }
        assert_eq!(profile.phase(), ProfilePhase::Measured);
        let warm_pose = profile.pose().expect("measured lap has a pose");
        assert!((warm_pose.position_xz - Vec2::new(2.0, -4.0)).length() < 0.001);
        for _ in 0..(120 * 60) {
            profile.advance_fixed_step();
        }
        assert_eq!(profile.phase(), ProfilePhase::Drain);
        assert!((profile.distance_metres() - 1_080.0).abs() < 0.01);
        assert!(profile.pose().is_none());
        profile.complete_drain();
        assert_eq!(profile.phase(), ProfilePhase::Complete);
        assert!(!profile.running());
    }

    #[test]
    fn rail_pose_is_stable_for_the_same_fixed_tick() {
        let mut left = ProfileAutomation::default();
        let mut right = ProfileAutomation::default();
        left.start(Vec3::ZERO);
        right.start(Vec3::ZERO);
        for _ in 0..1_337 {
            left.advance_fixed_step();
            right.advance_fixed_step();
        }
        let Some(left) = left.pose() else {
            panic!("rail should still be active");
        };
        let Some(right) = right.pose() else {
            panic!("rail should still be active");
        };
        assert_eq!(left.position_xz, right.position_xz);
        assert_eq!(left.yaw, right.yaw);
    }
}
