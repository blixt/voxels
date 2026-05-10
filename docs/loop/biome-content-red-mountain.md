# Red Mountain + Ashen Badlands Implementation Brief

Date: 2026-05-08

Goal: make the volcanic center read like a huge, authored Morrowind-like province, not a different random-noise biome. `src/engine/worldgen-region.ts` is now the macro source of truth; implementation should treat its regions as the first director layer and use local fields only to add erosion, landmarks, and traversal detail.

## Source Regions

- `red-mountain`: center `(-520m, -1080m)` / world units `(-5200, -10800)`, radius `1080m x 1020m`, biome `ember`, variant `ember_caldera`, ambient `ashfall`.
- `ashen-badlands`: center `(-840m, -2360m)` / world units `(-8400, -23600)`, radius `2200m x 1760m`, biome `badlands`, variant `ash_wastes`, ambient `ashfall`.
- Transition neighbors that should matter for authored routes: `west-gash` at `(-2720m, -3260m)`, `grazelands` at `(3420m, -3080m)`, and `inner-sea` at `(1320m, 120m)`.

`sampleWorldRegion` already exposes `volcanicHeart` from `red-mountain` and `ashRing` from `ashen-badlands` plus a Red Mountain halo. These should be the main content directors.

## Terrain Shapes

Red Mountain should be a legible massif:

- Build a dominant cone/caldera silhouette around `(-520m, -1080m)`, with high `volcanicHeart` producing the peak province and extra volcanic lift.
- Push the caldera rim into a broken ring rather than a smooth mound: jagged basalt ridges, several saddle cuts, and steep exterior scarps.
- Keep a navigable ash ramp on the south/southwest face, where Red Mountain blends into Ashen Badlands near roughly `(-700m, -1700m)` to `(-1200m, -2100m)`.
- Use `ember_caldera` terrain delta for lift, but add local depressions inside the rim so the peak reads as a cratered crown, not just the tallest hill.

Ashen Badlands should be the ash apron:

- Around `(-840m, -2360m)`, prefer wide, dry, grey-brown ash flats cut by ravines and mesa shelves.
- Use `ash_wastes` to lower and roughen the surface: low basins, diagonal cracked crust, scree fans, and eroded hoodoo fields.
- Increase authored contrast toward the Red Mountain border: denser basalt, more ash markers, more route debris, less vegetation.
- Let the west edge bleed into `west-gash` as broken highland passes rather than a hard biome seam.

## Cave Entrances

Current cave logic already favors badlands/ember, `ember_caldera`, ruggedness, cliffiness, and dry subterranean conditions. The next step should make entrances discoverable and placed like authored content:

- Add cave-mouth anchor windows along caldera rim saddles: northwest `(-1100m, -900m)`, east `(-20m, -1050m)`, and south `(-650m, -1750m)`.
- Add badlands mine/kwama entrances around route-adjacent ravines near `(-1450m, -1750m)`, `(-1400m, -2450m)`, and `(-300m, -2150m)`.
- Bias entrances to face routes or basins: entrance marker density should rise when `ashRing > 0.45`, `caveEntranceStrength > 0.35`, and distance to a pilgrim route band is low.
- Surface tells should reuse existing IDs before adding new assets: `kwama_mound`, `scree_fan`, `ash_marker`, `rib_arch`, `buried_ribs`, `pilgrim_lantern`.

## Routes

Existing pilgrim route bands are already the strongest authored skeleton:

- Band A: `(-540m, 420m) -> (~1016m, -1136m)`, heading `315`, length `2200m`, half-width `72m`. This is the Inner Sea to Red Mountain approach.
- Band B: `(960m, -780m) -> (~-1451m, -1754m)`, heading `202`, length `2600m`, half-width `74m`. This cuts across Red Mountain's south/west flank.
- Band C: `(236m, -4624m) -> (~2038m, -1741m)`, heading `58`, length `3400m`, half-width `82m`. This is the southern/eastern badlands approach.
- Band D: `(-1880m, -2860m) -> (~498m, -719m)`, heading `42`, length `3200m`, half-width `86m`. This should be the main Ashen Badlands to Red Mountain pilgrimage road.

High ROI route work:

- Treat Band D and Band B as the Red Mountain pilgrimage spine. Make them visibly continuous with `old_road_causeway`, `paver_debris`, `pilgrim_lantern`, `pilgrim_cairn`, and `ash_marker`.
- Set-piece spacing already lands every `240m` after the first `180m`; in volcanic regions, the selector already promotes `velothi_ziggurat` and `ash_obelisk`. Lean into that by adding route-side cave entrances and landmark clusters at route intersections, not isolated noise points.
- Material language on route cores should stay dusty/dark: current route colors `#655`, `#887`, `#433`, `#544` fit Red Mountain.

