Yes. I went through the strongest public materials I could find for **Teardown**, **Lay of the Land**, **Douglas Dwyer’s Octo**, **John Lin’s voxel project**, and the canonical papers and engine writeups that explain the underlying techniques.

The main conclusion is very clear: **a 60 FPS traversable voxel world at roughly 10 cm resolution is never “just a voxel renderer.”** It is always a bundle of systems: sparse or object-local storage, aggressive empty-space skipping, localized invalidation after edits, multiple representations for rendering vs physics vs lighting, LOD/streaming, data-oriented multithreading, and strict content-authoring constraints. Teardown’s public material is the clearest proof: it uses **thousands of smaller voxel volumes instead of one giant world volume**, does **CPU voxel-vs-voxel collision**, renders on the GPU by **rasterizing a bounding box per object and raymarching that object’s voxel grid**, and keeps a **separate occlusion structure** for ray-based lighting/shadowing. ([Game Developer][1])

At 10 cm resolution, the memory math alone forces this architecture. A **2 km cube** at **0.1 m voxels** is **20,000³ ≈ 8 trillion voxels**; even at **1 byte per voxel**, that is about **8 TB** before lighting, physics, or metadata. That is why the successful engines are **chunked, object-local, sparse, or hybridized**, not monolithic.

## The highest-value engine-specific materials

These are the public sources I would read first, in this order.

* **Teardown / current shipping-engine design:** the Game Developer interview, Dennis Gustafsson’s **“From screen space to voxel space”**, **“The Spraycan”**, **“Teardown quicksave”**, and the official modding docs. Together they reveal most of the practical architecture: local volumes, GPU raymarching of object bounds, separate occlusion data, bit-packed voxel occupancy, palette-indexed materials, acceleration structures regenerated from authoritative voxel data, and hard asset-authoring rules that directly affect performance and physics stability. ([Game Developer][1])

* **Dennis Gustafsson’s 2024 “Year summary”** is the best public description of where this design is heading next: hardware RT prototype, lock-free task manager, **sparse 8×8×8 voxel chunks** instead of dense per-shape grids, and a substepped parallel physics solver. Treat this as **future-direction material**, not as a literal description of current Teardown. ([Voxagon Blog][2])

* **Matt Tooley / Lay of the Land:** the public Patreon and devlog snippets confirm the most important runtime ideas: **32³ voxel meshes with LOD-dependent world size**, **same-LOD chunk merging** to cut draw calls, **ISPC** speedups for bulk voxel queries/generation, **smooth LOD transitions**, and repeated optimization of “thousands of moving voxels.” The Steam page also states explicit 60 FPS targets. ([Patreon][3])

* **Douglas Dwyer / Octo:** the public repo confirms a browser/WebGPU-capable engine with **ray marching**, **LODs**, **editable terrain**, **path-traced lighting features**, and **task-level + data-level multithreading**. That makes it a very useful “browser-targeted Teardown-adjacent” reference. ([GitHub][4])

* **John Lin:** public material clearly shows a **path-traced voxel renderer** that he said became **20× faster** and reached **60 FPS HD** in at least one milestone. However, I could **not** verify the exact internal claims from public primary sources for things like **SVDAGs, ASVGF, ReSTIR, or exact voxel mip/trilinear filtering**. For John Lin specifically, I would use his devlogs as visual proof of what is possible, then study the canonical papers for the underlying methods instead of assuming a one-to-one match. ([YouTube][5])

## What 60 FPS voxel engines at this scale have in common

### 1) They do not store “the world” as one giant dense grid

Teardown explicitly says it is built from **thousands of smaller volumes** rather than one huge aligned volume. Its current engine uses a **dense regular 3D grid per shape**, and Dennis says that forced artists to split assets so empty space would not waste memory. His 2024 summary explains that the next engine moves to **sparse 8×8×8 voxel chunks tracked by a 3D bitmap**, specifically because it saves memory, keeps updates fast, and makes many algorithms more efficient. ([Game Developer][1])

The official Teardown modding docs expose the same reality from the content side. They recommend **not** using objects at the 256³ maximum because it can cause stutters; **128³** is generally okay; objects with lots of empty space should be **split into several thinner objects** because otherwise the engine wastes time traversing empty space; and side-connected voxels are required for stable connectivity. That is not “just modding advice”; it is a direct public window into the runtime performance model. ([teardowngame.com][6])

