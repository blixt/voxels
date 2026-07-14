# Voxels

A Rust-first voxel world rendered with WGPU/WebGPU. The browser main thread captures input and
transfers an `OffscreenCanvas`; a dedicated Rust/WASM worker runs simulation, meshing, rendering,
streaming, and persistence while the native Rust world service owns generation.

## Workflow

1. Install the Vite+ CLI from <https://viteplus.dev/> and Rust through
   [rustup](https://rustup.rs/).
2. Install the CLI version paired with the pinned Rust library using
   `cargo install --locked wasm-bindgen-cli --version 0.2.117`.
3. Install managed Node, pnpm, and project dependencies with `vp install`.
4. Start `vp run world:serve` in one terminal and `vp dev` in another.
5. Run TypeScript checks and tests with `vp check` and `vp test`.
6. Run host Rust tests and host/WASM Clippy with `vp run check:rust`.
7. Build production WASM and assets with `vp build`; inspect them with `vp preview`.
8. Stress isolated rapid-refresh and multi-tab OPFS handoff in Chrome with
   `vp run test:persistence-browser`.
9. Force a complete stale-lease retry cycle with `vp run test:persistence-recovery`.
10. With the world service running, launch two named release clients and verify remote-avatar
    streaming, rendering, and movement in real Chrome with `vp run test:multiplayer-browser`.

`vp run verify` runs the complete TypeScript, Rust, test, lint, and production-build gate.
The browser persistence stress test remains explicit because it launches a system Chrome instance.
`vp run bench:world` runs native Criterion baselines for chunk generation, VXCH encode/decode, and
greedy meshing. Reports are written under the ignored `target/criterion/` directory.
`vp run bench:core` compares 120 dry and submerged fixed simulation steps.
`vp run terrain:fetch`, `vp run terrain:counterproof`, `vp run terrain:smoke`,
`vp run terrain:base`, and `vp run terrain:detail` exercise the optional native Rust/Metal Terrain
Diffusion provider on Apple silicon; see
[the provider notes](docs/terrain-diffusion-metal.md).
The server-owned [world source configuration](docs/world-service-config.md) selects procedural or
Terrain Diffusion generation for the native service bootstrap without exposing the provider to
clients. `vp run world:source-smoke` loads that TOML configuration and verifies the selected source
with one macro-field request. `vp run world:serve` starts the procedural-capable loopback daemon;
`vp run world:serve-metal` enables the native Metal Terrain Diffusion provider selected by the same
file. The browser always consumes the daemon; switch experiences only by changing the server's
`source` value and restarting it. See
[Native world streaming](docs/native-world-streaming.md) for the matching endpoint/token settings,
Chrome local-network permission, binary VXWP protocol, transport rationale, and exact run steps.
Client runtime, streaming, rendering/Mission Control, persistence, diagnostics, and profiling
defaults live in `config/client.toml`. See [Configuration](docs/configuration.md) for ownership,
deployment, validation, and testing conventions for both files.
Opening the bare development URL reuses the browser's default local player and last position. Open
`/?player=alice` and `/?player=bob` in two windows to test distinct simultaneous players against the
same daemon; see [Local players](docs/native-world-streaming.md#local-players-and-two-browser-testing).
Each client renders the others as unique saturated-color articulated voxel figures with distance-
driven gait, independent head look, body-follow hysteresis, shadows, depth, and contact AO.
`vp run profile:browser` builds release WASM, serves it from an isolated origin, and records raw
frame/CPU phase distributions for steady, traversal, and underwater scenarios in system Chrome.
`vp run profile:sustained` drives a Rust-owned 1.08 km fixed-step closed rail. One lap warms the exact
terrain used by two measured laps, making its arena-plateau gate prove allocation reuse before every
canonical and LOD queue must drain.
`vp run profile:materials` toggles Rust-generated material detail through canvas hit-testing and gates
identical geometry/residency plus the incremental frame and GPU cost.
`vp run profile:atmosphere` drives the Rust canvas context menu through dawn, clear day, golden hour,
and blue hour, captures each phase, and gates identical geometry plus frame/GPU budgets.
`vp run profile:gtao` follows the Rust-owned pilgrim route to a dense badlands view, toggles
screen-space contact AO through canvas hit-testing, captures the A/B, and gates exact geometry,
resource residency, frame pacing, and depth-plus-AO GPU cost.
`vp run profile:heroes` visits the six semantic regional heroes through the Rust canvas context menu,
captures a deterministic contact sheet, and gates settled streaming, depth ownership, frame pacing,
and GPU cost in every region.
`vp run profile:caves` tours the Cinder Vault approach, descent, and chamber and gates streaming,
eye adaptation, enclosure probes, frame pacing, and GPU cost at every stop.
`vp run profile:lights` isolates bounded voxel-emissive lighting from the cave headlamp, captures the
Rust-toggle A/B, and gates active-light count, unchanged geometry/residency, frame pacing, and GPU cost.
`vp run profile:portals` moves to the playable terrain directly above Cinder Vault and proves that its
resident chamber lights are rejected by Rust's edit-aware portal geodesic rather than leaking through
the sealed shell.
`vp run profile:portal-edits` seals the vault mouth with 25 durable 10 cm basalt edits, observes the
derived closure in a second isolated tab, reloads it from SQLite/OPFS, then restores and reloads the
pristine opening.
`vp run profile:portal-streaming` traverses open and sealed Cinder components and gates conservative
portal-directed chunk look-ahead, exact activation ownership, the 320-chunk ceiling, zero truncation,
and zero unreachable active geometry.
Recorded decision baselines and their test hardware live in [docs/performance.md](docs/performance.md).

## Architecture

- `client-config/`: strict, versioned, host-testable client TOML schema and validation.
- `core/`: portable player, input, physics, and game simulation.
- `world/`: deterministic chunks, generation, versioned transport codecs, surface LODs, and meshing.
- `runtime/`: deterministic streaming priorities, budgets, revision tickets, eviction, and diagnostics.
- `render/`: web-free WGPU resources, pipelines, shaders, and frame rendering.
- `shell/`: WASM/browser worker leaf, packed input decoding, display clock, and persistence seam.
- `world-service/`: bounded multi-client native server and source-neutral provider bootstrap.
- `world-terrain-diffusion/`: optional native Rust/Metal learned macro-terrain provider.
- `web/`: the single body canvas, normalized input transport, pointer lock, and worker boot only. All
  visible HUD, status, crosshair, controls, menus, and text are composed by Rust/WGPU.

The canonical world is generator identity plus sparse edits. Near meshes and four streamed surface LOD
rings are derived caches; the rings sample at 0.2, 0.4, 0.8, and 1.6 m while the editable near field
retains authoritative 10 cm voxels. The current generator shares one regional surface sample across canonical
chunks and every LOD, blending forest, moor, alpine, badlands, dune, and volcanic terrain influences.
Grid-aligned Rust draw ownership selects whole surface patches, closes resolution boundaries with
conditional skirts, and activates a newly streamed coverage set only when it is complete.
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
destination remain editable, collidable, and continuous through every streamed LOD rather than
becoming renderer-only decals. Mission Control derives chapter and progress status from the Rust atlas.
See [docs/architecture.md](docs/architecture.md) for format, persistence, and research decisions.

## Controls

- Click the world to capture the pointer.
- Move with <kbd>W</kbd><kbd>A</kbd><kbd>S</kbd><kbd>D</kbd>.
- Jump with <kbd>Space</kbd>; hold <kbd>Shift</kbd> to sprint.
- In deep water, use <kbd>W</kbd><kbd>A</kbd><kbd>S</kbd><kbd>D</kbd> to swim along the view direction,
  <kbd>Space</kbd> to ascend, and <kbd>Shift</kbd> to dive.
- Look with the mouse; <kbd>Esc</kbd> releases pointer lock.
- Remove the targeted voxel with the left mouse button; place the Rust-selected material with the
  right mouse button. Cycle Grass, Stone, Basalt, and Glow Crystal from the Mission Control context
  menu. Sparse edits are saved transactionally in SQLite on OPFS.
- Press <kbd>F3</kbd> for the Rust-rendered Mission Control panel. Its live counters and context menu
  can toggle cascaded sun shadows, voxel AO, screen-space contact AO, fog, far terrain, animated
  water, target highlighting, material surface detail, or voxel emissive lights without a DOM UI
  layer. Its Rust-rendered
  more menu can teleport to the coastal showcase, follow the 26 marks of the pilgrim road, or cycle
  through regional landmarks and regional daylight phases for repeatable graphics and streaming
  checks.

## Automation

`window.__VOXELS__.snapshot()` returns a versioned numeric telemetry stream for browser automation.
It contains camera, renderer, streaming, memory, water, GPU, sustained-profile, frame-history, and
edit-convergence records. It is intentionally an automation hook rather than a second JavaScript game
model; the browser owns no simulation or UI state.
