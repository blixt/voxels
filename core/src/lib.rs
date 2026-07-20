//! Portable simulation and player state. GPU, world generation, and browser concerns live elsewhere.

mod presence;
mod profile;

pub use presence::{
    PresenceInterpolationConfig, REMOTE_POSE_DISCONTINUITY, REMOTE_POSE_FLYING,
    REMOTE_POSE_GLIDING, RemoteAvatarPose, RemotePlayerId, RemotePoseSample, RemotePoseUpdate,
    RemotePresenceDelta, RemotePresenceTimeline,
};
pub use profile::{ProfileAutomation, ProfileConfig, ProfilePhase, ProfilePose, ProfileRoute};

use glam::{Vec2, Vec3};

pub const PLAYER_RADIUS_METRES: f32 = 0.20;
pub const PLAYER_HEIGHT_METRES: f32 = 1.70;
pub const PLAYER_EYE_HEIGHT_METRES: f32 = 1.54;
const WALK_SPEED: f32 = 4.6;
const SPRINT_MULTIPLIER: f32 = 1.55;
const SPECTATOR_SPEED: f32 = 8.0;
const SPECTATOR_RESPONSE: f32 = 10.0;
const GLIDER_FORWARD_SPEED: f32 = 8.4;
const GLIDER_ACCELERATION: f32 = 12.0;
const GLIDER_TERMINAL_DESCENT_SPEED: f32 = 2.2;
const JUMP_SPEED: f32 = 5.9;
const GRAVITY: f32 = 19.5;
const WALK_TERMINAL_FALL_SPEED: f32 = 18.0;
const EFFORTLESS_STEP_HEIGHT: f32 = 0.2;
const ASSISTED_STEP_HEIGHT: f32 = 0.52;
const ASSISTED_STEP_SPEED_SCALE: f32 = 0.72;
const ASSISTED_STEP_RECOVERY_SECONDS: f32 = 0.18;
const AIRBORNE_MANTLE_LIFT: f32 = 0.22;
const GROUND_FOLLOW_DISTANCE: f32 = 0.22;
const GROUND_FOLLOW_SPEED: f32 = 4.0;
const GROUND_GRACE_SECONDS: f32 = 0.1;
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

/// Player locomotion is explicit so a bodyless spectator camera cannot silently leak into normal
/// collision or interaction semantics.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LocomotionMode {
    #[default]
    Walking,
    Gliding,
    Spectator,
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

/// The camera position is the player's eye position in metres. Terrain collision uses a vertical
/// capsule: its rounded foot rides continuously over small voxel ledges while its round horizontal
/// footprint avoids snagging on corners. Collision stays deterministic and allocation-free.
#[derive(Clone, Copy, Debug)]
pub struct CameraState {
    pub position: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub velocity: Vec3,
    pub grounded: bool,
    locomotion: LocomotionMode,
    gliding_available: bool,
    jump_was_down: bool,
    ground_grace_seconds: f32,
    assisted_step_seconds: f32,
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
            locomotion: LocomotionMode::Walking,
            gliding_available: false,
            jump_was_down: false,
            ground_grace_seconds: 0.0,
            assisted_step_seconds: 0.0,
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
            locomotion: LocomotionMode::Walking,
            gliding_available: false,
            jump_was_down: false,
            ground_grace_seconds: 0.0,
            assisted_step_seconds: 0.0,
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

    /// Velocity intent used to prefetch canonical collision data before physics reaches it.
    ///
    /// This is derived from input and locomotion rather than post-collision velocity: a
    /// conservative unloaded-space boundary can zero an axis, but it must not also erase the
    /// request pressure that will remove that boundary.
    pub fn streaming_velocity(self, input: &InputState) -> Vec3 {
        let horizontal_forward = Vec3::new(self.yaw.sin(), 0.0, -self.yaw.cos());
        let right = Vec3::new(-horizontal_forward.z, 0.0, horizontal_forward.x);
        let mut horizontal_wish = Vec3::ZERO;
        if input.forward {
            horizontal_wish += horizontal_forward;
        }
        if input.backward {
            horizontal_wish -= horizontal_forward;
        }
        if input.right {
            horizontal_wish += right;
        }
        if input.left {
            horizontal_wish -= right;
        }

        if self.locomotion == LocomotionMode::Spectator {
            let mut wish = horizontal_wish;
            if input.jump {
                wish += Vec3::Y;
            }
            if input.sprint {
                wish -= Vec3::Y;
            }
            return wish.normalize_or_zero() * SPECTATOR_SPEED;
        }

        if self.fluid.swimming {
            let forward = self.forward();
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
            return wish.normalize_or_zero() * SWIM_SPEED;
        }

        if self.locomotion == LocomotionMode::Gliding {
            let direction = if horizontal_wish.length_squared() > f32::EPSILON {
                horizontal_wish.normalize()
            } else {
                horizontal_forward
            };
            return Vec3::new(
                direction.x * GLIDER_FORWARD_SPEED,
                -GLIDER_TERMINAL_DESCENT_SPEED,
                direction.z * GLIDER_FORWARD_SPEED,
            );
        }

        let speed = WALK_SPEED * if input.sprint { SPRINT_MULTIPLIER } else { 1.0 };
        let horizontal = horizontal_wish.normalize_or_zero() * speed;
        Vec3::new(horizontal.x, self.velocity.y, horizontal.z)
    }