The broad implementation lesson is: **use dense local storage where you need edit speed, but wrap it in a sparse outer structure.** Teardown’s current engine is object-local dense; Dennis’s next engine is sparse-chunked; Lay of the Land uses 32³ LOD-scaled meshes; Sector’s Edge also uses 32³ chunks; Project Ascendant uses chunk-sized draw units plus a giant preallocated GPU buffer. ([Voxagon Blog][2])

### 2) They separate authoritative voxel data from render, lighting, and physics accelerators

One of the most important Teardown references is the quicksave article. Dennis says the engine **does not save** some derived structures, including a **separate physics representation** and **spatial acceleration structures for physics, culling, rendering and lighting**, because those are reproducible from the main voxel data and get regenerated at load time. That is an extremely strong design clue: the live engine uses multiple representations, and only one of them is the durable “truth.” ([Voxagon Blog][7])

Teardown’s rendering path reinforces that split. The engine uses a **separate voxel structure for 3D occlusion data**, and because that structure has no color, reflections use **SSR for reflection color** instead of full path-traced material lookups. Primary visibility works by **rasterizing a bounding box for each object** and raymarching the object’s visual grid in shader, with **palette lookups** for material attributes. ([Game Developer][1])

That same split shows up in other engines. Lay of the Land publicly emphasizes chunk merging for rendering, ISPC for generation/query work, and separate simulation optimization passes. Project Ascendant splits close-range meshed voxels from far-range cheaper voxel rendering. Octo’s public changelog shows it switched rendering approaches over time while keeping editability and physics features. ([Patreon][3])

The practical rule is: **one representation for truth, others for speed**.

### 3) There are two main rendering families, and successful engines often hybridize them

The first family is **object-local volume rendering / raymarching / ray tracing**. Teardown is the clearest public example. It raytraces a separate occlusion volume, rasterizes object bounds, and raymarches per-object voxel grids. Dennis’s 2018 post shows the same lineage in more detail: he traced rays in a large voxel-space texture, updated it with `glTexSubImage3D`, stored **one-bit voxel occupancy** in an octant-packed layout, used **mip levels** to accelerate ray tracing, and combined **screen-space** and **voxel-space** tracing so short-range detail came from screen space while long-range visibility came from voxel space. He reports AO, lighting, fog, and reflections in about **9 ms at 1080p on a GTX 1080** in that scene. ([Game Developer][1])

The second family is **surface extraction and rasterization**. Lay of the Land’s public material points to chunked surface rendering plus merged same-LOD render units. Sector’s Edge is the best openly documented destruction-heavy example: it uses **32³ chunks**, regenerates meshes constantly, and explicitly rejected standard greedy meshing because it was not fast enough for their workload, opting for a faster run-based approach that better balanced rebuild cost and triangle count. Project Ascendant uses **near-field voxel meshes** and a different far-distance voxel renderer because close-range surface quality and far-range cost are different problems. ([Patreon][3])

The strongest practical takeaway is not “pick one forever.” It is: **hybridize by distance and workload**. Near-field editable terrain often wants meshed surfaces or high-fidelity local raymarching. Far-field terrain wants cheaper representations, larger voxels, or sprite/raycast-style impostors. Project Ascendant says this outright with different geometry systems for near voxels, far voxels, vegetation, and regular meshes. ([Vulkan Guide][8])

### 4) Empty-space skipping is a first-class feature, not a nice-to-have

Teardown’s 2018 rendering writeup is unusually explicit here. The shadow texture stores **eight neighboring voxels per byte**, each bit representing an octant of a **10 cm cube**. Zero means the whole region is empty, and Dennis says that fact is directly exploited to speed up ray tracing. He also keeps mip levels for **20 cm** and **40 cm** to start coarse and refine only on hits. ([Voxagon Blog][9])

