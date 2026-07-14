mod model;
mod rng;
mod scheduler;

use crate::{
    MODEL_FILES, MODEL_REPOSITORY, MODEL_REVISION, TerrainDiffusionConfig, TerrainDiffusionError,
    TerrainPrecision, validate_model_root,
};
use candle_core::{DType, Device, Tensor};
use model::{EdmUnet, EdmUnetConfig};
use rng::gaussian_patch;
use scheduler::DpmSolver;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::Instant;
use voxels_world::{
    MACRO_FIELD_SCHEMA_VERSION, MAX_MACRO_BLOCK_SAMPLES, MAX_WORLD_PRODUCT_BATCH, MacroBlock,
    MacroBlockBatch, MacroBlockBatchResult, MacroTerrainSource, WorldSourceError,
    WorldSourceIdentity, WorldSourceIdentityHash,
};

const TILE_EDGE: usize = 64;
const COARSE_SAMPLE_CHANNELS: usize = 6;
const COARSE_CONDITIONING_CHANNELS: usize = 5;
const LATENT_CHANNELS: usize = 5;
const LATENT_TILE_EDGE: usize = 64;
const LATENT_COMPRESSION: usize = 8;
const SIGMA_DATA: f64 = 0.5;
const LOW_FREQUENCY_MEAN: f32 = -31.4;
const LOW_FREQUENCY_STD: f32 = 38.6;
const RESIDUAL_STD: f32 = 0.7;
const CONDITIONING_OUTPUT_CHANNELS: [usize; COARSE_CONDITIONING_CHANNELS] = [0, 2, 3, 4, 5];

#[derive(Debug, Deserialize)]
struct PipelineConfig {
    coarse_means: [f32; COARSE_SAMPLE_CHANNELS],
    coarse_stds: [f32; COARSE_SAMPLE_CHANNELS],
}

#[derive(Clone, Debug)]
pub struct CoarseTile {
    pub channels: usize,
    pub edge: usize,
    pub values: Vec<f32>,
    pub elapsed_seconds: f64,
}

#[derive(Clone, Debug)]
pub struct LatentTile {
    pub channels: usize,
    pub edge: usize,
    /// Global latent-grid origin used by the coordinate-keyed spatial RNG.
    pub origin: [i32; 2],
    /// Top-left coarse sample selected as this latent tile's physical context.
    pub coarse_patch_origin: [usize; 2],
    pub values: Vec<f32>,
    pub elapsed_seconds: f64,
}

#[derive(Clone, Debug)]
pub struct DetailTile {
    pub edge: usize,
    pub horizontal_resolution_metres: u32,
    pub elevation_metres: Vec<f32>,
    pub residual_sqrt_elevation: Vec<f32>,
    pub elapsed_seconds: f64,
}

#[derive(Clone, Debug)]
pub struct FullDetailTile {
    pub coarse: CoarseTile,
    pub latent: LatentTile,
    pub latent_patch_origin: [usize; 2],
    pub detail: DetailTile,
}

/// Finite native-Metal macro source produced by one complete model-chain experiment.
///
/// It is deliberately an owned canonical product: after generation it has no Candle or Metal
/// types, so the same source can cross the planned local-process protocol. Expanding this into an
/// infinite source requires overlap-aware stage caching, but does not change the trait boundary.
#[derive(Clone, Debug)]
pub struct TerrainDiffusionMacroTileSource {
    identity: WorldSourceIdentity,
    origin_voxels: [i32; 2],
    detail: DetailTile,
    temperature: f32,
    moisture: f32,
}

pub struct MetalTerrainDiffusion {
    device: Device,
    dtype: DType,
    coarse: Option<EdmUnet>,
    base: Option<EdmUnet>,
    decoder: Option<EdmUnet>,
    pipeline: PipelineConfig,
    config: TerrainDiffusionConfig,
}

impl MetalTerrainDiffusion {
    pub fn load_coarse(config: TerrainDiffusionConfig) -> Result<Self, TerrainDiffusionError> {
        Self::load(config, false, false)
    }

    pub fn load_base(config: TerrainDiffusionConfig) -> Result<Self, TerrainDiffusionError> {
        Self::load(config, true, false)
    }

    pub fn load_full(config: TerrainDiffusionConfig) -> Result<Self, TerrainDiffusionError> {
        Self::load(config, true, true)
    }

    fn load(
        config: TerrainDiffusionConfig,
        include_base: bool,
        include_decoder: bool,
    ) -> Result<Self, TerrainDiffusionError> {
        if !config.require_metal {
            return Err(TerrainDiffusionError::MetalUnavailable(
                "CPU fallback is forbidden for this provider".to_owned(),
            ));
        }
        let device = Device::new_metal(0)
            .map_err(|error| TerrainDiffusionError::MetalUnavailable(error.to_string()))?;
        let dtype = match config.precision {
            TerrainPrecision::Float32 => DType::F32,
            TerrainPrecision::Float16 => DType::F16,
        };
        let pipeline_path = config.model_root.join("config.json");
        let pipeline = std::fs::read(&pipeline_path)
            .map_err(|error| TerrainDiffusionError::InvalidModelFile {
                path: pipeline_path.clone(),
                reason: error.to_string(),
            })
            .and_then(|bytes| {
                serde_json::from_slice(&bytes).map_err(|error| {
                    TerrainDiffusionError::InvalidModelFile {
                        path: pipeline_path,
                        reason: error.to_string(),
                    }
                })
            })?;
        let coarse = load_model(&config.model_root, "coarse_model", dtype, &device)?;
        let base = if include_base {
            Some(load_model(
                &config.model_root,
                "base_model",
                dtype,
                &device,
            )?)
        } else {
            None
        };
        let decoder = if include_decoder {
            Some(load_model(
                &config.model_root,
                "decoder_model",
                dtype,
                &device,
            )?)
        } else {
            None
        };
        Ok(Self {
            device,
            dtype,
            coarse: Some(coarse),
            base,
            decoder,
            pipeline,
            config,
        })
    }

    pub fn device_description(&self) -> String {
        format!("{:?}", self.device)
    }

    pub fn precision(&self) -> TerrainPrecision {
        self.config.precision
    }

    pub fn source_identity(&self) -> Result<WorldSourceIdentity, TerrainDiffusionError> {
        self.config.source_identity()
    }