    pub const fn fluid_state(self) -> FluidState {
        self.fluid
    }

    pub const fn locomotion(self) -> LocomotionMode {
        self.locomotion
    }

    pub const fn gliding_available(self) -> bool {
        self.gliding_available
    }

    pub fn set_gliding_available(&mut self, available: bool) {
        self.gliding_available = available;
        if !available && self.locomotion == LocomotionMode::Gliding {
            self.locomotion = LocomotionMode::Walking;
        }
    }

    pub fn set_locomotion(&mut self, locomotion: LocomotionMode) {
        let locomotion = if locomotion == LocomotionMode::Gliding && !self.gliding_available {
            LocomotionMode::Walking
        } else {
            locomotion
        };
        if self.locomotion == locomotion {
            return;
        }
        let spectator_transition =
            self.locomotion == LocomotionMode::Spectator || locomotion == LocomotionMode::Spectator;
        self.locomotion = locomotion;
        self.grounded = false;
        self.jump_was_down = false;
        self.ground_grace_seconds = 0.0;
        self.assisted_step_seconds = 0.0;
        if spectator_transition {
            self.velocity = Vec3::ZERO;
        }
    }

    pub fn intersects_voxel(self, voxel: [i32; 3], voxel_size: f32) -> bool {
        player_intersects_voxel(self.position.to_array(), voxel, voxel_size)
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
        if self.locomotion == LocomotionMode::Spectator {
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
            if input.jump {
                wish += Vec3::Y;
            }
            if input.sprint {
                wish -= Vec3::Y;
            }
            let target = wish.normalize_or_zero() * SPECTATOR_SPEED;
            let response = 1.0 - (-SPECTATOR_RESPONSE * dt).exp();
            self.velocity += (target - self.velocity) * response;
            self.grounded = false;
            self.jump_was_down = input.jump;
            self.ground_grace_seconds = 0.0;
            self.assisted_step_seconds = 0.0;
            let next = self.position + self.velocity * dt;
            if next.is_finite() {
                self.position = next;
            }
            self.fluid = FluidState::default();
            return;
        }
        let was_swimming = self.fluid.swimming;
        if was_swimming && self.locomotion == LocomotionMode::Gliding {
            self.locomotion = LocomotionMode::Walking;
        }
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
            self.ground_grace_seconds = 0.0;
            self.assisted_step_seconds = 0.0;
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

            if self.grounded {
                self.ground_grace_seconds = GROUND_GRACE_SECONDS;
            } else {
                self.ground_grace_seconds = (self.ground_grace_seconds - dt).max(0.0);
            }
            let jump_pressed = input.jump && !self.jump_was_down;
            self.jump_was_down = input.jump;
            if (self.grounded || self.ground_grace_seconds > 0.0) && jump_pressed {
                self.velocity.y = JUMP_SPEED;
                self.grounded = false;
                self.ground_grace_seconds = 0.0;
                self.locomotion = LocomotionMode::Walking;
            } else if !self.grounded && jump_pressed && self.gliding_available {
                self.locomotion = if self.locomotion == LocomotionMode::Gliding {
                    LocomotionMode::Walking
                } else {
                    LocomotionMode::Gliding
                };
            }

            if self.locomotion == LocomotionMode::Gliding {
                let direction = if wish.length_squared() > f32::EPSILON {
                    wish.normalize()
                } else {
                    forward
                };
                let target = direction * GLIDER_FORWARD_SPEED;
                let acceleration = GLIDER_ACCELERATION * dt;
                self.velocity.x += (target.x - self.velocity.x).clamp(-acceleration, acceleration);
                self.velocity.z += (target.z - self.velocity.z).clamp(-acceleration, acceleration);
                let target_y = -GLIDER_TERMINAL_DESCENT_SPEED;
                self.velocity.y += (target_y - self.velocity.y).clamp(-acceleration, acceleration);
            } else {
                let assisted_scale = if self.assisted_step_seconds > 0.0 {
                    ASSISTED_STEP_SPEED_SCALE
                } else {
                    1.0
                };
                self.assisted_step_seconds = (self.assisted_step_seconds - dt).max(0.0);
                let speed = WALK_SPEED
                    * if input.sprint { SPRINT_MULTIPLIER } else { 1.0 }
                    * assisted_scale;
                let target = wish.normalize_or_zero() * speed;
                let response = 1.0 - (-(if self.grounded { 18.0 } else { 5.0 }) * dt).exp();
                self.velocity.x += (target.x - self.velocity.x) * response;
                self.velocity.z += (target.z - self.velocity.z) * response;
                self.velocity.y = (self.velocity.y - GRAVITY * dt).max(-WALK_TERMINAL_FALL_SPEED);
            }
            horizontal_grounded = self.grounded || self.ground_grace_seconds > 0.0;
        }

