# World Model Notes

Date: 2026-03-11

This note captures the first architectural scouting pass for moving from a fixed demo scene to an infinite procedural world.

## Main finding

The smallest safe seam is not the renderer. It is the world-access boundary.

The renderer mostly cares about:

- resident chunks
- chunk meshes
- palette colors
- a view-projection matrix

The real fixed-world coupling lives in the current `VoxelWorld` storage model and in finite-world ray entry logic.

## Where the bounded assumptions currently live

- `src/engine/world.ts`
  - stores `width/height/depth`
  - derives dense `chunkCountX/Y/Z`
  - encodes chunk keys from those fixed counts
  - rejects any voxel or chunk outside the finite grid
- `src/engine/camera.ts`
  - assumes a global world AABB for ray entry and ray casting
  - computes orbit defaults from finite world dimensions
- `src/client/game-controller.ts`
  - still boots from a finite scene and chooses spawn from the center of that scene
- `src/engine/scene-format.ts`
  - current `VXSC` format stores finite dimensions and `u16` chunk coords
- `src/engine/reference-render.ts`
  - brute-force iterates the whole finite volume for validation

## Chosen first refactor

Introduce and start using a resident-chunk world interface before adding streaming.

That interface should cover:

- `getVoxel(x, y, z)`
- `getPaletteColor(materialIndex)`
- `getResidentChunk(cx, cy, cz)`
- `hasResidentChunk(cx, cy, cz)`
- `iterateResidentChunks()`
- `getChunkSolidBounds(cx, cy, cz)`
- `chunkSize`
- vertical limits only: `minY` and `maxYExclusive`

The current finite `VoxelWorld` implements this first.

## Why this is the right first cut

- It lets the mesher and renderer stop depending on dense finite-grid internals.
- It keeps `/bench` stable because the current finite `VoxelWorld` remains valid.
- It makes a later streaming implementation possible without forcing a renderer rewrite first.
- It narrows the next hard problems to:
  - chunk residency
  - generation
  - ray traversal in infinite `X/Z`
  - spawn/search logic in world space

## Important caution

Do not hide generation behind `getVoxel()`.

Meshing and raycasts can issue a large number of voxel reads. If `getVoxel()` also generates chunks implicitly, it becomes very hard to reason about performance and residency. Sampling resident data and ensuring chunk availability should stay separate operations.
