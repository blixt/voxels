I found no single canonical “how to build biome-rich voxel worlds” source. The useful material is split across engine docs for Luanti/Minetest, Godot Voxel Tools, Unreal’s Voxel Plugin, Terasology modules, original Voxel Farm engineering notes, and a handful of rendering/storage papers such as Transvoxel, OpenVDB, sparse voxel octrees, and procedural cave papers. Taken together, though, they form a pretty coherent implementation playbook. ([Luanti Documentation][1])

## What the strongest sources converge on

**1. Treat biome as a derived classification over continuous fields, not as your only stored truth.**
The best-supported pattern is to compute stable environmental fields first—temperature, humidity, elevation, roughness, sea influence, hydrology, geology, depth, cave humidity, geothermality—and derive surface biome, underground biome, material stack, flora, structures, and effects from those fields. Luanti already models biomes with heat and humidity points arranged on a Voronoi diagram plus `vertical_blend` and `weight`; Terasology’s biome provider combines sea level, elevation, surface roughness, temperature, and humidity; PolyWorld derives rivers, moisture, and elevation before assigning biomes. That is a much better foundation for 100 wildly different biomes than a flat enum. ([GitHub][2])

**2. Split generation into deterministic passes with different spatial scopes.**
The recurring architecture is: a region-scale pass for climate/hydrology/geology, a voxel-density/material pass for terrain, a cave/underground pass, then later passes for decorations, schematics, foliage, settlements, and gameplay objects. Voxel Tools explicitly says its graph generator is per-voxel and suitable for base terrain and biomes but not for trees or villages; it recommends a second pass for structures. Its multipass column generator lets later passes see previous results and neighboring columns. Terasology similarly separates facet generation from rasterization and warns that chunk-local rasterizers are the wrong place to invent large objects like trees. Luanti’s mapgen API also supports batching ores, decorations, and schematics in the same `VoxelManip` buffer before committing. ([Voxel Tools][3])

**3. Use a multichannel chunk/block payload rather than overloading block IDs.**
Luanti’s `VoxelManip` works on separate bulk arrays for nodes, lighting, and `param2`; Voxel Tools’ block serialization format supports eight channels with independent depths/compression and block/per-voxel metadata; Voxel Plugin adds queryable metadata that can affect shape, materials, and PCG. The practical lesson is that “biome”, wetness, snow load, cave humidity, poison level, magical anomaly, ownership, and effect intensity should not all be encoded as separate block variants. Keep dense voxel data compact, but give yourself metadata channels or query fields. ([GitHub][2])

**4. Save the generated world and the edited world differently.**
The cleanest pattern is: base terrain stays procedural and deterministic; permanent changes are stored as sparse deltas per chunk/block plus object/metadata changes. Voxel Tools streams blocks in and out block-by-block, only loading nearby blocks, and by default saves only modified blocks; it can also persist generated output immediately when the generator is expensive. Luanti stores mapblock data in a map database and keeps static objects with each mapblock. Voxel Plugin exposes explicit save/load for runtime edits. For big worlds, this “procedural base + sparse overlay” model is the most substantiated design. ([Voxel Tools][4])

**5. Make all save/load operations asynchronous, but treat async lifetime bugs as a first-class engine problem.**
Voxel Tools is unusually explicit here: save/load tasks are asynchronous; blocks may be saved when they unload or when you call `save_modified_blocks()`; and reusing the same stream resource/path while previous async tasks are still finishing can corrupt a new save session. That is a real warning sign for any voxel engine with background streaming. Your save path, worldgen version, chunk format version, and async task ownership all need hard lifecycle rules. ([Voxel Tools][4])

**6. Memory wins come mostly from sparsity, uniformity, and scratch-buffer discipline.**
Luanti’s mapgen optimization notes are very concrete: create Perlin objects once, reuse a single noise table, reuse data tables, and avoid expensive repeated indexing. Voxel Tools’ block format has a `COMPRESSION_UNIFORM` mode for channels whose entire block has one value. OpenVDB reduces memory by representing data as background values, tiles, and leaf voxels in a sparse hierarchy. A volumetric terrain paper by Santamaría-Ibirika et al. also reports memory savings from stacked material structures rather than naïve per-voxel material storage. ([Luanti Documentation][5])