## Landmark Density And Directors

Existing Red Mountain-compatible landmark IDs:

- Sacred/Velothi: `velothi_ziggurat`, `velothi_shrine`, `ash_obelisk`, `ancestor_pillar`, `pilgrim_cairn`, `pilgrim_lantern`, `bone_chimes`.
- Road/ruin debris: `old_road_causeway`, `paver_debris`, `shrine_debris`, `buried_ribs`, `rib_arch`, `scree_fan`.
- Natural volcanic/ash: `basalt_spire`, `boulder`, `hoodoo`, `dead_tree`, `dead_snag`, `ash_marker`, `crystal_cluster`.
- Dweller/travel tells: `kwama_mound`, `ashlander_travel_pack`, `silt_shell`.

Use three density bands:

- Caldera core, `volcanicHeart > 0.55`: sparse but monumental. Favor `basalt_spire`, `ash_obelisk`, `velothi_ziggurat`, `ash_marker`, `crystal_cluster`; avoid too many small props.
- Ash ring, `ashRing > 0.45`: dense authored debris. Favor `old_road_causeway`, `paver_debris`, `scree_fan`, `pilgrim_lantern`, `rib_arch`, `buried_ribs`, `kwama_mound`.
- Route corridor: highest story density within route shoulders. Favor repeated navigational language: road slabs, lanterns, cairns, obelisks, shrine debris, cave-mouth tells.

`ASH_WASTES_LANDMARKS` is currently the strongest authored roster. `EMBER_CALDERA_LANDMARKS` is good for the peak but should remain less noisy and more monumental.

## Palette And Material Targets

Current useful palettes:

- `ember`: surface `#543`, transition `#754`, variant `#764`, accent `#F74`, rock `#433`, subsurface `#654`, variant `#765`.
- `ember_caldera` override: surface `#433`, secondary `#654`, subsurface `#322`, secondary `#433`.
- `badlands`: surface `#A65`, transition `#B87`, variant `#A76`, accent `#854`, rock `#654`, subsurface `#743`.
- `ash_wastes` override: surface `#655`, secondary `#887`, subsurface `#433`, secondary `#544`, lower transition threshold.
- `basaltic` underground: stone `#544`, deep stone `#322`, accent `#F74`.

Target read:

- Red Mountain core: dark basalt, warm lava accents, limited high-contrast orange. It should be black/red-brown first, fiery second.
- Ashen Badlands: grey ash and dusty brown, with bone/ruin accents. Avoid saturating the whole region orange.
- Routes: slightly lighter worn slabs in a dark dust bed so the player can read paths through fog/ashfall.

## Programmatic Verification Ideas

- Region probe: sample a fixed grid over both ellipses and assert dominant `regionId`, `regionalVariantId`, `ambientProfileId`, `volcanicHeart`, and `ashRing` are stable at known anchors.
- Route continuity probe: sample every `40m` along Bands B and D and assert route material influence or route landmarks appear frequently enough to read as continuous.
- Landmark density histogram: bucket landmark IDs by `red-mountain`, `ashen-badlands`, route corridor, and off-route ash ring. Verify caldera is monument-heavy and ash ring is debris-heavy.
- Cave entrance probe: sample candidate anchor windows and assert at least one strong `caveEntranceStrength` cluster or nearby surface tell per window.
- Palette probe: sample surface material colors at representative anchors and assert Red Mountain stays in `#322/#433/#543/#654/#F74` families while Ashen Badlands stays in `#433/#544/#655/#887` families.
- Screenshot route: reuse deterministic camera paths through `(-1880m, -2860m) -> (498m, -719m)` and `(960m, -780m) -> (-1451m, -1754m)` to catch continuity, landmark silhouette, and fog readability regressions.

## Highest ROI Next

1. Add an explicit route/cave director for Bands B and D so cave mouths and landmark clusters key off route distance plus `ashRing`/`volcanicHeart`.
2. Add caldera rim shaping around `red-mountain` so the macro silhouette becomes recognizable from far render.
3. Split `EMBER_CALDERA_LANDMARKS` into core/rim/route sub-rosters so the crater is monumental, the rim is navigational, and the road has dense authored debris.
4. Add verification probes before tuning visuals further; density and route continuity are easy to regress when landmark chance/cell sizes change.
