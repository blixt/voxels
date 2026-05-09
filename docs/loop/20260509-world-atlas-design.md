# 2026-05-09 WorldAtlas Design

## Purpose

Track C needs a finite, authored-procedural island world that stops reading as infinite Minecraft-like noise. This document defines the `WorldAtlas` design before generator code changes begin.

The target is a huge Morrowind-like island with coherent regions, macro terrain, roads, travel routes, cave systems, strong biome identity, and persisted editable canonical chunks. The generator should become a pristine chunk producer from the atlas, while persisted chunks and edits become the durable source of truth.

## Non-Goals For Wave 1

- Do not edit `src/engine/procedural-generator.ts` yet.
- Do not change runtime streaming, LOD, renderer, or persistence behavior yet.
- Do not add new biome IDs just to create variety. Prefer region directors, route/cave graphs, and landmark rosters first.
- Do not make derived LOD or summaries authoritative world data.

## Wave 1 Pure Checkpoint

Implemented as a narrow, pure atlas layer in `src/engine/world-atlas.ts` with focused tests in `tests/world-atlas.test.ts`.

Scope included:

- finite island mask sampling with deterministic land, shoreline, coastal shelf, and deep ocean classification;
- eight macro region definitions with stable centers, ellipse area estimates, explicit edge graph membership, route membership, and cave membership helpers;
- route anchor extraction for authored route nodes and validation anchors;
- six initial cave systems with deterministic entrance/chamber/overlook anchors, tunnel links, material profiles, expected nearby routes, and cave-anchor sampling;
- tests for region centers, meaningful region area, island grid finiteness, outside-island ocean identity, region-edge anchors, route anchors, and cave anchors.

Scope deliberately excluded:

- procedural generator sampling or chunk generation changes;
- route terrain conforming, cave carving, or landmark placement;
- renderer, controller, browser scripts, persistence, and package metadata changes.

## Coordinate System

### Units

- Authoring coordinates are meters.
- Engine world units remain `10` world units per meter.
- Atlas APIs should accept authoring meters internally and expose conversion helpers where needed.
- Existing code can keep world-unit sampling during migration, but atlas definitions should be written in meters for readability.

### Island Frame

Use a named island-local frame:

- `origin_m`: island center in authoring meters.
- `x_m`: east-positive.
- `z_m`: south-positive, matching current world-space use.
- `y_m`: vertical meters, derived from existing world unit heights.

Initial envelope:

| Field | Value |
| --- | ---: |
| Island center | `(-180, -520)` m |
| Radius X | `6400` m |
| Radius Z | `5850` m |
| Playable width | about `12.8 km` |
| Playable depth | about `11.7 km` |

The island is finite by design. Outside the island envelope, chunks should deterministically become ocean, shelf, or deep water. They are still valid generated chunks, but not meaningful biome land.

### Island Mask Outputs

The atlas should expose these continuous fields:

- `islandInterior`: `1` in interior land, `0` outside meaningful island.
- `shorelineBand`: active around beaches, cliffs, tidal flats, marsh edges.
- `coastalShelf`: shallow water and exposed shelf identity.
- `deepOcean`: outside shelf.
- `interiorDistance`: useful for difficulty, route placement, and fog/landmark density.
- `edgeNormal`: optional later, useful for cliff/coast orientation.

Acceptance for the first island mask:

- Sampling a `256 x 256` grid over the bounding rectangle reports a finite land area.
- No non-ocean biome dominates outside `islandInterior < 0.08`.
- Coastline remains irregular but connected; no large accidental detached continents.

## WorldAtlas Data Model

`WorldAtlas` should be a deterministic, mostly data-driven layer with explicit graph/field objects:

```ts
interface WorldAtlas {
  version: string;
  island: IslandEnvelope;
  regions: RegionDefinition[];
  regionEdges: RegionEdgeDefinition[];
  macroTerrain: MacroTerrainDefinition;
  routes: RouteDefinition[];
  caveSystems: CaveSystemDefinition[];
  landmarkZones: LandmarkZoneDefinition[];
}
```

The intended boundary:

