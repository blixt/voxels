use std::collections::{HashMap, VecDeque};
use voxels_core::VoxelPhysics;
use voxels_world::{Chunk, ChunkCoord, Material, VoxelCoord};

pub struct ChunkCache {
    capacity: usize,
    chunks: HashMap<ChunkCoord, Chunk>,
    insertion_order: VecDeque<ChunkCoord>,
}

impl ChunkCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            chunks: HashMap::with_capacity(capacity),
            insertion_order: VecDeque::with_capacity(capacity),
        }
    }

    pub fn insert(&mut self, chunk: Chunk) {
        let coord = chunk.coord();
        if self.chunks.insert(coord, chunk).is_some() {
            return;
        }
        self.insertion_order.push_back(coord);
        while self.chunks.len() > self.capacity {
            if let Some(evicted) = self.insertion_order.pop_front() {
                self.chunks.remove(&evicted);
            }
        }
    }

    pub fn apply(&mut self, coord: VoxelCoord, material: Material) {
        if let Some(chunk) = self.chunks.get_mut(&coord.chunk()) {
            let [x, y, z] = coord.local();
            chunk.set(x, y, z, material);
        }
    }

    pub fn clear(&mut self) {
        self.chunks.clear();
        self.insertion_order.clear();
    }

    pub fn contains(&self, coord: ChunkCoord) -> bool {
        self.chunks.contains_key(&coord)
    }

    pub fn material(&self, coord: VoxelCoord) -> Option<Material> {
        self.chunks.get(&coord.chunk()).map(|chunk| {
            let [x, y, z] = coord.local();
            chunk.get(x, y, z)
        })
    }

    pub fn physics(&self, coord: VoxelCoord) -> VoxelPhysics {
        let material = self.material(coord).unwrap_or(Material::Stone);
        VoxelPhysics {
            collidable: material.is_collidable(),
            fluid: material.is_fluid(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_is_bounded_and_applies_authoritative_mutations() {
        let mut cache = ChunkCache::new(1);
        cache.insert(Chunk::filled(ChunkCoord::new(0, 0, 0), Material::Dirt));
        let changed = VoxelCoord::new(3, 4, 5);
        cache.apply(changed, Material::Air);
        assert!(!cache.physics(changed).collidable);
        assert_eq!(cache.material(changed), Some(Material::Air));
        cache.insert(Chunk::filled(ChunkCoord::new(1, 0, 0), Material::Stone));
        assert!(cache.physics(changed).collidable);
        assert_eq!(cache.material(changed), None);
        assert_eq!(cache.chunks.len(), 1);
        cache.clear();
        assert!(cache.chunks.is_empty());
        assert!(cache.insertion_order.is_empty());
    }
}
