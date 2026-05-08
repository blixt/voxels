# Bold ROI Plan - 2026-05-08

## Scoring

ROI = `impact * confidence / effort`, each 1-5. Impact is player-visible movement toward a Morrowind-like exploration RPG. Confidence includes how well we can verify it with scripts/browser lab. Effort is implementation plus validation cost.

## Ranked Backlog

| Rank | Change | Impact | Confidence | Effort | ROI | Why Now | Verification |
| ---: | --- | ---: | ---: | ---: | ---: | --- | --- |
| 1 | Route-visible megastructures: ziggurat ruins, rib arches, obelisks, pilgrim lanterns, shrine clusters | 5 | 4 | 2 | 10.0 | The current world lacks memorable silhouettes; concept art is dominated by landmarks, not color nuance | route-atlas required landmarks, object-lab, browser screenshots, draw/triangle budget |
| 2 | Ashland road network: old stone causeways, lantern markers, shrine/cairn breadcrumbs | 5 | 4 | 3 | 6.7 | Exploration needs readable routes and destinations; roads change composition immediately | route-atlas path hits, objective breadcrumb probe, browser route budget |
| 3 | Dramatic sky/weather: ash clouds, storm bands, lightning/glow tint, darker horizon | 5 | 3 | 3 | 5.0 | The current sky is sterile; ambiance shift is visible everywhere and cheap if shader-only | browser screenshot identity, render CPU/frame gates |
| 4 | Salt-marsh basin set piece: water tiles, crystal reeds, fungal bridge caps, rib remains | 5 | 3 | 4 | 3.8 | Strong Morrowind-like biome contrast, but requires more generation work | route-atlas biome route, water/LOD overlap gates |
| 5 | Object silhouette overhaul pass: ash lantern, bone chimes, masks, travel pack, fungal water collector | 4 | 4 | 3 | 5.3 | Ugly props are a direct complaint; object-lab now makes this fast and honest | object-lab diagnostics/contact sheets |
| 6 | RPG travel loop: named pilgrimage route, first shrine objective, discovery reward, journal note | 4 | 4 | 3 | 5.3 | Gives purpose to roads/landmarks without heavy NPC systems | objective tests, browser persistence/objective probe |
| 7 | Large distant landmarks forced into route vistas | 4 | 4 | 2 | 8.0 | Fixes "nothing characteristic" at far view without needing dense content everywhere | route-atlas max notable gap, LOD draw levels |
| 8 | Terrain shape macro pass: reduce blocky terraces, add ravines/plateaus/cliff breaks | 4 | 3 | 4 | 3.0 | Important, but riskier because terrain affects physics, route continuity, LOD | surface continuity, physics, route budget |
| 9 | Better lighting model: ambient occlusion-like vertex shade or material roughness tint | 3 | 3 | 4 | 2.3 | Could improve blockiness, but smaller than silhouettes/sky/routes | reference render/browser screenshot |
| 10 | Creature silhouettes/static encounters | 4 | 2 | 5 | 1.6 | High fantasy value, but without AI/animation can become fake set dressing | object-lab, route screenshots |
| 11 | NPC/dialogue/inventory/equipment | 5 | 2 | 5 | 2.0 | Core RPG long-term, but too slow before place identity exists | unit tests + browser UI |
| 12 | Far-view increase beyond current fog | 3 | 3 | 4 | 2.3 | Nice after vistas are worth seeing; currently risks LOD cost for weak content | LOD/perf route gates |

## Execution Order

1. Implement route-visible megastructures and old-road landmarks in worldgen.
2. Delegate sky/weather and RPG-loop slices in parallel.
3. Validate with object-lab, route-atlas, browser lab, and full focused tests.
4. Commit only changes that visibly move screenshots or route metrics without FPS/LOD regressions.
5. Re-rank after each checkpoint using the newest artifacts, not vibes.

## Current Anti-Pattern To Avoid

Small color and lighting tweaks can be worthwhile later, but the current screenshot remains dominated by generic block terrain and weak landmarks. Do not spend another cycle on palette-only changes unless a browser artifact shows a clear visual metric or screenshot improvement.

## Re-Rank After First Checkpoints

Completed since the first ranking:

- Route-visible ashland megastructures, causeways, rib arches, obelisks, and pilgrim lanterns.
- Direct route-atlas coverage for those landmark families.
- Removal of the old player-facing material gather/build modules and HUD surface.
- Browser-lab visual gates for blank/too-dark screenshots and legacy HUD absence.

The current problem is no longer "there are zero landmarks"; it is that the whole scene can still read as voxel blocks with scattered props. The next work should change composition, traversal feel, and measured moving performance, not only palette values.

