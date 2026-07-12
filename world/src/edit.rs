use crate::{CHUNK_EDGE, Chunk, ChunkCoord, Generator, Material, SkylineFeature};
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
    chunk_overrides: BTreeMap<(i32, i32, i32), BTreeMap<[usize; 3], Material>>,
    column_overrides: BTreeMap<(i32, i32), BTreeMap<i32, Material>>,
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
            self.replace_override(coord, None);
        } else {
            self.replace_override(coord, Some(material));
        }
    }

    /// Used only when hydrating already validated durable rows.
    pub fn insert_override(&mut self, coord: VoxelCoord, material: Material) {
        self.replace_override(coord, Some(material));
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
        let key = (coord.x, coord.y, coord.z);
        let Some(overrides) = self.chunk_overrides.get(&key) else {
            return;
        };
        for (&[x, y, z], &material) in overrides {
            chunk.set(x, y, z, material);
        }
    }

    /// Returns whether a disposable skyline proxy still represents pristine canonical feature
    /// voxels. The query walks only chunk-index buckets touched by the small analytic feature, so
    /// unrelated large edit journals do not affect surface-tile generation cost.
    pub fn skyline_feature_is_pristine(
        &self,
        generator: Generator,
        feature: SkylineFeature,
    ) -> bool {
        if self.overrides.is_empty() {
            return true;
        }
        let [min, max] = feature.bounds();
        let chunk_min = VoxelCoord::new(min[0], min[1], min[2]).chunk();
        let chunk_max = VoxelCoord::new(max[0] - 1, max[1] - 1, max[2] - 1).chunk();
        for chunk_z in chunk_min.z..=chunk_max.z {
            for chunk_y in chunk_min.y..=chunk_max.y {
                for chunk_x in chunk_min.x..=chunk_max.x {
                    let Some(overrides) = self.chunk_overrides.get(&(chunk_x, chunk_y, chunk_z))
                    else {
                        continue;
                    };
                    let origin = ChunkCoord::new(chunk_x, chunk_y, chunk_z).world_origin();
                    for &[local_x, local_y, local_z] in overrides.keys() {
                        let coord = VoxelCoord::new(
                            origin[0] + local_x as i32,
                            origin[1] + local_y as i32,
                            origin[2] + local_z as i32,
                        );
                        if coord.x < min[0]
                            || coord.x >= max[0]
                            || coord.y < min[1]
                            || coord.y >= max[1]
                            || coord.z < min[2]
                            || coord.z >= max[2]
                        {
                            continue;
                        }
                        let Some(feature_material) = feature.material_at(coord) else {
                            continue;
                        };
                        if generator.sample(coord.x, coord.y, coord.z) == feature_material {
                            return false;
                        }
                    }
                }
            }
        }
        true
    }

    /// Resolves the visible material of one edited column. Far surface summaries use this to remain
    /// derived from generator + edits rather than silently becoming a second world authority.
    pub fn surface_sample(&self, generator: Generator, x: i32, z: i32) -> (i32, Material) {
        let generated = generator.surface_sample(x, z);
        let Some(column) = self.column_overrides.get(&(x, z)) else {
            return (generated.height, generated.material);
        };
        let highest_override = column
            .iter()
            .rev()
            .find(|(_, material)| material.is_collidable())
            .map(|(&y, _)| y);
        let mut y = highest_override.map_or(generated.height, |value| value.max(generated.height));
        loop {
            let material = self.sample(generator, VoxelCoord::new(x, y, z));
            if material.is_collidable() || y <= -16 {
                return (y, material);
            }
            y -= 1;
        }
    }

    fn replace_override(&mut self, coord: VoxelCoord, material: Option<Material>) {
        if self.overrides.get(&coord).copied() == material {
            return;
        }
        let chunk = coord.chunk();
        let chunk_key = (chunk.x, chunk.y, chunk.z);
        let column_key = (coord.x, coord.z);
        if let Some(material) = material {
            self.overrides.insert(coord, material);
            self.chunk_overrides
                .entry(chunk_key)
                .or_default()
                .insert(coord.local(), material);
            self.column_overrides
                .entry(column_key)
                .or_default()
                .insert(coord.y, material);
            return;
        }

        self.overrides.remove(&coord);
        let remove_chunk = self
            .chunk_overrides
            .get_mut(&chunk_key)
            .is_some_and(|chunk| {
                chunk.remove(&coord.local());
                chunk.is_empty()
            });
        if remove_chunk {
            self.chunk_overrides.remove(&chunk_key);
        }
        let remove_column = self
            .column_overrides
            .get_mut(&column_key)
            .is_some_and(|column| {
                column.remove(&coord.y);
                column.is_empty()
            });
        if remove_column {
            self.column_overrides.remove(&column_key);
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
    fn generated_water_can_be_removed_and_restored_sparsely() {
        let generator = Generator::new(0x5eed_cafe);
        let coord = VoxelCoord::new(18_016, crate::SEA_LEVEL_VOXELS, 12_896);
        assert_eq!(generator.sample(coord.x, coord.y, coord.z), Material::Water);
        let mut edits = EditMap::default();
        edits.set(generator, coord, Material::Air);
        assert_eq!(edits.sample(generator, coord), Material::Air);
        assert_eq!(edits.override_at(coord), Some(Material::Air));
        edits.set(generator, coord, Material::Water);
        assert_eq!(edits.sample(generator, coord), Material::Water);
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

    #[test]
    fn chunk_and_column_indices_follow_override_replacement() {
        let generator = Generator::new(19);
        let coord = VoxelCoord::new(-33, 65, 31);
        let mut edits = EditMap::default();
        edits.insert_override(coord, Material::Snow);
        edits.insert_override(coord, Material::Basalt);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits.override_at(coord), Some(Material::Basalt));

        let mut chunk = generator.generate_chunk(coord.chunk());
        edits.apply_to_chunk(&mut chunk);
        let [x, y, z] = coord.local();
        assert_eq!(chunk.get(x, y, z), Material::Basalt);
        assert_eq!(edits.surface_sample(generator, coord.x, coord.z).0, coord.y);

        edits.set(
            generator,
            coord,
            generator.sample(coord.x, coord.y, coord.z),
        );
        assert!(edits.is_empty());
        assert!(edits.chunk_overrides.is_empty());
        assert!(edits.column_overrides.is_empty());
    }
}
