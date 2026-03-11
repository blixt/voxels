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
- `globalHeight`
- `mountainness`
- `oceanness`

These are all deterministic noise fields at different scales. They are intended to act more like "laws of the world" than direct biome IDs.
The important follow-up lesson from implementation is that `globalHeight` must stay genuinely slow; fast mountain/mesa signal belongs in local relief, not in the global envelope itself.

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

- a slower global terrain envelope shared by all biomes
- blended terrain parameters from the top base-biome influences
- dithered material mixing when two biome families have similar influence
- drainage/basin logic that produces local wetlands without forcing a hard biome wall
- suppressing biome-local relief and terrace strength near borders through a biome-core weight

### Stronger / intentional transitions

These should still be explained by the world rules:

- `badlands`
- `ember`

The goal is not random cliffs at a biome boundary. The goal is to let mesa / volcanic systems create stronger shape and palette changes in places where the fields support them.

## Landmark families

The first distinct landmark set should stay simple and cheap, but each biome should already have a small roster instead of one fixed family:

- `verdant`: oak, birch, shrub, boulder
- `steppe`: standing stone, shrub, boulder, birch
- `dunes`: palm, cactus, boulder
- `badlands`: hoodoo, standing stone, boulder, cactus
- `highland`: fir, boulder, birch, standing stone
- `tundra`: ice spire, fir, boulder, shrub
- `marsh`: cypress, reed cluster, shrub
- `ember`: basalt spire, crystal cluster, boulder
- `bloom`: glowcap, birch, crystal cluster, shrub

These should be deterministic and cell-local so chunk generation stays cheap and easy to cache.

## Surface detail

The current surface pass also needs to use the `10 cm` voxel scale more aggressively than a single smooth top material.
The present approach is still cheap and deterministic:

- drive surface/subsurface variants from:
  - `surfacePatch`
  - `surfaceGrain`
  - `scatter`
- expose:
  - flatter/wetter transition materials
  - stronger accent colors
  - exposed rock materials
- keep this logic column-local so later persistence/caching can reuse it directly

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

## Landmark scale follow-up

The first biome-rehaul landmark pass still made too many props read like shrubs or bonsai relative to a `1.8 m` player. The current rule is:

- scale is now part of the placement profile, not hardcoded only inside the feature switch
- each biome roster can change:
  - density
  - cell spacing
  - silhouette scale
  - variant/material look

This lets the engine keep one compact feature slot while still making:

- verdant feel tree-dominant with `oak` and `canopy_tree`
- steppe feel sparse and woody with `acacia`, `dead_snag`, and stones
- badlands feel more vertical with larger `hoodoo` and `standing_stone`
- highland and tundra feel taller through `tall_fir`, `ice_spire`, and larger rock silhouettes
- marsh and bloom feel more special through `mangrove`, `cypress`, `glowcap`, and `mega_glowcap`

The broad fixed-seed scan now finds landmark silhouettes up to about `9.7 m`, which is the first point where the world starts reading correctly against the player scale.
