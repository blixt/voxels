# Plan

## Current target

Improve Chrome 146 voxel-engine performance with repeatable profiling and correctness guardrails:

- keep Bun as the server, bundler entrypoint, and test runner
- keep `/` as the editable playground
- keep `/bench` as the repeatable benchmark runner
- separate tiny validation scenes from large performance scenes
- keep the 256x256x256 default scene and live-edit workflow healthy
- prefer measured rewrites of hot paths over additive complexity

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
10. Added stress scenes and drag-behavior regression coverage.
11. Reduced mesher cost with chunk-local sampling, packed face masks, and flat quad records.
12. Reduced terrain-scene build cost with chunk-aware vertical bulk writes.
13. Expanded the benchmark harness with first-frame vs warm-frame metrics and a repeatable local profiling script.

## Next steps

- Restore fully automatic browser-side GPU verification once the tool-owned Chrome profile lock is cleared.
- Use the new first-frame metrics to profile GPU upload and resource-sync cost on Chrome 146, then decide whether buffer reuse and `queue.writeBuffer()` are worth keeping.
- Turn the stress scenes into explicit regression targets with per-scene warm-frame and first-frame baseline ranges.
- Consider a worker-based meshing path only after the current single-thread mesher stops producing worthwhile wins.
- Add fuller MagicaVoxel scene graph support, especially rotation decoding.
- Experiment with GPU-driven culling once the current CPU meshing path is profiled more deeply.
