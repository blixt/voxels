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

### 10 cm scale and step-up verification

#### Commands

- `mise exec -- bun test tests/player-physics.test.ts`
- `mise run test`
- `mise run build`

#### Automated checks

- Focused player-physics file: passing after rescaling the player/body constants and adding `0.3 m` auto-step coverage.
- Full test suite: passing after updating the spawn-footprint test to match the new physical scale.
- Production build: passing.

#### Chrome 146 browser checks on `http://localhost:3015/`

- `/` HUD now reports gameplay position in meters instead of raw voxel units:
  - sample snapshot showed `Position -19.2m, 144.5m, -19.2m`
  - sample snapshot showed `Feet -19.2m, 142.8m, -19.2m`
  - sample snapshot showed `Surface Y 143.3 m`
- Browser snapshot after the hot reload confirms the new scaled player body:
  - eye-to-feet difference is `16.8` voxel units, matching `1.68 m`
  - the game remained grounded after reload with no runtime errors

### Bun full-stack dev/prod environment verification

#### Commands

- `mise run test`
- `mise run build`
- `PORT=3012 mise run dev`
- `PORT=3014 mise run serve`

#### Automated checks

- `mise run test`: passing after the HTML-import server switch, HMR-safe client bootstrap refactor, and production-start wrapper.
- `mise run build`: passing, producing:
  - `dist/server.js`
  - `dist/pages/game.html`
  - `dist/pages/bench.html`
  - shared hashed CSS/JS chunks

#### Chrome 146 dev-server checks

- `/` on `http://localhost:3012/` now loads through Bun's full-stack dev server with:
  - `GET /`
  - `GET /_bun/asset/<hash>.css`
  - `GET /_bun/client/game-<hash>.js`
- `/bench` on `http://localhost:3012/bench` now loads through the same Bun-managed asset graph with:
  - `GET /bench`
  - `GET /_bun/asset/<hash>.css`
  - `GET /_bun/client/bench-<hash>.js`
- Verified live HTML editing without restarting the server:
  - temporary change to `src/pages/game.html` updated the subtitle on the open `/` page
- Verified live TypeScript editing without restarting the server:
  - temporary change to `src/client/game.ts` updated the on-screen `Avg Frame CPU` metric label on the open `/` page
- Verified browser console forwarding through Bun development mode:
  - `console.log("dev-console-probe-stable")` emitted `[browser] dev-console-probe-stable` in the dev-server terminal
- Found and fixed one HMR issue during implementation:
  - self-accepting async entry modules triggered Bun runtime `TypeError: K.onDispose is not iterable`
  - refactoring `game.ts` and `bench.ts` to stay synchronous at module level removed the error

#### Chrome 146 production-bundle checks

- `/` on `http://localhost:3014/` loads successfully from the built bundle with:
  - `GET /`
  - `GET /chunk-<hash>.css`
  - `GET /chunk-<hash>.js`
- `/bench` on `http://localhost:3014/bench` loads successfully from the built bundle with:
  - `GET /bench`
  - `GET /chunk-<hash>.css`
  - `GET /chunk-<hash>.js`
- Found and fixed one production-start issue during implementation:
  - starting `dist/server.js` from repo root failed with `Bundled file "./chunk-....js" not found`
  - starting the bundle from inside `dist/` via `scripts/start.ts` fixed asset resolution
- Found and fixed one production-mode issue during implementation:
  - the first built server accidentally inlined development mode because `process.env.NODE_ENV` was read too statically
  - switching to a runtime-bound env read in `src/server.ts` preserved the correct production/dev split

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

### Known-empty chunk cache verification

#### Commands

- `mise run test`
- `mise run build`
- `mise run profile-stream -- --iterations=3 --warmup=1`

#### Automated checks

- `mise run test`: passing after adding resident-world cache regression coverage.
- `mise run build`: passing after exposing cached-empty metrics through the game HUD/debug surface and the local stream profiler.

#### Warmed local stream-profiler results

- `bootstrap-r3`:
  - stream avg `203.8 ms`
  - mesh avg `179.4 ms`
  - generated chunks `239`
  - empty chunks skipped `58`
  - cached empty hits `0`
- `widen-r2-to-r3`:
  - stream avg `103.8 ms`
  - mesh avg `146.2 ms`
  - generated chunks `115`
  - empty chunks skipped `32 -> 0`
  - cached empty hits `26`
  - chunk generation avg `102.6 ms`
- `shrink-r3-to-r2`:
  - stream avg `0.25 ms`
  - mesh avg `58.1 ms`
  - generated chunks `0`
  - evicted chunks `115`
  - cached empty hits `26`
  - chunk generation avg `0.0 ms`

#### Direct local probe checks

- `mise exec -- bun -e 'import { ProceduralResidentWorld } ...'` against seed `1337`:
  - bootstrap `r3`: generated `239`, empty skipped `58`, cached empty hits `0`, chunk generation `209.0 ms`, total stream `211.5 ms`
  - same-anchor refresh at `r3`: generated `0`, empty skipped `0`, cached empty hits `58`, chunk generation `0 ms`
  - shrink `r3 -> r2`: generated `0`, evicted `115`, cached empty hits `26`, chunk generation `0 ms`, total stream `0.36 ms`
  - widen `r2 -> r3`: generated `115`, evicted `0`, cached empty hits `26`, empty skipped `0`, chunk generation `81.0 ms`, total stream `81.9 ms`

#### Chrome 146 browser checks on `http://localhost:3001/`

- Reloaded `/` with cache bypass and queried `window.__VOXELS_GAME__`.
- Same-anchor forced refresh now reports:
  - generated chunks `0`
  - empty chunks skipped `0`
  - cached empty hits `58`
  - chunk generation `0 ms`
  - total stream time `0.2 ms`
- Radius shrink `r3 -> r2` now reports:
  - generated chunks `0`
  - evicted chunks `115`
  - cached empty hits `26`
  - chunk generation `0 ms`
  - total stream time `0.3 ms`
