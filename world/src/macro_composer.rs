//! Experimental canonical heightfield composition over a [`MacroTerrainSource`].
//!
//! This adapter intentionally materializes only the information the macro schema actually carries:
//! learned or procedural elevation becomes solid stone, low terrain is flooded to one pinned sea
//! level, and everything above is air. It does not invent caves, strata, vegetation, routes, or
//! authored atlas content. The existing procedural-v16 engine remains the parity authority.

use crate::{
    AtmosphereSample, CHUNK_EDGE, Chunk, ChunkCoord, ChunkSnapshot, EditMap,
    MACRO_FIELD_SCHEMA_VERSION, MAX_MACRO_BLOCK_SAMPLES, MAX_SURFACE_SAMPLE_BLOCK_SAMPLES,
    MAX_SURFACE_SEARCH_RADIUS, MAX_VOXEL_BLOCK_SAMPLES, MAX_WORLD_PRODUCT_BATCH, MacroBlock,
    MacroBlockBatch, MacroBlockRequest, MacroTerrainSource, Material, MeshingHalo, SkylineFeature,
    SkylineFeatureKind, SurfaceLodLevel, SurfaceRegion, SurfaceSample, SurfaceSampleBlockRequest,
    SurfaceSampleBlockSnapshot, SurfaceSearchHit, SurfaceSearchKind, SurfaceSearchRequest,
    SurfaceSearchSnapshot, SurfaceTileCoord, SurfaceTileSnapshot, VoxelBlockRequest,
    VoxelBlockSnapshot, VoxelCoord, WorldProduct, WorldProductBatch, WorldProductBatchItem,
    WorldProductBatchResult, WorldProductPriority, WorldProductRequest, WorldSourceEngine,
    WorldSourceError, WorldSourceIdentity, WorldSourceIdentityHash,
    generate_surface_tile_mesh_with, generate_water_tile_mesh_with,
    surface_tiles_affected_by_column,
};

const SURFACE_TILE_SAMPLE_EDGE: u32 = 34;

/// Minimal, fidelity-honest field-to-voxel adapter used to exercise non-procedural macro sources.
pub struct HeightfieldWorldSource {
    source: Box<dyn MacroTerrainSource>,
    identity: WorldSourceIdentity,
    identity_hash: WorldSourceIdentityHash,
    sea_level_voxels: i32,
}

impl HeightfieldWorldSource {
    pub fn new(
        source: Box<dyn MacroTerrainSource>,
        sea_level_voxels: i32,
    ) -> Result<Self, WorldSourceError> {
        let identity = source.identity().clone();
        let transform = identity.macro_coordinate_transform;
        if identity.macro_field_schema_version != MACRO_FIELD_SCHEMA_VERSION
            || transform.horizontal_unit_millimetres == 0
            || !matches!(transform.x_axis_sign, -1 | 1)
            || !matches!(transform.z_axis_sign, -1 | 1)
        {
            return Err(WorldSourceError::MalformedMacroBlock);
        }
        let identity_hash = identity.identity_hash();
        Ok(Self {
            source,
            identity,
            identity_hash,
            sea_level_voxels,
        })
    }

    pub const fn sea_level_voxels(&self) -> i32 {
        self.sea_level_voxels
    }

