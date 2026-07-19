# Voxels

A Rust-first voxel world rendered with WGPU/WebGPU. The browser main thread captures input and
transfers an `OffscreenCanvas`; a dedicated Rust/WASM worker runs simulation, meshing, rendering,
and streaming while the native Rust world service owns generation and durable world/player state.

## Workflow

1. Install the Vite+ CLI from <https://viteplus.dev/> and Rust through
   [rustup](https://rustup.rs/).
2. Install the CLI version paired with the pinned Rust library using
   `cargo install --locked wasm-bindgen-cli --version 0.2.117`.
3. Install managed Node, pnpm, and project dependencies with `vp install`.
4. Run `vp dev`; it owns both the native world service and browser development server.
5. Run TypeScript checks and tests with `vp check` and `vp test`.
6. Run host Rust tests and host/WASM Clippy with `vp run check:rust`.
7. Build production WASM and assets with `vp build`; inspect them with `vp preview`.
8. Launch six isolated release clients with shaped links and verify avatars plus a five-builder
   far-LOD tower in real Chrome with `vp run automation -- run multiplayer`.

`vp run verify` runs the complete TypeScript, Rust, test, lint, and production-build gate.
`vp run automation -- list` is the single index for behavioral tests, benchmarks, bot loads,
screenshots, traces, video, and provider setup. Every run is isolated under `target/automation/`;
see [Automation scenarios](docs/automation.md) for the authoring API and composition model.
`vp run automation -- run bench-world` runs native Criterion baselines for chunk generation, VXCH
encode/decode, and greedy meshing. Reports are written under the ignored `target/criterion/`
directory.
`vp run automation -- run bench-core` compares 120 dry and submerged fixed simulation steps.
`vp run automation -- run network-benchmark` owns a temporary native world service, deterministic
bidirectional WAN shaper, release preview, and isolated Chrome contexts; it reports
viewport/full-coverage settling, stream bytes, message attribution, and offline compression
headroom. Compare two JSON artifacts with
`vp run automation -- run network-compare <baseline.json> <candidate.json>`; see
[Remote world streaming benchmarks](docs/network-benchmark.md).
The following is a fast four-client native multiplayer smoke:

```sh
vp run automation -- run bot-load --counts=4 --duration=3 \
  --service-profile=worldgen-dev --no-browser
```

`vp run automation -- run bot-load` drives 4/8/16/32/64 protocol-faithful explorers, diggers,
builders, and followers while measuring server and driver CPU/RAM, exact wire traffic, SQLite
growth, edit latency, and one real Chromium observer.
All verification and profiling entry points are indexed in the canonical
[testing and performance map](docs/testing.md); the initial bot capacity results are in the
[2026-07-17 load report](docs/20260717-bot-load-report.md).
`vp run automation -- run terrain-fetch` and the `terrain-diffusion` scenario's `full`,
`counterproof`, `base`, `detail`, and `survey` modes exercise the optional native Rust/Metal Terrain
Diffusion provider on Apple silicon; see [the provider notes](docs/terrain-diffusion-metal.md).
The server-owned [world source configuration](docs/world-service-config.md) defaults to native
Terrain Diffusion/Metal generation and can select the procedural source without exposing that choice
to clients. `vp run automation -- run world-source` loads the TOML and verifies the selected source
with one macro-field request. Its `ecology-survey` mode prints a 3.2 km deterministic forest-density
map, tree-count percentiles, and the densest spawn coordinates. `vp dev` starts and stops the
Metal-capable daemon with Vite. Either source can be selected by changing only the server's `source`
value and restarting development. The browser always consumes the same canonical protocol. See
[Native world streaming](docs/native-world-streaming.md) for the matching endpoint/token settings,
Chrome local-network permission, binary VXWP protocol, transport rationale, and exact run steps.
Client runtime, streaming, rendering/World Lab, diagnostics, and profiling
defaults live in `config/client.toml`. See [Configuration](docs/configuration.md) for ownership,
deployment, validation, and testing conventions for both files.
Opening the bare development URL reuses the browser's default local player and last position. Open
`/?player=alice` and `/?player=bob` in two windows to test distinct simultaneous players against the
same daemon; see [Local players](docs/native-world-streaming.md#local-players-and-two-browser-testing).
Each client renders the others as unique saturated-color articulated voxel figures with distance-
driven gait, independent head look, body-follow hysteresis, shadows, depth, and contact AO.
`vp run automation -- run render-profile` builds release WASM, serves it from an isolated origin,
and records raw frame/CPU phase distributions for steady and traversal scenarios in system Chrome.
Provider-specific coast, route, landmark, and cave tours remain unavailable until the world protocol
advertises those queries and authored locations.
`vp run automation -- run render-profile --trace` runs the same isolated headless Chromium workload
and also writes a CDP performance trace into its artifact directory. A reproducible fixed pose can be
expressed entirely in the scenario file or CLI:

```sh
vp run automation -- run render-profile --mode=stationary --build=wasm-dev \
  --source=terrain-diffusion-30m --dpr=2 --spawn=-12800,25600 --look=2.07,-0.37 \
  --shadows=off --ssao=off --screenshot
```

`render-profile` also has `sustained`, `materials`, `atmosphere`, and `weather` modes for the warmed
1.08 km traversal, material A/B, celestial anchors, and synchronized weather budgets respectively.
`vp run automation -- run weather-motion` verifies that projected rain moves downward and that cloud
lighting/density remain stable when the camera rotates away and returns.
Recorded decision baselines and their test hardware live in [docs/performance.md](docs/performance.md).

## Architecture

- `automation/`: typed TypeScript scenario API, reusable capabilities, and all capture, benchmark,
  bot, browser-validation, video, screenshot, trace, and analysis scenarios.
- `client-config/`: strict, versioned, host-testable client TOML schema and validation.
- `core/`: portable player, input, physics, and game simulation.
- `world/`: deterministic chunks, generation, versioned transport codecs, surface LODs, and meshing.
- `runtime/`: deterministic streaming priorities, budgets, revision tickets, eviction, and diagnostics.
- `render/`: web-free WGPU resources, pipelines, shaders, and frame rendering.
- `shell/`: WASM/browser worker leaf, packed input decoding, display clock, and remote clients.
- `world-service/`: bounded multi-client native server and source-neutral provider bootstrap.
- `world-terrain-diffusion/`: optional native Rust/Metal learned macro-terrain provider.
- `web/`: the single body canvas, normalized input transport, pointer lock, and worker boot only. All
  visible HUD, status, crosshair, controls, menus, and text are composed by Rust/WGPU.

The canonical world is generator identity plus sparse edits. Near meshes and six streamed surface LOD
rings are derived caches; the rings sample at 0.2, 0.4, 0.8, 1.6, 3.2, and 6.4 m while the editable
near field retains authoritative 10 cm voxels. The two horizon-only rings are capped background
prefetches and extend default coverage to 1 km without delaying the four interactive rings. The current
generator shares one regional surface sample across canonical
chunks and every LOD, blending forest, moor, alpine, badlands, dune, and volcanic terrain influences.
Grid-aligned Rust draw ownership selects whole surface patches, closes resolution boundaries with
exact connectors built from the resident coarse and fine height profiles, and activates a newly
streamed coverage set only when it is complete.
Analytic landmark identities add ordinary regional forms plus elder canopies, tor circles, needle
gates, buried ribs, colonnades, and basalt crowns as ordinary editable voxels. An 8x8-cell composition
director arranges backgrounds and companions into clusters, rings, clearings, and procession lines,
then gives each macro cell one distinct semantic hero. Bounded edit-aware proxies preserve every form
across the surface rings. A deterministic Rust-generated
material atlas adds world-anchored pixel albedo, micro-normal, and roughness structure quantized to an
exact 3x3 grid on every 10 cm voxel face (one block per 3.33 cm), with nearest sampling and a
distance-stable mip chain, without changing geometry, draw ownership, or durable voxel data.
The 753 m First Pilgrim Road is a stable Rust polyline graded through those same canonical columns.
Its 10 cm paving, shoulders, 26 cairns/waystones/arches, five named chapters, and alpine Needle Gate
destination remain editable, collidable, and continuous through every interactive LOD rather than
becoming renderer-only decals. Mission Control derives chapter and progress status from the Rust atlas.
See [docs/architecture.md](docs/architecture.md) for format, authority, and research decisions.

## Controls

- Click the world to capture the pointer.
- Move with <kbd>W</kbd><kbd>A</kbd><kbd>S</kbd><kbd>D</kbd>.
- Jump with <kbd>Space</kbd>; after releasing it, tap <kbd>Space</kbd> again while airborne to
  deploy or retract the glider. Hold <kbd>Shift</kbd> to sprint on foot.
- In deep water, use <kbd>W</kbd><kbd>A</kbd><kbd>S</kbd><kbd>D</kbd> to swim along the view direction,
  <kbd>Space</kbd> to ascend, and <kbd>Shift</kbd> to dive.
- Look with the mouse; <kbd>Esc</kbd> releases pointer lock.
- Excavate a 0.5 m-wide sphere around the targeted voxel with the left mouse button; place the
  Rust-selected material with the right mouse button. Cycle stocked materials with the mouse wheel
  or inventory wheel. Sparse edits are committed transactionally by the native
  world service and streamed to every interested browser.
- Press <kbd>F3</kbd> for the Rust-rendered World Lab. It keeps the useful navigation and performance
  readout, copies a complete diagnostic report, and—when developer controls are enabled—can preview
  time-of-day and weather locally without mutating the shared server clock. Spectator mode leaves
  the authoritative player body in place and switches to a collisionless, bodyless, read-only
  camera; use <kbd>Space</kbd> to rise and <kbd>Shift</kbd> to descend. Leaving spectator mode
  returns to the exact saved body pose. It activates only when the server advertises that capability.
  Renderer policy stays in `config/client.toml` instead of filling the play UI with operational
  toggles.

The shared server clock drives a familiar full year: seasonal sun paths, lunar phases, a rotating
sidereal star field, and subtle deterministic twinkle are identical for every client. World
coordinate `(0, 0)` is the celestial equator; moving `-Z` travels north and `+X` east across a
repeating spherical observer frame while the editable terrain itself remains infinite and unwrapped.

## Automation

`window.__VOXELS__.snapshot()` returns a versioned numeric telemetry stream for browser automation.
It contains camera, renderer, streaming, memory, water, GPU, sustained-profile, frame-history, and
edit-convergence records. It is intentionally an automation hook rather than a second JavaScript game
model; the browser owns no simulation or UI state. `submitEdit(...)` and `surfaceEditState(...)`
exercise the production server-edit path and expose only bounded convergence state for multiplayer
regression tests.

The same typed boundary can operate a product-facing observer without granting edit authority:

```sh
# Start `vp dev`, then attach a temporary spectator camera and record a regional orbit.
vp run automation -- run spectator-feed --url=http://127.0.0.1:5173 --duration=30
```

The capture uses the production Rust camera, streaming, renderer, and presence protocol. It writes
start/end screenshots and WebM video, verifies that edits are unavailable, and returns the parked
body to its exact original pose. See [Automation scenarios](docs/automation.md#spectator-feeds).
