# Multiplayer scaling envelope

VXWP v5 supports one unsharded authoritative world with up to 512 admitted player sessions. Spatial
interest management is an internal replication index, not a gameplay shard: every player inside the
same location's 256 m interest radius can discover every other player there. Players beyond that
radius contribute no entity records or bytes to one another's presence streams.

## Why v5 replaced complete rosters

The v4 server encoded every player every 33 ms and sent the complete roster to every connection.
The exact frame size was `48 + 80 * players` bytes, making aggregate egress quadratic. At 500 players
that design would send about 607 MB/s (4.85 Gbit/s) before WebSocket/TCP overhead.

V5 hard-bumps the protocol and has no v4 decoder or endpoint. One receiver-specific `PresenceDelta`
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

Run the focused optimized probe with:

```sh
PATH="$HOME/.cargo/bin:$PATH" cargo test --release -p voxels-world-service presence::tests:: -- --nocapture
```

On 2026-07-14 on an Apple M3 Max (16 CPU cores), the checked-in tests measured:

| Scenario                                                              |                                              Result |
| --------------------------------------------------------------------- | --------------------------------------------------: |
| 512 players at one location; one observer discovers 511 peers         |                        8 deltas, 41,264 bytes total |
| Scheduler query, ranking, validation, and encoding for those 8 deltas |                                      1.401 ms total |
| One observer plus 511 players outside its interest cells              | 0 entity bytes after the initial empty stream frame |

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
still render the other five articulated avatars. Per-player `/v6/world` and `/v6/presence` stream and
VXWP byte counts are written to `target/multiplayer-browser/latest.json`; the far observer screenshot
is `target/multiplayer-browser/observer-far-five.png`.

This deliberately does not reuse the persistence browser test's same-profile BroadcastChannel. Such
a test would make local OPFS edits look like networked edits and produce a false multiplayer result.
The JSON currently records the collaborative-tower scenario as `skipped-unsupported`: although VXWP
v6 has a server-edit protocol, the browser has no deterministic hook that can submit an edit via that
production path or inspect a far surface-tile revision. Use the strict form to turn that known gap
into a failing release gate:

```sh
vp run test:multiplayer-browser -- --require-tower
```

Removing the skip requires a diagnostic API which still exercises production behavior: submit a
voxel operation and receive its authoritative revision, place a deterministic camera while marking a
presence discontinuity, and read a bounded voxel or surface-tile revision plus content hash. The
tower assertion must compare the observer's far-LOD revision and rendered fingerprint, not merely the
number of edit rows. Canonical chunks cover only the near field, so an edit-count assertion would not
prove that a tower changes the distant silhouette.

## Capacity and backpressure

The checked-in service admits 512 world sockets plus 512 independently bounded presence sockets.
The world request queue holds 8,192 jobs, exactly one negotiated 16-request window per admitted world
connection. Each session can occupy at most two of the eight blocking generation workers. Presence
has no per-socket outbound queue: a socket builds the newest delta at its next tick and awaits that
single send, so a slow client cannot accumulate stale movement frames.

Identical immutable chunk or surface batches use process-wide single-flight generation. Concurrent
requesters join the first computation, and later requesters reuse its compressed VXWP response from
a 256 MiB byte-bounded LRU while receiving their own request ID. This prevents a crowd spawning in
one place from performing the same CPU or Metal work hundreds of times. The cache key includes
product kind, priority, coordinate order, and the server's immutable source instance; server-owned
edits must add chunk revision before edited products can enter this cache.

The finite generation worker pool is still shared compute. Interest isolation guarantees zero
cross-region presence candidates and entity bytes, but it cannot promise zero CPU contention while
hundreds of clients simultaneously request distinct cold chunks. Region-aware generation admission
and per-region reservations are the next server-throughput step; they do not require world sharding.

## Authority boundaries still to move

Presence is server-relayed but movement is not cheat-safe yet: browsers author poses, and the server
checks finite values, monotonic time/sequence, rate, velocity bounds, and explicit large jumps. A WAN
game needs server-owned input simulation and authenticated identity.

Voxel edits are also not network multiplayer yet. They remain world-scoped SQLite/OPFS state shared
only among tabs in one browser profile, and the service deliberately does not advertise
`SERVER_EDITS`. Enabling a second partial edit authority would be less correct than leaving this
boundary explicit. The next atomic edit phase needs:

1. chunk-partitioned sparse edit state with monotonic per-chunk revisions;
2. idempotent operation IDs and deterministic ordering for same-voxel conflicts;
3. inverse chunk-interest subscriptions so distant clients receive zero edit work;
4. bounded per-client commit queues with an explicit resync marker on overflow;
5. revisioned chunk/surface responses that close the generation-versus-edit race; and
6. native durable server storage before removing the current OPFS authority.

Independent chunks can then commit concurrently while edits touching the same chunk serialize. That
is internal state partitioning in one world, not sharding: everyone subscribed to a location sees the
same ordered edits.
