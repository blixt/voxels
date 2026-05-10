# RPG World Roadmap

## Direction

Build a large, coherent island RPG world rather than a builder sandbox. The player should read the land through silhouettes, routes, weather, cave mouths, creature signs, vegetation, loot traces, and journal progression.

## Work Streams

- World shape: finite island identity, readable coast, Red Mountain-style center, regional highlands, wetlands, ashlands, glass coast, and old-road corridors.
- Traversal: stable first-person movement, roads as navigation language, cave entrances that resolve to coherent underground routes, and goals that can be completed by walking and inspecting.
- Ambiance: day/night cycle, weather, sky color, fog, water atmosphere, soundscape prompts, and region-specific visibility.
- Ecology: passive mobs, regional encounter signs, vegetation clusters, forage sites, and authored-looking prop families generated from deterministic rules.
- RPG loop: inspect/read/use interactions, discovery journal, loot journal, bestiary signs, skills, route goals, quest hooks, and persistence.
- Rendering scale: chunk streaming, resident LOD, gap-free handoff, and disposable derived render data fed by canonical chunks.

## Immediate Priorities

1. Keep the root game playable and visually readable before adding deeper systems.
2. Add player-facing content in vertical slices: one route, one region, one cave, one creature/forage loop, one skill reward loop.
3. Prefer deterministic procedural helpers over hand-placing every voxel, but keep authored anchors for route identity and regional composition.
4. Remove or rewrite old harness assumptions before relying on them; the repo is intentionally starting fresh on verification.
