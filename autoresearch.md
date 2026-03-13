# Autoresearch: Game Stream Performance

## Objective
Optimize the total CPU time for the game-stream crossing-d2 scenario: a 2-chunk anchor crossing that exercises chunk generation, greedy meshing, and far-field LOD rebuilds. This is the core per-crossing cost that determines how smoothly the world streams around the player.

## Metrics
- **Primary**: `total_ms` (ms, lower is better) — sum of stream + mesh + far-field time for crossing-d2
- **Secondary**: `stream_ms`, `mesh_ms`, `far_field_ms`, `chunk_gen_ms`, `max_frame_ms`, `far_sample_cache_ms`, `far_mesh_build_ms`

## How to Run
`./autoresearch.sh` — outputs `METRIC name=number` lines.

## Files in Scope
- `src/engine/procedural-generator.ts` — Procedural terrain generation (4324 lines). Generates chunk voxel data from noise. **Biggest CPU cost** (~240ms of stream time).
- `src/engine/mesher.ts` — Greedy mesher producing vertex/index buffers from chunk data (~1100 lines). ~180ms.
- `src/engine/opaque-chunk-mesher.ts` — Standalone opaque mesher with Int32Array quad buffer (~805 lines). Used for async path.
- `src/engine/procedural-resident-world.ts` — Manages chunk residency, generation budgets, eviction (~1646 lines).
- `src/engine/procedural-far-field.ts` — Far-field LOD band system. ~280ms total, dominated by sample cache (~230ms).
- `src/engine/noise.ts` — FBM noise functions used by the generator.
- `src/engine/world.ts` — Core voxel world with chunk storage (~506 lines).
- `src/engine/stream-work.ts` — Frame work budget helper (33 lines).
- `src/engine/water-visuals.ts` — Water depth tinting.
- `src/engine/math.ts` — Math utilities.

## Off Limits
- Test files (don't weaken tests to pass)
- Scene definitions, types, renderer, camera, server, client UI
- Don't change benchmark parameters or methodology
- Don't change the procedural generation output (world must look identical)

## Constraints
- `bun run typecheck` must pass
- `bun test` must pass (180 tests, correctness is non-negotiable)
- Generated terrain must be bit-identical (same seed → same voxels)
- No new dependencies

## What's Been Tried
*(Updated as experiments run)*