- `WorldAtlas` owns geography and authored intent.
- `ProceduralWorldGenerator` samples `WorldAtlas` to produce pristine chunks.
- `ChunkStore` owns persisted generated chunks and edited chunks.
- Render summaries, region summaries, and derived LOD are chunk-derived caches.

## Region Graph

Regions are large authored provinces. Noise can vary a region, but it should not decide the region's identity.

### Region Nodes

Initial region graph:

| ID | Center m | Radius m | Core Identity | Biome / Variant | Ambient |
| --- | ---: | ---: | --- | --- | --- |
| `red-mountain` | `(-520, -1080)` | `(1080, 1020)` | volcanic massif, caldera, sacred ash | `ember` / `ember_caldera` | `ashfall` |
| `ashen-badlands` | `(-840, -2360)` | `(2200, 1760)` | ash apron, ravines, pilgrimage roads | `badlands` / `ash_wastes` | `ashfall` |
| `bitter-coast` | `(-4360, 920)` | `(2320, 2040)` | drowned blackwater coast, roots, fog | `marsh` / `marsh_blackwater` | `silt-mist` |
| `grazelands` | `(3420, -3080)` | `(2180, 1880)` | open rolling plains, stones, camps | `savanna` / `savanna_flowersea` | `dry-haze` |
| `salt-marsh-basin` | `(-180, 4040)` | `(2640, 1820)` | mirror salt flats and dead causeways | `saltflat` / `saltflat_mirror` | `silt-mist` |
| `glass-shard-coast` | `(4740, 2180)` | `(1820, 2040)` | cold glass coast, shard ridges, cairn chains | `shardlands` / `dunes_glass` | `cold-glass` |
| `west-gash` | `(-2720, -3260)` | `(1820, 1760)` | ravines, high passes, redleaf valleys | `highland` / `highland_redleaf` | `green-canopy` |
| `inner-sea` | `(1320, 120)` | `(2260, 2180)` | central moor/sea shelf, roads, ruins | `moor` / `moor_shadowglass` | `silt-mist` |

### Region Edges

The graph should explicitly describe important transition seams:

| Edge | Purpose |
| --- | --- |
| `inner-sea -> red-mountain` | pilgrimage approach, ashfall visible from wet lowlands |
| `red-mountain -> ashen-badlands` | caldera apron, ash roads, cave mouths |
| `ashen-badlands -> west-gash` | highland pass and ravine route |
| `ashen-badlands -> grazelands` | dry eastward trail, camp cadence |
| `bitter-coast -> inner-sea` | wetland to moor crossing, bridges and low islands |
| `inner-sea -> salt-marsh-basin` | shelf road to mirror flats |
| `salt-marsh-basin -> glass-shard-coast` | salt/glass mineral transition |
| `grazelands -> glass-shard-coast` | open route into hazardous shard coast |

Each edge should provide:

- transition width in meters;
- expected route/cave/landmark behavior;
- material bridge palette;
- terrain blend rule;
- screenshot or route validation anchor.

### Region Sampling

Atlas sampling should return:

- `primaryRegionId`
- `secondaryRegionId`
- `regionStrength`
- `regionBlend`
- `regionEdgeId`
- `regionLocalX`
- `regionLocalZ`
- `regionDistance`
- region-owned material, biome, ambient, landmark, route, and cave weights.

Acceptance:

- Each region center samples the expected primary region, biome, variant, and ambient.
- At least `70%` of samples inside each region's `0.65` ellipse radius report that region as primary.
- Edge samples near region boundaries report plausible secondary regions instead of random unrelated regions.

## Macro Terrain Layers

Macro terrain must be authored first and noisy second. The core height pipeline:

```text
height =
  islandBase
  + regionBaseElevation
  + volcanoConeAndCaldera
  + mountainRidgeField
  + basinAndValleyCarve
  + coastAndShelfShape
  + roadAndSettlementConforming
  + localBiomeRelief
  + material-scale micro detail
```

### Layer 1: Island Base

Purpose:

- establish finite island mass;
- lower shoreline and shelf;
- suppress non-ocean terrain outside the island.