- Radius widen `r2 -> r3` now reports:
  - generated chunks `115`
  - cached empty hits `58`
  - empty chunks skipped `0`
  - chunk generation `76.5 ms`
  - total stream time `76.7 ms`
- Post-widen game snapshot confirms the new HUD/debug field is live:
  - `streamCachedEmptyChunkHits = 58`
  - `streamGeneratedChunks = 115`
  - `streamDirtyResidentChunks = 179`

### Fully occluded solid chunk meshing verification

This line of investigation was screened locally and not kept in the runtime yet.

#### Direct local checks

- resident-world analysis on the current procedural spawn:
  - bootstrap `r3`: `18` fully solid resident chunks had all six neighbors and already produced zero-triangle meshes
  - widen `r2 -> r3`: `12` dirty chunks matched the same fully occluded-solid pattern
- face-aware neighbor invalidation was also screened and rejected for the same spawn:
  - widen would still touch the same `179` dirty resident chunks
  - shrink would still remesh the same `64` resident chunks
- No code from this screening pass is currently kept in the runtime because it was not yet verified strongly enough in Chrome to justify the added complexity.

### Feedback grounding verification

#### Commands

- `mise exec -- bun -e 'import { ProceduralResidentWorld } ...'`
- `mise exec -- bun -e 'import { ProceduralWorldGenerator } ...'`

#### Direct local checks

- Default scale probe:
  - chunk size `32`
  - default radius `3`
  - horizontal residency distance `96 cm`
  - spawn eye height above surface `8 cm`
- Radius sweep at the current chunk size:
  - radius `3`: `96 cm`, `239` resident chunks, about `212 ms` stream and `267 ms` mesh
  - radius `4`: `128 cm`, `382` resident chunks, about `322 ms` stream and `251 ms` mesh
  - radius `5`: `160 cm`, `587` resident chunks, about `501 ms` stream and `379 ms` mesh
  - radius `6`: `192 cm`, `790` resident chunks, about `682 ms` stream and `493 ms` mesh
- Chunk-size sweep at radius `3`:
  - `32^3` chunks: `96 cm`, `239` resident chunks, about `270 ms` stream and `265 ms` mesh
  - `64^3` chunks: `192 cm`, `191` resident chunks, about `982 ms` stream and `875 ms` mesh
  - `128^3` chunks: `384 cm`, `177` resident chunks, about `5925 ms` stream and `6509 ms` mesh

#### Conclusions

- The current draw-distance problem is not fixable by a small radius bump.
- The current scale problem is not only visual; the player camera is physically too close to the ground for a `1 cm` voxel world.
- Naively increasing chunk size is not a viable near-term fix with the current synchronous generation + meshing architecture.

### Grounded player slice verification

#### Commands

- `mise run test`
- `mise run build`

#### Automated checks

- `mise run test`: passing after adding `tests/player-physics.test.ts`.
- `mise run build`: passing after switching the game runtime over to a grounded player body.

#### Direct local checks

- `tests/player-physics.test.ts` now verifies:
  - falling onto flat ground settles the player at `feetY = 1` and sets `grounded = true`
  - a solid wall blocks forward movement at the expected body radius
  - jumping only works from the grounded state
  - eye position is derived from feet position plus `168 cm`
- `tests/procedural-resident-world.test.ts` now also verifies:
  - spawn selection keeps the sampled `32 cm` standing footprint within `12 cm` of vertical spread
  - spawn `y` is set to the highest sampled footprint voxel plus `1`
- Direct spawn/body probe on the default procedural world:
  - spawn feet position `[-831.5, 1614, -831.5]`
  - sampled surface `1609`
  - derived eye position `[-831.5, 1782, -831.5]`
  - eye height `168 cm`

#### Chrome 146 browser checks

- Verified `/` on a fresh server port at `http://localhost:3005/` to avoid stale long-lived Bun processes.
- Fresh page telemetry after load:
  - position `[-831.5, 1783.0, -831.5]`
  - feet `[-831.5, 1615.0, -831.5]`
  - grounded `Yes`
  - surface `1609`
- The loaded page has already run its first idle physics step, so the HUD reflects the post-settle body position rather than the raw `getSpawnPosition()` sample.
- Scripted idle-settle probe through `window.__VOXELS_GAME__.controller`:
  - forced `pointerLocked = false`
  - reset the player to `world.getSpawnPosition()`
  - cleared velocity and grounded state
  - ran five `updateMovement(1 / 60)` steps
  - final state:
    - `feetY = 1615`
    - `grounded = true`
    - `eyeMinusFeet = 168`
    - `feetMinusSurface = 6`
- Synthetic click on `CLICK TO ENTER THE WORLD` still reports `Pointer lock request was blocked` rather than throwing, which preserves the earlier automation-safe behavior.

### Procedural stream optimization verification

- warmed local `profile-stream` after the early-out:
  - `bootstrap-r3` mesh avg `162.1 ms`
  - `widen-r2-to-r3` mesh avg `174.9 ms`
  - `shrink-r3-to-r2` mesh avg `55.0 ms`
- local multi-scene profile remained roughly flat outside the procedural stream path:
  - `terrain256`
  - `denseCore128`
  - `stressMicroCubes256`

#### Chrome 146 worktree A/B on `http://localhost:3002/` vs `http://localhost:3001/`

- Baseline committed worktree (`9e9c9d5`) on `:3002`:
  - average `shrink-r3-to-r2` mesh `79.3 ms`
  - average `widen-r2-to-r3` mesh `175.3 ms`
- Current working tree on `:3001`:
  - average `shrink-r3-to-r2` mesh `69.4 ms`
  - average `widen-r2-to-r3` mesh `162.0 ms`
- Both pages used the same scripted `window.__VOXELS_GAME__.teleportAndSettle(...)` loop over three shrink/widen cycles after a cache-bypass reload.

