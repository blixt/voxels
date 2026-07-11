use crate::{CHUNK_EDGE, Chunk, ChunkCoord, Material};

/// Generator version is part of world identity. Changing terrain semantics requires incrementing it.
pub const GENERATOR_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug)]
pub struct Generator {
    seed: u64,
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
        for y in 0..CHUNK_EDGE {
            for z in 0..CHUNK_EDGE {
                for x in 0..CHUNK_EDGE {
                    let world = [
                        origin[0] + x as i32,
                        origin[1] + y as i32,
                        origin[2] + z as i32,
                    ];
                    chunk.set(x, y, z, self.sample(world[0], world[1], world[2]));
                }
            }
        }
        chunk
    }

    pub fn sample(self, x: i32, y: i32, z: i32) -> Material {
        if y < -16 {
            return Material::Basalt;
        }
        let continental = self.fractal_2d(x, z, 96, 4, 0x71a9);
        let detail = self.fractal_2d(x, z, 24, 3, 0x2d31);
        let ridge = 1.0 - (self.value_2d(x, z, 54, 0xc43b) * 2.0 - 1.0).abs();
        let height = (10.0 + continental * 15.0 + detail * 5.0 + ridge * ridge * 9.0) as i32;
        if y > height {
            return Material::Air;
        }

        // Broad caves only affect material well under the surface, leaving a stable walking crust.
        if y + 4 < height {
            let cave = self.value_3d(x, y, z, 18, 0x9e37);
            let tunnel = self.value_3d(x, y * 2, z, 31, 0xb529);
            if cave > 0.73 && tunnel > 0.43 {
                return Material::Air;
            }
        }

        let moisture = self.value_2d(x, z, 110, 0x4f1b);
        let temperature = self.value_2d(x, z, 170, 0xa18d) - y as f32 * 0.008;
        let depth = height - y;
        if depth == 0 {
            if height < 12 {
                Material::Sand
            } else if temperature < 0.31 {
                Material::Snow
            } else if moisture < 0.25 {
                Material::Clay
            } else {
                Material::Grass
            }
        } else if depth < 4 {
            if height < 12 {
                Material::Sand
            } else {
                Material::Dirt
            }
        } else if y < 2 && self.value_3d(x, y, z, 40, 0x55ad) > 0.66 {
            Material::Basalt
        } else {
            Material::Stone
        }
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
        let mut value = self.seed ^ salt;
        value ^= (x as i64 as u64).wrapping_mul(0x9e37_79b1_85eb_ca87);
        value ^= (y as i64 as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f);
        value ^= (z as i64 as u64).wrapping_mul(0x1656_67b1_9e37_79f9);
        value ^= value >> 30;
        value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value ^= value >> 27;
        value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^= value >> 31;
        (value >> 40) as f32 / ((1u32 << 24) - 1) as f32
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
    fn different_seeds_produce_different_chunks() {
        let coord = ChunkCoord::new(1, 0, -2);
        assert_ne!(
            Generator::new(1).generate_chunk(coord),
            Generator::new(2).generate_chunk(coord)
        );
    }
}
