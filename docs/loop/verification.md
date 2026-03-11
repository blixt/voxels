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
- `mise exec -- bun run versions`
- `mise exec -- bun --eval 'import { buildChunkMesh, rebuildDirtyMeshes } from "./src/engine/mesher.ts"; ...'`
- `mise exec -- bun <temporary A/B mesher harness comparing the working tree against HEAD on identical scene builds>`
- `mise exec -- bun <temporary A/B scene-build harness comparing the working tree against HEAD on identical scene builds>`

### Automated checks

- `bun test`: 21 passing tests covering scene roundtrips, greedy meshing, MagicaVoxel import, stress-scene discovery, edit raycasting, reference-render fixtures, camera depth ordering/drag mapping, bulk column writes, and benchmark-metric helpers.
- `tsc --noEmit`: passing.
- `bun run build`: passing.
- `bun run versions`: passing, with all tracked project/global tool versions matching upstream latest releases.

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

### Meshing optimization probe

- `bun run check`: passing after adding chunk-local voxel sampling and the adjacent-chunk hidden-face regression test.
- `bun run build`: passing.
- Local mesher microbench after the chunk-local sampling change:
  - sparse `2x2x2` chunk: average `0.0165 ms`
  - dense `32x32x32` chunk: average `1.79 ms`
- Local scene mesh timings after the change:
  - `terrain256`: mesh `518.5 ms`
  - `editStorm256`: mesh `351.8 ms`
  - `stressDrawCalls512`: mesh `2.0 ms`
  - `stressMicroCubes256`: mesh `320.4 ms`
  - `stressScreens256`: mesh `268.2 ms`
  - `denseCore128`: mesh `120.9 ms`
- Fresh Chrome 146 `/bench` stress run (`runStress(1, 5)`) after the change:
  - `editStorm256`: build `137.7 ms`, mesh `309.7 ms`, avg CPU frame `0.48 ms`, avg GPU frame `0.08 ms`
  - `stressDrawCalls512`: build `2.9 ms`, mesh `2.2 ms`, avg CPU frame `0.90 ms`, avg GPU frame `0.05 ms`
  - `stressMicroCubes256`: build `12.3 ms`, mesh `268.9 ms`, avg CPU frame `1.40 ms`, avg GPU frame `0.16 ms`
  - `stressScreens256`: build `9.3 ms`, mesh `246.8 ms`, avg CPU frame `1.00 ms`, avg GPU frame `0.04 ms`

### Meshing allocation rewrite probe

- `bun run check`: passing after flattening face masks and quad records.
- `bun run build`: passing.
- Current warmed local Bun profile (`mise run profile -- --iterations=3 --warmup=1 ...`) on the active branch:
  - `terrain256`: build avg `36.3 ms`, mesh avg `155.1 ms`
  - `editStorm256`: build avg `39.5 ms`, mesh avg `233.8 ms`
  - `stressDrawCalls512`: build avg `1.84 ms`, mesh avg `2.38 ms`
  - `stressMicroCubes256`: build avg `4.74 ms`, mesh avg `850.1 ms`
  - `stressScreens256`: build avg `3.13 ms`, mesh avg `279.2 ms`
  - `denseCore128`: build avg `62.0 ms`, mesh avg `61.1 ms`
- Temporary direct `HEAD` comparison in a detached worktree showed that Bun/JSC does not rank the mesher changes the same way Chrome/V8 does, so Bun-only numbers are now treated as a screening tool rather than the final oracle.

### Scene build rewrite probe

- `bun run check`: passing after adding `fillColumn()` and the cross-chunk column tests.
- `bun run build`: passing.
- Current warmed local Bun profile:
  - `terrain256`: build avg `36.3 ms`
  - `editStorm256`: build avg `39.5 ms`

### Harness metrics probe

- `tests/benchmark-metrics.test.ts`: passing.
- `/bench` result rows now expose:
  - first-frame CPU/GPU cost
  - warm-frame CPU/GPU cost
  - first-frame sync/upload/encode cost
  - first-frame upload chunk count and upload bytes

### Local profile script probe

- `mise run profile -- --iterations=3 --warmup=1 terrain256 stressDrawCalls512`: passing.
- Sample output:
  - `terrain256`: build avg `36.3 ms`, mesh avg `155.1 ms`, triangles `100,062`, chunks `136`
  - `stressDrawCalls512`: build avg `1.84 ms`, mesh avg `2.38 ms`, triangles `6,144`, chunks `512`

### Clean Chrome 146 rerun after the current performance branch