**7. Use different placement systems for different classes of “things in a biome.”**
Natural props, structures, and geology should not be spawned by the same mechanism. Luanti’s decoration system already distinguishes simple decorations, schematics, and L-systems, with placement filters such as `place_on`, biome restrictions, density noise, `spawn_by`, and neighborhood checks. Luanti ores similarly distinguish scatter, sheet, puff, blob, vein, and stratum. Voxel Tools recommends instancing for grass/rocks/trees but not for complex man-made structures. Voxel Plugin likewise separates stamps, PCG sampling, and spawners. The practical takeaway is to build at least three layers: geological fillers, natural decorations, and authored/grammatical structures. ([GitHub][2])

**8. For object placement, prefer blue-noise or cell-based candidates plus surface queries.**
The most portable approach I found is: generate candidate points by Poisson-disc or cell sampling, then filter them with terrain queries such as height, normal, slope, aspect, material, depth-to-water, metadata, and biome weights. Bridson’s Poisson-disc method is still the standard O(N) blue-noise reference. Terasology’s `SurfaceNormalFacet` and `SurfaceSteepnessFacet` are explicit examples of derived query fields for placement. Voxel Plugin exposes query nodes for height or distance, normal, surface type, and metadata; its PCG docs note that the voxel sampler’s cell size controls cost, and its spawner docs explain when height spawners are much faster than ray/volume methods. ([Computer Science at UBC][6])

**9. Height-based placement is a huge optimization, but only in truly 2.5D biomes.**
Voxel Plugin’s docs are blunt: height stamps are cheaper and cannot represent caves or overhangs; volume stamps can. Its spawner docs also note that height spawners are dramatically faster when terrain has no tunnels or overhangs. For your biome catalogue, that suggests a hybrid engine: plains, dunes, salt pans, some forests, and many surface wetlands can stay height-query-driven; floating islands, cave gardens, arches, lava tubes, undercities, and fractured gravity biomes need full 3D/volume queries. ([Voxel Plugin Documentation][7])

**10. Structure generation must see beyond the current chunk.**
A lot of implementation pain is really boundary pain. Voxel Tools shows two practical solutions for trees that extend across block borders: regenerate them deterministically from coordinate-based rules, or use a multipass generator with neighboring-column access; it even demonstrates computing a structure’s bounds and pasting only the intersecting part into the current block. Terasology’s worldgen issue makes the same point from the opposite angle: generating trees in chunk-local rasterizers leads to cut-off trees, so object creation belongs in facets over a larger region. This “chunk + halo” design is not optional for large roots, villages, cave mouths, cliffs, riverbanks, or border ecotones. ([Voxel Tools][8])

**11. Surface biomes and underground biomes should be separate but coupled systems.**
The sources do not all say this explicitly, but the strongest inference is that caves, caverns, lava tubes, flooded chambers, fungal gardens, ore belts, and dungeons need their own classifier driven by depth, rock type, water table, void size, cave humidity, temperature, and geothermal activity. Luanti’s biome registration already includes cave-liquid and dungeon-related fields, and its ore system can be biome-restricted with multiple morphology types. Voxel Plugin’s volume stamps exist precisely because caves/overhangs need 3D shaping. Santamaría-Ibirika’s terrain paper includes material mixtures, mineral veins, caves, and underground material flow; a separate cave paper generates playable cave systems using Voronoi/Delaunay structure rather than just raw noise. ([GitHub][2])

**12. Good cave generation is not just “subtract 3D noise.”**
Voxel Tools’ cave tutorial is already more sophisticated than the usual blog-post approach: it combines low-frequency noise “worms,” Y-modulated thresholds, a parabola to confine caves vertically, extra perturbation noise, and smooth subtraction from the base SDF terrain. Voxel Farm’s cave notes push the same idea further: good caves behave more like graph-connected spaces with larger rooms and meaningful tunnel connections than like homogeneous Swiss cheese. For developer use, that means at least two cave classes: ambient erosion/noise caves and authored graph-like cave systems. ([Voxel Tools][8])

