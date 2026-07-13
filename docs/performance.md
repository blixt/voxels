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
generation: one canonical chunk measured 0.719 ms, greedy meshing measured 1.201 ms, and a structured
surface tile measured 0.168 ms including independently selectable patch and transition-skirt ranges.

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
returned to a canvas-only rendered world with no OPFS/database warning or error. The module script now
lives in the document head, leaving the canvas as the only document-body child.

The persistence coordinator now retains the SAH-pool management handle and releases ownership in the
strict order SQLite close -> VFS pause -> Web Lock release. Its dedicated Chrome regression now runs 38
reloads across three tabs: a 12-reload solo burst with no stable follower, four simultaneous two-tab
reload rounds, and two owner handoffs on an isolated localhost origin. All three workers reached
resident rendered geometry, every body retained only its canvas, and the gate recorded zero OPFS,
SQLite, SyncAccessHandle, Web Lock, or persistence warnings/errors. A second fault-injection gate forced
all 20 acquisitions in the first VFS-open cycle to fail with `NoModificationAllowedError`; election
recovered on call 21, rendered the world, and emitted zero console warnings/errors.

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
baseline. The release edit profile also restored all 40 terrain/water operations with 9.3 ms frame p95,
2.6 ms SQLite/OPFS enqueue p95, and 29.4 ms full canonical-plus-LOD convergence p95.

The first one-way sustained run reached denser landmark geometry late and grew the arena from 60 to
68 MiB during the nominal plateau window. That exposed a flaw in the benchmark: different terrain was
being mistaken for allocator growth. The rail now closes after exactly the 30-second warm-up lap and
measures two repetitions of that same terrain. A flat measured capacity therefore demonstrates page
reuse after eviction while retaining the original distance, streaming, and drain gates.

## 2026-07-12: Rust-generated material surface detail

The opaque world shader now samples deterministic Rust-generated albedo and normal/roughness arrays
through world-anchored face coordinates. Eight explicitly generated mip levels preserve linear albedo
energy and unresolved normal variance. A second specialized flat pipeline provides a true zero-sample
comparison path, selected by the seventh Rust-rendered Mission Control toggle.

`vp run profile:materials` opens the panel and changes that toggle through canvas hit-testing, then
records five-second OFF and ON windows in one release browser session. It requires both observed states
and identical geometry, residency, draw calls, arena allocations, water work, and refraction bandwidth.
The final M3 Max / system Chrome run produced:

| Material-detail measurement         |             OFF |              ON |       ON - OFF / gate |
| ----------------------------------- | --------------: | --------------: | --------------------: |
| World GPU p95                       |        1.222 ms |        1.443 ms |  +0.220 / at most 0.5 |
| Active-window GPU p95               |        5.518 ms |        5.535 ms | +0.017 / at most 0.75 |
| Frame p95                           |        9.500 ms |        9.700 ms |        +0.200 / 12 ms |
| Draw calls / visible quads          |   319 / 865,905 |   319 / 865,905 |             unchanged |
| Mesh allocation / capacity          | 26.425 / 32 MiB | 26.425 / 32 MiB |             unchanged |
| Dropped samples / frames over 33 ms |           0 / 0 |           0 / 0 |                  zero |

The two `128x128x14` eight-mip arrays occupy exactly 2,446,640 bytes (2.333 MiB), asserted by a stable
host checksum and included in core GPU diagnostics. They add no mesh bytes, render pass, draw object,
draw call, or refraction copy. Caching each periodic height layer before finite-difference normal
generation reduced the atlas host-test runtime from about 1.05 to 0.68 seconds in the same debug test
process; release browser settling was 3.273 seconds.

