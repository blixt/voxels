# Voxel Engine Research Survey

Date: 2026-03-11

I went through canonical papers, production-engine writeups, voxel engine docs, and current Chrome/WebGPU documentation. The strongest practical conclusion is that an editable voxel engine for modern Chrome/WebGPU should **not** start from a “pure sparse-voxel-raytracing everywhere” design. The safest and highest-value baseline is: **authoritative sparse world storage on the CPU, chunked updates, worker-based generation/streaming, near-field rasterized surfaces, and selective ray traversal only where it clearly wins**. SVO/SVDAG ray tracers are essential references, but they fit static or carefully-managed data much better than a world that is constantly edited. ([NVIDIA][1])

## The architecture I would start with in Chrome/WebGPU

1. **Use a sparse map of dense chunks as the authoritative world format.** In practice, chunked dense bricks are still the workhorse for editable worlds because they are simple to stream, simple to mutate, and simple to rebuild locally. Vercidium’s Sector’s Edge used 32³ chunks in a destruction-heavy game, and both Voxel Tools and Project Ascendant discuss the tradeoff between chunk size, draw-call count, culling granularity, and edit cost. ([vercidium.com][2])

2. **Choose the surface extractor by terrain style, not by fashion.** For blocky worlds, greedy meshing or a faster run-based variant is the practical baseline. For smooth terrain, store density/SDF and use Marching Cubes, Surface Nets, or Dual Contouring near the camera, then use Transvoxel to stitch across LOD boundaries without cracks. ([0 FPS][3])

3. **Treat LOD as a separate engineering problem, not a free byproduct of chunking.** Volumetric LOD is much harder than heightfield LOD, and naive downsampling breaks thin features and topology. For smooth terrain, Transvoxel is the standard seam solution; for blocky worlds, the 0fps POP-buffer / geomorph work is the most useful conceptual reference. ([0 FPS][4])

4. **Build an async edit pipeline early.** Dirty-region tracking, rebuild queues, multithreaded jobs, and region locking are not “optimizations”; they are core architecture for editable voxels. Voxel Plugin and Voxel Tools both document this explicitly, and Vercidium’s meshing work is a good illustration of how rebuild cost becomes the dominant concern in destructible worlds. ([docs.voxelplugin.com][5])

5. **Start with rasterized surfaces plus cheap voxel-local lighting.** Shadow maps, local AO, flood-fill sky/block lighting for block worlds, and possibly limited SDF/raymarched shadows are the pragmatic path. Full-scene GI via voxel cone tracing or compute ray tracing is real, but it is a second-phase feature after storage, streaming, and edits are stable. ([0 FPS][6])

6. **Use voxel-aware collision first, generic mesh physics second.** DDA/grid traversal for rays, specialized movers for player collision, and limited mesh-collider generation near players are much more practical than regenerating high-quality physics meshes everywhere. Voxel Tools and Voxel Plugin both push in this direction. ([Diglib][7])

7. **Design around actual WebGPU, not native-API wishful thinking.** Modern Chrome/WebGPU gives you compute, workers, storage buffers, subgroups, 3D compressed textures on supporting hardware, and read-write storage textures, but not standard hardware ray tracing or sparse residency. That pushes a voxel engine toward explicit chunk/brick pools, explicit indirection tables, and compute-based traversal rather than “just use sparse textures + DXR.” ([Chrome for Developers][8])

## 1) World representations: what each one is good for

**Sparse map of dense chunks** is the best first representation for an editable engine. It gives excellent locality, trivial mutation, easy serialization, and straightforward streaming. It maps cleanly to worker jobs on the CPU and to storage-buffer/texture uploads on the GPU. The downside is that it does not compress global redundancy nearly as well as SVO/SVDAG-style structures, so far-field memory pressure must be handled with LOD, compression, or eviction. ([vercidium.com][2])

**Sparse Voxel Octrees (SVOs)** are still foundational. Laine and Karras’s *Efficient Sparse Voxel Octrees* is the paper to study for compact node layout, contour information, normal compression, and efficient ray casting. It is one of the best references for understanding empty-space skipping and high-detail voxel traversal. The catch is that SVOs are much more natural for static or batched-updated content than for arbitrary fine-grained edits every frame. ([NVIDIA][1])

