//! Versioned virtual microvoxel terrain pages.
//!
//! Pages retain canonical 10 cm integer ownership while allowing different payload encodings.
//! Representation is never ownership: every page key identifies one half-open spatial owner, and
//! every payload reconstructs only that owner's certified surface.

use crate::{
    BoundaryCertificate, BoundarySide, CanonicalFaceKey, FaceAxis, Material, VoxelBounds,
    VoxelCoord, WorldSourceIdentityHash, canonical_exposed_faces,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::Read;

pub const TERRAIN_PAGE_SCHEMA_VERSION: u16 = 1;
pub const TERRAIN_PAGE_EDGE_SAMPLES: u32 = 32;
pub const TERRAIN_PAGE_MAX_LEVEL: u8 = 20;
pub const TERRAIN_PAGE_MAX_CHILDREN: usize = 8;
pub const TERRAIN_PAGE_MAX_COMPRESSED_BYTES: usize = 262_144;
pub const TERRAIN_PAGE_MAX_PAYLOAD_BYTES: usize = 2_097_152;
const SPARSE_BRICK_EDGE: u8 = 8;
const PAGE_FINGERPRINT_DOMAIN: &[u8] = b"voxels-terrain-page-v1\0";
const PAGE_MAGIC: &[u8; 4] = b"VXTP";
const PAGE_HEADER_LEN: u16 = 344;
const PAGE_COMPRESSION_BROTLI: u8 = 1;
const PAGE_BROTLI_QUALITY: i32 = 5;
const PAGE_BROTLI_WINDOW_BITS: i32 = 20;
const PAGE_BROTLI_BUFFER_BYTES: usize = 16_384;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TerrainPageKey {
    pub level: u8,
    pub coord: [i32; 3],
}

impl TerrainPageKey {
    pub fn bounds(self) -> Option<VoxelBounds> {
        if self.level > TERRAIN_PAGE_MAX_LEVEL {
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
            coord: self.coord.map(|component| component.div_euclid(2)),
        })
    }

    pub fn children(self) -> Option<[Self; 8]> {
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
    pub shape_xz: [u16; 2],
    pub columns: Vec<TerrainColumn>,
    pub runs: Vec<TerrainMaterialRun>,
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
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerrainPageRepresentation {
    SteppedSurfaceResidual(SteppedSurfaceResidual),
    SparseVoxelBrick(SparseVoxelBrickPayload),
    SurfaceCluster(Vec<TerrainSurfaceQuad>),
}

impl TerrainPageRepresentation {
    pub const fn kind(&self) -> TerrainPageRepresentationKind {
        match self {
            Self::SteppedSurfaceResidual(_) => {
                TerrainPageRepresentationKind::SteppedSurfaceResidual
            }
            Self::SparseVoxelBrick(_) => TerrainPageRepresentationKind::SparseVoxelBrick,
            Self::SurfaceCluster(_) => TerrainPageRepresentationKind::SurfaceCluster,
        }
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerrainPageRepresentationKind {
    SteppedSurfaceResidual = 1,
    SparseVoxelBrick = 2,
    SurfaceCluster = 3,
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
        self.key.bounds() == Some(self.bounds)
            && matches!(self.children.len(), 0 | TERRAIN_PAGE_MAX_CHILDREN)
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
    SamplingBoundsOverflow,
    MaterialPaletteOverflow,
    PayloadOverflow,
}

impl fmt::Display for TerrainPageBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotExactLeaf => formatter.write_str("exact page builder requires a level-0 key"),
            Self::InvalidPageKey => formatter.write_str("terrain page key has no valid bounds"),
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
    mut material_at: impl FnMut(VoxelCoord) -> Material,
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
    let (stepped, topology) = build_stepped(bounds, &samples, &palette_indices)?;
    let sparse = build_sparse(bounds, &samples, &palette_indices);
    let clusters = merge_surface_faces(&faces, &palette_indices);
    let representation = if topology == TerrainTopologyClass::SingleRunColumns {
        TerrainPageRepresentation::SteppedSurfaceResidual(stepped)
    } else {
        let sparse_payload =
            TerrainPageRepresentation::SparseVoxelBrick(SparseVoxelBrickPayload {
                brick_edge: SPARSE_BRICK_EDGE,
                bricks: sparse,
            });
        let cluster_payload = TerrainPageRepresentation::SurfaceCluster(clusters);
        if representation_bytes(&cluster_payload).len() <= representation_bytes(&sparse_payload).len()
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
        let material = Material::from_id(id).ok_or(TerrainPageBuildError::MaterialPaletteOverflow)?;
        let index = u8::try_from(index).map_err(|_| TerrainPageBuildError::MaterialPaletteOverflow)?;
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
    let mut columns = Vec::with_capacity(
        TERRAIN_PAGE_EDGE_SAMPLES as usize * TERRAIN_PAGE_EDGE_SAMPLES as usize,
    );
    let mut runs = Vec::new();
    let mut topology = TerrainTopologyClass::SingleRunColumns;
    for z in bounds.min.z..bounds.max.z {
        for x in bounds.min.x..bounds.max.x {
            let first_run = u32::try_from(runs.len())
                .map_err(|_| TerrainPageBuildError::PayloadOverflow)?;
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
            shape_xz: [
                TERRAIN_PAGE_EDGE_SAMPLES as u16,
                TERRAIN_PAGE_EDGE_SAMPLES as u16,
            ],
            columns,
            runs,
        },
        topology,
    ))
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
                                bounds.min.x
                                    + i32::from(brick_x * SPARSE_BRICK_EDGE + local_x),
                                bounds.min.y
                                    + i32::from(brick_y * SPARSE_BRICK_EDGE + local_y),
                                bounds.min.z
                                    + i32::from(brick_z * SPARSE_BRICK_EDGE + local_z),
                            );
                            let material = samples.sample(coord);
                            let Some(material_index) =
                                palette_indices.get(&material.id()).copied()
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
            push_u16(&mut bytes, surface.shape_xz[0]);
            push_u16(&mut bytes, surface.shape_xz[1]);
            push_u32(&mut bytes, surface.columns.len() as u32);
            push_u32(&mut bytes, surface.runs.len() as u32);
            for column in &surface.columns {
                push_u32(&mut bytes, column.first_run);
                push_u16(&mut bytes, column.run_count);
            }
            for run in &surface.runs {
                push_i32(&mut bytes, run.minimum_y);
                push_u16(&mut bytes, run.length);
                bytes.push(run.material_index);
            }
        }
        TerrainPageRepresentation::SparseVoxelBrick(payload) => {
            bytes.push(payload.brick_edge);
            push_u32(&mut bytes, payload.bricks.len() as u32);
            for brick in &payload.bricks {
                bytes.extend_from_slice(&brick.local_brick);
                for occupancy in brick.occupancy {
                    bytes.extend_from_slice(&occupancy.to_le_bytes());
                }
                push_u16(&mut bytes, brick.material_indices.len() as u16);
                bytes.extend_from_slice(&brick.material_indices);
            }
        }
        TerrainPageRepresentation::SurfaceCluster(quads) => {
            push_u32(&mut bytes, quads.len() as u32);
            for quad in quads {
                bytes.push(quad.axis as u8);
                bytes.push(u8::from(quad.positive));
                bytes.push(quad.material_index);
                push_i32(&mut bytes, quad.plane);
                push_i32(&mut bytes, quad.u);
                push_i32(&mut bytes, quad.v);
                push_u16(&mut bytes, quad.width);
                push_u16(&mut bytes, quad.height);
            }
        }
    }
    bytes
}

