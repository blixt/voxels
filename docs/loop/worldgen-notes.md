# Worldgen Notes

Date: 2026-03-11

This note captures the first isolated procedural-world slice before it is wired into the live game.

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

The first generator deliberately keeps the biome logic simple and testable:

- low-frequency selector field for coarse biome regions
- climate fields (`temperature`, `moisture`, `weirdness`) to rebalance or override that selector
- biome families:
  - `verdant`
  - `dunes`
  - `badlands`
  - `tundra`
  - `ember`
- each biome defines:
  - surface color
  - subsurface color
  - stone/deep-stone colors
  - accent color
  - water color
  - snow color
  - terrain-height parameters

## Terrain strategy

- world height is sampled per column from several low-frequency fields:
  - macro relief
  - ridge emphasis
  - medium detail
  - basin offset
- voxel material is then resolved from:
  - surface level
  - water level
  - shallow subsurface band
  - deeper strata bands
  - rare accent pockets

This is intentionally not trying to solve caves, structures, vegetation, or streaming yet.

## Verification added with this slice

- palette mapping tests for `#RGB <-> material index`
- chunk determinism test
- chunk data vs direct sampler consistency test
- biome-distribution test over a large coordinate grid
- explicit `Y`-range guard test

## Next likely connection point

Wire the generator into a resident-world wrapper rather than directly into the renderer or `getVoxel()`.

That keeps generation explicit, testable, and measurable, and avoids accidental chunk creation from meshing or ray traversal.