        let maximum_step_height = if horizontal_grounded {
            ASSISTED_STEP_HEIGHT
        } else if input.jump && self.locomotion == LocomotionMode::Walking && self.velocity.y > -2.0
        {
            // A held jump may finish mounting a ledge only after the body has already risen close
            // enough to its top. This supports 60–100 cm ledges without teleporting a standing
            // player up them or turning ordinary air movement into wall climbing.
            AIRBORNE_MANTLE_LIFT
        } else {
            0.0
        };
        self.move_horizontal(
            Vec2::new(self.velocity.x, self.velocity.z) * dt,
            voxel_size,
            maximum_step_height,
            &mut sample_voxel,
        );
        self.grounded = false;
        self.move_axis(1, self.velocity.y * dt, voxel_size, &mut sample_voxel);
        if !was_swimming
            && !self.grounded
            && horizontal_grounded
            && self.velocity.y <= 0.0
            && let Some(snapped_y) = ground_follow_eye_y(
                self.position,
                voxel_size,
                GROUND_FOLLOW_DISTANCE,
                &mut sample_voxel,
            )
        {
            let followed_y = snapped_y.max(self.position.y - GROUND_FOLLOW_SPEED * dt);
            let snapped = self.position.with_y(followed_y);
            if !collides(snapped, voxel_size, &mut sample_voxel) {
                self.position = snapped;
                self.grounded = true;
                self.ground_grace_seconds = GROUND_GRACE_SECONDS;
                self.velocity.y = 0.0;
            }
        }
        if !self.grounded
            && has_ground_support(
                self.position,
                voxel_size,
                voxel_size * 0.08,
                &mut sample_voxel,
            )
        {
            self.grounded = true;
            self.ground_grace_seconds = GROUND_GRACE_SECONDS;
            self.velocity.y = self.velocity.y.max(0.0);
        }
        if self.grounded && self.locomotion == LocomotionMode::Gliding {
            self.locomotion = LocomotionMode::Walking;
        }
        self.refresh_fluid_state(voxel_size, &mut sample_voxel);
        if self.fluid.swimming && self.locomotion == LocomotionMode::Gliding {
            self.locomotion = LocomotionMode::Walking;
        }
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

    fn move_horizontal(
        &mut self,
        distance: Vec2,
        voxel_size: f32,
        maximum_step_height: f32,
        sample_voxel: &mut impl FnMut(i32, i32, i32) -> VoxelPhysics,
    ) {
        if distance.length_squared() <= f32::EPSILON {
            return;
        }
        let step_count = (distance.length() / (voxel_size * 0.45)).ceil().max(1.0) as u32;
        let delta = distance / step_count as f32;
        let mut blocked_x = false;
        let mut blocked_z = false;
        for _ in 0..step_count {
            let origin = self.position;
            let full = origin + Vec3::new(delta.x, 0.0, delta.y);
            if let Some((resolved, lift)) =
                resolve_horizontal_candidate(full, voxel_size, maximum_step_height, sample_voxel)
            {
                if lift > EFFORTLESS_STEP_HEIGHT {
                    self.assisted_step_seconds = ASSISTED_STEP_RECOVERY_SECONDS;
                    let scale = assisted_step_speed_scale(lift);
                    let slowed = origin + Vec3::new(delta.x * scale, 0.0, delta.y * scale);
                    self.position = resolve_horizontal_candidate(
                        slowed,
                        voxel_size,
                        maximum_step_height,
                        sample_voxel,
                    )
                    .map_or(resolved, |(position, _)| position);
                } else {
                    self.position = resolved;
                }
                continue;
            }

            let x = (delta.x.abs() > f32::EPSILON)
                .then(|| origin + Vec3::X * delta.x)
                .and_then(|candidate| {
                    resolve_horizontal_candidate(
                        candidate,
                        voxel_size,
                        maximum_step_height,
                        sample_voxel,
                    )
                });
            let z = (delta.y.abs() > f32::EPSILON)
                .then(|| origin + Vec3::Z * delta.y)
                .and_then(|candidate| {
                    resolve_horizontal_candidate(
                        candidate,
                        voxel_size,
                        maximum_step_height,
                        sample_voxel,
                    )
                });
            match (x, z) {
                (Some(x), Some(_)) if delta.x.abs() >= delta.y.abs() => {
                    self.position = x.0;
                    blocked_z = true;
                }
                (Some(_), Some(z)) => {
                    self.position = z.0;
                    blocked_x = true;
                }
                (Some(x), None) => {
                    self.position = x.0;
                    blocked_z = true;
                }
                (None, Some(z)) => {
                    self.position = z.0;
                    blocked_x = true;
                }
                (None, None) => {
                    blocked_x |= delta.x.abs() > f32::EPSILON;
                    blocked_z |= delta.y.abs() > f32::EPSILON;
                    break;
                }
            }
        }
        if blocked_x {
            self.velocity.x = 0.0;
        }
        if blocked_z {
            self.velocity.z = 0.0;
        }
    }

    fn move_axis(
        &mut self,
        axis: usize,
        distance: f32,
        voxel_size: f32,
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
            self.velocity[axis] = 0.0;
            if axis == 1 && distance < 0.0 {
                self.grounded = true;
            }
            break;
        }
    }
}

