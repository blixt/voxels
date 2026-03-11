# Worldgen Notes

Date: 2026-03-11

This note started as the first isolated procedural-world slice before it was wired into the live game.
The current biome rework is tracked in `biome-rehaul-notes.md`; this file now serves as the concise worldgen baseline plus the delta from that first pass.

## Goals for the first generator

- deterministic per chunk coordinate and seed
- compatible with an effectively infinite `X/Z` world
- explicit `Y` range up to `16384`
- chunk-local generation with no hidden global state
- color-driven biome variation that can later support exploration and inventory
- independent verification before it touches streaming or the live game path

## Material model

The generator uses a fixed `#RGB` material palette:

- `4096` addressable colors
- palette index `1..4096`
- material `n` maps directly to `#RGB = (n - 1)` in hex
- this lines up with the eventual inventory model and avoids a later remap from “generator materials” to “inventory materials”

## Current biome strategy

The current generator is no longer a hard selector over a tiny biome roster.
It now uses field-driven surface/underground rules:

- large-scale world-rule fields:
  - `temperature`
  - `moisture`
  - `uplift`
  - `drainage`
  - `volcanism`
  - `magic`
  - `globalHeight`
  - `mountainness`
  - `oceanness`
- base biome families:
  - `verdant`
  - `steppe`
  - `dunes`
  - `badlands`
  - `highland`
  - `tundra`
- special / nested biome families:
  - `marsh`
  - `ember`
  - `bloom`
- underground families:
  - `rooted`
  - `sedimentary`
  - `sandy`
  - `granitic`
  - `froststone`
  - `basaltic`
- deterministic landmark families:
  - oak
  - birch
  - boulder
  - standing stone
  - shrub
  - palm
  - cactus
  - hoodoo
  - fir
  - ice spire
  - cypress
  - reed cluster
  - basalt spire
  - crystal cluster
  - glowcap

The detailed rules and transition policy live in `biome-rehaul-notes.md`.

## Terrain strategy

- world height is now built in two layers:
  - a slower global terrain envelope
  - biome-local relief that is suppressed near borders
- height transitions are softened by blending the strongest base-biome influences instead of switching terrain parameters at a selector border
- special biomes can still create stronger local identity, but that sharpness is tied to the field system rather than random biome boundaries
- voxel material is then resolved from:
  - surface level
  - surface patch / grain / scatter variation
  - water level
  - shallow subsurface band
  - deeper strata bands
  - underground-family accents
  - landmark/object voxels above the terrain surface when a column falls inside a deterministic landmark footprint

This is still intentionally not trying to solve caves or general structures yet, but it now does solve the first landmark/object pass.

## Verification added with this slice

- palette mapping tests for `#RGB <-> material index`
- chunk determinism test
- chunk data vs direct sampler consistency test
- biome-distribution test over a large coordinate grid
- host-rule test for special biomes
- forbidden-adjacency probe
- soft-edge height-budget probe
- landmark-family coverage probe
- surface-material diversity probe
- underground-family material-variation probe
- explicit `Y`-range guard test
- landmark-aware column-top envelope for resident-world vertical streaming

## Current next likely connection points

- grow landmark/object generation beyond simple single-cell silhouettes
- add richer biome/landmark-aware spawn heuristics instead of string sets
- keep worldgen measurements alongside the richer biome system so the new variety does not silently undo the performance work

Update:

- landmark generation now uses per-biome placement profiles instead of only per-biome landmark id sets
- landmark scale is now treated as first-class worldgen data because object readability at `10 cm` voxels matters just as much as palette choice
- landmark regions now also use slow overlay fields, not just base-biome identity:
  - `grove`
  - `oldGrowth`
  - `orchard`
  - `desolation`
- current landmark expansion is still intentionally compact:
  - new families like `willow`, `giant_flower`, and `thorn_tree` reuse the existing feature slot
  - but the reused feature paths now carry silhouette-specific variants so those families are not just recolors
- the latest variety pass pushed this same idea one step further with rare regional-extreme overlays:
  - they are still derived from the same world-rule fields
  - they do not become new top-level biome ids
  - they can influence local relief, palette, water behavior, and landmark rosters together
- current regional-extreme overlays are:
  - `verdant_karst`
  - `steppe_monolith`
  - `dunes_glass`
  - `badlands_crater`
  - `highland_redleaf`
  - `tundra_blue_ice`
  - `marsh_blackwater`
  - `ember_caldera`
  - `bloom_prism`
- the important implementation lesson was that "rare" selectors must still be measured directly:
  - the first multiplicative selector made most overlays effectively unreachable
  - the kept pass uses averaged signals and fixed-seed rarity scans to guard against dead-code variety
- the next worldgen extension did add more top-level biome families, but still kept the same field-driven structure:
  - new base biomes:
    - `savanna`
    - `moor`
  - new special biomes:
    - `firefly`
    - `saltflat`
    - `fern`
    - `fungal`
    - `shardlands`
- the useful distinction from the earlier overlay-only pass is:
  - new top-level biome ids are now used when the world-rule combinations are materially different enough to deserve a separate family
  - but the implementation still derives them from the same continuous fields instead of adding a second independent selector system
- newly added landmark families for those biome families are:
  - `giant_fern`
  - `lantern_tree`
  - `salt_spire`
- another now-verified lesson:
  - presence alone is too weak a guardrail for biome work
  - each new biome family now needs a deterministic landmark-identity check, otherwise a biome can exist numerically while still looking like a recolored old one
