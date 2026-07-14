# Local world-service Phase 1 follow-up

This is the handoff after the first change set in
`20260713-local-world-service-plan.md`. No daemon, network transport, SQLite authority migration, or
Terrain Diffusion runtime has been added yet.

## Landed boundary

- `world::source` owns canonical `WorldSourceIdentity`, `WorldManifest`, macro-field schema types,
  keyed batch product requests, source traits, validation, and the procedural-v16 adapter.
- Chunk products contain an owned 32³ core and the exact 6,536-cell meshing shell. The game worker
  batches generation tickets, retains the shell with the chunk, and no longer calls the generator
  while meshing.
- `EditMap::resolve_generated`, `set_against_generated`, `surface_sample_with`,
  `source_values_for_overrides`, and `skyline_feature_is_pristine_with` accept or retain already
  obtained source values. The shell captures pristine values before applying a chunk's sparse
  overlays, so removing an edit restores the exact source material without a point request.
- The shell owns only `Box<dyn WorldSourceEngine>`. Collision, fluid, spawn, visibility, and edit
  comparison consume resident chunks or prepared bounded blocks. Simulation, enclosure, raycast,
  and light-visibility callbacks never call the source; missing resident data is conservative only
  for those non-authoritative paths. Spawn, persisted-camera validation, teleports, portal probes,
  and diagnostic fixtures abort on a missing or mismatched authoritative product instead of
  inventing a material. No production shell path names `Generator` or `ProceduralWorldSource`.
- Surface probes use bounded 64x64 sample blocks and a 64-block bounded cache. Coast and underwater
  teleports use a bounded keyed `SurfaceSearch` product that preserves the existing deterministic
  ring order inside the provider and returns only the first match, rather than issuing one source
  call per column or materializing a large temporary search grid.
- Batch results retain their request keys and independent item errors. The shell validates the
  batch identity, item identity, request key, chunk coordinate, and halo coordinate before
  accepting a product; a malformed halo is retried rather than meshed as air.
- VXCH v2 requires an expected 32-byte source identity. Its content hash binds identity, chunk
  coordinate, material schema, and canonical voxel IDs while preserving v1 palette packing.
- Procedural-v16 core parity is frozen for ordinary, route, water, landmark, and Cinder Vault chunks.
- Criterion baselines now cover source chunk/halo products and runtime frame admission.

## Remaining concrete generator dependencies

These are intentionally visible rather than hidden behind a point-sampling transport API:

1. `ProceduralWorldSource` still delegates to `generation::Generator`; `Generator`,
   `GeneratedColumn`, and `GeneratedRegion` remain public because world tests, LOD helpers, and
   benchmarks still exercise the original implementation directly.
2. Atmosphere and debug landmark lookup are deliberately high-level bounded source methods rather
   than bulk protocol messages. `world-client` must either keep them as development capabilities or
   derive/cache equivalent session products; a production `SampleVoxel` message remains prohibited.
3. `WorldSourceEngine::generate_batch` is intentionally synchronous for the in-process Phase 1
   adapter. Phase 2 must put cache misses and explicit teleport/profile batches behind asynchronous
   `world-client` admission; no simulation or render callback depends on completing such a miss.
4. Edit-aware terrain/water surface generation and invalidation are source-neutral trait methods but
   still receive the browser-owned `EditMap`. They move behind the authoritative session in Phase 3.
5. `EditMap` retains generator-specific convenience methods for procedural world-unit tests. New
   service/client code must use the prepared-value APIs instead.
6. Browser OPFS still identifies the old phase-1 world by seed plus `GENERATOR_VERSION`. That design
   is superseded: current builds use a manifest- and persistence-schema-derived namespace and never
   import or upgrade the phase-1 database.
7. Camera persistence and world edits remain combined in the browser `Store`; splitting them belongs
   to a hard authority cut after protocol and daemon counterproof exist.
8. `MacroTerrainSource` now defines and tests versioned, bounded field blocks, but procedural-v16
   still reaches its existing internal composition pipeline directly. Before the Terrain Diffusion
   spike, extract one shared canonical composer that consumes these macro fields so providers cannot
   fork roads, water, caves, materials, or landmarks.

## Next extraction

The next change should add `world-protocol`, `world-client`, `world-service`, and `worldd` around the
owned products already used by the game worker. Start read-only: negotiate a manifest, request chunk
batches with cancellation and bounded queues, and compare sidecar snapshots against the in-process
procedural path. Do not import OPFS edits; use a fresh versioned namespace after snapshot parity and
reconnect behavior are proven.