| Rank | Change | Impact | Confidence | Effort | ROI | Why Now | Verification |
| ---: | --- | ---: | ---: | ---: | ---: | --- | --- |
| 1 | Moving-performance truth pass: make walking FPS/frame-time measurement impossible to misread and gate live-forward p95/max | 5 | 5 | 2 | 12.5 | User saw 10 FPS while the old visible counter reported nonsense; all later content depends on honest movement cost | `run-browser-route-trace --benchmark=live-forward`, owned-browser wall/frame metrics, HUD smoke |
| 2 | Terrain grid-breaker pass: diagonal strata, cracked ash crust, rubble/salt patches, less perfect terrace read | 5 | 4 | 3 | 6.7 | The dominant complaint is still Minecrafty/block-grid terrain; material-only changes are not enough unless they break the grid read | browser-lab grid dominance, route p95/max, procedural material distribution tests |
| 3 | Salt-marsh basin set piece pack: blackwater pads, crystal reeds, fungal bridge shelves, rib remains | 5 | 4 | 3 | 6.7 | The inspiration art has a strong salt-marsh identity; it gives one biome an immediate non-generic look | route-atlas marsh route, object-lab, water/LOD overlap gates |
| 4 | Ashland travel kit: bone chimes, ashlander lanterns, travel packs, mask markers, small shrine silhouettes | 4 | 4 | 2 | 8.0 | Ugly generic props are a direct weakness; small distinctive objects can be validated in isolation | object-lab diagnostics/contact sheets, browser screenshot, draw budget |
| 5 | Object-lab quality gates: silhouette area, clipping, material count, height/width ratios, contact-sheet notes | 4 | 5 | 2 | 10.0 | Delegated asset work needs self-monitoring or it will become subjective and slow | `bun test tests/object-lab.test.ts`, object-lab reports |
| 6 | Distant-vista scan harness: route-visible landmark detection off the route line | 4 | 4 | 2 | 8.0 | Big landmarks matter when seen at a distance, not only when the route hits their exact footprint | route-atlas visible-nearby failures |
| 7 | Storm sky rebuilt safely: cloud shelf, ash streaks, horizon glow with luma/color gates active | 5 | 3 | 3 | 5.0 | Ambiance is a whole-screen multiplier, but a previous shader attempt caused a black frame | owned-browser luma/contrast/color gates, frame budget |
| 8 | Causeway traversal polish: routes that naturally lead to shrines, obelisks, and safe viewpoints | 4 | 4 | 3 | 5.3 | Roads should shape player behavior, not exist as decoration | route objectives, route-atlas gaps, journal/objective tests |
| 9 | Reduce streaming spikes while moving: amortize chunk/LOD work and expose moving backlog honestly | 5 | 3 | 4 | 3.8 | If the user's 10 FPS report reproduces, this jumps to rank 1 for implementation | live-forward trace, moving HUD backlog metrics |
| 10 | Better lighting/ambient occlusion-like vertex shade | 4 | 3 | 4 | 3.0 | Could improve depth and reduce flat voxel look, but shader correctness risk is higher | reference render, screenshot gates, GPU/draw budget |
| 11 | Landmark density director per route: ensure every 250-350 m has a meaningful silhouette or route token | 4 | 3 | 3 | 4.0 | Fixes empty stretches faster than large biome rewrites | route-atlas notable-gap metrics |
| 12 | First non-combat creature silhouettes as static world objects: silt-shell walkers, insect husks, pack-beast remains | 4 | 3 | 4 | 3.0 | Characteristic world read, but bad silhouettes can look worse than no creatures | object-lab, route screenshot, triangle/draw budgets |
| 13 | Discovery gameplay pass: shrine blessings, route lore cards, skill XP tuned to travel and findings | 4 | 4 | 3 | 5.3 | Makes exploration feel like RPG progression without inventory clutter | unit tests, browser objective probe |
| 14 | Weather-driven biome audio/visual state placeholders in HUD/debug snapshot | 3 | 4 | 2 | 6.0 | Good ambiance hook, low cost, but less visible without audio/assets | snapshot tests, browser HUD smoke |
| 15 | Far-view distance increase after content and LOD correctness hold | 4 | 3 | 4 | 3.0 | User wants to see farther, but distance is only valuable once far content is distinctive | owned-browser LOD overlap/gap/fps gates |
| 16 | Caves/underground entrances visible from surface roads | 4 | 3 | 4 | 3.0 | Strong Morrowind exploration promise, but traversal correctness risk | underground discovery tests, physics route probes |
| 17 | Hand-authored route seed/save fixtures for repeatable visual QA | 3 | 5 | 2 | 7.5 | Faster iteration and honest comparisons across changes | scripts load fixed camera/route seeds |
| 18 | Screenshot diff dashboard: compare current browser/object-lab output against prior checkpoint | 3 | 4 | 3 | 4.0 | Prevents barely visible tweaks from masquerading as progress | artifact comparison script |
| 19 | UI typography/layout final polish | 3 | 4 | 3 | 4.0 | Current UI is clean enough; polish should follow world/perf work | browser screenshots desktop/mobile |
| 20 | NPC/dialogue/equipment foundation | 5 | 2 | 5 | 2.0 | Core RPG eventually, but premature before place identity and performance are solid | unit tests + browser UI |

Execution order for the next loop:

1. Verify and harden moving-performance truth.
2. Start the terrain grid-breaker pass, because it directly addresses the Minecrafty read and can be measured.
3. Let delegated object-lab/performance investigations finish while implementing terrain/material changes.
4. Use the next checkpoint to either continue terrain if metrics improve, or immediately switch to moving-performance implementation if the live-forward trace reproduces the user's 10 FPS report.

## Re-Rank After Salt-Marsh And Vista Harness Checkpoint

Completed since the second ranking:

- Route-atlas now credits visible-nearby landmark silhouettes instead of only direct route hits.
- Salt-marsh/fungal basin coverage now includes `crystal_reeds`, `fungal_bridge`, and `rib_remains`.
- Regional water override materials are classified as procedural water, fixing a correctness issue in object sampling and water/material handling.

The newest browser lab stayed performant (`14.00 ms` route max frame, no LOD overlap, no handoff holes), but the visual grid metric stayed at `0.68`. That means the next work should favor larger readable silhouettes, sky/composition, or terrain breakup with strict screenshot gates.

| Rank | Change | Impact | Confidence | Effort | ROI | Why Now | Verification |
| ---: | --- | ---: | ---: | ---: | ---: | --- | --- |
| 1 | Object-lab representative-root upgrade: choose central/high-density samples and flag edge/clipped roots harder | 4 | 5 | 2 | 10.0 | Delegated asset work is only useful if workers inspect representative, non-water, non-clipped objects | object-lab tests, before/after reports for new basin IDs |
| 2 | Polish weakest basin silhouettes: fungal bridge thickness/material variety and crystal reed material breakup | 5 | 4 | 3 | 6.7 | Current new props exist but still look first-pass; object-lab already identifies the flaws | object-lab material/scale warnings, route screenshots, draw budget |
| 3 | Storm sky rebuilt safely with luma/color gates active | 5 | 3 | 3 | 5.0 | Whole-screen ambiance gain and less block focus, but previous shader attempt failed black-frame gates | owned-browser screenshot luma/grid/color, route frame budget |
| 4 | Landmark density director: every 250-350 m route stretch gets a meaningful vista token | 4 | 4 | 3 | 5.3 | Route-atlas can finally measure this honestly via visible-nearby scanning | route-atlas notable-gap and vista-hit metrics |
| 5 | Terrain grid-breaker geometry/material pass, second attempt | 5 | 3 | 4 | 3.8 | Still essential, but the first shader attempt proved this must be incremental and heavily gated | browser grid metric, procedural terrain tests, route physics |
| 6 | Ashland travel-kit props: bone chimes, masks, lantern variants, travel packs | 4 | 4 | 3 | 5.3 | Directly targets ugly generic props while staying isolated enough for subagents | object-lab contact sheets and warnings |
| 7 | Streaming/residency spike reduction during movement | 5 | 3 | 4 | 3.8 | Current browser lab is fine, but user saw severe walking drops; keep watching live-forward traces | live-forward p95/max stream/mesh/LOD |
| 8 | Far-view distance increase after density/sky improvements | 4 | 3 | 4 | 3.0 | Seeing farther only helps after far content and LOD correctness are worth trusting | owned-browser LOD/fps/gap gates |

## Re-Rank After Basin Polish And Storm Shelf Checkpoint

Completed since the third ranking:

- Object-lab representative-root selection now avoids edge/clipped samples for the weak wet/set-piece landmarks.
- `crystal_reeds` and `fungal_bridge` are warning-free, three-material object-lab samples.
- The renderer has a cheap ambient sky pass with storm shelf/fungal horizon tint, validated in browser.

The grid metric still has not moved from `0.68`, so the next loop should stop treating individual props as the main blocker and attack the terrain/composition read directly.