**Sparse Voxel DAGs (SVDAGs)** are the compression monster. Chalmers’s *High Resolution Sparse Voxel DAGs* shows that identical subtrees can be merged so aggressively that memory drops by 1–3 orders of magnitude while traversal stays efficient enough for ray-traced hard shadows, AO, and primary visibility. The downside is equally important: classic SVDAGs are not comfortable as a highly editable runtime world structure, and later work on interactive modification exists because that limitation is real. ([research.chalmers.se][9])

**Symmetry-aware DAG variants** are worth knowing but not worth building first. SSVDAGs extend the idea by reusing symmetric subtrees, which can compress further than ordinary SVDAGs. That is valuable if your endgame is static sparse ray tracing or offline-baked worlds, not if your immediate goal is a robust editable engine. ([publications.crs4.it][10])

**OpenVDB and NanoVDB** are extremely useful conceptual references. OpenVDB’s hierarchical sparse tree with tiles, leaf nodes, active/inactive voxels, and background values is one of the clearest real-world designs for sparse volumes. NanoVDB shows how to flatten sparse volumetric data into a more GPU-friendly form. Even if you never port these structures literally into WebGPU, the design ideas transfer directly to chunk pools, brick metadata tables, and sparse leaf allocation. ([OpenVDB][11])

**Voxel Hashing** is a better reference than many game-devs realize. It is particularly strong for sparse TSDF-style scenes, scanned worlds, or worlds where only observed regions should exist in memory. The key idea—store dense local bricks only where needed, back them with a hash structure, and stream aggressively—transfers well to editable sparse worlds and especially to distance-field-based systems. ([Chair of Visual Computing][12])

**What works best in WebGPU:** dense chunks, chunk maps, clipboxes, and explicit brick pools. **What is usually a phase-two system:** SVO/SVDAG ray-traced backends. **What is great as a design reference but not usually a first browser runtime structure:** OpenVDB/NanoVDB-class sparse trees. ([GitHub][13])

## 2) Rendering huge amounts of small voxels

The first hard lesson is that **“one voxel = one cube draw” is not a serious endpoint** once worlds get large. The mature choices are: extract surfaces and rasterize them, ray-traverse the voxel structure directly, or use a hybrid that does both depending on distance and content. The reason is simple: geometry count, bandwidth, and CPU/GPU setup cost become the bottleneck long before raw occupancy storage does. Project Ascendant explicitly describes geometry density and memory as the limiting factors, which is why it split rendering into multiple systems instead of treating every voxel as ordinary mesh geometry. ([0 FPS][3])

For **block worlds**, the 0fps meshing articles are still among the best practical references. The sequence from naive cube rendering to face culling to greedy quad merging is the clearest explanation of where the big wins come from. The AO follow-up is also valuable because it shows how to add cheap local ambient occlusion while still allowing greedy merges when the AO values match. ([0 FPS][3])

Vercidium is important because it shows that **triangle count is not the only metric that matters**. For a highly destructive game, they favored a run-based mesher because it regenerated chunks faster than a more general greedy approach. Their published optimization work reduced average chunk generation time from 5.15 ms to 0.89 ms. That is exactly the kind of trade that matters in an editable engine: a mesher with slightly worse compression can still be the correct choice if it rebuilds dramatically faster. ([vercidium.com][2])

For **smooth worlds**, the baseline choices are familiar but still worth separating clearly. **Marching Cubes** is the easiest to implement and parallelize. **Surface Nets** usually produce a more rounded, lower-complexity surface. **Dual Contouring** is the sharp-feature option because it uses Hermite data and QEF minimization on adaptive octrees. **Transvoxel** is then the essential companion if you want crack-free LOD transitions. ([Space Frontiers][14])

For a Chrome/WebGPU implementation, Will Usher’s WebGPU Marching Cubes articles are directly applicable. They show the exact GPU pattern that matters in practice: classify active voxels, prefix-sum counts, compact active cells, and emit triangles. They also discuss very WebGPU-specific realities such as splitting dispatches to respect compute-dispatch limits and using dynamic buffer offsets. On an RTX 3080, their WebGPU implementation was close to Vulkan on 256³ test sets, which is strong evidence that serious voxel compute pipelines are viable in Chrome. ([willusher.io][15])

