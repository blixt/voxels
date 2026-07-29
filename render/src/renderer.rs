use crate::ambient_occlusion::AmbientOcclusionGpu;
use crate::arena::{Allocation, ArenaAllocator};
use crate::avatar::AvatarGpu;
pub use crate::clouds::VolumetricCloudConfig;
use crate::clouds::VolumetricCloudGpu;
use crate::environment::{
    DaylightPhase, DebugEnvironmentOverride, InteriorEnvironment, OutdoorEnvironment,
    WorldEnvironmentState, surface_region_label,
};
use crate::material_detail::MaterialDetailGpu;
use crate::shadow::{
    AabbClipClassification, AabbClipVolume, CASCADE_COUNT, DirectionalShadowBasis,
    DirectionalShadowCascades, DirectionalShadowConfig, ShadowDirectionTracker,
    build_directional_shadow_cascades,
};
use crate::ui::{Color, InventoryItem, LiveStats, MissionControlUi, UiAction, UiKey, Viewport};
pub use crate::ui::{MissionControlConfig, RendererFeatureConfig};
use crate::ui_gpu::{SCENE_FORMAT, UiGpu};
use crate::virtual_terrain::{
    ExactSurfaceDomain, PresentationEnvelope, PresentationLocus, VirtualTerrainCapacity,
    VirtualTerrainCut, VirtualTerrainError, VirtualTerrainHierarchy, VirtualTerrainView,
};
use crate::virtual_terrain_gpu::{
    GpuVirtualTerrainFeedback, VIRTUAL_TERRAIN_SURFACE_HANDLE_SOURCE_BYTES,
    VIRTUAL_TERRAIN_SURFACE_INDIRECT_OFFSET, VIRTUAL_TERRAIN_TRIANGLE_HANDLE_SOURCE_BYTES,
    VIRTUAL_TERRAIN_TRIANGLE_INDIRECT_OFFSET, VIRTUAL_TERRAIN_WATER_SURFACE_HANDLE_SOURCE_BYTES,
    VIRTUAL_TERRAIN_WATER_SURFACE_INDIRECT_OFFSET,
    VIRTUAL_TERRAIN_WATER_TRIANGLE_HANDLE_SOURCE_BYTES,
    VIRTUAL_TERRAIN_WATER_TRIANGLE_INDIRECT_OFFSET, VirtualTerrainCandidateEncodeOutcome,
    VirtualTerrainCandidateWork, VirtualTerrainGpuControl, VirtualTerrainGpuGeometry,
    VirtualTerrainGpuGeometryRange, VirtualTerrainGpuTimestampWrites,
    VirtualTerrainSnapshotIdentity,
};
use bytemuck::{Pod, Zeroable};
use hashbrown::{HashMap, HashSet};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use voxels_core::{CameraState, EnclosureSample, FluidState, RemoteAvatarPose};
use voxels_world::protocol::{EditShape, EditVolume};
use voxels_world::{
    AtmosphereSample, CHUNK_EDGE, CelestialObservation, Chunk, ChunkCoord, FaceAxis, Material,
    MeshedChunk, Quad, RenderLayer, SurfaceRegion, TERRAIN_COVERAGE_ROOT_LEVEL,
    TERRAIN_PAGE_EDGE_SAMPLES, TERRAIN_REGION_ROOT_LEVEL, TerrainHierarchyDirectoryV1,
    TerrainPageKey, TerrainPageRepresentation, TerrainPageRepresentationKind, TerrainPageV1,
    VOXEL_SIZE_METRES, WorldManifest, reconstruct_exact_terrain_surface,
};
use wgpu::util::DeviceExt;
use wgpu::{
    Backends, BindGroup, Buffer, CurrentSurfaceTexture, Device, DeviceDescriptor, Features,
    Instance, InstanceDescriptor, PowerPreference, PresentMode, QuerySet, Queue, RenderPipeline,
    RequestAdapterOptions, Surface, SurfaceConfiguration, Texture, TextureFormat, TextureUsages,
    TextureView,
};

const DEPTH_FORMAT: TextureFormat = TextureFormat::Depth32Float;
const MAX_SHADOW_ALLOCATION_BYTES: u64 = 256 * 1024 * 1024;
const MAX_ACTIVE_LOCAL_LIGHTS: usize = 16;
const MAX_LOCAL_LIGHT_VISIBILITY_TESTS: usize = 32;
const _: () = assert!(MAX_LOCAL_LIGHT_VISIBILITY_TESTS >= MAX_ACTIVE_LOCAL_LIGHTS);
const PLACEMENT_MATERIALS: [Material; Material::ALL.len() - 1] = [
    Material::Grass,
    Material::Dirt,
    Material::Stone,
    Material::Sand,
    Material::Snow,
    Material::Clay,
    Material::Basalt,
    Material::Wood,
    Material::Leaves,
    Material::Moss,
    Material::Limestone,
    Material::RedSand,
    Material::Water,
    Material::GlowCrystal,
];
const MATERIAL_WHEEL_SLOTS: usize = 10;
const ARENA_PAGE_BYTES: u32 = 4 * 1024 * 1024;
// Immutable virtual-page geometry is the durable render representation. Two independently
// bindable segments stay below WebGPU's common 128 MiB storage-binding ceiling while reserving
// enough transition headroom to stage a maximum complete child group beside the published cut.
const VIRTUAL_TERRAIN_GPU_POOL_BYTES: u64 = 192 * 1024 * 1024;
const VIRTUAL_TERRAIN_GPU_POOL_PAGES: usize = 2;
const VIRTUAL_TERRAIN_GPU_ARENA_PAGE_BYTES: u32 = 96 * 1024 * 1024;
const GPU_FACE_SHIFT: u32 = 16;
const GPU_FACE_MASK: u32 = 0b111 << GPU_FACE_SHIFT;
const GPU_SOURCE_SHIFT: u32 = 5;
const GPU_SOURCE_MASK: u32 = 0b111 << GPU_SOURCE_SHIFT;
const GPU_SOURCE_FRONTIER: u32 = 1;
const EXACT_VOLUME_FRONTIER_MESH_KEY: MeshKey = (u8::MAX, 2, 0, 0);
pub const EXACT_VOLUME_FRONTIER_FACE_WORDS: usize = CHUNK_EDGE * CHUNK_EDGE / 64;
/// Greedy canonical rectangles are triangulated from their center to unit boundary segments.
/// Matching every possible 10 cm boundary vertex prevents merged faces from leaving T-junctions
/// against differently sized neighbors while retaining perimeter rather than area complexity.
const CANONICAL_TRIANGLE_FLAG: u16 = 1 << 13;
const CANONICAL_TRIANGLE_OFFSET_MASK: u16 = (1 << 6) - 1;
const CANONICAL_TRIANGLE_EXTENT_SHIFT: u16 = 6;
const CANONICAL_TRIANGLE_EDGE_SHIFT: u16 = 11;
const CANONICAL_TRIANGLE_ANCHOR_SHIFT: u16 = 11;
const CANONICAL_TRIANGLE_SHADOW_OWNER_FLAG: u16 = 1 << 14;
const CANONICAL_TRIANGLE_LATTICE_ANCHOR: u16 = 5;
const CANONICAL_TRIANGLE_ANCHOR_U_SHIFT: u32 = 8;
const CANONICAL_TRIANGLE_ANCHOR_V_SHIFT: u32 = 14;
const GPU_QUERY_COUNT: u32 = 28;
const PRECIPITATION_INSTANCE_COUNT: u32 = 48 * 48 * 2;
const QUAD_VERTEX_COUNT: u32 = 4;
const GPU_QUERY_BUFFER_BYTES: u64 = GPU_QUERY_COUNT as u64 * size_of::<u64>() as u64;
const GPU_RESOLVE_BUFFER_BYTES: u64 = 256;
const GPU_READBACK_SLOTS: usize = 4;
const GPU_TIMING_HISTORY_CAPACITY: usize = 512;
const GPU_TIMER_BUFFER_BYTES: u64 =
    GPU_RESOLVE_BUFFER_BYTES + GPU_QUERY_BUFFER_BYTES * GPU_READBACK_SLOTS as u64;
type MeshKey = (u8, i32, i32, i32);

#[derive(Clone, Debug, Eq, PartialEq)]
struct PlacementInventory {
    counts: [u64; Material::ALL.len()],
    selected: Option<Material>,
}

impl PlacementInventory {
    fn new() -> Self {
        Self {
            counts: [0; Material::ALL.len()],
            selected: None,
        }
    }

    const fn selected(&self) -> Option<Material> {
        self.selected
    }

    fn count(&self, material: Material) -> u64 {
        self.counts[usize::from(material.id())]
    }

    fn set_counts(&mut self, counts: [u64; Material::ALL.len()]) {
        self.counts = counts;
        if self
            .selected
            .is_none_or(|material| self.count(material) == 0)
        {
            self.selected = PLACEMENT_MATERIALS
                .into_iter()
                .find(|material| self.count(*material) > 0);
        }
    }

    fn select(&mut self, material: Material) -> bool {
        if !is_placeable_material(material) || self.count(material) == 0 {
            return false;
        }
        self.selected = Some(material);
        true
    }

    fn cycle(&mut self, direction: i32) -> bool {
        if direction == 0 {
            return false;
        }
        let current = self
            .selected
            .and_then(|selected| {
                PLACEMENT_MATERIALS
                    .iter()
                    .position(|material| *material == selected)
            })
            .unwrap_or_else(|| {
                if direction.is_positive() {
                    PLACEMENT_MATERIALS.len() - 1
                } else {
                    0
                }
            });
        let step = direction.signum();
        for distance in 1..=PLACEMENT_MATERIALS.len() {
            let index = (current as i32 + step * distance as i32)
                .rem_euclid(PLACEMENT_MATERIALS.len() as i32) as usize;
            let candidate = PLACEMENT_MATERIALS[index];
            if Some(candidate) != self.selected && self.count(candidate) > 0 {
                return self.select(candidate);
            }
        }
        false
    }

    fn visible_materials(&self) -> Vec<Material> {
        let available = PLACEMENT_MATERIALS
            .into_iter()
            .filter(|material| self.count(*material) > 0)
            .collect::<Vec<_>>();
        if available.len() <= MATERIAL_WHEEL_SLOTS {
            return available;
        }
        let selected = self
            .selected
            .and_then(|selected| available.iter().position(|material| *material == selected))
            .unwrap_or(0);
        let start = (selected + available.len() - MATERIAL_WHEEL_SLOTS / 2) % available.len();
        (0..MATERIAL_WHEEL_SLOTS)
            .map(|offset| available[(start + offset) % available.len()])
            .collect()
    }

    fn select_visible_slot(&mut self, slot: usize) -> bool {
        let Some(material) = self.visible_materials().get(slot).copied() else {
            return false;
        };
        self.select(material)
    }
}

/// Host-provided renderer startup and reset configuration.
///
/// This type deliberately contains no browser or serialization concerns. A shell may deserialize its
/// own file format, validate it, and then construct this portable renderer-domain value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RendererConfig {
    pub features: RendererFeatureConfig,
    pub mission_control: MissionControlConfig,
    pub view_distance_metres: f32,
    pub directional_shadows: DirectionalShadowConfig,
    pub volumetric_clouds: VolumetricCloudConfig,
    pub diagnostic_sky_color: Option<[f32; 3]>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum VirtualTerrainRenderMode {
    #[default]
    Disabled,
    Shadow,
    Visible,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VirtualTerrainPublicationAdvance {
    Idle,
    AwaitCertificate,
    CommitActive,
    PromoteCertified,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct VirtualTerrainPublication {
    cut: VirtualTerrainCut,
    envelope: PresentationEnvelope,
}

const fn virtual_terrain_publication_advance(
    has_transaction: bool,
    active_matches: bool,
    candidate_certified: bool,
) -> VirtualTerrainPublicationAdvance {
    if !has_transaction {
        VirtualTerrainPublicationAdvance::Idle
    } else if active_matches {
        VirtualTerrainPublicationAdvance::CommitActive
    } else if candidate_certified {
        VirtualTerrainPublicationAdvance::PromoteCertified
    } else {
        VirtualTerrainPublicationAdvance::AwaitCertificate
    }
}

const fn virtual_terrain_publication_can_stage(has_transaction: bool) -> bool {
    !has_transaction
}

const fn virtual_terrain_committed_snapshot_is_safe(
    has_committed_cut: bool,
    committed_cut_is_renderable: bool,
    active_bank_matches_committed: bool,
) -> bool {
    has_committed_cut && committed_cut_is_renderable && active_bank_matches_committed
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VirtualTerrainRendererError {
    Hierarchy(VirtualTerrainError),
    UnsupportedRepresentation(TerrainPageRepresentationKind),
    InvalidSurfaceCluster(TerrainPageKey),
    InvalidTriangleCluster(TerrainPageKey),
    GpuPageTooLarge(TerrainPageKey),
    GpuPoolCapacity,
    GpuSnapshot,
    SelectedCutSnapshotCapacity { required_source_bytes: [u64; 4] },
    NoRenderableCut,
    SelectedPageMissingGpu(TerrainPageKey),
    GpuCutNotCertified,
    IncompleteRootPartition(TerrainPageKey),
}

impl std::fmt::Display for VirtualTerrainRendererError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Hierarchy(error) => write!(formatter, "{error}"),
            Self::UnsupportedRepresentation(kind) => {
                write!(
                    formatter,
                    "virtual terrain representation {kind:?} has no GPU path"
                )
            }
            Self::InvalidSurfaceCluster(key) => {
                write!(
                    formatter,
                    "virtual terrain surface cluster {key:?} is invalid"
                )
            }
            Self::InvalidTriangleCluster(key) => {
                write!(
                    formatter,
                    "virtual terrain triangle cluster {key:?} is invalid"
                )
            }
            Self::GpuPageTooLarge(key) => {
                write!(
                    formatter,
                    "virtual terrain GPU page {key:?} exceeds its hard bound"
                )
            }
            Self::GpuPoolCapacity => {
                formatter.write_str("virtual terrain GPU page pool capacity exceeded")
            }
            Self::GpuSnapshot => {
                formatter.write_str("virtual terrain GPU handle snapshot state is inconsistent")
            }
            Self::SelectedCutSnapshotCapacity {
                required_source_bytes,
            } => {
                write!(
                    formatter,
                    "selected virtual terrain cut exceeds handle snapshot coverage: surface/triangle/water-surface/water-triangle source geometry = {:.1}/{:.1}/{:.1}/{:.1} MiB",
                    required_source_bytes[0] as f64 / (1024.0 * 1024.0),
                    required_source_bytes[1] as f64 / (1024.0 * 1024.0),
                    required_source_bytes[2] as f64 / (1024.0 * 1024.0),
                    required_source_bytes[3] as f64 / (1024.0 * 1024.0),
                )
            }
            Self::NoRenderableCut => {
                formatter.write_str("virtual terrain has no complete renderable cut")
            }
            Self::SelectedPageMissingGpu(key) => {
                write!(
                    formatter,
                    "selected virtual terrain page {key:?} has no resident GPU record"
                )
            }
            Self::GpuCutNotCertified => formatter
                .write_str("virtual terrain GPU cut has not been certified for publication"),
            Self::IncompleteRootPartition(key) => {
                write!(
                    formatter,
                    "virtual terrain root {key:?} is not a complete selected partition"
                )
            }
        }
    }
}

impl std::error::Error for VirtualTerrainRendererError {}

impl From<VirtualTerrainError> for VirtualTerrainRendererError {
    fn from(error: VirtualTerrainError) -> Self {
        Self::Hierarchy(error)
    }
}

/// Source/build identity embedded in every screenshot reproduction package.
///
/// This is supplied by the host because the portable renderer deliberately has no knowledge of a
/// browser bundle, repository, or client configuration format.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenshotReproductionIdentity {
    pub build_commit: String,
    pub build_dirty: bool,
    pub build_profile: String,
    pub protocol_version: u16,
    pub client_config_hash: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScreenshotCanonicalPageState {
    pub coord: ChunkCoord,
    pub revision: u64,
    pub phase: u8,
    pub desired: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScreenshotVirtualRegionState {
    pub root: TerrainPageKey,
    pub minimum_revision: u64,
    pub registered: bool,
    pub in_flight: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScreenshotVirtualColumnState {
    pub column: [i32; 2],
    pub resolved_revision: Option<u64>,
    pub minimum_revision: u64,
    pub in_flight: bool,
}

/// Exact host-side residency/request state captured on the frame that owns screenshot readback.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ScreenshotStreamingManifest {
    pub canonical_pages: Vec<ScreenshotCanonicalPageState>,
    pub virtual_columns: Vec<ScreenshotVirtualColumnState>,
    pub virtual_regions: Vec<ScreenshotVirtualRegionState>,
    pub virtual_pending_pages: usize,
    pub virtual_in_flight_pages: usize,
    pub virtual_obsolete_in_flight_pages: usize,
    pub virtual_cancelled_pending_pages: u64,
    pub virtual_useful_bytes: u64,
    pub virtual_cancellation_waste_bytes: u64,
    pub virtual_failed_pages: u64,
    pub virtual_cache_pages: usize,
    pub virtual_cache_bytes: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenshotMutableRenderState {
    pub world_lab_open: bool,
    pub diagnostic_sky_color: Option<[f32; 3]>,
    pub geometry_source_debug: bool,
    pub material_detail: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScreenshotFeatureState {
    pub shadows: bool,
    pub voxel_ambient_occlusion: bool,
    pub screen_space_ambient_occlusion: bool,
    pub fog: bool,
    pub far_terrain: bool,
    pub water: bool,
    pub target_outline: bool,
    pub cave_headlamp: bool,
    pub local_lighting: bool,
}

impl Default for RendererConfig {
    fn default() -> Self {
        Self {
            features: RendererFeatureConfig::default(),
            mission_control: MissionControlConfig::default(),
            view_distance_metres: 3_200.0,
            directional_shadows: DirectionalShadowConfig::default(),
            volumetric_clouds: VolumetricCloudConfig::default(),
            diagnostic_sky_color: None,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FrameUniform {
    view_projection: [[f32; 4]; 4],
    inverse_view_projection: [[f32; 4]; 4],
    camera_time: [f32; 4],
    viewport_voxel: [f32; 4],
    target_voxel: [f32; 4],
    target_voxel_max: [f32; 4],
    render_options: [f32; 4],
    debug_options: [f32; 4],
    camera_forward: [f32; 4],
    shadow_splits: [f32; 4],
    shadow_texel_sizes: [f32; 4],
    shadow_view_projection: [[[f32; 4]; 4]; CASCADE_COUNT],
    key_light_direction: [f32; 4],
    key_light_radiance: [f32; 4],
    sun_direction: [f32; 4],
    moon_direction: [f32; 4],
    equatorial_east: [f32; 4],
    equatorial_up: [f32; 4],
    equatorial_north: [f32; 4],
    environment_time: [f32; 4],
    atmosphere_motion: [f32; 4],
    sky_horizon: [f32; 4],
    sky_zenith: [f32; 4],
    ground_atmosphere: [f32; 4],
    fog_exposure: [f32; 4],
    weather: [f32; 4],
    cloud_layer: [f32; 4],
    medium: [f32; 4],
    interior: [f32; 4],
    diagnostic_sky: [f32; 4],
}

const _: () = assert!(size_of::<FrameUniform>() == 752);
const _: () = assert!(std::mem::offset_of!(FrameUniform, weather) == 672);
const _: () = assert!(std::mem::offset_of!(FrameUniform, cloud_layer) == 688);
const _: () = assert!(std::mem::offset_of!(FrameUniform, medium) == 704);
const _: () = assert!(std::mem::offset_of!(FrameUniform, interior) == 720);
const _: () = assert!(std::mem::offset_of!(FrameUniform, diagnostic_sky) == 736);

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct GpuLocalLight {
    position_radius: [f32; 4],
    color_intensity: [f32; 4],
}

const _: () = assert!(size_of::<GpuLocalLight>() == 32);

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LocalLightUniform {
    metadata: [u32; 4],
    lights: [GpuLocalLight; MAX_ACTIVE_LOCAL_LIGHTS],
}

impl Default for LocalLightUniform {
    fn default() -> Self {
        Self {
            metadata: [0; 4],
            lights: [GpuLocalLight::default(); MAX_ACTIVE_LOCAL_LIGHTS],
        }
    }
}

const _: () = assert!(size_of::<LocalLightUniform>() == 528);
const _: () = assert!(std::mem::offset_of!(LocalLightUniform, lights) == 16);

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ShadowFrameUniform {
    clip_from_world: [[f32; 4]; 4],
    camera_voxel: [f32; 4],
}

const _: () = assert!(size_of::<ShadowFrameUniform>() == 80);

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Pod, Zeroable)]
struct GpuQuad {
    origin: [i32; 3],
    extent_voxels: [u16; 2],
    material_face: u32,
    ao: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
struct GpuTerrainVertex {
    position: [f32; 3],
    material: u32,
    normal: [i16; 4],
}

const _: () = assert!(size_of::<GpuTerrainVertex>() == 24);

/// A visible opening from a resident exact-volume chunk into a chunk whose mesh is not ready.
///
/// Unknown data is not empty space. The renderer temporarily closes only these reachable portal
/// cells until the requested neighbor has an exact mesh, preventing atmospheric background from
/// leaking through a tunnel frontier without inventing coverage at exterior silhouettes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExactVolumeFrontierFace {
    pub chunk: ChunkCoord,
    /// Portal face order: -X, +X, -Y, +Y, -Z, +Z.
    pub face: u8,
    pub cells: [u64; EXACT_VOLUME_FRONTIER_FACE_WORDS],
}

const _: () = assert!(size_of::<GpuQuad>() == 24);
const _: () = assert!(std::mem::offset_of!(GpuQuad, extent_voxels) == 12);
const _: () = assert!(std::mem::offset_of!(GpuQuad, material_face) == 16);
const _: () = assert!(std::mem::offset_of!(GpuQuad, ao) == 20);

fn pack_gpu_material_face(material: u32, face: u8) -> u32 {
    debug_assert_eq!(material & GPU_FACE_MASK, 0);
    debug_assert_eq!(material & GPU_SOURCE_MASK, 0);
    debug_assert!(face <= 5);
    material | (u32::from(face) << GPU_FACE_SHIFT)
}

fn pack_gpu_source_material(material_face: u32, source: u32) -> u32 {
    debug_assert_eq!(material_face & GPU_SOURCE_MASK, 0);
    debug_assert!(source < 1 << 3);
    material_face | (source << GPU_SOURCE_SHIFT)
}

fn pack_virtual_material(material: Material, level: u8) -> u32 {
    u32::from(material.id()) | (u32::from(level.min(7)) << 27) | (1 << 31)
}

fn packed_ao_corner(packed: u8, corner: usize) -> u8 {
    (packed >> (corner * 2)) & 3
}

fn rounded_ao_lerp(start: u8, end: u8, offset: u16, extent: u16) -> u8 {
    debug_assert!(extent > 0);
    let numerator =
        u32::from(start) * u32::from(extent - offset) + u32::from(end) * u32::from(offset);
    ((numerator + u32::from(extent) / 2) / u32::from(extent)) as u8
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QuadEdge {
    NegativeX,
    PositiveX,
    NegativeZ,
    PositiveZ,
}

impl QuadEdge {
    const ALL: [Self; 4] = [
        Self::NegativeX,
        Self::PositiveX,
        Self::NegativeZ,
        Self::PositiveZ,
    ];

    const fn index(self) -> usize {
        match self {
            Self::NegativeX => 0,
            Self::PositiveX => 1,
            Self::NegativeZ => 2,
            Self::PositiveZ => 3,
        }
    }
}

fn canonical_triangle_ao(
    packed: u8,
    edge: QuadEdge,
    bounds: [u16; 2],
    extent: [u16; 2],
    anchor: Option<usize>,
) -> u32 {
    let corners = std::array::from_fn::<_, 4, _>(|corner| packed_ao_corner(packed, corner));
    let flip = u16::from(corners[0]) + u16::from(corners[2])
        > u16::from(corners[1]) + u16::from(corners[3]);
    let anchor_ao = if let Some(anchor) = anchor {
        u16::from(corners[anchor])
    } else if flip {
        (u16::from(corners[1]) + u16::from(corners[3])).div_ceil(2)
    } else {
        (u16::from(corners[0]) + u16::from(corners[2])).div_ceil(2)
    } as u8;
    let (edge_corners, edge_extent) = match edge {
        QuadEdge::NegativeX => ([corners[0], corners[3]], extent[1]),
        QuadEdge::PositiveX => ([corners[1], corners[2]], extent[1]),
        QuadEdge::NegativeZ => ([corners[0], corners[1]], extent[0]),
        QuadEdge::PositiveZ => ([corners[3], corners[2]], extent[0]),
    };
    let mut edge_ao = [
        rounded_ao_lerp(edge_corners[0], edge_corners[1], bounds[0], edge_extent),
        rounded_ao_lerp(edge_corners[0], edge_corners[1], bounds[1], edge_extent),
    ];
    if matches!(edge, QuadEdge::NegativeX | QuadEdge::PositiveZ) {
        edge_ao.swap(0, 1);
    }
    u32::from(anchor_ao)
        | (u32::from(edge_ao[0]) << 2)
        | (u32::from(edge_ao[1]) << 4)
        | (u32::from(edge_ao[1]) << 6)
}

fn canonical_gpu_quad(world_origin: [i32; 3], quad: &Quad) -> GpuQuad {
    let extent = quad.extent.map(u16::from);
    GpuQuad {
        origin: [
            world_origin[0] + i32::from(quad.origin[0]),
            world_origin[1] + i32::from(quad.origin[1]),
            world_origin[2] + i32::from(quad.origin[2]),
        ],
        extent_voxels: extent,
        material_face: pack_gpu_material_face(u32::from(quad.material), quad.face),
        ao: u32::from(quad.ao),
    }
}

fn gpu_quad_bounds(quads: &[GpuQuad]) -> Option<(glam::Vec3, glam::Vec3)> {
    let first = *quads.first()?;
    let mut minimum = glam::IVec3::from_array(canonical_quad_corners(first)[0]);
    let mut maximum = minimum;
    for quad in quads {
        for corner in canonical_quad_corners(*quad) {
            let corner = glam::IVec3::from_array(corner);
            minimum = minimum.min(corner);
            maximum = maximum.max(corner);
        }
    }
    Some((
        minimum.as_vec3() * VOXEL_SIZE_METRES,
        maximum.as_vec3() * VOXEL_SIZE_METRES,
    ))
}

fn virtual_surface_gpu_quads(
    page: &TerrainPageV1,
) -> Result<Vec<GpuQuad>, VirtualTerrainRendererError> {
    let reconstructed;
    let quads = match &page.representation {
        TerrainPageRepresentation::SurfaceCluster(quads) => quads,
        TerrainPageRepresentation::SteppedSurfaceResidual(_)
        | TerrainPageRepresentation::SparseVoxelBrick(_) => {
            reconstructed = reconstruct_exact_terrain_surface(page)
                .map_err(|_| VirtualTerrainRendererError::InvalidSurfaceCluster(page.key))?;
            &reconstructed
        }
        TerrainPageRepresentation::TriangleCluster(_) => {
            return Err(VirtualTerrainRendererError::UnsupportedRepresentation(
                page.representation.kind(),
            ));
        }
        TerrainPageRepresentation::HeightfieldGrid(_) => {
            return Err(VirtualTerrainRendererError::UnsupportedRepresentation(
                page.representation.kind(),
            ));
        }
    };
    let mut gpu_quads = Vec::with_capacity(quads.len());
    for quad in quads {
        let material = page
            .materials
            .get(usize::from(quad.material_index))
            .ok_or(VirtualTerrainRendererError::InvalidSurfaceCluster(page.key))?
            .material;
        if quad.width == 0 || quad.height == 0 {
            return Err(VirtualTerrainRendererError::InvalidSurfaceCluster(page.key));
        }
        let normal_origin = if quad.positive {
            quad.plane
                .checked_sub(1)
                .ok_or(VirtualTerrainRendererError::InvalidSurfaceCluster(page.key))?
        } else {
            quad.plane
        };
        let (origin, extent_voxels, face) = match quad.axis {
            // The existing quad shader's X faces use GPU-u for world Z and GPU-v for world Y.
            FaceAxis::X => (
                [normal_origin, quad.u, quad.v],
                [quad.height, quad.width],
                if quad.positive { 0 } else { 1 },
            ),
            FaceAxis::Y => (
                [quad.u, normal_origin, quad.v],
                [quad.width, quad.height],
                if quad.positive { 2 } else { 3 },
            ),
            FaceAxis::Z => (
                [quad.u, quad.v, normal_origin],
                [quad.width, quad.height],
                if quad.positive { 4 } else { 5 },
            ),
        };
        gpu_quads.push(GpuQuad {
            origin,
            extent_voxels,
            material_face: pack_gpu_material_face(
                pack_virtual_material(material, page.key.level),
                face,
            ),
            // Page clusters do not currently carry per-corner occluders. Encode fully open
            // corners instead of zero, which in the canonical AO convention means maximally
            // occluded.
            ao: u32::from(u8::MAX),
        });
    }
    Ok(gpu_quads)
}

#[derive(Clone, Debug)]
struct VirtualHeightfieldSamples {
    ground: Vec<f32>,
    water: Vec<Option<f32>>,
    exact_neighbor_sides: [bool; 4],
    finer_neighbor_sides: [bool; 4],
}

#[derive(Clone, Debug)]
struct CachedVirtualHeightfieldSamples {
    revision: u64,
    content_fingerprint: [u8; 32],
    ancestor_fingerprint: u64,
    samples: VirtualHeightfieldSamples,
}

fn virtual_triangle_gpu_vertices(
    page: &TerrainPageV1,
    constrained_heightfield: Option<&VirtualHeightfieldSamples>,
) -> Result<Vec<GpuTerrainVertex>, VirtualTerrainRendererError> {
    match &page.representation {
        TerrainPageRepresentation::TriangleCluster(cluster) => {
            let vertex_count = cluster
                .triangles
                .len()
                .checked_mul(3)
                .ok_or(VirtualTerrainRendererError::GpuPageTooLarge(page.key))?;
            let mut vertices = Vec::with_capacity(vertex_count);
            for triangle in &cluster.triangles {
                let material = page
                    .materials
                    .get(usize::from(triangle.material_index))
                    .ok_or(VirtualTerrainRendererError::InvalidTriangleCluster(
                        page.key,
                    ))?
                    .material;
                let source_vertices = triangle
                    .vertices
                    .map(|index| cluster.vertices.get(index as usize));
                let [Some(left), Some(middle), Some(right)] = source_vertices else {
                    return Err(VirtualTerrainRendererError::InvalidTriangleCluster(
                        page.key,
                    ));
                };
                if [left, middle, right]
                    .iter()
                    .any(|vertex| vertex.material_index != triangle.material_index)
                {
                    return Err(VirtualTerrainRendererError::InvalidTriangleCluster(
                        page.key,
                    ));
                }
                push_virtual_triangle_i32(
                    &mut vertices,
                    [left.position, middle.position, right.position],
                    material,
                    page.key,
                )?;
            }
            Ok(vertices)
        }
        TerrainPageRepresentation::HeightfieldGrid(grid) => {
            let edge = TERRAIN_PAGE_EDGE_SAMPLES as usize + 1;
            let owned_heightfield;
            let heightfield = if let Some(heightfield) = constrained_heightfield {
                heightfield
            } else {
                owned_heightfield = unconstrained_virtual_heightfield_samples(grid);
                &owned_heightfield
            };
            if heightfield.ground.len() != edge * edge || heightfield.water.len() != edge * edge {
                return Err(VirtualTerrainRendererError::InvalidTriangleCluster(
                    page.key,
                ));
            }
            let cell_count =
                TERRAIN_PAGE_EDGE_SAMPLES as usize * TERRAIN_PAGE_EDGE_SAMPLES as usize;
            let mut vertices = Vec::with_capacity(cell_count.saturating_mul(12));
            let water = page
                .materials
                .iter()
                .find(|coverage| coverage.material == Material::Water)
                .map(|coverage| coverage.material);
            if page.key.level == 0 {
                push_virtual_microvoxel_ground(&mut vertices, page, grid, heightfield)?;
                if let Some(water) = water {
                    push_virtual_heightfield_water(&mut vertices, page, grid, heightfield, water)?;
                }
                return Ok(vertices);
            }
            for z in 0..TERRAIN_PAGE_EDGE_SAMPLES as usize {
                for x in 0..TERRAIN_PAGE_EDGE_SAMPLES as usize {
                    let cell = x + z * TERRAIN_PAGE_EDGE_SAMPLES as usize;
                    let material = page
                        .materials
                        .get(usize::from(grid.cell_material_indices[cell]))
                        .ok_or(VirtualTerrainRendererError::InvalidTriangleCluster(
                            page.key,
                        ))?
                        .material;
                    let sample_indices = [
                        x + z * edge,
                        x + 1 + z * edge,
                        x + 1 + (z + 1) * edge,
                        x + (z + 1) * edge,
                    ];
                    let stride = i32::try_from(grid.sample_stride_voxels).map_err(|_| {
                        VirtualTerrainRendererError::InvalidTriangleCluster(page.key)
                    })?;
                    let positions = [
                        [
                            (page.bounds.min.x + x as i32 * stride) as f32,
                            heightfield.ground[sample_indices[0]],
                            (page.bounds.min.z + z as i32 * stride) as f32,
                        ],
                        [
                            (page.bounds.min.x + (x as i32 + 1) * stride) as f32,
                            heightfield.ground[sample_indices[1]],
                            (page.bounds.min.z + z as i32 * stride) as f32,
                        ],
                        [
                            (page.bounds.min.x + (x as i32 + 1) * stride) as f32,
                            heightfield.ground[sample_indices[2]],
                            (page.bounds.min.z + (z as i32 + 1) * stride) as f32,
                        ],
                        [
                            (page.bounds.min.x + x as i32 * stride) as f32,
                            heightfield.ground[sample_indices[3]],
                            (page.bounds.min.z + (z as i32 + 1) * stride) as f32,
                        ],
                    ];
                    let [negative_x, positive_x, negative_z, positive_z] =
                        heightfield.finer_neighbor_sides;
                    let refined_sides = [
                        z == 0 && negative_z,
                        x + 1 == TERRAIN_PAGE_EDGE_SAMPLES as usize && positive_x,
                        z + 1 == TERRAIN_PAGE_EDGE_SAMPLES as usize && positive_z,
                        x == 0 && negative_x,
                    ];
                    if refined_sides.into_iter().any(|refined| refined) {
                        push_virtual_heightfield_boundary_cell(
                            &mut vertices,
                            positions,
                            refined_sides,
                            page.key.level == 1,
                            material,
                            page.key,
                        )?;
                        continue;
                    }
                    for triangle in [[0, 2, 1], [0, 3, 2]] {
                        push_virtual_triangle(
                            &mut vertices,
                            triangle.map(|index| positions[index]),
                            material,
                            page.key,
                        )?;
                    }
                }
            }
            if let Some(water) = water {
                push_virtual_heightfield_water(&mut vertices, page, grid, heightfield, water)?;
            }
            Ok(vertices)
        }
        _ => Err(VirtualTerrainRendererError::UnsupportedRepresentation(
            page.representation.kind(),
        )),
    }
}

/// Emits one non-overlapping coarse boundary cell with the midpoint required by the next-finer
/// heightfield.
///
/// Recursive edge constraints make that midpoint identical on both owners. At the L1/L0 boundary,
/// the two half-edges instead follow the exact lower-coordinate voxel-height convention. The cell
/// fans once to its ordinary coarse center, so this is a conforming triangulation rather than an
/// overlapping seam cover.
fn push_virtual_heightfield_boundary_cell(
    vertices: &mut Vec<GpuTerrainVertex>,
    positions: [[f32; 3]; 4],
    refined: [bool; 4],
    exact_staircase: bool,
    material: Material,
    key: TerrainPageKey,
) -> Result<(), VirtualTerrainRendererError> {
    let sides = [
        [positions[0], positions[1]],
        [positions[1], positions[2]],
        [positions[2], positions[3]],
        [positions[3], positions[0]],
    ];
    let mut segments = Vec::with_capacity(8);
    for (side, [start, end]) in sides.into_iter().enumerate() {
        if !refined[side] {
            segments.push([start, end]);
            continue;
        }
        for offset in 0..2 {
            let fraction = offset as f32 * 0.5;
            let next_fraction = (offset + 1) as f32 * 0.5;
            let horizontal_axis = if start[0] != end[0] { 0 } else { 2 };
            // The exact L0 surface assigns each 10 cm top cell the sample at its
            // lower X/Z coordinate. Two sides of the clockwise coarse polygon run
            // in the opposite direction, so using traversal order here shifts the
            // staircase by one voxel on positive-Z and negative-X boundaries.
            let canonical_fraction = if start[horizontal_axis] < end[horizontal_axis] {
                fraction
            } else {
                next_fraction
            };
            let height_at = |height_fraction: f32| {
                let height = start[1] + (end[1] - start[1]) * height_fraction;
                if exact_staircase {
                    height.round()
                } else {
                    height
                }
            };
            let interpolate_position = |amount: f32, height: f32| {
                [
                    start[0] + (end[0] - start[0]) * amount,
                    height,
                    start[2] + (end[2] - start[2]) * amount,
                ]
            };
            let start_height = height_at(if exact_staircase {
                canonical_fraction
            } else {
                fraction
            });
            let end_height = height_at(if exact_staircase {
                canonical_fraction
            } else {
                next_fraction
            });
            segments.push([
                interpolate_position(fraction, start_height),
                interpolate_position(next_fraction, end_height),
            ]);
        }
    }
    let center = std::array::from_fn(|axis| {
        positions.iter().map(|position| position[axis]).sum::<f32>() * 0.25
    });
    let Some(first) = segments.first().copied() else {
        return Err(VirtualTerrainRendererError::InvalidTriangleCluster(key));
    };
    let mut previous = first[0];
    for [start, end] in &segments {
        if previous != *start {
            push_virtual_triangle(vertices, [previous, center, *start], material, key)?;
        }
        push_virtual_triangle(vertices, [*start, center, *end], material, key)?;
        previous = *end;
    }
    if previous != first[0] {
        push_virtual_triangle(vertices, [previous, center, first[0]], material, key)?;
    }
    Ok(())
}

fn push_virtual_heightfield_water(
    vertices: &mut Vec<GpuTerrainVertex>,
    page: &TerrainPageV1,
    grid: &voxels_world::TerrainHeightfieldGrid,
    heightfield: &VirtualHeightfieldSamples,
    material: Material,
) -> Result<(), VirtualTerrainRendererError> {
    let cells = TERRAIN_PAGE_EDGE_SAMPLES as usize;
    let edge = cells + 1;
    let stride = i32::try_from(grid.sample_stride_voxels)
        .map_err(|_| VirtualTerrainRendererError::InvalidTriangleCluster(page.key))?;
    for z in 0..cells {
        for x in 0..cells {
            let sample_indices = [
                x + z * edge,
                x + 1 + z * edge,
                x + 1 + (z + 1) * edge,
                x + (z + 1) * edge,
            ];
            let positions = [
                [
                    (page.bounds.min.x + x as i32 * stride) as f32,
                    0.0,
                    (page.bounds.min.z + z as i32 * stride) as f32,
                ],
                [
                    (page.bounds.min.x + (x as i32 + 1) * stride) as f32,
                    0.0,
                    (page.bounds.min.z + z as i32 * stride) as f32,
                ],
                [
                    (page.bounds.min.x + (x as i32 + 1) * stride) as f32,
                    0.0,
                    (page.bounds.min.z + (z as i32 + 1) * stride) as f32,
                ],
                [
                    (page.bounds.min.x + x as i32 * stride) as f32,
                    0.0,
                    (page.bounds.min.z + (z as i32 + 1) * stride) as f32,
                ],
            ];
            let water_positions = std::array::from_fn::<_, 4, _>(|index| {
                let mut position = positions[index];
                heightfield.water[sample_indices[index]].map(|height| {
                    position[1] = height;
                    position
                })
            });
            let flat_height = water_positions
                .iter()
                .copied()
                .reduce(|left, right| {
                    left.zip(right)
                        .filter(|(left, right)| left[1] == right[1])
                        .map(|(left, _)| left)
                })
                .flatten();
            if flat_height.is_some() {
                continue;
            }
            for triangle in [[0, 2, 1], [0, 3, 2]] {
                let [Some(left), Some(middle), Some(right)] =
                    triangle.map(|index| water_positions[index])
                else {
                    continue;
                };
                push_virtual_triangle(vertices, [left, middle, right], material, page.key)?;
            }
        }
    }
    push_virtual_flat_water_rectangles(vertices, page, grid, heightfield, material)
}

fn push_bounded_virtual_quad(quads: &mut Vec<GpuQuad>, quad: GpuQuad) {
    let [width, height] = quad.extent_voxels;
    for v in (0..height).step_by(63) {
        for u in (0..width).step_by(63) {
            quads.push(canonical_gpu_subrectangle(
                quad,
                i32::from(u),
                i32::from(v),
                [(width - u).min(63), (height - v).min(63)],
            ));
        }
    }
}

/// Encodes the exact level-0 heightfield as compact axis-aligned voxel-face instances.
///
/// The earlier unindexed triangle stream stored three 24-byte vertices for every conforming
/// triangle. These instances preserve the same unit perimeter segments in 24 bytes per triangle,
/// keeping a complete replacement cut resident beside the published cut without eviction churn.
fn virtual_microvoxel_gpu_quads(
    page: &TerrainPageV1,
    grid: &voxels_world::TerrainHeightfieldGrid,
    heightfield: &VirtualHeightfieldSamples,
) -> Result<Option<Vec<GpuQuad>>, VirtualTerrainRendererError> {
    if page.key.level != 0 || grid.sample_stride_voxels != 1 {
        return Err(VirtualTerrainRendererError::InvalidTriangleCluster(
            page.key,
        ));
    }
    let cells = TERRAIN_PAGE_EDGE_SAMPLES as usize;
    let edge = cells + 1;
    if heightfield.ground.len() != edge * edge || heightfield.water.len() != edge * edge {
        return Err(VirtualTerrainRendererError::InvalidTriangleCluster(
            page.key,
        ));
    }
    let is_lattice_height = |height: f32| {
        let integer = height as i32;
        height.is_finite() && integer as f32 == height
    };
    let lattice_height = |height: f32| {
        let integer = height as i32;
        is_lattice_height(height).then_some(integer).ok_or(
            VirtualTerrainRendererError::InvalidTriangleCluster(page.key),
        )
    };
    let ground_at = |x: usize, z: usize| {
        let height = heightfield.ground[x + z * edge];
        height.is_finite().then_some(height.round() as i32).ok_or(
            VirtualTerrainRendererError::InvalidTriangleCluster(page.key),
        )
    };
    let material_at = |x: usize, z: usize| {
        grid.sample_material_indices
            .get(x + z * edge)
            .and_then(|index| page.materials.get(usize::from(*index)))
            .map(|coverage| coverage.material)
            .ok_or(VirtualTerrainRendererError::InvalidTriangleCluster(
                page.key,
            ))
    };
    let coordinate = |origin: i32, offset: usize| {
        i32::try_from(offset)
            .ok()
            .and_then(|offset| origin.checked_add(offset))
            .ok_or(VirtualTerrainRendererError::InvalidTriangleCluster(
                page.key,
            ))
    };
    let make_quad =
        |origin: [i32; 3], extent_voxels: [u16; 2], face: u8, material: Material| GpuQuad {
            origin,
            extent_voxels,
            material_face: pack_gpu_material_face(
                pack_virtual_material(material, page.key.level),
                face,
            ),
            ao: u32::from(u8::MAX),
        };

    let mut base = Vec::new();
    let mut emitted = vec![false; cells * cells];
    for z in 0..cells {
        for x in 0..cells {
            let cell = x + z * cells;
            if emitted[cell] {
                continue;
            }
            let height = ground_at(x, z)?;
            let material = material_at(x, z)?;
            let matches = |candidate_x: usize, candidate_z: usize| {
                let candidate = candidate_x + candidate_z * cells;
                !emitted[candidate]
                    && ground_at(candidate_x, candidate_z).is_ok_and(|value| value == height)
                    && material_at(candidate_x, candidate_z)
                        .is_ok_and(|candidate| candidate == material)
            };
            let mut width = 1usize;
            while x + width < cells && matches(x + width, z) {
                width += 1;
            }
            let mut depth = 1usize;
            'grow: while z + depth < cells {
                for offset in 0..width {
                    if !matches(x + offset, z + depth) {
                        break 'grow;
                    }
                }
                depth += 1;
            }
            for row in z..z + depth {
                emitted[row * cells + x..row * cells + x + width].fill(true);
            }
            push_bounded_virtual_quad(
                &mut base,
                make_quad(
                    [
                        coordinate(page.bounds.min.x, x)?,
                        height.saturating_sub(1),
                        coordinate(page.bounds.min.z, z)?,
                    ],
                    [
                        u16::try_from(width).map_err(|_| {
                            VirtualTerrainRendererError::InvalidTriangleCluster(page.key)
                        })?,
                        u16::try_from(depth).map_err(|_| {
                            VirtualTerrainRendererError::InvalidTriangleCluster(page.key)
                        })?,
                    ],
                    2,
                    material,
                ),
            );
        }
    }

    for x in 0..cells {
        let mut z = 0usize;
        while z < cells {
            let left = ground_at(x, z)?;
            let right = ground_at(x + 1, z)?;
            if left == right {
                z += 1;
                continue;
            }
            let positive = left > right;
            let lower = left.min(right);
            let upper = left.max(right);
            let material = material_at(if positive { x } else { x + 1 }, z)?;
            let mut depth = 1usize;
            while z + depth < cells {
                let next_left = ground_at(x, z + depth)?;
                let next_right = ground_at(x + 1, z + depth)?;
                let next_positive = next_left > next_right;
                if next_left.min(next_right) != lower
                    || next_left.max(next_right) != upper
                    || next_positive != positive
                    || material_at(if next_positive { x } else { x + 1 }, z + depth)? != material
                {
                    break;
                }
                depth += 1;
            }
            let extent = [
                u16::try_from(depth)
                    .map_err(|_| VirtualTerrainRendererError::InvalidTriangleCluster(page.key))?,
                u16::try_from(upper - lower)
                    .map_err(|_| VirtualTerrainRendererError::InvalidTriangleCluster(page.key))?,
            ];
            push_bounded_virtual_quad(
                &mut base,
                make_quad(
                    [
                        coordinate(page.bounds.min.x, x + 1)?.saturating_sub(i32::from(positive)),
                        lower,
                        coordinate(page.bounds.min.z, z)?,
                    ],
                    extent,
                    if positive { 0 } else { 1 },
                    material,
                ),
            );
            z += depth;
        }
    }

    for z in 0..cells {
        let mut x = 0usize;
        while x < cells {
            let near = ground_at(x, z)?;
            let far = ground_at(x, z + 1)?;
            if near == far {
                x += 1;
                continue;
            }
            let positive = near > far;
            let lower = near.min(far);
            let upper = near.max(far);
            let material = material_at(x, if positive { z } else { z + 1 })?;
            let mut width = 1usize;
            while x + width < cells {
                let next_near = ground_at(x + width, z)?;
                let next_far = ground_at(x + width, z + 1)?;
                let next_positive = next_near > next_far;
                if next_near.min(next_far) != lower
                    || next_near.max(next_far) != upper
                    || next_positive != positive
                    || material_at(x + width, if next_positive { z } else { z + 1 })? != material
                {
                    break;
                }
                width += 1;
            }
            let extent = [
                u16::try_from(width)
                    .map_err(|_| VirtualTerrainRendererError::InvalidTriangleCluster(page.key))?,
                u16::try_from(upper - lower)
                    .map_err(|_| VirtualTerrainRendererError::InvalidTriangleCluster(page.key))?,
            ];
            push_bounded_virtual_quad(
                &mut base,
                make_quad(
                    [
                        coordinate(page.bounds.min.x, x)?,
                        lower,
                        coordinate(page.bounds.min.z, z + 1)?.saturating_sub(i32::from(positive)),
                    ],
                    extent,
                    if positive { 4 } else { 5 },
                    material,
                ),
            );
            x += width;
        }
    }

    let water = page
        .materials
        .iter()
        .find(|coverage| coverage.material == Material::Water)
        .map(|coverage| coverage.material);
    if let Some(water) = water {
        let mut flat_heights = vec![None; cells * cells];
        for z in 0..cells {
            for x in 0..cells {
                let samples = [
                    x + z * edge,
                    x + 1 + z * edge,
                    x + 1 + (z + 1) * edge,
                    x + (z + 1) * edge,
                ];
                let heights = samples.map(|sample| heightfield.water[sample]);
                if heights.into_iter().all(|height| height.is_some()) {
                    let values = heights.map(|height| lattice_height(height.unwrap()));
                    let [left, right, far_right, far_left] = values;
                    let [left, right, far_right, far_left] = [left?, right?, far_right?, far_left?];
                    if left != right || left != far_right || left != far_left {
                        return Ok(None);
                    }
                    flat_heights[x + z * cells] = Some(left);
                }
            }
        }
        let mut water_emitted = vec![false; cells * cells];
        for z in 0..cells {
            for x in 0..cells {
                let cell = x + z * cells;
                let Some(height) = flat_heights[cell].filter(|_| !water_emitted[cell]) else {
                    continue;
                };
                let mut width = 1usize;
                while x + width < cells
                    && !water_emitted[x + width + z * cells]
                    && flat_heights[x + width + z * cells] == Some(height)
                {
                    width += 1;
                }
                let mut depth = 1usize;
                'water: while z + depth < cells {
                    for offset in 0..width {
                        let candidate = x + offset + (z + depth) * cells;
                        if water_emitted[candidate] || flat_heights[candidate] != Some(height) {
                            break 'water;
                        }
                    }
                    depth += 1;
                }
                for row in z..z + depth {
                    water_emitted[row * cells + x..row * cells + x + width].fill(true);
                }
                push_bounded_virtual_quad(
                    &mut base,
                    make_quad(
                        [
                            coordinate(page.bounds.min.x, x)?,
                            height.saturating_sub(1),
                            coordinate(page.bounds.min.z, z)?,
                        ],
                        [
                            u16::try_from(width).map_err(|_| {
                                VirtualTerrainRendererError::InvalidTriangleCluster(page.key)
                            })?,
                            u16::try_from(depth).map_err(|_| {
                                VirtualTerrainRendererError::InvalidTriangleCluster(page.key)
                            })?,
                        ],
                        2,
                        water,
                    ),
                );
            }
        }
    }

    Ok(Some(
        constrain_gpu_quad_t_junctions(&base, |_, _| true, |_, _, _, _| true, false)
            .into_iter()
            .flatten()
            .collect(),
    ))
}

fn push_virtual_microvoxel_ground(
    vertices: &mut Vec<GpuTerrainVertex>,
    page: &TerrainPageV1,
    grid: &voxels_world::TerrainHeightfieldGrid,
    heightfield: &VirtualHeightfieldSamples,
) -> Result<(), VirtualTerrainRendererError> {
    let coarse_cells = TERRAIN_PAGE_EDGE_SAMPLES as usize;
    let coarse_edge = coarse_cells + 1;
    let stride = usize::try_from(grid.sample_stride_voxels)
        .map_err(|_| VirtualTerrainRendererError::InvalidTriangleCluster(page.key))?;
    let cells = coarse_cells.checked_mul(stride).ok_or(
        VirtualTerrainRendererError::InvalidTriangleCluster(page.key),
    )?;
    if stride != 1
        || grid.sample_material_indices.len() != coarse_edge * coarse_edge
        || grid.cell_material_indices.len() != coarse_cells * coarse_cells
        || heightfield.ground.len() != coarse_edge * coarse_edge
    {
        return Err(VirtualTerrainRendererError::InvalidTriangleCluster(
            page.key,
        ));
    }
    let coordinate = |origin: i32, cell: usize| {
        i32::try_from(cell)
            .ok()
            .and_then(|offset| origin.checked_add(offset))
            .map(|value| value as f32)
            .ok_or(VirtualTerrainRendererError::InvalidTriangleCluster(
                page.key,
            ))
    };
    let ground_at = |x: usize, z: usize| {
        let coarse_x = (x / stride).min(coarse_cells);
        let coarse_z = (z / stride).min(coarse_cells);
        let next_x = (coarse_x + 1).min(coarse_cells);
        let next_z = (coarse_z + 1).min(coarse_cells);
        let fraction_x = if coarse_x == coarse_cells {
            0.0
        } else {
            (x % stride) as f32 / stride as f32
        };
        let fraction_z = if coarse_z == coarse_cells {
            0.0
        } else {
            (z % stride) as f32 / stride as f32
        };
        let near_left = heightfield.ground[coarse_x + coarse_z * coarse_edge];
        let near_right = heightfield.ground[next_x + coarse_z * coarse_edge];
        let far_left = heightfield.ground[coarse_x + next_z * coarse_edge];
        let far_right = heightfield.ground[next_x + next_z * coarse_edge];
        let near = near_left + (near_right - near_left) * fraction_x;
        let far = far_left + (far_right - far_left) * fraction_x;
        // L1 carries the same values as its L0 children, interpolated locally instead of
        // transmitted. Rounding on the canonical integer lattice gives both owners the same
        // axis-aligned 10 cm boundary trace.
        (near + (far - near) * fraction_z).round()
    };
    let material_at = |x: usize, z: usize| {
        let material_index = if stride == 1 {
            grid.sample_material_indices
                .get(x + z * coarse_edge)
                .copied()
        } else {
            let coarse_x = (x / stride).min(coarse_cells - 1);
            let coarse_z = (z / stride).min(coarse_cells - 1);
            grid.cell_material_indices
                .get(coarse_x + coarse_z * coarse_cells)
                .copied()
        };
        material_index
            .and_then(|index| page.materials.get(usize::from(index)))
            .map(|coverage| coverage.material)
            .ok_or(VirtualTerrainRendererError::InvalidTriangleCluster(
                page.key,
            ))
    };

    // Merge coplanar cells, but preserve every 10 cm perimeter vertex so the result remains a
    // conforming tessellation against wall faces and independently merged neighboring pages.
    let mut emitted = vec![false; cells * cells];
    for z in 0..cells {
        for x in 0..cells {
            let cell = x + z * cells;
            if emitted[cell] {
                continue;
            }
            let height = ground_at(x, z);
            let material = material_at(x, z)?;
            let matches = |candidate_x: usize, candidate_z: usize| {
                let candidate_cell = candidate_x + candidate_z * cells;
                !emitted[candidate_cell]
                    && ground_at(candidate_x, candidate_z).to_bits() == height.to_bits()
                    && material_at(candidate_x, candidate_z)
                        .is_ok_and(|candidate| candidate == material)
            };
            let mut width = 1usize;
            while x + width < cells && matches(x + width, z) {
                width += 1;
            }
            let mut depth = 1usize;
            'grow: while z + depth < cells {
                for offset in 0..width {
                    if !matches(x + offset, z + depth) {
                        break 'grow;
                    }
                }
                depth += 1;
            }
            for row in z..z + depth {
                emitted[row * cells + x..row * cells + x + width].fill(true);
            }
            push_virtual_conforming_rectangle(
                vertices,
                [
                    [
                        coordinate(page.bounds.min.x, x)?,
                        height,
                        coordinate(page.bounds.min.z, z)?,
                    ],
                    [
                        coordinate(page.bounds.min.x, x + width)?,
                        height,
                        coordinate(page.bounds.min.z, z)?,
                    ],
                    [
                        coordinate(page.bounds.min.x, x + width)?,
                        height,
                        coordinate(page.bounds.min.z, z + depth)?,
                    ],
                    [
                        coordinate(page.bounds.min.x, x)?,
                        height,
                        coordinate(page.bounds.min.z, z + depth)?,
                    ],
                ],
                true,
                material,
                page.key,
            )?;
        }
    }

    // The positive-X half-open edge is the sole owner of each exact vertical step. Compatible
    // runs share one conforming rectangle without removing their unit edge vertices.
    for x in 0..cells {
        let mut z = 0usize;
        while z < cells {
            let left = ground_at(x, z);
            let right = ground_at(x + 1, z);
            if left == right {
                z += 1;
                continue;
            }
            let positive = left > right;
            let lower = left.min(right);
            let upper = left.max(right);
            let material = material_at(if positive { x } else { x + 1 }, z)?;
            let mut depth = 1usize;
            while z + depth < cells {
                let next_left = ground_at(x, z + depth);
                let next_right = ground_at(x + 1, z + depth);
                let next_positive = next_left > next_right;
                let next_material = material_at(if next_positive { x } else { x + 1 }, z + depth)?;
                if next_left.min(next_right) != lower
                    || next_left.max(next_right) != upper
                    || next_positive != positive
                    || next_material != material
                {
                    break;
                }
                depth += 1;
            }
            let plane_x = coordinate(page.bounds.min.x, x + 1)?;
            push_virtual_conforming_rectangle(
                vertices,
                [
                    [plane_x, lower, coordinate(page.bounds.min.z, z)?],
                    [plane_x, upper, coordinate(page.bounds.min.z, z)?],
                    [plane_x, upper, coordinate(page.bounds.min.z, z + depth)?],
                    [plane_x, lower, coordinate(page.bounds.min.z, z + depth)?],
                ],
                !positive,
                material,
                page.key,
            )?;
            z += depth;
        }
    }

    // Apply the same single-owner rule to positive-Z steps.
    for z in 0..cells {
        let mut x = 0usize;
        while x < cells {
            let near = ground_at(x, z);
            let far = ground_at(x, z + 1);
            if near == far {
                x += 1;
                continue;
            }
            let positive = near > far;
            let lower = near.min(far);
            let upper = near.max(far);
            let material = material_at(x, if positive { z } else { z + 1 })?;
            let mut width = 1usize;
            while x + width < cells {
                let next_near = ground_at(x + width, z);
                let next_far = ground_at(x + width, z + 1);
                let next_positive = next_near > next_far;
                let next_material = material_at(x + width, if next_positive { z } else { z + 1 })?;
                if next_near.min(next_far) != lower
                    || next_near.max(next_far) != upper
                    || next_positive != positive
                    || next_material != material
                {
                    break;
                }
                width += 1;
            }
            let plane_z = coordinate(page.bounds.min.z, z + 1)?;
            push_virtual_conforming_rectangle(
                vertices,
                [
                    [coordinate(page.bounds.min.x, x)?, lower, plane_z],
                    [coordinate(page.bounds.min.x, x + width)?, lower, plane_z],
                    [coordinate(page.bounds.min.x, x + width)?, upper, plane_z],
                    [coordinate(page.bounds.min.x, x)?, upper, plane_z],
                ],
                !positive,
                material,
                page.key,
            )?;
            x += width;
        }
    }
    Ok(())
}

fn push_virtual_rectangle(
    vertices: &mut Vec<GpuTerrainVertex>,
    positions: [[f32; 3]; 4],
    reverse_winding: bool,
    material: Material,
    key: TerrainPageKey,
) -> Result<(), VirtualTerrainRendererError> {
    let triangles = if reverse_winding {
        [[0, 2, 1], [0, 3, 2]]
    } else {
        [[0, 1, 2], [0, 2, 3]]
    };
    for triangle in triangles {
        push_virtual_triangle(
            vertices,
            triangle.map(|corner| positions[corner]),
            material,
            key,
        )?;
    }
    Ok(())
}

fn push_virtual_conforming_rectangle(
    vertices: &mut Vec<GpuTerrainVertex>,
    positions: [[f32; 3]; 4],
    reverse_winding: bool,
    material: Material,
    key: TerrainPageKey,
) -> Result<(), VirtualTerrainRendererError> {
    let sides = [
        [positions[0], positions[1]],
        [positions[1], positions[2]],
        [positions[2], positions[3]],
        [positions[3], positions[0]],
    ];
    let mut segments = Vec::new();
    let mut side_lengths = [0usize; 4];
    for (side, [start, end]) in sides.into_iter().enumerate() {
        let length = (0..3)
            .map(|axis| (end[axis] - start[axis]).abs())
            .fold(0.0f32, f32::max);
        let subdivisions = usize::try_from(length as u32)
            .ok()
            .filter(|subdivisions| {
                *subdivisions > 0 && (*subdivisions as f32 - length).abs() <= f32::EPSILON
            })
            .ok_or(VirtualTerrainRendererError::InvalidTriangleCluster(key))?;
        side_lengths[side] = subdivisions;
        for offset in 0..subdivisions {
            let interpolate = |amount: f32| {
                std::array::from_fn(|axis| start[axis] + (end[axis] - start[axis]) * amount)
            };
            segments.push([
                interpolate(offset as f32 / subdivisions as f32),
                interpolate((offset + 1) as f32 / subdivisions as f32),
            ]);
        }
    }
    if segments.len() == 4 {
        return push_virtual_rectangle(vertices, positions, reverse_winding, material, key);
    }

    let width = side_lengths[0];
    let height = side_lengths[1];
    if width >= 2 && height >= 2 {
        // An integer point one cell in from both sides is strictly inside the rectangle.
        // Fanning every unit perimeter segment to that point keeps all vertices on the
        // canonical voxel lattice, while avoiding the T-junctions produced by a single quad.
        let anchor = std::array::from_fn(|axis| {
            positions[0][axis]
                + (positions[1][axis] - positions[0][axis]) / width as f32
                + (positions[3][axis] - positions[0][axis]) / height as f32
        });
        for [start, end] in segments {
            let triangle = if reverse_winding {
                [start, anchor, end]
            } else {
                [start, end, anchor]
            };
            push_virtual_triangle(vertices, triangle, material, key)?;
        }
        return Ok(());
    }

    // A one-cell-wide rectangle has no strictly interior lattice point. Split only the long
    // direction into unit quads; this preserves the same conforming boundary without a
    // fractional center vertex.
    let (first, second, strips) = if width >= height {
        (
            [positions[0], positions[1]],
            [positions[3], positions[2]],
            width,
        )
    } else {
        (
            [positions[0], positions[3]],
            [positions[1], positions[2]],
            height,
        )
    };
    let interpolate = |edge: [[f32; 3]; 2], amount: f32| {
        std::array::from_fn(|axis| edge[0][axis] + (edge[1][axis] - edge[0][axis]) * amount)
    };
    for offset in 0..strips {
        let start = offset as f32 / strips as f32;
        let end = (offset + 1) as f32 / strips as f32;
        push_virtual_rectangle(
            vertices,
            [
                interpolate(first, start),
                interpolate(first, end),
                interpolate(second, end),
                interpolate(second, start),
            ],
            reverse_winding,
            material,
            key,
        )?;
    }
    Ok(())
}

fn push_virtual_flat_water_rectangles(
    vertices: &mut Vec<GpuTerrainVertex>,
    page: &TerrainPageV1,
    grid: &voxels_world::TerrainHeightfieldGrid,
    heightfield: &VirtualHeightfieldSamples,
    material: Material,
) -> Result<(), VirtualTerrainRendererError> {
    let cells = TERRAIN_PAGE_EDGE_SAMPLES as usize;
    let edge = cells + 1;
    let mut flat_heights = vec![None; cells * cells];
    for z in 0..cells {
        for x in 0..cells {
            let samples = [
                x + z * edge,
                x + 1 + z * edge,
                x + 1 + (z + 1) * edge,
                x + (z + 1) * edge,
            ];
            let heights = samples.map(|sample| heightfield.water[sample]);
            if let Some(height) = heights[0]
                && heights
                    .into_iter()
                    .all(|candidate| candidate == Some(height))
            {
                flat_heights[x + z * cells] = Some(height);
            }
        }
    }

    let stride = i32::try_from(grid.sample_stride_voxels)
        .map_err(|_| VirtualTerrainRendererError::InvalidTriangleCluster(page.key))?;
    let mut emitted = vec![false; cells * cells];
    for z in 0..cells {
        for x in 0..cells {
            let index = x + z * cells;
            let Some(height) = flat_heights[index].filter(|_| !emitted[index]) else {
                continue;
            };
            let mut width = 1usize;
            while x + width < cells {
                let candidate = x + width + z * cells;
                if emitted[candidate] || flat_heights[candidate] != Some(height) {
                    break;
                }
                width += 1;
            }
            let mut depth = 1usize;
            'grow: while z + depth < cells {
                for offset in 0..width {
                    let candidate = x + offset + (z + depth) * cells;
                    if emitted[candidate] || flat_heights[candidate] != Some(height) {
                        break 'grow;
                    }
                }
                depth += 1;
            }
            for row in z..z + depth {
                emitted[row * cells + x..row * cells + x + width].fill(true);
            }

            let coordinate = |origin: i32, cell: usize| {
                i32::try_from(cell)
                    .ok()
                    .and_then(|cell| cell.checked_mul(stride))
                    .and_then(|offset| origin.checked_add(offset))
                    .map(|coordinate| coordinate as f32)
                    .ok_or(VirtualTerrainRendererError::InvalidTriangleCluster(
                        page.key,
                    ))
            };
            let minimum_x = coordinate(page.bounds.min.x, x)?;
            let maximum_x = coordinate(page.bounds.min.x, x + width)?;
            let minimum_z = coordinate(page.bounds.min.z, z)?;
            let maximum_z = coordinate(page.bounds.min.z, z + depth)?;
            let rectangle = [
                [minimum_x, height, minimum_z],
                [maximum_x, height, minimum_z],
                [maximum_x, height, maximum_z],
                [minimum_x, height, maximum_z],
            ];
            for triangle in [[0, 2, 1], [0, 3, 2]] {
                push_virtual_triangle(
                    vertices,
                    triangle.map(|corner| rectangle[corner]),
                    material,
                    page.key,
                )?;
            }
        }
    }
    Ok(())
}

fn unconstrained_virtual_heightfield_samples(
    grid: &voxels_world::TerrainHeightfieldGrid,
) -> VirtualHeightfieldSamples {
    VirtualHeightfieldSamples {
        ground: grid
            .ground_heights
            .iter()
            .map(|height| *height as f32)
            .collect(),
        water: grid
            .water_heights
            .iter()
            .map(|height| (*height != i32::MIN).then_some(*height as f32))
            .collect(),
        exact_neighbor_sides: [false; 4],
        finer_neighbor_sides: [false; 4],
    }
}

fn cut_finer_neighbor_sides(selected: &BTreeSet<TerrainPageKey>, key: TerrainPageKey) -> [bool; 4] {
    let Some(finer_level) = key.level.checked_sub(1) else {
        return [false; 4];
    };
    if !key.is_surface() {
        return [false; 4];
    }
    let x = key.coord[0].saturating_mul(2);
    let z = key.coord[2].saturating_mul(2);
    let sides = [
        [
            TerrainPageKey::surface(finer_level, x.saturating_sub(1), z),
            TerrainPageKey::surface(finer_level, x.saturating_sub(1), z.saturating_add(1)),
        ],
        [
            TerrainPageKey::surface(finer_level, x.saturating_add(2), z),
            TerrainPageKey::surface(finer_level, x.saturating_add(2), z.saturating_add(1)),
        ],
        [
            TerrainPageKey::surface(finer_level, x, z.saturating_sub(1)),
            TerrainPageKey::surface(finer_level, x.saturating_add(1), z.saturating_sub(1)),
        ],
        [
            TerrainPageKey::surface(finer_level, x, z.saturating_add(2)),
            TerrainPageKey::surface(finer_level, x.saturating_add(1), z.saturating_add(2)),
        ],
    ];
    sides.map(|neighbors| neighbors.into_iter().all(|key| selected.contains(&key)))
}

fn cut_exact_neighbor_sides(selected: &BTreeSet<TerrainPageKey>, key: TerrainPageKey) -> [bool; 4] {
    if key.level != 0 || !key.is_surface() {
        return [false; 4];
    }
    [(-1, 0), (1, 0), (0, -1), (0, 1)].map(|(offset_x, offset_z)| {
        selected.contains(&TerrainPageKey::surface(
            0,
            key.coord[0].saturating_add(offset_x),
            key.coord[2].saturating_add(offset_z),
        ))
    })
}

fn restore_exact_neighbor_heightfield_boundaries(
    page: &TerrainPageV1,
    grid: &voxels_world::TerrainHeightfieldGrid,
    constrained: &VirtualHeightfieldSamples,
    exact_sides: [bool; 4],
) -> VirtualHeightfieldSamples {
    let raw = unconstrained_virtual_heightfield_samples(grid);
    let mut restored = constrained.clone();
    let edge = TERRAIN_PAGE_EDGE_SAMPLES as usize + 1;
    let cells = TERRAIN_PAGE_EDGE_SAMPLES as usize;
    debug_assert_eq!(page.key.level, 0);
    restored.exact_neighbor_sides = exact_sides;
    for offset in 0..edge {
        let samples = [
            offset * edge,
            cells + offset * edge,
            offset,
            offset + cells * edge,
        ];
        for (exact, sample) in exact_sides.into_iter().zip(samples) {
            if !exact {
                continue;
            }
            restored.ground[sample] = raw.ground[sample];
            restored.water[sample] = raw.water[sample];
        }
    }
    // A corner belongs to both incident boundaries. Raw exact-neighbor restoration is valid only
    // when both boundaries have exact owners; otherwise the parent-constrained value is the sole
    // value shared with the adjacent coarse page. Letting either exact side win independently
    // creates a raised corner and a vertical pinhole at four-page L0/L1 junctions.
    for (sample, exact) in [
        (0, exact_sides[0] && exact_sides[2]),
        (cells, exact_sides[1] && exact_sides[2]),
        (cells + cells * edge, exact_sides[1] && exact_sides[3]),
        (cells * edge, exact_sides[0] && exact_sides[3]),
    ] {
        if !exact {
            restored.ground[sample] = constrained.ground[sample];
            restored.water[sample] = constrained.water[sample];
        }
    }
    restored
}

#[cfg(test)]
fn constrained_virtual_heightfield_samples(
    hierarchy: &VirtualTerrainHierarchy,
    page: &TerrainPageV1,
) -> Result<VirtualHeightfieldSamples, VirtualTerrainRendererError> {
    let samples = parent_constrained_virtual_heightfield_samples(hierarchy, page)?;
    let TerrainPageRepresentation::HeightfieldGrid(grid) = &page.representation else {
        return Err(VirtualTerrainRendererError::UnsupportedRepresentation(
            page.representation.kind(),
        ));
    };
    if page.key.level == 0 {
        let selected = [(-1, 0), (1, 0), (0, -1), (0, 1)]
            .into_iter()
            .filter_map(|(offset_x, offset_z)| {
                let key = TerrainPageKey::surface(
                    0,
                    page.key.coord[0].saturating_add(offset_x),
                    page.key.coord[2].saturating_add(offset_z),
                );
                hierarchy.directory_node(key).map(|_| key)
            })
            .chain(std::iter::once(page.key))
            .collect::<BTreeSet<_>>();
        Ok(restore_exact_neighbor_heightfield_boundaries(
            page,
            grid,
            &samples,
            cut_exact_neighbor_sides(&selected, page.key),
        ))
    } else {
        Ok(samples)
    }
}

/// Returns only the recursively parent-constrained samples.
///
/// Exact-neighbor restoration depends on the current directory set and must be reapplied whenever
/// a neighboring refinement arrives. Caching that restored edge made geometry depend on response
/// order and left adjacent L0 pages with different values for the same boundary.
fn heightfield_ancestor_fingerprint(
    hierarchy: &VirtualTerrainHierarchy,
    mut key: TerrainPageKey,
) -> Option<u64> {
    let mut fingerprint = FINGERPRINT_OFFSET;
    loop {
        let node = hierarchy.directory_node(key)?;
        fingerprint = fingerprint_value(fingerprint, u64::from(key.level));
        fingerprint = fingerprint_value(fingerprint, key.coord[0] as u64);
        fingerprint = fingerprint_value(fingerprint, key.coord[2] as u64);
        fingerprint = fingerprint_value(fingerprint, node.revision);
        for chunk in node.content_fingerprint.chunks_exact(8) {
            fingerprint =
                fingerprint_value(fingerprint, u64::from_le_bytes(chunk.try_into().ok()?));
        }
        let Some(parent) = key.parent() else {
            break;
        };
        if hierarchy.directory_node(parent).is_none() {
            break;
        }
        key = parent;
    }
    Some(fingerprint)
}

#[cfg(test)]
fn parent_constrained_virtual_heightfield_samples(
    hierarchy: &VirtualTerrainHierarchy,
    page: &TerrainPageV1,
) -> Result<VirtualHeightfieldSamples, VirtualTerrainRendererError> {
    parent_constrained_virtual_heightfield_samples_with_cache(hierarchy, &BTreeMap::new(), page)
        .map(|(samples, _)| samples)
}

fn parent_constrained_virtual_heightfield_samples_with_cache(
    hierarchy: &VirtualTerrainHierarchy,
    cache: &BTreeMap<TerrainPageKey, CachedVirtualHeightfieldSamples>,
    page: &TerrainPageV1,
) -> Result<(VirtualHeightfieldSamples, bool), VirtualTerrainRendererError> {
    let TerrainPageRepresentation::HeightfieldGrid(grid) = &page.representation else {
        return Err(VirtualTerrainRendererError::UnsupportedRepresentation(
            page.representation.kind(),
        ));
    };
    let mut samples = unconstrained_virtual_heightfield_samples(grid);
    let Some(parent_key) = page.key.parent() else {
        return Ok((samples, true));
    };
    let expected_parent_ancestor_fingerprint =
        heightfield_ancestor_fingerprint(hierarchy, parent_key);
    let (parent_samples, complete) = if let Some(cached) = cache.get(&parent_key).filter(|cached| {
        hierarchy.directory_node(parent_key).is_some_and(|node| {
            node.revision == cached.revision
                && node.content_fingerprint == cached.content_fingerprint
                && expected_parent_ancestor_fingerprint == Some(cached.ancestor_fingerprint)
        })
    }) {
        (cached.samples.clone(), true)
    } else if let Some(parent) = hierarchy.resident_page(parent_key) {
        if !matches!(
            parent.representation,
            TerrainPageRepresentation::HeightfieldGrid(_)
        ) {
            return Ok((samples, false));
        }
        parent_constrained_virtual_heightfield_samples_with_cache(hierarchy, cache, parent)?
    } else {
        return Ok((samples, false));
    };
    let edge = TERRAIN_PAGE_EDGE_SAMPLES as usize + 1;
    let cells = TERRAIN_PAGE_EDGE_SAMPLES as usize;
    if samples.ground.len() != edge * edge
        || samples.water.len() != edge * edge
        || parent_samples.ground.len() != edge * edge
        || parent_samples.water.len() != edge * edge
    {
        return Err(VirtualTerrainRendererError::InvalidTriangleCluster(
            page.key,
        ));
    }
    let quadrant_x = usize::try_from(page.key.coord[0].rem_euclid(2))
        .map_err(|_| VirtualTerrainRendererError::InvalidTriangleCluster(page.key))?;
    let quadrant_z = usize::try_from(page.key.coord[2].rem_euclid(2))
        .map_err(|_| VirtualTerrainRendererError::InvalidTriangleCluster(page.key))?;
    let interpolate_ground = |fixed: usize, combined: usize, row: bool| {
        let coarse = combined / 2;
        let first = if row {
            coarse + fixed * edge
        } else {
            fixed + coarse * edge
        };
        if combined.is_multiple_of(2) {
            parent_samples.ground[first]
        } else {
            let second = if row { first + 1 } else { first + edge };
            (parent_samples.ground[first] + parent_samples.ground[second]) * 0.5
        }
    };
    let interpolate_water = |fixed: usize, combined: usize, row: bool| {
        let coarse = combined / 2;
        let first = if row {
            coarse + fixed * edge
        } else {
            fixed + coarse * edge
        };
        if combined.is_multiple_of(2) {
            parent_samples.water[first]
        } else {
            let second = if row { first + 1 } else { first + edge };
            parent_samples.water[first]
                .zip(parent_samples.water[second])
                .map(|(left, right)| (left + right) * 0.5)
        }
    };
    for offset in 0..=cells {
        if quadrant_z == 0 {
            let combined = quadrant_x * cells + offset;
            samples.ground[offset] = interpolate_ground(0, combined, true);
            samples.water[offset] = interpolate_water(0, combined, true);
        } else {
            let combined = quadrant_x * cells + offset;
            let child = offset + cells * edge;
            samples.ground[child] = interpolate_ground(cells, combined, true);
            samples.water[child] = interpolate_water(cells, combined, true);
        }
        if quadrant_x == 0 {
            let combined = quadrant_z * cells + offset;
            let child = offset * edge;
            samples.ground[child] = interpolate_ground(0, combined, false);
            samples.water[child] = interpolate_water(0, combined, false);
        } else {
            let combined = quadrant_z * cells + offset;
            let child = cells + offset * edge;
            samples.ground[child] = interpolate_ground(cells, combined, false);
            samples.water[child] = interpolate_water(cells, combined, false);
        }
    }
    Ok((samples, complete))
}

fn push_virtual_triangle_i32(
    vertices: &mut Vec<GpuTerrainVertex>,
    positions: [[i32; 3]; 3],
    material: Material,
    key: TerrainPageKey,
) -> Result<(), VirtualTerrainRendererError> {
    push_virtual_triangle(
        vertices,
        positions.map(|position| position.map(|component| component as f32)),
        material,
        key,
    )
}

fn push_virtual_triangle(
    vertices: &mut Vec<GpuTerrainVertex>,
    positions: [[f32; 3]; 3],
    material: Material,
    key: TerrainPageKey,
) -> Result<(), VirtualTerrainRendererError> {
    let [left, middle, right] = positions;
    let edge_a =
        std::array::from_fn::<_, 3, _>(|axis| f64::from(middle[axis]) - f64::from(left[axis]));
    let edge_b =
        std::array::from_fn::<_, 3, _>(|axis| f64::from(right[axis]) - f64::from(left[axis]));
    let cross = [
        edge_a[1] * edge_b[2] - edge_a[2] * edge_b[1],
        edge_a[2] * edge_b[0] - edge_a[0] * edge_b[2],
        edge_a[0] * edge_b[1] - edge_a[1] * edge_b[0],
    ];
    let length = (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt();
    if !length.is_finite() || length <= f64::EPSILON {
        return Err(VirtualTerrainRendererError::InvalidTriangleCluster(key));
    }
    let normal = std::array::from_fn::<_, 3, _>(|axis| {
        let value = (cross[axis] / length * f64::from(i16::MAX)).round();
        value.clamp(f64::from(i16::MIN + 1), f64::from(i16::MAX)) as i16
    });
    for position in positions {
        vertices.push(GpuTerrainVertex {
            position,
            material: pack_virtual_material(material, key.level),
            normal: [normal[0], normal[1], normal[2], 0],
        });
    }
    Ok(())
}

fn virtual_terrain_surface_slice(
    relative_offset: u32,
    size: u32,
    quad_count: u32,
    bounds_min: glam::Vec3,
    bounds_max: glam::Vec3,
    render_layer: RenderLayer,
) -> MeshSlice {
    MeshSlice {
        relative_offset,
        size,
        quad_count,
        bounds_min,
        bounds_max,
        render_layer,
    }
}

fn partition_virtual_surface_geometry(quads: Vec<GpuQuad>) -> Option<(Vec<GpuQuad>, u32, u32)> {
    let (opaque, water): (Vec<_>, Vec<_>) = quads
        .into_iter()
        .partition(|quad| quad.material_face & 0xffff != u32::from(Material::Water.id()));
    let opaque_count = u32::try_from(opaque.len()).ok()?;
    let water_count = u32::try_from(water.len()).ok()?;
    Some((
        opaque.into_iter().chain(water).collect(),
        opaque_count,
        water_count,
    ))
}

fn partition_virtual_triangle_geometry(
    vertices: Vec<GpuTerrainVertex>,
) -> Option<(Vec<GpuTerrainVertex>, u32, u32)> {
    let (opaque, water): (Vec<_>, Vec<_>) = vertices
        .into_iter()
        .partition(|vertex| vertex.material & 0xffff != u32::from(Material::Water.id()));
    let opaque_count = u32::try_from(opaque.len()).ok()?;
    let water_count = u32::try_from(water.len()).ok()?;
    Some((
        opaque.into_iter().chain(water).collect(),
        opaque_count,
        water_count,
    ))
}

const fn pack_canonical_triangle_extent(extent: u16) -> u16 {
    debug_assert!(extent > 0 && extent <= 64);
    let value = extent - 1;
    ((value & 31) << CANONICAL_TRIANGLE_EXTENT_SHIFT) | ((value & 32) << (15 - 5))
}

fn canonical_quad_point(quad: GpuQuad, u: i32, v: i32) -> [i32; 3] {
    let face = ((quad.material_face & GPU_FACE_MASK) >> GPU_FACE_SHIFT) as u8;
    let local = match face {
        0 => [1, v, u],
        1 => [0, v, u],
        2 => [u, 1, v],
        3 => [u, 0, v],
        4 => [u, v, 1],
        _ => [u, v, 0],
    };
    std::array::from_fn(|axis| quad.origin[axis].saturating_add(local[axis]))
}

fn canonical_quad_corners(quad: GpuQuad) -> [[i32; 3]; 4] {
    let [u, v] = quad.extent_voxels.map(i32::from);
    [
        canonical_quad_point(quad, 0, 0),
        canonical_quad_point(quad, u, 0),
        canonical_quad_point(quad, u, v),
        canonical_quad_point(quad, 0, v),
    ]
}

fn canonical_gpu_subrectangle(base: GpuQuad, u: i32, v: i32, extent_voxels: [u16; 2]) -> GpuQuad {
    let face = ((base.material_face & GPU_FACE_MASK) >> GPU_FACE_SHIFT) as u8;
    let mut origin = canonical_quad_point(base, u, v);
    let positive_normal_axis = match face {
        0 => Some(0),
        2 => Some(1),
        4 => Some(2),
        _ => None,
    };
    if let Some(axis) = positive_normal_axis {
        origin[axis] = origin[axis].saturating_sub(1);
    }
    GpuQuad {
        origin,
        extent_voxels,
        ..base
    }
}

fn canonical_gpu_subquad(base: GpuQuad, u: i32, v: i32) -> GpuQuad {
    canonical_gpu_subrectangle(base, u, v, [1, 1])
}

fn axis_line_key(axis: usize, point: [i32; 3]) -> (u8, i32, i32) {
    match axis {
        0 => (0, point[1], point[2]),
        1 => (1, point[0], point[2]),
        _ => (2, point[0], point[1]),
    }
}

fn constrain_gpu_quad_t_junctions(
    base_quads: &[GpuQuad],
    eligible: impl Fn(usize, GpuQuad) -> bool,
    force_unit_edge: impl Fn(usize, usize, [i32; 3], [i32; 3]) -> bool,
    preserve_packed_ao: bool,
) -> Vec<Vec<GpuQuad>> {
    let edge_corners = [(0, 3), (1, 2), (0, 1), (3, 2)];
    let mut line_segments = HashMap::<(u8, i32, i32), Vec<[i32; 2]>>::new();
    for &quad in base_quads {
        let corners = canonical_quad_corners(quad);
        for (start_corner, end_corner) in edge_corners {
            let start = corners[start_corner];
            let end = corners[end_corner];
            let Some(axis) = (0..3).find(|&axis| start[axis] != end[axis]) else {
                continue;
            };
            line_segments
                .entry(axis_line_key(axis, start))
                .or_default()
                .push([start[axis], end[axis]]);
        }
    }
    let mut output = Vec::with_capacity(base_quads.len());
    for (index, &base) in base_quads.iter().enumerate() {
        let corners = canonical_quad_corners(base);
        let mut edge_offsets = std::array::from_fn::<_, 4, _>(|edge_index| {
            let (start_corner, end_corner) = edge_corners[edge_index];
            let start = corners[start_corner];
            let end = corners[end_corner];
            let Some(axis) = (0..3).find(|&axis| start[axis] != end[axis]) else {
                return vec![0];
            };
            let extent = end[axis].saturating_sub(start[axis]);
            debug_assert!(extent > 0);
            let mut offsets = vec![0, extent as u16];
            if force_unit_edge(index, edge_index, start, end) {
                offsets.extend(1..extent as u16);
            }
            for segment in line_segments
                .get(&axis_line_key(axis, start))
                .into_iter()
                .flatten()
                .filter(|segment| segment[0] < end[axis] && start[axis] < segment[1])
            {
                offsets.extend(segment.iter().filter_map(|&coordinate| {
                    (start[axis]..=end[axis])
                        .contains(&coordinate)
                        .then(|| u16::try_from(coordinate - start[axis]).ok())
                        .flatten()
                }));
            }
            offsets.sort_unstable();
            offsets.dedup();
            offsets
        });
        let needs_constraints =
            eligible(index, base) && edge_offsets.iter().any(|offsets| offsets.len() > 2);
        if !needs_constraints {
            output.push(vec![base]);
            continue;
        }
        let constrained = QuadEdge::ALL
            .into_iter()
            .filter(|edge| edge_offsets[edge.index()].len() > 2)
            .collect::<Vec<_>>();
        let (anchor, fill_edge) = match constrained.as_slice() {
            [QuadEdge::NegativeX] => (Some(1), Some(QuadEdge::PositiveZ)),
            [QuadEdge::PositiveX] => (Some(0), Some(QuadEdge::PositiveZ)),
            [QuadEdge::NegativeZ] => (Some(3), Some(QuadEdge::PositiveX)),
            [QuadEdge::PositiveZ] => (Some(0), Some(QuadEdge::PositiveX)),
            [QuadEdge::NegativeX, QuadEdge::NegativeZ] => (Some(2), None),
            [QuadEdge::PositiveX, QuadEdge::NegativeZ] => (Some(3), None),
            [QuadEdge::PositiveX, QuadEdge::PositiveZ] => (Some(0), None),
            [QuadEdge::NegativeX, QuadEdge::PositiveZ] => (Some(1), None),
            _ => (None, None),
        };
        let extent = base.extent_voxels;
        if anchor.is_none() && (extent[0] == 1 || extent[1] == 1) {
            output.push(
                (0..i32::from(extent[1]))
                    .flat_map(|v| {
                        (0..i32::from(extent[0])).map(move |u| canonical_gpu_subquad(base, u, v))
                    })
                    .collect(),
            );
            continue;
        }
        let lattice_anchor =
            (anchor.is_none() && extent[0] >= 2 && extent[1] >= 2).then_some([1_u16, 1_u16]);
        let emitted_edges = if anchor.is_some() {
            constrained.into_iter().chain(fill_edge).collect::<Vec<_>>()
        } else {
            QuadEdge::ALL.to_vec()
        };
        let mut triangles = Vec::new();
        for edge in emitted_edges {
            let edge_index = edge.index();
            let offsets = &mut edge_offsets[edge_index];
            offsets.sort_unstable();
            offsets.dedup();
            let fallback;
            let offsets = if Some(edge) == fill_edge {
                fallback = match edge {
                    QuadEdge::NegativeX | QuadEdge::PositiveX => [0, extent[1]],
                    QuadEdge::NegativeZ | QuadEdge::PositiveZ => [0, extent[0]],
                };
                fallback.as_slice()
            } else {
                offsets.as_slice()
            };
            for bounds in offsets.windows(2) {
                let [start, end] = [bounds[0], bounds[1]];
                debug_assert!(end <= CANONICAL_TRIANGLE_OFFSET_MASK);
                triangles.push(GpuQuad {
                    extent_voxels: [
                        start
                            | pack_canonical_triangle_extent(extent[0])
                            | ((edge.index() as u16) << CANONICAL_TRIANGLE_EDGE_SHIFT)
                            | CANONICAL_TRIANGLE_FLAG,
                        end | pack_canonical_triangle_extent(extent[1])
                            | ((anchor.map_or_else(
                                || {
                                    if lattice_anchor.is_some() {
                                        usize::from(CANONICAL_TRIANGLE_LATTICE_ANCHOR)
                                    } else {
                                        0
                                    }
                                },
                                |corner| corner + 1,
                            ) as u16)
                                << CANONICAL_TRIANGLE_ANCHOR_SHIFT),
                    ],
                    ao: (if preserve_packed_ao {
                        base.ao
                    } else {
                        canonical_triangle_ao(base.ao as u8, edge, [start, end], extent, anchor)
                    }) | lattice_anchor.map_or(0, |[u, v]| {
                        (u32::from(u) << CANONICAL_TRIANGLE_ANCHOR_U_SHIFT)
                            | (u32::from(v) << CANONICAL_TRIANGLE_ANCHOR_V_SHIFT)
                    }),
                    ..base
                });
            }
        }
        if let Some(first) = triangles.first_mut() {
            first.extent_voxels[1] |= CANONICAL_TRIANGLE_SHADOW_OWNER_FLAG;
        }
        output.push(triangles);
    }
    output
}

fn canonical_gpu_quads(world_origin: [i32; 3], quads: &[Quad]) -> Vec<GpuQuad> {
    let base_quads = quads
        .iter()
        .map(|quad| canonical_gpu_quad(world_origin, quad))
        .collect::<Vec<_>>();
    let chunk_max = world_origin.map(|value| value.saturating_add(CHUNK_EDGE as i32));
    constrain_gpu_quad_t_junctions(
        &base_quads,
        |_, quad| {
            quad.extent_voxels[0] <= 63
                && quad.extent_voxels[1] <= 63
                && quad.extent_voxels.into_iter().all(|extent| extent > 0)
        },
        |_, _, start, end| {
            // Adjacent canonical chunks are uploaded independently, so their boundary vertices
            // are not present in `base_quads`.
            // Subdivide every chunk-boundary edge onto the authoritative 10 cm lattice. Both
            // owners then rasterize identical short edges instead of a greedy long edge meeting
            // several independently rounded segments at a T-junction.
            (0..3).any(|axis| {
                start[axis] == end[axis]
                    && (start[axis] == world_origin[axis] || start[axis] == chunk_max[axis])
            })
        },
        false,
    )
    .into_iter()
    .flatten()
    .collect()
}

struct ChunkMesh {
    allocation: Allocation,
    quad_count: u32,
    content_fingerprint: u64,
    slices: Vec<MeshSlice>,
    bounds_min: glam::Vec3,
    bounds_max: glam::Vec3,
    activation_mask: u8,
}

struct VirtualTerrainGpuPage {
    revision: u64,
    content_fingerprint: [u8; 32],
    representation: TerrainPageRepresentationKind,
    heightfield_exact_neighbor_sides: [bool; 4],
    heightfield_finer_neighbor_sides: [bool; 4],
    heightfield_ground_corner_bits: [u32; 4],
    heightfield_ground_boundary_bits: [[u32; TERRAIN_PAGE_EDGE_SAMPLES as usize + 1]; 4],
    mesh: VirtualTerrainGpuMesh,
}

enum VirtualTerrainGpuMesh {
    Empty,
    Surface(ChunkMesh),
    Triangle(TerrainTriangleMesh),
}

impl VirtualTerrainGpuMesh {
    const fn allocation(&self) -> Option<Allocation> {
        match self {
            Self::Empty => None,
            Self::Surface(mesh) => Some(mesh.allocation),
            Self::Triangle(mesh) => Some(mesh.allocation),
        }
    }
}

struct TerrainTriangleMesh {
    allocation: Allocation,
    vertex_count: u32,
    opaque_vertex_count: u32,
    water_vertex_count: u32,
    content_fingerprint: u64,
    bounds_min: glam::Vec3,
    bounds_max: glam::Vec3,
}

struct PreparedCanonicalChunkUpload {
    key: MeshKey,
    opaque: Option<ChunkMesh>,
    translucent: Option<ChunkMesh>,
    local_lights: Vec<GpuLocalLight>,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChunkActivationReason {
    Radial = 1,
    Portal = 2,
    Interaction = 4,
    Surface = 8,
    EnclosedView = 16,
}

impl ChunkMesh {
    const fn active(&self) -> bool {
        self.activation_mask != 0
    }
}

const fn update_activation_mask(mask: u8, reason: ChunkActivationReason, active: bool) -> u8 {
    if active {
        mask | reason as u8
    } else {
        mask & !(reason as u8)
    }
}

#[derive(Default)]
struct ChunkActivations {
    masks: BTreeMap<MeshKey, u8>,
}

impl ChunkActivations {
    fn set(&mut self, key: MeshKey, reason: ChunkActivationReason, active: bool) -> u8 {
        debug_assert_eq!(key.0, 0);
        let mask =
            update_activation_mask(self.masks.get(&key).copied().unwrap_or(0), reason, active);
        if mask == 0 {
            self.masks.remove(&key);
        } else {
            self.masks.insert(key, mask);
        }
        mask
    }

    fn upload_mask(&self, key: MeshKey) -> u8 {
        if key.0 == 0 {
            self.masks.get(&key).copied().unwrap_or(0)
        } else {
            u8::MAX
        }
    }

    fn remove(&mut self, key: MeshKey) {
        self.masks.remove(&key);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct MeshSlice {
    relative_offset: u32,
    size: u32,
    quad_count: u32,
    bounds_min: glam::Vec3,
    bounds_max: glam::Vec3,
    render_layer: RenderLayer,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DrawItem {
    page: u16,
    offset: u32,
    size: u32,
    quad_count: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DrawSpan {
    page: u16,
    offset: u32,
    size: u32,
    quad_count: u32,
}

#[derive(Debug, Default, Eq, PartialEq)]
struct DrawList {
    spans: Vec<DrawSpan>,
    mesh_count: u32,
    quad_count: u32,
    fingerprint: u64,
    tested_slices: u32,
    selected_slices: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TerrainTriangleDrawSpan {
    page: u16,
    offset: u32,
    size: u32,
    vertex_count: u32,
}

#[derive(Debug, Default, Eq, PartialEq)]
struct TerrainTriangleDrawList {
    spans: Vec<TerrainTriangleDrawSpan>,
    mesh_count: u32,
    vertex_count: u32,
    fingerprint: u64,
}

#[derive(Debug, Default, Eq, PartialEq)]
struct VirtualTerrainDrawLists {
    surfaces: DrawList,
    triangles: TerrainTriangleDrawList,
    water_surfaces: DrawList,
    water_triangles: TerrainTriangleDrawList,
    fingerprint: u64,
    mesh_count: u32,
    primitive_count: u32,
}

/// Complete surface columns owned by the currently published virtual cut.
///
/// The renderer never guesses ownership from visual depth. A region joins this set only when the
/// selected pages form a complete quadtree partition of its coverage root.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct VirtualTerrainOwnership {
    roots: BTreeSet<TerrainPageKey>,
}

impl VirtualTerrainOwnership {
    fn from_cut(cut: &VirtualTerrainCut) -> Result<Self, VirtualTerrainRendererError> {
        let selected = cut.selected_pages.iter().copied().collect::<BTreeSet<_>>();
        let roots = cut
            .selected_pages
            .iter()
            .filter_map(|key| {
                key.is_surface()
                    .then(|| key.ancestor_at(TERRAIN_COVERAGE_ROOT_LEVEL))
                    .flatten()
            })
            .collect::<BTreeSet<_>>();
        if roots.is_empty() && !selected.is_empty() {
            return Err(VirtualTerrainRendererError::IncompleteRootPartition(
                cut.selected_pages[0],
            ));
        }
        for root in &roots {
            if !selected_pages_cover(*root, &selected) {
                return Err(VirtualTerrainRendererError::IncompleteRootPartition(*root));
            }
        }
        Ok(Self { roots })
    }

    fn covers_aabb(&self, minimum: glam::Vec3, maximum: glam::Vec3) -> bool {
        let Some((minimum, maximum)) = voxel_bounds_from_metres(minimum, maximum) else {
            return false;
        };
        self.covers_voxel_bounds(minimum, maximum)
    }

    fn covers_voxel_bounds(&self, minimum: [i32; 3], maximum: [i32; 3]) -> bool {
        let Some(ranges) = terrain_surface_root_coords_for_bounds(minimum, maximum) else {
            return false;
        };
        let required = ranges
            .iter()
            .map(|[minimum, maximum]| {
                i64::from(*maximum)
                    .saturating_sub(i64::from(*minimum))
                    .saturating_add(1)
            })
            .try_fold(1_i64, i64::checked_mul);
        if required.is_none_or(|required| required as usize > self.roots.len()) {
            return false;
        }
        terrain_surface_root_coords(ranges).all(|key| self.roots.contains(&key))
    }
}

fn selected_pages_cover(key: TerrainPageKey, selected: &BTreeSet<TerrainPageKey>) -> bool {
    selected.contains(&key)
        || key.refinement_children().is_some_and(|children| {
            children
                .into_iter()
                .all(|child| selected_pages_cover(child, selected))
        })
}

fn voxel_bounds_from_metres(
    minimum: glam::Vec3,
    maximum: glam::Vec3,
) -> Option<([i32; 3], [i32; 3])> {
    let convert = |value: f32, upper: bool| {
        let scaled = f64::from(value) / f64::from(VOXEL_SIZE_METRES);
        if !scaled.is_finite() {
            return None;
        }
        // All terrain bounds originate as integer voxel coordinates, but f32 multiplication by
        // 0.1 is not exact. Snap only numerical dust; genuinely displaced bounds still expand
        // conservatively.
        let nearest = scaled.round();
        let coordinate = if (scaled - nearest).abs() <= 1.0e-4 {
            nearest
        } else if upper {
            scaled.ceil()
        } else {
            scaled.floor()
        };
        (coordinate >= f64::from(i32::MIN) && coordinate <= f64::from(i32::MAX))
            .then_some(coordinate as i32)
    };
    let minimum = [
        convert(minimum.x, false)?,
        convert(minimum.y, false)?,
        convert(minimum.z, false)?,
    ];
    let maximum = [
        convert(maximum.x, true)?,
        convert(maximum.y, true)?,
        convert(maximum.z, true)?,
    ];
    minimum
        .into_iter()
        .zip(maximum)
        .all(|(minimum, maximum)| minimum < maximum)
        .then_some((minimum, maximum))
}

fn terrain_surface_root_coords_for_bounds(
    minimum: [i32; 3],
    maximum: [i32; 3],
) -> Option<[[i32; 2]; 2]> {
    let root_span =
        i32::try_from(32_u32.checked_shl(u32::from(TERRAIN_COVERAGE_ROOT_LEVEL))?).ok()?;
    minimum
        .into_iter()
        .zip(maximum)
        .all(|(minimum, maximum)| minimum < maximum)
        .then(|| {
            [0, 2].map(|axis| {
                [
                    minimum[axis].div_euclid(root_span),
                    maximum[axis].saturating_sub(1).div_euclid(root_span),
                ]
            })
        })
}

fn terrain_surface_root_coords(ranges: [[i32; 2]; 2]) -> impl Iterator<Item = TerrainPageKey> {
    (ranges[0][0]..=ranges[0][1]).flat_map(move |x| {
        (ranges[1][0]..=ranges[1][1])
            .map(move |z| TerrainPageKey::surface(TERRAIN_COVERAGE_ROOT_LEVEL, x, z))
    })
}

#[derive(Debug, Default, Eq, PartialEq)]
struct WorldDrawLists {
    fixed: DrawList,
    mesh_count: u32,
    quad_count: u32,
    fingerprint: u64,
    tested_slices: u32,
    selected_slices: u32,
}

#[derive(Debug)]
struct WorldDrawListBuilder {
    fixed: DrawListBuilder,
    mesh_count: u32,
    quad_count: u32,
    fingerprint: u64,
    tested_slices: u32,
    selected_slices: u32,
}

impl Default for WorldDrawListBuilder {
    fn default() -> Self {
        Self {
            fixed: DrawListBuilder::without_fingerprint(),
            mesh_count: 0,
            quad_count: 0,
            fingerprint: FINGERPRINT_OFFSET,
            tested_slices: 0,
            selected_slices: 0,
        }
    }
}

impl WorldDrawListBuilder {
    fn test_slice(&mut self) {
        self.tested_slices = self.tested_slices.saturating_add(1);
    }

    fn select_slice(&mut self, chunk: &ChunkMesh, slice: &MeshSlice) {
        self.selected_slices = self.selected_slices.saturating_add(1);
        self.quad_count = self.quad_count.saturating_add(slice.quad_count);
        self.fixed.select_slice(chunk, slice);
    }

    fn select_mesh(&mut self, key: MeshKey, chunk: &ChunkMesh) {
        self.mesh_count = self.mesh_count.saturating_add(1);
        self.fingerprint = fingerprint_value(self.fingerprint, u64::from(key.0));
        self.fingerprint = fingerprint_value(self.fingerprint, key.1 as u32 as u64);
        self.fingerprint = fingerprint_value(self.fingerprint, key.2 as u32 as u64);
        self.fingerprint = fingerprint_value(self.fingerprint, key.3 as u32 as u64);
        self.fingerprint = fingerprint_value(self.fingerprint, chunk.content_fingerprint);
    }

    fn finish(mut self) -> WorldDrawLists {
        let fixed = self.fixed.finish();
        for span in &fixed.spans {
            self.fingerprint = fingerprint_value(self.fingerprint, u64::from(span.page));
            self.fingerprint = fingerprint_value(self.fingerprint, u64::from(span.offset));
            self.fingerprint = fingerprint_value(self.fingerprint, u64::from(span.size));
            self.fingerprint = fingerprint_value(self.fingerprint, u64::from(span.quad_count));
        }
        WorldDrawLists {
            fixed,
            mesh_count: self.mesh_count,
            quad_count: self.quad_count,
            fingerprint: self.fingerprint,
            tested_slices: self.tested_slices,
            selected_slices: self.selected_slices,
        }
    }
}

#[derive(Debug)]
struct DrawListBuilder {
    items: Vec<DrawItem>,
    mesh_count: u32,
    quad_count: u32,
    fingerprint: u64,
    fingerprint_enabled: bool,
    tested_slices: u32,
    selected_slices: u32,
}

impl Default for DrawListBuilder {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            mesh_count: 0,
            quad_count: 0,
            fingerprint: FINGERPRINT_OFFSET,
            fingerprint_enabled: true,
            tested_slices: 0,
            selected_slices: 0,
        }
    }
}

impl DrawListBuilder {
    fn without_fingerprint() -> Self {
        Self {
            fingerprint_enabled: false,
            ..Self::default()
        }
    }

    fn select_slice(&mut self, chunk: &ChunkMesh, slice: &MeshSlice) {
        self.selected_slices = self.selected_slices.saturating_add(1);
        let offset = chunk.allocation.offset + slice.relative_offset;
        self.items.push(DrawItem {
            page: chunk.allocation.page,
            offset,
            size: slice.size,
            quad_count: slice.quad_count,
        });
        self.quad_count = self.quad_count.saturating_add(slice.quad_count);
    }

    fn finish(mut self) -> DrawList {
        let spans = coalesce_draw_items(self.items);
        if self.fingerprint_enabled {
            // Hash the actual coalesced GPU ranges rather than every selected source slice. This
            // describes the same presented geometry with hundreds of inputs instead of tens of
            // thousands on a distant viewport.
            for span in &spans {
                self.fingerprint = fingerprint_value(self.fingerprint, u64::from(span.page));
                self.fingerprint = fingerprint_value(self.fingerprint, u64::from(span.offset));
                self.fingerprint = fingerprint_value(self.fingerprint, u64::from(span.size));
                self.fingerprint = fingerprint_value(self.fingerprint, u64::from(span.quad_count));
            }
        }
        DrawList {
            spans,
            mesh_count: self.mesh_count,
            quad_count: self.quad_count,
            fingerprint: self.fingerprint,
            tested_slices: self.tested_slices,
            selected_slices: self.selected_slices,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RenderDiagnostics {
    pub resident_chunks: u32,
    pub visible_chunks: u32,
    pub draw_calls: u32,
    pub water_draw_calls: u32,
    pub shadow_draw_calls: u32,
    pub shadow_cascades: u32,
    pub quads: u32,
    pub water_quads: u32,
    pub virtual_terrain_cpu_selected_pages: u32,
    pub virtual_terrain_cpu_requested_pages: u32,
    pub virtual_terrain_cpu_refinement_roots: u32,
    pub virtual_terrain_cpu_ownerless_roots: u32,
    pub virtual_terrain_cpu_exact_lod_discontinuities: u32,
    pub virtual_terrain_gpu_selected_pages: u32,
    pub virtual_terrain_gpu_ownerless_roots: u32,
    pub virtual_terrain_gpu_encoded_surface_handles: u32,
    pub virtual_terrain_gpu_encoded_triangle_handles: u32,
    pub virtual_terrain_gpu_encoded_water_surface_handles: u32,
    pub virtual_terrain_gpu_encoded_water_triangle_handles: u32,
    pub virtual_terrain_gpu_encoded_pages: u32,
    pub virtual_terrain_gpu_encoding_overflow_flags: u32,
    pub virtual_terrain_gpu_matches_cpu_cut: bool,
    pub virtual_terrain_gpu_match_failure_flags: u32,
    pub virtual_terrain_published_pages: u32,
    pub virtual_terrain_published_ownerless_roots: u32,
    pub virtual_terrain_published_exact_pages: u32,
    pub virtual_terrain_published_minimum_level: u32,
    pub virtual_terrain_published_maximum_level: u32,
    pub virtual_terrain_published_exact_lod_discontinuities: u32,
    /// Whether the immutable motion domain used by this frame was enumerated without overflow.
    pub virtual_terrain_exact_domain_complete: bool,
    /// Number of level-0 surface pages required by that exact motion domain.
    pub virtual_terrain_exact_domain_required_leaves: u32,
    /// Required leaves owned by the currently committed virtual-terrain cut.
    pub virtual_terrain_exact_domain_current_coverage: u32,
    /// Stable identity of the exact motion domain shared by selection and presentation.
    pub virtual_terrain_exact_domain_fingerprint: u64,
    /// Whether the independently bounded current-position safety core is representable.
    pub virtual_terrain_exact_core_complete: bool,
    /// Mandatory current-position leaves which must remain exact even if prediction truncates.
    pub virtual_terrain_exact_core_required_leaves: u32,
    /// Mandatory current-position leaves owned by the committed cut.
    pub virtual_terrain_exact_core_current_coverage: u32,
    /// Whether the full swept prediction, rather than only its current core, was enumerated.
    pub virtual_terrain_exact_prediction_complete: bool,
    /// Full swept-prediction leaves, or zero when prediction was deliberately truncated.
    pub virtual_terrain_exact_prediction_required_leaves: u32,
    /// Full swept-prediction leaves owned by the committed cut.
    pub virtual_terrain_exact_prediction_current_coverage: u32,
    /// Stable identity of the complete virtual hierarchy cut selected for presentation.
    pub virtual_terrain_cut_fingerprint: u64,
    /// Monotonic identity of the immutable GPU handle bank used by this presented frame.
    pub virtual_terrain_presented_snapshot_generation: u64,
    /// CPU-cut fingerprint stored in that exact immutable GPU handle bank.
    pub virtual_terrain_presented_snapshot_fingerprint: u64,
    /// Whether the presented handle bank exactly owns the published CPU cut.
    pub virtual_terrain_presented_snapshot_matches_cut: bool,
    /// Stable identity of the world geometry selected for the latest presented viewport.
    pub viewport_fingerprint: u64,
    pub refraction_copy_bytes: u64,
    pub arena_pages: u32,
    pub arena_capacity_bytes: u64,
    pub arena_allocated_bytes: u64,
    pub core_gpu_bytes: u64,
    pub gpu_sample_id: u32,
    pub gpu_total_ms: Option<f32>,
    pub gpu_shadow_ms: Option<f32>,
    pub gpu_depth_prepass_ms: Option<f32>,
    pub gpu_world_ms: Option<f32>,
    pub gpu_water_ms: Option<f32>,
    pub gpu_ambient_occlusion_ms: Option<f32>,
    pub gpu_cloud_ms: Option<f32>,
    pub gpu_weather_ms: Option<f32>,
    pub gpu_ui_ms: Option<f32>,
    pub gpu_virtual_terrain_snapshot_encode_ms: Option<f32>,
    pub gpu_virtual_terrain_snapshot_validation_ms: Option<f32>,
    pub cpu_cull_ms: f32,
    pub cpu_encode_ms: f32,
    pub cpu_submit_ms: f32,
    pub draw_list_tested_slices: u32,
    pub draw_list_selected_slices: u32,
    pub surface_width: u32,
    pub surface_height: u32,
    pub dpr: f32,
    pub ambient_occlusion_bytes: u64,
    pub depth_prepass_draw_calls: u32,
    pub screen_space_ambient_occlusion: bool,
    pub material_detail: bool,
    pub daylight_phase: u8,
    /// Prime-meridian fraction authored by the server clock.
    pub day_fraction: f32,
    /// Observer-local apparent solar fraction after longitude and pole transport.
    pub local_solar_day_fraction: f32,
    pub year_fraction: f32,
    pub moon_orbit_fraction: f32,
    pub twinkle_phase: f32,
    pub latitude_degrees: f32,
    pub longitude_degrees: f32,
    pub local_sidereal_angle_radians: f32,
    pub sun_direction: [f32; 3],
    pub moon_direction: [f32; 3],
    pub moon_illuminated_fraction: f32,
    pub celestial_revision: u64,
    pub shadow_strength: f32,
    pub surface_region: u8,
    pub cloud_coverage: f32,
    pub cloud_density: f32,
    pub cloud_base_metres: f32,
    pub cloud_top_metres: f32,
    pub cloud_offset_metres: [f32; 2],
    pub cloud_velocity_metres_per_second: [f32; 2],
    pub cloud_render_resolution: [u32; 2],
    pub cloud_steps: [u32; 2],
    pub weather_kind: u8,
    pub weather_fraction: f32,
    pub precipitation: f32,
    pub storminess: f32,
    pub lightning: f32,
    pub fog_density: f32,
    pub outdoor_exposure: f32,
    pub weather_revision: u64,
    pub enclosure: f32,
    pub interior_exposure: f32,
    pub cave_headlamp: bool,
    pub local_light_candidates: u32,
    pub active_local_lights: u32,
    pub clipped_local_lights: u32,
    pub occluded_local_lights: u32,
    pub portal_rejected_local_lights: u32,
    pub local_light_visibility_tests: u32,
    pub local_lighting: bool,
    pub remote_avatars: u32,
    pub avatar_parts: u32,
    pub avatar_draw_calls: u32,
}

#[derive(Debug, Eq, PartialEq)]
pub struct ScreenshotCapture {
    pub filename: String,
    pub metadata: String,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// Top-down little-endian `u32x5` pixels from the terrain draw cut used by the visible frame:
    /// 64-bit owner ID, primitive/face hash, packed representation descriptor, reverse-Z f32 bits.
    pub terrain_diagnostic_u32x5: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ScreenshotWorldIdentity {
    world_id: String,
    source_identity_hash: String,
    source_kind: u8,
    seed: u64,
    world_schema_version: u32,
    material_schema_version: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ScreenshotGpuIdentity {
    name: String,
    vendor: u32,
    device: u32,
    device_type: String,
    device_pci_bus_id: String,
    driver: String,
    driver_info: String,
    backend: String,
    subgroup_min_size: u32,
    subgroup_max_size: u32,
    supported_features: [u64; 2],
    enabled_features: [u64; 2],
    limits: String,
}

#[derive(Default)]
struct ScreenshotReadbackState {
    in_flight: bool,
    completed: Option<ScreenshotCapture>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalLightVisibility {
    Visible,
    Occluded,
    PortalRejected,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct GpuTimingSample {
    pub frame_id: u32,
    pub total_ms: f32,
    pub shadow_ms: f32,
    pub shadow_cascade_ms: [f32; CASCADE_COUNT],
    pub depth_prepass_ms: f32,
    pub world_ms: f32,
    pub water_ms: f32,
    pub ambient_occlusion_ms: f32,
    pub cloud_ms: f32,
    pub weather_ms: f32,
    pub ui_ms: f32,
    pub virtual_terrain_snapshot_encode_ms: f32,
    pub virtual_terrain_snapshot_validation_ms: f32,
}

#[derive(Debug, Default)]
pub struct GpuTimingBatch {
    pub samples: Vec<GpuTimingSample>,
    pub dropped: u32,
}

#[derive(Default)]
struct GpuTimingState {
    latest: Option<GpuTimingSample>,
    history: VecDeque<GpuTimingSample>,
    dropped: u32,
}

struct GpuTimingSlot {
    buffer: Buffer,
    available: Arc<AtomicBool>,
}

struct GpuTimingFrame {
    query_set: QuerySet,
    slot: usize,
    frame_id: u32,
    passes: GpuPassMask,
}

#[derive(Clone, Copy, Debug, Default)]
struct GpuPassMask {
    shadows: bool,
    water: bool,
    ambient_occlusion: bool,
    clouds: bool,
    weather: bool,
    virtual_terrain: bool,
}

impl GpuTimingFrame {
    fn pass(&self, first_query: u32) -> wgpu::RenderPassTimestampWrites<'_> {
        wgpu::RenderPassTimestampWrites {
            query_set: &self.query_set,
            beginning_of_pass_write_index: Some(first_query),
            end_of_pass_write_index: Some(first_query + 1),
        }
    }
}

struct GpuTimer {
    query_set: QuerySet,
    resolve_buffer: Buffer,
    readback: [GpuTimingSlot; GPU_READBACK_SLOTS],
    next_slot: usize,
    timestamp_period: f32,
    state: Arc<Mutex<GpuTimingState>>,
}

fn parse_gpu_timestamps(
    timestamps: &[u64; GPU_QUERY_COUNT as usize],
    timestamp_period: f32,
    passes: GpuPassMask,
) -> Option<GpuTimingSample> {
    if !timestamp_period.is_finite() || timestamp_period <= 0.0 {
        return None;
    }
    let elapsed_ms = |start: usize, end: usize| {
        timestamps[end]
            .checked_sub(timestamps[start])
            .map(|ticks| ticks as f32 * timestamp_period / 1_000_000.0)
            .filter(|milliseconds| milliseconds.is_finite())
    };
    let shadow_cascade_ms = if passes.shadows {
        [elapsed_ms(0, 1)?, elapsed_ms(2, 3)?, elapsed_ms(4, 5)?]
    } else {
        [0.0; CASCADE_COUNT]
    };
    let shadow_ms = shadow_cascade_ms.into_iter().sum();
    let depth_prepass_ms = if passes.ambient_occlusion {
        elapsed_ms(6, 7)?
    } else {
        0.0
    };
    let cloud_ms = if passes.clouds {
        elapsed_ms(12, 13)? + elapsed_ms(16, 17)?
    } else {
        0.0
    };
    let world_ms = elapsed_ms(14, 15)?;
    let water_ms = if passes.water {
        elapsed_ms(18, 19)?
    } else {
        0.0
    };
    let weather_ms = if passes.weather {
        elapsed_ms(20, 21)?
    } else {
        0.0
    };
    let ambient_occlusion_ms = if passes.ambient_occlusion {
        elapsed_ms(8, 9)? + elapsed_ms(10, 11)?
    } else {
        0.0
    };
    let ui_ms = elapsed_ms(22, 23)?;
    let virtual_terrain_snapshot_encode_ms = if passes.virtual_terrain {
        elapsed_ms(24, 25)?
    } else {
        0.0
    };
    let virtual_terrain_snapshot_validation_ms = if passes.virtual_terrain {
        elapsed_ms(26, 27)?
    } else {
        0.0
    };
    let mut first = timestamps[14].min(timestamps[22]);
    let mut last = timestamps[15].max(timestamps[23]);
    if passes.shadows {
        for (start, end) in [(0, 1), (2, 3), (4, 5)] {
            first = first.min(timestamps[start]);
            last = last.max(timestamps[end]);
        }
    }
    if passes.clouds {
        first = first.min(timestamps[12]).min(timestamps[16]);
        last = last.max(timestamps[13]).max(timestamps[17]);
    }
    if passes.water {
        first = first.min(timestamps[18]);
        last = last.max(timestamps[19]);
    }
    if passes.weather {
        first = first.min(timestamps[20]);
        last = last.max(timestamps[21]);
    }
    if passes.ambient_occlusion {
        first = first
            .min(timestamps[6])
            .min(timestamps[8])
            .min(timestamps[10]);
        last = last
            .max(timestamps[7])
            .max(timestamps[9])
            .max(timestamps[11]);
    }
    if passes.virtual_terrain {
        first = first.min(timestamps[24]).min(timestamps[26]);
        last = last.max(timestamps[25]).max(timestamps[27]);
    }
    let total_ms = last.checked_sub(first)? as f32 * timestamp_period / 1_000_000.0;
    if total_ms > 1_000.0 {
        return None;
    }
    Some(GpuTimingSample {
        frame_id: 0,
        total_ms,
        shadow_ms,
        shadow_cascade_ms,
        depth_prepass_ms,
        world_ms,
        water_ms,
        ambient_occlusion_ms,
        cloud_ms,
        weather_ms,
        ui_ms,
        virtual_terrain_snapshot_encode_ms,
        virtual_terrain_snapshot_validation_ms,
    })
}

impl GpuTimer {
    fn new(device: &Device, queue: &Queue) -> Option<Self> {
        let timestamp_period = queue.get_timestamp_period();
        if !timestamp_period.is_finite() || timestamp_period <= 0.0 {
            return None;
        }
        let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("frame GPU timestamps"),
            ty: wgpu::QueryType::Timestamp,
            count: GPU_QUERY_COUNT,
        });
        let resolve_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frame GPU timestamp resolve"),
            size: GPU_RESOLVE_BUFFER_BYTES,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = std::array::from_fn(|_| GpuTimingSlot {
            buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("frame GPU timestamp readback"),
                size: GPU_QUERY_BUFFER_BYTES,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            available: Arc::new(AtomicBool::new(true)),
        });
        Some(Self {
            query_set,
            resolve_buffer,
            readback,
            next_slot: 0,
            timestamp_period,
            state: Arc::new(Mutex::new(GpuTimingState::default())),
        })
    }

    fn begin_frame(&mut self, frame_id: u32, passes: GpuPassMask) -> Option<GpuTimingFrame> {
        for offset in 0..GPU_READBACK_SLOTS {
            let slot = (self.next_slot + offset) % GPU_READBACK_SLOTS;
            if self.readback[slot]
                .available
                .compare_exchange(true, false, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                self.next_slot = (slot + 1) % GPU_READBACK_SLOTS;
                return Some(GpuTimingFrame {
                    query_set: self.query_set.clone(),
                    slot,
                    frame_id,
                    passes,
                });
            }
        }
        None
    }

    fn resolve(&self, encoder: &mut wgpu::CommandEncoder, frame: &GpuTimingFrame) {
        encoder.resolve_query_set(
            &frame.query_set,
            0..GPU_QUERY_COUNT,
            &self.resolve_buffer,
            0,
        );
        encoder.copy_buffer_to_buffer(
            &self.resolve_buffer,
            0,
            &self.readback[frame.slot].buffer,
            0,
            GPU_QUERY_BUFFER_BYTES,
        );
    }

    fn schedule_readback(&self, encoder: &wgpu::CommandEncoder, frame: GpuTimingFrame) {
        let slot = &self.readback[frame.slot];
        let buffer = slot.buffer.clone();
        let callback_buffer = buffer.clone();
        let available = Arc::clone(&slot.available);
        let state = Arc::clone(&self.state);
        let period = self.timestamp_period;
        encoder.map_buffer_on_submit(&buffer, wgpu::MapMode::Read, .., move |result| {
            let sample = if result.is_ok() {
                let mut parsed = None;
                if let Ok(mapped) = callback_buffer.get_mapped_range(..) {
                    let mut timestamps = [0u64; GPU_QUERY_COUNT as usize];
                    for (timestamp, bytes) in timestamps.iter_mut().zip(mapped.chunks_exact(8)) {
                        let mut raw = [0u8; 8];
                        raw.copy_from_slice(bytes);
                        *timestamp = u64::from_le_bytes(raw);
                    }
                    drop(mapped);
                    parsed = parse_gpu_timestamps(&timestamps, period, frame.passes);
                }
                callback_buffer.unmap();
                parsed
            } else {
                None
            };
            if let Some(mut sample) = sample
                && let Ok(mut state) = state.lock()
            {
                sample.frame_id = frame.frame_id;
                state.latest = Some(sample);
                if state.history.len() == GPU_TIMING_HISTORY_CAPACITY {
                    state.history.pop_front();
                    state.dropped = state.dropped.saturating_add(1);
                }
                state.history.push_back(sample);
            }
            available.store(true, Ordering::Release);
        });
    }

    fn latest(&self) -> Option<GpuTimingSample> {
        self.state.lock().ok().and_then(|state| state.latest)
    }

    fn drain(&self) -> GpuTimingBatch {
        let Ok(mut state) = self.state.lock() else {
            return GpuTimingBatch::default();
        };
        GpuTimingBatch {
            samples: state.history.drain(..).collect(),
            dropped: std::mem::take(&mut state.dropped),
        }
    }

    fn cancel_frame(&self, frame: GpuTimingFrame) {
        self.readback[frame.slot]
            .available
            .store(true, Ordering::Release);
    }
}

struct DepthTarget {
    texture: Texture,
    view: TextureView,
    width: u32,
    height: u32,
}

impl DepthTarget {
    fn new(
        device: &Device,
        label: &'static str,
        width: u32,
        height: u32,
        usage: TextureUsages,
    ) -> Self {
        let width = width.max(1);
        let height = height.max(1);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            texture,
            view,
            width,
            height,
        }
    }

    fn world(device: &Device, width: u32, height: u32) -> Self {
        Self::new(device, "world depth", width, height, world_depth_usage())
    }

    fn opaque_snapshot(device: &Device, width: u32, height: u32) -> Self {
        Self::new(
            device,
            "opaque world depth",
            width,
            height,
            opaque_depth_usage(),
        )
    }

    const fn view(&self) -> &TextureView {
        &self.view
    }

    fn copy_to(&self, encoder: &mut wgpu::CommandEncoder, destination: &Self) {
        debug_assert_eq!(
            (self.width, self.height),
            (destination.width, destination.height)
        );
        encoder.copy_texture_to_texture(
            self.texture.as_image_copy(),
            destination.texture.as_image_copy(),
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
    }
}

fn world_depth_usage() -> TextureUsages {
    TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_SRC
}

fn opaque_depth_usage() -> TextureUsages {
    TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST
}

pub struct Renderer {
    surface: Surface<'static>,
    device: Device,
    queue: Queue,
    config: SurfaceConfiguration,
    sky_pipeline: RenderPipeline,
    depth_prepass_fast_pipeline: RenderPipeline,
    voxel_pipeline: RenderPipeline,
    voxel_flat_pipeline: RenderPipeline,
    voxel_ambient_occlusion_pipeline: RenderPipeline,
    voxel_ambient_occlusion_flat_pipeline: RenderPipeline,
    virtual_surface_depth_pipeline: RenderPipeline,
    virtual_surface_pipeline: RenderPipeline,
    virtual_surface_flat_pipeline: RenderPipeline,
    virtual_surface_ambient_occlusion_pipeline: RenderPipeline,
    virtual_surface_ambient_occlusion_flat_pipeline: RenderPipeline,
    virtual_triangle_depth_pipeline: RenderPipeline,
    virtual_triangle_pipeline: RenderPipeline,
    virtual_triangle_flat_pipeline: RenderPipeline,
    virtual_triangle_ambient_occlusion_pipeline: RenderPipeline,
    virtual_triangle_ambient_occlusion_flat_pipeline: RenderPipeline,
    virtual_triangle_diagnostic_pipeline: RenderPipeline,
    screenshot_diagnostic_pipeline: RenderPipeline,
    water_pipeline: RenderPipeline,
    virtual_surface_water_pipeline: RenderPipeline,
    virtual_triangle_water_pipeline: RenderPipeline,
    weather_pipeline: RenderPipeline,
    avatar_gpu: AvatarGpu,
    remote_avatars: Vec<RemoteAvatarPose>,
    water_scene_layout: wgpu::BindGroupLayout,
    water_scene_bind_group: BindGroup,
    shadow_gpu: ShadowGpu,
    shadow_direction: ShadowDirectionTracker,
    frame_buffer: Buffer,
    frame_bind_group: BindGroup,
    local_light_buffer: Buffer,
    material_detail: MaterialDetailGpu,
    chunks: BTreeMap<MeshKey, ChunkMesh>,
    water_chunks: BTreeMap<MeshKey, ChunkMesh>,
    virtual_terrain: VirtualTerrainHierarchy,
    virtual_terrain_gpu: VirtualTerrainGpuControl,
    virtual_terrain_mode: VirtualTerrainRenderMode,
    /// Last GPU-certified ownership cut. This is the only virtual-terrain cut that may be
    /// presented; it deliberately does not track the latest view-quality target.
    virtual_terrain_cut: Option<VirtualTerrainCut>,
    /// Safety and horizon proof committed with `virtual_terrain_cut` and the active handle bank.
    virtual_terrain_committed_envelope: Option<PresentationEnvelope>,
    /// Ephemeral quality/demand target selected from the latest view and resident directory.
    virtual_terrain_oracle_cut: Option<VirtualTerrainCut>,
    /// Immutable cut currently being encoded or awaiting GPU certification. Directory growth,
    /// cache arrivals, and a newer desired view may replace `virtual_terrain_oracle_cut`, but may
    /// not replace this transaction until it promotes or fails.
    virtual_terrain_publication: Option<VirtualTerrainPublication>,
    /// Geometry admitted for the next legal cut. This includes every complete replacement group
    /// and any already-resident balance dependencies selected while accumulating the microbatch.
    /// The fence survives oracle invalidation and an empty retention request; it is released only
    /// when a cut promotes or the transaction explicitly aborts.
    virtual_terrain_staging_frontier: BTreeSet<TerrainPageKey>,
    /// An encode failure releases the transaction immediately, but expanded candidate geometry is
    /// reclaimed at the next streaming boundary rather than while frame draw lists borrow it.
    virtual_terrain_publication_abort_pending: bool,
    /// Hysteretic screen-error scale selected by the compact-output capacity solver.
    virtual_terrain_error_scale: f64,
    virtual_terrain_headroom_frames: u16,
    /// Unmodified camera/error request used to cache the CPU selection decision.
    virtual_terrain_requested_view: Option<VirtualTerrainView>,
    /// Capacity-adjusted view used by the CPU oracle and handle-snapshot encoder.
    virtual_terrain_oracle_view: Option<VirtualTerrainView>,
    /// Exact discrete motion domain used by selection, publication, and presentation readiness.
    virtual_terrain_exact_surface_domain: Option<ExactSurfaceDomain>,
    /// Latest desired presentation proof. It may change while a frozen publication is in flight.
    virtual_terrain_desired_envelope: Option<PresentationEnvelope>,
    virtual_terrain_pages: BTreeMap<TerrainPageKey, VirtualTerrainGpuPage>,
    virtual_terrain_retired_published_pages: BTreeMap<TerrainPageKey, VirtualTerrainGpuPage>,
    virtual_terrain_heightfield_samples: BTreeMap<TerrainPageKey, CachedVirtualHeightfieldSamples>,
    virtual_terrain_arena: ArenaAllocator,
    virtual_terrain_arena_buffers: Vec<Buffer>,
    canonical_ready_chunks: HashSet<(i32, i32, i32)>,
    canonical_surface_ready_chunks: HashSet<(i32, i32, i32)>,
    enclosed_view_ready_chunks: HashSet<(i32, i32, i32)>,
    chunk_activations: ChunkActivations,
    local_light_candidates: BTreeMap<MeshKey, Vec<GpuLocalLight>>,
    arena: ArenaAllocator,
    arena_buffers: Vec<Buffer>,
    water_arena: ArenaAllocator,
    water_arena_buffers: Vec<Buffer>,
    depth: DepthTarget,
    opaque_depth: DepthTarget,
    ambient_occlusion_gpu: AmbientOcclusionGpu,
    volumetric_cloud_gpu: VolumetricCloudGpu,
    time: f32,
    diagnostics: RenderDiagnostics,
    gpu_timer: Option<GpuTimer>,
    target_voxel: Option<[i32; 3]>,
    target_volume: Option<EditVolume>,
    edit_shape: EditShape,
    options: RenderOptions,
    geometry_source_debug: bool,
    environment: OutdoorEnvironment,
    server_world_environment: WorldEnvironmentState,
    debug_environment_override: DebugEnvironmentOverride,
    reproduction_environment_override: Option<WorldEnvironmentState>,
    world_environment: WorldEnvironmentState,
    observer_world_xz_metres: [f64; 2],
    celestial_observation: CelestialObservation,
    atmosphere_sample: AtmosphereSample,
    surface_region: SurfaceRegion,
    daylight_phase: DaylightPhase,
    ui: MissionControlUi,
    ui_gpu: UiGpu,
    dpr: f32,
    log_error: fn(&str),
    ui_text_error_reported: bool,
    diagnostics_copy_requested: bool,
    screenshot_requested: bool,
    screenshot_world_identity: Option<ScreenshotWorldIdentity>,
    screenshot_reproduction_identity: Option<ScreenshotReproductionIdentity>,
    screenshot_streaming_manifest: ScreenshotStreamingManifest,
    screenshot_gpu_identity: ScreenshotGpuIdentity,
    screenshot_readback: Arc<Mutex<ScreenshotReadbackState>>,
    host_ui_action: Option<HostUiAction>,
    interior: InteriorEnvironment,
    interior_target: InteriorEnvironment,
    directional_light_occluded: bool,
    placement_inventory: PlacementInventory,
    runtime_config: RendererConfig,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostUiAction {
    SpectatorRequested(bool),
}

struct ShadowGpu {
    layout: wgpu::BindGroupLayout,
    _texture: Texture,
    sample_view: TextureView,
    sampler: wgpu::Sampler,
    layer_views: [TextureView; CASCADE_COUNT],
    uniform_buffers: [Buffer; CASCADE_COUNT],
    bind_groups: [BindGroup; CASCADE_COUNT],
    fixed_pipeline: RenderPipeline,
    virtual_surface_pipeline: RenderPipeline,
    virtual_triangle_pipeline: RenderPipeline,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RenderOptions {
    shadows: bool,
    ambient_occlusion: bool,
    screen_space_ambient_occlusion: bool,
    fog: bool,
    far_terrain: bool,
    water: bool,
    target_outline: bool,
    material_detail: bool,
    cave_headlamp: bool,
    local_lighting: bool,
}

#[derive(Clone, Copy, Debug)]
struct FrameState {
    options: RenderOptions,
    geometry_source_debug: bool,
    environment: OutdoorEnvironment,
    world_environment: WorldEnvironmentState,
    celestial_observation: CelestialObservation,
    interior: InteriorEnvironment,
    direct_light_visibility: f32,
}

impl From<RendererFeatureConfig> for RenderOptions {
    fn from(config: RendererFeatureConfig) -> Self {
        Self {
            shadows: config.cascaded_sun_shadows,
            ambient_occlusion: config.voxel_ambient_occlusion,
            screen_space_ambient_occlusion: config.screen_space_ambient_occlusion,
            fog: config.atmospheric_fog,
            far_terrain: config.far_terrain,
            water: config.water_surface,
            target_outline: config.target_outline,
            material_detail: config.material_surface_detail,
            cave_headlamp: config.cave_headlamp,
            local_lighting: config.voxel_emissive_lights,
        }
    }
}

fn validate_shadow_allocation(
    resolution: u32,
    max_texture_dimension_2d: u32,
) -> Result<(), String> {
    if resolution == 0 {
        return Err("shadow-map resolution must be greater than zero".to_owned());
    }
    if resolution > max_texture_dimension_2d {
        return Err(format!(
            "shadow-map resolution {resolution} exceeds the device limit {max_texture_dimension_2d}"
        ));
    }
    let allocation_bytes = u64::from(resolution)
        .checked_mul(u64::from(resolution))
        .and_then(|texels| texels.checked_mul(CASCADE_COUNT as u64))
        .and_then(|texels| texels.checked_mul(size_of::<f32>() as u64))
        .ok_or_else(|| "shadow-map allocation size overflowed".to_owned())?;
    if allocation_bytes > MAX_SHADOW_ALLOCATION_BYTES {
        return Err(format!(
            "shadow maps require {allocation_bytes} bytes, above the {}-byte safety budget",
            MAX_SHADOW_ALLOCATION_BYTES
        ));
    }
    Ok(())
}

impl ShadowGpu {
    fn new(
        device: &Device,
        virtual_geometry_layout: &wgpu::BindGroupLayout,
        camera: &CameraState,
        light_basis: DirectionalShadowBasis,
        config: DirectionalShadowConfig,
    ) -> Result<Self, String> {
        let cascades = build_directional_shadow_cascades(camera, 1.0, light_basis, config)
            .map_err(|error| format!("build initial shadow cascades: {error:?}"))?;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sun shadow cascade array"),
            size: wgpu::Extent3d {
                width: config.shadow_map_resolution,
                height: config.shadow_map_resolution,
                depth_or_array_layers: CASCADE_COUNT as u32,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let sample_view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("sun shadow sampling view"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            array_layer_count: Some(CASCADE_COUNT as u32),
            ..Default::default()
        });
        let layer_views = std::array::from_fn(|index| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("sun shadow cascade attachment"),
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_array_layer: index as u32,
                array_layer_count: Some(1),
                ..Default::default()
            })
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sun shadow comparison sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow caster frame layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let initial_uniforms: [ShadowFrameUniform; CASCADE_COUNT] =
            std::array::from_fn(|index| shadow_frame_uniform(&cascades, index, camera));
        let uniform_buffers = std::array::from_fn(|index| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("shadow caster frame uniform"),
                contents: bytemuck::bytes_of(&initial_uniforms[index]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            })
        });
        let bind_groups = std::array::from_fn(|index| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("shadow caster frame bind group"),
                layout: &layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffers[index].as_entire_binding(),
                }],
            })
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shadow caster pipeline layout"),
            bind_group_layouts: &[Some(&layout), Some(virtual_geometry_layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/shadow.wgsl"));
        let fixed_pipeline = shadow_caster_pipeline(
            device,
            "fixed shadow caster pipeline",
            &pipeline_layout,
            &shader,
        );
        let virtual_surface_pipeline = virtual_surface_shadow_caster_pipeline(
            device,
            "virtual terrain surface handle shadow caster pipeline",
            &pipeline_layout,
            &shader,
        );
        let virtual_triangle_pipeline = virtual_triangle_shadow_caster_pipeline(
            device,
            "virtual terrain triangle shadow caster pipeline",
            &pipeline_layout,
            &shader,
        );
        Ok(Self {
            layout,
            _texture: texture,
            sample_view,
            sampler,
            layer_views,
            uniform_buffers,
            bind_groups,
            fixed_pipeline,
            virtual_surface_pipeline,
            virtual_triangle_pipeline,
        })
    }

    fn write_cascades(
        &self,
        queue: &Queue,
        cascades: &DirectionalShadowCascades,
        camera: &CameraState,
    ) {
        for index in 0..CASCADE_COUNT {
            let uniform = shadow_frame_uniform(cascades, index, camera);
            queue.write_buffer(
                &self.uniform_buffers[index],
                0,
                bytemuck::bytes_of(&uniform),
            );
        }
    }
}

impl Renderer {
    pub async fn new(
        target: wgpu::SurfaceTarget<'static>,
        width: u32,
        height: u32,
        dpr: f32,
        log_error: fn(&str),
        runtime_config: RendererConfig,
    ) -> Result<Self, String> {
        if !runtime_config.view_distance_metres.is_finite()
            || runtime_config.view_distance_metres <= 0.0
        {
            return Err("renderer view distance must be finite and positive".to_owned());
        }
        let instance = Instance::new(InstanceDescriptor {
            backends: Backends::BROWSER_WEBGPU,
            ..InstanceDescriptor::new_without_display_handle()
        });
        let surface = instance
            .create_surface(target)
            .map_err(|error| format!("create_surface: {error:?}"))?;
        let adapter = instance
            .request_adapter(&RequestAdapterOptions {
                power_preference: PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
                apply_limit_buckets: false,
            })
            .await
            .map_err(|error| format!("request_adapter: {error:?}"))?;
        let adapter_info = adapter.get_info();
        let supported_features = adapter.features();
        let timestamp_queries = supported_features.contains(Features::TIMESTAMP_QUERY);
        let required_features = if timestamp_queries {
            Features::TIMESTAMP_QUERY
        } else {
            Features::empty()
        };
        let (device, queue) = adapter
            .request_device(&DeviceDescriptor {
                required_limits: wgpu::Limits::default(),
                required_features,
                ..Default::default()
            })
            .await
            .map_err(|error| format!("request_device: {error:?}"))?;
        let enabled_features = device.features();
        let screenshot_gpu_identity = ScreenshotGpuIdentity {
            name: adapter_info.name,
            vendor: adapter_info.vendor,
            device: adapter_info.device,
            device_type: format!("{:?}", adapter_info.device_type),
            device_pci_bus_id: adapter_info.device_pci_bus_id,
            driver: adapter_info.driver,
            driver_info: adapter_info.driver_info,
            backend: format!("{:?}", adapter_info.backend),
            subgroup_min_size: adapter_info.subgroup_min_size,
            subgroup_max_size: adapter_info.subgroup_max_size,
            supported_features: supported_features.bits().0,
            enabled_features: enabled_features.bits().0,
            limits: format!("{:?}", device.limits()),
        };
        validate_shadow_allocation(
            runtime_config.directional_shadows.shadow_map_resolution,
            device.limits().max_texture_dimension_2d,
        )?;
        device.on_uncaptured_error(Arc::new(move |error| {
            log_error(&format!("wgpu validation: {error}"));
        }));
        let caps = surface.get_capabilities(&adapter);
        let format = preferred_format(&caps.formats);
        let config = SurfaceConfiguration {
            usage: TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: width.max(1),
            height: height.max(1),
            present_mode: PresentMode::Fifo,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let options = RenderOptions::from(runtime_config.features);
        let gpu_timer = if timestamp_queries {
            GpuTimer::new(&device, &queue)
        } else {
            None
        };
        let atmosphere_sample = AtmosphereSample {
            humidity: 0.68,
            coldness: 0.32,
            aerosol: 0.08,
            cloudiness: 0.62,
            horizon_warmth: 0.30,
            haze: 0.38,
        };
        let surface_region = SurfaceRegion::VerdantForest;
        let world_environment = WorldEnvironmentState::default();
        let initial_camera = CameraState::default();
        let observer_world_xz_metres = [
            f64::from(initial_camera.position.x),
            f64::from(initial_camera.position.z),
        ];
        let celestial_observation = world_environment
            .celestial_observation(observer_world_xz_metres)
            .ok_or_else(|| "initial celestial observation is invalid".to_owned())?;
        let daylight_phase = DaylightPhase::for_solar_position(
            celestial_observation.sun_direction[1],
            celestial_observation.solar_hour_angle_radians,
        );
        let environment = OutdoorEnvironment::for_celestial(
            atmosphere_sample,
            celestial_observation,
            world_environment.weather(atmosphere_sample.coldness),
        );
        let shadow_direction = ShadowDirectionTracker::new(
            -environment.key_light_direction,
            runtime_config
                .directional_shadows
                .direction_update_threshold_radians,
        )
        .map_err(|error| format!("initialize retained shadow direction: {error:?}"))?;
        let virtual_terrain_capacity = VirtualTerrainCapacity::DEVELOPMENT_128_MIB;
        let virtual_terrain_gpu = VirtualTerrainGpuControl::new(&device, virtual_terrain_capacity)
            .map_err(|error| format!("virtual terrain GPU snapshots: {error:?}"))?;
        let shadow_gpu = ShadowGpu::new(
            &device,
            virtual_terrain_gpu.render_layout(),
            &initial_camera,
            shadow_direction.basis(),
            runtime_config.directional_shadows,
        )?;
        let material_detail = MaterialDetailGpu::new(&device, &queue);
        let shadow_cascades = directional_shadow_cascades(
            &config,
            &initial_camera,
            shadow_direction.basis(),
            runtime_config.directional_shadows,
        )?;
        let frame = frame_uniform(
            &config,
            &initial_camera,
            0.0,
            None,
            FrameState {
                options,
                geometry_source_debug: false,
                environment,
                world_environment,
                celestial_observation,
                interior: InteriorEnvironment::default(),
                direct_light_visibility: 1.0,
            },
            &shadow_cascades,
            runtime_config,
        );
        let frame_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("frame uniform"),
            contents: bytemuck::bytes_of(&frame),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let local_light_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("bounded local light uniform"),
            contents: bytemuck::bytes_of(&LocalLightUniform::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("frame layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(
                            size_of::<LocalLightUniform>() as u64
                        ),
                    },
                    count: None,
                },
            ],
        });
        let frame_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("frame bind group"),
            layout: &frame_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: frame_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&shadow_gpu.sample_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&shadow_gpu.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&material_detail.albedo_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(
                        &material_detail.normal_roughness_view,
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(&material_detail.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: local_light_buffer.as_entire_binding(),
                },
            ],
        });
        let depth = DepthTarget::world(&device, config.width, config.height);
        let opaque_depth = DepthTarget::opaque_snapshot(&device, config.width, config.height);
        let ambient_occlusion_gpu = AmbientOcclusionGpu::new(
            &device,
            &frame_layout,
            depth.view(),
            config.width,
            config.height,
        );
        let volumetric_cloud_gpu = VolumetricCloudGpu::new(
            &device,
            &queue,
            &frame_layout,
            SCENE_FORMAT,
            DEPTH_FORMAT,
            config.width,
            config.height,
            runtime_config.volumetric_clouds,
        );
        let avatar_gpu = AvatarGpu::new(
            &device,
            &frame_layout,
            &shadow_gpu.layout,
            SCENE_FORMAT,
            DEPTH_FORMAT,
        );
        let sky_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sky pipeline layout"),
            bind_group_layouts: &[Some(&frame_layout)],
            immediate_size: 0,
        });
        let water_scene_layout = water_scene_layout(&device);
        let world_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("world pipeline layout"),
                bind_group_layouts: &[
                    Some(&frame_layout),
                    None,
                    Some(ambient_occlusion_gpu.sample_layout()),
                ],
                immediate_size: 0,
            });
        let virtual_world_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("virtual terrain handle world pipeline layout"),
                bind_group_layouts: &[
                    Some(&frame_layout),
                    None,
                    Some(ambient_occlusion_gpu.sample_layout()),
                    Some(virtual_terrain_gpu.render_layout()),
                ],
                immediate_size: 0,
            });
        let water_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("water pipeline layout"),
                bind_group_layouts: &[Some(&frame_layout), Some(&water_scene_layout)],
                immediate_size: 0,
            });
        let virtual_water_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("virtual terrain handle water pipeline layout"),
                bind_group_layouts: &[
                    Some(&frame_layout),
                    Some(&water_scene_layout),
                    None,
                    Some(virtual_terrain_gpu.render_layout()),
                ],
                immediate_size: 0,
            });
        let sky_shader =
            crate::shader::frame_shader(&device, "sky shader", include_str!("shaders/sky.wgsl"));
        let sky_pipeline = pipeline(
            &device,
            "sky pipeline",
            &sky_pipeline_layout,
            &sky_shader,
            SCENE_FORMAT,
            &[],
            PipelineOptions {
                vertex_entry: "vs_main",
                fragment_entry: "fs_main",
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                fragment_constants: &[],
            },
        );
        let weather_pipeline_error_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let weather_shader = crate::shader::frame_shader(
            &device,
            "precipitation weather shader",
            include_str!("shaders/weather.wgsl"),
        );
        let weather_pipeline = pipeline(
            &device,
            "precipitation weather pipeline",
            &sky_pipeline_layout,
            &weather_shader,
            SCENE_FORMAT,
            &[],
            PipelineOptions {
                vertex_entry: "vs_main",
                fragment_entry: "fs_main",
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                fragment_constants: &[],
            },
        );
        if let Some(error) = weather_pipeline_error_scope.pop().await {
            return Err(format!("create precipitation weather pipeline: {error}"));
        }
        let voxel_shader = crate::shader::frame_pbr_shader(
            &device,
            "voxel shader",
            include_str!("shaders/voxels.wgsl"),
        );
        let depth_prepass_fast_pipeline = fragmentless_depth_pipeline(
            &device,
            "spatial AO depth pipeline",
            &sky_pipeline_layout,
            &voxel_shader,
        );
        let virtual_surface_depth_pipeline = virtual_surface_depth_pipeline(
            &device,
            "virtual terrain surface handle depth pipeline",
            &virtual_world_pipeline_layout,
            &voxel_shader,
        );
        let virtual_triangle_depth_pipeline = virtual_triangle_depth_pipeline(
            &device,
            "virtual terrain triangle depth pipeline",
            &virtual_world_pipeline_layout,
            &voxel_shader,
        );
        let voxel_pipeline = create_voxel_pipeline(
            &device,
            "voxel pipeline",
            &world_pipeline_layout,
            &voxel_shader,
            VoxelPipelineVariant::new(true, false),
        );
        let voxel_flat_pipeline = create_voxel_pipeline(
            &device,
            "flat voxel pipeline",
            &world_pipeline_layout,
            &voxel_shader,
            VoxelPipelineVariant::new(false, false),
        );
        let voxel_ambient_occlusion_pipeline = create_voxel_pipeline(
            &device,
            "spatial AO voxel pipeline",
            &world_pipeline_layout,
            &voxel_shader,
            VoxelPipelineVariant::new(true, true),
        );
        let voxel_ambient_occlusion_flat_pipeline = create_voxel_pipeline(
            &device,
            "flat spatial AO voxel pipeline",
            &world_pipeline_layout,
            &voxel_shader,
            VoxelPipelineVariant::new(false, true),
        );
        let virtual_surface_pipeline = create_virtual_surface_pipeline(
            &device,
            "virtual terrain surface handle pipeline",
            &virtual_world_pipeline_layout,
            &voxel_shader,
            true,
            false,
        );
        let virtual_surface_flat_pipeline = create_virtual_surface_pipeline(
            &device,
            "flat virtual terrain surface handle pipeline",
            &virtual_world_pipeline_layout,
            &voxel_shader,
            false,
            false,
        );
        let virtual_surface_ambient_occlusion_pipeline = create_virtual_surface_pipeline(
            &device,
            "spatial AO virtual terrain surface handle pipeline",
            &virtual_world_pipeline_layout,
            &voxel_shader,
            true,
            true,
        );
        let virtual_surface_ambient_occlusion_flat_pipeline = create_virtual_surface_pipeline(
            &device,
            "flat spatial AO virtual terrain surface handle pipeline",
            &virtual_world_pipeline_layout,
            &voxel_shader,
            false,
            true,
        );
        let virtual_triangle_pipeline = create_virtual_triangle_pipeline(
            &device,
            "virtual terrain triangle pipeline",
            &virtual_world_pipeline_layout,
            &voxel_shader,
            true,
            false,
        );
        let virtual_triangle_flat_pipeline = create_virtual_triangle_pipeline(
            &device,
            "flat virtual terrain triangle pipeline",
            &virtual_world_pipeline_layout,
            &voxel_shader,
            false,
            false,
        );
        let virtual_triangle_ambient_occlusion_pipeline = create_virtual_triangle_pipeline(
            &device,
            "spatial AO virtual terrain triangle pipeline",
            &virtual_world_pipeline_layout,
            &voxel_shader,
            true,
            true,
        );
        let virtual_triangle_ambient_occlusion_flat_pipeline = create_virtual_triangle_pipeline(
            &device,
            "flat spatial AO virtual terrain triangle pipeline",
            &virtual_world_pipeline_layout,
            &voxel_shader,
            false,
            true,
        );
        let virtual_triangle_diagnostic_pipeline = virtual_triangle_diagnostic_pipeline(
            &device,
            "virtual terrain triangle diagnostic pipeline",
            &world_pipeline_layout,
            &voxel_shader,
        );
        let screenshot_diagnostic_pipeline = create_voxel_diagnostic_pipeline(
            &device,
            "screenshot integer diagnostic pipeline",
            &world_pipeline_layout,
            &voxel_shader,
        );
        let water_pipeline = pipeline(
            &device,
            "water pipeline",
            &water_pipeline_layout,
            &voxel_shader,
            SCENE_FORMAT,
            &[Some(quad_layout())],
            PipelineOptions {
                vertex_entry: "vs_main_fixed",
                fragment_entry: "fs_water",
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Greater),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                fragment_constants: &[],
            },
        );
        let virtual_surface_water_pipeline = create_virtual_surface_water_pipeline(
            &device,
            "virtual terrain surface handle water pipeline",
            &virtual_water_pipeline_layout,
            &voxel_shader,
        );
        let virtual_triangle_water_pipeline = create_virtual_triangle_water_pipeline(
            &device,
            "virtual terrain triangle water pipeline",
            &virtual_water_pipeline_layout,
            &voxel_shader,
        );
        let ui_gpu = UiGpu::new(&device, format, config.width, config.height, dpr)?;
        let water_scene_bind_group =
            ui_gpu.water_scene_bind_group(&device, &water_scene_layout, opaque_depth.view());

        let placement_inventory = PlacementInventory::new();
        let virtual_terrain = VirtualTerrainHierarchy::new(virtual_terrain_capacity)
            .map_err(|error| format!("virtual terrain hierarchy: {error}"))?;
        let virtual_terrain_arena = ArenaAllocator::new_bounded(
            VIRTUAL_TERRAIN_GPU_ARENA_PAGE_BYTES,
            size_of::<GpuQuad>() as u32,
            VIRTUAL_TERRAIN_GPU_POOL_BYTES,
            VIRTUAL_TERRAIN_GPU_POOL_PAGES,
        )
        .ok_or_else(|| "virtual terrain GPU pool has invalid capacity".to_owned())?;
        let mut ui = MissionControlUi::new(runtime_config.mission_control);
        ui.set_diagnostic_sky_active(runtime_config.diagnostic_sky_color.is_some());
        ui.set_environment_status(daylight_phase.label(), surface_region_label(surface_region));
        ui.set_world_clock(
            celestial_observation.local_solar_day_fraction as f32,
            world_environment
                .weather(atmosphere_sample.coldness)
                .kind
                .label(),
            environment.precipitation,
            environment.cloud_coverage,
            world_environment.cloud_velocity_metres_per_second,
            world_environment.weather_revision,
        );
        sync_inventory_ui(&mut ui, &placement_inventory);
        Ok(Self {
            surface,
            device,
            queue,
            config,
            sky_pipeline,
            depth_prepass_fast_pipeline,
            voxel_pipeline,
            voxel_flat_pipeline,
            voxel_ambient_occlusion_pipeline,
            voxel_ambient_occlusion_flat_pipeline,
            virtual_surface_depth_pipeline,
            virtual_surface_pipeline,
            virtual_surface_flat_pipeline,
            virtual_surface_ambient_occlusion_pipeline,
            virtual_surface_ambient_occlusion_flat_pipeline,
            virtual_triangle_depth_pipeline,
            virtual_triangle_pipeline,
            virtual_triangle_flat_pipeline,
            virtual_triangle_ambient_occlusion_pipeline,
            virtual_triangle_ambient_occlusion_flat_pipeline,
            virtual_triangle_diagnostic_pipeline,
            screenshot_diagnostic_pipeline,
            water_pipeline,
            virtual_surface_water_pipeline,
            virtual_triangle_water_pipeline,
            weather_pipeline,
            avatar_gpu,
            remote_avatars: Vec::new(),
            water_scene_layout,
            water_scene_bind_group,
            shadow_gpu,
            shadow_direction,
            frame_buffer,
            frame_bind_group,
            local_light_buffer,
            material_detail,
            chunks: BTreeMap::new(),
            water_chunks: BTreeMap::new(),
            virtual_terrain,
            virtual_terrain_gpu,
            virtual_terrain_mode: VirtualTerrainRenderMode::Disabled,
            virtual_terrain_cut: None,
            virtual_terrain_committed_envelope: None,
            virtual_terrain_oracle_cut: None,
            virtual_terrain_publication: None,
            virtual_terrain_staging_frontier: BTreeSet::new(),
            virtual_terrain_publication_abort_pending: false,
            virtual_terrain_error_scale: 1.0,
            virtual_terrain_headroom_frames: 0,
            virtual_terrain_requested_view: None,
            virtual_terrain_oracle_view: None,
            virtual_terrain_exact_surface_domain: None,
            virtual_terrain_desired_envelope: None,
            virtual_terrain_pages: BTreeMap::new(),
            virtual_terrain_retired_published_pages: BTreeMap::new(),
            virtual_terrain_heightfield_samples: BTreeMap::new(),
            virtual_terrain_arena,
            virtual_terrain_arena_buffers: Vec::new(),
            canonical_ready_chunks: HashSet::new(),
            canonical_surface_ready_chunks: HashSet::new(),
            enclosed_view_ready_chunks: HashSet::new(),
            chunk_activations: ChunkActivations::default(),
            local_light_candidates: BTreeMap::new(),
            arena: ArenaAllocator::new(ARENA_PAGE_BYTES, size_of::<GpuQuad>() as u32),
            arena_buffers: Vec::new(),
            water_arena: ArenaAllocator::new(ARENA_PAGE_BYTES, size_of::<GpuQuad>() as u32),
            water_arena_buffers: Vec::new(),
            depth,
            opaque_depth,
            ambient_occlusion_gpu,
            volumetric_cloud_gpu,
            time: 0.0,
            diagnostics: RenderDiagnostics::default(),
            gpu_timer,
            target_voxel: None,
            target_volume: None,
            edit_shape: EditShape::Sphere,
            options,
            geometry_source_debug: false,
            environment,
            server_world_environment: world_environment,
            debug_environment_override: DebugEnvironmentOverride::default(),
            reproduction_environment_override: None,
            world_environment,
            observer_world_xz_metres,
            celestial_observation,
            atmosphere_sample,
            surface_region,
            daylight_phase,
            ui,
            ui_gpu,
            dpr: valid_dpr(dpr),
            log_error,
            ui_text_error_reported: false,
            diagnostics_copy_requested: false,
            screenshot_requested: false,
            screenshot_world_identity: None,
            screenshot_reproduction_identity: None,
            screenshot_streaming_manifest: ScreenshotStreamingManifest::default(),
            screenshot_gpu_identity,
            screenshot_readback: Arc::new(Mutex::new(ScreenshotReadbackState::default())),
            host_ui_action: None,
            interior: InteriorEnvironment::default(),
            interior_target: InteriorEnvironment::default(),
            directional_light_occluded: false,
            placement_inventory,
            runtime_config,
        })
    }

    pub fn resize(&mut self, width: u32, height: u32, dpr: f32) {
        if width == 0 || height == 0 {
            return;
        }
        let dpr = valid_dpr(dpr);
        let (size_changed, dpr_changed) = resize_changes(
            self.config.width,
            self.config.height,
            self.dpr,
            width,
            height,
            dpr,
        );
        if !size_changed && !dpr_changed {
            return;
        }
        if size_changed {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&self.device, &self.config);
            self.depth = DepthTarget::world(&self.device, width, height);
            self.opaque_depth = DepthTarget::opaque_snapshot(&self.device, width, height);
            self.ambient_occlusion_gpu
                .resize(&self.device, self.depth.view(), width, height);
            self.volumetric_cloud_gpu
                .resize(&self.device, width, height);
        }
        self.dpr = dpr;
        if self
            .ui_gpu
            .resize(&self.device, &self.queue, width, height, self.dpr)
        {
            self.water_scene_bind_group = self.ui_gpu.water_scene_bind_group(
                &self.device,
                &self.water_scene_layout,
                self.opaque_depth.view(),
            );
        }
    }

    pub fn quad_count(&self) -> u32 {
        self.chunks
            .values()
            .chain(self.water_chunks.values())
            .map(|chunk| chunk.quad_count)
            .sum()
    }

    pub const fn diagnostics(&self) -> RenderDiagnostics {
        self.diagnostics
    }

    pub fn drain_gpu_timings(&mut self) -> GpuTimingBatch {
        self.gpu_timer
            .as_ref()
            .map_or_else(GpuTimingBatch::default, GpuTimer::drain)
    }

    pub fn set_remote_avatars(&mut self, avatars: &[RemoteAvatarPose]) {
        self.remote_avatars.clear();
        self.remote_avatars.extend_from_slice(avatars);
    }

    pub fn set_dig_target(&mut self, target: Option<([i32; 3], EditVolume)>) {
        self.target_voxel = target.map(|(hit, _)| hit);
        self.target_volume = target.map(|(_, volume)| volume);
    }

    pub const fn edit_shape(&self) -> EditShape {
        self.edit_shape
    }

    pub fn cycle_edit_shape(&mut self) -> EditShape {
        self.edit_shape = self.edit_shape.next();
        self.ui.set_edit_shape(self.edit_shape);
        self.edit_shape
    }

    pub fn set_atmosphere(&mut self, sample: AtmosphereSample, region: SurfaceRegion) {
        if self.atmosphere_sample == sample && self.surface_region == region {
            return;
        }
        self.atmosphere_sample = sample;
        self.surface_region = region;
    }

    pub fn set_world_environment(&mut self, state: WorldEnvironmentState) {
        self.server_world_environment = state.sanitized();
        self.world_environment = self.effective_environment_state();
    }

    pub fn set_reproduction_environment(&mut self, state: Option<WorldEnvironmentState>) -> bool {
        if state.is_some_and(|state| state != state.sanitized()) {
            return false;
        }
        self.reproduction_environment_override = state;
        self.refresh_effective_environment()
    }

    fn effective_environment_state(&self) -> WorldEnvironmentState {
        self.reproduction_environment_override.unwrap_or_else(|| {
            self.debug_environment_override
                .apply(self.server_world_environment)
        })
    }

    fn refresh_effective_environment(&mut self) -> bool {
        let state = self.effective_environment_state();
        self.world_environment = state;
        let Some(celestial_observation) =
            state.celestial_observation(self.observer_world_xz_metres)
        else {
            return false;
        };
        self.celestial_observation = celestial_observation;
        self.daylight_phase = DaylightPhase::for_solar_position(
            self.celestial_observation.sun_direction[1],
            self.celestial_observation.solar_hour_angle_radians,
        );
        let weather = state.weather(self.atmosphere_sample.coldness);
        self.environment = OutdoorEnvironment::for_celestial(
            self.atmosphere_sample,
            self.celestial_observation,
            weather,
        );
        self.ui.set_environment_status(
            self.daylight_phase.label(),
            surface_region_label(self.surface_region),
        );
        self.ui.set_world_clock(
            self.celestial_observation.local_solar_day_fraction as f32,
            weather.kind.label(),
            self.environment.precipitation,
            self.environment.cloud_coverage,
            state.cloud_velocity_metres_per_second,
            state.weather_revision,
        );
        true
    }

    pub fn set_route_status(&mut self, chapter_label: &'static str, progress_percent: u8) {
        self.ui.set_route_status(chapter_label, progress_percent);
    }

    pub fn set_enclosure(&mut self, sample: EnclosureSample, directional_light_occluded: bool) {
        self.interior_target = InteriorEnvironment::for_enclosure(sample);
        self.directional_light_occluded = directional_light_occluded;
    }

    /// Current surface-to-key-light direction used by the host's resident-voxel visibility ray.
    pub fn key_light_direction(&self) -> glam::Vec3 {
        self.environment.key_light_direction
    }

    pub fn set_chunk_activation(
        &mut self,
        coord: ChunkCoord,
        reason: ChunkActivationReason,
        active: bool,
    ) {
        let key = (0, coord.x, coord.y, coord.z);
        let activation_mask = self.chunk_activations.set(key, reason, active);
        for chunks in [&mut self.chunks, &mut self.water_chunks] {
            let Some(chunk) = chunks.get_mut(&key) else {
                continue;
            };
            chunk.activation_mask = activation_mask;
        }
    }

    /// Atomically replaces the exact-volume working set and independently complete surface
    /// columns. These are exact leaves of the virtual terrain hierarchy, never a second LOD owner.
    pub fn set_canonical_cut_ready_chunks(
        &mut self,
        canonical_chunks: impl IntoIterator<Item = (i32, i32, i32)>,
        surface_chunks: impl IntoIterator<Item = (i32, i32, i32)>,
    ) {
        let canonical_replacement = canonical_chunks.into_iter().collect::<HashSet<_>>();
        let surface_replacement = surface_chunks.into_iter().collect::<HashSet<_>>();
        self.canonical_ready_chunks = canonical_replacement;
        self.canonical_surface_ready_chunks = surface_replacement;
    }

    /// Whether an exact-volume chunk belongs to the current renderable working set.
    pub fn exact_volume_chunk_presented(&self, coord: ChunkCoord) -> bool {
        let coord = (coord.x, coord.y, coord.z);
        self.canonical_ready_chunks.contains(&coord)
            || self.canonical_surface_ready_chunks.contains(&coord)
            || self.enclosed_view_ready_chunks.contains(&coord)
    }

    /// Whether an edited canonical chunk is represented by either remaining exact-volume
    /// ownership or the exact level-0 page in the published virtual hierarchy.
    pub fn edited_chunk_presented(&self, coord: ChunkCoord) -> bool {
        if self.exact_volume_chunk_presented(coord) {
            return true;
        }
        if self.virtual_terrain_mode != VirtualTerrainRenderMode::Visible {
            return false;
        }
        let key = TerrainPageKey::surface(0, coord.x, coord.z);
        if !self
            .virtual_terrain_cut
            .as_ref()
            .is_some_and(|cut| cut.selected_pages.contains(&key))
        {
            return false;
        }
        let Some(page) = self.virtual_terrain.resident_page(key) else {
            return false;
        };
        let chunk_minimum_y = coord.world_origin()[1];
        let chunk_maximum_y = chunk_minimum_y.saturating_add(CHUNK_EDGE as i32);
        page.topology == voxels_world::TerrainTopologyClass::Volumetric
            && page.errors == voxels_world::TerrainErrorBounds::EXACT
            && matches!(
                page.representation,
                TerrainPageRepresentation::SurfaceCluster(_)
            )
            && page.bounds.min.y < chunk_maximum_y
            && chunk_minimum_y < page.bounds.max.y
    }

    /// Replaces the exact underground chunks selected through visible tunnel apertures.
    ///
    /// These chunks supplement the height-surface hierarchy in three dimensions. They deliberately
    /// do not claim the whole X/Z column, so the far terrain surface remains selected above them.
    pub fn set_enclosed_view_ready_chunks(
        &mut self,
        chunks: impl IntoIterator<Item = (i32, i32, i32)>,
    ) {
        self.enclosed_view_ready_chunks = chunks.into_iter().collect();
    }

    /// Replaces the conservative terminators for visible exact-volume streaming frontiers.
    ///
    /// The mesh is one greedily merged face mask, independent of the canonical/surface LOD cut.
    /// It disappears atomically as soon as the shell stops reporting that neighbor as unknown.
    pub fn set_exact_volume_frontier_faces(
        &mut self,
        frontiers: &[ExactVolumeFrontierFace],
    ) -> bool {
        let mut quads_by_root = BTreeMap::<TerrainPageKey, Vec<GpuQuad>>::new();
        for frontier in frontiers {
            let Some(root) = (TerrainPageKey {
                level: 0,
                coord: [frontier.chunk.x, frontier.chunk.y, frontier.chunk.z],
            })
            .ancestor_at(TERRAIN_REGION_ROOT_LEVEL) else {
                return false;
            };
            quads_by_root
                .entry(root)
                .or_default()
                .extend(frontier_face_gpu_quads(frontier));
        }
        let mut gpu_quads = Vec::new();
        let mut slices = Vec::new();
        let quad_bytes = size_of::<GpuQuad>() as u32;
        for quads in quads_by_root.into_values() {
            if quads.is_empty() {
                continue;
            }
            let Some((bounds_min, bounds_max)) = gpu_quad_bounds(&quads) else {
                return false;
            };
            let start = gpu_quads.len() as u32;
            gpu_quads.extend(quads);
            let end = gpu_quads.len() as u32;
            slices.push(MeshSlice {
                relative_offset: start * quad_bytes,
                size: (end - start) * quad_bytes,
                quad_count: end - start,
                bounds_min,
                bounds_max,
                render_layer: RenderLayer::Opaque,
            });
        }
        if gpu_quads.is_empty() {
            let existed = self.chunks.contains_key(&EXACT_VOLUME_FRONTIER_MESH_KEY);
            self.remove_opaque_mesh(EXACT_VOLUME_FRONTIER_MESH_KEY);
            return existed;
        }
        if gpu_quads_match_resident(self.chunks.get(&EXACT_VOLUME_FRONTIER_MESH_KEY), &gpu_quads)
            && mesh_slices_match_resident(
                self.chunks.get(&EXACT_VOLUME_FRONTIER_MESH_KEY),
                &slices,
                gpu_quads.len(),
            )
        {
            return false;
        }
        let Some(prepared) =
            self.prepare_mesh_sliced(EXACT_VOLUME_FRONTIER_MESH_KEY, &gpu_quads, slices)
        else {
            return false;
        };
        commit_prepared_mesh(
            &mut self.arena,
            &mut self.chunks,
            EXACT_VOLUME_FRONTIER_MESH_KEY,
            Some(prepared),
        );
        true
    }

    pub fn enclosed_view_chunk_owned(&self, coord: ChunkCoord) -> bool {
        self.enclosed_view_ready_chunks
            .contains(&(coord.x, coord.y, coord.z))
    }

    /// Overrides the atmospheric background for deterministic geometry-coverage diagnostics.
    ///
    /// This is runtime-mutable so automation can measure ordinary weather first, then suppress it
    /// for an unambiguous sky-leak capture.
    pub fn set_diagnostic_sky_color(&mut self, color: Option<[f32; 3]>) {
        self.runtime_config.diagnostic_sky_color =
            color.map(|value| value.map(|channel| channel.clamp(0.0, 1.0)));
        self.ui
            .set_diagnostic_sky_active(self.runtime_config.diagnostic_sky_color.is_some());
    }

    /// Colors visible geometry by its actual resident page representation and hierarchy depth.
    pub fn set_geometry_source_debug(&mut self, active: bool) {
        self.geometry_source_debug = active;
        self.ui.set_geometry_sources_active(active);
    }

    /// Selects the material-detail pipeline for deterministic profiling without adding a
    /// developer-only control to the player-facing World Lab.
    pub fn set_material_detail_enabled(&mut self, enabled: bool) {
        self.options.material_detail = enabled;
    }

    pub const fn ui_open(&self) -> bool {
        self.ui.open()
    }

    pub const fn target_voxel(&self) -> Option<[i32; 3]> {
        self.target_voxel
    }

    pub const fn placement_material(&self) -> Option<Material> {
        self.placement_inventory.selected()
    }

    /// Replaces the complete server-authored inventory snapshot. Selection follows the first
    /// stocked material only when the current material has become unavailable.
    pub fn set_inventory_counts(&mut self, counts: [u64; Material::ALL.len()]) {
        self.placement_inventory.set_counts(counts);
        sync_inventory_ui(&mut self.ui, &self.placement_inventory);
    }

    /// Cycles in either direction, skipping every material whose authoritative count is zero.
    pub fn cycle_placement_material(&mut self, direction: i32) -> bool {
        let changed = self.placement_inventory.cycle(direction);
        if changed {
            sync_inventory_ui(&mut self.ui, &self.placement_inventory);
        }
        changed
    }

    /// Selects one of the ten currently visible wheel slots (`1` through `0`).
    pub fn select_placement_slot(&mut self, slot: usize) -> bool {
        let changed = self.placement_inventory.select_visible_slot(slot);
        if changed {
            sync_inventory_ui(&mut self.ui, &self.placement_inventory);
        }
        changed
    }

    pub fn show_gameplay_toast(&mut self, message: impl Into<String>) {
        self.ui.show_gameplay_toast(message);
    }

    pub fn take_diagnostics_copy(&mut self) -> Option<String> {
        std::mem::take(&mut self.diagnostics_copy_requested).then(|| self.ui.diagnostics_report())
    }

    pub fn report_diagnostics_copy_result(&mut self, copied: bool) {
        self.ui.show_gameplay_toast(if copied {
            "WORLD LAB COPIED"
        } else {
            "COULD NOT COPY WORLD LAB"
        });
    }

    pub fn screenshot_pending(&self) -> bool {
        self.screenshot_requested
            || self
                .screenshot_readback
                .lock()
                .is_ok_and(|state| state.in_flight || state.completed.is_some())
    }

    pub fn request_screenshot(&mut self) -> bool {
        if self.screenshot_pending() {
            return false;
        }
        self.screenshot_requested = true;
        true
    }

    pub fn set_screenshot_world_manifest(&mut self, manifest: &WorldManifest) {
        self.screenshot_world_identity = Some(ScreenshotWorldIdentity {
            world_id: hex_bytes(manifest.world_id.as_bytes()),
            source_identity_hash: manifest.source_identity_hash().to_string(),
            source_kind: manifest.source.source_kind as u8,
            seed: manifest.seed,
            world_schema_version: manifest.world_schema_version,
            material_schema_version: manifest.material_schema_version,
        });
    }

    pub fn set_screenshot_reproduction_identity(
        &mut self,
        identity: ScreenshotReproductionIdentity,
    ) {
        self.screenshot_reproduction_identity = Some(identity);
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the reproduction contract mirrors independently validated capture fields"
    )]
    pub fn validate_screenshot_reproduction_contract(
        &self,
        identity: &ScreenshotReproductionIdentity,
        world_id: &str,
        source_identity_hash: &str,
        seed: u64,
        pixel_width: u32,
        pixel_height: u32,
        device_pixel_ratio: f32,
        vertical_fov_radians: f32,
        near_plane_metres: f32,
        far_plane_metres: f32,
        features: ScreenshotFeatureState,
    ) -> Result<(), String> {
        if self.screenshot_reproduction_identity.as_ref() != Some(identity) {
            return Err(
                "capture build, protocol, or client configuration does not match".to_owned(),
            );
        }
        let world_matches = self
            .screenshot_world_identity
            .as_ref()
            .is_some_and(|world| {
                world.world_id == world_id
                    && world.source_identity_hash == source_identity_hash
                    && world.seed == seed
            });
        if !world_matches {
            return Err("capture world identity does not match the connected world".to_owned());
        }
        if self.config.width != pixel_width
            || self.config.height != pixel_height
            || self.dpr != valid_dpr(device_pixel_ratio)
        {
            return Err(format!(
                "capture viewport is {pixel_width}x{pixel_height} at DPR {device_pixel_ratio}, current viewport is {}x{} at DPR {}",
                self.config.width, self.config.height, self.dpr
            ));
        }
        let expected_fov = 68.0_f32.to_radians();
        if (vertical_fov_radians - expected_fov).abs() > 1.0e-6
            || (near_plane_metres - 0.05).abs() > 1.0e-6
            || (far_plane_metres - self.runtime_config.view_distance_metres).abs() > 1.0e-3
        {
            return Err("capture projection does not match the active renderer".to_owned());
        }
        let active_features = ScreenshotFeatureState {
            shadows: self.options.shadows,
            voxel_ambient_occlusion: self.options.ambient_occlusion,
            screen_space_ambient_occlusion: self.options.screen_space_ambient_occlusion,
            fog: self.options.fog,
            far_terrain: self.options.far_terrain,
            water: self.options.water,
            target_outline: self.options.target_outline,
            cave_headlamp: self.options.cave_headlamp,
            local_lighting: self.options.local_lighting,
        };
        if features != active_features {
            return Err("capture renderer features do not match the active client".to_owned());
        }
        Ok(())
    }

    pub fn set_reproduction_render_state(&mut self, state: ScreenshotMutableRenderState) -> bool {
        if state.diagnostic_sky_color.is_some_and(|color| {
            color
                .into_iter()
                .any(|channel| !channel.is_finite() || !(0.0..=1.0).contains(&channel))
        }) {
            return false;
        }
        _ = self.ui.set_open(state.world_lab_open);
        self.set_diagnostic_sky_color(state.diagnostic_sky_color);
        self.set_geometry_source_debug(state.geometry_source_debug);
        self.set_material_detail_enabled(state.material_detail);
        true
    }

    pub fn set_screenshot_streaming_manifest(&mut self, manifest: ScreenshotStreamingManifest) {
        self.screenshot_streaming_manifest = manifest;
    }

    pub fn take_screenshot_capture(&mut self) -> Option<ScreenshotCapture> {
        self.screenshot_readback
            .lock()
            .ok()
            .and_then(|mut state| state.completed.take())
    }

    pub fn report_screenshot_result(&mut self, saved: bool) {
        self.ui.show_gameplay_toast(if saved {
            "SCREENSHOT DOWNLOADED"
        } else {
            "COULD NOT SAVE SCREENSHOT"
        });
    }

    pub fn set_reduced_motion(&mut self, reduced_motion: bool) {
        self.ui.set_reduced_motion(reduced_motion);
    }

    pub fn handle_ui_key(&mut self, code: u8, pressed: bool, repeat: bool) -> bool {
        let key = if code == 8 { UiKey::F3 } else { UiKey::Other };
        let action = self.ui.handle_key(key, pressed, repeat);
        self.apply_ui_action(action);
        self.ui.open()
    }

    pub fn handle_ui_pointer_move(&mut self, css_x: f32, css_y: f32) -> bool {
        let viewport = self.ui_viewport();
        self.ui
            .pointer_move_device([css_x * self.dpr, css_y * self.dpr], viewport)
    }

    pub fn handle_ui_pointer_down(&mut self, css_x: f32, css_y: f32) -> bool {
        let viewport = self.ui_viewport();
        let point = [css_x * self.dpr, css_y * self.dpr];
        let action = self.ui.activate_device(point, viewport);
        self.apply_ui_action(action);
        self.ui.open()
    }

    pub fn inventory_wheel_contains(&self, css_x: f32, css_y: f32) -> bool {
        self.ui
            .inventory_contains_css([css_x, css_y], self.ui_viewport())
    }

    pub fn edit_shape_control_contains(&self, css_x: f32, css_y: f32) -> bool {
        self.ui
            .edit_shape_contains_css([css_x, css_y], self.ui_viewport())
    }

    fn ui_viewport(&self) -> Viewport {
        Viewport::new(
            self.config.width as f32,
            self.config.height as f32,
            self.dpr,
        )
    }

    fn apply_ui_action(&mut self, action: UiAction) {
        match action {
            UiAction::EditShapeChanged(shape) => {
                self.edit_shape = shape;
                self.ui.set_edit_shape(shape);
            }
            UiAction::CopyDiagnostics => {
                self.diagnostics_copy_requested = true;
            }
            UiAction::DiagnosticSkyChanged(active) => {
                self.set_diagnostic_sky_color(active.then_some([1.0, 0.0, 1.0]));
            }
            UiAction::GeometrySourcesChanged(active) => {
                self.set_geometry_source_debug(active);
            }
            UiAction::TakeScreenshot => {
                self.request_screenshot();
            }
            UiAction::TimeChanged(control) => {
                self.debug_environment_override.day_fraction = control.day_fraction();
                _ = self.refresh_effective_environment();
            }
            UiAction::WeatherChanged(control) => {
                self.debug_environment_override.weather_fraction = control
                    .preset()
                    .map(|preset| preset.anchor_weather_fraction());
                _ = self.refresh_effective_environment();
            }
            UiAction::SpectatorRequested(active) => {
                self.host_ui_action = Some(HostUiAction::SpectatorRequested(active));
            }
            UiAction::None | UiAction::PanelOpenChanged(_) => {}
        }
    }

    fn screenshot_reproduction_metadata(&self, frame_id: u32, camera: &CameraState) -> String {
        let world = self.screenshot_world_identity.as_ref().map_or_else(
            || "null".to_owned(),
            |world| {
                format!(
                    concat!(
                        r#"{{"worldId":"{}","sourceIdentityHash":"{}","sourceKind":{},"seed":"{}","#,
                        r#""worldSchemaVersion":{},"materialSchemaVersion":{}}}"#
                    ),
                    world.world_id,
                    world.source_identity_hash,
                    world.source_kind,
                    world.seed,
                    world.world_schema_version,
                    world.material_schema_version,
                )
            },
        );
        let diagnostic_sky = json_optional_vec3(self.runtime_config.diagnostic_sky_color);
        let debug_day_fraction = json_optional_f32(self.debug_environment_override.day_fraction);
        let debug_weather_fraction =
            json_optional_f32(self.debug_environment_override.weather_fraction);
        let locomotion = match camera.locomotion() {
            voxels_core::LocomotionMode::Walking => "walking",
            voxels_core::LocomotionMode::Gliding => "gliding",
            voxels_core::LocomotionMode::Spectator => "spectator",
        };
        let fluid = camera.fluid_state();
        let environment = self.world_environment;
        let celestial = self.celestial_observation;
        let options = self.options;
        let vertical_fov_radians = 68.0_f32.to_radians();
        let runtime_identity =
            screenshot_runtime_identity_json(self.screenshot_reproduction_identity.as_ref());
        let gpu_identity = screenshot_gpu_identity_json(&self.screenshot_gpu_identity);
        let streaming_manifest =
            screenshot_streaming_manifest_json(&self.screenshot_streaming_manifest);
        let gpu_virtual_feedback = self.virtual_terrain_gpu.latest_feedback();
        let virtual_terrain_manifest = screenshot_virtual_terrain_manifest_json(
            self.virtual_terrain_mode,
            &self.virtual_terrain_pages,
            &self.virtual_terrain_retired_published_pages,
            (self.virtual_terrain_mode == VirtualTerrainRenderMode::Visible)
                .then_some(self.virtual_terrain_cut.as_ref())
                .flatten(),
            self.virtual_terrain_oracle_cut.as_ref(),
            self.virtual_terrain_exact_surface_domain.as_ref(),
            gpu_virtual_feedback.as_ref(),
        );
        let published_cut = (self.virtual_terrain_mode == VirtualTerrainRenderMode::Visible)
            .then_some(self.virtual_terrain_cut.as_ref())
            .flatten();
        let cut_manifest = format!(
            r#"{{"kind":"virtualTerrain","cut":{}}}"#,
            screenshot_virtual_cut_json(published_cut),
        );
        let cut_fingerprint = published_cut.map_or(0, |cut| cut.fingerprint);
        let inverse_view_projection = view_projection(
            &self.config,
            camera,
            self.runtime_config.view_distance_metres,
        )
        .inverse()
        .to_cols_array();
        let representation_kinds = r#"{"canonicalExact":1,"steppedSurfaceResidual":2,"sparseVoxelBrick":3,"surfaceCluster":4,"triangleCluster":5,"heightfieldGrid":6,"exactVolumeFrontier":8}"#;
        let attachment_manifest = format!(
            concat!(
                r#"{{"terrainPixelOwnership":{{"chunkType":"vpDI","#,
                r#""schema":"voxels.terrain-pixel-ownership.v1","compression":"deflate","#,
                r#""populated":{},"#,
                r#""format":"u32x5","byteOrder":"little-endian","rowOrder":"top-down","#,
                r#""channels":["ownerIdHashLow","ownerIdHashHigh","primitiveFaceHash","packedRepresentationDepthFaceMaterial","reverseZDepthF32Bits"],"#,
                r#""backgroundOwnerId":["0","0"],"ownerHash":{{"algorithm":"fnv1a32+jenkins-oaat32","#,
                r#""words":["representationKind","hierarchyDepth","pageX","pageY","pageZ"],"#,
                r#""representationKind":{}}},"#,
                r#""descriptorBits":{{"representationSource":[0,4],"hierarchyDepth":[4,4],"face":[8,3],"material":[11,16]}},"#,
                r#""worldPositionReconstruction":{{"pixelCenter":true,"depthConvention":"reverse-z-webgpu","#,
                r#""inverseViewProjectionColumns":{:?}}}}}}}"#
            ),
            self.geometry_source_debug, representation_kinds, inverse_view_projection,
        );
        format!(
            concat!(
                r#"{{"schema":"voxels.reproduction.v2","frameSequence":{},"runtime":{},"gpu":{},"image":{{"#,
                r#""pixelWidth":{},"pixelHeight":{},"cssWidth":{},"cssHeight":{},"devicePixelRatio":{}}},"#,
                r#""camera":{{"eyeMetres":{:?},"velocityMetresPerSecond":{:?},"yawRadians":{},"pitchRadians":{},"headingDegrees":{},"verticalFovRadians":{},"nearPlaneMetres":0.05,"farPlaneMetres":{},"grounded":{},"locomotion":"{}","fluid":{{"immersion":{},"eyeDepthMetres":{},"signedEyeDepthMetres":{},"surfaceYMetres":{},"surfaceKnown":{},"eyesSubmerged":{},"swimming":{}}}}},"#,
                r#""world":{},"environment":{{"serverTimeSeconds":{},"worldDays":{},"dayFraction":{},"yearFraction":{},"moonOrbitFraction":{},"twinklePhase":{},"planetCircumferenceMetres":{},"axialTiltRadians":{},"moonOrbitInclinationRadians":{},"celestialSeed":"{}","celestialRevision":"{}","weatherFraction":{},"weatherCycleSeconds":{},"cloudOffsetMetres":{:?},"cloudVelocityMetresPerSecond":{:?},"cloudCoverage":{},"cloudBaseMetres":{},"cloudTopMetres":{},"weatherSeed":"{}","weatherRevision":"{}","sunDirection":{:?},"moonDirection":{:?},"debugDayFraction":{},"debugWeatherFraction":{},"reproductionOverride":{},"surfaceRegion":{}}},"#,
                r#""presentation":{{"viewportFingerprint":"{:016x}","selectedCutFingerprint":"{:016x}","terrainHandleSnapshot":{{"generation":"{}","cutFingerprint":"{:016x}","matchesPublishedCut":{}}},"selectedCut":{},"virtualTerrain":{},"worldQuads":{},"waterQuads":{},"drawCalls":{},"waterDrawCalls":{},"surfaceWidth":{},"surfaceHeight":{}}},"#,
                r#""streaming":{},"#,
                r#""attachments":{},"#,
                r#""render":{{"worldLabOpen":{},"features":{{"shadows":{},"voxelAmbientOcclusion":{},"screenSpaceAmbientOcclusion":{},"fog":{},"farTerrain":{},"water":{},"targetOutline":{},"materialDetail":{},"caveHeadlamp":{},"localLighting":{}}},"diagnosticSkyColor":{},"geometrySourceDebug":{},"viewDistanceMetres":{}}}}}"#
            ),
            frame_id,
            runtime_identity,
            gpu_identity,
            self.config.width,
            self.config.height,
            self.config.width as f32 / self.dpr,
            self.config.height as f32 / self.dpr,
            self.dpr,
            camera.position.to_array(),
            camera.velocity.to_array(),
            camera.yaw,
            camera.pitch,
            camera.yaw.to_degrees().rem_euclid(360.0),
            vertical_fov_radians,
            self.runtime_config.view_distance_metres,
            camera.grounded,
            locomotion,
            fluid.immersion,
            fluid.eye_depth_metres,
            fluid.signed_eye_depth_metres,
            fluid.surface_y_metres,
            fluid.surface_known,
            fluid.eyes_submerged,
            fluid.swimming,
            world,
            environment.server_time_seconds,
            environment.world_days,
            environment.day_fraction,
            environment.year_fraction,
            environment.moon_orbit_fraction,
            environment.twinkle_phase,
            environment.planet_circumference_metres,
            environment.axial_tilt_radians,
            environment.moon_orbit_inclination_radians,
            environment.celestial_seed,
            environment.celestial_revision,
            environment.weather_fraction,
            environment.weather_cycle_seconds,
            environment.cloud_offset_metres,
            environment.cloud_velocity_metres_per_second,
            environment.cloud_coverage,
            environment.cloud_base_metres,
            environment.cloud_top_metres,
            environment.weather_seed,
            environment.weather_revision,
            celestial.sun_direction,
            celestial.moon_direction,
            debug_day_fraction,
            debug_weather_fraction,
            self.reproduction_environment_override.is_some(),
            self.surface_region as u8,
            self.diagnostics.viewport_fingerprint,
            cut_fingerprint,
            self.diagnostics
                .virtual_terrain_presented_snapshot_generation,
            self.diagnostics
                .virtual_terrain_presented_snapshot_fingerprint,
            self.diagnostics
                .virtual_terrain_presented_snapshot_matches_cut,
            cut_manifest,
            virtual_terrain_manifest,
            self.diagnostics.quads,
            self.diagnostics.water_quads,
            self.diagnostics.draw_calls,
            self.diagnostics.water_draw_calls,
            self.diagnostics.surface_width,
            self.diagnostics.surface_height,
            streaming_manifest,
            attachment_manifest,
            self.ui.open(),
            options.shadows,
            options.ambient_occlusion,
            options.screen_space_ambient_occlusion,
            options.fog,
            options.far_terrain,
            options.water,
            options.target_outline,
            options.material_detail,
            options.cave_headlamp,
            options.local_lighting,
            diagnostic_sky,
            self.geometry_source_debug,
            self.runtime_config.view_distance_metres,
        )
    }

    pub fn take_host_ui_action(&mut self) -> Option<HostUiAction> {
        self.host_ui_action.take()
    }

    pub fn set_spectator_active(&mut self, active: bool) {
        self.ui.set_spectator_active(active);
    }

    pub fn set_spectator_available(&mut self, available: bool) {
        self.ui.set_spectator_available(available);
    }

    fn invalidate_virtual_terrain_desired_plan(&mut self) {
        self.virtual_terrain_oracle_cut = None;
        self.virtual_terrain_requested_view = None;
        self.virtual_terrain_oracle_view = None;
        self.virtual_terrain_headroom_frames = 0;
    }

    /// Records the desired immutable presentation envelope before streaming can return early.
    ///
    /// The desired envelope may advance while an older publication is frozen. Screenshot and
    /// startup diagnostics still identify that debt, while the transaction retains its own exact
    /// safety and horizon proof.
    pub fn begin_virtual_terrain_presentation_envelope(&mut self, envelope: &PresentationEnvelope) {
        if self.virtual_terrain_desired_envelope.as_ref() != Some(envelope) {
            self.invalidate_virtual_terrain_desired_plan();
            self.virtual_terrain_exact_surface_domain =
                Some(envelope.exact_surface_domain().clone());
            self.virtual_terrain_desired_envelope = Some(envelope.clone());
        }
    }

    pub fn register_virtual_terrain_directory(
        &mut self,
        directory: &TerrainHierarchyDirectoryV1,
    ) -> Result<(), VirtualTerrainRendererError> {
        self.virtual_terrain.register_region_directory(directory)?;
        self.invalidate_virtual_terrain_desired_plan();
        Ok(())
    }

    /// Registers replacement data without making any of its roots visible.
    pub fn register_virtual_terrain_staging_directory(
        &mut self,
        directory: &TerrainHierarchyDirectoryV1,
    ) -> Result<(), VirtualTerrainRendererError> {
        self.virtual_terrain.register_staging_directory(directory)?;
        Ok(())
    }

    /// Extends one resident surface node with an independently streamed four-child segment.
    pub fn register_virtual_terrain_refinement_directory(
        &mut self,
        directory: &TerrainHierarchyDirectoryV1,
    ) -> Result<(), VirtualTerrainRendererError> {
        self.virtual_terrain
            .register_refinement_directory(directory)?;
        self.invalidate_virtual_terrain_desired_plan();
        Ok(())
    }

    /// Atomically transfers terrain ownership between complete registered root partitions.
    pub fn set_virtual_terrain_active_roots(
        &mut self,
        roots: impl IntoIterator<Item = TerrainPageKey>,
    ) -> Result<(), VirtualTerrainRendererError> {
        let next = roots.into_iter().collect::<BTreeSet<_>>();
        self.virtual_terrain
            .set_active_roots(next.iter().copied())?;
        self.invalidate_virtual_terrain_desired_plan();
        Ok(())
    }

    /// Uploads a certified virtual-terrain page without publishing a partial render owner.
    ///
    /// GPU storage is prepared first, hierarchy identity is installed second, and only then is the
    /// page made addressable by a selected cut. Any failure frees the provisional allocation and
    /// leaves the prior resident page/cut untouched.
    pub fn upload_virtual_terrain_page(
        &mut self,
        page: TerrainPageV1,
    ) -> Result<(), VirtualTerrainRendererError> {
        let (exact_neighbor_sides, finer_neighbor_sides) = self
            .virtual_terrain_pages
            .get(&page.key)
            .map_or(([false; 4], [false; 4]), |existing| {
                (
                    existing.heightfield_exact_neighbor_sides,
                    existing.heightfield_finer_neighbor_sides,
                )
            });
        self.upload_virtual_terrain_page_with_seams(
            page,
            exact_neighbor_sides,
            finer_neighbor_sides,
            true,
        )
        .map(|_| ())
    }

    /// Stages and installs one complete refinement replacement as a failure-atomic group.
    ///
    /// The parent remains resident and drawable throughout. Child geometry is allocated and
    /// registered without hierarchy exposure first; only after every sibling succeeds are all
    /// child pages installed synchronously. A failed upload or coherence proof removes only the
    /// newly staged children, so no partial replacement can become selectable.
    pub fn upload_virtual_terrain_replacement_group(
        &mut self,
        parent: TerrainPageKey,
        mut pages: Vec<TerrainPageV1>,
    ) -> Result<(), VirtualTerrainRendererError> {
        let Some(mut expected) = parent.refinement_children() else {
            return Err(VirtualTerrainRendererError::IncompleteRootPartition(parent));
        };
        expected.sort_unstable();
        pages.sort_unstable_by_key(|page| page.key);
        if pages.len() != expected.len()
            || pages.iter().map(|page| page.key).collect::<Vec<_>>() != expected
            || self.virtual_terrain.resident_page(parent).is_none()
        {
            return Err(VirtualTerrainRendererError::IncompleteRootPartition(parent));
        }
        for page in &pages {
            if self
                .virtual_terrain
                .resident_page(page.key)
                .is_some_and(|resident| resident != page)
                || self
                    .virtual_terrain_pages
                    .get(&page.key)
                    .is_some_and(|resident| {
                        resident.revision != page.revision
                            || resident.content_fingerprint != page.content_fingerprint
                            || resident.representation != page.representation.kind()
                    })
            {
                return Err(VirtualTerrainError::StalePage(page.key).into());
            }
        }

        // Reclaim unrelated travel history once, then prove that every missing sibling can coexist
        // with the immutable committed owner. This is a transaction-local free-range simulation;
        // future requested groups remain encoded in the shell cache and do not affect admission.
        self.retain_virtual_terrain_pages(std::iter::empty())?;
        let allocation_sizes = pages
            .iter()
            .filter(|page| !self.virtual_terrain_pages.contains_key(&page.key))
            .map(|page| {
                self.virtual_terrain
                    .directory_node(page.key)
                    .map(|node| node.source_geometry_bytes)
                    .ok_or(VirtualTerrainRendererError::SelectedPageMissingGpu(
                        page.key,
                    ))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if !self
            .virtual_terrain_arena
            .can_allocate_batch(allocation_sizes)
        {
            return Err(VirtualTerrainRendererError::GpuPoolCapacity);
        }

        let mut staged = Vec::new();
        for page in &pages {
            if self.virtual_terrain_pages.contains_key(&page.key) {
                continue;
            }
            match self.upload_virtual_terrain_page_with_seams(
                page.clone(),
                [false; 4],
                [false; 4],
                false,
            ) {
                Ok(true) => staged.push(page.key),
                Ok(false) => {}
                Err(error) => {
                    self.virtual_terrain_heightfield_samples.remove(&page.key);
                    self.rollback_virtual_terrain_replacement_geometry(&staged);
                    return Err(error);
                }
            }
        }

        if let Err(error) = install_virtual_terrain_replacement_pages(
            &mut self.virtual_terrain,
            parent,
            &pages,
            |_| Ok(()),
        ) {
            self.rollback_virtual_terrain_replacement_geometry(&staged);
            return Err(error.into());
        }
        self.virtual_terrain_staging_frontier.insert(parent);
        self.virtual_terrain_staging_frontier
            .extend(expected.iter().copied());
        self.invalidate_virtual_terrain_desired_plan();
        Ok(())
    }

    fn rollback_virtual_terrain_replacement_geometry(&mut self, staged: &[TerrainPageKey]) {
        for key in staged.iter().rev() {
            self.virtual_terrain_heightfield_samples.remove(key);
            self.virtual_terrain_gpu.remove_page_geometry(*key);
            if let Some(page) = self.virtual_terrain_pages.remove(key) {
                discard_virtual_terrain_mesh(&mut self.virtual_terrain_arena, page.mesh);
            }
        }
    }

    fn upload_virtual_terrain_page_with_seams(
        &mut self,
        page: TerrainPageV1,
        exact_neighbor_sides: [bool; 4],
        finer_neighbor_sides: [bool; 4],
        install_page: bool,
    ) -> Result<bool, VirtualTerrainRendererError> {
        let (base_heightfield, heightfield_ancestors_complete) = if matches!(
            page.representation,
            TerrainPageRepresentation::HeightfieldGrid(_)
        ) {
            let (samples, complete) = parent_constrained_virtual_heightfield_samples_with_cache(
                &self.virtual_terrain,
                &self.virtual_terrain_heightfield_samples,
                &page,
            )?;
            (Some(samples), complete)
        } else {
            (None, false)
        };
        if page.key.level > 0 && heightfield_ancestors_complete {
            if let (Some(samples), Some(ancestor_fingerprint)) = (
                base_heightfield.as_ref(),
                heightfield_ancestor_fingerprint(&self.virtual_terrain, page.key),
            ) {
                self.virtual_terrain_heightfield_samples.insert(
                    page.key,
                    CachedVirtualHeightfieldSamples {
                        revision: page.revision,
                        content_fingerprint: page.content_fingerprint,
                        ancestor_fingerprint,
                        samples: samples.clone(),
                    },
                );
            }
        } else {
            self.virtual_terrain_heightfield_samples.remove(&page.key);
        }
        let mut constrained_heightfield = match (&page.representation, base_heightfield) {
            (TerrainPageRepresentation::HeightfieldGrid(grid), Some(samples))
                if page.key.level == 0 =>
            {
                Some(restore_exact_neighbor_heightfield_boundaries(
                    &page,
                    grid,
                    &samples,
                    exact_neighbor_sides,
                ))
            }
            (_, samples) => samples,
        };
        if let Some(heightfield) = constrained_heightfield.as_mut() {
            heightfield.finer_neighbor_sides = finer_neighbor_sides;
        }
        let heightfield_exact_neighbor_sides = constrained_heightfield
            .as_ref()
            .map_or([false; 4], |heightfield| heightfield.exact_neighbor_sides);
        let heightfield_finer_neighbor_sides = constrained_heightfield
            .as_ref()
            .map_or([false; 4], |heightfield| heightfield.finer_neighbor_sides);
        let heightfield_ground_corner_bits =
            constrained_heightfield
                .as_ref()
                .map_or([0; 4], |heightfield| {
                    let cells = TERRAIN_PAGE_EDGE_SAMPLES as usize;
                    let edge = cells + 1;
                    [0, cells, cells + cells * edge, cells * edge]
                        .map(|index| heightfield.ground[index].to_bits())
                });
        let heightfield_ground_boundary_bits = constrained_heightfield.as_ref().map_or(
            [[0; TERRAIN_PAGE_EDGE_SAMPLES as usize + 1]; 4],
            |heightfield| {
                let cells = TERRAIN_PAGE_EDGE_SAMPLES as usize;
                let edge = cells + 1;
                std::array::from_fn(|side| {
                    std::array::from_fn(|offset| {
                        let index = match side {
                            0 => offset * edge,
                            1 => cells + offset * edge,
                            2 => offset,
                            _ => offset + cells * edge,
                        };
                        heightfield.ground[index].to_bits()
                    })
                })
            },
        );
        if let Some(existing) = self.virtual_terrain_pages.get(&page.key)
            && existing.revision == page.revision
            && existing.content_fingerprint == page.content_fingerprint
            && existing.representation == page.representation.kind()
            && existing.heightfield_exact_neighbor_sides == heightfield_exact_neighbor_sides
            && existing.heightfield_finer_neighbor_sides == heightfield_finer_neighbor_sides
            && existing.heightfield_ground_corner_bits == heightfield_ground_corner_bits
            && existing.heightfield_ground_boundary_bits == heightfield_ground_boundary_bits
        {
            if install_page {
                let key = page.key;
                self.virtual_terrain.install_page(page)?;
                self.virtual_terrain_staging_frontier.insert(key);
            }
            return Ok(false);
        }
        let exact_heightfield_quads = match (&page.representation, constrained_heightfield.as_ref())
        {
            (TerrainPageRepresentation::HeightfieldGrid(grid), Some(heightfield))
                if page.key.level == 0 =>
            {
                virtual_microvoxel_gpu_quads(&page, grid, heightfield)?
            }
            _ => None,
        };
        let minimum = glam::Vec3::from_array(
            page.bounds
                .min
                .as_array()
                .map(|value| value as f32 * VOXEL_SIZE_METRES),
        );
        let maximum = glam::Vec3::from_array(
            page.bounds
                .max
                .as_array()
                .map(|value| value as f32 * VOXEL_SIZE_METRES),
        );
        let mesh = match &page.representation {
            TerrainPageRepresentation::SteppedSurfaceResidual(_)
            | TerrainPageRepresentation::SparseVoxelBrick(_)
            | TerrainPageRepresentation::SurfaceCluster(_) => {
                let gpu_quads = virtual_surface_gpu_quads(&page)?;
                let (gpu_quads, opaque_quad_count, water_quad_count) =
                    partition_virtual_surface_geometry(gpu_quads)
                        .ok_or(VirtualTerrainRendererError::GpuPageTooLarge(page.key))?;
                let gpu_bytes = gpu_quads
                    .len()
                    .checked_mul(size_of::<GpuQuad>())
                    .ok_or(VirtualTerrainRendererError::GpuPageTooLarge(page.key))?;
                if gpu_bytes > ARENA_PAGE_BYTES as usize {
                    return Err(VirtualTerrainRendererError::GpuPageTooLarge(page.key));
                }
                self.ensure_virtual_terrain_gpu_allocation(
                    u32::try_from(gpu_bytes)
                        .map_err(|_| VirtualTerrainRendererError::GpuPageTooLarge(page.key))?,
                )?;
                if gpu_quads.is_empty() {
                    VirtualTerrainGpuMesh::Empty
                } else {
                    let opaque_size = opaque_quad_count
                        .checked_mul(size_of::<GpuQuad>() as u32)
                        .ok_or(VirtualTerrainRendererError::GpuPageTooLarge(page.key))?;
                    let water_size = water_quad_count
                        .checked_mul(size_of::<GpuQuad>() as u32)
                        .ok_or(VirtualTerrainRendererError::GpuPageTooLarge(page.key))?;
                    let mut slices = Vec::with_capacity(2);
                    if opaque_quad_count > 0 {
                        slices.push(virtual_terrain_surface_slice(
                            0,
                            opaque_size,
                            opaque_quad_count,
                            minimum,
                            maximum,
                            RenderLayer::Opaque,
                        ));
                    }
                    if water_quad_count > 0 {
                        slices.push(virtual_terrain_surface_slice(
                            opaque_size,
                            water_size,
                            water_quad_count,
                            minimum,
                            maximum,
                            RenderLayer::Translucent,
                        ));
                    }
                    VirtualTerrainGpuMesh::Surface(
                        prepare_mesh_sliced_into(
                            &self.device,
                            &self.queue,
                            &mut self.virtual_terrain_arena,
                            &mut self.virtual_terrain_arena_buffers,
                            &gpu_quads,
                            slices,
                            u8::MAX,
                            "bounded virtual terrain page pool",
                        )
                        .ok_or(VirtualTerrainRendererError::GpuPoolCapacity)?,
                    )
                }
            }
            TerrainPageRepresentation::HeightfieldGrid(_) if exact_heightfield_quads.is_some() => {
                let gpu_quads = exact_heightfield_quads.unwrap_or_default();
                let (gpu_quads, opaque_quad_count, water_quad_count) =
                    partition_virtual_surface_geometry(gpu_quads)
                        .ok_or(VirtualTerrainRendererError::GpuPageTooLarge(page.key))?;
                let gpu_bytes = gpu_quads
                    .len()
                    .checked_mul(size_of::<GpuQuad>())
                    .ok_or(VirtualTerrainRendererError::GpuPageTooLarge(page.key))?;
                if gpu_bytes > ARENA_PAGE_BYTES as usize {
                    return Err(VirtualTerrainRendererError::GpuPageTooLarge(page.key));
                }
                self.ensure_virtual_terrain_gpu_allocation(
                    u32::try_from(gpu_bytes)
                        .map_err(|_| VirtualTerrainRendererError::GpuPageTooLarge(page.key))?,
                )?;
                if gpu_quads.is_empty() {
                    VirtualTerrainGpuMesh::Empty
                } else {
                    let opaque_size = opaque_quad_count
                        .checked_mul(size_of::<GpuQuad>() as u32)
                        .ok_or(VirtualTerrainRendererError::GpuPageTooLarge(page.key))?;
                    let water_size = water_quad_count
                        .checked_mul(size_of::<GpuQuad>() as u32)
                        .ok_or(VirtualTerrainRendererError::GpuPageTooLarge(page.key))?;
                    let mut slices = Vec::with_capacity(2);
                    if opaque_quad_count > 0 {
                        slices.push(virtual_terrain_surface_slice(
                            0,
                            opaque_size,
                            opaque_quad_count,
                            minimum,
                            maximum,
                            RenderLayer::Opaque,
                        ));
                    }
                    if water_quad_count > 0 {
                        slices.push(virtual_terrain_surface_slice(
                            opaque_size,
                            water_size,
                            water_quad_count,
                            minimum,
                            maximum,
                            RenderLayer::Translucent,
                        ));
                    }
                    VirtualTerrainGpuMesh::Surface(
                        prepare_mesh_sliced_into(
                            &self.device,
                            &self.queue,
                            &mut self.virtual_terrain_arena,
                            &mut self.virtual_terrain_arena_buffers,
                            &gpu_quads,
                            slices,
                            u8::MAX,
                            "bounded exact virtual terrain page pool",
                        )
                        .ok_or(VirtualTerrainRendererError::GpuPoolCapacity)?,
                    )
                }
            }
            TerrainPageRepresentation::TriangleCluster(_)
            | TerrainPageRepresentation::HeightfieldGrid(_) => {
                let vertices =
                    virtual_triangle_gpu_vertices(&page, constrained_heightfield.as_ref())?;
                let (vertices, opaque_vertex_count, water_vertex_count) =
                    partition_virtual_triangle_geometry(vertices)
                        .ok_or(VirtualTerrainRendererError::GpuPageTooLarge(page.key))?;
                let gpu_bytes = vertices
                    .len()
                    .checked_mul(size_of::<GpuTerrainVertex>())
                    .ok_or(VirtualTerrainRendererError::GpuPageTooLarge(page.key))?;
                if gpu_bytes > ARENA_PAGE_BYTES as usize {
                    return Err(VirtualTerrainRendererError::GpuPageTooLarge(page.key));
                }
                self.ensure_virtual_terrain_gpu_allocation(
                    u32::try_from(gpu_bytes)
                        .map_err(|_| VirtualTerrainRendererError::GpuPageTooLarge(page.key))?,
                )?;
                if vertices.is_empty() {
                    VirtualTerrainGpuMesh::Empty
                } else {
                    VirtualTerrainGpuMesh::Triangle(
                        prepare_terrain_triangle_mesh_into(
                            &self.device,
                            &self.queue,
                            &mut self.virtual_terrain_arena,
                            &mut self.virtual_terrain_arena_buffers,
                            &vertices,
                            opaque_vertex_count,
                            water_vertex_count,
                            minimum,
                            maximum,
                            "bounded virtual terrain page pool",
                        )
                        .ok_or(VirtualTerrainRendererError::GpuPoolCapacity)?,
                    )
                }
            }
        };
        let geometry = virtual_terrain_gpu_geometry(&mesh);
        let allocation_page = match &mesh {
            VirtualTerrainGpuMesh::Empty => None,
            VirtualTerrainGpuMesh::Surface(mesh) => Some(mesh.allocation.page),
            VirtualTerrainGpuMesh::Triangle(mesh) => Some(mesh.allocation.page),
        };
        if allocation_page.is_some()
            && self.virtual_terrain_gpu.bound_geometry_source_count()
                != self.virtual_terrain_arena_buffers.len()
        {
            // Source segments are append-only and never replaced in place. Rebuild the bank bind
            // groups exactly when a newly allocated arena segment changes buffer identity.
            if self
                .virtual_terrain_gpu
                .bind_geometry_sources(&self.device, &self.virtual_terrain_arena_buffers)
                .is_err()
            {
                discard_virtual_terrain_mesh(&mut self.virtual_terrain_arena, mesh);
                return Err(VirtualTerrainRendererError::GpuSnapshot);
            }
        }
        if install_page {
            if let Err(error) = self.virtual_terrain.install_page(page.clone()) {
                discard_virtual_terrain_mesh(&mut self.virtual_terrain_arena, mesh);
                return Err(error.into());
            }
            self.virtual_terrain_staging_frontier.insert(page.key);
            // Residency can make a complete child replacement selectable without changing the
            // view. Invalidate the CPU demand oracle before encoding another handle snapshot.
            self.invalidate_virtual_terrain_desired_plan();
        }
        let geometry_update = self
            .virtual_terrain_gpu
            .update_page_geometry(page.key, geometry);
        if geometry_update.is_err() {
            discard_virtual_terrain_mesh(&mut self.virtual_terrain_arena, mesh);
            return Err(VirtualTerrainRendererError::GpuSnapshot);
        }
        let resident = VirtualTerrainGpuPage {
            revision: page.revision,
            content_fingerprint: page.content_fingerprint,
            representation: page.representation.kind(),
            heightfield_exact_neighbor_sides,
            heightfield_finer_neighbor_sides,
            heightfield_ground_corner_bits,
            heightfield_ground_boundary_bits,
            mesh,
        };
        if let Some(old) = self.virtual_terrain_pages.insert(page.key, resident) {
            let published = self
                .virtual_terrain_cut
                .as_ref()
                .is_some_and(|cut| cut.selected_pages.contains(&page.key));
            if published
                && !self
                    .virtual_terrain_retired_published_pages
                    .contains_key(&page.key)
            {
                // Published geometry is immutable. The replacement can be selected and certified
                // beside it, while direct rendering and the active handle snapshot continue to
                // reference this exact allocation until the whole new snapshot publishes.
                self.virtual_terrain_retired_published_pages
                    .insert(page.key, old);
            } else {
                discard_virtual_terrain_mesh(&mut self.virtual_terrain_arena, old.mesh);
            }
        }
        Ok(true)
    }

    pub fn select_virtual_terrain_cut(
        &mut self,
        view: VirtualTerrainView,
        exact_surface_domain: &ExactSurfaceDomain,
    ) -> Result<VirtualTerrainCut, VirtualTerrainRendererError> {
        let known_fitting_scale = self.virtual_terrain_error_scale.max(1.0);
        let mut recovery_probe = false;
        if self.virtual_terrain_requested_view == Some(view)
            && self.virtual_terrain_exact_surface_domain.as_ref() == Some(exact_surface_domain)
        {
            let sustained_headroom = known_fitting_scale > 1.0
                && self
                    .virtual_terrain_oracle_cut
                    .as_ref()
                    .and_then(|cut| self.virtual_terrain_cut_snapshot_source_bytes(cut).ok())
                    .is_some_and(|required| {
                        required
                            .into_iter()
                            .zip([
                                VIRTUAL_TERRAIN_SURFACE_HANDLE_SOURCE_BYTES,
                                VIRTUAL_TERRAIN_TRIANGLE_HANDLE_SOURCE_BYTES,
                                VIRTUAL_TERRAIN_WATER_SURFACE_HANDLE_SOURCE_BYTES,
                                VIRTUAL_TERRAIN_WATER_TRIANGLE_HANDLE_SOURCE_BYTES,
                            ])
                            .all(|(used, capacity)| {
                                used.saturating_mul(4) <= capacity.saturating_mul(3)
                            })
                    });
            if sustained_headroom {
                self.virtual_terrain_headroom_frames =
                    self.virtual_terrain_headroom_frames.saturating_add(1);
            } else {
                self.virtual_terrain_headroom_frames = 0;
            }
            if self.virtual_terrain_headroom_frames >= 120 {
                // Probe one finer setting only after the compact ownership bank has sustained
                // headroom. Expanded source geometry is admitted transactionally per real group;
                // it must never globally degrade the screen-error target.
                recovery_probe = true;
                self.virtual_terrain_headroom_frames = 0;
                self.virtual_terrain_requested_view = None;
            } else if let Some(cut) = self.virtual_terrain_oracle_cut.as_ref() {
                return Ok(cut.clone());
            }
        }
        // Handle-bank coverage is a hard rendering capacity, not an error to discover after a cut
        // has already streamed. Increase the screen-error threshold only as far as needed to keep
        // the selected cut publishable. Both the CPU oracle and GPU receive the same adjusted
        // view, so this remains one deterministic ownership decision rather than a fallback draw.
        let mut error_scale = if recovery_probe {
            (known_fitting_scale / 1.1).max(1.0)
        } else {
            known_fitting_scale
        };
        let (oracle_view, cut) = loop {
            let mut candidate_view = view;
            if !view.force_exact_leaves {
                candidate_view.refine_above_pixels *= error_scale;
                candidate_view.coarsen_below_pixels *= error_scale;
            }
            let candidate = self
                .virtual_terrain
                .select_cut(candidate_view, exact_surface_domain)?;
            let capacity = self.virtual_terrain_cut_fits_snapshot(&candidate);
            match capacity {
                Ok(()) => break (candidate_view, candidate),
                Err(VirtualTerrainRendererError::SelectedCutSnapshotCapacity { .. })
                    if recovery_probe && error_scale < known_fitting_scale =>
                {
                    error_scale = known_fitting_scale;
                    recovery_probe = false;
                }
                Err(VirtualTerrainRendererError::SelectedCutSnapshotCapacity { .. })
                    if !view.force_exact_leaves && error_scale < 64.0 =>
                {
                    error_scale = (error_scale * 1.25).min(64.0);
                }
                Err(error) => return Err(error),
            }
        };
        if error_scale > self.virtual_terrain_error_scale {
            self.virtual_terrain_headroom_frames = 0;
        }
        self.virtual_terrain_error_scale = error_scale;
        if !self.virtual_terrain_staging_frontier.is_empty() {
            self.virtual_terrain_staging_frontier.extend(
                cut.selected_pages
                    .iter()
                    .copied()
                    .filter(|key| self.virtual_terrain_pages.contains_key(key)),
            );
            self.virtual_terrain_staging_frontier.extend(
                cut.requested_pages
                    .iter()
                    .map(|identity| identity.key)
                    .filter(|key| self.virtual_terrain_pages.contains_key(key)),
            );
        }
        self.virtual_terrain_requested_view = Some(view);
        self.virtual_terrain_oracle_view = Some(oracle_view);
        self.virtual_terrain_exact_surface_domain = Some(exact_surface_domain.clone());
        self.virtual_terrain_oracle_cut = Some(cut.clone());
        if self.virtual_terrain_mode == VirtualTerrainRenderMode::Disabled {
            // A fresh renderer has no published virtual owner yet. Shadow mode certifies the
            // candidate handle snapshot before its first publication.
            self.virtual_terrain_mode = VirtualTerrainRenderMode::Shadow;
        }
        Ok(cut)
    }

    pub fn virtual_terrain_cut(&self) -> Option<&VirtualTerrainCut> {
        self.virtual_terrain_cut.as_ref()
    }

    /// Whether the immutable active handle bank is the last committed ownership cut.
    ///
    /// A newer desired cut or an in-flight publication is expected during travel and does not make
    /// the committed snapshot unsafe. Presentation readiness therefore never depends on agreement
    /// with the ephemeral quality target.
    pub fn virtual_terrain_committed_snapshot_is_valid(&self) -> bool {
        let committed = self.virtual_terrain_cut.as_ref();
        let envelope = self.virtual_terrain_committed_envelope.as_ref();
        virtual_terrain_committed_snapshot_is_safe(
            committed.is_some() && envelope.is_some(),
            committed.is_some_and(VirtualTerrainCut::is_renderable)
                && committed
                    .zip(envelope)
                    .is_some_and(|(cut, envelope)| cut.covers_presentation_envelope(envelope)),
            committed
                .zip(envelope)
                .is_some_and(|(committed, envelope)| {
                    self.virtual_terrain_gpu.presented_snapshot_matches(
                        virtual_terrain_snapshot_identity(committed, envelope),
                    )
                }),
        )
    }

    pub fn virtual_terrain_committed_covers_presentation_envelope(
        &self,
        envelope: &PresentationEnvelope,
    ) -> bool {
        self.virtual_terrain_committed_snapshot_is_valid()
            && self.virtual_terrain_committed_envelope.as_ref() == Some(envelope)
    }

    pub fn virtual_terrain_committed_envelope(&self) -> Option<&PresentationEnvelope> {
        self.virtual_terrain_committed_envelope.as_ref()
    }

    pub fn virtual_terrain_committed_locus(&self) -> Option<PresentationLocus> {
        self.virtual_terrain_committed_envelope
            .as_ref()
            .and_then(PresentationEnvelope::locus)
    }

    pub fn virtual_terrain_committed_contains_position(&self, position_metres: [f32; 3]) -> bool {
        self.virtual_terrain_committed_snapshot_is_valid()
            && self
                .virtual_terrain_committed_envelope
                .as_ref()
                .is_some_and(|envelope| envelope.contains_position(position_metres))
    }

    pub fn clamp_to_virtual_terrain_committed_locus(
        &self,
        position_metres: [f32; 3],
    ) -> Option<[f32; 3]> {
        self.virtual_terrain_committed_snapshot_is_valid()
            .then(|| {
                self.virtual_terrain_committed_envelope
                    .as_ref()
                    .and_then(|envelope| envelope.clamp_position(position_metres))
            })
            .flatten()
    }

    fn synchronize_virtual_terrain_cut_seams(
        &mut self,
        cut: &VirtualTerrainCut,
    ) -> Result<bool, VirtualTerrainRendererError> {
        let selected = cut.selected_pages.iter().copied().collect::<BTreeSet<_>>();
        let published = self
            .virtual_terrain_cut
            .as_ref()
            .filter(|_| self.virtual_terrain_mode == VirtualTerrainRenderMode::Visible)
            .map(|cut| cut.selected_pages.iter().copied().collect::<BTreeSet<_>>())
            .unwrap_or_default();
        let rebuilds = cut
            .selected_pages
            .iter()
            .filter_map(|key| {
                let page = self.virtual_terrain.resident_page(*key)?;
                if !matches!(
                    page.representation,
                    TerrainPageRepresentation::HeightfieldGrid(_)
                ) {
                    return None;
                }
                let exact_sides = cut_exact_neighbor_sides(&selected, *key);
                let finer_sides = cut_finer_neighbor_sides(&selected, *key);
                self.virtual_terrain_pages.get(key).and_then(|resident| {
                    (resident.heightfield_exact_neighbor_sides != exact_sides
                        || resident.heightfield_finer_neighbor_sides != finer_sides)
                        .then(|| (page.clone(), exact_sides, finer_sides))
                })
            })
            .collect::<Vec<_>>();
        if rebuilds.is_empty() {
            return Ok(false);
        }
        // Invalidate before the first allocation or geometry-directory mutation. If a later page
        // fails, the old published bank remains drawable but can no longer certify this candidate;
        // recovery must encode a fresh complete snapshot from the surviving source records.
        self.virtual_terrain_gpu.invalidate_candidate();
        let mut changed = false;
        for (page, exact_sides, finer_sides) in rebuilds {
            let page_key = page.key;
            let can_replace_in_place = !published.contains(&page.key)
                && self
                    .virtual_terrain_pages
                    .get(&page.key)
                    .is_some_and(|resident| {
                        resident.heightfield_exact_neighbor_sides != exact_sides
                            || resident.heightfield_finer_neighbor_sides != finer_sides
                    });
            if can_replace_in_place {
                // Candidate-only geometry has no visible owner. Release it before rebuilding so
                // startup/travel does not need a second full page allocation merely to change its
                // conforming boundary. Published owners continue through the transactional path.
                self.virtual_terrain_gpu.remove_page_geometry(page.key);
                if let Some(old) = self.virtual_terrain_pages.remove(&page.key) {
                    discard_virtual_terrain_mesh(&mut self.virtual_terrain_arena, old.mesh);
                }
                let _ = self.virtual_terrain.remove_page(page.key);
                let rebuilt = self.upload_virtual_terrain_page_with_seams(
                    page,
                    exact_sides,
                    finer_sides,
                    true,
                );
                changed |= recover_candidate_only_seam_rebuild(rebuilt, || {
                    self.recover_failed_candidate_only_seam_page(page_key)
                })?;
            } else {
                changed |= self.upload_virtual_terrain_page_with_seams(
                    page,
                    exact_sides,
                    finer_sides,
                    false,
                )?;
            }
        }
        debug_assert!(changed);
        Ok(changed)
    }

    fn recover_failed_candidate_only_seam_page(&mut self, key: TerrainPageKey) {
        self.virtual_terrain_heightfield_samples.remove(&key);
        let _ = self.virtual_terrain.remove_page(key);
        self.virtual_terrain_gpu.remove_page_geometry(key);
        if let Some(page) = self.virtual_terrain_pages.remove(&key) {
            discard_virtual_terrain_mesh(&mut self.virtual_terrain_arena, page.mesh);
        }
        // The failed page was candidate-only: keep presenting the immutable published bank and
        // force the oracle to choose a complete resident fallback on the next selection.
        self.virtual_terrain_oracle_cut = None;
        self.virtual_terrain_requested_view = None;
        self.virtual_terrain_oracle_view = None;
    }

    /// Freezes the latest desired cut as the next publication transaction.
    ///
    /// This is intentionally separate from selection: directory and cache state may continue to
    /// change while the frozen cut is encoded and certified. Only one transaction can own the
    /// inactive bank at a time.
    pub fn prepare_virtual_terrain_publication(
        &mut self,
    ) -> Result<bool, VirtualTerrainRendererError> {
        if !virtual_terrain_publication_can_stage(self.virtual_terrain_publication.is_some()) {
            return Ok(false);
        }
        let Some(cut) = self
            .virtual_terrain_oracle_cut
            .as_ref()
            .filter(|cut| cut.is_renderable())
            .cloned()
        else {
            return Ok(false);
        };
        let Some(envelope) = self
            .virtual_terrain_desired_envelope
            .as_ref()
            .filter(|envelope| envelope.is_complete())
            .cloned()
        else {
            return Ok(false);
        };
        if !cut.covers_presentation_envelope(&envelope) {
            return Ok(false);
        }
        if let Some(missing) = cut
            .selected_pages
            .iter()
            .find(|key| !self.virtual_terrain_pages.contains_key(key))
        {
            return Err(VirtualTerrainRendererError::SelectedPageMissingGpu(
                *missing,
            ));
        }
        self.synchronize_virtual_terrain_cut_seams(&cut)?;
        self.virtual_terrain_cut_fits_snapshot(&cut)?;
        let publication = VirtualTerrainPublication { cut, envelope };
        if self
            .virtual_terrain_gpu
            .active_snapshot_matches(virtual_terrain_snapshot_identity(
                &publication.cut,
                &publication.envelope,
            ))
        {
            if self
                .virtual_terrain_cut
                .as_ref()
                .is_none_or(|committed| committed != &publication.cut)
                || self.virtual_terrain_committed_envelope.as_ref() != Some(&publication.envelope)
            {
                self.virtual_terrain_cut = Some(publication.cut);
                self.virtual_terrain_committed_envelope = Some(publication.envelope);
            }
            self.virtual_terrain_staging_frontier.clear();
            self.invalidate_virtual_terrain_desired_plan();
            return Ok(false);
        }
        self.virtual_terrain_publication = Some(publication);
        Ok(true)
    }

    /// Promotes a certified frozen transaction before any new geometry mutation.
    ///
    /// Promotion is valid in Shadow mode. This lets cold start progress through several locally
    /// certified cuts while canonical terrain remains the sole visible owner.
    pub fn advance_virtual_terrain_publication(
        &mut self,
    ) -> Result<bool, VirtualTerrainRendererError> {
        if self.virtual_terrain_publication_abort_pending {
            self.retain_virtual_terrain_pages(std::iter::empty())?;
            self.virtual_terrain_publication_abort_pending = false;
        }
        let Some(publication) = self.virtual_terrain_publication.as_ref().cloned() else {
            return Ok(false);
        };
        if !publication
            .cut
            .covers_presentation_envelope(&publication.envelope)
        {
            self.abort_virtual_terrain_publication();
            return Ok(false);
        }
        let identity = virtual_terrain_snapshot_identity(&publication.cut, &publication.envelope);
        match virtual_terrain_publication_advance(
            true,
            self.virtual_terrain_gpu.active_snapshot_matches(identity),
            self.virtual_terrain_gpu.candidate_is_certified(identity),
        ) {
            VirtualTerrainPublicationAdvance::PromoteCertified => {
                self.virtual_terrain_gpu
                    .promote_certified_candidate(identity)
                    .map_err(|_| VirtualTerrainRendererError::GpuCutNotCertified)?;
            }
            VirtualTerrainPublicationAdvance::CommitActive => {}
            VirtualTerrainPublicationAdvance::AwaitCertificate
            | VirtualTerrainPublicationAdvance::Idle => return Ok(false),
        }
        self.discard_retired_virtual_terrain_pages();
        self.virtual_terrain_cut = Some(publication.cut);
        self.virtual_terrain_committed_envelope = Some(publication.envelope);
        self.virtual_terrain_publication = None;
        self.virtual_terrain_staging_frontier.clear();
        // The unchanged-view cache was selected against the old committed overlap/capacity. Force
        // the next frame to derive demand and admission from the cut that actually promoted.
        self.invalidate_virtual_terrain_desired_plan();
        Ok(true)
    }

    pub const fn virtual_terrain_publication_in_flight(&self) -> bool {
        self.virtual_terrain_publication.is_some()
    }

    /// Whether source geometry is fenced by either a frozen publication or an accumulating
    /// microbatch. Region retirement must not invalidate either owner.
    pub fn virtual_terrain_publication_owns_geometry(&self) -> bool {
        self.virtual_terrain_publication.is_some()
            || !self.virtual_terrain_staging_frontier.is_empty()
    }

    fn abort_virtual_terrain_publication(&mut self) {
        self.virtual_terrain_gpu.invalidate_candidate();
        self.virtual_terrain_publication = None;
        self.virtual_terrain_staging_frontier.clear();
        self.virtual_terrain_publication_abort_pending = true;
        self.invalidate_virtual_terrain_desired_plan();
    }

    pub fn set_virtual_terrain_render_mode(
        &mut self,
        mode: VirtualTerrainRenderMode,
    ) -> Result<(), VirtualTerrainRendererError> {
        if mode == VirtualTerrainRenderMode::Visible
            && !self.virtual_terrain_committed_snapshot_is_valid()
        {
            return Err(VirtualTerrainRendererError::GpuCutNotCertified);
        }
        self.virtual_terrain_mode = mode;
        Ok(())
    }

    pub const fn virtual_terrain_render_mode(&self) -> VirtualTerrainRenderMode {
        self.virtual_terrain_mode
    }

    pub fn virtual_terrain_region_roots(&self) -> Vec<TerrainPageKey> {
        self.virtual_terrain.roots().collect()
    }

    pub fn registered_virtual_terrain_region_roots(&self) -> Vec<TerrainPageKey> {
        self.virtual_terrain.registered_roots().collect()
    }

    /// Retires immutable region directories outside the current streaming working set.
    ///
    /// Removing directory roots invalidates the next candidate snapshot. Pages belonging to the
    /// published cut move into an immutable retirement set rather than disappearing; the old
    /// virtual owner remains visible while its complete revised replacement is built beside it.
    pub fn retain_virtual_terrain_regions(
        &mut self,
        keep: impl IntoIterator<Item = TerrainPageKey>,
    ) -> Result<usize, VirtualTerrainRendererError> {
        let keep = keep.into_iter().collect::<BTreeSet<_>>();
        let remove = self
            .virtual_terrain
            .registered_roots()
            .filter(|root| !keep.contains(root))
            .collect::<Vec<_>>();
        if remove.is_empty() {
            return Ok(0);
        }
        self.virtual_terrain_oracle_cut = None;
        self.virtual_terrain_requested_view = None;
        self.virtual_terrain_oracle_view = None;
        let mut removed_pages = BTreeSet::new();
        for root in remove {
            removed_pages.extend(self.virtual_terrain.remove_region_directory(root));
        }
        let invalidates_publication =
            self.virtual_terrain_publication
                .as_ref()
                .is_some_and(|publication| {
                    publication
                        .cut
                        .selected_pages
                        .iter()
                        .any(|key| removed_pages.contains(key))
                })
                || self
                    .virtual_terrain_staging_frontier
                    .iter()
                    .any(|key| removed_pages.contains(key));
        if invalidates_publication {
            self.abort_virtual_terrain_publication();
        }
        let published = self
            .virtual_terrain_cut
            .as_ref()
            .map(|cut| cut.selected_pages.iter().copied().collect::<BTreeSet<_>>())
            .unwrap_or_default();
        for key in &removed_pages {
            self.virtual_terrain_heightfield_samples.remove(key);
            // Remove the candidate-directory handle before the backing allocation can be freed or
            // reused. A published allocation is retained separately until a new bank promotes.
            self.virtual_terrain_gpu.remove_page_geometry(*key);
            let Some(page) = self.virtual_terrain_pages.remove(key) else {
                continue;
            };
            if published.contains(key)
                && !self
                    .virtual_terrain_retired_published_pages
                    .contains_key(key)
            {
                self.virtual_terrain_retired_published_pages
                    .insert(*key, page);
            } else {
                discard_virtual_terrain_mesh(&mut self.virtual_terrain_arena, page.mesh);
            }
        }
        Ok(removed_pages.len())
    }

    pub fn remove_virtual_terrain_page(
        &mut self,
        key: TerrainPageKey,
    ) -> Result<bool, VirtualTerrainRendererError> {
        if !self.virtual_terrain.remove_page(key) {
            return Ok(false);
        }
        if self
            .virtual_terrain_publication
            .as_ref()
            .is_some_and(|publication| publication.cut.selected_pages.contains(&key))
            || self.virtual_terrain_staging_frontier.contains(&key)
        {
            self.abort_virtual_terrain_publication();
        }
        self.virtual_terrain_oracle_cut = None;
        self.virtual_terrain_requested_view = None;
        self.virtual_terrain_oracle_view = None;
        let published = self
            .virtual_terrain_cut
            .as_ref()
            .is_some_and(|cut| cut.selected_pages.contains(&key));
        // Invalidate any pending absolute handles before this allocation can be released or reused.
        self.virtual_terrain_gpu.remove_page_geometry(key);
        if let Some(page) = self.virtual_terrain_pages.remove(&key) {
            if published
                && !self
                    .virtual_terrain_retired_published_pages
                    .contains_key(&key)
            {
                self.virtual_terrain_retired_published_pages
                    .insert(key, page);
            } else {
                discard_virtual_terrain_mesh(&mut self.virtual_terrain_arena, page.mesh);
            }
        }
        Ok(true)
    }

    /// Retains only pages that can contribute to the current or next certified ownership cut.
    ///
    /// Immutable encoded pages remain in the shell's memory cache and directory nodes remain on
    /// the GPU, so an evicted page can be restored without rebuilding hierarchy metadata. This
    /// bounds expanded GPU geometry independently from the much smaller encoded-page budget.
    pub fn retain_virtual_terrain_pages(
        &mut self,
        keep: impl IntoIterator<Item = TerrainPageKey>,
    ) -> Result<usize, VirtualTerrainRendererError> {
        let mut keep = keep.into_iter().collect::<BTreeSet<_>>();
        if let Some(cut) = &self.virtual_terrain_cut {
            keep.extend(cut.selected_pages.iter().copied());
        }
        if let Some(cut) = &self.virtual_terrain_oracle_cut {
            keep.extend(cut.selected_pages.iter().copied());
        }
        if let Some(publication) = &self.virtual_terrain_publication {
            keep.extend(publication.cut.selected_pages.iter().copied());
        }
        keep.extend(self.virtual_terrain_staging_frontier.iter().copied());
        // Keep exactly one nearest real ancestor per active/candidate page. That is sufficient for
        // the conforming coarsen fallback if a neighboring replacement is incomplete; retaining
        // the entire expanded chain wastes the source budget, while retaining none makes the next
        // refinement arrival oscillate between conforming and unrepairable cuts.
        let ancestry_sources = keep.iter().copied().collect::<Vec<_>>();
        for key in ancestry_sources {
            let mut ancestor = key.parent();
            while let Some(parent) = ancestor {
                if self.virtual_terrain_pages.contains_key(&parent) {
                    keep.insert(parent);
                    break;
                }
                ancestor = parent.parent();
            }
        }
        // Keep constrained ancestor samples for rebuilding conforming child boundaries without
        // retaining unrelated travel history.
        let mut sample_keep = BTreeSet::new();
        for key in keep.iter().copied() {
            let mut ancestor = key.parent();
            while let Some(parent) = ancestor {
                sample_keep.insert(parent);
                ancestor = parent.parent();
            }
        }
        self.virtual_terrain_heightfield_samples
            .retain(|key, _| key.level > 0 && sample_keep.contains(key));
        let remove = self
            .virtual_terrain_pages
            .keys()
            .filter(|key| !keep.contains(key))
            .copied()
            .collect::<Vec<_>>();
        for key in &remove {
            if !self.virtual_terrain.remove_page(*key) {
                continue;
            }
            self.virtual_terrain_gpu.remove_page_geometry(*key);
            if let Some(page) = self.virtual_terrain_pages.remove(key) {
                discard_virtual_terrain_mesh(&mut self.virtual_terrain_arena, page.mesh);
            }
        }
        Ok(remove.len())
    }

    fn discard_retired_virtual_terrain_pages(&mut self) {
        let retired = std::mem::take(&mut self.virtual_terrain_retired_published_pages);
        for (_, page) in retired {
            discard_virtual_terrain_mesh(&mut self.virtual_terrain_arena, page.mesh);
        }
    }

    fn ensure_virtual_terrain_gpu_allocation(
        &mut self,
        requested_bytes: u32,
    ) -> Result<(), VirtualTerrainRendererError> {
        if requested_bytes == 0 || self.virtual_terrain_arena.can_allocate(requested_bytes) {
            return Ok(());
        }
        self.retain_virtual_terrain_pages(std::iter::empty())?;
        if self.virtual_terrain_arena.can_allocate(requested_bytes) {
            return Ok(());
        }
        let arena = self.virtual_terrain_arena.stats();
        let allocation_bytes = self
            .virtual_terrain_arena
            .aligned_allocation_size(requested_bytes)
            .ok_or(VirtualTerrainRendererError::GpuPoolCapacity)?;
        if u64::from(allocation_bytes) > arena.free_bytes {
            return Err(VirtualTerrainRendererError::GpuPoolCapacity);
        }
        debug_assert!(u64::from(allocation_bytes) > arena.largest_free_range_bytes);
        Err(VirtualTerrainRendererError::GpuPoolCapacity)
    }

    /// Resident page count, encoded CPU bytes, primitive count, GPU capacity, and GPU allocation.
    pub fn virtual_terrain_usage(&self) -> (usize, usize, usize, u64, u64) {
        let (pages, encoded_bytes, primitives) = self.virtual_terrain.resident_usage();
        let gpu = self.virtual_terrain_arena.stats();
        let handle_capacity = self.virtual_terrain_gpu.handle_bank_capacity_bytes();
        let handle_allocated = self.virtual_terrain_gpu.allocated_handle_bytes();
        (
            pages,
            encoded_bytes,
            primitives,
            gpu.capacity_bytes.saturating_add(handle_capacity),
            gpu.allocated_bytes.saturating_add(handle_allocated),
        )
    }

    fn virtual_terrain_cut_fits_snapshot(
        &self,
        cut: &VirtualTerrainCut,
    ) -> Result<(), VirtualTerrainRendererError> {
        let [
            surface_bytes,
            triangle_bytes,
            water_surface_bytes,
            water_triangle_bytes,
        ] = self.virtual_terrain_cut_snapshot_source_bytes(cut)?;
        if surface_bytes > VIRTUAL_TERRAIN_SURFACE_HANDLE_SOURCE_BYTES
            || triangle_bytes > VIRTUAL_TERRAIN_TRIANGLE_HANDLE_SOURCE_BYTES
            || water_surface_bytes > VIRTUAL_TERRAIN_WATER_SURFACE_HANDLE_SOURCE_BYTES
            || water_triangle_bytes > VIRTUAL_TERRAIN_WATER_TRIANGLE_HANDLE_SOURCE_BYTES
        {
            return Err(VirtualTerrainRendererError::SelectedCutSnapshotCapacity {
                required_source_bytes: [
                    surface_bytes,
                    triangle_bytes,
                    water_surface_bytes,
                    water_triangle_bytes,
                ],
            });
        }
        Ok(())
    }

    fn virtual_terrain_cut_snapshot_source_bytes(
        &self,
        cut: &VirtualTerrainCut,
    ) -> Result<[u64; 4], VirtualTerrainRendererError> {
        let mut surface_bytes = 0u64;
        let mut triangle_bytes = 0u64;
        let mut water_surface_bytes = 0u64;
        let mut water_triangle_bytes = 0u64;
        for key in &cut.selected_pages {
            let page = self
                .virtual_terrain_pages
                .get(key)
                .ok_or(VirtualTerrainRendererError::SelectedPageMissingGpu(*key))?;
            match &page.mesh {
                VirtualTerrainGpuMesh::Empty => {}
                VirtualTerrainGpuMesh::Surface(mesh) => {
                    for slice in &mesh.slices {
                        let bytes = u64::from(slice.quad_count) * size_of::<GpuQuad>() as u64;
                        match slice.render_layer {
                            RenderLayer::Opaque => {
                                surface_bytes = surface_bytes.saturating_add(bytes);
                            }
                            RenderLayer::Translucent => {
                                water_surface_bytes = water_surface_bytes.saturating_add(bytes);
                            }
                            RenderLayer::Empty => {}
                        }
                    }
                }
                VirtualTerrainGpuMesh::Triangle(mesh) => {
                    triangle_bytes = triangle_bytes.saturating_add(
                        u64::from(mesh.opaque_vertex_count) * size_of::<GpuTerrainVertex>() as u64,
                    );
                    water_triangle_bytes = water_triangle_bytes.saturating_add(
                        u64::from(mesh.water_vertex_count) * size_of::<GpuTerrainVertex>() as u64,
                    );
                }
            }
        }
        Ok([
            surface_bytes,
            triangle_bytes,
            water_surface_bytes,
            water_triangle_bytes,
        ])
    }

    pub fn upload_chunk(&mut self, chunk: &Chunk, mesh: &MeshedChunk) -> bool {
        self.upload_chunks_atomic(std::iter::once((chunk, mesh)))
    }

    /// Publishes one complete canonical edit cut.
    ///
    /// All replacement allocations and queue writes are prepared before any resident directory
    /// entry changes. Allocation failure therefore leaves the previous complete cut visible; a
    /// successful call switches every opaque/translucent chunk and its derived lighting/profile
    /// metadata in one CPU transaction before the next command encoder is built.
    pub fn upload_chunks_atomic<'a>(
        &mut self,
        chunks: impl IntoIterator<Item = (&'a Chunk, &'a MeshedChunk)>,
    ) -> bool {
        let mut prepared = Vec::new();
        for (chunk, mesh) in chunks {
            let Some(upload) = self.prepare_canonical_chunk_upload(chunk, mesh) else {
                for upload in prepared {
                    self.discard_canonical_chunk_upload(upload);
                }
                return false;
            };
            prepared.push(upload);
        }
        for upload in prepared {
            self.commit_canonical_chunk_upload(upload);
        }
        true
    }

    fn prepare_canonical_chunk_upload(
        &mut self,
        chunk: &Chunk,
        mesh: &MeshedChunk,
    ) -> Option<PreparedCanonicalChunkUpload> {
        let coord = chunk.coord();
        let key = (0, coord.x, coord.y, coord.z);
        let origin = coord.world_origin();
        let convert = |quad: &Quad| GpuQuad {
            origin: [
                origin[0] + i32::from(quad.origin[0]),
                origin[1] + i32::from(quad.origin[1]),
                origin[2] + i32::from(quad.origin[2]),
            ],
            extent_voxels: quad.extent.map(u16::from),
            material_face: pack_gpu_material_face(u32::from(quad.material), quad.face),
            ao: u32::from(quad.ao),
        };
        let opaque_quads = canonical_gpu_quads(origin, &mesh.opaque);
        let water_surface_count = mesh
            .translucent
            .iter()
            .filter(|quad| quad.face == 2)
            .count() as u32;
        let water_quads: Vec<_> = mesh
            .translucent
            .iter()
            .filter(|quad| quad.face == 2)
            .chain(mesh.translucent.iter().filter(|quad| quad.face != 2))
            .map(convert)
            .collect();
        let min = glam::Vec3::from_array(origin.map(|value| value as f32 * VOXEL_SIZE_METRES));
        let max = min + glam::Vec3::splat(CHUNK_EDGE as f32 * VOXEL_SIZE_METRES);
        let quad_bytes = size_of::<GpuQuad>() as u32;
        let opaque_count = opaque_quads.len() as u32;
        let opaque_update = if opaque_count == 0 {
            None
        } else {
            let prepared = self.prepare_mesh_sliced(
                key,
                &opaque_quads,
                vec![MeshSlice {
                    relative_offset: 0,
                    size: opaque_count * quad_bytes,
                    quad_count: opaque_count,
                    bounds_min: min,
                    bounds_max: max,
                    render_layer: RenderLayer::Opaque,
                }],
            )?;
            Some(prepared)
        };
        let translucent_count = mesh.translucent.len() as u32;
        let water_update = if translucent_count == 0 {
            None
        } else {
            let mut slices = Vec::with_capacity(2);
            if water_surface_count != 0 {
                slices.push(MeshSlice {
                    relative_offset: 0,
                    size: water_surface_count * quad_bytes,
                    quad_count: water_surface_count,
                    bounds_min: min,
                    bounds_max: max,
                    render_layer: RenderLayer::Translucent,
                });
            }
            let volume_count = translucent_count - water_surface_count;
            if volume_count != 0 {
                slices.push(MeshSlice {
                    relative_offset: water_surface_count * quad_bytes,
                    size: volume_count * quad_bytes,
                    quad_count: volume_count,
                    bounds_min: min,
                    bounds_max: max,
                    render_layer: RenderLayer::Translucent,
                });
            }
            let Some(prepared) = self.prepare_water_mesh_sliced(key, &water_quads, slices) else {
                discard_prepared_mesh(&mut self.arena, opaque_update);
                return None;
            };
            Some(prepared)
        };
        Some(PreparedCanonicalChunkUpload {
            key,
            opaque: opaque_update,
            translucent: water_update,
            local_lights: local_lights_for_mesh(origin, mesh),
        })
    }

    fn discard_canonical_chunk_upload(&mut self, upload: PreparedCanonicalChunkUpload) {
        discard_prepared_mesh(&mut self.arena, upload.opaque);
        discard_prepared_mesh(&mut self.water_arena, upload.translucent);
    }

    fn commit_canonical_chunk_upload(&mut self, upload: PreparedCanonicalChunkUpload) {
        commit_prepared_mesh(&mut self.arena, &mut self.chunks, upload.key, upload.opaque);
        commit_prepared_mesh(
            &mut self.water_arena,
            &mut self.water_chunks,
            upload.key,
            upload.translucent,
        );
        if upload.local_lights.is_empty() {
            self.local_light_candidates.remove(&upload.key);
        } else {
            self.local_light_candidates
                .insert(upload.key, upload.local_lights);
        }
    }

    fn prepare_mesh_sliced(
        &mut self,
        key: MeshKey,
        gpu_quads: &[GpuQuad],
        slices: Vec<MeshSlice>,
    ) -> Option<ChunkMesh> {
        let activation_mask = self.chunk_activations.upload_mask(key);
        prepare_mesh_sliced_into(
            &self.device,
            &self.queue,
            &mut self.arena,
            &mut self.arena_buffers,
            gpu_quads,
            slices,
            activation_mask,
            "opaque voxel mesh arena page",
        )
    }

    fn prepare_water_mesh_sliced(
        &mut self,
        key: MeshKey,
        gpu_quads: &[GpuQuad],
        slices: Vec<MeshSlice>,
    ) -> Option<ChunkMesh> {
        let activation_mask = self.chunk_activations.upload_mask(key);
        prepare_mesh_sliced_into(
            &self.device,
            &self.queue,
            &mut self.water_arena,
            &mut self.water_arena_buffers,
            gpu_quads,
            slices,
            activation_mask,
            "water mesh arena page",
        )
    }

    pub fn remove_chunk(&mut self, coord: ChunkCoord) {
        let key = (0, coord.x, coord.y, coord.z);
        self.remove_chunk_mesh(key);
        self.chunk_activations.remove(key);
    }

    /// Whether one published owner reconstructs this coordinate on the canonical 10 cm lattice.
    pub fn canonical_lattice_presented(&self, voxel_x: i32, voxel_y: i32, voxel_z: i32) -> bool {
        let chunk = (
            voxel_x.div_euclid(CHUNK_EDGE as i32),
            voxel_y.div_euclid(CHUNK_EDGE as i32),
            voxel_z.div_euclid(CHUNK_EDGE as i32),
        );
        if self.canonical_ready_chunks.contains(&chunk)
            || self.canonical_surface_ready_chunks.contains(&chunk)
            || self.enclosed_view_ready_chunks.contains(&chunk)
        {
            return true;
        }
        let leaf = TerrainPageKey::surface(0, chunk.0, chunk.2);
        self.virtual_terrain_mode == VirtualTerrainRenderMode::Visible
            && self.virtual_terrain_cut.as_ref().is_some_and(|cut| {
                cut.selected_pages.iter().any(|selected| {
                    selected.is_surface() && leaf.ancestor_at(selected.level) == Some(*selected)
                })
            })
    }

    /// Number of horizontal cells owned by the currently active exact canonical vertical band.
    ///
    /// Ownership follows transactional chunk readiness, not the presence of a top-surface sample:
    /// a dug shaft is valid empty canonical space and must not resurrect its surface parent.
    pub fn terrain_column_ownership_at(&self, voxel_x: i32, voxel_z: i32) -> (u16, u16) {
        let column = (
            voxel_x.div_euclid(CHUNK_EDGE as i32),
            voxel_z.div_euclid(CHUNK_EDGE as i32),
        );
        let virtual_leaf = TerrainPageKey::surface(0, column.0, column.1);
        if self.virtual_terrain_mode == VirtualTerrainRenderMode::Visible
            && self.virtual_terrain_cut.as_ref().is_some_and(|cut| {
                cut.selected_pages.iter().any(|selected| {
                    selected.is_surface()
                        && virtual_leaf.ancestor_at(selected.level) == Some(*selected)
                })
            })
        {
            let required = (CHUNK_EDGE * CHUNK_EDGE) as u16;
            return (required, required);
        }
        let covered = if self
            .canonical_surface_ready_chunks
            .iter()
            .any(|&(x, _, z)| (x, z) == column)
        {
            CHUNK_EDGE * CHUNK_EDGE
        } else {
            0
        };
        (covered as u16, (CHUNK_EDGE * CHUNK_EDGE) as u16)
    }

    fn remove_mesh(&mut self, key: MeshKey) {
        self.remove_opaque_mesh(key);
        self.remove_water_mesh(key);
    }

    fn remove_chunk_mesh(&mut self, key: MeshKey) {
        self.remove_mesh(key);
        self.local_light_candidates.remove(&key);
    }

    fn remove_opaque_mesh(&mut self, key: MeshKey) {
        if let Some(chunk) = self.chunks.remove(&key) {
            let _ = self.arena.free(chunk.allocation);
        }
    }

    fn remove_water_mesh(&mut self, key: MeshKey) {
        if let Some(chunk) = self.water_chunks.remove(&key) {
            let _ = self.water_arena.free(chunk.allocation);
        }
    }

    fn selected_local_lights(
        &self,
        camera: &CameraState,
        mut visibility: impl FnMut([f32; 3], f32) -> LocalLightVisibility,
    ) -> (LocalLightUniform, u32, u32, u32) {
        let mut uniform = LocalLightUniform::default();
        let enabled = self.options.local_lighting;
        let mut ranked =
            [(f32::NEG_INFINITY, GpuLocalLight::default()); MAX_LOCAL_LIGHT_VISIBILITY_TESTS];
        let mut ranked_count = 0usize;
        let mut candidates = 0u32;
        let mut in_range = 0u32;
        let mut occluded = 0u32;
        let mut portal_rejected = 0u32;
        let mut visibility_tests = 0u32;
        for (key, lights) in &self.local_light_candidates {
            if !self.chunks.get(key).is_some_and(ChunkMesh::active) {
                continue;
            }
            for light in lights {
                candidates = candidates.saturating_add(1);
                if !enabled {
                    continue;
                }
                let position = glam::Vec3::from_array([
                    light.position_radius[0],
                    light.position_radius[1],
                    light.position_radius[2],
                ]);
                let distance_squared = position.distance_squared(camera.position);
                let selection_radius = light.position_radius[3] * 2.0;
                if distance_squared > selection_radius * selection_radius {
                    continue;
                }
                in_range = in_range.saturating_add(1);
                let score = light.color_intensity[3] / distance_squared.max(0.15 * 0.15);
                rank_local_light(&mut ranked, &mut ranked_count, score, *light);
            }
        }
        let mut selected = 0usize;
        for (_, light) in ranked.into_iter().take(ranked_count) {
            visibility_tests = visibility_tests.saturating_add(1);
            match visibility(
                [
                    light.position_radius[0],
                    light.position_radius[1],
                    light.position_radius[2],
                ],
                light.position_radius[3] * 2.0,
            ) {
                LocalLightVisibility::Visible => {}
                LocalLightVisibility::Occluded => {
                    occluded = occluded.saturating_add(1);
                    continue;
                }
                LocalLightVisibility::PortalRejected => {
                    portal_rejected = portal_rejected.saturating_add(1);
                    continue;
                }
            }
            uniform.lights[selected] = light;
            selected += 1;
            if selected == MAX_ACTIVE_LOCAL_LIGHTS {
                break;
            }
        }
        uniform.metadata = [
            selected as u32,
            candidates,
            in_range
                .saturating_sub(selected as u32)
                .saturating_sub(occluded)
                .saturating_sub(portal_rejected),
            u32::from(enabled),
        ];
        (uniform, occluded, portal_rejected, visibility_tests)
    }

    /// Encodes and submits one frame, returning `false` when the surface could not be presented.
    #[must_use]
    pub fn render(
        &mut self,
        frame_id: u32,
        dt: f32,
        camera: &CameraState,
        ui_stats: LiveStats,
        local_light_visibility: impl FnMut([f32; 3], f32) -> LocalLightVisibility,
        mut now_ms: impl FnMut() -> f64,
    ) -> bool {
        let dt = bounded_frame_delta(dt);
        self.time += dt;
        self.observer_world_xz_metres =
            [f64::from(camera.position.x), f64::from(camera.position.z)];
        if !self.refresh_effective_environment() {
            return false;
        }
        let interior_seconds = if self.interior_target.enclosure > self.interior.enclosure {
            0.25
        } else {
            0.45
        };
        let interior_response = 1.0 - (-dt / interior_seconds).exp();
        let exposure_seconds =
            if self.interior_target.exposure_multiplier > self.interior.exposure_multiplier {
                2.5
            } else {
                0.45
            };
        let exposure_response = 1.0 - (-dt / exposure_seconds).exp();
        self.interior =
            self.interior
                .lerp(self.interior_target, interior_response, exposure_response);
        let direct_light_visibility = interior_direct_light_visibility(
            self.interior.enclosure,
            self.directional_light_occluded,
        );
        let shadows_active = self.options.shadows
            && self.environment.shadow_strength > 0.01
            && direct_light_visibility > 0.01;
        let mut frame_options = self.options;
        frame_options.shadows = shadows_active;
        if self
            .shadow_direction
            .update(-self.environment.key_light_direction)
            .is_err()
        {
            return false;
        }
        let Ok(shadow_cascades) = directional_shadow_cascades(
            &self.config,
            camera,
            self.shadow_direction.basis(),
            self.runtime_config.directional_shadows,
        ) else {
            return false;
        };
        self.ui.set_stats(ui_stats);
        self.ui.advance(dt);
        let ui_draw = self.ui.build_draw_list(self.ui_viewport());
        if let Err(error) = self.ui_gpu.prepare(&self.device, &self.queue, &ui_draw)
            && !self.ui_text_error_reported
        {
            (self.log_error)(&error);
            self.ui_text_error_reported = true;
        }
        let (
            local_lights,
            occluded_local_lights,
            portal_rejected_local_lights,
            local_light_visibility_tests,
        ) = self.selected_local_lights(camera, local_light_visibility);
        self.queue.write_buffer(
            &self.local_light_buffer,
            0,
            bytemuck::bytes_of(&local_lights),
        );
        let uniform = frame_uniform(
            &self.config,
            camera,
            self.time,
            self.target_volume,
            FrameState {
                options: frame_options,
                geometry_source_debug: self.geometry_source_debug,
                environment: self.environment,
                world_environment: self.world_environment,
                celestial_observation: self.celestial_observation,
                interior: self.interior,
                direct_light_visibility,
            },
            &shadow_cascades,
            self.runtime_config,
        );
        let view_projection = glam::Mat4::from_cols_array_2d(&uniform.view_projection);
        let view_clip = AabbClipVolume::new(view_projection);
        let shadow_clips = shadow_cascades
            .cascades
            .map(|cascade| AabbClipVolume::new(cascade.clip_from_world));
        let cull_started = now_ms();
        let (virtual_visible, virtual_ownership) =
            if self.virtual_terrain_mode == VirtualTerrainRenderMode::Visible {
                let Some((cut, envelope)) = self
                    .virtual_terrain_cut
                    .as_ref()
                    .zip(self.virtual_terrain_committed_envelope.as_ref())
                    .filter(|(cut, envelope)| {
                        cut.is_renderable() && cut.covers_presentation_envelope(envelope)
                    })
                else {
                    return false;
                };
                if !self
                    .virtual_terrain_gpu
                    .presented_snapshot_matches(virtual_terrain_snapshot_identity(cut, envelope))
                {
                    // A visible frame has exactly one terrain generation. Never combine a CPU
                    // ownership cut or diagnostic sidecar with handles from another bank.
                    return false;
                }
                let Ok(ownership) = VirtualTerrainOwnership::from_cut(cut) else {
                    return false;
                };
                (true, ownership)
            } else {
                (false, VirtualTerrainOwnership::default())
            };
        let virtual_candidate_work =
            self.virtual_terrain_publication
                .as_ref()
                .and_then(|publication| {
                    self.virtual_terrain_gpu
                        .candidate_work(virtual_terrain_snapshot_identity(
                            &publication.cut,
                            &publication.envelope,
                        ))
                });
        let (shadow_draw_lists, world_draw_list) = collect_opaque_draw_lists(
            &mut self.chunks,
            shadows_active,
            view_clip,
            shadow_clips,
            &virtual_ownership,
        );
        let virtual_world_draw_lists = if virtual_visible {
            let Ok(draw_lists) = self.collect_virtual_terrain_draw_list(view_clip) else {
                return false;
            };
            draw_lists
        } else {
            VirtualTerrainDrawLists::default()
        };
        let water_draw_list = self.collect_draw_list(
            &self.water_chunks,
            |key, chunk| {
                self.options.water
                    && key.0 == 0
                    && view_clip.contains_aabb(chunk.bounds_min, chunk.bounds_max)
            },
            |_key, slice| {
                slice.render_layer == RenderLayer::Translucent
                    && !virtual_ownership.covers_aabb(slice.bounds_min, slice.bounds_max)
                    && view_clip.contains_aabb(slice.bounds_min, slice.bounds_max)
            },
        );
        let cpu_cull_ms = (now_ms() - cull_started).max(0.0) as f32;
        let encode_started = now_ms();
        self.avatar_gpu
            .prepare(&self.queue, &self.remote_avatars, self.time);
        let avatar_instances = self.avatar_gpu.instance_count();
        let has_avatars = avatar_instances != 0;
        let refract_water = self.options.water
            && (!water_draw_list.spans.is_empty()
                || (virtual_visible
                    && (virtual_world_draw_lists.water_surfaces.quad_count > 0
                        || virtual_world_draw_lists.water_triangles.vertex_count > 0)));
        let diagnostic_sky = self.runtime_config.diagnostic_sky_color.is_some();
        let diagnostic_geometry = self.geometry_source_debug;
        let clouds_active =
            self.volumetric_cloud_gpu.enabled() && !diagnostic_sky && !diagnostic_geometry;
        let weather_active =
            self.environment.precipitation > 0.002 && !diagnostic_sky && !diagnostic_geometry;
        self.queue
            .write_buffer(&self.frame_buffer, 0, bytemuck::bytes_of(&uniform));
        self.volumetric_cloud_gpu
            .update(&self.queue, self.world_environment, self.environment);
        if shadows_active {
            self.shadow_gpu
                .write_cascades(&self.queue, &shadow_cascades, camera);
        }
        let frame = match self.surface.get_current_texture() {
            CurrentSurfaceTexture::Success(frame) | CurrentSurfaceTexture::Suboptimal(frame) => {
                frame
            }
            CurrentSurfaceTexture::Outdated | CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return false;
            }
            _ => return false,
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame encoder"),
            });
        let virtual_candidate = if virtual_candidate_work.is_some() {
            let Some(publication) = self.virtual_terrain_publication.as_ref() else {
                return false;
            };
            Some(virtual_terrain_snapshot_identity(
                &publication.cut,
                &publication.envelope,
            ))
        } else {
            None
        };
        let mut gpu_frame = self.gpu_timer.as_mut().and_then(|timer| {
            timer.begin_frame(
                frame_id,
                GpuPassMask {
                    shadows: shadows_active,
                    water: refract_water,
                    ambient_occlusion: self.options.screen_space_ambient_occlusion,
                    clouds: clouds_active,
                    weather: weather_active,
                    virtual_terrain: virtual_candidate_work
                        == Some(VirtualTerrainCandidateWork::Encode),
                },
            )
        });
        let mut virtual_candidate_encode_failed = false;
        let mut virtual_candidate_generation_to_submit = None;
        if let (Some(identity), Some(work)) = (virtual_candidate, virtual_candidate_work) {
            let timestamps = (work == VirtualTerrainCandidateWork::Encode)
                .then(|| {
                    gpu_frame
                        .as_ref()
                        .map(|frame| VirtualTerrainGpuTimestampWrites {
                            query_set: &frame.query_set,
                            encoding_first_query: 24,
                            validation_first_query: 26,
                        })
                })
                .flatten();
            match self.virtual_terrain_gpu.encode_candidate(
                &self.queue,
                &mut encoder,
                identity,
                timestamps,
            ) {
                Ok(VirtualTerrainCandidateEncodeOutcome::Encoded(generation)) => {
                    debug_assert_eq!(work, VirtualTerrainCandidateWork::Encode);
                    virtual_candidate_generation_to_submit = Some(generation);
                }
                Ok(VirtualTerrainCandidateEncodeOutcome::ReadbackOnly(generation)) => {
                    debug_assert_eq!(work, VirtualTerrainCandidateWork::ReadbackOnly);
                    virtual_candidate_generation_to_submit = Some(generation);
                }
                Err(_) => {
                    if let (Some(timer), Some(frame)) = (self.gpu_timer.as_ref(), gpu_frame.take())
                    {
                        timer.cancel_frame(frame);
                    }
                    virtual_candidate_encode_failed = true;
                    self.abort_virtual_terrain_publication();
                    if !can_present_after_candidate_encode_failure(virtual_visible) {
                        return false;
                    }
                }
            }
        }
        let mut shadow_draw_calls = 0u32;
        if shadows_active {
            for (cascade_index, draw_list) in shadow_draw_lists.iter().enumerate() {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("sun shadow cascade pass"),
                    color_attachments: &[],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &self.shadow_gpu.layer_views[cascade_index],
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: gpu_frame
                        .as_ref()
                        .map(|frame| frame.pass(cascade_index as u32 * 2)),
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_bind_group(0, &self.shadow_gpu.bind_groups[cascade_index], &[]);
                pass.set_pipeline(&self.shadow_gpu.fixed_pipeline);
                shadow_draw_calls = shadow_draw_calls.saturating_add(draw_spans(
                    &mut pass,
                    &self.arena_buffers,
                    &draw_list.fixed,
                ));
                if virtual_visible {
                    pass.set_bind_group(
                        1,
                        self.virtual_terrain_gpu.active_render_bind_group(),
                        &[],
                    );
                    pass.set_pipeline(&self.shadow_gpu.virtual_surface_pipeline);
                    pass.draw_indirect(
                        self.virtual_terrain_gpu.active_indirect_buffer(),
                        VIRTUAL_TERRAIN_SURFACE_INDIRECT_OFFSET,
                    );
                    pass.set_pipeline(&self.shadow_gpu.virtual_triangle_pipeline);
                    pass.draw_indirect(
                        self.virtual_terrain_gpu.active_indirect_buffer(),
                        VIRTUAL_TERRAIN_TRIANGLE_INDIRECT_OFFSET,
                    );
                    shadow_draw_calls = shadow_draw_calls.saturating_add(2);
                }
                if has_avatars {
                    self.avatar_gpu.draw_shadow(&mut pass);
                    shadow_draw_calls += 1;
                }
            }
        }
        let mut depth_prepass_draw_calls = 0u32;
        if self.options.screen_space_ambient_occlusion {
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("spatial AO depth ownership pass"),
                    color_attachments: &[],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: self.depth.view(),
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(0.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: gpu_frame.as_ref().map(|frame| frame.pass(6)),
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_bind_group(0, &self.frame_bind_group, &[]);
                if virtual_visible {
                    pass.set_bind_group(
                        3,
                        self.virtual_terrain_gpu.active_render_bind_group(),
                        &[],
                    );
                    pass.set_pipeline(&self.virtual_surface_depth_pipeline);
                    pass.draw_indirect(
                        self.virtual_terrain_gpu.active_indirect_buffer(),
                        VIRTUAL_TERRAIN_SURFACE_INDIRECT_OFFSET,
                    );
                    pass.set_pipeline(&self.virtual_triangle_depth_pipeline);
                    pass.draw_indirect(
                        self.virtual_terrain_gpu.active_indirect_buffer(),
                        VIRTUAL_TERRAIN_TRIANGLE_INDIRECT_OFFSET,
                    );
                    depth_prepass_draw_calls = depth_prepass_draw_calls.saturating_add(2);
                }
                pass.set_pipeline(&self.depth_prepass_fast_pipeline);
                depth_prepass_draw_calls = depth_prepass_draw_calls.saturating_add(draw_spans(
                    &mut pass,
                    &self.arena_buffers,
                    &world_draw_list.fixed,
                ));
                if has_avatars {
                    self.avatar_gpu.draw_depth(&mut pass);
                    depth_prepass_draw_calls = depth_prepass_draw_calls.saturating_add(1);
                }
            }
            self.ambient_occlusion_gpu.evaluate(
                &mut encoder,
                &self.frame_bind_group,
                gpu_frame.as_ref().map(|frame| frame.pass(8)),
            );
            self.ambient_occlusion_gpu.denoise(
                &mut encoder,
                &self.frame_bind_group,
                gpu_frame.as_ref().map(|frame| frame.pass(10)),
            );
        }
        if clouds_active {
            self.volumetric_cloud_gpu.trace(
                &mut encoder,
                &self.frame_bind_group,
                gpu_frame.as_ref().map(|frame| frame.pass(12)),
            );
        }
        let opaque_scene_view = if refract_water {
            self.ui_gpu.opaque_scene_view()
        } else {
            self.ui_gpu.scene_view()
        };
        let (screenshot_opaque_owners, screenshot_virtual_opaque_owners, screenshot_water_owners) =
            if self.screenshot_requested && self.geometry_source_debug {
                let Some(opaque) = screenshot_diagnostic_owner_buffers(
                    &self.device,
                    &self.queue,
                    &self.arena_buffers,
                    &self.chunks,
                    "screenshot opaque terrain owner sidecar",
                ) else {
                    if let (Some(timer), Some(frame)) = (self.gpu_timer.as_ref(), gpu_frame.take())
                    {
                        timer.cancel_frame(frame);
                    }
                    return false;
                };
                let virtual_opaque = if virtual_visible {
                    let Some(owners) = screenshot_virtual_terrain_owner_buffers(
                        &self.device,
                        &self.queue,
                        &self.virtual_terrain_arena_buffers,
                        &self.virtual_terrain_pages,
                        &self.virtual_terrain_retired_published_pages,
                        "screenshot virtual terrain owner sidecar",
                    ) else {
                        if let (Some(timer), Some(frame)) =
                            (self.gpu_timer.as_ref(), gpu_frame.take())
                        {
                            timer.cancel_frame(frame);
                        }
                        return false;
                    };
                    Some(owners)
                } else {
                    None
                };
                let Some(water) = screenshot_diagnostic_owner_buffers(
                    &self.device,
                    &self.queue,
                    &self.water_arena_buffers,
                    &self.water_chunks,
                    "screenshot water terrain owner sidecar",
                ) else {
                    if let (Some(timer), Some(frame)) = (self.gpu_timer.as_ref(), gpu_frame.take())
                    {
                        timer.cancel_frame(frame);
                    }
                    return false;
                };
                (Some(opaque), virtual_opaque, Some(water))
            } else if self.screenshot_requested {
                // Ordinary F2 captures retain the complete reproduction/cut metadata without
                // allocating and uploading a second owner stream for every resident primitive.
                // Geometry-source debug captures opt into that expensive pixel attachment.
                (
                    Some(Vec::new()),
                    virtual_visible.then_some(Vec::new()),
                    Some(Vec::new()),
                )
            } else {
                (None, None, None)
            };
        let screenshot_target = self.screenshot_requested.then(|| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("screenshot composite target"),
                size: wgpu::Extent3d {
                    width: self.config.width,
                    height: self.config.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.config.format,
                usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        });
        let screenshot_diagnostic_identity_target =
            (self.screenshot_requested && self.geometry_source_debug).then(|| {
                self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("screenshot integer ownership target"),
                    size: wgpu::Extent3d {
                        width: self.config.width,
                        height: self.config.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: TextureFormat::Rgba32Uint,
                    usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
                    view_formats: &[],
                })
            });
        let screenshot_diagnostic_depth_target =
            (self.screenshot_requested && self.geometry_source_debug).then(|| {
                self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("screenshot exact reverse-z target"),
                    size: wgpu::Extent3d {
                        width: self.config.width,
                        height: self.config.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: TextureFormat::R32Uint,
                    usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
                    view_formats: &[],
                })
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("opaque world pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: opaque_scene_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: self.depth.view(),
                    depth_ops: Some(wgpu::Operations {
                        load: if self.options.screen_space_ambient_occlusion {
                            wgpu::LoadOp::Load
                        } else {
                            wgpu::LoadOp::Clear(0.0)
                        },
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: gpu_frame.as_ref().map(|frame| frame.pass(14)),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.frame_bind_group, &[]);
            pass.set_bind_group(2, self.ambient_occlusion_gpu.sample_bind_group(), &[]);
            let fixed_pipeline = if self.options.screen_space_ambient_occlusion {
                if self.options.material_detail {
                    &self.voxel_ambient_occlusion_pipeline
                } else {
                    &self.voxel_ambient_occlusion_flat_pipeline
                }
            } else if self.options.material_detail {
                &self.voxel_pipeline
            } else {
                &self.voxel_flat_pipeline
            };
            if virtual_visible {
                let virtual_surface_pipeline = if self.options.screen_space_ambient_occlusion {
                    if self.options.material_detail {
                        &self.virtual_surface_ambient_occlusion_pipeline
                    } else {
                        &self.virtual_surface_ambient_occlusion_flat_pipeline
                    }
                } else if self.options.material_detail {
                    &self.virtual_surface_pipeline
                } else {
                    &self.virtual_surface_flat_pipeline
                };
                let virtual_triangle_pipeline = if self.options.screen_space_ambient_occlusion {
                    if self.options.material_detail {
                        &self.virtual_triangle_ambient_occlusion_pipeline
                    } else {
                        &self.virtual_triangle_ambient_occlusion_flat_pipeline
                    }
                } else if self.options.material_detail {
                    &self.virtual_triangle_pipeline
                } else {
                    &self.virtual_triangle_flat_pipeline
                };
                pass.set_bind_group(3, self.virtual_terrain_gpu.active_render_bind_group(), &[]);
                pass.set_pipeline(virtual_surface_pipeline);
                pass.draw_indirect(
                    self.virtual_terrain_gpu.active_indirect_buffer(),
                    VIRTUAL_TERRAIN_SURFACE_INDIRECT_OFFSET,
                );
                pass.set_pipeline(virtual_triangle_pipeline);
                pass.draw_indirect(
                    self.virtual_terrain_gpu.active_indirect_buffer(),
                    VIRTUAL_TERRAIN_TRIANGLE_INDIRECT_OFFSET,
                );
            }
            pass.set_pipeline(fixed_pipeline);
            draw_spans(&mut pass, &self.arena_buffers, &world_draw_list.fixed);
            self.avatar_gpu
                .draw_scene(&mut pass, self.options.screen_space_ambient_occlusion);
            // Draw the fullscreen sky at the far plane after opaque geometry so early depth
            // rejection avoids running its procedural clouds behind terrain.
            pass.set_pipeline(&self.sky_pipeline);
            pass.draw(0..3, 0..1);
        }
        if clouds_active {
            self.volumetric_cloud_gpu.composite(
                &mut encoder,
                &self.frame_bind_group,
                opaque_scene_view,
                self.depth.view(),
                if refract_water || weather_active {
                    wgpu::StoreOp::Store
                } else {
                    wgpu::StoreOp::Discard
                },
                gpu_frame.as_ref().map(|frame| frame.pass(16)),
            );
        }
        if refract_water {
            self.ui_gpu.copy_opaque_to_scene(&mut encoder);
            // Water samples the opaque depth for screen-space refraction while writing its own
            // depth for the later precipitation pass. WebGPU forbids sampling a writable depth
            // attachment in the same pass, so preserve the pre-water depth in a separate texture.
            self.depth.copy_to(&mut encoder, &self.opaque_depth);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("refractive water color pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: self.ui_gpu.scene_view(),
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: self.depth.view(),
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: if weather_active {
                            wgpu::StoreOp::Store
                        } else {
                            wgpu::StoreOp::Discard
                        },
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: gpu_frame.as_ref().map(|frame| frame.pass(18)),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.water_pipeline);
            pass.set_bind_group(0, &self.frame_bind_group, &[]);
            pass.set_bind_group(1, &self.water_scene_bind_group, &[]);
            if virtual_visible {
                pass.set_bind_group(3, self.virtual_terrain_gpu.active_render_bind_group(), &[]);
                pass.set_pipeline(&self.virtual_surface_water_pipeline);
                pass.draw_indirect(
                    self.virtual_terrain_gpu.active_indirect_buffer(),
                    VIRTUAL_TERRAIN_WATER_SURFACE_INDIRECT_OFFSET,
                );
                pass.set_pipeline(&self.virtual_triangle_water_pipeline);
                pass.draw_indirect(
                    self.virtual_terrain_gpu.active_indirect_buffer(),
                    VIRTUAL_TERRAIN_WATER_TRIANGLE_INDIRECT_OFFSET,
                );
            }
            pass.set_pipeline(&self.water_pipeline);
            for span in &water_draw_list.spans {
                let Some(buffer) = self.water_arena_buffers.get(span.page as usize) else {
                    continue;
                };
                let start = u64::from(span.offset);
                let end = start + u64::from(span.size);
                pass.set_vertex_buffer(0, buffer.slice(start..end));
                pass.draw(0..QUAD_VERTEX_COUNT, 0..span.quad_count);
            }
        }
        if weather_active {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("world-space precipitation pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: self.ui_gpu.scene_view(),
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: self.depth.view(),
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: gpu_frame.as_ref().map(|frame| frame.pass(20)),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.weather_pipeline);
            pass.set_bind_group(0, &self.frame_bind_group, &[]);
            pass.draw(0..6, 0..PRECIPITATION_INSTANCE_COUNT);
        }
        let arena = self.arena.stats();
        let water_arena = self.water_arena.stats();
        let virtual_terrain_arena = self.virtual_terrain_arena.stats();
        let scene_pixels = u64::from(self.config.width) * u64::from(self.config.height);
        let shadow_resolution = u64::from(
            self.runtime_config
                .directional_shadows
                .shadow_map_resolution,
        );
        let shadow_bytes = shadow_resolution * shadow_resolution * CASCADE_COUNT as u64 * 4;
        let gpu_timing = self.gpu_timer.as_ref().and_then(GpuTimer::latest);
        let terrain_fingerprint = if virtual_visible {
            fingerprint_value(
                world_draw_list.fingerprint,
                virtual_world_draw_lists.fingerprint,
            )
        } else {
            world_draw_list.fingerprint
        };
        let viewport_fingerprint = fingerprint_value(
            fingerprint_value(
                fingerprint_value(FINGERPRINT_OFFSET, terrain_fingerprint),
                water_draw_list.fingerprint,
            ),
            0,
        );
        let visible_terrain_meshes =
            world_draw_list
                .mesh_count
                .saturating_add(if virtual_visible {
                    virtual_world_draw_lists.mesh_count
                } else {
                    0
                });
        let visible_terrain_draw_calls = world_draw_list
            .fixed
            .spans
            .len()
            .saturating_add(usize::from(virtual_visible) * 2);
        let visible_terrain_primitives =
            world_draw_list
                .quad_count
                .saturating_add(if virtual_visible {
                    virtual_world_draw_lists.primitive_count
                } else {
                    0
                });
        let visible_water_draw_calls = water_draw_list
            .spans
            .len()
            .saturating_add(usize::from(virtual_visible && refract_water) * 2)
            as u32;
        let visible_water_primitives =
            water_draw_list
                .quad_count
                .saturating_add(if virtual_visible {
                    virtual_world_draw_lists
                        .water_surfaces
                        .quad_count
                        .saturating_add(virtual_world_draw_lists.water_triangles.vertex_count / 3)
                } else {
                    0
                });
        let gpu_virtual_feedback = self.virtual_terrain_gpu.latest_feedback();
        let certification_cut = self
            .virtual_terrain_publication
            .as_ref()
            .map(|publication| (&publication.cut, &publication.envelope))
            .or_else(|| {
                self.virtual_terrain_cut
                    .as_ref()
                    .zip(self.virtual_terrain_committed_envelope.as_ref())
            });
        let gpu_virtual_matches_cpu = gpu_virtual_feedback.as_ref().is_some_and(|feedback| {
            gpu_feedback_matches_cut(
                feedback,
                certification_cut.map(|(cut, _)| cut),
                certification_cut.map(|(_, envelope)| envelope),
            )
        });
        let gpu_virtual_match_failure_flags = gpu_feedback_match_failure_flags(
            gpu_virtual_feedback.as_ref(),
            certification_cut.map(|(cut, _)| cut),
            certification_cut.map(|(_, envelope)| envelope),
            virtual_candidate_encode_failed,
        );
        let oracle_virtual_selected_pages = self
            .virtual_terrain_oracle_cut
            .as_ref()
            .map_or(0, |cut| cut.selected_pages.len());
        let oracle_virtual_requested_pages = self
            .virtual_terrain_oracle_cut
            .as_ref()
            .map_or(0, |cut| cut.requested_pages.len());
        let oracle_virtual_refinement_roots = self
            .virtual_terrain_oracle_cut
            .as_ref()
            .map_or(0, |cut| cut.refinement_roots.len());
        let oracle_virtual_ownerless_roots = self
            .virtual_terrain_oracle_cut
            .as_ref()
            .map_or(0, |cut| cut.ownerless_roots.len());
        let oracle_virtual_exact_lod_discontinuities = self
            .virtual_terrain_oracle_cut
            .as_ref()
            .map_or(0, |cut| cut.exact_surface_lod_discontinuities);
        let published_virtual_pages = virtual_visible
            .then_some(self.virtual_terrain_cut.as_ref())
            .flatten()
            .map(|cut| cut.selected_pages.as_slice())
            .unwrap_or_default();
        let published_virtual_ownerless_roots = virtual_visible
            .then_some(self.virtual_terrain_cut.as_ref())
            .flatten()
            .map_or(0, |cut| cut.ownerless_roots.len());
        let published_virtual_exact_pages = published_virtual_pages
            .iter()
            .filter(|key| key.level == 0)
            .count();
        let published_virtual_minimum_level = published_virtual_pages
            .iter()
            .map(|key| key.level)
            .min()
            .unwrap_or(0);
        let published_virtual_maximum_level = published_virtual_pages
            .iter()
            .map(|key| key.level)
            .max()
            .unwrap_or(0);
        let published_virtual_exact_lod_discontinuities = virtual_visible
            .then_some(self.virtual_terrain_cut.as_ref())
            .flatten()
            .map_or(0, |cut| cut.exact_surface_lod_discontinuities);
        let (
            virtual_terrain_exact_domain_complete,
            virtual_terrain_exact_domain_required_leaves,
            virtual_terrain_exact_domain_current_coverage,
            virtual_terrain_exact_domain_fingerprint,
            virtual_terrain_exact_core_complete,
            virtual_terrain_exact_core_required_leaves,
            virtual_terrain_exact_core_current_coverage,
            virtual_terrain_exact_prediction_complete,
            virtual_terrain_exact_prediction_required_leaves,
            virtual_terrain_exact_prediction_current_coverage,
        ) = self.virtual_terrain_exact_surface_domain.as_ref().map_or(
            (false, 0, 0, 0, false, 0, 0, false, 0, 0),
            |domain| {
                let committed = self.virtual_terrain_cut.as_ref();
                (
                    domain.is_complete(),
                    domain.required_leaf_count(),
                    committed.map_or(0, |cut| cut.exact_surface_coverage(domain)),
                    domain.fingerprint(),
                    domain.core_is_complete(),
                    domain.core_required_leaf_count(),
                    committed.map_or(0, |cut| cut.exact_surface_core_coverage(domain)),
                    domain.prediction_is_complete(),
                    domain.prediction_required_leaf_count(),
                    if domain.prediction_is_complete() {
                        committed.map_or(0, |cut| cut.exact_surface_coverage(domain))
                    } else {
                        0
                    },
                )
            },
        );
        let virtual_terrain_cut_fingerprint = virtual_visible
            .then_some(self.virtual_terrain_cut.as_ref())
            .flatten()
            .map_or(0, |cut| cut.fingerprint);
        let presented_snapshot_identity = virtual_visible
            .then(|| self.virtual_terrain_gpu.active_snapshot_identity())
            .flatten();
        let (
            virtual_terrain_presented_snapshot_generation,
            virtual_terrain_presented_snapshot_fingerprint,
        ) = presented_snapshot_identity.unwrap_or((0, 0));
        let virtual_terrain_presented_snapshot_matches_cut = virtual_visible
            && self
                .virtual_terrain_cut
                .as_ref()
                .zip(self.virtual_terrain_committed_envelope.as_ref())
                .is_some_and(|(cut, envelope)| {
                    self.virtual_terrain_gpu.presented_snapshot_matches(
                        virtual_terrain_snapshot_identity(cut, envelope),
                    )
                });
        self.diagnostics = RenderDiagnostics {
            resident_chunks: (self.chunks.len()
                + usize::from(virtual_visible) * self.virtual_terrain_pages.len())
                as u32,
            visible_chunks: visible_terrain_meshes,
            draw_calls: visible_terrain_draw_calls
                .saturating_add(visible_water_draw_calls as usize)
                .saturating_add(usize::from(has_avatars)) as u32,
            water_draw_calls: visible_water_draw_calls,
            shadow_draw_calls,
            shadow_cascades: if shadows_active {
                CASCADE_COUNT as u32
            } else {
                0
            },
            quads: visible_terrain_primitives,
            water_quads: visible_water_primitives,
            virtual_terrain_cpu_selected_pages: oracle_virtual_selected_pages as u32,
            virtual_terrain_cpu_requested_pages: oracle_virtual_requested_pages as u32,
            virtual_terrain_cpu_refinement_roots: oracle_virtual_refinement_roots as u32,
            virtual_terrain_cpu_ownerless_roots: oracle_virtual_ownerless_roots as u32,
            virtual_terrain_cpu_exact_lod_discontinuities: oracle_virtual_exact_lod_discontinuities
                as u32,
            virtual_terrain_gpu_selected_pages: gpu_virtual_feedback
                .as_ref()
                .map_or(0, |feedback| feedback.selected_pages.len() as u32),
            virtual_terrain_gpu_ownerless_roots: gpu_virtual_feedback
                .as_ref()
                .map_or(0, |feedback| feedback.ownerless_roots),
            virtual_terrain_gpu_encoded_surface_handles: gpu_virtual_feedback
                .as_ref()
                .map_or(0, |feedback| feedback.encoded_surface_handles),
            virtual_terrain_gpu_encoded_triangle_handles: gpu_virtual_feedback
                .as_ref()
                .map_or(0, |feedback| feedback.encoded_triangle_handles),
            virtual_terrain_gpu_encoded_water_surface_handles: gpu_virtual_feedback
                .as_ref()
                .map_or(0, |feedback| feedback.encoded_water_surface_handles),
            virtual_terrain_gpu_encoded_water_triangle_handles: gpu_virtual_feedback
                .as_ref()
                .map_or(0, |feedback| feedback.encoded_water_triangle_handles),
            virtual_terrain_gpu_encoded_pages: gpu_virtual_feedback
                .as_ref()
                .map_or(0, |feedback| feedback.encoded_pages),
            virtual_terrain_gpu_encoding_overflow_flags: gpu_virtual_feedback
                .as_ref()
                .map_or(0, |feedback| feedback.encoding_overflow_flags),
            virtual_terrain_gpu_matches_cpu_cut: gpu_virtual_matches_cpu,
            virtual_terrain_gpu_match_failure_flags: gpu_virtual_match_failure_flags,
            virtual_terrain_published_pages: published_virtual_pages.len() as u32,
            virtual_terrain_published_ownerless_roots: published_virtual_ownerless_roots as u32,
            virtual_terrain_published_exact_pages: published_virtual_exact_pages as u32,
            virtual_terrain_published_minimum_level: u32::from(published_virtual_minimum_level),
            virtual_terrain_published_maximum_level: u32::from(published_virtual_maximum_level),
            virtual_terrain_published_exact_lod_discontinuities:
                published_virtual_exact_lod_discontinuities as u32,
            virtual_terrain_exact_domain_complete,
            virtual_terrain_exact_domain_required_leaves:
                virtual_terrain_exact_domain_required_leaves as u32,
            virtual_terrain_exact_domain_current_coverage:
                virtual_terrain_exact_domain_current_coverage as u32,
            virtual_terrain_exact_domain_fingerprint,
            virtual_terrain_exact_core_complete,
            virtual_terrain_exact_core_required_leaves: virtual_terrain_exact_core_required_leaves
                as u32,
            virtual_terrain_exact_core_current_coverage: virtual_terrain_exact_core_current_coverage
                as u32,
            virtual_terrain_exact_prediction_complete,
            virtual_terrain_exact_prediction_required_leaves:
                virtual_terrain_exact_prediction_required_leaves as u32,
            virtual_terrain_exact_prediction_current_coverage:
                virtual_terrain_exact_prediction_current_coverage as u32,
            virtual_terrain_cut_fingerprint,
            virtual_terrain_presented_snapshot_generation,
            virtual_terrain_presented_snapshot_fingerprint,
            virtual_terrain_presented_snapshot_matches_cut,
            viewport_fingerprint,
            refraction_copy_bytes: refraction_copy_bytes(
                self.config.width,
                self.config.height,
                refract_water,
            ),
            arena_pages: arena
                .pages
                .saturating_add(water_arena.pages)
                .saturating_add(virtual_terrain_arena.pages)
                .saturating_add(2) as u32,
            arena_capacity_bytes: arena
                .capacity_bytes
                .saturating_add(water_arena.capacity_bytes)
                .saturating_add(virtual_terrain_arena.capacity_bytes)
                .saturating_add(self.virtual_terrain_gpu.handle_bank_capacity_bytes()),
            arena_allocated_bytes: arena
                .allocated_bytes
                .saturating_add(water_arena.allocated_bytes)
                .saturating_add(virtual_terrain_arena.allocated_bytes)
                .saturating_add(self.virtual_terrain_gpu.allocated_handle_bytes()),
            core_gpu_bytes: arena
                .capacity_bytes
                .saturating_add(water_arena.capacity_bytes)
                .saturating_add(virtual_terrain_arena.capacity_bytes)
                .saturating_add(self.virtual_terrain_gpu.handle_bank_capacity_bytes())
                // Two RGBA16F scene targets plus writable and sampled Depth32Float targets.
                .saturating_add(scene_pixels.saturating_mul(24))
                .saturating_add(shadow_bytes)
                .saturating_add(self.ambient_occlusion_gpu.bytes())
                .saturating_add(self.volumetric_cloud_gpu.bytes())
                .saturating_add(self.material_detail.bytes)
                .saturating_add(size_of::<LocalLightUniform>() as u64)
                .saturating_add(self.avatar_gpu.buffer_bytes())
                .saturating_add(if self.gpu_timer.is_some() {
                    GPU_TIMER_BUFFER_BYTES
                } else {
                    0
                }),
            gpu_sample_id: gpu_timing.map_or(0, |timing| timing.frame_id),
            gpu_total_ms: gpu_timing.map(|timing| timing.total_ms),
            gpu_shadow_ms: gpu_timing.map(|timing| timing.shadow_ms),
            gpu_depth_prepass_ms: gpu_timing.map(|timing| timing.depth_prepass_ms),
            gpu_world_ms: gpu_timing.map(|timing| timing.world_ms),
            gpu_water_ms: gpu_timing.map(|timing| timing.water_ms),
            gpu_ambient_occlusion_ms: gpu_timing.map(|timing| timing.ambient_occlusion_ms),
            gpu_cloud_ms: gpu_timing.map(|timing| timing.cloud_ms),
            gpu_weather_ms: gpu_timing.map(|timing| timing.weather_ms),
            gpu_ui_ms: gpu_timing.map(|timing| timing.ui_ms),
            gpu_virtual_terrain_snapshot_encode_ms: gpu_timing
                .map(|timing| timing.virtual_terrain_snapshot_encode_ms),
            gpu_virtual_terrain_snapshot_validation_ms: gpu_timing
                .map(|timing| timing.virtual_terrain_snapshot_validation_ms),
            cpu_cull_ms,
            cpu_encode_ms: 0.0,
            cpu_submit_ms: 0.0,
            draw_list_tested_slices: shadow_draw_lists
                .iter()
                .map(|draw_list| draw_list.tested_slices)
                .sum::<u32>()
                .saturating_add(world_draw_list.tested_slices)
                .saturating_add(if virtual_visible {
                    virtual_world_draw_lists.surfaces.tested_slices
                } else {
                    0
                })
                .saturating_add(water_draw_list.tested_slices),
            draw_list_selected_slices: shadow_draw_lists
                .iter()
                .map(|draw_list| draw_list.selected_slices)
                .sum::<u32>()
                .saturating_add(world_draw_list.selected_slices)
                .saturating_add(if virtual_visible {
                    virtual_world_draw_lists.surfaces.selected_slices
                } else {
                    0
                })
                .saturating_add(water_draw_list.selected_slices),
            surface_width: self.config.width,
            surface_height: self.config.height,
            dpr: self.dpr,
            ambient_occlusion_bytes: self.ambient_occlusion_gpu.bytes(),
            depth_prepass_draw_calls,
            screen_space_ambient_occlusion: self.options.screen_space_ambient_occlusion,
            material_detail: self.options.material_detail,
            daylight_phase: self.daylight_phase as u8,
            day_fraction: self.world_environment.day_fraction,
            local_solar_day_fraction: self.celestial_observation.local_solar_day_fraction as f32,
            year_fraction: self.world_environment.year_fraction,
            moon_orbit_fraction: self.world_environment.moon_orbit_fraction,
            twinkle_phase: self.world_environment.twinkle_phase,
            latitude_degrees: self
                .celestial_observation
                .coordinates
                .latitude_radians
                .to_degrees() as f32,
            longitude_degrees: self
                .celestial_observation
                .coordinates
                .longitude_radians
                .to_degrees() as f32,
            local_sidereal_angle_radians: self.celestial_observation.local_sidereal_angle_radians
                as f32,
            sun_direction: self.environment.sun_direction.to_array(),
            moon_direction: self.environment.moon_direction.to_array(),
            moon_illuminated_fraction: self.celestial_observation.moon_illuminated_fraction,
            celestial_revision: self.world_environment.celestial_revision,
            shadow_strength: self.environment.shadow_strength,
            surface_region: self.surface_region as u8,
            cloud_coverage: self.environment.cloud_coverage,
            cloud_density: self.environment.cloud_density,
            cloud_base_metres: self.world_environment.cloud_base_metres,
            cloud_top_metres: self.world_environment.cloud_top_metres,
            cloud_offset_metres: self.world_environment.cloud_offset_metres,
            cloud_velocity_metres_per_second: self
                .world_environment
                .cloud_velocity_metres_per_second,
            cloud_render_resolution: self.volumetric_cloud_gpu.resolution(),
            cloud_steps: self.volumetric_cloud_gpu.quality(),
            weather_kind: self
                .world_environment
                .weather(self.atmosphere_sample.coldness)
                .kind as u8,
            weather_fraction: self.world_environment.weather_fraction,
            precipitation: self.environment.precipitation,
            storminess: self.environment.storminess,
            lightning: self.environment.lightning,
            fog_density: self.environment.fog_density,
            outdoor_exposure: self.environment.exposure,
            weather_revision: self.world_environment.weather_revision,
            enclosure: self.interior.enclosure,
            interior_exposure: self.interior.exposure_multiplier,
            cave_headlamp: self.options.cave_headlamp && self.interior.headlamp_strength > 0.01,
            local_light_candidates: local_lights.metadata[1],
            active_local_lights: local_lights.metadata[0],
            clipped_local_lights: local_lights.metadata[2],
            occluded_local_lights,
            portal_rejected_local_lights,
            local_light_visibility_tests,
            local_lighting: self.options.local_lighting,
            remote_avatars: self.avatar_gpu.avatar_count(),
            avatar_parts: avatar_instances,
            avatar_draw_calls: u32::from(has_avatars)
                + u32::from(has_avatars && self.options.screen_space_ambient_occlusion)
                + if has_avatars && shadows_active {
                    CASCADE_COUNT as u32
                } else {
                    0
                },
        };
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("present and Rust UI pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: gpu_frame.as_ref().map(|frame| frame.pass(22)),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.ui_gpu.draw(&mut pass);
        }
        if let Some(target) = screenshot_target.as_ref() {
            let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("screenshot composite pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.ui_gpu.draw(&mut pass);
        }
        if let (
            Some(identity_target),
            Some(reverse_z_target),
            Some(opaque_owners),
            Some(water_owners),
        ) = (
            screenshot_diagnostic_identity_target.as_ref(),
            screenshot_diagnostic_depth_target.as_ref(),
            screenshot_opaque_owners.as_ref(),
            screenshot_water_owners.as_ref(),
        ) {
            let identity_view =
                identity_target.create_view(&wgpu::TextureViewDescriptor::default());
            let reverse_z_view =
                reverse_z_target.create_view(&wgpu::TextureViewDescriptor::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("screenshot integer terrain ownership pass"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: &identity_view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: &reverse_z_view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: self.depth.view(),
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.frame_bind_group, &[]);
            pass.set_bind_group(2, self.ambient_occlusion_gpu.sample_bind_group(), &[]);
            if let Some(virtual_owners) = screenshot_virtual_opaque_owners.as_ref() {
                pass.set_pipeline(&self.screenshot_diagnostic_pipeline);
                draw_diagnostic_spans(
                    &mut pass,
                    &self.virtual_terrain_arena_buffers,
                    virtual_owners,
                    &virtual_world_draw_lists.surfaces,
                );
                pass.set_pipeline(&self.virtual_triangle_diagnostic_pipeline);
                draw_diagnostic_triangle_spans(
                    &mut pass,
                    &self.virtual_terrain_arena_buffers,
                    virtual_owners,
                    &virtual_world_draw_lists.triangles,
                );
            }
            pass.set_pipeline(&self.screenshot_diagnostic_pipeline);
            draw_diagnostic_spans(
                &mut pass,
                &self.arena_buffers,
                opaque_owners,
                &world_draw_list.fixed,
            );
            if refract_water {
                pass.set_pipeline(&self.screenshot_diagnostic_pipeline);
                if let Some(virtual_owners) = screenshot_virtual_opaque_owners.as_ref() {
                    draw_diagnostic_spans(
                        &mut pass,
                        &self.virtual_terrain_arena_buffers,
                        virtual_owners,
                        &virtual_world_draw_lists.water_surfaces,
                    );
                    pass.set_pipeline(&self.virtual_triangle_diagnostic_pipeline);
                    draw_diagnostic_triangle_spans(
                        &mut pass,
                        &self.virtual_terrain_arena_buffers,
                        virtual_owners,
                        &virtual_world_draw_lists.water_triangles,
                    );
                }
                draw_diagnostic_spans(
                    &mut pass,
                    &self.water_arena_buffers,
                    water_owners,
                    &water_draw_list,
                );
            }
        }
        self.schedule_screenshot_readback(
            &mut encoder,
            screenshot_target.as_ref(),
            screenshot_diagnostic_identity_target.as_ref(),
            screenshot_diagnostic_depth_target.as_ref(),
            frame_id,
            camera,
        );
        if let Some(generation) = virtual_candidate_generation_to_submit {
            // No fallible frame construction remains below this point. A recorded generation only
            // becomes submitted/readback-eligible here, so abandoning an earlier encoder cannot
            // strand a slot or certify counters that were never produced.
            self.virtual_terrain_gpu
                .submit_pending_readback(&mut encoder, generation);
        }
        if let (Some(timer), Some(gpu_frame)) = (self.gpu_timer.as_ref(), gpu_frame.as_ref()) {
            timer.resolve(&mut encoder, gpu_frame);
        }
        if let (Some(timer), Some(gpu_frame)) = (self.gpu_timer.as_ref(), gpu_frame) {
            timer.schedule_readback(&encoder, gpu_frame);
        }
        let command_buffer = encoder.finish();
        self.diagnostics.cpu_encode_ms = (now_ms() - encode_started).max(0.0) as f32;
        let submit_started = now_ms();
        self.queue.submit([command_buffer]);
        self.queue.present(frame);
        self.diagnostics.cpu_submit_ms = (now_ms() - submit_started).max(0.0) as f32;
        true
    }

    fn schedule_screenshot_readback(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        texture: Option<&wgpu::Texture>,
        diagnostic_identity_texture: Option<&wgpu::Texture>,
        diagnostic_depth_texture: Option<&wgpu::Texture>,
        frame_id: u32,
        camera: &CameraState,
    ) {
        if !self.screenshot_requested {
            return;
        }
        self.screenshot_requested = false;
        let Some(texture) = texture else {
            (self.log_error)("screenshot capture failed: composite target was not created");
            self.report_screenshot_result(false);
            return;
        };
        let diagnostic_targets = match (diagnostic_identity_texture, diagnostic_depth_texture) {
            (Some(identity), Some(depth)) => Some((identity, depth)),
            (None, None) => None,
            _ => {
                (self.log_error)("screenshot capture failed: diagnostic targets disagree");
                self.report_screenshot_result(false);
                return;
            }
        };
        let bgra = match self.config.format {
            TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb => true,
            TextureFormat::Rgba8Unorm | TextureFormat::Rgba8UnormSrgb => false,
            _ => {
                (self.log_error)(
                    "screenshot capture unavailable: presentation format is not RGBA8 or BGRA8",
                );
                self.report_screenshot_result(false);
                return;
            }
        };
        let width = self.config.width;
        let height = self.config.height;
        let Some(unpadded_bytes_per_row) = width.checked_mul(4) else {
            self.report_screenshot_result(false);
            return;
        };
        let padded_bytes_per_row = unpadded_bytes_per_row
            .div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let color_buffer_size = u64::from(padded_bytes_per_row) * u64::from(height);
        let diagnostic_identity_unpadded_bytes_per_row = if diagnostic_targets.is_some() {
            match width.checked_mul(16) {
                Some(bytes) => bytes,
                None => {
                    self.report_screenshot_result(false);
                    return;
                }
            }
        } else {
            0
        };
        let diagnostic_identity_padded_bytes_per_row = diagnostic_identity_unpadded_bytes_per_row
            .div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let diagnostic_identity_buffer_size = diagnostic_targets.map_or(0, |_| {
            u64::from(diagnostic_identity_padded_bytes_per_row) * u64::from(height)
        });
        let diagnostic_depth_padded_bytes_per_row =
            diagnostic_targets.map_or(0, |_| padded_bytes_per_row);
        let diagnostic_depth_buffer_size =
            u64::from(diagnostic_depth_padded_bytes_per_row) * u64::from(height);
        let Some(buffer_size) = color_buffer_size
            .checked_add(diagnostic_identity_buffer_size)
            .and_then(|size| size.checked_add(diagnostic_depth_buffer_size))
        else {
            self.report_screenshot_result(false);
            return;
        };
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("screenshot readback"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        if let Some((diagnostic_identity_texture, diagnostic_depth_texture)) = diagnostic_targets {
            encoder.copy_texture_to_buffer(
                diagnostic_identity_texture.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: color_buffer_size,
                        bytes_per_row: Some(diagnostic_identity_padded_bytes_per_row),
                        rows_per_image: Some(height),
                    },
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
            encoder.copy_texture_to_buffer(
                diagnostic_depth_texture.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: color_buffer_size + diagnostic_identity_buffer_size,
                        bytes_per_row: Some(diagnostic_depth_padded_bytes_per_row),
                        rows_per_image: Some(height),
                    },
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        }
        let filename = self.ui.screenshot_filename();
        let metadata = self.screenshot_reproduction_metadata(frame_id, camera);
        let state = Arc::clone(&self.screenshot_readback);
        if let Ok(mut readback) = state.lock() {
            readback.in_flight = true;
            readback.completed = None;
        } else {
            self.report_screenshot_result(false);
            return;
        }
        let callback_buffer = buffer.clone();
        let log_error = self.log_error;
        encoder.map_buffer_on_submit(&buffer, wgpu::MapMode::Read, .., move |result| {
            let mapped = result.is_ok();
            let capture = if mapped {
                let capture = callback_buffer
                    .get_mapped_range(..)
                    .ok()
                    .and_then(|mapped| {
                        let color_end = usize::try_from(color_buffer_size).ok()?;
                        let identity_end =
                            usize::try_from(color_buffer_size + diagnostic_identity_buffer_size)
                                .ok()?;
                        let rgba = unpack_screenshot_rgba(
                            mapped.get(..color_end)?,
                            width,
                            height,
                            padded_bytes_per_row,
                            bgra,
                        )?;
                        let terrain_diagnostic_u32x5 = if diagnostic_identity_buffer_size == 0 {
                            Vec::new()
                        } else {
                            let diagnostic_identity = mapped.get(color_end..identity_end)?;
                            let diagnostic_depth = mapped.get(identity_end..)?;
                            let diagnostic_identity = unpack_screenshot_diagnostic_rows(
                                diagnostic_identity,
                                width,
                                height,
                                16,
                                diagnostic_identity_padded_bytes_per_row,
                            )?;
                            let diagnostic_depth = unpack_screenshot_diagnostic_rows(
                                diagnostic_depth,
                                width,
                                height,
                                4,
                                diagnostic_depth_padded_bytes_per_row,
                            )?;
                            interleave_screenshot_diagnostic(
                                &diagnostic_identity,
                                &diagnostic_depth,
                                width,
                                height,
                            )?
                        };
                        Some(ScreenshotCapture {
                            filename,
                            metadata,
                            width,
                            height,
                            rgba,
                            terrain_diagnostic_u32x5,
                        })
                    });
                callback_buffer.unmap();
                capture
            } else {
                log_error("screenshot capture failed: GPU readback buffer could not be mapped");
                None
            };
            if mapped && capture.is_none() {
                log_error("screenshot capture failed: GPU pixels could not be decoded");
            }
            if let Ok(mut readback) = state.lock() {
                readback.in_flight = false;
                readback.completed = capture;
            }
        });
    }

    fn collect_draw_list(
        &self,
        chunks: &BTreeMap<MeshKey, ChunkMesh>,
        mut include_chunk: impl FnMut(&MeshKey, &ChunkMesh) -> bool,
        mut include_slice: impl FnMut(&MeshKey, &MeshSlice) -> bool,
    ) -> DrawList {
        let mut items = Vec::new();
        let mut mesh_count = 0u32;
        let mut quad_count = 0u32;
        let mut fingerprint = FINGERPRINT_OFFSET;
        let mut tested_slices = 0u32;
        let mut selected_slices = 0u32;
        for (key, chunk) in chunks {
            if !chunk.active() || !include_chunk(key, chunk) {
                continue;
            }
            debug_assert!(chunk.allocation.size >= chunk.quad_count * size_of::<GpuQuad>() as u32);
            let mut selected = false;
            for slice in &chunk.slices {
                tested_slices = tested_slices.saturating_add(1);
                if !include_slice(key, slice) {
                    continue;
                }
                selected_slices = selected_slices.saturating_add(1);
                items.push(DrawItem {
                    page: chunk.allocation.page,
                    offset: chunk.allocation.offset + slice.relative_offset,
                    size: slice.size,
                    quad_count: slice.quad_count,
                });
                selected = true;
                quad_count = quad_count.saturating_add(slice.quad_count);
            }
            if selected {
                mesh_count = mesh_count.saturating_add(1);
                fingerprint = fingerprint_value(fingerprint, u64::from(key.0));
                fingerprint = fingerprint_value(fingerprint, key.1 as u32 as u64);
                fingerprint = fingerprint_value(fingerprint, key.2 as u32 as u64);
                fingerprint = fingerprint_value(fingerprint, key.3 as u32 as u64);
                fingerprint = fingerprint_value(fingerprint, chunk.content_fingerprint);
            }
        }
        DrawList {
            spans: coalesce_draw_items(items),
            mesh_count,
            quad_count,
            fingerprint,
            tested_slices,
            selected_slices,
        }
    }

    fn collect_virtual_terrain_draw_list(
        &self,
        view_clip: AabbClipVolume,
    ) -> Result<VirtualTerrainDrawLists, VirtualTerrainRendererError> {
        let Some(cut) = self.virtual_terrain_cut.as_ref() else {
            return Ok(VirtualTerrainDrawLists::default());
        };
        let mut surface_items = Vec::new();
        let mut triangle_items = Vec::new();
        let mut water_surface_items = Vec::new();
        let mut water_triangle_items = Vec::new();
        let mut mesh_count = 0u32;
        let mut primitive_count = 0u32;
        let mut surface_mesh_count = 0u32;
        let mut surface_quad_count = 0u32;
        let mut triangle_mesh_count = 0u32;
        let mut triangle_vertex_count = 0u32;
        let mut water_surface_quads = 0u32;
        let mut water_triangle_vertices = 0u32;
        let mut water_surface_mesh_count = 0u32;
        let mut water_triangle_mesh_count = 0u32;
        let mut fingerprint = cut.fingerprint;
        let mut surface_fingerprint = cut.fingerprint;
        let mut triangle_fingerprint = cut.fingerprint;
        let mut water_surface_fingerprint = cut.fingerprint;
        let mut water_triangle_fingerprint = cut.fingerprint;
        let mut tested_slices = 0u32;
        let mut selected_slices = 0u32;
        let mut water_selected_slices = 0u32;
        for key in &cut.selected_pages {
            let page = self
                .virtual_terrain_retired_published_pages
                .get(key)
                .or_else(|| self.virtual_terrain_pages.get(key))
                .ok_or(VirtualTerrainRendererError::SelectedPageMissingGpu(*key))?;
            fingerprint = fingerprint_value(fingerprint, u64::from(key.level));
            for component in key.coord {
                fingerprint = fingerprint_value(fingerprint, component as u32 as u64);
            }
            fingerprint = fingerprint_value(fingerprint, page.revision);
            fingerprint =
                fingerprint_value(fingerprint, fingerprint_bytes(&page.content_fingerprint));
            match &page.mesh {
                VirtualTerrainGpuMesh::Empty => {}
                VirtualTerrainGpuMesh::Surface(mesh) => {
                    if !view_clip.contains_aabb(mesh.bounds_min, mesh.bounds_max) {
                        continue;
                    }
                    mesh_count = mesh_count.saturating_add(1);
                    surface_mesh_count = surface_mesh_count.saturating_add(1);
                    fingerprint = fingerprint_value(fingerprint, mesh.content_fingerprint);
                    surface_fingerprint =
                        fingerprint_value(surface_fingerprint, mesh.content_fingerprint);
                    let mut has_water_surface = false;
                    for slice in &mesh.slices {
                        tested_slices = tested_slices.saturating_add(1);
                        match slice.render_layer {
                            RenderLayer::Opaque => {}
                            RenderLayer::Translucent => {
                                has_water_surface = true;
                                water_selected_slices = water_selected_slices.saturating_add(1);
                                water_surface_quads =
                                    water_surface_quads.saturating_add(slice.quad_count);
                                water_surface_items.push(DrawItem {
                                    page: mesh.allocation.page,
                                    offset: mesh.allocation.offset + slice.relative_offset,
                                    size: slice.size,
                                    quad_count: slice.quad_count,
                                });
                                water_surface_fingerprint = fingerprint_value(
                                    water_surface_fingerprint,
                                    mesh.content_fingerprint,
                                );
                                primitive_count = primitive_count
                                    .saturating_add(slice.quad_count.saturating_mul(2));
                                continue;
                            }
                            RenderLayer::Empty => continue,
                        }
                        selected_slices = selected_slices.saturating_add(1);
                        surface_items.push(DrawItem {
                            page: mesh.allocation.page,
                            offset: mesh.allocation.offset + slice.relative_offset,
                            size: slice.size,
                            quad_count: slice.quad_count,
                        });
                        surface_quad_count = surface_quad_count.saturating_add(slice.quad_count);
                        primitive_count =
                            primitive_count.saturating_add(slice.quad_count.saturating_mul(2));
                    }
                    water_surface_mesh_count =
                        water_surface_mesh_count.saturating_add(u32::from(has_water_surface));
                }
                VirtualTerrainGpuMesh::Triangle(mesh) => {
                    if !view_clip.contains_aabb(mesh.bounds_min, mesh.bounds_max) {
                        continue;
                    }
                    mesh_count = mesh_count.saturating_add(1);
                    triangle_mesh_count = triangle_mesh_count.saturating_add(1);
                    fingerprint = fingerprint_value(fingerprint, mesh.content_fingerprint);
                    triangle_fingerprint =
                        fingerprint_value(triangle_fingerprint, mesh.content_fingerprint);
                    if mesh.opaque_vertex_count > 0 {
                        triangle_items.push(TerrainTriangleDrawSpan {
                            page: mesh.allocation.page,
                            offset: mesh.allocation.offset,
                            size: mesh
                                .opaque_vertex_count
                                .saturating_mul(size_of::<GpuTerrainVertex>() as u32),
                            vertex_count: mesh.opaque_vertex_count,
                        });
                        triangle_vertex_count =
                            triangle_vertex_count.saturating_add(mesh.opaque_vertex_count);
                    }
                    water_triangle_vertices =
                        water_triangle_vertices.saturating_add(mesh.water_vertex_count);
                    if mesh.water_vertex_count > 0 {
                        water_triangle_mesh_count = water_triangle_mesh_count.saturating_add(1);
                        let offset = mesh.allocation.offset.saturating_add(
                            mesh.opaque_vertex_count
                                .saturating_mul(size_of::<GpuTerrainVertex>() as u32),
                        );
                        water_triangle_items.push(TerrainTriangleDrawSpan {
                            page: mesh.allocation.page,
                            offset,
                            size: mesh
                                .water_vertex_count
                                .saturating_mul(size_of::<GpuTerrainVertex>() as u32),
                            vertex_count: mesh.water_vertex_count,
                        });
                        water_triangle_fingerprint =
                            fingerprint_value(water_triangle_fingerprint, mesh.content_fingerprint);
                    }
                    primitive_count = primitive_count.saturating_add(mesh.vertex_count / 3);
                }
            }
        }
        Ok(VirtualTerrainDrawLists {
            surfaces: DrawList {
                spans: coalesce_draw_items(surface_items),
                mesh_count: surface_mesh_count,
                quad_count: surface_quad_count,
                fingerprint: surface_fingerprint,
                tested_slices,
                selected_slices,
            },
            triangles: TerrainTriangleDrawList {
                spans: coalesce_triangle_draw_spans(triangle_items),
                mesh_count: triangle_mesh_count,
                vertex_count: triangle_vertex_count,
                fingerprint: triangle_fingerprint,
            },
            water_surfaces: DrawList {
                spans: coalesce_draw_items(water_surface_items),
                mesh_count: water_surface_mesh_count,
                quad_count: water_surface_quads,
                fingerprint: water_surface_fingerprint,
                tested_slices,
                selected_slices: water_selected_slices,
            },
            water_triangles: TerrainTriangleDrawList {
                spans: coalesce_triangle_draw_spans(water_triangle_items),
                mesh_count: water_triangle_mesh_count,
                vertex_count: water_triangle_vertices,
                fingerprint: water_triangle_fingerprint,
            },
            fingerprint,
            mesh_count,
            primitive_count,
        })
    }
}

fn install_virtual_terrain_replacement_pages(
    hierarchy: &mut VirtualTerrainHierarchy,
    parent: TerrainPageKey,
    pages: &[TerrainPageV1],
    mut before_install: impl FnMut(usize) -> Result<(), VirtualTerrainError>,
) -> Result<(), VirtualTerrainError> {
    // Reject stale pre-existing children before mutating any sibling. Identical residents are
    // intentionally idempotent and are never removed by rollback.
    for page in pages {
        if hierarchy
            .resident_page(page.key)
            .is_some_and(|resident| resident != page)
        {
            return Err(VirtualTerrainError::StalePage(page.key));
        }
    }

    let mut installed = Vec::new();
    for (index, page) in pages.iter().enumerate() {
        if hierarchy.resident_page(page.key).is_some() {
            continue;
        }
        if let Err(error) =
            before_install(index).and_then(|()| hierarchy.install_page(page.clone()))
        {
            for key in installed.into_iter().rev() {
                let removed = hierarchy.remove_page(key);
                debug_assert!(removed);
            }
            return Err(error);
        }
        installed.push(page.key);
    }
    if !hierarchy.replacement_is_resident_and_coherent(parent) {
        for key in installed.into_iter().rev() {
            let removed = hierarchy.remove_page(key);
            debug_assert!(removed);
        }
        return Err(VirtualTerrainError::IncoherentRootReplacement(parent));
    }
    Ok(())
}

fn recover_candidate_only_seam_rebuild<T, E>(
    result: Result<T, E>,
    recover: impl FnOnce(),
) -> Result<T, E> {
    if result.is_err() {
        recover();
    }
    result
}

fn draw_spans<'pass>(
    pass: &mut wgpu::RenderPass<'pass>,
    arena_buffers: &'pass [Buffer],
    draw_list: &DrawList,
) -> u32 {
    let mut draws = 0u32;
    for span in &draw_list.spans {
        let Some(buffer) = arena_buffers.get(span.page as usize) else {
            continue;
        };
        let start = u64::from(span.offset);
        let end = start + u64::from(span.size);
        pass.set_vertex_buffer(0, buffer.slice(start..end));
        pass.draw(0..QUAD_VERTEX_COUNT, 0..span.quad_count);
        draws = draws.saturating_add(1);
    }
    draws
}

/// Builds a screenshot-only owner sidecar mirroring the resident arena slots.
///
/// Ordinary frames retain the compact 24-byte terrain instance. On an explicit capture request,
/// the directory's authoritative slice identities are expanded into a 16-byte transient vertex
/// stream. This avoids permanently increasing terrain GPU memory merely to support diagnostics.
fn screenshot_diagnostic_owner_buffers(
    device: &Device,
    queue: &Queue,
    arena_buffers: &[Buffer],
    chunks: &BTreeMap<MeshKey, ChunkMesh>,
    label: &'static str,
) -> Option<Vec<Buffer>> {
    let quad_bytes = size_of::<GpuQuad>() as u64;
    let owner_bytes = size_of::<[u32; 4]>() as u64;
    let mut packed = arena_buffers
        .iter()
        .map(|base| {
            let slots = base.size().div_ceil(quad_bytes);
            usize::try_from(slots)
                .ok()
                .map(|slots| vec![[0u32; 4]; slots])
        })
        .collect::<Option<Vec<_>>>()?;
    for (key, chunk) in chunks {
        let owners = packed.get_mut(chunk.allocation.page as usize)?;
        for slice in &chunk.slices {
            let base_offset =
                u64::from(chunk.allocation.offset).checked_add(u64::from(slice.relative_offset))?;
            if !base_offset.is_multiple_of(quad_bytes) {
                return None;
            }
            let owner_offset = usize::try_from(base_offset / quad_bytes).ok()?;
            let owner_id = diagnostic_owner_for_slice(*key);
            let source = if key.0 == 0 {
                DIAGNOSTIC_SOURCE_CANONICAL
            } else {
                DIAGNOSTIC_SOURCE_FRONTIER
            };
            let owner = [owner_id[0], owner_id[1], source, 0];
            let end = owner_offset.checked_add(slice.quad_count as usize)?;
            owners.get_mut(owner_offset..end)?.fill(owner);
        }
    }
    Some(
        arena_buffers
            .iter()
            .zip(packed)
            .map(|(base, owners)| {
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size: base
                        .size()
                        .div_ceil(quad_bytes)
                        .saturating_mul(owner_bytes)
                        .max(owner_bytes),
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                queue.write_buffer(&buffer, 0, bytemuck::cast_slice(&owners));
                buffer
            })
            .collect(),
    )
}

fn screenshot_virtual_terrain_owner_buffers(
    device: &Device,
    queue: &Queue,
    arena_buffers: &[Buffer],
    pages: &BTreeMap<TerrainPageKey, VirtualTerrainGpuPage>,
    retired_published_pages: &BTreeMap<TerrainPageKey, VirtualTerrainGpuPage>,
    label: &'static str,
) -> Option<Vec<Buffer>> {
    let primitive_bytes = size_of::<GpuQuad>() as u64;
    debug_assert_eq!(primitive_bytes, size_of::<GpuTerrainVertex>() as u64);
    let owner_bytes = size_of::<[u32; 4]>() as u64;
    let mut packed = arena_buffers
        .iter()
        .map(|base| {
            let slots = base.size().div_ceil(primitive_bytes);
            usize::try_from(slots)
                .ok()
                .map(|slots| vec![[0u32; 4]; slots])
        })
        .collect::<Option<Vec<_>>>()?;
    for (key, page) in pages.iter().chain(retired_published_pages) {
        let owner_id = diagnostic_owner_id(
            DIAGNOSTIC_VIRTUAL_REPRESENTATION_BASE + u32::from(page.representation as u8),
            u32::from(key.level),
            key.coord[0],
            key.coord[1],
            key.coord[2],
        );
        let owner = [
            owner_id[0],
            owner_id[1],
            (u32::from(page.representation as u8) + 1) | (u32::from(key.level) << 4),
            0,
        ];
        let (allocation, count) = match &page.mesh {
            VirtualTerrainGpuMesh::Empty => continue,
            VirtualTerrainGpuMesh::Surface(mesh) => (mesh.allocation, mesh.quad_count),
            VirtualTerrainGpuMesh::Triangle(mesh) => (mesh.allocation, mesh.vertex_count),
        };
        let owners = packed.get_mut(allocation.page as usize)?;
        let base_offset = u64::from(allocation.offset);
        if !base_offset.is_multiple_of(primitive_bytes) {
            return None;
        }
        let owner_offset = usize::try_from(base_offset / primitive_bytes).ok()?;
        let end = owner_offset.checked_add(count as usize)?;
        owners.get_mut(owner_offset..end)?.fill(owner);
    }
    Some(
        arena_buffers
            .iter()
            .zip(packed)
            .map(|(base, owners)| {
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size: base
                        .size()
                        .div_ceil(primitive_bytes)
                        .saturating_mul(owner_bytes)
                        .max(owner_bytes),
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                queue.write_buffer(&buffer, 0, bytemuck::cast_slice(&owners));
                buffer
            })
            .collect(),
    )
}

fn diagnostic_owner_range(span: &DrawSpan) -> Option<std::ops::Range<u64>> {
    let quad_bytes = size_of::<GpuQuad>() as u64;
    let owner_bytes = size_of::<[u32; 4]>() as u64;
    let base_offset = u64::from(span.offset);
    if !base_offset.is_multiple_of(quad_bytes) {
        return None;
    }
    let start = (base_offset / quad_bytes).checked_mul(owner_bytes)?;
    let end = start.checked_add(u64::from(span.quad_count).checked_mul(owner_bytes)?)?;
    Some(start..end)
}

fn diagnostic_triangle_owner_range(span: &TerrainTriangleDrawSpan) -> Option<std::ops::Range<u64>> {
    let vertex_bytes = size_of::<GpuTerrainVertex>() as u64;
    let owner_bytes = size_of::<[u32; 4]>() as u64;
    let base_offset = u64::from(span.offset);
    if !base_offset.is_multiple_of(vertex_bytes) {
        return None;
    }
    let start = (base_offset / vertex_bytes).checked_mul(owner_bytes)?;
    let end = start.checked_add(u64::from(span.vertex_count).checked_mul(owner_bytes)?)?;
    Some(start..end)
}

fn draw_diagnostic_spans<'pass>(
    pass: &mut wgpu::RenderPass<'pass>,
    arena_buffers: &'pass [Buffer],
    owner_buffers: &'pass [Buffer],
    draw_list: &DrawList,
) -> u32 {
    let mut draws = 0u32;
    for span in &draw_list.spans {
        let (Some(base_buffer), Some(owner_buffer), Some(owner_range)) = (
            arena_buffers.get(span.page as usize),
            owner_buffers.get(span.page as usize),
            diagnostic_owner_range(span),
        ) else {
            continue;
        };
        let base_start = u64::from(span.offset);
        let base_end = base_start + u64::from(span.size);
        pass.set_vertex_buffer(0, base_buffer.slice(base_start..base_end));
        pass.set_vertex_buffer(1, owner_buffer.slice(owner_range));
        pass.draw(0..QUAD_VERTEX_COUNT, 0..span.quad_count);
        draws = draws.saturating_add(1);
    }
    draws
}

fn draw_diagnostic_triangle_spans<'pass>(
    pass: &mut wgpu::RenderPass<'pass>,
    arena_buffers: &'pass [Buffer],
    owner_buffers: &'pass [Buffer],
    draw_list: &TerrainTriangleDrawList,
) -> u32 {
    let mut draws = 0u32;
    for span in &draw_list.spans {
        let (Some(base_buffer), Some(owner_buffer), Some(owner_range)) = (
            arena_buffers.get(span.page as usize),
            owner_buffers.get(span.page as usize),
            diagnostic_triangle_owner_range(span),
        ) else {
            continue;
        };
        let base_start = u64::from(span.offset);
        let base_end = base_start + u64::from(span.size);
        pass.set_vertex_buffer(0, base_buffer.slice(base_start..base_end));
        pass.set_vertex_buffer(1, owner_buffer.slice(owner_range));
        pass.draw(0..span.vertex_count, 0..1);
        draws = draws.saturating_add(1);
    }
    draws
}

/// Builds the camera and three shadow selections in one resident-mesh traversal.
fn collect_opaque_draw_lists(
    chunks: &mut BTreeMap<MeshKey, ChunkMesh>,
    shadows: bool,
    view_clip: AabbClipVolume,
    shadow_clips: [AabbClipVolume; CASCADE_COUNT],
    virtual_ownership: &VirtualTerrainOwnership,
) -> ([WorldDrawLists; CASCADE_COUNT], WorldDrawLists) {
    let mut shadow_builders: [WorldDrawListBuilder; CASCADE_COUNT] =
        std::array::from_fn(|_| WorldDrawListBuilder::default());
    let mut world_builder = WorldDrawListBuilder::default();
    for (key, chunk) in chunks {
        if !chunk.active() || (key.0 != 0 && *key != EXACT_VOLUME_FRONTIER_MESH_KEY) {
            continue;
        }
        let world_chunk_clip = view_clip.classify_aabb(chunk.bounds_min, chunk.bounds_max);
        let world_chunk_visible = world_chunk_clip != AabbClipClassification::Outside;
        let shadow_chunk_clip: [AabbClipClassification; CASCADE_COUNT] =
            std::array::from_fn(|cascade_index| {
                if shadows && mesh_casts_directional_shadow(key) {
                    shadow_clips[cascade_index].classify_aabb(chunk.bounds_min, chunk.bounds_max)
                } else {
                    AabbClipClassification::Outside
                }
            });
        let shadow_chunk_visible = shadow_chunk_clip
            .map(|classification| classification != AabbClipClassification::Outside);
        if !world_chunk_visible && !shadow_chunk_visible.into_iter().any(|visible| visible) {
            continue;
        }
        let mut world_mesh_selected = false;
        let mut shadow_mesh_selected = [false; CASCADE_COUNT];
        for slice in &chunk.slices {
            if world_chunk_visible {
                world_builder.test_slice();
            }
            for cascade_index in 0..CASCADE_COUNT {
                if shadow_chunk_visible[cascade_index] {
                    shadow_builders[cascade_index].test_slice();
                }
            }
            if slice.render_layer != RenderLayer::Opaque
                || virtual_ownership.covers_aabb(slice.bounds_min, slice.bounds_max)
            {
                continue;
            }
            if world_chunk_visible
                && (world_chunk_clip == AabbClipClassification::Inside
                    || view_clip.contains_aabb(slice.bounds_min, slice.bounds_max))
            {
                world_builder.select_slice(chunk, slice);
                world_mesh_selected = true;
            }
            for cascade_index in 0..CASCADE_COUNT {
                if shadow_chunk_visible[cascade_index]
                    && (shadow_chunk_clip[cascade_index] == AabbClipClassification::Inside
                        || shadow_clips[cascade_index]
                            .contains_aabb(slice.bounds_min, slice.bounds_max))
                {
                    shadow_builders[cascade_index].select_slice(chunk, slice);
                    shadow_mesh_selected[cascade_index] = true;
                }
            }
        }
        if world_mesh_selected {
            world_builder.select_mesh(*key, chunk);
        }
        for cascade_index in 0..CASCADE_COUNT {
            if shadow_mesh_selected[cascade_index] {
                shadow_builders[cascade_index].select_mesh(*key, chunk);
            }
        }
    }

    let shadow_draw_lists = if shadows {
        shadow_builders.map(WorldDrawListBuilder::finish)
    } else {
        std::array::from_fn(|_| WorldDrawLists::default())
    };
    (shadow_draw_lists, world_builder.finish())
}

const FINGERPRINT_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FINGERPRINT_PRIME: u64 = 0x100_0000_01b3;

fn fingerprint_bytes(bytes: &[u8]) -> u64 {
    bytes.iter().fold(FINGERPRINT_OFFSET, |fingerprint, byte| {
        (fingerprint ^ u64::from(*byte)).wrapping_mul(FINGERPRINT_PRIME)
    })
}

fn fingerprint_value(fingerprint: u64, value: u64) -> u64 {
    value
        .to_le_bytes()
        .iter()
        .fold(fingerprint, |fingerprint, byte| {
            (fingerprint ^ u64::from(*byte)).wrapping_mul(FINGERPRINT_PRIME)
        })
}

const DIAGNOSTIC_FNV32_OFFSET: u32 = 2_166_136_261;
const DIAGNOSTIC_FNV32_PRIME: u32 = 16_777_619;
const DIAGNOSTIC_VIRTUAL_REPRESENTATION_BASE: u32 = 16;
const DIAGNOSTIC_SOURCE_CANONICAL: u32 = 1;
const DIAGNOSTIC_SOURCE_FRONTIER: u32 = 8;

fn diagnostic_owner_id(
    representation_kind: u32,
    hierarchy_depth: u32,
    page_x: i32,
    page_y: i32,
    page_z: i32,
) -> [u32; 2] {
    let mut low = DIAGNOSTIC_FNV32_OFFSET;
    let mut high = 0u32;
    for word in [
        representation_kind,
        hierarchy_depth,
        page_x as u32,
        page_y as u32,
        page_z as u32,
    ] {
        low = (low ^ word).wrapping_mul(DIAGNOSTIC_FNV32_PRIME);
        high = high.wrapping_add(word);
        high = high.wrapping_add(high << 10);
        high ^= high >> 6;
    }
    high = high.wrapping_add(high << 3);
    high ^= high >> 11;
    high = high.wrapping_add(high << 15);
    if low == 0 && high == 0 {
        low = 1;
    }
    [low, high]
}

fn diagnostic_owner_for_slice(key: MeshKey) -> [u32; 2] {
    if key.0 == 0 {
        diagnostic_owner_id(1, 0, key.1, key.2, key.3)
    } else {
        // Renderer-generated frontier products are explicit owners too.
        diagnostic_owner_id(3, u32::from(key.0), key.1, key.2, key.3)
    }
}

fn gpu_quad_content_fingerprint(quads: &[GpuQuad]) -> u64 {
    fingerprint_bytes(bytemuck::cast_slice(quads))
}

#[allow(
    clippy::too_many_arguments,
    reason = "the helper borrows independent renderer-owned arena resources transactionally"
)]
fn prepare_mesh_sliced_into(
    device: &Device,
    queue: &Queue,
    arena: &mut ArenaAllocator,
    arena_buffers: &mut Vec<Buffer>,
    gpu_quads: &[GpuQuad],
    mut slices: Vec<MeshSlice>,
    activation_mask: u8,
    buffer_label: &'static str,
) -> Option<ChunkMesh> {
    if gpu_quads.is_empty() {
        return None;
    }
    slices.retain(|slice| slice.size > 0 && slice.quad_count > 0);
    if slices.is_empty() {
        return None;
    }
    let (mut bounds_min, mut bounds_max) = slices
        .first()
        .map(|slice| (slice.bounds_min, slice.bounds_max))
        .unwrap_or((glam::Vec3::ZERO, glam::Vec3::ZERO));
    for slice in slices.iter().skip(1) {
        bounds_min = bounds_min.min(slice.bounds_min);
        bounds_max = bounds_max.max(slice.bounds_max);
    }
    for slice in &slices {
        if slice.relative_offset % size_of::<GpuQuad>() as u32 != 0
            || slice.size != slice.quad_count.saturating_mul(size_of::<GpuQuad>() as u32)
        {
            return None;
        }
        let first = (slice.relative_offset / size_of::<GpuQuad>() as u32) as usize;
        let end = first.checked_add(slice.quad_count as usize)?;
        gpu_quads.get(first..end)?;
    }
    let bytes = bytemuck::cast_slice(gpu_quads);
    let Ok(byte_len) = u32::try_from(bytes.len()) else {
        return None;
    };
    let allocation = arena.allocate(byte_len)?;
    while arena_buffers.len() <= allocation.page as usize {
        let page = arena_buffers.len() as u16;
        let Some(capacity) = arena.page_capacity(page) else {
            let _ = arena.free(allocation);
            return None;
        };
        arena_buffers.push(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(buffer_label),
            size: u64::from(capacity),
            usage: wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
    }
    let Some(buffer) = arena_buffers.get(allocation.page as usize) else {
        let _ = arena.free(allocation);
        return None;
    };
    queue.write_buffer(buffer, u64::from(allocation.offset), bytes);
    let content_fingerprint = gpu_quad_content_fingerprint(gpu_quads);
    Some(ChunkMesh {
        allocation,
        quad_count: gpu_quads.len() as u32,
        content_fingerprint,
        slices,
        bounds_min,
        bounds_max,
        activation_mask,
    })
}

#[allow(
    clippy::too_many_arguments,
    reason = "the helper borrows the independent virtual terrain pool transactionally"
)]
fn prepare_terrain_triangle_mesh_into(
    device: &Device,
    queue: &Queue,
    arena: &mut ArenaAllocator,
    arena_buffers: &mut Vec<Buffer>,
    vertices: &[GpuTerrainVertex],
    opaque_vertex_count: u32,
    water_vertex_count: u32,
    bounds_min: glam::Vec3,
    bounds_max: glam::Vec3,
    buffer_label: &'static str,
) -> Option<TerrainTriangleMesh> {
    if vertices.is_empty() || !vertices.len().is_multiple_of(3) {
        return None;
    }
    if opaque_vertex_count
        .checked_add(water_vertex_count)
        .is_none_or(|count| count != vertices.len() as u32)
    {
        return None;
    }
    let bytes = bytemuck::cast_slice(vertices);
    let byte_len = u32::try_from(bytes.len()).ok()?;
    let allocation = arena.allocate(byte_len)?;
    while arena_buffers.len() <= allocation.page as usize {
        let page = arena_buffers.len() as u16;
        let Some(capacity) = arena.page_capacity(page) else {
            let _ = arena.free(allocation);
            return None;
        };
        arena_buffers.push(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(buffer_label),
            size: u64::from(capacity),
            usage: wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
    }
    let Some(buffer) = arena_buffers.get(allocation.page as usize) else {
        let _ = arena.free(allocation);
        return None;
    };
    queue.write_buffer(buffer, u64::from(allocation.offset), bytes);
    Some(TerrainTriangleMesh {
        allocation,
        vertex_count: vertices.len() as u32,
        opaque_vertex_count,
        water_vertex_count,
        content_fingerprint: fingerprint_bytes(bytes),
        bounds_min,
        bounds_max,
    })
}

fn discard_virtual_terrain_mesh(arena: &mut ArenaAllocator, mesh: VirtualTerrainGpuMesh) {
    match mesh {
        VirtualTerrainGpuMesh::Empty => {}
        VirtualTerrainGpuMesh::Surface(mesh) => {
            discard_prepared_mesh(arena, Some(mesh));
        }
        VirtualTerrainGpuMesh::Triangle(mesh) => {
            let _ = arena.free(mesh.allocation);
        }
    }
}

fn virtual_terrain_gpu_geometry(mesh: &VirtualTerrainGpuMesh) -> VirtualTerrainGpuGeometry {
    let Some(allocation) = mesh.allocation() else {
        return VirtualTerrainGpuGeometry::default();
    };
    virtual_terrain_gpu_geometry_at(mesh, allocation)
}

fn virtual_terrain_gpu_geometry_at(
    mesh: &VirtualTerrainGpuMesh,
    allocation: Allocation,
) -> VirtualTerrainGpuGeometry {
    match mesh {
        VirtualTerrainGpuMesh::Empty => VirtualTerrainGpuGeometry::default(),
        VirtualTerrainGpuMesh::Surface(mesh) => {
            let mut geometry = VirtualTerrainGpuGeometry::default();
            for slice in &mesh.slices {
                let range = VirtualTerrainGpuGeometryRange {
                    source_segment: u32::from(allocation.page),
                    source_offset_bytes: u64::from(allocation.offset + slice.relative_offset),
                    element_count: slice.quad_count,
                };
                match slice.render_layer {
                    RenderLayer::Opaque => geometry.opaque_surface = range,
                    RenderLayer::Translucent => geometry.water_surface = range,
                    RenderLayer::Empty => {}
                }
            }
            geometry
        }
        VirtualTerrainGpuMesh::Triangle(mesh) => VirtualTerrainGpuGeometry {
            opaque_triangle: VirtualTerrainGpuGeometryRange {
                source_segment: u32::from(allocation.page),
                source_offset_bytes: u64::from(allocation.offset),
                element_count: mesh.opaque_vertex_count,
            },
            water_triangle: VirtualTerrainGpuGeometryRange {
                source_segment: u32::from(allocation.page),
                source_offset_bytes: u64::from(allocation.offset)
                    + u64::from(mesh.opaque_vertex_count) * size_of::<GpuTerrainVertex>() as u64,
                element_count: mesh.water_vertex_count,
            },
            ..VirtualTerrainGpuGeometry::default()
        },
    }
}

fn gpu_quads_match_resident(mesh: Option<&ChunkMesh>, quads: &[GpuQuad]) -> bool {
    let content_fingerprint = gpu_quad_content_fingerprint(quads);
    gpu_quad_content_matches(
        mesh.map(|mesh| (mesh.quad_count, mesh.content_fingerprint)),
        quads.len() as u32,
        content_fingerprint,
    )
}

fn mesh_slices_match_resident(
    mesh: Option<&ChunkMesh>,
    slices: &[MeshSlice],
    quad_count: usize,
) -> bool {
    if quad_count == 0 {
        return mesh.is_none();
    }
    mesh.is_some_and(|mesh| mesh.slices == slices)
}

fn gpu_quad_content_matches(
    resident: Option<(u32, u64)>,
    quad_count: u32,
    content_fingerprint: u64,
) -> bool {
    if quad_count == 0 {
        return resident.is_none();
    }
    resident.is_some_and(|(resident_count, fingerprint)| {
        resident_count == quad_count && fingerprint == content_fingerprint
    })
}

fn discard_prepared_mesh(arena: &mut ArenaAllocator, prepared: Option<ChunkMesh>) {
    if let Some(prepared) = prepared {
        let _ = arena.free(prepared.allocation);
    }
}

fn commit_prepared_mesh(
    arena: &mut ArenaAllocator,
    chunks: &mut BTreeMap<MeshKey, ChunkMesh>,
    key: MeshKey,
    prepared: Option<ChunkMesh>,
) {
    let old = if let Some(prepared) = prepared {
        chunks.insert(key, prepared)
    } else {
        chunks.remove(&key)
    };
    if let Some(old) = old {
        let _ = arena.free(old.allocation);
    }
}

fn frontier_face_gpu_quads(frontier: &ExactVolumeFrontierFace) -> Vec<GpuQuad> {
    if frontier.face >= 6 || frontier.cells.iter().all(|word| *word == 0) {
        return Vec::new();
    }
    let edge = CHUNK_EDGE;
    let occupied = |u: usize, v: usize| {
        let index = if frontier.face <= 1 {
            // Portal X faces are indexed y + z * edge, while voxel quad U/V are Z/Y.
            v + u * edge
        } else {
            u + v * edge
        };
        frontier.cells[index / 64] & (1_u64 << (index % 64)) != 0
    };
    let mut consumed = vec![false; edge * edge];
    let world = frontier.chunk.world_origin();
    let material_face = pack_gpu_material_face(u32::from(Material::Stone.id()), frontier.face);
    let mut quads = Vec::new();
    for v in 0..edge {
        for u in 0..edge {
            let index = u + v * edge;
            if consumed[index] || !occupied(u, v) {
                continue;
            }
            let mut width = 1;
            while u + width < edge && !consumed[u + width + v * edge] && occupied(u + width, v) {
                width += 1;
            }
            let mut height = 1;
            'height: while v + height < edge {
                for offset in 0..width {
                    if consumed[u + offset + (v + height) * edge]
                        || !occupied(u + offset, v + height)
                    {
                        break 'height;
                    }
                }
                height += 1;
            }
            for clear_v in v..v + height {
                for clear_u in u..u + width {
                    consumed[clear_u + clear_v * edge] = true;
                }
            }
            let origin = match frontier.face {
                0 => [world[0] - 1, world[1] + v as i32, world[2] + u as i32],
                1 => [
                    world[0] + edge as i32,
                    world[1] + v as i32,
                    world[2] + u as i32,
                ],
                2 => [world[0] + u as i32, world[1] - 1, world[2] + v as i32],
                3 => [
                    world[0] + u as i32,
                    world[1] + edge as i32,
                    world[2] + v as i32,
                ],
                4 => [world[0] + u as i32, world[1] + v as i32, world[2] - 1],
                5 => [
                    world[0] + u as i32,
                    world[1] + v as i32,
                    world[2] + edge as i32,
                ],
                _ => unreachable!(),
            };
            quads.push(GpuQuad {
                origin,
                extent_voxels: [width as u16, height as u16],
                material_face: pack_gpu_source_material(material_face, GPU_SOURCE_FRONTIER),
                ao: 0xff,
            });
        }
    }
    quads
}

fn mesh_casts_directional_shadow(key: &MeshKey) -> bool {
    key.0 == 0 || *key == EXACT_VOLUME_FRONTIER_MESH_KEY
}

fn coalesce_draw_items(mut items: Vec<DrawItem>) -> Vec<DrawSpan> {
    items.sort_unstable_by_key(|item| (item.page, item.offset));
    let mut spans: Vec<DrawSpan> = Vec::with_capacity(items.len());
    for item in items {
        if let Some(last) = spans.last_mut()
            && last.page == item.page
            && last.offset.checked_add(last.size) == Some(item.offset)
            && let (Some(size), Some(quad_count)) = (
                last.size.checked_add(item.size),
                last.quad_count.checked_add(item.quad_count),
            )
        {
            last.size = size;
            last.quad_count = quad_count;
            continue;
        }
        spans.push(DrawSpan {
            page: item.page,
            offset: item.offset,
            size: item.size,
            quad_count: item.quad_count,
        });
    }
    spans
}

fn coalesce_triangle_draw_spans(
    mut items: Vec<TerrainTriangleDrawSpan>,
) -> Vec<TerrainTriangleDrawSpan> {
    items.sort_unstable_by_key(|item| (item.page, item.offset));
    let mut spans: Vec<TerrainTriangleDrawSpan> = Vec::with_capacity(items.len());
    for item in items {
        if let Some(last) = spans.last_mut()
            && last.page == item.page
            && last.offset.checked_add(last.size) == Some(item.offset)
            && let (Some(size), Some(vertex_count)) = (
                last.size.checked_add(item.size),
                last.vertex_count.checked_add(item.vertex_count),
            )
        {
            last.size = size;
            last.vertex_count = vertex_count;
            continue;
        }
        spans.push(item);
    }
    spans
}

const fn placement_material_label(material: Material) -> &'static str {
    match material {
        Material::Grass => "GRASS",
        Material::Dirt => "DIRT",
        Material::Stone => "STONE",
        Material::Sand => "SAND",
        Material::Snow => "SNOW",
        Material::Clay => "CLAY",
        Material::Basalt => "BASALT",
        Material::Wood => "WOOD",
        Material::Leaves => "LEAVES",
        Material::Moss => "MOSS",
        Material::Limestone => "LIMESTONE",
        Material::RedSand => "RED SAND",
        Material::Water => "WATER",
        Material::GlowCrystal => "GLOW CRYSTAL",
        Material::Air => "AIR",
    }
}

const fn inventory_material_code(material: Material) -> &'static str {
    match material {
        Material::Air => "AI",
        Material::Grass => "GR",
        Material::Dirt => "DI",
        Material::Stone => "ST",
        Material::Sand => "SA",
        Material::Snow => "SN",
        Material::Clay => "CL",
        Material::Basalt => "BA",
        Material::Wood => "WO",
        Material::Leaves => "LE",
        Material::Moss => "MO",
        Material::Limestone => "LI",
        Material::RedSand => "RS",
        Material::Water => "WA",
        Material::GlowCrystal => "GL",
    }
}

const fn is_placeable_material(material: Material) -> bool {
    !matches!(material, Material::Air)
}

fn inventory_summary(inventory: &PlacementInventory) -> [String; 2] {
    let half = PLACEMENT_MATERIALS.len().div_ceil(2);
    std::array::from_fn(|line| {
        let range = if line == 0 {
            0..half
        } else {
            half..PLACEMENT_MATERIALS.len()
        };
        range
            .map(|index| {
                let material = PLACEMENT_MATERIALS[index];
                format!(
                    "{} {}",
                    inventory_material_code(material),
                    compact_inventory_count(inventory.count(material))
                )
            })
            .collect::<Vec<_>>()
            .join(" · ")
    })
}

const fn inventory_material_color(material: Material) -> Color {
    let rgb = match material {
        Material::Grass => [0.18, 0.42, 0.12],
        Material::Dirt => [0.36, 0.20, 0.095],
        Material::Stone => [0.34, 0.38, 0.43],
        Material::Sand => [0.72, 0.53, 0.25],
        Material::Snow => [0.76, 0.86, 0.91],
        Material::Clay => [0.56, 0.25, 0.15],
        Material::Basalt => [0.12, 0.15, 0.20],
        Material::Wood => [0.31, 0.15, 0.055],
        Material::Leaves => [0.08, 0.30, 0.10],
        Material::Moss => [0.12, 0.32, 0.14],
        Material::Limestone => [0.58, 0.55, 0.44],
        Material::RedSand => [0.62, 0.20, 0.075],
        Material::Water => [0.02, 0.22, 0.30],
        Material::GlowCrystal => [0.12, 0.58, 0.78],
        Material::Air => [1.0, 0.0, 1.0],
    };
    Color::new(rgb[0], rgb[1], rgb[2], 0.92)
}

fn compact_inventory_count(value: u64) -> String {
    if value >= 1_000_000 {
        format!("{:.1}m", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.1}k", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}

fn sync_inventory_ui(ui: &mut MissionControlUi, inventory: &PlacementInventory) {
    let selected = inventory.selected();
    let materials = inventory.visible_materials();
    let items = materials
        .iter()
        .copied()
        .map(|material| InventoryItem {
            label: placement_material_label(material),
            count: inventory.count(material),
            color: inventory_material_color(material),
        })
        .collect::<Vec<_>>();
    let selected_index =
        selected.and_then(|selected| materials.iter().position(|material| *material == selected));
    ui.set_inventory(
        selected.map(placement_material_label),
        selected.map_or(0, |material| inventory.count(material)),
        inventory_summary(inventory),
        items,
        selected_index,
    );
}

fn local_lights_for_mesh(origin: [i32; 3], mesh: &MeshedChunk) -> Vec<GpuLocalLight> {
    mesh.emissive_clusters
        .iter()
        .filter_map(|cluster| {
            let material = Material::from_id(cluster.material)?;
            let emission = material.emission()?;
            let count = f32::from(cluster.voxel_count);
            let denominator = count * 2.0;
            let position: [f32; 3] = std::array::from_fn(|axis| {
                (origin[axis] as f32 + cluster.position_half_voxel_sum[axis] as f32 / denominator)
                    * VOXEL_SIZE_METRES
            });
            Some(GpuLocalLight {
                position_radius: [
                    position[0],
                    position[1],
                    position[2],
                    emission.radius_metres,
                ],
                color_intensity: [
                    emission.color_linear[0],
                    emission.color_linear[1],
                    emission.color_linear[2],
                    emission.intensity * count.sqrt().min(2.25),
                ],
            })
        })
        .collect()
}

fn rank_local_light<const CAPACITY: usize>(
    ranked: &mut [(f32, GpuLocalLight); CAPACITY],
    count: &mut usize,
    score: f32,
    light: GpuLocalLight,
) {
    let insertion = (0..*count)
        .find(|index| score > ranked[*index].0)
        .unwrap_or(*count);
    if insertion >= CAPACITY {
        return;
    }
    let new_count = (*count + 1).min(CAPACITY);
    for index in (insertion + 1..new_count).rev() {
        ranked[index] = ranked[index - 1];
    }
    ranked[insertion] = (score, light);
    *count = new_count;
}

#[allow(
    clippy::too_many_arguments,
    reason = "the GPU frame contract combines camera, lighting, LOD, interaction, and config state"
)]
fn frame_uniform(
    config: &SurfaceConfiguration,
    camera: &CameraState,
    time: f32,
    target: Option<EditVolume>,
    state: FrameState,
    shadows: &DirectionalShadowCascades,
    renderer_config: RendererConfig,
) -> FrameUniform {
    let FrameState {
        options,
        geometry_source_debug,
        environment,
        world_environment,
        celestial_observation,
        interior,
        direct_light_visibility,
    } = state;
    let view_projection = view_projection(config, camera, renderer_config.view_distance_metres);
    let camera_forward = camera.forward();
    let fluid = camera.fluid_state();
    FrameUniform {
        view_projection: view_projection.to_cols_array_2d(),
        inverse_view_projection: view_projection.inverse().to_cols_array_2d(),
        camera_time: [
            camera.position.x,
            camera.position.y,
            camera.position.z,
            time,
        ],
        viewport_voxel: [
            config.width as f32,
            config.height as f32,
            VOXEL_SIZE_METRES,
            renderer_config.view_distance_metres,
        ],
        target_voxel: target.map_or([0.0; 4], |volume| {
            [
                volume.min.x as f32,
                volume.min.y as f32,
                volume.min.z as f32,
                f32::from(volume.shape().id()) + 1.0,
            ]
        }),
        target_voxel_max: target.map_or([0.0; 4], |volume| {
            [
                volume.max.x as f32,
                volume.max.y as f32,
                volume.max.z as f32,
                0.0,
            ]
        }),
        render_options: [
            if options.ambient_occlusion { 1.0 } else { 0.0 },
            if options.fog { 1.0 } else { 0.0 },
            if options.far_terrain { 1.0 } else { 0.0 },
            if options.target_outline { 1.0 } else { 0.0 },
        ],
        debug_options: [if geometry_source_debug { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0],
        camera_forward: [
            camera_forward.x,
            camera_forward.y,
            camera_forward.z,
            if options.screen_space_ambient_occlusion {
                1.0
            } else {
                0.0
            },
        ],
        shadow_splits: [
            shadows.split_depths[0],
            shadows.split_depths[1],
            shadows.split_depths[2],
            if options.shadows { 1.0 } else { 0.0 },
        ],
        shadow_texel_sizes: [
            shadows.cascades[0].texel_world_size,
            shadows.cascades[1].texel_world_size,
            shadows.cascades[2].texel_world_size,
            1.0 / renderer_config.directional_shadows.shadow_map_resolution as f32,
        ],
        shadow_view_projection: std::array::from_fn(|index| {
            shadows.cascades[index].clip_from_world.to_cols_array_2d()
        }),
        key_light_direction: environment
            .key_light_direction
            .extend(direct_light_visibility)
            .to_array(),
        key_light_radiance: environment
            .key_light_radiance
            .extend(environment.shadow_strength)
            .to_array(),
        sun_direction: environment
            .sun_direction
            .extend(environment.sun_visibility)
            .to_array(),
        moon_direction: environment
            .moon_direction
            .extend(environment.moon_visibility)
            .to_array(),
        equatorial_east: [
            celestial_observation.equatorial_east[0],
            celestial_observation.equatorial_east[1],
            celestial_observation.equatorial_east[2],
            world_environment.twinkle_phase,
        ],
        equatorial_up: [
            celestial_observation.equatorial_up[0],
            celestial_observation.equatorial_up[1],
            celestial_observation.equatorial_up[2],
            celestial_observation.moon_illuminated_fraction,
        ],
        equatorial_north: [
            celestial_observation.equatorial_north[0],
            celestial_observation.equatorial_north[1],
            celestial_observation.equatorial_north[2],
            (world_environment.celestial_seed & 0x00ff_ffff) as f32,
        ],
        environment_time: [
            world_environment.day_fraction,
            world_environment.cloud_offset_metres[0],
            world_environment.cloud_offset_metres[1],
            (world_environment.weather_seed & 0x00ff_ffff) as f32,
        ],
        atmosphere_motion: [
            world_environment.server_time_seconds,
            camera.velocity.x,
            camera.velocity.y,
            camera.velocity.z,
        ],
        sky_horizon: environment.sky_horizon.extend(0.0).to_array(),
        sky_zenith: environment.sky_zenith.extend(0.0).to_array(),
        ground_atmosphere: [
            environment.ground_irradiance.x,
            environment.ground_irradiance.y,
            environment.ground_irradiance.z,
            environment.fog_density,
        ],
        fog_exposure: [
            environment.fog_height_falloff,
            environment.exposure,
            environment.cloud_coverage,
            environment.star_visibility,
        ],
        weather: [
            environment.precipitation,
            environment.storminess,
            environment.cloud_density,
            environment.snow,
        ],
        cloud_layer: [
            world_environment.cloud_base_metres,
            world_environment.cloud_top_metres,
            world_environment.cloud_velocity_metres_per_second[0],
            world_environment.cloud_velocity_metres_per_second[1],
        ],
        medium: [
            water_optical_immersion(fluid),
            fluid.signed_eye_depth_metres,
            fluid.immersion.clamp(0.0, 1.0),
            fluid.surface_y_metres,
        ],
        interior: [
            interior.enclosure,
            interior.exposure_multiplier,
            interior.fog_density,
            if options.cave_headlamp {
                interior.headlamp_strength
            } else {
                0.0
            },
        ],
        diagnostic_sky: renderer_config
            .diagnostic_sky_color
            .map_or([0.0; 4], |color| [color[0], color[1], color[2], 1.0]),
    }
}

fn shadow_frame_uniform(
    shadows: &DirectionalShadowCascades,
    cascade_index: usize,
    camera: &CameraState,
) -> ShadowFrameUniform {
    ShadowFrameUniform {
        clip_from_world: shadows.cascades[cascade_index]
            .clip_from_world
            .to_cols_array_2d(),
        camera_voxel: [
            camera.position.x,
            camera.position.y,
            camera.position.z,
            VOXEL_SIZE_METRES,
        ],
    }
}

fn directional_shadow_cascades(
    config: &SurfaceConfiguration,
    camera: &CameraState,
    light_basis: DirectionalShadowBasis,
    shadow_config: DirectionalShadowConfig,
) -> Result<DirectionalShadowCascades, String> {
    let aspect = config.width as f32 / config.height.max(1) as f32;
    build_directional_shadow_cascades(camera, aspect, light_basis, shadow_config)
        .map_err(|error| format!("build shadow cascades: {error:?}"))
}

fn bounded_frame_delta(dt: f32) -> f32 {
    if dt.is_finite() && dt > 0.0 {
        dt.min(0.1)
    } else {
        0.0
    }
}

fn interior_direct_light_visibility(enclosure: f32, directional_light_occluded: bool) -> f32 {
    let enclosure = enclosure.clamp(0.0, 1.0);
    let existing_interior_attenuation = 1.0 - enclosure * 0.9;
    if !directional_light_occluded {
        return existing_interior_attenuation;
    }
    // Nine upper-hemisphere rays make 8/9 the highest sampled enclosure that still has a known
    // opening. Only fade the final directional contribution after every one is blocked and an
    // independent ray toward the live key light also hits resident canonical terrain.
    let transition = ((enclosure - 8.0 / 9.0) / (0.98 - 8.0 / 9.0)).clamp(0.0, 1.0);
    let sealed = transition * transition * (3.0 - 2.0 * transition);
    existing_interior_attenuation * (1.0 - sealed)
}

fn valid_dpr(dpr: f32) -> f32 {
    if dpr.is_finite() && dpr > 0.0 {
        dpr
    } else {
        1.0
    }
}

fn resize_changes(
    current_width: u32,
    current_height: u32,
    current_dpr: f32,
    width: u32,
    height: u32,
    dpr: f32,
) -> (bool, bool) {
    (
        current_width != width || current_height != height,
        current_dpr != valid_dpr(dpr),
    )
}

const fn refraction_copy_bytes(width: u32, height: u32, active: bool) -> u64 {
    if active {
        // Snapshot both RGBA16F scene color and Depth32Float opaque depth before water writes.
        width as u64 * height as u64 * 12
    } else {
        0
    }
}

fn view_projection(
    config: &SurfaceConfiguration,
    camera: &CameraState,
    view_distance_metres: f32,
) -> glam::Mat4 {
    let aspect = config.width as f32 / config.height.max(1) as f32;
    let projection =
        reverse_z_perspective(68.0f32.to_radians(), aspect, 0.05, view_distance_metres);
    let view =
        glam::camera::rh::view::look_to_mat4(camera.position, camera.forward(), glam::Vec3::Y);
    projection * view
}

fn virtual_terrain_snapshot_identity<'cut>(
    cut: &'cut VirtualTerrainCut,
    envelope: &PresentationEnvelope,
) -> VirtualTerrainSnapshotIdentity<'cut> {
    VirtualTerrainSnapshotIdentity {
        fingerprint: virtual_terrain_presentation_fingerprint(cut, envelope),
        selected_pages: &cut.selected_pages,
        ownerless_roots: cut.ownerless_roots.len() as u32,
    }
}

fn virtual_terrain_presentation_fingerprint(
    cut: &VirtualTerrainCut,
    envelope: &PresentationEnvelope,
) -> u64 {
    (cut.fingerprint ^ envelope.fingerprint().rotate_left(23)).wrapping_mul(0x0000_0100_0000_01b3)
}

fn gpu_feedback_matches_cut(
    feedback: &GpuVirtualTerrainFeedback,
    cut: Option<&VirtualTerrainCut>,
    envelope: Option<&PresentationEnvelope>,
) -> bool {
    let Some((cut, envelope)) = cut.zip(envelope) else {
        return feedback.selected_pages.is_empty()
            && feedback.ownerless_roots == 0
            && !feedback.ownership_overflowed();
    };
    if feedback.submission_id == 0
        || feedback.ownership_overflowed()
        || feedback.oracle_fingerprint != virtual_terrain_presentation_fingerprint(cut, envelope)
        || feedback.ownerless_roots != cut.ownerless_roots.len() as u32
        || feedback.encoded_pages != cut.selected_pages.len() as u32
        || cut.selection_overflow
        || cut.traversal_overflow
    {
        return false;
    }
    let mut selected = feedback.selected_pages.clone();
    selected.sort_unstable();
    selected.dedup();
    selected == cut.selected_pages
}

fn gpu_feedback_match_failure_flags(
    feedback: Option<&GpuVirtualTerrainFeedback>,
    cut: Option<&VirtualTerrainCut>,
    envelope: Option<&PresentationEnvelope>,
    candidate_encode_failed: bool,
) -> u32 {
    const MISSING_FEEDBACK: u32 = 1 << 0;
    const MISSING_CUT: u32 = 1 << 1;
    const INVALID_SUBMISSION: u32 = 1 << 2;
    const OWNERSHIP_OVERFLOW: u32 = 1 << 3;
    const FINGERPRINT_MISMATCH: u32 = 1 << 4;
    const OWNERLESS_MISMATCH: u32 = 1 << 5;
    const ENCODED_COUNT_MISMATCH: u32 = 1 << 6;
    const CPU_OVERFLOW: u32 = 1 << 7;
    const SELECTED_PAGES_MISMATCH: u32 = 1 << 8;
    const CANDIDATE_ENCODE_FAILED: u32 = 1 << 9;

    let Some(feedback) = feedback else {
        return MISSING_FEEDBACK | u32::from(candidate_encode_failed) * CANDIDATE_ENCODE_FAILED;
    };
    let Some((cut, envelope)) = cut.zip(envelope) else {
        return MISSING_CUT | u32::from(candidate_encode_failed) * CANDIDATE_ENCODE_FAILED;
    };
    let mut failures = 0;
    failures |= u32::from(feedback.submission_id == 0) * INVALID_SUBMISSION;
    failures |= u32::from(feedback.ownership_overflowed()) * OWNERSHIP_OVERFLOW;
    failures |= u32::from(
        feedback.oracle_fingerprint != virtual_terrain_presentation_fingerprint(cut, envelope),
    ) * FINGERPRINT_MISMATCH;
    failures |= u32::from(feedback.ownerless_roots != cut.ownerless_roots.len() as u32)
        * OWNERLESS_MISMATCH;
    failures |= u32::from(feedback.encoded_pages != cut.selected_pages.len() as u32)
        * ENCODED_COUNT_MISMATCH;
    failures |= u32::from(cut.selection_overflow || cut.traversal_overflow) * CPU_OVERFLOW;
    let mut selected = feedback.selected_pages.clone();
    selected.sort_unstable();
    selected.dedup();
    failures |= u32::from(selected != cut.selected_pages) * SELECTED_PAGES_MISMATCH;
    failures |= u32::from(candidate_encode_failed) * CANDIDATE_ENCODE_FAILED;
    failures
}

const fn can_present_after_candidate_encode_failure(published_snapshot_is_valid: bool) -> bool {
    published_snapshot_is_valid
}

/// Finite right-handed DirectX/WebGPU projection with near -> 1 and far -> 0.
///
/// Floating-point precision is concentrated near zero, so reversing depth retains useful
/// separation between distant water and terrain instead of quantizing both onto the same plane.
fn reverse_z_perspective(vertical_fov: f32, aspect: f32, near: f32, far: f32) -> glam::Mat4 {
    debug_assert!(vertical_fov > 0.0 && vertical_fov < std::f32::consts::PI);
    debug_assert!(aspect > 0.0);
    debug_assert!(near > 0.0 && far > near);
    let half_fov_tangent = (vertical_fov * 0.5).tan();
    let height = 1.0 / half_fov_tangent;
    let depth_range = far - near;
    glam::Mat4::from_cols(
        glam::Vec4::new(height / aspect, 0.0, 0.0, 0.0),
        glam::Vec4::new(0.0, height, 0.0, 0.0),
        glam::Vec4::new(0.0, 0.0, near / depth_range, -1.0),
        glam::Vec4::new(0.0, 0.0, near * far / depth_range, 0.0),
    )
}

fn water_optical_immersion(fluid: FluidState) -> f32 {
    if !fluid.surface_known || fluid.signed_eye_depth_metres <= 0.0 {
        return 0.0;
    }
    let normalized = (fluid.signed_eye_depth_metres / 0.04).clamp(0.0, 1.0);
    normalized * normalized * (3.0 - 2.0 * normalized)
}

fn water_scene_layout(device: &Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("water scene color and depth layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
        ],
    })
}

struct PipelineOptions<'a> {
    vertex_entry: &'a str,
    fragment_entry: &'a str,
    blend: Option<wgpu::BlendState>,
    write_mask: wgpu::ColorWrites,
    depth_stencil: Option<wgpu::DepthStencilState>,
    fragment_constants: &'a [(&'a str, f64)],
}

#[derive(Clone, Copy)]
struct VoxelPipelineVariant {
    material_detail: bool,
    spatial_ao: bool,
}

impl VoxelPipelineVariant {
    const fn new(material_detail: bool, spatial_ao: bool) -> Self {
        Self {
            material_detail,
            spatial_ao,
        }
    }
}

fn create_voxel_pipeline(
    device: &Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    variant: VoxelPipelineVariant,
) -> RenderPipeline {
    let constants = [(
        "MATERIAL_DETAIL",
        if variant.material_detail { 1.0 } else { 0.0 },
    )];
    pipeline(
        device,
        label,
        layout,
        shader,
        SCENE_FORMAT,
        &[Some(quad_layout())],
        PipelineOptions {
            vertex_entry: "vs_main_fixed",
            fragment_entry: "fs_main",
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(!variant.spatial_ao),
                depth_compare: Some(if variant.spatial_ao {
                    wgpu::CompareFunction::GreaterEqual
                } else {
                    wgpu::CompareFunction::Greater
                }),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            fragment_constants: &constants,
        },
    )
}

fn create_virtual_triangle_pipeline(
    device: &Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    material_detail: bool,
    spatial_ao: bool,
) -> RenderPipeline {
    let constants = [("MATERIAL_DETAIL", if material_detail { 1.0 } else { 0.0 })];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_virtual_triangle_handle"),
            buffers: &[],
            compilation_options: wgpu::PipelineCompilationOptions {
                constants: &constants,
                ..Default::default()
            },
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: SCENE_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions {
                constants: &constants,
                ..Default::default()
            },
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(!spatial_ao),
            depth_compare: Some(if spatial_ao {
                wgpu::CompareFunction::GreaterEqual
            } else {
                wgpu::CompareFunction::Greater
            }),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn create_virtual_surface_pipeline(
    device: &Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    material_detail: bool,
    spatial_ao: bool,
) -> RenderPipeline {
    let constants = [("MATERIAL_DETAIL", if material_detail { 1.0 } else { 0.0 })];
    pipeline(
        device,
        label,
        layout,
        shader,
        SCENE_FORMAT,
        &[],
        PipelineOptions {
            vertex_entry: "vs_virtual_surface_handle",
            fragment_entry: "fs_main",
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(!spatial_ao),
                depth_compare: Some(if spatial_ao {
                    wgpu::CompareFunction::GreaterEqual
                } else {
                    wgpu::CompareFunction::Greater
                }),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            fragment_constants: &constants,
        },
    )
}

fn create_virtual_surface_water_pipeline(
    device: &Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
) -> RenderPipeline {
    pipeline(
        device,
        label,
        layout,
        shader,
        SCENE_FORMAT,
        &[],
        PipelineOptions {
            vertex_entry: "vs_virtual_water_surface_handle",
            fragment_entry: "fs_water",
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            fragment_constants: &[],
        },
    )
}

fn create_virtual_triangle_water_pipeline(
    device: &Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
) -> RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_virtual_water_triangle_handle"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_water"),
            targets: &[Some(wgpu::ColorTargetState {
                format: SCENE_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Greater),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn virtual_triangle_depth_pipeline(
    device: &Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
) -> RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_virtual_triangle_handle"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: None,
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Greater),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn virtual_surface_depth_pipeline(
    device: &Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
) -> RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_virtual_surface_handle"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: None,
        primitive: quad_primitive_state(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Greater),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn virtual_triangle_diagnostic_pipeline(
    device: &Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
) -> RenderPipeline {
    let constants = [("MATERIAL_DETAIL", 0.0)];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_virtual_cluster_diagnostic"),
            buffers: &[
                Some(terrain_triangle_layout()),
                Some(terrain_triangle_diagnostic_owner_layout()),
            ],
            compilation_options: wgpu::PipelineCompilationOptions {
                constants: &constants,
                ..Default::default()
            },
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_diagnostic"),
            targets: &[
                Some(wgpu::ColorTargetState {
                    format: TextureFormat::Rgba32Uint,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                }),
                Some(wgpu::ColorTargetState {
                    format: TextureFormat::R32Uint,
                    blend: None,
                    write_mask: wgpu::ColorWrites::RED,
                }),
            ],
            compilation_options: wgpu::PipelineCompilationOptions {
                constants: &constants,
                ..Default::default()
            },
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Greater),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn create_voxel_diagnostic_pipeline(
    device: &Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
) -> RenderPipeline {
    let constants = [("MATERIAL_DETAIL", 0.0)];
    let buffers = [Some(quad_layout()), Some(diagnostic_owner_layout())];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main_fixed_diagnostic"),
            buffers: &buffers,
            compilation_options: wgpu::PipelineCompilationOptions {
                constants: &constants,
                ..Default::default()
            },
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_diagnostic"),
            targets: &[
                Some(wgpu::ColorTargetState {
                    format: TextureFormat::Rgba32Uint,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                }),
                Some(wgpu::ColorTargetState {
                    format: TextureFormat::R32Uint,
                    blend: None,
                    write_mask: wgpu::ColorWrites::RED,
                }),
            ],
            compilation_options: wgpu::PipelineCompilationOptions {
                constants: &constants,
                ..Default::default()
            },
        }),
        primitive: quad_primitive_state(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Greater),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn pipeline(
    device: &Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    format: TextureFormat,
    buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
    options: PipelineOptions<'_>,
) -> RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(options.vertex_entry),
            buffers,
            compilation_options: wgpu::PipelineCompilationOptions {
                constants: options.fragment_constants,
                ..Default::default()
            },
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(options.fragment_entry),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: options.blend,
                write_mask: options.write_mask,
            })],
            compilation_options: wgpu::PipelineCompilationOptions {
                constants: options.fragment_constants,
                ..Default::default()
            },
        }),
        primitive: quad_primitive_state(),
        depth_stencil: options.depth_stencil,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn fragmentless_depth_pipeline(
    device: &Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
) -> RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main_fixed"),
            buffers: &[Some(quad_layout())],
            compilation_options: Default::default(),
        },
        fragment: None,
        primitive: quad_primitive_state(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Greater),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn shadow_caster_pipeline(
    device: &Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
) -> RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main_fixed"),
            buffers: &[Some(quad_layout())],
            compilation_options: Default::default(),
        },
        fragment: None,
        primitive: quad_primitive_state(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState {
                constant: 2,
                slope_scale: 2.0,
                clamp: 0.0,
            },
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn virtual_triangle_shadow_caster_pipeline(
    device: &Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
) -> RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_virtual_triangle_handle"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: None,
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState {
                constant: 2,
                slope_scale: 2.0,
                clamp: 0.0,
            },
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn virtual_surface_shadow_caster_pipeline(
    device: &Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
) -> RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_virtual_surface_handle"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: None,
        primitive: quad_primitive_state(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState {
                constant: 2,
                slope_scale: 2.0,
                clamp: 0.0,
            },
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn quad_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRIBUTES: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
        0 => Sint32x3,
        1 => Uint16x2,
        2 => Uint32,
        3 => Uint32
    ];
    wgpu::VertexBufferLayout {
        array_stride: size_of::<GpuQuad>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &ATTRIBUTES,
    }
}

fn terrain_triangle_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRIBUTES: [wgpu::VertexAttribute; 3] = wgpu::vertex_attr_array![
        0 => Float32x3,
        1 => Uint32,
        2 => Snorm16x4
    ];
    wgpu::VertexBufferLayout {
        array_stride: size_of::<GpuTerrainVertex>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &ATTRIBUTES,
    }
}

fn terrain_triangle_diagnostic_owner_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRIBUTES: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![3 => Uint32x4];
    wgpu::VertexBufferLayout {
        array_stride: size_of::<[u32; 4]>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &ATTRIBUTES,
    }
}

fn diagnostic_owner_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRIBUTES: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![4 => Uint32x4];
    wgpu::VertexBufferLayout {
        array_stride: size_of::<[u32; 4]>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &ATTRIBUTES,
    }
}

fn quad_primitive_state() -> wgpu::PrimitiveState {
    wgpu::PrimitiveState {
        topology: wgpu::PrimitiveTopology::TriangleStrip,
        ..Default::default()
    }
}

fn preferred_format(formats: &[TextureFormat]) -> TextureFormat {
    formats
        .iter()
        .copied()
        .find(|format| *format == TextureFormat::Bgra8Unorm)
        .or_else(|| {
            formats
                .iter()
                .copied()
                .find(|format| *format == TextureFormat::Rgba8Unorm)
        })
        // Presentation shaders already apply the sRGB transfer function. If the common 8-bit
        // formats are absent, preserve that contract with any other linear surface format before
        // accepting an sRGB target that would encode the output a second time.
        .or_else(|| formats.iter().copied().find(|format| !format.is_srgb()))
        .unwrap_or(formats[0])
}

fn unpack_screenshot_rgba(
    padded: &[u8],
    width: u32,
    height: u32,
    padded_bytes_per_row: u32,
    bgra: bool,
) -> Option<Vec<u8>> {
    let row_bytes = usize::try_from(width.checked_mul(4)?).ok()?;
    let padded_row_bytes = usize::try_from(padded_bytes_per_row).ok()?;
    let height = usize::try_from(height).ok()?;
    if padded_row_bytes < row_bytes || padded.len() < padded_row_bytes.checked_mul(height)? {
        return None;
    }
    let mut rgba = vec![0; row_bytes.checked_mul(height)?];
    for (source, destination) in padded
        .chunks_exact(padded_row_bytes)
        .take(height)
        .zip(rgba.chunks_exact_mut(row_bytes))
    {
        destination.copy_from_slice(&source[..row_bytes]);
        if bgra {
            for pixel in destination.chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }
        }
    }
    Some(rgba)
}

fn unpack_screenshot_diagnostic_rows(
    padded: &[u8],
    width: u32,
    height: u32,
    bytes_per_pixel: u32,
    padded_bytes_per_row: u32,
) -> Option<Vec<u8>> {
    let row_bytes = usize::try_from(width.checked_mul(bytes_per_pixel)?).ok()?;
    let padded_row_bytes = usize::try_from(padded_bytes_per_row).ok()?;
    let height = usize::try_from(height).ok()?;
    if padded_row_bytes < row_bytes || padded.len() < padded_row_bytes.checked_mul(height)? {
        return None;
    }
    let mut attachment = vec![0; row_bytes.checked_mul(height)?];
    for (source, destination) in padded
        .chunks_exact(padded_row_bytes)
        .take(height)
        .zip(attachment.chunks_exact_mut(row_bytes))
    {
        destination.copy_from_slice(&source[..row_bytes]);
    }
    Some(attachment)
}

fn interleave_screenshot_diagnostic(
    identity: &[u8],
    reverse_z: &[u8],
    width: u32,
    height: u32,
) -> Option<Vec<u8>> {
    let pixels = usize::try_from(width.checked_mul(height)?).ok()?;
    if identity.len() != pixels.checked_mul(16)? || reverse_z.len() != pixels.checked_mul(4)? {
        return None;
    }
    let mut interleaved = vec![0; pixels.checked_mul(20)?];
    for pixel in 0..pixels {
        let identity_start = pixel * 16;
        let depth_start = pixel * 4;
        let destination = pixel * 20;
        interleaved[destination..destination + 16]
            .copy_from_slice(&identity[identity_start..identity_start + 16]);
        interleaved[destination + 16..destination + 20]
            .copy_from_slice(&reverse_z[depth_start..depth_start + 4]);
    }
    Some(interleaved)
}

fn hex_bytes(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

fn json_optional_f32(value: Option<f32>) -> String {
    value.map_or_else(|| "null".to_owned(), |value| value.to_string())
}

fn json_optional_vec3(value: Option<[f32; 3]>) -> String {
    value.map_or_else(|| "null".to_owned(), |value| format!("{value:?}"))
}

fn json_string(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len() + 2);
    encoded.push('"');
    for character in value.chars() {
        match character {
            '"' => encoded.push_str("\\\""),
            '\\' => encoded.push_str("\\\\"),
            '\u{08}' => encoded.push_str("\\b"),
            '\u{0c}' => encoded.push_str("\\f"),
            '\n' => encoded.push_str("\\n"),
            '\r' => encoded.push_str("\\r"),
            '\t' => encoded.push_str("\\t"),
            character if character <= '\u{1f}' => {
                let _ = write!(encoded, "\\u{:04x}", character as u32);
            }
            character if !character.is_ascii() => {
                for unit in character.encode_utf16(&mut [0; 2]) {
                    let _ = write!(encoded, "\\u{unit:04x}");
                }
            }
            character => encoded.push(character),
        }
    }
    encoded.push('"');
    encoded
}

fn screenshot_runtime_identity_json(identity: Option<&ScreenshotReproductionIdentity>) -> String {
    identity.map_or_else(
        || "null".to_owned(),
        |identity| {
            format!(
                concat!(
                    r#"{{"buildCommit":{},"buildDirty":{},"buildProfile":{},"#,
                    r#""protocolVersion":{},"clientConfigHash":{}}}"#
                ),
                json_string(&identity.build_commit),
                identity.build_dirty,
                json_string(&identity.build_profile),
                identity.protocol_version,
                json_string(&identity.client_config_hash),
            )
        },
    )
}

fn screenshot_gpu_identity_json(identity: &ScreenshotGpuIdentity) -> String {
    format!(
        concat!(
            r#"{{"adapterName":{},"vendor":{},"device":{},"deviceType":{},"#,
            r#""devicePciBusId":{},"driver":{},"driverInfo":{},"backend":{},"#,
            r#""subgroupMinSize":{},"subgroupMaxSize":{},"supportedFeatures":["{:016x}","{:016x}"],"#,
            r#""enabledFeatures":["{:016x}","{:016x}"],"limits":{}}}"#
        ),
        json_string(&identity.name),
        identity.vendor,
        identity.device,
        json_string(&identity.device_type),
        json_string(&identity.device_pci_bus_id),
        json_string(&identity.driver),
        json_string(&identity.driver_info),
        json_string(&identity.backend),
        identity.subgroup_min_size,
        identity.subgroup_max_size,
        identity.supported_features[0],
        identity.supported_features[1],
        identity.enabled_features[0],
        identity.enabled_features[1],
        json_string(&identity.limits),
    )
}

fn screenshot_streaming_manifest_json(manifest: &ScreenshotStreamingManifest) -> String {
    let mut canonical_pages = manifest.canonical_pages.clone();
    canonical_pages.sort_unstable_by_key(|page| {
        (
            page.coord.x,
            page.coord.y,
            page.coord.z,
            page.phase,
            page.revision,
        )
    });
    let mut virtual_regions = manifest.virtual_regions.clone();
    virtual_regions.sort_unstable_by_key(|region| region.root);
    let mut virtual_columns = manifest.virtual_columns.clone();
    virtual_columns.sort_unstable_by_key(|column| column.column);
    let mut encoded = String::from(r#"{"canonicalPages":["#);
    for (index, page) in canonical_pages.iter().enumerate() {
        if index != 0 {
            encoded.push(',');
        }
        let _ = write!(
            encoded,
            concat!(
                r#"{{"key":"canonical:{}:{}:{}","x":{},"y":{},"z":{},"revision":"{}","#,
                r#""phase":{},"desired":{}}}"#
            ),
            page.coord.x,
            page.coord.y,
            page.coord.z,
            page.coord.x,
            page.coord.y,
            page.coord.z,
            page.revision,
            page.phase,
            page.desired,
        );
    }
    encoded.push_str("],\"virtualColumns\":[");
    for (index, column) in virtual_columns.iter().enumerate() {
        if index != 0 {
            encoded.push(',');
        }
        let _ = write!(
            encoded,
            concat!(
                r#"{{"key":"virtual-column:{}:{}","x":{},"z":{},"resolvedRevision":{},"#,
                r#""minimumRevision":"{}","inFlight":{}}}"#
            ),
            column.column[0],
            column.column[1],
            column.column[0],
            column.column[1],
            json_optional_u64(column.resolved_revision),
            column.minimum_revision,
            column.in_flight,
        );
    }
    encoded.push_str("],\"virtualRegions\":[");
    for (index, region) in virtual_regions.iter().enumerate() {
        if index != 0 {
            encoded.push(',');
        }
        let _ = write!(
            encoded,
            concat!(
                r#"{{"key":"virtual:{}:{}:{}:{}","level":{},"x":{},"y":{},"z":{},"#,
                r#""minimumRevision":"{}","registered":{},"inFlight":{}}}"#
            ),
            region.root.level,
            region.root.coord[0],
            region.root.coord[1],
            region.root.coord[2],
            region.root.level,
            region.root.coord[0],
            region.root.coord[1],
            region.root.coord[2],
            region.minimum_revision,
            region.registered,
            region.in_flight,
        );
    }
    let _ = write!(
        encoded,
        concat!(
            r#"],"virtualStream":{{"pendingPages":{},"inFlightPages":{},"obsoleteInFlightPages":{},"#,
            r#""cancelledPendingPages":"{}","usefulBytes":"{}","cancellationWasteBytes":"{}","#,
            r#""failedPages":"{}","cachePages":{},"cacheBytes":"{}"}}}}"#
        ),
        manifest.virtual_pending_pages,
        manifest.virtual_in_flight_pages,
        manifest.virtual_obsolete_in_flight_pages,
        manifest.virtual_cancelled_pending_pages,
        manifest.virtual_useful_bytes,
        manifest.virtual_cancellation_waste_bytes,
        manifest.virtual_failed_pages,
        manifest.virtual_cache_pages,
        manifest.virtual_cache_bytes,
    );
    encoded
}

fn json_optional_u64(value: Option<u64>) -> String {
    value.map_or_else(|| "null".to_owned(), |value| format!(r#""{value}""#))
}

fn screenshot_virtual_terrain_manifest_json(
    mode: VirtualTerrainRenderMode,
    resident: &BTreeMap<TerrainPageKey, VirtualTerrainGpuPage>,
    retired_published: &BTreeMap<TerrainPageKey, VirtualTerrainGpuPage>,
    published_cut: Option<&VirtualTerrainCut>,
    oracle_cut: Option<&VirtualTerrainCut>,
    exact_surface_domain: Option<&ExactSurfaceDomain>,
    feedback: Option<&GpuVirtualTerrainFeedback>,
) -> String {
    let mode = match mode {
        VirtualTerrainRenderMode::Disabled => "disabled",
        VirtualTerrainRenderMode::Shadow => "shadow",
        VirtualTerrainRenderMode::Visible => "visible",
    };
    let published = screenshot_virtual_cut_json(published_cut);
    let oracle = screenshot_virtual_cut_json(oracle_cut);
    let exact_surface_domain = exact_surface_domain.map_or_else(
        || "null".to_owned(),
        |domain| {
            format!(
                concat!(
                    r#"{{"complete":{},"requiredLeaves":{},"fingerprint":"{:016x}","#,
                    r#""currentExactCoverage":{},"oracleExactCoverage":{},"#,
                    r#""coreComplete":{},"coreRequiredLeaves":{},"coreFingerprint":"{:016x}","#,
                    r#""coreCurrentCoverage":{},"coreOracleCoverage":{},"#,
                    r#""predictionComplete":{},"predictionRequiredLeaves":{},"#,
                    r#""predictionFingerprint":"{:016x}","predictionCurrentCoverage":{},"#,
                    r#""predictionOracleCoverage":{}}}"#
                ),
                domain.is_complete(),
                domain.required_leaf_count(),
                domain.fingerprint(),
                published_cut.map_or(0, |cut| cut.exact_surface_coverage(domain)),
                oracle_cut.map_or(0, |cut| cut.exact_surface_coverage(domain)),
                domain.core_is_complete(),
                domain.core_required_leaf_count(),
                domain.core_fingerprint(),
                published_cut.map_or(0, |cut| cut.exact_surface_core_coverage(domain)),
                oracle_cut.map_or(0, |cut| cut.exact_surface_core_coverage(domain)),
                domain.prediction_is_complete(),
                domain.prediction_required_leaf_count(),
                domain.prediction_fingerprint(),
                if domain.prediction_is_complete() {
                    published_cut.map_or(0, |cut| cut.exact_surface_coverage(domain))
                } else {
                    0
                },
                if domain.prediction_is_complete() {
                    oracle_cut.map_or(0, |cut| cut.exact_surface_coverage(domain))
                } else {
                    0
                },
            )
        },
    );
    let mut encoded = format!(
        concat!(
            r#"{{"mode":"{}","exactSurfaceDomain":{},"#,
            r#""publishedCut":{},"oracleCut":{},"residentPages":["#
        ),
        mode, exact_surface_domain, published, oracle,
    );
    for (index, (key, page)) in resident.iter().enumerate() {
        if index != 0 {
            encoded.push(',');
        }
        let _ = write!(
            encoded,
            concat!(
                r#"{{"level":{},"coord":{:?},"revision":"{}","contentFingerprint":"{}","#,
                r#""representation":"{}","representationKind":{},"heightfieldExactNeighborSides":{:?},"#,
                r#""heightfieldFinerNeighborSides":{:?},"heightfieldGroundCornerBits":{:?},"#,
                r#""heightfieldGroundBoundaryBits":{:?}}}"#
            ),
            key.level,
            key.coord,
            page.revision,
            hex_bytes(&page.content_fingerprint),
            virtual_representation_label(page.representation),
            page.representation as u8,
            page.heightfield_exact_neighbor_sides,
            page.heightfield_finer_neighbor_sides,
            page.heightfield_ground_corner_bits,
            page.heightfield_ground_boundary_bits,
        );
    }
    encoded.push_str("],\"retiredPublishedPages\":[");
    for (index, (key, page)) in retired_published.iter().enumerate() {
        if index != 0 {
            encoded.push(',');
        }
        let _ = write!(
            encoded,
            concat!(
                r#"{{"level":{},"coord":{:?},"revision":"{}","contentFingerprint":"{}","#,
                r#""representation":"{}","representationKind":{},"heightfieldExactNeighborSides":{:?},"#,
                r#""heightfieldFinerNeighborSides":{:?},"heightfieldGroundCornerBits":{:?},"#,
                r#""heightfieldGroundBoundaryBits":{:?}}}"#
            ),
            key.level,
            key.coord,
            page.revision,
            hex_bytes(&page.content_fingerprint),
            virtual_representation_label(page.representation),
            page.representation as u8,
            page.heightfield_exact_neighbor_sides,
            page.heightfield_finer_neighbor_sides,
            page.heightfield_ground_corner_bits,
            page.heightfield_ground_boundary_bits,
        );
    }
    encoded.push_str("],\"gpuFeedback\":");
    if let Some(feedback) = feedback {
        let _ = write!(
            encoded,
            concat!(
                r#"{{"submissionId":"{}","oracleFingerprint":"{:016x}","#,
                r#""ownerlessRoots":{},"encodingOverflowFlags":{},"encodedPages":{},"#,
                r#""encodedOpaqueSurfaceHandles":{},"encodedOpaqueTriangleHandles":{},"#,
                r#""encodedWaterSurfaceHandles":{},"encodedWaterTriangleHandles":{},"#,
                r#""selectedPages":["#
            ),
            feedback.submission_id,
            feedback.oracle_fingerprint,
            feedback.ownerless_roots,
            feedback.encoding_overflow_flags,
            feedback.encoded_pages,
            feedback.encoded_surface_handles,
            feedback.encoded_triangle_handles,
            feedback.encoded_water_surface_handles,
            feedback.encoded_water_triangle_handles,
        );
        write_virtual_page_keys(&mut encoded, &feedback.selected_pages);
        encoded.push_str("]}");
    } else {
        encoded.push_str("null");
    }
    encoded.push('}');
    encoded
}

fn screenshot_virtual_cut_json(cut: Option<&VirtualTerrainCut>) -> String {
    let Some(cut) = cut else {
        return "null".to_owned();
    };
    let mut encoded = format!(
        concat!(
            r#"{{"fingerprint":"{:016x}","renderable":{},"visitedNodes":{},"#,
            r#""selectedPrimitives":{},"selectedEncodedBytes":{},"feedbackOverflow":{},"#,
            r#""selectionOverflow":{},"traversalOverflow":{},"incoherentReplacementGroups":{},"exactSurfaceLodDiscontinuities":{},"#,
            r#""selectedPages":["#
        ),
        cut.fingerprint,
        cut.is_renderable(),
        cut.visited_nodes,
        cut.selected_primitives,
        cut.selected_encoded_bytes,
        cut.feedback_overflow,
        cut.selection_overflow,
        cut.traversal_overflow,
        cut.incoherent_replacement_groups,
        cut.exact_surface_lod_discontinuities,
    );
    write_virtual_page_keys(&mut encoded, &cut.selected_pages);
    encoded.push_str("],\"requestedPages\":[");
    for (index, request) in cut.requested_pages.iter().enumerate() {
        if index != 0 {
            encoded.push(',');
        }
        let _ = write!(
            encoded,
            r#"{{"level":{},"coord":{:?},"revision":"{}","contentFingerprint":"{}"}}"#,
            request.key.level,
            request.key.coord,
            request.revision,
            hex_bytes(&request.content_fingerprint),
        );
    }
    encoded.push_str("],\"refinementRoots\":[");
    write_virtual_page_keys(&mut encoded, &cut.refinement_roots);
    encoded.push_str("],\"ownerlessRoots\":[");
    write_virtual_page_keys(&mut encoded, &cut.ownerless_roots);
    encoded.push_str("]}");
    encoded
}

fn write_virtual_page_keys(encoded: &mut String, pages: &[TerrainPageKey]) {
    for (index, key) in pages.iter().enumerate() {
        if index != 0 {
            encoded.push(',');
        }
        let _ = write!(
            encoded,
            r#"{{"level":{},"coord":{:?}}}"#,
            key.level, key.coord
        );
    }
}

const fn virtual_representation_label(kind: TerrainPageRepresentationKind) -> &'static str {
    match kind {
        TerrainPageRepresentationKind::SteppedSurfaceResidual => "steppedSurfaceResidual",
        TerrainPageRepresentationKind::SparseVoxelBrick => "sparseVoxelBrick",
        TerrainPageRepresentationKind::SurfaceCluster => "surfaceCluster",
        TerrainPageRepresentationKind::TriangleCluster => "triangleCluster",
        TerrainPageRepresentationKind::HeightfieldGrid => "heightfieldGrid",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_heightfield_boundary_midpoints(
        key: TerrainPageKey,
        sample: voxels_world::SurfaceSample,
    ) -> Vec<voxels_world::SurfaceSample> {
        if key.level == 0 {
            Vec::new()
        } else {
            vec![sample; 4 * TERRAIN_PAGE_EDGE_SAMPLES as usize]
        }
    }

    #[test]
    fn screenshot_json_escaping_preserves_adapter_text_as_data() {
        assert_eq!(
            json_string("gpu \"driver\"\\Málaga\n\u{0001}"),
            "\"gpu \\\"driver\\\"\\\\M\\u00e1laga\\n\\u0001\""
        );
    }

    #[test]
    fn screenshot_streaming_manifest_includes_virtual_transport_state() {
        let manifest = ScreenshotStreamingManifest {
            virtual_columns: vec![ScreenshotVirtualColumnState {
                column: [-2, 4],
                resolved_revision: Some(16),
                minimum_revision: 15,
                in_flight: true,
            }],
            virtual_regions: vec![ScreenshotVirtualRegionState {
                root: TerrainPageKey {
                    level: TERRAIN_REGION_ROOT_LEVEL,
                    coord: [-2, 3, 4],
                },
                minimum_revision: 17,
                registered: true,
                in_flight: false,
            }],
            virtual_pending_pages: 5,
            virtual_in_flight_pages: 6,
            virtual_obsolete_in_flight_pages: 2,
            virtual_cancelled_pending_pages: 7,
            virtual_useful_bytes: 8,
            virtual_cancellation_waste_bytes: 9,
            virtual_failed_pages: 10,
            virtual_cache_pages: 11,
            virtual_cache_bytes: 12,
            ..ScreenshotStreamingManifest::default()
        };
        assert_eq!(
            screenshot_streaming_manifest_json(&manifest),
            concat!(
                r#"{"canonicalPages":[],"virtualColumns":["#,
                r#"{"key":"virtual-column:-2:4","x":-2,"z":4,"resolvedRevision":"16","minimumRevision":"15","inFlight":true}],"virtualRegions":["#,
                r#"{"key":"virtual:2:-2:3:4","level":2,"x":-2,"y":3,"z":4,"minimumRevision":"17","registered":true,"inFlight":false}],"#,
                r#""virtualStream":{"pendingPages":5,"inFlightPages":6,"obsoleteInFlightPages":2,"cancelledPendingPages":"7","usefulBytes":"8","#,
                r#""cancellationWasteBytes":"9","failedPages":"10","cachePages":11,"cacheBytes":"12"}}"#
            )
        );
    }

    #[test]
    fn virtual_screenshot_manifest_records_cut_residency_and_gpu_certificate() {
        let key = TerrainPageKey {
            level: 1,
            coord: [-2, 3, 4],
        };
        let mut resident = BTreeMap::new();
        resident.insert(
            key,
            VirtualTerrainGpuPage {
                revision: 17,
                content_fingerprint: [0xab; 32],
                representation: TerrainPageRepresentationKind::SparseVoxelBrick,
                heightfield_exact_neighbor_sides: [false; 4],
                heightfield_finer_neighbor_sides: [false; 4],
                heightfield_ground_corner_bits: [0; 4],
                heightfield_ground_boundary_bits: [[0; TERRAIN_PAGE_EDGE_SAMPLES as usize + 1]; 4],
                mesh: VirtualTerrainGpuMesh::Empty,
            },
        );
        let cut = VirtualTerrainCut {
            selected_pages: vec![key],
            requested_pages: Vec::new(),
            refinement_roots: Vec::new(),
            ownerless_roots: Vec::new(),
            fingerprint: 0x1234,
            visited_nodes: 1,
            selected_primitives: 2,
            selected_encoded_bytes: 3,
            feedback_overflow: false,
            selection_overflow: false,
            traversal_overflow: false,
            incoherent_replacement_groups: 0,
            exact_surface_lod_discontinuities: 0,
        };
        let feedback = GpuVirtualTerrainFeedback {
            submission_id: 8,
            oracle_fingerprint: cut.fingerprint,
            selected_pages: vec![key],
            encoded_pages: 1,
            ..GpuVirtualTerrainFeedback::default()
        };
        let exact_surface_domain = ExactSurfaceDomain::swept_horizontal_capsule(
            [-6.0, 3.0, 13.0],
            [-6.0, 3.0, 13.0],
            0.0,
            16,
        );
        let manifest = screenshot_virtual_terrain_manifest_json(
            VirtualTerrainRenderMode::Visible,
            &resident,
            &BTreeMap::new(),
            Some(&cut),
            Some(&cut),
            Some(&exact_surface_domain),
            Some(&feedback),
        );
        assert!(manifest.contains(r#""mode":"visible""#));
        assert!(manifest.contains(
            r#""exactSurfaceDomain":{"complete":true,"requiredLeaves":1,"fingerprint":"#
        ));
        assert!(manifest.contains(r#""currentExactCoverage":0,"oracleExactCoverage":0"#));
        assert!(manifest.contains(r#""coord":[-2, 3, 4]"#));
        assert!(manifest.contains(r#""revision":"17""#));
        assert!(manifest.contains(r#""representation":"sparseVoxelBrick""#));
        assert!(manifest.contains(r#""submissionId":"8""#));
        assert!(manifest.contains(r#""oracleFingerprint":"0000000000001234""#));
        assert!(manifest.contains(&"ab".repeat(32)));
    }

    #[test]
    fn failed_candidate_encoding_keeps_a_valid_published_snapshot_visible() {
        assert!(
            can_present_after_candidate_encode_failure(true),
            "a missing or invalid candidate must fail closed for promotion without blanking the immutable old bank"
        );
        assert!(
            !can_present_after_candidate_encode_failure(false),
            "without a certified published bank there is no virtual owner to present"
        );
        assert_ne!(
            gpu_feedback_match_failure_flags(None, None, None, true) & (1 << 9),
            0,
            "the fail-open presentation path still surfaces the candidate encoding failure"
        );
    }

    #[test]
    fn screenshot_readback_removes_row_padding_and_normalizes_bgra() {
        let mut padded = vec![0xEE; 512];
        padded[..8].copy_from_slice(&[3, 2, 1, 4, 7, 6, 5, 8]);
        padded[256..264].copy_from_slice(&[11, 10, 9, 12, 15, 14, 13, 16]);
        assert_eq!(
            unpack_screenshot_rgba(&padded, 2, 2, 256, true),
            Some(vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16])
        );
        assert_eq!(
            unpack_screenshot_rgba(&padded, 2, 3, 256, false),
            None,
            "incomplete mapped rows must never become a truncated PNG"
        );
    }

    #[test]
    fn geometry_source_tag_preserves_material_face_and_extents_need_no_tag_bits() {
        assert!(
            Material::ALL
                .iter()
                .all(|material| u32::from(material.id()) & GPU_SOURCE_MASK == 0)
        );
        let material_face = pack_gpu_material_face(u32::from(Material::Stone.id()), 5);
        let quad = GpuQuad {
            origin: [0; 3],
            extent_voxels: [u16::MAX, u16::MAX],
            material_face,
            ao: 0,
        };
        let tagged = GpuQuad {
            material_face: pack_gpu_source_material(quad.material_face, GPU_SOURCE_FRONTIER),
            ..quad
        };
        assert_eq!(
            (tagged.material_face >> GPU_SOURCE_SHIFT) & 7,
            GPU_SOURCE_FRONTIER
        );
        assert_eq!(tagged.material_face & !GPU_SOURCE_MASK, material_face);
        assert_eq!(tagged.extent_voxels, quad.extent_voxels);
    }

    fn counts(entries: &[(Material, u64)]) -> [u64; Material::ALL.len()] {
        let mut counts = [0; Material::ALL.len()];
        for &(material, count) in entries {
            counts[usize::from(material.id())] = count;
        }
        counts
    }

    #[test]
    fn exact_volume_frontier_caps_only_the_reported_portal_cells() {
        let coord = ChunkCoord::new(3, -2, 5);
        let mut x_cells = [0_u64; EXACT_VOLUME_FRONTIER_FACE_WORDS];
        for z in 4..6 {
            for y in 7..10 {
                let index = y + z * CHUNK_EDGE;
                x_cells[index / 64] |= 1_u64 << (index % 64);
            }
        }
        let x_quads = frontier_face_gpu_quads(&ExactVolumeFrontierFace {
            chunk: coord,
            face: 0,
            cells: x_cells,
        });
        assert_eq!(x_quads.len(), 1);
        assert_eq!(
            x_quads[0].origin,
            [
                coord.world_origin()[0] - 1,
                coord.world_origin()[1] + 7,
                coord.world_origin()[2] + 4,
            ]
        );
        assert_eq!(x_quads[0].extent_voxels, [2, 3]);
        assert_eq!(
            (x_quads[0].material_face & GPU_SOURCE_MASK) >> GPU_SOURCE_SHIFT,
            GPU_SOURCE_FRONTIER
        );
        assert_eq!(x_quads[0].material_face >> GPU_FACE_SHIFT & 7, 0);

        let mut z_cells = [0_u64; EXACT_VOLUME_FRONTIER_FACE_WORDS];
        for y in 10..12 {
            for x in 2..5 {
                let index = x + y * CHUNK_EDGE;
                z_cells[index / 64] |= 1_u64 << (index % 64);
            }
        }
        let z_quads = frontier_face_gpu_quads(&ExactVolumeFrontierFace {
            chunk: coord,
            face: 5,
            cells: z_cells,
        });
        assert_eq!(z_quads.len(), 1);
        assert_eq!(
            z_quads[0].origin,
            [
                coord.world_origin()[0] + 2,
                coord.world_origin()[1] + 10,
                coord.world_origin()[2] + CHUNK_EDGE as i32,
            ]
        );
        assert_eq!(z_quads[0].extent_voxels, [3, 2]);
        assert_eq!(
            (z_quads[0].material_face & GPU_SOURCE_MASK) >> GPU_SOURCE_SHIFT,
            GPU_SOURCE_FRONTIER
        );
        assert_eq!(z_quads[0].material_face >> GPU_FACE_SHIFT & 7, 5);

        assert!(
            frontier_face_gpu_quads(&ExactVolumeFrontierFace {
                chunk: coord,
                face: 6,
                cells: [u64::MAX; EXACT_VOLUME_FRONTIER_FACE_WORDS],
            })
            .is_empty()
        );
    }

    #[test]
    fn canonical_greedy_t_junctions_split_only_at_real_top_vertices() {
        let top = |origin, extent| Quad {
            origin,
            face: 2,
            extent,
            material: Material::Stone.id(),
            ao: 0xff,
            _pad: 0,
        };
        let large = top([4, 0, 4], [4, 4]);
        assert_eq!(
            canonical_gpu_quads([0; 3], &[large]),
            vec![canonical_gpu_quad([0; 3], &large)],
            "an isolated greedy face keeps its single compact instance"
        );

        let adjacent = top([8, 0, 6], [1, 1]);
        let constrained = canonical_gpu_quads([0; 3], &[large, adjacent]);
        let large_triangles = constrained
            .iter()
            .filter(|quad| {
                quad.origin == [4, 0, 4] && quad.extent_voxels[0] & CANONICAL_TRIANGLE_FLAG != 0
            })
            .collect::<Vec<_>>();
        assert_eq!(
            large_triangles.len(),
            4,
            "three exact segments on the touched edge plus one unsplit fill triangle"
        );
        let positive_x_offsets = large_triangles
            .iter()
            .filter(|quad| {
                (quad.extent_voxels[0] >> CANONICAL_TRIANGLE_EDGE_SHIFT) & 3
                    == QuadEdge::PositiveX.index() as u16
            })
            .map(|quad| {
                [
                    quad.extent_voxels[0] & CANONICAL_TRIANGLE_OFFSET_MASK,
                    quad.extent_voxels[1] & CANONICAL_TRIANGLE_OFFSET_MASK,
                ]
            })
            .collect::<Vec<_>>();
        assert_eq!(positive_x_offsets, [[0, 2], [2, 3], [3, 4]]);
        assert_eq!(
            constrained
                .iter()
                .filter(|quad| quad.origin == [8, 0, 6])
                .count(),
            1,
            "the unit neighbor already has the exact shared endpoints"
        );
    }

    #[test]
    fn every_canonical_chunk_boundary_face_uses_ten_centimetre_raster_edges() {
        let boundary_faces = [
            (0, [31, 4, 4]),
            (1, [0, 4, 4]),
            (2, [4, 31, 4]),
            (3, [4, 0, 4]),
            (4, [4, 4, 31]),
            (5, [4, 4, 0]),
        ];
        for (face, origin) in boundary_faces {
            let quad = Quad {
                origin,
                face,
                extent: [4, 4],
                material: Material::Stone.id(),
                ao: 0xff,
                _pad: 0,
            };
            let constrained = canonical_gpu_quads([0; 3], &[quad]);
            assert_eq!(
                constrained.len(),
                16,
                "face {face} must split all four boundary edges into four unit segments"
            );
            assert!(constrained.iter().all(|triangle| {
                triangle.extent_voxels[0] & CANONICAL_TRIANGLE_FLAG != 0
                    && (triangle.extent_voxels[1] & CANONICAL_TRIANGLE_OFFSET_MASK)
                        - (triangle.extent_voxels[0] & CANONICAL_TRIANGLE_OFFSET_MASK)
                        == 1
            }));
        }
    }

    #[test]
    fn vertical_t_junctions_use_shared_lattice_vertices_without_unbounded_wall_instances() {
        let large = GpuQuad {
            origin: [0, 0, 0],
            extent_voxels: [2, 63],
            material_face: pack_gpu_material_face(u32::from(Material::Stone.id()), 0),
            ao: 0xff,
        };
        let neighbor = GpuQuad {
            origin: [0, 20, 1],
            extent_voxels: [1, 10],
            material_face: pack_gpu_material_face(u32::from(Material::Stone.id()), 4),
            ao: 0xff,
        };
        let constrained = constrain_gpu_quad_t_junctions(
            &[large, neighbor],
            |_, _| true,
            |_, _, _, _| false,
            true,
        );
        assert_eq!(constrained[0].len(), 4);
        assert_eq!(
            constrained[0]
                .iter()
                .filter(|quad| {
                    (quad.extent_voxels[0] >> CANONICAL_TRIANGLE_EDGE_SHIFT) & 3
                        == QuadEdge::PositiveX.index() as u16
                })
                .map(|quad| {
                    [
                        quad.extent_voxels[0] & CANONICAL_TRIANGLE_OFFSET_MASK,
                        quad.extent_voxels[1] & CANONICAL_TRIANGLE_OFFSET_MASK,
                    ]
                })
                .collect::<Vec<_>>(),
            [[0, 20], [20, 30], [30, 63]]
        );
        assert_eq!(
            constrained[0]
                .iter()
                .filter(|quad| {
                    quad.extent_voxels[1] & CANONICAL_TRIANGLE_SHADOW_OWNER_FLAG != 0
                })
                .count(),
            1
        );
        let unpack_extent = |encoded: u16| {
            (((encoded >> CANONICAL_TRIANGLE_EXTENT_SHIFT) & 31)
                | ((encoded >> (CANONICAL_TRIANGLE_EXTENT_SHIFT + 4)) & 32))
                + 1
        };
        assert!(constrained[0].iter().all(|quad| {
            unpack_extent(quad.extent_voxels[0]) == 2 && unpack_extent(quad.extent_voxels[1]) == 63
        }));
    }

    #[test]
    fn adjacent_gpu_quads_convert_the_same_integer_corner_to_identical_float_bits() {
        for origin in (-20_000..=20_000).step_by(97) {
            for extent in [1_u16, 2, 4, 8, 16, 32, 64, 255] {
                let left = GpuQuad {
                    origin: [origin, -31, 47],
                    extent_voxels: [extent, 1],
                    material_face: pack_gpu_material_face(u32::from(Material::Grass.id()), 2),
                    ao: 0,
                };
                let right = GpuQuad {
                    origin: [origin + i32::from(extent), -31, 47],
                    ..left
                };
                let left_endpoint =
                    (left.origin[0] + i32::from(left.extent_voxels[0])) as f32 * VOXEL_SIZE_METRES;
                let right_origin = right.origin[0] as f32 * VOXEL_SIZE_METRES;
                assert_eq!(left_endpoint.to_bits(), right_origin.to_bits());
            }
        }
    }

    #[test]
    fn placement_inventory_follows_authoritative_stock_and_skips_empty_materials() {
        let mut inventory = PlacementInventory::new();
        assert_eq!(inventory.selected(), None);
        assert!(!inventory.cycle(1));
        inventory.set_counts(counts(&[(Material::Dirt, 12), (Material::Water, 3)]));
        assert_eq!(inventory.selected(), Some(Material::Dirt));
        assert_eq!(inventory.count(Material::Dirt), 12);
        assert!(!inventory.select(Material::Stone));
        assert!(!inventory.select(Material::Air));
        assert!(inventory.cycle(1));
        assert_eq!(inventory.selected(), Some(Material::Water));
        assert!(inventory.cycle(-1));
        assert_eq!(inventory.selected(), Some(Material::Dirt));
    }

    #[test]
    fn placement_inventory_exposes_ten_keyboard_slots_around_the_selection() {
        let mut inventory = PlacementInventory::new();
        inventory.set_counts(std::array::from_fn(|index| u64::from(index > 0)));
        assert_eq!(inventory.visible_materials().len(), MATERIAL_WHEEL_SLOTS);
        assert!(inventory.select(Material::GlowCrystal));
        let visible = inventory.visible_materials();
        assert!(visible.contains(&Material::GlowCrystal));
        let expected = visible[3];
        assert!(inventory.select_visible_slot(3));
        assert_eq!(inventory.selected(), Some(expected));
    }

    #[test]
    fn every_non_air_material_is_placeable_and_visible_in_the_inventory_summary() {
        assert_eq!(PLACEMENT_MATERIALS.len(), Material::ALL.len() - 1);
        assert!(
            PLACEMENT_MATERIALS
                .iter()
                .all(|material| *material != Material::Air)
        );
        assert!(
            Material::ALL
                .into_iter()
                .filter(|material| *material != Material::Air)
                .all(|material| PLACEMENT_MATERIALS.contains(&material))
        );

        let mut inventory = PlacementInventory::new();
        inventory.set_counts(std::array::from_fn(|index| index as u64));
        let summary = inventory_summary(&inventory).join(" / ");
        for material in PLACEMENT_MATERIALS {
            assert!(summary.contains(inventory_material_code(material)));
            assert_ne!(placement_material_label(material), "AIR");
        }
    }

    fn mixed_feature_baseline() -> RendererFeatureConfig {
        RendererFeatureConfig {
            cascaded_sun_shadows: false,
            voxel_ambient_occlusion: true,
            screen_space_ambient_occlusion: false,
            atmospheric_fog: true,
            far_terrain: false,
            water_surface: true,
            target_outline: false,
            material_surface_detail: true,
            cave_headlamp: false,
            voxel_emissive_lights: true,
        }
    }

    #[test]
    fn configured_feature_baseline_drives_initial_options() {
        let baseline = mixed_feature_baseline();
        let expected = RenderOptions::from(baseline);
        assert_eq!(
            expected,
            RenderOptions {
                shadows: false,
                ambient_occlusion: true,
                screen_space_ambient_occlusion: false,
                fog: true,
                far_terrain: false,
                water: true,
                target_outline: false,
                material_detail: true,
                cave_headlamp: false,
                local_lighting: true,
            }
        );
    }

    #[test]
    fn shadow_allocation_is_bounded_by_device_limits_and_memory_budget() {
        assert_eq!(validate_shadow_allocation(1_024, 8_192), Ok(()));
        assert_eq!(validate_shadow_allocation(4_096, 8_192), Ok(()));
        assert!(validate_shadow_allocation(0, 8_192).is_err());
        assert!(validate_shadow_allocation(4_096, 2_048).is_err());
        assert!(validate_shadow_allocation(8_192, 16_384).is_err());
    }

    #[test]
    fn surface_format_fallback_keeps_explicit_srgb_encoding_linear() {
        assert_eq!(
            preferred_format(&[
                TextureFormat::Bgra8UnormSrgb,
                TextureFormat::Rgba16Float,
                TextureFormat::Rgba8UnormSrgb,
            ]),
            TextureFormat::Rgba16Float
        );
        assert_eq!(
            preferred_format(&[TextureFormat::Rgba8UnormSrgb, TextureFormat::Bgra8Unorm]),
            TextureFormat::Bgra8Unorm
        );
    }

    #[test]
    fn radial_and_portal_activation_reasons_do_not_disable_each_other() {
        let key = (0, 3, -2, 7);
        let mut activations = ChunkActivations::default();
        let radial = activations.set(key, ChunkActivationReason::Radial, true);
        assert_eq!(activations.upload_mask(key), radial);
        let both = activations.set(key, ChunkActivationReason::Portal, true);
        assert_ne!(both, 0);
        assert_eq!(activations.upload_mask(key), both);
        let portal_only = activations.set(key, ChunkActivationReason::Radial, false);
        assert_ne!(portal_only, 0);
        let inactive = activations.set(key, ChunkActivationReason::Portal, false);
        assert_eq!(inactive, 0);
        assert!(!activations.masks.contains_key(&key));
    }

    #[test]
    fn chunk_activation_survives_empty_meshes_but_not_eviction() {
        let key = (0, -4, 8, 12);
        let mut activations = ChunkActivations::default();
        activations.set(key, ChunkActivationReason::Radial, true);
        let both = activations.set(key, ChunkActivationReason::Portal, true);

        // An empty upload does not touch the independent registry, so later opaque and water
        // allocations both inherit the same active reasons.
        assert_eq!(activations.upload_mask(key), both);
        assert_eq!(activations.upload_mask(key), both);
        activations.remove(key);
        assert_eq!(activations.upload_mask(key), 0);

        // Shell reconciliation can clear reasons after eviction without recreating zero tombstones.
        assert_eq!(
            activations.set(key, ChunkActivationReason::Radial, false),
            0
        );
        assert!(activations.masks.is_empty());
        assert_eq!(activations.upload_mask((1, 0, 0, 0)), u8::MAX);
    }

    #[test]
    fn diagnostic_owner_hashes_match_the_attachment_decoder_for_signed_pages() {
        assert_eq!(
            diagnostic_owner_id(1, 0, -9, 4, -7),
            [1_853_617_194, 3_627_557_418]
        );
        assert_eq!(
            diagnostic_owner_id(2, 3, -7, 0, 13),
            [1_589_663_946, 251_670_784]
        );
        assert_eq!(
            diagnostic_owner_id(3, 255, 2, 0, 0),
            [964_022_151, 1_104_392_134]
        );
    }

    fn test_view_projection(camera: &CameraState) -> glam::Mat4 {
        reverse_z_perspective(68.0f32.to_radians(), 1.0, 0.01, 80.0)
            * glam::camera::rh::view::look_to_mat4(camera.position, camera.forward(), glam::Vec3::Y)
    }

    #[test]
    fn reverse_z_projection_maps_near_and_far_to_the_webgpu_depth_interval() {
        let near = 0.05;
        let far = 3_200.0;
        let projection = reverse_z_perspective(68.0f32.to_radians(), 16.0 / 9.0, near, far);
        let depth = |distance: f32| {
            let clip = projection * glam::Vec4::new(0.0, 0.0, -distance, 1.0);
            clip.z / clip.w
        };

        assert!((depth(near) - 1.0).abs() <= f32::EPSILON * 2.0);
        assert!(depth(far).abs() <= f32::EPSILON);
        assert!(depth(1.0) > depth(10.0));
        assert!(depth(10.0) > depth(1_000.0));
        assert!(depth(near * 0.5) > 1.0);
        assert!(depth(far * 2.0) < 0.0);
    }

    #[test]
    fn optical_immersion_is_spatial_and_continuous_at_the_free_surface() {
        let fluid = |signed_eye_depth_metres| FluidState {
            signed_eye_depth_metres,
            surface_known: true,
            ..FluidState::default()
        };

        assert_eq!(water_optical_immersion(FluidState::default()), 0.0);
        assert_eq!(water_optical_immersion(fluid(-0.001)), 0.0);
        assert_eq!(water_optical_immersion(fluid(0.0)), 0.0);
        let just_below = water_optical_immersion(fluid(0.001));
        assert!(just_below > 0.0 && just_below < 0.01);
        assert!((water_optical_immersion(fluid(0.02)) - 0.5).abs() < 0.0001);
        assert_eq!(water_optical_immersion(fluid(0.04)), 1.0);
        assert_eq!(water_optical_immersion(fluid(0.4)), 1.0);
    }

    #[test]
    fn meshed_emissive_clusters_become_linear_world_space_lights() {
        let mut mesh = MeshedChunk::default();
        mesh.emissive_clusters.push(voxels_world::EmissiveCluster {
            position_half_voxel_sum: [18, 22, 26],
            voxel_count: 2,
            material: Material::GlowCrystal.id(),
        });
        let lights = local_lights_for_mesh([0, 0, 0], &mesh);
        assert_eq!(lights.len(), 1);
        let light = lights[0];
        assert!((light.position_radius[0] - 0.45).abs() < 0.0001);
        assert!((light.position_radius[1] - 0.55).abs() < 0.0001);
        assert!((light.position_radius[2] - 0.65).abs() < 0.0001);
        assert_eq!(light.position_radius[3], 3.2);
        assert!(light.color_intensity[3] > 2.4);
    }

    #[test]
    fn local_light_ranking_is_stable_and_hard_capped() {
        let mut ranked = [(f32::NEG_INFINITY, GpuLocalLight::default()); MAX_ACTIVE_LOCAL_LIGHTS];
        let mut count = 0;
        for ordinal in 0..20 {
            let light = GpuLocalLight {
                position_radius: [ordinal as f32, 0.0, 0.0, 3.0],
                color_intensity: [1.0, 1.0, 1.0, ordinal as f32],
            };
            rank_local_light(&mut ranked, &mut count, ordinal as f32, light);
        }
        assert_eq!(count, MAX_ACTIVE_LOCAL_LIGHTS);
        assert_eq!(ranked[0].0, 19.0);
        assert_eq!(ranked[MAX_ACTIVE_LOCAL_LIGHTS - 1].0, 4.0);
        assert!(ranked.windows(2).all(|pair| pair[0].0 >= pair[1].0));
    }

    #[test]
    fn refraction_bandwidth_is_paid_only_for_visible_water() {
        assert_eq!(refraction_copy_bytes(1_280, 720, false), 0);
        assert_eq!(refraction_copy_bytes(1_280, 720, true), 11_059_200);
    }

    #[test]
    fn water_samples_a_separate_non_attachment_depth_snapshot() {
        let world = world_depth_usage();
        assert!(world.contains(TextureUsages::RENDER_ATTACHMENT));
        assert!(world.contains(TextureUsages::COPY_SRC));

        let opaque = opaque_depth_usage();
        assert!(opaque.contains(TextureUsages::TEXTURE_BINDING));
        assert!(opaque.contains(TextureUsages::COPY_DST));
        assert!(!opaque.contains(TextureUsages::RENDER_ATTACHMENT));
    }

    #[test]
    fn frame_delta_rejects_invalid_time_and_caps_long_frames() {
        assert_eq!(bounded_frame_delta(f32::NAN), 0.0);
        assert_eq!(bounded_frame_delta(f32::INFINITY), 0.0);
        assert_eq!(bounded_frame_delta(-0.25), 0.0);
        assert_eq!(bounded_frame_delta(0.0), 0.0);
        assert_eq!(bounded_frame_delta(0.025), 0.025);
        assert_eq!(bounded_frame_delta(0.25), 0.1);
    }

    #[test]
    fn directional_shadows_remain_until_a_fully_enclosed_key_light_ray_is_blocked() {
        assert_eq!(interior_direct_light_visibility(0.0, false), 1.0);
        assert!(interior_direct_light_visibility(0.95, false) > 0.1);
        assert!(
            interior_direct_light_visibility(8.0 / 9.0, true) > 0.19,
            "one known sky opening must retain directional lighting and its shadow map"
        );
        assert_eq!(interior_direct_light_visibility(0.98, true), 0.0);
        assert_eq!(interior_direct_light_visibility(1.0, true), 0.0);
    }

    #[test]
    fn identical_resizes_skip_gpu_resource_recreation() {
        assert_eq!(
            resize_changes(1_280, 720, 2.0, 1_280, 720, 2.0),
            (false, false)
        );
        assert_eq!(
            resize_changes(1_280, 720, 1.0, 1_280, 720, 0.0),
            (false, false)
        );
        assert_eq!(
            resize_changes(1_280, 720, 1.0, 1_280, 720, 2.0),
            (false, true)
        );
        assert_eq!(
            resize_changes(1_280, 720, 1.0, 1_281, 720, 1.0),
            (true, false)
        );
    }

    #[test]
    fn gpu_timestamp_breakdown_uses_only_active_passes() {
        let timestamps = [
            1_000_000, 2_000_000, 2_100_000, 3_100_000, 3_200_000, 4_200_000, 4_500_000, 6_500_000,
            6_700_000, 7_700_000, 7_900_000, 8_400_000, 8_600_000, 10_600_000, 10_800_000,
            12_800_000, 13_000_000, 13_400_000, 13_600_000, 16_600_000, 16_800_000, 17_100_000,
            17_300_000, 18_300_000, 18_500_000, 18_900_000, 18_400_000, 19_800_000,
        ];
        let active = GpuPassMask {
            shadows: true,
            water: true,
            ambient_occlusion: true,
            clouds: true,
            weather: true,
            virtual_terrain: true,
        };
        let timing = parse_gpu_timestamps(&timestamps, 1.0, active)
            .unwrap_or_else(|| panic!("valid timestamps should parse"));
        assert!((timing.total_ms - 18.8).abs() < f32::EPSILON);
        assert!((timing.shadow_ms - 3.0).abs() < f32::EPSILON);
        assert!((timing.depth_prepass_ms - 2.0).abs() < f32::EPSILON);
        assert!((timing.ambient_occlusion_ms - 1.5).abs() < f32::EPSILON);
        assert!((timing.world_ms - 2.0).abs() < f32::EPSILON);
        assert!((timing.water_ms - 3.0).abs() < f32::EPSILON);
        assert!((timing.cloud_ms - 2.4).abs() < f32::EPSILON);
        assert!((timing.weather_ms - 0.3).abs() < f32::EPSILON);
        assert!((timing.ui_ms - 1.0).abs() < f32::EPSILON);
        assert!((timing.virtual_terrain_snapshot_encode_ms - 0.4).abs() < f32::EPSILON);
        assert!((timing.virtual_terrain_snapshot_validation_ms - 1.4).abs() < f32::EPSILON);
        assert!(
            timing.virtual_terrain_snapshot_validation_ms
                > timing.virtual_terrain_snapshot_encode_ms,
            "validation spans descriptor structure through post-encode exact comparison",
        );

        let mut skipped = timestamps;
        skipped[0..6].copy_from_slice(&[90, 80, 70, 60, 50, 40]);
        skipped[6..12].copy_from_slice(&[39, 38, 37, 36, 35, 34]);
        skipped[12..14].copy_from_slice(&[33, 32]);
        skipped[16..22].copy_from_slice(&[31, 30, 29, 28, 27, 26]);
        skipped[24..28].copy_from_slice(&[25, 24, 23, 22]);
        let timing = parse_gpu_timestamps(&skipped, 1.0, GpuPassMask::default())
            .unwrap_or_else(|| panic!("inactive timestamp pairs should be ignored"));
        assert_eq!(timing.shadow_ms, 0.0);
        assert_eq!(timing.depth_prepass_ms, 0.0);
        assert_eq!(timing.ambient_occlusion_ms, 0.0);
        assert_eq!(timing.water_ms, 0.0);
        assert_eq!(timing.cloud_ms, 0.0);
        assert_eq!(timing.weather_ms, 0.0);
        assert_eq!(timing.virtual_terrain_snapshot_encode_ms, 0.0);
        assert_eq!(timing.virtual_terrain_snapshot_validation_ms, 0.0);
    }

    #[test]
    fn gpu_timestamp_parser_rejects_invalid_or_implausible_samples() {
        let mut timestamps = [0u64; GPU_QUERY_COUNT as usize];
        for (index, timestamp) in timestamps.iter_mut().enumerate() {
            *timestamp = index as u64 * 1_000_000;
        }
        assert!(parse_gpu_timestamps(&timestamps, 0.0, GpuPassMask::default()).is_none());
        timestamps[15] = timestamps[14] - 1;
        assert!(parse_gpu_timestamps(&timestamps, 1.0, GpuPassMask::default()).is_none());
        timestamps[15] = timestamps[14] + 1;
        timestamps[23] = timestamps[14] + 1_100_000_000;
        assert!(parse_gpu_timestamps(&timestamps, 1.0, GpuPassMask::default()).is_none());
        assert_eq!(GPU_QUERY_BUFFER_BYTES, 224);
        assert_eq!(GPU_RESOLVE_BUFFER_BYTES % 256, 0);
    }

    #[test]
    fn virtual_gpu_cut_requires_exact_submission_certificate() {
        let envelope = crate::virtual_terrain::PresentationEnvelopeCache::default().resolve(
            [0.0, 0.0, 0.0],
            0.0,
            1.0,
            64,
            16,
        );
        let key = TerrainPageKey {
            level: 2,
            coord: [-3, 1, 4],
        };
        let cut = VirtualTerrainCut {
            selected_pages: vec![key],
            requested_pages: Vec::new(),
            refinement_roots: Vec::new(),
            ownerless_roots: Vec::new(),
            fingerprint: 0x1234_5678_9abc_def0,
            visited_nodes: 1,
            selected_primitives: 4,
            selected_encoded_bytes: 64,
            feedback_overflow: false,
            selection_overflow: false,
            traversal_overflow: false,
            incoherent_replacement_groups: 0,
            exact_surface_lod_discontinuities: 0,
        };
        let certified = GpuVirtualTerrainFeedback {
            submission_id: 9,
            oracle_fingerprint: virtual_terrain_presentation_fingerprint(&cut, &envelope),
            selected_pages: vec![key],
            encoded_pages: 1,
            ..GpuVirtualTerrainFeedback::default()
        };
        assert!(gpu_feedback_matches_cut(
            &certified,
            Some(&cut),
            Some(&envelope)
        ));

        let mut ownership_overflow = certified.clone();
        ownership_overflow.encoding_overflow_flags = 1;
        assert!(!gpu_feedback_matches_cut(
            &ownership_overflow,
            Some(&cut),
            Some(&envelope)
        ));

        let mut stale = certified.clone();
        stale.oracle_fingerprint ^= 1;
        assert!(!gpu_feedback_matches_cut(
            &stale,
            Some(&cut),
            Some(&envelope)
        ));

        let mut incomplete = certified;
        incomplete.encoded_pages = 0;
        assert!(!gpu_feedback_matches_cut(
            &incomplete,
            Some(&cut),
            Some(&envelope)
        ));
    }

    #[test]
    fn frustum_rejects_chunks_behind_camera_and_beyond_far_plane() {
        let camera = CameraState::spawn(glam::Vec3::new(0.0, 1.7, 0.0));
        let matrix = test_view_projection(&camera);
        let edge = CHUNK_EDGE as f32 * VOXEL_SIZE_METRES;
        let bounds = |coord: ChunkCoord| {
            let min = glam::Vec3::from_array(
                coord
                    .world_origin()
                    .map(|value| value as f32 * VOXEL_SIZE_METRES),
            );
            (min, min + glam::Vec3::splat(edge))
        };
        let (front_min, front_max) = bounds(ChunkCoord::new(0, 0, -1));
        let (back_min, back_max) = bounds(ChunkCoord::new(0, 0, 2));
        let (far_min, far_max) = bounds(ChunkCoord::new(0, 0, -120));
        let clip = AabbClipVolume::new(matrix);
        assert!(clip.contains_aabb(front_min, front_max));
        assert!(!clip.contains_aabb(back_min, back_max));
        assert!(!clip.contains_aabb(far_min, far_max));
    }

    #[test]
    fn virtual_surface_quads_preserve_the_global_integer_lattice_on_every_face() {
        let key = TerrainPageKey {
            level: 0,
            coord: [0, 0, 0],
        };
        let source_quads = [
            (FaceAxis::X, true, 10, 20, 24, 2, 3),
            (FaceAxis::X, false, 11, 20, 24, 2, 3),
            (FaceAxis::Y, true, 12, 20, 24, 2, 3),
            (FaceAxis::Y, false, 13, 20, 24, 2, 3),
            (FaceAxis::Z, true, 14, 20, 24, 2, 3),
            (FaceAxis::Z, false, 15, 20, 24, 2, 3),
        ]
        .map(|(axis, positive, plane, u, v, width, height)| {
            voxels_world::TerrainSurfaceQuad {
                axis,
                plane,
                u,
                v,
                width,
                height,
                positive,
                material_index: 0,
            }
        });
        let page = TerrainPageV1 {
            source_identity_hash: voxels_world::WorldSourceIdentityHash::from_bytes([7; 32]),
            key,
            revision: 1,
            bounds: key.bounds().expect("valid test bounds"),
            children: Vec::new(),
            errors: voxels_world::TerrainErrorBounds::EXACT,
            topology: voxels_world::TerrainTopologyClass::Volumetric,
            boundary_fingerprints: [[0; 32]; 6],
            materials: vec![voxels_world::TerrainMaterialCoverage {
                material: Material::Stone,
                occupied_voxels: 1,
                exposed_unit_faces: 1,
            }],
            representation: TerrainPageRepresentation::SurfaceCluster(source_quads.to_vec()),
            content_fingerprint: [0; 32],
        };
        let gpu = virtual_surface_gpu_quads(&page).expect("surface conversion");
        assert_eq!(gpu.len(), source_quads.len());
        for (source, converted) in source_quads.iter().zip(&gpu) {
            let expected_face = source.axis as u8 * 2 + u8::from(!source.positive);
            assert_eq!(
                (converted.material_face & GPU_FACE_MASK) >> GPU_FACE_SHIFT,
                u32::from(expected_face)
            );
            let mut actual = canonical_quad_corners(*converted);
            actual.sort_unstable();
            let mut expected = match source.axis {
                FaceAxis::X => [
                    [source.plane, source.u, source.v],
                    [source.plane, source.u + i32::from(source.width), source.v],
                    [
                        source.plane,
                        source.u + i32::from(source.width),
                        source.v + i32::from(source.height),
                    ],
                    [source.plane, source.u, source.v + i32::from(source.height)],
                ],
                FaceAxis::Y => [
                    [source.u, source.plane, source.v],
                    [source.u + i32::from(source.width), source.plane, source.v],
                    [
                        source.u + i32::from(source.width),
                        source.plane,
                        source.v + i32::from(source.height),
                    ],
                    [source.u, source.plane, source.v + i32::from(source.height)],
                ],
                FaceAxis::Z => [
                    [source.u, source.v, source.plane],
                    [source.u + i32::from(source.width), source.v, source.plane],
                    [
                        source.u + i32::from(source.width),
                        source.v + i32::from(source.height),
                        source.plane,
                    ],
                    [source.u, source.v + i32::from(source.height), source.plane],
                ],
            };
            expected.sort_unstable();
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn virtual_surface_quads_accept_every_exact_compact_encoding() {
        let source = voxels_world::WorldSourceIdentityHash::from_bytes([19; 32]);
        let stepped = voxels_world::build_compact_exact_terrain_page(
            source,
            TerrainPageKey {
                level: 0,
                coord: [-2, -1, 1],
            },
            1,
            |coord| {
                let height = -8 + (coord.x + coord.z).rem_euclid(2);
                if coord.y <= height {
                    Material::Stone
                } else {
                    Material::Air
                }
            },
        )
        .unwrap();
        assert!(matches!(
            stepped.representation,
            TerrainPageRepresentation::SteppedSurfaceResidual(_)
        ));
        let stepped_gpu = virtual_surface_gpu_quads(&stepped).unwrap();
        assert!(!stepped_gpu.is_empty());
        assert!(stepped_gpu.iter().all(|quad| {
            quad.extent_voxels.into_iter().all(|extent| extent > 0)
                && quad.material_face & 0xffff == u32::from(Material::Stone.id())
                && quad.material_face & (1 << 31) != 0
        }));

        let sparse = voxels_world::build_compact_exact_terrain_page(
            source,
            TerrainPageKey {
                level: 0,
                coord: [-1, 0, -2],
            },
            2,
            |coord| {
                let hash = coord.x.wrapping_mul(73_856_093)
                    ^ coord.y.wrapping_mul(19_349_663)
                    ^ coord.z.wrapping_mul(83_492_791);
                if hash.rem_euclid(97) == 0 {
                    Material::Basalt
                } else {
                    Material::Air
                }
            },
        )
        .unwrap();
        assert!(matches!(
            sparse.representation,
            TerrainPageRepresentation::SparseVoxelBrick(_)
        ));
        let sparse_gpu = virtual_surface_gpu_quads(&sparse).unwrap();
        assert!(!sparse_gpu.is_empty());
        assert!(sparse_gpu.iter().all(|quad| {
            quad.extent_voxels.into_iter().all(|extent| extent > 0)
                && quad.material_face & 0xffff == u32::from(Material::Basalt.id())
                && quad.material_face & (1 << 31) != 0
        }));
    }

    #[test]
    fn sampled_heightfield_merges_flat_water_without_changing_its_lattice_plane() {
        let key = TerrainPageKey {
            level: 2,
            coord: [-1, 0, -1],
        };
        let samples = vec![
            voxels_world::SurfaceSample {
                height: 10,
                material: Material::Grass,
                water_level: Some(15),
                region: SurfaceRegion::VerdantForest,
                moisture: 0.5,
                temperature: 0.5,
                ridge: 0.0,
                route: None,
            };
            (TERRAIN_PAGE_EDGE_SAMPLES as usize + 1).pow(2)
        ];
        let page = voxels_world::build_sampled_heightfield_terrain_page(
            voxels_world::WorldSourceIdentityHash::from_bytes([29; 32]),
            key,
            3,
            &samples,
            &test_heightfield_boundary_midpoints(key, samples[0]),
            voxels_world::TerrainErrorBounds::EXACT,
        )
        .unwrap();
        let vertices = virtual_triangle_gpu_vertices(&page, None).unwrap();
        let ground_vertices =
            TERRAIN_PAGE_EDGE_SAMPLES as usize * TERRAIN_PAGE_EDGE_SAMPLES as usize * 6;
        assert_eq!(vertices.len(), ground_vertices + 6);
        assert!(vertices.iter().all(|vertex| vertex.normal[1] > 0));
        let (_, opaque, water) = partition_virtual_triangle_geometry(vertices).unwrap();
        assert_eq!(opaque as usize, ground_vertices);
        assert_eq!(water, 6);
    }

    #[test]
    fn exact_surface_heightfield_emits_only_axis_aligned_voxel_faces() {
        let key = TerrainPageKey::surface(0, 3, -2);
        let edge = TERRAIN_PAGE_EDGE_SAMPLES as usize + 1;
        let samples = (0..edge * edge)
            .map(|index| {
                let x = index % edge;
                let z = index / edge;
                voxels_world::SurfaceSample {
                    height: i32::try_from((x / 3 + z / 5) % 7).unwrap(),
                    material: if (x / 2 + z / 4).is_multiple_of(2) {
                        Material::Grass
                    } else {
                        Material::Dirt
                    },
                    water_level: None,
                    region: SurfaceRegion::VerdantForest,
                    moisture: 0.5,
                    temperature: 0.5,
                    ridge: 0.0,
                    route: None,
                }
            })
            .collect::<Vec<_>>();
        let page = voxels_world::build_sampled_heightfield_terrain_page(
            voxels_world::WorldSourceIdentityHash::from_bytes([37; 32]),
            key,
            4,
            &samples,
            &[],
            voxels_world::TerrainErrorBounds::EXACT,
        )
        .unwrap();

        let vertices = virtual_triangle_gpu_vertices(&page, None).unwrap();

        assert!(!vertices.is_empty());
        assert!(vertices.iter().all(|vertex| {
            vertex
                .position
                .iter()
                .all(|coordinate| coordinate.fract() == 0.0)
                && vertex
                    .normal
                    .iter()
                    .take(3)
                    .filter(|component| **component != 0)
                    .count()
                    == 1
        }));
        assert!(vertices.chunks_exact(3).all(|triangle| {
            (0..3).any(|axis| {
                triangle[0].position[axis] == triangle[1].position[axis]
                    && triangle[1].position[axis] == triangle[2].position[axis]
            })
        }));
    }

    #[test]
    fn exact_heightfield_uses_compact_integer_anchored_conforming_instances() {
        let key = TerrainPageKey::surface(0, -3, 2);
        let edge = TERRAIN_PAGE_EDGE_SAMPLES as usize + 1;
        let samples = vec![
            voxels_world::SurfaceSample {
                height: 7,
                material: Material::Grass,
                water_level: None,
                region: SurfaceRegion::VerdantForest,
                moisture: 0.5,
                temperature: 0.5,
                ridge: 0.0,
                route: None,
            };
            edge * edge
        ];
        let page = voxels_world::build_sampled_heightfield_terrain_page(
            voxels_world::WorldSourceIdentityHash::from_bytes([41; 32]),
            key,
            5,
            &samples,
            &[],
            voxels_world::TerrainErrorBounds::EXACT,
        )
        .unwrap();
        let TerrainPageRepresentation::HeightfieldGrid(grid) = &page.representation else {
            panic!("exact sampled surface must remain a heightfield");
        };
        let heightfield = unconstrained_virtual_heightfield_samples(grid);
        let quads = virtual_microvoxel_gpu_quads(&page, grid, &heightfield)
            .unwrap()
            .expect("flat exact water-free heightfield is compactable");
        let vertices = virtual_triangle_gpu_vertices(&page, Some(&heightfield)).unwrap();

        assert_eq!(
            quads.len(),
            TERRAIN_PAGE_EDGE_SAMPLES as usize * 4,
            "the flat page stores one instance per unit perimeter segment"
        );
        assert!(
            quads
                .iter()
                .all(|quad| quad.extent_voxels[0] & CANONICAL_TRIANGLE_FLAG != 0)
        );
        assert!(quads.iter().all(|quad| {
            (quad.extent_voxels[1] >> CANONICAL_TRIANGLE_ANCHOR_SHIFT) & 7
                == CANONICAL_TRIANGLE_LATTICE_ANCHOR
                && (quad.ao >> CANONICAL_TRIANGLE_ANCHOR_U_SHIFT) & 63 == 1
                && (quad.ao >> CANONICAL_TRIANGLE_ANCHOR_V_SHIFT) & 63 == 1
        }));
        assert_eq!(
            vertices.len(),
            quads.len() * 3,
            "compact instances retain the same conforming triangle count without triplicated vertices"
        );

        let mut parent_constrained = heightfield;
        parent_constrained.ground[0] = 7.5;
        assert!(
            virtual_microvoxel_gpu_quads(&page, grid, &parent_constrained)
                .unwrap()
                .is_some(),
            "the compact exact path must apply the same 10 cm rounding as the triangle fallback"
        );
    }

    #[test]
    fn level_one_boundary_cell_exposes_ten_centimetre_shared_vertices() {
        let key = TerrainPageKey::surface(1, 1, -1);
        let edge = TERRAIN_PAGE_EDGE_SAMPLES as usize + 1;
        let samples = (0..edge * edge)
            .map(|index| {
                let x = index % edge;
                let z = index / edge;
                voxels_world::SurfaceSample {
                    height: i32::try_from(x * 2 + z).unwrap(),
                    material: Material::Grass,
                    water_level: None,
                    region: SurfaceRegion::VerdantForest,
                    moisture: 0.5,
                    temperature: 0.5,
                    ridge: 0.0,
                    route: None,
                }
            })
            .collect::<Vec<_>>();
        let page = voxels_world::build_sampled_heightfield_terrain_page(
            voxels_world::WorldSourceIdentityHash::from_bytes([38; 32]),
            key,
            4,
            &samples,
            &test_heightfield_boundary_midpoints(key, samples[0]),
            voxels_world::TerrainErrorBounds::EXACT,
        )
        .unwrap();

        let mut vertices = Vec::new();
        push_virtual_heightfield_boundary_cell(
            &mut vertices,
            [
                [page.bounds.min.x as f32, 0.0, page.bounds.min.z as f32],
                [
                    (page.bounds.min.x + 2) as f32,
                    2.0,
                    page.bounds.min.z as f32,
                ],
                [
                    (page.bounds.min.x + 2) as f32,
                    3.0,
                    (page.bounds.min.z + 2) as f32,
                ],
                [
                    page.bounds.min.x as f32,
                    1.0,
                    (page.bounds.min.z + 2) as f32,
                ],
            ],
            [true, false, false, false],
            true,
            Material::Grass,
            key,
        )
        .unwrap();

        assert!(!vertices.is_empty());
        assert!(
            vertices
                .iter()
                .filter(|vertex| {
                    vertex.position[0] == page.bounds.min.x as f32
                        || vertex.position[0] == page.bounds.max.x as f32
                        || vertex.position[2] == page.bounds.min.z as f32
                        || vertex.position[2] == page.bounds.max.z as f32
                })
                .all(|vertex| vertex
                    .position
                    .iter()
                    .all(|coordinate| coordinate.fract() == 0.0))
        );
        assert!(
            vertices
                .iter()
                .any(|vertex| (vertex.position[0] as i32 - page.bounds.min.x).rem_euclid(2) == 1),
            "L1 presentation must materialize the 10 cm points between transmitted samples"
        );
    }

    #[test]
    fn reverse_boundary_sides_keep_the_exact_lower_coordinate_height_owner() {
        let key = TerrainPageKey::surface(1, 0, 0);
        let mut vertices = Vec::new();
        push_virtual_heightfield_boundary_cell(
            &mut vertices,
            [
                [0.0, 0.0, 0.0],
                [2.0, 2.0, 0.0],
                [2.0, 4.0, 2.0],
                [0.0, 0.0, 2.0],
            ],
            [false, false, true, false],
            true,
            Material::Grass,
            key,
        )
        .unwrap();

        let has_boundary_edge = |left_x: f32, right_x: f32, height: f32| {
            vertices.chunks_exact(3).any(|triangle| {
                let boundary = triangle
                    .iter()
                    .filter(|vertex| vertex.position[2] == 2.0)
                    .map(|vertex| vertex.position)
                    .collect::<Vec<_>>();
                boundary
                    .iter()
                    .any(|vertex| vertex[0] == left_x && vertex[1] == height)
                    && boundary
                        .iter()
                        .any(|vertex| vertex[0] == right_x && vertex[1] == height)
            })
        };
        assert!(has_boundary_edge(2.0, 1.0, 2.0));
        assert!(has_boundary_edge(1.0, 0.0, 0.0));
        assert!(
            !has_boundary_edge(2.0, 1.0, 4.0),
            "positive-Z traversal must not shift the L0 staircase by one voxel"
        );
    }

    #[test]
    fn child_heightfield_outer_vertices_use_the_recursive_parent_edge_equation() {
        let source = voxels_world::WorldSourceIdentityHash::from_bytes([31; 32]);
        let parent_key = TerrainPageKey::surface(1, 0, 0);
        let edge = TERRAIN_PAGE_EDGE_SAMPLES as usize + 1;
        let parent_samples = (0..edge * edge)
            .map(|index| voxels_world::SurfaceSample {
                height: if index % edge == 0 { 0 } else { 1 },
                material: Material::Grass,
                water_level: None,
                region: SurfaceRegion::VerdantForest,
                moisture: 0.5,
                temperature: 0.5,
                ridge: 0.0,
                route: None,
            })
            .collect::<Vec<_>>();
        let parent = voxels_world::build_sampled_heightfield_terrain_page(
            source,
            parent_key,
            1,
            &parent_samples,
            &test_heightfield_boundary_midpoints(parent_key, parent_samples[0]),
            voxels_world::TerrainErrorBounds::EXACT,
        )
        .unwrap();
        let children = parent_key
            .refinement_children()
            .unwrap()
            .into_iter()
            .map(|key| {
                voxels_world::build_sampled_heightfield_terrain_page(
                    source,
                    key,
                    1,
                    &vec![
                        voxels_world::SurfaceSample {
                            height: 9,
                            material: Material::Grass,
                            water_level: None,
                            region: SurfaceRegion::VerdantForest,
                            moisture: 0.5,
                            temperature: 0.5,
                            ridge: 0.0,
                            route: None,
                        };
                        edge * edge
                    ],
                    &[],
                    voxels_world::TerrainErrorBounds::EXACT,
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        let pages = children
            .iter()
            .cloned()
            .chain([parent.clone()])
            .collect::<Vec<_>>();
        let directory = voxels_world::TerrainHierarchyDirectoryV1::from_surface_refinement_pages(
            parent_key, &pages,
        )
        .unwrap();
        let mut hierarchy =
            VirtualTerrainHierarchy::new(VirtualTerrainCapacity::DEVELOPMENT_128_MIB).unwrap();
        hierarchy.register_region_directory(&directory).unwrap();
        hierarchy.install_page(parent).unwrap();

        let constrained =
            constrained_virtual_heightfield_samples(&hierarchy, &children[0]).unwrap();
        assert_eq!(constrained.ground[1], 1.5);
        assert_eq!(constrained.ground[1 + edge], 10.0);
    }

    #[test]
    fn level_zero_heightfield_restores_only_the_boundary_owned_with_an_exact_neighbor() {
        let source = voxels_world::WorldSourceIdentityHash::from_bytes([32; 32]);
        let parent_key = TerrainPageKey::surface(1, 0, 0);
        let edge = TERRAIN_PAGE_EDGE_SAMPLES as usize + 1;
        let sample = |height| voxels_world::SurfaceSample {
            height,
            material: Material::Grass,
            water_level: None,
            region: SurfaceRegion::VerdantForest,
            moisture: 0.5,
            temperature: 0.5,
            ridge: 0.0,
            route: None,
        };
        let parent = voxels_world::build_sampled_heightfield_terrain_page(
            source,
            parent_key,
            1,
            &vec![sample(2); edge * edge],
            &test_heightfield_boundary_midpoints(parent_key, sample(9)),
            voxels_world::TerrainErrorBounds::EXACT,
        )
        .unwrap();
        let child_keys = parent_key.refinement_children().unwrap();
        let heightfield = voxels_world::build_sampled_heightfield_terrain_page(
            source,
            child_keys[0],
            1,
            &vec![sample(9); edge * edge],
            &[],
            voxels_world::TerrainErrorBounds::EXACT,
        )
        .unwrap();
        let exact = voxels_world::build_exact_surface_terrain_page(
            source,
            child_keys[1],
            1,
            [-4, 12],
            |coord| {
                if coord.y <= 9 {
                    Material::Grass
                } else {
                    Material::Air
                }
            },
        )
        .unwrap();
        let other_children = child_keys[2..]
            .iter()
            .map(|key| {
                voxels_world::build_sampled_heightfield_terrain_page(
                    source,
                    *key,
                    1,
                    &vec![sample(9); edge * edge],
                    &[],
                    voxels_world::TerrainErrorBounds::EXACT,
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        let pages = [parent.clone(), heightfield.clone(), exact]
            .into_iter()
            .chain(other_children)
            .collect::<Vec<_>>();
        let directory = voxels_world::TerrainHierarchyDirectoryV1::from_surface_refinement_pages(
            parent_key, &pages,
        )
        .unwrap();
        let mut hierarchy =
            VirtualTerrainHierarchy::new(VirtualTerrainCapacity::DEVELOPMENT_128_MIB).unwrap();
        hierarchy.register_region_directory(&directory).unwrap();
        hierarchy.install_page(parent).unwrap();

        let constrained =
            constrained_virtual_heightfield_samples(&hierarchy, &heightfield).unwrap();
        assert_eq!(constrained.exact_neighbor_sides, [false, true, false, true]);
        assert_eq!(constrained.ground[edge - 1], 3.0);
        for z in 1..edge {
            assert_eq!(constrained.ground[(edge - 1) + z * edge], 10.0);
        }
        assert_eq!(constrained.ground[(edge - 1) * edge], 3.0);
        for x in 1..edge {
            assert_eq!(constrained.ground[x + (edge - 1) * edge], 10.0);
        }
        assert_eq!(constrained.ground[0], 3.0);

        let TerrainPageRepresentation::HeightfieldGrid(grid) = &heightfield.representation else {
            panic!("sampled child must be a heightfield");
        };
        let parent_edge = VirtualHeightfieldSamples {
            ground: vec![3.0; edge * edge],
            water: vec![None; edge * edge],
            exact_neighbor_sides: [false; 4],
            finer_neighbor_sides: [false; 4],
        };
        let mixed_corner = restore_exact_neighbor_heightfield_boundaries(
            &heightfield,
            grid,
            &parent_edge,
            [false, true, false, false],
        );
        assert_eq!(mixed_corner.ground[(edge - 1) + edge], 10.0);
        assert_eq!(
            mixed_corner.ground[edge - 1],
            3.0,
            "an exact X side must not overwrite the corner owned by a coarse Z transition"
        );
        assert_eq!(mixed_corner.ground[(edge - 1) + (edge - 1) * edge], 3.0);
    }

    #[test]
    fn virtual_geometry_partition_keeps_water_in_the_same_page_but_a_distinct_stream() {
        let quad = |material: Material| GpuQuad {
            origin: [0; 3],
            extent_voxels: [1; 2],
            material_face: pack_gpu_material_face(u32::from(material.id()), 2),
            ao: u32::from(u8::MAX),
        };
        let (quads, opaque_quads, water_quads) = partition_virtual_surface_geometry(vec![
            quad(Material::Water),
            quad(Material::Stone),
            quad(Material::Water),
            quad(Material::Grass),
        ])
        .unwrap();
        assert_eq!((opaque_quads, water_quads), (2, 2));
        assert!(quads[..opaque_quads as usize].iter().all(|quad| {
            quad.material_face & !GPU_FACE_MASK != u32::from(Material::Water.id())
        }));
        assert!(quads[opaque_quads as usize..].iter().all(|quad| {
            quad.material_face & !GPU_FACE_MASK == u32::from(Material::Water.id())
        }));

        let vertex = |material: Material| GpuTerrainVertex {
            position: [0.0; 3],
            material: u32::from(material.id()),
            normal: [0, i16::MAX, 0, 0],
        };
        let (vertices, opaque_vertices, water_vertices) =
            partition_virtual_triangle_geometry(vec![
                vertex(Material::Water),
                vertex(Material::Water),
                vertex(Material::Water),
                vertex(Material::Basalt),
                vertex(Material::Basalt),
                vertex(Material::Basalt),
            ])
            .unwrap();
        assert_eq!((opaque_vertices, water_vertices), (3, 3));
        assert!(
            vertices[..3]
                .iter()
                .all(|vertex| { vertex.material == u32::from(Material::Basalt.id()) })
        );
        assert!(
            vertices[3..]
                .iter()
                .all(|vertex| { vertex.material == u32::from(Material::Water.id()) })
        );
    }

    #[test]
    fn virtual_triangle_vertices_preserve_positions_materials_and_flat_winding() {
        let key = TerrainPageKey {
            level: 0,
            coord: [0, 0, 0],
        };
        let page = TerrainPageV1 {
            source_identity_hash: voxels_world::WorldSourceIdentityHash::from_bytes([9; 32]),
            key,
            revision: 1,
            bounds: key.bounds().expect("valid test bounds"),
            children: Vec::new(),
            errors: voxels_world::TerrainErrorBounds {
                geometric_millivoxels: 1,
                ..voxels_world::TerrainErrorBounds::EXACT
            },
            topology: voxels_world::TerrainTopologyClass::Volumetric,
            boundary_fingerprints: [[0; 32]; 6],
            materials: vec![voxels_world::TerrainMaterialCoverage {
                material: Material::Basalt,
                occupied_voxels: 1,
                exposed_unit_faces: 1,
            }],
            representation: TerrainPageRepresentation::TriangleCluster(
                voxels_world::TerrainTriangleCluster {
                    vertices: vec![
                        voxels_world::TerrainClusterVertex {
                            position: [0, 0, 0],
                            material_index: 0,
                        },
                        voxels_world::TerrainClusterVertex {
                            position: [2, 0, 0],
                            material_index: 0,
                        },
                        voxels_world::TerrainClusterVertex {
                            position: [0, 3, 0],
                            material_index: 0,
                        },
                    ],
                    triangles: vec![voxels_world::TerrainClusterTriangle {
                        vertices: [0, 1, 2],
                        material_index: 0,
                    }],
                },
            ),
            content_fingerprint: [0; 32],
        };
        let vertices = virtual_triangle_gpu_vertices(&page, None).expect("triangle conversion");
        assert_eq!(
            vertices
                .iter()
                .map(|vertex| vertex.position)
                .collect::<Vec<_>>(),
            [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 3.0, 0.0]]
        );
        assert!(vertices.iter().all(|vertex| vertex.material & 0xffff
            == u32::from(Material::Basalt.id())
            && vertex.material & (1 << 31) != 0));
        assert!(
            vertices
                .iter()
                .all(|vertex| vertex.normal == [0, 0, i16::MAX, 0])
        );
    }

    fn replacement_transaction_fixture()
    -> (VirtualTerrainHierarchy, TerrainPageV1, Vec<TerrainPageV1>) {
        let source = voxels_world::WorldSourceIdentityHash::from_bytes([0x57; 32]);
        let parent_key = TerrainPageKey::surface(1, -2, 3);
        let edge = TERRAIN_PAGE_EDGE_SAMPLES as usize + 1;
        let sample = voxels_world::SurfaceSample {
            height: 4,
            material: Material::Stone,
            water_level: None,
            region: SurfaceRegion::VerdantForest,
            moisture: 0.5,
            temperature: 0.5,
            ridge: 0.0,
            route: None,
        };
        let children = parent_key
            .refinement_children()
            .unwrap()
            .into_iter()
            .map(|key| {
                voxels_world::build_sampled_heightfield_terrain_page(
                    source,
                    key,
                    1,
                    &vec![sample; edge * edge],
                    &[],
                    voxels_world::TerrainErrorBounds::EXACT,
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        let parent = voxels_world::build_sampled_heightfield_terrain_page(
            source,
            parent_key,
            1,
            &vec![sample; edge * edge],
            &test_heightfield_boundary_midpoints(parent_key, sample),
            voxels_world::TerrainErrorBounds::EXACT,
        )
        .unwrap();
        let pages = std::iter::once(parent.clone())
            .chain(children.iter().cloned())
            .collect::<Vec<_>>();
        let directory = voxels_world::TerrainHierarchyDirectoryV1::from_surface_refinement_pages(
            parent_key, &pages,
        )
        .unwrap();
        let mut hierarchy =
            VirtualTerrainHierarchy::new(VirtualTerrainCapacity::DEVELOPMENT_128_MIB).unwrap();
        hierarchy.register_region_directory(&directory).unwrap();
        hierarchy.install_page(parent.clone()).unwrap();
        (hierarchy, parent, children)
    }

    #[test]
    fn replacement_install_failure_rolls_back_new_siblings_but_keeps_identical_residents() {
        let (mut hierarchy, parent, children) = replacement_transaction_fixture();
        hierarchy.install_page(children[0].clone()).unwrap();
        let error = install_virtual_terrain_replacement_pages(
            &mut hierarchy,
            parent.key,
            &children,
            |index| {
                if index == 2 {
                    Err(VirtualTerrainError::ResidentPrimitiveCapacity)
                } else {
                    Ok(())
                }
            },
        )
        .unwrap_err();
        assert_eq!(error, VirtualTerrainError::ResidentPrimitiveCapacity);
        assert_eq!(hierarchy.resident_page(parent.key), Some(&parent));
        assert_eq!(
            hierarchy.resident_page(children[0].key),
            Some(&children[0]),
            "an identical child present before the transaction must survive rollback"
        );
        for child in &children[1..] {
            assert_eq!(hierarchy.resident_page(child.key), None);
        }
        assert!(!hierarchy.replacement_is_resident_and_coherent(parent.key));
    }

    #[test]
    fn replacement_rejects_a_stale_preexisting_child_before_installing_any_sibling() {
        let (mut hierarchy, parent, mut children) = replacement_transaction_fixture();
        let resident = children[0].clone();
        hierarchy.install_page(resident.clone()).unwrap();
        children[0].revision = children[0].revision.saturating_add(1);
        let mut attempted_install = false;
        let error = install_virtual_terrain_replacement_pages(
            &mut hierarchy,
            parent.key,
            &children,
            |_| {
                attempted_install = true;
                Ok(())
            },
        )
        .unwrap_err();
        assert_eq!(error, VirtualTerrainError::StalePage(resident.key));
        assert!(!attempted_install);
        assert_eq!(hierarchy.resident_page(resident.key), Some(&resident));
        for child in &children[1..] {
            assert_eq!(hierarchy.resident_page(child.key), None);
        }
    }

    #[test]
    fn candidate_only_seam_rebuild_failure_always_runs_oracle_recovery() {
        let mut recovered = false;
        let injected = recover_candidate_only_seam_rebuild::<(), _>(
            Err(VirtualTerrainRendererError::GpuPoolCapacity),
            || recovered = true,
        );
        assert_eq!(injected, Err(VirtualTerrainRendererError::GpuPoolCapacity));
        assert!(
            recovered,
            "an allocation failure after releasing candidate-only geometry must invalidate that candidate"
        );
    }

    fn virtual_cut_with_selected(selected_pages: Vec<TerrainPageKey>) -> VirtualTerrainCut {
        VirtualTerrainCut {
            selected_pages,
            requested_pages: Vec::new(),
            refinement_roots: Vec::new(),
            ownerless_roots: Vec::new(),
            fingerprint: 1,
            visited_nodes: 1,
            selected_primitives: 0,
            selected_encoded_bytes: 0,
            feedback_overflow: false,
            selection_overflow: false,
            traversal_overflow: false,
            incoherent_replacement_groups: 0,
            exact_surface_lod_discontinuities: 0,
        }
    }

    #[test]
    fn virtual_ownership_rejects_an_incomplete_root_partition() {
        let root = TerrainPageKey::surface(TERRAIN_COVERAGE_ROOT_LEVEL, -1, 2);
        let incomplete = root.refinement_children().unwrap()[..2].to_vec();
        assert_eq!(
            VirtualTerrainOwnership::from_cut(&virtual_cut_with_selected(incomplete)),
            Err(VirtualTerrainRendererError::IncompleteRootPartition(root))
        );
    }

    #[test]
    fn virtual_ownership_covers_only_complete_half_open_surface_columns() {
        let root = TerrainPageKey::surface(TERRAIN_COVERAGE_ROOT_LEVEL, -1, 2);
        let ownership = VirtualTerrainOwnership::from_cut(&virtual_cut_with_selected(
            root.refinement_children().unwrap(),
        ))
        .unwrap();
        assert!(ownership.covers_aabb(
            glam::Vec3::new(-100.0, -10_000.0, 6_600.0),
            glam::Vec3::new(-50.0, 10_000.0, 6_650.0),
        ));
        assert!(!ownership.covers_aabb(
            glam::Vec3::new(-1.0, -1.0, 6_600.0),
            glam::Vec3::new(1.0, 1.0, 6_650.0),
        ));
    }
}
