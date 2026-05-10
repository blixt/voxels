# Track D Art Direction Bible

## Goal

Make the game read first as a strange exploration RPG, not a block-building sandbox. The target is Morrowind-like in function: alien ecology, old road culture, ritual landmarks, heavy atmosphere, readable travel goals, and quiet RPG UI. The implementation can remain voxel-based, but the visible language must stop relying on cube novelty, primary colors, craft-grid interfaces, or generic forest/desert tropes.

This document is enforceable. A Track D change is not accepted because it "looks better"; it is accepted when it passes the relevant object-lab, golden-view, route-atlas, HUD, and performance gates listed here.

## Visual Pillars

1. Alien archaeology

Ancient structures should imply a world with pilgrimage, burial, route marking, and ruin reuse. The strongest shapes are stepped, slanted, broken, ribbed, cairn-like, or shrine-like. Avoid readable house, castle, tower, or village-builder silhouettes unless they are heavily eroded and culturally specific.

2. Harsh ecology

Plants and natural objects should feel adapted to ash, salt, fungus, glass, marsh, cold, and wind. Trees should rarely read as ordinary trees near important routes. Prefer dead snags, thorn silhouettes, fungal shelves, glass reeds, mangrove knots, salt spires, rib remains, and clustered low ground cover.

3. Route legibility

Props should help the player infer direction and purpose. Small objects mark paths, medium objects mark thresholds, and large objects anchor vistas. A route should have visible beats before a HUD objective is needed.

4. Silhouette first, ornament second

Every prop family needs a recognizable outline from top, front, and side. If a shape only works because of its material color, it is not strong enough. If it looks like a rectangular stack with trim, it is not Track D-ready.

5. Muted world, sharp accents

The base palette should be dirty, mineral, weathered, or organic. Accents should be rare and meaningful: lantern glow, fungal cyan, glass blue, oxidized green, dull gold, bone, rust, ash-black. Avoid saturated toy colors and single-hue scenes.

6. Measured atmosphere

Sky, fog, and lighting are part of world identity, but they must remain compatible with LOD, screenshot validation, and route visibility. Fog may create mystery; it must not hide missing art or correctness bugs.

## Anti-Goals

- No builder-game first read: no visible hotbar, craft grid, block targeting overlay, material inventory, or block-placement language in the main game HUD.
- No cube-house or plank-box architecture.
- No lollipop trees as route anchors.
- No bright primary-color material families except tiny magical or warning accents.
- No beige-only, blue-only, purple-only, or brown-only art pass.
- No prop accepted from a single camera angle.
- No atmosphere pass that makes terrain unreadable or masks LOD gaps.
- No route content that only appears in debug metrics and not in screenshots.
- No large asset batch without before/after route and performance artifacts.

## Prop And Model Families

### Ash Pilgrim Kit

Objects:
- `pilgrim_lantern`
- `bone_chimes`
- `ashlander_travel_pack`
- `ash_marker`
- small cairn and warning-marker variants when added

Shape rules:
- Thin verticals, hanging forms, off-center weights, broken symmetry.
- The strongest read should be "someone traveled through here" or "this place is ritually marked."
- Lanterns must not become square street lamps. Chimes must not become fence posts.

Material rules:
- Bone, soot, dull metal, ash cloth, ember or lantern accent.
- Two to four visible materials for authored props.
- Dominant material share target: below `0.80` unless the object is intentionally monolithic.

Placement rules:
- Near old roads, shrine approaches, ash routes, and transition areas.
- Small markers can repeat; hero props need spacing and route purpose.

Object-lab gates:
- No horizontal edge clipping.
- No top clipping.
- `materialVariety >= 2` for all non-debris props.
- `crossViewVariation > 0.15` for lantern/chime forms.
- Vertical profile must not be `none`.

### Velothi Ruin Kit

Objects:
- `velothi_shrine`
- `velothi_ziggurat`
- `shrine_debris`
- `ancestor_pillar`
- `old_road_causeway`
- `paver_debris`

Shape rules:
- Stepped, sloped, eroded, and asymmetrical.
- Large forms should have broken edges or negative space.
- Roads and pavers should read as route evidence, not as modern paving.

Material rules:
- Ash stone, warm gray, dull gold, old red-brown, blackened seams.
- Use accent material to imply ritual importance, not decoration.

Placement rules:
- Major silhouettes at approaches, ridges, or route junctions.
- Debris and pavers should bridge empty travel stretches.

