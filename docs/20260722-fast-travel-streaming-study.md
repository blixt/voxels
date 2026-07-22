# Fast-travel world streaming study

## Goal and method

The target is continuous travel at the configured maximum spectator speed of 128 m/s while keeping
the terrain around the camera at stride 1 whenever possible. The comparison uses the real release
WASM client, native `voxels-worldd`, Chrome 150, a fresh procedural world, and the `good_remote`
link: 40 ms RTT, 50 Mbit/s down, and 10 Mbit/s up. The full route includes cold spawn, a short walk,
a cached turn, a 35 m streaming walk, 10.5 seconds of spectator acceleration, 15 seconds at full
speed, and a turn during spawn.

These are single-run engineering experiments on the same Apple M3 Max, not confidence intervals.
They are sufficient to reject large effects and choose the next design; performance claims that gate
a release should repeat the winning and baseline commits at least five times.

At 128 m/s, the camera crosses 40 canonical 3.2 m chunk columns per second and 20 stride-2 surface
tiles per second. Generating the complete fine square around every successive camera tile asks for
far more new work than can become visible. The useful question is therefore not only how quickly a
byte crosses the socket, but which bounded subset of geometry receives CPU, response-window,
network, decode, upload, and draw-list work before the camera arrives.

## Diagnosis

The original full-speed run stayed coarse for its complete 15-second sample:

- stride-1 presentation ratio: 0%;
- longest degraded interval: 14,996.7 ms;
- median surface queue: 658 tiles;
- downstream world traffic: 5.25 MB;
- downstream peak queue delay: 1.84 ms with no proxy backpressure.

The link was not saturated. Generation and scheduling debt accumulated while the client ranked a
1.5-second prediction approximately 192 m ahead, outside the resident window still centered on the
camera. The highest-ranked requests were therefore often not part of the set that could replace the
coarse terrain currently being presented.

## Experiments

All flight figures below are from the mixed full route unless labelled flight-only. “Fine” is the
fraction of full-speed samples whose renderer actually presented stride-1 terrain. Parent LODs stay
resident during every candidate, so an unfinished refinement must retain geometric coverage.

| Candidate                      | Commit    |      Fine | Longest coarse | Median queue | Flight world bytes | Worst frame | Verdict                        |
| ------------------------------ | --------- | --------: | -------------: | -----------: | -----------------: | ----------: | ------------------------------ |
| Original camera-centred window | `71178f0` |      0.0% |    14,996.7 ms |          658 |            5.25 MB |     25.4 ms | Baseline                       |
| Priority/preemption only       | `30eb86b` |      0.0% |    14,983.3 ms |          639 |            5.59 MB |     25.1 ms | Necessary, insufficient        |
| Led square window              | `0acb49e` |     71.7% |     2,065.9 ms |          655 |            5.06 MB |     50.1 ms | Large fidelity win, unstable   |
| Smaller led square at speed    | `d944078` |     78.8% |       581.9 ms |          457 |            5.84 MB |     17.6 ms | Better, but shortened lead     |
| Led velocity corridor          | `b4affed` | **90.7%** |   **151.5 ms** |          497 |            5.67 MB | **17.6 ms** | Winner                         |
| One tile per request           | `684a53f` |     62.0% |     1,041.1 ms |          552 |            2.50 MB |     25.1 ms | Request-window bound; reverted |
| Eight tiles per request        | `3668576` |     87.3% |       156.0 ms |          452 |            9.89 MB |     25.6 ms | Too much stale work; reverted  |

The isolated flight-only led-window run reached 98.9% stride-1 presentation with a 42.8 ms longest
degraded interval. The mixed route is deliberately harder because its earlier movement and turn
leave realistic resident and queued work behind; the corridor's 90.7% result is the more useful
number.

A five-second flight-only sanity run on the native Metal Terrain Diffusion source also presented
stride 1 for 98.5% of the measurement, with an 18.4 ms longest degraded interval, 2.08 MB of world
traffic, and a 25.9 ms worst frame. Its strict whole-corridor surface-ready ratio remained zero while
moving; that metric requires every desired tile to be current simultaneously, whereas the renderer's
presented-stride metric measures the terrain actually visible at the camera.

