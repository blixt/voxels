# Plan

## Active target

Turn `/` into the first slice of a real voxel game while preserving `/bench` as the repeatable correctness and performance harness.

The game direction is intentionally demanding:

- full-screen WebGPU canvas with click-to-capture cursor
- Minecraft-inspired interaction model
- voxel scale around `10 cm^3`
- procedurally generated world
- effectively infinite `X/Z`
- `Y` range `0..16383`
- lazy generation and streaming on demand
- architecture that can grow into persistence, remote authority, and multiplayer
- inventory-driven gather/build loop, starting with color-coded voxels

## Feedback-driven priorities

The first hands-on game pass exposed five immediate problems that now outrank micro-optimizing the current renderer path:

- the player has no collision
- the camera/body scale does not read as a `10 cm^3` voxel world
- terrain envelopes feel too extreme for an early exploration baseline
- the current resident radius is physically tiny at the intended voxel scale
- synchronous stream-in/remesh work is still hitching visibly during movement

The current numbers explain the feedback:

- default chunk size `32` with radius `3` only keeps about `96 cm` of horizontal residency
- the camera currently spawns only `8 cm` above the sampled surface
- increasing radius or chunk size alone will not solve long-distance visibility cleanly with the current synchronous streaming path

So the next slices need to separate:

- immediate scale/collision fixes that make the current world readable
- terrain-envelope tuning
- hitch reduction through streaming architecture
- true long-distance visibility, which will need something stronger than just changing constants

## Current principles

- Prefer broad research and broad error-case enumeration before locking onto one fix or design.
- Prefer tiny verification cases before large integrated scenes.
- Keep `/bench` as the engine oracle for deterministic validation and performance tracking.
- Run the repo as an alternating `feature -> performance/harness` loop instead of a pile of unrelated slices.
- Treat `mise run cycle-bench` as the default multi-front command-line acceptance gate.
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

Status:
- first seam landed: mesher and renderer now depend on a resident-chunk world interface instead of directly depending on `VoxelWorld` internals
- remaining work is the actual streaming/generation implementation behind that seam

### Slice 3: infinite world prototype

- Infinite `X/Z` chunk addressing.
- `Y` domain `0..16383`.
- Lazy procedural chunk generation near the player only.
- Initial biome/color distribution aimed at exploration rather than realism.
- Bench and validation cases for generation boundaries and stream-in churn.

Status:
- procedural generation primitives now exist in isolation:
  - fixed `#RGB` material palette (`4096` colors)
  - deterministic per-chunk generator
  - multi-biome column sampling and chunk generation
- `/` now boots from a procedural resident world instead of from the old finite demo scene
- resident chunk radius can be changed through the game debug API for repeatable stream churn probes
- remaining work is now verification and cost control:
  - add machine-readable generation/stream probes
  - reduce synchronous startup stream cost
  - keep the resident-world seam compatible with later edit overlays and persistence

### Slice 4: streaming verification and hot-path instrumentation

- Add machine-readable generation and residency probes instead of relying on HUD text or screenshots.
- Expose resident chunk sets, per-chunk hashes, and stream traces to automation.
- Instrument chunk generation and residency phases before attempting deeper optimizations.
- Use the new probe data to decide which startup/streaming optimizations actually survive verification.

Status:
- first machine-readable probe layer is now landing:
  - `/` exposes resident-world snapshots and teleport-and-settle traces
  - `/bench` exposes deterministic generation probes
- first hot-path win is now landing:
  - per-column generator caching removed the worst startup bottleneck
  - chunk solid bounds now come out of generation instead of being recomputed immediately
- phase instrumentation is now landing:
  - `profile-stream` reports residency phase times and dirty resident chunk counts
  - the data says neighbor-dirty bookkeeping itself is tiny, so naive batching is not the next win
- remesh accounting is now landing:
  - mesh rebuilds separate newly generated chunk meshes from remeshed boundary chunks
  - shrink still spends time on empty-chunk generation checks even when no solid chunks are adopted
