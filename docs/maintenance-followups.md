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

## Prove cross-directory terrain root coherence

The `world-lab` procedural-v16 fixture can fill the encoded cache while the exact terrain envelope
remains at zero coverage. A bounded publication-frontier prototype made the next blocker explicit:
the renderer rejected the level-3 children of surface root
`TerrainPageKey { level: 4, coord: [-1, i32::MIN, -1] }` as an incoherent replacement. At that
point the CPU cut reported one ownerless root and three skipped-level edges, so publishing the
candidate would violate the single-cut ownership contract.

Do not solve this by admitting the children independently or weakening replacement validation.
First add a world-service fixture that generates the parent and all four child-root pages through
their real directory requests, then compare their persisted schema-v7 boundary witnesses and
reconstructed shared edges. The eventual fix belongs in generation/persistence if those products
disagree; renderer admission should continue failing closed. Re-run `world-lab` through exact
coverage and GPU certification before revisiting bounded staging retention.

The default `player-rendering` dev-server scenario exposes the other side of the same staging
problem with Terrain Diffusion: startup fails while admitting
`TerrainPageKey { level: 2, coord: [-2, i32::MIN, -7] }` because the virtual-terrain GPU page pool
is exhausted. A bounded frontier must therefore prove both liveness and a hard memory bound for the
published cut, its causal balancing closure, and one maximum replacement group. The direct-child
prototype bounded memory but did not converge; retaining every historical descendant progressed
farther but exposed the incoherent root above and is not safe to ship.
