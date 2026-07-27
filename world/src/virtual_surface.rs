use crate::{Material, VoxelCoord};
use std::collections::{BTreeMap, BTreeSet};

const BOUNDARY_HASH_DOMAIN: &[u8] = b"voxels-boundary-certificate-v1\0";

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum FaceAxis {
    X = 0,
    Y = 1,
    Z = 2,
}

impl FaceAxis {
    pub const ALL: [Self; 3] = [Self::X, Self::Y, Self::Z];
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum BoundarySide {
    NegativeX = 0,
    PositiveX = 1,
    NegativeY = 2,
    PositiveY = 3,
    NegativeZ = 4,
    PositiveZ = 5,
}

impl BoundarySide {
    pub const ALL: [Self; 6] = [
        Self::NegativeX,
        Self::PositiveX,
        Self::NegativeY,
        Self::PositiveY,
        Self::NegativeZ,
        Self::PositiveZ,
    ];

    pub const fn axis(self) -> FaceAxis {
        match self {
            Self::NegativeX | Self::PositiveX => FaceAxis::X,
            Self::NegativeY | Self::PositiveY => FaceAxis::Y,
            Self::NegativeZ | Self::PositiveZ => FaceAxis::Z,
        }
    }

    pub const fn positive(self) -> bool {
        matches!(self, Self::PositiveX | Self::PositiveY | Self::PositiveZ)
    }

    pub const fn opposite(self) -> Self {
        match self {
            Self::NegativeX => Self::PositiveX,
            Self::PositiveX => Self::NegativeX,
            Self::NegativeY => Self::PositiveY,
            Self::PositiveY => Self::NegativeY,
            Self::NegativeZ => Self::PositiveZ,
            Self::PositiveZ => Self::NegativeZ,
        }
    }

    const fn index(self) -> usize {
        self as usize
    }
}

/// A validated half-open box in canonical 10 cm integer voxel coordinates.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct VoxelBounds {
    pub min: VoxelCoord,
    pub max: VoxelCoord,
}

impl VoxelBounds {
    pub const fn new(min: VoxelCoord, max: VoxelCoord) -> Option<Self> {
        if min.x >= max.x || min.y >= max.y || min.z >= max.z {
            return None;
        }
        Some(Self { min, max })
    }

    pub fn contains(self, coord: VoxelCoord) -> bool {
        coord.x >= self.min.x
            && coord.y >= self.min.y
            && coord.z >= self.min.z
            && coord.x < self.max.x
            && coord.y < self.max.y
            && coord.z < self.max.z
    }

    pub fn volume(self) -> Option<u64> {
        let x = u64::try_from(i64::from(self.max.x) - i64::from(self.min.x)).ok()?;
        let y = u64::try_from(i64::from(self.max.y) - i64::from(self.min.y)).ok()?;
        let z = u64::try_from(i64::from(self.max.z) - i64::from(self.min.z)).ok()?;
        x.checked_mul(y)?.checked_mul(z)
    }

    const fn plane(self, side: BoundarySide) -> i32 {
        match side {
            BoundarySide::NegativeX => self.min.x,
            BoundarySide::PositiveX => self.max.x,
            BoundarySide::NegativeY => self.min.y,
            BoundarySide::PositiveY => self.max.y,
            BoundarySide::NegativeZ => self.min.z,
            BoundarySide::PositiveZ => self.max.z,
        }
    }

    fn contains_bounds(self, child: Self) -> bool {
        self.contains(child.min)
            && child.max.x <= self.max.x
            && child.max.y <= self.max.y
            && child.max.z <= self.max.z
    }
}

/// Stable key for one exact exposed unit face. No chunk, page, or LOD coordinate participates.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CanonicalFaceKey {
    pub axis: FaceAxis,
    pub plane: i32,
    pub u: i32,
    pub v: i32,
    pub solid_side: VoxelCoord,
    pub material_id: u16,
}

impl CanonicalFaceKey {
    fn exposed(solid: VoxelCoord, material: Material, axis: FaceAxis, positive: bool) -> Self {
        let component = match axis {
            FaceAxis::X => solid.x,
            FaceAxis::Y => solid.y,
            FaceAxis::Z => solid.z,
        };
        let (u, v) = match axis {
            FaceAxis::X => (solid.y, solid.z),
            FaceAxis::Y => (solid.x, solid.z),
            FaceAxis::Z => (solid.x, solid.y),
        };
        Self {
            axis,
            plane: component + i32::from(positive),
            u,
            v,
            solid_side: solid,
            material_id: material.id(),
        }
    }
}

