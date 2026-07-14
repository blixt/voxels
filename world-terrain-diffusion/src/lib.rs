//! Optional native Terrain Diffusion provider for Apple silicon.
//!
//! The normal workspace does not compile an ML runtime. Enabling the `metal` feature on macOS loads
//! the pinned Terrain Diffusion safetensors directly into Candle's Metal backend. There is no CPU,
//! Python, PyTorch, CUDA, Swift, or HTTP inference fallback.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use voxels_world::{
    MACRO_FIELD_SCHEMA_VERSION, MacroCoordinateTransform, ModelIdentity,
    NO_AUTHORED_CONTENT_VERSION, SourceDeviceRequirement, VOXEL_COMPOSER_VERSION,
    WorldSourceIdentity, WorldSourceIdentityHash, WorldSourceKind,
};

pub const MODEL_REPOSITORY: &str = "xandergos/terrain-diffusion-30m";
pub const MODEL_REVISION: &str = "9ef8030cb805b433b98ec25c5dddefbac07a9e26";
pub const IMPLEMENTATION_VERSION: u32 = 4;
pub const SAMPLER_VERSION: u32 = 1;
pub const SCHEDULER_VERSION: u32 = 1;
pub const COARSE_WEIGHT_SHA256: &str =
    "f88cdc26c70a2c73ee4c91088b8cec3f45202fc452682ac759554b099c5c2f33";
pub const BASE_WEIGHT_SHA256: &str =
    "351d1b1a77cf32e15adc4f72ed5fd26d317340598329d35f5dce2ff6dbcce735";
pub const DECODER_WEIGHT_SHA256: &str =
    "b6c7fa99f836ad75c514236c9529e18a68ea207ed59dd39fd1341fc9a8a03bcc";

pub const MODEL_FILES: [&str; 7] = [
    "config.json",
    "coarse_model/config.json",
    "coarse_model/diffusion_pytorch_model.safetensors",
    "base_model/config.json",
    "base_model/diffusion_pytorch_model.safetensors",
    "decoder_model/config.json",
    "decoder_model/diffusion_pytorch_model.safetensors",
];

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[repr(u8)]
pub enum TerrainPrecision {
    #[default]
    Float32 = 1,
    Float16 = 2,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TerrainDiffusionConfig {
    pub model_root: PathBuf,
    pub seed: u64,
    pub precision: TerrainPrecision,
    pub require_metal: bool,
    /// Canonical voxel X/Z coordinate where the generated finite macro tile is placed.
    pub world_origin_voxels: [i32; 2],
    /// Horizontal presentation scale relative to the model's native 30 m sample spacing.
    pub horizontal_scale: u32,
    /// Terrain Diffusion model-grid row/column used to key spatial sampling and noise.
    pub model_origin: [i32; 2],
}

impl TerrainDiffusionConfig {
    pub fn pinned(model_root: impl Into<PathBuf>, seed: u64) -> Self {
        Self {
            model_root: model_root.into(),
            seed,
            precision: TerrainPrecision::Float16,
            require_metal: true,
            world_origin_voxels: [0, 0],
            horizontal_scale: 2,
            model_origin: [0, 0],
        }
    }