A later 120-second procedural flight verified that this behavior does not decay with distance. The
camera travelled 15.36 km at 128 m/s. Ten-second generation windows stayed at roughly 4,790 accepted
completions with zero stale completions; the final window delivered 1.21 times the first window's
world bytes per second. The surface queue rose during acceleration, then remained in a bounded
roughly 700-790-tile band instead of growing with distance. Stride 1 was presented for 99.8-100% of
samples in the final nine complete windows; isolated coarser samples recovered within 87.1 ms.

The retained four-tile batch is the measured balance. One-tile responses consumed the request
window, made cold spawn 43% slower, and reduced useful flight throughput. Eight-tile responses
amortized scheduling but produced 74% more flight bytes than four, canceled more large requests,
and caused one 34.1 ms streaming-walk frame. A future per-item response format should preserve
four-item generation locality while allowing completed items to publish and cancel independently.

Increasing the per-client generation allowance from two to seven workers improved one cold spawn
from 10.44 s to 8.75 s but left full-speed fine presentation at 0% in both runs. More concurrency is
useful for a bounded cold burst, but it did not repair bad request selection and would multiply
per-player CPU consumption under load.

## Retained design

1. Exact chunks and surface tiles move their streaming focus a bounded distance along velocity. The
   lead is capped by each resident radius, so the current camera never falls outside coverage.
2. Fine surface levels become velocity-aligned corridors only when their tile-crossing rate exceeds
   the configured useful-completion rate. Longitudinal lead is unchanged; only cross-track work is
   reduced. Ordinary running and a stopped camera retain the complete square footprint.
3. Direction, focus, and corridor changes cancel a batch as soon as any item is obsolete. Useful
   siblings are requeued under the current focus; obsolete siblings are discarded. This prevents
   one useful tile from letting three obsolete tiles retain an equal-priority socket slot.
4. A higher-priority request may preempt stale lower-priority work in the bounded client window.
   Collision remains the strongest class, followed by visible exact chunks, the current tile at
   each interactive surface level, visible surface work, replacements, and prefetch.
5. Canceling a fragmented response emits an explicit VXWP fragment-abort frame. Without it, the
   client retained a partial reassembly slot forever; repeated fast-travel cancellation exhausted
   all 32 slots and disconnected the world socket. VXWP v32 deliberately has no legacy path.
6. Four surface tiles remain one generation request. Coarser parents remain rendered until their
   exact replacement cut is ready. The connector builder joins any selected finer level directly,
   not only the adjacent level, using the child product's embedded immediate-parent height. A
   30-second diagnostic-sky run covered 3.25 km in 80 captured frames with zero exposed sky pixels,
   zero ownerless camera samples, and zero incomplete connector edges.

The policy is controlled by the versioned client configuration:

```toml
[streaming.surface]
load_radius_tiles = [5, 5, 5, 5, 4, 5, 4, 4]
fast_travel_min_cross_radius_tiles = [2, 2, 4, 5, 4, 5, 4, 4]
fast_travel_full_rate_tiles_per_second = 4.0
```

## Hypothesis disposition

### Implemented and supported by measurements

- **Velocity-led residency, not only velocity sorting.** Sorting work outside the current coverage
  set cannot improve the presented viewport. Moving the bounded window gave the dominant win.
- **Speed-adaptive, anisotropic fine coverage.** A narrow corridor retains lead time and spends
  fine-detail work where travel can expose it. A smaller square was measurably weaker.
- **Preemption and cancellation.** They prevent obsolete prefetch from owning a scarce request slot,
  but only after the desired set itself is useful.
- **Explicit cancellation at the fragmentation layer.** Application cancellation must release both
  server work and client reassembly state.
- **Four-item surface batches.** Batch sizes 1, 4, and 8 were measured; 4 won the combined fidelity,
  latency, CPU, and bandwidth trade.

### Investigated and rejected for the current bottleneck

- **More raw link throughput or multiple WebSockets.** Peak shaped-link delay stayed below 5 ms and
  no run applied downstream backpressure. More TCP connections would not generate the right tile
  sooner and would add competing congestion controllers.
- **More per-client workers as the primary fix.** Seven workers accelerated cold completion but did
  not improve full-speed presentation before request selection was fixed. It is an expensive
  multiplayer default.
- **Smaller canonical chunks.** At a fixed volume, halving each dimension creates eight times as many
  identities, halos, cache entries, requests, and activation decisions. Canonical collision already
  remains useful during flight; the missing visual product is the surface hierarchy. Smaller chunks
  are not justified without a benchmark showing locality outweighs this amplification.
