# Maintenance follow-ups

This is the small running list of repository improvements that have concrete evidence but are not
safe to land as incidental cleanup. An item leaves this list only after its decision or measurement
gate is satisfied.

## Reuse procedural chunk generation for meshing halos

`ProceduralWorldSource::chunk_with_halo` currently calls `Generator::generate_chunk`, then builds a
34-by-34 `GeneratedRegion` for the halo. The region recomputes the 32-by-32 core column profiles and
feature descriptors that chunk generation just evaluated.

The combined path should construct the authoritative region once and derive both the core `Chunk`
and `MeshingHalo` from it. Before replacing the current path, require:

- material-for-material equivalence across representative, negative-boundary, feature, cave, and
  water chunks;
- identical encoded `ChunkSnapshot` products;
- a Criterion comparison of the production `ChunkWithHalo` request, including allocation counts or
  peak resident memory when available.

Relevant code: `world/src/source.rs` (`chunk_with_halo`) and `world/src/generation.rs`
(`generate_chunk`, `GeneratedRegion`).

## Decide exact-corner voxel ray semantics

The portable DDA traversal in `core/src/lib.rs` advances one axis when two or three boundary times
are exactly equal. That is deterministic, but it chooses a voxel reached only through a mathematical
edge or corner before the other tied neighbors.

Changing it requires a gameplay decision: picking and visibility can use the current thin-ray rule,
or a conservative supercover rule can visit every tied neighbor. Define the expected behavior for
placing, digging, and occlusion at exact grid corners, then add symmetric positive/negative tie
fixtures before changing traversal.