### Game-path hitch probe verification

#### Commands

- `mise run test`
- `mise run build`

#### Chrome 146 browser checks

- Verified `/` on a fresh server port at `http://localhost:3006/`.
- `window.__VOXELS_GAME__.benchmarkChunkCrossing(2, 1)` now returns compact benchmark samples plus summary data instead of dumping full resident-world snapshots.
- The benchmark restores the live player state after running:
  - `before.position == after.position`
  - `before.feetPosition == after.feetPosition`
  - `before.grounded == after.grounded`
- Baseline result before stream-anchor hysteresis:
  - `sampleCount = 4`
  - `changedCount = 4`
  - average `streamMs = 48.15`
  - average `meshMs = 194.55`
  - average `uploadMs = 0.73`
  - average `uploadChunks = 99.5`
- First one-chunk-cross sample:
  - target chunk `[-25, 50, -26]`
  - `generatedChunks = 42`
  - `evictedChunks = 43`
  - `streamMs = 69`
  - `meshMs = 242.5`
  - `uploadMs = 0.8`

#### Conclusion

- The current hitch is not primarily GPU upload.
- The current game path is still paying full residency + remesh cost on every one-chunk boundary crossing.

### Stream-anchor hysteresis verification

#### Commands

- `mise run test`
- `mise run build`

#### Automated checks

- `tests/stream-anchor.test.ts` verifies:
  - anchor initialization
  - staying within a one-chunk margin
  - shifting once the player exceeds that margin
  - chunk-center anchor positioning

#### Chrome 146 browser checks

- Verified `/` on a fresh page at `http://localhost:3006/`.
- Fresh idle HUD now reports:
  - `Stream = 0.0 ms`
  - `Mesh = 0.0 ms`
  - `Dirty Resident = 0`
  - `Stream Anchor = -26, -26`
- `window.__VOXELS_GAME__.benchmarkChunkCrossing(2, 1)` after the hysteresis change:
  - `sampleCount = 4`
  - `changedCount = 0`
  - average `streamMs = 0`
  - average `meshMs = 0`
  - average `uploadChunks = 0`
  - first sample:
    - target chunk `[-25, 50, -26]`
    - `changed = false`
    - `generatedChunks = 0`
    - `evictedChunks = 0`
- `window.__VOXELS_GAME__.benchmarkChunkCrossing(2, 2)` still reports the larger churn path:
  - `sampleCount = 4`
  - `changedCount = 4`
  - average `streamMs = 81.0`
  - average `meshMs = 179.93`
  - average `uploadChunks = 99`

#### Conclusion

- The one-chunk hitch path is now removed.
- The next hitch target is the still-expensive two-chunk update path rather than the already-fixed single-chunk churn case.

### Terrain envelope retune verification

#### Commands

- `mise run test`
- `mise run build`

#### Automated checks

- `tests/procedural-generator.test.ts` now verifies:
  - the early terrain envelope stays near sea level
  - the sampled world exposes visible lowlands and water coverage
  - adjacent-column steps stay within a walkable budget
  - biome-edge height jumps stay within a starter-world budget

#### Direct local checks

- Fixed-grid terrain probe on `seed = 1337`:
  - `minSurfaceY = 1298`
  - `maxSurfaceY = 1555`
  - `avgSurfaceY = 1424.58`
  - `underwaterRatio = 0.254`
  - `maxAdjacentStep = 30`
  - `maxBoundaryJump = 30`

#### Chrome 146 browser checks

- Fresh `/` boot on `http://localhost:3006/` now reports:
  - position `[-255.5, 1588.0, -255.5]`
  - feet `[-255.5, 1420.0, -255.5]`
  - surface `1418`
  - resident chunks `119`
  - triangles `15,862`
- The streamed world now boots near sea level instead of in the old `1609+` height range.

#### Conclusion

- The terrain now reads as a much flatter, lower, more traversable starter world.
- The next remaining visual problem is true far-field draw distance, not the vertical terrain envelope.

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

### Procedural stream profiler baseline

#### Commands

- `mise run test`
- `mise run build`
- `mise run profile-stream -- --iterations=2 --warmup=1`

#### Automated checks

- `mise run test`: passing after adding `scripts/profile-procedural-stream.ts`.
- `mise run build`: passing after exposing the new profile task.

#### Local profiler samples

- `bootstrap-r3`:
  - stream avg `3673.1 ms`
  - mesh avg `188.2 ms`
  - generated chunks `239`
  - resident chunks `239`
- `widen-r2-to-r3`:
  - stream avg `2113.3 ms`
  - mesh avg `147.0 ms`
  - generated chunks `115`
  - resident chunks `239`
- `shrink-r3-to-r2`:
  - stream avg `307.5 ms`
  - mesh avg `57.7 ms`
  - evicted chunks `115`
  - resident chunks `124`

### Procedural generator hot-path rewrite

#### Commands

- `mise run test`
- `mise run build`
- `mise run profile-stream -- --iterations=2 --warmup=1`

#### Automated checks

- `mise run test`: passing after the generator rewrite and the added solid-bounds assertions.
- `mise run build`: passing after the chunk-generation changes.

#### Local profiler comparison

- `bootstrap-r3`:
  - stream avg `3673.1 ms -> 202.2 ms`
  - mesh avg `188.2 ms -> 179.7 ms`
- `widen-r2-to-r3`:
  - stream avg `2113.3 ms -> 111.5 ms`
  - mesh avg `147.0 ms -> 140.2 ms`
- `shrink-r3-to-r2`:
  - stream avg `307.5 ms -> 13.7 ms`
  - mesh avg `57.7 ms -> 55.4 ms`

#### Chrome 146 browser checks

- Reloaded `http://localhost:3001/` and re-ran the procedural game probes after the generator rewrite.
- Fresh bootstrap snapshot on `/`:
  - resident chunks `239`
  - radius `3 chunks`
  - stream `206.5 ms`
  - generated `239`
  - empty skipped `58`
  - mesh `255.7 ms`
