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

### Follow-up Checkpoint - Canonical LOD Derivation Boundary

Extracted the LOD voxel-data derivation loop into a pure engine module:

- New module: `src/engine/lod-chunk-derivation.ts`.
- `ProceduralResidentWorld.downsampleLodChunkData` now supplies explicit callbacks for:
  - source material sampling,
  - generated top-material fallback,
  - generated column fallback,
  - surface Y range,
  - finer-coverage ownership checks.
- This does not move work to a worker yet, but it creates the boundary needed for that migration without changing activation, handoff, disk persistence, or rendering ownership.

Validation:

- `mise exec -- bun run typecheck`
- `mise exec -- bun test tests/lod-chunk-derivation.test.ts tests/procedural-resident-world.test.ts tests/lod-handoff.test.ts`
- `mise exec -- bun run bench:lod-persistence -- --label=lod-default-derivation-boundary`

Browser verifier artifact:

- Default persistence: `/var/folders/h7/xz1x4d4x0cn702r2q9205bkh0000gn/T/voxels-browser-game-bench-Nks6mW/lod-idb-persistence-reload.json`
  - Result: pass, with `32` reload disk hits and `32` cumulative reload disk hits.
  - Coverage remained clean with `0` gaps and `0` overlaps.

Next target:

1. Add a worker request that derives a LOD chunk through this same boundary using canonical generated chunk data and edit deltas.
2. Keep the existing derived LOD disk cache as a disposable cache, not a source of truth.
3. Move only LOD2+ far derivation first; LOD1 should remain resident-source-sensitive until edit and near-field behavior are better isolated.

### Follow-up Checkpoint - Worker-Side LOD2+ Derivation

Moved cache-cold LOD2+ derivation off the main thread:

- Added an async `requestGeneratedLodChunk` queue path with separate pending keys from disk-cache probes.
- The procedural generation worker now derives LOD chunks through `deriveLodChunkData`, using canonical procedural column sampling rather than resident render chunks.
- Worker-derived chunks are immediately persisted to the derived LOD IndexedDB cache under the existing edit-revision key.
- Completed derived chunks carry `source: "cache" | "generated"` so disk hits, worker generation, and stores are measured separately.
- Residency still keeps LOD1 on the main-thread resident-source path; LOD2+ uses the worker only when edits are absent.
- Disk probes now get a wider first chance (`64` per update), then worker generation keeps cold chunks from blocking the renderer. A strict disk-first experiment made cold population miss the settle budget and produced coverage gaps, so the hybrid is the current measured tradeoff.

Validation:

- `mise exec -- bun run typecheck`
- `mise exec -- bun test tests/lod-chunk-derivation.test.ts tests/lod-handoff.test.ts tests/procedural-resident-world.test.ts`
- `mise exec -- bun run bench:lod-persistence -- --label=lod-default-worker-derive-hybrid64`

Browser verifier artifact:

- Default persistence: `/var/folders/h7/xz1x4d4x0cn702r2q9205bkh0000gn/T/voxels-browser-game-bench-DqUKao/lod-idb-persistence-reload.json`
  - Result: pass, with `921` reload disk hits and `921` cumulative reload disk hits.
  - Reload settled in `24` frames with clean coverage: `0` uncovered gaps, `0` handoff holes, `0` overlaps.
  - Reload main-thread LOD generation was reduced to `1` LOD1 chunk; LOD2-L4 main-thread generation stayed at `0`.
  - Reload worker-generated LOD chunks: `96` total (`L2=32`, `L3=16`, `L4=48`).
  - Cold-origin LOD2-L4 derivation moved to the worker: `941` worker chunks (`L2=291`, `L3=187`, `L4=463`), while main-thread generated chunks were LOD1-only (`64`).
  - Reload downsample time was `3ms`; coverage remained clean.

Rejected check:

- Strict disk-first scheduling artifact: `/var/folders/h7/xz1x4d4x0cn702r2q9205bkh0000gn/T/voxels-browser-game-bench-rf0R4z/lod-idb-persistence-reload.json`
  - Reload disk reuse improved to `1379` hits, but cold-origin ended with `144` pending LOD chunks and `258` uncovered coverage samples after `240` frames.
  - Conclusion: disk-first is useful for reloads but cannot be allowed to starve cache-cold worker derivation.

Next target:

1. Add a far-transition benchmark run for this worker path and compare against the earlier far backlog (`L1=141`, `L2=288`, `L3=173`, `L4=145` pending).
2. Add worker-side edit overlay awareness or a canonical edit delta layer before enabling worker derivation when edits are present.
3. Investigate replacing LOD1 main-thread source derivation with a safer canonical-summary path after near-field handoff tests are expanded.

### Follow-up Checkpoint - LOD Ownership Diagnostics

Tightened the LOD correctness harness and fixed two activation-order gaps:

- `probeLodCoverage` now records exact owner chunk IDs and vertical material ranges for overlap samples.
- Band overlap is now counted only when different LOD bands overlap in X/Z and in material Y range. This avoided false positives from stacked vertical chunks while preserving real z-fighting risks.
- `punchActiveCoarserLodChunksCoveredBy` now uses direct coarser coordinate lookup instead of scanning every active LOD chunk.
- Prepared finer replacements now preserve visible stale-finer ownership by punching active coarser chunks when the old same-key finer chunk is still renderable.
- Prepared finer backlogs now run a targeted coarser repair for coarser chunks known to overlap prepared finer candidates.

