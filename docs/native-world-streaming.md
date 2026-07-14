# Native world streaming

The browser obtains canonical chunks and coarse-to-fine surface coverage from the native
`voxels-worldd` process over a bounded binary WebSocket connection. The daemon selects
procedural-v16 or Terrain Diffusion; there is no embedded client world generator. Changing the
provider therefore adds no provider branch, model dependency, or Metal API to the browser.

## Why binary WebSocket first

VXWP v5 uses the standard `WebSocket` API over loopback. This is the best first transport for
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

## VXWP v5 contract

Each WebSocket message contains exactly one little-endian VXWP envelope with `VXWP` magic, protocol
version, message kind, request ID, payload length, and reserved fields. The format is code-versioned;
Rust enum layout and Serde output are not wire formats.

1. The browser upgrades `/v5/world`, offering `voxels.world.v5` and the configured local auth token,
   then sends `OpenWorld` with its maximum in-flight batch count and browser-local player claim.
2. The daemon replies with `WorldOpened`: immutable world manifest, source identity/hash,
   capabilities, negotiated request window, echoed player claim, and spawn sample. The client
   rejects an identity mismatch and does not inspect the provider type to choose generation behavior.
3. The browser first submits priority-tagged surface-tile batches in stride-16, stride-8, stride-4,
   then stride-2 order. Each result is independently renderable and cancellable, so useful horizon
   coverage arrives before exact near terrain.
4. The browser concurrently submits exact chunk-coordinate batches. Results retain the request ID
   and coordinate keys, and every decoded product is checked against the negotiated source identity.
5. Each successful near item carries the existing palette/bit-packed VXCH chunk plus an exact,
   palette/bit-packed meshing halo. Both are integrity checked before meshing.
6. Surface meshes use a separate `VXST` version-1 payload containing bounded terrain, water, patch,
   bounds, and transition-skirt records. The coarse parent remains resident until replacement
   coverage is complete.
7. Every chunk or surface result body is independently Brotli-compressed at quality 2 with a 20-bit
   window. Its mandatory v5 envelope declares the exact uncompressed length; the decoder rejects
   unknown codecs, nonzero reserved bytes, outputs above the 16 MiB frame bound, truncated streams,
   and streams producing even one byte beyond the declaration before semantic validation.
8. `Cancel` is best effort. Late, canceled, mismatched, or stale-revision responses are discarded;
   they cannot resurrect an evicted scheduler ticket.
9. `WorldOpened` also returns a connection-scoped, random presence session token. The browser uses
   it to open `/v5/presence` on a dedicated socket; a token cannot be reused by another world
   connection.
10. Browsers send bounded `PlayerPose` latest-state frames. The server validates monotonic sequence,
    finite coordinates, update rate, and explicit teleport discontinuities, assigns a unique color,
    and indexes the pose in a 2D spatial cell. Every receiver gets its own explicit enter/update/leave
    delta. Omission means unchanged, so distant or slow-cadence players are never accidentally
    despawned.
11. Presence ping/pong frames estimate the server clock with an NTP-style four-timestamp offset.
    Receivers render a delayed server timeline, adapt its 67-200 ms interpolation buffer to jitter,
    use clamped Hermite position interpolation and shortest-arc angles, extrapolate for at most
    750 ms, then hold instead of letting an avatar run away. This follows the delayed timeline and
    bounded extrapolation patterns described by [Gaffer on Games][snapshot-interpolation] and
    [Unity Netcode][unity-interpolation], with clock-offset estimation derived from [NTP][ntp].

The required capability set is `CANONICAL_CHUNKS | SURFACE_LOD | PLAYER_PRESENCE`. Server-owned
edits, environment queries, authored routes, inventory, and authoritative multiplayer simulation
still need separate versioned products; the client never substitutes procedural answers for a
remote learned world.

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
endpoint = "ws://127.0.0.1:9777/v5/world"
presence_endpoint = "ws://127.0.0.1:9777/v5/presence"
subprotocol = "voxels.world.v5"
auth_subprotocol_token = "the-same-random-local-token"
```

Ensure the Vite origin is in the server's `allowed_origins`. Then choose one server source.

For procedural-v16:

```toml
source = "procedural-v16"
```

```sh
# Terminal 1
vp run world:serve

