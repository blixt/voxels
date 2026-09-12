# Latest Chrome and wgpu: capability audit

Research date: 2026-09-12. Companion to the
[mutable-world research update](20260912-mutable-voxel-research.md).
This records the source audit and isolated browser probe performed during this research session;
it is not a renderer benchmark or an implemented fork.

## Recommendation

Keep Rust and wgpu. A small, pinned browser-backend patch is reasonable when a measured kernel
needs a Chrome feature that wgpu does not expose. Start with ordinary compute and storage buffers;
the first integrated mutable-world milestone does not require a fork.

The browser backend forwards `ShaderSource::Wgsl` to Chrome's `GPUShaderModuleDescriptor`.
Chrome/Dawn compiles and executes it; wgpu's native Metal backend and Naga translation are not
on that path. Consequently, exposing a browser capability can be much smaller work than adding
portable native compiler/backend support. Host Naga tests do not establish that the shipping
browser shader works, and native wgpu ray-tracing features do not make hardware ray tracing
available in WebGPU.

Inspected versions: wgpu **30.0.1** and upstream commit
[`d6e5619bb72c742b92ce435b62644fd6f1f372d5`](https://github.com/gfx-rs/wgpu/tree/d6e5619bb72c742b92ce435b62644fd6f1f372d5).
Relevant source:
[browser backend](https://github.com/gfx-rs/wgpu/blob/d6e5619bb72c742b92ce435b62644fd6f1f372d5/wgpu/src/backend/webgpu.rs),
[feature definitions](https://github.com/gfx-rs/wgpu/blob/d6e5619bb72c742b92ce435b62644fd6f1f372d5/wgpu-types/src/features.rs),
[language-feature API](https://github.com/gfx-rs/wgpu/blob/d6e5619bb72c742b92ce435b62644fd6f1f372d5/wgpu/src/api/instance.rs).
Recheck upstream state before implementation; the open PR statuses below are a dated snapshot.

## What actually changed in Chrome during 2026

These are platform posts, not voxel-engine performance demonstrations. All were read on
2026-09-12 and are by François Beaufort on Chrome for Developers.

| Publication                                                                          | New capability                     | Relevance and limit                                                                                                                                                                                 |
| ------------------------------------------------------------------------------------ | ---------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [Chrome 146, February 25](https://developer.chrome.com/blog/new-in-webgpu-146)       | Transient attachments              | Pass-local attachments may avoid external-memory traffic, especially on tile GPUs. Contents cannot survive the render pass. Depth sampled by later lighting, water or reconstruction is ineligible. |
| [Chrome 149–150, June 17](https://developer.chrome.com/blog/new-in-webgpu-149-150)   | Immediate data and `setImmediates` | Small per-pass/dispatch constants without another bind group. Detect the WGSL `immediate_address_space` extension; this is not a WebGPU required-feature string called `immediates`.                |
| [Chrome 151–152, August 12](https://developer.chrome.com/blog/new-in-webgpu-151-152) | Subgroup size control              | Optional `subgroup-size-control`, WGSL `enable subgroup_size_control`, and `@subgroup_size(N)`. Useful only for kernels and adapters that benefit from a particular supported size.                 |

An isolated headless probe of installed **Chrome 153.0.8010.37**, using the Apple Metal adapter
(`isFallbackAdapter = false`), advertised subgroups, subgroup size control, `shader-f16`,
timestamps, `primitive-index`, transient attachment usage, and a 64-byte immediate-data limit.
It exposed `setImmediates`; `multiDrawIndirect` was absent. Its subgroup minimum and maximum
were both **32**. These are feature-advertisement/API observations, not successful execution
tests of each feature. They apply to that executable and adapter, not every Chrome installation.

The same probe advertised WGSL `buffer_view`. Typed views could simplify packed GPU storage,
but explicit decoding from `u32` arenas remains a working baseline. Do not make the renderer
depend on this convenience before its actual shader path is tested.

## Exposure gaps and upstream signals

| Feature                    | Released browser-backend finding                                                                   | Upstream evidence as of September 12                                                                                                                                                                                                                             | Decision                                                                                                |
| -------------------------- | -------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------- |
| Basic subgroups            | Existing wgpu bit and generated browser enum, but missing feature mapping                          | [Issue 10146](https://github.com/gfx-rs/wgpu/issues/10146), [PR 10220](https://github.com/gfx-rs/wgpu/pull/10220): one-file mapping change; open with review concerns about native/spec differences                                                              | First narrow patch candidate when scans, compaction or traversal benefit.                               |
| Subgroup size control      | Missing public/browser exposure despite generated browser enum                                     | [Issue 10049](https://github.com/gfx-rs/wgpu/issues/10049); [PR 9523](https://github.com/gfx-rs/wgpu/pull/9523) is a draft native Vulkan passthrough effort, not a browser implementation                                                                        | Add a browser bridge only when needed. Fixed 32-lane hardware offers little size-selection opportunity. |
| Immediates                 | Reported limit remains zero, explicit layout field is not forwarded, and pass/bundle setters panic | [PR 10098](https://github.com/gfx-rs/wgpu/pull/10098) is open/conflicted; capability policy and browser support remain under review. [wasm-bindgen PR 5289](https://github.com/wasm-bindgen/wasm-bindgen/pull/5289) merged an immutable-slice binding correction | More than wiring setters; audit capability, limits, layout, bindings and all encoder paths together.    |
| Primitive index            | Existing bit/generated enum, missing browser feature mapping                                       | Release source `FEATURES_MAPPING`; [Chrome explanation](https://developer.chrome.com/blog/new-in-webgpu-142#primitive_index_in_wgsl)                                                                                                                             | Optional for a visibility buffer. Explicit flat IDs remain available; exposure alone proves no speedup. |
| New WGSL language features | Reporting recognizes only a subset of Chrome's extensions                                          | Release source language-feature mapping; [WGSL buffer views](https://gpuweb.github.io/gpuweb/wgsl/#language_extension-buffer_view)                                                                                                                               | Separate capability reporting from Naga's native language implementation.                               |
| Transient attachments      | Available in 30.0.1                                                                                | [PR 9568](https://github.com/gfx-rs/wgpu/pull/9568) merged June 19                                                                                                                                                                                               | No fork needed. Respect pass-local lifetime.                                                            |

[PR 10265](https://github.com/gfx-rs/wgpu/pull/10265) proposes centralizing feature mapping.
Its September 11 review requests an allowlist: advertising a string before the corresponding
backend fields work is unsafe. This supports narrowly audited exposure, not enabling every
feature Chrome happens to report. The inspected open issues/PRs had no committed release date.

## Scope of a reasonable fork

Keep a small patch series on a released wgpu version, pin patched workspace crates consistently,
and upstream/retire patches as equivalents land. Browser details remain within wgpu's backend;
Voxels `render` stays free of browser and WASM types.

For **subgroups**, map adapter reporting and device requests, use standard browser WGSL
directives, and test vote/shuffle/reduction/scan results including partially active subgroups.
Do not assume native-only operations or subgroup barriers exist in browser WGSL.

For **subgroup size control**, add the public bit/name and report/request mapping. Chrome's
shader attribute carries the size; the browser path does not need the native Vulkan PR.
Test supported sizes, workgroup constraints, reported builtins and invalid-size rejection.

For **immediates**, cover every seam:

1. Detect actual API/language support without sending a fictional required-feature name.
2. Read `maxImmediateSize`, retaining a correct fallback when absent.
3. Forward requested limits and preserve adapter/device limit semantics.
4. Forward `PipelineLayoutDescriptor::immediate_size`; test explicit and automatic layouts.
5. Implement compute-pass, render-pass and render-bundle setters with reproducibly generated
   bindings, correct byte slices, offsets, alignment and bounds.

PR 10098 alone does not cover all the release-30 limit and explicit-layout omissions found
in this audit. Treat it as source material, not a complete drop-in fix.

Keep host validation for portable shaders and pure algorithms. Add real Chrome shader-compilation
and GPU-result tests for browser-specific variants, with error scopes and unsupported-feature
behavior. Measure the actual workload after correctness passes. A crate build or an advertised
feature is insufficient evidence.

This is a plausible bounded maintenance commitment. Full native Naga/Metal/HLSL feature parity
is a separate, much larger project and is unnecessary for the Chrome-first target. A fork cannot
add browser APIs that Chrome does not provide, such as standard hardware ray tracing, hardware
sparse residency or mesh shaders. None of these small feature bridges compensates for whole-atlas
copies, global edit invalidation, excessive traversal or insufficient sampling.
