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
pub use generation::Generator;
pub use lod::{
    FAR_STRIDE_VOXELS, FAR_TILE_EDGE_CELLS, FAR_TILE_SPAN_VOXELS, FarTileCoord, SurfaceQuad,
    generate_far_tile, generate_far_tile_with,
};
pub use material::Material;
pub use mesh::{Quad, mesh_chunk};

/// One canonical voxel is a 10 cm cube. World-space simulation and rendering use metres.
pub const VOXEL_SIZE_METRES: f32 = 0.1;
