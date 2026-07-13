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
    /// Known logical B-tree payload. This deliberately excludes allocator/node overhead while
    /// counting the authoritative row and both lookup-index rows retained for every edit.
    pub fn logical_bytes(&self) -> usize {
        size_of::<Self>()
            + self.overrides.len() * size_of::<(VoxelCoord, Material)>()
            + self.chunk_overrides.len()
                * size_of::<((i32, i32, i32), BTreeMap<[usize; 3], Material>)>()
            + self.overrides.len() * size_of::<([usize; 3], Material)>()
            + self.column_overrides.len() * size_of::<((i32, i32), BTreeMap<i32, Material>)>()
            + self.overrides.len() * size_of::<(i32, Material)>()
    }
    pub fn sample(&self, generator: Generator, coord: VoxelCoord) -> Material {
        self.resolve_generated(coord, generator.sample(coord.x, coord.y, coord.z))
    }

    pub fn set(&mut self, generator: Generator, coord: VoxelCoord, material: Material) {
        self.set_against_generated(coord, material, generator.sample(coord.x, coord.y, coord.z));
    }

    /// Resolves one authoritative source value against the sparse edit overlay. This is the
    /// transport-safe form: callers can supply a value from a chunk, halo, or bounded sample block
    /// without giving the edit journal a concrete generator.
    pub fn resolve_generated(&self, coord: VoxelCoord, generated: Material) -> Material {
        self.overrides.get(&coord).copied().unwrap_or(generated)
    }

    /// Stores only values that differ from an already obtained authoritative source value.
    pub fn set_against_generated(
        &mut self,
        coord: VoxelCoord,
        material: Material,
        generated: Material,
    ) {
        if generated == material {
            self.replace_override(coord, None);
        } else {
            self.replace_override(coord, Some(material));
        }
    }

    /// Used only when hydrating already validated durable rows.
    pub fn insert_override(&mut self, coord: VoxelCoord, material: Material) {
        self.replace_override(coord, Some(material));
    }

    /// Applies one already validated durable row/change notification. `None` means the row was
    /// removed and the generator is authoritative again.
    pub fn replace_durable_override(&mut self, coord: VoxelCoord, material: Option<Material>) {
        self.replace_override(coord, material);
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

    /// Captures pristine values for the sparse overrides that touch a generated chunk. Clients can
    /// retain these few values for edit comparison and exact reversion without keeping a second
    /// full pristine chunk cache or issuing a point source request later.
    pub fn source_values_for_overrides(&self, chunk: &Chunk) -> Vec<(VoxelCoord, Material)> {
        let coord = chunk.coord();
        let Some(overrides) = self.chunk_overrides.get(&(coord.x, coord.y, coord.z)) else {
            return Vec::new();
        };
        let origin = coord.world_origin();
        overrides
            .keys()
            .map(|local| {
                (
                    VoxelCoord::new(
                        origin[0] + local[0] as i32,
                        origin[1] + local[1] as i32,
                        origin[2] + local[2] as i32,
                    ),
                    chunk.get(local[0], local[1], local[2]),
                )
            })
            .collect()
    }

    /// Returns whether a disposable skyline proxy still represents pristine canonical feature
    /// voxels. The query walks only chunk-index buckets touched by the small analytic feature, so
    /// unrelated large edit journals do not affect surface-tile generation cost.
    pub fn skyline_feature_is_pristine(
        &self,
        generator: Generator,
        feature: SkylineFeature,
    ) -> bool {
        self.skyline_feature_is_pristine_with(feature, |coord| {
            generator.sample(coord.x, coord.y, coord.z)
        })
    }

    /// Source-agnostic pristine check over the bounded feature coordinates touched by edits.
    pub fn skyline_feature_is_pristine_with(
        &self,
        feature: SkylineFeature,
        mut generated_at: impl FnMut(VoxelCoord) -> Material,
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
                    for (&[local_x, local_y, local_z], &override_material) in overrides {
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
                        if generated_at(coord) == feature_material
                            && override_material != feature_material
                        {
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
        self.surface_sample_with(
            x,
            z,
            (generated.height, generated.material),
            i32::MIN,
            |coord| generator.sample(coord.x, coord.y, coord.z),
        )
    }

    /// Resolves one edited surface column from prepared source data and a bounded column sampler.
    /// `generated_min_y` is the inclusive floor of the prepared product, preventing an accidental
    /// scan outside the caller's authoritative data.
    pub fn surface_sample_with(
        &self,
        x: i32,
        z: i32,
        generated_surface: (i32, Material),
        generated_min_y: i32,
        mut generated_at: impl FnMut(VoxelCoord) -> Material,
    ) -> (i32, Material) {
        let Some(column) = self.column_overrides.get(&(x, z)) else {
            return generated_surface;
        };
        let highest_override = column
            .iter()
            .rev()
            .find(|(_, material)| material.is_collidable())
            .map(|(&y, _)| y);
        let mut y =
            highest_override.map_or(generated_surface.0, |value| value.max(generated_surface.0));
        loop {
            let coord = VoxelCoord::new(x, y, z);
            let material = self
                .override_at(coord)
                .unwrap_or_else(|| generated_at(coord));
            if material.is_collidable() || y <= generated_min_y {
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

    /// Chunks whose meshes can change after this voxel changes. The mesher samples a full one-voxel
    /// shell for face visibility and ambient occlusion, so edge and corner edits invalidate the
    /// Cartesian product of all touching chunk owners, not only face neighbors.
    pub fn affected_chunks(coord: VoxelCoord) -> Vec<ChunkCoord> {
        let edge = CHUNK_EDGE as i32;
        let local = [
            coord.x.rem_euclid(edge),
            coord.y.rem_euclid(edge),
            coord.z.rem_euclid(edge),
        ];
        let base = coord.chunk();
        let axis_offsets = local.map(|value| {
            if value == 0 {
                [0, -1]
            } else if value == edge - 1 {
                [0, 1]
            } else {
                [0, 0]
            }
        });
        let mut chunks = Vec::with_capacity(8);
        for x in axis_offsets[0] {
            for y in axis_offsets[1] {
                for z in axis_offsets[2] {
                    let Some(x) = base.x.checked_add(x) else {
                        continue;
                    };
                    let Some(y) = base.y.checked_add(y) else {
                        continue;
                    };
                    let Some(z) = base.z.checked_add(z) else {
                        continue;
                    };
                    let neighbor = ChunkCoord::new(x, y, z);
                    if neighbor.is_world_representable() && !chunks.contains(&neighbor) {
                        chunks.push(neighbor);
                    }
                }
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
    fn prepared_source_values_drive_sparse_comparison_and_resolution() {
        let coord = VoxelCoord::new(-4, 30, 9);
        let mut edits = EditMap::default();
        edits.set_against_generated(coord, Material::Stone, Material::Air);
        assert_eq!(
            edits.resolve_generated(coord, Material::Air),
            Material::Stone
        );
        edits.set_against_generated(coord, Material::Air, Material::Air);
        assert!(edits.is_empty());
    }

    #[test]
    fn pristine_chunk_values_are_captured_before_sparse_overrides_are_applied() {
        let generator = Generator::new(0x5eed_cafe);
        let coord = VoxelCoord::new(-33, 7, 65);
        let chunk = generator.generate_chunk(coord.chunk());
        let [x, y, z] = coord.local();
        let pristine = chunk.get(x, y, z);
        let replacement = if pristine == Material::Air {
            Material::Stone
        } else {
            Material::Air
        };
        let mut edits = EditMap::default();
        edits.insert_override(coord, replacement);

        assert_eq!(
            edits.source_values_for_overrides(&chunk),
            vec![(coord, pristine)]
        );
        let mut edited = chunk;
        edits.apply_to_chunk(&mut edited);
        assert_eq!(edited.get(x, y, z), replacement);
    }

    #[test]
    fn prepared_surface_sampling_is_lazy_and_stops_at_the_supplied_block_floor() {
        let x = 4;
        let z = -7;
        let removed_surface = VoxelCoord::new(x, 10, z);
        let mut edits = EditMap::default();
        edits.replace_durable_override(removed_surface, Some(Material::Air));
        let mut sampled = Vec::new();
        let resolved = edits.surface_sample_with(x, z, (10, Material::Stone), 9, |coord| {
            sampled.push(coord);
            assert_ne!(
                coord, removed_surface,
                "override must win before source lookup"
            );
            Material::Stone
        });
        assert_eq!(resolved, (9, Material::Stone));
        assert_eq!(sampled, vec![VoxelCoord::new(x, 9, z)]);
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
    fn corner_edit_invalidates_all_eight_chunks_sampled_for_ambient_occlusion() {
        let chunks = EditMap::affected_chunks(VoxelCoord::new(-1, 64, 31));
        assert_eq!(chunks.len(), 8);
        for x in [-1, 0] {
            for y in [1, 2] {
                for z in [0, 1] {
                    assert!(chunks.contains(&ChunkCoord::new(x, y, z)));
                }
            }
        }
    }

    #[test]
    fn world_boundary_edits_never_invalidate_unrepresentable_chunks() {
        for axis in 0..3 {
            for boundary in [i32::MIN, i32::MAX] {
                let mut voxel = [7, 7, 7];
                voxel[axis] = boundary;
                let chunks =
                    EditMap::affected_chunks(VoxelCoord::new(voxel[0], voxel[1], voxel[2]));

                assert_eq!(chunks.len(), 1);
                assert!(chunks[0].is_world_representable());
                assert_eq!(
                    chunks[0],
                    VoxelCoord::new(voxel[0], voxel[1], voxel[2]).chunk()
                );
            }
        }
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
    fn edited_column_surface_reaches_the_generated_floor() {
        let generator = Generator::new(11);
        let x = 7;
        let z = -9;
        let generated = generator.surface_height(x, z);
        let mut edits = EditMap::default();
        for y in -16..=generated {
            edits.set(generator, VoxelCoord::new(x, y, z), Material::Air);
        }

        assert_eq!(generator.sample(x, -17, z), Material::Basalt);
        assert_eq!(
            edits.surface_sample(generator, x, z),
            (-17, Material::Basalt)
        );
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

    #[test]
    fn committed_remote_override_can_be_inserted_and_removed() {
        let coord = VoxelCoord::new(7, 8, 9);
        let mut edits = EditMap::default();
        edits.replace_durable_override(coord, Some(Material::Clay));
        assert_eq!(edits.override_at(coord), Some(Material::Clay));
        edits.replace_durable_override(coord, None);
        assert_eq!(edits.override_at(coord), None);
        assert!(edits.chunk_overrides.is_empty());
        assert!(edits.column_overrides.is_empty());
    }

    #[test]
    fn redundant_durable_override_keeps_skyline_proxy_pristine() {
        let generator = Generator::new(0x5eed);
        let feature = generator
            .nearest_skyline_feature(0, 0, crate::SkylineFeatureKind::Broadleaf, 128)
            .expect("fixed catalog should contain a nearby broadleaf");
        let coord = VoxelCoord::new(feature.anchor[0], feature.anchor[1] + 1, feature.anchor[2]);
        let generated = generator.sample(coord.x, coord.y, coord.z);
        assert_eq!(feature.material_at(coord), Some(generated));

        let mut edits = EditMap::default();
        edits.insert_override(coord, generated);

        assert!(edits.skyline_feature_is_pristine(generator, feature));
        edits.insert_override(coord, Material::Air);
        assert!(!edits.skyline_feature_is_pristine(generator, feature));
    }

    #[test]
    fn logical_memory_tracks_rows_without_growing_on_replacement() {
        let coord = VoxelCoord::new(7, 8, 9);
        let mut edits = EditMap::default();
        let empty = edits.logical_bytes();
        edits.insert_override(coord, Material::Clay);
        let one_row = edits.logical_bytes();
        edits.insert_override(coord, Material::Basalt);
        assert_eq!(edits.logical_bytes(), one_row);
        assert!(one_row > empty);
        edits.replace_durable_override(coord, None);
        assert_eq!(edits.logical_bytes(), empty);
    }
}
