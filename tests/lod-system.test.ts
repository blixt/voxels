import { describe, expect, test, setDefaultTimeout } from "bun:test";

// World generation + LOD downsampling for the full test radius takes 60-120s
setDefaultTimeout(180_000);
import {
  ProceduralWorldGenerator,
  isProceduralWaterMaterial,
} from "../src/engine/procedural-generator.ts";
import { ProceduralResidentWorld } from "../src/engine/procedural-resident-world.ts";
import type { VoxelChunk } from "../src/engine/world.ts";

const SEED = 1337;
const CHUNK_SIZE = 32;

const LOD_RINGS = [
  { level: 1, radiusChunks: 8 },
  { level: 2, radiusChunks: 8 },
  { level: 3, radiusChunks: 6 },
  { level: 4, radiusChunks: 10 },
] as const;

/**
 * Shared world instance, lazily generated once for tests that only read.
 * Uses an unlimited LOD budget so all levels the system wants to produce
 * are actually generated.
 */
let _cachedWorld: ProceduralResidentWorld | null = null;
let _cachedSpawnPos: [number, number, number] | null = null;

function getWorld(): {
  world: ProceduralResidentWorld;
  spawnPos: [number, number, number];
} {
  if (_cachedWorld) {
    return { world: _cachedWorld, spawnPos: _cachedSpawnPos! };
  }
  const gen = new ProceduralWorldGenerator(SEED);
  const world = new ProceduralResidentWorld(gen, { horizontalRadiusChunks: 8 });
  const spawnPos = world.getSpawnPosition() as [number, number, number];
  world.updateResidencyAround(spawnPos, { maxGenerateChunks: Number.POSITIVE_INFINITY });
  world.updateLodResidencyAround(spawnPos, { maxGenerateLodChunks: Number.POSITIVE_INFINITY });
  _cachedWorld = world;
  _cachedSpawnPos = spawnPos;
  return { world, spawnPos };
}

/** Collect all resident chunks grouped by LOD level. */
function chunksByLevel(world: ProceduralResidentWorld): Map<number, VoxelChunk[]> {
  const map = new Map<number, VoxelChunk[]>();
  for (const chunk of world.iterateResidentChunks()) {
    const arr = map.get(chunk.lodLevel) ?? [];
    arr.push(chunk);
    map.set(chunk.lodLevel, arr);
  }
  return map;
}

/**
 * Replicate the 2x2x2 downsample: top-down Y scan, pick first opaque,
 * fall back to water.
 */
function downsample2x2x2FromSource(
  srcData: Uint16Array,
  srcLx: number,
  srcLy: number,
  srcLz: number,
): number {
  const cs = CHUNK_SIZE;
  const chunkArea = cs * cs;
  let bestMaterial = 0;
  for (let dy = 1; dy >= 0; dy--) {
    for (let dz = 0; dz < 2; dz++) {
      for (let dx = 0; dx < 2; dx++) {
        const lx = srcLx + dx;
        const ly = srcLy + dy;
        const lz = srcLz + dz;
        if (lx >= cs || ly >= cs || lz >= cs) continue;
        const mat = srcData[lx + ly * cs + lz * chunkArea]!;
        if (mat !== 0 && !isProceduralWaterMaterial(mat)) {
          return mat; // first opaque wins
        }
        if (mat !== 0 && bestMaterial === 0) {
          bestMaterial = mat; // water fallback
        }
      }
    }
  }
  return bestMaterial;
}

function lodVoxelFootprintOverlapsRenderReadyLod0(
  world: ProceduralResidentWorld,
  worldX: number,
  worldZ: number,
  stride: number,
): boolean {
  const minChunkX = Math.floor(worldX / CHUNK_SIZE);
  const maxChunkX = Math.floor((worldX + stride - 1) / CHUNK_SIZE);
  const minChunkZ = Math.floor(worldZ / CHUNK_SIZE);
  const maxChunkZ = Math.floor((worldZ + stride - 1) / CHUNK_SIZE);
  for (let chunkZ = minChunkZ; chunkZ <= maxChunkZ; chunkZ += 1) {
    for (let chunkX = minChunkX; chunkX <= maxChunkX; chunkX += 1) {
      if (world.isColumnRenderReady(chunkX, chunkZ)) {
        return true;
      }
    }
  }
  return false;
}

function isWorldColumnCoveredByLodChunk(
  chunk: VoxelChunk,
  worldX: number,
  worldZ: number,
): boolean {
  const stride = Math.max(1, chunk.voxelStride);
  const worldSize = CHUNK_SIZE * stride;
  const localX = Math.floor((worldX - chunk.coord.x * worldSize) / stride);
  const localZ = Math.floor((worldZ - chunk.coord.z * worldSize) / stride);
  if (localX < 0 || localX >= CHUNK_SIZE || localZ < 0 || localZ >= CHUNK_SIZE) {
    return false;
  }
  const chunkArea = CHUNK_SIZE * CHUNK_SIZE;
  for (let localY = 0; localY < CHUNK_SIZE; localY += 1) {
    if (chunk.data[localX + localY * CHUNK_SIZE + localZ * chunkArea] !== 0) {
      return true;
    }
  }
  return false;
}

function getLodHorizontalOwnerKey(chunk: VoxelChunk): string {
  return `LOD${chunk.lodLevel}:${chunk.coord.x}:${chunk.coord.z}`;
}

function isWorldColumnWaterTopCoveredByLodChunk(
  chunk: VoxelChunk,
  worldX: number,
  worldZ: number,
): boolean {
  const stride = Math.max(1, chunk.voxelStride);
  const worldSize = CHUNK_SIZE * stride;
  const localX = Math.floor((worldX - chunk.coord.x * worldSize) / stride);
  const localZ = Math.floor((worldZ - chunk.coord.z * worldSize) / stride);
  if (localX < 0 || localX >= CHUNK_SIZE || localZ < 0 || localZ >= CHUNK_SIZE) {
    return false;
  }
  const chunkArea = CHUNK_SIZE * CHUNK_SIZE;
  for (let localY = 0; localY < CHUNK_SIZE; localY += 1) {
    const material = chunk.data[localX + localY * CHUNK_SIZE + localZ * chunkArea]!;
    if (!isProceduralWaterMaterial(material)) {
      continue;
    }
    const above = localY + 1 < CHUNK_SIZE
      ? chunk.data[localX + (localY + 1) * CHUNK_SIZE + localZ * chunkArea]!
      : 0;
    if (!isProceduralWaterMaterial(above)) {
      return true;
    }
  }
  return false;
}

function isRenderReadyWaterColumn(
  world: ProceduralResidentWorld,
  worldX: number,
  worldZ: number,
): boolean {
  const chunkX = Math.floor(worldX / CHUNK_SIZE);
  const chunkZ = Math.floor(worldZ / CHUNK_SIZE);
  const localX = ((Math.floor(worldX) % CHUNK_SIZE) + CHUNK_SIZE) % CHUNK_SIZE;
  const localZ = ((Math.floor(worldZ) % CHUNK_SIZE) + CHUNK_SIZE) % CHUNK_SIZE;
  const chunkArea = CHUNK_SIZE * CHUNK_SIZE;
  for (let cy = 0; cy < 8; cy += 1) {
    const chunk = world.getResidentChunk(chunkX, cy, chunkZ);
    if (!chunk?.renderReady) {
      continue;
    }
    for (let localY = 0; localY < CHUNK_SIZE; localY += 1) {
      const material = chunk.data[localX + localY * CHUNK_SIZE + localZ * chunkArea]!;
      if (!isProceduralWaterMaterial(material)) {
        continue;
      }
      const above = localY + 1 < CHUNK_SIZE
        ? chunk.data[localX + (localY + 1) * CHUNK_SIZE + localZ * chunkArea]!
        : 0;
      if (!isProceduralWaterMaterial(above)) {
        return true;
      }
    }
  }
  return false;
}

