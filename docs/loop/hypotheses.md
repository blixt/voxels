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

## Locked-in regressions

- `tests/mesher.test.ts` now asserts that a single voxel only emits vertices inside `[1,1,1]..[2,2,2]`.
- `tests/camera.test.ts` now asserts that nearer points map to smaller clip-space depth values.
- Validation scenes now include tiny primitive cases before larger correctness scenes.
- The Bun server now serves HTML, CSS, and bundle responses with `Cache-Control: no-store` and cache-busted module URLs so browser-side probes exercise the current code by default.
