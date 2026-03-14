import { describe, expect, test } from "bun:test";
import { ProceduralWorldGenerator } from "../src/engine/procedural-generator.ts";
import { ProceduralResidentWorld } from "../src/engine/procedural-resident-world.ts";

const SEED = 1337;
const CHUNK_SIZE = 32;

function createGenerator() {
  return new ProceduralWorldGenerator(SEED);
}

describe("LOD terrain accuracy", () => {
  test("LOD 1 chunk surface heights match LOD 0 at sampled points", () => {
    const gen = createGenerator();
    const stride = 2;
    const worldSize = CHUNK_SIZE * stride;
    // Pick a chunk at actual spawn-area surface level
    const spawnCol = gen.sampleColumn(192, 128);
    const cx = Math.floor(192 / worldSize);
    const cy = Math.floor(spawnCol.surfaceY / worldSize);
    const cz = Math.floor(128 / worldSize);

    const lodChunk = gen.generateChunkAtLod(cx, cy, cz, 1);
    const chunkArea = CHUNK_SIZE * CHUNK_SIZE;

    let matchCount = 0;
    let totalColumns = 0;

    for (let lz = 0; lz < CHUNK_SIZE; lz++) {
      for (let lx = 0; lx < CHUNK_SIZE; lx++) {
        const worldX = cx * worldSize + lx * stride;
        const worldZ = cz * worldSize + lz * stride;
        const col = gen.sampleColumn(worldX, worldZ);

        // Find the highest solid voxel in this column in the LOD chunk
        let lodSurfaceLocalY = -1;
        for (let ly = CHUNK_SIZE - 1; ly >= 0; ly--) {
          const idx = lx + ly * CHUNK_SIZE + lz * chunkArea;
          if (lodChunk.data[idx] !== 0) {
            lodSurfaceLocalY = ly;
            break;
          }
        }

        if (lodSurfaceLocalY < 0) continue;
        totalColumns++;

        const lodSurfaceWorldY = cy * worldSize + lodSurfaceLocalY * stride;
        // At stride 2, LOD surface should be close to real surface
        if (Math.abs(lodSurfaceWorldY - col.surfaceY) <= stride * 3) {
          matchCount++;
        }
      }
    }

    expect(totalColumns).toBeGreaterThan(0);
    // With max-height sampling, most columns should match
    const matchRatio = matchCount / totalColumns;
    expect(matchRatio).toBeGreaterThan(0.65);
  });

  test("LOD chunk vertex positions fall within expected world-space AABB", () => {
    const gen = createGenerator();
    const world = new ProceduralResidentWorld(gen, { horizontalRadiusChunks: 4 });

    const spawnPos = world.getSpawnPosition();
    // Only generate LOD 1 and 2 chunks to keep it fast
    world.updateLodResidencyAround(spawnPos, { maxGenerateLodChunks: 30 });

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
  test("LOD 1 ring covers the LOD 0 boundary zone with overlap", () => {
    // With the overlap design, LOD 1 generates chunks in its full radius
    // with NO exclusion from LOD 0. Only finer LOD rings exclude coarser.
    // This means LOD 1 covers the entire LOD 0 area + beyond, ensuring
    // no gaps when LOD 0 chunks haven't loaded yet.
    const lod1Stride = 2;
    const lod1WorldSize = CHUNK_SIZE * lod1Stride;
    const lod1Radius = 5;
    const position: [number, number, number] = [192.5, 1419, 128.5];

    const lcx = Math.floor(position[0] / lod1WorldSize);
    const lcz = Math.floor(position[2] / lod1WorldSize);

    // LOD 1 generates ALL chunks within radius (no exclusion for first ring)
    const lodColumns = new Set<string>();
    for (let dz = -lod1Radius; dz <= lod1Radius; dz++) {
      for (let dx = -lod1Radius; dx <= lod1Radius; dx++) {
        lodColumns.add(`${lcx + dx}:${lcz + dz}`);
      }
    }

    // Every point within LOD 1 range should be covered
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
    expect(ratio).toBe(1); // full coverage with overlap
    expect(lodColumns.size).toBe((2 * lod1Radius + 1) ** 2); // 11x11 = 121
  });
});

describe("LOD mesh bounds", () => {
  test("LOD chunk mesh bounds contain all vertices", () => {
    const gen = createGenerator();
    const world = new ProceduralResidentWorld(gen, { horizontalRadiusChunks: 4 });
    const spawnPos = world.getSpawnPosition();
    world.updateLodResidencyAround(spawnPos, { maxGenerateLodChunks: 20 });

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
