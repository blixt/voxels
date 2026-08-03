# Maintenance follow-ups

This is the small running list of repository improvements that have concrete evidence but are not
safe to land as incidental cleanup. An item leaves this list only after its decision or measurement
gate is satisfied.

## Recreate a lost WebGPU surface

`Renderer::render` currently handles `CurrentSurfaceTexture::Lost` and `Outdated` identically by
configuring the existing `Surface`. In wgpu 30, `Outdated` calls for reconfiguration, while `Lost`
requires a new `Surface` from `Instance::create_surface` before configuration. The renderer consumes
its `SurfaceTarget` during construction and retains neither the target nor a host callback that can
create a replacement, so the correct recovery is larger than a local match-arm change.

Define surface ownership across `shell` and `render`, then add deterministic fault injection that
proves:

- `Outdated` reconfigures the existing surface and skips only the affected frame;
- `Lost` creates and configures a new surface without rebuilding unrelated GPU resources;
- device loss escalates to complete device/resource reconstruction instead of retrying the surface.

Reference: [wgpu 30 `CurrentSurfaceTexture`](https://docs.rs/wgpu/30.0.0/wgpu/enum.CurrentSurfaceTexture.html).

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

## Reserve memory before world response assembly

`max_queued_outbound_bytes_per_client` is acquired in `GenerationFrameDelivery::send`, after a
generation task has assembled and compressed its complete response. A slow client can therefore
retain up to its negotiated 16 in-flight responses outside the advertised 32 MiB outbound bound;
each response may approach the separate 16 MiB frame limit. Multiplying that gap across the public
128-client limit is not compatible with treating the outbound semaphore as a complete process-memory
bound.

The response size is not exact until compression finishes, so choose an explicit ownership model
before changing the scheduler: either reserve a conservative maximum before assembly and refund the
difference, or add a process-wide assembly budget that transfers the exact retained-byte permit to
the outbound frame. Then add an adversarial slow-reader test that fills every in-flight slot across
multiple clients and proves peak retained response bytes remain bounded, cancellation releases every
reservation, and collision-critical traffic cannot deadlock behind its own reservation.

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
their real directory requests, then compare their persisted schema-v11 boundary witnesses and
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
