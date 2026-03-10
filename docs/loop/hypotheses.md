# Renderer Hypotheses

## 2026-03-10 visual error search

| Hypothesis | Tiny verification case | Result | Status |
| --- | --- | --- | --- |
| `captureImage()` differs from the visible WebGPU canvas | Run `validationBlocks` in Chrome 146, inspect the live viewport, and compare it against the benchmark artifact after a cache-bypass reload | The live viewport and validation artifact agree once the page is running the current bundle | Rejected |
| Matrix layout or handedness is wrong | Project single points and single-voxel faces with `buildCameraMatrices()` and inspect projected bounds/orientation | CPU projection matched the software reference; row-major interpretation was obviously wrong | Rejected |
| Orthographic depth mapping is inverted | Compare clip-space depth for points stepped toward and away from the camera | Nearer points produce smaller depth values, matching `depthCompare: "less"` | Rejected |
| Face winding / culling is the main issue | Measure projected winding signs for single-voxel faces and render a custom cube through the current WebGPU depth/cull state | Winding was worth checking, but custom cube probes rendered correctly and did not reproduce the displaced frame | Rejected |
| Greedy mesher is emitting bad positions in the current source | Build the same single-voxel mesh under Bun and inside the browser page | Bun emitted `[1,1,1]..[2,2,2]`, while the cached browser page initially emitted `[1,1,1]..[10,10,10]` because it was still running an older bundle | Rejected for current source |
| A stale cached client bundle is serving pre-fix code | Compare browser-side single-voxel mesh bounds before and after `ignoreCache` reload, then rerun `validationBlocks` | Confirmed. After the no-cache reload, the browser emitted correct single-voxel bounds and `validationBlocks` passed in Chrome 146 | Confirmed root cause |
| Readback format or BGRA/RGBA swizzle is wrong | Render tiny offscreen primitives in `rgba8unorm` and inspect clear/background pixels | Minimal offscreen triangle and quad probes read back the expected bounds and colors | Rejected |
| Face-mask and quad object allocation are still a major meshing cost on the target runtime | Replace face-state objects with a packed integer mask, flatten quad records, and re-run the performance scenes in a fresh isolated Chrome 146 page with the new first-frame/warm-frame metrics | Confirmed on Chrome 146. Relative to the prior recorded browser baseline, mesh times fell to `terrain256 168.1 ms`, `editStorm256 173.0 ms`, `stressDrawCalls512 1.4 ms`, `stressMicroCubes256 155.4 ms`, and `stressScreens256 132.5 ms` | Confirmed |
| Terrain generation is wasting time doing one `setVoxel()` per vertical block | Add a chunk-aware `fillColumn()` path, rewrite terrain columns to use it, and compare current build timing in Bun and Chrome against the earlier recorded terrain/edit-storm baselines | Confirmed. Current build timings are down to `terrain256 36.3 ms` and `editStorm256 39.5 ms` in the warmed local Bun profiler, and `terrain256 53.6 ms` / `editStorm256 52.9 ms` in clean isolated Chrome 146 runs | Confirmed |
| One-shot Bun microbenches are a safe accept/reject oracle for renderer hot-path changes | Compare cold local probes, warmed local Bun profiles, and fresh isolated Chrome 146 reruns for the same mesher changes | Rejected. Bun/JSC is still valuable for fast local iteration, but keep/revert decisions for the engine now use Chrome 146 runs as the source of truth | Rejected as sole oracle |

## Locked-in regressions

- `tests/mesher.test.ts` now asserts that a single voxel only emits vertices inside `[1,1,1]..[2,2,2]`.
- `tests/camera.test.ts` now asserts that nearer points map to smaller clip-space depth values.
- Validation scenes now include tiny primitive cases before larger correctness scenes.
- The Bun server now serves HTML, CSS, and bundle responses with `Cache-Control: no-store` and cache-busted module URLs so browser-side probes exercise the current code by default.
