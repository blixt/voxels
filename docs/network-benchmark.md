# Remote world streaming benchmarks

This is the shaped-link streaming suite. See the canonical
[testing and performance map](testing.md) to choose between it, native bot capacity, the six-browser
collaboration gate, renderer profiles, and portable microbenchmarks.

`vp run automation -- run network-benchmark` is the repeatable end-to-end regression suite for
native-server world streaming. It builds release WASM, writes temporary client/server configs, starts
its own `voxels-worldd`, inserts a deterministic full-duplex TCP shaper, runs isolated real-Chrome
contexts, and tears everything down. It does not reuse or reset a developer's OPFS data.

The checked-in `good_remote` profile is deliberately explicit:

- 40 ms round-trip propagation latency, applied as 20 ms in each direction;
- 50 decimal Mbit/s downstream and 10 Mbit/s upstream;
- 16 KiB pacing quanta, ordered reliable delivery, zero jitter, and zero loss.

Each direction has one shared serialization clock across the world and presence sockets, plus a
bounded queue of two bandwidth-delay products (500,000 bytes down and 100,000 bytes up). Requests,
presence poses, and pings therefore compete for the same uplink before the server can react;
responses share the downlink. This follows the proven directional-toxic model used by
[Shopify Toxiproxy](https://github.com/Shopify/toxiproxy), while keeping the regression hermetic on
macOS without privileged firewall changes. Chrome DevTools can throttle WebSockets, but its preset
latency is an implementation-adjusted value rather than an exact one-way delay, so it is not the
canonical timing source here.

## Metrics

The Rust renderer hashes the content and coordinates of opaque and water meshes selected by its
actual frustum/LOD draw list. The harness samples at 16 ms and records snapshots until all canonical
generation/mesh/upload work and every surface queue/in-flight/revision stage are settled with the
same presented geometry for three consecutive samples. Full-coverage timing reports the first
sample in that streak, not the third. A turn is timed from look-input issuance rather than from its
80 ms fixture validation; walking convergence begins after the fixed target distance is reached.

`viewportFullyInformedMs` is retrospective: it is the first post-action sample whose presented
geometry fingerprint equals the final fully settled viewport and never changes again. This lets
view-priority changes improve visible completion even if background rings are still loading.
`fullCoverageSettledMs` is the stricter all-rings measurement. The split is analogous to Cesium's
view-complete contract, where all tiles needed for the current screen-space target must be loaded,
processed, and rendered before the view is complete.

Fixed-distance movement also reports `longestNoProgressMs`. A sample advances after at least 2.5 cm
of horizontal movement; the deterministic route fails if held movement makes no progress for more
than 150 ms. This makes conservative missing-chunk collision walls a direct regression rather than
inferring them from total walk duration or viewport convergence.

The proxy counts delivered TCP stream bytes in both directions and classifies WebSocket/VXWP frames
by endpoint and message kind. “Stream bytes” include HTTP Upgrade and WebSocket framing, but exclude
TCP/IP, TLS, retransmissions, and Ethernet overhead. The JSON also retains VXWP payload counts so the
two layers cannot be confused. Each sample stores byte counters at viewport completion and full
coverage separately. Drained Rust frame history reports frame interval, CPU, simulation, streaming,
and render-submission time so a byte win cannot hide WASM decode stalls.

Scenarios are:

- `cold_spawn`: fresh browser storage, navigation through full viewport and background convergence;
- `short_walk`: a fixed 2 m movement after cold coverage, exercising collision/view-corridor
  scheduling without conflating it with the longer streaming traversal;
- `cached_turn_180`: exact 180° look change after full radial coverage and the zero-world-byte
  control;
- `streaming_walk`: a fixed 35 m sprint followed by post-stop convergence;
- `turn_during_spawn`: exact 180° pivot as soon as first geometry appears, exposing view-priority and
  obsolete-work behavior during a cold load.

The generated versioned JSON and Markdown live under
`target/automation/network-benchmark/<run-id>/`. The stable
`target/automation/network-benchmark/latest.json` pointer identifies the last completed run. Five
repetitions are the default:

```sh
vp run automation -- run network-benchmark
vp run automation -- run network-benchmark --runs=1
vp run automation -- run network-benchmark --runs=1 --profile=constrained --rtt-ms=80 \
  --downstream-mbps=2 --upstream-mbps=1
vp run automation -- run network-compare target/automation/before/report.json \
  target/automation/after/report.json
```

The runner also records peak user-space queue delay after removing propagation and one pacing
quantum's own serialization, peak queued bytes, and every time its bounded queue pauses the TCP
source. These distinguish useful throughput from a faster sender that merely creates bufferbloat.
Any server control-error frame fails the run; rejected requests are neither useful throughput nor a
valid way to make a client appear settled.
The comparison command refuses different result schemas, browser snapshot schemas, fixtures,
protocols, link profiles, world sources, repetition counts, or execution environments. It reports
signed median, max, full-coverage, viewport-byte, and full-coverage-byte deltas; negative is an
improvement and positive is a degradation. Five samples are too few to call the maximum a
statistically useful p95, though the versioned JSON retains the order statistic. Millisecond
comparisons are machine-local, so even CPU, OS, Chrome, Node, or logical-core changes make artifacts
incomparable. World bytes are deterministic; presence bytes vary slightly with scenario duration.

## 2026-07-14 VXWP v4 compressed baseline

Five clean repetitions at commit `aec4dc4c8655` on an Apple M3 Max and Chrome 150.0.7871.115 used
result schema 2 and VXWP v4:

| Scenario          | Viewport median | Viewport max | Full coverage median | World bytes at viewport | Total bytes at full coverage | Frame p95 | Worst frame |
| ----------------- | --------------: | -----------: | -------------------: | ----------------------: | ---------------------------: | --------: | ----------: |
| Cold spawn        |      2,354.5 ms |   2,483.4 ms |           2,354.5 ms |               2,354,038 |                    2,383,225 |   10.1 ms |     18.4 ms |
| Resident walk     |         16.9 ms |      76.6 ms |              16.9 ms |                       0 |                        3,128 |   10.0 ms |     10.3 ms |
| Cached 180° turn  |         16.6 ms |      17.7 ms |              16.6 ms |                       0 |                          214 |   10.1 ms |     10.2 ms |
| 35 m stream walk  |         46.8 ms |      61.8 ms |              46.8 ms |                 429,388 |                      473,776 |   10.0 ms |     16.7 ms |
| Turn during spawn |      2,017.4 ms |   2,025.8 ms |           2,017.4 ms |               2,354,038 |                    2,383,119 |   10.1 ms |     16.9 ms |

Across 5,972 captured frames, none exceeded 33.33 ms and no frame-history samples were dropped.
Cold-spawn streaming work was 2.7 ms p95. The resident walk and cached turn still sent zero world
bytes; their small totals are presence traffic.

The directly comparable cold-spawn viewport metric improved from 3,448.0 ms in v3 to 2,354.5 ms,
an improvement of 1,093.5 ms or 31.7%. Full-coverage stream traffic fell from 18,352,347 to
2,383,225 bytes, an 87.0% reduction. The fixed-distance streaming walk fell from 5,008,628 to
473,776 bytes, a 90.5% reduction. Other time deltas are not quoted across schemas because v2 fixed
the stable-sample and turn-boundary biases described above.

## 2026-07-14 VXWP v3 historical baseline

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
during the action. This v3 result is retained as historical evidence, not as a comparator fixture:
result schema 2 intentionally rejects it.

### From compression headroom to VXWP v4

The v3 runner captured each chunk and surface result message, then compressed it independently after
timing with Node's native zstd level 1, Brotli quality 4, and deflate level 1. One cold spawn's exact
VXWP result payloads were:

| Message type            | Current bytes | zstd level 1 | Reduction | Brotli q4 | Deflate level 1 |
| ----------------------- | ------------: | -----------: | --------: | --------: | --------------: |
| 122 chunk batch results |     1,606,031 |       83,213 |     94.8% |    76,920 |          99,987 |
| 91 surface tile results |    16,707,432 |    2,461,682 |     85.3% | 2,396,450 |       3,238,430 |

Surface tiles were 91.2% of the v3 full-coverage VXWP payload and are fixed-width records. VXWP v4
therefore hard-bumped the protocol and made every chunk or surface result an independently decodable
Brotli frame. Independent results preserve request correlation and cancellation boundaries under
[RFC 6455 WebSocket ordering](https://www.rfc-editor.org/rfc/rfc6455.html). The wire envelope declares
the exact uncompressed length; the Rust decoder caps it at 16 MiB, reads at most one extra byte, and
rejects truncated, undersized, oversized, reserved, or unknown-codec payloads before semantic decode.

The implementation uses the safe, pure-Rust [`brotli` 8.0.4 crate](https://docs.rs/brotli/8.0.4/brotli/)
on both the native server and WASM client, with no C/FFI path. Exploratory one-run end-to-end tests
selected quality 2 and a 20-bit window: quality 1 used about 3.79 MB, quality 2 about 2.38 MB, quality
3 about 2.49 MB, while quality 4 raised cold convergence to about 4.18 s. The clean v4 cold fixture's
122 chunk results total 90,027 VXWP bytes and its 91 surface results total 2,262,694 bytes. Release
WASM grew from 3,501.58 KiB to 3,696.05 KiB (194.47 KiB, 5.6%), while captured frame timings show no
decode jank on this machine.

### Experiments that did not land

- Re-sorting unsent surface tiles by camera half-space was implemented in `f11b72b` and measured over
  five runs. Turn-during-spawn improved only 6.1 ms (0.3%) and used 996 more world bytes at viewport
  completion; cold spawn was unchanged. `fe55312` reverts the experiment while retaining it in
  history.
- Raising surface batches from four to eight saved about 4.7 KiB in one cold run but regressed cold
  convergence from 2.35 s to 2.85 s and produced a 275.1 ms main-thread frame, so the four-tile batch
  remains.

## 2026-07-18 canonical view-and-velocity priority

VXWP v26 preserves the complete immediate 3x3 canonical-chunk vicinity, then reorders unstarted
generation, meshing, and upload tickets every frame by a 55-degree camera cone and a 1.5-second
velocity prediction. This changes request order only: residency, cache identity, product fidelity,
and every traffic bound remain unchanged. The values are typed client configuration.

A controlled three-run comparison used the same 40 ms RTT, 50/10 Mbit link and fresh Terrain
Diffusion worlds. Baseline `fc79314611e8` is pre-priority commit `28ec0cf` with the identical
terminal-fall fix cherry-picked; candidate `0af25fb71c7f` is the canonical-only policy:

| Scenario               | Viewport median delta |         Max delta | World bytes at viewport |
| ---------------------- | --------------------: | ----------------: | ----------------------: |
| Streaming walk         |     -107.4 ms (-7.5%) | -128.8 ms (-8.7%) |                  +225 B |
| Turn during spawn      |     -266.9 ms (-2.9%) | -159.7 ms (-1.7%) |                  +516 B |
| Cached 180-degree turn |       -0.5 ms (-2.9%) |   -0.9 ms (-4.8%) |               unchanged |
| Cold spawn             |     +259.6 ms (+2.7%) | -125.9 ms (-1.2%) |                  +269 B |
| Resident pillar walk   |      +18.0 ms (+1.3%) |  +18.3 ms (+1.3%) |                  -692 B |

The candidate recorded zero frames above 33.33 ms. Dynamic scenarios improved with effectively zero
bandwidth change; the mixed cold median/max result is treated as neutral three-run variance rather
than a claimed win.

Applying the same directional policy to surface tiles did not land. A three-run candidate reached
the streaming-walk viewport 128 ms sooner than radial order, but made turn-during-spawn 1.69 seconds
slower and produced a 20.9-second outlier. Surface levels activate atomically, so partial directional
progress cannot be presented; reordering only disrupted spatial generation locality. Surface tiles
therefore retain their proven coarsest-first radial order.

The next credible gains are not constant changes: they require priority-aware server admission,
best-effort cancellation of genuinely obsolete focus work, or a bespoke progressive `VXST` surface
codec. Those need new protocol/scheduler invariants and multi-client fairness tests, so they are the
boundary beyond this low-hanging optimization pass.

## 2026-07-19 swept collision urgency

Canonical work under the player and across the configured intended-movement sweep now preempts
ordinary generation, meshing, upload, and world-service traffic. Server generation admission wakes
collision work before queued ordinary work, with one bounded collision lane per connection and no
increase to the process-wide worker cap. The direction comes from current input and locomotion rather
than post-collision velocity, so conservative missing-chunk collision cannot erase the request
pressure needed to remove its own temporary boundary. The independent view/edit corridor remains
urgent.

A one-run Chrome 150 check on the same 40 ms RTT, 50/10 Mbit link and procedural fixture covered
35.045 m in 4,984.1 ms. Its longest interval without at least 2.5 cm of progress was 27.3 ms, the
viewport was fully informed 169.1 ms after stopping, no frame exceeded 33.33 ms, and streaming work
was 2.4 ms p95. This is a correctness smoke test rather than a statistically stable performance
comparison; use the default five runs for optimization claims.
