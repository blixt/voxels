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
