//! Versioned virtual microvoxel terrain pages.
//!
//! Pages retain canonical 10 cm integer ownership while allowing different payload encodings.
//! Representation is never ownership: every page key identifies one half-open spatial owner, and
//! every payload reconstructs only that owner's certified surface.

use crate::{
    BoundaryCertificate, BoundarySide, CanonicalFaceKey, FaceAxis, Material, SurfaceSample,
    VoxelBounds, VoxelCoord, WorldSourceIdentityHash, canonical_exposed_faces,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::Read;

#[cfg(feature = "terrain-page-builder")]
use crate::terrain_error::certify_bidirectional_surface_error;

pub const TERRAIN_PAGE_SCHEMA_VERSION: u16 = 4;
pub const TERRAIN_PAGE_EDGE_SAMPLES: u32 = 32;
pub const TERRAIN_PAGE_MAX_LEVEL: u8 = 20;
pub const TERRAIN_PAGE_MAX_CHILDREN: usize = 8;
pub const TERRAIN_SURFACE_PAGE_CHILDREN: usize = 4;
pub const TERRAIN_SURFACE_PAGE_COORD_Y: i32 = i32::MIN;
/// Builder target selected from the 12.8–51.2 m Terrain Diffusion sizing corpus.
///
/// Exact pages may temporarily exceed this while an ancestor is being simplified, but published
/// production pages should stay at or below this target. The larger hard cap exists only so a
/// valid parent is never discarded before a replacement is available.
pub const TERRAIN_PAGE_TARGET_COMPRESSED_BYTES: usize = 65_536;
pub const TERRAIN_PAGE_MAX_COMPRESSED_BYTES: usize = 262_144;
pub const TERRAIN_PAGE_MAX_PAYLOAD_BYTES: usize = 2_097_152;
const SPARSE_BRICK_EDGE: u8 = 8;
const PAGE_FINGERPRINT_DOMAIN: &[u8] = b"voxels-terrain-page-v4\0";
const PARENT_BOUNDARY_DOMAIN: &[u8] = b"voxels-terrain-parent-boundary-v1\0";
const HEIGHTFIELD_BOUNDARY_DOMAIN: &[u8] = b"voxels-terrain-heightfield-boundary-v1\0";
const PAGE_MAGIC: &[u8; 4] = b"VXTP";
const PAGE_HEADER_LEN: u16 = 344;
const PAGE_COMPRESSION_BROTLI: u8 = 1;
const PAGE_BROTLI_QUALITY: i32 = 5;
const PAGE_BROTLI_WINDOW_BITS: i32 = 20;
const PAGE_BROTLI_BUFFER_BYTES: usize = 16_384;
#[cfg(feature = "terrain-page-builder")]
const SURFACE_ERROR_CERTIFICATE_MAX_SPACING_MILLIVOXELS: u32 = 250;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TerrainPageKey {
    pub level: u8,
    pub coord: [i32; 3],
}

impl TerrainPageKey {
    pub const fn surface(level: u8, x: i32, z: i32) -> Self {
        Self {
            level,
            coord: [x, TERRAIN_SURFACE_PAGE_COORD_Y, z],
        }
    }

    pub const fn is_surface(self) -> bool {
        self.coord[1] == TERRAIN_SURFACE_PAGE_COORD_Y
    }

    pub fn horizontal_bounds(self) -> Option<[[i32; 2]; 2]> {
        if self.level > TERRAIN_PAGE_MAX_LEVEL {
            return None;
        }
        let span = i64::from(TERRAIN_PAGE_EDGE_SAMPLES).checked_shl(u32::from(self.level))?;
        let minimum_x = i64::from(self.coord[0]).checked_mul(span)?;
        let minimum_z = i64::from(self.coord[2]).checked_mul(span)?;
        let maximum_x = minimum_x.checked_add(span)?;
        let maximum_z = minimum_z.checked_add(span)?;
        Some([
            [
                i32::try_from(minimum_x).ok()?,
                i32::try_from(minimum_z).ok()?,
            ],
            [
                i32::try_from(maximum_x).ok()?,
                i32::try_from(maximum_z).ok()?,
            ],
        ])
    }

    pub fn bounds(self) -> Option<VoxelBounds> {
        if self.level > TERRAIN_PAGE_MAX_LEVEL || self.is_surface() {
            return None;
        }
        let span = i64::from(TERRAIN_PAGE_EDGE_SAMPLES).checked_shl(u32::from(self.level))?;
        let minimum = self.coord.map(|component| i64::from(component) * span);
        let maximum = minimum.map(|component| component + span);
        let minimum = VoxelCoord::new(
            i32::try_from(minimum[0]).ok()?,
            i32::try_from(minimum[1]).ok()?,
            i32::try_from(minimum[2]).ok()?,
        );
        let maximum = VoxelCoord::new(
            i32::try_from(maximum[0]).ok()?,
            i32::try_from(maximum[1]).ok()?,
            i32::try_from(maximum[2]).ok()?,
        );
        VoxelBounds::new(minimum, maximum)
    }

    pub fn parent(self) -> Option<Self> {
        (self.level < TERRAIN_PAGE_MAX_LEVEL).then(|| Self {
            level: self.level + 1,
            coord: if self.is_surface() {
                [
                    self.coord[0].div_euclid(2),
                    TERRAIN_SURFACE_PAGE_COORD_Y,
                    self.coord[2].div_euclid(2),
                ]
            } else {
                self.coord.map(|component| component.div_euclid(2))
            },
        })
    }

    pub fn ancestor_at(self, level: u8) -> Option<Self> {
        if level < self.level || level > TERRAIN_PAGE_MAX_LEVEL {
            return None;
        }
        let scale = 1i32.checked_shl(u32::from(level - self.level))?;
        Some(Self {
            level,
            coord: if self.is_surface() {
                [
                    self.coord[0].div_euclid(scale),
                    TERRAIN_SURFACE_PAGE_COORD_Y,
                    self.coord[2].div_euclid(scale),
                ]
            } else {
                self.coord.map(|component| component.div_euclid(scale))
            },
        })
    }

    pub fn children(self) -> Option<[Self; 8]> {
        if self.is_surface() {
            return None;
        }
        let child_level = self.level.checked_sub(1)?;
        Some(std::array::from_fn(|index| Self {
            level: child_level,
            coord: [
                self.coord[0].saturating_mul(2) + (index & 1) as i32,
                self.coord[1].saturating_mul(2) + ((index >> 1) & 1) as i32,
                self.coord[2].saturating_mul(2) + ((index >> 2) & 1) as i32,
            ],
        }))
    }

    pub fn refinement_children(self) -> Option<Vec<Self>> {
        let child_level = self.level.checked_sub(1)?;
        if self.is_surface() {
            let base_x = self.coord[0].checked_mul(2)?;
            let base_z = self.coord[2].checked_mul(2)?;
            return Some(
                (0..TERRAIN_SURFACE_PAGE_CHILDREN)
                    .map(|index| {
                        Self::surface(
                            child_level,
                            base_x + (index & 1) as i32,
                            base_z + ((index >> 1) & 1) as i32,
                        )
                    })
                    .collect(),
            );
        }
        self.children().map(Vec::from)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerrainPageChild {
    pub key: TerrainPageKey,
    pub revision: u64,
    pub content_fingerprint: [u8; 32],
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TerrainErrorBounds {
    /// Fixed-point thousandths of a canonical voxel.
    pub geometric_millivoxels: u32,
    pub silhouette_millivoxels: u32,
    pub material_boundary_millivoxels: u32,
    /// Fixed-point thousandths of a radian.
    pub normal_milliradians: u32,
    /// An unresolved opening or ownership ambiguity has infinite selection error.
    pub unresolved_topology: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerrainSimplificationBudget {
    pub target_triangles: u32,
    pub max_error_millivoxels: u32,
    pub target_encoded_bytes: u32,
}

impl TerrainErrorBounds {
    pub const EXACT: Self = Self {
        geometric_millivoxels: 0,
        silhouette_millivoxels: 0,
        material_boundary_millivoxels: 0,
        normal_milliradians: 0,
        unresolved_topology: false,
    };
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerrainTopologyClass {
    SingleRunColumns = 0,
    Volumetric = 1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerrainMaterialCoverage {
    pub material: Material,
    pub occupied_voxels: u32,
    pub exposed_unit_faces: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerrainSurfaceQuad {
    pub axis: FaceAxis,
    pub plane: i32,
    pub u: i32,
    pub v: i32,
    pub width: u16,
    pub height: u16,
    pub positive: bool,
    pub material_index: u8,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TerrainClusterVertex {
    /// Signed canonical 10 cm lattice coordinate.
    pub position: [i32; 3],
    /// Duplicated at material seams so simplification cannot move the boundary.
    pub material_index: u8,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TerrainClusterTriangle {
    pub vertices: [u32; 3],
    pub material_index: u8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerrainTriangleCluster {
    pub vertices: Vec<TerrainClusterVertex>,
    pub triangles: Vec<TerrainClusterTriangle>,
}

/// Compact, independently renderable single-valued terrain reconstruction.
///
/// The payload stores only 33x33 source samples for a 32-cell page. Rendering interpolates between
/// signed canonical 10 cm lattice positions, so coarse source data never appears as macro cubes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerrainHeightfieldGrid {
    pub sample_stride_voxels: u32,
    pub shape_xz: [u16; 2],
    /// Canonical Y plane coordinates (`solid surface height + 1`), X fastest then Z.
    pub ground_heights: Vec<i32>,
    /// One material palette index per 32x32 cell, X fastest then Z.
    pub cell_material_indices: Vec<u8>,
    /// Canonical water top plane coordinates, or `i32::MIN` for a dry lattice vertex.
    pub water_heights: Vec<i32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerrainColumn {
    pub first_run: u32,
    pub run_count: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerrainMaterialRun {
    pub minimum_y: i32,
    pub length: u16,
    pub material_index: u8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SteppedSurfaceResidual {
    pub sample_stride_voxels: u32,
    pub shape_xz: [u16; 2],
    pub columns: Vec<TerrainColumn>,
    pub runs: Vec<TerrainMaterialRun>,
    /// Exact exposed faces on the six half-open page planes.
    ///
    /// Interior faces are reconstructed from the compact column occupancy. Boundary hashes prove
    /// replacement coherence but cannot recover this geometry, so the independently renderable
    /// page must carry the sparse boundary surface explicitly.
    pub boundary_quads: Vec<TerrainSurfaceQuad>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerrainSparseBrick {
    pub local_brick: [u8; 3],
    pub occupancy: [u64; 8],
    pub material_indices: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SparseVoxelBrickPayload {
    pub brick_edge: u8,
    pub bricks: Vec<TerrainSparseBrick>,
    /// Exact exposed faces on the six half-open page planes.
    ///
    /// Faces wholly inside the page are reconstructed from brick occupancy. Keeping only these
    /// boundary faces avoids both invented page walls and dependence on a neighboring page being
    /// resident.
    pub boundary_quads: Vec<TerrainSurfaceQuad>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerrainPageRepresentation {
    SteppedSurfaceResidual(SteppedSurfaceResidual),
    SparseVoxelBrick(SparseVoxelBrickPayload),
    SurfaceCluster(Vec<TerrainSurfaceQuad>),
    TriangleCluster(TerrainTriangleCluster),
    HeightfieldGrid(TerrainHeightfieldGrid),
}

impl TerrainPageRepresentation {
    pub const fn kind(&self) -> TerrainPageRepresentationKind {
        match self {
            Self::SteppedSurfaceResidual(_) => {
                TerrainPageRepresentationKind::SteppedSurfaceResidual
            }
            Self::SparseVoxelBrick(_) => TerrainPageRepresentationKind::SparseVoxelBrick,
            Self::SurfaceCluster(_) => TerrainPageRepresentationKind::SurfaceCluster,
            Self::TriangleCluster(_) => TerrainPageRepresentationKind::TriangleCluster,
            Self::HeightfieldGrid(_) => TerrainPageRepresentationKind::HeightfieldGrid,
        }
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerrainPageRepresentationKind {
    SteppedSurfaceResidual = 1,
    SparseVoxelBrick = 2,
    SurfaceCluster = 3,
    TriangleCluster = 4,
    HeightfieldGrid = 5,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerrainPageV1 {
    pub source_identity_hash: WorldSourceIdentityHash,
    pub key: TerrainPageKey,
    pub revision: u64,
    pub bounds: VoxelBounds,
    pub children: Vec<TerrainPageChild>,
    pub errors: TerrainErrorBounds,
    pub topology: TerrainTopologyClass,
    pub boundary_fingerprints: [[u8; 32]; 6],
    pub materials: Vec<TerrainMaterialCoverage>,
    pub representation: TerrainPageRepresentation,
    pub content_fingerprint: [u8; 32],
}

impl TerrainPageV1 {
    pub fn validates_identity(&self) -> bool {
        page_bounds_match_key(self.key, self.bounds)
            && (self.children.is_empty()
                || self.children.len()
                    == if self.key.is_surface() {
                        TERRAIN_SURFACE_PAGE_CHILDREN
                    } else {
                        TERRAIN_PAGE_MAX_CHILDREN
                    })
            && children_are_complete(self)
            && self.materials.len() <= Material::ALL.len()
            && self
                .materials
                .windows(2)
                .all(|pair| pair[0].material.id() < pair[1].material.id())
            && self
                .materials
                .iter()
                .all(|coverage| coverage.material.is_renderable())
            && representation_is_valid(self)
            && self.content_fingerprint == terrain_page_fingerprint(self)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerrainPageBuildError {
    NotExactLeaf,
    InvalidPageKey,
    InvalidSurfaceLattice,
    InvalidChildGroup(TerrainReplacementError),
    UnsupportedChildRepresentation,
    NoSurfaceToSimplify,
    NonManifoldSurface,
    OverlappingSurface,
    OpenInteriorSurface,
    InvalidSimplification,
    SimplificationTargetNotReached,
    SamplingBoundsOverflow,
    MaterialPaletteOverflow,
    PayloadOverflow,
}

impl fmt::Display for TerrainPageBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotExactLeaf => formatter.write_str("exact page builder requires a level-0 key"),
            Self::InvalidPageKey => formatter.write_str("terrain page key has no valid bounds"),
            Self::InvalidSurfaceLattice => {
                formatter.write_str("terrain heightfield lattice is invalid for its page")
            }
            Self::InvalidChildGroup(error) => {
                write!(formatter, "terrain parent child group is invalid: {error}")
            }
            Self::UnsupportedChildRepresentation => {
                formatter.write_str("exact terrain parent requires clustered child surfaces")
            }
            Self::NoSurfaceToSimplify => {
                formatter.write_str("terrain parent has no surface to simplify")
            }
            Self::NonManifoldSurface => {
                formatter.write_str("terrain surface is not a simplifiable two-manifold")
            }
            Self::OverlappingSurface => {
                formatter.write_str("terrain surface contains overlapping triangles")
            }
            Self::OpenInteriorSurface => {
                formatter.write_str("terrain surface contains an interior open edge")
            }
            Self::InvalidSimplification => {
                formatter.write_str("terrain simplifier produced an invalid surface")
            }
            Self::SimplificationTargetNotReached => {
                formatter.write_str("terrain simplifier could not reach the page budget")
            }
            Self::SamplingBoundsOverflow => {
                formatter.write_str("terrain page halo exceeds canonical coordinates")
            }
            Self::MaterialPaletteOverflow => {
                formatter.write_str("terrain page material palette exceeds u8 indexing")
            }
            Self::PayloadOverflow => formatter.write_str("terrain page payload exceeds hard limit"),
        }
    }
}

impl std::error::Error for TerrainPageBuildError {}

impl From<TerrainReplacementError> for TerrainPageBuildError {
    fn from(error: TerrainReplacementError) -> Self {
        Self::InvalidChildGroup(error)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerrainPageReconstructionError {
    InvalidPage,
    UnsupportedRepresentation,
    NonUnitStride,
}

impl fmt::Display for TerrainPageReconstructionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPage => formatter.write_str("terrain page cannot be reconstructed"),
            Self::UnsupportedRepresentation => {
                formatter.write_str("terrain page representation is not an exact occupancy payload")
            }
            Self::NonUnitStride => {
                formatter.write_str("terrain occupancy reconstruction requires unit voxel stride")
            }
        }
    }
}

impl std::error::Error for TerrainPageReconstructionError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerrainReplacementError {
    InvalidParent,
    WrongChildCount,
    InvalidChild,
    SourceMismatch,
    IncompleteChildKeys,
    ChildReferenceMismatch,
    InternalBoundaryMismatch,
    OuterBoundaryMismatch,
    InvalidRepresentation,
}

impl fmt::Display for TerrainReplacementError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidParent => formatter.write_str("replacement parent is invalid"),
            Self::WrongChildCount => formatter.write_str("replacement requires exactly 8 children"),
            Self::InvalidChild => formatter.write_str("replacement child is invalid"),
            Self::SourceMismatch => formatter.write_str("replacement source identities differ"),
            Self::IncompleteChildKeys => {
                formatter.write_str("replacement child keys do not complete the parent")
            }
            Self::ChildReferenceMismatch => {
                formatter.write_str("replacement child revision or fingerprint differs")
            }
            Self::InternalBoundaryMismatch => {
                formatter.write_str("replacement child boundaries do not cancel")
            }
            Self::OuterBoundaryMismatch => {
                formatter.write_str("replacement outer boundary differs from the parent")
            }
            Self::InvalidRepresentation => {
                formatter.write_str("replacement parent representation is invalid")
            }
        }
    }
}

impl std::error::Error for TerrainReplacementError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerrainPageCodecError {
    Truncated,
    InvalidMagic,
    UnsupportedVersion(u16),
    InvalidHeader(&'static str),
    SourceIdentityMismatch,
    UnknownMaterial(u16),
    InvalidRepresentation(&'static str),
    LimitExceeded(&'static str),
    Compression,
    CorruptHash,
}

impl fmt::Display for TerrainPageCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => formatter.write_str("truncated VXTP payload"),
            Self::InvalidMagic => formatter.write_str("invalid VXTP magic"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported VXTP version {version}")
            }
            Self::InvalidHeader(reason) => write!(formatter, "invalid VXTP header: {reason}"),
            Self::SourceIdentityMismatch => formatter.write_str("VXTP source identity mismatch"),
            Self::UnknownMaterial(id) => write!(formatter, "unknown VXTP material id {id}"),
            Self::InvalidRepresentation(reason) => {
                write!(formatter, "invalid VXTP representation: {reason}")
            }
            Self::LimitExceeded(limit) => write!(formatter, "VXTP limit exceeded: {limit}"),
            Self::Compression => formatter.write_str("VXTP Brotli payload is invalid"),
            Self::CorruptHash => formatter.write_str("VXTP semantic content hash mismatch"),
        }
    }
}

impl std::error::Error for TerrainPageCodecError {}

struct ExactPageSamples {
    halo_min: VoxelCoord,
    halo_edge: usize,
    materials: Vec<Material>,
}

impl ExactPageSamples {
    fn sample(&self, coord: VoxelCoord) -> Material {
        let local = [
            i64::from(coord.x) - i64::from(self.halo_min.x),
            i64::from(coord.y) - i64::from(self.halo_min.y),
            i64::from(coord.z) - i64::from(self.halo_min.z),
        ];
        if local
            .iter()
            .any(|component| *component < 0 || *component >= self.halo_edge as i64)
        {
            return Material::Air;
        }
        let x = local[0] as usize;
        let y = local[1] as usize;
        let z = local[2] as usize;
        self.materials[x + y * self.halo_edge + z * self.halo_edge * self.halo_edge]
    }
}

/// Builds an exact 32³ leaf from canonical occupancy plus a one-voxel halo.
pub fn build_exact_terrain_page(
    source_identity_hash: WorldSourceIdentityHash,
    key: TerrainPageKey,
    revision: u64,
    material_at: impl FnMut(VoxelCoord) -> Material,
) -> Result<TerrainPageV1, TerrainPageBuildError> {
    build_exact_terrain_page_with_policy(
        source_identity_hash,
        key,
        revision,
        material_at,
        TerrainLeafPolicy::Clustered,
    )
}

/// Builds the same exact owner while choosing the smallest raw payload among legal encodings.
/// This is useful for producer experiments; the production hierarchy begins with clustered leaves
/// so exact parents can be formed without recovering occupancy from a surface-only payload.
pub fn build_compact_exact_terrain_page(
    source_identity_hash: WorldSourceIdentityHash,
    key: TerrainPageKey,
    revision: u64,
    material_at: impl FnMut(VoxelCoord) -> Material,
) -> Result<TerrainPageV1, TerrainPageBuildError> {
    build_exact_terrain_page_with_policy(
        source_identity_hash,
        key,
        revision,
        material_at,
        TerrainLeafPolicy::Compact,
    )
}

#[derive(Clone, Copy)]
enum TerrainLeafPolicy {
    Clustered,
    Compact,
}

fn build_exact_terrain_page_with_policy(
    source_identity_hash: WorldSourceIdentityHash,
    key: TerrainPageKey,
    revision: u64,
    mut material_at: impl FnMut(VoxelCoord) -> Material,
    policy: TerrainLeafPolicy,
) -> Result<TerrainPageV1, TerrainPageBuildError> {
    if key.level != 0 {
        return Err(TerrainPageBuildError::NotExactLeaf);
    }
    let bounds = key.bounds().ok_or(TerrainPageBuildError::InvalidPageKey)?;
    let halo_min = VoxelCoord::new(
        bounds
            .min
            .x
            .checked_sub(1)
            .ok_or(TerrainPageBuildError::SamplingBoundsOverflow)?,
        bounds
            .min
            .y
            .checked_sub(1)
            .ok_or(TerrainPageBuildError::SamplingBoundsOverflow)?,
        bounds
            .min
            .z
            .checked_sub(1)
            .ok_or(TerrainPageBuildError::SamplingBoundsOverflow)?,
    );
    let halo_max = VoxelCoord::new(
        bounds
            .max
            .x
            .checked_add(1)
            .ok_or(TerrainPageBuildError::SamplingBoundsOverflow)?,
        bounds
            .max
            .y
            .checked_add(1)
            .ok_or(TerrainPageBuildError::SamplingBoundsOverflow)?,
        bounds
            .max
            .z
            .checked_add(1)
            .ok_or(TerrainPageBuildError::SamplingBoundsOverflow)?,
    );
    let halo_edge = TERRAIN_PAGE_EDGE_SAMPLES as usize + 2;
    let mut sampled = Vec::with_capacity(halo_edge * halo_edge * halo_edge);
    for z in halo_min.z..halo_max.z {
        for y in halo_min.y..halo_max.y {
            for x in halo_min.x..halo_max.x {
                sampled.push(material_at(VoxelCoord::new(x, y, z)));
            }
        }
    }
    let samples = ExactPageSamples {
        halo_min,
        halo_edge,
        materials: sampled,
    };
    let faces = canonical_exposed_faces(bounds, |coord| samples.sample(coord));
    let certificate = BoundaryCertificate::build(bounds, |coord| samples.sample(coord));
    let boundary_fingerprints =
        std::array::from_fn(|index| certificate.side(BoundarySide::ALL[index]).fingerprint);
    let (materials, palette_indices) = material_coverage(bounds, &samples, &faces)?;
    let clusters = merge_surface_faces(&faces, &palette_indices);
    let boundary_quads = clusters
        .iter()
        .copied()
        .filter(|quad| surface_quad_is_on_boundary(*quad, bounds))
        .collect::<Vec<_>>();
    let (mut stepped, topology) = build_stepped(bounds, &samples, &palette_indices)?;
    stepped.boundary_quads.clone_from(&boundary_quads);
    let sparse = build_sparse(bounds, &samples, &palette_indices);
    let cluster_payload = TerrainPageRepresentation::SurfaceCluster(clusters);
    let representation = if matches!(policy, TerrainLeafPolicy::Clustered) {
        cluster_payload
    } else if topology == TerrainTopologyClass::SingleRunColumns {
        let stepped_payload = TerrainPageRepresentation::SteppedSurfaceResidual(stepped);
        if representation_bytes(&cluster_payload).len()
            <= representation_bytes(&stepped_payload).len()
        {
            cluster_payload
        } else {
            stepped_payload
        }
    } else {
        let sparse_payload = TerrainPageRepresentation::SparseVoxelBrick(SparseVoxelBrickPayload {
            brick_edge: SPARSE_BRICK_EDGE,
            bricks: sparse,
            boundary_quads,
        });
        if representation_bytes(&cluster_payload).len()
            <= representation_bytes(&sparse_payload).len()
        {
            cluster_payload
        } else {
            sparse_payload
        }
    };
    if representation_bytes(&representation).len() > TERRAIN_PAGE_MAX_PAYLOAD_BYTES {
        return Err(TerrainPageBuildError::PayloadOverflow);
    }
    let mut page = TerrainPageV1 {
        source_identity_hash,
        key,
        revision,
        bounds,
        children: Vec::new(),
        errors: TerrainErrorBounds::EXACT,
        topology,
        boundary_fingerprints,
        materials,
        representation,
        content_fingerprint: [0; 32],
    };
    page.content_fingerprint = terrain_page_fingerprint(&page);
    Ok(page)
}

/// Builds a compact coarse surface page from a 33x33 regular source lattice.
///
/// This is a graceful-degradation owner, not an exact occupancy page. It remains a heightfield
/// only when the source has already certified single-valued topology; caves and overhangs use the
/// sparse/cluster path instead.
pub fn build_sampled_heightfield_terrain_page(
    source_identity_hash: WorldSourceIdentityHash,
    key: TerrainPageKey,
    revision: u64,
    samples: &[SurfaceSample],
    mut errors: TerrainErrorBounds,
) -> Result<TerrainPageV1, TerrainPageBuildError> {
    let stride = 1u32
        .checked_shl(u32::from(key.level))
        .ok_or(TerrainPageBuildError::InvalidPageKey)?;
    let shape = TERRAIN_PAGE_EDGE_SAMPLES + 1;
    let sample_count = usize::try_from(shape)
        .ok()
        .and_then(|edge| edge.checked_mul(edge))
        .ok_or(TerrainPageBuildError::InvalidSurfaceLattice)?;
    if samples.len() != sample_count {
        return Err(TerrainPageBuildError::InvalidSurfaceLattice);
    }
    let bounds = if key.is_surface() {
        let [[minimum_x, minimum_z], [maximum_x, maximum_z]] = key
            .horizontal_bounds()
            .ok_or(TerrainPageBuildError::InvalidPageKey)?;
        let mut minimum_y = i32::MAX;
        let mut maximum_y = i32::MIN;
        for sample in samples {
            let ground = sample
                .height
                .checked_add(1)
                .ok_or(TerrainPageBuildError::InvalidSurfaceLattice)?;
            minimum_y = minimum_y.min(ground);
            maximum_y = maximum_y.max(ground).max(
                sample
                    .water_level
                    .and_then(|height| height.checked_add(1))
                    .unwrap_or(ground),
            );
        }
        VoxelBounds::new(
            VoxelCoord::new(minimum_x, minimum_y.saturating_sub(1), minimum_z),
            VoxelCoord::new(maximum_x, maximum_y.saturating_add(1), maximum_z),
        )
        .ok_or(TerrainPageBuildError::InvalidSurfaceLattice)?
    } else {
        key.bounds().ok_or(TerrainPageBuildError::InvalidPageKey)?
    };

    let mut ground_heights = Vec::with_capacity(sample_count);
    let mut water_heights = Vec::with_capacity(sample_count);
    for sample in samples {
        if !sample.material.is_renderable() {
            return Err(TerrainPageBuildError::InvalidSurfaceLattice);
        }
        let ground = sample
            .height
            .checked_add(1)
            .ok_or(TerrainPageBuildError::InvalidSurfaceLattice)?;
        if !(bounds.min.y..=bounds.max.y).contains(&ground) {
            return Err(TerrainPageBuildError::InvalidSurfaceLattice);
        }
        let water = sample
            .water_level
            .map(|height| {
                height
                    .checked_add(1)
                    .filter(|height| {
                        *height >= ground && (bounds.min.y..=bounds.max.y).contains(height)
                    })
                    .ok_or(TerrainPageBuildError::InvalidSurfaceLattice)
            })
            .transpose()?
            .unwrap_or(i32::MIN);
        ground_heights.push(ground);
        water_heights.push(water);
    }

    let mut materials_by_id = samples
        .iter()
        .map(|sample| (sample.material.id(), sample.material))
        .collect::<BTreeMap<_, _>>();
    if samples.iter().any(|sample| sample.water_level.is_some()) {
        materials_by_id.insert(Material::Water.id(), Material::Water);
    }
    let palette_indices = materials_by_id
        .keys()
        .enumerate()
        .map(|(index, id)| {
            u8::try_from(index)
                .map(|index| (*id, index))
                .map_err(|_| TerrainPageBuildError::MaterialPaletteOverflow)
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;

    let edge = shape as usize;
    let mut cell_material_indices =
        Vec::with_capacity(TERRAIN_PAGE_EDGE_SAMPLES as usize * TERRAIN_PAGE_EDGE_SAMPLES as usize);
    let mut cell_counts = BTreeMap::<u16, u32>::new();
    for z in 0..TERRAIN_PAGE_EDGE_SAMPLES as usize {
        for x in 0..TERRAIN_PAGE_EDGE_SAMPLES as usize {
            let corners = [
                samples[x + z * edge].material,
                samples[x + 1 + z * edge].material,
                samples[x + (z + 1) * edge].material,
                samples[x + 1 + (z + 1) * edge].material,
            ];
            let mut counts = BTreeMap::<u16, u8>::new();
            for material in corners {
                *counts.entry(material.id()).or_default() += 1;
            }
            let material_id = counts
                .into_iter()
                .max_by_key(|(id, count)| (*count, std::cmp::Reverse(*id)))
                .map(|(id, _)| id)
                .ok_or(TerrainPageBuildError::InvalidSurfaceLattice)?;
            cell_material_indices.push(
                *palette_indices
                    .get(&material_id)
                    .ok_or(TerrainPageBuildError::MaterialPaletteOverflow)?,
            );
            *cell_counts.entry(material_id).or_default() += 1;
        }
    }

    let cell_area = stride.saturating_mul(stride);
    let materials = materials_by_id
        .into_iter()
        .map(|(id, material)| {
            let cells = cell_counts.get(&id).copied().unwrap_or(0);
            TerrainMaterialCoverage {
                material,
                occupied_voxels: cells.saturating_mul(cell_area),
                exposed_unit_faces: cells.saturating_mul(cell_area),
            }
        })
        .collect::<Vec<_>>();

    // A source sample cannot constrain an unsampled material boundary more tightly than one cell.
    let sampling_error = stride.saturating_mul(1_000);
    errors.geometric_millivoxels = errors.geometric_millivoxels.max(sampling_error);
    errors.silhouette_millivoxels = errors.silhouette_millivoxels.max(sampling_error);
    errors.material_boundary_millivoxels = errors.material_boundary_millivoxels.max(sampling_error);
    errors.normal_milliradians = errors.normal_milliradians.max(3_142);

    let grid = TerrainHeightfieldGrid {
        sample_stride_voxels: stride,
        shape_xz: [shape as u16; 2],
        ground_heights,
        cell_material_indices,
        water_heights,
    };
    let boundary_fingerprints = heightfield_boundary_fingerprints(bounds, &grid);
    let mut page = TerrainPageV1 {
        source_identity_hash,
        key,
        revision,
        bounds,
        children: Vec::new(),
        errors,
        topology: TerrainTopologyClass::SingleRunColumns,
        boundary_fingerprints,
        materials,
        representation: TerrainPageRepresentation::HeightfieldGrid(grid),
        content_fingerprint: [0; 32],
    };
    page.content_fingerprint = terrain_page_fingerprint(&page);
    encode_terrain_page(&page).map_err(|_| TerrainPageBuildError::PayloadOverflow)?;
    Ok(page)
}

fn heightfield_boundary_fingerprints(
    bounds: VoxelBounds,
    grid: &TerrainHeightfieldGrid,
) -> [[u8; 32]; 6] {
    let edge = usize::from(grid.shape_xz[0]);
    std::array::from_fn(|index| {
        let side = BoundarySide::ALL[index];
        let mut hasher = blake3::Hasher::new();
        hasher.update(HEIGHTFIELD_BOUNDARY_DOMAIN);
        hasher.update(&[side.axis() as u8]);
        hasher.update(&boundary_plane(bounds, side).to_le_bytes());
        match side {
            BoundarySide::NegativeX | BoundarySide::PositiveX => {
                let x = usize::from(side == BoundarySide::PositiveX) * (edge - 1);
                for z in 0..edge {
                    let sample = x + z * edge;
                    hasher.update(&grid.ground_heights[sample].to_le_bytes());
                    hasher.update(&grid.water_heights[sample].to_le_bytes());
                }
            }
            BoundarySide::NegativeZ | BoundarySide::PositiveZ => {
                let z = usize::from(side == BoundarySide::PositiveZ) * (edge - 1);
                for x in 0..edge {
                    let sample = x + z * edge;
                    hasher.update(&grid.ground_heights[sample].to_le_bytes());
                    hasher.update(&grid.water_heights[sample].to_le_bytes());
                }
            }
            BoundarySide::NegativeY | BoundarySide::PositiveY => {}
        }
        *hasher.finalize().as_bytes()
    })
}

/// Assembles a parent from an already-built representation and an exact complete child group.
///
/// Builders may choose any payload encoding, but cannot choose their own ownership boundary or
/// child identity. This function derives those fields from the children and refuses incoherent
/// groups before assigning the parent's semantic fingerprint.
pub fn assemble_terrain_parent(
    key: TerrainPageKey,
    revision: u64,
    errors: TerrainErrorBounds,
    topology: TerrainTopologyClass,
    materials: Vec<TerrainMaterialCoverage>,
    representation: TerrainPageRepresentation,
    children: &[TerrainPageV1],
) -> Result<TerrainPageV1, TerrainReplacementError> {
    if key.level == 0 {
        return Err(TerrainReplacementError::InvalidParent);
    }
    validate_children_for_key(key, children)?;
    let bounds = if key.is_surface() {
        let [[minimum_x, minimum_z], [maximum_x, maximum_z]] = key
            .horizontal_bounds()
            .ok_or(TerrainReplacementError::InvalidParent)?;
        let minimum_y = children
            .iter()
            .map(|child| child.bounds.min.y)
            .min()
            .ok_or(TerrainReplacementError::InvalidParent)?;
        let maximum_y = children
            .iter()
            .map(|child| child.bounds.max.y)
            .max()
            .ok_or(TerrainReplacementError::InvalidParent)?;
        VoxelBounds::new(
            VoxelCoord::new(minimum_x, minimum_y, minimum_z),
            VoxelCoord::new(maximum_x, maximum_y, maximum_z),
        )
        .ok_or(TerrainReplacementError::InvalidParent)?
    } else {
        key.bounds().ok_or(TerrainReplacementError::InvalidParent)?
    };
    let source_identity_hash = children[0].source_identity_hash;
    let boundary_fingerprints = aggregate_child_boundaries(key, children)?;
    let mut child_references = children
        .iter()
        .map(|child| TerrainPageChild {
            key: child.key,
            revision: child.revision,
            content_fingerprint: child.content_fingerprint,
        })
        .collect::<Vec<_>>();
    child_references.sort_unstable_by_key(|child| child.key);
    let mut page = TerrainPageV1 {
        source_identity_hash,
        key,
        revision,
        bounds,
        children: child_references,
        errors,
        topology,
        boundary_fingerprints,
        materials,
        representation,
        content_fingerprint: [0; 32],
    };
    if !representation_is_valid(&page) {
        return Err(TerrainReplacementError::InvalidRepresentation);
    }
    page.content_fingerprint = terrain_page_fingerprint(&page);
    Ok(page)
}

/// Builds an exact parent by taking the set union of eight exact clustered child surfaces.
///
/// Child quads are expanded only to canonical unit-face keys, deduplicated, and greedily merged
/// again. That deliberately trades build time for a simple proof: the parent renders exactly the
/// same owned 10 cm faces as the complete child group, including across negative coordinates and
/// former child boundaries. No geometric simplification is performed by this builder.
pub fn build_exact_cluster_terrain_parent(
    key: TerrainPageKey,
    revision: u64,
    children: &[TerrainPageV1],
) -> Result<TerrainPageV1, TerrainPageBuildError> {
    let page = assemble_exact_cluster_terrain_parent(key, revision, children)?;
    encode_terrain_page(&page).map_err(|_| TerrainPageBuildError::PayloadOverflow)?;
    Ok(page)
}

fn assemble_exact_cluster_terrain_parent(
    key: TerrainPageKey,
    revision: u64,
    children: &[TerrainPageV1],
) -> Result<TerrainPageV1, TerrainPageBuildError> {
    assemble_exact_cluster_terrain_parent_from_surfaces(key, revision, children, children)
}

#[cfg(feature = "terrain-page-builder")]
pub(crate) fn assemble_exact_cluster_terrain_parent_from_surfaces(
    key: TerrainPageKey,
    revision: u64,
    children: &[TerrainPageV1],
    exact_surface_children: &[TerrainPageV1],
) -> Result<TerrainPageV1, TerrainPageBuildError> {
    assemble_exact_cluster_terrain_parent_from_surfaces_impl(
        key,
        revision,
        children,
        exact_surface_children,
    )
}

#[cfg(not(feature = "terrain-page-builder"))]
fn assemble_exact_cluster_terrain_parent_from_surfaces(
    key: TerrainPageKey,
    revision: u64,
    children: &[TerrainPageV1],
    exact_surface_children: &[TerrainPageV1],
) -> Result<TerrainPageV1, TerrainPageBuildError> {
    assemble_exact_cluster_terrain_parent_from_surfaces_impl(
        key,
        revision,
        children,
        exact_surface_children,
    )
}

fn assemble_exact_cluster_terrain_parent_from_surfaces_impl(
    key: TerrainPageKey,
    revision: u64,
    children: &[TerrainPageV1],
    exact_surface_children: &[TerrainPageV1],
) -> Result<TerrainPageV1, TerrainPageBuildError> {
    if key.level == 0 || key.bounds().is_none() {
        return Err(TerrainPageBuildError::InvalidPageKey);
    }
    validate_children_for_key(key, children)?;
    validate_children_for_key(key, exact_surface_children)?;
    if children[0].source_identity_hash != exact_surface_children[0].source_identity_hash {
        return Err(TerrainPageBuildError::InvalidChildGroup(
            TerrainReplacementError::SourceMismatch,
        ));
    }
    if aggregate_child_boundaries(key, children)?
        != aggregate_child_boundaries(key, exact_surface_children)?
    {
        return Err(TerrainPageBuildError::InvalidChildGroup(
            TerrainReplacementError::OuterBoundaryMismatch,
        ));
    }
    if exact_surface_children.iter().any(|child| {
        !matches!(
            child.representation,
            TerrainPageRepresentation::SurfaceCluster(_)
        )
    }) {
        return Err(TerrainPageBuildError::UnsupportedChildRepresentation);
    }

    let mut occupied_by_material = BTreeMap::<u16, u32>::new();
    let mut exposed_by_material = BTreeMap::<u16, u32>::new();
    let mut material_by_id = BTreeMap::<u16, Material>::new();
    for child in exact_surface_children {
        for coverage in &child.materials {
            let material_id = coverage.material.id();
            material_by_id.insert(material_id, coverage.material);
            let occupied = occupied_by_material.entry(material_id).or_default();
            *occupied = occupied.saturating_add(coverage.occupied_voxels);
            let exposed = exposed_by_material.entry(material_id).or_default();
            *exposed = exposed.saturating_add(coverage.exposed_unit_faces);
        }
    }
    let mut palette_indices = BTreeMap::new();
    let mut materials = Vec::with_capacity(material_by_id.len());
    for (index, (material_id, material)) in material_by_id.into_iter().enumerate() {
        let material_index =
            u8::try_from(index).map_err(|_| TerrainPageBuildError::MaterialPaletteOverflow)?;
        palette_indices.insert(material_id, material_index);
        materials.push(TerrainMaterialCoverage {
            material,
            occupied_voxels: occupied_by_material
                .get(&material_id)
                .copied()
                .unwrap_or_default(),
            exposed_unit_faces: exposed_by_material
                .get(&material_id)
                .copied()
                .unwrap_or_default(),
        });
    }

    let mut faces = BTreeSet::new();
    for child in exact_surface_children {
        let TerrainPageRepresentation::SurfaceCluster(quads) = &child.representation else {
            unreachable!("child representation was checked above");
        };
        for quad in quads {
            let material_id = child.materials[usize::from(quad.material_index)]
                .material
                .id();
            expand_surface_quad(*quad, material_id, &mut faces);
        }
    }
    let faces = faces.into_iter().collect::<Vec<_>>();
    let representation =
        TerrainPageRepresentation::SurfaceCluster(merge_surface_faces(&faces, &palette_indices));
    let errors = children
        .iter()
        .fold(TerrainErrorBounds::EXACT, |aggregate, child| {
            max_error_bounds(aggregate, child.errors)
        });
    let topology = if children
        .iter()
        .any(|child| child.topology == TerrainTopologyClass::Volumetric)
    {
        TerrainTopologyClass::Volumetric
    } else {
        TerrainTopologyClass::SingleRunColumns
    };
    Ok(assemble_terrain_parent(
        key,
        revision,
        errors,
        topology,
        materials,
        representation,
        children,
    )?)
}

/// Simplifies a complete exact child group into a boundary-locked triangle page.
///
/// This builder is host-only because simplification belongs in the page producer, never in the
/// renderer. It uses topology-preserving edge collapse with absolute canonical-voxel error and
/// refuses surfaces that are not two-manifolds. Page-border and material-seam vertices remain
/// locked, so independently built neighbors retain the same mathematical join.
#[cfg(feature = "terrain-page-builder")]
pub fn build_simplified_triangle_terrain_parent(
    key: TerrainPageKey,
    revision: u64,
    children: &[TerrainPageV1],
    budget: TerrainSimplificationBudget,
) -> Result<TerrainPageV1, TerrainPageBuildError> {
    validate_simplification_budget(budget)?;
    let exact = assemble_exact_cluster_terrain_parent(key, revision, children)?;
    simplify_exact_terrain_parent(&exact, children, budget)
}

#[cfg(feature = "terrain-page-builder")]
fn validate_simplification_budget(
    budget: TerrainSimplificationBudget,
) -> Result<(), TerrainPageBuildError> {
    if budget.target_triangles == 0
        || budget.max_error_millivoxels == 0
        || budget.target_encoded_bytes == 0
        || usize::try_from(budget.target_encoded_bytes).unwrap_or(usize::MAX)
            > TERRAIN_PAGE_TARGET_COMPRESSED_BYTES
    {
        return Err(TerrainPageBuildError::SimplificationTargetNotReached);
    }
    Ok(())
}

#[cfg(feature = "terrain-page-builder")]
fn simplify_exact_terrain_parent(
    exact: &TerrainPageV1,
    children: &[TerrainPageV1],
    budget: TerrainSimplificationBudget,
) -> Result<TerrainPageV1, TerrainPageBuildError> {
    let TerrainPageRepresentation::SurfaceCluster(quads) = &exact.representation else {
        unreachable!("exact parent builder always produces clustered quads");
    };
    if quads.is_empty() {
        return Err(TerrainPageBuildError::NoSurfaceToSimplify);
    }
    let input = triangulate_surface_quads(quads, exact.bounds)?;
    triangle_cluster_validation_error(&input, exact.bounds, exact.materials.len())
        .map_err(|_| TerrainPageBuildError::NonManifoldSurface)?;
    let positions = input
        .vertices
        .iter()
        .map(|vertex| {
            [
                (vertex.position[0] - exact.bounds.min.x) as f32,
                (vertex.position[1] - exact.bounds.min.y) as f32,
                (vertex.position[2] - exact.bounds.min.z) as f32,
            ]
        })
        .collect::<Vec<_>>();
    let indices = input
        .triangles
        .iter()
        .flat_map(|triangle| triangle.vertices)
        .collect::<Vec<_>>();
    let locks = locked_cluster_vertices(&input, exact.bounds);
    let target_index_count = usize::try_from(budget.target_triangles)
        .unwrap_or(usize::MAX)
        .saturating_mul(3)
        .min(indices.len());
    let certificate_spacing_millivoxels = SURFACE_ERROR_CERTIFICATE_MAX_SPACING_MILLIVOXELS
        .min(budget.max_error_millivoxels.saturating_div(4).max(1));
    let simplifier_error_limit_millivoxels = budget
        .max_error_millivoxels
        .saturating_sub(certificate_spacing_millivoxels);
    let mut measured_error_voxels = 0.0f32;
    let simplified_indices = meshopt::simplify_with_locks_decoder(
        &indices,
        &positions,
        &locks,
        target_index_count,
        simplifier_error_limit_millivoxels as f32 / 1000.0,
        meshopt::SimplifyOptions::LockBorder
            | meshopt::SimplifyOptions::ErrorAbsolute
            | meshopt::SimplifyOptions::Regularize,
        Some(&mut measured_error_voxels),
    );
    if simplified_indices.is_empty() || simplified_indices.len() % 3 != 0 {
        return Err(TerrainPageBuildError::InvalidSimplification);
    }
    let cluster = compact_simplified_cluster(&input, &simplified_indices)?;
    triangle_cluster_validation_error(&cluster, exact.bounds, exact.materials.len())
        .map_err(simplification_validation_build_error)?;
    if open_surface_edge_signature(&cluster) != open_surface_edge_signature(&input) {
        return Err(TerrainPageBuildError::InvalidSimplification);
    }
    let reduced = cluster.triangles.len() < input.triangles.len();
    let simplifier_reported_millivoxels = if reduced {
        (measured_error_voxels.max(0.0) * 1_000.0)
            .ceil()
            .clamp(0.0, u32::MAX as f32) as u32
    } else {
        0
    };
    if simplifier_reported_millivoxels > simplifier_error_limit_millivoxels {
        return Err(TerrainPageBuildError::InvalidSimplification);
    }
    let introduced_millivoxels = if reduced {
        certify_bidirectional_surface_error(
            &input,
            &cluster,
            exact.bounds.min.as_array(),
            certificate_spacing_millivoxels,
            budget.max_error_millivoxels,
        )?
        .upper_bound_millivoxels
    } else {
        0
    };
    let child_errors = children
        .iter()
        .fold(TerrainErrorBounds::EXACT, |aggregate, child| {
            max_error_bounds(aggregate, child.errors)
        });
    let errors = TerrainErrorBounds {
        geometric_millivoxels: child_errors
            .geometric_millivoxels
            .max(introduced_millivoxels),
        silhouette_millivoxels: child_errors
            .silhouette_millivoxels
            .max(introduced_millivoxels),
        material_boundary_millivoxels: child_errors.material_boundary_millivoxels,
        normal_milliradians: if reduced {
            child_errors.normal_milliradians.max(3_142)
        } else {
            child_errors.normal_milliradians
        },
        unresolved_topology: child_errors.unresolved_topology,
    };
    let page = assemble_terrain_parent(
        exact.key,
        exact.revision,
        errors,
        exact.topology,
        exact.materials.clone(),
        TerrainPageRepresentation::TriangleCluster(cluster),
        children,
    )?;
    let encoded = encode_terrain_page(&page).map_err(|_| TerrainPageBuildError::PayloadOverflow)?;
    if encoded.len() > budget.target_encoded_bytes as usize
        || page
            .representation
            .triangle_count()
            .is_some_and(|count| count > budget.target_triangles as usize)
    {
        return Err(TerrainPageBuildError::SimplificationTargetNotReached);
    }
    Ok(page)
}

/// Publishes the exact clustered parent whenever it already fits, otherwise attempts the
/// boundary-locked simplifier. Exact geometry therefore never grows merely because a triangle
/// target was configured for larger ancestors.
#[cfg(feature = "terrain-page-builder")]
pub fn build_budgeted_terrain_parent(
    key: TerrainPageKey,
    revision: u64,
    children: &[TerrainPageV1],
    budget: TerrainSimplificationBudget,
) -> Result<TerrainPageV1, TerrainPageBuildError> {
    build_budgeted_terrain_parent_from_surfaces(key, revision, children, children, budget)
}

#[cfg(feature = "terrain-page-builder")]
pub(crate) fn build_budgeted_terrain_parent_from_surfaces(
    key: TerrainPageKey,
    revision: u64,
    children: &[TerrainPageV1],
    exact_surface_children: &[TerrainPageV1],
    budget: TerrainSimplificationBudget,
) -> Result<TerrainPageV1, TerrainPageBuildError> {
    validate_simplification_budget(budget)?;
    let exact = assemble_exact_cluster_terrain_parent_from_surfaces(
        key,
        revision,
        children,
        exact_surface_children,
    )?;
    select_budgeted_exact_terrain_parent(&exact, children, budget)
}

#[cfg(feature = "terrain-page-builder")]
pub(crate) fn select_budgeted_exact_terrain_parent(
    exact: &TerrainPageV1,
    children: &[TerrainPageV1],
    budget: TerrainSimplificationBudget,
) -> Result<TerrainPageV1, TerrainPageBuildError> {
    validate_simplification_budget(budget)?;
    if representation_bytes(&exact.representation).len() <= TERRAIN_PAGE_MAX_PAYLOAD_BYTES
        && encode_terrain_page(exact)
            .is_ok_and(|encoded| encoded.len() <= budget.target_encoded_bytes as usize)
    {
        return Ok(exact.clone());
    }
    simplify_exact_terrain_parent(exact, children, budget)
}

#[cfg(feature = "terrain-page-builder")]
impl TerrainPageRepresentation {
    fn triangle_count(&self) -> Option<usize> {
        match self {
            Self::TriangleCluster(cluster) => Some(cluster.triangles.len()),
            Self::HeightfieldGrid(_) => {
                Some(TERRAIN_PAGE_EDGE_SAMPLES as usize * TERRAIN_PAGE_EDGE_SAMPLES as usize * 2)
            }
            _ => None,
        }
    }
}

#[cfg(feature = "terrain-page-builder")]
fn triangulate_surface_quads(
    quads: &[TerrainSurfaceQuad],
    bounds: VoxelBounds,
) -> Result<TerrainTriangleCluster, TerrainPageBuildError> {
    const MAX_INPUT_UNIT_FACES: usize = 2_000_000;
    let face_count = quads.iter().try_fold(0usize, |count, quad| {
        count.checked_add(usize::from(quad.width) * usize::from(quad.height))
    });
    let Some(face_count) = face_count.filter(|count| *count <= MAX_INPUT_UNIT_FACES) else {
        return Err(TerrainPageBuildError::PayloadOverflow);
    };
    let mut vertex_indices = BTreeMap::<([i32; 3], u8), u32>::new();
    let mut vertices = Vec::new();
    let mut triangles = Vec::with_capacity(face_count.saturating_mul(2));
    for quad in quads {
        for delta_v in 0..i32::from(quad.height) {
            for delta_u in 0..i32::from(quad.width) {
                let u = quad.u + delta_u;
                let v = quad.v + delta_v;
                let corners = [
                    surface_position(quad.axis, quad.plane, u, v),
                    surface_position(quad.axis, quad.plane, u + 1, v),
                    surface_position(quad.axis, quad.plane, u + 1, v + 1),
                    surface_position(quad.axis, quad.plane, u, v + 1),
                ];
                let mut corner_indices = [0u32; 4];
                for (destination, position) in corner_indices.iter_mut().zip(corners) {
                    *destination =
                        if let Some(index) = vertex_indices.get(&(position, quad.material_index)) {
                            *index
                        } else {
                            let index = u32::try_from(vertices.len())
                                .map_err(|_| TerrainPageBuildError::PayloadOverflow)?;
                            vertex_indices.insert((position, quad.material_index), index);
                            vertices.push(TerrainClusterVertex {
                                position,
                                material_index: quad.material_index,
                            });
                            index
                        };
                }
                let base_positive = quad.axis != FaceAxis::Y;
                let winding = if quad.positive == base_positive {
                    [[0, 1, 2], [0, 2, 3]]
                } else {
                    [[0, 2, 1], [0, 3, 2]]
                };
                for triangle in winding {
                    triangles.push(TerrainClusterTriangle {
                        vertices: triangle.map(|index| corner_indices[index]),
                        material_index: quad.material_index,
                    });
                }
            }
        }
    }
    let cluster = TerrainTriangleCluster {
        vertices,
        triangles,
    };
    if input_cluster_has_geometric_overlap(&cluster) {
        return Err(TerrainPageBuildError::OverlappingSurface);
    }
    if input_cluster_has_open_interior_edge(&cluster, bounds) {
        return Err(TerrainPageBuildError::OpenInteriorSurface);
    }
    triangle_cluster_validation_error(&cluster, bounds, usize::from(u8::MAX) + 1)
        .map_err(|_| TerrainPageBuildError::NonManifoldSurface)?;
    Ok(cluster)
}

#[cfg(feature = "terrain-page-builder")]
fn simplification_validation_build_error(
    error: TriangleClusterValidationError,
) -> TerrainPageBuildError {
    match error {
        TriangleClusterValidationError::DuplicateGeometricTriangle => {
            TerrainPageBuildError::OverlappingSurface
        }
        TriangleClusterValidationError::OpenInteriorEdge => {
            TerrainPageBuildError::OpenInteriorSurface
        }
        _ => TerrainPageBuildError::InvalidSimplification,
    }
}

#[cfg(feature = "terrain-page-builder")]
fn input_cluster_has_geometric_overlap(cluster: &TerrainTriangleCluster) -> bool {
    let mut unique = BTreeSet::new();
    cluster.triangles.iter().any(|triangle| {
        let mut positions = triangle
            .vertices
            .map(|index| cluster.vertices[index as usize].position);
        positions.sort_unstable();
        !unique.insert(positions)
    })
}

#[cfg(feature = "terrain-page-builder")]
fn input_cluster_has_open_interior_edge(
    cluster: &TerrainTriangleCluster,
    bounds: VoxelBounds,
) -> bool {
    let mut counts = BTreeMap::<([i32; 3], [i32; 3]), u8>::new();
    for triangle in &cluster.triangles {
        let positions = triangle
            .vertices
            .map(|index| cluster.vertices[index as usize].position);
        for [left, right] in [
            [positions[0], positions[1]],
            [positions[1], positions[2]],
            [positions[2], positions[0]],
        ] {
            let edge = if left < right {
                (left, right)
            } else {
                (right, left)
            };
            *counts.entry(edge).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .any(|((left, right), count)| count % 2 == 1 && !edge_is_on_bounds(left, right, bounds))
}

#[cfg(feature = "terrain-page-builder")]
const fn surface_position(axis: FaceAxis, plane: i32, u: i32, v: i32) -> [i32; 3] {
    match axis {
        FaceAxis::X => [plane, u, v],
        FaceAxis::Y => [u, plane, v],
        FaceAxis::Z => [u, v, plane],
    }
}

#[cfg(feature = "terrain-page-builder")]
fn compact_simplified_cluster(
    input: &TerrainTriangleCluster,
    simplified_indices: &[u32],
) -> Result<TerrainTriangleCluster, TerrainPageBuildError> {
    let used = simplified_indices.iter().copied().collect::<BTreeSet<_>>();
    let mut remap = BTreeMap::new();
    let mut vertices = Vec::with_capacity(used.len());
    for old_index in used {
        let vertex = input
            .vertices
            .get(old_index as usize)
            .copied()
            .ok_or(TerrainPageBuildError::InvalidSimplification)?;
        let new_index =
            u32::try_from(vertices.len()).map_err(|_| TerrainPageBuildError::PayloadOverflow)?;
        remap.insert(old_index, new_index);
        vertices.push(vertex);
    }
    let mut triangles = Vec::with_capacity(simplified_indices.len() / 3);
    for indices in simplified_indices.chunks_exact(3) {
        let old = [indices[0], indices[1], indices[2]];
        let source_vertices = old.map(|index| {
            input
                .vertices
                .get(index as usize)
                .copied()
                .ok_or(TerrainPageBuildError::InvalidSimplification)
        });
        let [first, second, third] = source_vertices;
        let (first, second, third) = (first?, second?, third?);
        if first.material_index != second.material_index
            || first.material_index != third.material_index
        {
            return Err(TerrainPageBuildError::InvalidSimplification);
        }
        triangles.push(TerrainClusterTriangle {
            vertices: old.map(|index| remap[&index]),
            material_index: first.material_index,
        });
    }
    Ok(TerrainTriangleCluster {
        vertices,
        triangles,
    })
}

#[cfg(feature = "terrain-page-builder")]
fn vertex_is_on_bounds(position: [i32; 3], bounds: VoxelBounds) -> bool {
    let minimum = bounds.min.as_array();
    let maximum = bounds.max.as_array();
    (0..3).any(|axis| position[axis] == minimum[axis] || position[axis] == maximum[axis])
}

#[cfg(feature = "terrain-page-builder")]
fn locked_cluster_vertices(cluster: &TerrainTriangleCluster, bounds: VoxelBounds) -> Vec<bool> {
    let mut locks = cluster
        .vertices
        .iter()
        .map(|vertex| vertex_is_on_bounds(vertex.position, bounds))
        .collect::<Vec<_>>();
    let mut edge_counts = BTreeMap::<[u32; 2], u8>::new();
    for triangle in &cluster.triangles {
        for [left, right] in [
            [triangle.vertices[0], triangle.vertices[1]],
            [triangle.vertices[1], triangle.vertices[2]],
            [triangle.vertices[2], triangle.vertices[0]],
        ] {
            let edge = if left < right {
                [left, right]
            } else {
                [right, left]
            };
            *edge_counts.entry(edge).or_default() += 1;
        }
    }
    for ([left, right], count) in edge_counts {
        if count != 2 {
            locks[left as usize] = true;
            locks[right as usize] = true;
        }
    }
    locks
}

#[cfg(feature = "terrain-page-builder")]
fn open_surface_edge_signature(
    cluster: &TerrainTriangleCluster,
) -> BTreeSet<([i32; 3], [i32; 3], u8)> {
    let mut counts = BTreeMap::<([i32; 3], [i32; 3], u8), u8>::new();
    for triangle in &cluster.triangles {
        let positions = triangle
            .vertices
            .map(|index| cluster.vertices[index as usize].position);
        for [left, right] in [
            [positions[0], positions[1]],
            [positions[1], positions[2]],
            [positions[2], positions[0]],
        ] {
            let (left, right) = if left < right {
                (left, right)
            } else {
                (right, left)
            };
            *counts
                .entry((left, right, triangle.material_index))
                .or_default() += 1;
        }
    }
    counts
        .into_iter()
        .filter_map(|(edge, count)| (count != 2).then_some(edge))
        .collect()
}

fn max_error_bounds(left: TerrainErrorBounds, right: TerrainErrorBounds) -> TerrainErrorBounds {
    TerrainErrorBounds {
        geometric_millivoxels: left.geometric_millivoxels.max(right.geometric_millivoxels),
        silhouette_millivoxels: left
            .silhouette_millivoxels
            .max(right.silhouette_millivoxels),
        material_boundary_millivoxels: left
            .material_boundary_millivoxels
            .max(right.material_boundary_millivoxels),
        normal_milliradians: left.normal_milliradians.max(right.normal_milliradians),
        unresolved_topology: left.unresolved_topology || right.unresolved_topology,
    }
}

fn expand_surface_quad(
    quad: TerrainSurfaceQuad,
    material_id: u16,
    faces: &mut BTreeSet<CanonicalFaceKey>,
) {
    for delta_v in 0..i32::from(quad.height) {
        for delta_u in 0..i32::from(quad.width) {
            let u = quad.u + delta_u;
            let v = quad.v + delta_v;
            let normal = if quad.positive {
                quad.plane - 1
            } else {
                quad.plane
            };
            let solid_side = match quad.axis {
                FaceAxis::X => VoxelCoord::new(normal, u, v),
                FaceAxis::Y => VoxelCoord::new(u, normal, v),
                FaceAxis::Z => VoxelCoord::new(u, v, normal),
            };
            faces.insert(CanonicalFaceKey {
                axis: quad.axis,
                plane: quad.plane,
                u,
                v,
                solid_side,
                material_id,
            });
        }
    }
}

pub fn validate_terrain_replacement(
    parent: &TerrainPageV1,
    children: &[TerrainPageV1],
) -> Result<(), TerrainReplacementError> {
    if !parent.validates_identity() || parent.key.level == 0 {
        return Err(TerrainReplacementError::InvalidParent);
    }
    validate_children_for_key(parent.key, children)?;
    let references = children
        .iter()
        .map(|child| (child.key, (child.revision, child.content_fingerprint)))
        .collect::<BTreeMap<_, _>>();
    if parent.children.iter().any(|reference| {
        references.get(&reference.key) != Some(&(reference.revision, reference.content_fingerprint))
    }) {
        return Err(TerrainReplacementError::ChildReferenceMismatch);
    }
    if aggregate_child_boundaries(parent.key, children)? != parent.boundary_fingerprints {
        return Err(TerrainReplacementError::OuterBoundaryMismatch);
    }
    Ok(())
}

fn validate_children_for_key(
    parent_key: TerrainPageKey,
    children: &[TerrainPageV1],
) -> Result<(), TerrainReplacementError> {
    let expected = parent_key
        .refinement_children()
        .ok_or(TerrainReplacementError::InvalidParent)?;
    if children.len() != expected.len() {
        return Err(TerrainReplacementError::WrongChildCount);
    }
    if children.iter().any(|child| !child.validates_identity()) {
        return Err(TerrainReplacementError::InvalidChild);
    }
    let source = children[0].source_identity_hash;
    if children
        .iter()
        .any(|child| child.source_identity_hash != source)
    {
        return Err(TerrainReplacementError::SourceMismatch);
    }
    let actual = children
        .iter()
        .map(|child| child.key)
        .collect::<BTreeSet<_>>();
    if actual != expected.into_iter().collect() {
        return Err(TerrainReplacementError::IncompleteChildKeys);
    }
    for (index, left) in children.iter().enumerate() {
        for right in &children[index + 1..] {
            let sides = if parent_key.is_surface() {
                adjacent_surface_page_sides(left.key, right.key)
            } else {
                adjacent_page_sides(left.bounds, right.bounds)
            };
            let Some((left_side, right_side)) = sides else {
                continue;
            };
            if left.boundary_fingerprints[left_side as usize]
                != right.boundary_fingerprints[right_side as usize]
            {
                return Err(TerrainReplacementError::InternalBoundaryMismatch);
            }
        }
    }
    Ok(())
}

fn aggregate_child_boundaries(
    parent_key: TerrainPageKey,
    children: &[TerrainPageV1],
) -> Result<[[u8; 32]; 6], TerrainReplacementError> {
    validate_children_for_key(parent_key, children)?;
    let parent_bounds = if parent_key.is_surface() {
        let [[minimum_x, minimum_z], [maximum_x, maximum_z]] = parent_key
            .horizontal_bounds()
            .ok_or(TerrainReplacementError::InvalidParent)?;
        VoxelBounds::new(
            VoxelCoord::new(
                minimum_x,
                children
                    .iter()
                    .map(|child| child.bounds.min.y)
                    .min()
                    .ok_or(TerrainReplacementError::InvalidParent)?,
                minimum_z,
            ),
            VoxelCoord::new(
                maximum_x,
                children
                    .iter()
                    .map(|child| child.bounds.max.y)
                    .max()
                    .ok_or(TerrainReplacementError::InvalidParent)?,
                maximum_z,
            ),
        )
        .ok_or(TerrainReplacementError::InvalidParent)?
    } else {
        parent_key
            .bounds()
            .ok_or(TerrainReplacementError::InvalidParent)?
    };
    let mut fingerprints = [[0u8; 32]; 6];
    for side in BoundarySide::ALL {
        let mut side_children = children
            .iter()
            .filter(|child| {
                parent_key.is_surface()
                    && matches!(side, BoundarySide::NegativeY | BoundarySide::PositiveY)
                    || boundary_plane(child.bounds, side) == boundary_plane(parent_bounds, side)
            })
            .collect::<Vec<_>>();
        side_children.sort_unstable_by_key(|child| tangential_page_coord(child.key, side.axis()));
        let expected_side_children = if parent_key.is_surface()
            && matches!(
                side,
                BoundarySide::NegativeX
                    | BoundarySide::PositiveX
                    | BoundarySide::NegativeZ
                    | BoundarySide::PositiveZ
            ) {
            2
        } else {
            4
        };
        if side_children.len() != expected_side_children {
            return Err(TerrainReplacementError::IncompleteChildKeys);
        }
        let mut hasher = blake3::Hasher::new();
        hasher.update(PARENT_BOUNDARY_DOMAIN);
        hasher.update(&[side.axis() as u8]);
        hasher.update(&boundary_plane(parent_bounds, side).to_le_bytes());
        hasher.update(&[parent_key.level]);
        for child in side_children {
            let tangent = tangential_page_coord(child.key, side.axis());
            hasher.update(&tangent[0].to_le_bytes());
            hasher.update(&tangent[1].to_le_bytes());
            hasher.update(&child.boundary_fingerprints[side as usize]);
        }
        fingerprints[side as usize] = *hasher.finalize().as_bytes();
    }
    Ok(fingerprints)
}

fn boundary_plane(bounds: VoxelBounds, side: BoundarySide) -> i32 {
    match side {
        BoundarySide::NegativeX => bounds.min.x,
        BoundarySide::PositiveX => bounds.max.x,
        BoundarySide::NegativeY => bounds.min.y,
        BoundarySide::PositiveY => bounds.max.y,
        BoundarySide::NegativeZ => bounds.min.z,
        BoundarySide::PositiveZ => bounds.max.z,
    }
}

fn tangential_page_coord(key: TerrainPageKey, axis: FaceAxis) -> [i32; 2] {
    match axis {
        FaceAxis::X => [key.coord[1], key.coord[2]],
        FaceAxis::Y => [key.coord[0], key.coord[2]],
        FaceAxis::Z => [key.coord[0], key.coord[1]],
    }
}

fn adjacent_surface_page_sides(
    left: TerrainPageKey,
    right: TerrainPageKey,
) -> Option<(BoundarySide, BoundarySide)> {
    if !left.is_surface() || !right.is_surface() || left.level != right.level {
        return None;
    }
    if left.coord[2] == right.coord[2] && left.coord[0].checked_add(1) == Some(right.coord[0]) {
        return Some((BoundarySide::PositiveX, BoundarySide::NegativeX));
    }
    if left.coord[2] == right.coord[2] && right.coord[0].checked_add(1) == Some(left.coord[0]) {
        return Some((BoundarySide::NegativeX, BoundarySide::PositiveX));
    }
    if left.coord[0] == right.coord[0] && left.coord[2].checked_add(1) == Some(right.coord[2]) {
        return Some((BoundarySide::PositiveZ, BoundarySide::NegativeZ));
    }
    if left.coord[0] == right.coord[0] && right.coord[2].checked_add(1) == Some(left.coord[2]) {
        return Some((BoundarySide::NegativeZ, BoundarySide::PositiveZ));
    }
    None
}

fn adjacent_page_sides(
    left: VoxelBounds,
    right: VoxelBounds,
) -> Option<(BoundarySide, BoundarySide)> {
    if left.max.x == right.min.x
        && left.min.y == right.min.y
        && left.max.y == right.max.y
        && left.min.z == right.min.z
        && left.max.z == right.max.z
    {
        return Some((BoundarySide::PositiveX, BoundarySide::NegativeX));
    }
    if right.max.x == left.min.x
        && left.min.y == right.min.y
        && left.max.y == right.max.y
        && left.min.z == right.min.z
        && left.max.z == right.max.z
    {
        return Some((BoundarySide::NegativeX, BoundarySide::PositiveX));
    }
    if left.max.y == right.min.y
        && left.min.x == right.min.x
        && left.max.x == right.max.x
        && left.min.z == right.min.z
        && left.max.z == right.max.z
    {
        return Some((BoundarySide::PositiveY, BoundarySide::NegativeY));
    }
    if right.max.y == left.min.y
        && left.min.x == right.min.x
        && left.max.x == right.max.x
        && left.min.z == right.min.z
        && left.max.z == right.max.z
    {
        return Some((BoundarySide::NegativeY, BoundarySide::PositiveY));
    }
    if left.max.z == right.min.z
        && left.min.x == right.min.x
        && left.max.x == right.max.x
        && left.min.y == right.min.y
        && left.max.y == right.max.y
    {
        return Some((BoundarySide::PositiveZ, BoundarySide::NegativeZ));
    }
    if right.max.z == left.min.z
        && left.min.x == right.min.x
        && left.max.x == right.max.x
        && left.min.y == right.min.y
        && left.max.y == right.max.y
    {
        return Some((BoundarySide::NegativeZ, BoundarySide::PositiveZ));
    }
    None
}

fn material_coverage(
    bounds: VoxelBounds,
    samples: &ExactPageSamples,
    faces: &[CanonicalFaceKey],
) -> Result<(Vec<TerrainMaterialCoverage>, BTreeMap<u16, u8>), TerrainPageBuildError> {
    let mut occupied = BTreeMap::<u16, u32>::new();
    for z in bounds.min.z..bounds.max.z {
        for y in bounds.min.y..bounds.max.y {
            for x in bounds.min.x..bounds.max.x {
                let material = samples.sample(VoxelCoord::new(x, y, z));
                if material.is_renderable() {
                    *occupied.entry(material.id()).or_default() += 1;
                }
            }
        }
    }
    let mut exposed = BTreeMap::<u16, u32>::new();
    for face in faces {
        *exposed.entry(face.material_id).or_default() += 1;
    }
    let mut palette_indices = BTreeMap::new();
    let mut materials = Vec::with_capacity(occupied.len());
    for (index, (id, occupied_voxels)) in occupied.into_iter().enumerate() {
        let material =
            Material::from_id(id).ok_or(TerrainPageBuildError::MaterialPaletteOverflow)?;
        let index =
            u8::try_from(index).map_err(|_| TerrainPageBuildError::MaterialPaletteOverflow)?;
        palette_indices.insert(id, index);
        materials.push(TerrainMaterialCoverage {
            material,
            occupied_voxels,
            exposed_unit_faces: exposed.get(&id).copied().unwrap_or(0),
        });
    }
    Ok((materials, palette_indices))
}

fn build_stepped(
    bounds: VoxelBounds,
    samples: &ExactPageSamples,
    palette_indices: &BTreeMap<u16, u8>,
) -> Result<(SteppedSurfaceResidual, TerrainTopologyClass), TerrainPageBuildError> {
    let mut columns =
        Vec::with_capacity(TERRAIN_PAGE_EDGE_SAMPLES as usize * TERRAIN_PAGE_EDGE_SAMPLES as usize);
    let mut runs = Vec::new();
    let mut topology = TerrainTopologyClass::SingleRunColumns;
    for z in bounds.min.z..bounds.max.z {
        for x in bounds.min.x..bounds.max.x {
            let first_run =
                u32::try_from(runs.len()).map_err(|_| TerrainPageBuildError::PayloadOverflow)?;
            let mut occupancy_runs = 0u16;
            let mut active: Option<TerrainMaterialRun> = None;
            let mut prior_renderable = false;
            for y in bounds.min.y..bounds.max.y {
                let material = samples.sample(VoxelCoord::new(x, y, z));
                let renderable = material.is_renderable();
                if renderable && !prior_renderable {
                    occupancy_runs = occupancy_runs.saturating_add(1);
                }
                prior_renderable = renderable;
                let Some(material_index) = palette_indices.get(&material.id()).copied() else {
                    if let Some(run) = active.take() {
                        runs.push(run);
                    }
                    continue;
                };
                match active {
                    Some(mut run)
                        if run.material_index == material_index
                            && run.minimum_y + i32::from(run.length) == y =>
                    {
                        run.length = run.length.saturating_add(1);
                        active = Some(run);
                    }
                    Some(run) => {
                        runs.push(run);
                        active = Some(TerrainMaterialRun {
                            minimum_y: y,
                            length: 1,
                            material_index,
                        });
                    }
                    None => {
                        active = Some(TerrainMaterialRun {
                            minimum_y: y,
                            length: 1,
                            material_index,
                        });
                    }
                }
            }
            if let Some(run) = active {
                runs.push(run);
            }
            if occupancy_runs > 1 {
                topology = TerrainTopologyClass::Volumetric;
            }
            columns.push(TerrainColumn {
                first_run,
                run_count: u16::try_from(runs.len() - first_run as usize)
                    .map_err(|_| TerrainPageBuildError::PayloadOverflow)?,
            });
        }
    }
    Ok((
        SteppedSurfaceResidual {
            sample_stride_voxels: 1,
            shape_xz: [
                TERRAIN_PAGE_EDGE_SAMPLES as u16,
                TERRAIN_PAGE_EDGE_SAMPLES as u16,
            ],
            columns,
            runs,
            boundary_quads: Vec::new(),
        },
        topology,
    ))
}

fn surface_quad_is_on_boundary(quad: TerrainSurfaceQuad, bounds: VoxelBounds) -> bool {
    match quad.axis {
        FaceAxis::X => quad.plane == bounds.min.x || quad.plane == bounds.max.x,
        FaceAxis::Y => quad.plane == bounds.min.y || quad.plane == bounds.max.y,
        FaceAxis::Z => quad.plane == bounds.min.z || quad.plane == bounds.max.z,
    }
}

fn build_sparse(
    bounds: VoxelBounds,
    samples: &ExactPageSamples,
    palette_indices: &BTreeMap<u16, u8>,
) -> Vec<TerrainSparseBrick> {
    let bricks_per_axis = TERRAIN_PAGE_EDGE_SAMPLES as u8 / SPARSE_BRICK_EDGE;
    let mut bricks = Vec::new();
    for brick_z in 0..bricks_per_axis {
        for brick_y in 0..bricks_per_axis {
            for brick_x in 0..bricks_per_axis {
                let mut occupancy = [0u64; 8];
                let mut material_indices = Vec::new();
                for local_z in 0..SPARSE_BRICK_EDGE {
                    for local_y in 0..SPARSE_BRICK_EDGE {
                        for local_x in 0..SPARSE_BRICK_EDGE {
                            let coord = VoxelCoord::new(
                                bounds.min.x + i32::from(brick_x * SPARSE_BRICK_EDGE + local_x),
                                bounds.min.y + i32::from(brick_y * SPARSE_BRICK_EDGE + local_y),
                                bounds.min.z + i32::from(brick_z * SPARSE_BRICK_EDGE + local_z),
                            );
                            let material = samples.sample(coord);
                            let Some(material_index) = palette_indices.get(&material.id()).copied()
                            else {
                                continue;
                            };
                            let index = usize::from(local_x)
                                + usize::from(local_y) * usize::from(SPARSE_BRICK_EDGE)
                                + usize::from(local_z)
                                    * usize::from(SPARSE_BRICK_EDGE)
                                    * usize::from(SPARSE_BRICK_EDGE);
                            occupancy[index / 64] |= 1u64 << (index % 64);
                            material_indices.push(material_index);
                        }
                    }
                }
                if !material_indices.is_empty() {
                    bricks.push(TerrainSparseBrick {
                        local_brick: [brick_x, brick_y, brick_z],
                        occupancy,
                        material_indices,
                    });
                }
            }
        }
    }
    bricks
}

/// Reconstructs an exact clustered surface from a compact level-0 occupancy representation.
///
/// The compact payload owns all occupancy inside its half-open page. Interior exposed faces are
/// derived from that occupancy, while the explicit boundary quads preserve the one-voxel halo
/// decision made by the producer. This makes a page independently renderable without inventing
/// walls at page edges or requiring a neighboring page to be resident.
pub fn reconstruct_exact_terrain_surface(
    page: &TerrainPageV1,
) -> Result<Vec<TerrainSurfaceQuad>, TerrainPageReconstructionError> {
    if !page.validates_identity() {
        return Err(TerrainPageReconstructionError::InvalidPage);
    }
    let edge = TERRAIN_PAGE_EDGE_SAMPLES as usize;
    let volume = edge
        .checked_mul(edge)
        .and_then(|area| area.checked_mul(edge))
        .ok_or(TerrainPageReconstructionError::InvalidPage)?;
    let mut occupancy = vec![u8::MAX; volume];
    let boundary_quads = match &page.representation {
        TerrainPageRepresentation::SteppedSurfaceResidual(surface) => {
            if surface.sample_stride_voxels != 1 || page.key.level != 0 {
                return Err(TerrainPageReconstructionError::NonUnitStride);
            }
            for z in 0..edge {
                for x in 0..edge {
                    let column = surface.columns[x + z * edge];
                    let start = column.first_run as usize;
                    let end = start + usize::from(column.run_count);
                    for run in &surface.runs[start..end] {
                        for world_y in run.minimum_y..run.minimum_y + i32::from(run.length) {
                            let local_y = usize::try_from(world_y - page.bounds.min.y)
                                .map_err(|_| TerrainPageReconstructionError::InvalidPage)?;
                            occupancy[voxel_linear_index(x, local_y, z, edge)] = run.material_index;
                        }
                    }
                }
            }
            &surface.boundary_quads
        }
        TerrainPageRepresentation::SparseVoxelBrick(payload) => {
            if page.key.level != 0 {
                return Err(TerrainPageReconstructionError::NonUnitStride);
            }
            let brick_edge = usize::from(payload.brick_edge);
            for brick in &payload.bricks {
                let mut material_cursor = 0usize;
                for bit in 0..brick_edge.pow(3) {
                    if brick.occupancy[bit / 64] & (1u64 << (bit % 64)) == 0 {
                        continue;
                    }
                    let local_x = bit % brick_edge;
                    let local_y = bit / brick_edge % brick_edge;
                    let local_z = bit / (brick_edge * brick_edge);
                    let x = usize::from(brick.local_brick[0]) * brick_edge + local_x;
                    let y = usize::from(brick.local_brick[1]) * brick_edge + local_y;
                    let z = usize::from(brick.local_brick[2]) * brick_edge + local_z;
                    occupancy[voxel_linear_index(x, y, z, edge)] =
                        brick.material_indices[material_cursor];
                    material_cursor += 1;
                }
            }
            &payload.boundary_quads
        }
        _ => return Err(TerrainPageReconstructionError::UnsupportedRepresentation),
    };

    let mut faces = BTreeSet::new();
    for z in 0..edge {
        for y in 0..edge {
            for x in 0..edge {
                let material_index = occupancy[voxel_linear_index(x, y, z, edge)];
                if material_index == u8::MAX {
                    continue;
                }
                let material_id = page.materials[usize::from(material_index)].material.id();
                let world = VoxelCoord::new(
                    page.bounds.min.x + x as i32,
                    page.bounds.min.y + y as i32,
                    page.bounds.min.z + z as i32,
                );
                for axis in FaceAxis::ALL {
                    for positive in [false, true] {
                        let (nx, ny, nz) = match (axis, positive) {
                            (FaceAxis::X, false) => (x.checked_sub(1), Some(y), Some(z)),
                            (FaceAxis::X, true) => {
                                (x.checked_add(1).filter(|v| *v < edge), Some(y), Some(z))
                            }
                            (FaceAxis::Y, false) => (Some(x), y.checked_sub(1), Some(z)),
                            (FaceAxis::Y, true) => {
                                (Some(x), y.checked_add(1).filter(|v| *v < edge), Some(z))
                            }
                            (FaceAxis::Z, false) => (Some(x), Some(y), z.checked_sub(1)),
                            (FaceAxis::Z, true) => {
                                (Some(x), Some(y), z.checked_add(1).filter(|v| *v < edge))
                            }
                        };
                        let (Some(nx), Some(ny), Some(nz)) = (nx, ny, nz) else {
                            continue;
                        };
                        if occupancy[voxel_linear_index(nx, ny, nz, edge)] == u8::MAX {
                            faces.insert(canonical_face(world, material_id, axis, positive));
                        }
                    }
                }
            }
        }
    }
    for quad in boundary_quads {
        let material_id = page.materials[usize::from(quad.material_index)]
            .material
            .id();
        expand_surface_quad(*quad, material_id, &mut faces);
    }
    let palette_indices = page
        .materials
        .iter()
        .enumerate()
        .map(|(index, coverage)| (coverage.material.id(), index as u8))
        .collect::<BTreeMap<_, _>>();
    Ok(merge_surface_faces(
        &faces.into_iter().collect::<Vec<_>>(),
        &palette_indices,
    ))
}

fn voxel_linear_index(x: usize, y: usize, z: usize, edge: usize) -> usize {
    x + y * edge + z * edge * edge
}

fn canonical_face(
    solid: VoxelCoord,
    material_id: u16,
    axis: FaceAxis,
    positive: bool,
) -> CanonicalFaceKey {
    let component = match axis {
        FaceAxis::X => solid.x,
        FaceAxis::Y => solid.y,
        FaceAxis::Z => solid.z,
    };
    let (u, v) = match axis {
        FaceAxis::X => (solid.y, solid.z),
        FaceAxis::Y => (solid.x, solid.z),
        FaceAxis::Z => (solid.x, solid.y),
    };
    CanonicalFaceKey {
        axis,
        plane: component + i32::from(positive),
        u,
        v,
        solid_side: solid,
        material_id,
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct FacePlane {
    axis: FaceAxis,
    plane: i32,
    positive: bool,
    material_index: u8,
}

fn merge_surface_faces(
    faces: &[CanonicalFaceKey],
    palette_indices: &BTreeMap<u16, u8>,
) -> Vec<TerrainSurfaceQuad> {
    let mut planes = BTreeMap::<FacePlane, BTreeSet<(i32, i32)>>::new();
    for face in faces {
        let solid_component = match face.axis {
            FaceAxis::X => face.solid_side.x,
            FaceAxis::Y => face.solid_side.y,
            FaceAxis::Z => face.solid_side.z,
        };
        let Some(material_index) = palette_indices.get(&face.material_id).copied() else {
            continue;
        };
        planes
            .entry(FacePlane {
                axis: face.axis,
                plane: face.plane,
                positive: solid_component.saturating_add(1) == face.plane,
                material_index,
            })
            .or_default()
            .insert((face.u, face.v));
    }
    let mut quads = Vec::new();
    for (plane, mut cells) in planes {
        while let Some(&(u, v)) = cells.first() {
            let mut width = 1i32;
            while cells.contains(&(u.saturating_add(width), v)) {
                width += 1;
            }
            let mut height = 1i32;
            'rows: loop {
                let candidate_v = v.saturating_add(height);
                for offset in 0..width {
                    if !cells.contains(&(u.saturating_add(offset), candidate_v)) {
                        break 'rows;
                    }
                }
                height += 1;
            }
            for row in 0..height {
                for column in 0..width {
                    cells.remove(&(u.saturating_add(column), v.saturating_add(row)));
                }
            }
            quads.push(TerrainSurfaceQuad {
                axis: plane.axis,
                plane: plane.plane,
                u,
                v,
                width: u16::try_from(width).unwrap_or(u16::MAX),
                height: u16::try_from(height).unwrap_or(u16::MAX),
                positive: plane.positive,
                material_index: plane.material_index,
            });
        }
    }
    quads
}

fn terrain_page_fingerprint(page: &TerrainPageV1) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(PAGE_FINGERPRINT_DOMAIN);
    hasher.update(&TERRAIN_PAGE_SCHEMA_VERSION.to_le_bytes());
    hasher.update(page.source_identity_hash.as_bytes());
    hash_key(&mut hasher, page.key);
    hasher.update(&page.revision.to_le_bytes());
    hash_bounds(&mut hasher, page.bounds);
    hash_errors(&mut hasher, page.errors);
    hasher.update(&[page.topology as u8, page.representation.kind() as u8]);
    hasher.update(&(page.children.len() as u16).to_le_bytes());
    for child in &page.children {
        hash_key(&mut hasher, child.key);
        hasher.update(&child.revision.to_le_bytes());
        hasher.update(&child.content_fingerprint);
    }
    for fingerprint in &page.boundary_fingerprints {
        hasher.update(fingerprint);
    }
    hasher.update(&(page.materials.len() as u16).to_le_bytes());
    for coverage in &page.materials {
        hasher.update(&coverage.material.id().to_le_bytes());
        hasher.update(&coverage.occupied_voxels.to_le_bytes());
        hasher.update(&coverage.exposed_unit_faces.to_le_bytes());
    }
    let payload = representation_bytes(&page.representation);
    hasher.update(&(payload.len() as u32).to_le_bytes());
    hasher.update(&payload);
    *hasher.finalize().as_bytes()
}

fn hash_key(hasher: &mut blake3::Hasher, key: TerrainPageKey) {
    hasher.update(&[key.level]);
    for component in key.coord {
        hasher.update(&component.to_le_bytes());
    }
}

fn hash_bounds(hasher: &mut blake3::Hasher, bounds: VoxelBounds) {
    for component in [
        bounds.min.x,
        bounds.min.y,
        bounds.min.z,
        bounds.max.x,
        bounds.max.y,
        bounds.max.z,
    ] {
        hasher.update(&component.to_le_bytes());
    }
}

fn hash_errors(hasher: &mut blake3::Hasher, errors: TerrainErrorBounds) {
    hasher.update(&errors.geometric_millivoxels.to_le_bytes());
    hasher.update(&errors.silhouette_millivoxels.to_le_bytes());
    hasher.update(&errors.material_boundary_millivoxels.to_le_bytes());
    hasher.update(&errors.normal_milliradians.to_le_bytes());
    hasher.update(&[u8::from(errors.unresolved_topology)]);
}

fn representation_bytes(representation: &TerrainPageRepresentation) -> Vec<u8> {
    let mut bytes = Vec::new();
    match representation {
        TerrainPageRepresentation::SteppedSurfaceResidual(surface) => {
            push_u32(&mut bytes, surface.sample_stride_voxels);
            push_u16(&mut bytes, surface.shape_xz[0]);
            push_u16(&mut bytes, surface.shape_xz[1]);
            push_u32(&mut bytes, surface.columns.len() as u32);
            push_u32(&mut bytes, surface.runs.len() as u32);
            push_u32(&mut bytes, surface.boundary_quads.len() as u32);
            for column in &surface.columns {
                push_u32(&mut bytes, column.first_run);
                push_u16(&mut bytes, column.run_count);
            }
            for run in &surface.runs {
                push_i32(&mut bytes, run.minimum_y);
                push_u16(&mut bytes, run.length);
                bytes.push(run.material_index);
            }
            encode_surface_quads(&mut bytes, &surface.boundary_quads);
        }
        TerrainPageRepresentation::SparseVoxelBrick(payload) => {
            bytes.push(payload.brick_edge);
            push_u32(&mut bytes, payload.bricks.len() as u32);
            push_u32(&mut bytes, payload.boundary_quads.len() as u32);
            for brick in &payload.bricks {
                bytes.extend_from_slice(&brick.local_brick);
                for occupancy in brick.occupancy {
                    bytes.extend_from_slice(&occupancy.to_le_bytes());
                }
                push_u16(&mut bytes, brick.material_indices.len() as u16);
                bytes.extend_from_slice(&brick.material_indices);
            }
            encode_surface_quads(&mut bytes, &payload.boundary_quads);
        }
        TerrainPageRepresentation::SurfaceCluster(quads) => {
            push_u32(&mut bytes, quads.len() as u32);
            encode_surface_quads(&mut bytes, quads);
        }
        TerrainPageRepresentation::TriangleCluster(cluster) => {
            push_u32(&mut bytes, cluster.vertices.len() as u32);
            push_u32(&mut bytes, cluster.triangles.len() as u32);
            for vertex in &cluster.vertices {
                for component in vertex.position {
                    push_i32(&mut bytes, component);
                }
                bytes.push(vertex.material_index);
            }
            for triangle in &cluster.triangles {
                for vertex in triangle.vertices {
                    push_u32(&mut bytes, vertex);
                }
                bytes.push(triangle.material_index);
            }
        }
        TerrainPageRepresentation::HeightfieldGrid(grid) => {
            push_u32(&mut bytes, grid.sample_stride_voxels);
            for component in grid.shape_xz {
                push_u16(&mut bytes, component);
            }
            push_u32(&mut bytes, grid.ground_heights.len() as u32);
            push_u32(&mut bytes, grid.cell_material_indices.len() as u32);
            push_u32(&mut bytes, grid.water_heights.len() as u32);
            for height in &grid.ground_heights {
                push_i32(&mut bytes, *height);
            }
            bytes.extend_from_slice(&grid.cell_material_indices);
            for height in &grid.water_heights {
                push_i32(&mut bytes, *height);
            }
        }
    }
    bytes
}

fn encode_surface_quads(bytes: &mut Vec<u8>, quads: &[TerrainSurfaceQuad]) {
    for quad in quads {
        bytes.push(quad.axis as u8);
        bytes.push(u8::from(quad.positive));
        bytes.push(quad.material_index);
        push_i32(bytes, quad.plane);
        push_i32(bytes, quad.u);
        push_i32(bytes, quad.v);
        push_u16(bytes, quad.width);
        push_u16(bytes, quad.height);
    }
}

fn children_are_complete(page: &TerrainPageV1) -> bool {
    if page.children.is_empty() {
        return true;
    }
    let Some(expected) = page.key.refinement_children() else {
        return false;
    };
    let actual = page
        .children
        .iter()
        .map(|child| child.key)
        .collect::<BTreeSet<_>>();
    actual == expected.into_iter().collect()
}

fn page_bounds_match_key(key: TerrainPageKey, bounds: VoxelBounds) -> bool {
    if !key.is_surface() {
        return key.bounds() == Some(bounds);
    }
    key.horizontal_bounds().is_some_and(|[minimum, maximum]| {
        bounds.min.x == minimum[0]
            && bounds.min.z == minimum[1]
            && bounds.max.x == maximum[0]
            && bounds.max.z == maximum[1]
    })
}

fn representation_is_valid(page: &TerrainPageV1) -> bool {
    let palette_len = page.materials.len();
    match &page.representation {
        TerrainPageRepresentation::SteppedSurfaceResidual(surface) => {
            page.topology == TerrainTopologyClass::SingleRunColumns
                && surface.sample_stride_voxels
                    == 1u32.checked_shl(u32::from(page.key.level)).unwrap_or(0)
                && surface.shape_xz
                    == [
                        TERRAIN_PAGE_EDGE_SAMPLES as u16,
                        TERRAIN_PAGE_EDGE_SAMPLES as u16,
                    ]
                && surface.columns.len()
                    == TERRAIN_PAGE_EDGE_SAMPLES as usize * TERRAIN_PAGE_EDGE_SAMPLES as usize
                && surface.columns.iter().all(|column| {
                    let start = column.first_run as usize;
                    let end = start.saturating_add(usize::from(column.run_count));
                    end <= surface.runs.len()
                        && surface.runs[start..end].windows(2).all(|runs| {
                            runs[0].minimum_y + i32::from(runs[0].length) == runs[1].minimum_y
                        })
                })
                && surface.runs.iter().all(|run| {
                    run.length > 0
                        && usize::from(run.material_index) < palette_len
                        && run.minimum_y >= page.bounds.min.y
                        && run.minimum_y + i32::from(run.length) <= page.bounds.max.y
                })
                && boundary_quads_are_valid(&surface.boundary_quads, page, palette_len)
        }
        TerrainPageRepresentation::SparseVoxelBrick(payload) => {
            let bricks_per_axis = TERRAIN_PAGE_EDGE_SAMPLES as u8 / SPARSE_BRICK_EDGE;
            let unique = payload
                .bricks
                .iter()
                .map(|brick| brick.local_brick)
                .collect::<BTreeSet<_>>();
            payload.brick_edge == SPARSE_BRICK_EDGE
                && unique.len() == payload.bricks.len()
                && payload.bricks.len() <= usize::from(bricks_per_axis).pow(3)
                && payload.bricks.iter().all(|brick| {
                    brick
                        .local_brick
                        .into_iter()
                        .all(|component| component < bricks_per_axis)
                        && brick.material_indices.len()
                            == brick
                                .occupancy
                                .iter()
                                .map(|word| word.count_ones() as usize)
                                .sum::<usize>()
                        && brick
                            .material_indices
                            .iter()
                            .all(|index| usize::from(*index) < palette_len)
                })
                && boundary_quads_are_valid(&payload.boundary_quads, page, palette_len)
        }
        TerrainPageRepresentation::SurfaceCluster(quads) => quads.iter().all(|quad| {
            quad.width > 0
                && quad.height > 0
                && usize::from(quad.material_index) < palette_len
                && quad_inside_bounds(*quad, page.bounds)
        }),
        TerrainPageRepresentation::TriangleCluster(cluster) => {
            triangle_cluster_validation_error(cluster, page.bounds, palette_len).is_ok()
        }
        TerrainPageRepresentation::HeightfieldGrid(grid) => {
            let expected_stride = 1u32.checked_shl(u32::from(page.key.level)).unwrap_or(0);
            let edge = TERRAIN_PAGE_EDGE_SAMPLES as usize + 1;
            let cells = TERRAIN_PAGE_EDGE_SAMPLES as usize * TERRAIN_PAGE_EDGE_SAMPLES as usize;
            page.topology == TerrainTopologyClass::SingleRunColumns
                && grid.sample_stride_voxels == expected_stride
                && grid.shape_xz
                    == [
                        (TERRAIN_PAGE_EDGE_SAMPLES + 1) as u16,
                        (TERRAIN_PAGE_EDGE_SAMPLES + 1) as u16,
                    ]
                && grid.ground_heights.len() == edge * edge
                && grid.water_heights.len() == edge * edge
                && grid.cell_material_indices.len() == cells
                && grid
                    .ground_heights
                    .iter()
                    .all(|height| (page.bounds.min.y..=page.bounds.max.y).contains(height))
                && grid
                    .water_heights
                    .iter()
                    .zip(&grid.ground_heights)
                    .all(|(water, ground)| {
                        *water == i32::MIN
                            || (*water >= *ground
                                && (page.bounds.min.y..=page.bounds.max.y).contains(water))
                    })
                && grid
                    .cell_material_indices
                    .iter()
                    .all(|index| usize::from(*index) < palette_len)
        }
    }
}

fn boundary_quads_are_valid(
    quads: &[TerrainSurfaceQuad],
    page: &TerrainPageV1,
    palette_len: usize,
) -> bool {
    quads.iter().all(|quad| {
        quad.width > 0
            && quad.height > 0
            && usize::from(quad.material_index) < palette_len
            && quad_inside_bounds(*quad, page.bounds)
            && surface_quad_is_on_boundary(*quad, page.bounds)
    })
}

fn quad_inside_bounds(quad: TerrainSurfaceQuad, bounds: VoxelBounds) -> bool {
    let width = i32::from(quad.width);
    let height = i32::from(quad.height);
    match quad.axis {
        FaceAxis::X => {
            ((quad.positive && (bounds.min.x + 1..=bounds.max.x).contains(&quad.plane))
                || (!quad.positive && (bounds.min.x..bounds.max.x).contains(&quad.plane)))
                && quad.u >= bounds.min.y
                && quad.v >= bounds.min.z
                && quad.u.saturating_add(width) <= bounds.max.y
                && quad.v.saturating_add(height) <= bounds.max.z
        }
        FaceAxis::Y => {
            ((quad.positive && (bounds.min.y + 1..=bounds.max.y).contains(&quad.plane))
                || (!quad.positive && (bounds.min.y..bounds.max.y).contains(&quad.plane)))
                && quad.u >= bounds.min.x
                && quad.v >= bounds.min.z
                && quad.u.saturating_add(width) <= bounds.max.x
                && quad.v.saturating_add(height) <= bounds.max.z
        }
        FaceAxis::Z => {
            ((quad.positive && (bounds.min.z + 1..=bounds.max.z).contains(&quad.plane))
                || (!quad.positive && (bounds.min.z..bounds.max.z).contains(&quad.plane)))
                && quad.u >= bounds.min.x
                && quad.v >= bounds.min.y
                && quad.u.saturating_add(width) <= bounds.max.x
                && quad.v.saturating_add(height) <= bounds.max.y
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TriangleClusterValidationError {
    Empty,
    VertexLimit,
    VertexOutsideBounds,
    InvalidVertexMaterial,
    DuplicateTriangle,
    InvalidTriangleMaterial,
    MissingVertex,
    TriangleMaterialMismatch,
    DuplicateGeometricTriangle,
    DegenerateTriangle,
    UnusedVertex,
    OpenInteriorEdge,
}

fn triangle_cluster_validation_error(
    cluster: &TerrainTriangleCluster,
    bounds: VoxelBounds,
    palette_len: usize,
) -> Result<(), TriangleClusterValidationError> {
    if cluster.vertices.is_empty() || cluster.triangles.is_empty() {
        return Err(TriangleClusterValidationError::Empty);
    }
    if cluster.vertices.len() > u32::MAX as usize {
        return Err(TriangleClusterValidationError::VertexLimit);
    }
    for vertex in &cluster.vertices {
        if usize::from(vertex.material_index) >= palette_len {
            return Err(TriangleClusterValidationError::InvalidVertexMaterial);
        }
        if !(bounds.min.x..=bounds.max.x).contains(&vertex.position[0])
            || !(bounds.min.y..=bounds.max.y).contains(&vertex.position[1])
            || !(bounds.min.z..=bounds.max.z).contains(&vertex.position[2])
        {
            return Err(TriangleClusterValidationError::VertexOutsideBounds);
        }
    }
    let mut used = vec![false; cluster.vertices.len()];
    let mut unique_triangles = BTreeSet::new();
    let mut unique_geometric_triangles = BTreeSet::new();
    let mut edges = BTreeMap::<([i32; 3], [i32; 3]), u8>::new();
    for triangle in &cluster.triangles {
        if !unique_triangles.insert(*triangle) {
            return Err(TriangleClusterValidationError::DuplicateTriangle);
        }
        if usize::from(triangle.material_index) >= palette_len {
            return Err(TriangleClusterValidationError::InvalidTriangleMaterial);
        }
        let Some(vertices) = triangle
            .vertices
            .map(|index| {
                usize::try_from(index)
                    .ok()
                    .and_then(|index| cluster.vertices.get(index))
            })
            .into_iter()
            .collect::<Option<Vec<_>>>()
        else {
            return Err(TriangleClusterValidationError::MissingVertex);
        };
        if vertices
            .iter()
            .any(|vertex| vertex.material_index != triangle.material_index)
        {
            return Err(TriangleClusterValidationError::TriangleMaterialMismatch);
        }
        let mut geometric_triangle = [
            vertices[0].position,
            vertices[1].position,
            vertices[2].position,
        ];
        geometric_triangle.sort_unstable();
        if !unique_geometric_triangles.insert(geometric_triangle) {
            return Err(TriangleClusterValidationError::DuplicateGeometricTriangle);
        }
        for index in triangle.vertices {
            used[index as usize] = true;
        }
        let edge_a = vector_difference(vertices[1].position, vertices[0].position);
        let edge_b = vector_difference(vertices[2].position, vertices[0].position);
        if cross_product(edge_a, edge_b) == [0, 0, 0] {
            return Err(TriangleClusterValidationError::DegenerateTriangle);
        }
        for (from, to) in [(0, 1), (1, 2), (2, 0)] {
            let from = vertices[from].position;
            let to = vertices[to].position;
            let key = if from < to { (from, to) } else { (to, from) };
            let entry = edges.entry(key).or_default();
            *entry = entry.saturating_add(1);
        }
    }
    if used.into_iter().any(|used| !used) {
        return Err(TriangleClusterValidationError::UnusedVertex);
    }
    if edges.into_iter().any(|((left, right), count)| {
        !((count >= 2 && count % 2 == 0)
            || (count % 2 == 1 && edge_is_on_bounds(left, right, bounds)))
    }) {
        return Err(TriangleClusterValidationError::OpenInteriorEdge);
    }
    Ok(())
}

fn vector_difference(left: [i32; 3], right: [i32; 3]) -> [i64; 3] {
    [
        i64::from(left[0]) - i64::from(right[0]),
        i64::from(left[1]) - i64::from(right[1]),
        i64::from(left[2]) - i64::from(right[2]),
    ]
}

fn cross_product(left: [i64; 3], right: [i64; 3]) -> [i64; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

fn edge_is_on_bounds(left: [i32; 3], right: [i32; 3], bounds: VoxelBounds) -> bool {
    let minimum = bounds.min.as_array();
    let maximum = bounds.max.as_array();
    (0..3).any(|axis| {
        (left[axis] == minimum[axis] && right[axis] == minimum[axis])
            || (left[axis] == maximum[axis] && right[axis] == maximum[axis])
    })
}

pub fn encode_terrain_page(page: &TerrainPageV1) -> Result<Vec<u8>, TerrainPageCodecError> {
    if !page.validates_identity() {
        return Err(TerrainPageCodecError::InvalidHeader(
            "page identity or structure is invalid",
        ));
    }
    let payload = representation_bytes(&page.representation);
    if payload.len() > TERRAIN_PAGE_MAX_PAYLOAD_BYTES {
        return Err(TerrainPageCodecError::LimitExceeded(
            "uncompressed payload bytes",
        ));
    }
    let compressed = compress_page_payload(&payload)?;
    if compressed.len() > TERRAIN_PAGE_MAX_COMPRESSED_BYTES {
        return Err(TerrainPageCodecError::LimitExceeded(
            "compressed payload bytes",
        ));
    }
    let mut encoded = Vec::with_capacity(
        usize::from(PAGE_HEADER_LEN)
            + page.children.len() * 56
            + page.materials.len() * 12
            + compressed.len(),
    );
    encoded.extend_from_slice(PAGE_MAGIC);
    push_u16(&mut encoded, TERRAIN_PAGE_SCHEMA_VERSION);
    push_u16(&mut encoded, PAGE_HEADER_LEN);
    encoded.extend_from_slice(page.source_identity_hash.as_bytes());
    encoded.push(page.key.level);
    encoded.extend_from_slice(&[0; 3]);
    for component in page.key.coord {
        push_i32(&mut encoded, component);
    }
    push_u64(&mut encoded, page.revision);
    for component in [
        page.bounds.min.x,
        page.bounds.min.y,
        page.bounds.min.z,
        page.bounds.max.x,
        page.bounds.max.y,
        page.bounds.max.z,
    ] {
        push_i32(&mut encoded, component);
    }
    push_u32(&mut encoded, page.errors.geometric_millivoxels);
    push_u32(&mut encoded, page.errors.silhouette_millivoxels);
    push_u32(&mut encoded, page.errors.material_boundary_millivoxels);
    push_u32(&mut encoded, page.errors.normal_milliradians);
    encoded.push(page.topology as u8);
    encoded.push(u8::from(page.errors.unresolved_topology));
    encoded.push(page.representation.kind() as u8);
    encoded.push(PAGE_COMPRESSION_BROTLI);
    push_u16(&mut encoded, page.children.len() as u16);
    push_u16(&mut encoded, page.materials.len() as u16);
    push_u32(&mut encoded, payload.len() as u32);
    push_u32(&mut encoded, compressed.len() as u32);
    for fingerprint in &page.boundary_fingerprints {
        encoded.extend_from_slice(fingerprint);
    }
    encoded.extend_from_slice(&page.content_fingerprint);
    debug_assert_eq!(encoded.len(), usize::from(PAGE_HEADER_LEN));
    for child in &page.children {
        encoded.push(child.key.level);
        encoded.extend_from_slice(&[0; 3]);
        for component in child.key.coord {
            push_i32(&mut encoded, component);
        }
        push_u64(&mut encoded, child.revision);
        encoded.extend_from_slice(&child.content_fingerprint);
    }
    for coverage in &page.materials {
        push_u16(&mut encoded, coverage.material.id());
        push_u16(&mut encoded, 0);
        push_u32(&mut encoded, coverage.occupied_voxels);
        push_u32(&mut encoded, coverage.exposed_unit_faces);
    }
    encoded.extend_from_slice(&compressed);
    Ok(encoded)
}

pub fn decode_terrain_page(
    bytes: &[u8],
    expected_source_identity_hash: WorldSourceIdentityHash,
) -> Result<TerrainPageV1, TerrainPageCodecError> {
    let mut cursor = PageCursor::new(bytes);
    if cursor.bytes(4)? != PAGE_MAGIC {
        return Err(TerrainPageCodecError::InvalidMagic);
    }
    let version = cursor.u16()?;
    if version != TERRAIN_PAGE_SCHEMA_VERSION {
        return Err(TerrainPageCodecError::UnsupportedVersion(version));
    }
    if cursor.u16()? != PAGE_HEADER_LEN {
        return Err(TerrainPageCodecError::InvalidHeader(
            "unexpected header length",
        ));
    }
    let source_identity_hash = WorldSourceIdentityHash::from_bytes(cursor.array()?);
    if source_identity_hash != expected_source_identity_hash {
        return Err(TerrainPageCodecError::SourceIdentityMismatch);
    }
    let level = cursor.u8()?;
    if cursor.bytes(3)? != [0; 3] {
        return Err(TerrainPageCodecError::InvalidHeader(
            "reserved key bytes are nonzero",
        ));
    }
    let key = TerrainPageKey {
        level,
        coord: [cursor.i32()?, cursor.i32()?, cursor.i32()?],
    };
    let revision = cursor.u64()?;
    let bounds = VoxelBounds::new(
        VoxelCoord::new(cursor.i32()?, cursor.i32()?, cursor.i32()?),
        VoxelCoord::new(cursor.i32()?, cursor.i32()?, cursor.i32()?),
    )
    .ok_or(TerrainPageCodecError::InvalidHeader(
        "invalid half-open bounds",
    ))?;
    let errors = TerrainErrorBounds {
        geometric_millivoxels: cursor.u32()?,
        silhouette_millivoxels: cursor.u32()?,
        material_boundary_millivoxels: cursor.u32()?,
        normal_milliradians: cursor.u32()?,
        unresolved_topology: false,
    };
    let topology = match cursor.u8()? {
        0 => TerrainTopologyClass::SingleRunColumns,
        1 => TerrainTopologyClass::Volumetric,
        _ => return Err(TerrainPageCodecError::InvalidHeader("unknown topology")),
    };
    let unresolved_topology = cursor.u8()?;
    if unresolved_topology > 1 {
        return Err(TerrainPageCodecError::InvalidHeader(
            "invalid topology error flag",
        ));
    }
    let errors = TerrainErrorBounds {
        unresolved_topology: unresolved_topology != 0,
        ..errors
    };
    let representation_kind = match cursor.u8()? {
        1 => TerrainPageRepresentationKind::SteppedSurfaceResidual,
        2 => TerrainPageRepresentationKind::SparseVoxelBrick,
        3 => TerrainPageRepresentationKind::SurfaceCluster,
        4 => TerrainPageRepresentationKind::TriangleCluster,
        5 => TerrainPageRepresentationKind::HeightfieldGrid,
        _ => {
            return Err(TerrainPageCodecError::InvalidHeader(
                "unknown representation",
            ));
        }
    };
    if cursor.u8()? != PAGE_COMPRESSION_BROTLI {
        return Err(TerrainPageCodecError::InvalidHeader(
            "unknown compression codec",
        ));
    }
    let child_count = usize::from(cursor.u16()?);
    let material_count = usize::from(cursor.u16()?);
    let payload_len = cursor.u32()? as usize;
    let compressed_len = cursor.u32()? as usize;
    if !matches!(
        child_count,
        0 | TERRAIN_SURFACE_PAGE_CHILDREN | TERRAIN_PAGE_MAX_CHILDREN
    ) {
        return Err(TerrainPageCodecError::LimitExceeded("child count"));
    }
    if material_count > Material::ALL.len() {
        return Err(TerrainPageCodecError::LimitExceeded("material count"));
    }
    if payload_len > TERRAIN_PAGE_MAX_PAYLOAD_BYTES {
        return Err(TerrainPageCodecError::LimitExceeded(
            "uncompressed payload bytes",
        ));
    }
    if compressed_len > TERRAIN_PAGE_MAX_COMPRESSED_BYTES {
        return Err(TerrainPageCodecError::LimitExceeded(
            "compressed payload bytes",
        ));
    }
    let mut boundary_fingerprints = [[0u8; 32]; 6];
    for fingerprint in &mut boundary_fingerprints {
        *fingerprint = cursor.array()?;
    }
    if cursor.position != usize::from(PAGE_HEADER_LEN) - 32 {
        return Err(TerrainPageCodecError::Truncated);
    }
    let content_fingerprint = cursor.array()?;
    let mut children = Vec::with_capacity(child_count);
    for _ in 0..child_count {
        let level = cursor.u8()?;
        if cursor.bytes(3)? != [0; 3] {
            return Err(TerrainPageCodecError::InvalidHeader(
                "reserved child bytes are nonzero",
            ));
        }
        children.push(TerrainPageChild {
            key: TerrainPageKey {
                level,
                coord: [cursor.i32()?, cursor.i32()?, cursor.i32()?],
            },
            revision: cursor.u64()?,
            content_fingerprint: cursor.array()?,
        });
    }
    let mut materials = Vec::with_capacity(material_count);
    for _ in 0..material_count {
        let id = cursor.u16()?;
        let material = Material::from_id(id).ok_or(TerrainPageCodecError::UnknownMaterial(id))?;
        if cursor.u16()? != 0 {
            return Err(TerrainPageCodecError::InvalidHeader(
                "reserved material bytes are nonzero",
            ));
        }
        materials.push(TerrainMaterialCoverage {
            material,
            occupied_voxels: cursor.u32()?,
            exposed_unit_faces: cursor.u32()?,
        });
    }
    let expected_total = cursor
        .position
        .checked_add(compressed_len)
        .ok_or(TerrainPageCodecError::LimitExceeded("encoded length"))?;
    if bytes.len() < expected_total {
        return Err(TerrainPageCodecError::Truncated);
    }
    if bytes.len() != expected_total {
        return Err(TerrainPageCodecError::InvalidHeader("trailing bytes"));
    }
    let payload = decompress_page_payload(cursor.bytes(compressed_len)?, payload_len)?;
    let representation = decode_representation(representation_kind, &payload)?;
    let page = TerrainPageV1 {
        source_identity_hash,
        key,
        revision,
        bounds,
        children,
        errors,
        topology,
        boundary_fingerprints,
        materials,
        representation,
        content_fingerprint,
    };
    if !representation_is_valid(&page)
        || !page_bounds_match_key(page.key, page.bounds)
        || !children_are_complete(&page)
    {
        return Err(TerrainPageCodecError::InvalidRepresentation(
            "page structure is inconsistent",
        ));
    }
    if terrain_page_fingerprint(&page) != page.content_fingerprint {
        return Err(TerrainPageCodecError::CorruptHash);
    }
    Ok(page)
}

fn compress_page_payload(payload: &[u8]) -> Result<Vec<u8>, TerrainPageCodecError> {
    let params = brotli::enc::BrotliEncoderParams {
        quality: PAGE_BROTLI_QUALITY,
        lgwin: PAGE_BROTLI_WINDOW_BITS,
        ..Default::default()
    };
    let mut input = payload;
    let mut compressed = Vec::new();
    brotli::BrotliCompress(&mut input, &mut compressed, &params)
        .map_err(|_| TerrainPageCodecError::Compression)?;
    Ok(compressed)
}

fn decompress_page_payload(
    compressed: &[u8],
    expected_len: usize,
) -> Result<Vec<u8>, TerrainPageCodecError> {
    let mut decompressed = Vec::with_capacity(expected_len);
    let decoder = brotli::Decompressor::new(compressed, PAGE_BROTLI_BUFFER_BYTES);
    decoder
        .take((expected_len + 1) as u64)
        .read_to_end(&mut decompressed)
        .map_err(|_| TerrainPageCodecError::Compression)?;
    if decompressed.len() != expected_len {
        return Err(TerrainPageCodecError::Compression);
    }
    Ok(decompressed)
}

fn decode_representation(
    kind: TerrainPageRepresentationKind,
    payload: &[u8],
) -> Result<TerrainPageRepresentation, TerrainPageCodecError> {
    let mut cursor = PageCursor::new(payload);
    let representation = match kind {
        TerrainPageRepresentationKind::SteppedSurfaceResidual => {
            let sample_stride_voxels = cursor.u32()?;
            let shape_xz = [cursor.u16()?, cursor.u16()?];
            let column_count = cursor.u32()? as usize;
            let run_count = cursor.u32()? as usize;
            let boundary_quad_count = cursor.u32()? as usize;
            let expected_columns =
                TERRAIN_PAGE_EDGE_SAMPLES as usize * TERRAIN_PAGE_EDGE_SAMPLES as usize;
            if column_count != expected_columns
                || run_count > expected_columns * 32
                || boundary_quad_count > TERRAIN_PAGE_MAX_PAYLOAD_BYTES / 19
            {
                return Err(TerrainPageCodecError::InvalidRepresentation(
                    "invalid stepped counts",
                ));
            }
            let mut columns = Vec::with_capacity(column_count);
            for _ in 0..column_count {
                columns.push(TerrainColumn {
                    first_run: cursor.u32()?,
                    run_count: cursor.u16()?,
                });
            }
            let mut runs = Vec::with_capacity(run_count);
            for _ in 0..run_count {
                runs.push(TerrainMaterialRun {
                    minimum_y: cursor.i32()?,
                    length: cursor.u16()?,
                    material_index: cursor.u8()?,
                });
            }
            let boundary_quads = decode_surface_quads(&mut cursor, boundary_quad_count)?;
            TerrainPageRepresentation::SteppedSurfaceResidual(SteppedSurfaceResidual {
                sample_stride_voxels,
                shape_xz,
                columns,
                runs,
                boundary_quads,
            })
        }
        TerrainPageRepresentationKind::SparseVoxelBrick => {
            let brick_edge = cursor.u8()?;
            let brick_count = cursor.u32()? as usize;
            let boundary_quad_count = cursor.u32()? as usize;
            if brick_edge != SPARSE_BRICK_EDGE
                || brick_count > 64
                || boundary_quad_count > TERRAIN_PAGE_MAX_PAYLOAD_BYTES / 19
            {
                return Err(TerrainPageCodecError::InvalidRepresentation(
                    "invalid sparse brick header",
                ));
            }
            let mut bricks = Vec::with_capacity(brick_count);
            for _ in 0..brick_count {
                let local_brick = [cursor.u8()?, cursor.u8()?, cursor.u8()?];
                let mut occupancy = [0u64; 8];
                for word in &mut occupancy {
                    *word = cursor.u64()?;
                }
                let material_count = usize::from(cursor.u16()?);
                if material_count > 512 {
                    return Err(TerrainPageCodecError::InvalidRepresentation(
                        "sparse brick material count",
                    ));
                }
                bricks.push(TerrainSparseBrick {
                    local_brick,
                    occupancy,
                    material_indices: cursor.bytes(material_count)?.to_vec(),
                });
            }
            let boundary_quads = decode_surface_quads(&mut cursor, boundary_quad_count)?;
            TerrainPageRepresentation::SparseVoxelBrick(SparseVoxelBrickPayload {
                brick_edge,
                bricks,
                boundary_quads,
            })
        }
        TerrainPageRepresentationKind::SurfaceCluster => {
            let quad_count = cursor.u32()? as usize;
            if quad_count > TERRAIN_PAGE_MAX_PAYLOAD_BYTES / 19 {
                return Err(TerrainPageCodecError::InvalidRepresentation(
                    "surface cluster count",
                ));
            }
            let quads = decode_surface_quads(&mut cursor, quad_count)?;
            TerrainPageRepresentation::SurfaceCluster(quads)
        }
        TerrainPageRepresentationKind::TriangleCluster => {
            let vertex_count = cursor.u32()? as usize;
            let triangle_count = cursor.u32()? as usize;
            if vertex_count == 0
                || triangle_count == 0
                || vertex_count > TERRAIN_PAGE_MAX_PAYLOAD_BYTES / 13
                || triangle_count > TERRAIN_PAGE_MAX_PAYLOAD_BYTES / 13
            {
                return Err(TerrainPageCodecError::InvalidRepresentation(
                    "triangle cluster counts",
                ));
            }
            let mut vertices = Vec::with_capacity(vertex_count);
            for _ in 0..vertex_count {
                vertices.push(TerrainClusterVertex {
                    position: [cursor.i32()?, cursor.i32()?, cursor.i32()?],
                    material_index: cursor.u8()?,
                });
            }
            let mut triangles = Vec::with_capacity(triangle_count);
            for _ in 0..triangle_count {
                triangles.push(TerrainClusterTriangle {
                    vertices: [cursor.u32()?, cursor.u32()?, cursor.u32()?],
                    material_index: cursor.u8()?,
                });
            }
            TerrainPageRepresentation::TriangleCluster(TerrainTriangleCluster {
                vertices,
                triangles,
            })
        }
        TerrainPageRepresentationKind::HeightfieldGrid => {
            let sample_stride_voxels = cursor.u32()?;
            let shape_xz = [cursor.u16()?, cursor.u16()?];
            let ground_count = cursor.u32()? as usize;
            let material_count = cursor.u32()? as usize;
            let water_count = cursor.u32()? as usize;
            let expected_edge = TERRAIN_PAGE_EDGE_SAMPLES as usize + 1;
            let expected_samples = expected_edge * expected_edge;
            let expected_materials =
                TERRAIN_PAGE_EDGE_SAMPLES as usize * TERRAIN_PAGE_EDGE_SAMPLES as usize;
            if ground_count != expected_samples
                || material_count != expected_materials
                || water_count != expected_samples
            {
                return Err(TerrainPageCodecError::InvalidRepresentation(
                    "heightfield grid counts",
                ));
            }
            let mut ground_heights = Vec::with_capacity(ground_count);
            for _ in 0..ground_count {
                ground_heights.push(cursor.i32()?);
            }
            let cell_material_indices = cursor.bytes(material_count)?.to_vec();
            let mut water_heights = Vec::with_capacity(water_count);
            for _ in 0..water_count {
                water_heights.push(cursor.i32()?);
            }
            TerrainPageRepresentation::HeightfieldGrid(TerrainHeightfieldGrid {
                sample_stride_voxels,
                shape_xz,
                ground_heights,
                cell_material_indices,
                water_heights,
            })
        }
    };
    if cursor.position != payload.len() {
        return Err(TerrainPageCodecError::InvalidRepresentation(
            "trailing payload bytes",
        ));
    }
    Ok(representation)
}

fn decode_surface_quads(
    cursor: &mut PageCursor<'_>,
    quad_count: usize,
) -> Result<Vec<TerrainSurfaceQuad>, TerrainPageCodecError> {
    let mut quads = Vec::with_capacity(quad_count);
    for _ in 0..quad_count {
        let axis = match cursor.u8()? {
            0 => FaceAxis::X,
            1 => FaceAxis::Y,
            2 => FaceAxis::Z,
            _ => {
                return Err(TerrainPageCodecError::InvalidRepresentation(
                    "surface cluster axis",
                ));
            }
        };
        let positive = cursor.u8()?;
        if positive > 1 {
            return Err(TerrainPageCodecError::InvalidRepresentation(
                "surface cluster side",
            ));
        }
        quads.push(TerrainSurfaceQuad {
            axis,
            positive: positive != 0,
            material_index: cursor.u8()?,
            plane: cursor.i32()?,
            u: cursor.i32()?,
            v: cursor.i32()?,
            width: cursor.u16()?,
            height: cursor.u16()?,
        });
    }
    Ok(quads)
}

struct PageCursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> PageCursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn bytes(&mut self, length: usize) -> Result<&'a [u8], TerrainPageCodecError> {
        let end = self
            .position
            .checked_add(length)
            .ok_or(TerrainPageCodecError::Truncated)?;
        let bytes = self
            .bytes
            .get(self.position..end)
            .ok_or(TerrainPageCodecError::Truncated)?;
        self.position = end;
        Ok(bytes)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], TerrainPageCodecError> {
        self.bytes(N)?
            .try_into()
            .map_err(|_| TerrainPageCodecError::Truncated)
    }

    fn u8(&mut self) -> Result<u8, TerrainPageCodecError> {
        self.bytes(1).and_then(|bytes| {
            bytes
                .first()
                .copied()
                .ok_or(TerrainPageCodecError::Truncated)
        })
    }

    fn u16(&mut self) -> Result<u16, TerrainPageCodecError> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    fn u32(&mut self) -> Result<u32, TerrainPageCodecError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64, TerrainPageCodecError> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    fn i32(&mut self) -> Result<i32, TerrainPageCodecError> {
        Ok(i32::from_le_bytes(self.array()?))
    }
}

fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_i32(bytes: &mut Vec<u8>, value: i32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SurfaceRegion;

    fn identity() -> WorldSourceIdentityHash {
        WorldSourceIdentityHash::from_bytes([0x5a; 32])
    }

    fn sampled_surface(height: i32, material: Material, water_level: Option<i32>) -> SurfaceSample {
        SurfaceSample {
            height,
            material,
            water_level,
            region: SurfaceRegion::VerdantForest,
            moisture: 0.5,
            temperature: 0.5,
            ridge: 0.0,
            route: None,
        }
    }

    fn heightfield_samples(key: TerrainPageKey) -> Vec<SurfaceSample> {
        let bounds = key.bounds().unwrap();
        let stride = 1i32 << key.level;
        (0..=TERRAIN_PAGE_EDGE_SAMPLES as i32)
            .flat_map(|z| {
                (0..=TERRAIN_PAGE_EDGE_SAMPLES as i32).map(move |x| {
                    let world_x = bounds.min.x + x * stride;
                    let world_z = bounds.min.z + z * stride;
                    let height = 24 + world_x.div_euclid(32) + world_z.div_euclid(48);
                    let material =
                        if (world_x.div_euclid(16) + world_z.div_euclid(16)).rem_euclid(3) == 0 {
                            Material::Dirt
                        } else {
                            Material::Grass
                        };
                    sampled_surface(height, material, (height < 20).then_some(20))
                })
            })
            .collect()
    }

    fn surface_heightfield_samples(key: TerrainPageKey) -> Vec<SurfaceSample> {
        let [[minimum_x, minimum_z], _] = key.horizontal_bounds().unwrap();
        let stride = 1i32 << key.level;
        (0..=TERRAIN_PAGE_EDGE_SAMPLES as i32)
            .flat_map(|z| {
                (0..=TERRAIN_PAGE_EDGE_SAMPLES as i32).map(move |x| {
                    let world_x = minimum_x + x * stride;
                    let world_z = minimum_z + z * stride;
                    let height = (world_x + world_z).div_euclid(8);
                    let material =
                        if (world_x.div_euclid(16) + world_z.div_euclid(16)).rem_euclid(2) == 0 {
                            Material::Dirt
                        } else {
                            Material::Grass
                        };
                    sampled_surface(height, material, None)
                })
            })
            .collect()
    }

    #[test]
    fn sampled_heightfield_round_trips_and_adjacent_negative_pages_share_their_edge() {
        let left_key = TerrainPageKey {
            level: 2,
            coord: [-1, 0, -1],
        };
        let right_key = TerrainPageKey {
            level: 2,
            coord: [0, 0, -1],
        };
        let left = build_sampled_heightfield_terrain_page(
            identity(),
            left_key,
            17,
            &heightfield_samples(left_key),
            TerrainErrorBounds::EXACT,
        )
        .unwrap();
        let right = build_sampled_heightfield_terrain_page(
            identity(),
            right_key,
            17,
            &heightfield_samples(right_key),
            TerrainErrorBounds::EXACT,
        )
        .unwrap();

        assert_eq!(
            left.boundary_fingerprints[BoundarySide::PositiveX as usize],
            right.boundary_fingerprints[BoundarySide::NegativeX as usize]
        );
        assert!(left.errors.geometric_millivoxels >= 4_000);
        assert!(matches!(
            left.representation,
            TerrainPageRepresentation::HeightfieldGrid(_)
        ));
        let encoded = encode_terrain_page(&left).unwrap();
        assert!(encoded.len() <= TERRAIN_PAGE_TARGET_COMPRESSED_BYTES);
        assert_eq!(decode_terrain_page(&encoded, identity()).unwrap(), left);
    }

    #[test]
    fn sampled_heightfield_rejects_surface_outside_its_vertical_owner() {
        let key = TerrainPageKey {
            level: 1,
            coord: [0, 0, 0],
        };
        let samples = vec![
            sampled_surface(key.bounds().unwrap().max.y, Material::Grass, None);
            (TERRAIN_PAGE_EDGE_SAMPLES as usize + 1).pow(2)
        ];
        assert_eq!(
            build_sampled_heightfield_terrain_page(
                identity(),
                key,
                1,
                &samples,
                TerrainErrorBounds::EXACT,
            ),
            Err(TerrainPageBuildError::InvalidSurfaceLattice)
        );
    }

    #[test]
    fn signed_surface_page_keys_form_an_exact_horizontal_quadtree() {
        let key = TerrainPageKey::surface(2, -1, 1);
        assert!(key.is_surface());
        assert_eq!(key.coord[1], TERRAIN_SURFACE_PAGE_COORD_Y);
        assert_eq!(key.bounds(), None);
        assert_eq!(key.horizontal_bounds(), Some([[-128, 128], [0, 256]]));

        let children = key.refinement_children().unwrap();
        assert_eq!(children.len(), TERRAIN_SURFACE_PAGE_CHILDREN);
        assert_eq!(
            children,
            vec![
                TerrainPageKey::surface(1, -2, 2),
                TerrainPageKey::surface(1, -1, 2),
                TerrainPageKey::surface(1, -2, 3),
                TerrainPageKey::surface(1, -1, 3),
            ]
        );
        assert!(children.iter().all(|child| child.parent() == Some(key)));
        assert_eq!(
            TerrainPageKey::surface(0, -1, -1).ancestor_at(2).unwrap(),
            TerrainPageKey::surface(2, -1, -1)
        );
    }

    #[test]
    fn surface_quadtree_replacement_crosses_zero_height_without_clipping() {
        let key = TerrainPageKey::surface(1, -1, 0);
        let children = key
            .refinement_children()
            .unwrap()
            .into_iter()
            .map(|child| {
                build_sampled_heightfield_terrain_page(
                    identity(),
                    child,
                    27,
                    &surface_heightfield_samples(child),
                    TerrainErrorBounds::EXACT,
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        let sampled_parent = build_sampled_heightfield_terrain_page(
            identity(),
            key,
            28,
            &surface_heightfield_samples(key),
            TerrainErrorBounds::EXACT,
        )
        .unwrap();
        let parent = assemble_terrain_parent(
            key,
            28,
            sampled_parent.errors,
            sampled_parent.topology,
            sampled_parent.materials.clone(),
            sampled_parent.representation.clone(),
            &children,
        )
        .unwrap();

        assert!(parent.bounds.min.y < 0);
        assert!(parent.bounds.max.y > 0);
        assert_eq!(parent.children.len(), TERRAIN_SURFACE_PAGE_CHILDREN);
        validate_terrain_replacement(&parent, &children).unwrap();
        assert_eq!(
            decode_terrain_page(&encode_terrain_page(&parent).unwrap(), identity()).unwrap(),
            parent
        );
    }

    fn terrain(coord: VoxelCoord) -> Material {
        let height = (coord.x.div_euclid(7) + coord.z.div_euclid(11)).rem_euclid(9) - 4;
        if coord.y > height {
            Material::Air
        } else if coord.y == height {
            Material::Grass
        } else if coord.y >= height - 2 {
            Material::Dirt
        } else {
            Material::Stone
        }
    }

    fn material_from_payload(page: &TerrainPageV1, coord: VoxelCoord) -> Material {
        let palette = &page.materials;
        match &page.representation {
            TerrainPageRepresentation::SteppedSurfaceResidual(surface) => {
                let x = usize::try_from(coord.x - page.bounds.min.x).unwrap();
                let z = usize::try_from(coord.z - page.bounds.min.z).unwrap();
                let column = surface.columns[x + z * usize::from(surface.shape_xz[0])];
                surface.runs[column.first_run as usize
                    ..column.first_run as usize + usize::from(column.run_count)]
                    .iter()
                    .find(|run| {
                        coord.y >= run.minimum_y && coord.y < run.minimum_y + i32::from(run.length)
                    })
                    .map_or(Material::Air, |run| {
                        palette[usize::from(run.material_index)].material
                    })
            }
            TerrainPageRepresentation::SparseVoxelBrick(payload) => {
                let local = [
                    u8::try_from(coord.x - page.bounds.min.x).unwrap(),
                    u8::try_from(coord.y - page.bounds.min.y).unwrap(),
                    u8::try_from(coord.z - page.bounds.min.z).unwrap(),
                ];
                let brick_coord = local.map(|component| component / payload.brick_edge);
                let Some(brick) = payload
                    .bricks
                    .iter()
                    .find(|brick| brick.local_brick == brick_coord)
                else {
                    return Material::Air;
                };
                let local = local.map(|component| component % payload.brick_edge);
                let edge = usize::from(payload.brick_edge);
                let index = usize::from(local[0])
                    + usize::from(local[1]) * edge
                    + usize::from(local[2]) * edge * edge;
                if brick.occupancy[index / 64] & (1u64 << (index % 64)) == 0 {
                    return Material::Air;
                }
                let rank = brick.occupancy[..index / 64]
                    .iter()
                    .map(|word| word.count_ones() as usize)
                    .sum::<usize>()
                    + (brick.occupancy[index / 64] & ((1u64 << (index % 64)) - 1)).count_ones()
                        as usize;
                palette[usize::from(brick.material_indices[rank])].material
            }
            TerrainPageRepresentation::SurfaceCluster(_)
            | TerrainPageRepresentation::TriangleCluster(_)
            | TerrainPageRepresentation::HeightfieldGrid(_) => Material::Air,
        }
    }

    fn solid_stepped_leaf(key: TerrainPageKey, revision: u64) -> TerrainPageV1 {
        let mut page =
            build_exact_terrain_page(identity(), key, revision, |_| Material::Stone).unwrap();
        let runs = (0..TERRAIN_PAGE_EDGE_SAMPLES * TERRAIN_PAGE_EDGE_SAMPLES)
            .map(|_| TerrainMaterialRun {
                minimum_y: page.bounds.min.y,
                length: TERRAIN_PAGE_EDGE_SAMPLES as u16,
                material_index: 0,
            })
            .collect::<Vec<_>>();
        let columns = (0..runs.len())
            .map(|index| TerrainColumn {
                first_run: index as u32,
                run_count: 1,
            })
            .collect();
        page.representation =
            TerrainPageRepresentation::SteppedSurfaceResidual(SteppedSurfaceResidual {
                sample_stride_voxels: 1,
                shape_xz: [
                    TERRAIN_PAGE_EDGE_SAMPLES as u16,
                    TERRAIN_PAGE_EDGE_SAMPLES as u16,
                ],
                columns,
                runs,
                boundary_quads: Vec::new(),
            });
        page.content_fingerprint = terrain_page_fingerprint(&page);
        assert!(page.validates_identity());
        page
    }

    #[test]
    fn signed_page_keys_have_exact_nested_half_open_bounds() {
        let key = TerrainPageKey {
            level: 0,
            coord: [-2, -1, 3],
        };
        assert_eq!(
            key.bounds().unwrap(),
            VoxelBounds::new(VoxelCoord::new(-64, -32, 96), VoxelCoord::new(-32, 0, 128)).unwrap()
        );
        let parent = key.parent().unwrap();
        assert_eq!(parent.coord, [-1, -1, 1]);
        assert!(parent.children().unwrap().contains(&key));
        assert_eq!(key.ancestor_at(3).unwrap().coord, [-1, -1, 0]);
        assert_eq!(key.ancestor_at(3).unwrap().level, 3);
        assert_eq!(parent.ancestor_at(0), None);
    }

    #[test]
    fn exact_heightfield_leaf_round_trips_every_face_and_is_deterministic() {
        let key = TerrainPageKey {
            level: 0,
            coord: [-1, -1, -1],
        };
        let first = build_compact_exact_terrain_page(identity(), key, 7, terrain).unwrap();
        let second = build_compact_exact_terrain_page(identity(), key, 7, terrain).unwrap();
        assert_eq!(first, second);
        assert!(first.validates_identity());
        assert_eq!(first.topology, TerrainTopologyClass::SingleRunColumns);
        assert_eq!(
            clustered_face_set(&first),
            canonical_exposed_faces(first.bounds, terrain)
                .into_iter()
                .collect()
        );
    }

    #[test]
    fn volumetric_leaf_never_uses_the_stepped_payload() {
        let key = TerrainPageKey {
            level: 0,
            coord: [0, 0, 0],
        };
        let sampler = |coord: VoxelCoord| {
            let ground = coord.y <= 5;
            let roof = (12..=14).contains(&coord.y) && (5..28).contains(&coord.x);
            let floating = coord == VoxelCoord::new(4, 24, 4);
            if floating {
                Material::GlowCrystal
            } else if roof {
                Material::Basalt
            } else if ground {
                Material::Stone
            } else {
                Material::Air
            }
        };
        let page = build_exact_terrain_page(identity(), key, 11, sampler).unwrap();
        assert_eq!(page.topology, TerrainTopologyClass::Volumetric);
        assert!(!matches!(
            page.representation,
            TerrainPageRepresentation::SteppedSurfaceResidual(_)
        ));
        assert!(page.validates_identity());
        if matches!(
            page.representation,
            TerrainPageRepresentation::SparseVoxelBrick(_)
        ) {
            for z in 0..32 {
                for y in 0..32 {
                    for x in 0..32 {
                        let coord = VoxelCoord::new(x, y, z);
                        assert_eq!(material_from_payload(&page, coord), sampler(coord));
                    }
                }
            }
        }
    }

    #[test]
    fn adjacent_exact_pages_publish_identical_shared_boundary_hashes() {
        let left = build_exact_terrain_page(
            identity(),
            TerrainPageKey {
                level: 0,
                coord: [-1, 0, -1],
            },
            1,
            terrain,
        )
        .unwrap();
        let right = build_exact_terrain_page(
            identity(),
            TerrainPageKey {
                level: 0,
                coord: [0, 0, -1],
            },
            1,
            terrain,
        )
        .unwrap();
        assert_eq!(
            left.boundary_fingerprints[BoundarySide::PositiveX as usize],
            right.boundary_fingerprints[BoundarySide::NegativeX as usize]
        );
    }

    #[test]
    fn fingerprint_binds_revision_payload_and_source() {
        let key = TerrainPageKey {
            level: 0,
            coord: [0, -1, 0],
        };
        let page = build_exact_terrain_page(identity(), key, 1, terrain).unwrap();
        let changed_revision = build_exact_terrain_page(identity(), key, 2, terrain).unwrap();
        let changed_source = build_exact_terrain_page(
            WorldSourceIdentityHash::from_bytes([0x6b; 32]),
            key,
            1,
            terrain,
        )
        .unwrap();
        assert_ne!(
            page.content_fingerprint,
            changed_revision.content_fingerprint
        );
        assert_ne!(page.content_fingerprint, changed_source.content_fingerprint);
        let mut corrupted = page.clone();
        corrupted.revision += 1;
        assert!(!corrupted.validates_identity());
    }

    #[test]
    fn vxtp_codec_round_trips_stepped_and_volumetric_pages() {
        let stepped = solid_stepped_leaf(
            TerrainPageKey {
                level: 0,
                coord: [-1, -1, -1],
            },
            19,
        );
        let volumetric_sampler = |coord: VoxelCoord| {
            if coord.y <= 2 || ((12..=14).contains(&coord.y) && coord.x < 24) {
                Material::Stone
            } else {
                Material::Air
            }
        };
        let volumetric = build_exact_terrain_page(
            identity(),
            TerrainPageKey {
                level: 0,
                coord: [0, 0, 0],
            },
            20,
            volumetric_sampler,
        )
        .unwrap();
        for page in [stepped, volumetric] {
            let encoded = encode_terrain_page(&page).unwrap();
            assert!(encoded.len() < TERRAIN_PAGE_MAX_COMPRESSED_BYTES);
            assert_eq!(decode_terrain_page(&encoded, identity()).unwrap(), page);
            assert_eq!(encode_terrain_page(&page).unwrap(), encoded);
        }
    }

    fn clustered_face_set(page: &TerrainPageV1) -> BTreeSet<CanonicalFaceKey> {
        let TerrainPageRepresentation::SurfaceCluster(quads) = &page.representation else {
            panic!("expected clustered terrain page");
        };
        surface_face_set(page, quads)
    }

    fn surface_face_set(
        page: &TerrainPageV1,
        quads: &[TerrainSurfaceQuad],
    ) -> BTreeSet<CanonicalFaceKey> {
        let mut faces = BTreeSet::new();
        for quad in quads {
            let material_id = page.materials[usize::from(quad.material_index)]
                .material
                .id();
            expand_surface_quad(*quad, material_id, &mut faces);
        }
        faces
    }

    #[test]
    fn compact_stepped_surface_reconstructs_the_exact_halo_owned_faces() {
        let key = TerrainPageKey {
            level: 0,
            coord: [-2, -1, 1],
        };
        let sampler = |coord: VoxelCoord| {
            let height = -8 + (coord.x + coord.z).rem_euclid(2);
            if coord.y <= height {
                Material::Stone
            } else {
                Material::Air
            }
        };
        let page = build_compact_exact_terrain_page(identity(), key, 91, sampler).unwrap();
        assert!(matches!(
            page.representation,
            TerrainPageRepresentation::SteppedSurfaceResidual(_)
        ));
        let reconstructed = reconstruct_exact_terrain_surface(&page).unwrap();
        assert_eq!(
            surface_face_set(&page, &reconstructed),
            canonical_exposed_faces(page.bounds, sampler)
                .into_iter()
                .collect()
        );
        assert_eq!(
            decode_terrain_page(&encode_terrain_page(&page).unwrap(), identity()).unwrap(),
            page
        );
    }

    #[test]
    fn compact_sparse_bricks_reconstruct_the_exact_halo_owned_faces() {
        let key = TerrainPageKey {
            level: 0,
            coord: [-1, 0, -2],
        };
        let sampler = |coord: VoxelCoord| {
            let hash = coord.x.wrapping_mul(73_856_093)
                ^ coord.y.wrapping_mul(19_349_663)
                ^ coord.z.wrapping_mul(83_492_791);
            if hash.rem_euclid(97) == 0 {
                Material::Basalt
            } else {
                Material::Air
            }
        };
        let page = build_compact_exact_terrain_page(identity(), key, 92, sampler).unwrap();
        assert!(matches!(
            page.representation,
            TerrainPageRepresentation::SparseVoxelBrick(_)
        ));
        let reconstructed = reconstruct_exact_terrain_surface(&page).unwrap();
        assert_eq!(
            surface_face_set(&page, &reconstructed),
            canonical_exposed_faces(page.bounds, sampler)
                .into_iter()
                .collect()
        );
        assert_eq!(
            decode_terrain_page(&encode_terrain_page(&page).unwrap(), identity()).unwrap(),
            page
        );
    }

    fn exact_cluster_children(key: TerrainPageKey, revision: u64) -> Vec<TerrainPageV1> {
        key.children()
            .unwrap()
            .into_iter()
            .map(|child| build_exact_terrain_page(identity(), child, revision, terrain).unwrap())
            .collect()
    }

    #[test]
    fn exact_cluster_parent_is_the_canonical_union_at_negative_coordinates() {
        let key = TerrainPageKey {
            level: 1,
            coord: [-1, -1, -1],
        };
        let children = exact_cluster_children(key, 41);
        let expected = children
            .iter()
            .flat_map(clustered_face_set)
            .collect::<BTreeSet<_>>();
        let parent = build_exact_cluster_terrain_parent(key, 42, &children).unwrap();
        assert_eq!(clustered_face_set(&parent), expected);
        assert_eq!(parent.errors, TerrainErrorBounds::EXACT);
        validate_terrain_replacement(&parent, &children).unwrap();
        let encoded = encode_terrain_page(&parent).unwrap();
        assert_eq!(decode_terrain_page(&encoded, identity()).unwrap(), parent);
    }

    #[test]
    fn exact_cluster_hierarchy_remains_closed_through_two_levels() {
        let root_key = TerrainPageKey {
            level: 2,
            coord: [-1, -1, 0],
        };
        let level_one = root_key
            .children()
            .unwrap()
            .into_iter()
            .map(|key| {
                let leaves = exact_cluster_children(key, 51);
                build_exact_cluster_terrain_parent(key, 52, &leaves).unwrap()
            })
            .collect::<Vec<_>>();
        let expected = level_one
            .iter()
            .flat_map(clustered_face_set)
            .collect::<BTreeSet<_>>();
        let root = build_exact_cluster_terrain_parent(root_key, 53, &level_one).unwrap();
        assert_eq!(clustered_face_set(&root), expected);
        validate_terrain_replacement(&root, &level_one).unwrap();
    }

    #[test]
    fn exact_cluster_parent_rejects_compact_noncluster_children() {
        let key = TerrainPageKey {
            level: 1,
            coord: [0, -1, 0],
        };
        let children = key
            .children()
            .unwrap()
            .into_iter()
            .map(|child| solid_stepped_leaf(child, 61))
            .collect::<Vec<_>>();
        assert!(children.iter().any(|child| {
            !matches!(
                child.representation,
                TerrainPageRepresentation::SurfaceCluster(_)
            )
        }));
        assert_eq!(
            build_exact_cluster_terrain_parent(key, 62, &children),
            Err(TerrainPageBuildError::UnsupportedChildRepresentation)
        );
    }

    #[cfg(feature = "terrain-page-builder")]
    type TopologicalVertex = ([i32; 3], u8);

    #[cfg(feature = "terrain-page-builder")]
    type TopologicalEdge = (TopologicalVertex, TopologicalVertex);

    #[cfg(feature = "terrain-page-builder")]
    fn topological_boundary_edges(cluster: &TerrainTriangleCluster) -> BTreeSet<TopologicalEdge> {
        let mut counts = BTreeMap::new();
        for triangle in &cluster.triangles {
            for [left, right] in [
                [triangle.vertices[0], triangle.vertices[1]],
                [triangle.vertices[1], triangle.vertices[2]],
                [triangle.vertices[2], triangle.vertices[0]],
            ] {
                let left = cluster.vertices[left as usize];
                let right = cluster.vertices[right as usize];
                let left = (left.position, left.material_index);
                let right = (right.position, right.material_index);
                let key = if left < right {
                    (left, right)
                } else {
                    (right, left)
                };
                *counts.entry(key).or_insert(0u8) += 1;
            }
        }
        counts
            .into_iter()
            .filter_map(|(edge, count)| (count == 1).then_some(edge))
            .collect()
    }

    #[cfg(feature = "terrain-page-builder")]
    #[test]
    fn simplified_triangle_parent_locks_boundaries_and_meets_page_budget() {
        let key = TerrainPageKey {
            level: 2,
            coord: [-1, -1, -1],
        };
        let sloped = |coord: VoxelCoord| {
            let height = -64 + (coord.x + 128).div_euclid(4) + (coord.z + 128).div_euclid(8);
            if coord.y <= height {
                if coord.x < -64 {
                    Material::Stone
                } else {
                    Material::Dirt
                }
            } else {
                Material::Air
            }
        };
        let children = key
            .children()
            .unwrap()
            .into_iter()
            .map(|child_key| {
                let leaves = child_key
                    .children()
                    .unwrap()
                    .into_iter()
                    .map(|leaf| build_exact_terrain_page(identity(), leaf, 71, sloped).unwrap())
                    .collect::<Vec<_>>();
                build_exact_cluster_terrain_parent(child_key, 72, &leaves).unwrap()
            })
            .collect::<Vec<_>>();
        let exact = build_exact_cluster_terrain_parent(key, 73, &children).unwrap();
        let TerrainPageRepresentation::SurfaceCluster(exact_quads) = &exact.representation else {
            unreachable!();
        };
        let exact_triangles = triangulate_surface_quads(exact_quads, exact.bounds).unwrap();
        let budget = TerrainSimplificationBudget {
            target_triangles: 4_096,
            max_error_millivoxels: 8_000,
            target_encoded_bytes: TERRAIN_PAGE_TARGET_COMPRESSED_BYTES as u32,
        };
        let budgeted = build_budgeted_terrain_parent(key, 74, &children, budget).unwrap();
        assert!(matches!(
            budgeted.representation,
            TerrainPageRepresentation::SurfaceCluster(_)
        ));
        let simplified =
            build_simplified_triangle_terrain_parent(key, 75, &children, budget).unwrap();
        let TerrainPageRepresentation::TriangleCluster(cluster) = &simplified.representation else {
            panic!("expected triangle cluster");
        };
        assert!(cluster.triangles.len() < exact_triangles.triangles.len());
        assert_eq!(
            topological_boundary_edges(cluster),
            topological_boundary_edges(&exact_triangles)
        );
        assert!(simplified.errors.geometric_millivoxels <= 8_000);
        assert!(simplified.errors.silhouette_millivoxels <= 8_000);
        assert_eq!(simplified.errors.material_boundary_millivoxels, 0);
        assert!(!simplified.errors.unresolved_topology);
        assert!(simplified.validates_identity());
        validate_terrain_replacement(&simplified, &children).unwrap();
        let encoded = encode_terrain_page(&simplified).unwrap();
        assert!(encoded.len() <= TERRAIN_PAGE_TARGET_COMPRESSED_BYTES);
        assert_eq!(
            decode_terrain_page(&encoded, identity()).unwrap(),
            simplified
        );
    }

    #[cfg(feature = "terrain-page-builder")]
    #[test]
    fn exact_surface_sidecars_build_ancestors_over_simplified_children() {
        let root_key = TerrainPageKey {
            level: 2,
            coord: [-1, -1, 0],
        };
        let sloped = |coord: VoxelCoord| {
            let height = -32 + coord.x.div_euclid(5) - coord.z.div_euclid(7);
            if coord.y <= height {
                Material::Stone
            } else {
                Material::Air
            }
        };
        let budget = TerrainSimplificationBudget {
            target_triangles: 4_096,
            max_error_millivoxels: 8_000,
            target_encoded_bytes: TERRAIN_PAGE_TARGET_COMPRESSED_BYTES as u32,
        };
        let mut published_children = Vec::new();
        let mut exact_children = Vec::new();
        for child_key in root_key.children().unwrap() {
            let leaves = child_key
                .children()
                .unwrap()
                .into_iter()
                .map(|leaf| build_exact_terrain_page(identity(), leaf, 81, sloped).unwrap())
                .collect::<Vec<_>>();
            let exact = assemble_exact_cluster_terrain_parent(child_key, 82, &leaves).unwrap();
            let published = match &exact.representation {
                TerrainPageRepresentation::SurfaceCluster(quads) if quads.is_empty() => {
                    exact.clone()
                }
                TerrainPageRepresentation::SurfaceCluster(_) => {
                    simplify_exact_terrain_parent(&exact, &leaves, budget).unwrap()
                }
                _ => unreachable!(),
            };
            validate_terrain_replacement(&published, &leaves).unwrap();
            exact_children.push(exact);
            published_children.push(published);
        }
        assert!(published_children.iter().any(|page| matches!(
            page.representation,
            TerrainPageRepresentation::TriangleCluster(_)
        )));

        let exact_root = assemble_exact_cluster_terrain_parent_from_surfaces(
            root_key,
            83,
            &published_children,
            &exact_children,
        )
        .unwrap();
        let published_root =
            select_budgeted_exact_terrain_parent(&exact_root, &published_children, budget).unwrap();
        validate_terrain_replacement(&published_root, &published_children).unwrap();
        assert!(
            published_root.errors.geometric_millivoxels
                >= published_children
                    .iter()
                    .map(|child| child.errors.geometric_millivoxels)
                    .max()
                    .unwrap()
        );
        assert_eq!(
            clustered_face_set(&exact_root),
            exact_children.iter().flat_map(clustered_face_set).collect()
        );
    }

    #[test]
    fn vxtp_codec_rejects_wrong_source_corruption_limits_and_trailing_bytes() {
        let page = build_exact_terrain_page(
            identity(),
            TerrainPageKey {
                level: 0,
                coord: [0, -1, 0],
            },
            23,
            terrain,
        )
        .unwrap();
        let encoded = encode_terrain_page(&page).unwrap();
        assert_eq!(
            decode_terrain_page(&encoded, WorldSourceIdentityHash::from_bytes([0x33; 32])),
            Err(TerrainPageCodecError::SourceIdentityMismatch)
        );

        let mut corrupted = encoded.clone();
        let last = corrupted.len() - 1;
        corrupted[last] ^= 0x80;
        assert!(matches!(
            decode_terrain_page(&corrupted, identity()),
            Err(TerrainPageCodecError::Compression | TerrainPageCodecError::CorruptHash)
        ));

        let mut oversized = encoded.clone();
        oversized[116..120].copy_from_slice(
            &u32::try_from(TERRAIN_PAGE_MAX_COMPRESSED_BYTES + 1)
                .unwrap()
                .to_le_bytes(),
        );
        assert_eq!(
            decode_terrain_page(&oversized, identity()),
            Err(TerrainPageCodecError::LimitExceeded(
                "compressed payload bytes"
            ))
        );

        let mut trailing = encoded;
        trailing.push(0);
        assert_eq!(
            decode_terrain_page(&trailing, identity()),
            Err(TerrainPageCodecError::InvalidHeader("trailing bytes"))
        );
    }

    fn solid_parent_fixture() -> (TerrainPageV1, Vec<TerrainPageV1>) {
        let key = TerrainPageKey {
            level: 1,
            coord: [0, -1, 0],
        };
        let children = key
            .children()
            .unwrap()
            .into_iter()
            .map(|child| {
                build_exact_terrain_page(identity(), child, 31, |_| Material::Stone).unwrap()
            })
            .collect::<Vec<_>>();
        let runs = (0..TERRAIN_PAGE_EDGE_SAMPLES * TERRAIN_PAGE_EDGE_SAMPLES)
            .map(|_| TerrainMaterialRun {
                minimum_y: -64,
                length: 64,
                material_index: 0,
            })
            .collect::<Vec<_>>();
        let columns = (0..runs.len())
            .map(|index| TerrainColumn {
                first_run: index as u32,
                run_count: 1,
            })
            .collect();
        let parent = assemble_terrain_parent(
            key,
            32,
            TerrainErrorBounds::EXACT,
            TerrainTopologyClass::SingleRunColumns,
            vec![TerrainMaterialCoverage {
                material: Material::Stone,
                occupied_voxels: 64 * 64 * 64,
                exposed_unit_faces: 0,
            }],
            TerrainPageRepresentation::SteppedSurfaceResidual(SteppedSurfaceResidual {
                sample_stride_voxels: 2,
                shape_xz: [32, 32],
                columns,
                runs,
                boundary_quads: Vec::new(),
            }),
            &children,
        )
        .unwrap();
        (parent, children)
    }

    #[test]
    fn atomic_replacement_binds_complete_children_and_composable_boundaries() {
        let (parent, children) = solid_parent_fixture();
        assert!(parent.validates_identity());
        validate_terrain_replacement(&parent, &children).unwrap();
        let encoded = encode_terrain_page(&parent).unwrap();
        assert_eq!(decode_terrain_page(&encoded, identity()).unwrap(), parent);

        let mut reordered = children.clone();
        reordered.reverse();
        validate_terrain_replacement(&parent, &reordered).unwrap();

        assert_eq!(
            validate_terrain_replacement(&parent, &children[..7]),
            Err(TerrainReplacementError::WrongChildCount)
        );
    }

    #[test]
    fn atomic_replacement_rejects_stale_and_boundary_incoherent_children() {
        let (parent, children) = solid_parent_fixture();
        let mut stale = children.clone();
        stale[0] =
            build_exact_terrain_page(identity(), stale[0].key, 99, |_| Material::Stone).unwrap();
        assert_eq!(
            validate_terrain_replacement(&parent, &stale),
            Err(TerrainReplacementError::ChildReferenceMismatch)
        );

        let mut incoherent = children;
        let boundary_x = incoherent[0].bounds.max.x - 1;
        incoherent[0] = build_exact_terrain_page(identity(), incoherent[0].key, 31, |coord| {
            if coord.x == boundary_x {
                Material::Air
            } else {
                Material::Stone
            }
        })
        .unwrap();
        assert_eq!(
            validate_terrain_replacement(&parent, &incoherent),
            Err(TerrainReplacementError::InternalBoundaryMismatch)
        );
    }
}
