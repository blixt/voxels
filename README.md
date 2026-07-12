# Voxels

A Rust-first procedural voxel world rendered with WGPU/WebGPU. The browser main thread captures input
and transfers an `OffscreenCanvas`; a dedicated worker runs the Rust/WASM simulation, generation,
greedy meshing, rendering, and persistence boundary.

## Workflow

1. Install the Vite+ CLI from <https://viteplus.dev/>.
2. Install managed Node, pnpm, and project dependencies with `vp install`.
3. Start the app with `vp dev`.
4. Run TypeScript checks and tests with `vp check` and `vp test`.
5. Run host/WASM Rust tests and Clippy with `vp run check:rust`.
6. Build production WASM and assets with `vp build`; inspect them with `vp preview`.
7. Stress isolated rapid-refresh and multi-tab OPFS handoff in Chrome with
   `vp run test:persistence-browser`.

`vp run verify` runs the complete TypeScript, Rust, test, lint, and production-build gate.
The browser persistence stress test remains explicit because it launches a system Chrome instance.
`vp run bench:world` runs native Criterion baselines for chunk generation, VXCH encode/decode, and
greedy meshing. Reports are written under the ignored `target/criterion/` directory.
`vp run bench:core` compares 120 dry and submerged fixed simulation steps.
`vp run profile:browser` builds release WASM, serves it from an isolated origin, and records raw
frame/CPU phase distributions for steady, traversal, and underwater scenarios in system Chrome.
`vp run profile:sustained` drives a Rust-owned 1.08 km fixed-step closed rail. One lap warms the exact
terrain used by two measured laps, making its arena-plateau gate prove allocation reuse before every
canonical and LOD queue must drain.
`vp run profile:edits` runs 40 Rust-owned terrain/water remove-and-restore operations and gates SQLite
dispatch, canonical remesh, all four LOD replacements, submitted-frame convergence, and pristine
restoration.
Recorded decision baselines and their test hardware live in [docs/performance.md](docs/performance.md).

## Architecture

- `core/`: portable player, input, physics, and game simulation.
- `world/`: deterministic chunks, generation, palette/bit-packed codecs, and greedy meshing.
- `runtime/`: deterministic streaming priorities, budgets, revision tickets, eviction, and diagnostics.
- `render/`: web-free WGPU resources, pipelines, shaders, and frame rendering.
- `shell/`: WASM/browser worker leaf, packed input decoding, display clock, and persistence seam.
- `web/`: the single canvas, normalized input transport, pointer lock, and worker boot only. All visible
  HUD, status, crosshair, controls, menus, and text are composed by Rust/WGPU.

The canonical world is generator identity plus sparse edits. Near meshes and four streamed surface LOD
rings are derived caches; the rings sample at 0.2, 0.4, 0.8, and 1.6 m while the editable near field
retains authoritative 10 cm voxels. Generator v7 shares one regional surface sample across canonical
chunks and every LOD, blending forest, moor, alpine, badlands, dune, and volcanic terrain influences.
Grid-aligned Rust draw ownership selects whole surface patches, closes resolution boundaries with
conditional skirts, and activates a newly streamed coverage set only when it is complete.
Analytic landmark identities add broadleaf trees, limestone tors, alpine needles, hoodoos, dune
arches, and basalt columns as ordinary editable voxels, with bounded edit-aware proxies preserving
their silhouettes across the surface rings. A deterministic Rust-generated material atlas adds
world-anchored, mip-filtered albedo, micro-normal, and roughness structure without changing geometry,
draw ownership, or durable voxel data.
See [docs/architecture.md](docs/architecture.md) for format, persistence, and research decisions.

## Controls

- Click the world to capture the pointer.
- Move with <kbd>W</kbd><kbd>A</kbd><kbd>S</kbd><kbd>D</kbd>.
- Jump with <kbd>Space</kbd>; hold <kbd>Shift</kbd> to sprint.
- In deep water, use <kbd>W</kbd><kbd>A</kbd><kbd>S</kbd><kbd>D</kbd> to swim along the view direction,
  <kbd>Space</kbd> to ascend, and <kbd>Shift</kbd> to dive.
- Look with the mouse; <kbd>Esc</kbd> releases pointer lock.
- Remove the targeted voxel with the left mouse button; place a grass voxel with the right mouse
  button. Sparse edits are saved transactionally in SQLite on OPFS.
- Press <kbd>F3</kbd> for the Rust-rendered Mission Control panel. Its live counters and context menu
  can toggle cascaded sun shadows, ambient occlusion, fog, far terrain, animated water, and target
  highlighting or material surface detail without a DOM UI layer. Its Rust-rendered more menu can
  teleport to the coastal showcase or cycle through regional landmarks for repeatable graphics and
  streaming checks.

## Automation

`window.__VOXELS__.snapshot()` returns a versioned numeric telemetry stream for browser automation.
It contains camera, renderer, streaming, memory, water, GPU, sustained-profile, frame-history, and
edit-convergence records. It is intentionally an automation hook rather than a second JavaScript game
model; the browser owns no simulation or UI state.
