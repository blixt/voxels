use crate::{Material, RouteLandmarkId, VoxelCoord};

pub const FEATURE_CELL_VOXELS: i32 = 96;
pub const FEATURE_MAX_RADIUS_VOXELS: i32 = 18;

/// Stable procedural identity. The placement cell reconstructs the feature from generator identity.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SkylineFeatureId {
    pub cell_x: i32,
    pub cell_z: i32,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SkylineFeatureKind {
    #[default]
    Broadleaf = 0,
    MoorTor = 1,
    AlpineNeedle = 2,
    BadlandsHoodoo = 3,
    DuneArch = 4,
    BasaltColumns = 5,
    PilgrimCairn = 6,
    RouteWaystone = 7,
    RuinedArch = 8,
    ElderCanopy = 9,
    TorCircle = 10,
    NeedleGate = 11,
    BuriedRibs = 12,
    BuriedColonnade = 13,
    BasaltCrown = 14,
}

impl SkylineFeatureKind {
    pub const REGIONAL: [Self; 6] = [
        Self::Broadleaf,
        Self::MoorTor,
        Self::AlpineNeedle,
        Self::BadlandsHoodoo,
        Self::DuneArch,
        Self::BasaltColumns,
    ];
    pub const REGIONAL_HEROES: [Self; 6] = [
        Self::ElderCanopy,
        Self::TorCircle,
        Self::NeedleGate,
        Self::BuriedRibs,
        Self::BuriedColonnade,
        Self::BasaltCrown,
    ];
    pub const ALL: [Self; 15] = [
        Self::Broadleaf,
        Self::MoorTor,
        Self::AlpineNeedle,
        Self::BadlandsHoodoo,
        Self::DuneArch,
        Self::BasaltColumns,
        Self::PilgrimCairn,
        Self::RouteWaystone,
        Self::RuinedArch,
        Self::ElderCanopy,
        Self::TorCircle,
        Self::NeedleGate,
        Self::BuriedRibs,
        Self::BuriedColonnade,
        Self::BasaltCrown,
    ];

    pub const fn horizontal_radius_voxels(self) -> i32 {
        match self {
            Self::ElderCanopy => 16,
            Self::TorCircle | Self::BuriedRibs | Self::BuriedColonnade | Self::BasaltCrown => 14,
            Self::NeedleGate => 13,
            _ => 10,
        }
    }

    pub const fn top_allowance_voxels(self) -> i32 {
        match self {
            Self::Broadleaf => 3,
            Self::ElderCanopy => 6,
            _ => 1,
        }
    }

    pub const fn is_semantic_hero(self) -> bool {
        matches!(
            self,
            Self::ElderCanopy
                | Self::TorCircle
                | Self::NeedleGate
                | Self::BuriedRibs
                | Self::BuriedColonnade
                | Self::BasaltCrown
        )
    }
}

/// One deterministic landmark shared by canonical generation and disposable surface-LOD proxies.
/// Bounds are half-open canonical 10 cm voxel coordinates.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SkylineFeature {
    pub id: SkylineFeatureId,
    pub kind: SkylineFeatureKind,
    pub anchor: [i32; 3],
    pub trunk_top: i32,
    pub orientation: u8,
    pub variant: u8,
    /// 0 = background, 1 = composition companion, 2 = composition hero.
    pub prominence: u8,
    pub route_landmark: Option<RouteLandmarkId>,
}

impl SkylineFeature {
    pub const fn bounds(self) -> [[i32; 3]; 2] {
        let radius = self.kind.horizontal_radius_voxels();
        [
            [
                self.anchor[0] - radius,
                self.anchor[1] + 1,
                self.anchor[2] - radius,
            ],
            [
                self.anchor[0] + radius + 1,
                self.trunk_top + self.kind.top_allowance_voxels(),
                self.anchor[2] + radius + 1,
            ],
        ]
    }