    /// Executes one raw coarse U-Net forward pass on Metal.
    ///
    /// This is intentionally exposed as the first numerical counterproof milestone. `input` is
    /// NCHW `[1, 11, 64, 64]`, and `conditionals` contains the five scalar coarse controls.
    pub fn coarse_forward(
        &self,
        input: &[f32],
        noise_label: f32,
        conditionals: [f32; 5],
    ) -> Result<CoarseTile, TerrainDiffusionError> {
        let expected = 11 * TILE_EDGE * TILE_EDGE;
        if input.len() != expected {
            return Err(TerrainDiffusionError::Inference(format!(
                "coarse input has {} values, expected {expected}",
                input.len()
            )));
        }
        let started = Instant::now();
        let input = Tensor::from_slice(input, (1, 11, TILE_EDGE, TILE_EDGE), &self.device)
            .and_then(|tensor| tensor.to_dtype(self.dtype))
            .map_err(|error| TerrainDiffusionError::Inference(error.to_string()))?;
        let noise = Tensor::new(&[noise_label], &self.device)
            .and_then(|tensor| tensor.to_dtype(self.dtype))
            .map_err(|error| TerrainDiffusionError::Inference(error.to_string()))?;
        let conditionals = conditionals
            .into_iter()
            .map(|value| {
                Tensor::new(&[value], &self.device).and_then(|tensor| tensor.to_dtype(self.dtype))
            })
            .collect::<candle_core::Result<Vec<_>>>()
            .map_err(|error| TerrainDiffusionError::Inference(error.to_string()))?;
        let output = self
            .coarse_model()?
            .forward(&input, &noise, &conditionals)
            .and_then(|tensor| tensor.to_dtype(DType::F32))
            .map_err(|error| TerrainDiffusionError::Inference(error.to_string()))?;
        self.device
            .synchronize()
            .map_err(|error| TerrainDiffusionError::Inference(error.to_string()))?;
        let values = output
            .flatten_all()
            .and_then(|tensor| tensor.to_vec1::<f32>())
            .map_err(|error| TerrainDiffusionError::Inference(error.to_string()))?;
        Ok(CoarseTile {
            channels: 6,
            edge: TILE_EDGE,
            values,
            elapsed_seconds: started.elapsed().as_secs_f64(),
        })
    }

    #[doc(hidden)]
    pub fn base_forward_conditioning_delta(&self) -> Result<f32, TerrainDiffusionError> {
        let input = Tensor::zeros(
            (1, LATENT_CHANNELS, LATENT_TILE_EDGE, LATENT_TILE_EDGE),
            self.dtype,
            &self.device,
        )
        .map_err(inference_error)?;
        let noise = Tensor::new(&[0.5_f32], &self.device)
            .and_then(|tensor| tensor.to_dtype(self.dtype))
            .map_err(inference_error)?;
        let baseline = Tensor::zeros((1, 58), self.dtype, &self.device).map_err(inference_error)?;
        let mut changed = vec![0.0_f32; 58];
        changed[..16].fill(1.0);
        let changed = Tensor::from_slice(&changed, (1, 58), &self.device)
            .and_then(|tensor| tensor.to_dtype(self.dtype))
            .map_err(inference_error)?;
        let baseline = self
            .base_model()?
            .forward(&input, &noise, &[baseline])
            .and_then(|tensor| tensor.to_dtype(DType::F32))
            .and_then(|tensor| tensor.flatten_all())
            .and_then(|tensor| tensor.to_vec1::<f32>())
            .map_err(inference_error)?;
        let changed = self
            .base_model()?
            .forward(&input, &noise, &[changed])
            .and_then(|tensor| tensor.to_dtype(DType::F32))
            .and_then(|tensor| tensor.flatten_all())
            .and_then(|tensor| tensor.to_vec1::<f32>())
            .map_err(inference_error)?;
        Ok(baseline
            .into_iter()
            .zip(changed)
            .map(|(left, right)| (left - right).abs())
            .fold(0.0_f32, f32::max))
    }

    /// Generates one deterministic 64x64 coarse-world tile using the published 20-step sampler.
    ///
    /// `standardized_conditioning` is NCHW-without-batch `[5, 64, 64]`, with channels ordered as
    /// elevation, temperature, temperature standard deviation, precipitation, and precipitation
    /// variability. Values must already be normalized with the pinned pipeline's training
    /// statistics. Supplying zeros asks the learned model to generate around the training means.
    pub fn generate_coarse_tile(
        &self,
        origin: [i32; 2],
        standardized_conditioning: &[f32],
        conditioning_snr: [f32; COARSE_CONDITIONING_CHANNELS],
    ) -> Result<CoarseTile, TerrainDiffusionError> {
        let expected = COARSE_CONDITIONING_CHANNELS * TILE_EDGE * TILE_EDGE;
        if standardized_conditioning.len() != expected {
            return Err(TerrainDiffusionError::Inference(format!(
                "coarse conditioning has {} values, expected {expected}",
                standardized_conditioning.len()
            )));
        }

        let started = Instant::now();
        let conditioning = Tensor::from_slice(
            standardized_conditioning,
            (1, COARSE_CONDITIONING_CHANNELS, TILE_EDGE, TILE_EDGE),
            &self.device,
        )
        .and_then(|tensor| tensor.to_dtype(self.dtype))
        .map_err(inference_error)?;
        let conditioning_noise = gaussian_patch(
            self.config.seed,
            origin,
            [TILE_EDGE, TILE_EDGE],
            COARSE_CONDITIONING_CHANNELS,
            [TILE_EDGE, TILE_EDGE],
        );
        let conditioning_noise = Tensor::from_slice(
            &conditioning_noise,
            (1, COARSE_CONDITIONING_CHANNELS, TILE_EDGE, TILE_EDGE),
            &self.device,
        )
        .and_then(|tensor| tensor.to_dtype(self.dtype))
        .map_err(inference_error)?;
        let conditioning_angles = conditioning_snr.map(f32::atan);
        let cosine = Tensor::from_slice(
            &conditioning_angles.map(f32::cos),
            (1, COARSE_CONDITIONING_CHANNELS, 1, 1),
            &self.device,
        )
        .and_then(|tensor| tensor.to_dtype(self.dtype))
        .map_err(inference_error)?;
        let sine = Tensor::from_slice(
            &conditioning_angles.map(f32::sin),
            (1, COARSE_CONDITIONING_CHANNELS, 1, 1),
            &self.device,
        )
        .and_then(|tensor| tensor.to_dtype(self.dtype))
        .map_err(inference_error)?;
        let conditioning = conditioning
            .broadcast_mul(&cosine)
            .and_then(|left| left + conditioning_noise.broadcast_mul(&sine)?)
            .map_err(inference_error)?;
        let scalar_conditioning = conditioning_snr
            .map(|snr| (snr / 8.0).ln())
            .into_iter()
            .map(|value| {
                Tensor::new(&[value], &self.device).and_then(|tensor| tensor.to_dtype(self.dtype))
            })
            .collect::<candle_core::Result<Vec<_>>>()
            .map_err(inference_error)?;

        let mut scheduler = DpmSolver::new(20);
        let sample_noise = gaussian_patch(
            self.config.seed.wrapping_add(1),
            origin,
            [TILE_EDGE, TILE_EDGE],
            COARSE_SAMPLE_CHANNELS,
            [TILE_EDGE, TILE_EDGE],
        );
        let mut sample = Tensor::from_slice(
            &sample_noise,
            (1, COARSE_SAMPLE_CHANNELS, TILE_EDGE, TILE_EDGE),
            &self.device,
        )
        .and_then(|tensor| tensor.to_dtype(self.dtype))
        .and_then(|tensor| tensor.affine(scheduler.sigmas()[0], 0.0))
        .map_err(inference_error)?;
        for step in 0..20 {
            let scaled = scheduler
                .scaled_input(&sample, step)
                .map_err(inference_error)?;
            let model_input =
                Tensor::cat(&[scaled, conditioning.clone()], 1).map_err(inference_error)?;
            let noise_label = Tensor::new(&[scheduler.noise_label(step)], &self.device)
                .and_then(|tensor| tensor.to_dtype(self.dtype))
                .map_err(inference_error)?;
            let model_output = self
                .coarse_model()?
                .forward(&model_input, &noise_label, &scalar_conditioning)
                .map_err(inference_error)?;
            sample = scheduler
                .step(&model_output, &sample, step)
                .map_err(inference_error)?;
        }
        sample = sample
            .affine(1.0 / SIGMA_DATA, 0.0)
            .and_then(|tensor| tensor.to_dtype(DType::F32))
            .map_err(inference_error)?;
        self.device.synchronize().map_err(inference_error)?;
        let mut values = sample
            .flatten_all()
            .and_then(|tensor| tensor.to_vec1::<f32>())
            .map_err(inference_error)?;
        let pixels = TILE_EDGE * TILE_EDGE;
        for channel in 0..COARSE_SAMPLE_CHANNELS {
            for value in &mut values[channel * pixels..(channel + 1) * pixels] {
                *value = *value * self.pipeline.coarse_stds[channel]
                    + self.pipeline.coarse_means[channel];
            }
        }
        for pixel in 0..pixels {
            values[pixels + pixel] = values[pixel] - values[pixels + pixel];
        }
        Ok(CoarseTile {
            channels: COARSE_SAMPLE_CHANNELS,
            edge: TILE_EDGE,
            values,
            elapsed_seconds: started.elapsed().as_secs_f64(),
        })
    }