    fn prepare_region(
        &self,
        priority: WorldProductPriority,
        origin: [i32; 2],
        sample_shape: [u32; 2],
        stride_voxels: u32,
    ) -> Result<PreparedMacroRegion, WorldSourceError> {
        let [width, depth] = sample_shape;
        if width == 0 || depth == 0 || stride_voxels == 0 {
            return Err(WorldSourceError::EmptyMacroBlock);
        }
        if !sample_axis_is_representable(origin[0], width, stride_voxels)
            || !sample_axis_is_representable(origin[1], depth, stride_voxels)
        {
            return Err(WorldSourceError::InvalidBlockCoordinate);
        }
        let width_usize =
            usize::try_from(width).map_err(|_| WorldSourceError::MacroBlockTooLarge)?;
        if width_usize > MAX_MACRO_BLOCK_SAMPLES {
            return Err(WorldSourceError::MacroBlockTooLarge);
        }
        let rows_per_block = (MAX_MACRO_BLOCK_SAMPLES / width_usize).max(1);
        let block_count = usize::try_from(depth)
            .map_err(|_| WorldSourceError::MacroBlockTooLarge)?
            .div_ceil(rows_per_block);
        if block_count > MAX_WORLD_PRODUCT_BATCH {
            return Err(WorldSourceError::MacroBlockTooLarge);
        }

        let mut requests = Vec::with_capacity(block_count);
        let mut row = 0u32;
        while row < depth {
            let block_depth = (depth - row).min(rows_per_block as u32);
            let origin_z = i64::from(origin[1]) + i64::from(row) * i64::from(stride_voxels);
            let origin_z =
                i32::try_from(origin_z).map_err(|_| WorldSourceError::InvalidBlockCoordinate)?;
            requests.push(MacroBlockRequest {
                origin: [origin[0], origin_z],
                sample_shape: [width, block_depth],
                stride_voxels,
            });
            row += block_depth;
        }

        let result = self.source.request_blocks(MacroBlockBatch {
            priority,
            requests: requests.clone(),
        })?;
        if result.source_identity_hash != self.identity_hash
            || result.blocks.len() != requests.len()
        {
            return Err(WorldSourceError::MalformedMacroBlock);
        }
        let mut grids = Vec::with_capacity(requests.len());
        for (request, block) in requests.into_iter().zip(result.blocks) {
            grids.push(self.validate_block(request, block)?);
        }
        Ok(PreparedMacroRegion { grids })
    }

    fn validate_block(
        &self,
        request: MacroBlockRequest,
        block: MacroBlock,
    ) -> Result<PreparedMacroGrid, WorldSourceError> {
        let sample_count = usize::try_from(request.sample_shape[0])
            .ok()
            .and_then(|width| {
                usize::try_from(request.sample_shape[1])
                    .ok()
                    .and_then(|depth| width.checked_mul(depth))
            })
            .ok_or(WorldSourceError::MalformedMacroBlock)?;
        if block.schema_version != MACRO_FIELD_SCHEMA_VERSION
            || block.request != request
            || block.coordinate_transform != self.identity.macro_coordinate_transform
            || block.elevation_voxels.len() != sample_count
            || block.temperature.len() != sample_count
            || block.moisture.len() != sample_count
            || block.ridge.len() != sample_count
            || block.validity.len() != sample_count
        {
            return Err(WorldSourceError::MalformedMacroBlock);
        }
        let mut columns = Vec::with_capacity(sample_count);
        for index in 0..sample_count {
            if !block.validity[index] {
                return Err(WorldSourceError::SourceCoverageUnavailable);
            }
            let elevation = block.elevation_voxels[index];
            let height = f64::from(elevation).floor();
            if !elevation.is_finite()
                || height < f64::from(i32::MIN)
                || height > f64::from(i32::MAX)
                || !normalized(block.temperature[index])
                || !normalized(block.moisture[index])
                || !normalized(block.ridge[index])
            {
                return Err(WorldSourceError::MalformedMacroBlock);
            }
            columns.push(PreparedColumn {
                height: height as i32,
                temperature: block.temperature[index],
                moisture: block.moisture[index],
                ridge: block.ridge[index],
            });
        }
        Ok(PreparedMacroGrid { request, columns })
    }

    fn chunk_with_halo(
        &self,
        priority: WorldProductPriority,
        coord: ChunkCoord,
    ) -> Result<ChunkSnapshot, WorldSourceError> {
        if !coord.is_world_representable() {
            return Err(WorldSourceError::InvalidChunkCoordinate);
        }
        let origin = coord.world_origin();
        let min_x = origin[0].saturating_sub(1);
        let min_z = origin[2].saturating_sub(1);
        let max_x = origin[0].saturating_add(CHUNK_EDGE as i32);
        let max_z = origin[2].saturating_add(CHUNK_EDGE as i32);
        let width = u32::try_from(i64::from(max_x) - i64::from(min_x) + 1)
            .map_err(|_| WorldSourceError::InvalidChunkCoordinate)?;
        let depth = u32::try_from(i64::from(max_z) - i64::from(min_z) + 1)
            .map_err(|_| WorldSourceError::InvalidChunkCoordinate)?;
        let region = self.prepare_region(priority, [min_x, min_z], [width, depth], 1)?;

        let mut chunk = Chunk::empty(coord);
        for y in 0..CHUNK_EDGE {
            for z in 0..CHUNK_EDGE {
                for x in 0..CHUNK_EDGE {
                    let world_x = origin[0] + x as i32;
                    let world_y = origin[1] + y as i32;
                    let world_z = origin[2] + z as i32;
                    let material = self.material_at(&region, world_x, world_y, world_z)?;
                    chunk.set(x, y, z, material);
                }
            }
        }
        let halo = MeshingHalo::from_sampler(coord, |x, y, z| {
            self.material_at(&region, x, y, z).unwrap_or(Material::Air)
        });
        ChunkSnapshot::new(self.identity_hash, chunk, halo)
            .ok_or(WorldSourceError::MalformedMacroBlock)
    }

