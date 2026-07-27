//! Disposable, source-neutral prototypes for selecting the virtual microvoxel representation.
//!
//! This module deliberately shares only canonical integer occupancy with production code. Every
//! candidate is built from the same frozen volume and queried by the same rays, so a fast candidate
//! cannot hide a different source, camera, or correctness definition inside its benchmark.

use crate::{
    CanonicalFaceKey, FaceAxis, Material, VoxelBounds, VoxelCoord, canonical_exposed_faces,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::mem::size_of;
use std::time::{Duration, Instant};

const BRICK_EDGE: i32 = 8;
const CLUSTER_QUADS: usize = 128;
const RAY_EPSILON: f64 = 1.0e-7;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BakeoffError {
    InvalidBounds,
    WrongMaterialCount,
    VolumeTooLarge,
    InvalidCamera,
    InvalidRayGrid,
    EmptyCandidates,
}

impl fmt::Display for BakeoffError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBounds => formatter.write_str("bake-off volume bounds are invalid"),
            Self::WrongMaterialCount => {
                formatter.write_str("bake-off material count does not match its volume")
            }
            Self::VolumeTooLarge => formatter.write_str("bake-off volume is not addressable"),
            Self::InvalidCamera => formatter.write_str("bake-off camera is not finite"),
            Self::InvalidRayGrid => formatter.write_str("bake-off ray grid is invalid"),
            Self::EmptyCandidates => formatter.write_str("bake-off candidate set is empty"),
        }
    }
}

impl std::error::Error for BakeoffError {}

/// Frozen canonical occupancy in X-fastest, then Y, then Z order.
#[derive(Clone, Debug)]
pub struct BakeoffVolume {
    bounds: VoxelBounds,
    shape: [usize; 3],
    materials: Box<[Material]>,
}

impl BakeoffVolume {
    pub fn new(bounds: VoxelBounds, materials: Vec<Material>) -> Result<Self, BakeoffError> {
        let shape = shape(bounds)?;
        let count = shape
            .into_iter()
            .try_fold(1usize, usize::checked_mul)
            .ok_or(BakeoffError::VolumeTooLarge)?;
        if materials.len() != count {
            return Err(BakeoffError::WrongMaterialCount);
        }
        Ok(Self {
            bounds,
            shape,
            materials: materials.into_boxed_slice(),
        })
    }

    pub fn from_sampler(
        bounds: VoxelBounds,
        mut sample: impl FnMut(VoxelCoord) -> Material,
    ) -> Result<Self, BakeoffError> {
        let shape = shape(bounds)?;
        let count = shape
            .into_iter()
            .try_fold(1usize, usize::checked_mul)
            .ok_or(BakeoffError::VolumeTooLarge)?;
        let mut materials = Vec::with_capacity(count);
        for z in bounds.min.z..bounds.max.z {
            for y in bounds.min.y..bounds.max.y {
                for x in bounds.min.x..bounds.max.x {
                    materials.push(sample(VoxelCoord::new(x, y, z)));
                }
            }
        }
        Self::new(bounds, materials)
    }

    pub const fn bounds(&self) -> VoxelBounds {
        self.bounds
    }

    pub const fn shape(&self) -> [usize; 3] {
        self.shape
    }

    pub fn materials(&self) -> &[Material] {
        &self.materials
    }

    pub fn material_at(&self, coord: VoxelCoord) -> Material {
        self.index(coord)
            .and_then(|index| self.materials.get(index).copied())
            .unwrap_or(Material::Air)
    }

    pub fn logical_bytes(&self) -> usize {
        size_of::<Self>() + self.materials.len() * size_of::<Material>()
    }

    fn index(&self, coord: VoxelCoord) -> Option<usize> {
        if !self.bounds.contains(coord) {
            return None;
        }
        let x = usize::try_from(i64::from(coord.x) - i64::from(self.bounds.min.x)).ok()?;
        let y = usize::try_from(i64::from(coord.y) - i64::from(self.bounds.min.y)).ok()?;
        let z = usize::try_from(i64::from(coord.z) - i64::from(self.bounds.min.z)).ok()?;
        x.checked_add(y.checked_mul(self.shape[0])?)?
            .checked_add(z.checked_mul(self.shape[0].checked_mul(self.shape[1])?)?)
    }
}

