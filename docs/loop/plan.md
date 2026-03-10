# Plan

## Current target

Stabilize the Chrome 146 WebGPU voxel engine by tightening primitive-level validation and browser-side correctness checks:

- keep Bun as the server, bundler entrypoint, and test runner
- keep `/` as the editable playground
- keep `/bench` as the repeatable benchmark runner
- separate tiny validation scenes from large performance scenes
- keep the 256x256x256 default scene and live-edit workflow healthy

## Completed sequence

1. Established Bun + TypeScript + Mise project wiring.
2. Built chunked storage, scene generators, and persistence.
3. Implemented greedy meshing and GPU upload lifecycle.
4. Rendered the world with WebGPU and basic lighting.
5. Added interactive editing and import/export tools.
6. Added benchmark automation and correctness checks.
7. Diagnosed the first major renderer artifact with a hypothesis grid instead of ad-hoc tweaks.
8. Fixed the mesher bug where stale loop coordinates displaced emitted quads outside their voxel bounds.
9. Added primitive guardrails: single-voxel mesh bounds, depth-order tests, and tiny validation scenes.

## Next steps

- Turn the new stress scenes into explicit regression targets with per-scene baseline notes and acceptable ranges.
- Restore fully automatic browser-side GPU verification once the tool-owned Chrome profile lock is cleared.
- Record fresh `/bench` results for the tiny validation scenes, baseline scenes, and the new stress suite after larger renderer changes.
- Move chunk meshing into a Web Worker to reduce main-thread stalls during heavy edits.
- Add fuller MagicaVoxel scene graph support, especially rotation decoding.
- Experiment with GPU-driven culling once the current CPU meshing path is profiled more deeply.