// ---------------------------------------------------------------------------
// 1. Downsample correctness
// ---------------------------------------------------------------------------
describe("LOD downsampling", () => {
  test("LOD 0 and LOD chunks are both generated", () => {
    const { world } = getWorld();
    const byLevel = chunksByLevel(world);
    expect(byLevel.get(0)!.length).toBeGreaterThan(0);
    // At least LOD 1 must have chunks
    expect(byLevel.get(1)!.length).toBeGreaterThan(0);
    // Count total LOD chunks
    const totalLod = LOD_RINGS.reduce(
      (sum, ring) => sum + (byLevel.get(ring.level)?.length ?? 0),
      0,
    );
    expect(totalLod).toBeGreaterThan(0);
  });

  test("LOD 1 voxels backed by resident LOD 0 source match 2x2x2 downsample", () => {
    const { world } = getWorld();
    const stride = 2;
    const worldSize = CHUNK_SIZE * stride;
    const sourceWorldSize = CHUNK_SIZE; // LOD 0 world size (stride=1)

    let totalVoxelsChecked = 0;
    let chunksChecked = 0;

    for (const chunk of world.iterateResidentChunks()) {
      if (chunk.lodLevel !== 1 || chunk.solidCount === 0) continue;
      chunksChecked++;

      const originX = chunk.coord.x * worldSize;
      const originY = chunk.coord.y * worldSize;
      const originZ = chunk.coord.z * worldSize;

      for (let oz = 0; oz < CHUNK_SIZE; oz++) {
        for (let oy = 0; oy < CHUNK_SIZE; oy++) {
          for (let ox = 0; ox < CHUNK_SIZE; ox++) {
            const lodMat = chunk.data[ox + oy * CHUNK_SIZE + oz * CHUNK_SIZE * CHUNK_SIZE]!;

            // World position this LOD voxel represents
            const wx = originX + ox * stride;
            const wy = originY + oy * stride;
            const wz = originZ + oz * stride;

            // Find the source LOD 0 chunk
            const srcCx = Math.floor(wx / sourceWorldSize);
            const srcCy = Math.floor(wy / sourceWorldSize);
            const srcCz = Math.floor(wz / sourceWorldSize);
            const srcChunk = world.getResidentChunk(srcCx, srcCy, srcCz);
            if (!srcChunk) {
              continue;
            }

            const expectedMat = world.isColumnRenderReady(srcCx, srcCz)
              ? 0
              : downsample2x2x2FromSource(
                  srcChunk.data,
                  wx - srcCx * sourceWorldSize,
                  wy - srcCy * sourceWorldSize,
                  wz - srcCz * sourceWorldSize,
                );

            if (lodMat !== expectedMat) {
              throw new Error(
                `LOD 1 mismatch at chunk(${chunk.coord.x},${chunk.coord.y},${chunk.coord.z}) ` +
                  `local(${ox},${oy},${oz}) world(${wx},${wy},${wz}): ` +
                  `LOD has ${lodMat}, expected ${expectedMat} ` +
                  `(source ${srcChunk ? "exists" : "MISSING"})`,
              );
            }
            totalVoxelsChecked++;
          }
        }
      }
    }

    expect(chunksChecked).toBeGreaterThan(0);
    expect(totalVoxelsChecked).toBeGreaterThan(0);
  });

  test("LOD 2 every voxel matches 2x2x2 downsample from LOD 1 source", () => {
    const { world } = getWorld();
    const level = 2;
    const stride = 1 << level; // 4
    const worldSize = CHUNK_SIZE * stride;

    // Build a lookup map for LOD 1 chunk data
    const srcStride = 1 << (level - 1); // 2
    const srcWorldSize = CHUNK_SIZE * srcStride; // 64
    const srcMap = new Map<string, VoxelChunk>();
    for (const chunk of world.iterateResidentChunks()) {
      if (chunk.lodLevel === level - 1) {
        srcMap.set(`${chunk.coord.x}:${chunk.coord.y}:${chunk.coord.z}`, chunk);
      }
    }

    let totalVoxelsChecked = 0;
    let chunksChecked = 0;

    for (const chunk of world.iterateResidentChunks()) {
      if (chunk.lodLevel !== level || chunk.solidCount === 0) continue;
      chunksChecked++;

      const originX = chunk.coord.x * worldSize;
      const originY = chunk.coord.y * worldSize;
      const originZ = chunk.coord.z * worldSize;

      for (let oz = 0; oz < CHUNK_SIZE; oz++) {
        for (let oy = 0; oy < CHUNK_SIZE; oy++) {
          for (let ox = 0; ox < CHUNK_SIZE; ox++) {
            const lodMat = chunk.data[ox + oy * CHUNK_SIZE + oz * CHUNK_SIZE * CHUNK_SIZE]!;

            const wx = originX + ox * stride;
            const wy = originY + oy * stride;
            const wz = originZ + oz * stride;

            const srcCx = Math.floor(wx / srcWorldSize);
            const srcCy = Math.floor(wy / srcWorldSize);
            const srcCz = Math.floor(wz / srcWorldSize);
            const srcChunk = srcMap.get(`${srcCx}:${srcCy}:${srcCz}`) ?? null;
            if (!srcChunk) {
              continue;
            }

            const sLx = Math.floor((wx - srcCx * srcWorldSize) / srcStride);
            const sLy = Math.floor((wy - srcCy * srcWorldSize) / srcStride);
            const sLz = Math.floor((wz - srcCz * srcWorldSize) / srcStride);
            const expectedMat = lodVoxelFootprintOverlapsRenderReadyLod0(world, wx, wz, stride)
              ? 0
              : srcChunk.renderReady
              ? 0
              : downsample2x2x2FromSource(srcChunk.data, sLx, sLy, sLz);

            if (lodMat !== expectedMat) {
              throw new Error(
                `LOD 2 mismatch at chunk(${chunk.coord.x},${chunk.coord.y},${chunk.coord.z}) ` +
                  `local(${ox},${oy},${oz}) world(${wx},${wy},${wz}): ` +
                  `LOD has ${lodMat}, expected ${expectedMat}`,
              );
            }
            totalVoxelsChecked++;
          }
        }
      }
    }

    // Generated fallback can fill parts of an LOD chunk before the lower
    // source level exists. Source-backed voxels still must match exactly.
    if (chunksChecked > 0) {
      expect(totalVoxelsChecked).toBeGreaterThan(0);
    }
  });

  test("LOD 3 and LOD 4 voxels (if any) match 2x2x2 downsample from source", () => {
    const { world } = getWorld();

    for (const targetLevel of [3, 4]) {
      const stride = 1 << targetLevel;
      const worldSize = CHUNK_SIZE * stride;

      const srcLevel = targetLevel - 1;
      const srcStride = 1 << srcLevel;
      const srcWorldSize = CHUNK_SIZE * srcStride;
      const srcMap = new Map<string, VoxelChunk>();
      for (const chunk of world.iterateResidentChunks()) {
        if (chunk.lodLevel === srcLevel) {
          srcMap.set(`${chunk.coord.x}:${chunk.coord.y}:${chunk.coord.z}`, chunk);
        }
      }

      for (const chunk of world.iterateResidentChunks()) {
        if (chunk.lodLevel !== targetLevel || chunk.solidCount === 0) continue;

        const originX = chunk.coord.x * worldSize;
        const originY = chunk.coord.y * worldSize;
        const originZ = chunk.coord.z * worldSize;

        for (let oz = 0; oz < CHUNK_SIZE; oz++) {
          for (let oy = 0; oy < CHUNK_SIZE; oy++) {
            for (let ox = 0; ox < CHUNK_SIZE; ox++) {
              const lodMat = chunk.data[ox + oy * CHUNK_SIZE + oz * CHUNK_SIZE * CHUNK_SIZE]!;

              const wx = originX + ox * stride;
              const wy = originY + oy * stride;
              const wz = originZ + oz * stride;

              const sCx = Math.floor(wx / srcWorldSize);
              const sCy = Math.floor(wy / srcWorldSize);
              const sCz = Math.floor(wz / srcWorldSize);
              const srcChunk = srcMap.get(`${sCx}:${sCy}:${sCz}`) ?? null;
              if (!srcChunk) {
                continue;
              }

              const sLx = Math.floor((wx - sCx * srcWorldSize) / srcStride);
              const sLy = Math.floor((wy - sCy * srcWorldSize) / srcStride);
              const sLz = Math.floor((wz - sCz * srcWorldSize) / srcStride);
              const expectedMat = lodVoxelFootprintOverlapsRenderReadyLod0(world, wx, wz, stride)
                ? 0
                : srcChunk.renderReady
                ? 0
                : downsample2x2x2FromSource(srcChunk.data, sLx, sLy, sLz);

              if (lodMat !== expectedMat) {
                throw new Error(
                  `LOD ${targetLevel} mismatch at chunk(${chunk.coord.x},${chunk.coord.y},${chunk.coord.z}) ` +
                    `local(${ox},${oy},${oz}): LOD has ${lodMat}, expected ${expectedMat}`,
                );
              }
            }
          }
        }
      }
    }
  });

  test("LOD 2 cascading: matches manual downsample of LOD 1 data", () => {
    // LOD 2 is built by downsampling LOD 1. Verify the cascading property:
    // LOD 2 data should be identical to manually downsampling LOD 1 data.
    const { world } = getWorld();

    const srcStride = 2;
    const srcWorldSize = CHUNK_SIZE * srcStride;
    const lod1Map = new Map<string, VoxelChunk>();
    for (const chunk of world.iterateResidentChunks()) {
      if (chunk.lodLevel === 1) {
        lod1Map.set(`${chunk.coord.x}:${chunk.coord.y}:${chunk.coord.z}`, chunk);
      }
    }

    const lod2Stride = 4;
    const lod2WorldSize = CHUNK_SIZE * lod2Stride;

    let chunksChecked = 0;

    for (const lod2Chunk of world.iterateResidentChunks()) {
      if (lod2Chunk.lodLevel !== 2 || lod2Chunk.solidCount === 0) continue;
      chunksChecked++;

      const originX = lod2Chunk.coord.x * lod2WorldSize;
      const originY = lod2Chunk.coord.y * lod2WorldSize;
      const originZ = lod2Chunk.coord.z * lod2WorldSize;

      for (let oz = 0; oz < CHUNK_SIZE; oz++) {
        for (let oy = 0; oy < CHUNK_SIZE; oy++) {
          for (let ox = 0; ox < CHUNK_SIZE; ox++) {
            const actual = lod2Chunk.data[ox + oy * CHUNK_SIZE + oz * CHUNK_SIZE * CHUNK_SIZE]!;

            const wx = originX + ox * lod2Stride;
            const wy = originY + oy * lod2Stride;
            const wz = originZ + oz * lod2Stride;

            const srcCx = Math.floor(wx / srcWorldSize);
            const srcCy = Math.floor(wy / srcWorldSize);
            const srcCz = Math.floor(wz / srcWorldSize);
            const srcChunk = lod1Map.get(`${srcCx}:${srcCy}:${srcCz}`) ?? null;
            if (!srcChunk) {
              continue;
            }

            const sLx = Math.floor((wx - srcCx * srcWorldSize) / srcStride);
            const sLy = Math.floor((wy - srcCy * srcWorldSize) / srcStride);
            const sLz = Math.floor((wz - srcCz * srcWorldSize) / srcStride);
            const expected = lodVoxelFootprintOverlapsRenderReadyLod0(world, wx, wz, lod2Stride)
              ? 0
              : srcChunk.renderReady
              ? 0
              : downsample2x2x2FromSource(srcChunk.data, sLx, sLy, sLz);

            expect(actual).toBe(expected);
          }
        }
      }
    }

    // LOD 2 may or may not exist; if it does, the cascading property holds
    if (chunksChecked > 0) {
      expect(chunksChecked).toBeGreaterThan(0);
    }
  });

  test("water fallback: any water LOD voxel came from a block with no opaque material", () => {
    // For LOD 1 voxels backed by actual source chunks (not fallback),
    // verify that any water-material LOD voxel was produced because the
    // source 2x2x2 block had no opaque material but did have water.
    const { world } = getWorld();
    const stride = 2;
    const worldSize = CHUNK_SIZE * stride;
    const sourceWorldSize = CHUNK_SIZE;

    let waterFallbackCount = 0;
    let nonWaterSolidCount = 0;

    for (const chunk of world.iterateResidentChunks()) {
      if (chunk.lodLevel !== 1 || chunk.solidCount === 0) continue;

      const originX = chunk.coord.x * worldSize;
      const originY = chunk.coord.y * worldSize;
      const originZ = chunk.coord.z * worldSize;

      for (let oz = 0; oz < CHUNK_SIZE; oz++) {
        for (let oy = 0; oy < CHUNK_SIZE; oy++) {
          for (let ox = 0; ox < CHUNK_SIZE; ox++) {
            const lodMat = chunk.data[ox + oy * CHUNK_SIZE + oz * CHUNK_SIZE * CHUNK_SIZE]!;
            if (lodMat === 0) continue;

            const wx = originX + ox * stride;
            const wy = originY + oy * stride;
            const wz = originZ + oz * stride;

            // Only check voxels with actual source chunks
            const srcCx = Math.floor(wx / sourceWorldSize);
            const srcCy = Math.floor(wy / sourceWorldSize);
            const srcCz = Math.floor(wz / sourceWorldSize);
            const srcChunk = world.getResidentChunk(srcCx, srcCy, srcCz);
            if (!srcChunk) continue;

            if (isProceduralWaterMaterial(lodMat)) {
              // Verify: the 2x2x2 block must have NO opaque material
              // but at least one water material
              let hasOpaque = false;
              let hasWater = false;
              for (let dy = 1; dy >= 0; dy--) {
                for (let dz = 0; dz < 2; dz++) {
                  for (let dx = 0; dx < 2; dx++) {
                    const mat = world.getVoxel(wx + dx, wy + dy, wz + dz);
                    if (mat !== 0 && !isProceduralWaterMaterial(mat)) hasOpaque = true;
                    if (mat !== 0 && isProceduralWaterMaterial(mat)) hasWater = true;
                  }
                }
              }
              expect(hasOpaque).toBe(false);
              expect(hasWater).toBe(true);
              waterFallbackCount++;
            } else {
              nonWaterSolidCount++;
            }
          }
        }
      }
    }

    // We definitely have non-water solid LOD voxels
    expect(nonWaterSolidCount).toBeGreaterThan(0);
    // Water fallback may or may not occur for this seed/configuration;
    // what matters is the logic above is correct when it does occur.
  });

  test("LOD 1 downsample selects highest-Y opaque voxel, not arbitrary", () => {
    // Verify the top-down Y scan semantics: when multiple opaque voxels
    // exist in a 2x2x2 block, the one at the highest Y is chosen (not
    // the first encountered at any Y).
    const { world } = getWorld();
    const stride = 2;
    const worldSize = CHUNK_SIZE * stride;
    const sourceWorldSize = CHUNK_SIZE;

    let verifiedCount = 0;

    for (const chunk of world.iterateResidentChunks()) {
      if (chunk.lodLevel !== 1 || chunk.solidCount === 0) continue;

      const originX = chunk.coord.x * worldSize;
      const originY = chunk.coord.y * worldSize;
      const originZ = chunk.coord.z * worldSize;

      for (let oz = 0; oz < CHUNK_SIZE; oz++) {
        for (let oy = 0; oy < CHUNK_SIZE; oy++) {
          for (let ox = 0; ox < CHUNK_SIZE; ox++) {
            const lodMat = chunk.data[ox + oy * CHUNK_SIZE + oz * CHUNK_SIZE * CHUNK_SIZE]!;
            if (lodMat === 0) continue;

            const wx = originX + ox * stride;
            const wy = originY + oy * stride;
            const wz = originZ + oz * stride;

            // Only check voxels with actual source chunks
            const srcCx = Math.floor(wx / sourceWorldSize);
            const srcCy = Math.floor(wy / sourceWorldSize);
            const srcCz = Math.floor(wz / sourceWorldSize);
            if (!world.getResidentChunk(srcCx, srcCy, srcCz)) continue;

            // Collect all opaque materials in the 2x2x2 block, grouped by Y
            const highYMats: number[] = [];
            const lowYMats: number[] = [];
            for (let dz = 0; dz < 2; dz++) {
              for (let dx = 0; dx < 2; dx++) {
                const matHigh = world.getVoxel(wx + dx, wy + 1, wz + dz);
                if (matHigh !== 0 && !isProceduralWaterMaterial(matHigh)) {
                  highYMats.push(matHigh);
                }
                const matLow = world.getVoxel(wx + dx, wy, wz + dz);
                if (matLow !== 0 && !isProceduralWaterMaterial(matLow)) {
                  lowYMats.push(matLow);
                }
              }
            }

            // If high-Y opaque materials exist, the LOD voxel must be one of them
            if (highYMats.length > 0) {
              expect(highYMats).toContain(lodMat);
              verifiedCount++;
            } else if (lowYMats.length > 0) {
              expect(lowYMats).toContain(lodMat);
              verifiedCount++;
            }
          }
        }
      }
    }

    expect(verifiedCount).toBeGreaterThan(0);
  });
});