That exact style of optimization appears in modern open references too. **Binary Greedy Meshing** builds a binary occupancy mask, then visible-face masks, and merges quads using **bitwise operations**; the author notes the occupancy data is also useful for **physics and raycasting**. **VoxelRT** compares several acceleration structures including **2-level DDA**, brick maps, and bitmask-guided skipping over multi-level grids. A Godot voxel prototype explicitly documents a **2-level DDA** renderer and even lists “one bit per voxel texture to speed up skipping empty voxels” as the next step. ([GitHub][10])

For traversal itself, **Amanatides and Woo** remains the canonical reference for fast voxel traversal, and Teardown’s post adds a very useful real-world nuance: **exact/watertight traversal** matters for some rays, but approximate/fixed-step marching can be acceptable for effects like fog or AO. Dennis explicitly says he uses both, depending on the use case. ([diglib.eg.org][11])

So the rule here is simple: **if you are checking voxels one-by-one without hierarchy, masks, or DDA, you are leaving huge performance on the floor.**

### 5) Draw-call control and GPU-driven batching matter as much as voxel math

Matt Tooley’s public 2024 optimization note is a perfect example. He says the game had become **CPU bottlenecked on the render thread** because of huge draw counts, especially in shadow passes, and that merging nearby same-LOD voxel meshes into larger chunks drastically improved frame pacing and GPU utilization. ([Patreon][3])

Project Ascendant makes the same point from a different angle. The guide says the two main GPU bottlenecks for voxel engines are **memory usage** and **geometry density**, explains why far-distance meshes explode memory/triangle counts, and then shows a very GPU-driven solution: preallocated large buffers, suballocation, packed voxel draw structs, and **GPU indirect culling** because CPU culling for hundreds of thousands of chunks would not scale. ([Vulkan Guide][8])

There are also very practical open references on the raster side. Nick McDonald’s **vertex-pooling** writeup shows why chunked voxel worlds should avoid naïve “one VAO/VBO per chunk” thinking in favor of persistent memory pools and indirect draws, and **Binary Greedy Meshing** shows a renderer that uses **vertex pulling** and **one draw call** for all chunks. ([nickmcd.me][12])

So even if your renderer is fundamentally “about voxels,” you still need ordinary engine disciplines: batching, indirect draws, culling, buffer reuse, and compact vertex formats. Otherwise the engine dies on submission overhead before the voxel math gets a chance.

### 6) LOD and streaming are mandatory, and seam quality is a separate problem

Lay of the Land’s public devlog snippets show exactly why this matters: smooth LOD transitions were added specifically to make traversal feel better and reduce visible pop-in. ([Games in Progress][13])

Teardown’s 2018 post also demonstrates a form of multi-resolution logic, with lower-resolution mip levels in the voxel occlusion structure used to accelerate long rays. That is not full terrain LOD by itself, but it is the same principle: **coarse first, refine when needed**. ([Voxagon Blog][9])

For the general problem, 0fps’s volumetric LOD writeup is still one of the best explanations of why voxel LOD is hard: naive downsampling often destroys or distorts isosurfaces, while clipmap-style thinking is promising but more demanding in 3D than in heightfields. For smooth terrain, **Transvoxel** remains the standard seam-fixing reference for crack-free transitions between LOD levels. ([0 FPS][14])

Project Ascendant is the best modern engineering example of distance-based hybridization: close terrain is meshed, far terrain is drawn with a cheaper voxel representation, and the author explicitly frames the problem as an O(n³)-style explosion in geometry with draw distance. ([Vulkan Guide][8])

The rule is: **LOD is not optional at 10 cm resolution**, and **crack-free LOD transitions require dedicated work**.

### 7) Physics is not “render the same voxels again in PhysX”

Teardown publicly states that collision is **voxel versus voxel on the CPU**. Its modding docs are even more revealing: side-contact matters for connectivity, edge/corner contact is not enough, and certain hard/soft material combinations create pathological fragment behavior when damaged. Those are exactly the kinds of rules you only discover when physics is deeply integrated with voxel topology rather than being a generic mesh-collider afterthought. ([Game Developer][1])

Dennis’s 2024 summary then shows the next step in that evolution: a new solver with **substepping instead of solver iteration**, a **parallel solver** for large piles, and broad-phase/contact improvements, explicitly citing recent Box2D/physics research and Temporal Gauss-Seidel-style ideas. ([Voxagon Blog][2])

