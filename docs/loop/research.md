# Research Notes

Date: 2026-03-11

## Target platform

- Local browser target: Chrome stable 146 on macOS.
- Project scope: latest stable Chrome only, WebGPU only, no rendering libraries.
- Tooling baseline: `mise`, Bun, project-local TypeScript, and project-local WebGPU types.

## Current external facts

- Chrome desktop stable `146.0.7680.72/.73` rolled out on March 10, 2026.
- WebGPU in Chrome is the correct baseline for this repo, but browser-oriented constraints still matter:
  - no standard browser hardware ray tracing path
  - explicit GPU memory/resource management
  - worker-friendly APIs exist, but some advanced shared-memory workflows require cross-origin isolation
- Pointer Lock is the correct browser primitive for a full-screen first-person game on `/`.
- The Origin Private File System is the strongest local browser persistence candidate for chunk and region payloads because it provides private per-origin file storage and worker-friendly access patterns.
- IndexedDB is still relevant for metadata/index layers even if chunk payloads move to OPFS later.
- Web Workers should own generation, meshing, decode, and streaming work before the main thread becomes a bottleneck.
- OffscreenCanvas remains a candidate if render-thread separation becomes valuable, but it is not required for the first game slice.
- WebSocket is the simplest initial multiplayer transport; WebTransport is worth revisiting later for lower-level transport control once the replication model is clearer.
- SharedArrayBuffer-based worker strategies remain attractive for large-scale streaming and meshing, but they imply cross-origin isolation requirements that the dev server/harness must deliberately support before adopting them.
- Bun's `Bun.serve()` and bundling story are still sufficient for the current browser game + benchmark harness setup.

## Architectural implications

- The old bounded-scene "playground" framing should be treated as a completed phase, not as the long-term product shape.
- The next runtime split should isolate the game path from the existing orbit/benchmark controller stack.
- The authoritative world model should grow toward:
  - sparse chunk coordinates in world space
  - generated base terrain
  - edit overlays
  - local persistence
  - eventual remote authority
- `/bench` should remain the deterministic validation and performance surface even as `/` becomes a game.
- Correctness coverage must expand beyond rendering primitives into:
  - first-person camera invariants
  - picking
  - inventory rules
  - stream ordering
  - generation determinism

## Open technical questions

- What is the cleanest local storage split between OPFS payload files and IndexedDB metadata?
- How early should the server expose cross-origin-isolated dev modes so worker/shared-memory experiments are possible?
- Which stream/residency metrics should be built into `/bench` before lazy infinite-world work starts?
- At what point does far-distance rendering need LOD rather than just chunk culling?
- When multiplayer begins, does the first server protocol stay entirely on WebSocket, or is there a clear early win from WebTransport?

## Immediate implementation direction

- Replace the `/` UI shell with a full-screen game shell.
- Add a dedicated game controller and first-person camera path instead of mutating the orbit playground controller into a hybrid.
- Keep the current renderer and benchmark harness stable while the new game path is established.
- Add automation-friendly game debug surfaces early so future interaction and streaming work can be verified without human-only inspection.

## Source links

- Chrome Stable channel update for `146.0.7680.72/.73`:
  https://chromereleases.googleblog.com/2026/03/stable-channel-update-for-desktop_10.html
- Chrome WebGPU overview:
  https://developer.chrome.com/docs/web-platform/webgpu/overview
- Pointer Lock API:
  https://developer.mozilla.org/en-US/docs/Web/API/Pointer_Lock_API
- Origin Private File System:
  https://web.dev/origin-private-file-system/
- IndexedDB API:
  https://developer.mozilla.org/en-US/docs/Web/API/IndexedDB_API
- Web Workers API:
  https://developer.mozilla.org/en-US/docs/Web/API/Web_Workers_API
- OffscreenCanvas:
  https://developer.mozilla.org/en-US/docs/Web/API/OffscreenCanvas
- WebSocket API:
  https://developer.mozilla.org/en-US/docs/Web/API/WebSocket
- WebTransport:
  https://developer.mozilla.org/en-US/docs/Web/API/WebTransport_API
- SharedArrayBuffer security requirements:
  https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/SharedArrayBuffer
- Mise getting started and tool pinning:
  https://mise.jdx.dev/getting-started.html
- Bun HTTP server API:
  https://bun.sh/docs/api/http
- Bun build API:
  https://bun.sh/docs/bundler