# Terminal 2
vp dev
```

For Terrain Diffusion on Apple silicon, fetch the pinned model once, change the source, and run the
Metal-enabled daemon:

```sh
vp run terrain:fetch
```

```toml
source = "terrain-diffusion-30m"
```

```sh
# Terminal 1
vp run world:serve-metal

# Terminal 2
vp dev
```

Open `http://127.0.0.1:5173`, grant Chrome's local/loopback network permission when prompted, and
play normally. To compare generators, change only `source` in `config/world-service.toml`, restart
the matching daemon command, and reload. The client configuration does not change.

## Local players and two-browser testing

The bare URL selects the browser profile's automatic `default` player. A versioned local registry
stores one random browser-user ID and one random player ID per name. The default player therefore
returns to its own last camera location after reload without putting an ID in the URL.

For two local players, keep the same server and Vite processes running and open:

- `http://127.0.0.1:5173/?player=alice`
- `http://127.0.0.1:5173/?player=bob`

Names are lowercase local selectors, limited to 32 ASCII letters, digits, `_`, or `-`; they are not
authentication or secrets. The registry is updated under a Web Lock, so simultaneous first opens
cannot mint competing IDs. `window.__VOXELS__.player` exposes the selected IDs for diagnostics and
`window.__VOXELS__.playerUrl("carol")` returns a correctly formed URL for another named player.

Camera rows are keyed by player ID inside the manifest- and persistence-schema-specific database.
The client initializes only the current schema and never imports, upgrades, or rewrites an older
database. Voxel edits remain world-scoped and are shared live between named players in the same
browser profile.
Different browser profiles, private windows, hostnames, or ports have separate local storage and
OPFS; cross-device state requires the future server-owned player store.

The presence service now replicates positions, velocity, look direction, connection lifetime, and a
server-assigned saturated color. Every receiver animates that state as a thin 13-cuboid figure in one
instanced draw per pass: segmented arms and legs use distance-driven gait, following the same
distance-over-time principle as [Unreal distance matching][distance-matching]; a same-color face
nub makes head direction readable; and large look offsets pull the body around with hysteresis.
These poses are still untrusted client claims. Collision, inventory, edits, respawn, and durable
player state must move to an authenticated authoritative server before Internet multiplayer.

To run the automated two-client release smoke test, keep only the world service on port 9777 running
and leave port 5173 free, then run:

```sh
vp run test:multiplayer-browser
```

It launches two named clients in a temporary Chrome profile, requires both presence streams to
produce one remote avatar, walks Bob while Alice observes, rejects browser/WGPU/socket errors, and
writes ignored screenshots under `target/multiplayer-browser/`. It never opens or resets a user's
normal browser profile or OPFS world.

## Far-LOD transition policy and next step

The streamed surface scheduler requests the coarsest levels first, retains old coverage through
focus movement, and only activates a complete replacement set. The nearest geometric ownership snap
is one 32-voxel chunk (3.2 m), reduced from the former 96-voxel jump that replaced conspicuous 9.6 m
strips. Skirts remain crack protection; they are not treated as a substitute for smooth topology.

The next renderer format should carry conservative parent error and use projected screen-space error,
hysteresis, dual-resident parent/child transition bands, and parent-compatible geomorphing. Those are
the directly applicable parts of [Geometry Clipmaps][geometry-clipmaps], Hoppe's
[smooth view-dependent LOD][smooth-lod], and Epic's hierarchical, demand-streamed
[Nanite geometry model][nanite]. Exact 10 cm voxels should remain gameplay authority while these
cluster/page products remain disposable rendering caches.

Terrain Diffusion currently yields one finite 128x128 height tile at 30 m per sample: 3.84 km square,
with its minimum corner placed by `world_origin_voxels` (the checked-in value centers it on spawn).
Heights are bilinearly sampled into canonical 10 cm columns. The
fidelity-honest composer creates stone, air, and sea-level water only; it does not invent caves,
strata, vegetation, roads, landmarks, or procedural-v16 authored content that the model did not
produce. Out-of-coverage requests fail instead of tiling or falling back to procedural terrain.

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