    /// Runs the two published base consistency passes and returns a 240 m latent tile.
    ///
    /// `coarse_patch` is channel-major `[7, 4, 4]`: the six physical coarse channels followed by
    /// the overlap weight channel. This is the same product passed between the upstream infinite
    /// tensor stages, but represented as an owned Rust slice.
    pub fn generate_latent_tile(
        &self,
        origin: [i32; 2],
        coarse_patch: &[f32],
    ) -> Result<LatentTile, TerrainDiffusionError> {
        let expected = 7 * 4 * 4;
        if coarse_patch.len() != expected {
            return Err(TerrainDiffusionError::Inference(format!(
                "base conditioning has {} values, expected {expected}",
                coarse_patch.len()
            )));
        }
        let started = Instant::now();
        let conditioning = base_conditioning(coarse_patch, self.config.seed, origin)?;
        let conditioning = Tensor::from_slice(&conditioning, (1, 58), &self.device)
            .and_then(|tensor| tensor.to_dtype(self.dtype))
            .map_err(inference_error)?;
        let model = self.base_model()?;
        let mut sample = Tensor::zeros(
            (1, LATENT_CHANNELS, LATENT_TILE_EDGE, LATENT_TILE_EDGE),
            self.dtype,
            &self.device,
        )
        .map_err(inference_error)?;
        for (noise_seed_offset, sigma) in [(5_819_u64, 80.0_f32), (5_820_u64, 0.35_f32)] {
            sample = self.consistency_pass(
                model,
                &sample,
                std::slice::from_ref(&conditioning),
                origin,
                LATENT_TILE_EDGE,
                LATENT_CHANNELS,
                self.config.seed.wrapping_add(noise_seed_offset),
                sigma,
            )?;
        }
        let values = sample
            .affine(1.0 / SIGMA_DATA, 0.0)
            .and_then(|tensor| tensor.to_dtype(DType::F32))
            .and_then(|tensor| tensor.flatten_all())
            .and_then(|tensor| tensor.to_vec1::<f32>())
            .map_err(inference_error)?;
        self.device.synchronize().map_err(inference_error)?;
        Ok(LatentTile {
            channels: LATENT_CHANNELS,
            edge: LATENT_TILE_EDGE,
            origin,
            coarse_patch_origin: [0, 0],
            values,
            elapsed_seconds: started.elapsed().as_secs_f64(),
        })
    }

    /// Decodes a latent patch to the model's native 30 m terrain grid.
    ///
    /// The input is channel-major `[5, edge/8, edge/8]` in the base model's normalized latent
    /// units. The first four channels drive the learned decoder; channel five supplies the
    /// low-frequency elevation band used to reconstruct signed-square-root elevation.
    pub fn decode_detail_tile(
        &self,
        origin: [i32; 2],
        edge: usize,
        latent_patch: &[f32],
    ) -> Result<DetailTile, TerrainDiffusionError> {
        if edge < LATENT_COMPRESSION * 2 || !edge.is_multiple_of(LATENT_COMPRESSION) {
            return Err(TerrainDiffusionError::Inference(format!(
                "decoder edge {edge} must be at least {} and a multiple of {LATENT_COMPRESSION}",
                LATENT_COMPRESSION * 2
            )));
        }
        let latent_edge = edge / LATENT_COMPRESSION;
        let expected = LATENT_CHANNELS * latent_edge * latent_edge;
        if latent_patch.len() != expected {
            return Err(TerrainDiffusionError::Inference(format!(
                "decoder latent patch has {} values, expected {expected}",
                latent_patch.len()
            )));
        }
        let started = Instant::now();
        let latent_pixels = latent_edge * latent_edge;
        let learned_latents = Tensor::from_slice(
            &latent_patch[..4 * latent_pixels],
            (1, 4, latent_edge, latent_edge),
            &self.device,
        )
        .and_then(|tensor| tensor.to_dtype(self.dtype))
        .and_then(|tensor| tensor.upsample_nearest2d(edge, edge))
        .map_err(inference_error)?;
        let sample =
            Tensor::zeros((1, 1, edge, edge), self.dtype, &self.device).map_err(inference_error)?;
        let sample = self.consistency_pass_with_image_conditioning(
            self.decoder_model()?,
            &sample,
            &learned_latents,
            origin,
            edge,
            self.config.seed.wrapping_add(5_819),
            80.0,
        )?;
        let residual_sqrt_elevation = sample
            .affine(1.0 / SIGMA_DATA, 0.0)
            .and_then(|tensor| tensor.to_dtype(DType::F32))
            .and_then(|tensor| tensor.flatten_all())
            .and_then(|tensor| tensor.to_vec1::<f32>())
            .map_err(inference_error)?;
        self.device.synchronize().map_err(inference_error)?;

        let elevation_metres = reconstruct_elevation(
            &residual_sqrt_elevation,
            &latent_patch[4 * latent_pixels..],
            latent_edge,
            edge,
        );
        Ok(DetailTile {
            edge,
            horizontal_resolution_metres: 30,
            elevation_metres,
            residual_sqrt_elevation,
            elapsed_seconds: started.elapsed().as_secs_f64(),
        })
    }

