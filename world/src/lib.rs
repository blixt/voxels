//! Deterministic, host-testable voxel world representation, generation, meshing, and storage codecs.

pub mod atlas;
pub mod binary_mesh;
pub mod cave;
pub mod celestial;
pub mod chunk;
pub mod codec;
pub mod composition;
pub mod edit;
pub mod feature;
pub mod generation;
pub mod lod;
pub mod macro_composer;
pub mod material;
pub mod mesh;
pub mod protocol;
pub mod route;
pub mod source;
pub mod terrain_directory;
#[cfg(feature = "terrain-page-builder")]
mod terrain_error;
pub mod terrain_page;
#[cfg(feature = "terrain-page-builder")]
pub mod terrain_region;
pub mod terrain_stream;
pub mod terrain_transport;
pub mod virtual_surface;
#[cfg(any(test, feature = "virtual-surface-bakeoff"))]
#[doc(hidden)]
pub mod virtual_surface_bakeoff;
pub mod visibility;

pub use cave::{
    CINDER_VAULT_BOUNDS, CINDER_VAULT_CRYSTALS, CINDER_VAULT_EDGES, CINDER_VAULT_EXTERIOR_CELL,
    CINDER_VAULT_MOUTH_ANCHOR_XZ, CINDER_VAULT_MOUTH_CELL, CINDER_VAULT_NODES,
    CINDER_VAULT_PORTAL_COUNT, CINDER_VAULT_PORTAL_OPEN_LANES, CINDER_VAULT_PORTAL_PROBE_EDGE,
    CINDER_VAULT_STREAM_ACTIVATION_MARGIN_VOXELS, CINDER_VAULT_STREAM_INTEREST_CAPACITY,
    CINDER_VAULT_TOPOLOGY_VERSION, CINDER_VAULT_VISIBILITY_CELL_COUNT, CaveCrystalFormation,
    CaveEdge, CaveNode, CavePortalProbe, CaveSample, CaveStreamInterest, cinder_vault_crystal_at,
    cinder_vault_override, cinder_vault_portal_is_open, cinder_vault_portal_probe,
    cinder_vault_portal_probe_voxel, cinder_vault_portal_state,
    cinder_vault_portals_affected_by_voxel, cinder_vault_stream_interest,
    cinder_vault_visibility_cell, cinder_vault_visibility_graph, sample_cinder_vault,
};
pub use celestial::{CelestialModel, CelestialObservation, PlanetaryCoordinates};
pub use chunk::{CHUNK_EDGE, CHUNK_VOLUME, CHUNK_VOXEL_BYTES, Chunk, ChunkCoord};
pub use composition::{
    COMPOSITION_EDGE_FEATURE_CELLS, FeatureComposition, FeatureCompositionId,
    FeatureCompositionInfluence, FeatureCompositionMode, FeatureCompositionRole,
};
pub use edit::{EditMap, VoxelCoord, apply_resident_mutations};
pub use feature::{
    FEATURE_CELL_VOXELS, FEATURE_MAX_RADIUS_VOXELS, SkylineFeature, SkylineFeatureId,
    SkylineFeatureKind, TreeSpecies,
};
pub use generation::{
    AtmosphereSample, GENERATOR_VERSION, GeneratedColumn, GeneratedRegion, Generator,
    SEA_LEVEL_VOXELS, SurfaceRegion, SurfaceSample,
};
pub use lod::{
    SURFACE_HORIZON_CELL_COUNT, SURFACE_LOD_LEVEL_COUNT, SURFACE_PARENT_HORIZON_CELL_COUNT,
    SURFACE_PARENT_SHADING_EDGE_SAMPLES, SURFACE_PATCH_EDGE_CELLS, SURFACE_PATCHES_PER_TILE_EDGE,
    SURFACE_SHADING_EDGE_SAMPLES, SURFACE_TILE_EDGE_CELLS, SurfaceBounds, SurfaceLodLevel,
    SurfaceMorphClosure, SurfacePatch, SurfacePatchEdge, SurfacePatchId, SurfaceQuad,
    SurfaceShading, SurfaceTileCoord, SurfaceTileMesh, WaterPatch, WaterTileMesh,
    fallback_surface_wall_material, generate_edited_surface_tile_mesh,
    generate_edited_water_tile_mesh, generate_surface_tile_mesh, generate_surface_tile_mesh_with,
    generate_surface_tile_mesh_with_features, generate_surface_tile_mesh_with_features_and_shading,
    generate_water_tile_mesh_with, surface_tiles_affected_by_column,
    surface_tiles_affected_by_voxel,
};
pub use macro_composer::HeightfieldWorldSource;
pub use material::{Material, MaterialEmission, RenderLayer};
pub use mesh::{EmissiveCluster, MeshedChunk, Quad, mesh_chunk};
pub use route::{
    FIRST_PILGRIM_ROAD_BOUNDS, FIRST_PILGRIM_ROAD_NODES, ROUTE_CORE_HALF_WIDTH_VOXELS,
    ROUTE_SHOULDER_WIDTH_VOXELS, ROUTE_TOKEN_CADENCE_VOXELS, ROUTE_TOKEN_SIDE_OFFSET_VOXELS,
    RouteAnchor, RouteAnchorRole, RouteId, RouteLandmarkId, RouteNode, RouteSample,
    first_pilgrim_road_length_voxels, first_pilgrim_road_point_at_distance,
    first_pilgrim_route_anchor, first_pilgrim_route_anchor_count,
    first_pilgrim_route_anchor_for_feature_cell, sample_first_pilgrim_road,
};
pub use source::{
    ChunkSnapshot, MACRO_FIELD_SCHEMA_VERSION, MAX_MACRO_BLOCK_SAMPLES,
    MAX_SURFACE_SAMPLE_BLOCK_SAMPLES, MAX_SURFACE_SEARCH_RADIUS, MAX_VOXEL_BLOCK_SAMPLES,
    MAX_WORLD_PRODUCT_BATCH, MESHING_HALO_VOXELS, MacroBlock, MacroBlockBatch,
    MacroBlockBatchResult, MacroBlockRequest, MacroCoordinateTransform, MacroTerrainSource,
    MeshingHalo, ModelIdentity, NO_AUTHORED_CONTENT_VERSION, PROCEDURAL_SAMPLER_VERSION,
    PROCEDURAL_SCHEDULER_VERSION, ProceduralWorldSource, SourceDeviceRequirement,
    SurfaceSampleBlockRequest, SurfaceSampleBlockSnapshot, SurfaceSearchHit, SurfaceSearchKind,
    SurfaceSearchRequest, SurfaceSearchSnapshot, SurfaceTileSnapshot, VOXEL_COMPOSER_VERSION,
    VoxelBlockRequest, VoxelBlockSnapshot, WORLD_SCHEMA_VERSION, WorldId, WorldManifest,
    WorldManifestError, WorldManifestHash, WorldProduct, WorldProductBatch, WorldProductBatchItem,
    WorldProductBatchResult, WorldProductPriority, WorldProductRequest, WorldSourceEngine,
    WorldSourceError, WorldSourceIdentity, WorldSourceIdentityHash, WorldSourceKind,
    procedural_world_source,
};
pub use terrain_directory::{
    TERRAIN_COVERAGE_ROOT_LEVEL, TERRAIN_DIRECTORY_MAX_NODES, TERRAIN_DIRECTORY_MAX_ROOTS,
    TERRAIN_DIRECTORY_SCHEMA_VERSION, TERRAIN_REGION_ROOT_LEVEL, TerrainDirectoryError,
    TerrainHierarchyDirectoryV1, TerrainHierarchyNode, decode_region_terrain_directory,
    decode_terrain_directory, encode_terrain_directory,
};
pub use terrain_page::{
    SparseVoxelBrickPayload, SteppedSurfaceResidual, TERRAIN_PAGE_EDGE_SAMPLES,
    TERRAIN_PAGE_MAX_CHILDREN, TERRAIN_PAGE_MAX_COMPRESSED_BYTES, TERRAIN_PAGE_MAX_LEVEL,
    TERRAIN_PAGE_MAX_PAYLOAD_BYTES, TERRAIN_PAGE_SCHEMA_VERSION,
    TERRAIN_PAGE_TARGET_COMPRESSED_BYTES, TERRAIN_SURFACE_PAGE_CHILDREN,
    TERRAIN_SURFACE_PAGE_COORD_Y, TerrainClusterTriangle, TerrainClusterVertex, TerrainColumn,
    TerrainErrorBounds, TerrainHeightfieldGrid, TerrainMaterialCoverage, TerrainMaterialRun,
    TerrainPageBuildError, TerrainPageChild, TerrainPageCodecError, TerrainPageKey,
    TerrainPageReconstructionError, TerrainPageRepresentation, TerrainPageRepresentationKind,
    TerrainPageV1, TerrainReplacementError, TerrainSimplificationBudget, TerrainSparseBrick,
    TerrainSurfaceQuad, TerrainTopologyClass, TerrainTriangleCluster, assemble_terrain_parent,
    build_compact_exact_terrain_page, build_exact_cluster_terrain_parent, build_exact_terrain_page,
    build_sampled_heightfield_terrain_page, decode_terrain_page, encode_terrain_page,
    reconstruct_exact_terrain_surface, validate_terrain_replacement,
};
#[cfg(feature = "terrain-page-builder")]
pub use terrain_page::{build_budgeted_terrain_parent, build_simplified_triangle_terrain_parent};
#[cfg(feature = "terrain-page-builder")]
pub use terrain_region::{
    TerrainRegionBuildError, TerrainRegionBuildV1, build_terrain_coverage_root,
    build_terrain_region,
};
pub use terrain_stream::{
    TerrainDemandGroup, TerrainPageCacheError, TerrainPageDemand, TerrainPageMemoryCache,
    TerrainRequestBatch, TerrainStreamConfig, TerrainStreamError, TerrainStreamScheduler,
    TerrainStreamStats,
};
pub use terrain_transport::{
    TERRAIN_PAGE_TRANSFER_MAX_BYTES, TERRAIN_PAGE_TRANSFER_MAX_ITEMS,
    TERRAIN_PAGE_TRANSFER_SCHEMA_VERSION, TerrainPageBatchItemV1, TerrainPageBatchRequestV1,
    TerrainPageBatchResultV1, TerrainPageTransferCodecError, TerrainPageTransferFailure,
    TerrainPageTransferIdentity, decode_terrain_page_batch_request,
    decode_terrain_page_batch_result, encode_terrain_page_batch_request,
    encode_terrain_page_batch_result,
};
pub use virtual_surface::{
    BoundaryCertificate, BoundarySide, BoundarySideCertificate, CanonicalBoundarySample,
    CanonicalFaceKey, FaceAxis, VoxelBounds, canonical_exposed_faces,
};
#[cfg(any(test, feature = "virtual-surface-bakeoff"))]
#[doc(hidden)]
pub use virtual_surface_bakeoff::{
    BakeoffCamera, BakeoffCandidate, BakeoffCandidateKind, BakeoffClusterEditMetrics,
    BakeoffComparison, BakeoffError, BakeoffGpuQuad, BakeoffHit, BakeoffVolume,
    benchmark_clustered_page_rebuild, run_virtual_surface_bakeoff,
};
pub use visibility::{
    MAX_VISIBILITY_CELLS, MAX_VISIBILITY_PORTALS, PortalState, VisibilityCellId, VisibilityGraph,
    VisibilityGraphError, VisibilityPortal,
};

/// One canonical voxel is a 10 cm cube. World-space simulation and rendering use metres.
pub const VOXEL_SIZE_METRES: f32 = 0.1;
pub use atlas::{
    ATLAS_VERSION, CINDER_VAULT, CaveSystemDefinition, CaveSystemId, Destination, DestinationId,
    PILGRIM_CHAPTERS, PILGRIM_DESTINATIONS, RouteChapter, RouteChapterId,
    pilgrim_chapter_at_distance,
};
pub use binary_mesh::{BinaryMeshScratch, mesh_chunk_binary, mesh_chunk_binary_with_scratch};
