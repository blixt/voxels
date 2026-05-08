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