| Rank | Change | Impact | Confidence | Effort | ROI | Why Now | Verification |
| ---: | --- | ---: | ---: | ---: | ---: | --- | --- |
| 1 | Terrain/composition grid-breaker, second attempt: larger diagonal strata, eroded edges, non-rectangular rubble/salt pads | 5 | 4 | 4 | 5.0 | This is the blocker the metrics and screenshots both agree on | browser grid metric, route physics continuity, material distribution tests |
| 2 | Landmark density director using vista scan: route stretches get silhouette tokens every 250-350 m | 4 | 4 | 3 | 5.3 | Existing landmarks are better, but empty or block-dominated stretches still need composition | route-atlas vista-hit/gap metrics, browser route budget |
| 3 | Live-forward movement spike audit after content changes | 5 | 4 | 2 | 10.0 | Route lab is clean, but user reported severe walking drops; verify before heavier far-view work | live-forward p95/max stream/mesh/LOD, HUD wall-FPS/work-ms |
| 4 | Ashland travel-kit props: bone chimes, masks, lantern variants, travel packs | 4 | 4 | 3 | 5.3 | Good delegated work now that object-lab root selection is trustworthy | object-lab contact sheets and warnings |
| 5 | Stronger sky/weather iteration: lightning glow, cloud shelf shape, ash streak movement | 4 | 3 | 3 | 4.0 | Sky is now correct enough to iterate, but grid/composition still matters more | owned-browser luma/color/grid and route frame budget |
| 6 | Far-view distance increase after spike audit and density pass | 4 | 3 | 4 | 3.0 | The LOD system is clean, but farther view without better composition risks showing more grid | owned-browser LOD/fps/gap gates |

## Re-Rank After Movement Spike Budget Checkpoint

Completed since the fourth ranking:

- Live-forward walking max frame dropped from `21.00 ms` to `8.20 ms`.
- Stream outliers dropped from `17.20 ms` to `4.10 ms`.
- Mesh outliers dropped from `13.10 ms` to `3.70 ms` after rejecting an over-tight cap that starved readiness.
- Full browser route max is now `8.60 ms` with no LOD overlap, no handoff holes, and all near samples render-ready.

The performance foundation is much safer for the next visual push. The strongest remaining user-visible blocker is still the `0.68` grid-dominance metric and the blocky terrain read.

| Rank | Change | Impact | Confidence | Effort | ROI | Why Now | Verification |
| ---: | --- | ---: | ---: | ---: | ---: | --- | --- |
| 1 | Terrain/composition grid-breaker, second attempt | 5 | 4 | 4 | 5.0 | Movement budget now has headroom; the world still reads too blocky | browser grid metric, route physics continuity, material distribution tests |
| 2 | Landmark density director using vista scan | 4 | 4 | 3 | 5.3 | Better performance means we can add route composition deliberately, not blindly | route-atlas vista-hit/gap metrics, browser route budget |
| 3 | Ashland travel-kit props delegated through object-lab | 4 | 4 | 3 | 5.3 | Object-lab root selection is now good enough for parallel asset work | object-lab contact sheets and warnings |
| 4 | Stronger sky/weather iteration | 4 | 3 | 3 | 4.0 | Useful mood multiplier, but not the main blocker while grid dominance is unchanged | owned-browser luma/color/grid and route frame budget |
| 5 | Far-view distance increase | 4 | 3 | 4 | 3.0 | Performance now supports experimentation, but far content/composition should improve first | owned-browser LOD/fps/gap gates |

## Re-Rank During Terrain Grid-Breaker Checkpoint

Completed or in progress since the movement budget checkpoint:

- Route-atlas now enforces 300 m route-stretch coverage and reports tokenless windows.
- The new gate initially failed with `3` tokenless `ash-glass-traverse` windows, then passed after adding terrain-route tokens for wind-cut steppe, salt crust, and ash crust.
- Terrain now has diagonal top-surface fracture blending and small eroded crust height breakup.
- `pilgrim_lantern` was delegated through object-lab and now has a three-material hanging-cage silhouette.

The browser visual metric did not move: the final browser lab still reports grid dominance `0.68` with failures none. Stop adding micro surface noise and switch to composition-scale changes.

| Rank | Change | Impact | Confidence | Effort | ROI | Why Now | Verification |
| ---: | --- | ---: | ---: | ---: | ---: | --- | --- |
| 1 | Composition density director: guarantee foreground/midground silhouettes near route views, not just sampled route tokens | 5 | 4 | 3 | 6.7 | Route stretch coverage is now measured; the next visible issue is whether the camera actually sees strong forms | route-atlas vista gaps, owned-browser screenshot grid/edge metric, draw budget |
| 2 | Stronger terrain breakup, third attempt, only if browser grid improves this pass | 4 | 3 | 4 | 3.0 | Continue only if the browser proves surface breakup moves the metric | owned-browser grid dominance and route physics |
| 3 | Stronger sky/weather: ash streak motion, lightning shelf, horizon glow | 4 | 4 | 3 | 5.3 | Whole-screen ambiance can reduce block focus with low geometry cost | owned-browser luma/color/grid and frame budget |
| 4 | Ashland travel-kit pack: masks, bone chimes, travel packs, water collector | 4 | 4 | 3 | 5.3 | Object-lab workflow is working for delegated prop upgrades | object-lab warnings/materials/contact sheets |
| 5 | Far-view distance experiment with fixed route/vista screenshots | 4 | 3 | 4 | 3.0 | Worth trying only after visible far content density is stronger | LOD overlap/gap/fps gates |
| 6 | Discovery gameplay pass around route tokens and shrine blessings | 4 | 4 | 3 | 5.3 | The route atlas now has meaningful terrain and silhouette events that can feed RPG progression | unit tests, browser objective probe |

## Re-Rank After Strong Silhouette Director

Completed since the terrain grid-breaker checkpoint:

- Route-atlas now separates ordinary route-token coverage from strong visible silhouette coverage.
- Vista scanning now reaches `96 m`, which raised the honest baseline from `46.4%` to `57.0%` strong coverage before any generation changes.
- Main traversal routes now have an old-route skyline director for dusty/open terrain. Final route-atlas coverage is `87.7%` strong silhouette windows with only `22` empty windows.
- Object-lab now has a batch route-landmark comparison mode and exposed the next object quality queue.
- Live-forward movement remained healthy after density increased: max gameplay frame `7.30 ms`.

The next work should still be bold. The density director improves route composition, but it also makes weak hero props more visible. The highest ROI is now to replace or redesign the weakest visible skyline props, while a sky/atmosphere pass remains a whole-screen fallback if the browser grid metric stays stubborn.

