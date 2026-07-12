use crate::{
    CHUNK_EDGE, CINDER_VAULT, CaveSystemId, ChunkCoord, Material, PortalState, VOXEL_SIZE_METRES,
    VisibilityCellId, VisibilityGraph, VisibilityGraphError, VisibilityPortal,
};
use std::sync::LazyLock;

pub const CINDER_VAULT_BOUNDS: [[i32; 3]; 2] = [[-5_236, 2, 3_158], [-5_004, 77, 3_342]];
pub const CINDER_VAULT_MOUTH_CELL: [i32; 2] = [-53, 33];
pub const CINDER_VAULT_MOUTH_ANCHOR_XZ: [i32; 2] = [-5_020, 3_186];
pub const CINDER_VAULT_TOPOLOGY_VERSION: u16 = 1;
pub const CINDER_VAULT_VISIBILITY_CELL_COUNT: usize = CINDER_VAULT_NODES.len() + 1;
pub const CINDER_VAULT_PORTAL_COUNT: usize = CINDER_VAULT_EDGES.len() + 1;
pub const CINDER_VAULT_EXTERIOR_CELL: VisibilityCellId = VisibilityCellId::new(0);
pub const CINDER_VAULT_PORTAL_OPEN_LANES: usize = 4;
pub const CINDER_VAULT_PORTAL_PROBE_EDGE: i32 = 5;
pub const CINDER_VAULT_STREAM_INTEREST_CAPACITY: usize = 192;
pub const CINDER_VAULT_STREAM_ACTIVATION_MARGIN_VOXELS: i32 = 96;
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaveCrystalFormation {
    /// Inclusive base voxel in canonical 10 cm world coordinates.
    pub base: [i32; 3],
    pub height: u8,
    pub base_radius: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CavePortalProbe {
    pub center: [i32; 3],
    pub axis_u: [i32; 3],
    pub axis_v: [i32; 3],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaveStreamInterest {
    chunks: [ChunkCoord; CINDER_VAULT_STREAM_INTEREST_CAPACITY],
    len: u8,
    overflowed: bool,
}

impl CaveStreamInterest {
    pub const fn empty() -> Self {
        Self {
            chunks: [ChunkCoord::new(0, 0, 0); CINDER_VAULT_STREAM_INTEREST_CAPACITY],
            len: 0,
            overflowed: false,
        }
    }

    pub fn as_slice(&self) -> &[ChunkCoord] {
        &self.chunks[..self.len as usize]
    }

    pub const fn len(&self) -> usize {
        self.len as usize
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub const fn overflowed(&self) -> bool {
        self.overflowed
    }

    fn push_unique(&mut self, coord: ChunkCoord) {
        if self.as_slice().contains(&coord) {
            return;
        }
        if self.len() == CINDER_VAULT_STREAM_INTEREST_CAPACITY {
            self.overflowed = true;
            return;
        }
        self.chunks[self.len()] = coord;
        self.len += 1;
    }

    fn sort_nearest(&mut self, focus: ChunkCoord) {
        self.chunks[..self.len as usize].sort_unstable_by_key(|coord| {
            let dx = i64::from(coord.x) - i64::from(focus.x);
            let dy = i64::from(coord.y) - i64::from(focus.y);
            let dz = i64::from(coord.z) - i64::from(focus.z);
            (dx * dx + dz * dz + dy * dy * 4, coord.y, coord.z, coord.x)
        });
    }
}

impl Default for CaveStreamInterest {
    fn default() -> Self {
        Self::empty()
    }
}

/// Sparse authored mineral cues remain ordinary voxels: they mesh, collide, edit, persist, and
/// round-trip through the same versioned palette codec as terrain.
pub const CINDER_VAULT_CRYSTALS: [CaveCrystalFormation; 4] = [
    CaveCrystalFormation {
        base: [-5_192, 10, 3_293],
        height: 11,
        base_radius: 2,
    },
    CaveCrystalFormation {
        base: [-5_162, 13, 3_308],
        height: 9,
        base_radius: 2,
    },
    CaveCrystalFormation {
        base: [-5_208, 19, 3_332],
        height: 8,
        base_radius: 2,
    },
    CaveCrystalFormation {
        base: [-5_180, 18, 3_276],
        height: 8,
        base_radius: 2,
    },
];

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

static CINDER_VAULT_VISIBILITY_GRAPH: LazyLock<Result<VisibilityGraph, VisibilityGraphError>> =
    LazyLock::new(|| {
        let mut portals =
            [VisibilityPortal::new(CINDER_VAULT_EXTERIOR_CELL, VisibilityCellId::new(1), 0.0);
                CINDER_VAULT_PORTAL_COUNT];
        portals[0] = VisibilityPortal::new(
            CINDER_VAULT_EXTERIOR_CELL,
            VisibilityCellId::new(1),
            node_distance_metres(CINDER_VAULT.entrance, CINDER_VAULT_NODES[0].center),
        );
        for (index, edge) in CINDER_VAULT_EDGES.iter().enumerate() {
            portals[index + 1] = VisibilityPortal::new(
                VisibilityCellId::new(edge.from + 1),
                VisibilityCellId::new(edge.to + 1),
                node_distance_metres(
                    CINDER_VAULT_NODES[usize::from(edge.from)].center,
                    CINDER_VAULT_NODES[usize::from(edge.to)].center,
                ),
            );
        }
        VisibilityGraph::new(CINDER_VAULT_VISIBILITY_CELL_COUNT, &portals)
    });

pub fn cinder_vault_visibility_graph() -> Result<&'static VisibilityGraph, VisibilityGraphError> {
    CINDER_VAULT_VISIBILITY_GRAPH
        .as_ref()
        .map_err(|error| *error)
}

pub fn cinder_vault_visibility_cell(x: i32, y: i32, z: i32) -> VisibilityCellId {
    if sample_cinder_vault(x, y, z).is_none() {
        return CINDER_VAULT_EXTERIOR_CELL;
    }
    let point = [x, y, z];
    let nearest = CINDER_VAULT_NODES
        .iter()
        .enumerate()
        .min_by_key(|(index, node)| (squared_distance(point, node.center), *index))
        .map_or(0, |(index, _)| index);
    VisibilityCellId::new(nearest as u8 + 1)
}

/// Returns bounded full-resolution look-ahead along the open authored cave spine.
///
/// The radial streamer remains authoritative around the camera. This secondary set only exists
/// near Cinder Vault and follows cells reachable through the current edit-derived portal mask.
pub fn cinder_vault_stream_interest(
    camera_voxel: [i32; 3],
    portal_state: PortalState,
) -> CaveStreamInterest {
    if !inside_expanded_cinder_bounds(camera_voxel) {
        return CaveStreamInterest::empty();
    }
    let Ok(graph) = cinder_vault_visibility_graph() else {
        return CaveStreamInterest::empty();
    };
    let from = cinder_vault_visibility_cell(camera_voxel[0], camera_voxel[1], camera_voxel[2]);
    let mut reachable = [false; CINDER_VAULT_VISIBILITY_CELL_COUNT];
    for (index, value) in reachable.iter_mut().enumerate() {
        *value = graph
            .shortest_open_distance(from, VisibilityCellId::new(index as u8), portal_state)
            .is_some();
    }

    let mut interest = CaveStreamInterest::empty();
    for (node_index, node) in CINDER_VAULT_NODES.iter().enumerate() {
        if reachable[node_index + 1] {
            let horizontal = (node.horizontal_radius * 13 + 9) / 10 + 1;
            let vertical = (node.vertical_radius * 13 + 9) / 10 + 1;
            push_voxel_bounds(
                &mut interest,
                [
                    node.center[0] - horizontal,
                    node.center[1] - vertical,
                    node.center[2] - horizontal,
                ],
                [
                    node.center[0] + horizontal,
                    node.center[1] + vertical,
                    node.center[2] + horizontal,
                ],
            );
        }
    }
    if portal_state.is_open(0) && reachable[0] && reachable[1] {
        let node = CINDER_VAULT_NODES[0];
        let horizontal = (node.horizontal_radius * 13 + 9) / 10 + 1;
        push_voxel_bounds(
            &mut interest,
            [
                CINDER_VAULT.entrance[0].min(node.center[0]) - horizontal,
                CINDER_VAULT.entrance[1].min(node.center[1]) - 1,
                CINDER_VAULT.entrance[2].min(node.center[2]) - horizontal,
            ],
            [
                CINDER_VAULT.entrance[0].max(node.center[0]) + horizontal,
                CINDER_VAULT.entrance[1].max(node.center[1]) + 1,
                CINDER_VAULT.entrance[2].max(node.center[2]) + horizontal,
            ],
        );
    }
    for (portal_index, edge) in CINDER_VAULT_EDGES.iter().enumerate() {
        let from_cell = usize::from(edge.from) + 1;
        let to_cell = usize::from(edge.to) + 1;
        if portal_state.is_open(portal_index + 1) && reachable[from_cell] && reachable[to_cell] {
            let horizontal = (edge.horizontal_radius * 13 + 9) / 10 + 1;
            let vertical = (edge.vertical_radius * 13 + 9) / 10 + 1;
            let from = CINDER_VAULT_NODES[usize::from(edge.from)].center;
            let to = CINDER_VAULT_NODES[usize::from(edge.to)].center;
            push_voxel_bounds(
                &mut interest,
                [
                    from[0].min(to[0]) - horizontal,
                    from[1].min(to[1]) - vertical,
                    from[2].min(to[2]) - horizontal,
                ],
                [
                    from[0].max(to[0]) + horizontal,
                    from[1].max(to[1]) + vertical,
                    from[2].max(to[2]) + horizontal,
                ],
            );
        }
    }
    interest.sort_nearest(voxel_chunk(camera_voxel));
    interest
}

fn inside_expanded_cinder_bounds(point: [i32; 3]) -> bool {
    (0..3).all(|axis| {
        point[axis] >= CINDER_VAULT_BOUNDS[0][axis] - CINDER_VAULT_STREAM_ACTIVATION_MARGIN_VOXELS
            && point[axis]
                < CINDER_VAULT_BOUNDS[1][axis] + CINDER_VAULT_STREAM_ACTIVATION_MARGIN_VOXELS
    })
}

fn push_voxel_bounds(interest: &mut CaveStreamInterest, min: [i32; 3], max: [i32; 3]) {
    let min = voxel_chunk(min);
    let max = voxel_chunk(max);
    for y in min.y..=max.y {
        for z in min.z..=max.z {
            for x in min.x..=max.x {
                interest.push_unique(ChunkCoord::new(x, y, z));
            }
        }
    }
}

const fn voxel_chunk(voxel: [i32; 3]) -> ChunkCoord {
    ChunkCoord::new(
        voxel[0].div_euclid(CHUNK_EDGE as i32),
        voxel[1].div_euclid(CHUNK_EDGE as i32),
        voxel[2].div_euclid(CHUNK_EDGE as i32),
    )
}

pub fn cinder_vault_portal_probe(portal_index: usize) -> Option<CavePortalProbe> {
    if portal_index == 0 {
        return Some(CavePortalProbe {
            center: [CINDER_VAULT.entrance[0], 49, CINDER_VAULT.entrance[2]],
            axis_u: [1, 0, 0],
            axis_v: [0, 0, 1],
        });
    }
    let edge = *CINDER_VAULT_EDGES.get(portal_index - 1)?;
    let from = CINDER_VAULT_NODES[usize::from(edge.from)].center;
    let to = CINDER_VAULT_NODES[usize::from(edge.to)].center;
    let center = std::array::from_fn(|axis| (from[axis] + to[axis]).div_euclid(2));
    let tangent_x = (to[0] - from[0]).signum();
    let tangent_z = (to[2] - from[2]).signum();
    Some(CavePortalProbe {
        center,
        axis_u: [-tangent_z, 0, tangent_x],
        axis_v: [0, 1, 0],
    })
}

pub fn cinder_vault_portal_state(
    mut is_open_voxel: impl FnMut(i32, i32, i32) -> bool,
) -> PortalState {
    let mut state = PortalState::default();
    for portal_index in 0..CINDER_VAULT_PORTAL_COUNT {
        let open = cinder_vault_portal_is_open(portal_index, &mut is_open_voxel).unwrap_or(false);
        let _ = state.set_open(portal_index, open);
    }
    state
}

pub fn cinder_vault_portal_is_open(
    portal_index: usize,
    mut is_open_voxel: impl FnMut(i32, i32, i32) -> bool,
) -> Option<bool> {
    cinder_vault_portal_probe(portal_index)?;
    let mut open_lanes = 0usize;
    for sample_index in 0..(CINDER_VAULT_PORTAL_PROBE_EDGE.pow(2) as usize) {
        let voxel = cinder_vault_portal_probe_voxel(portal_index, sample_index)?;
        open_lanes += usize::from(is_open_voxel(voxel[0], voxel[1], voxel[2]));
    }
    Some(open_lanes >= CINDER_VAULT_PORTAL_OPEN_LANES)
}

pub fn cinder_vault_portals_affected_by_voxel(x: i32, y: i32, z: i32) -> u8 {
    let mut affected = 0u8;
    for portal_index in 0..CINDER_VAULT_PORTAL_COUNT {
        for sample_index in 0..(CINDER_VAULT_PORTAL_PROBE_EDGE.pow(2) as usize) {
            if cinder_vault_portal_probe_voxel(portal_index, sample_index) == Some([x, y, z]) {
                affected |= 1 << portal_index;
                break;
            }
        }
    }
    affected
}

/// Returns one canonical 10 cm voxel in an authored portal's fixed probe plane.
///
/// This is public so host shells and native tools can author, persist, and validate portal edits
/// without duplicating cave topology or converting metre-space coordinates back into voxels.
pub fn cinder_vault_portal_probe_voxel(
    portal_index: usize,
    sample_index: usize,
) -> Option<[i32; 3]> {
    let probe = cinder_vault_portal_probe(portal_index)?;
    if sample_index >= CINDER_VAULT_PORTAL_PROBE_EDGE.pow(2) as usize {
        return None;
    }
    let half = CINDER_VAULT_PORTAL_PROBE_EDGE / 2;
    let u = sample_index as i32 % CINDER_VAULT_PORTAL_PROBE_EDGE - half;
    let v = sample_index as i32 / CINDER_VAULT_PORTAL_PROBE_EDGE - half;
    Some([
        probe.center[0] + probe.axis_u[0] * u + probe.axis_v[0] * v,
        probe.center[1] + probe.axis_u[1] * u + probe.axis_v[1] * v,
        probe.center[2] + probe.axis_u[2] * u + probe.axis_v[2] * v,
    ])
}

fn node_distance_metres(left: [i32; 3], right: [i32; 3]) -> f32 {
    (squared_distance(left, right) as f32).sqrt() * VOXEL_SIZE_METRES
}

fn squared_distance(left: [i32; 3], right: [i32; 3]) -> i64 {
    (0..3)
        .map(|axis| {
            let delta = i64::from(left[axis]) - i64::from(right[axis]);
            delta * delta
        })
        .sum()
}

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
        if cinder_vault_crystal_at(x, y, z) {
            Some(Material::GlowCrystal)
        } else {
            Some(Material::Air)
        }
    } else if distance_squared <= 1.30 * 1.30 {
        Some(Material::Basalt)
    } else {
        None
    }
}

