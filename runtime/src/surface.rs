use std::collections::BTreeMap;

use crate::revision_satisfies;
use voxels_world::{SurfaceTileCoord, VOXEL_SIZE_METRES};

/// Orders unsent surface work for a view without dropping any requested coverage.
///
/// Tiles that intersect the forward camera half-space come before tiles wholly behind it. Within
/// either group, coarse coverage remains first and distance breaks ties. This gives a heading
/// change useful geometry sooner while preserving the clipmap's quality and eventual coverage.
pub fn prioritize_surface_tiles(
    tiles: &mut [SurfaceTileCoord],
    camera_xz: [f32; 2],
    camera_yaw: f32,
    focus: [SurfaceTileCoord; 4],
) {
    tiles.sort_by_key(|coord| surface_priority_key(*coord, camera_xz, camera_yaw, focus));
}

fn surface_priority_key(
    coord: SurfaceTileCoord,
    camera_xz: [f32; 2],
    camera_yaw: f32,
    focus: [SurfaceTileCoord; 4],
) -> (bool, u8, i64, i32, i32) {
    let span_metres = coord.voxel_span() as f32 * VOXEL_SIZE_METRES;
    let [origin_x, origin_z] = coord.voxel_origin();
    let center_x = origin_x as f32 * VOXEL_SIZE_METRES + span_metres * 0.5;
    let center_z = origin_z as f32 * VOXEL_SIZE_METRES + span_metres * 0.5;
    let (sin_yaw, cos_yaw) = camera_yaw.sin_cos();
    let forward_x = sin_yaw;
    let forward_z = -cos_yaw;
    let center_dot = (center_x - camera_xz[0]) * forward_x + (center_z - camera_xz[1]) * forward_z;
    let projected_half_extent = span_metres * 0.5 * (forward_x.abs() + forward_z.abs());
    let fully_behind_camera = center_dot + projected_half_extent <= 0.0;
    let level_index = coord.level.index() as usize;
    let dx = i64::from(coord.x) - i64::from(focus[level_index].x);
    let dz = i64::from(coord.z) - i64::from(focus[level_index].z);
    (
        fully_behind_camera,
        u8::MAX - coord.level.index(),
        dx * dx + dz * dz,
        coord.z,
        coord.x,
    )
}

