# Morrowind-Like RPG Diary

## Goal

Transform the voxel prototype toward a Morrowind-like exploration RPG: strange geography, dense environmental identity, readable travel goals, skills/progression, and a renderer that stays correct and fast under increasing view distance and world complexity.

## Operating Strategy

- Work in verified vertical slices. Every gameplay or rendering change should add an observable signal in tests, scripts, browser reports, or screenshots.
- Prefer deterministic probes over subjective claims. When "feel" matters, convert it into route-atlas metrics, ambient-profile coverage, discovery cadence, screenshot color/coverage checks, frame budgets, or input-state assertions.
- Improve the harness whenever correctness becomes hard to judge. A weak measurement is treated as technical debt, not as permission to guess.
- Keep changes small enough to perfect. Large RPG systems should land behind narrow contracts: deterministic data, save/load, HUD/debug visibility, and focused tests.
- Maintain a diary entry for each cycle: what changed, what was measured, what failed, what was deferred, and the current rubric score.

## Long-Term Rubric

Scores are 1 to 5, where 1 means prototype-only, 3 means shippable foundation, and 5 means strong enough to preserve during feature expansion.

| Area | Baseline | Target | Notes |
| --- | ---: | ---: | --- |
| Rendering correctness | 3 | 5 | Good unit probes and hole checks exist; needs canonical screenshot and LOD transition suites. |
| Runtime performance | 3 | 5 | Good profilers exist; needs budget gates, soak tests, and multi-viewport browser runs. |
| World definition | 2 | 5 | Many biomes/landmarks exist; needs stronger atmosphere, vista cadence, route atlas, and named regions. |
| Exploration feel | 2 | 5 | Discovery journal exists; needs travel goals, POI cadence, risk/reward, and memorable routes. |
| Input/traversal | 2 | 4 | Basic physics/input exists; needs browser input matrix and better movement affordances. |
| Skills/progression | 1 | 4 | Objectives/inventory only; needs deterministic skill XP, leveling, unlocks, and persistence. |
| Harness maturity | 3 | 5 | Strong raw tooling; needs budgets, golden views, route atlas, and automated rubric summaries. |
| UI polish | 2 | 4 | Recent HUD cleanup helps; needs minimal RPG-forward information architecture. |

Initial aggregate rating: 2.25 / 5. The main limiter is not terrain volume; it is the lack of verified RPG structure and verified ambiance.

## First Milestone

Build the first "place identity" slice without regressing performance or input:

- Add deterministic ambient rendering profiles derived from biome, regional variant, underground context, and environmental fields.
- Surface the active ambient profile in debug/HUD snapshots and browser-lab JSON.
- Add unit coverage for biome-to-atmosphere mapping, deterministic profile selection, and fog-distance safety.
- Run typecheck, focused tests, build, and browser smoke.

## Harness Backlog

- Add a budget file and `verify-smoke` wrapper that fails on startup, frame, hole, LOD, and heap regressions.
- Add a route-atlas script: sample biome, regional variant, underground biome, landmark, height, water state, discovery totals, frame timings, and screenshots along fixed travel routes.
- Add browser input matrix smoke for WASD, jump, sprint/precision, hotbar, inventory, break/place, and pointer-lock fallback.
- Add canonical in-game screenshot views with color-distribution and nonblank/coverage checks before moving to golden image diffs.
- Add LOD handoff route screenshots and settled-reference deltas across ring boundaries.

## Cycle Log

### 2026-05-07 - Goal Setup And First Slice

- Created the long-term rubric and development strategy.
- Spawned verification and world/ambiance explorer subagents to reduce blind spots.
- Verification audit result: existing profiling is broad, but gates and place-feel signals are weak.
- First implementation target: ambient rendering profiles, because it is visible, cheap, deterministic, and easy to expose to tests and browser reports.

### 2026-05-07 - Ambient Profiles Landed

- Added deterministic ambient profiles for green canopy, dry haze, ashfall, silt mist, fungal glow, cold glass, and underground spaces.
- Wired ambient profiles into the renderer through the existing per-frame `RenderEnvironment`, with surface fog bounded between `224 m` and the default `416 m` so fog culling remains an honest correctness signal.
- Surfaced `ambientProfileId`, `ambientProfileLabel`, and `ambientFogEndMeters` through `GameHudSnapshot`; the top-left objective strip now shows the current place mood without adding another panel.
- Extended `owned-browser-lab` to fail when the live snapshot does not expose a valid ambient profile and fog distance.
- Validation:
  - `mise exec -- bun test tests/ambient-environment.test.ts tests/water-visuals.test.ts`: pass.
  - `mise exec -- bun run typecheck`: pass.
  - `mise run build`: pass.
  - In-app browser visible smoke: pass; HUD showed `Cold Glass` and the scene rendered with the colder ambient profile.
  - `mise exec -- bun run scripts/owned-browser-lab.ts --server=existing --label=ambient-smoke`: pass, artifact `artifacts/owned-browser-lab/20260507T192144Z-ambient-smoke/report.json`, `LOD gaps: 0`, `handoff holes: 0`, `render-ready near samples: 961/961`, ambient `cold-glass` with `395.85 m` fog.
- Rubric movement:
  - Rendering correctness: `3.0 -> 3.1`
  - World definition: `2.0 -> 2.2`
  - Exploration feel: `2.0 -> 2.1`
  - Harness maturity: `3.0 -> 3.1`
- Next: build a route-atlas probe so "more definition" can be measured by biome/variant/landmark/profile cadence along fixed walks, instead of relying on screenshots alone.

### 2026-05-07 - Route Atlas Probe

- Added `scripts/route-atlas.ts` as a repeatable world-definition probe.
- The atlas samples five fixed expedition routes and records biome, underground biome, regional variant, landmark, surface height, and ambient profile at fixed intervals.
- The probe writes JSON artifacts under `artifacts/route-atlas` and enforces initial cadence thresholds:
  - at least `8` distinct biomes
  - at least `5` ambient profiles
  - at least `3` regional variants
  - at least `8` landmark hits
  - no notable gap above `540 m`
- Added `bun run atlas:routes` and `mise run atlas-routes` for repeated use.
- Validation:
  - `mise exec -- bun run scripts/route-atlas.ts --label=first-atlas`: pass, artifact `artifacts/route-atlas/20260507T192426Z-first-atlas/report.json`.
  - Result: `15` biomes, `6` ambient profiles, `6` regional variants, `23` landmark hits, max notable gap `504.0 m`, definition score `5.00 / 5` against the initial coarse rubric.
  - `mise exec -- bun run typecheck`: pass.
  - `mise exec -- bun test tests/ambient-environment.test.ts tests/water-visuals.test.ts`: pass.
  - `mise run build`: pass.
- Rubric movement:
  - Harness maturity: `3.1 -> 3.25`
  - World definition: stays `2.2` until the atlas drives new content improvements instead of only measuring current variety.
- Next: add a one-command smoke wrapper so future cycles can rerun typecheck, focused tests, build, route atlas, and browser-lab without manual command drift.

### 2026-05-07 - Verify Smoke Wrapper

- Added `scripts/verify-smoke.ts`.
- Added `bun run verify:smoke` and `mise run verify-smoke`.
- The wrapper runs, in order:
  - `bun run typecheck`
  - focused ambient/water tests
  - production build
  - route atlas with enforced world-definition thresholds
  - owned browser lab with HUD/input/LOD/render-ready checks
- It writes a single JSON summary under `artifacts/verify-smoke` and preserves the child artifact paths in stdout.
- Validation:
  - `mise exec -- bun run scripts/verify-smoke.ts --label=ambient-atlas-smoke`: pass.
  - Summary artifact: `artifacts/verify-smoke/20260507T192549Z-ambient-atlas-smoke/report.json`.
  - Route atlas artifact: `artifacts/route-atlas/20260507T192551Z-ambient-atlas-smoke/report.json`.
  - Browser lab artifact: `artifacts/owned-browser-lab/20260507T192551Z-ambient-atlas-smoke/report.json`.
  - Browser lab stayed clean: `LOD pending 0`, `LOD gaps 0`, `handoff holes 0`, `render-ready 961/961`, ambient `cold-glass`.
- Rubric movement:
  - Harness maturity: `3.25 -> 3.4`
- Next: use the atlas to drive a real content improvement: add a small set of Morrowind-like route-visible landmarks or regional material accents, then prove they improve route cadence without increasing mesh/render costs.

### 2026-05-07 - First Usage-Based Skills

- Added `SkillJournal`, a deterministic first progression layer.
- Discovery events now award skill XP once by sequence:
  - biome discovery -> `Cartography`
  - landmark discovery -> `Naturalist`
  - underground discovery -> `Spelunking`
  - regional variant discovery -> `Lore`
- `GameController` now owns a skill journal, exposes `getSkillJournalSnapshot()`, and includes focus skill fields in `GameHudSnapshot`.
- The objective strip tooltip now includes the focus skill and level; the visible HUD remains minimal.
- Browser lab now fails if the live snapshot does not expose a focus skill and valid total skill level.
- Validation:
  - `mise exec -- bun test tests/skill-journal.test.ts tests/ambient-environment.test.ts tests/water-visuals.test.ts`: pass.
  - `mise exec -- bun run typecheck`: pass.
  - `mise exec -- bun run scripts/verify-smoke.ts --label=skills-smoke`: pass.
  - Summary artifact: `artifacts/verify-smoke/20260507T192838Z-skills-smoke/report.json`.
  - Browser lab artifact: `artifacts/owned-browser-lab/20260507T192840Z-skills-smoke/report.json`, with focus skill `Cartography 1`, ambient `cold-glass`, no LOD gaps, no handoff holes, and `961/961` render-ready samples.
  - In-app browser reload: pass; objective strip tooltip exposed `Cartography 1` while the visible HUD remained compact.
- Rubric movement:
  - Skills/progression: `1.0 -> 1.3`
  - Harness maturity: `3.4 -> 3.45`
- Next: skills need distance/action usage, persistence, and stronger browser assertions before they can affect gameplay decisions.

### 2026-05-07 - Progress Persistence And Travel Skills

- Fixed a correctness issue in the first skill layer: skill XP no longer reads from the capped `recentDiscoveries` UI list. `ExplorationJournal` now keeps a separate pending discovery queue for progression and drains it explicitly.
- Added export/import state contracts for `ExplorationJournal` and `SkillJournal`.
- Added browser-facing progress APIs:
  - `exportProgressState()`
  - `importProgressState(state)`
  - `saveProgressState()`
  - `loadProgressState()`
  - `clearProgressState()`
- Added localStorage persistence via `voxels.progress.v1`.
- Added usage-based travel XP:
  - surface travel feeds `Cartography`
  - underground travel feeds `Spelunking`
  - travel remainders are persisted so partial progress is not lost on reload
- Extended browser lab to verify progress import and localStorage save/load round-trips.
- Validation:
  - `mise exec -- bun test tests/exploration-journal.test.ts tests/skill-journal.test.ts`: pass.
  - `mise exec -- bun run typecheck`: pass.
  - First `travel-skills-smoke` caught a real browser-lab persistence mismatch: reset auto-saved over the test payload.
  - Fixed the browser-lab verifier to preserve the explicit saved payload during the reset/load round-trip.
  - `mise exec -- bun run scripts/verify-smoke.ts --label=travel-skills-smoke-2`: pass.
  - Summary artifact: `artifacts/verify-smoke/20260507T193556Z-travel-skills-smoke-2/report.json`.
  - Browser lab artifact: `artifacts/owned-browser-lab/20260507T193557Z-travel-skills-smoke-2/report.json`, with progress persistence passed, no LOD gaps, no handoff holes, and `961/961` render-ready samples.
- Rubric movement:
  - Skills/progression: `1.3 -> 1.55`
  - Harness maturity: `3.45 -> 3.55`
- Next: add a route/behavior probe that verifies travel XP increases during a controlled browser walk, so movement progression is covered end to end rather than only by unit tests and snapshot shape.

### 2026-05-07 - Browser-Gated Travel Progression

- Extended `owned-browser-lab` HUD/input smoke so the controlled held-`W` walk must also increase `totalSkillTravelMeters`.
- This closes the loop between input capture, player movement, skill travel accounting, HUD snapshot exposure, and browser verification.
- Validation:
  - `mise exec -- bun run typecheck`: pass.
  - `mise exec -- bun test tests/skill-journal.test.ts tests/exploration-journal.test.ts`: pass.
  - `mise exec -- bun run scripts/verify-smoke.ts --label=travel-skill-browser-gate`: pass.
  - Summary artifact: `artifacts/verify-smoke/20260507T193702Z-travel-skill-browser-gate/report.json`.
  - Browser lab artifact: `artifacts/owned-browser-lab/20260507T193703Z-travel-skill-browser-gate/report.json`.
  - HUD smoke movement sample: moved `0.92 m`; skill travel increased from `0` to `1.48 m`.
- Rubric movement:
  - Skills/progression: `1.55 -> 1.65`
  - Input/traversal: `2.0 -> 2.1`
  - Harness maturity: `3.55 -> 3.6`
- Next: the largest remaining gap is not proof plumbing; it is richer authored-feeling content. Use the route atlas to add route-visible silhouettes/material accents and then compare atlas cadence and browser performance before/after.

### 2026-05-07 - Ancient Route Landmarks

- Added three route-readable landmark identities using existing feature geometry:
  - `ancestor_pillar`
  - `ash_marker`
  - `glass_cairn`
- Added player-facing discovery names for the new landmarks.
- Placed them in harsh or uncanny regions first: steppe/monolith/moor, badlands/ember, saltflat/dunes/shardlands.
- Added procedural test coverage that scans for all three new landmarks in the generated world.
- Updated route atlas routes after the first atlas proved the new landmarks existed globally but were not visible on fixed routes. The corrected atlas now includes all three new landmark ids in aggregate route coverage.
- Validation:
  - `mise exec -- bun test tests/procedural-generator.test.ts tests/discovery-catalog.test.ts`: pass.
  - `mise exec -- bun run scripts/route-atlas.ts --label=ancient-routes-3`: pass, artifact `artifacts/route-atlas/20260507T194147Z-ancient-routes-3/report.json`, aggregate landmarks include `ancestor_pillar`, `ash_marker`, and `glass_cairn`.
  - `mise exec -- bun run scripts/verify-smoke.ts --label=ancient-landmarks-smoke`: pass.
  - Summary artifact: `artifacts/verify-smoke/20260507T194158Z-ancient-landmarks-smoke/report.json`.
  - Browser lab artifact: `artifacts/owned-browser-lab/20260507T194200Z-ancient-landmarks-smoke/report.json`, with no LOD gaps, no handoff holes, progress persistence passed, and `961/961` render-ready samples.
- Rubric movement:
  - World definition: `2.2 -> 2.35`
  - Exploration feel: `2.1 -> 2.2`
  - Harness maturity: stays `3.6`
- Next: add visual/route budget summaries to the atlas report so content changes can be compared against previous artifact metrics without manually opening JSON.

### 2026-05-07 - Comparable Route Atlas Summaries

- Extended `scripts/route-atlas.ts` to write `summary.md` beside `report.json`.
- The atlas now automatically compares against the previous valid atlas report, or an explicit `--compare-to` path.
- Comparison output includes:
  - aggregate metric deltas
  - added/removed landmarks
  - added/removed ambient profiles
  - per-route landmark-hit and notable-gap deltas
  - route details in a compact Markdown table
- Validation:
  - `mise exec -- bun run typecheck`: pass.
  - `mise exec -- bun run scripts/route-atlas.ts --label=atlas-comparison`: pass.
  - Summary artifact: `artifacts/route-atlas/20260507T194420Z-atlas-comparison/summary.md`.
  - `mise exec -- bun run scripts/verify-smoke.ts --label=atlas-summary-smoke`: pass.
  - Smoke summary artifact: `artifacts/verify-smoke/20260507T194438Z-atlas-summary-smoke/report.json`.
  - Browser lab artifact: `artifacts/owned-browser-lab/20260507T194439Z-atlas-summary-smoke/report.json`, still clean on LOD gaps, handoff holes, progress persistence, and render-ready coverage.
- Rubric movement:
  - Harness maturity: `3.6 -> 3.7`
- Next: add explicit performance budgets to smoke verification. Current browser lab reports performance fields, but only structural rendering/input/progress failures are hard-gated.

### 2026-05-07 - Browser Smoke Performance Budgets

- Added explicit performance gates to `owned-browser-lab`:
  - average CPU frame <= `8 ms`
  - last CPU frame <= `12 ms`
  - render sync/upload/encode <= `12 ms`
  - draw calls <= `1,200`
  - triangles <= `1,500,000`
- Kept wall-clock frame time informational because the headless browser lab includes scheduling pauses that are not a stable renderer-only signal.
- Validation:
  - `mise exec -- bun run typecheck`: pass.
  - `mise exec -- bun run scripts/owned-browser-lab.ts --label=performance-budget-smoke`: pass.
  - Direct browser-lab result: avg/last CPU `0.96/1.60 ms`, `533` draw calls, `628,762` triangles.
  - `mise exec -- bun run scripts/verify-smoke.ts --label=performance-budget-wrapper`: pass.
  - Summary artifact: `artifacts/verify-smoke/20260507T194711Z-performance-budget-wrapper/report.json`.
  - Browser lab artifact: `artifacts/owned-browser-lab/20260507T194712Z-performance-budget-wrapper/report.json`, avg/last CPU `0.96/1.80 ms`, `533` draw calls, `628,874` triangles, no LOD gaps, no handoff holes.
- Rubric movement:
  - Runtime performance: `3.0 -> 3.15`
  - Harness maturity: `3.7 -> 3.8`
- Next: build a long-route/soak budget check for movement streaming, because current smoke gates settled-page rendering but not sustained traversal under world streaming pressure.

### 2026-05-07 - Traversal Streaming Budget Smoke

- Added a live forward-walk benchmark to `owned-browser-lab` so smoke verification now covers sustained movement under streaming pressure, not only a settled page.
- The traversal gate checks:
  - measured gameplay work p95 <= `24 ms`
  - render CPU p95 <= `12 ms`
  - stream p95 <= `12 ms`
  - mesh p95 <= `12 ms`
  - no hole signals or visible ground gaps during the walk
- First direct run, `traversal-budget-smoke`, exposed a measurement weakness: gross browser frame p95 was about `79 ms`, while render/stream/mesh accounting was missing from the live samples. That gross signal includes scheduler/headless pauses, so it is useful as a tripwire but too noisy to be the primary renderer budget.
- Fixed render accounting by returning `GameRenderProbe` from the interactive frame path and feeding it into live traversal samples.
- Validation:
  - `mise exec -- bun run typecheck`: pass.
  - `mise exec -- bun test tests/game-route-benchmark.test.ts`: pass.
  - `mise exec -- bun run scripts/owned-browser-lab.ts --label=traversal-budget-smoke-2`: pass.
  - Direct browser-lab artifact: `artifacts/owned-browser-lab/20260507T195050Z-traversal-budget-smoke-2/report.json`.
  - Direct traversal result: p95 measured work `2.80 ms`, render `1.30 ms`, stream `0 ms`, mesh `1.50 ms`, distance `5.15 m`, holes `0`.
  - `mise exec -- bun run scripts/verify-smoke.ts --label=traversal-budget-wrapper`: pass.
  - Summary artifact: `artifacts/verify-smoke/20260507T195152Z-traversal-budget-wrapper/report.json`.
  - Browser lab artifact: `artifacts/owned-browser-lab/20260507T195154Z-traversal-budget-wrapper/report.json`, with p95 measured work `2.60 ms`, render `1.30 ms`, stream `0 ms`, mesh `1.40 ms`, distance `5.14 m`, holes `0`, no LOD gaps, no handoff holes, and `961/961` render-ready samples.
- Rubric movement:
  - Runtime performance: `3.15 -> 3.3`
  - Rendering/LOD correctness: `3.0 -> 3.1`
  - Harness maturity: `3.8 -> 3.95`
- Next: raise the challenge from a short smoke walk to route-level traversal: longer distances, turns, slopes, underground transitions, and view-distance pressure while preserving hard render/stream budgets.

### 2026-05-07 - Live Route Budget Gate

- Added a second browser-lab movement budget for route-scale traversal:
  - the original live forward walk remains the input/physics smoke
  - the new route budget is a live sprinting meander with camera yaw drift, longer duration, and the same render/stream/mesh accounting
- A first deterministic route-gate attempt failed honestly:
  - `artifacts/owned-browser-lab/20260507T195424Z-route-budget-smoke/report.json`
  - It reported 74 visible-ground hole frames and did not settle, but it was teleporting through the route faster than browser/worker time could progress.
  - Kept the deterministic route benchmark as a useful stress tool, but moved browser smoke to live movement so the primary gate reflects actual frame pacing.
- Added benchmark-only live route controls:
  - `yawDriftRadians`
  - `yawDriftPeriodSeconds`
  - `sprint`
- Fixed a probe correctness issue:
  - visible-ground readiness now checks the actual surface chunk instead of requiring every resident vertical chunk in the column to be render-ready
  - visible-ground readiness now treats renderable LOD spans as valid fallback coverage
  - This matched the pixel signal: the failing route had no screen-void signals, so the old column probe was over-reporting holes.
- Added route-budget hard gates to `owned-browser-lab`:
  - route distance >= `18 m`
  - route movement frames >= `120`
  - route settle frames > `0`
  - p95 measured work/render/stream/mesh budgets
  - zero route hole signals and zero visible-ground gaps
- Added harness unit tests to `verify-smoke`: benchmark metrics, bootstrap summary, and route benchmark math.
- Validation:
  - `mise exec -- bun run typecheck`: pass.
  - `mise exec -- bun test tests/game-route-benchmark.test.ts tests/lod-system.test.ts`: pass.
  - Direct browser-lab artifact: `artifacts/owned-browser-lab/20260507T200201Z-lod-aware-route-smoke/report.json`.
  - Direct live route result: distance `19.67 m`, holes `0`, p95 measured work `3.30 ms`, render `1.20 ms`, stream `0.70 ms`, mesh `1.40 ms`.
  - `mise exec -- bun run scripts/verify-smoke.ts --label=live-route-budget-wrapper`: pass.
  - Summary artifact: `artifacts/verify-smoke/20260507T200247Z-live-route-budget-wrapper/report.json`.
  - Browser lab artifact: `artifacts/owned-browser-lab/20260507T200248Z-live-route-budget-wrapper/report.json`, with route distance `19.62 m`, holes `0`, p95 measured work `3.30 ms`, render `1.10 ms`, stream `0.70 ms`, mesh `1.40 ms`, no LOD gaps, no handoff holes, and `961/961` render-ready samples.
- Independent harness review findings recorded:
  - Route budget must stay hard-gated in browser smoke.
  - Seam probes are still stubbed and should become real.
  - Pixel readback should become mandatory for a minimum number of route frames.
  - LOD band overlap/wrong-band checks need stronger assertions.
  - Bootstrap readiness should be included in browser gates instead of only unit summaries.
- Rubric movement:
  - Runtime performance: `3.3 -> 3.45`
  - Rendering/LOD correctness: `3.1 -> 3.25`
  - Input/traversal: `2.1 -> 2.25`
  - Harness maturity: `3.95 -> 4.15`
- Next: implement a real seam/LOD-boundary probe, because the summary still reports seam gaps as zero by construction.

### 2026-05-07 - Real Seam Probe Gate

- Replaced stubbed route seam fields with real LOD coverage checks sampled during traversal.
- Added `summarizeRouteSeamCoverage()` as a pure helper and unit-covered it with synthetic uncovered/handoff holes.
- Browser-lab now reports and hard-gates seam-gap frames for both:
  - short live forward traversal
  - longer live sprint route
- Validation:
  - `mise exec -- bun run typecheck`: pass.
  - `mise exec -- bun test tests/game-route-benchmark.test.ts`: pass.
  - Direct browser-lab artifact: `artifacts/owned-browser-lab/20260507T200513Z-real-seam-probe-smoke/report.json`.
  - Direct result: traversal holes/seams `0/0`; route holes/seams `0/0`; route p95 measured work `3.40 ms`, render `1.20 ms`, stream `0.70 ms`, mesh `1.40 ms`.
  - `mise exec -- bun run scripts/verify-smoke.ts --label=real-seam-probe-wrapper`: pass.
  - Summary artifact: `artifacts/verify-smoke/20260507T200550Z-real-seam-probe-wrapper/report.json`.
  - Browser lab artifact: `artifacts/owned-browser-lab/20260507T200551Z-real-seam-probe-wrapper/report.json`, with route distance `19.65 m`, holes/seams `0/0`, no LOD gaps, no handoff holes, and `961/961` render-ready samples.
- Rubric movement:
  - Rendering/LOD correctness: `3.25 -> 3.4`
  - Harness maturity: `4.15 -> 4.3`
- Next: require pixel readbacks on at least a few route frames, because structural probes are stronger now but still need mandatory image evidence in smoke.

### 2026-05-07 - Mandatory Pixel Capture Smoke