**13. Biome borders should be modelled as transforms and overlays, not just nearest-neighbor switches.**
Luanti exposes `vertical_blend`, uses heat/humidity Voronoi points, and even documents smooth biome blending in older mapgen. Bedrock’s older biome-generation schema is still a useful design reference because it explicitly models `river_transformation`, `shore_transformation`, `mutate_transformation`, and `hills_transformation`. Voxel Plugin’s stamp system lets shape, material, and metadata blend independently, and its bounds extension helps with smoothing and foliage placement. The best interpretation is that subbiomes should usually be overlay masks or transformation passes: river modifies floodplain, coast modifies shore, height modifies alpine, cave humidity modifies underground garden, volcanic heat modifies ash fringe. ([GitHub][2])

**14. Cross-biome dependencies should be expressed through shared fields such as hydrology, slope, and road/settlement graphs.**
PolyWorld derives rivers from downhill flow and moisture partly from river distance before assigning biomes. Terasology’s Cities module partitions the world into sectors, chooses suitable settlement sites per sector, connects them by roads, then fills in lots and buildings. Voxel Farm’s water writeup uses seeded sources and A* routing to ocean level. All three point toward the same architecture: rivers, roads, settlements, coastlines, lava channels, and fungal spreads are not decorations inside a finished biome; they are world-scale dependency systems that in turn reshape the biome. ([GitHub][9])

**15. Slope/aspect-aware surface resolution matters more once overhangs and cliffs enter the picture.**
Terasology’s improved surface-facet work is specifically about reducing dependence on the assumption of a single height surface, and it cites better border behavior when using slope thresholds and aspect to place sand versus snow/grass. Voxel Plugin’s surface types and smart material graphs similarly support rules such as cliffs on steep slopes or snow above certain heights. That is exactly the kind of machinery you need for the border cases in your 100-biome set: basalt cliff versus ash apron, dune crest versus salt pan, alpine snowline versus lichen rock, swamp hummock versus blackwater channel. ([GitHub][10])

**16. Biome-specific effects should be driven by metadata and queryable fields, not hardwired per-biome code paths.**
Voxel Plugin’s metadata can be written by stamps, queried later, passed into materials, and is only computed when some output or query requires it. Voxel Tools’ serialization format supports custom metadata types and per-voxel metadata in addition to voxel channels. That makes a strong case for representing things like fog density, acid intensity, spore luminosity, ashfall rate, snow accumulation, ambient sound zone, hazard strength, or local magic distortion as fields/metadata that rendering, VFX, AI, and gameplay systems can query. ([Voxel Plugin Documentation][11])

**17. Separate static persistence from active simulation.**
Luanti explicitly distinguishes static objects saved in mapblocks from active objects that are loaded and updating. Voxel Tools similarly distinguishes cheap multimesh-style instancing from heavier scene instances. For a voxel biome engine, that translates to a very useful rule: grass, pebbles, passive coral, bones, dead trees, and decorative ruins should usually be static/instanced data; only nearby, interactive, or stateful things become active entities. That is one of the cleanest ways to keep memory and CPU under control in dense biomes. ([Luanti Documentation][12])

**18. Use chunk/block meshing and LOD systems that are explicitly designed for 3D terrain boundaries.**
Voxel Tools’ smooth terrain mode is SDF-based and uses `VoxelMesherTransvoxel`. Eric Lengyel’s Transvoxel algorithm exists specifically to stitch neighboring meshes of different resolutions without cracks using transition cells and only local voxel data. That is still one of the most practical answers for large editable smooth voxel terrain with caves and overhangs. ([Voxel Tools][13])