Outputs:

- `baseElevation`
- `shoreSlope`
- `coastCliffPotential`
- `shelfDepth`

### Layer 2: Red Mountain Volcanic Massif

Purpose:

- make the island recognizable from a distance;
- give the center a dominant skyline.

Controls:

- cone lift around `red-mountain`;
- broken caldera rim;
- crater depression;
- saddle cuts for routes and cave entrances;
- ash apron blending into `ashen-badlands`.

Acceptance:

- Fixed cameras from Inner Sea, Ashen Badlands, and Grazelands see a dominant Red Mountain silhouette.
- Caldera rim is not a smooth mound: slope/ridge metrics detect broken rim variance.

### Layer 3: Mountain Ridge And West Gash

Purpose:

- create long navigational ridges and passes;
- give `west-gash` vertical identity.

Use authored ridge splines:

- northwest highland spine;
- west-gash ravine walls;
- Red Mountain rim spurs;
- eastern shard coastal ridge.

Acceptance:

- Route continuity validator finds at least one navigable pass through West Gash.
- View atlas detects stronger horizon shape than baseline in West Gash cameras.

### Layer 4: Basins, Valleys, And Water

Purpose:

- make wetland, salt basin, and inner sea feel like large coherent lowlands.

Region-specific rules:

- Bitter Coast: depressed blackwater archipelago with dry hummocks.
- Salt Basin: huge shallow flat with salt ribs and isolated raised islands.
- Inner Sea: broad water shelves, moor islands, old road slabs.

Acceptance:

- Wetland validation reports frequent water plus dry route islands.
- Salt Basin flatness is high, but material entropy is not zero.
- Inner Sea has broad water views and traversable shelves.

### Layer 5: Roads And Settlements

Roads should conform terrain instead of bulldozing it:

- core path: subtle material and elevation smoothing;
- shoulder: debris, dry shelves, small cut/fill;
- edge: route set pieces and landmarks;
- bridge/causeway: explicit spans across water/salt/marsh.

Acceptance:

- Routes have bounded slope and no long gaps in readable route material or set pieces.
- Road flattening does not erase caldera, ravines, or salt basin identity.

### Layer 6: Local Biome Relief

Local noise is still useful for:

- ash crust cracks;
- salt ribs;
- marsh hummocks;
- grass swells;
- glass shard facets;
- redleaf grove floors.

It must be gated by region identity and should not create patchwork biomes.

## Route Graph

Routes are first-class atlas data. They are not landmarks sprinkled by chance.

```ts
interface RouteDefinition {
  id: string;
  kind: "pilgrim-road" | "coastal-walk" | "causeway" | "mountain-pass" | "hazard-route";
  nodes: RouteNode[];
  widthM: number;
  shoulderM: number;
  materialProfileId: string;
  expectedRegions: string[];
  landmarkCadenceM: number;
  strongVistaCadenceM: number;
}
```

### Initial Routes

| ID | Start -> End | Role |
| --- | --- | --- |
| `pilgrim-spine-red` | Inner Sea -> Red Mountain -> Ashen Badlands | main sacred road |
| `ash-gash-pass` | Ashen Badlands -> West Gash | mountain pass and cave route |
| `badlands-east-trail` | Ashen Badlands -> Grazelands | dry caravan trail |
| `bitter-inner-crossing` | Bitter Coast -> Inner Sea | wetland bridge/island path |
| `salt-causeway` | western salt basin -> eastern salt basin | long dead causeway across mirror flats |
| `inner-sea-shelf-road` | Inner Sea shelves and low islands | early navigation route |
| `grazelands-glass-road` | Grazelands -> Glass-Shard Coast | open plains into hazard coast |
| `glass-coastal-cairns` | Glass-Shard Coast north/south | cairn-led coastal traverse |

### Route Outputs

Route sampling should expose:

- `routeId`
- `routeInfluence`
- `routeCore`
- `routeShoulder`
- `distanceAlongM`
- `distanceToRouteM`
- `routeSegmentKind`
- `recommendedSetPieceIds`

Acceptance:

- Every named route has a route-atlas report.
- Max route-token gap:
  - pilgrimage/West Gash: `<= 300 m`
  - wetland/salt causeways: `<= 360 m`
  - glass coast: `<= 350 m`
  - open Grazelands: `<= 500 m`
- No named route has an unplanned impassable slope segment.

## Cave Graph

Caves should become authored regional networks with deterministic local variation.

```ts
interface CaveSystemDefinition {
  id: string;
  regionId: string;
  kind: "lava-tube" | "kwama-mine" | "sea-cave" | "root-cave" | "crystal-cavern" | "barrow";
  nodes: CaveNode[];
  edges: CaveTunnel[];
  entranceMarkers: LandmarkId[];
}
```

### Initial Cave Systems

| ID | Region | Kind | Purpose |
| --- | --- | --- | --- |
| `red-caldera-tubes` | `red-mountain` | lava tube | visible rim/saddle entrances |
| `ash-kwama-ravines` | `ashen-badlands` | kwama mine | route-adjacent mine/cave mouths |
| `bitter-root-grottos` | `bitter-coast` | root cave | rare dry hummock entrances |
| `west-gash-ravine-caves` | `west-gash` | granitic cave | pass-adjacent cave chain |
| `glass-crystal-caverns` | `glass-shard-coast` | crystal cavern | shard ridge entrances |
| `salt-crust-sinkholes` | `salt-marsh-basin` | saline cave | shallow sinkholes on raised ribs |

### Cave Carving

Use graph-first carving:

- tunnel spline envelopes;
- chamber spheres/ellipsoids;
- local cave noise for edge breakup;
- entrance orientation and dry-surface validation;
- region-specific material lining.

Wave 1 only defines cave-system anchors and tunnel intent. The anchors are dry-land, region-owned data that future generator work can use for entrance placement and tunnel carving; they do not carve chunks yet.

Acceptance:

- Cave connectivity probe confirms each initial system has connected entrances and chambers.
- No wetland/salt region spams underwater entrances.
- Chunk summaries expose face-open masks for cave propagation after chunks are generated.

## Biome-Worker Briefs

Biome workers should work from atlas briefs and validation artifacts. They should not add global random scatter.

### `C-BIO-RED-001`: Red Mountain

Deliverables:

- caldera rim and crater terrain rules;
- volcanic material profile;
- route-adjacent ash/sacred landmarks;
- lava-tube entrance windows.

Acceptance:

- three fixed cameras see the mountain silhouette;
- caldera core is monument-heavy but not prop-noisy;
- route to Red Mountain has visible pilgrimage cadence.

### `C-BIO-ASH-001`: Ashen Badlands

Deliverables:

- ash apron, ravines, scree fans;
- pilgrimage spine density;
- kwama/ravine cave entrance clusters.

Acceptance:

- route atlas detects ash road continuity;
- landmark histogram favors road debris, ash markers, ribs, cairns, and sparse monumental props.

### `C-BIO-BIT-001`: Bitter Coast

Deliverables:

- blackwater channels, dry hummocks, root/mangrove silhouettes;
- wetland crossing route;
- rare root cave entrances.

Acceptance:

- water coverage is high, but route islands remain dry;
- fixed cameras read as dark wetland, not generic marsh noise.

### `C-BIO-SALT-001`: Salt Basin

Deliverables:

- mirror flats, raised salt ribs, dead causeway route;
- salt spires and silt-shell/cairn content.

Acceptance:

- flatness metrics are high;
- route/causeway remains readable;
- material is not a monoculture of white/cyan.

### `C-BIO-INNER-001`: Inner Sea

Deliverables:

- brackish water shelves, moor islands, old roads and ruins;
- early route/spawn readability.

Acceptance:

- route screenshots show navigable shelves and landmark silhouettes;
- central region supports travel goals without HUD explanation.

### `C-BIO-GRA-001`: Grazelands

Deliverables:

- rolling grasslands, flower basins, sparse acacia/thorn silhouettes;
- caravan/grazing road.

Acceptance:

- strong silhouette every `350-500 m`;
- open ground does not become a flat voxel field.

