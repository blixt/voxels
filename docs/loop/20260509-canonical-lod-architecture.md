# Canonical LOD Architecture

## Contract

- Canonical terrain is generated base chunk plus persisted edit journal plus live overlay.
- Generated chunks and edit journals are durable authority.
- Summaries, derived LOD chunks, retained meshes, and disk LOD chunks are disposable render data.
- A derived LOD cache miss may affect speed, never correctness.
- A stale or prepared chunk must not render unless ownership marks it active.
- Near player collision and interaction use resident LOD0 data.
- Far visibility may use derived LOD, but handoff must avoid visible gaps, overlaps, and stale edits.

## Current Useful Seams

- `src/engine/generated-chunk-codec.ts`: canonical generated chunk payloads.
- `src/engine/derived-lod-chunk-codec.ts`: disposable LOD payloads.
- `src/engine/chunk-edit-journal.ts`: packed edit deltas and replay.
- `src/engine/lod-chunk-derivation.ts`: downsampling surface for LOD data.
- `src/engine/procedural-resident-world.ts`: active, prepared, retained, and visible ownership state.
- `src/client/procedural-generated-chunk-cache.ts`: browser persistence for generated data and derived summaries.

## Engineering Rules

- Keep durable and derived data visibly separate in names and APIs.
- Keep worker output deterministic from explicit inputs.
- Make ownership state observable in game snapshots.
- Prefer small helpers around chunk identity, edit revision, and ownership rather than stringly cache logic.
- When rebuilding verification later, start from these invariants instead of preserving deleted harness behavior.