    pub fn material_at(self, coord: VoxelCoord) -> Option<Material> {
        let dx = coord.x - self.anchor[0];
        let dy = coord.y - self.anchor[1];
        let dz = coord.z - self.anchor[2];
        let height = self.trunk_top - self.anchor[1];
        match self.kind {
            SkylineFeatureKind::Broadleaf => self.broadleaf_material(dx, dy, dz),
            SkylineFeatureKind::MoorTor => {
                if !(1..=height).contains(&dy) {
                    return None;
                }
                let radius = (if dy * 3 < height {
                    6
                } else if dy * 3 < height * 2 {
                    5
                } else {
                    3
                } + self.radius_bonus())
                .min(FEATURE_MAX_RADIUS_VOXELS);
                let notch =
                    (dx * 13 + dz * 7 + dy * 3 + i32::from(self.variant)).rem_euclid(17) == 0;
                (dx.abs().max(dz.abs()) <= radius
                    && dx.abs() + dz.abs() < radius * 2
                    && !(notch && dx.abs().max(dz.abs()) == radius))
                    .then_some(Material::Limestone)
            }
            SkylineFeatureKind::AlpineNeedle => {
                if !(1..=height).contains(&dy) {
                    return None;
                }
                let radius = ((7 - dy * 6 / height.max(1)).max(1) + self.radius_bonus())
                    .min(FEATURE_MAX_RADIUS_VOXELS);
                (dx.abs().max(dz.abs()) <= radius && dx.abs() + dz.abs() <= radius * 2).then_some(
                    if dy >= height - 5 {
                        Material::Snow
                    } else {
                        Material::Stone
                    },
                )
            }
            SkylineFeatureKind::BadlandsHoodoo => {
                if !(1..=height).contains(&dy) {
                    return None;
                }
                let radius = (if dy >= height - 4 {
                    6
                } else if dy >= height - 9 {
                    4
                } else if dy <= 4 {
                    3
                } else {
                    2
                } + self.radius_bonus())
                .min(FEATURE_MAX_RADIUS_VOXELS);
                (dx.abs().max(dz.abs()) <= radius && dx * dx + dz * dz <= radius * radius + radius)
                    .then_some(if dy >= height - 5 {
                        Material::RedSand
                    } else {
                        Material::Clay
                    })
            }
            SkylineFeatureKind::DuneArch => {
                if !(1..=height).contains(&dy) {
                    return None;
                }
                let [along, across] = self.oriented_offset(dx, dz);
                let pillar = (6..=9).contains(&along.abs()) && across.abs() <= 2;
                let lintel = dy >= height - 4 && along.abs() <= 9 && across.abs() <= 2;
                (pillar || lintel).then_some(Material::Limestone)
            }
            SkylineFeatureKind::BasaltColumns => {
                let bonus = self.radius_bonus();
                let columns = [
                    ([0, 0], height, 2 + bonus),
                    ([-5, -3], height * 3 / 4, 2 + bonus),
                    ([5, -2], height * 5 / 8, 1 + bonus),
                    ([2, 5], height * 7 / 8, 2 + bonus),
                ];
                columns
                    .into_iter()
                    .find_map(|(offset, column_height, radius)| {
                        let [offset_x, offset_z] = self.oriented_offset(offset[0], offset[1]);
                        ((1..=column_height).contains(&dy)
                            && (dx - offset_x).abs() <= radius
                            && (dz - offset_z).abs() <= radius)
                            .then_some(Material::Basalt)
                    })
            }
            SkylineFeatureKind::PilgrimCairn => {
                if !(1..=height).contains(&dy) {
                    return None;
                }
                let radius = if dy <= 3 {
                    4
                } else if dy <= 6 {
                    3
                } else {
                    2
                };
                (dx.abs().max(dz.abs()) <= radius && dx * dx + dz * dz <= radius * radius + radius)
                    .then_some(if dy & 1 == 0 {
                        Material::Stone
                    } else {
                        Material::Limestone
                    })
            }
            SkylineFeatureKind::RouteWaystone => {
                if !(1..=height).contains(&dy) {
                    return None;
                }
                let plinth = dy <= 3 && dx.abs() <= 3 && dz.abs() <= 3;
                let shaft = dy > 2 && dx.abs() <= 1 && dz.abs() <= 1;
                let cap = dy >= height - 2 && dx.abs() <= 2 && dz.abs() <= 2;
                (plinth || shaft || cap).then_some(Material::Limestone)
            }
            SkylineFeatureKind::RuinedArch => {
                if !(1..=height).contains(&dy) {
                    return None;
                }
                let [along, across] = self.oriented_offset(dx, dz);
                let pillars = (6..=8).contains(&along.abs()) && across.abs() <= 2;
                let lintel = dy >= height - 3 && along.abs() <= 8 && across.abs() <= 2;
                let broken = along > 2
                    && dy >= height - 1 - i32::from(self.variant & 1)
                    && across.abs() <= 2;
                ((pillars || lintel) && !broken).then_some(if dy >= height - 3 {
                    Material::Stone
                } else {
                    Material::Limestone
                })
            }
            SkylineFeatureKind::ElderCanopy => self.elder_canopy_material(dx, dy, dz),
            SkylineFeatureKind::TorCircle => self.tor_circle_material(dx, dy, dz),
            SkylineFeatureKind::NeedleGate => self.needle_gate_material(dx, dy, dz),
            SkylineFeatureKind::BuriedRibs => self.buried_ribs_material(dx, dy, dz),
            SkylineFeatureKind::BuriedColonnade => self.buried_colonnade_material(dx, dy, dz),
            SkylineFeatureKind::BasaltCrown => self.basalt_crown_material(dx, dy, dz),
        }
    }

