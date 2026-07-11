//! Portable game state for Voxels. GPU and browser concerns belong in sibling crates.

use glam::{Vec2, Vec3};

/// Snapshot-friendly player/camera state. Movement semantics stay in Rust even though raw input is
/// captured by the browser harness.
#[derive(Clone, Copy, Debug)]
pub struct CameraState {
    pub position: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub velocity: Vec3,
}

impl Default for CameraState {
    fn default() -> Self {
        Self {
            position: Vec3::new(0.0, 18.0, 28.0),
            yaw: 0.0,
            pitch: -0.28,
            velocity: Vec3::ZERO,
        }
    }
}

impl CameraState {
    pub fn look(&mut self, delta: Vec2) {
        const SENSITIVITY: f32 = 0.0022;
        self.yaw -= delta.x * SENSITIVITY;
        self.pitch = (self.pitch - delta.y * SENSITIVITY).clamp(-1.5, 1.5);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_pitch_is_clamped() {
        let mut camera = CameraState::default();
        camera.look(Vec2::new(0.0, 10_000.0));
        assert_eq!(camera.pitch, -1.5);
        camera.look(Vec2::new(0.0, -20_000.0));
        assert_eq!(camera.pitch, 1.5);
    }
}
