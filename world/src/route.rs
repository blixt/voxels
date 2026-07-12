use crate::FEATURE_CELL_VOXELS;

pub const ROUTE_CORE_HALF_WIDTH_VOXELS: f32 = 18.0;
pub const ROUTE_SHOULDER_WIDTH_VOXELS: f32 = 54.0;
pub const ROUTE_TOKEN_CADENCE_VOXELS: f32 = 288.0;
pub const ROUTE_TOKEN_SIDE_OFFSET_VOXELS: f32 = 32.0;
const ROUTE_TOKEN_START_VOXELS: f32 = ROUTE_TOKEN_CADENCE_VOXELS * 0.5;
const ROUTE_TOKEN_END_MARGIN_VOXELS: f32 = 96.0;
const FEATURE_ANCHOR_MARGIN_VOXELS: i32 = 12;
pub const FIRST_PILGRIM_ROAD_BOUNDS: [[i32; 2]; 2] = [[-1_234, -6], [55, 1_249]];

#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RouteId {
    #[default]
    FirstPilgrimRoad = 0,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RouteNode {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

/// Authored in canonical 10 cm voxel coordinates after a terrain-aware coarse search. The road begins
/// beside spawn, crosses forest, moor, and badlands, then terminates at a prominent hoodoo. Its node
/// grades require at most a 30 cm cut or 20 cm fill against generator v8's route-free terrain.
pub const FIRST_PILGRIM_ROAD_NODES: [RouteNode; 6] = [
    RouteNode { x: 0, y: 35, z: 48 },
    RouteNode {
        x: -256,
        y: 36,
        z: 304,
    },
    RouteNode {
        x: -512,
        y: 30,
        z: 528,
    },
    RouteNode {
        x: -752,
        y: 31,
        z: 784,
    },
    RouteNode {
        x: -992,
        y: 43,
        z: 1_040,
    },
    RouteNode {
        x: -1_180,
        y: 36,
        z: 1_194,
    },
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RouteSample {
    pub route_id: RouteId,
    pub segment_index: u8,
    pub segment_t: f32,
    pub closest: [f32; 2],
    pub tangent: [f32; 2],
    pub signed_lateral_voxels: f32,
    pub distance_to_route_voxels: f32,
    pub distance_along_voxels: f32,
    /// One throughout the road bed and a smooth fade through the outer shoulder.
    pub terrain_blend: f32,
    /// Shape-matched visual core, one across most of the bed and zero at its outer edge.
    pub core: f32,
    /// Smooth shoulder influence outside the core, one at the core edge and zero at maximum reach.
    pub shoulder: f32,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RouteAnchorRole {
    #[default]
    Cairn = 0,
    Waystone = 1,
    RuinedArch = 2,
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RouteLandmarkId {
    pub route_id: RouteId,
    pub ordinal: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RouteAnchor {
    pub route_id: RouteId,
    pub ordinal: u16,
    pub role: RouteAnchorRole,
    pub feature_cell: [i32; 2],
    pub anchor: [i32; 2],
    pub distance_along_voxels: i32,
}

pub fn sample_first_pilgrim_road(x: i32, z: i32) -> Option<RouteSample> {
    let [[min_x, min_z], [max_x, max_z]] = FIRST_PILGRIM_ROAD_BOUNDS;
    if !(min_x..max_x).contains(&x) || !(min_z..max_z).contains(&z) {
        return None;
    }
    sample_polyline(
        RouteId::FirstPilgrimRoad,
        &FIRST_PILGRIM_ROAD_NODES,
        x as f32,
        z as f32,
    )
    .filter(|sample| sample.distance_to_route_voxels <= ROUTE_SHOULDER_WIDTH_VOXELS)
}

pub fn first_pilgrim_road_length_voxels() -> f32 {
    polyline_length(&FIRST_PILGRIM_ROAD_NODES)
}

pub fn first_pilgrim_road_point_at_distance(distance: f32) -> Option<([f32; 2], [f32; 2])> {
    point_and_tangent_at_distance(&FIRST_PILGRIM_ROAD_NODES, distance)
}

pub fn first_pilgrim_route_anchor_count() -> u16 {
    let usable = first_pilgrim_road_length_voxels()
        - ROUTE_TOKEN_START_VOXELS
        - ROUTE_TOKEN_END_MARGIN_VOXELS;
    if usable <= 0.0 {
        0
    } else {
        (usable / ROUTE_TOKEN_CADENCE_VOXELS).floor() as u16 + 1
    }
}

pub fn first_pilgrim_route_anchor(ordinal: u16) -> Option<RouteAnchor> {
    if ordinal >= first_pilgrim_route_anchor_count() {
        return None;
    }
    let distance = ROUTE_TOKEN_START_VOXELS + f32::from(ordinal) * ROUTE_TOKEN_CADENCE_VOXELS;
    let (point, tangent) = point_and_tangent_at_distance(&FIRST_PILGRIM_ROAD_NODES, distance)?;
    let side = if ordinal & 1 == 0 { 1.0 } else { -1.0 };
    let raw = [
        point[0] - tangent[1] * ROUTE_TOKEN_SIDE_OFFSET_VOXELS * side,
        point[1] + tangent[0] * ROUTE_TOKEN_SIDE_OFFSET_VOXELS * side,
    ];
    let raw = [raw[0].round() as i32, raw[1].round() as i32];
    let feature_cell = [
        raw[0].div_euclid(FEATURE_CELL_VOXELS),
        raw[1].div_euclid(FEATURE_CELL_VOXELS),
    ];
    let cell_min = [
        feature_cell[0] * FEATURE_CELL_VOXELS,
        feature_cell[1] * FEATURE_CELL_VOXELS,
    ];
    let anchor = [
        raw[0].clamp(
            cell_min[0] + FEATURE_ANCHOR_MARGIN_VOXELS,
            cell_min[0] + FEATURE_CELL_VOXELS - FEATURE_ANCHOR_MARGIN_VOXELS - 1,
        ),
        raw[1].clamp(
            cell_min[1] + FEATURE_ANCHOR_MARGIN_VOXELS,
            cell_min[1] + FEATURE_CELL_VOXELS - FEATURE_ANCHOR_MARGIN_VOXELS - 1,
        ),
    ];
    let role = if ordinal % 3 == 0 {
        RouteAnchorRole::RuinedArch
    } else if ordinal & 1 == 0 {
        RouteAnchorRole::Waystone
    } else {
        RouteAnchorRole::Cairn
    };
    Some(RouteAnchor {
        route_id: RouteId::FirstPilgrimRoad,
        ordinal,
        role,
        feature_cell,
        anchor,
        distance_along_voxels: distance.round() as i32,
    })
}

pub fn first_pilgrim_route_anchor_for_feature_cell(
    cell_x: i32,
    cell_z: i32,
) -> Option<RouteAnchor> {
    (0..first_pilgrim_route_anchor_count())
        .filter_map(first_pilgrim_route_anchor)
        .find(|anchor| anchor.feature_cell == [cell_x, cell_z])
}

fn sample_polyline(route_id: RouteId, nodes: &[RouteNode], x: f32, z: f32) -> Option<RouteSample> {
    let mut accumulated = 0.0;
    let mut best: Option<RouteSample> = None;
    for (segment_index, pair) in nodes.windows(2).enumerate() {
        let from = [pair[0].x as f32, pair[0].z as f32];
        let delta = [pair[1].x as f32 - from[0], pair[1].z as f32 - from[1]];
        let length_squared = delta[0] * delta[0] + delta[1] * delta[1];
        if length_squared <= f32::EPSILON {
            continue;
        }
        let length = length_squared.sqrt();
        let tangent = [delta[0] / length, delta[1] / length];
        let relative = [x - from[0], z - from[1]];
        let t =
            ((relative[0] * delta[0] + relative[1] * delta[1]) / length_squared).clamp(0.0, 1.0);
        let closest = [from[0] + delta[0] * t, from[1] + delta[1] * t];
        let offset = [x - closest[0], z - closest[1]];
        let distance = (offset[0] * offset[0] + offset[1] * offset[1]).sqrt();
        if best
            .as_ref()
            .is_some_and(|candidate| candidate.distance_to_route_voxels <= distance)
        {
            accumulated += length;
            continue;
        }
        let normalized_core = (distance / ROUTE_CORE_HALF_WIDTH_VOXELS).clamp(0.0, 1.0);
        let core = 1.0 - smooth(((normalized_core - 0.70) / 0.30).clamp(0.0, 1.0));
        let shoulder_t = ((distance - ROUTE_CORE_HALF_WIDTH_VOXELS)
            / (ROUTE_SHOULDER_WIDTH_VOXELS - ROUTE_CORE_HALF_WIDTH_VOXELS))
            .clamp(0.0, 1.0);
        let shoulder = 1.0 - smooth(shoulder_t);
        let terrain_blend = if distance <= ROUTE_CORE_HALF_WIDTH_VOXELS {
            1.0
        } else {
            shoulder
        };
        best = Some(RouteSample {
            route_id,
            segment_index: segment_index as u8,
            segment_t: t,
            closest,
            tangent,
            signed_lateral_voxels: tangent[0] * offset[1] - tangent[1] * offset[0],
            distance_to_route_voxels: distance,
            distance_along_voxels: accumulated + length * t,
            terrain_blend,
            core,
            shoulder,
        });
        accumulated += length;
    }
    best
}

fn point_and_tangent_at_distance(
    nodes: &[RouteNode],
    distance: f32,
) -> Option<([f32; 2], [f32; 2])> {
    let mut remaining = distance.max(0.0);
    for pair in nodes.windows(2) {
        let from = [pair[0].x as f32, pair[0].z as f32];
        let delta = [pair[1].x as f32 - from[0], pair[1].z as f32 - from[1]];
        let length = (delta[0] * delta[0] + delta[1] * delta[1]).sqrt();
        if length <= f32::EPSILON {
            continue;
        }
        let tangent = [delta[0] / length, delta[1] / length];
        if remaining <= length {
            return Some((
                [
                    from[0] + tangent[0] * remaining,
                    from[1] + tangent[1] * remaining,
                ],
                tangent,
            ));
        }
        remaining -= length;
    }
    let pair = nodes.windows(2).next_back()?;
    let delta = [
        pair[1].x as f32 - pair[0].x as f32,
        pair[1].z as f32 - pair[0].z as f32,
    ];
    let length = (delta[0] * delta[0] + delta[1] * delta[1]).sqrt();
    (length > f32::EPSILON).then_some((
        [pair[1].x as f32, pair[1].z as f32],
        [delta[0] / length, delta[1] / length],
    ))
}

fn polyline_length(nodes: &[RouteNode]) -> f32 {
    nodes
        .windows(2)
        .map(|pair| {
            let dx = (pair[1].x - pair[0].x) as f32;
            let dz = (pair[1].z - pair[0].z) as f32;
            (dx * dx + dz * dz).sqrt()
        })
        .sum()
}

fn smooth(value: f32) -> f32 {
    value * value * (3.0 - 2.0 * value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn route_projection_is_continuous_and_distance_along_is_monotonic() {
        let mut previous = -1.0;
        for distance in (0..first_pilgrim_road_length_voxels() as i32).step_by(7) {
            let (point, _) =
                point_and_tangent_at_distance(&FIRST_PILGRIM_ROAD_NODES, distance as f32).unwrap();
            let sample =
                sample_first_pilgrim_road(point[0].round() as i32, point[1].round() as i32)
                    .unwrap();
            assert!(sample.distance_to_route_voxels <= 1.0);
            assert!(sample.distance_along_voxels + 1.5 >= previous);
            previous = sample.distance_along_voxels;
        }
    }

    #[test]
    fn route_core_and_shoulder_have_explicit_bounded_reach() {
        let from = FIRST_PILGRIM_ROAD_NODES[0];
        let to = FIRST_PILGRIM_ROAD_NODES[1];
        let point = RouteNode {
            x: (from.x + to.x) / 2,
            y: (from.y + to.y) / 2,
            z: (from.z + to.z) / 2,
        };
        let center = sample_first_pilgrim_road(point.x, point.z).unwrap();
        assert!(center.core > 0.99 && center.terrain_blend > 0.99);
        let tangent = center.tangent;
        let offset = |distance: f32| {
            [
                (point.x as f32 - tangent[1] * distance).round() as i32,
                (point.z as f32 + tangent[0] * distance).round() as i32,
            ]
        };
        let shoulder = offset(36.0);
        let shoulder = sample_first_pilgrim_road(shoulder[0], shoulder[1]).unwrap();
        assert_eq!(shoulder.core, 0.0);
        assert!(shoulder.shoulder > 0.0 && shoulder.terrain_blend > 0.0);
        let outside = offset(ROUTE_SHOULDER_WIDTH_VOXELS + 2.0);
        assert!(sample_first_pilgrim_road(outside[0], outside[1]).is_none());
    }

    #[test]
    fn route_tokens_are_unique_alternating_and_inside_feature_cells() {
        let mut cells = BTreeSet::new();
        let count = first_pilgrim_route_anchor_count();
        assert!(count >= 5);
        let mut previous_distance = 0;
        for ordinal in 0..count {
            let anchor = first_pilgrim_route_anchor(ordinal).unwrap();
            assert!(cells.insert(anchor.feature_cell));
            if ordinal > 0 {
                let gap = anchor.distance_along_voxels - previous_distance;
                assert!((287..=289).contains(&gap));
            }
            previous_distance = anchor.distance_along_voxels;
            let cell_min = [
                anchor.feature_cell[0] * FEATURE_CELL_VOXELS,
                anchor.feature_cell[1] * FEATURE_CELL_VOXELS,
            ];
            assert!(
                (cell_min[0] + FEATURE_ANCHOR_MARGIN_VOXELS
                    ..cell_min[0] + FEATURE_CELL_VOXELS - FEATURE_ANCHOR_MARGIN_VOXELS)
                    .contains(&anchor.anchor[0])
            );
            assert!(
                (cell_min[1] + FEATURE_ANCHOR_MARGIN_VOXELS
                    ..cell_min[1] + FEATURE_CELL_VOXELS - FEATURE_ANCHOR_MARGIN_VOXELS)
                    .contains(&anchor.anchor[1])
            );
            assert_eq!(
                first_pilgrim_route_anchor_for_feature_cell(
                    anchor.feature_cell[0],
                    anchor.feature_cell[1]
                ),
                Some(anchor)
            );
        }
    }

    #[test]
    fn route_begins_at_spawn_and_reaches_negative_space() {
        assert_eq!(
            FIRST_PILGRIM_ROAD_NODES[0],
            RouteNode { x: 0, y: 35, z: 48 }
        );
        assert!(FIRST_PILGRIM_ROAD_NODES.last().unwrap().x < 0);
        let spawn = sample_first_pilgrim_road(0, 52).unwrap();
        assert!(spawn.distance_to_route_voxels < ROUTE_CORE_HALF_WIDTH_VOXELS);
        assert!(spawn.core > 0.5);
    }

    #[test]
    fn fast_reject_bounds_contain_every_authored_node_and_shoulder() {
        for node in FIRST_PILGRIM_ROAD_NODES {
            assert!(sample_first_pilgrim_road(node.x, node.z).is_some());
            for [dx, dz] in [
                [-ROUTE_SHOULDER_WIDTH_VOXELS as i32, 0],
                [ROUTE_SHOULDER_WIDTH_VOXELS as i32, 0],
                [0, -ROUTE_SHOULDER_WIDTH_VOXELS as i32],
                [0, ROUTE_SHOULDER_WIDTH_VOXELS as i32],
            ] {
                // A cardinal shoulder point can be farther than the route's radial reach because the
                // segment is diagonal, but the manually duplicated broad-phase must never reject it.
                let x = node.x + dx;
                let z = node.z + dz;
                let [[min_x, min_z], [max_x, max_z]] = FIRST_PILGRIM_ROAD_BOUNDS;
                let broad_phase_contains =
                    (min_x..max_x).contains(&x) && (min_z..max_z).contains(&z);
                assert!(broad_phase_contains);
            }
        }
    }
}