- Fresh isolated Chrome 146 `/bench` page with no overlapping benchmark runs:
  - `terrain256`: build `53.6 ms`, mesh `168.1 ms`, first CPU `1.10 ms`, warm CPU `0.05 ms`, first upload `0.90 ms`, first upload chunks `136`, first upload bytes `5.20 MB`
  - `editStorm256`: build `52.9 ms`, mesh `173.0 ms`, first CPU `1.00 ms`, warm CPU `0.15 ms`, first upload `0.90 ms`, first upload chunks `172`, first upload bytes `5.86 MB`
  - `stressDrawCalls512`: build `1.8 ms`, mesh `1.4 ms`, first CPU `2.90 ms`, warm CPU `0.20 ms`, first upload `2.50 ms`, first upload chunks `512`, first upload bytes `0.30 MB`
  - `stressMicroCubes256`: build `6.8 ms`, mesh `155.4 ms`, first CPU `3.90 ms`, warm CPU `0.25 ms`, first upload `3.60 ms`, first upload chunks `448`, first upload bytes `9.66 MB`
  - `stressScreens256`: build `4.1 ms`, mesh `132.5 ms`, first CPU `2.00 ms`, warm CPU `0.15 ms`, first upload `1.60 ms`, first upload chunks `392`, first upload bytes `1.14 MB`
- Current validation guardrail in the same isolated Chrome 146 context:
  - `validationBlocks`: build `0.1 ms`, mesh `0.3 ms`, correctness `pass`, visual `pass`, `MAE 1.63`, `coverage mismatch 0.00%`

## 2026-03-11

### Docs pivot verification

- Updated only repository documentation and planning files:
  - `README.md`
  - `docs/roadmap.md`
  - `docs/loop/plan.md`
  - `docs/loop/research.md`
  - `docs/loop/progress.md`
  - `docs/loop/README.md`
  - `docs/agent-playbook.md`
- No code paths or runtime assets changed in this step.
- No automated checks were run for this docs-only pivot.

### First game slice verification

#### Commands

- `mise run test`
- `mise run build`
- `PORT=3001 mise exec -- bun run src/server.ts`

#### Automated checks

- `mise run test`: passing after adding the first-person camera module and tests.
- `mise run build`: passing after replacing the old playground entry with the game entry.

#### Chrome 146 browser checks

- `/` on `http://localhost:3001/` now loads as a full-screen game shell with:
  - game canvas
  - HUD/status panels
  - crosshair
  - click-to-capture overlay
- Browser-exposed game automation surface is present:
  - `window.__VOXELS_GAME__.snapshot()`
  - `window.__VOXELS_GAME__.teleport(x, y, z)`
  - `window.__VOXELS_GAME__.requestPointerLock()`
- DevTools automation can verify the failure path for Pointer Lock, but not grant it:
  - synthetic click on the capture overlay changes status to `Pointer lock request was blocked`
  - no console errors are emitted
- Browser-side game debug probe:
  - initial snapshot position: `[128.5, 55.0, 128.5]`
  - teleport probe result: `[140, 70, 90]`
- `/bench?auto=1&scenario=validationBlocks&iterations=1&frames=3` still completes successfully after the renderer change:
  - `validationBlocks`: build `0.1 ms`, mesh `0.3 ms`
  - first CPU `0.90 ms`, warm CPU `0.10 ms`
  - first GPU `0.07 ms`, warm GPU `0.72 ms`
  - `MAE 1.63`, coverage mismatch `0.00%`
  - visual `pass`, correctness `pass`

### Resident-chunk interface seam verification

#### Commands

- `mise run test`
- `mise run build`
- `mise run versions`

#### Automated checks

- `mise run test`: passing after introducing `ResidentChunkWorld` and the new `tests/world-access.test.ts`.
- `mise run build`: passing after the mesher/renderer refactor.
- `mise run versions`: passing, with all tracked tools still up to date:
  - Chrome stable `146.0.7680.72`
  - Bun `1.3.10`
  - `@types/bun` `1.3.10`
  - TypeScript `5.9.3`
  - `@webgpu/types` `0.1.69`
  - mise `2026.3.7`

#### Chrome 146 browser checks

- Reloaded `/bench?auto=1&scenario=validationBlocks&iterations=1&frames=3` after the interface refactor.
- `validationBlocks` still passes:
  - build `0.1 ms`
  - mesh `0.3 ms`
  - first CPU `1.00 ms`
  - warm CPU `0.20 ms`
  - first GPU `0.07 ms`
  - warm GPU `0.20 ms`
  - `MAE 1.63`
  - coverage mismatch `0.00%`
  - visual `pass`
  - correctness `pass`

### Procedural generator primitive verification

#### Commands

- `mise run test`
- `mise run build`

#### Automated checks

- `mise run test`: passing after adding `tests/procedural-generator.test.ts`.
- `mise run build`: passing after adding `src/engine/procedural-generator.ts` and `src/engine/noise.ts`.

#### Generator-specific checks

- `#RGB` palette mapping:
  - palette length `4097`
  - `#ABC -> material index -> #ABC` roundtrip passes
- deterministic chunk generation:
  - generating chunk `(18, 4, -7)` twice under seed `4242` yields identical `solidCount` and identical `fnv1a` checksum
- direct sampler consistency:
  - sampled world voxels match generated chunk-local data at multiple coordinates inside a generated chunk
