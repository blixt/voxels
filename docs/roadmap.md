# Future Directions

This document captures medium- and long-term directions that fit this WebGPU voxel engine well. It is intentionally broader than the active loop in `docs/loop/plan.md`.

## Guiding principles

- Keep Chrome stable as the primary runtime target.
- Prefer measured rewrites of hot paths over additive complexity.
- Preserve live editing as a first-class constraint.
- Treat correctness and benchmark coverage as part of feature work, not cleanup.
- Prefer chunked, streamable, incrementally updateable data structures.

## World scale

### Larger worlds

- Replace the fixed world volume with chunk coordinates in world space.
- Support sparse chunk residency so the engine can represent very large or effectively unbounded worlds.
- Add floating-origin or origin rebasing to avoid precision loss as the camera moves far from world origin.
- Group chunks into larger regions so storage, streaming, and cache management stay practical.

### Streaming

- Introduce a residency manager that loads, keeps, and evicts chunks based on distance, visibility, and camera motion.
- Load nearby chunks first and prefetch along the camera velocity vector.
- Move chunk generation, decoding, and meshing into workers so stream-in does not block interaction.
- Keep authored edits as an overlay on top of streamed or procedural base content.

### Storage and protocols

- Use chunk- or region-addressable binary formats so random access stays cheap.
- Keep payloads palette-based and compression-friendly.
- For static content, consider region files plus HTTP range requests or chunk bundles.
- For live worlds, use a snapshot plus binary delta stream for edits, light updates, and invalidations.
- Cache downloaded/generated chunks in browser storage such as OPFS or IndexedDB.

## Rendering scale

### Level of detail

- Keep full editable voxels near the camera.
- Use coarser chunk representations at medium range.
- Use very cheap far-distance representations such as merged shells, impostors, or heightfield-like approximations where appropriate.
- Add hysteresis and seam handling so LOD transitions do not shimmer or crack.
- Extend the benchmark harness with explicit LOD seam and far-distance silhouette checks.

### Streaming-aware rendering

- Separate first-frame upload cost from warm-frame draw cost, and keep using those metrics as acceptance gates.
- Add occlusion- and distance-aware chunk submission to cut draw and upload work.
- Consider render bundles or other repeated-pass encoding optimizations if draw submission becomes material again.

### Ray tracing and ray-based effects

- Use rays first for picking, visibility queries, shadows, and reflections.
- Treat full ray-traced or path-traced rendering as an optional experimental mode rather than the default path.
- Explore voxel-friendly acceleration structures such as occupancy mipmaps, brick maps, or sparse hierarchies.

## World creation

### Procedural generation

- Make terrain and structure generation deterministic per chunk from a seed.
- Separate generation into passes such as terrain, biome, caves, structures, decoration, and gameplay overlays.
- Persist only seeds, generation parameters, and edit layers where possible.
- Add benchmark scenes that simulate chunk generation under stream-in pressure.

### Authoring and import

- Expand beyond MagicaVoxel import where practical.
- Support a world model of `procedural base + imported assets + user edits`.
- Add prefab/stamp workflows so authored structures can be placed efficiently into large streamed worlds.

## Lighting

### Near-term lighting

- Add sunlight/skylight propagation that survives live edits.
- Add emissive voxel light propagation.
- Keep ambient occlusion cheap and incremental.
- Add fog and day/night controls that work with large view distances.

### Richer lighting

- Investigate shadow techniques suitable for large voxel worlds.
- Add colored local lights and probe-like indirect approximations for interiors.
- Keep any lighting model chunk-local enough to update incrementally after edits or destruction.

## Simulation

### Physics

- Add a simple voxel-friendly character controller first: capsule motion, swept collision, step-up logic, and broadphase against chunk occupancy.
- Add simplified rigid interactions only after the basic world pipeline is stable.
- For destruction-heavy scenes, model detached fragments coarsely and aggressively cull or sleep them.

### Gameplay interaction

- Add mining, building, painting, and replace tools.
- Add higher-level editing tools: line, box, sphere, flatten, copy/paste, prefab placement, and undo/redo.
- Add simple world interactions such as switches, doors, triggers, damage, and basic fluids/fire if needed.

## Tooling and verification

### Benchmark growth

- Add cold-start streaming benchmarks.
- Add sustained flythrough benchmarks with chunk churn.
- Add edit-burst benchmarks near chunk and region boundaries.
- Add lighting-update benchmarks after local edits.
- Add explicit first-frame vs warm-frame budgets for larger scenes and streaming scenarios.

### Correctness growth

- Add validation scenes for LOD seams, streaming pop-in, and far-distance silhouette drift.
- Add tests for chunk residency and stream ordering.
- Add deterministic capture routes for validation-only runs.
- Keep primitive-level render checks in place before scaling scene complexity further.

## Candidate phase order

1. Larger-world data model and chunk streaming.
2. LOD and far-distance rendering.
3. Incremental lighting upgrades.
4. World editing and authoring tools.
5. Simplified physics and interaction systems.
6. Experimental ray-based rendering features.
