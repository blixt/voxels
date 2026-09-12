//! Bounded CPU-side residency for directly traversed voxel bricks.
//!
//! The renderer keeps the authoritative voxel stream separate from GPU addresses.  A brick keeps
//! the same slot (and generation token) while it is resident, so an edit only uploads that brick
//! instead of rebuilding or copying a region-sized atlas.  This module contains no WGPU or shell
//! types and is consequently usable by host tests and by a future WebGPU upload path.

use std::collections::BTreeMap;
use std::sync::Arc;

/// The small brick used by the direct traversal path.  Larger 16³ pages can be layered above this
/// cache, but 8³ keeps edit uploads and cache invalidation bounded to 512 voxels.
pub const BRICK_EDGE: u32 = 8;
pub const BRICK_VOXEL_COUNT: usize = (BRICK_EDGE * BRICK_EDGE * BRICK_EDGE) as usize;
pub const BRICK_WORD_COUNT: usize = BRICK_VOXEL_COUNT / 4;
const MAX_TRACE_STEPS: usize = 1_048_576;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct BrickCoord {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl BrickCoord {
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }
}

/// A stable GPU address.  The generation changes whenever a slot is evicted and reused, allowing
/// a consumer to discard an in-flight upload that targets an old occupant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BrickAddress {
    pub slot: u32,
    pub generation: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BrickUpload {
    pub coord: BrickCoord,
    pub address: BrickAddress,
    pub revision: u64,
    pub payload: Arc<[u8]>,
}

impl BrickUpload {
    /// Byte range for the fixed-capacity WebGPU material arena. Four material IDs share each
    /// storage `u32`; the shader extracts byte lanes explicitly, independent of host endianness.
    pub const fn byte_offset(&self) -> u64 {
        self.address.slot as u64 * BRICK_VOXEL_COUNT as u64
    }

    pub fn material_words(&self) -> Option<[u32; BRICK_WORD_COUNT]> {
        if self.payload.len() != BRICK_VOXEL_COUNT {
            return None;
        }
        Some(std::array::from_fn(|word| {
            let first = word * 4;
            u32::from(self.payload[first])
                | (u32::from(self.payload[first + 1]) << 8)
                | (u32::from(self.payload[first + 2]) << 16)
                | (u32::from(self.payload[first + 3]) << 24)
        }))
    }