Visual A/B inspection rejected the first regular grass-blade wave because it produced field-scale
moiré despite passing the GPU gate. The retained profile uses lower-amplitude stochastic grass/moss
structure, while stone, limestone, snow, wood, sand, and landmark materials retain more distinct
weathered structure. Mip filtering and distance-faded normal strength keep the far field stable.

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
| Canonical evictions                       |                14,289 |             at least 500 |
| Tracked canonical high-water              |                   320 |              at most 320 |
| Surface-resident high-water               |                   725 |              at most 896 |
| Pending mesh high-water                   |                     1 |                at most 3 |
| Frame p95 / p99 / max                     |  9.3 / 10.1 / 18.3 ms | 12 / 16.67 / no 33.33 ms |
| Worker CPU p95 / p99                      |          6.5 / 7.0 ms |               7.5 ms p95 |
| Streaming p95 / p99                       |          3.3 / 3.6 ms |               4.5 ms p95 |
| GPU active-window p95 / p99 / max         | 5.44 / 5.87 / 6.02 ms |        recorded baseline |
| Final-20-second WASM committed range      |                 0 MiB |            at most 1 MiB |
| Final-20-second arena-capacity range      |                 0 MiB |         at most one page |
| Dropped telemetry / stale completions     |                 0 / 0 |                     zero |
| Final pending jobs / pending mesh payload |                 0 / 0 |                     zero |

The rail reached 26.938 MiB committed WASM and 56 MiB of mesh-arena capacity with material detail
enabled. Those are high-water figures, not live payload claims. The active surface plus pending
coverage union stayed below its conservative 896-tile bound, and the final focus activated only after
every replacement queue drained.

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
| SQLite/OPFS dispatch p95 / max        |   2.6 / 2.9 ms |       8 / 25 ms max |
| Canonical replacement p95 / max       | 29.4 / 29.6 ms |    100 / 200 ms max |
| Full submitted LOD p95 / max          | 29.4 / 29.6 ms |    150 / 250 ms max |
| Frame p95 / max                       |  9.3 / 10.4 ms | 16.67 / no 33.33 ms |
| Final edits / trackers / mesh payload |      0 / 0 / 0 |                zero |

The Mission Control panel receives the latest full convergence time and active tracker count through
`LiveStats`; those status cards remain Rust-composed and WGPU-rendered like the crosshair, startup
controls toast, toggles, hover surfaces, and context menu.

## 2026-07-12: composed landmark hierarchy

Generator v8 replaces independent landmark scatter with deterministic 76.8 m composition cells while
retaining one bounded editable feature per 9.6 m ownership cell. Cluster, ring, clearing, and procession
grammars select background, companion, and hero prominence; canonical shapes and all four LOD proxy
levels consume the same descriptor. The Rust landmark tour now prefers heroes, and browser inspection
confirmed distinct forest and moor hero/companion silhouettes without a DOM overlay.

Native 32-cubed generation measured 714.38 us mean, a 0.95% increase that Criterion classified inside
the noise threshold. Greedy meshing measured 1.196 ms mean with no detected change. The release browser
gates produced:

| Composition measurement               |               Result |                     Gate |
| ------------------------------------- | -------------------: | -----------------------: |
| Steady / traversal / underwater p95   |   9.9 / 9.5 / 9.5 ms |                    12 ms |
| Sustained frame p95 / p99 / max       | 9.8 / 10.3 / 16.7 ms | 12 / 16.67 / no 33.33 ms |
| Sustained CPU / streaming p95         |         6.5 / 3.3 ms |                7.5 / 4.5 |
| Sustained distance / evictions        |     1,080 m / 14,296 |          >=1,000 / >=500 |
| Final WASM / arena capacity range     |            0 / 0 MiB |         <=1 / <=one page |
| Edit enqueue / convergence p95        |        3.2 / 29.4 ms |               8 / 150 ms |
| Dropped telemetry / stale completions |                0 / 0 |                     zero |

The edit gate completed 40 canonical-plus-LOD remove/restore operations, returned to zero sparse edits,
and recorded no superseded work or browser errors. The sustained rail drained every final job and mesh
payload, so the larger hero silhouettes do not weaken bounded streaming or allocation reuse.

## 2026-07-12: editable pilgrim road and route landmarks

Generator v9 introduces one terrain-aware, 164.7 m route from the spawn forest through moor into the
badlands. Generator v10 adds five route-first cairn, waystone, and ruined-arch identities. The same
canonical column overlay owns paving, collision, water exclusion, sparse edits, and all four LODs;
host tests walk the road at 10 cm intervals and prove dry passability, bounded cut/fill, chunk/region
agreement, edit restoration, and cardinal LOD connectivity. Route landmark tests additionally cover
stable cadence, ambient-placement precedence, canonical visibility, proxy suppression, and restoration
at every LOD.

