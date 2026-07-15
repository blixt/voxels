use crate::ambient_occlusion::AmbientOcclusionGpu;
use crate::arena::{Allocation, ArenaAllocator};
use crate::avatar::AvatarGpu;
use crate::environment::{
    DaylightPhase, InteriorEnvironment, OutdoorEnvironment, surface_region_label,
};
use crate::lod::{GeometricLodFocus, SurfacePatchSelection};
use crate::material_detail::MaterialDetailGpu;
use crate::shadow::{
    AabbClipVolume, CASCADE_COUNT, DirectionalShadowCascades, DirectionalShadowConfig,
    build_directional_shadow_cascades,
};
use crate::ui::{
    Color, ContextAction, InventoryItem, LiveStats, MissionControlUi, RendererFeature, UiAction,
    UiKey, Viewport,
};
pub use crate::ui::{MissionControlConfig, RendererFeatureConfig};
use crate::ui_gpu::{SCENE_FORMAT, UiGpu, texture_sampler_layout};
use bytemuck::{Pod, Zeroable};
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use voxels_core::{CameraState, EnclosureSample, RemoteAvatarPose};
use voxels_world::protocol::DigVolume;
use voxels_world::{
    AtmosphereSample, CHUNK_EDGE, ChunkCoord, Material, MeshedChunk, Quad, RenderLayer,
    SurfaceBounds, SurfaceLodLevel, SurfacePatchEdge, SurfacePatchId, SurfaceRegion,
    SurfaceTileCoord, SurfaceTileMesh, VOXEL_SIZE_METRES, WaterTileMesh,
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
const ARENA_PAGE_BYTES: u32 = 4 * 1024 * 1024;
const FAR_MATERIAL_FLAG: u32 = 1 << 31;
const SURFACE_LOD_SHIFT: u32 = 27;
const GPU_FACE_SHIFT: u32 = 16;
const GPU_FACE_MASK: u32 = 0b111 << GPU_FACE_SHIFT;
const SURFACE_MACRO_NORMAL_FLAG: u32 = 1 << 24;
// Decimated height samples are not band-limited. Keeping their full derivative makes a one-voxel
// clipmap snap turn unresolved relief into a false near-horizontal slope (and an almost black
// valley at low sun angles). A conservative macro cue remains legible while staying stable across
// adjacent LOD sampling phases.
const SURFACE_MACRO_SLOPE_SCALE: f32 = 0.25;
const SURFACE_MACRO_SLOPE_MAX: f32 = 0.5;
const FAR_UNDERLAY_OFFSET_METRES: f32 = 0.025;
const GPU_QUERY_COUNT: u32 = 18;
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
    pub initial_daylight_phase: DaylightPhase,
}

