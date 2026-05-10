import { expect, test } from "bun:test";

import type { AsyncChunkGenerationQueue, AsyncDerivedLodChunkCacheKey } from "../src/engine/async-chunk-generation.ts";
import { encodeDerivedLodChunk } from "../src/engine/derived-lod-chunk-codec.ts";
import { rebuildDirtyMeshes } from "../src/engine/mesher.ts";
import { ProceduralResidentWorld } from "../src/engine/procedural-resident-world.ts";
import { ProceduralWorldGenerator } from "../src/engine/procedural-generator.ts";
import { metersToWorldUnits } from "../src/engine/scale.ts";
import type { VoxelChunk } from "../src/engine/world.ts";

class CountingProceduralWorldGenerator extends ProceduralWorldGenerator {
  sampleColumnCalls = 0;

  override sampleColumn(worldX: number, worldZ: number) {
    this.sampleColumnCalls += 1;
    return super.sampleColumn(worldX, worldZ);
  }
}

function createFakeAsyncQueue(overrides: Partial<AsyncChunkGenerationQueue> = {}): AsyncChunkGenerationQueue {
  return {
    requestChunk: () => false,
    requestSummary: () => false,
    requestRegionSummary: () => false,
    requestLodChunk: () => false,
    requestGeneratedLodChunk: () => false,
    storeLodChunk: () => false,
    hasPendingChunk: () => false,
    hasPendingRegionSummary: () => false,
    hasPendingLodChunk: () => false,
    hasPendingGeneratedLodChunk: () => false,
    getPendingCount: () => 0,
    drainCompletedChunks: () => [],
    drainCompletedSummaries: () => [],
    drainCompletedRegionSummaries: () => [],
    drainMissingRegionSummaries: () => [],
    drainCompletedLodChunks: () => [],
    drainMissingLodChunks: () => [],
    drainCompletionStats: () => ({ cacheHits: 0, generated: 0 }),
    drainSummaryCompletionStats: () => ({ cacheHits: 0, generated: 0 }),
    drainLodChunkCompletionStats: () => ({ cacheHits: 0, generated: 0, missing: 0, stored: 0 }),
    dispose: () => {},
    ...overrides,
  };
}

function toTestLodDiskKey(key: AsyncDerivedLodChunkCacheKey): string {
  return `${key.editRevision}:${key.lodLevel}:${key.coord.x}:${key.coord.y}:${key.coord.z}`;
}

function createTestDerivedLodPayload(key: AsyncDerivedLodChunkCacheKey, chunkSize: number) {
  const data = new Uint16Array(chunkSize * chunkSize * chunkSize);
  const chunkArea = chunkSize * chunkSize;
  const localX = Math.floor(chunkSize / 2);
  const localY = Math.floor(chunkSize / 2);
  const localZ = Math.floor(chunkSize / 2);
  data[localX + localY * chunkSize + localZ * chunkArea] = 1;
  return {
    coord: { ...key.coord },
    lodLevel: key.lodLevel,
    voxelStride: 1 << key.lodLevel,
    data,
    solidCount: 1,
    solidBounds: {
      min: [localX, localY, localZ] as [number, number, number],
      max: [localX + 1, localY + 1, localZ + 1] as [number, number, number],
    },
  };
}

function createTestVoxelChunk(
  cx: number,
  cy: number,
  cz: number,
  lodLevel: number,
  chunkSize: number,
): VoxelChunk {
  const data = new Uint16Array(chunkSize * chunkSize * chunkSize);
  const local = Math.floor(chunkSize / 2);
  data[local + local * chunkSize + local * chunkSize * chunkSize] = 1;
  return {
    coord: { x: cx, y: cy, z: cz },
    lodLevel,
    voxelStride: 1 << lodLevel,
    data,
    solidCount: 1,
    solidBounds: {
      min: [local, local, local],
      max: [local + 1, local + 1, local + 1],
      dirty: false,
    },
    meshBuilt: true,
    meshDirty: false,
    renderReady: true,
    meshRevision: 1,
    pendingMeshRevision: null,
    gpuDirty: false,
    mesh: null,
  };
}

