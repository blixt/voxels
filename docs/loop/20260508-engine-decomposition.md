# Engine Decomposition Diary - 2026-05-08

## Goal

Make renderer correctness observable before pushing more content. The current failure mode is that worldgen, LOD derivation, chunk residency, meshing, and cache behavior can diverge while still producing a plausible screenshot. The next architecture needs fewer sources of truth and purpose-built labs that make bad swaps, gaps, stale edits, and height divergence obvious.

## Source Of Truth

Chunks need one canonical baseline representation, with every rendered LOD derived from it:

- Canonical chunk key: `{worldVersion, seed, chunkSize, cx, cy, cz}` for base-resolution voxel chunks.
- Persistent baseline: generated chunks are stored once in an area-queryable store. Browser runtime can keep using IndexedDB, but the interface should be promoted from `ProceduralGeneratedChunkCache` to a world chunk store that also understands edits and range reads.
- Edit overlay: edits are stored as sparse per-voxel or run-length deltas keyed by base chunk. Loaded chunks apply overlays immediately; derived LOD chunks read the same composed base data.
- LOD cache: derived LOD meshes/chunks may be cached, but they are disposable. Base chunks plus edits are authoritative.
- Invalidation: an edit marks its base chunk dirty and invalidates every cached LOD chunk whose footprint intersects that base chunk.

Existing IndexedDB generated-chunk caching is useful plumbing, but it is not yet a complete world source of truth because it caches generated outputs and summaries rather than owning the canonical "generated baseline plus edits" contract.

## System Split

- World generation: owns island layout, biome fields, caves, roads, landmarks, placement rules, active NPC/mob zones, and base-resolution chunk material data.
- Chunk store: owns durable base chunks, overlays, range queries, cache versioning, and LOD invalidation metadata.
- LOD system: owns deterministic ownership, handoff, downsampling, edit propagation, and debug tooling. It must prove one visible representation per area, no disappearing chunks during swaps, and no source divergence.
- Render system: owns lighting, fog, sky, water, cave readability, shadow experiments, and frame-time reporting. It consumes resident chunks; it should not decide what world is true.
- Physics/pathing: owns collision sampling, traversability fields, movement probes, and route tests.
- NPC/mob system: owns spawned agents, schedules, combat/avoidance later, and active-zone budgets.

## LOD Lab Requirements

The first lab should not use production biome noise. It should use artificial terrain and debug colors:

- LOD0, LOD1, and edited voxels use distinct high-contrast materials.
- The lab reuses the renderer and GPU presentation path.
- Coverage assertions run over a known rectangle and report gaps/overlaps.
- Handoff assertions simulate a camera crossing LOD boundaries while pending chunks are unavailable.
- Edit assertions apply a far-away edit to the base source and verify the active LOD material changes after regeneration.

## ROI Queue

1. LOD lab + coverage/edit/handoff assertions. ROI: very high. It directly targets the recurring rendering failures and gives fast regression checks.
2. Promote chunk persistence to canonical chunk-store interfaces. ROI: very high. It reduces worldgen/LOD divergence by construction.
3. Browser-visible lab route. ROI: high. It gives shared visual debugging instead of hidden harness screenshots.
4. Production LOD migration onto the canonical store contract. ROI: high, but needs the first two items to avoid another patch stack.
5. Island biome redesign. ROI: high for game feel, but it should follow renderer correctness.
6. Lighting/cave render lab. ROI: medium-high once LOD is stable.
7. NPC/pathing lab. ROI: medium until the terrain contract is stable.

## Progress

- Server and Codex in-app browser are now the shared validation surface at `http://localhost:3000/`.
- Existing generated chunk cache has been reviewed and identified as a cache, not yet the canonical world store.
- Added `/lod-lab`, an artificial LOD route using the common renderer with high-contrast LOD0/LOD1/debug-edit materials.
- Added `bun run test:lod-lab`, covering artificial coverage ownership, staged handoff, far edit propagation, production stale-handoff invalidation, and frame timing buckets.
- Added 8 Hz frame timing buckets to the LOD lab and main game HUD. A first lab run caught a 692 ms max frame and 4 hitches; after staging data generation and meshing incrementally, the lab reached `max 46ms`, `hitches 0`, `gaps 0`, `overlaps 0` in the shared browser.
- Production now marks invalidated LOD chunks stale instead of deleting them immediately, so visible coverage is retained until replacement.
- Added production `preparedLodChunks`: finer chunks that would overlap active coarser coverage are staged offscreen, then committed only when the covered coarser footprint can be fully replaced. Focused production checks still pass: fog-range LOD coverage, single visible LOD ownership, and distant edit propagation.
- Made the `/lod-lab` buckets visible in the shared HUD as an 8 Hz strip, where each 125 ms bucket preserves its own max frame and hitch count. This makes periodic stalls easier to see honestly: a long shared-browser sample now shows `max 51ms`, `hitches 6`, `gaps 0`, `overlaps 0`; the strip caught hitch buckets but did not reproduce the earlier 400-600 ms pause in that run.
- Added attribution to the `/lod-lab` hitch line. It identified a live hitch as chunk `complete` work: `50ms` wall frame with `21.6ms` completion and `8.3ms` handoff. Reducing lab completions from two chunks/frame to one produced a later shared-browser sample with `max 38ms`, `hitches 0`, `gaps 0`, `overlaps 0`, and no stale hitch attribution in the recent window.

## Current Risk

The main game HUD now reports true wall-frame max and hitch buckets, and the current production scene still shows periodic hitches after startup. The next step is to correlate those hitches with streaming, LOD downsampling, mesh rebuilds, and GPU upload rather than treating the smoothed FPS number as authoritative. The LOD lab is currently correct on gaps/overlaps and its spinning-camera route is below the 50 ms hitch threshold in the latest shared sample, but it still reports a high dropped-frame estimate at roughly 30-40 fps. The next lab target is sustained frame pacing, not just avoiding severe hitches.
