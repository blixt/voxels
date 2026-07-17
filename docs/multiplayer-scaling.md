# Multiplayer scaling envelope

VXWP v17 supports one unsharded authoritative world with up to 1,024 admitted player sessions. Spatial
interest management is an internal replication index, not a gameplay shard: every player inside the
same location's 256 m interest radius can discover every other player there. Players beyond that
radius contribute no entity records or bytes to one another's presence streams.

## Why deltas replaced complete rosters

The v4 server encoded every player every 33 ms and sent the complete roster to every connection.
The exact frame size was `48 + 80 * players` bytes, making aggregate egress quadratic. At 500 players
that design would send about 607 MB/s (4.85 Gbit/s) before WebSocket/TCP overhead.

VXWP has no legacy roster decoder or endpoint. One receiver-specific `PresenceDelta`
contains:

- enter records: player ID, connection handle, color, and initial absolute pose (80 bytes);
- replaceable pose updates keyed only by the connection handle (60 bytes);
- explicit connection leaves (8 bytes); and
- a 48-byte frame and delta header.

Missing state means unchanged. Identity is therefore paid once per area-of-interest entry, and a
server can update any subset without removing the rest. Every pose is absolute and independently
replaceable, which also prepares movement for a future unreliable WebTransport datagram while
lifecycle remains on a reliable stream.

## Spatial replication and dense regions

The server stores player membership in configurable 64 m X/Z cells. A receiver scans only cells
intersecting its interest radius, filters by exact horizontal distance, and retains known players for
an extra 32 m hysteresis margin. Cell coordinates use floor division, so negative world space has
the same boundary semantics as positive space.

Each connection owns a persistent known set and last-sent state. The scheduler sends when any of
these becomes true:

- the entity has entered interest;
- an unsent teleport/discontinuity exists;
- dead-reckoned position or look direction exceeds the configured visual-error threshold; or
- the distance tier's maximum interval has elapsed.

Entries, discontinuities, overdue age, prediction error, and distance determine priority in that
order. Age precedes ordinary prediction error once overdue, preventing a continuously erratic actor
from starving an older ordinary update. At most 64 entries plus updates are placed in one receiver's
delta. A 500-player crowd therefore degrades freshness under a fixed budget; it never silently drops
membership. The browser retains omitted tracks, uses Hermite interpolation, extrapolates velocity for
at most 750 ms, and then holds.

