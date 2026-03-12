import { expect, test } from "bun:test";

import type { AsyncChunkGenerationQueue } from "../src/engine/async-chunk-generation.ts";
import { rebuildDirtyMeshes } from "../src/engine/mesher.ts";
import { ProceduralResidentWorld } from "../src/engine/procedural-resident-world.ts";
import { ProceduralWorldGenerator } from "../src/engine/procedural-generator.ts";
import { summarizeGeneratedRenderColumn } from "../src/engine/generated-render-column-summary.ts";
import { upsertGeneratedRenderSummaryRegion } from "../src/engine/generated-render-summary-region.ts";
import { metersToWorldUnits } from "../src/engine/scale.ts";

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
    hasPendingChunk: () => false,
    hasPendingRegionSummary: () => false,
    getPendingCount: () => 0,
    drainCompletedChunks: () => [],
    drainCompletedSummaries: () => [],
    drainCompletedRegionSummaries: () => [],
    drainMissingRegionSummaries: () => [],
    drainCompletionStats: () => ({ cacheHits: 0, generated: 0 }),
    drainSummaryCompletionStats: () => ({ cacheHits: 0, generated: 0 }),
    dispose: () => {},
    ...overrides,
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
  expect(spawn[1]).toBe(Math.max(...heights) + 1);
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
  const targetY = generator.sampleColumn(targetX, targetZ).surfaceY;
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

test("render-ready far-field mask only excludes columns after their meshes are built", () => {
  const world = new ProceduralResidentWorld(new ProceduralWorldGenerator(1337, { chunkSize: 16 }), {
    horizontalRadiusChunks: 1,
  });
  const spawn = world.getSpawnPosition();
  const centerChunkX = Math.floor(spawn[0] / world.chunkSize);
  const centerChunkZ = Math.floor(spawn[2] / world.chunkSize);

  world.updateResidencyAround(spawn, { maxGenerateChunks: 1 });

  const residentMask = world.getFarFieldExclusionMask("resident", 1);
  const renderReadyBefore = world.getFarFieldExclusionMask("render-ready", 1);
  expect(residentMask.excludesCell(
    centerChunkX * world.chunkSize,
    centerChunkX * world.chunkSize + 1,
    centerChunkZ * world.chunkSize,
    centerChunkZ * world.chunkSize + 1,
  )).toBe(true);
  expect(renderReadyBefore.excludesCell(
    centerChunkX * world.chunkSize,
    centerChunkX * world.chunkSize + 1,
    centerChunkZ * world.chunkSize,
    centerChunkZ * world.chunkSize + 1,
  )).toBe(false);

  rebuildDirtyMeshes(world, Number.POSITIVE_INFINITY);

  const renderReadyAfter = world.getFarFieldExclusionMask("render-ready", 2);
  expect(renderReadyAfter.excludesCell(
    centerChunkX * world.chunkSize,
    centerChunkX * world.chunkSize + 1,
    centerChunkZ * world.chunkSize,
    centerChunkZ * world.chunkSize + 1,
  )).toBe(true);
});

test("dirty remesh neighbors retain their existing mesh until rebuilt", () => {
  const world = new ProceduralResidentWorld(new ProceduralWorldGenerator(1337, { chunkSize: 16 }), {
    horizontalRadiusChunks: 1,
  });
  const spawn = world.getSpawnPosition();
  const centerChunkX = Math.floor(spawn[0] / world.chunkSize);
  const centerChunkZ = Math.floor(spawn[2] / world.chunkSize);
  const supportChunkY = Math.floor((spawn[1] - 1) / world.chunkSize);

  world.updateResidencyAround(spawn, { maxGenerateChunks: 1 });
  rebuildDirtyMeshes(world, Number.POSITIVE_INFINITY);
  const supportChunk = world.getResidentChunk(centerChunkX, supportChunkY, centerChunkZ);
  expect(supportChunk?.meshBuilt).toBe(true);
  expect(supportChunk?.mesh).not.toBeNull();

  world.updateResidencyAround(spawn, { maxGenerateChunks: 1 });

  const dirtySupportChunk = world.getResidentChunk(centerChunkX, supportChunkY, centerChunkZ);
  expect(dirtySupportChunk?.meshDirty).toBe(true);
  expect(dirtySupportChunk?.meshBuilt).toBe(true);
  expect(dirtySupportChunk?.mesh).not.toBeNull();
});

