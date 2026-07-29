//! Failure-atomic GPU snapshots for virtual microvoxel terrain.
//!
//! The CPU hierarchy is the sole selection authority. A candidate cut is supplied explicitly,
//! assigned deterministic stream destinations, and expanded into 32-bit geometry handles in the
//! inactive bank. Independent GPU passes certify descriptor structure and compare every destination
//! and value exactly. The uploaded descriptors are also read back and reconstructed against
//! canonical CPU ranges, so encoder and validator agreement cannot authenticate corrupt input. The
//! published bank is never written. Promotion is a CPU-side bank swap only after the exact candidate
//! generation, fingerprint, page count, geometry counts, bounds, and indirect commands are read
//! back.

use crate::virtual_terrain::VirtualTerrainCapacity;
use bytemuck::{Pod, Zeroable};
use std::collections::BTreeMap;
use std::mem::size_of;
#[cfg(test)]
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use voxels_world::TerrainPageKey;
use wgpu::util::DeviceExt;
use wgpu::{Buffer, CommandEncoder, ComputePipeline, Device, QuerySet, Queue};

const GPU_SNAPSHOT_READBACK_SLOTS: usize = 3;
const GPU_GEOMETRY_ELEMENT_BYTES: u64 = 24;
const GPU_HANDLE_SEGMENT_BIT: u32 = 1 << 31;
const GPU_HANDLE_ELEMENT_MASK: u32 = GPU_HANDLE_SEGMENT_BIT - 1;

// These are logical stream ceilings. The old implementation allocated this many geometry bytes
// again and copied every selected element. A handle bank needs only one u32 per logical element.
pub(crate) const VIRTUAL_TERRAIN_SURFACE_HANDLE_SOURCE_BYTES: u64 = 64 * 1_024 * 1_024;
pub(crate) const VIRTUAL_TERRAIN_TRIANGLE_HANDLE_SOURCE_BYTES: u64 = 96 * 1_024 * 1_024;
pub(crate) const VIRTUAL_TERRAIN_WATER_SURFACE_HANDLE_SOURCE_BYTES: u64 = 16 * 1_024 * 1_024;
pub(crate) const VIRTUAL_TERRAIN_WATER_TRIANGLE_HANDLE_SOURCE_BYTES: u64 = 16 * 1_024 * 1_024;

pub(crate) const VIRTUAL_TERRAIN_SURFACE_INDIRECT_OFFSET: u64 = 0;
pub(crate) const VIRTUAL_TERRAIN_TRIANGLE_INDIRECT_OFFSET: u64 = 16;
pub(crate) const VIRTUAL_TERRAIN_WATER_SURFACE_INDIRECT_OFFSET: u64 = 32;
pub(crate) const VIRTUAL_TERRAIN_WATER_TRIANGLE_INDIRECT_OFFSET: u64 = 48;

const STREAM_SURFACE: usize = 0;
const STREAM_TRIANGLE: usize = 1;
const STREAM_WATER_SURFACE: usize = 2;
const STREAM_WATER_TRIANGLE: usize = 3;
const STREAM_COUNT: usize = 4;

const fn stream_element_capacity(bytes: u64) -> u32 {
    (bytes / GPU_GEOMETRY_ELEMENT_BYTES) as u32
}

pub(crate) const VIRTUAL_TERRAIN_HANDLE_CAPACITIES: [u32; STREAM_COUNT] = [
    stream_element_capacity(VIRTUAL_TERRAIN_SURFACE_HANDLE_SOURCE_BYTES),
    stream_element_capacity(VIRTUAL_TERRAIN_TRIANGLE_HANDLE_SOURCE_BYTES),
    stream_element_capacity(VIRTUAL_TERRAIN_WATER_SURFACE_HANDLE_SOURCE_BYTES),
    stream_element_capacity(VIRTUAL_TERRAIN_WATER_TRIANGLE_HANDLE_SOURCE_BYTES),
];

pub(crate) const VIRTUAL_TERRAIN_HANDLE_OFFSETS: [u32; STREAM_COUNT] = [
    0,
    VIRTUAL_TERRAIN_HANDLE_CAPACITIES[0],
    VIRTUAL_TERRAIN_HANDLE_CAPACITIES[0] + VIRTUAL_TERRAIN_HANDLE_CAPACITIES[1],
    VIRTUAL_TERRAIN_HANDLE_CAPACITIES[0]
        + VIRTUAL_TERRAIN_HANDLE_CAPACITIES[1]
        + VIRTUAL_TERRAIN_HANDLE_CAPACITIES[2],
];

pub(crate) const VIRTUAL_TERRAIN_HANDLE_BANK_BYTES: u64 =
    (VIRTUAL_TERRAIN_HANDLE_OFFSETS[3] + VIRTUAL_TERRAIN_HANDLE_CAPACITIES[3]) as u64
        * size_of::<u32>() as u64;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
struct GpuCandidatePage {
    // Each pair is (packed first handle, element count).
    ranges: [[u32; 2]; STREAM_COUNT],
    // Stream-relative, exclusive-prefix destinations assigned deterministically by the CPU.
    destinations: [u32; STREAM_COUNT],
}

const _: () = assert!(size_of::<GpuCandidatePage>() == 48);

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
struct GpuSnapshotCounters {
    element_counts: [u32; STREAM_COUNT],
    encoded_pages: u32,
    overflow_flags: u32,
    generation: [u32; 2],
    fingerprint: [u32; 2],
    selected_count: u32,
    ownerless_roots: u32,
    source_element_capacities: [u32; 2],
    reserved: [u32; 2],
}

const _: () = assert!(size_of::<GpuSnapshotCounters>() == 64);

const GPU_SNAPSHOT_INDIRECT_WORDS: usize = 16;
const GPU_SNAPSHOT_INDIRECT_BYTES: u64 = (GPU_SNAPSHOT_INDIRECT_WORDS * size_of::<u32>()) as u64;
const GPU_SNAPSHOT_READBACK_PREFIX_BYTES: u64 =
    size_of::<GpuSnapshotCounters>() as u64 + GPU_SNAPSHOT_INDIRECT_BYTES;

