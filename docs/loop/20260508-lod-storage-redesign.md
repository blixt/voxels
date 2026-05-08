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

### Follow-up Checkpoint - Production Residency Wiring

Connected production LOD residency to the async derived-cache surface, still conservatively:

- `updateLodResidencyAround` drains completed derived LOD cache reads and can adopt them into active or prepared LOD chunks.
- Missing disk-cache entries are remembered by edit-revisioned key so cold-cache misses do not get re-requested forever.
- LOD cache reads are capped at 32 scheduled requests per update before the normal generation path takes over.
- Safe retained derived chunks enqueue disk-cache stores, flushed at 4 stores per update to avoid encoding hitches.
- Coverage-punched chunks and edited worlds remain excluded from disk persistence.
- HUD snapshots now expose disk-cache hits, misses, scheduled reads, scheduled stores, and completed stores.

Validation:

- `mise exec -- bun run typecheck`
- `mise exec -- bun test tests/procedural-resident-world.test.ts tests/async-chunk-generation.test.ts tests/derived-lod-chunk-codec.test.ts`
- `mise exec -- bun test tests/lod-system.test.ts -t "reuses retained"`
- `mise exec -- bun run profile:lod-cache --label=async-lod-disk-cache-wiring`

Profile artifact:

- `artifacts/lod-cache-reuse-profile/20260508T190754Z-async-lod-disk-cache-wiring.json`
- In the no-worker profile path, memory reuse stayed intact: return reused `1540` retained derived entries and rebuilt `0` chunks before reuse.

Next high-ROI work:

1. Promote the existing IndexedDB worker cache into an explicit engine-facing `ChunkStore` contract.
2. Persist base chunks, summaries, and edit overlays as the only authoritative world data.
3. Add browser/harness coverage that exercises real IndexedDB derived LOD reads/writes across reloads.
4. Add a prefetch planner that asks storage for base summaries and derived LOD chunks ahead of the active LOD window.
5. Move far-field rendering toward a hierarchy/clipmap model where more LOD levels do not linearly increase CPU rebuild cost.

## Rubric Delta

- Rendering/LOD correctness: `3.7 -> 3.85`
- LOD performance observability: `3.6 -> 3.9`
- Storage architecture: `2.4 -> 2.8`

The main remaining weakness is that retained derived LOD cache is still memory-only. Disk persistence is now a smaller, well-bounded follow-up instead of a cross-cutting rewrite.

### Follow-up Checkpoint - Browser-Proven Derived LOD Persistence

Added a real browser verifier for derived LOD persistence across reloads:

- `bench:lod-persistence` launches the production browser harness with startup/walk scenarios disabled.
- The scenario clears browser storage, builds and stores safe active derived LOD chunks, reloads the page without clearing IndexedDB, and verifies that the reloaded page adopts derived LOD chunks from the `lod_chunks` store.
- The verifier now samples cumulative LOD disk counters from `GameHudSnapshot`, so cache activity that happens during page startup is not lost before the sampled phase begins.
- `GameController.pumpWorldForBenchmark` gives harnesses a bounded way to advance streaming, meshing, rendering, and LOD work without using the unbounded force-settle path.

Engine changes made while building the verifier:

- Safe active derived LOD chunks now enqueue async disk stores immediately after they are built, instead of waiting for later eviction. This avoids a deadlock where pending replacement LOD work prevents eviction, and therefore prevents persistence from ever starting.
- Derived LOD disk-cache probes get reserved async queue headroom, so a saturated chunk-generation queue does not force immediate rebuilds before IndexedDB can be checked.

Validation:

- `mise exec -- bun run typecheck`
- `mise exec -- bun test tests/procedural-resident-world.test.ts tests/game-route-benchmark.test.ts tests/async-chunk-generation.test.ts`
- `mise exec -- bun test tests/procedural-resident-world.test.ts tests/async-chunk-generation.test.ts tests/derived-lod-chunk-codec.test.ts`
- `mise exec -- bun run bench:lod-persistence -- --label=lod-idb-default`

Browser verifier artifact:

- `/var/folders/h7/xz1x4d4x0cn702r2q9205bkh0000gn/T/voxels-browser-game-bench-3d6ZPC/lod-idb-persistence-reload.json`
- Result: pass, with `32` reload disk hits and `32` cumulative reload disk hits.

Important follow-up found by the broader stress variant:

- `--lod-persistence-chunk-delta=32` is now a useful transition stress mode, but it still exposes LOD overlap samples while a large far-window replacement is pending. That is a rendering correctness issue to attack separately from persistence.

