use serde::Deserialize;

const CHANNELS: usize = 5;
const QUANTILES: usize = 64;
const BASE_FREQUENCY: f32 = 0.05;
const FREQUENCY_MULTIPLIERS: [f32; CHANNELS] = [1.0; CHANNELS];
const OCTAVES: [usize; CHANNELS] = [4, 2, 4, 4, 4];
const PIPELINE_DATA: &str = include_str!("../../fixtures/pipeline-data.json");

#[derive(Clone, Debug, Deserialize)]
pub(super) struct SyntheticMapStats {
    n_quantiles: usize,
    noise_quantile_tables: Vec<Vec<f32>>,
    data_quantile_tables: Vec<Vec<f32>>,
    a_temp_std: f32,
    b_temp_std: f32,
    temp_std_p1: f32,
    temp_std_p99: f32,
}

impl SyntheticMapStats {
    pub(super) fn bundled() -> Result<Self, String> {
        let stats: Self = serde_json::from_str(PIPELINE_DATA)
            .map_err(|error| format!("bundled synthetic-map statistics are invalid: {error}"))?;
        stats.validate()?;
        Ok(stats)
    }

    fn validate(&self) -> Result<(), String> {
        if self.n_quantiles != QUANTILES
            || self.noise_quantile_tables.len() != CHANNELS
            || self.data_quantile_tables.len() != CHANNELS
        {
            return Err(format!(
                "synthetic-map statistics must contain {CHANNELS}x{QUANTILES} quantiles"
            ));
        }
        for (kind, tables) in [
            ("noise", &self.noise_quantile_tables),
            ("data", &self.data_quantile_tables),
        ] {
            for table in tables {
                if table.len() != QUANTILES
                    || table.iter().any(|value| !value.is_finite())
                    || table.windows(2).any(|pair| pair[0] >= pair[1])
                {
                    return Err(format!(
                        "each {kind} quantile table must contain {QUANTILES} finite increasing values"
                    ));
                }
            }
        }
        if [
            self.a_temp_std,
            self.b_temp_std,
            self.temp_std_p1,
            self.temp_std_p99,
        ]
        .iter()
        .any(|value| !value.is_finite())
            || self.temp_std_p1 >= self.temp_std_p99
        {
            return Err("synthetic-map climate correction constants are invalid".to_owned());
        }
        Ok(())
    }

    /// Reproduces the upstream five-channel ETOPO/WorldClim prior before model standardization.
    /// Coordinates are (row, column), while FastNoiseLite samples (column, row).
    pub(super) fn sample(&self, seed: u64, origin: [i32; 2], edge: usize) -> Vec<f32> {
        let pixels = edge * edge;
        let mut raw = vec![0.0_f32; CHANNELS * pixels];
        for channel in 0..CHANNELS {
            let noise = FastNoiseLitePerlinFbm {
                seed: (seed.wrapping_add(channel as u64 + 1) & 0x7fff_ffff) as i32,
                frequency: BASE_FREQUENCY * FREQUENCY_MULTIPLIERS[channel],
                octaves: OCTAVES[channel],
                lacunarity: 2.0,
                gain: 0.5,
            };
            for row in 0..edge {
                for column in 0..edge {
                    let noise_value = noise.sample(
                        origin[1].saturating_add(column as i32) as f32,
                        origin[0].saturating_add(row as i32) as f32,
                    );
                    raw[channel * pixels + row * edge + column] = interpolate_quantiles(
                        noise_value,
                        &self.noise_quantile_tables[channel],
                        &self.data_quantile_tables[channel],
                    );
                }
            }
        }

        for pixel in 0..pixels {
            let elevation = raw[pixel];
            let precipitation = raw[3 * pixels + pixel];
            let lapse_rate = (-6.5 + 0.0015 * precipitation).clamp(-9.8, -4.0) / 1_000.0;
            let temperature =
                (raw[pixels + pixel] + lapse_rate * elevation.max(0.0)).clamp(-10.0, 40.0);
            // Current upstream expands the colder half of the distribution around 20 C.
            let temperature = if temperature > 20.0 {
                temperature
            } else {
                (temperature - 20.0) * 1.25 + 20.0
            };
            raw[pixels + pixel] = temperature;

            let temperature_spread = raw[2 * pixels + pixel];
            let unit =
                (temperature_spread - self.temp_std_p1) / (self.temp_std_p99 - self.temp_std_p1);
            let baseline = self
                .temp_std_p1
                .max(-(self.a_temp_std * temperature + self.b_temp_std));
            raw[2 * pixels + pixel] = (unit * (self.temp_std_p99 - baseline)
                + baseline
                + self.a_temp_std * temperature
                + self.b_temp_std)
                .max(20.0);

            raw[4 * pixels + pixel] *= ((185.0 - 0.04111 * precipitation) / 185.0).max(0.0);
            raw[pixel] = elevation.signum() * elevation.abs().sqrt();
        }
        raw
    }
}