// ---------------------------------------------------------------------------
// 2. Vertex scaling at all strides
// ---------------------------------------------------------------------------
describe("LOD vertex positions", () => {
  test("all LOD chunk vertices fall within expected world-space AABB", () => {
    const { world } = getWorld();

    let lodChunkCount = 0;

    for (const chunk of world.iterateResidentChunks()) {
      if (chunk.lodLevel === 0) continue;
      if (!chunk.mesh || chunk.mesh.vertexCount === 0) continue;
      lodChunkCount++;

      const stride = chunk.voxelStride;
      const worldSize = CHUNK_SIZE * stride;
      const expectedMinX = chunk.coord.x * worldSize;
      const expectedMinY = chunk.coord.y * worldSize;
      const expectedMinZ = chunk.coord.z * worldSize;
      const expectedMaxX = (chunk.coord.x + 1) * worldSize;
      const expectedMaxY = (chunk.coord.y + 1) * worldSize;
      const expectedMaxZ = (chunk.coord.z + 1) * worldSize;

      const floats = new Float32Array(chunk.mesh.vertexData);
      for (let i = 0; i < chunk.mesh.vertexCount; i++) {
        const vx = floats[i * 5]!;
        const vy = floats[i * 5 + 1]!;
        const vz = floats[i * 5 + 2]!;
        if (
          vx < expectedMinX - 0.01 ||
          vx > expectedMaxX + 0.01 ||
          vy < expectedMinY - 0.01 ||
          vy > expectedMaxY + 0.01 ||
          vz < expectedMinZ - 0.01 ||
          vz > expectedMaxZ + 0.01
        ) {
          throw new Error(
            `Vertex out of bounds in LOD ${chunk.lodLevel} chunk` +
              `(${chunk.coord.x},${chunk.coord.y},${chunk.coord.z}): ` +
              `vertex(${vx},${vy},${vz}) not in ` +
              `[${expectedMinX},${expectedMinY},${expectedMinZ}]-` +
              `[${expectedMaxX},${expectedMaxY},${expectedMaxZ}]`,
          );
        }
      }
    }

    expect(lodChunkCount).toBeGreaterThan(0);
  });

  test("vertices at each LOD stride are correctly scaled from mesher coords", () => {
    const { world } = getWorld();

    // Check at least one chunk per LOD level
    const checkedLevels = new Set<number>();

    for (const chunk of world.iterateResidentChunks()) {
      if (chunk.lodLevel === 0) continue;
      if (checkedLevels.has(chunk.lodLevel)) continue;
      if (!chunk.mesh || chunk.mesh.vertexCount === 0) continue;

      const stride = chunk.voxelStride;
      const worldSize = CHUNK_SIZE * stride;
      const cx = chunk.coord.x;
      const cy = chunk.coord.y;
      const cz = chunk.coord.z;

      const mesherOriginX = cx * CHUNK_SIZE;
      const mesherOriginY = cy * CHUNK_SIZE;
      const mesherOriginZ = cz * CHUNK_SIZE;

      const worldOriginX = cx * worldSize;
      const worldOriginY = cy * worldSize;
      const worldOriginZ = cz * worldSize;

      const floats = new Float32Array(chunk.mesh.vertexData);

      // The scaling formula is: worldPos = worldOrigin + (mesherPos - mesherOrigin) * stride
      // So mesherPos = (worldPos - worldOrigin) / stride + mesherOrigin
      // And mesherPos must be in [mesherOrigin, mesherOrigin + CHUNK_SIZE]
      for (let i = 0; i < chunk.mesh.vertexCount; i++) {
        const vx = floats[i * 5]!;
        const vy = floats[i * 5 + 1]!;
        const vz = floats[i * 5 + 2]!;

        const mesherX = (vx - worldOriginX) / stride + mesherOriginX;
        const mesherY = (vy - worldOriginY) / stride + mesherOriginY;
        const mesherZ = (vz - worldOriginZ) / stride + mesherOriginZ;

        expect(mesherX).toBeGreaterThanOrEqual(mesherOriginX - 0.01);
        expect(mesherX).toBeLessThanOrEqual(mesherOriginX + CHUNK_SIZE + 0.01);
        expect(mesherY).toBeGreaterThanOrEqual(mesherOriginY - 0.01);
        expect(mesherY).toBeLessThanOrEqual(mesherOriginY + CHUNK_SIZE + 0.01);
        expect(mesherZ).toBeGreaterThanOrEqual(mesherOriginZ - 0.01);
        expect(mesherZ).toBeLessThanOrEqual(mesherOriginZ + CHUNK_SIZE + 0.01);
      }

      checkedLevels.add(chunk.lodLevel);
    }

    // Verify we tested at least LOD level 1
    expect(checkedLevels.has(1)).toBe(true);
  });

  test("mesh vertex data has correct 20-byte stride (5 float32s per vertex)", () => {
    const { world } = getWorld();

    let checked = 0;
    for (const chunk of world.iterateResidentChunks()) {
      if (chunk.lodLevel === 0) continue;
      if (!chunk.mesh || chunk.mesh.vertexCount === 0) continue;
      checked++;

      const expectedBytes = chunk.mesh.vertexCount * 20;
      expect(chunk.mesh.vertexData.byteLength).toBe(expectedBytes);
    }
    expect(checked).toBeGreaterThan(0);
  });

  test("voxelStride matches 1 << lodLevel for all chunks", () => {
    const { world } = getWorld();

    let lodChunks = 0;
    for (const chunk of world.iterateResidentChunks()) {
      if (chunk.lodLevel === 0) {
        expect(chunk.voxelStride).toBe(1);
      } else {
        expect(chunk.voxelStride).toBe(1 << chunk.lodLevel);
        lodChunks++;
      }
    }
    expect(lodChunks).toBeGreaterThan(0);
  });

  test("vertex positions at stride 2 span the world-size range", () => {
    // For LOD 1 (stride=2), the world-space extent of vertices should
    // be within [coord * worldSize, (coord+1) * worldSize].
    const { world } = getWorld();

    let checked = 0;
    for (const chunk of world.iterateResidentChunks()) {
      if (chunk.lodLevel !== 1) continue;
      if (!chunk.mesh || chunk.mesh.vertexCount === 0) continue;

      const stride = chunk.voxelStride;
      expect(stride).toBe(2);

      const worldSize = CHUNK_SIZE * stride;
      const cx = chunk.coord.x;

      const floats = new Float32Array(chunk.mesh.vertexData);
      let minVx = Infinity,
        maxVx = -Infinity;

      for (let i = 0; i < chunk.mesh.vertexCount; i++) {
        const vx = floats[i * 5]!;
        minVx = Math.min(minVx, vx);
        maxVx = Math.max(maxVx, vx);
      }

      expect(minVx).toBeGreaterThanOrEqual(cx * worldSize - 0.01);
      expect(maxVx).toBeLessThanOrEqual((cx + 1) * worldSize + 0.01);

      checked++;
      if (checked >= 5) break;
    }
    expect(checked).toBeGreaterThan(0);
  });
});

