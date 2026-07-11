use crate::material::Material;
use serde::{Deserialize, Serialize};

pub const CHUNK_EDGE: usize = 32;
pub const CHUNK_VOLUME: usize = CHUNK_EDGE * CHUNK_EDGE * CHUNK_EDGE;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct ChunkCoord {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl ChunkCoord {
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    pub const fn world_origin(self) -> [i32; 3] {
        [
            self.x * CHUNK_EDGE as i32,
            self.y * CHUNK_EDGE as i32,
            self.z * CHUNK_EDGE as i32,
        ]
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Chunk {
    coord: ChunkCoord,
    voxels: Box<[Material]>,
}

impl Chunk {
    pub(crate) fn from_voxels(coord: ChunkCoord, voxels: Vec<Material>) -> Option<Self> {
        (voxels.len() == CHUNK_VOLUME).then(|| Self {
            coord,
            voxels: voxels.into_boxed_slice(),
        })
    }

    pub fn empty(coord: ChunkCoord) -> Self {
        Self {
            coord,
            voxels: vec![Material::Air; CHUNK_VOLUME].into_boxed_slice(),
        }
    }

    pub fn filled(coord: ChunkCoord, material: Material) -> Self {
        Self {
            coord,
            voxels: vec![material; CHUNK_VOLUME].into_boxed_slice(),
        }
    }

    pub const fn coord(&self) -> ChunkCoord {
        self.coord
    }

    pub fn voxels(&self) -> &[Material] {
        &self.voxels
    }

    pub fn get(&self, x: usize, y: usize, z: usize) -> Material {
        self.voxels[index(x, y, z)]
    }

    pub fn set(&mut self, x: usize, y: usize, z: usize, material: Material) {
        self.voxels[index(x, y, z)] = material;
    }
}

const fn index(x: usize, y: usize, z: usize) -> usize {
    debug_assert!(x < CHUNK_EDGE && y < CHUNK_EDGE && z < CHUNK_EDGE);
    // Y-Z-X traversal: X is contiguous, then Z, then Y. Horizontal runs stay cache-friendly for the
    // mesher while terrain layers remain compression-friendly.
    x + z * CHUNK_EDGE + y * CHUNK_EDGE * CHUNK_EDGE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_coordinates_do_not_alias() {
        let mut chunk = Chunk::empty(ChunkCoord::new(-2, 1, 3));
        chunk.set(31, 30, 29, Material::Snow);
        assert_eq!(chunk.get(31, 30, 29), Material::Snow);
        assert_eq!(chunk.get(30, 31, 29), Material::Air);
        assert_eq!(chunk.coord().world_origin(), [-64, 32, 96]);
    }
}
