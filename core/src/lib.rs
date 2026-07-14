//! Portable simulation and player state. GPU, world generation, and browser concerns live elsewhere.

mod presence;
mod profile;

pub use presence::{
    PresenceInterpolationConfig, REMOTE_POSE_DISCONTINUITY, RemoteAvatarPose, RemotePlayerId,
    RemotePoseSample, RemotePresenceSnapshot, RemotePresenceTimeline,
};
pub use profile::{ProfileAutomation, ProfileConfig, ProfilePhase, ProfilePose};

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
const SWIM_SPEED: f32 = 3.2;
const SWIM_RESPONSE: f32 = 5.5;
const SWIM_ENTER_IMMERSION: f32 = 0.52;
const SWIM_EXIT_IMMERSION: f32 = 0.25;
const MAX_FLUID_SURFACE_SCAN_VOXELS: i32 = 256;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnclosureSample {
    pub sky_visibility: f32,
    pub enclosure: f32,
    pub ceiling_distance_metres: f32,
    pub escape_direction: Vec3,
    pub escaped_rays: u8,
    pub ray_count: u8,
}

impl Default for EnclosureSample {
    fn default() -> Self {
        Self::OPEN
    }
}

impl EnclosureSample {
    pub const OPEN: Self = Self {
        sky_visibility: 1.0,
        enclosure: 0.0,
        ceiling_distance_metres: 12.0,
        escape_direction: Vec3::Y,
        escaped_rays: 9,
        ray_count: 9,
    };
}

/// Deterministic upper-hemisphere visibility probe for caves and player-made interiors. The caller
/// supplies canonical occupancy, keeping this portable across browser, native, mobile, and console
/// shells. Runtime grows with crossed 10 cm cells and is intended to be cached at a low frequency.
pub fn probe_enclosure(
    eye: Vec3,
    max_distance_metres: f32,
    voxel_size_metres: f32,
    mut is_opaque: impl FnMut(i32, i32, i32) -> bool,
) -> EnclosureSample {
    const DIRECTIONS: [[f32; 3]; 9] = [
        [0.0, 1.0, 0.0],
        [0.694, 0.720, 0.0],
        [0.491, 0.720, 0.491],
        [0.0, 0.720, 0.694],
        [-0.491, 0.720, 0.491],
        [-0.694, 0.720, 0.0],
        [-0.491, 0.720, -0.491],
        [0.0, 0.720, -0.694],
        [0.491, 0.720, -0.491],
    ];
    if !eye.is_finite()
        || !max_distance_metres.is_finite()
        || max_distance_metres <= 0.0
        || !voxel_size_metres.is_finite()
        || voxel_size_metres <= 0.0
    {
        return EnclosureSample::OPEN;
    }
    let mut escaped = 0u8;
    let mut escape = Vec3::ZERO;
    let mut ceiling = max_distance_metres;
    for (index, direction) in DIRECTIONS.into_iter().enumerate() {
        let direction = Vec3::from_array(direction).normalize();
        let hit = raycast_voxels(
            eye,
            direction,
            max_distance_metres,
            voxel_size_metres,
            &mut is_opaque,
        );
        if let Some(hit) = hit {
            if index == 0 {
                ceiling = hit.distance_metres;
            }
        } else {
            escaped += 1;
            escape += direction;
        }
    }
    let sky_visibility = f32::from(escaped) / DIRECTIONS.len() as f32;
    EnclosureSample {
        sky_visibility,
        enclosure: 1.0 - sky_visibility,
        ceiling_distance_metres: ceiling,
        escape_direction: escape.normalize_or(Vec3::Y),
        escaped_rays: escaped,
        ray_count: DIRECTIONS.len() as u8,
    }
}

/// Host-independent physical meaning of one canonical voxel. `core` deliberately does not depend on
/// the world's material registry; every shell maps its own authoritative material into these traits.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VoxelPhysics {
    pub collidable: bool,
    pub fluid: bool,
}

impl VoxelPhysics {
    pub const EMPTY: Self = Self {
        collidable: false,
        fluid: false,
    };
    pub const SOLID: Self = Self {
        collidable: true,
        fluid: false,
    };
    pub const FLUID: Self = Self {
        collidable: false,
        fluid: true,
    };
}

