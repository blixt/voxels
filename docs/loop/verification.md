# Verification Log

## 2026-03-10

### Commands

- `mise install`
- `bun install`
- `mise run test`
- `bun run build`
- `mise exec -- bun test tests/mesher.test.ts tests/camera.test.ts tests/reference-render-fixtures.test.ts`
- `mise exec -- bun run check`
- `mise exec -- bun run build`

### Automated checks

- `bun test`: 15 passing tests covering scene roundtrips, greedy meshing, MagicaVoxel import, stress-scene discovery, edit raycasting, reference-render fixtures, and camera depth ordering/drag mapping.
- `tsc --noEmit`: passing.
- `bun run build`: passing.

### Chrome 146 smoke checks

- `/` loads with WebGPU active, no console errors, and renders the default 256x256x256 scene.
- Programmatic playground edit check:
  - remove voxel at a sampled screen position: solid voxel count `3,433,426 -> 3,433,425`
  - add voxel back at a sampled screen position: solid voxel count `3,433,425 -> 3,433,426`
- `/bench?auto=1&scenario=terrain256&iterations=1&frames=20` completes with correctness `pass` and no console errors.

### Browser benchmark sample

- `terrain256`: build `93.2 ms`, mesh `475.2 ms`, avg CPU frame `0.26 ms`, avg GPU frame `0.15 ms`, triangles `118,540`
- Full suite (`runAll(1, 10)`) produced correctness `pass` for:
  - `terrain256`
  - `scatter256`
  - `denseCore128`
  - `editStorm256`
- Stress suite (`runStress(1, 3)`) in fresh Chrome 146 tab:
  - `editStorm256`: build `101.6 ms`, mesh `602.2 ms`, avg CPU frame `0.63 ms`, avg GPU frame `0.42 ms`, triangles `130,928`
  - `stressDrawCalls512`: build `1.9 ms`, mesh `1709.7 ms`, avg CPU frame `0.90 ms`, avg GPU frame `0.09 ms`, draw calls `512`
  - `stressMicroCubes256`: build `7.3 ms`, mesh `1523.6 ms`, avg CPU frame `1.57 ms`, avg GPU frame `1.70 ms`, triangles `194,688`
  - `stressScreens256`: build `5.1 ms`, mesh `1297.7 ms`, avg CPU frame `0.67 ms`, avg GPU frame `0.81 ms`, triangles `27,648`

### Renderer probe checks after the visual regression investigation

- Single-voxel mesh probe before the fix:
  - voxel at `[1,1,1]` emitted mesh bounds `min=[1,1,1]`, `max=[10,10,10]`
- Single-voxel mesh probe after the fix:
  - voxel at `[1,1,1]` emits mesh bounds `min=[1,1,1]`, `max=[2,2,2]`
- Camera depth probe:
  - points stepped toward the camera now verify smaller clip-space depth than points stepped away from it
- Browser primitive probes:
  - offscreen `rgba8unorm` triangle and quad probes rendered expected centered bounds
  - packed `float32x3 + snorm8x4 + unorm8x4` vertex layout and uniform-matrix probes also rendered expected bounds
- Chrome 146 validation reruns after cache bypass:
  - browser-side single-voxel probe now emits `min=[1,1,1]`, `max=[2,2,2]`
  - `validationBlocks`: build `0.0 ms`, mesh `1.8 ms`, avg CPU frame `0.16 ms`, avg GPU frame `0.05 ms`
  - `validationBlocks` image validation: `MAE 1.63`, `max error 13`, `coverage mismatch 0.00%`, correctness `pass`

### Harness hardening

- The Bun server now serves HTML, CSS, and `/build/*` responses with `Cache-Control: no-store`.
- HTML now appends a cache-busting query string to module bundle URLs so page reloads fetch the latest client code by default.
- `/bench` now exposes a dedicated stress-suite path:
  - UI button: `Run Stress Suite`
  - URL automation: `/bench?auto=1&suite=stress&iterations=1&frames=30`
  - browser API: `window.__VOXELS_BENCH__.runStress(iterations, frameCount)`

### Camera interaction probe

- Synthetic pointer drag on the benchmark canvas after the orbit change:
  - drag down by `40px`: `pitch -0.61547 -> -0.93547`
  - the more-negative pitch confirms that downward drags now tilt the scene upward rather than lowering the camera
