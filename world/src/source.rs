//! Transport-neutral world identity, batch requests, and in-process procedural source adapter.
//!
//! These owned products deliberately stop at world-data boundaries. They contain no renderer,
//! browser, socket, or provider-runtime types, so the same shapes can be encoded by a later daemon.

use crate::generation::Generator;
use crate::{
    ATLAS_VERSION, AtmosphereSample, CHUNK_EDGE, Chunk, ChunkCoord, GENERATOR_VERSION, Material,
    SkylineFeature, SkylineFeatureKind, SurfaceLodLevel, SurfaceRegion, SurfaceSample,
    SurfaceTileCoord, SurfaceTileMesh, VoxelCoord, WaterTileMesh,
    generate_edited_surface_tile_mesh, generate_edited_water_tile_mesh, generate_surface_tile_mesh,
    generate_water_tile_mesh_with, surface_tiles_affected_by_voxel,
};
use serde::{Deserialize, Serialize};
use std::fmt;

pub const WORLD_SCHEMA_VERSION: u32 = 1;
pub const MACRO_FIELD_SCHEMA_VERSION: u32 = 1;
pub const VOXEL_COMPOSER_VERSION: u32 = 3;
/// Source identity marker for providers that intentionally contain no authored atlas overlays.
pub const NO_AUTHORED_CONTENT_VERSION: u32 = 0;
pub const PROCEDURAL_SAMPLER_VERSION: u32 = 1;
pub const PROCEDURAL_SCHEDULER_VERSION: u32 = 1;
pub const MAX_WORLD_PRODUCT_BATCH: usize = 256;
pub const MAX_MACRO_BLOCK_SAMPLES: usize = 1_048_576;
pub const MAX_VOXEL_BLOCK_SAMPLES: usize = 1_048_576;
pub const MAX_SURFACE_SAMPLE_BLOCK_SAMPLES: usize = 65_536;
pub const MAX_SURFACE_SEARCH_RADIUS: u16 = 512;
pub const MESHING_HALO_VOXELS: usize =
    (CHUNK_EDGE + 2) * (CHUNK_EDGE + 2) * (CHUNK_EDGE + 2) - CHUNK_EDGE * CHUNK_EDGE * CHUNK_EDGE;

const IDENTITY_HASH_DOMAIN: &[u8] = b"voxels-world-source-identity-v1\0";
const MANIFEST_HASH_DOMAIN: &[u8] = b"voxels-world-manifest-v1\0";
const PROCEDURAL_CONFIGURATION_DOMAIN: &[u8] = b"voxels-procedural-v16-configuration-v1\0";

/// Stable 32-byte digest used by caches, codecs, and future protocol negotiation.
#[derive(Clone, Copy, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct WorldSourceIdentityHash([u8; 32]);

impl WorldSourceIdentityHash {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for WorldSourceIdentityHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("WorldSourceIdentityHash(")?;
        write_hex(formatter, &self.0)?;
        formatter.write_str(")")
    }
}

impl fmt::Display for WorldSourceIdentityHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_hex(formatter, &self.0)
    }
}

/// Stable digest of the complete immutable world manifest.
///
/// This is deliberately a distinct type from [`WorldSourceIdentityHash`]: chunk products are bound
/// to the source identity, while session negotiation and persisted manifest records use this hash.
#[derive(Clone, Copy, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct WorldManifestHash([u8; 32]);

impl WorldManifestHash {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for WorldManifestHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("WorldManifestHash(")?;
        write_hex(formatter, &self.0)?;
        formatter.write_str(")")
    }
}

impl fmt::Display for WorldManifestHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_hex(formatter, &self.0)
    }
}

fn write_hex(formatter: &mut fmt::Formatter<'_>, bytes: &[u8]) -> fmt::Result {
    for byte in bytes {
        write!(formatter, "{byte:02x}")?;
    }
    Ok(())
}

/// Opaque world identifier. Creation policy belongs to a world-creation or migration workflow.
#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
pub struct WorldId([u8; 16]);

impl WorldId {
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[repr(u8)]
pub enum WorldSourceKind {
    ProceduralV16 = 1,
    TerrainDiffusion30m = 2,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[repr(u8)]
pub enum SourceDeviceRequirement {
    PortableCpu = 1,
    AppleMetal = 2,
}

/// Explicit mapping from canonical voxel coordinates to macro-field coordinates.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct MacroCoordinateTransform {
    pub origin_voxels: [i64; 2],
    pub horizontal_unit_millimetres: u32,
    pub x_axis_sign: i8,
    pub z_axis_sign: i8,
}

impl MacroCoordinateTransform {
    pub const CANONICAL_VOXELS: Self = Self {
        origin_voxels: [0, 0],
        horizontal_unit_millimetres: 100,
        x_axis_sign: 1,
        z_axis_sign: 1,
    };
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ModelIdentity {
    pub repository: String,
    pub immutable_revision: String,
    pub weight_hashes: Vec<WorldSourceIdentityHash>,
}

/// Everything that can change pristine world products under one declared source identity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorldSourceIdentity {
    pub source_kind: WorldSourceKind,
    pub implementation_version: u32,
    pub configuration_hash: WorldSourceIdentityHash,
    pub model: Option<ModelIdentity>,
    pub sampler_version: u32,
    pub scheduler_version: u32,
    pub macro_field_schema_version: u32,
    pub macro_coordinate_transform: MacroCoordinateTransform,
    pub voxel_composer_version: u32,
    pub authored_content_version: u32,
    pub device_requirement: SourceDeviceRequirement,
}

impl WorldSourceIdentity {
    pub fn procedural_v16(seed: u64) -> Self {
        let mut configuration = blake3::Hasher::new();
        configuration.update(PROCEDURAL_CONFIGURATION_DOMAIN);
        configuration.update(&seed.to_le_bytes());
        Self {
            source_kind: WorldSourceKind::ProceduralV16,
            implementation_version: GENERATOR_VERSION,
            configuration_hash: WorldSourceIdentityHash::from_bytes(
                *configuration.finalize().as_bytes(),
            ),
            model: None,
            sampler_version: PROCEDURAL_SAMPLER_VERSION,
            scheduler_version: PROCEDURAL_SCHEDULER_VERSION,
            macro_field_schema_version: MACRO_FIELD_SCHEMA_VERSION,
            macro_coordinate_transform: MacroCoordinateTransform::CANONICAL_VOXELS,
            voxel_composer_version: VOXEL_COMPOSER_VERSION,
            authored_content_version: ATLAS_VERSION,
            device_requirement: SourceDeviceRequirement::PortableCpu,
        }
    }