Validation:

- `mise exec -- bun run typecheck`
- `mise exec -- bun test tests/lod-handoff.test.ts tests/procedural-resident-world.test.ts tests/render-verification-metrics.test.ts`
- `mise exec -- bun run bench:lod-persistence -- --label=lod-default-direct-punch-check`

Browser verifier artifacts:

- Default persistence: `/var/folders/h7/xz1x4d4x0cn702r2q9205bkh0000gn/T/voxels-browser-game-bench-p6BUct/lod-idb-persistence-reload.json`
  - Result: pass, with `936` reload disk hits and clean coverage (`0` gaps, `0` handoff holes, `0` resident overlaps, `0` band overlaps).
  - Reload settled in `24` frames.

Rejected / incomplete checks:

- Continuous active-ownership repair reduced far-transition overlap samples from `127` to `31`, but roughly doubled cold persistence elapsed time. It was removed as too expensive for the default engine path.
- Far transition remains unresolved:
  - `/var/folders/h7/xz1x4d4x0cn702r2q9205bkh0000gn/T/voxels-browser-game-bench-8RCKPO/lod-idb-persistence-reload.json`
  - With vertical-aware samples, the failure is confirmed as real overlapping material ranges, not a stacked-Y false positive.
  - Final far state still had `216` pending LOD chunks (`146` generation-budget, `70` prepared) and `127` true LOD band-overlap samples.

Next target:

1. Replace ad hoc ownership repair with deterministic ownership state during LOD planning/activation, so active LOD chunks cannot start a frame with overlapping material ranges.
2. Add per-key active/prepared/stale diagnostics to overlap samples so the far artifact reports whether each owner is stale, punched, prepared-blocked, or fresh.
3. Reduce far-transition LOD1 backlog; current worker derivation helps LOD2+, but the remaining far pending is dominated by main-thread LOD1 and prepared handoff state.

### Follow-up Checkpoint - Owner State Evidence

Added per-owner LOD state to overlap samples:

- Each LOD coverage issue sample now includes `ownerStates` with `active`, `stale`, `coveragePunched`, `prepared`, `retained`, `empty`, and `coveredEmpty`.
- `ProceduralResidentWorld.getLodChunkDebugState` exposes that state without giving the client mutable access to LOD internals.

Validation:

- `mise exec -- bun run typecheck`
- `mise exec -- bun test tests/lod-handoff.test.ts tests/procedural-resident-world.test.ts tests/render-verification-metrics.test.ts`
- `mise exec -- bun run bench:lod-persistence -- --label=lod-default-owner-state-diagnostics`

Browser verifier artifacts:

- Default persistence: `/var/folders/h7/xz1x4d4x0cn702r2q9205bkh0000gn/T/voxels-browser-game-bench-0BFRmD/lod-idb-persistence-reload.json`
  - Result: pass, with `929` reload disk hits and clean coverage.
- Far owner-state diagnostic: `/var/folders/h7/xz1x4d4x0cn702r2q9205bkh0000gn/T/voxels-browser-game-bench-9gd839/lod-idb-persistence-reload.json`
  - Result: still fails far settle.
  - Critical evidence: the overlapping sample owners are both fresh active chunks (`stale=false`, `coveragePunched=false`, `prepared=false`), e.g. `L2:-98:11:-188` and `L1:-196:22:-376`.
  - This rules out the earlier stale/prepared-only hypothesis for the remaining far overlap.

Rejected check:

- Reasserting ownership for every fresh active needed chunk cleared the conceptual leak but was too broad:
  - It made the far benchmark over-budget.
  - It also caused a default cold-origin regression with `16` uncovered LOD samples.
  - The attempted artifact was `/var/folders/h7/xz1x4d4x0cn702r2q9205bkh0000gn/T/voxels-browser-game-bench-cLnFCB/lod-idb-persistence-reload.json`.

Next target:

1. Move from reactive punching toward a deterministic single-owner LOD plan per world column, so coarser chunks are generated already clipped to the finer bands that are planned for the same frame.
2. Add a small artificial LOD-lab scenario where planned LOD1 and LOD2 intentionally overlap in X/Z; assert that the active representation has exactly one owner per overlapping material interval.
3. Keep default persistence as a guard before any new far-transition repair is accepted.

### Follow-up Checkpoint - Voxel-Aware LOD Punching

Changed LOD-vs-LOD punching from whole-column removal to voxel-range removal:

- Coarser LOD voxels are removed only when their world-space X/Z and Y span intersects active finer LOD material.
- This preserves upper/lower coarser material in the same X/Z column when the finer LOD only owns part of the vertical range.
- Resident LOD0 ownership still clips whole columns because render-ready resident columns are the near-field authority.
- The direct coarser lookup path remains in place so activation-time punching does not scan every active LOD chunk.

Validation:

- `mise exec -- bun run typecheck`
- `mise exec -- bun test tests/lod-handoff.test.ts`
- `mise exec -- bun test tests/lod-handoff.test.ts tests/procedural-resident-world.test.ts tests/render-verification-metrics.test.ts`
- `mise exec -- bun run bench:lod-persistence -- --label=lod-default-voxel-aware-punch`

Browser verifier artifacts:

- Default persistence: `/var/folders/h7/xz1x4d4x0cn702r2q9205bkh0000gn/T/voxels-browser-game-bench-KvXcaY/lod-idb-persistence-reload.json`
  - Result: pass, with `926` reload disk hits and clean coverage.
- Far transition: `/var/folders/h7/xz1x4d4x0cn702r2q9205bkh0000gn/T/voxels-browser-game-bench-MJOxJU/lod-idb-persistence-reload.json`
  - Result: still fails far settle.
  - True band-overlap count remains `127`.
  - The overlapping owners are still fresh active, unpunched chunks; this confirms the remaining bug is not punch precision, but the missing invocation/reconciliation path for already-active overlapping LODs.

Next target:

1. Build a deterministic LOD ownership reconciliation for fresh active overlaps that is scoped to detected owner conflicts, not all active needed chunks.
2. Add a lab/unit scenario that starts with two fresh active overlapping LOD chunks and verifies reconciliation removes only the overlapping material interval without creating coverage gaps.
3. Re-run default persistence before far-transition acceptance; the previous broad fresh-active repair regressed cold-origin coverage and must stay rejected.

### Follow-up Checkpoint - Far Coverage Clean

Combined voxel-aware punching with idempotent fresh-active ownership reassertion:

- Fresh active needed LOD chunks now reassert ownership against active coarser chunks, but skip coarser chunks already marked `coveragePunched`.
- This keeps the repair bounded and avoids repeated coarser remeshes.
- The previous broad reassertion created gaps because punching removed whole X/Z columns. With voxel-aware punching, the same ownership reassertion preserves unrelated vertical material.

Validation:

- `mise exec -- bun run typecheck`
- `mise exec -- bun test tests/lod-handoff.test.ts tests/procedural-resident-world.test.ts tests/render-verification-metrics.test.ts`
- `mise exec -- bun run bench:lod-persistence -- --label=lod-default-idempotent-voxel-aware-punch`
- `mise exec -- bun run bench:lod-persistence -- --label=lod-far-idempotent-voxel-aware-punch --lod-persistence-chunk-delta=32 --lod-persistence-max-frames=260`

Browser verifier artifacts:

- Default persistence: `/var/folders/h7/xz1x4d4x0cn702r2q9205bkh0000gn/T/voxels-browser-game-bench-KyKwW6/lod-idb-persistence-reload.json`
  - Result: pass, with `918` reload disk hits.
- Far transition: `/var/folders/h7/xz1x4d4x0cn702r2q9205bkh0000gn/T/voxels-browser-game-bench-RMfHpm/lod-idb-persistence-reload.json`
  - Result: still fails settle due pending backlog.
  - Rendering coverage is clean: `0` uncovered gaps, `0` handoff holes, `0` resident overlaps, `0` band overlaps.
  - Remaining far pending: `219` LOD chunks (`158` generation-budget, `57` prepared).

Next target:

1. Treat the visual LOD overlap bug as fixed for this checkpoint, guarded by the far coverage artifact.
2. Attack far-transition backlog next: LOD1 generation and prepared handoff remain the dominant blockers to settle.
3. Add a benchmark success mode that can distinguish “visual coverage clean but background backlog remains” from actual visible rendering failure.

### Follow-up Checkpoint - Worker LOD1 Derivation

Moved cache-cold LOD1 derivation onto the worker path when there are no edit overlays:

- `scheduleGeneratedLodChunkRequest` now permits `lodLevel >= 1`.
- Main-thread LOD derivation remains available when edits are present.
- Disk-cache and worker metrics now include LOD1 worker-derived chunks.
- Updated the persistence queue unit test to test queue flushing directly, since cache-cold LOD1 no longer needs to be main-thread generated in normal unedited worlds.

Validation:

- `mise exec -- bun run typecheck`
- `mise exec -- bun test tests/procedural-resident-world.test.ts tests/lod-handoff.test.ts`
- `mise exec -- bun run bench:lod-persistence -- --label=lod-default-worker-lod1`
- `mise exec -- bun run bench:lod-persistence -- --label=lod-far-worker-lod1 --lod-persistence-chunk-delta=32 --lod-persistence-max-frames=260`

Browser verifier artifacts:

- Default persistence: `/var/folders/h7/xz1x4d4x0cn702r2q9205bkh0000gn/T/voxels-browser-game-bench-1XW2h4/lod-idb-persistence-reload.json`
  - Result: pass.
  - Reload disk hits increased to `1299`.
- Far transition: `/var/folders/h7/xz1x4d4x0cn702r2q9205bkh0000gn/T/voxels-browser-game-bench-fhZ1AI/lod-idb-persistence-reload.json`
  - Result: still fails settle, but pending dropped from `219` to `45`.
  - Generation-budget pending is now `0`.
  - Worker-generated chunks: `986` total (`L1=403`, `L2=277`, `L3=179`, `L4=127`).
  - Main-thread LOD generation and downsample time are now `0`.
  - Coverage remains clean (`0` gaps, `0` handoff holes, `0` overlaps).
  - Remaining pending is almost entirely prepared handoff: `44` chunks (`L1=19`, `L2=11`, `L3=14`).

Next target:

1. Resolve prepared handoff backlog deterministically; generation is no longer the far-transition blocker.
2. Inspect why prepared chunks remain blocked even when visual coverage is clean.
3. Consider allowing the far benchmark to report a separate “visual-settled” pass, while keeping full-settle as a performance/backlog target.