    pub const fn contains_xz(self, x: i32, z: i32) -> bool {
        let bounds = self.bounds();
        x >= bounds[0][0] && x < bounds[1][0] && z >= bounds[0][2] && z < bounds[1][2]
    }

    pub const fn oriented_offset(self, x: i32, z: i32) -> [i32; 2] {
        match self.orientation & 3 {
            0 => [x, z],
            1 => [-z, x],
            2 => [-x, -z],
            _ => [z, -x],
        }
    }

    fn radius_bonus(self) -> i32 {
        self.prominence.min(2) as i32
    }

    fn broadleaf_material(self, dx: i32, dy: i32, dz: i32) -> Option<Material> {
        let height = self.trunk_top - self.anchor[1];
        if (1..=height).contains(&dy) && dx.abs() <= 1 && dz.abs() <= 1 {
            return Some(Material::Wood);
        }
        let crown_dy = dy - (height - 3);
        let horizontal_radius =
            (9 + self.radius_bonus() - crown_dy.abs() / 2).min(FEATURE_MAX_RADIUS_VOXELS);
        let distance_squared = dx * dx + dz * dz + (crown_dy * crown_dy) / 2;
        ((-7..=5).contains(&crown_dy)
            && dx.abs() <= horizontal_radius
            && dz.abs() <= horizontal_radius
            && distance_squared <= 76 + i32::from(self.variant) * 2 + self.radius_bonus() * 12)
            .then_some(Material::Leaves)
    }

    fn elder_canopy_material(self, dx: i32, dy: i32, dz: i32) -> Option<Material> {
        let height = self.trunk_top - self.anchor[1];
        if !(1..=height + 5).contains(&dy) {
            return None;
        }
        let root = dy <= 4 && dx.abs().min(dz.abs()) <= 1 && dx.abs().max(dz.abs()) <= 8;
        let trunk_radius = if dy <= height / 3 { 3 } else { 2 };
        let trunk = dy <= height && dx.abs() <= trunk_radius && dz.abs() <= trunk_radius;
        let branch_y = height - 12 + i32::from(self.variant & 3);
        let branches = (branch_y..=height - 2).contains(&dy)
            && ((dx.abs() <= 11 && dz.abs() <= 1) || (dz.abs() <= 11 && dx.abs() <= 1));
        if root || trunk || branches {
            return Some(Material::Wood);
        }
        let crown_y = height - 3;
        let lobes = [
            ([0, 0], 15),
            ([-8, -5], 9),
            ([8, -4], 9),
            ([-5, 8], 9),
            ([6, 8], 9),
        ];
        lobes
            .into_iter()
            .find_map(|([offset_x, offset_z], radius)| {
                let local_x = dx - offset_x;
                let local_z = dz - offset_z;
                let local_y = dy - crown_y;
                (local_x * local_x + local_z * local_z + local_y * local_y * 2 <= radius * radius)
                    .then_some(Material::Leaves)
            })
    }

    fn tor_circle_material(self, dx: i32, dy: i32, dz: i32) -> Option<Material> {
        let height = self.trunk_top - self.anchor[1];
        let columns = [
            ([0, 0], height, 3),
            ([-11, 0], height * 3 / 4, 3),
            ([11, 0], height * 4 / 5, 3),
            ([0, -11], height * 2 / 3, 3),
            ([0, 11], height * 7 / 8, 3),
        ];
        columns
            .into_iter()
            .find_map(|(offset, column_height, radius)| {
                let [offset_x, offset_z] = self.oriented_offset(offset[0], offset[1]);
                let edge = (dx - offset_x).abs().max((dz - offset_z).abs());
                let chipped = (dx * 11 + dz * 5 + dy + i32::from(self.variant)).rem_euclid(19) == 0
                    && edge == radius;
                ((1..=column_height).contains(&dy) && edge <= radius && !chipped)
                    .then_some(Material::Limestone)
            })
    }

    fn needle_gate_material(self, dx: i32, dy: i32, dz: i32) -> Option<Material> {
        let height = self.trunk_top - self.anchor[1];
        if !(1..=height).contains(&dy) {
            return None;
        }
        let [along, across] = self.oriented_offset(dx, dz);
        let pillar_radius = (5 - dy * 4 / height.max(1)).max(2);
        let pillars = (along - 10).abs() <= pillar_radius || (along + 10).abs() <= pillar_radius;
        let pillars = pillars && across.abs() <= pillar_radius;
        let lintel = dy >= height - 5 && along.abs() <= 12 && across.abs() <= 3;
        (pillars || lintel).then_some(if dy >= height - 6 {
            Material::Snow
        } else {
            Material::Stone
        })
    }

