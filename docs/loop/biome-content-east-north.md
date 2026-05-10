# Biome Content Brief: East / North Regions

Date: 2026-05-08

Context: `src/engine/worldgen-region.ts` is now the macro source of truth for island regions. The next content pass should make regions feel authored at Morrowind scale: large readable provinces, named route corridors, sparse but memorable landmark cadence, and local micro-biomes that support the macro identity instead of looking like random noise.

Coordinates below are source meters first, with current world units in parentheses. `metersToWorldUnits()` is `10x`.

## Macro Anchors

| Region | Center | Radius | Current biome / variant / ambient | Field hook |
| --- | --- | --- | --- | --- |
| `grazelands` | `(3420, -3080)` m, `(34200, -30800)` wu | `(2180 x 1880)` m | `savanna`, `savanna_flowersea`, `dry-haze` | `northeastGrazelands` |
| `glass-shard-coast` | `(4740, 2180)` m, `(47400, 21800)` wu | `(1820 x 2040)` m | `shardlands`, `dunes_glass`, `cold-glass` | `easternShardCoast` |
| `west-gash` | `(-2720, -3260)` m, `(-27200, -32600)` wu | `(1820 x 1760)` m | `highland`, `highland_redleaf`, `green-canopy` | region id only today; can reuse `uplift`, `grove`, `oldGrowth`, `moisture` |

Relevant existing landmark IDs: `acacia`, `thorn_tree`, `flower_patch`, `fruit_tree`, `berry_bush`, `standing_stone`, `ancestor_pillar`, `boulder`, `crystal_cluster`, `glass_cairn`, `salt_spire`, `hoodoo`, `velothi_shrine`, `pilgrim_cairn`, `redleaf_tree`, `birch`, `redwood`, `tall_fir`, `fir`, `stone_tor`, `old_road_causeway`, `paver_debris`, `scree_fan`, `pilgrim_lantern`, `rib_arch`, `shrine_debris`, `bone_chimes`, `ashlander_travel_pack`.

## Grazelands

Terrain shape: broad rolling uplands, flower flats, and a few shallow dry washes. Keep relief lower than West Gash but not flat; the current `savanna_flowersea` delta already follows `hills`, so bias this region toward long low swells and visible horizon silhouettes. Avoid dense forest walls. The authored read should be "open grassland with intentional camps, stones, groves, and route markers."

Forest / vegetation: use stand directors rather than uniform scatter. Keep `flower_patch` dense in flowersea basins, `acacia` / `thorn_tree` as sparse skyline trees, and `fruit_tree` / `berry_bush` in oasis-like groves. Good target clusters: acacia pairs on ridges, flower carpets in depressions, fruit/berry pockets near route shoulders.

Caves: low cave density in open flowersea, but visible entrances along dry wash cuts and rocky standing-stone outcrops. Current cave affinities for `savanna` are low; that is fine. Add entrances only where `channel`, `ridge`, and `scatter` line up, so caves feel like authored gullies rather than random holes.

Routes: add a north/east grazing-road director crossing the region from the inner island toward `(3420, -3080)` and onward to the glass coast. Existing `old_road_causeway`, `paver_debris`, `pilgrim_lantern`, `standing_stone`, and `ancestor_pillar` are enough for the first pass. Cadence target: one small route token every 180-260 m, one strong silhouette every 350-500 m.

Landmark density / directors: do not increase global savanna noise. Add a Grazelands-specific director keyed by `regionId === "grazelands"` or `northeastGrazelands > 0.55`: sparse `standing_stone` rings, acacia groves, flowersea basins, rare `ancestor_pillar` waypoints, and a few `ashlander_travel_pack` camp remnants near route shoulders.

Palette / material targets: warm ochre grasses and dry-gold flowers without turning the entire region beige. Existing `savanna` / `savanna_flowersea` materials are `#BA6/#CB7/#DB8/#C86` and `#CB7/#DA8/#A86/#B97`; add contrast with cooler stone `#887/#998`, muted green tree tops, and occasional pale path wear.

## Glass-Shard Coast

Terrain shape: jagged coastal shelves, glass dunes, broken salt pans, and faceted low ridges. The region is centered at `(4740, 2180)` m and should read as the eastern coastal hazard, not just "white dunes." Existing `shardlands` special height adds ridge-tied lift and `dunes_glass` adds dune relief; use that but direct it into bands: inland shard ridges, middle broken pans, coast-facing shelves.

Forest / vegetation: almost none. The "forest" equivalent is mineral clusters: `crystal_cluster`, `glass_cairn`, `salt_spire`, rare dead `palm` / `dead_tree` silhouettes. Use `crystal_reeds` only where the coast blends into salt/marsh or magic-water pockets, not across the whole region.

Caves: high-value cave region. Current `shardlands` gets `crystalline` or `basaltic` underground plus high deep/upper cave affinity. Surface entrances should be obvious on faceted ridges and shelf edges. Prioritize cave mouths near `glass_cairn`, `velothi_shrine`, or `pilgrim_cairn` clusters so the coast feels hand-placed.

Routes: one hazardous coastal traverse from the inner/south basin into `(4740, 2180)` and one inland rim route skirting shard ridges. Existing `DUNES_GLASS_LANDMARKS` and `SPECIAL_BIOME_LANDMARKS.shardlands` already include `crystal_cluster`, `glass_cairn`, `salt_spire`, `velothi_shrine`, `pilgrim_cairn`, `hoodoo`, and `dead_tree`. Add route material wear sparingly; the coast should feel navigable by cairns more than by roads.

