# Mutable voxel worlds: 2026 research update

Research date: **2026-09-12**. Sources below were accessed on that date. This is a research and
implementation recommendation, not a new renderer, a benchmark reproduction or a deployment.
Repository findings refer to main at `05c3b1e` and the retained
volume branch at `d666770`.

This supplements the [March survey](20260311-voxel-research.md) and
[March engine-performance notes](20260311-voxel-perf-research.md). The capability audit is in
[Latest Chrome and wgpu](20260912-webgpu-wgpu-research.md).

## Current implementation baseline

The post-rewrite browser journey passed on Apple M3 Max with Chrome `153.0.8010.37`: default
spawn, sustained spectator flight, walking, jump, dig, placement, screenshot replay and restore.
The latest run, with a one-ray direct-traversal probe dispatched through every frame encoder after
pipeline initialization, used the generated v17 world and the real world-service protocol. Across
240 steady samples, frame time was 17.687 ms mean / 21.665 ms p95, CPU time 6.386 ms mean / 13.334
ms p95, and GPU time 8.992 ms mean / 9.634 ms p95. Peak core GPU allocation was 215.49 MiB;
exact-quality debt and missed terrain progress were both zero. The latest six-client shaped-40 ms RTT
multiplayer run applied 5,000
authoritative voxel placements to all clients with zero protocol errors; the distant tower converged
in 4.468 s at 109.7 m separation, with observer frame p95 33.4 ms and builder frame p95 50.1–66.8 ms.
These are acceptance baselines, not claims of the 1080p/120 FPS target. Artifacts are kept under
`target/automation/player-rendering/` and `target/automation/multiplayer/`.

## What changes our direction

The target is a fully 3D world of **10 cm edge-length cubes**, including floating islands,
elaborate caves, player constructions and many players editing the same visible area. Heightfields
are not the foundation, and predominantly stable terrain is not a valid design assumption.
World population alone does not determine client cost: visible mutation, simulation activity,
replication fan-out and the density of players in one area must be measured separately.

**Use direct voxel traversal as the default implementation path, with dense local bricks and
small, conservatively maintained occupancy summaries.** Preserve one authoritative voxel/edit
model, stable resident GPU slots and updates proportional to affected data. Extracted surfaces,
distance fields, global compression and rich lighting caches must justify their total cost under
the intended mutation workload. This is an engineering choice for this product, not a claim that
ray tracing always beats rasterization or that a universally optimal brick size has been found.

The current evidence supports that direction, with useful qualifications:

- Grimorium's graphics explainer is from **September 11, 2026**, not an old engine description.
  It directly explains choosing voxel traversal for heavily changing simulation data.
- Lay of the Land's July updates expose real simulation/render-update costs and overlapping-update
  and unload-race bugs. Recent engines still have to solve mutation scheduling and memory ownership.
- A detailed 2026 Solari post reports that some sophisticated light indexes cost more than they
  saved. The user's concern about overbuilding metadata is supported by implementation experience.
- The 2026 NAADF paper demonstrates fast mutable voxel traversal, but also broad cache
  invalidation in some cases, substantial temporal memory, and delayed indirect lighting.

The older papers are useful for algorithms and counterexamples. Their publication age does not
make DDA, dense bricks or local updates obsolete; their workload assumptions often make them
insufficient evidence for our multiplayer target.

## 2026 creator posts and transcripts

### Grimorium: direct traversal of simulation data

