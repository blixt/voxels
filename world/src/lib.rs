//! Deterministic, host-testable voxel world representation, generation, meshing, and storage codecs.

pub mod atlas;
pub mod cave;
pub mod chunk;
pub mod codec;
pub mod composition;
pub mod edit;
pub mod feature;
pub mod generation;
pub mod lod;
pub mod material;
pub mod mesh;
pub mod route;
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
pub use chunk::{CHUNK_EDGE, CHUNK_VOLUME, CHUNK_VOXEL_BYTES, Chunk, ChunkCoord};
pub use composition::{
    COMPOSITION_EDGE_FEATURE_CELLS, FeatureComposition, FeatureCompositionId,
    FeatureCompositionInfluence, FeatureCompositionMode, FeatureCompositionRole,
};
pub use edit::{EditMap, VoxelCoord};
pub use feature::{
    FEATURE_CELL_VOXELS, FEATURE_MAX_RADIUS_VOXELS, SkylineFeature, SkylineFeatureId,
    SkylineFeatureKind,
};
pub use generation::{
    AtmosphereSample, GENERATOR_VERSION, GeneratedColumn, GeneratedRegion, Generator,
    SEA_LEVEL_VOXELS, SurfaceRegion, SurfaceSample,
};
pub use lod::{
    FAR_STRIDE_VOXELS, FAR_TILE_EDGE_CELLS, FAR_TILE_SPAN_VOXELS, FarTileCoord,
    SURFACE_PATCH_EDGE_CELLS, SURFACE_PATCHES_PER_TILE_EDGE, SURFACE_TILE_EDGE_CELLS,
    SurfaceBounds, SurfaceLodLevel, SurfacePatch, SurfacePatchEdge, SurfaceQuad, SurfaceTileCoord,
    SurfaceTileMesh, WaterPatch, WaterTileMesh, generate_edited_surface_tile,
    generate_edited_surface_tile_mesh, generate_edited_water_tile_mesh, generate_far_tile,
    generate_far_tile_with, generate_surface_tile, generate_surface_tile_mesh,
    generate_surface_tile_mesh_with, generate_surface_tile_with, generate_water_tile_mesh_with,
    surface_tiles_affected_by_column, surface_tiles_affected_by_voxel,
};
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