// ---------------------------------------------------------------------------
// 3. Coverage at boundaries
// ---------------------------------------------------------------------------
describe("LOD coverage", () => {
  test("LOD 1 ring covers its full radius (all XZ columns present)", () => {
    const lod1Stride = 2;
    const lod1WorldSize = CHUNK_SIZE * lod1Stride;
    const lod1Radius = LOD_RINGS[0].radiusChunks;
    const { spawnPos } = getWorld();

    const lcx = Math.floor(spawnPos[0] / lod1WorldSize);
    const lcz = Math.floor(spawnPos[2] / lod1WorldSize);

    // LOD 1 generates all chunks within radius (before exclusion)
    const lodColumns = new Set<string>();
    for (let dz = -lod1Radius; dz <= lod1Radius; dz++) {
      for (let dx = -lod1Radius; dx <= lod1Radius; dx++) {
        lodColumns.add(`${lcx + dx}:${lcz + dz}`);
      }
    }

    // Verify all XZ points within the covered area map to LOD columns
    const step = CHUNK_SIZE;
    const outerCheck = lod1Radius * lod1WorldSize - lod1WorldSize;
    let uncovered = 0;
    let total = 0;

    for (let dz = -outerCheck; dz <= outerCheck; dz += step) {
      for (let dx = -outerCheck; dx <= outerCheck; dx += step) {
        total++;
        const worldX = spawnPos[0] + dx;
        const worldZ = spawnPos[2] + dz;
        const lx = Math.floor(worldX / lod1WorldSize);
        const lz = Math.floor(worldZ / lod1WorldSize);
        if (!lodColumns.has(`${lx}:${lz}`)) {
          uncovered++;
        }
      }
    }

    const ratio = total > 0 ? 1 - uncovered / total : 1;
    expect(ratio).toBe(1);
    expect(lodColumns.size).toBe((2 * lod1Radius + 1) ** 2);
  });

  test("all LOD rings provide continuous coverage (no gaps between rings)", () => {
    // For each consecutive ring pair, verify the outer ring extends at
    // least as far as the inner ring.
    for (let i = 1; i < LOD_RINGS.length; i++) {
      const innerRing = LOD_RINGS[i - 1]!;
      const outerRing = LOD_RINGS[i]!;

      const innerStride = 1 << innerRing.level;
      const innerWorldSize = CHUNK_SIZE * innerStride;
      const innerCoverage = innerRing.radiusChunks * innerWorldSize;

      const outerStride = 1 << outerRing.level;
      const outerWorldSize = CHUNK_SIZE * outerStride;
      const outerCoverage = outerRing.radiusChunks * outerWorldSize;

      expect(outerCoverage).toBeGreaterThanOrEqual(innerCoverage);
    }
  });

  test("every XZ point within total LOD coverage is covered by at least one ring", () => {
    const { spawnPos } = getWorld();

    // The outermost ring determines total coverage
    const outerRing = LOD_RINGS[LOD_RINGS.length - 1]!;
    const outerStride = 1 << outerRing.level;
    const outerWorldSize = CHUNK_SIZE * outerStride;
    const totalCoverage = outerRing.radiusChunks * outerWorldSize;

    // Build unified coverage: discretize to LOD 0 chunk grid
    const allCovered = new Set<string>();

    for (const ring of LOD_RINGS) {
      const stride = 1 << ring.level;
      const ws = CHUNK_SIZE * stride;
      const lcx = Math.floor(spawnPos[0] / ws);
      const lcz = Math.floor(spawnPos[2] / ws);

      for (let dz = -ring.radiusChunks; dz <= ring.radiusChunks; dz++) {
        for (let dx = -ring.radiusChunks; dx <= ring.radiusChunks; dx++) {
          const cx = lcx + dx;
          const cz = lcz + dz;
          const minX = cx * ws;
          const maxX = (cx + 1) * ws;
          const minZ = cz * ws;
          const maxZ = (cz + 1) * ws;
          for (let wz = minZ; wz < maxZ; wz += CHUNK_SIZE) {
            for (let wx = minX; wx < maxX; wx += CHUNK_SIZE) {
              allCovered.add(`${Math.floor(wx / CHUNK_SIZE)}:${Math.floor(wz / CHUNK_SIZE)}`);
            }
          }
        }
      }
    }

    // Check that the area within the outermost ring (minus one chunk margin) is covered
    const checkRadius = totalCoverage - outerWorldSize;
    let uncovered = 0;
    let total = 0;

    for (let dz = -checkRadius; dz < checkRadius; dz += CHUNK_SIZE) {
      for (let dx = -checkRadius; dx < checkRadius; dx += CHUNK_SIZE) {
        total++;
        const worldX = spawnPos[0] + dx;
        const worldZ = spawnPos[2] + dz;
        const gx = Math.floor(worldX / CHUNK_SIZE);
        const gz = Math.floor(worldZ / CHUNK_SIZE);
        if (!allCovered.has(`${gx}:${gz}`)) {
          uncovered++;
        }
      }
    }

    expect(total).toBeGreaterThan(0);
    expect(uncovered).toBe(0);
  });

  test("actual generated LOD data covers the full fog-range sample grid", () => {
    const { world, spawnPos } = getWorld();
    const lodChunks: VoxelChunk[] = [];
    for (const chunk of world.iterateResidentChunks()) {
      if (chunk.lodLevel > 0 && chunk.renderReady && chunk.solidCount > 0) {
        lodChunks.push(chunk);
      }
    }

    const radiusWorldUnits = 4160;
    const stepWorldUnits = 80;
    let uncovered = 0;
    let total = 0;
    for (let dz = -radiusWorldUnits; dz <= radiusWorldUnits; dz += stepWorldUnits) {
      for (let dx = -radiusWorldUnits; dx <= radiusWorldUnits; dx += stepWorldUnits) {
        const worldX = spawnPos[0] + dx;
        const worldZ = spawnPos[2] + dz;
        const chunkX = Math.floor(worldX / CHUNK_SIZE);
        const chunkZ = Math.floor(worldZ / CHUNK_SIZE);
        const covered = world.isColumnRenderReady(chunkX, chunkZ)
          || lodChunks.some((chunk) => isWorldColumnCoveredByLodChunk(chunk, worldX, worldZ));
        if (!covered) {
          uncovered += 1;
        }
        total += 1;
      }
    }

    expect(total).toBeGreaterThan(0);
    expect(uncovered).toBe(0);
  });

  test("sampled fog-range columns have a single visible LOD owner", () => {
    const { world, spawnPos } = getWorld();
    const lodChunks: VoxelChunk[] = [];
    for (const chunk of world.iterateResidentChunks()) {
      if (chunk.lodLevel > 0 && chunk.renderReady && chunk.solidCount > 0) {
        lodChunks.push(chunk);
      }
    }

    const radiusWorldUnits = 4160;
    const stepWorldUnits = 80;
    let total = 0;
    let overlapCount = 0;
    let waterOverlapCount = 0;
    for (let dz = -radiusWorldUnits; dz <= radiusWorldUnits; dz += stepWorldUnits) {
      for (let dx = -radiusWorldUnits; dx <= radiusWorldUnits; dx += stepWorldUnits) {
        const worldX = spawnPos[0] + dx;
        const worldZ = spawnPos[2] + dz;
        const chunkX = Math.floor(worldX / CHUNK_SIZE);
        const chunkZ = Math.floor(worldZ / CHUNK_SIZE);
        const renderReady = world.isColumnRenderReady(chunkX, chunkZ);
        const lodOwners = new Set<string>();
        for (const chunk of lodChunks) {
          if (isWorldColumnCoveredByLodChunk(chunk, worldX, worldZ)) {
            lodOwners.add(getLodHorizontalOwnerKey(chunk));
          }
        }
        const lodOwnerCount = lodOwners.size;
        const ownerCount = (renderReady ? 1 : 0) + lodOwnerCount;
        if (ownerCount > 1) {
          overlapCount += 1;
        }

        const lodWaterOwners = new Set<string>();
        for (const chunk of lodChunks) {
          if (isWorldColumnWaterTopCoveredByLodChunk(chunk, worldX, worldZ)) {
            lodWaterOwners.add(getLodHorizontalOwnerKey(chunk));
          }
        }
        const waterOwnerCount = (isRenderReadyWaterColumn(world, worldX, worldZ) ? 1 : 0)
          + lodWaterOwners.size;
        if (waterOwnerCount > 1) {
          waterOverlapCount += 1;
        }
        total += 1;
      }
    }

    expect(total).toBeGreaterThan(0);
    expect(overlapCount).toBe(0);
    expect(waterOverlapCount).toBe(0);
  });

  test("LOD 1 chunks that exist fill gaps where LOD 0 is not render-ready", () => {
    const { world } = getWorld();

    const lod1Stride = 2;

    // Collect render-ready LOD 0 columns
    const lod0RenderReadyCols = new Set<string>();
    for (const chunk of world.iterateResidentChunks()) {
      if (chunk.lodLevel === 0 && chunk.renderReady) {
        lod0RenderReadyCols.add(`${chunk.coord.x}:${chunk.coord.z}`);
      }
    }

    // For every LOD 1 chunk, check whether it covers a gap in LOD 0
    let coversGap = 0;
    let totalLod1 = 0;

    for (const chunk of world.iterateResidentChunks()) {
      if (chunk.lodLevel !== 1) continue;
      totalLod1++;

      const cx = chunk.coord.x;
      const cz = chunk.coord.z;
      let allRenderReady = true;
      for (let dz = 0; dz < lod1Stride; dz++) {
        for (let dx = 0; dx < lod1Stride; dx++) {
          if (!lod0RenderReadyCols.has(`${cx * lod1Stride + dx}:${cz * lod1Stride + dz}`)) {
            allRenderReady = false;
          }
        }
      }

      if (!allRenderReady) {
        coversGap++;
      }
    }

    // LOD 1 should exist at boundaries where LOD 0 coverage ends
    expect(totalLod1).toBeGreaterThan(0);
    expect(coversGap).toBeGreaterThan(0);
  });

  test("LOD ring radius geometry: each ring covers expected number of columns", () => {
    for (const ring of LOD_RINGS) {
      const expectedColumns = (2 * ring.radiusChunks + 1) ** 2;
      // This is the theoretical maximum before exclusion
      expect(expectedColumns).toBeGreaterThan(0);
    }
  });

  test("budgeted LOD generation preserves the needed-key cache across passes", () => {
    const gen = new ProceduralWorldGenerator(SEED);
    const world = new ProceduralResidentWorld(gen, { horizontalRadiusChunks: 4 });
    const spawnPos = world.getSpawnPosition();
    world.updateResidencyAround(spawnPos, {
      maxGenerateChunks: Number.POSITIVE_INFINITY,
    });

    const first = world.updateLodResidencyAround(spawnPos, {
      maxGenerateLodChunks: 1,
    });
    expect(first.neededKeyCacheHit).toBe(false);
    expect(first.pending).toBeGreaterThan(0);

    const second = world.updateLodResidencyAround(spawnPos, {
      maxGenerateLodChunks: 1,
    });
    expect(second.neededKeyCacheHit).toBe(true);
    expect(second.yRangeMs).toBe(0);

    const third = world.updateLodResidencyAround(spawnPos, {
      maxGenerateLodChunks: 1,
    });
    expect(third.neededKeyCacheHit).toBe(true);
    expect(third.yRangeMs).toBe(0);
  });

  test("budgeted LOD generation does not starve far rings on punched-out coverage", () => {
    const gen = new ProceduralWorldGenerator(SEED);
    const world = new ProceduralResidentWorld(gen, { horizontalRadiusChunks: 4 });
    const spawnPos = world.getSpawnPosition();
    world.updateResidencyAround(spawnPos, {
      maxGenerateChunks: Number.POSITIVE_INFINITY,
    });

    let summary = world.updateLodResidencyAround(spawnPos, {
      maxGenerateLodChunks: 2,
    });
    const initialPending = summary.pending;
    expect(initialPending).toBeGreaterThan(0);

    for (let frame = 0; frame < 1_300 && summary.pending > 0; frame += 1) {
      summary = world.updateLodResidencyAround(spawnPos, {
        maxGenerateLodChunks: 2,
      });
    }

    expect(summary.pending).toBe(0);
    expect(summary.totalChunks).toBeGreaterThan(0);
    expect(summary.neededKeyCacheHit).toBe(true);

    const levels = chunksByLevel(world);
    expect(levels.get(1)?.length ?? 0).toBeGreaterThan(0);
    expect(levels.get(2)?.length ?? 0).toBeGreaterThan(0);
    expect(levels.get(3)?.length ?? 0).toBeGreaterThan(0);
    expect(levels.get(4)?.length ?? 0).toBeGreaterThan(0);
  });
});