| Rank | Change | Impact | Confidence | Effort | ROI | Why Now | Verification |
| ---: | --- | ---: | ---: | ---: | ---: | --- | --- |
| 1 | Replace weak skyline prop families: ash obelisk material breakup, rib arch/remains scale, ziggurat sample/LOD budget | 5 | 5 | 3 | 8.3 | Route director makes these objects visible; object-lab batch identified concrete warnings | `object-lab --batch route-landmarks`, focused shape tests, draw/triangle budget |
| 2 | Full browser screenshot verdict on skyline density | 5 | 4 | 2 | 10.0 | Route metrics moved; need to verify actual player view/grid metric before piling on content | owned-browser luma/grid/color, route p95/max, LOD overlap/gap gates |
| 3 | Stronger sky/atmospheric layering: ash streaks, lightning shelf, far haze silhouettes | 5 | 4 | 3 | 6.7 | Whole-screen ambiance can shift the Minecrafty read without much geometry | owned-browser luma/color/grid, shader black-frame gates, route frame budget |
| 4 | Object-lab distinctiveness score: asymmetry, negative space, generic-envelope comparison | 4 | 5 | 3 | 6.7 | Batch comparison exists; now it should detect whether props are distinctive, not only valid | object-lab tests and comparison report |
| 5 | Discovery gameplay pass using route and terrain tokens | 4 | 4 | 3 | 5.3 | Route atlas now emits meaningful cadence signals that can drive RPG travel memory | objective tests, browser objective probe |
| 6 | Far-view distance experiment after prop and atmosphere quality | 4 | 3 | 4 | 3.0 | Strong route silhouettes help, but far view should wait for browser correctness and prop polish | owned-browser LOD/fps/gap gates |

## Re-Rank After Skyline Prop And Atmosphere Polish

Completed since the strong silhouette director:

- `ash_obelisk` now has warm vertical inlays and no dominant-material warning.
- `rib_arch` and `rib_remains` now have denser multi-rib silhouettes and no object-lab warnings.
- Ashfall/fungal skies now include stronger shelf haze, streaks, horizon tint, and luma-safe tests.
- Browser lab stayed clean and route max frame improved to `8.10 ms`; visual color buckets rose to `98`.
- The grid metric still stayed at `0.68`.

The next decision should be based on the stubborn grid read. Silhouettes, prop quality, and sky color now moved without performance regression, but axis-aligned dominance did not. Invest in a screenshot-aware diagnostic and a lighting/depth pass before more object density.

| Rank | Change | Impact | Confidence | Effort | ROI | Why Now | Verification |
| ---: | --- | ---: | ---: | ---: | ---: | --- | --- |
| 1 | Screenshot composition harness: fixed route cameras, ground/sky/object masks, axis-edge by region | 5 | 5 | 3 | 8.3 | Current grid metric is too blunt; need to know whether terrain, horizon, or HUD/camera dominates | owned-browser screenshots, per-region edge metrics, artifact diff |
| 2 | Lighting depth/contact shading pass | 5 | 4 | 3 | 6.7 | Stronger object and terrain depth may reduce flat block read without more geometry | browser luma/stddev/grid, shader black-frame gates, route render budget |
| 3 | Ziggurat/obelisk LOD budget and silhouette sampling | 4 | 4 | 3 | 5.3 | The object-lab still marks big skyline props huge; make them readable without excessive voxel mass | object-lab batch, draw/triangle budget |
| 4 | Object-lab distinctiveness score | 4 | 5 | 3 | 6.7 | Warning-free does not guarantee distinctive; add asymmetry/negative-space gates | object-lab tests and comparison report |
| 5 | Discovery gameplay pass using route and terrain tokens | 4 | 4 | 3 | 5.3 | Visual route identity is now better; gameplay can start rewarding it | objective tests, browser objective probe |
| 6 | Far-view distance experiment | 4 | 3 | 4 | 3.0 | Wait until screenshot harness shows far composition is worth exposing | owned-browser LOD/fps/gap gates |

## Re-Rank After View Atlas And Bone Chimes

Completed since the screenshot diagnostic:

- `bun run atlas:views` now captures five deterministic browser screenshots and reports per-region visual metrics.
- `bone_chimes` adds a warning-free old-road prop family and route-atlas credits it with `+35` route landmark hits.
- Browser performance and correctness held: owned-browser route p95/max `4.70/9.60 ms`, LOD overlap/gaps/handoff holes `0/0/0`.
- The old global browser grid metric still did not move: saturation/grid/color `0.37/0.68/94`.

This confirms the next pass must change camera-scale composition, not add another small route object. Use the five-view atlas as the comparison baseline.

| Rank | Change | Impact | Confidence | Effort | ROI | Why Now | Verification |
| ---: | --- | ---: | ---: | ---: | ---: | --- | --- |
| 1 | Ziggurat/obelisk lower-fill silhouette rewrite: cut solid mass, add broken steps/door voids/side towers | 5 | 5 | 3 | 8.3 | View/object-lab show the skyline props are visible but still rectilinear and huge | object-lab batch, view-atlas center/horizon grid, draw/triangle budget |
| 2 | Ground interrupter set: large non-rectangular ash drifts, salt cracks, scree fans near route cameras | 5 | 4 | 4 | 5.0 | Lower-ground color/grid remains weak; this attacks the foreground read directly | view-atlas lower-ground grid/color, route physics continuity, browser route budget |
| 3 | View-atlas comparison gate: require full-baseline deltas and fail blank/low-color fixed views | 4 | 5 | 2 | 10.0 | The tool exists; now make it a reliable guard against barely visible tweaks | `bun run atlas:views -- --compare-to=<baseline>` |
| 4 | Stronger atmospheric horizon separation without fragment-contact risk | 4 | 4 | 3 | 5.3 | Horizon is still the grid driver, but previous fragment shader attempts black-framed | view-atlas horizon grid/luma, browser black-frame gates |
| 5 | Object-lab distinctiveness score: asymmetry, negative space, generic-envelope comparison | 4 | 5 | 3 | 6.7 | Warning-free props can still be generic; distinctiveness needs a numeric gate | object-lab tests and comparison report |
| 6 | Discovery gameplay pass using route and terrain tokens | 4 | 4 | 3 | 5.3 | Valuable, but place identity and view correctness still lead | objective tests, browser objective probe |

## Re-Rank After Megastructure Lower-Fill

Completed since the view-atlas checkpoint:

- View atlas screenshots are now HUDless and include close `ziggurat-approach` / `obelisk-approach` hero views.
- `ash_obelisk` is warning-free in object-lab, `8351 -> 6345` voxels.
- `velothi_ziggurat` is still huge but much less filled, `40224 -> 24198` voxels and fill `0.425 -> 0.255`.
- Browser route p95/max stayed healthy at `4.60/7.40 ms`; LOD overlap/gap/handoff gates stayed clean.
- Global visual grid stayed `0.68`, and HUDless screenshots make the next blocker obvious: foreground/horizon terrain surfaces.