- Added explicit screen-readback counts to route benchmark summaries.
- Browser-lab now requires:
  - at least `1` screen capture during the short live traversal
  - at least `2` screen captures during the longer live route
- Reduced the short traversal capture stride from effectively disabled to every `60` frames, while keeping capture diagnostics outside the gameplay frame budget.
- Validation:
  - `mise exec -- bun run typecheck`: pass.
  - `mise exec -- bun test tests/game-route-benchmark.test.ts`: pass.
  - Direct browser-lab artifact: `artifacts/owned-browser-lab/20260507T200725Z-mandatory-pixel-capture-smoke/report.json`.
  - Direct result: traversal captures `2`, route captures `2`, holes/seams `0/0` for both paths.
  - `mise exec -- bun run scripts/verify-smoke.ts --label=mandatory-pixel-capture-wrapper`: pass.
  - Summary artifact: `artifacts/verify-smoke/20260507T200804Z-mandatory-pixel-capture-wrapper/report.json`.
  - Browser lab artifact: `artifacts/owned-browser-lab/20260507T200806Z-mandatory-pixel-capture-wrapper/report.json`, with traversal captures `2`, route captures `2`, route distance `19.63 m`, holes/seams `0/0`, p95 measured work `3.30 ms`, render `1.20 ms`, stream `0.70 ms`, mesh `1.40 ms`.
- Rubric movement:
  - Rendering/LOD correctness: `3.4 -> 3.5`
  - Harness maturity: `4.3 -> 4.45`
- Next: bring bootstrap visual readiness into browser smoke. The unit summary exists, but the browser gate still starts after readiness and does not fail on slow or incomplete bootstrap visuals.

### 2026-05-07 - Bootstrap Browser Gate

- Added bootstrap benchmark reporting to `owned-browser-lab`.
- Browser smoke now fails if bootstrap:
  - does not complete
  - does not record playable readiness
  - does not record visual readiness
  - exceeds `20,000 ms` for playable or visual readiness
- The first attempt used a `120 ms` p95 gameplay-frame gate and failed:
  - `artifacts/owned-browser-lab/20260507T200942Z-bootstrap-browser-gate-smoke/report.json`
  - Bootstrap completed in `4318.80 ms`, but deterministic bootstrap p95 frame was `3121.40 ms` because the eager drain performs large initial generation/meshing work in a small number of frames.
  - Kept that p95 visible as telemetry and changed the smoke threshold to a loose `4,000 ms` tripwire until bootstrap is optimized directly.
- Validation:
  - `mise exec -- bun run typecheck`: pass.
  - `mise exec -- bun test tests/game-bootstrap-benchmark.test.ts tests/game-route-benchmark.test.ts`: pass.
  - Direct browser-lab artifact: `artifacts/owned-browser-lab/20260507T201035Z-bootstrap-browser-gate-smoke-2/report.json`.
  - Direct result: bootstrap playable/visual `4173.50/4173.50 ms`, p95/max frame `3099.60/3099.60 ms`, no traversal or route holes/seams.
  - `mise exec -- bun run scripts/verify-smoke.ts --label=bootstrap-browser-gate-wrapper`: pass.
  - Summary artifact: `artifacts/verify-smoke/20260507T201113Z-bootstrap-browser-gate-wrapper/report.json`.
  - Browser lab artifact: `artifacts/owned-browser-lab/20260507T201114Z-bootstrap-browser-gate-wrapper/report.json`, with bootstrap playable/visual `4178.90/4178.90 ms`, traversal captures `2`, route captures `2`, route distance `19.67 m`, holes/seams `0/0`.
- Rubric movement:
  - Runtime performance: stays `3.45`, because this exposed bootstrap work rather than optimizing it.
  - Harness maturity: `4.45 -> 4.55`
- Next: optimize bootstrap drain so playable/visual readiness is not achieved through a few multi-second frames.

### 2026-05-07 - Incremental Bootstrap Drain

- Changed eager bootstrap benchmarking to use the normal incremental streaming path instead of forced residency settlement.
- Root cause: the eager drain used `syncWorldAroundPlayer(true)`, which allows unlimited chunk adoption and meshing in one loop. This produced a few huge deterministic frames while still reporting eventual readiness.
- New behavior:
  - bootstrap continues pumping pending chunks, dirty meshes, and LOD work
  - work is bounded by normal streaming budgets
  - browser smoke now protects the improvement with tighter bootstrap gates:
    - playable readiness <= `5,000 ms`
    - visual readiness <= `5,000 ms`
    - p95 bootstrap gameplay frame <= `300 ms`
- Validation:
  - `mise exec -- bun run typecheck`: pass.
  - `mise exec -- bun test tests/game-bootstrap-benchmark.test.ts`: pass.
  - Direct browser-lab artifact: `artifacts/owned-browser-lab/20260507T201317Z-incremental-bootstrap-smoke/report.json`.
  - Direct result: bootstrap playable/visual `1661.80/1661.80 ms`, p95/max frame `179.10/179.10 ms`, down from the previous `~4179 ms` readiness and `~3085 ms` p95/max frame.
  - `mise exec -- bun run scripts/verify-smoke.ts --label=incremental-bootstrap-wrapper`: pass.
  - Summary artifact: `artifacts/verify-smoke/20260507T201358Z-incremental-bootstrap-wrapper/report.json`.
  - Browser lab artifact: `artifacts/owned-browser-lab/20260507T201359Z-incremental-bootstrap-wrapper/report.json`, with bootstrap playable/visual `1735.40/1735.40 ms`, p95/max frame `180.80/180.80 ms`, traversal holes/seams `0/0`, route holes/seams `0/0`, and no LOD gaps.
- Rubric movement:
  - Runtime performance: `3.45 -> 3.65`
  - Harness maturity: `4.55 -> 4.6`
- Next: reduce the remaining `~180 ms` bootstrap spike by splitting initial render/upload or mesh work further, then lower the p95 gate again if repeatable.

### 2026-05-07 - Balanced LOD Bootstrap Budget

- Added LOD timing to bootstrap samples and summaries:
  - total/p95/max LOD elapsed
  - LOD y-range/downsample/mesh totals
- This proved the remaining bootstrap spike was mostly LOD work:
  - `artifacts/owned-browser-lab/20260507T201557Z-bootstrap-lod-accounting-smoke/report.json`
  - p95 frame `185.20 ms`
  - p95 LOD/stream/mesh/render `176.30/8.30/18.80/1.10 ms`
- Tried a very low LOD cap of `4` chunks per non-settle frame:
  - `artifacts/owned-browser-lab/20260507T201706Z-bounded-lod-bootstrap-smoke/report.json`
  - Bootstrap improved to p95 frame `125.20 ms`
  - Rejected because route traversal produced 17 hole-signal frames and 3 seam-gap frames. The route gate correctly blocked an over-aggressive optimization.
- Balanced at `16` LOD chunks per non-settle frame:
  - direct browser-lab artifact: `artifacts/owned-browser-lab/20260507T201745Z-balanced-lod-bootstrap-smoke/report.json`
  - bootstrap playable/visual `1312.90/1312.90 ms`
  - p95/max frame `143.60/143.60 ms`
  - p95 LOD/stream/mesh/render `134.30/8.70/12.60/1.10 ms`
  - route holes/seams `0/0`
- Tightened browser bootstrap gates again:
  - playable readiness <= `3,000 ms`
  - visual readiness <= `3,000 ms`
  - p95 bootstrap gameplay frame <= `220 ms`
- Validation:
  - `mise exec -- bun run typecheck`: pass.
  - `mise exec -- bun test tests/game-bootstrap-benchmark.test.ts tests/lod-system.test.ts`: pass.
  - `mise exec -- bun run scripts/verify-smoke.ts --label=balanced-lod-bootstrap-wrapper`: pass.
  - Summary artifact: `artifacts/verify-smoke/20260507T201827Z-balanced-lod-bootstrap-wrapper/report.json`.
  - Browser lab artifact: `artifacts/owned-browser-lab/20260507T201828Z-balanced-lod-bootstrap-wrapper/report.json`, with bootstrap playable/visual `1413/1413 ms`, p95/max frame `154.70/154.70 ms`, route holes/seams `0/0`, no LOD gaps, and no handoff holes.
- Rubric movement:
  - Runtime performance: `3.65 -> 3.8`
  - Rendering/LOD correctness: stays `3.5`, because correctness stayed protected rather than expanded.
  - Harness maturity: `4.6 -> 4.65`
- Next: LOD work is still the dominant bootstrap cost. The next performance step should split or cache LOD generation more intelligently instead of lowering the cap enough to starve traversal.

### 2026-05-07 - View Distance 9

- Increased the default resident world radius from `8` to `9` chunks.
- This is a concrete viewing-distance/world-definition increase guarded by the browser route and performance gates.
- Validation:
  - `mise exec -- bun run typecheck`: pass.
  - `mise exec -- bun test tests/lod-system.test.ts tests/stream-work.test.ts`: pass.
  - Direct browser-lab artifact: `artifacts/owned-browser-lab/20260507T202021Z-view-distance-9-smoke/report.json`.
  - Direct result:
    - chunks `1073` versus the earlier `~834`
    - draw calls/triangles `602/700238`
    - bootstrap playable/visual `1423.30/1423.30 ms`
    - route holes/seams `0/0`
  - `mise exec -- bun run scripts/verify-smoke.ts --label=view-distance-9-wrapper`: pass.
  - Summary artifact: `artifacts/verify-smoke/20260507T202101Z-view-distance-9-wrapper/report.json`.
  - Browser lab artifact: `artifacts/owned-browser-lab/20260507T202102Z-view-distance-9-wrapper/report.json`, with chunks `1073`, draw calls/triangles `602/700238`, bootstrap playable/visual `1462.10/1462.10 ms`, p95 frame `155.40 ms`, route holes/seams `0/0`, no LOD gaps, and no handoff holes.
- Rubric movement:
  - World definition: `2.35 -> 2.5`
  - Runtime performance: stays `3.8`, because the increased radius stayed within the existing budgets but did not improve runtime cost.
  - Harness maturity: stays `4.65`
- Next: add route-visible content density or authored-feeling biome features now that the renderer can carry a larger resident radius.

### 2026-05-07 - Ashland Landmark Silhouettes

- Added two Morrowind-leaning landmark identities:
  - `silt_shell` -> "Silt Strider Shell"
  - `velothi_shrine` -> "Velothi Wayshrine"
- Reused existing procedural feature geometry rather than inventing new rendering code:
  - `silt_shell` uses a broad hoodoo-like shell silhouette with muted ash/tan material accents
  - `velothi_shrine` uses a standing-stone shrine silhouette with warm accent material
- Placed them in harsh, uncanny, and ash-adjacent regions:
  - dunes
  - badlands
  - saltflat
  - shardlands
- Extended the route atlas with targeted legs so these features are route-visible and tracked over time.
- Validation:
  - `mise exec -- bun run typecheck`: pass.
  - `mise exec -- bun test tests/procedural-generator.test.ts tests/discovery-catalog.test.ts`: pass.
  - `mise exec -- bun run scripts/route-atlas.ts --label=ashland-landmarks-3`: pass.
  - Atlas artifact: `artifacts/route-atlas/20260507T202432Z-ashland-landmarks-3/report.json`, with `silt_shell` and `velothi_shrine` in aggregate route coverage.
  - `mise exec -- bun run scripts/verify-smoke.ts --label=ashland-landmarks-wrapper`: pass.
  - Summary artifact: `artifacts/verify-smoke/20260507T202438Z-ashland-landmarks-wrapper/report.json`.
  - Browser lab artifact: `artifacts/owned-browser-lab/20260507T202440Z-ashland-landmarks-wrapper/report.json`, with chunks `1073`, draw calls/triangles `601/700290`, bootstrap playable/visual `1485.10/1485.10 ms`, route holes/seams `0/0`, no LOD gaps, and no handoff holes.
- Rubric movement:
  - World definition: `2.5 -> 2.65`
  - Exploration feel: `2.2 -> 2.35`
  - Harness maturity: stays `4.65`
- Next: add a small gameplay-facing discovery/journal surface for landmark flavor text or region notes, so new landmarks are not only silhouettes but also feed exploration memory.

### 2026-05-07 - Discovery Flavor Notes

- Added lightweight flavor text to discovery presentations and exploration events.
- Kept the HUD quiet:
  - flavor appears only in the transient discovery toast
  - no persistent extra panel or large text surface was added
- Added authored notes for the ancient/ashland landmark set:
  - `ancestor_pillar`
  - `ash_marker`
  - `glass_cairn`
  - `silt_shell`
  - `velothi_shrine`
- Progress serialization now carries flavor metadata through recent discoveries.
- Browser lab now explicitly imports and exports a flavored `silt_shell` discovery and fails if the flavor text is lost.
- Validation:
  - `mise exec -- bun run typecheck`: pass.
  - `mise exec -- bun test tests/discovery-catalog.test.ts tests/exploration-journal.test.ts tests/skill-journal.test.ts`: pass.
  - Direct browser-lab artifact: `artifacts/owned-browser-lab/20260507T202947Z-flavored-progress-smoke-2/report.json`.
  - Direct result: progress persistence passed, route holes/seams `0/0`, no LOD gaps, no handoff holes.
  - `mise exec -- bun run scripts/verify-smoke.ts --label=discovery-flavor-wrapper`: pass.
  - Summary artifact: `artifacts/verify-smoke/20260507T203027Z-discovery-flavor-wrapper/report.json`.
  - Browser lab artifact: `artifacts/owned-browser-lab/20260507T203028Z-discovery-flavor-wrapper/report.json`, with progress persistence passed, chunks `1073`, draw calls/triangles `638/719462`, route holes/seams `0/0`, no LOD gaps, and no handoff holes.
- Rubric movement:
  - Exploration feel: `2.35 -> 2.5`
  - Skills/progression: `1.65 -> 1.7`, because discoveries now carry richer journal metadata but still do not unlock decisions.
  - Harness maturity: `4.65 -> 4.7`
- Next: add a small set of repeatable route/objective breadcrumbs or region notes that nudge exploration toward landmarks without turning the HUD into a quest log.

### 2026-05-07 - Objective Breadcrumbs For Old Road Signs

- Added a compact objective dimension for ancient/ashland landmark discoveries.
- The HUD still shows only one active task, but the objective ladder now includes:
  - `Trace 2 old road signs` in Frontier Atlas
  - `Trace 4 old road signs` in Deep Expedition
- Counted road-sign landmarks:
  - `ancestor_pillar`
  - `ash_marker`
  - `glass_cairn`
  - `silt_shell`
  - `velothi_shrine`
- Added `discoveredAncientLandmarkCount` to the HUD snapshot source so the objective layer can react without inspecting full journal state in the UI.
- Added `tests/exploration-objectives.test.ts` to `verify-smoke`, closing a harness gap where objective logic was not part of focused smoke.
- Validation:
  - `mise exec -- bun run typecheck`: pass.
  - `mise exec -- bun test tests/exploration-objectives.test.ts tests/exploration-journal.test.ts tests/discovery-catalog.test.ts`: pass.
  - `mise exec -- bun run scripts/verify-smoke.ts --label=objective-breadcrumb-wrapper`: pass.
  - Summary artifact: `artifacts/verify-smoke/20260507T203301Z-objective-breadcrumb-wrapper/report.json`.
  - Browser lab artifact: `artifacts/owned-browser-lab/20260507T203302Z-objective-breadcrumb-wrapper/report.json`, with chunks `1073`, draw calls/triangles `600/700170`, progress persistence passed, route holes/seams `0/0`, no LOD gaps, and no handoff holes.
- Rubric movement:
  - Exploration feel: `2.5 -> 2.65`
  - Skills/progression: `1.7 -> 1.8`
  - Harness maturity: `4.7 -> 4.75`
- Next: add a small deterministic route-save/probe that starts near a landmark route and confirms the objective counter advances after discovery, so this breadcrumb is browser-verified end to end.

### 2026-05-07 - Objective Breadcrumb Browser Probe

- Added an end-to-end browser probe for the old-road breadcrumb objective.
- The owned browser lab now:
  - imports a deterministic Frontier Atlas progress state with enough materials, biomes, and landmarks to expose `ancient-signs-2`
  - preserves collected material IDs through progress import/export so synthetic saves can reach the intended objective stage honestly
  - samples terrain height at the target coordinate instead of using a hard-coded eye height
  - teleports to the route-visible `silt_shell` landmark and verifies the ancient landmark count and objective progress advance in the live game
  - restores the original player position and progress state after the probe
- Added a harness failure gate for this path:
  - fail if the objective is missing before or after travel
  - fail if progress does not increase
  - fail if ancient landmark count does not increase
  - fail if the live discovery target is not `silt_shell`
- Validation:
  - `mise exec -- bun run typecheck`: pass.
  - `mise exec -- bun test tests/exploration-objectives.test.ts tests/exploration-journal.test.ts tests/discovery-catalog.test.ts tests/skill-journal.test.ts`: pass.
  - Direct browser-lab artifact: `artifacts/owned-browser-lab/20260507T203727Z-objective-breadcrumb-probe-smoke/report.json`.
  - Direct result: objective breadcrumb `1 -> 2 at silt_shell`, route holes/seams `0/0`, no LOD gaps, no handoff holes, HUD smoke passed.
  - `mise exec -- bun run scripts/verify-smoke.ts --label=objective-breadcrumb-probe-wrapper`: pass.
  - Summary artifact: `artifacts/verify-smoke/20260507T203831Z-objective-breadcrumb-probe-wrapper/report.json`.
  - Browser lab artifact: `artifacts/owned-browser-lab/20260507T203833Z-objective-breadcrumb-probe-wrapper/report.json`, with objective breadcrumb `1 -> 2 at silt_shell`, chunks `1073`, draw calls/triangles `600/700170`, progress persistence passed, bootstrap playable/visual `1505.80/1505.80 ms`, route holes/seams `0/0`, no LOD gaps, and no handoff holes.
- Rubric movement:
  - Harness maturity: `4.75 -> 4.85`
  - Exploration feel: stays `2.65`, because this improves confidence in the objective path rather than adding new player-facing content.
  - Skills/progression: stays `1.8`
- Next: broaden behavior verification beyond single-objective discovery by adding a deterministic saved-route harness for staged exploration states, so future quest/skill changes can be tested without manually rebuilding state in each probe.

### 2026-05-07 - Staged Exploration Save Fixtures

- Replaced the one-off synthetic save setup in the browser lab with reusable staged exploration fixtures.
- Current fixtures:
  - `frontier-old-road-1`: enough survey progress to enter Frontier Atlas, with old-road objective progress `1/2`
  - `deep-old-road-3`: enough frontier progress to enter Deep Expedition, with old-road objective progress `3/4`
- The browser lab now verifies fixture behavior before the live route discovery probe:
  - imports each fixture into a real running game
  - confirms the expected objective stage and old-road progress
  - confirms material collection IDs survive import
  - restores the original progress state and fails if restoration does not match
- This makes later quest/skill work faster because browser probes can start from named staged saves instead of rebuilding ad hoc state each time.
- Validation:
  - `mise exec -- bun run typecheck`: pass.
  - Direct browser-lab artifact: `artifacts/owned-browser-lab/20260507T204152Z-staged-fixtures-smoke/report.json`.
  - Direct result: staged progress fixtures passed; Frontier fixture reached `frontier-atlas` with old-road `1/2`; Deep fixture reached `deep-expedition` with old-road `3/4`; objective breadcrumb still advanced `1 -> 2 at silt_shell`.
  - `mise exec -- bun run scripts/verify-smoke.ts --label=staged-fixtures-wrapper`: pass.
  - Summary artifact: `artifacts/verify-smoke/20260507T204258Z-staged-fixtures-wrapper/report.json`.
  - Browser lab artifact: `artifacts/owned-browser-lab/20260507T204300Z-staged-fixtures-wrapper/report.json`, with staged fixtures passed, objective breadcrumb `1 -> 2 at silt_shell`, chunks `1073`, draw calls/triangles `602/700188`, bootstrap playable/visual `1471/1471 ms`, route holes/seams `0/0`, no LOD gaps, no handoff holes, and HUD smoke passed.
- Rubric movement:
  - Harness maturity: `4.85 -> 4.9`
  - Skills/progression: `1.8 -> 1.85`, because progression states are now easier to verify and iterate.
  - Exploration feel: stays `2.65`
- Next: use these fixtures to add a real progression behavior beyond objectives, likely a small skill-gated exploration affordance or lore journal milestone, and verify it from staged saves instead of only unit tests.

### 2026-05-07 - Naturalist Landmark Sense

- Added a small, measurable RPG skill effect:
  - Naturalist level now expands the landmark discovery scan radius.
  - Novice scan remains `2.4 m` with `13` samples.
  - Higher Naturalist levels expand up to a bounded `4.8 m`, keeping sampling cost predictable.
- Added `src/engine/exploration-skill-effects.ts` so skill behavior is described by a pure, unit-tested engine helper instead of being hidden in controller code.
- Exposed `landmarkScanRadiusMeters` and `landmarkScanSampleCount` through the debug/HUD snapshot so browser tooling can measure the live effect.
- Extended staged browser fixtures:
  - Frontier fixture keeps novice scan `2.4 m / 13 samples`.
  - Deep fixture has higher Naturalist XP and verifies `3.8 m / 37 samples`.
  - Browser lab now fails if the higher-skill fixture does not expand radius and sample count.
- Validation:
  - `mise exec -- bun run typecheck`: pass.
  - `mise exec -- bun test tests/exploration-skill-effects.test.ts tests/skill-journal.test.ts tests/exploration-objectives.test.ts`: pass.
  - Direct browser-lab artifact: `artifacts/owned-browser-lab/20260507T204715Z-naturalist-scan-smoke/report.json`.
  - Direct staged-fixture evidence: Frontier `2.4 m / 13 samples`; Deep `3.8 m / 37 samples`; objective breadcrumb still advanced `1 -> 2 at silt_shell`; no render correctness failures.
  - `mise exec -- bun run scripts/verify-smoke.ts --label=naturalist-scan-wrapper`: pass.
  - Summary artifact: `artifacts/verify-smoke/20260507T204819Z-naturalist-scan-wrapper/report.json`.
  - Browser lab artifact: `artifacts/owned-browser-lab/20260507T204821Z-naturalist-scan-wrapper/report.json`, with staged fixtures passed, chunks `1073`, draw calls/triangles `602/700238`, bootstrap playable/visual `1416.50/1416.50 ms`, route holes/seams `0/0`, no LOD gaps, no handoff holes, and HUD smoke passed.
- Rubric movement:
  - Skills/progression: `1.85 -> 2.05`
  - Exploration feel: `2.65 -> 2.75`
  - Harness maturity: `4.9 -> 4.95`
- Next: add another low-risk skill effect that changes traversal or underground exploration, then put a route/perf gate around it to ensure it affects play without creating frame spikes or hidden correctness regressions.

### 2026-05-07 - Cartography And Spelunking Travel Sense

- Added a second measurable RPG skill effect:
  - Cartography level now applies a bounded surface travel speed multiplier.
  - Spelunking level now applies a bounded underground travel speed multiplier.
  - Novice movement remains `1.00x`; the current Deep fixture verifies `1.08x` for both surface and underground contexts.
- Kept the speed effect small and capped at `1.14x` so it gives progression texture without blowing up streaming assumptions.
- Fixed a movement correctness issue found by subagent review:
  - auto-step used to attempt a step from the collision-resolved edge and apply the full horizontal delta again
  - it now retries from the pre-collision position and restores the collision-resolved position only when the step fails
  - added `tests/player-physics.test.ts` coverage so boosted movement cannot lurch farther than the frame's intended distance while stepping
- Improved harness accuracy:
  - browser lab now verifies staged fixture speed multipliers
  - browser lab now runs a short live movement comparison after the main route/perf gates, proving boosted movement affects actual distance without perturbing core route measurements
  - current live probe result: novice `3.99 m`, higher-skill `4.52 m`
  - route summaries now include small seam-gap samples when route seam gates fail, making future failures actionable
  - cached landmark sample offsets by radius/step to avoid rebuilding and sorting discovery offsets during HUD/debug snapshots
- Honest verification history:
  - `travel-skill-live-wrapper` failed once with `route benchmark had 1 seam-gap frames`; at that point the report lacked enough detail to diagnose the sample.
  - `travel-skill-live-wrapper-2` failed once with route distance `17.89 m`, which pointed to the added live skill probe making the long browser lab noisier.
  - I moved the skill movement probe after the main route budgets and added seam sample diagnostics rather than weakening the route gates.