/// Revision relationship between one disposable surface tile and the authoritative edit overlay.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SurfaceRevisionStatus {
    Missing { requested: u64 },
    Current { revision: u64 },
    Stale { resident: u64, requested: u64 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SurfaceFocusAction {
    Load { requested: u64 },
    Replace { resident: u64, requested: u64 },
    Current { revision: u64 },
}

/// Host-testable revision state for cached surface LOD tiles.
///
/// Geometry ownership and work queues remain host concerns. This state only proves whether a cached
/// mesh sampled every edit known to affect it, including edits received while it was retained just
/// outside active coverage.
#[derive(Clone, Debug)]
pub struct SurfaceRevisionCache {
    epoch: u64,
    requested: BTreeMap<SurfaceTileCoord, u64>,
    resident: BTreeMap<SurfaceTileCoord, u64>,
}

impl Default for SurfaceRevisionCache {
    fn default() -> Self {
        Self::new()
    }
}

impl SurfaceRevisionCache {
    pub const fn new() -> Self {
        Self {
            epoch: 1,
            requested: BTreeMap::new(),
            resident: BTreeMap::new(),
        }
    }

    pub fn begin_edit(&mut self) -> u64 {
        self.epoch = increment_nonzero(self.epoch);
        self.epoch
    }

    pub const fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn ensure_requested(&mut self, coord: SurfaceTileCoord) -> u64 {
        *self.requested.entry(coord).or_insert(self.epoch)
    }

    pub fn request(&mut self, coord: SurfaceTileCoord, revision: u64) {
        let requested = self.requested.entry(coord).or_insert(revision);
        if revision_satisfies(revision, *requested) {
            *requested = revision;
        }
    }

    pub fn requested_revision(&self, coord: SurfaceTileCoord) -> Option<u64> {
        self.requested.get(&coord).copied()
    }

    pub fn resident_revision(&self, coord: SurfaceTileCoord) -> Option<u64> {
        self.resident.get(&coord).copied()
    }

    pub fn status(&self, coord: SurfaceTileCoord) -> Option<SurfaceRevisionStatus> {
        let requested = self.requested_revision(coord)?;
        Some(match self.resident_revision(coord) {
            None => SurfaceRevisionStatus::Missing { requested },
            Some(resident) if !revision_satisfies(resident, requested) => {
                SurfaceRevisionStatus::Stale {
                    resident,
                    requested,
                }
            }
            Some(resident) => SurfaceRevisionStatus::Current { revision: resident },
        })
    }

    /// Classifies the work required when a tile enters active or pending focus.
    pub fn prepare_focus(&mut self, coord: SurfaceTileCoord) -> SurfaceFocusAction {
        let requested = self.ensure_requested(coord);
        match self.resident_revision(coord) {
            None => SurfaceFocusAction::Load { requested },
            Some(resident) if !revision_satisfies(resident, requested) => {
                SurfaceFocusAction::Replace {
                    resident,
                    requested,
                }
            }
            Some(revision) => SurfaceFocusAction::Current { revision },
        }
    }

    pub fn is_stale(&self, coord: SurfaceTileCoord) -> bool {
        matches!(
            self.status(coord),
            Some(SurfaceRevisionStatus::Stale { .. })
        )
    }

    pub fn is_current(&self, coord: SurfaceTileCoord) -> bool {
        matches!(
            self.status(coord),
            Some(SurfaceRevisionStatus::Current { .. })
        )
    }

    pub fn accepts(&self, coord: SurfaceTileCoord, revision: u64) -> bool {
        revision_satisfies(
            revision,
            self.requested_revision(coord).unwrap_or(self.epoch),
        )
    }

    /// Records a generated tile only if it still satisfies the newest request.
    pub fn commit(&mut self, coord: SurfaceTileCoord, revision: u64) -> bool {
        if !self.accepts(coord, revision) {
            return false;
        }
        // A host may finish work at a revision newer than the one that originally requested it.
        // Advance the request marker too, otherwise a later completion for the older request would
        // still be accepted and could replace this newer resident tile.
        self.request(coord, revision);
        self.resident.insert(coord, revision);
        true
    }

    pub fn evict(&mut self, coord: SurfaceTileCoord) {
        self.requested.remove(&coord);
        self.resident.remove(&coord);
    }

    /// Drops revision metadata for tiles that are neither retained nor desired by the host.
    pub fn retain(&mut self, mut keep: impl FnMut(SurfaceTileCoord) -> bool) {
        self.requested.retain(|coord, _| keep(*coord));
        self.resident.retain(|coord, _| keep(*coord));
    }
}

const fn increment_nonzero(value: u64) -> u64 {
    let next = value.wrapping_add(1);
    if next == 0 { 1 } else { next }
}

#[cfg(test)]
mod tests {
    use super::*;
    use voxels_world::SurfaceLodLevel;

    fn tile(x: i32) -> SurfaceTileCoord {
        SurfaceTileCoord::new(SurfaceLodLevel::Stride4, x, 7)
    }

    #[test]
    fn view_priority_puts_front_tiles_ahead_of_coarser_tiles_behind_camera() {
        let focus = std::array::from_fn(|index| {
            SurfaceTileCoord::containing(SurfaceLodLevel::ALL[index], 0, 0)
        });
        let front_coarse = SurfaceTileCoord::new(SurfaceLodLevel::Stride16, 0, -1);
        let front_fine = SurfaceTileCoord::new(SurfaceLodLevel::Stride2, 0, -1);
        let behind_coarse = SurfaceTileCoord::new(SurfaceLodLevel::Stride16, 0, 0);
        let mut tiles = [behind_coarse, front_fine, front_coarse];

        prioritize_surface_tiles(&mut tiles, [0.0, 0.0], 0.0, focus);
        assert_eq!(tiles, [front_coarse, front_fine, behind_coarse]);

        prioritize_surface_tiles(&mut tiles, [0.0, 0.0], std::f32::consts::PI, focus);
        assert_eq!(tiles[0], behind_coarse);
    }

    #[test]
    fn retained_tile_records_off_coverage_edits_and_becomes_stale() {
        let mut cache = SurfaceRevisionCache::new();
        let coord = tile(3);
        let pristine = cache.ensure_requested(coord);
        assert!(cache.commit(coord, pristine));

        let edited = cache.begin_edit();
        cache.request(coord, edited);

        assert_eq!(
            cache.status(coord),
            Some(SurfaceRevisionStatus::Stale {
                resident: pristine,
                requested: edited,
            })
        );
        assert_eq!(
            cache.prepare_focus(coord),
            SurfaceFocusAction::Replace {
                resident: pristine,
                requested: edited,
            }
        );
    }

    #[test]
    fn multiple_inactive_edits_coalesce_and_reject_old_completion() {
        let mut cache = SurfaceRevisionCache::new();
        let coord = tile(4);
        let pristine = cache.ensure_requested(coord);
        assert!(cache.commit(coord, pristine));
        let first = cache.begin_edit();
        cache.request(coord, first);
        let second = cache.begin_edit();
        cache.request(coord, second);

        assert!(!cache.commit(coord, first));
        assert_eq!(cache.resident_revision(coord), Some(pristine));
        assert!(cache.commit(coord, second));
        assert_eq!(
            cache.status(coord),
            Some(SurfaceRevisionStatus::Current { revision: second })
        );
    }

    #[test]
    fn newer_completion_advances_the_request_and_cannot_be_overwritten() {
        let mut cache = SurfaceRevisionCache::new();
        let coord = tile(11);
        let requested = cache.ensure_requested(coord);
        let newer = requested + 1;

        assert!(cache.commit(coord, newer));
        assert_eq!(cache.requested_revision(coord), Some(newer));
        assert!(!cache.commit(coord, requested));
        assert_eq!(cache.resident_revision(coord), Some(newer));
    }

    #[test]
    fn unrequested_tile_rejects_a_revision_older_than_the_current_epoch() {
        let mut cache = SurfaceRevisionCache::new();
        let coord = tile(9);
        let obsolete = cache.epoch();
        let current = cache.begin_edit();

        assert!(!cache.accepts(coord, obsolete));
        assert!(!cache.commit(coord, obsolete));
        assert_eq!(cache.status(coord), None);
        assert!(cache.commit(coord, current));
        assert_eq!(
            cache.status(coord),
            Some(SurfaceRevisionStatus::Current { revision: current })
        );
    }

    #[test]
    fn unrelated_tiles_stay_current_and_eviction_forgets_stale_work() {
        let mut cache = SurfaceRevisionCache::new();
        let affected = tile(5);
        let unrelated = tile(6);
        for coord in [affected, unrelated] {
            let revision = cache.ensure_requested(coord);
            assert!(cache.commit(coord, revision));
        }
        let edited = cache.begin_edit();
        cache.request(affected, edited);

        assert!(cache.is_stale(affected));
        assert!(!cache.is_stale(unrelated));
        cache.evict(affected);
        assert_eq!(cache.status(affected), None);
        assert_eq!(cache.resident_revision(affected), None);
    }

    #[test]
    fn abandoned_requests_are_pruned_without_forgetting_resident_tiles() {
        let mut cache = SurfaceRevisionCache::new();
        let resident = tile(7);
        let abandoned = tile(8);
        let revision = cache.ensure_requested(resident);
        assert!(cache.commit(resident, revision));
        cache.ensure_requested(abandoned);

        cache.retain(|coord| coord == resident);

        assert!(cache.is_current(resident));
        assert_eq!(cache.status(abandoned), None);
    }

    #[test]
    fn wrapped_revisions_reject_pre_wrap_completions() {
        let mut cache = SurfaceRevisionCache::new();
        let coord = tile(10);
        cache.epoch = u64::MAX - 1;
        let resident = cache.ensure_requested(coord);
        assert!(cache.commit(coord, resident));

        let final_pre_wrap = cache.begin_edit();
        cache.request(coord, final_pre_wrap);
        let wrapped = cache.begin_edit();
        cache.request(coord, wrapped);

        assert_eq!(wrapped, 1);
        assert_eq!(cache.requested_revision(coord), Some(wrapped));
        assert!(cache.is_stale(coord));
        assert!(!cache.commit(coord, final_pre_wrap));
        assert!(cache.commit(coord, wrapped));
        assert!(cache.is_current(coord));
    }

    #[test]
    fn newer_wrapped_completion_cannot_be_overwritten_by_pre_wrap_work() {
        let mut cache = SurfaceRevisionCache::new();
        let coord = tile(12);
        cache.epoch = u64::MAX;
        let requested = cache.ensure_requested(coord);

        assert!(cache.commit(coord, 1));
        assert_eq!(cache.requested_revision(coord), Some(1));
        assert!(!cache.commit(coord, requested));
        assert_eq!(cache.resident_revision(coord), Some(1));
    }
}
