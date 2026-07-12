use crate::{CaveSystemId, Material};
use std::sync::LazyLock;

pub const CINDER_VAULT_BOUNDS: [[i32; 3]; 2] = [[-5_236, 2, 3_158], [-5_004, 77, 3_342]];
const CAVE_X_BIN_EDGE: i32 = 32;
const CAVE_X_BIN_COUNT: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaveNode {
    pub center: [i32; 3],
    pub horizontal_radius: i32,
    pub vertical_radius: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaveEdge {
    pub from: u8,
    pub to: u8,
    pub horizontal_radius: i32,
    pub vertical_radius: i32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaveSample {
    pub system_id: CaveSystemId,
    /// Zero at a primitive centerline and one at its carved boundary.
    pub normalized_distance: f32,
}

pub const CINDER_VAULT_NODES: [CaveNode; 7] = [
    CaveNode {
        center: [-5_020, 60, 3_178],
        horizontal_radius: 10,
        vertical_radius: 12,
    },
    CaveNode {
        center: [-5_028, 50, 3_192],
        horizontal_radius: 11,
        vertical_radius: 14,
    },
    CaveNode {
        center: [-5_060, 42, 3_228],
        horizontal_radius: 10,
        vertical_radius: 13,
    },
    CaveNode {
        center: [-5_100, 34, 3_268],
        horizontal_radius: 10,
        vertical_radius: 13,
    },
    CaveNode {
        center: [-5_140, 28, 3_290],
        horizontal_radius: 14,
        vertical_radius: 15,
    },
    CaveNode {
        center: [-5_180, 24, 3_300],
        horizontal_radius: 26,
        vertical_radius: 17,
    },
    CaveNode {
        center: [-5_212, 25, 3_320],
        horizontal_radius: 15,
        vertical_radius: 13,
    },
];

pub const CINDER_VAULT_EDGES: [CaveEdge; 6] = [
    CaveEdge {
        from: 0,
        to: 1,
        horizontal_radius: 9,
        vertical_radius: 12,
    },
    CaveEdge {
        from: 1,
        to: 2,
        horizontal_radius: 9,
        vertical_radius: 12,
    },
    CaveEdge {
        from: 2,
        to: 3,
        horizontal_radius: 9,
        vertical_radius: 12,
    },
    CaveEdge {
        from: 3,
        to: 4,
        horizontal_radius: 9,
        vertical_radius: 12,
    },
    CaveEdge {
        from: 4,
        to: 5,
        horizontal_radius: 11,
        vertical_radius: 13,
    },
    CaveEdge {
        from: 5,
        to: 6,
        horizontal_radius: 11,
        vertical_radius: 12,
    },
];

static CINDER_VAULT_X_BIN_MASKS: LazyLock<[u16; CAVE_X_BIN_COUNT]> = LazyLock::new(|| {
    let mut masks = [0u16; CAVE_X_BIN_COUNT];
    let min_x = CINDER_VAULT_BOUNDS[0][0];
    for (index, node) in CINDER_VAULT_NODES.iter().enumerate() {
        let reach = (node.horizontal_radius * 13 + 9) / 10;
        mark_x_bins(
            &mut masks,
            node.center[0] - reach,
            node.center[0] + reach,
            index,
        );
    }
    for (index, edge) in CINDER_VAULT_EDGES.iter().enumerate() {
        let from = CINDER_VAULT_NODES[usize::from(edge.from)].center;
        let to = CINDER_VAULT_NODES[usize::from(edge.to)].center;
        let reach = (edge.horizontal_radius * 13 + 9) / 10;
        mark_x_bins(
            &mut masks,
            from[0].min(to[0]) - reach,
            from[0].max(to[0]) + reach,
            CINDER_VAULT_NODES.len() + index,
        );
    }
    debug_assert_eq!(min_x, -5_236);
    masks
});

fn mark_x_bins(
    masks: &mut [u16; CAVE_X_BIN_COUNT],
    primitive_min_x: i32,
    primitive_max_x: i32,
    primitive_index: usize,
) {
    let min_x = CINDER_VAULT_BOUNDS[0][0];
    let first = (primitive_min_x - min_x)
        .div_euclid(CAVE_X_BIN_EDGE)
        .clamp(0, CAVE_X_BIN_COUNT as i32 - 1);
    let last = (primitive_max_x - min_x)
        .div_euclid(CAVE_X_BIN_EDGE)
        .clamp(0, CAVE_X_BIN_COUNT as i32 - 1);
    for bin in first..=last {
        masks[bin as usize] |= 1 << primitive_index;
    }
}

pub fn sample_cinder_vault(x: i32, y: i32, z: i32) -> Option<CaveSample> {
    let distance_squared = cinder_vault_distance_squared(x, y, z)?;
    (distance_squared <= 1.0).then_some(CaveSample {
        system_id: CaveSystemId::CinderVault,
        normalized_distance: distance_squared.sqrt(),
    })
}

/// Final canonical CSG authority. `Air` is a protected authored void; `Basalt` is the sealed shell
/// that prevents ambient noise caves and surface features from puncturing the authored system.
pub fn cinder_vault_override(x: i32, y: i32, z: i32) -> Option<Material> {
    let distance_squared = cinder_vault_distance_squared(x, y, z)?;
    if distance_squared <= 1.0 {
        Some(Material::Air)
    } else if distance_squared <= 1.30 * 1.30 {
        Some(Material::Basalt)
    } else {
        None
    }
}

fn cinder_vault_distance_squared(x: i32, y: i32, z: i32) -> Option<f32> {
    let [[min_x, min_y, min_z], [max_x, max_y, max_z]] = CINDER_VAULT_BOUNDS;
    if !(min_x..max_x).contains(&x) || !(min_y..max_y).contains(&y) || !(min_z..max_z).contains(&z)
    {
        return None;
    }
    let point = [x as f32, y as f32, z as f32];
    let mut nearest = f32::INFINITY;
    let bin = (x - min_x).div_euclid(CAVE_X_BIN_EDGE) as usize;
    let mut candidates = CINDER_VAULT_X_BIN_MASKS[bin];
    while candidates != 0 {
        let primitive = candidates.trailing_zeros() as usize;
        candidates &= candidates - 1;
        if primitive < CINDER_VAULT_NODES.len() {
            let node = CINDER_VAULT_NODES[primitive];
            if node_may_reach(point, node) {
                nearest = nearest.min(ellipsoid_distance(point, node));
            }
        } else {
            let edge = CINDER_VAULT_EDGES[primitive - CINDER_VAULT_NODES.len()];
            if edge_may_reach(point, edge) {
                nearest = nearest.min(edge_distance(point, edge));
            }
        }
    }
    nearest.is_finite().then_some(nearest)
}

fn node_may_reach(point: [f32; 3], node: CaveNode) -> bool {
    let horizontal = node.horizontal_radius as f32 * 1.30;
    let vertical = node.vertical_radius as f32 * 1.30;
    (point[0] - node.center[0] as f32).abs() <= horizontal
        && (point[1] - node.center[1] as f32).abs() <= vertical
        && (point[2] - node.center[2] as f32).abs() <= horizontal
}

fn edge_may_reach(point: [f32; 3], edge: CaveEdge) -> bool {
    let from = CINDER_VAULT_NODES[usize::from(edge.from)].center;
    let to = CINDER_VAULT_NODES[usize::from(edge.to)].center;
    let horizontal = edge.horizontal_radius as f32 * 1.30;
    let vertical = edge.vertical_radius as f32 * 1.30;
    point[0] >= from[0].min(to[0]) as f32 - horizontal
        && point[0] <= from[0].max(to[0]) as f32 + horizontal
        && point[1] >= from[1].min(to[1]) as f32 - vertical
        && point[1] <= from[1].max(to[1]) as f32 + vertical
        && point[2] >= from[2].min(to[2]) as f32 - horizontal
        && point[2] <= from[2].max(to[2]) as f32 + horizontal
}

fn ellipsoid_distance(point: [f32; 3], node: CaveNode) -> f32 {
    let horizontal = node.horizontal_radius as f32;
    let vertical = node.vertical_radius as f32;
    let delta = [
        (point[0] - node.center[0] as f32) / horizontal,
        (point[1] - node.center[1] as f32) / vertical,
        (point[2] - node.center[2] as f32) / horizontal,
    ];
    delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]
}

fn edge_distance(point: [f32; 3], edge: CaveEdge) -> f32 {
    let from = CINDER_VAULT_NODES[usize::from(edge.from)].center;
    let to = CINDER_VAULT_NODES[usize::from(edge.to)].center;
    let scale = [
        edge.horizontal_radius as f32,
        edge.vertical_radius as f32,
        edge.horizontal_radius as f32,
    ];
    let from: [f32; 3] = std::array::from_fn(|axis| from[axis] as f32 / scale[axis]);
    let to: [f32; 3] = std::array::from_fn(|axis| to[axis] as f32 / scale[axis]);
    let point: [f32; 3] = std::array::from_fn(|axis| point[axis] / scale[axis]);
    let delta = std::array::from_fn::<_, 3, _>(|axis| to[axis] - from[axis]);
    let relative = std::array::from_fn::<_, 3, _>(|axis| point[axis] - from[axis]);
    let length_squared = dot(delta, delta);
    let t = (dot(relative, delta) / length_squared).clamp(0.0, 1.0);
    let offset = std::array::from_fn(|axis| relative[axis] - delta[axis] * t);
    dot(offset, offset)
}

fn dot(left: [f32; 3], right: [f32; 3]) -> f32 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_graph_node_and_edge_centerline_is_inside_the_cinder_vault() {
        for node in CINDER_VAULT_NODES {
            assert!(sample_cinder_vault(node.center[0], node.center[1], node.center[2]).is_some());
        }
        for edge in CINDER_VAULT_EDGES {
            let from = CINDER_VAULT_NODES[usize::from(edge.from)].center;
            let to = CINDER_VAULT_NODES[usize::from(edge.to)].center;
            for step in 0..=32 {
                let point: [i32; 3] = std::array::from_fn(|axis| {
                    (from[axis] as f32 + (to[axis] - from[axis]) as f32 * step as f32 / 32.0)
                        .round() as i32
                });
                assert!(sample_cinder_vault(point[0], point[1], point[2]).is_some());
            }
        }
    }

    #[test]
    fn cave_bounds_are_half_open_and_reject_unrelated_world_space() {
        let [[min_x, min_y, min_z], [max_x, max_y, max_z]] = CINDER_VAULT_BOUNDS;
        for point in [
            [min_x - 1, 24, 3_300],
            [max_x, 24, 3_300],
            [-5_180, min_y - 1, 3_300],
            [-5_180, max_y, 3_300],
            [-5_180, 24, min_z - 1],
            [-5_180, 24, max_z],
            [0, 0, 0],
        ] {
            assert!(sample_cinder_vault(point[0], point[1], point[2]).is_none());
        }
    }

    #[test]
    fn main_tunnel_has_player_width_and_headroom() {
        for edge in &CINDER_VAULT_EDGES[..5] {
            assert!(edge.horizontal_radius >= 9);
            assert!(edge.vertical_radius >= 12);
        }
    }

    #[test]
    fn authored_shell_is_solid_outside_the_protected_void() {
        let chamber = CINDER_VAULT_NODES[5];
        assert_eq!(
            cinder_vault_override(
                chamber.center[0],
                chamber.center[1],
                chamber.center[2] - chamber.horizontal_radius
            ),
            Some(Material::Air)
        );
        assert_eq!(
            cinder_vault_override(
                chamber.center[0],
                chamber.center[1],
                chamber.center[2] - chamber.horizontal_radius - 4
            ),
            Some(Material::Basalt)
        );
        assert_eq!(
            cinder_vault_override(
                chamber.center[0],
                chamber.center[1],
                chamber.center[2] - chamber.horizontal_radius - 12
            ),
            None
        );
    }
}