/// Enumerates exact occupancy faces under one half-open rule. Sampling outside `bounds` is
/// intentional: chunk boundaries cannot invent faces when the neighboring canonical voxel exists.
pub fn canonical_exposed_faces(
    bounds: VoxelBounds,
    mut material_at: impl FnMut(VoxelCoord) -> Material,
) -> Vec<CanonicalFaceKey> {
    let mut faces = Vec::new();
    for z in bounds.min.z..bounds.max.z {
        for y in bounds.min.y..bounds.max.y {
            for x in bounds.min.x..bounds.max.x {
                let solid = VoxelCoord::new(x, y, z);
                let material = material_at(solid);
                if !material.is_renderable() {
                    continue;
                }
                for axis in FaceAxis::ALL {
                    for positive in [false, true] {
                        let mut neighbor = solid;
                        let offset = if positive { 1 } else { -1 };
                        match axis {
                            FaceAxis::X => neighbor.x = neighbor.x.saturating_add(offset),
                            FaceAxis::Y => neighbor.y = neighbor.y.saturating_add(offset),
                            FaceAxis::Z => neighbor.z = neighbor.z.saturating_add(offset),
                        }
                        if !material_at(neighbor).is_renderable() {
                            faces.push(CanonicalFaceKey::exposed(solid, material, axis, positive));
                        }
                    }
                }
            }
        }
    }
    faces.sort_unstable();
    faces
}

/// Canonical two-sided occupancy/material state for one 10 cm square on a page boundary.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CanonicalBoundarySample {
    pub axis: FaceAxis,
    pub plane: i32,
    pub u: i32,
    pub v: i32,
    pub negative_material_id: u16,
    pub positive_material_id: u16,
}