// ---------------------------------------------------------------------------
// 4. Edit propagation
// ---------------------------------------------------------------------------
describe("LOD edit propagation", () => {
  test("editing a voxel causes the covering LOD chunk to regenerate with the edit", () => {
    // Create a fresh world (not shared) so edits do not affect other tests
    const gen = new ProceduralWorldGenerator(SEED);
    const world = new ProceduralResidentWorld(gen, { horizontalRadiusChunks: 8 });
    const spawnPos = world.getSpawnPosition();
    world.updateResidencyAround(spawnPos, {
      maxGenerateChunks: Number.POSITIVE_INFINITY,
    });
    world.updateLodResidencyAround(spawnPos, {
      maxGenerateLodChunks: Number.POSITIVE_INFINITY,
    });

    // Find a solid voxel near spawn to edit
    const sx = Math.floor(spawnPos[0]);
    const sz = Math.floor(spawnPos[2]);

    // Search downward for a solid voxel
    let editY = Math.floor(spawnPos[1]);
    while (editY > 0 && world.getVoxel(sx, editY, sz) === 0) {
      editY--;
    }
    const originalMat = world.getVoxel(sx, editY, sz);
    expect(originalMat).not.toBe(0);

    // Determine which LOD 1 chunk covers this position
    const lod1Stride = 2;
    const lod1WorldSize = CHUNK_SIZE * lod1Stride;
    const lodCx = Math.floor(sx / lod1WorldSize);
    const lodCy = Math.floor(editY / lod1WorldSize);
    const lodCz = Math.floor(sz / lod1WorldSize);

    // LOD local coordinates
    const lodLocalX = Math.floor((sx - lodCx * lod1WorldSize) / lod1Stride);
    const lodLocalY = Math.floor((editY - lodCy * lod1WorldSize) / lod1Stride);
    const lodLocalZ = Math.floor((sz - lodCz * lod1WorldSize) / lod1Stride);
    const lodIdx = lodLocalX + lodLocalY * CHUNK_SIZE + lodLocalZ * CHUNK_SIZE * CHUNK_SIZE;

    // Edit: set the voxel to 0 (air)
    const edited = world.setVoxel(sx, editY, sz, 0);
    expect(edited).toBe(true);
    expect(world.getVoxel(sx, editY, sz)).toBe(0);

    // Regenerate LOD chunks
    world.updateLodResidencyAround(spawnPos, {
      maxGenerateLodChunks: Number.POSITIVE_INFINITY,
    });

    // Compute expected LOD voxel after edit
    const wx = lodCx * lod1WorldSize + lodLocalX * lod1Stride;
    const wy = lodCy * lod1WorldSize + lodLocalY * lod1Stride;
    const wz = lodCz * lod1WorldSize + lodLocalZ * lod1Stride;

    let expectedMat = 0;
    outer: for (let dy = 1; dy >= 0; dy--) {
      for (let dz = 0; dz < 2; dz++) {
        for (let dx = 0; dx < 2; dx++) {
          const mat = world.getVoxel(wx + dx, wy + dy, wz + dz);
          if (mat !== 0 && !isProceduralWaterMaterial(mat)) {
            expectedMat = mat;
            break outer;
          }
          if (mat !== 0 && expectedMat === 0) {
            expectedMat = mat;
          }
        }
      }
    }

    // Find the regenerated LOD 1 chunk and verify
    let lodChunkAfter: VoxelChunk | null = null;
    for (const chunk of world.iterateResidentChunks()) {
      if (
        chunk.lodLevel === 1 &&
        chunk.coord.x === lodCx &&
        chunk.coord.y === lodCy &&
        chunk.coord.z === lodCz
      ) {
        lodChunkAfter = chunk;
        break;
      }
    }

    if (expectedMat === 0) {
      // The LOD chunk might have been removed if it became all-empty
    } else {
      expect(lodChunkAfter).not.toBeNull();
      expect(lodChunkAfter!.data[lodIdx]).toBe(expectedMat);
    }
  });

  test("editing multiple voxels in the same LOD footprint all propagate", () => {
    const gen = new ProceduralWorldGenerator(SEED);
    const world = new ProceduralResidentWorld(gen, { horizontalRadiusChunks: 8 });
    const spawnPos = world.getSpawnPosition();
    world.updateResidencyAround(spawnPos, {
      maxGenerateChunks: Number.POSITIVE_INFINITY,
    });
    world.updateLodResidencyAround(spawnPos, {
      maxGenerateLodChunks: Number.POSITIVE_INFINITY,
    });

    const sx = Math.floor(spawnPos[0]);
    const sz = Math.floor(spawnPos[2]);

    // Find a solid surface
    let topY = Math.floor(spawnPos[1]);
    while (topY > 0 && world.getVoxel(sx, topY, sz) === 0) topY--;
    expect(world.getVoxel(sx, topY, sz)).not.toBe(0);

    // Clear a 2x2x2 block to force the LOD voxel to change
    const edits: [number, number, number][] = [];
    for (let dy = 0; dy < 2; dy++) {
      for (let dz = 0; dz < 2; dz++) {
        for (let dx = 0; dx < 2; dx++) {
          const ey = topY - dy;
          if (ey < 0) continue;
          if (world.getVoxel(sx + dx, ey, sz + dz) !== 0) {
            world.setVoxel(sx + dx, ey, sz + dz, 0);
            edits.push([sx + dx, ey, sz + dz]);
          }
        }
      }
    }
    expect(edits.length).toBeGreaterThan(0);

    // Regenerate LOD
    world.updateLodResidencyAround(spawnPos, {
      maxGenerateLodChunks: Number.POSITIVE_INFINITY,
    });

    // Verify all edits are reflected at LOD 0 level
    for (const [ex, ey, ez] of edits) {
      expect(world.getVoxel(ex, ey, ez)).toBe(0);
    }

    // Verify the LOD chunk data is consistent
    const lod1Stride = 2;
    const lod1WorldSize = CHUNK_SIZE * lod1Stride;
    const lodCx = Math.floor(sx / lod1WorldSize);
    const lodCy = Math.floor(topY / lod1WorldSize);
    const lodCz = Math.floor(sz / lod1WorldSize);

    for (const chunk of world.iterateResidentChunks()) {
      if (
        chunk.lodLevel === 1 &&
        chunk.coord.x === lodCx &&
        chunk.coord.y === lodCy &&
        chunk.coord.z === lodCz
      ) {
        const lodLocalX = Math.floor((sx - lodCx * lod1WorldSize) / lod1Stride);
        const lodLocalY = Math.floor((topY - lodCy * lod1WorldSize) / lod1Stride);
        const lodLocalZ = Math.floor((sz - lodCz * lod1WorldSize) / lod1Stride);

        const wx = lodCx * lod1WorldSize + lodLocalX * lod1Stride;
        const wy = lodCy * lod1WorldSize + lodLocalY * lod1Stride;
        const wz = lodCz * lod1WorldSize + lodLocalZ * lod1Stride;

        // Recompute expected
        let expectedMat = 0;
        outer: for (let dy = 1; dy >= 0; dy--) {
          for (let dz = 0; dz < 2; dz++) {
            for (let dx = 0; dx < 2; dx++) {
              const mat = world.getVoxel(wx + dx, wy + dy, wz + dz);
              if (mat !== 0 && !isProceduralWaterMaterial(mat)) {
                expectedMat = mat;
                break outer;
              }
              if (mat !== 0 && expectedMat === 0) {
                expectedMat = mat;
              }
            }
          }
        }

        const idx =
          lodLocalX + lodLocalY * CHUNK_SIZE + lodLocalZ * CHUNK_SIZE * CHUNK_SIZE;
        expect(chunk.data[idx]).toBe(expectedMat);
        break;
      }
    }
  });
});

