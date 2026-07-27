//! Fixed-capacity WebGPU control plane for virtual microvoxel terrain.
//!
//! The CPU hierarchy remains the executable correctness oracle. This module mirrors its immutable
//! directory and mutable residency/coherence bits into bounded storage buffers, traverses one
//! region root per GPU invocation, and returns selected-page/request feedback for comparison and
//! later indirect rendering. Overflow is data, never an implicit allocation or fabricated owner.

use crate::virtual_terrain::{VirtualTerrainCapacity, VirtualTerrainHierarchy, VirtualTerrainView};
use bytemuck::{Pod, Zeroable};
use std::collections::{BTreeMap, BTreeSet};
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use voxels_world::{
    TERRAIN_PAGE_MAX_CHILDREN, TerrainHierarchyDirectoryV1, TerrainHierarchyNode, TerrainPageKey,
};
use wgpu::util::DeviceExt;
use wgpu::{Buffer, CommandEncoder, ComputePipeline, Device, QuerySet, Queue};

const NODE_HAS_CHILDREN: u32 = 1;
const NODE_IS_ROOT: u32 = 1 << 1;
const NODE_RESIDENT: u32 = 1 << 2;
const NODE_REPLACEMENT_COHERENT: u32 = 1 << 3;
const NODE_PRIOR_REFINED: u32 = 1 << 4;
const INVALID_NODE: u32 = u32::MAX;
const GPU_TRAVERSAL_READBACK_SLOTS: usize = 3;
const GPU_TRAVERSAL_WORKGROUP_SIZE: u32 = 64;
pub(crate) const VIRTUAL_TERRAIN_COMPACT_SURFACE_BYTES: u64 = 64 * 1_024 * 1_024;
pub(crate) const VIRTUAL_TERRAIN_COMPACT_TRIANGLE_BYTES: u64 = 64 * 1_024 * 1_024;
pub(crate) const VIRTUAL_TERRAIN_COMPACT_WATER_SURFACE_BYTES: u64 = 16 * 1_024 * 1_024;
pub(crate) const VIRTUAL_TERRAIN_COMPACT_WATER_TRIANGLE_BYTES: u64 = 16 * 1_024 * 1_024;
const VIRTUAL_TERRAIN_SURFACE_BUFFER_BYTES: u64 =
    VIRTUAL_TERRAIN_COMPACT_SURFACE_BYTES + VIRTUAL_TERRAIN_COMPACT_WATER_SURFACE_BYTES;
const VIRTUAL_TERRAIN_TRIANGLE_BUFFER_BYTES: u64 =
    VIRTUAL_TERRAIN_COMPACT_TRIANGLE_BYTES + VIRTUAL_TERRAIN_COMPACT_WATER_TRIANGLE_BYTES;
pub(crate) const VIRTUAL_TERRAIN_SURFACE_INDIRECT_OFFSET: u64 = 16;
pub(crate) const VIRTUAL_TERRAIN_TRIANGLE_INDIRECT_OFFSET: u64 = 32;
pub(crate) const VIRTUAL_TERRAIN_WATER_SURFACE_INDIRECT_OFFSET: u64 = 48;
pub(crate) const VIRTUAL_TERRAIN_WATER_TRIANGLE_INDIRECT_OFFSET: u64 = 64;
const GPU_GEOMETRY_ELEMENT_BYTES: u64 = 24;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
struct GpuVirtualTerrainNode {
    minimum_level: [i32; 4],
    maximum_flags: [i32; 4],
    errors: [u32; 4],
    children_low: [u32; 4],
    children_high: [u32; 4],
}

const _: () = assert!(size_of::<GpuVirtualTerrainNode>() == 80);

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
struct GpuVirtualTerrainView {
    camera_near: [f32; 4],
    forward_far: [f32; 4],
    right_tangent_horizontal: [f32; 4],
    up_tangent_vertical: [f32; 4],
    projection_thresholds: [f32; 4],
    counts_flags: [u32; 4],
    options: [u32; 4],
}

const _: () = assert!(size_of::<GpuVirtualTerrainView>() == 112);

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
struct GpuVirtualTerrainCounters {
    selected_count: u32,
    request_count: u32,
    ownerless_roots: u32,
    visited_nodes: u32,
    overflow_flags: u32,
    stack_peak: u32,
    submission_id: [u32; 2],
    oracle_fingerprint: [u32; 2],
}

const _: () = assert!(size_of::<GpuVirtualTerrainCounters>() == 40);

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
struct GpuVirtualTerrainGeometryPage {
    opaque_surface_offset: u32,
    opaque_surface_count: u32,
    opaque_triangle_offset: u32,
    opaque_triangle_count: u32,
    water_surface_offset: u32,
    water_surface_count: u32,
    water_triangle_offset: u32,
    water_triangle_count: u32,
}

const _: () = assert!(size_of::<GpuVirtualTerrainGeometryPage>() == 32);

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
struct GpuVirtualTerrainCompactionCounters {
    surface_elements: u32,
    triangle_elements: u32,
    water_surface_elements: u32,
    water_triangle_elements: u32,
    copied_pages: u32,
    overflow_flags: u32,
    surface_capacity: u32,
    triangle_capacity: u32,
    water_surface_capacity: u32,
    water_triangle_capacity: u32,
    surface_water_word_offset: u32,
    triangle_water_word_offset: u32,
}

const _: () = assert!(size_of::<GpuVirtualTerrainCompactionCounters>() == 48);