    fn voxel_block(
        &self,
        priority: WorldProductPriority,
        request: VoxelBlockRequest,
    ) -> Result<VoxelBlockSnapshot, WorldSourceError> {
        validate_voxel_block_request(request)?;
        let region = self.prepare_region(
            priority,
            [request.min.x, request.min.z],
            [request.sample_shape[0], request.sample_shape[2]],
            1,
        )?;
        let [width, height, depth] = request.sample_shape;
        let sample_count = usize::try_from(width)
            .ok()
            .and_then(|width| {
                usize::try_from(height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .and_then(|plane| {
                usize::try_from(depth)
                    .ok()
                    .and_then(|depth| plane.checked_mul(depth))
            })
            .ok_or(WorldSourceError::BlockTooLarge)?;
        let mut materials = Vec::with_capacity(sample_count);
        for z in 0..depth {
            for y in 0..height {
                for x in 0..width {
                    materials.push(self.material_at(
                        &region,
                        request.min.x + x as i32,
                        request.min.y + y as i32,
                        request.min.z + z as i32,
                    )?);
                }
            }
        }
        VoxelBlockSnapshot::from_materials(self.identity_hash, request, materials)
            .ok_or(WorldSourceError::MalformedMacroBlock)
    }

    fn surface_sample_block(
        &self,
        priority: WorldProductPriority,
        request: SurfaceSampleBlockRequest,
    ) -> Result<SurfaceSampleBlockSnapshot, WorldSourceError> {
        validate_surface_block_request(request)?;
        let region = self.prepare_region(priority, request.origin, request.sample_shape, 1)?;
        let [width, depth] = request.sample_shape;
        let mut samples = Vec::with_capacity((width as usize) * (depth as usize));
        for z in 0..depth {
            for x in 0..width {
                samples.push(self.surface_sample_from_region(
                    &region,
                    request.origin[0] + x as i32,
                    request.origin[1] + z as i32,
                )?);
            }
        }
        SurfaceSampleBlockSnapshot::from_samples(self.identity_hash, request, samples)
            .ok_or(WorldSourceError::MalformedMacroBlock)
    }

    fn surface_search(
        &self,
        priority: WorldProductPriority,
        request: SurfaceSearchRequest,
    ) -> Result<SurfaceSearchSnapshot, WorldSourceError> {
        if request.min_radius > request.max_radius {
            return Err(WorldSourceError::InvalidSearchRadius);
        }
        if request.max_radius > MAX_SURFACE_SEARCH_RADIUS {
            return Err(WorldSourceError::SearchRadiusTooLarge);
        }
        let radius = i32::from(request.max_radius);
        let min_x = request.origin[0]
            .checked_sub(radius)
            .ok_or(WorldSourceError::InvalidBlockCoordinate)?;
        let min_z = request.origin[1]
            .checked_sub(radius)
            .ok_or(WorldSourceError::InvalidBlockCoordinate)?;
        request.origin[0]
            .checked_add(radius)
            .ok_or(WorldSourceError::InvalidBlockCoordinate)?;
        request.origin[1]
            .checked_add(radius)
            .ok_or(WorldSourceError::InvalidBlockCoordinate)?;
        let edge = u32::from(request.max_radius) * 2 + 1;
        let region = self.prepare_region(priority, [min_x, min_z], [edge, edge], 1)?;
        let matches = |sample: SurfaceSample| match request.kind {
            SurfaceSearchKind::DryLand => sample.water_level.is_none(),
            SurfaceSearchKind::WaterDepthAtLeast { depth_voxels } => {
                sample.water_level.is_some_and(|water| {
                    i64::from(water) - i64::from(sample.height) >= i64::from(depth_voxels)
                })
            }
        };
        let hit = 'search: {
            for radius in i32::from(request.min_radius)..=i32::from(request.max_radius) {
                let min_x = request.origin[0] - radius;
                let max_x = request.origin[0] + radius;
                let min_z = request.origin[1] - radius;
                let max_z = request.origin[1] + radius;
                for x in min_x..=max_x {
                    for z in [min_z, max_z] {
                        let sample = self.surface_sample_from_region(&region, x, z)?;
                        if matches(sample) {
                            break 'search Some(SurfaceSearchHit {
                                coord: [x, z],
                                sample,
                            });
                        }
                    }
                }
                for z in min_z.saturating_add(1)..max_z {
                    for x in [min_x, max_x] {
                        let sample = self.surface_sample_from_region(&region, x, z)?;
                        if matches(sample) {
                            break 'search Some(SurfaceSearchHit {
                                coord: [x, z],
                                sample,
                            });
                        }
                    }
                }
            }
            None
        };
        Ok(SurfaceSearchSnapshot {
            source_identity_hash: self.identity_hash,
            request,
            hit,
        })
    }

    fn surface_tile(
        &self,
        edits: &EditMap,
        coord: SurfaceTileCoord,
        priority: WorldProductPriority,
    ) -> Result<SurfaceTileSnapshot, WorldSourceError> {
        if !coord.is_world_representable() {
            return Err(WorldSourceError::InvalidSurfaceTileCoordinate);
        }
        let [origin_x, origin_z] = coord.voxel_origin();
        let stride = coord.stride_voxels();
        let sample_origin = [
            origin_x
                .checked_sub(stride / 2)
                .ok_or(WorldSourceError::SourceCoverageUnavailable)?,
            origin_z
                .checked_sub(stride / 2)
                .ok_or(WorldSourceError::SourceCoverageUnavailable)?,
        ];
        let stride_u32 = stride as u32;
        let region = self.prepare_region(
            priority,
            sample_origin,
            [SURFACE_TILE_SAMPLE_EDGE; 2],
            stride_u32,
        )?;
        for sample_z in 0..SURFACE_TILE_SAMPLE_EDGE {
            for sample_x in 0..SURFACE_TILE_SAMPLE_EDGE {
                let x = i64::from(sample_origin[0]) + i64::from(sample_x) * i64::from(stride_u32);
                let z = i64::from(sample_origin[1]) + i64::from(sample_z) * i64::from(stride_u32);
                let Some((x, z)) = i32::try_from(x).ok().zip(i32::try_from(z).ok()) else {
                    return Err(WorldSourceError::SourceCoverageUnavailable);
                };
                if region.column(x, z).is_none() {
                    return Err(WorldSourceError::MalformedMacroBlock);
                }
            }
        }
        let terrain = generate_surface_tile_mesh_with(coord, |x, z| {
            self.edited_surface(&region, edits, x, z)
                .unwrap_or((i32::MIN, Material::Stone))
        });
        let water = generate_water_tile_mesh_with(coord, |x, z| {
            region.column(x, z).is_some_and(|column| {
                edits
                    .override_at(VoxelCoord::new(x, self.sea_level_voxels, z))
                    .map_or(column.height < self.sea_level_voxels, |material| {
                        material == Material::Water
                    })
            })
        });
        Ok(SurfaceTileSnapshot {
            source_identity_hash: self.identity_hash,
            terrain,
            water,
        })
    }

    fn edited_surface(
        &self,
        region: &PreparedMacroRegion,
        edits: &EditMap,
        x: i32,
        z: i32,
    ) -> Result<(i32, Material), WorldSourceError> {
        let column = region
            .column(x, z)
            .ok_or(WorldSourceError::MalformedMacroBlock)?;
        Ok(
            edits.surface_sample_with(x, z, (column.height, Material::Stone), i32::MIN, |coord| {
                material_for_column(column, self.sea_level_voxels, coord.y)
            }),
        )
    }

    fn surface_sample_from_region(
        &self,
        region: &PreparedMacroRegion,
        x: i32,
        z: i32,
    ) -> Result<SurfaceSample, WorldSourceError> {
        let column = region
            .column(x, z)
            .ok_or(WorldSourceError::MalformedMacroBlock)?;
        Ok(surface_sample(column, self.sea_level_voxels))
    }

    fn material_at(
        &self,
        region: &PreparedMacroRegion,
        x: i32,
        y: i32,
        z: i32,
    ) -> Result<Material, WorldSourceError> {
        let column = region
            .column(x, z)
            .ok_or(WorldSourceError::MalformedMacroBlock)?;
        Ok(material_for_column(column, self.sea_level_voxels, y))
    }
}