/// Derived fixed-step environment state. It is recomputed from canonical 10 cm voxels and never
/// persisted, so a saved camera remains portable across generators and native/browser hosts.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FluidState {
    pub immersion: f32,
    pub eyes_submerged: bool,
    pub eye_depth_metres: f32,
    pub surface_y_metres: f32,
    pub swimming: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoxelHit {
    pub voxel: [i32; 3],
    pub adjacent: [i32; 3],
    pub normal: [i32; 3],
    pub distance_metres: f32,
}

/// Amanatides-Woo grid traversal in metric world space. Runtime grows with crossed cells rather
/// than distance-sampling resolution, so picking remains exact at the canonical 10 cm scale.
pub fn raycast_voxels(
    origin: Vec3,
    direction: Vec3,
    max_distance_metres: f32,
    voxel_size_metres: f32,
    mut is_solid: impl FnMut(i32, i32, i32) -> bool,
) -> Option<VoxelHit> {
    if !origin.is_finite()
        || !direction.is_finite()
        || !max_distance_metres.is_finite()
        || max_distance_metres <= 0.0
        || !voxel_size_metres.is_finite()
        || voxel_size_metres <= 0.0
    {
        return None;
    }
    let direction = direction.normalize_or_zero();
    if direction == Vec3::ZERO {
        return None;
    }
    let mut voxel = (origin / voxel_size_metres).floor().as_ivec3();
    if is_solid(voxel.x, voxel.y, voxel.z) {
        return Some(VoxelHit {
            voxel: voxel.to_array(),
            adjacent: voxel.to_array(),
            normal: [0; 3],
            distance_metres: 0.0,
        });
    }
    let axis_step = |value: f32| {
        if value > 0.0 {
            1
        } else if value < 0.0 {
            -1
        } else {
            0
        }
    };
    let step = glam::IVec3::new(
        axis_step(direction.x),
        axis_step(direction.y),
        axis_step(direction.z),
    );
    let mut maximum = Vec3::splat(f32::INFINITY);
    let mut delta = Vec3::splat(f32::INFINITY);
    for axis in 0..3 {
        if step[axis] == 0 {
            continue;
        }
        let boundary_voxel = if step[axis] > 0 {
            voxel[axis].checked_add(1)?
        } else {
            voxel[axis]
        };
        let boundary = boundary_voxel as f32 * voxel_size_metres;
        maximum[axis] = (boundary - origin[axis]) / direction[axis];
        delta[axis] = voxel_size_metres / direction[axis].abs();
    }

    loop {
        let axis = if maximum.x <= maximum.y && maximum.x <= maximum.z {
            0
        } else if maximum.y <= maximum.z {
            1
        } else {
            2
        };
        let distance = maximum[axis];
        if distance > max_distance_metres {
            return None;
        }
        let adjacent = voxel;
        voxel[axis] = voxel[axis].checked_add(step[axis])?;
        let mut normal = [0; 3];
        normal[axis] = -step[axis];
        if is_solid(voxel.x, voxel.y, voxel.z) {
            return Some(VoxelHit {
                voxel: voxel.to_array(),
                adjacent: adjacent.to_array(),
                normal,
                distance_metres: distance,
            });
        }
        maximum[axis] += delta[axis];
    }
}

