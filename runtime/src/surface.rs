use std::collections::BTreeMap;

use voxels_world::SurfaceTileCoord;

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
        *requested = (*requested).max(revision);
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
            Some(resident) if resident < requested => SurfaceRevisionStatus::Stale {
                resident,
                requested,
            },
            Some(resident) => SurfaceRevisionStatus::Current { revision: resident },
        })
    }

    /// Classifies the work required when a tile enters active or pending focus.
    pub fn prepare_focus(&mut self, coord: SurfaceTileCoord) -> SurfaceFocusAction {
        let requested = self.ensure_requested(coord);
        match self.resident_revision(coord) {
            None => SurfaceFocusAction::Load { requested },
            Some(resident) if resident < requested => SurfaceFocusAction::Replace {
                resident,
                requested,
            },
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
        self.requested_revision(coord).unwrap_or(self.epoch) <= revision
    }

    /// Records a generated tile only if it still satisfies the newest request.
    pub fn commit(&mut self, coord: SurfaceTileCoord, revision: u64) -> bool {
        if !self.accepts(coord, revision) {
            return false;
        }
        let _ = self.ensure_requested(coord);
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
}