### `C-BIO-GLASS-001`: Glass-Shard Coast

Deliverables:

- shard ridges, crystal cairn chains, cold coast palette;
- crystal cave entrances.

Acceptance:

- mineral landmarks dominate organic ones;
- screenshots read as glass coast, not snow desert.

### `C-BIO-WG-001`: West Gash

Deliverables:

- ravine floors, switchback roads, redleaf valleys, cave overlooks.

Acceptance:

- route remains passable through vertical terrain;
- redleaf/stone tor/cave cadence is region-specific.

## Validation Scripts

### `C-VAL-001`: Island Atlas Probe

New or extended script should sample the whole atlas and report:

- land/ocean/shelf area;
- region coverage;
- shoreline roughness proxy;
- elevation histogram;
- slope histogram;
- biome patch-size histogram;
- unreachable/dead regions;
- out-of-bounds biome leakage.

Acceptance:

- all regions occupy meaningful area;
- no non-ocean biome leaks outside island bounds;
- average region patch size supports large coherent provinces.

### `C-VAL-002`: Region Identity Atlas

Extend `atlas:views` or add a region preset:

- one overview camera per region;
- one route camera per region;
- one transition camera per important edge.

Metrics:

- grid risk;
- luma/contrast;
- quantized color count;
- horizon silhouette;
- expected landmark/material hits;
- route token visibility.

Acceptance:

- every region has at least two fixed screenshots;
- screenshots and metrics are saved to artifacts with region IDs.

### `C-VAL-003`: Route Continuity Validator

Extend `atlas:routes`:

- sample route every `20-40 m`;
- check region/variant coverage;
- check surface slope;
- check water crossing classification;
- check route material influence;
- scan nearby landmark tokens;
- compute max token gap and max strong-silhouette gap.

Acceptance:

- every named route passes slope and cadence budgets;
- route failures include coordinates and suggested camera anchors.

### `C-VAL-004`: Cave Connectivity Probe

New script:

- sample cave graph nodes and chunk intersections;
- verify entrances connect to expected chambers;
- verify entrances sit on valid dry/wet surfaces based on region rules;
- verify generated chunks produce face-open summary masks.

Acceptance:

- every initial cave system has at least one valid entrance and one connected chamber;
- wetland/salt underwater cave spam is zero.

### `C-VAL-005`: Canonical Chunk Persistence Verifier

Browser verifier after Track A storage boundary:

- generate chunks;
- persist canonical chunks;
- apply edit;
- reload without clearing storage;
- verify canonical chunks and edits are used before generator fallback;
- verify summaries/LOD rebuild from current chunk revision.

Metrics:

- persisted chunk hits;
- generator fallback count;
- summary rebuild count;
- derived LOD hits/misses;
- edit invalidation count;
- p95/max frame and hitches.

Acceptance:

- edited chunk survives reload;
- stale derived data is not reused after edit;
- no correctness gaps/overlaps in route smoke.

### `C-VAL-006`: Source-Of-Truth Guard

Static/test guard for unsafe generator sampling:

- route and coverage diagnostics may use generator only where pristine-world sampling is explicitly intended;
- gameplay, edits, rendering, persistence, and LOD must prefer canonical chunks or derived summaries.

Acceptance:

- report lists all remaining direct `generator.sampleColumn()` runtime uses with owner and migration status.

## Migration Order

### `C0`: Baseline Current Worldgen Artifacts

Commands/artifacts:

- procedural generator tests;
- terrain-surface-lab representative patches;
- route atlas;
- view atlas;
- browser lab;
- current worldgen/region distribution report.

Acceptance:

- baseline artifact paths recorded before behavior changes.

### `C1`: Extract `WorldAtlas` Design Into Data

Implementation later:

- create atlas definitions behind compatibility wrappers;
- keep `sampleWorldRegion()` output compatible.

Acceptance:

- no behavior change expected;
- region tests still pass.

### `C2`: Finite Island Mask

Implementation later:

- enforce meaningful land only inside island envelope;
- convert outside to ocean/shelf.

Acceptance:

- island probe reports finite coherent island;
- spawn and current route cameras remain on land.

### `C3`: Region Graph

Implementation later:

- replace loose region scoring with region graph sampling and explicit edges;
- keep current region IDs.

Acceptance:

- region centers and edge samples deterministic;
- no dead regions.

### `C4`: Macro Terrain Layers

Implementation later:

- add volcano, ridges, basins, coast, route conforming;
- reduce reliance on global noise as primary terrain identity.

Acceptance:

- region identity atlas improves silhouette and grid metrics;
- route continuity does not regress.

### `C5`: Route Graph

Implementation later:

- promote routes from incidental bands to named atlas graph objects;
- expose route sampling for generator and validators.

Acceptance:

- route atlas can report per-route cadence and slope.

### `C6`: Cave Graph

Implementation later:

- add initial cave systems and connectivity validator.

Acceptance:

- caves are discoverable through valid entrances and chunk summaries.

### `C7`: Biome Worker Integration

Implementation later:

- biome workers tune region-specific rosters, palettes, terrain accents, and route/cave props.

Acceptance:

- each worker output passes its brief and shared validation scripts.

### `C8`: Canonical Chunk Persistence

Implementation later, dependent on Track A:

- generated and edited chunks are canonical;
- summaries/LOD are derived and revision-keyed.

Acceptance:

- browser persistence verifier proves reload/revisit correctness.

## Acceptance Metrics

### World Definition

- `8` named regions have non-trivial area and expected center classification.
- `0` non-ocean biome leaks outside island bounds.
- Region average patch size supports large provinces, not checkerboard noise.
- Fixed region screenshots exist for every region.

### Visual Identity

- Red Mountain visible from at least `3` named camera routes.
- Each region has expected landmark/material hits in deterministic probes.
- Whole-view and horizon grid risk do not regress from baseline after macro terrain.
- No region is dominated by one material family unless intentionally flat/salt, and even then route/debris accents must register.

### Routes

- Every named route has slope, material, region, and landmark-cadence metrics.
- Max route-token gap meets per-route budget.
- No route has unclassified water/cliff blockage.
- At least one strong silhouette per region route.

### Caves

- Every initial cave system has a valid dry or intended entrance.
- Connectivity probe finds chamber connectivity.
- Chunk summaries expose open-face continuity for cave propagation.
- Wetland/salt underwater entrance spam is zero.

### Persistence And Source Of Truth

- Generated chunks can be persisted as canonical chunks.
- Edits survive reload.
- Summaries and derived LOD are invalidated by chunk/edit revision.
- Remaining runtime generator sampling is inventoried and justified.

### Performance

- Browser route p95/max frame stays within existing route budgets.
- LOD gaps, resident overlaps, band overlaps, water overlaps, and handoff holes stay at `0` in smoke gates.
- Canonical persistence verifier reports rebuild/hitch metrics.

## Risks

- Macro terrain can break current route and spawn assumptions. Mitigate with route continuity and spawn probes before accepting terrain changes.
- Region identity can become too authored and lose procedural variation. Mitigate with local relief/material noise inside region-owned envelopes.
- Biome workers can drift into incompatible palettes or uniform scatter. Mitigate with per-region briefs and histogram checks.
- Cave carving can invalidate surface and LOD assumptions. Mitigate with chunk-derived summary checks and staged cave rollout.
- Persistence work can accidentally make generator output and persisted chunks compete. Mitigate with explicit `ChunkStore` source-of-truth boundaries.
- Validation metrics can miss subjective identity. Mitigate with fixed screenshots/contact sheets paired with numeric metrics.

## Dependencies

- Track A: `ChunkStore`, edit journal, revision-keyed derived data.
- Track B: render verification, golden views, route smoke, hitch/FPS truth.
- Track D: art direction and object-lab budgets for region props.
- Track E: exploration routes, travel goals, region discovery language.

## Wave 1 Deliverables

- This design document.
- Agreement on `WorldAtlas` as the Track C source layer.
- Initial task IDs and acceptance metrics.
- No generator edits until baseline artifacts and current engine checkpoint are stable.