Dedicated Criterion cases keep route cost visible beside the off-road reference:

| Native release operation                   | Mean time |
| ------------------------------------------ | --------: |
| Existing off-road canonical 32-cubed chunk | 708.71 us |
| Pilgrim-road canonical 32-cubed chunk      | 653.23 us |
| Route midpoint stride-2 surface tile       | 186.45 us |
| Route midpoint stride-16 surface tile      | 214.58 us |

The outer tile covers substantially more physical terrain and route-adjacent feature cells; both LOD
cases remain below 0.22 ms. The off-road reference stayed within Criterion's saved noise threshold.

The current release browser gates at 1280x720 produced:

| Integrated measurement                        |               Result |
| --------------------------------------------- | -------------------: |
| Steady / traversal / underwater frame p95     |   9.3 / 9.3 / 9.3 ms |
| Traversal CPU / streaming p95                 |         5.8 / 3.0 ms |
| Sustained frame p95 / p99 / max               | 9.1 / 10.0 / 10.4 ms |
| Sustained CPU / streaming p95                 |         6.6 / 3.4 ms |
| Sustained distance / evictions                |     1,080 m / 14,276 |
| Final-20-second WASM / arena-capacity range   |            0 / 0 MiB |
| Edit enqueue / full submitted convergence p95 |        3.0 / 30.4 ms |
| Dropped samples / stale completions / errors  |            0 / 0 / 0 |

The 40-operation edit profile restored the sparse map to zero rows with no superseded work. The OPFS
gate completed 38 rapid reloads across three tabs, including four simultaneous reload rounds and a
live follower edit, with zero persistence errors. Its fault-injection companion denied the first 20
VFS acquisitions and recovered on call 21 with a canvas-only document and zero console errors.

Visual release inspection used the Rust-rendered “Follow pilgrim road” context action to reconstruct
the first ruined arch beside the paved bed. Mission Control, context menu, telemetry, crosshair, and
startup controls remained WGPU draw data; the document body contained exactly one canvas.

## 2026-07-12: continuous regional daylight

The renderer no longer applies one static atmosphere to every region. The generator exposes six
normalized continuous fields from its existing regional classifier, while the shell samples them at
the camera and the renderer smooths toward the target with a frame-rate-independent response. Dawn,
clear day, golden hour, and blue hour alter one shared sun/sky/fog/cloud environment; blue hour adds a
sparse procedural star field. The sky and terrain evaluate the same world-anchored cloud field so
cloud motion also modulates direct sunlight without adding a render pass, texture, draw, or allocation.

`vp run profile:atmosphere` changes all four phases by opening the Rust Mission Control context menu
and clicking the canvas row. At 1280x720 on the M3 Max / system Chrome, every four-second window held
the 120 Hz cap with zero dropped samples and no frame over 16.67 ms:

| Phase       | Frame p95 | World GPU p95 | Active-window GPU p95 | Cloud cover |
| ----------- | --------: | ------------: | --------------------: | ----------: |
| Golden hour |    9.2 ms |      1.607 ms |              5.574 ms |       0.586 |
| Blue hour   |    9.7 ms |      0.961 ms |              2.899 ms |       0.574 |
| Dawn        |    9.4 ms |      1.465 ms |              4.987 ms |       0.621 |
| Clear day   |    9.5 ms |      0.900 ms |              2.945 ms |       0.609 |

All phases retained 243 resident chunks, 107 visible chunks, 843,444 quads, 319 draws, 32 MiB arena
capacity, and zero pending jobs. The profile stores one screenshot per phase under ignored `target/`
output and rejects any geometry, residency, draw, allocation, or dropped-sample difference.