**19. For far-field rendering, authoring caches, or ultra-sparse data, study VDB/SVO families even if your gameplay core stays chunk-grid based.**
OpenVDB uses a hierarchical sparse tree with background values, tiles, and leaf voxels to minimize memory while keeping fast access in effectively unbounded index space. Efficient Sparse Voxel Octrees adds contour information, normal compression, filtering, and discusses memory/disk management. NanoVDB is the GPU-friendly VDB variant, and GigaVoxels is a classic example of ray-guided streaming of several billion voxels. My read is that editable gameplay near the player is still easiest with mutable chunk/block structures, but VDB/SVO techniques are excellent references for far-field caches, offline preprocessing, or specialized sparse subsystems. ([OpenVDB][14])

**20. Pay real attention to storage keys, chunk format, and versioning.**
Voxel Tools’ SQLite stream offers multiple coordinate-key encodings and notes that key caching helps primarily when saved blocks are sparse. Its block format supports channel compression, standalone chunk serialization, and metadata sections. Luanti warns that changing biome registrations changes biome-to-ID correspondences and that dependent decorations/ores must be re-registered. This is a strong argument for versioned worldgen manifests, stable registration order, and a migration story before you ever ship user worlds. ([Voxel Tools][15])

**21. Small edits and bulk edits should use different APIs.**
Luanti’s API explicitly says direct node edits are faster than `VoxelManip` for very small regions such as 3×3×3 or smaller, while `VoxelManip` exists for bulk region operations and works on a snapshot buffer. That’s a good general engine rule: direct edits for tiny gameplay interactions, batched chunk buffers for tools, explosions, erosion, worldgen, and large player modifications. ([GitHub][2])

**22. Multiplayer edit replication deserves its own subsystem.**
Voxel Plugin notes that runtime edits are not replicated by default and warns that pushing a whole edited world state to players joining in progress can overflow RPC limits. That is exactly the kind of issue a biome-heavy voxel game will hit early if it has caves, digging, building, destruction, and dense procedural props. Save format, network delta format, and streaming format should be related but not identical. ([Voxel Plugin Documentation][16])

## A practical engine shape for your 100-biome catalogue

The most defensible architecture I can synthesize from these sources is this: keep a **region-scale cache** of continuous macrofields; synthesize **density/SDF and material strata** in chunk/block space; run a **3D cave/underground pass** with its own fields; do **structure/layout** generation with chunk halos or sector-scale generators; do **decoration/instancing** from queryable surface/volume data; drive **materials and biome FX** from metadata; then persist only **modified chunks, metadata, and stateful-object deltas**. That lines up with how Luanti batches worldgen inside `VoxelManip`, how Voxel Tools separates generators, streams, multipass columns, and instancing, how Voxel Plugin separates height/volume stamps, metadata, materials, and saves, and how Terasology separates world facets from rasterization and sector-scale settlement logic. ([GitHub][2])

## The best deep dives and references I found

### Engine and framework docs

* **Luanti / Minetest engine structure + database docs**: useful for the canonical chunked mutable-world pattern—`MapBlock` as 16×16×16 nodes plus metadata/static objects, with map data persisted via a world database backend. ([Luanti Documentation][17])
* **Luanti Lua API (`VoxelManip`, biomes, decorations, ores)**: one of the most concrete public references for batched voxel editing, biome fields, vertical blending, decoration placement filters, and multiple ore morphology types. ([GitHub][2])
* **Luanti mapgen memory optimization notes**: very practical advice on noise-object reuse, table reuse, and avoiding avoidable indexing overhead in scripted generation. ([Luanti Documentation][5])
* **Godot Voxel Tools docs**: especially the pages on generators, multipass generation, procedural generation, streams, SQLite storage, serialization format, instancing, smooth terrain, and performance. This is one of the densest public engine-documentation sets for a modern voxel terrain stack. ([Voxel Tools][3])
* **Unreal Voxel Plugin docs**: strong on procedural graphs, stamps, metadata, material/surface typing, queries, PCG sampling, spawners, and runtime save/load. Particularly valuable for “biome as layered overlays and metadata” thinking. ([Voxel Plugin Documentation][18])
* **Terasology CoreWorlds / PolyWorld / SurfaceFacets / Cities**: excellent examples of region/facet-based generation, climate+moisture biome resolution, slope-aware surface classification, and sector-scale structure networks. ([GitHub][19])