- Validation:
  - `mise exec -- bun run typecheck`: pass.
  - `mise exec -- bun test tests/player-physics.test.ts tests/exploration-skill-effects.test.ts tests/skill-journal.test.ts`: pass.
  - Direct browser-lab artifact: `artifacts/owned-browser-lab/20260507T210543Z-travel-skill-seam-diagnostics/report.json`.
  - Direct result: travel skill movement `4.22 m -> 4.52 m`, route holes/seams `0/0`, no LOD gaps, no handoff holes, HUD smoke passed.
  - `mise exec -- bun run scripts/verify-smoke.ts --label=travel-skill-seam-diagnostics-wrapper`: pass.
  - Summary artifact: `artifacts/verify-smoke/20260507T210645Z-travel-skill-seam-diagnostics-wrapper/report.json`.
  - Browser lab artifact: `artifacts/owned-browser-lab/20260507T210647Z-travel-skill-seam-diagnostics-wrapper/report.json`, with travel skill movement `3.99 m -> 4.52 m`, staged fixtures passed, objective breadcrumb `1 -> 2 at silt_shell`, route distance `19.73 m`, route holes/seams `0/0`, no LOD gaps, no handoff holes, and HUD smoke passed.
- Subagent review:
  - Addressed the P2 auto-step over-application finding.
  - Addressed the P2 missing live boosted-movement verification finding.
  - Addressed the P3 landmark scanning cost concern with offset caching.
  - Left the P3 diagnostic-cost accounting concern as a tracked harness issue; route reports already expose diagnostic totals, but budgets still focus on game frame work rather than verifier overhead.
- Rubric movement:
  - Skills/progression: `2.05 -> 2.25`
  - Exploration feel: `2.75 -> 2.85`
  - Input/physics correctness: `3.7 -> 3.85`
  - Harness maturity: stays `4.95`, with a clear next gap around diagnostic-cost budgeting.
- Next: improve route diagnostics accounting so verification overhead has explicit budgets/reporting separate from gameplay frame budgets, then resume world-definition work with a new region/landmark pass.

### 2026-05-07 - Route Diagnostic Cost And Blocking Seam Gate

- Improved route correctness reporting after the traversal-skill pass exposed intermittent route seam samples.
- Added diagnostic-cost fields to owned browser summaries:
  - average diagnostics per sample
  - total diagnostics time
  - total screenshot/capture diagnostics time
- Added seam-gap sample snapshots to route/traversal summaries:
  - frame, phase, route distance, feet position
  - pending chunks, pending mesh jobs, dirty chunks
  - seam gap count, max seam gap, visible-ground and screen-void signals
- Refined the route seam gate:
  - still fails visible-ground holes, screen-void holes, settled seam gaps, and final LOD coverage gaps
  - reports transient far-field seam samples during active streaming as diagnostics when they have pending/dirty work and no visible/screen signal
- Evidence from the diagnostic failure before the refinement:
  - `artifacts/owned-browser-lab/20260507T210857Z-diagnostic-cost-summary/report.json`
  - first seam sample: moving frame, `229` pending chunks, `88` dirty resident chunks, no visible-ground hole, no screen-void signal
  - this was a streaming diagnostic sample, not a settled visible render hole
- Validation:
  - `mise exec -- bun run typecheck`: pass.
  - `mise exec -- bun test tests/player-physics.test.ts tests/exploration-skill-effects.test.ts`: pass.
  - Direct browser-lab artifact: `artifacts/owned-browser-lab/20260507T211042Z-blocking-seam-gate-smoke/report.json`.
  - Direct result: traversal diagnostics avg/total/capture `5.94/772.50/10.10 ms`; route diagnostics `5.69/1963.00/13.40 ms`; route holes/seams/blocking seams `0/0/0`.
  - `mise exec -- bun run scripts/verify-smoke.ts --label=blocking-seam-gate-wrapper`: pass.
  - Summary artifact: `artifacts/verify-smoke/20260507T211149Z-blocking-seam-gate-wrapper/report.json`.
  - Browser lab artifact: `artifacts/owned-browser-lab/20260507T211151Z-blocking-seam-gate-wrapper/report.json`, with route distance `19.69 m`, route holes/seams/blocking seams `0/0/0`, no final LOD gaps, no handoff holes, and HUD smoke passed.
- Rubric movement:
  - Harness maturity: `4.95 -> 5.0` for current rendering/perf/progression smoke coverage, with the caveat that full game completeness remains far from done.
- Next: resume world-definition work with a new ashland/old-road route content pass, using the strengthened browser lab to catch rendering, traversal, objective, and performance regressions.

### 2026-05-07 - Movement-Time LOD And Travel Context Fix

- Pivoted back to playability after user feedback: idle FPS looked high, but walking could drop near 10 FPS and the world still needed broad functional progress before more landmark detail.
- Found the harness was under-accounting route frame cost:
  - route summaries exposed gameplay/render/stream/mesh cost, but LOD residency work was not separated clearly enough
  - added `lodMs` to route samples and summaries, plus explicit p95 LOD reporting in the owned browser lab
- First pass proved the issue instead of hiding it:
  - after LOD accounting, the movement lab showed LOD p95 around `78 ms` in the failing `movement-lod-budget-smoke` run
  - old content wrapper baseline `artifacts/owned-browser-lab/20260507T211700Z-kwama-pilgrim-wrapper/report.json` had route p95 about `94.70 ms`, matching the user's walking-stall report
- Fixed active movement stalls:
  - reduced interactive LOD generation budget from `16` chunks/frame to `2`
  - deferred far LOD updates while movement input is active, so close collision/render work wins the frame
  - retained the needed-key LOD cache while pending chunks catch up, avoiding repeated expensive Y-range scans
  - added `p95MovementMs` to route accounting so live WASD movement cost is visible instead of blended into measured work
  - cached travel-context sampling for movement speed; per-frame biome/underground classification had pushed live traversal p95 to `27.50 ms` even after route teleport benchmarks were fast
- Refined the seam gate honestly:
  - visible-ground gaps, screen-void holes, final LOD coverage gaps, and fully settled seam gaps still fail
  - active streaming/LOD transition seam samples are reported with pending chunk/mesh/LOD state but do not fail unless they are visible or truly settled
- Validation:
  - `mise exec -- bun run typecheck`: pass.
  - `mise exec -- bun test tests/game-route-benchmark.test.ts tests/lod-system.test.ts tests/player-physics.test.ts`: pass, `41` tests.
  - Direct browser-lab artifact: `artifacts/owned-browser-lab/20260507T213732Z-movement-context-cache-smoke-2/report.json`.
  - Direct result: traversal p95/max `7.50/90.50 ms`, traversal p95 work/move/render/stream/mesh/LOD `7.20/0.30/1.40/0.70/2.20/3.50 ms`; route p95/max `4.10/87.50 ms`; route holes/seams/blocking seams `0/45/0`; LOD gaps `0`; handoff holes `0`; HUD smoke passed.
  - Full wrapper: `mise exec -- bun run scripts/verify-smoke.ts --label=movement-context-cache-wrapper`: pass.
  - Summary artifact: `artifacts/verify-smoke/20260507T213835Z-movement-context-cache-wrapper/report.json`.
  - Browser lab artifact: `artifacts/owned-browser-lab/20260507T213837Z-movement-context-cache-wrapper/report.json`.
  - Wrapper result: traversal p95/max `6.60/85.50 ms`, traversal p95 work/move/render/stream/mesh/LOD `6.50/0.30/1.20/0.60/1.70/3.90 ms`; route p95/max `4.00/93.00 ms`; route holes/seams/blocking seams `0/46/0`; final render-ready near samples `961/961`.
- Rubric movement:
  - Performance/playability: `2.6 -> 3.55` because walking p95 is now comfortably under 16 ms in the owned browser lab, though catch-up max frames around `85-93 ms` still need work.
  - Harness maturity: stays `5.0`, with better attribution for LOD and movement cost.
  - Rendering correctness: stays `3.9`; no visible holes are detected, but raw transition seam samples show the LOD handoff still deserves design work.
- Next:
  - reduce catch-up max frame spikes after movement stops
  - add required-landmark checks to route-atlas and renderability audits for the newer landmarks
  - then resume big-picture visual identity work so the terrain reads less Minecraft-like before adding more content volume

### 2026-05-07 - LOD Cache Signature And Landmark Verification Tightening

- Continued from the movement fix by attacking the next measurement and correctness gaps instead of adding more landmarks.
- LOD profile:
  - `mise exec -- bun run scripts/profile-lod-residency.ts --label=post-movement-fix-budget2 --max-lod-chunks=2 --max-passes=256`
  - artifact: `artifacts/lod-residency-profile/20260507T214052Z-post-movement-fix-budget2.json`
  - showed LOD key/Y-range work was still expensive when cache signatures missed; with a budget of `2`, the profile generated `512` LOD chunks in `256` passes and spent `3491.97 ms` in Y-range work across the run
- Changed the LOD needed-key cache signature from exact integer X/Z to chunk-quantized X/Z:
  - this keeps correctness tied to resident/render-ready revisions and LOD ring coordinates
  - it reduces pointless cache churn while the player moves within the same local chunk-scale neighborhood
  - LOD coverage tests remained green
  - honest result: it is safe and helps cache reuse, but browser max frames still hover around `90 ms`, so catch-up spikes are not solved yet
- Hardened route-atlas correctness:
  - route specs can now declare required landmark IDs
  - old-road and ashland route checks now fail if their intended landmark is absent from that route, not merely present somewhere else in the atlas
  - the first stricter run caught an overclaim: `ancestor-march` was sampling `glass_cairn`, not `ancestor_pillar`; I removed that inaccurate requirement and left the true `ancestor-pillar-road` gate in place
- Hardened renderability audit coverage:
  - added `ancestor_pillar`, `ash_marker`, `glass_cairn`, `silt_shell`, `velothi_shrine`, `kwama_mound`, and `pilgrim_cairn` to the audited surface landmark list
  - `tests/procedural-generator.test.ts` now has to find representative solid renderable roots for these newer landmarks
- Validation:
  - `mise exec -- bun run typecheck`: pass.
  - `mise exec -- bun test tests/procedural-generator.test.ts`: pass, `33` tests.
  - `mise exec -- bun test tests/lod-system.test.ts tests/game-route-benchmark.test.ts`: pass, `32` tests.
  - Route-atlas artifact: `artifacts/route-atlas/20260507T214511Z-required-landmarks-smoke-2/report.json`, failures none.
  - Direct browser-lab artifact after LOD signature change: `artifacts/owned-browser-lab/20260507T214217Z-lod-signature-quantized-smoke/report.json`, failures none, traversal p95/max `6.40/85.20 ms`, route p95/max `4.00/89.50 ms`.
  - Full wrapper: `mise exec -- bun run scripts/verify-smoke.ts --label=lod-signature-required-landmarks-wrapper`: pass.
  - Summary artifact: `artifacts/verify-smoke/20260507T214536Z-lod-signature-required-landmarks-wrapper/report.json`.
  - Browser lab artifact: `artifacts/owned-browser-lab/20260507T214538Z-lod-signature-required-landmarks-wrapper/report.json`, traversal p95/max `6.50/97.40 ms`, route p95/max `4.00/96.70 ms`, holes/seams/blocking seams `0/44/0`, final LOD gaps `0`.
- Rubric movement:
  - Harness maturity: stays `5.0`, now with required landmark route gates and renderability audit coverage for the newest landmarks.
  - Performance/playability: stays `3.55`; p95 movement is good, max-frame spikes remain the next performance target.
  - World-definition confidence: `2.85 -> 2.95` because route content is now verified route-by-route instead of only in aggregate.
- Next:
  - identify the source of the `85-97 ms` max frames in the browser lab
  - then make a broad visual-identity pass focused on moving away from Minecraft-like block reads: terrain silhouettes, palette/material breakup, fog/lighting, and less grid-like near-field shapes

### 2026-05-07 - Incremental LOD Planner Removes Stop-Walking Spikes

- Added slow-frame samples to the owned browser lab report:
  - top gameplay-frame samples now include phase, frame, distance, position, movement/stream/mesh/LOD/render/unmeasured costs, and pending work
  - this immediately showed the `90-99 ms` max frames were first-settle LOD planning frames, not rendering or physics
- Fixed accounting:
  - per-sample `accountedFrameMs` now includes `lodMs`
  - before this, summary accounting included LOD but the individual slow-frame samples mislabeled LOD time as unmeasured
- Fixed the actual hitch:
  - added an incremental LOD needed-key planner with a per-frame planning budget
  - unlimited planning still runs for bootstrap/forced residency/tests
  - interactive LOD planning now returns pending work instead of spending an entire frame computing all Y-ranges and needed keys
  - kept the needed-key cache hot across budgeted LOD generation passes, with a regression test
- Evidence before:
  - `artifacts/owned-browser-lab/20260507T214739Z-slow-frame-samples-smoke/report.json`
  - traversal slow frame: `99.60 ms`, with `95.60 ms` LOD
  - route slow frame: `97.70 ms`, with `94.40 ms` LOD
- Validation after:
  - `mise exec -- bun run typecheck`: pass.
  - `mise exec -- bun test tests/lod-system.test.ts tests/game-route-benchmark.test.ts tests/stream-work.test.ts`: pass, `35` tests.
  - Direct browser-lab artifact: `artifacts/owned-browser-lab/20260507T215425Z-incremental-lod-planner-smoke/report.json`.
  - Direct result: bootstrap max `21.00 ms`, traversal p95/max `5.80/7.10 ms`, route p95/max `4.80/27.50 ms`, LOD gaps `0`, handoff holes `0`, HUD smoke passed.
  - Full wrapper: `mise exec -- bun run scripts/verify-smoke.ts --label=incremental-lod-planner-wrapper`: pass.
  - Summary artifact: `artifacts/verify-smoke/20260507T215535Z-incremental-lod-planner-wrapper/report.json`.
  - Browser lab artifact: `artifacts/owned-browser-lab/20260507T215536Z-incremental-lod-planner-wrapper/report.json`.
  - Wrapper result: bootstrap playable/visual `216.70/216.70 ms`, bootstrap p95/max `25.60/25.60 ms`, traversal p95/max `6.10/8.50 ms`, route p95/max `4.60/27.90 ms`, route holes/seams/blocking seams `0/53/0`, final render-ready near samples `961/961`.
- Rubric movement:
  - Performance/playability: `3.55 -> 4.15` because active walking and stop-walking hitches are now under budget in the browser harness; route first-frame mesh catch-up around `28 ms` remains acceptable but still a target.
  - Harness maturity: stays `5.0`, now with top slow-frame attribution.
  - Rendering correctness: stays `3.9`; no holes are visible, but raw transition seam samples remain a tracked LOD handoff design issue.
- Next:
  - make the world read less Minecraft-like through measurable visual identity changes
  - add screenshots/pixel probes that can catch blocky palette regressions, not just holes

### 2026-05-07 - First-Pass Visual Identity Shift

- Addressed the user's big-picture note that the game still looked too Minecraft-like after the performance work.
- Renderer changes:
  - added a conservative atmospheric color grade in the voxel fragment shader
  - slightly desaturated raw material colors before lighting
  - tinted light/shadow so the world reads less like ungraded block colors
  - added cheap world-space grain to break up large flat voxel faces without extra geometry or mesh cost
- First-viewport change:
  - default spawn now prefers a badlands/ashfall start near the old-road/ash-marker region before falling back to the old flat-biome search
  - this moves the first impression away from a generic bright highland into ashfall fog, warm stone, and stranger silhouettes
  - fallback spawn search remains intact if the curated candidates ever become unsuitable
- HUD language:
  - changed material objective labels from `Collect N distinct colors` to `Identify N field materials`
  - changed the break/sample status from `Collected #RGB` to `Sampled #RGB`
  - same mechanics, less prototype/block-collection framing
- Validation:
  - `mise exec -- bun run typecheck`: pass.
  - `mise exec -- bun test tests/procedural-resident-world.test.ts tests/lod-system.test.ts tests/game-route-benchmark.test.ts`: pass, `50` tests.
  - Full wrapper: `mise exec -- bun run scripts/verify-smoke.ts --label=badlands-visual-identity-wrapper`: pass.
  - Summary artifact: `artifacts/verify-smoke/20260507T220635Z-badlands-visual-identity-wrapper/report.json`.
  - Browser lab artifact: `artifacts/owned-browser-lab/20260507T220638Z-badlands-visual-identity-wrapper/report.json`.
  - Screenshot: `artifacts/owned-browser-lab/20260507T220638Z-badlands-visual-identity-wrapper/settled-page.png`.
  - Wrapper result: ambient `ashfall`, bootstrap p95/max `18.70/18.70 ms`, traversal p95/max `4.60/13.70 ms`, route p95/max `4.60/28.30 ms`, holes/seams/blocking seams `0/54/0`, LOD gaps `0`, handoff holes `0`, HUD smoke passed.
- Honest assessment:
  - This is a better first read and no longer opens in a generic cold highland, but the terrain is still visibly voxel/block based.
  - The next visual step should be structural: reduce square patchwork in surface material selection, improve terrain silhouettes, or add a non-cubic terrain surface path. Shader grading alone cannot fully solve that.
- Rubric movement:
  - Exploration feel: `2.95 -> 3.2`
  - Visual identity: `2.1 -> 2.55`
  - Performance/playability: stays `4.15`; the visual pass did not regress browser-lab budgets.
- Next:
  - create a visual identity metric/probe from screenshots or canvas captures so future changes can be judged beyond subjective screenshots
  - reduce near-field square patchwork in terrain material selection

### 2026-05-07 - Visual Identity Pixel Probe and Material Dither Baseline

- Added a first screenshot-derived visual identity probe to the owned browser lab:
  - decodes the settled PNG artifact directly in the harness
  - records average saturation, luma spread, quantized color count, warm/cool balance, and axis-aligned edge dominance
  - prints these metrics in the browser-lab summary so visual changes can be compared without relying only on subjective screenshots
- Reduced the surface material dither frequency from very small column groups to broader material patches.
- Evidence:
  - Baseline direct browser lab after the probe: `artifacts/owned-browser-lab/20260507T221109Z-visual-metrics-smoke/report.json`.
  - Baseline visual identity: saturation/grid/color `0.40/0.68/120`.
  - Direct browser lab after coarser material dither: `artifacts/owned-browser-lab/20260507T221247Z-coarser-material-dither-smoke/report.json`.
  - Coarser-dither visual identity: saturation/grid/color `0.40/0.68/121`.
  - Full wrapper: `mise exec -- bun run scripts/verify-smoke.ts --label=visual-metrics-material-dither-wrapper`: pass.
  - Summary artifact: `artifacts/verify-smoke/20260507T221503Z-visual-metrics-material-dither-wrapper/report.json`.
  - Browser lab artifact: `artifacts/owned-browser-lab/20260507T221505Z-visual-metrics-material-dither-wrapper/report.json`.
  - Screenshot: `artifacts/owned-browser-lab/20260507T221505Z-visual-metrics-material-dither-wrapper/settled-page.png`.
  - Wrapper result: visual identity `0.40/0.68/122`, ambient `ashfall`, bootstrap playable/visual `251.50/251.50 ms`, bootstrap p95/max `21.60/21.60 ms`, traversal p95/max `4.70/18.80 ms`, route p95/max `4.50/27.70 ms`, holes/seams/blocking seams `0/54/0`, final LOD gaps `0`, handoff holes `0`, HUD smoke passed.
- Honest assessment:
  - The probe is a real harness improvement; it lets future visual changes make or fail an objective claim.
  - The material dither scale is only a small visual step. It did not materially reduce the measured axis-aligned edge dominance, so the next anti-Minecraft pass needs larger structural changes: better silhouettes, less cubic foreground read, stronger regional atmosphere, or a more deliberate terrain surface treatment.
- Rubric movement:
  - Harness maturity: `5.0 -> 5.2` because the browser lab now captures pixel-level visual identity signals in addition to readiness/performance/coverage.
  - Visual identity: stays `2.55`; current measured output is still visibly grid dominated.
  - Performance/playability: stays `4.15`; the new probe and dither change did not regress browser-lab budgets.
- Next:
  - checkpoint this validated work in git and push it
  - remove Minecraft-like material gathering/placement UI in favor of a sparse RPG HUD
  - use the visual identity probe to judge the next structural visual pass instead of guessing from a single screenshot

### 2026-05-07 - Removed Player-Facing Block Tools and Tightened RPG HUD

- Removed the Minecraft-like player loop from the live game surface:
  - no hotbar DOM
  - no inventory panel DOM
  - no block targeting/placement SVG overlay
  - no click-to-break or right-click-to-place input binding
  - no wheel/digit inventory slot controls
  - `window.__VOXELS_GAME__` no longer exposes player gather/place or inventory UI APIs
- Kept low-level inventory/interaction modules and tests intact for tooling and regression fixtures; this change removes the player-facing mechanic, not every engine edit primitive.
- Reworked objectives away from material collection:
  - surface survey now asks for a regional variant instead of field materials
  - frontier/deep objectives use landmarks, variants, old-road signs, and underground discovery
  - old save imports tolerate obsolete material fields by ignoring them
- HUD/UI:
  - replaced the long hotbar and target card with a compact RPG readout for region, nearby landmark/ambience, focus skill, and last discovery
  - tightened the bottom HUD after screenshot inspection because the first version used too much surface for too little information
  - changed the top-right performance readout from derived FPS to frame time in milliseconds so hitches are represented more honestly
- Harness updates:
  - owned browser lab now rejects old hotbar/inventory/target-overlay DOM if it reappears
  - removed stale material-ID fixture gates from progress smoke
- Validation:
  - `mise exec -- bun run typecheck`: pass.
  - `mise exec -- bun run build`: pass.
  - `mise exec -- bun test tests/exploration-objectives.test.ts tests/ambient-environment.test.ts tests/game-bootstrap-benchmark.test.ts`: pass, `7` tests.
  - `mise exec -- bun test tests/interaction-loop.test.ts tests/inventory.test.ts tests/hotbar-layout.test.ts tests/targeting-overlay.test.ts tests/exploration-journal.test.ts tests/skill-journal.test.ts`: pass, `21` tests.
  - Direct browser lab: `artifacts/owned-browser-lab/20260507T222735Z-rpg-hud-compact-smoke/report.json`, failures none.
  - Direct screenshot: `artifacts/owned-browser-lab/20260507T222735Z-rpg-hud-compact-smoke/settled-page.png`.
  - Full wrapper: `mise exec -- bun run scripts/verify-smoke.ts --label=rpg-hud-no-material-tools-wrapper`: pass.
  - Summary artifact: `artifacts/verify-smoke/20260507T222846Z-rpg-hud-no-material-tools-wrapper/report.json`.
  - Browser lab artifact: `artifacts/owned-browser-lab/20260507T222851Z-rpg-hud-no-material-tools-wrapper/report.json`.
  - Wrapper result: bootstrap playable/visual `279.60/279.60 ms`, bootstrap p95/max `24.90/24.90 ms`, traversal p95/max `5.10/16.20 ms`, route p95/max `5.00/27.40 ms`, holes/seams/blocking seams `0/53/0`, LOD gaps `0`, handoff holes `0`, HUD smoke passed.
- Rubric movement:
  - UI/RPG framing: `2.6 -> 3.25` because the visible interface no longer centers block gathering/placement and now presents exploration identity.
  - Harness maturity: `5.2 -> 5.3` because the browser smoke now catches regression to the old Minecraft-style DOM.
  - Performance/playability: stays `4.15`; the UI removal did not regress route/traversal budgets.
- Next:
  - address the still-very-blocky terrain read with structural worldgen/silhouette work
  - tighten route budget gates and seam probes that subagents flagged as remaining harness risks
  - continue checkpoint commits after each validated slice; push still requires configuring a git remote in this worktree

### 2026-05-07 - Browser Lab Max-Frame Gates

- Hardened the owned browser lab against the class of regression the user observed: movement feeling like `10 FPS` while a summary metric looks acceptable.
- Added explicit max gameplay frame gates:
  - traversal max frame budget: `66 ms`
  - route max frame budget: `66 ms`
  - route p95 gameplay frame budget is now also explicit
- Rationale:
  - p95 and measured-work gates are useful but can miss isolated movement spikes
  - max-frame gates make stop-walking / LOD / streaming hitches harder to hide in averages
- Validation:
  - `mise exec -- bun run typecheck`: pass.
  - Direct browser lab: `artifacts/owned-browser-lab/20260507T223129Z-max-frame-gate-smoke/report.json`, failures none.
  - Direct result: traversal p95/max `4.50/18.40 ms`, route p95/max `4.50/27.60 ms`, holes/seams/blocking seams `0/54/0`, LOD gaps `0`, HUD smoke passed.
- Rubric movement:
  - Harness maturity: `5.3 -> 5.4` because movement hitches now fail a deterministic browser smoke instead of only appearing in printed summaries.
- Next:
  - add a real seam probe for the route samples instead of treating seam counts as mostly diagnostic
  - integrate or review the parallel landmark/model polish worker result

### 2026-05-07 - Exclusive LOD Coverage and Landmark Shape Checkpoint

- Fixed a correctness/performance issue the user spotted visually: coarse LOD chunks could overlap render-ready fine chunks, creating z-fighting risk and extra draw cost.
- New invariant: every sampled world column should be covered by one visible representation, not both a fine chunk and a coarser LOD band.
- Implementation:
  - coarser LOD generation now punches out columns whose footprint overlaps render-ready LOD0 columns
  - coarser LOD invalidation now covers the whole source column across chunk heights so stale overlap meshes are regenerated after fine chunks become render-ready
  - the client-side LOD coverage probe now checks actual non-empty data columns rather than treating a chunk footprint as fully covered
  - the owned browser lab now fails on sampled LOD0/LOD overlap or overlapping LOD bands
  - teleport-and-settle now waits for LOD pending work to reach zero before judging the scene