impl GpuVirtualTerrainCompactionCounters {
    fn reset() -> Result<Self, VirtualTerrainGpuError> {
        Ok(Self {
            surface_capacity: element_capacity(VIRTUAL_TERRAIN_COMPACT_SURFACE_BYTES)?,
            triangle_capacity: element_capacity(VIRTUAL_TERRAIN_COMPACT_TRIANGLE_BYTES)?,
            water_surface_capacity: element_capacity(VIRTUAL_TERRAIN_COMPACT_WATER_SURFACE_BYTES)?,
            water_triangle_capacity: element_capacity(
                VIRTUAL_TERRAIN_COMPACT_WATER_TRIANGLE_BYTES,
            )?,
            surface_water_word_offset: word_offset(VIRTUAL_TERRAIN_COMPACT_SURFACE_BYTES)?,
            triangle_water_word_offset: word_offset(VIRTUAL_TERRAIN_COMPACT_TRIANGLE_BYTES)?,
            ..Self::default()
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct VirtualTerrainGpuGeometryRange {
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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct RawGpuVirtualTerrainFeedback {
    counters: GpuVirtualTerrainCounters,
    compaction: GpuVirtualTerrainCompactionCounters,
    selected_indices: Vec<u32>,
    requested_indices: Vec<u32>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct GpuVirtualTerrainFeedback {
    pub submission_id: u64,
    pub oracle_fingerprint: u64,
    pub selected_pages: Vec<TerrainPageKey>,
    pub requested_pages: Vec<TerrainPageKey>,
    pub ownerless_roots: u32,
    pub visited_nodes: u32,
    pub overflow_flags: u32,
    pub stack_peak: u32,
    pub compacted_surface_elements: u32,
    pub compacted_triangle_elements: u32,
    pub compacted_water_surface_elements: u32,
    pub compacted_water_triangle_elements: u32,
    pub compacted_pages: u32,
    pub compaction_overflow_flags: u32,
}

impl GpuVirtualTerrainFeedback {
    pub const fn overflowed(&self) -> bool {
        self.overflow_flags != 0 || self.compaction_overflow_flags != 0
    }
}

#[derive(Clone, Copy)]
pub(crate) struct VirtualTerrainGpuTimestampWrites<'a> {
    pub query_set: &'a QuerySet,
    pub traversal_first_query: u32,
    pub compaction_first_query: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum VirtualTerrainGpuError {
    DirectoryCapacity,
    RootCapacity,
    GeometryCapacity,
    InvalidGeometry,
    DuplicateNodeMismatch(TerrainPageKey),
    MissingChild(TerrainPageKey),
    UnknownPage(TerrainPageKey),
    InvalidView,
    DeviceLimit,
}

struct TraversalReadbackSlot {
    buffer: Buffer,
    available: Arc<AtomicBool>,
}

pub(crate) struct VirtualTerrainGpuControl {
    capacity: VirtualTerrainCapacity,
    nodes: Vec<GpuVirtualTerrainNode>,
    node_indices: BTreeMap<TerrainPageKey, u32>,
    node_keys: Vec<TerrainPageKey>,
    root_indices: Vec<u32>,
    prior_refined: BTreeSet<TerrainPageKey>,
    geometry_pages: Vec<GpuVirtualTerrainGeometryPage>,
    geometry_source_bytes: u64,
    geometry_source_bound: bool,
    node_buffer: Buffer,
    root_buffer: Buffer,
    view_buffer: Buffer,
    counter_buffer: Buffer,
    selected_buffer: Buffer,
    request_buffer: Buffer,
    geometry_page_buffer: Buffer,
    compact_surface_buffer: Buffer,
    compact_triangle_buffer: Buffer,
    compaction_counter_buffer: Buffer,
    indirect_buffer: Buffer,
    bind_group: wgpu::BindGroup,
    pipeline: ComputePipeline,
    compaction_layout: wgpu::BindGroupLayout,
    compaction_bind_group: wgpu::BindGroup,
    compaction_prepare_pipeline: ComputePipeline,
    compaction_pipeline: ComputePipeline,
    compaction_finalize_pipeline: ComputePipeline,
    readback_slots: Vec<TraversalReadbackSlot>,
    next_readback_slot: usize,
    next_submission_id: u64,
    feedback: Arc<Mutex<Option<RawGpuVirtualTerrainFeedback>>>,
}

impl VirtualTerrainGpuControl {
    pub(crate) fn new(
        device: &Device,
        capacity: VirtualTerrainCapacity,
    ) -> Result<Self, VirtualTerrainGpuError> {
        let node_bytes = buffer_bytes::<GpuVirtualTerrainNode>(capacity.max_directory_nodes)?;
        let root_bytes = buffer_bytes::<u32>(capacity.max_roots)?;
        let selected_bytes = buffer_bytes::<u32>(capacity.max_selected_pages)?;
        let request_bytes = buffer_bytes::<u32>(capacity.max_feedback_pages)?;
        let geometry_page_bytes =
            buffer_bytes::<GpuVirtualTerrainGeometryPage>(capacity.max_directory_nodes)?;
        let maximum_storage = device.limits().max_storage_buffer_binding_size;
        if [
            node_bytes,
            root_bytes,
            selected_bytes,
            request_bytes,
            geometry_page_bytes,
            VIRTUAL_TERRAIN_SURFACE_BUFFER_BYTES,
            VIRTUAL_TERRAIN_TRIANGLE_BUFFER_BYTES,
        ]
        .into_iter()
        .any(|bytes| bytes > maximum_storage)
            || device.limits().max_storage_buffers_per_shader_stage < 8
        {
            return Err(VirtualTerrainGpuError::DeviceLimit);
        }
        let node_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bounded virtual terrain hierarchy nodes"),
            size: node_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let root_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bounded virtual terrain hierarchy roots"),
            size: root_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let view_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("virtual terrain traversal view"),
            contents: bytemuck::bytes_of(&GpuVirtualTerrainView::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let counter_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("virtual terrain traversal counters"),
            contents: bytemuck::bytes_of(&GpuVirtualTerrainCounters::default()),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
        });
        let selected_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bounded virtual terrain selected pages"),
            size: selected_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let request_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bounded virtual terrain request feedback"),
            size: request_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let geometry_page_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bounded virtual terrain geometry page directory"),
            size: geometry_page_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let geometry_source_placeholder = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("empty virtual terrain geometry source"),
            size: size_of::<GpuVirtualTerrainGeometryPage>() as u64,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let compact_surface_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bounded compact virtual terrain surface stream"),
            size: VIRTUAL_TERRAIN_SURFACE_BUFFER_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::VERTEX,
            mapped_at_creation: false,
        });
        let compact_triangle_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bounded compact virtual terrain triangle stream"),
            size: VIRTUAL_TERRAIN_TRIANGLE_BUFFER_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::VERTEX,
            mapped_at_creation: false,
        });
        let compaction_counter_buffer =
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("virtual terrain compaction counters"),
                contents: bytemuck::bytes_of(&GpuVirtualTerrainCompactionCounters::reset()?),
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
            });
        let indirect_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("virtual terrain dispatch and draw indirect commands"),
            size: 80,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("virtual terrain traversal layout"),
            entries: &[
                uniform_entry(0),
                storage_entry(1, true),
                storage_entry(2, true),
                storage_entry(3, false),
                storage_entry(4, false),
                storage_entry(5, false),
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("virtual terrain traversal bind group"),
            layout: &layout,
            entries: &[
                entire_entry(0, &view_buffer),
                entire_entry(1, &node_buffer),
                entire_entry(2, &root_buffer),
                entire_entry(3, &counter_buffer),
                entire_entry(4, &selected_buffer),
                entire_entry(5, &request_buffer),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("virtual terrain traversal pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader =
            device.create_shader_module(wgpu::include_wgsl!("shaders/virtual_terrain.wgsl"));
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("bounded virtual terrain hierarchy traversal"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("traverse"),
            compilation_options: Default::default(),
            cache: None,
        });
        let compaction_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("virtual terrain compaction layout"),
            entries: &[
                storage_entry(0, true),
                storage_entry(1, false),
                storage_entry(2, true),
                storage_entry(3, true),
                storage_entry(4, false),
                storage_entry(5, false),
                storage_entry(6, false),
                storage_entry(7, false),
            ],
        });
        let compaction_bind_group = create_compaction_bind_group(
            device,
            &compaction_layout,
            &selected_buffer,
            &counter_buffer,
            &geometry_page_buffer,
            &geometry_source_placeholder,
            &compact_surface_buffer,
            &compact_triangle_buffer,
            &compaction_counter_buffer,
            &indirect_buffer,
        );
        let compaction_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("virtual terrain compaction pipeline layout"),
                bind_group_layouts: &[None, Some(&compaction_layout)],
                immediate_size: 0,
            });
        let compaction_prepare_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("prepare virtual terrain compaction"),
                layout: Some(&compaction_pipeline_layout),
                module: &shader,
                entry_point: Some("prepare_compaction"),
                compilation_options: Default::default(),
                cache: None,
            });
        let compaction_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("compact selected virtual terrain"),
                layout: Some(&compaction_pipeline_layout),
                module: &shader,
                entry_point: Some("compact_selected"),
                compilation_options: Default::default(),
                cache: None,
            });
        let compaction_finalize_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("finalize virtual terrain compaction"),
                layout: Some(&compaction_pipeline_layout),
                module: &shader,
                entry_point: Some("finalize_compaction"),
                compilation_options: Default::default(),
                cache: None,
            });
        let readback_bytes = readback_bytes(capacity)?;
        let readback_slots = (0..GPU_TRAVERSAL_READBACK_SLOTS)
            .map(|_| TraversalReadbackSlot {
                buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("virtual terrain traversal readback"),
                    size: readback_bytes,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                available: Arc::new(AtomicBool::new(true)),
            })
            .collect();
        Ok(Self {
            capacity,
            nodes: Vec::new(),
            node_indices: BTreeMap::new(),
            node_keys: Vec::new(),
            root_indices: Vec::new(),
            prior_refined: BTreeSet::new(),
            geometry_pages: Vec::new(),
            geometry_source_bytes: geometry_source_placeholder.size(),
            geometry_source_bound: false,
            node_buffer,
            root_buffer,
            view_buffer,
            counter_buffer,
            selected_buffer,
            request_buffer,
            geometry_page_buffer,
            compact_surface_buffer,
            compact_triangle_buffer,
            compaction_counter_buffer,
            indirect_buffer,
            bind_group,
            pipeline,
            compaction_layout,
            compaction_bind_group,
            compaction_prepare_pipeline,
            compaction_pipeline,
            compaction_finalize_pipeline,
            readback_slots,
            next_readback_slot: 0,
            next_submission_id: 1,
            feedback: Arc::new(Mutex::new(None)),
        })
    }

    pub(crate) fn register_directory(
        &mut self,
        queue: &Queue,
        directory: &TerrainHierarchyDirectoryV1,
    ) -> Result<(), VirtualTerrainGpuError> {
        let new_nodes = directory
            .nodes
            .iter()
            .filter(|node| !self.node_indices.contains_key(&node.key))
            .collect::<Vec<_>>();
        if self.nodes.len().saturating_add(new_nodes.len()) > self.capacity.max_directory_nodes {
            return Err(VirtualTerrainGpuError::DirectoryCapacity);
        }
        let new_roots = new_nodes.iter().filter(|node| node.is_root).count();
        if self.root_indices.len().saturating_add(new_roots) > self.capacity.max_roots {
            return Err(VirtualTerrainGpuError::RootCapacity);
        }
        for node in &directory.nodes {
            if let Some(index) = self.node_indices.get(&node.key).copied() {
                let packed = self
                    .nodes
                    .get(index as usize)
                    .ok_or(VirtualTerrainGpuError::DirectoryCapacity)?;
                if packed_static_identity(*packed) != node_static_identity(node) {
                    return Err(VirtualTerrainGpuError::DuplicateNodeMismatch(node.key));
                }
            }
        }
        let base = u32::try_from(self.nodes.len())
            .map_err(|_| VirtualTerrainGpuError::DirectoryCapacity)?;
        for (offset, node) in new_nodes.iter().enumerate() {
            let index = base
                .checked_add(
                    u32::try_from(offset).map_err(|_| VirtualTerrainGpuError::DirectoryCapacity)?,
                )
                .ok_or(VirtualTerrainGpuError::DirectoryCapacity)?;
            self.node_indices.insert(node.key, index);
            self.node_keys.push(node.key);
        }
        let mut packed_nodes = Vec::with_capacity(new_nodes.len());
        for node in new_nodes {
            let packed = pack_node(node, &self.node_indices)?;
            if node.is_root {
                self.root_indices.push(
                    *self
                        .node_indices
                        .get(&node.key)
                        .ok_or(VirtualTerrainGpuError::DirectoryCapacity)?,
                );
            }
            packed_nodes.push(packed);
        }
        if !packed_nodes.is_empty() {
            let offset = u64::from(base)
                .checked_mul(size_of::<GpuVirtualTerrainNode>() as u64)
                .ok_or(VirtualTerrainGpuError::DirectoryCapacity)?;
            queue.write_buffer(
                &self.node_buffer,
                offset,
                bytemuck::cast_slice(&packed_nodes),
            );
            self.nodes.extend(packed_nodes);
            self.geometry_pages
                .resize(self.nodes.len(), GpuVirtualTerrainGeometryPage::default());
        }
        if !self.root_indices.is_empty() {
            queue.write_buffer(
                &self.root_buffer,
                0,
                bytemuck::cast_slice(&self.root_indices),
            );
        }
        Ok(())
    }

    /// Rebuilds the bounded GPU directory mirror after immutable region directories are retired.
    ///
    /// Node indices are an internal cache detail, so compaction is preferable to leaving tombstones
    /// that would eventually exhaust a long-travel session. Callers republish geometry records
    /// after this method because every index may have changed.
    pub(crate) fn synchronize_directory_set(
        &mut self,
        queue: &Queue,
        hierarchy: &VirtualTerrainHierarchy,
    ) -> Result<(), VirtualTerrainGpuError> {
        let directory_nodes = hierarchy.nodes().collect::<Vec<_>>();
        let roots = hierarchy.roots().collect::<BTreeSet<_>>();
        if directory_nodes.len() > self.capacity.max_directory_nodes {
            return Err(VirtualTerrainGpuError::DirectoryCapacity);
        }
        if roots.len() > self.capacity.max_roots {
            return Err(VirtualTerrainGpuError::RootCapacity);
        }
        let mut node_indices = BTreeMap::new();
        let mut node_keys = Vec::with_capacity(directory_nodes.len());
        for (index, node) in directory_nodes.iter().enumerate() {
            let index =
                u32::try_from(index).map_err(|_| VirtualTerrainGpuError::DirectoryCapacity)?;
            node_indices.insert(node.key, index);
            node_keys.push(node.key);
        }
        let prior_refined = hierarchy.refined_last_cut().collect::<BTreeSet<_>>();
        let mut nodes = Vec::with_capacity(directory_nodes.len());
        let mut root_indices = Vec::with_capacity(roots.len());
        for node in &directory_nodes {
            let mut packed = pack_node(node, &node_indices)?;
            let mut flags = static_flags(node);
            if hierarchy.resident_page(node.key).is_some() {
                flags |= NODE_RESIDENT;
            }
            if hierarchy.replacement_is_resident_and_coherent(node.key) {
                flags |= NODE_REPLACEMENT_COHERENT;
            }
            if prior_refined.contains(&node.key) {
                flags |= NODE_PRIOR_REFINED;
            }
            packed.maximum_flags[3] = flags as i32;
            if node.is_root {
                root_indices.push(
                    *node_indices
                        .get(&node.key)
                        .ok_or(VirtualTerrainGpuError::DirectoryCapacity)?,
                );
            }
            nodes.push(packed);
        }
        if !nodes.is_empty() {
            queue.write_buffer(&self.node_buffer, 0, bytemuck::cast_slice(&nodes));
        }
        if !root_indices.is_empty() {
            queue.write_buffer(&self.root_buffer, 0, bytemuck::cast_slice(&root_indices));
        }
        let geometry_pages = vec![GpuVirtualTerrainGeometryPage::default(); nodes.len()];
        if !geometry_pages.is_empty() {
            queue.write_buffer(
                &self.geometry_page_buffer,
                0,
                bytemuck::cast_slice(&geometry_pages),
            );
        }
        self.nodes = nodes;
        self.node_indices = node_indices;
        self.node_keys = node_keys;
        self.root_indices = root_indices;
        self.prior_refined = prior_refined;
        self.geometry_pages = geometry_pages;
        if let Ok(mut feedback) = self.feedback.lock() {
            *feedback = None;
        }
        Ok(())
    }

    pub(crate) fn bind_geometry_source(
        &mut self,
        device: &Device,
        source: &Buffer,
    ) -> Result<(), VirtualTerrainGpuError> {
        if source.size() > device.limits().max_storage_buffer_binding_size
            || !source.usage().contains(wgpu::BufferUsages::STORAGE)
        {
            return Err(VirtualTerrainGpuError::DeviceLimit);
        }
        if self.geometry_source_bound && self.geometry_source_bytes == source.size() {
            return Ok(());
        }
        self.compaction_bind_group = create_compaction_bind_group(
            device,
            &self.compaction_layout,
            &self.selected_buffer,
            &self.counter_buffer,
            &self.geometry_page_buffer,
            source,
            &self.compact_surface_buffer,
            &self.compact_triangle_buffer,
            &self.compaction_counter_buffer,
            &self.indirect_buffer,
        );
        self.geometry_source_bytes = source.size();
        self.geometry_source_bound = true;
        Ok(())
    }

    pub(crate) fn update_page_geometry(
        &mut self,
        queue: &Queue,
        key: TerrainPageKey,
        geometry: VirtualTerrainGpuGeometry,
    ) -> Result<(), VirtualTerrainGpuError> {
        let index = *self
            .node_indices
            .get(&key)
            .ok_or(VirtualTerrainGpuError::UnknownPage(key))?;
        let opaque_surface = self.pack_geometry_range(geometry.opaque_surface)?;
        let opaque_triangle = self.pack_geometry_range(geometry.opaque_triangle)?;
        let water_surface = self.pack_geometry_range(geometry.water_surface)?;
        let water_triangle = self.pack_geometry_range(geometry.water_triangle)?;
        let record = GpuVirtualTerrainGeometryPage {
            opaque_surface_offset: opaque_surface.0,
            opaque_surface_count: opaque_surface.1,
            opaque_triangle_offset: opaque_triangle.0,
            opaque_triangle_count: opaque_triangle.1,
            water_surface_offset: water_surface.0,
            water_surface_count: water_surface.1,
            water_triangle_offset: water_triangle.0,
            water_triangle_count: water_triangle.1,
        };
        let destination = self
            .geometry_pages
            .get_mut(index as usize)
            .ok_or(VirtualTerrainGpuError::UnknownPage(key))?;
        *destination = record;
        let offset = u64::from(index)
            .checked_mul(size_of::<GpuVirtualTerrainGeometryPage>() as u64)
            .ok_or(VirtualTerrainGpuError::GeometryCapacity)?;
        queue.write_buffer(
            &self.geometry_page_buffer,
            offset,
            bytemuck::bytes_of(&record),
        );
        Ok(())
    }

    fn pack_geometry_range(
        &self,
        range: VirtualTerrainGpuGeometryRange,
    ) -> Result<(u32, u32), VirtualTerrainGpuError> {
        if range.element_count == 0 {
            return Ok((0, 0));
        }
        if !range
            .source_offset_bytes
            .is_multiple_of(size_of::<u32>() as u64)
        {
            return Err(VirtualTerrainGpuError::InvalidGeometry);
        }
        let byte_count = u64::from(range.element_count)
            .checked_mul(GPU_GEOMETRY_ELEMENT_BYTES)
            .ok_or(VirtualTerrainGpuError::GeometryCapacity)?;
        if range
            .source_offset_bytes
            .checked_add(byte_count)
            .is_none_or(|end| end > self.geometry_source_bytes)
        {
            return Err(VirtualTerrainGpuError::GeometryCapacity);
        }
        let source_word_offset = u32::try_from(range.source_offset_bytes / size_of::<u32>() as u64)
            .map_err(|_| VirtualTerrainGpuError::GeometryCapacity)?;
        Ok((source_word_offset, range.element_count))
    }

    pub(crate) const fn compact_surface_buffer(&self) -> &Buffer {
        &self.compact_surface_buffer
    }

    pub(crate) const fn compact_triangle_buffer(&self) -> &Buffer {
        &self.compact_triangle_buffer
    }

    pub(crate) fn compact_water_surface_slice(&self) -> wgpu::BufferSlice<'_> {
        self.compact_surface_buffer
            .slice(VIRTUAL_TERRAIN_COMPACT_SURFACE_BYTES..)
    }

    pub(crate) fn compact_water_triangle_slice(&self) -> wgpu::BufferSlice<'_> {
        self.compact_triangle_buffer
            .slice(VIRTUAL_TERRAIN_COMPACT_TRIANGLE_BYTES..)
    }

    pub(crate) const fn indirect_buffer(&self) -> &Buffer {
        &self.indirect_buffer
    }

    pub(crate) fn update_page_residency(
        &mut self,
        queue: &Queue,
        hierarchy: &VirtualTerrainHierarchy,
        key: TerrainPageKey,
    ) -> Result<(), VirtualTerrainGpuError> {
        self.update_node_flags(queue, hierarchy, key)?;
        if let Some(parent) = key.parent()
            && hierarchy.directory_node(parent).is_some()
        {
            self.update_node_flags(queue, hierarchy, parent)?;
        }
        Ok(())
    }

    pub(crate) fn synchronize_prior_refinement(
        &mut self,
        queue: &Queue,
        hierarchy: &VirtualTerrainHierarchy,
    ) -> Result<(), VirtualTerrainGpuError> {
        let next = hierarchy.refined_last_cut().collect::<BTreeSet<_>>();
        let changed = self
            .prior_refined
            .symmetric_difference(&next)
            .copied()
            .collect::<Vec<_>>();
        self.prior_refined = next;
        for key in changed {
            self.update_node_flags(queue, hierarchy, key)?;
        }
        Ok(())
    }

    fn update_node_flags(
        &mut self,
        queue: &Queue,
        hierarchy: &VirtualTerrainHierarchy,
        key: TerrainPageKey,
    ) -> Result<(), VirtualTerrainGpuError> {
        let index = *self
            .node_indices
            .get(&key)
            .ok_or(VirtualTerrainGpuError::UnknownPage(key))?;
        let node = hierarchy
            .directory_node(key)
            .ok_or(VirtualTerrainGpuError::UnknownPage(key))?;
        let mut flags = static_flags(&node);
        if hierarchy.resident_page(key).is_some() {
            flags |= NODE_RESIDENT;
        }
        if hierarchy.replacement_is_resident_and_coherent(key) {
            flags |= NODE_REPLACEMENT_COHERENT;
        }
        if self.prior_refined.contains(&key) {
            flags |= NODE_PRIOR_REFINED;
        }
        let packed = self
            .nodes
            .get_mut(index as usize)
            .ok_or(VirtualTerrainGpuError::UnknownPage(key))?;
        packed.maximum_flags[3] = flags as i32;
        let offset = u64::from(index)
            .checked_mul(size_of::<GpuVirtualTerrainNode>() as u64)
            .and_then(|offset| {
                offset
                    .checked_add(std::mem::offset_of!(GpuVirtualTerrainNode, maximum_flags) as u64)
            })
            .and_then(|offset| offset.checked_add(3 * size_of::<i32>() as u64))
            .ok_or(VirtualTerrainGpuError::DirectoryCapacity)?;
        queue.write_buffer(&self.node_buffer, offset, bytemuck::bytes_of(&flags));
        Ok(())
    }

    pub(crate) fn encode_traversal(
        &mut self,
        queue: &Queue,
        encoder: &mut CommandEncoder,
        view: VirtualTerrainView,
        oracle_fingerprint: u64,
        timestamps: Option<VirtualTerrainGpuTimestampWrites<'_>>,
    ) -> Result<u64, VirtualTerrainGpuError> {
        let view = pack_view(
            view,
            self.root_indices.len(),
            self.nodes.len(),
            self.capacity,
        )?;
        let submission_id = self.next_submission_id;
        self.next_submission_id = self.next_submission_id.wrapping_add(1).max(1);
        let counters = GpuVirtualTerrainCounters {
            submission_id: split_u64(submission_id),
            oracle_fingerprint: split_u64(oracle_fingerprint),
            ..GpuVirtualTerrainCounters::default()
        };
        queue.write_buffer(&self.view_buffer, 0, bytemuck::bytes_of(&view));
        queue.write_buffer(&self.counter_buffer, 0, bytemuck::bytes_of(&counters));
        queue.write_buffer(
            &self.compaction_counter_buffer,
            0,
            bytemuck::bytes_of(&GpuVirtualTerrainCompactionCounters::reset()?),
        );
        if !self.root_indices.is_empty() {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("virtual terrain hierarchy traversal"),
                timestamp_writes: timestamps.map(|timestamps| wgpu::ComputePassTimestampWrites {
                    query_set: timestamps.query_set,
                    beginning_of_pass_write_index: Some(timestamps.traversal_first_query),
                    end_of_pass_write_index: Some(timestamps.traversal_first_query + 1),
                }),
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.dispatch_workgroups(
                (self.root_indices.len() as u32).div_ceil(GPU_TRAVERSAL_WORKGROUP_SIZE),
                1,
                1,
            );
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("virtual terrain selected geometry compaction"),
                timestamp_writes: timestamps.map(|timestamps| wgpu::ComputePassTimestampWrites {
                    query_set: timestamps.query_set,
                    beginning_of_pass_write_index: Some(timestamps.compaction_first_query),
                    end_of_pass_write_index: Some(timestamps.compaction_first_query + 1),
                }),
            });
            pass.set_bind_group(1, &self.compaction_bind_group, &[]);
            pass.set_pipeline(&self.compaction_prepare_pipeline);
            pass.dispatch_workgroups(1, 1, 1);
            pass.set_pipeline(&self.compaction_pipeline);
            pass.dispatch_workgroups_indirect(&self.indirect_buffer, 0);
            pass.set_pipeline(&self.compaction_finalize_pipeline);
            pass.dispatch_workgroups(1, 1, 1);
        }
        self.schedule_readback(encoder);
        Ok(submission_id)
    }

    fn schedule_readback(&mut self, encoder: &mut CommandEncoder) {
        let Some((slot_index, slot)) = (0..self.readback_slots.len())
            .map(|offset| (self.next_readback_slot + offset) % self.readback_slots.len())
            .find_map(|index| {
                let slot = self.readback_slots.get(index)?;
                slot.available
                    .compare_exchange(true, false, Ordering::AcqRel, Ordering::Acquire)
                    .ok()
                    .map(|_| (index, slot))
            })
        else {
            return;
        };
        self.next_readback_slot = (slot_index + 1) % self.readback_slots.len();
        let counter_bytes = size_of::<GpuVirtualTerrainCounters>() as u64;
        let compaction_bytes = size_of::<GpuVirtualTerrainCompactionCounters>() as u64;
        let selected_bytes = (self.capacity.max_selected_pages * size_of::<u32>()) as u64;
        let request_bytes = (self.capacity.max_feedback_pages * size_of::<u32>()) as u64;
        encoder.copy_buffer_to_buffer(&self.counter_buffer, 0, &slot.buffer, 0, counter_bytes);
        encoder.copy_buffer_to_buffer(
            &self.compaction_counter_buffer,
            0,
            &slot.buffer,
            counter_bytes,
            compaction_bytes,
        );
        encoder.copy_buffer_to_buffer(
            &self.selected_buffer,
            0,
            &slot.buffer,
            counter_bytes + compaction_bytes,
            selected_bytes,
        );
        encoder.copy_buffer_to_buffer(
            &self.request_buffer,
            0,
            &slot.buffer,
            counter_bytes + compaction_bytes + selected_bytes,
            request_bytes,
        );
        let callback_buffer = slot.buffer.clone();
        let available = Arc::clone(&slot.available);
        let feedback = Arc::clone(&self.feedback);
        let capacity = self.capacity;
        encoder.map_buffer_on_submit(&slot.buffer, wgpu::MapMode::Read, .., move |result| {
            if result.is_ok()
                && let Ok(mapped) = callback_buffer.get_mapped_range(..)
                && let Some(parsed) = parse_feedback(&mapped, capacity)
                && let Ok(mut destination) = feedback.lock()
            {
                let is_newer = destination.as_ref().is_none_or(|current| {
                    feedback_submission_id(&parsed) > feedback_submission_id(current)
                });
                if is_newer {
                    *destination = Some(parsed);
                }
            }
            callback_buffer.unmap();
            available.store(true, Ordering::Release);
        });
    }

    pub(crate) fn latest_feedback(&self) -> Option<GpuVirtualTerrainFeedback> {
        let raw = self.feedback.lock().ok()?.clone()?;
        Some(GpuVirtualTerrainFeedback {
            submission_id: feedback_submission_id(&raw),
            oracle_fingerprint: join_u64(raw.counters.oracle_fingerprint),
            selected_pages: raw
                .selected_indices
                .into_iter()
                .filter_map(|index| self.node_keys.get(index as usize).copied())
                .collect(),
            requested_pages: raw
                .requested_indices
                .into_iter()
                .filter_map(|index| self.node_keys.get(index as usize).copied())
                .collect(),
            ownerless_roots: raw.counters.ownerless_roots,
            visited_nodes: raw.counters.visited_nodes,
            overflow_flags: raw.counters.overflow_flags,
            stack_peak: raw.counters.stack_peak,
            compacted_surface_elements: raw.compaction.surface_elements,
            compacted_triangle_elements: raw.compaction.triangle_elements,
            compacted_water_surface_elements: raw.compaction.water_surface_elements,
            compacted_water_triangle_elements: raw.compaction.water_triangle_elements,
            compacted_pages: raw.compaction.copied_pages,
            compaction_overflow_flags: raw.compaction.overflow_flags,
        })
    }
}