### Placement, structures, and boundary handling

* **Voxel Tools tree/block-boundary guidance**: one of the clearest writeups on handling structures that cross chunk borders without cut-offs. ([Voxel Tools][8])
* **Terasology 3D worldgen discussion**: useful because it states the anti-pattern clearly—chunk-local rasterizers create cut-off trees, so object creation must happen in larger world facets. ([GitHub][20])
* **Luanti decoration docs**: a compact reference for density noise, biome restrictions, adjacency checks, schematic placement, and L-system style generation. ([GitHub][2])
* **Voxel Plugin spawner/PCG docs**: practical tuning details for chunk size, height-vs-ray methods, cell size, and performance/quality tradeoffs. ([Voxel Plugin Documentation][21])
* **Bridson Poisson-disc sampling**: still the canonical reference for blue-noise candidate generation in arbitrary dimensions. ([Computer Science at UBC][6])

### Underground, caves, geology, and materials

* **Voxel Tools cave tutorial**: the best public “actual node graph” explanation I found for cave carving beyond trivial noise subtraction. ([Voxel Tools][8])
* **Santamaría-Ibirika et al., procedural volumetric terrain**: valuable for layered materials, mineral veins, material mixtures, caves, and underground material flow, plus memory-minded stacked material structures. ([Springer][22])
* **Playable cave systems paper**: useful when you want caves to feel navigable and intentional rather than random voids. ([santosgrueiro.com][23])
* **Voxel Farm / ProcWorld cave and terrain-layer notes**: especially useful for layered geology and the “caves as connected space graph” mindset. ([Procedural World][24])
* **Luanti ore docs**: good public examples of different underground morphology primitives—veins, strata, puffs, blobs, sheets, scatter. ([GitHub][2])

### Persistence, storage, and compression

* **Voxel Tools streams + SQLite + block format docs**: block-by-block streaming, async save/load, sparse modified-block storage, coordinate keys, LZ4 compression, and uniform-channel compression are all directly relevant to shipping an editable voxel game. ([Voxel Tools][4])
* **Luanti database backend docs**: a straightforward example of world state partitioned across separate databases, with map data isolated from player/auth/mod data. ([Luanti Documentation][1])
* **OpenVDB overview/tree docs**: excellent for sparse hierarchical storage concepts—background values, tiles, leaf nodes, and fast access in huge index spaces. ([OpenVDB][25])
* **NanoVDB**: worth reading if you want GPU-friendly sparse volume structures for rendering/simulation subsystems. ([NVIDIA][26])
* **Out-of-core SVO construction**: useful for large offline builds and Morton-order IO thinking. ([graphics.cs.kuleuven.be][27])

### Meshing, LOD, and rendering

* **Transvoxel official site**: still the clearest high-level explanation of crack-free LOD stitching for volumetric terrain. ([Transvoxel][28])
* **Voxel Tools smooth terrain docs**: practical implementation context for Transvoxel in a shipping-style engine module. ([Voxel Tools][13])
* **Efficient Sparse Voxel Octrees technical report**: excellent on compact voxel storage, contour information, normal compression, post-filtering, and memory/disk management. ([NVIDIA][29])
* **GigaVoxels**: classic reading for ray-guided streaming and rendering very large voxel datasets. ([Delft Publications][30])

### Noise, masks, and spatial primitives

* **Worley cellular noise**: still a strong primitive for cell-like biome masks, cracked pans, blob regions, and stone-cell patterns. ([muugumuugu.github.io][31])
* **Simplex noise notes**: useful for lower directional artifacts and better scaling to higher-dimensional field generation than classic Perlin-style approaches. ([itn.liu.se][32])
* **PolyWorld’s Voronoi/Lloyd approach**: excellent reference for polygonal macroregions, moisture, rivers, and biome assignment. ([GitHub][9])