fn shape(bounds: VoxelBounds) -> Result<[usize; 3], BakeoffError> {
    let difference = |minimum: i32, maximum: i32| {
        usize::try_from(i64::from(maximum) - i64::from(minimum))
            .map_err(|_| BakeoffError::VolumeTooLarge)
    };
    let result = [
        difference(bounds.min.x, bounds.max.x)?,
        difference(bounds.min.y, bounds.max.y)?,
        difference(bounds.min.z, bounds.max.z)?,
    ];
    if result.contains(&0) {
        return Err(BakeoffError::InvalidBounds);
    }
    Ok(result)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BakeoffCandidateKind {
    ExactGreedy,
    SteppedSurface,
    ClusteredVirtualGeometry,
    SparseBrickRayCaster,
}

impl BakeoffCandidateKind {
    pub const ALL: [Self; 4] = [
        Self::ExactGreedy,
        Self::SteppedSurface,
        Self::ClusteredVirtualGeometry,
        Self::SparseBrickRayCaster,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::ExactGreedy => "exact-greedy",
            Self::SteppedSurface => "stepped-surface",
            Self::ClusteredVirtualGeometry => "clustered-virtual-geometry",
            Self::SparseBrickRayCaster => "sparse-brick-ray-caster",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BakeoffHit {
    pub distance_voxels: f64,
    pub material: Material,
    pub face: u8,
}

#[derive(Clone, Copy, Debug)]
struct Ray {
    origin: [f64; 3],
    direction: [f64; 3],
}

#[derive(Clone, Copy, Debug)]
pub struct BakeoffCamera {
    /// Canonical voxel coordinates, not metres.
    pub eye_voxels: [f64; 3],
    pub yaw_radians: f64,
    pub pitch_radians: f64,
    pub vertical_fov_radians: f64,
    pub aspect_ratio: f64,
}

impl BakeoffCamera {
    fn is_valid(self) -> bool {
        self.eye_voxels.into_iter().all(f64::is_finite)
            && self.yaw_radians.is_finite()
            && self.pitch_radians.is_finite()
            && self.vertical_fov_radians.is_finite()
            && (0.01..3.13).contains(&self.vertical_fov_radians)
            && self.aspect_ratio.is_finite()
            && self.aspect_ratio > 0.0
    }

    fn ray(self, x: u32, y: u32, width: u32, height: u32) -> Ray {
        let forward = [
            self.yaw_radians.sin() * self.pitch_radians.cos(),
            self.pitch_radians.sin(),
            -self.yaw_radians.cos() * self.pitch_radians.cos(),
        ];
        let right = [self.yaw_radians.cos(), 0.0, self.yaw_radians.sin()];
        let up = cross(right, forward);
        let tangent = (self.vertical_fov_radians * 0.5).tan();
        let screen_x =
            ((f64::from(x) + 0.5) / f64::from(width) * 2.0 - 1.0) * tangent * self.aspect_ratio;
        let screen_y = (1.0 - (f64::from(y) + 0.5) / f64::from(height) * 2.0) * tangent;
        Ray {
            origin: self.eye_voxels,
            direction: normalize([
                forward[0] + right[0] * screen_x + up[0] * screen_y,
                forward[1] + right[1] * screen_x + up[1] * screen_y,
                forward[2] + right[2] * screen_x + up[2] * screen_y,
            ]),
        }
    }
}

fn cross(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

fn normalize(vector: [f64; 3]) -> [f64; 3] {
    let length = vector
        .into_iter()
        .map(|value| value * value)
        .sum::<f64>()
        .sqrt();
    vector.map(|value| value / length)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct GreedyQuad {
    axis: FaceAxis,
    plane: i32,
    u: i32,
    v: i32,
    width: u32,
    height: u32,
    positive: bool,
    material: Material,
}

/// Fixed-layout quad payload used only by the isolated GPU prototype executable.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BakeoffGpuQuad {
    pub axis: i32,
    pub plane: i32,
    pub u: i32,
    pub v: i32,
    pub width: u32,
    pub height: u32,
    pub positive: u32,
    pub material_id: u32,
}

impl From<GreedyQuad> for BakeoffGpuQuad {
    fn from(quad: GreedyQuad) -> Self {
        Self {
            axis: quad.axis as i32,
            plane: quad.plane,
            u: quad.u,
            v: quad.v,
            width: quad.width,
            height: quad.height,
            positive: u32::from(quad.positive),
            material_id: u32::from(quad.material.id()),
        }
    }
}

impl GreedyQuad {
    fn bounds(self) -> FloatBounds {
        let plane = f64::from(self.plane);
        let u0 = f64::from(self.u);
        let v0 = f64::from(self.v);
        let u1 = u0 + f64::from(self.width);
        let v1 = v0 + f64::from(self.height);
        match self.axis {
            FaceAxis::X => FloatBounds::new([plane, u0, v0], [plane, u1, v1]),
            FaceAxis::Y => FloatBounds::new([u0, plane, v0], [u1, plane, v1]),
            FaceAxis::Z => FloatBounds::new([u0, v0, plane], [u1, v1, plane]),
        }
    }

    fn intersect(self, ray: Ray) -> Option<BakeoffHit> {
        let axis = self.axis as usize;
        let direction = ray.direction[axis];
        let normal = if self.positive { 1.0 } else { -1.0 };
        if direction * normal >= -RAY_EPSILON {
            return None;
        }
        let distance = (f64::from(self.plane) - ray.origin[axis]) / direction;
        if distance <= RAY_EPSILON {
            return None;
        }
        let point = [
            ray.origin[0] + ray.direction[0] * distance,
            ray.origin[1] + ray.direction[1] * distance,
            ray.origin[2] + ray.direction[2] * distance,
        ];
        let [u, v] = match self.axis {
            FaceAxis::X => [point[1], point[2]],
            FaceAxis::Y => [point[0], point[2]],
            FaceAxis::Z => [point[0], point[1]],
        };
        if u + RAY_EPSILON < f64::from(self.u)
            || v + RAY_EPSILON < f64::from(self.v)
            || u - RAY_EPSILON > f64::from(self.u) + f64::from(self.width)
            || v - RAY_EPSILON > f64::from(self.v) + f64::from(self.height)
        {
            return None;
        }
        Some(BakeoffHit {
            distance_voxels: distance,
            material: self.material,
            face: face_index(self.axis, self.positive),
        })
    }
}

fn face_index(axis: FaceAxis, positive: bool) -> u8 {
    axis as u8 * 2 + u8::from(positive)
}

#[derive(Clone, Copy, Debug)]
struct FloatBounds {
    minimum: [f64; 3],
    maximum: [f64; 3],
}

impl FloatBounds {
    const fn new(minimum: [f64; 3], maximum: [f64; 3]) -> Self {
        Self { minimum, maximum }
    }

    fn empty() -> Self {
        Self {
            minimum: [f64::INFINITY; 3],
            maximum: [f64::NEG_INFINITY; 3],
        }
    }

    fn include(mut self, other: Self) -> Self {
        for axis in 0..3 {
            self.minimum[axis] = self.minimum[axis].min(other.minimum[axis]);
            self.maximum[axis] = self.maximum[axis].max(other.maximum[axis]);
        }
        self
    }

    fn intersects(self, ray: Ray, maximum_distance: f64) -> bool {
        let mut near: f64 = 0.0;
        let mut far = maximum_distance;
        for axis in 0..3 {
            let direction = ray.direction[axis];
            if direction.abs() <= RAY_EPSILON {
                if ray.origin[axis] < self.minimum[axis] || ray.origin[axis] > self.maximum[axis] {
                    return false;
                }
                continue;
            }
            let inverse = 1.0 / direction;
            let first = (self.minimum[axis] - ray.origin[axis]) * inverse;
            let second = (self.maximum[axis] - ray.origin[axis]) * inverse;
            near = near.max(first.min(second));
            far = far.min(first.max(second));
            if far < near {
                return false;
            }
        }
        far > RAY_EPSILON
    }
}

#[derive(Clone, Debug)]
struct ExactGreedy {
    quads: Vec<GreedyQuad>,
}

impl ExactGreedy {
    fn build(volume: &BakeoffVolume) -> Self {
        let faces = canonical_exposed_faces(volume.bounds, |coord| volume.material_at(coord));
        Self {
            quads: merge_faces(&faces),
        }
    }

    fn trace(&self, ray: Ray) -> Option<BakeoffHit> {
        closest(self.quads.iter().filter_map(|quad| quad.intersect(ray)))
    }

    fn logical_bytes(&self) -> usize {
        size_of::<Self>() + self.quads.capacity() * size_of::<GreedyQuad>()
    }
}

#[derive(Clone, Copy, Debug)]
struct HeightColumn {
    top_exclusive: i32,
    first_run: u32,
    run_count: u16,
    valid: bool,
}

#[derive(Clone, Copy, Debug)]
struct MaterialRun {
    min_y: i32,
    max_y_exclusive: i32,
    material: Material,
}

#[derive(Clone, Debug)]
struct SteppedSurface {
    bounds: VoxelBounds,
    shape_x: usize,
    columns: Vec<HeightColumn>,
    material_runs: Vec<MaterialRun>,
    invalid_columns: usize,
}

impl SteppedSurface {
    fn build(volume: &BakeoffVolume) -> Self {
        let [shape_x, _, shape_z] = volume.shape;
        let mut columns = Vec::with_capacity(shape_x * shape_z);
        let mut material_runs = Vec::new();
        let mut invalid_columns = 0;
        for z in volume.bounds.min.z..volume.bounds.max.z {
            for x in volume.bounds.min.x..volume.bounds.max.x {
                let mut top_exclusive = volume.bounds.min.y;
                let mut saw_air_after_solid = false;
                let mut valid = true;
                let first_run = material_runs.len();
                let mut active_run: Option<MaterialRun> = None;
                for y in volume.bounds.min.y..volume.bounds.max.y {
                    let material = volume.material_at(VoxelCoord::new(x, y, z));
                    if material.is_renderable() {
                        if saw_air_after_solid {
                            valid = false;
                        }
                        top_exclusive = y.saturating_add(1);
                        match active_run {
                            Some(mut run) if run.material == material => {
                                run.max_y_exclusive = y.saturating_add(1);
                                active_run = Some(run);
                            }
                            Some(run) => {
                                material_runs.push(run);
                                active_run = Some(MaterialRun {
                                    min_y: y,
                                    max_y_exclusive: y.saturating_add(1),
                                    material,
                                });
                            }
                            None => {
                                active_run = Some(MaterialRun {
                                    min_y: y,
                                    max_y_exclusive: y.saturating_add(1),
                                    material,
                                });
                            }
                        }
                    } else if top_exclusive > volume.bounds.min.y {
                        saw_air_after_solid = true;
                    }
                }
                if let Some(run) = active_run {
                    material_runs.push(run);
                }
                if !valid {
                    invalid_columns += 1;
                }
                let run_count = material_runs.len().saturating_sub(first_run);
                columns.push(HeightColumn {
                    top_exclusive,
                    first_run: u32::try_from(first_run).unwrap_or(u32::MAX),
                    run_count: u16::try_from(run_count).unwrap_or(u16::MAX),
                    valid,
                });
            }
        }
        Self {
            bounds: volume.bounds,
            shape_x,
            columns,
            material_runs,
            invalid_columns,
        }
    }

    fn column(&self, x: i32, z: i32) -> Option<HeightColumn> {
        if x < self.bounds.min.x
            || x >= self.bounds.max.x
            || z < self.bounds.min.z
            || z >= self.bounds.max.z
        {
            return None;
        }
        let local_x = usize::try_from(i64::from(x) - i64::from(self.bounds.min.x)).ok()?;
        let local_z = usize::try_from(i64::from(z) - i64::from(self.bounds.min.z)).ok()?;
        self.columns.get(local_x + local_z * self.shape_x).copied()
    }

    fn material_at(&self, coord: VoxelCoord) -> Material {
        self.column(coord.x, coord.z)
            .map_or(Material::Air, |column| {
                if column.valid && coord.y >= self.bounds.min.y && coord.y < column.top_exclusive {
                    let start = column.first_run as usize;
                    let end = start.saturating_add(usize::from(column.run_count));
                    self.material_runs
                        .get(start..end)
                        .unwrap_or_default()
                        .iter()
                        .find(|run| coord.y >= run.min_y && coord.y < run.max_y_exclusive)
                        .map_or(Material::Air, |run| run.material)
                } else {
                    Material::Air
                }
            })
    }

    fn trace(&self, ray: Ray) -> Option<BakeoffHit> {
        trace_voxel_grid(ray, self.bounds, |coord| self.material_at(coord))
    }

    fn logical_bytes(&self) -> usize {
        size_of::<Self>()
            + self.columns.capacity() * size_of::<HeightColumn>()
            + self.material_runs.capacity() * size_of::<MaterialRun>()
    }
}

#[derive(Clone, Copy, Debug)]
struct ClusterNode {
    bounds: FloatBounds,
    first: u32,
    second: u32,
    count: u16,
    leaf: bool,
}

#[derive(Clone, Debug)]
struct ClusteredGeometry {
    quads: Vec<GreedyQuad>,
    ordered_quad_indices: Vec<u32>,
    nodes: Vec<ClusterNode>,
}

impl ClusteredGeometry {
    fn build(volume: &BakeoffVolume) -> Self {
        let quads = ExactGreedy::build(volume).quads;
        let mut ordered_quad_indices = (0..quads.len())
            .filter_map(|index| u32::try_from(index).ok())
            .collect::<Vec<_>>();
        ordered_quad_indices.sort_unstable_by_key(|index| {
            let quad = quads[*index as usize];
            (
                quad.bounds().minimum[0] as i32,
                quad.bounds().minimum[2] as i32,
                quad.bounds().minimum[1] as i32,
                quad.axis,
            )
        });
        let mut nodes = Vec::new();
        if !ordered_quad_indices.is_empty() {
            build_cluster_node(
                &quads,
                &ordered_quad_indices,
                0,
                ordered_quad_indices.len(),
                &mut nodes,
            );
        }
        Self {
            quads,
            ordered_quad_indices,
            nodes,
        }
    }

    fn trace(&self, ray: Ray) -> Option<BakeoffHit> {
        let mut closest_hit: Option<BakeoffHit> = None;
        let mut stack = self
            .nodes
            .is_empty()
            .then(Vec::new)
            .unwrap_or_else(|| vec![0usize]);
        while let Some(node_index) = stack.pop() {
            let Some(node) = self.nodes.get(node_index).copied() else {
                continue;
            };
            let maximum = closest_hit.map_or(f64::INFINITY, |hit| hit.distance_voxels);
            if !node.bounds.intersects(ray, maximum) {
                continue;
            }
            if node.leaf {
                let start = node.first as usize;
                let end = start.saturating_add(usize::from(node.count));
                for index in self
                    .ordered_quad_indices
                    .get(start..end)
                    .unwrap_or_default()
                {
                    let Some(hit) = self
                        .quads
                        .get(*index as usize)
                        .and_then(|quad| quad.intersect(ray))
                    else {
                        continue;
                    };
                    if closest_hit
                        .is_none_or(|current| hit.distance_voxels < current.distance_voxels)
                    {
                        closest_hit = Some(hit);
                    }
                }
            } else {
                stack.push(node.first as usize);
                if node.count > 1 {
                    stack.push(node.second as usize);
                }
            }
        }
        closest_hit
    }

    fn logical_bytes(&self) -> usize {
        size_of::<Self>()
            + self.quads.capacity() * size_of::<GreedyQuad>()
            + self.ordered_quad_indices.capacity() * size_of::<u32>()
            + self.nodes.capacity() * size_of::<ClusterNode>()
    }
}

fn build_cluster_node(
    quads: &[GreedyQuad],
    order: &[u32],
    start: usize,
    count: usize,
    nodes: &mut Vec<ClusterNode>,
) -> usize {
    let index = nodes.len();
    nodes.push(ClusterNode {
        bounds: FloatBounds::empty(),
        first: 0,
        second: 0,
        count: 0,
        leaf: false,
    });
    let bounds = order[start..start + count]
        .iter()
        .filter_map(|quad| quads.get(*quad as usize))
        .fold(FloatBounds::empty(), |bounds, quad| {
            bounds.include(quad.bounds())
        });
    if count <= CLUSTER_QUADS {
        nodes[index] = ClusterNode {
            bounds,
            first: u32::try_from(start).unwrap_or(u32::MAX),
            second: 0,
            count: u16::try_from(count).unwrap_or(u16::MAX),
            leaf: true,
        };
        return index;
    }
    let left_count = count / 2;
    let left = build_cluster_node(quads, order, start, left_count, nodes);
    let right = build_cluster_node(quads, order, start + left_count, count - left_count, nodes);
    nodes[index] = ClusterNode {
        bounds,
        first: u32::try_from(left).unwrap_or(u32::MAX),
        second: u32::try_from(right).unwrap_or(u32::MAX),
        count: 2,
        leaf: false,
    };
    index
}

#[derive(Clone, Debug)]
struct SparseBrick {
    occupancy: [u64; 8],
    palette: Vec<Material>,
    material_indices: Vec<u8>,
}

impl SparseBrick {
    fn material_at(&self, local_index: usize) -> Material {
        let word = local_index / 64;
        let bit = local_index % 64;
        if self.occupancy[word] & (1u64 << bit) == 0 {
            return Material::Air;
        }
        let rank = self.occupancy[..word]
            .iter()
            .map(|bits| bits.count_ones() as usize)
            .sum::<usize>()
            + (self.occupancy[word] & ((1u64 << bit) - 1)).count_ones() as usize;
        self.material_indices
            .get(rank)
            .and_then(|index| self.palette.get(usize::from(*index)))
            .copied()
            .unwrap_or(Material::Air)
    }

    fn logical_bytes(&self) -> usize {
        size_of::<Self>()
            + self.palette.capacity() * size_of::<Material>()
            + self.material_indices.capacity()
    }
}

#[derive(Clone, Debug)]
struct SparseBrickRayCaster {
    bounds: VoxelBounds,
    bricks: BTreeMap<[i32; 3], SparseBrick>,
}

impl SparseBrickRayCaster {
    fn build(volume: &BakeoffVolume) -> Self {
        let mut raw = BTreeMap::<[i32; 3], Vec<(usize, Material)>>::new();
        for z in volume.bounds.min.z..volume.bounds.max.z {
            for y in volume.bounds.min.y..volume.bounds.max.y {
                for x in volume.bounds.min.x..volume.bounds.max.x {
                    let coord = VoxelCoord::new(x, y, z);
                    let material = volume.material_at(coord);
                    if !material.is_renderable() {
                        continue;
                    }
                    let brick = [
                        x.div_euclid(BRICK_EDGE),
                        y.div_euclid(BRICK_EDGE),
                        z.div_euclid(BRICK_EDGE),
                    ];
                    let local = [
                        x.rem_euclid(BRICK_EDGE) as usize,
                        y.rem_euclid(BRICK_EDGE) as usize,
                        z.rem_euclid(BRICK_EDGE) as usize,
                    ];
                    let index = local[0]
                        + local[1] * BRICK_EDGE as usize
                        + local[2] * BRICK_EDGE as usize * BRICK_EDGE as usize;
                    raw.entry(brick).or_default().push((index, material));
                }
            }
        }
        let bricks = raw
            .into_iter()
            .map(|(coord, mut voxels)| {
                voxels.sort_unstable_by_key(|(index, _)| *index);
                let mut occupancy = [0u64; 8];
                let mut palette = Vec::new();
                let mut material_indices = Vec::with_capacity(voxels.len());
                for (index, material) in voxels {
                    occupancy[index / 64] |= 1u64 << (index % 64);
                    let palette_index = palette
                        .iter()
                        .position(|candidate| *candidate == material)
                        .unwrap_or_else(|| {
                            palette.push(material);
                            palette.len() - 1
                        });
                    material_indices.push(u8::try_from(palette_index).unwrap_or(u8::MAX));
                }
                (
                    coord,
                    SparseBrick {
                        occupancy,
                        palette,
                        material_indices,
                    },
                )
            })
            .collect();
        Self {
            bounds: volume.bounds,
            bricks,
        }
    }

    fn material_at(&self, coord: VoxelCoord) -> Material {
        if !self.bounds.contains(coord) {
            return Material::Air;
        }
        let brick_coord = [
            coord.x.div_euclid(BRICK_EDGE),
            coord.y.div_euclid(BRICK_EDGE),
            coord.z.div_euclid(BRICK_EDGE),
        ];
        let local = [
            coord.x.rem_euclid(BRICK_EDGE) as usize,
            coord.y.rem_euclid(BRICK_EDGE) as usize,
            coord.z.rem_euclid(BRICK_EDGE) as usize,
        ];
        let index = local[0]
            + local[1] * BRICK_EDGE as usize
            + local[2] * BRICK_EDGE as usize * BRICK_EDGE as usize;
        self.bricks
            .get(&brick_coord)
            .map_or(Material::Air, |brick| brick.material_at(index))
    }

    fn trace(&self, ray: Ray) -> Option<BakeoffHit> {
        trace_voxel_grid(ray, self.bounds, |coord| self.material_at(coord))
    }

    fn logical_bytes(&self) -> usize {
        size_of::<Self>()
            + self.bricks.len() * size_of::<([i32; 3], SparseBrick)>()
            + self
                .bricks
                .values()
                .map(SparseBrick::logical_bytes)
                .sum::<usize>()
    }
}

#[derive(Clone, Debug)]
enum CandidateStorage {
    ExactGreedy(ExactGreedy),
    SteppedSurface(SteppedSurface),
    ClusteredVirtualGeometry(ClusteredGeometry),
    SparseBrickRayCaster(SparseBrickRayCaster),
}

#[derive(Clone, Debug)]
pub struct BakeoffCandidate {
    pub kind: BakeoffCandidateKind,
    pub build_time: Duration,
    pub logical_bytes: usize,
    pub primitive_count: usize,
    pub volumetric_exception_columns: usize,
    storage: CandidateStorage,
}

impl BakeoffCandidate {
    pub fn build(kind: BakeoffCandidateKind, volume: &BakeoffVolume) -> Self {
        let start = Instant::now();
        let storage = match kind {
            BakeoffCandidateKind::ExactGreedy => {
                CandidateStorage::ExactGreedy(ExactGreedy::build(volume))
            }
            BakeoffCandidateKind::SteppedSurface => {
                CandidateStorage::SteppedSurface(SteppedSurface::build(volume))
            }
            BakeoffCandidateKind::ClusteredVirtualGeometry => {
                CandidateStorage::ClusteredVirtualGeometry(ClusteredGeometry::build(volume))
            }
            BakeoffCandidateKind::SparseBrickRayCaster => {
                CandidateStorage::SparseBrickRayCaster(SparseBrickRayCaster::build(volume))
            }
        };
        let build_time = start.elapsed();
        let (logical_bytes, primitive_count, volumetric_exception_columns) = match &storage {
            CandidateStorage::ExactGreedy(candidate) => {
                (candidate.logical_bytes(), candidate.quads.len(), 0)
            }
            CandidateStorage::SteppedSurface(candidate) => (
                candidate.logical_bytes(),
                candidate
                    .columns
                    .len()
                    .saturating_add(candidate.material_runs.len()),
                candidate.invalid_columns,
            ),
            CandidateStorage::ClusteredVirtualGeometry(candidate) => {
                (candidate.logical_bytes(), candidate.quads.len(), 0)
            }
            CandidateStorage::SparseBrickRayCaster(candidate) => {
                (candidate.logical_bytes(), candidate.bricks.len(), 0)
            }
        };
        Self {
            kind,
            build_time,
            logical_bytes,
            primitive_count,
            volumetric_exception_columns,
            storage,
        }
    }

    fn trace(&self, ray: Ray) -> Option<BakeoffHit> {
        match &self.storage {
            CandidateStorage::ExactGreedy(candidate) => candidate.trace(ray),
            CandidateStorage::SteppedSurface(candidate) => candidate.trace(ray),
            CandidateStorage::ClusteredVirtualGeometry(candidate) => candidate.trace(ray),
            CandidateStorage::SparseBrickRayCaster(candidate) => candidate.trace(ray),
        }
    }

    /// Returns the exact greedy surface used by the two hardware-raster candidates. The clustered
    /// prototype changes traversal and residency, not leaf geometry, so both intentionally produce
    /// byte-identical 10 cm-lattice quads.
    pub fn gpu_quads(&self) -> Option<Vec<BakeoffGpuQuad>> {
        let quads = match &self.storage {
            CandidateStorage::ExactGreedy(candidate) => &candidate.quads,
            CandidateStorage::ClusteredVirtualGeometry(candidate) => &candidate.quads,
            CandidateStorage::SteppedSurface(_) | CandidateStorage::SparseBrickRayCaster(_) => {
                return None;
            }
        };
        Some(quads.iter().copied().map(Into::into).collect())
    }
}

#[derive(Clone, Debug)]
pub struct BakeoffComparison {
    pub kind: BakeoffCandidateKind,
    pub rays: u64,
    pub reference_hits: u64,
    pub candidate_hits: u64,
    pub ownerless_reference_hits: u64,
    pub invented_hits: u64,
    pub material_mismatches: u64,
    pub depth_mismatches: u64,
    pub maximum_depth_error_voxels: f64,
    pub trace_time: Duration,
}

/// Runs the frozen-volume oracle at a deliberately lower sampling grid than presentation. The
/// camera and world coordinates remain exact; Phase 1's browser harness scales the selected
/// candidates to native resolution after CPU correctness eliminates invalid representations.
pub fn run_virtual_surface_bakeoff(
    volume: &BakeoffVolume,
    camera: BakeoffCamera,
    ray_grid: [u32; 2],
    kinds: &[BakeoffCandidateKind],
) -> Result<(Vec<BakeoffCandidate>, Vec<BakeoffComparison>), BakeoffError> {
    if !camera.is_valid() {
        return Err(BakeoffError::InvalidCamera);
    }
    let [width, height] = ray_grid;
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 4_194_304 {
        return Err(BakeoffError::InvalidRayGrid);
    }
    if kinds.is_empty() {
        return Err(BakeoffError::EmptyCandidates);
    }
    let candidates = kinds
        .iter()
        .copied()
        .map(|kind| BakeoffCandidate::build(kind, volume))
        .collect::<Vec<_>>();
    let rays = (0..height)
        .flat_map(|y| (0..width).map(move |x| camera.ray(x, y, width, height)))
        .collect::<Vec<_>>();
    let references = rays
        .iter()
        .copied()
        .map(|ray| trace_voxel_grid(ray, volume.bounds, |coord| volume.material_at(coord)))
        .collect::<Vec<_>>();
    let mut comparisons = Vec::with_capacity(candidates.len());
    for candidate in &candidates {
        let started = Instant::now();
        let hits = rays
            .iter()
            .copied()
            .map(|ray| candidate.trace(ray))
            .collect::<Vec<_>>();
        let trace_time = started.elapsed();
        let mut comparison = BakeoffComparison {
            kind: candidate.kind,
            rays: rays.len() as u64,
            reference_hits: 0,
            candidate_hits: 0,
            ownerless_reference_hits: 0,
            invented_hits: 0,
            material_mismatches: 0,
            depth_mismatches: 0,
            maximum_depth_error_voxels: 0.0,
            trace_time,
        };
        for (reference, candidate_hit) in references.iter().zip(&hits) {
            comparison.reference_hits += u64::from(reference.is_some());
            comparison.candidate_hits += u64::from(candidate_hit.is_some());
            match (reference, candidate_hit) {
                (Some(_), None) => comparison.ownerless_reference_hits += 1,
                (None, Some(_)) => comparison.invented_hits += 1,
                (Some(reference), Some(candidate_hit)) => {
                    if reference.material != candidate_hit.material {
                        comparison.material_mismatches += 1;
                    }
                    let depth_error =
                        (reference.distance_voxels - candidate_hit.distance_voxels).abs();
                    comparison.maximum_depth_error_voxels =
                        comparison.maximum_depth_error_voxels.max(depth_error);
                    if depth_error > 1.0e-5 {
                        comparison.depth_mismatches += 1;
                    }
                }
                (None, None) => {}
            }
        }
        comparisons.push(comparison);
    }
    Ok((candidates, comparisons))
}

fn trace_voxel_grid(
    ray: Ray,
    bounds: VoxelBounds,
    mut material_at: impl FnMut(VoxelCoord) -> Material,
) -> Option<BakeoffHit> {
    let world_bounds = FloatBounds::new(
        bounds.min.as_array().map(f64::from),
        bounds.max.as_array().map(f64::from),
    );
    if !world_bounds.intersects(ray, f64::INFINITY) {
        return None;
    }
    let mut enter = 0.0f64;
    let mut exit = f64::INFINITY;
    for axis in 0..3 {
        if ray.direction[axis].abs() <= RAY_EPSILON {
            continue;
        }
        let inverse = 1.0 / ray.direction[axis];
        let first = (world_bounds.minimum[axis] - ray.origin[axis]) * inverse;
        let second = (world_bounds.maximum[axis] - ray.origin[axis]) * inverse;
        enter = enter.max(first.min(second));
        exit = exit.min(first.max(second));
    }
    if exit <= RAY_EPSILON {
        return None;
    }
    let start_distance = enter.max(0.0) + RAY_EPSILON;
    let start = [
        ray.origin[0] + ray.direction[0] * start_distance,
        ray.origin[1] + ray.direction[1] * start_distance,
        ray.origin[2] + ray.direction[2] * start_distance,
    ];
    let mut cell = VoxelCoord::new(
        floor_i32(start[0])?,
        floor_i32(start[1])?,
        floor_i32(start[2])?,
    );
    let steps = ray.direction.map(|value| if value < 0.0 { -1 } else { 1 });
    let mut next = [0.0; 3];
    let mut delta = [0.0; 3];
    for axis in 0..3 {
        if ray.direction[axis].abs() <= RAY_EPSILON {
            next[axis] = f64::INFINITY;
            delta[axis] = f64::INFINITY;
            continue;
        }
        let component = cell.as_array()[axis];
        let boundary = if steps[axis] > 0 {
            f64::from(component) + 1.0
        } else {
            f64::from(component)
        };
        next[axis] = (boundary - ray.origin[axis]) / ray.direction[axis];
        delta[axis] = 1.0 / ray.direction[axis].abs();
    }
    let mut entry_face = 0u8;
    let max_steps = bounds
        .volume()
        .and_then(|volume| usize::try_from(volume).ok())
        .unwrap_or(0)
        .min(16_777_216);
    for _ in 0..=max_steps {
        if bounds.contains(cell) {
            let material = material_at(cell);
            if material.is_renderable() {
                return Some(BakeoffHit {
                    distance_voxels: start_distance.max(
                        next.into_iter()
                            .zip(delta)
                            .map(|(next, delta)| next - delta)
                            .fold(start_distance, f64::max),
                    ),
                    material,
                    face: entry_face,
                });
            }
        }
        let axis = if next[0] <= next[1] && next[0] <= next[2] {
            0
        } else if next[1] <= next[2] {
            1
        } else {
            2
        };
        if next[axis] > exit + RAY_EPSILON {
            break;
        }
        match axis {
            0 => cell.x = cell.x.saturating_add(steps[axis]),
            1 => cell.y = cell.y.saturating_add(steps[axis]),
            _ => cell.z = cell.z.saturating_add(steps[axis]),
        }
        entry_face = face_index(
            match axis {
                0 => FaceAxis::X,
                1 => FaceAxis::Y,
                _ => FaceAxis::Z,
            },
            steps[axis] < 0,
        );
        next[axis] += delta[axis];
    }
    None
}

fn floor_i32(value: f64) -> Option<i32> {
    let value = value.floor();
    (value >= f64::from(i32::MIN) && value <= f64::from(i32::MAX)).then_some(value as i32)
}

fn closest(hits: impl Iterator<Item = BakeoffHit>) -> Option<BakeoffHit> {
    hits.min_by(|left, right| left.distance_voxels.total_cmp(&right.distance_voxels))
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct FacePlane {
    axis: FaceAxis,
    plane: i32,
    positive: bool,
    material_id: u16,
}

fn merge_faces(faces: &[CanonicalFaceKey]) -> Vec<GreedyQuad> {
    let mut planes = BTreeMap::<FacePlane, BTreeSet<(i32, i32)>>::new();
    for face in faces {
        let solid_component = match face.axis {
            FaceAxis::X => face.solid_side.x,
            FaceAxis::Y => face.solid_side.y,
            FaceAxis::Z => face.solid_side.z,
        };
        let positive = solid_component.saturating_add(1) == face.plane;
        planes
            .entry(FacePlane {
                axis: face.axis,
                plane: face.plane,
                positive,
                material_id: face.material_id,
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
            let Some(material) = Material::from_id(plane.material_id) else {
                continue;
            };
            quads.push(GreedyQuad {
                axis: plane.axis,
                plane: plane.plane,
                u,
                v,
                width: width as u32,
                height: height as u32,
                positive: plane.positive,
                material,
            });
        }
    }
    quads
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> VoxelBounds {
        VoxelBounds::new(VoxelCoord::new(-4, -2, -4), VoxelCoord::new(4, 6, 4))
            .expect("fixture bounds")
    }

    fn camera() -> BakeoffCamera {
        BakeoffCamera {
            eye_voxels: [0.5, 4.5, 7.5],
            yaw_radians: 0.0,
            pitch_radians: -0.25,
            vertical_fov_radians: 1.0,
            aspect_ratio: 1.0,
        }
    }

    #[test]
    fn exact_cluster_and_brick_match_dense_reference_across_negative_coordinates() {
        let volume = BakeoffVolume::from_sampler(bounds(), |coord| {
            if coord.y < 1 || (coord.x == -2 && coord.y < 4 && coord.z == -1) {
                Material::Stone
            } else {
                Material::Air
            }
        })
        .expect("volume");
        let (_, comparisons) = run_virtual_surface_bakeoff(
            &volume,
            camera(),
            [96, 96],
            &[
                BakeoffCandidateKind::ExactGreedy,
                BakeoffCandidateKind::ClusteredVirtualGeometry,
                BakeoffCandidateKind::SparseBrickRayCaster,
            ],
        )
        .expect("bake-off");
        assert!(comparisons[0].reference_hits > 0);
        for comparison in comparisons {
            assert_eq!(comparison.ownerless_reference_hits, 0);
            assert_eq!(comparison.invented_hits, 0);
            assert_eq!(comparison.material_mismatches, 0);
            assert_eq!(comparison.depth_mismatches, 0);
            assert!(comparison.maximum_depth_error_voxels <= 1.0e-5);
        }
    }

    #[test]
    fn stepped_surface_reports_and_drops_non_heightfield_columns() {
        let volume = BakeoffVolume::from_sampler(bounds(), |coord| {
            if coord.y < 0 || (coord.x == -1 && coord.y == 3 && coord.z == -1) {
                Material::Stone
            } else {
                Material::Air
            }
        })
        .expect("volume");
        let candidate = BakeoffCandidate::build(BakeoffCandidateKind::SteppedSurface, &volume);
        assert_eq!(candidate.volumetric_exception_columns, 1);
        let CandidateStorage::SteppedSurface(surface) = &candidate.storage else {
            panic!("candidate kind changed");
        };
        assert_eq!(
            surface.material_at(VoxelCoord::new(-1, 3, -1)),
            Material::Air
        );
    }

    #[test]
    fn greedy_merging_preserves_material_boundaries_and_reduces_a_slab_to_six_quads() {
        let slab_bounds = VoxelBounds::new(VoxelCoord::new(-3, -1, -2), VoxelCoord::new(3, 0, 2))
            .expect("slab bounds");
        let volume = BakeoffVolume::from_sampler(slab_bounds, |_| Material::Grass).expect("volume");
        let greedy = ExactGreedy::build(&volume);
        assert_eq!(greedy.quads.len(), 6);

        let split = BakeoffVolume::from_sampler(slab_bounds, |coord| {
            if coord.x < 0 {
                Material::Grass
            } else {
                Material::Stone
            }
        })
        .expect("split volume");
        let split_greedy = ExactGreedy::build(&split);
        assert!(split_greedy.quads.len() > greedy.quads.len());
        assert!(
            split_greedy
                .quads
                .iter()
                .any(|quad| quad.material == Material::Stone)
        );
    }

    #[test]
    fn sparse_bricks_round_trip_every_material_and_keep_air_sparse() {
        let volume = BakeoffVolume::from_sampler(bounds(), |coord| {
            if coord == VoxelCoord::new(-3, 2, -1) {
                Material::Basalt
            } else {
                Material::Air
            }
        })
        .expect("volume");
        let bricks = SparseBrickRayCaster::build(&volume);
        for z in bounds().min.z..bounds().max.z {
            for y in bounds().min.y..bounds().max.y {
                for x in bounds().min.x..bounds().max.x {
                    let coord = VoxelCoord::new(x, y, z);
                    assert_eq!(bricks.material_at(coord), volume.material_at(coord));
                }
            }
        }
        let brick_voxels = BRICK_EDGE as usize * BRICK_EDGE as usize * BRICK_EDGE as usize;
        assert!(bricks.bricks.len() * brick_voxels < volume.materials.len() * 8);
    }

    #[test]
    fn clustered_hierarchy_traverses_non_contiguous_recursive_children() {
        let volume = BakeoffVolume::from_sampler(bounds(), |coord| {
            if (coord.x + coord.y + coord.z).rem_euclid(2) == 0 {
                Material::Limestone
            } else {
                Material::Air
            }
        })
        .expect("volume");
        let candidate =
            BakeoffCandidate::build(BakeoffCandidateKind::ClusteredVirtualGeometry, &volume);
        let CandidateStorage::ClusteredVirtualGeometry(clustered) = &candidate.storage else {
            panic!("candidate kind changed");
        };
        assert!(clustered.quads.len() > CLUSTER_QUADS * 2);
        assert!(clustered.nodes.len() > 3);
        let (_, comparisons) = run_virtual_surface_bakeoff(
            &volume,
            camera(),
            [64, 64],
            &[BakeoffCandidateKind::ClusteredVirtualGeometry],
        )
        .expect("bake-off");
        assert_eq!(comparisons[0].ownerless_reference_hits, 0);
        assert_eq!(comparisons[0].invented_hits, 0);
        assert_eq!(comparisons[0].material_mismatches, 0);
        assert_eq!(comparisons[0].depth_mismatches, 0);
    }
}
