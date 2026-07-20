# Native world streaming

The browser obtains canonical chunks and coarse-to-fine surface coverage from the native
`voxels-worldd` process over a bounded binary WebSocket connection. The daemon selects
procedural-v16 or Terrain Diffusion; there is no embedded client world generator. Changing the
provider therefore adds no provider branch, model dependency, or Metal API to the browser.

## Why binary WebSocket first

VXWP v27 uses the standard `WebSocket` API over loopback. This is the best first transport for
reliable chunk assets: it is mature, works in a dedicated worker, and requires neither an HTTP/3
certificate setup nor WebRTC signaling. Axum's native Rust server disables Nagle delay, and the
application bounds outstanding work so classic WebSocket's missing receive-side backpressure cannot
create an unbounded producer.

World products and player presence use separate WebSocket connections. Large reliable chunk frames
therefore cannot head-of-line block the small latest-state pose stream. The world connection remains
request/response and lossless; the presence sender skips an obsolete pose when its tiny
`bufferedAmount` budget is full instead of building latency by queueing it.

The important alternatives remain possible without changing the world-product contract:

- WebTransport is the likely WAN upgrade when independent reliable streams remove meaningful
  connection-level head-of-line blocking. Chrome's [WebTransport guidance][webtransport] confirms
  that it is client-server, supports stream backpressure, and offers both reliable streams and
  unreliable datagrams, but it also requires an HTTPS HTTP/3 server and has a less mature ecosystem
  than WebSocket. VXWP chunk, edit, and world-state products must use reliable streams. Datagrams
  are suitable only for disposable, frequently refreshed data such as transient input or movement
  snapshots; they are MTU-limited, unordered, and may be lost.
- WebRTC data channels are rejected for this client-server path. They add signaling plus
  ICE/DTLS/SCTP negotiation, while their peer-to-peer topology provides no benefit for a browser
  loading authoritative chunks. Chrome's comparison calls out those additional failure surfaces.
- `WebSocketStream` would provide native Streams backpressure, but Chrome still documents it as
  Chrome-only with launch not started. VXWP instead uses the interoperable WebSocket API plus an
  explicit request window and `bufferedAmount` watermarks. See Chrome's
  [WebSocketStream status and backpressure discussion][websocketstream].

The [W3C WebTransport specification][webtransport-spec] supports carrying the same complete VXWP
envelopes on a future reliable stream. Transport choice is deliberately below world identity,
request correlation, and chunk codecs.

## VXWP v27 contract

Each WebSocket message contains exactly one little-endian VXWP envelope with `VXWP` magic, protocol
version, message kind, request ID, payload length, and reserved fields. The format is code-versioned;
Rust enum layout and Serde output are not wire formats.

1. The browser upgrades `/v27/world`, offering `voxels.world.v27` and the configured local auth token,
   then sends `OpenWorld` with its maximum in-flight batch count and browser-local player claim.
2. The daemon replies with `WorldOpened`: immutable world manifest, source identity/hash,
   capabilities, negotiated request window, echoed player claim, spawn sample, authoritative resume
   pose, per-material inventory, and a random edit-session ID. The client
   rejects an identity mismatch and does not inspect the provider type to choose generation behavior.
3. The browser gives the current player body/support, intended movement sweep, and view/edit corridor
   hard urgency through every canonical pipeline stage. It preserves the complete immediate 3x3
   vicinity, then reorders ordinary queued chunks every frame by the configured camera cone and
   velocity-predicted focus. Surface work stays spatially ordered because each level activates
   atomically; it starts with stride-16, stride-8, stride-4, then stride-2 coverage. Once that complete
   set and exact near chunks are ready, stride-64 and stride-32 horizon batches run as `Prefetch`
   work. The server caps global prefetch workers, so kilometre silhouettes cannot consume all
   generation capacity.
4. The browser concurrently submits exact chunk-coordinate batches in that priority order. VXWP
   preserves the ordered coordinates, request ID, and coordinate keys; every decoded product is
   checked against the negotiated source identity. Directional priority changes no cache key,
   residency bound, quality level, or bandwidth ceiling.
5. Each successful near item carries the existing palette/bit-packed VXCH chunk plus an exact,
   palette/bit-packed meshing halo. Both are integrity checked before meshing.
6. Surface meshes use a separate `VXST` v7 payload containing bounded terrain, water, patch
   boundary-face ranges, child/parent shading-height grids, and compact cardinal landscape-horizon
   profiles. Interactive and horizon ownership activate at their own complete-level boundaries; the
   coarse parent remains resident until its replacement is complete, and the renderer derives exact
   height-matched connectors and lighting morphs from the two resident profiles.
