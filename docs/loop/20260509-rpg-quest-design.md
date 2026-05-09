# RPG Quest Planning Foundation

## Scope

`src/engine/rpg-quests.ts` is a pure planning layer for exploration RPG quest and rumor seeds. It does not know about HUD state, controller input, save-game mutation, NPC spawning, inventory, or terrain editing. The output is deterministic data that later systems can render, activate, persist, or bind to concrete discoveries.

The planner accepts atlas/discovery context:

- region id
- optional route id
- optional landmark id

It emits four hook families for the same context:

- pilgrimage
- cave rumor
- faction errand
- environmental mystery

## Design Rules

Quest hooks should make exploration legible rather than turn the voxel world into a material checklist.

- Objectives use verbs like visit, inspect, listen, interpret, and report.
- Objectives never ask the player to mine, craft, gather, or collect building materials.
- Dependencies are explicit ids, so later runtime systems can decide whether a hook is available without parsing prose.
- Faction errands depend on local pilgrimage bearings; mysteries can depend on cave rumors when they need cross-checking.
- Region text owns the mood. Red Mountain should feel like ash pressure and shrine dread; Bitter Coast should feel like blackwater, roots, and reeds.

## Integration Notes

This layer can be wired into discovery or journal systems by treating hook ids as stable seed ids:

```ts
const plan = planRpgQuestHooks({
  regionId: sample.primaryRegionId,
  routeId: sample.routeId,
  landmarkId: discoveredLandmarkId,
});
```

Callers should store only the selected hook id and player progress. The hook itself can be regenerated from the same region, route, and landmark context. If content changes require migration later, use `RPG_QUESTS_VERSION` as the boundary.

## Current Content Coverage

The first pass covers every atlas region:

- Inner Sea: silt-mist shelf roads and shrine vows
- Red Mountain: caldera pilgrimage and lava tube rumors
- Ashen Badlands: ash trails, caravan stewards, and kwama ravines
- Bitter Coast: wetland crossings and root grotto rumors
- Grazelands: camp trails, open distance, and tense hospitality
- Salt Marsh Basin: salt causeways and mirror-crust mysteries
- Glass Shard Coast: hazard cairns and crystal cavern rumors
- West Gash: ravine passes, redleaf marks, and granitic caves

The foundation is intentionally content-forward but system-light. Runtime quest state, NPC scheduling, map markers, HUD presentation, and journal UI should build on this data without adding UI assumptions back into the planner.
