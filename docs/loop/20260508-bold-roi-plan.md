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