- biome distribution:
  - the wide coordinate grid probe now yields at least `4` biome families
- Y-range guard:
  - `sampleMaterial(..., -1, ...)` returns `0`
  - `sampleMaterial(..., 16384, ...)` returns `0`

### Procedural resident-world game-path verification

#### Commands

- `mise run test`
- `mise run build`
- `mise run versions`

#### Automated checks

- `mise run test`: passing after adding `tests/procedural-resident-world.test.ts`.
- `mise run build`: passing after wiring the game path onto `ProceduralResidentWorld`.
- `mise run versions`: passing, with all tracked tools still current:
  - Chrome stable `146.0.7680.72`
  - Bun `1.3.10`
  - `@types/bun` `1.3.10`
  - TypeScript `5.9.3`
  - `@webgpu/types` `0.1.69`
  - mise `2026.3.7`

#### Chrome 146 browser checks

- Verified `/` against `http://localhost:3001/` because `http://localhost:3000/` was occupied by another Bun server that still served the old playground shell.
- `/` now loads as `Voxels Game` and exposes `window.__VOXELS_GAME__`.
- Browser-side snapshot probe on the procedural resident world:
  - position `[-191.5, 1661.0, -191.5]`
  - player chunk `[-6, 51, -6]`
  - resident chunks `124`
  - radius `2 chunks`
  - surface Y `1653`
  - stream `273.9 ms`
  - generated `0`
  - evicted `115`
  - empty skipped `26`
  - mesh `69.3 ms`
  - draw calls `118`
  - triangles `40,608`
- Game API residency probe:
  - `setViewDistance(3)` grows resident chunks `124 -> 239`
  - the widen step generated `115` chunks in about `1943.0 ms`
  - `setViewDistance(2)` then shrank resident chunks `239 -> 124`
  - the shrink step evicted `115` chunks in about `274.4 ms`
- `/bench?auto=1&scenario=validationBlocks&iterations=1&frames=3` still passes after the game-path change:
  - build `0.1 ms`
  - mesh `0.3 ms`
  - first CPU `0.60 ms`
  - warm CPU `0.00 ms`
  - first GPU `0.07 ms`
  - warm GPU `0.29 ms`
  - `MAE 1.63`
  - coverage mismatch `0.00%`
  - visual `pass`
  - correctness `pass`

#### Startup/streaming hotspot screen

- Local hotspot ranking from the resident-world path:
  - `sampleColumn()` `5000` calls: about `5.7 ms`
  - `generateChunk()` for full terrain chunks: about `14-17 ms` each
  - initial `updateResidencyAround(spawn)`: about `3683 ms` for `239` generated chunks
  - `rebuildDirtyMeshes()` over that resident set: about `256 ms`
- Current conclusion:
  - startup is generation-dominated first, meshing second
  - the next work should instrument chunk-generation and residency phases, then remove duplicate column sampling before deeper architectural changes

### Machine-readable probe verification

#### Commands

- `mise run test`
- `mise run build`

#### Automated checks

- `mise run test`: passing after adding `tests/procedural-probes.test.ts` and extending the resident-world tests.
- `mise run build`: passing after exposing new game and benchmark probe APIs.

#### Chrome 146 browser checks

- Reloaded `http://localhost:3001/` and verified the new game probe surface:
  - `window.__VOXELS_GAME__.snapshotResidentWorld()` returned `239` resident chunks at radius `3`
  - sample resident chunk probe:
    - coord `(-9, 47, -6)`
    - solid count `32,768`
    - checksum `3eff41cf`
    - solid bounds `min=[-288,1504,-192]`, `max=[-256,1536,-160]`
  - `window.__VOXELS_GAME__.teleportAndSettle(-191.5, 1661, -191.5, { radiusChunks: 2 })` returned:
    - resident chunks `239 -> 124`
    - entered `0`
    - evicted `115`
    - generated `0`
    - stream `275.8 ms`
    - radius `2`
- Reloaded `http://localhost:3001/bench?auto=1&scenario=validationBlocks&iterations=1&frames=3` and verified the new benchmark probe surface:
  - `window.__VOXELS_BENCH__.probeGeneration({ seed: 1337, chunkSize: 16, chunkCoords: [[0,87,0],[1,87,0]] })` returned deterministic chunk summaries including:
    - chunk `(0,87,0)`: checksum `2617b1c5`, solid count `4096`, center biome `tundra`, center surface `2162`
    - chunk `(1,87,0)`: checksum `4127ddc5`, solid count `4096`, center biome `tundra`, center surface `2172`
- `validationBlocks` remained green after the probe additions:
  - build `0.1 ms`
  - mesh `0.3 ms`
  - first CPU `0.70 ms`
  - warm CPU `0.20 ms`
  - first GPU `0.04 ms`
  - warm GPU `0.07 ms`
  - `MAE 1.63`
  - coverage mismatch `0.00%`
  - visual `pass`
  - correctness `pass`