    /// Exercises the complete published coarse -> base -> decoder model chain for one 128px tile.
    ///
    /// The finite provider uses a coordinate-keyed synthetic continent and climate field, matching
    /// the upstream pipeline's role for synthetic Perlin conditioning. Learned stages still own
    /// the 30 m terrain; this raster supplies only the coarse geographic intent.
    pub fn generate_full_detail_tile(
        &self,
        origin: [i32; 2],
    ) -> Result<FullDetailTile, TerrainDiffusionError> {
        let (coarse, latent) = self.generate_coarse_and_latent(origin)?;
        let detail_edge = 128;
        let latent_edge = detail_edge / LATENT_COMPRESSION;
        let latent_pixels = LATENT_TILE_EDGE * LATENT_TILE_EDGE;
        let [patch_z, patch_x] = patch_around_maximum(
            &latent.values[4 * latent_pixels..5 * latent_pixels],
            LATENT_TILE_EDGE,
            latent_edge,
        );
        let mut latent_patch = vec![0.0_f32; LATENT_CHANNELS * latent_edge * latent_edge];
        for channel in 0..LATENT_CHANNELS {
            for z in 0..latent_edge {
                for x in 0..latent_edge {
                    latent_patch[channel * latent_edge * latent_edge + z * latent_edge + x] =
                        latent.values[channel * LATENT_TILE_EDGE * LATENT_TILE_EDGE
                            + (patch_z + z) * LATENT_TILE_EDGE
                            + patch_x
                            + x];
                }
            }
        }
        let detail_origin = [
            latent.origin[0]
                .saturating_add(patch_z as i32)
                .saturating_mul(LATENT_COMPRESSION as i32),
            latent.origin[1]
                .saturating_add(patch_x as i32)
                .saturating_mul(LATENT_COMPRESSION as i32),
        ];
        let detail = self.decode_detail_tile(detail_origin, detail_edge, &latent_patch)?;
        Ok(FullDetailTile {
            coarse,
            latent,
            latent_patch_origin: [patch_z, patch_x],
            detail,
        })
    }

    pub fn generate_coarse_and_latent(
        &self,
        origin: [i32; 2],
    ) -> Result<(CoarseTile, LatentTile), TerrainDiffusionError> {
        let conditioning = synthetic_coarse_conditioning(
            self.config.seed,
            origin,
            &self.pipeline.coarse_means,
            &self.pipeline.coarse_stds,
        );
        let coarse =
            self.generate_coarse_tile(origin, &conditioning, [0.05, 0.5, 0.5, 0.5, 0.5])?;
        let [patch_z, patch_x] = highest_land_patch(&coarse);
        let mut coarse_patch = vec![1.0_f32; 7 * 4 * 4];
        for channel in 0..COARSE_SAMPLE_CHANNELS {
            for z in 0..4 {
                for x in 0..4 {
                    coarse_patch[channel * 16 + z * 4 + x] = coarse.values
                        [channel * TILE_EDGE * TILE_EDGE + (patch_z + z) * TILE_EDGE + patch_x + x];
                }
            }
        }
        // The upstream base graph requests a 4x4 coarse context at latent tile index - 1.
        // Preserve that coordinate relation so independently requested spatial noise overlaps.
        let latent_origin = [
            origin[0]
                .saturating_add(patch_z as i32)
                .saturating_add(1)
                .saturating_mul(32),
            origin[1]
                .saturating_add(patch_x as i32)
                .saturating_add(1)
                .saturating_mul(32),
        ];
        let mut latent = self.generate_latent_tile(latent_origin, &coarse_patch)?;
        latent.coarse_patch_origin = [patch_z, patch_x];
        Ok((coarse, latent))
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the arguments mirror one consistency stage"
    )]
    fn consistency_pass(
        &self,
        model: &EdmUnet,
        sample: &Tensor,
        conditional_inputs: &[Tensor],
        origin: [i32; 2],
        edge: usize,
        channels: usize,
        noise_seed: u64,
        sigma: f32,
    ) -> Result<Tensor, TerrainDiffusionError> {
        let noise = gaussian_patch(noise_seed, origin, [edge, edge], channels, [edge, edge]);
        let noise = Tensor::from_slice(&noise, (1, channels, edge, edge), &self.device)
            .and_then(|tensor| tensor.to_dtype(self.dtype))
            .and_then(|tensor| tensor.affine(SIGMA_DATA, 0.0))
            .map_err(inference_error)?;
        let angle = (sigma / SIGMA_DATA as f32).atan();
        let noisy = linear_combination(
            sample,
            f64::from(angle.cos()),
            &noise,
            f64::from(angle.sin()),
        )
        .map_err(inference_error)?;
        let model_input = noisy
            .affine(1.0 / SIGMA_DATA, 0.0)
            .map_err(inference_error)?;
        let noise_label = Tensor::new(&[angle], &self.device)
            .and_then(|tensor| tensor.to_dtype(self.dtype))
            .map_err(inference_error)?;
        let prediction = model
            .forward(&model_input, &noise_label, conditional_inputs)
            .map_err(inference_error)?;
        linear_combination(
            &noisy,
            f64::from(angle.cos()),
            &prediction,
            f64::from(angle.sin()) * SIGMA_DATA,
        )
        .map_err(inference_error)
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the arguments mirror one decoder stage"
    )]
    fn consistency_pass_with_image_conditioning(
        &self,
        model: &EdmUnet,
        sample: &Tensor,
        image_conditioning: &Tensor,
        origin: [i32; 2],
        edge: usize,
        noise_seed: u64,
        sigma: f32,
    ) -> Result<Tensor, TerrainDiffusionError> {
        let noise = gaussian_patch(noise_seed, origin, [edge, edge], 1, [edge, edge]);
        let noise = Tensor::from_slice(&noise, (1, 1, edge, edge), &self.device)
            .and_then(|tensor| tensor.to_dtype(self.dtype))
            .and_then(|tensor| tensor.affine(SIGMA_DATA, 0.0))
            .map_err(inference_error)?;
        let angle = (sigma / SIGMA_DATA as f32).atan();
        let noisy = linear_combination(
            sample,
            f64::from(angle.cos()),
            &noise,
            f64::from(angle.sin()),
        )
        .map_err(inference_error)?;
        let scaled = noisy
            .affine(1.0 / SIGMA_DATA, 0.0)
            .map_err(inference_error)?;
        let model_input =
            Tensor::cat(&[scaled, image_conditioning.clone()], 1).map_err(inference_error)?;
        let noise_label = Tensor::new(&[angle], &self.device)
            .and_then(|tensor| tensor.to_dtype(self.dtype))
            .map_err(inference_error)?;
        let prediction = model
            .forward(&model_input, &noise_label, &[])
            .map_err(inference_error)?;
        linear_combination(
            &noisy,
            f64::from(angle.cos()),
            &prediction,
            f64::from(angle.sin()) * SIGMA_DATA,
        )
        .map_err(inference_error)
    }

    fn coarse_model(&self) -> Result<&EdmUnet, TerrainDiffusionError> {
        self.coarse.as_ref().ok_or_else(|| {
            TerrainDiffusionError::Inference("coarse model was not loaded".to_owned())
        })
    }

    fn base_model(&self) -> Result<&EdmUnet, TerrainDiffusionError> {
        self.base.as_ref().ok_or_else(|| {
            TerrainDiffusionError::Inference(
                "base model was not loaded; construct the runtime with load_full".to_owned(),
            )
        })
    }

    fn decoder_model(&self) -> Result<&EdmUnet, TerrainDiffusionError> {
        self.decoder.as_ref().ok_or_else(|| {
            TerrainDiffusionError::Inference(
                "decoder model was not loaded; construct the runtime with load_full".to_owned(),
            )
        })
    }
}