test("resident world loads chunks around the player and exposes generated voxels", () => {
  const generator = new ProceduralWorldGenerator(1337, { chunkSize: 16 });
  const world = new ProceduralResidentWorld(generator, { horizontalRadiusChunks: 2 });
  const spawn = world.getSpawnPosition();
  const worldX = Math.floor(spawn[0]);
  const worldZ = Math.floor(spawn[2]);

  const residency = world.updateResidencyAround(spawn);
  const column = generator.sampleColumn(worldX, worldZ);

  expect(residency.changed).toBe(true);
  expect(world.getStats().chunkCount).toBeGreaterThan(0);
  expect(residency.generatedChunkCoords.length).toBe(residency.generatedChunks);
  expect(residency.evictedChunkCoords).toHaveLength(0);
  expect(residency.dirtyResidentChunks).toBeGreaterThanOrEqual(residency.generatedChunks);
  expect(residency.phaseMs.chunkGenerationMs).toBeGreaterThan(0);
  expect(residency.phaseMs.yRangeMs).toBeGreaterThan(0);
  expect(world.getVoxel(worldX, column.surfaceY, worldZ)).toBe(generator.sampleMaterial(worldX, column.surfaceY, worldZ));
});

test("resident world does not churn when the player stays in the same anchor chunk", () => {
  const world = new ProceduralResidentWorld(new ProceduralWorldGenerator(99, { chunkSize: 16 }), {
    horizontalRadiusChunks: 2,
  });
  const position: [number, number, number] = [2, 1450, 2];

  world.updateResidencyAround(position);
  const second = world.updateResidencyAround([position[0] + 1, position[1], position[2] + 1]);

  expect(second.changed).toBe(false);
  expect(second.generatedChunks).toBe(0);
  expect(second.evictedChunks).toBe(0);
  expect(second.generatedChunkCoords).toHaveLength(0);
  expect(second.evictedChunkCoords).toHaveLength(0);
  expect(second.phaseMs.surfaceSampleMs).toBeGreaterThanOrEqual(0);
  expect(second.phaseMs.chunkGenerationMs).toBe(0);
});

test("resident world evicts far chunks and loads new chunks after a large move", () => {
  const world = new ProceduralResidentWorld(new ProceduralWorldGenerator(7, { chunkSize: 16 }), {
    horizontalRadiusChunks: 1,
  });

  world.updateResidencyAround([0, 1400, 0]);
  const before = world.getStats().chunkCount;
  const shifted = world.updateResidencyAround([16 * 6, 1400, 0]);

  expect(before).toBeGreaterThan(0);
  expect(shifted.generatedChunks).toBeGreaterThan(0);
  expect(shifted.evictedChunks).toBeGreaterThan(0);
  expect(shifted.generatedChunkCoords.length).toBe(shifted.generatedChunks);
  expect(shifted.evictedChunkCoords.length).toBe(shifted.evictedChunks);
  expect(shifted.phaseMs.neighborDirtyMs).toBeGreaterThanOrEqual(0);
  expect(world.hasResidentChunk(0, Math.floor(1400 / 16), 0)).toBe(false);
});

test("changing view distance forces a broader residency window", () => {
  const world = new ProceduralResidentWorld(new ProceduralWorldGenerator(5, { chunkSize: 16 }), {
    horizontalRadiusChunks: 1,
  });
  const position: [number, number, number] = [0, 1400, 0];

  world.updateResidencyAround(position);
  const before = world.getStats().chunkCount;
  world.setHorizontalRadiusChunks(3);
  const after = world.updateResidencyAround(position);

  expect(after.changed).toBe(true);
  expect(after.radiusChunks).toBe(3);
  expect(world.getStats().chunkCount).toBeGreaterThan(before);
});

test("spawn selection prefers a flatter standing footprint", () => {
  const world = new ProceduralResidentWorld(new ProceduralWorldGenerator(1337));
  const spawn = world.getSpawnPosition();
  const centerX = Math.floor(spawn[0]);
  const centerZ = Math.floor(spawn[2]);
  const footprintRadius = metersToWorldUnits(0.8);
  const heights = [
    world.generator.sampleColumn(centerX, centerZ).surfaceY,
    world.generator.sampleColumn(centerX - footprintRadius, centerZ).surfaceY,
    world.generator.sampleColumn(centerX + footprintRadius, centerZ).surfaceY,
    world.generator.sampleColumn(centerX, centerZ - footprintRadius).surfaceY,
    world.generator.sampleColumn(centerX, centerZ + footprintRadius).surfaceY,
    world.generator.sampleColumn(centerX - footprintRadius, centerZ - footprintRadius).surfaceY,
    world.generator.sampleColumn(centerX - footprintRadius, centerZ + footprintRadius).surfaceY,
    world.generator.sampleColumn(centerX + footprintRadius, centerZ - footprintRadius).surfaceY,
    world.generator.sampleColumn(centerX + footprintRadius, centerZ + footprintRadius).surfaceY,
  ];

  expect(Math.max(...heights) - Math.min(...heights)).toBeLessThanOrEqual(12);
  // Spawn Y should be 1 above the highest standable surface in the footprint
  expect(spawn[1]).toBeGreaterThanOrEqual(Math.max(...heights));
  expect(spawn[1]).toBeLessThanOrEqual(Math.max(...heights) + 2);
});