fn resolve_horizontal_candidate(
    candidate: Vec3,
    voxel_size: f32,
    maximum_step_height: f32,
    sample_voxel: &mut impl FnMut(i32, i32, i32) -> VoxelPhysics,
) -> Option<(Vec3, f32)> {
    if !collides(candidate, voxel_size, sample_voxel) {
        return Some((candidate, 0.0));
    }
    if maximum_step_height <= 0.0 {
        return None;
    }
    let lift = required_step_lift(candidate, voxel_size, sample_voxel)?;
    if !(0.0..=maximum_step_height + COLLISION_EPSILON).contains(&lift) {
        return None;
    }
    let raised = candidate + Vec3::Y * lift;
    (!collides(raised, voxel_size, sample_voxel)).then_some((raised, lift))
}

fn assisted_step_speed_scale(lift: f32) -> f32 {
    let assisted = ((lift - EFFORTLESS_STEP_HEIGHT)
        / (ASSISTED_STEP_HEIGHT - EFFORTLESS_STEP_HEIGHT))
        .clamp(0.0, 1.0);
    1.0 - assisted * 0.38
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
    for z in min_voxel.z..=max_voxel.z {
        for x in min_voxel.x..=max_voxel.x {
            for y in min_voxel.y..=max_voxel.y {
                if sample_voxel(x, y, z).collidable
                    && capsule_intersects_voxel(eye_position, [x, y, z], voxel_size)
                {
                    return true;
                }
            }
        }
    }
    false
}

/// Tests the canonical standing-player capsule without requiring a client camera object. Native
/// servers use the same dimensions to reject placements that would enclose the player's body.
pub fn player_intersects_voxel(
    eye_position_metres: [f32; 3],
    voxel: [i32; 3],
    voxel_size: f32,
) -> bool {
    let eye_position = Vec3::from_array(eye_position_metres);
    eye_position.is_finite()
        && voxel_size.is_finite()
        && voxel_size > 0.0
        && capsule_intersects_voxel(eye_position, voxel, voxel_size)
}

fn capsule_intersects_voxel(eye_position: Vec3, voxel: [i32; 3], voxel_size: f32) -> bool {
    let radius = PLAYER_RADIUS_METRES - COLLISION_EPSILON;
    let feet_y = eye_position.y - PLAYER_EYE_HEIGHT_METRES;
    let head_y = feet_y + PLAYER_HEIGHT_METRES;
    let voxel_min = Vec3::from_array(voxel.map(|value| value as f32 * voxel_size));
    let voxel_max = voxel_min + Vec3::splat(voxel_size);
    let segment_min_y = feet_y + radius;
    let segment_max_y = head_y - radius;
    let horizontal_distance_squared =
        horizontal_distance_squared_to_voxel(eye_position.x, eye_position.z, voxel_min, voxel_max);
    let vertical_distance = if segment_max_y < voxel_min.y {
        voxel_min.y - segment_max_y
    } else if segment_min_y > voxel_max.y {
        segment_min_y - voxel_max.y
    } else {
        0.0
    };
    horizontal_distance_squared + vertical_distance * vertical_distance < radius * radius
}

fn horizontal_circle_overlaps_voxel(eye_position: Vec3, voxel_min: Vec3, voxel_max: Vec3) -> bool {
    let collision_radius = PLAYER_RADIUS_METRES - COLLISION_EPSILON;
    horizontal_distance_squared_to_voxel(eye_position.x, eye_position.z, voxel_min, voxel_max)
        < collision_radius * collision_radius
}

fn horizontal_distance_squared_to_voxel(x: f32, z: f32, voxel_min: Vec3, voxel_max: Vec3) -> f32 {
    let delta_x = x - x.clamp(voxel_min.x, voxel_max.x);
    let delta_z = z - z.clamp(voxel_min.z, voxel_max.z);
    delta_x * delta_x + delta_z * delta_z
}

fn required_step_lift(
    eye_position: Vec3,
    voxel_size: f32,
    sample_voxel: &mut impl FnMut(i32, i32, i32) -> VoxelPhysics,
) -> Option<f32> {
    let (min, max) = player_bounds(eye_position);
    let max = max - Vec3::splat(COLLISION_EPSILON);
    let min_voxel = (min / voxel_size).floor().as_ivec3();
    let max_voxel = (max / voxel_size).floor().as_ivec3();
    let radius = PLAYER_RADIUS_METRES - COLLISION_EPSILON;
    let segment_min_y = eye_position.y - PLAYER_EYE_HEIGHT_METRES + radius;
    let mut lift = 0.0_f32;
    let mut found = false;
    for z in min_voxel.z..=max_voxel.z {
        for x in min_voxel.x..=max_voxel.x {
            for y in min_voxel.y..=max_voxel.y {
                if !sample_voxel(x, y, z).collidable
                    || !capsule_intersects_voxel(eye_position, [x, y, z], voxel_size)
                {
                    continue;
                }
                found = true;
                let voxel_min = Vec3::new(x as f32, y as f32, z as f32) * voxel_size;
                let voxel_max = voxel_min + Vec3::splat(voxel_size);
                let horizontal_distance_squared = horizontal_distance_squared_to_voxel(
                    eye_position.x,
                    eye_position.z,
                    voxel_min,
                    voxel_max,
                );
                if horizontal_distance_squared >= radius * radius {
                    continue;
                }
                let cap_reach = (radius * radius - horizontal_distance_squared).sqrt();
                lift = lift.max(voxel_max.y + cap_reach - segment_min_y + COLLISION_EPSILON);
            }
        }
    }
    found.then_some(lift)
}