Object-lab gates:
- Major structures: `solidVoxelBudget` may be `large`, but huge requires explicit justification in the PR or diary.
- Debris can be `negative-space`, but must be labeled intentional by diagnostics or reviewed manually.
- `dominantMaterialShare < 0.90` for shrines and ziggurats.

### Ashland Ecology Kit

Objects:
- `dead_snag`
- `thorn_tree`
- `rib_arch`
- `buried_ribs`
- `scree_fan`
- `kwama_mound`

Shape rules:
- Wind-shaped, skeletal, low-canopy, or mound-based.
- Horizon interruption matters: ribs, snags, and arches should be visible against sky or fog.
- Avoid ordinary branch balls and generic boulders.

Material rules:
- Soot bark, bone, rust, ash, dark red, muted ochre.
- Mounds may use two close materials, but need contour or accent breaks.

Placement rules:
- Ashlands, red mountains, badlands, road edges, and exposed basins.
- Use rib and snag silhouettes as distant wayfinding marks.

Object-lab gates:
- Snag/rib forms: front and side occupied pixels must exceed family baseline from existing tests.
- Mounds: front aspect ratio should stay wider than tall unless intentionally towered.
- `dominantMaterialShare < 0.90` for authored mounds.

### Salt, Marsh, And Fungal Kit

Objects:
- `crystal_reeds`
- `fungal_bridge`
- `rib_remains`
- `silt_shell`
- `glowcap`
- `mega_glowcap`
- `mangrove`
- `reed_cluster`

Shape rules:
- Clustered vertical reeds, sagging bridges, shelf fungi, root knots, shells, and exposed ribs.
- Fungal bridges should imply crossing or obstruction, not random arches.
- Reeds and crystals should create dense local texture without becoming noisy walls.

Material rules:
- Chalk, silt, oxidized green, bruised violet, fungal cyan, cold glass blue.
- Glowing accents must be rare and spatially meaningful.

Placement rules:
- Wetlands, salt marsh, silt flats, fungal lowlands, water edges.
- Use bridge/remains props to make traversal routes feel authored.

Object-lab gates:
- Hero props: `crossViewVariation > 0.15`.
- Reeds/clusters: no edge clipping at expected sample radius.
- Fungal/glass props: material variety must distinguish body and accent.

## Atmosphere, Sky, Fog, And Lighting

### Ambient Profile Rules

Every region-facing atmosphere profile needs:
- A readable label for debug/HUD snapshots.
- Distinct sky top, horizon, fog, cloud, and lighting behavior.
- A fog end that stays within LOD-supported correctness bounds.
- A screenshot signature visible in the sky and horizon regions.

Required profile reads:
- Ashfall: darker upper sky, low dirty haze, ash shelf, muted ground contrast.
- Ashlands: compressed visibility, brown-gray zenith, rust horizon, high ash shelf, water biased opaque and silty.
- Salt marsh: chalky low horizon, green-gray brine fog, brackish water, shallow depth read.
- Silt mist: pale low horizon, softer depth, low saturation, water/silt ambiguity.
- Blackwater wetlands: short green-black visibility, humid low clouds, darker water, visible route silhouettes at close and mid range.
- Fungal glow: cool horizon glow, damp shadow tint, restrained cyan accents.
- Glass coast: clear blue-cyan distance, low cloud coverage, bright horizon, more transparent glassy water.
- Cold glass: sharp pale sky, cold fog, blue-black material separation.
- Dry haze: warm mineral distance, not orange desert.
- Green canopy: filtered green presence without becoming generic forest.
- Underground: low sky contribution, stronger local darkness, no flat black.

First checkpoint implementation rules:
- Prefer static profile parameters over new per-pixel or per-frame effects.
- Fog/sky/water distinctions should be measurable in unit tests through color distance, cloud coverage, fog distance, and water alpha/fog parameters.
- Regional identity should survive default blending without pushing surface fog beyond LOD-supported bounds.
- Water visuals may vary by preset, but the default mesher path must remain cheap and allocation-free beyond existing color packing work.

### Sky Pass Principles

- Sky should carry region identity before the player reads HUD text.
- Cloud bands and ash shelves should be broad forms, not small noisy speckles.
- Horizon color matters more than zenith color for route screenshots.
- Noise must be cheap and deterministic.
- Do not add texture dependencies until the current procedural pass has been measured and proven insufficient.

Sky gates:
- View-atlas sky region must not be flat.
- At least five ambient profiles should have distinguishable sky/fog color distributions.
- No sky pass may increase p95 render time above Track D budget.

