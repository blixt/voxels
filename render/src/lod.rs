//! Pure geometric ownership for nested block-surface LOD rings.

use voxels_world::{CHUNK_EDGE, SurfaceBounds, SurfaceLodLevel, SurfacePatchEdge};

/// Half extents in canonical 10 cm voxels. Every boundary is a multiple of the patch span on both
/// sides, so whole patches can change owner without overlap, holes, or fragment clipping.
pub const LOD_BOUNDARY_HALF_EXTENTS: [i32; 4] = [96, 256, 512, 1_024];
// Snap only as coarsely as both adjacent representations require. In particular, the near handoff
// moves in one 3.2 m chunk rather than a 9.6 m feature cell, cutting its worst visible replacement
// strip by two thirds while preserving whole-chunk and whole-patch ownership.
const LOD_BOUNDARY_SNAP: [i32; 4] = [32, 32, 64, 128];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LodOwner {
    Canonical,
    Surface(SurfaceLodLevel),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GeometricLodFocus {
    boundary_centres: [[i32; 2]; 4],
}

impl GeometricLodFocus {
    pub fn snapped(voxel_x: i32, voxel_z: i32) -> Self {
        Self {
            boundary_centres: std::array::from_fn(|index| {
                let snap = LOD_BOUNDARY_SNAP[index];
                [snap_nearest(voxel_x, snap), snap_nearest(voxel_z, snap)]
            }),
        }
    }

    pub const fn boundary_centres(self) -> [[i32; 2]; 4] {
        self.boundary_centres
    }

    pub fn owner_at(self, voxel_x: i32, voxel_z: i32) -> LodOwner {
        let voxel_x = i64::from(voxel_x);
        let voxel_z = i64::from(voxel_z);
        for (index, half_extent) in LOD_BOUNDARY_HALF_EXTENTS.into_iter().enumerate() {
            let centre = self.boundary_centres[index];
            let centre_x = i64::from(centre[0]);
            let centre_z = i64::from(centre[1]);
            let half_extent = i64::from(half_extent);
            if voxel_x >= centre_x - half_extent
                && voxel_x < centre_x + half_extent
                && voxel_z >= centre_z - half_extent
                && voxel_z < centre_z + half_extent
            {
                return if index == 0 {
                    LodOwner::Canonical
                } else {
                    LodOwner::Surface(SurfaceLodLevel::ALL[index - 1])
                };
            }
        }
        LodOwner::Surface(SurfaceLodLevel::Stride16)
    }

    pub fn owns_surface_bounds(self, level: SurfaceLodLevel, bounds: SurfaceBounds) -> bool {
        let centre_x = bounds.min[0] + (bounds.max[0] - bounds.min[0]) / 2;
        let centre_z = bounds.min[2] + (bounds.max[2] - bounds.min[2]) / 2;
        self.owner_at(centre_x, centre_z) == LodOwner::Surface(level)
    }

    pub fn owns_surface_skirt(
        self,
        level: SurfaceLodLevel,
        bounds: SurfaceBounds,
        edge: SurfacePatchEdge,
    ) -> bool {
        if !self.owns_surface_bounds(level, bounds) {
            return false;
        }
        let centre_x = bounds.min[0] + (bounds.max[0] - bounds.min[0]) / 2;
        let centre_z = bounds.min[2] + (bounds.max[2] - bounds.min[2]) / 2;
        let neighbor = match edge {
            SurfacePatchEdge::NegativeX => [bounds.min[0].saturating_sub(1), centre_z],
            SurfacePatchEdge::PositiveX => [bounds.max[0], centre_z],
            SurfacePatchEdge::NegativeZ => [centre_x, bounds.min[2].saturating_sub(1)],
            SurfacePatchEdge::PositiveZ => [centre_x, bounds.max[2]],
        };
        self.owner_at(neighbor[0], neighbor[1]) != LodOwner::Surface(level)
    }

    pub fn owns_canonical_chunk(self, chunk_x: i32, chunk_z: i32) -> bool {
        let edge = CHUNK_EDGE as i32;
        self.owner_at(chunk_x * edge + edge / 2, chunk_z * edge + edge / 2) == LodOwner::Canonical
    }
}

fn snap_nearest(value: i32, step: i32) -> i32 {
    let value = i64::from(value);
    let step = i64::from(step);
    let lower = value.div_euclid(step) * step;
    let upper = lower + step;
    let minimum = i64::from(i32::MIN);
    let maximum = i64::from(i32::MAX);
    let lower_valid = (minimum..=maximum).contains(&lower);
    let upper_valid = (minimum..=maximum).contains(&upper);
    let snapped = if lower_valid && (!upper_valid || value - lower < upper - value) {
        lower
    } else if upper_valid {
        upper
    } else {
        value
    };
    snapped as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundary_centres_are_grid_snapped_and_nested() {
        let focus = GeometricLodFocus::snapped(117, -73);
        assert_eq!(
            focus.boundary_centres(),
            [[128, -64], [128, -64], [128, -64], [128, -128]]
        );
        let canonical = focus.boundary_centres()[0];
        for axis in canonical {
            assert_eq!(axis.rem_euclid(CHUNK_EDGE as i32), 0);
            assert_eq!(
                (axis - LOD_BOUNDARY_HALF_EXTENTS[0]).rem_euclid(CHUNK_EDGE as i32),
                0
            );
            assert_eq!(
                (axis + LOD_BOUNDARY_HALF_EXTENTS[0]).rem_euclid(CHUNK_EDGE as i32),
                0
            );
        }
        for index in 1..4 {
            let inner = focus.boundary_centres[index - 1];
            let outer = focus.boundary_centres[index];
            let centre_delta = (inner[0] - outer[0]).abs().max((inner[1] - outer[1]).abs());
            assert!(
                LOD_BOUNDARY_HALF_EXTENTS[index - 1] + centre_delta
                    < LOD_BOUNDARY_HALF_EXTENTS[index]
            );
        }
    }

    #[test]
    fn boundary_focus_stays_grid_aligned_without_overflow() {
        for boundary in [i32::MIN, i32::MAX] {
            let focus = GeometricLodFocus::snapped(boundary, boundary);
            for (index, centre) in focus.boundary_centres().into_iter().enumerate() {
                let snap = LOD_BOUNDARY_SNAP[index];
                assert_eq!(centre[0].rem_euclid(snap), 0);
                assert_eq!(centre[1].rem_euclid(snap), 0);
            }
            for probe in [i32::MIN, i32::MAX] {
                assert!(matches!(
                    focus.owner_at(probe, probe),
                    LodOwner::Canonical | LodOwner::Surface(_)
                ));
            }
        }
    }

    #[test]
    fn every_world_point_has_exactly_one_ordered_owner() {
        let focus = GeometricLodFocus::snapped(117, -73);
        let probes = [
            ([128, -64], LodOwner::Canonical),
            ([224, -64], LodOwner::Surface(SurfaceLodLevel::Stride2)),
            ([384, -64], LodOwner::Surface(SurfaceLodLevel::Stride4)),
            ([704, -64], LodOwner::Surface(SurfaceLodLevel::Stride8)),
            ([1_280, -64], LodOwner::Surface(SurfaceLodLevel::Stride16)),
        ];
        for (point, expected) in probes {
            assert_eq!(focus.owner_at(point[0], point[1]), expected);
        }
        for z in (-1_600..=1_600).step_by(7) {
            for x in (-1_600..=1_600).step_by(7) {
                assert!(matches!(
                    focus.owner_at(x, z),
                    LodOwner::Canonical | LodOwner::Surface(_)
                ));
            }
        }
    }

    #[test]
    fn aligned_patch_grids_never_straddle_their_ownership_boundaries() {
        let focus = GeometricLodFocus::snapped(117, -73);
        let spans = [16, 32, 64, 128];
        for (level, span) in SurfaceLodLevel::ALL.into_iter().zip(spans) {
            for patch_z in -48..=48 {
                for patch_x in -48..=48 {
                    let min_x = patch_x * span;
                    let min_z = patch_z * span;
                    let bounds = SurfaceBounds {
                        min: [min_x, -64, min_z],
                        max: [min_x + span, 128, min_z + span],
                    };
                    let corners = [
                        [min_x, min_z],
                        [min_x + span - 1, min_z],
                        [min_x, min_z + span - 1],
                        [min_x + span - 1, min_z + span - 1],
                    ];
                    if focus.owns_surface_bounds(level, bounds) {
                        assert!(corners.into_iter().all(|point| {
                            focus.owner_at(point[0], point[1]) == LodOwner::Surface(level)
                        }));
                    }
                }
            }
        }
    }

    #[test]
    fn canonical_chunk_grid_aligns_with_the_inner_cut() {
        let focus = GeometricLodFocus::snapped(117, -73);
        for chunk_z in -12..=12 {
            for chunk_x in -12..=12 {
                let owned = focus.owns_canonical_chunk(chunk_x, chunk_z);
                let edge = CHUNK_EDGE as i32;
                let corners = [
                    [chunk_x * edge, chunk_z * edge],
                    [chunk_x * edge + edge - 1, chunk_z * edge],
                    [chunk_x * edge, chunk_z * edge + edge - 1],
                    [chunk_x * edge + edge - 1, chunk_z * edge + edge - 1],
                ];
                if owned {
                    assert!(corners.into_iter().all(|point| {
                        focus.owner_at(point[0], point[1]) == LodOwner::Canonical
                    }));
                }
            }
        }
    }

    #[test]
    fn skirts_are_owned_only_on_resolution_boundaries() {
        let focus = GeometricLodFocus::snapped(0, 0);
        let interior = SurfaceBounds {
            min: [112, -64, 0],
            max: [128, 128, 16],
        };
        assert!(!focus.owns_surface_skirt(
            SurfaceLodLevel::Stride2,
            interior,
            SurfacePatchEdge::PositiveX
        ));

        let canonical_boundary = SurfaceBounds {
            min: [96, -64, 0],
            max: [112, 128, 16],
        };
        assert!(focus.owns_surface_skirt(
            SurfaceLodLevel::Stride2,
            canonical_boundary,
            SurfacePatchEdge::NegativeX
        ));
        assert!(!focus.owns_surface_skirt(
            SurfaceLodLevel::Stride2,
            canonical_boundary,
            SurfacePatchEdge::PositiveX
        ));

        let wrong_level = SurfaceBounds {
            min: [256, -64, 0],
            max: [288, 128, 32],
        };
        assert!(!focus.owns_surface_skirt(
            SurfaceLodLevel::Stride2,
            wrong_level,
            SurfacePatchEdge::NegativeX
        ));
    }

    #[test]
    fn skirts_do_not_wrap_across_the_world_boundary() {
        let focus = GeometricLodFocus::snapped(i32::MIN, i32::MIN);
        let bounds = SurfaceBounds {
            min: [i32::MIN, -64, i32::MIN],
            max: [i32::MIN + 16, 128, i32::MIN + 16],
        };

        assert!(!focus.owns_surface_skirt(
            SurfaceLodLevel::Stride2,
            bounds,
            SurfacePatchEdge::NegativeX,
        ));
        assert!(!focus.owns_surface_skirt(
            SurfaceLodLevel::Stride2,
            bounds,
            SurfacePatchEdge::NegativeZ,
        ));
    }
}
