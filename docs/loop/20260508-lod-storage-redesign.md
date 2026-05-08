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

### Follow-up Checkpoint - Fog-Footprint LOD Planning

Reduced unnecessary far-field LOD work without changing ownership semantics:

- `planLodNeededKeys` now only schedules LOD chunk footprints that intersect the circular fog-cull radius, instead of scheduling every square-corner chunk in every ring.
- The intersection test uses chunk AABB vs. radius, not center-only distance, so chunks touching the visible fog footprint are retained.
- LOD2+ generated fallback now uses the generator's top-bucket material API, matching the highest non-empty bucket while avoiding full bucket-array scans.
- Fog-range LOD tests now sample the circular fog footprint rather than unreachable square corners.

Validation:

- `mise exec -- bun run typecheck`
- `mise exec -- bun test tests/procedural-generator.test.ts tests/lod-system.test.ts tests/lod-handoff.test.ts tests/procedural-resident-world.test.ts`
- `mise exec -- bun run bench:lod-persistence -- --label=lod-default-footprint-topbucket`
- `mise exec -- bun run bench:lod-persistence -- --label=lod-far-footprint-topbucket --lod-persistence-chunk-delta=32 --lod-persistence-max-frames=260`

Browser verifier artifacts:

- Default persistence: `/var/folders/h7/xz1x4d4x0cn702r2q9205bkh0000gn/T/voxels-browser-game-bench-MLkGzT/lod-idb-persistence-reload.json`
  - Result: pass, with `32` reload disk hits and `32` cumulative reload disk hits.
- Far transition stress: `/var/folders/h7/xz1x4d4x0cn702r2q9205bkh0000gn/T/voxels-browser-game-bench-XdH85A/lod-idb-persistence-reload.json`
  - Result: still fails settle budget; visible far coverage stayed clean with `0` gaps and `0` overlaps.
  - Pending after `260` frames improved modestly from `923` to `895`; generation-budget pending moved from `843` to `813`.
  - This is a safe but small reduction. The remaining backlog is still structural enough that the next meaningful step should be partial handoff punching or a far-field hierarchy/clipmap, not more small pruning.

### Follow-up Checkpoint - LOD Level Attribution

Backed out a speculative partial-handoff experiment before committing it:

- The experiment reduced far-transition `pendingPrepared` to `0`, but it broke the persistence contract: default reload adopted `0` derived LOD chunks from IndexedDB.
- Root issue: the experiment mixed canonical persisted LOD chunks with context-punched render copies. That needs a proper canonical/render-instance split before it can be safe.
- Baseline was revalidated after the backout: default persistence passed again with `32` reload disk hits.

Added by-level LOD counters to the runtime snapshot and persistence artifact:

- Generated chunks by LOD level.
- Memory/disk cache hits by LOD level.
- Pending disk-cache, generation-budget, partial-build, and prepared-handoff chunks by LOD level.
- Active LOD chunk counts by level.

Validation:

- `mise exec -- bun run typecheck`
- `mise exec -- bun test tests/lod-handoff.test.ts tests/procedural-resident-world.test.ts`
- `mise exec -- bun run bench:lod-persistence -- --label=lod-default-level-attribution`
- `mise exec -- bun run bench:lod-persistence -- --label=lod-far-level-attribution --lod-persistence-chunk-delta=32 --lod-persistence-max-frames=260`

Browser verifier artifacts:

- Default persistence: `/var/folders/h7/xz1x4d4x0cn702r2q9205bkh0000gn/T/voxels-browser-game-bench-77ow0s/lod-idb-persistence-reload.json`
  - Result: pass, with `32` reload disk hits and `32` cumulative reload disk hits.
- Far level attribution: `/var/folders/h7/xz1x4d4x0cn702r2q9205bkh0000gn/T/voxels-browser-game-bench-6ZWv5k/lod-idb-persistence-reload.json`
  - Result: still fails settle budget; visible far coverage stayed clean with `0` gaps and `0` overlaps.
  - Pending after `260` frames: `898` total, `817` generation-budget, `69` prepared handoff, `12` disk-cache.
  - Generation-budget pending by level: `L1=142`, `L2=314`, `L3=198`, `L4=163`.
  - Prepared-handoff pending by level: `L1=69`, all other levels `0`.
  - Active chunk counts by level: `L1=611`, `L2=243`, `L3=151`, `L4=427`.

Next target:

