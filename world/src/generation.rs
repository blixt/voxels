use crate::{CHUNK_EDGE, Chunk, ChunkCoord, Material};

/// Generator version is part of world identity. Changing terrain semantics requires incrementing it.
pub const GENERATOR_VERSION: u32 = 3;

#[derive(Clone, Copy, Debug)]
pub struct Generator {
    seed: u64,
}

#[derive(Clone, Copy)]
struct ColumnProfile {
    height: i32,
    moisture: f32,
    temperature: f32,
    local_biome: f32,
}

impl Generator {
    pub const fn new(seed: u64) -> Self {
        Self { seed }
    }

    pub const fn seed(self) -> u64 {
        self.seed
    }

    pub fn generate_chunk(self, coord: ChunkCoord) -> Chunk {
        let mut chunk = Chunk::empty(coord);
        let origin = coord.world_origin();
        let columns = (0..CHUNK_EDGE)
            .flat_map(|z| {
                (0..CHUNK_EDGE)
                    .map(move |x| self.column_profile(origin[0] + x as i32, origin[2] + z as i32))
            })
            .collect::<Vec<_>>();
        for y in 0..CHUNK_EDGE {
            for z in 0..CHUNK_EDGE {
                for x in 0..CHUNK_EDGE {
                    let world = [
                        origin[0] + x as i32,
                        origin[1] + y as i32,
                        origin[2] + z as i32,
                    ];
                    chunk.set(
                        x,
                        y,
                        z,
                        self.sample_terrain_with_profile(
                            world[0],
                            world[1],
                            world[2],
                            columns[x + z * CHUNK_EDGE],
                        ),
                    );
                }
            }
        }
        self.decorate_trees(&mut chunk);
        chunk
    }

    pub fn sample(self, x: i32, y: i32, z: i32) -> Material {
        let terrain = self.sample_terrain(x, y, z);
        if terrain.is_solid() {
            terrain
        } else {
            self.tree_material(x, y, z).unwrap_or(Material::Air)
        }
    }

    fn sample_terrain(self, x: i32, y: i32, z: i32) -> Material {
        self.sample_terrain_with_profile(x, y, z, self.column_profile(x, z))
    }

    fn sample_terrain_with_profile(
        self,
        x: i32,
        y: i32,
        z: i32,
        profile: ColumnProfile,
    ) -> Material {
        if y < -16 {
            return Material::Basalt;
        }
        let height = profile.height;
        if y > height {
            return Material::Air;
        }

        // Broad caves only affect material well under the surface, leaving a stable walking crust.
        if y + 4 < height {
            let cave = self.value_3d(x, y, z, 90, 0x9e37);
            let tunnel = self.value_3d(x, y * 2, z, 150, 0xb529);
            if cave > 0.73 && tunnel > 0.43 {
                return Material::Air;
            }
        }

        let moisture = profile.moisture;
        let temperature = profile.temperature - y as f32 * 0.006;
        let local_biome = profile.local_biome;
        let depth = height - y;
        if depth == 0 {
            if height < 11 {
                Material::Sand
            } else if temperature < 0.31 {
                Material::Snow
            } else if moisture < 0.23 {
                if local_biome > 0.58 {
                    Material::RedSand
                } else {
                    Material::Clay
                }
            } else if moisture > 0.68 && local_biome > 0.48 {
                Material::Moss
            } else if local_biome < 0.18 {
                Material::Limestone
            } else {
                Material::Grass
            }
        } else if depth < 4 {
            if height < 11 {
                Material::Sand
            } else if local_biome < 0.18 {
                Material::Limestone
            } else {
                Material::Dirt
            }
        } else if y < 2 && self.value_3d(x, y, z, 40, 0x55ad) > 0.66 {
            Material::Basalt
        } else {
            Material::Stone
        }
    }

