# Biome Content Brief: Western Wetlands, Salt Basin, Inner Sea

## Source Of Truth

`src/engine/worldgen-region.ts` is now the macro region authority. Coordinates below are authoring meters; world units are `meters * 10`.

| Region | Center meters | Radius meters | Biome / variant | Ambient |
| --- | ---: | ---: | --- | --- |
| `bitter-coast` | `(-4360, 920)` | `(2320, 2040)` | `marsh` / `marsh_blackwater` | `silt-mist` |
| `salt-marsh-basin` | `(-180, 4040)` | `(2640, 1820)` | `saltflat` / `saltflat_mirror` | `silt-mist` |
| `inner-sea` | `(1320, 120)` | `(2260, 2180)` | `moor` / `moor_shadowglass` | `silt-mist` |
| Island envelope | `(-180, -520)` | `(6400, 5850)` | shoreline/coastal shelf control | mixed |

The current `sampleWorldRegion` fields already expose the wetland levers:

- `westWetlands = max(bitter-coast influence, inner-sea influence * 0.34) * islandInterior`
- `southernSaltBasin = salt-marsh-basin influence * islandInterior`
- `coastalShelf`, `islandInterior`, `regionStrength`, `regionBlend`
- region-forced biome, regional variant, and ambient profile

Treat these as authored biome masses, not noise patches.

## Target Experience

The goal is Morrowind-like scale and authored-feeling cadence: large legible provinces, weird silhouettes, repeatable routes, and memorable content clusters. The player should be able to describe place by shape before reading debug text:

- Bitter Coast: drowned western blackwater, fungal roots, mangrove/cypress walls, short sightlines, rib remains half-sunk in peat.
- Salt-Marsh Basin: huge shallow southern basin, reflective flats, salt crust islands, silt channels, dead pilgrim causeways.
- Inner Sea: central/eastern brackish moor-sea transition, shadowglass pools, low islands, standing stones and old route debris on exposed shelves.

## Terrain Shapes

### Bitter Coast

Use `bitter-coast` center `(-4360, 920)` as the wettest anchor. Shape it as a low western archipelago rather than a uniform swamp:

- Broad base: depressed wetland plain from roughly `x=-6500..-2200`, `z=-1100..2900`.
- Landform grammar: peat shelves, raised root hummocks, mangrove curtains, meandering blackwater cuts, and occasional fungal/root bridges over channels.
- Height target: keep most playable ground close to sea level with local hummock relief; avoid frequent steep ridges except at the east transition into Inner Sea/highlands.
- Blend edge: the `inner-sea` secondary contribution should make Bitter Coast decay into moor and brackish water, not abruptly become dry land.

### Salt-Marsh Basin

Use `salt-marsh-basin` center `(-180, 4040)` as a giant southern shallow pan:

- Broad base: south basin from roughly `x=-2800..2500`, `z=2200..5850`, clipped by island envelope.
- Landform grammar: mirror flats, saline crust shelves, thin raised salt ribs, silt fans, isolated salt spires, and low causeway remnants.
- Height target: flatter than Bitter Coast. Large continuous readable flats with small, rare raised islands are better than noisy relief.
- Basin edge: use `southernSaltBasin` to lower surface and increase `channel`/`dune`; keep peripheral marsh fingers where it blends into Bitter Coast and Inner Sea.

### Inner Sea

Use `inner-sea` center `(1320, 120)` as the brackish central lowland and sea-shelf identity:

- Broad base: central/eastern wet moor and shallow inland sea from roughly `x=-900..3600`, `z=-2000..2300`.
- Landform grammar: open water views, low moor islands, shadowglass pools, exposed old road slabs, standing stones, and rib/cairn silhouettes on shelves.
- Height target: broader visibility than Bitter Coast, less flat than Salt Basin. Alternating low islands and water channels should give navigation landmarks.
- Blend edge: because `westWetlands` includes `inner-sea * 0.34`, the west side can inherit reeds and blackwater without stealing Bitter Coast's dense swamp identity.

## Water And Channel Logic

Current code already biases:

- `westWetlands` increases `moisture`, `drainage`, `channel`, `grove`, and `oldGrowth`, and lowers `basin`.
- `southernSaltBasin` increases `channel`, `dune`, `surfacePatch`, and lowers `basin`/`moisture`.
- `marsh_blackwater` forces water to `surfaceY + 2..4`.
- `saltflat_mirror` forces water to `surfaceY + 1`.
- `moor_shadowglass` lowers terrain and uses darker water material.

Implementation direction:

- Add authored channel fields derived from region-local coordinates, not only FBM. For example, Bitter Coast should have 3-5 named west/east blackwater channels; Salt Basin should have a few long distributary cuts converging into the mirror flats; Inner Sea should have broad inlet shelves and island rings.
- Keep water decisions thresholded by macro region: blackwater pools are normal in Bitter Coast, shallow mirror water is normal in Salt Basin, and Inner Sea water should be broad enough to form vistas.
- Avoid making all low terrain wet. Preserve dry raised route shelves and landmark islands so landmarks remain readable and traversable.
- Make `coastalShelf` amplify water and exposed-shell content at the island edge, but do not let it override the three regional identities.

## Caves And Subsurface

Current cave setup suppresses cave strength under standing water and gives `marsh`/`saltflat` low cave affinity. That is sane for surface readability, but wetlands still need authored entrances:

- Bitter Coast: rare peat/root cave entrances on raised hummocks, not underwater holes. Use `rooted`/`peaty` identity and bias entrances near root bridges, rib remains, and dry cypress islands.
- Salt Basin: shallow saline crust caverns and sinkholes on raised salt ribs. Keep deep caves rare; prioritize small undercut shelves near salt spires and crystal clusters.
- Inner Sea: moor-shadowglass grottos on rocky shelves and route edges. Entrances should align with standing stones, road debris, and exposed basin rims.

High-value code hook later: add a regional cave entrance override that can bypass standing-water suppression only when the entrance anchor is on a dry raised island or route shoulder.

## Routes And Authored Content

Existing route tools:

- `PILGRIM_ROUTE_BANDS` include authored bands from `(0, 0)`, `(220, -340)`, `(-540, 420)`, `(960, -780)`, `(236, -4624)`, and `(-1880, -2860)`.
- Route set pieces can already choose `old_road_causeway`, `pilgrim_lantern`, `rib_arch`, `ash_obelisk`, `bone_chimes`, `velothi_shrine`, `buried_ribs`, `shrine_debris`.
- Wetland route set-piece override already swaps in `fungal_bridge` and `crystal_reeds` when `westWetlands > 0.50` or `magic > 0.62`.

Next route pass should add explicit wetland/basin routes rather than hoping current ash/highland bands cross the new macro regions:

- Bitter Coast smuggler walk: start near `(-6100, 700)`, bends east/northeast through blackwater islands toward `(-3200, 1200)`.
- Bitter-to-Inner crossing: start near `(-3000, 700)`, crosses reed water and low moor shelves toward `(-800, 400)`.
- Salt causeway: start near `(-2100, 3600)`, long east/southeast causeway across mirror flats toward `(1800, 4300)`.
- Inner Sea shelf road: start near `(-600, -700)`, arcs around low islands toward `(2200, 700)`.

Route landmark grammar:

- Bitter Coast: `fungal_bridge`, `crystal_reeds`, `rib_remains`, `mangrove`, `cypress`, `reed_cluster`, `glowcap`.
- Salt Basin: `salt_spire`, `crystal_cluster`, `glass_cairn`, `silt_shell`, `old_road_causeway`, `paver_debris`, `pilgrim_lantern`.
- Inner Sea: `standing_stone`, `ancestor_pillar`, `rib_arch`, `buried_ribs`, `old_road_causeway`, `pilgrim_cairn`, `dead_tree`, `glowcap`.

## Landmark And Palette Targets

Existing landmark IDs to reuse before adding new art:

- Fungal: `glowcap`, `mega_glowcap`, `fungal_bridge`, `lantern_tree`.
- Crystal/salt: `crystal_reeds`, `crystal_cluster`, `salt_spire`, `glass_cairn`.
- Ribs/bones: `rib_arch`, `rib_remains`, `buried_ribs`, `bone_chimes`, `silt_shell`.
- Route/settlement traces: `old_road_causeway`, `paver_debris`, `pilgrim_lantern`, `pilgrim_cairn`, `velothi_shrine`, `shrine_debris`.
- Wetland vegetation: `mangrove`, `cypress`, `willow`, `reed_cluster`, `dead_tree`, `root_stump`.

Palette targets from current material overrides:

- Bitter Coast / `marsh_blackwater`: surface `#354/#465`, subsurface `#243/#354`, water `#134`.
- Salt Basin / `saltflat_mirror`: surface `#FFF/#DEF`, subsurface `#CCB/#EED`, water `#9CF`.
- Inner Sea / `moor_shadowglass`: surface `#546/#768`, subsurface `#435/#657`, water `#245`.

Material direction:

- Bitter Coast should read dark green/black-blue with small bright reed/crystal/fungal accents.
- Salt Basin should read white/cyan/cream with sparse dark debris; avoid turning the whole region into noisy high-contrast speckles.
- Inner Sea should read muted violet-gray/blue-black with occasional pale stone and bone silhouettes.

## Programmatic Verification Ideas

Review wetland changes with deterministic route samples and player screenshots before heavy tuning.
5. Dry-island cave entrances: add a narrow regional cave override for hummocks, salt ribs, and moor shelves after route and surface identity are stable.
