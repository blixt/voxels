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
vp run automation -- run bot-load --counts=4 --duration=3 --service-profile=worldgen-dev --no-browser
vp run automation -- run multiplayer

# Server capacity, dense edits, presence, or avatar rendering
vp run automation -- run bot-load

# Streaming, scheduling, compression, or remote-link change
vp run automation -- run network-benchmark
```

`vp run verify` is the complete static and build gate: TypeScript checks, TypeScript tests, host Rust
tests, host/WASM Clippy, and the production browser build. The specialized harnesses below provide
behavioral, visual, resource, or transport evidence that the general gate cannot.

## Test surfaces

| Area                | Scenario command                                                                                        | What it proves                                                                               |
| ------------------- | ------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------- |
| TypeScript          | `vp check`, `vp test`                                                                                   | Formatting, lint, types, and unit contracts                                                  |
| Rust and WASM       | `vp run check:rust`                                                                                     | Workspace tests plus host and WASM Clippy                                                    |
| Production build    | `vp build`                                                                                              | Optimized WASM, shaders, and web assets compile                                              |
| Native bot smoke    | `vp run automation -- run bot-load --counts=4 --duration=3 --service-profile=worldgen-dev --no-browser` | Four real VXWP clients connect, move, stream, edit, and replicate                            |
| Bot population/load | `vp run automation -- run bot-load`                                                                     | Mixed bots, process resources, disk, wire bytes, and a real browser observer                 |
| Six browser users   | `vp run automation -- run multiplayer`                                                                  | Isolated identities, shaped WAN links, avatars, collaborative edits, and far-LOD convergence |
| Remote streaming    | `vp run automation -- run network-benchmark`                                                            | Cold spawn, walk, turn, viewport readiness, full coverage, and exact link bytes              |
| Network comparison  | `vp run automation -- run network-compare before.json after.json`                                       | Compatible before/after timing and byte deltas                                               |
| LOD transition      | `vp run automation -- run lod-transition`                                                               | Same-pose real-browser handoff and image evidence                                            |
| Watertight LOD      | `vp run automation -- run lod-transition --mode=watertight`                                             | Strict seam/coverage regression path                                                         |
| Terrain boundary    | `vp run automation -- run lod-transition --mode=boundary-coverage`                                      | Reported vertical boundary pose, sky exposure, and exact browser scale                       |
| LOD video           | `vp run automation -- run lod-transition --video`                                                       | The same validated traversal captured as raw WebM                                            |
| World Lab/UI        | `vp run automation -- run world-lab`                                                                    | Rust UI interaction and synchronized world diagnostics                                       |
| Spectator feed      | `vp run automation -- run spectator-feed`                                                               | Bodyless read-only camera, movement, body restore, screenshots, and video                    |
| Weather motion      | `vp run automation -- run weather-motion`                                                               | World-anchored clouds and downward precipitation                                             |
| Renderer profile    | `vp run automation -- run render-profile`                                                               | Release frame/CPU/GPU distributions in Chrome                                                |
| Chromium trace      | `vp run automation -- run render-profile --trace`                                                       | Renderer profile plus a standalone CDP trace                                                 |
| Sustained traversal | `vp run automation -- run render-profile --mode=sustained`                                              | Warmed rail, allocation plateau, streaming, and frame pacing                                 |
| Material cost       | `vp run automation -- run render-profile --mode=materials`                                              | Geometry-invariant material-detail A/B                                                       |
| Day/night cost      | `vp run automation -- run render-profile --mode=atmosphere`                                             | Synchronized celestial anchors and lighting budgets                                          |
| Weather cost        | `vp run automation -- run render-profile --mode=weather`                                                | Weather anchors, geometry invariants, clouds, rain, and GPU budgets                          |
| Portable world      | `vp run automation -- run bench-world`                                                                  | Generation, VXCH codecs, and meshing Criterion baselines                                     |
| Portable simulation | `vp run automation -- run bench-core`                                                                   | Dry/submerged fixed-step simulation baselines                                                |
| Streaming runtime   | `vp run automation -- run bench-runtime`                                                                | Portable runtime scheduling baselines                                                        |

Every scenario writes to `target/automation/<scenario>/<run-id>/`; its
`target/automation/<scenario>/latest.json` points to the last completed run.

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
vp run automation -- run bot-load
vp run automation -- run bot-load --counts=16,32,64 --duration=30
vp run automation -- run bot-load --counts=64 --layout=dense
vp run automation -- run bot-load --counts=16 --duration=10 --video
vp run automation -- run bot-load --counts=256,512,1000 --duration=10 --no-browser \
  --service-profile=worldgen --bot-profile=worldgen
```

Use growth mode to retain one temporary database and daemon across successive waves. Stable bot
identities resume their server-owned position, inventory, and edited world:

```sh
vp run automation -- run bot-load --counts=4,8,16,32,64 --duration=60 --growth
vp run automation -- run bot-load --counts=16 --duration=600 --growth --no-browser
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

Each run records JSON, Markdown, process samples, link accounting, and the observer screenshot in its
artifact directory. Preserve a run directory when comparing a change; the per-scenario `latest.json`
pointer is intentionally replaced by the next run.

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
- Use `network-benchmark` rather than `bot-load` for WAN claims. The bot link is intentionally
  near-unshaped so it attributes bytes and server capacity without introducing a second bottleneck.
- Use the strict six-browser tower test for collaborative far-LOD claims. Bot commits prove protocol
  and state pressure, not rendered edit silhouettes.
- Never point a harness at the normal development database. The checked-in runners own and remove
  their temporary paths.
