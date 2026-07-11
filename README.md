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

`vp run verify` runs the complete TypeScript, Rust, test, lint, and production-build gate.
`vp run bench:world` runs native Criterion baselines for chunk generation, VXCH encode/decode, and
greedy meshing. Reports are written under the ignored `target/criterion/` directory.

## Architecture

- `core/`: portable player, input, physics, and game simulation.
- `world/`: deterministic chunks, generation, palette/bit-packed codecs, and greedy meshing.
- `runtime/`: deterministic streaming priorities, budgets, revision tickets, eviction, and diagnostics.
- `render/`: web-free WGPU resources, pipelines, shaders, and frame rendering.
- `shell/`: WASM/browser worker leaf, packed input decoding, display clock, and persistence seam.
- `web/`: the single canvas, normalized input transport, pointer lock, and worker boot only. All visible
  HUD, status, crosshair, controls, menus, and text are composed by Rust/WGPU.

The canonical world is generator identity plus sparse edits. Meshes and future LODs are derived caches.
See [docs/architecture.md](docs/architecture.md) for format, persistence, and research decisions.

## Controls

- Click the world to capture the pointer.
- Move with <kbd>W</kbd><kbd>A</kbd><kbd>S</kbd><kbd>D</kbd>.
- Jump with <kbd>Space</kbd>; hold <kbd>Shift</kbd> to sprint.
- Look with the mouse; <kbd>Esc</kbd> releases pointer lock.
- Remove the targeted voxel with the left mouse button; place a grass voxel with the right mouse
  button. Sparse edits are saved transactionally in SQLite on OPFS.
- Press <kbd>F3</kbd> for the Rust-rendered Mission Control panel. Its live counters and context menu
  can toggle cascaded sun shadows, ambient occlusion, fog, far terrain, and target highlighting
  without a DOM UI layer.

## Automation

`window.__VOXELS__.snapshot()` returns a promise containing camera position, yaw, pitch, grounded
state, resident greedy-quad count, persisted edit count, resident chunk count, and tracked chunk
count, followed by visible chunks, draw calls, arena pages, allocated/capacity MiB, and queued stream
work and far-tile residency. It is intentionally an automation hook rather than a second JavaScript
game model. The last three values are exponentially smoothed display-frame cadence in milliseconds,
shadow-caster draw calls, and active shadow-cascade count.