- `window.__VOXELS_GAME__.teleportAndSettle(-191.5, 1661, -191.5, { radiusChunks: 2 })` returned:
  - resident chunks `239 -> 124`
  - stream `13.0 ms`
  - evicted `115`
  - generated `0`
- `window.__VOXELS_GAME__.teleportAndSettle(-191.5, 1661, -191.5, { radiusChunks: 3 })` returned:
  - resident chunks `124 -> 239`
  - stream `104.6 ms`
  - evicted `0`
  - generated `115`
- Reloaded `/bench?auto=1&scenario=validationBlocks&iterations=1&frames=3` after the rewrite and confirmed the shared engine guardrail stayed green:
  - build `0.1 ms`
  - mesh `0.2 ms`
  - first CPU `0.80 ms`
  - warm CPU `0.20 ms`
  - first GPU `0.09 ms`
  - warm GPU `0.00 ms`
  - `MAE 1.63`
  - coverage mismatch `0.00%`
  - visual `pass`
  - correctness `pass`

### Residency phase and remesh instrumentation

#### Commands

- `mise run test`
- `mise run build`
- `mise run profile-stream -- --iterations=2 --warmup=1`

#### Automated checks

- `mise run test`: passing after adding dirty-resident phase metrics and new-vs-remesh mesh accounting.
- `mise run build`: passing after exposing the new metrics through the game route.

#### Local profiler samples

- `bootstrap-r3`:
  - stream avg `204.6 ms`
  - mesh avg `180.7 ms`
  - mesh new chunks `239`
  - mesh remesh chunks `0`
  - dirty resident chunks `239`
  - residency phases:
    - chunk generation `202.9 ms`
    - neighbor dirty `0.67 ms`
- `widen-r2-to-r3`:
  - stream avg `117.0 ms`
  - mesh avg `145.0 ms`
  - mesh new chunks `115`
  - mesh remesh chunks `64`
  - dirty resident chunks `179`
  - residency phases:
    - chunk generation `115.9 ms`
    - neighbor dirty `0.31 ms`
- `shrink-r3-to-r2`:
  - stream avg `14.0 ms`
  - mesh avg `55.7 ms`
  - mesh new chunks `0`
  - mesh remesh chunks `64`
  - dirty resident chunks `64`
  - residency phases:
    - chunk generation `13.7 ms`
    - eviction `0.04 ms`
    - neighbor dirty `0.09 ms`

#### Chrome 146 browser checks

- Reloaded `http://localhost:3001/` and verified the new HUD/debug metrics on the live game route:
  - fresh bootstrap snapshot:
    - dirty resident chunks `239`
    - mesh new chunks `239`
    - mesh remesh chunks `0`
    - stream `202.4 ms`
    - mesh `262.6 ms`
- `window.__VOXELS_GAME__.teleportAndSettle(-191.5, 1661, -191.5, { radiusChunks: 2 })` returned:
  - dirty resident chunks `64`
  - mesh count `64`
  - new mesh chunks `0`
  - remesh chunks `64`
  - stream `12.6 ms`
  - phases:
    - chunk generation `12.4 ms`
    - eviction `0.1 ms`
- `window.__VOXELS_GAME__.teleportAndSettle(-191.5, 1661, -191.5, { radiusChunks: 3 })` returned:
  - dirty resident chunks `179`
  - mesh count `179`
  - new mesh chunks `115`
  - remesh chunks `64`
  - stream `103.9 ms`
  - phases:
    - chunk generation `103.4 ms`
    - chunk adoption `0.2 ms`
    - neighbor dirty `0.3 ms`
- Conclusion from the instrumented run:
  - duplicate dirty bookkeeping is not the current bottleneck
  - the remaining synchronous cost is split between real boundary remesh work and repeated empty-chunk generation checks

### Residency phase instrumentation

#### Commands

- `mise run test`
- `mise run build`
- `mise run profile-stream -- --iterations=2 --warmup=1`

#### Automated checks

- `mise run test`: passing after adding phase metrics and dirty resident-chunk counts to the residency summary.
- `mise run build`: passing after extending the stream profiler output.

#### Local profiler samples

- `bootstrap-r3`:
  - stream avg `202.0 ms`
  - dirty resident chunks `239`
  - phase breakdown:
    - surface sample `0.00 ms`
    - Y-range `0.28 ms`
    - chunk generation `200.37 ms`
    - chunk adoption `0.31 ms`
    - neighbor dirty `0.61 ms`
- `widen-r2-to-r3`:
  - stream avg `111.8 ms`
  - generated chunks `115`
  - dirty resident chunks `179`
  - phase breakdown:
    - surface sample `0.01 ms`
    - Y-range `0.19 ms`
    - chunk generation `110.95 ms`
    - chunk adoption `0.12 ms`
    - neighbor dirty `0.27 ms`
- `shrink-r3-to-r2`:
  - stream avg `13.6 ms`
  - evicted chunks `115`
  - dirty resident chunks `64`
  - phase breakdown:
    - surface sample `0.01 ms`
    - Y-range `0.05 ms`
    - chunk generation `13.34 ms`
    - eviction `0.04 ms`
    - neighbor dirty `0.08 ms`

#### Current conclusion

- Neighbor-dirty bookkeeping is measurable now, and it is not the main bottleneck.
- The next win is unlikely to come from batching `markAdjacentChunksDirty()` alone, because the bookkeeping itself is sub-millisecond.
- The meaningful remaining cost is real chunk generation plus real boundary remeshing, so the next investigation should target either boundary remesh cost or incremental/worker streaming.

#### Browser note

- I did not complete a fresh post-instrumentation Chrome DevTools probe because the DevTools browser session entered a profile-conflict state during this slice.
- The runtime server is still live on `http://localhost:3001/`, and the browser-facing functional guardrail remains the earlier post-optimization Chrome 146 run recorded above.

