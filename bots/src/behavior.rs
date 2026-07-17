use serde::{Deserialize, Serialize};
use std::f32::consts::{PI, TAU};
use voxels_world::protocol::{EditAction, VoxelFace};
use voxels_world::{Material, VOXEL_SIZE_METRES, VoxelCoord};

const DECISION_INTERVAL_SECONDS: f32 = 0.5;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BehaviorKind {
    Explorer,
    Digger,
    Builder,
    Follower,
}

impl BehaviorKind {
    pub const ALL: [Self; 4] = [Self::Explorer, Self::Digger, Self::Builder, Self::Follower];

    pub const fn for_index(index: usize) -> Self {
        Self::ALL[index % Self::ALL.len()]
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BotLayout {
    Dense,
    Mixed,
}

#[derive(Clone, Copy, Debug)]
pub struct BehaviorContext {
    pub elapsed_seconds: f32,
    pub eye_position_metres: [f32; 3],
    pub placeable_material: Option<Material>,
    pub leader_pose: Option<LeaderPose>,
    pub leader_action: Option<ObservedAction>,
}

#[derive(Clone, Copy, Debug)]
pub struct LeaderPose {
    pub eye_position_metres: [f32; 3],
    pub yaw_radians: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct ObservedAction {
    pub serial: u64,
    pub action: EditAction,
}

#[derive(Clone, Copy, Debug)]
pub struct BehaviorIntent {
    pub yaw_radians: f32,
    pub pitch_radians: f32,
    pub forward: bool,
    pub sprint: bool,
    pub edit: Option<EditAction>,
    pub copied_leader_action: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct BehaviorState {
    kind: BehaviorKind,
    layout: BotLayout,
    index: usize,
    base_heading: f32,
    next_decision_seconds: f32,
    decision_index: u64,
    tower_height: i32,
    last_leader_action_serial: u64,
}

impl BehaviorState {
    pub fn new(kind: BehaviorKind, layout: BotLayout, index: usize, seed: u64) -> Self {
        let hash = splitmix64(seed ^ (index as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15));
        let base_heading = hash as f32 / u64::MAX as f32 * TAU - PI;
        Self {
            kind,
            layout,
            index,
            base_heading,
            next_decision_seconds: 0.25 + index as f32 * 0.013,
            decision_index: 0,
            tower_height: 1,
            last_leader_action_serial: 0,
        }
    }

    pub const fn kind(self) -> BehaviorKind {
        self.kind
    }

    pub fn plan(&mut self, context: BehaviorContext) -> BehaviorIntent {
        let decision_due = context.elapsed_seconds >= self.next_decision_seconds;
        if decision_due {
            self.next_decision_seconds += DECISION_INTERVAL_SECONDS;
            self.decision_index = self.decision_index.saturating_add(1);
        }
        match self.kind {
            BehaviorKind::Explorer => self.explorer(context),
            BehaviorKind::Digger => self.digger(context, decision_due),
            BehaviorKind::Builder => self.builder(context, decision_due),
            BehaviorKind::Follower => self.follower(context, decision_due),
        }
    }

    fn explorer(self, context: BehaviorContext) -> BehaviorIntent {
        let meander = (context.elapsed_seconds * 0.13 + self.index as f32 * 0.7).sin() * 0.22;
        let dense_turn = if self.layout == BotLayout::Dense {
            (context.elapsed_seconds * 0.25).sin() * 0.9
        } else {
            0.0
        };
        BehaviorIntent {
            yaw_radians: normalize_yaw(self.base_heading + meander + dense_turn),
            pitch_radians: -0.08,
            forward: true,
            sprint: true,
            edit: None,
            copied_leader_action: false,
        }
    }

    fn digger(self, context: BehaviorContext, decision_due: bool) -> BehaviorIntent {
        let descending = (self.decision_index / 5).is_multiple_of(2);
        let yaw = normalize_yaw(self.base_heading + (self.decision_index / 10) as f32 * 0.7);
        let edit = decision_due.then(|| {
            if descending {
                downward_dig(
                    context.eye_position_metres,
                    self.index,
                    ((self.decision_index.saturating_sub(1) % 5) * 5) as i32,
                )
            } else {
                forward_dig(
                    context.eye_position_metres,
                    yaw,
                    14 + (self.decision_index % 5) as i32 * 5,
                )
            }
        });
        BehaviorIntent {
            yaw_radians: yaw,
            pitch_radians: if descending { 0.55 } else { 0.0 },
            forward: !descending,
            sprint: false,
            edit,
            copied_leader_action: false,
        }
    }

    fn builder(&mut self, context: BehaviorContext, decision_due: bool) -> BehaviorIntent {
        let yaw = normalize_yaw(self.base_heading);
        let edit = if !decision_due {
            None
        } else if let Some(material) = context.placeable_material {
            let position = metres_to_voxel(context.eye_position_metres);
            let [offset_x, offset_z] = worksite_offset(self.index);
            let coord = VoxelCoord::new(
                position.x + offset_x,
                position.y - 18 + self.tower_height,
                position.z + offset_z,
            );
            self.tower_height = (self.tower_height + 1).min(36);
            Some(EditAction::Place { coord, material })
        } else {
            Some(tower_site_dig(context.eye_position_metres, self.index))
        };
        BehaviorIntent {
            yaw_radians: yaw,
            pitch_radians: 0.45,
            forward: false,
            sprint: false,
            edit,
            copied_leader_action: false,
        }
    }

    fn follower(&mut self, context: BehaviorContext, decision_due: bool) -> BehaviorIntent {
        let (yaw, forward) = context
            .leader_pose
            .map_or((self.base_heading, false), |leader| {
                let delta_x = leader.eye_position_metres[0] - context.eye_position_metres[0];
                let delta_z = leader.eye_position_metres[2] - context.eye_position_metres[2];
                let distance = delta_x.hypot(delta_z);
                let yaw = if distance > 0.01 {
                    delta_x.atan2(-delta_z)
                } else {
                    leader.yaw_radians
                };
                (yaw, distance > 1.8)
            });
        let copied = decision_due
            && context
                .leader_action
                .is_some_and(|action| action.serial > self.last_leader_action_serial);
        let edit = if copied {
            let observed = context.leader_action;
            if let Some(observed) = observed {
                self.last_leader_action_serial = observed.serial;
            }
            observed.and_then(|observed| {
                copied_action(
                    observed,
                    context.eye_position_metres,
                    context.placeable_material,
                    self.index,
                )
            })
        } else if decision_due && context.placeable_material.is_none() {
            Some(downward_dig(context.eye_position_metres, self.index, 0))
        } else {
            None
        };
        BehaviorIntent {
            yaw_radians: normalize_yaw(yaw),
            pitch_radians: 0.0,
            forward,
            sprint: forward,
            edit,
            copied_leader_action: copied,
        }
    }
}

fn downward_dig(eye: [f32; 3], index: usize, extra_depth_voxels: i32) -> EditAction {
    let position = metres_to_voxel(eye);
    let [offset_x, offset_z] = worksite_offset(index);
    EditAction::Dig {
        hit: VoxelCoord::new(
            position.x + offset_x,
            position.y - 17 - extra_depth_voxels,
            position.z + offset_z,
        ),
        face: VoxelFace::PositiveY,
    }
}

fn tower_site_dig(eye: [f32; 3], index: usize) -> EditAction {
    let position = metres_to_voxel(eye);
    let [offset_x, offset_z] = worksite_offset(index);
    EditAction::Dig {
        hit: VoxelCoord::new(
            position.x + offset_x,
            position.y - 17,
            position.z + offset_z,
        ),
        face: VoxelFace::NegativeY,
    }
}

fn forward_dig(eye: [f32; 3], yaw: f32, distance_voxels: i32) -> EditAction {
    let position = metres_to_voxel(eye);
    let direction = [yaw.sin(), -yaw.cos()];
    let hit = VoxelCoord::new(
        position.x + (direction[0] * distance_voxels as f32).round() as i32,
        position.y - 9,
        position.z + (direction[1] * distance_voxels as f32).round() as i32,
    );
    let face = if direction[0].abs() > direction[1].abs() {
        if direction[0] >= 0.0 {
            VoxelFace::NegativeX
        } else {
            VoxelFace::PositiveX
        }
    } else if direction[1] >= 0.0 {
        VoxelFace::NegativeZ
    } else {
        VoxelFace::PositiveZ
    };
    EditAction::Dig { hit, face }
}

fn copied_action(
    observed: ObservedAction,
    follower_eye: [f32; 3],
    placeable_material: Option<Material>,
    index: usize,
) -> Option<EditAction> {
    let follower = metres_to_voxel(follower_eye);
    match observed.action {
        EditAction::Dig { face, .. } => {
            let [offset_x, offset_z] = worksite_offset(index);
            Some(EditAction::Dig {
                hit: VoxelCoord::new(
                    follower.x + offset_x,
                    follower.y - 17,
                    follower.z + offset_z,
                ),
                face,
            })
        }
        EditAction::Place { .. } => {
            let [offset_x, offset_z] = worksite_offset(index);
            placeable_material.map(|material| EditAction::Place {
                coord: VoxelCoord::new(
                    follower.x + offset_x,
                    follower.y - 16 + (observed.serial % 20) as i32,
                    follower.z + offset_z,
                ),
                material,
            })
        }
    }
}

fn metres_to_voxel(position: [f32; 3]) -> VoxelCoord {
    VoxelCoord::new(
        (position[0] / VOXEL_SIZE_METRES).floor() as i32,
        (position[1] / VOXEL_SIZE_METRES).floor() as i32,
        (position[2] / VOXEL_SIZE_METRES).floor() as i32,
    )
}

fn normalize_yaw(yaw: f32) -> f32 {
    (yaw + PI).rem_euclid(TAU) - PI
}

fn worksite_offset(index: usize) -> [i32; 2] {
    const GRID_EDGE: i32 = 8;
    const SPACING_VOXELS: i32 = 6;
    let x = index as i32 % GRID_EDGE;
    let z = index as i32 / GRID_EDGE % GRID_EDGE;
    [
        (x * 2 - (GRID_EDGE - 1)) * SPACING_VOXELS / 2,
        (z * 2 - (GRID_EDGE - 1)) * SPACING_VOXELS / 2,
    ]
}

const fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> BehaviorContext {
        BehaviorContext {
            elapsed_seconds: 1.0,
            eye_position_metres: [2.0, 4.0, -3.0],
            placeable_material: None,
            leader_pose: None,
            leader_action: None,
        }
    }

    #[test]
    fn mixed_roster_cycles_through_all_behaviors() {
        assert_eq!(BehaviorKind::for_index(0), BehaviorKind::Explorer);
        assert_eq!(BehaviorKind::for_index(1), BehaviorKind::Digger);
        assert_eq!(BehaviorKind::for_index(2), BehaviorKind::Builder);
        assert_eq!(BehaviorKind::for_index(3), BehaviorKind::Follower);
        assert_eq!(BehaviorKind::for_index(4), BehaviorKind::Explorer);
    }

    #[test]
    fn explorer_is_deterministic_and_keeps_moving() {
        let mut first = BehaviorState::new(BehaviorKind::Explorer, BotLayout::Mixed, 3, 41);
        let mut second = BehaviorState::new(BehaviorKind::Explorer, BotLayout::Mixed, 3, 41);
        let first_intent = first.plan(context());
        let second_intent = second.plan(context());
        assert_eq!(first_intent.yaw_radians, second_intent.yaw_radians);
        assert!(first_intent.forward);
        assert!(first_intent.sprint);
    }

    #[test]
    fn digger_alternates_reachable_downward_and_horizontal_work() {
        let mut state = BehaviorState::new(BehaviorKind::Digger, BotLayout::Mixed, 1, 7);
        let mut sample = context();
        let first = state.plan(sample);
        assert!(matches!(
            first.edit,
            Some(EditAction::Dig {
                face: VoxelFace::PositiveY,
                ..
            })
        ));
        for step in 2..=7 {
            sample.elapsed_seconds = step as f32 * DECISION_INTERVAL_SECONDS;
            let _ = state.plan(sample);
        }
        sample.elapsed_seconds += DECISION_INTERVAL_SECONDS;
        let later = state.plan(sample);
        assert!(matches!(
            later.edit,
            Some(EditAction::Dig {
                face: VoxelFace::NegativeX
                    | VoxelFace::PositiveX
                    | VoxelFace::NegativeZ
                    | VoxelFace::PositiveZ,
                ..
            })
        ));
    }

    #[test]
    fn builder_mines_before_placing_and_then_builds_upward() {
        let mut state = BehaviorState::new(BehaviorKind::Builder, BotLayout::Dense, 2, 9);
        assert!(matches!(
            state.plan(context()).edit,
            Some(EditAction::Dig { .. })
        ));
        let mut supplied = context();
        supplied.elapsed_seconds = 2.0;
        supplied.placeable_material = Some(Material::Stone);
        assert!(matches!(
            state.plan(supplied).edit,
            Some(EditAction::Place {
                material: Material::Stone,
                ..
            })
        ));
    }

    #[test]
    fn follower_moves_to_leader_and_copies_each_action_once() {
        let mut state = BehaviorState::new(BehaviorKind::Follower, BotLayout::Mixed, 3, 11);
        let mut sample = context();
        sample.leader_pose = Some(LeaderPose {
            eye_position_metres: [8.0, 4.0, -3.0],
            yaw_radians: std::f32::consts::FRAC_PI_2,
        });
        sample.leader_action = Some(ObservedAction {
            serial: 5,
            action: EditAction::Dig {
                hit: VoxelCoord::new(80, 20, -30),
                face: VoxelFace::PositiveY,
            },
        });
        let copied = state.plan(sample);
        assert!(copied.forward);
        assert!(copied.copied_leader_action);
        assert!(matches!(copied.edit, Some(EditAction::Dig { .. })));
        sample.elapsed_seconds += DECISION_INTERVAL_SECONDS;
        let repeated = state.plan(sample);
        assert!(!repeated.copied_leader_action);
    }
}