7. Every chunk or surface result body is independently Brotli-compressed at quality 2 with a 20-bit
   window. Its mandatory v27 envelope declares the exact uncompressed length; the decoder rejects
   unknown codecs, nonzero reserved bytes, outputs above the 16 MiB frame bound, truncated streams,
   and streams producing even one byte beyond the declaration before semantic validation. Large
   logical frames are paced in fragments keyed by a connection-local transfer ID, independent of
   their embedded request or edit-operation ID. This keeps concurrent edits from different players
   unambiguous even when those players use the same operation number.
8. `Cancel` is best effort. Late, canceled, mismatched, or stale-revision responses are discarded;
   they cannot resurrect an evicted scheduler ticket.
9. `WorldOpened` also returns a connection-scoped, random presence session token. The browser uses
   it to open `/v27/presence` on a dedicated socket; a token cannot be reused by another world
   connection.
10. Browsers send bounded `PlayerPose` latest-state frames. The server validates monotonic sequence,
    finite coordinates, update rate, reported velocity, and receipt-time horizontal/vertical movement
    budgets. Grounded, swimming, gliding, and spectator states are explicit, mutually consistent
    flags; gliding and spectator roles are accepted only when `WorldOpened` advertises their
    capabilities. Spectators retain camera-driven stream interest but have no replicated avatar or
    edit authority. Entering parks the body; leaving restores that authoritative pose. A
    client discontinuity bit never grants movement; discontinuities are server-authored
    interpolation hints. Under inbound pressure, validated poses coalesce to the newest sample before
    authority admission; control and malformed frames remain lossless. The last accepted body pose,
    never a detached spectator camera, becomes the player's reconnect resume point. The server
    assigns a unique color and indexes the current body or spectator camera in a 2D spatial cell.
11. Presence ping/pong frames estimate the server clock with an NTP-style four-timestamp offset.
    Each subsequent ping also returns the previous observed RTT to the server. The shared world and
    presence traffic shaper learns the session's minimum RTT, raises its application pacing rate
    only while real demand is waiting and excess delay is low, halves it when a standing queue grows
    past the configured target, and returns to its safe floor if feedback disappears. Large
    world-product messages are split into rate-aware, bounded VXWP fragments so the scheduler can
    preempt them instead of discovering their byte debt only after one indivisible WebSocket write.
    Receivers render a delayed server timeline, adapt its 67-200 ms interpolation buffer to jitter,
    use clamped Hermite position interpolation and shortest-arc angles, extrapolate for at most
    750 ms, then hold instead of letting an avatar run away. This follows the delayed timeline and
    bounded extrapolation patterns described by [Gaffer on Games][snapshot-interpolation] and
    [Unity Netcode][unity-interpolation], with clock-offset estimation derived from [NTP][ntp].
12. `EditCommand` carries a typed `Dig` or `Place` action plus the server-issued edit-session ID.
    The server expands one dig hit to a fixed 0.5 m canonical-voxel sphere centred on the target,
    credits every removed material, and atomically commits at most 81 exact
    voxel values. Placement changes one voxel and consumes one selected material only when the
    destination is empty. Inventory, mutations, idempotency records, and product revisions share one
    SQLite transaction.
13. Every edit requires a fresh pose from the same connection and the target must be inside the 5 m
    interaction reach plus a hard 1 m cross-socket latency allowance. Commits are fanned out only to
    interested players; only the editor receipt contains the private inventory snapshot. Bounded
    queue overflow produces `ResyncRequired`.

The required capability set is
`CANONICAL_CHUNKS | SURFACE_LOD | PLAYER_PRESENCE | SERVER_EDITS`. Environment queries, authored
routes and fully server-simulated movement inputs still need separate versioned products;
the client never substitutes procedural answers for a remote learned world.

This is progressive at the product level, not an image-style byte-prefix codec: every coarser tile
is a complete useful view and a finer tile is an independently cached replacement. A future `VXSP`
surface-page codec can remove repeated bytes with a lossless parent-predicted height residual,
palette material deltas, and water bitsets. That follows the residual pyramid in
[Geometry Clipmaps][geometry-clipmaps] without making exact chunks depend on earlier layers or
exposing unstable intermediate diffusion states.

## Backpressure and security bounds