const VALIDATION_DESCRIPTOR_MISMATCH: u32 = 1 << 8;
const VALIDATION_INDIRECT_MISMATCH: u32 = 1 << 10;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct VirtualTerrainGpuGeometryRange {
    pub source_segment: u32,
    pub source_offset_bytes: u64,
    pub element_count: u32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct VirtualTerrainGpuGeometry {
    pub opaque_surface: VirtualTerrainGpuGeometryRange,
    pub opaque_triangle: VirtualTerrainGpuGeometryRange,
    pub water_surface: VirtualTerrainGpuGeometryRange,
    pub water_triangle: VirtualTerrainGpuGeometryRange,
}

impl VirtualTerrainGpuGeometry {
    const fn ranges(self) -> [VirtualTerrainGpuGeometryRange; STREAM_COUNT] {
        [
            self.opaque_surface,
            self.opaque_triangle,
            self.water_surface,
            self.water_triangle,
        ]
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct GpuVirtualTerrainFeedback {
    pub submission_id: u64,
    pub oracle_fingerprint: u64,
    pub selected_pages: Vec<TerrainPageKey>,
    pub ownerless_roots: u32,
    pub encoded_surface_handles: u32,
    pub encoded_triangle_handles: u32,
    pub encoded_water_surface_handles: u32,
    pub encoded_water_triangle_handles: u32,
    pub encoded_pages: u32,
    pub encoding_overflow_flags: u32,
}

impl GpuVirtualTerrainFeedback {
    pub const fn ownership_overflowed(&self) -> bool {
        self.encoding_overflow_flags != 0
    }
}

#[derive(Clone, Copy)]
pub(crate) struct VirtualTerrainGpuTimestampWrites<'a> {
    pub query_set: &'a QuerySet,
    pub encoding_first_query: u32,
    pub finalize_first_query: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum VirtualTerrainGpuError {
    GeometryCapacity,
    InvalidGeometry,
    UnknownPage(TerrainPageKey),
    DeviceLimit,
    CandidateNotCertified,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SnapshotMetadata {
    generation: u64,
    fingerprint: u64,
    selected_pages: Vec<TerrainPageKey>,
    ownerless_roots: u32,
    expected_counts: [u32; STREAM_COUNT],
    expected_ranges: Vec<[[u32; 2]; STREAM_COUNT]>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SnapshotPendingState {
    Recorded,
    SubmittedAwaitingReadback,
    InFlight,
    Succeeded,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PendingSnapshotFeedback {
    generation: u64,
    state: SnapshotPendingState,
}

#[derive(Clone, Debug)]
struct GpuSnapshotReadback {
    counters: GpuSnapshotCounters,
    indirect_commands: [u32; GPU_SNAPSHOT_INDIRECT_WORDS],
    candidate_pages: Vec<GpuCandidatePage>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct VirtualTerrainSnapshotIdentity<'a> {
    pub fingerprint: u64,
    pub selected_pages: &'a [TerrainPageKey],
    pub ownerless_roots: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum VirtualTerrainCandidateWork {
    Encode,
    ReadbackOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum VirtualTerrainCandidateEncodeOutcome {
    Encoded(u64),
    ReadbackOnly(u64),
}

struct SnapshotBank {
    handles: Buffer,
    counters: Buffer,
    indirect: Buffer,
    encode_bind_group: wgpu::BindGroup,
    render_bind_group: wgpu::BindGroup,
    metadata: Option<SnapshotMetadata>,
}

struct SnapshotReadbackSlot {
    buffer: Buffer,
    available: Arc<AtomicBool>,
}

pub(crate) struct VirtualTerrainGpuControl {
    capacity: VirtualTerrainCapacity,
    geometries: BTreeMap<TerrainPageKey, VirtualTerrainGpuGeometry>,
    candidate_pages: Buffer,
    candidate_page_tokens: Buffer,
    source_buffers: [Buffer; 2],
    source_element_capacities: [u32; 2],
    bound_source_count: usize,
    render_layout: wgpu::BindGroupLayout,
    structural_pipeline: ComputePipeline,
    encode_pipeline: ComputePipeline,
    validate_pipeline: ComputePipeline,
    banks: [SnapshotBank; 2],
    active_bank: usize,
    pending_bank: Option<usize>,
    active_geometry_dirty: bool,
    next_generation: u64,
    latest_raw_feedback: Arc<Mutex<Option<GpuSnapshotReadback>>>,
    minimum_feedback_generation: Arc<Mutex<u64>>,
    pending_feedback: Arc<Mutex<Option<PendingSnapshotFeedback>>>,
    readback_slots: Vec<SnapshotReadbackSlot>,
    next_readback_slot: usize,
}

impl VirtualTerrainGpuControl {
    pub(crate) fn new(
        device: &Device,
        capacity: VirtualTerrainCapacity,
    ) -> Result<Self, VirtualTerrainGpuError> {
        let maximum_storage = u64::from(device.limits().max_storage_buffer_binding_size);
        let candidate_bytes = buffer_bytes::<GpuCandidatePage>(capacity.max_selected_pages)?;
        let candidate_token_bytes = buffer_bytes::<u32>(capacity.max_selected_pages)?;
        let readback_bytes = GPU_SNAPSHOT_READBACK_PREFIX_BYTES
            .checked_add(candidate_bytes)
            .ok_or(VirtualTerrainGpuError::DeviceLimit)?;
        if VIRTUAL_TERRAIN_HANDLE_BANK_BYTES > maximum_storage
            || candidate_bytes > maximum_storage
            || candidate_token_bytes > maximum_storage
            || readback_bytes > device.limits().max_buffer_size
            || device.limits().max_storage_buffers_per_shader_stage < 4
        {
            return Err(VirtualTerrainGpuError::DeviceLimit);
        }
        let candidate_pages = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bounded virtual terrain CPU-selected candidate pages"),
            size: candidate_bytes,
            usage: candidate_page_buffer_usage(),
            mapped_at_creation: false,
        });
        let candidate_page_tokens = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bounded virtual terrain GPU page validation tokens"),
            size: candidate_token_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let placeholder = || {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("empty virtual terrain geometry segment"),
                size: GPU_GEOMETRY_ELEMENT_BYTES,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            })
        };
        let source_buffers = [placeholder(), placeholder()];
        let render_layout = create_render_layout(device);
        let encode_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("virtual terrain snapshot encoding layout"),
            entries: &[
                storage_entry(0, true, wgpu::ShaderStages::COMPUTE),
                storage_entry(1, false, wgpu::ShaderStages::COMPUTE),
                storage_entry(2, false, wgpu::ShaderStages::COMPUTE),
                storage_entry(3, false, wgpu::ShaderStages::COMPUTE),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("virtual terrain snapshot encoding pipeline layout"),
            bind_group_layouts: &[Some(&encode_layout)],
            immediate_size: 0,
        });
        let shader =
            device.create_shader_module(wgpu::include_wgsl!("shaders/virtual_terrain.wgsl"));
        let structural_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("validate virtual terrain candidate descriptor structure"),
                layout: Some(&pipeline_layout),
                module: &shader,
                entry_point: Some("validate_candidate_structure"),
                compilation_options: Default::default(),
                cache: None,
            });
        let encode_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("encode CPU-selected virtual terrain handles"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("encode_snapshot"),
            compilation_options: Default::default(),
            cache: None,
        });
        let validate_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("validate encoded virtual terrain handles"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("validate_snapshot"),
            compilation_options: Default::default(),
            cache: None,
        });
        let make_bank = |index| {
            let handles = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(if index == 0 {
                    "virtual terrain handle bank A"
                } else {
                    "virtual terrain handle bank B"
                }),
                size: VIRTUAL_TERRAIN_HANDLE_BANK_BYTES,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            });
            let counters = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("virtual terrain snapshot counters"),
                contents: bytemuck::bytes_of(&GpuSnapshotCounters::default()),
                usage: snapshot_counter_buffer_usage(),
            });
            let indirect = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("virtual terrain snapshot indirect commands"),
                size: 64,
                usage: snapshot_indirect_buffer_usage(),
                mapped_at_creation: false,
            });
            let encode_bind_group = create_encode_bind_group(
                device,
                &encode_layout,
                &candidate_pages,
                &handles,
                &counters,
                &candidate_page_tokens,
            );
            let render_bind_group =
                create_render_bind_group(device, &render_layout, &handles, &source_buffers);
            SnapshotBank {
                handles,
                counters,
                indirect,
                encode_bind_group,
                render_bind_group,
                metadata: None,
            }
        };
        let banks = [make_bank(0), make_bank(1)];
        let readback_slots = (0..GPU_SNAPSHOT_READBACK_SLOTS)
            .map(|_| SnapshotReadbackSlot {
                buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("virtual terrain snapshot feedback readback"),
                    size: readback_bytes,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
                available: Arc::new(AtomicBool::new(true)),
            })
            .collect();
        Ok(Self {
            capacity,
            geometries: BTreeMap::new(),
            candidate_pages,
            candidate_page_tokens,
            source_buffers,
            source_element_capacities: [1, 1],
            bound_source_count: 0,
            render_layout,
            structural_pipeline,
            encode_pipeline,
            validate_pipeline,
            banks,
            active_bank: 0,
            pending_bank: None,
            active_geometry_dirty: false,
            next_generation: 1,
            latest_raw_feedback: Arc::new(Mutex::new(None)),
            minimum_feedback_generation: Arc::new(Mutex::new(1)),
            pending_feedback: Arc::new(Mutex::new(None)),
            readback_slots,
            next_readback_slot: 0,
        })
    }

    pub(crate) const fn render_layout(&self) -> &wgpu::BindGroupLayout {
        &self.render_layout
    }

    pub(crate) fn active_render_bind_group(&self) -> &wgpu::BindGroup {
        &self.banks[self.active_bank].render_bind_group
    }

    pub(crate) fn active_indirect_buffer(&self) -> &Buffer {
        &self.banks[self.active_bank].indirect
    }

    pub(crate) fn active_generation(&self) -> Option<u64> {
        self.banks[self.active_bank]
            .metadata
            .as_ref()
            .map(|metadata| metadata.generation)
    }

    pub(crate) fn active_snapshot_identity(&self) -> Option<(u64, u64)> {
        self.banks[self.active_bank]
            .metadata
            .as_ref()
            .map(|metadata| (metadata.generation, metadata.fingerprint))
    }

    pub(crate) fn bind_geometry_sources(
        &mut self,
        device: &Device,
        sources: &[Buffer],
    ) -> Result<(), VirtualTerrainGpuError> {
        if sources.len() > 2 {
            return Err(VirtualTerrainGpuError::DeviceLimit);
        }
        for source in sources {
            if source.size() > u64::from(device.limits().max_storage_buffer_binding_size)
                || !source.usage().contains(wgpu::BufferUsages::STORAGE)
                || !source.size().is_multiple_of(GPU_GEOMETRY_ELEMENT_BYTES)
            {
                return Err(VirtualTerrainGpuError::DeviceLimit);
            }
        }
        for (index, source) in sources.iter().enumerate() {
            self.source_buffers[index] = source.clone();
            self.source_element_capacities[index] =
                u32::try_from(source.size() / GPU_GEOMETRY_ELEMENT_BYTES)
                    .map_err(|_| VirtualTerrainGpuError::DeviceLimit)?;
        }
        for index in sources.len()..2 {
            self.source_element_capacities[index] = 1;
        }
        self.bound_source_count = sources.len();
        for bank in &mut self.banks {
            bank.render_bind_group = create_render_bind_group(
                device,
                &self.render_layout,
                &bank.handles,
                &self.source_buffers,
            );
        }
        Ok(())
    }

    pub(crate) const fn bound_geometry_source_count(&self) -> usize {
        self.bound_source_count
    }

    pub(crate) fn update_page_geometry(
        &mut self,
        key: TerrainPageKey,
        geometry: VirtualTerrainGpuGeometry,
    ) -> Result<(), VirtualTerrainGpuError> {
        self.validate_geometry(geometry)?;
        self.replace_page_geometry(key, Some(geometry));
        Ok(())
    }

    pub(crate) fn remove_page_geometry(&mut self, key: TerrainPageKey) {
        self.replace_page_geometry(key, None);
    }

    fn replace_page_geometry(
        &mut self,
        key: TerrainPageKey,
        replacement: Option<VirtualTerrainGpuGeometry>,
    ) {
        if !geometry_directory_entry_changes(&self.geometries, key, replacement) {
            return;
        }

        let (active_changes, pending_changes) = geometry_mutation_impact(
            self.banks[self.active_bank].metadata.as_ref(),
            self.pending_bank
                .and_then(|bank| self.banks[bank].metadata.as_ref()),
            key,
        );
        if active_changes {
            // The active handles remain safe to present because the renderer retains their
            // allocation until another bank publishes. They are no longer a current encoding of
            // the geometry directory, however, so a matching logical cut must be re-encoded.
            self.active_geometry_dirty = true;
        }
        if pending_changes {
            // A candidate bank contains absolute handles. Invalidate it before its allocation may
            // be released or reused; stale feedback can then neither certify nor publish it.
            self.discard_pending_candidate();
        }

        replace_geometry_directory_entry(&mut self.geometries, key, replacement);
    }

    fn validate_geometry(
        &self,
        geometry: VirtualTerrainGpuGeometry,
    ) -> Result<(), VirtualTerrainGpuError> {
        for range in geometry.ranges() {
            self.pack_range(range)?;
        }
        Ok(())
    }

    fn pack_range(
        &self,
        range: VirtualTerrainGpuGeometryRange,
    ) -> Result<[u32; 2], VirtualTerrainGpuError> {
        if range.element_count == 0 {
            return Ok([0, 0]);
        }
        let segment = usize::try_from(range.source_segment)
            .ok()
            .filter(|segment| *segment < 2)
            .ok_or(VirtualTerrainGpuError::InvalidGeometry)?;
        if !range
            .source_offset_bytes
            .is_multiple_of(GPU_GEOMETRY_ELEMENT_BYTES)
        {
            return Err(VirtualTerrainGpuError::InvalidGeometry);
        }
        let first = u32::try_from(range.source_offset_bytes / GPU_GEOMETRY_ELEMENT_BYTES)
            .map_err(|_| VirtualTerrainGpuError::GeometryCapacity)?;
        let end = first
            .checked_add(range.element_count)
            .ok_or(VirtualTerrainGpuError::GeometryCapacity)?;
        if end > self.source_element_capacities[segment] || first > GPU_HANDLE_ELEMENT_MASK {
            return Err(VirtualTerrainGpuError::GeometryCapacity);
        }
        let segment_bit = if segment == 0 {
            0
        } else {
            GPU_HANDLE_SEGMENT_BIT
        };
        Ok([segment_bit | first, range.element_count])
    }

    pub(crate) fn candidate_work(
        &self,
        identity: VirtualTerrainSnapshotIdentity<'_>,
    ) -> Option<VirtualTerrainCandidateWork> {
        let active_matches =
            snapshot_metadata_matches(self.banks[self.active_bank].metadata.as_ref(), identity);
        let pending = self.pending_bank.and_then(|bank| {
            self.banks[bank]
                .metadata
                .as_ref()
                .filter(|metadata| snapshot_metadata_matches(Some(metadata), identity))
        });
        let pending_state = pending.and_then(|metadata| self.pending_state(metadata.generation));
        let pending_feedback_is_current = pending.is_some_and(|metadata| {
            self.latest_raw_feedback.lock().is_ok_and(|feedback| {
                feedback.as_ref().is_some_and(|feedback| {
                    join_u64(feedback.counters.generation) == metadata.generation
                })
            })
        });
        snapshot_candidate_work(
            self.active_geometry_dirty,
            active_matches,
            pending.is_some(),
            pending_state,
            pending_feedback_is_current,
        )
    }

    pub(crate) fn active_snapshot_matches(
        &self,
        identity: VirtualTerrainSnapshotIdentity<'_>,
    ) -> bool {
        !self.active_geometry_dirty
            && snapshot_metadata_matches(self.banks[self.active_bank].metadata.as_ref(), identity)
    }

    /// Whether the immutable active bank still represents the already-published cut.
    ///
    /// Candidate geometry may have changed under the same logical key/fingerprint while the old
    /// source allocation remains retired and immutable for presentation. In that case the active
    /// bank is still safe to draw, but `active_snapshot_matches` correctly requires a new candidate
    /// encoding before it can be treated as current.
    pub(crate) fn presented_snapshot_matches(
        &self,
        identity: VirtualTerrainSnapshotIdentity<'_>,
    ) -> bool {
        snapshot_metadata_matches(self.banks[self.active_bank].metadata.as_ref(), identity)
    }

    pub(crate) fn encode_candidate(
        &mut self,
        queue: &Queue,
        encoder: &mut CommandEncoder,
        identity: VirtualTerrainSnapshotIdentity<'_>,
        timestamps: Option<VirtualTerrainGpuTimestampWrites<'_>>,
    ) -> Result<VirtualTerrainCandidateEncodeOutcome, VirtualTerrainGpuError> {
        if identity.selected_pages.len() > self.capacity.max_selected_pages
            || identity
                .selected_pages
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
        {
            return Err(VirtualTerrainGpuError::GeometryCapacity);
        }
        if let Some(bank) = self.pending_bank {
            let matching_generation = self.banks[bank].metadata.as_ref().and_then(|metadata| {
                snapshot_metadata_matches(Some(metadata), identity).then_some(metadata.generation)
            });
            if let Some(generation) = matching_generation
                && self
                    .pending_state(generation)
                    .is_some_and(|state| state != SnapshotPendingState::Recorded)
            {
                return Ok(VirtualTerrainCandidateEncodeOutcome::ReadbackOnly(
                    generation,
                ));
            }
        }
        let inactive = 1 - self.active_bank;
        let mut pages = Vec::with_capacity(identity.selected_pages.len());
        let mut expected_counts = [0u32; STREAM_COUNT];
        for key in identity.selected_pages {
            let geometry = self
                .geometries
                .get(key)
                .copied()
                .ok_or(VirtualTerrainGpuError::UnknownPage(*key))?;
            let geometry_ranges = geometry.ranges();
            let ranges = [
                self.pack_range(geometry_ranges[0])?,
                self.pack_range(geometry_ranges[1])?,
                self.pack_range(geometry_ranges[2])?,
                self.pack_range(geometry_ranges[3])?,
            ];
            pages.push(assign_candidate_page_destinations(
                ranges,
                &mut expected_counts,
                VIRTUAL_TERRAIN_HANDLE_CAPACITIES,
            )?);
        }
        if !pages.is_empty() {
            queue.write_buffer(&self.candidate_pages, 0, bytemuck::cast_slice(&pages));
        }
        let expected_ranges = pages.iter().map(|page| page.ranges).collect();
        let generation = self.next_generation;
        self.next_generation = self.next_generation.wrapping_add(1).max(1);
        let counters = GpuSnapshotCounters {
            element_counts: expected_counts,
            generation: split_u64(generation),
            fingerprint: split_u64(identity.fingerprint),
            selected_count: identity.selected_pages.len() as u32,
            ownerless_roots: identity.ownerless_roots,
            source_element_capacities: self.source_element_capacities,
            ..GpuSnapshotCounters::default()
        };
        queue.write_buffer(
            &self.banks[inactive].counters,
            0,
            bytemuck::bytes_of(&counters),
        );
        let indirect_commands = expected_indirect_commands(expected_counts);
        queue.write_buffer(
            &self.banks[inactive].indirect,
            0,
            bytemuck::bytes_of(&indirect_commands),
        );
        self.banks[inactive].metadata = Some(SnapshotMetadata {
            generation,
            fingerprint: identity.fingerprint,
            selected_pages: identity.selected_pages.to_vec(),
            ownerless_roots: identity.ownerless_roots,
            expected_counts,
            expected_ranges,
        });
        self.pending_bank = Some(inactive);
        if let Ok(mut pending) = self.pending_feedback.lock() {
            *pending = Some(PendingSnapshotFeedback {
                generation,
                state: SnapshotPendingState::Recorded,
            });
        }
        if let Ok(mut minimum) = self.minimum_feedback_generation.lock() {
            *minimum = generation;
        }
        if !pages.is_empty() {
            encoder.clear_buffer(
                &self.candidate_page_tokens,
                0,
                Some(pages.len() as u64 * size_of::<u32>() as u64),
            );
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("validate virtual terrain candidate descriptor structure"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.structural_pipeline);
            pass.set_bind_group(0, &self.banks[inactive].encode_bind_group, &[]);
            if !pages.is_empty() {
                pass.dispatch_workgroups(pages.len() as u32, 1, 1);
            }
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("encode CPU-selected virtual terrain snapshot"),
                timestamp_writes: timestamps.map(|timestamps| wgpu::ComputePassTimestampWrites {
                    query_set: timestamps.query_set,
                    beginning_of_pass_write_index: Some(timestamps.encoding_first_query),
                    end_of_pass_write_index: Some(timestamps.encoding_first_query + 1),
                }),
            });
            pass.set_pipeline(&self.encode_pipeline);
            pass.set_bind_group(0, &self.banks[inactive].encode_bind_group, &[]);
            if !pages.is_empty() {
                pass.dispatch_workgroups(pages.len() as u32, 1, 1);
            }
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("validate CPU-selected virtual terrain snapshot"),
                timestamp_writes: timestamps.map(|timestamps| wgpu::ComputePassTimestampWrites {
                    query_set: timestamps.query_set,
                    beginning_of_pass_write_index: Some(timestamps.finalize_first_query),
                    end_of_pass_write_index: Some(timestamps.finalize_first_query + 1),
                }),
            });
            pass.set_bind_group(0, &self.banks[inactive].encode_bind_group, &[]);
            pass.set_pipeline(&self.validate_pipeline);
            if !pages.is_empty() {
                pass.dispatch_workgroups(pages.len() as u32, 1, 1);
            }
        }
        Ok(VirtualTerrainCandidateEncodeOutcome::Encoded(generation))
    }

    fn pending_state(&self, generation: u64) -> Option<SnapshotPendingState> {
        self.pending_feedback.lock().ok().and_then(|pending| {
            pending
                .filter(|pending| pending.generation == generation)
                .map(|pending| pending.state)
        })
    }

    /// Marks the recorded candidate as belonging to the command buffer that is now guaranteed to
    /// submit, and schedules its bounded feedback readback when a slot is available.
    ///
    /// Calling this only in the renderer's final no-failure region is what distinguishes an
    /// abandoned recording from a submitted snapshot. An abandoned `Recorded` generation is
    /// encoded again rather than reading counters that no GPU command ever produced.
    pub(crate) fn submit_pending_readback(
        &mut self,
        encoder: &mut CommandEncoder,
        generation: u64,
    ) {
        let Some(bank) = self.pending_bank.filter(|bank| {
            self.banks[*bank]
                .metadata
                .as_ref()
                .is_some_and(|metadata| metadata.generation == generation)
        }) else {
            return;
        };
        let Some(descriptor_count) = self.banks[bank]
            .metadata
            .as_ref()
            .map(|metadata| metadata.expected_ranges.len())
        else {
            return;
        };
        let Some(descriptor_bytes) = descriptor_count.checked_mul(size_of::<GpuCandidatePage>())
        else {
            return;
        };
        let should_schedule = self.pending_feedback.lock().is_ok_and(|mut pending| {
            let Some(current) = pending
                .as_mut()
                .filter(|pending| pending.generation == generation)
            else {
                return false;
            };
            match current.state {
                SnapshotPendingState::Recorded => {
                    current.state = SnapshotPendingState::SubmittedAwaitingReadback;
                    true
                }
                SnapshotPendingState::SubmittedAwaitingReadback | SnapshotPendingState::Failed => {
                    true
                }
                SnapshotPendingState::InFlight | SnapshotPendingState::Succeeded => false,
            }
        });
        if !should_schedule {
            return;
        }
        let Some(slot_index) = claim_readback_slot(
            self.readback_slots.len(),
            self.next_readback_slot,
            |index| {
                let slot = self.readback_slots.get(index)?;
                slot.available
                    .compare_exchange(true, false, Ordering::AcqRel, Ordering::Acquire)
                    .ok()
                    .map(|_| index)
            },
        ) else {
            return;
        };
        let slot = &self.readback_slots[slot_index];
        let marked_in_flight = self.pending_feedback.lock().is_ok_and(|mut pending| {
            pending
                .as_mut()
                .filter(|pending| pending.generation == generation)
                .is_some_and(|pending| {
                    pending.state = SnapshotPendingState::InFlight;
                    true
                })
        });
        if !marked_in_flight {
            slot.available.store(true, Ordering::Release);
            return;
        }
        self.next_readback_slot = (slot_index + 1) % self.readback_slots.len();
        encoder.copy_buffer_to_buffer(
            &self.banks[bank].counters,
            0,
            &slot.buffer,
            0,
            size_of::<GpuSnapshotCounters>() as u64,
        );
        encoder.copy_buffer_to_buffer(
            &self.banks[bank].indirect,
            0,
            &slot.buffer,
            size_of::<GpuSnapshotCounters>() as u64,
            GPU_SNAPSHOT_INDIRECT_BYTES,
        );
        if descriptor_bytes > 0 {
            encoder.copy_buffer_to_buffer(
                &self.candidate_pages,
                0,
                &slot.buffer,
                GPU_SNAPSHOT_READBACK_PREFIX_BYTES,
                descriptor_bytes as u64,
            );
        }
        let callback_buffer = slot.buffer.clone();
        let available = Arc::clone(&slot.available);
        let feedback = Arc::clone(&self.latest_raw_feedback);
        let minimum = Arc::clone(&self.minimum_feedback_generation);
        let pending = Arc::clone(&self.pending_feedback);
        encoder.map_buffer_on_submit(&slot.buffer, wgpu::MapMode::Read, .., move |result| {
            let parsed = result.is_ok().then(|| -> Option<GpuSnapshotReadback> {
                let mapped = callback_buffer.get_mapped_range(..).ok()?;
                let counters = bytemuck::try_from_bytes::<GpuSnapshotCounters>(
                    mapped.get(..size_of::<GpuSnapshotCounters>())?,
                )
                .ok()
                .copied()?;
                let indirect_start = size_of::<GpuSnapshotCounters>();
                let indirect_end = indirect_start + GPU_SNAPSHOT_INDIRECT_BYTES as usize;
                let indirect_commands =
                    bytemuck::try_cast_slice::<u8, u32>(mapped.get(indirect_start..indirect_end)?)
                        .ok()?
                        .try_into()
                        .ok()?;
                let descriptor_end = GPU_SNAPSHOT_READBACK_PREFIX_BYTES as usize + descriptor_bytes;
                let candidate_pages = bytemuck::try_cast_slice::<u8, GpuCandidatePage>(
                    mapped.get(GPU_SNAPSHOT_READBACK_PREFIX_BYTES as usize..descriptor_end)?,
                )
                .ok()?
                .to_vec();
                Some(GpuSnapshotReadback {
                    counters,
                    indirect_commands,
                    candidate_pages,
                })
            });
            let parsed = parsed.flatten();
            let succeeded = parsed.is_some_and(|parsed| {
                if join_u64(parsed.counters.generation) != generation
                    || !minimum.lock().is_ok_and(|minimum| generation >= *minimum)
                {
                    return false;
                }
                feedback.lock().is_ok_and(|mut destination| {
                    let is_newer = destination
                        .as_ref()
                        .is_none_or(|current| generation > join_u64(current.counters.generation));
                    if is_newer {
                        *destination = Some(parsed);
                    }
                    is_newer
                })
            });
            callback_buffer.unmap();
            record_readback_completion(&pending, generation, succeeded);
            available.store(true, Ordering::Release);
        });
    }

    fn feedback_metadata(&self, readback: &GpuSnapshotReadback) -> Option<&SnapshotMetadata> {
        let generation = join_u64(readback.counters.generation);
        self.banks
            .iter()
            .filter_map(|bank| bank.metadata.as_ref())
            .find(|metadata| metadata.generation == generation)
    }

    pub(crate) fn latest_feedback(&self) -> Option<GpuVirtualTerrainFeedback> {
        let raw = self.latest_raw_feedback.lock().ok()?;
        let raw = raw.as_ref()?;
        let metadata = self.feedback_metadata(raw)?;
        let counters = raw.counters;
        Some(GpuVirtualTerrainFeedback {
            submission_id: metadata.generation,
            oracle_fingerprint: join_u64(counters.fingerprint),
            selected_pages: metadata.selected_pages.clone(),
            ownerless_roots: counters.ownerless_roots,
            encoded_surface_handles: counters.element_counts[STREAM_SURFACE],
            encoded_triangle_handles: counters.element_counts[STREAM_TRIANGLE],
            encoded_water_surface_handles: counters.element_counts[STREAM_WATER_SURFACE],
            encoded_water_triangle_handles: counters.element_counts[STREAM_WATER_TRIANGLE],
            encoded_pages: counters.encoded_pages,
            encoding_overflow_flags: counters.overflow_flags
                | snapshot_validation_failure_flags(raw, metadata),
        })
    }

    pub(crate) fn candidate_is_certified(
        &self,
        identity: VirtualTerrainSnapshotIdentity<'_>,
    ) -> bool {
        let Some(bank) = self.pending_bank else {
            return false;
        };
        let Some(metadata) = self.banks[bank].metadata.as_ref() else {
            return false;
        };
        let Ok(feedback) = self.latest_raw_feedback.lock() else {
            return false;
        };
        let Some(feedback) = feedback.as_ref() else {
            return false;
        };
        let counters = feedback.counters;
        snapshot_metadata_matches(Some(metadata), identity)
            && snapshot_counter_evidence_matches(counters, metadata)
            && snapshot_validation_failure_flags(&feedback, metadata) == 0
    }

    pub(crate) fn promote_certified_candidate(
        &mut self,
        identity: VirtualTerrainSnapshotIdentity<'_>,
    ) -> Result<u64, VirtualTerrainGpuError> {
        if !self.candidate_is_certified(identity) {
            return Err(VirtualTerrainGpuError::CandidateNotCertified);
        }
        let bank = self
            .pending_bank
            .take()
            .ok_or(VirtualTerrainGpuError::CandidateNotCertified)?;
        self.active_bank = bank;
        self.active_geometry_dirty = false;
        if let Ok(mut pending) = self.pending_feedback.lock() {
            *pending = None;
        }
        self.active_generation()
            .ok_or(VirtualTerrainGpuError::CandidateNotCertified)
    }

    pub(crate) fn invalidate_candidate(&mut self) {
        self.active_geometry_dirty = true;
        self.discard_pending_candidate();
    }

    fn discard_pending_candidate(&mut self) {
        if let Some(bank) = self.pending_bank.take() {
            self.banks[bank].metadata = None;
        }
        if let Ok(mut pending) = self.pending_feedback.lock() {
            *pending = None;
        }
        let generation = self.next_generation;
        self.next_generation = self.next_generation.wrapping_add(1).max(1);
        if let Ok(mut minimum) = self.minimum_feedback_generation.lock() {
            *minimum = generation;
        }
        if let Ok(mut feedback) = self.latest_raw_feedback.lock() {
            *feedback = None;
        }
    }

    pub(crate) const fn handle_bank_capacity_bytes(&self) -> u64 {
        VIRTUAL_TERRAIN_HANDLE_BANK_BYTES * 2
    }

    pub(crate) fn allocated_handle_bytes(&self) -> u64 {
        self.banks
            .iter()
            .filter_map(|bank| bank.metadata.as_ref())
            .map(|metadata| {
                metadata
                    .expected_counts
                    .into_iter()
                    .map(u64::from)
                    .sum::<u64>()
                    * 4
            })
            .sum()
    }
}

