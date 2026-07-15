//! Pure geometric ownership for nested block-surface LOD rings.

use std::collections::HashSet;
use voxels_world::{CHUNK_EDGE, SurfaceBounds, SurfaceLodLevel, SurfacePatchEdge, SurfacePatchId};

/// Half extents in canonical 10 cm voxels. Every boundary is a multiple of the patch span on both
/// sides, so whole patches can change owner without overlap, holes, or fragment clipping.
pub const LOD_BOUNDARY_HALF_EXTENTS: [i32; 6] = [96, 256, 512, 1_024, 2_048, 4_096];
// Snap only as coarsely as both adjacent representations require. In particular, the near handoff
// moves in one 3.2 m chunk rather than a 9.6 m feature cell, cutting its worst visible replacement
// strip by two thirds while preserving whole-chunk and whole-patch ownership.
const LOD_BOUNDARY_SNAP: [i32; 6] = [32, 32, 64, 128, 256, 512];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LodOwner {
    Canonical,
    Surface(SurfaceLodLevel),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GeometricLodFocus {
    boundary_centres: [[i32; 2]; 6],
    surface_level_count: u8,
}

impl GeometricLodFocus {
    pub fn snapped(voxel_x: i32, voxel_z: i32) -> Self {
        Self::snapped_for_levels(voxel_x, voxel_z, SurfaceLodLevel::ALL.len())
    }

    pub fn snapped_for_levels(voxel_x: i32, voxel_z: i32, surface_level_count: usize) -> Self {
        assert!(
            (1..=SurfaceLodLevel::ALL.len()).contains(&surface_level_count),
            "geometric LOD focus must own at least one known surface level"
        );
        Self {
            boundary_centres: std::array::from_fn(|index| {
                let snap = LOD_BOUNDARY_SNAP[index];
                [snap_nearest(voxel_x, snap), snap_nearest(voxel_z, snap)]
            }),
            surface_level_count: surface_level_count as u8,
        }
    }

    /// Advances only the innermost boundaries whose replacement coverage is ready.
    ///
    /// Keeping the remaining centres unchanged prevents a slow horizon tile from freezing the
    /// nearby cut and also prevents several independently snapped rings from jumping together.
    pub fn advanced_for_levels(
        mut self,
        voxel_x: i32,
        voxel_z: i32,
        ready_level_count: usize,
        surface_level_count: usize,
    ) -> Self {
        assert!(ready_level_count > 0 && ready_level_count <= surface_level_count);
        let target = Self::snapped_for_levels(voxel_x, voxel_z, surface_level_count);
        self.boundary_centres[..ready_level_count]
            .copy_from_slice(&target.boundary_centres[..ready_level_count]);
        self.surface_level_count = surface_level_count as u8;
        self
    }

    pub const fn boundary_centres(self) -> [[i32; 2]; 6] {
        self.boundary_centres
    }

    pub fn owner_at(self, voxel_x: i32, voxel_z: i32) -> LodOwner {
        let voxel_x = i64::from(voxel_x);
        let voxel_z = i64::from(voxel_z);
        let surface_level_count = usize::from(self.surface_level_count);
        for (index, half_extent) in LOD_BOUNDARY_HALF_EXTENTS
            .into_iter()
            .take(surface_level_count)
            .enumerate()
        {
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
        LodOwner::Surface(SurfaceLodLevel::ALL[surface_level_count - 1])
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

/// Selects a resident surface hierarchy without ever exposing a partially loaded sibling group.
/// A parent remains visible until all four immediate children are resident, then those children
/// independently repeat the same rule toward the fixed geometric owner requested by `focus`.
pub fn surface_patch_is_selected(
    focus: GeometricLodFocus,
    resident: &HashSet<SurfacePatchId>,
    canonical_ready_columns: &HashSet<(i32, i32)>,
    patch: SurfacePatchId,
) -> bool {
    if !resident.contains(&patch) {
        return false;
    }
    let Some(center) = patch.voxel_center_xz() else {
        return false;
    };
    let owner = focus.owner_at(center[0], center[1]);
    if owner == LodOwner::Canonical {
        let column = (
            center[0].div_euclid(CHUNK_EDGE as i32),
            center[1].div_euclid(CHUNK_EDGE as i32),
        );
        if canonical_ready_columns.contains(&column) {
            return false;
        }
        if refinement_blocked_by_ancestor(resident, patch) {
            return false;
        }
        return patch.level == SurfaceLodLevel::Stride2
            || patch.children().is_none_or(|children| {
                !children.into_iter().all(|child| resident.contains(&child))
            });
    }
    let LodOwner::Surface(target_level) = owner else {
        unreachable!();
    };
    let ordering = patch.level.index().cmp(&target_level.index());
    if ordering == std::cmp::Ordering::Less || refinement_blocked_by_ancestor(resident, patch) {
        return false;
    }
    match ordering {
        std::cmp::Ordering::Less => unreachable!(),
        std::cmp::Ordering::Equal => true,
        std::cmp::Ordering::Greater => patch
            .children()
            .is_none_or(|children| !children.into_iter().all(|child| resident.contains(&child))),
    }
}

pub fn surface_patch_skirt_is_selected(
    focus: GeometricLodFocus,
    resident: &HashSet<SurfacePatchId>,
    canonical_ready_columns: &HashSet<(i32, i32)>,
    patch: SurfacePatchId,
    edge: SurfacePatchEdge,
) -> bool {
    if !surface_patch_is_selected(focus, resident, canonical_ready_columns, patch) {
        return false;
    }
    let delta = match edge {
        SurfacePatchEdge::NegativeX => [-1, 0],
        SurfacePatchEdge::PositiveX => [1, 0],
        SurfacePatchEdge::NegativeZ => [0, -1],
        SurfacePatchEdge::PositiveZ => [0, 1],
    };
    let Some(neighbor) = patch.neighbor(delta[0], delta[1]) else {
        return false;
    };
    if surface_patch_is_selected(focus, resident, canonical_ready_columns, neighbor) {
        return false;
    }
    if resident.contains(&neighbor) {
        return true;
    }
    let Some(center) = neighbor.voxel_center_xz() else {
        return false;
    };
    focus.owner_at(center[0], center[1]) != LodOwner::Surface(patch.level)
}

/// Cached result of hierarchy selection. Building it touches each resident patch once; draw-list
/// construction then performs only constant-time membership checks instead of repeating ancestor
/// walks for the main shell and four skirt slices of every patch.
#[derive(Debug, Default)]
pub struct SurfacePatchSelection {
    patches: HashSet<SurfacePatchId>,
    skirts: HashSet<(SurfacePatchId, u8)>,
}

impl SurfacePatchSelection {
    pub fn rebuild(
        &mut self,
        focus: GeometricLodFocus,
        resident: &HashSet<SurfacePatchId>,
        canonical_ready_columns: &HashSet<(i32, i32)>,
    ) {
        self.patches.clear();
        self.skirts.clear();
        self.patches
            .extend(resident.iter().copied().filter(|patch| {
                surface_patch_is_selected(focus, resident, canonical_ready_columns, *patch)
            }));
        for patch in self.patches.iter().copied() {
            for edge in SurfacePatchEdge::ALL {
                let delta = match edge {
                    SurfacePatchEdge::NegativeX => [-1, 0],
                    SurfacePatchEdge::PositiveX => [1, 0],
                    SurfacePatchEdge::NegativeZ => [0, -1],
                    SurfacePatchEdge::PositiveZ => [0, 1],
                };
                let Some(neighbor) = patch.neighbor(delta[0], delta[1]) else {
                    continue;
                };
                if self.patches.contains(&neighbor) {
                    continue;
                }
                let residency_boundary = resident.contains(&neighbor);
                let resolution_boundary = neighbor.voxel_center_xz().is_some_and(|center| {
                    focus.owner_at(center[0], center[1]) != LodOwner::Surface(patch.level)
                });
                if residency_boundary || resolution_boundary {
                    self.skirts.insert((patch, edge.index() as u8));
                }
            }
        }
    }

    pub fn owns(&self, patch: SurfacePatchId, skirt_edge: Option<SurfacePatchEdge>) -> bool {
        skirt_edge.map_or_else(
            || self.patches.contains(&patch),
            |edge| self.skirts.contains(&(patch, edge.index() as u8)),
        )
    }

    pub fn patch_count(&self) -> usize {
        self.patches.len()
    }
}

fn refinement_blocked_by_ancestor(
    resident: &HashSet<SurfacePatchId>,
    patch: SurfacePatchId,
) -> bool {
    let mut descendant = patch;
    while let Some(parent) = descendant.parent() {
        if resident.contains(&parent)
            && parent.children().is_some_and(|siblings| {
                siblings
                    .into_iter()
                    .any(|sibling| !resident.contains(&sibling))
            })
        {
            return true;
        }
        descendant = parent;
    }
    false
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

    fn resident(patches: impl IntoIterator<Item = SurfacePatchId>) -> HashSet<SurfacePatchId> {
        patches.into_iter().collect()
    }

    #[test]
    fn incomplete_child_groups_keep_the_parent_without_overlap() {
        let focus = GeometricLodFocus::snapped(0, 0);
        let parent = SurfacePatchId::new(SurfaceLodLevel::Stride4, 4, 0);
        let children = parent.children().expect("finer children");
        let parent_center = parent.voxel_center_xz().unwrap();
        assert_eq!(
            focus.owner_at(parent_center[0], parent_center[1]),
            LodOwner::Surface(SurfaceLodLevel::Stride2)
        );

        let partial = resident([parent, children[0], children[1], children[2]]);
        assert!(surface_patch_is_selected(
            focus,
            &partial,
            &HashSet::new(),
            parent
        ));
        assert!(children.into_iter().all(|child| !surface_patch_is_selected(
            focus,
            &partial,
            &HashSet::new(),
            child
        )));

        let complete = resident(std::iter::once(parent).chain(children));
        assert!(!surface_patch_is_selected(
            focus,
            &complete,
            &HashSet::new(),
            parent
        ));
        assert!(children.into_iter().all(|child| surface_patch_is_selected(
            focus,
            &complete,
            &HashSet::new(),
            child
        )));
        let mut selection = SurfacePatchSelection::default();
        selection.rebuild(focus, &complete, &HashSet::new());
        assert_eq!(selection.patch_count(), 4);
        assert!(!selection.owns(parent, None));
        assert!(
            children
                .into_iter()
                .all(|child| selection.owns(child, None))
        );
    }

    #[test]
    fn selected_patch_skirts_only_resolution_or_residency_boundaries() {
        let focus = GeometricLodFocus::snapped(0, 0);
        let left = SurfacePatchId::new(SurfaceLodLevel::Stride2, 7, 0);
        let right = left.neighbor(1, 0).unwrap();
        let both = resident([left, right]);
        assert!(!surface_patch_skirt_is_selected(
            focus,
            &both,
            &HashSet::new(),
            left,
            SurfacePatchEdge::PositiveX
        ));
        assert!(!surface_patch_skirt_is_selected(
            focus,
            &resident([left]),
            &HashSet::new(),
            left,
            SurfacePatchEdge::PositiveX
        ));

        let boundary = SurfacePatchId::new(SurfaceLodLevel::Stride2, 6, 0);
        assert!(surface_patch_skirt_is_selected(
            focus,
            &resident([boundary]),
            &HashSet::new(),
            boundary,
            SurfacePatchEdge::NegativeX
        ));
    }

    #[test]
    fn negative_child_groups_refine_with_euclidean_parent_identity() {
        let focus = GeometricLodFocus::snapped(0, 0);
        let parent = SurfacePatchId::new(SurfaceLodLevel::Stride4, -5, 0);
        let children = parent.children().unwrap();
        let complete = resident(std::iter::once(parent).chain(children));
        assert!(!surface_patch_is_selected(
            focus,
            &complete,
            &HashSet::new(),
            parent
        ));
        assert!(
            children
                .into_iter()
                .all(|child| child.parent() == Some(parent))
        );
    }

    #[test]
    fn stride_two_surface_covers_canonical_columns_until_the_column_is_ready() {
        let focus = GeometricLodFocus::snapped(0, 0);
        let patch = SurfacePatchId::new(SurfaceLodLevel::Stride2, 0, 0);
        let resident = resident([patch]);
        assert_eq!(focus.owner_at(8, 8), LodOwner::Canonical);
        assert!(surface_patch_is_selected(
            focus,
            &resident,
            &HashSet::new(),
            patch
        ));
        assert!(!surface_patch_is_selected(
            focus,
            &resident,
            &HashSet::from([(0, 0)]),
            patch
        ));
    }

    #[test]
    fn boundary_centres_are_grid_snapped_and_nested() {
        let focus = GeometricLodFocus::snapped(117, -73);
        assert_eq!(
            focus.boundary_centres(),
            [
                [128, -64],
                [128, -64],
                [128, -64],
                [128, -128],
                [0, 0],
                [0, 0],
            ]
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
        for index in 1..SurfaceLodLevel::ALL.len() {
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
            ([2_560, -64], LodOwner::Surface(SurfaceLodLevel::Stride32)),
            ([5_120, -64], LodOwner::Surface(SurfaceLodLevel::Stride64)),
        ];
        for (point, expected) in probes {
            assert_eq!(focus.owner_at(point[0], point[1]), expected);
        }
        for z in (-7_000..=7_000).step_by(31) {
            for x in (-7_000..=7_000).step_by(31) {
                assert!(matches!(
                    focus.owner_at(x, z),
                    LodOwner::Canonical | LodOwner::Surface(_)
                ));
            }
        }
    }

    #[test]
    fn interactive_focus_keeps_stride16_as_its_unbounded_outer_owner() {
        let focus = GeometricLodFocus::snapped_for_levels(117, -73, 4);
        assert_eq!(
            focus.owner_at(100_000, 100_000),
            LodOwner::Surface(SurfaceLodLevel::Stride16)
        );
        assert!(!focus.owns_surface_bounds(
            SurfaceLodLevel::Stride32,
            SurfaceBounds {
                min: [2_048, -64, 0],
                max: [2_304, 128, 256],
            }
        ));
    }

    #[test]
    fn advancing_ready_prefix_does_not_move_slow_horizon_boundaries() {
        let initial = GeometricLodFocus::snapped(0, 0);
        let advanced = initial.advanced_for_levels(1_100, -900, 4, 6);
        let expected = GeometricLodFocus::snapped(1_100, -900);
        assert_eq!(
            &advanced.boundary_centres()[..4],
            &expected.boundary_centres()[..4]
        );
        assert_eq!(
            &advanced.boundary_centres()[4..],
            &initial.boundary_centres()[4..]
        );
        assert_eq!(
            advanced.owner_at(100_000, 100_000),
            LodOwner::Surface(SurfaceLodLevel::Stride64)
        );
    }

    #[test]
    fn aligned_patch_grids_never_straddle_their_ownership_boundaries() {
        let focus = GeometricLodFocus::snapped(117, -73);
        for level in SurfaceLodLevel::ALL {
            let span = level.stride_voxels() * voxels_world::SURFACE_PATCH_EDGE_CELLS;
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
