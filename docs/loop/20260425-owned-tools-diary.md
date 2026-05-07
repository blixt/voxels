# 2026-04-25 Owned Tools Diary

## Intent

Build an independent correctness and performance loop instead of treating the existing route/browser harness as the only oracle.

Rules for this pass:

- Use direct browser inspection for visual truth.
- Keep repeatable scripts small enough to audit.
- Record artifacts before trusting a conclusion.
- Separate "the app is slow" from "the measurement is missing a bucket."
- Treat LOD, visual coverage, input, physics, and streaming as first-class failure surfaces.

## Baseline

- Branch: `blixt/render-correctness-performance`
- Commit: `40d9047`
- Local checks:
  - `mise exec -- bun run typecheck`: passed
  - `mise exec -- bun test`: passed, `175` tests
  - `mise run build`: passed
- Local cycle artifact:
  - `artifacts/cycle-bench/20260425T095853Z-baseline-20260425.json`
  - No failed steps.
  - Notable local buckets:
    - `terrain256` mesh avg: about `289 ms`
    - `crossing-d2` max frame work avg: about `114 ms`

## Browser Observations

- Opened the game in the in-app browser at `http://localhost:3000/`.
- Visual state reached "World Ready" with terrain visible.
- The visible HUD reported low frame/render numbers after startup, but that is not enough to validate LOD coverage or streaming correctness.

## Measurement Gaps Found

- `probeLodCoverage()` was a placeholder in practice:
  - It only checked LOD0 render-ready columns.
  - It returned empty band lists.
  - It could not detect actual LOD chunk coverage, LOD handoff holes, or multi-band overlaps.
- The existing route benchmark showed high unmeasured frame time on a short Chrome run, and the likely missing bucket is LOD update work.
- The old visible-ground proxy only knows about LOD0 render-ready columns, so it can over-report holes when rendered LOD fallback is actually present.

## New Tooling Started

- Added `scripts/owned-browser-lab.ts`.
- Added package script `lab:browser`.
- Added mise task `browser-lab`.

The owned browser lab:

- Does not import the old browser harness.
- Starts or connects to the game.
- Launches a fresh Chrome profile through its own minimal CDP driver.
- Captures a page screenshot.
- Runs the game page probes and writes a JSON report under `artifacts/owned-browser-lab/`.
- Fails only on core settled-state issues for now:
  - missing game API
  - missing WebGPU
  - no resident chunks
  - no LOD chunks
  - pending LOD chunks after forced settle
  - sampled LOD coverage gaps
  - sampled LOD handoff holes

## Code Changes

- Replaced the placeholder `GameController.probeLodCoverage()` logic with actual renderable LOD chunk span inspection.
- The probe now reports:
  - LOD0 readiness
  - renderable LOD band labels
  - sampled stride in meters
  - uncovered gaps
  - resident-but-uncovered handoff holes
  - LOD0/LOD overlap
  - multi-LOD overlap
- Changed LOD empty-result caching so an LOD chunk downsampled from unknown/missing source chunks is not cached as a known-empty result.

## Next Checks

- `mise exec -- bun run typecheck`: passed.
- `mise exec -- bun test tests/lod-system.test.ts tests/procedural-resident-world.test.ts`: passed.
- `mise run browser-lab -- --server=existing --port=3000 --label=first-owned-lab`: failed usefully because the first lab version probed before resident chunks existed.
- `mise run browser-lab -- --server=existing --port=3000 --label=ready-gated-owned-lab`: failed after readiness was fixed:
  - resident chunks: `834`
  - renderable LOD chunks: `0`
  - sampled LOD gaps: `2933`
- `mise run browser-lab -- --server=existing --port=3000 --label=lod-cache-fix-owned-lab`: failed after the unknown-source cache fix, but improved:
  - resident chunks: `834`
  - renderable LOD chunks: `66`
  - LOD pending keys: `2330`
  - sampled LOD gaps: `2793`

## Current Findings

- The old browser route harness was not the first problem to fix; the app lacked an owned browser oracle that could independently inspect LOD coverage.
- The new lab caught a real asynchronous-game-path LOD issue that pure synchronous unit tests did not expose.
- The first fix recovered some LOD chunks in the browser path, but the LOD system still does not provide continuous sampled coverage out to the claimed `48 m` probe radius.
- The remaining LOD work is now measurable:
  - decide what the expected browser-visible LOD radius actually is
  - stop queuing unsupported far LOD keys as pending work
  - add a browser-facing acceptance threshold once the intended coverage contract is explicit