fn snapshot_metadata_matches(
    metadata: Option<&SnapshotMetadata>,
    identity: VirtualTerrainSnapshotIdentity<'_>,
) -> bool {
    metadata.is_some_and(|metadata| {
        metadata.fingerprint == identity.fingerprint
            && metadata.selected_pages == identity.selected_pages
            && metadata.ownerless_roots == identity.ownerless_roots
    })
}

fn assign_candidate_page_destinations(
    ranges: [[u32; 2]; STREAM_COUNT],
    stream_prefixes: &mut [u32; STREAM_COUNT],
    stream_capacities: [u32; STREAM_COUNT],
) -> Result<GpuCandidatePage, VirtualTerrainGpuError> {
    let destinations = *stream_prefixes;
    let mut next_prefixes = destinations;
    for stream in 0..STREAM_COUNT {
        next_prefixes[stream] = next_prefixes[stream]
            .checked_add(ranges[stream][1])
            .filter(|end| *end <= stream_capacities[stream])
            .ok_or(VirtualTerrainGpuError::GeometryCapacity)?;
    }
    *stream_prefixes = next_prefixes;
    Ok(GpuCandidatePage {
        ranges,
        destinations,
    })
}

fn expected_indirect_commands(counts: [u32; STREAM_COUNT]) -> [u32; GPU_SNAPSHOT_INDIRECT_WORDS] {
    [
        4, counts[0], 0, 0, counts[1], 1, 0, 0, 4, counts[2], 0, 0, counts[3], 1, 0, 0,
    ]
}

