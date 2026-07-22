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
    GeometricLodFocus, LOD_BOUNDARY_HALF_EXTENTS, LodOwner, SurfacePatchSelection,
    incomplete_resident_parents, lod_boundary_half_extents_are_valid,
};
use crate::material_detail::MaterialDetailGpu;
use crate::shadow::{
    AabbClipClassification, AabbClipVolume, CASCADE_COUNT, DirectionalShadowBasis,
    DirectionalShadowCascades, DirectionalShadowConfig, ShadowDirectionTracker,
    build_directional_shadow_cascades,
};
use crate::ui::{Color, InventoryItem, LiveStats, MissionControlUi, UiAction, UiKey, Viewport};
pub use crate::ui::{MissionControlConfig, RendererFeatureConfig};
use crate::ui_gpu::{SCENE_FORMAT, UiGpu, texture_sampler_layout};
use bytemuck::{Pod, Zeroable};
use hashbrown::{HashMap, HashSet};
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use voxels_core::{CameraState, EnclosureSample, RemoteAvatarPose};
use voxels_world::protocol::{EditShape, EditVolume};
use voxels_world::{
    AtmosphereSample, CHUNK_EDGE, CelestialObservation, Chunk, ChunkCoord, Material, MeshedChunk,
    Quad, RenderLayer, SURFACE_PATCHES_PER_TILE_EDGE, SurfaceLodLevel, SurfacePatchEdge,
    SurfacePatchId, SurfaceRegion, SurfaceTileCoord, SurfaceTileMesh, VOXEL_SIZE_METRES,
    WaterTileMesh,
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
const FAR_MATERIAL_FLAG: u32 = 1 << 31;
const SURFACE_LOD_SHIFT: u32 = 27;
const GPU_FACE_SHIFT: u32 = 16;
const GPU_FACE_MASK: u32 = 0b111 << GPU_FACE_SHIFT;
const CANONICAL_RASTER_COVERAGE_FLAG: u32 = 1 << 23;
const SURFACE_MACRO_NORMAL_FLAG: u32 = 1 << 24;
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
const CUT_TRANSITION_SECONDS: f32 = 0.24;
const LOD_PLAN_REBUILD_FOCUS: u32 = 1;
const LOD_PLAN_REBUILD_CANONICAL_COLUMNS: u32 = 1 << 1;
const LOD_PLAN_REBUILD_CANONICAL_PROFILE: u32 = 1 << 2;
const LOD_PLAN_REBUILD_SURFACE_RESIDENCY: u32 = 1 << 3;
const LOD_PLAN_REBUILD_SURFACE_PROFILE: u32 = 1 << 4;
const LOD_PLAN_REBUILD_ENCLOSED_VIEW: u32 = 1 << 5;
const LOD_PLAN_REBUILD_CANONICAL_VOLUME: u32 = 1 << 6;
const GPU_QUERY_COUNT: u32 = 24;
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
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuQuad {
    origin: [i32; 3],
    extent_voxels: [u16; 2],
    material_face: u32,
    ao: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuCutTransition {
    /// x is the normalized transition phase; y is 0 stable, 1 outgoing, or 2 incoming.
    phase_role: [f32; 4],
}

const _: () = assert!(size_of::<GpuCutTransition>() == 16);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SurfaceCell {
    height: i32,
    material: Material,
    macro_normal: u32,
    horizon_profile: u16,
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
    morph_heights: Vec<u32>,
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
    debug_assert!(face <= 5);
    material | (u32::from(face) << GPU_FACE_SHIFT)
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

    fn owns_source_edge(&self, patch: SurfacePatchId, edge: SurfacePatchEdge) -> bool {
        self.owns_patch(patch)
            && !self
                .exact_transition_edges
                .contains(&(patch, edge.index() as u8))
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

#[derive(Clone, Copy, Debug)]
struct MeshSlice {
    relative_offset: u32,
    size: u32,
    quad_count: u32,
    bounds_min: glam::Vec3,
    bounds_max: glam::Vec3,
    surface_patch_id: Option<SurfacePatchId>,
    boundary_edge: Option<SurfacePatchEdge>,
    morph_closure: bool,
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
    stable: WorldDrawLists,
    outgoing: WorldDrawLists,
    incoming: WorldDrawLists,
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
            morph_offset: morph_allocation.offset + first_quad * size_of::<u32>() as u32,
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
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
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
    water_pipeline: RenderPipeline,
    weather_pipeline: RenderPipeline,
    avatar_gpu: AvatarGpu,
    remote_avatars: Vec<RemoteAvatarPose>,
    water_scene_layout: wgpu::BindGroupLayout,
    water_scene_bind_group: BindGroup,
    shadow_gpu: ShadowGpu,
    shadow_direction: ShadowDirectionTracker,
    frame_buffer: Buffer,
    frame_bind_group: BindGroup,
    cut_transition_buffers: [Buffer; 3],
    cut_transition_bind_groups: [BindGroup; 3],
    local_light_buffer: Buffer,
    material_detail: MaterialDetailGpu,
    chunks: BTreeMap<MeshKey, ChunkMesh>,
    water_chunks: BTreeMap<MeshKey, ChunkMesh>,
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
    cut_transition: Option<CutTransition>,
    chunk_activations: ChunkActivations,
    local_light_candidates: BTreeMap<MeshKey, Vec<GpuLocalLight>>,
    arena: ArenaAllocator,
    arena_buffers: Vec<Buffer>,
    water_arena: ArenaAllocator,
    water_arena_buffers: Vec<Buffer>,
    depth_view: TextureView,
    ambient_occlusion_gpu: AmbientOcclusionGpu,
    volumetric_cloud_gpu: VolumetricCloudGpu,
    time: f32,
    diagnostics: RenderDiagnostics,
    gpu_timer: Option<GpuTimer>,
    target_voxel: Option<[i32; 3]>,
    target_volume: Option<EditVolume>,
    edit_shape: EditShape,
    options: RenderOptions,
    environment: OutdoorEnvironment,
    server_world_environment: WorldEnvironmentState,
    debug_environment_override: DebugEnvironmentOverride,
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
    screenshot_readback: Arc<Mutex<ScreenshotReadbackState>>,
    host_ui_action: Option<HostUiAction>,
    underwater_blend: f32,
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
    environment: OutdoorEnvironment,
    world_environment: WorldEnvironmentState,
    celestial_observation: CelestialObservation,
    underwater_blend: f32,
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
        let timestamp_queries = adapter.features().contains(Features::TIMESTAMP_QUERY);
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
                environment,
                world_environment,
                celestial_observation,
                underwater_blend: 0.0,
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
        let depth_view = depth_view(&device, config.width, config.height);
        let ambient_occlusion_gpu = AmbientOcclusionGpu::new(
            &device,
            &frame_layout,
            &depth_view,
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
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let cut_transition_buffers = std::array::from_fn(|role| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("complete cut transition uniform"),
                contents: bytemuck::bytes_of(&GpuCutTransition {
                    phase_role: [1.0, role as f32, 0.0, 0.0],
                }),
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
        let water_scene_layout = texture_sampler_layout(&device, "water refraction scene layout");
        let water_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("water pipeline layout"),
                bind_group_layouts: &[Some(&frame_layout), Some(&water_scene_layout)],
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
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
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
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
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
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                fragment_constants: &[],
            },
        );

        let ui_gpu = UiGpu::new(&device, format, config.width, config.height, dpr)?;
        let water_scene_bind_group = ui_gpu.refraction_bind_group(&device, &water_scene_layout);

        let placement_inventory = PlacementInventory::new();
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
            water_pipeline,
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
            cut_transition: None,
            chunk_activations: ChunkActivations::default(),
            local_light_candidates: BTreeMap::new(),
            arena: ArenaAllocator::new(ARENA_PAGE_BYTES, size_of::<GpuQuad>() as u32),
            arena_buffers: Vec::new(),
            water_arena: ArenaAllocator::new(ARENA_PAGE_BYTES, size_of::<GpuQuad>() as u32),
            water_arena_buffers: Vec::new(),
            depth_view,
            ambient_occlusion_gpu,
            volumetric_cloud_gpu,
            time: 0.0,
            diagnostics: RenderDiagnostics::default(),
            gpu_timer,
            target_voxel: None,
            target_volume: None,
            edit_shape: EditShape::Sphere,
            options,
            environment,
            server_world_environment: world_environment,
            debug_environment_override: DebugEnvironmentOverride::default(),
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
            screenshot_readback: Arc::new(Mutex::new(ScreenshotReadbackState::default())),
            host_ui_action: None,
            underwater_blend: 0.0,
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
            self.depth_view = depth_view(&self.device, width, height);
            self.ambient_occlusion_gpu
                .resize(&self.device, &self.depth_view, width, height);
            self.volumetric_cloud_gpu
                .resize(&self.device, width, height);
        }
        self.dpr = dpr;
        if self
            .ui_gpu
            .resize(&self.device, &self.queue, width, height, self.dpr)
        {
            self.water_scene_bind_group = self
                .ui_gpu
                .refraction_bind_group(&self.device, &self.water_scene_layout);
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
        self.world_environment = self
            .debug_environment_override
            .apply(self.server_world_environment);
    }

    fn refresh_effective_environment(&mut self) -> bool {
        let state = self
            .debug_environment_override
            .apply(self.server_world_environment);
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

    /// Replaces the exact canonical chunks in complete current vertical bands.
    ///
    /// This set owns only exact-volume meshes. It deliberately does not suppress the heightfield
    /// fallback: an all-air band around a high spectator or a deep tunnel is complete volume data,
    /// but says nothing about whether the terrain surface in the same X/Z column is represented.
    pub fn set_canonical_ready_chunks(
        &mut self,
        chunks: impl IntoIterator<Item = (i32, i32, i32)>,
    ) {
        let replacement = chunks.into_iter().collect::<HashSet<_>>();
        if replacement == self.canonical_ready_chunks {
            return;
        }
        let mut changed_chunks = self
            .canonical_ready_chunks
            .symmetric_difference(&replacement)
            .copied()
            .collect::<HashSet<_>>();
        self.canonical_ready_chunks = replacement;
        changed_chunks.retain(|&(x, _, z)| {
            self.lod_draw_plan_focus
                .is_some_and(|focus| focus.owns_canonical_chunk(x, z))
        });
        if changed_chunks.is_empty() {
            return;
        }
        for (key, mesh) in &mut self.chunks {
            if key.0 == 0 && changed_chunks.contains(&(key.1, key.2, key.3)) {
                mesh.lod_ownership_stale = true;
            }
        }
        self.invalidate_lod_draw_plan(LOD_PLAN_REBUILD_CANONICAL_VOLUME);
    }

    /// Replaces the exact chunks that form a complete terrain-following surface cut.
    ///
    /// Only this independently proven set may suppress stride-two fallback patches. The shell
    /// derives it from surface-height hints and publishes a column only after every requested Y
    /// chunk is renderable, so unrelated air, tunnel, and interaction bands cannot punch square
    /// holes through the terrain when viewed from above.
    pub fn set_canonical_surface_ready_chunks(
        &mut self,
        chunks: impl IntoIterator<Item = (i32, i32, i32)>,
    ) {
        let replacement = chunks.into_iter().collect::<HashSet<_>>();
        if replacement == self.canonical_surface_ready_chunks {
            return;
        }
        let mut changed_columns =
            changed_canonical_ready_columns(&self.canonical_surface_ready_chunks, &replacement);
        self.canonical_surface_ready_chunks = replacement;
        changed_columns.retain(|(x, z)| {
            self.lod_draw_plan_focus
                .is_some_and(|focus| focus.owns_canonical_chunk(*x, *z))
        });
        if changed_columns.is_empty() {
            return;
        }
        for (key, mesh) in &mut self.chunks {
            if key.0 == 0 && changed_columns.contains(&(key.1, key.3)) {
                mesh.lod_ownership_stale = true;
            }
        }
        self.invalidate_lod_draw_plan(LOD_PLAN_REBUILD_CANONICAL_COLUMNS);
    }

    /// Whether an exact canonical chunk currently owns its LOD cells.
    ///
    /// Automation uses this to verify that an edit replacement never relinquishes render
    /// ownership while its previous uploaded mesh is still the correct transactional fallback.
    pub fn canonical_chunk_owned(&self, coord: ChunkCoord) -> bool {
        self.canonical_ready_chunks
            .contains(&(coord.x, coord.y, coord.z))
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
            UiAction::TakeScreenshot => {
                if !self.screenshot_pending() {
                    self.screenshot_requested = true;
                }
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

    pub fn take_host_ui_action(&mut self) -> Option<HostUiAction> {
        self.host_ui_action.take()
    }

    pub fn set_spectator_active(&mut self, active: bool) {
        self.ui.set_spectator_active(active);
    }

    pub fn set_spectator_available(&mut self, available: bool) {
        self.ui.set_spectator_available(available);
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
        let convert = |quad: &Quad, conservative_coverage: bool| GpuQuad {
            origin: [
                origin[0] + i32::from(quad.origin[0]),
                origin[1] + i32::from(quad.origin[1]),
                origin[2] + i32::from(quad.origin[2]),
            ],
            extent_voxels: quad.extent.map(u16::from),
            material_face: pack_gpu_material_face(u32::from(quad.material), quad.face),
            ao: u32::from(quad.ao)
                | if conservative_coverage {
                    CANONICAL_RASTER_COVERAGE_FLAG
                } else {
                    0
                },
        };
        // Greedy canonical terrain has intentional T-junctions where one long quad meets several
        // shorter neighbors. Mark only its opaque faces for subpixel conservative coverage; water
        // remains translucent and must not overlap its own alpha edges.
        let opaque_quads: Vec<_> = mesh.opaque.iter().map(|quad| convert(quad, true)).collect();
        let water_quads: Vec<_> = mesh
            .translucent
            .iter()
            .map(|quad| convert(quad, false))
            .collect();
        let min = glam::Vec3::from_array(origin.map(|value| value as f32 * VOXEL_SIZE_METRES));
        let max = min + glam::Vec3::splat(CHUNK_EDGE as f32 * VOXEL_SIZE_METRES);
        let quad_bytes = size_of::<GpuQuad>() as u32;
        let opaque_count = mesh.opaque.len() as u32;
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
                    morph_closure: false,
                    render_layer: RenderLayer::Opaque,
                }],
            )?;
            Some(prepared)
        };
        let translucent_count = mesh.translucent.len() as u32;
        let water_update = if translucent_count == 0 {
            None
        } else {
            let Some(prepared) = self.prepare_water_mesh_sliced(
                key,
                &water_quads,
                vec![MeshSlice {
                    relative_offset: 0,
                    size: translucent_count * quad_bytes,
                    quad_count: translucent_count,
                    bounds_min: min,
                    bounds_max: max,
                    surface_patch_id: None,
                    boundary_edge: None,
                    morph_closure: false,
                    render_layer: RenderLayer::Translucent,
                }],
            ) else {
                discard_prepared_mesh(&mut self.arena, opaque_update);
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
        let macro_normals = surface_macro_normals(tile);
        let horizon_profiles = surface_horizon_profiles(tile);
        let geometry_morphs = surface_geometry_morphs(tile, &macro_normals);
        let patch_profiles = surface_patch_profiles(tile, &macro_normals, &horizon_profiles);
        let mut gpu_quads: Vec<_> = tile
            .quads
            .iter()
            .zip(macro_normals.iter().copied())
            .zip(horizon_profiles.iter().copied())
            .map(|((quad, macro_normal), horizon_profile)| GpuQuad {
                origin: quad.origin,
                extent_voxels: quad.extent,
                material_face: pack_surface_horizon_material(
                    pack_gpu_material_face(
                        u32::from(quad.material.id())
                            | FAR_MATERIAL_FLAG
                            | (u32::from(coord.level.index()) << SURFACE_LOD_SHIFT),
                        quad.face,
                    ),
                    horizon_profile,
                ),
                ao: pack_surface_horizon_ao(macro_normal, horizon_profile),
            })
            .collect();
        let mut gpu_morph_heights = geometry_morphs;
        let closure_base = gpu_quads.len() as u32;
        for (quad, morph_heights) in
            surface_morph_closure_gpu_quads(tile, &macro_normals, &horizon_profiles)
        {
            gpu_quads.push(quad);
            gpu_morph_heights.push(morph_heights);
        }
        let water_gpu_quads: Vec<_> = water
            .quads
            .iter()
            .map(|quad| GpuQuad {
                origin: quad.origin,
                extent_voxels: quad.extent,
                material_face: pack_gpu_material_face(
                    u32::from(quad.material.id())
                        | FAR_MATERIAL_FLAG
                        | (u32::from(coord.level.index()) << SURFACE_LOD_SHIFT),
                    quad.face,
                ),
                ao: 0xff,
            })
            .collect();
        if gpu_quads_match_resident(self.chunks.get(&key), &gpu_quads, Some(&gpu_morph_heights))
            && gpu_quads_match_resident(self.water_chunks.get(&key), &water_gpu_quads, None)
        {
            // Underground edits commonly dirty the enclosing stride-two transport tile without
            // changing its top surface. Preserve the exact resident GPU products and LOD plan in
            // that case instead of reallocating identical bytes and invalidating every slice.
            return true;
        }
        let quad_bytes = size_of::<GpuQuad>() as u32;
        let mut slices = Vec::new();
        for patch in &tile.patches {
            let Some(patch_id) = SurfacePatchId::from_tile_cell_min(
                coord,
                [patch.cell_bounds[0][0], patch.cell_bounds[0][1]],
            ) else {
                continue;
            };
            let bounds_min = glam::Vec3::from_array(
                patch
                    .bounds
                    .min
                    .map(|value| value as f32 * VOXEL_SIZE_METRES),
            );
            let bounds_max = glam::Vec3::from_array(
                patch
                    .bounds
                    .max
                    .map(|value| value as f32 * VOXEL_SIZE_METRES),
            );
            let mut push_slice = |range: std::ops::Range<u32>, boundary_edge, morph_closure| {
                if range.start < range.end {
                    slices.push(MeshSlice {
                        relative_offset: range.start * quad_bytes,
                        size: (range.end - range.start) * quad_bytes,
                        quad_count: range.end - range.start,
                        bounds_min,
                        bounds_max,
                        surface_patch_id: Some(patch_id),
                        boundary_edge,
                        morph_closure,
                        render_layer: RenderLayer::Opaque,
                    });
                }
            };
            push_slice(patch.quad_range.clone(), None, false);
            for edge in SurfacePatchEdge::ALL {
                push_slice(patch.edge_ranges[edge.index()].clone(), Some(edge), false);
            }
            push_slice(
                offset_range(&patch.morph_closure_range, closure_base),
                None,
                true,
            );
            for edge in SurfacePatchEdge::ALL {
                push_slice(
                    offset_range(&patch.edge_morph_closure_ranges[edge.index()], closure_base),
                    Some(edge),
                    true,
                );
            }
        }
        let water_slices = water
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
                    morph_closure: false,
                    render_layer: RenderLayer::Translucent,
                }
            })
            .collect();
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
                discard_prepared_mesh(&mut self.arena, opaque_update);
                return false;
            };
            Some(prepared)
        };
        commit_prepared_mesh(&mut self.arena, &mut self.chunks, key, opaque_update);
        commit_prepared_mesh(
            &mut self.water_arena,
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
        morph_heights: Option<&[u32]>,
        slices: Vec<MeshSlice>,
    ) -> Option<ChunkMesh> {
        let activation_mask = self.chunk_activations.upload_mask(key);
        prepare_mesh_sliced_into(
            &self.device,
            &self.queue,
            &mut self.arena,
            &mut self.arena_buffers,
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
        self.remove_mesh((coord.level.index() + 1, coord.x, 0, coord.z));
        self.surface_patch_profiles
            .retain(|patch, _| !surface_patch_belongs_to_tile(*patch, coord));
        self.replace_surface_patch_residency(coord, HashSet::new());
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
        let canonical_chunks =
            canonical_ready_chunks_for_focus(focus, &self.canonical_ready_chunks);
        let surface_hierarchy_reasons = LOD_PLAN_REBUILD_FOCUS
            | LOD_PLAN_REBUILD_CANONICAL_COLUMNS
            | LOD_PLAN_REBUILD_CANONICAL_PROFILE
            | LOD_PLAN_REBUILD_SURFACE_RESIDENCY
            | LOD_PLAN_REBUILD_SURFACE_PROFILE;
        if rebuild_reason & surface_hierarchy_reasons == 0 {
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
            let previous_plan_resident = self.lod_draw_plan_is_resident();
            let mut next_plan = self.lod_draw_plan.clone();
            next_plan.canonical_chunks = canonical_chunks;
            next_plan.enclosed_view_chunks = self.enclosed_view_ready_chunks.clone();
            if next_plan != self.lod_draw_plan
                && self.lod_draw_plan.has_geometry()
                && previous_plan_resident
                && self.lod_draw_plan_focus.is_some()
                && focus.is_some()
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
            return rebuild_reason;
        }
        let canonical_surface_chunks =
            canonical_ready_chunks_for_focus(focus, &self.canonical_surface_ready_chunks);
        let canonical_columns = canonical_surface_chunks
            .iter()
            .map(|&(x, _, z)| (x, z))
            .collect::<HashSet<_>>();
        let mut patches = SurfacePatchSelection::default();
        if let Some(focus) = focus {
            patches.rebuild_with_incomplete_parents(
                focus,
                &self.surface_patch_residency,
                &canonical_columns,
                &self.surface_incomplete_parents,
            );
        }
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
        let previous_plan_resident = self.lod_draw_plan_is_resident();
        let next_plan = LodDrawPlan {
            patches,
            canonical_columns,
            canonical_chunks,
            enclosed_view_chunks: self.enclosed_view_ready_chunks.clone(),
            exact_transition_edges,
            incomplete_transition_edges,
            transition_mesh_key,
        };
        if next_plan != self.lod_draw_plan
            && self.lod_draw_plan.has_geometry()
            && previous_plan_resident
            && self.lod_draw_plan_focus.is_some()
            && focus.is_some()
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
        let surface_resident = self
            .lod_draw_plan
            .patches
            .owned_patches()
            .all(|patch| self.surface_patch_residency.contains(&patch));
        let canonical_resident = self
            .lod_draw_plan
            .canonical_chunks
            .iter()
            .all(|&(x, y, z)| {
                self.chunks
                    .get(&(0, x, y, z))
                    .is_some_and(ChunkMesh::active)
            });
        let enclosed_resident = self
            .lod_draw_plan
            .enclosed_view_chunks
            .iter()
            .all(|key| self.chunks.contains_key(&(0, key.0, key.1, key.2)));
        let connector_resident = self
            .lod_draw_plan
            .transition_mesh_key
            .is_none_or(|key| self.chunks.contains_key(&key));
        surface_resident && canonical_resident && enclosed_resident && connector_resident
    }

    fn publish_lod_transition_mesh(
        &mut self,
        gpu_quads: &[GpuQuad],
        morph_heights: &[u32],
    ) -> Result<Option<MeshKey>, ()> {
        if gpu_quads.is_empty() {
            return Ok(None);
        }
        let active = self.lod_draw_plan.transition_mesh_key;
        let key = if active == Some(LOD_TRANSITION_MESH_KEYS[0]) {
            LOD_TRANSITION_MESH_KEYS[1]
        } else {
            LOD_TRANSITION_MESH_KEYS[0]
        };
        if gpu_quads_match_resident(self.chunks.get(&key), gpu_quads, Some(morph_heights)) {
            return Ok(Some(key));
        }
        let Some((bounds_min, bounds_max)) = gpu_quad_bounds(gpu_quads) else {
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
            morph_closure: false,
            render_layer: RenderLayer::Opaque,
        };
        let Some(prepared) =
            self.prepare_mesh_sliced(key, gpu_quads, Some(morph_heights), vec![slice])
        else {
            return Err(());
        };
        commit_prepared_mesh(&mut self.arena, &mut self.chunks, key, Some(prepared));
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
            ((self.time - transition.started_at) / CUT_TRANSITION_SECONDS).clamp(0.0, 1.0)
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
            for role in 1..=2 {
                self.queue.write_buffer(
                    &self.cut_transition_buffers[role],
                    0,
                    bytemuck::bytes_of(&GpuCutTransition {
                        phase_role: [phase, role as f32, 0.0, 0.0],
                    }),
                );
            }
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
                let _ = self.arena.free(morph_allocation);
            }
        }
    }

    fn remove_water_mesh(&mut self, key: MeshKey) {
        if let Some(chunk) = self.water_chunks.remove(&key) {
            let _ = self.water_arena.free(chunk.allocation);
            if let Some(morph_allocation) = chunk.morph_allocation {
                let _ = self.water_arena.free(morph_allocation);
            }
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
        let target_underwater = f32::from(camera.fluid_state().eyes_submerged);
        let response_seconds = if target_underwater > self.underwater_blend {
            0.12
        } else {
            0.22
        };
        let response = 1.0 - (-dt / response_seconds).exp();
        self.underwater_blend += (target_underwater - self.underwater_blend) * response;
        if (target_underwater - self.underwater_blend).abs() < 0.000_5 {
            self.underwater_blend = target_underwater;
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
                environment: self.environment,
                world_environment: self.world_environment,
                celestial_observation: self.celestial_observation,
                underwater_blend: self.underwater_blend,
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
        let _ = self.maintain_cut_transition(resident_hierarchy);
        let cpu_lod_plan_ms = (now_ms() - lod_plan_started).max(0.0) as f32;
        // Queue readiness is not a proof that every fixed geometric owner is resident. Canonical
        // columns can still replace atomically and retained surface tiles can be incomplete. Keep
        // the cached resident hierarchy authoritative after settling as well as while streaming.
        let lod_draw_plan = resident_hierarchy.then_some(&self.lod_draw_plan);
        let Ok((shadow_draw_lists, world_draw_list, lod_ownership_refreshes)) =
            collect_opaque_draw_lists(
                &mut self.chunks,
                lod_draw_plan,
                self.options.far_terrain,
                shadows_active,
                geometric_lod_focus,
                view_clip,
                shadow_clips,
            )
        else {
            return false;
        };
        let cut_draw_lists = if let Some(transition) = self.cut_transition.as_ref() {
            let Ok(draw_lists) = collect_cut_transition_draw_lists(
                &self.chunks,
                &self.lod_draw_plan,
                geometric_lod_focus,
                transition,
                self.options.far_terrain,
                view_clip,
            ) else {
                return false;
            };
            Some(draw_lists)
        } else {
            None
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
                    && view_clip.contains_aabb(slice.bounds_min, slice.bounds_max)
            },
        );
        let cpu_cull_ms = (now_ms() - cull_started).max(0.0) as f32;
        let encode_started = now_ms();
        self.avatar_gpu
            .prepare(&self.queue, &self.remote_avatars, self.time);
        let avatar_instances = self.avatar_gpu.instance_count();
        let has_avatars = avatar_instances != 0;
        let refract_water = !water_draw_list.spans.is_empty();
        let diagnostic_sky = self.runtime_config.diagnostic_sky_color.is_some();
        let clouds_active = self.volumetric_cloud_gpu.enabled() && !diagnostic_sky;
        let weather_active = self.environment.precipitation > 0.002 && !diagnostic_sky;
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
        let gpu_frame = self.gpu_timer.as_mut().and_then(|timer| {
            timer.begin_frame(
                frame_id,
                GpuPassMask {
                    shadows: shadows_active,
                    water: refract_water,
                    ambient_occlusion: self.options.screen_space_ambient_occlusion,
                    clouds: clouds_active,
                    weather: weather_active,
                },
            )
        });
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
                    &draw_list.morphing,
                ));
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
                        view: &self.depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: gpu_frame.as_ref().map(|frame| frame.pass(6)),
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&self.depth_prepass_fast_pipeline);
                pass.set_bind_group(0, &self.frame_bind_group, &[]);
                if let Some(cut_draw_lists) = &cut_draw_lists {
                    depth_prepass_draw_calls = depth_prepass_draw_calls.saturating_add(draw_spans(
                        &mut pass,
                        &self.arena_buffers,
                        &cut_draw_lists.stable.fixed,
                    ));
                    pass.set_pipeline(&self.depth_prepass_morph_pipeline);
                    depth_prepass_draw_calls =
                        depth_prepass_draw_calls.saturating_add(draw_morph_spans(
                            &mut pass,
                            &self.arena_buffers,
                            &cut_draw_lists.stable.morphing,
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
                            &cut_draw_lists.outgoing.morphing,
                        ));
                    pass.set_pipeline(&self.depth_prepass_transition_fixed_pipeline);
                    pass.set_bind_group(3, &self.cut_transition_bind_groups[2], &[]);
                    depth_prepass_draw_calls = depth_prepass_draw_calls.saturating_add(draw_spans(
                        &mut pass,
                        &self.arena_buffers,
                        &cut_draw_lists.incoming.fixed,
                    ));
                    pass.set_pipeline(&self.depth_prepass_transition_pipeline);
                    depth_prepass_draw_calls =
                        depth_prepass_draw_calls.saturating_add(draw_morph_spans(
                            &mut pass,
                            &self.arena_buffers,
                            &cut_draw_lists.incoming.morphing,
                        ));
                } else {
                    depth_prepass_draw_calls = depth_prepass_draw_calls.saturating_add(draw_spans(
                        &mut pass,
                        &self.arena_buffers,
                        &world_draw_list.fixed,
                    ));
                    pass.set_pipeline(&self.depth_prepass_morph_pipeline);
                    depth_prepass_draw_calls = depth_prepass_draw_calls.saturating_add(
                        draw_morph_spans(&mut pass, &self.arena_buffers, &world_draw_list.morphing),
                    );
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
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: if self.options.screen_space_ambient_occlusion {
                            wgpu::LoadOp::Load
                        } else {
                            wgpu::LoadOp::Clear(1.0)
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
            if let Some(cut_draw_lists) = &cut_draw_lists {
                pass.set_pipeline(fixed_pipeline);
                draw_spans(&mut pass, &self.arena_buffers, &cut_draw_lists.stable.fixed);
                pass.set_pipeline(morph_pipeline);
                draw_morph_spans(
                    &mut pass,
                    &self.arena_buffers,
                    &cut_draw_lists.stable.morphing,
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
                    &cut_draw_lists.outgoing.morphing,
                );
                pass.set_pipeline(transition_pipeline);
                pass.set_bind_group(3, &self.cut_transition_bind_groups[2], &[]);
                draw_spans(
                    &mut pass,
                    &self.arena_buffers,
                    &cut_draw_lists.incoming.fixed,
                );
                pass.set_pipeline(morph_transition_pipeline);
                draw_morph_spans(
                    &mut pass,
                    &self.arena_buffers,
                    &cut_draw_lists.incoming.morphing,
                );
            } else {
                pass.set_pipeline(fixed_pipeline);
                draw_spans(&mut pass, &self.arena_buffers, &world_draw_list.fixed);
                pass.set_pipeline(morph_pipeline);
                draw_morph_spans(&mut pass, &self.arena_buffers, &world_draw_list.morphing);
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
                &self.depth_view,
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
                    view: &self.depth_view,
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
                    view: &self.depth_view,
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
        let scene_pixels = u64::from(self.config.width) * u64::from(self.config.height);
        let shadow_resolution = u64::from(
            self.runtime_config
                .directional_shadows
                .shadow_map_resolution,
        );
        let shadow_bytes = shadow_resolution * shadow_resolution * CASCADE_COUNT as u64 * 4;
        let gpu_timing = self.gpu_timer.as_ref().and_then(GpuTimer::latest);
        self.diagnostics = RenderDiagnostics {
            resident_chunks: self.chunks.len() as u32,
            visible_chunks: world_draw_list.mesh_count,
            draw_calls: world_draw_list
                .fixed
                .spans
                .len()
                .saturating_add(world_draw_list.morphing.spans.len())
                .saturating_add(water_draw_list.spans.len())
                .saturating_add(usize::from(has_avatars)) as u32,
            water_draw_calls: water_draw_list.spans.len() as u32,
            shadow_draw_calls,
            shadow_cascades: if shadows_active {
                CASCADE_COUNT as u32
            } else {
                0
            },
            quads: world_draw_list
                .quad_count
                .saturating_add(water_draw_list.quad_count),
            water_quads: water_draw_list.quad_count,
            viewport_fingerprint: fingerprint_value(
                fingerprint_value(FINGERPRINT_OFFSET, world_draw_list.fingerprint),
                water_draw_list.fingerprint,
            ),
            refraction_copy_bytes: refraction_copy_bytes(
                self.config.width,
                self.config.height,
                refract_water,
            ),
            arena_pages: arena.pages.saturating_add(water_arena.pages) as u32,
            arena_capacity_bytes: arena
                .capacity_bytes
                .saturating_add(water_arena.capacity_bytes),
            arena_allocated_bytes: arena
                .allocated_bytes
                .saturating_add(water_arena.allocated_bytes),
            core_gpu_bytes: arena
                .capacity_bytes
                .saturating_add(water_arena.capacity_bytes)
                .saturating_add(scene_pixels.saturating_mul(20))
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
                .saturating_add(water_draw_list.tested_slices),
            draw_list_selected_slices: shadow_draw_lists
                .iter()
                .map(|draw_list| draw_list.selected_slices)
                .sum::<u32>()
                .saturating_add(world_draw_list.selected_slices)
                .saturating_add(water_draw_list.selected_slices),
            lod_transition_quads: self
                .lod_draw_plan
                .transition_mesh_key
                .and_then(|key| self.chunks.get(&key))
                .map_or(0, |mesh| mesh.quad_count),
            lod_incomplete_transition_edges: self.lod_draw_plan.incomplete_transition_edges,
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
        self.schedule_screenshot_readback(&mut encoder, screenshot_target.as_ref());
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
        let buffer_size = u64::from(padded_bytes_per_row) * u64::from(height);
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
        let filename = self.ui.screenshot_filename();
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
                        unpack_screenshot_rgba(&mapped, width, height, padded_bytes_per_row, bgra)
                            .map(|rgba| ScreenshotCapture {
                                filename,
                                width,
                                height,
                                rgba,
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
            debug_assert_eq!(
                chunk.allocation.size,
                chunk.quad_count * size_of::<GpuQuad>() as u32
            );
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

fn draw_morph_spans<'pass>(
    pass: &mut wgpu::RenderPass<'pass>,
    arena_buffers: &'pass [Buffer],
    draw_list: &DrawList,
) -> u32 {
    let mut draws = 0u32;
    for span in &draw_list.spans {
        let (Some(base_buffer), Some(morph_page)) =
            (arena_buffers.get(span.page as usize), span.morph_page)
        else {
            continue;
        };
        let Some(morph_buffer) = arena_buffers.get(morph_page as usize) else {
            continue;
        };
        let base_start = u64::from(span.offset);
        let base_end = base_start + u64::from(span.size);
        let morph_start = u64::from(span.morph_offset);
        let morph_end = morph_start + u64::from(span.quad_count) * size_of::<u32>() as u64;
        pass.set_vertex_buffer(0, base_buffer.slice(base_start..base_end));
        pass.set_vertex_buffer(1, morph_buffer.slice(morph_start..morph_end));
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
    far_terrain: bool,
    shadows: bool,
    geometric_lod_focus: Option<GeometricLodFocus>,
    view_clip: AabbClipVolume,
    shadow_clips: [AabbClipVolume; CASCADE_COUNT],
) -> Result<([WorldDrawLists; CASCADE_COUNT], WorldDrawLists, u32), MissingMorphSidecar> {
    let mut shadow_builders: [WorldDrawListBuilder; CASCADE_COUNT] =
        std::array::from_fn(|_| WorldDrawListBuilder::default());
    let mut world_builder = WorldDrawListBuilder::default();
    let mut lod_ownership_refreshes = 0u32;

    for (key, chunk) in chunks {
        if !chunk.active() || (key.0 != 0 && !far_terrain) {
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
            {
                continue;
            }
            if world_chunk_visible
                && (world_chunk_clip == AabbClipClassification::Inside
                    || view_clip.contains_aabb(slice.bounds_min, slice.bounds_max))
            {
                world_builder.select_slice(
                    chunk,
                    slice,
                    slice_uses_geometry_morph(key, geometric_lod_focus, slice),
                )?;
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

/// Splits only the camera-visible geometry whose complete-cut ownership changed. Stable clusters
/// stay on the ordinary single-draw path; the outgoing cut remains intact while the incoming cut
/// dithers over it for the short transition interval. This permits brief overdraw but cannot expose
/// the sky when two independently simplified cuts do not cover exactly the same pixels.
fn collect_cut_transition_draw_lists(
    chunks: &BTreeMap<MeshKey, ChunkMesh>,
    current_plan: &LodDrawPlan,
    current_focus: Option<GeometricLodFocus>,
    transition: &CutTransition,
    far_terrain: bool,
    view_clip: AabbClipVolume,
) -> Result<CutDrawLists, MissingMorphSidecar> {
    let mut stable = WorldDrawListBuilder::default();
    let mut outgoing = WorldDrawListBuilder::default();
    let mut incoming = WorldDrawListBuilder::default();
    for (key, chunk) in chunks {
        if !chunk.active()
            || (key.0 != 0 && !far_terrain)
            || !view_clip.contains_aabb(chunk.bounds_min, chunk.bounds_max)
        {
            continue;
        }
        let mut selected_mesh = [false; 3];
        for slice in &chunk.slices {
            stable.test_slice();
            outgoing.test_slice();
            incoming.test_slice();
            if slice.render_layer != RenderLayer::Opaque
                || !view_clip.contains_aabb(slice.bounds_min, slice.bounds_max)
            {
                continue;
            }
            let was_owned =
                slice_owned_by_lod(transition.from_focus, Some(&transition.from), key, slice);
            let is_owned = slice_owned_by_lod(current_focus, Some(current_plan), key, slice);
            let morphing = slice_uses_geometry_morph(key, current_focus, slice);
            match (was_owned, is_owned) {
                (true, true) => {
                    stable.select_slice(chunk, slice, morphing)?;
                    selected_mesh[0] = true;
                }
                (true, false) => {
                    outgoing.select_slice(chunk, slice, morphing)?;
                    selected_mesh[1] = true;
                }
                (false, true) => {
                    incoming.select_slice(chunk, slice, morphing)?;
                    selected_mesh[2] = true;
                }
                (false, false) => {}
            }
        }
        if selected_mesh[0] {
            stable.select_mesh(*key, chunk);
        }
        if selected_mesh[1] {
            outgoing.select_mesh(*key, chunk);
        }
        if selected_mesh[2] {
            incoming.select_mesh(*key, chunk);
        }
    }
    Ok(CutDrawLists {
        stable: stable.finish(),
        outgoing: outgoing.finish(),
        incoming: incoming.finish(),
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
    morph_heights: Option<&[u32]>,
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
    let bytes = bytemuck::cast_slice(gpu_quads);
    let Ok(byte_len) = u32::try_from(bytes.len()) else {
        return None;
    };
    let allocation = arena.allocate(byte_len)?;
    let morph_bytes = morph_heights.map(bytemuck::cast_slice::<u32, u8>);
    let morph_allocation = if let Some(morph_bytes) = morph_bytes {
        let Ok(morph_byte_len) = u32::try_from(morph_bytes.len()) else {
            let _ = arena.free(allocation);
            return None;
        };
        let Some(morph_allocation) = arena.allocate(morph_byte_len) else {
            let _ = arena.free(allocation);
            return None;
        };
        Some(morph_allocation)
    } else {
        None
    };
    let highest_page =
        morph_allocation.map_or(allocation.page, |morph| allocation.page.max(morph.page));
    while arena_buffers.len() <= highest_page as usize {
        let page = arena_buffers.len() as u16;
        let Some(capacity) = arena.page_capacity(page) else {
            let _ = arena.free(allocation);
            if let Some(morph_allocation) = morph_allocation {
                let _ = arena.free(morph_allocation);
            }
            return None;
        };
        arena_buffers.push(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(buffer_label),
            size: u64::from(capacity),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
    }
    let Some(buffer) = arena_buffers.get(allocation.page as usize) else {
        let _ = arena.free(allocation);
        if let Some(morph_allocation) = morph_allocation {
            let _ = arena.free(morph_allocation);
        }
        return None;
    };
    queue.write_buffer(buffer, u64::from(allocation.offset), bytes);
    if let (Some(morph_bytes), Some(morph_allocation)) = (morph_bytes, morph_allocation) {
        let Some(morph_buffer) = arena_buffers.get(morph_allocation.page as usize) else {
            let _ = arena.free(allocation);
            let _ = arena.free(morph_allocation);
            return None;
        };
        queue.write_buffer(
            morph_buffer,
            u64::from(morph_allocation.offset),
            morph_bytes,
        );
    }
    let content_fingerprint = morph_bytes.map_or_else(
        || fingerprint_bytes(bytes),
        |morph_bytes| fingerprint_value(fingerprint_bytes(bytes), fingerprint_bytes(morph_bytes)),
    );
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

fn gpu_quads_match_resident(
    mesh: Option<&ChunkMesh>,
    quads: &[GpuQuad],
    morph_heights: Option<&[u32]>,
) -> bool {
    let quad_bytes = bytemuck::cast_slice(quads);
    let content_fingerprint = morph_heights.map_or_else(
        || fingerprint_bytes(quad_bytes),
        |heights| {
            fingerprint_value(
                fingerprint_bytes(quad_bytes),
                fingerprint_bytes(bytemuck::cast_slice(heights)),
            )
        },
    );
    gpu_quad_content_matches(
        mesh.map(|mesh| (mesh.quad_count, mesh.content_fingerprint)),
        quads.len() as u32,
        content_fingerprint,
    )
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
        if let Some(morph_allocation) = prepared.morph_allocation {
            let _ = arena.free(morph_allocation);
        }
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
        if let Some(morph_allocation) = old.morph_allocation {
            let _ = arena.free(morph_allocation);
        }
    }
}

fn surface_macro_normals(tile: &SurfaceTileMesh) -> Vec<u32> {
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
                    x / 2 + 1,
                    z / 2 + 1,
                    stride * 2,
                )
            };
            let value = pack_surface_macro_normals(normal, parent_normal);
            packed[quad_index] = value;
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
            for (quad, packed_normal) in tile.quads[start..end]
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
                if quad_top == i64::from(height) + 1
                    && let Some(normal) = cell_normals[cell]
                {
                    *packed_normal = normal;
                }
            }
        }
    }
    packed
}

fn pack_surface_morph_heights(bottom_delta: i32, top_delta: i32) -> u32 {
    let (Ok(bottom), Ok(top)) = (i16::try_from(bottom_delta), i16::try_from(top_delta)) else {
        // A pathological height discontinuity must remain exact rather than wrap into unrelated
        // geometry. Normal generated terrain is several orders of magnitude inside this range.
        return 0;
    };
    u32::from(bottom as u16) | (u32::from(top as u16) << 16)
}

fn surface_parent_height(tile: &SurfaceTileMesh, x: i32, z: i32) -> Option<i32> {
    if tile.shading.parent_heights.is_empty() {
        return None;
    }
    let [origin_x, origin_z] = tile.coord.voxel_origin();
    let parent_stride = tile.coord.stride_voxels().checked_mul(2)?;
    let sample_x = (i64::from(x) - i64::from(origin_x)).div_euclid(i64::from(parent_stride)) + 1;
    let sample_z = (i64::from(z) - i64::from(origin_z)).div_euclid(i64::from(parent_stride)) + 1;
    let edge = voxels_world::SURFACE_PARENT_SHADING_EDGE_SAMPLES as i64;
    if !(0..edge).contains(&sample_x) || !(0..edge).contains(&sample_z) {
        return None;
    }
    tile.shading
        .parent_heights
        .get((sample_x + sample_z * edge) as usize)
        .copied()
}

/// Resolves the exact parent-height endpoints for generated terrain body faces. Top faces move as
/// a unit; vertical faces move their lower and upper edges independently, so adjacent cells retain
/// a closed shell throughout the transition. Skyline proxies and outermost tiles intentionally do
/// not morph.
fn surface_geometry_morphs(tile: &SurfaceTileMesh, macro_normals: &[u32]) -> Vec<u32> {
    let stride = tile.coord.stride_voxels();
    tile.quads
        .iter()
        .zip(macro_normals)
        .map(|(quad, &macro_normal)| {
            if macro_normal & SURFACE_MACRO_NORMAL_FLAG == 0 {
                return 0;
            }
            if quad.face == 2 {
                let Some(parent_height) =
                    surface_parent_height(tile, quad.origin[0], quad.origin[2])
                else {
                    return 0;
                };
                let delta = parent_height.saturating_sub(quad.origin[1]);
                return pack_surface_morph_heights(delta, delta);
            }
            if !matches!(quad.face, 0 | 1 | 4 | 5) {
                return 0;
            }
            let own_x = quad.origin[0] - if quad.face == 0 { stride - 1 } else { 0 };
            let own_z = quad.origin[2] - if quad.face == 4 { stride - 1 } else { 0 };
            let (neighbor_x, neighbor_z) = match quad.face {
                0 => (own_x.saturating_add(stride), own_z),
                1 => (own_x.saturating_sub(stride), own_z),
                4 => (own_x, own_z.saturating_add(stride)),
                _ => (own_x, own_z.saturating_sub(stride)),
            };
            let (Some(parent_own), Some(parent_neighbor)) = (
                surface_parent_height(tile, own_x, own_z),
                surface_parent_height(tile, neighbor_x, neighbor_z),
            ) else {
                return 0;
            };
            let child_neighbor = quad.origin[1].saturating_sub(1);
            let child_own = quad.origin[1]
                .saturating_add(i32::from(quad.extent[1]))
                .saturating_sub(1);
            pack_surface_morph_heights(
                parent_neighbor.saturating_sub(child_neighbor),
                parent_own.saturating_sub(child_own),
            )
        })
        .collect()
}

fn surface_morph_closure_gpu_quads(
    tile: &SurfaceTileMesh,
    macro_normals: &[u32],
    horizon_profiles: &[u16],
) -> Vec<(GpuQuad, u32)> {
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
                    material_face: pack_surface_horizon_material(
                        pack_gpu_material_face(
                            u32::from(quad.material.id())
                                | FAR_MATERIAL_FLAG
                                | (u32::from(tile.coord.level.index()) << SURFACE_LOD_SHIFT),
                            quad.face,
                        ),
                        horizon_profile,
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

fn offset_range(range: &std::ops::Range<u32>, offset: u32) -> std::ops::Range<u32> {
    range.start.saturating_add(offset)..range.end.saturating_add(offset)
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
    // Six bits per horizontal component are ample for the deliberately band-limited terrain
    // slopes and free the high seven AO bits for the parent-aware horizon profile.
    let encode = |component: f32| ((component.clamp(-1.0, 1.0) * 0.5 + 0.5) * 63.0).round() as u32;
    encode(normal.x)
        | (encode(normal.z) << 6)
        | (encode(parent.x) << 12)
        | (encode(parent.z) << 18)
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
                if quad.face != 2 || quad.extent != [stride as u16; 2] {
                    continue;
                }
                let local_x = (quad.origin[0] - origin[0]).div_euclid(stride);
                let local_z = (quad.origin[2] - origin[1]).div_euclid(stride);
                if !(0..edge as i32).contains(&local_x) || !(0..edge as i32).contains(&local_z) {
                    continue;
                }
                cells[local_x as usize + local_z as usize * edge] = Some(SurfaceCell {
                    height: quad.origin[1],
                    material: quad.material,
                    macro_normal: macro_normals[index],
                    horizon_profile: horizon_profiles[index],
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
                        material,
                        macro_normal: 0xff,
                        horizon_profile: 0,
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
            build.exact_edges.insert((patch, edge.index() as u8));
            for (quad, morph_heights) in edge_quads {
                build.quads.push(quad);
                build.morph_heights.push(morph_heights);
            }
        } else {
            build.incomplete_edges = build.incomplete_edges.saturating_add(1);
        }
    }
    build
}

fn append_lod_transition(
    quads: &mut Vec<(GpuQuad, u32)>,
    selection: &SurfacePatchSelection,
    surface_profiles: &HashMap<SurfacePatchId, SurfacePatchProfile>,
    canonical_profiles: &CanonicalColumnProfiles,
    patch: SurfacePatchId,
    edge: SurfacePatchEdge,
    coarse: &SurfacePatchProfile,
) -> bool {
    let coarse_stride = coarse.stride;
    let fine_stride = coarse_stride / 2;
    let patch_span = patch.voxel_span();
    let fine_segments = voxels_world::SURFACE_PATCH_EDGE_CELLS * 2;
    for fine_segment in 0..fine_segments {
        let tangent = fine_segment * fine_stride;
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
        let (fine_cell, fine_parent_height) =
            if let Some(fine_patch) = selection.selected_patch_at(fine_point) {
                if fine_patch.level.next_coarser() != Some(patch.level) {
                    return false;
                }
                let fine_cell = surface_profiles
                    .get(&fine_patch)
                    .and_then(|profile| profile.sample_world(fine_x, fine_z));
                let fine_parent = fine_patch
                    .parent()
                    .and_then(|parent| surface_profiles.get(&parent))
                    .and_then(|profile| profile.sample_world(fine_x, fine_z));
                let (Some(fine_cell), Some(fine_parent)) = (fine_cell, fine_parent) else {
                    return false;
                };
                (fine_cell, Some(fine_parent.height))
            } else if patch.level == SurfaceLodLevel::Stride2 {
                let Some(fine_cell) = canonical_surface_sample(canonical_profiles, fine_x, fine_z)
                else {
                    return false;
                };
                (fine_cell, None)
            } else {
                return false;
            };
        if coarse_cell.height == fine_cell.height {
            let Some(fine_parent_height) = fine_parent_height else {
                continue;
            };
            if fine_parent_height == fine_cell.height {
                continue;
            }
            let Some(fine_level) = SurfaceLodLevel::from_stride_voxels(fine_stride) else {
                return false;
            };
            let (lower, upper, face, surface) = if coarse_cell.height > fine_parent_height {
                (
                    fine_parent_height,
                    coarse_cell.height,
                    outward_face,
                    coarse_cell,
                )
            } else {
                (
                    coarse_cell.height,
                    fine_parent_height,
                    inward_face,
                    fine_cell,
                )
            };
            let collapsed_plane = coarse_cell.height.saturating_add(1);
            let mut remaining = i64::from(upper) - i64::from(lower);
            let mut y = lower.saturating_add(1);
            while remaining > 0 {
                let vertical_extent = remaining.min(i64::from(u16::MAX)) as u16;
                let origin_voxels = match face {
                    0 => [boundary[0].saturating_sub(1), y, boundary[1]],
                    1 => [boundary[0], y, boundary[1]],
                    4 => [boundary[0], y, boundary[1].saturating_sub(1)],
                    5 => [boundary[0], y, boundary[1]],
                    _ => unreachable!(),
                };
                let top = y.saturating_add(i32::from(vertical_extent));
                quads.push((
                    GpuQuad {
                        origin: origin_voxels,
                        extent_voxels: [
                            fine_stride as u16 | MORPH_CLOSURE_EXTENT_FLAG,
                            vertical_extent,
                        ],
                        material_face: pack_surface_horizon_material(
                            pack_gpu_material_face(
                                u32::from(surface.material.id())
                                    | FAR_MATERIAL_FLAG
                                    | (u32::from(fine_level.index()) << SURFACE_LOD_SHIFT),
                                face,
                            ),
                            fine_cell.horizon_profile,
                        ),
                        ao: pack_surface_horizon_ao(
                            fine_cell.macro_normal,
                            fine_cell.horizon_profile,
                        ),
                    },
                    pack_surface_morph_heights(
                        collapsed_plane.saturating_sub(y),
                        collapsed_plane.saturating_sub(top),
                    ),
                ));
                remaining -= i64::from(vertical_extent);
                y = top;
            }
            continue;
        }
        let (lower, upper, face, surface) = if coarse_cell.height > fine_cell.height {
            (
                fine_cell.height,
                coarse_cell.height,
                outward_face,
                coarse_cell,
            )
        } else {
            (coarse_cell.height, fine_cell.height, inward_face, fine_cell)
        };
        let mut remaining = i64::from(upper) - i64::from(lower);
        let mut y = lower.saturating_add(1);
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
                0,
            )
        };
        while remaining > 0 {
            let vertical_extent = remaining.min(i64::from(u16::MAX)) as u16;
            let origin_voxels = match face {
                0 => [boundary[0].saturating_sub(1), y, boundary[1]],
                1 => [boundary[0], y, boundary[1]],
                4 => [boundary[0], y, boundary[1].saturating_sub(1)],
                5 => [boundary[0], y, boundary[1]],
                _ => unreachable!(),
            };
            quads.push((
                GpuQuad {
                    origin: origin_voxels,
                    extent_voxels: [fine_stride as u16, vertical_extent],
                    material_face: pack_surface_horizon_material(
                        pack_gpu_material_face(
                            u32::from(surface.material.id())
                                | FAR_MATERIAL_FLAG
                                | (u32::from(encoded_level.index()) << SURFACE_LOD_SHIFT),
                            face,
                        ),
                        transition_horizon,
                    ),
                    // Between surface levels the connector follows the fine level's parent blend
                    // and collapses exactly as that shell reaches the coarse height. The canonical
                    // seam remains exact and static because canonical geometry has no sidecar.
                    ao: pack_surface_horizon_ao(transition_normal, transition_horizon),
                },
                morph_heights,
            ));
            remaining -= i64::from(vertical_extent);
            y = y.saturating_add(i32::from(vertical_extent));
        }
    }
    true
}

fn gpu_quad_bounds(quads: &[GpuQuad]) -> Option<(glam::Vec3, glam::Vec3)> {
    let mut minimum = glam::Vec3::splat(f32::INFINITY);
    let mut maximum = glam::Vec3::splat(f32::NEG_INFINITY);
    for quad in quads {
        let face = (quad.material_face & GPU_FACE_MASK) >> GPU_FACE_SHIFT;
        let extent = glam::Vec2::new(
            f32::from(quad.extent_voxels[0]) * VOXEL_SIZE_METRES,
            f32::from(quad.extent_voxels[1]) * VOXEL_SIZE_METRES,
        );
        let size = match face {
            0 | 1 => glam::Vec3::new(VOXEL_SIZE_METRES, extent.y, extent.x),
            2 | 3 => glam::Vec3::new(extent.x, VOXEL_SIZE_METRES, extent.y),
            _ => glam::Vec3::new(extent.x, extent.y, VOXEL_SIZE_METRES),
        };
        let origin =
            glam::Vec3::from_array(quad.origin.map(|value| value as f32 * VOXEL_SIZE_METRES));
        minimum = minimum.min(origin);
        maximum = maximum.max(origin + size);
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
    patch.level == tile.level
        && patch.x.div_euclid(SURFACE_PATCHES_PER_TILE_EDGE) == tile.x
        && patch.z.div_euclid(SURFACE_PATCHES_PER_TILE_EDGE) == tile.z
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
    let Some(focus) = focus else {
        return key.0 == 0;
    };
    let Some(plan) = lod_draw_plan else {
        return false;
    };
    if LOD_TRANSITION_MESH_KEYS.contains(key) {
        return plan.transition_mesh_key == Some(*key);
    }
    if key.0 == 0 {
        return plan.owns_enclosed_view_chunk(key) || plan.owns_canonical_chunk(key);
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
    if slice.morph_closure && !surface_patch_intersects_morph_band(focus, patch_id) {
        return false;
    }
    slice.boundary_edge.map_or_else(
        || plan.owns_patch(patch_id),
        |edge| plan.owns_source_edge(patch_id, edge),
    )
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
    // Matches the shader's max(1.6m, half_extent * 0.02) band in canonical 10cm voxels.
    let width = 16_i64.max((i64::from(half_extent) + 49) / 50);
    maximum_axis_delta >= i64::from(half_extent) - width
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
                    .checked_mul(size_of::<u32>() as u32)
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
        environment,
        world_environment,
        celestial_observation,
        underwater_blend,
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
        lod_options: [0.0, 0.0, 0.0, if lod_focus.is_some() { 1.0 } else { 0.0 }],
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
            underwater_blend.clamp(0.0, 1.0),
            fluid.eye_depth_metres.max(0.0),
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
        width as u64 * height as u64 * 8
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
    let projection = glam::camera::rh::proj::directx::perspective(
        68.0f32.to_radians(),
        aspect,
        0.05,
        view_distance_metres,
    );
    let view =
        glam::camera::rh::view::look_to_mat4(camera.position, camera.forward(), glam::Vec3::Y);
    projection * view
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
                "vs_main_morph"
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
                    wgpu::CompareFunction::LessEqual
                } else {
                    wgpu::CompareFunction::Less
                }),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            fragment_constants: &constants,
        },
    )
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
            compilation_options: Default::default(),
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
            depth_compare: Some(wgpu::CompareFunction::Less),
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
            depth_compare: Some(wgpu::CompareFunction::Less),
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

fn morph_height_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRIBUTES: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![4 => Uint32];
    wgpu::VertexBufferLayout {
        array_stride: size_of::<u32>() as wgpu::BufferAddress,
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

fn depth_view(device: &Device, width: u32, height: u32) -> TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("world depth"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
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

#[cfg(test)]
mod tests {
    use super::*;

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

    fn flat_patch_profile(patch: SurfacePatchId, height: i32) -> SurfacePatchProfile {
        SurfacePatchProfile {
            origin: patch.voxel_bounds_xz().unwrap()[0],
            stride: patch.level.stride_voxels(),
            cells: vec![
                Some(SurfaceCell {
                    height,
                    material: Material::Grass,
                    macro_normal: pack_surface_macro_normals(glam::Vec3::Y, glam::Vec3::Y),
                    horizon_profile: 0,
                });
                (voxels_world::SURFACE_PATCH_EDGE_CELLS.pow(2)) as usize
            ],
        }
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
        let normal_x = (value & 63) as f32 * (2.0 / 63.0) - 1.0;
        let normal_z = ((value >> 6) & 63) as f32 * (2.0 / 63.0) - 1.0;
        assert!(
            (-0.23..-0.18).contains(&normal_x),
            "uphill +X must retain a gentle, stable tilt toward -X: {normal_x}"
        );
        assert!(normal_z.abs() < 0.02);
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
    fn adjacent_surface_cells_morph_to_the_exact_same_parent_height() {
        let coord = SurfaceTileCoord::new(SurfaceLodLevel::Stride2, 0, 0);
        let tile =
            voxels_world::generate_surface_tile_mesh_with(coord, |x, _| (x, Material::Grass));
        let macro_normals = surface_macro_normals(&tile);
        let morphs = surface_geometry_morphs(&tile, &macro_normals);
        let resolved_height = |origin: [i32; 3]| {
            let index = tile
                .quads
                .iter()
                .position(|quad| quad.origin == origin && quad.face == 2)
                .expect("terrain top exists");
            let packed = morphs[index];
            let bits = (packed & 0xffff) as u16;
            tile.quads[index].origin[1] + i32::from(bits as i16)
        };
        assert_eq!(resolved_height([0, 1, 0]), 2);
        assert_eq!(resolved_height([2, 3, 0]), 2);
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
            let bottom_delta = (morph_heights as u16) as i16;
            let top_delta = ((morph_heights >> 16) as u16) as i16;
            assert_eq!(bottom_delta, 0);
            assert_eq!(top_delta, -2);
            assert_eq!(quad.origin[1] + i32::from(bottom_delta), 11);
            assert_eq!(
                quad.origin[1] + i32::from(quad.extent_voxels[1]) + i32::from(top_delta),
                11
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
        let outermost = SurfacePatchId::new(SurfaceLodLevel::Stride256, 0, 0);
        assert!(!surface_patch_intersects_morph_band(focus, inner));
        assert!(surface_patch_intersects_morph_band(focus, boundary));
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
        assert_eq!(transitions.quads.len(), 16);
        for (quad, &morph_heights) in transitions.quads.iter().zip(&transitions.morph_heights) {
            assert_eq!(quad.extent_voxels, [2, 10]);
            assert_eq!(quad.origin[0], 255);
            assert_eq!(quad.origin[1], 11);
            assert_eq!(quad.material_face >> GPU_FACE_SHIFT & 7, 0);
            assert_ne!(quad.ao & SURFACE_MACRO_NORMAL_FLAG, 0);
            assert_eq!(quad.origin[1] + i32::from(quad.extent_voxels[1]), 21,);
            assert_eq!((morph_heights as u16) as i16, 0);
            assert_eq!(((morph_heights >> 16) as u16) as i16, -8);
            assert_eq!(
                quad.origin[1]
                    + i32::from(quad.extent_voxels[1])
                    + i32::from(((morph_heights >> 16) as u16) as i16),
                13,
                "the fine endpoint must meet its own hidden parent at height 12"
            );
        }

        let main = MeshSlice {
            relative_offset: 0,
            size: size_of::<GpuQuad>() as u32,
            quad_count: 1,
            bounds_min: glam::Vec3::ZERO,
            bounds_max: glam::Vec3::ONE,
            surface_patch_id: Some(coarse),
            boundary_edge: None,
            morph_closure: false,
            render_layer: RenderLayer::Opaque,
        };
        let edge = MeshSlice {
            boundary_edge: Some(SurfacePatchEdge::NegativeX),
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
        assert_eq!(transitions.quads.len(), 16);
        for (quad, &morph_heights) in transitions.quads.iter().zip(&transitions.morph_heights) {
            assert_eq!(quad.extent_voxels, [2 | MORPH_CLOSURE_EXTENT_FLAG, 2]);
            assert_eq!(quad.origin[1], 11);
            assert_eq!((morph_heights as u16) as i16, 0);
            assert_eq!(((morph_heights >> 16) as u16) as i16, -2);
        }
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
        assert_eq!(transitions.quads.len(), 16 * 3);
        for segments in transitions.quads.chunks_exact(3) {
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
            incomplete_plan.owns_source_edge(coarse, edge),
            "a source edge remains authoritative until its whole replacement is available"
        );

        let mut canonical_cells = vec![None; CHUNK_EDGE * CHUNK_EDGE];
        for local_x in 16..32 {
            canonical_cells[local_x + 31 * CHUNK_EDGE] = Some(SurfaceCell {
                height: 20,
                material: Material::Grass,
                macro_normal: 0xff,
                horizon_profile: 0,
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
        assert_eq!(complete.quads.len(), 16);
        let complete_plan = LodDrawPlan {
            patches: complete_selection,
            canonical_columns: HashSet::from([(131, 191)]),
            canonical_chunks: HashSet::new(),
            enclosed_view_chunks: HashSet::new(),
            exact_transition_edges: complete.exact_edges,
            incomplete_transition_edges: complete.incomplete_edges,
            transition_mesh_key: None,
        };
        assert!(!complete_plan.owns_source_edge(coarse, edge));
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
    fn underground_profile_edits_do_not_change_the_resolved_transition_surface() {
        let cell = |height| {
            Some(SurfaceCell {
                height,
                material: Material::Stone,
                macro_normal: 0,
                horizon_profile: 0,
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
                    let child_parent = (child_normals[child_quad] >> 12) & 0x0fff;
                    let parent_own = parent_normals[parent_quad] & 0x0fff;
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
        let fingerprint = fingerprint_bytes(bytemuck::cast_slice(&quads));
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
        let changed_fingerprint = fingerprint_bytes(bytemuck::bytes_of(&changed));
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
            material: Material::Stone,
            macro_normal: 0,
            horizon_profile: 0,
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
        let resident = arena.allocate(32).expect("resident allocation");
        let resident_morph = arena.allocate(8).expect("resident morph allocation");
        let prepared = arena.allocate(64).expect("prepared allocation");
        let prepared_morph = arena.allocate(8).expect("prepared morph allocation");
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
        assert_eq!(
            arena.stats().allocated_bytes,
            u64::from(resident.size + resident_morph.size)
        );
        assert!(!arena.free(prepared));
        assert!(!arena.free(prepared_morph));
        let resident_mesh = chunks.remove(&key).expect("resident mesh");
        assert!(arena.free(resident_mesh.allocation));
        assert!(arena.free(resident_mesh.morph_allocation.expect("resident morph")));
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
            morph_closure: false,
            render_layer: RenderLayer::Opaque,
        }
    }

    fn test_view_projection(camera: &CameraState) -> glam::Mat4 {
        glam::camera::rh::proj::directx::perspective(68.0f32.to_radians(), 1.0, 0.01, 80.0)
            * glam::camera::rh::view::look_to_mat4(camera.position, camera.forward(), glam::Vec3::Y)
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
        assert_eq!(refraction_copy_bytes(1_280, 720, true), 7_372_800);
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
            17_300_000, 18_300_000,
        ];
        let active = GpuPassMask {
            shadows: true,
            water: true,
            ambient_occlusion: true,
            clouds: true,
            weather: true,
        };
        let timing = parse_gpu_timestamps(&timestamps, 1.0, active)
            .unwrap_or_else(|| panic!("valid timestamps should parse"));
        assert!((timing.total_ms - 17.3).abs() < f32::EPSILON);
        assert!((timing.shadow_ms - 3.0).abs() < f32::EPSILON);
        assert!((timing.depth_prepass_ms - 2.0).abs() < f32::EPSILON);
        assert!((timing.ambient_occlusion_ms - 1.5).abs() < f32::EPSILON);
        assert!((timing.world_ms - 2.0).abs() < f32::EPSILON);
        assert!((timing.water_ms - 3.0).abs() < f32::EPSILON);
        assert!((timing.cloud_ms - 2.4).abs() < f32::EPSILON);
        assert!((timing.weather_ms - 0.3).abs() < f32::EPSILON);
        assert!((timing.ui_ms - 1.0).abs() < f32::EPSILON);

        let mut skipped = timestamps;
        skipped[0..6].copy_from_slice(&[90, 80, 70, 60, 50, 40]);
        skipped[6..12].copy_from_slice(&[39, 38, 37, 36, 35, 34]);
        skipped[12..14].copy_from_slice(&[33, 32]);
        skipped[16..22].copy_from_slice(&[31, 30, 29, 28, 27, 26]);
        let timing = parse_gpu_timestamps(&skipped, 1.0, GpuPassMask::default())
            .unwrap_or_else(|| panic!("inactive timestamp pairs should be ignored"));
        assert_eq!(timing.shadow_ms, 0.0);
        assert_eq!(timing.depth_prepass_ms, 0.0);
        assert_eq!(timing.ambient_occlusion_ms, 0.0);
        assert_eq!(timing.water_ms, 0.0);
        assert_eq!(timing.cloud_ms, 0.0);
        assert_eq!(timing.weather_ms, 0.0);
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
        assert_eq!(GPU_QUERY_BUFFER_BYTES, 192);
        assert_eq!(GPU_RESOLVE_BUFFER_BYTES % 256, 0);
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
        let contiguous = coalesce_draw_items(vec![item(0, 0), item(24, 4)]);
        assert_eq!(contiguous.len(), 1);
        assert_eq!(contiguous[0].quad_count, 2);

        let split_sidecar = coalesce_draw_items(vec![item(0, 0), item(24, 8)]);
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
            morph_closure: false,
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
            morph_closure: false,
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
            true,
            true,
            focus,
            view_clip,
            shadow_clips,
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
            true,
            true,
            focus,
            view_clip,
            shadow_clips,
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
            true,
            true,
            moved_focus,
            view_clip,
            shadow_clips,
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
}