impl TerrainDiffusionMacroTileSource {
    pub fn generate(runtime: &MetalTerrainDiffusion) -> Result<Self, TerrainDiffusionError> {
        let generated = runtime.generate_full_detail_tile(runtime.config.model_origin)?;
        let pixels = generated.coarse.edge * generated.coarse.edge;
        let [coarse_z, coarse_x] = generated.latent.coarse_patch_origin;
        let coarse_index = coarse_z * generated.coarse.edge + coarse_x;
        let temperature_celsius = generated.coarse.values[2 * pixels + coarse_index];
        let precipitation = generated.coarse.values[4 * pixels + coarse_index];
        Ok(Self {
            identity: runtime.source_identity()?,
            origin_voxels: runtime.config.world_origin_voxels,
            detail: generated.detail,
            temperature: ((temperature_celsius + 50.0) / 100.0).clamp(0.0, 1.0),
            moisture: (precipitation / 3_000.0).clamp(0.0, 1.0),
        })
    }

    pub fn detail(&self) -> &DetailTile {
        &self.detail
    }

    fn source_identity_hash(&self) -> WorldSourceIdentityHash {
        self.identity.identity_hash()
    }

    fn sample(&self, world_x: i64, world_z: i64) -> Option<(f32, f32)> {
        const VOXELS_PER_MODEL_PIXEL: i64 = 300;
        let local_x = world_x - i64::from(self.origin_voxels[0]);
        let local_z = world_z - i64::from(self.origin_voxels[1]);
        let extent = self.detail.edge as i64 * VOXELS_PER_MODEL_PIXEL;
        if self.detail.edge == 0
            || self.detail.elevation_metres.len() != self.detail.edge * self.detail.edge
            || local_x < 0
            || local_z < 0
            || local_x >= extent
            || local_z >= extent
        {
            return None;
        }
        let x = usize::try_from(local_x / VOXELS_PER_MODEL_PIXEL).ok()?;
        let z = usize::try_from(local_z / VOXELS_PER_MODEL_PIXEL).ok()?;
        let fraction_x = (local_x % VOXELS_PER_MODEL_PIXEL) as f32 / VOXELS_PER_MODEL_PIXEL as f32;
        let fraction_z = (local_z % VOXELS_PER_MODEL_PIXEL) as f32 / VOXELS_PER_MODEL_PIXEL as f32;
        let right = (x + 1).min(self.detail.edge - 1);
        let bottom = (z + 1).min(self.detail.edge - 1);
        let elevation = |sample_x: usize, sample_z: usize| {
            self.detail.elevation_metres[sample_z * self.detail.edge + sample_x]
        };
        let top_left = elevation(x, z);
        let top_right = elevation(right, z);
        let bottom_left = elevation(x, bottom);
        let bottom_right = elevation(right, bottom);
        let top = top_left + (top_right - top_left) * fraction_x;
        let bottom = bottom_left + (bottom_right - bottom_left) * fraction_x;
        let interpolated = top + (bottom - top) * fraction_z;
        let dx_top = top_right - top_left;
        let dx_bottom = bottom_right - bottom_left;
        let dx = dx_top + (dx_bottom - dx_top) * fraction_z;
        let dz_left = bottom_left - top_left;
        let dz_right = bottom_right - top_right;
        let dz = dz_left + (dz_right - dz_left) * fraction_x;
        let ridge = ((dx.abs() + dz.abs()) / 1_000.0).clamp(0.0, 1.0);
        Some((interpolated * 10.0, ridge))
    }
}

impl MacroTerrainSource for TerrainDiffusionMacroTileSource {
    fn identity(&self) -> &WorldSourceIdentity {
        &self.identity
    }

    fn request_blocks(
        &self,
        request: MacroBlockBatch,
    ) -> Result<MacroBlockBatchResult, WorldSourceError> {
        if request.requests.len() > MAX_WORLD_PRODUCT_BATCH {
            return Err(WorldSourceError::BatchTooLarge);
        }
        let mut blocks = Vec::with_capacity(request.requests.len());
        for request in request.requests {
            if request.sample_shape.contains(&0) || request.stride_voxels == 0 {
                return Err(WorldSourceError::EmptyMacroBlock);
            }
            let sample_count = usize::try_from(request.sample_shape[0])
                .ok()
                .and_then(|width| {
                    usize::try_from(request.sample_shape[1])
                        .ok()
                        .and_then(|depth| width.checked_mul(depth))
                })
                .ok_or(WorldSourceError::MacroBlockTooLarge)?;
            if sample_count > MAX_MACRO_BLOCK_SAMPLES {
                return Err(WorldSourceError::MacroBlockTooLarge);
            }
            let mut elevation_voxels = Vec::with_capacity(sample_count);
            let mut temperature = Vec::with_capacity(sample_count);
            let mut moisture = Vec::with_capacity(sample_count);
            let mut ridge = Vec::with_capacity(sample_count);
            let mut validity = Vec::with_capacity(sample_count);
            for z in 0..request.sample_shape[1] {
                for x in 0..request.sample_shape[0] {
                    let world_x = i64::from(request.origin[0])
                        + i64::from(x) * i64::from(request.stride_voxels);
                    let world_z = i64::from(request.origin[1])
                        + i64::from(z) * i64::from(request.stride_voxels);
                    if let Some((elevation, ridge_value)) = self.sample(world_x, world_z) {
                        elevation_voxels.push(elevation);
                        temperature.push(self.temperature);
                        moisture.push(self.moisture);
                        ridge.push(ridge_value);
                        validity.push(true);
                    } else {
                        elevation_voxels.push(0.0);
                        temperature.push(0.0);
                        moisture.push(0.0);
                        ridge.push(0.0);
                        validity.push(false);
                    }
                }
            }
            blocks.push(MacroBlock {
                schema_version: MACRO_FIELD_SCHEMA_VERSION,
                request,
                coordinate_transform: self.identity.macro_coordinate_transform,
                elevation_voxels,
                temperature,
                moisture,
                ridge,
                validity,
            });
        }
        Ok(MacroBlockBatchResult {
            source_identity_hash: self.source_identity_hash(),
            blocks,
        })
    }
}

fn load_model(
    root: &Path,
    folder: &str,
    dtype: DType,
    device: &Device,
) -> Result<EdmUnet, TerrainDiffusionError> {
    let model_dir = root.join(folder);
    let config_path = model_dir.join("config.json");
    let model_config = EdmUnetConfig::from_path(&config_path).map_err(|reason| {
        TerrainDiffusionError::InvalidModelFile {
            path: config_path,
            reason,
        }
    })?;
    EdmUnet::load(
        model_config,
        &model_dir.join("diffusion_pytorch_model.safetensors"),
        dtype,
        device,
    )
    .map_err(inference_error)
}

fn inference_error(error: candle_core::Error) -> TerrainDiffusionError {
    TerrainDiffusionError::Inference(error.to_string())
}

fn linear_combination(
    first: &Tensor,
    first_weight: f64,
    second: &Tensor,
    second_weight: f64,
) -> candle_core::Result<Tensor> {
    first.affine(first_weight, 0.0)? + second.affine(second_weight, 0.0)?
}