fn candidate_descriptors_match_canonical(
    candidate_pages: &[GpuCandidatePage],
    expected_ranges: &[[[u32; 2]; STREAM_COUNT]],
    expected_counts: [u32; STREAM_COUNT],
) -> bool {
    if candidate_pages.len() != expected_ranges.len() {
        return false;
    }
    let mut prefixes = [0; STREAM_COUNT];
    for (candidate, ranges) in candidate_pages.iter().zip(expected_ranges) {
        let Ok(expected) = assign_candidate_page_destinations(
            *ranges,
            &mut prefixes,
            VIRTUAL_TERRAIN_HANDLE_CAPACITIES,
        ) else {
            return false;
        };
        if candidate != &expected {
            return false;
        }
    }
    prefixes == expected_counts
}

fn snapshot_counter_evidence_matches(
    counters: GpuSnapshotCounters,
    metadata: &SnapshotMetadata,
) -> bool {
    join_u64(counters.generation) == metadata.generation
        && join_u64(counters.fingerprint) == metadata.fingerprint
        && counters.selected_count == metadata.selected_pages.len() as u32
        && counters.ownerless_roots == metadata.ownerless_roots
        && counters.encoded_pages == metadata.selected_pages.len() as u32
        && counters.element_counts == metadata.expected_counts
        && counters.overflow_flags == 0
}

