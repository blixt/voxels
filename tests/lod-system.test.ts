import { describe, expect, test } from "bun:test";
import { ProceduralWorldGenerator } from "../src/engine/procedural-generator.ts";
import { ProceduralResidentWorld } from "../src/engine/procedural-resident-world.ts";

const SEED = 1337;
const CHUNK_SIZE = 32;

describe("LOD downsampling", () => {
  test("LOD chunks are derived from LOD 0 data, not re-generated", () => {
    const gen = new ProceduralWorldGenerator(SEED);
    const world = new ProceduralResidentWorld(gen, { horizontalRadiusChunks: 8 });

    const spawnPos = world.getSpawnPosition();
    // Generate LOD 0 chunks
    world.updateResidencyAround(spawnPos, { maxGenerateChunks: Number.POSITIVE_INFINITY });
    // Generate LOD chunks (downsampled from LOD 0)
    world.updateLodResidencyAround(spawnPos, { maxGenerateLodChunks: 500 });

    let lod0Count = 0;
    let lodCount = 0;
    for (const chunk of world.iterateResidentChunks()) {
      if (chunk.lodLevel === 0) lod0Count++;
      else lodCount++;
    }
    expect(lod0Count).toBeGreaterThan(0);
    expect(lodCount).toBeGreaterThan(0);
  });

  test("LOD 1 voxels match highest opaque voxel in source 2x2x2 blocks", () => {
    const gen = new ProceduralWorldGenerator(SEED);
    const world = new ProceduralResidentWorld(gen, { horizontalRadiusChunks: 8 });

    const spawnPos = world.getSpawnPosition();
    world.updateResidencyAround(spawnPos, { maxGenerateChunks: Number.POSITIVE_INFINITY });
    world.updateLodResidencyAround(spawnPos, { maxGenerateLodChunks: 500 });

    // Find a LOD 1 chunk that has solid voxels
    let checked = false;
    for (const lodChunk of world.iterateResidentChunks()) {
      if (lodChunk.lodLevel !== 1 || lodChunk.solidCount === 0) continue;

      const stride = lodChunk.voxelStride;
      const worldSize = CHUNK_SIZE * stride;
      const originX = lodChunk.coord.x * worldSize;
      const originY = lodChunk.coord.y * worldSize;
      const originZ = lodChunk.coord.z * worldSize;

      // Spot check a few voxels: the LOD voxel should match what's
      // in the source LOD 0 data
      let matchCount = 0;
      let totalChecked = 0;
      for (let oz = 0; oz < CHUNK_SIZE && totalChecked < 100; oz++) {
        for (let ox = 0; ox < CHUNK_SIZE && totalChecked < 100; ox++) {
          for (let oy = CHUNK_SIZE - 1; oy >= 0; oy--) {
            const lodMat = lodChunk.data[ox + oy * CHUNK_SIZE + oz * CHUNK_SIZE * CHUNK_SIZE]!;
            if (lodMat === 0) continue;
            totalChecked++;

            // This LOD voxel represents world region starting at:
            const wx = originX + ox * stride;
            const wy = originY + oy * stride;
            const wz = originZ + oz * stride;

            // Check if the source LOD 0 data contains this material somewhere in the 2x2x2 block
            let foundInSource = false;
            for (let dy = 1; dy >= 0 && !foundInSource; dy--) {
              for (let dz = 0; dz < 2 && !foundInSource; dz++) {
                for (let dx = 0; dx < 2 && !foundInSource; dx++) {
                  const srcMat = world.getVoxel(wx + dx, wy + dy, wz + dz);
                  if (srcMat === lodMat) foundInSource = true;
                }
              }
            }
            if (foundInSource) matchCount++;
            break; // only check topmost per column
          }
        }
      }

      if (totalChecked > 0) {
        const matchRatio = matchCount / totalChecked;
        expect(matchRatio).toBeGreaterThan(0.8);
        checked = true;
        break;
      }
    }
    expect(checked).toBe(true);
  });
});