test("spawn selection avoids unsupported cave-breached footprint columns", () => {
  const world = new ProceduralResidentWorld(new ProceduralWorldGenerator(1337));
  const spawn = world.getSpawnPosition();
  const centerX = Math.floor(spawn[0]);
  const centerZ = Math.floor(spawn[2]);
  const footprintRadius = metersToWorldUnits(0.8);
  const maxSurfaceDrop = metersToWorldUnits(1.2);
  const headroom = metersToWorldUnits(1.8);

  for (const [offsetX, offsetZ] of [
    [0, 0],
    [-footprintRadius, 0],
    [footprintRadius, 0],
    [0, -footprintRadius],
    [0, footprintRadius],
    [-footprintRadius, -footprintRadius],
    [-footprintRadius, footprintRadius],
    [footprintRadius, -footprintRadius],
    [footprintRadius, footprintRadius],
  ] as const) {
    const worldX = centerX + offsetX;
    const worldZ = centerZ + offsetZ;
    const column = world.generator.sampleSurfaceColumn(worldX, worldZ);
    let supported = false;
    for (let worldY = column.surfaceY; worldY >= column.surfaceY - maxSurfaceDrop; worldY -= 1) {
      if (!world.isCollisionMaterial(world.generator.sampleMaterial(worldX, worldY, worldZ))) {
        continue;
      }
      if (world.generator.sampleMaterial(worldX, worldY + 1, worldZ) !== 0) {
        continue;
      }
      if (world.generator.sampleMaterial(worldX, worldY + headroom - 1, worldZ) !== 0) {
        continue;
      }
      supported = true;
      break;
    }
    expect(supported).toBe(true);
  }
});

test("spawn position is cached after the first search", () => {
  const generator = new CountingProceduralWorldGenerator(1337);
  const world = new ProceduralResidentWorld(generator);

  const first = world.getSpawnPosition();
  const firstSampleColumnCalls = generator.sampleColumnCalls;

  generator.sampleColumnCalls = 0;
  const second = world.getSpawnPosition();

  expect(first).toEqual(second);
  expect(firstSampleColumnCalls).toBeGreaterThan(0);
  expect(generator.sampleColumnCalls).toBe(0);
});

test("resident world reuses cached empty chunk knowledge across residency changes", () => {
  const world = new ProceduralResidentWorld(new ProceduralWorldGenerator(1337, { chunkSize: 16 }), {
    horizontalRadiusChunks: 2,
  });
  const spawn = world.getSpawnPosition();

  const bootstrap = world.updateResidencyAround(spawn);
  world.setHorizontalRadiusChunks(1);
  const shrink = world.updateResidencyAround(spawn);
  world.setHorizontalRadiusChunks(2);
  const widen = world.updateResidencyAround(spawn);

  expect(bootstrap.emptyChunksSkipped).toBeGreaterThan(0);
  expect(bootstrap.cachedEmptyChunkHits).toBe(0);

  expect(shrink.generatedChunks).toBe(0);
  expect(shrink.emptyChunksSkipped).toBe(0);
  expect(shrink.cachedEmptyChunkHits).toBeGreaterThan(0);
  expect(shrink.phaseMs.chunkGenerationMs).toBe(0);

  expect(widen.generatedChunks).toBeGreaterThan(0);
  expect(widen.emptyChunksSkipped).toBe(0);
  expect(widen.cachedEmptyChunkHits).toBe(bootstrap.emptyChunksSkipped);
});

test("resident world reuses known-empty chunk results on a repeated anchor refresh", () => {
  const world = new ProceduralResidentWorld(new ProceduralWorldGenerator(1337, { chunkSize: 16 }), {
    horizontalRadiusChunks: 2,
  });
  const spawn = world.getSpawnPosition();

  const first = world.updateResidencyAround(spawn);
  world.setHorizontalRadiusChunks(2);
  const second = world.updateResidencyAround(spawn);

  expect(first.emptyChunksSkipped).toBeGreaterThan(0);
  expect(first.cachedEmptyChunkHits).toBe(0);
  expect(second.generatedChunks).toBe(0);
  expect(second.emptyChunksSkipped).toBe(0);
  expect(second.cachedEmptyChunkHits).toBe(first.emptyChunksSkipped);
  expect(second.phaseMs.chunkGenerationMs).toBe(0);
});