### Fully occluded solid-chunk mesh skip verification

#### Commands

- `mise run test`
- `mise run build`
- `mise run profile-stream -- --iterations=3 --warmup=1`
- `mise run profile -- --iterations=3 --warmup=1 terrain256 denseCore128 stressMicroCubes256`

#### Automated checks

- `mise run test`: passing after adding mesher regressions for:
  - fully buried solid chunk culling
  - single neighbor-face hole still exposing the center chunk
- `mise run build`: passing after the mesher fast path cleanup.

#### Warmed local stream-profiler results

- `bootstrap-r3`:
  - stream avg `199.8 ms`
  - mesh avg `160.6 ms`
  - resident chunks `239`
  - generated chunks `239`
- `widen-r2-to-r3`:
  - stream avg `101.3 ms`
  - mesh avg `133.9 ms`
  - generated chunks `115`
  - dirty resident chunks `179`
- `shrink-r3-to-r2`:
  - stream avg `0.2 ms`
  - mesh avg `58.0 ms`
  - remeshed chunks `64`

#### Local hypothesis probes

- Fully solid and fully surrounded resident chunks at the default spawn:
  - bootstrap `18`
  - dirty after widen `12`
- All `18` fully surrounded, fully solid bootstrap chunks already produced zero-triangle meshes before the shortcut, which confirms the optimization is avoiding wasted work rather than changing visible geometry.

#### Local benchmark-scene profile spot check

- `terrain256`: build avg `36.1 ms`, mesh avg `154.6 ms`
- `denseCore128`: build avg `67.4 ms`, mesh avg `65.8 ms`
- `stressMicroCubes256`: build avg `4.8 ms`, mesh avg `156.4 ms`
- Interpretation:
  - the new mesher shortcut is valuable for the streamed procedural world
  - the older benchmark scenes stay roughly flat, so this is not a universal mesher win

#### Browser note

- Chrome DevTools browser automation was not available for this slice because the tool-managed Chrome profile became locked after earlier runs and local cleanup of that automation profile was blocked by policy.
- Browser acceptance is therefore still pending for this mesher-specific change.

### Budgeted streaming and far-field overlap verification

#### Commands

- `mise run test`
- `mise run build`
- `mise run profile-stream -- --iterations=2 --warmup=1 --near-radius=2 --far-radius=3`

#### Automated checks

- `mise run test`: passing after:
  - adding far-field regression coverage for in-place clear-radius growth
  - adding resident-world coverage for repeated budgeted updates converging to a complete anchor
- `mise run build`: passing after wiring the game path onto:
  - budgeted chunk generation
  - budgeted dirty meshing
  - the renderer extra-mesh path for procedural far-field bands

#### Warmed local stream-profiler results

- `bootstrap-r3`:
  - stream avg `142.1 ms`
  - mesh avg `94.0 ms`
  - generated chunks `134`
  - dirty resident chunks `134`
- `widen-r2-to-r3`:
  - stream avg `75.4 ms`
  - mesh avg `77.3 ms`
  - generated chunks `73`
  - dirty resident chunks `108`
- `shrink-r3-to-r2`:
  - stream avg `0.21 ms`
  - mesh avg `31.4 ms`
  - evicted chunks `73`
  - dirty resident chunks `35`

#### Notes

- The new `profile-stream` numbers are lower than the earlier full-refresh baseline, but they still measure the non-interactive path that finishes all generation and meshing in one go.
- The live game path is now intentionally different:
  - resident-world generation is capped per update
  - dirty meshing is capped per frame
  - far-field bands fill the long-distance gap visually
- This means the current warmed local profiler is still useful for full-refresh cost, but it is no longer a trustworthy proxy for perceived game-path hitch on `/`.

#### Browser note

- I attempted to re-establish a clean local Chrome/CDP verification lane for this slice.
- The tool-managed DevTools path was still unavailable, and direct local Chrome launch from the current environment failed before exposing a usable debugging endpoint.
- Because of that, this slice was accepted on:
  - unit tests
  - type/build verification
  - the warmed local stream profiler
- The existing Bun dev server is still live on `http://localhost:3015/`, so the next slice should either restore a headless Chrome lane or add a dedicated scriptable browser harness before relying on more visual-distance claims.

### Incremental game-stream tuning verification

#### Commands

- `mise run test`
- `mise run build`
- `mise run profile-game-stream -- --iterations=2 --warmup=1 --radius=5 --generate-budget=6 --mesh-budget=6 --chunk-delta=2`

#### Automated checks

- `mise run test`: passing after:
  - adding pure control-flow tests for same-anchor refresh continuation and background work pumping
  - increasing total passing coverage to `65` tests across `18` files
- `mise run build`: passing after:
  - adding distance fog uniforms and shader logic to the shared WebGPU renderer
  - increasing the default nearby resident radius to `5`
  - tuning the default per-frame streaming budgets to `6/6`

#### Incremental game-stream profile results

- `crossing-d2`, `radius = 5`, `generate-budget = 6`, `mesh-budget = 6`:
  - average settle frames `29`
  - total stream avg `97.7 ms`
  - total mesh avg `127.5 ms`
  - max frame work avg `13.13 ms`
  - max pending chunks `114`
  - total generated chunks `82`
  - total remesh chunks `91`
- `crossing-d1` under the same settings:
  - anchor changed `false`
  - total stream `0 ms`
  - total mesh `0 ms`
  - max frame work about `0.003 ms`

#### Local tuning sweep notes

- `radius = 5`, `12/12` budgets:
  - max frame work about `24.5 ms`
  - rejected as too bursty
- `radius = 5`, `8/8` budgets:
  - max frame work about `17.5 ms`
  - still too close to the budget line
- `radius = 4`, `8/8` budgets:
  - max frame work about `16.3 ms`
  - viable, but gives up too much near voxel detail
