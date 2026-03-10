# Voxels

Chrome 146 WebGPU voxel engine built from scratch with Bun and TypeScript.

## Workflow

1. Install tooling with `mise install`.
2. Install dependencies with `bun install`.
3. Start the app with `mise run dev`.
4. Open `http://localhost:3000/` for the playground and `http://localhost:3000/bench` for the benchmark suite.

The implementation notes, research links, and verification log live under `docs/loop/`.

## Features

- Sparse chunked voxel storage with live edits
- CPU greedy meshing for WebGPU rendering
- Orthographic isometric default camera
- Custom `VXSC` scene format with RLE-compressed chunk payloads
- MagicaVoxel `.vox` import
- Repeatable benchmark harness with separate primitive validation and performance scenes
- Targeted stress scenes for draw calls, tiny-surface throughput, overdraw, and edit-heavy workloads
- Image-diff validation against a software reference renderer

## Benchmark automation

- `http://localhost:3000/bench?auto=1&scenario=terrain256&iterations=2&frames=60`
- `http://localhost:3000/bench?auto=1&suite=stress&iterations=1&frames=30`
- `window.__VOXELS_BENCH__.run(sceneId, iterations, frameCount)`
- `window.__VOXELS_BENCH__.runStress(iterations, frameCount)`
- `window.__VOXELS_BENCH__.runAll(iterations, frameCount)`