The first implementation derived atmosphere for every `SurfaceSample`; Criterion immediately exposed
2–13% regressions in surface-LOD cases. The retained design keeps regional classification shared but
derives atmosphere only in the explicit camera sampler. Final native means were 704.98 us for an
off-road canonical chunk, 652.94 us for the pilgrim-road chunk, 187.38 us for its stride-2 tile,
215.32 us for its stride-16 tile, 122.67 us for edit-aware stride-8 water, and 160.99 us for the far
surface tile. Criterion classified the canonical, water, far, codec, and meshing comparisons within
noise; the route cases returned to their pre-atmosphere range.

The browser boundary remains exactly one canvas. Crosshair, startup control help, atmosphere status,
toggles, menus, hover states, and performance counters are Rust draw commands; fatal boot/worker errors
go to the console. The persistence regression completed 38 rapid reloads across three tabs with live
edit synchronization and zero persistence errors. Fault injection denied 20 consecutive OPFS VFS
acquisitions, then recovered on call 21 with zero console errors.

## 2026-07-12: spatial horizon ambient occlusion

The Rust renderer now reconstructs screen-space contact occlusion from its own depth prepass and
applies the depth-aware half-resolution result only to indirect light. The implementation has no
temporal history and therefore adds no invalidation protocol for streaming replacements, edits, or
teleports. Mission Control owns the toggle and live AO timing; the browser still owns no UI state.

`vp run profile:gtao` followed all five pilgrim-road marks to a dense badlands and ruin view before
measuring matched four-second windows at 1280x720 on the M3 Max / system Chrome:

| Spatial AO | Frame p95 | Active GPU p95 | World GPU p95 | Depth p95 |   AO p95 |
| ---------- | --------: | -------------: | ------------: | --------: | -------: |
| Off        |    9.2 ms |       2.620 ms |      0.776 ms |  0.000 ms | 0.000 ms |
| On         |    9.0 ms |       3.125 ms |      0.681 ms |  0.461 ms | 0.499 ms |

Both windows held the 120 Hz cap for 677 samples with zero drops or errors. They retained identical
243 resident chunks, 115 visible chunks, 1,382,838 quads, 333 opaque draws, arena allocation, and
canonical edit memory. The AO depth replay issued exactly 333 draws when enabled and none when
disabled. Its two `Rg16Float` targets consume 1.7578125 MiB at this viewport; the measured p95
depth-plus-AO cost was 0.960 ms. Captured A/B frames showed stronger step, crease, and landmark-base
contact without sky halos or attenuation of direct highlights.

The exact head also repeated the canvas and persistence release gates: 38 rapid reloads across three
tabs completed with live edit synchronization and zero persistence errors, while 20 injected OPFS VFS
acquisition failures recovered on call 21 with zero console errors. Both harnesses asserted that the
document body contained exactly one canvas.

## 2026-07-12: semantic regional heroes

Generator v11 gives the composition director six region-specific hero forms instead of scaling the
ordinary regional prop. Elder canopies, tor circles, needle gates, buried ribs, buried colonnades, and
basalt crowns remain canonical 10 cm voxels. Their exact bounds drive chunk decoration and edit
invalidation, while each LOD proxy remains capped at four boxes and 24 quads. Existing background and
route anchors keep their established placement; only the one hero cell in each 8x8 composition area
pays for the larger form.

`vp run profile:heroes` advanced the Rust landmark catalog exclusively through canvas clicks and
captured all six fixed views. At 1280x720 on the M3 Max / system Chrome, every three-second view held
the 120 Hz cap with zero dropped samples, no frame above 16.67 ms, no stale completions, settled queues,
and exact opaque/depth ownership. The worst frame p95 was 9.7 ms and the worst active GPU p95 was
5.967 ms. The six views exercised 1.13–1.63 million visible quads, 353–420 draw calls, regional cloud
lighting, water/refraction where present, and the full screen-space AO path without exceeding its
7.5 ms active-GPU gate.

Native Criterion means for the largest forest form were 300.77 us for a hero-intersecting canonical
crown chunk, 152.01 us for its stride-2 tile, and 179.96 us for its stride-16 tile. The representative
off-road and pilgrim-road chunks measured 716.96 and 669.67 us, remaining below 3% of the prior measured
class and well under the 1 ms generation budget. Codec and greedy-mesh cases remained within noise.
The first implementation put generalized hero anchor work on every candidate; the retained version
restores cheap established anchors for ordinary forms and computes the clamped wider anchor only for
the rare semantic hero.