**Direct voxel rendering without meshing** is not dead; it is just situational. ESVO is the classic static sparse-traversal reference. GigaVoxels is the classic large-data streaming reference. Majercik et al.’s ray-box work is the reference for the opposite extreme: fully dynamic scenes where every voxel may change every frame and precomputed acceleration structures are not acceptable. In a browser engine, this family of approaches is most attractive for special effects, far-field rendering, or research prototypes—not as the first end-to-end gameplay renderer. ([NVIDIA][1])

A particularly strong **hybrid** reference is Project Ascendant. It uses near-field quantized voxel meshes, then a different far-distance solution that raycasts per sprite/object because carrying far geometry literally becomes too expensive. That is one of the best transferable lessons from native engines to WebGPU: **near = mesh, far = something cheaper and more implicit**. ([Vulkan Guide][16])

## 3) LOD and streaming: the part that usually hurts the most

Volumetric LOD is harder than terrain LOD because the data is 3D, not 2D. 0fps’s volumetric terrain LOD series is still one of the best explanations of why standard mipmapping intuition fails here. The extra dimension makes bandwidth and simplification much more expensive, and naive volume reduction can change the actual topology of the surface. ([0 FPS][4])

The most important “what not to do” is **naive downsampling**. Nearest-neighbor downsampling can simply erase thin structures. Linear or trilinear filtering can produce scalar fields whose extracted isosurface is no longer a faithful simplification of the original. 0fps explores these failures directly, then looks at morphological filtering and later blocky-voxel approaches. Those articles are especially valuable because they explain not just one solution, but why some obvious-looking solutions are wrong. ([0 FPS][4])

For **smooth terrain**, Transvoxel is still the default answer for LOD seams. Its whole reason for existing is to stitch between adjacent voxel regions at different resolutions using only local data, so edits stay local and you do not need global retriangulation. Voxel Tools’ smooth-terrain docs are a practical companion here because they show the SDF + Transvoxel workflow as an actual engine feature rather than just a paper concept. ([Transvoxel][17])

For **blocky terrain**, the 0fps 2018 POP-buffer / stable-rounding posts are unusually useful. They discuss transition seams, skirts, and continuous rounding/geomorph strategies that avoid obvious popping. That material is valuable even if you do not copy the exact algorithm, because it teaches the right mental model: blocky LOD needs explicit seam thinking just as much as smooth terrain does. ([0 FPS][18])

For **streaming**, there are three reference families worth keeping separate. **Geometry clipmaps** are the classic mental model for nested, incrementally updated rings/grids. **Octree or clipbox streaming** is the more game-engine-friendly version used by practical voxel tools. **Ray-guided streaming** from GigaVoxels is the big sparse-volume research path. Voxel Tools’ docs are especially valuable because they compare octree streaming and clipbox streaming in concrete operational terms rather than as abstractions. ([CiNii Research][19])

In Chrome/WebGPU, the streaming conclusion is straightforward: **assume explicit software-managed streaming**. Sparse resources are still an investigation/open topic in GPUWeb, not a standard WebGPU feature, so a browser voxel engine should plan for chunk/brick pools, explicit indirection tables, and explicit upload/eviction logic. Architecturally, that points toward chunked clipboxes or octree/clipbox hybrids rather than “one giant virtual sparse 3D texture.” ([GitHub][13])

For offline or semi-static content pipelines, the out-of-core SVO construction papers are still worth reading. They matter if you plan to voxelize large meshes or scanned assets into sparse structures without requiring the entire scene to fit in memory at once. They are less relevant to your realtime browser runtime, but very relevant to asset baking. ([Diglib][20])

## 4) Realtime modification and destructibility

Editable voxels turn almost every elegant static data structure into a write-amplification problem. The practical pattern is dirty chunks, localized remeshing, async rebuild queues, and some form of spatial locking or job ownership over regions. Voxel Plugin explicitly documents read-only/read-write locks over spatial data, and Voxel Tools documents heavily multithreaded tasks and techniques to keep only the modified data resident when possible. ([docs.voxelplugin.com][5])

One of the most useful practical lessons is that **edit frequency changes the right meshing algorithm**. Vercidium’s work is worth studying because it shows a real engine choosing faster regeneration over theoretically prettier meshing. In other words: if your world changes often, remesh time is usually more important than perfect quad optimality. ([vercidium.com][2])

