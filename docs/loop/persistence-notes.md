# Persistence Notes

## Current seam

The procedural world now has a first real persistence-oriented boundary:

- authoritative worldgen still lives in `ProceduralWorldGenerator`
- nearby editable/runtime chunks are still dense resident chunks in `ProceduralResidentWorld`
- generated chunk payloads are now serialized through a compact codec before crossing the worker boundary or hitting browser storage
- the browser generation worker now treats IndexedDB as a persistent generated-chunk cache keyed by:
  - generation version
  - seed
  - sea level
  - chunk size
  - max Y
  - chunk coordinate

This keeps one world-generation truth while still letting non-resident chunks become small, durable payloads.

## Chunk payload direction

The new generated-chunk codec is intentionally simple and mutation-friendly:

- chunk size remains `32^3` today
- each chunk is encoded as `8^3` subchunks
- each subchunk is one of:
  - `empty`
  - `uniform(material)`
  - `palette-packed`
  - `dense16`

That gives us:

- almost-free all-air chunks
- cheap storage for very uniform chunks
- smaller transfer/persistence cost for sparse or low-material-variety chunks
- a path toward later resident-memory compaction without changing the durable format again

## Why this shape

This follows the repo research rather than fighting it:

- keep a mutable chunk-grid core for gameplay/editing
- wrap it in a sparse outer storage strategy
- separate authoritative voxel data from derived runtime structures
- avoid a separate far-field world generator

The far-field path now consumes `sampleSurfaceColumn(...)`, which is a lighter query mode of the same generator, not a second terrain system.

## Current limitations

- Persistence currently exists only on the async browser worker path.
- Resident chunks are still dense in RAM once adopted.
- We are using IndexedDB directly today; OPFS payload files plus IndexedDB metadata are still the stronger next storage split.
- Stored chunks are generated-base chunks only; edit overlays are not persisted yet.
- Far-field still samples procedural surface queries directly; it does not yet consume a persisted column-tile or region-summary cache.
- The new cache-reuse benchmark seam exists, but the first headless proof attempt exposed that its runtime-ready gate still needs tightening before it becomes a trustworthy automated acceptance check.

## Next practical steps

1. Add persistent column/region summaries for surface/water/top bounds so far-field and Y-range work stop paying repeated procedural sampling.
2. Split browser persistence into:
   - OPFS payload files
   - IndexedDB manifest / LRU / version metadata
3. Persist edit overlays separately from generated base chunks.
4. Add a clean browser acceptance harness for chunk-cache reuse and revisit latency.
5. Decide whether resident chunks should adopt the same sparse subchunk representation or stay dense-hot / sparse-cold.
