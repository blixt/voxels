# Biome Rehaul Notes

Date: 2026-03-11

This note captures the intended replacement for the first procedural biome pass.

## Goals

- replace the current hard biome selector with large-scale "world-rule" fields
- make biome transitions feel intentional instead of accidental
- keep terrain-height transitions mostly soft even when surface materials change
- allow a small number of intentionally stronger biome transitions through special-case overlays rather than by random selector boundaries
- make below-ground material identity explicit
- add simple landmark/object families so biomes are visually distinct before deeper gameplay systems land
- preserve chunk-local deterministic generation and avoid obvious hot-path regressions

## Large-scale fields

The new biome sampler is built from a small set of slowly varying fields:

- `temperature`
- `moisture`
- `uplift`
- `drainage`
- `volcanism`
- `magic`

These are all deterministic noise fields at different scales. They are intended to act more like "laws of the world" than direct biome IDs.

## Surface biome roster

### Base biomes

- `verdant`
- `steppe`
- `dunes`
- `badlands`
- `highland`
- `tundra`

Base biomes are scored from the world-rule fields. Terrain parameters are blended from the strongest nearby base-biome candidates so height transitions remain continuous.

### Special / nested biomes

- `marsh`
  - only appears inside or beside wet `verdant` / `steppe`
  - low slope, high drainage, and basin/channel support required
  - transition should be soft
- `ember`
  - only appears inside or beside dry `badlands` / `highland`
  - high volcanism and low moisture required
  - may feel more abrupt, but that abruptness should come from the volcanic system itself rather than random selector borders
- `bloom`
  - only appears inside moist `verdant` / `highland`
  - high `magic` required
  - should feel rare and discoverable

## Below-ground families

Surface biomes are not enough. The new generator also assigns an underground family:

- `rooted`
- `sedimentary`
- `sandy`
- `granitic`
- `froststone`
- `basaltic`

These families drive subsurface, stone, deep-stone, and accent materials so the underground palette is not just a recolored copy of the surface.

## Transition policy

### Soft transitions

These are the default:

- `verdant <-> steppe`
- `steppe <-> dunes`
- `verdant <-> highland`
- `highland <-> tundra`
- host biome to `marsh`
- host biome to `bloom`

Soft transitions come from:

- blended terrain parameters from the top base-biome influences
- dithered material mixing when two biome families have similar influence
- drainage/basin logic that produces local wetlands without forcing a hard biome wall

### Stronger / intentional transitions

These should still be explained by the world rules:

- `badlands`
- `ember`

The goal is not random cliffs at a biome boundary. The goal is to let mesa / volcanic systems create stronger shape and palette changes in places where the fields support them.

## Landmark families

The first distinct landmark set should stay simple and cheap:

- `verdant`: oak-like tree
- `steppe`: standing stone / scrub monolith
- `dunes`: palm / oasis marker
- `badlands`: hoodoo spire
- `highland`: fir
- `tundra`: ice spire
- `marsh`: cypress
- `ember`: basalt vent / spire
- `bloom`: giant glowcap

These should be deterministic and cell-local so chunk generation stays cheap and easy to cache.

## Performance guardrails

- keep generation chunk-local and deterministic
- avoid per-voxel biome recomputation
- keep per-column state compact enough to store in typed scratch arrays
- keep landmark lookup local and bounded
- prefer a single "column state" pass that feeds both `sampleColumn()` and `generateChunk()`
- accept slightly higher cold generation cost if the result is richer and remains cache-friendly for later persistence

## Verification targets

- deterministic biome distribution for a fixed seed
- soft biome boundaries stay within a bounded height-jump budget
- special biomes obey host / adjacency rules
- underground families vary across the world and are reflected in sampled materials
- landmark families appear at deterministic frequencies
- chunk generation remains deterministic and consistent with direct sampling