impl Default for RendererConfig {
    fn default() -> Self {
        Self {
            features: RendererFeatureConfig::default(),
            mission_control: MissionControlConfig::default(),
            view_distance_metres: 1_000.0,
            directional_shadows: DirectionalShadowConfig::default(),
            initial_daylight_phase: DaylightPhase::default(),
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
    camera_forward: [f32; 4],
    shadow_splits: [f32; 4],
    shadow_texel_sizes: [f32; 4],
    shadow_view_projection: [[[f32; 4]; 4]; CASCADE_COUNT],
    sun_direction: [f32; 4],
    sun_radiance: [f32; 4],
    sky_horizon: [f32; 4],
    sky_zenith: [f32; 4],
    ground_atmosphere: [f32; 4],
    fog_exposure: [f32; 4],
    medium: [f32; 4],
    interior: [f32; 4],
}

const _: () = assert!(size_of::<FrameUniform>() == 592);
const _: () = assert!(std::mem::offset_of!(FrameUniform, medium) == 560);
const _: () = assert!(std::mem::offset_of!(FrameUniform, interior) == 576);

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
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuQuad {
    origin: [f32; 3],
    extent_voxels: [u16; 2],
    material_face: u32,
    ao: u32,
}

const _: () = assert!(size_of::<GpuQuad>() == 24);
const _: () = assert!(std::mem::offset_of!(GpuQuad, extent_voxels) == 12);
const _: () = assert!(std::mem::offset_of!(GpuQuad, material_face) == 16);
const _: () = assert!(std::mem::offset_of!(GpuQuad, ao) == 20);

fn pack_gpu_material_face(material: u32, face: u8) -> u32 {
    debug_assert_eq!(material & GPU_FACE_MASK, 0);
    debug_assert!(face <= 5);
    material | (u32::from(face) << GPU_FACE_SHIFT)
}

struct ChunkMesh {
    allocation: Allocation,
    quad_count: u32,
    content_fingerprint: u64,
    slices: Vec<MeshSlice>,
    lod_ownership_focus: Option<GeometricLodFocus>,
    lod_residency_revision: u64,
    lod_owned_slices: Vec<bool>,
    bounds_min: glam::Vec3,
    bounds_max: glam::Vec3,
    activation_mask: u8,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChunkActivationReason {
    Radial = 1,
    Portal = 2,
    Interaction = 4,
}

impl ChunkMesh {
    fn refresh_lod_ownership(
        &mut self,
        key: &MeshKey,
        focus: Option<GeometricLodFocus>,
        surface_patch_selection: Option<&SurfacePatchSelection>,
        residency_revision: u64,
    ) {
        let Some(focus) = focus else {
            return;
        };
        if self.lod_ownership_focus == Some(focus)
            && self.lod_residency_revision == residency_revision
            && self.lod_owned_slices.len() == self.slices.len()
        {
            return;
        }
        self.lod_owned_slices = self
            .slices
            .iter()
            .map(|slice| slice_owned_by_lod(Some(focus), surface_patch_selection, key, slice))
            .collect();
        self.lod_ownership_focus = Some(focus);
        self.lod_residency_revision = residency_revision;
    }

    fn lod_owns_slice(&self, focus: Option<GeometricLodFocus>, slice_index: usize) -> bool {
        focus.is_none() || self.lod_owned_slices.get(slice_index) == Some(&true)
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
    surface_bounds: Option<SurfaceBounds>,
    surface_patch_id: Option<SurfacePatchId>,
    skirt_edge: Option<SurfacePatchEdge>,
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

#[derive(Debug)]
struct DrawListBuilder {
    items: Vec<DrawItem>,
    mesh_count: u32,
    quad_count: u32,
    fingerprint: u64,
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
            tested_slices: 0,
            selected_slices: 0,
        }
    }
}

impl DrawListBuilder {
    fn test_slice(&mut self) {
        self.tested_slices = self.tested_slices.saturating_add(1);
    }

    fn select_slice(&mut self, chunk: &ChunkMesh, slice: &MeshSlice) {
        self.selected_slices = self.selected_slices.saturating_add(1);
        self.items.push(DrawItem {
            page: chunk.allocation.page,
            offset: chunk.allocation.offset + slice.relative_offset,
            size: slice.size,
            quad_count: slice.quad_count,
        });
        self.quad_count = self.quad_count.saturating_add(slice.quad_count);
    }

    fn select_mesh(&mut self, key: MeshKey, chunk: &ChunkMesh) {
        self.mesh_count = self.mesh_count.saturating_add(1);
        self.fingerprint = fingerprint_value(self.fingerprint, u64::from(key.0));
        self.fingerprint = fingerprint_value(self.fingerprint, key.1 as u32 as u64);
        self.fingerprint = fingerprint_value(self.fingerprint, key.2 as u32 as u64);
        self.fingerprint = fingerprint_value(self.fingerprint, key.3 as u32 as u64);
        self.fingerprint = fingerprint_value(self.fingerprint, chunk.content_fingerprint);
    }

    fn finish(self) -> DrawList {
        DrawList {
            spans: coalesce_draw_items(self.items),
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
    pub gpu_ui_ms: Option<f32>,
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
    pub surface_region: u8,
    pub cloud_coverage: f32,
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
    shadows: bool,
    water: bool,
    ambient_occlusion: bool,
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
    shadows: bool,
    water: bool,
    ambient_occlusion: bool,
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
    let shadow_cascade_ms = if shadows {
        [elapsed_ms(0, 1)?, elapsed_ms(2, 3)?, elapsed_ms(4, 5)?]
    } else {
        [0.0; CASCADE_COUNT]
    };
    let shadow_ms = shadow_cascade_ms.into_iter().sum();
    let depth_prepass_ms = if ambient_occlusion {
        elapsed_ms(6, 7)?
    } else {
        0.0
    };
    let world_ms = elapsed_ms(12, 13)?;
    let water_ms = if water { elapsed_ms(14, 15)? } else { 0.0 };
    let ambient_occlusion_ms = if ambient_occlusion {
        elapsed_ms(8, 9)? + elapsed_ms(10, 11)?
    } else {
        0.0
    };
    let ui_ms = elapsed_ms(16, 17)?;
    let mut first = timestamps[12].min(timestamps[16]);
    let mut last = timestamps[13].max(timestamps[17]);
    if shadows {
        for (start, end) in [(0, 1), (2, 3), (4, 5)] {
            first = first.min(timestamps[start]);
            last = last.max(timestamps[end]);
        }
    }
    if water {
        first = first.min(timestamps[14]);
        last = last.max(timestamps[15]);
    }
    if ambient_occlusion {
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

    fn begin_frame(
        &mut self,
        frame_id: u32,
        shadows: bool,
        water: bool,
        ambient_occlusion: bool,
    ) -> Option<GpuTimingFrame> {
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
                    shadows,
                    water,
                    ambient_occlusion,
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
                    parsed = parse_gpu_timestamps(
                        &timestamps,
                        period,
                        frame.shadows,
                        frame.water,
                        frame.ambient_occlusion,
                    );
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
    depth_prepass_pipeline: RenderPipeline,
    depth_prepass_fast_pipeline: RenderPipeline,
    voxel_pipeline: RenderPipeline,
    voxel_flat_pipeline: RenderPipeline,
    voxel_ambient_occlusion_pipeline: RenderPipeline,
    voxel_ambient_occlusion_flat_pipeline: RenderPipeline,
    water_pipeline: RenderPipeline,
    avatar_gpu: AvatarGpu,
    remote_avatars: Vec<RemoteAvatarPose>,
    water_scene_layout: wgpu::BindGroupLayout,
    water_scene_bind_group: BindGroup,
    shadow_gpu: ShadowGpu,
    frame_buffer: Buffer,
    frame_bind_group: BindGroup,
    local_light_buffer: Buffer,
    material_detail: MaterialDetailGpu,
    chunks: BTreeMap<MeshKey, ChunkMesh>,
    water_chunks: BTreeMap<MeshKey, ChunkMesh>,
    surface_patch_residency: HashSet<SurfacePatchId>,
    canonical_ready_columns: HashSet<(i32, i32)>,
    surface_patch_residency_revision: u64,
    surface_patch_selection: SurfacePatchSelection,
    surface_patch_selection_focus: Option<GeometricLodFocus>,
    surface_patch_selection_revision: u64,
    chunk_activations: ChunkActivations,
    local_light_candidates: BTreeMap<MeshKey, Vec<GpuLocalLight>>,
    arena: ArenaAllocator,
    arena_buffers: Vec<Buffer>,
    water_arena: ArenaAllocator,
    water_arena_buffers: Vec<Buffer>,
    depth_view: TextureView,
    ambient_occlusion_gpu: AmbientOcclusionGpu,
    time: f32,
    diagnostics: RenderDiagnostics,
    gpu_timer: Option<GpuTimer>,
    target_voxel: Option<[i32; 3]>,
    target_volume: Option<DigVolume>,
    options: RenderOptions,
    environment: OutdoorEnvironment,
    environment_target: OutdoorEnvironment,
    atmosphere_sample: AtmosphereSample,
    surface_region: SurfaceRegion,
    daylight_phase: DaylightPhase,
    geometric_lod_focus: Option<GeometricLodFocus>,
    fine_coverage_ready: bool,
    lod_coverage_ready: bool,
    ui: MissionControlUi,
    ui_gpu: UiGpu,
    dpr: f32,
    log_error: fn(&str),
    ui_text_error_reported: bool,
    diagnostics_copy_requested: bool,
    coast_teleport_requested: bool,
    underwater_teleport_requested: bool,
    route_teleport_requested: bool,
    cave_teleport_requested: bool,
    landmark_teleport_requested: bool,
    underwater_blend: f32,
    interior: InteriorEnvironment,
    interior_target: InteriorEnvironment,
    placement_inventory: PlacementInventory,
    runtime_config: RendererConfig,
}

struct ShadowGpu {
    layout: wgpu::BindGroupLayout,
    _texture: Texture,
    sample_view: TextureView,
    sampler: wgpu::Sampler,
    layer_views: [TextureView; CASCADE_COUNT],
    uniform_buffers: [Buffer; CASCADE_COUNT],
    bind_groups: [BindGroup; CASCADE_COUNT],
    pipeline: RenderPipeline,
    settled_pipeline: RenderPipeline,
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

#[derive(Clone, Copy, Debug, Default)]
struct LodReadiness {
    fine: bool,
    all: bool,
    geometric: bool,
}

#[derive(Clone, Copy, Debug)]
struct FrameState {
    options: RenderOptions,
    readiness: LodReadiness,
    environment: OutdoorEnvironment,
    underwater_blend: f32,
    interior: InteriorEnvironment,
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

fn reset_renderer_features(
    ui: &mut MissionControlUi,
    baseline: RendererFeatureConfig,
) -> RenderOptions {
    for feature in RendererFeature::ALL {
        let _ = ui.set_feature(feature, baseline.enabled(feature));
    }
    baseline.into()
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
        environment: OutdoorEnvironment,
        config: DirectionalShadowConfig,
        options: RenderOptions,
    ) -> Result<Self, String> {
        let cascades =
            build_directional_shadow_cascades(camera, 1.0, -environment.sun_direction, config)
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
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let initial_uniforms: [ShadowFrameUniform; CASCADE_COUNT] = std::array::from_fn(|index| {
            shadow_frame_uniform(&cascades, index, camera, options, LodReadiness::default())
        });
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
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow caster pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(quad_layout())],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
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
        });
        // Once CPU geometric ownership is active and fine coverage is complete, the fragment
        // shader's LOD/discard predicate is identically true. Omitting that stage preserves the
        // exact depth result while allowing the GPU to use its fragmentless depth fast path.
        let settled_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("settled shadow caster pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(quad_layout())],
                compilation_options: Default::default(),
            },
            fragment: None,
            primitive: wgpu::PrimitiveState::default(),
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
        });
        Ok(Self {
            layout,
            _texture: texture,
            sample_view,
            sampler,
            layer_views,
            uniform_buffers,
            bind_groups,
            pipeline,
            settled_pipeline,
        })
    }

    fn write_cascades(
        &self,
        queue: &Queue,
        cascades: &DirectionalShadowCascades,
        camera: &CameraState,
        options: RenderOptions,
        readiness: LodReadiness,
    ) {
        for index in 0..CASCADE_COUNT {
            let uniform = shadow_frame_uniform(cascades, index, camera, options, readiness);
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
        let daylight_phase = runtime_config.initial_daylight_phase;
        let environment =
            OutdoorEnvironment::for_atmosphere(atmosphere_sample, daylight_phase).sanitized();
        let initial_camera = CameraState::default();
        let shadow_gpu = ShadowGpu::new(
            &device,
            &initial_camera,
            environment,
            runtime_config.directional_shadows,
            options,
        )?;
        let material_detail = MaterialDetailGpu::new(&device, &queue);
        let shadow_cascades = directional_shadow_cascades(
            &config,
            &initial_camera,
            environment.sun_direction,
            runtime_config.directional_shadows,
        )?;
        let frame = frame_uniform(
            &config,
            &initial_camera,
            0.0,
            None,
            FrameState {
                options,
                readiness: LodReadiness::default(),
                environment,
                underwater_blend: 0.0,
                interior: InteriorEnvironment::default(),
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
        let depth_view = depth_view(&device, config.width, config.height);
        let ambient_occlusion_gpu = AmbientOcclusionGpu::new(
            &device,
            &frame_layout,
            &depth_view,
            config.width,
            config.height,
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
        let water_scene_layout = texture_sampler_layout(&device, "water refraction scene layout");
        let water_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("water pipeline layout"),
                bind_group_layouts: &[Some(&frame_layout), Some(&water_scene_layout)],
                immediate_size: 0,
            });
        let sky_shader = device.create_shader_module(wgpu::include_wgsl!("shaders/sky.wgsl"));
        let sky_pipeline = pipeline(
            &device,
            "sky pipeline",
            &sky_pipeline_layout,
            &sky_shader,
            SCENE_FORMAT,
            &[],
            PipelineOptions {
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
        let voxel_shader = device.create_shader_module(wgpu::include_wgsl!("shaders/voxels.wgsl"));
        let depth_prepass_pipeline = depth_only_pipeline(
            &device,
            "spatial AO depth ownership pipeline",
            &sky_pipeline_layout,
            &voxel_shader,
        );
        let depth_prepass_fast_pipeline = fragmentless_depth_pipeline(
            &device,
            "settled spatial AO depth pipeline",
            &sky_pipeline_layout,
            &voxel_shader,
        );
        let voxel_pipeline = pipeline(
            &device,
            "voxel pipeline",
            &world_pipeline_layout,
            &voxel_shader,
            SCENE_FORMAT,
            &[Some(quad_layout())],
            PipelineOptions {
                fragment_entry: "fs_main",
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                fragment_constants: &[("MATERIAL_DETAIL", 1.0)],
            },
        );
        let voxel_flat_pipeline = pipeline(
            &device,
            "flat voxel pipeline",
            &world_pipeline_layout,
            &voxel_shader,
            SCENE_FORMAT,
            &[Some(quad_layout())],
            PipelineOptions {
                fragment_entry: "fs_main",
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                fragment_constants: &[("MATERIAL_DETAIL", 0.0)],
            },
        );
        let voxel_ambient_occlusion_pipeline = pipeline(
            &device,
            "spatial AO voxel pipeline",
            &world_pipeline_layout,
            &voxel_shader,
            SCENE_FORMAT,
            &[Some(quad_layout())],
            PipelineOptions {
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
                fragment_constants: &[("MATERIAL_DETAIL", 1.0)],
            },
        );
        let voxel_ambient_occlusion_flat_pipeline = pipeline(
            &device,
            "flat spatial AO voxel pipeline",
            &world_pipeline_layout,
            &voxel_shader,
            SCENE_FORMAT,
            &[Some(quad_layout())],
            PipelineOptions {
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
                fragment_constants: &[("MATERIAL_DETAIL", 0.0)],
            },
        );
        let water_pipeline = pipeline(
            &device,
            "water pipeline",
            &water_pipeline_layout,
            &voxel_shader,
            SCENE_FORMAT,
            &[Some(quad_layout())],
            PipelineOptions {
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
        let mut ui = MissionControlUi::new(runtime_config.mission_control, runtime_config.features);
        ui.set_environment_status(daylight_phase.label(), surface_region_label(surface_region));
        sync_inventory_ui(&mut ui, &placement_inventory);
        Ok(Self {
            surface,
            device,
            queue,
            config,
            sky_pipeline,
            depth_prepass_pipeline,
            depth_prepass_fast_pipeline,
            voxel_pipeline,
            voxel_flat_pipeline,
            voxel_ambient_occlusion_pipeline,
            voxel_ambient_occlusion_flat_pipeline,
            water_pipeline,
            avatar_gpu,
            remote_avatars: Vec::new(),
            water_scene_layout,
            water_scene_bind_group,
            shadow_gpu,
            frame_buffer,
            frame_bind_group,
            local_light_buffer,
            material_detail,
            chunks: BTreeMap::new(),
            water_chunks: BTreeMap::new(),
            surface_patch_residency: HashSet::new(),
            canonical_ready_columns: HashSet::new(),
            surface_patch_residency_revision: 0,
            surface_patch_selection: SurfacePatchSelection::default(),
            surface_patch_selection_focus: None,
            surface_patch_selection_revision: u64::MAX,
            chunk_activations: ChunkActivations::default(),
            local_light_candidates: BTreeMap::new(),
            arena: ArenaAllocator::new(ARENA_PAGE_BYTES, size_of::<GpuQuad>() as u32),
            arena_buffers: Vec::new(),
            water_arena: ArenaAllocator::new(ARENA_PAGE_BYTES, size_of::<GpuQuad>() as u32),
            water_arena_buffers: Vec::new(),
            depth_view,
            ambient_occlusion_gpu,
            time: 0.0,
            diagnostics: RenderDiagnostics::default(),
            gpu_timer,
            target_voxel: None,
            target_volume: None,
            options,
            environment,
            environment_target: environment,
            atmosphere_sample,
            surface_region,
            daylight_phase,
            geometric_lod_focus: None,
            fine_coverage_ready: false,
            lod_coverage_ready: false,
            ui,
            ui_gpu,
            dpr: valid_dpr(dpr),
            log_error,
            ui_text_error_reported: false,
            diagnostics_copy_requested: false,
            coast_teleport_requested: false,
            underwater_teleport_requested: false,
            route_teleport_requested: false,
            cave_teleport_requested: false,
            landmark_teleport_requested: false,
            underwater_blend: 0.0,
            interior: InteriorEnvironment::default(),
            interior_target: InteriorEnvironment::default(),
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

    pub fn set_dig_target(&mut self, target: Option<([i32; 3], DigVolume)>) {
        self.target_voxel = target.map(|(hit, _)| hit);
        self.target_volume = target.map(|(_, volume)| volume);
    }

    pub fn set_atmosphere(&mut self, sample: AtmosphereSample, region: SurfaceRegion) {
        if self.atmosphere_sample == sample && self.surface_region == region {
            return;
        }
        self.atmosphere_sample = sample;
        self.surface_region = region;
        self.environment_target = OutdoorEnvironment::for_atmosphere(sample, self.daylight_phase);
        self.ui
            .set_environment_status(self.daylight_phase.label(), surface_region_label(region));
    }

    pub fn set_route_status(&mut self, chapter_label: &'static str, progress_percent: u8) {
        self.ui.set_route_status(chapter_label, progress_percent);
    }

    pub fn set_enclosure(&mut self, sample: EnclosureSample) {
        self.interior_target = InteriorEnvironment::for_enclosure(sample);
    }

    pub const fn daylight_phase(&self) -> DaylightPhase {
        self.daylight_phase
    }

    pub fn set_geometric_lod_focus(
        &mut self,
        voxel_x: i32,
        voxel_z: i32,
        surface_level_count: usize,
    ) {
        self.geometric_lod_focus = Some(GeometricLodFocus::snapped_for_levels(
            voxel_x,
            voxel_z,
            surface_level_count,
        ));
    }

    pub fn advance_geometric_lod_focus(
        &mut self,
        voxel_x: i32,
        voxel_z: i32,
        ready_level_count: usize,
        surface_level_count: usize,
    ) {
        self.geometric_lod_focus = Some(self.geometric_lod_focus.map_or_else(
            || GeometricLodFocus::snapped_for_levels(voxel_x, voxel_z, surface_level_count),
            |focus| {
                focus.advanced_for_levels(voxel_x, voxel_z, ready_level_count, surface_level_count)
            },
        ));
    }

    pub const fn set_lod_coverage_ready(&mut self, fine_ready: bool, all_lods_ready: bool) {
        self.fine_coverage_ready = fine_ready;
        self.lod_coverage_ready = all_lods_ready;
    }

    const fn lod_readiness(&self) -> LodReadiness {
        LodReadiness {
            fine: self.fine_coverage_ready,
            all: self.lod_coverage_ready,
            geometric: self.geometric_lod_focus.is_some(),
        }
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

    /// Replaces the canonical X/Z columns whose complete vertical chunk set is resident and
    /// active. Incomplete columns keep their stride-two surface parent until the shell atomically
    /// marks the column ready.
    pub fn set_canonical_ready_columns(&mut self, columns: impl IntoIterator<Item = (i32, i32)>) {
        let replacement = columns.into_iter().collect::<HashSet<_>>();
        if replacement == self.canonical_ready_columns {
            return;
        }
        self.canonical_ready_columns = replacement;
        self.surface_patch_residency_revision =
            self.surface_patch_residency_revision.wrapping_add(1);
    }

    pub const fn ui_open(&self) -> bool {
        self.ui.open()
    }

    pub const fn target_voxel(&self) -> Option<[i32; 3]> {
        self.target_voxel
    }

    pub fn take_coast_teleport_request(&mut self) -> bool {
        std::mem::take(&mut self.coast_teleport_requested)
    }

    pub fn take_underwater_teleport_request(&mut self) -> bool {
        std::mem::take(&mut self.underwater_teleport_requested)
    }

    pub fn take_route_teleport_request(&mut self) -> bool {
        std::mem::take(&mut self.route_teleport_requested)
    }

    pub fn take_cave_teleport_request(&mut self) -> bool {
        std::mem::take(&mut self.cave_teleport_requested)
    }

    pub fn take_landmark_teleport_request(&mut self) -> bool {
        std::mem::take(&mut self.landmark_teleport_requested)
    }

    pub const fn placement_material(&self) -> Option<Material> {
        self.placement_inventory.selected()
    }

    pub fn inventory_counts(&self) -> [u64; Material::ALL.len()] {
        self.placement_inventory.counts
    }

    pub fn inventory_count(&self, material: Material) -> u64 {
        self.placement_inventory.count(material)
    }

    /// Replaces the complete server-authored inventory snapshot. Selection follows the first
    /// stocked material only when the current material has become unavailable.
    pub fn set_inventory_counts(&mut self, counts: [u64; Material::ALL.len()]) {
        self.placement_inventory.set_counts(counts);
        sync_inventory_ui(&mut self.ui, &self.placement_inventory);
    }

    /// Selects a material only when the latest authoritative inventory says it is available.
    pub fn set_placement_material(&mut self, material: Material) -> bool {
        let selected = self.placement_inventory.select(material);
        if selected {
            sync_inventory_ui(&mut self.ui, &self.placement_inventory);
        }
        selected
    }

    /// Cycles in either direction, skipping every material whose authoritative count is zero.
    pub fn cycle_placement_material(&mut self, direction: i32) -> bool {
        let changed = self.placement_inventory.cycle(direction);
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
            "MISSION CONTROL COPIED"
        } else {
            "COULD NOT COPY MISSION CONTROL"
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

    pub fn handle_ui_pointer_down(&mut self, css_x: f32, css_y: f32, secondary: bool) -> bool {
        let viewport = self.ui_viewport();
        let point = [css_x * self.dpr, css_y * self.dpr];
        let action = if secondary && self.ui.open() {
            self.ui.open_context_menu_device(point, viewport)
        } else {
            self.ui.activate_device(point, viewport)
        };
        self.apply_ui_action(action);
        self.ui.open()
    }

    pub fn inventory_wheel_contains(&self, css_x: f32, css_y: f32) -> bool {
        self.ui
            .inventory_contains_css([css_x, css_y], self.ui_viewport())
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
            UiAction::CopyDiagnostics => {
                self.diagnostics_copy_requested = true;
            }
            UiAction::FeatureChanged(feature, enabled) => match feature {
                RendererFeature::CascadedSunShadows => self.options.shadows = enabled,
                RendererFeature::VoxelAmbientOcclusion => {
                    self.options.ambient_occlusion = enabled;
                }
                RendererFeature::ScreenSpaceAmbientOcclusion => {
                    self.options.screen_space_ambient_occlusion = enabled;
                }
                RendererFeature::AtmosphericFog => self.options.fog = enabled,
                RendererFeature::FarTerrain => self.options.far_terrain = enabled,
                RendererFeature::WaterSurface => self.options.water = enabled,
                RendererFeature::TargetOutline => self.options.target_outline = enabled,
                RendererFeature::MaterialSurfaceDetail => self.options.material_detail = enabled,
                RendererFeature::CaveHeadlamp => self.options.cave_headlamp = enabled,
                RendererFeature::VoxelEmissiveLights => self.options.local_lighting = enabled,
            },
            UiAction::ContextAction(ContextAction::ResetRendererFeatures) => {
                self.options = reset_renderer_features(&mut self.ui, self.runtime_config.features);
            }
            UiAction::ContextAction(ContextAction::TeleportToCoast) => {
                self.coast_teleport_requested = true;
            }
            UiAction::ContextAction(ContextAction::DiveBelowSurface) => {
                self.underwater_teleport_requested = true;
            }
            UiAction::ContextAction(ContextAction::ToggleCompactTelemetry) => {
                let _ = self.ui.set_compact(!self.ui.compact());
            }
            UiAction::ContextAction(ContextAction::CycleDaylight) => {
                self.daylight_phase = self.daylight_phase.next();
                self.environment_target =
                    OutdoorEnvironment::for_atmosphere(self.atmosphere_sample, self.daylight_phase);
                self.ui.set_environment_status(
                    self.daylight_phase.label(),
                    surface_region_label(self.surface_region),
                );
            }
            UiAction::ContextAction(ContextAction::FollowPilgrimRoad) => {
                self.route_teleport_requested = true;
            }
            UiAction::ContextAction(ContextAction::VisitCinderVault) => {
                self.cave_teleport_requested = true;
            }
            UiAction::ContextAction(ContextAction::VisitLandmark) => {
                self.landmark_teleport_requested = true;
            }
            UiAction::ContextAction(ContextAction::CyclePlacementMaterial) => {
                let _ = self.cycle_placement_material(1);
            }
            UiAction::ContextAction(ContextAction::CloseMissionControl) => {
                let _ = self.ui.set_open(false);
            }
            UiAction::None
            | UiAction::PanelOpenChanged(_)
            | UiAction::CompactChanged(_)
            | UiAction::ContextMenuOpened
            | UiAction::ContextMenuClosed => {}
        }
    }

    pub fn upload_chunk(&mut self, coord: ChunkCoord, mesh: &MeshedChunk) -> bool {
        let key = (0, coord.x, coord.y, coord.z);
        if mesh.is_empty() {
            self.remove_chunk_mesh(key);
            return true;
        }
        let origin = coord.world_origin();
        let convert = |quad: &Quad| GpuQuad {
            origin: [
                (origin[0] + i32::from(quad.origin[0])) as f32 * VOXEL_SIZE_METRES,
                (origin[1] + i32::from(quad.origin[1])) as f32 * VOXEL_SIZE_METRES,
                (origin[2] + i32::from(quad.origin[2])) as f32 * VOXEL_SIZE_METRES,
            ],
            extent_voxels: quad.extent.map(u16::from),
            material_face: pack_gpu_material_face(u32::from(quad.material), quad.face),
            ao: u32::from(quad.ao),
        };
        let opaque_quads: Vec<_> = mesh.opaque.iter().map(convert).collect();
        let water_quads: Vec<_> = mesh.translucent.iter().map(convert).collect();
        let min = glam::Vec3::from_array(origin.map(|value| value as f32 * VOXEL_SIZE_METRES));
        let max = min + glam::Vec3::splat(CHUNK_EDGE as f32 * VOXEL_SIZE_METRES);
        let quad_bytes = size_of::<GpuQuad>() as u32;
        let opaque_count = mesh.opaque.len() as u32;
        let opaque_update = if opaque_count == 0 {
            None
        } else {
            let Some(prepared) = self.prepare_mesh_sliced(
                key,
                &opaque_quads,
                vec![MeshSlice {
                    relative_offset: 0,
                    size: opaque_count * quad_bytes,
                    quad_count: opaque_count,
                    bounds_min: min,
                    bounds_max: max,
                    surface_bounds: None,
                    surface_patch_id: None,
                    skirt_edge: None,
                    render_layer: RenderLayer::Opaque,
                }],
            ) else {
                return false;
            };
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
                    surface_bounds: None,
                    surface_patch_id: None,
                    skirt_edge: None,
                    render_layer: RenderLayer::Translucent,
                }],
            ) else {
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
        let lights = local_lights_for_mesh(origin, mesh);
        if lights.is_empty() {
            self.local_light_candidates.remove(&key);
        } else {
            self.local_light_candidates.insert(key, lights);
        }
        true
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
        let underlay_offset = FAR_UNDERLAY_OFFSET_METRES * (f32::from(coord.level.index()) + 1.0);
        let macro_normals = surface_macro_normals(tile);
        let gpu_quads: Vec<_> = tile
            .quads
            .iter()
            .zip(macro_normals)
            .map(|(quad, macro_normal)| GpuQuad {
                origin: [
                    quad.origin[0] as f32 * VOXEL_SIZE_METRES,
                    quad.origin[1] as f32 * VOXEL_SIZE_METRES - underlay_offset,
                    quad.origin[2] as f32 * VOXEL_SIZE_METRES,
                ],
                extent_voxels: quad.extent,
                material_face: pack_gpu_material_face(
                    u32::from(quad.material.id())
                        | FAR_MATERIAL_FLAG
                        | (u32::from(coord.level.index()) << SURFACE_LOD_SHIFT),
                    quad.face,
                ),
                ao: macro_normal,
            })
            .collect();
        let water_gpu_quads: Vec<_> = water
            .quads
            .iter()
            .map(|quad| GpuQuad {
                origin: [
                    quad.origin[0] as f32 * VOXEL_SIZE_METRES,
                    quad.origin[1] as f32 * VOXEL_SIZE_METRES,
                    quad.origin[2] as f32 * VOXEL_SIZE_METRES,
                ],
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
        let stride = coord.stride_voxels();
        let [tile_x, tile_z] = coord.voxel_origin();
        let quad_bytes = size_of::<GpuQuad>() as u32;
        let slices: Vec<_> = tile
            .patches
            .iter()
            .flat_map(|patch| {
                let patch_id = SurfacePatchId::from_tile_cell_min(
                    coord,
                    [patch.cell_bounds[0][0], patch.cell_bounds[0][1]],
                )
                .expect("validated surface terrain patch grid");
                let surface_bounds =
                    surface_patch_bounds([tile_x, tile_z], stride, patch.cell_bounds, patch.bounds);
                std::iter::once((patch.quad_range.clone(), patch.bounds, None))
                    .chain(SurfacePatchEdge::ALL.into_iter().map(|edge| {
                        let range = patch.skirt_ranges[edge.index()].clone();
                        let bounds = SurfaceBounds::from_quads(
                            &tile.quads[range.start as usize..range.end as usize],
                        )
                        .unwrap_or(patch.bounds);
                        (range, bounds, Some(edge))
                    }))
                    .map(move |(range, bounds, skirt_edge)| {
                        let bounds_min = glam::Vec3::from_array(
                            bounds.min.map(|value| value as f32 * VOXEL_SIZE_METRES),
                        ) - glam::Vec3::Y * underlay_offset;
                        let bounds_max = glam::Vec3::from_array(
                            bounds.max.map(|value| value as f32 * VOXEL_SIZE_METRES),
                        );
                        MeshSlice {
                            relative_offset: range.start * quad_bytes,
                            size: (range.end - range.start) * quad_bytes,
                            quad_count: range.end - range.start,
                            bounds_min,
                            bounds_max,
                            surface_bounds: Some(surface_bounds),
                            surface_patch_id: Some(patch_id),
                            skirt_edge,
                            render_layer: RenderLayer::Opaque,
                        }
                    })
            })
            .collect();
        let water_slices = water
            .patches
            .iter()
            .map(|patch| {
                let patch_id = SurfacePatchId::from_tile_cell_min(
                    coord,
                    [patch.cell_bounds[0][0], patch.cell_bounds[0][1]],
                );
                let surface_bounds =
                    surface_patch_bounds([tile_x, tile_z], stride, patch.cell_bounds, patch.bounds);
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
                    surface_bounds: Some(surface_bounds),
                    surface_patch_id: patch_id,
                    skirt_edge: None,
                    render_layer: RenderLayer::Translucent,
                }
            })
            .collect();
        let opaque_update = if gpu_quads.is_empty() {
            None
        } else {
            let Some(prepared) = self.prepare_mesh_sliced(key, &gpu_quads, slices) else {
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
        self.replace_surface_patch_residency(coord, resident_patch_ids);
        true
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

    pub fn remove_surface_tile(&mut self, coord: SurfaceTileCoord) {
        self.remove_mesh((coord.level.index() + 1, coord.x, 0, coord.z));
        self.replace_surface_patch_residency(coord, HashSet::new());
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
        self.surface_patch_residency_revision =
            self.surface_patch_residency_revision.wrapping_add(1);
    }

    fn refresh_surface_patch_selection(&mut self, focus: Option<GeometricLodFocus>) {
        if self.surface_patch_selection_focus == focus
            && self.surface_patch_selection_revision == self.surface_patch_residency_revision
        {
            return;
        }
        if let Some(focus) = focus {
            self.surface_patch_selection.rebuild(
                focus,
                &self.surface_patch_residency,
                &self.canonical_ready_columns,
            );
        } else {
            self.surface_patch_selection = SurfacePatchSelection::default();
        }
        self.surface_patch_selection_focus = focus;
        self.surface_patch_selection_revision = self.surface_patch_residency_revision;
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
        let environment_response = 1.0 - (-dt / 0.85).exp();
        self.environment = self
            .environment
            .lerp(self.environment_target, environment_response);
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
        let Ok(shadow_cascades) = directional_shadow_cascades(
            &self.config,
            camera,
            self.environment.sun_direction,
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
                options: self.options,
                readiness: self.lod_readiness(),
                environment: self.environment,
                underwater_blend: self.underwater_blend,
                interior: self.interior,
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
        let geometric_lod_focus =
            active_geometric_lod_focus(self.geometric_lod_focus, self.options.far_terrain);
        let lod_readiness = self.lod_readiness();
        let resident_hierarchy = geometric_lod_focus.is_some();
        if resident_hierarchy {
            self.refresh_surface_patch_selection(geometric_lod_focus);
        }
        // Queue readiness is not a proof that every fixed geometric owner is resident. Canonical
        // columns can still replace atomically and retained surface tiles can be incomplete. Keep
        // the cached resident hierarchy authoritative after settling as well as while streaming.
        let surface_patch_selection = resident_hierarchy.then_some(&self.surface_patch_selection);
        let (shadow_draw_lists, world_draw_list) = collect_opaque_draw_lists(
            &mut self.chunks,
            surface_patch_selection,
            if resident_hierarchy {
                self.surface_patch_residency_revision
            } else {
                u64::MAX
            },
            self.options.far_terrain,
            self.options.shadows,
            geometric_lod_focus,
            view_clip,
            shadow_clips,
        );
        let water_draw_list = self.collect_draw_list(
            &self.water_chunks,
            |key, chunk| {
                self.options.water
                    && (key.0 == 0 || self.options.far_terrain)
                    && view_clip.contains_aabb(chunk.bounds_min, chunk.bounds_max)
            },
            |key, slice| {
                slice.render_layer == RenderLayer::Translucent
                    && slice_owned_by_lod(geometric_lod_focus, surface_patch_selection, key, slice)
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
        self.queue
            .write_buffer(&self.frame_buffer, 0, bytemuck::bytes_of(&uniform));
        if self.options.shadows {
            self.shadow_gpu.write_cascades(
                &self.queue,
                &shadow_cascades,
                camera,
                self.options,
                self.lod_readiness(),
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
                self.options.shadows,
                refract_water,
                self.options.screen_space_ambient_occlusion,
            )
        });
        let mut shadow_draw_calls = 0;
        if self.options.shadows {
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
                pass.set_pipeline(if use_fragmentless_shadow_pipeline(lod_readiness) {
                    &self.shadow_gpu.settled_pipeline
                } else {
                    &self.shadow_gpu.pipeline
                });
                pass.set_bind_group(0, &self.shadow_gpu.bind_groups[cascade_index], &[]);
                for span in &draw_list.spans {
                    let Some(buffer) = self.arena_buffers.get(span.page as usize) else {
                        continue;
                    };
                    let start = u64::from(span.offset);
                    let end = start + u64::from(span.size);
                    pass.set_vertex_buffer(0, buffer.slice(start..end));
                    pass.draw(0..6, 0..span.quad_count);
                    shadow_draw_calls += 1;
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
                pass.set_pipeline(if self.lod_readiness().geometric {
                    &self.depth_prepass_fast_pipeline
                } else {
                    &self.depth_prepass_pipeline
                });
                pass.set_bind_group(0, &self.frame_bind_group, &[]);
                for span in &world_draw_list.spans {
                    let Some(buffer) = self.arena_buffers.get(span.page as usize) else {
                        continue;
                    };
                    let start = u64::from(span.offset);
                    let end = start + u64::from(span.size);
                    pass.set_vertex_buffer(0, buffer.slice(start..end));
                    pass.draw(0..6, 0..span.quad_count);
                    depth_prepass_draw_calls = depth_prepass_draw_calls.saturating_add(1);
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
        {
            let scene_view = if refract_water {
                self.ui_gpu.opaque_scene_view()
            } else {
                self.ui_gpu.scene_view()
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("opaque world pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: scene_view,
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
                        store: if refract_water {
                            wgpu::StoreOp::Store
                        } else {
                            wgpu::StoreOp::Discard
                        },
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: gpu_frame.as_ref().map(|frame| frame.pass(12)),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.frame_bind_group, &[]);
            pass.set_bind_group(2, self.ambient_occlusion_gpu.sample_bind_group(), &[]);
            pass.set_pipeline(if self.options.screen_space_ambient_occlusion {
                if self.options.material_detail {
                    &self.voxel_ambient_occlusion_pipeline
                } else {
                    &self.voxel_ambient_occlusion_flat_pipeline
                }
            } else if self.options.material_detail {
                &self.voxel_pipeline
            } else {
                &self.voxel_flat_pipeline
            });
            for span in &world_draw_list.spans {
                let Some(buffer) = self.arena_buffers.get(span.page as usize) else {
                    continue;
                };
                let start = u64::from(span.offset);
                let end = start + u64::from(span.size);
                pass.set_vertex_buffer(0, buffer.slice(start..end));
                pass.draw(0..6, 0..span.quad_count);
            }
            self.avatar_gpu
                .draw_scene(&mut pass, self.options.screen_space_ambient_occlusion);
            // Draw the fullscreen sky at the far plane after opaque geometry so early depth
            // rejection avoids running its procedural clouds behind terrain.
            pass.set_pipeline(&self.sky_pipeline);
            pass.draw(0..3, 0..1);
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
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: gpu_frame.as_ref().map(|frame| frame.pass(14)),
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
                pass.draw(0..6, 0..span.quad_count);
            }
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
                .spans
                .len()
                .saturating_add(water_draw_list.spans.len())
                .saturating_add(usize::from(has_avatars)) as u32,
            water_draw_calls: water_draw_list.spans.len() as u32,
            shadow_draw_calls,
            shadow_cascades: if self.options.shadows {
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
            gpu_ui_ms: gpu_timing.map(|timing| timing.ui_ms),
            cpu_cull_ms,
            cpu_encode_ms: 0.0,
            cpu_submit_ms: 0.0,
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
            surface_width: self.config.width,
            surface_height: self.config.height,
            dpr: self.dpr,
            ambient_occlusion_bytes: self.ambient_occlusion_gpu.bytes(),
            depth_prepass_draw_calls,
            screen_space_ambient_occlusion: self.options.screen_space_ambient_occlusion,
            material_detail: self.options.material_detail,
            daylight_phase: self.daylight_phase as u8,
            surface_region: self.surface_region as u8,
            cloud_coverage: self.environment.cloud_coverage,
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
                + if has_avatars && self.options.shadows {
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
                timestamp_writes: gpu_frame.as_ref().map(|frame| frame.pass(16)),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.ui_gpu.draw(&mut pass);
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

/// Builds the camera and three shadow selections in one resident-mesh traversal.
///
/// Geometric LOD ownership is independent of clip volume. Computing it once per opaque slice avoids
/// repeating the most expensive culling predicate for the camera and every shadow cascade while
/// preserving each list's independent clip tests, diagnostics, ordering, and fingerprint.
fn collect_opaque_draw_lists(
    chunks: &mut BTreeMap<MeshKey, ChunkMesh>,
    surface_patch_selection: Option<&SurfacePatchSelection>,
    residency_revision: u64,
    far_terrain: bool,
    shadows: bool,
    geometric_lod_focus: Option<GeometricLodFocus>,
    view_clip: AabbClipVolume,
    shadow_clips: [AabbClipVolume; CASCADE_COUNT],
) -> ([DrawList; CASCADE_COUNT], DrawList) {
    let mut shadow_builders: [DrawListBuilder; CASCADE_COUNT] = Default::default();
    let mut world_builder = DrawListBuilder::default();

    for (key, chunk) in chunks {
        if !chunk.active() || (key.0 != 0 && !far_terrain) {
            continue;
        }
        let world_chunk_visible = view_clip.contains_aabb(chunk.bounds_min, chunk.bounds_max);
        let shadow_chunk_visible: [bool; CASCADE_COUNT] = std::array::from_fn(|cascade_index| {
            shadows
                && mesh_casts_directional_shadow(key)
                && shadow_clips[cascade_index].contains_aabb(chunk.bounds_min, chunk.bounds_max)
        });
        if !world_chunk_visible && !shadow_chunk_visible.into_iter().any(|visible| visible) {
            continue;
        }
        chunk.refresh_lod_ownership(
            key,
            geometric_lod_focus,
            surface_patch_selection,
            residency_revision,
        );

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
                || !chunk.lod_owns_slice(geometric_lod_focus, slice_index)
            {
                continue;
            }
            if world_chunk_visible && view_clip.contains_aabb(slice.bounds_min, slice.bounds_max) {
                world_builder.select_slice(chunk, slice);
                world_mesh_selected = true;
            }
            for cascade_index in 0..CASCADE_COUNT {
                if shadow_chunk_visible[cascade_index]
                    // Transition skirts are synthetic crack-hiding curtains rather than terrain.
                    // Letting their deep vertical faces cast directional shadows creates moving
                    // shadow walls whenever a snapped LOD boundary advances with the camera.
                    && slice.skirt_edge.is_none()
                    && shadow_clips[cascade_index].contains_aabb(slice.bounds_min, slice.bounds_max)
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
        shadow_builders.map(DrawListBuilder::finish)
    } else {
        std::array::from_fn(|_| DrawList::default())
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
    slices: Vec<MeshSlice>,
    activation_mask: u8,
    buffer_label: &'static str,
) -> Option<ChunkMesh> {
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
    while arena_buffers.len() <= allocation.page as usize {
        let page = arena_buffers.len() as u16;
        let Some(capacity) = arena.page_capacity(page) else {
            let _ = arena.free(allocation);
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
        return None;
    };
    queue.write_buffer(buffer, u64::from(allocation.offset), bytes);
    Some(ChunkMesh {
        allocation,
        quad_count: gpu_quads.len() as u32,
        content_fingerprint: fingerprint_bytes(bytes),
        slices,
        lod_ownership_focus: None,
        lod_residency_revision: 0,
        lod_owned_slices: Vec::new(),
        bounds_min,
        bounds_max,
        activation_mask,
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
    let height_at = |x: usize, z: usize| heights[x + z * edge].map(|sample| sample.0);
    for z in 0..edge {
        for x in 0..edge {
            let Some((height, quad_index)) = heights[x + z * edge] else {
                continue;
            };
            let left = x.checked_sub(1).and_then(|x| height_at(x, z));
            let right = (x + 1 < edge).then(|| height_at(x + 1, z)).flatten();
            let back = z.checked_sub(1).and_then(|z| height_at(x, z));
            let front = (z + 1 < edge).then(|| height_at(x, z + 1)).flatten();
            let slope_x = sampled_surface_slope(height, left, right, stride);
            let slope_z = sampled_surface_slope(height, back, front, stride);
            // A decimated height field can turn sub-cell relief into a much steeper apparent
            // gradient when its sampling phase changes at an LOD handoff. Preserve the direction
            // and broad slope cue while compressing that unstable high-frequency component.
            let horizontal = stabilized_surface_gradient(glam::Vec2::new(slope_x, slope_z));
            let normal = glam::Vec3::new(-horizontal.x, 1.0, -horizontal.y).normalize();
            let encode =
                |component: f32| ((component.clamp(-1.0, 1.0) * 0.5 + 0.5) * 255.0).round() as u32;
            let value = 0xff
                | (encode(normal.x) << 8)
                | (encode(normal.z) << 16)
                | SURFACE_MACRO_NORMAL_FLAG;
            packed[quad_index] = value;
            cell_normals[x + z * edge] = Some(value);
        }
    }

    // Coarse height fields represent a smooth slope with flat tops separated by tall voxel walls.
    // Give only those generated terrain-body walls the owning cell's bounded slope normal. This
    // prevents distant hills from becoming black combs without adding geometry or accidentally
    // smoothing canonical cliffs, skyline proxies, or disposable transition skirts.
    for patch in &tile.patches {
        let start = patch.quad_range.start as usize;
        let end = patch.quad_range.end as usize;
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
    packed
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

fn surface_patch_bounds(
    tile_origin: [i32; 2],
    stride: i32,
    cell_bounds: [[u8; 2]; 2],
    geometry_bounds: SurfaceBounds,
) -> SurfaceBounds {
    let [[min_x, min_z], [max_x, max_z]] = cell_bounds;
    let offset =
        |origin: i32, cell: u8| origin.saturating_add(i32::from(cell).saturating_mul(stride));
    SurfaceBounds {
        min: [
            offset(tile_origin[0], min_x),
            geometry_bounds.min[1],
            offset(tile_origin[1], min_z),
        ],
        max: [
            offset(tile_origin[0], max_x),
            geometry_bounds.max[1],
            offset(tile_origin[1], max_z),
        ],
    }
}

fn surface_patch_belongs_to_tile(patch: SurfacePatchId, tile: SurfaceTileCoord) -> bool {
    patch.level == tile.level
        && patch
            .x
            .div_euclid(voxels_world::SURFACE_PATCHES_PER_TILE_EDGE)
            == tile.x
        && patch
            .z
            .div_euclid(voxels_world::SURFACE_PATCHES_PER_TILE_EDGE)
            == tile.z
}

fn slice_owned_by_lod(
    focus: Option<GeometricLodFocus>,
    surface_patch_selection: Option<&SurfacePatchSelection>,
    key: &MeshKey,
    slice: &MeshSlice,
) -> bool {
    let Some(focus) = focus else {
        return true;
    };
    if key.0 == 0 {
        return focus.owns_canonical_chunk(key.1, key.3);
    }
    let Some(level) = SurfaceLodLevel::ALL.get(usize::from(key.0 - 1)).copied() else {
        return false;
    };
    if slice
        .surface_patch_id
        .is_some_and(|patch_id| patch_id.level != level)
    {
        return false;
    }
    if let (Some(patch_id), Some(selection)) = (slice.surface_patch_id, surface_patch_selection) {
        return selection.owns(patch_id, slice.skirt_edge);
    }
    slice.surface_bounds.is_some_and(|bounds| {
        slice.skirt_edge.map_or_else(
            || focus.owns_surface_bounds(level, bounds),
            |edge| focus.owns_surface_skirt(level, bounds, edge),
        )
    })
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

const fn use_fragmentless_shadow_pipeline(readiness: LodReadiness) -> bool {
    // The shadow shader first hides canonical vegetation until fine coverage is complete, then
    // accepts every fragment when CPU geometric ownership is active. Both conditions are required
    // before removing its fragment stage.
    readiness.fine && readiness.geometric
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
    let items = PLACEMENT_MATERIALS
        .into_iter()
        .filter(|material| inventory.count(*material) > 0)
        .map(|material| InventoryItem {
            label: placement_material_label(material),
            count: inventory.count(material),
            color: inventory_material_color(material),
        })
        .collect::<Vec<_>>();
    let selected_index = selected.and_then(|selected| {
        PLACEMENT_MATERIALS
            .into_iter()
            .filter(|material| inventory.count(*material) > 0)
            .position(|material| material == selected)
    });
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

fn frame_uniform(
    config: &SurfaceConfiguration,
    camera: &CameraState,
    time: f32,
    target: Option<DigVolume>,
    state: FrameState,
    shadows: &DirectionalShadowCascades,
    renderer_config: RendererConfig,
) -> FrameUniform {
    let FrameState {
        options,
        readiness,
        environment,
        underwater_blend,
        interior,
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
                1.0,
            ]
        }),
        target_voxel_max: target.map_or([0.0; 4], |volume| {
            [
                volume.max.x as f32,
                volume.max.y as f32,
                volume.max.z as f32,
                1.0,
            ]
        }),
        render_options: [
            if options.ambient_occlusion { 1.0 } else { 0.0 },
            if options.fog { 1.0 } else { 0.0 },
            if options.far_terrain { 1.0 } else { 0.0 },
            if options.target_outline { 1.0 } else { 0.0 },
        ],
        lod_options: [
            if readiness.fine { 1.0 } else { 0.0 },
            if options.far_terrain { 1.0 } else { 0.0 },
            if readiness.all { 1.0 } else { 0.0 },
            if readiness.geometric { 1.0 } else { 0.0 },
        ],
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
        sun_direction: environment.sun_direction.extend(0.0).to_array(),
        sun_radiance: environment.sun_radiance.extend(0.0).to_array(),
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
    }
}

fn shadow_frame_uniform(
    shadows: &DirectionalShadowCascades,
    cascade_index: usize,
    camera: &CameraState,
    options: RenderOptions,
    readiness: LodReadiness,
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
        lod_options: [
            if readiness.fine { 1.0 } else { 0.0 },
            if options.far_terrain { 1.0 } else { 0.0 },
            if readiness.all { 1.0 } else { 0.0 },
            if readiness.geometric { 1.0 } else { 0.0 },
        ],
    }
}

fn directional_shadow_cascades(
    config: &SurfaceConfiguration,
    camera: &CameraState,
    sun_direction: glam::Vec3,
    shadow_config: DirectionalShadowConfig,
) -> Result<DirectionalShadowCascades, String> {
    let aspect = config.width as f32 / config.height.max(1) as f32;
    build_directional_shadow_cascades(camera, aspect, -sun_direction, shadow_config)
        .map_err(|error| format!("build shadow cascades: {error:?}"))
}

fn bounded_frame_delta(dt: f32) -> f32 {
    if dt.is_finite() && dt > 0.0 {
        dt.min(0.1)
    } else {
        0.0
    }
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
    fragment_entry: &'a str,
    blend: Option<wgpu::BlendState>,
    write_mask: wgpu::ColorWrites,
    depth_stencil: Option<wgpu::DepthStencilState>,
    fragment_constants: &'a [(&'a str, f64)],
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
            entry_point: Some("vs_main"),
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
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: options.depth_stencil,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn depth_only_pipeline(
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
            entry_point: Some("vs_main"),
            buffers: &[Some(quad_layout())],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_depth"),
            targets: &[],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
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
            entry_point: Some("vs_main"),
            buffers: &[Some(quad_layout())],
            compilation_options: Default::default(),
        },
        fragment: None,
        primitive: wgpu::PrimitiveState::default(),
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

fn quad_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRIBUTES: [wgpu::VertexAttribute; 4] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Uint16x2, 2 => Uint32, 3 => Uint32];
    wgpu::VertexBufferLayout {
        array_stride: size_of::<GpuQuad>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &ATTRIBUTES,
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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(value & 0xff, 0xff, "voxel AO remains fully visible");
        let normal_x = ((value >> 8) & 255) as f32 * (2.0 / 255.0) - 1.0;
        let normal_z = ((value >> 16) & 255) as f32 * (2.0 / 255.0) - 1.0;
        assert!(
            (-0.14..-0.10).contains(&normal_x),
            "uphill +X must retain a gentle, stable tilt toward -X: {normal_x}"
        );
        assert!(normal_z.abs() < 0.01);
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
            origin: [-123.5, 0.25, 8192.0],
            extent_voxels: [u16::MAX, 1],
            material_face: pack_gpu_material_face(materials[3], 5),
            ao: u32::MAX,
        };
        let bytes = bytemuck::bytes_of(&quad);
        assert_eq!(bytes.len(), 24);
        assert_eq!(quad.extent_voxels, [u16::MAX, 1]);
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
    fn configured_feature_baseline_drives_initial_options_and_reset() {
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

        let mut ui = MissionControlUi::new(MissionControlConfig::default(), baseline);
        let _ = ui.set_feature(RendererFeature::CascadedSunShadows, true);
        let _ = ui.set_feature(RendererFeature::AtmosphericFog, false);
        let reset = reset_renderer_features(&mut ui, baseline);

        assert_eq!(reset, expected);
        for feature in RendererFeature::ALL {
            assert_eq!(ui.feature_enabled(feature), baseline.enabled(feature));
        }
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
    fn fragmentless_shadows_require_complete_fine_cpu_geometric_ownership() {
        for fine in [false, true] {
            for all in [false, true] {
                for geometric in [false, true] {
                    let readiness = LodReadiness {
                        fine,
                        all,
                        geometric,
                    };
                    assert_eq!(
                        use_fragmentless_shadow_pipeline(readiness),
                        fine && geometric
                    );
                }
            }
        }
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
        let prepared = arena.allocate(64).expect("prepared allocation");
        let mut chunks = BTreeMap::from([(
            key,
            ChunkMesh {
                allocation: resident,
                quad_count: 1,
                content_fingerprint: 1,
                slices: Vec::new(),
                lod_ownership_focus: None,
                lod_residency_revision: 0,
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
                quad_count: 2,
                content_fingerprint: 2,
                slices: Vec::new(),
                lod_ownership_focus: None,
                lod_residency_revision: 0,
                lod_owned_slices: Vec::new(),
                bounds_min: glam::Vec3::ZERO,
                bounds_max: glam::Vec3::ZERO,
                activation_mask: u8::MAX,
            }),
        );

        assert_eq!(chunks.get(&key).map(|mesh| mesh.allocation), Some(resident));
        assert_eq!(arena.stats().allocated_bytes, u64::from(resident.size));
        assert!(!arena.free(prepared));
        assert!(arena.free(chunks.remove(&key).expect("resident mesh").allocation));
    }

    fn test_slice(surface_bounds: Option<SurfaceBounds>) -> MeshSlice {
        MeshSlice {
            relative_offset: 0,
            size: size_of::<GpuQuad>() as u32,
            quad_count: 1,
            bounds_min: glam::Vec3::splat(-10_000.0),
            bounds_max: glam::Vec3::splat(10_000.0),
            surface_bounds,
            surface_patch_id: None,
            skirt_edge: None,
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
            13_800_000, 14_000_000, 15_000_000,
        ];
        let timing = parse_gpu_timestamps(&timestamps, 1.0, true, true, true)
            .unwrap_or_else(|| panic!("valid timestamps should parse"));
        assert!((timing.total_ms - 14.0).abs() < f32::EPSILON);
        assert!((timing.shadow_ms - 3.0).abs() < f32::EPSILON);
        assert!((timing.depth_prepass_ms - 2.0).abs() < f32::EPSILON);
        assert!((timing.ambient_occlusion_ms - 1.5).abs() < f32::EPSILON);
        assert!((timing.world_ms - 2.0).abs() < f32::EPSILON);
        assert!((timing.water_ms - 3.0).abs() < f32::EPSILON);
        assert!((timing.ui_ms - 1.0).abs() < f32::EPSILON);

        let mut skipped = timestamps;
        skipped[0..6].copy_from_slice(&[90, 80, 70, 60, 50, 40]);
        skipped[6..12].copy_from_slice(&[39, 38, 37, 36, 35, 34]);
        skipped[14..16].copy_from_slice(&[30, 20]);
        let timing = parse_gpu_timestamps(&skipped, 1.0, false, false, false)
            .unwrap_or_else(|| panic!("inactive timestamp pairs should be ignored"));
        assert_eq!(timing.shadow_ms, 0.0);
        assert_eq!(timing.depth_prepass_ms, 0.0);
        assert_eq!(timing.ambient_occlusion_ms, 0.0);
        assert_eq!(timing.water_ms, 0.0);
    }

    #[test]
    fn gpu_timestamp_parser_rejects_invalid_or_implausible_samples() {
        let mut timestamps = [0u64; GPU_QUERY_COUNT as usize];
        for (index, timestamp) in timestamps.iter_mut().enumerate() {
            *timestamp = index as u64 * 1_000_000;
        }
        assert!(parse_gpu_timestamps(&timestamps, 0.0, false, false, false).is_none());
        timestamps[13] = timestamps[12] - 1;
        assert!(parse_gpu_timestamps(&timestamps, 1.0, false, false, false).is_none());
        timestamps[13] = timestamps[12] + 1;
        timestamps[17] = timestamps[12] + 1_100_000_000;
        assert!(parse_gpu_timestamps(&timestamps, 1.0, false, false, false).is_none());
        assert_eq!(GPU_QUERY_BUFFER_BYTES, 144);
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
            },
            DrawItem {
                page: 0,
                offset: 96,
                size: 64,
                quad_count: 2,
            },
            DrawItem {
                page: 0,
                offset: 0,
                size: 96,
                quad_count: 3,
            },
            DrawItem {
                page: 0,
                offset: 192,
                size: 32,
                quad_count: 1,
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
                },
                DrawSpan {
                    page: 0,
                    offset: 192,
                    size: 32,
                    quad_count: 1,
                },
                DrawSpan {
                    page: 1,
                    offset: 64,
                    size: 32,
                    quad_count: 1,
                },
            ]
        );
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
            surface_bounds: None,
            surface_patch_id: None,
            skirt_edge: None,
            render_layer: RenderLayer::Opaque,
        };
        let surface_slice = MeshSlice {
            relative_offset: 0,
            size: size_of::<GpuQuad>() as u32 * 2,
            quad_count: 2,
            bounds_min,
            bounds_max,
            surface_bounds: Some(SurfaceBounds {
                min: [96, -20, 0],
                max: [112, 80, 16],
            }),
            surface_patch_id: Some(SurfacePatchId::new(SurfaceLodLevel::Stride2, 6, 0)),
            skirt_edge: None,
            render_layer: RenderLayer::Opaque,
        };
        let surface_skirt_slice = MeshSlice {
            relative_offset: surface_slice.size,
            size: size_of::<GpuQuad>() as u32,
            quad_count: 1,
            skirt_edge: Some(SurfacePatchEdge::NegativeX),
            ..surface_slice
        };
        let mut arena = ArenaAllocator::new(256, 1);
        let canonical_allocation = arena
            .allocate(canonical_slice.size)
            .expect("canonical test allocation");
        let surface_allocation = arena
            .allocate(surface_slice.size + surface_skirt_slice.size)
            .expect("surface test allocation");
        let mut chunks = BTreeMap::from([
            (
                canonical_key,
                ChunkMesh {
                    allocation: canonical_allocation,
                    quad_count: canonical_slice.quad_count,
                    content_fingerprint: 11,
                    slices: vec![canonical_slice],
                    lod_ownership_focus: None,
                    lod_residency_revision: 0,
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
                    quad_count: surface_slice.quad_count + surface_skirt_slice.quad_count,
                    content_fingerprint: 22,
                    slices: vec![surface_slice, surface_skirt_slice],
                    lod_ownership_focus: None,
                    lod_residency_revision: 0,
                    lod_owned_slices: Vec::new(),
                    bounds_min,
                    bounds_max,
                    activation_mask: u8::MAX,
                },
            ),
        ]);
        let focus = Some(GeometricLodFocus::snapped(0, 0));
        let surface_patch_residency =
            HashSet::from([surface_slice.surface_patch_id.expect("surface patch id")]);
        let mut surface_patch_selection = SurfacePatchSelection::default();
        surface_patch_selection.rebuild(focus.unwrap(), &surface_patch_residency, &HashSet::new());
        let view_clip = AabbClipVolume::new(glam::Mat4::IDENTITY);
        let shadow_clips = [view_clip; CASCADE_COUNT];
        let (actual_shadows, actual_world) = collect_opaque_draw_lists(
            &mut chunks,
            Some(&surface_patch_selection),
            1,
            true,
            true,
            focus,
            view_clip,
            shadow_clips,
        );
        let expected_world = reference_draw_list(
            &chunks,
            |_, chunk| view_clip.contains_aabb(chunk.bounds_min, chunk.bounds_max),
            |key, slice| {
                slice.render_layer == RenderLayer::Opaque
                    && slice_owned_by_lod(focus, Some(&surface_patch_selection), key, slice)
                    && view_clip.contains_aabb(slice.bounds_min, slice.bounds_max)
            },
        );
        let expected_shadows = std::array::from_fn(|cascade_index| {
            reference_draw_list(
                &chunks,
                |key, chunk| {
                    mesh_casts_directional_shadow(key)
                        && shadow_clips[cascade_index]
                            .contains_aabb(chunk.bounds_min, chunk.bounds_max)
                },
                |key, slice| {
                    slice.render_layer == RenderLayer::Opaque
                        && slice_owned_by_lod(focus, Some(&surface_patch_selection), key, slice)
                        && slice.skirt_edge.is_none()
                        && shadow_clips[cascade_index]
                            .contains_aabb(slice.bounds_min, slice.bounds_max)
                },
            )
        });
        assert_eq!(actual_world, expected_world);
        assert_eq!(actual_shadows, expected_shadows);
        assert!(actual_world.quad_count > actual_shadows[0].quad_count);

        let cached_world = collect_opaque_draw_lists(
            &mut chunks,
            Some(&surface_patch_selection),
            1,
            true,
            true,
            focus,
            view_clip,
            shadow_clips,
        )
        .1;
        assert_eq!(cached_world, expected_world);
        assert!(
            chunks
                .values()
                .all(|chunk| chunk.lod_ownership_focus == focus)
        );

        let moved_focus = Some(GeometricLodFocus::snapped(256, -192));
        surface_patch_selection.rebuild(
            moved_focus.unwrap(),
            &surface_patch_residency,
            &HashSet::new(),
        );
        let moved_world = collect_opaque_draw_lists(
            &mut chunks,
            Some(&surface_patch_selection),
            1,
            true,
            true,
            moved_focus,
            view_clip,
            shadow_clips,
        )
        .1;
        let moved_expected = reference_draw_list(
            &chunks,
            |_, chunk| view_clip.contains_aabb(chunk.bounds_min, chunk.bounds_max),
            |key, slice| {
                slice.render_layer == RenderLayer::Opaque
                    && slice_owned_by_lod(moved_focus, Some(&surface_patch_selection), key, slice)
                    && view_clip.contains_aabb(slice.bounds_min, slice.bounds_max)
            },
        );
        assert_eq!(moved_world, moved_expected);
        assert!(
            chunks
                .values()
                .all(|chunk| chunk.lod_ownership_focus == moved_focus)
        );
    }

    #[test]
    fn geometric_lod_selects_canonical_chunks_and_surface_patches_exclusively() {
        let focus = GeometricLodFocus::snapped(0, 0);
        assert!(slice_owned_by_lod(
            Some(focus),
            None,
            &(0, 0, 0, 0),
            &test_slice(None)
        ));
        assert!(!slice_owned_by_lod(
            Some(focus),
            None,
            &(0, 7, 0, 0),
            &test_slice(None)
        ));

        let stride_two_patch = test_slice(Some(SurfaceBounds {
            min: [96, -20, 0],
            max: [112, 80, 16],
        }));
        assert!(slice_owned_by_lod(
            Some(focus),
            None,
            &(SurfaceLodLevel::Stride2.index() + 1, 1, 0, 0),
            &stride_two_patch
        ));
        assert!(!slice_owned_by_lod(
            Some(focus),
            None,
            &(SurfaceLodLevel::Stride4.index() + 1, 1, 0, 0),
            &stride_two_patch
        ));
    }

    #[test]
    fn resident_hierarchy_keeps_surface_cover_until_canonical_column_is_complete() {
        let focus = GeometricLodFocus::snapped(0, 0);
        let patch_id = SurfacePatchId::new(SurfaceLodLevel::Stride2, 0, 0);
        let mut surface = test_slice(Some(SurfaceBounds {
            min: [0, -20, 0],
            max: [16, 80, 16],
        }));
        surface.surface_patch_id = Some(patch_id);
        let resident = HashSet::from([patch_id]);
        let mut selection = SurfacePatchSelection::default();
        selection.rebuild(focus, &resident, &HashSet::new());
        assert!(slice_owned_by_lod(
            Some(focus),
            Some(&selection),
            &(SurfaceLodLevel::Stride2.index() + 1, 0, 0, 0),
            &surface,
        ));

        selection.rebuild(focus, &resident, &HashSet::from([(0, 0)]));
        assert!(!slice_owned_by_lod(
            Some(focus),
            Some(&selection),
            &(SurfaceLodLevel::Stride2.index() + 1, 0, 0, 0),
            &surface,
        ));
    }

    #[test]
    fn geometric_lod_uses_fixed_coverage_not_protruding_geometry_bounds() {
        let focus = GeometricLodFocus::snapped(0, 0);
        let slice = test_slice(Some(SurfaceBounds {
            min: [256, -500, 0],
            max: [288, 500, 32],
        }));
        assert!(slice.bounds_min.x < -9_000.0);
        assert!(slice.bounds_max.x > 9_000.0);
        assert!(slice_owned_by_lod(
            Some(focus),
            None,
            &(SurfaceLodLevel::Stride4.index() + 1, 2, 0, 0),
            &slice
        ));
        assert!(!slice_owned_by_lod(
            Some(focus),
            None,
            &(SurfaceLodLevel::Stride8.index() + 1, 1, 0, 0),
            &slice
        ));
    }

    #[test]
    fn surface_patch_coverage_stays_inside_the_positive_world_boundary() {
        let coord = SurfaceTileCoord::containing(SurfaceLodLevel::Stride16, i32::MAX, i32::MAX);
        let geometry_bounds = SurfaceBounds {
            min: [i32::MAX - 127, -40, i32::MAX - 127],
            max: [i32::MAX, 80, i32::MAX],
        };

        assert_eq!(
            surface_patch_bounds(
                coord.voxel_origin(),
                coord.stride_voxels(),
                [[24, 24], [32, 32]],
                geometry_bounds,
            ),
            geometry_bounds
        );
    }

    #[test]
    fn geometric_lod_falls_back_to_all_resident_meshes_until_ready() {
        let surface = test_slice(None);
        assert!(slice_owned_by_lod(None, None, &(99, 0, 0, 0), &surface));
        assert!(!slice_owned_by_lod(
            Some(GeometricLodFocus::snapped(0, 0)),
            None,
            &(99, 0, 0, 0),
            &surface
        ));
    }

    #[test]
    fn disabling_far_terrain_keeps_resident_canonical_coverage() {
        let settled = Some(GeometricLodFocus::snapped(0, 0));
        let canonical = test_slice(None);
        let outside_inner_cut = (0, 4, 0, 0);

        assert!(!slice_owned_by_lod(
            active_geometric_lod_focus(settled, true),
            None,
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
    }
}