| Rank | Change | Impact | Confidence | Effort | ROI | Why Now | Verification |
| ---: | --- | ---: | ---: | ---: | ---: | --- | --- |
| 1 | Ground interrupter set: non-rectangular ash drifts, salt cracks, scree fans, broken road shoulders | 5 | 5 | 4 | 6.3 | HUDless atlas shows terrain/road pixels dominate the Minecrafty read | view-atlas lower-ground/horizon grid and colors, route physics continuity, browser route budget |
| 2 | View-atlas world-only baseline gate and per-view budgets | 4 | 5 | 2 | 10.0 | The tool is useful; make future claims compare against the clean seven-view baseline | `bun run atlas:views -- --compare-to=artifacts/view-atlas/20260508T045619Z-megastructure-hudless-atlas/report.json` |
| 3 | Stronger atmospheric horizon separation without fragment-contact risk | 4 | 4 | 3 | 5.3 | Terrain horizon still has high edge load; atmosphere may soften distant block read cheaply | view-atlas horizon grid/luma, browser black-frame gates |
| 4 | Push ziggurat below huge budget with more negative space | 3 | 4 | 3 | 4.0 | Good cost target, but current screen metrics say ground is now higher ROI | object-lab target `<18000` voxels, route coverage |
| 5 | Object-lab distinctiveness score | 4 | 5 | 3 | 6.7 | Needed for scalable asset delegation, but not the immediate visible blocker | object-lab tests and comparison report |
| 6 | Discovery gameplay pass using route and terrain tokens | 4 | 4 | 3 | 5.3 | Place identity still needs visual grounding before deeper RPG UI work | objective tests, browser objective probe |

## Re-Rank After Old-Road Surface Lab And Ash Haze

Completed since the megastructure checkpoint:

- `bun run lab:terrain` now gives a cheap terrain/route surface report before browser capture.
- Old-road columns expose `pilgrimRouteInfluence`, so ambient and future gameplay can react to being on a route.
- Route surfaces are now narrower broken ash-stone/salt paver bands instead of broad grassy corridors.
- Dry old-road views resolve to `ashfall`; wet/salt routes resolve to `silt-mist`.
- Route atlas max notable gap improved from `108.0 m` to `72.0 m` with no lost route coverage.
- View atlas mixed result: `ziggurat-approach` center/lower grid improved, `obelisk-approach` lower-ground improved, but origin and obelisk center still show too much block structure.

The important learning: atmosphere helps the close route views, and route identity is now real data, but the foreground is still too rectilinear. The next work should create larger shape-language changes or reduce patch contrast at render time; do not spend another cycle on small color swaps.

| Rank | Change | Impact | Confidence | Effort | ROI | Why Now | Verification |
| ---: | --- | ---: | ---: | ---: | ---: | --- | --- |
| 1 | Distance/material modulation shader pass with world-space ash/dust and lower far-LOD contrast | 5 | 4 | 3 | 6.7 | Large surface patches remain the dominant visual issue; shader-side modulation can affect every foreground/horizon pixel with little CPU cost | view-atlas luma/color/grid, black-frame gate, owned-browser frame budget |
| 2 | Foreground silhouette interrupter pack: half-buried ribs, scree fans, shrine plinth debris, paver islands | 5 | 4 | 4 | 5.0 | Screenshots need actual non-rectangular shapes in the camera foreground, not more background tokens | terrain lab patch warnings, object-lab batches, route atlas, browser draw budget |
| 3 | Terrain-lab comparison mode and thresholds | 4 | 5 | 2 | 10.0 | The lab is useful; adding compare/budget mode makes future ground claims faster and less subjective | `bun run lab:terrain -- --compare-to=...`, focused tests |
| 4 | Route-aware gameplay/discovery hooks | 4 | 4 | 3 | 5.3 | `pilgrimRouteInfluence` is now available; it can drive RPG exploration memory and skill gains | objective tests, browser objective probe |
| 5 | Push ziggurat below huge budget with negative space | 3 | 4 | 3 | 4.0 | Still worthwhile, but current view metrics point more at surface composition than the ziggurat voxel count | object-lab target `<18000`, route coverage |
| 6 | Far-view distance experiment | 4 | 3 | 4 | 3.0 | Wait until far surfaces read less like chessboards | owned-browser LOD/fps/gap gates |

## Re-Rank After Terrain-Lab Compare And Shader Rejection

Completed since the old-road haze checkpoint:

- Terrain lab now has `--compare-to` and reports aggregate/per-patch deltas.
- A shader-side ash/distance modulation attempt was rejected by the view atlas: every view blanked (`luma=0.0`, `colors=3`), then the renderer edit was reverted.

The next highest-confidence visual path is geometry/composition that the existing generator and object-lab can verify. Another shader attempt needs a tighter shader-specific smoke gate first.

| Rank | Change | Impact | Confidence | Effort | ROI | Why Now | Verification |
| ---: | --- | ---: | ---: | ---: | ---: | --- | --- |
| 1 | Foreground silhouette interrupter pack: half-buried ribs, scree fans, shrine debris, paver islands | 5 | 4 | 4 | 5.0 | Shader path just failed; object/terrain generation has better current verification | terrain-lab compare, object-lab, view-atlas, owned-browser budget |
| 2 | Object-lab distinctiveness score for foreground/debris forms | 4 | 5 | 3 | 6.7 | New assets need gates for asymmetry and negative space, not only no warnings | object-lab tests and comparison report |
| 3 | Dedicated shader smoke harness for small WGSL edits | 4 | 4 | 3 | 5.3 | Full atlas catches failures, but too late for fast shader iteration | one-view black-frame gate, typecheck/build, atlas |
| 4 | Route-aware gameplay/discovery hooks | 4 | 4 | 3 | 5.3 | `pilgrimRouteInfluence` is available, but visual foreground still leads | objective tests, browser objective probe |
| 5 | Push ziggurat below huge budget with negative space | 3 | 4 | 3 | 4.0 | Still worthwhile, but less visible than foreground shape breakup | object-lab target `<18000`, route coverage |
| 6 | Far-view distance experiment | 4 | 3 | 4 | 3.0 | Wait until far surfaces read less like chessboards | owned-browser LOD/fps/gap gates |

## Re-Rank After Foreground Interrupter Pack

Completed since the rejected shader attempt:

- Added old-road foreground forms: `paver_debris`, `scree_fan`, `shrine_debris`, and `buried_ribs`.
- Added `bun run lab:object` for faster isolated asset review.
- Route atlas recovered to strong silhouette coverage `87.7%`, with `+3` distinct route landmarks versus the paver-only pass.
- Browser performance held: traversal p95/max `4.50/7.80 ms`, route p95/max `4.70/9.20 ms`, draw calls `511`, failures none.
- View atlas is still mixed: close approach screenshots are more authored and moodier, but darker ground contrast worsened close-view grid metrics. Whole-browser visual grid remains `0.68`.

The next ROI is less about adding assets and more about making the evaluation gates stricter so future work cannot pass by being merely darker or busier.