1. Stop treating the far window as hundreds of unrelated one-off chunk builds. The level breakdown points toward a hierarchical far-field/clipmap plan that reuses coarser canonical data and generates visible coverage in larger batches.
2. Redesign partial handoff only after adding separate canonical persisted chunks and render-local punched instances, so disk reuse cannot regress.
3. Keep the current correctness gate strict: far stress may fail settle budget, but it must keep `0` gaps and `0` overlaps while performance work continues.

### Follow-up Checkpoint - Exclusive LOD Ownership Bands

Reduced redundant LOD planning by changing higher LOD rings from nested circles into ownership bands:

- A coarser LOD chunk is no longer scheduled if its full XZ footprint is already inside the next finer ring's coverage radius.
- Boundary chunks are retained when they cross the finer/coarser radius, so the handoff edge remains conservative.
- LOD2+ Y-range planning now prefers canonical generated column summaries before inheriting lower-LOD solid bounds. This avoids carrying lower-level padding into higher-level far chunks when summary data is available.

Validation:

- `mise exec -- bun run typecheck`
- `mise exec -- bun test tests/procedural-resident-world.test.ts tests/lod-handoff.test.ts`
- `mise exec -- bun test tests/lod-system.test.ts`
- `mise exec -- bun run bench:lod-persistence -- --label=lod-default-exclusive-rings-summary-y`
- `mise exec -- bun run bench:lod-persistence -- --label=lod-far-exclusive-rings-summary-y --lod-persistence-chunk-delta=32 --lod-persistence-max-frames=260`

Browser verifier artifacts:

- Default persistence: `/var/folders/h7/xz1x4d4x0cn702r2q9205bkh0000gn/T/voxels-browser-game-bench-jm9hfU/lod-idb-persistence-reload.json`
  - Result: pass, with `32` reload disk hits and `32` cumulative reload disk hits.
  - Cold generated LOD chunks dropped from `49` to `41`.
  - Reload generated LOD chunks dropped from `16` to `8`.
  - Reload downsample time dropped from `227.6ms` to `22.6ms`.
  - Worst recent reload frame dropped from `25.3ms` to `17ms`.
- Far transition stress: `/var/folders/h7/xz1x4d4x0cn702r2q9205bkh0000gn/T/voxels-browser-game-bench-MlOlTw/lod-idb-persistence-reload.json`
  - Result: still fails settle budget; visible far coverage stayed clean with `0` gaps and `0` overlaps.
  - Pending after `260` frames improved from `898` to `817`; generation-budget pending moved from `817` to `743`.
  - This confirms the change reduces real work but does not solve the structural far-transition backlog by itself.

Subagent review synthesis:

- Keep canonical generated chunks and generated summaries as the source of truth.
- Keep derived LOD chunks disposable and keyed by generation settings plus edit revision.
- Do not persist coverage-punched/render-context LOD chunks.
- The next big step should move LOD derivation toward a canonical source boundary and eventually a worker-side derivation path, rather than adding more main-thread special cases.

Metric follow-up:

- Added phase-total by-level counters for generated LOD chunks, memory cache hits, empty cache hits, and disk cache hits.
- Fixed the previous by-level cache-hit array so disk hits are no longer mixed into memory cache-hit counts.
- Default persistence with total metrics: `/var/folders/h7/xz1x4d4x0cn702r2q9205bkh0000gn/T/voxels-browser-game-bench-3ckSKg/lod-idb-persistence-reload.json`
  - Cold generated by level: `L1=32`, `L2=9`.
  - Reload generated by level: `L2=8`.
  - Reload disk hits by level: `L1=32`.
- Far transition with total metrics: `/var/folders/h7/xz1x4d4x0cn702r2q9205bkh0000gn/T/voxels-browser-game-bench-JF0Tvf/lod-idb-persistence-reload.json`
  - Generated by level over `260` frames: `L1=262`, all other levels `0`.
  - Final pending remained generation-budget dominated: `L1=141`, `L2=288`, `L3=173`, `L4=145`.
  - This proves the next improvement is scheduling/worker derivation, not more disk cache probing.

Rejected scheduler experiment:

- A naive coarse-first budgeted pass generated `0` chunks because it hit partial L3/L4 work first under the frame-time budget.
- An L2-first budgeted pass generated `278` L2 chunks and kept coverage clean, but far downsample time rose to `7389.3ms` for only a small pending reduction (`817` to `807`).
- Both scheduler experiments were backed out. The useful conclusion is that reordering main-thread derivation cannot solve this cleanly; LOD2+ derivation needs a worker/canonical-source path or much cheaper derived inputs.