The client and server negotiate the smaller in-flight batch count. Request identifiers and
cancellation maps are scoped to each connection, while world identities, player IDs, and absolute
product keys are connection-independent. Two clients can therefore use the same request ID at
unrelated world locations without aliasing. The daemon admits different players concurrently and
rejects a second live connection claiming the same player ID. The client stops admitting work
at that window, pauses sends above its high `bufferedAmount` watermark, resumes below the low
watermark, times out requests, and reconnects with bounded backoff. The daemon independently limits
frame bytes, accepted connections, admitted requests, the generation queue, global blocking jobs,
and blocking jobs per client.
Generation runs off the async socket loop. This makes memory proportional to configured bounds, not
camera speed or server response latency.

The daemon refuses non-loopback listeners, checks the browser `Origin`, and requires a random token
as a second offered WebSocket subprotocol because browser WebSockets cannot set an authorization
header. This token is a local cross-site request defense, not production authentication: it is
present in client configuration. Browser-generated user/player IDs are likewise untrusted local
claims, not proof of identity. A future network deployment must use `wss://`, real credentials,
server-validated player ownership, authorization, quotas, and the same exact VXWP version checks.

Chrome 147 added [Local Network Access restrictions for WebSockets][chrome-147], so Chrome 150+
prompts before a page may connect to loopback. Grant the prompt for the Vite origin. Local development
uses `http://127.0.0.1:5173` to `ws://127.0.0.1:9777`; for a hosted HTTPS client, use a literal local
target and test Chrome's permission/mixed-content behavior, or terminate `wss://` locally. Chrome's
[LNA guidance][lna] explains the secure-context requirement and the mixed-content exemption for
permission-gated destinations that are known to be local.

## Configuration and running

Generate one local token:

```sh
openssl rand -hex 32
```

Copy it to both `config/client.toml` as `[world].auth_subprotocol_token` and
`config/world-service.toml` as `[transport].auth_subprotocol_token`. Keep these client settings:

```toml
[world]
endpoint = "ws://127.0.0.1:9777/v27/world"
presence_endpoint = "ws://127.0.0.1:9777/v27/presence"
subprotocol = "voxels.world.v27"
auth_subprotocol_token = "the-same-random-local-token"
```

Streaming direction is deployment-tunable without changing the wire format:

```toml
[streaming.priority]
collision_lookahead_seconds = 2.0
velocity_lookahead_seconds = 1.5
view_cone_half_angle_degrees = 55.0
```

The current body and support, its intended two-second swept path, and the independent view/edit
corridor are collision-critical. They preempt ordinary generation, meshing, upload, and world-service
traffic, including after conservative unloaded-space collision has stopped actual velocity. The
server gives each connection one bounded collision-generation lane and wakes collision work before
queued ordinary generation without increasing the process-wide worker cap. This urgent set is
bounded secondary interest: it neither increases residency limits nor cancels in-flight work. Beyond
it, queued work inside the cone wins over side and rear work, while velocity look-ahead orders that
cone toward where the player will be. Surface tiles retain spatial order: browser measurements
showed that directional surface ordering only disrupted generation locality because a partial level
cannot be presented.

Ensure the Vite origin is in the server's `allowed_origins`. Terrain Diffusion is the checked-in
default. Fetch the pinned model once, then start the Metal-capable daemon:

```sh
vp run automation -- run terrain-fetch
```

```toml
source = "terrain-diffusion-30m"
```

```sh
vp dev
```

To use procedural-v16 instead, change only the source field and restart the same daemon command:

```toml
source = "procedural-v16"
```

```sh
vp dev
```

Open `http://127.0.0.1:5173`, grant Chrome's local/loopback network permission when prompted, and
play normally. To compare generators, change only `source` in `config/world-service.toml`, restart
`vp dev`, and reload. Vite owns the native daemon lifecycle, so Ctrl-C stops both processes. The
client configuration does not change. Native provider-only checks use the `world-source` and
`terrain-diffusion` automation scenarios instead of managing a persistent diagnostic daemon.

## Local players and two-browser testing

The bare URL selects the browser profile's automatic `default` player. A versioned local registry
stores one random browser-user ID and one random player ID per name. The default player therefore
returns to its server-owned last accepted location and inventory after reload without putting an ID
in the URL.

For two local players, keep the same server and Vite processes running and open:

- `http://127.0.0.1:5173/?player=alice`
- `http://127.0.0.1:5173/?player=bob`

Names are lowercase local selectors, limited to 32 ASCII letters, digits, `_`, or `-`; they are not
authentication or secrets. The registry is updated under a Web Lock, so simultaneous first opens
cannot mint competing IDs. `window.__VOXELS__.player` exposes the selected IDs for diagnostics and
`window.__VOXELS__.playerUrl("carol")` returns a correctly formed URL for another named player.