**Burkelbear Games, [How graphics work in my 3D Noita-Like game](https://www.youtube.com/watch?v=Y8HRCXxI0BY),
September 11, 2026, 14:00:02 UTC.**

Access: the complete user-supplied transcript, plus primary video metadata and description.
The [official Steam page](https://store.steampowered.com/app/4415850/Grimorium/) links the
Burkelbear Games channel; video metadata supplies the title and date. This identifies the
baseline transcript already supplied by the user, rather than pretending it is a newly obtained
second transcript. No engine source or independent benchmark was available in this review.

| Transcript time | Technique described                                                                                                                                                   | Relevance                                                                                                                                                  |
| --------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1:50–3:47       | Compute rays traverse the same voxel data used by simulation; multilevel chunk occupancy flags accelerate DDA; a debug view shows work per ray                        | Directly addresses remeshing under fluids, gases, falling material and destruction. Measure traversal and summary maintenance together.                    |
| 3:47–4:28       | Transparent voxels accumulate along a ray; gas is treated as density; distant voxel rings use 2× and 4× voxel sizes                                                   | Volumetric rendering and LOD do not require a heightfield. The ring scheme is an implementation example, not a requirement for our streaming architecture. |
| 5:29–6:55       | Material-ID shading, local corner AO, one aggregated point light per chunk octant, and receiver-local light lists; secondary rays test sun and local-light visibility | A concrete route to mesh-independent emissive lighting with a modest spatial summary. Aggregation cost and approximation errors still need budgets.        |
| 7:02–7:28       | Coordinate-seeded grass, leaves and vines are intersected procedurally inside a voxel                                                                                 | Fine visual detail can avoid another stored geometry representation. Its per-ray cost must be measured.                                                    |
| 7:59–8:28       | One-third internal width and height, camera jitter and temporal antialiasing                                                                                          | The example traces **one ninth** of the output pixel count. Reconstruction quality and actual internal resolution are essential benchmark fields.          |

The creator identifies a laptop RTX 4060 and says increasing internal resolution starts to hurt
performance. The description identifies a custom C++ engine. Neither the transcript nor the
description establishes browser WebGPU, a specific output resolution, a measured FPS distribution,
or simultaneous multiplayer editors. The claimed millions of changing voxels per second describe
the intended simulation workload without an accompanying benchmark table. This source strongly
supports the design rationale; it does not establish our 1080p/120 FPS target.

Two additional current videos are worth revisiting, but **only their metadata/descriptions were
read**, so they are reading leads rather than algorithm evidence:

- [Simulating Hundreds of Millions of Micro-Voxels in Real-Time in a 3D Noita-Like Engine](https://www.youtube.com/watch?v=BySRC4HwLYg),
  March 31, 2026. Description chapters cover parallel physics, chunking, destruction, island formation
  and streaming. The title does not establish that every listed voxel changes every frame.
- [Realistic voxel world destruction and rigidbody physics in my 3D Noita-like Engine!](https://www.youtube.com/watch?v=mWdlTZ_FoBc),
  April 25, 2026. The description says detached terrain becomes rigid bodies. No collision or
  fracture algorithm is inferred from chapter headings. The September transcript says the physics
  system has since changed, so details of the April implementation may be superseded.

### Lay of the Land: mutation costs and failure cases after release

The following are complete official Steam development/patch posts. Dates were checked against
Steam news timestamps; authors are the official accounts named below. These posts disclose
production behavior, not a new rendering algorithm.

| Date          | Source and author                                                                                                              | New, relevant evidence                                                                                                                                                                                                                         |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| July 27, 2026 | [Development Update & 1.1.15 Patch Notes](https://store.steampowered.com/news/app/2776090/view/674001285755701888), Tooley1998 | Voxel simulation and world creation were the two main performance pain points. Same-seed generation on a Ryzen 7 7800X3D decreased from about 2 minutes to 40 seconds. Large moving-voxel workloads improved but remained slower than desired. |
| July 3, 2026  | [1.1.10 Patch Notes](https://store.steampowered.com/news/app/2776090/view/679628250084806656), southerncrossinteractive        | Reports substantial voxel-simulation render-update improvements and reduced CPU/GPU memory use. No method or timings are disclosed.                                                                                                            |
| July 4, 2026  | [1.1.11 Patch Notes](https://store.steampowered.com/news/app/2776090/view/679628250084807006), southerncrossinteractive        | Fixes a crash when several voxel simulation updates affected the same area during one frame.                                                                                                                                                   |
| July 14, 2026 | [1.1.14 Patch Notes](https://store.steampowered.com/news/app/2776090/view/710028996470900382), southerncrossinteractive        | Smoother physics-object splitting, no rendering of object voxel simulations outside simulation update range, and a fix for regions stepping simulations while unloading.                                                                       |

These are strong additions to our test workloads: overlapping mutations within one tick,
eviction during in-flight region work, and bursts of moving voxels. They do not prove multiplayer
capacity or identify an optimal data structure. The world-generation improvement must not be
reported as an FPS or render-time improvement.

### Teardown: contemporary multiplayer destruction, with a bounded scale

**Official Teardown news, Marcus,
[Greenwash Gambit DLC is now playable in Multiplayer!](https://store.steampowered.com/news/app/1167630/view/670621050832162948),
August 18, 2026.** Access: complete official announcement and its publication metadata.

The full DLC campaign and all four maps now support up to **12 players**; the developers recommend
**2–4** for the best experience. Campaign tools and anti-gravity crystals work in multiplayer.
This is useful shipped evidence that voxel destruction and unusual rigid-body gameplay can coexist
with multiplayer. The party-size recommendation is not an identified technical capacity limit, and
the post provides no sustained-edit throughput or MMO result.

For architecture, the current [scripting API](https://teardowngame.com/modding/api.html) exposes
voxel shapes in body-local coordinates, transforms, sleeping, splitting and joints; the
[multiplayer documentation](https://teardowngame.com/modding-mp/index.html) separates server authority
from client presentation. These are current living documents, not newly dated 2026 blog posts.
They support retaining a volume/object boundary: move a detached body's transform instead of
rewriting all of its voxels into the static world lattice every physics tick.

## Two 2026 lighting implementation posts

These are adjacent lighting references, **not voxel-engine or multiplayer benchmarks**. They add
concrete implementation tradeoffs beyond older GI papers.

### Webgiya: surfel-based GI in actual browser WebGPU

**Jure Triglav, [Surfel-based global illumination on the web](https://juretriglav.si/surfel-based-global-illumination-on-the-web/),
January 29, 2026.** Access: full article, linked [implementation](https://github.com/jure/webgiya)
and README. The [demo](https://jure.github.io/webgiya/) was not benchmarked locally.

The system computes reusable illumination on surface patches, using a cascaded 3D grid, software
rays against a CPU-built triangle BVH, temporal lighting estimates, and visibility information
to reduce leaking. The article's comparison of roughly 50,000 surfels with two million image
pixels is illustrative, not a controlled performance measurement.

The relevant implementation cost is unusually explicit: **seven spatial-grid passes and four
surfel-lifecycle passes**, before lighting integration and resolve; the full system has 13+ compute
passes. Array textures and merged buffers help with binding limits. Short- and long-term lighting
estimates improve response to changing light, but thin geometry and fine detail remain difficult.

The article explicitly limits this implementation to **static geometry**, diffuse indirect light,
one directional light and an environment map. It excludes emissives, transparency and glossy/specular
reflections; fast-moving lights can break it. The named-device result is author-reported approximately
60 FPS for Cornell Box on iPhone 14/mobile Safari, without a stated rendering resolution. Sponza is
discouraged on that device. This is not a demonstration of editable geometry or our Chrome target.

**Use:** a later bounded diffuse-lighting-cache reference, including its debugging views and pass
accounting. If we try surfels, query voxel bricks directly rather than adding triangle conversion
and BVH maintenance to every voxel edit. That integration is our proposal, not a Webgiya feature.

### Solari: when light indexing and temporal caches stop paying for themselves

**JMS55, [Realtime Raytracing in Bevy 0.19](https://jms55.github.io/posts/2026-04-12-solari-bevy-0-19/),
April 12, 2026.** Access: full creator writeup, code excerpts and linked
[Solari implementation](https://github.com/bevyengine/bevy/tree/main/crates/bevy_solari).

The most relevant findings are the negative results:

- Light-tree construction/traversal overhead was too high for this application. Per-cache-cell
  alias tables sampled cheaply but cost too much to rebuild for the quality improvement.
- A simpler fixed world-space grid of contributing lights was promising and cheap to build,
  but remained a prototype: overflow, fixed world extent and camera movement were unresolved.
- Stochastically updating a target of 40,000 world-cache cells reduced Bistro's cache-update pass
  from **2.65 ms to 0.47 ms**, at the expense of lighting responsiveness. This is a component timing,
  not total frame time, and the target is an expected workload rather than a hard maximum. Hardware
  and resolution are not specified alongside those timings in the readable article.
- Reducing cache-cell size and forcing finer cache levels for short rays reduced corner leaking.
  This reinforces the need to test thin cave walls and recently opened holes, rather than assuming
  a lighting cache is valid because primary visibility is correct.
- Reflections need suitable reconstruction guides: reflected geometry may matter more than the
  mirror's own depth/normal. Its DLSS Ray Reconstruction path is not available as an assumed Chrome
  feature; the transferable lesson is guide-buffer design and history validity.

Solari uses native experimental ray-query and binding-array capabilities. This is encouraging
evidence of substantial rendering work in the **native wgpu ecosystem**, not evidence that its
hardware-RT path is exposed in WebGPU. A wgpu fork cannot add an API absent from Chrome.

**Use:** small local light candidates and immediate direct visibility first. Add richer indirect
caches only after measuring construction, maintenance, queries and response latency together.

## A genuinely new 2026 voxel paper and implementation: NAADF

**Annalena Ulschmid, M. Ott, J. Macho, Michael Wimmer and Stefan Ohrhallinger,
[NAADF: Globally Illuminated Voxel Worlds Accelerated with Nested Axis-Aligned Distance Fields](https://www.cg.tuwien.ac.at/research/publications/2026/ulschmid-2026-naadf/),
Computer Graphics Forum / Eurographics, May 2026, DOI [10.1111/cgf.70413](https://doi.org/10.1111/cgf.70413).**
Access: [full paper](https://www.cg.tuwien.ac.at/research/publications/2026/ulschmid-2026-naadf/ulschmid-2026-naadf-paper.pdf)
and author code at
[`3ea3bb6a836aa4a30277c055f812d65a9d7d6766`](https://github.com/cg-tuwien/NAADF/tree/3ea3bb6a836aa4a30277c055f812d65a9d7d6766).
The retained branch's August research already named this paper. The following update adds the
actual maintenance costs, benchmark context and a source/paper scheduling discrepancy.

The representation uses three layers: **4³ voxels per block and 4³ blocks per chunk**. Empty cells
cache an axis-aligned empty cuboid, allowing rays to skip further than a single DDA cell.
Uniform cells and repeated blocks reduce storage. Crucially, the basic hierarchy can be traversed
while acceleration is conservative and incomplete; background work improves skipping later.
This is a useful example of acceleration as optional cached work, rather than a condition for
rendering current geometry.

The impressive ray-throughput comparisons are not the whole engineering result:

| Evidence                 | What the paper/code actually establishes                                                                                                                                                                                                            |
| ------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Hardware/API, §5         | Ryzen 9800X3D, RTX 3090 Ti with 24 GB VRAM, native MonoGame/DirectX 11 compute; editing/entity logic on CPU. No hardware ray-tracing API is required, but this is not a WebGPU benchmark.                                                           |
| GI pipeline, Table 5     | At 1440p in San Miguel, **13.45 ms** with denoising and 0.25 secondary samples/pixel, versus **27.83 ms** at 1 secondary sample/pixel. These sample rates concern secondary lighting, not quarter-resolution primary geometry.                      |
| Temporal memory, Table 4 | The 32-sample TAA component uses **973 MB** and **1.42 ms**, versus 59 MB/0.16 ms for the compared standard TAA. Do not attribute this memory just to voxel storage.                                                                                |
| Editing, §3.5            | An empty/non-empty chunk transition can require resetting AADFs in a surrounding **63³-chunk volume**. Flood filling coalesces overlapping invalidation and operates in 4³-chunk groups. This is not the cost of every material-only voxel edit.    |
| Dynamic entities, §3.6   | Non-aligned local voxel volumes use transforms and per-chunk entity references. Movement can also invalidate empty-space acceleration. Rigid transforms do not make all world-index work disappear.                                                 |
| Limitations, §5.3        | Extensive editing or many entities can become CPU/synchronization limited. Indirect lighting has about **32 frames of noticeable temporal lag**, especially for sun-shadow changes and edited light sources; missing sample validation contributes. |

There is also an instructive implementation detail. Section 3.3 describes processing one queue
per frame. The inspected
[`WorldBoundHandler.Update`](https://github.com/cg-tuwien/NAADF/blob/3ea3bb6a836aa4a30277c055f812d65a9d7d6766/NAADF/World/Data/WorldBoundHandler.cs#L88)
instead runs **five** prepare/indirect-dispatch iterations per update, with
`maxGroupBoundDispatch = 512 * 64` by default. A per-dispatch budget is not a total edit-to-frame
latency guarantee, and queued recomputation does not make preceding CPU invalidation free.

**Use:** borrow conservative reset, traversal without fully rebuilt acceleration, local-coordinate
data and pooled updates. A large-radius distance cache and 32-frame lighting history should not
become our baseline by default. First measure whether a smaller occupancy summary already removes
most empty-space work. If testing AADFs, include concentrated edits, cold caches, reset work,
memory traffic and time to recover traversal efficiency. The paper contains no MMO throughput
result. Its out-of-core extension is future work, so its large resident scenes do not prove our
streaming architecture either.

## What we keep, and what we build next

Use current **main** as the base. The branch is useful source material, not a whole-engine upgrade
to merge indiscriminately.

| Existing work                                                                                                             | Decision                                                                                                                                                         |
| ------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Main authoritative dense chunks, materials, durable/idempotent edits, authentication, binary networking, reconnect/resync | Keep. They provide the real multiplayer/edit path; do not build a second world service for the renderer.                                                         |
| Main collision, movement, tools and avatars                                                                               | Keep. Server pose/reach admission is not full authoritative collision simulation.                                                                                |
| Main material detail, sky, atmosphere, HDR and tone mapping                                                               | Reuse the shading behavior for direct voxel hits. Existing entry points depend on raster inputs and need integration.                                            |
| `blixt/adversarial-volumetric-v1-mainline` at `d666770`                                                                   | Selectively port arbitrary 3D fixtures, conservative occupancy/discovery semantics, brick encoding ideas and exact hit/boundary oracle tests.                    |
| Branch global directory lifecycle                                                                                         | Preserve reference-capture guarantees, but replace this as a live-render prerequisite: any intervening global edit can retire its directory cohort.              |
| Branch packed GPU atlas publication                                                                                       | Replace whole-atlas allocation/copying with stable resident slots and changed-range publication. Bounded copy calls do not mean work is proportional to an edit. |
| Main meshes                                                                                                               | Retain as a working fallback during integration. The new path must pass with mesh generation disabled; do not make new lighting depend on remeshing.             |
| `blixt/explorable-island-world` at `3162d0c`                                                                              | Keep gameplay/scenario ideas; do not port the old TypeScript engine infrastructure wholesale.                                                                    |
| `daily-improvement-20260729` at `0d9392d`                                                                                 | Do not port the superseded staging experiment; its mechanism has been removed from main.                                                                         |

The volume branch uses protocol/config versions **50/30**, while the audited main uses **44/26**.
Selective integration must deliberately version any affected messages or formats. The critical
branch code is `world-service/src/virtual_terrain.rs::issue_exact_volume_publication_under_directory`
and `render/src/exact_volume_gpu.rs::upload_directory_candidate_internal`, inspected at `d666770`.

The next integrated milestone should be a **lit, fully volumetric multiplayer playground**:

1. Port caves, detached islands, hollow rooms, thin constructions and emissive-block fixtures onto
   main using a dedicated versioned source and temporary world data.
2. Render its resident chunk/edit stream directly through addressable GPU bricks. Produce hit depth,
   material, normal and identity so avatars and shared shading compose correctly.
3. Coalesce edits by brick, update bounded ranges and dependent summaries, reject late snapshots,
   and protect reused slots from in-flight readers. A remote edit must not invalidate unrelated
   local geometry. Missing data must never certify empty space.
4. Add sunlight visibility, inexpensive local AO and occluded local emissive lighting, preserving
   material detail and exposure. Derive light candidates from dirty bricks. Main currently has
   mesh-derived emission and a global 16-light ceiling; neither is the long-term lighting design.
5. Exercise two real clients and then a reproducible concentrated editor load through the actual
   service. Include same-cell conflicts, edits across brick boundaries, eviction while updates are
   pending, reconnect, persistence and edits far from the observer.

Main's 1,024-session admission and 1,000-player presence test are not proof of that many active
editors. Its X/Z interest cells should become fully 3D subscriptions for stacked caves and islands.
Measure accepted changes, actual modified voxels, replication bytes and edit-to-visible latency;
rejected attempts and idle connections do not count as editing throughput.

For every acceleration cache, report saved traversal/shading work against build/update time,
allocation, upload/copy bytes, query overhead, peak memory and response latency. Distinguish local
primary geometry/direct-shadow freshness from budgeted indirect-light convergence. A removed wall
must become visible without waiting for its indirect illumination to converge.

Keep **1920×1080 / 120 FPS** as the long-term target. Report named hardware, exact Chrome version,
physical output resolution, internal sample resolution, lighting quality, p95/p99 frame times and
missed presentation frames under sustained editing. A loading screenshot, a fast individual pass
or a one-ninth-pixel image is not achievement of that target. These are acceptance requirements,
not performance claims for code not yet built.

## Integrated browser evidence (12 September 2026)

Main now has a bounded direct-traversal slice rather than a dispatch-only probe. Chrome
**153.0.8010.37** on an Apple M3 Max generated a 320x180 camera-ray image from resident 8³ bricks,
composited over the certified mesh/page scene, and completed the existing player scenario with
240 samples at **17.906 ms mean / 22.177 ms p95** frame time and **4.663 ms mean / 7.209 ms p95**
GPU time. The same build passed the six-client 40 ms RTT scenario: five builders placed 5,000
authoritative voxels, every client converged, and the observer accepted the tower at 109.819 m in
4,667.7 ms. This is evidence that the migration can coexist with mutable multiplayer publication;
it is not evidence of full-resolution direct shading or 120 FPS.

After adding traversal-derived face normals, directional shading, GPU-generated camera rays, and
batched direct-brick eviction, a fresh run remained clean: **17.348 ms mean / 20.597 ms p95** frame
time and **4.538 ms mean / 7.078 ms p95** GPU time. The multiplayer travel/edit run also passed with
**4,702.8 ms** tower convergence at **109.712 m**, with zero browser or protocol-control errors.

After the integrated path works, add bounded diffuse bounce lighting and then glass/reflection
reconstruction. For Teardown-style physics, use body-local voxel volumes plus transforms and
replicate motion separately from geometry edits/fracture. That is a substantial later simulation
and networking feature; it does not need to delay the lit editable-world milestone.

## Older references retained for specific reasons

- [HashDAG (2020)](https://github.com/Phyronnaz/HashDAG) and
  [its supplement](https://diglib.eg.org/server/api/core/bitstreams/d58628d5-75aa-4a8a-82d7-269543a3bfba/content)
  demonstrate that compressed DAGs can be edited and discuss the cost of incoherent compressed
  reads. The main PDF was inaccessible in this review; no unread benchmark table is being cited.
- [Editing Compressed High-resolution Voxel Scenes with Attributes (2023)](https://repository.tudelft.nl/file/File_6f87e286-d7ce-4d4a-89e2-92c36519467a)
  accounts for attribute-maintenance cost and describes large edits as interactive rather than
  real-time. It is evidence to cost compression, not to ban all mutable compressed structures.
- [GVDB (2016)](https://www.ramakarl.com/pdfs/2016_Hoetzlein_GVDB.pdf) and
  [Voxel Hashing (2013)](https://niessnerlab.org/papers/2013/4hashing/niessner2013hashing.pdf)
  support dense local storage under sparse indexing. Their volume/reconstruction workloads do not
  prescribe an optimal game brick size or establish editor capacity.

## Freshness and coverage limits

This was a focused search of creator/author sites, official game news, public repositories and
dated discovery links; it is not an exhaustive census. Source records distinguish full text,
supplied transcript, living documentation and metadata-only leads. No third-party demonstration
was benchmarked locally, and full copyrighted transcripts/PDFs are not copied into the repository.

The [Octo public README](https://github.com/DouglasDwyer/octo-release) explicitly identifies its
engine release as **August 2024** and outdated. Recent repository activity does not make it a 2026
engine writeup. The newest post found on [Vercidium's blog](https://vercidium.com/blog/) was from
2022; its 2026 footer is not a publication date. The useful
[sparse 64-tree tutorial](https://dubiousconst282.github.io/2024/10/03/voxel-ray-tracing/) is from 2024. They remain useful references without being counted as newly published research.

A Voxile creator interview returned HTTP 403 and was not used; no claims were inferred from its
headline. Full transcripts for the two additional Grimorium videos were not obtained. None of
the reviewed sources proves 10–100× Minecraft player concurrency in an unrestricted 10 cm world.