- `radius = 6`, `8/8` budgets:
  - max frame work about `16.3 ms`
  - viable, but wider resident churn than needed right now
- Decision:
  - keep `radius = 5`
  - keep per-frame budgets at `6/6`

#### Browser note

- I still do not have a restored Chrome automation lane for visual acceptance of the new fog path.
- This slice is therefore locally verified for:
  - correctness and control-flow behavior
  - buildability of the fog-enabled render path
  - performance tuning through the new incremental profiler
- The next browser-facing step should be either:
  - restore direct Chrome automation
  - or add a dedicated scriptable screenshot/metric harness that does not depend on the shared DevTools session

### Budgeted bootstrap stabilization verification

#### Commands

- `mise exec -- bun test tests/procedural-resident-world.test.ts tests/player-physics.test.ts tests/procedural-far-field.test.ts`
- `mise run test`
- `mise run build`
- fresh Chrome 146 reload on `http://localhost:3015/`

#### Automated checks

- `mise run test`: passing after:
  - adding a resident-world test that proves repeated budgeted updates converge
  - adding a resident-world test that proves a `maxGenerateChunks = 1` update loads the spawn support chunk first
  - keeping far-field clear-radius coverage green
- `mise run build`: passing after restoring full-settle behavior on correctness-critical game paths.

#### Browser checks

- Fresh `/` reload now boots grounded instead of falling through the world:
  - eye position about `144.5 m`
  - feet position about `142.8 m`
  - `Grounded = Yes`
  - `Resident Chunks = 134`
  - `Pending = 0`
- The far field remains active after the stabilization fix:
  - `Far Field = 416 m`
  - `Far Tris = 94,794`
- Scripted browser probe:
  - `window.__VOXELS_GAME__.teleportAndSettle(currentX, currentY, currentZ)` completed with `pending = 0`, `complete = true`, and the player still grounded afterward
  - `window.__VOXELS_GAME__.benchmarkChunkCrossing(2, 1)` reported:
    - stream avg `31.2 ms`
    - mesh avg `81.9 ms`
    - frame CPU avg `0.55 ms`
    - upload avg `0.38 ms`
    - upload chunks avg `83.5`

#### Conclusion

- Bounded streaming is still the right default for ordinary movement.
- Bootstrap, teleports, and other correctness-oriented probes cannot share the same budgeted path, because they need fully loaded support around the player before the first frame is accepted.
- The next verification gap is now explicit: add a separate incremental-movement benchmark so the repo measures ordinary hitch and full-settle correctness with different tools instead of conflating them.

### Incremental movement benchmark and smoother default budgets

#### Commands

- `mise run test`
- `mise run build`
- `mise run profile-game-stream -- --iterations=2 --warmup=1 --radius=5 --generate-budget=6 --mesh-budget=4 --chunk-delta=2`
- fresh Chrome 146 reload on `http://localhost:3015/`

#### Automated checks

- `mise run test`: passing after adding:
  - `tests/stream-work.test.ts`
  - the game-path streaming policy seam used by the incremental benchmark loop
- `mise run build`: passing after widening the default resident radius and adding fog uniforms/shading to the renderer.

#### Warmed local game-stream profiler

- `crossing-d2` with `radius=5`, `generate=6`, `mesh=4`:
  - frames `44`
  - total stream `94.0 ms`
  - total mesh `123.9 ms`
  - max frame work `11.7 ms`
  - max pending chunks `114`
- `crossing-d1` with the same settings:
  - no anchor change
  - zero stream and mesh work
  - only trivial far-field refresh cost

#### Browser checks

- Fresh `/` reload now reports the new default operating point:
  - `Grounded = Yes`
  - `Radius = 5 chunks`
  - `Gen Budget = 6`
  - `Mesh Budget = 4`
  - `Far Field = 416 m`
- Scripted browser benchmark:
  - `window.__VOXELS_GAME__.benchmarkIncrementalCrossing(2, 2, 12, 20)` returned:
    - sample count `128`
    - p95 combined work `12.7 ms`
    - max combined work `13.8 ms`
    - p95 stream `8.1 ms`
    - p95 mesh `4.7 ms`
    - max pending chunks `114`

#### Search result kept

- A broader Chrome search over budget pairs showed the current smoothness tradeoff clearly:
  - `12/12` reduced backlog faster but created combined spikes in the mid-20 ms range
  - `4/4` was even smoother but left too much work pending for too long
  - `6/4` was the best measured compromise worth keeping right now

#### Conclusion

- The main hitch source on the movement path was not upload; it was synchronous generation plus meshing budgets landing in the same frame.
- The new incremental harness is now the acceptance gate for ordinary movement smoothness, while `benchmarkChunkCrossing()` stays as the correctness/full-settle probe.
- The next likely wins are architectural rather than constant-level:
  - differential residency updates instead of full anchor rescans
  - background workers for generation/meshing
  - buffer reuse or larger merged render units once movement hitch is no longer dominated by CPU chunk work

### Game-stream phase breakdown follow-up

#### Commands

- `mise exec -- bunx tsc --noEmit`
- `mise run profile-game-stream -- --iterations=2 --warmup=1 --radius=5 --generate-budget=6 --mesh-budget=4 --chunk-delta=2`

#### Local profiler result

- `crossing-d2` now also records stream subphases:
  - total stream `94.4 ms`
  - total Y-range work `2.8 ms`
  - total chunk generation `88.9 ms`
  - total empty chunks skipped `40`

#### Conclusion

- This rejected the “chunk Y-range recomputation is the next big win” hypothesis.
- Y-range sampling is visible but small; the next meaningful stream-side win must come from generating less or cheaper chunk data, not from memoizing the column-range scan.

### Far-field LOD seam correctness follow-up

#### Commands

