# Testing and performance map

This is the canonical index for Voxels verification. Use the smallest row that covers a change, then
move down to a real-browser or multi-client gate when the change crosses that boundary. All
integration harnesses create explicit temporary configs, databases, ports, browser contexts, and
profiles. They do not reuse or reset the development world or browser OPFS data.

## Fast path

```sh
# Ordinary code change
vp run verify

# Multiplayer protocol, state, or edit change
vp run test:bots
vp run test:multiplayer-browser -- --require-tower

# Server capacity, dense edits, presence, or avatar rendering
vp run bench:bots

# Streaming, scheduling, compression, or remote-link change
vp run bench:network
```

`vp run verify` is the complete static and build gate: TypeScript checks, TypeScript tests, host Rust
tests, host/WASM Clippy, and the production browser build. The specialized harnesses below provide
behavioral, visual, resource, or transport evidence that the general gate cannot.

## Test surfaces

| Area                 | Command                                                  | What it proves                                                                               | Output                              |
| -------------------- | -------------------------------------------------------- | -------------------------------------------------------------------------------------------- | ----------------------------------- |
| TypeScript           | `vp check`, `vp test`                                    | Formatting, lint, types, and script/unit contracts                                           | Terminal                            |
| Rust and WASM        | `vp run check:rust`                                      | Workspace tests plus host and WASM Clippy                                                    | Terminal                            |
| Production build     | `vp build`                                               | Optimized WASM, shaders, and web assets compile                                              | `dist/`                             |
| Complete static gate | `vp run verify`                                          | All three rows above                                                                         | Terminal                            |
| Native bot smoke     | `vp run test:bots`                                       | Four real VXWP clients connect, move, stream, edit, and replicate                            | `target/harness/bots/`              |
| Bot population/load  | `vp run bench:bots`                                      | 4/8/16/32/64 mixed bots, process resources, disk, wire bytes, and one real browser           | `target/harness/bots/`              |
| Six browser users    | `vp run test:multiplayer-browser -- --require-tower`     | Isolated identities, shaped WAN links, avatars, collaborative edits, and far-LOD convergence | `target/multiplayer-browser/`       |
| Remote streaming     | `vp run bench:network`                                   | Cold spawn, walk, turn, viewport readiness, full coverage, and exact link bytes              | `target/network-benchmark/`         |
| Network comparison   | `vp run bench:network:compare -- before.json after.json` | Compatible before/after timing and byte deltas                                               | Terminal                            |
| LOD transition       | `vp run test:lod-browser`                                | Real-browser handoff and image evidence while crossing LOD boundaries                        | `target/lod-transition/`            |
| Watertight LOD       | `vp run test:lod-watertight-browser`                     | Strict seam/coverage regression path                                                         | `target/lod-transition/`            |
| Terrain boundary     | `vp run test:terrain-boundary-browser`                   | Reported vertical chunk-boundary camera, cool sky exposure, and exact browser scale          | `target/terrain-boundary-coverage/` |
| LOD comparison video | `vp run capture:lod-video`                               | Raw Chromium traversal plus an annotated same-pose A/B MP4 delivered to the Desktop          | `target/lod-video/`                 |
| World Lab/UI         | `vp run test:world-lab-browser`                          | Rust UI interaction and synchronized world diagnostics                                       | `target/world-lab/`                 |
| Weather motion       | `vp run test:weather-motion-browser`                     | World-anchored clouds and downward precipitation                                             | `target/weather-motion/`            |
| Renderer profile     | `vp run profile:browser`                                 | Release frame/CPU/GPU distributions in system Chrome                                         | `target/render-profile/`            |
| Chromium trace       | `vp run profile:chromium`                                | The renderer profile plus a standalone CDP trace                                             | `target/render-profile/`            |
| Sustained traversal  | `vp run profile:sustained`                               | Warmed fixed rail, allocation plateau, streaming, and frame pacing                           | `target/render-profile/`            |
| Material cost        | `vp run profile:materials`                               | Geometry-invariant material-detail A/B                                                       | `target/render-profile/`            |
| Day/night cost       | `vp run profile:atmosphere`                              | Synchronized celestial anchors and lighting budgets                                          | `target/render-profile/`            |
| Weather cost         | `vp run profile:weather`                                 | Six weather anchors, geometry invariants, clouds, rain, and GPU budgets                      | `target/render-profile/`            |
| Portable world       | `vp run bench:world`                                     | Generation, VXCH codecs, and meshing Criterion baselines                                     | `target/criterion/`                 |
| Portable simulation  | `vp run bench:core`                                      | Dry/submerged fixed-step simulation baselines                                                | `target/criterion/`                 |
| Streaming runtime    | `vp run bench:runtime`                                   | Portable runtime scheduling baselines                                                        | `target/criterion/`                 |

