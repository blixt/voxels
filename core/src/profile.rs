use glam::{Vec2, Vec3};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProfileConfig {
    pub fixed_step_seconds: f32,
    pub speed_metres_per_second: f32,
    pub warmup_seconds: f32,
    pub measure_seconds: f32,
}

impl Default for ProfileConfig {
    fn default() -> Self {
        Self {
            fixed_step_seconds: 1.0 / 120.0,
            speed_metres_per_second: 12.0,
            warmup_seconds: 30.0,
            measure_seconds: 60.0,
        }
    }
}

impl ProfileConfig {
    fn total_seconds(self) -> f32 {
        self.warmup_seconds + self.measure_seconds
    }

    fn loop_metres(self) -> f32 {
        self.speed_metres_per_second * self.warmup_seconds
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ProfilePhase {
    Idle = 0,
    Warmup = 1,
    Measured = 2,
    Drain = 3,
    Complete = 4,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ProfileRoute {
    #[default]
    Loop,
    Straight,
}

#[derive(Clone, Copy, Debug)]
pub struct ProfilePose {
    pub position_xz: Vec2,
    pub yaw: f32,
    pub pitch: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct ProfileAutomation {
    config: ProfileConfig,
    origin: Vec3,
    ticks: u64,
    phase: ProfilePhase,
    route: ProfileRoute,
}

impl Default for ProfileAutomation {
    fn default() -> Self {
        Self {
            config: ProfileConfig::default(),
            origin: Vec3::ZERO,
            ticks: 0,
            phase: ProfilePhase::Idle,
            route: ProfileRoute::Loop,
        }
    }
}

impl ProfileAutomation {
    pub fn with_config(config: ProfileConfig) -> Self {
        debug_assert!(config.fixed_step_seconds.is_finite() && config.fixed_step_seconds > 0.0);
        debug_assert!(config.speed_metres_per_second.is_finite());
        debug_assert!(config.speed_metres_per_second > 0.0);
        debug_assert!(config.warmup_seconds.is_finite() && config.warmup_seconds > 0.0);
        debug_assert!(config.measure_seconds.is_finite() && config.measure_seconds >= 0.0);
        Self {
            config,
            ..Self::default()
        }
    }

    pub fn start(&mut self, origin: Vec3) {
        self.start_route(origin, ProfileRoute::Loop);
    }

    pub fn start_route(&mut self, origin: Vec3, route: ProfileRoute) {
        self.origin = origin;
        self.ticks = 0;
        self.phase = ProfilePhase::Warmup;
        self.route = route;
    }

    pub fn advance_fixed_step(&mut self) {
        if matches!(self.phase, ProfilePhase::Warmup | ProfilePhase::Measured) {
            self.ticks = self.ticks.saturating_add(1);
            let elapsed = self.elapsed_seconds();
            self.phase = if elapsed >= self.config.total_seconds() {
                ProfilePhase::Drain
            } else if elapsed >= self.config.warmup_seconds {
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
        self.ticks as f32 * self.config.fixed_step_seconds
    }

    pub fn distance_metres(self) -> f32 {
        self.elapsed_seconds().min(self.config.total_seconds())
            * self.config.speed_metres_per_second
    }

    pub fn pose(self) -> Option<ProfilePose> {
        if !matches!(self.phase, ProfilePhase::Warmup | ProfilePhase::Measured) {
            return None;
        }
        if self.route == ProfileRoute::Straight {
            return Some(ProfilePose {
                position_xz: Vec2::new(self.origin.x, self.origin.z - self.distance_metres()),
                yaw: 0.0,
                pitch: -0.22,
            });
        }
        // Warm the exact route measured by the allocator gate. The configured warmup completes one
        // lap and discovers its geometry; measurement then revisits the same canonical/LOD working
        // set so allocator growth can be distinguished from page reuse.
        let loop_metres = self.config.loop_metres();
        let radius = loop_metres / std::f32::consts::TAU;
        let angle = (self.distance_metres() % loop_metres) / radius;
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
        let config = ProfileConfig {
            fixed_step_seconds: 0.25,
            speed_metres_per_second: 12.0,
            warmup_seconds: 2.0,
            measure_seconds: 4.0,
        };
        let mut profile = ProfileAutomation::with_config(config);
        profile.start(Vec3::new(2.0, 3.0, -4.0));
        assert_eq!(profile.phase(), ProfilePhase::Warmup);
        for _ in 0..8 {
            profile.advance_fixed_step();
        }
        assert_eq!(profile.phase(), ProfilePhase::Measured);
        let warm_pose = profile.pose().expect("measured lap has a pose");
        assert!((warm_pose.position_xz - Vec2::new(2.0, -4.0)).length() < 0.001);
        for _ in 0..16 {
            profile.advance_fixed_step();
        }
        assert_eq!(profile.phase(), ProfilePhase::Drain);
        assert!((profile.distance_metres() - 72.0).abs() < 0.01);
        assert!(profile.pose().is_none());
        profile.complete_drain();
        assert_eq!(profile.phase(), ProfilePhase::Complete);
        assert!(!profile.running());
    }

    #[test]
    fn rail_pose_is_stable_for_the_same_fixed_tick() {
        let config = ProfileConfig {
            fixed_step_seconds: 0.01,
            speed_metres_per_second: 5.0,
            warmup_seconds: 20.0,
            measure_seconds: 20.0,
        };
        let mut left = ProfileAutomation::with_config(config);
        let mut right = ProfileAutomation::with_config(config);
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

    #[test]
    fn straight_route_never_revisits_streamed_world() {
        let config = ProfileConfig {
            fixed_step_seconds: 0.25,
            speed_metres_per_second: 8.0,
            warmup_seconds: 2.0,
            measure_seconds: 4.0,
        };
        let mut profile = ProfileAutomation::with_config(config);
        profile.start_route(Vec3::new(2.0, 3.0, -4.0), ProfileRoute::Straight);
        for _ in 0..24 {
            profile.advance_fixed_step();
        }
        assert_eq!(profile.phase(), ProfilePhase::Drain);
        assert!((profile.distance_metres() - 48.0).abs() < 0.01);
        assert!(profile.pose().is_none());

        let mut moving = ProfileAutomation::with_config(config);
        moving.start_route(Vec3::new(2.0, 3.0, -4.0), ProfileRoute::Straight);
        for _ in 0..12 {
            moving.advance_fixed_step();
        }
        let pose = moving.pose().expect("straight route is still moving");
        assert_eq!(pose.position_xz, Vec2::new(2.0, -28.0));
        assert_eq!(pose.yaw, 0.0);
    }
}