const fn split_u64(value: u64) -> [u32; 2] {
    [value as u32, (value >> 32) as u32]
}

const fn join_u64(value: [u32; 2]) -> u64 {
    value[0] as u64 | ((value[1] as u64) << 32)
}

const fn feedback_submission_id(feedback: &RawGpuVirtualTerrainFeedback) -> u64 {
    join_u64(feedback.counters.submission_id)
}

fn pack_node(
    node: &TerrainHierarchyNode,
    indices: &BTreeMap<TerrainPageKey, u32>,
) -> Result<GpuVirtualTerrainNode, VirtualTerrainGpuError> {
    let bounds = node
        .key
        .bounds()
        .ok_or(VirtualTerrainGpuError::MissingChild(node.key))?;
    let children = if node.has_children {
        node.key
            .children()
            .ok_or(VirtualTerrainGpuError::MissingChild(node.key))?
            .map(|child| {
                indices
                    .get(&child)
                    .copied()
                    .ok_or(VirtualTerrainGpuError::MissingChild(child))
            })
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?
    } else {
        vec![INVALID_NODE; TERRAIN_PAGE_MAX_CHILDREN]
    };
    let positional_error = node
        .errors
        .geometric_millivoxels
        .max(node.errors.silhouette_millivoxels)
        .max(node.errors.material_boundary_millivoxels);
    Ok(GpuVirtualTerrainNode {
        minimum_level: [
            bounds.min.x,
            bounds.min.y,
            bounds.min.z,
            i32::from(node.key.level),
        ],
        maximum_flags: [
            bounds.max.x,
            bounds.max.y,
            bounds.max.z,
            static_flags(node) as i32,
        ],
        errors: [
            positional_error,
            node.errors.normal_milliradians,
            node.encoded_bytes,
            u32::from(node.representation as u8)
                | (u32::from(node.errors.unresolved_topology) << 8),
        ],
        children_low: children[..4].try_into().unwrap_or([INVALID_NODE; 4]),
        children_high: children[4..].try_into().unwrap_or([INVALID_NODE; 4]),
    })
}