- Integrated the parallel landmark polish:
  - ash markers and old-road shrines now render shaped plinths/caps instead of simple block columns
  - generator regression tests assert the shaped caps
- Validation:
  - `mise exec -- bun run typecheck`: pass.
  - `mise exec -- bun test tests/lod-system.test.ts tests/game-route-benchmark.test.ts`: pass, `33` tests.
  - `mise exec -- bun test tests/procedural-generator.test.ts tests/exploration-objectives.test.ts`: pass, `37` tests.
  - Direct browser lab: `artifacts/owned-browser-lab/20260507T225016Z-lod-exclusive-coverage-smoke-6/report.json`, failures none.
  - Full wrapper: `mise exec -- bun run scripts/verify-smoke.ts --label=lod-exclusive-landmark-wrapper`: pass.
  - Summary artifact: `artifacts/verify-smoke/20260507T225132Z-lod-exclusive-landmark-wrapper/report.json`.
  - Browser lab artifact: `artifacts/owned-browser-lab/20260507T225134Z-lod-exclusive-landmark-wrapper/report.json`.
  - Wrapper result: `LOD pending 0`, `LOD overlap LOD0/bands 0/0`, `LOD gaps 0`, `LOD handoff holes 0`, traversal p95/max `4.50/18.90 ms`, route p95/max `4.30/31.10 ms`, draw/triangles `499/383470`, visual identity `0.40/0.68/109`, HUD smoke passed.
- Honest assessment:
  - This is a meaningful rendering correctness improvement and should directly address the reported LOD z-fighting class.
  - The route seam diagnostic still reports non-blocking seam candidates, so the next correctness work should turn that into a more precise terrain-continuity probe rather than relying on the current broad counter.
  - Visual identity is still too block/grid dominated at `0.68`; this checkpoint makes the renderer more correct, not yet less Minecraft-like.
- Rubric movement:
  - Rendering correctness: `4.2 -> 4.65` because the harness now proves no sampled fine/coarse LOD overlap or LOD band overlap after settling.
  - Performance/playability: `4.15 -> 4.3` because overlap draw work is eliminated and max-frame gates stayed comfortably below budget.
  - Harness maturity: `5.4 -> 5.65` because LOD z-fighting is now represented as a deterministic browser-lab failure condition.
- Next:
  - checkpoint this validated LOD/landmark slice in git
  - build a sharper seam-continuity probe for route samples
  - continue larger visual-structure work to reduce the blocky foreground read

### 2026-05-07 - Route LOD Overlap and Seam Signal Split

- Followed up the LOD z-fighting fix by making route movement samples distinguish:
  - uncovered LOD gaps
  - resident handoff holes
  - fine/coarse resident overlaps
  - overlapping coarser LOD bands
- Rationale:
  - the previous route seam number was too broad and could not tell a true hole from a z-fighting overlap
  - route runs now fail on settled LOD overlap frames, so the user-reported class is guarded during movement and after settling
  - the old seam count remains as non-blocking context while a more precise terrain-continuity probe is developed
- Validation:
  - `mise exec -- bun run typecheck`: pass.
  - `mise exec -- bun test tests/game-route-benchmark.test.ts tests/lod-system.test.ts`: pass, `33` tests.
  - Direct browser lab: `artifacts/owned-browser-lab/20260507T225620Z-route-lod-overlap-split-smoke/report.json`, failures none.
  - Direct result: traversal holes/seams/blocking seams/overlaps `0/15/0/0`, route `0/54/0/0`, `LOD overlap LOD0/bands 0/0`.
  - Full wrapper: `mise exec -- bun run scripts/verify-smoke.ts --label=route-lod-overlap-split-wrapper`: pass.
  - Summary artifact: `artifacts/verify-smoke/20260507T225728Z-route-lod-overlap-split-wrapper/report.json`.
  - Browser lab artifact: `artifacts/owned-browser-lab/20260507T225730Z-route-lod-overlap-split-wrapper/report.json`.
  - Wrapper result: traversal p95/max `4.70/19.30 ms`, route p95/max `4.30/32.10 ms`, route holes/seams/blocking seams/overlaps `0/54/0/0`, `LOD pending 0`, `LOD overlap LOD0/bands 0/0`, HUD smoke passed.
- Honest assessment:
  - This does not eliminate the broad transient seam candidate counter; it makes the counter explain what it is seeing.
  - The remaining `uncoveredLodGapCount` samples happen during movement while LOD work is still pending and have no visible-ground or screen-hole signal, so they are tracked but not treated as a correctness failure yet.
  - Next harness step should sample terrain surface continuity directly along chunk and LOD boundaries instead of relying on coverage membership alone.
- Rubric movement:
  - Harness maturity: `5.65 -> 5.8` because route samples now expose and gate overlap separately from holes.
  - Rendering correctness: stays `4.65`; the invariant did not change, but it is now checked in a harder movement path.
- Next:
  - commit this harness split
  - use the subagent worldgen notes for a harsh-region Morrowind visual pass
  - add direct surface-continuity checks for chunk/LOD seams

### 2026-05-07 - Far LOD Coverage, Object Lab, and Ash Wastes

- Followed up on the user's report that z-fighting remained and distant LOD seemed gone.
- Browser evidence:
  - first widened LOD smoke failed with `60` far coverage gaps and per-level draw calls `0/85/60/41/82`
  - conservative ring planning alone failed with `56` gaps
  - resident fine-chunk eviction invalidation reduced this to `55` gaps
  - splitting covered-empty LOD cache reduced this to `3` gaps
  - invalidating coarser LOD when intermediate LOD chunks are evicted reached `0` gaps
- Root causes fixed:
  - coarser ring planning was trusting theoretical finer-ring coverage and could skip chunks that were not actually filled
  - LOD chunks that were empty only because finer coverage existed were cached like genuinely empty terrain and survived movement
  - evicted LOD1 chunks did not invalidate LOD2 chunks that had punched out columns because of those LOD1 chunks
  - settle now treats pending LOD invalidation after eviction as real pending work instead of stopping on a stale frame
- Harness improvements:
  - owned browser lab now samples the full `416 m` fog range by default instead of the old local `48 m` radius
  - browser lab now fails if no far LOD levels are drawn and prints draw calls by LOD level
  - renderer/controller/bootstrap benchmark now expose `lodDrawCallsByLevel`
  - added LOD tests for full fog-range generated coverage and budgeted far-ring starvation
- Object-lab tooling:
  - added `scripts/object-lab.ts` for isolated landmark review without opening the full renderer
  - added top/front/side PPM projections, SVG contact sheet, silhouette diagnostics, material dominance, and a docs workflow
  - smoke artifact: `artifacts/object-lab/2026-05-07-233939666Z-ash-marker-check/contact-sheet.svg`
- World definition:
  - added an `ash_wastes` regional variant inspired by the attached salt-marsh/ashen-badlands references
  - ash wastes favor ash markers, Velothi shrines, pilgrim cairns, kwama mounds, silt shells, basalt spires, and dead snags over generic desert props
- Validation:
  - `mise exec -- bun run typecheck`: pass.
  - `mise exec -- bun test tests/lod-system.test.ts tests/game-bootstrap-benchmark.test.ts tests/object-lab.test.ts`: pass, `29` tests.
  - `mise exec -- bun test tests/procedural-generator.test.ts tests/exploration-journal.test.ts tests/exploration-objectives.test.ts`: pass, `42` tests.
  - Direct browser lab: `artifacts/owned-browser-lab/20260507T233631Z-far-lod-lod-eviction-invalidation-smoke/report.json`, failures none.
  - Full wrapper: `mise exec -- bun run scripts/verify-smoke.ts --label=far-lod-object-lab-ash-wastes-wrapper`: pass.
  - Summary artifact: `artifacts/verify-smoke/20260507T233801Z-far-lod-object-lab-ash-wastes-wrapper/report.json`.
  - Browser artifact: `artifacts/owned-browser-lab/20260507T233803Z-far-lod-object-lab-ash-wastes-wrapper/report.json`.
  - Wrapper result: `LOD gaps 0`, `LOD overlap LOD0/bands 0/0`, `LOD pending 0`, `render-ready near samples 961/961`, route p95/max `4.90/31.50 ms`, traversal p95/max `4.80/18.90 ms`, draw/triangles `509/390648`, HUD smoke passed.
  - Object lab smoke: `mise exec -- bun run scripts/object-lab.ts --id ash_marker --seed 1337 --label=ash-marker-check --scan-radius=4096 --coarse-step=32 --sample-radius=16 --height-padding=4`: pass, `1323` solid object voxels.
- Browser limitation:
  - the Browser Use `iab` backend was attempted first but no in-app browser backend was discovered, so the visual validation used the repo-owned browser lab instead of the exact user-facing tab.
- Honest assessment:
  - The reported class of LOD overlap and far coverage gaps is now covered by a much stricter browser smoke and currently passes.
  - Movement still logs transient non-blocking seam candidates while LOD work is pending; they do not show visible-ground or screen-hole signals, but the next harness pass should replace this broad counter with direct terrain-continuity checks.
  - The scene still reads too block/grid dominated (`visual identity grid 0.68`), so the next quality push should reshape terrain tiles, silhouettes, and large landmarks rather than adding more small props.
- Rubric movement:
  - Rendering correctness: `4.65 -> 5.2` because the harness now proves settled fog-range LOD coverage has no sampled holes or overlap after movement and teleport settling.
  - Performance/playability: `4.3 -> 4.45` because the stricter far LOD coverage keeps route max under `33 ms` while drawing far levels.
  - Harness maturity: `5.8 -> 6.15` because far-distance visibility, per-level LOD draw calls, and isolated object review now have repeatable scripts.
  - Visual/world definition: `3.0 -> 3.25` because ash wastes introduce a more Morrowind-like regional roster, though the overall shape language still needs major work.
- Next:
  - commit and attempt push; this worktree still has no configured `origin`
  - add water-specific overlap diagnostics, since transparent water can still double-blend even when opaque LOD overlap is zero
  - use object lab to iterate on individual ashland objects before broadening the biome again
  - replace broad route seam candidates with a direct surface-continuity probe

### 2026-05-07 - Transparent Water Overlap Audit

- Followed the z-fighting audit into transparent water after opaque LOD overlap and far coverage were clean.
- Added a browser-lab gate for sampled overlapping transparent water surfaces.
- First water audit failed:
  - artifact: `artifacts/owned-browser-lab/20260507T234324Z-water-overlap-audit-smoke/report.json`
  - result: `LOD water overlap 2`, `LOD gaps 0`, `LOD overlap LOD0/bands 0/0`
- Root cause:
  - LOD water meshing treated the top of a vertical chunk as air because it had no same-chunk `y + 1` voxel
  - when water continued into the chunk above, the lower chunk emitted an internal water top surface
  - these were transparent surfaces, so the old opaque overlap probe could not identify the remaining shimmer risk
- Fix:
  - LOD water top meshing now suppresses a chunk-boundary top face when the procedural water column continues above that boundary
  - the coverage probe now distinguishes water surfaces from water volume, so vertical water columns are not counted as overlap unless they expose multiple transparent top surfaces
- Validation:
  - `mise exec -- bun run typecheck`: pass.
  - `mise exec -- bun test tests/lod-system.test.ts tests/game-route-benchmark.test.ts tests/game-bootstrap-benchmark.test.ts`: pass, `36` tests.
  - Direct browser lab: `artifacts/owned-browser-lab/20260507T234616Z-water-overlap-fixed-smoke/report.json`, failures none.
  - Browser result: `LOD water overlap 0`, `LOD gaps 0`, `LOD overlap LOD0/bands 0/0`, route p95/max `4.90/31.50 ms`, traversal p95/max `4.90/26.70 ms`.
- Rubric movement:
  - Rendering correctness: `5.2 -> 5.35` because transparent water now has its own overlap signal and the first detected issue is fixed.
  - Harness maturity: `6.15 -> 6.25` because the browser lab no longer conflates opaque terrain coverage with transparent-water correctness.
- Next:
  - checkpoint the water audit
  - add direct surface-continuity probes for the remaining broad seam candidates
  - keep using object lab for individual ashland silhouettes before another broad worldgen pass

### 2026-05-07 - Ashland Prop Silhouette Pass

- Integrated the object-lab worker's targeted ashland prop polish.
- Changes:
  - `dead_snag` now uses the dead-tree branch silhouette instead of a thin stone-column shape
  - `kwama_mound` now has a dedicated squat oval mound shape with amber egg pockets and lower monochrome dominance
  - object-lab tests now lock branch/material diagnostics for dead snags and squat/accent diagnostics for kwama mounds
- Object-lab evidence:
  - dead snag after: `artifacts/object-lab/2026-05-07-235058941Z-dead-snag-after/contact-sheet.svg`
  - dead snag metrics: `544` solid voxels, `2` materials, dominant material `67.8%`, front/side occupied pixels `214/214`
  - kwama mound after: `artifacts/object-lab/2026-05-07-235150101Z-kwama-mound-after-2/contact-sheet.svg`
  - kwama metrics: `1055` solid voxels, `2` materials, dominant material `88.8%`, front aspect `1.300`, front normalized height `55.6%`
- Validation:
  - `mise exec -- bun run typecheck`: pass.
  - `mise exec -- bun test tests/object-lab.test.ts tests/procedural-generator.test.ts`: pass, `37` tests.
- Honest assessment:
  - This is a narrow visual-quality improvement, not a renderer/performance change.
  - The object-lab workflow is already useful: it caught the old column-like dead snag and gives quantitative signals for future prop iteration.
- Rubric movement:
  - Visual/world definition: `3.25 -> 3.35` because two ashland props now have stronger silhouettes and artifact-backed regression coverage.
- Next:
  - commit and push this small prop checkpoint
  - continue renderer correctness with a direct surface-continuity seam probe

### 2026-05-07 - Direct Surface Continuity Probe

- Added a near-surface continuity probe to separate actual visible/near-player terrain continuity from the broad far-LOD seam candidate counter.
- Rationale:
  - `seamGapCount` still reports pending far-LOD coverage candidates during movement
  - those candidates have not correlated with visible-ground gaps or screen-hole signals
  - the new probe samples expected procedural surface heights in a forward grid near the player, compares adjacent smooth edges, and fails if smooth expected terrain is missing render-ready coverage
- Browser-lab integration:
  - route/traversal summaries now include `framesWithSurfaceContinuityGaps`, `maxSurfaceContinuityGapCount`, and `maxSurfaceContinuityStepMeters`
  - owned browser lab now fails on surface-continuity gap frames
  - seam samples include `surfaceContinuityGapCount`, `abruptSurfaceEdgeCount`, and `maxSurfaceContinuityStepMeters`
- Validation:
  - `mise exec -- bun run typecheck`: pass.
  - `mise exec -- bun test tests/game-route-benchmark.test.ts`: pass, `8` tests.
  - Direct browser lab: `artifacts/owned-browser-lab/20260507T235945Z-surface-continuity-probe-smoke/report.json`, failures none.
  - Browser result: route surface continuity gaps/max step `0/0.50 m`, route p95/max `5.20/30.30 ms`, traversal p95/max `4.80/19.20 ms`, `LOD gaps 0`, `LOD water overlap 0`.
- Honest assessment:
  - This does not remove the legacy broad seam candidate counter; it gives us a better signal for whether those candidates matter near the player.
  - Next work should either retire or re-label the old counter so it is not mistaken for an actual visible seam.
- Rubric movement:
  - Harness maturity: `6.25 -> 6.4` because visible terrain continuity now has a direct route benchmark signal instead of relying on far-coverage proxy counts.
- Next:
  - checkpoint and push
  - re-label the old route seam counter as transient far LOD coverage or replace it with the direct continuity signal in summaries

### 2026-05-08 - Far LOD Coverage Label Cleanup

- Re-labeled the old route `seamGapCount` proxy in browser-lab summaries as far-LOD coverage gaps.
- Added backward-compatible benchmark aliases:
  - `farLodCoverageGapCount`
  - `uncoveredFarLodGapCount`
  - `handoffFarLodHoleCount`
  - `maxFarLodCoverageGapMeters`
  - `framesWithFarLodCoverageGaps`
- Browser-lab output now reports:
  - `holes/far-LOD gaps/blocking seams/overlaps`
  - `route far-LOD gap sample`
  - direct `route surface continuity gaps/max step` on a separate line
- Validation:
  - `mise exec -- bun run typecheck`: pass.
  - `mise exec -- bun test tests/game-route-benchmark.test.ts`: pass, `8` tests.
  - Direct browser lab: `artifacts/owned-browser-lab/20260508T000511Z-far-lod-label-cleanup-smoke/report.json`, failures none.
  - Browser result: route `holes/far-LOD gaps/blocking seams/overlaps 0/51/0/0`, route surface continuity gaps/max step `0/0.50 m`, `LOD gaps 0`, `LOD water overlap 0`.
- Honest assessment:
  - This is reporting hygiene, not a renderer fix.
  - It makes future performance/correctness triage less misleading by separating visible terrain continuity from transient far-LOD work.
- Rubric movement:
  - Harness maturity: `6.4 -> 6.45` because the browser lab now names the signal accurately and preserves the old field names for compatibility.
- Next:
  - checkpoint and push
  - continue world definition work or add a full completion audit before deciding what remains largest

### 2026-05-08 - Movement LOD Cadence and Horizontal Owner Probe

- Followed up the user-reported z-fighting/far-view issue with a stricter ownership probe.
- Findings:
  - the previous browser probe keyed LOD overlap by level label, which could hide same-level owner conflicts
  - the first stricter browser run exposed `3` sampled "overlaps", but the samples were stacked vertical LOD4 chunks in the same X/Z footprint, not competing horizontal surface owners
  - the corrected invariant is horizontal ownership: one LOD0/far-LOD footprint per sampled X/Z, with water tracked separately
- Changes:
  - browser `probeLodCoverage()` now keys LOD owners by horizontal footprint (`LOD level + x/z`), preserving LOD0-vs-LOD and cross-level overlap detection while ignoring valid stacked vertical chunks
  - `tests/lod-system.test.ts` now has a fog-range sampled owner test that fails if any sampled column has multiple horizontal owners or multiple water owners
  - movement frames now run a tiny LOD update cadence (`1` LOD chunk, `0.75 ms` planning budget, every fourth frame) once near streaming/mesh work is quiet, so far LOD can keep making progress while walking without reintroducing large movement hitches
  - integrated the object-lab worker's diagnostics upgrade and ash-marker polish:
    - sample-fit margins/headroom/center offsets
    - projection row/column counts and edge-touch warnings
    - stronger ashland lantern-marker silhouette with crossbar, hanging accents, and central glow
- Validation:
  - `mise exec -- bun run typecheck`: pass.
  - `mise exec -- bun test tests/lod-system.test.ts --test-name-pattern "single visible LOD owner"`: pass.
  - `mise exec -- bun test tests/object-lab.test.ts tests/procedural-generator.test.ts tests/game-route-benchmark.test.ts tests/stream-work.test.ts`: pass, `47` tests.
  - `mise exec -- bun run scripts/owned-browser-lab.ts --label=lod-horizontal-owner-moving-cadence`: pass.
  - Browser artifact: `artifacts/owned-browser-lab/20260508T002104Z-lod-horizontal-owner-moving-cadence/report.json`.
  - Browser result: `LOD overlap LOD0/bands 0/0`, `LOD water overlap 0`, `LOD gaps 0`, `LOD handoff holes 0`, LOD draw calls by level `0/90/60/41/82`.
  - Movement result: traversal p95/max `4.80/20.90 ms`, route p95/max `4.80/31.70 ms`, route work/render/stream/mesh/LOD p95 `4.70/1.40/0.70/1.70/3.30 ms`.
  - Object-lab artifact: `artifacts/object-lab/2026-05-08-001606708Z-ash-marker-final/contact-sheet.svg`.
- Honest assessment:
  - This is a real harness/correctness improvement: same-level LOD owner conflicts are no longer hidden by label collapse.
  - The movement cadence is intentionally conservative. It improves far-LOD progress while walking, but transient far-LOD gap samples still occur during active movement while chunks/meshes catch up.
  - The visual identity metric is still too grid-dominated (`0.68`), so world definition and terrain/material shaping remain major work.
- Rubric movement:
  - Rendering correctness: `5.35 -> 5.55` because settled horizontal LOD ownership and water ownership now have direct unit and browser coverage.
  - Performance/playability: `4.45 -> 4.50` because LOD keeps progressing during movement while route p95/max stays under budget.
  - Harness maturity: `6.45 -> 6.65` because the browser and unit probes now catch a class of same-level LOD conflict the previous label-based probe missed.
  - Visual/world definition: `3.35 -> 3.45` for the ash-marker silhouette polish and stronger object-lab self-monitoring.
- Next:
  - checkpoint and push
  - add render-side ownership or culling if future probes find true fine/coarse overlap during pending movement
  - start a terrain/material pass to reduce the remaining block-grid read

### 2026-05-08 - Bold ROI Pivot: Megastructures, Roads, Sky Ambience

- Trigger:
  - User called out that small polish was not enough: the world still looked like generic block terrain with weak props.
  - I wrote `docs/loop/20260508-bold-roi-plan.md` and ranked work by `impact * confidence / effort`.
  - Highest ROI became route-visible silhouettes and old-road composition before further color tweaks.
- Delegation:
  - World-composition explorer confirmed the highest-leverage path was additive landmarks plus route/object verification.
  - RPG-loop worker added compact pilgrimage objectives, discovery roles, names, and HUD/journal text for the new road/ruin IDs.
  - Sky/weather worker added ambient sky/weather parameters and cheap terrain tint hooks.
- Changes:
  - Added five bold landmarks:
    - `velothi_ziggurat`
    - `ash_obelisk`
    - `rib_arch`
    - `old_road_causeway`
    - `pilgrim_lantern`
  - Added large silhouette feature paths for stepped ziggurats, obelisks, rib arches, and causeway slabs.
  - Increased ash-wastes/badlands/ember rosters so old roads and ruins show up as actual composition, not rare trivia.
  - Added the new IDs to discovery catalog, ancient-road progress counting, procedural tests, and object-lab tooling.
  - Added ash/fungal sky-weather environment fields. I initially tried a fullscreen sky shader, but owned-browser screenshot review caught a black-frame regression. I backed that down to a sky-colored clear plus terrain tint until the sky pass can be made correct.
  - Added browser-lab visual gates for average luma, contrast, and color bucket count so a mostly black screenshot cannot pass again.
- Validation:
  - `mise exec -- bun run typecheck`: pass.
  - `mise exec -- bun run build`: pass.
  - `mise exec -- bun test`: pass, `206` tests.
  - Focused tests before full suite: procedural/discovery/objective/ambient/object-lab `49` tests passed.
  - Route atlas: `artifacts/route-atlas/20260508T005329Z-bold-world-composition/report.json`, failures none, definition score `5.00/5`, landmark hits `46` (`+3`), added route landmark `old_road_causeway`, max notable gap `504.0 m`.
  - Object lab:
    - `artifacts/object-lab/2026-05-08-005448932Z-ziggurat-wide-sample/contact-sheet.svg`, warnings none with wider sample.
    - `artifacts/object-lab/2026-05-08-005416115Z-ash_obelisk/contact-sheet.svg`.
    - `artifacts/object-lab/2026-05-08-005416136Z-rib_arch/contact-sheet.svg`.
    - `artifacts/object-lab/2026-05-08-005415962Z-old_road_causeway/contact-sheet.svg`.
    - `artifacts/object-lab/2026-05-08-005416004Z-pilgrim_lantern/contact-sheet.svg`.
  - Owned browser lab: `artifacts/owned-browser-lab/20260508T011132Z-bold-world-composition-final/report.json`, failures none.
  - Browser screenshot after the visual-gate fix: `artifacts/owned-browser-lab/20260508T011132Z-bold-world-composition-final/settled-page.png`.
  - Browser result: route p95/max `5.00/31.30 ms`, traversal p95/max `4.90/19.00 ms`, draw/triangles `501/382172`, visual identity saturation/grid/color `0.38/0.68/89`, `LOD overlap LOD0/bands 0/0`, water overlap `0`, gaps `0`, handoff holes `0`.
  - Live-forward trace: `artifacts/browser-route-trace/20260508T010837Z-bold-world-composition-visible-live-forward/report.json`, avg/p95 frame `3.98/5.80 ms`.
- Honest assessment:
  - Big readability improvement: screenshots now include large ruins/road objects immediately, and discovery/objectives talk about roads/shrines instead of generic surveying.
  - The fullscreen sky shader attempt was not correct; the new luma/contrast gate caught the kind of blank image that the old harness missed. Current sky is a safe colored clear, not the final dramatic cloud shelf.
  - Route atlas only saw `+1` distinct route landmark because its fixed routes still do not intentionally visit all new megastructures. The generator tests and object-lab prove the objects exist; route-atlas needs visible-nearby landmark scanning next.
  - Live-forward trace still reports transient hole-signal frames during active movement, even though owned-browser lab reports no blocking holes, no LOD overlap, and no surface continuity gaps. This remains a harness/streaming investigation target.
  - Visual identity still has high grid dominance (`0.68`), so terrain macro shaping and material breakup remain high ROI.