fn snapshot_validation_failure_flags(
    readback: &GpuSnapshotReadback,
    metadata: &SnapshotMetadata,
) -> u32 {
    u32::from(!candidate_descriptors_match_canonical(
        &readback.candidate_pages,
        &metadata.expected_ranges,
        metadata.expected_counts,
    )) * VALIDATION_DESCRIPTOR_MISMATCH
        | u32::from(
            readback.indirect_commands != expected_indirect_commands(metadata.expected_counts),
        ) * VALIDATION_INDIRECT_MISMATCH
}

fn snapshot_counter_buffer_usage() -> wgpu::BufferUsages {
    wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC
}

fn candidate_page_buffer_usage() -> wgpu::BufferUsages {
    wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC
}

fn snapshot_indirect_buffer_usage() -> wgpu::BufferUsages {
    wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC
}

#[cfg(test)]
fn exact_candidate_handles_match(
    pages: &[GpuCandidatePage],
    stream_handles: &[Vec<u32>; STREAM_COUNT],
) -> bool {
    pages.iter().all(|page| {
        (0..STREAM_COUNT).all(|stream| {
            let [first, count] = page.ranges[stream];
            let Some(end) = page.destinations[stream].checked_add(count) else {
                return false;
            };
            let Ok(destination) = usize::try_from(page.destinations[stream]) else {
                return false;
            };
            let Ok(end) = usize::try_from(end) else {
                return false;
            };
            let Some(actual) = stream_handles[stream].get(destination..end) else {
                return false;
            };
            actual.iter().enumerate().all(|(index, actual)| {
                u32::try_from(index)
                    .ok()
                    .and_then(|index| first.checked_add(index))
                    == Some(*actual)
            })
        })
    })
}

fn geometry_directory_entry_changes(
    geometries: &BTreeMap<TerrainPageKey, VirtualTerrainGpuGeometry>,
    key: TerrainPageKey,
    replacement: Option<VirtualTerrainGpuGeometry>,
) -> bool {
    geometries.get(&key).copied() != replacement
}

fn replace_geometry_directory_entry(
    geometries: &mut BTreeMap<TerrainPageKey, VirtualTerrainGpuGeometry>,
    key: TerrainPageKey,
    replacement: Option<VirtualTerrainGpuGeometry>,
) {
    match replacement {
        Some(geometry) => {
            // Default geometry is a valid resident empty page. It must remain in the directory so
            // the GPU certifies the page owner with zero handles instead of reporting UnknownPage.
            geometries.insert(key, geometry);
        }
        None => {
            geometries.remove(&key);
        }
    }
}

fn geometry_mutation_impact(
    active: Option<&SnapshotMetadata>,
    pending: Option<&SnapshotMetadata>,
    key: TerrainPageKey,
) -> (bool, bool) {
    let selects = |metadata: Option<&SnapshotMetadata>| {
        metadata.is_some_and(|metadata| metadata.selected_pages.binary_search(&key).is_ok())
    };
    (selects(active), selects(pending))
}

const fn snapshot_candidate_work(
    active_geometry_dirty: bool,
    active_matches: bool,
    pending_matches: bool,
    pending_state: Option<SnapshotPendingState>,
    pending_feedback_is_current: bool,
) -> Option<VirtualTerrainCandidateWork> {
    if !active_geometry_dirty && active_matches {
        return None;
    }
    if !pending_matches {
        return Some(VirtualTerrainCandidateWork::Encode);
    }
    match pending_state {
        Some(SnapshotPendingState::InFlight) => None,
        Some(SnapshotPendingState::Succeeded) if pending_feedback_is_current => None,
        Some(
            SnapshotPendingState::SubmittedAwaitingReadback
            | SnapshotPendingState::Succeeded
            | SnapshotPendingState::Failed,
        ) => Some(VirtualTerrainCandidateWork::ReadbackOnly),
        Some(SnapshotPendingState::Recorded) | None => Some(VirtualTerrainCandidateWork::Encode),
    }
}

fn claim_readback_slot(
    slot_count: usize,
    next_slot: usize,
    mut try_claim: impl FnMut(usize) -> Option<usize>,
) -> Option<usize> {
    (0..slot_count)
        .map(|offset| (next_slot + offset) % slot_count)
        .find_map(&mut try_claim)
}

fn record_readback_completion(
    pending: &Mutex<Option<PendingSnapshotFeedback>>,
    generation: u64,
    succeeded: bool,
) {
    if let Ok(mut pending) = pending.lock()
        && let Some(current) = pending
            .as_mut()
            .filter(|pending| pending.generation == generation)
    {
        current.state = if succeeded {
            SnapshotPendingState::Succeeded
        } else {
            SnapshotPendingState::Failed
        };
    }
}

fn create_render_layout(device: &Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("virtual terrain immutable geometry handle layout"),
        entries: &[
            storage_entry(0, true, wgpu::ShaderStages::VERTEX),
            storage_entry(1, true, wgpu::ShaderStages::VERTEX),
            storage_entry(2, true, wgpu::ShaderStages::VERTEX),
        ],
    })
}

fn create_render_bind_group(
    device: &Device,
    layout: &wgpu::BindGroupLayout,
    handles: &Buffer,
    sources: &[Buffer; 2],
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("virtual terrain immutable geometry handles"),
        layout,
        entries: &[
            entire_entry(0, handles),
            entire_entry(1, &sources[0]),
            entire_entry(2, &sources[1]),
        ],
    })
}

fn create_encode_bind_group(
    device: &Device,
    layout: &wgpu::BindGroupLayout,
    candidates: &Buffer,
    handles: &Buffer,
    counters: &Buffer,
    page_tokens: &Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("virtual terrain inactive snapshot encoder"),
        layout,
        entries: &[
            entire_entry(0, candidates),
            entire_entry(1, handles),
            entire_entry(2, counters),
            entire_entry(3, page_tokens),
        ],
    })
}