    /// Hashes an explicit, stable field encoding rather than Rust memory or serde output.
    pub fn identity_hash(&self) -> WorldSourceIdentityHash {
        let mut hasher = blake3::Hasher::new();
        hasher.update(IDENTITY_HASH_DOMAIN);
        hasher.update(&[self.source_kind as u8]);
        hasher.update(&self.implementation_version.to_le_bytes());
        hasher.update(self.configuration_hash.as_bytes());
        hash_model_identity(&mut hasher, self.model.as_ref());
        hasher.update(&self.sampler_version.to_le_bytes());
        hasher.update(&self.scheduler_version.to_le_bytes());
        hasher.update(&self.macro_field_schema_version.to_le_bytes());
        for origin in self.macro_coordinate_transform.origin_voxels {
            hasher.update(&origin.to_le_bytes());
        }
        hasher.update(
            &self
                .macro_coordinate_transform
                .horizontal_unit_millimetres
                .to_le_bytes(),
        );
        hasher.update(&self.macro_coordinate_transform.x_axis_sign.to_le_bytes());
        hasher.update(&self.macro_coordinate_transform.z_axis_sign.to_le_bytes());
        hasher.update(&self.voxel_composer_version.to_le_bytes());
        hasher.update(&self.authored_content_version.to_le_bytes());
        hasher.update(&[self.device_requirement as u8]);
        WorldSourceIdentityHash::from_bytes(*hasher.finalize().as_bytes())
    }
}

fn hash_model_identity(hasher: &mut blake3::Hasher, model: Option<&ModelIdentity>) {
    let Some(model) = model else {
        hasher.update(&[0]);
        return;
    };
    hasher.update(&[1]);
    hash_string(hasher, &model.repository);
    hash_string(hasher, &model.immutable_revision);
    hasher.update(&(model.weight_hashes.len() as u64).to_le_bytes());
    for hash in &model.weight_hashes {
        hasher.update(hash.as_bytes());
    }
}

fn hash_string(hasher: &mut blake3::Hasher, value: &str) {
    hasher.update(&(value.len() as u64).to_le_bytes());
    hasher.update(value.as_bytes());
}

/// Immutable world metadata returned during future session negotiation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorldManifest {
    pub world_id: WorldId,
    pub seed: u64,
    pub world_schema_version: u32,
    pub material_schema_version: u16,
    pub source: WorldSourceIdentity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorldManifestError {
    WorldSchemaMismatch,
    MaterialSchemaMismatch,
    MacroFieldSchemaMismatch,
    VoxelComposerVersionMismatch,
    AuthoredContentVersionMismatch,
    InvalidMacroCoordinateTransform,
    ProceduralSourceMismatch,
    TerrainDiffusionModelMissing,
    TerrainDiffusionDeviceMismatch,
}

impl fmt::Display for WorldManifestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WorldSchemaMismatch => formatter.write_str("unsupported world schema"),
            Self::MaterialSchemaMismatch => formatter.write_str("unsupported material schema"),
            Self::MacroFieldSchemaMismatch => formatter.write_str("unsupported macro-field schema"),
            Self::VoxelComposerVersionMismatch => {
                formatter.write_str("unsupported voxel composer version")
            }
            Self::AuthoredContentVersionMismatch => {
                formatter.write_str("unsupported authored content version")
            }
            Self::InvalidMacroCoordinateTransform => {
                formatter.write_str("invalid macro coordinate transform")
            }
            Self::ProceduralSourceMismatch => {
                formatter.write_str("procedural source identity does not match the manifest seed")
            }
            Self::TerrainDiffusionModelMissing => {
                formatter.write_str("Terrain Diffusion source has no immutable model identity")
            }
            Self::TerrainDiffusionDeviceMismatch => {
                formatter.write_str("Terrain Diffusion source does not require Apple Metal")
            }
        }
    }
}

impl std::error::Error for WorldManifestError {}

impl WorldManifest {
    pub fn procedural_v16(world_id: WorldId, seed: u64) -> Self {
        Self {
            world_id,
            seed,
            world_schema_version: WORLD_SCHEMA_VERSION,
            material_schema_version: Material::SCHEMA_VERSION,
            source: WorldSourceIdentity::procedural_v16(seed),
        }
    }

    pub fn source_identity_hash(&self) -> WorldSourceIdentityHash {
        self.source.identity_hash()
    }

    pub fn validate(&self) -> Result<(), WorldManifestError> {
        if self.world_schema_version != WORLD_SCHEMA_VERSION {
            return Err(WorldManifestError::WorldSchemaMismatch);
        }
        if self.material_schema_version != Material::SCHEMA_VERSION {
            return Err(WorldManifestError::MaterialSchemaMismatch);
        }
        if self.source.macro_field_schema_version != MACRO_FIELD_SCHEMA_VERSION {
            return Err(WorldManifestError::MacroFieldSchemaMismatch);
        }
        if self.source.voxel_composer_version != VOXEL_COMPOSER_VERSION {
            return Err(WorldManifestError::VoxelComposerVersionMismatch);
        }
        let transform = self.source.macro_coordinate_transform;
        if transform.horizontal_unit_millimetres == 0
            || !matches!(transform.x_axis_sign, -1 | 1)
            || !matches!(transform.z_axis_sign, -1 | 1)
        {
            return Err(WorldManifestError::InvalidMacroCoordinateTransform);
        }
        match self.source.source_kind {
            WorldSourceKind::ProceduralV16 => {
                if self.source.authored_content_version != ATLAS_VERSION {
                    return Err(WorldManifestError::AuthoredContentVersionMismatch);
                }
                if self.source != WorldSourceIdentity::procedural_v16(self.seed) {
                    return Err(WorldManifestError::ProceduralSourceMismatch);
                }
            }
            WorldSourceKind::TerrainDiffusion30m => {
                if self.source.authored_content_version != NO_AUTHORED_CONTENT_VERSION {
                    return Err(WorldManifestError::AuthoredContentVersionMismatch);
                }
                let Some(model) = self.source.model.as_ref() else {
                    return Err(WorldManifestError::TerrainDiffusionModelMissing);
                };
                if model.repository.is_empty()
                    || model.immutable_revision.is_empty()
                    || model.weight_hashes.is_empty()
                {
                    return Err(WorldManifestError::TerrainDiffusionModelMissing);
                }
                if self.source.device_requirement != SourceDeviceRequirement::AppleMetal {
                    return Err(WorldManifestError::TerrainDiffusionDeviceMismatch);
                }
            }
        }
        Ok(())
    }