## 2026-07-12: scalable authored-route queries

Before extending the pilgrim road, its five-segment linear projection was replaced by an immutable
Rust route index. Segment length, tangent, cumulative distance, and expanded corridor bounds are
computed once. A host reference test compares indexed and legacy sampling across the complete route
rectangle at 1.3 m spacing, verifies cumulative positions to 0.1 mm, and proves that every rounded
landmark station remains identical.

Criterion with runtime-black-boxed inputs measured 3.25 ns for global route rejection, 5.45 ns for a
point inside the global rectangle but outside every segment corridor, 8.64 ns for a near-segment
projection, 3.54 ns for cumulative-distance lookup, and 38.8–53.9 ns for the existing station-cell
lookup. The release browser profile then held steady, walking, and underwater frame p95 at 9.7, 9.5,
and 9.2 ms with zero drops, errors, stale completions, or residual jobs. This establishes a measured
route-sampling foundation before the authored path grows from 165 m to the planned 600 m class.

## 2026-07-12: 753 m terrain-authored pilgrimage

Generator v12 extends the road to 42 frozen nodes and 752.96 m. A deterministic Rust terrain search
threads four distinct regions and terminates 11.9 m from the alpine Needle Gate. Exact canonical replay
at 10 cm spacing proves dry paving, player-width clearance, no more than 30 cm cut or 20 cm fill, an
8.84% maximum node grade, canonical/chunk/edit agreement, and one cardinal-connected road component at
all four LOD strides. Landmark cadence expands from five to 26 stable route identities.

The runtime index now switches routes above 16 segments to sorted sparse 25.6 m spatial bins. Bin
memberships retain authored segment order, including shared-endpoint and self-crossing tie behavior.
Route stations are precomputed in ordinal and feature-cell order. The exhaustive LOD validator now
unions per-segment corridors instead of scanning the 22.4-million-column global rectangle. Host tests
cover an additional synthetic 819.2 m/256-segment route and exact indexed-versus-reference sampling.

The release browser remained at the 120 Hz cap after the extension:

| Integrated measurement                       |             Result |
| -------------------------------------------- | -----------------: |
| Steady / traversal / underwater frame p95    | 9.9 / 8.8 / 9.9 ms |
| Traversal CPU / streaming p95                |       5.7 / 3.1 ms |
| Worst six-hero frame / active GPU p95        |     9.6 / 5.484 ms |
| Dropped samples / stale completions / errors |          0 / 0 / 0 |

The exact generator-v12 build also completed 38 rapid reloads across three tabs with live edit
synchronization and zero persistence errors. Fault injection rejected 20 consecutive OPFS VFS
acquisitions and recovered on call 21 with zero console errors. Both gates reasserted that the body
contains exactly one canvas; crosshair, startup help, telemetry, statuses, menus, and toggles remained
Rust/WGPU draw data.

Atlas schema v1 subsequently froze six destination and five chapter IDs. Mission Control renders the
current Rust-projected chapter and whole-route percentage in the same WGPU header; no additional DOM,
TypeScript geography state, draw pass, or persistent terrain data was introduced.

## 2026-07-12: Cinder Vault vertical slice

Generator v13 adds the connected Cinder Vault without weakening its 10 cm edit authority; generator
v14 adds four sparse material-14 crystal formations. Host validation proves more than 180 path samples retain a
7x7x18-voxel player-clear volume, floor steps stay within 30 cm, the chamber is dry, the sealed shell
survives ambient cave noise, and random, chunk, region, edit, and versioned codec paths agree. The
material-detail atlas grows from 14 to 15 deterministic layers and 2,621,400 derived bytes.

`vp run profile:caves` drives the three-stop tour only through Rust canvas controls and records the
approach, descent, and chamber after streaming and eye adaptation settle:

| Integrated cave measurement                        | Approach / descent / chamber |
| -------------------------------------------------- | ---------------------------: |
| Frame p95                                          |           8.5 / 8.8 / 8.8 ms |
| Active GPU p95                                     |     3.853 / 6.082 / 6.299 ms |
| Enclosure                                          |            0 / 0.222 / 1.000 |
| Interior exposure                                  |            1 / 1.222 / 2.037 |
| Enclosure-probe p95                                |             0 / 400 / 100 us |
| Dropped samples / pending jobs / stale completions |                    0 / 0 / 0 |

The first probe implementation regenerated procedural columns along every ray and measured roughly
1.8 ms at the descent. Consulting authoritative edited resident chunks before the procedural fallback
reduced that p95 to 0.4 ms while preserving player-made openings. All three views stay under the
12 ms frame, 7.5 ms active-GPU, and 1 ms probe gates. The captured chamber shows the emissive crystal
surface cue under enclosure-correct ambient light; the renderer contains no cave-coordinate special
case and the cue remains editable world data.

Generator v15 adds the edit-aware 24-quad mouth tell without changing the watertight surface mesh.
Criterion measured the complete mouth-owning tile at 198.48 us for stride 2 and 199.63 us for stride 16. Host tests prove the four-box proxy is byte-identical at every level, owns one anchor, never
intersects protected cave Air, disappears after a canonical sentinel edit, and restores exactly.

After the proxy, a six-second-per-stop release run recorded 9.8 / 10.2 / 10.1 ms frame p95 and
2.643 / 3.420 / 3.545 ms active-GPU p95 across approach, descent, and chamber. All phases retained
zero dropped samples, pending jobs, pending mesh bytes, and stale completions; the enclosure-probe
p95 remained 0 / 300 / 100 us. Twenty-four GPU timestamp samples per stop make the 95th percentile
distinct from a single scheduling outlier while retaining the 12 ms frame and 7.5 ms GPU gates.

## 2026-07-12: bounded voxel-emissive lighting

Material emission is now ordinary derived world data rather than a Cinder Vault shader special case.
Meshing bins exposed emitters into deterministic 0.8 m cells, and the renderer selects no more than 16
active lights from successfully uploaded canonical chunks. Host tests cover exposed/buried clustering,
deterministic centroids and ordering, stable capped ranking, world-space conversion, GPU layout, and
the Rust placement palette. The release WGSL validator covers the fixed uniform and bounded loop.

`vp run profile:lights` disables the independent cave headlamp and measures matched six-second chamber
windows through the Rust Mission Control toggle:

| Cinder Vault chamber         | Local lights off | Local lights on |    Delta |
| ---------------------------- | ---------------: | --------------: | -------: |
| Frame p95                    |           8.9 ms |          8.9 ms |   0.0 ms |
| World GPU p95                |         1.154 ms |        1.298 ms | 0.144 ms |
| Active-window GPU p95        |         5.657 ms |        5.937 ms | 0.281 ms |
| Candidate / active / clipped |       10 / 0 / 0 |     10 / 10 / 0 |        — |

Both phases retained identical chunks, geometry, draws, and mesh allocation with zero dropped samples.
The regular three-stop cave gate independently observed 0 active lights on the approach and descent,
then 10 in the chamber, with 8.5 / 9.1 / 9.1 ms frame p95 and no pending or stale work. Candidate
selection remains visible while lighting is disabled, so the A/B proves a render-only cost rather than
changing residency.

The next exact-head run added authoritative camera-to-emitter DDA visibility, capped at 32 segment
tests before the 16-light GPU budget. The chamber retained all 10 connected lights and reported zero
occluded or clipped candidates. CPU p95 remained 3.7 ms and render-submission p95 moved from 3.2 to
3.3 ms across the off/on windows; frame p95 remained 8.9 ms with zero drops. A portable host test
proves that the shared DDA rejects rock before a target, accepts a target before the rock, and handles
a zero-length segment without leaving the 10 cm grid contract.

The follow-on portable visibility graph adds four host gates for deterministic geodesic routing,
closed-portal disconnection and reopening, stable equal-cost paths, and hard definition capacities.
It uses fixed arrays for 16 cells and 32 portals and performs no allocation per query. Portal-directed
streaming was integrated only after the lighting and durability gates were independently stable.