impl WorldSourceEngine for HeightfieldWorldSource {
    fn identity(&self) -> &WorldSourceIdentity {
        &self.identity
    }

    fn generate_batch(
        &self,
        request: WorldProductBatch,
    ) -> Result<WorldProductBatchResult, WorldSourceError> {
        if request.requests.len() > MAX_WORLD_PRODUCT_BATCH {
            return Err(WorldSourceError::BatchTooLarge);
        }
        let mut items = Vec::with_capacity(request.requests.len());
        for product_request in request.requests {
            let result = match product_request {
                WorldProductRequest::ChunkWithHalo(coord) => self
                    .chunk_with_halo(request.priority, coord)
                    .map(WorldProduct::Chunk),
                WorldProductRequest::VoxelBlock(block) => self
                    .voxel_block(request.priority, block)
                    .map(WorldProduct::VoxelBlock),
                WorldProductRequest::SurfaceSampleBlock(block) => self
                    .surface_sample_block(request.priority, block)
                    .map(WorldProduct::SurfaceSampleBlock),
                WorldProductRequest::SurfaceSearch(search) => self
                    .surface_search(request.priority, search)
                    .map(WorldProduct::SurfaceSearch),
                WorldProductRequest::SurfaceTile(coord) => self
                    .surface_tile(&EditMap::default(), coord, request.priority)
                    .map(WorldProduct::SurfaceTile),
            };
            items.push(WorldProductBatchItem {
                request: product_request,
                result,
            });
        }
        Ok(WorldProductBatchResult {
            source_identity_hash: self.identity_hash,
            items,
        })
    }

