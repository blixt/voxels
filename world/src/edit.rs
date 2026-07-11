use crate::{CHUNK_EDGE, Chunk, ChunkCoord, Generator, Material};
use std::collections::BTreeMap;

/// Integer address of one canonical 10 cm voxel.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct VoxelCoord {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl VoxelCoord {
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    pub const fn as_array(self) -> [i32; 3] {
        [self.x, self.y, self.z]
    }

    pub fn chunk(self) -> ChunkCoord {
        let edge = CHUNK_EDGE as i32;
        ChunkCoord::new(
            self.x.div_euclid(edge),
            self.y.div_euclid(edge),
            self.z.div_euclid(edge),
        )
    }

    pub fn local(self) -> [usize; 3] {
        let edge = CHUNK_EDGE as i32;
        [
            self.x.rem_euclid(edge) as usize,
            self.y.rem_euclid(edge) as usize,
            self.z.rem_euclid(edge) as usize,
        ]
    }
}

/// Sparse, deterministic overlay on top of procedural terrain. Only values that differ from the
/// generator are retained, making the edit set suitable for an append journal and compact SQLite
/// snapshots without copying untouched chunks.
#[derive(Clone, Debug, Default)]
pub struct EditMap {
    overrides: BTreeMap<VoxelCoord, Material>,
}

impl EditMap {
    pub fn sample(&self, generator: Generator, coord: VoxelCoord) -> Material {
        self.overrides
            .get(&coord)
            .copied()
            .unwrap_or_else(|| generator.sample(coord.x, coord.y, coord.z))
    }

    pub fn set(&mut self, generator: Generator, coord: VoxelCoord, material: Material) {
        if generator.sample(coord.x, coord.y, coord.z) == material {
            self.overrides.remove(&coord);
        } else {
            self.overrides.insert(coord, material);
        }
    }

    /// Used only when hydrating already validated durable rows.
    pub fn insert_override(&mut self, coord: VoxelCoord, material: Material) {
        self.overrides.insert(coord, material);
    }

    pub fn override_at(&self, coord: VoxelCoord) -> Option<Material> {
        self.overrides.get(&coord).copied()
    }

    pub fn len(&self) -> usize {
        self.overrides.len()
    }

    pub fn is_empty(&self) -> bool {
        self.overrides.is_empty()
    }

    pub fn apply_to_chunk(&self, chunk: &mut Chunk) {
        let coord = chunk.coord();
        for (voxel, material) in &self.overrides {
            if voxel.chunk() == coord {
                let [x, y, z] = voxel.local();
                chunk.set(x, y, z, *material);
            }
        }
    }

    /// Resolves the visible material of one edited column. Far surface summaries use this to remain
    /// derived from generator + edits rather than silently becoming a second world authority.
    pub fn surface_sample(&self, generator: Generator, x: i32, z: i32) -> (i32, Material) {
        let generated_height = generator.surface_height(x, z);
        let highest_override = self
            .overrides
            .iter()
            .filter(|(coord, material)| coord.x == x && coord.z == z && material.is_solid())
            .map(|(coord, _)| coord.y)
            .max();
        let mut y = highest_override.map_or(generated_height, |value| value.max(generated_height));
        loop {
            let material = self.sample(generator, VoxelCoord::new(x, y, z));
            if material.is_solid() || y <= -16 {
                return (y, material);
            }
            y -= 1;
        }
    }

    /// Chunks whose meshes can change after this voxel changes. A face on a chunk boundary also
    /// invalidates the neighboring mesh so a removed or added block cannot leave a stale seam.
    pub fn affected_chunks(coord: VoxelCoord) -> Vec<ChunkCoord> {
        let edge = CHUNK_EDGE as i32;
        let local = [
            coord.x.rem_euclid(edge),
            coord.y.rem_euclid(edge),
            coord.z.rem_euclid(edge),
        ];
        let base = coord.chunk();
        let mut chunks = vec![base];
        for axis in 0..3 {
            for direction in [-1, 1] {
                let boundary = if direction < 0 { 0 } else { edge - 1 };
                if local[axis] != boundary {
                    continue;
                }
                let mut neighbor = [base.x, base.y, base.z];
                neighbor[axis] += direction;
                chunks.push(ChunkCoord::new(neighbor[0], neighbor[1], neighbor[2]));
            }
        }
        chunks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverting_to_generated_value_removes_override() {
        let generator = Generator::new(7);
        let coord = VoxelCoord::new(4, 30, -2);
        let generated = generator.sample(coord.x, coord.y, coord.z);
        let replacement = if generated == Material::Air {
            Material::Stone
        } else {
            Material::Air
        };
        let mut edits = EditMap::default();
        edits.set(generator, coord, replacement);
        assert_eq!(edits.sample(generator, coord), replacement);
        assert_eq!(edits.len(), 1);
        edits.set(generator, coord, generated);
        assert!(edits.is_empty());
    }

    #[test]
    fn boundary_edit_invalidates_both_chunks() {
        let chunks = EditMap::affected_chunks(VoxelCoord::new(-1, 64, 31));
        assert_eq!(chunks.len(), 4);
        assert!(chunks.contains(&ChunkCoord::new(-1, 2, 0)));
        assert!(chunks.contains(&ChunkCoord::new(0, 2, 0)));
        assert!(chunks.contains(&ChunkCoord::new(-1, 1, 0)));
        assert!(chunks.contains(&ChunkCoord::new(-1, 2, 1)));
    }

    #[test]
    fn edited_column_surface_tracks_additions_and_removals() {
        let generator = Generator::new(11);
        let x = 7;
        let z = -9;
        let generated = generator.surface_height(x, z);
        let mut edits = EditMap::default();
        edits.set(
            generator,
            VoxelCoord::new(x, generated + 5, z),
            Material::Stone,
        );
        assert_eq!(edits.surface_sample(generator, x, z).0, generated + 5);
        edits.set(
            generator,
            VoxelCoord::new(x, generated + 5, z),
            Material::Air,
        );
        edits.set(generator, VoxelCoord::new(x, generated, z), Material::Air);
        assert_eq!(edits.surface_sample(generator, x, z).0, generated - 1);
    }
}