    fn column_profile(self, x: i32, z: i32) -> ColumnProfile {
        ColumnProfile {
            height: self.surface_height(x, z),
            moisture: self.value_2d(x, z, 1_200, 0x4f1b),
            temperature: self.value_2d(x, z, 1_900, 0xa18d),
            local_biome: self.value_2d(x, z, 260, 0x8b21),
        }
    }

    pub fn surface_height(self, x: i32, z: i32) -> i32 {
        let continental = self.fractal_2d(x, z, 1_800, 4, 0x71a9);
        let hills = self.fractal_2d(x, z, 420, 3, 0x2d31);
        let detail = self.fractal_2d(x, z, 64, 2, 0x51f7);
        let ridge = 1.0 - (self.value_2d(x, z, 620, 0xc43b) * 2.0 - 1.0).abs();
        (8.0 + continental * 27.0 + hills * 11.0 + detail * 4.0 + ridge * ridge * 10.0) as i32
    }

    fn tree_material(self, x: i32, y: i32, z: i32) -> Option<Material> {
        const CELL: i32 = 96;
        let cell_x = x.div_euclid(CELL);
        let cell_z = z.div_euclid(CELL);
        for dz_cell in -1..=1 {
            for dx_cell in -1..=1 {
                let candidate_x = cell_x + dx_cell;
                let candidate_z = cell_z + dz_cell;
                let (anchor_x, anchor_z, tree_height) = self.tree_anchor(candidate_x, candidate_z);
                let dx = x - anchor_x;
                let dz = z - anchor_z;
                if dx.abs() > 9 || dz.abs() > 9 {
                    continue;
                }
                if !self.tree_grows_at(anchor_x, anchor_z) {
                    continue;
                }
                let ground = self.surface_height(anchor_x, anchor_z);
                let trunk_top = ground + tree_height;
                if y > ground && y <= trunk_top && dx.abs() <= 1 && dz.abs() <= 1 {
                    return Some(Material::Wood);
                }
                let crown_y = trunk_top - 3;
                let dy = y - crown_y;
                if (-7..=5).contains(&dy) {
                    let horizontal_radius = 9 - dy.abs() / 2;
                    let distance_squared = dx * dx + dz * dz + (dy * dy) / 2;
                    if dx.abs() <= horizontal_radius
                        && dz.abs() <= horizontal_radius
                        && distance_squared <= 78
                    {
                        return Some(Material::Leaves);
                    }
                }
            }
        }
        None
    }

    fn decorate_trees(self, chunk: &mut Chunk) {
        const CELL: i32 = 96;
        const CROWN_RADIUS: i32 = 9;
        let origin = chunk.coord().world_origin();
        let max_x = origin[0] + CHUNK_EDGE as i32 - 1;
        let max_z = origin[2] + CHUNK_EDGE as i32 - 1;
        let min_cell_x = (origin[0] - CROWN_RADIUS).div_euclid(CELL);
        let max_cell_x = (max_x + CROWN_RADIUS).div_euclid(CELL);
        let min_cell_z = (origin[2] - CROWN_RADIUS).div_euclid(CELL);
        let max_cell_z = (max_z + CROWN_RADIUS).div_euclid(CELL);
        for cell_z in min_cell_z..=max_cell_z {
            for cell_x in min_cell_x..=max_cell_x {
                let (anchor_x, anchor_z, tree_height) = self.tree_anchor(cell_x, cell_z);
                if !self.tree_grows_at(anchor_x, anchor_z) {
                    continue;
                }
                let ground = self.surface_height(anchor_x, anchor_z);
                let trunk_top = ground + tree_height;
                let min_y = (ground + 1).max(origin[1]);
                let max_y = (trunk_top + 5).min(origin[1] + CHUNK_EDGE as i32 - 1);
                for world_y in min_y..=max_y {
                    for world_z in (anchor_z - CROWN_RADIUS).max(origin[2])
                        ..=(anchor_z + CROWN_RADIUS).min(max_z)
                    {
                        for world_x in (anchor_x - CROWN_RADIUS).max(origin[0])
                            ..=(anchor_x + CROWN_RADIUS).min(max_x)
                        {
                            let local = [
                                (world_x - origin[0]) as usize,
                                (world_y - origin[1]) as usize,
                                (world_z - origin[2]) as usize,
                            ];
                            if chunk.get(local[0], local[1], local[2]).is_solid() {
                                continue;
                            }
                            let dx = world_x - anchor_x;
                            let dz = world_z - anchor_z;
                            let material = if world_y <= trunk_top && dx.abs() <= 1 && dz.abs() <= 1
                            {
                                Some(Material::Wood)
                            } else {
                                let dy = world_y - (trunk_top - 3);
                                let radius = 9 - dy.abs() / 2;
                                let distance_squared = dx * dx + dz * dz + (dy * dy) / 2;
                                ((-7..=5).contains(&dy)
                                    && dx.abs() <= radius
                                    && dz.abs() <= radius
                                    && distance_squared <= 78)
                                    .then_some(Material::Leaves)
                            };
                            if let Some(material) = material {
                                chunk.set(local[0], local[1], local[2], material);
                            }
                        }
                    }
                }
            }
        }
    }