- **One request per tile.** The direct experiment exposed request-window starvation. Finer delivery
  granularity must not discard generation batching.

### High-value next experiments

1. **Progressive per-item batch results.** Generate a four-item batch for locality, emit each encoded
   item when ready, and cancel only unfinished items. Compare time-to-first-fine-tile, useful bytes,
   cancellation waste, and server fairness. This is the most direct next step.
2. **Screen-space-error refinement.** Replace fixed ring readiness with projected error, but request
   a complete sibling/group cut before replacing its parent. OGC 3D Tiles defines screen-space
   error, replacement/additive refinement, and request volumes; Nanite similarly streams fixed pages
   and switches complete cluster groups rather than arbitrary children. See the
   [3D Tiles specification](https://docs.ogc.org/cs/22-025r4/22-025r4.html) and
   [Nanite SIGGRAPH 2021 slides](https://advances.realtimerendering.com/s2021/Karis_Nanite_SIGGRAPH_Advances_2021_final.pdf).
3. **A surface-specific progressive codec.** Send occupancy/material parent data first, followed by
   residuals that refine the same stable tile identity. Measure decode cost as well as bytes.
   Meshoptimizer's published codecs and hierarchical cluster tooling are a useful reference for
   independently decodable, quantized geometry pages, including shared-position quantization that
   avoids cracks: [meshoptimizer](https://github.com/zeux/meshoptimizer).
4. **Deadline feedback.** Feed measured generation EWMA and link RTT into corridor lead instead of a
   fixed prediction, with bounded hysteresis so changes do not thrash the desired set.
5. **Per-session work conservation.** Permit extra workers only while global capacity is idle, then
   revoke them immediately when other players have collision or visible work. Validate with the
   1,000-player harness before changing production defaults.
6. **WebTransport after the application pipeline is progressive.** QUIC streams avoid head-of-line
   blocking between independent streams and can be canceled independently
   ([RFC 9000](https://www.rfc-editor.org/rfc/rfc9000.html),
   [RFC 9308](https://www.rfc-editor.org/rfc/rfc9308.html)). WebTransport exposes reliable streams
   and datagrams with Streams API backpressure ([W3C WebTransport](https://www.w3.org/TR/webtransport/)),
   but its HTTP/3 mapping deliberately does not define application priority. Geometry grouping and
   scheduling still belong to VXWP. It becomes compelling when packet-loss tests show one large
   reliable response delaying independent collision or fine-tile data.

Chrome's `WebSocketStream` would provide cleaner receive-side backpressure but is not a portable
transport and does not add QUIC multiplexing. The current application already has bounded send
windows and measured zero link backpressure, so changing only that API has low expected value
([Chrome WebSocketStream](https://developer.chrome.com/docs/capabilities/web-apis/websocketstream)).

## Reproduction

```sh
# Complete real-player route plus 15 seconds at 128 m/s
vp run automation -- run network-benchmark --runs=1 --source=procedural-v16 \
  --flight-seconds=15

# Faster iteration on only the accelerated flight segment
vp run automation -- run network-benchmark --runs=1 --source=procedural-v16 \
  --flight-seconds=15 --flight-only

# Explicit server concurrency experiment
vp run automation -- run network-benchmark --runs=1 --source=procedural-v16 \
  --flight-seconds=15 --flight-only --generation-workers-per-client=7

# Strict moving geometry-coverage gate (fails on one diagnostic-sky pixel)
vp run automation -- run lod-transition --mode=travel-coverage \
  --source=procedural-v16 --travel-seconds=30
```

The retained mixed-route artifact is
`target/automation/network-benchmark/2026-07-22T15-48-23-734Z-b074a8c9/report.json`.
The baseline is
`target/automation/network-benchmark/2026-07-22T14-30-47-533Z-024e62a8/report.json`.
The sustained-flight artifact is
`target/automation/network-benchmark/2026-07-22T19-41-07-699Z-084b6d85/report.json`; the strict
non-adjacent-connector coverage artifact is
`target/automation/lod-transition/2026-07-22T19-49-21-064Z-6c9441ce/report.json`.
Artifacts are intentionally ignored and use isolated temporary worlds; the measurements above are
recorded here so the design evidence survives artifact cleanup.
