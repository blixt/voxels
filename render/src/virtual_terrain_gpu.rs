//! Failure-atomic GPU snapshots for virtual microvoxel terrain.
//!
//! The CPU hierarchy is the sole selection authority. A candidate cut is supplied explicitly,
//! expanded into 32-bit geometry handles in the inactive bank, and certified by bounded GPU
//! counters. The published bank is never written. Promotion is a CPU-side bank swap after the
//! exact candidate generation, fingerprint, page count, geometry counts, and bounds are read back.

use crate::virtual_terrain::VirtualTerrainCapacity;
use bytemuck::{Pod, Zeroable};
use std::collections::{BTreeMap, BTreeSet};
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
}

const _: () = assert!(size_of::<GpuCandidatePage>() == 32);

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
    selected_page_set: BTreeSet<TerrainPageKey>,
    ownerless_roots: u32,
    expected_counts: [u32; STREAM_COUNT],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SnapshotReadbackState {
    InFlight,
    Succeeded,
    Failed,
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
    source_buffers: [Buffer; 2],
    source_element_capacities: [u32; 2],
    bound_source_count: usize,
    render_layout: wgpu::BindGroupLayout,
    encode_pipeline: ComputePipeline,
    finalize_pipeline: ComputePipeline,
    banks: [SnapshotBank; 2],
    active_bank: usize,
    pending_bank: Option<usize>,
    active_geometry_dirty: bool,
    next_generation: u64,
    latest_raw_feedback: Arc<Mutex<Option<GpuSnapshotCounters>>>,
    minimum_feedback_generation: Arc<Mutex<u64>>,
    readback_states: Arc<Mutex<BTreeMap<u64, SnapshotReadbackState>>>,
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
        if VIRTUAL_TERRAIN_HANDLE_BANK_BYTES > maximum_storage
            || candidate_bytes > maximum_storage
            || device.limits().max_storage_buffers_per_shader_stage < 4
        {
            return Err(VirtualTerrainGpuError::DeviceLimit);
        }
        let candidate_pages = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bounded virtual terrain CPU-selected candidate pages"),
            size: candidate_bytes,
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
        let encode_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("encode CPU-selected virtual terrain handles"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("encode_snapshot"),
            compilation_options: Default::default(),
            cache: None,
        });
        let finalize_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("finalize virtual terrain snapshot"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("finalize_snapshot"),
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
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
            });
            let indirect = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("virtual terrain snapshot indirect commands"),
                size: 64,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::INDIRECT
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let encode_bind_group = create_encode_bind_group(
                device,
                &encode_layout,
                &candidate_pages,
                &handles,
                &counters,
                &indirect,
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
        let readback_bytes = size_of::<GpuSnapshotCounters>() as u64;
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
            source_buffers,
            source_element_capacities: [1, 1],
            bound_source_count: 0,
            render_layout,
            encode_pipeline,
            finalize_pipeline,
            banks,
            active_bank: 0,
            pending_bank: None,
            active_geometry_dirty: false,
            next_generation: 1,
            latest_raw_feedback: Arc::new(Mutex::new(None)),
            minimum_feedback_generation: Arc::new(Mutex::new(1)),
            readback_states: Arc::new(Mutex::new(BTreeMap::new())),
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

    pub(crate) fn candidate_requires_encoding(
        &self,
        fingerprint: u64,
        selected_pages: &[TerrainPageKey],
    ) -> bool {
        let matches = |metadata: Option<&SnapshotMetadata>| {
            snapshot_metadata_matches(metadata, fingerprint, selected_pages)
        };
        snapshot_requires_encoding(
            self.active_geometry_dirty,
            matches(self.banks[self.active_bank].metadata.as_ref()),
            self.pending_bank
                .is_some_and(|bank| matches(self.banks[bank].metadata.as_ref())),
            self.pending_bank
                .is_some_and(|bank| self.pending_feedback_is_observable_or_in_flight(bank)),
        )
    }

    pub(crate) fn active_snapshot_matches(
        &self,
        fingerprint: u64,
        selected_pages: &[TerrainPageKey],
    ) -> bool {
        !self.active_geometry_dirty
            && snapshot_metadata_matches(
                self.banks[self.active_bank].metadata.as_ref(),
                fingerprint,
                selected_pages,
            )
    }

    /// Whether the immutable active bank still represents the already-published cut.
    ///
    /// Candidate geometry may have changed under the same logical key/fingerprint while the old
    /// source allocation remains retired and immutable for presentation. In that case the active
    /// bank is still safe to draw, but `active_snapshot_matches` correctly requires a new candidate
    /// encoding before it can be treated as current.
    pub(crate) fn presented_snapshot_matches(
        &self,
        fingerprint: u64,
        selected_pages: &[TerrainPageKey],
    ) -> bool {
        snapshot_metadata_matches(
            self.banks[self.active_bank].metadata.as_ref(),
            fingerprint,
            selected_pages,
        )
    }

    pub(crate) fn encode_candidate(
        &mut self,
        queue: &Queue,
        encoder: &mut CommandEncoder,
        selected_pages: &[TerrainPageKey],
        ownerless_roots: u32,
        fingerprint: u64,
        timestamps: Option<VirtualTerrainGpuTimestampWrites<'_>>,
    ) -> Result<u64, VirtualTerrainGpuError> {
        if selected_pages.len() > self.capacity.max_selected_pages {
            return Err(VirtualTerrainGpuError::GeometryCapacity);
        }
        if let Some(bank) = self.pending_bank {
            let matching_generation = self.banks[bank].metadata.as_ref().and_then(|metadata| {
                (metadata.fingerprint == fingerprint && metadata.selected_pages == selected_pages)
                    .then_some(metadata.generation)
            });
            if let Some(generation) = matching_generation {
                if !self.pending_feedback_is_observable_or_in_flight(bank) {
                    self.schedule_readback(encoder, bank);
                }
                return Ok(generation);
            }
        }
        let inactive = 1 - self.active_bank;
        let mut pages = Vec::with_capacity(selected_pages.len());
        let mut expected_counts = [0u32; STREAM_COUNT];
        for key in selected_pages {
            let geometry = self
                .geometries
                .get(key)
                .copied()
                .ok_or(VirtualTerrainGpuError::UnknownPage(*key))?;
            let ranges = geometry
                .ranges()
                .map(|range| self.pack_range(range))
                .into_iter()
                .collect::<Result<Vec<_>, _>>()?
                .try_into()
                .map_err(|_| VirtualTerrainGpuError::InvalidGeometry)?;
            for (stream, range) in geometry.ranges().into_iter().enumerate() {
                expected_counts[stream] = expected_counts[stream]
                    .checked_add(range.element_count)
                    .ok_or(VirtualTerrainGpuError::GeometryCapacity)?;
            }
            pages.push(GpuCandidatePage { ranges });
        }
        if !pages.is_empty() {
            queue.write_buffer(&self.candidate_pages, 0, bytemuck::cast_slice(&pages));
        }
        let generation = self.next_generation;
        self.next_generation = self.next_generation.wrapping_add(1).max(1);
        let counters = GpuSnapshotCounters {
            generation: split_u64(generation),
            fingerprint: split_u64(fingerprint),
            selected_count: selected_pages.len() as u32,
            ownerless_roots,
            source_element_capacities: self.source_element_capacities,
            ..GpuSnapshotCounters::default()
        };
        queue.write_buffer(
            &self.banks[inactive].counters,
            0,
            bytemuck::bytes_of(&counters),
        );
        queue.write_buffer(&self.banks[inactive].indirect, 0, &[0; 64]);
        self.banks[inactive].metadata = Some(SnapshotMetadata {
            generation,
            fingerprint,
            selected_pages: selected_pages.to_vec(),
            selected_page_set: selected_pages.iter().copied().collect(),
            ownerless_roots,
            expected_counts,
        });
        self.pending_bank = Some(inactive);
        if let Ok(mut states) = self.readback_states.lock() {
            states.remove(&generation);
        }
        if let Ok(mut minimum) = self.minimum_feedback_generation.lock() {
            *minimum = generation;
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
                label: Some("finalize CPU-selected virtual terrain snapshot"),
                timestamp_writes: timestamps.map(|timestamps| wgpu::ComputePassTimestampWrites {
                    query_set: timestamps.query_set,
                    beginning_of_pass_write_index: Some(timestamps.finalize_first_query),
                    end_of_pass_write_index: Some(timestamps.finalize_first_query + 1),
                }),
            });
            pass.set_pipeline(&self.finalize_pipeline);
            pass.set_bind_group(0, &self.banks[inactive].encode_bind_group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        self.schedule_readback(encoder, inactive);
        Ok(generation)
    }

    fn pending_feedback_is_observable_or_in_flight(&self, bank: usize) -> bool {
        let Some(generation) = self.banks[bank]
            .metadata
            .as_ref()
            .map(|metadata| metadata.generation)
        else {
            return false;
        };
        let state = self
            .readback_states
            .lock()
            .ok()
            .and_then(|states| states.get(&generation).copied());
        match state {
            Some(SnapshotReadbackState::InFlight) => true,
            Some(SnapshotReadbackState::Succeeded) => {
                self.latest_raw_feedback.lock().is_ok_and(|feedback| {
                    feedback
                        .as_ref()
                        .is_some_and(|feedback| join_u64(feedback.generation) == generation)
                })
            }
            Some(SnapshotReadbackState::Failed) | None => false,
        }
    }

    fn schedule_readback(&mut self, encoder: &mut CommandEncoder, bank: usize) {
        let Some(generation) = self.banks[bank]
            .metadata
            .as_ref()
            .map(|metadata| metadata.generation)
        else {
            return;
        };
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
        if let Ok(mut states) = self.readback_states.lock() {
            states.insert(generation, SnapshotReadbackState::InFlight);
        }
        self.next_readback_slot = (slot_index + 1) % self.readback_slots.len();
        encoder.copy_buffer_to_buffer(
            &self.banks[bank].counters,
            0,
            &slot.buffer,
            0,
            size_of::<GpuSnapshotCounters>() as u64,
        );
        let callback_buffer = slot.buffer.clone();
        let available = Arc::clone(&slot.available);
        let feedback = Arc::clone(&self.latest_raw_feedback);
        let minimum = Arc::clone(&self.minimum_feedback_generation);
        let states = Arc::clone(&self.readback_states);
        encoder.map_buffer_on_submit(&slot.buffer, wgpu::MapMode::Read, .., move |result| {
            let parsed = result.is_ok().then(|| {
                let mapped = callback_buffer.get_mapped_range(..).ok()?;
                bytemuck::try_from_bytes::<GpuSnapshotCounters>(&mapped)
                    .ok()
                    .copied()
            });
            let parsed = parsed.flatten();
            let succeeded = parsed.is_some_and(|parsed| {
                join_u64(parsed.generation) == generation
                    && minimum.lock().is_ok_and(|minimum| generation >= *minimum)
                    && feedback.lock().is_ok_and(|mut destination| {
                        let is_newer = destination
                            .as_ref()
                            .is_none_or(|current| generation > join_u64(current.generation));
                        if is_newer {
                            *destination = Some(parsed);
                        }
                        is_newer
                    })
            });
            callback_buffer.unmap();
            record_readback_completion(&states, generation, succeeded);
            available.store(true, Ordering::Release);
        });
    }

    fn feedback_metadata(&self, counters: &GpuSnapshotCounters) -> Option<&SnapshotMetadata> {
        let generation = join_u64(counters.generation);
        self.banks
            .iter()
            .filter_map(|bank| bank.metadata.as_ref())
            .find(|metadata| metadata.generation == generation)
    }

    pub(crate) fn latest_feedback(&self) -> Option<GpuVirtualTerrainFeedback> {
        let raw = *self.latest_raw_feedback.lock().ok()?.as_ref()?;
        let metadata = self.feedback_metadata(&raw)?;
        Some(GpuVirtualTerrainFeedback {
            submission_id: metadata.generation,
            oracle_fingerprint: join_u64(raw.fingerprint),
            selected_pages: metadata.selected_pages.clone(),
            ownerless_roots: raw.ownerless_roots,
            encoded_surface_handles: raw.element_counts[STREAM_SURFACE],
            encoded_triangle_handles: raw.element_counts[STREAM_TRIANGLE],
            encoded_water_surface_handles: raw.element_counts[STREAM_WATER_SURFACE],
            encoded_water_triangle_handles: raw.element_counts[STREAM_WATER_TRIANGLE],
            encoded_pages: raw.encoded_pages,
            encoding_overflow_flags: raw.overflow_flags,
        })
    }

    pub(crate) fn candidate_is_certified(
        &self,
        fingerprint: u64,
        selected_pages: &[TerrainPageKey],
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
        metadata.fingerprint == fingerprint
            && metadata.selected_pages == selected_pages
            && join_u64(feedback.generation) == metadata.generation
            && join_u64(feedback.fingerprint) == metadata.fingerprint
            && feedback.selected_count == metadata.selected_pages.len() as u32
            && feedback.ownerless_roots == metadata.ownerless_roots
            && feedback.encoded_pages == metadata.selected_pages.len() as u32
            && feedback.element_counts == metadata.expected_counts
            && feedback.overflow_flags == 0
    }

    pub(crate) fn promote_certified_candidate(
        &mut self,
        fingerprint: u64,
        selected_pages: &[TerrainPageKey],
    ) -> Result<u64, VirtualTerrainGpuError> {
        if !self.candidate_is_certified(fingerprint, selected_pages) {
            return Err(VirtualTerrainGpuError::CandidateNotCertified);
        }
        let bank = self
            .pending_bank
            .take()
            .ok_or(VirtualTerrainGpuError::CandidateNotCertified)?;
        self.active_bank = bank;
        self.active_geometry_dirty = false;
        self.active_generation()
            .ok_or(VirtualTerrainGpuError::CandidateNotCertified)
    }

    pub(crate) fn invalidate_candidate(&mut self) {
        self.active_geometry_dirty = true;
        self.discard_pending_candidate();
    }

    fn discard_pending_candidate(&mut self) {
        if let Some(bank) = self.pending_bank.take() {
            if let Some(generation) = self.banks[bank]
                .metadata
                .as_ref()
                .map(|metadata| metadata.generation)
                && let Ok(mut states) = self.readback_states.lock()
            {
                states.remove(&generation);
            }
            self.banks[bank].metadata = None;
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
    fingerprint: u64,
    selected_pages: &[TerrainPageKey],
) -> bool {
    metadata.is_some_and(|metadata| {
        metadata.fingerprint == fingerprint && metadata.selected_pages == selected_pages
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
        metadata.is_some_and(|metadata| metadata.selected_page_set.contains(&key))
    };
    (selects(active), selects(pending))
}

const fn snapshot_requires_encoding(
    active_geometry_dirty: bool,
    active_matches: bool,
    pending_matches: bool,
    pending_feedback_observable_or_in_flight: bool,
) -> bool {
    (active_geometry_dirty || !active_matches)
        && (!pending_matches || !pending_feedback_observable_or_in_flight)
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
    states: &Mutex<BTreeMap<u64, SnapshotReadbackState>>,
    generation: u64,
    succeeded: bool,
) {
    if let Ok(mut states) = states.lock() {
        states.insert(
            generation,
            if succeeded {
                SnapshotReadbackState::Succeeded
            } else {
                SnapshotReadbackState::Failed
            },
        );
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
    indirect: &Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("virtual terrain inactive snapshot encoder"),
        layout,
        entries: &[
            entire_entry(0, candidates),
            entire_entry(1, handles),
            entire_entry(2, counters),
            entire_entry(3, indirect),
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
        assert!(shader.contains("fn encode_snapshot"));
        assert!(shader.contains("handles[destination + element] = first_handle + element"));
        assert!(!shader.contains("traverse"));
        assert!(!shader.contains("geometry_source"));
        assert!(!shader.contains("compact_surfaces"));
    }

    #[test]
    fn matching_metadata_cannot_override_a_dirty_active_geometry_generation() {
        let key = TerrainPageKey::surface(3, -2, 7);
        let metadata = SnapshotMetadata {
            generation: 9,
            fingerprint: 42,
            selected_pages: vec![key],
            selected_page_set: BTreeSet::from([key]),
            ownerless_roots: 0,
            expected_counts: [1, 0, 0, 0],
        };
        assert!(snapshot_metadata_matches(Some(&metadata), 42, &[key]));
        let partial_seam_rebuild_failed_after_mutation = true;
        let snapshot_is_current = !partial_seam_rebuild_failed_after_mutation
            && snapshot_metadata_matches(Some(&metadata), 42, &[key]);
        assert!(
            !snapshot_is_current,
            "a failed seam batch must leave the matching active snapshot non-current"
        );
        assert!(
            snapshot_metadata_matches(Some(&metadata), 42, &[key]),
            "the immutable previously published allocation remains safe to present while a replacement is encoded"
        );
        assert!(
            snapshot_requires_encoding(true, true, false, false),
            "dirty same-key geometry must force a new inactive handle snapshot"
        );
        assert!(
            !snapshot_requires_encoding(true, true, true, true),
            "an already pending matching replacement must not be encoded twice"
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
        assert!(
            snapshot_requires_encoding(false, false, true, false),
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
        assert!(
            !snapshot_requires_encoding(false, false, true, true),
            "once feedback is scheduled the immutable pending bank must not be re-encoded"
        );
    }

    #[test]
    fn failed_readback_callback_returns_pending_generation_to_retryable_state() {
        let states = Mutex::new(BTreeMap::from([(44, SnapshotReadbackState::InFlight)]));
        record_readback_completion(&states, 44, false);
        assert_eq!(
            states.lock().unwrap().get(&44),
            Some(&SnapshotReadbackState::Failed)
        );
        assert!(
            snapshot_requires_encoding(false, false, true, false),
            "map or parse failure must retry feedback for the immutable pending bank"
        );
        record_readback_completion(&states, 44, true);
        assert_eq!(
            states.lock().unwrap().get(&44),
            Some(&SnapshotReadbackState::Succeeded)
        );
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
        let metadata = SnapshotMetadata {
            generation: 12,
            fingerprint: 99,
            selected_pages: vec![key],
            selected_page_set: BTreeSet::from([key]),
            ownerless_roots: 0,
            expected_counts: [3, 0, 0, 0],
        };
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
    fn geometry_mutation_uses_an_indexed_selected_page_membership_set() {
        let selected_pages = (0..16_384)
            .map(|x| TerrainPageKey::surface(0, x, -7))
            .collect::<Vec<_>>();
        let selected_page_set = selected_pages.iter().copied().collect::<BTreeSet<_>>();
        let metadata = SnapshotMetadata {
            generation: 1,
            fingerprint: 2,
            selected_pages,
            selected_page_set,
            ownerless_roots: 0,
            expected_counts: [0; STREAM_COUNT],
        };
        assert_eq!(metadata.selected_page_set.len(), 16_384);
        assert_eq!(
            geometry_mutation_impact(
                Some(&metadata),
                Some(&metadata),
                TerrainPageKey::surface(0, 16_383, -7),
            ),
            (true, true),
            "retention checks indexed membership instead of rescanning the full selected vector"
        );
    }
}