test("LOD Y-range planning avoids full procedural column fallback", () => {
  const generator = new CountingProceduralWorldGenerator(1337);
  const world = new ProceduralResidentWorld(generator, { horizontalRadiusChunks: 4 });
  const spawn = world.getSpawnPosition();
  world.updateResidencyAround(spawn, {
    maxGenerateChunks: Number.POSITIVE_INFINITY,
  });

  generator.sampleColumnCalls = 0;
  const lod = world.updateLodResidencyAround(spawn, {
    maxGenerateLodChunks: 0,
  });

  expect(lod.neededKeyCount).toBeGreaterThan(0);
  expect(lod.pending).toBeGreaterThan(0);
  expect(generator.sampleColumnCalls).toBe(0);
});

test("LOD planning asks the persistent chunk store for missing render-summary regions", () => {
  const requested = new Set<string>();
  const queue = createFakeAsyncQueue({
    requestRegionSummary: (regionX, regionZ) => {
      requested.add(`${regionX}:${regionZ}`);
      return true;
    },
    hasPendingRegionSummary: (regionX, regionZ) => requested.has(`${regionX}:${regionZ}`),
  });
  const world = new ProceduralResidentWorld(new ProceduralWorldGenerator(1337, { chunkSize: 16 }), {
    asyncChunkGeneration: queue,
    horizontalRadiusChunks: 1,
  });

  const lod = world.updateLodResidencyAround(world.getSpawnPosition(), {
    maxGenerateLodChunks: 0,
  });

  expect(requested.size).toBeGreaterThan(0);
  expect(lod.scheduledRegionSummaryRequests).toBe(requested.size);
});

test("LOD residency schedules a bounded number of derived LOD disk-cache requests before rebuilding", () => {
  const requestedKeys: AsyncDerivedLodChunkCacheKey[] = [];
  const pendingKeys = new Set<string>();
  const queue = createFakeAsyncQueue({
    requestLodChunk: (key) => {
      requestedKeys.push({ ...key, coord: { ...key.coord } });
      pendingKeys.add(toTestLodDiskKey(key));
      return true;
    },
    hasPendingLodChunk: (key) => pendingKeys.has(toTestLodDiskKey(key)),
  });
  const world = new ProceduralResidentWorld(new ProceduralWorldGenerator(1337, { chunkSize: 16 }), {
    asyncChunkGeneration: queue,
    horizontalRadiusChunks: 1,
  });

  const lod = world.updateLodResidencyAround(world.getSpawnPosition(), {
    maxGenerateLodChunks: 0,
  });

  expect(lod.scheduledLodDiskRequests).toBeGreaterThan(0);
  expect(lod.scheduledLodDiskRequests).toBeLessThanOrEqual(64);
  expect(requestedKeys.length).toBe(lod.scheduledLodDiskRequests);
  expect(lod.pending).toBeGreaterThan(0);
});

test("LOD residency schedules async generated derived chunks after disk-cache misses", () => {
  const diskRequests: AsyncDerivedLodChunkCacheKey[] = [];
  const generatedKeys: AsyncDerivedLodChunkCacheKey[] = [];
  const pendingDiskKeys = new Set<string>();
  const pendingGeneratedKeys = new Set<string>();
  const queue = createFakeAsyncQueue({
    requestLodChunk: (key) => {
      diskRequests.push({ ...key, coord: { ...key.coord } });
      pendingDiskKeys.add(toTestLodDiskKey(key));
      return true;
    },
    hasPendingLodChunk: (key) => pendingDiskKeys.has(toTestLodDiskKey(key)),
    drainMissingLodChunks: () => {
      const missing = diskRequests.splice(0, diskRequests.length);
      pendingDiskKeys.clear();
      return missing;
    },
    drainLodChunkCompletionStats: () => ({ cacheHits: 0, generated: 0, missing: diskRequests.length, stored: 0 }),
    requestGeneratedLodChunk: (key) => {
      generatedKeys.push({ ...key, coord: { ...key.coord } });
      pendingGeneratedKeys.add(toTestLodDiskKey(key));
      return true;
    },
    hasPendingGeneratedLodChunk: (key) => pendingGeneratedKeys.has(toTestLodDiskKey(key)),
  });
  const world = new ProceduralResidentWorld(new ProceduralWorldGenerator(1337, { chunkSize: 16 }), {
    asyncChunkGeneration: queue,
    horizontalRadiusChunks: 1,
  });

  let lod = world.updateLodResidencyAround(world.getSpawnPosition(), {
    maxGenerateLodChunks: Number.POSITIVE_INFINITY,
  });
  for (let update = 0; update < 24 && lod.scheduledLodWorkerRequests === 0; update += 1) {
    lod = world.updateLodResidencyAround(world.getSpawnPosition(), {
      maxGenerateLodChunks: Number.POSITIVE_INFINITY,
    });
  }

  expect(lod.scheduledLodWorkerRequests).toBeGreaterThan(0);
  expect(lod.scheduledLodWorkerRequests).toBeLessThanOrEqual(16);
  expect(generatedKeys.length).toBe(lod.scheduledLodWorkerRequests);
  expect(generatedKeys.every((key) => key.lodLevel >= 1)).toBe(true);
  expect(lod.generatedByLevel.slice(1).every((count) => count === 0)).toBe(true);
  expect(lod.pendingGenerationBudget).toBeGreaterThan(0);
});