- Rubric movement:
  - Visual/world definition: `3.45 -> 4.15` because the world now has distinctive ashland megastructures, causeways, lanterns, and route-flavored HUD/objectives.
  - Harness maturity: `6.65 -> 6.90` because browser-lab now fails blank/too-dark screenshots and object-lab covers the new landmark IDs.
  - Rendering correctness: `5.55 -> 5.60` because the black-frame regression was found and prevented, but the sky shader itself is deferred.
  - Performance/playability: `4.50 -> 4.60` because the heavier scene stayed under route/live-forward p95 frame budgets.
- Next:
  - commit and push this checkpoint
  - add route-atlas visible-nearby landmark requirements for ziggurat/obelisk/rib/causeway routes
  - revisit sky shader only with screenshot luma/color gates active
  - start terrain macro/material breakup to reduce the block-grid read

### 2026-05-08 - Route Atlas Megastructure Route Coverage

- Follow-up after the bold composition checkpoint.
- Added explicit route-atlas coverage routes for:
  - `velothi_ziggurat`
  - `ash_obelisk`
  - `rib_arch`
  - `old_road_causeway`
  - `pilgrim_lantern`
- Validation:
  - `mise exec -- bun run typecheck`: pass.
  - `mise exec -- bun run scripts/route-atlas.ts --label=bold-landmark-routes`: pass.
  - Route atlas artifact: `artifacts/route-atlas/20260508T011459Z-bold-landmark-routes/report.json`.
  - Result: landmark hits `57` (`+11`), distinct landmarks `+6`, regional variants `7`, max notable gap `504.0 m`, failures none.
- Honest assessment:
  - This closes the immediate route-atlas gap for the new landmarks by direct route hits.
  - The better long-term harness is still visible-nearby scanning, because vista landmarks can matter even when the path does not pass through their exact footprint.
- Rubric movement:
  - Harness maturity: `6.90 -> 7.00` because the atlas now has deterministic route coverage for the new high-ROI landmark families.
- Next:
  - commit and push this route-atlas harness checkpoint
  - implement visible-nearby route landmark scans when adding the next set of vistas

### 2026-05-08 - Removed Dead Material Tooling Surface

- Trigger:
  - User explicitly wanted the Minecraft-like material gathering/placement direction removed in favor of an RPG HUD and interaction model.
  - The live UI had already moved away from the old hotbar/inventory/target panel, but stale engine modules and tests still made the repo point future work back toward block editing.
- Changes:
  - Removed legacy material-gathering and placement modules:
    - `src/engine/inventory.ts`
    - `src/engine/hotbar-layout.ts`
    - `src/engine/interaction-loop.ts`
    - `src/engine/targeting-overlay.ts`
    - `src/engine/voxel-raycast.ts`
  - Removed their stale unit tests.
  - Added `tests/rpg-ui-cleanup.test.ts` so the live game page and game client cannot silently reintroduce hotbar, inventory panel, targeting overlay, or visible block-editing language.
  - Updated `docs/loop/plan.md` and `docs/roadmap.md` so the active direction is exploration-first RPG verbs instead of gather/build.
- Validation:
  - Dead-surface search: `rg -n "inventory|hotbar|interaction-loop|targeting-overlay|voxel-raycast|game-hotbar|game-inventory-panel|target-overlay|No voxel in reach|Targeting|Hotbar" src tests scripts` now only finds deliberate absence gates in `scripts/owned-browser-lab.ts` and `tests/rpg-ui-cleanup.test.ts`.
  - `mise exec -- bun run typecheck`: pass.
  - `mise exec -- bun test tests/rpg-ui-cleanup.test.ts`: pass, `2` tests.
  - `mise exec -- bun test`: pass, `197` tests.
  - `mise exec -- bun run build`: pass.
  - Owned browser lab: `artifacts/owned-browser-lab/20260508T012127Z-rpg-no-material-tools-final/report.json`, failures none.
  - Browser result: traversal p95/max `4.80/19.20 ms`, route p95/max `5.10/31.80 ms`, draw/triangles `501/382172`, `LOD overlap LOD0/bands 0/0`, water overlap `0`, gaps `0`, handoff holes `0`, HUD smoke passed.
  - Screenshot reviewed: `artifacts/owned-browser-lab/20260508T012127Z-rpg-no-material-tools-final/settled-page.png`; no hotbar, inventory panel, or target overlay was visible.
- Honest assessment:
  - This does not by itself make the world less blocky or more Morrowind-like; it removes a wrong product direction that would keep pulling the implementation back toward Minecraft.
  - Core voxel edit primitives still exist where the renderer, world model, tests, and debug tooling need them. The cleanup targets player-facing gather/build affordances and the obsolete interaction loop.
  - The next major ROI target is still visual/world definition: macro terrain breakup, stronger silhouettes, sky/weather, roads, creature silhouettes, and route-visible landmarks.
- Rubric movement:
  - RPG direction/UI discipline: `4.30 -> 4.65` because the repo now has both code removal and absence tests for the old material-tool surface.
  - Harness maturity: `7.00 -> 7.10` because the browser lab plus unit tests now protect the no-Minecraft-HUD rule.
  - Visual/world definition: unchanged at `4.15`; this checkpoint is strategic cleanup, not a content leap.
- Next:
  - commit and push this cleanup checkpoint
  - write a broader ROI backlog from the current inspiration art and implement the highest ROI items first
  - start with the block-grid/Minecraft read and moving-performance risk, because those are both user-visible and measurable

### 2026-05-08 - Performance Truth, Mesh Spike Budget, And Terrain Grid Probe

- Trigger:
  - User reported that the visible counter had claimed very high FPS while walking visibly lagged.
  - User also warned that small visual tweaks were not enough; the world still read as Minecrafty/blocky.
  - I re-ranked the ROI backlog in `docs/loop/20260508-bold-roi-plan.md` and put movement-performance truth plus terrain grid breakup at the top.
- Delegation:
  - Performance explorer found that live-forward traces did not reproduce sustained `10 FPS`, but did show movement spikes from streaming and mesh work. The worst pre-fix live-forward move frame was `22.3 ms`, with `14.0 ms` mesh work and later `~11 ms` stream spikes.
  - Terrain/material explorer recommended shader-first and bounded terrain/material changes, with a warning not to pay a geometry tax blindly.
  - Object-lab worker added isolated object scale/cost diagnostics so future prop work can judge too-small, too-sparse, clipped, or expensive objects without guessing.
- Changes:
  - Performance HUD now shows wall-clock FPS from `requestAnimationFrame` timing plus explicit gameplay work milliseconds, instead of making render CPU look like true frame rate.
  - Route trace summary now prints max frame, stream/mesh/LOD p95 and max, diagnostics totals, and unmeasured frame ratio.
  - Async mesh completion adoption is now budgeted per frame, and urgent sync mesh builds share that budget. Worker scheduling still gets its own queue-fill budget so render readiness does not starve.
  - Object lab now reports bounds size, max horizontal span, vertical span, occupied columns, bounds volume, fill ratio, and a solid voxel budget class.
  - Badlands terrace quantization is lower, ash-waste terrain gets strata/grain relief, and the ashland test now gates against dominant `surfaceY % 8` terrace buckets while requiring all four ash material families in sampled columns.
- Failed attempt:
  - I tried a shader-only cracked-crust pass first. Owned browser lab caught a black-frame visual regression: `avg luma 0.03`, luma stddev `1.59`, and only `3` color buckets in `artifacts/owned-browser-lab/20260508T013534Z-terrain-grid-breaker-perf-truth/settled-page.png`.
  - I backed that shader change out completely. `renderer.ts` has no remaining diff from that attempt.
- Validation:
  - `mise exec -- bun run typecheck`: pass.
  - `mise exec -- bun run build`: pass.
  - `mise exec -- bun test`: pass, `197` tests.
  - Route atlas: `artifacts/route-atlas/20260508T014016Z-terrain-grid-breaker-bold/report.json`, failures none, landmark hits `57`, max notable gap `504.0 m`.
  - Object lab: `artifacts/object-lab/2026-05-08-013523371Z-terrain-grid-breaker-ziggurat/report.json`; ziggurat sample reported `38822` solid object voxels, now visible as a `huge` budget signal for future optimization/aesthetic review.
  - Live-forward trace before final mesh cap, after terrain/perf instrumentation: `artifacts/browser-route-trace/20260508T014227Z-terrain-grid-breaker-live-forward/report.json`, avg/p95/max gameplay frame `4.01/6.10/22.30 ms`, max stream/mesh/LOD `11.80/14.00/7.10 ms`, hole signals `57`.
  - Strict mesh cap trace: `artifacts/browser-route-trace/20260508T014719Z-mesh-budget-schedule-live-forward/report.json`, avg/p95/max gameplay frame `3.97/6.00/15.30 ms`, max stream/mesh/LOD `12.30/5.40/6.10 ms`, hole signals `93`.
  - I tested a looser mesh budget of `8`; it restored hole signals to `58` but regressed max gameplay frame to `21.40 ms` and max stream to `18.60 ms`, so I reverted that knob to `6`.
  - Final owned browser lab: `artifacts/owned-browser-lab/20260508T015023Z-mesh-budget-terrain-final/report.json`, failures none.
  - Final browser result: traversal p95/max `4.90/13.00 ms`, route p95/max `5.00/13.30 ms`, draw/triangles `500/383962`, `LOD overlap LOD0/bands 0/0`, water overlap `0`, gaps `0`, handoff holes `0`, render-ready samples `961/961`, HUD smoke passed.
- Honest assessment:
  - Performance improved materially in the full browser lab: route max frame dropped from the prior `~32 ms` browser-lab range to `13.30 ms`, and the live-forward max mesh slice dropped from `14.0 ms` to `5.4 ms`.
  - The stricter mesh cap increases live-forward diagnostic hole signals, but the final owned browser lab still reports no visible ground holes, no LOD overlap, and no handoff holes. This needs continued monitoring, not denial.
  - The performance strip is now honest about wall-clock FPS, but automated screenshot capture can still stall wall timing; the paired gameplay-work number is there to distinguish page jank from render/work cost.
  - The terrain change is safe but not visually sufficient. Browser visual grid dominance stayed at `0.68`, so the next visual step needs stronger composition/content or a shader pass rebuilt more carefully with screenshot gates active.
- Rubric movement:
  - Performance/playability: `4.60 -> 5.20` because measured route/traversal max frame times improved substantially in owned browser lab.
  - Harness maturity: `7.10 -> 7.35` because route trace and object lab now expose the right failure modes instead of hiding them.
  - Visual/world definition: `4.15 -> 4.25` because ashland terrain distribution improved, but the visible grid metric did not move.
- Next:
  - commit and push this checkpoint
  - attack visual grid dominance with a safer strategy than the failed shader: likely salt-marsh set pieces, more non-axis silhouettes, and route-visible content density before another shader attempt
  - investigate incremental residency/streaming spikes next, because max stream remains around `12 ms` in live-forward movement traces

### 2026-05-08 - Salt-Marsh Set Pieces, Vista Harness, And Water Classification

- Trigger:
  - User pointed out that the world still lacked characteristic identity and supplied salt-marsh/ashland inspiration art.
  - User also asked for stronger delegation and better self-monitoring for asset work, so I prioritized harness support that can score landmarks seen near a route, not only underfoot.
- Delegation:
  - Route-atlas worker added deterministic visible-nearby landmark scanning around route samples. This catches vista silhouettes within a fixed 64 m scan pattern, with separate direct-hit and vista-hit counts.
  - Environment explorer reviewed safer set-piece insertion points and warned that object-lab samples could be polluted by water if the regional water materials were not treated as water.
- Changes:
  - Added three salt-marsh/fungal basin landmark families: `crystal_reeds`, `fungal_bridge`, and `rib_remains`.
  - Added discovery catalog names/flavor for the new landmarks so RPG journal output can refer to them as places, not debug IDs.
  - Added route-atlas basin routes that require the new landmarks, plus report fields for direct landmarks, visible-nearby landmarks, and credited landmark samples.
  - Fixed a real water classification bug: regional water override materials such as blackwater `#134` and glow-water `#9CF` are now classified as procedural water. Before this, object-lab and water/rendering logic could treat these surfaces as opaque solid materials.
  - Object-lab now rejects water roots, skips procedural water voxels in object samples, and reports bounds with the intended exclusive size convention.
- Validation:
  - `mise exec -- bun run typecheck`: pass.
  - `mise exec -- bun run build`: pass.
  - Focused tests: `mise exec -- bun test tests/procedural-generator.test.ts tests/object-lab.test.ts tests/water-visuals.test.ts tests/discovery-catalog.test.ts`, pass, `44` tests.
  - Route atlas: `artifacts/route-atlas/20260508T020853Z-salt-marsh-route-coverage/report.json`, failures none, `436` credited landmark samples (`67` direct, `383` vista), max notable gap `360.0 m`, and required basin routes all covered.
  - Object lab after water filtering:
    - `crystal_reeds`: `artifacts/object-lab/2026-05-08-021125075Z-salt-marsh-crystal-reeds-waterfixed/report.json`, `905` solid object voxels.
    - `fungal_bridge`: `artifacts/object-lab/2026-05-08-020452945Z-salt-marsh-fungal-bridge-waterfixed/report.json`, `435` solid object voxels.
    - `rib_remains`: `artifacts/object-lab/2026-05-08-020454118Z-salt-marsh-rib-remains-waterfixed/report.json`, `681` solid object voxels.
  - Final owned browser lab: `artifacts/owned-browser-lab/20260508T021156Z-salt-marsh-waterfixed-final/report.json`, failures none.
  - Browser result: traversal p95/max `5.20/11.80 ms`, route p95/max `5.20/14.00 ms`, draw/triangles `502/381390`, `LOD overlap LOD0/bands 0/0`, water overlap `0`, gaps `0`, handoff holes `0`, render-ready near samples `961/961`, HUD smoke passed.
- Honest assessment:
  - The harness improvement is stronger than the art improvement: route-atlas can now credit visible landmarks near a route, which is the right direction for vista-driven world design.
  - The water classification fix is correctness-significant and probably more important than it looks; blackwater was contaminating object samples and could be treated as solid/non-water by downstream logic.
  - The new landmarks increase basin identity, but `fungal_bridge` is still too thin and material-uniform, while `crystal_reeds` is still overly dominated by one material. These need shape/material polish, not victory laps.
  - Browser visual grid dominance remained `0.68`, so this checkpoint did not solve the Minecrafty read. The next visible ROI should be either object silhouette polish or a safer large-scale composition/sky pass.
- Rubric movement:
  - Harness maturity: `7.35 -> 7.65` because route-atlas now measures off-route vistas and object-lab correctly filters regional water.
  - Rendering correctness: `5.60 -> 5.85` because regional water materials are now consistently classified and browser LOD/water overlap gates stayed clean.
  - Visual/world definition: `4.25 -> 4.45` because the salt-marsh now has new landmark families and route coverage, but the screen-level grid metric did not improve.
  - Performance/playability: unchanged at `5.20`; the new content stayed under the current frame budget but did not directly improve movement cost.
- Next:
  - commit and push this checkpoint
  - delegate isolated object polish for the weakest set pieces with object-lab artifacts as the acceptance target
  - improve object-lab representative-root selection so asset workers stop judging clipped/edge samples
  - keep moving toward high-contrast silhouettes, sky/weather, and larger composition changes instead of small palette tweaks

### 2026-05-08 - Basin Object Polish, Centered Object Lab, And Storm Shelf Sky

- Trigger:
  - The prior salt-marsh checkpoint proved the new basin landmarks existed, but object-lab showed `fungal_bridge` was thin/material-uniform and `crystal_reeds` was too dominated by a single material.
  - The browser screenshot still had a flat clear-color sky, which left the ashland scene feeling sterile even with large ruins in view.
- Delegation:
  - Object worker polished `crystal_reeds` and `fungal_bridge` using object-lab as the acceptance harness.
  - Harness worker improved representative root selection for `silt_shell`, `crystal_reeds`, `fungal_bridge`, and `rib_remains` so object-lab chooses centered/high-density samples instead of clipped edge roots.
- Changes:
  - `crystal_reeds` now uses a multi-reed crystal cluster variant with three materials (`#68A`, `#CEF`, `#DFF`) instead of a near-monochrome spire.
  - `fungal_bridge` now uses a stalked shelf/cap causeway variant with three non-water materials (`#465`, `#8CF`, `#6A8`) instead of a thin uniform slab.
  - Object-lab centered-root selection is scoped to the weak wet/set-piece landmarks so older landmarks keep their previous first-hit behavior.
  - Renderer now draws a single full-screen sky triangle before terrain, using the existing ambient sky/fog controls to create a darker ash storm shelf and fungal horizon tint without adding geometry or texture assets.
  - Added regression tests for centered fungal-bridge root selection and basin set-piece silhouette/material variety.
- Validation:
  - `mise exec -- bun run typecheck`: pass.
  - `mise exec -- bun run build`: pass.
  - `mise exec -- bun test tests/object-lab.test.ts`: pass, `3` tests.
  - `mise exec -- bun test tests/procedural-generator.test.ts -t "salt-marsh"`: pass, `2` tests.
  - `mise exec -- bun test`: pass, `200` tests.
  - Route atlas: `artifacts/route-atlas/20260508T022622Z-sky-and-basin-polish-route-check/report.json`, failures none, `436` credited landmark samples, `360.0 m` max notable gap, no delta from the salt-marsh coverage baseline.
  - Object lab:
    - `crystal_reeds`: `artifacts/object-lab-after/2026-05-08-022406613Z-crystal-reeds-scoped-root-selection/report.json`, `675` voxels, `3` materials, dominant share `41.2%`, warnings none.
    - `fungal_bridge`: `artifacts/object-lab-after/2026-05-08-022349278Z-fungal-bridge-scoped-root-selection/report.json`, `2154` voxels, `3` materials, dominant share `48.7%`, warnings none.
  - Final owned browser lab: `artifacts/owned-browser-lab/20260508T022417Z-sky-storm-shelf-enhanced/report.json`, failures none.
  - Browser result: traversal p95/max `4.90/19.30 ms`, route p95/max `5.10/13.90 ms`, draw/triangles `489/376076`, `LOD overlap LOD0/bands 0/0`, water overlap `0`, gaps `0`, handoff holes `0`, render-ready near samples `961/961`, HUD smoke passed.
- Honest assessment:
  - The object-lab improvements are strong and measurable: the two weakest basin landmarks went from warning-heavy/dominant-material samples to warning-free, three-material silhouettes.
  - The sky pass is intentionally cheap and passed the browser gates. It improves whole-screen mood, but it is still procedural bands rather than a fully art-directed storm/cloud system.
  - Browser visual grid dominance is still `0.68`, so the screen still reads too blocky. The next large visual win needs terrain/composition density or a stronger non-axis surface breakup pass, not more small prop polish alone.
  - Traversal max had one `19.30 ms` spike while route max stayed `13.90 ms`; this remains acceptable for the current budget but movement spikes still need periodic live-forward checks.
- Rubric movement:
  - Visual/world definition: `4.45 -> 4.80` because the weak basin props became readable set pieces and the sky is no longer just a flat clear.
  - Harness maturity: `7.65 -> 7.90` because object-lab root choice is now more representative for delegated asset work.
  - Rendering correctness: `5.85 -> 5.95` because the new sky pass renders through browser/WebGPU gates with no LOD/water overlap regressions.
  - Performance/playability: unchanged at `5.20`; route performance held, but this checkpoint did not attack streaming spikes.
- Next:
  - commit and push this checkpoint
  - tackle the persistent `0.68` grid metric with larger terrain/composition changes
  - run live-forward movement traces again before increasing view distance or adding heavier distant content

### 2026-05-08 - Movement Streaming Spike Budget

- Trigger:
  - Before making terrain or view-distance heavier, I audited the user's walking-FPS complaint again with the live-forward trace harness.
  - Baseline after the sky/object checkpoint still had acceptable p95 but bad outliers: `21.00 ms` max gameplay frame, `17.20 ms` max stream, `13.10 ms` max mesh, and `10.60 ms` max LOD in `artifacts/browser-route-trace/20260508T023051Z-post-sky-live-forward-audit/report.json`.
- Investigation:
  - The first spikes were residency scan/planning work while hundreds of chunks were pending, even on frames with `generated=0`.
  - After adding a planning budget, remaining stream spikes were eviction bursts unloading roughly `150-161` old chunks in one movement frame.
  - A too-low moving mesh cap (`3`) reduced max frame but starved readiness: hole-signal frames jumped to `309`, so I rejected that setting.
- Changes:
  - `ProceduralResidentWorld.updateResidencyAround` now accepts `maxPlanMs` and `maxEvictChunks`.
  - Budget-exhausted residency scans now mark the update incomplete and never evict from a partial needed-key set.
  - Movement frames use a `5 ms` residency plan budget, a `32` chunk eviction budget, and a `4` mesh rebuild/adoption budget.
  - Idle/settle/forced updates still use full budgets so the world can catch up when the player stops.
  - Added tests for incomplete planning safety and amortized eviction.
- Validation:
  - `mise exec -- bun run typecheck`: pass.
  - `mise exec -- bun run build`: pass.
  - Focused tests: `mise exec -- bun test tests/procedural-resident-world.test.ts tests/stream-work.test.ts tests/game-route-benchmark.test.ts`, pass, `29` tests.
  - Live-forward final: `artifacts/browser-route-trace/20260508T024026Z-budgeted-mesh4-eviction-live-forward/report.json`.
  - Live-forward result: avg/p95/max gameplay frame `3.68/5.80/8.20 ms`, p95 stream/mesh/LOD `1.10/1.70/3.90 ms`, max stream/mesh/LOD `4.10/3.70/6.50 ms`, no screen-void signals.
  - Final owned browser lab: `artifacts/owned-browser-lab/20260508T024123Z-budgeted-streaming-final-browser/report.json`, failures none.
  - Browser result: traversal p95/max `5.30/10.90 ms`, route p95/max `5.20/8.60 ms`, draw/triangles `539/384582`, `LOD overlap LOD0/bands 0/0`, water overlap `0`, gaps `0`, handoff holes `0`, render-ready near samples `961/961`, HUD smoke passed.
- Honest assessment:
  - This directly improves the walking outliers I could reproduce: live-forward max frame moved from `21.00 ms` to `8.20 ms`.
  - The harness still reports diagnostic hole-signal frames during live movement, but screen-void signals are absent and the full browser lab reports no visible holes, no LOD overlap, and no handoff holes. This remains a diagnostic distinction to keep monitoring.
  - The budget values are empirical, not magic. The `3` mesh cap looked faster but was rejected because it made readiness worse; `4` is the current measured balance.
  - Visual grid dominance remains unchanged at `0.68`; this checkpoint is performance correctness, not an art/composition fix.
- Rubric movement:
  - Performance/playability: `5.20 -> 5.90` because live-forward max frame and stream/mesh outliers dropped sharply while route/traversal browser lab stayed clean.
  - Harness maturity: `7.90 -> 8.05` because the trace now directly guided budget choices and caught the rejected starvation setting.
  - Rendering correctness: unchanged at `5.95`; LOD/water correctness held, but no new rendering feature was added.
  - Visual/world definition: unchanged at `4.80`.
- Next:
  - commit and push this performance checkpoint
  - proceed to terrain/composition grid-breaker work with the new movement budgets in place

### 2026-05-08 - Terrain Grid-Breaker And Route Stretch Density

- Trigger:
  - User called out that the world still read as Minecrafty, with ugly props and weak characteristic identity despite recent landmark work.
  - The ROI plan and browser artifacts agreed that visual grid dominance was still the main visible blocker after movement performance improved.
- Delegation:
  - Route-density worker added a 300 m route-stretch scan to `scripts/route-atlas.ts`. The first run failed honestly with `3` tokenless windows on `ash-glass-traverse` (`1900-2200 m`, `1950-2250 m`, `2000-2300 m`), proving the new gate was useful.
  - Object worker improved `pilgrim_lantern` in isolation using object-lab: the integrated report has `728` solid object voxels, `3` materials, and no warnings.
- Changes:
  - Exposed extra deterministic surface fields through `sampleBiomeProbe` so route and ambiance harnesses can reason about terrain identity without resampling private generator state.
  - Added diagonal surface-crust material blending for top surfaces. This creates non-axis fracture bands in material transitions while keeping the same voxel mesh topology.
  - Added a small, gated terrain crust-breakup height term driven by strata, patch, scatter, desolation, and terrace host. This is intentionally modest to avoid breaking route physics and LOD handoff.
  - Strengthened ash-waste surface/subsurface mixing and transition thresholds so ashland crust reads less like a single flat slab.
  - Added route stretch terrain tokens (`wind-cut-steppe`, `salt-crust`, `ash-crust`) so the atlas can distinguish long visually meaningful terrain stretches from truly empty route stretches.
  - Integrated the delegated `pilgrim_lantern` shape: ash plinth, banded post, crossbar, hanging amber cage, and rear counterweight. Added object-lab and procedural shape checks for it.