fn static_flags(node: &TerrainHierarchyNode) -> u32 {
    (u32::from(node.has_children) * NODE_HAS_CHILDREN) | (u32::from(node.is_root) * NODE_IS_ROOT)
}

fn node_static_identity(node: &TerrainHierarchyNode) -> ([i32; 4], [i32; 3], [u32; 4]) {
    let bounds = node
        .key
        .bounds()
        .map(|bounds| bounds.min.as_array())
        .unwrap_or([0; 3]);
    (
        [bounds[0], bounds[1], bounds[2], i32::from(node.key.level)],
        node.key
            .bounds()
            .map(|bounds| bounds.max.as_array())
            .unwrap_or([0; 3]),
        [
            node.errors
                .geometric_millivoxels
                .max(node.errors.silhouette_millivoxels)
                .max(node.errors.material_boundary_millivoxels),
            node.errors.normal_milliradians,
            node.encoded_bytes,
            u32::from(node.representation as u8)
                | (u32::from(node.errors.unresolved_topology) << 8),
        ],
    )
}

fn packed_static_identity(node: GpuVirtualTerrainNode) -> ([i32; 4], [i32; 3], [u32; 4]) {
    (
        node.minimum_level,
        [
            node.maximum_flags[0],
            node.maximum_flags[1],
            node.maximum_flags[2],
        ],
        node.errors,
    )
}