### Follow-up Checkpoint - Reused LOD Handoff Correctness

Fixed the overlap path exposed by `--lod-persistence-chunk-delta=32`:

- Retained and IndexedDB-derived LOD chunks are now clipped against currently active finer coverage before they can become renderable.
- Render-ready resident columns also clip any stale active coarser LOD chunk they invalidate, so old coverage can stay visible for handoff without drawing on top of LOD0.
- Coverage-punched chunks remain context-local: they are marked in `coveragePunchedLodKeys` and are not retained or stored as canonical derived LOD data.
- Added handoff unit coverage for both inverse cases: retained coarser-over-active-finer and resident-ready-over-stale-LOD.

Validation:

- `mise exec -- bun run typecheck`
- `mise exec -- bun test tests/procedural-resident-world.test.ts tests/game-route-benchmark.test.ts tests/async-chunk-generation.test.ts tests/lod-debug-world.test.ts tests/lod-handoff.test.ts tests/frame-timing-buckets.test.ts`
- `mise exec -- bun run bench:lod-persistence -- --label=lod-default-after-handoff`
- `mise exec -- bun run bench:lod-persistence -- --label=lod-far-overlap-after-resident-punch --lod-persistence-chunk-delta=32 --lod-persistence-max-frames=260`

Browser verifier artifacts:

- Default persistence: `/var/folders/h7/xz1x4d4x0cn702r2q9205bkh0000gn/T/voxels-browser-game-bench-eWmq8w/lod-idb-persistence-reload.json`
  - Result: pass, with `32` reload disk hits and `32` cumulative reload disk hits.
- Far transition stress: `/var/folders/h7/xz1x4d4x0cn702r2q9205bkh0000gn/T/voxels-browser-game-bench-OEP0sL/lod-idb-persistence-reload.json`
  - Result: correctness improved to `0` uncovered gaps, `0` handoff holes, `0` resident overlaps, and `0` LOD-band overlaps.
  - The scenario still fails its current pass gate because the far phase does not settle within `260` frames: `923` LOD chunks remain pending. That is now a backlog/performance scheduling problem rather than a visible multi-owner rendering bug.

Next target:

1. Reduce the large-move LOD backlog by avoiding thousands of doomed missing IndexedDB probes in a cold far window.
2. Split stress validation so correctness failures (gaps/overlaps) and settle-budget failures are reported separately.
3. Add per-phase pending-source counters so `lodPendingChunks` can be attributed to disk probes, generation budget, prepared handoff chunks, or planning.

### Follow-up Checkpoint - LOD Pending Attribution

Added pending-source counters to the LOD summary, HUD snapshot, and browser persistence artifact:

- `pendingPlanning`
- `pendingDiskCache`
- `pendingGenerationBudget`
- `pendingPartialBuild`
- `pendingPrepared`
- `pendingInvalidatedEviction`

The large far-transition stress now reports why it fails the settle budget instead of only reporting one opaque pending total.

Validation:

- `mise exec -- bun run typecheck`
- `mise exec -- bun test tests/lod-handoff.test.ts tests/procedural-resident-world.test.ts`
- `mise exec -- bun run bench:lod-persistence -- --label=lod-default-pending-attribution`
- `mise exec -- bun run bench:lod-persistence -- --label=lod-far-pending-attribution --lod-persistence-chunk-delta=32 --lod-persistence-max-frames=260`

Browser verifier artifacts:

- Default persistence: `/var/folders/h7/xz1x4d4x0cn702r2q9205bkh0000gn/T/voxels-browser-game-bench-OsPjF1/lod-idb-persistence-reload.json`
  - Result: pass, with `32` reload disk hits and `32` cumulative reload disk hits.
- Far pending attribution: `/var/folders/h7/xz1x4d4x0cn702r2q9205bkh0000gn/T/voxels-browser-game-bench-GwgAtM/lod-idb-persistence-reload.json`
  - Result: still fails settle budget; visible far coverage stayed clean with `0` gaps and `0` overlaps.
  - Pending breakdown after `260` frames: `923` total pending, `843` generation-budget, `68` prepared handoff, `12` disk-cache, `0` planning, `0` partial-build, `0` invalidated-eviction.
  - A speculative early-covered-coarser skip reduced pending to `726`, but introduced a one-sample cold-origin coverage gap, so it was backed out. Next optimization needs a stricter coverage invariant before it can ship.