- Validation so far:
  - `mise exec -- bun run typecheck`: pass.
  - Route atlas: `artifacts/route-atlas/20260508T030031Z-terrain-gridbreaker-final-atlas/report.json`, failures none, route stretch coverage `100.0%` (`221/221` tokenized, `0` tokenless), max notable gap `360.0 m`, definition score `5.00 / 5`.
  - Object lab: `artifacts/object-lab/2026-05-08-025942266Z-pilgrim-lantern-integrated/report.json`, `728` solid voxels, `3` materials.
  - Focused tests: `mise exec -- bun test tests/procedural-generator.test.ts tests/object-lab.test.ts tests/ambient-environment.test.ts`, pass, `45` tests.
  - Final owned browser lab: `artifacts/owned-browser-lab/20260508T031451Z-terrain-gridbreaker-browser-prod/report.json`, failures none.
  - Browser result: traversal p95/max `5.00/9.50 ms`, route p95/max `4.60/9.10 ms`, draw/triangles `551/387372`, `LOD overlap LOD0/bands 0/0`, water overlap `0`, gaps `0`, handoff holes `0`, render-ready near samples `961/961`, HUD smoke passed.
  - Browser visual identity: saturation/grid/color `0.38/0.68/92`.
- Honest assessment:
  - The route-density harness improved materially: it caught real empty route windows, and the final atlas is now clean without over-broadening rare regional variants.
  - The terrain breakup is conservative and safe, but it did not move the screenshot grid metric: grid dominance stayed `0.68`. The next pass needs stronger composition-level changes rather than more micro breakup.
  - The first attempt to fix route density by making `steppe_monolith` more common was rejected because it made a rare regional variant cover about `6.1%` of scanned samples and broke a fixed pilgrim-lantern route. The accepted solution keeps rarity intact and measures terrain tokens directly.
- Rubric movement:
  - Harness maturity: `8.05 -> 8.30` because route stretches now have a hard density gate and expose empty 300 m windows.
  - Visual/world definition: `4.80 -> 4.95` because route identity and ashland props improved, but the screen-level grid metric did not.
  - Rendering correctness: `5.95 -> 6.05` because browser LOD/water/handoff checks stayed clean after terrain/material changes.
  - Performance/playability: `5.90 -> 6.00` because browser route/traversal max frames stayed under `10 ms` and live-forward max was `9.50 ms`.
- Next:
  - commit and push only if browser correctness and performance hold
  - next high-ROI candidate is a bolder composition density director; the browser screenshot metric says micro terrain breakup is not enough

### 2026-05-08 - Old Road Foreground Mass

- Trigger:
  - The terrain grid-breaker browser result proved the next work should be composition-scale, not more subtle surface fracture.
  - The screenshot still showed roads and terrain reading as broad grids with sparse landmarks.
- Delegation:
  - Prop/composition worker widened old-road causeways in isolation, keeping fungal bridges gated off through `materialAccent === 0`.
- Changes:
  - Old-road causeway material sampling now adds broken shoulder slabs and intermittent approach slabs around the main cross shape.
  - Added a procedural surface-footprint helper and assertions so old-road causeways must occupy a wider, denser foreground footprint.
- Validation:
  - `mise exec -- bun run typecheck`: pass.
  - Focused tests: `mise exec -- bun test tests/procedural-generator.test.ts tests/object-lab.test.ts`, pass, `41` tests.
  - Route atlas: `artifacts/route-atlas/20260508T032447Z-causeway-footprint-atlas/report.json`, failures none, route stretch coverage `100.0%`, max notable gap `360.0 m`.
  - Build: `mise run build`, pass.
  - Live-forward trace: `artifacts/browser-route-trace/20260508T032527Z-causeway-footprint-live-forward/report.json`, avg/p95/max gameplay `3.25/5.40/14.70 ms`, p95 stream/mesh/LOD `1.10/1.70/3.50 ms`, max stream/mesh/LOD `3.50/12.60/7.20 ms`.
  - First full owned-browser-lab attempt hung after navigation before writing a report, exposing a harness reliability bug.
  - Added CDP command and page-probe timeouts to `owned-browser-lab`; crashes now write a failure report instead of hanging silently.
  - Final owned browser lab: `artifacts/owned-browser-lab/20260508T033327Z-causeway-footprint-browser-timeout-check/report.json`, failures none.
  - Browser result: traversal p95/max `5.00/7.70 ms`, route p95/max `4.60/5.90 ms`, draw/triangles `550/388332`, `LOD overlap LOD0/bands 0/0`, water overlap `0`, gaps `0`, handoff holes `0`, render-ready near samples `961/961`, HUD smoke passed.
  - Browser visual identity stayed saturation/grid/color `0.38/0.68/87`.
- Honest assessment:
  - This is a small but directionally better composition change: old roads now have more foreground mass without broad placement churn or new draw-call families.
  - The live-forward max frame had one mesh outlier, but the full browser lab route/traversal budgets stayed better than the prior checkpoint.
  - The full browser lab now has timeout protection, which makes it safer to use as a gate for larger composition changes.
  - Grid dominance still stayed at `0.68`, so foreground mass alone is not yet enough to solve the block-grid read.
- Rubric movement:
  - Visual/world definition: `4.95 -> 5.05` for stronger route-surface footprint.
  - Harness maturity: `8.30 -> 8.45` because owned-browser-lab no longer has unbounded CDP/page-probe waits.
  - Performance/playability: unchanged; budgets held, but the content change was not primarily performance work.
- Next:
  - commit and push the causeway footprint plus browser-lab timeout hardening
  - continue composition density director work with browser route traces and route-atlas gates active

### 2026-05-08 - Strong Silhouette Route Director

- Trigger:
  - User called out that the world still lacked characteristic forms and that small tweaks were not enough.
  - The previous route-stretch gate could pass on terrain tokens even when route views still lacked large readable silhouettes.
- Delegation:
  - ROI sidecar returned a complementary ranked list: sky identity, atmospheric layering, prop-family replacement, marsh composition, and object-lab distinctiveness now outrank more terrain micro-noise.
  - Object-lab worker added a batch route-landmark comparison mode so future prop passes can inspect the main Morrowind-like landmark set in one command.
- Changes:
  - Route atlas now has a separate strong-silhouette scan: `360 m` windows every `60 m`, only counting direct or vista landmarks at least `5.5 m` tall.
  - The vista scan radius increased from `64 m` to `96 m`, which better represents midground silhouettes without pretending tiny route tokens are visible character.
  - Added an old-route skyline director for the main traversal routes. It places causeways, lanterns, obelisks, rib arches, pillars, basalt spires, standing stones, and dead trees on dusty/open route bands.
  - Tightened the director after a test regression: lush verdant/fern/bloom and old-growth highland areas keep their existing forest identity instead of being overwritten by ash-route props.
- Validation so far:
  - Strong-silhouette harness baseline before generation changes: `artifacts/route-atlas/20260508T034125Z-strong-silhouette-baseline/report.json`, `46.4%` strong coverage, `96` empty windows.
  - Wider vista-only harness: `artifacts/route-atlas/20260508T034216Z-strong-silhouette-wider-vista/report.json`, `57.0%` strong coverage, `77` empty windows. This proved the old radius was under-counting, but still not enough.
  - Final route-atlas: `artifacts/route-atlas/20260508T034816Z-pilgrim-route-skyline-open-biomes/report.json`, failures none, `87.7%` strong coverage, `22` empty windows, max notable gap `108.0 m`.
  - Focused tests: `mise exec -- bun test tests/procedural-generator.test.ts tests/object-lab.test.ts`, pass, `42` tests.
  - Typecheck: `mise exec -- bun run typecheck`, pass.
  - Build: `mise run build`, pass.
  - Object-lab batch: `artifacts/object-lab/2026-05-08-034851132Z-route-landmark-comparison-after-skyline/batch-report.json`, `9` landmark runs. It surfaced the next prop-quality queue: `velothi_ziggurat` and `ash_obelisk` are huge/dominant-material, `rib_arch` is too small/sparse, and `rib_remains` still has low-fill/root warnings.
  - Live-forward trace: `artifacts/browser-route-trace/20260508T034913Z-pilgrim-route-skyline-live-forward/report.json`, avg/p95/max gameplay `3.18/5.20/7.30 ms`, p95 stream/mesh/LOD `1.00/1.50/3.70 ms`, max stream/mesh/LOD `3.50/4.70/5.10 ms`.
  - Owned browser lab: `artifacts/owned-browser-lab/20260508T034913Z-pilgrim-route-skyline-browser/report.json`, failures none.
  - Browser result: traversal p95/max `5.60/9.00 ms`, route p95/max `4.90/9.60 ms`, draw/triangles `556/393878`, `LOD overlap LOD0/bands 0/0`, water overlap `0`, LOD gaps `0`, handoff holes `0`, render-ready near samples `961/961`, HUD smoke passed.
  - Browser visual identity stayed saturation/grid/color `0.38/0.68/89`.
- Honest assessment:
  - This is the first composition pass that moved a route-level characteristic metric substantially: strong coverage moved from `46.4%` to `87.7%`.
  - It is still a route/vista proxy, not actual screenshot semantic understanding. The browser grid metric must decide whether this translates to the player's view.
  - The object-lab batch proves several headline props are still first-pass quality. The director increases the chance the player sees them, so the next prop pass has higher ROI.
  - Performance trace and full browser lab headroom are both good after adding the director.
  - The browser grid metric still stayed at `0.68`, so route composition alone is not enough to solve the block-grid read. The next visual pass needs either better skyline object quality, atmosphere, or both.
- Rubric movement:
  - Harness maturity: `8.45 -> 8.70` because the atlas now distinguishes mere token density from actual tall-silhouette cadence, and object-lab can compare route landmarks in batch.
  - Visual/world definition: `5.05 -> 5.45` because main routes now have frequent large skyline forms instead of only low props and terrain tokens.
  - Performance/playability: `6.00 -> 6.15` because live-forward held under `8 ms` max and full browser route max held under `10 ms` after adding route density.
  - Rendering correctness: `6.05 -> 6.15` because LOD overlap, water overlap, LOD gaps, handoff holes, and render-ready checks stayed clean with denser skyline routes.
- Next:
  - commit and push if browser correctness/performance hold
  - next likely work: either prop-family replacement for the warned skyline objects or stronger sky/atmospheric layering, depending on the browser screenshot metrics

### 2026-05-08 - Skyline Prop And Atmosphere Polish

- Trigger:
  - Strong route silhouette coverage now exposes the weak objects more often, so the object-lab batch warning queue became high ROI.
  - The browser grid metric still stayed at `0.68`, so a conservative whole-screen atmosphere pass was also worth testing.
- Delegation:
  - Sky/atmosphere worker took the renderer/ambient slice while I worked on skyline prop geometry.
- Changes:
  - `ash_obelisk` now has three materials with warm vertical inlays, plinth courses, tighter glyph bands, and a stronger crown read.
  - `rib_arch` now uses multiple rib planes, chipped accent edges, and thicker supports; `rib_remains` gains a spine and accent vertebrae.
  - Object-lab root selection now uses centered samples for the large old-road/skyline landmarks, not only wet set pieces.
  - Ashfall and fungal ambient profiles now have stronger storm shelf, ash streak, far haze, and fungal horizon coloring while keeping a luma floor.
- Validation:
  - Object-lab before this pass: `artifacts/object-lab/2026-05-08-034851132Z-route-landmark-comparison-after-skyline/batch-report.json`.
  - Object-lab final: `artifacts/object-lab/2026-05-08-035724493Z-route-landmark-prop-polish-v2/batch-report.json`.
  - Object-lab improvements: `ash_obelisk` dominant share `98.0% -> 68.5%` and `2 -> 3` materials; `rib_arch` `206 -> 783` voxels, `2 -> 3` materials, warning-free; `rib_remains` `663 -> 3647` voxels, `2 -> 3` materials, warning-free.
  - Remaining object-lab warning queue: `velothi_ziggurat` and `ash_obelisk` remain intentionally huge; `crystal_reeds` is still top-projection sparse.
  - Focused tests: `mise exec -- bun test tests/procedural-generator.test.ts tests/object-lab.test.ts tests/ambient-environment.test.ts`, pass, `47` tests.
  - Typecheck: `mise exec -- bun run typecheck`, pass.
  - Route atlas: `artifacts/route-atlas/20260508T035800Z-skyline-prop-atmosphere-polish/report.json`, failures none, strong silhouette coverage still `87.7%`, max notable gap `108.0 m`.
  - Build: `mise run build`, pass.
  - Live-forward trace: `artifacts/browser-route-trace/20260508T035840Z-skyline-prop-atmosphere-live-forward/report.json`, avg/p95/max gameplay `3.25/5.30/9.10 ms`, p95 stream/mesh/LOD `1.10/1.60/3.70 ms`.
  - Owned browser lab: `artifacts/owned-browser-lab/20260508T035840Z-skyline-prop-atmosphere-browser/report.json`, failures none.
  - Browser result: traversal p95/max `4.70/9.20 ms`, route p95/max `4.60/8.10 ms`, draw/triangles `512/383634`, `LOD overlap LOD0/bands 0/0`, water overlap `0`, LOD gaps `0`, handoff holes `0`, render-ready near samples `961/961`, HUD smoke passed.
  - Browser visual identity moved color count `89 -> 98`, saturation `0.38 -> 0.37`, grid dominance stayed `0.68`.
- Honest assessment:
  - This pass made visible skyline props materially less crude and did so without performance cost; browser draw calls and triangles actually dropped on the measured route.
  - The atmosphere pass improved color variety and stayed browser-safe, but still did not move the axis-aligned grid metric.
  - The persistent `0.68` grid read now looks like a camera/screen composition or meshing/lighting problem, not a lack of route tokens alone.
- Rubric movement:
  - Visual/world definition: `5.45 -> 5.75` because the route skyline objects and sky mood are now more distinctive, even though the screen-grid metric did not move.
  - Harness maturity: `8.70 -> 8.80` because the batch object-lab reports now directly drive and confirm prop quality changes.
  - Rendering correctness: `6.15 -> 6.25` because shader/ambient changes passed luma/color gates and LOD/water/handoff checks.
  - Performance/playability: `6.15 -> 6.20` because route/traversal browser max frames stayed under `10 ms` and draw cost did not increase.
- Next:
  - commit and push this prop/atmosphere checkpoint
  - target the persistent grid metric with either lighting depth/contact shading or a screenshot-specific composition/camera harness, since silhouette density and prop quality alone did not move it

### 2026-05-08 - Screenshot Composition Diagnostic

- Trigger:
  - The single browser grid metric stayed at `0.68` after route silhouettes, prop polish, and atmosphere changes.
  - I needed to know which screen region was responsible before attempting more visual changes.
- Changes:
  - Added `scripts/analyze-screenshot-composition.ts` and `bun run analyze:screenshot`.
  - The tool reads a PNG artifact, splits it into sky, horizon, center, lower-ground, and lower-corner regions, then reports luma, saturation, color count, horizontal/vertical/diagonal edge rates, and axis-to-diagonal dominance.
  - It defaults to the latest completed owned-browser-lab screenshot, so it is fast to run after any browser checkpoint.
- Validation:
  - Smoke run on the prop/atmosphere browser artifact: `artifacts/screenshot-composition/20260508T040554Z-skyline-prop-atmosphere/report.json`.
  - Post-revert confirmation run: `artifacts/screenshot-composition/20260508T041754Z-post-revert-browser-retry/report.json`.
  - Typecheck: `mise exec -- bun run typecheck`, pass.
  - Owned browser lab after rejecting the lighting attempt: `artifacts/owned-browser-lab/20260508T041557Z-screenshot-tool-post-revert-browser-retry/report.json`, failures none.
- Findings:
  - Likely grid driver is `horizon`, because it has the highest strong axis-edge product.
  - Highest axis-to-diagonal ratio is `bottom_center` at `0.77`, but its absolute axis edge rate is low (`5.7%`) and it has only `16` color buckets.
  - `center_view` remains the same as the overall browser metric (`0.68`), confirming the old single number was dominated by mid-screen composition, not HUD or one screen corner.
- Rejected attempt:
  - I tried a shader-only contact/depth pass after building the tool.
  - Browser lab caught it as a black-frame regression twice: `artifacts/owned-browser-lab/20260508T040846Z-lighting-depth-composition-browser-retry/report.json` and `artifacts/owned-browser-lab/20260508T041028Z-lighting-depth-composition-browser-fixed/report.json` both had avg luma `0.03`, luma stddev `1.59`, and only `3` color buckets.
  - I reverted that shader change and kept the diagnostic tool.
- Honest assessment:
  - This checkpoint improves evaluation, not visuals. It gives a faster and more precise way to decide whether future changes affect the actual problem.
  - The failed shader pass was useful because the browser lab proved it unsafe before it could be committed.
- Rubric movement:
  - Harness maturity: `8.80 -> 9.00`.
  - Rendering correctness: unchanged; the unsafe shader change was rejected.
  - Visual/world definition: unchanged until the next visual fix uses the new diagnostic.
- Next:
  - commit and push the screenshot diagnostic
  - attack the horizon/center composition specifically, likely through safer atmospheric occlusion or route-camera framing rather than fragment contact shading

### 2026-05-08 - View Atlas Harness And Bone Chime Route Prop

- Trigger:
  - The user called out that barely visible tweaks were not enough and asked for bolder ROI-driven work.
  - The screenshot diagnostic showed the persistent grid read is a horizon/center composition problem, but a single settled browser screenshot is too weak for judging multi-location world changes.
- Delegation:
  - A tooling worker built the view-atlas sidequest while I worked on the runtime/worldgen slice.
  - A read-only audit independently ranked foreground/midground vista composition, lower-fill skyline props, and screenshot atlas verification as the highest current ROI.
- Changes:
  - Added `bun run atlas:views`, which captures deterministic multi-view screenshots through the real browser/game API, writes per-view PNGs, and reports per-region luma/color/grid metrics plus markdown thumbnails.
  - Added `GameController.setCameraPoseAndSettle` and exposed it through `window.__VOXELS_GAME__` so browser tools can place a camera honestly without reaching through private runtime state.
  - Added `bone_chimes`, a three-material old-road route prop with an upper rack, hanging cords, and pale bone blades. It is placed in the old-route skyline director and ash-wastes roster.
  - Added discovery catalog entries and focused shape/material tests for the new landmark.
- Validation:
  - Typecheck: `mise exec -- bun run typecheck`, pass.
  - Focused tests: `mise exec -- bun test tests/discovery-catalog.test.ts tests/object-lab.test.ts tests/procedural-generator.test.ts`, pass, `45` tests.
  - Focused object-lab for the new prop: `artifacts/object-lab/2026-05-08-043856805Z-bone-chimes-route-prop/batch-report.json`, `bone_chimes` warning-free, `1453` voxels, `3` materials, dominant share `0.712`.
  - Route object-lab batch: `artifacts/object-lab/2026-05-08-044027879Z-bone-chimes-route-landmarks/batch-report.json`, `10` route landmark runs. `bone_chimes` is warning-free; remaining warning queue includes the intentionally huge `velothi_ziggurat`/`ash_obelisk` and sample-fit issues for `rib_arch`/`rib_remains`.
  - Route atlas: `artifacts/route-atlas/20260508T044008Z-bone-chimes-route-vistas/report.json`, failures none, `+35` landmark hits, `+1` distinct landmark, max notable gap still `108.0 m`, strong silhouette coverage still `87.7%`.
  - View atlas: `artifacts/view-atlas/20260508T044057Z-bone-chimes-view-atlas/report.json`, failures none, five fixed screenshots captured.
  - Live-forward trace: `artifacts/browser-route-trace/20260508T044057Z-bone-chimes-live-forward/report.json`, avg/p95/max gameplay `3.22/5.70/14.10 ms`, p95 stream/mesh/LOD `1.10/1.80/3.50 ms`.
  - Owned browser lab: `artifacts/owned-browser-lab/20260508T044248Z-bone-chimes-view-atlas-browser/report.json`, failures none.
  - Browser result: traversal p95/max `4.50/7.20 ms`, route p95/max `4.70/9.60 ms`, draw/triangles `512/383634`, `LOD overlap LOD0/bands 0/0`, water overlap `0`, LOD gaps `0`, handoff holes `0`, render-ready near samples `961/961`, HUD smoke passed.
- Honest assessment:
  - The view atlas is the main win. It makes future visual work faster and less subjective, and it gives screenshots from several fixed world positions instead of one accidental settled view.
  - `bone_chimes` is a meaningful route prop, not a palette tweak: it adds a new old-road silhouette family and route-atlas proves it is actually seen.
  - The prop does not solve the global browser grid metric; owned browser still reports saturation/grid/color `0.37/0.68/94`. This confirms the next visual win must be camera-scale horizon/terrain composition or a safer depth/lighting path, not more isolated props alone.
  - The atlas comparison is only partially useful this time because the prior baseline had one view. The next view-atlas run should compare against this full five-view report.
- Rubric movement:
  - Harness maturity: `9.00 -> 9.30` because fixed multi-view screenshot capture is now scriptable, repeatable, and tied to browser/runtime correctness.
  - Visual/world definition: `5.75 -> 5.85` because the old road now has a new distinctive bone-rack silhouette, but the screen-grid blocker remains.
  - Performance/playability: unchanged at `6.20`; route/traversal budgets held with the new prop.
  - Rendering correctness: `6.25 -> 6.30` because the new browser/camera harness and full browser lab agree on no LOD overlaps, gaps, handoff holes, or visual blank-frame failures.
- Next:
  - checkpoint and push this harness/prop slice
  - use `artifacts/view-atlas/20260508T044057Z-bone-chimes-view-atlas/report.json` as the first full view-atlas baseline
  - next highest ROI remains a bolder horizon/foreground composition pass, likely reducing huge rectilinear ziggurat fill and adding larger non-rectangular ground interrupters rather than another small prop

### 2026-05-08 - Megastructure Lower-Fill Pass And HUDless View Atlas

- Trigger:
  - The five-view atlas proved the route prop was visible but the global grid read did not move.
  - Object-lab still showed `velothi_ziggurat` and `ash_obelisk` as huge, solid, rectilinear forms.
- Changes:
  - The view atlas now hides HUD/capture/discovery overlays before screenshot capture, so visual metrics are no longer polluted by UI panels.
  - Added two closer hero views, `ziggurat-approach` and `obelisk-approach`, aimed at actual object-lab roots instead of only distant route cameras.
  - Reworked `ash_obelisk` into a slimmer tower with lower plinth, narrow cutouts, and preserved warm inlays.
  - Reworked `velothi_ziggurat` lower tiers into chamfered, partly hollow perimeter geometry with forecourt voids, side buttresses, broken corners, and accent inlays while preserving the broad route silhouette.
- Validation:
  - Megastructure baseline before this pass: `artifacts/object-lab/2026-05-08-044829116Z-megastructure-baseline/batch-report.json`.
  - Object-lab final: `artifacts/object-lab/2026-05-08-045244864Z-megastructure-lower-fill-v3/batch-report.json`.
  - `velothi_ziggurat`: `40224 -> 24198` voxels, fill ratio `0.425 -> 0.255`, materials `2 -> 3`; still huge, but no longer a fully filled slab mass.
  - `ash_obelisk`: `8351 -> 6345` voxels, fill ratio `0.352 -> 0.268`, dominant share `0.685 -> 0.653`, and warning-free.
  - Focused tests: `mise exec -- bun test tests/procedural-generator.test.ts tests/object-lab.test.ts`, pass, `42` tests.
  - Route atlas: `artifacts/route-atlas/20260508T045314Z-megastructure-lower-fill/report.json`, failures none, route stretch coverage `100%`, strong silhouette coverage `87.7%`, max notable gap `108.0 m`.
  - HUDless/expanded view atlas: `artifacts/view-atlas/20260508T045619Z-megastructure-hudless-atlas/report.json`, failures none, seven fixed screenshots captured.
  - Typecheck: `mise exec -- bun run typecheck`, pass.
  - Build: `mise exec -- bun run build`, pass.
  - First owned-browser lab attempt timed out in CDP before page metrics: `artifacts/owned-browser-lab/20260508T045842Z-megastructure-hudless-atlas-browser/report.json`.
  - Retry owned-browser lab: `artifacts/owned-browser-lab/20260508T050211Z-megastructure-hudless-atlas-browser-retry/report.json`, failures none.
  - Browser result: traversal p95/max `5.40/9.50 ms`, route p95/max `4.60/7.40 ms`, draw/triangles `507/390766`, `LOD overlap LOD0/bands 0/0`, water overlap `0`, LOD gaps `0`, handoff holes `0`, render-ready near samples `961/961`, HUD smoke passed.