    /// Stable 32-byte descriptor for the GPU lookup table. Signed coordinates use two's-complement
    /// words and both generation and revision retain their complete 64-bit identity.
    pub const fn descriptor_words(&self) -> [u32; 8] {
        [
            self.coord.x as u32,
            self.coord.y as u32,
            self.coord.z as u32,
            self.address.slot,
            self.address.generation as u32,
            (self.address.generation >> 32) as u32,
            self.revision as u32,
            (self.revision >> 32) as u32,
        ]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BrickVoxelHit {
    pub voxel: [i32; 3],
    pub material: u8,
    pub distance_voxels: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BrickTrace {
    Hit(BrickVoxelHit),
    Miss,
    /// Traversal reached a brick that is not resident. Callers must not treat this as empty space.
    Unknown(BrickCoord),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueueUpdate {
    /// A new brick was queued for upload.
    Queued,
    /// A pending upload for this brick was replaced by a newer revision.
    ReplacedPending,
    /// The revision was already applied or queued and was ignored.
    Stale,
    /// Revision zero is reserved for an uninitialized brick.
    InvalidRevision,
    /// Every resident brick has one complete 8³ material payload.
    InvalidPayload,
    /// The bounded pending queue is full; callers should defer and retry this brick.
    Capacity,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Resident {
    address: BrickAddress,
    revision: u64,
    payload: Arc<[u8]>,
    non_empty: bool,
}

#[derive(Clone, Debug)]
struct Pending {
    revision: u64,
    payload: Arc<[u8]>,
    sequence: u64,
}

/// Fixed-capacity, edit-friendly brick residency.
#[derive(Clone, Debug)]
pub struct BrickResidency {
    slots: Vec<Option<BrickCoord>>,
    residents: BTreeMap<BrickCoord, Resident>,
    pending: BTreeMap<BrickCoord, Pending>,
    sequence: u64,
    next_generation: u64,
    pending_capacity: usize,
}

impl BrickResidency {
    /// Creates a cache with a fixed number of stable GPU slots.
    pub fn new(capacity: u32) -> Self {
        Self {
            slots: vec![None; capacity as usize],
            residents: BTreeMap::new(),
            pending: BTreeMap::new(),
            sequence: 0,
            next_generation: 1,
            pending_capacity: (capacity as usize).saturating_mul(4).max(1),
        }
    }

    pub fn capacity(&self) -> u32 {
        self.slots.len() as u32
    }

    pub fn resident_len(&self) -> usize {
        self.residents.len()
    }

    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    pub const fn pending_capacity(&self) -> usize {
        self.pending_capacity
    }

    pub fn address(&self, coord: BrickCoord) -> Option<BrickAddress> {
        self.residents.get(&coord).map(|resident| resident.address)
    }

    /// Returns whether an upload token still names the current occupant of a slot.
    pub fn is_current(&self, coord: BrickCoord, address: BrickAddress) -> bool {
        self.address(coord) == Some(address)
    }

    pub fn revision(&self, coord: BrickCoord) -> Option<u64> {
        self.residents
            .get(&coord)
            .map(|resident| resident.revision)
            .or_else(|| self.pending.get(&coord).map(|pending| pending.revision))
    }

    /// Returns the current resident addresses and their occupancy flags for building a GPU
    /// spatial index. The returned snapshot is bounded by the fixed residency capacity.
    pub fn resident_bricks(&self) -> Vec<(BrickCoord, BrickAddress, bool)> {
        self.residents
            .iter()
            .map(|(&coord, resident)| (coord, resident.address, resident.non_empty))
            .collect()
    }

    /// Samples a resident voxel using the canonical x + z*edge + y*edge² layout. Missing bricks
    /// return `None`; this distinction is required by a renderer that must fail closed while
    /// streaming.
    pub fn sample(&self, voxel: [i32; 3]) -> Option<u8> {
        let brick = BrickCoord::new(
            voxel[0].div_euclid(BRICK_EDGE as i32),
            voxel[1].div_euclid(BRICK_EDGE as i32),
            voxel[2].div_euclid(BRICK_EDGE as i32),
        );
        let resident = self.residents.get(&brick)?;
        let x = voxel[0].rem_euclid(BRICK_EDGE as i32) as usize;
        let y = voxel[1].rem_euclid(BRICK_EDGE as i32) as usize;
        let z = voxel[2].rem_euclid(BRICK_EDGE as i32) as usize;
        resident
            .payload
            .get(x + z * BRICK_EDGE as usize + y * BRICK_EDGE as usize * BRICK_EDGE as usize)
            .copied()
    }

    /// Traverses resident bricks in voxel space. A missing brick is an explicit `Unknown` result,
    /// never a transparent gap, so callers can keep the last certified image while streaming.
    pub fn trace(
        &self,
        origin: [f32; 3],
        direction: [f32; 3],
        max_distance_voxels: f32,
    ) -> BrickTrace {
        if !origin.iter().all(|value| value.is_finite())
            || !direction.iter().all(|value| value.is_finite())
            || !max_distance_voxels.is_finite()
            || max_distance_voxels <= 0.0
        {
            return BrickTrace::Miss;
        }
        let length = direction
            .iter()
            .map(|value| value * value)
            .sum::<f32>()
            .sqrt();
        if !length.is_finite() || length <= f32::EPSILON {
            return BrickTrace::Miss;
        }
        let direction = direction.map(|value| value / length);
        let mut voxel = [
            origin[0].floor() as i32,
            origin[1].floor() as i32,
            origin[2].floor() as i32,
        ];
        let axis_step = |value: f32| {
            value
                .partial_cmp(&0.0)
                .map_or(0, |ordering| match ordering {
                    std::cmp::Ordering::Greater => 1,
                    std::cmp::Ordering::Less => -1,
                    std::cmp::Ordering::Equal => 0,
                })
        };
        let step = direction.map(axis_step);
        let mut next = [f32::INFINITY; 3];
        let mut delta = [f32::INFINITY; 3];
        for axis in 0..3 {
            if step[axis] == 0 {
                continue;
            }
            let boundary = if step[axis] > 0 {
                voxel[axis] as f32 + 1.0
            } else {
                voxel[axis] as f32
            };
            next[axis] = (boundary - origin[axis]) / direction[axis];
            delta[axis] = 1.0 / direction[axis].abs();
        }

        let mut distance = 0.0;
        let max_steps = (max_distance_voxels.ceil() as usize)
            .saturating_add(3)
            .min(MAX_TRACE_STEPS);
        for _ in 0..max_steps {
            let brick = BrickCoord::new(
                voxel[0].div_euclid(BRICK_EDGE as i32),
                voxel[1].div_euclid(BRICK_EDGE as i32),
                voxel[2].div_euclid(BRICK_EDGE as i32),
            );
            let Some(resident) = self.residents.get(&brick) else {
                return BrickTrace::Unknown(brick);
            };
            if !resident.non_empty {
                let mut brick_boundary = [f32::INFINITY; 3];
                let brick_axes = [brick.x, brick.y, brick.z];
                for axis in 0..3 {
                    if step[axis] == 0 {
                        continue;
                    }
                    let boundary_voxel = if step[axis] > 0 {
                        (brick_axes[axis] + 1) * BRICK_EDGE as i32
                    } else {
                        brick_axes[axis] * BRICK_EDGE as i32
                    };
                    brick_boundary[axis] = (boundary_voxel as f32 - origin[axis]) / direction[axis];
                }
                let next_boundary = if brick_boundary[0] <= brick_boundary[1]
                    && brick_boundary[0] <= brick_boundary[2]
                {
                    brick_boundary[0]
                } else if brick_boundary[1] <= brick_boundary[2] {
                    brick_boundary[1]
                } else {
                    brick_boundary[2]
                };
                if !next_boundary.is_finite() || next_boundary > max_distance_voxels {
                    return BrickTrace::Miss;
                }
                distance = next_boundary;
                let point = [
                    origin[0] + direction[0] * (distance + 1.0e-4),
                    origin[1] + direction[1] * (distance + 1.0e-4),
                    origin[2] + direction[2] * (distance + 1.0e-4),
                ];
                voxel = [
                    point[0].floor() as i32,
                    point[1].floor() as i32,
                    point[2].floor() as i32,
                ];
                for axis in 0..3 {
                    next[axis] = if step[axis] == 0 {
                        f32::INFINITY
                    } else {
                        let boundary = if step[axis] > 0 {
                            voxel[axis] as f32 + 1.0
                        } else {
                            voxel[axis] as f32
                        };
                        (boundary - origin[axis]) / direction[axis]
                    };
                }
                continue;
            }
            let x = voxel[0].rem_euclid(BRICK_EDGE as i32) as usize;
            let y = voxel[1].rem_euclid(BRICK_EDGE as i32) as usize;
            let z = voxel[2].rem_euclid(BRICK_EDGE as i32) as usize;
            let index = x + z * BRICK_EDGE as usize + y * BRICK_EDGE as usize * BRICK_EDGE as usize;
            if resident.payload.get(index).copied().unwrap_or(0) != 0 {
                return BrickTrace::Hit(BrickVoxelHit {
                    voxel,
                    material: resident.payload[index],
                    distance_voxels: distance,
                });
            }
            let axis = if next[0] <= next[1] && next[0] <= next[2] {
                0
            } else if next[1] <= next[2] {
                1
            } else {
                2
            };
            distance = next[axis];
            if distance > max_distance_voxels {
                return BrickTrace::Miss;
            }
            voxel[axis] = match voxel[axis].checked_add(step[axis]) {
                Some(value) => value,
                None => return BrickTrace::Miss,
            };
            next[axis] += delta[axis];
        }
        // A bounded traversal that did not reach the requested distance is incomplete, so keep
        // the current brick unknown rather than certifying a false miss.
        BrickTrace::Unknown(BrickCoord::new(
            voxel[0].div_euclid(BRICK_EDGE as i32),
            voxel[1].div_euclid(BRICK_EDGE as i32),
            voxel[2].div_euclid(BRICK_EDGE as i32),
        ))
    }

    /// Queues the newest edit for one brick.  Multiple edits before a frame are coalesced, so the
    /// upload budget is bounded by `drain_uploads(max_uploads)` rather than edit count.
    pub fn queue_update(
        &mut self,
        coord: BrickCoord,
        revision: u64,
        payload: Arc<[u8]>,
    ) -> QueueUpdate {
        if revision == 0 {
            return QueueUpdate::InvalidRevision;
        }
        if payload.len() != BRICK_VOXEL_COUNT {
            return QueueUpdate::InvalidPayload;
        }
        if self
            .revision(coord)
            .is_some_and(|current| revision <= current)
        {
            return QueueUpdate::Stale;
        }
        if !self.pending.contains_key(&coord) && self.pending.len() >= self.pending_capacity {
            return QueueUpdate::Capacity;
        }
        self.sequence = self.sequence.wrapping_add(1);
        let replaced = self.pending.insert(
            coord,
            Pending {
                revision,
                payload,
                sequence: self.sequence,
            },
        );
        if replaced.is_some() {
            QueueUpdate::ReplacedPending
        } else {
            QueueUpdate::Queued
        }
    }

    /// Applies at most `max_uploads` queued bricks.  Existing residents retain their address;
    /// newly allocated bricks use the first free slot.  If all slots are occupied, pending edits
    /// remain queued until the caller explicitly evicts a brick.
    pub fn drain_uploads(&mut self, max_uploads: usize) -> Vec<BrickUpload> {
        let mut uploads = Vec::with_capacity(max_uploads);
        for _ in 0..max_uploads {
            let free_slot = self.slots.iter().position(Option::is_none);
            let Some(coord) = self
                .pending
                .iter()
                .filter(|(coord, _)| free_slot.is_some() || self.residents.contains_key(coord))
                .min_by_key(|(_, pending)| pending.sequence)
                .map(|(coord, _)| *coord)
            else {
                break;
            };

            let address = if let Some(resident) = self.residents.get(&coord) {
                resident.address
            } else {
                let Some(slot) = free_slot else {
                    break;
                };
                let address = BrickAddress {
                    slot: slot as u32,
                    generation: self.next_generation,
                };
                self.next_generation = self.next_generation.wrapping_add(1).max(1);
                self.slots[slot] = Some(coord);
                address
            };
            let pending = self
                .pending
                .remove(&coord)
                .expect("pending coordinate selected above");
            self.residents.insert(
                coord,
                Resident {
                    address,
                    revision: pending.revision,
                    non_empty: pending.payload.iter().any(|material| *material != 0),
                    payload: Arc::clone(&pending.payload),
                },
            );
            uploads.push(BrickUpload {
                coord,
                address,
                revision: pending.revision,
                payload: pending.payload,
            });
        }
        uploads
    }

    /// Evicts one resident brick and any superseded pending edit.  The slot is immediately free,
    /// while its generation token is advanced on the next allocation.
    pub fn evict(&mut self, coord: BrickCoord) -> bool {
        self.pending.remove(&coord);
        let Some(resident) = self.residents.remove(&coord) else {
            return false;
        };
        if self
            .slots
            .get(resident.address.slot as usize)
            .is_some_and(|slot| *slot == Some(coord))
        {
            self.slots[resident.address.slot as usize] = None;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(value: u8) -> Arc<[u8]> {
        Arc::from(vec![value; BRICK_VOXEL_COUNT])
    }

    #[test]
    fn edits_coalesce_and_keep_stable_address() {
        let coord = BrickCoord::new(2, -1, 7);
        let mut cache = BrickResidency::new(1);
        assert_eq!(
            cache.queue_update(coord, 1, payload(1)),
            QueueUpdate::Queued
        );
        assert_eq!(
            cache.queue_update(coord, 2, payload(2)),
            QueueUpdate::ReplacedPending
        );
        let first = cache.drain_uploads(8);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].revision, 2);
        assert_eq!(first[0].payload[0], 2);
        let address = first[0].address;

        assert_eq!(
            cache.queue_update(coord, 3, payload(3)),
            QueueUpdate::Queued
        );
        let second = cache.drain_uploads(1);
        assert_eq!(second[0].address, address);
        assert_eq!(cache.address(coord), Some(address));
    }

    #[test]
    fn capacity_bounds_uploads_until_explicit_eviction() {
        let mut cache = BrickResidency::new(1);
        let a = BrickCoord::new(0, 0, 0);
        let b = BrickCoord::new(1, 0, 0);
        cache.queue_update(a, 1, payload(4));
        cache.queue_update(b, 1, payload(5));
        let first = cache.drain_uploads(8);
        assert_eq!(first.len(), 1);
        assert_eq!(cache.pending_len(), 1);
        assert!(cache.is_current(a, first[0].address));
        assert!(cache.evict(a));
        assert!(!cache.is_current(a, first[0].address));
        let uploads = cache.drain_uploads(8);
        assert_eq!(uploads.len(), 1);
        assert_ne!(uploads[0].address.generation, first[0].address.generation);
    }

    #[test]
    fn full_cache_newcomers_do_not_block_edits_to_resident_bricks() {
        let mut cache = BrickResidency::new(1);
        let resident = BrickCoord::new(0, 0, 0);
        let newcomer = BrickCoord::new(1, 0, 0);
        cache.queue_update(resident, 1, payload(1));
        cache.drain_uploads(1);
        cache.queue_update(newcomer, 1, payload(2));
        cache.queue_update(resident, 2, payload(3));
        let uploads = cache.drain_uploads(1);
        assert_eq!(uploads.len(), 1);
        assert_eq!(uploads[0].coord, resident);
        assert_eq!(uploads[0].revision, 2);
        assert_eq!(cache.pending_len(), 1);
    }

    #[test]
    fn pending_queue_is_bounded_but_existing_bricks_can_still_be_revised() {
        let mut cache = BrickResidency::new(1);
        assert_eq!(cache.pending_capacity(), 4);
        for index in 0..4 {
            assert_eq!(
                cache.queue_update(BrickCoord::new(index, 0, 0), 1, payload(index as u8)),
                QueueUpdate::Queued
            );
        }
        assert_eq!(
            cache.queue_update(BrickCoord::new(4, 0, 0), 1, payload(4)),
            QueueUpdate::Capacity
        );
        assert_eq!(
            cache.queue_update(BrickCoord::new(0, 0, 0), 2, payload(9)),
            QueueUpdate::ReplacedPending
        );
    }

    #[test]
    fn stale_and_zero_revisions_are_rejected() {
        let coord = BrickCoord::new(0, 0, 0);
        let mut cache = BrickResidency::new(1);
        assert_eq!(
            cache.queue_update(coord, 0, payload(0)),
            QueueUpdate::InvalidRevision
        );
        cache.queue_update(coord, 4, payload(4));
        assert_eq!(cache.queue_update(coord, 3, payload(3)), QueueUpdate::Stale);
        cache.drain_uploads(1);
        assert_eq!(cache.queue_update(coord, 4, payload(4)), QueueUpdate::Stale);
    }

    #[test]
    fn malformed_payloads_never_enter_residency() {
        let mut cache = BrickResidency::new(1);
        assert_eq!(
            cache.queue_update(BrickCoord::new(0, 0, 0), 1, Arc::from(vec![1, 2, 3])),
            QueueUpdate::InvalidPayload
        );
        assert_eq!(cache.pending_len(), 0);
        assert_eq!(cache.resident_len(), 0);
    }

    #[test]
    fn upload_budget_is_honored_and_order_is_deterministic() {
        let mut cache = BrickResidency::new(3);
        for (index, revision) in [10_u64, 20, 30].into_iter().enumerate() {
            cache.queue_update(
                BrickCoord::new(index as i32, 0, 0),
                revision,
                payload(index as u8),
            );
        }
        assert_eq!(cache.drain_uploads(2).len(), 2);
        assert_eq!(cache.pending_len(), 1);
        assert_eq!(cache.drain_uploads(2).len(), 1);
    }

    #[test]
    fn trace_hits_negative_voxels_and_preserves_material_ids() {
        let mut cache = BrickResidency::new(1);
        let mut bytes = vec![0; BRICK_VOXEL_COUNT];
        let x = 7usize;
        let y = 2usize;
        let z = 3usize;
        bytes[x + z * BRICK_EDGE as usize + y * BRICK_EDGE as usize * BRICK_EDGE as usize] = 9;
        let coord = BrickCoord::new(-1, -1, -1);
        assert_eq!(
            cache.queue_update(coord, 1, Arc::from(bytes)),
            QueueUpdate::Queued
        );
        cache.drain_uploads(1);
        assert_eq!(cache.sample([-1, -6, -5]), Some(9));
        assert_eq!(
            cache.trace([-1.9, -5.5, -4.5], [1.0, 0.0, 0.0], 4.0),
            BrickTrace::Hit(BrickVoxelHit {
                voxel: [-1, -6, -5],
                material: 9,
                distance_voxels: 0.9,
            })
        );
    }

    #[test]
    fn trace_reports_unknown_bricks_instead_of_empty_space() {
        let mut cache = BrickResidency::new(1);
        let payload = Arc::from(vec![0; BRICK_VOXEL_COUNT]);
        assert_eq!(
            cache.queue_update(BrickCoord::new(0, 0, 0), 1, payload),
            QueueUpdate::Queued
        );
        cache.drain_uploads(1);
        assert_eq!(
            cache.trace([0.5, 0.5, 0.5], [1.0, 0.0, 0.0], 16.0),
            BrickTrace::Unknown(BrickCoord::new(1, 0, 0))
        );
    }

    #[test]
    fn huge_finite_trace_distance_is_bounded_without_overflow() {
        let cache = BrickResidency::new(1);
        assert_eq!(
            cache.trace([0.5, 0.5, 0.5], [1.0, 0.0, 0.0], f32::MAX),
            BrickTrace::Unknown(BrickCoord::new(0, 0, 0))
        );
    }

    #[test]
    fn gpu_words_preserve_material_lanes_and_full_identity() {
        let mut bytes = vec![0; BRICK_VOXEL_COUNT];
        bytes[..4].copy_from_slice(&[1, 2, 128, 255]);
        let upload = BrickUpload {
            coord: BrickCoord::new(-2, 3, -4),
            address: BrickAddress {
                slot: 7,
                generation: 0x1234_5678_9abc_def0,
            },
            revision: 0xfedc_ba98_7654_3210,
            payload: Arc::from(bytes),
        };
        assert_eq!(upload.byte_offset(), 7 * BRICK_VOXEL_COUNT as u64);
        assert_eq!(upload.material_words().unwrap()[0], 0xff80_0201);
        assert_eq!(
            upload.descriptor_words(),
            [
                (-2i32) as u32,
                3,
                (-4i32) as u32,
                7,
                0x9abc_def0,
                0x1234_5678,
                0x7654_3210,
                0xfedc_ba98
            ]
        );
    }
}