    fn generate_edited_surface_tile(
        &self,
        edits: &EditMap,
        coord: SurfaceTileCoord,
    ) -> Result<SurfaceTileSnapshot, WorldSourceError> {
        self.surface_tile(edits, coord, WorldProductPriority::ReplacementSurface)
    }

    fn surface_tiles_affected_by_voxel(
        &self,
        _edits: &EditMap,
        level: SurfaceLodLevel,
        coord: VoxelCoord,
    ) -> Vec<SurfaceTileCoord> {
        surface_tiles_affected_by_column(level, coord.x, coord.z)
    }

    fn atmosphere_sample(&self, x: i32, z: i32) -> (AtmosphereSample, SurfaceRegion) {
        let sample = self
            .prepare_region(WorldProductPriority::VisibleSurface, [x, z], [1, 1], 1)
            .ok()
            .and_then(|region| region.column(x, z).copied());
        let Some(column) = sample else {
            return (AtmosphereSample::default(), SurfaceRegion::Alpine);
        };
        (
            AtmosphereSample {
                humidity: column.moisture,
                coldness: 1.0 - column.temperature,
                aerosol: column.ridge,
                cloudiness: (column.moisture + column.ridge) * 0.5,
                horizon_warmth: column.temperature,
                haze: column.moisture * 0.5,
            },
            SurfaceRegion::Alpine,
        )
    }

    fn skyline_features_anchored_in(&self, _bounds: [[i32; 2]; 2]) -> Vec<SkylineFeature> {
        Vec::new()
    }

    fn skyline_features_at(&self, _coord: VoxelCoord) -> Vec<SkylineFeature> {
        Vec::new()
    }

    fn nearest_skyline_feature(
        &self,
        _x: i32,
        _z: i32,
        _kind: SkylineFeatureKind,
        _max_radius_cells: i32,
    ) -> Option<SkylineFeature> {
        None
    }

    fn nearest_prominent_skyline_feature(
        &self,
        _x: i32,
        _z: i32,
        _kind: SkylineFeatureKind,
        _max_radius_cells: i32,
    ) -> Option<SkylineFeature> {
        None
    }
}

#[derive(Clone, Copy, Debug)]
struct PreparedColumn {
    height: i32,
    temperature: f32,
    moisture: f32,
    ridge: f32,
}

struct PreparedMacroGrid {
    request: MacroBlockRequest,
    columns: Vec<PreparedColumn>,
}

impl PreparedMacroGrid {
    fn column(&self, x: i32, z: i32) -> Option<&PreparedColumn> {
        let stride = i64::from(self.request.stride_voxels);
        let offset_x = i64::from(x) - i64::from(self.request.origin[0]);
        let offset_z = i64::from(z) - i64::from(self.request.origin[1]);
        if offset_x < 0 || offset_z < 0 || offset_x % stride != 0 || offset_z % stride != 0 {
            return None;
        }
        let sample_x = offset_x / stride;
        let sample_z = offset_z / stride;
        let width = i64::from(self.request.sample_shape[0]);
        let depth = i64::from(self.request.sample_shape[1]);
        if sample_x >= width || sample_z >= depth {
            return None;
        }
        let index = usize::try_from(sample_x + sample_z * width).ok()?;
        self.columns.get(index)
    }
}