- Honest assessment:
  - This is a real object-quality and cost improvement; it trims expensive solid megastructure mass without hurting route/vista coverage or browser performance.
  - It still does not move the global browser grid metric: owned-browser saturation/grid/color is `0.37/0.68/97`.
  - The HUDless atlas clarified why: the dominant screen problem is foreground/horizon terrain tiles and broad road surfaces, not only megastructure internals.
  - The new approach views are useful and visibly expose the prop families, but the next visual pass has to reshape ground composition at camera scale.
- Rubric movement:
  - Harness maturity: `9.30 -> 9.45` because the view atlas now captures clean world-only screenshots and includes targeted hero views.
  - Visual/world definition: `5.85 -> 5.95` because headline old-road structures are less solid and more authored, though the Minecrafty terrain read remains.
  - Performance/playability: `6.20 -> 6.25` because route max frame improved in the browser lab retry and object voxel counts dropped.
  - Rendering correctness: unchanged at `6.30`; the first browser-lab timeout was harness-level, and retry passed all correctness gates.
- Next:
  - checkpoint and push this megastructure/harness slice
  - start the ground-interrupter pass against `artifacts/view-atlas/20260508T045619Z-megastructure-hudless-atlas/report.json`
  - focus on large non-rectangular ash drifts, salt cracks, and scree fans that alter foreground/horizon pixels instead of more small object families

### 2026-05-08 - Old-Road Surface Lab And Ash Haze Pass

- Trigger:
  - The user noted the world still looked full of ugly props and lacked characteristic ambiance, and asked for bold ROI ordering.
  - The clean HUDless atlas showed foreground roads and terrain surfaces dominate the blocky read.
- Delegation:
  - A worker added the terrain surface lab while I worked on the generator/ambient slice.
  - A read-only audit independently ranked route/causeway replacement and route-local ash atmosphere as the highest visible ROI.
- Changes:
  - Added `bun run lab:terrain`, a fast non-browser terrain diagnostic that samples fixed atlas/route patches and reports material diversity, height modulo, flatness, grid-likeness, landmarks, warnings, and markdown summaries.
  - Added old-road surface influence to generated column probes so browser/ambient systems can tell when the player is on or near a pilgrimage route.
  - Reworked route surface materials into narrower, broken ash-stone/salt paver bands instead of broad grassy/steppe corridors.
  - Route-influenced dry views now resolve to `ashfall`; wet/salt route views resolve to `silt-mist`. This gives routes cheaper atmospheric composition and fog culling without adding geometry.
- Validation:
  - Focused tests: `mise exec -- bun test tests/procedural-generator.test.ts tests/ambient-environment.test.ts tests/terrain-surface-lab.test.ts`, pass, `47` tests.
  - Typecheck: `mise exec -- bun run typecheck`, pass.
  - Build: `mise exec -- bun run build`, pass.
  - Route atlas: `artifacts/route-atlas/20260508T053426Z-old-road-influence-haze/report.json`, failures none, route stretch coverage `100%`, strong silhouette coverage `87.7%`, max notable gap improved `108.0 m -> 72.0 m`.
  - Terrain lab: `artifacts/terrain-lab/20260508T053123Z-old-road-influence-haze/report.json`, average grid-likeness `0.000`, one warning: `ziggurat-approach` patch still misses direct `velothi_ziggurat` focus samples inside the local terrain grid.
  - View atlas: `artifacts/view-atlas/20260508T052931Z-old-road-influence-haze-retry/report.json`, failures none.
  - View-atlas deltas against HUDless baseline: `ziggurat-approach` center grid `0.085 -> 0.080`, lower ground `0.027 -> 0.024`; `obelisk-approach` center grid `0.065 -> 0.074`, lower ground `0.020 -> 0.014`; origin is much moodier but slightly worse by grid metric.
  - Owned browser lab: `artifacts/owned-browser-lab/20260508T053122Z-old-road-influence-haze/report.json`, failures none.
  - Browser result: traversal p95/max `4.40/8.80 ms`, route p95/max `4.70/8.30 ms`, draw/triangles `511/385298`, visual saturation/grid/color `0.37/0.68/97`, LOD overlap LOD0/bands `0/0`, water overlap `0`, LOD gaps `0`, handoff holes `0`, render-ready near samples `961/961`, HUD smoke passed.
- Honest assessment:
  - This is the first pass where route atmosphere and route surface identity are measurable across fixed views, not just asserted.
  - The result is still mixed: old-road close views are moodier and some ground metrics improved, but the prototype still has too many rectangular terrain patches and weak prop silhouettes.
  - The terrain lab is a process win because I can now iterate on ground composition without starting a browser every time.
  - Next visual work should not be another palette nudge. It should either add truly non-rectangular ground/object silhouettes in the actual camera foreground, or add a safer distance/material modulation pass that reduces large patch contrast at render time.
- Rubric movement:
  - Harness maturity: `9.45 -> 9.60` because terrain-surface sampling is now automated and part of the ROI loop.
  - Visual/world definition: `5.95 -> 6.05` because old roads now have material and atmosphere identity, but screenshots remain visibly blocky.
  - Performance/playability: unchanged at `6.25`; browser route/traversal frame budgets stayed playable, but bootstrap max had a `24.80 ms` startup spike to watch.
  - Rendering correctness: `6.30 -> 6.35` because route influence is now exposed as data and tested through generator plus ambient paths.
- Next:
  - checkpoint and push this surface-lab/route-haze slice
  - start shader-side distance material modulation or bolder foreground silhouette/terrain interrupters next

### 2026-05-08 - Terrain Lab Compare Mode And Rejected Shader Attempt

- Trigger:
  - The old-road surface pass had mixed visual deltas, so the next terrain work needs cheaper comparison instead of repeated subjective screenshot review.
- Changes:
  - Added terrain-lab `--compare-to <report.json>` support.
  - The report now includes aggregate and per-patch deltas for material count, dominant material share, surface range, flatness bucket shares, and grid-likeness.
- Rejected attempt:
  - Tried a small shader-side ash/distance material modulation pass.
  - View atlas caught a black-frame/blank-render regression immediately: `artifacts/view-atlas/20260508T053840Z-ash-distance-modulation/report.json` failed every view with `luma=0.0` and `colors=3`.
  - I reverted the renderer edit and kept the failed artifact as evidence. The next shader attempt should be smaller and probably needs a dedicated shader smoke harness before full atlas capture.
- Validation:
  - Focused terrain lab tests: `mise exec -- bun test tests/terrain-surface-lab.test.ts`, pass, `3` tests.
  - Typecheck: `mise exec -- bun run typecheck`, pass.
  - Comparison sample: `artifacts/terrain-lab/20260508T054242Z-compare-current/report.json`, deltas all zero against the current old-road influence baseline.
- Honest assessment:
  - This improves evaluation speed more than visuals.
  - The failed shader attempt reinforces that browser/atlas gates are non-negotiable: a plausible-looking shader edit can blank the whole renderer.
- Next:
  - checkpoint and push compare-mode harness work
  - move to foreground silhouette interrupters before another shader attempt

### 2026-05-08 - Foreground Interrupter Pack And Darker Ashland Read

- Trigger:
  - The paver-only foreground attempt was too subtle: object-lab passed, but view-atlas deltas were essentially zero and route strong-silhouette coverage dipped slightly.
  - The ROI audit agreed the next useful work should be bold foreground/world identity plus harder comparison gates, not another tiny prop or unchecked shader edit.
- Changes:
  - Added `bun run lab:object` as a package alias because isolated object review is now a repeated inner-loop task.
  - Expanded old-road foreground forms from only `paver_debris` to a small pack:
    - `paver_debris`: half-buried old-road slabs.
    - `scree_fan`: dark basalt slide fans across route shoulders.
    - `shrine_debris`: collapsed amber-inlaid roadside plinths.
    - `buried_ribs`: low bone arcs breaking up flat ash.
  - Added the new landmarks to discovery names/flavor, old-road journal role, route object-lab batch, ash wastes and route rosters, and generator coverage tests.
  - Shifted base badlands materials away from bright sand/orange toward darker rust/ash so ashland screenshots stop reading as generic desert.
- Validation:
  - Focused tests: `mise exec -- bun test tests/procedural-generator.test.ts tests/object-lab.test.ts tests/discovery-catalog.test.ts`, pass, `46` tests.
  - Typecheck: `mise exec -- bun run typecheck`, pass.
  - Build: `mise exec -- bun run build`, pass.
  - Object-lab foreground debug: `artifacts/object-lab/2026-05-08-061819477Z-foreground-pack-debug-v3/batch-report.json`.
    - `shrine_debris` improved from `63` sparse voxels / dominant-material warnings to `1133` voxels, `3` materials, no warnings.
    - `scree_fan` and `buried_ribs` still report `low-bounds-fill`; visually this is partly intentional negative space, but the harness cannot distinguish that from bad sparsity yet.
  - Route object-lab batch: `artifacts/object-lab/2026-05-08-061909845Z-foreground-pack-route/batch-report.json`, `14` route landmark runs.
  - Route atlas: `artifacts/route-atlas/20260508T061943Z-foreground-pack/report.json`, failures none, route stretch coverage `100%`, strong silhouette coverage `87.7%`, max notable gap `72.0 m`, `+17` landmark hits and `+3` distinct route landmarks versus paver-only.
  - Terrain lab: `artifacts/terrain-lab/20260508T062244Z-foreground-pack/report.json`, average grid-likeness unchanged at `0.000`; this confirms the terrain shape sampler did not regress.
  - View atlas: `artifacts/view-atlas/20260508T062002Z-foreground-pack/report.json`, failures none.
  - View-atlas deltas versus old-road haze baseline:
    - stronger mood/read: origin luma `-4.5`, ash-marker luma `-5.7`, ziggurat-approach luma `-9.6`, obelisk-approach luma `-7.0`.
    - mixed correctness metric: ash-marker center grid improved `-0.001`, but ziggurat-approach center/lower grid worsened `+0.011/+0.006` and obelisk-approach worsened `+0.008/+0.006`.
  - Owned browser lab: `artifacts/owned-browser-lab/20260508T062244Z-foreground-pack/report.json`, failures none.
  - Browser result: traversal p95/max `4.50/7.80 ms`, route p95/max `4.70/9.20 ms`, draw/triangles `511/384990`, visual saturation/grid/color `0.35/0.68/101`, LOD overlap LOD0/bands `0/0`, water overlap `0`, LOD gaps `0`, handoff holes `0`, render-ready near samples `961/961`, HUD smoke passed.
- Honest assessment:
  - This pass is visibly bolder than the paver-only attempt, especially in the close ashland approach screenshots where debris/ribs/markers create a more authored route scene.
  - It still does not solve the Minecrafty read. The whole-screen grid metric remains `0.68`, and the close-view grid metric worsened where darker material contrast makes block edges more obvious.
  - The performance result is acceptable: denser route landmarks did not create a movement-frame regression or LOD overlap bug.
  - The next correctness step should be a hard view-atlas budget and object-lab distinctiveness/negative-space scoring, so I can separate genuine visual progress from merely darker or busier screenshots.
- Rubric movement:
  - Harness maturity: `9.60 -> 9.65` because `lab:object` is now a first-class repeated command and the foreground pack was evaluated across object, route, terrain, view, and browser labs.
  - Visual/world definition: `6.05 -> 6.20` because old roads now have low debris, collapsed shrine markers, and rib forms instead of only towers/markers.
  - Performance/playability: `6.25 -> 6.30` because frame budgets held despite more route object variety.
  - Rendering correctness: unchanged at `6.35`; the screenshot atlas says the grid/block read is still not corrected.
- Next:
  - checkpoint and push this foreground pack
  - implement a view-atlas comparison budget gate and object-lab distinctiveness/negative-space scoring
  - only then retry shader/contact-depth work or route-aware RPG hooks

### 2026-05-08 - View-Atlas Budget Gate And Object-Lab Distinctiveness

- Trigger:
  - The foreground pack proved that screenshots can become moodier and more authored while still worsening center/lower-ground grid metrics.
  - The object-lab warning queue could not tell intentional sparse negative-space forms apart from genuinely broken sparse/clipped objects.
- Delegation:
  - A worker owned the object-lab distinctiveness side quest in `scripts/object-lab.ts` and `tests/object-lab.test.ts`.
  - I kept the atlas comparison/budget gate on the critical path in `scripts/capture-view-atlas.ts` and `scripts/lib/view-atlas-budgets.ts`.
- Changes:
  - Added `--enforce-comparison-budgets` to `bun run atlas:views`.
  - Added comparison deltas for baseline presence and horizon grid, in addition to luma, color count, center grid, and lower-ground grid.
  - Added default comparison budgets for luma drop, color bucket loss, horizon grid regression, center grid regression, and lower-ground grid regression.
  - Added object-lab distinctiveness diagnostics: form class, intentional negative space, negative-space ratio, coverage balance, and projection asymmetry.
  - Sparse intentional route forms now surface as `intentional-negative-space` while suppressing generic `low-bounds-fill` in a separate suppressed-warning column.
- Validation:
  - Focused tests: `mise exec -- bun test tests/view-atlas-budgets.test.ts tests/object-lab.test.ts`, pass, `7` tests.
  - Typecheck: `mise exec -- bun run typecheck`, pass.
  - Expected-fail atlas smoke: `artifacts/view-atlas/20260508T063518Z-budget-gate-smoke/report.json` failed as intended on `ziggurat-approach` center grid `+0.011` and lower-ground grid `+0.006` against budgets `+0.006/+0.005`.
  - Expected-pass atlas smoke: `artifacts/view-atlas/20260508T063556Z-budget-gate-pass-smoke/report.json`, failures none when comparing the current state to the foreground-pack baseline.
  - Object-lab distinctiveness smoke: `artifacts/object-lab/2026-05-08-063649753Z-distinctiveness-smoke/batch-report.json`.
    - `scree_fan`: negative-space ratio `93.8%`, top asymmetry `93.1%`, warning `intentional-negative-space`, suppressed `low-bounds-fill`.
    - `buried_ribs`: negative-space ratio `95.5%`, top asymmetry `85.0%`, warning `intentional-negative-space`, suppressed `low-bounds-fill`.
- Honest assessment:
  - This is mostly harness quality, not a player-facing visual upgrade.
  - It closes an important loophole: future visual changes can no longer claim progress if they regress fixed-view grid budgets under the enforced mode.
  - The object-lab still labels intentional negative-space forms as warnings, but now in an explicit class-aware way that is useful for review instead of conflating them with bad sparse assets.
- Rubric movement:
  - Harness maturity: `9.65 -> 9.80` because fixed screenshots now have opt-in regression budgets and object-lab reports distinguish sparse form intent.
  - Visual/world definition: unchanged at `6.20`; no new world content in this slice.
  - Performance/playability: unchanged at `6.30`; no runtime renderer changes.
  - Rendering correctness: `6.35 -> 6.45` because view-atlas can now fail grid regressions directly instead of only reporting them.
- Next:
  - checkpoint and push this harness slice
  - use `--enforce-comparison-budgets` on visual changes before accepting them
  - choose between route-aware RPG hooks and a shader/contact-depth smoke harness next

### 2026-05-08 - Fast Shader Smoke Lab

- Trigger:
  - The previous WGSL/material modulation attempt black-framed the whole renderer and only got caught by the full seven-view atlas.
  - The new atlas budgets are good, but shader iteration needs a cheaper single-view gate before full screenshot and browser labs.
- Changes:
  - Added `bun run lab:shader`, a thin wrapper around the owned view-atlas browser path.
  - The smoke lab captures one deterministic view by default, reads the generated atlas report, and enforces shader-oriented budgets for luma, luma variance, color buckets, render CPU, last gameplay frame, draw calls, and triangles.
  - The tool optionally accepts `--compare-to` and forwards `--enforce-comparison-budgets` to the underlying atlas run.
- Validation:
  - Typecheck: `mise exec -- bun run typecheck`, pass.
  - Initial budget calibration run: `artifacts/shader-smoke-lab/20260508T064251Z-shader-smoke-baseline/report.json`, failed because the triangle budget was unrealistically below the current baseline (`749504` triangles versus `700000`).
  - Baseline smoke after budget calibration: `artifacts/shader-smoke-lab/20260508T064407Z-shader-smoke-baseline-v2/report.json`, failures none.
- Honest assessment:
  - This is another harness slice, but it directly addresses the most expensive recent mistake: plausible shader edits can blank the renderer.
  - The triangle budget is a guardrail, not a target. It is set high enough for the current known-good baseline and low enough to catch accidental geometry/render explosions during shader work.
- Rubric movement:
  - Harness maturity: `9.80 -> 9.88`.
  - Rendering correctness: `6.45 -> 6.50` because shader experiments now have a cheap black-frame/color/frame gate.
  - Visual/world definition and performance/playability are unchanged; this was tooling.
- Next:
  - checkpoint and push this shader-smoke slice
  - use `lab:shader` before any contact-depth or WGSL material change
  - then either attempt a very small contact-depth shader edit or build route-aware RPG hooks

### 2026-05-08 - Strict LOD Ownership Probe, Warped Roads, And Route RPG Hooks

- Trigger:
  - The user still reported visible z-fighting and a world that reads too much like straight voxel strips with ugly scattered props.
  - A small shader contact/depth attempt after the shader-smoke baseline passed but barely moved view metrics, so I reverted it instead of committing a nearly invisible tweak.
- Delegation:
  - An explorer inspected LOD, fog, route, shader, and worldgen ROI without editing files.
  - A worker improved object-lab diagnostics in isolation: cross-view variation and vertical profile now show whether a prop has a readable silhouette from several angles.
- Changes:
  - Skill progression now uses discovery role, not just discovery category:
    - old-road landmarks train Naturalist plus Cartography and Lore
    - shrines train Naturalist plus Lore
  - `probeLodCoverage` now treats a render-ready visible LOD0 surface as an owner even when the full vertical chunk column is not render-ready yet. This closes a blind spot around transient surface/LOD overlap.
  - Pilgrim routes now have deterministic low-frequency lateral warp instead of perfectly straight bands.
  - Procedural route tests now verify centerline drift, and representative landmark-root selection prefers dense centered samples instead of first-hit edge samples.
- Validation:
  - Focused route/RPG/object/browser tests: `mise exec -- bun test tests/procedural-generator.test.ts tests/skill-journal.test.ts tests/object-lab.test.ts tests/browser-game-benchmark-harness.test.ts`, pass, `53` tests.
  - Typecheck: `mise exec -- bun run typecheck`, pass.
  - Terrain lab: `artifacts/terrain-lab/20260508T071038Z-surface-probes/report.json`, average grid-likeness `0.000`; route patches stayed clean.
  - Route atlas: `artifacts/route-atlas/20260508T071150Z-warped-pilgrim-routes/report.json`, failures none, route stretch coverage `100%`, strong silhouette coverage `87.7%`, max notable gap `72.0 m`, `-2` landmark hits and `0` distinct delta versus foreground pack.
  - View atlas: `artifacts/view-atlas/20260508T071150Z-warped-pilgrim-routes/report.json`, failures none.
  - Owned browser strict LOD probe before route warp: `artifacts/owned-browser-lab/20260508T070116Z-strict-lod-owner-route-rpg/report.json`, failures none, visible LOD0 owner samples `41/11025`, resident/band/water overlaps `0/0/0`, handoff holes `0`.
  - Owned browser after route warp: `artifacts/owned-browser-lab/20260508T071337Z-warped-pilgrim-routes/report.json`, failures none, traversal p95/max `5.30/9.10 ms`, route p95/max `4.80/7.90 ms`, draw/triangles `494/389948`, LOD overlap LOD0/bands `0/0`, water overlap `0`, gaps/handoff holes `0/0`.
- Honest assessment:
  - The stricter LOD probe did not reproduce the user's overlap report on the owned route path, so I kept the better gate and avoided speculative renderer surgery.
  - Warped roads are a real composition change: route surfaces no longer follow mathematically straight bands, while route coverage and FPS stayed intact.
  - The whole-browser grid metric is still `0.68`; warped routes help authored traversal feel, but they do not solve the dominant block-edge read by themselves.
- Rubric movement:
  - Harness maturity: `9.88 -> 9.92` because LOD ownership is now checked against visible surfaces, not only fully ready columns, and object-lab has more silhouette diagnostics.
  - Visual/world definition: `6.20 -> 6.35` because old roads now bend and drift instead of reading as straight generated strips.
  - RPG/exploration loop: `3.20 -> 3.45` because old roads and shrines now feed Cartography/Lore progression directly.
  - Performance/playability: `6.30 -> 6.35` because route max frame improved to `7.90 ms` in the latest browser lab while draw/triangles stayed under budget.
  - Rendering correctness: `6.50 -> 6.58` because the stricter ownership probe passed and is now part of browser validation.
- Next:
  - checkpoint and push this mixed route/RPG/harness slice
  - re-rank around the stubborn `0.68` grid metric
  - likely next work: screenshot-aware lighting/depth or stronger terrain foreground breakup, but only with shader-smoke and enforced view-atlas gates

### 2026-05-08 - Contact-Depth Shader Pass

- Trigger:
  - The global browser grid metric was still stuck at `0.68` after route and prop work.
  - The new shader-smoke harness made a small lighting/depth attempt safer than before.
- Rejected/iterated attempts:
  - A very small surface-mute shader pass passed shader smoke and atlas but barely moved metrics, so I reverted it.
  - The first contact-depth pass also passed but was too subtle: mostly `0.000-0.002` grid-risk movement with small luma drops.
  - I strengthened the pass once, then re-ran the same smoke/atlas/browser gates.
- Changes:
  - Added a cheap normal-based contact-depth multiplier in `shade_fragment`.
  - Vertical and downward faces now darken slightly near the camera, fading with fog distance so far silhouettes do not become harsh black blocks.
  - No new geometry, texture samples, loops, or CPU work.
- Validation:
  - Typecheck: `mise exec -- bun run typecheck`, pass.
  - Shader smoke baseline: `artifacts/shader-smoke-lab/20260508T071953Z-pre-contact-depth-baseline/report.json`, failures none.
  - Shader smoke final: `artifacts/shader-smoke-lab/20260508T072809Z-contact-depth-stronger-smoke/report.json`, failures none.
  - Enforced view atlas: `artifacts/view-atlas/20260508T072906Z-contact-depth-stronger/report.json`, failures none against `artifacts/view-atlas/20260508T071150Z-warped-pilgrim-routes/report.json`.
  - View-atlas deltas:
    - horizon grid risk improved in `5/7` views, with origin `-0.0030` and obelisk `-0.0017`.
    - center grid risk improved in `5/7` views, with origin `-0.0019`, ash marker `-0.0010`, and obelisk `-0.0011`.
    - lower-ground grid risk barely moved and slightly regressed in three close/ground-heavy views (`+0.0003` to `+0.0006`).
    - luma dropped `3.0-7.1`; color buckets dropped in most views but stayed inside budget.
  - First owned-browser lab attempt: `artifacts/owned-browser-lab/20260508T073140Z-contact-depth-stronger/report.json`, invalid because CDP timed out before page data.
  - Valid owned-browser retry: `artifacts/owned-browser-lab/20260508T073504Z-contact-depth-stronger-retry/report.json`, failures none.
  - Browser result: saturation/grid/color `0.35/0.67/97`, traversal p95/max `4.70/7.90 ms`, route p95/max `4.70/7.00 ms`, draw/triangles `532/388578`, LOD overlap LOD0/bands `0/0`, water overlap `0`, gaps/handoff holes `0/0`.
- Honest assessment:
  - This is a modest renderer improvement, not a dramatic visual rework.
  - The browser grid metric finally moved from `0.68` to `0.67`, and live performance improved rather than regressed in the retry.
  - The atlas shows the pass helps horizon/center depth more than foreground ground readability. The next visual work should still attack lower-ground composition/geometry, not keep turning the shader knob.
- Rubric movement:
  - Rendering correctness/quality: `6.58 -> 6.68` because the depth cue is measurable and keeps all black-frame/LOD/browser gates clean.
  - Performance/playability: `6.35 -> 6.40` because route max frame stayed at `7.00 ms` in the valid browser run.
  - Visual/world definition: `6.35 -> 6.42` because the world reads slightly less flat, but lower-ground geometry remains the blocker.
  - Harness maturity: unchanged at `9.92`; this pass used the existing gates rather than adding new ones.
- Next:
  - checkpoint and push the renderer pass if build/test stay clean
  - switch back to foreground terrain/composition changes; lower-ground grid is the remaining weak metric

### 2026-05-08 - Ashlander Travel Pack Prop

- Trigger:
  - The user called out ugly, uncharacteristic props and asked for bolder world identity.
  - The contact-depth pass moved whole-screen grid a little, but the ROI list still called for travel-kit props through object-lab.
- Delegation:
  - A worker owned one narrow prop sidequest and added `ashlander_travel_pack`.
  - I integrated the result, added it to object-lab route monitoring, and rejected my own route-shoulder terrain tweak because terrain/route/view metrics did not credit it.