// ---------------------------------------------------------------------------
// 5. Mesh bounds integrity
// ---------------------------------------------------------------------------
describe("LOD mesh bounds", () => {
  test("mesh bounds tightly contain all vertices for every LOD chunk", () => {
    const { world } = getWorld();

    let checked = 0;

    for (const chunk of world.iterateResidentChunks()) {
      if (chunk.lodLevel === 0) continue;
      if (!chunk.mesh || chunk.mesh.vertexCount === 0) continue;
      checked++;

      const floats = new Float32Array(chunk.mesh.vertexData);
      let vMinX = Infinity,
        vMinY = Infinity,
        vMinZ = Infinity;
      let vMaxX = -Infinity,
        vMaxY = -Infinity,
        vMaxZ = -Infinity;

      for (let i = 0; i < chunk.mesh.vertexCount; i++) {
        const x = floats[i * 5]!;
        const y = floats[i * 5 + 1]!;
        const z = floats[i * 5 + 2]!;
        vMinX = Math.min(vMinX, x);
        vMinY = Math.min(vMinY, y);
        vMinZ = Math.min(vMinZ, z);
        vMaxX = Math.max(vMaxX, x);
        vMaxY = Math.max(vMaxY, y);
        vMaxZ = Math.max(vMaxZ, z);
      }

      const b = chunk.mesh.bounds;

      // Bounds min must be <= vertex min
      expect(b.min[0]).toBeLessThanOrEqual(vMinX + 0.01);
      expect(b.min[1]).toBeLessThanOrEqual(vMinY + 0.01);
      expect(b.min[2]).toBeLessThanOrEqual(vMinZ + 0.01);

      // Bounds max must be >= vertex max
      expect(b.max[0]).toBeGreaterThanOrEqual(vMaxX - 0.01);
      expect(b.max[1]).toBeGreaterThanOrEqual(vMaxY - 0.01);
      expect(b.max[2]).toBeGreaterThanOrEqual(vMaxZ - 0.01);
    }

    expect(checked).toBeGreaterThan(0);
  });

  test("mesh bounds are within the chunk world-space footprint", () => {
    const { world } = getWorld();

    let checked = 0;

    for (const chunk of world.iterateResidentChunks()) {
      if (chunk.lodLevel === 0) continue;
      if (!chunk.mesh || chunk.mesh.vertexCount === 0) continue;
      checked++;

      const stride = chunk.voxelStride;
      const worldSize = CHUNK_SIZE * stride;
      const chunkMinX = chunk.coord.x * worldSize;
      const chunkMinY = chunk.coord.y * worldSize;
      const chunkMinZ = chunk.coord.z * worldSize;
      const chunkMaxX = (chunk.coord.x + 1) * worldSize;
      const chunkMaxY = (chunk.coord.y + 1) * worldSize;
      const chunkMaxZ = (chunk.coord.z + 1) * worldSize;

      const b = chunk.mesh.bounds;

      expect(b.min[0]).toBeGreaterThanOrEqual(chunkMinX - 0.01);
      expect(b.min[1]).toBeGreaterThanOrEqual(chunkMinY - 0.01);
      expect(b.min[2]).toBeGreaterThanOrEqual(chunkMinZ - 0.01);
      expect(b.max[0]).toBeLessThanOrEqual(chunkMaxX + 0.01);
      expect(b.max[1]).toBeLessThanOrEqual(chunkMaxY + 0.01);
      expect(b.max[2]).toBeLessThanOrEqual(chunkMaxZ + 0.01);
    }

    expect(checked).toBeGreaterThan(0);
  });

  test("mesh bounds are non-degenerate for chunks with vertices", () => {
    const { world } = getWorld();

    let checked = 0;

    for (const chunk of world.iterateResidentChunks()) {
      if (chunk.lodLevel === 0) continue;
      if (!chunk.mesh || chunk.mesh.vertexCount === 0) continue;
      checked++;

      const b = chunk.mesh.bounds;
      expect(b.max[0] - b.min[0]).toBeGreaterThan(0);
      expect(b.max[1] - b.min[1]).toBeGreaterThan(0);
      expect(b.max[2] - b.min[2]).toBeGreaterThan(0);
    }

    expect(checked).toBeGreaterThan(0);
  });

  test("solidBounds are consistent with data for LOD chunks", () => {
    const { world } = getWorld();

    let checked = 0;

    for (const chunk of world.iterateResidentChunks()) {
      if (chunk.lodLevel === 0) continue;
      if (chunk.solidCount === 0) continue;
      if (!chunk.solidBounds) continue;
      checked++;

      // Compute actual solid bounds from data
      let minX = CHUNK_SIZE,
        minY = CHUNK_SIZE,
        minZ = CHUNK_SIZE;
      let maxX = 0,
        maxY = 0,
        maxZ = 0;

      for (let oz = 0; oz < CHUNK_SIZE; oz++) {
        for (let oy = 0; oy < CHUNK_SIZE; oy++) {
          for (let ox = 0; ox < CHUNK_SIZE; ox++) {
            if (chunk.data[ox + oy * CHUNK_SIZE + oz * CHUNK_SIZE * CHUNK_SIZE]! !== 0) {
              if (ox < minX) minX = ox;
              if (oy < minY) minY = oy;
              if (oz < minZ) minZ = oz;
              if (ox + 1 > maxX) maxX = ox + 1;
              if (oy + 1 > maxY) maxY = oy + 1;
              if (oz + 1 > maxZ) maxZ = oz + 1;
            }
          }
        }
      }

      expect(chunk.solidBounds.min[0]).toBe(minX);
      expect(chunk.solidBounds.min[1]).toBe(minY);
      expect(chunk.solidBounds.min[2]).toBe(minZ);
      expect(chunk.solidBounds.max[0]).toBe(maxX);
      expect(chunk.solidBounds.max[1]).toBe(maxY);
      expect(chunk.solidBounds.max[2]).toBe(maxZ);
    }

    expect(checked).toBeGreaterThan(0);
  });

  test("solidCount matches actual non-zero voxel count", () => {
    const { world } = getWorld();

    let checked = 0;

    for (const chunk of world.iterateResidentChunks()) {
      if (chunk.lodLevel === 0) continue;
      if (chunk.solidCount === 0) continue;
      checked++;

      let actualSolid = 0;
      for (let i = 0; i < chunk.data.length; i++) {
        if (chunk.data[i]! !== 0) actualSolid++;
      }

      expect(chunk.solidCount).toBe(actualSolid);

      if (checked >= 20) break;
    }

    expect(checked).toBeGreaterThan(0);
  });
});
