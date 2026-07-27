use crate::ambient_occlusion::AmbientOcclusionGpu;
use crate::arena::{Allocation, ArenaAllocator};
use crate::avatar::AvatarGpu;
pub use crate::clouds::VolumetricCloudConfig;
use crate::clouds::VolumetricCloudGpu;
use crate::environment::{
    DaylightPhase, DebugEnvironmentOverride, InteriorEnvironment, OutdoorEnvironment,
    WorldEnvironmentState, surface_region_label,
};
use crate::lod::{
    GeometricLodFocus, LOD_BOUNDARY_HALF_EXTENTS, LOD_BOUNDARY_SNAP, LodOwner,
    SurfacePatchSelection, SurfacePatchSelectionBuild, incomplete_resident_parents,
    lod_boundary_half_extents_are_valid,
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
    VirtualTerrainCapacity, VirtualTerrainCut, VirtualTerrainError, VirtualTerrainHierarchy,
    VirtualTerrainView,
};
use crate::virtual_terrain_gpu::{
    GpuVirtualTerrainFeedback, VIRTUAL_TERRAIN_COMPACT_SURFACE_BYTES,
    VIRTUAL_TERRAIN_COMPACT_TRIANGLE_BYTES, VIRTUAL_TERRAIN_COMPACT_WATER_SURFACE_BYTES,
    VIRTUAL_TERRAIN_COMPACT_WATER_TRIANGLE_BYTES, VIRTUAL_TERRAIN_SURFACE_INDIRECT_OFFSET,
    VIRTUAL_TERRAIN_TRIANGLE_INDIRECT_OFFSET, VIRTUAL_TERRAIN_WATER_SURFACE_INDIRECT_OFFSET,
    VIRTUAL_TERRAIN_WATER_TRIANGLE_INDIRECT_OFFSET, VirtualTerrainGpuControl,
    VirtualTerrainGpuGeometry, VirtualTerrainGpuGeometryRange, VirtualTerrainGpuTimestampWrites,
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
    MeshedChunk, Quad, RenderLayer, SURFACE_PATCHES_PER_TILE_EDGE, SurfaceLodLevel, SurfacePatch,
    SurfacePatchEdge, SurfacePatchId, SurfaceQuad, SurfaceRegion, SurfaceTileCoord,
    SurfaceTileMesh, TERRAIN_REGION_ROOT_LEVEL, TerrainHierarchyDirectoryV1, TerrainPageKey,
    TerrainPageRepresentation, TerrainPageRepresentationKind, TerrainPageV1, VOXEL_SIZE_METRES,
    WaterTileMesh, WorldManifest, fallback_surface_wall_material,
    reconstruct_exact_terrain_surface,
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
const VIRTUAL_TERRAIN_GPU_POOL_BYTES: u64 = 128 * 1024 * 1024;
const VIRTUAL_TERRAIN_GPU_POOL_PAGES: usize = 1;
const VIRTUAL_TERRAIN_GPU_ARENA_PAGE_BYTES: u32 = VIRTUAL_TERRAIN_GPU_POOL_BYTES as u32;
const FAR_MATERIAL_FLAG: u32 = 1 << 31;
const SURFACE_LOD_SHIFT: u32 = 27;
const GPU_FACE_SHIFT: u32 = 16;
const GPU_FACE_MASK: u32 = 0b111 << GPU_FACE_SHIFT;
const GPU_SOURCE_SHIFT: u32 = 5;
const GPU_SOURCE_MASK: u32 = 0b111 << GPU_SOURCE_SHIFT;
const GPU_SOURCE_FRONTIER: u32 = 1;
const GPU_SOURCE_LOD_CONNECTOR: u32 = 2;
const GPU_SOURCE_SURFACE_FALLBACK: u32 = 3;
const GPU_SOURCE_WATER: u32 = 4;
const GPU_SOURCE_SKYLINE_PROXY: u32 = 5;
const GPU_SOURCE_LOD_STITCH_TOP: u32 = 6;
const GPU_SOURCE_CROSSING_LOD_CONNECTOR: u32 = 7;
const EXACT_VOLUME_FRONTIER_MESH_KEY: MeshKey = (u8::MAX, 2, 0, 0);
pub const EXACT_VOLUME_FRONTIER_FACE_WORDS: usize = CHUNK_EDGE * CHUNK_EDGE / 64;
/// Transition top triangles reuse the compact quad instance format. The vertex shader decodes
/// one coarse-cell anchor plus two offsets on a boundary edge and degenerates the strip's second
/// triangle. Only the low nine bits remain an offset; every supported surface stride fits there.
const TRANSITION_TRIANGLE_FLAG: u16 = 1 << 14;
const TRANSITION_TRIANGLE_OFFSET_MASK: u16 = (1 << 9) - 1;
const TRANSITION_TRIANGLE_ANCHOR_SHIFT: u16 = 9;
const TRANSITION_TRIANGLE_EDGE_SHIFT: u16 = 11;
/// Greedy canonical rectangles are triangulated from their center to unit boundary segments.
/// Matching every possible 10 cm boundary vertex prevents merged faces from leaving T-junctions
/// against differently sized neighbors while retaining perimeter rather than area complexity.
const CANONICAL_TRIANGLE_FLAG: u16 = 1 << 13;
const CANONICAL_TRIANGLE_OFFSET_MASK: u16 = (1 << 6) - 1;
const CANONICAL_TRIANGLE_EXTENT_SHIFT: u16 = 6;
const CANONICAL_TRIANGLE_EDGE_SHIFT: u16 = 11;
const CANONICAL_TRIANGLE_ANCHOR_SHIFT: u16 = 11;
const CANONICAL_TRIANGLE_SHADOW_OWNER_FLAG: u16 = 1 << 14;
const SURFACE_MACRO_NORMAL_FLAG: u32 = 1 << 24;
const SURFACE_SHAPE_MATERIAL_SHIFT: u32 = 8;
const SURFACE_SHAPE_AO_SHIFT: u32 = 20;
const SURFACE_SHAPE_MIN_DELTA_VOXELS: i32 = -4;
const SURFACE_SHAPE_MAX_DELTA_VOXELS: i32 = 3;
// Sixteen horizon bits occupy otherwise unused material and AO bits: eight cardinal 2-bit angles
// (own + parent LOD). Keeping the parent profile lets the shader use the same geomorph band as
// macro normals instead of popping lighting at a surface-ring handoff.
const SURFACE_HORIZON_MATERIAL_LOW_SHIFT: u32 = 19;
const SURFACE_HORIZON_MATERIAL_HIGH_SHIFT: u32 = 30;
const SURFACE_HORIZON_AO_SHIFT: u32 = 25;
const MORPH_CLOSURE_EXTENT_FLAG: u16 = 1 << 15;
// Decimated height samples are not band-limited. Keeping their full derivative makes a one-voxel
// clipmap snap turn unresolved relief into a false near-horizontal slope (and an almost black
// valley at low sun angles). A conservative macro cue remains legible while staying stable across
// adjacent LOD sampling phases.
const SURFACE_MACRO_SLOPE_SCALE: f32 = 0.40;
const SURFACE_MACRO_SLOPE_MAX: f32 = 0.5;
const LOD_TRANSITION_MESH_KEYS: [MeshKey; 2] = [(u8::MAX, 0, 0, 0), (u8::MAX, 1, 0, 0)];
// A selected cut is already one complete topology product. Publishing its previous and current
// draw plans together reintroduced two independent owners and exposed holes while moving. Keep
// the legacy transition plumbing inert until it can be replaced by vertex morphs inside one mesh.
const CUT_TRANSITION_SECONDS: f32 = 0.0;
const LOD_PLAN_REBUILD_FOCUS: u32 = 1;
const LOD_PLAN_REBUILD_CANONICAL_COLUMNS: u32 = 1 << 1;
const LOD_PLAN_REBUILD_CANONICAL_PROFILE: u32 = 1 << 2;
const LOD_PLAN_REBUILD_SURFACE_RESIDENCY: u32 = 1 << 3;
const LOD_PLAN_REBUILD_SURFACE_PROFILE: u32 = 1 << 4;
const LOD_PLAN_REBUILD_ENCLOSED_VIEW: u32 = 1 << 5;
const LOD_PLAN_REBUILD_CANONICAL_VOLUME: u32 = 1 << 6;
const LOD_SELECTION_WORK_ITEMS_PER_FRAME: usize = 1_024;
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
    pub lod_boundary_half_extents_voxels: [i32; 8],
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VirtualTerrainRendererError {
    Hierarchy(VirtualTerrainError),
    UnsupportedRepresentation(TerrainPageRepresentationKind),
    InvalidSurfaceCluster(TerrainPageKey),
    InvalidTriangleCluster(TerrainPageKey),
    GpuPageTooLarge(TerrainPageKey),
    GpuPoolCapacity,
    GpuTraversal,
    SelectedCutCompactionCapacity,
    NoRenderableCut,
    SelectedPageMissingGpu(TerrainPageKey),
    GpuCutNotCertified,
    IncompleteRootPartition(TerrainPageKey),
    LegacyOwnerCrossesVirtualBoundary,
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
            Self::GpuTraversal => {
                formatter.write_str("virtual terrain GPU traversal state is inconsistent")
            }
            Self::SelectedCutCompactionCapacity => {
                formatter.write_str("selected virtual terrain cut exceeds compact draw capacity")
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
            Self::LegacyOwnerCrossesVirtualBoundary => formatter
                .write_str("legacy terrain ownership crosses a virtual terrain root boundary"),
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
pub struct ScreenshotSurfacePageState {
    pub coord: SurfaceTileCoord,
    pub resident_revision: Option<u64>,
    pub requested_revision: Option<u64>,
    pub queued: bool,
    pub in_flight: bool,
    pub dirty: bool,
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

/// Exact host-side residency/request state captured on the frame that owns screenshot readback.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ScreenshotStreamingManifest {
    pub surface_epoch: u64,
    pub surface_pages: Vec<ScreenshotSurfacePageState>,
    pub canonical_pages: Vec<ScreenshotCanonicalPageState>,
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
            lod_boundary_half_extents_voxels: LOD_BOUNDARY_HALF_EXTENTS,
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
    lod_options: [f32; 4],
    lod_boundary_centres: [[f32; 4]; 4],
    lod_boundary_half_extents: [[f32; 4]; 2],
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

const _: () = assert!(size_of::<FrameUniform>() == 848);
const _: () = assert!(std::mem::offset_of!(FrameUniform, weather) == 768);
const _: () = assert!(std::mem::offset_of!(FrameUniform, cloud_layer) == 784);
const _: () = assert!(std::mem::offset_of!(FrameUniform, medium) == 800);
const _: () = assert!(std::mem::offset_of!(FrameUniform, interior) == 816);
const _: () = assert!(std::mem::offset_of!(FrameUniform, diagnostic_sky) == 832);

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
    lod_options: [f32; 4],
    lod_boundary_centres: [[f32; 4]; 4],
    lod_boundary_half_extents: [[f32; 4]; 2],
}

const _: () = assert!(size_of::<ShadowFrameUniform>() == 192);
const _: () = assert!(std::mem::offset_of!(ShadowFrameUniform, lod_options) == 80);
const _: () = assert!(std::mem::offset_of!(ShadowFrameUniform, lod_boundary_centres) == 96);
const _: () = assert!(std::mem::offset_of!(ShadowFrameUniform, lod_boundary_half_extents) == 160);

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Pod, Zeroable)]
struct GpuQuad {
    origin: [i32; 3],
    extent_voxels: [u16; 2],
    material_face: u32,
    ao: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Pod, Zeroable)]
struct GpuTerrainVertex {
    position: [i32; 3],
    material: u32,
    normal: [i16; 4],
}

const _: () = assert!(size_of::<GpuTerrainVertex>() == 24);

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
struct GpuMorph {
    /// Four exact signed half-voxel deltas, one per source-quad corner.
    deltas: [i16; 4],
}

const _: () = assert!(size_of::<GpuMorph>() == 8);

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

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuCutTransition {
    /// x is the normalized transition phase; y is 0 stable, 1 outgoing, or 2 incoming.
    phase_role: [f32; 4],
    /// The outgoing cut must keep the LOD coordinate system in which it was selected. Otherwise
    /// moving the focus changes its vertex morph on the first transition frame before it fades.
    lod_boundary_centres: [[f32; 4]; 4],
    lod_boundary_half_extents: [[f32; 4]; 2],
}

const _: () = assert!(size_of::<GpuCutTransition>() == 112);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SurfaceCell {
    height: i32,
    /// Height of the exact immediate-parent sample used by this cell's GPU morph sidecar.
    /// Carrying it with the child profile lets connectors remain exact even when streaming skips
    /// one or more selected LOD levels or the parent tile has already left the active cut.
    parent_height: Option<i32>,
    material: Material,
    macro_normal: u32,
    horizon_profile: u16,
    /// Signed three-bit offsets for the four top corners. LOD stitches subdivide the owning
    /// coarse face from this exact source instead of placing a second repair surface over it.
    shape: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SurfacePatchProfile {
    origin: [i32; 2],
    stride: i32,
    cells: Vec<Option<SurfaceCell>>,
}

#[derive(Default)]
struct LodTransitionBuild {
    quads: Vec<GpuQuad>,
    morph_heights: Vec<GpuMorph>,
    exact_edges: HashSet<(SurfacePatchId, u8)>,
    incomplete_edges: u32,
}

impl SurfacePatchProfile {
    fn sample_world(&self, x: i32, z: i32) -> Option<SurfaceCell> {
        let local_x = (i64::from(x) - i64::from(self.origin[0])).div_euclid(i64::from(self.stride));
        let local_z = (i64::from(z) - i64::from(self.origin[1])).div_euclid(i64::from(self.stride));
        if !(0..i64::from(voxels_world::SURFACE_PATCH_EDGE_CELLS)).contains(&local_x)
            || !(0..i64::from(voxels_world::SURFACE_PATCH_EDGE_CELLS)).contains(&local_z)
        {
            return None;
        }
        let edge = voxels_world::SURFACE_PATCH_EDGE_CELLS as usize;
        self.cells[local_x as usize + local_z as usize * edge]
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CanonicalChunkProfile {
    cells: Vec<Option<SurfaceCell>>,
}

type CanonicalColumnProfiles = HashMap<(i32, i32), BTreeMap<i32, CanonicalChunkProfile>>;

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

fn pack_surface_horizon_material(material_face: u32, horizon_profile: u16) -> u32 {
    let profile = u32::from(horizon_profile);
    material_face
        | ((profile & 0xff) << SURFACE_HORIZON_MATERIAL_LOW_SHIFT)
        | (((profile >> 8) & 1) << SURFACE_HORIZON_MATERIAL_HIGH_SHIFT)
}

fn pack_surface_horizon_ao(macro_normal: u32, horizon_profile: u16) -> u32 {
    macro_normal | ((u32::from(horizon_profile) >> 9) << SURFACE_HORIZON_AO_SHIFT)
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

fn canonical_triangle_ao(
    packed: u8,
    edge: SurfacePatchEdge,
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
        SurfacePatchEdge::NegativeX => ([corners[0], corners[3]], extent[1]),
        SurfacePatchEdge::PositiveX => ([corners[1], corners[2]], extent[1]),
        SurfacePatchEdge::NegativeZ => ([corners[0], corners[1]], extent[0]),
        SurfacePatchEdge::PositiveZ => ([corners[3], corners[2]], extent[0]),
    };
    let mut edge_ao = [
        rounded_ao_lerp(edge_corners[0], edge_corners[1], bounds[0], edge_extent),
        rounded_ao_lerp(edge_corners[0], edge_corners[1], bounds[1], edge_extent),
    ];
    if matches!(
        edge,
        SurfacePatchEdge::NegativeX | SurfacePatchEdge::PositiveZ
    ) {
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
            material_face: pack_gpu_material_face(u32::from(material.id()), face),
            // Page clusters do not currently carry per-corner occluders. Encode fully open
            // corners instead of zero, which in the canonical AO convention means maximally
            // occluded.
            ao: u32::from(u8::MAX),
        });
    }
    Ok(gpu_quads)
}

fn virtual_triangle_gpu_vertices(
    page: &TerrainPageV1,
) -> Result<Vec<GpuTerrainVertex>, VirtualTerrainRendererError> {
    let TerrainPageRepresentation::TriangleCluster(cluster) = &page.representation else {
        return Err(VirtualTerrainRendererError::UnsupportedRepresentation(
            page.representation.kind(),
        ));
    };
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
        let edge_a = std::array::from_fn::<_, 3, _>(|axis| {
            f64::from(middle.position[axis]) - f64::from(left.position[axis])
        });
        let edge_b = std::array::from_fn::<_, 3, _>(|axis| {
            f64::from(right.position[axis]) - f64::from(left.position[axis])
        });
        let cross = [
            edge_a[1] * edge_b[2] - edge_a[2] * edge_b[1],
            edge_a[2] * edge_b[0] - edge_a[0] * edge_b[2],
            edge_a[0] * edge_b[1] - edge_a[1] * edge_b[0],
        ];
        let length = (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt();
        if !length.is_finite() || length <= f64::EPSILON {
            return Err(VirtualTerrainRendererError::InvalidTriangleCluster(
                page.key,
            ));
        }
        let normal = std::array::from_fn::<_, 3, _>(|axis| {
            let value = (cross[axis] / length * f64::from(i16::MAX)).round();
            value.clamp(f64::from(i16::MIN + 1), f64::from(i16::MAX)) as i16
        });
        for vertex in [left, middle, right] {
            vertices.push(GpuTerrainVertex {
                position: vertex.position,
                material: u32::from(material.id()),
                normal: [normal[0], normal[1], normal[2], 0],
            });
        }
    }
    Ok(vertices)
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
        surface_patch_id: None,
        boundary_edge: None,
        stitch_edges: 0,
        morph_closure: false,
        exact_replacement_chunk: None,
        canonical_water_surface: render_layer == RenderLayer::Translucent,
        render_layer,
    }
}

fn partition_virtual_surface_geometry(quads: Vec<GpuQuad>) -> Option<(Vec<GpuQuad>, u32, u32)> {
    let (opaque, water): (Vec<_>, Vec<_>) = quads
        .into_iter()
        .partition(|quad| quad.material_face & !GPU_FACE_MASK != u32::from(Material::Water.id()));
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
        .partition(|vertex| vertex.material != u32::from(Material::Water.id()));
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

const fn unpack_canonical_triangle_extent(encoded: u16) -> u16 {
    (((encoded >> CANONICAL_TRIANGLE_EXTENT_SHIFT) & 31) | ((encoded >> (15 - 5)) & 32)) + 1
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
        let constrained = SurfacePatchEdge::ALL
            .into_iter()
            .filter(|edge| edge_offsets[edge.index()].len() > 2)
            .collect::<Vec<_>>();
        let (anchor, fill_edge) = match constrained.as_slice() {
            [SurfacePatchEdge::NegativeX] => (Some(1), Some(SurfacePatchEdge::PositiveZ)),
            [SurfacePatchEdge::PositiveX] => (Some(0), Some(SurfacePatchEdge::PositiveZ)),
            [SurfacePatchEdge::NegativeZ] => (Some(3), Some(SurfacePatchEdge::PositiveX)),
            [SurfacePatchEdge::PositiveZ] => (Some(0), Some(SurfacePatchEdge::PositiveX)),
            [SurfacePatchEdge::NegativeX, SurfacePatchEdge::NegativeZ] => (Some(2), None),
            [SurfacePatchEdge::PositiveX, SurfacePatchEdge::NegativeZ] => (Some(3), None),
            [SurfacePatchEdge::PositiveX, SurfacePatchEdge::PositiveZ] => (Some(0), None),
            [SurfacePatchEdge::NegativeX, SurfacePatchEdge::PositiveZ] => (Some(1), None),
            _ => (None, None),
        };
        let extent = base.extent_voxels;
        let emitted_edges = if anchor.is_some() {
            constrained.into_iter().chain(fill_edge).collect::<Vec<_>>()
        } else {
            SurfacePatchEdge::ALL.to_vec()
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
                    SurfacePatchEdge::NegativeX | SurfacePatchEdge::PositiveX => [0, extent[1]],
                    SurfacePatchEdge::NegativeZ | SurfacePatchEdge::PositiveZ => [0, extent[0]],
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
                            | ((anchor.map_or(0, |corner| corner + 1) as u16)
                                << CANONICAL_TRIANGLE_ANCHOR_SHIFT),
                    ],
                    ao: if preserve_packed_ao {
                        base.ao
                    } else {
                        canonical_triangle_ao(base.ao as u8, edge, [start, end], extent, anchor)
                    },
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
            // Adjacent canonical chunks and the surface-LOD transition mesh are uploaded
            // independently, so their boundary vertices are not present in `base_quads`.
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

fn split_gpu_quad_vertical_extent(quad: GpuQuad, maximum_extent: u16) -> Vec<GpuQuad> {
    let face = ((quad.material_face & GPU_FACE_MASK) >> GPU_FACE_SHIFT) as u8;
    if !matches!(face, 0 | 1 | 4 | 5) || quad.extent_voxels[1] <= maximum_extent {
        return vec![quad];
    }
    let mut output = Vec::new();
    let mut origin_y = quad.origin[1];
    let mut remaining = i32::from(quad.extent_voxels[1]);
    let extent = i32::from(maximum_extent);
    while remaining > 0 {
        let next_boundary = origin_y
            .div_euclid(extent)
            .saturating_add(1)
            .saturating_mul(extent);
        let height = remaining.min(next_boundary.saturating_sub(origin_y).max(1));
        output.push(GpuQuad {
            origin: [quad.origin[0], origin_y, quad.origin[2]],
            extent_voxels: [quad.extent_voxels[0], height as u16],
            ..quad
        });
        origin_y = origin_y.saturating_add(height);
        remaining -= height;
    }
    output
}

fn split_surface_morph(original: GpuQuad, piece: GpuQuad, morph: GpuMorph) -> GpuMorph {
    let height = i64::from(original.extent_voxels[1]);
    if height == 0 || piece == original {
        return morph;
    }
    let start = i64::from(piece.origin[1]) - i64::from(original.origin[1]);
    let end = start + i64::from(piece.extent_voxels[1]);
    let interpolate = |bottom: i16, top: i16, offset: i64| {
        let numerator =
            i64::from(bottom) * (height - offset) + i64::from(top) * offset + height / 2;
        i16::try_from(numerator.div_euclid(height)).unwrap_or_else(|_| {
            debug_assert!(false, "split surface morph exceeds i16");
            if numerator.is_negative() {
                i16::MIN
            } else {
                i16::MAX
            }
        })
    };
    GpuMorph {
        deltas: [
            interpolate(morph.deltas[0], morph.deltas[3], start),
            interpolate(morph.deltas[1], morph.deltas[2], start),
            interpolate(morph.deltas[1], morph.deltas[2], end),
            interpolate(morph.deltas[0], morph.deltas[3], end),
        ],
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct LodDrawPlan {
    patches: SurfacePatchSelection,
    canonical_columns: HashSet<(i32, i32)>,
    canonical_chunks: HashSet<(i32, i32, i32)>,
    enclosed_view_chunks: HashSet<(i32, i32, i32)>,
    exact_transition_edges: HashSet<(SurfacePatchId, u8)>,
    incomplete_transition_edges: u32,
    transition_mesh_key: Option<MeshKey>,
}

#[derive(Clone, Debug)]
struct CutTransition {
    from: LodDrawPlan,
    from_focus: Option<GeometricLodFocus>,
    started_at: f32,
}

fn cut_transition_is_active(started_at: Option<f32>, time: f32) -> bool {
    started_at.is_some_and(|started_at| time - started_at < CUT_TRANSITION_SECONDS)
}

struct PendingSurfaceSelection {
    focus: GeometricLodFocus,
    canonical_columns: HashSet<(i32, i32)>,
    build: SurfacePatchSelectionBuild,
}

impl LodDrawPlan {
    fn has_geometry(&self) -> bool {
        self.patches.owned_patches().next().is_some()
            || !self.canonical_columns.is_empty()
            || !self.canonical_chunks.is_empty()
            || !self.enclosed_view_chunks.is_empty()
    }

    fn owns_patch(&self, patch: SurfacePatchId) -> bool {
        self.patches.owns(patch)
    }

    fn owns_canonical_column(&self, chunk_x: i32, chunk_z: i32) -> bool {
        self.canonical_columns.contains(&(chunk_x, chunk_z))
    }

    fn owns_canonical_chunk(&self, key: &MeshKey) -> bool {
        key.0 == 0 && self.canonical_chunks.contains(&(key.1, key.2, key.3))
    }

    fn owns_enclosed_view_chunk(&self, key: &MeshKey) -> bool {
        key.0 == 0 && self.enclosed_view_chunks.contains(&(key.1, key.2, key.3))
    }

    fn owns_exact_volume_coord(&self, coord: (i32, i32, i32)) -> bool {
        self.canonical_chunks.contains(&coord) || self.enclosed_view_chunks.contains(&coord)
    }

    fn owns_surface_top_edge(&self, patch: SurfacePatchId, edge: SurfacePatchEdge) -> bool {
        self.owns_patch(patch)
            && !self
                .exact_transition_edges
                .contains(&(patch, edge.index() as u8))
    }

    fn connector_owns_boundary_edge(&self, patch: SurfacePatchId, edge: SurfacePatchEdge) -> bool {
        if self
            .exact_transition_edges
            .contains(&(patch, edge.index() as u8))
        {
            return true;
        }
        let Some([[min_x, min_z], [max_x, max_z]]) = patch.voxel_bounds_xz() else {
            return false;
        };
        let center_x = min_x.saturating_add(max_x.saturating_sub(min_x) / 2);
        let center_z = min_z.saturating_add(max_z.saturating_sub(min_z) / 2);
        let across = match edge {
            SurfacePatchEdge::NegativeX => [min_x.saturating_sub(1), center_z],
            SurfacePatchEdge::PositiveX => [max_x, center_z],
            SurfacePatchEdge::NegativeZ => [center_x, min_z.saturating_sub(1)],
            SurfacePatchEdge::PositiveZ => [center_x, max_z],
        };
        self.patches
            .selected_patch_at(across)
            .filter(|neighbor| neighbor.level.stride_voxels() > patch.level.stride_voxels())
            .is_some_and(|neighbor| {
                self.exact_transition_edges
                    .contains(&(neighbor, opposite_surface_patch_edge(edge).index() as u8))
            })
    }

    fn owns_boundary_wall_edge(&self, patch: SurfacePatchId, edge: SurfacePatchEdge) -> bool {
        self.owns_patch(patch) && !self.connector_owns_boundary_edge(patch, edge)
    }

    fn presented_stride_at(
        &self,
        focus: Option<GeometricLodFocus>,
        voxel_x: i32,
        voxel_y: i32,
        voxel_z: i32,
    ) -> u16 {
        let chunk_x = voxel_x.div_euclid(CHUNK_EDGE as i32);
        let chunk_y = voxel_y.div_euclid(CHUNK_EDGE as i32);
        let chunk_z = voxel_z.div_euclid(CHUNK_EDGE as i32);
        if self.owns_canonical_chunk(&(0, chunk_x, chunk_y, chunk_z))
            || self.owns_enclosed_view_chunk(&(0, chunk_x, chunk_y, chunk_z))
        {
            return 1;
        }
        if focus.is_some_and(|focus| {
            focus.owner_at(voxel_x, voxel_z) == LodOwner::Canonical
                && self.owns_canonical_column(chunk_x, chunk_z)
        }) {
            return 1;
        }
        self.patches
            .selected_patch_at([voxel_x, voxel_z])
            .map_or(0, |patch| patch.level.stride_voxels() as u16)
    }
}

struct ChunkMesh {
    allocation: Allocation,
    morph_allocation: Option<Allocation>,
    quad_count: u32,
    content_fingerprint: u64,
    slices: Vec<MeshSlice>,
    lod_ownership_focus: Option<GeometricLodFocus>,
    lod_ownership_stale: bool,
    lod_owned_slices: Vec<bool>,
    bounds_min: glam::Vec3,
    bounds_max: glam::Vec3,
    activation_mask: u8,
}

struct VirtualTerrainGpuPage {
    revision: u64,
    content_fingerprint: [u8; 32],
    representation: TerrainPageRepresentationKind,
    mesh: VirtualTerrainGpuMesh,
}

enum VirtualTerrainGpuMesh {
    Empty,
    Surface(ChunkMesh),
    Triangle(TerrainTriangleMesh),
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
    coord: ChunkCoord,
    key: MeshKey,
    surface_profile: CanonicalChunkProfile,
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
    fn refresh_lod_ownership(
        &mut self,
        key: &MeshKey,
        focus: Option<GeometricLodFocus>,
        lod_draw_plan: Option<&LodDrawPlan>,
    ) -> bool {
        let Some(focus) = focus else {
            return false;
        };
        let canonical = key.0 == 0;
        if !self.lod_ownership_stale
            && (!canonical || self.lod_ownership_focus == Some(focus))
            && self.lod_owned_slices.len() == self.slices.len()
        {
            return false;
        }
        self.lod_owned_slices = self
            .slices
            .iter()
            .map(|slice| slice_owned_by_lod(Some(focus), lod_draw_plan, key, slice))
            .collect();
        self.lod_ownership_focus = Some(focus);
        self.lod_ownership_stale = false;
        true
    }

    fn lod_owns_slice(
        &self,
        key: &MeshKey,
        focus: Option<GeometricLodFocus>,
        slice_index: usize,
    ) -> bool {
        focus.map_or(key.0 == 0, |_| {
            self.lod_owned_slices.get(slice_index) == Some(&true)
        })
    }

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
    surface_patch_id: Option<SurfacePatchId>,
    boundary_edge: Option<SurfacePatchEdge>,
    /// Patch edges touched by a coarse top face. If any of these edges has an exact LOD stitch,
    /// this source face is suppressed and replaced by the stitch's non-overlapping subfaces.
    stitch_edges: u8,
    morph_closure: bool,
    /// Synthetic heightfield cover is owned until this exact-volume chunk is resident.
    exact_replacement_chunk: Option<(i32, i32, i32)>,
    /// Canonical water top faces follow the 2D surface cut. Enclosed-view chunks may still own
    /// their volume faces without drawing a second coplanar free surface over streamed water.
    canonical_water_surface: bool,
    render_layer: RenderLayer,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DrawItem {
    page: u16,
    offset: u32,
    size: u32,
    quad_count: u32,
    morph_page: Option<u16>,
    morph_offset: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DrawSpan {
    page: u16,
    offset: u32,
    size: u32,
    quad_count: u32,
    morph_page: Option<u16>,
    morph_offset: u32,
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

/// Complete fixed-region volumes owned by the currently published virtual cut.
///
/// The renderer never guesses ownership from visual depth. A region joins this set only when the
/// selected pages form a complete octree partition of its 25.6 m root. Legacy slices can then be
/// retired iff their entire conservative bounds are covered by this set; a crossing slice blocks
/// publication rather than allowing either a gap or overlapping sources.
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
            .filter_map(|key| key.ancestor_at(TERRAIN_REGION_ROOT_LEVEL))
            .collect::<BTreeSet<_>>();
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

    fn intersects_aabb(&self, minimum: glam::Vec3, maximum: glam::Vec3) -> bool {
        let Some((minimum, maximum)) = voxel_bounds_from_metres(minimum, maximum) else {
            return false;
        };
        terrain_root_coords_for_bounds(minimum, maximum)
            .is_some_and(|ranges| terrain_root_coords(ranges).any(|key| self.roots.contains(&key)))
    }

    fn covers_voxel_bounds(&self, minimum: [i32; 3], maximum: [i32; 3]) -> bool {
        let Some(ranges) = terrain_root_coords_for_bounds(minimum, maximum) else {
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
        terrain_root_coords(ranges).all(|key| self.roots.contains(&key))
    }
}

fn selected_pages_cover(key: TerrainPageKey, selected: &BTreeSet<TerrainPageKey>) -> bool {
    selected.contains(&key)
        || key.children().is_some_and(|children| {
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

fn terrain_root_coords_for_bounds(minimum: [i32; 3], maximum: [i32; 3]) -> Option<[[i32; 2]; 3]> {
    let root_span =
        i32::try_from(32_u32.checked_shl(u32::from(TERRAIN_REGION_ROOT_LEVEL))?).ok()?;
    minimum
        .into_iter()
        .zip(maximum)
        .all(|(minimum, maximum)| minimum < maximum)
        .then(|| {
            std::array::from_fn(|axis| {
                [
                    minimum[axis].div_euclid(root_span),
                    maximum[axis].saturating_sub(1).div_euclid(root_span),
                ]
            })
        })
}

fn terrain_root_coords(ranges: [[i32; 2]; 3]) -> impl Iterator<Item = TerrainPageKey> {
    (ranges[0][0]..=ranges[0][1]).flat_map(move |x| {
        (ranges[1][0]..=ranges[1][1]).flat_map(move |y| {
            (ranges[2][0]..=ranges[2][1]).map(move |z| TerrainPageKey {
                level: TERRAIN_REGION_ROOT_LEVEL,
                coord: [x, y, z],
            })
        })
    })
}

/// Camera-visible opaque geometry split by whether its vertices can move in the current LOD band.
/// Most resident geometry is fixed, so its pipeline can compile out parent-height decoding and
/// boundary-distance math while the narrow morph band retains the exact same geometry contract.
#[derive(Debug, Default, Eq, PartialEq)]
struct WorldDrawLists {
    fixed: DrawList,
    morphing: DrawList,
    mesh_count: u32,
    quad_count: u32,
    fingerprint: u64,
    tested_slices: u32,
    selected_slices: u32,
}

#[derive(Debug, Default, Eq, PartialEq)]
struct CutDrawLists {
    incoming: WorldDrawLists,
    outgoing: WorldDrawLists,
    /// Incoming transition geometry is another draw of one exact current slice. Replacing its
    /// whole patch would also suppress fixed top/wall slices that never entered the transition,
    /// opening a patch-shaped hole while the camera moves.
    replaced_current_slices: HashSet<(MeshKey, usize)>,
    /// A departing set of four fine patches can replace the current coarse parent as a unit.
    replaced_current_patches: HashSet<SurfacePatchId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MissingMorphSidecar;

#[derive(Debug)]
struct WorldDrawListBuilder {
    fixed: DrawListBuilder,
    morphing: DrawListBuilder,
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
            morphing: DrawListBuilder::without_fingerprint(),
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

    fn select_slice(
        &mut self,
        chunk: &ChunkMesh,
        slice: &MeshSlice,
        morphing: bool,
    ) -> Result<(), MissingMorphSidecar> {
        self.selected_slices = self.selected_slices.saturating_add(1);
        self.quad_count = self.quad_count.saturating_add(slice.quad_count);
        if morphing {
            self.morphing.select_morph_slice(chunk, slice)?;
        } else {
            self.fixed.select_slice(chunk, slice);
        }
        Ok(())
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
        let morphing = self.morphing.finish();
        for (role, draw_list) in [(0_u64, &fixed), (1, &morphing)] {
            self.fingerprint = fingerprint_value(self.fingerprint, role);
            for span in &draw_list.spans {
                self.fingerprint = fingerprint_value(self.fingerprint, u64::from(span.page));
                self.fingerprint = fingerprint_value(self.fingerprint, u64::from(span.offset));
                self.fingerprint = fingerprint_value(self.fingerprint, u64::from(span.size));
                self.fingerprint = fingerprint_value(self.fingerprint, u64::from(span.quad_count));
                self.fingerprint = fingerprint_value(
                    self.fingerprint,
                    span.morph_page.map_or(u64::MAX, u64::from),
                );
                self.fingerprint =
                    fingerprint_value(self.fingerprint, u64::from(span.morph_offset));
            }
        }
        WorldDrawLists {
            fixed,
            morphing,
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

    #[cfg(test)]
    fn test_slice(&mut self) {
        self.tested_slices = self.tested_slices.saturating_add(1);
    }

    fn select_slice(&mut self, chunk: &ChunkMesh, slice: &MeshSlice) {
        self.selected_slices = self.selected_slices.saturating_add(1);
        let offset = chunk.allocation.offset + slice.relative_offset;
        self.items.push(DrawItem {
            page: chunk.allocation.page,
            offset,
            size: slice.size,
            quad_count: slice.quad_count,
            morph_page: None,
            morph_offset: 0,
        });
        self.quad_count = self.quad_count.saturating_add(slice.quad_count);
    }

    fn select_morph_slice(
        &mut self,
        chunk: &ChunkMesh,
        slice: &MeshSlice,
    ) -> Result<(), MissingMorphSidecar> {
        let morph_allocation = chunk.morph_allocation.ok_or(MissingMorphSidecar)?;
        let quad_bytes = size_of::<GpuQuad>() as u32;
        debug_assert_eq!(slice.relative_offset % quad_bytes, 0);
        let first_quad = slice.relative_offset / quad_bytes;
        self.selected_slices = self.selected_slices.saturating_add(1);
        self.items.push(DrawItem {
            page: chunk.allocation.page,
            offset: chunk.allocation.offset + slice.relative_offset,
            size: slice.size,
            quad_count: slice.quad_count,
            morph_page: Some(morph_allocation.page),
            morph_offset: morph_allocation.offset + first_quad * size_of::<GpuMorph>() as u32,
        });
        self.quad_count = self.quad_count.saturating_add(slice.quad_count);
        Ok(())
    }

    #[cfg(test)]
    fn select_mesh(&mut self, key: MeshKey, chunk: &ChunkMesh) {
        self.mesh_count = self.mesh_count.saturating_add(1);
        if self.fingerprint_enabled {
            self.fingerprint = fingerprint_value(self.fingerprint, u64::from(key.0));
            self.fingerprint = fingerprint_value(self.fingerprint, key.1 as u32 as u64);
            self.fingerprint = fingerprint_value(self.fingerprint, key.2 as u32 as u64);
            self.fingerprint = fingerprint_value(self.fingerprint, key.3 as u32 as u64);
            self.fingerprint = fingerprint_value(self.fingerprint, chunk.content_fingerprint);
        }
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
                self.fingerprint = fingerprint_value(
                    self.fingerprint,
                    span.morph_page.map_or(u64::MAX, u64::from),
                );
                self.fingerprint =
                    fingerprint_value(self.fingerprint, u64::from(span.morph_offset));
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
    pub virtual_terrain_gpu_selected_pages: u32,
    pub virtual_terrain_gpu_requested_pages: u32,
    pub virtual_terrain_gpu_ownerless_roots: u32,
    pub virtual_terrain_gpu_visited_nodes: u32,
    pub virtual_terrain_gpu_overflow_flags: u32,
    pub virtual_terrain_gpu_stack_peak: u32,
    pub virtual_terrain_gpu_compacted_surface_elements: u32,
    pub virtual_terrain_gpu_compacted_triangle_elements: u32,
    pub virtual_terrain_gpu_compacted_water_surface_elements: u32,
    pub virtual_terrain_gpu_compacted_water_triangle_elements: u32,
    pub virtual_terrain_gpu_compacted_pages: u32,
    pub virtual_terrain_gpu_compaction_overflow_flags: u32,
    pub virtual_terrain_gpu_matches_cpu_cut: bool,
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
    pub gpu_virtual_terrain_traversal_ms: Option<f32>,
    pub gpu_virtual_terrain_compaction_ms: Option<f32>,
    pub cpu_cull_ms: f32,
    pub cpu_lod_plan_ms: f32,
    pub lod_plan_rebuild_reason: u32,
    pub cpu_encode_ms: f32,
    pub cpu_submit_ms: f32,
    pub lod_ownership_refreshes: u32,
    pub draw_list_tested_slices: u32,
    pub draw_list_selected_slices: u32,
    /// Number of exact resident-profile connector quads selected for the current LOD focus.
    pub lod_transition_quads: u32,
    /// Candidate LOD edges still covered by their resident source edge because an exact connector
    /// was not complete when the current draw plan was installed.
    pub lod_incomplete_transition_edges: u32,
    /// Whether the latest presented viewport contains both sides of an active geometric LOD cut.
    pub lod_cut_transition_active: bool,
    /// Normalized lifetime of the active geometric LOD cut, or zero while no cut is active.
    pub lod_cut_transition_phase: f32,
    /// Grid-snapped centres, in canonical voxels, for the eight geometric LOD boundaries.
    pub lod_boundary_centres: [[i32; 2]; 8],
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
    pub virtual_terrain_traversal_ms: f32,
    pub virtual_terrain_compaction_ms: f32,
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
    let virtual_terrain_traversal_ms = if passes.virtual_terrain {
        elapsed_ms(24, 25)?
    } else {
        0.0
    };
    let virtual_terrain_compaction_ms = if passes.virtual_terrain {
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
        virtual_terrain_traversal_ms,
        virtual_terrain_compaction_ms,
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
    depth_prepass_morph_pipeline: RenderPipeline,
    depth_prepass_transition_fixed_pipeline: RenderPipeline,
    depth_prepass_transition_pipeline: RenderPipeline,
    voxel_pipeline: RenderPipeline,
    voxel_flat_pipeline: RenderPipeline,
    voxel_ambient_occlusion_pipeline: RenderPipeline,
    voxel_ambient_occlusion_flat_pipeline: RenderPipeline,
    voxel_morph_pipeline: RenderPipeline,
    voxel_morph_flat_pipeline: RenderPipeline,
    voxel_morph_ambient_occlusion_pipeline: RenderPipeline,
    voxel_morph_ambient_occlusion_flat_pipeline: RenderPipeline,
    voxel_transition_pipeline: RenderPipeline,
    voxel_transition_flat_pipeline: RenderPipeline,
    voxel_transition_ambient_occlusion_pipeline: RenderPipeline,
    voxel_transition_ambient_occlusion_flat_pipeline: RenderPipeline,
    voxel_morph_transition_pipeline: RenderPipeline,
    voxel_morph_transition_flat_pipeline: RenderPipeline,
    voxel_morph_transition_ambient_occlusion_pipeline: RenderPipeline,
    voxel_morph_transition_ambient_occlusion_flat_pipeline: RenderPipeline,
    virtual_triangle_depth_pipeline: RenderPipeline,
    virtual_triangle_pipeline: RenderPipeline,
    virtual_triangle_flat_pipeline: RenderPipeline,
    virtual_triangle_ambient_occlusion_pipeline: RenderPipeline,
    virtual_triangle_ambient_occlusion_flat_pipeline: RenderPipeline,
    virtual_triangle_diagnostic_pipeline: RenderPipeline,
    screenshot_diagnostic_pipeline: RenderPipeline,
    screenshot_diagnostic_morph_pipeline: RenderPipeline,
    screenshot_diagnostic_transition_pipeline: RenderPipeline,
    screenshot_diagnostic_morph_transition_pipeline: RenderPipeline,
    water_pipeline: RenderPipeline,
    virtual_triangle_water_pipeline: RenderPipeline,
    water_transition_pipeline: RenderPipeline,
    weather_pipeline: RenderPipeline,
    avatar_gpu: AvatarGpu,
    remote_avatars: Vec<RemoteAvatarPose>,
    water_scene_layout: wgpu::BindGroupLayout,
    water_scene_bind_group: BindGroup,
    shadow_gpu: ShadowGpu,
    shadow_direction: ShadowDirectionTracker,
    frame_buffer: Buffer,
    frame_bind_group: BindGroup,
    cut_transition_buffers: [Buffer; 2],
    cut_transition_bind_groups: [BindGroup; 2],
    local_light_buffer: Buffer,
    material_detail: MaterialDetailGpu,
    chunks: BTreeMap<MeshKey, ChunkMesh>,
    water_chunks: BTreeMap<MeshKey, ChunkMesh>,
    virtual_terrain: VirtualTerrainHierarchy,
    virtual_terrain_gpu: VirtualTerrainGpuControl,
    virtual_terrain_mode: VirtualTerrainRenderMode,
    virtual_terrain_cut: Option<VirtualTerrainCut>,
    virtual_terrain_oracle_cut: Option<VirtualTerrainCut>,
    virtual_terrain_oracle_view: Option<VirtualTerrainView>,
    virtual_terrain_pages: BTreeMap<TerrainPageKey, VirtualTerrainGpuPage>,
    virtual_terrain_arena: ArenaAllocator,
    virtual_terrain_arena_buffers: Vec<Buffer>,
    surface_patch_profiles: HashMap<SurfacePatchId, SurfacePatchProfile>,
    canonical_surface_profiles: CanonicalColumnProfiles,
    surface_patch_residency: HashSet<SurfacePatchId>,
    surface_incomplete_parents: HashSet<SurfacePatchId>,
    canonical_ready_chunks: HashSet<(i32, i32, i32)>,
    canonical_surface_ready_chunks: HashSet<(i32, i32, i32)>,
    enclosed_view_ready_chunks: HashSet<(i32, i32, i32)>,
    surface_patch_residency_revision: u64,
    lod_draw_plan: LodDrawPlan,
    lod_draw_plan_focus: Option<GeometricLodFocus>,
    lod_draw_plan_revision: u64,
    lod_draw_plan_dirty_reasons: u32,
    pending_surface_selection: Option<PendingSurfaceSelection>,
    cut_transition: Option<CutTransition>,
    chunk_activations: ChunkActivations,
    local_light_candidates: BTreeMap<MeshKey, Vec<GpuLocalLight>>,
    arena: ArenaAllocator,
    arena_buffers: Vec<Buffer>,
    morph_arena: ArenaAllocator,
    morph_arena_buffers: Vec<Buffer>,
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
    geometric_lod_focus: Option<GeometricLodFocus>,
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
    morph_pipeline: RenderPipeline,
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
            std::array::from_fn(|index| shadow_frame_uniform(&cascades, index, camera, None));
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
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/shadow.wgsl"));
        let fixed_pipeline = shadow_caster_pipeline(
            device,
            "fixed shadow caster pipeline",
            &pipeline_layout,
            &shader,
            false,
        );
        let morph_pipeline = shadow_caster_pipeline(
            device,
            "morphing shadow caster pipeline",
            &pipeline_layout,
            &shader,
            true,
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
            morph_pipeline,
            virtual_triangle_pipeline,
        })
    }

    fn write_cascades(
        &self,
        queue: &Queue,
        cascades: &DirectionalShadowCascades,
        camera: &CameraState,
        lod_focus: Option<GeometricLodFocus>,
    ) {
        for index in 0..CASCADE_COUNT {
            let uniform = shadow_frame_uniform(cascades, index, camera, lod_focus);
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
        if !lod_boundary_half_extents_are_valid(runtime_config.lod_boundary_half_extents_voxels) {
            return Err(
                "renderer LOD boundary half extents must be positive and strictly increasing"
                    .to_owned(),
            );
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
        let shadow_gpu = ShadowGpu::new(
            &device,
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
            None,
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
        let cut_transition_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("complete cut transition layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let cut_transition_buffers = std::array::from_fn(|role| {
            let encoded_role = if role == 0 { 2.0 } else { 1.0 };
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("complete cut transition uniform"),
                contents: bytemuck::bytes_of(&gpu_cut_transition(1.0, encoded_role, None)),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            })
        });
        let cut_transition_bind_groups = std::array::from_fn(|role| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("complete cut transition bind group"),
                layout: &cut_transition_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: cut_transition_buffers[role].as_entire_binding(),
                }],
            })
        });
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
                    Some(&cut_transition_layout),
                ],
                immediate_size: 0,
            });
        let cut_transition_depth_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("complete cut transition depth layout"),
                bind_group_layouts: &[
                    Some(&frame_layout),
                    None,
                    None,
                    Some(&cut_transition_layout),
                ],
                immediate_size: 0,
            });
        let water_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("water pipeline layout"),
                bind_group_layouts: &[
                    Some(&frame_layout),
                    Some(&water_scene_layout),
                    None,
                    Some(&cut_transition_layout),
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
            false,
        );
        let depth_prepass_morph_pipeline = fragmentless_depth_pipeline(
            &device,
            "spatial AO morph depth pipeline",
            &sky_pipeline_layout,
            &voxel_shader,
            true,
        );
        let virtual_triangle_depth_pipeline = virtual_triangle_depth_pipeline(
            &device,
            "virtual terrain triangle depth pipeline",
            &sky_pipeline_layout,
            &voxel_shader,
        );
        let depth_prepass_transition_fixed_pipeline = transition_depth_pipeline(
            &device,
            "complete cut transition fixed depth pipeline",
            &cut_transition_depth_layout,
            &voxel_shader,
            false,
        );
        let depth_prepass_transition_pipeline = transition_depth_pipeline(
            &device,
            "complete cut transition depth pipeline",
            &cut_transition_depth_layout,
            &voxel_shader,
            true,
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
        let voxel_morph_pipeline = create_voxel_pipeline(
            &device,
            "morphing voxel pipeline",
            &world_pipeline_layout,
            &voxel_shader,
            VoxelPipelineVariant::new(true, false).morphing(),
        );
        let voxel_morph_flat_pipeline = create_voxel_pipeline(
            &device,
            "flat morphing voxel pipeline",
            &world_pipeline_layout,
            &voxel_shader,
            VoxelPipelineVariant::new(false, false).morphing(),
        );
        let voxel_morph_ambient_occlusion_pipeline = create_voxel_pipeline(
            &device,
            "spatial AO morphing voxel pipeline",
            &world_pipeline_layout,
            &voxel_shader,
            VoxelPipelineVariant::new(true, true).morphing(),
        );
        let voxel_morph_ambient_occlusion_flat_pipeline = create_voxel_pipeline(
            &device,
            "flat spatial AO morphing voxel pipeline",
            &world_pipeline_layout,
            &voxel_shader,
            VoxelPipelineVariant::new(false, true).morphing(),
        );
        let voxel_transition_pipeline = create_voxel_pipeline(
            &device,
            "transition voxel pipeline",
            &world_pipeline_layout,
            &voxel_shader,
            VoxelPipelineVariant::new(true, false).transition(),
        );
        let voxel_transition_flat_pipeline = create_voxel_pipeline(
            &device,
            "flat transition voxel pipeline",
            &world_pipeline_layout,
            &voxel_shader,
            VoxelPipelineVariant::new(false, false).transition(),
        );
        let voxel_transition_ambient_occlusion_pipeline = create_voxel_pipeline(
            &device,
            "spatial AO transition voxel pipeline",
            &world_pipeline_layout,
            &voxel_shader,
            VoxelPipelineVariant::new(true, true).transition(),
        );
        let voxel_transition_ambient_occlusion_flat_pipeline = create_voxel_pipeline(
            &device,
            "flat spatial AO transition voxel pipeline",
            &world_pipeline_layout,
            &voxel_shader,
            VoxelPipelineVariant::new(false, true).transition(),
        );
        let voxel_morph_transition_pipeline = create_voxel_pipeline(
            &device,
            "morphing transition voxel pipeline",
            &world_pipeline_layout,
            &voxel_shader,
            VoxelPipelineVariant::new(true, false).morphing_transition(),
        );
        let voxel_morph_transition_flat_pipeline = create_voxel_pipeline(
            &device,
            "flat morphing transition voxel pipeline",
            &world_pipeline_layout,
            &voxel_shader,
            VoxelPipelineVariant::new(false, false).morphing_transition(),
        );
        let voxel_morph_transition_ambient_occlusion_pipeline = create_voxel_pipeline(
            &device,
            "spatial AO morphing transition voxel pipeline",
            &world_pipeline_layout,
            &voxel_shader,
            VoxelPipelineVariant::new(true, true).morphing_transition(),
        );
        let voxel_morph_transition_ambient_occlusion_flat_pipeline = create_voxel_pipeline(
            &device,
            "flat spatial AO morphing transition voxel pipeline",
            &world_pipeline_layout,
            &voxel_shader,
            VoxelPipelineVariant::new(false, true).morphing_transition(),
        );
        let virtual_triangle_pipeline = create_virtual_triangle_pipeline(
            &device,
            "virtual terrain triangle pipeline",
            &world_pipeline_layout,
            &voxel_shader,
            true,
            false,
        );
        let virtual_triangle_flat_pipeline = create_virtual_triangle_pipeline(
            &device,
            "flat virtual terrain triangle pipeline",
            &world_pipeline_layout,
            &voxel_shader,
            false,
            false,
        );
        let virtual_triangle_ambient_occlusion_pipeline = create_virtual_triangle_pipeline(
            &device,
            "spatial AO virtual terrain triangle pipeline",
            &world_pipeline_layout,
            &voxel_shader,
            true,
            true,
        );
        let virtual_triangle_ambient_occlusion_flat_pipeline = create_virtual_triangle_pipeline(
            &device,
            "flat spatial AO virtual terrain triangle pipeline",
            &world_pipeline_layout,
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
            VoxelPipelineVariant::new(false, false),
        );
        let screenshot_diagnostic_morph_pipeline = create_voxel_diagnostic_pipeline(
            &device,
            "screenshot integer diagnostic morph pipeline",
            &world_pipeline_layout,
            &voxel_shader,
            VoxelPipelineVariant::new(false, false).morphing(),
        );
        let screenshot_diagnostic_transition_pipeline = create_voxel_diagnostic_pipeline(
            &device,
            "screenshot integer diagnostic transition pipeline",
            &world_pipeline_layout,
            &voxel_shader,
            VoxelPipelineVariant::new(false, false).transition(),
        );
        let screenshot_diagnostic_morph_transition_pipeline = create_voxel_diagnostic_pipeline(
            &device,
            "screenshot integer diagnostic morph transition pipeline",
            &world_pipeline_layout,
            &voxel_shader,
            VoxelPipelineVariant::new(false, false).morphing_transition(),
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
        let virtual_triangle_water_pipeline = create_virtual_triangle_water_pipeline(
            &device,
            "virtual terrain triangle water pipeline",
            &water_pipeline_layout,
            &voxel_shader,
        );
        let water_transition_pipeline = pipeline(
            &device,
            "outgoing water transition pipeline",
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
                fragment_constants: &[("CUT_TRANSITION", 1.0)],
            },
        );

        let ui_gpu = UiGpu::new(&device, format, config.width, config.height, dpr)?;
        let water_scene_bind_group =
            ui_gpu.water_scene_bind_group(&device, &water_scene_layout, opaque_depth.view());

        let placement_inventory = PlacementInventory::new();
        let virtual_terrain_capacity = VirtualTerrainCapacity::DEVELOPMENT_128_MIB;
        let virtual_terrain = VirtualTerrainHierarchy::new(virtual_terrain_capacity)
            .map_err(|error| format!("virtual terrain hierarchy: {error}"))?;
        let virtual_terrain_gpu = VirtualTerrainGpuControl::new(&device, virtual_terrain_capacity)
            .map_err(|error| format!("virtual terrain GPU control: {error:?}"))?;
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
            depth_prepass_morph_pipeline,
            depth_prepass_transition_fixed_pipeline,
            depth_prepass_transition_pipeline,
            voxel_pipeline,
            voxel_flat_pipeline,
            voxel_ambient_occlusion_pipeline,
            voxel_ambient_occlusion_flat_pipeline,
            voxel_morph_pipeline,
            voxel_morph_flat_pipeline,
            voxel_morph_ambient_occlusion_pipeline,
            voxel_morph_ambient_occlusion_flat_pipeline,
            voxel_transition_pipeline,
            voxel_transition_flat_pipeline,
            voxel_transition_ambient_occlusion_pipeline,
            voxel_transition_ambient_occlusion_flat_pipeline,
            voxel_morph_transition_pipeline,
            voxel_morph_transition_flat_pipeline,
            voxel_morph_transition_ambient_occlusion_pipeline,
            voxel_morph_transition_ambient_occlusion_flat_pipeline,
            virtual_triangle_depth_pipeline,
            virtual_triangle_pipeline,
            virtual_triangle_flat_pipeline,
            virtual_triangle_ambient_occlusion_pipeline,
            virtual_triangle_ambient_occlusion_flat_pipeline,
            virtual_triangle_diagnostic_pipeline,
            screenshot_diagnostic_pipeline,
            screenshot_diagnostic_morph_pipeline,
            screenshot_diagnostic_transition_pipeline,
            screenshot_diagnostic_morph_transition_pipeline,
            water_pipeline,
            virtual_triangle_water_pipeline,
            water_transition_pipeline,
            weather_pipeline,
            avatar_gpu,
            remote_avatars: Vec::new(),
            water_scene_layout,
            water_scene_bind_group,
            shadow_gpu,
            shadow_direction,
            frame_buffer,
            frame_bind_group,
            cut_transition_buffers,
            cut_transition_bind_groups,
            local_light_buffer,
            material_detail,
            chunks: BTreeMap::new(),
            water_chunks: BTreeMap::new(),
            virtual_terrain,
            virtual_terrain_gpu,
            virtual_terrain_mode: VirtualTerrainRenderMode::Disabled,
            virtual_terrain_cut: None,
            virtual_terrain_oracle_cut: None,
            virtual_terrain_oracle_view: None,
            virtual_terrain_pages: BTreeMap::new(),
            virtual_terrain_arena,
            virtual_terrain_arena_buffers: Vec::new(),
            surface_patch_profiles: HashMap::new(),
            canonical_surface_profiles: HashMap::new(),
            surface_patch_residency: HashSet::new(),
            surface_incomplete_parents: HashSet::new(),
            canonical_ready_chunks: HashSet::new(),
            canonical_surface_ready_chunks: HashSet::new(),
            enclosed_view_ready_chunks: HashSet::new(),
            surface_patch_residency_revision: 0,
            lod_draw_plan: LodDrawPlan::default(),
            lod_draw_plan_focus: None,
            lod_draw_plan_revision: u64::MAX,
            lod_draw_plan_dirty_reasons: 0,
            pending_surface_selection: None,
            cut_transition: None,
            chunk_activations: ChunkActivations::default(),
            local_light_candidates: BTreeMap::new(),
            arena: ArenaAllocator::new(ARENA_PAGE_BYTES, size_of::<GpuQuad>() as u32),
            arena_buffers: Vec::new(),
            morph_arena: ArenaAllocator::new(ARENA_PAGE_BYTES, size_of::<GpuMorph>() as u32),
            morph_arena_buffers: Vec::new(),
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
            geometric_lod_focus: None,
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

    pub fn advance_geometric_lod_focus(
        &mut self,
        voxel_x: i32,
        voxel_z: i32,
        ready_level_count: usize,
        surface_level_count: usize,
    ) {
        self.geometric_lod_focus = Some(self.geometric_lod_focus.map_or_else(
            || {
                GeometricLodFocus::snapped_with_half_extents_for_levels(
                    voxel_x,
                    voxel_z,
                    surface_level_count,
                    self.runtime_config.lod_boundary_half_extents_voxels,
                )
            },
            |focus| {
                focus.advanced_for_levels(voxel_x, voxel_z, ready_level_count, surface_level_count)
            },
        ));
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

    /// Atomically replaces the exact-volume set and the independently complete surface columns.
    ///
    /// Installing these together is the renderer's coverage transaction: a stride-two patch is
    /// never suppressed before every exact chunk intended to replace it is renderable, and those
    /// exact meshes are never withheld for a frame after the fallback is removed.
    pub fn set_canonical_cut_ready_chunks(
        &mut self,
        canonical_chunks: impl IntoIterator<Item = (i32, i32, i32)>,
        surface_chunks: impl IntoIterator<Item = (i32, i32, i32)>,
    ) {
        let canonical_replacement = canonical_chunks.into_iter().collect::<HashSet<_>>();
        let surface_replacement = surface_chunks.into_iter().collect::<HashSet<_>>();
        if canonical_replacement == self.canonical_ready_chunks
            && surface_replacement == self.canonical_surface_ready_chunks
        {
            return;
        }
        let focus = self.lod_draw_plan_focus;
        let previous_surface =
            canonical_surface_ready_chunks_for_focus(focus, &self.canonical_surface_ready_chunks);
        let next_surface = canonical_surface_ready_chunks_for_focus(focus, &surface_replacement);
        let previous_columns = canonical_ready_columns(&previous_surface);
        let next_columns = canonical_ready_columns(&next_surface);
        self.canonical_ready_chunks = canonical_replacement;
        self.canonical_surface_ready_chunks = surface_replacement;

        let changed_columns = previous_columns
            .symmetric_difference(&next_columns)
            .copied()
            .collect::<HashSet<_>>();
        for (key, mesh) in &mut self.chunks {
            if key.0 == 0 && changed_columns.contains(&(key.1, key.3)) {
                mesh.lod_ownership_stale = true;
            }
        }
        self.invalidate_lod_draw_plan(if changed_columns.is_empty() {
            LOD_PLAN_REBUILD_CANONICAL_VOLUME
        } else {
            LOD_PLAN_REBUILD_CANONICAL_COLUMNS | LOD_PLAN_REBUILD_CANONICAL_VOLUME
        });
    }

    /// Whether an exact-volume chunk is part of a LOD cut currently reaching the screen.
    ///
    /// Canonical surface bands and enclosed tunnel/cavern interest are independent exact-volume
    /// ownership reasons. During a short geometric handoff, either the incoming or outgoing
    /// complete cut remains presented. Reporting only the canonical-ready input set would
    /// incorrectly call a resident tunnel chunk unowned even while the renderer draws it.
    pub fn exact_volume_chunk_presented(&self, coord: ChunkCoord) -> bool {
        let coord = (coord.x, coord.y, coord.z);
        self.lod_draw_plan.owns_exact_volume_coord(coord)
            || self
                .cut_transition
                .as_ref()
                .is_some_and(|transition| transition.from.owns_exact_volume_coord(coord))
    }

    /// Whether a sparse exact surface column can replace a whole resident stride-two column in
    /// the current geometric cut. Callers use this before requesting detail chunks so an
    /// artificially narrow debug cut cannot create unused exact-volume traffic farther out.
    pub fn supports_sparse_exact_surface_column(&self, chunk_x: i32, chunk_z: i32) -> bool {
        self.lod_draw_plan_focus.is_some_and(|focus| {
            matches!(
                focus.owner_at(
                    chunk_x * CHUNK_EDGE as i32 + CHUNK_EDGE as i32 / 2,
                    chunk_z * CHUNK_EDGE as i32 + CHUNK_EDGE as i32 / 2,
                ),
                crate::lod::LodOwner::Canonical
                    | crate::lod::LodOwner::Surface(SurfaceLodLevel::Stride2)
            )
        })
    }

    /// Tiles referenced by the complete cut currently reaching the screen.
    ///
    /// Streaming eviction uses this as a transaction boundary: the current cut and its short
    /// outgoing transition remain resident until the renderer has selected their replacement.
    /// Removing one of these tiles first would invalidate the very fallback that is meant to hide
    /// asynchronous LOD arrival.
    pub fn presented_surface_tiles(&self) -> Vec<SurfaceTileCoord> {
        let mut tiles = self
            .lod_draw_plan
            .patches
            .owned_patches()
            .map(surface_tile_for_patch)
            .collect::<HashSet<_>>();
        if let Some(transition) = &self.cut_transition {
            tiles.extend(
                transition
                    .from
                    .patches
                    .owned_patches()
                    .map(surface_tile_for_patch),
            );
        }
        tiles.into_iter().collect()
    }

    /// Replaces the exact underground chunks selected through visible tunnel apertures.
    ///
    /// These chunks supplement the height-surface hierarchy in three dimensions. They deliberately
    /// do not claim the whole X/Z column, so the far terrain surface remains selected above them.
    pub fn set_enclosed_view_ready_chunks(
        &mut self,
        chunks: impl IntoIterator<Item = (i32, i32, i32)>,
    ) {
        let replacement = chunks.into_iter().collect::<HashSet<_>>();
        if replacement == self.enclosed_view_ready_chunks {
            return;
        }
        let changed = self
            .enclosed_view_ready_chunks
            .symmetric_difference(&replacement)
            .copied()
            .collect::<HashSet<_>>();
        self.enclosed_view_ready_chunks = replacement;
        for (x, y, z) in changed {
            if let Some(mesh) = self.chunks.get_mut(&(0, x, y, z)) {
                mesh.lod_ownership_stale = true;
            }
        }
        self.invalidate_lod_draw_plan(LOD_PLAN_REBUILD_ENCLOSED_VIEW);
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
                surface_patch_id: None,
                boundary_edge: None,
                stitch_edges: 0,
                morph_closure: false,
                exact_replacement_chunk: None,
                canonical_water_surface: false,
                render_layer: RenderLayer::Opaque,
            });
        }
        if gpu_quads.is_empty() {
            let existed = self.chunks.contains_key(&EXACT_VOLUME_FRONTIER_MESH_KEY);
            self.remove_opaque_mesh(EXACT_VOLUME_FRONTIER_MESH_KEY);
            return existed;
        }
        if gpu_quads_match_resident(
            self.chunks.get(&EXACT_VOLUME_FRONTIER_MESH_KEY),
            &gpu_quads,
            None,
        ) && mesh_slices_match_resident(
            self.chunks.get(&EXACT_VOLUME_FRONTIER_MESH_KEY),
            &slices,
            gpu_quads.len(),
        ) {
            return false;
        }
        let Some(prepared) =
            self.prepare_mesh_sliced(EXACT_VOLUME_FRONTIER_MESH_KEY, &gpu_quads, None, slices)
        else {
            return false;
        };
        commit_prepared_mesh(
            &mut self.arena,
            Some(&mut self.morph_arena),
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

    /// Colors visible geometry by its actual resident draw source and surface LOD level.
    ///
    /// This is deliberately a renderer diagnostic rather than a distance visualization: canonical
    /// voxel meshes, temporary frontier caps, streamed fallback walls, cross-LOD connectors, and
    /// every streamed level remain distinguishable even where their geometric ranges meet.
    pub fn set_geometry_source_debug(&mut self, active: bool) {
        self.geometry_source_debug = active;
        self.ui.set_geometry_sources_active(active);
    }

    /// Selects the material-detail pipeline for deterministic profiling without adding a
    /// developer-only control to the player-facing World Lab.
    pub fn set_material_detail_enabled(&mut self, enabled: bool) {
        self.options.material_detail = enabled;
    }

    /// Replaces the complete geometric ownership policy for controlled fidelity experiments.
    /// The currently presented plan stays resident until the next exact plan is built.
    pub fn set_lod_boundary_half_extents_voxels(&mut self, extents: [i32; 8]) -> bool {
        if !lod_boundary_half_extents_are_valid(extents) {
            return false;
        }
        if self.runtime_config.lod_boundary_half_extents_voxels == extents {
            return true;
        }
        self.runtime_config.lod_boundary_half_extents_voxels = extents;
        self.geometric_lod_focus = None;
        self.invalidate_lod_draw_plan(LOD_PLAN_REBUILD_FOCUS);
        true
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
        let lod_focus = self.geometric_lod_focus.map_or_else(
            || "null".to_owned(),
            |focus| {
                format!(
                    r#"{{"boundaryCentresVoxels":{:?},"boundaryHalfExtentsVoxels":{:?}}}"#,
                    focus.boundary_centres(),
                    focus.boundary_half_extents(),
                )
            },
        );
        let cut_transition = self.cut_transition.as_ref().map_or_else(
            || "null".to_owned(),
            |transition| {
                let phase = if CUT_TRANSITION_SECONDS > 0.0 {
                    ((self.time - transition.started_at) / CUT_TRANSITION_SECONDS).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                format!(r#"{{"active":true,"phase":{phase}}}"#)
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
        let legacy_cut_manifest = screenshot_cut_manifest_json(
            &self.lod_draw_plan,
            self.lod_draw_plan_focus,
            self.cut_transition.as_ref(),
        );
        let gpu_virtual_feedback = self.virtual_terrain_gpu.latest_feedback();
        let virtual_terrain_manifest = screenshot_virtual_terrain_manifest_json(
            self.virtual_terrain_mode,
            &self.virtual_terrain_pages,
            (self.virtual_terrain_mode == VirtualTerrainRenderMode::Visible)
                .then_some(self.virtual_terrain_cut.as_ref())
                .flatten(),
            self.virtual_terrain_oracle_cut.as_ref(),
            gpu_virtual_feedback.as_ref(),
        );
        let (cut_manifest, cut_fingerprint) =
            if self.virtual_terrain_mode == VirtualTerrainRenderMode::Visible {
                let cut = self.virtual_terrain_cut.as_ref();
                let virtual_fingerprint = cut.map_or(0, |cut| cut.fingerprint);
                (
                    format!(
                        r#"{{"kind":"spatialHybrid","legacy":{},"virtual":{}}}"#,
                        legacy_cut_manifest,
                        screenshot_virtual_cut_json(cut),
                    ),
                    fingerprint_value(
                        fingerprint_bytes(legacy_cut_manifest.as_bytes()),
                        virtual_fingerprint,
                    ),
                )
            } else {
                (
                    format!(r#"{{"kind":"legacy","cut":{legacy_cut_manifest}}}"#),
                    fingerprint_bytes(legacy_cut_manifest.as_bytes()),
                )
            };
        let inverse_view_projection = view_projection(
            &self.config,
            camera,
            self.runtime_config.view_distance_metres,
        )
        .inverse()
        .to_cols_array();
        let representation_kinds = if self.virtual_terrain_mode == VirtualTerrainRenderMode::Visible
        {
            r#"{"legacy":{"canonical":1,"steppedSurface":2,"rendererProduct":3},"virtual":{"steppedSurfaceResidual":17,"sparseVoxelBrick":18,"surfaceCluster":19,"triangleCluster":20}}"#
        } else {
            r#"{"canonical":1,"steppedSurface":2,"rendererProduct":3}"#
        };
        let attachment_manifest = format!(
            concat!(
                r#"{{"terrainPixelOwnership":{{"chunkType":"vpDI","#,
                r#""schema":"voxels.terrain-pixel-ownership.v1","compression":"deflate","#,
                r#""format":"u32x5","byteOrder":"little-endian","rowOrder":"top-down","#,
                r#""channels":["ownerIdHashLow","ownerIdHashHigh","primitiveFaceHash","packedRepresentationDepthFaceMaterial","reverseZDepthF32Bits"],"#,
                r#""backgroundOwnerId":["0","0"],"ownerHash":{{"algorithm":"fnv1a32+jenkins-oaat32","#,
                r#""words":["representationKind","hierarchyDepth","pageX","pageY","pageZ"],"#,
                r#""representationKind":{}}},"#,
                r#""descriptorBits":{{"representationSource":[0,4],"hierarchyDepth":[4,4],"face":[8,3],"material":[11,16]}},"#,
                r#""worldPositionReconstruction":{{"pixelCenter":true,"depthConvention":"reverse-z-webgpu","#,
                r#""inverseViewProjectionColumns":{:?}}}}}}}"#
            ),
            representation_kinds, inverse_view_projection,
        );
        format!(
            concat!(
                r#"{{"schema":"voxels.reproduction.v2","frameSequence":{},"runtime":{},"gpu":{},"image":{{"#,
                r#""pixelWidth":{},"pixelHeight":{},"cssWidth":{},"cssHeight":{},"devicePixelRatio":{}}},"#,
                r#""camera":{{"eyeMetres":{:?},"velocityMetresPerSecond":{:?},"yawRadians":{},"pitchRadians":{},"headingDegrees":{},"verticalFovRadians":{},"nearPlaneMetres":0.05,"farPlaneMetres":{},"grounded":{},"locomotion":"{}","fluid":{{"immersion":{},"eyeDepthMetres":{},"signedEyeDepthMetres":{},"surfaceYMetres":{},"surfaceKnown":{},"eyesSubmerged":{},"swimming":{}}}}},"#,
                r#""world":{},"environment":{{"serverTimeSeconds":{},"worldDays":{},"dayFraction":{},"yearFraction":{},"moonOrbitFraction":{},"twinklePhase":{},"planetCircumferenceMetres":{},"axialTiltRadians":{},"moonOrbitInclinationRadians":{},"celestialSeed":"{}","celestialRevision":"{}","weatherFraction":{},"weatherCycleSeconds":{},"cloudOffsetMetres":{:?},"cloudVelocityMetresPerSecond":{:?},"cloudCoverage":{},"cloudBaseMetres":{},"cloudTopMetres":{},"weatherSeed":"{}","weatherRevision":"{}","sunDirection":{:?},"moonDirection":{:?},"debugDayFraction":{},"debugWeatherFraction":{},"reproductionOverride":{},"surfaceRegion":{}}},"#,
                r#""presentation":{{"viewportFingerprint":"{:016x}","selectedCutFingerprint":"{:016x}","selectedCut":{},"virtualTerrain":{},"worldQuads":{},"waterQuads":{},"drawCalls":{},"waterDrawCalls":{},"lodTransitionQuads":{},"incompleteTransitionEdges":{},"lodCutTransitionActive":{},"lodCutTransitionPhase":{},"surfaceWidth":{},"surfaceHeight":{}}},"#,
                r#""streaming":{},"#,
                r#""attachments":{},"#,
                r#""render":{{"worldLabOpen":{},"features":{{"shadows":{},"voxelAmbientOcclusion":{},"screenSpaceAmbientOcclusion":{},"fog":{},"farTerrain":{},"water":{},"targetOutline":{},"materialDetail":{},"caveHeadlamp":{},"localLighting":{}}},"diagnosticSkyColor":{},"geometrySourceDebug":{},"viewDistanceMetres":{},"lodFocus":{},"cutTransition":{}}}}}"#
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
            cut_manifest,
            virtual_terrain_manifest,
            self.diagnostics.quads,
            self.diagnostics.water_quads,
            self.diagnostics.draw_calls,
            self.diagnostics.water_draw_calls,
            self.diagnostics.lod_transition_quads,
            self.diagnostics.lod_incomplete_transition_edges,
            self.diagnostics.lod_cut_transition_active,
            self.diagnostics.lod_cut_transition_phase,
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
            lod_focus,
            cut_transition,
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

    pub fn register_virtual_terrain_directory(
        &mut self,
        directory: &TerrainHierarchyDirectoryV1,
    ) -> Result<(), VirtualTerrainRendererError> {
        self.virtual_terrain.register_region_directory(directory)?;
        self.virtual_terrain_gpu
            .register_directory(&self.queue, directory)
            .map_err(|_| VirtualTerrainRendererError::GpuTraversal)?;
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
        let page_key = page.key;
        if let Some(existing) = self.virtual_terrain_pages.get(&page.key)
            && existing.revision == page.revision
            && existing.content_fingerprint == page.content_fingerprint
            && existing.representation == page.representation.kind()
        {
            self.virtual_terrain.install_page(page)?;
            self.virtual_terrain_gpu
                .update_page_residency(&self.queue, &self.virtual_terrain, page_key)
                .map_err(|_| VirtualTerrainRendererError::GpuTraversal)?;
            return Ok(());
        }
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
                            None,
                            &gpu_quads,
                            None,
                            slices,
                            u8::MAX,
                            "bounded virtual terrain page pool",
                        )
                        .ok_or(VirtualTerrainRendererError::GpuPoolCapacity)?,
                    )
                }
            }
            TerrainPageRepresentation::TriangleCluster(_) => {
                let vertices = virtual_triangle_gpu_vertices(&page)?;
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
        if allocation_page.is_some_and(|page| page != 0) {
            discard_virtual_terrain_mesh(&mut self.virtual_terrain_arena, mesh);
            return Err(VirtualTerrainRendererError::GpuPoolCapacity);
        }
        if allocation_page.is_some() {
            let Some(source) = self.virtual_terrain_arena_buffers.first() else {
                discard_virtual_terrain_mesh(&mut self.virtual_terrain_arena, mesh);
                return Err(VirtualTerrainRendererError::GpuPoolCapacity);
            };
            if self
                .virtual_terrain_gpu
                .bind_geometry_source(&self.device, source)
                .is_err()
            {
                discard_virtual_terrain_mesh(&mut self.virtual_terrain_arena, mesh);
                return Err(VirtualTerrainRendererError::GpuTraversal);
            }
        }
        if let Err(error) = self.virtual_terrain.install_page(page.clone()) {
            discard_virtual_terrain_mesh(&mut self.virtual_terrain_arena, mesh);
            return Err(error.into());
        }
        if self
            .virtual_terrain_gpu
            .update_page_geometry(&self.queue, page.key, geometry)
            .and_then(|()| {
                self.virtual_terrain_gpu.update_page_residency(
                    &self.queue,
                    &self.virtual_terrain,
                    page.key,
                )
            })
            .is_err()
        {
            discard_virtual_terrain_mesh(&mut self.virtual_terrain_arena, mesh);
            return Err(VirtualTerrainRendererError::GpuTraversal);
        }
        let resident = VirtualTerrainGpuPage {
            revision: page.revision,
            content_fingerprint: page.content_fingerprint,
            representation: page.representation.kind(),
            mesh,
        };
        if let Some(old) = self.virtual_terrain_pages.insert(page.key, resident) {
            discard_virtual_terrain_mesh(&mut self.virtual_terrain_arena, old.mesh);
        }
        Ok(())
    }

    pub fn select_virtual_terrain_cut(
        &mut self,
        view: VirtualTerrainView,
    ) -> Result<VirtualTerrainCut, VirtualTerrainRendererError> {
        self.virtual_terrain_gpu
            .synchronize_prior_refinement(&self.queue, &self.virtual_terrain)
            .map_err(|_| VirtualTerrainRendererError::GpuTraversal)?;
        let cut = self.virtual_terrain.select_cut(view)?;
        self.virtual_terrain_oracle_view = Some(view);
        self.virtual_terrain_oracle_cut = Some(cut.clone());
        let renderable =
            cut.is_renderable() && self.virtual_terrain_cut_fits_compaction(&cut).is_ok();
        let preserves_visible_cut = self.virtual_terrain_mode == VirtualTerrainRenderMode::Visible
            && renderable
            && self
                .virtual_terrain_cut
                .as_ref()
                .is_some_and(|visible| visible.fingerprint == cut.fingerprint);
        if self.virtual_terrain_mode != VirtualTerrainRenderMode::Visible || preserves_visible_cut {
            self.virtual_terrain_cut = Some(cut.clone());
        } else {
            // A changed candidate must traverse, compact, and round-trip through the bounded GPU
            // feedback oracle before it can replace the currently visible owner. Shadow mode
            // keeps the legacy path visible while that candidate is being certified.
            self.virtual_terrain_mode = VirtualTerrainRenderMode::Shadow;
            self.virtual_terrain_cut = Some(cut.clone());
        }
        Ok(cut)
    }

    pub fn virtual_terrain_cut(&self) -> Option<&VirtualTerrainCut> {
        self.virtual_terrain_cut.as_ref()
    }

    pub fn set_virtual_terrain_render_mode(
        &mut self,
        mode: VirtualTerrainRenderMode,
    ) -> Result<(), VirtualTerrainRendererError> {
        if mode == VirtualTerrainRenderMode::Visible {
            let Some(cut) = self
                .virtual_terrain_cut
                .as_ref()
                .filter(|cut| cut.is_renderable())
            else {
                return Err(VirtualTerrainRendererError::NoRenderableCut);
            };
            if let Some(missing) = cut
                .selected_pages
                .iter()
                .find(|key| !self.virtual_terrain_pages.contains_key(key))
            {
                return Err(VirtualTerrainRendererError::SelectedPageMissingGpu(
                    *missing,
                ));
            }
            let ownership = VirtualTerrainOwnership::from_cut(cut)?;
            self.validate_virtual_terrain_handoff(&ownership)?;
            self.virtual_terrain_cut_fits_compaction(cut)?;
            let certified = self
                .virtual_terrain_gpu
                .latest_feedback()
                .is_some_and(|feedback| gpu_feedback_matches_cut(&feedback, Some(cut)));
            if !certified {
                return Err(VirtualTerrainRendererError::GpuCutNotCertified);
            }
        }
        self.virtual_terrain_mode = mode;
        Ok(())
    }

    fn validate_virtual_terrain_handoff(
        &self,
        ownership: &VirtualTerrainOwnership,
    ) -> Result<(), VirtualTerrainRendererError> {
        let focus = active_geometric_lod_focus(self.geometric_lod_focus, self.options.far_terrain);
        let lod_draw_plan = focus.is_some().then_some(&self.lod_draw_plan);
        for chunks in [&self.chunks, &self.water_chunks] {
            for (key, chunk) in chunks {
                if !chunk.active()
                    || (key.0 != 0
                        && *key != EXACT_VOLUME_FRONTIER_MESH_KEY
                        && !self.options.far_terrain)
                {
                    continue;
                }
                for slice in &chunk.slices {
                    if !slice_owned_by_lod(focus, lod_draw_plan, key, slice)
                        || !ownership.intersects_aabb(slice.bounds_min, slice.bounds_max)
                    {
                        continue;
                    }
                    if !ownership.covers_aabb(slice.bounds_min, slice.bounds_max) {
                        return Err(VirtualTerrainRendererError::LegacyOwnerCrossesVirtualBoundary);
                    }
                }
            }
        }
        Ok(())
    }

    pub const fn virtual_terrain_render_mode(&self) -> VirtualTerrainRenderMode {
        self.virtual_terrain_mode
    }

    pub fn virtual_terrain_region_roots(&self) -> Vec<TerrainPageKey> {
        self.virtual_terrain.roots().collect()
    }

    /// Retires immutable region directories outside the current streaming working set.
    ///
    /// Any directory compaction invalidates GPU node indices, so publication drops to shadow mode,
    /// resident geometry records are rebound under their new indices, and a later certified cut
    /// performs the next visible handoff.
    pub fn retain_virtual_terrain_regions(
        &mut self,
        keep: impl IntoIterator<Item = TerrainPageKey>,
    ) -> Result<usize, VirtualTerrainRendererError> {
        let keep = keep.into_iter().collect::<BTreeSet<_>>();
        let remove = self
            .virtual_terrain
            .roots()
            .filter(|root| !keep.contains(root))
            .collect::<Vec<_>>();
        if remove.is_empty() {
            return Ok(0);
        }
        self.virtual_terrain_mode = VirtualTerrainRenderMode::Shadow;
        self.virtual_terrain_cut = None;
        self.virtual_terrain_oracle_cut = None;
        self.virtual_terrain_oracle_view = None;
        let mut removed_pages = BTreeSet::new();
        for root in remove {
            removed_pages.extend(self.virtual_terrain.remove_region_directory(root));
        }
        for key in &removed_pages {
            if let Some(page) = self.virtual_terrain_pages.remove(key) {
                discard_virtual_terrain_mesh(&mut self.virtual_terrain_arena, page.mesh);
            }
        }
        self.virtual_terrain_gpu
            .synchronize_directory_set(&self.queue, &self.virtual_terrain)
            .map_err(|_| VirtualTerrainRendererError::GpuTraversal)?;
        for (key, page) in &self.virtual_terrain_pages {
            self.virtual_terrain_gpu
                .update_page_geometry(&self.queue, *key, virtual_terrain_gpu_geometry(&page.mesh))
                .map_err(|_| VirtualTerrainRendererError::GpuTraversal)?;
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
        self.virtual_terrain_mode = VirtualTerrainRenderMode::Shadow;
        self.virtual_terrain_cut = None;
        self.virtual_terrain_oracle_cut = None;
        self.virtual_terrain_oracle_view = None;
        if let Some(page) = self.virtual_terrain_pages.remove(&key) {
            discard_virtual_terrain_mesh(&mut self.virtual_terrain_arena, page.mesh);
        }
        self.virtual_terrain_gpu
            .update_page_geometry(&self.queue, key, VirtualTerrainGpuGeometry::default())
            .and_then(|()| {
                self.virtual_terrain_gpu.update_page_residency(
                    &self.queue,
                    &self.virtual_terrain,
                    key,
                )
            })
            .map_err(|_| VirtualTerrainRendererError::GpuTraversal)?;
        Ok(true)
    }

    /// Resident page count, encoded CPU bytes, primitive count, GPU capacity, and GPU allocation.
    pub fn virtual_terrain_usage(&self) -> (usize, usize, usize, u64, u64) {
        let (pages, encoded_bytes, primitives) = self.virtual_terrain.resident_usage();
        let gpu = self.virtual_terrain_arena.stats();
        let compact_capacity = VIRTUAL_TERRAIN_COMPACT_SURFACE_BYTES
            .saturating_add(VIRTUAL_TERRAIN_COMPACT_TRIANGLE_BYTES)
            .saturating_add(VIRTUAL_TERRAIN_COMPACT_WATER_SURFACE_BYTES)
            .saturating_add(VIRTUAL_TERRAIN_COMPACT_WATER_TRIANGLE_BYTES);
        let compact_allocated =
            self.virtual_terrain_gpu
                .latest_feedback()
                .map_or(0, |feedback| {
                    u64::from(feedback.compacted_surface_elements)
                        .saturating_mul(size_of::<GpuQuad>() as u64)
                        .saturating_add(
                            u64::from(feedback.compacted_triangle_elements)
                                .saturating_mul(size_of::<GpuTerrainVertex>() as u64),
                        )
                        .saturating_add(
                            u64::from(feedback.compacted_water_surface_elements)
                                .saturating_mul(size_of::<GpuQuad>() as u64),
                        )
                        .saturating_add(
                            u64::from(feedback.compacted_water_triangle_elements)
                                .saturating_mul(size_of::<GpuTerrainVertex>() as u64),
                        )
                });
        (
            pages,
            encoded_bytes,
            primitives,
            gpu.capacity_bytes.saturating_add(compact_capacity),
            gpu.allocated_bytes.saturating_add(compact_allocated),
        )
    }

    fn virtual_terrain_cut_fits_compaction(
        &self,
        cut: &VirtualTerrainCut,
    ) -> Result<(), VirtualTerrainRendererError> {
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
        if surface_bytes > VIRTUAL_TERRAIN_COMPACT_SURFACE_BYTES
            || triangle_bytes > VIRTUAL_TERRAIN_COMPACT_TRIANGLE_BYTES
            || water_surface_bytes > VIRTUAL_TERRAIN_COMPACT_WATER_SURFACE_BYTES
            || water_triangle_bytes > VIRTUAL_TERRAIN_COMPACT_WATER_TRIANGLE_BYTES
        {
            return Err(VirtualTerrainRendererError::SelectedCutCompactionCapacity);
        }
        Ok(())
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
        let surface_profile = canonical_chunk_profile(chunk);
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
                None,
                vec![MeshSlice {
                    relative_offset: 0,
                    size: opaque_count * quad_bytes,
                    quad_count: opaque_count,
                    bounds_min: min,
                    bounds_max: max,
                    surface_patch_id: None,
                    boundary_edge: None,
                    stitch_edges: 0,
                    morph_closure: false,
                    exact_replacement_chunk: None,
                    canonical_water_surface: false,
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
                    surface_patch_id: None,
                    boundary_edge: None,
                    stitch_edges: 0,
                    morph_closure: false,
                    exact_replacement_chunk: None,
                    canonical_water_surface: true,
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
                    surface_patch_id: None,
                    boundary_edge: None,
                    stitch_edges: 0,
                    morph_closure: false,
                    exact_replacement_chunk: None,
                    canonical_water_surface: false,
                    render_layer: RenderLayer::Translucent,
                });
            }
            let Some(prepared) = self.prepare_water_mesh_sliced(key, &water_quads, slices) else {
                discard_prepared_mesh(&mut self.arena, Some(&mut self.morph_arena), opaque_update);
                return None;
            };
            Some(prepared)
        };
        Some(PreparedCanonicalChunkUpload {
            coord,
            key,
            surface_profile,
            opaque: opaque_update,
            translucent: water_update,
            local_lights: local_lights_for_mesh(origin, mesh),
        })
    }

    fn discard_canonical_chunk_upload(&mut self, upload: PreparedCanonicalChunkUpload) {
        discard_prepared_mesh(&mut self.arena, Some(&mut self.morph_arena), upload.opaque);
        discard_prepared_mesh(&mut self.water_arena, None, upload.translucent);
    }

    fn commit_canonical_chunk_upload(&mut self, upload: PreparedCanonicalChunkUpload) {
        commit_prepared_mesh(
            &mut self.arena,
            Some(&mut self.morph_arena),
            &mut self.chunks,
            upload.key,
            upload.opaque,
        );
        commit_prepared_mesh(
            &mut self.water_arena,
            None,
            &mut self.water_chunks,
            upload.key,
            upload.translucent,
        );
        self.replace_canonical_surface_profile(upload.coord, upload.surface_profile);
        if upload.local_lights.is_empty() {
            self.local_light_candidates.remove(&upload.key);
        } else {
            self.local_light_candidates
                .insert(upload.key, upload.local_lights);
        }
    }

    pub fn upload_surface_tile_meshes(
        &mut self,
        tile: &SurfaceTileMesh,
        water: &WaterTileMesh,
    ) -> bool {
        let coord = tile.coord;
        if water.coord != coord {
            return false;
        }
        let key = (coord.level.index() + 1, coord.x, 0, coord.z);
        if tile.quads.is_empty() && water.quads.is_empty() {
            self.remove_surface_tile(coord);
            return true;
        }
        let resident_patch_ids = tile
            .patches
            .iter()
            .filter_map(|patch| {
                SurfacePatchId::from_tile_cell_min(
                    coord,
                    [patch.cell_bounds[0][0], patch.cell_bounds[0][1]],
                )
            })
            .collect::<HashSet<_>>();
        let (macro_normals, geometry_shapes) = surface_macro_normals_and_shapes(tile);
        let horizon_profiles = surface_horizon_profiles(tile);
        let geometry_morphs = surface_geometry_morphs(tile, &macro_normals, &geometry_shapes);
        let encoded_gpu_quads: Vec<_> = tile
            .quads
            .iter()
            .zip(macro_normals.iter().copied())
            .zip(horizon_profiles.iter().copied())
            .zip(geometry_shapes.iter().copied())
            .map(
                |(((quad, macro_normal), horizon_profile), surface_shape)| GpuQuad {
                    origin: quad.origin,
                    extent_voxels: quad.extent,
                    material_face: pack_gpu_source_material(
                        pack_surface_horizon_material(
                            pack_gpu_material_face(
                                u32::from(quad.material.id())
                                    | FAR_MATERIAL_FLAG
                                    | (u32::from(coord.level.index()) << SURFACE_LOD_SHIFT),
                                quad.face,
                            ),
                            horizon_profile,
                        ) | (u32::from(surface_shape & 0xff) << SURFACE_SHAPE_MATERIAL_SHIFT),
                        if quad.synthetic_fallback {
                            GPU_SOURCE_SURFACE_FALLBACK
                        } else if macro_normal & SURFACE_MACRO_NORMAL_FLAG == 0 {
                            GPU_SOURCE_SKYLINE_PROXY
                        } else {
                            0
                        },
                    ),
                    ao: pack_surface_horizon_ao(
                        macro_normal | (u32::from(surface_shape >> 8) << SURFACE_SHAPE_AO_SHIFT),
                        horizon_profile,
                    ),
                },
            )
            .collect();
        let (constrained_gpu_quads, constrained_gpu_morphs) =
            if coord.level == SurfaceLodLevel::Stride2 {
                let mut owners = Vec::new();
                let mut pieces = Vec::new();
                let mut piece_morphs = Vec::new();
                for (owner, &quad) in encoded_gpu_quads.iter().enumerate() {
                    let split = split_gpu_quad_vertical_extent(quad, 63);
                    for piece in split {
                        owners.push(owner);
                        pieces.push(piece);
                        piece_morphs.push(split_surface_morph(quad, piece, geometry_morphs[owner]));
                    }
                }
                let constrained_pieces = constrain_gpu_quad_t_junctions(
                    &pieces,
                    |_, quad| {
                        let surface_shape = ((quad.material_face >> SURFACE_SHAPE_MATERIAL_SHIFT)
                            & 0xff)
                            | (((quad.ao >> SURFACE_SHAPE_AO_SHIFT) & 0x0f) << 8);
                        surface_shape == 0
                            && quad.extent_voxels[0] <= 63
                            && quad.extent_voxels[1] <= 63
                            && quad.extent_voxels.into_iter().all(|extent| extent > 0)
                    },
                    |_, _, start, end| {
                        let [tile_min_x, tile_min_z] = coord.voxel_origin();
                        let tile_max_x = tile_min_x.saturating_add(coord.voxel_span());
                        let tile_max_z = tile_min_z.saturating_add(coord.voxel_span());
                        start[0] == end[0]
                            && start[2] == end[2]
                            && (start[0] == tile_min_x
                                || start[0] == tile_max_x
                                || start[2] == tile_min_z
                                || start[2] == tile_max_z)
                    },
                    true,
                );
                let mut constrained = vec![Vec::new(); encoded_gpu_quads.len()];
                let mut constrained_morphs = vec![Vec::new(); encoded_gpu_quads.len()];
                for ((owner, morph), triangles) in
                    owners.into_iter().zip(piece_morphs).zip(constrained_pieces)
                {
                    // Canonical transition triangles retain the source piece's four corner
                    // deltas; the shader evaluates that piecewise-linear field at inserted
                    // vertices. Adjacent tall-wall pieces share the same rounded half-voxel
                    // endpoint, so their artificial partition cannot open a crack.
                    constrained_morphs[owner].extend(std::iter::repeat_n(morph, triangles.len()));
                    constrained[owner].extend(triangles);
                }
                (constrained, constrained_morphs)
            } else {
                (
                    encoded_gpu_quads
                        .iter()
                        .copied()
                        .map(|quad| vec![quad])
                        .collect(),
                    geometry_morphs
                        .iter()
                        .copied()
                        .map(|morph| vec![morph])
                        .collect(),
                )
            };
        let patch_profiles =
            surface_patch_profiles(tile, &macro_normals, &horizon_profiles, &geometry_shapes);
        let exact_replacement_chunks = tile
            .quads
            .iter()
            .map(surface_exact_replacement_chunk)
            .collect::<Vec<_>>();
        let closure_gpu = surface_morph_closure_gpu_quads(tile, &macro_normals, &horizon_profiles);
        let closure_exact_replacement_chunks = tile
            .morph_closures
            .iter()
            .map(|closure| surface_exact_replacement_chunk(&closure.quad))
            .collect::<Vec<_>>();
        let water_gpu_quads: Vec<_> = water
            .quads
            .iter()
            .map(|quad| GpuQuad {
                origin: quad.origin,
                extent_voxels: quad.extent,
                material_face: pack_gpu_source_material(
                    pack_gpu_material_face(
                        u32::from(quad.material.id())
                            | (u32::from(coord.level.index()) << SURFACE_LOD_SHIFT),
                        quad.face,
                    ),
                    GPU_SOURCE_WATER,
                ),
                ao: 0xff,
            })
            .collect();
        let quad_bytes = size_of::<GpuQuad>() as u32;
        let mut gpu_quads =
            Vec::with_capacity(encoded_gpu_quads.len().saturating_add(closure_gpu.len()));
        let mut gpu_morph_heights = Vec::with_capacity(gpu_quads.capacity());
        let mut slices = Vec::new();
        for patch in &tile.patches {
            let Some(patch_id) = SurfacePatchId::from_tile_cell_min(
                coord,
                [patch.cell_bounds[0][0], patch.cell_bounds[0][1]],
            ) else {
                continue;
            };
            let (bounds_min, bounds_max) = surface_patch_render_bounds(patch, coord.level);
            // Reorder each patch into a handful of ownership groups. Ordinary patch geometry has
            // no edge tag; terrain top cells touching a patch boundary carry all touched edges;
            // generated source-edge walls retain their single edge. This lets an exact stitch
            // replace one coarse top exactly once, including at patch corners, without thousands
            // of per-cell draw slices or any overlapping repair geometry.
            let mut groups = BTreeMap::<(u8, u8, Option<(i32, i32, i32)>), Vec<usize>>::new();
            for quad_index in patch.quad_range.clone() {
                let index = quad_index as usize;
                let stitch_edges =
                    surface_top_stitch_edges(tile, patch, tile.quads[index], macro_normals[index]);
                groups
                    .entry((stitch_edges, 0, exact_replacement_chunks[index]))
                    .or_default()
                    .push(index);
            }
            for edge in SurfacePatchEdge::ALL {
                for quad_index in patch.edge_ranges[edge.index()].clone() {
                    let index = quad_index as usize;
                    groups
                        .entry((0, edge.index() as u8 + 1, exact_replacement_chunks[index]))
                        .or_default()
                        .push(index);
                }
            }
            for ((stitch_edges, encoded_edge, exact_replacement_chunk), source_indices) in groups {
                let start = gpu_quads.len() as u32;
                gpu_quads.extend(
                    source_indices
                        .iter()
                        .flat_map(|&index| constrained_gpu_quads[index].iter().copied()),
                );
                gpu_morph_heights.extend(
                    source_indices
                        .iter()
                        .flat_map(|&index| constrained_gpu_morphs[index].iter().copied()),
                );
                let end = gpu_quads.len() as u32;
                slices.push(MeshSlice {
                    relative_offset: start * quad_bytes,
                    size: (end - start) * quad_bytes,
                    quad_count: end - start,
                    bounds_min,
                    bounds_max,
                    surface_patch_id: Some(patch_id),
                    boundary_edge: encoded_edge
                        .checked_sub(1)
                        .and_then(|index| SurfacePatchEdge::ALL.get(index as usize).copied()),
                    stitch_edges,
                    morph_closure: false,
                    exact_replacement_chunk,
                    canonical_water_surface: false,
                    render_layer: RenderLayer::Opaque,
                });
            }
        }
        // Keep dormant topology closures after all ordinary patches. Interleaving them patch by
        // patch leaves an unselected allocation gap after nearly every fixed slice and prevents
        // the base terrain stream from coalescing. The active morph band still selects the exact
        // closure slices from this compact tail.
        for patch in &tile.patches {
            let Some(patch_id) = SurfacePatchId::from_tile_cell_min(
                coord,
                [patch.cell_bounds[0][0], patch.cell_bounds[0][1]],
            ) else {
                continue;
            };
            let (bounds_min, bounds_max) = surface_patch_render_bounds(patch, coord.level);
            let closure_groups = std::iter::once((0_u8, patch.morph_closure_range.clone())).chain(
                SurfacePatchEdge::ALL.into_iter().map(|edge| {
                    (
                        edge.index() as u8 + 1,
                        patch.edge_morph_closure_ranges[edge.index()].clone(),
                    )
                }),
            );
            for (encoded_edge, range) in closure_groups {
                let mut groups = BTreeMap::<Option<(i32, i32, i32)>, Vec<usize>>::new();
                for closure_index in range {
                    let index = closure_index as usize;
                    groups
                        .entry(closure_exact_replacement_chunks[index])
                        .or_default()
                        .push(index);
                }
                for (exact_replacement_chunk, source_indices) in groups {
                    let start = gpu_quads.len() as u32;
                    for index in source_indices {
                        let (quad, morph) = closure_gpu[index];
                        gpu_quads.push(quad);
                        gpu_morph_heights.push(morph);
                    }
                    let end = gpu_quads.len() as u32;
                    slices.push(MeshSlice {
                        relative_offset: start * quad_bytes,
                        size: (end - start) * quad_bytes,
                        quad_count: end - start,
                        bounds_min,
                        bounds_max,
                        surface_patch_id: Some(patch_id),
                        boundary_edge: encoded_edge
                            .checked_sub(1)
                            .and_then(|index| SurfacePatchEdge::ALL.get(index as usize).copied()),
                        stitch_edges: 0,
                        morph_closure: true,
                        exact_replacement_chunk,
                        canonical_water_surface: false,
                        render_layer: RenderLayer::Opaque,
                    });
                }
            }
        }
        let water_slices: Vec<_> = water
            .patches
            .iter()
            .map(|patch| {
                let patch_id = SurfacePatchId::from_tile_cell_min(
                    coord,
                    [patch.cell_bounds[0][0], patch.cell_bounds[0][1]],
                );
                MeshSlice {
                    relative_offset: patch.quad_range.start * quad_bytes,
                    size: (patch.quad_range.end - patch.quad_range.start) * quad_bytes,
                    quad_count: patch.quad_range.end - patch.quad_range.start,
                    bounds_min: glam::Vec3::from_array(
                        patch
                            .bounds
                            .min
                            .map(|value| value as f32 * VOXEL_SIZE_METRES),
                    ),
                    bounds_max: glam::Vec3::from_array(
                        patch
                            .bounds
                            .max
                            .map(|value| value as f32 * VOXEL_SIZE_METRES),
                    ),
                    surface_patch_id: patch_id,
                    boundary_edge: None,
                    stitch_edges: 0,
                    morph_closure: false,
                    exact_replacement_chunk: None,
                    canonical_water_surface: false,
                    render_layer: RenderLayer::Translucent,
                }
            })
            .collect();
        debug_assert_eq!(gpu_quads.len(), gpu_morph_heights.len());
        if gpu_quads_match_resident(self.chunks.get(&key), &gpu_quads, Some(&gpu_morph_heights))
            && mesh_slices_match_resident(self.chunks.get(&key), &slices, gpu_quads.len())
            && gpu_quads_match_resident(self.water_chunks.get(&key), &water_gpu_quads, None)
            && mesh_slices_match_resident(
                self.water_chunks.get(&key),
                &water_slices,
                water_gpu_quads.len(),
            )
        {
            // Underground edits commonly dirty the enclosing stride-two transport tile without
            // changing its GPU geometry or ownership metadata. Preserve the exact resident
            // products and LOD plan instead of reallocating identical bytes and slices.
            return true;
        }
        let opaque_update = if gpu_quads.is_empty() {
            None
        } else {
            let Some(prepared) =
                self.prepare_mesh_sliced(key, &gpu_quads, Some(&gpu_morph_heights), slices)
            else {
                return false;
            };
            Some(prepared)
        };
        let water_update = if water_gpu_quads.is_empty() {
            None
        } else {
            let Some(prepared) =
                self.prepare_water_mesh_sliced(key, &water_gpu_quads, water_slices)
            else {
                discard_prepared_mesh(&mut self.arena, Some(&mut self.morph_arena), opaque_update);
                return false;
            };
            Some(prepared)
        };
        commit_prepared_mesh(
            &mut self.arena,
            Some(&mut self.morph_arena),
            &mut self.chunks,
            key,
            opaque_update,
        );
        commit_prepared_mesh(
            &mut self.water_arena,
            None,
            &mut self.water_chunks,
            key,
            water_update,
        );
        let changed_profiles =
            changed_surface_patch_profiles(coord, &self.surface_patch_profiles, &patch_profiles);
        let profiles_affect_active_transition =
            self.surface_profiles_affect_active_transition(&changed_profiles);
        self.surface_patch_profiles
            .retain(|patch, _| !surface_patch_belongs_to_tile(*patch, coord));
        self.surface_patch_profiles.extend(patch_profiles);
        self.replace_surface_patch_residency(coord, resident_patch_ids);
        if profiles_affect_active_transition {
            self.invalidate_lod_draw_plan(LOD_PLAN_REBUILD_SURFACE_PROFILE);
        }
        true
    }

    fn prepare_mesh_sliced(
        &mut self,
        key: MeshKey,
        gpu_quads: &[GpuQuad],
        morph_heights: Option<&[GpuMorph]>,
        slices: Vec<MeshSlice>,
    ) -> Option<ChunkMesh> {
        let activation_mask = self.chunk_activations.upload_mask(key);
        prepare_mesh_sliced_into(
            &self.device,
            &self.queue,
            &mut self.arena,
            &mut self.arena_buffers,
            Some((&mut self.morph_arena, &mut self.morph_arena_buffers)),
            gpu_quads,
            morph_heights,
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
            None,
            gpu_quads,
            None,
            slices,
            activation_mask,
            "water mesh arena page",
        )
    }

    pub fn remove_chunk(&mut self, coord: ChunkCoord) {
        let key = (0, coord.x, coord.y, coord.z);
        self.remove_canonical_surface_profile(coord);
        self.remove_chunk_mesh(key);
        self.chunk_activations.remove(key);
    }

    pub fn remove_surface_tile(&mut self, coord: SurfaceTileCoord) {
        self.remove_surface_tiles([coord]);
    }

    pub fn remove_surface_tiles(&mut self, coords: impl IntoIterator<Item = SurfaceTileCoord>) {
        let coords = coords.into_iter().collect::<HashSet<_>>();
        if coords.is_empty() {
            return;
        }
        for coord in &coords {
            self.remove_mesh((coord.level.index() + 1, coord.x, 0, coord.z));
        }
        self.surface_patch_profiles
            .retain(|patch, _| !coords.contains(&surface_tile_for_patch(*patch)));
        let previous_len = self.surface_patch_residency.len();
        self.surface_patch_residency
            .retain(|patch| !coords.contains(&surface_tile_for_patch(*patch)));
        if self.surface_patch_residency.len() != previous_len {
            self.surface_incomplete_parents =
                incomplete_resident_parents(&self.surface_patch_residency);
            self.invalidate_lod_draw_plan(LOD_PLAN_REBUILD_SURFACE_RESIDENCY);
        }
    }

    fn remove_canonical_surface_profile(&mut self, coord: ChunkCoord) {
        let affects_active_transition = self.canonical_profile_affects_active_transition(coord);
        let column = (coord.x, coord.z);
        let mut remove_column = false;
        let mut resolved_profile_changed = false;
        if let Some(profiles) = self.canonical_surface_profiles.get_mut(&column) {
            let previous_resolved =
                affects_active_transition.then(|| resolved_canonical_column_profile(profiles));
            if profiles.remove(&coord.y).is_some() {
                resolved_profile_changed = previous_resolved.is_some_and(|previous| {
                    previous != resolved_canonical_column_profile(profiles)
                });
            }
            remove_column = profiles.is_empty();
        }
        if remove_column {
            self.canonical_surface_profiles.remove(&column);
        }
        if resolved_profile_changed {
            self.invalidate_lod_draw_plan(LOD_PLAN_REBUILD_CANONICAL_PROFILE);
        }
    }

    fn invalidate_lod_plan_for_canonical_profile(&mut self, coord: ChunkCoord) {
        if self.canonical_profile_affects_active_transition(coord) {
            self.invalidate_lod_draw_plan(LOD_PLAN_REBUILD_CANONICAL_PROFILE);
        }
    }

    fn replace_canonical_surface_profile(
        &mut self,
        coord: ChunkCoord,
        replacement: CanonicalChunkProfile,
    ) {
        let affects_active_transition = self.canonical_profile_affects_active_transition(coord);
        let resolved_profile_changed = {
            let profiles = self
                .canonical_surface_profiles
                .entry((coord.x, coord.z))
                .or_default();
            if profiles.get(&coord.y) == Some(&replacement) {
                return;
            }
            let previous_resolved =
                affects_active_transition.then(|| resolved_canonical_column_profile(profiles));
            profiles.insert(coord.y, replacement);
            previous_resolved
                .is_some_and(|previous| previous != resolved_canonical_column_profile(profiles))
        };
        if resolved_profile_changed {
            self.invalidate_lod_plan_for_canonical_profile(coord);
        }
    }

    fn canonical_profile_affects_active_transition(&self, coord: ChunkCoord) -> bool {
        self.lod_draw_plan
            .patches
            .transition_candidates()
            .any(|(patch, edge)| {
                patch.level == SurfaceLodLevel::Stride2
                    && canonical_column_touches_patch_edge((coord.x, coord.z), patch, edge)
            })
    }

    fn surface_profiles_affect_active_transition(
        &self,
        changed_profiles: &HashSet<SurfacePatchId>,
    ) -> bool {
        surface_profiles_affect_transition(&self.lod_draw_plan.patches, changed_profiles)
    }

    fn replace_surface_patch_residency(
        &mut self,
        coord: SurfaceTileCoord,
        replacement: HashSet<SurfacePatchId>,
    ) {
        let current = self
            .surface_patch_residency
            .iter()
            .copied()
            .filter(|patch| surface_patch_belongs_to_tile(*patch, coord))
            .collect::<HashSet<_>>();
        if current == replacement {
            return;
        }
        self.surface_patch_residency
            .retain(|patch| !surface_patch_belongs_to_tile(*patch, coord));
        self.surface_patch_residency.extend(replacement);
        self.surface_incomplete_parents =
            incomplete_resident_parents(&self.surface_patch_residency);
        self.invalidate_lod_draw_plan(LOD_PLAN_REBUILD_SURFACE_RESIDENCY);
    }

    fn invalidate_lod_draw_plan(&mut self, reason: u32) {
        self.surface_patch_residency_revision =
            self.surface_patch_residency_revision.wrapping_add(1);
        self.lod_draw_plan_dirty_reasons |= reason;
    }

    fn update_exact_lod_membership(&mut self, canonical_chunks: HashSet<(i32, i32, i32)>) {
        let enclosed_view_chunks = self.enclosed_view_ready_chunks.clone();
        self.mark_exact_replacement_ownership_stale(&canonical_chunks, &enclosed_view_chunks);
        for &(x, y, z) in self
            .lod_draw_plan
            .canonical_chunks
            .symmetric_difference(&canonical_chunks)
        {
            if let Some(mesh) = self.chunks.get_mut(&(0, x, y, z)) {
                mesh.lod_ownership_stale = true;
            }
        }
        self.lod_draw_plan.canonical_chunks = canonical_chunks;
        self.lod_draw_plan.enclosed_view_chunks = enclosed_view_chunks;
    }

    fn mark_exact_replacement_ownership_stale(
        &mut self,
        canonical_chunks: &HashSet<(i32, i32, i32)>,
        enclosed_view_chunks: &HashSet<(i32, i32, i32)>,
    ) {
        let changed = self
            .lod_draw_plan
            .canonical_chunks
            .symmetric_difference(canonical_chunks)
            .chain(
                self.lod_draw_plan
                    .enclosed_view_chunks
                    .symmetric_difference(enclosed_view_chunks),
            )
            .copied()
            .collect::<HashSet<_>>();
        if changed.is_empty() {
            return;
        }
        for (key, mesh) in &mut self.chunks {
            if key.0 == SurfaceLodLevel::Stride2.index() + 1
                && mesh.slices.iter().any(|slice| {
                    slice
                        .exact_replacement_chunk
                        .is_some_and(|coord| changed.contains(&coord))
                })
            {
                mesh.lod_ownership_stale = true;
            }
        }
    }

    fn refresh_lod_draw_plan(&mut self, focus: Option<GeometricLodFocus>) -> u32 {
        if self.lod_draw_plan_focus == focus
            && self.lod_draw_plan_revision == self.surface_patch_residency_revision
        {
            return 0;
        }
        let rebuild_reason = self.lod_draw_plan_dirty_reasons
            | if self.lod_draw_plan_focus != focus {
                LOD_PLAN_REBUILD_FOCUS
            } else {
                0
            };
        let mut canonical_chunks =
            canonical_ready_chunks_for_focus(focus, &self.canonical_ready_chunks);
        let canonical_surface_chunks =
            canonical_surface_ready_chunks_for_focus(focus, &self.canonical_surface_ready_chunks);
        canonical_chunks.extend(canonical_surface_chunks.iter().copied());
        let canonical_columns = canonical_surface_chunks
            .iter()
            .map(|&(x, _, z)| (x, z))
            .collect::<HashSet<_>>();
        let synchronous_surface_reasons = LOD_PLAN_REBUILD_FOCUS
            | LOD_PLAN_REBUILD_CANONICAL_PROFILE
            | LOD_PLAN_REBUILD_SURFACE_RESIDENCY
            | LOD_PLAN_REBUILD_SURFACE_PROFILE;
        let canonical_columns_changed = canonical_columns != self.lod_draw_plan.canonical_columns;
        if rebuild_reason & synchronous_surface_reasons == 0 && !canonical_columns_changed {
            // Exact-volume chunks supplement the unchanged surface cut. Their prior uploaded mesh
            // remains drawable throughout transactional replacement, so canonical/enclosed-view
            // membership can switch atomically without exposing the sky. Starting a global cut
            // transition here would reclassify every visible surface slice for 240 ms after each
            // sparse tunnel edit even though no surface owner changed. A canonical-ready update
            // whose X/Z column set is unchanged belongs here too: vertical chunk readiness cannot
            // alter the 2D surface cut.
            self.pending_surface_selection = None;
            self.update_exact_lod_membership(canonical_chunks);
            self.lod_draw_plan_focus = focus;
            self.lod_draw_plan_revision = self.surface_patch_residency_revision;
            self.lod_draw_plan_dirty_reasons = 0;
            return rebuild_reason;
        }
        let patches = if rebuild_reason & synchronous_surface_reasons == 0
            && self.lod_draw_plan_focus == focus
            && let Some(focus) = focus
        {
            let target_changed = self
                .pending_surface_selection
                .as_ref()
                .is_none_or(|pending| {
                    pending.focus != focus || pending.canonical_columns != canonical_columns
                });
            if target_changed {
                self.pending_surface_selection = Some(PendingSurfaceSelection {
                    focus,
                    canonical_columns: canonical_columns.clone(),
                    build: SurfacePatchSelectionBuild::new(
                        focus,
                        &self.surface_patch_residency,
                        &canonical_columns,
                        &self.surface_incomplete_parents,
                    ),
                });
            }
            let Some(patches) = self
                .pending_surface_selection
                .as_mut()
                .and_then(|pending| pending.build.advance(LOD_SELECTION_WORK_ITEMS_PER_FRAME))
            else {
                // Keep presenting the last complete surface cut while its refinement is built.
                // Exact tunnel/cavern membership is independent and can still advance immediately.
                self.update_exact_lod_membership(canonical_chunks);
                return rebuild_reason;
            };
            self.pending_surface_selection = None;
            patches
        } else {
            self.pending_surface_selection = None;
            let mut patches = SurfacePatchSelection::default();
            if let Some(focus) = focus {
                patches.rebuild_with_incomplete_parents(
                    focus,
                    &self.surface_patch_residency,
                    &canonical_columns,
                    &self.surface_incomplete_parents,
                );
            }
            patches
        };
        let profile_changed = rebuild_reason
            & (LOD_PLAN_REBUILD_CANONICAL_PROFILE | LOD_PLAN_REBUILD_SURFACE_PROFILE)
            != 0;
        let (exact_transition_edges, incomplete_transition_edges, transition_mesh_key) =
            if patches == self.lod_draw_plan.patches && !profile_changed {
                (
                    self.lod_draw_plan.exact_transition_edges.clone(),
                    self.lod_draw_plan.incomplete_transition_edges,
                    self.lod_draw_plan.transition_mesh_key,
                )
            } else {
                let mut transitions = build_lod_transitions(
                    &patches,
                    &self.surface_patch_profiles,
                    &self.canonical_surface_profiles,
                );
                let transition_mesh_key = match self
                    .publish_lod_transition_mesh(&transitions.quads, &transitions.morph_heights)
                {
                    Ok(key) => key,
                    Err(()) => {
                        transitions.incomplete_edges = transitions
                            .incomplete_edges
                            .saturating_add(transitions.exact_edges.len() as u32);
                        transitions.exact_edges.clear();
                        None
                    }
                };
                (
                    transitions.exact_edges,
                    transitions.incomplete_edges,
                    transition_mesh_key,
                )
            };
        for key in changed_surface_lod_ownership_keys(
            &self.lod_draw_plan,
            &patches,
            &exact_transition_edges,
        ) {
            if let Some(mesh) = self.chunks.get_mut(&key) {
                mesh.lod_ownership_stale = true;
            }
        }
        let changed_canonical_columns = self
            .lod_draw_plan
            .canonical_columns
            .symmetric_difference(&canonical_columns)
            .copied()
            .collect::<HashSet<_>>();
        if !changed_canonical_columns.is_empty() {
            for (key, mesh) in &mut self.chunks {
                if key.0 == 0 && changed_canonical_columns.contains(&(key.1, key.3)) {
                    mesh.lod_ownership_stale = true;
                }
            }
        }
        let changed_canonical_chunks = self
            .lod_draw_plan
            .canonical_chunks
            .symmetric_difference(&canonical_chunks)
            .copied()
            .collect::<HashSet<_>>();
        for &(x, y, z) in &changed_canonical_chunks {
            if let Some(mesh) = self.chunks.get_mut(&(0, x, y, z)) {
                mesh.lod_ownership_stale = true;
            }
        }
        let enclosed_view_chunks = self.enclosed_view_ready_chunks.clone();
        self.mark_exact_replacement_ownership_stale(&canonical_chunks, &enclosed_view_chunks);
        let previous_plan_resident = self.lod_draw_plan_is_resident();
        let next_plan = LodDrawPlan {
            patches,
            canonical_columns,
            canonical_chunks,
            enclosed_view_chunks,
            exact_transition_edges,
            incomplete_transition_edges,
            transition_mesh_key,
        };
        if CUT_TRANSITION_SECONDS > 0.0
            && next_plan != self.lod_draw_plan
            && self.lod_draw_plan.has_geometry()
            && previous_plan_resident
            && self.lod_draw_plan_focus.is_some()
            && focus.is_some()
            && !cut_transition_is_active(
                self.cut_transition
                    .as_ref()
                    .map(|transition| transition.started_at),
                self.time,
            )
        {
            self.cut_transition = Some(CutTransition {
                from: self.lod_draw_plan.clone(),
                from_focus: self.lod_draw_plan_focus,
                started_at: self.time,
            });
        }
        self.lod_draw_plan = next_plan;
        self.lod_draw_plan_focus = focus;
        self.lod_draw_plan_revision = self.surface_patch_residency_revision;
        self.lod_draw_plan_dirty_reasons = 0;
        rebuild_reason
    }

    fn lod_draw_plan_is_resident(&self) -> bool {
        lod_draw_plan_resident(
            &self.lod_draw_plan,
            &self.surface_patch_residency,
            &self.chunks,
            &self.canonical_surface_profiles,
        )
    }

    fn publish_lod_transition_mesh(
        &mut self,
        gpu_quads: &[GpuQuad],
        morph_heights: &[GpuMorph],
    ) -> Result<Option<MeshKey>, ()> {
        if gpu_quads.is_empty() {
            return Ok(None);
        }
        let gpu_quads = gpu_quads
            .iter()
            .copied()
            .map(|mut quad| {
                if quad.material_face & GPU_SOURCE_MASK == 0 {
                    quad.material_face =
                        pack_gpu_source_material(quad.material_face, GPU_SOURCE_LOD_CONNECTOR);
                }
                quad
            })
            .collect::<Vec<_>>();
        let active = self.lod_draw_plan.transition_mesh_key;
        let key = if active == Some(LOD_TRANSITION_MESH_KEYS[0]) {
            LOD_TRANSITION_MESH_KEYS[1]
        } else {
            LOD_TRANSITION_MESH_KEYS[0]
        };
        if gpu_quads_match_resident(self.chunks.get(&key), &gpu_quads, Some(morph_heights)) {
            return Ok(Some(key));
        }
        let Some((bounds_min, bounds_max)) = gpu_quad_bounds(&gpu_quads) else {
            return Err(());
        };
        let quad_count = gpu_quads.len() as u32;
        let slice = MeshSlice {
            relative_offset: 0,
            size: quad_count * size_of::<GpuQuad>() as u32,
            quad_count,
            bounds_min,
            bounds_max,
            surface_patch_id: None,
            boundary_edge: None,
            stitch_edges: 0,
            morph_closure: false,
            exact_replacement_chunk: None,
            canonical_water_surface: false,
            render_layer: RenderLayer::Opaque,
        };
        let Some(prepared) =
            self.prepare_mesh_sliced(key, &gpu_quads, Some(morph_heights), vec![slice])
        else {
            return Err(());
        };
        commit_prepared_mesh(
            &mut self.arena,
            Some(&mut self.morph_arena),
            &mut self.chunks,
            key,
            Some(prepared),
        );
        Ok(Some(key))
    }

    fn maintain_cut_transition(&mut self, resident_hierarchy: bool) -> Option<f32> {
        if !resident_hierarchy
            || self.cut_transition.as_ref().is_some_and(|transition| {
                self.time - transition.started_at >= CUT_TRANSITION_SECONDS
            })
        {
            self.cut_transition = None;
        }
        let phase = self.cut_transition.as_ref().map(|transition| {
            if CUT_TRANSITION_SECONDS > 0.0 {
                ((self.time - transition.started_at) / CUT_TRANSITION_SECONDS).clamp(0.0, 1.0)
            } else {
                1.0
            }
        });
        let outgoing_key = self
            .cut_transition
            .as_ref()
            .and_then(|transition| transition.from.transition_mesh_key);
        for key in LOD_TRANSITION_MESH_KEYS {
            if self.lod_draw_plan.transition_mesh_key != Some(key) && outgoing_key != Some(key) {
                self.remove_opaque_mesh(key);
            }
        }
        if let Some(phase) = phase {
            self.queue.write_buffer(
                &self.cut_transition_buffers[0],
                0,
                bytemuck::bytes_of(&gpu_cut_transition(phase, 2.0, self.lod_draw_plan_focus)),
            );
            self.queue.write_buffer(
                &self.cut_transition_buffers[1],
                0,
                bytemuck::bytes_of(&gpu_cut_transition(
                    phase,
                    1.0,
                    self.cut_transition
                        .as_ref()
                        .and_then(|transition| transition.from_focus),
                )),
            );
        }
        phase
    }

    /// Browser-smoke diagnostics for proving that a revised remote surface product reached the
    /// resident GPU mesh, rather than stopping at the stream scheduler's revision bookkeeping.
    pub fn surface_tile_diagnostics(&self, coord: SurfaceTileCoord) -> Option<(u64, u32, u8)> {
        self.chunks
            .get(&(coord.level.index() + 1, coord.x, 0, coord.z))
            .map(|mesh| {
                (
                    mesh.content_fingerprint,
                    mesh.quad_count,
                    mesh.activation_mask,
                )
            })
    }

    /// Exact geometric representation selected at one world coordinate in the most
    /// recently built draw plan. Zero means no selected owner and is therefore a coverage bug.
    pub fn presented_lod_stride_voxels(&self, voxel_x: i32, voxel_y: i32, voxel_z: i32) -> u16 {
        let current = self.lod_draw_plan.presented_stride_at(
            self.lod_draw_plan_focus,
            voxel_x,
            voxel_y,
            voxel_z,
        );
        let outgoing = self.cut_transition.as_ref().map_or(0, |transition| {
            transition
                .from
                .presented_stride_at(transition.from_focus, voxel_x, voxel_y, voxel_z)
        });
        match (current, outgoing) {
            (0, stride) | (stride, 0) => stride,
            (left, right) => left.min(right),
        }
    }

    /// Number of horizontal cells owned by the currently active exact canonical vertical band.
    ///
    /// Ownership follows transactional chunk readiness, not the presence of a top-surface sample:
    /// a dug shaft is valid empty canonical space and must not resurrect its surface parent.
    pub fn canonical_surface_coverage_at(&self, voxel_x: i32, voxel_z: i32) -> (u16, u16) {
        let column = (
            voxel_x.div_euclid(CHUNK_EDGE as i32),
            voxel_z.div_euclid(CHUNK_EDGE as i32),
        );
        let covered = canonical_surface_cell_coverage(column, &self.canonical_surface_ready_chunks);
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
            if let Some(morph_allocation) = chunk.morph_allocation {
                let _ = self.morph_arena.free(morph_allocation);
            }
        }
    }

    fn remove_water_mesh(&mut self, key: MeshKey) {
        if let Some(chunk) = self.water_chunks.remove(&key) {
            let _ = self.water_arena.free(chunk.allocation);
            debug_assert!(chunk.morph_allocation.is_none());
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
            self.geometric_lod_focus,
            self.runtime_config,
        );
        let view_projection = glam::Mat4::from_cols_array_2d(&uniform.view_projection);
        let view_clip = AabbClipVolume::new(view_projection);
        let shadow_clips = shadow_cascades
            .cascades
            .map(|cascade| AabbClipVolume::new(cascade.clip_from_world));
        let cull_started = now_ms();
        let geometric_lod_focus =
            active_geometric_lod_focus(self.geometric_lod_focus, self.options.far_terrain);
        let resident_hierarchy = geometric_lod_focus.is_some();
        let lod_plan_started = now_ms();
        let lod_plan_rebuild_reason = if resident_hierarchy {
            self.refresh_lod_draw_plan(geometric_lod_focus)
        } else {
            0
        };
        let cut_transition_phase = self.maintain_cut_transition(resident_hierarchy);
        let cpu_lod_plan_ms = (now_ms() - lod_plan_started).max(0.0) as f32;
        let (virtual_visible, virtual_ownership) =
            if self.virtual_terrain_mode == VirtualTerrainRenderMode::Visible {
                let Some(cut) = self
                    .virtual_terrain_cut
                    .as_ref()
                    .filter(|cut| cut.is_renderable())
                else {
                    return false;
                };
                let Ok(ownership) = VirtualTerrainOwnership::from_cut(cut) else {
                    return false;
                };
                if self.validate_virtual_terrain_handoff(&ownership).is_ok() {
                    (true, ownership)
                } else {
                    // The legacy LOD plan can change after a cut was certified. Fall back
                    // atomically rather than allowing a newly crossing slice to overlap a virtual
                    // root for even one frame.
                    self.virtual_terrain_mode = VirtualTerrainRenderMode::Shadow;
                    (false, VirtualTerrainOwnership::default())
                }
            } else {
                (false, VirtualTerrainOwnership::default())
            };
        let virtual_shadow = self.virtual_terrain_mode == VirtualTerrainRenderMode::Shadow;
        // Queue readiness is not a proof that every fixed geometric owner is resident. Canonical
        // columns can still replace atomically and retained surface tiles can be incomplete. Keep
        // the cached resident hierarchy authoritative after settling as well as while streaming.
        let lod_draw_plan = resident_hierarchy.then_some(&self.lod_draw_plan);
        let cut_draw_lists = if let Some(transition) = self.cut_transition.as_ref() {
            let Ok(draw_lists) = collect_cut_transition_draw_lists(
                &self.chunks,
                &self.lod_draw_plan,
                geometric_lod_focus,
                transition,
                self.options.far_terrain,
                view_clip,
                &virtual_ownership,
            ) else {
                return false;
            };
            Some(draw_lists)
        } else {
            None
        };
        let Ok((shadow_draw_lists, world_draw_list, lod_ownership_refreshes)) =
            collect_opaque_draw_lists(
                &mut self.chunks,
                lod_draw_plan,
                cut_draw_lists.as_ref(),
                self.options.far_terrain,
                shadows_active,
                geometric_lod_focus,
                view_clip,
                shadow_clips,
                &virtual_ownership,
            )
        else {
            return false;
        };
        // Shadow mode builds the same bounded directories without mixing their geometry into the
        // visible legacy owner. Visible mode replaces only complete fixed-region volumes; legacy
        // remains authoritative everywhere else.
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
                    && (key.0 == 0 || self.options.far_terrain)
                    && view_clip.contains_aabb(chunk.bounds_min, chunk.bounds_max)
            },
            |key, slice| {
                slice.render_layer == RenderLayer::Translucent
                    && slice_owned_by_lod(geometric_lod_focus, lod_draw_plan, key, slice)
                    && !virtual_ownership.covers_aabb(slice.bounds_min, slice.bounds_max)
                    && view_clip.contains_aabb(slice.bounds_min, slice.bounds_max)
            },
        );
        // Water has no parent-height sidecar. Drawing its stale cut through a screen-space Bayer
        // mask produced detached square fragments, so it switches atomically with the complete
        // current owner instead of overlaying old geometry.
        let outgoing_water_draw_list = DrawList::default();
        let cpu_cull_ms = (now_ms() - cull_started).max(0.0) as f32;
        let encode_started = now_ms();
        self.avatar_gpu
            .prepare(&self.queue, &self.remote_avatars, self.time);
        let avatar_instances = self.avatar_gpu.instance_count();
        let has_avatars = avatar_instances != 0;
        let refract_water = self.options.water
            && (!water_draw_list.spans.is_empty()
                || !outgoing_water_draw_list.spans.is_empty()
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
            self.shadow_gpu.write_cascades(
                &self.queue,
                &shadow_cascades,
                camera,
                geometric_lod_focus,
            );
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
        let virtual_candidate = if virtual_shadow {
            let (Some(oracle_view), Some(oracle_cut)) = (
                self.virtual_terrain_oracle_view,
                self.virtual_terrain_oracle_cut.as_ref(),
            ) else {
                return false;
            };
            Some((oracle_view, oracle_cut.fingerprint))
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
                    virtual_terrain: virtual_shadow,
                },
            )
        });
        if let Some((oracle_view, oracle_fingerprint)) = virtual_candidate {
            let timestamps = gpu_frame
                .as_ref()
                .map(|frame| VirtualTerrainGpuTimestampWrites {
                    query_set: &frame.query_set,
                    traversal_first_query: 24,
                    compaction_first_query: 26,
                });
            if self
                .virtual_terrain_gpu
                .encode_traversal(
                    &self.queue,
                    &mut encoder,
                    oracle_view,
                    oracle_fingerprint,
                    timestamps,
                )
                .is_err()
            {
                if let (Some(timer), Some(frame)) = (self.gpu_timer.as_ref(), gpu_frame.take()) {
                    timer.cancel_frame(frame);
                }
                return false;
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
                pass.set_pipeline(&self.shadow_gpu.morph_pipeline);
                shadow_draw_calls = shadow_draw_calls.saturating_add(draw_morph_spans(
                    &mut pass,
                    &self.arena_buffers,
                    &self.morph_arena_buffers,
                    &draw_list.morphing,
                ));
                if virtual_visible {
                    pass.set_pipeline(&self.shadow_gpu.fixed_pipeline);
                    pass.set_vertex_buffer(
                        0,
                        self.virtual_terrain_gpu.compact_surface_buffer().slice(..),
                    );
                    pass.draw_indirect(
                        self.virtual_terrain_gpu.indirect_buffer(),
                        VIRTUAL_TERRAIN_SURFACE_INDIRECT_OFFSET,
                    );
                    pass.set_pipeline(&self.shadow_gpu.virtual_triangle_pipeline);
                    pass.set_vertex_buffer(
                        0,
                        self.virtual_terrain_gpu.compact_triangle_buffer().slice(..),
                    );
                    pass.draw_indirect(
                        self.virtual_terrain_gpu.indirect_buffer(),
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
                    pass.set_pipeline(&self.depth_prepass_fast_pipeline);
                    pass.set_vertex_buffer(
                        0,
                        self.virtual_terrain_gpu.compact_surface_buffer().slice(..),
                    );
                    pass.draw_indirect(
                        self.virtual_terrain_gpu.indirect_buffer(),
                        VIRTUAL_TERRAIN_SURFACE_INDIRECT_OFFSET,
                    );
                    pass.set_pipeline(&self.virtual_triangle_depth_pipeline);
                    pass.set_vertex_buffer(
                        0,
                        self.virtual_terrain_gpu.compact_triangle_buffer().slice(..),
                    );
                    pass.draw_indirect(
                        self.virtual_terrain_gpu.indirect_buffer(),
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
                pass.set_pipeline(&self.depth_prepass_morph_pipeline);
                depth_prepass_draw_calls =
                    depth_prepass_draw_calls.saturating_add(draw_morph_spans(
                        &mut pass,
                        &self.arena_buffers,
                        &self.morph_arena_buffers,
                        &world_draw_list.morphing,
                    ));
                if let Some(cut_draw_lists) = &cut_draw_lists {
                    pass.set_pipeline(&self.depth_prepass_transition_pipeline);
                    pass.set_bind_group(3, &self.cut_transition_bind_groups[0], &[]);
                    depth_prepass_draw_calls =
                        depth_prepass_draw_calls.saturating_add(draw_morph_spans(
                            &mut pass,
                            &self.arena_buffers,
                            &self.morph_arena_buffers,
                            &cut_draw_lists.incoming.morphing,
                        ));
                    pass.set_pipeline(&self.depth_prepass_transition_fixed_pipeline);
                    pass.set_bind_group(3, &self.cut_transition_bind_groups[1], &[]);
                    depth_prepass_draw_calls = depth_prepass_draw_calls.saturating_add(draw_spans(
                        &mut pass,
                        &self.arena_buffers,
                        &cut_draw_lists.outgoing.fixed,
                    ));
                    pass.set_pipeline(&self.depth_prepass_transition_pipeline);
                    depth_prepass_draw_calls =
                        depth_prepass_draw_calls.saturating_add(draw_morph_spans(
                            &mut pass,
                            &self.arena_buffers,
                            &self.morph_arena_buffers,
                            &cut_draw_lists.outgoing.morphing,
                        ));
                }
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
            if self.screenshot_requested {
                let Some(opaque) = screenshot_diagnostic_owner_buffers(
                    &self.device,
                    &self.queue,
                    &self.arena_buffers,
                    &self.chunks,
                    "screenshot opaque terrain owner sidecar",
                ) else {
                    return false;
                };
                let virtual_opaque = if virtual_visible {
                    let Some(owners) = screenshot_virtual_terrain_owner_buffers(
                        &self.device,
                        &self.queue,
                        &self.virtual_terrain_arena_buffers,
                        &self.virtual_terrain_pages,
                        "screenshot virtual terrain owner sidecar",
                    ) else {
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
                    return false;
                };
                (Some(opaque), virtual_opaque, Some(water))
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
        let screenshot_diagnostic_identity_target = self.screenshot_requested.then(|| {
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
        let screenshot_diagnostic_depth_target = self.screenshot_requested.then(|| {
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
            pass.set_bind_group(3, &self.cut_transition_bind_groups[0], &[]);
            let (fixed_pipeline, morph_pipeline, transition_pipeline, morph_transition_pipeline) =
                if self.options.screen_space_ambient_occlusion {
                    if self.options.material_detail {
                        (
                            &self.voxel_ambient_occlusion_pipeline,
                            &self.voxel_morph_ambient_occlusion_pipeline,
                            &self.voxel_transition_ambient_occlusion_pipeline,
                            &self.voxel_morph_transition_ambient_occlusion_pipeline,
                        )
                    } else {
                        (
                            &self.voxel_ambient_occlusion_flat_pipeline,
                            &self.voxel_morph_ambient_occlusion_flat_pipeline,
                            &self.voxel_transition_ambient_occlusion_flat_pipeline,
                            &self.voxel_morph_transition_ambient_occlusion_flat_pipeline,
                        )
                    }
                } else if self.options.material_detail {
                    (
                        &self.voxel_pipeline,
                        &self.voxel_morph_pipeline,
                        &self.voxel_transition_pipeline,
                        &self.voxel_morph_transition_pipeline,
                    )
                } else {
                    (
                        &self.voxel_flat_pipeline,
                        &self.voxel_morph_flat_pipeline,
                        &self.voxel_transition_flat_pipeline,
                        &self.voxel_morph_transition_flat_pipeline,
                    )
                };
            if virtual_visible {
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
                pass.set_pipeline(fixed_pipeline);
                pass.set_vertex_buffer(
                    0,
                    self.virtual_terrain_gpu.compact_surface_buffer().slice(..),
                );
                pass.draw_indirect(
                    self.virtual_terrain_gpu.indirect_buffer(),
                    VIRTUAL_TERRAIN_SURFACE_INDIRECT_OFFSET,
                );
                pass.set_pipeline(virtual_triangle_pipeline);
                pass.set_vertex_buffer(
                    0,
                    self.virtual_terrain_gpu.compact_triangle_buffer().slice(..),
                );
                pass.draw_indirect(
                    self.virtual_terrain_gpu.indirect_buffer(),
                    VIRTUAL_TERRAIN_TRIANGLE_INDIRECT_OFFSET,
                );
            }
            pass.set_pipeline(fixed_pipeline);
            draw_spans(&mut pass, &self.arena_buffers, &world_draw_list.fixed);
            pass.set_pipeline(morph_pipeline);
            draw_morph_spans(
                &mut pass,
                &self.arena_buffers,
                &self.morph_arena_buffers,
                &world_draw_list.morphing,
            );
            if let Some(cut_draw_lists) = &cut_draw_lists {
                pass.set_pipeline(morph_transition_pipeline);
                pass.set_bind_group(3, &self.cut_transition_bind_groups[0], &[]);
                draw_morph_spans(
                    &mut pass,
                    &self.arena_buffers,
                    &self.morph_arena_buffers,
                    &cut_draw_lists.incoming.morphing,
                );
                pass.set_pipeline(transition_pipeline);
                pass.set_bind_group(3, &self.cut_transition_bind_groups[1], &[]);
                draw_spans(
                    &mut pass,
                    &self.arena_buffers,
                    &cut_draw_lists.outgoing.fixed,
                );
                pass.set_pipeline(morph_transition_pipeline);
                draw_morph_spans(
                    &mut pass,
                    &self.arena_buffers,
                    &self.morph_arena_buffers,
                    &cut_draw_lists.outgoing.morphing,
                );
            }
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
            pass.set_bind_group(3, &self.cut_transition_bind_groups[0], &[]);
            if virtual_visible {
                pass.set_pipeline(&self.water_pipeline);
                pass.set_vertex_buffer(0, self.virtual_terrain_gpu.compact_water_surface_slice());
                pass.draw_indirect(
                    self.virtual_terrain_gpu.indirect_buffer(),
                    VIRTUAL_TERRAIN_WATER_SURFACE_INDIRECT_OFFSET,
                );
                pass.set_pipeline(&self.virtual_triangle_water_pipeline);
                pass.set_vertex_buffer(0, self.virtual_terrain_gpu.compact_water_triangle_slice());
                pass.draw_indirect(
                    self.virtual_terrain_gpu.indirect_buffer(),
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
            if !outgoing_water_draw_list.spans.is_empty() {
                pass.set_pipeline(&self.water_transition_pipeline);
                pass.set_bind_group(3, &self.cut_transition_bind_groups[1], &[]);
                for span in &outgoing_water_draw_list.spans {
                    let Some(buffer) = self.water_arena_buffers.get(span.page as usize) else {
                        continue;
                    };
                    let start = u64::from(span.offset);
                    let end = start + u64::from(span.size);
                    pass.set_vertex_buffer(0, buffer.slice(start..end));
                    pass.draw(0..QUAD_VERTEX_COUNT, 0..span.quad_count);
                }
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
        let morph_arena = self.morph_arena.stats();
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
        let cut_draw_calls = cut_draw_lists.as_ref().map_or(0, |lists| {
            lists
                .incoming
                .fixed
                .spans
                .len()
                .saturating_add(lists.incoming.morphing.spans.len())
                .saturating_add(lists.outgoing.fixed.spans.len())
                .saturating_add(lists.outgoing.morphing.spans.len())
        });
        let cut_quads = cut_draw_lists.as_ref().map_or(0, |lists| {
            lists
                .incoming
                .quad_count
                .saturating_add(lists.outgoing.quad_count)
        });
        let cut_meshes = cut_draw_lists.as_ref().map_or(0, |lists| {
            lists
                .incoming
                .mesh_count
                .saturating_add(lists.outgoing.mesh_count)
        });
        let cut_tested_slices = cut_draw_lists.as_ref().map_or(0, |lists| {
            lists
                .incoming
                .tested_slices
                .saturating_add(lists.outgoing.tested_slices)
        });
        let cut_selected_slices = cut_draw_lists.as_ref().map_or(0, |lists| {
            lists
                .incoming
                .selected_slices
                .saturating_add(lists.outgoing.selected_slices)
        });
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
            outgoing_water_draw_list.fingerprint,
        );
        let viewport_fingerprint = cut_draw_lists
            .as_ref()
            .map_or(viewport_fingerprint, |lists| {
                fingerprint_value(
                    fingerprint_value(viewport_fingerprint, lists.incoming.fingerprint),
                    lists.outgoing.fingerprint,
                )
            });
        let visible_terrain_meshes = world_draw_list
            .mesh_count
            .saturating_add(cut_meshes)
            .saturating_add(if virtual_visible {
                virtual_world_draw_lists.mesh_count
            } else {
                0
            });
        let visible_terrain_draw_calls = world_draw_list
            .fixed
            .spans
            .len()
            .saturating_add(world_draw_list.morphing.spans.len())
            .saturating_add(cut_draw_calls)
            .saturating_add(usize::from(virtual_visible) * 2);
        let visible_terrain_primitives = world_draw_list
            .quad_count
            .saturating_add(cut_quads)
            .saturating_add(if virtual_visible {
                virtual_world_draw_lists.primitive_count
            } else {
                0
            });
        let visible_water_draw_calls = water_draw_list
            .spans
            .len()
            .saturating_add(outgoing_water_draw_list.spans.len())
            .saturating_add(usize::from(virtual_visible && refract_water) * 2)
            as u32;
        let visible_water_primitives = water_draw_list
            .quad_count
            .saturating_add(outgoing_water_draw_list.quad_count)
            .saturating_add(if virtual_visible {
                virtual_world_draw_lists
                    .water_surfaces
                    .quad_count
                    .saturating_add(virtual_world_draw_lists.water_triangles.vertex_count / 3)
            } else {
                0
            });
        let gpu_virtual_feedback = self.virtual_terrain_gpu.latest_feedback();
        let gpu_virtual_matches_cpu = gpu_virtual_feedback.as_ref().is_some_and(|feedback| {
            gpu_feedback_matches_cut(feedback, self.virtual_terrain_oracle_cut.as_ref())
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
            virtual_terrain_gpu_selected_pages: gpu_virtual_feedback
                .as_ref()
                .map_or(0, |feedback| feedback.selected_pages.len() as u32),
            virtual_terrain_gpu_requested_pages: gpu_virtual_feedback
                .as_ref()
                .map_or(0, |feedback| feedback.requested_pages.len() as u32),
            virtual_terrain_gpu_ownerless_roots: gpu_virtual_feedback
                .as_ref()
                .map_or(0, |feedback| feedback.ownerless_roots),
            virtual_terrain_gpu_visited_nodes: gpu_virtual_feedback
                .as_ref()
                .map_or(0, |feedback| feedback.visited_nodes),
            virtual_terrain_gpu_overflow_flags: gpu_virtual_feedback
                .as_ref()
                .map_or(0, |feedback| feedback.overflow_flags),
            virtual_terrain_gpu_stack_peak: gpu_virtual_feedback
                .as_ref()
                .map_or(0, |feedback| feedback.stack_peak),
            virtual_terrain_gpu_compacted_surface_elements: gpu_virtual_feedback
                .as_ref()
                .map_or(0, |feedback| feedback.compacted_surface_elements),
            virtual_terrain_gpu_compacted_triangle_elements: gpu_virtual_feedback
                .as_ref()
                .map_or(0, |feedback| feedback.compacted_triangle_elements),
            virtual_terrain_gpu_compacted_water_surface_elements: gpu_virtual_feedback
                .as_ref()
                .map_or(0, |feedback| feedback.compacted_water_surface_elements),
            virtual_terrain_gpu_compacted_water_triangle_elements: gpu_virtual_feedback
                .as_ref()
                .map_or(0, |feedback| feedback.compacted_water_triangle_elements),
            virtual_terrain_gpu_compacted_pages: gpu_virtual_feedback
                .as_ref()
                .map_or(0, |feedback| feedback.compacted_pages),
            virtual_terrain_gpu_compaction_overflow_flags: gpu_virtual_feedback
                .as_ref()
                .map_or(0, |feedback| feedback.compaction_overflow_flags),
            virtual_terrain_gpu_matches_cpu_cut: gpu_virtual_matches_cpu,
            viewport_fingerprint,
            refraction_copy_bytes: refraction_copy_bytes(
                self.config.width,
                self.config.height,
                refract_water,
            ),
            arena_pages: arena
                .pages
                .saturating_add(morph_arena.pages)
                .saturating_add(water_arena.pages)
                .saturating_add(virtual_terrain_arena.pages)
                .saturating_add(2) as u32,
            arena_capacity_bytes: arena
                .capacity_bytes
                .saturating_add(morph_arena.capacity_bytes)
                .saturating_add(water_arena.capacity_bytes)
                .saturating_add(virtual_terrain_arena.capacity_bytes)
                .saturating_add(VIRTUAL_TERRAIN_COMPACT_SURFACE_BYTES)
                .saturating_add(VIRTUAL_TERRAIN_COMPACT_TRIANGLE_BYTES)
                .saturating_add(VIRTUAL_TERRAIN_COMPACT_WATER_SURFACE_BYTES)
                .saturating_add(VIRTUAL_TERRAIN_COMPACT_WATER_TRIANGLE_BYTES),
            arena_allocated_bytes: arena
                .allocated_bytes
                .saturating_add(morph_arena.allocated_bytes)
                .saturating_add(water_arena.allocated_bytes)
                .saturating_add(virtual_terrain_arena.allocated_bytes)
                .saturating_add(gpu_virtual_feedback.as_ref().map_or(0, |feedback| {
                    u64::from(feedback.compacted_surface_elements)
                        .saturating_mul(size_of::<GpuQuad>() as u64)
                        .saturating_add(
                            u64::from(feedback.compacted_triangle_elements)
                                .saturating_mul(size_of::<GpuTerrainVertex>() as u64),
                        )
                        .saturating_add(
                            u64::from(feedback.compacted_water_surface_elements)
                                .saturating_mul(size_of::<GpuQuad>() as u64),
                        )
                        .saturating_add(
                            u64::from(feedback.compacted_water_triangle_elements)
                                .saturating_mul(size_of::<GpuTerrainVertex>() as u64),
                        )
                })),
            core_gpu_bytes: arena
                .capacity_bytes
                .saturating_add(morph_arena.capacity_bytes)
                .saturating_add(water_arena.capacity_bytes)
                .saturating_add(virtual_terrain_arena.capacity_bytes)
                .saturating_add(VIRTUAL_TERRAIN_COMPACT_SURFACE_BYTES)
                .saturating_add(VIRTUAL_TERRAIN_COMPACT_TRIANGLE_BYTES)
                .saturating_add(VIRTUAL_TERRAIN_COMPACT_WATER_SURFACE_BYTES)
                .saturating_add(VIRTUAL_TERRAIN_COMPACT_WATER_TRIANGLE_BYTES)
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
            gpu_virtual_terrain_traversal_ms: gpu_timing
                .map(|timing| timing.virtual_terrain_traversal_ms),
            gpu_virtual_terrain_compaction_ms: gpu_timing
                .map(|timing| timing.virtual_terrain_compaction_ms),
            cpu_cull_ms,
            cpu_lod_plan_ms,
            lod_plan_rebuild_reason,
            cpu_encode_ms: 0.0,
            cpu_submit_ms: 0.0,
            lod_ownership_refreshes,
            draw_list_tested_slices: shadow_draw_lists
                .iter()
                .map(|draw_list| draw_list.tested_slices)
                .sum::<u32>()
                .saturating_add(world_draw_list.tested_slices)
                .saturating_add(cut_tested_slices)
                .saturating_add(if virtual_visible {
                    virtual_world_draw_lists.surfaces.tested_slices
                } else {
                    0
                })
                .saturating_add(water_draw_list.tested_slices)
                .saturating_add(outgoing_water_draw_list.tested_slices),
            draw_list_selected_slices: shadow_draw_lists
                .iter()
                .map(|draw_list| draw_list.selected_slices)
                .sum::<u32>()
                .saturating_add(world_draw_list.selected_slices)
                .saturating_add(cut_selected_slices)
                .saturating_add(if virtual_visible {
                    virtual_world_draw_lists.surfaces.selected_slices
                } else {
                    0
                })
                .saturating_add(water_draw_list.selected_slices)
                .saturating_add(outgoing_water_draw_list.selected_slices),
            lod_transition_quads: self
                .lod_draw_plan
                .transition_mesh_key
                .and_then(|key| self.chunks.get(&key))
                .map_or(0, |mesh| mesh.quad_count),
            lod_incomplete_transition_edges: self.lod_draw_plan.incomplete_transition_edges,
            lod_cut_transition_active: cut_transition_phase.is_some(),
            lod_cut_transition_phase: cut_transition_phase.unwrap_or(0.0),
            lod_boundary_centres: geometric_lod_focus
                .map_or([[0; 2]; 8], GeometricLodFocus::boundary_centres),
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
            pass.set_bind_group(3, &self.cut_transition_bind_groups[0], &[]);
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
            pass.set_pipeline(&self.screenshot_diagnostic_morph_pipeline);
            draw_diagnostic_morph_spans(
                &mut pass,
                &self.arena_buffers,
                opaque_owners,
                &self.morph_arena_buffers,
                &world_draw_list.morphing,
            );
            if let Some(cut_draw_lists) = &cut_draw_lists {
                pass.set_pipeline(&self.screenshot_diagnostic_morph_transition_pipeline);
                pass.set_bind_group(3, &self.cut_transition_bind_groups[0], &[]);
                draw_diagnostic_morph_spans(
                    &mut pass,
                    &self.arena_buffers,
                    opaque_owners,
                    &self.morph_arena_buffers,
                    &cut_draw_lists.incoming.morphing,
                );
                pass.set_pipeline(&self.screenshot_diagnostic_transition_pipeline);
                pass.set_bind_group(3, &self.cut_transition_bind_groups[1], &[]);
                draw_diagnostic_spans(
                    &mut pass,
                    &self.arena_buffers,
                    opaque_owners,
                    &cut_draw_lists.outgoing.fixed,
                );
                pass.set_pipeline(&self.screenshot_diagnostic_morph_transition_pipeline);
                draw_diagnostic_morph_spans(
                    &mut pass,
                    &self.arena_buffers,
                    opaque_owners,
                    &self.morph_arena_buffers,
                    &cut_draw_lists.outgoing.morphing,
                );
            }
            if refract_water {
                pass.set_pipeline(&self.screenshot_diagnostic_pipeline);
                pass.set_bind_group(3, &self.cut_transition_bind_groups[0], &[]);
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
                if !outgoing_water_draw_list.spans.is_empty() {
                    pass.set_pipeline(&self.screenshot_diagnostic_transition_pipeline);
                    pass.set_bind_group(3, &self.cut_transition_bind_groups[1], &[]);
                    draw_diagnostic_spans(
                        &mut pass,
                        &self.water_arena_buffers,
                        water_owners,
                        &outgoing_water_draw_list,
                    );
                }
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
        let (Some(diagnostic_identity_texture), Some(diagnostic_depth_texture)) =
            (diagnostic_identity_texture, diagnostic_depth_texture)
        else {
            (self.log_error)("screenshot capture failed: diagnostic targets were not created");
            self.report_screenshot_result(false);
            return;
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
        let diagnostic_identity_unpadded_bytes_per_row = match width.checked_mul(16) {
            Some(bytes) => bytes,
            None => {
                self.report_screenshot_result(false);
                return;
            }
        };
        let diagnostic_identity_padded_bytes_per_row = diagnostic_identity_unpadded_bytes_per_row
            .div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let diagnostic_identity_buffer_size =
            u64::from(diagnostic_identity_padded_bytes_per_row) * u64::from(height);
        let diagnostic_depth_padded_bytes_per_row = padded_bytes_per_row;
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
                        let diagnostic_identity = mapped.get(color_end..identity_end)?;
                        let diagnostic_depth = mapped.get(identity_end..)?;
                        let rgba = unpack_screenshot_rgba(
                            mapped.get(..color_end)?,
                            width,
                            height,
                            padded_bytes_per_row,
                            bgra,
                        )?;
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
                        let terrain_diagnostic_u32x5 = interleave_screenshot_diagnostic(
                            &diagnostic_identity,
                            &diagnostic_depth,
                            width,
                            height,
                        )?;
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
                    morph_page: None,
                    morph_offset: 0,
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
                .virtual_terrain_pages
                .get(key)
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
                                    morph_page: None,
                                    morph_offset: 0,
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
                            morph_page: None,
                            morph_offset: 0,
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
/// the directory's authoritative slice identities are expanded into an 8-byte transient vertex
/// stream. This avoids permanently increasing terrain GPU memory merely to support diagnostics.
fn screenshot_diagnostic_owner_buffers(
    device: &Device,
    queue: &Queue,
    arena_buffers: &[Buffer],
    chunks: &BTreeMap<MeshKey, ChunkMesh>,
    label: &'static str,
) -> Option<Vec<Buffer>> {
    let quad_bytes = size_of::<GpuQuad>() as u64;
    let owner_bytes = size_of::<[u32; 2]>() as u64;
    let buffers = arena_buffers
        .iter()
        .map(|base| {
            let slots = base.size().div_ceil(quad_bytes);
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: slots.saturating_mul(owner_bytes).max(owner_bytes),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        })
        .collect::<Vec<_>>();
    for (key, chunk) in chunks {
        let owner_buffer = buffers.get(chunk.allocation.page as usize)?;
        for slice in &chunk.slices {
            let base_offset =
                u64::from(chunk.allocation.offset).checked_add(u64::from(slice.relative_offset))?;
            if !base_offset.is_multiple_of(quad_bytes) {
                return None;
            }
            let owner_offset = (base_offset / quad_bytes).checked_mul(owner_bytes)?;
            let owner = diagnostic_owner_for_slice(*key, slice);
            let owners = vec![owner; slice.quad_count as usize];
            let bytes = bytemuck::cast_slice(&owners);
            if owner_offset.checked_add(bytes.len() as u64)? > owner_buffer.size() {
                return None;
            }
            queue.write_buffer(owner_buffer, owner_offset, bytes);
        }
    }
    Some(buffers)
}

fn screenshot_virtual_terrain_owner_buffers(
    device: &Device,
    queue: &Queue,
    arena_buffers: &[Buffer],
    pages: &BTreeMap<TerrainPageKey, VirtualTerrainGpuPage>,
    label: &'static str,
) -> Option<Vec<Buffer>> {
    let primitive_bytes = size_of::<GpuQuad>() as u64;
    debug_assert_eq!(primitive_bytes, size_of::<GpuTerrainVertex>() as u64);
    let owner_bytes = size_of::<[u32; 2]>() as u64;
    let buffers = arena_buffers
        .iter()
        .map(|base| {
            let slots = base.size().div_ceil(primitive_bytes);
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: slots.saturating_mul(owner_bytes).max(owner_bytes),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        })
        .collect::<Vec<_>>();
    for (key, page) in pages {
        let owner = diagnostic_owner_id(
            DIAGNOSTIC_VIRTUAL_REPRESENTATION_BASE + u32::from(page.representation as u8),
            u32::from(key.level),
            key.coord[0],
            key.coord[1],
            key.coord[2],
        );
        let (allocation, count) = match &page.mesh {
            VirtualTerrainGpuMesh::Empty => continue,
            VirtualTerrainGpuMesh::Surface(mesh) => (mesh.allocation, mesh.quad_count),
            VirtualTerrainGpuMesh::Triangle(mesh) => (mesh.allocation, mesh.vertex_count),
        };
        let owner_buffer = buffers.get(allocation.page as usize)?;
        let base_offset = u64::from(allocation.offset);
        if !base_offset.is_multiple_of(primitive_bytes) {
            return None;
        }
        let owner_offset = (base_offset / primitive_bytes).checked_mul(owner_bytes)?;
        let owners = vec![owner; count as usize];
        let bytes = bytemuck::cast_slice(&owners);
        if owner_offset.checked_add(bytes.len() as u64)? > owner_buffer.size() {
            return None;
        }
        queue.write_buffer(owner_buffer, owner_offset, bytes);
    }
    Some(buffers)
}

fn diagnostic_owner_range(span: &DrawSpan) -> Option<std::ops::Range<u64>> {
    let quad_bytes = size_of::<GpuQuad>() as u64;
    let owner_bytes = size_of::<[u32; 2]>() as u64;
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
    let owner_bytes = size_of::<[u32; 2]>() as u64;
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

fn draw_morph_spans<'pass>(
    pass: &mut wgpu::RenderPass<'pass>,
    arena_buffers: &'pass [Buffer],
    morph_arena_buffers: &'pass [Buffer],
    draw_list: &DrawList,
) -> u32 {
    let mut draws = 0u32;
    for span in &draw_list.spans {
        let (Some(base_buffer), Some(morph_page)) =
            (arena_buffers.get(span.page as usize), span.morph_page)
        else {
            continue;
        };
        let Some(morph_buffer) = morph_arena_buffers.get(morph_page as usize) else {
            continue;
        };
        let base_start = u64::from(span.offset);
        let base_end = base_start + u64::from(span.size);
        let morph_start = u64::from(span.morph_offset);
        let morph_end = morph_start + u64::from(span.quad_count) * size_of::<GpuMorph>() as u64;
        pass.set_vertex_buffer(0, base_buffer.slice(base_start..base_end));
        pass.set_vertex_buffer(1, morph_buffer.slice(morph_start..morph_end));
        pass.draw(0..QUAD_VERTEX_COUNT, 0..span.quad_count);
        draws = draws.saturating_add(1);
    }
    draws
}

fn draw_diagnostic_morph_spans<'pass>(
    pass: &mut wgpu::RenderPass<'pass>,
    arena_buffers: &'pass [Buffer],
    owner_buffers: &'pass [Buffer],
    morph_arena_buffers: &'pass [Buffer],
    draw_list: &DrawList,
) -> u32 {
    let mut draws = 0u32;
    for span in &draw_list.spans {
        let (Some(base_buffer), Some(owner_buffer), Some(morph_page), Some(owner_range)) = (
            arena_buffers.get(span.page as usize),
            owner_buffers.get(span.page as usize),
            span.morph_page,
            diagnostic_owner_range(span),
        ) else {
            continue;
        };
        let Some(morph_buffer) = morph_arena_buffers.get(morph_page as usize) else {
            continue;
        };
        let base_start = u64::from(span.offset);
        let base_end = base_start + u64::from(span.size);
        let morph_start = u64::from(span.morph_offset);
        let morph_end = morph_start + u64::from(span.quad_count) * size_of::<GpuMorph>() as u64;
        pass.set_vertex_buffer(0, base_buffer.slice(base_start..base_end));
        pass.set_vertex_buffer(1, owner_buffer.slice(owner_range));
        pass.set_vertex_buffer(2, morph_buffer.slice(morph_start..morph_end));
        pass.draw(0..QUAD_VERTEX_COUNT, 0..span.quad_count);
        draws = draws.saturating_add(1);
    }
    draws
}

/// Builds the camera and three shadow selections in one resident-mesh traversal.
///
/// Geometric LOD ownership is independent of clip volume. Computing it once per opaque slice avoids
/// repeating the most expensive culling predicate for the camera and every shadow cascade while
/// preserving each list's independent clip tests, diagnostics, and ordering. Only the camera list
/// computes a presentation fingerprint; shadow fingerprints are never consumed.
#[allow(
    clippy::too_many_arguments,
    reason = "one traversal needs the independent camera, shadow, residency, and feature inputs"
)]
fn collect_opaque_draw_lists(
    chunks: &mut BTreeMap<MeshKey, ChunkMesh>,
    lod_draw_plan: Option<&LodDrawPlan>,
    cut_draw_lists: Option<&CutDrawLists>,
    far_terrain: bool,
    shadows: bool,
    geometric_lod_focus: Option<GeometricLodFocus>,
    view_clip: AabbClipVolume,
    shadow_clips: [AabbClipVolume; CASCADE_COUNT],
    virtual_ownership: &VirtualTerrainOwnership,
) -> Result<([WorldDrawLists; CASCADE_COUNT], WorldDrawLists, u32), MissingMorphSidecar> {
    let mut shadow_builders: [WorldDrawListBuilder; CASCADE_COUNT] =
        std::array::from_fn(|_| WorldDrawListBuilder::default());
    let mut world_builder = WorldDrawListBuilder::default();
    let mut lod_ownership_refreshes = 0u32;

    for (key, chunk) in chunks {
        if !chunk.active() || (key.0 != 0 && *key != EXACT_VOLUME_FRONTIER_MESH_KEY && !far_terrain)
        {
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
        if chunk.refresh_lod_ownership(key, geometric_lod_focus, lod_draw_plan) {
            lod_ownership_refreshes = lod_ownership_refreshes.saturating_add(1);
        }

        let mut world_mesh_selected = false;
        let mut shadow_mesh_selected = [false; CASCADE_COUNT];
        for (slice_index, slice) in chunk.slices.iter().enumerate() {
            if world_chunk_visible {
                world_builder.test_slice();
            }
            for cascade_index in 0..CASCADE_COUNT {
                if shadow_chunk_visible[cascade_index] {
                    shadow_builders[cascade_index].test_slice();
                }
            }
            if slice.render_layer != RenderLayer::Opaque
                || !chunk.lod_owns_slice(key, geometric_lod_focus, slice_index)
                || virtual_ownership.covers_aabb(slice.bounds_min, slice.bounds_max)
            {
                continue;
            }
            let cut_replaces_current = cut_draw_lists.is_some_and(|lists| {
                lists.replaced_current_slices.contains(&(*key, slice_index))
                    || slice
                        .surface_patch_id
                        .is_some_and(|patch| lists.replaced_current_patches.contains(&patch))
            });
            if world_chunk_visible
                && !cut_replaces_current
                && (world_chunk_clip == AabbClipClassification::Inside
                    || view_clip.contains_aabb(slice.bounds_min, slice.bounds_max))
            {
                let morphing = slice_uses_geometry_morph(key, geometric_lod_focus, slice);
                world_builder.select_slice(chunk, slice, morphing)?;
                world_mesh_selected = true;
            }
            for cascade_index in 0..CASCADE_COUNT {
                if shadow_chunk_visible[cascade_index]
                    && (shadow_chunk_clip[cascade_index] == AabbClipClassification::Inside
                        || shadow_clips[cascade_index]
                            .contains_aabb(slice.bounds_min, slice.bounds_max))
                {
                    shadow_builders[cascade_index].select_slice(
                        chunk,
                        slice,
                        slice_uses_geometry_morph(key, geometric_lod_focus, slice),
                    )?;
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
    Ok((
        shadow_draw_lists,
        world_builder.finish(),
        lod_ownership_refreshes,
    ))
}

fn lod_draw_plan_resident(
    plan: &LodDrawPlan,
    surface_patch_residency: &HashSet<SurfacePatchId>,
    chunks: &BTreeMap<MeshKey, ChunkMesh>,
    canonical_surface_profiles: &CanonicalColumnProfiles,
) -> bool {
    let committed_without_opaque_mesh = |x, y, z| {
        canonical_surface_profiles
            .get(&(x, z))
            .is_some_and(|profiles| profiles.contains_key(&y))
    };
    let surface_resident = plan
        .patches
        .owned_patches()
        .all(|patch| surface_patch_residency.contains(&patch));
    let canonical_resident = plan.canonical_chunks.iter().all(|&(x, y, z)| {
        chunks
            .get(&(0, x, y, z))
            .map_or_else(|| committed_without_opaque_mesh(x, y, z), ChunkMesh::active)
    });
    let enclosed_resident = plan.enclosed_view_chunks.iter().all(|&(x, y, z)| {
        chunks.contains_key(&(0, x, y, z)) || committed_without_opaque_mesh(x, y, z)
    });
    let connector_resident = plan
        .transition_mesh_key
        .is_none_or(|key| chunks.contains_key(&key));
    surface_resident && canonical_resident && enclosed_resident && connector_resident
}

/// Splits changed, morphable surface ownership away from the ordinary current draw list.
///
/// Incoming fine geometry unfolds from its parent in the same exact slice that it replaces.
///
/// Coarsening switches to the complete current parent atomically. A departing group of fine
/// profiles is not proof that its rendered shell covers the new parent: sparse surface cells can
/// legitimately leave openings that the coarser owner fills. Suppressing that parent exposed
/// patch-sized sky holes; overlaying both owners instead produced the source mixing and z-fighting
/// this transition exists to avoid.
fn collect_cut_transition_draw_lists(
    chunks: &BTreeMap<MeshKey, ChunkMesh>,
    current_plan: &LodDrawPlan,
    current_focus: Option<GeometricLodFocus>,
    transition: &CutTransition,
    far_terrain: bool,
    view_clip: AabbClipVolume,
    virtual_ownership: &VirtualTerrainOwnership,
) -> Result<CutDrawLists, MissingMorphSidecar> {
    let mut incoming = WorldDrawListBuilder::default();
    let mut replaced_current_slices = HashSet::new();
    for (key, chunk) in chunks {
        if !chunk.active()
            || (key.0 != 0 && *key != EXACT_VOLUME_FRONTIER_MESH_KEY && !far_terrain)
            || !view_clip.contains_aabb(chunk.bounds_min, chunk.bounds_max)
        {
            continue;
        }
        let mut incoming_mesh_selected = false;
        for (slice_index, slice) in chunk.slices.iter().enumerate() {
            incoming.test_slice();
            if slice.render_layer != RenderLayer::Opaque
                || virtual_ownership.covers_aabb(slice.bounds_min, slice.bounds_max)
                || !view_clip.contains_aabb(slice.bounds_min, slice.bounds_max)
            {
                continue;
            }
            let was_owned =
                slice_owned_by_lod(transition.from_focus, Some(&transition.from), key, slice);
            let is_owned = slice_owned_by_lod(current_focus, Some(current_plan), key, slice);
            let parent = slice.surface_patch_id.and_then(SurfacePatchId::parent);
            if is_owned
                && !was_owned
                && parent.is_some_and(|parent| transition.from.owns_patch(parent))
                && slice_uses_geometry_morph(key, current_focus, slice)
            {
                incoming.select_slice(chunk, slice, true)?;
                incoming_mesh_selected = true;
                replaced_current_slices.insert((*key, slice_index));
            }
        }
        if incoming_mesh_selected {
            incoming.select_mesh(*key, chunk);
        }
    }
    Ok(CutDrawLists {
        incoming: incoming.finish(),
        outgoing: WorldDrawLists::default(),
        replaced_current_slices,
        replaced_current_patches: HashSet::new(),
    })
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

fn diagnostic_owner_for_slice(key: MeshKey, slice: &MeshSlice) -> [u32; 2] {
    if let Some(patch) = slice.surface_patch_id {
        diagnostic_owner_id(2, u32::from(patch.level.index()) + 1, patch.x, 0, patch.z)
    } else if key.0 == 0 {
        diagnostic_owner_id(1, 0, key.1, key.2, key.3)
    } else {
        // Renderer-generated frontier and transition products are explicit owners too. Their mesh
        // key is stable in the selected-cut manifest and distinct from the page whose edge closes.
        diagnostic_owner_id(3, u32::from(key.0), key.1, key.2, key.3)
    }
}

fn gpu_quad_content_fingerprint(quads: &[GpuQuad], morph_heights: Option<&[GpuMorph]>) -> u64 {
    let quad_fingerprint = fingerprint_bytes(bytemuck::cast_slice(quads));
    morph_heights.map_or(quad_fingerprint, |heights| {
        fingerprint_value(
            quad_fingerprint,
            fingerprint_bytes(bytemuck::cast_slice(heights)),
        )
    })
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
    mut morph_storage: Option<(&mut ArenaAllocator, &mut Vec<Buffer>)>,
    gpu_quads: &[GpuQuad],
    morph_heights: Option<&[GpuMorph]>,
    mut slices: Vec<MeshSlice>,
    activation_mask: u8,
    buffer_label: &'static str,
) -> Option<ChunkMesh> {
    if gpu_quads.is_empty() {
        return None;
    }
    if morph_heights.is_some_and(|heights| heights.len() != gpu_quads.len()) {
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
    let morph_bytes = morph_heights.map(bytemuck::cast_slice::<GpuMorph, u8>);
    let morph_allocation = if let Some(morph_bytes) = morph_bytes {
        let Some((morph_arena, _)) = morph_storage.as_mut() else {
            let _ = arena.free(allocation);
            return None;
        };
        let Ok(morph_byte_len) = u32::try_from(morph_bytes.len()) else {
            let _ = arena.free(allocation);
            return None;
        };
        let Some(morph_allocation) = morph_arena.allocate(morph_byte_len) else {
            let _ = arena.free(allocation);
            return None;
        };
        Some(morph_allocation)
    } else {
        None
    };
    while arena_buffers.len() <= allocation.page as usize {
        let page = arena_buffers.len() as u16;
        let Some(capacity) = arena.page_capacity(page) else {
            let _ = arena.free(allocation);
            if let (Some(morph_allocation), Some((morph_arena, _))) =
                (morph_allocation, morph_storage.as_mut())
            {
                let _ = morph_arena.free(morph_allocation);
            }
            return None;
        };
        arena_buffers.push(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(buffer_label),
            size: u64::from(capacity),
            usage: wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
    }
    let Some(buffer) = arena_buffers.get(allocation.page as usize) else {
        let _ = arena.free(allocation);
        if let (Some(morph_allocation), Some((morph_arena, _))) =
            (morph_allocation, morph_storage.as_mut())
        {
            let _ = morph_arena.free(morph_allocation);
        }
        return None;
    };
    queue.write_buffer(buffer, u64::from(allocation.offset), bytes);
    if let (Some(morph_bytes), Some(morph_allocation)) = (morph_bytes, morph_allocation) {
        let Some((morph_arena, morph_arena_buffers)) = morph_storage.as_mut() else {
            let _ = arena.free(allocation);
            return None;
        };
        while morph_arena_buffers.len() <= morph_allocation.page as usize {
            let page = morph_arena_buffers.len() as u16;
            let Some(capacity) = morph_arena.page_capacity(page) else {
                let _ = arena.free(allocation);
                let _ = morph_arena.free(morph_allocation);
                return None;
            };
            morph_arena_buffers.push(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("voxel morph sidecar arena page"),
                size: u64::from(capacity),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let Some(morph_buffer) = morph_arena_buffers.get(morph_allocation.page as usize) else {
            let _ = arena.free(allocation);
            let _ = morph_arena.free(morph_allocation);
            return None;
        };
        queue.write_buffer(
            morph_buffer,
            u64::from(morph_allocation.offset),
            morph_bytes,
        );
    }
    let content_fingerprint = gpu_quad_content_fingerprint(gpu_quads, morph_heights);
    Some(ChunkMesh {
        allocation,
        morph_allocation,
        quad_count: gpu_quads.len() as u32,
        content_fingerprint,
        slices,
        lod_ownership_focus: None,
        lod_ownership_stale: true,
        lod_owned_slices: Vec::new(),
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
            discard_prepared_mesh(arena, None, Some(mesh));
        }
        VirtualTerrainGpuMesh::Triangle(mesh) => {
            let _ = arena.free(mesh.allocation);
        }
    }
}

fn virtual_terrain_gpu_geometry(mesh: &VirtualTerrainGpuMesh) -> VirtualTerrainGpuGeometry {
    match mesh {
        VirtualTerrainGpuMesh::Empty => VirtualTerrainGpuGeometry::default(),
        VirtualTerrainGpuMesh::Surface(mesh) => {
            let mut geometry = VirtualTerrainGpuGeometry::default();
            for slice in &mesh.slices {
                let range = VirtualTerrainGpuGeometryRange {
                    source_offset_bytes: u64::from(mesh.allocation.offset + slice.relative_offset),
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
                source_offset_bytes: u64::from(mesh.allocation.offset),
                element_count: mesh.opaque_vertex_count,
            },
            water_triangle: VirtualTerrainGpuGeometryRange {
                source_offset_bytes: u64::from(mesh.allocation.offset)
                    + u64::from(mesh.opaque_vertex_count) * size_of::<GpuTerrainVertex>() as u64,
                element_count: mesh.water_vertex_count,
            },
            ..VirtualTerrainGpuGeometry::default()
        },
    }
}

fn gpu_quads_match_resident(
    mesh: Option<&ChunkMesh>,
    quads: &[GpuQuad],
    morph_heights: Option<&[GpuMorph]>,
) -> bool {
    let content_fingerprint = gpu_quad_content_fingerprint(quads, morph_heights);
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

fn discard_prepared_mesh(
    arena: &mut ArenaAllocator,
    mut morph_arena: Option<&mut ArenaAllocator>,
    prepared: Option<ChunkMesh>,
) {
    if let Some(prepared) = prepared {
        let _ = arena.free(prepared.allocation);
        if let Some(morph_allocation) = prepared.morph_allocation {
            let Some(morph_arena) = morph_arena.as_mut() else {
                debug_assert!(false, "morph allocation has no owning arena");
                return;
            };
            let _ = morph_arena.free(morph_allocation);
        }
    }
}

fn commit_prepared_mesh(
    arena: &mut ArenaAllocator,
    mut morph_arena: Option<&mut ArenaAllocator>,
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
        if let Some(morph_allocation) = old.morph_allocation {
            let Some(morph_arena) = morph_arena.as_mut() else {
                debug_assert!(false, "morph allocation has no owning arena");
                return;
            };
            let _ = morph_arena.free(morph_allocation);
        }
    }
}

fn surface_patch_render_bounds(
    patch: &SurfacePatch,
    level: SurfaceLodLevel,
) -> (glam::Vec3, glam::Vec3) {
    let mut minimum = patch.bounds.min;
    let mut maximum = patch.bounds.max;
    if level != SurfaceLodLevel::Stride2 {
        // Coarse surface shaping displaces existing vertices vertically without changing the
        // streamed patch bounds. Keep culling conservative for the full signed three-bit range;
        // otherwise an edge-on patch can disappear while its deformed vertices remain on screen.
        minimum[1] = minimum[1].saturating_add(SURFACE_SHAPE_MIN_DELTA_VOXELS);
        maximum[1] = maximum[1].saturating_add(SURFACE_SHAPE_MAX_DELTA_VOXELS);
    }
    (
        glam::Vec3::from_array(minimum.map(|value| value as f32 * VOXEL_SIZE_METRES)),
        glam::Vec3::from_array(maximum.map(|value| value as f32 * VOXEL_SIZE_METRES)),
    )
}

fn surface_top_stitch_edges(
    tile: &SurfaceTileMesh,
    patch: &SurfacePatch,
    quad: voxels_world::SurfaceQuad,
    macro_normal: u32,
) -> u8 {
    let stride = tile.coord.stride_voxels();
    if quad.face != 2
        || quad.extent != [stride as u16; 2]
        || macro_normal & SURFACE_MACRO_NORMAL_FLAG == 0
    {
        return 0;
    }
    let [tile_x, tile_z] = tile.coord.voxel_origin();
    let min_x = tile_x.saturating_add(i32::from(patch.cell_bounds[0][0]).saturating_mul(stride));
    let min_z = tile_z.saturating_add(i32::from(patch.cell_bounds[0][1]).saturating_mul(stride));
    let max_x = tile_x.saturating_add(i32::from(patch.cell_bounds[1][0]).saturating_mul(stride));
    let max_z = tile_z.saturating_add(i32::from(patch.cell_bounds[1][1]).saturating_mul(stride));
    let mut mask = 0;
    if quad.origin[0] == min_x {
        mask |= 1 << SurfacePatchEdge::NegativeX.index();
    }
    if quad.origin[0].saturating_add(stride) == max_x {
        mask |= 1 << SurfacePatchEdge::PositiveX.index();
    }
    if quad.origin[2] == min_z {
        mask |= 1 << SurfacePatchEdge::NegativeZ.index();
    }
    if quad.origin[2].saturating_add(stride) == max_z {
        mask |= 1 << SurfacePatchEdge::PositiveZ.index();
    }
    mask
}

fn surface_macro_normals_and_shapes(tile: &SurfaceTileMesh) -> (Vec<u32>, Vec<u16>) {
    let stride = tile.coord.stride_voxels();
    let span = tile.coord.voxel_span();
    let [origin_x, origin_z] = tile.coord.voxel_origin();
    let edge = voxels_world::SURFACE_TILE_EDGE_CELLS as usize;
    let mut heights = vec![None::<(i32, usize)>; edge * edge];
    for (quad_index, quad) in tile.quads.iter().enumerate() {
        let local_x = i64::from(quad.origin[0]) - i64::from(origin_x);
        let local_z = i64::from(quad.origin[2]) - i64::from(origin_z);
        let is_base_top = quad.face == 2
            && quad.extent == [stride as u16; 2]
            && local_x >= 0
            && local_z >= 0
            && local_x < i64::from(span)
            && local_z < i64::from(span)
            && local_x % i64::from(stride) == 0
            && local_z % i64::from(stride) == 0;
        if is_base_top {
            // Base terrain is emitted before skyline proxies. Retaining the first value prevents
            // an aligned proxy cap from replacing the terrain sample underneath it.
            let cell_x = (local_x / i64::from(stride)) as usize;
            let cell_z = (local_z / i64::from(stride)) as usize;
            let cell = cell_x + cell_z * edge;
            if heights[cell].is_none() {
                heights[cell] = Some((quad.origin[1], quad_index));
            }
        }
    }

    let mut packed = vec![0xff; tile.quads.len()];
    let mut packed_shapes = vec![0_u16; tile.quads.len()];
    let shape_heights = if tile.coord.level == SurfaceLodLevel::Stride2 {
        Vec::new()
    } else {
        let grid_edge = voxels_world::SURFACE_TILE_EDGE_CELLS + 1;
        (0..grid_edge)
            .flat_map(|grid_z| {
                (0..grid_edge).map(move |grid_x| surface_shape_vertex_height(tile, grid_x, grid_z))
            })
            .collect::<Vec<_>>()
    };
    let mut cell_normals = vec![None::<u32>; edge * edge];
    for z in 0..edge {
        for x in 0..edge {
            let Some((_, quad_index)) = heights[x + z * edge] else {
                continue;
            };
            let normal = sampled_shading_normal(
                &tile.shading.heights,
                voxels_world::SURFACE_SHADING_EDGE_SAMPLES,
                x + 1,
                z + 1,
                stride,
            );
            let parent_normal = if tile.shading.parent_heights.is_empty() {
                normal
            } else {
                sampled_shading_normal(
                    &tile.shading.parent_heights,
                    voxels_world::SURFACE_PARENT_SHADING_EDGE_SAMPLES,
                    x / 2 + 2,
                    z / 2 + 2,
                    stride * 2,
                )
            };
            let value = pack_surface_macro_normals(normal, parent_normal);
            packed[quad_index] = value;
            if !shape_heights.is_empty() {
                packed_shapes[quad_index] =
                    surface_top_shape(tile, &shape_heights, &tile.quads[quad_index]);
            }
            cell_normals[x + z * edge] = Some(value);
        }
    }

    // Coarse height fields represent a smooth slope with flat tops separated by tall voxel walls.
    // Give only those generated terrain-body walls the owning cell's bounded slope normal. This
    // prevents distant hills from becoming black combs without adding geometry or accidentally
    // smoothing canonical cliffs or skyline proxies.
    for patch in &tile.patches {
        for range in std::iter::once(&patch.quad_range).chain(&patch.edge_ranges) {
            let start = range.start as usize;
            let end = range.end as usize;
            for (offset, (quad, packed_normal)) in tile.quads[start..end]
                .iter()
                .copied()
                .zip(&mut packed[start..end])
                .enumerate()
            {
                if quad.face == 2 || i32::from(quad.extent[0]) != stride {
                    continue;
                }
                if !shape_heights.is_empty() {
                    packed_shapes[start + offset] = surface_wall_shape(tile, &shape_heights, &quad);
                }
                let adjusted_x = i64::from(quad.origin[0])
                    - if quad.face == 0 {
                        i64::from(stride - 1)
                    } else {
                        0
                    };
                let adjusted_z = i64::from(quad.origin[2])
                    - if quad.face == 4 {
                        i64::from(stride - 1)
                    } else {
                        0
                    };
                let local_x = adjusted_x - i64::from(origin_x);
                let local_z = adjusted_z - i64::from(origin_z);
                if local_x < 0
                    || local_z < 0
                    || local_x >= i64::from(span)
                    || local_z >= i64::from(span)
                    || local_x % i64::from(stride) != 0
                    || local_z % i64::from(stride) != 0
                {
                    continue;
                }
                let cell_x = (local_x / i64::from(stride)) as usize;
                let cell_z = (local_z / i64::from(stride)) as usize;
                let cell = cell_x + cell_z * edge;
                let Some((height, _)) = heights[cell] else {
                    continue;
                };
                let quad_top = i64::from(quad.origin[1]) + i64::from(quad.extent[1]);
                if quad_top == i64::from(height) + 1
                    && let Some(normal) = cell_normals[cell]
                {
                    *packed_normal = normal;
                }
            }
        }
    }
    (packed, packed_shapes)
}

#[cfg(test)]
fn surface_macro_normals(tile: &SurfaceTileMesh) -> Vec<u32> {
    surface_macro_normals_and_shapes(tile).0
}

fn pack_surface_morph_deltas_half_voxels(deltas: [i32; 4]) -> GpuMorph {
    GpuMorph {
        deltas: deltas.map(|delta| {
            i16::try_from(delta).unwrap_or_else(|_| {
                debug_assert!(false, "adjacent surface LOD height delta exceeds i16");
                delta.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
            })
        }),
    }
}

fn pack_surface_morph_heights(bottom_delta: i32, top_delta: i32) -> GpuMorph {
    let (Some(bottom), Some(top)) = (bottom_delta.checked_mul(2), top_delta.checked_mul(2)) else {
        return GpuMorph::default();
    };
    pack_surface_morph_deltas_half_voxels([bottom, bottom, top, top])
}

#[cfg(test)]
fn unpack_surface_morph_delta_half_voxels(packed: GpuMorph, corner: usize) -> i32 {
    i32::from(packed.deltas[corner])
}

fn pack_surface_shape_deltas(deltas: [i32; 4]) -> u16 {
    let mut packed = 0_u16;
    for (corner, delta) in deltas.into_iter().enumerate() {
        if !(-4..=3).contains(&delta) {
            return 0;
        }
        packed |= ((delta as u16) & 0b111) << (corner * 3);
    }
    packed
}

fn surface_shading_height(tile: &SurfaceTileMesh, cell_x: i32, cell_z: i32) -> Option<i32> {
    let edge = voxels_world::SURFACE_SHADING_EDGE_SAMPLES as i32;
    let sample_x = cell_x.checked_add(1)?;
    let sample_z = cell_z.checked_add(1)?;
    if !(0..edge).contains(&sample_x) || !(0..edge).contains(&sample_z) {
        return None;
    }
    tile.shading
        .heights
        .get((sample_x + sample_z * edge) as usize)
        .copied()
}

/// Returns the common height for a coarse heightfield grid vertex.
///
/// The four surrounding cell-centre samples already travel with every surface tile for lighting.
/// Reusing them here makes gentle terrain continuous without requesting finer world data. The
/// one-cell shading halo gives neighboring patches and tiles the same world-space answer at shared
/// vertices; the shader independently fades displacement to zero at true LOD ownership cuts.
/// Steep height ranges stay voxel cliffs instead of being rounded away.
fn surface_shape_vertex_height(tile: &SurfaceTileMesh, grid_x: i32, grid_z: i32) -> Option<i32> {
    let tile_edge = voxels_world::SURFACE_TILE_EDGE_CELLS;
    if !(0..=tile_edge).contains(&grid_x) || !(0..=tile_edge).contains(&grid_z) {
        return None;
    }
    let heights = [
        surface_shading_height(tile, grid_x - 1, grid_z - 1)?,
        surface_shading_height(tile, grid_x, grid_z - 1)?,
        surface_shading_height(tile, grid_x, grid_z)?,
        surface_shading_height(tile, grid_x - 1, grid_z)?,
    ];
    let minimum = *heights.iter().min()?;
    let maximum = *heights.iter().max()?;
    // Three signed bits per corner keep the base GPU instance at 24 bytes. A six-voxel local
    // range covers the natural slopes where interpolation removes visible stairs; steeper relief
    // remains an intentional voxel cliff.
    let maximum_range = tile.coord.stride_voxels().saturating_mul(2).min(6);
    if maximum.saturating_sub(minimum) > maximum_range {
        return None;
    }
    let target = heights
        .into_iter()
        .map(i64::from)
        .sum::<i64>()
        .saturating_add(2)
        .div_euclid(4) as i32;
    heights
        .into_iter()
        .all(|height| (-4..=3).contains(&target.saturating_sub(height)))
        .then_some(target)
}

fn cached_surface_shape_vertex_height(
    heights: &[Option<i32>],
    grid_x: i32,
    grid_z: i32,
) -> Option<i32> {
    let edge = voxels_world::SURFACE_TILE_EDGE_CELLS + 1;
    if !(0..edge).contains(&grid_x) || !(0..edge).contains(&grid_z) {
        return None;
    }
    heights[(grid_x + grid_z * edge) as usize]
}

fn surface_top_shape(
    tile: &SurfaceTileMesh,
    shape_heights: &[Option<i32>],
    quad: &SurfaceQuad,
) -> u16 {
    let stride = tile.coord.stride_voxels();
    if quad.face != 2 || quad.extent != [stride as u16; 2] {
        return 0;
    }
    let [origin_x, origin_z] = tile.coord.voxel_origin();
    let local_x = i64::from(quad.origin[0]) - i64::from(origin_x);
    let local_z = i64::from(quad.origin[2]) - i64::from(origin_z);
    let span = i64::from(tile.coord.voxel_span());
    let stride_i64 = i64::from(stride);
    if local_x < 0
        || local_z < 0
        || local_x >= span
        || local_z >= span
        || local_x % stride_i64 != 0
        || local_z % stride_i64 != 0
    {
        return 0;
    }
    let cell_x = (local_x / stride_i64) as i32;
    let cell_z = (local_z / stride_i64) as i32;
    if surface_shading_height(tile, cell_x, cell_z) != Some(quad.origin[1]) {
        return 0;
    }
    let target_delta = |grid_x, grid_z| {
        cached_surface_shape_vertex_height(shape_heights, grid_x, grid_z)
            .map_or(0, |height| height.saturating_sub(quad.origin[1]))
    };
    pack_surface_shape_deltas([
        target_delta(cell_x, cell_z),
        target_delta(cell_x + 1, cell_z),
        target_delta(cell_x + 1, cell_z + 1),
        target_delta(cell_x, cell_z + 1),
    ])
}

fn surface_wall_shape(
    tile: &SurfaceTileMesh,
    shape_heights: &[Option<i32>],
    quad: &SurfaceQuad,
) -> u16 {
    let stride = tile.coord.stride_voxels();
    if !matches!(quad.face, 0 | 1 | 4 | 5)
        || i32::from(quad.extent[0]) != stride
        || quad.synthetic_fallback
    {
        return 0;
    }
    let [origin_x, origin_z] = tile.coord.voxel_origin();
    let own_x = quad.origin[0] - if quad.face == 0 { stride - 1 } else { 0 };
    let own_z = quad.origin[2] - if quad.face == 4 { stride - 1 } else { 0 };
    let local_x = i64::from(own_x) - i64::from(origin_x);
    let local_z = i64::from(own_z) - i64::from(origin_z);
    let span = i64::from(tile.coord.voxel_span());
    let stride_i64 = i64::from(stride);
    if local_x < 0
        || local_z < 0
        || local_x >= span
        || local_z >= span
        || local_x % stride_i64 != 0
        || local_z % stride_i64 != 0
    {
        return 0;
    }
    let cell_x = (local_x / stride_i64) as i32;
    let cell_z = (local_z / stride_i64) as i32;
    let (neighbor_x, neighbor_z) = match quad.face {
        0 => (cell_x + 1, cell_z),
        1 => (cell_x - 1, cell_z),
        4 => (cell_x, cell_z + 1),
        5 => (cell_x, cell_z - 1),
        _ => unreachable!(),
    };
    let (Some(own_height), Some(neighbor_height)) = (
        surface_shading_height(tile, cell_x, cell_z),
        surface_shading_height(tile, neighbor_x, neighbor_z),
    ) else {
        return 0;
    };
    let bottom_height = quad.origin[1].saturating_sub(1);
    let top_height = quad.origin[1]
        .saturating_add(i32::from(quad.extent[1]))
        .saturating_sub(1);
    if own_height <= neighbor_height
        || bottom_height < neighbor_height
        || top_height > own_height
        || bottom_height >= top_height
    {
        return 0;
    }
    let endpoints = match quad.face {
        0 => [(cell_x + 1, cell_z), (cell_x + 1, cell_z + 1)],
        1 => [(cell_x, cell_z), (cell_x, cell_z + 1)],
        4 => [(cell_x, cell_z + 1), (cell_x + 1, cell_z + 1)],
        5 => [(cell_x, cell_z), (cell_x + 1, cell_z)],
        _ => unreachable!(),
    };
    let endpoint = |index: usize| {
        cached_surface_shape_vertex_height(shape_heights, endpoints[index].0, endpoints[index].1)
    };
    let first = endpoint(0);
    let second = endpoint(1);
    pack_surface_shape_deltas([
        first.map_or(0, |height| height.saturating_sub(bottom_height)),
        second.map_or(0, |height| height.saturating_sub(bottom_height)),
        second.map_or(0, |height| height.saturating_sub(top_height)),
        first.map_or(0, |height| height.saturating_sub(top_height)),
    ])
}

#[cfg(test)]
fn surface_geometry_shapes(tile: &SurfaceTileMesh) -> Vec<u16> {
    surface_macro_normals_and_shapes(tile).1
}

fn surface_parent_height(tile: &SurfaceTileMesh, x: i32, z: i32) -> Option<i32> {
    if tile.shading.parent_heights.is_empty() {
        return None;
    }
    let [origin_x, origin_z] = tile.coord.voxel_origin();
    let parent_stride = tile.coord.stride_voxels().checked_mul(2)?;
    let sample_x = (i64::from(x) - i64::from(origin_x)).div_euclid(i64::from(parent_stride)) + 2;
    let sample_z = (i64::from(z) - i64::from(origin_z)).div_euclid(i64::from(parent_stride)) + 2;
    let edge = voxels_world::SURFACE_PARENT_SHADING_EDGE_SAMPLES as i64;
    if !(0..edge).contains(&sample_x) || !(0..edge).contains(&sample_z) {
        return None;
    }
    tile.shading
        .parent_heights
        .get((sample_x + sample_z * edge) as usize)
        .copied()
}

fn parent_shading_height(tile: &SurfaceTileMesh, cell_x: i32, cell_z: i32) -> Option<i32> {
    let edge = voxels_world::SURFACE_PARENT_SHADING_EDGE_SAMPLES as i32;
    let sample_x = cell_x.checked_add(2)?;
    let sample_z = cell_z.checked_add(2)?;
    if !(0..edge).contains(&sample_x) || !(0..edge).contains(&sample_z) {
        return None;
    }
    tile.shading
        .parent_heights
        .get((sample_x + sample_z * edge) as usize)
        .copied()
}

fn parent_shape_vertex_height(tile: &SurfaceTileMesh, grid_x: i32, grid_z: i32) -> Option<i32> {
    let parent_cells = voxels_world::SURFACE_TILE_EDGE_CELLS / 2;
    if !(-1..=parent_cells + 1).contains(&grid_x) || !(-1..=parent_cells + 1).contains(&grid_z) {
        return None;
    }
    let heights = [
        parent_shading_height(tile, grid_x - 1, grid_z - 1)?,
        parent_shading_height(tile, grid_x, grid_z - 1)?,
        parent_shading_height(tile, grid_x, grid_z)?,
        parent_shading_height(tile, grid_x - 1, grid_z)?,
    ];
    let minimum = *heights.iter().min()?;
    let maximum = *heights.iter().max()?;
    let parent_stride = tile.coord.stride_voxels().checked_mul(2)?;
    let maximum_range = parent_stride.saturating_mul(2).min(6);
    if maximum.saturating_sub(minimum) > maximum_range {
        return None;
    }
    let target = heights
        .into_iter()
        .map(i64::from)
        .sum::<i64>()
        .saturating_add(2)
        .div_euclid(4) as i32;
    heights
        .into_iter()
        .all(|height| (-4..=3).contains(&target.saturating_sub(height)))
        .then_some(target)
}

fn parent_cell_shape(tile: &SurfaceTileMesh, cell_x: i32, cell_z: i32) -> Option<(i32, u16)> {
    let height = parent_shading_height(tile, cell_x, cell_z)?;
    let delta = |grid_x, grid_z| {
        parent_shape_vertex_height(tile, grid_x, grid_z)
            .map_or(0, |vertex| vertex.saturating_sub(height))
    };
    Some((
        height,
        pack_surface_shape_deltas([
            delta(cell_x, cell_z),
            delta(cell_x + 1, cell_z),
            delta(cell_x + 1, cell_z + 1),
            delta(cell_x, cell_z + 1),
        ]),
    ))
}

/// Evaluates the exact next-coarser triangle mesh at a child-grid vertex. The result is expressed
/// in half-voxel units: child vertices can land at a parent edge midpoint, while four signed
/// 16-bit deltas retain that precision within the dedicated morph sidecar.
fn parent_surface_height_half_voxels(
    tile: &SurfaceTileMesh,
    cell_world_x: i32,
    cell_world_z: i32,
    vertex_world_x: i32,
    vertex_world_z: i32,
) -> Option<i32> {
    if tile.shading.parent_heights.is_empty() {
        return None;
    }
    let [origin_x, origin_z] = tile.coord.voxel_origin();
    let parent_stride = tile.coord.stride_voxels().checked_mul(2)?;
    let cell_x =
        (i64::from(cell_world_x) - i64::from(origin_x)).div_euclid(i64::from(parent_stride));
    let cell_z =
        (i64::from(cell_world_z) - i64::from(origin_z)).div_euclid(i64::from(parent_stride));
    let parent_cells = i64::from(voxels_world::SURFACE_TILE_EDGE_CELLS / 2);
    // Parent shading carries two cells of halo on every side. Boundary walls need the complete
    // adjacent halo cell to resolve both its shared edge and its triangle/shape decision;
    // rejecting that cell zeroed the wall's valid upper morph too and left a half-voxel crack.
    if !(-1..=parent_cells).contains(&cell_x) || !(-1..=parent_cells).contains(&cell_z) {
        return None;
    }
    let cell_x = cell_x as i32;
    let cell_z = cell_z as i32;
    let cell_origin_x =
        i64::from(origin_x).checked_add(i64::from(cell_x) * i64::from(parent_stride))?;
    let cell_origin_z =
        i64::from(origin_z).checked_add(i64::from(cell_z) * i64::from(parent_stride))?;
    let offset_x = i64::from(vertex_world_x).checked_sub(cell_origin_x)?;
    let offset_z = i64::from(vertex_world_z).checked_sub(cell_origin_z)?;
    let doubled_x = offset_x.checked_mul(2)?;
    let doubled_z = offset_z.checked_mul(2)?;
    if doubled_x % i64::from(parent_stride) != 0 || doubled_z % i64::from(parent_stride) != 0 {
        return None;
    }
    let u = i32::try_from(doubled_x / i64::from(parent_stride)).ok()?;
    let v = i32::try_from(doubled_z / i64::from(parent_stride)).ok()?;
    if !(0..=2).contains(&u) || !(0..=2).contains(&v) {
        return None;
    }
    let (height, shape) = parent_cell_shape(tile, cell_x, cell_z)?;
    let corners =
        std::array::from_fn::<_, 4, _>(|corner| unpack_surface_shape_delta(shape, corner));
    let flip = (corners[0] - corners[2]).abs() > (corners[1] - corners[3]).abs();
    let shaped_half_voxels = if flip {
        if u + v <= 2 {
            corners[0] * (2 - u - v) + corners[1] * u + corners[3] * v
        } else {
            corners[1] * (2 - v) + corners[2] * (u + v - 2) + corners[3] * (2 - u)
        }
    } else if v <= u {
        corners[0] * (2 - u) + corners[1] * (u - v) + corners[2] * v
    } else {
        corners[0] * (2 - v) + corners[2] * u + corners[3] * (v - u)
    };
    height.checked_mul(2)?.checked_add(shaped_half_voxels)
}

/// Resolves the exact next-coarser target for every generated terrain vertex. Top and wall corners
/// move independently, so the child shell converges to the same shaped parent triangles without
/// cracks, giant-block overlap, or a second geometry layer. Skyline proxies and outermost tiles do
/// not morph.
fn surface_geometry_morphs(
    tile: &SurfaceTileMesh,
    macro_normals: &[u32],
    geometry_shapes: &[u16],
) -> Vec<GpuMorph> {
    let stride = tile.coord.stride_voxels();
    tile.quads
        .iter()
        .zip(macro_normals)
        .zip(geometry_shapes)
        .map(|((quad, &macro_normal), &shape)| {
            if macro_normal & SURFACE_MACRO_NORMAL_FLAG == 0 {
                return GpuMorph::default();
            }
            if quad.face == 2 {
                let vertices = [
                    [quad.origin[0], quad.origin[2]],
                    [quad.origin[0].saturating_add(stride), quad.origin[2]],
                    [
                        quad.origin[0].saturating_add(stride),
                        quad.origin[2].saturating_add(stride),
                    ],
                    [quad.origin[0], quad.origin[2].saturating_add(stride)],
                ];
                let mut deltas = [0; 4];
                for (corner, [vertex_x, vertex_z]) in vertices.into_iter().enumerate() {
                    let Some(parent_height) = parent_surface_height_half_voxels(
                        tile,
                        quad.origin[0],
                        quad.origin[2],
                        vertex_x,
                        vertex_z,
                    ) else {
                        return GpuMorph::default();
                    };
                    let child_height =
                        quad.origin[1].saturating_add(unpack_surface_shape_delta(shape, corner));
                    deltas[corner] = parent_height.saturating_sub(child_height.saturating_mul(2));
                }
                return pack_surface_morph_deltas_half_voxels(deltas);
            }
            if !matches!(quad.face, 0 | 1 | 4 | 5) {
                return GpuMorph::default();
            }
            let own_x = quad.origin[0] - if quad.face == 0 { stride - 1 } else { 0 };
            let own_z = quad.origin[2] - if quad.face == 4 { stride - 1 } else { 0 };
            let (neighbor_x, neighbor_z) = match quad.face {
                0 => (own_x.saturating_add(stride), own_z),
                1 => (own_x.saturating_sub(stride), own_z),
                4 => (own_x, own_z.saturating_add(stride)),
                _ => (own_x, own_z.saturating_sub(stride)),
            };
            let endpoints = match quad.face {
                0 => [
                    [own_x.saturating_add(stride), own_z],
                    [own_x.saturating_add(stride), own_z.saturating_add(stride)],
                ],
                1 => [[own_x, own_z], [own_x, own_z.saturating_add(stride)]],
                4 => [
                    [own_x, own_z.saturating_add(stride)],
                    [own_x.saturating_add(stride), own_z.saturating_add(stride)],
                ],
                _ => [[own_x, own_z], [own_x.saturating_add(stride), own_z]],
            };
            let parent_at = |cell_x, cell_z, endpoint: [i32; 2]| {
                parent_surface_height_half_voxels(tile, cell_x, cell_z, endpoint[0], endpoint[1])
                    .and_then(|height| height.checked_add(2))
            };
            let (Some(parent_bottom_first), Some(parent_bottom_second)) = (
                parent_at(neighbor_x, neighbor_z, endpoints[0]),
                parent_at(neighbor_x, neighbor_z, endpoints[1]),
            ) else {
                return GpuMorph::default();
            };
            let (Some(parent_top_first), Some(parent_top_second)) = (
                parent_at(own_x, own_z, endpoints[0]),
                parent_at(own_x, own_z, endpoints[1]),
            ) else {
                return GpuMorph::default();
            };
            let current = [
                quad.origin[1].saturating_add(unpack_surface_shape_delta(shape, 0)),
                quad.origin[1].saturating_add(unpack_surface_shape_delta(shape, 1)),
                quad.origin[1]
                    .saturating_add(i32::from(quad.extent[1]))
                    .saturating_add(unpack_surface_shape_delta(shape, 2)),
                quad.origin[1]
                    .saturating_add(i32::from(quad.extent[1]))
                    .saturating_add(unpack_surface_shape_delta(shape, 3)),
            ];
            let target = [
                parent_bottom_first,
                parent_bottom_second,
                parent_top_second,
                parent_top_first,
            ];
            let deltas = std::array::from_fn(|corner| {
                target[corner].saturating_sub(current[corner].saturating_mul(2))
            });
            pack_surface_morph_deltas_half_voxels(deltas)
        })
        .collect()
}

fn surface_morph_closure_gpu_quads(
    tile: &SurfaceTileMesh,
    macro_normals: &[u32],
    horizon_profiles: &[u16],
) -> Vec<(GpuQuad, GpuMorph)> {
    let stride = tile.coord.stride_voxels();
    let attributes = tile
        .quads
        .iter()
        .zip(macro_normals)
        .zip(horizon_profiles)
        .filter_map(|((quad, &macro_normal), &horizon_profile)| {
            (quad.face == 2 && quad.extent == [stride as u16; 2]).then_some((
                (quad.origin[0], quad.origin[2]),
                (macro_normal, horizon_profile),
            ))
        })
        .collect::<HashMap<_, _>>();

    tile.morph_closures
        .iter()
        .map(|closure| {
            let quad = closure.quad;
            let preferred_cell = match quad.face {
                0 => [quad.origin[0].saturating_sub(stride - 1), quad.origin[2]],
                1 => [quad.origin[0], quad.origin[2]],
                4 => [quad.origin[0], quad.origin[2].saturating_sub(stride - 1)],
                5 => [quad.origin[0], quad.origin[2]],
                _ => unreachable!("morph closures are vertical faces"),
            };
            let fallback_cell = match quad.face {
                0 => [preferred_cell[0].saturating_add(stride), preferred_cell[1]],
                1 => [preferred_cell[0].saturating_sub(stride), preferred_cell[1]],
                4 => [preferred_cell[0], preferred_cell[1].saturating_add(stride)],
                5 => [preferred_cell[0], preferred_cell[1].saturating_sub(stride)],
                _ => unreachable!(),
            };
            let (macro_normal, horizon_profile) = attributes
                .get(&(preferred_cell[0], preferred_cell[1]))
                .or_else(|| attributes.get(&(fallback_cell[0], fallback_cell[1])))
                .copied()
                .unwrap_or((pack_surface_macro_normals(glam::Vec3::Y, glam::Vec3::Y), 0));
            let collapsed_plane = closure.collapsed_height.saturating_add(1);
            let static_bottom = quad.origin[1];
            let static_top = static_bottom.saturating_add(i32::from(quad.extent[1]));
            debug_assert_eq!(quad.extent[0] & MORPH_CLOSURE_EXTENT_FLAG, 0);
            (
                GpuQuad {
                    origin: quad.origin,
                    extent_voxels: [quad.extent[0] | MORPH_CLOSURE_EXTENT_FLAG, quad.extent[1]],
                    material_face: pack_gpu_source_material(
                        pack_surface_horizon_material(
                            pack_gpu_material_face(
                                u32::from(quad.material.id())
                                    | FAR_MATERIAL_FLAG
                                    | (u32::from(tile.coord.level.index()) << SURFACE_LOD_SHIFT),
                                quad.face,
                            ),
                            horizon_profile,
                        ),
                        GPU_SOURCE_SKYLINE_PROXY,
                    ),
                    ao: pack_surface_horizon_ao(macro_normal, horizon_profile),
                },
                pack_surface_morph_heights(
                    collapsed_plane.saturating_sub(static_bottom),
                    collapsed_plane.saturating_sub(static_top),
                ),
            )
        })
        .collect()
}

fn surface_exact_replacement_chunk(quad: &SurfaceQuad) -> Option<(i32, i32, i32)> {
    if !quad.synthetic_fallback {
        return None;
    }
    debug_assert!(matches!(quad.face, 0 | 1 | 4 | 5));
    debug_assert!(!quad.extent.contains(&0));
    let chunk_edge = CHUNK_EDGE as i32;
    let max = match quad.face {
        0 | 1 => [
            quad.origin[0],
            quad.origin[1].saturating_add(i32::from(quad.extent[1]) - 1),
            quad.origin[2].saturating_add(i32::from(quad.extent[0]) - 1),
        ],
        4 | 5 => [
            quad.origin[0].saturating_add(i32::from(quad.extent[0]) - 1),
            quad.origin[1].saturating_add(i32::from(quad.extent[1]) - 1),
            quad.origin[2],
        ],
        _ => return None,
    };
    let chunk = quad.origin.map(|value| value.div_euclid(chunk_edge));
    let max_chunk = max.map(|value| value.div_euclid(chunk_edge));
    debug_assert_eq!(
        chunk, max_chunk,
        "synthetic fallback spans must be partitioned on exact chunk boundaries"
    );
    Some((chunk[0], chunk[1], chunk[2]))
}

fn sampled_shading_normal(
    heights: &[i32],
    edge: usize,
    x: usize,
    z: usize,
    stride: i32,
) -> glam::Vec3 {
    debug_assert!(x > 0 && x + 1 < edge && z > 0 && z + 1 < edge);
    let height = |x: usize, z: usize| heights[x + z * edge];
    let slope_x = sampled_surface_slope(
        height(x, z),
        Some(height(x - 1, z)),
        Some(height(x + 1, z)),
        stride,
    );
    let slope_z = sampled_surface_slope(
        height(x, z),
        Some(height(x, z - 1)),
        Some(height(x, z + 1)),
        stride,
    );
    let horizontal = stabilized_surface_gradient(glam::Vec2::new(slope_x, slope_z));
    glam::Vec3::new(-horizontal.x, 1.0, -horizontal.y).normalize()
}

fn pack_surface_macro_normals(normal: glam::Vec3, parent: glam::Vec3) -> u32 {
    // Five bits per horizontal component are ample for the deliberately band-limited terrain
    // slopes. The freed four bits carry the high bit of each signed three-bit corner offset while
    // the high seven AO bits remain the parent-aware horizon profile.
    let encode = |component: f32| ((component.clamp(-1.0, 1.0) * 0.5 + 0.5) * 31.0).round() as u32;
    encode(normal.x)
        | (encode(normal.z) << 5)
        | (encode(parent.x) << 10)
        | (encode(parent.z) << 15)
        | SURFACE_MACRO_NORMAL_FLAG
}

fn surface_horizon_profiles(tile: &SurfaceTileMesh) -> Vec<u16> {
    let stride = tile.coord.stride_voxels();
    let span = tile.coord.voxel_span();
    let [origin_x, origin_z] = tile.coord.voxel_origin();
    let edge = voxels_world::SURFACE_TILE_EDGE_CELLS as usize;
    let mut heights = vec![None::<(i32, usize)>; edge * edge];
    for (quad_index, quad) in tile.quads.iter().enumerate() {
        let local_x = i64::from(quad.origin[0]) - i64::from(origin_x);
        let local_z = i64::from(quad.origin[2]) - i64::from(origin_z);
        let is_base_top = quad.face == 2
            && quad.extent == [stride as u16; 2]
            && local_x >= 0
            && local_z >= 0
            && local_x < i64::from(span)
            && local_z < i64::from(span)
            && local_x % i64::from(stride) == 0
            && local_z % i64::from(stride) == 0;
        if is_base_top {
            let cell_x = (local_x / i64::from(stride)) as usize;
            let cell_z = (local_z / i64::from(stride)) as usize;
            let cell = cell_x + cell_z * edge;
            if heights[cell].is_none() {
                heights[cell] = Some((quad.origin[1], quad_index));
            }
        }
    }

    let mut packed = vec![0_u16; tile.quads.len()];
    let mut cell_profiles = vec![0_u16; edge * edge];
    for z in 0..edge {
        for x in 0..edge {
            let Some((_, quad_index)) = heights[x + z * edge] else {
                continue;
            };
            let own = tile.shading.horizons[x + z * edge];
            let parent = if tile.shading.parent_horizons.is_empty() {
                own
            } else {
                let parent_edge = edge / 2;
                tile.shading.parent_horizons[x / 2 + z / 2 * parent_edge]
            };
            let profile = u16::from(own) | (u16::from(parent) << 8);
            packed[quad_index] = profile;
            cell_profiles[x + z * edge] = profile;
        }
    }

    // Use the same profile on generated terrain-body walls as on their top cell. Standalone
    // features keep profile zero, so trees and authored cliffs retain ordinary voxel lighting.
    for patch in &tile.patches {
        for range in std::iter::once(&patch.quad_range).chain(&patch.edge_ranges) {
            let start = range.start as usize;
            let end = range.end as usize;
            for (quad, packed_profile) in tile.quads[start..end]
                .iter()
                .copied()
                .zip(&mut packed[start..end])
            {
                if quad.face == 2 || i32::from(quad.extent[0]) != stride {
                    continue;
                }
                let adjusted_x = i64::from(quad.origin[0])
                    - if quad.face == 0 {
                        i64::from(stride - 1)
                    } else {
                        0
                    };
                let adjusted_z = i64::from(quad.origin[2])
                    - if quad.face == 4 {
                        i64::from(stride - 1)
                    } else {
                        0
                    };
                let local_x = adjusted_x - i64::from(origin_x);
                let local_z = adjusted_z - i64::from(origin_z);
                if local_x < 0
                    || local_z < 0
                    || local_x >= i64::from(span)
                    || local_z >= i64::from(span)
                    || local_x % i64::from(stride) != 0
                    || local_z % i64::from(stride) != 0
                {
                    continue;
                }
                let cell_x = (local_x / i64::from(stride)) as usize;
                let cell_z = (local_z / i64::from(stride)) as usize;
                let cell = cell_x + cell_z * edge;
                let Some((height, _)) = heights[cell] else {
                    continue;
                };
                let quad_top = i64::from(quad.origin[1]) + i64::from(quad.extent[1]);
                if quad_top == i64::from(height) + 1 {
                    *packed_profile = cell_profiles[cell];
                }
            }
        }
    }
    packed
}

fn surface_patch_profiles(
    tile: &SurfaceTileMesh,
    macro_normals: &[u32],
    horizon_profiles: &[u16],
    geometry_shapes: &[u16],
) -> Vec<(SurfacePatchId, SurfacePatchProfile)> {
    let stride = tile.coord.stride_voxels();
    let [tile_x, tile_z] = tile.coord.voxel_origin();
    let edge = voxels_world::SURFACE_PATCH_EDGE_CELLS as usize;
    tile.patches
        .iter()
        .filter_map(|patch| {
            let patch_id = SurfacePatchId::from_tile_cell_min(
                tile.coord,
                [patch.cell_bounds[0][0], patch.cell_bounds[0][1]],
            )?;
            let origin = [
                tile_x.saturating_add(i32::from(patch.cell_bounds[0][0]) * stride),
                tile_z.saturating_add(i32::from(patch.cell_bounds[0][1]) * stride),
            ];
            let mut cells = vec![None; edge * edge];
            for quad_index in patch.quad_range.clone() {
                let index = quad_index as usize;
                let quad = tile.quads[index];
                if quad.face != 2
                    || quad.extent != [stride as u16; 2]
                    || macro_normals[index] & SURFACE_MACRO_NORMAL_FLAG == 0
                {
                    continue;
                }
                let local_x = (quad.origin[0] - origin[0]).div_euclid(stride);
                let local_z = (quad.origin[2] - origin[1]).div_euclid(stride);
                if !(0..edge as i32).contains(&local_x) || !(0..edge as i32).contains(&local_z) {
                    continue;
                }
                cells[local_x as usize + local_z as usize * edge] = Some(SurfaceCell {
                    height: quad.origin[1],
                    parent_height: surface_parent_height(tile, quad.origin[0], quad.origin[2]),
                    material: quad.material,
                    macro_normal: macro_normals[index],
                    horizon_profile: horizon_profiles[index],
                    shape: geometry_shapes[index],
                });
            }
            Some((
                patch_id,
                SurfacePatchProfile {
                    origin,
                    stride,
                    cells,
                },
            ))
        })
        .collect()
}

fn canonical_chunk_profile(chunk: &Chunk) -> CanonicalChunkProfile {
    let edge = CHUNK_EDGE;
    let origin = chunk.coord().world_origin();
    let mut cells = vec![None; edge * edge];
    for local_z in 0..edge {
        for local_x in 0..edge {
            for local_y in (0..edge).rev() {
                let material = chunk.get(local_x, local_y, local_z);
                if material_belongs_to_surface_heightfield(material) {
                    cells[local_x + local_z * edge] = Some(SurfaceCell {
                        height: origin[1] + local_y as i32,
                        parent_height: None,
                        material,
                        macro_normal: 0xff,
                        horizon_profile: 0,
                        shape: 0,
                    });
                    break;
                }
            }
        }
    }
    CanonicalChunkProfile { cells }
}

const fn material_belongs_to_surface_heightfield(material: Material) -> bool {
    matches!(
        material,
        Material::Grass
            | Material::Dirt
            | Material::Stone
            | Material::Sand
            | Material::Snow
            | Material::Clay
            | Material::Basalt
            | Material::Moss
            | Material::Limestone
            | Material::RedSand
    )
}

fn canonical_ready_columns(ready_chunks: &HashSet<(i32, i32, i32)>) -> HashSet<(i32, i32)> {
    ready_chunks.iter().map(|&(x, _, z)| (x, z)).collect()
}

fn canonical_ready_chunks_for_focus(
    focus: Option<GeometricLodFocus>,
    ready_chunks: &HashSet<(i32, i32, i32)>,
) -> HashSet<(i32, i32, i32)> {
    let Some(focus) = focus else {
        return HashSet::new();
    };
    ready_chunks
        .iter()
        .copied()
        .filter(|&(x, _, z)| focus.owns_canonical_chunk(x, z))
        .collect()
}

fn canonical_surface_ready_chunks_for_focus(
    focus: Option<GeometricLodFocus>,
    ready_chunks: &HashSet<(i32, i32, i32)>,
) -> HashSet<(i32, i32, i32)> {
    let Some(focus) = focus else {
        return HashSet::new();
    };
    ready_chunks
        .iter()
        .copied()
        .filter(|&(x, _, z)| {
            matches!(
                focus.owner_at(
                    x * CHUNK_EDGE as i32 + CHUNK_EDGE as i32 / 2,
                    z * CHUNK_EDGE as i32 + CHUNK_EDGE as i32 / 2,
                ),
                crate::lod::LodOwner::Canonical
                    | crate::lod::LodOwner::Surface(SurfaceLodLevel::Stride2)
            )
        })
        .collect()
}

#[cfg(test)]
fn changed_canonical_ready_columns(
    previous: &HashSet<(i32, i32, i32)>,
    replacement: &HashSet<(i32, i32, i32)>,
) -> HashSet<(i32, i32)> {
    canonical_ready_columns(previous)
        .symmetric_difference(&canonical_ready_columns(replacement))
        .copied()
        .collect()
}

fn canonical_surface_cell_coverage(
    column: (i32, i32),
    ready_chunks: &HashSet<(i32, i32, i32)>,
) -> usize {
    if ready_chunks.iter().any(|&(x, _, z)| (x, z) == column) {
        CHUNK_EDGE * CHUNK_EDGE
    } else {
        0
    }
}

fn resolved_canonical_column_profile(
    profiles: &BTreeMap<i32, CanonicalChunkProfile>,
) -> CanonicalChunkProfile {
    let mut cells: Vec<Option<SurfaceCell>> = vec![None; CHUNK_EDGE * CHUNK_EDGE];
    for profile in profiles.values() {
        for (resolved, candidate) in cells.iter_mut().zip(&profile.cells) {
            if candidate.is_some_and(|candidate| {
                resolved.is_none_or(|resolved| candidate.height > resolved.height)
            }) {
                *resolved = *candidate;
            }
        }
    }
    CanonicalChunkProfile { cells }
}

fn canonical_surface_sample(
    profiles: &CanonicalColumnProfiles,
    x: i32,
    z: i32,
) -> Option<SurfaceCell> {
    let edge = CHUNK_EDGE as i32;
    let chunk_x = x.div_euclid(edge);
    let chunk_z = z.div_euclid(edge);
    let local_x = x.rem_euclid(edge) as usize;
    let local_z = z.rem_euclid(edge) as usize;
    profiles
        .get(&(chunk_x, chunk_z))?
        .values()
        .filter_map(|profile| profile.cells[local_x + local_z * CHUNK_EDGE])
        .max_by_key(|sample| sample.height)
}

fn build_lod_transitions(
    selection: &SurfacePatchSelection,
    surface_profiles: &HashMap<SurfacePatchId, SurfacePatchProfile>,
    canonical_profiles: &CanonicalColumnProfiles,
) -> LodTransitionBuild {
    let mut transitions = selection.transition_candidates().collect::<Vec<_>>();
    transitions.sort_unstable_by_key(|(patch, edge)| (*patch, edge.index()));
    let mut build = LodTransitionBuild {
        quads: Vec::with_capacity(transitions.len() * 16),
        morph_heights: Vec::with_capacity(transitions.len() * 16),
        ..LodTransitionBuild::default()
    };
    let mut connector_edges = BTreeMap::<(SurfacePatchId, u8), Vec<(GpuQuad, GpuMorph)>>::new();
    for (patch, edge) in transitions {
        let Some(coarse) = surface_profiles.get(&patch) else {
            build.incomplete_edges = build.incomplete_edges.saturating_add(1);
            continue;
        };
        let mut edge_quads = Vec::with_capacity(16);
        if append_lod_transition(
            &mut edge_quads,
            selection,
            surface_profiles,
            canonical_profiles,
            patch,
            edge,
            coarse,
        ) {
            connector_edges.insert((patch, edge.index() as u8), edge_quads);
        } else {
            build.incomplete_edges = build.incomplete_edges.saturating_add(1);
        }
    }
    let mut edges_by_patch = BTreeMap::<SurfacePatchId, u8>::new();
    for &(patch, edge) in connector_edges.keys() {
        edges_by_patch
            .entry(patch)
            .and_modify(|mask| *mask |= 1 << edge)
            .or_insert(1 << edge);
    }
    let mut stitched_tops = Vec::new();
    for (&patch, &exact_edge_mask) in &edges_by_patch {
        let Some(coarse) = surface_profiles.get(&patch) else {
            build.incomplete_edges = build
                .incomplete_edges
                .saturating_add(connector_edges.len() as u32);
            return build;
        };
        let Some(patch_stitches) =
            stitched_patch_top_quads(selection, patch, coarse, exact_edge_mask)
        else {
            build.incomplete_edges = build
                .incomplete_edges
                .saturating_add(connector_edges.len() as u32);
            return build;
        };
        stitched_tops.extend(patch_stitches);
    }
    for ((patch, encoded_edge), edge_quads) in connector_edges {
        build.exact_edges.insert((patch, encoded_edge));
        for (quad, morph_heights) in edge_quads {
            build.quads.push(quad);
            build.morph_heights.push(morph_heights);
        }
    }
    for mut quad in stitched_tops {
        quad.material_face =
            pack_gpu_source_material(quad.material_face, GPU_SOURCE_LOD_STITCH_TOP);
        build.quads.push(quad);
        build.morph_heights.push(GpuMorph::default());
    }
    build
}

fn transition_neighbor_stride(
    selection: &SurfacePatchSelection,
    patch: SurfacePatchId,
    point: [i32; 2],
) -> Option<i32> {
    if let Some(neighbor) = selection.selected_patch_at(point) {
        let stride = neighbor.level.stride_voxels();
        return (stride < patch.level.stride_voxels()).then_some(stride);
    }
    (patch.level == SurfaceLodLevel::Stride2).then_some(1)
}

fn opposite_surface_patch_edge(edge: SurfacePatchEdge) -> SurfacePatchEdge {
    match edge {
        SurfacePatchEdge::NegativeX => SurfacePatchEdge::PositiveX,
        SurfacePatchEdge::PositiveX => SurfacePatchEdge::NegativeX,
        SurfacePatchEdge::NegativeZ => SurfacePatchEdge::PositiveZ,
        SurfacePatchEdge::PositiveZ => SurfacePatchEdge::NegativeZ,
    }
}

fn unpack_surface_shape_delta(shape: u16, corner: usize) -> i32 {
    let bits = i32::from((shape >> (corner * 3)) & 0b111);
    if bits >= 4 { bits - 8 } else { bits }
}

fn rounded_ratio(numerator: i64, denominator: i64) -> i32 {
    debug_assert!(denominator > 0);
    let rounded = if numerator >= 0 {
        (numerator + denominator / 2) / denominator
    } else {
        (numerator - denominator / 2) / denominator
    };
    rounded.clamp(
        i64::from(SURFACE_SHAPE_MIN_DELTA_VOXELS),
        i64::from(SURFACE_SHAPE_MAX_DELTA_VOXELS),
    ) as i32
}

fn surface_shape_edge_endpoints(cell: SurfaceCell, edge: SurfacePatchEdge) -> [i32; 2] {
    let corner = |corner| unpack_surface_shape_delta(cell.shape, corner);
    match edge {
        SurfacePatchEdge::NegativeX => [corner(0), corner(3)],
        SurfacePatchEdge::PositiveX => [corner(1), corner(2)],
        SurfacePatchEdge::NegativeZ => [corner(0), corner(1)],
        SurfacePatchEdge::PositiveZ => [corner(3), corner(2)],
    }
}

fn interpolate_shape_edge(endpoints: [i32; 2], stride: i32, bounds: [i32; 2]) -> [i32; 2] {
    let interpolate = |offset: i32| {
        rounded_ratio(
            i64::from(endpoints[0]) * i64::from(stride - offset)
                + i64::from(endpoints[1]) * i64::from(offset),
            i64::from(stride),
        )
    };
    [interpolate(bounds[0]), interpolate(bounds[1])]
}

fn transition_triangle_shape(
    cell: SurfaceCell,
    edge: SurfacePatchEdge,
    anchor: usize,
    bounds: [i32; 2],
    stride: i32,
) -> u16 {
    let [mut boundary_start, mut boundary_end] =
        interpolate_shape_edge(surface_shape_edge_endpoints(cell, edge), stride, bounds);
    // The positive-X and negative-Z edges run in the A-B-C-D polygon direction as their tangent
    // increases. The other two run against it. Store all triangle vertices in the same polygon
    // winding so both the world and shadow pipelines can use one fixed strip order.
    if matches!(
        edge,
        SurfacePatchEdge::NegativeX | SurfacePatchEdge::PositiveZ
    ) {
        std::mem::swap(&mut boundary_start, &mut boundary_end);
    }
    pack_surface_shape_deltas([
        unpack_surface_shape_delta(cell.shape, anchor),
        boundary_start,
        boundary_end,
        boundary_end,
    ])
}

fn append_transition_top_fan(
    quads: &mut Vec<GpuQuad>,
    patch: SurfacePatchId,
    cell_origin: [i32; 2],
    cell: SurfaceCell,
    anchor: usize,
    edge: SurfacePatchEdge,
    segment_stride: i32,
) -> bool {
    let stride = patch.level.stride_voxels();
    if segment_stride <= 0
        || stride % segment_stride != 0
        || stride > i32::from(TRANSITION_TRIANGLE_OFFSET_MASK)
    {
        return false;
    }
    let encoded_material = u32::from(cell.material.id())
        | FAR_MATERIAL_FLAG
        | (u32::from(patch.level.index()) << SURFACE_LOD_SHIFT);
    for start in (0..stride).step_by(segment_stride as usize) {
        let end = start.saturating_add(segment_stride);
        let Ok(start) = u16::try_from(start) else {
            return false;
        };
        let Ok(end) = u16::try_from(end) else {
            return false;
        };
        let shape = transition_triangle_shape(
            cell,
            edge,
            anchor,
            [i32::from(start), i32::from(end)],
            stride,
        );
        quads.push(GpuQuad {
            origin: [cell_origin[0], cell.height, cell_origin[1]],
            extent_voxels: [
                start
                    | TRANSITION_TRIANGLE_FLAG
                    | ((anchor as u16) << TRANSITION_TRIANGLE_ANCHOR_SHIFT)
                    | ((edge.index() as u16) << TRANSITION_TRIANGLE_EDGE_SHIFT),
                end,
            ],
            material_face: pack_surface_horizon_material(
                pack_gpu_material_face(encoded_material, 2),
                cell.horizon_profile,
            ) | (u32::from(shape & 0xff) << SURFACE_SHAPE_MATERIAL_SHIFT),
            ao: pack_surface_horizon_ao(
                cell.macro_normal | (u32::from(shape >> 8) << SURFACE_SHAPE_AO_SHIFT),
                cell.horizon_profile,
            ),
        });
    }
    true
}

fn stitched_patch_top_quads(
    selection: &SurfacePatchSelection,
    patch: SurfacePatchId,
    coarse: &SurfacePatchProfile,
    exact_edge_mask: u8,
) -> Option<Vec<GpuQuad>> {
    let edge = voxels_world::SURFACE_PATCH_EDGE_CELLS;
    let stride = coarse.stride;
    let patch_span = patch.voxel_span();
    let mut quads = Vec::new();
    for cell_z in 0..edge {
        for cell_x in 0..edge {
            let mut cell_edge_mask = 0;
            if cell_x == 0 {
                cell_edge_mask |= 1 << SurfacePatchEdge::NegativeX.index();
            }
            if cell_x == edge - 1 {
                cell_edge_mask |= 1 << SurfacePatchEdge::PositiveX.index();
            }
            if cell_z == 0 {
                cell_edge_mask |= 1 << SurfacePatchEdge::NegativeZ.index();
            }
            if cell_z == edge - 1 {
                cell_edge_mask |= 1 << SurfacePatchEdge::PositiveZ.index();
            }
            let active_edges = cell_edge_mask & exact_edge_mask;
            if active_edges == 0 {
                continue;
            }
            let cell_origin = [
                coarse.origin[0].saturating_add(cell_x.saturating_mul(stride)),
                coarse.origin[1].saturating_add(cell_z.saturating_mul(stride)),
            ];
            let center = [
                cell_origin[0].saturating_add(stride / 2),
                cell_origin[1].saturating_add(stride / 2),
            ];
            let mut split_x = stride;
            let mut split_z = stride;
            for boundary_edge in SurfacePatchEdge::ALL {
                if active_edges & (1 << boundary_edge.index()) == 0 {
                    continue;
                }
                let across = match boundary_edge {
                    SurfacePatchEdge::NegativeX => [coarse.origin[0].saturating_sub(1), center[1]],
                    SurfacePatchEdge::PositiveX => {
                        [coarse.origin[0].saturating_add(patch_span), center[1]]
                    }
                    SurfacePatchEdge::NegativeZ => [center[0], coarse.origin[1].saturating_sub(1)],
                    SurfacePatchEdge::PositiveZ => {
                        [center[0], coarse.origin[1].saturating_add(patch_span)]
                    }
                };
                let fine_stride = transition_neighbor_stride(selection, patch, across)?;
                if stride % fine_stride != 0 {
                    return None;
                }
                match boundary_edge {
                    SurfacePatchEdge::NegativeX | SurfacePatchEdge::PositiveX => {
                        split_z = split_z.min(fine_stride);
                    }
                    SurfacePatchEdge::NegativeZ | SurfacePatchEdge::PositiveZ => {
                        split_x = split_x.min(fine_stride);
                    }
                }
            }
            let coarse_cell = coarse.sample_world(center[0], center[1])?;
            let active = SurfacePatchEdge::ALL
                .into_iter()
                .filter(|edge| active_edges & (1 << edge.index()) != 0)
                .collect::<Vec<_>>();
            let (anchor, fill_edge) = match active.as_slice() {
                [SurfacePatchEdge::NegativeX] => (1, Some(SurfacePatchEdge::PositiveZ)),
                [SurfacePatchEdge::PositiveX] => (0, Some(SurfacePatchEdge::PositiveZ)),
                [SurfacePatchEdge::NegativeZ] => (3, Some(SurfacePatchEdge::PositiveX)),
                [SurfacePatchEdge::PositiveZ] => (0, Some(SurfacePatchEdge::PositiveX)),
                [SurfacePatchEdge::NegativeX, SurfacePatchEdge::NegativeZ]
                | [SurfacePatchEdge::NegativeZ, SurfacePatchEdge::NegativeX] => (2, None),
                [SurfacePatchEdge::PositiveX, SurfacePatchEdge::NegativeZ]
                | [SurfacePatchEdge::NegativeZ, SurfacePatchEdge::PositiveX] => (3, None),
                [SurfacePatchEdge::PositiveX, SurfacePatchEdge::PositiveZ]
                | [SurfacePatchEdge::PositiveZ, SurfacePatchEdge::PositiveX] => (0, None),
                [SurfacePatchEdge::NegativeX, SurfacePatchEdge::PositiveZ]
                | [SurfacePatchEdge::PositiveZ, SurfacePatchEdge::NegativeX] => (1, None),
                _ => return None,
            };
            for edge in active {
                let segment_stride = match edge {
                    SurfacePatchEdge::NegativeX | SurfacePatchEdge::PositiveX => split_z,
                    SurfacePatchEdge::NegativeZ | SurfacePatchEdge::PositiveZ => split_x,
                };
                if !append_transition_top_fan(
                    &mut quads,
                    patch,
                    cell_origin,
                    coarse_cell,
                    anchor,
                    edge,
                    segment_stride,
                ) {
                    return None;
                }
            }
            if let Some(fill_edge) = fill_edge
                && !append_transition_top_fan(
                    &mut quads,
                    patch,
                    cell_origin,
                    coarse_cell,
                    anchor,
                    fill_edge,
                    stride,
                )
            {
                return None;
            }
        }
    }
    Some(quads)
}

fn for_each_fallback_surface_wall_run(
    lower_height: i32,
    upper_height: i32,
    surface_material: Material,
    mut emit: impl FnMut(i32, u16, Material),
) {
    let first_y = lower_height.saturating_add(1);
    if first_y > upper_height {
        return;
    }
    // The fallback stratification changes only in the shallow 1.6 m below the surface. Keep a
    // potentially enormous defensive remainder constant-time, then coalesce the shallow samples.
    let shallow_first_y = first_y.max(upper_height.saturating_sub(16));
    let mut emit_split = |mut y: i32, mut length: i64, material: Material| {
        while length > 0 {
            let extent = length.min(i64::from(u16::MAX)) as u16;
            emit(y, extent, material);
            length -= i64::from(extent);
            y = y.saturating_add(i32::from(extent));
        }
    };
    let mut run_start = first_y;
    let mut run_material = fallback_surface_wall_material(
        surface_material,
        i64::from(upper_height) - i64::from(run_start),
    );
    let mut y = if shallow_first_y > first_y {
        shallow_first_y
    } else {
        run_start.saturating_add(1)
    };
    while y <= upper_height {
        let material = fallback_surface_wall_material(
            surface_material,
            i64::from(upper_height) - i64::from(y),
        );
        if material != run_material {
            emit_split(run_start, i64::from(y) - i64::from(run_start), run_material);
            run_start = y;
            run_material = material;
        }
        if y == i32::MAX {
            break;
        }
        y += 1;
    }
    emit_split(
        run_start,
        i64::from(upper_height) - i64::from(run_start) + 1,
        run_material,
    );
}

fn append_lod_transition(
    quads: &mut Vec<(GpuQuad, GpuMorph)>,
    selection: &SurfacePatchSelection,
    surface_profiles: &HashMap<SurfacePatchId, SurfacePatchProfile>,
    canonical_profiles: &CanonicalColumnProfiles,
    patch: SurfacePatchId,
    edge: SurfacePatchEdge,
    coarse: &SurfacePatchProfile,
) -> bool {
    let coarse_stride = coarse.stride;
    let patch_span = patch.voxel_span();
    let mut tangent = 0;
    while tangent < patch_span {
        let neighbor_point = match edge {
            SurfacePatchEdge::NegativeX => [
                coarse.origin[0].saturating_sub(1),
                coarse.origin[1].saturating_add(tangent),
            ],
            SurfacePatchEdge::PositiveX => [
                coarse.origin[0].saturating_add(patch_span),
                coarse.origin[1].saturating_add(tangent),
            ],
            SurfacePatchEdge::NegativeZ => [
                coarse.origin[0].saturating_add(tangent),
                coarse.origin[1].saturating_sub(1),
            ],
            SurfacePatchEdge::PositiveZ => [
                coarse.origin[0].saturating_add(tangent),
                coarse.origin[1].saturating_add(patch_span),
            ],
        };
        let selected_fine_patch = selection.selected_patch_at(neighbor_point);
        let fine_stride = if let Some(fine_patch) = selected_fine_patch {
            fine_patch.level.stride_voxels()
        } else if patch.level == SurfaceLodLevel::Stride2 {
            1
        } else {
            return false;
        };
        if fine_stride >= coarse_stride
            || coarse_stride % fine_stride != 0
            || tangent.saturating_add(fine_stride) > patch_span
        {
            return false;
        }
        let tangent_sample = tangent + fine_stride / 2;
        let (coarse_x, coarse_z, fine_x, fine_z, outward_face, inward_face, boundary) = match edge {
            SurfacePatchEdge::NegativeX => (
                coarse.origin[0] + coarse_stride / 2,
                coarse.origin[1] + tangent_sample,
                coarse.origin[0] - fine_stride + fine_stride / 2,
                coarse.origin[1] + tangent_sample,
                1,
                0,
                [coarse.origin[0], coarse.origin[1] + tangent],
            ),
            SurfacePatchEdge::PositiveX => (
                coarse.origin[0] + patch_span - coarse_stride / 2,
                coarse.origin[1] + tangent_sample,
                coarse.origin[0] + patch_span + fine_stride / 2,
                coarse.origin[1] + tangent_sample,
                0,
                1,
                [coarse.origin[0] + patch_span, coarse.origin[1] + tangent],
            ),
            SurfacePatchEdge::NegativeZ => (
                coarse.origin[0] + tangent_sample,
                coarse.origin[1] + coarse_stride / 2,
                coarse.origin[0] + tangent_sample,
                coarse.origin[1] - fine_stride + fine_stride / 2,
                5,
                4,
                [coarse.origin[0] + tangent, coarse.origin[1]],
            ),
            SurfacePatchEdge::PositiveZ => (
                coarse.origin[0] + tangent_sample,
                coarse.origin[1] + patch_span - coarse_stride / 2,
                coarse.origin[0] + tangent_sample,
                coarse.origin[1] + patch_span + fine_stride / 2,
                4,
                5,
                [coarse.origin[0] + tangent, coarse.origin[1] + patch_span],
            ),
        };
        let Some(coarse_cell) = coarse.sample_world(coarse_x, coarse_z) else {
            return false;
        };
        let fine_point = [fine_x, fine_z];
        let (fine_cell, fine_parent_height) = if let Some(fine_patch) = selected_fine_patch {
            if selection.selected_patch_at(fine_point) != Some(fine_patch) {
                return false;
            }
            let fine_cell = surface_profiles
                .get(&fine_patch)
                .and_then(|profile| profile.sample_world(fine_x, fine_z));
            let Some(fine_cell) = fine_cell else {
                return false;
            };
            let parent_height = fine_cell.parent_height.or_else(|| {
                fine_patch
                    .parent()
                    .and_then(|parent| surface_profiles.get(&parent))
                    .and_then(|profile| profile.sample_world(fine_x, fine_z))
                    .map(|parent| parent.height)
            });
            (fine_cell, parent_height)
        } else if patch.level == SurfaceLodLevel::Stride2 {
            let Some(fine_cell) = canonical_surface_sample(canonical_profiles, fine_x, fine_z)
            else {
                return false;
            };
            (fine_cell, None)
        } else {
            return false;
        };
        let coarse_edge_shape = interpolate_shape_edge(
            surface_shape_edge_endpoints(coarse_cell, edge),
            coarse_stride,
            [
                tangent.rem_euclid(coarse_stride),
                tangent.rem_euclid(coarse_stride) + fine_stride,
            ],
        );
        let fine_edge_shape =
            surface_shape_edge_endpoints(fine_cell, opposite_surface_patch_edge(edge));
        let endpoint_order_reverses = {
            let difference = |endpoint: usize| {
                coarse_cell
                    .height
                    .saturating_add(coarse_edge_shape[endpoint])
                    .saturating_sub(fine_cell.height.saturating_add(fine_edge_shape[endpoint]))
            };
            let start = difference(0);
            let end = difference(1);
            (start < 0 && end > 0) || (start > 0 && end < 0)
        };
        if coarse_cell.height == fine_cell.height {
            if coarse_edge_shape != fine_edge_shape {
                let shape = pack_surface_shape_deltas([
                    fine_edge_shape[0],
                    fine_edge_shape[1],
                    coarse_edge_shape[1],
                    coarse_edge_shape[0],
                ]);
                let material = u32::from(coarse_cell.material.id())
                    | FAR_MATERIAL_FLAG
                    | (u32::from(patch.level.index()) << SURFACE_LOD_SHIFT);
                let origin_voxels = match outward_face {
                    0 => [
                        boundary[0].saturating_sub(1),
                        coarse_cell.height.saturating_add(1),
                        boundary[1],
                    ],
                    1 => [
                        boundary[0],
                        coarse_cell.height.saturating_add(1),
                        boundary[1],
                    ],
                    4 => [
                        boundary[0],
                        coarse_cell.height.saturating_add(1),
                        boundary[1].saturating_sub(1),
                    ],
                    5 => [
                        boundary[0],
                        coarse_cell.height.saturating_add(1),
                        boundary[1],
                    ],
                    _ => unreachable!(),
                };
                let mut quad = GpuQuad {
                    origin: origin_voxels,
                    extent_voxels: [fine_stride as u16, 0],
                    material_face: pack_surface_horizon_material(
                        pack_gpu_material_face(material, outward_face),
                        coarse_cell.horizon_profile,
                    ) | (u32::from(shape & 0xff) << SURFACE_SHAPE_MATERIAL_SHIFT),
                    ao: pack_surface_horizon_ao(
                        coarse_cell.macro_normal
                            | (u32::from(shape >> 8) << SURFACE_SHAPE_AO_SHIFT),
                        coarse_cell.horizon_profile,
                    ),
                };
                if endpoint_order_reverses {
                    quad.material_face = pack_gpu_source_material(
                        quad.material_face,
                        GPU_SOURCE_CROSSING_LOD_CONNECTOR,
                    );
                }
                quads.push((quad, GpuMorph::default()));
            }
            tangent += fine_stride;
            continue;
        }
        let (lower, upper, face, surface, lower_shape, upper_shape) =
            if coarse_cell.height > fine_cell.height {
                (
                    fine_cell.height,
                    coarse_cell.height,
                    outward_face,
                    coarse_cell,
                    fine_edge_shape,
                    coarse_edge_shape,
                )
            } else {
                (
                    coarse_cell.height,
                    fine_cell.height,
                    inward_face,
                    fine_cell,
                    coarse_edge_shape,
                    fine_edge_shape,
                )
            };
        let fine_level = SurfaceLodLevel::from_stride_voxels(fine_stride);
        let (encoded_level, transition_normal, transition_horizon, morph_heights) = if let (
            Some(fine_level),
            Some(fine_parent_height),
        ) =
            (fine_level, fine_parent_height)
        {
            // The fine endpoint morphs to the hidden parent sample on its own side of the
            // boundary, not to the selected coarse sample across the boundary. Adjacent
            // parent cells may have different heights; collapsing to the latter opened a
            // crack whenever Terrain Diffusion produced relief along an LOD cut.
            let fine_parent_delta = fine_parent_height.saturating_sub(fine_cell.height);
            let (bottom_delta, top_delta) = if coarse_cell.height > fine_cell.height {
                (fine_parent_delta, 0)
            } else {
                (0, fine_parent_delta)
            };
            (
                fine_level,
                fine_cell.macro_normal,
                fine_cell.horizon_profile,
                pack_surface_morph_heights(bottom_delta, top_delta),
            )
        } else {
            (
                patch.level,
                coarse_cell.macro_normal,
                coarse_cell.horizon_profile,
                GpuMorph::default(),
            )
        };
        for_each_fallback_surface_wall_run(
            lower,
            upper,
            surface.material,
            |y, vertical_extent, material| {
                let bottom_shape = if y == lower.saturating_add(1) {
                    lower_shape
                } else {
                    [0; 2]
                };
                let top_shape =
                    if y.saturating_add(i32::from(vertical_extent)) == upper.saturating_add(1) {
                        upper_shape
                    } else {
                        [0; 2]
                    };
                let shape = pack_surface_shape_deltas([
                    bottom_shape[0],
                    bottom_shape[1],
                    top_shape[1],
                    top_shape[0],
                ]);
                let origin_voxels = match face {
                    0 => [boundary[0].saturating_sub(1), y, boundary[1]],
                    1 => [boundary[0], y, boundary[1]],
                    4 => [boundary[0], y, boundary[1].saturating_sub(1)],
                    5 => [boundary[0], y, boundary[1]],
                    _ => unreachable!(),
                };
                let mut quad = GpuQuad {
                    origin: origin_voxels,
                    extent_voxels: [fine_stride as u16, vertical_extent],
                    material_face: pack_surface_horizon_material(
                        pack_gpu_material_face(
                            u32::from(material.id())
                                | FAR_MATERIAL_FLAG
                                | (u32::from(encoded_level.index()) << SURFACE_LOD_SHIFT),
                            face,
                        ),
                        transition_horizon,
                    ) | (u32::from(shape & 0xff) << SURFACE_SHAPE_MATERIAL_SHIFT),
                    // Between surface levels the connector follows the fine level's parent blend
                    // and collapses exactly as that shell reaches the coarse height. The canonical
                    // seam remains exact and static because canonical geometry has no sidecar.
                    ao: pack_surface_horizon_ao(
                        transition_normal | (u32::from(shape >> 8) << SURFACE_SHAPE_AO_SHIFT),
                        transition_horizon,
                    ),
                };
                if endpoint_order_reverses {
                    quad.material_face = pack_gpu_source_material(
                        quad.material_face,
                        GPU_SOURCE_CROSSING_LOD_CONNECTOR,
                    );
                }
                quads.push((quad, morph_heights));
            },
        );
        tangent += fine_stride;
    }
    true
}

fn gpu_quad_bounds(quads: &[GpuQuad]) -> Option<(glam::Vec3, glam::Vec3)> {
    let mut minimum = glam::Vec3::splat(f32::INFINITY);
    let mut maximum = glam::Vec3::splat(f32::NEG_INFINITY);
    for quad in quads {
        let face = (quad.material_face & GPU_FACE_MASK) >> GPU_FACE_SHIFT;
        let transition_triangle = quad.extent_voxels[0] & TRANSITION_TRIANGLE_FLAG != 0;
        let canonical_triangle = quad.extent_voxels[0] & CANONICAL_TRIANGLE_FLAG != 0;
        let extent = if transition_triangle {
            let level = (quad.material_face >> SURFACE_LOD_SHIFT) & 7;
            let stride = (2_u16).checked_shl(level).unwrap_or(u16::MAX);
            glam::Vec2::splat(f32::from(stride) * VOXEL_SIZE_METRES)
        } else if canonical_triangle {
            glam::Vec2::new(
                f32::from(unpack_canonical_triangle_extent(quad.extent_voxels[0]))
                    * VOXEL_SIZE_METRES,
                f32::from(unpack_canonical_triangle_extent(quad.extent_voxels[1]))
                    * VOXEL_SIZE_METRES,
            )
        } else {
            glam::Vec2::new(
                f32::from(quad.extent_voxels[0] & !MORPH_CLOSURE_EXTENT_FLAG) * VOXEL_SIZE_METRES,
                f32::from(quad.extent_voxels[1]) * VOXEL_SIZE_METRES,
            )
        };
        let size = match face {
            0 | 1 => glam::Vec3::new(VOXEL_SIZE_METRES, extent.y, extent.x),
            2 | 3 => glam::Vec3::new(extent.x, VOXEL_SIZE_METRES, extent.y),
            _ => glam::Vec3::new(extent.x, extent.y, VOXEL_SIZE_METRES),
        };
        let origin =
            glam::Vec3::from_array(quad.origin.map(|value| value as f32 * VOXEL_SIZE_METRES));
        let mut quad_minimum = origin;
        let mut quad_maximum = origin + size;
        let shape = ((quad.material_face >> SURFACE_SHAPE_MATERIAL_SHIFT) & 0xff)
            | (((quad.ao >> SURFACE_SHAPE_AO_SHIFT) & 0x0f) << 8);
        if shape != 0 {
            for corner in 0..4 {
                let vertical_corner = matches!(face, 0 | 1 | 4 | 5) && corner >= 2;
                let base_y = origin.y
                    + if face == 2 {
                        VOXEL_SIZE_METRES
                    } else if vertical_corner {
                        extent.y
                    } else {
                        0.0
                    };
                let shaped_y = base_y
                    + unpack_surface_shape_delta(shape as u16, corner) as f32 * VOXEL_SIZE_METRES;
                quad_minimum.y = quad_minimum.y.min(shaped_y);
                quad_maximum.y = quad_maximum.y.max(shaped_y);
            }
        }
        minimum = minimum.min(quad_minimum);
        maximum = maximum.max(quad_maximum);
    }
    minimum.is_finite().then_some((minimum, maximum))
}

fn stabilized_surface_gradient(mut gradient: glam::Vec2) -> glam::Vec2 {
    gradient *= SURFACE_MACRO_SLOPE_SCALE;
    let length = gradient.length();
    if length > SURFACE_MACRO_SLOPE_MAX {
        gradient *= SURFACE_MACRO_SLOPE_MAX / length;
    }
    gradient
}

fn sampled_surface_slope(
    center: i32,
    negative: Option<i32>,
    positive: Option<i32>,
    stride: i32,
) -> f32 {
    let delta = |from: i32, to: i32, distance: i32| {
        (i64::from(to) - i64::from(from)) as f32 / distance as f32
    };
    match (negative, positive) {
        (Some(negative), Some(positive)) => delta(negative, positive, 2 * stride),
        (Some(negative), None) => delta(negative, center, stride),
        (None, Some(positive)) => delta(center, positive, stride),
        (None, None) => 0.0,
    }
}

fn surface_patch_belongs_to_tile(patch: SurfacePatchId, tile: SurfaceTileCoord) -> bool {
    surface_tile_for_patch(patch) == tile
}

fn surface_tile_for_patch(patch: SurfacePatchId) -> SurfaceTileCoord {
    SurfaceTileCoord::new(
        patch.level,
        patch.x.div_euclid(SURFACE_PATCHES_PER_TILE_EDGE),
        patch.z.div_euclid(SURFACE_PATCHES_PER_TILE_EDGE),
    )
}

fn changed_surface_patch_profiles(
    tile: SurfaceTileCoord,
    previous: &HashMap<SurfacePatchId, SurfacePatchProfile>,
    replacement: &[(SurfacePatchId, SurfacePatchProfile)],
) -> HashSet<SurfacePatchId> {
    previous
        .iter()
        .filter_map(|(patch, profile)| {
            let replacement_profile =
                replacement
                    .iter()
                    .find_map(|(replacement_patch, replacement_profile)| {
                        (*replacement_patch == *patch).then_some(replacement_profile)
                    });
            (surface_patch_belongs_to_tile(*patch, tile) && replacement_profile != Some(profile))
                .then_some(*patch)
        })
        .chain(replacement.iter().filter_map(|(patch, profile)| {
            (previous.get(patch) != Some(profile)).then_some(*patch)
        }))
        .collect()
}

fn surface_profiles_affect_transition(
    selection: &SurfacePatchSelection,
    changed_profiles: &HashSet<SurfacePatchId>,
) -> bool {
    selection.transition_candidates().any(|(coarse, _)| {
        changed_profiles.contains(&coarse)
            || changed_profiles
                .iter()
                .any(|changed| changed.parent() == Some(coarse))
    })
}

fn surface_patch_mesh_key(patch: SurfacePatchId) -> MeshKey {
    (
        patch.level.index() + 1,
        patch.x.div_euclid(SURFACE_PATCHES_PER_TILE_EDGE),
        0,
        patch.z.div_euclid(SURFACE_PATCHES_PER_TILE_EDGE),
    )
}

fn changed_surface_lod_ownership_keys(
    previous: &LodDrawPlan,
    patches: &SurfacePatchSelection,
    exact_transition_edges: &HashSet<(SurfacePatchId, u8)>,
) -> HashSet<MeshKey> {
    let changed_patches = previous
        .patches
        .owned_patches()
        .filter(|patch| !patches.owns(*patch))
        .chain(
            patches
                .owned_patches()
                .filter(|patch| !previous.patches.owns(*patch)),
        );
    let changed_edges = previous
        .exact_transition_edges
        .symmetric_difference(exact_transition_edges)
        .map(|(patch, _)| *patch);
    changed_patches
        .chain(changed_edges)
        .map(surface_patch_mesh_key)
        .collect()
}

fn canonical_column_touches_patch_edge(
    column: (i32, i32),
    patch: SurfacePatchId,
    edge: SurfacePatchEdge,
) -> bool {
    let Some([[min_x, min_z], [max_x, max_z]]) = patch.voxel_bounds_xz() else {
        return false;
    };
    let chunk_edge = CHUNK_EDGE as i64;
    let column_min_x = i64::from(column.0) * chunk_edge;
    let column_max_x = column_min_x + chunk_edge;
    let column_min_z = i64::from(column.1) * chunk_edge;
    let column_max_z = column_min_z + chunk_edge;
    let contains_x = |x: i32| (column_min_x..column_max_x).contains(&i64::from(x));
    let contains_z = |z: i32| (column_min_z..column_max_z).contains(&i64::from(z));
    let overlaps_x = column_min_x < i64::from(max_x) && i64::from(min_x) < column_max_x;
    let overlaps_z = column_min_z < i64::from(max_z) && i64::from(min_z) < column_max_z;
    match edge {
        SurfacePatchEdge::NegativeX => min_x.checked_sub(1).is_some_and(contains_x) && overlaps_z,
        SurfacePatchEdge::PositiveX => contains_x(max_x) && overlaps_z,
        SurfacePatchEdge::NegativeZ => min_z.checked_sub(1).is_some_and(contains_z) && overlaps_x,
        SurfacePatchEdge::PositiveZ => contains_z(max_z) && overlaps_x,
    }
}

fn slice_owned_by_lod(
    focus: Option<GeometricLodFocus>,
    lod_draw_plan: Option<&LodDrawPlan>,
    key: &MeshKey,
    slice: &MeshSlice,
) -> bool {
    if *key == EXACT_VOLUME_FRONTIER_MESH_KEY {
        return true;
    }
    if focus.is_none() {
        return key.0 == 0;
    }
    let Some(plan) = lod_draw_plan else {
        return false;
    };
    if LOD_TRANSITION_MESH_KEYS.contains(key) {
        return plan.transition_mesh_key == Some(*key);
    }
    if key.0 == 0 {
        if slice.canonical_water_surface {
            return plan.owns_canonical_chunk(key);
        }
        return plan.owns_enclosed_view_chunk(key) || plan.owns_canonical_chunk(key);
    }
    if slice
        .exact_replacement_chunk
        .is_some_and(|coord| plan.owns_exact_volume_coord(coord))
    {
        return false;
    }
    let Some(level) = SurfaceLodLevel::ALL.get(usize::from(key.0 - 1)).copied() else {
        return false;
    };
    let Some(patch_id) = slice.surface_patch_id else {
        return false;
    };
    if patch_id.level != level {
        return false;
    }
    if slice.morph_closure {
        return plan.owns_patch(patch_id)
            && focus.is_some_and(|focus| surface_patch_intersects_morph_band(focus, patch_id));
    }
    if slice.stitch_edges != 0 {
        return plan.owns_patch(patch_id)
            && SurfacePatchEdge::ALL.into_iter().all(|edge| {
                slice.stitch_edges & (1 << edge.index()) == 0
                    || plan.owns_surface_top_edge(patch_id, edge)
            });
    }
    slice.boundary_edge.map_or_else(
        || plan.owns_patch(patch_id),
        |edge| plan.owns_boundary_wall_edge(patch_id, edge),
    )
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

fn surface_patch_intersects_morph_band(focus: GeometricLodFocus, patch: SurfacePatchId) -> bool {
    let boundary = usize::from(patch.level.index()) + 1;
    let boundary_half_extents = focus.boundary_half_extents();
    let Some(&half_extent) = boundary_half_extents.get(boundary) else {
        return false;
    };
    let Some([[min_x, min_z], [max_x, max_z]]) = patch.voxel_bounds_xz() else {
        return false;
    };
    let centre = focus.boundary_centres()[boundary];
    let maximum_axis_delta = [min_x, max_x]
        .into_iter()
        .map(|x| (i64::from(x) - i64::from(centre[0])).abs())
        .chain(
            [min_z, max_z]
                .into_iter()
                .map(|z| (i64::from(z) - i64::from(centre[1])).abs()),
        )
        .max()
        .unwrap_or(0);
    let nearest_axis_delta = |minimum: i32, maximum: i32, centre: i32| {
        if centre < minimum {
            i64::from(minimum) - i64::from(centre)
        } else if centre > maximum {
            i64::from(centre) - i64::from(maximum)
        } else {
            0
        }
    };
    let minimum_axis_delta = nearest_axis_delta(min_x, max_x, centre[0])
        .max(nearest_axis_delta(min_z, max_z, centre[1]));
    // The shader moves its morph field continuously with the camera while the single-owner cut
    // remains snapped. Hysteresis can hold a cut 5/8 of one snap step behind the camera. Cover the
    // union of every possible continuous ramp relative to that snapped centre: the cut itself is
    // the outer edge, and width + twice the lag is the conservative inner edge.
    let width = 16_i64.max((i64::from(half_extent) + 49) / 50);
    let maximum_lag = i64::from(LOD_BOUNDARY_SNAP[boundary])
        .saturating_mul(5)
        .div_euclid(8);
    maximum_axis_delta >= i64::from(half_extent) - width - maximum_lag * 2
        && minimum_axis_delta <= i64::from(half_extent)
}

fn slice_uses_geometry_morph(
    key: &MeshKey,
    focus: Option<GeometricLodFocus>,
    slice: &MeshSlice,
) -> bool {
    if LOD_TRANSITION_MESH_KEYS.contains(key) {
        return true;
    }
    let (Some(focus), Some(patch)) = (focus, slice.surface_patch_id) else {
        return false;
    };
    surface_patch_intersects_morph_band(focus, patch)
}

fn mesh_casts_directional_shadow(key: &MeshKey) -> bool {
    key.0 == 0 || key.0 <= SurfaceLodLevel::Stride16.index() + 1
}

fn active_geometric_lod_focus(
    focus: Option<GeometricLodFocus>,
    far_terrain: bool,
) -> Option<GeometricLodFocus> {
    focus.filter(|_| far_terrain)
}

fn coalesce_draw_items(mut items: Vec<DrawItem>) -> Vec<DrawSpan> {
    items.sort_unstable_by_key(|item| (item.page, item.offset));
    let mut spans: Vec<DrawSpan> = Vec::with_capacity(items.len());
    for item in items {
        if let Some(last) = spans.last_mut()
            && last.page == item.page
            && last.offset.checked_add(last.size) == Some(item.offset)
            && last.morph_page == item.morph_page
            && last.morph_page.is_none_or(|_| {
                last.quad_count
                    .checked_mul(size_of::<GpuMorph>() as u32)
                    .and_then(|size| last.morph_offset.checked_add(size))
                    == Some(item.morph_offset)
            })
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
            morph_page: item.morph_page,
            morph_offset: item.morph_offset,
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
    lod_focus: Option<GeometricLodFocus>,
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
        lod_options: [
            if geometry_source_debug { 1.0 } else { 0.0 },
            0.0,
            0.0,
            if lod_focus.is_some() { 1.0 } else { 0.0 },
        ],
        lod_boundary_centres: lod_boundary_centres_uniform(lod_focus),
        lod_boundary_half_extents: lod_boundary_half_extents_uniform(lod_focus),
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
    lod_focus: Option<GeometricLodFocus>,
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
        lod_options: [0.0, 0.0, 0.0, if lod_focus.is_some() { 1.0 } else { 0.0 }],
        lod_boundary_centres: lod_boundary_centres_uniform(lod_focus),
        lod_boundary_half_extents: lod_boundary_half_extents_uniform(lod_focus),
    }
}

fn lod_boundary_centres_uniform(lod_focus: Option<GeometricLodFocus>) -> [[f32; 4]; 4] {
    let boundary_centres = lod_focus.map_or([[0; 2]; 8], GeometricLodFocus::boundary_centres);
    std::array::from_fn(|pair| {
        let first = boundary_centres[pair * 2];
        let second = boundary_centres[pair * 2 + 1];
        [
            first[0] as f32 * VOXEL_SIZE_METRES,
            first[1] as f32 * VOXEL_SIZE_METRES,
            second[0] as f32 * VOXEL_SIZE_METRES,
            second[1] as f32 * VOXEL_SIZE_METRES,
        ]
    })
}

fn lod_boundary_half_extents_uniform(lod_focus: Option<GeometricLodFocus>) -> [[f32; 4]; 2] {
    let boundary_half_extents = lod_focus.map_or(
        LOD_BOUNDARY_HALF_EXTENTS,
        GeometricLodFocus::boundary_half_extents,
    );
    std::array::from_fn(|group| {
        std::array::from_fn(|index| {
            boundary_half_extents[group * 4 + index] as f32 * VOXEL_SIZE_METRES
        })
    })
}

fn gpu_cut_transition(
    phase: f32,
    role: f32,
    lod_focus: Option<GeometricLodFocus>,
) -> GpuCutTransition {
    GpuCutTransition {
        phase_role: [phase, role, 0.0, 0.0],
        lod_boundary_centres: lod_boundary_centres_uniform(lod_focus),
        lod_boundary_half_extents: lod_boundary_half_extents_uniform(lod_focus),
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

fn gpu_feedback_matches_cut(
    feedback: &GpuVirtualTerrainFeedback,
    cut: Option<&VirtualTerrainCut>,
) -> bool {
    let Some(cut) = cut else {
        return feedback.selected_pages.is_empty()
            && feedback.requested_pages.is_empty()
            && feedback.ownerless_roots == 0
            && !feedback.overflowed();
    };
    if feedback.submission_id == 0
        || feedback.overflowed()
        || feedback.oracle_fingerprint != cut.fingerprint
        || feedback.ownerless_roots != cut.ownerless_roots.len() as u32
        || feedback.compacted_pages != cut.selected_pages.len() as u32
        || cut.feedback_overflow
        || cut.selection_overflow
        || cut.traversal_overflow
    {
        return false;
    }
    let mut selected = feedback.selected_pages.clone();
    selected.sort_unstable();
    selected.dedup();
    let mut requested = feedback.requested_pages.clone();
    requested.sort_unstable();
    requested.dedup();
    let mut expected_requests = cut
        .requested_pages
        .iter()
        .map(|identity| identity.key)
        .collect::<Vec<_>>();
    expected_requests.sort_unstable();
    expected_requests.dedup();
    selected == cut.selected_pages && requested == expected_requests
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
    morph_geometry: bool,
    cut_transition: bool,
}

impl VoxelPipelineVariant {
    const fn new(material_detail: bool, spatial_ao: bool) -> Self {
        Self {
            material_detail,
            spatial_ao,
            morph_geometry: false,
            cut_transition: false,
        }
    }

    const fn morphing(mut self) -> Self {
        self.morph_geometry = true;
        self
    }

    const fn transition(mut self) -> Self {
        self.cut_transition = true;
        self
    }

    const fn morphing_transition(self) -> Self {
        self.morphing().transition()
    }
}

fn create_voxel_pipeline(
    device: &Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    variant: VoxelPipelineVariant,
) -> RenderPipeline {
    let constants = [
        (
            "MATERIAL_DETAIL",
            if variant.material_detail { 1.0 } else { 0.0 },
        ),
        (
            "CUT_TRANSITION",
            if variant.cut_transition { 1.0 } else { 0.0 },
        ),
    ];
    let fixed_buffers = [Some(quad_layout())];
    let morph_buffers = [Some(quad_layout()), Some(morph_height_layout())];
    pipeline(
        device,
        label,
        layout,
        shader,
        SCENE_FORMAT,
        if variant.morph_geometry {
            &morph_buffers
        } else {
            &fixed_buffers
        },
        PipelineOptions {
            vertex_entry: if variant.morph_geometry {
                if variant.cut_transition {
                    "vs_transition_morph"
                } else {
                    "vs_main_morph"
                }
            } else if variant.cut_transition {
                "vs_transition_fixed"
            } else {
                "vs_main_fixed"
            },
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
    let constants = [
        ("MATERIAL_DETAIL", if material_detail { 1.0 } else { 0.0 }),
        ("CUT_TRANSITION", 0.0),
    ];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_virtual_cluster"),
            buffers: &[Some(terrain_triangle_layout())],
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
            entry_point: Some("vs_virtual_cluster"),
            buffers: &[Some(terrain_triangle_layout())],
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
            entry_point: Some("vs_virtual_cluster"),
            buffers: &[Some(terrain_triangle_layout())],
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

fn virtual_triangle_diagnostic_pipeline(
    device: &Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
) -> RenderPipeline {
    let constants = [("MATERIAL_DETAIL", 0.0), ("CUT_TRANSITION", 0.0)];
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
    variant: VoxelPipelineVariant,
) -> RenderPipeline {
    let constants = [
        ("MATERIAL_DETAIL", 0.0),
        (
            "CUT_TRANSITION",
            if variant.cut_transition { 1.0 } else { 0.0 },
        ),
    ];
    let fixed_buffers = [Some(quad_layout()), Some(diagnostic_owner_layout())];
    let morph_buffers = [
        Some(quad_layout()),
        Some(diagnostic_owner_layout()),
        Some(diagnostic_morph_height_layout()),
    ];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(if variant.morph_geometry {
                if variant.cut_transition {
                    "vs_transition_morph_diagnostic"
                } else {
                    "vs_main_morph_diagnostic"
                }
            } else if variant.cut_transition {
                "vs_transition_fixed_diagnostic"
            } else {
                "vs_main_fixed_diagnostic"
            }),
            buffers: if variant.morph_geometry {
                &morph_buffers
            } else {
                &fixed_buffers
            },
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
    morph_geometry: bool,
) -> RenderPipeline {
    let fixed_buffers = [Some(quad_layout())];
    let morph_buffers = [Some(quad_layout()), Some(morph_height_layout())];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(if morph_geometry {
                "vs_main_morph"
            } else {
                "vs_main_fixed"
            }),
            buffers: if morph_geometry {
                &morph_buffers
            } else {
                &fixed_buffers
            },
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

fn transition_depth_pipeline(
    device: &Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    morph_geometry: bool,
) -> RenderPipeline {
    let fixed_buffers = [Some(quad_layout())];
    let morph_buffers = [Some(quad_layout()), Some(morph_height_layout())];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(if morph_geometry {
                "vs_transition_morph"
            } else {
                "vs_transition_fixed"
            }),
            buffers: if morph_geometry {
                &morph_buffers
            } else {
                &fixed_buffers
            },
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_depth_transition"),
            targets: &[],
            compilation_options: wgpu::PipelineCompilationOptions {
                constants: &[("CUT_TRANSITION", 1.0)],
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

fn shadow_caster_pipeline(
    device: &Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    morph_geometry: bool,
) -> RenderPipeline {
    let fixed_buffers = [Some(quad_layout())];
    let morph_buffers = [Some(quad_layout()), Some(morph_height_layout())];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(if morph_geometry {
                "vs_main_morph"
            } else {
                "vs_main_fixed"
            }),
            buffers: if morph_geometry {
                &morph_buffers
            } else {
                &fixed_buffers
            },
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
            entry_point: Some("vs_virtual_cluster"),
            buffers: &[Some(terrain_triangle_layout())],
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
        0 => Sint32x3,
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
    const ATTRIBUTES: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![3 => Uint32x2];
    wgpu::VertexBufferLayout {
        array_stride: size_of::<[u32; 2]>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &ATTRIBUTES,
    }
}

fn diagnostic_owner_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRIBUTES: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![4 => Uint32x2];
    wgpu::VertexBufferLayout {
        array_stride: size_of::<[u32; 2]>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &ATTRIBUTES,
    }
}

fn morph_height_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRIBUTES: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![4 => Sint16x4];
    wgpu::VertexBufferLayout {
        array_stride: size_of::<GpuMorph>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &ATTRIBUTES,
    }
}

fn diagnostic_morph_height_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRIBUTES: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![5 => Sint16x4];
    wgpu::VertexBufferLayout {
        array_stride: size_of::<GpuMorph>() as wgpu::BufferAddress,
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
    let mut surface_pages = manifest.surface_pages.clone();
    surface_pages.sort_unstable_by_key(|page| page.coord);
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
    let mut encoded = format!(
        r#"{{"surfaceEpoch":"{}","surfacePages":["#,
        manifest.surface_epoch
    );
    for (index, page) in surface_pages.iter().enumerate() {
        if index != 0 {
            encoded.push(',');
        }
        let _ = write!(
            encoded,
            concat!(
                r#"{{"key":"surface:{}:{}:{}","hierarchyDepth":{},"strideVoxels":{},"x":{},"z":{},"#,
                r#""residentRevision":{},"requestedRevision":{},"queued":{},"inFlight":{},"dirty":{}}}"#
            ),
            page.coord.stride_voxels(),
            page.coord.x,
            page.coord.z,
            page.coord.level.index(),
            page.coord.stride_voxels(),
            page.coord.x,
            page.coord.z,
            json_optional_u64(page.resident_revision),
            json_optional_u64(page.requested_revision),
            page.queued,
            page.in_flight,
            page.dirty,
        );
    }
    encoded.push_str("],\"canonicalPages\":[");
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

fn screenshot_cut_manifest_json(
    current: &LodDrawPlan,
    current_focus: Option<GeometricLodFocus>,
    transition: Option<&CutTransition>,
) -> String {
    let current = screenshot_cut_plan_json(current, current_focus);
    let outgoing = transition.map_or_else(
        || "null".to_owned(),
        |transition| screenshot_cut_plan_json(&transition.from, transition.from_focus),
    );
    format!(r#"{{"current":{current},"outgoing":{outgoing}}}"#)
}

fn screenshot_virtual_terrain_manifest_json(
    mode: VirtualTerrainRenderMode,
    resident: &BTreeMap<TerrainPageKey, VirtualTerrainGpuPage>,
    published_cut: Option<&VirtualTerrainCut>,
    oracle_cut: Option<&VirtualTerrainCut>,
    feedback: Option<&GpuVirtualTerrainFeedback>,
) -> String {
    let mode = match mode {
        VirtualTerrainRenderMode::Disabled => "disabled",
        VirtualTerrainRenderMode::Shadow => "shadow",
        VirtualTerrainRenderMode::Visible => "visible",
    };
    let published = screenshot_virtual_cut_json(published_cut);
    let oracle = screenshot_virtual_cut_json(oracle_cut);
    let mut encoded = format!(
        r#"{{"mode":"{mode}","publishedCut":{published},"oracleCut":{oracle},"residentPages":["#
    );
    for (index, (key, page)) in resident.iter().enumerate() {
        if index != 0 {
            encoded.push(',');
        }
        let _ = write!(
            encoded,
            concat!(
                r#"{{"level":{},"coord":{:?},"revision":"{}","contentFingerprint":"{}","#,
                r#""representation":"{}","representationKind":{}}}"#
            ),
            key.level,
            key.coord,
            page.revision,
            hex_bytes(&page.content_fingerprint),
            virtual_representation_label(page.representation),
            page.representation as u8,
        );
    }
    encoded.push_str("],\"gpuFeedback\":");
    if let Some(feedback) = feedback {
        let _ = write!(
            encoded,
            concat!(
                r#"{{"submissionId":"{}","oracleFingerprint":"{:016x}","#,
                r#""ownerlessRoots":{},"visitedNodes":{},"overflowFlags":{},"stackPeak":{},"#,
                r#""compactionOverflowFlags":{},"compactedPages":{},"#,
                r#""compactedOpaqueSurfaceElements":{},"compactedOpaqueTriangleElements":{},"#,
                r#""compactedWaterSurfaceElements":{},"compactedWaterTriangleElements":{},"#,
                r#""selectedPages":["#
            ),
            feedback.submission_id,
            feedback.oracle_fingerprint,
            feedback.ownerless_roots,
            feedback.visited_nodes,
            feedback.overflow_flags,
            feedback.stack_peak,
            feedback.compaction_overflow_flags,
            feedback.compacted_pages,
            feedback.compacted_surface_elements,
            feedback.compacted_triangle_elements,
            feedback.compacted_water_surface_elements,
            feedback.compacted_water_triangle_elements,
        );
        write_virtual_page_keys(&mut encoded, &feedback.selected_pages);
        encoded.push_str("],\"requestedPages\":[");
        write_virtual_page_keys(&mut encoded, &feedback.requested_pages);
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
            r#""selectionOverflow":{},"traversalOverflow":{},"incoherentReplacementGroups":{},"#,
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
    }
}

fn screenshot_cut_plan_json(plan: &LodDrawPlan, focus: Option<GeometricLodFocus>) -> String {
    let mut patches = plan.patches.owned_patches().collect::<Vec<_>>();
    patches.sort_unstable();
    let mut canonical_columns = plan.canonical_columns.iter().copied().collect::<Vec<_>>();
    canonical_columns.sort_unstable();
    let mut canonical_chunks = plan.canonical_chunks.iter().copied().collect::<Vec<_>>();
    canonical_chunks.sort_unstable();
    let mut enclosed_chunks = plan
        .enclosed_view_chunks
        .iter()
        .copied()
        .collect::<Vec<_>>();
    enclosed_chunks.sort_unstable();
    let mut transition_edges = plan
        .exact_transition_edges
        .iter()
        .copied()
        .collect::<Vec<_>>();
    transition_edges.sort_unstable();
    let focus = focus.map_or_else(
        || "null".to_owned(),
        |focus| {
            format!(
                r#"{{"boundaryCentresVoxels":{:?},"boundaryHalfExtentsVoxels":{:?}}}"#,
                focus.boundary_centres(),
                focus.boundary_half_extents(),
            )
        },
    );
    let mut encoded = format!(
        concat!(
            r#"{{"focus":{},"ownerCounts":{{"surfacePatches":{},"canonicalColumns":{},"#,
            r#""canonicalChunks":{},"enclosedViewChunks":{},"transitionEdges":{},"#,
            r#""ownerlessVisibleSamples":null,"conflictingVisibleSamples":null}},"surfacePatches":["#
        ),
        focus,
        patches.len(),
        canonical_columns.len(),
        canonical_chunks.len(),
        enclosed_chunks.len(),
        transition_edges.len(),
    );
    for (index, patch) in patches.iter().enumerate() {
        if index != 0 {
            encoded.push(',');
        }
        let _ = write!(
            encoded,
            r#"{{"key":"surface-patch:{}:{}:{}","hierarchyDepth":{},"strideVoxels":{},"x":{},"z":{}}}"#,
            patch.level.stride_voxels(),
            patch.x,
            patch.z,
            patch.level.index(),
            patch.level.stride_voxels(),
            patch.x,
            patch.z,
        );
    }
    encoded.push_str("],\"canonicalColumns\":[");
    write_pairs(&mut encoded, &canonical_columns);
    encoded.push_str("],\"canonicalChunks\":[");
    write_triples(&mut encoded, &canonical_chunks);
    encoded.push_str("],\"enclosedViewChunks\":[");
    write_triples(&mut encoded, &enclosed_chunks);
    encoded.push_str("],\"transitionEdges\":[");
    for (index, (patch, edge)) in transition_edges.iter().enumerate() {
        if index != 0 {
            encoded.push(',');
        }
        let _ = write!(
            encoded,
            r#"["surface-patch:{}:{}:{}",{}]"#,
            patch.level.stride_voxels(),
            patch.x,
            patch.z,
            edge,
        );
    }
    let transition_mesh = plan.transition_mesh_key.map_or_else(
        || "null".to_owned(),
        |key| format!("[{},{},{},{}]", key.0, key.1, key.2, key.3),
    );
    let _ = write!(
        encoded,
        r#"],"incompleteTransitionEdges":{},"transitionMeshKey":{}}}"#,
        plan.incomplete_transition_edges, transition_mesh,
    );
    encoded
}

fn write_pairs(destination: &mut String, values: &[(i32, i32)]) {
    for (index, (x, z)) in values.iter().enumerate() {
        if index != 0 {
            destination.push(',');
        }
        let _ = write!(destination, "[{x},{z}]");
    }
}

fn write_triples(destination: &mut String, values: &[(i32, i32, i32)]) {
    for (index, (x, y, z)) in values.iter().enumerate() {
        if index != 0 {
            destination.push(',');
        }
        let _ = write!(destination, "[{x},{y},{z}]");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lod_cut_publication_is_atomic() {
        assert_eq!(CUT_TRANSITION_SECONDS, 0.0);
        assert!(!cut_transition_is_active(Some(10.0), 10.0));
        assert!(!cut_transition_is_active(None, 10.1));
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
                r#"{"surfaceEpoch":"0","surfacePages":[],"canonicalPages":[],"virtualRegions":["#,
                r#"{"key":"virtual:3:-2:3:4","level":3,"x":-2,"y":3,"z":4,"minimumRevision":"17","registered":true,"inFlight":false}],"#,
                r#""virtualStream":{"pendingPages":5,"inFlightPages":6,"obsoleteInFlightPages":2,"cancelledPendingPages":"7","usefulBytes":"8","#,
                r#""cancellationWasteBytes":"9","failedPages":"10","cachePages":11,"cacheBytes":"12"}}"#
            )
        );
    }

    #[test]
    fn screenshot_cut_manifest_is_stable_across_hash_insertion_order() {
        let focus = GeometricLodFocus::snapped(0, 0);
        let first = SurfacePatchId::new(SurfaceLodLevel::Stride2, -1, 2);
        let second = SurfacePatchId::new(SurfaceLodLevel::Stride4, 3, -4);
        let mut left = LodDrawPlan::default();
        left.patches.rebuild(
            focus,
            &[first, second].into_iter().collect(),
            &HashSet::new(),
        );
        let mut right = LodDrawPlan::default();
        right.patches.rebuild(
            focus,
            &[second, first].into_iter().collect(),
            &HashSet::new(),
        );
        assert_eq!(
            screenshot_cut_plan_json(&left, Some(focus)),
            screenshot_cut_plan_json(&right, Some(focus))
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
                mesh: VirtualTerrainGpuMesh::Empty,
            },
        );
        let cut = VirtualTerrainCut {
            selected_pages: vec![key],
            requested_pages: Vec::new(),
            ownerless_roots: Vec::new(),
            fingerprint: 0x1234,
            visited_nodes: 1,
            selected_primitives: 2,
            selected_encoded_bytes: 3,
            feedback_overflow: false,
            selection_overflow: false,
            traversal_overflow: false,
            incoherent_replacement_groups: 0,
        };
        let feedback = GpuVirtualTerrainFeedback {
            submission_id: 8,
            oracle_fingerprint: cut.fingerprint,
            selected_pages: vec![key],
            compacted_pages: 1,
            ..GpuVirtualTerrainFeedback::default()
        };
        let manifest = screenshot_virtual_terrain_manifest_json(
            VirtualTerrainRenderMode::Visible,
            &resident,
            Some(&cut),
            Some(&cut),
            Some(&feedback),
        );
        assert!(manifest.contains(r#""mode":"visible""#));
        assert!(manifest.contains(r#""coord":[-2, 3, 4]"#));
        assert!(manifest.contains(r#""revision":"17""#));
        assert!(manifest.contains(r#""representation":"sparseVoxelBrick""#));
        assert!(manifest.contains(r#""submissionId":"8""#));
        assert!(manifest.contains(r#""oracleFingerprint":"0000000000001234""#));
        assert!(manifest.contains(&"ab".repeat(32)));
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
        let material_face =
            pack_gpu_material_face(u32::from(Material::Stone.id()) | FAR_MATERIAL_FLAG, 5);
        let quad = GpuQuad {
            origin: [0; 3],
            extent_voxels: [u16::MAX, u16::MAX],
            material_face,
            ao: 0,
        };
        let tagged = GpuQuad {
            material_face: pack_gpu_source_material(quad.material_face, GPU_SOURCE_LOD_CONNECTOR),
            ..quad
        };
        assert_eq!(
            (tagged.material_face >> GPU_SOURCE_SHIFT) & 7,
            GPU_SOURCE_LOD_CONNECTOR
        );
        assert_eq!(tagged.material_face & !GPU_SOURCE_MASK, material_face);
        assert_eq!(tagged.extent_voxels, quad.extent_voxels);
    }

    fn flat_patch_profile(patch: SurfacePatchId, height: i32) -> SurfacePatchProfile {
        flat_patch_profile_with_parent(patch, height, None)
    }

    fn flat_patch_profile_with_parent(
        patch: SurfacePatchId,
        height: i32,
        parent_height: Option<i32>,
    ) -> SurfacePatchProfile {
        SurfacePatchProfile {
            origin: patch.voxel_bounds_xz().unwrap()[0],
            stride: patch.level.stride_voxels(),
            cells: vec![
                Some(SurfaceCell {
                    height,
                    parent_height,
                    material: Material::Stone,
                    macro_normal: pack_surface_macro_normals(glam::Vec3::Y, glam::Vec3::Y),
                    horizon_profile: 0,
                    shape: 0,
                });
                (voxels_world::SURFACE_PATCH_EDGE_CELLS.pow(2)) as usize
            ],
        }
    }

    #[test]
    fn lod_connector_fallback_keeps_surface_cover_one_voxel_deep() {
        let mut runs = Vec::new();
        for_each_fallback_surface_wall_run(0, 20, Material::Grass, |y, extent, material| {
            runs.push((y, extent, material));
        });
        assert_eq!(
            runs,
            [
                (1, 9, Material::Stone),
                (10, 10, Material::Dirt),
                (20, 1, Material::Grass),
            ]
        );
        assert!(runs.iter().all(|(_, extent, material)| {
            !matches!(material, Material::Grass | Material::Moss | Material::Snow) || *extent == 1
        }));
    }

    fn counts(entries: &[(Material, u64)]) -> [u64; Material::ALL.len()] {
        let mut counts = [0; Material::ALL.len()];
        for &(material, count) in entries {
            counts[usize::from(material.id())] = count;
        }
        counts
    }

    #[test]
    fn distant_surface_normals_encode_macro_slope_without_growing_quads() {
        let coord = SurfaceTileCoord::new(SurfaceLodLevel::Stride2, 0, 0);
        let tile = voxels_world::generate_surface_tile_mesh_with(coord, |x, _| {
            (x.div_euclid(2), Material::Grass)
        });
        let packed = surface_macro_normals(&tile);
        let quad_index = tile
            .quads
            .iter()
            .position(|quad| quad.origin == [2, 1, 0] && quad.face == 2)
            .expect("interior terrain top exists");
        let value = packed[quad_index];
        assert_ne!(value & SURFACE_MACRO_NORMAL_FLAG, 0);
        let normal_x = (value & 31) as f32 * (2.0 / 31.0) - 1.0;
        let normal_z = ((value >> 5) & 31) as f32 * (2.0 / 31.0) - 1.0;
        assert!(
            (-0.23..-0.18).contains(&normal_x),
            "uphill +X must retain a gentle, stable tilt toward -X: {normal_x}"
        );
        assert!(normal_z.abs() < 0.04);
        let side_index = tile
            .quads
            .iter()
            .position(|quad| quad.origin[0] == 2 && quad.origin[2] == 0 && quad.face == 1)
            .expect("uphill cell has a generated negative-X terrain wall");
        assert_eq!(
            packed[side_index], value,
            "terrain wall shares its cell's macro normal"
        );
        assert_eq!(size_of::<GpuQuad>(), 24);
    }

    #[test]
    fn coarse_surface_shapes_share_exact_interpolated_corners_without_extra_quads() {
        let coord = SurfaceTileCoord::new(SurfaceLodLevel::Stride4, 0, 0);
        let tile = voxels_world::generate_surface_tile_mesh_with(coord, |x, z| {
            (x.div_euclid(4) + z.div_euclid(8), Material::Grass)
        });
        let shapes = surface_geometry_shapes(&tile);
        assert_eq!(shapes.len(), tile.quads.len());
        let top = |origin: [i32; 3]| {
            tile.quads
                .iter()
                .position(|quad| quad.origin == origin && quad.face == 2)
                .expect("terrain top exists")
        };
        let left = top([4, 1, 4]);
        let right = top([8, 2, 4]);
        let signed_corner = |shape: u16, corner: usize| {
            let bits = i32::from((shape >> (corner * 3)) & 0b111);
            if bits >= 4 { bits - 8 } else { bits }
        };
        let left_high = tile.quads[left].origin[1] + signed_corner(shapes[left], 1);
        let left_low = tile.quads[left].origin[1] + signed_corner(shapes[left], 2);
        let right_high = tile.quads[right].origin[1] + signed_corner(shapes[right], 0);
        let right_low = tile.quads[right].origin[1] + signed_corner(shapes[right], 3);
        assert_eq!([left_high, left_low], [right_high, right_low]);
        assert!(
            shapes.iter().any(|shape| *shape != 0),
            "gentle coarse relief must use the existing quad vertices for interpolation"
        );
        assert_eq!(tile.quads.len(), shapes.len());
    }

    #[test]
    fn coarse_surface_shapes_preserve_cliffs_and_stride_two_voxels() {
        let cliff = voxels_world::generate_surface_tile_mesh_with(
            SurfaceTileCoord::new(SurfaceLodLevel::Stride4, 0, 0),
            |x, _| (if x < 8 { 0 } else { 100 }, Material::Stone),
        );
        let cliff_shapes = surface_geometry_shapes(&cliff);
        let cliff_top = cliff
            .quads
            .iter()
            .position(|quad| quad.origin == [4, 0, 4] && quad.face == 2)
            .expect("top before cliff");
        assert_eq!(
            (cliff_shapes[cliff_top] >> 3) & 0b111,
            0,
            "a steep shared corner stays on its voxel height"
        );

        let slope = voxels_world::generate_surface_tile_mesh_with(
            SurfaceTileCoord::new(SurfaceLodLevel::Stride4, 0, 0),
            |x, _| (x.div_euclid(4), Material::Grass),
        );
        let slope_shapes = surface_geometry_shapes(&slope);
        let left_patch_top = slope
            .quads
            .iter()
            .position(|quad| quad.origin == [28, 7, 4] && quad.face == 2)
            .expect("top beside patch boundary");
        let right_patch_top = slope
            .quads
            .iter()
            .position(|quad| quad.origin == [32, 8, 4] && quad.face == 2)
            .expect("top across patch boundary");
        let signed_corner = |shape: u16, corner: usize| {
            let bits = i32::from((shape >> (corner * 3)) & 0b111);
            if bits >= 4 { bits - 8 } else { bits }
        };
        assert_eq!(
            slope.quads[left_patch_top].origin[1] + signed_corner(slope_shapes[left_patch_top], 1),
            slope.quads[right_patch_top].origin[1]
                + signed_corner(slope_shapes[right_patch_top], 0),
            "internal patch ownership must not create a geometric step"
        );

        let neighbor = voxels_world::generate_surface_tile_mesh_with(
            SurfaceTileCoord::new(SurfaceLodLevel::Stride4, 1, 0),
            |x, _| (x.div_euclid(4), Material::Grass),
        );
        let neighbor_shapes = surface_geometry_shapes(&neighbor);
        let left_tile_top = slope
            .quads
            .iter()
            .position(|quad| quad.origin == [124, 31, 4] && quad.face == 2)
            .expect("top beside tile boundary");
        let right_tile_top = neighbor
            .quads
            .iter()
            .position(|quad| quad.origin == [128, 32, 4] && quad.face == 2)
            .expect("top across tile boundary");
        assert_eq!(
            slope.quads[left_tile_top].origin[1] + signed_corner(slope_shapes[left_tile_top], 1),
            neighbor.quads[right_tile_top].origin[1]
                + signed_corner(neighbor_shapes[right_tile_top], 0),
            "the shading halo must give neighboring tiles the same shared vertex"
        );

        let stride_two = voxels_world::generate_surface_tile_mesh_with(
            SurfaceTileCoord::new(SurfaceLodLevel::Stride2, 0, 0),
            |x, _| (x.div_euclid(2), Material::Grass),
        );
        assert!(
            surface_geometry_shapes(&stride_two)
                .into_iter()
                .all(|shape| shape == 0),
            "the nearest fallback remains literal twenty-centimetre voxels"
        );
    }

    #[test]
    fn coarse_surface_patch_bounds_cover_every_shaped_vertex_delta() {
        let coarse = voxels_world::generate_surface_tile_mesh_with(
            SurfaceTileCoord::new(SurfaceLodLevel::Stride4, 0, 0),
            |x, z| (x.div_euclid(4) + z.div_euclid(8), Material::Grass),
        );
        let patch = &coarse.patches[0];
        let (minimum, maximum) = surface_patch_render_bounds(patch, coarse.coord.level);
        assert_eq!(
            minimum.y,
            (patch.bounds.min[1] + SURFACE_SHAPE_MIN_DELTA_VOXELS) as f32 * VOXEL_SIZE_METRES
        );
        assert_eq!(
            maximum.y,
            (patch.bounds.max[1] + SURFACE_SHAPE_MAX_DELTA_VOXELS) as f32 * VOXEL_SIZE_METRES
        );
        assert_eq!(minimum.x, patch.bounds.min[0] as f32 * VOXEL_SIZE_METRES);
        assert_eq!(maximum.z, patch.bounds.max[2] as f32 * VOXEL_SIZE_METRES);

        let nearest = voxels_world::generate_surface_tile_mesh_with(
            SurfaceTileCoord::new(SurfaceLodLevel::Stride2, 0, 0),
            |x, _| (x.div_euclid(2), Material::Grass),
        );
        let nearest_patch = &nearest.patches[0];
        let (nearest_minimum, nearest_maximum) =
            surface_patch_render_bounds(nearest_patch, nearest.coord.level);
        assert_eq!(
            nearest_minimum,
            glam::Vec3::from_array(
                nearest_patch
                    .bounds
                    .min
                    .map(|value| value as f32 * VOXEL_SIZE_METRES)
            )
        );
        assert_eq!(
            nearest_maximum,
            glam::Vec3::from_array(
                nearest_patch
                    .bounds
                    .max
                    .map(|value| value as f32 * VOXEL_SIZE_METRES)
            )
        );
    }

    #[test]
    fn adjacent_surface_cells_morph_their_shared_vertex_to_the_exact_same_parent_height() {
        let coord = SurfaceTileCoord::new(SurfaceLodLevel::Stride2, 0, 0);
        let tile =
            voxels_world::generate_surface_tile_mesh_with(coord, |x, _| (x, Material::Grass));
        let (macro_normals, geometry_shapes) = surface_macro_normals_and_shapes(&tile);
        let morphs = surface_geometry_morphs(&tile, &macro_normals, &geometry_shapes);
        let resolved_height = |origin: [i32; 3], corner: usize| {
            let index = tile
                .quads
                .iter()
                .position(|quad| quad.origin == origin && quad.face == 2)
                .expect("terrain top exists");
            let packed = morphs[index];
            let child_height = tile.quads[index].origin[1]
                .saturating_add(unpack_surface_shape_delta(geometry_shapes[index], corner));
            child_height * 2 + unpack_surface_morph_delta_half_voxels(packed, corner)
        };
        assert_eq!(resolved_height([0, 1, 0], 1), 4);
        assert_eq!(resolved_height([2, 3, 0], 0), 4);
    }

    #[test]
    fn every_morphed_surface_wall_endpoint_matches_the_shared_parent_top() {
        let coord = SurfaceTileCoord::new(SurfaceLodLevel::Stride2, 0, 0);
        let surface = |x: i32, z: i32| {
            (
                x.div_euclid(3)
                    .saturating_add(z.div_euclid(5))
                    .saturating_add(if x >= 0 { 7 } else { 0 }),
                Material::Grass,
            )
        };
        let tile = voxels_world::generate_surface_tile_mesh_with(coord, surface);
        let (macro_normals, geometry_shapes) = surface_macro_normals_and_shapes(&tile);
        let morphs = surface_geometry_morphs(&tile, &macro_normals, &geometry_shapes);
        let mut parent_tops = BTreeMap::<(i32, i32), std::collections::BTreeSet<i32>>::new();
        for tile_z in -1..=1 {
            for tile_x in -1..=1 {
                let neighbor = voxels_world::generate_surface_tile_mesh_with(
                    SurfaceTileCoord::new(SurfaceLodLevel::Stride2, tile_x, tile_z),
                    surface,
                );
                let (neighbor_normals, neighbor_shapes) =
                    surface_macro_normals_and_shapes(&neighbor);
                let neighbor_morphs =
                    surface_geometry_morphs(&neighbor, &neighbor_normals, &neighbor_shapes);
                for (index, quad) in neighbor
                    .quads
                    .iter()
                    .enumerate()
                    .filter(|(_, quad)| quad.face == 2)
                {
                    let corners = [
                        [quad.origin[0], quad.origin[2]],
                        [quad.origin[0] + i32::from(quad.extent[0]), quad.origin[2]],
                        [
                            quad.origin[0] + i32::from(quad.extent[0]),
                            quad.origin[2] + i32::from(quad.extent[1]),
                        ],
                        [quad.origin[0], quad.origin[2] + i32::from(quad.extent[1])],
                    ];
                    for (corner, [x, z]) in corners.into_iter().enumerate() {
                        let current = quad.origin[1]
                            + 1
                            + unpack_surface_shape_delta(neighbor_shapes[index], corner);
                        let target = current * 2
                            + unpack_surface_morph_delta_half_voxels(
                                neighbor_morphs[index],
                                corner,
                            );
                        parent_tops.entry((x, z)).or_default().insert(target);
                    }
                }
            }
        }
        for (index, quad) in tile.quads.iter().enumerate().filter(|(_, quad)| {
            matches!(quad.face, 0 | 1 | 4 | 5) && quad.extent[0] == coord.stride_voxels() as u16
        }) {
            let width = i32::from(quad.extent[0]);
            let endpoints = match quad.face {
                0 => [
                    [quad.origin[0] + 1, quad.origin[2]],
                    [quad.origin[0] + 1, quad.origin[2] + width],
                ],
                1 => [
                    [quad.origin[0], quad.origin[2]],
                    [quad.origin[0], quad.origin[2] + width],
                ],
                4 => [
                    [quad.origin[0], quad.origin[2] + 1],
                    [quad.origin[0] + width, quad.origin[2] + 1],
                ],
                5 => [
                    [quad.origin[0], quad.origin[2]],
                    [quad.origin[0] + width, quad.origin[2]],
                ],
                _ => unreachable!(),
            };
            for corner in 0..4 {
                let endpoint = endpoints[usize::from(corner == 1 || corner == 2)];
                let current = quad.origin[1]
                    + if corner >= 2 {
                        i32::from(quad.extent[1])
                    } else {
                        0
                    }
                    + unpack_surface_shape_delta(geometry_shapes[index], corner);
                let target =
                    current * 2 + unpack_surface_morph_delta_half_voxels(morphs[index], corner);
                let tops = parent_tops
                    .get(&(endpoint[0], endpoint[1]))
                    .expect("neighboring top target exists");
                assert!(
                    tops.contains(&target),
                    "wall face {} corner {corner} target {target} misses tops {tops:?} at ({}, {}); quad {quad:?}, shape {}, morph {:?}",
                    quad.face,
                    endpoint[0],
                    endpoint[1],
                    geometry_shapes[index],
                    morphs[index],
                );
            }
        }
    }

    #[test]
    fn parent_only_steps_are_explicit_quads_collapsed_onto_the_child_surface() {
        let coord = SurfaceTileCoord::new(SurfaceLodLevel::Stride2, 0, 0);
        let child = |_x, _z| (10, Material::Grass);
        let parent = |x, _z| {
            if x >= 4 {
                (12, Material::Grass)
            } else {
                (10, Material::Grass)
            }
        };
        let tile = voxels_world::generate_surface_tile_mesh_with_features_and_shading(
            coord,
            child,
            child,
            parent,
            &[],
        );
        let macro_normals = surface_macro_normals(&tile);
        let horizons = surface_horizon_profiles(&tile);
        let gpu = surface_morph_closure_gpu_quads(&tile, &macro_normals, &horizons);

        assert_eq!(gpu.len(), 32);
        for (quad, morph_heights) in gpu {
            assert_ne!(quad.extent_voxels[0] & MORPH_CLOSURE_EXTENT_FLAG, 0);
            assert_eq!(quad.extent_voxels[0] & !MORPH_CLOSURE_EXTENT_FLAG, 2);
            assert_eq!(quad.extent_voxels[1], 2);
            let bottom_delta = unpack_surface_morph_delta_half_voxels(morph_heights, 0);
            let top_delta = unpack_surface_morph_delta_half_voxels(morph_heights, 2);
            assert_eq!(bottom_delta, 0);
            assert_eq!(top_delta, -4);
            assert_eq!(quad.origin[1] * 2 + bottom_delta, 22);
            assert_eq!(
                (quad.origin[1] + i32::from(quad.extent_voxels[1])) * 2 + top_delta,
                22
            );
        }
    }

    #[test]
    fn collapsed_parent_step_quads_are_drawn_only_inside_the_morph_band() {
        let focus = GeometricLodFocus::snapped(0, 0);
        let inner = SurfacePatchId::new(SurfaceLodLevel::Stride2, 4, 0);
        let boundary = SurfacePatchId::new(
            SurfaceLodLevel::Stride2,
            LOD_BOUNDARY_HALF_EXTENTS[1]
                / SurfacePatchId::new(SurfaceLodLevel::Stride2, 0, 0).voxel_span()
                - 2,
            0,
        );
        let beyond_boundary = SurfacePatchId::new(
            SurfaceLodLevel::Stride2,
            LOD_BOUNDARY_HALF_EXTENTS[1]
                / SurfacePatchId::new(SurfaceLodLevel::Stride2, 0, 0).voxel_span()
                + 8,
            0,
        );
        let beyond_corner = SurfacePatchId::new(
            SurfaceLodLevel::Stride2,
            LOD_BOUNDARY_HALF_EXTENTS[1]
                / SurfacePatchId::new(SurfaceLodLevel::Stride2, 0, 0).voxel_span()
                - 2,
            LOD_BOUNDARY_HALF_EXTENTS[1]
                / SurfacePatchId::new(SurfaceLodLevel::Stride2, 0, 0).voxel_span()
                + 8,
        );
        let outermost = SurfacePatchId::new(SurfaceLodLevel::Stride256, 0, 0);
        assert!(!surface_patch_intersects_morph_band(focus, inner));
        assert!(surface_patch_intersects_morph_band(focus, boundary));
        assert!(!surface_patch_intersects_morph_band(focus, beyond_boundary));
        assert!(!surface_patch_intersects_morph_band(focus, beyond_corner));
        assert!(!surface_patch_intersects_morph_band(focus, outermost));
    }

    #[test]
    fn visible_and_shadow_passes_share_exact_lod_boundaries() {
        let focus = GeometricLodFocus::snapped(1_614, 294);
        let packed = lod_boundary_centres_uniform(Some(focus));
        for (index, expected) in focus.boundary_centres().into_iter().enumerate() {
            let pair = packed[index / 2];
            let actual = if index % 2 == 0 {
                [pair[0], pair[1]]
            } else {
                [pair[2], pair[3]]
            };
            assert_eq!(
                actual,
                [
                    expected[0] as f32 * VOXEL_SIZE_METRES,
                    expected[1] as f32 * VOXEL_SIZE_METRES,
                ]
            );
        }
        assert_eq!(lod_boundary_centres_uniform(None), [[0.0; 4]; 4]);
        let expected = std::array::from_fn(|group| {
            std::array::from_fn(|entry| {
                LOD_BOUNDARY_HALF_EXTENTS[group * 4 + entry] as f32 * VOXEL_SIZE_METRES
            })
        });
        assert_eq!(lod_boundary_half_extents_uniform(Some(focus)), expected);
    }

    #[test]
    fn cut_uniforms_freeze_outgoing_and_follow_current_incoming_lod_coordinates() {
        let previous = GeometricLodFocus::snapped(1_614, 294);
        let outgoing = gpu_cut_transition(0.25, 1.0, Some(previous));
        assert_eq!(outgoing.phase_role, [0.25, 1.0, 0.0, 0.0]);
        assert_eq!(
            outgoing.lod_boundary_centres,
            lod_boundary_centres_uniform(Some(previous))
        );
        assert_eq!(
            outgoing.lod_boundary_half_extents,
            lod_boundary_half_extents_uniform(Some(previous))
        );
        let current = GeometricLodFocus::snapped(1_742, 422);
        let incoming = gpu_cut_transition(0.75, 2.0, Some(current));
        assert_eq!(incoming.phase_role, [0.75, 2.0, 0.0, 0.0]);
        assert_eq!(
            incoming.lod_boundary_centres,
            lod_boundary_centres_uniform(Some(current))
        );
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
    fn surface_horizons_distinguish_open_ground_from_a_coarse_valley() {
        let coord = SurfaceTileCoord::new(SurfaceLodLevel::Stride16, 0, 0);
        let flat =
            voxels_world::generate_surface_tile_mesh_with(coord, |_, _| (12, Material::Grass));
        assert!(
            surface_horizon_profiles(&flat)
                .into_iter()
                .all(|value| value == 0)
        );

        let valley = voxels_world::generate_surface_tile_mesh_with(coord, |x, z| {
            (
                ((x - 168).abs() + (z - 168).abs()).div_euclid(2),
                Material::Grass,
            )
        });
        let quad_index = valley
            .quads
            .iter()
            .position(|quad| quad.origin[0] == 160 && quad.origin[2] == 160 && quad.face == 2)
            .expect("valley-floor terrain top exists");
        let profile = surface_horizon_profiles(&valley)[quad_index];
        assert_eq!(
            profile & 0xff,
            0xaa,
            "all four fine horizons rise about 27 degrees"
        );
        assert_ne!(
            profile >> 8,
            0,
            "the parent horizon remains available for LOD morphing"
        );
    }

    #[test]
    fn transition_mesh_bounds_include_actual_shaped_vertices() {
        let shape = pack_surface_shape_deltas([-4, 3, 3, -4]);
        let quad = GpuQuad {
            origin: [10, 20, 30],
            extent_voxels: [8, 8],
            material_face: pack_gpu_material_face(u32::from(Material::Stone.id()), 2)
                | (u32::from(shape & 0xff) << SURFACE_SHAPE_MATERIAL_SHIFT),
            ao: u32::from(shape >> 8) << SURFACE_SHAPE_AO_SHIFT,
        };
        let (minimum, maximum) = gpu_quad_bounds(&[quad]).expect("one quad has finite bounds");
        assert!(minimum.abs_diff_eq(glam::vec3(1.0, 1.7, 3.0), 1e-5));
        assert!(maximum.abs_diff_eq(glam::vec3(1.8, 2.4, 3.8), 1e-5));
    }

    #[test]
    fn surface_horizon_bits_round_trip_alongside_geometry_morphs() {
        let base_material = u32::from(Material::Stone.id())
            | FAR_MATERIAL_FLAG
            | (u32::from(SurfaceLodLevel::Stride16.index()) << SURFACE_LOD_SHIFT);
        for profile in [0_u16, 0x00ff, 0xa55a, u16::MAX] {
            let material_face =
                pack_surface_horizon_material(pack_gpu_material_face(base_material, 5), profile);
            let ao = pack_surface_horizon_ao(
                pack_surface_macro_normals(glam::Vec3::Y, glam::Vec3::Y),
                profile,
            );
            let unpacked = ((material_face >> SURFACE_HORIZON_MATERIAL_LOW_SHIFT) & 0xff)
                | (((material_face >> SURFACE_HORIZON_MATERIAL_HIGH_SHIFT) & 1) << 8)
                | (((ao >> SURFACE_HORIZON_AO_SHIFT) & 0x7f) << 9);
            assert_eq!(unpacked, u32::from(profile));
            assert_eq!(material_face & 0xffff, u32::from(Material::Stone.id()));
            assert_eq!((material_face & GPU_FACE_MASK) >> GPU_FACE_SHIFT, 5);
            assert_eq!((material_face >> SURFACE_LOD_SHIFT) & 7, 3);
            assert_ne!(ao & SURFACE_MACRO_NORMAL_FLAG, 0);
        }
        assert_eq!(size_of::<GpuQuad>(), 24);
    }

    #[test]
    fn surface_shape_bits_round_trip_without_changing_material_or_horizon() {
        let deltas = [-4, -1, 2, 3];
        let shape = pack_surface_shape_deltas(deltas);
        let profile = 0xa55a;
        let base_material = u32::from(Material::Stone.id())
            | FAR_MATERIAL_FLAG
            | (u32::from(SurfaceLodLevel::Stride16.index()) << SURFACE_LOD_SHIFT);
        let material_face =
            pack_surface_horizon_material(pack_gpu_material_face(base_material, 5), profile)
                | (u32::from(shape & 0xff) << SURFACE_SHAPE_MATERIAL_SHIFT);
        let ao = pack_surface_horizon_ao(
            pack_surface_macro_normals(glam::Vec3::Y, glam::Vec3::Y)
                | (u32::from(shape >> 8) << SURFACE_SHAPE_AO_SHIFT),
            profile,
        );
        let unpacked_shape = ((material_face >> SURFACE_SHAPE_MATERIAL_SHIFT) & 0xff)
            | (((ao >> SURFACE_SHAPE_AO_SHIFT) & 0x0f) << 8);
        assert_eq!(unpacked_shape, u32::from(shape));
        for (corner, expected) in deltas.into_iter().enumerate() {
            let bits = ((unpacked_shape >> (corner * 3)) & 0b111) as i32;
            let decoded = if bits >= 4 { bits - 8 } else { bits };
            assert_eq!(decoded, expected);
        }
        let unpacked_profile = ((material_face >> SURFACE_HORIZON_MATERIAL_LOW_SHIFT) & 0xff)
            | (((material_face >> SURFACE_HORIZON_MATERIAL_HIGH_SHIFT) & 1) << 8)
            | (((ao >> SURFACE_HORIZON_AO_SHIFT) & 0x7f) << 9);
        assert_eq!(unpacked_profile, u32::from(profile));
        assert_eq!(material_face & 0xff, u32::from(Material::Stone.id()));
        assert_eq!((material_face & GPU_FACE_MASK) >> GPU_FACE_SHIFT, 5);
        assert_eq!((material_face >> SURFACE_LOD_SHIFT) & 7, 3);
        assert_ne!(ao & SURFACE_MACRO_NORMAL_FLAG, 0);
        assert_eq!(size_of::<GpuQuad>(), 24);
    }

    #[test]
    fn canonical_heightfield_profile_uses_ground_beneath_standalone_geometry() {
        let mut chunk = Chunk::empty(ChunkCoord::new(4, 3, -2));
        chunk.set(7, 5, 11, Material::Grass);
        for y in 6..CHUNK_EDGE {
            chunk.set(7, y, 11, Material::Wood);
        }
        let profile = canonical_chunk_profile(&chunk);
        let sample = profile.cells[7 + 11 * CHUNK_EDGE].expect("terrain surface sample");
        assert_eq!(sample.height, 3 * CHUNK_EDGE as i32 + 5);
        assert_eq!(sample.material, Material::Grass);
    }

    #[test]
    fn streamed_heightfield_profile_uses_ground_beneath_aligned_proxy_caps() {
        let coord = SurfaceTileCoord::new(SurfaceLodLevel::Stride4, 0, 0);
        let mut tile =
            voxels_world::generate_surface_tile_mesh_with(coord, |_, _| (10, Material::Grass));
        let patch_index = tile.patches.len() - 1;
        let source_range = tile.patches[patch_index].quad_range.clone();
        let terrain_top = tile.quads[source_range.start as usize..source_range.end as usize]
            .iter()
            .copied()
            .find(|quad| quad.face == 2 && quad.extent == [4, 4])
            .expect("terrain top exists");
        tile.quads.push(voxels_world::SurfaceQuad {
            origin: [
                terrain_top.origin[0],
                terrain_top.origin[1] + 80,
                terrain_top.origin[2],
            ],
            material: Material::Stone,
            ..terrain_top
        });
        tile.patches[patch_index].quad_range.end = tile.quads.len() as u32;

        let (macro_normals, shapes) = surface_macro_normals_and_shapes(&tile);
        let horizons = surface_horizon_profiles(&tile);
        let profiles = surface_patch_profiles(&tile, &macro_normals, &horizons, &shapes);
        let profile = profiles
            .into_iter()
            .find_map(|(_, profile)| {
                profile
                    .sample_world(terrain_top.origin[0], terrain_top.origin[2])
                    .map(|sample| (profile, sample))
            })
            .expect("terrain profile contains the selected cell");
        assert_eq!(profile.1.height, terrain_top.origin[1]);
        assert_eq!(profile.1.material, Material::Grass);
    }

    #[test]
    fn active_lod_transition_exactly_joins_the_two_resident_height_profiles() {
        let focus = GeometricLodFocus::snapped(0, 0);
        let coarse = SurfacePatchId::new(SurfaceLodLevel::Stride4, 8, 0);
        let fine_low = SurfacePatchId::new(SurfaceLodLevel::Stride2, 15, 0);
        let fine_high = SurfacePatchId::new(SurfaceLodLevel::Stride2, 15, 1);
        let fine_parent = fine_low.parent().unwrap();
        assert_eq!(fine_high.parent(), Some(fine_parent));
        let resident = HashSet::from([coarse, fine_low, fine_high]);
        let mut selection = SurfacePatchSelection::default();
        selection.rebuild(focus, &resident, &HashSet::new());
        assert!(selection.is_transition_candidate(coarse, SurfacePatchEdge::NegativeX));

        let profiles = HashMap::from([
            (coarse, flat_patch_profile(coarse, 10)),
            (fine_low, flat_patch_profile(fine_low, 20)),
            (fine_high, flat_patch_profile(fine_high, 20)),
            (fine_parent, flat_patch_profile(fine_parent, 12)),
        ]);
        let transitions = build_lod_transitions(&selection, &profiles, &HashMap::new());
        assert_eq!(transitions.incomplete_edges, 0);
        assert_eq!(transitions.exact_edges.len(), 1);
        let connectors = transitions
            .quads
            .iter()
            .zip(&transitions.morph_heights)
            .filter(|(quad, _)| quad.material_face >> GPU_FACE_SHIFT & 7 != 2)
            .collect::<Vec<_>>();
        let stitches = transitions
            .quads
            .iter()
            .filter(|quad| quad.material_face >> GPU_FACE_SHIFT & 7 == 2)
            .collect::<Vec<_>>();
        assert_eq!(connectors.len(), 16);
        assert_eq!(stitches.len(), 24);
        for (quad, &morph_heights) in connectors {
            assert_eq!(quad.extent_voxels, [2, 10]);
            assert_eq!(quad.origin[0], 255);
            assert_eq!(quad.origin[1], 11);
            assert_eq!(quad.material_face >> GPU_FACE_SHIFT & 7, 0);
            assert_ne!(quad.ao & SURFACE_MACRO_NORMAL_FLAG, 0);
            assert_eq!(quad.origin[1] + i32::from(quad.extent_voxels[1]), 21,);
            assert_eq!(unpack_surface_morph_delta_half_voxels(morph_heights, 0), 0);
            assert_eq!(
                unpack_surface_morph_delta_half_voxels(morph_heights, 2),
                -16
            );
            assert_eq!(
                (quad.origin[1] + i32::from(quad.extent_voxels[1])) * 2
                    + unpack_surface_morph_delta_half_voxels(morph_heights, 2),
                26,
                "the fine endpoint must meet its own hidden parent at height 12"
            );
        }
        assert!(stitches.iter().all(|quad| {
            quad.extent_voxels[0] & TRANSITION_TRIANGLE_FLAG != 0 && quad.origin[1] == 10
        }));
        assert_eq!(
            stitches
                .iter()
                .filter(|quad| {
                    quad.extent_voxels[1]
                        - (quad.extent_voxels[0] & TRANSITION_TRIANGLE_OFFSET_MASK)
                        == 2
                })
                .count(),
            16,
            "two fine segments replace each of the eight coarse boundary edges"
        );

        let main = MeshSlice {
            relative_offset: 0,
            size: size_of::<GpuQuad>() as u32,
            quad_count: 1,
            bounds_min: glam::Vec3::ZERO,
            bounds_max: glam::Vec3::ONE,
            surface_patch_id: Some(coarse),
            boundary_edge: None,
            stitch_edges: 0,
            morph_closure: false,
            exact_replacement_chunk: None,
            canonical_water_surface: false,
            render_layer: RenderLayer::Opaque,
        };
        let edge = MeshSlice {
            boundary_edge: Some(SurfacePatchEdge::NegativeX),
            ..main
        };
        let top = MeshSlice {
            stitch_edges: 1 << SurfacePatchEdge::NegativeX.index(),
            ..main
        };
        let key = (SurfaceLodLevel::Stride4.index() + 1, 0, 0, 0);
        let plan = LodDrawPlan {
            patches: selection,
            canonical_columns: HashSet::new(),
            canonical_chunks: HashSet::new(),
            enclosed_view_chunks: HashSet::new(),
            exact_transition_edges: transitions.exact_edges,
            incomplete_transition_edges: transitions.incomplete_edges,
            transition_mesh_key: None,
        };
        assert!(slice_owned_by_lod(Some(focus), Some(&plan), &key, &main));
        assert!(!slice_owned_by_lod(Some(focus), Some(&plan), &key, &edge));
        assert!(!slice_owned_by_lod(Some(focus), Some(&plan), &key, &top));

        let fine_main = MeshSlice {
            surface_patch_id: Some(fine_low),
            ..main
        };
        let fine_edge = MeshSlice {
            boundary_edge: Some(SurfacePatchEdge::PositiveX),
            ..fine_main
        };
        let fine_top = MeshSlice {
            stitch_edges: 1 << SurfacePatchEdge::PositiveX.index(),
            ..fine_main
        };
        let fine_key = (SurfaceLodLevel::Stride2.index() + 1, 0, 0, 0);
        assert!(slice_owned_by_lod(
            Some(focus),
            Some(&plan),
            &fine_key,
            &fine_main
        ));
        assert!(
            !slice_owned_by_lod(Some(focus), Some(&plan), &fine_key, &fine_edge),
            "the connector is the sole vertical owner on both sides of the exact seam"
        );
        assert!(
            slice_owned_by_lod(Some(focus), Some(&plan), &fine_key, &fine_top),
            "only the coarse boundary top is replaced by subdivided stitch tops"
        );
    }

    #[test]
    fn active_lod_transition_exactly_joins_non_adjacent_resident_levels() {
        let focus = GeometricLodFocus::snapped(0, 0);
        let coarse = SurfacePatchId::new(SurfaceLodLevel::Stride16, 2, 0);
        let fine = (0..8)
            .map(|z| SurfacePatchId::new(SurfaceLodLevel::Stride2, 15, z))
            .collect::<Vec<_>>();
        let resident = HashSet::from_iter(std::iter::once(coarse).chain(fine.iter().copied()));
        let mut selection = SurfacePatchSelection::default();
        selection.rebuild(focus, &resident, &HashSet::new());
        assert!(selection.is_transition_candidate(coarse, SurfacePatchEdge::NegativeX));
        let mut profiles = HashMap::from([(coarse, flat_patch_profile(coarse, 10))]);
        profiles.extend(
            fine.iter()
                .copied()
                .map(|patch| (patch, flat_patch_profile_with_parent(patch, 20, Some(12)))),
        );

        let transitions = build_lod_transitions(&selection, &profiles, &HashMap::new());

        assert_eq!(transitions.incomplete_edges, 0);
        assert_eq!(transitions.exact_edges.len(), 1);
        let connectors = transitions
            .quads
            .iter()
            .zip(&transitions.morph_heights)
            .filter(|(quad, _)| quad.material_face >> GPU_FACE_SHIFT & 7 != 2)
            .collect::<Vec<_>>();
        let stitches = transitions
            .quads
            .iter()
            .filter(|quad| quad.material_face >> GPU_FACE_SHIFT & 7 == 2)
            .collect::<Vec<_>>();
        assert_eq!(connectors.len(), 64);
        assert_eq!(stitches.len(), 72);
        for (quad, &morph_heights) in connectors {
            assert_eq!(quad.extent_voxels, [2, 10]);
            assert_eq!(
                unpack_surface_morph_delta_half_voxels(morph_heights, 2),
                -16
            );
        }
        assert_eq!(
            stitches
                .iter()
                .filter(|quad| {
                    quad.extent_voxels[1]
                        - (quad.extent_voxels[0] & TRANSITION_TRIANGLE_OFFSET_MASK)
                        == 2
                })
                .count(),
            64
        );
    }

    #[test]
    fn active_lod_transition_grows_a_parent_only_step_from_the_shared_child_surface() {
        let focus = GeometricLodFocus::snapped(0, 0);
        let coarse = SurfacePatchId::new(SurfaceLodLevel::Stride4, 8, 0);
        let fine_low = SurfacePatchId::new(SurfaceLodLevel::Stride2, 15, 0);
        let fine_high = SurfacePatchId::new(SurfaceLodLevel::Stride2, 15, 1);
        let fine_parent = fine_low.parent().unwrap();
        let resident = HashSet::from([coarse, fine_low, fine_high]);
        let mut selection = SurfacePatchSelection::default();
        selection.rebuild(focus, &resident, &HashSet::new());
        let profiles = HashMap::from([
            (coarse, flat_patch_profile(coarse, 10)),
            (fine_low, flat_patch_profile(fine_low, 10)),
            (fine_high, flat_patch_profile(fine_high, 10)),
            (fine_parent, flat_patch_profile(fine_parent, 12)),
        ]);

        let transitions = build_lod_transitions(&selection, &profiles, &HashMap::new());

        assert_eq!(transitions.incomplete_edges, 0);
        assert_eq!(transitions.exact_edges.len(), 1);
        assert_eq!(transitions.quads.len(), 24);
        for (quad, &morph_heights) in transitions.quads.iter().zip(&transitions.morph_heights) {
            assert_eq!(quad.material_face >> GPU_FACE_SHIFT & 7, 2);
            assert_ne!(quad.extent_voxels[0] & TRANSITION_TRIANGLE_FLAG, 0);
            assert_eq!(quad.origin[1], 10);
            assert_eq!(unpack_surface_morph_delta_half_voxels(morph_heights, 0), 0);
            assert_eq!(unpack_surface_morph_delta_half_voxels(morph_heights, 2), 0);
        }
        assert_eq!(
            transitions
                .quads
                .iter()
                .filter(|quad| quad.extent_voxels[0] & TRANSITION_TRIANGLE_FLAG != 0)
                .count(),
            24
        );
    }

    #[test]
    fn active_lod_transition_splits_unbounded_height_differences_without_a_hole() {
        let focus = GeometricLodFocus::snapped(0, 0);
        let coarse = SurfacePatchId::new(SurfaceLodLevel::Stride4, 8, 0);
        let fine_low = SurfacePatchId::new(SurfaceLodLevel::Stride2, 15, 0);
        let fine_high = SurfacePatchId::new(SurfaceLodLevel::Stride2, 15, 1);
        let fine_parent = fine_low.parent().unwrap();
        let resident = HashSet::from([coarse, fine_low, fine_high]);
        let mut selection = SurfacePatchSelection::default();
        selection.rebuild(focus, &resident, &HashSet::new());
        let profiles = HashMap::from([
            (coarse, flat_patch_profile(coarse, 0)),
            (fine_low, flat_patch_profile(fine_low, 131_071)),
            (fine_high, flat_patch_profile(fine_high, 131_071)),
            (fine_parent, flat_patch_profile(fine_parent, 131_071)),
        ]);
        let transitions = build_lod_transitions(&selection, &profiles, &HashMap::new());
        assert_eq!(transitions.incomplete_edges, 0);
        assert_eq!(transitions.exact_edges.len(), 1);
        let connectors = transitions
            .quads
            .iter()
            .filter(|quad| quad.material_face >> GPU_FACE_SHIFT & 7 != 2)
            .collect::<Vec<_>>();
        let stitches = transitions
            .quads
            .iter()
            .filter(|quad| quad.material_face >> GPU_FACE_SHIFT & 7 == 2)
            .collect::<Vec<_>>();
        assert_eq!(connectors.len(), 16 * 3);
        assert_eq!(stitches.len(), 24);
        for segments in connectors.chunks_exact(3) {
            assert_eq!(
                segments
                    .iter()
                    .map(|quad| u32::from(quad.extent_voxels[1]))
                    .sum::<u32>(),
                131_071
            );
        }
    }

    #[test]
    fn incomplete_canonical_transition_keeps_the_resident_source_edge() {
        let focus = GeometricLodFocus::snapped(4_194, 6_034);
        let coarse = SurfacePatchId::new(SurfaceLodLevel::Stride2, 263, 384);
        let edge = SurfacePatchEdge::NegativeZ;
        let mut selection = SurfacePatchSelection::default();
        selection.rebuild(
            focus,
            &HashSet::from([coarse]),
            &HashSet::from([(131, 191)]),
        );
        assert!(selection.is_transition_candidate(coarse, edge));

        let profiles = HashMap::from([(coarse, flat_patch_profile(coarse, 10))]);
        let incomplete = build_lod_transitions(&selection, &profiles, &HashMap::new());
        assert_eq!(incomplete.incomplete_edges, 1);
        assert!(incomplete.exact_edges.is_empty());
        assert!(incomplete.quads.is_empty());
        let incomplete_plan = LodDrawPlan {
            patches: selection,
            canonical_columns: HashSet::new(),
            canonical_chunks: HashSet::new(),
            enclosed_view_chunks: HashSet::new(),
            exact_transition_edges: incomplete.exact_edges,
            incomplete_transition_edges: incomplete.incomplete_edges,
            transition_mesh_key: None,
        };
        assert!(
            incomplete_plan.owns_boundary_wall_edge(coarse, edge),
            "a source edge remains authoritative until its whole replacement is available"
        );

        let mut canonical_cells = vec![None; CHUNK_EDGE * CHUNK_EDGE];
        for local_x in 16..32 {
            canonical_cells[local_x + 31 * CHUNK_EDGE] = Some(SurfaceCell {
                height: 20,
                parent_height: None,
                material: Material::Stone,
                macro_normal: 0xff,
                horizon_profile: 0,
                shape: 0,
            });
        }
        let canonical_profiles = HashMap::from([(
            (131, 191),
            BTreeMap::from([(
                0,
                CanonicalChunkProfile {
                    cells: canonical_cells,
                },
            )]),
        )]);
        let mut complete_selection = SurfacePatchSelection::default();
        complete_selection.rebuild(
            focus,
            &HashSet::from([coarse]),
            &HashSet::from([(131, 191)]),
        );
        let complete = build_lod_transitions(&complete_selection, &profiles, &canonical_profiles);
        assert_eq!(complete.incomplete_edges, 0);
        assert_eq!(complete.exact_edges.len(), 1);
        assert_eq!(
            complete
                .quads
                .iter()
                .filter(|quad| quad.material_face >> GPU_FACE_SHIFT & 7 != 2)
                .count(),
            16
        );
        assert_eq!(
            complete
                .quads
                .iter()
                .filter(|quad| quad.material_face >> GPU_FACE_SHIFT & 7 == 2)
                .count(),
            24
        );
        let complete_plan = LodDrawPlan {
            patches: complete_selection,
            canonical_columns: HashSet::from([(131, 191)]),
            canonical_chunks: HashSet::new(),
            enclosed_view_chunks: HashSet::new(),
            exact_transition_edges: complete.exact_edges,
            incomplete_transition_edges: complete.incomplete_edges,
            transition_mesh_key: None,
        };
        assert!(!complete_plan.owns_boundary_wall_edge(coarse, edge));
    }

    #[test]
    fn canonical_surface_ownership_follows_exact_ready_bands_even_when_the_surface_is_empty() {
        let column = (50, 9);
        let cell_count = CHUNK_EDGE * CHUNK_EDGE;
        let inactive_profiles_only = HashSet::new();
        assert_eq!(
            canonical_surface_cell_coverage(column, &inactive_profiles_only),
            0
        );
        assert!(
            !canonical_ready_columns(&inactive_profiles_only).contains(&column),
            "an inactive retained profile must not suppress the surface fallback"
        );

        // The shell only publishes complete exact vertical bands. Once it does, empty cells are
        // legitimate canonical air (for example a dug shaft), not missing surface coverage.
        let ready = HashSet::from([(column.0, 41, column.1), (column.0, 42, column.1)]);
        assert!(canonical_ready_columns(&ready).contains(&column));
        assert_eq!(canonical_surface_cell_coverage(column, &ready), cell_count);
    }

    #[test]
    fn draw_plan_residency_accepts_committed_chunks_without_opaque_geometry() {
        let coord = (4, 5, 6);
        let key = (0, coord.0, coord.1, coord.2);
        let mut plan = LodDrawPlan {
            canonical_chunks: HashSet::from([coord]),
            ..LodDrawPlan::default()
        };
        let surface_residency = HashSet::new();
        let mut profiles = CanonicalColumnProfiles::from([(
            (coord.0, coord.2),
            BTreeMap::from([(
                coord.1,
                CanonicalChunkProfile {
                    cells: vec![None; CHUNK_EDGE * CHUNK_EDGE],
                },
            )]),
        )]);
        let mut chunks = BTreeMap::new();

        assert!(
            lod_draw_plan_resident(&plan, &surface_residency, &chunks, &profiles),
            "an empty or water-only committed chunk has no opaque mesh but is still resident"
        );
        profiles.clear();
        assert!(
            !lod_draw_plan_resident(&plan, &surface_residency, &chunks, &profiles),
            "evicting the upload marker must make the plan nonresident"
        );

        let mut arena = ArenaAllocator::new(64, 1);
        let allocation = arena
            .allocate(size_of::<GpuQuad>() as u32)
            .expect("test allocation");
        chunks.insert(
            key,
            ChunkMesh {
                allocation,
                morph_allocation: None,
                quad_count: 1,
                content_fingerprint: 0,
                slices: Vec::new(),
                lod_ownership_focus: None,
                lod_ownership_stale: true,
                lod_owned_slices: Vec::new(),
                bounds_min: glam::Vec3::ZERO,
                bounds_max: glam::Vec3::ONE,
                activation_mask: 0,
            },
        );
        assert!(
            !lod_draw_plan_resident(&plan, &surface_residency, &chunks, &profiles),
            "inactive opaque geometry cannot be replaced by a stale profile marker"
        );

        plan.canonical_chunks.clear();
        plan.enclosed_view_chunks.insert(coord);
        assert!(
            lod_draw_plan_resident(&plan, &surface_residency, &chunks, &profiles),
            "enclosed residency preserves the existing uploaded-mesh contract"
        );
    }

    #[test]
    fn vertical_ready_band_changes_do_not_invalidate_horizontal_lod_ownership() {
        let previous = HashSet::from([(4, 10, 7), (4, 11, 7), (5, 10, 7)]);
        let same_columns = HashSet::from([(4, 11, 7), (4, 12, 7), (5, 9, 7), (5, 10, 7)]);
        assert!(changed_canonical_ready_columns(&previous, &same_columns).is_empty());

        let removed_column = HashSet::from([(4, 11, 7), (4, 12, 7)]);
        assert_eq!(
            changed_canonical_ready_columns(&same_columns, &removed_column),
            HashSet::from([(5, 7)])
        );
    }

    #[test]
    fn canonical_plan_preserves_exact_vertical_ownership_inside_geometric_focus() {
        let focus = GeometricLodFocus::snapped(0, 0);
        let ready = HashSet::from([(0, 0, 0), (0, 1, 0), (100, 0, 100)]);
        assert_eq!(
            canonical_ready_chunks_for_focus(Some(focus), &ready),
            HashSet::from([(0, 0, 0), (0, 1, 0)])
        );
        assert!(canonical_ready_chunks_for_focus(None, &ready).is_empty());
    }

    #[test]
    fn canonical_surface_plan_keeps_ready_stride_two_handoff_chunks() {
        let focus = GeometricLodFocus::snapped_with_half_extents_for_levels(
            0,
            0,
            SurfaceLodLevel::ALL.len(),
            [32, 64, 128, 256, 512, 1_024, 2_048, 4_096],
        );
        let ready = HashSet::from([(1, 4, 0), (2, 4, 0)]);
        assert_eq!(
            canonical_surface_ready_chunks_for_focus(Some(focus), &ready),
            HashSet::from([(1, 4, 0)])
        );
    }

    #[test]
    fn underground_profile_edits_do_not_change_the_resolved_transition_surface() {
        let cell = |height| {
            Some(SurfaceCell {
                height,
                parent_height: None,
                material: Material::Stone,
                macro_normal: 0,
                horizon_profile: 0,
                shape: 0,
            })
        };
        let mut lower_cells = vec![None; CHUNK_EDGE * CHUNK_EDGE];
        lower_cells[0] = cell(12);
        let mut surface_cells = vec![None; CHUNK_EDGE * CHUNK_EDGE];
        surface_cells[0] = cell(40);
        let mut profiles = BTreeMap::from([
            (0, CanonicalChunkProfile { cells: lower_cells }),
            (
                1,
                CanonicalChunkProfile {
                    cells: surface_cells,
                },
            ),
        ]);
        let resolved = resolved_canonical_column_profile(&profiles);

        profiles.get_mut(&0).expect("lower profile").cells[0] = cell(13);
        assert_eq!(resolved_canonical_column_profile(&profiles), resolved);

        profiles.remove(&1);
        assert_ne!(resolved_canonical_column_profile(&profiles), resolved);
    }

    #[test]
    fn presented_stride_reports_the_actual_canonical_or_fallback_owner() {
        let focus = GeometricLodFocus::snapped(0, 0);
        let stride_two = SurfacePatchId::new(SurfaceLodLevel::Stride2, 0, 0);
        let resident = HashSet::from([stride_two]);
        let mut fallback = SurfacePatchSelection::default();
        fallback.rebuild(focus, &resident, &HashSet::new());
        let fallback_plan = LodDrawPlan {
            patches: fallback,
            canonical_columns: HashSet::new(),
            ..LodDrawPlan::default()
        };
        assert_eq!(fallback_plan.presented_stride_at(Some(focus), 1, 1, 1), 2);

        let mut canonical = SurfacePatchSelection::default();
        canonical.rebuild(focus, &resident, &HashSet::from([(0, 0)]));
        let canonical_plan = LodDrawPlan {
            patches: canonical,
            canonical_columns: HashSet::from([(0, 0)]),
            ..LodDrawPlan::default()
        };
        assert_eq!(canonical_plan.presented_stride_at(Some(focus), 1, 1, 1), 1);
        assert_eq!(canonical_plan.presented_stride_at(None, 1, 1, 1), 0);

        let enclosed_plan = LodDrawPlan {
            enclosed_view_chunks: HashSet::from([(0, -2, 0)]),
            ..fallback_plan
        };
        assert_eq!(
            enclosed_plan.presented_stride_at(Some(focus), 1, -63, 1),
            1,
            "an exact underground owner must win over the surface proxy in the same column"
        );
    }

    #[test]
    fn canonical_profile_invalidation_is_limited_to_the_touching_transition_edge() {
        let patch = SurfacePatchId::new(SurfaceLodLevel::Stride2, 0, 0);
        assert!(canonical_column_touches_patch_edge(
            (-1, 0),
            patch,
            SurfacePatchEdge::NegativeX,
        ));
        assert!(canonical_column_touches_patch_edge(
            (0, 0),
            patch,
            SurfacePatchEdge::PositiveX,
        ));
        assert!(canonical_column_touches_patch_edge(
            (0, -1),
            patch,
            SurfacePatchEdge::NegativeZ,
        ));
        assert!(canonical_column_touches_patch_edge(
            (0, 0),
            patch,
            SurfacePatchEdge::PositiveZ,
        ));
        for edge in SurfacePatchEdge::ALL {
            assert!(!canonical_column_touches_patch_edge((1, 1), patch, edge));
        }
    }

    #[test]
    fn distant_surface_normals_bound_decimation_outliers() {
        let gradient = stabilized_surface_gradient(glam::Vec2::new(80.0, -60.0));
        assert!((gradient.length() - SURFACE_MACRO_SLOPE_MAX).abs() < 0.0001);
        let normal = glam::Vec3::new(-gradient.x, 1.0, -gradient.y).normalize();
        assert!(
            normal.y >= 0.89,
            "macro lighting must not turn unresolved relief into a near-horizontal face: {normal:?}"
        );
    }

    #[test]
    fn every_child_parent_normal_bit_matches_the_parent_tiles_own_normal() {
        let surface = |x: i32, z: i32| {
            (
                x.div_euclid(7) + z.div_euclid(11) + (x * x + z * z).rem_euclid(17),
                Material::Stone,
            )
        };
        for child_level in SurfaceLodLevel::ALL.into_iter().take(5) {
            let parent_level = child_level.next_coarser().unwrap();
            let child_coord = SurfaceTileCoord::new(child_level, 0, 0);
            let parent_coord = SurfaceTileCoord::new(parent_level, 0, 0);
            let child = voxels_world::generate_surface_tile_mesh_with(child_coord, surface);
            let parent = voxels_world::generate_surface_tile_mesh_with(parent_coord, surface);
            let child_normals = surface_macro_normals(&child);
            let parent_normals = surface_macro_normals(&parent);
            let child_horizons = surface_horizon_profiles(&child);
            let parent_horizons = surface_horizon_profiles(&parent);
            let child_stride = child_level.stride_voxels();
            let parent_stride = parent_level.stride_voxels();
            for z in 0..voxels_world::SURFACE_TILE_EDGE_CELLS {
                for x in 0..voxels_world::SURFACE_TILE_EDGE_CELLS {
                    let child_origin = [x * child_stride, z * child_stride];
                    let child_quad = child
                        .quads
                        .iter()
                        .position(|quad| {
                            quad.face == 2
                                && quad.origin[0] == child_origin[0]
                                && quad.origin[2] == child_origin[1]
                                && quad.extent == [child_stride as u16; 2]
                        })
                        .unwrap();
                    let parent_origin = [
                        child_origin[0].div_euclid(parent_stride) * parent_stride,
                        child_origin[1].div_euclid(parent_stride) * parent_stride,
                    ];
                    let parent_quad = parent
                        .quads
                        .iter()
                        .position(|quad| {
                            quad.face == 2
                                && quad.origin[0] == parent_origin[0]
                                && quad.origin[2] == parent_origin[1]
                                && quad.extent == [parent_stride as u16; 2]
                        })
                        .unwrap();
                    let child_parent = (child_normals[child_quad] >> 10) & 0x03ff;
                    let parent_own = parent_normals[parent_quad] & 0x03ff;
                    assert_eq!(
                        child_parent, parent_own,
                        "{child_level:?} child ({x}, {z}) disagrees with {parent_level:?}"
                    );
                    assert_eq!(
                        child_horizons[child_quad] >> 8,
                        parent_horizons[parent_quad] & 0xff,
                        "{child_level:?} child horizon ({x}, {z}) disagrees with {parent_level:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn gpu_quad_packing_preserves_every_material_bit_face_and_extent() {
        let materials = [
            u32::from(Material::Grass.id()),
            u32::from(Material::GlowCrystal.id()),
            u32::from(Material::Water.id()) | FAR_MATERIAL_FLAG,
            u32::from(Material::Stone.id())
                | FAR_MATERIAL_FLAG
                | (u32::from(SurfaceLodLevel::Stride16.index()) << SURFACE_LOD_SHIFT),
        ];
        for material in materials {
            assert_eq!(material & GPU_FACE_MASK, 0);
            for face in 0..=5 {
                let packed = pack_gpu_material_face(material, face);
                assert_eq!((packed & GPU_FACE_MASK) >> GPU_FACE_SHIFT, u32::from(face));
                assert_eq!(packed & !GPU_FACE_MASK, material);
            }
        }
        let quad = GpuQuad {
            origin: [-1_235, 3, 81_920],
            extent_voxels: [u16::MAX, 1],
            material_face: pack_gpu_material_face(materials[3], 5),
            ao: u32::MAX,
        };
        let bytes = bytemuck::bytes_of(&quad);
        assert_eq!(bytes.len(), 24);
        assert_eq!(quad.extent_voxels, [u16::MAX, 1]);
    }

    #[test]
    fn identical_surface_gpu_products_do_not_replace_resident_meshes() {
        let quad = GpuQuad {
            origin: [11, 23, 37],
            extent_voxels: [8, 5],
            material_face: pack_gpu_material_face(u32::from(Material::Grass.id()), 2),
            ao: 0xff,
        };
        let quads = [quad];
        let fingerprint = gpu_quad_content_fingerprint(&quads, None);
        assert!(gpu_quad_content_matches(
            Some((1, fingerprint)),
            1,
            fingerprint
        ));
        assert!(!gpu_quad_content_matches(
            Some((2, fingerprint)),
            1,
            fingerprint
        ));

        let mut changed = quad;
        changed.origin[1] += 1;
        let changed_fingerprint = gpu_quad_content_fingerprint(&[changed], None);
        assert!(!gpu_quad_content_matches(
            Some((1, fingerprint)),
            1,
            changed_fingerprint,
        ));
        assert!(gpu_quad_content_matches(None, 0, FINGERPRINT_OFFSET));
        assert!(!gpu_quad_content_matches(
            Some((1, fingerprint)),
            0,
            FINGERPRINT_OFFSET,
        ));
        assert!(!gpu_quad_content_matches(None, 1, fingerprint));

        let first_morph = [pack_surface_morph_heights(-3, 7)];
        let second_morph = [pack_surface_morph_heights(-3, 8)];
        let first_fingerprint = fingerprint_value(
            fingerprint,
            fingerprint_bytes(bytemuck::cast_slice(&first_morph)),
        );
        let second_fingerprint = fingerprint_value(
            fingerprint,
            fingerprint_bytes(bytemuck::cast_slice(&second_morph)),
        );
        assert_ne!(first_fingerprint, second_fingerprint);
        assert!(gpu_quad_content_matches(
            Some((1, first_fingerprint)),
            1,
            first_fingerprint,
        ));
        assert!(!gpu_quad_content_matches(
            Some((1, first_fingerprint)),
            1,
            second_fingerprint,
        ));
    }

    #[test]
    fn surface_profile_change_detection_ignores_identical_tile_replacements() {
        let tile = SurfaceTileCoord::new(SurfaceLodLevel::Stride2, 0, 0);
        let patch = SurfacePatchId::new(SurfaceLodLevel::Stride2, 0, 0);
        let profile = SurfacePatchProfile {
            origin: [0, 0],
            stride: 2,
            cells: vec![
                None;
                (voxels_world::SURFACE_PATCH_EDGE_CELLS * voxels_world::SURFACE_PATCH_EDGE_CELLS)
                    as usize
            ],
        };
        let previous = HashMap::from([(patch, profile.clone())]);
        let identical = vec![(patch, profile.clone())];
        assert!(changed_surface_patch_profiles(tile, &previous, &identical).is_empty());

        let mut changed_profile = profile;
        changed_profile.cells[0] = Some(SurfaceCell {
            height: 7,
            parent_height: None,
            material: Material::Stone,
            macro_normal: 0,
            horizon_profile: 0,
            shape: 0,
        });
        let changed = vec![(patch, changed_profile)];
        assert_eq!(
            changed_surface_patch_profiles(tile, &previous, &changed),
            HashSet::from([patch])
        );
        assert_eq!(
            changed_surface_patch_profiles(tile, &previous, &[]),
            HashSet::from([patch])
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
                    == SurfacePatchEdge::PositiveX.index() as u16
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
                        == SurfacePatchEdge::PositiveX.index() as u16
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
        assert!(constrained[0].iter().all(|quad| {
            unpack_canonical_triangle_extent(quad.extent_voxels[0]) == 2
                && unpack_canonical_triangle_extent(quad.extent_voxels[1]) == 63
        }));

        let split = split_gpu_quad_vertical_extent(
            GpuQuad {
                origin: [3, -70, 5],
                extent_voxels: [2, 150],
                ..large
            },
            63,
        );
        assert_eq!(
            split
                .iter()
                .map(|quad| (quad.origin[1], quad.extent_voxels[1]))
                .collect::<Vec<_>>(),
            [(-70, 7), (-63, 63), (0, 63), (63, 17)]
        );
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
    fn failed_second_layer_keeps_resident_mesh_and_releases_prepared_storage() {
        let key = (0, 1, 2, 3);
        let mut arena = ArenaAllocator::new(128, 4);
        let mut morph_arena = ArenaAllocator::new(128, 4);
        let resident = arena.allocate(32).expect("resident allocation");
        let resident_morph = morph_arena.allocate(8).expect("resident morph allocation");
        let prepared = arena.allocate(64).expect("prepared allocation");
        let prepared_morph = morph_arena.allocate(8).expect("prepared morph allocation");
        let mut chunks = BTreeMap::from([(
            key,
            ChunkMesh {
                allocation: resident,
                morph_allocation: Some(resident_morph),
                quad_count: 1,
                content_fingerprint: 1,
                slices: Vec::new(),
                lod_ownership_focus: None,
                lod_ownership_stale: true,
                lod_owned_slices: Vec::new(),
                bounds_min: glam::Vec3::ZERO,
                bounds_max: glam::Vec3::ZERO,
                activation_mask: u8::MAX,
            },
        )]);

        discard_prepared_mesh(
            &mut arena,
            Some(&mut morph_arena),
            Some(ChunkMesh {
                allocation: prepared,
                morph_allocation: Some(prepared_morph),
                quad_count: 2,
                content_fingerprint: 2,
                slices: Vec::new(),
                lod_ownership_focus: None,
                lod_ownership_stale: true,
                lod_owned_slices: Vec::new(),
                bounds_min: glam::Vec3::ZERO,
                bounds_max: glam::Vec3::ZERO,
                activation_mask: u8::MAX,
            }),
        );

        assert_eq!(chunks.get(&key).map(|mesh| mesh.allocation), Some(resident));
        assert_eq!(arena.stats().allocated_bytes, u64::from(resident.size));
        assert_eq!(
            morph_arena.stats().allocated_bytes,
            u64::from(resident_morph.size)
        );
        assert!(!arena.free(prepared));
        assert!(!morph_arena.free(prepared_morph));
        let resident_mesh = chunks.remove(&key).expect("resident mesh");
        assert!(arena.free(resident_mesh.allocation));
        assert!(morph_arena.free(resident_mesh.morph_allocation.expect("resident morph")));
    }

    fn test_slice() -> MeshSlice {
        MeshSlice {
            relative_offset: 0,
            size: size_of::<GpuQuad>() as u32,
            quad_count: 1,
            bounds_min: glam::Vec3::splat(-10_000.0),
            bounds_max: glam::Vec3::splat(10_000.0),
            surface_patch_id: None,
            boundary_edge: None,
            stitch_edges: 0,
            morph_closure: false,
            exact_replacement_chunk: None,
            canonical_water_surface: false,
            render_layer: RenderLayer::Opaque,
        }
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

    #[test]
    fn directory_slices_assign_the_actual_canonical_surface_and_renderer_owners() {
        assert_eq!(
            diagnostic_owner_for_slice((0, -9, 4, -7), &test_slice()),
            diagnostic_owner_id(1, 0, -9, 4, -7)
        );

        let mut surface_slice = test_slice();
        surface_slice.surface_patch_id =
            Some(SurfacePatchId::new(SurfaceLodLevel::Stride8, -7, 13));
        assert_eq!(
            diagnostic_owner_for_slice(
                (SurfaceLodLevel::Stride8.index() + 1, 0, 0, 0),
                &surface_slice,
            ),
            diagnostic_owner_id(2, 3, -7, 0, 13)
        );
        assert_eq!(
            diagnostic_owner_for_slice((255, 2, 0, 0), &test_slice()),
            diagnostic_owner_id(3, 255, 2, 0, 0)
        );
    }

    #[test]
    fn enclosed_volume_does_not_double_own_the_streamed_water_surface() {
        let key = (0, 3, 1, -2);
        let mut plan = LodDrawPlan::default();
        plan.enclosed_view_chunks.insert((key.1, key.2, key.3));
        let volume = MeshSlice {
            render_layer: RenderLayer::Translucent,
            ..test_slice()
        };
        let surface = MeshSlice {
            canonical_water_surface: true,
            ..volume
        };
        let focus = GeometricLodFocus::snapped(0, 0);

        assert!(slice_owned_by_lod(Some(focus), Some(&plan), &key, &volume));
        assert!(!slice_owned_by_lod(
            Some(focus),
            Some(&plan),
            &key,
            &surface
        ));

        plan.canonical_chunks.insert((key.1, key.2, key.3));
        assert!(slice_owned_by_lod(Some(focus), Some(&plan), &key, &surface));
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
            17_300_000, 18_300_000, 18_500_000, 18_900_000, 19_000_000, 19_800_000,
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
        assert!((timing.virtual_terrain_traversal_ms - 0.4).abs() < f32::EPSILON);
        assert!((timing.virtual_terrain_compaction_ms - 0.8).abs() < f32::EPSILON);

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
        assert_eq!(timing.virtual_terrain_traversal_ms, 0.0);
        assert_eq!(timing.virtual_terrain_compaction_ms, 0.0);
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
        let key = TerrainPageKey {
            level: 2,
            coord: [-3, 1, 4],
        };
        let cut = VirtualTerrainCut {
            selected_pages: vec![key],
            requested_pages: Vec::new(),
            ownerless_roots: Vec::new(),
            fingerprint: 0x1234_5678_9abc_def0,
            visited_nodes: 1,
            selected_primitives: 4,
            selected_encoded_bytes: 64,
            feedback_overflow: false,
            selection_overflow: false,
            traversal_overflow: false,
            incoherent_replacement_groups: 0,
        };
        let certified = GpuVirtualTerrainFeedback {
            submission_id: 9,
            oracle_fingerprint: cut.fingerprint,
            selected_pages: vec![key],
            compacted_pages: 1,
            ..GpuVirtualTerrainFeedback::default()
        };
        assert!(gpu_feedback_matches_cut(&certified, Some(&cut)));

        let mut stale = certified.clone();
        stale.oracle_fingerprint ^= 1;
        assert!(!gpu_feedback_matches_cut(&stale, Some(&cut)));

        let mut incomplete = certified;
        incomplete.compacted_pages = 0;
        assert!(!gpu_feedback_matches_cut(&incomplete, Some(&cut)));
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
    fn contiguous_mesh_allocations_coalesce_into_one_instanced_draw() {
        let spans = coalesce_draw_items(vec![
            DrawItem {
                page: 1,
                offset: 64,
                size: 32,
                quad_count: 1,
                morph_page: None,
                morph_offset: 0,
            },
            DrawItem {
                page: 0,
                offset: 96,
                size: 64,
                quad_count: 2,
                morph_page: None,
                morph_offset: 0,
            },
            DrawItem {
                page: 0,
                offset: 0,
                size: 96,
                quad_count: 3,
                morph_page: None,
                morph_offset: 0,
            },
            DrawItem {
                page: 0,
                offset: 192,
                size: 32,
                quad_count: 1,
                morph_page: None,
                morph_offset: 0,
            },
        ]);
        assert_eq!(
            spans,
            vec![
                DrawSpan {
                    page: 0,
                    offset: 0,
                    size: 160,
                    quad_count: 5,
                    morph_page: None,
                    morph_offset: 0,
                },
                DrawSpan {
                    page: 0,
                    offset: 192,
                    size: 32,
                    quad_count: 1,
                    morph_page: None,
                    morph_offset: 0,
                },
                DrawSpan {
                    page: 1,
                    offset: 64,
                    size: 32,
                    quad_count: 1,
                    morph_page: None,
                    morph_offset: 0,
                },
            ]
        );
    }

    #[test]
    fn morph_draws_coalesce_only_when_base_and_sidecar_are_both_contiguous() {
        let item = |offset, morph_offset| DrawItem {
            page: 0,
            offset,
            size: size_of::<GpuQuad>() as u32,
            quad_count: 1,
            morph_page: Some(1),
            morph_offset,
        };
        let quad_bytes = size_of::<GpuQuad>() as u32;
        let morph_bytes = size_of::<GpuMorph>() as u32;
        let contiguous = coalesce_draw_items(vec![item(0, 0), item(quad_bytes, morph_bytes)]);
        assert_eq!(contiguous.len(), 1);
        assert_eq!(contiguous[0].quad_count, 2);

        let split_sidecar =
            coalesce_draw_items(vec![item(0, 0), item(quad_bytes, morph_bytes * 2)]);
        assert_eq!(split_sidecar.len(), 2);
    }

    fn reference_draw_list(
        chunks: &BTreeMap<MeshKey, ChunkMesh>,
        mut include_chunk: impl FnMut(&MeshKey, &ChunkMesh) -> bool,
        mut include_slice: impl FnMut(&MeshKey, &MeshSlice) -> bool,
    ) -> DrawList {
        let mut builder = DrawListBuilder::default();
        for (key, chunk) in chunks {
            if !chunk.active() || !include_chunk(key, chunk) {
                continue;
            }
            let mut selected = false;
            for slice in &chunk.slices {
                builder.test_slice();
                if include_slice(key, slice) {
                    builder.select_slice(chunk, slice);
                    selected = true;
                }
            }
            if selected {
                builder.select_mesh(*key, chunk);
            }
        }
        builder.finish()
    }

    fn assert_world_draw_lists_match_reference(actual: &WorldDrawLists, expected: &DrawList) {
        let items = actual
            .fixed
            .spans
            .iter()
            .chain(&actual.morphing.spans)
            .map(|span| DrawItem {
                page: span.page,
                offset: span.offset,
                size: span.size,
                quad_count: span.quad_count,
                morph_page: None,
                morph_offset: 0,
            })
            .collect();
        assert_eq!(coalesce_draw_items(items), expected.spans);
        assert_eq!(actual.mesh_count, expected.mesh_count);
        assert_eq!(actual.quad_count, expected.quad_count);
        assert_eq!(actual.tested_slices, expected.tested_slices);
        assert_eq!(actual.selected_slices, expected.selected_slices);
    }

    #[test]
    fn one_pass_opaque_lists_match_independent_camera_and_shadow_traversals() {
        let canonical_key = (0, 0, 0, 0);
        let surface_key = (SurfaceLodLevel::Stride2.index() + 1, 1, 0, 0);
        let bounds_min = glam::Vec3::new(-0.5, 0.1, -0.5);
        let bounds_max = glam::Vec3::new(0.5, 0.9, 0.5);
        let canonical_slice = MeshSlice {
            relative_offset: 0,
            size: size_of::<GpuQuad>() as u32,
            quad_count: 1,
            bounds_min,
            bounds_max,
            surface_patch_id: None,
            boundary_edge: None,
            stitch_edges: 0,
            morph_closure: false,
            exact_replacement_chunk: None,
            canonical_water_surface: false,
            render_layer: RenderLayer::Opaque,
        };
        let surface_slice = MeshSlice {
            relative_offset: 0,
            size: size_of::<GpuQuad>() as u32 * 2,
            quad_count: 2,
            bounds_min,
            bounds_max,
            surface_patch_id: Some(SurfacePatchId::new(SurfaceLodLevel::Stride2, 6, 0)),
            boundary_edge: None,
            stitch_edges: 0,
            morph_closure: false,
            exact_replacement_chunk: None,
            canonical_water_surface: false,
            render_layer: RenderLayer::Opaque,
        };
        let surface_edge_slice = MeshSlice {
            relative_offset: surface_slice.size,
            size: size_of::<GpuQuad>() as u32,
            quad_count: 1,
            boundary_edge: Some(SurfacePatchEdge::NegativeX),
            ..surface_slice
        };
        let mut arena = ArenaAllocator::new(256, 1);
        let canonical_allocation = arena
            .allocate(canonical_slice.size)
            .expect("canonical test allocation");
        let surface_allocation = arena
            .allocate(surface_slice.size + surface_edge_slice.size)
            .expect("surface test allocation");
        let mut chunks = BTreeMap::from([
            (
                canonical_key,
                ChunkMesh {
                    allocation: canonical_allocation,
                    morph_allocation: None,
                    quad_count: canonical_slice.quad_count,
                    content_fingerprint: 11,
                    slices: vec![canonical_slice],
                    lod_ownership_focus: None,
                    lod_ownership_stale: true,
                    lod_owned_slices: Vec::new(),
                    bounds_min,
                    bounds_max,
                    activation_mask: u8::MAX,
                },
            ),
            (
                surface_key,
                ChunkMesh {
                    allocation: surface_allocation,
                    morph_allocation: None,
                    quad_count: surface_slice.quad_count + surface_edge_slice.quad_count,
                    content_fingerprint: 22,
                    slices: vec![surface_slice, surface_edge_slice],
                    lod_ownership_focus: None,
                    lod_ownership_stale: true,
                    lod_owned_slices: Vec::new(),
                    bounds_min,
                    bounds_max,
                    activation_mask: u8::MAX,
                },
            ),
        ]);
        let main_only = reference_draw_list(
            &chunks,
            |key, _| *key == surface_key,
            |_, slice| slice.boundary_edge.is_none(),
        );
        let edge_only = reference_draw_list(
            &chunks,
            |key, _| *key == surface_key,
            |_, slice| slice.boundary_edge.is_some(),
        );
        assert_ne!(
            main_only.fingerprint, edge_only.fingerprint,
            "viewport identity must include the selected submesh range"
        );
        let focus_value = GeometricLodFocus::snapped(0, 0);
        let focus = Some(focus_value);
        let surface_patch_residency =
            HashSet::from([surface_slice.surface_patch_id.expect("surface patch id")]);
        let mut lod_draw_plan = LodDrawPlan::default();
        lod_draw_plan
            .patches
            .rebuild(focus_value, &surface_patch_residency, &HashSet::new());
        let view_clip = AabbClipVolume::new(glam::Mat4::IDENTITY);
        let shadow_clips = [view_clip; CASCADE_COUNT];
        let (actual_shadows, actual_world, _) = collect_opaque_draw_lists(
            &mut chunks,
            Some(&lod_draw_plan),
            None,
            true,
            true,
            focus,
            view_clip,
            shadow_clips,
            &VirtualTerrainOwnership::default(),
        )
        .unwrap_or_else(|_| panic!("test meshes must have every required morph sidecar"));
        let expected_world = reference_draw_list(
            &chunks,
            |_, chunk| view_clip.contains_aabb(chunk.bounds_min, chunk.bounds_max),
            |key, slice| {
                slice.render_layer == RenderLayer::Opaque
                    && slice_owned_by_lod(focus, Some(&lod_draw_plan), key, slice)
                    && view_clip.contains_aabb(slice.bounds_min, slice.bounds_max)
            },
        );
        let expected_shadows: [DrawList; CASCADE_COUNT] = std::array::from_fn(|cascade_index| {
            let mut draw_list = reference_draw_list(
                &chunks,
                |key, chunk| {
                    mesh_casts_directional_shadow(key)
                        && shadow_clips[cascade_index]
                            .contains_aabb(chunk.bounds_min, chunk.bounds_max)
                },
                |key, slice| {
                    slice.render_layer == RenderLayer::Opaque
                        && slice_owned_by_lod(focus, Some(&lod_draw_plan), key, slice)
                        && shadow_clips[cascade_index]
                            .contains_aabb(slice.bounds_min, slice.bounds_max)
                },
            );
            draw_list.fingerprint = FINGERPRINT_OFFSET;
            draw_list
        });
        assert_world_draw_lists_match_reference(&actual_world, &expected_world);
        for (actual, expected) in actual_shadows.iter().zip(&expected_shadows) {
            assert_world_draw_lists_match_reference(actual, expected);
        }
        assert_eq!(actual_world.quad_count, actual_shadows[0].quad_count);

        let cached_world = collect_opaque_draw_lists(
            &mut chunks,
            Some(&lod_draw_plan),
            None,
            true,
            true,
            focus,
            view_clip,
            shadow_clips,
            &VirtualTerrainOwnership::default(),
        )
        .unwrap_or_else(|_| panic!("test meshes must have every required morph sidecar"))
        .1;
        assert_eq!(cached_world, actual_world);
        assert!(
            chunks
                .values()
                .all(|chunk| chunk.lod_ownership_focus == focus)
        );

        let moved_focus_value = GeometricLodFocus::snapped(256, -192);
        let moved_focus = Some(moved_focus_value);
        let previous_plan = std::mem::take(&mut lod_draw_plan);
        lod_draw_plan
            .patches
            .rebuild(moved_focus_value, &surface_patch_residency, &HashSet::new());
        for key in changed_surface_lod_ownership_keys(
            &previous_plan,
            &lod_draw_plan.patches,
            &lod_draw_plan.exact_transition_edges,
        ) {
            if let Some(chunk) = chunks.get_mut(&key) {
                chunk.lod_ownership_stale = true;
            }
        }
        let moved_world = collect_opaque_draw_lists(
            &mut chunks,
            Some(&lod_draw_plan),
            None,
            true,
            true,
            moved_focus,
            view_clip,
            shadow_clips,
            &VirtualTerrainOwnership::default(),
        )
        .unwrap_or_else(|_| panic!("test meshes must have every required morph sidecar"))
        .1;
        let moved_expected = reference_draw_list(
            &chunks,
            |_, chunk| view_clip.contains_aabb(chunk.bounds_min, chunk.bounds_max),
            |key, slice| {
                slice.render_layer == RenderLayer::Opaque
                    && slice_owned_by_lod(moved_focus, Some(&lod_draw_plan), key, slice)
                    && view_clip.contains_aabb(slice.bounds_min, slice.bounds_max)
            },
        );
        assert_world_draw_lists_match_reference(&moved_world, &moved_expected);
        assert_eq!(
            chunks
                .get(&canonical_key)
                .and_then(|chunk| chunk.lod_ownership_focus),
            moved_focus
        );
    }

    #[test]
    fn exact_view_volume_supplements_the_surface_lod_without_claiming_its_column() {
        let focus = GeometricLodFocus::snapped(0, 0);
        let patch_id = SurfacePatchId::new(SurfaceLodLevel::Stride2, 3, 0);
        let resident = HashSet::from([patch_id]);
        let mut plan = LodDrawPlan {
            canonical_columns: HashSet::from([(0, 0)]),
            canonical_chunks: HashSet::from([(0, 0, 0)]),
            ..Default::default()
        };
        plan.patches.rebuild(focus, &resident, &HashSet::new());
        assert!(slice_owned_by_lod(
            Some(focus),
            Some(&plan),
            &(0, 0, 0, 0),
            &test_slice()
        ));
        assert!(
            !slice_owned_by_lod(Some(focus), Some(&plan), &(0, 0, 1, 0), &test_slice()),
            "a ready X/Z column must not claim an unrelated vertical chunk"
        );
        assert!(!slice_owned_by_lod(
            Some(focus),
            Some(&plan),
            &(0, 7, 0, 0),
            &test_slice()
        ));
        plan.enclosed_view_chunks
            .extend([(7, -3, 0), (7, -2, 0), (7, -1, 0)]);
        assert!(plan.owns_exact_volume_coord((0, 0, 0)));
        assert!(plan.owns_exact_volume_coord((7, -2, 0)));
        assert!(!plan.owns_exact_volume_coord((7, 0, 0)));
        for y in -3..=-1 {
            assert!(
                slice_owned_by_lod(Some(focus), Some(&plan), &(0, 7, y, 0), &test_slice()),
                "every selected vertical chunk remains available for tunnels, caverns, and overhangs"
            );
        }
        assert!(
            !slice_owned_by_lod(Some(focus), Some(&plan), &(0, 7, 0, 0), &test_slice()),
            "exact-volume ownership is three-dimensional, not an accidental whole-column claim"
        );

        let mut stride_two_patch = test_slice();
        stride_two_patch.surface_patch_id = Some(patch_id);
        assert!(
            slice_owned_by_lod(
                Some(focus),
                Some(&plan),
                &(SurfaceLodLevel::Stride2.index() + 1, 1, 0, 0),
                &stride_two_patch
            ),
            "enclosed volume must not suppress the far surface above it"
        );
        assert!(!slice_owned_by_lod(
            Some(focus),
            Some(&plan),
            &(SurfaceLodLevel::Stride4.index() + 1, 1, 0, 0),
            &stride_two_patch
        ));

        let mut fallback = stride_two_patch;
        fallback.exact_replacement_chunk = Some((7, -2, 0));
        assert!(
            !slice_owned_by_lod(
                Some(focus),
                Some(&plan),
                &(SurfaceLodLevel::Stride2.index() + 1, 1, 0, 0),
                &fallback,
            ),
            "ready exact volume must replace only its tagged synthetic cover"
        );
        fallback.exact_replacement_chunk = Some((7, 1, 0));
        assert!(
            slice_owned_by_lod(
                Some(focus),
                Some(&plan),
                &(SurfaceLodLevel::Stride2.index() + 1, 1, 0, 0),
                &fallback,
            ),
            "unrelated exact-volume readiness must leave fallback coverage resident"
        );
    }

    #[test]
    fn replaceable_surface_quads_map_to_one_exact_chunk_in_negative_space() {
        for (face, origin, extent, expected) in [
            (0, [-33, -2, -32], [2, 2], (-2, -1, -1)),
            (1, [-32, 32, -34], [2, 1], (-1, 1, -2)),
            (4, [-32, -34, 31], [2, 2], (-1, -2, 0)),
            (5, [-34, 0, 32], [2, 1], (-2, 0, 1)),
        ] {
            let quad = SurfaceQuad {
                origin,
                face,
                extent,
                material: Material::Stone,
                synthetic_fallback: true,
            };
            assert_eq!(surface_exact_replacement_chunk(&quad), Some(expected));
        }
        let ordinary = SurfaceQuad {
            origin: [0; 3],
            face: 0,
            extent: [2, 2],
            material: Material::Stone,
            synthetic_fallback: false,
        };
        assert_eq!(surface_exact_replacement_chunk(&ordinary), None);
    }

    #[test]
    fn resident_hierarchy_keeps_surface_cover_until_canonical_column_is_complete() {
        let focus = GeometricLodFocus::snapped(0, 0);
        let patch_id = SurfacePatchId::new(SurfaceLodLevel::Stride2, 0, 0);
        let mut surface = test_slice();
        surface.surface_patch_id = Some(patch_id);
        let resident = HashSet::from([patch_id]);
        let mut plan = LodDrawPlan::default();
        plan.patches.rebuild(focus, &resident, &HashSet::new());
        assert!(slice_owned_by_lod(
            Some(focus),
            Some(&plan),
            &(SurfaceLodLevel::Stride2.index() + 1, 0, 0, 0),
            &surface,
        ));

        plan.patches
            .rebuild(focus, &resident, &HashSet::from([(0, 0)]));
        assert!(!slice_owned_by_lod(
            Some(focus),
            Some(&plan),
            &(SurfaceLodLevel::Stride2.index() + 1, 0, 0, 0),
            &surface,
        ));
    }

    #[test]
    fn geometric_lod_uses_patch_identity_not_protruding_geometry_bounds() {
        let focus = GeometricLodFocus::snapped(0, 0);
        let patch_id = SurfacePatchId::new(SurfaceLodLevel::Stride4, 8, 0);
        let mut slice = test_slice();
        slice.surface_patch_id = Some(patch_id);
        let resident = HashSet::from([patch_id]);
        let mut plan = LodDrawPlan::default();
        plan.patches.rebuild(focus, &resident, &HashSet::new());
        assert!(slice.bounds_min.x < -9_000.0);
        assert!(slice.bounds_max.x > 9_000.0);
        assert!(slice_owned_by_lod(
            Some(focus),
            Some(&plan),
            &(SurfaceLodLevel::Stride4.index() + 1, 2, 0, 0),
            &slice
        ));
        assert!(!slice_owned_by_lod(
            Some(focus),
            Some(&plan),
            &(SurfaceLodLevel::Stride8.index() + 1, 1, 0, 0),
            &slice
        ));
    }

    #[test]
    fn missing_hierarchy_plan_never_exposes_overlapping_surface_meshes() {
        let surface = test_slice();
        assert!(!slice_owned_by_lod(None, None, &(99, 0, 0, 0), &surface));
        assert!(slice_owned_by_lod(None, None, &(0, 0, 0, 0), &surface));
        assert!(!slice_owned_by_lod(
            Some(GeometricLodFocus::snapped(0, 0)),
            None,
            &(99, 0, 0, 0),
            &surface
        ));

        let mut arena = ArenaAllocator::new(256, 1);
        let allocation = arena.allocate(surface.size).expect("test mesh allocation");
        let chunk = ChunkMesh {
            allocation,
            morph_allocation: None,
            quad_count: surface.quad_count,
            content_fingerprint: 1,
            slices: vec![surface],
            lod_ownership_focus: None,
            lod_ownership_stale: true,
            lod_owned_slices: Vec::new(),
            bounds_min: surface.bounds_min,
            bounds_max: surface.bounds_max,
            activation_mask: u8::MAX,
        };
        assert!(chunk.lod_owns_slice(&(0, 0, 0, 0), None, 0));
        assert!(!chunk.lod_owns_slice(&(SurfaceLodLevel::Stride2.index() + 1, 0, 0, 0), None, 0));
    }

    #[test]
    fn disabling_far_terrain_keeps_resident_canonical_coverage() {
        let settled = Some(GeometricLodFocus::snapped(0, 0));
        let canonical = test_slice();
        let outside_inner_cut = (0, 4, 0, 0);
        let plan = LodDrawPlan::default();

        assert!(!slice_owned_by_lod(
            active_geometric_lod_focus(settled, true),
            Some(&plan),
            &outside_inner_cut,
            &canonical
        ));
        assert!(slice_owned_by_lod(
            active_geometric_lod_focus(settled, false),
            None,
            &outside_inner_cut,
            &canonical
        ));
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
                && quad.material_face & !GPU_FACE_MASK == u32::from(Material::Stone.id())
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
                && quad.material_face & !GPU_FACE_MASK == u32::from(Material::Basalt.id())
        }));
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
            position: [0; 3],
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
        let vertices = virtual_triangle_gpu_vertices(&page).expect("triangle conversion");
        assert_eq!(
            vertices
                .iter()
                .map(|vertex| vertex.position)
                .collect::<Vec<_>>(),
            [[0, 0, 0], [2, 0, 0], [0, 3, 0]]
        );
        assert!(
            vertices
                .iter()
                .all(|vertex| vertex.material == u32::from(Material::Basalt.id()))
        );
        assert!(
            vertices
                .iter()
                .all(|vertex| vertex.normal == [0, 0, i16::MAX, 0])
        );
    }

    #[test]
    fn horizon_only_surface_levels_never_enter_directional_shadow_passes() {
        assert!(mesh_casts_directional_shadow(&(0, 0, 0, 0)));
        assert!(mesh_casts_directional_shadow(&(
            SurfaceLodLevel::Stride16.index() + 1,
            0,
            0,
            0,
        )));
        assert!(!mesh_casts_directional_shadow(&(
            SurfaceLodLevel::Stride32.index() + 1,
            0,
            0,
            0,
        )));
        assert!(!mesh_casts_directional_shadow(&(
            SurfaceLodLevel::Stride64.index() + 1,
            0,
            0,
            0,
        )));
        assert!(!mesh_casts_directional_shadow(&(
            SurfaceLodLevel::Stride128.index() + 1,
            0,
            0,
            0,
        )));
        assert!(!mesh_casts_directional_shadow(&(
            SurfaceLodLevel::Stride256.index() + 1,
            0,
            0,
            0,
        )));
    }

    fn virtual_cut_with_selected(selected_pages: Vec<TerrainPageKey>) -> VirtualTerrainCut {
        VirtualTerrainCut {
            selected_pages,
            requested_pages: Vec::new(),
            ownerless_roots: Vec::new(),
            fingerprint: 1,
            visited_nodes: 1,
            selected_primitives: 0,
            selected_encoded_bytes: 0,
            feedback_overflow: false,
            selection_overflow: false,
            traversal_overflow: false,
            incoherent_replacement_groups: 0,
        }
    }

    #[test]
    fn virtual_ownership_rejects_an_incomplete_root_partition() {
        let root = TerrainPageKey {
            level: TERRAIN_REGION_ROOT_LEVEL,
            coord: [-1, 0, 2],
        };
        let incomplete = root.children().unwrap()[..4].to_vec();
        assert_eq!(
            VirtualTerrainOwnership::from_cut(&virtual_cut_with_selected(incomplete)),
            Err(VirtualTerrainRendererError::IncompleteRootPartition(root))
        );
    }

    #[test]
    fn virtual_ownership_covers_only_complete_half_open_root_volumes() {
        let root = TerrainPageKey {
            level: TERRAIN_REGION_ROOT_LEVEL,
            coord: [-1, 0, 2],
        };
        let ownership = VirtualTerrainOwnership::from_cut(&virtual_cut_with_selected(
            root.children().unwrap().to_vec(),
        ))
        .unwrap();
        assert!(ownership.covers_aabb(
            glam::Vec3::new(-25.6, 0.0, 51.2),
            glam::Vec3::new(0.0, 25.6, 76.8),
        ));
        assert!(ownership.intersects_aabb(
            glam::Vec3::new(-1.0, 1.0, 52.0),
            glam::Vec3::new(1.0, 2.0, 53.0),
        ));
        assert!(!ownership.covers_aabb(
            glam::Vec3::new(-1.0, 1.0, 52.0),
            glam::Vec3::new(1.0, 2.0, 53.0),
        ));
        assert!(!ownership.intersects_aabb(
            glam::Vec3::new(0.0, 0.0, 51.2),
            glam::Vec3::new(25.6, 25.6, 76.8),
        ));
    }
}
