//! WGPU storage for the direct voxel traversal brick path.
//!
//! The atlas deliberately has no renderer policy: callers decide which authoritative bricks are
//! resident, queue revisioned payloads, and drain a bounded number of uploads per frame. Material
//! bytes and descriptors use the layouts documented by [`crate::brick_residency`].

use wgpu::{Buffer, BufferUsages, Device, Queue};

use crate::brick_residency::{
    BRICK_VOXEL_COUNT, BrickCoord, BrickResidency, BrickUpload, QueueUpdate,
};
use crate::brick_hash::{BrickHashItem, BrickHashTable, GpuBrickHashEntry};

pub const GPU_BRICK_DESCRIPTOR_WORDS: usize = 8;
pub const GPU_BRICK_DESCRIPTOR_BYTES: u64 = (GPU_BRICK_DESCRIPTOR_WORDS * size_of::<u32>()) as u64;

#[derive(Debug)]
pub struct GpuBrickAtlas {
    residency: BrickResidency,
    material_buffer: Buffer,
    descriptor_buffer: Buffer,
    hash_buffer: Buffer,
    hash_capacity: u32,
    hash_table: BrickHashTable,
}

impl GpuBrickAtlas {
    pub fn new(device: &Device, capacity: u32) -> Result<Self, String> {
        if capacity == 0 {
            return Err("GPU brick atlas capacity must be greater than zero".to_owned());
        }
        let material_size = u64::from(capacity)
            .checked_mul(BRICK_VOXEL_COUNT as u64)
            .ok_or_else(|| "GPU brick material buffer size overflowed".to_owned())?;
        let descriptor_size = u64::from(capacity)
            .checked_mul(GPU_BRICK_DESCRIPTOR_BYTES)
            .ok_or_else(|| "GPU brick descriptor buffer size overflowed".to_owned())?;
        let hash_capacity = capacity
            .checked_next_power_of_two()
            .ok_or_else(|| "GPU brick hash capacity overflowed".to_owned())?
            .checked_mul(2)
            .ok_or_else(|| "GPU brick hash capacity overflowed".to_owned())?;
        let hash_size = u64::from(hash_capacity)
            .checked_mul(size_of::<GpuBrickHashEntry>() as u64)
            .ok_or_else(|| "GPU brick hash buffer size overflowed".to_owned())?;
        let limits = device.limits();
        if material_size > limits.max_storage_buffer_binding_size as u64
            || descriptor_size > limits.max_storage_buffer_binding_size as u64
            || hash_size > limits.max_storage_buffer_binding_size as u64
        {
            return Err(format!(
                "GPU brick atlas exceeds storage binding limit: material={material_size}, descriptors={descriptor_size}, hash={hash_size}, limit={}",
                limits.max_storage_buffer_binding_size
            ));
        }
        let material_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("direct voxel brick material atlas"),
            size: material_size,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let descriptor_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("direct voxel brick descriptor table"),
            size: descriptor_size,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let hash_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("direct voxel brick hash table"),
            size: hash_size,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Self {
            residency: BrickResidency::new(capacity),
            material_buffer,
            descriptor_buffer,
            hash_buffer,
            hash_capacity,
            hash_table: BrickHashTable::new(hash_capacity)
                .expect("derived GPU brick hash capacity is a power of two"),
        })
    }

    pub fn capacity(&self) -> u32 {
        self.residency.capacity()
    }

    pub fn resident_len(&self) -> usize {
        self.residency.resident_len()
    }

    pub fn pending_len(&self) -> usize {
        self.residency.pending_len()
    }

    pub fn queue_update(
        &mut self,
        coord: BrickCoord,
        revision: u64,
        payload: std::sync::Arc<[u8]>,
    ) -> QueueUpdate {
        self.residency.queue_update(coord, revision, payload)
    }

    /// Writes at most `max_uploads` changed bricks. Queue writes are ordered, so a descriptor is
    /// visible to a later dispatch only after its corresponding material words have been copied.
    pub fn flush(&mut self, queue: &Queue, max_uploads: usize) -> usize {
        let uploads = self.residency.drain_uploads(max_uploads);
        for upload in &uploads {
            let words = upload
                .material_words()
                .expect("residency rejects malformed brick payloads");
            queue.write_buffer(
                &self.material_buffer,
                upload.byte_offset(),
                bytemuck::cast_slice(&words),
            );
            queue.write_buffer(
                &self.descriptor_buffer,
                descriptor_offset(upload),
                bytemuck::cast_slice(&upload.descriptor_words()),
            );
        }
        self.refresh_hash_table(queue);
        uploads.len()
    }

    /// Clears the descriptor before releasing its slot. A future generation can reuse the same
    /// slot without an in-flight traversal observing the old coordinate as a valid brick.
    pub fn evict(&mut self, queue: &Queue, coord: BrickCoord) -> bool {
        let Some(address) = self.residency.address(coord) else {
            return false;
        };
        queue.write_buffer(
            &self.descriptor_buffer,
            u64::from(address.slot) * GPU_BRICK_DESCRIPTOR_BYTES,
            &[0; GPU_BRICK_DESCRIPTOR_WORDS * size_of::<u32>()],
        );
        let evicted = self.residency.evict(coord);
        self.refresh_hash_table(queue);
        evicted
    }

    pub fn material_buffer(&self) -> &Buffer {
        &self.material_buffer
    }

    pub fn descriptor_buffer(&self) -> &Buffer {
        &self.descriptor_buffer
    }

    pub const fn hash_capacity(&self) -> u32 {
        self.hash_capacity
    }

    pub fn hash_buffer(&self) -> &Buffer {
        &self.hash_buffer
    }

    /// Uploads a complete, bounded hash-table snapshot. The caller chooses when to rebuild and
    /// can keep the previous table active until this queue write is ordered before dispatch.
    pub fn upload_hash_table(&self, queue: &Queue, table: &BrickHashTable) -> Result<(), String> {
        if table.capacity() != self.hash_capacity {
            return Err(format!(
                "brick hash capacity {} does not match atlas capacity {}",
                table.capacity(),
                self.hash_capacity
            ));
        }
        queue.write_buffer(&self.hash_buffer, 0, bytemuck::cast_slice(table.entries()));
        Ok(())
    }

    fn refresh_hash_table(&mut self, queue: &Queue) {
        let items = self
            .residency
            .resident_bricks()
            .into_iter()
            .map(|(coord, address, non_empty)| BrickHashItem {
                coord,
                address,
                non_empty,
            });
        self.hash_table
            .rebuild(items)
            .expect("GPU brick hash table capacity must accommodate residency");
        queue.write_buffer(
            &self.hash_buffer,
            0,
            bytemuck::cast_slice(self.hash_table.entries()),
        );
    }
}

fn descriptor_offset(upload: &BrickUpload) -> u64 {
    u64::from(upload.address.slot) * GPU_BRICK_DESCRIPTOR_BYTES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atlas_layout_is_fixed_and_word_aligned() {
        assert_eq!(BRICK_VOXEL_COUNT % 4, 0);
        assert_eq!(GPU_BRICK_DESCRIPTOR_BYTES, 32);
        assert_eq!(descriptor_offset_for_slot(3), 96);
    }

    const fn descriptor_offset_for_slot(slot: u32) -> u64 {
        slot as u64 * GPU_BRICK_DESCRIPTOR_BYTES
    }
}