## 2026-04-25 LOD Fallback Pass

- Added generated far-LOD fallback for missing lower-level source chunks so the LOD chain can produce real distant coverage before every intermediate source chunk exists.
- Added `pendingLodChunks` to the world-work pump condition so browser startup keeps generating LOD after LOD0 residency and meshing settle.
- Fixed the LOD tests to distinguish source-backed downsample correctness from generated fallback coverage.
- `mise exec -- bun test tests/lod-system.test.ts tests/stream-work.test.ts`: passed, `26` tests, before the shell-fallback optimization.
- Local full-sync LOD measurement before shell fallback:
  - generated LOD chunks: `2368`
  - renderable LOD chunks: `1970`
  - total LOD update elapsed: about `8281 ms`
  - downsample time: about `7765 ms`
  - finding: generated fallback worked for coverage, but it filled too much underground volume and created too many vertical chunks.
- Added `scripts/profile-lod-residency.ts`, package script `profile:lod-residency`, and mise task `profile-lod-residency` so repeated local LOD profiles write JSON artifacts instead of relying on ad hoc one-liners.
- Changed generated fallback LOD to a surface/water shell and marked unknown fallback source as incomplete, so unknown empty results are not cached as final known-empty chunks.
- Reworked missing-source fallback downsampling to operate per source block and generated surface column instead of per output voxel.
- Cached empty generated fallback keys again, with explicit invalidation from edits, newly adopted resident chunks, and newly generated finer LOD chunks.
- Added settled no-op passes to `profile-lod-residency` to distinguish first-fill cost from steady-state update cost.
- Local profile artifact `artifacts/lod-residency-profile/20260425T103710Z-settled-cache.json`:
  - first LOD fill: about `2379 ms`
  - downsample: about `2193 ms`
  - mesh: about `148 ms`
  - first settled pass: about `29 ms`, with about `1.2 ms` downsample
  - second settled pass: about `29 ms`, with `0 ms` downsample
- `mise exec -- bun test tests/lod-system.test.ts tests/stream-work.test.ts`: passed, `26` tests, after block fallback and cache invalidation.
- `mise run browser-lab -- --server=existing --port=3000 --label=lod-settled-cache`: passed:
  - resident chunks: `834`
  - renderable LOD chunks: `1266`
  - LOD pending keys: `0`
  - sampled LOD gaps: `0`
  - sampled handoff holes: `0`
  - near render-ready samples: `961/961`
- Browser report also exposed a probe bug: duplicate vertical chunks in one X/Z column were being counted as same-band overlaps. The LOD coverage probe now de-duplicates bands per sample before counting overlap.
- `mise run browser-lab -- --server=existing --port=3000 --label=deduped-lod-probe`: passed:
  - LOD chunks: `1248`
  - LOD pending keys: `0`
  - sampled coverage: `3721/3721`
  - sampled LOD gaps: `0`
  - sampled handoff holes: `0`
  - sampled band overlaps after de-duplication: `0`
  - total draw calls in the settled browser frame: `672`
- Added an LOD needed-key cache guarded by player X/Z position, resident-column revision, and render-ready-column revision.
- Local profile artifact `artifacts/lod-residency-profile/20260425T104120Z-needed-key-cache.json`:
  - first settled no-op after fill: about `31 ms`
  - cache-building no-op: about `31 ms`
  - cache-hit no-op passes: about `0.4 ms`
- `mise run browser-lab -- --server=existing --port=3000 --label=needed-key-cache`: passed:
  - LOD chunks: `1248`
  - LOD pending keys: `0`
  - LOD update in final snapshot: about `0.5 ms`
  - LOD y-range/downsample/mesh in final snapshot: `0 ms`
  - LOD needed-key cache hit: `true`
  - sampled coverage: `3721/3721`
  - sampled gaps/handoff holes/band overlaps: `0/0/0`
- Compacted `owned-browser-lab` transition reporting so the JSON remains reviewable:
  - `artifacts/owned-browser-lab/20260425T104228Z-compact-report/report.json`
  - report size: `28K`, `1220` lines
  - still records transition summaries, sampled chunks, screenshot path, coverage probes, and final LOD timing.