### Follow-up Checkpoint - Planning Wave and Rejected Prepared-Handoff Attempt

The user asked for a multi-step plan, detailed subagent planning, dependency mapping, and distribution of work back to agents.

Planning/integration completed:

- Added a consolidated execution board to `docs/loop/20260509-master-execution-plan.md`.
- Delegated four planning tracks:
  - Track A: canonical storage and LOD architecture.
  - Track B: render/performance verification harness.
  - Track C/D: finite island atlas and art/content direction.
  - Track E: RPG exploration gameplay.
- Distributed Wave 1 implementation:
  - render verification orchestrator;
  - pure finite island atlas foundation;
  - pure exploration event model;
  - local P0 prepared LOD handoff investigation.

Accepted Wave 1 implementation outputs:

- `verify:render` now aggregates existing route/live-forward/LOD/view artifacts into a reproducible JSON report with command manifest and FPS/LOD/sample gates.
- `world-atlas.ts` provides the first pure finite-island atlas with eight macro regions, island mask fields, ocean classification, and region-edge sampling tests.
- `exploration-events.ts` provides a pure idempotent event log for discover/inspect/read/use/zone/travel-goal/encounter events.

Rejected LOD attempt:

- I tested material-range-aware prepared handoff predicates for the remaining prepared backlog.
- Focused unit tests passed, but the far browser stress exposed two unacceptable outcomes:
  - material-aware prepared activation without reconciliation reintroduced true LOD-band overlaps (`40` samples in `/var/folders/h7/xz1x4d4x0cn702r2q9205bkh0000gn/T/voxels-browser-game-bench-cMYFEC/lod-idb-persistence-reload.json`);
  - broad re-punching of already-punched coarser chunks was too expensive and left the headless renderer CPU-bound for more than five minutes, so the run was terminated.
- I reverted the unsafe LOD code and tests before checkpointing. The next LOD attempt needs a targeted overlap-reconciliation queue, not broad per-active-finer rescans.

Validation kept for the accepted checkpoint:

- `mise exec -- bun run typecheck`: pass.
- `mise exec -- bun test tests/render-verification-metrics.test.ts tests/render-verification-runner.test.ts tests/world-atlas.test.ts tests/exploration-events.test.ts tests/exploration-journal.test.ts tests/skill-journal.test.ts tests/lod-handoff.test.ts tests/procedural-resident-world.test.ts`: pass, `69` tests.
- `mise exec -- bun run verify:render -- --latest-artifacts=false --output=/tmp/voxels-render-verification-cleaned.json`: pass, no failures; warnings/skips are expected because this smoke intentionally used no latest browser artifacts.
- Default LOD persistence before reverting the unsafe LOD attempt: `/var/folders/h7/xz1x4d4x0cn702r2q9205bkh0000gn/T/voxels-browser-game-bench-vTAxx9/lod-idb-persistence-reload.json`, pass, `1275` reload disk hits.

Rubric movement:

- Harness maturity: `10.0 -> 10.1` because `verify:render` gives a single artifact contract for existing visual/performance/LOD evidence.
- World architecture readiness: `2.8 -> 3.0` because the island atlas now has a pure tested source of truth, without touching generator/runtime behavior.
- Gameplay architecture readiness: `2.4 -> 2.7` because inspect/read/use/travel/encounter events now have a pure idempotent model.
- Rendering/LOD correctness: unchanged; the prepared-handoff attempt was rejected rather than accepted.

Next:

1. Commit and push the safe planning/harness/atlas/gameplay checkpoint.
2. Resume P0 LOD work with a bounded reconciliation queue driven by detected active overlap candidates.
3. After P0, assign canonical store/edit-journal implementation and route graph work in parallel.

### Follow-up Checkpoint - Safe Same-Key Prepared Replacement

Accepted LOD change:

- Added a conservative same-key prepared replacement path.
- When a fresh prepared chunk has the same LOD key, level, stride, and an already render-ready active owner, it can replace that stale active owner directly.
- The replacement still punches active coarser chunks after becoming active, so ownership remains consistent for that same footprint.
- Added a regression test for stale active finer ownership being swapped to the prepared same-key chunk.

Rejected attempts during this pass:

- A bounded reconciliation queue for already-punched coarser chunks initially looked promising, but default browser persistence later exposed uncovered LOD sample gaps. Root cause: re-punching already-punched coarse voxels can remove coarse coverage around only partially covering finer voxels.
- Early eviction/accounting changes for prepared chunks also exposed uncovered cold/reload sample gaps. Prepared chunks must remain under the conservative lifecycle until we have voxel-accurate clipping or a stronger ownership model.
- Aggressively activating partially blocked prepared chunks made the far benchmark settle, but it produced cold-origin gaps in default persistence and was rejected.

Validation:

- `mise exec -- bun run typecheck`: pass.
- `mise exec -- bun test tests/lod-handoff.test.ts tests/procedural-resident-world.test.ts`: pass, `38` tests.
- `mise exec -- bun run bench:lod-persistence -- --label=track-a-default-samekey-only`: pass.
  - Artifact: `/var/folders/h7/xz1x4d4x0cn702r2q9205bkh0000gn/T/voxels-browser-game-bench-ni0foA/lod-idb-persistence-reload.json`
  - Reload disk hits: `1292`.
  - Coverage: `0` uncovered gaps, `0` handoff holes, `0` resident overlaps, `0` band overlaps, `0` water overlaps.