struct PreparedMacroRegion {
    grids: Vec<PreparedMacroGrid>,
}

impl PreparedMacroRegion {
    fn column(&self, x: i32, z: i32) -> Option<&PreparedColumn> {
        self.grids.iter().find_map(|grid| grid.column(x, z))
    }
}

fn material_for_column(column: &PreparedColumn, sea_level_voxels: i32, y: i32) -> Material {
    if y <= column.height {
        Material::Stone
    } else if y <= sea_level_voxels {
        Material::Water
    } else {
        Material::Air
    }
}

fn surface_sample(column: &PreparedColumn, sea_level_voxels: i32) -> SurfaceSample {
    SurfaceSample {
        height: column.height,
        material: Material::Stone,
        water_level: (column.height < sea_level_voxels).then_some(sea_level_voxels),
        region: SurfaceRegion::Alpine,
        moisture: column.moisture,
        temperature: column.temperature,
        ridge: column.ridge,
        route: None,
    }
}

fn normalized(value: f32) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

fn sample_axis_is_representable(origin: i32, sample_count: u32, stride: u32) -> bool {
    sample_count > 0
        && i64::from(origin) + i64::from(sample_count - 1) * i64::from(stride)
            <= i64::from(i32::MAX)
}

fn validate_voxel_block_request(request: VoxelBlockRequest) -> Result<(), WorldSourceError> {
    if request.sample_shape.contains(&0) {
        return Err(WorldSourceError::EmptyBlock);
    }
    let [width, height, depth] = request.sample_shape;
    let sample_count = usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|plane| {
            usize::try_from(depth)
                .ok()
                .and_then(|depth| plane.checked_mul(depth))
        })
        .ok_or(WorldSourceError::BlockTooLarge)?;
    if sample_count > MAX_VOXEL_BLOCK_SAMPLES {
        return Err(WorldSourceError::BlockTooLarge);
    }
    if !sample_axis_is_representable(request.min.x, width, 1)
        || !sample_axis_is_representable(request.min.y, height, 1)
        || !sample_axis_is_representable(request.min.z, depth, 1)
    {
        return Err(WorldSourceError::InvalidBlockCoordinate);
    }
    Ok(())
}

