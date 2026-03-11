# Game Roadmap

This roadmap replaces the earlier "voxel playground" framing with the actual target for the repo: a browser-first voxel game and engine that can grow into a shared persistent world.

The goal is not to jump straight to the final architecture. The goal is to move toward it through small slices that are easy to verify and easy to discard when they prove wrong.

## Product target

Build a Chrome-first WebGPU voxel game where:

- `/` is a full-screen playable experience
- the world is procedurally generated and lazily streamed
- the world is effectively infinite in `X/Z`
- the vertical axis is `0..16383`
- voxels are tiny enough that engine throughput matters constantly
- players can gather and place colored voxels
- the architecture can become persistent and multiplayer without a rewrite

## Guiding architecture

- Keep chunked sparse voxel storage as the practical editable baseline.
- Separate generated terrain, authored/imported content, and player edits.
- Separate authoritative world state from resident/renderable chunks.
- Treat persistence and networking as part of the world model, not as bolt-ons.
- Keep correctness and benchmark coverage growing alongside features.
- Favor small validated rewrites over additive complexity.

## Staged path

### Stage 1: full-screen game shell

- Replace the current `/` playground UI with a full-screen canvas, HUD, crosshair, and click-to-capture pointer lock.
- Add a dedicated game controller and first-person camera path.
- Reuse the existing renderer and current terrain scene at first so the slice stays small and verifiable.
- Add a game debug API so browser automation can query state without scraping the UI.

### Stage 2: runtime world model

- Replace the fixed demo-scene mindset with world-space chunk coordinates.
- Introduce lazy chunk residency and a generation interface keyed by chunk coordinate.
- Keep generated content and edit overlays independent.
- Start exposing stream/residency metrics in `/bench`.

### Stage 3: infinite procedural world

- Support effectively infinite `X/Z` traversal.
- Fix the world `Y` domain at `0..16383`.
- Generate terrain lazily on demand near the player.
- Add biome and height-driven variation designed around exploration and color discovery, not only topography.
- Keep generation deterministic from world seed plus chunk coordinate.

### Stage 4: basic interaction loop

- Add center-screen voxel targeting for break/place.
- Add inventory management:
  - `32` stacks total
  - `1024` voxels max per stack
  - voxel identity begins as `#ABC` color
- Route destroyed voxels into the inventory.
- Add deterministic tests for inventory and interaction rules.

### Stage 5: persistence-ready storage

- Define chunk and region payloads that can be cached locally and later served remotely.
- Add browser persistence for chunk data and edit overlays.
- Keep storage chunk-addressable and binary.
- Record chunk versions and edit lineage so multiplayer/server authority can arrive without changing the local model completely.

### Stage 6: multiplayer MVP

- Introduce a remote authoritative server for the shared world.
- Support multiple concurrent players interacting in the same persistent world.
- Start with world snapshots plus edit deltas rather than complex simulation replication.
- Add multi-client verification scenarios before deeper gameplay features.

### Stage 7: richer world simulation

- Add stable collision and movement against voxel occupancy.
- Add simple tools, placement rules, and interaction affordances.
- Expand generation with caves, vertical biome bands, landmarks, and color-specific regions worth exploring.
- Add imported prefabs or structures where that improves variety.

### Stage 8: visual upgrades

- Improve sunlight/skylight, emissive lighting, and fog first.
- Add better shadows and more stable large-distance lighting.
- Investigate reflections or ray-based effects once the raster baseline is strong.
- Keep each visual upgrade measurable in `/bench`, not only subjectively visible.

## Core technical tracks

### Streaming and scale

- Residency manager
- chunk priority and eviction
- region files / chunk manifests
- local cache
- stream-in correctness tests
- sustained flythrough benchmarks

### LOD

- full editable voxels near the player
- coarser chunk representations farther away
- cheap horizon/impostor representations at extreme distance
- seam and silhouette verification scenes

### Generation

- deterministic chunk generation
- biome selection
- vertical world layering
- color/resource distribution tuned for exploration
- generation benchmarks separate from render-only benchmarks

### Persistence and networking

- chunk snapshots
- edit overlays
- versioned deltas
- player state replication
- eventually remote authority

### Physics and interaction

- first-person controller
- voxel collision
- break/place targeting
- tool rules
- later simplified debris and world reactions

### Rendering

- preserve strong primitive correctness coverage
- keep first-frame vs warm-frame budgets visible
- add stream-aware and distance-aware metrics
- add validation scenes for picking, streaming seams, and LOD transitions

## Acceptance mindset

Every serious step should follow the same loop:

1. Broad research and failure-case enumeration.
2. Hypothesis list with cheap kill conditions.
3. Smallest useful implementation slice.
4. Unit tests and tiny deterministic scenes.
5. `/bench` or another purpose-built harness endpoint.
6. Clean Chrome verification.
7. Docs update and commit.

## Near-term focus

The near-term repo priority is:

1. turn `/` into a usable full-screen game shell
2. establish a dedicated game runtime path
3. give the player a real physical scale:
   grounded movement, collision, gravity, and a sensible eye height for `10 cm` voxels
4. tame the first terrain envelope so the world reads as traversable rather than as a stress test
5. introduce a world model that can grow beyond bounded scenes
6. make world generation, streaming, and player interaction measurable enough that future work is not guesswork
