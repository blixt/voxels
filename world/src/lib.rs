//! Deterministic, host-testable voxel world representation, generation, meshing, and storage codecs.

pub mod chunk;
pub mod codec;
pub mod generation;
pub mod material;
pub mod mesh;

pub use chunk::{CHUNK_EDGE, CHUNK_VOLUME, Chunk, ChunkCoord};
pub use generation::Generator;
pub use material::Material;
pub use mesh::{Quad, mesh_chunk};