The engine-doc side says the same thing. **Voxel Plugin** recommends generating collision and nav data only where needed, often around invokers or visible chunks, because voxel worlds are too large to support full-quality collisions/nav everywhere. **Voxel Tools** goes further: mesh collider creation can be **3–5×** more expensive than meshing itself, and for blocky terrain it provides a **VoxelBoxMover** specifically because game-style cube collisions are faster and avoid some tunneling issues. ([docs.voxelplugin.com][15])

The correct mental model is:

* **topology-aware voxel collisions** for terrain and connectivity,
* **specialized movers/rays** for the player and gameplay tests,
* **selective collider/navmesh generation** only where actors need it,
* **separate rigid-body/debris representations** when fragments become dynamic. ([Game Developer][1])

### 8) Dynamic edits must be local, asynchronous, and cheap to invalidate

Sector’s Edge spells out the dynamic-edit problem very clearly: with destruction-heavy gameplay, chunks may need regeneration nearly every frame, and meshing speed becomes a first-order design constraint. ([vercidium.com][16])

Lay of the Land’s public notes match that pattern. ISPC was adopted to speed bulk voxel queries and reduce caching, same-LOD chunk merging attacked draw-call pressure, and thousands-of-voxels simulation was refactored repeatedly for performance. Octo’s changelog shows the same theme in a smaller engine: chunk loading priority fixes, new voxel data structures, multithreading, and later a ray-marching rewrite that reintroduced editing and LODs after performance work. ([Patreon][17])

Teardown’s quicksave post shows the best abstraction here: save/load the authoritative state, and rebuild the derivative accelerators. That only works if derivative structures are **local enough** and **cheap enough** to regenerate in pieces. ([Voxagon Blog][7])

So for an editable 10 cm world, your invalidation unit should be **small and explicit**: chunk, subchunk, object volume, palette row, mip brick, collision cell, whatever fits your renderer. Whole-world rebuilds are not viable.

### 9) Lighting at 60 FPS is mostly about choosing the right compromise

Teardown is a beautiful example of disciplined compromise. It uses ray-based voxel occlusion/shadowing, but **SSR for reflection color** because the occlusion structure doesn’t contain color. Dennis’s 2018 writeup also shows the hybrid logic in more detail: short-range detail can come from screen-space tracing, long-range visibility from voxel-space tracing, and soft shadows or blurry reflections are not just prettier but also hide block artifacts better. ([Game Developer][1])

For blocky engines, the classic low-cost path is still local voxel AO and flood-fill style lighting. 0fps’s AO article remains the clearest reference for the cheap per-vertex/block-local ambient term, and the 2018 voxel-lighting writeup explains flood-fill lighting, multiple light channels, and bit-level acceleration of propagation. ([0 FPS][18])

For true path-traced or heavily ray-based voxel rendering, the public literature is unambiguous: **denoising and sample reuse are mandatory**. **SVGF** was a foundational result for getting temporally stable images from one-path-per-pixel style input; **ReSTIR** showed how to reuse and resample direct-light candidates for huge many-light scenes; and the later **GRIS / ReSTIR PT** work extended the theory to path-traced reuse with many-bounce lighting. ([NVIDIA][19])

That is why I would treat John Lin-like “60 FPS path-traced voxels” as a **denoiser + reuse + traversal** problem, not just a data-structure problem. His public video proves the performance milestone; the canonical papers explain how such a milestone is generally achieved. ([YouTube][5])

### 10) Content-authoring rules are part of the engine architecture

Teardown’s public modding docs are unusually explicit here:

* do not build giant all-in-one objects when they contain lots of empty space,
* keep objects reasonably sized,
* make sure voxels connect by sides, not just edges/corners,
* avoid trapping soft materials between hard materials because it creates physics problems. ([teardowngame.com][6])

This is one of the best public clues to how 60 FPS voxel engines really work: they are not generic containers for arbitrary voxel art. They rely on **authoring conventions that preserve culling efficiency, traversal efficiency, and fragment stability**. If your content pipeline ignores those rules, runtime performance and physics quality collapse together. ([teardowngame.com][6])

## How your example ideas map to solid reference material

Some of the ideas in your summaries are absolutely the right things to study, but not all are equally well verified as engine-specific facts.

