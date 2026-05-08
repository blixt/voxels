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
