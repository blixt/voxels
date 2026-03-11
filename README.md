# Voxels

Chrome 146 WebGPU voxel game and engine built from scratch with Bun and TypeScript.

## Workflow

1. Install tooling with `mise install`.
2. Install dependencies with `bun install`.
3. Start the app with `mise run dev`.
4. Open `http://localhost:3000/` for the game and `http://localhost:3000/bench` for the benchmark suite.
5. Build production output with `mise run build`, then run it with `mise run serve`.

`mise run dev` now uses Bun's hot-reload/full-stack dev path instead of the older startup bundle flow, so edits to the server, page shells, CSS, and browser entrypoints should update without manual cache-busting.

The implementation notes, research links, and verification log live under `docs/loop/`.
The staged game/engine roadmap lives in `docs/roadmap.md`.
A broader architecture/reference survey lives in `docs/20260311-voxel-research.md`.
The Bun live-edit/HMR research note lives in `docs/20260311-bun-hmr-research.md`.
Repo-specific guidance for agent-driven research, implementation, and verification lives in `docs/agent-playbook.md`.

## Features

- Full-screen `/` game shell with a first-person camera bootstrap
- Procedural resident-world bootstrap with lazy chunk streaming around the player
- Sparse chunked voxel storage with live edits
- CPU greedy meshing for WebGPU rendering
- Current orthographic isometric renderer baseline and benchmark scenes
- Custom `VXSC` scene format with RLE-compressed chunk payloads
- MagicaVoxel `.vox` import
- Repeatable benchmark harness with separate primitive validation and performance scenes
- Targeted stress scenes for draw calls, tiny-surface throughput, overdraw, and edit-heavy workloads
- Image-diff validation against a software reference renderer
- Browser-exposed debug surfaces for both the game and benchmark paths

## Current direction

- `/` now boots into the first dedicated game slice instead of the old demo playground UI.
- The world target is procedurally generated, lazily streamed, effectively infinite in `X/Z`, and bounded to `Y = 0..16383`.
- The architecture is being pushed toward persistence, remote authority, and multiplayer instead of standalone scenes.
- `/bench` remains the correctness and performance oracle while the game runtime grows.

## Game automation

- `window.__VOXELS_GAME__.snapshot()`
- `window.__VOXELS_GAME__.snapshotResidentWorld()`
- `window.__VOXELS_GAME__.teleport(x, y, z)`
- `window.__VOXELS_GAME__.teleportAndSettle(x, y, z, { radiusChunks })`
- `window.__VOXELS_GAME__.requestPointerLock()`
- `window.__VOXELS_GAME__.setViewDistance(chunks)`
- `window.__VOXELS_GAME__.forceResidencyUpdate()`

## Benchmark automation

- `http://localhost:3000/bench?auto=1&scenario=terrain256&iterations=2&frames=60`
- `http://localhost:3000/bench?auto=1&suite=stress&iterations=1&frames=30`
- `window.__VOXELS_BENCH__.run(sceneId, iterations, frameCount)`
- `window.__VOXELS_BENCH__.runStress(iterations, frameCount)`
- `window.__VOXELS_BENCH__.runAll(iterations, frameCount)`
- `window.__VOXELS_BENCH__.probeGeneration({ seed, chunkCoords, chunkSize })`
- `mise run profile -- --iterations=3 --warmup=1 terrain256 stressDrawCalls512`
- `mise run profile-stream -- --iterations=3 --warmup=1`
- `mise run profile-game-stream -- --iterations=2 --warmup=1 --radius=5 --generate-budget=6 --mesh-budget=4 --chunk-delta=2`

The benchmark table separates first-frame costs from warm-frame costs and exposes first-frame sync/upload/encode metrics so scene-load and live-edit regressions are visible without manual spreadsheet work.