fn storage_entry(
    binding: u32,
    read_only: bool,
    visibility: wgpu::ShaderStages,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn entire_entry(binding: u32, buffer: &Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}

fn buffer_bytes<T>(count: usize) -> Result<u64, VirtualTerrainGpuError> {
    count
        .checked_mul(size_of::<T>())
        .and_then(|bytes| u64::try_from(bytes.max(size_of::<T>())).ok())
        .ok_or(VirtualTerrainGpuError::DeviceLimit)
}

const fn split_u64(value: u64) -> [u32; 2] {
    [value as u32, (value >> 32) as u32]
}

const fn join_u64(value: [u32; 2]) -> u64 {
    value[0] as u64 | ((value[1] as u64) << 32)
}

#[cfg(test)]
const fn handle_stream_byte_range(stream: usize) -> Range<u64> {
    let start = VIRTUAL_TERRAIN_HANDLE_OFFSETS[stream] as u64 * 4;
    start..start + VIRTUAL_TERRAIN_HANDLE_CAPACITIES[stream] as u64 * 4
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block_on_native<F: std::future::Future>(future: F) -> F::Output {
        struct ThreadWake(std::thread::Thread);

        impl std::task::Wake for ThreadWake {
            fn wake(self: Arc<Self>) {
                self.0.unpark();
            }

            fn wake_by_ref(self: &Arc<Self>) {
                self.0.unpark();
            }
        }

        let waker = std::task::Waker::from(Arc::new(ThreadWake(std::thread::current())));
        let mut context = std::task::Context::from_waker(&waker);
        let mut future = std::pin::pin!(future);
        loop {
            match future.as_mut().poll(&mut context) {
                std::task::Poll::Ready(output) => return output,
                std::task::Poll::Pending => std::thread::park(),
            }
        }
    }

    fn metadata(
        generation: u64,
        fingerprint: u64,
        selected_pages: Vec<TerrainPageKey>,
        ownerless_roots: u32,
        expected_counts: [u32; STREAM_COUNT],
    ) -> SnapshotMetadata {
        SnapshotMetadata {
            generation,
            fingerprint,
            selected_pages,
            ownerless_roots,
            expected_counts,
            expected_ranges: Vec::new(),
        }
    }

    #[test]
    fn two_handle_banks_replace_the_old_geometry_copy_without_more_memory() {
        let old_copied_bytes = VIRTUAL_TERRAIN_SURFACE_HANDLE_SOURCE_BYTES
            + VIRTUAL_TERRAIN_TRIANGLE_HANDLE_SOURCE_BYTES
            + VIRTUAL_TERRAIN_WATER_SURFACE_HANDLE_SOURCE_BYTES
            + VIRTUAL_TERRAIN_WATER_TRIANGLE_HANDLE_SOURCE_BYTES;
        assert_eq!(old_copied_bytes, 192 * 1_024 * 1_024);
        assert!(VIRTUAL_TERRAIN_HANDLE_BANK_BYTES < 33 * 1_024 * 1_024);
        assert!(VIRTUAL_TERRAIN_HANDLE_BANK_BYTES * 2 < old_copied_bytes);
    }

    #[test]
    fn handle_stream_partitions_are_bounded_and_disjoint() {
        for stream in 0..STREAM_COUNT {
            let range = handle_stream_byte_range(stream);
            assert!(range.end <= VIRTUAL_TERRAIN_HANDLE_BANK_BYTES);
            if stream > 0 {
                assert_eq!(handle_stream_byte_range(stream - 1).end, range.start);
            }
        }
        assert_eq!(
            handle_stream_byte_range(STREAM_COUNT - 1).end,
            VIRTUAL_TERRAIN_HANDLE_BANK_BYTES
        );
        assert_eq!(VIRTUAL_TERRAIN_SURFACE_INDIRECT_OFFSET, 0);
        assert_eq!(VIRTUAL_TERRAIN_WATER_TRIANGLE_INDIRECT_OFFSET, 48);
    }

    #[test]
    fn packed_handle_addresses_one_of_two_24_byte_segments() {
        let first = 123_456u32;
        let segment_zero = first;
        let segment_one = GPU_HANDLE_SEGMENT_BIT | first;
        assert_eq!(segment_zero >> 31, 0);
        assert_eq!(segment_one >> 31, 1);
        assert_eq!(segment_zero & GPU_HANDLE_ELEMENT_MASK, first);
        assert_eq!(segment_one & GPU_HANDLE_ELEMENT_MASK, first);
        assert_eq!(
            u64::from(first) * (GPU_GEOMETRY_ELEMENT_BYTES / 4) * 4,
            u64::from(first) * GPU_GEOMETRY_ELEMENT_BYTES
        );
    }

    #[test]
    fn snapshot_shader_has_no_hierarchy_ownership_or_geometry_copy_path() {
        let shader = include_str!("shaders/virtual_terrain.wgsl");
        let module = wgpu::naga::front::wgsl::parse_str(shader)
            .expect("virtual terrain snapshot shader must parse as WGSL");
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .expect("virtual terrain snapshot shader must pass Naga validation");
        assert!(shader.contains("fn encode_snapshot"));
        assert!(shader.contains("fn validate_candidate_structure"));
        assert!(shader.contains("fn validate_snapshot"));
        assert!(shader.contains("handles[destination + element] = first_handle + element"));
        assert!(shader.contains("if handles[destination + element] != first_handle + element"));
        assert!(shader.contains("page.destinations[stream]"));
        assert!(shader.contains("page_tokens[page_index] = 1u"));
        for line in shader
            .lines()
            .filter(|line| line.contains("atomic") && !line.trim_start().starts_with("//"))
        {
            assert!(
                line.contains("overflow_flags: atomic<u32>")
                    || line.contains("encoded_pages: atomic<u32>")
                    || line.contains("atomicOr")
                    || line.contains("atomicAdd(&counters.encoded_pages, 1u)"),
                "only failure flags and one bounded page completion may be atomic: {line}"
            );
        }
        assert_eq!(
            shader.matches("atomicAdd").count(),
            1,
            "there is exactly one completion atomic in the page validator and none per handle"
        );
        assert!(!shader.contains("fingerprint_sum"));
        assert!(!shader.contains("fingerprint_square"));
        assert!(!shader.contains("finalize_snapshot"));
        assert!(!shader.contains("indirect_commands"));
        assert!(!shader.contains("traverse"));
        assert!(!shader.contains("geometry_source"));
        assert!(!shader.contains("compact_surfaces"));
    }

    #[test]
    fn snapshot_feedback_buffers_remain_copyable_for_exact_readback() {
        assert!(snapshot_counter_buffer_usage().contains(wgpu::BufferUsages::COPY_SRC));
        assert!(candidate_page_buffer_usage().contains(wgpu::BufferUsages::COPY_SRC));
        assert!(snapshot_indirect_buffer_usage().contains(wgpu::BufferUsages::COPY_SRC));
        assert!(snapshot_indirect_buffer_usage().contains(wgpu::BufferUsages::INDIRECT));
        assert!(!snapshot_indirect_buffer_usage().contains(wgpu::BufferUsages::STORAGE));
    }

    #[test]
    #[ignore = "requires an actual native WGPU adapter; browser automation must run this path"]
    fn real_wgpu_executes_structure_encode_exact_validation_and_readback() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = block_on_native(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            apply_limit_buckets: false,
        }))
        .expect("the explicit hardware test requires a real WGPU adapter");
        let (device, queue) = block_on_native(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: wgpu::Limits::default(),
            required_features: wgpu::Features::empty(),
            ..Default::default()
        }))
        .expect("the explicit hardware test requires a WGPU device");

        let mut control =
            VirtualTerrainGpuControl::new(&device, VirtualTerrainCapacity::DEVELOPMENT_128_MIB)
                .expect("the real device must support the production snapshot layout");
        let source = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("exact snapshot hardware-test source"),
            size: GPU_GEOMETRY_ELEMENT_BYTES * 8,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        control
            .bind_geometry_sources(&device, &[source])
            .expect("hardware-test geometry source");
        let key = TerrainPageKey::surface(0, 0, 0);
        control
            .update_page_geometry(
                key,
                VirtualTerrainGpuGeometry {
                    opaque_surface: VirtualTerrainGpuGeometryRange {
                        source_segment: 0,
                        source_offset_bytes: 0,
                        element_count: 3,
                    },
                    opaque_triangle: VirtualTerrainGpuGeometryRange {
                        source_segment: 0,
                        source_offset_bytes: GPU_GEOMETRY_ELEMENT_BYTES * 3,
                        element_count: 2,
                    },
                    ..VirtualTerrainGpuGeometry::default()
                },
            )
            .expect("hardware-test geometry");
        let identity = VirtualTerrainSnapshotIdentity {
            fingerprint: 0x1234_5678_9abc_def0,
            selected_pages: &[key],
            ownerless_roots: 0,
        };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("exact snapshot hardware-test encoder"),
        });
        let generation = match control
            .encode_candidate(&queue, &mut encoder, identity, None)
            .expect("hardware-test candidate encoding")
        {
            VirtualTerrainCandidateEncodeOutcome::Encoded(generation) => generation,
            VirtualTerrainCandidateEncodeOutcome::ReadbackOnly(_) => {
                panic!("a new hardware-test candidate must encode")
            }
        };
        control.submit_pending_readback(&mut encoder, generation);
        queue.submit([encoder.finish()]);
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("hardware-test submission");

        let feedback = control.latest_feedback().expect("hardware-test feedback");
        assert_eq!(feedback.encoding_overflow_flags, 0);
        assert_eq!(feedback.encoded_pages, 1);
        assert_eq!(feedback.encoded_surface_handles, 3);
        assert_eq!(feedback.encoded_triangle_handles, 2);
        assert!(control.candidate_is_certified(identity));

        control.invalidate_candidate();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("self-referential descriptor rejection hardware-test encoder"),
        });
        let generation = match control
            .encode_candidate(&queue, &mut encoder, identity, None)
            .expect("second hardware-test candidate encoding")
        {
            VirtualTerrainCandidateEncodeOutcome::Encoded(generation) => generation,
            VirtualTerrainCandidateEncodeOutcome::ReadbackOnly(_) => {
                panic!("the invalidated hardware-test candidate must encode again")
            }
        };
        let self_consistent_but_wrong = GpuCandidatePage {
            ranges: [[1, 3], [3, 2], [0, 0], [0, 0]],
            destinations: [0; STREAM_COUNT],
        };
        queue.write_buffer(
            &control.candidate_pages,
            0,
            bytemuck::bytes_of(&self_consistent_but_wrong),
        );
        control.submit_pending_readback(&mut encoder, generation);
        queue.submit([encoder.finish()]);
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("self-referential descriptor rejection submission");

        let feedback = control
            .latest_feedback()
            .expect("self-referential descriptor rejection feedback");
        assert_ne!(
            feedback.encoding_overflow_flags & VALIDATION_DESCRIPTOR_MISMATCH,
            0,
            "GPU encode and exact validation agreeing with a corrupted descriptor cannot certify it against canonical CPU metadata",
        );
        assert!(!control.candidate_is_certified(identity));
    }

    #[test]
    fn matching_metadata_cannot_override_a_dirty_active_geometry_generation() {
        let key = TerrainPageKey::surface(3, -2, 7);
        let metadata = metadata(9, 42, vec![key], 0, [1, 0, 0, 0]);
        let identity = VirtualTerrainSnapshotIdentity {
            fingerprint: 42,
            selected_pages: &[key],
            ownerless_roots: 0,
        };
        assert!(snapshot_metadata_matches(Some(&metadata), identity));
        let partial_seam_rebuild_failed_after_mutation = true;
        let snapshot_is_current = !partial_seam_rebuild_failed_after_mutation
            && snapshot_metadata_matches(Some(&metadata), identity);
        assert!(
            !snapshot_is_current,
            "a failed seam batch must leave the matching active snapshot non-current"
        );
        assert!(
            snapshot_metadata_matches(Some(&metadata), identity),
            "the immutable previously published allocation remains safe to present while a replacement is encoded"
        );
        assert_eq!(
            snapshot_candidate_work(true, true, false, None, false),
            Some(VirtualTerrainCandidateWork::Encode),
            "dirty same-key geometry must force a new inactive handle snapshot"
        );
        assert_eq!(
            snapshot_candidate_work(
                true,
                true,
                true,
                Some(SnapshotPendingState::InFlight),
                false,
            ),
            None,
            "an already pending matching replacement must not be encoded twice"
        );
    }

    #[test]
    fn ownerless_roots_are_part_of_every_snapshot_identity() {
        let key = TerrainPageKey::surface(1, 4, -8);
        let metadata = metadata(7, 91, vec![key], 1, [0; STREAM_COUNT]);
        assert!(!snapshot_metadata_matches(
            Some(&metadata),
            VirtualTerrainSnapshotIdentity {
                fingerprint: 91,
                selected_pages: &[key],
                ownerless_roots: 0,
            }
        ));
        assert_eq!(
            snapshot_candidate_work(false, false, false, None, false),
            Some(VirtualTerrainCandidateWork::Encode),
            "the same selected handles cannot reuse a certificate from a different ownership state"
        );
    }

    #[test]
    fn saturated_readback_ring_retries_matching_pending_feedback() {
        let mut slots = [false; GPU_SNAPSHOT_READBACK_SLOTS];
        assert_eq!(
            claim_readback_slot(slots.len(), 0, |index| slots[index].then_some(index)),
            None,
            "a saturated ring cannot make the pending bank observable yet"
        );
        assert_eq!(
            snapshot_candidate_work(
                false,
                false,
                true,
                Some(SnapshotPendingState::SubmittedAwaitingReadback),
                false,
            ),
            Some(VirtualTerrainCandidateWork::ReadbackOnly),
            "a matching pending bank without an observable readback must be revisited"
        );
        slots[1] = true;
        assert_eq!(
            claim_readback_slot(slots.len(), 0, |index| {
                slots[index].then(|| {
                    slots[index] = false;
                    index
                })
            }),
            Some(1),
            "the same pending bank claims the first slot released by an older generation"
        );
        assert_eq!(
            snapshot_candidate_work(
                false,
                false,
                true,
                Some(SnapshotPendingState::InFlight),
                false,
            ),
            None,
            "once feedback is scheduled the immutable pending bank must not be re-encoded"
        );
    }

    #[test]
    fn failed_readback_callback_returns_pending_generation_to_retryable_state() {
        let pending = Mutex::new(Some(PendingSnapshotFeedback {
            generation: 44,
            state: SnapshotPendingState::InFlight,
        }));
        record_readback_completion(&pending, 44, false);
        assert_eq!(
            pending
                .lock()
                .unwrap()
                .as_ref()
                .map(|pending| pending.state),
            Some(SnapshotPendingState::Failed)
        );
        assert_eq!(
            snapshot_candidate_work(
                false,
                false,
                true,
                Some(SnapshotPendingState::Failed),
                false,
            ),
            Some(VirtualTerrainCandidateWork::ReadbackOnly),
            "map or parse failure must retry feedback for the immutable pending bank"
        );
        record_readback_completion(&pending, 44, true);
        assert_eq!(
            pending
                .lock()
                .unwrap()
                .as_ref()
                .map(|pending| pending.state),
            Some(SnapshotPendingState::Succeeded)
        );
    }

    #[test]
    fn abandoned_or_discarded_generation_cannot_be_resurrected_by_a_late_callback() {
        assert_eq!(
            snapshot_candidate_work(
                false,
                false,
                true,
                Some(SnapshotPendingState::Recorded),
                false,
            ),
            Some(VirtualTerrainCandidateWork::Encode),
            "a candidate recorded into an encoder that never submitted must be encoded again"
        );
        let pending = Mutex::new(None);
        record_readback_completion(&pending, 44, true);
        assert_eq!(
            *pending.lock().unwrap(),
            None,
            "late completion cannot recreate discarded generation state"
        );
        let pending = Mutex::new(Some(PendingSnapshotFeedback {
            generation: 45,
            state: SnapshotPendingState::InFlight,
        }));
        record_readback_completion(&pending, 44, true);
        assert_eq!(
            pending
                .lock()
                .unwrap()
                .as_ref()
                .map(|pending| pending.generation),
            Some(45),
            "late completion cannot overwrite the one bounded current generation"
        );
    }

    #[test]
    fn readback_validates_actual_indirect_arguments() {
        let ranges = vec![[[17, 3], [29, 2], [41, 1], [0, 0]]];
        let mut counts = [0; STREAM_COUNT];
        let candidate_pages = ranges
            .iter()
            .map(|ranges| {
                assign_candidate_page_destinations(
                    *ranges,
                    &mut counts,
                    VIRTUAL_TERRAIN_HANDLE_CAPACITIES,
                )
                .unwrap()
            })
            .collect();
        let mut metadata = metadata(5, 8, vec![], 0, counts);
        metadata.expected_ranges = ranges;
        let mut readback = GpuSnapshotReadback {
            counters: GpuSnapshotCounters::default(),
            indirect_commands: expected_indirect_commands(metadata.expected_counts),
            candidate_pages,
        };
        assert_eq!(snapshot_validation_failure_flags(&readback, &metadata), 0);
        readback.indirect_commands[1] = 2;
        assert_ne!(
            snapshot_validation_failure_flags(&readback, &metadata) & VALIDATION_INDIRECT_MISMATCH,
            0
        );
    }

    #[test]
    fn deterministic_prefix_destinations_partition_every_stream() {
        let ranges = [
            [[10, 2], [20, 0], [30, 1], [40, 3]],
            [[12, 1], [50, 4], [31, 0], [43, 2]],
            [[13, 3], [54, 1], [31, 2], [45, 0]],
        ];
        let capacities = [6, 5, 3, 5];
        let build = || {
            let mut prefixes = [0; STREAM_COUNT];
            let pages = ranges
                .into_iter()
                .map(|ranges| {
                    assign_candidate_page_destinations(ranges, &mut prefixes, capacities).unwrap()
                })
                .collect::<Vec<_>>();
            (pages, prefixes)
        };
        let (pages, counts) = build();
        assert_eq!(build(), (pages.clone(), counts));
        assert_eq!(counts, capacities);
        assert_eq!(pages[0].destinations, [0, 0, 0, 0]);
        assert_eq!(pages[1].destinations, [2, 0, 1, 3]);
        assert_eq!(pages[2].destinations, [3, 4, 1, 5]);
        for stream in 0..STREAM_COUNT {
            for pair in pages.windows(2) {
                assert_eq!(
                    pair[0].destinations[stream] + pair[0].ranges[stream][1],
                    pair[1].destinations[stream]
                );
            }
            assert_eq!(
                pages.last().unwrap().destinations[stream]
                    + pages.last().unwrap().ranges[stream][1],
                counts[stream]
            );
        }
    }

    #[test]
    fn deterministic_prefix_assignment_is_capacity_checked_and_transactional() {
        let mut prefixes = [3, u32::MAX, 0, 0];
        let before = prefixes;
        assert_eq!(
            assign_candidate_page_destinations(
                [[0, 2], [0, 1], [0, 0], [0, 0]],
                &mut prefixes,
                [4, u32::MAX, 0, 0],
            ),
            Err(VirtualTerrainGpuError::GeometryCapacity)
        );
        assert_eq!(
            prefixes, before,
            "a rejected page cannot partially advance another stream"
        );

        let mut prefixes = [3, 0, 0, 0];
        let page = assign_candidate_page_destinations(
            [[0, 1], [0, 0], [0, 0], [0, 0]],
            &mut prefixes,
            [4, 0, 0, 0],
        )
        .unwrap();
        assert_eq!(page.destinations, [3, 0, 0, 0]);
        assert_eq!(prefixes, [4, 0, 0, 0]);
    }

    #[test]
    fn readback_descriptors_must_match_canonical_ranges_and_prefixes_exactly() {
        let expected_ranges = vec![
            [[10, 2], [20, 1], [0, 0], [40, 1]],
            [[12, 2], [21, 0], [30, 1], [41, 2]],
        ];
        let mut expected_counts = [0; STREAM_COUNT];
        let canonical = expected_ranges
            .iter()
            .map(|ranges| {
                assign_candidate_page_destinations(
                    *ranges,
                    &mut expected_counts,
                    VIRTUAL_TERRAIN_HANDLE_CAPACITIES,
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        assert!(candidate_descriptors_match_canonical(
            &canonical,
            &expected_ranges,
            expected_counts,
        ));
        let mut metadata = metadata(4, 5, vec![], 0, expected_counts);
        metadata.expected_ranges = expected_ranges.clone();
        let mut readback = GpuSnapshotReadback {
            counters: GpuSnapshotCounters::default(),
            indirect_commands: expected_indirect_commands(expected_counts),
            candidate_pages: canonical.clone(),
        };
        assert_eq!(snapshot_validation_failure_flags(&readback, &metadata), 0);

        let mut wrong_first = canonical.clone();
        wrong_first[0].ranges[0][0] += 1;
        assert!(!candidate_descriptors_match_canonical(
            &wrong_first,
            &expected_ranges,
            expected_counts,
        ));
        readback.candidate_pages = wrong_first.clone();
        assert_ne!(
            snapshot_validation_failure_flags(&readback, &metadata)
                & VALIDATION_DESCRIPTOR_MISMATCH,
            0
        );

        let mut wrong_count_zero = canonical.clone();
        wrong_count_zero[1].ranges[0][1] = 0;
        assert!(!candidate_descriptors_match_canonical(
            &wrong_count_zero,
            &expected_ranges,
            expected_counts,
        ));

        let mut destination_gap = canonical.clone();
        destination_gap[1].destinations[0] += 1;
        assert!(!candidate_descriptors_match_canonical(
            &destination_gap,
            &expected_ranges,
            expected_counts,
        ));

        let mut destination_overlap = canonical.clone();
        destination_overlap[1].destinations[0] -= 1;
        assert!(!candidate_descriptors_match_canonical(
            &destination_overlap,
            &expected_ranges,
            expected_counts,
        ));

        let mut stale_descriptor = canonical.clone();
        stale_descriptor[1].ranges = canonical[0].ranges;
        assert!(!candidate_descriptors_match_canonical(
            &stale_descriptor,
            &expected_ranges,
            expected_counts,
        ));
        assert!(!candidate_descriptors_match_canonical(
            &canonical[..1],
            &expected_ranges,
            expected_counts,
        ));
    }

    #[test]
    fn canonical_empty_page_is_evidence_bearing_and_cannot_be_skipped() {
        let selected_pages = vec![
            TerrainPageKey::surface(0, 0, 0),
            TerrainPageKey::surface(0, 1, 0),
            TerrainPageKey::surface(0, 2, 0),
        ];
        let expected_ranges = vec![
            [[10, 1], [0, 0], [0, 0], [0, 0]],
            [[0, 0], [0, 0], [0, 0], [0, 0]],
            [[11, 1], [0, 0], [0, 0], [0, 0]],
        ];
        let mut expected_counts = [0; STREAM_COUNT];
        let descriptors = expected_ranges
            .iter()
            .map(|ranges| {
                assign_candidate_page_destinations(
                    *ranges,
                    &mut expected_counts,
                    VIRTUAL_TERRAIN_HANDLE_CAPACITIES,
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        let mut metadata = metadata(9, 17, selected_pages, 0, expected_counts);
        metadata.expected_ranges = expected_ranges;
        assert!(candidate_descriptors_match_canonical(
            &descriptors,
            &metadata.expected_ranges,
            metadata.expected_counts,
        ));

        let complete = GpuSnapshotCounters {
            element_counts: expected_counts,
            encoded_pages: 3,
            generation: split_u64(metadata.generation),
            fingerprint: split_u64(metadata.fingerprint),
            selected_count: 3,
            ..GpuSnapshotCounters::default()
        };
        assert!(snapshot_counter_evidence_matches(complete, &metadata));
        assert!(
            !snapshot_counter_evidence_matches(
                GpuSnapshotCounters {
                    encoded_pages: 2,
                    ..complete
                },
                &metadata,
            ),
            "skipping even a zero-handle page dispatch must withhold publication"
        );
    }

    #[test]
    fn exact_validation_rejects_sum_square_collision_duplicates_and_missing_handles() {
        let mut prefixes = [0; STREAM_COUNT];
        let pages = [0, 3, 3]
            .map(|first| {
                assign_candidate_page_destinations(
                    [[first, 1], [0, 0], [0, 0], [0, 0]],
                    &mut prefixes,
                    [3, 0, 0, 0],
                )
                .unwrap()
            })
            .to_vec();
        assert!(exact_candidate_handles_match(
            &pages,
            &[vec![0, 3, 3], vec![], vec![], vec![]]
        ));
        assert!(
            !exact_candidate_handles_match(&pages, &[vec![1, 1, 4], vec![], vec![], vec![]]),
            "[0, 3, 3] and [1, 1, 4] collide under sum and square-sum but not exact comparison"
        );
        assert!(!exact_candidate_handles_match(
            &pages,
            &[vec![0, 3, 0], vec![], vec![], vec![]]
        ));
        assert!(!exact_candidate_handles_match(
            &pages,
            &[vec![0, 3], vec![], vec![], vec![]]
        ));
    }

    #[test]
    fn exact_validation_rejects_wrong_segment_and_wrong_destination() {
        let mut prefixes = [0; STREAM_COUNT];
        let pages = [
            assign_candidate_page_destinations(
                [[GPU_HANDLE_SEGMENT_BIT | 7, 1], [0, 0], [0, 0], [0, 0]],
                &mut prefixes,
                [2, 0, 0, 0],
            )
            .unwrap(),
            assign_candidate_page_destinations(
                [[19, 1], [0, 0], [0, 0], [0, 0]],
                &mut prefixes,
                [2, 0, 0, 0],
            )
            .unwrap(),
        ];
        assert!(exact_candidate_handles_match(
            &pages,
            &[vec![GPU_HANDLE_SEGMENT_BIT | 7, 19], vec![], vec![], vec![]]
        ));
        assert!(!exact_candidate_handles_match(
            &pages,
            &[vec![7, 19], vec![], vec![], vec![]]
        ));
        assert!(!exact_candidate_handles_match(
            &pages,
            &[vec![19, GPU_HANDLE_SEGMENT_BIT | 7], vec![], vec![], vec![]]
        ));
        let mut wrong_destination = pages;
        wrong_destination[1].destinations[0] = 2;
        assert!(!exact_candidate_handles_match(
            &wrong_destination,
            &[vec![GPU_HANDLE_SEGMENT_BIT | 7, 19], vec![], vec![], vec![]]
        ));
    }

    #[test]
    fn empty_pages_and_zero_streams_have_exact_zero_width_partitions() {
        let mut prefixes = [0; STREAM_COUNT];
        let empty = assign_candidate_page_destinations(
            [[123, 0], [456, 0], [789, 0], [999, 0]],
            &mut prefixes,
            [0; STREAM_COUNT],
        )
        .unwrap();
        assert_eq!(empty.destinations, [0; STREAM_COUNT]);
        assert_eq!(prefixes, [0; STREAM_COUNT]);
        assert!(exact_candidate_handles_match(
            &[empty],
            &[vec![], vec![], vec![], vec![]]
        ));
    }

    #[test]
    fn resident_empty_geometry_remains_an_encodable_page_owner() {
        let key = TerrainPageKey::surface(0, 4, -9);
        let empty = VirtualTerrainGpuGeometry::default();
        let mut geometries = BTreeMap::new();

        assert!(geometry_directory_entry_changes(
            &geometries,
            key,
            Some(empty)
        ));
        replace_geometry_directory_entry(&mut geometries, key, Some(empty));
        assert_eq!(
            geometries.get(&key),
            Some(&empty),
            "zero-handle terrain is resident data, not an absent directory record"
        );
        assert_eq!(
            geometries[&key]
                .ranges()
                .into_iter()
                .map(|range| range.element_count)
                .sum::<u32>(),
            0
        );
    }

    #[test]
    fn eviction_and_same_address_rehydration_invalidate_absolute_handle_snapshots() {
        let key = TerrainPageKey::surface(2, -5, 11);
        let reused = VirtualTerrainGpuGeometry {
            opaque_surface: VirtualTerrainGpuGeometryRange {
                source_segment: 1,
                source_offset_bytes: GPU_GEOMETRY_ELEMENT_BYTES * 17,
                element_count: 3,
            },
            ..VirtualTerrainGpuGeometry::default()
        };
        let metadata = metadata(12, 99, vec![key], 0, [3, 0, 0, 0]);
        let mut geometries = BTreeMap::from([(key, reused)]);

        assert_eq!(
            geometry_mutation_impact(None, Some(&metadata), key),
            (false, true),
            "eviction must invalidate a pending bank before its allocation is reused"
        );
        assert!(geometry_directory_entry_changes(&geometries, key, None));
        replace_geometry_directory_entry(&mut geometries, key, None);

        assert!(
            geometry_directory_entry_changes(&geometries, key, Some(reused)),
            "rehydrating into the same numeric address is still a new allocation lifetime"
        );
        assert_eq!(
            geometry_mutation_impact(Some(&metadata), Some(&metadata), key),
            (true, true),
            "active presentation becomes non-current and pending publication is discarded"
        );
        replace_geometry_directory_entry(&mut geometries, key, Some(reused));
        assert_eq!(geometries.get(&key), Some(&reused));
    }

    #[test]
    fn geometry_mutation_uses_sorted_snapshot_membership_without_a_duplicate_set() {
        let selected_pages = (0..16_384)
            .map(|x| TerrainPageKey::surface(0, x, -7))
            .collect::<Vec<_>>();
        let metadata = metadata(1, 2, selected_pages, 0, [0; STREAM_COUNT]);
        assert_eq!(metadata.selected_pages.len(), 16_384);
        assert_eq!(
            geometry_mutation_impact(
                Some(&metadata),
                Some(&metadata),
                TerrainPageKey::surface(0, 16_383, -7),
            ),
            (true, true),
            "retention uses binary search over the immutable sorted selected vector"
        );
    }
}