    fn tree_grows_at(self, x: i32, z: i32) -> bool {
        self.value_2d(x, z, 1_200, 0x4f1b) >= 0.28 && self.value_2d(x, z, 1_900, 0xa18d) >= 0.30
    }

    fn tree_anchor(self, cell_x: i32, cell_z: i32) -> (i32, i32, i32) {
        const CELL: i32 = 96;
        let hash = self.hash(cell_x, 0, cell_z, 0x7a11_5eed);
        let x_offset = 12 + (hash & 63) as i32;
        let z_offset = 12 + ((hash >> 8) & 63) as i32;
        let height = 25 + ((hash >> 16) & 15) as i32;
        (cell_x * CELL + x_offset, cell_z * CELL + z_offset, height)
    }

    fn fractal_2d(self, x: i32, z: i32, scale: i32, octaves: u32, salt: u64) -> f32 {
        let mut amplitude = 1.0;
        let mut total = 0.0;
        let mut weight = 0.0;
        let mut octave_scale = scale;
        for octave in 0..octaves {
            total += self.value_2d(x, z, octave_scale.max(2), salt + u64::from(octave)) * amplitude;
            weight += amplitude;
            amplitude *= 0.5;
            octave_scale /= 2;
        }
        total / weight
    }

    fn value_2d(self, x: i32, z: i32, scale: i32, salt: u64) -> f32 {
        let x0 = x.div_euclid(scale);
        let z0 = z.div_euclid(scale);
        let tx = smooth(x.rem_euclid(scale) as f32 / scale as f32);
        let tz = smooth(z.rem_euclid(scale) as f32 / scale as f32);
        let a = self.hash_unit(x0, 0, z0, salt);
        let b = self.hash_unit(x0 + 1, 0, z0, salt);
        let c = self.hash_unit(x0, 0, z0 + 1, salt);
        let d = self.hash_unit(x0 + 1, 0, z0 + 1, salt);
        lerp(lerp(a, b, tx), lerp(c, d, tx), tz)
    }

    fn value_3d(self, x: i32, y: i32, z: i32, scale: i32, salt: u64) -> f32 {
        let x0 = x.div_euclid(scale);
        let y0 = y.div_euclid(scale);
        let z0 = z.div_euclid(scale);
        let tx = smooth(x.rem_euclid(scale) as f32 / scale as f32);
        let ty = smooth(y.rem_euclid(scale) as f32 / scale as f32);
        let tz = smooth(z.rem_euclid(scale) as f32 / scale as f32);
        let plane = |dy: i32| {
            let a = self.hash_unit(x0, y0 + dy, z0, salt);
            let b = self.hash_unit(x0 + 1, y0 + dy, z0, salt);
            let c = self.hash_unit(x0, y0 + dy, z0 + 1, salt);
            let d = self.hash_unit(x0 + 1, y0 + dy, z0 + 1, salt);
            lerp(lerp(a, b, tx), lerp(c, d, tx), tz)
        };
        lerp(plane(0), plane(1), ty)
    }

