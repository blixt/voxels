//! Deterministic, host-testable voxel world representation, generation, meshing, and storage codecs.

pub mod chunk;
pub mod codec;
pub mod edit;
pub mod generation;
pub mod lod;
pub mod material;
pub mod mesh;

pub use chunk::{CHUNK_EDGE, CHUNK_VOLUME, Chunk, ChunkCoord};
pub use edit::{EditMap, VoxelCoord};
pub use generation::{
    GENERATOR_VERSION, GeneratedColumn, Generator, SEA_LEVEL_VOXELS, SkylineFeature,
    SkylineFeatureId, SurfaceRegion, SurfaceSample,
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
pub use material::{Material, RenderLayer};
pub use mesh::{MeshedChunk, Quad, mesh_chunk};

/// One canonical voxel is a 10 cm cube. World-space simulation and rendering use metres.
pub const VOXEL_SIZE_METRES: f32 = 0.1;