- `mise exec -- bun test tests/procedural-far-field.test.ts`
- `mise run test`
- `mise run build`
- fresh Chrome 146 reload on `http://localhost:3015/`
- `window.__VOXELS_GAME__.teleportAndSettle(...)`
- `window.__VOXELS_GAME__.probeLodCoverage(48, 0.8)`
- `window.__VOXELS_GAME__.benchmarkIncrementalCrossing(1, 2, 12, 20)`

#### Automated checks

- `tests/procedural-far-field.test.ts`: passing after adding:
  - inner-hole centering coverage
  - four-direction vertical-face emission
  - exclusion-mask overlap removal
- `mise run test`: passing
- `mise run build`: passing

#### Browser heuristic checks

- Settled spawn probe:
  - `sampleCount = 14641`
  - `residentOverlapCount = 0`
  - `uncoveredGapCount = 0`
  - `bandOverlapCount = 0`
  - `wrongBandCount = 0`
- Settled offset sweep across the same coarse anchor window:
  - tested offsets `0.0 m`, `2.4 m`, `6.4 m`, `9.6 m`, `12.8 m`, `16.0 m`, `19.2 m`
  - every probe stayed at zero for overlaps, gaps, band overlap, and wrong-band coverage
- Budgeted movement-path sweep:
  - stepped across a two-chunk move without waiting for full settle on each step
  - max pending chunks reached `76`
  - max resident overlap `0`
  - max uncovered gaps `0`
  - max band overlap `0`
  - max wrong-band samples `0`
- Broad settled probe:
  - `probeLodCoverage(224, 4.8)` now reports `0` resident overlaps and `0` uncovered gaps across `8,836` samples
  - current residual: `555` inter-band overlap/wrong-band samples at the far/horizon handoff around `224 m`
  - that residual is outside the resident-vs-far seam bug the user reported, but it is now measurable and documented

#### Browser performance spot-check

- `benchmarkIncrementalCrossing(1, 2, 12, 20)` after the far-field fix returned:
  - `sampleCount = 64`
  - `p95WorkMs = 14.0`
  - `maxWorkMs = 15.2`
  - `p95StreamMs = 8.2`
  - `p95MeshMs = 5.6`
  - `maxPendingChunks = 114`

#### Conclusion

- The visible LOD corruption was not one bug. It was the combination of:
  - coverage being centered on a coarse anchor instead of the player
  - masking against a generic clear radius instead of the actual resident columns
  - incomplete side-face emission in the coarse mesh
- The current acceptance gate for this area should stay heuristic-first:
  - unit tests for coverage centering and face directions
  - browser `probeLodCoverage()` for resident overlap/gap counts first, then broader inter-band overlap sampling as a secondary check
  - only then visual spot checks

### Render-ready handoff and transition-band follow-up

#### Commands

- `mise exec -- bunx tsc --noEmit`
- `mise exec -- bun test tests/procedural-resident-world.test.ts tests/procedural-far-field.test.ts`
- `mise run test`
- `mise run build`
- fresh Chrome 146 reload on `http://localhost:3015/`
- `window.__VOXELS_GAME__.probeLodCoverage(...)`
- `window.__VOXELS_GAME__.benchmarkIncrementalCrossing(1, 2, 12, 20)`

#### Automated checks

- `tests/procedural-resident-world.test.ts` now includes:
  - render-ready columns stay visible in far field until their meshes are actually built
- `tests/procedural-far-field.test.ts` now includes:
  - masked-lower-neighbor seam walls
  - water-top preservation inside coarse cells
- `mise run test`: passing
- `mise run build`: passing

#### Browser heuristic checks

- Settled near probe `probeLodCoverage(48, 0.8)`:
  - `uncoveredGapCount = 0`
  - `handoffHoleCount = 0`
  - `bandOverlapCount = 0`
- Budgeted movement sweep across a two-chunk move:
  - `maxPendingChunks = 114`
  - `maxHandoffHoles = 0`
  - this is the important result for the “far chunks disappear before near chunks appear” complaint
- Transition-band stride samples after the change:
  - `18 m -> 0.8 m`
  - `24 m -> 0.8 m`
  - `36 m -> 0.8 m`
  - `48 m -> 1.6 m`
  - `60 m -> 1.6 m`
- Browser water preservation check:
  - found a preserved water patch in the live `transition` band at world cell `(232, -72)`

#### Browser performance spot-check

- `benchmarkIncrementalCrossing(1, 2, 12, 20)` after the transition-band/handoff changes returned:
  - `p95WorkMs = 10.2`
  - `maxWorkMs = 11.2`
  - `p95StreamMs = 6.0`
  - `p95MeshMs = 4.4`
  - `maxPendingChunks = 78`

#### Residual

- The render-ready handoff hole is fixed.
- There is still measurable inter-band overlap at the far/horizon handoff around `224 m`.
- The coverage probe still reports some exact-radius band-boundary artifacts, which now appear to be coarse-band compositing issues rather than resident-vs-far handoff failures.

### Corrected movement benchmark and clipmap-style far-field throttling

#### Commands

- `mise exec -- bun test tests/procedural-far-field.test.ts`
- `mise exec -- bun test tests/procedural-lod-coverage.test.ts tests/stream-work.test.ts`
- `mise run test`
- `mise run build`
- fresh Chrome 146 load on `http://localhost:3016/`
- browser-side monkey-patch probe around `window.__VOXELS_GAME__.benchmarkIncrementalCrossing(1, 2, 12, 20)`

#### Automated checks

- `tests/procedural-far-field.test.ts` now includes:
  - default bands stay stable across a few meters of movement
  - excess band rebuilds can be deferred behind a per-frame budget and drained on the next update
- `tests/stream-work.test.ts` now includes:
  - pending far-field bands keep background work pumping
- `mise run test`: passing
- `mise run build`: passing

#### Browser heuristic checks

- Before the fix, the browser monkey-patch probe around `benchmarkIncrementalCrossing(1, 2, 12, 20)` showed:
  - `66` far-field update calls
  - about `5120.6 ms` total far-field work
  - about `77.6 ms` average far-field work per call
  - about `192.8 ms` max far-field work
  - the old benchmark summary did not include any of that cost
