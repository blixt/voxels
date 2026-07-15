//! Canonical heightfield composition over a [`MacroTerrainSource`].
//!
//! The macro source owns elevation and climate. This adapter deterministically turns those fields
//! into biome surfaces, shallow soil, geology, and bounded sub-30 m relief while preserving the
//! learned macro shape. It does not invent caves, vegetation, routes, or authored atlas content.

use crate::{
    AtmosphereSample, CHUNK_EDGE, Chunk, ChunkCoord, ChunkSnapshot, EditMap,
    MACRO_FIELD_SCHEMA_VERSION, MAX_MACRO_BLOCK_SAMPLES, MAX_SURFACE_SAMPLE_BLOCK_SAMPLES,
    MAX_SURFACE_SEARCH_RADIUS, MAX_VOXEL_BLOCK_SAMPLES, MAX_WORLD_PRODUCT_BATCH, MacroBlock,
    MacroBlockBatch, MacroBlockRequest, MacroTerrainSource, Material, MeshingHalo,
    SURFACE_TILE_EDGE_CELLS, SkylineFeature, SkylineFeatureKind, SurfaceLodLevel, SurfaceRegion,
    SurfaceSample, SurfaceSampleBlockRequest, SurfaceSampleBlockSnapshot, SurfaceSearchHit,
    SurfaceSearchKind, SurfaceSearchRequest, SurfaceSearchSnapshot, SurfaceTileCoord,
    SurfaceTileSnapshot, VoxelBlockRequest, VoxelBlockSnapshot, VoxelCoord, WorldProduct,
    WorldProductBatch, WorldProductBatchItem, WorldProductBatchResult, WorldProductPriority,
    WorldProductRequest, WorldSourceEngine, WorldSourceError, WorldSourceIdentity,
    WorldSourceIdentityHash, generate_surface_tile_mesh_with, generate_water_tile_mesh_with,
    surface_tiles_affected_by_column,
};
use std::collections::BTreeMap;

const SURFACE_TILE_SAMPLE_EDGE: u32 = 34;

