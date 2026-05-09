# Canonical LOD Architecture Contract

Date: 2026-05-09

Track A owns the storage and LOD architecture contract for the procedural voxel RPG world. The target is a robust Morrowind-like engine where terrain has one durable source of truth, edits survive reloads, derived LOD is cheap and disposable, LOD handoff is gap-free and overlap-free, and warm reloads reuse disk data without trusting stale visual artifacts.

This document is code-facing: each rule below should map to a small set of APIs, tests, and benchmark gates.

## Goals

- Persist canonical chunks and edits once.
- Derive summaries and LOD from canonical chunk data plus edit deltas.
- Treat derived LOD chunks as disposable render artifacts.
- Keep visible ownership exact during LOD switching.
- Make far transitions fast by doing derivation off the main thread and reusing canonical disk data.
- Preserve existing safe fallback behavior when IndexedDB or workers are unavailable.

## Non-Goals

- Do not make derived LOD a durable source of truth.
- Do not require all canonical source chunks to be resident in `ProceduralResidentWorld`.
- Do not use renderer depth bias as the primary correctness mechanism for LOD ownership.
- Do not persist edited full chunks as a second terrain authority unless explicitly compacting a canonical edit journal entry.

## Source-of-Truth Rules

1. **Canonical terrain**
   - The canonical material for a voxel is:
     `procedural generated chunk material + persisted edit delta + live edit overlay`.
   - Canonical chunks are addressed by generation version, seed, sea level, chunk size, max Y, and chunk coordinate.
   - Canonical edit deltas are addressed by the same world key plus chunk coordinate and edit revision.

2. **Summaries**
   - Chunk summaries and region summaries are derived indexes over canonical data.
   - They are allowed to drive planning, Y-range estimates, and cache prefetch.
   - They must not be used as material authority for derived LOD.
   - Edited chunks must invalidate or refresh affected summaries.

3. **Derived LOD**
   - Derived LOD chunks are render artifacts.
   - They may be retained in memory for fast return trips.
   - They may be transferred through `derived-lod-chunk-codec.ts`.
   - If a disk LOD cache remains, it is strictly optional and must be validated against canonical generation version and edit revision. Clearing it must not alter correctness.

4. **Rendering**
   - `iterateResidentChunks()` must expose only active render owners.
   - Prepared, stale, retained, and pending chunks must not be uploaded or drawn unless they are explicitly part of the active ownership set.
   - LOD depth bias is a visual safety net, not the ownership model.

5. **Edits**
   - Every edit increments a world edit revision and records a chunk-scoped delta.
   - LOD derivation for any footprint containing edited chunks must consume those deltas.
   - Reload must replay persisted deltas before exposing LOD0 or derived LOD for the edited footprint.

## Storage Contract

### Stores

The IndexedDB schema should evolve toward:

```ts
interface CanonicalChunkRecord {
  key: string;
  encodedBuffer: ArrayBuffer;
  encodedByteLength: number;
  generationVersion: string;
  storedAt: number;
}

interface ChunkEditJournalRecord {
  key: string;
  worldKey: string;
  coord: ChunkCoordinate;
  baseGenerationVersion: string;
  minRevision: number;
  maxRevision: number;
  edits: PackedChunkEditDelta[];
  storedAt: number;
}

interface ChunkSummaryRecord {
  key: string;
  summary: TransferredGeneratedChunkRenderSummary;
  canonicalRevision: number;
  storedAt: number;
}

interface RenderSummaryRegionRecord {
  key: string;
  summary: TransferredGeneratedRenderSummaryRegion;
  canonicalRevision: number;
  storedAt: number;
}
```

`lod_chunks` can remain during migration for backwards compatibility, but Track A code must not require it for correctness.

### API Sketch