    fn buried_ribs_material(self, dx: i32, dy: i32, dz: i32) -> Option<Material> {
        let height = self.trunk_top - self.anchor[1];
        if !(1..=height).contains(&dy) {
            return None;
        }
        let [along, across] = self.oriented_offset(dx, dz);
        let mut occupied = false;
        for rib in [-11, -4, 4, 11] {
            if (along - rib).abs() > 1 {
                continue;
            }
            let vertical = across.abs() >= 8 && across.abs() <= 11 && dy <= height - 6;
            let arch_radius = 10;
            let arch_y = dy - (height - 9);
            let arch = arch_y >= 0
                && across * across + arch_y * arch_y >= (arch_radius - 2) * (arch_radius - 2)
                && across * across + arch_y * arch_y <= arch_radius * arch_radius + 8;
            occupied |= vertical || arch;
        }
        let crown = along.abs() <= 2 && across.abs() <= 9 && dy >= height - 2;
        (occupied || crown).then_some(if dy >= height - 8 {
            Material::Limestone
        } else {
            Material::Stone
        })
    }

    fn buried_colonnade_material(self, dx: i32, dy: i32, dz: i32) -> Option<Material> {
        let height = self.trunk_top - self.anchor[1];
        if !(1..=height).contains(&dy) {
            return None;
        }
        let [along, across] = self.oriented_offset(dx, dz);
        let column = [-11, -4, 4, 11].into_iter().any(|offset| {
            (along - offset).abs() <= 2
                && across.abs() <= 2
                && dy <= height - (offset.abs() / 5 + i32::from(self.variant & 1))
        });
        let plinth = dy <= 3 && along.abs() <= 14 && across.abs() <= 4;
        let beam = dy >= height - 4
            && along.abs() <= 14
            && across.abs() <= 3
            && !(along > 6 && dy >= height - 1);
        (column || plinth || beam).then_some(if dy <= 3 {
            Material::Stone
        } else {
            Material::Limestone
        })
    }

    fn basalt_crown_material(self, dx: i32, dy: i32, dz: i32) -> Option<Material> {
        let height = self.trunk_top - self.anchor[1];
        let columns = [
            ([0, 0], height, 3),
            ([-10, -5], height * 4 / 5, 3),
            ([10, -5], height * 3 / 4, 3),
            ([-9, 7], height * 2 / 3, 3),
            ([9, 7], height * 7 / 8, 3),
            ([0, 12], height * 3 / 5, 2),
        ];
        columns
            .into_iter()
            .find_map(|(offset, column_height, radius)| {
                let [offset_x, offset_z] = self.oriented_offset(offset[0], offset[1]);
                ((1..=column_height).contains(&dy)
                    && (dx - offset_x).abs() <= radius
                    && (dz - offset_z).abs() <= radius)
                    .then_some(Material::Basalt)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_hero_voxels_are_nonempty_and_inside_declared_bounds() {
        for (index, kind) in SkylineFeatureKind::REGIONAL_HEROES.into_iter().enumerate() {
            let feature = SkylineFeature {
                id: SkylineFeatureId::default(),
                kind,
                anchor: [0, 20, 0],
                trunk_top: 90,
                orientation: index as u8,
                variant: (index & 3) as u8,
                prominence: 2,
                route_landmark: None,
            };
            let [min, max] = feature.bounds();
            let mut occupied = 0usize;
            for y in min[1]..max[1] {
                for z in min[2]..max[2] {
                    for x in min[0]..max[0] {
                        occupied +=
                            usize::from(feature.material_at(VoxelCoord::new(x, y, z)).is_some());
                    }
                }
            }
            assert!(
                occupied > 100,
                "{kind:?} was not a substantial canonical form"
            );
            assert!(
                feature
                    .material_at(VoxelCoord::new(
                        feature.anchor[0],
                        feature.trunk_top,
                        feature.anchor[2]
                    ))
                    .is_some(),
                "{kind:?} did not occupy its stable inspection probe"
            );
            for probe in [
                VoxelCoord::new(min[0] - 1, feature.trunk_top, feature.anchor[2]),
                VoxelCoord::new(max[0], feature.trunk_top, feature.anchor[2]),
                VoxelCoord::new(feature.anchor[0], min[1] - 1, feature.anchor[2]),
                VoxelCoord::new(feature.anchor[0], max[1], feature.anchor[2]),
                VoxelCoord::new(feature.anchor[0], feature.trunk_top, min[2] - 1),
                VoxelCoord::new(feature.anchor[0], feature.trunk_top, max[2]),
            ] {
                assert_eq!(
                    feature.material_at(probe),
                    None,
                    "{kind:?} escaped its bounds"
                );
            }
        }
    }
}
