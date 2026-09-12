//! Fixed-capacity hash indexing for the direct voxel traversal atlas.
//!
//! The material atlas is linear and keeps stable slots.  Traversal needs a spatial lookup, so this
//! module builds a separate power-of-two open-addressing table whose entries match
//! `shaders/brick_traversal.wgsl`.  Rebuilding a table is explicit and bounded by its configured
//! capacity; callers can measure that cost against incremental tombstone updates before choosing a
//! publication policy.

use crate::brick_residency::{BrickAddress, BrickCoord};

pub const INVALID_SLOT: u32 = u32::MAX;
pub const RESIDENT_EMPTY: u32 = 1;
pub const RESIDENT_NON_EMPTY: u32 = 2;

/// A 32-byte storage entry consumed by the traversal shader.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuBrickHashEntry {
    /// xyz is the signed brick coordinate; w is reserved and remains zero.
    pub coord: [i32; 4],
    /// x is the atlas slot, y/z are the complete generation, and w is the residency flag.
    pub metadata: [u32; 4],
}

impl GpuBrickHashEntry {
    pub const EMPTY: Self = Self {
        coord: [0; 4],
        metadata: [INVALID_SLOT, 0, 0, 0],
    };

    pub const fn is_empty(self) -> bool {
        self.metadata[0] == INVALID_SLOT
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BrickHashItem {
    pub coord: BrickCoord,
    pub address: BrickAddress,
    pub non_empty: bool,
}

/// Deterministic fixed-capacity table. A table never grows implicitly.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BrickHashTable {
    entries: Vec<GpuBrickHashEntry>,
    mask: u32,
}

impl BrickHashTable {
    pub fn new(capacity: u32) -> Result<Self, String> {
        if capacity < 2 || !capacity.is_power_of_two() {
            return Err("brick hash capacity must be a power of two >= 2".to_owned());
        }
        Ok(Self {
            entries: vec![GpuBrickHashEntry::EMPTY; capacity as usize],
            mask: capacity - 1,
        })
    }

    pub const fn capacity(&self) -> u32 {
        self.entries.len() as u32
    }

    pub const fn mask(&self) -> u32 {
        self.mask
    }

    pub fn entries(&self) -> &[GpuBrickHashEntry] {
        &self.entries
    }

    pub fn clear(&mut self) {
        self.entries.fill(GpuBrickHashEntry::EMPTY);
    }

    /// Rebuilds the table in input order. Duplicate coordinates are rejected so a caller cannot
    /// accidentally publish two GPU addresses for one brick.
    pub fn rebuild<I>(&mut self, items: I) -> Result<(), String>
    where
        I: IntoIterator<Item = BrickHashItem>,
    {
        let mut candidate = Self {
            entries: vec![GpuBrickHashEntry::EMPTY; self.entries.len()],
            mask: self.mask,
        };
        for item in items {
            candidate.insert(item)?;
        }
        self.entries = candidate.entries;
        Ok(())
    }

    fn insert(&mut self, item: BrickHashItem) -> Result<(), String> {
        let start = hash(item.coord) & self.mask;
        for probe in 0..self.entries.len() as u32 {
            let index = ((start + probe) & self.mask) as usize;
            let entry = self.entries[index];
            if entry.is_empty() {
                self.entries[index] = GpuBrickHashEntry {
                    coord: [item.coord.x, item.coord.y, item.coord.z, 0],
                    metadata: [
                        item.address.slot,
                        item.address.generation as u32,
                        (item.address.generation >> 32) as u32,
                        if item.non_empty {
                            RESIDENT_NON_EMPTY
                        } else {
                            RESIDENT_EMPTY
                        },
                    ],
                };
                return Ok(());
            }
            if entry.coord[..3] == [item.coord.x, item.coord.y, item.coord.z] {
                return Err(format!(
                    "duplicate direct-traversal brick coordinate ({}, {}, {})",
                    item.coord.x, item.coord.y, item.coord.z
                ));
            }
        }
        Err("brick hash table is full".to_owned())
    }

    pub fn lookup(&self, coord: BrickCoord) -> Option<(u32, bool)> {
        let start = hash(coord) & self.mask;
        for probe in 0..self.entries.len() as u32 {
            let entry = self.entries[((start + probe) & self.mask) as usize];
            if entry.is_empty() {
                return None;
            }
            if entry.coord[..3] == [coord.x, coord.y, coord.z] {
                return Some((entry.metadata[0], entry.metadata[3] == RESIDENT_NON_EMPTY));
            }
        }
        None
    }
}

const fn hash(coord: BrickCoord) -> u32 {
    let mut hash = 2_166_136_261u32;
    hash = (hash ^ coord.x as u32).wrapping_mul(16_777_619);
    hash = (hash ^ coord.y as u32).wrapping_mul(16_777_619);
    (hash ^ coord.z as u32).wrapping_mul(16_777_619)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(x: i32, slot: u32, non_empty: bool) -> BrickHashItem {
        BrickHashItem {
            coord: BrickCoord::new(x, -2, 7),
            address: BrickAddress {
                slot,
                generation: 0x1122_3344_5566_7788,
            },
            non_empty,
        }
    }

    #[test]
    fn table_requires_power_of_two_capacity() {
        assert_eq!(std::mem::size_of::<GpuBrickHashEntry>(), 32);
        assert!(BrickHashTable::new(0).is_err());
        assert!(BrickHashTable::new(3).is_err());
        assert_eq!(BrickHashTable::new(8).unwrap().capacity(), 8);
    }

    #[test]
    fn lookup_preserves_negative_coordinates_and_empty_state() {
        let mut table = BrickHashTable::new(8).unwrap();
        table.rebuild([item(-9, 4, false), item(12, 5, true)]).unwrap();
        assert_eq!(table.lookup(BrickCoord::new(-9, -2, 7)), Some((4, false)));
        assert_eq!(table.lookup(BrickCoord::new(12, -2, 7)), Some((5, true)));
        assert_eq!(table.lookup(BrickCoord::new(99, 0, 0)), None);
    }

    #[test]
    fn duplicate_and_full_publications_fail_without_partial_state() {
        let mut table = BrickHashTable::new(2).unwrap();
        let duplicate = [item(1, 1, true), item(1, 2, true)];
        assert!(table.rebuild(duplicate).is_err());
        assert!(table.entries().iter().all(|entry| entry.is_empty()));
        assert!(table.rebuild([item(1, 1, true), item(2, 2, true), item(3, 3, true)]).is_err());
    }
}