Voxel Tools’ smooth-terrain documentation adds another important point: in SDF terrain, you can often reduce storage and update cost by **quantizing or clamping values away from the surface** and by aggressively recognizing constant regions. That is exactly the right instinct for editable terrain—most of the world is not near the surface, so do not spend full precision everywhere. ([voxel-tools.readthedocs.io][21])

The compressed sparse ray-tracing family has a real edit problem. SVDAGs are superb for scale, but they were not originally intended as fast arbitrary-edit structures, which is why papers on interactively modifying compressed sparse voxel representations had to appear later. That makes them outstanding research references and poor first choices for a fully destructible browser sandbox. ([research.chalmers.se][9])

A different route is to avoid remeshing entirely and render voxels directly. Majercik et al.’s direct dynamic voxel rendering by ray tracing boxes is specifically about that case. It is an important counterpoint in the literature because it reminds you that “mesh everything” is not the only answer when the world changes constantly. ([NVIDIA][22])

## 5) Lighting, shadows, GI, and raytracing vs other methods

There is no single “voxel lighting” problem; there are at least three. First is **cheap local lighting for block worlds**. Second is **general direct lighting/shadows for meshed voxel terrain**. Third is **full or partial global illumination**. Those are very different problems and should not be mixed in one design phase. ([0 FPS][23])

For block worlds, the cheapest proven stack is still **flood-fill sky/block light + local ambient occlusion**. The 0fps AO article is still one of the cleanest practical references for local voxel AO, and the broader block-lighting tradition is fundamentally a BFS/flood-fill problem over neighboring cells. This class of solution is attractive precisely because it updates locally and plays well with editable worlds. ([0 FPS][6])

For direct visibility, shadows, picking, and bullet traces, **grid traversal is the key primitive**. Amanatides and Woo remains the foundational uniform-grid traversal paper, and Vercidium’s raymarching writeup shows how the same core idea maps into a chunked voxel engine with practical lookup optimizations. Even if you never do full-scene ray tracing, you almost certainly want DDA-style traversal somewhere in your engine. ([Diglib][7])

For ambitious GI, the core references are still **Crassin’s voxel cone tracing**, **CryEngine SVOGI**, and related clustered/SVO GI papers. These systems are powerful because once the world is voxelized in a useful way, diffuse GI, AO, and some glossy response become natural ray/cone queries. But they are expensive not just in shading cost, but in world-maintenance cost: you must keep the voxelized representation coherent enough for lighting to trust it. ([NVIDIA][24])

A very good modern production reference is **Raymarching The Gunk**. It shows a hybrid strategy: coarse voxels on the CPU for gameplay/collision, GPU generation of an SDF volume with jump flooding, clipmaps, raymarched rendering, low-resolution cone tracing, and custom shadowing. It is not WebGPU-specific, but it is highly transferable because it is exactly the kind of hybrid volumetric thinking that works well when a pure triangle or pure voxel solution is not enough. ([jarllarsson.github.io][25])

On **WebGPU specifically**, do not plan around hardware RT. The current standardized WebGPU feature set does not include a ray-tracing feature, and the ray-tracing extension is still an open GPUWeb issue. So “ray tracing in WebGPU” today means compute/fragment shader techniques, custom BVHs or grids, DDA over chunks, SDF ray marching, or compute path tracers built on ordinary compute shaders. ([GPU Web][26])

There are useful WebGPU ray-tracing examples, but they are examples of **custom compute pipelines**, not of a browser equivalent of DXR/Vulkan RT. gnikoloff’s WebGPU ray tracer and the “Practical ray traced GI on WebGPU” project are useful implementation references precisely because they show what is possible in Chrome with ordinary compute shaders today. ([GitHub][27])

So the practical recommendation is: **rasterize surfaces first, ray-trace selected effects later**. In a Chrome/WebGPU voxel engine, the first good lighting stack is usually direct lighting + shadow maps + local AO + maybe localized DDA/SDF shadowing. Full-scene GI or path tracing makes sense after the content, streaming, and edit systems are already healthy. ([0 FPS][6])

## 6) Collision detection, rigid bodies, navigation, and fluids