fn ground_follow_eye_y(
    eye_position: Vec3,
    voxel_size: f32,
    maximum_drop: f32,
    sample_voxel: &mut impl FnMut(i32, i32, i32) -> VoxelPhysics,
) -> Option<f32> {
    let radius = PLAYER_RADIUS_METRES - COLLISION_EPSILON;
    let feet_y = eye_position.y - PLAYER_EYE_HEIGHT_METRES;
    let horizontal_min = Vec3::new(eye_position.x - radius, 0.0, eye_position.z - radius);
    let horizontal_max = Vec3::new(eye_position.x + radius, 0.0, eye_position.z + radius);
    let min_voxel = (horizontal_min / voxel_size).floor().as_ivec3();
    let max_voxel = (horizontal_max / voxel_size).floor().as_ivec3();
    let min_y = ((feet_y - maximum_drop - radius) / voxel_size).floor() as i32;
    let max_y = ((feet_y + COLLISION_EPSILON) / voxel_size).floor() as i32;
    let mut highest = f32::NEG_INFINITY;
    for z in min_voxel.z..=max_voxel.z {
        for x in min_voxel.x..=max_voxel.x {
            for y in min_y..=max_y {
                if !sample_voxel(x, y, z).collidable {
                    continue;
                }
                let voxel_min = Vec3::new(x as f32, y as f32, z as f32) * voxel_size;
                let voxel_max = voxel_min + Vec3::splat(voxel_size);
                let horizontal_distance_squared = horizontal_distance_squared_to_voxel(
                    eye_position.x,
                    eye_position.z,
                    voxel_min,
                    voxel_max,
                );
                if horizontal_distance_squared >= radius * radius {
                    continue;
                }
                let cap_reach = (radius * radius - horizontal_distance_squared).sqrt();
                let contact_eye_y = voxel_max.y + cap_reach - radius + PLAYER_EYE_HEIGHT_METRES;
                let drop = eye_position.y - contact_eye_y;
                if drop >= -COLLISION_EPSILON && drop <= maximum_drop + COLLISION_EPSILON {
                    highest = highest.max(contact_eye_y);
                }
            }
        }
    }
    highest.is_finite().then_some(highest)
}