test("LOD residency adopts completed derived LOD disk-cache hits before rebuilding", () => {
  const probeRequests: AsyncDerivedLodChunkCacheKey[] = [];
  const probeWorld = new ProceduralResidentWorld(new ProceduralWorldGenerator(1337, { chunkSize: 16 }), {
    asyncChunkGeneration: createFakeAsyncQueue({
      requestLodChunk: (key) => {
        probeRequests.push({ ...key, coord: { ...key.coord } });
        return true;
      },
    }),
    horizontalRadiusChunks: 1,
  });
  const spawn = probeWorld.getSpawnPosition();
  probeWorld.updateLodResidencyAround(spawn, { maxGenerateLodChunks: 0 });
  const key = probeRequests[0]!;

  const encoded = encodeDerivedLodChunk(createTestDerivedLodPayload(key, 16));
  let drained = false;
  const world = new ProceduralResidentWorld(new ProceduralWorldGenerator(1337, { chunkSize: 16 }), {
    asyncChunkGeneration: createFakeAsyncQueue({
      drainCompletedLodChunks: () => {
        if (drained) {
          return [];
        }
        drained = true;
        return [{
          key,
          buffer: encoded.buffer,
          byteLength: encoded.stats.byteLength,
        }];
      },
    }),
    horizontalRadiusChunks: 1,
  });

  const lod = world.updateLodResidencyAround(spawn, { maxGenerateLodChunks: 0 });

  expect(lod.lodDiskCacheHits).toBe(1);
  expect(lod.generated).toBe(0);
});

test("LOD residency tracks worker-generated derived LOD chunks separately from disk hits", () => {
  const probeRequests: AsyncDerivedLodChunkCacheKey[] = [];
  const probeWorld = new ProceduralResidentWorld(new ProceduralWorldGenerator(1337, { chunkSize: 16 }), {
    asyncChunkGeneration: createFakeAsyncQueue({
      requestLodChunk: (key) => {
        probeRequests.push({ ...key, coord: { ...key.coord } });
        return true;
      },
    }),
    horizontalRadiusChunks: 1,
  });
  const spawn = probeWorld.getSpawnPosition();
  probeWorld.updateLodResidencyAround(spawn, { maxGenerateLodChunks: 0 });
  const key = probeRequests.find((request) => request.lodLevel >= 2) ?? probeRequests[0]!;

  const encoded = encodeDerivedLodChunk(createTestDerivedLodPayload(key, 16));
  let drained = false;
  const world = new ProceduralResidentWorld(new ProceduralWorldGenerator(1337, { chunkSize: 16 }), {
    asyncChunkGeneration: createFakeAsyncQueue({
      drainCompletedLodChunks: () => {
        if (drained) {
          return [];
        }
        drained = true;
        return [{
          key,
          source: "generated",
          buffer: encoded.buffer,
          byteLength: encoded.stats.byteLength,
        }];
      },
    }),
    horizontalRadiusChunks: 1,
  });

  const lod = world.updateLodResidencyAround(spawn, { maxGenerateLodChunks: 0 });

  expect(lod.lodDiskCacheHits).toBe(0);
  expect(lod.lodWorkerGenerated).toBe(1);
  expect(lod.generated).toBe(0);
});

test("LOD residency queues active derived chunks for persistence before eviction", () => {
  const storedKeys: AsyncDerivedLodChunkCacheKey[] = [];
  const world = new ProceduralResidentWorld(new ProceduralWorldGenerator(1337, { chunkSize: 16 }), {
    asyncChunkGeneration: createFakeAsyncQueue({
      storeLodChunk: (chunk) => {
        storedKeys.push({ ...chunk.key, coord: { ...chunk.key.coord } });
        return true;
      },
    }),
    horizontalRadiusChunks: 1,
  });
  const worldWithInternals = world as unknown as {
    lodChunks: Map<string, VoxelChunk>;
    enqueueLodDiskStore(key: string): void;
    flushQueuedLodDiskStores(): number;
  };
  worldWithInternals.lodChunks.set("L1:0:0:0", createTestVoxelChunk(0, 0, 0, 1, 16));
  worldWithInternals.enqueueLodDiskStore("L1:0:0:0");
  const scheduledStores = worldWithInternals.flushQueuedLodDiskStores();

  expect(scheduledStores).toBeGreaterThan(0);
  expect(storedKeys.length).toBe(scheduledStores);
  expect(storedKeys.every((key) => key.lodLevel > 0)).toBe(true);
});