impl CanonicalBoundarySample {
    fn encode(self, hasher: &mut blake3::Hasher) {
        hasher.update(&[self.axis as u8]);
        hasher.update(&self.plane.to_le_bytes());
        hasher.update(&self.u.to_le_bytes());
        hasher.update(&self.v.to_le_bytes());
        hasher.update(&self.negative_material_id.to_le_bytes());
        hasher.update(&self.positive_material_id.to_le_bytes());
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundarySideCertificate {
    pub fingerprint: [u8; 32],
    pub samples: Vec<CanonicalBoundarySample>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundaryCertificate {
    pub bounds: VoxelBounds,
    pub sides: [BoundarySideCertificate; 6],
}

impl BoundaryCertificate {
    pub fn build(bounds: VoxelBounds, mut material_at: impl FnMut(VoxelCoord) -> Material) -> Self {
        let sides = std::array::from_fn(|index| {
            let side = BoundarySide::ALL[index];
            let samples = boundary_samples(bounds, side, &mut material_at);
            BoundarySideCertificate {
                fingerprint: boundary_fingerprint(&samples),
                samples,
            }
        });
        Self { bounds, sides }
    }

    pub fn side(&self, side: BoundarySide) -> &BoundarySideCertificate {
        &self.sides[side.index()]
    }

    pub fn matches_adjacent(&self, side: BoundarySide, adjacent: &Self) -> bool {
        self.bounds.plane(side) == adjacent.bounds.plane(side.opposite())
            && self.side(side) == adjacent.side(side.opposite())
    }

    /// Validates one atomic child-group replacement without trusting page order.
    ///
    /// Every parent outer sample must occur once, while each child-only internal sample must occur
    /// exactly twice with identical normalized negative/positive material state.
    pub fn validates_child_group(&self, children: &[Self]) -> bool {
        if children.is_empty()
            || children
                .iter()
                .any(|child| !self.bounds.contains_bounds(child.bounds))
            || children
                .iter()
                .filter_map(|child| child.bounds.volume())
                .sum::<u64>()
                != self.bounds.volume().unwrap_or(0)
        {
            return false;
        }
        for (index, left) in children.iter().enumerate() {
            for right in &children[index + 1..] {
                if bounds_overlap(left.bounds, right.bounds) {
                    return false;
                }
            }
        }

        let parent_samples = self
            .sides
            .iter()
            .flat_map(|side| side.samples.iter().copied())
            .collect::<BTreeSet<_>>();
        let mut outer = BTreeMap::<CanonicalBoundarySample, u8>::new();
        let mut internal = BTreeMap::<CanonicalBoundarySample, u8>::new();
        for child in children {
            for side in BoundarySide::ALL {
                let parent_plane = self.bounds.plane(side);
                let destination = if child.bounds.plane(side) == parent_plane {
                    &mut outer
                } else {
                    &mut internal
                };
                for sample in &child.side(side).samples {
                    let count = destination.entry(*sample).or_default();
                    *count = count.saturating_add(1);
                }
            }
        }
        parent_samples.len() == outer.len()
            && parent_samples.iter().eq(outer.keys())
            && outer.values().all(|count| *count == 1)
            && internal.values().all(|count| *count == 2)
    }
}

fn boundary_samples(
    bounds: VoxelBounds,
    side: BoundarySide,
    material_at: &mut impl FnMut(VoxelCoord) -> Material,
) -> Vec<CanonicalBoundarySample> {
    let axis = side.axis();
    let plane = bounds.plane(side);
    let (u_range, v_range) = match axis {
        FaceAxis::X => (bounds.min.y..bounds.max.y, bounds.min.z..bounds.max.z),
        FaceAxis::Y => (bounds.min.x..bounds.max.x, bounds.min.z..bounds.max.z),
        FaceAxis::Z => (bounds.min.x..bounds.max.x, bounds.min.y..bounds.max.y),
    };
    let mut samples = Vec::new();
    for v in v_range {
        for u in u_range.clone() {
            let negative = boundary_voxel(axis, plane, u, v, false);
            let positive = boundary_voxel(axis, plane, u, v, true);
            samples.push(CanonicalBoundarySample {
                axis,
                plane,
                u,
                v,
                negative_material_id: material_at(negative).id(),
                positive_material_id: material_at(positive).id(),
            });
        }
    }
    samples
}

const fn boundary_voxel(axis: FaceAxis, plane: i32, u: i32, v: i32, positive: bool) -> VoxelCoord {
    let normal = if positive {
        plane
    } else {
        plane.saturating_sub(1)
    };
    match axis {
        FaceAxis::X => VoxelCoord::new(normal, u, v),
        FaceAxis::Y => VoxelCoord::new(u, normal, v),
        FaceAxis::Z => VoxelCoord::new(u, v, normal),
    }
}

fn boundary_fingerprint(samples: &[CanonicalBoundarySample]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(BOUNDARY_HASH_DOMAIN);
    hasher.update(&(samples.len() as u64).to_le_bytes());
    for sample in samples {
        sample.encode(&mut hasher);
    }
    *hasher.finalize().as_bytes()
}

fn bounds_overlap(left: VoxelBounds, right: VoxelBounds) -> bool {
    left.min.x < right.max.x
        && right.min.x < left.max.x
        && left.min.y < right.max.y
        && right.min.y < left.max.y
        && left.min.z < right.max.z
        && right.min.z < left.max.z
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn deterministic_material(coord: VoxelCoord) -> Material {
        let hash = (coord.x as u32).wrapping_mul(0x9e37_79b1)
            ^ (coord.y as u32).rotate_left(11)
            ^ (coord.z as u32).wrapping_mul(0x85eb_ca6b);
        if hash % 7 < 3 {
            Material::Air
        } else {
            Material::ALL[1 + hash as usize % (Material::ALL.len() - 1)]
        }
    }

    #[test]
    fn exact_faces_have_one_owner_across_negative_chunk_boundaries() {
        let bounds =
            VoxelBounds::new(VoxelCoord::new(-40, -35, -34), VoxelCoord::new(39, 36, 35)).unwrap();
        let faces = canonical_exposed_faces(bounds, deterministic_material);
        let mut geometric = BTreeSet::new();
        for face in &faces {
            assert!(geometric.insert((face.axis, face.plane, face.u, face.v)));
            assert!(deterministic_material(face.solid_side).is_renderable());
            assert!(matches!(face.solid_side.chunk().x, -2..=1));
        }

        let mut expected = 0usize;
        for z in bounds.min.z..bounds.max.z {
            for y in bounds.min.y..bounds.max.y {
                for x in bounds.min.x..bounds.max.x {
                    let coord = VoxelCoord::new(x, y, z);
                    if !deterministic_material(coord).is_renderable() {
                        continue;
                    }
                    for offset in [
                        [-1, 0, 0],
                        [1, 0, 0],
                        [0, -1, 0],
                        [0, 1, 0],
                        [0, 0, -1],
                        [0, 0, 1],
                    ] {
                        let neighbor = VoxelCoord::new(x + offset[0], y + offset[1], z + offset[2]);
                        expected += usize::from(!deterministic_material(neighbor).is_renderable());
                    }
                }
            }
        }
        assert_eq!(faces.len(), expected);
    }

    #[test]
    fn randomized_child_groups_match_outer_boundaries_and_cancel_internal_boundaries() {
        let mut seed = 0x4d59_5df4_d0f3_3173u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..10_000 {
            let size = 2_i32 << ((next() % 4) as u32);
            let min = VoxelCoord::new(
                (next() as i32).rem_euclid(513) - 256,
                (next() as i32).rem_euclid(129) - 64,
                (next() as i32).rem_euclid(513) - 256,
            );
            let max = VoxelCoord::new(min.x + size, min.y + size, min.z + size);
            let parent_bounds = VoxelBounds::new(min, max).unwrap();
            let middle = VoxelCoord::new(min.x + size / 2, min.y + size / 2, min.z + size / 2);
            let parent = BoundaryCertificate::build(parent_bounds, deterministic_material);
            let children = [false, true]
                .into_iter()
                .flat_map(|high_z| {
                    [false, true].into_iter().flat_map(move |high_y| {
                        [false, true].into_iter().map(move |high_x| {
                            let child_min = VoxelCoord::new(
                                if high_x { middle.x } else { min.x },
                                if high_y { middle.y } else { min.y },
                                if high_z { middle.z } else { min.z },
                            );
                            let child_max = VoxelCoord::new(
                                if high_x { max.x } else { middle.x },
                                if high_y { max.y } else { middle.y },
                                if high_z { max.z } else { middle.z },
                            );
                            BoundaryCertificate::build(
                                VoxelBounds::new(child_min, child_max).unwrap(),
                                deterministic_material,
                            )
                        })
                    })
                })
                .collect::<Vec<_>>();
            assert!(parent.validates_child_group(&children));

            let mut corrupt = children.clone();
            corrupt[0].sides[BoundarySide::PositiveX.index()].samples[0].positive_material_id ^= 1;
            assert!(!parent.validates_child_group(&corrupt));
        }
    }

    #[test]
    fn adjacent_certificates_are_identical_from_either_page() {
        let left_bounds =
            VoxelBounds::new(VoxelCoord::new(-32, -8, -16), VoxelCoord::new(0, 8, 16)).unwrap();
        let right_bounds =
            VoxelBounds::new(VoxelCoord::new(0, -8, -16), VoxelCoord::new(32, 8, 16)).unwrap();
        let left = BoundaryCertificate::build(left_bounds, deterministic_material);
        let right = BoundaryCertificate::build(right_bounds, deterministic_material);
        assert!(left.matches_adjacent(BoundarySide::PositiveX, &right));
        assert_eq!(
            left.side(BoundarySide::PositiveX).fingerprint,
            right.side(BoundarySide::NegativeX).fingerprint
        );
    }

    #[test]
    fn canonical_face_keys_retain_material_and_solid_side() {
        let mut materials = BTreeMap::new();
        materials.insert(VoxelCoord::new(-1, 0, 0), Material::Stone);
        materials.insert(VoxelCoord::new(0, 0, 0), Material::Dirt);
        let bounds = VoxelBounds::new(VoxelCoord::new(-1, 0, 0), VoxelCoord::new(1, 1, 1)).unwrap();
        let faces = canonical_exposed_faces(bounds, |coord| {
            *materials.get(&coord).unwrap_or(&Material::Air)
        });
        assert_eq!(faces.len(), 10);
        assert!(
            faces
                .iter()
                .all(|face| face.plane != 0 || face.axis != FaceAxis::X)
        );
        assert_eq!(
            faces
                .iter()
                .map(|face| face.material_id)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([Material::Dirt.id(), Material::Stone.id()])
        );
    }
}