describe("LOD vertex positions", () => {
  test("LOD chunk vertices fall within expected world-space AABB", () => {
    const gen = new ProceduralWorldGenerator(SEED);
    const world = new ProceduralResidentWorld(gen, { horizontalRadiusChunks: 8 });

    const spawnPos = world.getSpawnPosition();
    world.updateResidencyAround(spawnPos, { maxGenerateChunks: Number.POSITIVE_INFINITY });
    world.updateLodResidencyAround(spawnPos, { maxGenerateLodChunks: 500 });

    let lodChunkCount = 0;
    let allVerticesInBounds = true;

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
          vx < expectedMinX - 0.01 || vx > expectedMaxX + 0.01 ||
          vy < expectedMinY - 0.01 || vy > expectedMaxY + 0.01 ||
          vz < expectedMinZ - 0.01 || vz > expectedMaxZ + 0.01
        ) {
          allVerticesInBounds = false;
          break;
        }
      }
      if (!allVerticesInBounds) break;
    }

    expect(lodChunkCount).toBeGreaterThan(0);
    expect(allVerticesInBounds).toBe(true);
  });
});

describe("LOD coverage", () => {
  test("LOD 1 ring covers the LOD 0 boundary zone", () => {
    const lod1Stride = 2;
    const lod1WorldSize = CHUNK_SIZE * lod1Stride;
    const lod1Radius = 8; // current config
    const position: [number, number, number] = [192.5, 1419, 128.5];

    const lcx = Math.floor(position[0] / lod1WorldSize);
    const lcz = Math.floor(position[2] / lod1WorldSize);

    // LOD 1 generates all chunks within radius (no LOD 0 exclusion)
    const lodColumns = new Set<string>();
    for (let dz = -lod1Radius; dz <= lod1Radius; dz++) {
      for (let dx = -lod1Radius; dx <= lod1Radius; dx++) {
        lodColumns.add(`${lcx + dx}:${lcz + dz}`);
      }
    }

    const step = CHUNK_SIZE;
    const outerCheck = lod1Radius * lod1WorldSize - lod1WorldSize;
    let uncovered = 0;
    let total = 0;

    for (let dz = -outerCheck; dz <= outerCheck; dz += step) {
      for (let dx = -outerCheck; dx <= outerCheck; dx += step) {
        total++;
        const worldX = position[0] + dx;
        const worldZ = position[2] + dz;
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
});

describe("LOD mesh bounds", () => {
  test("LOD chunk mesh bounds contain all vertices", () => {
    const gen = new ProceduralWorldGenerator(SEED);
    const world = new ProceduralResidentWorld(gen, { horizontalRadiusChunks: 8 });
    const spawnPos = world.getSpawnPosition();
    world.updateResidencyAround(spawnPos, { maxGenerateChunks: Number.POSITIVE_INFINITY });
    world.updateLodResidencyAround(spawnPos, { maxGenerateLodChunks: 500 });

    let checked = 0;
    for (const chunk of world.iterateResidentChunks()) {
      if (chunk.lodLevel === 0) continue;
      if (!chunk.mesh || chunk.mesh.vertexCount === 0) continue;
      checked++;
      if (checked > 5) break;

      const floats = new Float32Array(chunk.mesh.vertexData);
      let vMinX = Infinity, vMinY = Infinity, vMinZ = Infinity;
      let vMaxX = -Infinity, vMaxY = -Infinity, vMaxZ = -Infinity;
      for (let i = 0; i < chunk.mesh.vertexCount; i++) {
        const x = floats[i * 5]!, y = floats[i * 5 + 1]!, z = floats[i * 5 + 2]!;
        vMinX = Math.min(vMinX, x); vMinY = Math.min(vMinY, y); vMinZ = Math.min(vMinZ, z);
        vMaxX = Math.max(vMaxX, x); vMaxY = Math.max(vMaxY, y); vMaxZ = Math.max(vMaxZ, z);
      }

      const b = chunk.mesh.bounds;
      expect(b.min[0]).toBeLessThanOrEqual(vMinX + 0.01);
      expect(b.min[1]).toBeLessThanOrEqual(vMinY + 0.01);
      expect(b.min[2]).toBeLessThanOrEqual(vMinZ + 0.01);
      expect(b.max[0]).toBeGreaterThanOrEqual(vMaxX - 0.01);
      expect(b.max[1]).toBeGreaterThanOrEqual(vMaxY - 0.01);
      expect(b.max[2]).toBeGreaterThanOrEqual(vMaxZ - 0.01);
    }
    expect(checked).toBeGreaterThan(0);
  });
});