- Validation after the LOD/cache/tooling pass:
  - `mise exec -- bun run typecheck`: passed
  - `mise exec -- bun test`: passed, `175` tests
  - `mise run build`: passed

## 2026-04-25 Fog-Distance LOD Water Pass

- Expanded the owned browser LOD probe to the full fog end distance:
  - `mise run browser-lab -- --server=existing --port=3000 --label=lod-radius-128m --lod-radius=128 --lod-step=4`: passed
  - `mise run browser-lab -- --server=existing --port=3000 --label=lod-radius-fog --lod-radius=416 --lod-step=8`: failed with `14` uncovered samples.
- Root cause:
  - The uncovered fog-edge samples were water-only LOD chunks.
  - LOD data contained water voxels, but LOD meshing only built opaque geometry, so water-only chunks had `solidCount > 0` and `mesh.indexCount = 0`.
- Added far-water LOD rendering:
  - LOD chunks now build greedy top-surface water quads.
  - Renderer stats count LOD water draw calls.
  - LOD coverage treats water-only LOD chunks as renderable.
- Validation:
  - `mise exec -- bun run typecheck`: passed
  - `mise exec -- bun test tests/lod-system.test.ts tests/water-visuals.test.ts tests/mesher.test.ts`: passed, `36` tests
  - `mise run browser-lab -- --server=existing --port=3000 --label=lod-fog-water --lod-radius=416 --lod-step=8`: passed:
    - sampled coverage: `11025/11025`
    - sampled LOD gaps: `0`
    - sampled handoff holes: `0`
    - LOD chunks: `1266`
    - final LOD update: about `0.4 ms`, cache hit `true`
    - final browser draw calls: `732`

## 2026-04-25 Fog Cull Pass

- Added renderer culling for chunks whose entire AABB is beyond the fog end distance.
- Exposed `fogCulledChunks` in render stats, game snapshots, and bootstrap benchmark summaries.
- Validation:
  - `mise exec -- bun run typecheck`: passed
  - `mise exec -- bun test tests/game-bootstrap-benchmark.test.ts tests/lod-system.test.ts tests/water-visuals.test.ts tests/mesher.test.ts`: passed, `37` tests
  - `mise run browser-lab -- --server=existing --port=3000 --label=fog-cull --lod-radius=416 --lod-step=8`: passed:
    - sampled coverage: `11025/11025`
    - sampled LOD gaps/handoff holes: `0/0`
    - final draw calls: `602` (`732` before fog cull on the comparable full-fog probe)
    - final LOD draw calls: `365`
    - fog-culled chunks: `121`

## 2026-04-25 HUD Polish Pass

- Compacted the collapsed telemetry panel into a smaller top strip so it no longer dominates screenshots.
- Moved the "World Ready" capture prompt lower once loading is complete, keeping the terrain and horizon more visible before pointer lock.
- `mise run browser-lab -- --server=existing --port=3000 --label=ui-polish --lod-radius=416 --lod-step=8`: passed:
  - sampled coverage: `11025/11025`
  - sampled LOD gaps/handoff holes: `0/0`
  - final LOD update: about `0.4 ms`, cache hit `true`
  - screenshot: `artifacts/owned-browser-lab/20260425T105226Z-ui-polish/settled-page.png`
- Final validation for this checkpoint:
  - `mise exec -- bun run typecheck`: passed
  - `mise exec -- bun test`: passed, `175` tests
  - `mise run build`: passed
- Standard cycle artifact:
  - `artifacts/cycle-bench/20260425T105008Z-render-lod-water-fog-cull.json`
  - failed steps: none
  - `crossing-d2` max frame work avg: about `82 ms` (`114 ms` in the initial baseline diary)
  - `terrain256` mesh avg stayed about `289 ms`, as expected for unchanged scene meshing.

## 2026-04-25 HUD Reset And Capture Repair

- Replaced the layered HUD with a smaller passive overlay:
  - one top-left objective strip
  - one top-right performance strip
  - a center crosshair
  - a compact bottom hotbar
  - no telemetry panel, target text wall, top brand block, or nested panel stack