For **ray tests**, the answer is simple: study Amanatides and Woo, then implement it on your chunk structure. That covers picking, bullet traces, occlusion checks, and often simple AI line-of-sight. Vercidium’s raymarch article is a nice practical companion because it explains the chunk-local lookup optimizations that matter in a real engine. ([Diglib][7])

For **character movement on blocky terrain**, specialized voxel-aware collision is usually much better than generic triangle-mesh collision everywhere. Voxel Tools explicitly notes that mesh colliders are static/concave, are more prone to tunneling because they have no thickness, and can cost roughly 3x–5x more to generate than the visible mesh because the physics engine builds its own acceleration structures. Their `VoxelBoxMover` exists to avoid those costs on blocky terrain. ([voxel-tools.readthedocs.io][28])

Voxel Plugin’s docs make the operational version of the same point: **only generate collisions and navmesh where needed**. Tying collision/nav generation to invokers or visible chunks is a strong practical pattern because it caps the amount of heavy geometry you ask the runtime to maintain. ([docs.voxelplugin.com][29])

For **navigation and smooth collision**, TSDF/ESDF systems are very relevant. nvblox is a strong reference because it builds surface and Euclidean signed distance fields on the GPU, and those ESDFs are exactly what you want for nearest-obstacle distance, obstacle inflation, and planner-friendly queries. Even if your engine is a game and not a robot stack, the data structure ideas are transferable. ([nvidia-isaac.github.io][30])

For **rigid-body broad phase**, voxel engines still benefit from ordinary broad-phase algorithms. GPU Gems 3 remains a good reference for sort-and-sweep and spatial subdivision on the GPU. The right split is usually: standard broad-phase for moving objects, voxel/grid-specific narrow-phase or terrain contact once a candidate pair reaches the terrain. ([NVIDIA Developer][31])

For **fluid or smoke-style voxel physics**, keep the computation and the rendering data on the GPU as much as possible. The fluid/voxel thesis you found is especially useful because it documents the failure mode clearly: CPU round-trips for 3D textures and activation data became the bottleneck, while a GPU-only design improved performance dramatically. For sparse high-end fluid work, NVIDIA’s sparse-volume FLIP/GVDB references are the right next step. ([Springer Nature Link][32])

## 7) Chrome/WebGPU reality check for voxel engines

The good news is that **modern Chrome/WebGPU is strong enough for real voxel engines**. Chrome’s WebGPU rollout is no longer experimental in the old sense, and the platform has gained useful features such as subgroups, 3D block-compressed textures on supporting hardware, and read-write storage textures. That makes serious compute-heavy voxel pipelines plausible in the browser. ([Chrome for Developers][33])

The bad news is that **you must design around the actual standardized feature set**. There is still no standard hardware ray-tracing feature in WebGPU. Sparse resources are still an open/investigative area, not a standard feature. Bindless-adjacent resource models are still evolving, and multi-draw indirect in Chrome has been exposed experimentally behind `Unsafe WebGPU`, not as something you should casually assume in a shipping baseline. ([GitHub][34])

That leads to very concrete design consequences for a voxel engine:

* **Use storage buffers heavily.** Chrome’s WebGPU guidance points out that storage buffers are vastly larger than uniforms, and MDN’s supported-limits docs give conservative floor values such as a 128 MiB storage-buffer binding size and a 256 MiB max buffer size. Chunk metadata, brick headers, indirect-argument buffers, draw lists, and traversal stacks belong in storage buffers, not in uniform buffers. ([Chrome for Developers][35])

* **Assume conservative 3D texture budgets.** MDN’s limits are tiered for privacy rather than exact hardware disclosure, and the portability floor for `maxTextureDimension3D` is conservative. That means you should not build a browser voxel engine around giant monolithic 3D textures. Use chunked/brick atlases or multiple pooled volumes instead. ([MDN Web Docs][36])

* **Put generation and streaming in workers.** WebGPU interfaces are available in Web Workers, which makes off-main-thread chunk generation, meshing preparation, and streaming coordination the obvious architecture for Chrome. A browser voxel engine that does heavy world work on the main thread is choosing pain. ([MDN Web Docs][37])

* **Feature-detect subgroups and use them, but keep a fallback path.** Subgroups are now available and are exactly the kind of feature voxel engines can exploit for scans, reductions, and compaction-style kernels. But because feature availability is adapter-dependent, you should not make them a hard requirement for correctness. ([Chrome for Developers][38])