test("resident world tracks dirty chunks without rescanning the full resident set", () => {
  const world = new ProceduralResidentWorld(new ProceduralWorldGenerator(1337, { chunkSize: 16 }), {
    horizontalRadiusChunks: 2,
  });
  const spawn = world.getSpawnPosition();

  world.updateResidencyAround(spawn, { maxGenerateChunks: Number.POSITIVE_INFINITY });
  const dirtyBefore = Array.from(world.iterateDirtyResidentChunks());

  expect(dirtyBefore.length).toBeGreaterThan(0);
  expect(dirtyBefore.length).toBe(world.countDirtyResidentChunks());

  rebuildDirtyMeshes(world, Number.POSITIVE_INFINITY, { priorityPosition: spawn });

  expect(Array.from(world.iterateDirtyResidentChunks())).toHaveLength(0);
  expect(world.countDirtyResidentChunks()).toBe(0);
});

test("resident world caches column y-range sampling across repeated updates", () => {
  const generator = new CountingProceduralWorldGenerator(1337, { chunkSize: 16 });
  const world = new ProceduralResidentWorld(generator, {
    horizontalRadiusChunks: 2,
  });
  const spawn = world.getSpawnPosition();

  world.updateResidencyAround(spawn, { maxGenerateChunks: 4 });
  expect(generator.sampleColumnCalls).toBeGreaterThan(1);

  generator.sampleColumnCalls = 0;
  world.updateResidencyAround(spawn, { maxGenerateChunks: 4 });

  expect(generator.sampleColumnCalls).toBe(1);
});

test("resident world can complete a residency window across multiple budgeted updates", () => {
  const world = new ProceduralResidentWorld(new ProceduralWorldGenerator(1337, { chunkSize: 16 }), {
    horizontalRadiusChunks: 2,
  });
  const spawn = world.getSpawnPosition();

  const first = world.updateResidencyAround(spawn, { maxGenerateChunks: 4 });

  expect(first.complete).toBe(false);
  expect(first.pendingChunks).toBeGreaterThan(0);
  expect(first.generatedChunks).toBeGreaterThan(0);
  expect(first.generatedChunks).toBeLessThanOrEqual(4);

  let latest = first;
  for (let attempt = 0; attempt < 24 && !latest.complete; attempt += 1) {
    latest = world.updateResidencyAround(spawn, { maxGenerateChunks: 4 });
  }

  expect(latest.complete).toBe(true);
  expect(latest.pendingChunks).toBe(0);
  expect(world.getStats().chunkCount).toBeGreaterThan(0);

  const settled = world.updateResidencyAround(spawn, { maxGenerateChunks: 4 });
  expect(settled.changed).toBe(false);
  expect(settled.complete).toBe(true);
  expect(settled.pendingChunks).toBe(0);
});

test("budgeted residency planning does not evict from an incomplete needed-key scan", () => {
  const world = new ProceduralResidentWorld(new ProceduralWorldGenerator(1337, { chunkSize: 16 }), {
    horizontalRadiusChunks: 1,
  });
  const spawn = world.getSpawnPosition();
  world.updateResidencyAround(spawn, { maxGenerateChunks: Number.POSITIVE_INFINITY });
  const originalChunk = {
    x: Math.floor(spawn[0] / world.chunkSize),
    y: Math.floor(spawn[1] / world.chunkSize),
    z: Math.floor(spawn[2] / world.chunkSize),
  };
  expect(world.hasResidentChunk(originalChunk.x, originalChunk.y, originalChunk.z)).toBe(true);

  const farPosition: [number, number, number] = [
    spawn[0] + world.chunkSize * 8,
    spawn[1],
    spawn[2],
  ];
  const budgeted = world.updateResidencyAround(farPosition, {
    maxGenerateChunks: 0,
    maxPlanMs: 0,
  });

  expect(budgeted.complete).toBe(false);
  expect(budgeted.pendingChunks).toBeGreaterThan(0);
  expect(budgeted.evictedChunks).toBe(0);
  expect(world.hasResidentChunk(originalChunk.x, originalChunk.y, originalChunk.z)).toBe(true);

  const full = world.updateResidencyAround(farPosition, {
    maxGenerateChunks: Number.POSITIVE_INFINITY,
  });

  expect(full.complete).toBe(true);
  expect(full.evictedChunks).toBeGreaterThan(0);
  expect(world.hasResidentChunk(originalChunk.x, originalChunk.y, originalChunk.z)).toBe(false);
});

