# 2026-05-08 - LOD Storage Redesign Diary

## Source Audit

Research notes used:

- `/Users/blixt/Downloads/deep-research-report (2).md`
- `/Users/blixt/Downloads/geminivoxelresearch.md`

Useful takeaways:

- Treat LOD as a residency policy, not a mesh simplification afterthought.
- Keep gameplay/editable voxels chunk-sparse and authoritative in the near field.
- Treat derived LOD data as cacheable and disposable; it must never become a competing source of truth.
- Use compressed or procedural representations for static/far-field scale later, but first isolate generation, storage, LOD selection, and render submission.
- Cache-heavy systems are only trustworthy if invalidation is explicit and measurable.

## Current Code Reality

What exists today:

- Base generated chunks already persist through the browser worker IndexedDB cache in `src/client/procedural-generated-chunk-cache.ts`.
- `ProceduralResidentWorld` only keeps active LOD chunks in memory. Once a chunk leaves the active LOD window, it is deleted and rebuilt if revisited.
- Region summaries can be requested from the worker, but production LOD planning was not proactively requesting them for far-field footprints.
- Edits are held as sparse overlays and are applied to regenerated base chunks, but derived LOD cache invalidation had no retained-cache layer to clear.

Problem:

- The renderer was repeatedly paying to derive the same LOD chunks after travel, and the architecture encouraged generator fallback because persisted world summaries were not being pulled into planning.

## Implemented Step

Changed `ProceduralResidentWorld` so derived LOD chunks now have a bounded retained cache:

- Non-active LOD chunks are moved into `retainedLodChunks` when they leave the active window.
- Known-empty LOD keys are moved into `retainedEmptyLodKeys`.
- The retained chunk cap is 2048, sized to hold roughly one full derived LOD window rather than a token sample.
- Retained chunks are revived before any new downsample/generation work, and this bypasses `maxGenerateLodChunks`.
- Chunks whose data was punched out by finer active coverage are deliberately not retained, because those holes depend on the current LOD ownership state.
- Any edit clears the retained LOD cache; source-chunk and source-column invalidation delete intersecting retained entries.

Also changed LOD planning:

- Missing LOD-footprint column summaries now schedule persistent render-summary region requests through the existing async worker.
- The HUD snapshot and LOD summary now expose cache hits, empty cache hits, retained cache sizes, and scheduled region-summary requests.
- Added `bun run profile:lod-cache`, which moves the LOD window away and back, then records whether the return pass reused retained derived chunks before doing any rebuild work.

## Validation

Commands:

- `mise exec -- bun run typecheck`
- `mise exec -- bun test tests/procedural-resident-world.test.ts`
- `mise exec -- bun test tests/lod-system.test.ts -t "reuses retained"`
- `mise exec -- bun test tests/lod-system.test.ts`
- `mise exec -- bun run test:lod-lab`
- `mise exec -- bun run profile:lod-cache --label=derived-lod-cache-reuse`
- `mise exec -- bun run profile:lod-cache --label=derived-lod-cache-reuse-cap2048`

Important regression checks:

- LOD planning requests persisted render-summary regions for missing far-field metadata.
- Moving the LOD window far away and then back with `maxGenerateLodChunks: 0` revives retained derived chunks and performs no new generation on the return pass.
- Cache profile artifact `artifacts/lod-cache-reuse-profile/20260508T183155Z-derived-lod-cache-reuse-cap2048.json` reused `1540` derived LOD entries on return and rebuilt `0` chunks before reuse. The remaining `229` rebuilt chunks were not retained because they were coverage-punched or otherwise unsafe to cache.

## Next Architecture Move

This is the safe first step, not the final storage system.

### Follow-up Checkpoint - Derived LOD Payloads

Added the binary payload needed for disk-backed derived LOD cache:

- `src/engine/derived-lod-chunk-codec.ts` stores derived LOD chunks as a compact header plus run-length material spans.
- The payload records coord, LOD level, voxel stride, solid count, solid bounds, and voxel data.
- Empty derived chunks encode to less than 64 bytes in the focused test.
- `src/client/procedural-generated-chunk-cache.ts` now has a versioned `lod_chunks` IndexedDB store plus `getLodChunk`/`putLodChunk` methods keyed by edit revision, LOD level, and chunk coord.

Validation:

- `mise exec -- bun run typecheck`
- `mise exec -- bun test tests/derived-lod-chunk-codec.test.ts tests/generated-chunk-codec.test.ts tests/procedural-deferred-persistence.test.ts`

### Follow-up Checkpoint - Async Worker Surface

Added the nonblocking worker API needed before production LOD can safely read/write disk:

- `AsyncChunkGenerationQueue` now has derived LOD cache request, store, pending, drain, and completion-stat methods.
- The browser worker now handles `get-lod-chunk` and `put-lod-chunk` messages through the `lod_chunks` IndexedDB store.
- Completion stats distinguish cache hits, missing entries, and successful stores.
- Async LOD cache keys include edit revision, LOD level, and full chunk coord.

Validation:

- `mise exec -- bun run typecheck`
- `mise exec -- bun test tests/procedural-resident-world.test.ts tests/derived-lod-chunk-codec.test.ts`
- `mise exec -- bun test tests/async-chunk-generation.test.ts`

Next high-ROI work:

1. Promote the existing IndexedDB worker cache into an explicit engine-facing `ChunkStore` contract.
2. Persist base chunks, summaries, and edit overlays as the only authoritative world data.
3. Wire production LOD residency to request cached derived chunks before rebuilding and to enqueue safe derived chunks after eviction.
4. Add a prefetch planner that asks storage for base summaries and derived LOD chunks ahead of the active LOD window.
5. Move far-field rendering toward a hierarchy/clipmap model where more LOD levels do not linearly increase CPU rebuild cost.

## Rubric Delta

- Rendering/LOD correctness: `3.7 -> 3.85`
- LOD performance observability: `3.6 -> 3.9`
- Storage architecture: `2.4 -> 2.8`

The main remaining weakness is that retained derived LOD cache is still memory-only. Disk persistence is now a smaller, well-bounded follow-up instead of a cross-cutting rewrite.