fn synthetic_coarse_conditioning(
    seed: u64,
    origin: [i32; 2],
    means: &[f32; COARSE_SAMPLE_CHANNELS],
    stds: &[f32; COARSE_SAMPLE_CHANNELS],
) -> Vec<f32> {
    let pixels = TILE_EDGE * TILE_EDGE;
    let mut output = vec![0.0_f32; COARSE_CONDITIONING_CHANNELS * pixels];
    for z in 0..TILE_EDGE {
        for x in 0..TILE_EDGE {
            let world_x = f64::from(origin[1]) + x as f64;
            let world_z = f64::from(origin[0]) + z as f64;
            let continental = fractal_value_noise(seed ^ 0x34ae_91d2, world_x, world_z, 28.0, 4);
            let ridge_noise =
                fractal_value_noise(seed ^ 0xc13f_a9a9, world_x + 173.0, world_z - 89.0, 11.0, 3);
            let ridges = (1.0 - ridge_noise.abs()).clamp(0.0, 1.0).powi(3);
            let land = smoothstep(-0.45, 0.55, continental);
            let local_relief =
                fractal_value_noise(seed ^ 0x7f4a_7c15, world_x - 41.0, world_z + 227.0, 6.0, 2);
            let elevation_metres = -650.0
                + (continental + 1.0) * 700.0
                + ridges * land * 2_200.0
                + local_relief * land * 180.0;
            let signed_root_elevation = elevation_metres.signum() * elevation_metres.abs().sqrt();

            let temperature_noise =
                fractal_value_noise(seed ^ 0x9e37_79b9, world_x + 311.0, world_z + 97.0, 42.0, 2);
            let precipitation_noise = fractal_value_noise(
                seed ^ 0x6a09_e667,
                world_x - 199.0,
                world_z - 313.0,
                24.0,
                4,
            );
            let seasonality_noise =
                fractal_value_noise(seed ^ 0xbb67_ae85, world_x + 61.0, world_z - 151.0, 30.0, 3);
            let temperature_celsius = (21.0 + temperature_noise * 11.0
                - elevation_metres.max(0.0) * 0.0065)
                .clamp(-15.0, 38.0);
            let temperature_seasonality =
                (260.0 + seasonality_noise.abs() * 320.0).clamp(20.0, 900.0);
            let precipitation = (1_050.0 + precipitation_noise * 900.0 + ridges * land * 450.0)
                .clamp(50.0, 3_500.0);
            let precipitation_variability =
                (58.0 - precipitation_noise * 24.0 + seasonality_noise * 12.0).clamp(5.0, 120.0);
            let raw = [
                signed_root_elevation,
                temperature_celsius,
                temperature_seasonality,
                precipitation,
                precipitation_variability,
            ];
            let pixel = z * TILE_EDGE + x;
            for (output_channel, value) in raw.into_iter().enumerate() {
                let mean_index = CONDITIONING_OUTPUT_CHANNELS[output_channel];
                output[output_channel * pixels + pixel] =
                    (value as f32 - means[mean_index]) / stds[mean_index];
            }
        }
    }
    output
}

fn fractal_value_noise(seed: u64, x: f64, z: f64, base_period: f64, octaves: usize) -> f64 {
    let mut sum = 0.0;
    let mut amplitude = 1.0;
    let mut amplitude_sum = 0.0;
    let mut period = base_period;
    for octave in 0..octaves {
        sum +=
            value_noise(seed.wrapping_add(octave as u64 * 0x9e37_79b9), x, z, period) * amplitude;
        amplitude_sum += amplitude;
        amplitude *= 0.5;
        period *= 0.5;
    }
    sum / amplitude_sum
}

fn value_noise(seed: u64, x: f64, z: f64, period: f64) -> f64 {
    let scaled_x = x / period;
    let scaled_z = z / period;
    let x0 = scaled_x.floor() as i64;
    let z0 = scaled_z.floor() as i64;
    let x_weight = smootherstep(scaled_x - x0 as f64);
    let z_weight = smootherstep(scaled_z - z0 as f64);
    let top = lerp(
        lattice_noise(seed, x0, z0),
        lattice_noise(seed, x0 + 1, z0),
        x_weight,
    );
    let bottom = lerp(
        lattice_noise(seed, x0, z0 + 1),
        lattice_noise(seed, x0 + 1, z0 + 1),
        x_weight,
    );
    lerp(top, bottom, z_weight)
}