### Cross-biome transformations and data-driven rules

* **Minecraft Bedrock biome-generation rules**: old and explicitly pre-Caves-and-Cliffs, but still valuable as a schema example because it names river, shore, mutate, and hills transformations. ([Microsoft Learn][33])
* **Minecraft Bedrock feature rules**: useful as an official example of data-driven feature placement conditioned by biome. ([Microsoft Learn][34])
* **Voxel Plugin stamps/materials/metadata**: arguably the modern engine-doc equivalent of data-driven subbiomes and biome overlays. ([Voxel Plugin Documentation][7])

## The clearest “do this, not that” rules I’d carry into an implementation

Do not make a single biome ID the master truth; make it a derived label over stable fields. Do not place trees, cities, or cave biomes inside a purely per-voxel or chunk-local pass; use larger-region generators, multipass generation, or deterministic neighbor regeneration. Do not save the whole generated world eagerly unless generation is genuinely expensive; save sparse edits by default. Do not pack every environmental state into block IDs; use metadata/channels/fields. Do not use height-only placement in volumes that contain overhangs, tunnels, floating islands, or multilayer geometry. Do not leave worldgen registration order and async streaming lifetime implicit; version them explicitly. ([Voxel Tools][3])

The next useful step is turning this into a concrete engine spec: chunk schema, generator pass graph, save format, metadata fields, and a biome-rule DSL for the 100-biome catalogue.