fn interpolate_quantiles(value: f32, source: &[f32], target: &[f32]) -> f32 {
    if value <= source[0] {
        return target[0];
    }
    let last = source.len() - 1;
    if value >= source[last] {
        return target[last];
    }
    let upper = source.partition_point(|candidate| *candidate <= value);
    let lower = upper - 1;
    let weight = (value - source[lower]) / (source[upper] - source[lower]);
    target[lower] + weight * (target[upper] - target[lower])
}

#[derive(Clone, Copy, Debug)]
struct FastNoiseLitePerlinFbm {
    seed: i32,
    frequency: f32,
    octaves: usize,
    lacunarity: f32,
    gain: f32,
}

impl FastNoiseLitePerlinFbm {
    fn sample(self, x: f32, y: f32) -> f32 {
        let mut x = x * self.frequency;
        let mut y = y * self.frequency;
        let gain = self.gain.abs();
        let mut amplitude_sum = 1.0;
        let mut amplitude = gain;
        for _ in 1..self.octaves {
            amplitude_sum += amplitude;
            amplitude *= gain;
        }
        let mut amplitude = amplitude_sum.recip();
        let mut sum = 0.0;
        let mut octave_seed = self.seed;
        for _ in 0..self.octaves {
            sum += single_perlin(octave_seed, x, y) * amplitude;
            octave_seed = octave_seed.wrapping_add(1);
            x *= self.lacunarity;
            y *= self.lacunarity;
            amplitude *= self.gain;
        }
        sum
    }
}

fn single_perlin(seed: i32, x: f32, y: f32) -> f32 {
    const PRIME_X: i32 = 501_125_321;
    const PRIME_Y: i32 = 1_136_930_381;
    let grid_x = fast_floor(x);
    let grid_y = fast_floor(y);
    let offset_x0 = x - grid_x as f32;
    let offset_y0 = y - grid_y as f32;
    let offset_x1 = offset_x0 - 1.0;
    let offset_y1 = offset_y0 - 1.0;
    let blend_x = quintic(offset_x0);
    let blend_y = quintic(offset_y0);
    let grid_x0 = grid_x.wrapping_mul(PRIME_X);
    let grid_y0 = grid_y.wrapping_mul(PRIME_Y);
    let grid_x1 = grid_x0.wrapping_add(PRIME_X);
    let grid_y1 = grid_y0.wrapping_add(PRIME_Y);
    let top = lerp(
        gradient(seed, grid_x0, grid_y0, offset_x0, offset_y0),
        gradient(seed, grid_x1, grid_y0, offset_x1, offset_y0),
        blend_x,
    );
    let bottom = lerp(
        gradient(seed, grid_x0, grid_y1, offset_x0, offset_y1),
        gradient(seed, grid_x1, grid_y1, offset_x1, offset_y1),
        blend_x,
    );
    lerp(top, bottom, blend_y) * 1.424_769
}

fn fast_floor(value: f32) -> i32 {
    if value >= 0.0 {
        value as i32
    } else {
        (value as i32).wrapping_sub(1)
    }
}

fn quintic(value: f32) -> f32 {
    value * value * value * (value * (value * 6.0 - 15.0) + 10.0)
}

fn lerp(left: f32, right: f32, weight: f32) -> f32 {
    left + weight * (right - left)
}

