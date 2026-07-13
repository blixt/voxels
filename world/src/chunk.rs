use crate::material::Material;
use serde::{Deserialize, Serialize};

pub const CHUNK_EDGE: usize = 32;
pub const CHUNK_VOLUME: usize = CHUNK_EDGE * CHUNK_EDGE * CHUNK_EDGE;
pub const CHUNK_VOXEL_BYTES: usize = CHUNK_VOLUME * size_of::<Material>();

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct ChunkCoord {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl ChunkCoord {
    const MIN_WORLD_AXIS: i32 = i32::MIN.div_euclid(CHUNK_EDGE as i32);
    const MAX_WORLD_AXIS: i32 = i32::MAX.div_euclid(CHUNK_EDGE as i32);

    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    pub const fn is_world_representable(self) -> bool {
        self.x >= Self::MIN_WORLD_AXIS
            && self.x <= Self::MAX_WORLD_AXIS
            && self.y >= Self::MIN_WORLD_AXIS
            && self.y <= Self::MAX_WORLD_AXIS
            && self.z >= Self::MIN_WORLD_AXIS
            && self.z <= Self::MAX_WORLD_AXIS
    }

    pub const fn world_origin(self) -> [i32; 3] {
        assert!(
            self.is_world_representable(),
            "chunk coordinate is outside the canonical voxel grid"
        );
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
        (coord.is_world_representable() && voxels.len() == CHUNK_VOLUME).then(|| Self {
            coord,
            voxels: voxels.into_boxed_slice(),
        })
    }

    pub fn empty(coord: ChunkCoord) -> Self {
        assert!(
            coord.is_world_representable(),
            "chunk coordinate is outside the canonical voxel grid"
        );
        Self {
            coord,
            voxels: vec![Material::Air; CHUNK_VOLUME].into_boxed_slice(),
        }
    }

    pub fn filled(coord: ChunkCoord, material: Material) -> Self {
        assert!(
            coord.is_world_representable(),
            "chunk coordinate is outside the canonical voxel grid"
        );
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
    assert!(
        x < CHUNK_EDGE && y < CHUNK_EDGE && z < CHUNK_EDGE,
        "chunk-local coordinate is outside the chunk"
    );
    // Y-Z-X traversal: X is contiguous, then Z, then Y. Horizontal runs stay cache-friendly for the
    // mesher while terrain layers remain compression-friendly.
    x + z * CHUNK_EDGE + y * CHUNK_EDGE * CHUNK_EDGE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_coordinates_do_not_alias() {
        assert_eq!(CHUNK_VOXEL_BYTES, 65_536);
        let mut chunk = Chunk::empty(ChunkCoord::new(-2, 1, 3));
        chunk.set(31, 30, 29, Material::Snow);
        assert_eq!(chunk.get(31, 30, 29), Material::Snow);
        assert_eq!(chunk.get(30, 31, 29), Material::Air);
        assert_eq!(chunk.coord().world_origin(), [-64, 32, 96]);
    }

    #[test]
    fn boundary_chunks_have_exact_representable_origins() {
        let minimum = ChunkCoord::new(ChunkCoord::MIN_WORLD_AXIS, 0, 0);
        let maximum = ChunkCoord::new(ChunkCoord::MAX_WORLD_AXIS, 0, 0);
        assert_eq!(minimum.world_origin()[0], i32::MIN);
        assert_eq!(maximum.world_origin()[0], i32::MAX - CHUNK_EDGE as i32 + 1);
        assert!(!ChunkCoord::new(ChunkCoord::MIN_WORLD_AXIS - 1, 0, 0).is_world_representable());
        assert!(!ChunkCoord::new(ChunkCoord::MAX_WORLD_AXIS + 1, 0, 0).is_world_representable());
    }

    #[test]
    #[should_panic(expected = "chunk-local coordinate is outside the chunk")]
    fn out_of_range_x_cannot_alias_the_next_z_row() {
        let chunk = Chunk::empty(ChunkCoord::new(0, 0, 0));
        let _ = chunk.get(CHUNK_EDGE, 0, 0);
    }

    #[test]
    #[should_panic(expected = "chunk-local coordinate is outside the chunk")]
    fn out_of_range_z_cannot_alias_the_next_y_layer() {
        let mut chunk = Chunk::empty(ChunkCoord::new(0, 0, 0));
        chunk.set(0, 0, CHUNK_EDGE, Material::Stone);
    }
}
