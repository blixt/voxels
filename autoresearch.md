# Autoresearch: Game Stream Performance

## Objective
Optimize the total CPU time for the game-stream crossing-d2 scenario: a 2-chunk anchor crossing that exercises chunk generation and greedy meshing. This is the core per-crossing cost that determines how smoothly the world streams around the player.

## Metrics
- **Primary**: `total_ms` (ms, lower is better) — sum of stream + mesh time for crossing-d2
- **Secondary**: `stream_ms`, `mesh_ms`, `chunk_gen_ms`, `max_frame_ms`

## How to Run
`./autoresearch.sh` — outputs `METRIC name=number` lines.

## Files in Scope
- `src/engine/procedural-generator.ts` — Procedural terrain generation. Generates chunk voxel data from noise. **Biggest CPU cost** (~240ms of stream time). Also provides `generateChunkAtLod()` for multi-LOD chunk generation.
- `src/engine/mesher.ts` — Greedy mesher producing vertex/index buffers from chunk data (~1100 lines). ~180ms.
- `src/engine/opaque-chunk-mesher.ts` — Standalone opaque mesher with Int32Array quad buffer (~805 lines). Used for async path and LOD chunk meshing.
- `src/engine/procedural-resident-world.ts` — Manages chunk residency, generation budgets, eviction, and multi-LOD chunk management.
- `src/engine/noise.ts` — FBM noise functions used by the generator.
- `src/engine/world.ts` — Core voxel world with chunk storage.
- `src/engine/stream-work.ts` — Frame work budget helper.
- `src/engine/renderer.ts` — WebGPU renderer with frustum culling.
- `src/engine/water-visuals.ts` — Water depth tinting.
- `src/engine/math.ts` — Math utilities.

## Off Limits
- Test files (don't weaken tests to pass)
- Scene definitions, types, camera, server, client UI
- Don't change benchmark parameters or methodology
- Don't change the procedural generation output (world must look identical)

## Constraints
- `bun run typecheck` must pass
- `bun test` must pass (correctness is non-negotiable)
- Generated terrain must be bit-identical (same seed → same voxels)
- No new dependencies

## Architecture
The rendering pipeline uses a unified multi-LOD chunk system:
- **LOD 0**: radius 8 chunks, stride 1 (32 WU/chunk) — full resolution near-field
- **LOD 1-4**: progressively coarser chunks (stride 2/4/8/16) extending view to ~340m
- All LOD levels produce 32³ `Uint16Array` chunks with identical format
- Frustum culling eliminates off-screen chunks from draw calls
- Fog starts at 96m, fully obscures at 416m

## What's Been Tried

### Wins
- **DataView → Float32Array/Uint32Array** for vertex writing in mesher.ts: mesh -10%
- **Reusable scratch objects** for SurfaceFieldSample, CaveFieldSample, fillSurfaceColumnState return: chunk_gen -12%
- **Reusable BaseBiomeBlendSelection** in selectBaseBiomes: chunk_gen -3%
- **Reusable RegionalVariantSelection** in selectRegionalVariant
- **Fixed-arity avg3/4/5** replacing variadic averageSignal: chunk_gen -7% (eliminated ~26 array allocs per column)
- **Precomputed opaque mask Uint8Array** in mesher: mesh -16% (avoids isWaterMaterial virtual dispatch)
- **Early Y-exit** in voxel loop + branch instead of Math.min/max: chunk_gen -8%
- **Unified multi-LOD system** replacing far-field: eliminated ~280ms far-field rebuild cost, simpler architecture

### Dead Ends
- Inlining hashUint32 into noise.ts: Bun already optimizes cross-module calls
- Converting mesher quad arrays from number[] to Int32Array: overhead from growth/append logic negated benefit
- Inline above-surface fast path in voxel loop: JIT regression from function complexity
- Precomputed packed normals: negligible impact