Cinder topology v1 adds three more host gates over its eight cells and seven portals: all authored node
centers classify stably and the 175-sample pristine evaluation leaves every portal open; closing one probe
plane disconnects only its edge and reverting restores it; and an exterior-to-chamber route both
requires the mouth and exceeds the current 6.4 m light-selection radius. The world suite now contains
101 tests without adding a persisted format or changing generator output.

`vp run profile:portals` visits a fourth Rust-owned Cinder tour stop on playable terrain directly above
the chamber. Radial streaming deliberately keeps the underground source chunks resident, creating the
adversarial case that Euclidean lighting gets wrong. The exact release result retained 10 candidates,
performed 10 bounded visibility queries, rejected all 10 by portal geodesic, and submitted zero active,
occluded, or clipped lights. All seven pristine portals remained open at revision zero.

The six-second overhead window measured 8.8 ms frame p95, 3.5 ms CPU p95, 3.1 ms render-submission
p95, 3.895 ms active-GPU p95, and 0.789 ms world-GPU p95 with zero dropped samples, pending work, or
stale completions. The chamber A/B then retained all 10 connected lights with zero portal rejection,
8.8 ms frame p95, and unchanged chunks, geometry, draws, and mesh allocation.

`vp run profile:portal-edits` adds an end-to-end durability gate without controlling the user's
browser. Rust places basalt in all 25 canonical mouth-probe voxels, reducing the leader and observer
tab from seven open portals to six. A fresh worker then hydrated all 25 sparse edits and reconstructed
six open portals at revision zero. Rust reverted the generated-air overrides, returned live state to
seven portals, and a second reload hydrated zero edits and the pristine seven-portal mask. The isolated
two-tab release run reported no OPFS, SQLite, worker, or WebGPU errors.

`vp run profile:portal-streaming` then gates the topology as a bounded streaming input. Far from
Cinder it requested and activated zero secondary chunks with 243 primary chunks tracked. Every open
approach, descent, chamber, and overhead phase requested, admitted, and atomically activated all 73
conservative cave chunks across 32 columns; tracking never exceeded 320, neither capacity layer
truncated, the world plan never overflowed, and no unreachable chunk remained active. Sealing the
mouth from outside dropped requested and active portal chunks to zero while keeping retained
allocations harmless; entering the sealed chamber restored its 73-chunk connected interior plan.

The final reopened chamber window measured 8.7 ms frame p95, 3.5 ms CPU p95, 0.1 ms streaming p95,
3.1 ms render-submission p95, and 5.451 ms active-GPU p95 at 1280x720. It recorded no frames above
16.67 ms, dropped samples, pending jobs, stale completions, truncation, or unreachable active chunks.
Runtime host coverage now also proves deterministic interest ordering, observable capacity loss,
same-focus interest replacement, atomic column readiness, and convergence of edits to retained
undesired chunks. Renderer coverage proves radial and portal activation reasons cannot disable one
another.

The material atlas simultaneously returned to nearest-neighbor sampling for the original pixelated
style. The sampler contract is host-tested as nearest magnification/minification/mip selection with
anisotropy 1, while retaining the averaged mip chain for distance stability. An isolated headless
release run held steady, traversal, and underwater frame p95 to 9.2 / 9.2 / 9.1 ms at 1280x720 with
zero dropped samples or browser errors.

The follow-up face-resolution gate quantizes both albedo and tangent normal/roughness lookup to three
cells per 10 cm voxel axis, or 3.33 cm per visible block. A host test covers all three texel centers,
exact and just-inside face boundaries, negative coordinates, nearest sampling, and the explicit-gradient
WGSL path. `vp run profile:materials` kept material-off/on frame p95 identical at 9.2 ms, moved world
GPU p95 by 0.239 ms, changed no geometry, residency, draws, or mesh allocation, and recorded zero
dropped samples or browser errors in isolated headless Chrome.

## 2026-07-13: local world-source Phase 0 baseline

