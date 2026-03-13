# Autoresearch Ideas

## Chunk Generation
- Column-level Y-range bounds: skip entire columns in voxel loop if chunk Y is entirely above column surface+feature
- Batch noise: compute noise for multiple columns at once to improve cache coherence
- SoA (struct of arrays) for surface field samples in the column loop — avoid per-column object creation for intermediate results
- Consider computing low-frequency noise fields (continent, uplift) at chunk granularity and interpolating

## Meshing
- The main mesher.ts still uses JS `number[]` for quads — converting to typed Int32Array with inline append (like opaque-chunk-mesher) could help but previous attempt had overhead from growth logic
- Consider pre-allocating quad arrays based on estimated face count from solid bounds
- The `isOpaqueMaterial` call in the mesher does `material !== 0 && !world.isWaterMaterial(material)` per face — could precompute an opaque mask like opaque-chunk-mesher.ts does

## Far Field
- The `sampleFarFieldColumn` still creates a return object per call — could return into a reusable struct
- Consider numeric keys for the `generatedRenderColumnSummaries` map instead of string `${cx}:${cz}` to avoid string allocation
- Band sample iteration order matches Z-then-X which may not match column summary storage order — could reorder for better locality

## Other
- The `selectBaseBiomes` iterates all 8 base biomes per column — could precompute a spatial lookup or use cheaper biome selection
- `resolveLandmark` does expensive voronoi-style cell lookups per column