| Rank | Change | Impact | Confidence | Effort | ROI | Why Now | Verification |
| ---: | --- | ---: | ---: | ---: | ---: | --- | --- |
| 1 | View-atlas comparison budget gate with per-view center/lower/horizon thresholds and regression output | 4 | 5 | 2 | 10.0 | The latest pass proved screenshots can look moodier while grid metrics regress | `bun run atlas:views -- --compare-to=...`, tests for failing/passing budgets |
| 2 | Object-lab distinctiveness and negative-space score | 4 | 5 | 3 | 6.7 | `scree_fan` and `buried_ribs` are intentionally sparse, but the lab treats them like bad low-fill assets | object-lab tests, route batch comparison |
| 3 | Fast shader smoke harness before any WGSL retry | 4 | 4 | 2 | 8.0 | The previous shader attempt black-framed; shader work needs a one-view gate before the full atlas | one-view luma/color/frame gate, typecheck/build |
| 4 | Contact-depth/lighting pass with strict atlas budgets | 5 | 3 | 4 | 3.8 | Block edges need softer depth cues, but it must not repeat the failed broad shader edit | shader smoke, view-atlas, owned-browser budget |
| 5 | Route-aware RPG hooks: named old roads, shrine blessings, Cartography/Naturalist/Lore progress | 5 | 4 | 3 | 6.7 | `pilgrimRouteInfluence` and richer route landmarks can now support exploration gameplay | objective/skill tests, browser objective probe |
| 6 | Ziggurat budget/negative-space follow-up | 3 | 4 | 3 | 4.0 | Still useful, but not the current screenshot bottleneck | object-lab target `<18000`, route coverage |
| 7 | Far-view distance experiment | 4 | 3 | 4 | 3.0 | Still blocked by foreground/horizon readability | owned-browser LOD/fps/gap gates |

## Re-Rank After Atlas Budget Gate And Object Distinctiveness

Completed since the foreground pack:

- `bun run atlas:views -- --enforce-comparison-budgets` now fails luma/color/horizon/center/lower-ground regressions against a baseline.
- The budget gate caught the known foreground-pack close-view regression and passed a same-state comparison.
- Object-lab now reports form class, intentional negative space, negative-space ratio, coverage balance, and asymmetry.
- `scree_fan` and `buried_ribs` now report `intentional-negative-space` with `low-bounds-fill` suppressed instead of looking like generic broken sparse props.

The next work can move back to player-facing value, but every visual branch should use the enforced atlas gate before being accepted.

| Rank | Change | Impact | Confidence | Effort | ROI | Why Now | Verification |
| ---: | --- | ---: | ---: | ---: | ---: | --- | --- |
| 1 | Fast shader smoke harness before any WGSL/contact-depth retry | 4 | 5 | 2 | 10.0 | Atlas budgets exist, but shader iteration needs a cheaper first gate after the black-frame failure | one-view luma/color/frame gate, typecheck/build |
| 2 | Route-aware RPG hooks: named old roads, shrine blessings, Cartography/Naturalist/Lore progress | 5 | 4 | 3 | 6.7 | Route identity data and foreground markers are now available; gameplay can reward exploration without risking renderer perf | objective/skill tests, browser objective probe |
| 3 | Contact-depth/lighting pass under enforced atlas budgets | 5 | 3 | 4 | 3.8 | Still likely needed for the block-edge read, but only after shader smoke exists | shader smoke, enforced view-atlas, owned-browser budget |
| 4 | UI/journal polish for region and road discoveries | 4 | 4 | 3 | 5.3 | Good RPG feel return, now that block-building HUD is gone and route landmarks have names | UI tests, browser HUD smoke |
| 5 | Ziggurat budget/negative-space follow-up | 3 | 4 | 3 | 4.0 | Useful cost cleanup but not the current gate blocker | object-lab target `<18000`, route coverage |
| 6 | Far-view distance experiment | 4 | 3 | 4 | 3.0 | Wait until contact/depth work improves foreground and horizon readability | owned-browser LOD/fps/gap gates |

## Re-Rank After Shader Smoke Lab

Completed since the atlas budget gate:

- `bun run lab:shader` now captures one deterministic owned-browser atlas view and enforces luma/color/frame/draw/triangle budgets.
- A too-tight initial triangle budget failed against current baseline and was recalibrated honestly.
- The calibrated baseline smoke passed with failures none.

This removes the main blocker to careful shader/contact-depth work. Route-aware RPG hooks are still high ROI, but a small contact-depth pass can now be attempted with a cheap fail-fast gate.

| Rank | Change | Impact | Confidence | Effort | ROI | Why Now | Verification |
| ---: | --- | ---: | ---: | ---: | ---: | --- | --- |
| 1 | Small contact-depth/lighting shader pass under shader smoke and enforced atlas budgets | 5 | 4 | 4 | 5.0 | The tooling now catches black frames and view regressions; block-edge depth remains the visible blocker | `lab:shader`, enforced view-atlas, owned-browser lab |
| 2 | Route-aware RPG hooks: named old roads, shrine blessings, Cartography/Naturalist/Lore progress | 5 | 4 | 3 | 6.7 | Visual route identity and route influence are available; gameplay value can advance without renderer risk | objective/skill tests, browser objective probe |
| 3 | UI/journal polish for region and road discoveries | 4 | 4 | 3 | 5.3 | Good RPG feel return after route gameplay hooks | UI tests, browser HUD smoke |
| 4 | Ziggurat budget/negative-space follow-up | 3 | 4 | 3 | 4.0 | Useful cost cleanup but not the current blocker | object-lab target `<18000`, route coverage |
| 5 | Far-view distance experiment | 4 | 3 | 4 | 3.0 | Wait until contact/depth work improves foreground and horizon readability | owned-browser LOD/fps/gap gates |

## Re-Rank After Strict LOD Probe And Warped Routes

Completed since the shader-smoke checkpoint:

- Rejected a very small shader surface-mute attempt because shader smoke and atlas passed but the metric movement was effectively invisible.
- `probeLodCoverage` now counts visible render-ready LOD0 surfaces as owners even when the full vertical column is not ready, closing a blind spot around transient LOD overlap.
- Owned browser strict LOD probe passed with resident/band/water overlaps all `0`.
- Old-road and shrine discoveries now train RPG skills beyond generic Naturalist XP.
- Pilgrim routes now warp laterally instead of staying mathematically straight, and route/view/browser gates stayed clean.
- Object-lab now reports cross-view variation and vertical profile diagnostics for delegated prop work.

The latest browser result is healthy (`route p95/max 4.80/7.90 ms`, LOD overlap/gaps/handoff holes `0/0/0`), but the global visual grid metric remains `0.68`. The next changes need to attack camera-scale block-edge perception more directly.