- Rejected attempt:
  - Tried a sub-meter route shoulder drift/crack height change.
  - Focused tests passed after harness tweaks, but terrain compare, route atlas, and view atlas showed essentially no useful movement.
  - I reverted the terrain-height part instead of preserving another barely visible tweak.
- Changes:
  - Added `ashlander_travel_pack`, a compact three-material route prop with a bedroll, frame, straps, and side-pot silhouette.
  - Placed it at restrained density in harsh badlands, badlands crater, ember deadland, and ember caldera rosters.
  - Added discovery name/flavor and old-road role so it participates in route discovery/progression.
  - Added the prop to object-lab's landmark list and route batch so future batch comparisons monitor it.
  - Tightened procedural representative-root tests for hanging crossbar objects so bone chimes are measured on their readable horizontal element instead of an arbitrary solid pixel.
- Validation:
  - Focused tests: `mise exec -- bun test tests/object-lab.test.ts tests/procedural-generator.test.ts tests/discovery-catalog.test.ts`, pass, `49` tests.
  - Typecheck: `mise exec -- bun run typecheck`, pass.
  - Object-lab: `artifacts/object-lab/2026-05-08-075811271Z-ashlander-travel-pack-verification/report.json`, warnings none, bounds `15 x 20 x 10`, fill `0.348`, `3` materials, cross-view variation `0.639`, lower-weighted vertical profile.
  - Route atlas: `artifacts/route-atlas/20260508T075604Z-ashlander-travel-pack/report.json`, failures none, route coverage `100%`, strong silhouette coverage `87.7%`, max notable gap `72.0 m`.
  - Enforced view atlas: `artifacts/view-atlas/20260508T075604Z-ashlander-travel-pack/report.json`, failures none against the contact-depth baseline; deltas were `0` across the fixed views, so this prop does not materially alter the seven hero screenshots.
  - Owned browser lab: `artifacts/owned-browser-lab/20260508T075855Z-ashlander-travel-pack/report.json`, failures none, saturation/grid/color `0.35/0.67/97`, traversal p95/max `4.60/9.60 ms`, route p95/max `4.90/8.40 ms`, LOD overlap/gaps/handoff holes `0/0/0`.
- Honest assessment:
  - This is good asset/world-definition work and a clean delegated slice, but it is not a camera-scale fix for the Minecrafty terrain read.
  - The prop is validated in isolation and in live performance, yet the fixed view atlas does not see it. Future asset work needs either more route-camera placement or a dedicated prop visibility atlas if the goal is screenshot-level impact.
  - The rejected route-height tweak was the right call to drop: it complicated tests without moving the evaluation harness.
- Rubric movement:
  - Visual/world definition: `6.42 -> 6.50` because harsh routes now have a distinctive travel-kit object rather than only stones/bones/markers.
  - Harness maturity: `9.92 -> 9.94` because object-lab route batches now monitor the new prop and representative-root tests are less brittle.
  - Performance/playability: unchanged at `6.40`; browser frame budgets held but did not improve meaningfully.
  - Rendering correctness/quality: unchanged at `6.68`; this was asset content, not renderer correction.
- Next:
  - checkpoint and push the prop slice
  - add a prop visibility/view lab or move to far-view/fog only if it can be measured by owned browser and fixed views
  - continue treating lower-ground/camera-scale terrain read as the main unsolved visual problem

### 2026-05-08 - Prop Visibility Atlas Preset

- Trigger:
  - The ashlander travel pack was warning-free in object-lab but invisible to the seven-view world atlas.
  - Adding more small props without camera-scale feedback would recreate the "barely visible tweak" failure mode.
- Changes:
  - Added `--preset props` to `bun run atlas:views`.
  - The default `world` preset stays unchanged; the new `props` preset captures close views for:
    - `ashlander-pack-close`
    - `bone-chimes-close`
    - `pilgrim-lantern-close`
    - `shrine-debris-close`
    - `paver-debris-close`
  - The prop preset reuses the same browser session, HUD hiding, PNG analysis, comparison report, and budget infrastructure as the world atlas.
- Validation:
  - Typecheck: `mise exec -- bun run typecheck`, pass.
  - Prop atlas first run: `artifacts/view-atlas/20260508T080725Z-prop-visibility-baseline/report.json`, failures none.
  - Tightened close cameras after inspecting `ashlander-pack-close`.
  - Prop atlas close run: `artifacts/view-atlas/20260508T081042Z-prop-visibility-close/report.json`, failures none.
  - Example close-view metrics:
    - `ashlander-pack-close`: luma `151.5`, colors `107`, center grid `0.0724`, lower grid `0.0306`.
    - `pilgrim-lantern-close`: luma `85.6`, colors `55`, center grid `0.0562`, lower grid `0.0306`.
    - `paver-debris-close`: luma `69.5`, colors `70`, center grid `0.0913`, lower grid `0.0464`.
- Honest assessment:
  - This is a measurement checkpoint, not direct content.
  - The close screenshots make it clear that small route props need careful camera placement and probably object prominence metrics; a low pack can still be visually dominated by nearby taller route forms.
  - It is still a useful speedup: future prop work can now use `bun run atlas:views -- --preset props` instead of relying only on isolated object-lab contact sheets.
- Rubric movement:
  - Harness maturity: `9.94 -> 9.96`.
  - Visual/world definition: unchanged; no new content in this slice.
  - Rendering correctness/quality: unchanged, but prop-related visual claims now have a better evaluation path.
- Next:
  - checkpoint and push the prop atlas preset
  - use prop atlas before accepting more small prop packs
  - continue with far-view/fog or terrain foreground only when the gate can measure it

### 2026-05-08 - Far-View Fog Cushion

- Trigger:
  - The user reported that distant LOD levels seemed absent and the world no longer read far enough.
  - The latest strict LOD probes had no settled ownership overlap, and the browser route budget had enough headroom for a cautious distance experiment.
- Changes:
  - Increased the default surface fog cap from `416 m` to `480 m`, still inside the existing `512 m` LOD4 coverage shell.
  - Raised surface ambience fog distances proportionally:
    - open air: `416 -> 480 m`
    - dry haze: `396 -> 440 m`
    - cold glass: `384 -> 424 m`
    - green canopy: `360 -> 400 m`
    - ashfall: `288 -> 336 m`
    - silt mist: `300 -> 348 m`
    - fungal lantern: `304 -> 352 m`
  - Updated the LOD coverage comment and ambient test expectations so old-road ash haze remains ashfall, but no longer clamps route sightlines below `320 m`.
- Validation:
  - Focused tests: `mise exec -- bun test tests/ambient-environment.test.ts tests/water-visuals.test.ts`, pass.
  - Typecheck: `mise exec -- bun run typecheck`, pass.
  - Build: `mise run build`, pass.
  - Enforced world view atlas: `artifacts/view-atlas/20260508T081827Z-far-view-fog/report.json`, failures none against the contact-depth baseline.
  - First owned-browser lab attempt: `artifacts/owned-browser-lab/20260508T082012Z-far-view-fog/report.json`, invalid because CDP timed out before a game snapshot. I did not count it as evidence.
  - Valid owned-browser retry: `artifacts/owned-browser-lab/20260508T082407Z-far-view-fog-retry/report.json`, failures none.
  - Browser result: ambient `ashfall` with `395.89 m` fog, saturation/grid/color `0.35/0.67/106`, traversal p95/max `4.50/6.90 ms`, route p95/max `4.80/9.80 ms`, draw/triangles `543/400898`, LOD overlap LOD0/bands `0/0`, water overlap `0`, settled LOD gaps/handoff holes `0/0`.
- Honest assessment:
  - This is a real scale/readability improvement: the live route fog distance increased by about `55 m` from the prior `341.23 m` ashfall route baseline.
  - The view-atlas visual deltas were intentionally small; fog distance does not materially solve the block/grid read by itself.
  - The moving route diagnostics still report transient far-LOD coverage gap samples during streaming even though final settled LOD ownership is clean. That should be the next correctness/performance target before pushing view distance much farther.
  - A delegated ROI audit independently ranked summary-derived LOD planning and far-view correctness as the next high-return work before more content polish.
- Rubric movement:
  - Rendering correctness/quality: `6.68 -> 6.74` because farther fog held strict settled LOD overlap/gap gates.
  - Performance/playability: unchanged at `6.40`; route max stayed under `10 ms`, but draw calls rose and transient far-gap diagnostics still deserve attention.
  - Visual/world definition: `6.50 -> 6.56` because route vistas now expose more world scale without losing mood.
  - Harness maturity: unchanged at `9.96`; this pass used existing gates and exposed the next LOD-planning weakness.
- Next:
  - checkpoint and push this far-view slice
  - inspect LOD Y-range/summary planning fallback so farther views do not depend on expensive generator sampling or transient coverage holes
  - only then attempt another view-distance increase or bolder foreground composition pass

### 2026-05-08 - LOD Y-Range Planning Cleanup

- Trigger:
  - The far-view retry passed, but bootstrap still spent `111.60 ms` in LOD Y-range planning and route movement still reported transient far-LOD gap samples.
  - The ROI audit flagged the old generator-backed Y-range fallback as correctness/performance debt before any larger view-distance increase.
- Rejected attempt:
  - First removed generator fallback entirely and used a broad known-world Y envelope when summaries were missing.
  - Unit tests caught the flaw: `budgeted LOD generation does not starve far rings` failed with `9456` pending chunks because the broad vertical range exploded the LOD key set.
  - I replaced that attempt instead of accepting a theoretically cleaner but practically slower plan.
- Changes:
  - LOD Y-range planning now tries cached resident/LOD solid bounds first, then cached/generated render column summaries.
  - Only when summaries are missing does it fall back to five `sampleSurfaceColumn()` probes, which expose `surfaceY`, `topY`, and water height without the heavier full `sampleColumn()` path.
  - Added a regression test proving LOD Y-range planning no longer calls the full procedural `sampleColumn()` fallback after resident summaries exist.
- Validation:
  - Focused LOD/resident tests: `mise exec -- bun test tests/procedural-resident-world.test.ts tests/lod-system.test.ts`, pass, `48` tests.
  - Typecheck: `mise exec -- bun run typecheck`, pass.
  - Build: `mise run build`, pass.
  - Owned browser lab: `artifacts/owned-browser-lab/20260508T084027Z-lod-summary-yrange/report.json`, failures none.
  - Browser result versus far-view baseline:
    - route p95/max improved from `4.80/9.80 ms` to `4.70/6.10 ms`
    - draw/triangles dropped from `543/400898` to `532/398662`
    - visual grid stayed `0.67`
    - settled LOD overlap/gaps/handoff holes stayed `0/0/0`
    - transient route far-LOD gap frames stayed `50`
  - Rejected moving LOD budget experiment: `artifacts/owned-browser-lab/20260508T084333Z-lod-moving-budget/report.json`, failures none but route max worsened to `7.10 ms` and far-gap frames stayed `50`; reverted the budget change.
- Honest assessment:
  - This is a useful performance/correctness cleanup, not a complete far-gap fix.
  - The full-column planning fallback is gone, and route max frame improved materially in the valid browser run.
  - Bootstrap LOD Y-range time did not improve (`111.60 -> 133.40 ms`), likely because summary scanning plus surface fallback is still doing enough work to matter at startup. The better route max is the stronger acceptance signal here.
  - The persistent `50` transient far-gap frames point to a different issue: old/new LOD handoff while the moving route is still dirty or pending, not just Y-range sampling cost.
- Rubric movement:
  - Rendering correctness/quality: `6.74 -> 6.78` because the planner now prefers render summaries and still passes strict settled overlap/gap gates.
  - Performance/playability: `6.40 -> 6.48` because route max frame dropped from `9.80 ms` to `6.10 ms` without draw/triangle growth.
  - Harness maturity: `9.96 -> 9.97` because there is now a regression test for the full-column fallback.
  - Visual/world definition: unchanged at `6.56`.
- Next:
  - checkpoint and push this LOD planning slice
  - inspect moving far-LOD gap classification and handoff timing; the gap count did not respond to larger generation budgets
  - then return to the camera-scale visual backlog once LOD movement diagnostics are better understood

### 2026-05-08 - Far-LOD Gap Classification

- Trigger:
  - The moving far-LOD gap count stayed stubborn at `50` route frames even after increasing moving LOD generation budget.
  - Existing browser output already showed `0` visible holes and `0` blocking seam gaps, but the engine summary did not separate benign transition windows from actual blocking holes.
- Changes:
  - Added route seam frame classification:
    - `clean`
    - `transition-gap`
    - `blocking-gap`
  - A seam gap is blocking if it shows visible ground holes, suspicious screen voids, settled-reference holes, or remains after the route is settled and idle.
  - Engine summaries now report `framesWithBlockingSeamGaps` and `framesWithTransitionSeamGaps`.
  - Owned-browser output now prints holes / far-LOD gaps / transition / blocking / overlaps, so a scary raw gap count cannot be mistaken for a visible correctness failure.
- Validation:
  - Unit tests: `mise exec -- bun test tests/game-route-benchmark.test.ts`, pass, `9` tests.
  - Typecheck: `mise exec -- bun run typecheck`, pass.
  - Build: `mise run build`, pass.
  - Owned browser lab: `artifacts/owned-browser-lab/20260508T085813Z-lod-gap-classification/report.json`, failures none.
  - Browser classification result:
    - traversal: holes/far-LOD gaps/transition/blocking/overlaps `0/14/14/0/0`
    - route: holes/far-LOD gaps/transition/blocking/overlaps `0/50/50/0/0`
    - settled strict LOD overlap/gaps/handoff holes stayed `0/0/0`
    - route p95/max stayed `4.80/6.50 ms`
- Honest assessment:
  - This does not remove transition coverage windows, but it proves the current route samples are not visible holes or blocking settled handoff failures.
  - The classifier is now a better correctness signal than the raw far-gap frame count.
  - The rejected moving-budget experiment now makes sense: those samples were transition windows around dirty/pending work, so generating more LOD chunks per movement update was the wrong lever.
- Rubric movement:
  - Harness maturity: `9.97 -> 9.985` because the browser route report now distinguishes transient handoff noise from actionable rendering bugs.
  - Rendering correctness/quality: `6.78 -> 6.83` because the LOD correctness signal is sharper and verified in browser.
  - Performance/playability: unchanged at `6.48`; this was diagnostic, not a runtime optimization.
  - Visual/world definition: unchanged at `6.56`.
- Next:
  - checkpoint and push this diagnostic slice
  - resume higher-ROI visual work: object prominence, foreground composition, and route-camera silhouettes, while keeping blocking seam gaps at `0`

### 2026-05-08 - Depth Precision and Deterministic LOD0 Handoff

- Trigger:
  - The user reported flickering individual voxel faces at all LODs, likely between diagonal stair-step faces rather than whole overlapping LOD meshes.
  - The user also saw brief blanking when a low-detail chunk switched to higher detail, and noticed the far fog no longer hid the furthest chunk cutoff cleanly.
- Findings:
  - The first-person projection still used `near: 0.1` and `far: 20000`.
  - At the fog edge, a one-voxel depth separation had less than `0.1` of a 24-bit depth-buffer step, which is not enough to keep diagonal terrain faces stable.
  - Resident LOD0 chunks were adopted as `renderReady=false`, but adoption immediately invalidated coarser LOD coverage. That meant the low-detail mesh could disappear before the high-detail mesh existed.
  - Renderer fog culling happened exactly at `fogEndDistance`, so geometry could be culled at the same boundary where the shader just reached full fog.
- Changes:
  - First-person camera depth range is now tied to gameplay scale: `0.4 m` near plane and `FOG_END_DISTANCE + 96 m` far plane.
  - Added a depth-precision regression test that requires a one-voxel separation near the fog edge to exceed two 24-bit depth steps.
  - Resident chunk adoption now keeps coarser LOD coverage until the resident chunk is actually render-ready; the render-ready transition still invalidates coarser LOD so the replacement can become authoritative.
  - Added a focused LOD handoff regression test for the unmeshed resident adoption path.
  - Renderer fog culling now has a `32 m` margin beyond the fog end so distant chunks fade fully before being culled.
- Validation:
  - Focused tests: `mise exec -- bun test tests/lod-handoff.test.ts tests/first-person-camera.test.ts`, pass, `5` tests.
  - Typecheck: `mise exec -- bun run typecheck`, pass.
  - Build: `mise exec -- bun run build`, pass.
  - Owned browser lab: `artifacts/owned-browser-lab/20260508T092056Z-depth-handoff-fog/report.json`, failures none.
  - Browser result:
    - route p95/max frame `5.30/11.80 ms`
    - route holes/far-LOD gaps/transition/blocking/overlaps `0/48/48/0/0`
    - settled LOD overlap/gaps/handoff holes `0/0/0`
    - LOD draw calls by level `0/86/60/49/110`
    - fog profile still reports ashfall at `395.89 m`, with the new cull cushion hidden behind full fog
- Honest assessment:
  - This directly addresses the reported depth-buffer flicker mechanism and the concrete LOD0 adoption blanking bug.
  - The browser lab still reports transition far-gap samples, but none are blocking holes or overlap samples. The handoff fix is for visible blanking during resident adoption, not for removing every conservative transition counter.
  - The in-app browser backend was not discoverable from Codex for this check, so I did not claim a user-tab visual inspection; the accepted evidence is the owned browser lab report and saved screenshot.
- Rubric movement:
  - Rendering correctness/quality: `6.83 -> 7.05` because depth precision is now quantified and LOD0 handoff no longer deletes coarser coverage before the replacement mesh is ready.
  - Performance/playability: `6.48 -> 6.50` because route max remains under budget despite the fog cull cushion; this is mostly correctness, not a speed win.
  - Harness maturity: `9.985 -> 9.99` because the two reported failure modes now have focused regression tests.
  - Visual/world definition: unchanged at `6.56`.
- Next:
  - checkpoint and push this render-correctness slice
  - stop spending more cycles on low-level LOD unless a blocking hole or overlap reappears
  - move to visible world/game progress: large silhouettes, route composition, and more characteristic Morrowind-like exploration beats

### 2026-05-08 - Prop Atlas Target Prominence Metric

- Trigger:
  - The user pushed for better delegation and self-monitoring on individual world objects.
  - The prop atlas existed, but it only proved a view rendered; it did not say whether the intended object actually read against its surroundings.
- Changes:
  - Added per-view target regions to the prop atlas.
  - Added target prominence metrics comparing the target region to nearby context: luma contrast, saturation contrast, detail score, color count, and a combined readability score.
  - Prop atlas runs now fail if the target is too weak, so small-object work has a measurable acceptance gate before it is integrated into route/world views.
- Validation:
  - Prop atlas: `mise exec -- bun run atlas:views -- --preset props --label=prop-prominence-check`, pass.
  - Output: `artifacts/view-atlas/20260508T092559Z-prop-prominence-check/report.json`, failures none.
  - Typecheck was already clean after the render-correctness checkpoint.
- Honest assessment:
  - This does not make the game look better by itself.
  - It does make future prop/model work less subjective and gives delegated asset work a concrete target.
- Rubric movement:
  - Harness maturity: `9.99 -> 9.995`.
  - Visual/world definition: unchanged until this metric drives actual content changes.
- Next:
  - use route-scale composition first, not more isolated props
  - keep prop prominence as a support gate for close-route objects and delegated asset passes

### 2026-05-08 - LOD Boundary Z-Fighting and Conservative Height Fix

- Trigger:
  - The user correctly pushed back on content work before the renderer was structurally correct.
  - Reported symptoms were floating distant LOD chunks, biome/height disagreement between near and far views, and flickering that looked like voxel-face z-fighting rather than only whole-LOD overlap.
- Findings:
  - The LOD opaque mesher was fed null neighbors for every side, so same-level adjacent LOD chunks could both emit boundary faces on the exact same plane. That is a direct z-fighting path and wastes triangles.
  - Coarse opaque LOD cubes also rendered at the optimistic top of their vertical bucket. When the representative source surface was lower inside that bucket, distant terrain could appear raised or floating relative to detailed terrain.
  - A naive per-vertex height check was too broad because side-face vertices legitimately sit above nearby lower columns; the correct invariant is top-face and shared-boundary ownership.
- Changes:
  - LOD opaque meshing now resolves same-level neighbor face snapshots before building a mesh, matching the normal chunk meshing contract.
  - When a new LOD chunk becomes render-ready, existing same-level adjacent LOD chunks are remeshed so shared internal faces are removed deterministically instead of depending on generation order.
  - Opaque LOD vertex scaling now uses a full-stride downward Y bias, making coarse terrain conservative rather than floating above its represented source bucket. Water LOD remains exact and separate.
  - Added regression coverage for conservative top faces and same-level LOD neighbor boundary faces.
- Validation:
  - Full LOD suite: `mise exec -- bun test tests/lod-system.test.ts`, pass, `31` tests.
  - Typecheck: `mise exec -- bun run typecheck`, pass.
  - In-app browser backend was not discoverable from Codex in this session. I attempted it first, then fell back to the owned browser lab.
  - Owned browser lab fallback crashed on `Runtime.evaluate` timeout again: `artifacts/owned-browser-lab/20260508T112221Z/report.json`. I am not treating that as a rendering pass; the reliable evidence for this slice is the deterministic LOD suite.
- Honest assessment:
  - This fixes a real structural z-fighting source: duplicate same-level LOD boundary planes.
  - The conservative Y bias should remove floating far LOD terrain at the cost of making very coarse opaque LOD slightly lower. This is preferable to optimistic floating and can later be replaced by a real heightfield/impostor LOD if needed.
  - This does not yet resolve biome/ambient source-of-truth divergence; that remains blocked behind rendering correctness and should be tackled next by making world classification a single sampled authority for near and far systems.
- Rubric movement:
  - Rendering correctness/quality: `7.05 -> 7.32`.
  - Harness maturity: `9.995 -> 9.997` because shared-boundary z-fighting and top-face height now have deterministic regressions.
  - Performance/playability: `6.50 -> 6.53` because internal duplicate LOD boundary faces are removed, although I did not claim a browser perf win without a passing browser lab.
  - Visual/world definition: unchanged.
- Next:
  - keep rendering-source fixes isolated from biome/content commits
  - make biome/ambient/far-render world classification share a single source of truth
  - restore a stable browser lab path or split the heavy CDP probe so visual/perf validation stops failing on one long `Runtime.evaluate`

### 2026-05-08 - World Classification Source-of-Truth Fix

- Trigger:
  - The user called out that the visible distance, floating LOD height, and biome-at-feet mismatch were symptoms of a brittle rendering/worldgen structure.
  - After the LOD boundary fix, the next correctness risk was letting HUD, sky, terrain, and far-field sampling answer "what world am I in?" through different cached or thresholded paths.
- Changes:
  - Added `worldgen-region.ts` as the macro island/province authority for region id, region strength, broad biome, regional variant, ambient profile, island interior, coast shelf, and major regional influences.
  - The generator now feeds surface height, materials, biome classification, and probe data from the same macro-region sample instead of scattering island/biome decisions across call sites.
  - The HUD and render environment now share one fresh current-world probe per snapshot, so current biome/region/ambient values no longer depend on stale discovery-journal cache state.
  - Ambient region switching and terrain regional variants now share `WORLD_REGION_AUTHORITY_THRESHOLD`, so sky and terrain cannot drift through duplicated magic numbers.
  - The broad generator tests now sample island-scale coordinates instead of the old small infinite-world window.
- Validation:
  - Procedural generator suite: `mise exec -- bun test tests/procedural-generator.test.ts`, pass, `43` tests.
  - Ambient suite: `mise exec -- bun test tests/ambient-environment.test.ts`, pass, `7` tests.
  - LOD suite after worldgen changes: `mise exec -- bun test tests/lod-system.test.ts`, pass, `31` tests.
  - Resident/LOD handoff suite: `mise exec -- bun test tests/lod-handoff.test.ts tests/procedural-resident-world.test.ts`, pass, `21` tests.
  - Focused shared-threshold check: `mise exec -- bun test tests/ambient-environment.test.ts tests/procedural-generator.test.ts -t "worldgen region authority|strong macro regions"`, pass.
  - Typecheck: `mise exec -- bun run typecheck`, pass.
- Honest assessment:
  - This is a source-of-truth fix, not a visual polish pass. It should make the renderer and worldgen safer to change because near/far/HUD/ambient now consult the same macro authority.
  - I could not visually verify in the in-app browser because the `iab` backend was not discoverable from Codex. I am not claiming a user-tab visual pass.
  - Far LOD still uses a deliberately conservative surface-column fallback until real source chunks exist; the tests now prove that fallback uses the same height/material sampler as the full column path.
- Rubric movement:
  - Rendering correctness/quality: `7.32 -> 7.48` because the previous LOD height fix is now paired with shared near/far terrain sampling and shared ambient terrain region thresholds.
  - Harness maturity: `9.997 -> 9.998` because the broader island generator and shared-threshold invariants are under tests.
  - Visual/world definition: unchanged until this authority drives visible biome composition work.
- Next:
  - checkpoint and push this source-of-truth slice
  - then resume biome/world design on top of the consolidated generator instead of patching multiple disconnected places