* **3D compressed textures are now interesting for static bricks and far-field data.** Chrome added sliced 3D BC and ASTC compressed texture support on supporting hardware. That makes compressed brick volumes, SDF clipmaps, or read-mostly far-field data more realistic than they were a few years ago, although you still need a fallback for unsupported adapters. ([Chrome for Developers][39])

* **Use queries and profiling early.** WebGPU has occlusion queries and feature-gated timestamp queries. In voxel engines this matters because the bottleneck is often bandwidth, overdraw, traversal divergence, or hidden rebuild cost, not just visible triangle count. ([MDN Web Docs][40])

The single most important WebGPU-specific design rule is this: **build a chunk/brick engine with explicit CPU/GPU resource management, not a fantasy engine that assumes sparse residency, bindless everything, and hardware RT.** That rule alone will keep the architecture realistic. ([GitHub][13])

## 8) What tends to work, and what tends not to

**Usually works well**

* Sparse map of dense chunks for authority and edits. ([vercidium.com][2])
* Greedy or run-based meshing for block worlds. ([0 FPS][3])
* SDF + Marching Cubes/Surface Nets/Dual Contouring + Transvoxel for smooth terrain. ([voxel-tools.readthedocs.io][21])
* DDA traversal for rays, picking, bullets, some shadows. ([Diglib][7])
* Async chunk rebuilds, worker jobs, localized collision/nav generation. ([docs.voxelplugin.com][5])
* Hybrid near-mesh / far-implicit rendering. ([Vulkan Guide][16])

**Usually fails or becomes painful**

* One cube per occupied voxel as the long-term renderer. ([0 FPS][3])
* Naive scalar-field downsampling for LOD. ([0 FPS][4])
* Treating SVDAG-class compression as a free editable-world runtime structure. ([research.chalmers.se][9])
* Depending on mesh colliders everywhere for voxel terrain. ([voxel-tools.readthedocs.io][28])
* Architecting around sparse GPU resources or hardware RT in WebGPU. ([GitHub][13])
* CPU round-trips for volumetric simulation data. ([Springer Nature Link][32])

## 9) The highest-value reading order

**Foundations**

1. **Efficient Sparse Voxel Octrees** — the classic SVO ray-tracing/traversal paper. Read it for data layout, traversal, contour data, and empty-space skipping. ([NVIDIA][1])
2. **High Resolution Sparse Voxel DAGs** — the canonical compression follow-up to SVOs. Read it to understand how far compression can go for mostly static data. ([research.chalmers.se][9])
3. **Interactively Modifying Compressed Sparse Voxel Representations** — read this immediately after SVDAGs so you internalize why static sparse compression and realtime edits are in tension. ([TU Delft Research Portal][41])
4. **Amanatides & Woo: fast voxel traversal** — still the core ray primitive for voxel engines. ([Diglib][7])

**Block worlds and practical meshing**
5. **0fps meshing in a Minecraft game** and the follow-ups — the clearest practical explanation of why culling/greedy meshing matter. ([0 FPS][3])
6. **0fps ambient occlusion for Minecraft-like worlds** — cheap voxel-local shading that actually fits editable worlds. ([0 FPS][6])
7. **Vercidium’s Sector’s Edge meshing and raymarch writeups** — read these for destructibility-driven tradeoffs, fast chunk rebuilds, and practical DDA in chunked worlds. ([vercidium.com][2])

**Smooth terrain**
8. **Transvoxel** — the standard seam-fixing reference for smooth-terrain LOD. ([Transvoxel][17])
9. **Dual Contouring of Hermite Data** — sharp-feature adaptive octree surfaces. ([WashU Medicine Research Profiles][42])
10. **Voxel Tools smooth terrain docs** — an excellent engine-level companion to the papers, especially for SDF handling and Transvoxel usage. ([voxel-tools.readthedocs.io][21])
11. **Will Usher’s WebGPU Marching Cubes** — the most directly transferable GPU meshing reference for Chrome/WebGPU. ([willusher.io][15])