| Rank | Change | Impact | Confidence | Effort | ROI | Why Now | Verification |
| ---: | --- | ---: | ---: | ---: | ---: | --- | --- |
| 1 | Screenshot-aware contact-depth/lighting pass under shader smoke and enforced atlas budgets | 5 | 4 | 4 | 5.0 | Route composition improved but block-edge read is still dominant; shader smoke now prevents black-frame regressions | `lab:shader`, enforced view-atlas, owned-browser route budget |
| 2 | Foreground terrain shape breakup, second large pass: route-adjacent ash drifts/salt cracks that alter silhouettes, not just materials | 5 | 4 | 4 | 5.0 | The stubborn `0.68` grid metric needs geometry/composition-scale foreground change | terrain lab compare, view-atlas lower-ground grid, route physics continuity |
| 3 | Far-view/fog cushion experiment with strict LOD ownership gate | 4 | 4 | 3 | 5.3 | LOD ownership probes are clean; a cautious distance/fog pass can improve scale if draw/triangles hold | owned-browser LOD overlap/gap/fps, view-atlas horizon grid |
| 4 | Ashland travel-kit pack delegated through improved object-lab: masks, travel packs, water collectors | 4 | 4 | 3 | 5.3 | Cross-view/vertical diagnostics now make isolated prop work less subjective | object-lab contact sheets, route atlas, browser draw budget |
| 5 | Route-aware UI/journal polish for old-road and shrine discoveries | 4 | 4 | 3 | 5.3 | Skill hooks exist; player-facing journal can now explain them without clutter | UI tests, browser HUD smoke |
| 6 | Ziggurat budget/negative-space follow-up | 3 | 4 | 3 | 4.0 | Still useful for cost and silhouette, but not the current whole-screen blocker | object-lab target `<18000`, route coverage |

## Re-Rank After Contact-Depth Shader Pass

Completed since the strict LOD/route checkpoint:

- A normal-based contact-depth shader pass was accepted only after shader smoke, enforced seven-view atlas, and owned-browser retry all passed.
- Browser visual grid finally moved from `0.68` to `0.67`, with route p95/max `4.70/7.00 ms`.
- Atlas deltas show the improvement is mainly horizon/center depth; lower-ground grid barely moved and slightly regressed in a few close views.

The next work should not keep darkening. The renderer now has a small depth cue; the remaining visible blocker is foreground/lower-ground shape language.

| Rank | Change | Impact | Confidence | Effort | ROI | Why Now | Verification |
| ---: | --- | ---: | ---: | ---: | ---: | --- | --- |
| 1 | Foreground terrain shape breakup, second large pass: route-adjacent ash drifts/salt cracks that alter silhouettes, not just materials | 5 | 4 | 4 | 5.0 | Lower-ground grid is still the weak metric after shader depth | terrain lab compare, enforced view-atlas lower-ground grid, route physics continuity |
| 2 | Far-view/fog cushion experiment with strict LOD ownership gate | 4 | 4 | 3 | 5.3 | LOD correctness and frame time are currently healthy, so a cautious scale pass is plausible | owned-browser LOD overlap/gap/fps, view-atlas horizon grid |
| 3 | Ashland travel-kit pack delegated through improved object-lab: masks, travel packs, water collectors | 4 | 4 | 3 | 5.3 | Cross-view/vertical diagnostics now make isolated prop work less subjective | object-lab contact sheets, route atlas, browser draw budget |
| 4 | Route-aware UI/journal polish for old-road and shrine discoveries | 4 | 4 | 3 | 5.3 | Skill hooks exist; player-facing journal can now explain them without clutter | UI tests, browser HUD smoke |
| 5 | Ziggurat budget/negative-space follow-up | 3 | 4 | 3 | 4.0 | Useful cost cleanup, but not what the current view metrics identify | object-lab target `<18000`, route coverage |

## Re-Rank After Ashlander Travel Pack

Completed since the contact-depth checkpoint:

- Added a warning-free `ashlander_travel_pack` prop with object-lab validation and restrained harsh-biome placement.
- Rejected a route-shoulder height tweak because terrain, route, and view metrics did not move enough to justify the complexity.
- View atlas stayed unchanged across the seven fixed views, which is useful evidence: isolated prop quality does not automatically become screenshot-level world identity.
- Browser performance and LOD correctness held cleanly.

The asset pipeline is working, but the next visible-progress bottleneck is measurement and camera-scale composition. Either make prop visibility measurable in route cameras, or move to a far-view/fog scale experiment with strict LOD gates.

| Rank | Change | Impact | Confidence | Effort | ROI | Why Now | Verification |
| ---: | --- | ---: | ---: | ---: | ---: | --- | --- |
| 1 | Prop visibility atlas: fixed close route cameras for travel-kit/debris/shrine props, with object-lab links | 4 | 5 | 2 | 10.0 | Current fixed view atlas misses small props, so prop work lacks camera-scale feedback | new/extended atlas report, object-lab artifacts, browser frame budget |
| 2 | Foreground terrain shape breakup, third attempt only with a stronger camera-visible design | 5 | 3 | 4 | 3.8 | Lower-ground grid remains weak, but small height tweaks failed to move metrics | terrain lab compare, enforced view-atlas lower-ground grid, route continuity |
| 3 | Far-view/fog cushion experiment with strict LOD ownership gate | 4 | 4 | 3 | 5.3 | LOD correctness and frame time are healthy; user wants farther views | owned-browser LOD overlap/gap/fps, view-atlas horizon grid |
| 4 | Route-aware UI/journal polish for old-road and shrine discoveries | 4 | 4 | 3 | 5.3 | Skill hooks exist and could make discoveries feel less like raw IDs | UI tests, browser HUD smoke |
| 5 | Ziggurat budget/negative-space follow-up | 3 | 4 | 3 | 4.0 | Useful cost cleanup, but not the main visual blocker | object-lab target `<18000`, route coverage |

## Re-Rank After Prop Visibility Atlas

Completed since the travel-pack checkpoint:

- `bun run atlas:views -- --preset props` now captures five close prop views through the same browser/PNG/comparison machinery as the world atlas.
- The prop preset exposed a useful issue: low route props can be technically valid in object-lab while still visually dominated by taller nearby route markers in the actual camera.

Next work should use the prop preset for small assets, but the highest user-facing ROI is now a scale/readability pass: either far-view/fog with strict LOD gates or a bolder terrain foreground design that the world atlas can actually see.

| Rank | Change | Impact | Confidence | Effort | ROI | Why Now | Verification |
| ---: | --- | ---: | ---: | ---: | ---: | --- | --- |
| 1 | Far-view/fog cushion experiment with strict LOD ownership gate | 4 | 4 | 3 | 5.3 | LOD correctness and frame time are healthy, and the user explicitly noticed short view distance | owned-browser LOD overlap/gap/fps, world view-atlas horizon grid |
| 2 | Object prominence metric for prop atlas | 4 | 4 | 3 | 5.3 | Prop screenshots exist; now the harness should quantify whether the target object reads clearly | prop atlas region metrics, object-lab links |
| 3 | Foreground terrain shape breakup with a camera-visible design | 5 | 3 | 4 | 3.8 | Lower-ground grid remains weak, but small route-height tweaks failed | terrain lab compare, enforced world atlas lower-ground grid, route continuity |
| 4 | Route-aware UI/journal polish for old-road and shrine discoveries | 4 | 4 | 3 | 5.3 | Skill hooks exist and could make discoveries feel less like raw IDs | UI tests, browser HUD smoke |
| 5 | Ziggurat budget/negative-space follow-up | 3 | 4 | 3 | 4.0 | Useful cost cleanup, but not the current visual blocker | object-lab target `<18000`, route coverage |