/// Deterministic field-to-voxel adapter for non-procedural macro sources.
pub struct HeightfieldWorldSource {
    source: Box<dyn MacroTerrainSource>,
    identity: WorldSourceIdentity,
    identity_hash: WorldSourceIdentityHash,
    composer_seed: u64,
    add_subgrid_relief: bool,
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
        let composer_seed = u64::from_le_bytes(
            identity_hash.as_bytes()[..8]
                .try_into()
                .expect("identity hashes always contain eight seed bytes"),
        );
        let add_subgrid_relief = matches!(
            identity.source_kind,
            crate::WorldSourceKind::TerrainDiffusion30m
        );
        Ok(Self {
            source,
            identity,
            identity_hash,
            composer_seed,
            add_subgrid_relief,
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
            let sample_x = index % request.sample_shape[0] as usize;
            let sample_z = index / request.sample_shape[0] as usize;
            let world_x =
                i64::from(request.origin[0]) + sample_x as i64 * i64::from(request.stride_voxels);
            let world_z =
                i64::from(request.origin[1]) + sample_z as i64 * i64::from(request.stride_voxels);
            let world_x =
                i32::try_from(world_x).map_err(|_| WorldSourceError::InvalidBlockCoordinate)?;
            let world_z =
                i32::try_from(world_z).map_err(|_| WorldSourceError::InvalidBlockCoordinate)?;
            let relief = if self.add_subgrid_relief {
                micro_relief_voxels(self.composer_seed, world_x, world_z, block.ridge[index])
            } else {
                0.0
            };
            let height = f64::from(elevation + relief).floor();
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
                geology: geology_signal(self.composer_seed, world_x, world_z),
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
        let aliases = self.collidable_edit_aliases(edits, coord, priority)?;
        let terrain = generate_surface_tile_mesh_with(coord, |x, z| {
            let sampled = self
                .edited_surface(&region, edits, x, z)
                .unwrap_or((i32::MIN, Material::Stone));
            aliases
                .get(&(x, z))
                .copied()
                .filter(|(height, _)| *height >= sampled.0)
                .unwrap_or(sampled)
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

    fn collidable_edit_aliases(
        &self,
        edits: &EditMap,
        coord: SurfaceTileCoord,
        priority: WorldProductPriority,
    ) -> Result<BTreeMap<(i32, i32), (i32, Material)>, WorldSourceError> {
        let [origin_x, origin_z] = coord.voxel_origin();
        let stride = coord.stride_voxels();
        let halo_span = (SURFACE_TILE_EDGE_CELLS + 1).saturating_mul(stride);
        let bounds = [
            [
                origin_x.saturating_sub(stride),
                origin_z.saturating_sub(stride),
            ],
            [
                origin_x.saturating_add(halo_span),
                origin_z.saturating_add(halo_span),
            ],
        ];
        let coordinates = edits.edited_column_coordinates_in(bounds);
        let mut aliases = BTreeMap::new();
        for coordinate_batch in coordinates.chunks(MAX_WORLD_PRODUCT_BATCH) {
            let requests = coordinate_batch
                .iter()
                .map(|&(x, z)| MacroBlockRequest {
                    origin: [x, z],
                    sample_shape: [1, 1],
                    stride_voxels: 1,
                })
                .collect::<Vec<_>>();
            let result = self.source.request_blocks(MacroBlockBatch {
                priority,
                requests: requests.clone(),
            })?;
            if result.source_identity_hash != self.identity_hash
                || result.blocks.len() != requests.len()
            {
                return Err(WorldSourceError::MalformedMacroBlock);
            }
            for (request, block) in requests.into_iter().zip(result.blocks) {
                let grid = self.validate_block(request, block)?;
                let [x, z] = request.origin;
                let column = grid
                    .column(x, z)
                    .ok_or(WorldSourceError::MalformedMacroBlock)?;
                let generated_surface = (
                    column.height,
                    surface_profile(column, self.sea_level_voxels).0,
                );
                let (height, material) =
                    edits.surface_sample_with(x, z, generated_surface, i32::MIN, |voxel| {
                        material_for_column(column, self.sea_level_voxels, voxel.y)
                    });
                if !material.is_collidable()
                    || edits.override_at(VoxelCoord::new(x, height, z)) != Some(material)
                {
                    continue;
                }
                let cell_x = (i64::from(x) - i64::from(origin_x)).div_euclid(i64::from(stride));
                let cell_z = (i64::from(z) - i64::from(origin_z)).div_euclid(i64::from(stride));
                if !(-1..=i64::from(SURFACE_TILE_EDGE_CELLS)).contains(&cell_x)
                    || !(-1..=i64::from(SURFACE_TILE_EDGE_CELLS)).contains(&cell_z)
                {
                    continue;
                }
                let sample_x =
                    i64::from(origin_x) + cell_x * i64::from(stride) + i64::from(stride / 2);
                let sample_z =
                    i64::from(origin_z) + cell_z * i64::from(stride) + i64::from(stride / 2);
                let Some((sample_x, sample_z)) = i32::try_from(sample_x)
                    .ok()
                    .zip(i32::try_from(sample_z).ok())
                else {
                    continue;
                };
                aliases
                    .entry((sample_x, sample_z))
                    .and_modify(|current: &mut (i32, Material)| {
                        if height > current.0 {
                            *current = (height, material);
                        }
                    })
                    .or_insert((height, material));
            }
        }
        Ok(aliases)
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
        let surface_material = surface_profile(column, self.sea_level_voxels).0;
        Ok(
            edits.surface_sample_with(x, z, (column.height, surface_material), i32::MIN, |coord| {
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
            surface_profile(&column, self.sea_level_voxels).1,
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
    geology: f32,
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
        let depth = column.height - y;
        let (surface, region) = surface_profile(column, sea_level_voxels);
        if depth == 0 {
            return surface;
        }
        let soil_depth = 5 + (column.moisture * 7.0).round() as i32;
        if depth <= soil_depth {
            return match surface {
                Material::Grass | Material::Moss | Material::Snow => Material::Dirt,
                Material::Sand | Material::RedSand | Material::Clay => surface,
                Material::Basalt | Material::Limestone => surface,
                _ => Material::Stone,
            };
        }
        let band = (y.div_euclid(48) + (column.geology * 4.0).round() as i32).rem_euclid(13);
        if matches!(region, SurfaceRegion::Volcanic) || (column.geology < -0.58 && band < 5) {
            Material::Basalt
        } else if column.geology > 0.38 && band < 8 {
            Material::Limestone
        } else {
            Material::Stone
        }
    } else if y <= sea_level_voxels {
        Material::Water
    } else {
        Material::Air
    }
}

fn surface_sample(column: &PreparedColumn, sea_level_voxels: i32) -> SurfaceSample {
    let (material, region) = surface_profile(column, sea_level_voxels);
    SurfaceSample {
        height: column.height,
        material,
        water_level: (column.height < sea_level_voxels).then_some(sea_level_voxels),
        region,
        moisture: column.moisture,
        temperature: column.temperature,
        ridge: column.ridge,
        route: None,
    }
}

fn surface_profile(column: &PreparedColumn, sea_level_voxels: i32) -> (Material, SurfaceRegion) {
    let region = if column.temperature < 0.28
        || (column.ridge > 0.72 && column.temperature < 0.56)
        || column.height > sea_level_voxels.saturating_add(12_000)
    {
        SurfaceRegion::Alpine
    } else if column.temperature > 0.62 && column.moisture < 0.20 {
        SurfaceRegion::RedBadlands
    } else if column.temperature > 0.50 && column.moisture < 0.34 {
        SurfaceRegion::PaleDunes
    } else if column.ridge > 0.76 && column.geology < -0.18 {
        SurfaceRegion::Volcanic
    } else if column.moisture > 0.60 {
        SurfaceRegion::VerdantForest
    } else {
        SurfaceRegion::WindMoor
    };
    let material = if column.height <= sea_level_voxels.saturating_add(3) {
        if column.moisture > 0.62 || column.geology < -0.45 {
            Material::Clay
        } else {
            Material::Sand
        }
    } else {
        match region {
            SurfaceRegion::VerdantForest => {
                if column.moisture > 0.78 && column.geology > 0.12 {
                    Material::Moss
                } else {
                    Material::Grass
                }
            }
            SurfaceRegion::WindMoor => {
                // Learned 30 m terrain carries important escarpments even when its regional
                // climate occupies a narrow band. Expose coherent bedrock on those faces instead
                // of painting an entire rugged tile with grass.
                if column.ridge > 0.30 && column.geology < -0.30 {
                    Material::Basalt
                } else if (column.ridge > 0.30 && column.geology > 0.30)
                    || (column.ridge > 0.18 && column.geology > 0.56)
                {
                    Material::Limestone
                } else if column.ridge > 0.30 {
                    Material::Stone
                } else {
                    Material::Grass
                }
            }
            SurfaceRegion::Alpine => {
                if column.temperature < 0.44 && column.ridge < 0.88 {
                    Material::Snow
                } else if column.geology > 0.34 {
                    Material::Limestone
                } else {
                    Material::Stone
                }
            }
            SurfaceRegion::RedBadlands => {
                if column.geology > -0.20 {
                    Material::RedSand
                } else {
                    Material::Clay
                }
            }
            SurfaceRegion::PaleDunes => {
                if column.geology > 0.62 {
                    Material::Limestone
                } else {
                    Material::Sand
                }
            }
            SurfaceRegion::Volcanic => {
                if column.geology < 0.52 {
                    Material::Basalt
                } else {
                    Material::RedSand
                }
            }
        }
    };
    (material, region)
}

fn micro_relief_voxels(seed: u64, x: i32, z: i32, ridge: f32) -> f32 {
    let ridge = ridge.clamp(0.0, 1.0).powf(1.35);
    let broad = coherent_noise(seed ^ 0x6a09_e667, x, z, 600);
    let medium = coherent_noise(seed ^ 0xbb67_ae85, x, z, 170);
    let fine = coherent_noise(seed ^ 0x3c6e_f372, x, z, 55);
    broad * (3.0 + ridge * 18.0) + medium * (1.0 + ridge * 5.0) + fine * (0.25 + ridge * 1.5)
}

fn geology_signal(seed: u64, x: i32, z: i32) -> f32 {
    (coherent_noise(seed ^ 0xa54f_f53a, x, z, 1_800) * 0.68
        + coherent_noise(seed ^ 0x510e_527f, x, z, 520) * 0.32)
        .clamp(-1.0, 1.0)
}

fn coherent_noise(seed: u64, x: i32, z: i32, period: i32) -> f32 {
    let cell_x = x.div_euclid(period);
    let cell_z = z.div_euclid(period);
    let fraction_x = x.rem_euclid(period) as f32 / period as f32;
    let fraction_z = z.rem_euclid(period) as f32 / period as f32;
    let weight_x = fraction_x * fraction_x * (3.0 - 2.0 * fraction_x);
    let weight_z = fraction_z * fraction_z * (3.0 - 2.0 * fraction_z);
    let upper = lerp_noise(
        hash_noise(seed, cell_x, cell_z),
        hash_noise(seed, cell_x + 1, cell_z),
        weight_x,
    );
    let lower = lerp_noise(
        hash_noise(seed, cell_x, cell_z + 1),
        hash_noise(seed, cell_x + 1, cell_z + 1),
        weight_x,
    );
    lerp_noise(upper, lower, weight_z)
}

fn hash_noise(seed: u64, x: i32, z: i32) -> f32 {
    let mut value = seed
        ^ (x as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ (z as u64).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^= value >> 31;
    (value >> 40) as f32 * (2.0 / 16_777_215.0) - 1.0
}

fn lerp_noise(left: f32, right: f32, weight: f32) -> f32 {
    left + (right - left) * weight
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

    fn column(
        height: i32,
        temperature: f32,
        moisture: f32,
        ridge: f32,
        geology: f32,
    ) -> PreparedColumn {
        PreparedColumn {
            height,
            temperature,
            moisture,
            ridge,
            geology,
        }
    }

    #[test]
    fn climate_slope_and_geology_classify_distinct_surfaces_and_strata() {
        let cases = [
            (
                column(100, 0.60, 0.82, 0.20, 0.25),
                Material::Moss,
                SurfaceRegion::VerdantForest,
            ),
            (
                column(100, 0.20, 0.60, 0.50, 0.00),
                Material::Snow,
                SurfaceRegion::Alpine,
            ),
            (
                column(100, 0.80, 0.10, 0.30, 0.00),
                Material::RedSand,
                SurfaceRegion::RedBadlands,
            ),
            (
                column(100, 0.70, 0.30, 0.30, 0.00),
                Material::Sand,
                SurfaceRegion::PaleDunes,
            ),
            (
                column(100, 0.58, 0.40, 0.90, -0.50),
                Material::Basalt,
                SurfaceRegion::Volcanic,
            ),
            (
                column(100, 0.50, 0.45, 0.30, 0.70),
                Material::Limestone,
                SurfaceRegion::WindMoor,
            ),
            (
                column(100, 0.50, 0.45, 0.40, 0.00),
                Material::Stone,
                SurfaceRegion::WindMoor,
            ),
            (
                column(100, 0.50, 0.45, 0.40, -0.50),
                Material::Basalt,
                SurfaceRegion::WindMoor,
            ),
        ];
        for (column, material, region) in cases {
            assert_eq!(surface_profile(&column, 0), (material, region));
            assert_eq!(material_for_column(&column, 0, column.height), material);
        }

        let meadow = column(100, 0.60, 0.82, 0.20, 0.25);
        assert_eq!(material_for_column(&meadow, 0, 99), Material::Dirt);
        assert_eq!(material_for_column(&meadow, 0, 80), Material::Stone);
        let lava_field = column(100, 0.58, 0.40, 0.90, -0.70);
        assert_eq!(material_for_column(&lava_field, 0, 80), Material::Basalt);
        let shore = column(1, 0.60, 0.82, 0.20, -0.60);
        assert_eq!(surface_profile(&shore, 3).0, Material::Clay);
    }

    #[test]
    fn subgrid_relief_is_continuous_deterministic_and_slope_sensitive() {
        let seed = 0xfeed_beef;
        let first = micro_relief_voxels(seed, -321, 654, 0.8);
        assert_eq!(first, micro_relief_voxels(seed, -321, 654, 0.8));
        assert!((first - micro_relief_voxels(seed, -320, 654, 0.8)).abs() < 4.0);

        let range = |ridge| {
            let values = (0..200)
                .map(|step| micro_relief_voxels(seed, step * 7 - 500, step * 3 + 100, ridge))
                .collect::<Vec<_>>();
            let minimum = values.iter().copied().fold(f32::INFINITY, f32::min);
            let maximum = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            maximum - minimum
        };
        assert!(range(0.9) > range(0.0) * 3.0);
        assert!(range(0.9) < 60.0, "micro relief must remain below 6 metres");
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
    fn all_products_share_one_biome_strata_water_and_air_composition() {
        let source = heightfield(FakeBehavior::Valid);
        let coord = ChunkCoord::new(0, 0, 0);
        let WorldProduct::Chunk(chunk) =
            product(&source, WorldProductRequest::ChunkWithHalo(coord)).expect("covered chunk")
        else {
            panic!("expected chunk product");
        };
        assert_eq!(chunk.chunk.get(0, 1, 0), Material::Clay);
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
        assert_eq!(sample.material, Material::Clay);
        assert_eq!(sample.water_level, Some(3));
        assert_eq!(sample.region, SurfaceRegion::Alpine);
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
    fn off_lattice_player_tower_is_promoted_into_every_coarse_surface_tile() {
        let source = heightfield(FakeBehavior::Valid);
        let mut edits = EditMap::default();
        for y in 2..=45 {
            edits.insert_override(VoxelCoord::new(1, y, 1), Material::Dirt);
        }
        for level in SurfaceLodLevel::ALL {
            let coord = SurfaceTileCoord::new(level, 0, 0);
            let pristine = source
                .generate_edited_surface_tile(&EditMap::default(), coord)
                .expect("pristine tile");
            let built = source
                .generate_edited_surface_tile(&edits, coord)
                .expect("edited tile");
            assert_ne!(
                built.terrain, pristine.terrain,
                "off-lattice tower vanished at {level:?}"
            );
        }
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