fn children_are_complete(page: &TerrainPageV1) -> bool {
    if page.children.is_empty() {
        return true;
    }
    let Some(expected) = page.key.children() else {
        return false;
    };
    let actual = page
        .children
        .iter()
        .map(|child| child.key)
        .collect::<BTreeSet<_>>();
    actual == BTreeSet::from(expected)
}

fn representation_is_valid(page: &TerrainPageV1) -> bool {
    let palette_len = page.materials.len();
    match &page.representation {
        TerrainPageRepresentation::SteppedSurfaceResidual(surface) => {
            page.topology == TerrainTopologyClass::SingleRunColumns
                && surface.shape_xz
                    == [
                        TERRAIN_PAGE_EDGE_SAMPLES as u16,
                        TERRAIN_PAGE_EDGE_SAMPLES as u16,
                    ]
                && surface.columns.len()
                    == TERRAIN_PAGE_EDGE_SAMPLES as usize
                        * TERRAIN_PAGE_EDGE_SAMPLES as usize
                && surface.columns.iter().all(|column| {
                    let start = column.first_run as usize;
                    let end = start.saturating_add(usize::from(column.run_count));
                    end <= surface.runs.len()
                        && surface.runs[start..end].windows(2).all(|runs| {
                            runs[0].minimum_y + i32::from(runs[0].length)
                                <= runs[1].minimum_y
                        })
                })
                && surface.runs.iter().all(|run| {
                    run.length > 0
                        && usize::from(run.material_index) < palette_len
                        && run.minimum_y >= page.bounds.min.y
                        && run.minimum_y + i32::from(run.length) <= page.bounds.max.y
                })
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
        }
        TerrainPageRepresentation::SurfaceCluster(quads) => quads.iter().all(|quad| {
            quad.width > 0
                && quad.height > 0
                && usize::from(quad.material_index) < palette_len
                && quad_inside_bounds(*quad, page.bounds)
        }),
    }
}

