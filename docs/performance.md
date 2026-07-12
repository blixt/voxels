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
49/49/49/81 circular sets. Each terrain tile remains one GPU allocation but exposes sixteen 8x8-cell
body ranges and four conditional edge-skirt ranges per patch. Color and all three shadow cascades
filter the same ranges, and adjacent ranges still coalesce into a single draw span where allocation
order permits.

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

## 2026-07-12: edit-aware skyline proxies

The surface rings now retain procedural trees instead of flattening the horizon once canonical chunks
end. Each generator placement cell yields one analytic feature descriptor used by canonical voxels and
by anchor-owned LOD proxies. Stride-2 proxies use four boxes (24 quads), stride-4 uses three (18 quads),
and the outer levels use two (12 quads); they stay inside the owning patch allocation and therefore add
no mesh object or JavaScript overhead. A single-tab debug build rendered the expanded forest view at
26 FPS in the in-app browser, versus 9 FPS while a second WebGPU tab rendered the same world; release
profiling remains the next useful end-to-end comparison.

Native stride-8 surface generation remained unchanged within noise at 0.161 ms. The 10,000-unrelated-
edit case measured 0.164 ms and improved slightly, confirming that pristine-feature checks use indexed
chunk buckets instead of scanning the edit journal. Host tests prove single anchor ownership, fixed
patch coverage despite protruding culling bounds, cross-tile crown invalidation, proxy suppression on a
canonical tree edit, and restoration when the edit returns to generated state.

## 2026-07-12: authoritative water and draw-range packing

Generator v5 and material schema v2 add editable canonical Water, edit-aware water masks at all four
surface levels, and a dedicated refractive renderer. A pristine stride-8 water tile first
measured 1.351 ms when every mask sample invoked the general 3D voxel sampler. Using the shared
`SurfaceSample` for generated occupancy and consulting the sparse override index only at the sea cell
reduced that work to 0.140 ms, an 89.6% reduction. The benchmark remains in `world/benches/world.rs`.

The first coastal browser capture exposed 1,375 main-pass draw spans at full 81/81/81/121 surface
residency. Transition skirt bytes were interleaved after every patch body, so rejecting internal skirts
prevented otherwise adjacent bodies from coalescing. Packing all common bodies first, keeping skirts
at the tail, and separating opaque/water arenas reduced the same view to 432 main-pass spans while
holding the 120 Hz display cap. The view contained 127,800 total quads, including 503 visible water
quads; the water depth/color pair required 94 draws across the patch-aligned LOD boundaries.

Live browser validation also opened two tabs simultaneously and reloaded both twice in parallel. Both
returned to a canvas-only rendered world with no OPFS/database warning or error. The only document-body
children remained the canvas and its non-rendering module script.

The persistence coordinator now retains the SAH-pool management handle and releases ownership in the
strict order SQLite close -> VFS pause -> Web Lock release. Its dedicated Chrome regression ran 18
rapid reloads across three tabs and two owner handoffs on an isolated localhost origin. All three
workers reached resident rendered geometry, every body stayed canvas-plus-script only, and the gate
recorded zero OPFS, SQLite, SyncAccessHandle, Web Lock, or persistence warnings/errors.

## 2026-07-12: navigable water and underwater medium

Generator v6 replaces the former shallow basin subtraction with continental shelf/slope/basin
bathymetry. The deterministic coastal showcase column now contains 2.2 m of canonical water instead of
0.5 m, and a host invariant requires at least 2.0 m there. Water remains editable at 10 cm resolution.

Exact fluid sensing roughly doubles portable camera-update cost, but the absolute budget is small. On
the same release benchmark host, 120 dry grounded ticks measured 90.48 us and 120 submerged ticks
measured 177.87 us: about 0.75 us versus 1.48 us per 120 Hz tick. `vp run bench:core` preserves both
baselines. The browser gate entered the Rust underwater showcase, toggled water rendering off and back
on while verifying physics stayed submerged, and completed with no WGPU/shader warning or error. The
captured frame retained only canvas-rendered crosshair, depth/immersion status, and swim-help chrome.
The same isolated browser sequence now performs a real pointer-locked removal in a follower tab and
waits for the leader's live Rust edit map to reach the same revision before any reload. This covers the
follower proxy, SQLite commit, committed-edit broadcast, and canonical/LOD invalidation path in addition
to storage ownership handoff.

## 2026-07-12: raw release frame profiling and bounded streaming

`vp run profile:browser` now builds optimized WASM, serves `dist` from an isolated ephemeral origin,
and drains raw Rust worker frame samples from a fixed 512-entry ring. The versioned snapshot keeps
display interval separate from total worker CPU, simulation, streaming, and render-submission time;
the harness reports p50/p95/p99/max and hitch counts without treating an exponentially smoothed UI
number as a distribution.

Measured in system Chrome at 1280x720 DPR 1 on the same Apple M3 Max, reducing canonical meshing
admission from two chunks to one removed busy-frame overload without slowing the measured cold settle.

