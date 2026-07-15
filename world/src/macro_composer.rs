//! Canonical heightfield composition over a [`MacroTerrainSource`].
//!
//! The macro source owns elevation and climate. This adapter deterministically turns those fields
//! into biome surfaces, shallow soil, geology, and bounded sub-30 m relief while preserving the
//! learned macro shape. It adds deterministic climate-driven vegetation, but does not invent caves,
//! routes, or authored atlas content.

use crate::{
    AtmosphereSample, CHUNK_EDGE, Chunk, ChunkCoord, ChunkSnapshot, EditMap,
    FEATURE_MAX_RADIUS_VOXELS, MACRO_FIELD_SCHEMA_VERSION, MAX_MACRO_BLOCK_SAMPLES,
    MAX_SURFACE_SAMPLE_BLOCK_SAMPLES, MAX_SURFACE_SEARCH_RADIUS, MAX_VOXEL_BLOCK_SAMPLES,
    MAX_WORLD_PRODUCT_BATCH, MacroBlock, MacroBlockBatch, MacroBlockRequest, MacroTerrainSource,
    Material, MeshingHalo, SURFACE_TILE_EDGE_CELLS, SkylineFeature, SkylineFeatureId,
    SkylineFeatureKind, SurfaceLodLevel, SurfaceRegion, SurfaceSample, SurfaceSampleBlockRequest,
    SurfaceSampleBlockSnapshot, SurfaceSearchHit, SurfaceSearchKind, SurfaceSearchRequest,
    SurfaceSearchSnapshot, SurfaceTileCoord, SurfaceTileSnapshot, TreeSpecies, VoxelBlockRequest,
    VoxelBlockSnapshot, VoxelCoord, WorldProduct, WorldProductBatch, WorldProductBatchItem,
    WorldProductBatchResult, WorldProductPriority, WorldProductRequest, WorldSourceEngine,
    WorldSourceError, WorldSourceIdentity, WorldSourceIdentityHash,
    generate_surface_tile_mesh_with_features, generate_water_tile_mesh_with,
    surface_tiles_affected_by_column,
};
use std::collections::BTreeMap;