**LOD, streaming, huge data**
12. **0fps volumetric LOD series** — invaluable because it explains failure modes, not just algorithms. ([0 FPS][4])
13. **Geometry clipmaps** — still the right mental model for incremental multiresolution streaming. ([CiNii Research][19])
14. **GigaVoxels** — classic ray-guided streaming of massive sparse data under tight memory. ([Inria Sophia Antipolis][43])
15. **Voxel Hashing** — the sparse-hash/TSDF reference that transfers well to sparse runtime worlds. ([Chair of Visual Computing][12])

**Lighting and advanced rendering**
16. **Crassin’s voxel cone tracing** and **CryEngine SVOGI docs** — the core GI references for voxelized scenes. ([NVIDIA][24])
17. **Raymarching The Gunk** — a modern production example of hybrid voxel/SDF rendering ideas that transfer well. ([jarllarsson.github.io][25])
18. **Direct dynamic voxel rendering by ray tracing boxes** — the key counterexample to “always remesh.” ([NVIDIA][22])

**Browser-specific**
19. **Chrome WebGPU docs, GPUFeatureName list, GPUWeb issues** — read these alongside your architecture work so you do not design around nonexistent browser features. ([Chrome for Developers][8])
20. **MDN WebGPU limits and worker support docs** — these tell you what your portability floor actually looks like. ([MDN Web Docs][36])

The next productive step is to turn this survey into a concrete Chrome/WebGPU architecture spec: chunk format, GPU buffer layout, worker job graph, meshing path, LOD path, and a staged roadmap from “editable block world” to “hybrid voxel renderer.”