| Release scenario                      |         Metric | Two meshes/frame | One mesh/frame |
| ------------------------------------- | -------------: | ---------------: | -------------: |
| Cold navigation to all queues settled |      wall time |           3.43 s |         3.13 s |
| Traversal                             |      frame p95 |          16.6 ms |         9.6 ms |
| Traversal                             | worker CPU p95 |          11.2 ms |         7.6 ms |
| Traversal                             |  streaming p95 |           8.7 ms |         5.1 ms |
| Underwater load                       |      frame p95 |          16.6 ms |         9.5 ms |
| Underwater load                       | worker CPU p95 |          10.8 ms |         7.3 ms |
| Underwater load                       |  streaming p95 |           8.3 ms |         4.7 ms |

The next bottleneck was repeated analytic-tree discovery for all 1,156 columns in a chunk's 34x34
meshing halo. `GeneratedRegion` now discovers intersecting features once, then builds authoritative
column samplers from that shared set. Criterion measured canonical meshing at 1.177 ms, down from
2.620 ms (55.1%), and ocean meshing at 0.871 ms, down from 2.316 ms (62.4%). Generator output is
unchanged; host tests compare region sampling with random-access generation across negative space,
tree-cell boundaries, and the ocean showcase.

With that sampler in the integrated release build, traversal frame p95 remained 9.4 ms while worker
CPU p95 fell to 5.6 ms and streaming p95 to 3.0 ms. Underwater frame p95 reached 9.1 ms, CPU p95
5.3 ms, and streaming p95 2.6 ms. The traversal and underwater samples recorded no interval above
16.67 ms and no dropped telemetry samples.

The renderer now shades its procedural sky at the far depth after opaque terrain, allowing early
depth rejection behind the world. Opaque refractive water writes color and depth in one pass rather
than submitting identical geometry for separate depth and color passes. The measured underwater
view kept 520 visible water quads and reduced total world draws from 509 to 430 while retaining the
same 7.03 MiB full-resolution refraction copy. These CPU and submission results predate the GPU
timestamp instrumentation below and therefore make no claim about GPU pass time.

## 2026-07-12: asynchronous GPU pass timing

The renderer now requests WebGPU's optional `timestamp-query` feature only when the adapter exposes
it. Twelve pass-boundary timestamps attribute three shadow cascades, opaque world/sky, refractive
water, and final present/Rust UI work. Results resolve into a four-buffer asynchronous readback ring;
if every slot is still in flight, that frame simply skips measurement rather than polling or waiting.
Unsupported adapters continue rendering and report GPU time as unavailable.

System Chrome exposed timestamp queries on the Apple M3 Max. At 1280x720, 4-second release samples
reported the following GPU active-window and individual-pass distributions:

| Scenario         | Active window p50 / p95 | Shadows p50 / p95 | World p50 / p95 | Water p50 / p95 |   UI p50 / p95 |
| ---------------- | ----------------------: | ----------------: | --------------: | --------------: | -------------: |
| Settled forest   |          2.69 / 4.59 ms |    2.57 / 4.63 ms |  0.66 / 1.15 ms |        0 / 0 ms | 0.28 / 0.46 ms |
| Traversal        |          2.52 / 4.17 ms |    2.34 / 3.97 ms |  0.66 / 0.99 ms |        0 / 0 ms | 0.28 / 0.36 ms |
| Underwater coast |          1.22 / 2.87 ms |    0.99 / 2.69 ms |  0.37 / 0.63 ms |  0.24 / 0.37 ms | 0.20 / 0.37 ms |

The active window is `max(active pass end) - min(active pass start)`. WebGPU implementations may
overlap or reorder passes, so individual pass durations are not summed and the interval between world
and water is not mislabeled as pure refraction-copy cost. The Rust Mission Control GPU field now shows
the latest completed active-window sample; it remains an explicit unavailable value without adapter
support.

## 2026-07-12: live versus committed memory telemetry

Snapshot schema v4 separates three kinds of memory instead of presenting one ambiguous total:

- committed WASM linear memory, sampled from the current `WebAssembly.Memory` buffer;
- known logical Rust payload for canonical voxel arrays, pending mesh vector capacity, and the
  triple-indexed sparse edit map;
- estimated core GPU resources, including mesh arena pages, scene/depth targets, shadows, and GPU
  timestamp buffers.

The logical figures exclude B-tree node/allocator overhead, SQLite internals, WGPU backend objects,
and JavaScript allocations. Committed WASM memory includes allocator slack and opaque Rust/WASM
consumers and may grow without shrinking, so neither is labeled process memory.

The release run started at 20.875 MiB committed WASM with 15.188 MiB of canonical voxel payload.
Short traversal grew those figures to 22.750 MiB and 18.000 MiB while retaining 288 chunks; after the
underwater focus jump and queue drain, canonical payload returned to 15.188 MiB while committed WASM
remained at its allocator high-water mark. Pending mesh payload returned to zero and the scheduler
recorded 540 total evictions with no stale completion. A longer deterministic traversal is still
required before setting a plateau regression ceiling.

