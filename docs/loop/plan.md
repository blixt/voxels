# Plan

## Active target

Turn `/` into the first slice of a real voxel game while preserving `/bench` as the repeatable correctness and performance harness.

The game direction is intentionally demanding:

- full-screen WebGPU canvas with click-to-capture cursor
- Minecraft-inspired interaction model
- voxel scale around `1 cm^3`
- procedurally generated world
- effectively infinite `X/Z`
- `Y` range `0..16383`
- lazy generation and streaming on demand
- architecture that can grow into persistence, remote authority, and multiplayer
- inventory-driven gather/build loop, starting with color-coded voxels

## Current principles

- Prefer broad research and broad error-case enumeration before locking onto one fix or design.
- Prefer tiny verification cases before large integrated scenes.
- Keep `/bench` as the engine oracle for deterministic validation and performance tracking.
- Keep work incremental enough that every slice can be researched, implemented, verified, documented, and committed.
- Prefer rewriting/removing over layering on more abstractions.
- Keep the repository usable for parallel agent work and worktree-based A/B comparisons.

## Phase status

### Completed foundation

1. Bun + TypeScript + Mise project setup.
2. Chunked sparse world storage and scene serialization.
3. WebGPU renderer with greedy-meshed chunk uploads.
4. Orthographic isometric camera path and default terrain scene.
5. Import/export support and live voxel edits in the old playground flow.
6. `/bench` correctness and performance harness with primitive validation scenes and stress scenes.
7. Repo-local docs for research, progress, verification, and agent workflow.

### Active pivot

1. Replace the `/` playground shell with a dedicated game shell.
2. Split game input/runtime code away from the orbit/benchmark controller stack.
3. Establish a first-person camera/input path that does not destabilize the current benchmark renderer.
4. Reframe the roadmap around an iterative path to a shared persistent world instead of around standalone demo scenes.

## Next slices

### Slice 1: game shell and movement

- Full-screen `/` experience with HUD, crosshair, and click-to-capture pointer lock.
- Dedicated game controller instead of reusing the orbit playground controller.
- First-person camera, mouse-look, movement, and a debug API that automation can call.
- Reuse the current world/renderer stack where possible so the slice stays small.

Status:
- delivered over the existing `terrain256` bootstrap world
- pointer-lock failure is surfaced cleanly in automation instead of crashing the page
- next work moves to the world model split rather than polishing the old playground path further

### Slice 2: world model split

- Separate "authoritative world state" from "currently resident/renderable chunks".
- Introduce chunk coordinates in world space instead of a single bounded scene volume.
- Add a generation interface that can lazily provide chunk contents by coordinate.
- Keep edit overlays separate from generated base terrain.

### Slice 3: infinite world prototype

- Infinite `X/Z` chunk addressing.
- `Y` domain `0..16383`.
- Lazy procedural chunk generation near the player only.
- Initial biome/color distribution aimed at exploration rather than realism.
- Bench and validation cases for generation boundaries and stream-in churn.

### Slice 4: interaction loop

- Break/place interaction from center-screen picking.
- Inventory with `32` stacks and `1024` voxels per stack.
- Destroyed voxels flow into inventory.
- Color is the first material identity (`#ABC`, `4096` possible values).
- Unit tests for inventory rules and deterministic interaction cases.

### Slice 5: persistence boundary

- Define region/chunk serialization suitable for browser cache and future remote sync.
- Add browser-side persistent cache for generated/edited chunks.
- Define protocol shapes for chunk snapshots, edit deltas, and player state.
- Keep the single-player path compatible with eventual server authority.

### Slice 6: multiplayer MVP path

- Move to a remote authoritative world service.
- Multiple clients can connect and edit the same persistent world.
- Keep protocol and storage binary and chunk-addressable from the start.
- Add repeatable multi-client verification cases before deeper gameplay systems.

## Immediate research questions

- Which smallest first-person slice proves the `/` pivot without destabilizing `/bench`?
- Which browser storage path should be treated as the local persistence baseline: OPFS alone, IndexedDB metadata plus OPFS payloads, or something simpler first?
- When multiplayer arrives, should the first transport be WebSocket or WebTransport for this repo's needs?
- Which debug/verification endpoints should be added now so future world-streaming work is measurable instead of visual-only?
- What extra deterministic scenes should `/bench` gain so streaming, picking, and interaction changes can be verified quickly?

## Immediate implementation tasks

- Split the world model into world-space chunk coordinates and resident/renderable chunks.
- Add the first generation/residency interfaces before introducing infinite-world traversal.
- Add deterministic verification cases for first-person pick rays and movement-related invariants.
- Keep the game debug API growing alongside `/bench` so browser automation does not depend on visual inspection alone.
- Log each slice in `progress.md` and `verification.md`.