- After the fix on fresh Chrome 146 `http://localhost:3016/`:
  - `66` far-field update calls still occurred, but only `57` changed anything
  - about `717.4 ms` total far-field work
  - about `10.9 ms` average far-field work per call
  - about `89.4 ms` max far-field work
  - `maxPendingBands = 0` on that movement path with the new default center strides and `Far Budget = 1`

#### Corrected browser performance spot-check

- `benchmarkIncrementalCrossing(1, 2, 12, 20)` now reports real total work:
  - `avgWorkMs = 17.5`
  - `p95WorkMs = 32.7`
  - `maxWorkMs = 112.4`
  - `avgFarFieldMs = 11.1`
  - `p95FarFieldMs = 19.5`
  - `maxFarFieldMs = 89.3`
  - `avgStreamMs = 3.1`
  - `avgMeshMs = 3.0`
  - `maxPendingChunks = 114`

#### Residual

- The benchmark now measures the real movement cost instead of hiding far-field rebuilds.
- Far-field churn is much lower, but corrected total work is still too high for the target feel.
- The next target should be near-chunk prioritization or a different near renderer path, not more blind tuning against the old misleading metric.

### Near-detail mesh prioritization follow-up

#### Commands

- `mise exec -- bun test tests/mesher.test.ts tests/stream-work.test.ts tests/procedural-far-field.test.ts`
- `mise run profile-game-stream -- --iterations=2 --warmup=1 --radius=5 --generate-budget=6 --mesh-budget=4 --far-band-budget=1 --chunk-delta=2`
- fresh Chrome 146 dev load on `http://localhost:3016/`
- `window.__VOXELS_GAME__.benchmarkIncrementalCrossing(1, 2, 12, 20)`
- `window.__VOXELS_GAME__.probeRenderReadyCoverage(...)`

#### Automated checks

- `tests/mesher.test.ts` now includes:
  - budgeted meshing prioritizes the chunk nearest the focus point instead of following raw insertion order
- targeted tests above: passing

#### Browser heuristic checks

- Fresh Chrome 146 corrected `benchmarkIncrementalCrossing(1, 2, 12, 20)` after player-centered mesh prioritization returned:
  - `avgWorkMs = 9.93`
  - `p95WorkMs = 17.4`
  - `maxWorkMs = 101.4`
  - `avgFarFieldMs = 2.45`
  - `avgStreamMs = 2.95`
  - `avgMeshMs = 4.38`
  - `changedCount = 22`
  - `incompleteFrameCount = 20`
  - `maxPendingChunks = 112`
- The same benchmark before this slice was at roughly:
  - `avgWorkMs = 17.5`
  - `p95WorkMs = 32.7`
  - `avgFarFieldMs = 11.1`
  - `changedCount = 28`
  - `incompleteFrameCount = 26`
- The new near-detail coverage oracle on that same run showed:
  - `avgResidentNotReadyNearSamples = 223.8`
  - `maxResidentNotReadyNearSamples = 360`
  - this is the clearest current measurement of why the game can still feel behind even after the big far-field fix

#### Local profile spot-check

- `profile-game-stream -- --radius=5 --generate-budget=6 --mesh-budget=4 --far-band-budget=1 --chunk-delta=2` returned:
  - `crossing-d2 totalStreamMs avg = 97.6`
  - `crossing-d2 totalMeshMs avg = 165.9`
  - `crossing-d2 totalFarFieldMs avg = 89.7`
  - `crossing-d2 maxFrameWorkMs avg = 80.6`

#### Residual

- Near-detail scheduling is better, but the new coverage probe shows there is still too much nearby resident-but-not-ready work during a crossing.
- The next likely win is a true mesh work queue or a more radical near-render experiment, not more blind sorting changes.

### Render-ready mask presentation and near-mesh prioritization

#### Commands

- `mise run test`
- `mise run build`
- `mise run profile-game-stream -- --radius=5 --generate-budget=8 --mesh-budget=6 --far-band-budget=1 --chunk-delta=2 --iterations=2 --warmup=1`
- focused local policy probes via `mise exec -- bun -e '...'` comparing:
  - immediate render-ready mask presentation
  - frozen presented mask revision while detailed chunks are still pending or dirty

#### Automated checks

- `tests/mesher.test.ts` now includes:
  - budgeted meshing prioritizes nearby unbuilt chunks around a focus point
- `mise run test`: passing
- `mise run build`: passing

#### Local profiler checks

- With the old immediate render-ready mask presentation policy, the local `crossing-d2` probe still showed roughly:
  - `totalFar ≈ 958 ms`
  - `avgFar ≈ 21.8 ms/frame`
  - `maxFar ≈ 129.1 ms`
- Freezing only the presented render-ready mask revision until detailed chunks are settled dropped the same local probe to about:
  - `totalFar ≈ 97.1 ms`
  - `avgFar ≈ 2.2 ms/frame`
  - `maxFar ≈ 73.3 ms`
- After wiring that policy into the game-path profiler and pairing it with nearby dirty-chunk prioritization plus `8/6` default budgets, `mise run profile-game-stream -- --radius=5 --generate-budget=8 --mesh-budget=6 --far-band-budget=1 --chunk-delta=2 --iterations=2 --warmup=1` reports:
  - `crossing-d2`: `36` frames, `96.4 ms` total stream, `163.0 ms` total mesh, `91.1 ms` total far field, `83.4 ms` max frame work
  - `crossing-d1`: `15.2 ms` total far field for the single no-stream movement step

#### Residual

- The main continuous movement hitch is much smaller now because transition-band mask churn no longer rebuilds every frame.
- A real anchor-crossing spike remains when the far field has to sample and rebuild around a new anchor.
- The next likely target is an incremental transition-band representation or a more radical renderer experiment, not another blind pass over chunk-generation constants.