fn validate_surface_block_request(
    request: SurfaceSampleBlockRequest,
) -> Result<(), WorldSourceError> {
    if request.sample_shape.contains(&0) {
        return Err(WorldSourceError::EmptyBlock);
    }
    let [width, depth] = request.sample_shape;
    let sample_count = usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(depth)
                .ok()
                .and_then(|depth| width.checked_mul(depth))
        })
        .ok_or(WorldSourceError::BlockTooLarge)?;
    if sample_count > MAX_SURFACE_SAMPLE_BLOCK_SAMPLES {
        return Err(WorldSourceError::BlockTooLarge);
    }
    if !sample_axis_is_representable(request.origin[0], width, 1)
        || !sample_axis_is_representable(request.origin[1], depth, 1)
    {
        return Err(WorldSourceError::InvalidBlockCoordinate);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        MESHING_HALO_VOXELS, MacroBlockBatchResult, MacroCoordinateTransform,
        WorldSourceIdentityHash,
    };

    #[derive(Clone, Copy)]
    enum FakeBehavior {
        Valid,
        Unavailable,
        MalformedLength,
        WrongIdentity,
    }

    struct FakeMacroSource {
        identity: WorldSourceIdentity,
        behavior: FakeBehavior,
        elevation: f32,
    }

    impl FakeMacroSource {
        fn new(behavior: FakeBehavior, elevation: f32) -> Self {
            Self {
                identity: WorldSourceIdentity::procedural_v16(42),
                behavior,
                elevation,
            }
        }
    }

    impl MacroTerrainSource for FakeMacroSource {
        fn identity(&self) -> &WorldSourceIdentity {
            &self.identity
        }

        fn request_blocks(
            &self,
            request: MacroBlockBatch,
        ) -> Result<MacroBlockBatchResult, WorldSourceError> {
            let mut blocks = Vec::with_capacity(request.requests.len());
            for request in request.requests {
                let sample_count =
                    request.sample_shape[0] as usize * request.sample_shape[1] as usize;
                let mut elevation_voxels = vec![self.elevation; sample_count];
                if matches!(self.behavior, FakeBehavior::MalformedLength) {
                    elevation_voxels.pop();
                }
                blocks.push(MacroBlock {
                    schema_version: MACRO_FIELD_SCHEMA_VERSION,
                    request,
                    coordinate_transform: MacroCoordinateTransform::CANONICAL_VOXELS,
                    elevation_voxels,
                    temperature: vec![0.25; sample_count],
                    moisture: vec![0.75; sample_count],
                    ridge: vec![0.5; sample_count],
                    validity: vec![
                        !matches!(self.behavior, FakeBehavior::Unavailable);
                        sample_count
                    ],
                });
            }
            let source_identity_hash = if matches!(self.behavior, FakeBehavior::WrongIdentity) {
                WorldSourceIdentityHash::from_bytes([0xff; 32])
            } else {
                self.identity.identity_hash()
            };
            Ok(MacroBlockBatchResult {
                source_identity_hash,
                blocks,
            })
        }
    }

    fn heightfield(behavior: FakeBehavior) -> HeightfieldWorldSource {
        HeightfieldWorldSource::new(Box::new(FakeMacroSource::new(behavior, 1.75)), 3)
            .expect("valid fake source identity")
    }

    fn product(
        source: &HeightfieldWorldSource,
        request: WorldProductRequest,
    ) -> Result<WorldProduct, WorldSourceError> {
        source
            .generate_batch(WorldProductBatch {
                priority: WorldProductPriority::CollisionCritical,
                requests: vec![request],
            })
            .expect("bounded fake batch")
            .items
            .into_iter()
            .next()
            .expect("one keyed result")
            .result
    }

    #[test]
    fn all_products_share_one_floor_water_and_air_composition() {
        let source = heightfield(FakeBehavior::Valid);
        let coord = ChunkCoord::new(0, 0, 0);
        let WorldProduct::Chunk(chunk) =
            product(&source, WorldProductRequest::ChunkWithHalo(coord)).expect("covered chunk")
        else {
            panic!("expected chunk product");
        };
        assert_eq!(chunk.chunk.get(0, 1, 0), Material::Stone);
        assert_eq!(chunk.chunk.get(0, 2, 0), Material::Water);
        assert_eq!(chunk.chunk.get(0, 3, 0), Material::Water);
        assert_eq!(chunk.chunk.get(0, 4, 0), Material::Air);
        assert_eq!(chunk.meshing_halo.voxels().len(), MESHING_HALO_VOXELS);
        assert_eq!(
            chunk.meshing_halo.sample_local([-1, 2, 0]),
            Some(Material::Water)
        );

        let voxel_request = VoxelBlockRequest {
            min: VoxelCoord::new(-1, 1, -1),
            sample_shape: [2, 4, 2],
        };
        let WorldProduct::VoxelBlock(voxels) =
            product(&source, WorldProductRequest::VoxelBlock(voxel_request))
                .expect("covered voxel block")
        else {
            panic!("expected voxel block");
        };
        for y in 1..=4 {
            assert_eq!(
                voxels.sample(VoxelCoord::new(-1, y, -1)),
                Some(chunk_material_at(&chunk, -1, y, -1))
            );
        }

        let surface_request = SurfaceSampleBlockRequest {
            origin: [-1, -1],
            sample_shape: [2, 2],
        };
        let WorldProduct::SurfaceSampleBlock(samples) = product(
            &source,
            WorldProductRequest::SurfaceSampleBlock(surface_request),
        )
        .expect("covered surface block") else {
            panic!("expected surface sample block");
        };
        let sample = samples.sample(-1, -1).expect("requested sample");
        assert_eq!(sample.height, 1);
        assert_eq!(sample.material, Material::Stone);
        assert_eq!(sample.water_level, Some(3));
        assert_eq!(sample.temperature, 0.25);
        assert_eq!(sample.moisture, 0.75);
        assert_eq!(sample.ridge, 0.5);

        let search = SurfaceSearchRequest {
            origin: [0, 0],
            min_radius: 0,
            max_radius: 0,
            kind: SurfaceSearchKind::WaterDepthAtLeast { depth_voxels: 2 },
        };
        let WorldProduct::SurfaceSearch(search) =
            product(&source, WorldProductRequest::SurfaceSearch(search))
                .expect("covered surface search")
        else {
            panic!("expected surface search");
        };
        assert_eq!(search.hit.map(|hit| hit.coord), Some([0, 0]));

        let tile_coord = SurfaceTileCoord::new(SurfaceLodLevel::Stride2, 0, 0);
        let WorldProduct::SurfaceTile(tile) =
            product(&source, WorldProductRequest::SurfaceTile(tile_coord))
                .expect("covered surface tile")
        else {
            panic!("expected surface tile");
        };
        assert_eq!(tile.terrain.coord, tile_coord);
        assert_eq!(tile.water.coord, tile_coord);
        assert!(!tile.terrain.quads.is_empty());
        assert!(!tile.water.quads.is_empty());
        assert_eq!(tile.source_identity_hash, source.identity().identity_hash());
    }

    fn chunk_material_at(chunk: &ChunkSnapshot, x: i32, y: i32, z: i32) -> Material {
        let coord = VoxelCoord::new(x, y, z);
        if coord.chunk() == chunk.chunk.coord() {
            let [x, y, z] = coord.local();
            chunk.chunk.get(x, y, z)
        } else {
            chunk
                .meshing_halo
                .sample_world(x, y, z)
                .unwrap_or(Material::Air)
        }
    }

    #[test]
    fn unavailable_and_malformed_macro_products_fail_terminally_per_item() {
        for (behavior, expected) in [
            (
                FakeBehavior::Unavailable,
                WorldSourceError::SourceCoverageUnavailable,
            ),
            (
                FakeBehavior::MalformedLength,
                WorldSourceError::MalformedMacroBlock,
            ),
            (
                FakeBehavior::WrongIdentity,
                WorldSourceError::MalformedMacroBlock,
            ),
        ] {
            let source = heightfield(behavior);
            assert_eq!(
                product(
                    &source,
                    WorldProductRequest::ChunkWithHalo(ChunkCoord::new(0, 0, 0)),
                ),
                Err(expected)
            );
        }
    }

    #[test]
    fn edited_surface_tiles_change_and_restore_without_a_generator() {
        let source = heightfield(FakeBehavior::Valid);
        let coord = SurfaceTileCoord::new(SurfaceLodLevel::Stride2, 0, 0);
        let pristine = source
            .generate_edited_surface_tile(&EditMap::default(), coord)
            .expect("pristine tile");
        let target = VoxelCoord::new(1, 1, 1);
        let mut edits = EditMap::default();
        edits.insert_override(target, Material::Air);
        let excavated = source
            .generate_edited_surface_tile(&edits, coord)
            .expect("edited tile");
        assert_ne!(excavated.terrain, pristine.terrain);
        assert!(
            source
                .surface_tiles_affected_by_voxel(&edits, coord.level, target)
                .contains(&coord)
        );

        edits.replace_durable_override(target, None);
        assert_eq!(
            source
                .generate_edited_surface_tile(&edits, coord)
                .expect("restored tile"),
            pristine
        );

        edits.insert_override(VoxelCoord::new(1, 3, 1), Material::Air);
        let drained = source
            .generate_edited_surface_tile(&edits, coord)
            .expect("edited water tile");
        assert_ne!(drained.water, pristine.water);
    }

    #[test]
    fn checked_snapshot_constructors_reject_shape_and_coordinate_mismatches() {
        let identity = WorldSourceIdentityHash::from_bytes([7; 32]);
        let coord = ChunkCoord::new(0, 0, 0);
        assert!(MeshingHalo::from_voxels(coord, vec![Material::Air; 1]).is_none());
        let halo = MeshingHalo::from_voxels(coord, vec![Material::Air; MESHING_HALO_VOXELS])
            .expect("exact halo length");
        assert!(
            ChunkSnapshot::new(identity, Chunk::empty(ChunkCoord::new(1, 0, 0)), halo).is_none()
        );
        assert!(
            VoxelBlockSnapshot::from_materials(
                identity,
                VoxelBlockRequest {
                    min: VoxelCoord::new(0, 0, 0),
                    sample_shape: [2, 2, 2],
                },
                vec![Material::Air; 7],
            )
            .is_none()
        );
        assert!(
            SurfaceSampleBlockSnapshot::from_samples(
                identity,
                SurfaceSampleBlockRequest {
                    origin: [0, 0],
                    sample_shape: [2, 2],
                },
                Vec::new(),
            )
            .is_none()
        );
    }
}