**Strongly verified and directly supported by public sources**

* **Teardown-style object-local dense volumes**, separate occlusion data, GPU raymarching of rasterized bounds, palette-indexed materials, bit-packed occupancy/mips, and CPU voxel-vs-voxel collision. ([Game Developer][1])
* **Lay of the Land:** 32³ voxel meshes, draw-call reduction by merging same-LOD meshes, ISPC voxel query/generation speedups, smooth LOD transitions, thousands-of-voxels simulation optimization, and explicit 60 FPS hardware targets. ([Patreon][3])
* **Octo:** compute/ray-marched voxel rendering, LODs, editable terrain, path-traced lighting features, browser/WebGPU target, multithreaded physics. ([GitHub][4])

**Good techniques, but not fully verified as engine-specific facts from public primary sources I found**

* For **Douglas Dwyer**, I could verify ray marching, compute, LODs, editable terrain, browser/WebGPU, and physics. I could **not** independently verify the exact public-summary claims about **low-resolution depth prepass**, **specific bitmask layout**, or **temporal upscaling/denoising** as definite Octo internals. Those are still good techniques to study, but I would not attribute them to Octo without stronger sources. ([GitHub][4])
* For **John Lin**, I could verify the 20× speedup / 60 FPS HD milestone, but not the exact claims that he specifically uses **SVDAGs, ASVGF, ReSTIR, or a particular voxel mipmapping path**. So below I recommend the canonical papers for those topics rather than treating them as confirmed John Lin internals. ([YouTube][5])

## The most useful references by subsystem

### Teardown-style object-local voxel rendering

* **Game Developer interview with Dennis Gustafsson** — best single summary of current Teardown architecture. ([Game Developer][1])
* **From screen space to voxel space** — best detailed public writeup on the raytraced occlusion/shadow side, bit-packing, mips, hybrid screen/voxel tracing, and performance numbers. ([Voxagon Blog][9])
* **The Spraycan** — invaluable for palette-indexed materials, one-byte voxels, and GPU palette lookup strategy. ([Voxagon Blog][20])
* **Teardown quicksave** — best proof that render/physics/culling/lighting accelerators are separate from authoritative voxel data. ([Voxagon Blog][7])
* **Teardown modding docs** — the best public source on object sizing, empty-space splitting, connectivity, and material-mix constraints. ([teardowngame.com][6])

### Dynamic ray traversal / box intersections / direct voxel rendering

* **Amanatides & Woo** — still the core DDA traversal reference. ([diglib.eg.org][11])
* **A Ray-Box Intersection Algorithm and Efficient Dynamic Voxel Rendering** — best reference for fully dynamic box-based voxels and fast ray-box intersection on GPU. ([NVIDIA][21])
* **VoxelRT** — practical comparison of flat-grid DDA, multi-level DDA, brick maps, occupancy masks, distance-field skipping, ESVO, and BVH hybrids. ([GitHub][22])
* **Godot GPU DDA voxel renderer** — compact modern reference for 2-level GPU DDA. ([GitHub][23])

### Meshing, draw-call reduction, and destruction-friendly surface rendering

* **0fps: Meshing in a Minecraft Game** — the canonical greedy-meshing reference. ([0 FPS][24])
* **Sector’s Edge / Voxel World Optimisations** — the best public example of choosing faster rebuilds over theoretically prettier meshing in a destructible game. ([vercidium.com][16])
* **Binary Greedy Meshing** — practical bitwise occupancy masks, face masks, fast meshing, and vertex pulling. ([GitHub][10])
* **Nick McDonald: Vertex Pooling** — strong reference for chunk batching, persistent GPU memory pools, and indirect rendering in voxel worlds. ([nickmcd.me][12])
* **Project Ascendant geometry guide** — excellent modern “hybrid near-mesh / far-cheaper-voxel” rendering reference. ([Vulkan Guide][8])

### LOD, streaming, and seam handling