```ts
interface CanonicalChunkStore {
  getCanonicalChunk(coord: ChunkCoordinate): Promise<CachedEncodedGeneratedChunk | null>;
  putCanonicalChunk(coord: ChunkCoordinate, chunk: EncodedGeneratedChunk, summary: TransferredGeneratedChunkRenderSummary): Promise<void>;

  getEditJournal(coord: ChunkCoordinate): Promise<ChunkEditJournalRecord | null>;
  appendEditDeltas(coord: ChunkCoordinate, deltas: PackedChunkEditDelta[], revision: number): Promise<void>;

  getChunkSummary(coord: ChunkCoordinate): Promise<TransferredGeneratedChunkRenderSummary | null>;
  putChunkSummary(coord: ChunkCoordinate, summary: TransferredGeneratedChunkRenderSummary, canonicalRevision: number): Promise<void>;

  getRegionSummary(coord: RenderSummaryRegionCoordinate): Promise<TransferredGeneratedRenderSummaryRegion | null>;
  close(): void;
}
```

Recommended file ownership:

- `src/client/procedural-generated-chunk-cache.ts`: IndexedDB schema and canonical store API.
- `src/client/procedural-deferred-persistence.ts`: queued chunk, summary, and edit-journal writes.
- `src/engine/generated-chunk-codec.ts`: canonical chunk codec.
- New optional `src/engine/chunk-edit-journal.ts`: edit delta packing, replay, compaction.

## Effective Canonical Chunk Contract

All LOD derivation and LOD0 residency should consume an effective chunk:

```ts
interface EffectiveCanonicalChunk {
  coord: ChunkCoordinate;
  data: Uint16Array;
  solidCount: number;
  solidBounds: GeneratedChunk["solidBounds"];
  renderSummary: GeneratedChunkRenderSummary;
  canonicalRevision: number;
  source: "disk" | "generated" | "memory";
  appliedEditCount: number;
}

interface EffectiveCanonicalChunkSource {
  getEffectiveChunk(coord: ChunkCoordinate): Promise<EffectiveCanonicalChunk>;
  getKnownEmpty(coord: ChunkCoordinate): Promise<boolean>;
  getColumnSummary(cx: number, cz: number): Promise<GeneratedRenderColumnSummary | null>;
}
```

Rules:

- `getEffectiveChunk()` may generate a missing chunk, persist it, then apply edits.
- Applying edits must recompute `solidCount`, `solidBounds`, and `renderSummary`.
- For resident chunks, live overlays may be applied synchronously in `ProceduralResidentWorld` until the async store catches up.
- A missing worker or missing IndexedDB falls back to current synchronous generation, but the same source-of-truth rule still applies.

## Worker Derivation Contract

LOD generation should move from main-thread material sampling to worker derivation from canonical sources.

### Request/Response Sketch

```ts
interface DeriveLodChunkRequest {
  type: "derive-lod-chunk";
  requestId: number;
  key: {
    lodLevel: number;
    coord: ChunkCoordinate;
    canonicalRevision: number;
  };
  sourceFootprint: {
    minChunkX: number;
    maxChunkX: number;
    minChunkY: number;
    maxChunkY: number;
    minChunkZ: number;
    maxChunkZ: number;
  };
}

interface DerivedLodChunkResponse {
  type: "derived-lod-chunk";
  requestId: number;
  key: DeriveLodChunkRequest["key"];
  source: {
    generatedChunks: number;
    diskChunkHits: number;
    editDeltasApplied: number;
  };
  chunk: DerivedLodChunkPayload;
}
```

Recommended file ownership:

- `src/engine/async-chunk-generation.ts`: queue interface and key helpers.
- `src/client/async-procedural-chunk-generation.ts`: request scheduling and completion draining.
- `src/client/procedural-generation-worker.ts`: canonical chunk fetch/generate/edit replay/LOD derivation.
- `src/engine/derived-lod-chunk-codec.ts`: transfer payload codec, not durable terrain authority.

Worker derivation rules:

- Fetch or generate canonical source chunks for the full LOD footprint.
- Replay persisted edit journal and live edit batch supplied by the main thread.
- Downsample from effective canonical materials only.
- Return empty LOD chunks explicitly when the full source footprint is known empty.
- Report source stats for benchmarks.

## ProceduralResidentWorld Contract

`ProceduralResidentWorld` owns resident LOD state and visible ownership. It should not own durable storage policy.

### State Model

```ts
type LodResidencyState =
  | "active"
  | "prepared"
  | "retiring"
  | "retained"
  | "empty"
  | "stale";
```

