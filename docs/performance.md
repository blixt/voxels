# Performance baselines

Performance changes must keep a reproducible before/after number beside the architectural reason for
the change. Native Criterion results are stable microbenchmarks for portable Rust algorithms; browser
Mission Control and `window.__VOXELS__.snapshot()` cover integrated frame pacing and residency.

## 2026-07-11: streamed chunk preparation

Measured on an Apple M3 Max with Rust 1.97.0 using the release-profile command below. Criterion used
100 samples after its standard warm-up.

```sh
vp run bench:world
```

| Operation                                  |                Before |    After |  Change |
| ------------------------------------------ | --------------------: | -------: | ------: |
| Generate one canonical 32³ chunk           |              2.795 ms | 0.600 ms |  -78.5% |
| Greedy mesh one generated chunk            |              1.517 ms | 1.470 ms |   -3.1% |
| Generate far tile with 10k unrelated edits | unbounded linear scan | 0.750 ms | indexed |

Generation previously recalculated height and biome noise for every Y voxel. The optimized path
computes an immutable profile once for each of the chunk's 1,024 X/Z columns, then uses it while
filling all 32 layers. A host test compares stratified generated voxels with the authoritative
random-access sampler, so the optimization does not change generator identity or saved edits.

Meshing now stores its short-lived 34³ solid halo as bytes instead of `Vec<bool>` proxy bits. This is
a small speed win and keeps each hot solidity read branch-free.

Sparse edits now maintain secondary chunk and X/Z-column indices. Applying edits to one canonical
chunk and sampling one LOD surface column no longer scans the entire world edit journal. A far tile
with 10,000 unrelated durable edits generated in 0.750 ms, only about 12% above the unedited 0.670 ms
baseline instead of multiplying 5,120 surface queries by the global edit count.

The worker admits two generation tickets per display frame after this change. Their measured native
CPU budget is about 1.20 ms combined, less than half the former one-ticket cost of 2.80 ms, while
doubling initial residency throughput. Meshing and GPU upload remain independently bounded, so a
large focus jump cannot enqueue unbounded same-frame work.

Integrated browser validation with three 1024² sun-shadow cascades held the 120 Hz display cap in the
tested spawn view. Before expanding the world, coalescing byte-contiguous visible arena allocations
preserved 51 visible meshes and 45,800 quads while reducing main draws from 51 to 26 and
light-volume-culled shadow draws from 299 to 28.

The expanded configuration keeps 243 canonical chunks plus four complete surface rings at 0.2, 0.4,
0.8, and 1.6 m sample spacing. The steady spawn view contained 49/49/49/81 surface tiles, 471 total
resident meshes, 144 visible meshes, about 134,900 quads, 81 main draw spans, 123 shadow draw spans,
and 10.9 MiB of live arena allocations while remaining at the 120 Hz display cap. The outer ring
extends beyond the 220 m fog cutoff. Turning shadows off still reduces their passes to zero; disabling
far terrain remains a hard CPU filter in both kinds of pass.

The linear-HDR outdoor-lighting pass retained that 120 Hz cap at 1280×720 with the same 144 visible
meshes, roughly 134,900 quads, 81 main draw spans, and 123 shadow draw spans. Its `Rgba16Float` scene
target costs 8 bytes per pixel (7.03 MiB at that viewport); Mission Control now reports core GPU
residency including allocated mesh arena pages, the HDR scene target, the depth target, and all three
shadow cascades instead of presenting mesh payload bytes as total rendering memory. Procedural clouds,
linear material response, dielectric highlights, tone mapping, and exponential-height aerial
perspective remained inside the existing frame budget.

Initial fine-chunk residency in this larger configuration measured 118 scheduler frames p95 and 124
frames maximum, roughly 983/1,033 ms at 120 Hz. Mission Control renders load and edit-remesh p95/max
counters and all four ring populations directly through Rust/WGPU; the scheduler uses fixed,
allocation-free histograms so collecting them does not add per-frame heap work.

These numbers are a decision record, not universal hardware targets. Criterion's stored baselines in
`target/criterion/` remain ignored and local; repeat the command on target hardware before changing
per-frame admission budgets.

## 2026-07-12: regional surface ecology

Generator v4 replaces the single generic surface formula with six regional terrain influences and a
shared climate/geology sample. On the same Apple M3 Max and Criterion configuration, canonical chunk
generation rose from 0.600 ms to 0.730 ms while remaining below the 1 ms admission target. Caching the
resulting regional profile and tree intersections per X/Z column reduced greedy meshing from 1.470 ms
to 1.198 ms, leaving generation plus meshing at roughly 1.93 ms rather than 2.07 ms.

| Operation                             | Previous | Regional v4 | Change |
| ------------------------------------- | -------: | ----------: | -----: |
| Generate one canonical 32³ chunk      | 0.600 ms |    0.730 ms | +21.7% |
| Greedy mesh one generated chunk       | 1.470 ms |    1.198 ms | -18.5% |
| Generate one stride-8 surface tile    | 0.670 ms |    0.156 ms | -76.7% |
| Surface tile with 10k unrelated edits | 0.750 ms |    0.173 ms | -76.9% |

The far-tile improvement comes from making generator and edit-overlay surface queries consume the
same material-bearing `SurfaceSample` once, rather than separately recomputing height and top
material. Runtime meshing uses `GeneratedColumn` through a bounded 34×34 halo cache. A version-coupled
golden checksum and a fixed-grid catalog test prove deterministic output, surface/material
self-consistency, and representation of all six regions.

## 2026-07-12: geometric surface LOD handoff

Surface residency now uses complete square coverage sets so every grid-aligned CPU ownership patch is
available before activation. The four sets contain 81/81/81/121 tiles at the configured radii, up from
49/49/49/81 circular sets. Each tile remains one GPU allocation but exposes sixteen 8x8-cell body
ranges and four conditional edge-skirt ranges per patch. Color and all three shadow cascades filter the
same ranges, and adjacent ranges still coalesce into a single draw span where allocation order permits.

The larger coverage set is prepared behind the currently active set. A focus change retains active
tiles while pending tiles generate, then swaps the grid-snapped ownership plan only when canonical and
all surface queues are clean. This intentionally trades bounded transient residency for a hole-free
handoff rather than showing half of a new ring.

The portable release benchmarks remained within their noise thresholds after adding structured mesh
generation: one canonical chunk measured 0.719 ms, greedy meshing measured 1.201 ms, and the legacy
surface-shell API measured 0.168 ms. That compatibility API skips transition-skirt construction;
runtime structured tiles construct skirts because they can select those ranges independently.

Browser validation opened two simultaneous tabs against the same OPFS world and rapidly reloaded one
tab several times. Both workers recovered to a rendered world through Web-Lock ownership handoff, and
both document bodies contained only the canvas plus the non-rendering module script. This exercises the
same stale sync-access-handle retry path used after a quick refresh.