fn gradient(seed: i32, x: i32, y: i32, offset_x: f32, offset_y: f32) -> f32 {
    let mut hash = seed ^ x ^ y;
    hash = hash.wrapping_mul(0x27d4_eb2d);
    hash ^= hash >> 15;
    let index = ((hash & (127 << 1)) >> 1) as usize;
    let [gradient_x, gradient_y] = if index < 120 {
        GRADIENTS_24[index % 24]
    } else {
        GRADIENTS_LAST_8[index - 120]
    };
    offset_x * gradient_x + offset_y * gradient_y
}

const GRADIENTS_24: [[f32; 2]; 24] = [
    [0.130_526_19, 0.991_444_9],
    [0.382_683_43, 0.923_879_5],
    [0.608_761_4, 0.793_353_3],
    [0.793_353_3, 0.608_761_4],
    [0.923_879_5, 0.382_683_43],
    [0.991_444_9, 0.130_526_19],
    [0.991_444_9, -0.130_526_19],
    [0.923_879_5, -0.382_683_43],
    [0.793_353_3, -0.608_761_4],
    [0.608_761_4, -0.793_353_3],
    [0.382_683_43, -0.923_879_5],
    [0.130_526_19, -0.991_444_9],
    [-0.130_526_19, -0.991_444_9],
    [-0.382_683_43, -0.923_879_5],
    [-0.608_761_4, -0.793_353_3],
    [-0.793_353_3, -0.608_761_4],
    [-0.923_879_5, -0.382_683_43],
    [-0.991_444_9, -0.130_526_19],
    [-0.991_444_9, 0.130_526_19],
    [-0.923_879_5, 0.382_683_43],
    [-0.793_353_3, 0.608_761_4],
    [-0.608_761_4, 0.793_353_3],
    [-0.382_683_43, 0.923_879_5],
    [-0.130_526_19, 0.991_444_9],
];

const GRADIENTS_LAST_8: [[f32; 2]; 8] = [
    [0.382_683_43, 0.923_879_5],
    [0.923_879_5, 0.382_683_43],
    [0.923_879_5, -0.382_683_43],
    [0.382_683_43, -0.923_879_5],
    [-0.382_683_43, -0.923_879_5],
    [-0.923_879_5, -0.382_683_43],
    [-0.923_879_5, 0.382_683_43],
    [-0.382_683_43, 0.923_879_5],
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_statistics_are_complete() {
        SyntheticMapStats::bundled().expect("valid bundled statistics");
    }

    #[test]
    fn quantile_interpolation_clamps_and_interpolates() {
        assert_eq!(interpolate_quantiles(-2.0, &[-1.0, 1.0], &[2.0, 6.0]), 2.0);
        assert_eq!(interpolate_quantiles(0.0, &[-1.0, 1.0], &[2.0, 6.0]), 4.0);
        assert_eq!(interpolate_quantiles(2.0, &[-1.0, 1.0], &[2.0, 6.0]), 6.0);
    }

    #[test]
    fn fast_noise_lite_perlin_fbm_matches_reference_samples() {
        let noise = FastNoiseLitePerlinFbm {
            seed: 7,
            frequency: 0.05,
            octaves: 4,
            lacunarity: 2.0,
            gain: 0.5,
        };
        // Reference values come from FastNoiseLite Java 1.1.1, the implementation used upstream.
        for ((x, y), expected) in [
            ((0.0, 0.0), 0.0),
            ((1.0, 2.0), 0.000_707_746),
            ((-12.0, 34.0), -0.239_917_07),
            ((1_024.0, -512.0), 0.030_046_597),
        ] {
            let actual = noise.sample(x, y);
            assert!(
                (actual - expected).abs() < 1.0e-6,
                "FastNoiseLite mismatch at ({x}, {y}): expected {expected}, got {actual}"
            );
        }
    }

    #[test]
    fn synthetic_map_is_spatial_deterministic_and_climate_corrected() {
        let stats = SyntheticMapStats::bundled().expect("stats");
        let first = stats.sample(7, [-12, 34], 8);
        assert_eq!(first, stats.sample(7, [-12, 34], 8));
        assert_ne!(first, stats.sample(7, [-11, 34], 8));
        assert_eq!(first.len(), CHANNELS * 64);
        assert!(first.iter().all(|value| value.is_finite()));
        assert!(first[2 * 64..3 * 64].iter().all(|value| *value >= 20.0));
        assert!(first[4 * 64..5 * 64].iter().all(|value| *value >= 0.0));
    }
}