* **0fps: Simplifying Isosurfaces Part 2** — still one of the best explanations of why volumetric LOD is hard and naive filters fail. ([0 FPS][14])
* **Transvoxel** — the standard seam solution for smooth voxel terrain. ([transvoxel.org][25])
* **GigaVoxels** — foundational for ray-guided streaming, high-quality filtering, and huge-data sparse voxel rendering. ([www-sop.inria.fr][26])
* **Lay of the Land seamless LOD transitions** — practical proof that traversal feel depends heavily on transition quality, not just raw LOD existence. ([Games in Progress][13])

### Physics, collision, and simulation

* **Teardown current/future public materials** — voxel-vs-voxel CPU collision today; substepped parallel solver direction tomorrow. ([Game Developer][1])
* **Voxel Plugin collision/nav docs** — selective collision/nav generation near invokers or visible chunks. ([docs.voxelplugin.com][15])
* **Voxel Tools performance docs** — why collider generation is so expensive and why specialized blocky collision movers exist. ([voxel-tools.readthedocs.io][27])
* **Small Steps in Physics Simulation** — strong reference for the substepping direction Dennis mentions. ([mmacklin.com][28])

### Lighting, denoising, and path tracing

* **Teardown’s hybrid screen/voxel tracing posts** — practical hybrid lighting/shadow/reflection logic. ([Voxagon Blog][9])
* **0fps ambient occlusion + voxel lighting posts** — excellent low-cost lighting references for block worlds. ([0 FPS][18])
* **SVGF** — foundational real-time path-tracing reconstruction paper. ([NVIDIA][19])
* **ReSTIR** — direct-light reuse for many dynamic lights. ([NVIDIA][29])
* **GRIS / ReSTIR PT** — path-traced sample reuse for many-bounce lighting. ([NVIDIA][30])

### Sparse voxel hierarchies and compression

* **Efficient Sparse Voxel Octrees** — foundational SVO traversal/data-layout reference. ([NVIDIA][31])
* **High Resolution Sparse Voxel DAGs** — the classic compressed sparse-voxel reference. ([research.chalmers.se][32])

## What I would copy first if I were building a Teardown-like 60 FPS engine

I would start from this recipe:

Use **many local dense volumes or dense subchunks inside a sparse wrapper**, not a monolithic field. Store materials as compact indices, not fat per-voxel structs, and bit-pack occupancy whenever a pass only needs visibility/solidity. Keep **authoritative voxel data** separate from the renderable representation, separate again from physics/contact accelerators, and regenerate derivatives locally after edits. For rendering, choose either **Teardown-style object-local raymarching** or **meshed surfaces**, then add a hybrid far-field path later. Add **empty-space skipping** on day one: DDA, bitmasks, mip bricks, or all three. Solve draw calls and geometry pressure with **chunk merging, batching, indirect rendering, and compact formats**. Treat **LOD/streaming** and **collision budgeting** as core architecture, not polish. And if you go down the path-traced route, plan for **temporal reuse and denoising** from the start. ([Game Developer][1])

The next useful deliverable would be a **concrete architecture spec** for a 10 cm voxel engine: world format, chunk size, invalidation model, traversal stack, render path, physics path, and a staged roadmap from “editable world at 60 FPS” to “hybrid ray-traced lighting.”