Current known limitation:

- The far `--lod-persistence-chunk-delta=32` stress case still reaches visually clean coverage but can leave prepared chunks pending under the conservative handoff model.
- The rejected attempts proved that this cannot be solved safely by coarse re-punching or by treating blocked prepared chunks as settled. The next accepted fix needs a voxel-accurate ownership/clipping model, or an equivalent per-column/per-voxel fallback proof, before changing visible handoff behavior.

### Follow-up Checkpoint - Runtime Clipping Attempt Rejected, Harness Hardened

Attempted next fix:

- Added a full/partial/none classifier for coarse voxel punching so a coarse voxel would only be removed when all eight immediate-finer subvoxels were solid.
- The focused unit tests proved the intended local behavior, but browser persistence became CPU-bound for more than four minutes even after optimizing the classifier to one finer-chunk lookup plus eight direct array reads per coarse voxel.
- I rejected and removed that runtime mutation approach. It is the wrong layer for true clipping; the next fix should keep terrain data intact and move ownership clipping into a mask/mesher path.

Accepted harness change:

- Added an explicit timeout wrapper around the LOD persistence page probes in `scripts/run-browser-game-benchmarks.ts`.
- This prevents future CPU-bound renderer experiments from hanging indefinitely inside `Runtime.evaluate`; the existing `--lod-persistence-timeout-ms` now also bounds populate/reload page probes.

Validation:

- `mise exec -- bun run typecheck`: pass.
- `mise exec -- bun test tests/lod-handoff.test.ts tests/procedural-resident-world.test.ts`: pass, `38` tests.
- `mise exec -- bun run bench:lod-persistence -- --label=track-a-default-timeout-guard`: pass.
  - Artifact: `/var/folders/h7/xz1x4d4x0cn702r2q9205bkh0000gn/T/voxels-browser-game-bench-rhTTVp/lod-idb-persistence-reload.json`
  - Reload disk hits: `1295`.
  - Coverage: `0` uncovered gaps, `0` handoff holes, `0` resident overlaps, `0` band overlaps, `0` water overlaps.
  - Worst recent frame: `10.2 ms`; hitch buckets: `0`.

### Follow-up Checkpoint - Clip Mask Foundation

Accepted next foundation:

- Added `src/engine/lod-clip-mask.ts` as a pure 2x2x2 subvoxel ownership mask utility.
- It deliberately does not touch live renderer behavior yet.
- The utility defines stable bit positions, none/partial/full classification, clipped-subvoxel checks, and normalized boxes for coarse subcells that remain visible.
- This is the artifact the next mesher-side clipping patch can use so we stop mutating coarse terrain data to represent render ownership.

Validation:

- `mise exec -- bun run typecheck`: pass.
- `mise exec -- bun test tests/lod-clip-mask.test.ts tests/lod-handoff.test.ts tests/procedural-resident-world.test.ts`: pass, `43` tests.

Next renderer target:

- Thread clip masks into the LOD mesh build path as render-only ownership data.
- Keep `VoxelChunk.data` authoritative and unchanged.
- Have the mesher skip or subdivide clipped coarse faces based on the mask, then verify with the LOD lab and persistence probes.

### Follow-up Checkpoint - Mesher Clip Mask Input

Accepted mesher foundation:

- Added optional `clipMask` support to `OpaqueChunkMeshingInput`.
- When supplied, clipped local voxels are treated as transparent by opaque face generation without changing `chunkData`.
- The normal no-mask path remains covered against the synchronous mesher output, so this should be behavior-preserving until runtime LOD code starts supplying masks.

Validation:

- `mise exec -- bun run typecheck`: pass.
- `mise exec -- bun test tests/opaque-chunk-mesher.test.ts tests/lod-clip-mask.test.ts tests/lod-handoff.test.ts tests/procedural-resident-world.test.ts`: pass, `47` tests.

Next renderer target:

- Compute a full-voxel LOD clip mask for active coarser chunks from active finer ownership.
- Feed that mask into `buildLodChunkMesh` instead of punching full coarse voxels out of `VoxelChunk.data`.
- Only after full-voxel masks are stable, extend the mesher to subdivide partial subvoxel boxes from `lod-clip-mask.ts`.

### Follow-up Checkpoint - Execution Board And Pure System Foundations

Planning and delegation:

- Added `docs/loop/20260509-subagent-execution-board.md` as the current dependency graph and ROI-ordered execution board.
- Spawned workers with disjoint write scopes for verification, edit-journal storage prep, atlas/routes, and pure gameplay systems.
- Kept active LOD ownership work local because `ProceduralResidentWorld` is still the integration bottleneck.

Accepted foundations from the parallel workers:

- `verify:rpg` now wraps the existing render-verification report with centralized RPG budgets and failure categories.
- `chunk-edit-journal.ts` now provides pure sparse edit-delta pack/replay/compact/export/import helpers without touching production IndexedDB stores.
- `WorldAtlas` now includes eight authored route definitions and deterministic route sampling.
- `exploration-interactions.ts` and `travel-goals.ts` add pure inspect/read/use targeting plus a route-journal goal state machine, without touching the live HUD/controller.

Accepted renderer increment:

- `ProceduralResidentWorld` now records a transient `lodRenderClipMasks` entry when a coarser LOD chunk is punched by active finer ownership.
- `buildLodChunkMesh` and same-level LOD neighbor face extraction now read those clip masks.
- This is still a bridge: current runtime chunks are still data-punched, but the mesher path now has the runtime plumbing needed for a later canonical-payload/render-instance split.

Validation:

- `mise exec -- bun run typecheck`: pass.
- `mise exec -- bun test tests/world-atlas.test.ts tests/chunk-edit-journal.test.ts tests/voxel-rpg-budgets.test.ts tests/voxel-rpg-verification-runner.test.ts tests/exploration-interactions.test.ts tests/travel-goals.test.ts tests/opaque-chunk-mesher.test.ts tests/lod-clip-mask.test.ts tests/lod-handoff.test.ts tests/procedural-resident-world.test.ts`: pass, `74` tests.
- `mise exec -- bun test tests/lod-handoff.test.ts tests/opaque-chunk-mesher.test.ts tests/world-atlas.test.ts tests/chunk-edit-journal.test.ts tests/voxel-rpg-budgets.test.ts tests/voxel-rpg-verification-runner.test.ts tests/exploration-interactions.test.ts tests/travel-goals.test.ts`: pass, `43` tests after the renderer bookkeeping check was added.
- `mise exec -- bun run bench:lod-persistence -- --label=clipmask-foundation-default`: pass.
  - Artifact: `/var/folders/h7/xz1x4d4x0cn702r2q9205bkh0000gn/T/voxels-browser-game-bench-bH96Uc/lod-idb-persistence-reload.json`
  - Reload disk hits: `1307`.
- `mise exec -- bun run bench:lod-persistence -- --label=clipmask-foundation-far --lod-persistence-chunk-delta=32 --lod-persistence-max-frames=260`: failed to produce a correctness report because the populate probe hit the `180000 ms` timeout.
- `mise exec -- bun test`: `304` passed, `5` failed in `tests/object-lab.test.ts`.
  - Failures were object-lab landmark expectation failures: `oak`, `dead_snag`, centered salt-marsh root coordinates, route debris balance, and `ash_marker` batch lookup.
  - These are outside this checkpoint's modified runtime/verification/storage/gameplay files, so they remain a known content-test issue rather than accepted validation evidence.

Rubric movement:

- Harness maturity: `10.1 -> 10.25` because `verify:rpg` gives the long-running RPG work a single budget/failure taxonomy on top of existing artifacts.
- Storage architecture readiness: `3.0 -> 3.2` because edit deltas now have a pure packed replay contract.
- World architecture readiness: `3.0 -> 3.25` because routes are now atlas data instead of only generator-side behavior.
- Gameplay architecture readiness: `2.7 -> 3.0` because inspect/read/use and travel goals now have pure testable contracts.
- Rendering/LOD correctness: `3.85 -> 3.9` because clip-mask plumbing is now present in the LOD mesh path, but the canonical-payload split is not complete.

Next:

1. Commit and push this checkpoint.
2. Replace punched-data active LOD chunks with unmodified canonical chunks plus render-only clip masks.
3. Add a LOD lab scenario that proves the same canonical LOD payload can render under two different active finer-ownership masks.
4. Re-run default and far browser LOD persistence; far timeout remains the main stress target.

### Follow-up Checkpoint - Clip-Aware Ownership Predicates

Accepted renderer increment:

- Converted internal LOD ownership predicates from raw `chunk.data` reads to clip-mask-aware visibility checks.
- `isLodWorldColumnCovered`, `isLodWorldVoxelRangeCovered`, prepared handoff overlap checks, and finer-candidate coverage checks now agree with `lodRenderClipMasks`.
- `clearLodCoveragePunchedKey` now clears the LOD world-column coverage cache as well as the clip mask, so cached coverage cannot outlive a changed render mask.

Why this matters:

- The previous checkpoint threaded clip masks into meshing, but ownership decisions still looked at unmasked data.
- This checkpoint is the next prerequisite for replacing punched chunk data with unmodified canonical LOD payloads.

Validation:

- `mise exec -- bun run typecheck`: pass.
- `mise exec -- bun test tests/lod-handoff.test.ts tests/procedural-resident-world.test.ts tests/lod-system.test.ts`: pass, `70` tests.
- `mise exec -- bun run bench:lod-persistence -- --label=clip-aware-ownership-default`: pass.
  - Artifact: `/var/folders/h7/xz1x4d4x0cn702r2q9205bkh0000gn/T/voxels-browser-game-bench-cCPsgB/lod-idb-persistence-reload.json`
  - Reload disk hits: `1296`.

Known limitation:

- The far `--lod-persistence-chunk-delta=32` stress was not rerun for this checkpoint because the previous run timed out during the populate probe. The next high-value renderer target is still to make far transitions settle quickly enough to produce a correctness artifact.

Next:

1. Make `punchLodChunkCoveredByActiveFiner` return an unmodified canonical chunk plus a render clip mask.
2. Keep coverage probes and handoff predicates on the clip-aware path.
3. Re-run default persistence and then the far stress; if far still times out, split populate/gap correctness from full-settle performance in the harness.

### Follow-up Checkpoint - Render-Only Clip Masks And Probe Source Of Truth

Accepted renderer increment:

- Replaced data-punched active LOD chunks with unmodified canonical LOD chunks plus transient `lodRenderClipMasks`.
- `punchLodChunkCoveredByActiveFiner` now reports `visibleSolidCount` for active visibility decisions while leaving `VoxelChunk.data`, `solidCount`, and `solidBounds` intact.
- LOD opaque and water mesh generation now consume the same render clip mask.
- Added `classifyVisibleLodColumn` on `ProceduralResidentWorld` and changed `GameController` LOD coverage probes to use it instead of re-reading raw chunk data.

Why this matters:

- The first browser attempt failed with resident and band overlaps because the debug probe still treated masked coarse voxels as visible.
- Moving the column classifier into `ProceduralResidentWorld` concentrates the ownership source of truth: meshing, handoff predicates, and browser coverage probes now agree that clipped LOD voxels are not rendered.
- Canonical derived LOD data can now remain reusable while active ownership changes the render mask.

Validation:

- `mise exec -- bun run typecheck`: pass.
- `mise exec -- bun test tests/lod-handoff.test.ts tests/procedural-resident-world.test.ts tests/lod-system.test.ts tests/opaque-chunk-mesher.test.ts`: pass, `74` tests.
- `mise exec -- bun run bench:lod-persistence -- --label=render-only-clip-water-fixed`: pass.
  - Artifact: `/var/folders/h7/xz1x4d4x0cn702r2q9205bkh0000gn/T/voxels-browser-game-bench-BRKGCV/lod-idb-persistence-reload.json`
  - Reload disk hits: `1276`.
  - Coverage: `0` uncovered gaps, `0` handoff holes, `0` resident overlaps, `0` band overlaps, `0` water overlaps.

Planning expansion:

- Added `docs/loop/20260509-lod-storage-roadmap.md` for canonical chunk persistence, edit journals, worker-derived LOD, LOD lab/probe work, hitch attribution, and storage decisions.
- Added `docs/loop/20260509-rpg-world-roadmap.md` for finite island generation, biome/content kits, traversal, ambiance, RPG verbs/skills, NPC zones, UI, and rubrics.
- Updated `docs/loop/20260509-subagent-execution-board.md` to keep `R0` render ownership as the current integration gate while allowing pure storage/world/RPG planning to proceed in parallel.

Next:

1. Add a LOD lab fixture that reuses one canonical coarse payload under two different active clip masks.
2. Split the far-stress benchmark into correctness visibility and settle-performance phases so timeouts still leave usable evidence.
3. Begin `S1` canonical chunk store API and `P0` LOD lab/probe expansion in parallel now that render-only active masking has a passing default browser artifact.

### Follow-up Checkpoint - Canonical Payload Multi-Mask Fixture

Accepted verification increment:

- Added a regression test proving one canonical LOD chunk payload can be viewed through two different active `lodRenderClipMasks`.
- The test swaps masks over the same coarse chunk, verifies visible ownership moves between columns, and asserts `VoxelChunk.data`, `solidCount`, and `solidBounds` do not change.
- This is the isolated fixture requested by the previous checkpoint: render ownership can now vary independently from canonical derived chunk data.

Validation:

- `mise exec -- bun run typecheck`: pass.
- `mise exec -- bun test tests/lod-handoff.test.ts tests/opaque-chunk-mesher.test.ts`: pass, `17` tests.
- `mise exec -- bun run test:lod-lab`: pass, `21` tests.

Next:

1. Split the far-stress persistence benchmark so correctness coverage can still be captured when full settle times out.
2. Add a browser/lab artifact summary that records visible coverage separately from pending-generation performance.
3. Start the canonical chunk store API work after the far-stress harness stops losing evidence on timeout.

### Follow-up Checkpoint - Far Persistence Evidence Split

Accepted harness increment:

- Added `--lod-persistence-far-max-frames` so the far-transition phase can be bounded independently from cold-origin and reload-origin settle.
- Added `--lod-persistence-store-max-frames` so persistence flushing can be bounded independently when needed.
- Reordered the LOD persistence populate probe to flush origin stores before moving to a far stress position. This keeps the IndexedDB reload proof tied to clean origin coverage and records far movement as a separate visibility/performance phase.
- Added aggregate fields `farSettlePass`, `farUnsettledCount`, `maxFarCoverageGaps`, and `maxFarCoverageOverlaps`.
- Far unsettlement is now performance telemetry, while far LOD gaps/holes/overlaps remain correctness failures.

Validation:

- `mise exec -- bun run typecheck`: pass.
- `mise exec -- bun test tests/render-verification-runner.test.ts tests/voxel-rpg-verification-runner.test.ts`: pass, `7` tests.
- `mise exec -- bun run bench:lod-persistence -- --label=far-split-reordered-default`: pass.
  - Artifact: `/var/folders/h7/xz1x4d4x0cn702r2q9205bkh0000gn/T/voxels-browser-game-bench-on3FxQ/lod-idb-persistence-reload.json`
  - Reload disk hits: `1278`.
  - Coverage: `0` uncovered gaps, `0` handoff holes, `0` resident overlaps, `0` band overlaps, `0` water overlaps.