fn has_ground_support(
    eye_position: Vec3,
    voxel_size: f32,
    probe_distance: f32,
    sample_voxel: &mut impl FnMut(i32, i32, i32) -> VoxelPhysics,
) -> bool {
    let feet_y = eye_position.y - PLAYER_EYE_HEIGHT_METRES;
    let horizontal_min = Vec3::new(
        eye_position.x - PLAYER_RADIUS_METRES,
        0.0,
        eye_position.z - PLAYER_RADIUS_METRES,
    );
    let horizontal_max = Vec3::new(
        eye_position.x + PLAYER_RADIUS_METRES - COLLISION_EPSILON,
        0.0,
        eye_position.z + PLAYER_RADIUS_METRES - COLLISION_EPSILON,
    );
    let min_voxel = (horizontal_min / voxel_size).floor().as_ivec3();
    let max_voxel = (horizontal_max / voxel_size).floor().as_ivec3();
    let min_y = ((feet_y - probe_distance) / voxel_size)
        .floor()
        .clamp(i32::MIN as f32, i32::MAX as f32) as i32;
    let min_y = min_y.saturating_sub(1);
    let max_y = ((feet_y + COLLISION_EPSILON) / voxel_size).floor() as i32;
    for z in min_voxel.z..=max_voxel.z {
        for x in min_voxel.x..=max_voxel.x {
            let voxel_min = Vec3::new(x as f32, 0.0, z as f32) * voxel_size;
            let voxel_max = voxel_min + Vec3::splat(voxel_size);
            if !horizontal_circle_overlaps_voxel(eye_position, voxel_min, voxel_max) {
                continue;
            }
            for y in min_y..=max_y {
                let surface_y = (y as f32 + 1.0) * voxel_size;
                if surface_y >= feet_y - probe_distance - COLLISION_EPSILON
                    && surface_y <= feet_y + COLLISION_EPSILON
                    && sample_voxel(x, y, z).collidable
                {
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
    fn player_capsule_is_one_point_seven_metres_tall_and_point_four_wide() {
        assert_eq!(PLAYER_HEIGHT_METRES, 1.70);
        assert_eq!(PLAYER_RADIUS_METRES * 2.0, 0.40);
        assert!((PLAYER_HEIGHT_METRES - PLAYER_EYE_HEIGHT_METRES - 0.16).abs() < 1.0e-6);

        let centred = CameraState::spawn(Vec3::new(0.2, PLAYER_EYE_HEIGHT_METRES, 0.05));
        assert!(!centred.intersects_voxel([-1, 2, 0], 0.1));
        assert!(!centred.intersects_voxel([4, 2, 0], 0.1));
        let shifted = CameraState::spawn(Vec3::new(0.19, PLAYER_EYE_HEIGHT_METRES, 0.05));
        assert!(shifted.intersects_voxel([-1, 2, 0], 0.1));
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
    fn long_falls_converge_on_a_finite_authorizable_terminal_speed() {
        let mut camera = CameraState::spawn(Vec3::new(0.0, 1_000.0, 0.0));
        for _ in 0..2_000 {
            camera.update(&InputState::default(), 1.0 / 120.0, 0.1, |_, _, _| {
                VoxelPhysics::EMPTY
            });
        }
        assert_eq!(camera.velocity.y, -WALK_TERMINAL_FALL_SPEED);
        assert!(!camera.grounded);
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
    fn spectator_uses_space_and_shift_without_gravity() {
        let mut camera = CameraState::spawn(Vec3::new(0.0, 8.0, 0.0));
        camera.set_locomotion(LocomotionMode::Spectator);
        let mut input = InputState::default();
        input.set_key(5, true);
        for _ in 0..120 {
            camera.update(&input, 1.0 / 120.0, 0.1, |_, _, _| VoxelPhysics::EMPTY);
        }
        let raised_y = camera.position.y;
        assert!(raised_y > 14.0);
        assert_eq!(camera.locomotion(), LocomotionMode::Spectator);
        assert!(!camera.grounded);

        input.set_key(5, false);
        input.set_key(6, true);
        for _ in 0..120 {
            camera.update(&input, 1.0 / 120.0, 0.1, |_, _, _| VoxelPhysics::EMPTY);
        }
        assert!(camera.position.y < raised_y - 5.0);
    }

    #[test]
    fn spectator_is_collisionless_and_walking_restores_gravity() {
        let mut camera = CameraState::spawn(Vec3::new(0.0, PLAYER_EYE_HEIGHT_METRES, 0.0));
        camera.set_locomotion(LocomotionMode::Spectator);
        let mut input = InputState::default();
        input.set_key(4, true);
        for _ in 0..120 {
            camera.update(&input, 1.0 / 120.0, 0.1, |x, _, _| solid_if(x >= 5));
        }
        assert!(camera.position.x > 1.0);

        input.clear();
        camera.set_locomotion(LocomotionMode::Walking);
        let before = camera.position.y;
        for _ in 0..12 {
            camera.update(&input, 1.0 / 120.0, 0.1, |_, _, _| VoxelPhysics::EMPTY);
        }
        assert!(camera.position.y < before);
    }

    #[test]
    fn rounded_footprint_does_not_catch_on_empty_voxel_corners() {
        let clear = CameraState::spawn(Vec3::new(-0.22, PLAYER_EYE_HEIGHT_METRES, -0.22));
        assert!(!clear.intersects_voxel([0, 0, 0], 0.1));
        assert!(!clear.overlaps_collidable(0.1, |x, y, z| { solid_if([x, y, z] == [0, 0, 0]) }));

        let touching = CameraState::spawn(Vec3::new(-0.12, PLAYER_EYE_HEIGHT_METRES, -0.12));
        assert!(touching.intersects_voxel([0, 0, 0], 0.1));
        assert!(touching.overlaps_collidable(0.1, |x, y, z| { solid_if([x, y, z] == [0, 0, 0]) }));
    }

    #[test]
    fn grounded_player_walks_over_a_ten_centimetre_bump_without_jumping() {
        let mut camera = CameraState::spawn(Vec3::new(0.0, PLAYER_EYE_HEIGHT_METRES, 0.0));
        camera.grounded = true;
        let mut input = InputState::default();
        input.set_key(4, true);
        for _ in 0..120 {
            camera.update(&input, 1.0 / 120.0, 0.1, |x, y, _| {
                solid_if(y < 0 || (x >= 5 && y == 0))
            });
        }

        assert!(camera.position.x > 1.0);
        assert!(camera.grounded);
        assert!(
            (camera.position.y - PLAYER_EYE_HEIGHT_METRES - 0.1).abs() < 0.011,
            "feet should settle on the bump: {:?}",
            camera.position
        );
    }

    #[test]
    fn grounded_player_walks_over_two_voxel_steps_without_jumping() {
        let mut camera = CameraState::spawn(Vec3::new(0.0, PLAYER_EYE_HEIGHT_METRES, 0.0));
        camera.grounded = true;
        let mut input = InputState::default();
        input.set_key(4, true);
        for _ in 0..120 {
            camera.update(&input, 1.0 / 120.0, 0.1, |x, y, _| {
                solid_if(y < 0 || (x >= 5 && y < 2))
            });
        }

        assert!(camera.position.x > 1.0, "position: {:?}", camera.position);
        assert!(camera.grounded);
        assert!(
            (camera.position.y - PLAYER_EYE_HEIGHT_METRES - 0.2).abs() < 0.011,
            "feet should settle on the two-voxel step: {:?}",
            camera.position
        );
    }

    #[test]
    fn five_voxel_step_is_climbable_but_slower_than_planar_ground() {
        fn travel(step_height: i32) -> f32 {
            let mut camera = CameraState::spawn(Vec3::new(0.0, PLAYER_EYE_HEIGHT_METRES, 0.0));
            camera.grounded = true;
            let mut input = InputState::default();
            input.set_key(4, true);
            for _ in 0..90 {
                camera.update(&input, 1.0 / 120.0, 0.1, |x, y, _| {
                    solid_if(y < 0 || (x >= 5 && y < step_height))
                });
            }
            camera.position.x
        }

        let planar = travel(0);
        let assisted = travel(5);
        assert!(assisted > 0.8, "five-voxel climb stalled at {assisted}m");
        assert!(
            assisted < planar - 0.04,
            "assisted climb {assisted}m was not slower than planar travel {planar}m"
        );
    }

    #[test]
    fn held_jump_can_finish_mounting_a_one_metre_ledge() {
        let mut camera = CameraState::spawn(Vec3::new(0.0, PLAYER_EYE_HEIGHT_METRES, 0.0));
        camera.grounded = true;
        let mut input = InputState::default();
        input.set_key(4, true);
        input.set_key(5, true);
        for _ in 0..180 {
            camera.update(&input, 1.0 / 120.0, 0.1, |x, y, _| {
                solid_if(y < 0 || (x >= 9 && y < 10))
            });
        }

        assert!(camera.position.x > 1.2, "ledge climb stalled: {camera:?}");
        assert!(
            camera.position.y - PLAYER_EYE_HEIGHT_METRES > 0.98,
            "player did not mount the one-metre ledge: {camera:?}"
        );
    }

    #[test]
    fn jump_remains_available_through_a_short_ground_contact_gap() {
        let mut camera = CameraState::spawn(Vec3::new(0.0, PLAYER_EYE_HEIGHT_METRES, 0.0));
        camera.grounded = true;
        camera.update(&InputState::default(), 1.0 / 120.0, 0.1, |_, _, _| {
            VoxelPhysics::EMPTY
        });
        assert!(!camera.grounded);

        let mut input = InputState::default();
        input.set_key(5, true);
        camera.update(&input, 1.0 / 120.0, 0.1, |_, _, _| VoxelPhysics::EMPTY);

        assert!(
            camera.velocity.y > JUMP_SPEED - GRAVITY / 120.0 - 0.001,
            "contact grace did not produce a jump: {camera:?}"
        );
    }

    #[test]
    fn capsule_follows_a_ten_centimetre_dip_then_climbs_without_stalling() {
        let mut camera = CameraState::spawn(Vec3::new(0.0, PLAYER_EYE_HEIGHT_METRES + 0.1, 0.0));
        camera.grounded = true;
        let mut input = InputState::default();
        input.set_key(4, true);
        let mut minimum_feet = f32::INFINITY;
        let mut maximum_vertical_step = 0.0_f32;
        let mut previous_y = camera.position.y;
        for _ in 0..180 {
            camera.update(&input, 1.0 / 120.0, 0.1, |x, y, _| {
                solid_if(y < 0 || (!(5..15).contains(&x) && y == 0))
            });
            minimum_feet = minimum_feet.min(camera.position.y - PLAYER_EYE_HEIGHT_METRES);
            maximum_vertical_step =
                maximum_vertical_step.max((camera.position.y - previous_y).abs());
            previous_y = camera.position.y;
        }

        assert!(
            camera.position.x > 2.0,
            "controller stalled at a rolling ledge"
        );
        assert!(minimum_feet < 0.03, "rounded foot never followed the dip");
        assert!(
            maximum_vertical_step < 0.055,
            "terrain following snapped by {maximum_vertical_step}m"
        );
    }

    #[test]
    fn diagonal_capsule_motion_climbs_a_convex_voxel_corner() {
        let mut camera = CameraState::spawn(Vec3::new(0.0, PLAYER_EYE_HEIGHT_METRES, 0.0));
        camera.grounded = true;
        camera.yaw = std::f32::consts::FRAC_PI_4;
        let mut input = InputState::default();
        input.set_key(1, true);
        for _ in 0..180 {
            camera.update(&input, 1.0 / 120.0, 0.1, |x, y, z| {
                solid_if(y < 0 || ((x >= 5 || z <= -5) && y == 0))
            });
        }

        assert!(camera.position.x > 1.2, "position: {:?}", camera.position);
        assert!(camera.position.z < -1.2, "position: {:?}", camera.position);
        assert!(camera.grounded);
    }

    #[test]
    fn low_ceiling_prevents_capsule_step_up() {
        let mut camera = CameraState::spawn(Vec3::new(0.0, PLAYER_EYE_HEIGHT_METRES, 0.0));
        camera.grounded = true;
        let mut input = InputState::default();
        input.set_key(4, true);
        for _ in 0..120 {
            camera.update(&input, 1.0 / 120.0, 0.1, |x, y, _| {
                solid_if(y < 0 || (x >= 5 && y == 0) || y == 17)
            });
        }

        assert!(
            camera.position.x < 0.5,
            "low ceiling allowed the player to clear the ledge: {:?}",
            camera.position
        );
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
    fn fresh_airborne_space_press_deploys_and_retracts_the_glider() {
        let mut camera = CameraState::spawn(Vec3::new(0.0, 12.0, 0.0));
        camera.set_gliding_available(true);
        camera.velocity = Vec3::new(0.0, -8.0, 0.0);
        let mut input = InputState::default();
        input.set_key(5, true);
        camera.update(&input, 1.0 / 120.0, 0.1, |_, _, _| VoxelPhysics::EMPTY);
        assert_eq!(camera.locomotion(), LocomotionMode::Gliding);

        input.set_key(5, false);
        for _ in 0..240 {
            camera.update(&input, 1.0 / 120.0, 0.1, |_, _, _| VoxelPhysics::EMPTY);
        }
        assert!(
            (-2.21..=-2.19).contains(&camera.velocity.y),
            "glider should converge on its bounded descent speed: {}",
            camera.velocity.y
        );
        assert!(
            camera.velocity.z < -8.3,
            "a deployed glider should build forward airspeed"
        );

        input.set_key(5, true);
        camera.update(&input, 1.0 / 120.0, 0.1, |_, _, _| VoxelPhysics::EMPTY);
        assert_eq!(camera.locomotion(), LocomotionMode::Walking);
        let descent_before = camera.velocity.y;
        input.set_key(5, false);
        camera.update(&input, 1.0 / 120.0, 0.1, |_, _, _| VoxelPhysics::EMPTY);
        assert!(camera.velocity.y < descent_before);
    }

    #[test]
    fn streaming_velocity_keeps_request_pressure_after_collision_stops_motion() {
        let camera = CameraState::spawn(Vec3::new(0.0, PLAYER_EYE_HEIGHT_METRES, 0.0));
        let mut input = InputState::default();
        input.set_key(1, true);
        input.set_key(6, true);

        let intent = camera.streaming_velocity(&input);

        assert_eq!(camera.velocity, Vec3::ZERO);
        assert!(intent.z < -WALK_SPEED * 1.5);
        assert_eq!(intent.y, 0.0);
    }

    #[test]
    fn streaming_velocity_projects_the_glider_even_without_directional_input() {
        let mut camera = CameraState::spawn(Vec3::new(0.0, 12.0, 0.0));
        camera.set_gliding_available(true);
        camera.set_locomotion(LocomotionMode::Gliding);
        camera.velocity = Vec3::ZERO;

        let intent = camera.streaming_velocity(&InputState::default());

        assert!((intent.z + GLIDER_FORWARD_SPEED).abs() < f32::EPSILON);
        assert!((intent.y + GLIDER_TERMINAL_DESCENT_SPEED).abs() < f32::EPSILON);
    }

    #[test]
    fn gliding_requires_authority_and_landing_or_water_cancels_it() {
        let mut unavailable = CameraState::spawn(Vec3::new(0.0, 8.0, 0.0));
        let mut input = InputState::default();
        input.set_key(5, true);
        unavailable.update(&input, 1.0 / 60.0, 0.1, |_, _, _| VoxelPhysics::EMPTY);
        assert_eq!(unavailable.locomotion(), LocomotionMode::Walking);

        let mut landing = CameraState::spawn(Vec3::new(0.0, PLAYER_EYE_HEIGHT_METRES + 0.4, 0.0));
        landing.set_gliding_available(true);
        landing.set_locomotion(LocomotionMode::Gliding);
        landing.velocity.y = -2.2;
        for _ in 0..60 {
            landing.update(&InputState::default(), 1.0 / 120.0, 0.1, |_, y, _| {
                solid_if(y < 0)
            });
        }
        assert!(landing.grounded);
        assert_eq!(landing.locomotion(), LocomotionMode::Walking);

        let mut swimming = CameraState::spawn(Vec3::new(0.05, PLAYER_EYE_HEIGHT_METRES, 0.05));
        swimming.set_gliding_available(true);
        swimming.set_locomotion(LocomotionMode::Gliding);
        swimming.refresh_fluid_state(0.1, |_, y, _| VoxelPhysics {
            collidable: false,
            fluid: y <= 20,
        });
        swimming.update(&InputState::default(), 1.0 / 120.0, 0.1, |_, y, _| {
            VoxelPhysics {
                collidable: false,
                fluid: y <= 20,
            }
        });
        assert_eq!(swimming.locomotion(), LocomotionMode::Walking);
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
            assert_eq!(camera.locomotion, before.locomotion);
            assert_eq!(camera.gliding_available, before.gliding_available);
            assert_eq!(camera.jump_was_down, before.jump_was_down);
            assert_eq!(camera.ground_grace_seconds, before.ground_grace_seconds);
            assert_eq!(camera.assisted_step_seconds, before.assisted_step_seconds);
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
        assert!(
            (fluid.eye_depth_metres - (fluid.surface_y_metres - PLAYER_EYE_HEIGHT_METRES)).abs()
                < 0.0001
        );
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
