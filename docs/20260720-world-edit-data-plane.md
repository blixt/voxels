# World edit data plane

This records the compatibility-free July 2026 switch from per-voxel persistence and duplicated
in-memory indexes to chunk-owned sparse overlays, semantic edit commits, bounded retry receipts, and
region-parallel planning.

## Decision

The canonical world remains an immutable generated source plus sparse authoritative overrides.
Generated voxels are never copied into the edit database. Every override now has one owner at each
layer:

- memory: one `EditedChunk` with sorted `(u16 local_index, u16 material)` entries;
- disk: one versioned, hashed `VXED` blob per edited 32³ chunk;
- wire: one semantic tool action plus an adaptive changed-voxel mask;
- concurrency: one or more sorted 25.6 m horizontal edit-region locks plus one player lock.

SQLite remains the durable transaction engine. The schema is intentionally replaced rather than
migrated. The checked-in `{edit_schema}` path token gives schema 13 a fresh database while preserving
older local files.

## Measured outcome

`vp run automation -- run storage-benchmark` exercises the production planner, SQLite transaction,
restart hydration, exact retry path, chunk codec, and public/editor wire encoders. The numbers below
are from the same Apple development machine; they are comparison evidence, not portable performance
budgets.

| Measure | Previous | Current |
| --- | ---: | ---: |
| Clustered logical edit RAM | 61.6 B/mutation | 4.0 B/mutation |
| Frontier logical edit RAM | 63.0 B/mutation | 4.2 B/mutation |
| Clustered public commit wire | 4.38 B/mutation | 0.84 B/mutation |
| Frontier public commit wire | 4.23 B/mutation | 0.47 B/mutation |
| Clustered median commit | 3.23 ms | 2.55 ms |
| Frontier median commit | 5.24 ms | 4.77 ms |
| Clustered restart | 88.4 ms at 1,000 operations | 3.4 ms at 2,000 operations |
| Eight-worker disjoint edit probe | 182.2 ms sequential | 52.9 ms concurrent (3.45x) |

The 16-player cap-exercising run retained exactly 1,024 operation receipts after 2,000 operations.
Its clustered database was 7.46 MiB instead of the 12.0 MiB 100-player run whose 20 receipts per
player had not reached the cap. Long-running receipt storage is therefore bounded by active player
count rather than play time.

## Alternatives considered

### Per-voxel SQLite rows

This was the previous design. It provided simple point updates, but one metre-scale action wrote
hundreds of B-tree rows. Restart rebuilt three in-memory B-trees, and durable size grew with a large
key/row overhead. It is no longer retained.

### SQLite chunk blobs

Selected. Chunk blobs preserve atomic transactions across world edits, inventory, product revisions,
and idempotency receipts while collapsing hundreds of durable rows into a few owner updates.
`WITHOUT ROWID` remains appropriate for the small composite-key tables, as documented by
[SQLite](https://www.sqlite.org/withoutrowid.html). WAL keeps readers independent of the short
writer transaction; expensive generation and LOD planning no longer happen inside that transaction.

### A custom append log plus snapshots

An append log can make individual writes cheap, but it creates recovery, checksumming, compaction,
snapshot publication, and atomic player/world-state work that SQLite already supplies. It becomes
attractive only after region ownership makes each log independently recoverable and compactable.

### redb

[redb](https://github.com/cberner/redb) is a credible pure-Rust embedded database with ACID
transactions, but its single-writer model would not remove the current durable-writer boundary.
Replacing mature SQLite behavior without a measured gain would add risk.

### Fjall

[Fjall](https://docs.rs/fjall/latest/fjall/) is the most interesting pure-Rust LSM candidate for a
future region store: it offers keyspaces, atomic batches, snapshots, and background maintenance.
The trade is an LSM compaction and write-amplification regime that this workload has not yet proven
superior. It should be compared against one-SQLite-file-per-region only when the global writer is a
measured bottleneck.

### RocksDB

RocksDB has mature compaction controls and high write throughput, but brings a C++ dependency and
the operational tuning described in its
[compaction documentation](https://github.com/facebook/rocksdb/wiki/Compaction). That is a poor
local-first/Rust-first trade until a region-scale benchmark demonstrates a need.

## Current concurrency contract

An edit acquires its player key, then every horizontal region touched by its stencil in sorted order.
This has three useful properties:

1. Commands for one inventory/session remain ordered.
2. Overlapping or vertically related terrain columns serialize.
3. Distant regions sample the world source and calculate before/after surface invalidations in
   parallel.

Only the short canonical merge and SQLite commit retain the global authority mutex. Keyed lock slots
use weak references, so exploring new regions does not create another permanent in-memory world
index.

The horizontal region key is deliberate. Surface visibility couples every height in one `(x,z)`
column, so a three-dimensional lock could incorrectly classify a cave edit and a surface edit in the
same column as independent.

## Distribution path

Do not distribute the current global revision counter directly. The next switch, when benchmarks
show the single durable writer is limiting throughput, should be:

1. Introduce an explicit `RegionKey`/`RegionRevision` in the authority result and product cache.
2. Route each region to one owner worker. A local deployment may map many owners to threads; a
   distributed deployment maps the same keys to processes or machines.
3. Give each owner an independent SQLite WAL or evaluated Fjall keyspace containing `VXED` blobs and
   region-scoped product revisions.
4. Keep player inventory and operation receipts in a player authority. An edit is normally one
   region transaction; the rare stencil crossing a boundary acquires owners in sorted order and
   uses a small coordinator record.
5. Stream `(region, revision)` watermarks instead of forcing unrelated clients through one global
   revision. Resync requests name only regions retained by that client.
6. Replicate immutable generated-source identity and content-hashed `VXED` blobs. Derived chunks and
   LOD products remain disposable caches.

Before that change, extend the production benchmark with durable-writer wait time, per-region queue
depth, cross-boundary edit frequency, WAL bytes, checkpoint stalls, and p99 acknowledgement latency.
Switch storage engines only if the candidate improves those end-user and server-density measures
without weakening exact restart and retry behavior.