    pub fn source_identity(&self) -> Result<WorldSourceIdentity, TerrainDiffusionError> {
        let mut configuration = Sha256::new();
        if !(1..=8).contains(&self.horizontal_scale) {
            return Err(TerrainDiffusionError::InvalidHorizontalScale(
                self.horizontal_scale,
            ));
        }
        configuration.update(b"voxels-terrain-diffusion-configuration-v5\0");
        configuration.update(self.seed.to_le_bytes());
        configuration.update([self.precision as u8]);
        for coordinate in self.world_origin_voxels {
            configuration.update(coordinate.to_le_bytes());
        }
        configuration.update(self.horizontal_scale.to_le_bytes());
        for coordinate in self.model_origin {
            configuration.update(coordinate.to_le_bytes());
        }
        configuration.update(
            b"coordinate-keyed-fractal-continent-climate-v1;elevation-noise-ratio-0.05;highest-variance-positive-4x4-coarse;highest-variance-16x16-latent;spatial-coarse-climate-lapse-aridity;physical-gradient-ridge;configurable-horizontal-scale-v1\0",
        );
        configuration.update(128_u32.to_le_bytes());
        let configuration_hash: [u8; 32] = configuration.finalize().into();
        Ok(WorldSourceIdentity {
            source_kind: WorldSourceKind::TerrainDiffusion30m,
            implementation_version: IMPLEMENTATION_VERSION,
            configuration_hash: WorldSourceIdentityHash::from_bytes(configuration_hash),
            model: Some(ModelIdentity {
                repository: MODEL_REPOSITORY.to_owned(),
                immutable_revision: MODEL_REVISION.to_owned(),
                weight_hashes: [
                    COARSE_WEIGHT_SHA256,
                    BASE_WEIGHT_SHA256,
                    DECODER_WEIGHT_SHA256,
                ]
                .map(parse_sha256)
                .into_iter()
                .collect::<Result<Vec<_>, _>>()?,
            }),
            sampler_version: SAMPLER_VERSION,
            scheduler_version: SCHEDULER_VERSION,
            macro_field_schema_version: MACRO_FIELD_SCHEMA_VERSION,
            macro_coordinate_transform: MacroCoordinateTransform {
                origin_voxels: self.world_origin_voxels.map(i64::from),
                horizontal_unit_millimetres: 30_000 * self.horizontal_scale,
                x_axis_sign: 1,
                z_axis_sign: 1,
            },
            voxel_composer_version: VOXEL_COMPOSER_VERSION,
            authored_content_version: NO_AUTHORED_CONTENT_VERSION,
            device_requirement: SourceDeviceRequirement::AppleMetal,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerrainDiffusionError {
    UnsupportedPlatform,
    MetalFeatureDisabled,
    MetalUnavailable(String),
    MissingModelFile(PathBuf),
    InvalidHorizontalScale(u32),
    InvalidModelFile {
        path: PathBuf,
        reason: String,
    },
    ModelHashMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    ModelDownload(String),
    Inference(String),
}

impl fmt::Display for TerrainDiffusionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPlatform => {
                formatter.write_str("Terrain Diffusion requires Apple silicon macOS")
            }
            Self::MetalFeatureDisabled => formatter
                .write_str("Terrain Diffusion was built without the optional `metal` feature"),
            Self::MetalUnavailable(reason) => write!(formatter, "Metal is unavailable: {reason}"),
            Self::MissingModelFile(path) => {
                write!(formatter, "missing model file {}", path.display())
            }
            Self::InvalidHorizontalScale(scale) => {
                write!(formatter, "horizontal scale {scale} is outside 1..=8")
            }
            Self::InvalidModelFile { path, reason } => {
                write!(formatter, "invalid model file {}: {reason}", path.display())
            }
            Self::ModelHashMismatch {
                path,
                expected,
                actual,
            } => write!(
                formatter,
                "model hash mismatch for {}: expected {expected}, got {actual}",
                path.display()
            ),
            Self::ModelDownload(reason) => write!(formatter, "model download failed: {reason}"),
            Self::Inference(reason) => {
                write!(formatter, "Terrain Diffusion inference failed: {reason}")
            }
        }
    }
}

impl std::error::Error for TerrainDiffusionError {}

pub fn validate_model_root(root: &Path) -> Result<(), TerrainDiffusionError> {
    for relative in MODEL_FILES {
        let path = root.join(relative);
        if !path.is_file() {
            return Err(TerrainDiffusionError::MissingModelFile(path));
        }
    }
    for (relative, expected) in [
        (
            "coarse_model/diffusion_pytorch_model.safetensors",
            COARSE_WEIGHT_SHA256,
        ),
        (
            "base_model/diffusion_pytorch_model.safetensors",
            BASE_WEIGHT_SHA256,
        ),
        (
            "decoder_model/diffusion_pytorch_model.safetensors",
            DECODER_WEIGHT_SHA256,
        ),
    ] {
        validate_hash(&root.join(relative), expected)?;
    }
    Ok(())
}

fn validate_hash(path: &Path, expected: &str) -> Result<(), TerrainDiffusionError> {
    let mut file =
        std::fs::File::open(path).map_err(|error| TerrainDiffusionError::InvalidModelFile {
            path: path.to_owned(),
            reason: error.to_string(),
        })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read =
            file.read(&mut buffer)
                .map_err(|error| TerrainDiffusionError::InvalidModelFile {
                    path: path.to_owned(),
                    reason: error.to_string(),
                })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let actual = format!("{:x}", hasher.finalize());
    if actual != expected {
        return Err(TerrainDiffusionError::ModelHashMismatch {
            path: path.to_owned(),
            expected: expected.to_owned(),
            actual,
        });
    }
    Ok(())
}

fn parse_sha256(value: &str) -> Result<WorldSourceIdentityHash, TerrainDiffusionError> {
    if value.len() != 64 {
        return Err(TerrainDiffusionError::Inference(
            "pinned SHA-256 digest has an invalid length".to_owned(),
        ));
    }
    let mut output = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let high = parse_hex_nibble(pair[0]).ok_or_else(|| {
            TerrainDiffusionError::Inference("pinned SHA-256 digest is not hexadecimal".to_owned())
        })?;
        let low = parse_hex_nibble(pair[1]).ok_or_else(|| {
            TerrainDiffusionError::Inference("pinned SHA-256 digest is not hexadecimal".to_owned())
        })?;
        output[index] = high << 4 | low;
    }
    Ok(WorldSourceIdentityHash::from_bytes(output))
}

const fn parse_hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

#[cfg(all(target_os = "macos", feature = "metal"))]
mod metal;

#[cfg(all(target_os = "macos", feature = "metal"))]
pub use metal::{
    CoarseTile, DetailTile, FullDetailTile, LatentTile, MetalTerrainDiffusion,
    TerrainDiffusionMacroTileSource, fetch_pinned_model,
};

#[cfg(not(all(target_os = "macos", feature = "metal")))]
pub fn fetch_pinned_model(_cache_root: &Path) -> Result<PathBuf, TerrainDiffusionError> {
    if cfg!(target_os = "macos") {
        Err(TerrainDiffusionError::MetalFeatureDisabled)
    } else {
        Err(TerrainDiffusionError::UnsupportedPlatform)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_configuration_requires_metal_and_fast_metal_precision_by_default() {
        let config = TerrainDiffusionConfig::pinned("model", 7);
        assert!(config.require_metal);
        assert_eq!(config.precision, TerrainPrecision::Float16);
        assert_eq!(config.model_root, PathBuf::from("model"));
        assert_eq!(config.world_origin_voxels, [0, 0]);
        assert_eq!(config.horizontal_scale, 2);
        assert_eq!(config.model_origin, [0, 0]);
    }

    #[test]
    fn source_identity_pins_model_hashes_and_apple_metal() {
        let identity = TerrainDiffusionConfig::pinned("model", 7)
            .source_identity()
            .expect("pinned identity");
        assert_eq!(identity.source_kind, WorldSourceKind::TerrainDiffusion30m);
        assert_eq!(
            identity.authored_content_version,
            NO_AUTHORED_CONTENT_VERSION
        );
        assert_eq!(
            identity.device_requirement,
            SourceDeviceRequirement::AppleMetal
        );
        assert_eq!(
            identity
                .macro_coordinate_transform
                .horizontal_unit_millimetres,
            60_000
        );
        let model = identity.model.expect("model identity");
        assert_eq!(model.repository, MODEL_REPOSITORY);
        assert_eq!(model.immutable_revision, MODEL_REVISION);
        assert_eq!(model.weight_hashes.len(), 3);
    }

    #[test]
    fn source_identity_binds_world_placement_and_model_sampling_origins() {
        let base = TerrainDiffusionConfig::pinned("model", 7);
        let base_identity = base.source_identity().expect("base identity");

        let mut placed = base.clone();
        placed.world_origin_voxels = [1_200, -900];
        let placed_identity = placed.source_identity().expect("placed identity");
        assert_ne!(
            base_identity.configuration_hash,
            placed_identity.configuration_hash
        );
        assert_eq!(
            placed_identity.macro_coordinate_transform.origin_voxels,
            [1_200, -900]
        );

        let mut sampled = base;
        sampled.model_origin = [-64, 128];
        let sampled_identity = sampled.source_identity().expect("sampled identity");
        assert_ne!(
            base_identity.configuration_hash,
            sampled_identity.configuration_hash
        );
        assert_ne!(
            placed_identity.identity_hash(),
            sampled_identity.identity_hash()
        );

        let mut scaled = sampled;
        scaled.horizontal_scale = 1;
        let scaled_identity = scaled.source_identity().expect("scaled identity");
        assert_ne!(
            base_identity.configuration_hash,
            scaled_identity.configuration_hash
        );

        scaled.horizontal_scale = 0;
        assert_eq!(
            scaled.source_identity(),
            Err(TerrainDiffusionError::InvalidHorizontalScale(0))
        );
    }

    #[test]
    fn missing_model_root_is_rejected_without_downloading_or_mutating_it() {
        let root = std::env::temp_dir().join(format!(
            "voxels-missing-terrain-diffusion-model-{}",
            std::process::id()
        ));
        assert!(matches!(
            validate_model_root(&root),
            Err(TerrainDiffusionError::MissingModelFile(_))
        ));
    }
}