fn pack_view(
    view: VirtualTerrainView,
    root_count: usize,
    node_count: usize,
    capacity: VirtualTerrainCapacity,
) -> Result<GpuVirtualTerrainView, VirtualTerrainGpuError> {
    if !view.validates()
        || root_count > u32::MAX as usize
        || node_count > capacity.max_directory_nodes
        || capacity.max_selected_pages > u32::MAX as usize
        || capacity.max_feedback_pages > u32::MAX as usize
        || capacity.max_traversal_nodes > u32::MAX as usize
    {
        return Err(VirtualTerrainGpuError::InvalidView);
    }
    let forward = normalize(view.camera_forward);
    let mut right = cross(forward, [0.0, 1.0, 0.0]);
    if length_squared(right) <= f64::EPSILON {
        right = [1.0, 0.0, 0.0];
    } else {
        right = normalize(right);
    }
    let up = normalize(cross(right, forward));
    let tangent_vertical = (view.vertical_fov_radians * 0.5).tan();
    let tangent_horizontal = tangent_vertical * view.aspect_ratio;
    let projection_scale = f64::from(view.viewport_height_pixels) / (2.0 * tangent_vertical);
    let f32x3 = |value: [f64; 3]| value.map(|component| component as f32);
    let camera = f32x3(view.camera_position_metres);
    let forward = f32x3(forward);
    let right = f32x3(right);
    let up = f32x3(up);
    Ok(GpuVirtualTerrainView {
        camera_near: [camera[0], camera[1], camera[2], view.near_metres as f32],
        forward_far: [forward[0], forward[1], forward[2], view.far_metres as f32],
        right_tangent_horizontal: [right[0], right[1], right[2], tangent_horizontal as f32],
        up_tangent_vertical: [up[0], up[1], up[2], tangent_vertical as f32],
        projection_thresholds: [
            projection_scale as f32,
            view.refine_above_pixels as f32,
            view.coarsen_below_pixels as f32,
            view.wet_specular_sensitivity as f32,
        ],
        counts_flags: [
            root_count as u32,
            capacity.max_selected_pages as u32,
            capacity.max_feedback_pages as u32,
            capacity.max_traversal_nodes as u32,
        ],
        options: [
            u32::from(view.force_exact_leaves),
            0,
            u32::try_from(node_count).map_err(|_| VirtualTerrainGpuError::InvalidView)?,
            0,
        ],
    })
}

