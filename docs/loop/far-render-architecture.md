# Far Render Architecture

## Strong opinion

The long-term engine should have exactly one world truth: persisted/generated/edited chunks. The generator is only a fallback producer for pristine chunks. Rendering, LOD, streaming, persistence, and multiplayer must all consume chunk state or metadata derived from chunk state.

That rules out two dead ends:

- no render-time procedural sampling for far terrain
- no single surface-only far representation as the permanent solution

## Authoritative data

- Keep authoritative chunk payloads as sparse-wrapped local dense data.
- Keep chunk size at `32^3` for now, with `8^3` subchunks in the durable codec.
- Persist generated base chunks and edited chunks in the same logical format.
- Treat empty chunks as real chunk outcomes, not as missing data.

## Derived data

Every chunk should produce a compact render summary from its voxel bytes:

- surface columns for outdoor clipmaps
- coarse `8x8x8`-cell occupancy states inside the chunk
- boundary face-open masks for conservative cave/void propagation

That is why the repo now has `GeneratedChunkRenderSummary` instead of a surface-only summary. A summary is not a second world model; it is chunk-derived metadata for cheaper queries.

## Far rendering

Use two far representations, both derived from chunk summaries:

1. Above ground: a surface clipmap
- Keep the current surface-style clipmap idea for horizon and terrain silhouette.
- Build it from chunk-derived surface columns only.
- Promote the current far path into the explicit “surface far renderer”, not the whole far-render architecture.

2. Underground / large voids: volumetric visibility
- Do not try to fake huge caverns with top-surface data.
- Traverse coarse occupancy cells and face-open masks from the current air cell or from known cave-mouth/frontier cells.
- Render only reachable coarse shell geometry or a cheap voxel DDA/ray style representation, not every hidden distant chunk.
- Use portal-style propagation across summary face openings so underground visibility follows actual air connectivity instead of generator heuristics.

## LOD

Use chunk-derived multires summaries, not naive voxel downsampling:

- near: full chunk meshes from authoritative resident chunks
- mid: chunk-summary-driven surface clipmaps and coarse volumetric shells
- far: coarser clipmap rings / occupancy bricks with stricter visibility and streaming budgets

The important detail is that LOD is derived from actual chunk state. If a player edits the world, both surface and volumetric far representations update from edited chunks, not from generator assumptions.

## Streaming

Split world streaming into two bubbles:

- resident bubble: full chunks needed for gameplay, collision, edits, and detailed rendering
- summary bubble: much larger chunk/region summaries needed for horizon, cavern visibility, and streaming decisions

Initial load should:

- choose a spawn
- generate/load the resident gameplay bubble
- show progress until the required near bubble is ready
- load summary data for the first wider far ring in parallel

After that:

- full chunks load only when the player or the visibility frontier needs them
- summary data loads much farther out than full chunks
- cold full chunks are evicted aggressively while summaries remain cached much longer

## Persistence and server direction

The client should eventually persist:

- chunk payloads
- chunk render summaries
- region/manifest metadata
- edit deltas

The server should become authoritative for the same objects:

- chunk snapshots
- chunk summaries
- edit/event streams

That gives one consistent contract for single-player, multiplayer, and revisits from disk.

## Migration order

1. Keep the current surface clipmap working, but only over chunk-derived summaries.
2. Persist chunk render summaries explicitly instead of relying on in-memory post-eviction archives.
3. Introduce region-level summary indexes so far rendering and Y-range/visibility work stop scanning individual chunks blindly.
4. Add volumetric far visibility for caves/voids from chunk summaries.
5. Move full far streaming decisions onto summary visibility/frontier logic rather than procedural estimates.
