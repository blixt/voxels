use crate::{Material, VoxelCoord};

pub const FEATURE_CELL_VOXELS: i32 = 96;
pub const FEATURE_MAX_RADIUS_VOXELS: i32 = 10;

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
}

impl SkylineFeatureKind {
    pub const ALL: [Self; 6] = [
        Self::Broadleaf,
        Self::MoorTor,
        Self::AlpineNeedle,
        Self::BadlandsHoodoo,
        Self::DuneArch,
        Self::BasaltColumns,
    ];
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
}

impl SkylineFeature {
    pub const fn bounds(self) -> [[i32; 3]; 2] {
        let crown = if matches!(self.kind, SkylineFeatureKind::Broadleaf) {
            3
        } else {
            1
        };
        [
            [
                self.anchor[0] - FEATURE_MAX_RADIUS_VOXELS,
                self.anchor[1] + 1,
                self.anchor[2] - FEATURE_MAX_RADIUS_VOXELS,
            ],
            [
                self.anchor[0] + FEATURE_MAX_RADIUS_VOXELS + 1,
                self.trunk_top + crown,
                self.anchor[2] + FEATURE_MAX_RADIUS_VOXELS + 1,
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
                let radius = if dy * 3 < height {
                    6
                } else if dy * 3 < height * 2 {
                    5
                } else {
                    3
                };
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
                let radius = (7 - dy * 6 / height.max(1)).max(1);
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
                let radius = if dy >= height - 4 {
                    6
                } else if dy >= height - 9 {
                    4
                } else if dy <= 4 {
                    3
                } else {
                    2
                };
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
                let columns = [
                    ([0, 0], height, 2),
                    ([-5, -3], height * 3 / 4, 2),
                    ([5, -2], height * 5 / 8, 1),
                    ([2, 5], height * 7 / 8, 2),
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

    fn broadleaf_material(self, dx: i32, dy: i32, dz: i32) -> Option<Material> {
        let height = self.trunk_top - self.anchor[1];
        if (1..=height).contains(&dy) && dx.abs() <= 1 && dz.abs() <= 1 {
            return Some(Material::Wood);
        }
        let crown_dy = dy - (height - 3);
        let horizontal_radius = 9 - crown_dy.abs() / 2;
        let distance_squared = dx * dx + dz * dz + (crown_dy * crown_dy) / 2;
        ((-7..=5).contains(&crown_dy)
            && dx.abs() <= horizontal_radius
            && dz.abs() <= horizontal_radius
            && distance_squared <= 76 + i32::from(self.variant) * 2)
            .then_some(Material::Leaves)
    }
}