test("far-field sampling only becomes available after actual chunk pre-generation", () => {
  const world = new ProceduralResidentWorld(new ProceduralWorldGenerator(1337, { chunkSize: 16 }), {
    horizontalRadiusChunks: 1,
  });
  const spawn = world.getSpawnPosition();
  const farWorldX = Math.floor(spawn[0] + world.chunkSize * 2);
  const farWorldZ = Math.floor(spawn[2]);
  world.updateResidencyAround(spawn, { maxGenerateChunks: Number.POSITIVE_INFINITY });

  expect(world.sampleFarFieldColumn(farWorldX, farWorldZ)).toBeNull();

  const generated = world.prefetchFarFieldSummariesAround(spawn, world.chunkSize * 6, 64);

  expect(generated).toBeGreaterThan(0);
  expect(world.sampleFarFieldColumn(farWorldX, farWorldZ)).not.toBeNull();
});

test("far-field summary prefetch grows from actual summary frontier instead of probing generator columns", () => {
  const generator = new CountingProceduralWorldGenerator(1337, { chunkSize: 16 });
  const world = new ProceduralResidentWorld(generator, {
    horizontalRadiusChunks: 1,
  });
  const spawn = world.getSpawnPosition();
  world.updateResidencyAround(spawn, { maxGenerateChunks: Number.POSITIVE_INFINITY });

  generator.sampleColumnCalls = 0;
  const generated = world.prefetchFarFieldSummariesAround(spawn, world.chunkSize * 6, 64);

  expect(generated).toBeGreaterThan(0);
  expect(generator.sampleColumnCalls).toBe(0);
});

test("far-field summary prefetch requests persisted summary regions before any live frontier exists", () => {
  const generator = new CountingProceduralWorldGenerator(1337, { chunkSize: 16 });
  const requestedRegions: Array<[number, number]> = [];
  const world = new ProceduralResidentWorld(generator, {
    horizontalRadiusChunks: 1,
    asyncChunkGeneration: createFakeAsyncQueue({
      requestRegionSummary(regionX, regionZ) {
        requestedRegions.push([regionX, regionZ]);
        return true;
      },
    }),
  });
  const spawn = world.getSpawnPosition();
  generator.sampleColumnCalls = 0;

  const generated = world.prefetchFarFieldSummariesAround(spawn, world.chunkSize * 6, 64);

  expect(generated).toBe(0);
  expect(requestedRegions.length).toBeGreaterThan(0);
  expect(generator.sampleColumnCalls).toBe(0);
});

test("resident world adopts completed persisted region summaries for far-field sampling", () => {
  const generator = new ProceduralWorldGenerator(1337, { chunkSize: 16 });
  const farChunkX = 4;
  const farChunkZ = -3;
  const farChunkY = 0;
  const generated = generator.generateChunk(farChunkX, farChunkY, farChunkZ);
  const columnSummary = summarizeGeneratedRenderColumn(farChunkX, farChunkZ, [generated.renderSummary], generator.chunkSize);

  expect(columnSummary).not.toBeNull();

  const world = new ProceduralResidentWorld(generator, {
    horizontalRadiusChunks: 1,
    asyncChunkGeneration: createFakeAsyncQueue({
      drainCompletedRegionSummaries: () => [upsertGeneratedRenderSummaryRegion(null, columnSummary!)],
    }),
  });

  world.updateResidencyAround(world.getSpawnPosition(), { maxGenerateChunks: 0 });

  const sampled = world.sampleFarFieldColumn(farChunkX * generator.chunkSize + 1, farChunkZ * generator.chunkSize + 1);
  expect(sampled).not.toBeNull();
  expect(sampled!.surfaceY).toBeGreaterThanOrEqual(0);
});

test("far-field sampling follows resident voxel edits instead of stale generator output", () => {
  const generator = new ProceduralWorldGenerator(1337, { chunkSize: 16 });
  const world = new ProceduralResidentWorld(generator, { horizontalRadiusChunks: 1 });
  const spawn = world.getSpawnPosition();
  world.updateResidencyAround(spawn, { maxGenerateChunks: Number.POSITIVE_INFINITY });

  const worldX = Math.floor(spawn[0]);
  const worldZ = Math.floor(spawn[2]);
  const sampledBefore = world.sampleFarFieldColumn(worldX, worldZ);
  expect(sampledBefore).not.toBeNull();
  const targetY = sampledBefore!.surfaceY;
  const replacementMaterial = 0x0abc;

  expect(world.setVoxel(worldX, targetY, worldZ, replacementMaterial)).toBe(true);

  const sampledAfter = world.sampleFarFieldColumn(worldX, worldZ);
  expect(sampledAfter).not.toBeNull();
  expect(sampledAfter!.surfaceMaterial).toBe(replacementMaterial);
});