- Fixed the click-to-play path:
  - the capture gate is a full-screen click target, so the prompt is not the only clickable area
  - the gate registers as a pointer-lock target
  - if the browser refuses real pointer lock, the controller enters an input-captured fallback instead of leaving the player trapped behind the prompt
  - gameplay mouse buttons and wheel now listen at the document level while captured, so they still work when the lock target is not the canvas
- Added a repeatable HUD smoke check to `owned-browser-lab`:
  - verifies old HUD text is absent
  - clicks the gate through CDP input
  - verifies the prompt leaves the visible HUD and input capture becomes active
- In-app browser verification:
  - before the fallback, CUA clicks focused the button but the prompt stayed visible
  - after the fallback, CUA click removed the visible prompt and left only the compact HUD
- Validation:
  - `mise exec -- bun run typecheck`: passed
  - `mise exec -- bun test`: passed, `175` tests
  - `mise run build`: passed
  - `mise run browser-lab -- --server=existing --port=3000 --label=hud-reset --lod-radius=128 --lod-step=8`: passed
    - HUD smoke: passed
    - sampled LOD gaps/handoff holes: `0/0`
    - render-ready near samples: `961/961`
    - screenshot: `artifacts/owned-browser-lab/20260425T111040Z-hud-reset/settled-page.png`

## 2026-04-25 Canvas Pointer Lock Tightening

- Changed the click-to-play path so real gameplay capture only counts when the canvas owns pointer lock:
  - removed the overlay-button pointer-lock target
  - removed the automatic "captured" fallback from the real controller path
  - capture clicks now focus the canvas and call `canvas.requestPointerLock()`
  - mouse users request lock on `pointerdown`; `click` remains as a secondary activation path
- Kept the browser lab honest:
  - it instruments `canvas.requestPointerLock()` and verifies the click path calls it on the canvas
  - it records whether the browser actually granted or rejected pointer lock instead of faking success
  - current headless Chrome rejects the real lock with `WrongDocumentError`, but the click path target is confirmed as the canvas
- Validation:
  - `mise exec -- bun run typecheck`: passed
  - `mise exec -- bun test tests/interaction-loop.test.ts tests/targeting-overlay.test.ts`: passed, `6` tests
  - `mise run browser-lab -- --server=existing --port=3000 --label=canvas-pointer-lock-final --lod-radius=128 --lod-step=8`: passed
    - HUD smoke: passed
    - `requestPointerLockCalled`: `true`
    - `requestPointerLockTargetWasCanvas`: `true`
    - headless grant result: rejected with `WrongDocumentError`
    - sampled LOD gaps/handoff holes: `0/0`

## 2026-04-25 Input Capture And True FPS Fix

- Reworked capture after browser verification showed the Pointer Lock API can be rejected with `WrongDocumentError` in the current browser harness.
- The click path now:
  - requests real pointer lock on the capture gate the user clicked
  - treats both the canvas and capture gate as valid real pointer-lock owners
  - falls back to input-captured mode if the browser rejects Pointer Lock, so the prompt disappears and WASD still works
  - lets `Escape` leave fallback capture
- Replaced the HUD FPS source:
  - old FPS was `1000 / avgFrameCpuMs`, which measured render CPU work and could report hundreds or thousands of FPS while the browser was visibly presenting much slower
  - new FPS is `1000 / avgFrameWallMs`, based on `requestAnimationFrame` wall-clock deltas
  - the performance tooltip still exposes CPU, stream, mesh, LOD, and fog-cull details
- In-app browser check:
  - clicking removed the play prompt instead of showing "Pointer lock blocked"
  - while the browser was visibly slow during world work, the HUD reported about `9.8 FPS` instead of a fake near-1000 value
- Validation:
  - `mise exec -- bun run typecheck`: passed
  - `mise exec -- bun test tests/interaction-loop.test.ts tests/targeting-overlay.test.ts tests/game-bootstrap-benchmark.test.ts`: passed, `7` tests
  - `mise run build`: passed
  - `mise run browser-lab -- --server=existing --port=3000 --label=input-capture-fps-truth --lod-radius=128 --lod-step=8`: passed
    - HUD smoke: passed
    - request target: `capture`
    - browser Pointer Lock result: rejected with `WrongDocumentError`
    - fallback input capture: active
    - held `W` moved about `0.92 m`
    - sampled LOD gaps/handoff holes: `0/0`
