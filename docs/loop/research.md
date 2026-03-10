# Research Notes

Date: 2026-03-10

## Target platform

- Local browser target: Chrome 146.0.7680.72 on macOS.
- Project scope: latest stable Chrome only, WebGPU only, no rendering libraries.

## Current external facts

- Chrome desktop stable 146.0.7680.72/.73 rolled out on March 10, 2026.
- Chrome's WebGPU implementation has recent performance work around faster `GPUQueue.writeBuffer()` / `writeTexture()` uploads, which is relevant for chunk mesh uploads and live edits.
- Chrome 145 exposes the WGSL `subgroup_uniformity` language feature when the adapter supports the `subgroups` feature, but this project does not need subgroups for the first implementation.
- WebGPU timestamp queries are quantized to 100 microseconds in Chrome for privacy, which is still sufficient for repeated benchmark runs.
- WebGPU guaranteed limits from the spec are large enough for per-chunk mesh buffers and practical chunked rendering workloads.
- `mise` supports pinning Bun directly in `mise.toml`; TypeScript static checking can be provided by a project-local `typescript` dependency.
- Bun's `Bun.serve()` HTTP API and build pipeline are sufficient for this project without extra web tooling.
- MagicaVoxel's published `.vox` format is the most practical standard voxel interchange format to support first.

## Architecture decision

The first engine revision uses chunked sparse voxel storage plus CPU greedy meshing:

- It is straightforward to keep editable.
- It avoids pushing hidden interior voxels to the GPU.
- It maps well to Bun/TypeScript without native extensions.
- It keeps the benchmark harness focused on meshing, uploads, edits, and frame time.

## Future investigation

- Recover reliable browser automation for the real WebGPU path. The current shell cannot clear the tool-owned Chrome DevTools profile lock, so browser verification is partially blocked outside manual runs.
- Move chunk meshing to a Web Worker if live-edit stalls become visible.
- Evaluate compute-driven culling or GPU-driven meshing after the core renderer is stable.
- Add fuller `.vox` scene graph support if real-world assets demand it.

## Source links

- Chrome Stable channel update for 146.0.7680.72/.73:
  https://chromereleases.googleblog.com/2026/03/stable-channel-update-for-desktop_10.html
- Chrome WebGPU developer features in 144:
  https://developer.chrome.com/blog/new-in-webgpu-144
- Chrome WebGPU developer features in 145:
  https://developer.chrome.com/blog/new-in-webgpu-145
- WebGPU specification and guaranteed limits:
  https://gpuweb.github.io/gpuweb/
- Mise getting started and tool pinning:
  https://mise.jdx.dev/getting-started.html
- Mise npm backend:
  https://mise.jdx.dev/lang/npm.html
- Bun HTTP server API:
  https://bun.sh/docs/api/http
- Bun build API:
  https://bun.sh/docs/bundler
- MagicaVoxel format reference:
  https://github.com/ephtracy/voxel-model/blob/master/MagicaVoxel-file-format-vox.txt
- MagicaVoxel scene graph extension reference:
  https://github.com/ephtracy/voxel-model/blob/master/ext/vox-extension.txt