Rules:

- `active`: may be yielded from `iterateResidentChunks()`.
- `prepared`: fully meshed and render-ready, but not visible yet.
- `retiring`: still visible until replacement commit completes.
- `retained`: memory-only reuse candidate, not visible.
- `empty`: known-empty derived result.
- `stale`: cannot be reused without rederivation against current canonical revision.

### API Sketch

```ts
private prepareLodChunk(key: string, chunk: VoxelChunk): void;
private commitPreparedLodChunks(): LodCommitSummary;
private retireCoveredCoarserChunks(finerCandidates: Iterable<VoxelChunk>): void;
private markLodFootprintStale(reason: "edit" | "lod0-ready" | "source-changed", footprint: LodFootprint): void;
```

Implementation rules:

- All generated, retained, and worker-returned LOD chunks must pass through `prepareLodChunk()` or the same activation predicate.
- `commitPreparedLodChunks()` is the only place where a prepared finer chunk can become active while an overlapping coarser chunk is removed.
- LOD0 render-ready columns should be treated as finer candidates in the same ownership algorithm.
- Stale active chunks may remain visible only while no replacement has committed.
- Punched coarser chunks are a temporary compatibility path. Long term, prefer exact ownership commit over partial coarser mutation.

Recommended file ownership:

- `src/engine/procedural-resident-world.ts`: state machine, planning, handoff, metrics.
- `src/engine/procedural-probes.ts`: low-cost invariant snapshots.
- `src/client/game-controller.ts`: budget configuration and benchmark/HUD surfacing.

## Handoff Invariants

For every visible sampled world column:

1. If LOD0 render-ready coverage exists, LOD0 owns that column.
2. Else the finest active LOD chunk with a non-empty covered column owns it.
3. If a finer prepared chunk overlaps a coarser active chunk, it remains invisible until the coarser chunk's covered columns are fully replaced by active or prepared finer owners.
4. Removing a coarser active chunk and activating its finer replacements happens in one commit.
5. Empty LOD results count as coverage only when the canonical source footprint is fully known empty.
6. Stale active chunks may cover gaps temporarily, but must not be retained or persisted as reusable LOD.

Renderer acceptance:

- `iterateResidentChunks()` yields active LOD chunks coarsest-first, then LOD0.
- No prepared, retained, or stale-nonactive chunks are yielded.
- Depth bias remains only to make transitional overlap visually stable if a bounded overlap still exists.

## Migration Sequence

### M1. Contract and Metrics

- Add the state and source-of-truth rules to code comments around LOD residency entry points.
- Add metrics for canonical chunk hits, generated chunk misses, edit deltas replayed, worker-derived LOD count, and optional derived LOD cache hits.

### M2. Canonical Edit Journal

- Add edit journal persistence and replay.
- Keep live overlays as the immediate write-through cache.
- Add compaction for chunk-local deltas after repeated edits.

### M3. Effective Canonical Chunk API

- Introduce effective chunk loading in the worker and a synchronous fallback in the world.
- Convert LOD0 async generation to use effective chunks.
- Summaries become derived indexes with canonical revision stamps.

### M4. Worker LOD Derivation

- Add `derive-lod-chunk` requests.
- Keep current main-thread `downsampleLodChunkData()` as fallback.
- Stop using generator-only material shortcuts for authoritative LOD output.

### M5. Single Handoff State Machine

- Route all LOD creation paths through `prepareLodChunk()` and `commitPreparedLodChunks()`.
- Make LOD0 ready events participate in the same handoff logic.
- Remove direct active insertion paths except where no coarser overlap exists.

### M6. Durable LOD Decommission

- Stop requiring `lod_chunks` for warm reload.
- Optionally retain it as a best-effort disposable cache with canonical revision validation.
- Add a benchmark mode that clears `lod_chunks` but keeps canonical chunks and edits.

## Tests to Add or Update

### Unit Tests

- `tests/canonical-world-store.test.ts`
  - stores generated chunk once;
  - appends edit deltas;
  - replays edits into an effective chunk;
  - refreshes summary revision after edits.