fn parse_feedback(
    bytes: &[u8],
    capacity: VirtualTerrainCapacity,
) -> Option<RawGpuVirtualTerrainFeedback> {
    let counter_bytes = size_of::<GpuVirtualTerrainCounters>();
    let counters =
        bytemuck::try_from_bytes::<GpuVirtualTerrainCounters>(bytes.get(..counter_bytes)?)
            .ok()?
            .to_owned();
    let compaction_start = counter_bytes;
    let compaction_end =
        compaction_start.checked_add(size_of::<GpuVirtualTerrainCompactionCounters>())?;
    let compaction = bytemuck::try_from_bytes::<GpuVirtualTerrainCompactionCounters>(
        bytes.get(compaction_start..compaction_end)?,
    )
    .ok()?
    .to_owned();
    let selected_count = usize::try_from(counters.selected_count)
        .ok()?
        .min(capacity.max_selected_pages);
    let request_count = usize::try_from(counters.request_count)
        .ok()?
        .min(capacity.max_feedback_pages);
    let selected_region_bytes = capacity.max_selected_pages.checked_mul(size_of::<u32>())?;
    let selected_start = compaction_end;
    let selected_end = selected_start.checked_add(selected_region_bytes)?;
    let request_start = selected_end;
    let selected =
        bytemuck::try_cast_slice::<u8, u32>(bytes.get(selected_start..selected_end)?).ok()?;
    let request = bytemuck::try_cast_slice::<u8, u32>(bytes.get(request_start..)?).ok()?;
    Some(RawGpuVirtualTerrainFeedback {
        counters,
        compaction,
        selected_indices: selected.get(..selected_count)?.to_vec(),
        requested_indices: request.get(..request_count)?.to_vec(),
    })
}