[1]: https://docs.luanti.org/for-server-hosts/database-backends/ "https://docs.luanti.org/for-server-hosts/database-backends/"
[2]: https://raw.githubusercontent.com/luanti-org/luanti/master/doc/lua_api.md "https://raw.githubusercontent.com/luanti-org/luanti/master/doc/lua_api.md"
[3]: https://voxel-tools.readthedocs.io/en/latest/generators/ "https://voxel-tools.readthedocs.io/en/latest/generators/"
[4]: https://voxel-tools.readthedocs.io/en/latest/streams/ "https://voxel-tools.readthedocs.io/en/latest/streams/"
[5]: https://docs.luanti.org/for-creators/mapgen/memory-optimizations/ "https://docs.luanti.org/for-creators/mapgen/memory-optimizations/"
[6]: https://www.cs.ubc.ca/~rbridson/docs/bridson-siggraph07-poissondisk.pdf "https://www.cs.ubc.ca/~rbridson/docs/bridson-siggraph07-poissondisk.pdf"
[7]: https://docs.voxelplugin.com/2.0p7/knowledgebase/working-with-stamps/ "https://docs.voxelplugin.com/2.0p7/knowledgebase/working-with-stamps/"
[8]: https://voxel-tools.readthedocs.io/en/latest/procedural_generation/ "https://voxel-tools.readthedocs.io/en/latest/procedural_generation/"
[9]: https://github.com/Terasology/PolyWorld "https://github.com/Terasology/PolyWorld"
[10]: https://github.com/Terasology/SurfaceFacets/issues/5 "https://github.com/Terasology/SurfaceFacets/issues/5"
[11]: https://docs.voxelplugin.com/knowledgebase/materials/working-with-metadata "https://docs.voxelplugin.com/knowledgebase/materials/working-with-metadata"
[12]: https://docs.luanti.org/for-engine-devs/objects/ "https://docs.luanti.org/for-engine-devs/objects/"
[13]: https://voxel-tools.readthedocs.io/en/latest/smooth_terrain/ "https://voxel-tools.readthedocs.io/en/latest/smooth_terrain/"
[14]: https://www.openvdb.org/documentation/doxygen/overview.html?utm_source=chatgpt.com "OpenVDB: OpenVDB Overview"
[15]: https://voxel-tools.readthedocs.io/en/latest/api/VoxelStreamSQLite/ "https://voxel-tools.readthedocs.io/en/latest/api/VoxelStreamSQLite/"
[16]: https://docs.voxelplugin.com/knowledgebase/blueprints/runtime-edits-and-sculpting "https://docs.voxelplugin.com/knowledgebase/blueprints/runtime-edits-and-sculpting"
[17]: https://docs.luanti.org/for-engine-devs/structure/ "https://docs.luanti.org/for-engine-devs/structure/"
[18]: https://docs.voxelplugin.com/1.2/core-systems/voxel-graphs/ "https://docs.voxelplugin.com/1.2/core-systems/voxel-graphs/"
[19]: https://github.com/Terasology/CoreWorlds/blob/develop/src/main/java/org/terasology/core/world/generator/facetProviders/BiomeProvider.java "https://github.com/Terasology/CoreWorlds/blob/develop/src/main/java/org/terasology/core/world/generator/facetProviders/BiomeProvider.java"
[20]: https://github.com/MovingBlocks/Terasology/issues/2458 "https://github.com/MovingBlocks/Terasology/issues/2458"
[21]: https://docs.voxelplugin.com/2.0p8%20-%20v2/usage-docs/foliage/using-pcg-on-voxel-terrains "https://docs.voxelplugin.com/2.0p8%20-%20v2/usage-docs/foliage/using-pcg-on-voxel-terrains"
[22]: https://link.springer.com/article/10.1007/s00371-013-0909-y "https://link.springer.com/article/10.1007/s00371-013-0909-y"
[23]: https://santosgrueiro.com/papers/2014/2014-Santam-Cave.pdf "https://santosgrueiro.com/papers/2014/2014-Santam-Cave.pdf"
[24]: https://procworld.blogspot.com/2012/01/introducing-voxel-studio.html "https://procworld.blogspot.com/2012/01/introducing-voxel-studio.html"
[25]: https://www.openvdb.org/documentation/doxygen/?utm_source=chatgpt.com "OpenVDB: OpenVDB"
[26]: https://research.nvidia.com/labs/prl/publication/nanovdb/?utm_source=chatgpt.com "NanoVDB: A GPU-Friendly and Portable VDB Data Structure For Real-Time Rendering And Simulation | NVIDIA High Fidelity Simulation Research"
[27]: https://graphics.cs.kuleuven.be/publications/BLD13OCCSVO/BLD13OCCSVO_paper.pdf "https://graphics.cs.kuleuven.be/publications/BLD13OCCSVO/BLD13OCCSVO_paper.pdf"
[28]: https://transvoxel.org/?utm_source=chatgpt.com "The Transvoxel Algorithm for Voxel Terrain"
[29]: https://research.nvidia.com/publication/2010-02_efficient-sparse-voxel-octrees-analysis-extensions-and-implementation?utm_source=chatgpt.com "Efficient Sparse Voxel Octrees - Analysis, Extensions, and Implementation | Research"
[30]: https://publications.graphics.tudelft.nl/papers/423 "https://publications.graphics.tudelft.nl/papers/423"
[31]: https://muugumuugu.github.io/bOOkshelF/standalone/Worley%20Noise.pdf "https://muugumuugu.github.io/bOOkshelF/standalone/Worley%20Noise.pdf"
[32]: https://www.itn.liu.se/~stegu76/simplexnoise/simplexnoise.pdf "https://www.itn.liu.se/~stegu76/simplexnoise/simplexnoise.pdf"
[33]: https://learn.microsoft.com/en-us/minecraft/creator/reference/content/biomesreference/examples/components/minecraftbiomes_overworld_generation_rules?view=minecraft-bedrock-stable "https://learn.microsoft.com/en-us/minecraft/creator/reference/content/biomesreference/examples/components/minecraftbiomes_overworld_generation_rules?view=minecraft-bedrock-stable"
[34]: https://learn.microsoft.com/en-us/minecraft/creator/reference/content/featurerulesreference/examples/featurerulescomponents/feature_rules_document?view=minecraft-bedrock-stable "https://learn.microsoft.com/en-us/minecraft/creator/reference/content/featurerulesreference/examples/featurerulescomponents/feature_rules_document?view=minecraft-bedrock-stable"