test("resident world can amortize eviction across budgeted updates", () => {
  const world = new ProceduralResidentWorld(new ProceduralWorldGenerator(1337, { chunkSize: 16 }), {
    horizontalRadiusChunks: 1,
  });
  const spawn = world.getSpawnPosition();
  world.updateResidencyAround(spawn, { maxGenerateChunks: Number.POSITIVE_INFINITY });

  const shifted: [number, number, number] = [
    spawn[0] + world.chunkSize * 8,
    spawn[1],
    spawn[2],
  ];
  const first = world.updateResidencyAround(shifted, {
    maxGenerateChunks: Number.POSITIVE_INFINITY,
    maxEvictChunks: 2,
  });

  expect(first.complete).toBe(false);
  expect(first.evictedChunks).toBe(2);
  expect(first.pendingChunks).toBeGreaterThan(0);

  let latest = first;
  for (let attempt = 0; attempt < 16 && !latest.complete; attempt += 1) {
    latest = world.updateResidencyAround(shifted, {
      maxGenerateChunks: Number.POSITIVE_INFINITY,
      maxEvictChunks: 2,
    });
  }

  expect(latest.complete).toBe(true);
  expect(latest.pendingChunks).toBe(0);
});

test("resident world can stream the same anchor incrementally under a generation budget", () => {
  const world = new ProceduralResidentWorld(new ProceduralWorldGenerator(1337, { chunkSize: 16 }), {
    horizontalRadiusChunks: 3,
  });
  const spawn = world.getSpawnPosition();

  const first = world.updateResidencyAround(spawn, { maxGenerateChunks: 4 });

  expect(first.complete).toBe(false);
  expect(first.pendingChunks).toBeGreaterThan(0);
  expect(first.generatedChunks).toBeGreaterThan(0);
  expect(first.generatedChunks).toBeLessThanOrEqual(4);

  let latest = first;
  for (let iteration = 0; iteration < 64 && !latest.complete; iteration += 1) {
    latest = world.updateResidencyAround(spawn, { maxGenerateChunks: 4 });
  }

  expect(latest.complete).toBe(true);
  expect(latest.pendingChunks).toBe(0);
  expect(latest.generatedChunks).toBeGreaterThanOrEqual(0);

  const settled = world.updateResidencyAround(spawn, { maxGenerateChunks: 4 });
  expect(settled.changed).toBe(false);
  expect(settled.complete).toBe(true);
  expect(settled.pendingChunks).toBe(0);
});

test("resident world budgets adoption of completed async chunks across frames", () => {
  const generator = new ProceduralWorldGenerator(1337, { chunkSize: 16 });
  const requestedCoords: Array<{ x: number; y: number; z: number }> = [];
  const requestedKeys = new Set<string>();
  const discoveryWorld = new ProceduralResidentWorld(generator, {
    horizontalRadiusChunks: 2,
    asyncChunkGeneration: createFakeAsyncQueue({
      requestChunk(cx, cy, cz) {
        const key = `${cx}:${cy}:${cz}`;
        if (requestedKeys.has(key)) {
          return false;
        }
        requestedKeys.add(key);
        requestedCoords.push({ x: cx, y: cy, z: cz });
        return true;
      },
      hasPendingChunk(cx, cy, cz) {
        return requestedKeys.has(`${cx}:${cy}:${cz}`);
      },
      getPendingCount() {
        return requestedKeys.size;
      },
    }),
  });
  const spawn = discoveryWorld.getSpawnPosition();

  discoveryWorld.updateResidencyAround(spawn, { maxGenerateChunks: Number.POSITIVE_INFINITY });

  expect(requestedCoords.length).toBeGreaterThan(0);

  const completedChunks = requestedCoords.map((coord) => generator.generateChunk(coord.x, coord.y, coord.z));
  let drained = false;
  let unexpectedRequests = 0;
  const world = new ProceduralResidentWorld(generator, {
    horizontalRadiusChunks: 2,
    asyncChunkGeneration: createFakeAsyncQueue({
      drainCompletedChunks() {
        if (drained) {
          return [];
        }
        drained = true;
        return completedChunks;
      },
      requestChunk() {
        unexpectedRequests += 1;
        return false;
      },
    }),
  });

  const first = world.updateResidencyAround(spawn, { maxGenerateChunks: 4 });

  expect(unexpectedRequests).toBe(0);
  expect(first.generatedChunks).toBeLessThanOrEqual(4);
  expect(first.complete).toBe(false);
  expect(first.pendingChunks).toBeGreaterThan(0);
  expect(first.phaseMs.readyGeneratedChunkBacklog).toBeGreaterThan(0);

  let latest = first;
  for (let iteration = 0; iteration < 64 && !latest.complete; iteration += 1) {
    latest = world.updateResidencyAround(spawn, { maxGenerateChunks: 4 });
  }

  expect(latest.complete).toBe(true);
  expect(latest.pendingChunks).toBe(0);
  expect(latest.phaseMs.readyGeneratedChunkBacklog).toBe(0);
  expect(unexpectedRequests).toBe(0);
});

