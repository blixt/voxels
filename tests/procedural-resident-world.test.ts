import { expect, test } from "bun:test";

import { ProceduralResidentWorld } from "../src/engine/procedural-resident-world.ts";
import { ProceduralWorldGenerator } from "../src/engine/procedural-generator.ts";

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