[Remote world streaming benchmarks](network-benchmark.md), [multiplayer scaling](multiplayer-scaling.md),
and [recorded renderer baselines](performance.md) explain the corresponding metrics and historical
results. Terrain Diffusion has additional provider-specific smoke and survey commands in
[the native Metal provider notes](terrain-diffusion-metal.md).

## Native multiplayer bots

`voxels-bots` is one native Rust process containing concurrent protocol-faithful players. Each bot
opens the same strict VXWP world and presence WebSockets as a browser. There is no bot-only endpoint,
server bypass, direct database mutation, or legacy protocol path.

The deterministic roster cycles through:

- **Explorer:** sprints along a seeded meandering heading, requests newly encountered chunks, and
  exercises cold generation and streaming.
- **Digger:** alternates descending and horizontal excavation, using ordinary reach-checked edits and
  authoritative inventory commits.
- **Builder:** mines material, clears a unique worksite, and extends a terrain-aware tower column.
  Returning builders inspect streamed authoritative voxels and continue above existing work.
- **Follower:** tracks the preceding builder's replicated pose and copies each observed dig/place
  intent once at its own reachable worksite.

The normal capacity curve uses a fresh isolated world service and database for each population:

```sh
vp run bench:bots
vp run bench:bots -- --counts=16,32,64 --duration=30
vp run bench:bots -- --counts=64 --layout=dense
vp run bench:bots -- --counts=256,512,1000 --duration=10 --no-browser \
  --service-profile=worldgen --bot-profile=worldgen
```

Use growth mode to retain one temporary database and daemon across successive waves. Stable bot
identities resume their server-owned position, inventory, and edited world:

```sh
vp run bench:bots -- --counts=4,8,16,32,64 --duration=60 --growth
vp run bench:bots -- --counts=16 --duration=600 --growth --no-browser
```

The default includes one real Chromium observer. At population `N`, each native bot should see `N`
other players: `N - 1` bots plus the observer. The observer must see all `N` bots. `--no-browser`
removes rendering/build cost when measuring raw daemon or long-duration database capacity.
`--source=procedural-v16` is the reproducible default; another configured source can be named
explicitly. CPU percentages follow `ps` semantics, so 100% means one fully occupied logical core.

Each stage records:

- daemon and bot-driver CPU, RSS, virtual memory, and thread distributions;
- delivered TCP stream bytes, WebSocket frame bytes, exact VXWP payloads, paths, and message kinds;
- per-client adaptive floor/ceiling and burst payload envelopes, p95/max rates, ceiling violations,
  queue-delay targets, and bandwidth split among presence, edits, and visible world products;
- ping, chunk, surface, and edit latency distributions;
- connected/visible players, pose traffic, edit acceptance, mutations, copies, resyncs, and errors;
- SQLite main/WAL/SHM bytes over time plus players, inventories, live edits, operation history,
  affected chunks, and affected surfaces;
- browser avatar readiness, interactive/full LOD readiness, final terrain coverage, frame history,
  CPU/GPU timing, WASM memory, GPU memory, and console/WebGPU failures.

The harness fails on missing clients or avatars, unexpected edit rejection, resyncs, protocol
errors, per-client budget violations, or browser errors. Expected authoritative placement conflicts
in a dense shared worksite are counted separately rather than mislabeled as protocol failures. A
partial observer world is reported explicitly rather than allowing a low frame time with less
rendered geometry to look like a performance improvement.

Timestamped JSON and Markdown plus `latest.json`, `latest.md`, and the last observer screenshot are
written to `target/harness/bots/`. Preserve a timestamped JSON file when comparing a change; `latest`
is intentionally replaced by the next run.

## Interpreting world growth

Generated terrain is deterministic derived data and is cached in RAM, not persisted. An explorer can
travel indefinitely without making the database contain every visited chunk. Durable growth comes
from player resumes, per-material inventories, sparse voxel overrides, edit idempotency history, and
the chunk/surface revision index required to stream those edits.

Physical SQLite file growth is bursty because the WAL grows and checkpoints into the main file.
Compare both total bytes and logical row counts. For a long-duration result, the most useful ratios
are bytes per live edited voxel, operation-history rows per accepted action, and bandwidth per
connected player-second. Run growth mode without a browser when the database trend itself is the
experiment.

## Reproducibility rules

- Compare timing only on the same machine, source, profiles, duration, population, browser mode, and
  layout. The JSON records these inputs.
- Keep the observer enabled for client/rendering claims. A native-only run cannot prove avatar or
  WebGPU behavior.
- Use `bench:network` rather than `bench:bots` for WAN claims. The bot link is intentionally
  near-unshaped so it attributes bytes and server capacity without introducing a second bottleneck.
- Use the strict six-browser tower test for collaborative far-LOD claims. Bot commits prove protocol
  and state pressure, not rendered edit silhouettes.
- Never point a harness at the normal development database. The checked-in runners own and remove
  their temporary paths.