test("budgeted residency prioritizes the spawn support chunk first", () => {
  const world = new ProceduralResidentWorld(new ProceduralWorldGenerator(1337, { chunkSize: 16 }), {
    horizontalRadiusChunks: 3,
  });
  const spawn = world.getSpawnPosition();

  world.updateResidencyAround(spawn, { maxGenerateChunks: 1 });

  const centerChunkX = Math.floor(spawn[0] / world.chunkSize);
  const centerChunkZ = Math.floor(spawn[2] / world.chunkSize);
  const supportChunkY = Math.floor((spawn[1] - 1) / world.chunkSize);
  expect(world.hasResidentChunk(centerChunkX, supportChunkY, centerChunkZ)).toBe(true);
});

test("resident world reapplies edit overlays after chunk eviction and regeneration", () => {
  const generator = new ProceduralWorldGenerator(1337, { chunkSize: 16 });
  const world = new ProceduralResidentWorld(generator, { horizontalRadiusChunks: 1 });
  const spawn = world.getSpawnPosition();
  world.updateResidencyAround(spawn, { maxGenerateChunks: Number.POSITIVE_INFINITY });

  const targetX = Math.floor(spawn[0]);
  const targetZ = Math.floor(spawn[2]);
  // Find a solid voxel near the surface (surface block may be cave-breached)
  let targetY = generator.sampleColumn(targetX, targetZ).surfaceY;
  while (targetY > 0 && world.getVoxel(targetX, targetY, targetZ) === 0) targetY--;
  const previous = world.getVoxel(targetX, targetY, targetZ);

  expect(previous).not.toBe(0);
  expect(world.setVoxel(targetX, targetY, targetZ, 0)).toBe(true);
  expect(world.getVoxel(targetX, targetY, targetZ)).toBe(0);

  const farPosition: [number, number, number] = [spawn[0] + generator.chunkSize * 8, spawn[1], spawn[2]];
  world.updateResidencyAround(farPosition, { maxGenerateChunks: Number.POSITIVE_INFINITY });
  world.updateResidencyAround(spawn, { maxGenerateChunks: Number.POSITIVE_INFINITY });

  expect(world.getVoxel(targetX, targetY, targetZ)).toBe(0);
  expect(world.getEditLogSnapshot()).toHaveLength(1);
  expect(world.getEditLogSnapshot()[0]?.previousMaterial).toBe(previous);
});

test("dirty remesh neighbors retain their existing mesh until rebuilt", () => {
  const world = new ProceduralResidentWorld(new ProceduralWorldGenerator(1337, { chunkSize: 16 }), {
    horizontalRadiusChunks: 2,
  });
  const spawn = world.getSpawnPosition();

  // Generate all chunks and mesh them
  world.updateResidencyAround(spawn, { maxGenerateChunks: Number.POSITIVE_INFINITY });
  rebuildDirtyMeshes(world, Number.POSITIVE_INFINITY);

  // Find a meshed chunk that has at least one adjacent resident chunk
  let targetChunk: import("../src/engine/world.ts").VoxelChunk | null = null;
  for (const chunk of world.iterateResidentChunks()) {
    if (chunk.lodLevel !== 0 || !chunk.meshBuilt || !chunk.mesh) continue;
    targetChunk = chunk;
    break;
  }
  expect(targetChunk).not.toBeNull();
  expect(targetChunk!.meshDirty).toBe(false);

  // Edit a voxel in this chunk to make it dirty, then verify mesh is retained
  const wx = targetChunk!.coord.x * world.chunkSize;
  const wy = targetChunk!.coord.y * world.chunkSize;
  const wz = targetChunk!.coord.z * world.chunkSize;
  // Find a solid voxel to edit
  let editY = wy;
  for (let y = wy; y < wy + world.chunkSize; y++) {
    if (world.getVoxel(wx, y, wz) !== 0) { editY = y; break; }
  }
  world.setVoxel(wx, editY, wz, 0);

  // The chunk should now be dirty but still retain its old mesh
  expect(targetChunk!.meshDirty).toBe(true);
  expect(targetChunk!.meshBuilt).toBe(true);
  expect(targetChunk!.mesh).not.toBeNull();
});
