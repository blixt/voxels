//! Portable simulation and player state. GPU, world generation, and browser concerns live elsewhere.

use glam::{Vec2, Vec3};

pub const PLAYER_RADIUS_METRES: f32 = 0.28;
pub const PLAYER_HEIGHT_METRES: f32 = 1.78;
pub const PLAYER_EYE_HEIGHT_METRES: f32 = 1.62;
const WALK_SPEED: f32 = 4.6;
const SPRINT_MULTIPLIER: f32 = 1.55;
const JUMP_SPEED: f32 = 5.6;
const GRAVITY: f32 = 19.5;
const STEP_HEIGHT: f32 = 0.35;
const COLLISION_EPSILON: f32 = 0.0001;

#[derive(Clone, Copy, Debug, Default)]
pub struct InputState {
    forward: bool,
    left: bool,
    backward: bool,
    right: bool,
    jump: bool,
    sprint: bool,
}

impl InputState {
    pub fn set_key(&mut self, code: u8, pressed: bool) {
        match code {
            1 => self.forward = pressed,
            2 => self.left = pressed,
            3 => self.backward = pressed,
            4 => self.right = pressed,
            5 => self.jump = pressed,
            6 => self.sprint = pressed,
            _ => {}
        }
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

/// The camera position is the player's eye position in metres. The collision capsule is approximated
/// by an axis-aligned box because the world itself is axis-aligned and block edits need deterministic
/// contact behavior.
#[derive(Clone, Copy, Debug)]
pub struct CameraState {
    pub position: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub velocity: Vec3,
    pub grounded: bool,
    jump_was_down: bool,
}

impl Default for CameraState {
    fn default() -> Self {
        Self::spawn(Vec3::new(0.0, 3.2, 5.2))
    }
}

impl CameraState {
    pub fn spawn(position: Vec3) -> Self {
        Self {
            position,
            yaw: 0.0,
            pitch: -0.18,
            velocity: Vec3::ZERO,
            grounded: false,
            jump_was_down: false,
        }
    }

    pub fn from_persisted(position: Vec3, yaw: f32, pitch: f32) -> Self {
        Self {
            position,
            yaw,
            pitch: pitch.clamp(-1.5, 1.5),
            velocity: Vec3::ZERO,
            grounded: false,
            jump_was_down: false,
        }
    }

    pub fn look(&mut self, delta: Vec2) {
        const SENSITIVITY: f32 = 0.0022;
        // Pointer movement right turns right; browser Y grows downward, so moving up raises pitch.
        self.yaw += delta.x * SENSITIVITY;
        self.pitch = (self.pitch - delta.y * SENSITIVITY).clamp(-1.5, 1.5);
    }

    pub fn forward(self) -> Vec3 {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        Vec3::new(sin_yaw * cos_pitch, sin_pitch, -cos_yaw * cos_pitch).normalize()
    }

    pub fn update(
        &mut self,
        input: &InputState,
        dt: f32,
        voxel_size: f32,
        is_solid: impl Fn(i32, i32, i32) -> bool,
    ) {
        let dt = dt.clamp(0.0, 0.05);
        let forward = Vec3::new(self.yaw.sin(), 0.0, -self.yaw.cos());
        let right = Vec3::new(-forward.z, 0.0, forward.x);
        let mut wish = Vec3::ZERO;
        if input.forward {
            wish += forward;
        }
        if input.backward {
            wish -= forward;
        }
        if input.right {
            wish += right;
        }
        if input.left {
            wish -= right;
        }
        let speed = WALK_SPEED * if input.sprint { SPRINT_MULTIPLIER } else { 1.0 };
        let target = wish.normalize_or_zero() * speed;
        let response = 1.0 - (-(if self.grounded { 18.0 } else { 5.0 }) * dt).exp();
        self.velocity.x += (target.x - self.velocity.x) * response;
        self.velocity.z += (target.z - self.velocity.z) * response;

        let jump_pressed = input.jump && !self.jump_was_down;
        self.jump_was_down = input.jump;
        if self.grounded && jump_pressed {
            self.velocity.y = JUMP_SPEED;
            self.grounded = false;
        }
        self.velocity.y -= GRAVITY * dt;

        let horizontal_grounded = self.grounded;
        self.move_axis(
            0,
            self.velocity.x * dt,
            voxel_size,
            horizontal_grounded,
            &is_solid,
        );
        self.move_axis(
            2,
            self.velocity.z * dt,
            voxel_size,
            horizontal_grounded,
            &is_solid,
        );
        self.grounded = false;
        self.move_axis(1, self.velocity.y * dt, voxel_size, false, &is_solid);
        if !self.grounded
            && collides(
                self.position - Vec3::Y * voxel_size * 0.08,
                voxel_size,
                &is_solid,
            )
        {
            self.grounded = true;
            self.velocity.y = self.velocity.y.max(0.0);
        }
    }

    fn move_axis(
        &mut self,
        axis: usize,
        distance: f32,
        voxel_size: f32,
        allow_step: bool,
        is_solid: &impl Fn(i32, i32, i32) -> bool,
    ) {
        if distance.abs() <= f32::EPSILON {
            return;
        }
        let step_count = (distance.abs() / (voxel_size * 0.45)).ceil().max(1.0) as u32;
        let delta = distance / step_count as f32;
        for _ in 0..step_count {
            let mut candidate = self.position;
            candidate[axis] += delta;
            if !collides(candidate, voxel_size, is_solid) {
                self.position = candidate;
                continue;
            }
            if axis != 1 && allow_step {
                let increments = (STEP_HEIGHT / voxel_size).floor() as u32;
                let stepped = (1..=increments).find_map(|increment| {
                    let raised = candidate + Vec3::Y * voxel_size * increment as f32;
                    (!collides(raised, voxel_size, is_solid)).then_some(raised)
                });
                if let Some(raised) = stepped {
                    self.position = raised;
                    continue;
                }
            }
            self.velocity[axis] = 0.0;
            if axis == 1 && distance < 0.0 {
                self.grounded = true;
            }
            break;
        }
    }
}

fn collides(
    eye_position: Vec3,
    voxel_size: f32,
    is_solid: &impl Fn(i32, i32, i32) -> bool,
) -> bool {
    let feet_y = eye_position.y - PLAYER_EYE_HEIGHT_METRES;
    let min = Vec3::new(
        eye_position.x - PLAYER_RADIUS_METRES,
        feet_y,
        eye_position.z - PLAYER_RADIUS_METRES,
    );
    let max = Vec3::new(
        eye_position.x + PLAYER_RADIUS_METRES - COLLISION_EPSILON,
        feet_y + PLAYER_HEIGHT_METRES - COLLISION_EPSILON,
        eye_position.z + PLAYER_RADIUS_METRES - COLLISION_EPSILON,
    );
    let min_voxel = (min / voxel_size).floor().as_ivec3();
    let max_voxel = (max / voxel_size).floor().as_ivec3();
    for y in min_voxel.y..=max_voxel.y {
        for z in min_voxel.z..=max_voxel.z {
            for x in min_voxel.x..=max_voxel.x {
                if is_solid(x, y, z) {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_right_turns_camera_right_and_pitch_is_clamped() {
        let mut camera = CameraState::default();
        camera.look(Vec2::new(20.0, 10_000.0));
        assert!(camera.forward().x > 0.0);
        assert_eq!(camera.pitch, -1.5);
        camera.look(Vec2::new(0.0, -20_000.0));
        assert_eq!(camera.pitch, 1.5);
    }

    #[test]
    fn gravity_lands_player_on_ten_centimetre_ground() {
        let mut camera = CameraState::spawn(Vec3::new(0.0, 3.0, 0.0));
        let input = InputState::default();
        for _ in 0..240 {
            camera.update(&input, 1.0 / 120.0, 0.1, |_, y, _| y < 0);
        }
        assert!(camera.grounded);
        assert!((camera.position.y - PLAYER_EYE_HEIGHT_METRES).abs() < 0.011);
    }

    #[test]
    fn player_cannot_walk_through_voxel_wall() {
        let mut camera = CameraState::spawn(Vec3::new(0.0, PLAYER_EYE_HEIGHT_METRES, 0.0));
        camera.grounded = true;
        let mut input = InputState::default();
        input.set_key(4, true);
        for _ in 0..120 {
            camera.update(&input, 1.0 / 60.0, 0.1, |x, y, _| y < 0 || x >= 5);
        }
        assert!(camera.position.x <= 0.5 - PLAYER_RADIUS_METRES + 0.001);
    }

    #[test]
    fn jump_is_edge_triggered() {
        let mut camera = CameraState::spawn(Vec3::new(0.0, PLAYER_EYE_HEIGHT_METRES, 0.0));
        camera.grounded = true;
        let mut input = InputState::default();
        input.set_key(5, true);
        camera.update(&input, 1.0 / 60.0, 0.1, |_, y, _| y < 0);
        assert!(camera.velocity.y > 0.0);
        let initial_velocity = camera.velocity.y;
        camera.grounded = true;
        camera.update(&input, 1.0 / 60.0, 0.1, |_, y, _| y < 0);
        assert!(camera.velocity.y < initial_velocity);
    }
}