fn quad_inside_bounds(quad: TerrainSurfaceQuad, bounds: VoxelBounds) -> bool {
    let width = i32::from(quad.width);
    let height = i32::from(quad.height);
    match quad.axis {
        FaceAxis::X => {
            (bounds.min.x..=bounds.max.x).contains(&quad.plane)
                && quad.u >= bounds.min.y
                && quad.v >= bounds.min.z
                && quad.u.saturating_add(width) <= bounds.max.y
                && quad.v.saturating_add(height) <= bounds.max.z
        }
        FaceAxis::Y => {
            (bounds.min.y..=bounds.max.y).contains(&quad.plane)
                && quad.u >= bounds.min.x
                && quad.v >= bounds.min.z
                && quad.u.saturating_add(width) <= bounds.max.x
                && quad.v.saturating_add(height) <= bounds.max.z
        }
        FaceAxis::Z => {
            (bounds.min.z..=bounds.max.z).contains(&quad.plane)
                && quad.u >= bounds.min.x
                && quad.v >= bounds.min.y
                && quad.u.saturating_add(width) <= bounds.max.x
                && quad.v.saturating_add(height) <= bounds.max.y
        }
    }
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
    if !matches!(child_count, 0 | TERRAIN_PAGE_MAX_CHILDREN) {
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
        let material =
            Material::from_id(id).ok_or(TerrainPageCodecError::UnknownMaterial(id))?;
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
        || page.key.bounds() != Some(page.bounds)
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
            let shape_xz = [cursor.u16()?, cursor.u16()?];
            let column_count = cursor.u32()? as usize;
            let run_count = cursor.u32()? as usize;
            let expected_columns =
                TERRAIN_PAGE_EDGE_SAMPLES as usize * TERRAIN_PAGE_EDGE_SAMPLES as usize;
            if column_count != expected_columns || run_count > expected_columns * 32 {
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
            TerrainPageRepresentation::SteppedSurfaceResidual(SteppedSurfaceResidual {
                shape_xz,
                columns,
                runs,
            })
        }
        TerrainPageRepresentationKind::SparseVoxelBrick => {
            let brick_edge = cursor.u8()?;
            let brick_count = cursor.u32()? as usize;
            if brick_edge != SPARSE_BRICK_EDGE || brick_count > 64 {
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
            TerrainPageRepresentation::SparseVoxelBrick(SparseVoxelBrickPayload {
                brick_edge,
                bricks,
            })
        }
        TerrainPageRepresentationKind::SurfaceCluster => {
            let quad_count = cursor.u32()? as usize;
            if quad_count > TERRAIN_PAGE_MAX_PAYLOAD_BYTES / 19 {
                return Err(TerrainPageCodecError::InvalidRepresentation(
                    "surface cluster count",
                ));
            }
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
            TerrainPageRepresentation::SurfaceCluster(quads)
        }
    };
    if cursor.position != payload.len() {
        return Err(TerrainPageCodecError::InvalidRepresentation(
            "trailing payload bytes",
        ));
    }
    Ok(representation)
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
        self.bytes(1)
            .and_then(|bytes| bytes.first().copied().ok_or(TerrainPageCodecError::Truncated))
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

    fn identity() -> WorldSourceIdentityHash {
        WorldSourceIdentityHash::from_bytes([0x5a; 32])
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
                        coord.y >= run.minimum_y
                            && coord.y < run.minimum_y + i32::from(run.length)
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
                let index =
                    usize::from(local[0]) + usize::from(local[1]) * edge + usize::from(local[2]) * edge * edge;
                if brick.occupancy[index / 64] & (1u64 << (index % 64)) == 0 {
                    return Material::Air;
                }
                let rank = brick.occupancy[..index / 64]
                    .iter()
                    .map(|word| word.count_ones() as usize)
                    .sum::<usize>()
                    + (brick.occupancy[index / 64] & ((1u64 << (index % 64)) - 1))
                        .count_ones() as usize;
                palette[usize::from(brick.material_indices[rank])].material
            }
            TerrainPageRepresentation::SurfaceCluster(_) => Material::Air,
        }
    }

    #[test]
    fn signed_page_keys_have_exact_nested_half_open_bounds() {
        let key = TerrainPageKey {
            level: 0,
            coord: [-2, -1, 3],
        };
        assert_eq!(
            key.bounds().unwrap(),
            VoxelBounds::new(VoxelCoord::new(-64, -32, 96), VoxelCoord::new(-32, 0, 128))
                .unwrap()
        );
        let parent = key.parent().unwrap();
        assert_eq!(parent.coord, [-1, -1, 1]);
        assert!(parent.children().unwrap().contains(&key));
    }

    #[test]
    fn exact_heightfield_leaf_round_trips_every_voxel_and_is_deterministic() {
        let key = TerrainPageKey {
            level: 0,
            coord: [-1, -1, -1],
        };
        let first = build_exact_terrain_page(identity(), key, 7, terrain).unwrap();
        let second = build_exact_terrain_page(identity(), key, 7, terrain).unwrap();
        assert_eq!(first, second);
        assert!(first.validates_identity());
        assert_eq!(first.topology, TerrainTopologyClass::SingleRunColumns);
        assert!(matches!(
            first.representation,
            TerrainPageRepresentation::SteppedSurfaceResidual(_)
        ));
        for z in first.bounds.min.z..first.bounds.max.z {
            for y in first.bounds.min.y..first.bounds.max.y {
                for x in first.bounds.min.x..first.bounds.max.x {
                    let coord = VoxelCoord::new(x, y, z);
                    assert_eq!(material_from_payload(&first, coord), terrain(coord));
                }
            }
        }
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
        let stepped = build_exact_terrain_page(
            identity(),
            TerrainPageKey {
                level: 0,
                coord: [-1, -1, -1],
            },
            19,
            terrain,
        )
        .unwrap();
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
            decode_terrain_page(
                &encoded,
                WorldSourceIdentityHash::from_bytes([0x33; 32])
            ),
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
}