pub fn cinder_vault_crystal_at(x: i32, y: i32, z: i32) -> bool {
    CINDER_VAULT_CRYSTALS.iter().any(|formation| {
        let dy = y - formation.base[1];
        if !(0..i32::from(formation.height)).contains(&dy) {
            return false;
        }
        let last = (i32::from(formation.height) - 1).max(1);
        let radius = (last - dy) * i32::from(formation.base_radius) / last;
        (x - formation.base[0]).abs() + (z - formation.base[2]).abs() <= radius
    })
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
    fn visibility_cells_and_pristine_portals_match_the_authored_graph() {
        assert_eq!(CINDER_VAULT_TOPOLOGY_VERSION, 1);
        let graph = cinder_vault_visibility_graph().unwrap();
        assert_eq!(graph.cell_count(), 8);
        assert_eq!(graph.portal_count(), 7);
        for (index, node) in CINDER_VAULT_NODES.iter().enumerate() {
            assert_eq!(
                cinder_vault_visibility_cell(node.center[0], node.center[1], node.center[2]),
                VisibilityCellId::new(index as u8 + 1)
            );
        }

        let generator = crate::Generator::new(0x5eed_cafe);
        let state =
            cinder_vault_portal_state(|x, y, z| !generator.sample(x, y, z).occludes_ambient());
        for portal_index in 0..CINDER_VAULT_PORTAL_COUNT {
            assert!(
                state.is_open(portal_index),
                "portal {portal_index} was not pristine-open"
            );
            for sample_index in 0..(CINDER_VAULT_PORTAL_PROBE_EDGE.pow(2) as usize) {
                let voxel = cinder_vault_portal_probe_voxel(portal_index, sample_index).unwrap();
                assert_eq!(
                    cinder_vault_override(voxel[0], voxel[1], voxel[2]),
                    Some(Material::Air)
                );
                assert_eq!(
                    generator.sample(voxel[0], voxel[1], voxel[2]),
                    Material::Air
                );
                assert_ne!(
                    cinder_vault_portals_affected_by_voxel(voxel[0], voxel[1], voxel[2])
                        & (1 << portal_index),
                    0
                );
            }
        }
        for formation in CINDER_VAULT_CRYSTALS {
            let [x, y, z] = formation.base;
            let cell = cinder_vault_visibility_cell(x, y, z);
            assert!((1..=CINDER_VAULT_NODES.len()).contains(&cell.index()));
        }
    }

    #[test]
    fn exterior_to_chamber_visibility_requires_the_mouth_and_geodesic_range() {
        let graph = cinder_vault_visibility_graph().unwrap();
        let chamber = cinder_vault_visibility_cell(
            CINDER_VAULT.chamber[0],
            CINDER_VAULT.chamber[1],
            CINDER_VAULT.chamber[2],
        );
        let mut state = PortalState::all_open(graph.portal_count());
        let geodesic = graph
            .shortest_open_distance(CINDER_VAULT_EXTERIOR_CELL, chamber, state)
            .expect("pristine chamber must connect to the exterior");
        assert!((geodesic - 22.239_94).abs() < 0.000_1);
        let final_cell = VisibilityCellId::new(7);
        let final_distance = graph
            .shortest_open_distance(CINDER_VAULT_EXTERIOR_CELL, final_cell, state)
            .unwrap();
        assert!((final_distance - 26.014_86).abs() < 0.000_1);
        assert!(state.set_open(0, false));
        assert_eq!(
            graph.shortest_open_distance(CINDER_VAULT_EXTERIOR_CELL, chamber, state),
            None
        );
    }

    #[test]
    fn portal_probes_close_from_edits_and_unrelated_voxels_touch_nothing() {
        let generator = crate::Generator::new(0x5eed_cafe);
        let pristine =
            cinder_vault_portal_state(|x, y, z| !generator.sample(x, y, z).occludes_ambient());
        let portal_index = 4usize;
        let portal_mask = 1u8 << portal_index;
        let closed = cinder_vault_portal_state(|x, y, z| {
            cinder_vault_portals_affected_by_voxel(x, y, z) & portal_mask == 0
                && !generator.sample(x, y, z).occludes_ambient()
        });
        assert!(pristine.is_open(portal_index));
        assert!(!closed.is_open(portal_index));
        for other in 0..CINDER_VAULT_PORTAL_COUNT {
            if other != portal_index {
                assert_eq!(closed.is_open(other), pristine.is_open(other));
            }
        }
        assert_eq!(cinder_vault_portals_affected_by_voxel(0, 0, 0), 0);
        let probe = cinder_vault_portal_probe(portal_index).unwrap();
        assert_ne!(
            cinder_vault_portals_affected_by_voxel(
                probe.center[0],
                probe.center[1],
                probe.center[2]
            ) & portal_mask,
            0
        );

        let mut edits = crate::EditMap::default();
        for sample_index in 0..(CINDER_VAULT_PORTAL_PROBE_EDGE.pow(2) as usize) {
            let voxel = cinder_vault_portal_probe_voxel(0, sample_index).unwrap();
            edits.set(
                generator,
                crate::VoxelCoord::new(voxel[0], voxel[1], voxel[2]),
                Material::Basalt,
            );
        }
        let sealed = cinder_vault_portal_state(|x, y, z| {
            !edits
                .sample(generator, crate::VoxelCoord::new(x, y, z))
                .occludes_ambient()
        });
        assert!(!sealed.is_open(0));
        assert_eq!(
            cinder_vault_visibility_graph()
                .unwrap()
                .shortest_open_distance(
                    CINDER_VAULT_EXTERIOR_CELL,
                    VisibilityCellId::new(6),
                    sealed,
                ),
            None
        );
        for sample_index in 0..(CINDER_VAULT_PORTAL_PROBE_EDGE.pow(2) as usize) {
            let voxel = cinder_vault_portal_probe_voxel(0, sample_index).unwrap();
            let coord = crate::VoxelCoord::new(voxel[0], voxel[1], voxel[2]);
            edits.set(
                generator,
                coord,
                generator.sample(voxel[0], voxel[1], voxel[2]),
            );
        }
        assert!(edits.is_empty());
        let restored = cinder_vault_portal_state(|x, y, z| {
            !edits
                .sample(generator, crate::VoxelCoord::new(x, y, z))
                .occludes_ambient()
        });
        assert_eq!(restored, pristine);
    }

    #[test]
    fn portal_stream_interest_is_local_bounded_and_follows_connectivity() {
        let all_open = PortalState::all_open(CINDER_VAULT_PORTAL_COUNT);
        assert!(cinder_vault_stream_interest([0, 0, 0], all_open).is_empty());

        let exterior_above_chamber = [
            CINDER_VAULT.chamber[0],
            CINDER_VAULT_BOUNDS[1][1] + 1,
            CINDER_VAULT.chamber[2],
        ];
        assert_eq!(
            cinder_vault_visibility_cell(
                exterior_above_chamber[0],
                exterior_above_chamber[1],
                exterior_above_chamber[2]
            ),
            CINDER_VAULT_EXTERIOR_CELL
        );
        let pristine = cinder_vault_stream_interest(exterior_above_chamber, all_open);
        assert!(!pristine.is_empty());
        assert!(!pristine.overflowed());
        assert!(pristine.len() <= CINDER_VAULT_STREAM_INTEREST_CAPACITY);
        let unique: std::collections::BTreeSet<_> = pristine
            .as_slice()
            .iter()
            .map(|coord| (coord.x, coord.y, coord.z))
            .collect();
        assert_eq!(unique.len(), pristine.len());
        let minimum = voxel_chunk(CINDER_VAULT_BOUNDS[0]);
        let maximum = voxel_chunk(CINDER_VAULT_BOUNDS[1]);
        assert!(pristine.as_slice().iter().all(|coord| {
            (minimum.x - 1..=maximum.x + 1).contains(&coord.x)
                && (minimum.y - 1..=maximum.y + 1).contains(&coord.y)
                && (minimum.z - 1..=maximum.z + 1).contains(&coord.z)
        }));
        for node in CINDER_VAULT_NODES {
            assert!(pristine.as_slice().contains(&voxel_chunk(node.center)));
        }

        let mut mouth_closed = all_open;
        assert!(mouth_closed.set_open(0, false));
        assert!(cinder_vault_stream_interest(exterior_above_chamber, mouth_closed).is_empty());
        assert!(!cinder_vault_stream_interest(CINDER_VAULT.chamber, mouth_closed).is_empty());

        let mut middle_closed = all_open;
        assert!(middle_closed.set_open(4, false));
        let upstream = cinder_vault_stream_interest(exterior_above_chamber, middle_closed);
        assert!(
            upstream
                .as_slice()
                .contains(&voxel_chunk(CINDER_VAULT_NODES[0].center))
        );
        assert!(
            !upstream
                .as_slice()
                .contains(&voxel_chunk(CINDER_VAULT_NODES[6].center))
        );
    }

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

    #[test]
    fn authored_crystals_are_sparse_ordinary_voxels_inside_the_void() {
        for formation in CINDER_VAULT_CRYSTALS {
            let [x, y, z] = formation.base;
            assert!(sample_cinder_vault(x, y, z).is_some());
            assert_eq!(cinder_vault_override(x, y, z), Some(Material::GlowCrystal));
            assert_eq!(
                cinder_vault_override(x, y - 1, z),
                Some(Material::Basalt),
                "formation at {:?} must grow from the sealed shell",
                formation.base
            );
            assert!(Material::GlowCrystal.is_collidable());
            assert!(Material::GlowCrystal.occludes_ambient());
            let tip_y = y + i32::from(formation.height) - 1;
            let tip_voxels = (-2..=2)
                .flat_map(|dz| (-2..=2).map(move |dx| (x + dx, z + dz)))
                .filter(|(tip_x, tip_z)| {
                    cinder_vault_override(*tip_x, tip_y, *tip_z) == Some(Material::GlowCrystal)
                })
                .count();
            assert_eq!(tip_voxels, 1, "formation must taper to one 10 cm voxel");
            assert_eq!(
                cinder_vault_override(x, y + i32::from(formation.height), z),
                Some(Material::Air)
            );
        }
    }
}