## Re-Rank After Far-View Fog Cushion

Completed since the prop visibility checkpoint:

- Default surface fog cap increased from `416 m` to `480 m`, still within existing LOD4 coverage.
- Browser route ambience now reports `395.89 m` ashfall fog, up from the prior `341.23 m` route baseline.
- Enforced world view atlas and owned-browser retry passed with no settled LOD overlap, water overlap, gaps, or handoff holes.
- The retry stayed performant, but moving diagnostics still recorded transient far-LOD coverage gaps while streaming.
- A delegated ROI audit independently flagged summary-derived LOD planning and far-view correctness as the next best work before content polish.

| Rank | Change | Impact | Confidence | Effort | ROI | Why Now | Verification |
| ---: | --- | ---: | ---: | ---: | ---: | --- | --- |
| 1 | Remove or hard-gate generator-backed LOD Y-range fallback; far LOD planning should come from summaries/regions | 5 | 4 | 3 | 6.7 | Farther fog passed, but transient moving far-gap diagnostics show this is the next correctness/perf risk | focused resident-world tests, browser route far-gap diagnostics, owned-browser LOD gates |
| 2 | Water/shoreline far-LOD correctness pass for marsh/saltflat horizons | 4 | 4 | 3 | 5.3 | Longer fog exposes more water horizon and z-fighting risk, even though current settled water overlap is `0` | extended `probeLodCoverage`, marsh/salt view atlas, owned-browser water overlap |
| 3 | Object prominence metric for prop atlas | 4 | 4 | 3 | 5.3 | Prop screenshots exist, but target readability is still not quantified | prop atlas region metrics, object-lab links |
| 4 | Foreground terrain shape breakup with a camera-visible design | 5 | 3 | 4 | 3.8 | The global grid metric remains `0.67`; this still needs real geometry/composition, not fog | terrain lab compare, enforced world atlas lower-ground grid, route continuity |
| 5 | Composition director for actual route camera sightlines | 5 | 3 | 4 | 3.8 | Farther views matter more when the camera has strong foreground/midground silhouettes | route-atlas vista scan, view-atlas deltas, draw/triangle budget |
| 6 | Route-aware UI/journal polish for old-road and shrine discoveries | 4 | 4 | 3 | 5.3 | Valuable RPG feel, but rendering correctness and scale are currently more fragile | UI tests, browser HUD smoke |

## Re-Rank After LOD Y-Range Planning Cleanup

Completed since the far-view checkpoint:

- LOD Y-range planning now prefers cached generated column summaries and no longer calls the full `sampleColumn()` fallback in the covered regression path.
- A broad no-generator envelope attempt was rejected because it exploded pending LOD keys and failed the far-ring starvation test.
- Owned browser route max improved from `9.80 ms` to `6.10 ms`, while settled overlap/gap/handoff gates stayed clean.
- Transient far-LOD gap frames did not improve, and increasing moving LOD generation budget failed to reduce them.

| Rank | Change | Impact | Confidence | Effort | ROI | Why Now | Verification |
| ---: | --- | ---: | ---: | ---: | ---: | --- | --- |
| 1 | Moving far-LOD handoff diagnostic: separate true visible holes from conservative far-gap samples and dirty/old coverage windows | 5 | 4 | 3 | 6.7 | The gap count is stubborn and may be measuring a benign handoff; correctness work needs sharper classification before more fixes | route benchmark samples, settled-reference capture, owned-browser gap counters |
| 2 | Water/shoreline far-LOD correctness pass for marsh/saltflat horizons | 4 | 4 | 3 | 5.3 | Longer fog exposes more water horizon; water overlap is clean but needs route-specific probes | extended `probeLodCoverage`, marsh/salt view atlas, owned-browser water overlap |
| 3 | Object prominence metric for prop atlas | 4 | 4 | 3 | 5.3 | Prop screenshots exist, but target readability is still not quantified | prop atlas region metrics, object-lab links |
| 4 | Foreground terrain shape breakup with a camera-visible design | 5 | 3 | 4 | 3.8 | The global grid metric remains `0.67`; this still needs real geometry/composition, not fog | terrain lab compare, enforced world atlas lower-ground grid, route continuity |
| 5 | Composition director for actual route camera sightlines | 5 | 3 | 4 | 3.8 | Farther views matter more when the camera has strong foreground/midground silhouettes | route-atlas vista scan, view-atlas deltas, draw/triangle budget |
| 6 | Route-aware UI/journal polish for old-road and shrine discoveries | 4 | 4 | 3 | 5.3 | Useful RPG feel, but rendering diagnostics are still the sharper blocker | UI tests, browser HUD smoke |

## Re-Rank After Far-LOD Gap Classification

Completed since the LOD planning checkpoint:

- Raw far-gap frames are now classified as transition or blocking.
- Owned browser shows the current route gaps are `50` transition frames and `0` blocking frames, with `0` visible holes and `0` overlaps.
- This clears the path back to player-visible world work without ignoring LOD correctness.

| Rank | Change | Impact | Confidence | Effort | ROI | Why Now | Verification |
| ---: | --- | ---: | ---: | ---: | ---: | --- | --- |
| 1 | Object prominence metric for prop atlas | 4 | 4 | 3 | 5.3 | Prop screenshots exist, but target readability is still not quantified; this helps delegated asset work become self-monitoring | prop atlas region metrics, object-lab links, tests |
| 2 | Foreground terrain shape breakup with a camera-visible design | 5 | 3 | 4 | 3.8 | The global grid metric remains `0.67`; now LOD blocking risk is not the immediate blocker | terrain lab compare, enforced world atlas lower-ground grid, route continuity |
| 3 | Composition director for actual route camera sightlines | 5 | 3 | 4 | 3.8 | Farther fog helps only if route cameras see strong foreground/midground silhouettes | route-atlas vista scan, view-atlas deltas, draw/triangle budget |
| 4 | Water/shoreline far-LOD correctness pass for marsh/saltflat horizons | 4 | 4 | 3 | 5.3 | Longer fog exposes more water horizon; useful, but current water overlap is already `0` | extended `probeLodCoverage`, marsh/salt view atlas, owned-browser water overlap |
| 5 | Route-aware UI/journal polish for old-road and shrine discoveries | 4 | 4 | 3 | 5.3 | Useful RPG feel after the visual/verification backlog moves | UI tests, browser HUD smoke |
