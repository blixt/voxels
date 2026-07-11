//! Portable game state for Voxels. GPU and browser concerns belong in sibling crates.

use glam::{Vec2, Vec3};

#[derive(Clone, Copy, Debug, Default)]
pub struct InputState {
    forward: bool,
    left: bool,
    backward: bool,
    right: bool,
    up: bool,
    down: bool,
}

impl InputState {
    pub fn set_key(&mut self, code: u8, pressed: bool) {
        match code {
            1 => self.forward = pressed,
            2 => self.left = pressed,
            3 => self.backward = pressed,
            4 => self.right = pressed,
            5 => self.up = pressed,
            6 => self.down = pressed,
            _ => {}
        }
    }
}

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
            position: Vec3::new(0.0, 28.0, 52.0),
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

    pub fn forward(self) -> Vec3 {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        Vec3::new(sin_yaw * cos_pitch, sin_pitch, -cos_yaw * cos_pitch).normalize()
    }

    pub fn update(&mut self, input: &InputState, dt: f32) {
        let forward = Vec3::new(self.yaw.sin(), 0.0, -self.yaw.cos());
        let right = Vec3::new(-forward.z, 0.0, forward.x);
        let mut direction = Vec3::ZERO;
        if input.forward {
            direction += forward;
        }
        if input.backward {
            direction -= forward;
        }
        if input.right {
            direction += right;
        }
        if input.left {
            direction -= right;
        }
        if input.up {
            direction += Vec3::Y;
        }
        if input.down {
            direction -= Vec3::Y;
        }
        let target = direction.normalize_or_zero() * 18.0;
        let response = 1.0 - (-10.0 * dt.min(0.1)).exp();
        self.velocity = self.velocity.lerp(target, response);
        self.position += self.velocity * dt.min(0.1);
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

    #[test]
    fn forward_input_moves_toward_negative_z() {
        let mut camera = CameraState::default();
        let start = camera.position;
        let mut input = InputState::default();
        input.set_key(1, true);
        camera.update(&input, 0.1);
        assert!(camera.position.z < start.z);
        assert_eq!(camera.position.x, start.x);
    }
}