### Fog Principles

- Fog is a composition tool and a correctness boundary.
- It should reveal silhouettes at mid-distance and hide only true distance.
- It must not hide LOD holes, handoff holes, or prop absence.
- Fog color should harmonize with biome and profile, but avoid one-hue scenes.

Fog gates:
- `LOD gaps = 0`.
- `handoff holes = 0`.
- `residentOverlapCount = 0`.
- `bandOverlapCount = 0`.
- Fog end remains at or below the range supported by active LOD rings.

### Lighting And Material Grounding

- Surfaces should feel grounded through contact depth, luma compression, subtle grain, and profile tint.
- Keep direct lighting readable enough for navigation.
- Avoid glossy or plastic reads unless a glass/crystal material is explicitly intended.
- Top surfaces may collect ash or haze tint; verticals should retain enough contrast for silhouettes.

Lighting gates:
- Screenshot ground/material regions should not be dominated by a single color family.
- Reference and shader smoke outputs must remain nonblank and bounded.
- Render p95 must stay within budget.

## HUD And RPG UI Principles

The HUD must say "exploration RPG" rather than "voxel editor."

Primary HUD content:
- Current place or ambient mood.
- Current landmark/discovery, when relevant.
- Current exploration objective.
- Focus skill name, level, and progress only as a quiet RPG signal.

Allowed patterns:
- Compact top-left place/objective strip.
- Small discovery toast.
- Optional compass or bearing strip for named routes and landmarks.
- Debug/performance strip only in lab/debug mode.
- Tooltips may expose detail; visible HUD should stay sparse.

Forbidden patterns:
- Hotbar.
- Material inventory.
- Block targeting overlay.
- Build/place/break prompts as default game language.
- Large floating cards inside gameplay.
- Control-instruction panels in the primary view.

Tone rules:
- Player-facing labels should be names or diegetic descriptions, not raw IDs.
- Skill/objective text should describe exploration, route knowledge, lore, survival, or discovery.
- Debug IDs can remain in artifacts and developer-only snapshots.

HUD gates:
- Existing RPG UI cleanup tests must continue to reject block-editing surfaces.
- Browser smoke must expose valid ambient, objective, focus skill, and progress fields.
- HUD must not cover central traversal/aim area.
- Text must not overlap or overflow at common desktop and narrow viewports.

## Object-Lab Workflow

Use object lab before world integration for every new or materially changed prop family.

Required command pattern:

```sh
mise exec -- bun run scripts/object-lab.ts --id <landmark_id> --seed 1337 --label=<track-d-label>
```

For route families, prefer batch runs over isolated single-object review once batch tooling exists.

Review order:
1. Open `contact-sheet.svg`.
2. Check top, front, and side silhouettes.
3. Read `summary.md` diagnostics.
4. Inspect `report.json` for thresholds.
5. Only then judge a world screenshot.

Object acceptance checklist:
- The object reads from at least two of top/front/side views.
- Sample does not clip horizontal edges or top, unless explicitly reviewed.
- Dominant material share is below the family target.
- Material variety is appropriate to family.
- Vertical profile is intentional.
- Negative space warnings are either absent or explicitly intentional.
- Solid voxel budget is within family budget.
- The prop has a placement rule and at least one golden-view target.

Object rejection triggers:
- Looks like a simple cube stack.
- Only readable from one angle.
- Dominant material share near `1.0` without monolith justification.
- Unintended clipping.
- Route prop has no route purpose.
- Large object has no performance budget note.

## Golden-View Workflow

Golden views are stable screenshots plus metrics used to review world feel and performance.

Required world golden views:
- Ash pilgrim route.
- Velothi ruin approach.
- Salt marsh crossing.
- Fungal lowland.
- Cold glass or highland vista.
- Underground threshold.
- Water-edge or silt-mist view.
- Far LOD vista.

Required prop golden views:
- One representative per Track D prop family.
- One dense cluster view per biome group.
- One route sequence showing small, medium, and large markers together.

Each golden view must record:
- Seed.
- World position.
- Yaw and pitch.
- Expected biome or region.
- Expected ambient profile.
- Visible landmark family, if any.
- Screenshot path.
- Region analysis.
- Draw calls.
- Triangles.
- p95 measured/render work when captured in browser.
- LOD gap, handoff hole, and overlap counts.