Persistence hydration now moves the initial `EditMap` into the engine exactly once. Previously the
store cloned all three edit indices and retained the original for the entire session, creating an
avoidable near-2x edit-journal payload before any new edit was made.

## 2026-07-12: editable regional landmarks

Generator v7 generalizes the old broadleaf-only feature path into six region-specific landmark
archetypes. Canonical shapes remain ordinary 10 cm voxels and coarse representations stay packed into
their owning surface patch with at most four proxy boxes. The native `generate 32^3 chunk` Criterion
mean remained 709.12 us on the M3 Max, with no statistically significant change from its saved
baseline. The release edit profile also restored all 40 terrain/water operations with 9.4 ms frame p95,
3.1 ms SQLite/OPFS enqueue p95, and 29.7 ms full canonical-plus-LOD convergence p95.

The first one-way sustained run reached denser landmark geometry late and grew the arena from 60 to
68 MiB during the nominal plateau window. That exposed a flaw in the benchmark: different terrain was
being mistaken for allocator growth. The rail now closes after exactly the 30-second warm-up lap and
measures two repetitions of that same terrain. A flat measured capacity therefore demonstrates page
reuse after eviction while retaining the original distance, streaming, and drain gates.

## 2026-07-12: sustained deterministic streaming rail

`vp run profile:sustained` starts one numeric debug command through the browser transport; all
scenario timing and movement remain in portable Rust. The camera follows a fixed 120 Hz circular rail
at 12 m/s: one 360 m lap warms the allocator, two identical laps measure it, and the cumulative 1.08 km
run then stops and requires canonical and all four surface-LOD queues to drain. The harness runs against
release WASM on an isolated ephemeral origin and enforces frame, memory, residency, and queue gates.

The first run exposed unfinished, undesired chunks retained inside scheduler hysteresis. Such entries
were deliberately excluded from future admission but still counted as queued forever, preventing a
final drain. Retention now preserves useful resident geometry only; unfinished undesired work is
evicted immediately and any late ticket is stale. A host test covers the inside-retention case.

The corrected M3 Max / system Chrome run produced:

| Sustained measurement                     |                Result |                     Gate |
| ----------------------------------------- | --------------------: | -----------------------: |
| Distance                                  |               1,080 m |         at least 1,000 m |
| Canonical evictions                       |                14,287 |             at least 500 |
| Tracked canonical high-water              |                   320 |              at most 320 |
| Surface-resident high-water               |                   725 |              at most 896 |
| Pending mesh high-water                   |                     1 |                at most 3 |
| Frame p95 / p99 / max                     |  9.3 / 10.3 / 16.9 ms | 12 / 16.67 / no 33.33 ms |
| Worker CPU p95 / p99                      |          6.4 / 7.0 ms |               7.5 ms p95 |
| Streaming p95 / p99                       |          3.3 / 3.6 ms |               4.5 ms p95 |
| GPU active-window p95 / p99 / max         | 6.88 / 7.57 / 7.63 ms |        recorded baseline |
| Final-20-second WASM committed range      |             0.375 MiB |            at most 1 MiB |
| Final-20-second arena-capacity range      |                 0 MiB |         at most one page |
| Dropped telemetry / stale completions     |                 0 / 0 |                     zero |
| Final pending jobs / pending mesh payload |                 0 / 0 |                     zero |

The rail reached 27.063 MiB committed WASM and 56 MiB of mesh-arena capacity. Those are high-water
figures, not live payload claims. The active surface plus pending coverage union stayed below its
conservative 896-tile bound, and the final focus activated only after every replacement queue drained.

## 2026-07-12: submitted edit convergence

`vp run profile:edits` teleports through one numeric browser command, then Rust serializes 40 edits at
a fixed coastal column: ten terrain removals/restores and ten water removals/restores. Each operation
invalidates three 32-cubed canonical chunks and one tile at each of the 0.2, 0.4, 0.8, and 1.6 metre
surface levels. The next edit is withheld until all exact revisions reach residency and a frame using
them reaches WGPU submission. The fixture refuses follower execution and pre-existing edits, and must
finish with the sparse edit map restored to its starting row count.

The M3 Max / system Chrome release run produced:

| Edit measurement                      |         Result |                Gate |
| ------------------------------------- | -------------: | ------------------: |
| Operations / dropped / superseded     |     40 / 0 / 0 |      exactly 40/0/0 |
| SQLite/OPFS dispatch p95 / max        |   3.0 / 3.2 ms |       8 / 25 ms max |
| Canonical replacement p95 / max       | 30.3 / 30.9 ms |    100 / 200 ms max |
| Full submitted LOD p95 / max          | 30.3 / 30.9 ms |    150 / 250 ms max |
| Frame p95 / max                       |  9.3 / 10.3 ms | 16.67 / no 33.33 ms |
| Final edits / trackers / mesh payload |      0 / 0 / 0 |                zero |

The Mission Control panel receives the latest full convergence time and active tracker count through
`LiveStats`; those status cards remain Rust-composed and WGPU-rendered like the crosshair, startup
controls toast, toggles, hover surfaces, and context menu.
