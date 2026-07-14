# Remote world streaming benchmarks

`vp run bench:network` is the repeatable end-to-end regression suite for native-server world
streaming. It builds release WASM, writes temporary client/server configs, starts its own
`voxels-worldd`, inserts a deterministic full-duplex TCP shaper, runs isolated real-Chrome contexts,
and tears everything down. It does not reuse or reset a developer's OPFS data.

The checked-in `good_remote` profile is deliberately explicit:

- 40 ms round-trip propagation latency, applied as 20 ms in each direction;
- 50 decimal Mbit/s downstream and 10 Mbit/s upstream;
- 16 KiB pacing quanta, ordered reliable delivery, zero jitter, and zero loss.

Each direction has an independent serialization clock and bounded queue. Requests, presence poses,
and pings therefore pay uplink delay and bandwidth before the server can react; responses pay the
downlink independently. This follows the proven directional-toxic model used by
[Shopify Toxiproxy](https://github.com/Shopify/toxiproxy), while keeping the regression hermetic on
macOS without privileged firewall changes. Chrome DevTools can throttle WebSockets, but its preset
latency is an implementation-adjusted value rather than an exact one-way delay, so it is not the
canonical timing source here.

## Metrics

The Rust renderer hashes the content and coordinates of opaque and water meshes selected by its
actual frustum/LOD draw list. After the camera action finishes, the harness records snapshots until
all canonical generation/mesh/upload work and every surface queue/in-flight/revision stage have
settled for three samples.

`viewportFullyInformedMs` is retrospective: it is the first post-action sample whose presented
geometry fingerprint equals the final fully settled viewport and never changes again. This lets a
future view-priority scheduler improve visible completion even if background rings are still loading.
`fullCoverageSettledMs` is the stricter all-rings measurement. The split is analogous to Cesium's
view-complete contract, where all tiles needed for the current screen-space target must be loaded,
processed, and rendered before the view is complete.

The proxy counts delivered TCP stream bytes in both directions and classifies WebSocket/VXWP frames
by endpoint and message kind. “Stream bytes” include HTTP Upgrade and WebSocket framing, but exclude
TCP/IP, TLS, retransmissions, and Ethernet overhead. The JSON also retains VXWP payload counts so the
two layers cannot be confused.

Scenarios are:

- `cold_spawn`: fresh browser storage, navigation through full viewport and background convergence;
- `resident_walk`: about 2.1 m inside the already loaded spawn chunk, a zero-world-byte control;
- `cached_turn_180`: exact 180° look change after full radial coverage;
- `streaming_walk`: five seconds of sprinting, about 35.3 m, followed by post-stop convergence;
- `turn_during_spawn`: exact 180° pivot as soon as first geometry appears, exposing view-priority and
  obsolete-work behavior during a cold load.

The generated versioned JSON and Markdown live under ignored `target/network-benchmark/`, including
`latest.json` and `latest.md`. Three repetitions are the default:

```sh
vp run bench:network
vp run bench:network -- --runs=1
vp run bench:network:compare -- target/network-benchmark/before.json \
  target/network-benchmark/after.json
```

The comparison command refuses different result schemas or link profiles. It reports signed median,
p95, full-coverage, and byte deltas; negative is an improvement and positive is a degradation. Keep
millisecond comparisons machine-local. Stream-byte comparisons are deterministic enough for CI once
presence duration is separated or tolerated.

## 2026-07-14 baseline

Three clean repetitions at commit `4ef449fed07a` on an Apple M3 Max and Chrome 150.0.7871.115 produced:

| Scenario              | Viewport informed median |        p95 | Full coverage after action median | Median stream bytes | Median downstream |
| --------------------- | -----------------------: | ---------: | --------------------------------: | ------------------: | ----------------: |
| Cold spawn            |               3,448.0 ms | 3,466.1 ms |                        3,552.8 ms |          18,352,347 |        18,328,769 |
| Resident walk         |                 115.5 ms |   122.9 ms |                          116.0 ms |               5,240 |             3,226 |
| Cached 180° turn      |                  21.4 ms |    25.1 ms |                          126.4 ms |               1,416 |               924 |
| 35.3 m streaming walk |                   7.6 ms |    12.1 ms |                          111.2 ms |           5,008,628 |         4,983,970 |
| Turn during spawn     |               3,124.4 ms | 3,146.0 ms |                        3,229.9 ms |          18,352,643 |        18,328,851 |

The resident walk and cached turn sent zero bytes on `/v3/world`; their small totals are live
presence traffic. The streaming-walk viewport was already informed by the time the five-second action
ended, so its 7.6 ms number is explicitly post-stop, while its 5.01 MB total captures all traffic
during the action. The cold pivot is only about 324 ms faster than navigation-start cold spawn even
though its timer begins at the pivot: current canonical and surface schedulers are distance-based,
not view-direction based, so this is the benchmark expected to improve most from viewport priorities
and cancellation.

### Compression headroom

The runner captures each chunk and surface result message, then compresses it independently after
timing with Node's native zstd level 1, Brotli quality 4, and deflate level 1. This is a size experiment,
not enabled protocol compression, and it does not yet measure Rust encoder or WASM decoder CPU cost.
Independent messages preserve cancellation and avoid whole-stream dependencies, consistent with
[RFC 6455 WebSocket ordering](https://www.rfc-editor.org/rfc/rfc6455.html) and the independently
decodable frame model in [RFC 8878 Zstandard](https://www.rfc-editor.org/rfc/rfc8878.html).

One cold spawn's exact VXWP result payloads were:

| Message type            | Current bytes | zstd level 1 | Reduction | Brotli q4 | Deflate level 1 |
| ----------------------- | ------------: | -----------: | --------: | --------: | --------------: |
| 122 chunk batch results |     1,606,031 |       83,213 |     94.8% |    76,920 |          99,987 |
| 91 surface tile results |    16,707,432 |    2,461,682 |     85.3% | 2,396,450 |       3,238,430 |

Surface tiles are 91.2% of the current full-coverage VXWP payload and are fixed-width records. The
first production compression experiment should therefore hard-bump VXWP and compress each chunk or
surface result independently with zstd level 1, declaring and bounding the uncompressed length before
allocation. A bespoke surface codec can follow. Do not infer that Brotli is the runtime winner from
its slightly smaller size here: encode/decode latency and WASM scratch memory still need a Rust corpus
benchmark before selecting the production codec.