- known-empty chunk caching is now landing:
  - resident-world updates remember empty chunk coordinates after the first probe
  - `profile-stream` and the game HUD now expose cached-empty hits separately from first-time empty skips
  - shrink and forced refresh probes now avoid repeated empty-chunk generation entirely
- occluded-solid chunk skipping is now landing:
  - the mesher now has a narrow fast path for fully solid chunks whose six adjacent face planes are fully occupied by resident neighbors
  - new mesher regression cases cover both the fully buried chunk and the "one neighbor-face hole" escape hatch
  - the win is specific to the streamed procedural world; the older benchmark scenes do not show the same buried-solid-chunk pattern
- remaining work is to choose between:
  - moving to incremental or worker streaming
  - reducing solid-chunk generation cost on widen/bootstrap
  - reducing real boundary remesh cost in cases other than the already-rejected face-aware invalidation path
  - hardening browser-side verification so acceptance does not depend on the shared DevTools MCP profile

### Slice 5: interaction loop

- Break/place interaction from center-screen picking.
- Inventory with `32` stacks and `1024` voxels per stack.
- Destroyed voxels flow into inventory.
- Color is the first material identity (`#ABC`, `4096` possible values).
- Unit tests for inventory rules and deterministic interaction cases.

### Slice 6: persistence boundary

- Define region/chunk serialization suitable for browser cache and future remote sync.
- Add browser-side persistent cache for generated/edited chunks.
- Define protocol shapes for chunk snapshots, edit deltas, and player state.
- Keep the single-player path compatible with eventual server authority.

### Slice 7: multiplayer MVP path

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

- Keep machine-readable generation and residency probes aligned with the game/runtime code.
- Use the new discovery journal as the first measurable exploration seam, then add browser route + trace automation so feature and performance slices share a live-game oracle.
- Use the new `trace-route` harness to drive the next streaming/LOD performance slices, and improve symbolized trace readability if the current production traces stay too minified.
- Treat the new chunk-derived render summary as the durable far-render seam:
  - keep surface clipmaps as the current above-ground renderer
  - add persisted region/summary loading next
  - then add general volumetric interior/void far visibility from chunk summaries instead of generator sampling
- Keep the grounded-player slice honest:
  - add spawn-footprint regression coverage so player starts remain stable
  - preserve feet position and grounded state in the game debug surface
- Re-tune the early terrain envelope so the first generated world reads as traversable terrain instead of a stress case.
- Add a dedicated stream-hitch benchmark path so movement-triggered chunk work is measurable without relying on ad hoc game probes.
- Use the current phase metrics to move residency/meshing off the synchronous movement path instead of chasing only narrow mesher micro-optimizations.
- Use the new game-path boundary-cross probe to separate:
  - residency and meshing cost
  - first-frame sync/upload cost
  - how often one-chunk movement is needlessly triggering the full pipeline
- Apply stream-anchor hysteresis next if the game-path probe confirms that one-chunk crossings are still causing full updates.
- With hysteresis in place, use the same probe to choose the next hitch slice:
  - either deeper async streaming
  - or boundary remesh reduction for the larger jumps that still churn
- Keep pushing the world toward a readable `10 cm` baseline:
  - tame terrain envelopes before adding more visual complexity
  - then attack true draw distance with a far-field representation rather than just increasing near-chunk radius
- Keep the procedural stream profiler and browser probes aligned so local and Chrome decisions stay comparable.
- When a micro-optimization is noisy, compare the working tree against a clean committed worktree on a separate port before keeping it.
- If the next change targets meshing, build or adapt cases that focus on boundary remesh cost rather than only initial chunk creation.
- Add deterministic verification cases for resident-set stability, seam consistency, and stream churn.
- Keep the game debug API growing alongside `/bench` so browser automation does not depend on visual inspection alone.
- Add a browser-capable verification path that still works when the shared DevTools MCP profile is stale or unavailable.
- Treat long-distance visibility as an architectural track:
  - not just a radius constant
  - likely involving asynchronous streaming, coarser far representations, or both
- Keep the recursive task list fresh:
  - finish one feature slice
  - finish one performance/harness slice
  - then rewrite the next cycle instead of letting the backlog drift
- Log each slice in `progress.md` and `verification.md`.