fn readback_bytes(capacity: VirtualTerrainCapacity) -> Result<u64, VirtualTerrainGpuError> {
    (size_of::<GpuVirtualTerrainCounters>() as u64)
        .checked_add(size_of::<GpuVirtualTerrainCompactionCounters>() as u64)
        .and_then(|bytes| bytes.checked_add(buffer_bytes::<u32>(capacity.max_selected_pages).ok()?))
        .and_then(|bytes| bytes.checked_add(buffer_bytes::<u32>(capacity.max_feedback_pages).ok()?))
        .ok_or(VirtualTerrainGpuError::DeviceLimit)
}

fn buffer_bytes<T>(count: usize) -> Result<u64, VirtualTerrainGpuError> {
    let bytes = count
        .checked_mul(size_of::<T>())
        .ok_or(VirtualTerrainGpuError::DeviceLimit)?;
    u64::try_from(bytes.max(size_of::<T>())).map_err(|_| VirtualTerrainGpuError::DeviceLimit)
}

fn element_capacity(bytes: u64) -> Result<u32, VirtualTerrainGpuError> {
    u32::try_from(bytes / GPU_GEOMETRY_ELEMENT_BYTES)
        .map_err(|_| VirtualTerrainGpuError::DeviceLimit)
}

fn word_offset(bytes: u64) -> Result<u32, VirtualTerrainGpuError> {
    if !bytes.is_multiple_of(size_of::<u32>() as u64) {
        return Err(VirtualTerrainGpuError::DeviceLimit);
    }
    u32::try_from(bytes / size_of::<u32>() as u64).map_err(|_| VirtualTerrainGpuError::DeviceLimit)
}

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn storage_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
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