Golden acceptance checklist:
- The first read is place/route/landmark, not block grid.
- The sky and fog support the region identity.
- At least one silhouette interrupts terrain or horizon where expected.
- HUD variant reads as RPG UI.
- Debug-hidden variant keeps the world composition readable.
- Performance and LOD metrics pass budgets.

## Budgets

These are initial Track D budgets. Tighten them after baseline artifacts exist.

Object budgets:
- Tiny route clue: `1` to `250` solid voxels.
- Small prop: `251` to `800` solid voxels.
- Medium route marker: `801` to `2,500` solid voxels.
- Large landmark: `2,501` to `9,000` solid voxels.
- Huge landmark: above `9,000` solid voxels; requires explicit justification and before/after performance evidence.

Material budgets:
- Authored props should usually use `2` to `4` visible materials.
- Dominant material share should usually be below `0.80`.
- Monoliths and route debris may exceed `0.80` only with explicit style reason.

Route budgets:
- Track D routes should show a memorable object beat before the player travels `420 m` without a landmark or route clue.
- No single prop family should exceed `35%` of route landmark hits in the primary route atlas unless the route is intentionally themed.
- At least four Track D prop families should appear across fixed route-atlas runs.

Performance budgets:
- p95 measured work: baseline + `15%` maximum.
- p95 render work: baseline + `15%` maximum.
- Stream and mesh p95 must not regress enough to hide in the render budget.
- Draw calls and triangles must be reported for every accepted asset batch.

Correctness budgets:
- `LOD gaps = 0`.
- `handoff holes = 0`.
- `residentOverlapCount = 0`.
- `bandOverlapCount = 0`.
- `waterOverlapCount = 0`.
- Render-ready near samples must remain fully covered in browser smoke.

## Acceptance Gates By Change Type

### Prop-Only Change

Required:
- Object-lab artifact for each changed prop.
- Object-lab metrics satisfy family gates.
- At least one golden prop or world view.
- Focused object/procedural tests if IDs, diagnostics, or placement changed.

### Placement Or World-Cadence Change

Required:
- Route atlas before/after comparison.
- Golden view for each affected route family.
- Browser smoke if route visibility or density changed materially.
- No correctness budget regression.

### Atmosphere Or Renderer Change

Required:
- Shader or render-focused tests where applicable.
- World golden views across affected profiles.
- Browser smoke with render, LOD, and HUD metrics.
- p95 render work within budget.

### HUD Or RPG UI Change

Required:
- RPG UI cleanup tests pass.
- Browser smoke exposes ambient/objective/skill/progress fields.
- Desktop and narrow viewport screenshot review.
- No block-builder language or affordance returns.

### Large Combined Art Batch

Required:
- All relevant gates above.
- Before/after summary with artifact paths.
- Route-atlas metric deltas.
- Object-lab warning deltas.
- Performance delta table.
- Explicit rollback or follow-up plan for any accepted warning.

## Review Rubric

Score every Track D batch from `1` to `5` in these categories:

- Silhouette: recognizable, asymmetric, non-builder forms.
- Regional identity: clear ash/salt/fungal/glass/ruin read.
- Route readability: helps movement and destination choice.
- Material discipline: muted base, meaningful accents, no toy palette.
- Atmosphere: sky/fog/light reinforce place without hiding bugs.
- HUD fit: RPG information, low obstruction, no builder UI.
- Performance: measured and within budget.
- Correctness: no LOD, coverage, or render-readiness regressions.

Minimum acceptance:
- No category below `3`.
- Performance and correctness categories must be `5` by budget definition.
- Any visual category below `4` needs a written follow-up task.

## Artifact Requirements For PRs

Every Track D PR must include:
- Changed prop families or atmosphere/HUD surfaces.
- Object-lab artifact paths when props changed.
- Golden-view artifact paths when visuals changed.
- Route-atlas artifact path when placement/cadence changed.
- Browser smoke or traversal artifact path when render cost could change.
- Summary of failed or suppressed warnings.
- Statement that block-builder HUD surfaces remain absent.

## Coordination Notes

- Asset subagents may own separate prop families in parallel, but only one subagent should tune shared placement frequency for a biome at a time.
- Atmosphere and renderer work must coordinate with LOD/fog correctness owners before increasing fog distance or reducing visibility.
- HUD changes should not depend on art changes; they should consume existing ambient, discovery, objective, and skill snapshot fields.
- Object-lab tooling changes should stay isolated from renderer and generator changes unless a runtime bug is confirmed.
- If a visual improvement requires violating a budget, stop and write a targeted exception proposal before implementation.