[1]: https://research.nvidia.com/publication/2010-02_efficient-sparse-voxel-octrees-analysis-extensions-and-implementation "https://research.nvidia.com/publication/2010-02_efficient-sparse-voxel-octrees-analysis-extensions-and-implementation"
[2]: https://vercidium.com/blog/voxel-world-optimisations/ "https://vercidium.com/blog/voxel-world-optimisations/"
[3]: https://0fps.net/2012/06/30/meshing-in-a-minecraft-game/ "https://0fps.net/2012/06/30/meshing-in-a-minecraft-game/"
[4]: https://0fps.net/2012/08/20/simplifying-isosurfaces-part-2/ "https://0fps.net/2012/08/20/simplifying-isosurfaces-part-2/"
[5]: https://docs.voxelplugin.com/1.2/technical-notes/performance-and-profiling.html "https://docs.voxelplugin.com/1.2/technical-notes/performance-and-profiling.html"
[6]: https://0fps.net/2013/07/03/ambient-occlusion-for-minecraft-like-worlds/ "https://0fps.net/2013/07/03/ambient-occlusion-for-minecraft-like-worlds/"
[7]: https://diglib.eg.org/items/60c72224-00f3-416d-9952-ee41e8c408da/full "https://diglib.eg.org/items/60c72224-00f3-416d-9952-ee41e8c408da/full"
[8]: https://developer.chrome.com/docs/web-platform/webgpu/overview "https://developer.chrome.com/docs/web-platform/webgpu/overview"
[9]: https://research.chalmers.se/en/publication/182658 "https://research.chalmers.se/en/publication/182658"
[10]: https://publications.crs4.it/pubdocs/2017/JMG17/ "https://publications.crs4.it/pubdocs/2017/JMG17/"
[11]: https://www.openvdb.org/documentation/doxygen/ "https://www.openvdb.org/documentation/doxygen/"
[12]: https://www.lgdv.tf.fau.de/publications/real-time-3d-reconstruction-at-scale-using-voxel-hashing/ "https://www.lgdv.tf.fau.de/publications/real-time-3d-reconstruction-at-scale-using-voxel-hashing/"
[13]: https://github.com/gpuweb/gpuweb/issues/455 "https://github.com/gpuweb/gpuweb/issues/455"
[14]: https://spacefrontiers.org/r/10.1145/37401.37422 "https://spacefrontiers.org/r/10.1145/37401.37422"
[15]: https://www.willusher.io/graphics/2024/04/22/webgpu-marching-cubes/ "https://www.willusher.io/graphics/2024/04/22/webgpu-marching-cubes/"
[16]: https://vkguide.dev/docs/ascendant/ascendant_geometry/ "https://vkguide.dev/docs/ascendant/ascendant_geometry/"
[17]: https://transvoxel.org/ "https://transvoxel.org/"
[18]: https://0fps.net/2018/03/ "https://0fps.net/2018/03/"
[19]: https://cir.nii.ac.jp/crid/1361699994101884800 "https://cir.nii.ac.jp/crid/1361699994101884800"
[20]: https://diglib.eg.org/items/8c45ccdd-b384-42d1-8d09-3d85f456e40c "https://diglib.eg.org/items/8c45ccdd-b384-42d1-8d09-3d85f456e40c"
[21]: https://voxel-tools.readthedocs.io/en/latest/smooth_terrain/ "https://voxel-tools.readthedocs.io/en/latest/smooth_terrain/"
[22]: https://research.nvidia.com/labs/rtr/publication/majercik2018raybox/ "https://research.nvidia.com/labs/rtr/publication/majercik2018raybox/"
[23]: https://0fps.net/category/programming/ "https://0fps.net/category/programming/"
[24]: https://research.nvidia.com/publication/2011-09_interactive-indirect-illumination-using-voxel-cone-tracing "https://research.nvidia.com/publication/2011-09_interactive-indirect-illumination-using-voxel-cone-tracing"
[25]: https://jarllarsson.github.io/gen/gunkraymarcher.html "https://jarllarsson.github.io/gen/gunkraymarcher.html"
[26]: https://gpuweb.github.io/types/types/GPUFeatureName.html "https://gpuweb.github.io/types/types/GPUFeatureName.html"
[27]: https://github.com/gnikoloff/webgpu-raytracer "https://github.com/gnikoloff/webgpu-raytracer"
[28]: https://voxel-tools.readthedocs.io/en/latest/performance/ "https://voxel-tools.readthedocs.io/en/latest/performance/"
[29]: https://docs.voxelplugin.com/2.0p8%20-%20v2/design-docs/collision-mesh-navmesh "https://docs.voxelplugin.com/2.0p8%20-%20v2/design-docs/collision-mesh-navmesh"
[30]: https://nvidia-isaac.github.io/nvblox/index.html "https://nvidia-isaac.github.io/nvblox/index.html"
[31]: https://developer.nvidia.com/gpugems/gpugems3/part-v-physics-simulation/chapter-32-broad-phase-collision-detection-cuda "https://developer.nvidia.com/gpugems/gpugems3/part-v-physics-simulation/chapter-32-broad-phase-collision-detection-cuda"
[32]: https://link.springer.com/article/10.1007/s40869-016-0020-5 "https://link.springer.com/article/10.1007/s40869-016-0020-5"
[33]: https://developer.chrome.com/blog/webgpu-release "https://developer.chrome.com/blog/webgpu-release"
[34]: https://github.com/gpuweb/gpuweb/issues/535 "https://github.com/gpuweb/gpuweb/issues/535"
[35]: https://developer.chrome.com/docs/web-platform/webgpu/from-webgl-to-webgpu "https://developer.chrome.com/docs/web-platform/webgpu/from-webgl-to-webgpu"
[36]: https://developer.mozilla.org/en-US/docs/Web/API/GPUAdapter/limits "https://developer.mozilla.org/en-US/docs/Web/API/GPUAdapter/limits"
[37]: https://developer.mozilla.org/en-US/docs/Web/API/GPUDevice "https://developer.mozilla.org/en-US/docs/Web/API/GPUDevice"
[38]: https://developer.chrome.com/blog/new-in-webgpu-134 "https://developer.chrome.com/blog/new-in-webgpu-134"
[39]: https://developer.chrome.com/blog/new-in-webgpu-139 "https://developer.chrome.com/blog/new-in-webgpu-139"
[40]: https://developer.mozilla.org/en-US/docs/Web/API/GPUQuerySet "https://developer.mozilla.org/en-US/docs/Web/API/GPUQuerySet"
[41]: https://research.tudelft.nl/en/publications/interactively-modifying-compressed-sparse-voxel-representations "https://research.tudelft.nl/en/publications/interactively-modifying-compressed-sparse-voxel-representations"
[42]: https://profiles.wustl.edu/en/publications/dual-contouring-of-hermite-data/ "https://profiles.wustl.edu/en/publications/dual-contouring-of-hermite-data/"
[43]: https://www-sop.inria.fr/reves/Basilic/2009/CNLE09/ "https://www-sop.inria.fr/reves/Basilic/2009/CNLE09/"