fn lattice_noise(seed: u64, x: i64, z: i64) -> f64 {
    let mut value = seed
        ^ (x as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ (z as u64).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^= value >> 31;
    let unit = (value >> 11) as f64 * (1.0 / ((1_u64 << 53) as f64));
    unit * 2.0 - 1.0
}

fn smootherstep(value: f64) -> f64 {
    value * value * value * (value * (value * 6.0 - 15.0) + 10.0)
}

fn smoothstep(minimum: f64, maximum: f64, value: f64) -> f64 {
    let normalized = ((value - minimum) / (maximum - minimum)).clamp(0.0, 1.0);
    normalized * normalized * (3.0 - 2.0 * normalized)
}

fn lerp(left: f64, right: f64, weight: f64) -> f64 {
    left + (right - left) * weight
}

fn highest_land_patch(coarse: &CoarseTile) -> [usize; 2] {
    highest_average_patch(&coarse.values[..coarse.edge * coarse.edge], coarse.edge, 4)
}

fn highest_average_patch(values: &[f32], edge: usize, patch_edge: usize) -> [usize; 2] {
    debug_assert!(patch_edge > 0 && patch_edge <= edge);
    debug_assert_eq!(values.len(), edge * edge);
    let mut best_origin = [0, 0];
    let mut best_sum = f32::NEG_INFINITY;
    for z in 0..=edge - patch_edge {
        for x in 0..=edge - patch_edge {
            let mut sum = 0.0;
            for patch_z in 0..patch_edge {
                for patch_x in 0..patch_edge {
                    sum += values[(z + patch_z) * edge + x + patch_x];
                }
            }
            if sum > best_sum {
                best_sum = sum;
                best_origin = [z, x];
            }
        }
    }
    best_origin
}

fn patch_around_maximum(values: &[f32], edge: usize, patch_edge: usize) -> [usize; 2] {
    debug_assert!(patch_edge > 0 && patch_edge <= edge);
    debug_assert_eq!(values.len(), edge * edge);
    let maximum_index = values
        .iter()
        .enumerate()
        .max_by(|(_, left), (_, right)| left.total_cmp(right))
        .map_or(0, |(index, _)| index);
    let maximum = [maximum_index / edge, maximum_index % edge];
    [
        maximum[0]
            .saturating_sub(patch_edge / 2)
            .min(edge - patch_edge),
        maximum[1]
            .saturating_sub(patch_edge / 2)
            .min(edge - patch_edge),
    ]
}

fn base_conditioning(
    coarse_patch: &[f32],
    seed: u64,
    origin: [i32; 2],
) -> Result<Vec<f32>, TerrainDiffusionError> {
    const MEANS: [f32; 7] = [14.99, 11.65, 15.87, 619.26, 833.12, 69.40, 0.66];
    const STDS: [f32; 7] = [21.72, 21.78, 10.40, 452.29, 738.09, 34.59, 0.47];
    let mut normalized = [[0.0_f32; 16]; 7];
    for pixel in 0..16 {
        let weight = coarse_patch[6 * 16 + pixel];
        if !weight.is_finite() || weight.abs() < f32::EPSILON {
            return Err(TerrainDiffusionError::Inference(
                "coarse overlap weight is zero or non-finite".to_owned(),
            ));
        }
        for channel in 0..6 {
            normalized[channel][pixel] =
                (coarse_patch[channel * 16 + pixel] / weight - MEANS[channel]) / STDS[channel];
        }
        normalized[6][pixel] = (1.0 - MEANS[6]) / STDS[6];
    }
    for channel in 0..2 {
        for value in &mut normalized[channel] {
            if !value.is_finite() {
                *value = MEANS[channel];
            }
        }
    }

    let mut climate = [0.0_f32; 4];
    for (output, channel) in climate.iter_mut().zip(2..6) {
        *output = [5, 6, 9, 10]
            .into_iter()
            .map(|pixel| normalized[channel][pixel])
            .sum::<f32>()
            * 0.25;
    }
    let non_finite = climate
        .iter()
        .enumerate()
        .filter_map(|(index, value)| (!value.is_finite()).then_some(index))
        .collect::<Vec<_>>();
    if !non_finite.is_empty() {
        let tile_offset = i64::from(origin[0].div_euclid(32))
            .wrapping_mul(65_536)
            .wrapping_add(i64::from(origin[1].div_euclid(32))) as u64;
        let replacement = gaussian_patch(
            seed.wrapping_add(9_999).wrapping_add(tile_offset),
            [0, 0],
            [1, non_finite.len()],
            1,
            [1, non_finite.len()],
        );
        for (index, value) in non_finite.into_iter().zip(replacement) {
            climate[index] = value;
        }
    }

    let groups: [&[f32]; 6] = [
        &normalized[0],
        &normalized[1],
        &climate,
        &normalized[6],
        &[0.0; 5],
        &[-3.0_f32.sqrt()],
    ];
    let total = groups.iter().map(|group| group.len()).sum::<usize>() as f32;
    let group_count = groups.len() as f32;
    let mut output = Vec::with_capacity(total as usize);
    for group in groups {
        let scale = (total / (group_count * group.len() as f32)).sqrt();
        output.extend(group.iter().map(|value| value * scale));
    }
    debug_assert_eq!(output.len(), 58);
    Ok(output)
}

fn bilinear_upsample(input: &[f32], input_edge: usize, output_edge: usize) -> Vec<f32> {
    let mut output = vec![0.0; output_edge * output_edge];
    let scale = input_edge as f32 / output_edge as f32;
    for output_y in 0..output_edge {
        let source_y = (output_y as f32 + 0.5) * scale - 0.5;
        let y0 = source_y.floor().max(0.0) as usize;
        let y1 = (y0 + 1).min(input_edge - 1);
        let y_weight = (source_y - source_y.floor()).clamp(0.0, 1.0);
        for output_x in 0..output_edge {
            let source_x = (output_x as f32 + 0.5) * scale - 0.5;
            let x0 = source_x.floor().max(0.0) as usize;
            let x1 = (x0 + 1).min(input_edge - 1);
            let x_weight = (source_x - source_x.floor()).clamp(0.0, 1.0);
            let top = input[y0 * input_edge + x0] * (1.0 - x_weight)
                + input[y0 * input_edge + x1] * x_weight;
            let bottom = input[y1 * input_edge + x0] * (1.0 - x_weight)
                + input[y1 * input_edge + x1] * x_weight;
            output[output_y * output_edge + output_x] = top * (1.0 - y_weight) + bottom * y_weight;
        }
    }
    output
}

fn reconstruct_elevation(
    residual_normalized: &[f32],
    low_frequency_normalized: &[f32],
    low_edge: usize,
    high_edge: usize,
) -> Vec<f32> {
    let residual = residual_normalized
        .iter()
        .map(|value| value * RESIDUAL_STD)
        .collect::<Vec<_>>();
    let low_frequency = low_frequency_normalized
        .iter()
        .map(|value| value * LOW_FREQUENCY_STD + LOW_FREQUENCY_MEAN)
        .collect::<Vec<_>>();

    // This mirrors laplacian_denoise(..., sigma=5): decode with linearly extrapolated borders,
    // re-estimate the low band at latent resolution, Gaussian-filter it, then decode normally.
    let padded = linear_extrapolation_pad(&low_frequency, low_edge);
    let scale = high_edge / low_edge;
    let padded_up = bilinear_upsample(&padded, low_edge + 2, high_edge + 2 * scale);
    let mut decoded = vec![0.0; high_edge * high_edge];
    for y in 0..high_edge {
        for x in 0..high_edge {
            decoded[y * high_edge + x] = residual[y * high_edge + x]
                + padded_up[(y + scale) * (high_edge + 2 * scale) + x + scale];
        }
    }
    let reestimated_low = area_downsample(&decoded, high_edge, low_edge);
    let filtered_low = gaussian_blur(&reestimated_low, low_edge, 5.0, 5);
    let filtered_low = bilinear_upsample(&filtered_low, low_edge, high_edge);
    residual
        .iter()
        .zip(filtered_low)
        .map(|(residual, low_frequency)| {
            let signed_root = residual + low_frequency;
            signed_root.signum() * signed_root * signed_root
        })
        .collect()
}

fn linear_extrapolation_pad(input: &[f32], edge: usize) -> Vec<f32> {
    let padded_edge = edge + 2;
    let mut output = vec![0.0; padded_edge * padded_edge];
    for y in 0..edge {
        for x in 0..edge {
            output[(y + 1) * padded_edge + x + 1] = input[y * edge + x];
        }
    }
    for x in 0..edge {
        output[x + 1] = 2.0 * input[x] - input[edge + x];
        output[(padded_edge - 1) * padded_edge + x + 1] =
            2.0 * input[(edge - 1) * edge + x] - input[(edge - 2) * edge + x];
    }
    for y in 0..padded_edge {
        output[y * padded_edge] = 2.0 * output[y * padded_edge + 1] - output[y * padded_edge + 2];
        output[y * padded_edge + padded_edge - 1] = 2.0 * output[y * padded_edge + padded_edge - 2]
            - output[y * padded_edge + padded_edge - 3];
    }
    output
}

fn area_downsample(input: &[f32], input_edge: usize, output_edge: usize) -> Vec<f32> {
    let factor = input_edge / output_edge;
    let divisor = (factor * factor) as f32;
    let mut output = vec![0.0; output_edge * output_edge];
    for output_y in 0..output_edge {
        for output_x in 0..output_edge {
            let mut sum = 0.0;
            for y in 0..factor {
                for x in 0..factor {
                    sum += input[(output_y * factor + y) * input_edge + output_x * factor + x];
                }
            }
            output[output_y * output_edge + output_x] = sum / divisor;
        }
    }
    output
}

fn gaussian_blur(input: &[f32], edge: usize, sigma: f32, radius: usize) -> Vec<f32> {
    let mut kernel = (-(radius as i32)..=radius as i32)
        .map(|offset| (-(offset * offset) as f32 / (2.0 * sigma * sigma)).exp())
        .collect::<Vec<_>>();
    let sum = kernel.iter().sum::<f32>();
    for value in &mut kernel {
        *value /= sum;
    }
    let mut horizontal = vec![0.0; edge * edge];
    for y in 0..edge {
        for x in 0..edge {
            horizontal[y * edge + x] = kernel
                .iter()
                .enumerate()
                .map(|(kernel_x, weight)| {
                    let offset = kernel_x as i32 - radius as i32;
                    input[y * edge + reflect_index(x as i32 + offset, edge)] * weight
                })
                .sum();
        }
    }
    let mut output = vec![0.0; edge * edge];
    for y in 0..edge {
        for x in 0..edge {
            output[y * edge + x] = kernel
                .iter()
                .enumerate()
                .map(|(kernel_y, weight)| {
                    let offset = kernel_y as i32 - radius as i32;
                    horizontal[reflect_index(y as i32 + offset, edge) * edge + x] * weight
                })
                .sum();
        }
    }
    output
}

fn reflect_index(mut index: i32, length: usize) -> usize {
    let maximum = length as i32 - 1;
    while index < 0 || index > maximum {
        index = if index < 0 {
            -index
        } else {
            maximum * 2 - index
        };
    }
    index as usize
}

pub fn fetch_pinned_model(cache_root: &Path) -> Result<PathBuf, TerrainDiffusionError> {
    let root = cache_root.join(MODEL_REVISION);
    let client = reqwest::blocking::Client::builder()
        .user_agent("voxels-terrain-diffusion/0.0.0")
        .build()
        .map_err(|error| TerrainDiffusionError::ModelDownload(error.to_string()))?;
    for file in MODEL_FILES {
        let path = root.join(file);
        if path.is_file() {
            continue;
        }
        let parent = path.parent().ok_or_else(|| {
            TerrainDiffusionError::ModelDownload("model path has no parent".to_owned())
        })?;
        std::fs::create_dir_all(parent)
            .map_err(|error| TerrainDiffusionError::ModelDownload(error.to_string()))?;
        let partial = path.with_extension("partial");
        let url = format!(
            "https://huggingface.co/{MODEL_REPOSITORY}/resolve/{MODEL_REVISION}/{file}?download=true"
        );
        let mut last_error = None;
        for attempt in 0..4 {
            match download_to_partial(&client, &url, &partial) {
                Ok(()) => {
                    last_error = None;
                    break;
                }
                Err(error) => {
                    last_error = Some(error);
                    std::thread::sleep(std::time::Duration::from_millis(250 << attempt));
                }
            }
        }
        if let Some(error) = last_error {
            return Err(TerrainDiffusionError::ModelDownload(error));
        }
        std::fs::rename(&partial, &path)
            .map_err(|error| TerrainDiffusionError::ModelDownload(error.to_string()))?;
    }
    validate_model_root(&root)?;
    Ok(root)
}

fn download_to_partial(
    client: &reqwest::blocking::Client,
    url: &str,
    partial: &Path,
) -> Result<(), String> {
    let mut response = client
        .get(url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| error.to_string())?;
    let mut output = std::fs::File::create(partial).map_err(|error| error.to_string())?;
    std::io::copy(&mut response, &mut output).map_err(|error| error.to_string())?;
    output.sync_all().map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use voxels_world::{MacroBlockRequest, WorldProductPriority};

    #[test]
    fn base_conditioning_has_the_published_58_components() {
        let means = [14.99, 11.65, 15.87, 619.26, 833.12, 69.40];
        let mut patch = vec![1.0; 7 * 4 * 4];
        for (channel, mean) in means.into_iter().enumerate() {
            patch[channel * 16..(channel + 1) * 16].fill(mean);
        }
        let output = base_conditioning(&patch, 7, [0, 0]).expect("conditioning");
        assert_eq!(output.len(), 58);
        assert!(output.iter().all(|value| value.is_finite()));
    }

    #[test]
    fn bilinear_upsampling_preserves_a_constant_field() {
        assert_eq!(bilinear_upsample(&[3.5; 4], 2, 8), vec![3.5; 64]);
    }

    #[test]
    fn synthetic_coarse_conditioning_is_spatial_deterministic_and_multichannel() {
        let means = [-37.7, 1.14, 18.1, 332.8, 1_332.2, 52.7];
        let stds = [39.7, 1.77, 8.9, 321.8, 842.9, 31.1];
        let first = synthetic_coarse_conditioning(7, [-12, 34], &means, &stds);
        let repeated = synthetic_coarse_conditioning(7, [-12, 34], &means, &stds);
        let shifted = synthetic_coarse_conditioning(7, [-11, 34], &means, &stds);
        assert_eq!(first, repeated);
        assert_ne!(first, shifted);
        assert_eq!(
            first.len(),
            COARSE_CONDITIONING_CHANNELS * TILE_EDGE * TILE_EDGE
        );
        for channel in 0..COARSE_CONDITIONING_CHANNELS {
            let values =
                &first[channel * TILE_EDGE * TILE_EDGE..(channel + 1) * TILE_EDGE * TILE_EDGE];
            let minimum = values.iter().copied().fold(f32::INFINITY, f32::min);
            let maximum = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            assert!(
                maximum - minimum > 0.1,
                "conditioning channel {channel} collapsed"
            );
            assert!(values.iter().all(|value| value.is_finite()));
        }
    }

    #[test]
    fn representative_preview_selects_the_highest_average_window() {
        let mut values = vec![0.0; 16];
        values[10] = 2.0;
        values[11] = 2.0;
        values[14] = 2.0;
        values[15] = 2.0;
        assert_eq!(highest_average_patch(&values, 4, 2), [2, 2]);
    }

    #[test]
    fn representative_detail_preview_centres_the_highest_low_frequency_point() {
        let mut values = vec![0.0; 64];
        values[5 * 8 + 6] = 4.0;
        assert_eq!(patch_around_maximum(&values, 8, 4), [3, 4]);
    }

    #[test]
    fn finite_macro_source_marks_samples_outside_its_owned_tile_invalid() {
        let mut config = TerrainDiffusionConfig::pinned("model", 7);
        config.world_origin_voxels = [600, -300];
        let identity = config.source_identity().expect("identity");
        let source = TerrainDiffusionMacroTileSource {
            identity,
            origin_voxels: config.world_origin_voxels,
            detail: DetailTile {
                edge: 2,
                horizontal_resolution_metres: 30,
                elevation_metres: vec![1.0, 2.0, 3.0, 4.0],
                residual_sqrt_elevation: vec![0.0; 4],
                elapsed_seconds: 0.0,
            },
            temperature: 0.5,
            moisture: 0.25,
        };
        let result = source
            .request_blocks(MacroBlockBatch {
                priority: WorldProductPriority::Prefetch,
                requests: vec![
                    MacroBlockRequest {
                        origin: [600, -300],
                        sample_shape: [3, 1],
                        stride_voxels: 300,
                    },
                    MacroBlockRequest {
                        origin: [750, -150],
                        sample_shape: [1, 1],
                        stride_voxels: 1,
                    },
                ],
            })
            .expect("macro block");
        assert_eq!(result.blocks[0].elevation_voxels, vec![10.0, 20.0, 0.0]);
        assert_eq!(result.blocks[0].validity, vec![true, true, false]);
        assert_eq!(result.blocks[1].elevation_voxels, vec![25.0]);
        assert_eq!(result.blocks[1].ridge, vec![0.003]);
        assert_eq!(
            result.blocks[0].coordinate_transform.origin_voxels,
            [600, -300]
        );
    }
}