Measured on an Apple M3 Max running macOS 26.5.2 with Rust 1.97.0. The working tree was based on
commit `75c7140`; Criterion used 100 samples after its standard warm-up. The benchmark commands write
only ignored artifacts under `target/criterion`; the browser profile uses a preview server on a
reserved ephemeral port and does not open or reset an existing browser origin:

```sh
vp run bench:world
vp run bench:runtime
vp run profile:browser
```

| Portable operation                                             | Criterion mean |            Derived throughput |
| -------------------------------------------------------------- | -------------: | ----------------------------: |
| Generate one procedural-v16 32³ chunk                          |      718.33 us |                1,392 chunks/s |
| Generate chunk plus deduplicated meshing halo                  |      1.0483 ms |                  954 chunks/s |
| Generate two chunk-plus-halo products in one batch             |      2.0574 ms | 486 batches/s; 972 products/s |
| Materialize the 6,536-cell meshing halo                        |      187.55 us |                 5,332 halos/s |
| Generate one 25.6 m far-surface tile                           |      165.24 us |                 6,052 tiles/s |
| Generate one edit-aware stride-8 water tile                    |      129.87 us |                 7,700 tiles/s |
| Encode source-bound VXCH v2                                    |      412.70 us |                  151.44 MiB/s |
| Decode and validate source-bound VXCH v2                       |      412.57 us |                  151.49 MiB/s |
| Populate the 243-chunk cold scheduler interest set             |       22.51 us |                 44,434 sets/s |
| Admit a mixed `{ generation: 2, meshing: 1, upload: 3 }` frame |       10.01 us |               99,900 frames/s |

The exact mesher envelope is the complete local `[-1, 32]` cube with the `[0, 31]` core removed:
6,536 unique material values, including AO edge and corner samples. Its logical payload is 13,072
bytes at the current `u16` material width. Six unpadded 32x32 planes are therefore insufficient.
At the settled 243-chunk browser working set, cores account for 15.188 MiB and halos add 3.029 MiB,
for 18.217 MiB of logical canonical voxel data. The release WASM heap committed 24 MiB in the same
steady-state capture.

VXCH v2 keeps the existing palette and bit packing. Encoded sizes for ordinary surface, midpoint Pilgrim
Road, water, Alpine Needle, and Cinder Vault representative chunks are respectively 8,304, 12,402,
4,204, 8,302, and 8,304 bytes. The five codec-independent canonical voxel hashes live in
`world/tests/procedural_v16_golden.rs`; codec v2 has a separate intentional envelope golden so a
future envelope version cannot masquerade as a generator change.

The isolated release browser reached its fully drained 243-chunk working set in 3.270 seconds. Steady,
traversal, and underwater frame p95 were 9.6, 9.3, and 9.2 ms respectively, with no frame above
16.67 ms, no dropped samples, no stale completions, and no browser errors. Traversal streaming work
measured 4.1 ms p95 while the mixed frame-admission budget remained bounded.

These are in-process baselines. The plan's 5% process-split threshold is intentionally not evaluated
against codec microbenchmarks; warmed service throughput, daemon RSS, queue wait, and transport
latency must be measured when the daemon exists.

The completed Phase 1 in-process boundary was re-profiled after collision, enclosure, raycast, and
light visibility stopped invoking the source from callbacks. Startup drained in 3.356 seconds;
steady, traversal, and underwater frame p95 measured 9.1, 9.2, and 9.2 ms. All three windows recorded
zero frames above 16.67 ms, zero dropped samples, zero stale completions, and zero browser errors.
Steady committed WASM remained 24 MiB; the underwater window reached 27.875 MiB after traversal and
the bounded teleport probes. Coast and underwater discovery use a bounded first-match surface-search
product, avoiding both per-column requests and a temporary 513x513 sample-grid allocation.
The 40-operation edit profile restored both fixtures exactly with zero remaining edits, in-flight
work, superseded operations, dropped samples, pending jobs, or browser errors; full-convergence p95
was 38.0 ms. The isolated two-tab portal gate sealed all 25 mouth voxels, observed six open portals
before and after reload, then restored seven portals and zero persisted edits after a second reload.
