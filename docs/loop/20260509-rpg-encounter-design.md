# 2026-05-09 RPG Encounter Foundation

## Purpose

Add a pure-data encounter and faction layer that can sit above the `WorldAtlas` without spawning actors, touching rendering, or changing terrain. The immediate goal is to give future NPC, creature, journal, and quest work a deterministic answer to this question:

> At this world-meter coordinate, what encounter pressure, mood, and faction hints are likely?

This is a foundation for a Morrowind-like exploration loop: roads feel safer than wilderness, cave mouths feel specific, and regions imply different social/ecological presence before any full simulation exists.

## Runtime Boundary

Implemented in `src/engine/rpg-encounters.ts`.

The module:

- samples `sampleWorldAtlasMeters`;
- reads atlas region IDs, route IDs, route influence, cave system IDs, and cave-anchor influence;
- blends small authored tables into a deterministic `RpgEncounterSample`;
- stays pure and allocation-light enough for occasional gameplay queries, validators, or debug probes.

The module deliberately does not:

- spawn, tick, or path NPCs;
- touch renderer, terrain, HUD, controller, styles, package metadata, or generator code;
- introduce random mutable state;
- invent a geography separate from `WorldAtlas`.

## Data Surfaces

### Region Zones

Every `AtlasRegionId` has one `RpgEncounterZoneDefinition`:

- `moodId`: baseline encounter mood.
- `pressureBase`: low-cost scalar for likely encounter intensity.
- `wildernessRisk`: extra pressure away from roads.
- `factionHints`: weighted faction/social/ecology hints.
- `flavorTags`: compact descriptors for later placement, journals, and debug UI.

### Route Modifiers

Every `AtlasRouteId` has one `RpgRouteEncounterModifier`.

Routes contribute:

- `safety`: how much the route core suppresses pressure.
- `road-truce` mood while strongly on route.
- route-specific faction hints and flavor tags.

This makes roads a gameplay signal without requiring pathing or actor simulation yet.

### Cave Modifiers

Every `AtlasCaveSystemId` has one `RpgCaveEncounterModifier`.

Caves contribute:

- cave pressure bonus from cave core/influence;
- `cave-threshold` mood;
- cave-specific faction and flavor, such as `kwama-mine`, `crystal-cavern`, or `saline-sinkhole`.

## Sampling Contract

`sampleRpgEncounterMeters(xM, zM)` returns:

- atlas identity: `regionId`, `routeId`, `caveSystemId`, `caveAnchorId`;
- blended scalars: `pressure`, `routeSafety`, `wildernessRisk`, `cavePressure`;
- authored interpretation: `moodId`, normalized `factionHints`, and `flavorTags`.

Pressure is clamped to `[0, 1]` and deterministic. A tiny hash jitter based on coarse world-meter cells prevents perfectly flat pressure fields while keeping repeated samples stable.

## Design Notes

- Road safety is strongest in the route core and weaker on shoulders.
- Wilderness risk fades down near route influence instead of disappearing abruptly.
- Cave pressure can override a safe route because several existing cave anchors sit directly on authored roads.
- Faction hints are not spawn tables. They are likely encounter context for later systems.
- Flavor tags are intentionally short strings so future systems can filter without coupling to prose.

## Validation

Focused coverage lives in `tests/rpg-encounters.test.ts`:

- encounter definitions cover all atlas regions, routes, and cave systems;
- repeated coordinate sampling is deterministic;
- regions retain distinct moods, pressure, and top faction identities;
- route core samples are safer than nearby wilderness;
- safer pilgrim roads score higher route safety than hazard routes;
- cave anchors add cave-specific mood, pressure, faction, and flavor.