/// Returns whether the segment between two metric world-space points crosses no occluding voxel.
/// This shares the exact canonical-grid traversal used by picking, but deliberately stops at the
/// target point instead of tracing an unbounded ray. Render hosts can use it for conservative light
/// or portal visibility without coupling portable simulation to a world implementation.
pub fn voxel_segment_is_clear(
    origin: Vec3,
    target: Vec3,
    voxel_size_metres: f32,
    is_occluder: impl FnMut(i32, i32, i32) -> bool,
) -> bool {
    if !voxel_size_metres.is_finite() || voxel_size_metres <= 0.0 {
        return false;
    }
    let offset = target - origin;
    let distance = offset.length();
    distance.is_finite()
        && (distance <= f32::EPSILON
            || raycast_voxels(origin, offset, distance, voxel_size_metres, is_occluder).is_none())
}

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
    fluid: FluidState,
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
            fluid: FluidState::default(),
        }
    }

    pub fn from_persisted(position: Vec3, yaw: f32, pitch: f32) -> Self {
        let fallback = Self::default();
        Self {
            position: if position.is_finite() {
                position
            } else {
                fallback.position
            },
            yaw: normalized_yaw(yaw),
            pitch: if pitch.is_finite() {
                pitch.clamp(-1.5, 1.5)
            } else {
                fallback.pitch
            },
            velocity: Vec3::ZERO,
            grounded: false,
            jump_was_down: false,
            fluid: FluidState::default(),
        }
    }

    pub fn look(&mut self, delta: Vec2) {
        const SENSITIVITY: f32 = 0.0022;
        // Pointer movement right turns right; browser Y grows downward, so moving up raises pitch.
        let yaw = self.yaw + delta.x * SENSITIVITY;
        if yaw.is_finite() {
            self.yaw = normalized_yaw(yaw);
        }
        let pitch = self.pitch - delta.y * SENSITIVITY;
        if pitch.is_finite() {
            self.pitch = pitch.clamp(-1.5, 1.5);
        }
    }

    pub fn forward(self) -> Vec3 {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        Vec3::new(sin_yaw * cos_pitch, sin_pitch, -cos_yaw * cos_pitch).normalize()
    }

    pub const fn fluid_state(self) -> FluidState {
        self.fluid
    }

    pub fn intersects_voxel(self, voxel: [i32; 3], voxel_size: f32) -> bool {
        let feet_y = self.position.y - PLAYER_EYE_HEIGHT_METRES;
        let player_min = Vec3::new(
            self.position.x - PLAYER_RADIUS_METRES,
            feet_y,
            self.position.z - PLAYER_RADIUS_METRES,
        );
        let player_max = Vec3::new(
            self.position.x + PLAYER_RADIUS_METRES,
            feet_y + PLAYER_HEIGHT_METRES,
            self.position.z + PLAYER_RADIUS_METRES,
        );
        let voxel_min = Vec3::from_array(voxel.map(|value| value as f32 * voxel_size));
        let voxel_max = voxel_min + Vec3::splat(voxel_size);
        player_min.cmplt(voxel_max).all() && player_max.cmpgt(voxel_min).all()
    }

    pub fn update(
        &mut self,
        input: &InputState,
        dt: f32,
        voxel_size: f32,
        mut sample_voxel: impl FnMut(i32, i32, i32) -> VoxelPhysics,
    ) {
        if !dt.is_finite() || !voxel_size.is_finite() || voxel_size <= 0.0 {
            return;
        }
        let dt = dt.clamp(0.0, 0.05);
        let was_swimming = self.fluid.swimming;
        let horizontal_grounded;
        if was_swimming {
            let forward = self.forward();
            let horizontal_forward = Vec3::new(self.yaw.sin(), 0.0, -self.yaw.cos());
            let right = Vec3::new(-horizontal_forward.z, 0.0, horizontal_forward.x);
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
            if input.jump {
                wish += Vec3::Y;
            }
            if input.sprint {
                wish -= Vec3::Y;
            }
            let target = wish.normalize_or_zero() * SWIM_SPEED;
            let response = 1.0 - (-SWIM_RESPONSE * dt).exp();
            self.velocity += (target - self.velocity) * response;
            let support = 1.0 - (1.0 - self.fluid.immersion).powi(2);
            self.velocity.y += (-GRAVITY * (1.0 - support) + 0.8 * self.fluid.immersion) * dt;
            self.velocity.y = self.velocity.y.clamp(-4.0, 3.0);
            self.grounded = false;
            self.jump_was_down = input.jump;
            horizontal_grounded = input.jump;
        } else {
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
            horizontal_grounded = self.grounded;
        }

        self.move_axis(
            0,
            self.velocity.x * dt,
            voxel_size,
            horizontal_grounded,
            &mut sample_voxel,
        );
        self.move_axis(
            2,
            self.velocity.z * dt,
            voxel_size,
            horizontal_grounded,
            &mut sample_voxel,
        );
        self.grounded = false;
        self.move_axis(
            1,
            self.velocity.y * dt,
            voxel_size,
            false,
            &mut sample_voxel,
        );
        if !self.grounded
            && collides(
                self.position - Vec3::Y * voxel_size * 0.08,
                voxel_size,
                &mut sample_voxel,
            )
        {
            self.grounded = true;
            self.velocity.y = self.velocity.y.max(0.0);
        }
        self.refresh_fluid_state(voxel_size, &mut sample_voxel);
    }

    /// Refresh derived fluid state without advancing movement, used after restoring or teleporting a
    /// camera so the very first rendered frame already has the correct medium.
    pub fn refresh_fluid_state(
        &mut self,
        voxel_size: f32,
        mut sample_voxel: impl FnMut(i32, i32, i32) -> VoxelPhysics,
    ) {
        if !voxel_size.is_finite() || voxel_size <= 0.0 {
            self.fluid = FluidState::default();
            return;
        }
        let previous_swimming = self.fluid.swimming;
        let (player_min, player_max) = player_bounds(self.position);
        let range_max = player_max - Vec3::splat(COLLISION_EPSILON);
        let min_voxel = (player_min / voxel_size).floor().as_ivec3();
        let max_voxel = (range_max / voxel_size).floor().as_ivec3();
        let mut fluid_volume = 0.0;
        let mut highest_surface = f32::NEG_INFINITY;
        for y in min_voxel.y..=max_voxel.y {
            for z in min_voxel.z..=max_voxel.z {
                for x in min_voxel.x..=max_voxel.x {
                    if !sample_voxel(x, y, z).fluid {
                        continue;
                    }
                    let voxel_min = Vec3::new(x as f32, y as f32, z as f32) * voxel_size;
                    let voxel_max = voxel_min + Vec3::splat(voxel_size);
                    let overlap = player_max.min(voxel_max) - player_min.max(voxel_min);
                    let overlap = overlap.max(Vec3::ZERO);
                    fluid_volume += overlap.x * overlap.y * overlap.z;
                    highest_surface = highest_surface.max(voxel_max.y);
                }
            }
        }
        let player_size = player_max - player_min;
        let player_volume = player_size.x * player_size.y * player_size.z;
        let immersion =
            if fluid_volume.is_finite() && player_volume.is_finite() && player_volume > 0.0 {
                (fluid_volume / player_volume).clamp(0.0, 1.0)
            } else {
                0.0
            };
        let eye_voxel = (self.position / voxel_size).floor().as_ivec3();
        let eyes_submerged = sample_voxel(eye_voxel.x, eye_voxel.y, eye_voxel.z).fluid;
        let surface_y_metres = if eyes_submerged {
            let mut first_air = eye_voxel.y;
            for _ in 0..MAX_FLUID_SURFACE_SCAN_VOXELS {
                if !sample_voxel(eye_voxel.x, first_air, eye_voxel.z).fluid {
                    break;
                }
                let Some(next) = first_air.checked_add(1) else {
                    break;
                };
                first_air = next;
            }
            first_air as f32 * voxel_size
        } else if highest_surface.is_finite() {
            highest_surface
        } else {
            self.position.y
        };
        let swimming = if previous_swimming {
            eyes_submerged || immersion > SWIM_EXIT_IMMERSION
        } else {
            eyes_submerged || immersion >= SWIM_ENTER_IMMERSION
        };
        self.fluid = FluidState {
            immersion,
            eyes_submerged,
            eye_depth_metres: if eyes_submerged {
                (surface_y_metres - self.position.y).max(0.0)
            } else {
                0.0
            },
            surface_y_metres,
            swimming,
        };
    }

    pub fn overlaps_collidable(
        self,
        voxel_size: f32,
        mut sample_voxel: impl FnMut(i32, i32, i32) -> VoxelPhysics,
    ) -> bool {
        !voxel_size.is_finite()
            || voxel_size <= 0.0
            || collides(self.position, voxel_size, &mut sample_voxel)
    }

    fn move_axis(
        &mut self,
        axis: usize,
        distance: f32,
        voxel_size: f32,
        allow_step: bool,
        sample_voxel: &mut impl FnMut(i32, i32, i32) -> VoxelPhysics,
    ) {
        if distance.abs() <= f32::EPSILON {
            return;
        }
        let step_count = (distance.abs() / (voxel_size * 0.45)).ceil().max(1.0) as u32;
        let delta = distance / step_count as f32;
        for _ in 0..step_count {
            let mut candidate = self.position;
            candidate[axis] += delta;
            if !collides(candidate, voxel_size, sample_voxel) {
                self.position = candidate;
                continue;
            }
            if axis != 1 && allow_step {
                let increments = (STEP_HEIGHT / voxel_size).floor() as u32;
                let stepped = (1..=increments).find_map(|increment| {
                    let raised = candidate + Vec3::Y * voxel_size * increment as f32;
                    (!collides(raised, voxel_size, sample_voxel)).then_some(raised)
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

fn normalized_yaw(yaw: f32) -> f32 {
    if yaw.is_finite() {
        (yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
    } else {
        0.0
    }
}

fn player_bounds(eye_position: Vec3) -> (Vec3, Vec3) {
    let feet_y = eye_position.y - PLAYER_EYE_HEIGHT_METRES;
    (
        Vec3::new(
            eye_position.x - PLAYER_RADIUS_METRES,
            feet_y,
            eye_position.z - PLAYER_RADIUS_METRES,
        ),
        Vec3::new(
            eye_position.x + PLAYER_RADIUS_METRES,
            feet_y + PLAYER_HEIGHT_METRES,
            eye_position.z + PLAYER_RADIUS_METRES,
        ),
    )
}

fn collides(
    eye_position: Vec3,
    voxel_size: f32,
    sample_voxel: &mut impl FnMut(i32, i32, i32) -> VoxelPhysics,
) -> bool {
    let (min, max) = player_bounds(eye_position);
    let max = max - Vec3::splat(COLLISION_EPSILON);
    let min_voxel = (min / voxel_size).floor().as_ivec3();
    let max_voxel = (max / voxel_size).floor().as_ivec3();
    for y in min_voxel.y..=max_voxel.y {
        for z in min_voxel.z..=max_voxel.z {
            for x in min_voxel.x..=max_voxel.x {
                if sample_voxel(x, y, z).collidable {
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

    fn solid_if(value: bool) -> VoxelPhysics {
        if value {
            VoxelPhysics::SOLID
        } else {
            VoxelPhysics::EMPTY
        }
    }

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
    fn restored_yaw_stays_bounded_and_retains_mouse_precision() {
        let mut camera = CameraState::from_persisted(Vec3::ZERO, 1.0e30, 0.0);
        assert!((-std::f32::consts::PI..std::f32::consts::PI).contains(&camera.yaw));
        let before = camera.yaw;

        camera.look(Vec2::new(1.0, 0.0));

        let turn = normalized_yaw(camera.yaw - before);
        assert!((turn - 0.0022).abs() < 1.0e-6);
    }

    #[test]
    fn non_finite_persisted_pose_falls_back_to_a_valid_camera() {
        let default = CameraState::default();
        let restored = CameraState::from_persisted(
            Vec3::new(f32::NAN, 42.0, f32::INFINITY),
            f32::NEG_INFINITY,
            f32::NAN,
        );

        assert_eq!(restored.position, default.position);
        assert_eq!(restored.yaw, 0.0);
        assert_eq!(restored.pitch, default.pitch);
        assert!(restored.forward().is_finite());
    }

    #[test]
    fn non_finite_mouse_axes_do_not_poison_camera_angles() {
        let mut camera = CameraState::default();
        let initial_yaw = camera.yaw;
        camera.look(Vec2::new(f32::NAN, 10.0));
        assert_eq!(camera.yaw, initial_yaw);
        assert!(camera.pitch.is_finite());

        let initial_pitch = camera.pitch;
        camera.look(Vec2::new(10.0, f32::INFINITY));
        assert_ne!(camera.yaw, initial_yaw);
        assert_eq!(camera.pitch, initial_pitch);
    }

    #[test]
    fn gravity_lands_player_on_ten_centimetre_ground() {
        let mut camera = CameraState::spawn(Vec3::new(0.0, 3.0, 0.0));
        let input = InputState::default();
        for _ in 0..240 {
            camera.update(&input, 1.0 / 120.0, 0.1, |_, y, _| solid_if(y < 0));
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
            camera.update(&input, 1.0 / 60.0, 0.1, |x, y, _| solid_if(y < 0 || x >= 5));
        }
        assert!(camera.position.x <= 0.5 - PLAYER_RADIUS_METRES + 0.001);
    }

    #[test]
    fn jump_is_edge_triggered() {
        let mut camera = CameraState::spawn(Vec3::new(0.0, PLAYER_EYE_HEIGHT_METRES, 0.0));
        camera.grounded = true;
        let mut input = InputState::default();
        input.set_key(5, true);
        camera.update(&input, 1.0 / 60.0, 0.1, |_, y, _| solid_if(y < 0));
        assert!(camera.velocity.y > 0.0);
        let initial_velocity = camera.velocity.y;
        camera.grounded = true;
        camera.update(&input, 1.0 / 60.0, 0.1, |_, y, _| solid_if(y < 0));
        assert!(camera.velocity.y < initial_velocity);
    }

    #[test]
    fn non_finite_timesteps_do_not_poison_camera_state() {
        for dt in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut camera = CameraState::default();
            let before = camera;
            let mut sampled = false;
            camera.update(&InputState::default(), dt, 0.1, |_, _, _| {
                sampled = true;
                VoxelPhysics::EMPTY
            });
            assert_eq!(camera.position, before.position);
            assert_eq!(camera.yaw, before.yaw);
            assert_eq!(camera.pitch, before.pitch);
            assert_eq!(camera.velocity, before.velocity);
            assert_eq!(camera.grounded, before.grounded);
            assert_eq!(camera.jump_was_down, before.jump_was_down);
            assert_eq!(camera.fluid, before.fluid);
            assert!(!sampled);
        }
    }

    #[test]
    fn dda_hits_ten_centimetre_voxel_and_reports_place_cell() {
        let hit = raycast_voxels(Vec3::new(0.05, 0.05, 0.05), Vec3::X, 1.0, 0.1, |x, y, z| {
            [x, y, z] == [4, 0, 0]
        });
        assert_eq!(
            hit,
            Some(VoxelHit {
                voxel: [4, 0, 0],
                adjacent: [3, 0, 0],
                normal: [-1, 0, 0],
                distance_metres: 0.35,
            })
        );
    }

    #[test]
    fn dda_handles_negative_coordinates_and_range_limit() {
        let hit = raycast_voxels(
            Vec3::new(0.05, 0.05, 0.05),
            -Vec3::X,
            0.3,
            0.1,
            |x, _, _| x == -2,
        );
        assert_eq!(hit.map(|value| value.voxel), Some([-2, 0, 0]));
        assert!(
            raycast_voxels(Vec3::new(0.05, 0.05, 0.05), Vec3::X, 0.2, 0.1, |x, _, _| x
                == 4)
            .is_none()
        );
    }

    #[test]
    fn dda_rejects_non_finite_inputs_without_sampling() {
        let invalid = [
            (Vec3::splat(f32::NAN), Vec3::X, 1.0, 0.1),
            (Vec3::ZERO, Vec3::splat(f32::INFINITY), 1.0, 0.1),
            (Vec3::ZERO, Vec3::X, f32::NAN, 0.1),
            (Vec3::ZERO, Vec3::X, f32::INFINITY, 0.1),
            (Vec3::ZERO, Vec3::X, 1.0, f32::NAN),
            (Vec3::ZERO, Vec3::X, 1.0, f32::INFINITY),
        ];
        for (origin, direction, max_distance, voxel_size) in invalid {
            let mut sampled = false;
            assert_eq!(
                raycast_voxels(origin, direction, max_distance, voxel_size, |_, _, _| {
                    sampled = true;
                    false
                }),
                None
            );
            assert!(!sampled);
        }
    }

    #[test]
    fn dda_does_not_step_or_overflow_zero_direction_axes() {
        let extreme_x = i32::MAX as f32 * 0.1;
        let mut sampled_x = None;
        assert_eq!(
            raycast_voxels(
                Vec3::new(extreme_x, 0.05, 0.05),
                Vec3::Y,
                0.2,
                0.1,
                |x, _, _| {
                    sampled_x = Some(x);
                    false
                },
            ),
            None
        );
        assert_eq!(sampled_x, Some(i32::MAX));
    }

    #[test]
    fn dda_returns_cleanly_at_grid_coordinate_limits() {
        for (coordinate, direction) in [(i32::MAX, Vec3::X), (i32::MIN, -Vec3::X)] {
            let origin = Vec3::new(coordinate as f32 * 0.1, 0.05, 0.05);
            let mut samples = 0;
            assert_eq!(
                raycast_voxels(origin, direction, 0.2, 0.1, |_, _, _| {
                    samples += 1;
                    false
                }),
                None
            );
            assert_eq!(samples, 1);
        }
    }

    #[test]
    fn voxel_segment_visibility_rejects_only_occluders_before_the_target() {
        let origin = Vec3::new(0.05, 0.05, 0.05);
        assert!(!voxel_segment_is_clear(
            origin,
            Vec3::new(0.85, 0.05, 0.05),
            0.1,
            |x, _, _| x == 4,
        ));
        assert!(voxel_segment_is_clear(
            origin,
            Vec3::new(0.35, 0.05, 0.05),
            0.1,
            |x, _, _| x == 4,
        ));
        assert!(voxel_segment_is_clear(origin, origin, 0.1, |_, _, _| true));
        assert!(!voxel_segment_is_clear(origin, origin, 0.0, |_, _, _| {
            false
        }));
    }

    #[test]
    fn enclosure_probe_distinguishes_open_sky_from_a_sealed_roof() {
        let eye = Vec3::splat(0.05);
        let open = probe_enclosure(eye, 12.0, 0.1, |_, _, _| false);
        assert_eq!(open.escaped_rays, open.ray_count);
        assert_eq!(open.sky_visibility, 1.0);
        assert!(open.escape_direction.distance(Vec3::Y) < 0.0001);

        let sealed = probe_enclosure(eye, 12.0, 0.1, |_, y, _| y >= 3);
        assert_eq!(sealed.escaped_rays, 0);
        assert_eq!(sealed.sky_visibility, 0.0);
        assert_eq!(sealed.enclosure, 1.0);
        assert!(sealed.ceiling_distance_metres < 0.31);
    }

    #[test]
    fn enclosure_probe_points_toward_a_directional_mouth_and_tracks_edits() {
        let eye = Vec3::splat(0.05);
        let mouth = probe_enclosure(eye, 4.0, 0.1, |x, y, _| y >= 3 && x < 2);
        assert!(mouth.escaped_rays > 0 && mouth.escaped_rays < mouth.ray_count);
        assert!(mouth.escape_direction.x > 0.5);
        assert!(mouth.enclosure > 0.0 && mouth.enclosure < 1.0);

        let opened_roof = probe_enclosure(eye, 4.0, 0.1, |x, y, z| y >= 3 && !(x == 0 && z == 0));
        assert!(opened_roof.sky_visibility > 0.0);
    }

    #[test]
    fn enclosure_probe_treats_non_finite_inputs_as_open() {
        for (eye, max_distance, voxel_size) in [
            (Vec3::splat(f32::NAN), 12.0, 0.1),
            (Vec3::ZERO, f32::NAN, 0.1),
            (Vec3::ZERO, f32::INFINITY, 0.1),
            (Vec3::ZERO, 12.0, f32::NAN),
            (Vec3::ZERO, 12.0, f32::INFINITY),
        ] {
            let mut sampled = false;
            assert_eq!(
                probe_enclosure(eye, max_distance, voxel_size, |_, _, _| {
                    sampled = true;
                    false
                }),
                EnclosureSample::OPEN
            );
            assert!(!sampled);
        }
    }

    #[test]
    fn placement_cannot_overlap_player() {
        let camera = CameraState::spawn(Vec3::new(0.0, PLAYER_EYE_HEIGHT_METRES, 0.0));
        assert!(camera.intersects_voxel([0, 0, 0], 0.1));
        assert!(!camera.intersects_voxel([20, 0, 0], 0.1));
    }

    #[test]
    fn canonical_fluid_overlap_drives_immersion_and_eye_depth() {
        let mut camera = CameraState::spawn(Vec3::new(0.05, PLAYER_EYE_HEIGHT_METRES, 0.05));
        camera.refresh_fluid_state(0.1, |_, y, _| {
            if (0..=17).contains(&y) {
                VoxelPhysics::FLUID
            } else {
                VoxelPhysics::EMPTY
            }
        });
        let fluid = camera.fluid_state();
        assert!((fluid.immersion - 1.0).abs() < 0.0001);
        assert!(fluid.eyes_submerged);
        assert!((fluid.surface_y_metres - 1.8).abs() < 0.0001);
        assert!((fluid.eye_depth_metres - 0.18).abs() < 0.0001);
        assert!(fluid.swimming);
    }

    #[test]
    fn fluid_surface_scan_stops_at_the_canonical_grid_limit() {
        let mut camera = CameraState::spawn(Vec3::new(0.0, i32::MAX as f32, 0.0));
        camera.refresh_fluid_state(1.0, |_, _, _| VoxelPhysics::FLUID);

        let fluid = camera.fluid_state();
        assert!(fluid.eyes_submerged);
        assert!(fluid.surface_y_metres.is_finite());
        assert!(fluid.eye_depth_metres.is_finite());
        assert!((0.0..=1.0).contains(&fluid.immersion));
    }

    #[test]
    fn swim_hysteresis_is_stable_at_a_blocky_shoreline() {
        let mut camera = CameraState::spawn(Vec3::new(0.05, PLAYER_EYE_HEIGHT_METRES, 0.05));
        camera.refresh_fluid_state(0.1, |_, y, _| {
            if (0..=9).contains(&y) {
                VoxelPhysics::FLUID
            } else {
                VoxelPhysics::EMPTY
            }
        });
        assert!(camera.fluid_state().swimming);

        camera.refresh_fluid_state(0.1, |_, y, _| {
            if (0..=4).contains(&y) {
                VoxelPhysics::FLUID
            } else {
                VoxelPhysics::EMPTY
            }
        });
        assert!(camera.fluid_state().swimming);

        camera.refresh_fluid_state(0.1, |_, y, _| {
            if (0..=3).contains(&y) {
                VoxelPhysics::FLUID
            } else {
                VoxelPhysics::EMPTY
            }
        });
        assert!(!camera.fluid_state().swimming);
    }

    #[test]
    fn space_ascends_and_shift_dives_while_swimming() {
        let fluid = |_: i32, _: i32, _: i32| VoxelPhysics::FLUID;
        let mut ascending = CameraState::spawn(Vec3::new(0.05, PLAYER_EYE_HEIGHT_METRES, 0.05));
        ascending.refresh_fluid_state(0.1, fluid);
        let mut input = InputState::default();
        input.set_key(5, true);
        for _ in 0..30 {
            ascending.update(&input, 1.0 / 120.0, 0.1, fluid);
        }
        assert!(ascending.velocity.y > 0.5);

        let mut diving = CameraState::spawn(Vec3::new(0.05, PLAYER_EYE_HEIGHT_METRES, 0.05));
        diving.refresh_fluid_state(0.1, fluid);
        let mut input = InputState::default();
        input.set_key(6, true);
        for _ in 0..30 {
            diving.update(&input, 1.0 / 120.0, 0.1, fluid);
        }
        assert!(diving.velocity.y < -0.5);
    }

    #[test]
    fn swimming_player_still_collides_with_solid_voxels() {
        let sample = |x: i32, _: i32, _: i32| {
            if x >= 5 {
                VoxelPhysics::SOLID
            } else {
                VoxelPhysics::FLUID
            }
        };
        let mut camera = CameraState::spawn(Vec3::new(0.0, PLAYER_EYE_HEIGHT_METRES, 0.0));
        camera.refresh_fluid_state(0.1, sample);
        let mut input = InputState::default();
        input.set_key(4, true);
        for _ in 0..240 {
            camera.update(&input, 1.0 / 120.0, 0.1, sample);
        }
        assert!(camera.position.x <= 0.5 - PLAYER_RADIUS_METRES + 0.001);
    }

    #[test]
    fn restored_camera_can_be_valid_inside_water_but_not_solid() {
        let camera =
            CameraState::from_persisted(Vec3::new(0.05, PLAYER_EYE_HEIGHT_METRES, 0.05), 0.0, 0.0);
        assert!(!camera.overlaps_collidable(0.1, |_, _, _| VoxelPhysics::FLUID));
        assert!(camera.overlaps_collidable(0.1, |_, y, _| solid_if(y == 0)));
    }
}
