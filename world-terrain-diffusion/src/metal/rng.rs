const PCG_MULTIPLIER: u64 = 6_364_136_223_846_793_005;
const PCG_INCREMENT: u64 = 1_442_695_040_888_963_407;
const SPATIAL_SEED_MULTIPLIER: u64 = 0x9e37_79b9;

#[derive(Clone, Copy, Debug)]
struct PortableNormal {
    state: u64,
}

impl PortableNormal {
    const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u32(&mut self) -> u32 {
        self.state = self
            .state
            .wrapping_mul(PCG_MULTIPLIER)
            .wrapping_add(PCG_INCREMENT);
        let xor_shifted = (((self.state >> 18) ^ self.state) >> 27) as u32;
        xor_shifted.rotate_right((self.state >> 59) as u32)
    }

    fn fill(&mut self, output: &mut [f32]) {
        let inverse_u32_range = 1.0 / 4_294_967_296.0;
        let mut index = 0;
        while index < output.len() {
            let first = 2.0 * (f64::from(self.next_u32()) + 1.0) * inverse_u32_range - 1.0;
            let second = 2.0 * (f64::from(self.next_u32()) + 1.0) * inverse_u32_range - 1.0;
            let radius_squared = first * first + second * second;
            if !(0.0..1.0).contains(&radius_squared) || radius_squared == 0.0 {
                continue;
            }
            let scale = (-2.0 * radius_squared.ln() / radius_squared).sqrt();
            output[index] = (first * scale) as f32;
            index += 1;
            if index < output.len() {
                output[index] = (second * scale) as f32;
                index += 1;
            }
        }
    }
}

fn spatial_seed(base: u64, tile_y: i64, tile_x: i64) -> u64 {
    base.wrapping_mul(SPATIAL_SEED_MULTIPLIER)
        .wrapping_add(u64::from(tile_y as u32))
        .wrapping_mul(SPATIAL_SEED_MULTIPLIER)
        .wrapping_add(u64::from(tile_x as u32))
}

/// Exact Rust port of Terrain Diffusion's coordinate-keyed PCG/Marsaglia Gaussian field.
pub fn gaussian_patch(
    base_seed: u64,
    origin: [i32; 2],
    shape: [usize; 2],
    channels: usize,
    noise_tile: [usize; 2],
) -> Vec<f32> {
    let [height, width] = shape;
    let [tile_height, tile_width] = noise_tile;
    let mut output = vec![0.0; channels * height * width];
    assert!(
        tile_height > 0 && tile_width > 0,
        "noise tiles must be non-empty"
    );
    if height == 0 || width == 0 || channels == 0 {
        return output;
    }
    let origin_y = i64::from(origin[0]);
    let origin_x = i64::from(origin[1]);
    let height_i64 = height as i64;
    let width_i64 = width as i64;
    let tile_height_i64 = tile_height as i64;
    let tile_width_i64 = tile_width as i64;
    let first_tile_y = origin_y.div_euclid(tile_height_i64);
    let final_tile_y = (origin_y + height_i64 - 1).div_euclid(tile_height_i64);
    let first_tile_x = origin_x.div_euclid(tile_width_i64);
    let final_tile_x = (origin_x + width_i64 - 1).div_euclid(tile_width_i64);
    for tile_y in first_tile_y..=final_tile_y {
        let tile_origin_y = tile_y * tile_height_i64;
        for tile_x in first_tile_x..=final_tile_x {
            let tile_origin_x = tile_x * tile_width_i64;
            let mut tile = vec![0.0; channels * tile_height * tile_width];
            PortableNormal::new(spatial_seed(base_seed, tile_y, tile_x)).fill(&mut tile);
            let overlap_y0 = origin_y.max(tile_origin_y);
            let overlap_y1 = (origin_y + height_i64).min(tile_origin_y + tile_height_i64);
            let overlap_x0 = origin_x.max(tile_origin_x);
            let overlap_x1 = (origin_x + width_i64).min(tile_origin_x + tile_width_i64);
            for channel in 0..channels {
                for world_y in overlap_y0..overlap_y1 {
                    for world_x in overlap_x0..overlap_x1 {
                        let output_y = (world_y - origin_y) as usize;
                        let output_x = (world_x - origin_x) as usize;
                        let tile_local_y = (world_y - tile_origin_y) as usize;
                        let tile_local_x = (world_x - tile_origin_x) as usize;
                        output[channel * height * width + output_y * width + output_x] = tile
                            [channel * tile_height * tile_width
                                + tile_local_y * tile_width
                                + tile_local_x];
                    }
                }
            }
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_normal_matches_published_reference_values() {
        let mut values = [0.0; 8];
        PortableNormal::new(0x5eed_cafe).fill(&mut values);
        let expected = [
            0.819_231_7,
            0.822_242_9,
            -1.141_384_2,
            -1.754_104_3,
            -0.138_199_69,
            -0.137_597_43,
            0.014_115_555,
            -0.241_322_79,
        ];
        for (actual, expected) in values.into_iter().zip(expected) {
            assert!((actual - expected).abs() < 1.0e-6, "{actual} != {expected}");
        }
    }

    #[test]
    fn overlapping_spatial_patches_are_identical_in_their_overlap() {
        let first = gaussian_patch(7, [-17, 23], [31, 29], 3, [16, 16]);
        let second = gaussian_patch(7, [-3, 37], [11, 9], 3, [16, 16]);
        for channel in 0..3 {
            for y in 0..11 {
                for x in 0..9 {
                    assert_eq!(
                        first[channel * 31 * 29 + (y + 14) * 29 + x + 14],
                        second[channel * 11 * 9 + y * 9 + x]
                    );
                }
            }
        }
    }

    #[test]
    fn patches_can_reach_the_i32_coordinate_boundaries() {
        for origin in [[i32::MIN, i32::MIN], [i32::MAX - 7, i32::MAX - 7]] {
            let patch = gaussian_patch(7, origin, [8, 8], 1, [8, 8]);
            assert_eq!(patch.len(), 64);
            assert!(patch.iter().all(|value| value.is_finite()));
        }
    }
}