Landmark density / directors: medium-to-high mineral density, low organic density. Target strong vista tokens every 250-350 m along routes and every 500-700 m off-route. Use directors for shard fields, cairn chains, shrine overlooks, and cave-mouth clusters. Avoid sprinkling `glass_cairn` evenly; arrange them as breadcrumb lines and warning piles.

Palette / material targets: cold glass, pale sand, cyan-white highlights, violet accent. Existing `shardlands` uses `#BCA/#CED/#DFF/#A7C/#889/#667/#88A/#6BE/#FFF`; `dunes_glass` overrides to `#EDC/#CDD/#BAA/#CBB`. Keep `#DFF/#6BE/#A7C` as the recognizable region signal, but use darker basalt/crystalline cuts to break the grid and avoid a washed-out snow read.

## West Gash

Terrain shape: highland ravines, red-leaf valleys, tors, and steep wooded shoulders. Center `(-2720, -3260)` m, radius `(1820 x 1760)` m. This is the northwestern vertical contrast to Grazelands: more uplift, more ravine shadow, more enclosed navigation. Current `highland_redleaf` adds a positive height delta; use it for raised shoulders and valley walls, not uniform bumps.

Forest / vegetation: redleaf woods with fir/redwood pockets in basins. Existing `HIGHLAND_REDLEAF_LANDMARKS` is `redleaf_tree`, `birch`, `standing_stone`, `boulder`, `flower_patch`; existing highland logic can also pick `HIGHLAND_REDWOOD_*` when `oldGrowth`, `grove`, and `uplift` align. High ROI: make redleaf groves visually dominant in valley floors and leave ridge crests more stone/tor-heavy.

Caves: strong cave region. `highland` and `granitic` both have high cave affinity, and `stone_tor` / `standing_stone` already mark granitic surface leakage. Entrances should prefer ravine walls, tors, and route-adjacent overlooks. Add a "gash cave chain" director before adding new cave systems: visible entrance mouths plus nearby `stone_tor`, `boulder`, and `pilgrim_cairn`.

Routes: winding gash road along ravine floors with switchback-feeling tokens on shoulders. Reuse `old_road_causeway`, `paver_debris`, `scree_fan`, `pilgrim_lantern`, `standing_stone`, and `pilgrim_cairn`. Keep route density higher than Grazelands because terrain occludes vistas: route token every 140-220 m, strong silhouette every 250-400 m.

Landmark density / directors: dense forest in basins, sparse exposed crests. Add a West Gash director for alternating segments: redleaf grove, stone tor field, cave overlook, old-road ruin. This should be macro-authored by route distance and ravine bands, not chosen only from `oldGrowth` noise.

Palette / material targets: red/orange leaves against cool highland stone and green canopy. Existing `highland_redleaf` materials are `#A86/#C97/#875/#986`; keep those as leaf/soil identity, with `#778/#889` stone and darker ravine subsurface. Avoid turning all highland ground orange; redleaf should be canopy and leaf-litter pockets.

## Shared Implementation Notes

- Treat `worldgen-region.ts` centers/radii as authored provinces. Region identity should override or direct local noise when `regionStrength > 0.34`, and become strongest around `> 0.55`.
- Prefer directors over new global biome IDs: region-specific landmark rosters, route-band overlays, cave-mouth clusters, and material accents are enough for the first pass.
- Existing pilgrim route machinery is a good template, but its current bands are generic. Add named regional route bands so verification can assert authored coverage by region.
- Keep landmarks clustered by intent. Uniformly increasing `chance` will make the world look noisier, not more authored.
- Favor recognizable silhouettes already in the engine before adding new feature kinds: `stone_tor`, `glass_cairn`, `crystal_cluster`, `old_road_causeway`, `pilgrim_lantern`, `rib_arch`, `standing_stone`, `ancestor_pillar`, `redleaf_tree`, `acacia`.

## Programmatic Verification Ideas

- Region probe: sample grids inside each region ellipse and assert primary `regionId` plus expected biome/variant dominance near the center and blended edges near radius `0.85-1.15`.
- Landmark identity probe: count landmark IDs by region. Assert Grazelands has high `flower_patch/acacia/standing_stone`, Glass-Shard has high `crystal_cluster/glass_cairn/salt_spire`, West Gash has high `redleaf_tree/stone_tor/fir/boulder`.
- Route cadence probe: for each named regional route, check max gap between route tokens and strong silhouettes. Targets: Grazelands `<= 500 m`, Glass-Shard `<= 350 m`, West Gash `<= 400 m`.
- Cave entrance probe: assert visible cave entrance candidates cluster in Glass-Shard and West Gash and remain sparse in central Grazelands.
- Palette probe: sample top materials per region and assert target material families appear without monoculture. Also guard Glass-Shard against reading as only `#FFF/#DFF`.
- Browser route screenshots: fixed camera sweeps for one route per region, with grid/luma checks plus manual contact-sheet notes for first implementation.

## Highest ROI Next Slices

1. Add named regional route bands and route-atlas checks for Grazelands, Glass-Shard Coast, and West Gash. This gives immediate authored traversal and measurable cadence.
2. Add region-specific landmark directors using existing IDs only. Start with Glass-Shard cairn/shard/cave clusters and West Gash redleaf/tor/cave clusters.
3. Add cave-mouth visibility directors for Glass-Shard and West Gash. This turns existing cave affinity into player-facing content.
4. Add palette/material overrides only after route and landmark composition exist. Palette alone will not fix the authored-feeling problem.
5. Add one new prop family later only if object-lab proves existing IDs cannot carry the region: likely a Grazelands camp marker or Glass-Shard mine/claim marker.
