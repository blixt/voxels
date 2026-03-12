# Persistence Notes

## 2026-03-12 column-summary layer

- The generated cache now persists three levels of data:
  - chunk payloads
  - per-chunk render summaries
  - per-column render summaries
- The column summary is still derived metadata, not authoritative world state.
- The main reason to persist it separately is to let far rendering and visibility bootstrap from disk-backed chunk-derived metadata without reopening generator coupling or forcing full chunk decode just to learn a column’s coarse extent and top surface.
- Current limitation:
  - the generated cache can merge column summaries incrementally because generated chunks are deterministic and effectively append-only in this cache
  - a future authoritative edited-world store should treat column/region summaries as versioned derived data and rebuild or replace them when chunk contents change, not just monotonically merge them

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
It also keeps the generator in the right place: generation is a producer of pristine chunks, not a render-time source of far terrain.

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
- a place to persist chunk-derived render summaries alongside chunk payloads

## Why this shape

This follows the repo research rather than fighting it:

- keep a mutable chunk-grid core for gameplay/editing
- wrap it in a sparse outer storage strategy
- separate authoritative voxel data from derived runtime structures
- avoid a separate far-field world generator

The far-field path no longer samples the generator at render time. It now reads chunk-derived render summaries instead. Those summaries are derived from actual chunk bytes and are the right place to grow both surface and volumetric far visibility over time.

## Current limitations

- Persistence currently exists only on the async browser worker path.
- Resident chunks are still dense in RAM once adopted.
- We are using IndexedDB directly today; OPFS payload files plus IndexedDB metadata are still the stronger next storage split.
- Stored chunks are generated-base chunks only; edit overlays are not persisted yet.
- Chunk render summaries are now persisted alongside chunk payloads in IndexedDB and can be loaded independently on the worker path.
- Render summaries are still archived in memory after generation/eviction on the main thread; there is not yet a dedicated region-summary stream/index on top of the persisted summary records.
- The current far renderer is still surface-oriented; the new render summary seam now supports a future volumetric interior/void renderer for arbitrary edited spaces, but that second renderer is not implemented yet.
- The new cache-reuse benchmark seam exists, but the first headless proof attempt exposed that its runtime-ready gate still needs tightening before it becomes a trustworthy automated acceptance check.

## Next practical steps

1. Add region/column summary indexes on top of the persisted chunk-summary records so far-field and visibility work stop scanning unknown chunk columns blindly.
2. Split browser persistence into:
   - OPFS payload files
   - IndexedDB manifest / LRU / version metadata
3. Persist edit overlays separately from generated base chunks.
4. Add a clean browser acceptance harness for chunk-cache reuse and revisit latency.
5. Add a volumetric far-visibility path that consumes chunk render summaries for arbitrary large interior/void views.
6. Decide whether resident chunks should adopt the same sparse subchunk representation or stay dense-hot / sparse-cold.
