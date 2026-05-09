# 2026-05-09 Subagent Execution Board

## Objective

Return to the active long-term goal with a task graph that puts renderer correctness and performance verification first, then unlocks bold world, art, and gameplay changes without letting any subsystem invent a second source of truth.

Target outcome:

- no visible or measured LOD gaps, overlaps, floating chunks, or z-fighting-like ownership bugs;
- true frame/hitch reporting, including movement and LOD transition spikes;
- canonical chunks plus edit deltas as durable world authority;
- finite Morrowind-like island geography with large coherent regions, routes, caves, and landmark cadence;
- RPG-style exploration, discovery, skills, inspect/read/use verbs, and later passive encounters;
- every checkpoint measured, documented, committed, and pushed.

## P0 Rule

Content work can proceed in pure modules and tools, but content is not accepted into the game loop until the render gate is clean. The renderer must be trustworthy enough that screenshots and route probes mean something.

Current P0 is now more specific than the older board: `OpaqueChunkMesher` supports render-only clip masks. The next renderer checkpoint is to replace unsafe data-punching semantics with deterministic canonical-payload plus render-instance clipping, then prove it with unit tests and browser LOD persistence artifacts.

## Multi-Step Plan

### Step 1 - Make Measurement Authoritative

Owner: verification worker, with self reviewing the final gates.

Deliverables:

- central budget module;
- one RPG verification command that collects existing route, browser, LOD, and view artifacts;
- gate summary that separates correctness failures from performance or settle-budget failures;
- diary/rubric draft generated from real artifacts only.

Dependencies:

- can start immediately;
- must not require renderer changes;
- blocks subjective visual/content acceptance.

Acceptance:

- LOD gap/overlap gates are explicit;
- p95/max frame and hitch buckets are reported;
- artifact manifest records command lines and JSON paths.

### Step 2 - Stabilize LOD Ownership

Owner: self.

Deliverables:

- canonical derived LOD payload remains unmodified;
- active render instance carries a transient clip mask or equivalent visibility ownership state;
- mesher receives clip masks from residency when finer active coverage owns the same space;
- prepared/retained/stale chunks are never yielded unless explicitly active;
- same canonical payload can be reused with different render clip masks.

Dependencies:

- uses the mesher clip-mask foundation already pushed;
- must land before any generator rewrite is accepted into the running game.

Acceptance:

- `typecheck`;
- focused mesher/LOD tests;
- default LOD persistence pass;
- far-transition stress reports `0` uncovered gaps, handoff holes, resident overlaps, band overlaps, and water overlaps;
- remaining pending chunks, if any, are classified as non-visible settle backlog rather than correctness failure.

### Step 3 - Prepare Canonical Storage

Owner: storage worker.

Deliverables:

- pure edit-journal module with pack/replay/compact behavior;
- revision-aware record shapes documented and tested;
- no production cache migration until the pure contracts are proven.

Dependencies:

- can start immediately in new files;
- production IndexedDB integration waits for Step 2 and review.

Acceptance:

- edit replay is deterministic;
- repeated voxel edits collapse correctly;
- import/export is stable across negative chunk coordinates and sparse chunks.

### Step 4 - Make the Island Source of Truth

Owner: world atlas worker.

Deliverables:

- tested finite island mask and region graph in `WorldAtlas`;
- route/cave/landmark definitions as data, not generator side effects;
- validation tests for region centers, edge blends, outside-island ocean classification, and route anchors.

Dependencies:

- pure atlas work can start now;
- `ProceduralWorldGenerator` rewrites wait for Step 1 and Step 2 gates.

Acceptance:

- all eight macro regions have stable centers and meaningful area;
- out-of-island sampling returns ocean/shelf/deep water identity;
- route anchors and region edges are deterministic and test-covered.

### Step 5 - Build RPG Gameplay as Pure Systems

Owner: gameplay worker.

Deliverables:

- interaction target resolver for inspect/read/use prompts;
- route journal and travel goals;
- encounter-zone data model and passive simulation contract;
- browser/controller integration deferred until pure tests are clean.

Dependencies:

- pure event/goal work can start now;
- HUD/controller integration waits for renderer and verification gates.

Acceptance:

- idempotent events and goals;
- save/load snapshots stable;
- no Minecraft-style hotbar or block-placement assumptions;
- later browser probe can complete inspect/read/use and route-goal checks.

### Step 6 - Integrate Bold Content

Owner: self plus content workers after gates.

Deliverables:

- finite island macro terrain in generator;
- Red Mountain skyline, wetland/salt/glass/ash region identity, authored route graph, cave graph;
- object-lab acceptance for prop families;
- golden-view and route-atlas before/after metrics.

Dependencies:

- Step 1 verification gate;
- Step 2 renderer ownership gate;
- Step 4 atlas contract.

Acceptance:

- route/view artifacts show visible identity changes;
- FPS/hitch and LOD correctness remain within budget;
- world reads as large-region island, not patchwork voxel biome soup.

## Dependency Graph

```mermaid
flowchart TD
  V0["Step 1: Verification budgets/orchestrator"] --> C0["Content acceptance"]
  R0["Step 2: Render-only LOD ownership"] --> C0
  R0 --> S1["Canonical storage integration"]
  S0["Step 3: Pure edit journal"] --> S1
  A0["Step 4: Pure WorldAtlas"] --> WG["Generator island rewrite"]
  V0 --> WG
  R0 --> WG
  G0["Step 5: Pure gameplay systems"] --> UI["HUD/controller integration"]
  V0 --> UI
  R0 --> UI
  WG --> C1["Region content packs"]
  UI --> C2["RPG loop polish"]
```

## Task Distribution

### Self

Task: `R0` render-only LOD ownership.

Write scope:

- `src/engine/procedural-resident-world.ts`
- `src/engine/lod-clip-mask.ts`
- `src/engine/opaque-chunk-mesher.ts`
- `tests/lod-handoff.test.ts`
- `tests/lod-clip-mask.test.ts`
- `tests/opaque-chunk-mesher.test.ts`
- LOD diary entries

First implementation target:

- feed computed clip masks into LOD mesh building without mutating canonical derived chunk data.

### Verification Worker

Task: `V0` verification budgets and orchestrator.

Write scope:

- `scripts/lib/voxel-rpg-budgets.ts`
- `scripts/run-voxel-rpg-verification.ts`
- `tests/voxel-rpg-budgets.test.ts`
- `tests/voxel-rpg-verification-runner.test.ts`
- `package.json`

Do not edit renderer, generator, controller, or existing benchmark logic unless explicitly asked after review.

### Storage Worker

Task: `S0` pure edit journal foundation.

Write scope:

- `src/engine/chunk-edit-journal.ts`
- `tests/chunk-edit-journal.test.ts`
- optional notes in `docs/loop/20260509-canonical-lod-architecture.md`

Do not edit IndexedDB production cache yet.

### World Atlas Worker

Task: `A0` pure atlas expansion and validation.

Write scope:

- `src/engine/world-atlas.ts`
- `tests/world-atlas.test.ts`
- optional notes in `docs/loop/20260509-world-atlas-design.md`

Do not edit `src/engine/procedural-generator.ts` yet.

### Gameplay Worker

Task: `G0` pure RPG interaction and travel systems.

Write scope:

- `src/engine/exploration-interactions.ts`
- `src/engine/travel-goals.ts`
- `tests/exploration-interactions.test.ts`
- `tests/travel-goals.test.ts`
- optional notes in `docs/loop/20260509-gameplay-exploration-plan.md`

Do not edit `src/client/game-controller.ts` or HUD files yet.

## ROI Order

| Rank | Task | ROI | Reason |
| --- | --- | ---: | --- |
| 1 | `R0` render-only LOD ownership | 10 | Fixes the trust foundation: no meaningful content work matters while the world can render two realities. |
| 2 | `V0` verification orchestrator | 9 | Converts repeated manual checks into one evidence-producing command and prevents false visual/performance claims. |
| 3 | `S0` edit journal foundation | 8 | Enables canonical editable world persistence without risking production cache behavior immediately. |
| 4 | `A0` tested WorldAtlas | 8 | Enables bold island/worldgen changes from a stable data model instead of ad hoc generator patches. |
| 5 | `G0` pure RPG systems | 7 | Adds Morrowind-like loop structure while staying independent of renderer/generator churn. |
| 6 | generator island rewrite | 9 after gates | High player-visible payoff, but unsafe until renderer and atlas gates are trustworthy. |
| 7 | prop/atmosphere content packs | 7 after gates | Good visual payoff, but must be measured through object/view labs to avoid barely visible tweaks. |

## Evidence Checklist

Every accepted checkpoint must record:

- command list;
- artifact paths;
- LOD gap/overlap counts;
- p95/max frame and hitch buckets for browser-facing changes;
- screenshots/contact sheets for visual changes;
- diary update with rubric delta;
- commit and push.