This is the same broad production pattern as Epic's persistent per-connection
[Replication Graph](https://dev.epicgames.com/documentation/en-us/unreal-engine/replication-graph-in-unreal-engine),
which Epic describes as designed for Fortnite's 100 players and roughly 50,000 replicated actors.
Epic's detailed replication flow filters by frequency and relevance, then sorts by per-connection
priority under bandwidth pressure:
[actor replication flow](https://dev.epicgames.com/documentation/en-us/unreal-engine/detailed-actor-replication-flow-in-unreal-engine).
Valve likewise documents snapshot rate limits, interpolation, and bounded extrapolation in
[Source Multiplayer Networking](https://developer.valvesoftware.com/wiki/Source_Multiplayer_Networking).

## Deterministic scale checks

The canonical command map, including native bot populations and browser gates, is
[Testing and performance](testing.md). `vp run bench:bots` adds real VXWP movement, terrain
streaming, digging, building, following, process sampling, exact per-client wire budgets, SQLite
growth, and an optional Chromium observer at up to 1,000 clients. See the
[2026-07-17 bot load baseline](20260717-bot-load-report.md) and the
[1,000-player traffic-shaping report](20260717-1000-player-traffic-report.md) for measured results.

Run the focused optimized probe with:

```sh
PATH="$HOME/.cargo/bin:$PATH" cargo test --release -p voxels-world-service presence::tests:: -- --nocapture
```

On 2026-07-17 on an Apple M3 Max (16 CPU cores), the checked-in tests measured:

| Scenario                                                               |                                              Result |
| ---------------------------------------------------------------------- | --------------------------------------------------: |
| 1,000 players at one location; one observer discovers 999 peers        |                       16 deltas, 80,688 bytes total |
| Scheduler query, ranking, validation, and encoding for those 16 deltas |                                      2.543 ms total |
| One observer plus 999 players outside its interest cells               | 0 entity bytes after the initial empty stream frame |

Timing is a local diagnostic, not a CI threshold. The durable regressions assert the record budget,
complete dense membership, exact wire bytes, negative-cell behavior, explicit disconnect leaves, and
zero isolated entity bytes. The 512-player dense discovery bound is eight network ticks, about
264 ms at the configured 33 ms scheduler cadence. A full dense movement frame is bounded to 3,888
bytes per receiver (`48 + 64 * 60`), or about 118 KiB/s if saturated on every tick; ordinary near,
mid, and far cadence plus prediction suppresses unchanged or predictable records.

### Six independent browser users

`vp run test:multiplayer-browser` owns its native service, preview server, temporary configuration,
and six independent ephemeral BrowserContexts. Each context has separate local storage and OPFS data,
receives an independently shaped 40 ms RTT, 50/10 Mbit/s link, and must negotiate a distinct browser
user and player identity. Five builders and one observer start together. The observer then travels at
least 120 m from the builders, beyond the configured 96 m mid-presence tier, and all six clients must
still render the other five articulated avatars. Per-player `/v17/world` and `/v17/presence` stream and
VXWP byte counts are written to `target/multiplayer-browser/latest.json`; the far observer screenshot
is `target/multiplayer-browser/observer-far-five.png`.

The harness also samples a steady-state frame-history window for every context. Because all six
unthrottled WebGPU renderers contend on one local GPU and worker pool, this is a bounded-stall gate,
not a claim about each remote player's independent device. The current limits are versioned into the
result JSON from the runner; exact p95, maximum, over-33.33 ms count, and dropped-history count remain
there so local contention regressions are visible rather than hidden behind one final-frame sample.

This deliberately does not reuse the persistence browser test's same-profile BroadcastChannel. The
strict scenario uses five distinct world sockets to submit 40 reachable voxels through the production
server edit path while the sixth browser observes from about 120 m away. Every client must apply all 40
commits, and the observer's resident stride-16 tile must advance to its required server revision and
finish clean rather than remain dirty. A before/after pixel gate also requires a legible tall change
around the aimed tower; revision bookkeeping alone cannot pass. Use the explicit strict form in
release checks:

```sh
vp run test:multiplayer-browser -- --require-tower
```

The run records convergence time, edit-only world traffic, the observer's required/accepted surface
revision, pre/post GPU mesh fingerprints, pixel evidence, per-client frame health, and screenshots in
`target/multiplayer-browser/`. A separate native WebSocket regression decodes the resulting coarse
mesh and proves the tower's top survives stride-16; the browser gate proves the real worker uploads
that revised product and changes the rendered silhouette.

As a pre-reach-enforcement baseline on 2026-07-14 on an M3 Max with Chrome 150, the old 10 m/
100-voxel tower converged in the far observer in 170.6 ms. After world coverage had settled, the
observer received 99,979 world-stream bytes for
commits plus its replacement product; builders received 99,979-135,244 bytes and sent 1,166-1,248
bytes while the observer sent 246 bytes.
All six applied 100 edits, the observer's required and accepted surface revision matched, its tile
finished resident and clean, and both its active GPU mesh and presented viewport fingerprints changed.
The aimed screenshot contained 125 materially changed pixels in a 34-pixel-tall silhouette, including
56 new cyan pixels. Steady six-client frame p95 was 42.1-50.1 ms with zero dropped history samples.

## Capacity and backpressure

The checked-in service admits 512 world sockets plus 512 independently bounded presence sockets.
The world request queue holds 8,192 jobs, exactly one negotiated 16-request window per admitted world
connection. Each session can occupy at most two of the eight blocking generation workers. Presence
has no per-socket outbound queue: a socket builds the newest delta at its next tick and awaits that
single send, so a slow client cannot accumulate stale movement frames.

Individual chunks and surface tiles use process-wide single-flight generation. Overlapping batches
join the first computation per product, and later requesters reuse validated encoded items from a
256 MiB byte-bounded LRU while receiving their own request ID and requested item order. This prevents
a crowd spawning in one place from performing the same CPU or Metal work hundreds of times even when
clients choose different batch boundaries. The cache key contains product kind, coordinate, and
per-product edit revision; the service instance supplies one immutable source identity. Priority and
coordinate order remain scheduling/envelope data and do not fragment the cache. An edit therefore
invalidates only affected chunk/surface products; unrelated regions retain cache reuse.

The finite generation worker pool is still shared compute. Interest isolation guarantees zero
cross-region presence candidates and entity bytes, but it cannot promise zero CPU contention while
hundreds of clients simultaneously request distinct cold chunks. Region-aware generation admission
and per-region reservations are the next server-throughput step; they do not require world sharding.

## Authority boundaries still to move

Browsers still simulate movement locally, but the server admits poses through bounded receipt-time
horizontal/vertical movement credit; a client discontinuity bit cannot authorize a jump. Edits also
require a fresh same-connection pose and bounded reach. A WAN game still needs server-owned input and
collision simulation plus authenticated identity.

Voxel edits and per-material inventories are native server authority: strict SQLite schema 5 is bound
to the world/source manifest, operations are idempotent per player/edit-session, and commits carry
connection identity and global order, while local chunk/surface revisions prevent unrelated cache
invalidation. The presence spatial index
also acts as the inverse edit-interest subscription, so a player 1 km away receives zero commit bytes.
Bounded per-client queues fail into an explicit full-product resync instead of silently dropping state.

The remaining WAN trust boundary is authenticated ownership and world policy. The local token and
claimed player ID do not prove ownership. Although the server accepts only typed dig/place actions
and enforces fresh poses, bounded reach and movement, and material inventory, Internet deployment
still needs authenticated accounts, permissions, protected regions, moderation/audit policy, and
server-owned movement.