#[allow(
    clippy::too_many_arguments,
    reason = "the fixed WebGPU layout has eight independently bounded storage roles"
)]
fn create_compaction_bind_group(
    device: &Device,
    layout: &wgpu::BindGroupLayout,
    selected: &Buffer,
    traversal_counters: &Buffer,
    geometry_pages: &Buffer,
    geometry_source: &Buffer,
    compact_surfaces: &Buffer,
    compact_triangles: &Buffer,
    compaction_counters: &Buffer,
    indirect_commands: &Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("virtual terrain compaction bind group"),
        layout,
        entries: &[
            entire_entry(0, selected),
            entire_entry(1, traversal_counters),
            entire_entry(2, geometry_pages),
            entire_entry(3, geometry_source),
            entire_entry(4, compact_surfaces),
            entire_entry(5, compact_triangles),
            entire_entry(6, compaction_counters),
            entire_entry(7, indirect_commands),
        ],
    })
}

fn dot(left: [f64; 3], right: [f64; 3]) -> f64 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn cross(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

fn length_squared(vector: [f64; 3]) -> f64 {
    dot(vector, vector)
}

fn normalize(vector: [f64; 3]) -> [f64; 3] {
    let inverse = length_squared(vector).sqrt().recip();
    [
        vector[0] * inverse,
        vector[1] * inverse,
        vector[2] * inverse,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feedback_parser_clamps_untrusted_gpu_counts_to_fixed_regions() {
        let capacity = VirtualTerrainCapacity {
            max_directories: 1,
            max_roots: 1,
            max_directory_nodes: 8,
            max_resident_pages: 8,
            max_resident_encoded_bytes: 1,
            max_resident_primitives: 1,
            max_selected_pages: 2,
            max_traversal_nodes: 8,
            max_feedback_pages: 1,
        };
        let counters = GpuVirtualTerrainCounters {
            selected_count: u32::MAX,
            request_count: u32::MAX,
            ownerless_roots: 3,
            visited_nodes: 4,
            overflow_flags: 3,
            stack_peak: 192,
            submission_id: split_u64(0x1234_5678_9abc_def0),
            oracle_fingerprint: split_u64(0xfedc_ba98_7654_3210),
        };
        let mut bytes = bytemuck::bytes_of(&counters).to_vec();
        let compaction = GpuVirtualTerrainCompactionCounters {
            surface_elements: 11,
            triangle_elements: 12,
            water_surface_elements: 13,
            water_triangle_elements: 14,
            copied_pages: 2,
            overflow_flags: 4,
            ..GpuVirtualTerrainCompactionCounters::reset().unwrap()
        };
        bytes.extend_from_slice(bytemuck::bytes_of(&compaction));
        bytes.extend_from_slice(bytemuck::cast_slice(&[7u32, 8]));
        bytes.extend_from_slice(bytemuck::cast_slice(&[9u32]));
        let parsed = parse_feedback(&bytes, capacity).expect("bounded feedback");
        assert_eq!(feedback_submission_id(&parsed), 0x1234_5678_9abc_def0);
        assert_eq!(
            join_u64(parsed.counters.oracle_fingerprint),
            0xfedc_ba98_7654_3210
        );
        assert_eq!(parsed.selected_indices, [7, 8]);
        assert_eq!(parsed.requested_indices, [9]);
        assert_eq!(parsed.counters.ownerless_roots, 3);
        assert_eq!(parsed.compaction, compaction);
    }

    #[test]
    fn compact_stream_partitions_are_bounded_disjoint_and_vertex_aligned() {
        let counters = GpuVirtualTerrainCompactionCounters::reset().unwrap();
        assert_eq!(
            u64::from(counters.surface_water_word_offset) * size_of::<u32>() as u64,
            VIRTUAL_TERRAIN_COMPACT_SURFACE_BYTES
        );
        assert_eq!(
            u64::from(counters.triangle_water_word_offset) * size_of::<u32>() as u64,
            VIRTUAL_TERRAIN_COMPACT_TRIANGLE_BYTES
        );
        assert!(
            u64::from(counters.surface_water_word_offset) * size_of::<u32>() as u64
                + u64::from(counters.water_surface_capacity) * GPU_GEOMETRY_ELEMENT_BYTES
                <= VIRTUAL_TERRAIN_SURFACE_BUFFER_BYTES
        );
        assert!(
            u64::from(counters.triangle_water_word_offset) * size_of::<u32>() as u64
                + u64::from(counters.water_triangle_capacity) * GPU_GEOMETRY_ELEMENT_BYTES
                <= VIRTUAL_TERRAIN_TRIANGLE_BUFFER_BYTES
        );
        assert_eq!(VIRTUAL_TERRAIN_WATER_SURFACE_INDIRECT_OFFSET, 48);
        assert_eq!(VIRTUAL_TERRAIN_WATER_TRIANGLE_INDIRECT_OFFSET, 64);
    }

    #[test]
    fn view_pack_keeps_negative_world_coordinates_and_hard_caps() {
        let capacity = VirtualTerrainCapacity {
            max_directories: 4,
            max_roots: 8,
            max_directory_nodes: 64,
            max_resident_pages: 64,
            max_resident_encoded_bytes: 1,
            max_resident_primitives: 1,
            max_selected_pages: 32,
            max_traversal_nodes: 48,
            max_feedback_pages: 7,
        };
        let packed = pack_view(
            VirtualTerrainView {
                camera_position_metres: [-1961.5, 54.0, -1616.0],
                camera_forward: [-1.0, -0.2, 0.4],
                vertical_fov_radians: 1.0,
                aspect_ratio: 2.0,
                viewport_height_pixels: 1814,
                near_metres: 0.05,
                far_metres: 3_200.0,
                refine_above_pixels: 0.65,
                coarsen_below_pixels: 0.35,
                wet_specular_sensitivity: 1.0,
                force_exact_leaves: false,
            },
            3,
            64,
            capacity,
        )
        .expect("view pack");
        assert_eq!(packed.camera_near[0], -1961.5);
        assert_eq!(packed.camera_near[2], -1616.0);
        assert_eq!(packed.counts_flags, [3, 32, 7, 48]);
        assert_eq!(packed.options, [0, 0, 64, 0]);
        assert!(packed.projection_thresholds[0] > 0.0);
    }
}