    pub fn manifest_hash(&self) -> Result<WorldManifestHash, WorldManifestError> {
        self.validate()?;
        let mut hasher = blake3::Hasher::new();
        hasher.update(MANIFEST_HASH_DOMAIN);
        hasher.update(self.world_id.as_bytes());
        hasher.update(&self.seed.to_le_bytes());
        hasher.update(&self.world_schema_version.to_le_bytes());
        hasher.update(&self.material_schema_version.to_le_bytes());
        hasher.update(self.source_identity_hash().as_bytes());
        Ok(WorldManifestHash::from_bytes(*hasher.finalize().as_bytes()))
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[repr(u8)]
pub enum WorldProductPriority {
    CollisionCritical = 1,
    VisibleChunk = 2,
    VisibleSurface = 3,
    ReplacementSurface = 4,
    Prefetch = 5,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum WorldProductRequest {
    ChunkWithHalo(ChunkCoord),
    VoxelBlock(VoxelBlockRequest),
    SurfaceSampleBlock(SurfaceSampleBlockRequest),
    SurfaceSearch(SurfaceSearchRequest),
    SurfaceTile(SurfaceTileCoord),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorldProductBatch {
    pub priority: WorldProductPriority,
    pub requests: Vec<WorldProductRequest>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChunkSnapshot {
    pub source_identity_hash: WorldSourceIdentityHash,
    pub chunk: Chunk,
    pub meshing_halo: MeshingHalo,
}

impl ChunkSnapshot {
    /// Checked construction seam used by sibling codecs and source adapters.
    pub(crate) fn new(
        source_identity_hash: WorldSourceIdentityHash,
        chunk: Chunk,
        meshing_halo: MeshingHalo,
    ) -> Option<Self> {
        (chunk.coord() == meshing_halo.coord()).then_some(Self {
            source_identity_hash,
            chunk,
            meshing_halo,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SurfaceTileSnapshot {
    pub source_identity_hash: WorldSourceIdentityHash,
    pub terrain: SurfaceTileMesh,
    pub water: WaterTileMesh,
}

#[derive(Clone, Debug, PartialEq)]
pub enum WorldProduct {
    Chunk(ChunkSnapshot),
    VoxelBlock(VoxelBlockSnapshot),
    SurfaceSampleBlock(SurfaceSampleBlockSnapshot),
    SurfaceSearch(SurfaceSearchSnapshot),
    SurfaceTile(SurfaceTileSnapshot),
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorldProductBatchResult {
    pub source_identity_hash: WorldSourceIdentityHash,
    pub items: Vec<WorldProductBatchItem>,
}

/// One keyed result from a product batch. Request identity is retained so transports may reorder
/// or partially complete a batch without allowing callers to attach a product to the wrong key.
#[derive(Clone, Debug, PartialEq)]
pub struct WorldProductBatchItem {
    pub request: WorldProductRequest,
    pub result: Result<WorldProduct, WorldSourceError>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorldSourceError {
    BatchTooLarge,
    InvalidChunkCoordinate,
    InvalidSurfaceTileCoordinate,
    InvalidBlockCoordinate,
    EmptyBlock,
    BlockTooLarge,
    InvalidSearchRadius,
    SearchRadiusTooLarge,
    EmptyMacroBlock,
    MacroBlockTooLarge,
    MalformedMacroBlock,
    SourceCoverageUnavailable,
}

impl fmt::Display for WorldSourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BatchTooLarge => {
                formatter.write_str("world product batch exceeds its hard limit")
            }
            Self::InvalidChunkCoordinate => {
                formatter.write_str("chunk coordinate is outside the canonical voxel grid")
            }
            Self::InvalidSurfaceTileCoordinate => {
                formatter.write_str("surface tile is outside the canonical voxel grid")
            }
            Self::InvalidBlockCoordinate => {
                formatter.write_str("sample block is outside the canonical voxel grid")
            }
            Self::EmptyBlock => formatter.write_str("sample block dimensions must be non-zero"),
            Self::BlockTooLarge => formatter.write_str("sample block exceeds its hard limit"),
            Self::InvalidSearchRadius => {
                formatter.write_str("surface search minimum radius exceeds its maximum")
            }
            Self::SearchRadiusTooLarge => {
                formatter.write_str("surface search radius exceeds its hard limit")
            }
            Self::EmptyMacroBlock => {
                formatter.write_str("macro block dimensions and stride must be non-zero")
            }
            Self::MacroBlockTooLarge => {
                formatter.write_str("macro block exceeds its hard sample limit")
            }
            Self::MalformedMacroBlock => {
                formatter.write_str("macro source returned a malformed or mismatched field block")
            }
            Self::SourceCoverageUnavailable => {
                formatter.write_str("world product crosses unavailable macro-source coverage")
            }
        }
    }
}

impl std::error::Error for WorldSourceError {}

/// Provider boundary for owned canonical products. Production transports must stay batch-shaped.
pub trait WorldSourceEngine: Send + Sync {
    fn identity(&self) -> &WorldSourceIdentity;

    fn generate_batch(
        &self,
        request: WorldProductBatch,
    ) -> Result<WorldProductBatchResult, WorldSourceError>;

    fn generate_edited_surface_tile(
        &self,
        edits: &crate::EditMap,
        coord: SurfaceTileCoord,
    ) -> Result<SurfaceTileSnapshot, WorldSourceError>;

    fn surface_tiles_affected_by_voxel(
        &self,
        edits: &crate::EditMap,
        level: SurfaceLodLevel,
        coord: VoxelCoord,
    ) -> Vec<SurfaceTileCoord>;

    fn atmosphere_sample(&self, x: i32, z: i32) -> (AtmosphereSample, SurfaceRegion);

    fn skyline_features_anchored_in(&self, bounds: [[i32; 2]; 2]) -> Vec<SkylineFeature>;

    fn skyline_features_at(&self, coord: VoxelCoord) -> Vec<SkylineFeature>;

    fn nearest_skyline_feature(
        &self,
        x: i32,
        z: i32,
        kind: SkylineFeatureKind,
        max_radius_cells: i32,
    ) -> Option<SkylineFeature>;

    fn nearest_prominent_skyline_feature(
        &self,
        x: i32,
        z: i32,
        kind: SkylineFeatureKind,
        max_radius_cells: i32,
    ) -> Option<SkylineFeature>;
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct VoxelBlockRequest {
    pub min: VoxelCoord,
    /// Sample counts along X, Y, and Z. X is the fastest-moving encoded axis.
    pub sample_shape: [u32; 3],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VoxelBlockSnapshot {
    pub source_identity_hash: WorldSourceIdentityHash,
    pub request: VoxelBlockRequest,
    materials: Box<[Material]>,
}

impl VoxelBlockSnapshot {
    /// Checked construction seam used by sibling codecs and source adapters.
    pub(crate) fn from_materials(
        source_identity_hash: WorldSourceIdentityHash,
        request: VoxelBlockRequest,
        materials: Vec<Material>,
    ) -> Option<Self> {
        let [width, height, depth] = request.sample_shape;
        let sample_count = usize::try_from(width)
            .ok()?
            .checked_mul(usize::try_from(height).ok()?)?
            .checked_mul(usize::try_from(depth).ok()?)?;
        (sample_count > 0
            && sample_count <= MAX_VOXEL_BLOCK_SAMPLES
            && materials.len() == sample_count
            && block_axis_is_representable(request.min.x, width)
            && block_axis_is_representable(request.min.y, height)
            && block_axis_is_representable(request.min.z, depth))
        .then(|| Self {
            source_identity_hash,
            request,
            materials: materials.into_boxed_slice(),
        })
    }

    pub fn materials(&self) -> &[Material] {
        &self.materials
    }

    pub fn sample(&self, coord: VoxelCoord) -> Option<Material> {
        let offset = [
            i64::from(coord.x) - i64::from(self.request.min.x),
            i64::from(coord.y) - i64::from(self.request.min.y),
            i64::from(coord.z) - i64::from(self.request.min.z),
        ];
        let [width, height, depth] = self.request.sample_shape.map(i64::from);
        if offset[0] < 0
            || offset[1] < 0
            || offset[2] < 0
            || offset[0] >= width
            || offset[1] >= height
            || offset[2] >= depth
        {
            return None;
        }
        let index = offset[0] + offset[1] * width + offset[2] * width * height;
        self.materials.get(usize::try_from(index).ok()?).copied()
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SurfaceSampleBlockRequest {
    pub origin: [i32; 2],
    /// Sample counts along X and Z. X is the fastest-moving encoded axis.
    pub sample_shape: [u32; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceSampleBlockSnapshot {
    pub source_identity_hash: WorldSourceIdentityHash,
    pub request: SurfaceSampleBlockRequest,
    samples: Box<[SurfaceSample]>,
}

impl SurfaceSampleBlockSnapshot {
    /// Checked construction seam used by sibling codecs and source adapters.
    pub(crate) fn from_samples(
        source_identity_hash: WorldSourceIdentityHash,
        request: SurfaceSampleBlockRequest,
        samples: Vec<SurfaceSample>,
    ) -> Option<Self> {
        let [width, depth] = request.sample_shape;
        let sample_count = usize::try_from(width)
            .ok()?
            .checked_mul(usize::try_from(depth).ok()?)?;
        (sample_count > 0
            && sample_count <= MAX_SURFACE_SAMPLE_BLOCK_SAMPLES
            && samples.len() == sample_count
            && block_axis_is_representable(request.origin[0], width)
            && block_axis_is_representable(request.origin[1], depth))
        .then(|| Self {
            source_identity_hash,
            request,
            samples: samples.into_boxed_slice(),
        })
    }

    pub fn samples(&self) -> &[SurfaceSample] {
        &self.samples
    }

    pub fn sample(&self, x: i32, z: i32) -> Option<SurfaceSample> {
        let offset_x = i64::from(x) - i64::from(self.request.origin[0]);
        let offset_z = i64::from(z) - i64::from(self.request.origin[1]);
        let [width, depth] = self.request.sample_shape.map(i64::from);
        if offset_x < 0 || offset_z < 0 || offset_x >= width || offset_z >= depth {
            return None;
        }
        let index = offset_x + offset_z * width;
        self.samples.get(usize::try_from(index).ok()?).copied()
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SurfaceSearchKind {
    DryLand,
    WaterDepthAtLeast { depth_voxels: u16 },
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SurfaceSearchRequest {
    pub origin: [i32; 2],
    pub min_radius: u16,
    pub max_radius: u16,
    pub kind: SurfaceSearchKind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceSearchHit {
    pub coord: [i32; 2],
    pub sample: SurfaceSample,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceSearchSnapshot {
    pub source_identity_hash: WorldSourceIdentityHash,
    pub request: SurfaceSearchRequest,
    pub hit: Option<SurfaceSearchHit>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MacroBlockRequest {
    /// First sample in canonical voxel X/Z coordinates.
    pub origin: [i32; 2],
    /// Row-major sample count along X and Z.
    pub sample_shape: [u32; 2],
    pub stride_voxels: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MacroBlockBatch {
    pub priority: WorldProductPriority,
    pub requests: Vec<MacroBlockRequest>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MacroBlock {
    pub schema_version: u32,
    pub request: MacroBlockRequest,
    pub coordinate_transform: MacroCoordinateTransform,
    /// Canonical voxel elevation, row-major over `request.sample_shape`.
    pub elevation_voxels: Vec<f32>,
    /// Normalized source field in the inclusive range zero to one.
    pub temperature: Vec<f32>,
    /// Normalized source field in the inclusive range zero to one.
    pub moisture: Vec<f32>,
    /// Normalized source field in the inclusive range zero to one.
    pub ridge: Vec<f32>,
    /// False values mark coordinates outside the finite canonical voxel grid.
    pub validity: Vec<bool>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MacroBlockBatchResult {
    pub source_identity_hash: WorldSourceIdentityHash,
    pub blocks: Vec<MacroBlock>,
}

/// Provider boundary for versioned continuous macro terrain fields.
pub trait MacroTerrainSource: Send + Sync {
    fn identity(&self) -> &WorldSourceIdentity;

    fn request_blocks(
        &self,
        request: MacroBlockBatch,
    ) -> Result<MacroBlockBatchResult, WorldSourceError>;
}

/// Exact material shell read by `mesh_chunk`: the 34-cubed envelope minus the 32-cubed core.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MeshingHalo {
    coord: ChunkCoord,
    voxels: Box<[Material]>,
}

impl MeshingHalo {
    /// Checked construction seam used by sibling codecs and source adapters.
    pub(crate) fn from_voxels(coord: ChunkCoord, voxels: Vec<Material>) -> Option<Self> {
        (coord.is_world_representable() && voxels.len() == MESHING_HALO_VOXELS).then(|| Self {
            coord,
            voxels: voxels.into_boxed_slice(),
        })
    }

    pub fn from_sampler(
        coord: ChunkCoord,
        mut sample: impl FnMut(i32, i32, i32) -> Material,
    ) -> Self {
        let origin = coord.world_origin();
        let edge = CHUNK_EDGE as i32;
        let mut voxels = Vec::with_capacity(MESHING_HALO_VOXELS);
        for y in -1..=edge {
            for z in -1..=edge {
                for x in -1..=edge {
                    if is_chunk_core([x, y, z]) {
                        continue;
                    }
                    let world = [
                        origin[0].checked_add(x),
                        origin[1].checked_add(y),
                        origin[2].checked_add(z),
                    ];
                    voxels.push(match world {
                        [Some(x), Some(y), Some(z)] => sample(x, y, z),
                        _ => Material::Air,
                    });
                }
            }
        }
        debug_assert_eq!(voxels.len(), MESHING_HALO_VOXELS);
        Self {
            coord,
            voxels: voxels.into_boxed_slice(),
        }
    }

    pub const fn coord(&self) -> ChunkCoord {
        self.coord
    }

    pub fn voxels(&self) -> &[Material] {
        &self.voxels
    }

    pub fn logical_bytes(&self) -> usize {
        self.voxels.len() * size_of::<Material>()
    }

    pub fn sample_local(&self, local: [i32; 3]) -> Option<Material> {
        meshing_halo_index(local).map(|index| self.voxels[index])
    }

    /// Replaces one material in the shell when the coordinate belongs to this halo.
    ///
    /// Authoritative edit commits use this to keep resident near-world meshes coherent without a
    /// redundant network round trip. Core coordinates deliberately return `false`; their owning
    /// [`Chunk`] is the canonical resident storage for those voxels.
    pub fn set_world(&mut self, x: i32, y: i32, z: i32, material: Material) -> bool {
        let origin = self.coord.world_origin();
        let local = [
            i64::from(x) - i64::from(origin[0]),
            i64::from(y) - i64::from(origin[1]),
            i64::from(z) - i64::from(origin[2]),
        ];
        let (Ok(x), Ok(y), Ok(z)) = (
            i32::try_from(local[0]),
            i32::try_from(local[1]),
            i32::try_from(local[2]),
        ) else {
            return false;
        };
        let local = [x, y, z];
        let Some(index) = meshing_halo_index(local) else {
            return false;
        };
        self.voxels[index] = material;
        true
    }

    pub fn sample_world(&self, x: i32, y: i32, z: i32) -> Option<Material> {
        let origin = self.coord.world_origin();
        let local = [
            i64::from(x) - i64::from(origin[0]),
            i64::from(y) - i64::from(origin[1]),
            i64::from(z) - i64::from(origin[2]),
        ];
        let local = [
            i32::try_from(local[0]).ok()?,
            i32::try_from(local[1]).ok()?,
            i32::try_from(local[2]).ok()?,
        ];
        self.sample_local(local)
    }
}

fn is_chunk_core([x, y, z]: [i32; 3]) -> bool {
    let edge = CHUNK_EDGE as i32;
    [x, y, z]
        .into_iter()
        .all(|value| value >= 0 && value < edge)
}

fn meshing_halo_index([x, y, z]: [i32; 3]) -> Option<usize> {
    let edge = CHUNK_EDGE as i32;
    if [x, y, z]
        .into_iter()
        .any(|value| value < -1 || value > edge)
        || is_chunk_core([x, y, z])
    {
        return None;
    }
    let padded = CHUNK_EDGE + 2;
    let outer_plane = padded * padded;
    if y == -1 {
        return Some((x + 1) as usize + (z + 1) as usize * padded);
    }
    if y == edge {
        return Some(
            outer_plane
                + CHUNK_EDGE * (CHUNK_EDGE * 2 + padded * 2)
                + (x + 1) as usize
                + (z + 1) as usize * padded,
        );
    }
    let slice = outer_plane + y as usize * (CHUNK_EDGE * 2 + padded * 2);
    if z == -1 {
        Some(slice + (x + 1) as usize)
    } else if z == edge {
        Some(slice + padded + CHUNK_EDGE * 2 + (x + 1) as usize)
    } else if x == -1 {
        Some(slice + padded + z as usize * 2)
    } else {
        Some(slice + padded + z as usize * 2 + 1)
    }
}

/// Current deterministic provider, kept behind the same owned batch contract used by future sources.
#[derive(Clone, Debug)]
pub struct ProceduralWorldSource {
    generator: Generator,
    identity: WorldSourceIdentity,
}

/// Selects the current portable provider behind the source-neutral engine boundary.
pub fn procedural_world_source(seed: u64) -> Box<dyn WorldSourceEngine> {
    Box::new(ProceduralWorldSource::new(seed))
}

impl ProceduralWorldSource {
    pub fn new(seed: u64) -> Self {
        Self {
            generator: Generator::new(seed),
            identity: WorldSourceIdentity::procedural_v16(seed),
        }
    }

    pub const fn seed(&self) -> u64 {
        self.generator.seed()
    }

    pub fn source_identity_hash(&self) -> WorldSourceIdentityHash {
        self.identity.identity_hash()
    }

    fn voxel_block(
        &self,
        request: VoxelBlockRequest,
    ) -> Result<VoxelBlockSnapshot, WorldSourceError> {
        let [width, height, depth] = request.sample_shape;
        if request.sample_shape.contains(&0) {
            return Err(WorldSourceError::EmptyBlock);
        }
        let sample_count = usize::try_from(width)
            .ok()
            .and_then(|width| {
                usize::try_from(height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .and_then(|plane| {
                usize::try_from(depth)
                    .ok()
                    .and_then(|depth| plane.checked_mul(depth))
            })
            .ok_or(WorldSourceError::BlockTooLarge)?;
        if sample_count > MAX_VOXEL_BLOCK_SAMPLES
            || !block_axis_is_representable(request.min.x, width)
            || !block_axis_is_representable(request.min.y, height)
            || !block_axis_is_representable(request.min.z, depth)
        {
            return if sample_count > MAX_VOXEL_BLOCK_SAMPLES {
                Err(WorldSourceError::BlockTooLarge)
            } else {
                Err(WorldSourceError::InvalidBlockCoordinate)
            };
        }
        let region =
            self.generator
                .region(request.min.x, request.min.z, width as usize, depth as usize);
        let mut materials = Vec::with_capacity(sample_count);
        for z in 0..depth {
            for y in 0..height {
                for x in 0..width {
                    materials.push(region.sample(
                        request.min.x + x as i32,
                        request.min.y + y as i32,
                        request.min.z + z as i32,
                    ));
                }
            }
        }
        Ok(VoxelBlockSnapshot {
            source_identity_hash: self.source_identity_hash(),
            request,
            materials: materials.into_boxed_slice(),
        })
    }

    fn surface_sample_block(
        &self,
        request: SurfaceSampleBlockRequest,
    ) -> Result<SurfaceSampleBlockSnapshot, WorldSourceError> {
        let [width, depth] = request.sample_shape;
        if request.sample_shape.contains(&0) {
            return Err(WorldSourceError::EmptyBlock);
        }
        let sample_count = usize::try_from(width)
            .ok()
            .and_then(|width| {
                usize::try_from(depth)
                    .ok()
                    .and_then(|depth| width.checked_mul(depth))
            })
            .ok_or(WorldSourceError::BlockTooLarge)?;
        if sample_count > MAX_SURFACE_SAMPLE_BLOCK_SAMPLES {
            return Err(WorldSourceError::BlockTooLarge);
        }
        if !block_axis_is_representable(request.origin[0], width)
            || !block_axis_is_representable(request.origin[1], depth)
        {
            return Err(WorldSourceError::InvalidBlockCoordinate);
        }
        let mut samples = Vec::with_capacity(sample_count);
        for z in 0..depth {
            for x in 0..width {
                samples.push(
                    self.generator
                        .surface_sample(request.origin[0] + x as i32, request.origin[1] + z as i32),
                );
            }
        }
        Ok(SurfaceSampleBlockSnapshot {
            source_identity_hash: self.source_identity_hash(),
            request,
            samples: samples.into_boxed_slice(),
        })
    }

    fn surface_search(
        &self,
        request: SurfaceSearchRequest,
    ) -> Result<SurfaceSearchSnapshot, WorldSourceError> {
        if request.min_radius > request.max_radius {
            return Err(WorldSourceError::InvalidSearchRadius);
        }
        if request.max_radius > MAX_SURFACE_SEARCH_RADIUS {
            return Err(WorldSourceError::SearchRadiusTooLarge);
        }
        let radius = i32::from(request.max_radius);
        if request.origin[0].checked_sub(radius).is_none()
            || request.origin[0].checked_add(radius).is_none()
            || request.origin[1].checked_sub(radius).is_none()
            || request.origin[1].checked_add(radius).is_none()
        {
            return Err(WorldSourceError::InvalidBlockCoordinate);
        }
        let matches = |sample: SurfaceSample| match request.kind {
            SurfaceSearchKind::DryLand => sample.water_level.is_none(),
            SurfaceSearchKind::WaterDepthAtLeast { depth_voxels } => {
                sample.water_level.is_some_and(|water| {
                    i64::from(water) - i64::from(sample.height) >= i64::from(depth_voxels)
                })
            }
        };
        let hit = 'search: {
            for radius in i32::from(request.min_radius)..=i32::from(request.max_radius) {
                let min_x = request.origin[0] - radius;
                let max_x = request.origin[0] + radius;
                let min_z = request.origin[1] - radius;
                let max_z = request.origin[1] + radius;
                for x in min_x..=max_x {
                    for z in [min_z, max_z] {
                        let sample = self.generator.surface_sample(x, z);
                        if matches(sample) {
                            break 'search Some(SurfaceSearchHit {
                                coord: [x, z],
                                sample,
                            });
                        }
                    }
                }
                for z in min_z.saturating_add(1)..max_z {
                    for x in [min_x, max_x] {
                        let sample = self.generator.surface_sample(x, z);
                        if matches(sample) {
                            break 'search Some(SurfaceSearchHit {
                                coord: [x, z],
                                sample,
                            });
                        }
                    }
                }
            }
            None
        };
        Ok(SurfaceSearchSnapshot {
            source_identity_hash: self.source_identity_hash(),
            request,
            hit,
        })
    }

    fn chunk_with_halo(&self, coord: ChunkCoord) -> ChunkSnapshot {
        let chunk = self.generator.generate_chunk(coord);
        let origin = coord.world_origin();
        let min_x = origin[0].saturating_sub(1);
        let min_z = origin[2].saturating_sub(1);
        let max_x = origin[0].saturating_add(CHUNK_EDGE as i32);
        let max_z = origin[2].saturating_add(CHUNK_EDGE as i32);
        let width = (i64::from(max_x) - i64::from(min_x) + 1) as usize;
        let depth = (i64::from(max_z) - i64::from(min_z) + 1) as usize;
        let region = self.generator.region(min_x, min_z, width, depth);
        let meshing_halo = MeshingHalo::from_sampler(coord, |x, y, z| region.sample(x, y, z));
        ChunkSnapshot {
            source_identity_hash: self.source_identity_hash(),
            chunk,
            meshing_halo,
        }
    }
}

fn block_axis_is_representable(origin: i32, sample_count: u32) -> bool {
    i64::from(origin) + i64::from(sample_count) - 1 <= i64::from(i32::MAX)
}

impl WorldSourceEngine for ProceduralWorldSource {
    fn identity(&self) -> &WorldSourceIdentity {
        &self.identity
    }

    fn generate_batch(
        &self,
        request: WorldProductBatch,
    ) -> Result<WorldProductBatchResult, WorldSourceError> {
        if request.requests.len() > MAX_WORLD_PRODUCT_BATCH {
            return Err(WorldSourceError::BatchTooLarge);
        }
        let mut items = Vec::with_capacity(request.requests.len());
        for request in request.requests {
            let result = match request {
                WorldProductRequest::ChunkWithHalo(coord) => {
                    if !coord.is_world_representable() {
                        Err(WorldSourceError::InvalidChunkCoordinate)
                    } else {
                        Ok(WorldProduct::Chunk(self.chunk_with_halo(coord)))
                    }
                }
                WorldProductRequest::VoxelBlock(request) => {
                    self.voxel_block(request).map(WorldProduct::VoxelBlock)
                }
                WorldProductRequest::SurfaceSampleBlock(request) => self
                    .surface_sample_block(request)
                    .map(WorldProduct::SurfaceSampleBlock),
                WorldProductRequest::SurfaceSearch(request) => self
                    .surface_search(request)
                    .map(WorldProduct::SurfaceSearch),
                WorldProductRequest::SurfaceTile(coord) => {
                    if !coord.is_world_representable() {
                        Err(WorldSourceError::InvalidSurfaceTileCoordinate)
                    } else {
                        let terrain = generate_surface_tile_mesh(self.generator, coord);
                        let water = generate_water_tile_mesh_with(coord, |x, z| {
                            self.generator.surface_sample(x, z).water_level
                                == Some(crate::SEA_LEVEL_VOXELS)
                        });
                        Ok(WorldProduct::SurfaceTile(SurfaceTileSnapshot {
                            source_identity_hash: self.source_identity_hash(),
                            terrain,
                            water,
                        }))
                    }
                }
            };
            items.push(WorldProductBatchItem { request, result });
        }
        Ok(WorldProductBatchResult {
            source_identity_hash: self.source_identity_hash(),
            items,
        })
    }

    fn generate_edited_surface_tile(
        &self,
        edits: &crate::EditMap,
        coord: SurfaceTileCoord,
    ) -> Result<SurfaceTileSnapshot, WorldSourceError> {
        if !coord.is_world_representable() {
            return Err(WorldSourceError::InvalidSurfaceTileCoordinate);
        }
        Ok(SurfaceTileSnapshot {
            source_identity_hash: self.source_identity_hash(),
            terrain: generate_edited_surface_tile_mesh(self.generator, edits, coord),
            water: generate_edited_water_tile_mesh(self.generator, edits, coord),
        })
    }

    fn surface_tiles_affected_by_voxel(
        &self,
        edits: &crate::EditMap,
        level: SurfaceLodLevel,
        coord: VoxelCoord,
    ) -> Vec<SurfaceTileCoord> {
        surface_tiles_affected_by_voxel(self.generator, edits, level, coord)
    }

    fn atmosphere_sample(&self, x: i32, z: i32) -> (AtmosphereSample, SurfaceRegion) {
        self.generator.atmosphere_sample(x, z)
    }

    fn skyline_features_anchored_in(&self, bounds: [[i32; 2]; 2]) -> Vec<SkylineFeature> {
        self.generator.skyline_features_anchored_in(bounds)
    }

    fn skyline_features_at(&self, coord: VoxelCoord) -> Vec<SkylineFeature> {
        self.generator.skyline_features_at(coord)
    }

    fn nearest_skyline_feature(
        &self,
        x: i32,
        z: i32,
        kind: SkylineFeatureKind,
        max_radius_cells: i32,
    ) -> Option<SkylineFeature> {
        self.generator
            .nearest_skyline_feature(x, z, kind, max_radius_cells)
    }

    fn nearest_prominent_skyline_feature(
        &self,
        x: i32,
        z: i32,
        kind: SkylineFeatureKind,
        max_radius_cells: i32,
    ) -> Option<SkylineFeature> {
        self.generator
            .nearest_prominent_skyline_feature(x, z, kind, max_radius_cells)
    }
}

impl MacroTerrainSource for ProceduralWorldSource {
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
                    let offset_x = i64::from(x) * i64::from(request.stride_voxels);
                    let offset_z = i64::from(z) * i64::from(request.stride_voxels);
                    let world_x = i64::from(request.origin[0]) + offset_x;
                    let world_z = i64::from(request.origin[1]) + offset_z;
                    let valid = i32::try_from(world_x).ok().zip(i32::try_from(world_z).ok());
                    if let Some((x, z)) = valid {
                        let sample = self.generator.natural_surface_sample(x, z);
                        elevation_voxels.push(sample.height as f32);
                        temperature.push(sample.temperature);
                        moisture.push(sample.moisture);
                        ridge.push(sample.ridge);
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
                coordinate_transform: MacroCoordinateTransform::CANONICAL_VOXELS,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn terrain_diffusion_manifest() -> WorldManifest {
        let mut manifest = WorldManifest::procedural_v16(WorldId::from_bytes([7; 16]), 7);
        manifest.source.source_kind = WorldSourceKind::TerrainDiffusion30m;
        manifest.source.authored_content_version = NO_AUTHORED_CONTENT_VERSION;
        manifest.source.model = Some(ModelIdentity {
            repository: "immutable/model".to_owned(),
            immutable_revision: "0123456789abcdef".to_owned(),
            weight_hashes: vec![WorldSourceIdentityHash::from_bytes([3; 32])],
        });
        manifest.source.device_requirement = SourceDeviceRequirement::AppleMetal;
        manifest
    }

    fn assert_manifest_error(manifest: &WorldManifest, error: WorldManifestError) {
        assert_eq!(manifest.validate(), Err(error));
        assert_eq!(manifest.manifest_hash(), Err(error));
    }

    #[test]
    fn procedural_identity_is_stable_and_seed_sensitive() {
        let first = WorldSourceIdentity::procedural_v16(7);
        let same = WorldSourceIdentity::procedural_v16(7);
        let other = WorldSourceIdentity::procedural_v16(8);
        assert_eq!(first.identity_hash(), same.identity_hash());
        assert_ne!(first.identity_hash(), other.identity_hash());
        assert_eq!(
            first.identity_hash().to_string(),
            "46f9a1966f87782eee3a4cc66470e3d357f1282defb8bdf07013d34f7679570f"
        );
    }

    #[test]
    fn manifest_hash_binds_world_identity_without_changing_source_identity() {
        let first = WorldManifest::procedural_v16(WorldId::from_bytes([1; 16]), 7);
        let second = WorldManifest::procedural_v16(WorldId::from_bytes([2; 16]), 7);
        assert_eq!(first.source_identity_hash(), second.source_identity_hash());
        assert_ne!(first.manifest_hash(), second.manifest_hash());
        assert_eq!(first.validate(), Ok(()));
        assert_eq!(
            first.manifest_hash().map(|hash| hash.to_string()),
            Ok("4d59d545152ea1419ce3100ba7b3fc0adf371921ee36865c398c98ec877be90d".to_owned())
        );

        let mut inconsistent = first;
        inconsistent.seed = 8;
        assert_eq!(
            inconsistent.validate(),
            Err(WorldManifestError::ProceduralSourceMismatch)
        );
        assert_eq!(
            inconsistent.manifest_hash(),
            Err(WorldManifestError::ProceduralSourceMismatch)
        );
    }

    #[test]
    fn manifests_reject_unsupported_shared_versions_and_invalid_coordinate_transforms() {
        let valid = terrain_diffusion_manifest();
        assert_eq!(valid.validate(), Ok(()));

        let mut manifest = valid.clone();
        manifest.source.macro_field_schema_version = MACRO_FIELD_SCHEMA_VERSION + 1;
        assert_manifest_error(&manifest, WorldManifestError::MacroFieldSchemaMismatch);

        let mut manifest = valid.clone();
        manifest.source.voxel_composer_version = VOXEL_COMPOSER_VERSION + 1;
        assert_manifest_error(&manifest, WorldManifestError::VoxelComposerVersionMismatch);

        let mut manifest = valid.clone();
        manifest.source.authored_content_version = ATLAS_VERSION + 1;
        assert_manifest_error(
            &manifest,
            WorldManifestError::AuthoredContentVersionMismatch,
        );

        for transform in [
            MacroCoordinateTransform {
                horizontal_unit_millimetres: 0,
                ..MacroCoordinateTransform::CANONICAL_VOXELS
            },
            MacroCoordinateTransform {
                x_axis_sign: 0,
                ..MacroCoordinateTransform::CANONICAL_VOXELS
            },
            MacroCoordinateTransform {
                x_axis_sign: 2,
                ..MacroCoordinateTransform::CANONICAL_VOXELS
            },
            MacroCoordinateTransform {
                z_axis_sign: 0,
                ..MacroCoordinateTransform::CANONICAL_VOXELS
            },
            MacroCoordinateTransform {
                z_axis_sign: -2,
                ..MacroCoordinateTransform::CANONICAL_VOXELS
            },
        ] {
            let mut manifest = valid.clone();
            manifest.source.macro_coordinate_transform = transform;
            assert_manifest_error(
                &manifest,
                WorldManifestError::InvalidMacroCoordinateTransform,
            );
        }
    }

    #[test]
    fn source_identity_hash_binds_every_provider_semantic_field() {
        let base = WorldSourceIdentity::procedural_v16(7);
        let base_hash = base.identity_hash();
        let mut mutations = vec![
            {
                let mut value = base.clone();
                value.source_kind = WorldSourceKind::TerrainDiffusion30m;
                value
            },
            {
                let mut value = base.clone();
                value.implementation_version += 1;
                value
            },
            {
                let mut value = base.clone();
                value.configuration_hash = WorldSourceIdentityHash::from_bytes([9; 32]);
                value
            },
            {
                let mut value = base.clone();
                value.model = Some(ModelIdentity {
                    repository: "immutable/model".to_owned(),
                    immutable_revision: "0123456789abcdef".to_owned(),
                    weight_hashes: vec![WorldSourceIdentityHash::from_bytes([3; 32])],
                });
                value
            },
            {
                let mut value = base.clone();
                value.sampler_version += 1;
                value
            },
            {
                let mut value = base.clone();
                value.scheduler_version += 1;
                value
            },
            {
                let mut value = base.clone();
                value.macro_field_schema_version += 1;
                value
            },
            {
                let mut value = base.clone();
                value.macro_coordinate_transform.origin_voxels[0] += 1;
                value
            },
            {
                let mut value = base.clone();
                value.voxel_composer_version += 1;
                value
            },
            {
                let mut value = base.clone();
                value.authored_content_version += 1;
                value
            },
            {
                let mut value = base.clone();
                value.device_requirement = SourceDeviceRequirement::AppleMetal;
                value
            },
        ];
        for index in 0..2 {
            let mut value = base.clone();
            value.macro_coordinate_transform.origin_voxels[index] += 1;
            mutations.push(value);
        }
        let mut value = base.clone();
        value.macro_coordinate_transform.horizontal_unit_millimetres += 1;
        mutations.push(value);
        let mut value = base.clone();
        value.macro_coordinate_transform.x_axis_sign = -1;
        mutations.push(value);
        let mut value = base.clone();
        value.macro_coordinate_transform.z_axis_sign = -1;
        mutations.push(value);

        let model_base = ModelIdentity {
            repository: "immutable/model".to_owned(),
            immutable_revision: "0123456789abcdef".to_owned(),
            weight_hashes: vec![WorldSourceIdentityHash::from_bytes([3; 32])],
        };
        for model in [
            ModelIdentity {
                repository: "immutable/other".to_owned(),
                ..model_base.clone()
            },
            ModelIdentity {
                immutable_revision: "fedcba9876543210".to_owned(),
                ..model_base.clone()
            },
            ModelIdentity {
                weight_hashes: vec![WorldSourceIdentityHash::from_bytes([4; 32])],
                ..model_base
            },
        ] {
            let mut value = base.clone();
            value.model = Some(model);
            mutations.push(value);
        }
        assert!(
            mutations
                .into_iter()
                .all(|identity| identity.identity_hash() != base_hash)
        );
    }

    #[test]
    fn meshing_halo_has_exact_shell_and_random_access() {
        let coord = ChunkCoord::new(-2, 3, 4);
        let origin = coord.world_origin();
        let mut halo = MeshingHalo::from_sampler(coord, |x, y, z| {
            if (x ^ y ^ z) & 1 == 0 {
                Material::Stone
            } else {
                Material::Air
            }
        });
        assert_eq!(halo.voxels().len(), MESHING_HALO_VOXELS);
        assert_eq!(MESHING_HALO_VOXELS, 6_536);
        let mut materialized_index = 0;
        for y in -1..=CHUNK_EDGE as i32 {
            for z in -1..=CHUNK_EDGE as i32 {
                for x in -1..=CHUNK_EDGE as i32 {
                    if is_chunk_core([x, y, z]) {
                        continue;
                    }
                    assert_eq!(meshing_halo_index([x, y, z]), Some(materialized_index));
                    materialized_index += 1;
                }
            }
        }
        assert_eq!(materialized_index, MESHING_HALO_VOXELS);
        for local in [[-1, -1, -1], [32, 32, 32], [-1, 7, 19], [31, 32, 0]] {
            let world = [
                origin[0] + local[0],
                origin[1] + local[1],
                origin[2] + local[2],
            ];
            let expected = if (world[0] ^ world[1] ^ world[2]) & 1 == 0 {
                Material::Stone
            } else {
                Material::Air
            };
            assert_eq!(halo.sample_local(local), Some(expected));
            assert_eq!(
                halo.sample_world(world[0], world[1], world[2]),
                Some(expected)
            );
        }
        assert_eq!(halo.sample_local([0, 0, 0]), None);
        assert_eq!(halo.sample_local([-2, 0, 0]), None);

        for local in [[-1, -1, -1], [32, 32, 32], [-1, 7, 19], [31, 32, 0]] {
            let world = [
                origin[0] + local[0],
                origin[1] + local[1],
                origin[2] + local[2],
            ];
            assert!(halo.set_world(world[0], world[1], world[2], Material::GlowCrystal));
            assert_eq!(halo.sample_local(local), Some(Material::GlowCrystal));
        }
        assert!(!halo.set_world(origin[0], origin[1], origin[2], Material::GlowCrystal));
        assert!(!halo.set_world(origin[0] - 2, origin[1], origin[2], Material::GlowCrystal));

        let mut indices = std::collections::BTreeSet::new();
        for y in -1..=CHUNK_EDGE as i32 {
            for z in -1..=CHUNK_EDGE as i32 {
                for x in -1..=CHUNK_EDGE as i32 {
                    let local = [x, y, z];
                    if !is_chunk_core(local) {
                        assert!(
                            indices
                                .insert(meshing_halo_index(local).unwrap_or(MESHING_HALO_VOXELS))
                        );
                    }
                }
            }
        }
        assert_eq!(indices, (0..MESHING_HALO_VOXELS).collect());
    }

    #[test]
    fn product_batch_keys_each_result_and_preserves_identity() {
        let source = ProceduralWorldSource::new(0x5eed_cafe);
        let chunk_coord = ChunkCoord::new(2, 0, -3);
        let surface_coord = SurfaceTileCoord::containing(crate::SurfaceLodLevel::Stride8, 0, 0);
        let result = source
            .generate_batch(WorldProductBatch {
                priority: WorldProductPriority::VisibleChunk,
                requests: vec![
                    WorldProductRequest::ChunkWithHalo(chunk_coord),
                    WorldProductRequest::SurfaceTile(surface_coord),
                ],
            })
            .expect("procedural products are infallible");
        assert_eq!(result.source_identity_hash, source.source_identity_hash());
        assert!(matches!(
            &result.items[0],
            WorldProductBatchItem {
                request: WorldProductRequest::ChunkWithHalo(request_coord),
                result: Ok(WorldProduct::Chunk(snapshot)),
            }
                if *request_coord == chunk_coord
                    && snapshot.chunk == source.generator.generate_chunk(chunk_coord)
        ));
        let WorldProductBatchItem {
            request: WorldProductRequest::SurfaceTile(request_coord),
            result: Ok(WorldProduct::SurfaceTile(snapshot)),
        } = &result.items[1]
        else {
            assert_eq!(
                result.items.len(),
                0,
                "second product must be a surface tile"
            );
            return;
        };
        assert_eq!(*request_coord, surface_coord);
        assert_eq!(
            snapshot.terrain,
            generate_surface_tile_mesh(source.generator, surface_coord)
        );
        assert_eq!(
            snapshot.water,
            generate_water_tile_mesh_with(surface_coord, |x, z| {
                source.generator.surface_sample(x, z).water_level == Some(crate::SEA_LEVEL_VOXELS)
            })
        );
    }

    #[test]
    fn bounded_probe_products_match_procedural_sampling_and_fail_independently() {
        let source = ProceduralWorldSource::new(0x5eed_cafe);
        let voxel_request = VoxelBlockRequest {
            min: VoxelCoord::new(-3, 29, 7),
            sample_shape: [2, 3, 2],
        };
        let surface_request = SurfaceSampleBlockRequest {
            origin: [-3, 7],
            sample_shape: [2, 2],
        };
        let invalid_request = VoxelBlockRequest {
            min: VoxelCoord::new(i32::MAX, 0, 0),
            sample_shape: [2, 1, 1],
        };
        let result = source
            .generate_batch(WorldProductBatch {
                priority: WorldProductPriority::CollisionCritical,
                requests: vec![
                    WorldProductRequest::VoxelBlock(voxel_request),
                    WorldProductRequest::VoxelBlock(invalid_request),
                    WorldProductRequest::SurfaceSampleBlock(surface_request),
                ],
            })
            .expect("a bad item does not discard useful batch results");
        assert_eq!(result.items.len(), 3);

        let Ok(WorldProduct::VoxelBlock(voxels)) = &result.items[0].result else {
            panic!("first result must be the requested voxel block");
        };
        assert_eq!(voxels.materials().len(), 12);
        for z in 7..9 {
            for y in 29..32 {
                for x in -3..-1 {
                    assert_eq!(
                        voxels.sample(VoxelCoord::new(x, y, z)),
                        Some(source.generator.sample(x, y, z))
                    );
                }
            }
        }
        assert_eq!(
            result.items[1].result,
            Err(WorldSourceError::InvalidBlockCoordinate)
        );

        let Ok(WorldProduct::SurfaceSampleBlock(samples)) = &result.items[2].result else {
            panic!("third result must be the requested surface block");
        };
        assert_eq!(samples.samples().len(), 4);
        for z in 7..9 {
            for x in -3..-1 {
                assert_eq!(
                    samples.sample(x, z),
                    Some(source.generator.surface_sample(x, z))
                );
            }
        }
    }

    #[test]
    fn bounded_surface_search_preserves_ring_order_and_validates_limits() {
        let source = ProceduralWorldSource::new(0x5eed_cafe);
        let dry = SurfaceSearchRequest {
            origin: [18_016, 12_896],
            min_radius: 1,
            max_radius: 192,
            kind: SurfaceSearchKind::DryLand,
        };
        let deep = SurfaceSearchRequest {
            origin: [18_016, 12_896],
            min_radius: 0,
            max_radius: 256,
            kind: SurfaceSearchKind::WaterDepthAtLeast { depth_voxels: 18 },
        };
        let result = source
            .generate_batch(WorldProductBatch {
                priority: WorldProductPriority::CollisionCritical,
                requests: vec![
                    WorldProductRequest::SurfaceSearch(dry),
                    WorldProductRequest::SurfaceSearch(deep),
                    WorldProductRequest::SurfaceSearch(SurfaceSearchRequest {
                        max_radius: MAX_SURFACE_SEARCH_RADIUS + 1,
                        ..dry
                    }),
                ],
            })
            .expect("search item errors remain independent");
        let expected = |request: SurfaceSearchRequest| {
            let matches = |sample: SurfaceSample| match request.kind {
                SurfaceSearchKind::DryLand => sample.water_level.is_none(),
                SurfaceSearchKind::WaterDepthAtLeast { depth_voxels } => {
                    assert_eq!(depth_voxels, 18);
                    sample.water_level.is_some_and(|water| {
                        let eye_y = (sample.height + 1) as f32 * 0.1 + 1.62 + 0.04;
                        let water_top = (water + 1) as f32 * 0.1;
                        eye_y <= water_top - 0.12
                    })
                }
            };
            for radius in i32::from(request.min_radius)..=i32::from(request.max_radius) {
                let min_x = request.origin[0] - radius;
                let max_x = request.origin[0] + radius;
                let min_z = request.origin[1] - radius;
                let max_z = request.origin[1] + radius;
                for x in min_x..=max_x {
                    for z in [min_z, max_z] {
                        let sample = source.generator.surface_sample(x, z);
                        if matches(sample) {
                            return Some(SurfaceSearchHit {
                                coord: [x, z],
                                sample,
                            });
                        }
                    }
                }
                for z in min_z.saturating_add(1)..max_z {
                    for x in [min_x, max_x] {
                        let sample = source.generator.surface_sample(x, z);
                        if matches(sample) {
                            return Some(SurfaceSearchHit {
                                coord: [x, z],
                                sample,
                            });
                        }
                    }
                }
            }
            None
        };
        for (index, request) in [dry, deep].into_iter().enumerate() {
            let Ok(WorldProduct::SurfaceSearch(snapshot)) = &result.items[index].result else {
                panic!("valid search must return a search product");
            };
            assert_eq!(snapshot.request, request);
            assert_eq!(snapshot.hit, expected(request));
        }
        assert_eq!(
            result.items[2].result,
            Err(WorldSourceError::SearchRadiusTooLarge)
        );
        assert_eq!(
            source.surface_search(SurfaceSearchRequest {
                min_radius: 2,
                max_radius: 1,
                ..dry
            }),
            Err(WorldSourceError::InvalidSearchRadius)
        );
    }

    #[test]
    fn procedural_chunk_product_halo_matches_random_access_source() {
        let source = ProceduralWorldSource::new(0x5eed_cafe);
        let coord = ChunkCoord::new(-7, 1, -41);
        let origin = coord.world_origin();
        let snapshot = source.chunk_with_halo(coord);
        for y in -1..=CHUNK_EDGE as i32 {
            for z in -1..=CHUNK_EDGE as i32 {
                for x in -1..=CHUNK_EDGE as i32 {
                    let local = [x, y, z];
                    if is_chunk_core(local) {
                        continue;
                    }
                    assert_eq!(
                        snapshot.meshing_halo.sample_local(local),
                        Some(
                            source
                                .generator
                                .sample(origin[0] + x, origin[1] + y, origin[2] + z),
                        )
                    );
                }
            }
        }

        let minimum = VoxelCoord::new(i32::MIN, 0, 0).chunk();
        let maximum = VoxelCoord::new(i32::MAX, 0, 0).chunk();
        assert_eq!(
            source
                .chunk_with_halo(minimum)
                .meshing_halo
                .sample_local([-1, 0, 0]),
            Some(Material::Air)
        );
        assert_eq!(
            source
                .chunk_with_halo(maximum)
                .meshing_halo
                .sample_local([CHUNK_EDGE as i32, 0, 0]),
            Some(Material::Air)
        );
    }

    #[test]
    fn procedural_macro_block_exposes_versioned_fields() {
        let source = ProceduralWorldSource::new(9);
        let result = source
            .request_blocks(MacroBlockBatch {
                priority: WorldProductPriority::Prefetch,
                requests: vec![MacroBlockRequest {
                    origin: [-10, 20],
                    sample_shape: [3, 2],
                    stride_voxels: 4,
                }],
            })
            .expect("small procedural macro block is valid");
        assert_eq!(result.blocks[0].elevation_voxels.len(), 6);
        assert_eq!(result.blocks[0].validity, vec![true; 6]);
        assert_eq!(result.blocks[0].schema_version, MACRO_FIELD_SCHEMA_VERSION);
    }

    #[test]
    fn source_rejects_unrepresentable_and_unbounded_requests() {
        let source = ProceduralWorldSource::new(9);
        let invalid_chunk = ChunkCoord::new(i32::MAX, 0, 0);
        let invalid_chunks = source
            .generate_batch(WorldProductBatch {
                priority: WorldProductPriority::Prefetch,
                requests: vec![WorldProductRequest::ChunkWithHalo(invalid_chunk)],
            })
            .expect("one invalid item does not fail its whole batch");
        assert_eq!(
            invalid_chunks.items[0].result,
            Err(WorldSourceError::InvalidChunkCoordinate)
        );
        let invalid_surfaces = source
            .generate_batch(WorldProductBatch {
                priority: WorldProductPriority::Prefetch,
                requests: vec![WorldProductRequest::SurfaceTile(SurfaceTileCoord::new(
                    SurfaceLodLevel::Stride16,
                    i32::MAX,
                    0,
                ))],
            })
            .expect("one invalid item does not fail its whole batch");
        assert_eq!(
            invalid_surfaces.items[0].result,
            Err(WorldSourceError::InvalidSurfaceTileCoordinate)
        );
        assert_eq!(
            source.generate_edited_surface_tile(
                &crate::EditMap::default(),
                SurfaceTileCoord::new(SurfaceLodLevel::Stride16, i32::MAX, 0),
            ),
            Err(WorldSourceError::InvalidSurfaceTileCoordinate)
        );
        assert_eq!(
            source.request_blocks(MacroBlockBatch {
                priority: WorldProductPriority::Prefetch,
                requests: vec![MacroBlockRequest {
                    origin: [0, 0],
                    sample_shape: [2_048, 2_048],
                    stride_voxels: 1,
                }],
            }),
            Err(WorldSourceError::MacroBlockTooLarge)
        );
    }
}