const SURFACE_TILE_SAMPLE_EDGE: u32 = 34;
const ECOLOGY_CELL_VOXELS: i32 = 64;
const ECOLOGY_MIN_TREE_SPACING_VOXELS: i32 = 30;
type SurfaceAliasMap = BTreeMap<(i32, i32), (i32, Material)>;

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
        let mut composer_seed_bytes = [0_u8; 8];
        composer_seed_bytes.copy_from_slice(&identity_hash.as_bytes()[..8]);
        let composer_seed = u64::from_le_bytes(composer_seed_bytes);
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
            if !elevation.is_finite()
                || !normalized(block.temperature[index])
                || !normalized(block.moisture[index])
                || !normalized(block.ridge[index])
            {
                return Err(WorldSourceError::MalformedMacroBlock);
            }
            let ridge = block.ridge[index];
            let relief = if self.add_subgrid_relief {
                micro_relief_voxels(self.composer_seed, world_x, world_z, ridge)
            } else {
                0.0
            };
            let height = f64::from(elevation + relief).floor();
            if height < f64::from(i32::MIN) || height > f64::from(i32::MAX) {
                return Err(WorldSourceError::MalformedMacroBlock);
            }
            let (temperature, moisture, ecotone) = if self.add_subgrid_relief {
                downscaled_climate(
                    self.composer_seed,
                    world_x,
                    world_z,
                    block.temperature[index],
                    block.moisture[index],
                    ridge,
                )
            } else {
                (block.temperature[index], block.moisture[index], 0.0)
            };
            columns.push(PreparedColumn {
                height: height as i32,
                temperature,
                moisture,
                ridge,
                geology: geology_signal(self.composer_seed, world_x, world_z),
                ecotone,
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
        let features = self.ecology_features_anchored_in(
            [
                [
                    min_x.saturating_sub(FEATURE_MAX_RADIUS_VOXELS),
                    min_z.saturating_sub(FEATURE_MAX_RADIUS_VOXELS),
                ],
                [
                    max_x.saturating_add(FEATURE_MAX_RADIUS_VOXELS + 1),
                    max_z.saturating_add(FEATURE_MAX_RADIUS_VOXELS + 1),
                ],
            ],
            priority,
        )?;

        let mut chunk = Chunk::empty(coord);
        for y in 0..CHUNK_EDGE {
            for z in 0..CHUNK_EDGE {
                for x in 0..CHUNK_EDGE {
                    let world_x = origin[0] + x as i32;
                    let world_y = origin[1] + y as i32;
                    let world_z = origin[2] + z as i32;
                    let material = self
                        .material_at_with_features(&region, &features, world_x, world_y, world_z)?;
                    chunk.set(x, y, z, material);
                }
            }
        }
        let halo = MeshingHalo::from_sampler(coord, |x, y, z| {
            self.material_at_with_features(&region, &features, x, y, z)
                .unwrap_or(Material::Air)
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
        let max_x = request.min.x.saturating_add(width as i32 - 1);
        let max_z = request.min.z.saturating_add(depth as i32 - 1);
        let features = self.ecology_features_anchored_in(
            [
                [
                    request.min.x.saturating_sub(FEATURE_MAX_RADIUS_VOXELS),
                    request.min.z.saturating_sub(FEATURE_MAX_RADIUS_VOXELS),
                ],
                [
                    max_x.saturating_add(FEATURE_MAX_RADIUS_VOXELS + 1),
                    max_z.saturating_add(FEATURE_MAX_RADIUS_VOXELS + 1),
                ],
            ],
            priority,
        )?;
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
                    materials.push(self.material_at_with_features(
                        &region,
                        &features,
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
        let mut features = self.ecology_features_anchored_in(coord.voxel_bounds_xz(), priority)?;
        features.retain(|feature| {
            edits.skyline_feature_is_pristine_with(*feature, |coord| {
                feature.material_at(coord).unwrap_or(Material::Air)
            })
        });
        let terrain = generate_surface_tile_mesh_with_features(
            coord,
            |x, z| {
                let sampled = self
                    .edited_surface(&region, edits, x, z)
                    .unwrap_or((i32::MIN, Material::Stone));
                aliases
                    .get(&(x, z))
                    .copied()
                    .filter(|(height, _)| *height >= sampled.0)
                    .unwrap_or(sampled)
            },
            &features,
        );
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
    ) -> Result<SurfaceAliasMap, WorldSourceError> {
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

    fn material_at_with_features(
        &self,
        region: &PreparedMacroRegion,
        features: &[SkylineFeature],
        x: i32,
        y: i32,
        z: i32,
    ) -> Result<Material, WorldSourceError> {
        let terrain = self.material_at(region, x, y, z)?;
        if terrain != Material::Air {
            return Ok(terrain);
        }
        let coord = VoxelCoord::new(x, y, z);
        Ok(features
            .iter()
            .copied()
            .find_map(|feature| feature.material_at(coord))
            .unwrap_or(Material::Air))
    }

    fn ecology_features_anchored_in(
        &self,
        bounds: [[i32; 2]; 2],
        priority: WorldProductPriority,
    ) -> Result<Vec<SkylineFeature>, WorldSourceError> {
        if !self.add_subgrid_relief {
            return Ok(Vec::new());
        }
        let candidates = ecology_candidates_in(self.composer_seed, bounds)?;
        let mut features = Vec::new();
        for candidate_batch in candidates.chunks(MAX_WORLD_PRODUCT_BATCH) {
            let requests = candidate_batch
                .iter()
                .map(|candidate| MacroBlockRequest {
                    origin: [candidate.x, candidate.z],
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
            for ((candidate, request), block) in candidate_batch
                .iter()
                .copied()
                .zip(requests)
                .zip(result.blocks)
            {
                let grid = match self.validate_block(request, block) {
                    Ok(grid) => grid,
                    Err(WorldSourceError::SourceCoverageUnavailable) => continue,
                    Err(error) => return Err(error),
                };
                let column = grid
                    .column(candidate.x, candidate.z)
                    .ok_or(WorldSourceError::MalformedMacroBlock)?;
                if let Some(feature) =
                    ecology_tree(self.composer_seed, self.sea_level_voxels, candidate, column)
                {
                    features.push(feature);
                }
            }
        }
        Ok(features)
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
        let mut affected = surface_tiles_affected_by_column(level, coord.x, coord.z);
        for feature in self.skyline_features_at(coord) {
            if feature.material_at(coord).is_none() {
                continue;
            }
            let owner = SurfaceTileCoord::containing(level, feature.anchor[0], feature.anchor[2]);
            if owner.is_world_representable() && !affected.contains(&owner) {
                affected.push(owner);
            }
        }
        affected
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

    fn skyline_features_anchored_in(&self, bounds: [[i32; 2]; 2]) -> Vec<SkylineFeature> {
        self.ecology_features_anchored_in(bounds, WorldProductPriority::VisibleSurface)
            .unwrap_or_default()
    }

    fn skyline_features_at(&self, coord: VoxelCoord) -> Vec<SkylineFeature> {
        self.ecology_features_anchored_in(
            [
                [
                    coord.x.saturating_sub(FEATURE_MAX_RADIUS_VOXELS),
                    coord.z.saturating_sub(FEATURE_MAX_RADIUS_VOXELS),
                ],
                [
                    coord.x.saturating_add(FEATURE_MAX_RADIUS_VOXELS + 1),
                    coord.z.saturating_add(FEATURE_MAX_RADIUS_VOXELS + 1),
                ],
            ],
            WorldProductPriority::CollisionCritical,
        )
        .unwrap_or_default()
        .into_iter()
        .filter(|feature| feature.material_at(coord).is_some())
        .collect()
    }

    fn nearest_skyline_feature(
        &self,
        x: i32,
        z: i32,
        kind: SkylineFeatureKind,
        max_radius_cells: i32,
    ) -> Option<SkylineFeature> {
        if max_radius_cells < 0 {
            return None;
        }
        let radius = i64::from(max_radius_cells) * i64::from(crate::FEATURE_CELL_VOXELS);
        let clamp = |value: i64| value.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
        self.ecology_features_anchored_in(
            [
                [clamp(i64::from(x) - radius), clamp(i64::from(z) - radius)],
                [
                    clamp(i64::from(x) + radius + 1),
                    clamp(i64::from(z) + radius + 1),
                ],
            ],
            WorldProductPriority::Prefetch,
        )
        .ok()?
        .into_iter()
        .filter(|feature| feature.kind == kind)
        .min_by_key(|feature| {
            let dx = i64::from(feature.anchor[0]) - i64::from(x);
            let dz = i64::from(feature.anchor[2]) - i64::from(z);
            dx * dx + dz * dz
        })
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
    ecotone: f32,
}

#[derive(Clone, Copy, Debug)]
struct EcologyCandidate {
    cell_x: i32,
    cell_z: i32,
    x: i32,
    z: i32,
    priority: u64,
    occurrence: f32,
    species_draw: f32,
    variation: u8,
    orientation: u8,
    height_noise: u8,
}

#[derive(Clone, Copy, Debug)]
struct EcologyStructure {
    canopy: f32,
    gap: f32,
    succession: f32,
    warped_x: i32,
    warped_z: i32,
}

#[derive(Clone, Copy)]
struct TreeNiche {
    temperature: f32,
    temperature_width: f32,
    moisture: f32,
    moisture_width: f32,
    maximum_ridge: f32,
    prevalence: f32,
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

fn ecology_candidates_in(
    seed: u64,
    bounds: [[i32; 2]; 2],
) -> Result<Vec<EcologyCandidate>, WorldSourceError> {
    let [[min_x, min_z], [max_x, max_z]] = bounds;
    if min_x >= max_x || min_z >= max_z {
        return Err(WorldSourceError::InvalidBlockCoordinate);
    }
    let first_cell_x = min_x.div_euclid(ECOLOGY_CELL_VOXELS);
    let first_cell_z = min_z.div_euclid(ECOLOGY_CELL_VOXELS);
    let last_cell_x = max_x.saturating_sub(1).div_euclid(ECOLOGY_CELL_VOXELS);
    let last_cell_z = max_z.saturating_sub(1).div_euclid(ECOLOGY_CELL_VOXELS);
    let cell_count = (i64::from(last_cell_x) - i64::from(first_cell_x) + 1)
        .checked_mul(i64::from(last_cell_z) - i64::from(first_cell_z) + 1)
        .ok_or(WorldSourceError::BlockTooLarge)?;
    if cell_count > MAX_MACRO_BLOCK_SAMPLES as i64 {
        return Err(WorldSourceError::BlockTooLarge);
    }
    let mut candidates = Vec::with_capacity(cell_count as usize);
    for cell_z in first_cell_z..=last_cell_z {
        for cell_x in first_cell_x..=last_cell_x {
            let Some(candidate) = ecology_candidate(seed, cell_x, cell_z) else {
                continue;
            };
            if candidate.x < min_x
                || candidate.x >= max_x
                || candidate.z < min_z
                || candidate.z >= max_z
                || !ecology_candidate_wins_spacing(seed, candidate)
            {
                continue;
            }
            candidates.push(candidate);
        }
    }
    Ok(candidates)
}

fn ecology_candidate(seed: u64, cell_x: i32, cell_z: i32) -> Option<EcologyCandidate> {
    let placement = ecology_hash(seed ^ 0x6a09_e667_f3bc_c909, cell_x, cell_z);
    let form = ecology_hash(seed ^ 0xbb67_ae85_84ca_a73b, cell_x, cell_z);
    let origin_x = i64::from(cell_x) * i64::from(ECOLOGY_CELL_VOXELS);
    let origin_z = i64::from(cell_z) * i64::from(ECOLOGY_CELL_VOXELS);
    let offset_x = (placement % ECOLOGY_CELL_VOXELS as u64) as i32;
    let offset_z = ((placement >> 32) % ECOLOGY_CELL_VOXELS as u64) as i32;
    Some(EcologyCandidate {
        cell_x,
        cell_z,
        x: i32::try_from(origin_x + i64::from(offset_x)).ok()?,
        z: i32::try_from(origin_z + i64::from(offset_z)).ok()?,
        priority: ecology_hash(seed ^ 0x3c6e_f372_fe94_f82b, cell_x, cell_z),
        occurrence: hash_unit(ecology_hash(seed ^ 0xa54f_f53a_5f1d_36f1, cell_x, cell_z)),
        species_draw: hash_unit(ecology_hash(seed ^ 0x510e_527f_ade6_82d1, cell_x, cell_z)),
        variation: (form & 7) as u8,
        orientation: (ecology_hash(seed ^ 0x9b05_688c_2b3e_6c1f, cell_x, cell_z) & 3) as u8,
        height_noise: ecology_hash(seed ^ 0x1f83_d9ab_fb41_bd6b, cell_x, cell_z) as u8,
    })
}

fn ecology_candidate_wins_spacing(seed: u64, candidate: EcologyCandidate) -> bool {
    let minimum_distance_squared = i64::from(ECOLOGY_MIN_TREE_SPACING_VOXELS).pow(2);
    for dz in -1..=1 {
        for dx in -1..=1 {
            if dx == 0 && dz == 0 {
                continue;
            }
            let Some(neighbor) = ecology_candidate(
                seed,
                candidate.cell_x.saturating_add(dx),
                candidate.cell_z.saturating_add(dz),
            ) else {
                continue;
            };
            let delta_x = i64::from(neighbor.x) - i64::from(candidate.x);
            let delta_z = i64::from(neighbor.z) - i64::from(candidate.z);
            if delta_x * delta_x + delta_z * delta_z >= minimum_distance_squared {
                continue;
            }
            if (neighbor.priority, neighbor.cell_x, neighbor.cell_z)
                < (candidate.priority, candidate.cell_x, candidate.cell_z)
            {
                return false;
            }
        }
    }
    true
}

fn ecology_tree(
    seed: u64,
    sea_level_voxels: i32,
    candidate: EcologyCandidate,
    column: &PreparedColumn,
) -> Option<SkylineFeature> {
    if column.height <= sea_level_voxels.saturating_add(4) || column.ridge > 0.82 {
        return None;
    }
    let (surface, _) = surface_profile(column, sea_level_voxels);
    let structure = ecology_structure(seed, candidate.x, candidate.z);
    let mut scores = [0.0; TreeSpecies::ALL.len()];
    let mut best_index = 0;
    let mut best_score = 0.0_f32;
    for (index, species) in TreeSpecies::ALL.into_iter().enumerate() {
        let niche = tree_niche(species);
        let temperature = unimodal_suitability(
            column.temperature,
            niche.temperature,
            niche.temperature_width,
        );
        let moisture = unimodal_suitability(column.moisture, niche.moisture, niche.moisture_width);
        let slope = smooth_fall(
            column.ridge,
            niche.maximum_ridge * 0.58,
            niche.maximum_ridge,
        );
        let substrate = match surface {
            Material::Grass | Material::Moss | Material::Clay => 1.0,
            Material::Sand | Material::RedSand
                if matches!(species, TreeSpecies::Acacia | TreeSpecies::Juniper) =>
            {
                0.78
            }
            Material::Stone | Material::Limestone | Material::Basalt
                if matches!(species, TreeSpecies::Pine | TreeSpecies::Juniper) =>
            {
                0.42
            }
            _ => 0.08,
        };
        let stand = coherent_noise(
            seed ^ 0x510e_527f_ade6_82d1 ^ (species as u64).wrapping_mul(0x9e37_79b9),
            structure.warped_x,
            structure.warped_z,
            1_100,
        );
        let (succession_optimum, succession_width) = succession_niche(species);
        let succession =
            unimodal_suitability(structure.succession, succession_optimum, succession_width);
        let community = (0.78 + (stand * 0.5 + 0.5) * 0.24) * (0.84 + succession * 0.16);
        let score = (temperature.min(moisture) * slope * niche.prevalence * substrate * community)
            .clamp(0.0, 1.0);
        scores[index] = score;
        if score > best_score {
            best_index = index;
            best_score = score;
        }
    }
    let effective_canopy = structure.canopy * (1.0 - structure.gap * 0.94);
    let stand_density = smooth_rise(effective_canopy, 0.10, 0.82);
    let density = (best_score * (0.012 + stand_density * 0.91)).clamp(0.0, 0.92);
    if best_score < 0.24 || candidate.occurrence > density {
        return None;
    }
    let species = ecology_species_choice(scores, best_index, candidate.species_draw);
    let (minimum_height, height_span) = tree_height_range(species);
    let height = minimum_height + i32::from(candidate.height_noise) % height_span;
    Some(SkylineFeature {
        id: SkylineFeatureId {
            cell_x: candidate.cell_x,
            cell_z: candidate.cell_z,
        },
        kind: SkylineFeatureKind::Broadleaf,
        anchor: [candidate.x, column.height, candidate.z],
        trunk_top: column.height.checked_add(height)?,
        orientation: candidate.orientation,
        variant: species.encode_variant(candidate.variation),
        prominence: 0,
        route_landmark: None,
    })
}

fn ecology_species_choice(
    scores: [f32; TreeSpecies::ALL.len()],
    best_index: usize,
    draw: f32,
) -> TreeSpecies {
    if draw < 0.78 {
        return TreeSpecies::ALL[best_index];
    }
    let minimum = scores[best_index] * 0.38;
    let weights = scores.map(|score| (score - minimum).max(0.0).powi(2));
    let total = weights.iter().sum::<f32>();
    if total <= f32::EPSILON {
        return TreeSpecies::ALL[best_index];
    }
    let mut target = ((draw - 0.78) / 0.22).clamp(0.0, 1.0) * total;
    for (species, weight) in TreeSpecies::ALL.into_iter().zip(weights) {
        target -= weight;
        if target <= 0.0 {
            return species;
        }
    }
    TreeSpecies::ALL[best_index]
}

fn succession_niche(species: TreeSpecies) -> (f32, f32) {
    match species {
        TreeSpecies::Birch | TreeSpecies::Aspen => (0.18, 0.52),
        TreeSpecies::Alder | TreeSpecies::Pine | TreeSpecies::Acacia => (0.32, 0.58),
        TreeSpecies::Larch | TreeSpecies::Juniper | TreeSpecies::Willow => (0.48, 0.62),
        TreeSpecies::Oak | TreeSpecies::Spruce => (0.68, 0.58),
        TreeSpecies::Beech | TreeSpecies::Fir => (0.84, 0.50),
    }
}

fn ecology_structure(seed: u64, x: i32, z: i32) -> EcologyStructure {
    let warp_x = coherent_noise(seed ^ 0x428a_2f98, x, z, 5_300) * 720.0;
    let warp_z = coherent_noise(seed ^ 0x7137_4491, x, z, 4_700) * 720.0;
    let warped_x = offset_coordinate(x, warp_x);
    let warped_z = offset_coordinate(z, warp_z);
    let canopy_signal = coherent_noise(seed ^ 0xb5c0_fbcf, warped_x, warped_z, 5_100) * 0.56
        + coherent_noise(seed ^ 0xe9b5_dba5, warped_x, warped_z, 1_750) * 0.30
        + coherent_noise(seed ^ 0x3956_c25b, warped_x, warped_z, 570) * 0.14;
    let canopy = smooth_rise(canopy_signal, -0.22, 0.30);
    let gap_signal = coherent_noise(seed ^ 0x59f1_11f1, warped_x, warped_z, 760) * 0.68
        + coherent_noise(seed ^ 0x923f_82a4, warped_x, warped_z, 270) * 0.32;
    let gap = smooth_rise(gap_signal, 0.38, 0.72) * smooth_rise(canopy, 0.34, 0.74);
    let succession_signal = coherent_noise(seed ^ 0xab1c_5ed5, warped_x, warped_z, 2_600) * 0.72
        + coherent_noise(seed ^ 0xd807_aa98, warped_x, warped_z, 680) * 0.28;
    EcologyStructure {
        canopy,
        gap,
        succession: (succession_signal * 0.5 + 0.5).clamp(0.0, 1.0),
        warped_x,
        warped_z,
    }
}

fn offset_coordinate(coordinate: i32, offset: f32) -> i32 {
    (i64::from(coordinate) + offset.round() as i64).clamp(i64::from(i32::MIN), i64::from(i32::MAX))
        as i32
}

const fn tree_niche(species: TreeSpecies) -> TreeNiche {
    match species {
        TreeSpecies::Oak => TreeNiche {
            temperature: 0.62,
            temperature_width: 0.34,
            moisture: 0.62,
            moisture_width: 0.38,
            maximum_ridge: 0.62,
            prevalence: 0.96,
        },
        TreeSpecies::Beech => TreeNiche {
            temperature: 0.52,
            temperature_width: 0.28,
            moisture: 0.72,
            moisture_width: 0.30,
            maximum_ridge: 0.54,
            prevalence: 0.90,
        },
        TreeSpecies::Birch => TreeNiche {
            temperature: 0.40,
            temperature_width: 0.36,
            moisture: 0.60,
            moisture_width: 0.36,
            maximum_ridge: 0.68,
            prevalence: 0.99,
        },
        TreeSpecies::Aspen => TreeNiche {
            temperature: 0.47,
            temperature_width: 0.34,
            moisture: 0.54,
            moisture_width: 0.36,
            maximum_ridge: 0.64,
            prevalence: 0.98,
        },
        TreeSpecies::Willow => TreeNiche {
            temperature: 0.60,
            temperature_width: 0.30,
            moisture: 0.90,
            moisture_width: 0.30,
            maximum_ridge: 0.34,
            prevalence: 0.82,
        },
        TreeSpecies::Alder => TreeNiche {
            temperature: 0.52,
            temperature_width: 0.34,
            moisture: 0.82,
            moisture_width: 0.32,
            maximum_ridge: 0.42,
            prevalence: 0.86,
        },
        TreeSpecies::Pine => TreeNiche {
            temperature: 0.48,
            temperature_width: 0.46,
            moisture: 0.42,
            moisture_width: 0.42,
            maximum_ridge: 0.76,
            prevalence: 0.92,
        },
        TreeSpecies::Spruce => TreeNiche {
            temperature: 0.32,
            temperature_width: 0.32,
            moisture: 0.68,
            moisture_width: 0.34,
            maximum_ridge: 0.68,
            prevalence: 0.90,
        },
        TreeSpecies::Fir => TreeNiche {
            temperature: 0.38,
            temperature_width: 0.32,
            moisture: 0.76,
            moisture_width: 0.30,
            maximum_ridge: 0.62,
            prevalence: 0.86,
        },
        TreeSpecies::Larch => TreeNiche {
            temperature: 0.28,
            temperature_width: 0.34,
            moisture: 0.50,
            moisture_width: 0.38,
            maximum_ridge: 0.72,
            prevalence: 0.98,
        },
        TreeSpecies::Juniper => TreeNiche {
            temperature: 0.56,
            temperature_width: 0.40,
            moisture: 0.28,
            moisture_width: 0.32,
            maximum_ridge: 0.74,
            prevalence: 0.70,
        },
        TreeSpecies::Acacia => TreeNiche {
            temperature: 0.82,
            temperature_width: 0.30,
            moisture: 0.20,
            moisture_width: 0.26,
            maximum_ridge: 0.48,
            prevalence: 0.74,
        },
    }
}

fn unimodal_suitability(value: f32, optimum: f32, width: f32) -> f32 {
    smooth_fall((value - optimum).abs(), width * 0.58, width)
}

const fn tree_height_range(species: TreeSpecies) -> (i32, i32) {
    match species {
        TreeSpecies::Oak => (55, 19),
        TreeSpecies::Beech => (62, 21),
        TreeSpecies::Birch => (48, 18),
        TreeSpecies::Aspen => (58, 22),
        TreeSpecies::Willow => (42, 18),
        TreeSpecies::Alder => (45, 19),
        TreeSpecies::Pine => (70, 27),
        TreeSpecies::Spruce => (76, 31),
        TreeSpecies::Fir => (68, 29),
        TreeSpecies::Larch => (65, 29),
        TreeSpecies::Juniper => (24, 16),
        TreeSpecies::Acacia => (38, 17),
    }
}

fn ecology_hash(seed: u64, x: i32, z: i32) -> u64 {
    let mut value = seed
        ^ (x as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ (z as u64).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn hash_unit(value: u64) -> f32 {
    (value >> 40) as f32 / 16_777_215.0
}

fn material_for_column(column: &PreparedColumn, sea_level_voxels: i32, y: i32) -> Material {
    if y <= column.height {
        let depth = i64::from(column.height) - i64::from(y);
        let (surface, region) = surface_profile(column, sea_level_voxels);
        if depth == 0 {
            return surface;
        }
        let soil_depth = 5 + (column.moisture * 7.0).round() as i32;
        if depth <= i64::from(soil_depth) {
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
    let region = surface_region(column, sea_level_voxels);
    let material = if column.height <= sea_level_voxels.saturating_add(3) {
        if column.moisture > 0.62 || column.geology < -0.45 {
            Material::Clay
        } else {
            Material::Sand
        }
    } else {
        match region {
            SurfaceRegion::VerdantForest => {
                if rock_exposure(column) > 0.72 {
                    exposed_rock(column)
                } else if column.moisture + column.ecotone * 0.08 > 0.72 && column.geology > 0.02 {
                    Material::Moss
                } else {
                    Material::Grass
                }
            }
            SurfaceRegion::WindMoor => {
                if rock_exposure(column) > 0.56 {
                    exposed_rock(column)
                } else if column.moisture + column.ecotone * 0.06 > 0.68 {
                    Material::Moss
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

fn surface_region(column: &PreparedColumn, sea_level_voxels: i32) -> SurfaceRegion {
    let temperature = column.temperature;
    let moisture = column.moisture;
    let ridge = column.ridge;
    let altitude = smooth_rise(
        column.height.saturating_sub(sea_level_voxels) as f32,
        9_000.0,
        13_000.0,
    );
    let temperate = smooth_rise(temperature, 0.22, 0.42) * smooth_fall(temperature, 0.72, 0.92);
    let cold = smooth_fall(temperature, 0.24, 0.42);
    let dry = smooth_fall(moisture, 0.12, 0.40);
    let hot = smooth_rise(temperature, 0.50, 0.78);

    // These are overlapping ecological affinities rather than ordered thresholds. The learned
    // climate remains the dominant signal, while continuous world-space variation bends broad
    // boundaries into ecotones instead of exposing the source model's sampling lattice.
    let scores = [
        smooth_rise(moisture, 0.38, 0.68) * temperate * smooth_fall(ridge, 0.30, 0.72) * 1.24
            + column.ecotone * 0.12,
        0.43 + temperate * 0.18 + smooth_rise(moisture, 0.24, 0.52) * 0.10 - column.ecotone * 0.04,
        cold * 1.36
            + smooth_rise(ridge, 0.56, 0.92) * smooth_fall(temperature, 0.42, 0.68) * 0.76
            + altitude * 1.40
            - column.ecotone * 0.05,
        hot * dry * 1.25 - column.ecotone * 0.08,
        smooth_rise(temperature, 0.44, 0.68) * smooth_fall(moisture, 0.20, 0.48) * 0.96
            + column.geology.max(0.0) * 0.08
            + column.ecotone * 0.06,
        smooth_rise(ridge, 0.50, 0.86) * smooth_fall(column.geology, -0.44, 0.10) * 1.38,
    ];
    let index = scores
        .into_iter()
        .enumerate()
        .max_by(|left, right| left.1.total_cmp(&right.1))
        .map_or(1, |(index, _)| index);
    SurfaceRegion::ALL[index]
}

fn rock_exposure(column: &PreparedColumn) -> f32 {
    (smooth_rise(column.ridge, 0.12, 0.42) + column.geology.abs() * 0.10 + column.ecotone * 0.12)
        .clamp(0.0, 1.0)
}

fn exposed_rock(column: &PreparedColumn) -> Material {
    let composition = column.geology + column.ecotone * 0.10;
    if composition < -0.28 {
        Material::Basalt
    } else if composition > 0.28 {
        Material::Limestone
    } else {
        Material::Stone
    }
}

fn smooth_rise(value: f32, start: f32, end: f32) -> f32 {
    let normalized = ((value - start) / (end - start)).clamp(0.0, 1.0);
    normalized * normalized * (3.0 - 2.0 * normalized)
}

fn smooth_fall(value: f32, start: f32, end: f32) -> f32 {
    1.0 - smooth_rise(value, start, end)
}

fn downscaled_climate(
    seed: u64,
    x: i32,
    z: i32,
    temperature: f32,
    moisture: f32,
    ridge: f32,
) -> (f32, f32, f32) {
    // Terrain Diffusion climate is intentionally macro-scale (its climate grid is much coarser
    // than the 30 m elevation product). Add bounded, seamless local variation as an ecological
    // downscaling layer; never normalize a requested tile, so overlapping products stay exact.
    let temperature_offset = coherent_noise(seed ^ 0x243f_6a88, x, z, 4_200) * 0.026
        + coherent_noise(seed ^ 0x85a3_08d3, x, z, 1_100) * 0.014;
    let drainage = coherent_noise(seed ^ 0x1319_8a2e, x, z, 3_200) * 0.72
        + coherent_noise(seed ^ 0x0370_7344, x, z, 850) * 0.28;
    let ecotone = (coherent_noise(seed ^ 0xa409_3822, x, z, 1_600) * 0.72
        + coherent_noise(seed ^ 0x299f_31d0, x, z, 460) * 0.28)
        .clamp(-1.0, 1.0);
    let temperature = (temperature + temperature_offset - ridge * 0.018).clamp(0.0, 1.0);
    let moisture = (moisture + drainage * 0.15 - ridge * 0.045).clamp(0.0, 1.0);
    (temperature, moisture, ecotone)
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
    use std::collections::BTreeSet;

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
        temperature: f32,
        moisture: f32,
        ridge: f32,
    }

    impl FakeMacroSource {
        fn new(behavior: FakeBehavior, elevation: f32) -> Self {
            Self {
                identity: WorldSourceIdentity::procedural_v16(42),
                behavior,
                elevation,
                temperature: 0.25,
                moisture: 0.75,
                ridge: 0.5,
            }
        }

        fn terrain_diffusion(elevation: f32, temperature: f32, moisture: f32, ridge: f32) -> Self {
            let mut source = Self::new(FakeBehavior::Valid, elevation);
            source.identity.source_kind = crate::WorldSourceKind::TerrainDiffusion30m;
            source.temperature = temperature;
            source.moisture = moisture;
            source.ridge = ridge;
            source
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
                    temperature: vec![self.temperature; sample_count],
                    moisture: vec![self.moisture; sample_count],
                    ridge: vec![self.ridge; sample_count],
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

    fn diffusion_heightfield() -> HeightfieldWorldSource {
        HeightfieldWorldSource::new(
            Box::new(FakeMacroSource::terrain_diffusion(12.0, 0.58, 0.62, 0.12)),
            0,
        )
        .expect("valid Terrain Diffusion fake source")
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
            ecotone: 0.0,
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
    fn deepest_voxel_below_a_high_provider_surface_uses_deep_geology() {
        let extreme = column(i32::MAX - 127, 0.5, 0.45, 0.4, 0.0);
        assert_eq!(material_for_column(&extreme, 0, i32::MIN), Material::Stone);
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

    #[test]
    fn climate_downscaling_is_seamless_bounded_and_preserves_macro_ordering() {
        let seed = 0x0ddc_0ffe;
        let left = downscaled_climate(seed, 3_199, -781, 0.58, 0.48, 0.22);
        let right = downscaled_climate(seed, 3_200, -781, 0.58, 0.48, 0.22);
        assert_eq!(
            left,
            downscaled_climate(seed, 3_199, -781, 0.58, 0.48, 0.22)
        );
        assert!((left.0 - right.0).abs() < 0.002);
        assert!((left.1 - right.1).abs() < 0.002);
        assert!((left.2 - right.2).abs() < 0.002);

        let mut moisture_minimum = f32::INFINITY;
        let mut moisture_maximum = f32::NEG_INFINITY;
        for step in -40..=40 {
            let x = step * 173;
            let z = step * step * 11 - 7_000;
            let local = downscaled_climate(seed, x, z, 0.58, 0.48, 0.22);
            let dry = downscaled_climate(seed, x, z, 0.58, 0.28, 0.22);
            assert!((0.0..=1.0).contains(&local.0));
            assert!((0.0..=1.0).contains(&local.1));
            assert!((-1.0..=1.0).contains(&local.2));
            assert!(local.1 > dry.1);
            moisture_minimum = moisture_minimum.min(local.1);
            moisture_maximum = moisture_maximum.max(local.1);
        }
        assert!(moisture_maximum - moisture_minimum > 0.08);
    }

    #[test]
    fn overlapping_affinities_form_an_ecotone_instead_of_a_straight_threshold() {
        let mut sheltered = column(100, 0.58, 0.55, 0.20, 0.0);
        sheltered.ecotone = 1.0;
        let mut exposed = sheltered;
        exposed.ecotone = -1.0;
        assert_eq!(surface_region(&sheltered, 0), SurfaceRegion::VerdantForest);
        assert_eq!(surface_region(&exposed, 0), SurfaceRegion::WindMoor);
    }

    #[test]
    fn diffusion_ecology_is_deterministic_canonical_lod_visible_and_edit_suppressed() {
        let source = diffusion_heightfield();
        let bounds = [[-2_048, -2_048], [2_048, 2_048]];
        let features = source.skyline_features_anchored_in(bounds);
        assert!(!features.is_empty());
        assert_eq!(features, source.skyline_features_anchored_in(bounds));
        assert!(
            features
                .iter()
                .all(|feature| feature.tree_species().is_some())
        );
        assert!(
            features
                .iter()
                .map(|feature| feature.tree_variation())
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                >= 3
        );

        let feature = features[0];
        let trunk = VoxelCoord::new(feature.anchor[0], feature.anchor[1] + 1, feature.anchor[2]);
        let WorldProduct::Chunk(chunk) =
            product(&source, WorldProductRequest::ChunkWithHalo(trunk.chunk()))
                .expect("tree chunk is covered")
        else {
            panic!("expected canonical chunk");
        };
        assert_eq!(
            chunk_material_at(&chunk, trunk.x, trunk.y, trunk.z),
            Material::Wood
        );

        let mut edits = EditMap::default();
        edits.insert_override(trunk, Material::Air);
        for level in SurfaceLodLevel::ALL {
            let owner = SurfaceTileCoord::containing(level, feature.anchor[0], feature.anchor[2]);
            let pristine = source
                .generate_edited_surface_tile(&EditMap::default(), owner)
                .expect("pristine ecology surface tile");
            let pristine_tree_quads = pristine
                .terrain
                .quads
                .iter()
                .filter(|quad| matches!(quad.material, Material::Wood | Material::Leaves))
                .count();
            assert!(pristine_tree_quads > 0, "tree absent at {level:?}");
            let edited = source
                .generate_edited_surface_tile(&edits, owner)
                .expect("edited ecology surface tile");
            let edited_tree_quads = edited
                .terrain
                .quads
                .iter()
                .filter(|quad| matches!(quad.material, Material::Wood | Material::Leaves))
                .count();
            assert!(
                edited_tree_quads < pristine_tree_quads,
                "edit absent at {level:?}"
            );
            assert!(
                source
                    .surface_tiles_affected_by_voxel(&edits, level, trunk)
                    .contains(&owner)
            );
        }
    }

    #[test]
    fn climate_niches_can_select_at_least_ten_tree_species() {
        let mut species = std::collections::BTreeSet::new();
        for temperature_step in 0..=20 {
            for moisture_step in 0..=20 {
                let candidate = EcologyCandidate {
                    cell_x: temperature_step,
                    cell_z: moisture_step,
                    x: temperature_step * 431 - 4_000,
                    z: moisture_step * 379 - 4_000,
                    priority: 0,
                    occurrence: 0.0,
                    species_draw: 0.0,
                    variation: ((temperature_step + moisture_step) & 7) as u8,
                    orientation: (temperature_step & 3) as u8,
                    height_noise: 7,
                };
                let column = PreparedColumn {
                    height: 100,
                    temperature: temperature_step as f32 / 20.0,
                    moisture: moisture_step as f32 / 20.0,
                    ridge: 0.10,
                    geology: 0.0,
                    ecotone: 0.0,
                };
                if let Some(feature) = ecology_tree(0x5eed, 0, candidate, &column)
                    && let Some(selected) = feature.tree_species()
                {
                    species.insert(selected);
                }
            }
        }
        assert!(
            species.len() >= 10,
            "climate catalogue collapsed to {species:?}"
        );
    }

    #[test]
    fn ecology_priority_thinning_is_active_and_enforces_cross_cell_spacing() {
        let seed = 0x5eed_5eed;
        let mut raw = 0;
        let mut accepted = BTreeMap::new();
        for cell_z in -80..=80 {
            for cell_x in -80..=80 {
                let candidate = ecology_candidate(seed, cell_x, cell_z).expect("bounded candidate");
                raw += 1;
                if ecology_candidate_wins_spacing(seed, candidate) {
                    accepted.insert((cell_x, cell_z), candidate);
                }
            }
        }
        assert!(
            accepted.len() < raw * 9 / 10,
            "spacing rejected too few candidates"
        );
        assert!(
            accepted.len() > raw / 2,
            "spacing rejected implausibly many candidates"
        );
        let minimum_squared = i64::from(ECOLOGY_MIN_TREE_SPACING_VOXELS).pow(2);
        for (&(cell_x, cell_z), candidate) in &accepted {
            for dz in -1..=1 {
                for dx in -1..=1 {
                    if (dx, dz) <= (0, 0) {
                        continue;
                    }
                    let Some(neighbor) = accepted.get(&(cell_x + dx, cell_z + dz)) else {
                        continue;
                    };
                    let delta_x = i64::from(neighbor.x) - i64::from(candidate.x);
                    let delta_z = i64::from(neighbor.z) - i64::from(candidate.z);
                    assert!(delta_x * delta_x + delta_z * delta_z >= minimum_squared);
                }
            }
        }
    }

    #[test]
    fn ecology_bounds_are_seamless_across_negative_and_positive_tiles() {
        let seed = 0xa11c_e5eed;
        let whole = ecology_candidates_in(seed, [[-4_317, -2_119], [5_083, 3_777]])
            .expect("whole ecology bounds");
        let mut tiled = Vec::new();
        for min_z in [-2_119, 1137] {
            let max_z = if min_z < 0 { 1137 } else { 3_777 };
            for min_x in [-4_317, -731, 2_041] {
                let max_x = match min_x {
                    -4_317 => -731,
                    -731 => 2_041,
                    _ => 5_083,
                };
                tiled.extend(
                    ecology_candidates_in(seed, [[min_x, min_z], [max_x, max_z]])
                        .expect("tiled ecology bounds"),
                );
            }
        }
        let key = |candidate: &EcologyCandidate| {
            (
                candidate.cell_x,
                candidate.cell_z,
                candidate.x,
                candidate.z,
                candidate.priority,
            )
        };
        assert_eq!(
            whole.iter().map(key).collect::<BTreeSet<_>>(),
            tiled.iter().map(key).collect::<BTreeSet<_>>()
        );
        assert_eq!(whole.len(), tiled.len());
    }

    #[test]
    fn multiscale_ecology_contains_dense_stands_open_land_and_mixed_species() {
        let seed = 0x0ddc_0ffe;
        let climate = column(100, 0.56, 0.70, 0.16, 0.10);
        let window_cells = 20;
        let mut counts = Vec::<u32>::new();
        let mut dense_species = BTreeSet::new();
        let mut dominant_mixed_stands = 0;
        for window_z in -8..8 {
            for window_x in -8..8 {
                let mut count = 0_u32;
                let mut species = BTreeMap::<TreeSpecies, u32>::new();
                for local_z in 0..window_cells {
                    for local_x in 0..window_cells {
                        let cell_x = window_x * window_cells + local_x;
                        let cell_z = window_z * window_cells + local_z;
                        let candidate = ecology_candidate(seed, cell_x, cell_z)
                            .expect("bounded ecology candidate");
                        if !ecology_candidate_wins_spacing(seed, candidate) {
                            continue;
                        }
                        if let Some(feature) = ecology_tree(seed, 0, candidate, &climate) {
                            count += 1;
                            *species
                                .entry(feature.tree_species().expect("ecology tree species"))
                                .or_default() += 1;
                        }
                    }
                }
                if count >= 30 {
                    dense_species.extend(species.keys().copied());
                }
                if count >= 100
                    && species.len() >= 2
                    && species.values().copied().max().unwrap_or_default() * 100 >= count * 55
                {
                    dominant_mixed_stands += 1;
                }
                counts.push(count);
            }
        }
        counts.sort_unstable();
        let sparse = counts[counts.len() / 10];
        let dense = counts[counts.len() * 9 / 10];
        assert!(sparse <= 6, "landscape lacked open areas: p10={sparse}");
        assert!(
            dense >= 30,
            "landscape lacked forest interiors: p90={dense}"
        );
        assert!(
            dense >= sparse.saturating_mul(5),
            "stand contrast collapsed: p10={sparse} p90={dense}"
        );
        assert!(
            dense_species.len() >= 4,
            "dense stands collapsed to {dense_species:?}"
        );
        assert!(
            dominant_mixed_stands >= 4,
            "species lacked locally dominant but mixed stands"
        );
    }

    #[test]
    fn ecology_random_channels_do_not_select_tree_height() {
        let seed = 0xdec0_ded;
        let mut height_totals = [0_u64; 4];
        let mut bucket_counts = [0_u64; 4];
        for cell_z in -64..64 {
            for cell_x in -64..64 {
                let candidate = ecology_candidate(seed, cell_x, cell_z).expect("bounded candidate");
                let bucket = (candidate.occurrence * 4.0).floor().min(3.0) as usize;
                height_totals[bucket] += u64::from(candidate.height_noise);
                bucket_counts[bucket] += 1;
            }
        }
        let means = std::array::from_fn::<_, 4, _>(|index| {
            height_totals[index] as f64 / bucket_counts[index] as f64
        });
        let minimum = means.into_iter().fold(f64::INFINITY, f64::min);
        let maximum = means.into_iter().fold(f64::NEG_INFINITY, f64::max);
        assert!(
            maximum - minimum < 6.0,
            "occurrence selected height: {means:?}"
        );
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
