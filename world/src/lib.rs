//! Deterministic, host-testable voxel world representation, generation, meshing, and storage codecs.

pub mod chunk;
pub mod codec;
pub mod edit;
pub mod generation;
pub mod material;
pub mod mesh;

pub use chunk::{CHUNK_EDGE, CHUNK_VOLUME, Chunk, ChunkCoord};
pub use edit::{EditMap, VoxelCoord};
pub use generation::Generator;
pub use material::Material;
pub use mesh::{Quad, mesh_chunk};

/// One canonical voxel is a 10 cm cube. World-space simulation and rendering use metres.
pub const VOXEL_SIZE_METRES: f32 = 0.1;