[1]: https://www.gamedeveloper.com/design/how-beautiful-voxels-laid-the-way-for-i-teardown-s-i-heist-y-framework "https://www.gamedeveloper.com/design/how-beautiful-voxels-laid-the-way-for-i-teardown-s-i-heist-y-framework"
[2]: https://blog.voxagon.se/2024/12/29/year-summary.html "https://blog.voxagon.se/2024/12/29/year-summary.html"
[3]: https://www.patreon.com/posts/optimisations-110203918 "https://www.patreon.com/posts/optimisations-110203918"
[4]: https://github.com/DouglasDwyer/octo-release "https://github.com/DouglasDwyer/octo-release"
[5]: https://www.youtube.com/watch?v=VQv1OEm_www "https://www.youtube.com/watch?v=VQv1OEm_www"
[6]: https://teardowngame.com/modding/ "https://teardowngame.com/modding/"
[7]: https://blog.voxagon.se/2020/11/18/teardown-quicksave.html "https://blog.voxagon.se/2020/11/18/teardown-quicksave.html"
[8]: https://vkguide.dev/docs/ascendant/ascendant_geometry/ "https://vkguide.dev/docs/ascendant/ascendant_geometry/"
[9]: https://blog.voxagon.se/2018/10/17/from-screen-space-to-voxel-space.html "https://blog.voxagon.se/2018/10/17/from-screen-space-to-voxel-space.html"
[10]: https://github.com/cgerikj/binary-greedy-meshing "https://github.com/cgerikj/binary-greedy-meshing"
[11]: https://diglib.eg.org/items/60c72224-00f3-416d-9952-ee41e8c408da/full "https://diglib.eg.org/items/60c72224-00f3-416d-9952-ee41e8c408da/full"
[12]: https://nickmcd.me/2021/04/04/high-performance-voxel-engine/ "https://nickmcd.me/2021/04/04/high-performance-voxel-engine/"
[13]: https://www.gamesinprogress.com/indie-game-developers/tooley1998/adding-seamless-lod-transitions-to-my-voxel-game "https://www.gamesinprogress.com/indie-game-developers/tooley1998/adding-seamless-lod-transitions-to-my-voxel-game"
[14]: https://0fps.net/2012/08/20/simplifying-isosurfaces-part-2/ "https://0fps.net/2012/08/20/simplifying-isosurfaces-part-2/"
[15]: https://docs.voxelplugin.com/1.2/core-systems/voxelworld/collisions-and-navmesh "https://docs.voxelplugin.com/1.2/core-systems/voxelworld/collisions-and-navmesh"
[16]: https://vercidium.com/blog/voxel-world-optimisations/ "https://vercidium.com/blog/voxel-world-optimisations/"
[17]: https://www.patreon.com/posts/voxel-engine-105078253 "https://www.patreon.com/posts/voxel-engine-105078253"
[18]: https://0fps.net/2013/07/03/ambient-occlusion-for-minecraft-like-worlds/ "https://0fps.net/2013/07/03/ambient-occlusion-for-minecraft-like-worlds/"
[19]: https://research.nvidia.com/labs/rtr/publication/schied2017spatiotemporal/ "https://research.nvidia.com/labs/rtr/publication/schied2017spatiotemporal/"
[20]: https://blog.voxagon.se/2020/12/03/spraycan.html "https://blog.voxagon.se/2020/12/03/spraycan.html"
[21]: https://research.nvidia.com/publication/2018-09_ray-box-intersection-algorithm-and-efficient-dynamic-voxel-rendering "https://research.nvidia.com/publication/2018-09_ray-box-intersection-algorithm-and-efficient-dynamic-voxel-rendering"
[22]: https://github.com/dubiousconst282/VoxelRT "https://github.com/dubiousconst282/VoxelRT"
[23]: https://github.com/viktor-ferenczi/godot-voxel "https://github.com/viktor-ferenczi/godot-voxel"
[24]: https://0fps.net/2012/06/30/meshing-in-a-minecraft-game/ "https://0fps.net/2012/06/30/meshing-in-a-minecraft-game/"
[25]: https://transvoxel.org/ "https://transvoxel.org/"
[26]: https://www-sop.inria.fr/reves/Basilic/2009/CNLE09/ "https://www-sop.inria.fr/reves/Basilic/2009/CNLE09/"
[27]: https://voxel-tools.readthedocs.io/en/latest/performance/ "https://voxel-tools.readthedocs.io/en/latest/performance/"
[28]: https://mmacklin.com/smallsteps.pdf "https://mmacklin.com/smallsteps.pdf"
[29]: https://research.nvidia.com/publication/2020-07_spatiotemporal-reservoir-resampling-real-time-ray-tracing-dynamic-direct "https://research.nvidia.com/publication/2020-07_spatiotemporal-reservoir-resampling-real-time-ray-tracing-dynamic-direct"
[30]: https://research.nvidia.com/labs/rtr/publication/lin2022generalized/ "https://research.nvidia.com/labs/rtr/publication/lin2022generalized/"
[31]: https://research.nvidia.com/publication/2010-02_efficient-sparse-voxel-octrees-analysis-extensions-and-implementation "https://research.nvidia.com/publication/2010-02_efficient-sparse-voxel-octrees-analysis-extensions-and-implementation"
[32]: https://research.chalmers.se/publication/182658 "https://research.chalmers.se/publication/182658"