    fn hash_unit(self, x: i32, y: i32, z: i32, salt: u64) -> f32 {
        (self.hash(x, y, z, salt) >> 40) as f32 / ((1u32 << 24) - 1) as f32
    }

    fn hash(self, x: i32, y: i32, z: i32, salt: u64) -> u64 {
        let mut value = self.seed ^ salt;
        value ^= (x as i64 as u64).wrapping_mul(0x9e37_79b1_85eb_ca87);
        value ^= (y as i64 as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f);
        value ^= (z as i64 as u64).wrapping_mul(0x1656_67b1_9e37_79f9);
        value ^= value >> 30;
        value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value ^= value >> 27;
        value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^= value >> 31;
        value
    }
}

fn smooth(value: f32) -> f32 {
    value * value * (3.0 - 2.0 * value)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_is_deterministic_across_chunk_boundaries() {
        let generator = Generator::new(0x5eed);
        let left = generator.generate_chunk(ChunkCoord::new(-1, 0, 0));
        let right = generator.generate_chunk(ChunkCoord::new(0, 0, 0));
        assert_eq!(left.get(31, 17, 9), generator.sample(-1, 17, 9));
        assert_eq!(right.get(0, 17, 9), generator.sample(0, 17, 9));
        assert_ne!(left.voxels(), right.voxels());
    }

    #[test]
    fn cached_column_generation_matches_random_access_sampling() {
        let generator = Generator::new(0x5eed);
        let coord = ChunkCoord::new(-2, 0, 3);
        let chunk = generator.generate_chunk(coord);
        let origin = coord.world_origin();
        for y in [0, 5, 15, 31] {
            for z in [0, 11, 23, 31] {
                for x in [0, 7, 15, 31] {
                    assert_eq!(
                        chunk.get(x, y, z),
                        generator.sample(
                            origin[0] + x as i32,
                            origin[1] + y as i32,
                            origin[2] + z as i32,
                        )
                    );
                }
            }
        }
    }

    #[test]
    fn different_seeds_produce_different_chunks() {
        let coord = ChunkCoord::new(1, 0, -2);
        assert_ne!(
            Generator::new(1).generate_chunk(coord),
            Generator::new(2).generate_chunk(coord)
        );
    }

    #[test]
    fn macro_terrain_varies_over_real_world_distances() {
        let generator = Generator::new(0x5eed);
        let nearby_delta = (generator.surface_height(0, 0) - generator.surface_height(10, 0)).abs();
        let distant_delta =
            (generator.surface_height(0, 0) - generator.surface_height(2_000, 0)).abs();
        assert!(
            nearby_delta <= 8,
            "one metre should not cross a macro biome"
        );
        assert!(distant_delta >= nearby_delta);
    }

    #[test]
    fn trees_are_deterministic_editable_voxels() {
        let generator = Generator::new(0x5eed);
        let mut found = None;
        for cell_z in -4..=4 {
            for cell_x in -4..=4 {
                let (x, z, height) = generator.tree_anchor(cell_x, cell_z);
                let ground = generator.surface_height(x, z);
                if generator.sample(x, ground + 1, z) == Material::Wood {
                    found = Some((x, ground, z, height));
                    break;
                }
            }
        }
        assert!(
            found.is_some(),
            "seed should produce at least one nearby tree"
        );
        let Some((x, ground, z, height)) = found else {
            return;
        };
        assert_eq!(generator.sample(x, ground + 1, z), Material::Wood);
        assert_eq!(generator.sample(x, ground + height, z), Material::Wood);
        assert_eq!(
            generator.sample(x + 5, ground + height - 3, z),
            Material::Leaves
        );
    }
}