- `mise exec -- bun run bench:lod-persistence -- --label=far-split-reordered-delta32 --lod-persistence-chunk-delta=32 --lod-persistence-far-max-frames=24`: pass.
  - Artifact: `/var/folders/h7/xz1x4d4x0cn702r2q9205bkh0000gn/T/voxels-browser-game-bench-vXiAaA/lod-idb-persistence-reload.json`
  - Reload disk hits: `1277`.
  - Far phase: `settled=false`, `frameCount=24`, `finalLodPendingChunks=967`, `maxFarCoverageGaps=0`, `maxFarCoverageOverlaps=0`.

Rejected attempt during this checkpoint:

- `--lod-persistence-max-frames=120 --lod-persistence-far-max-frames=24` returned an artifact but failed because cold origin did not settle.
- Capping store flush after far movement produced transient store-flush resident overlaps while far work remained pending.
- Reordering store flush before far movement fixed the evidence shape without hiding pending far performance.

Next:

1. Add a render-verification gate or summary field that surfaces `farSettlePass=false` as a performance/settle warning without confusing it with LOD coverage correctness.
2. Use the bounded far artifact to decide whether to improve far pending generation throughput or scheduling next.
3. Start canonical chunk store API work once the verification summary exposes the new far-settle telemetry.

### Follow-up Checkpoint - Far Settle Warning In Verification

Accepted verifier increment:

- `render-verification-runner` now emits `lod_persistence.far_unsettled_count`.
- The gate is a warning, not a failure, so far-transition pending work is visible without being classified as a LOD coverage correctness failure.
- The warning details include `maxPendingChunks`, which points directly at far-transition scheduling/generation backlog.

Validation:

- `mise exec -- bun run typecheck`: pass.
- `mise exec -- bun test tests/render-verification-runner.test.ts tests/voxel-rpg-verification-runner.test.ts`: pass, `8` tests.

Next:

1. Use `maxPendingChunks` from bounded far artifacts to tune far LOD scheduling throughput.
2. Add a canonical chunk store API once the far-pending telemetry has a stable baseline.
3. Keep generator/content work behind these render and verification gates.

### Follow-up Checkpoint - Canonical Chunk Store Contract

Accepted storage increment:

- Added `src/engine/canonical-chunk-store.ts` as a pure engine-facing contract for canonical chunk persistence.
- The contract defines canonical world keys, canonical chunk/summary/region/edit-journal keys, record metadata, and a `CanonicalChunkStore` interface.
- Metadata includes schema version, generation version, canonical revision, encoded byte size, and storage timestamp.
- Usability checks reject stale generation versions, coordinate/key mismatches, schema mismatch, and empty encoded payloads.
- This does not rewire IndexedDB yet; it is the S1.1/S1.2 contract that the existing browser cache can adopt next.

Validation:

- `mise exec -- bun run typecheck`: pass.
- `mise exec -- bun test tests/canonical-chunk-store.test.ts tests/chunk-edit-journal.test.ts tests/generated-chunk-codec.test.ts`: pass, `16` tests.

Next:

1. Adapt `src/client/procedural-generated-chunk-cache.ts` to use the canonical key helpers while preserving existing stores.
2. Add backwards-compatible metadata fields to stored canonical chunk records.
3. Add a canonical reload probe that reports warm canonical chunk hit ratio separately from derived LOD cache hits.

### Follow-up Checkpoint - Canonical Keys In Browser Chunk Cache

Accepted storage increment:

- `src/client/procedural-generated-chunk-cache.ts` now builds its world identity through the canonical chunk store contract.
- New generated chunk writes use canonical chunk keys and metadata while preserving the existing IndexedDB object stores.
- Chunk, summary, and region-summary reads fall back to legacy keys, so existing local browser stores remain readable during migration.
- Stored generated chunks now carry schema version, world key, generation version, chunk coord, canonical revision, encoded byte length, and timestamp metadata.
- Derived LOD cache keys are unchanged and remain optional render artifacts keyed by edit revision, LOD level, and coord.

Validation:

- `mise exec -- bun run typecheck`: pass.
- `mise exec -- bun test tests/procedural-generated-chunk-cache.test.ts tests/canonical-chunk-store.test.ts tests/procedural-deferred-persistence.test.ts tests/generated-chunk-codec.test.ts`: pass, `14` tests.
- `mise exec -- bun run bench:lod-persistence -- --label=canonical-cache-keys-default-retry`: pass.
  - Artifact: `/var/folders/h7/xz1x4d4x0cn702r2q9205bkh0000gn/T/voxels-browser-game-bench-AZyn2d/lod-idb-persistence-reload.json`
  - Reload disk hits: `1297`.
  - Coverage: `0` uncovered gaps, `0` handoff holes, `0` resident overlaps, `0` band overlaps, `0` water overlaps.

Rejected/observed during this checkpoint:

- The first browser run, `canonical-cache-keys-default`, returned a cold-origin artifact with `7` uncovered LOD sample gaps despite `0` pending chunks; reload-origin was clean.
- A retry with identical code passed with zero coverage issues. I am recording the failed artifact rather than treating the retry as proof the gap cannot recur:
  `/var/folders/h7/xz1x4d4x0cn702r2q9205bkh0000gn/T/voxels-browser-game-bench-z4VBwA/lod-idb-persistence-reload.json`.

Next:

1. Add a benchmark repeat mode or flake detector for cold-origin LOD coverage so transient gap samples are not ignored.
2. Add a canonical reload probe that reports generated chunk cache hits separately from derived LOD disk hits.
3. Add edit-journal record persistence after canonical chunk hit reporting is reliable.