`voxels-worldd` persists last accepted pose, per-material inventory, and voxel edits in its
manifest-bound native SQLite authority and streams the same revisions to every interested browser
profile. The browser stores only its local player-ID registry; identity is still a local claim until
authenticated accounts exist.

The presence service now replicates positions, velocity, look direction, connection lifetime, and a
server-assigned saturated color. Every receiver animates that state as a thin 13-cuboid figure in one
instanced draw per pass: segmented arms and legs use distance-driven gait, following the same
distance-over-time principle as [Unreal distance matching][distance-matching]; a same-color face
nub makes head direction readable; and large look offsets pull the body around with hysteresis.
These poses remain client simulation output, but bounded server movement admission prevents
teleports and gates edit reach. Collision/input simulation and authenticated ownership must still
move fully server-side before Internet multiplayer.

To run the automated six-client release smoke test, run:

```sh
vp run automation -- run multiplayer
```

It launches six independent browser contexts with shaped links and fresh identities, keeps five
builders visible while an observer walks about 120 m away, then has each builder mine material and
collectively place a 4.5 m column with a 1.7 m crossbar (77 authoritative voxels) from within
validated reach. The gate waits until every browser
has applied all commits, the far observer has accepted and uploaded the revised stride-16 surface
tile, and a before/after pixel comparison proves the aimed tower is visibly legible. It owns a
temporary native edit database, rejects browser/WGPU/socket errors, and writes ignored
metrics/screenshots under
`target/automation/multiplayer/<run-id>/`; the stable
`target/automation/multiplayer/latest.json` pointer identifies the newest run. It never opens or
resets a user's normal browser profile or world.

## Far-LOD transition policy and next step

The streamed surface scheduler requests the coarsest levels first, retains old coverage through
focus movement, and only activates a complete replacement set. The nearest geometric ownership snap
is one 32-voxel chunk (3.2 m), reduced from the former 96-voxel jump that replaced conspicuous 9.6 m
strips. Exact resident connectors close height disagreement at active coarse/fine boundaries, while
hysteretic ownership and shared parent normals reduce the remaining lighting and replacement pop.

The next renderer format should carry conservative parent error and use projected screen-space error,
hysteresis, dual-resident parent/child transition bands, and parent-compatible geomorphing. Those are
the directly applicable parts of [Geometry Clipmaps][geometry-clipmaps], Hoppe's
[smooth view-dependent LOD][smooth-lod], and Epic's hierarchical, demand-streamed
[Nanite geometry model][nanite]. Exact 10 cm voxels should remain gameplay authority while these
cluster/page products remain disposable rendering caches.

Terrain Diffusion currently yields one finite 512x512 height tile at 30 m native resolution. The
checked-in `horizontal_scale = 1` preserves that spacing, making the tile 15.36 km square,
with its minimum corner placed by `world_origin_voxels` (the checked-in value centers it on spawn).
Heights are bilinearly sampled into canonical 10 cm columns. The canonical composer adds bounded,
source-identity-bound subgrid relief, climate-classified surface materials, shallow soil, and
coherent stone/limestone/basalt strata. It still does not invent caves, vegetation, roads,
landmarks, or procedural-v16 authored content that the model did not produce. Out-of-coverage
requests fail instead of tiling or falling back to procedural terrain.

[chrome-147]: https://developer.chrome.com/release-notes/147#local_network_access_lna
[lna]: https://developer.chrome.com/blog/local-network-access
[websocketstream]: https://developer.chrome.com/docs/capabilities/web-apis/websocketstream
[webtransport]: https://developer.chrome.com/docs/capabilities/web-apis/webtransport
[webtransport-spec]: https://www.w3.org/TR/webtransport/
[geometry-clipmaps]: https://developer.nvidia.com/gpugems/gpugems2/part-i-geometric-complexity/chapter-2-terrain-rendering-using-gpu-based-geometry
[smooth-lod]: https://www.microsoft.com/en-us/research/publication/smooth-view-dependent-level-of-detail-control-and-its-application-to-terrain-rendering/
[nanite]: https://dev.epicgames.com/documentation/en-us/unreal-engine/nanite-virtualized-geometry-in-unreal-engine
[snapshot-interpolation]: https://gafferongames.com/post/snapshot_interpolation/
[unity-interpolation]: https://docs.unity.cn/Packages/com.unity.netcode%401.5/manual/interpolation.html
[ntp]: https://www.rfc-editor.org/info/rfc5905/
[distance-matching]: https://dev.epicgames.com/documentation/unreal-engine/distance-matching-in-unreal-engine