- `tests/chunk-edit-journal.test.ts`
  - packs and unpacks repeated edits;
  - last edit wins per voxel;
  - compaction preserves final material state.

- `tests/async-chunk-generation.test.ts`
  - worker derivation keys include canonical revision;
  - pending derivation dedupes by LOD coord and revision.

- `tests/procedural-resident-world.test.ts`
  - worker-returned LOD goes through prepared state;
  - stale active chunks are not retained;
  - LOD0 render-ready commits through the same ownership path.

- `tests/lod-handoff.test.ts`
  - no gap while worker derivation is pending;
  - no active overlap after commit;
  - edited footprint updates LOD0 and derived LOD after reload.

### Integration and Benchmark Tests

- Extend `tests/browser-game-benchmark-harness.test.ts` for canonical-only warm reload setup.
- Extend `scripts/run-browser-game-benchmarks.ts` `bench:lod-persistence` scenario:
  - cold origin;
  - far travel;
  - reload origin with canonical store retained and derived LOD cleared;
  - verify no visual coverage regression.

- Extend `scripts/profile-lod-residency.ts`:
  - worker-derived LOD count;
  - main-thread fallback count;
  - canonical disk hits;
  - edit replay count.

- Extend `scripts/profile-lod-cache-reuse.ts`:
  - split memory retained LOD hits from canonical disk reuse;
  - add mode that disables derived LOD disk reuse.

## Acceptance Gates

Correctness gates:

- Route benchmark reports zero `uncoveredGapCount` and zero `handoffHoleCount`.
- Post-commit `bandOverlapCount` is zero for active ownership probes.
- Clearing derived LOD storage does not change rendered coverage after warm reload.
- Edited voxels appear in LOD0 and all relevant derived LOD after reload.
- Stale active LOD chunks are never persisted or retained as reusable LOD.

Performance gates:

- Warm reload of a previously visited route generates at least 80% fewer canonical chunks than cold load.
- Moving p95 `lodMs` is under 4 ms.
- Moving max LOD work is under 12 ms outside initial bootstrap.
- Worker-derived LOD accounts for at least 90% of non-fallback LOD chunks when workers are available.
- Main-thread fallback remains correct and bounded when workers or IndexedDB are unavailable.

Storage gates:

- Canonical chunks are persisted once per world key and chunk coordinate.
- Edit journal size remains bounded by chunk-local compaction.
- Region summaries can be rebuilt from canonical chunks and edits.
- Derived LOD cache can be deleted independently of canonical stores.

## Risks and Mitigations

- **Worker derivation increases disk reads.**
  - Batch source-footprint reads and prefetch canonical chunks near planned LOD rings.

- **Edit replay becomes expensive.**
  - Compact edit journals per chunk and keep live overlays in memory for hot chunks.

- **Summaries go stale after edits.**
  - Stamp summaries with canonical revision and rebuild affected columns during edit flush.

- **Handoff state becomes too complex.**
  - Keep all visibility changes inside `commitPreparedLodChunks()` and test with synthetic full-coverage chunks before procedural worlds.

- **Removing durable LOD cache regresses warm far transitions.**
  - Use memory retained LOD cache first; add optional validated derived disk cache only after canonical reload metrics pass.

- **IndexedDB migration breaks existing players.**
  - Keep old stores readable, avoid destructive deletion in the first migration, and fall back to regeneration on decode failure.

## Parallel Work Boundaries

Can run in parallel:

- Canonical store/edit journal implementation and handoff state-machine tests.
- Worker derivation request plumbing and benchmark metric wiring.
- Reload benchmark scenario and unit tests for edit journal packing.
- Renderer verification, as long as renderer remains a consumer of active ownership only.

Should not run in parallel without coordination:

- Changes to `ProceduralResidentWorld.updateLodResidencyAround()`.
- IndexedDB version/schema migration.
- `AsyncChunkGenerationQueue` interface changes.
- LOD handoff ownership predicates.

Track A public contract for other tracks:

- Other tracks may request canonical terrain through effective chunk APIs.
- Other tracks must not read or write derived LOD as durable gameplay state.
- Other tracks may add visual effects or renderer optimizations only after active ownership invariants remain observable through probes and benchmarks.
