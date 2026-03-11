import { expect, test } from "bun:test";

import {
  diffChunkCoords,
  summarizeGeneratedChunk,
  summarizeResidentWorld,
} from "../src/engine/procedural-probes.ts";
import { ProceduralWorldGenerator } from "../src/engine/procedural-generator.ts";
import { ProceduralResidentWorld } from "../src/engine/procedural-resident-world.ts";

test("generated chunk probe is deterministic and reports world-space bounds", () => {
  const generator = new ProceduralWorldGenerator(4242, { chunkSize: 16 });
  const first = summarizeGeneratedChunk(generator, generator.generateChunk(2, 90, -3));
  const second = summarizeGeneratedChunk(generator, generator.generateChunk(2, 90, -3));

  expect(first.checksum).toBe(second.checksum);
  expect(first.solidCount).toBe(second.solidCount);
  expect(first.centerColumn).toEqual(generator.sampleColumn(40, -40));
  if (first.solidBounds) {
    expect(first.solidBounds.min[0]).toBeGreaterThanOrEqual(32);
    expect(first.solidBounds.max[0]).toBeLessThanOrEqual(48);
    expect(first.solidBounds.min[1]).toBeGreaterThanOrEqual(90 * 16);
    expect(first.solidBounds.max[1]).toBeLessThanOrEqual((90 + 1) * 16);
    expect(first.solidBounds.min[2]).toBeGreaterThanOrEqual(-48);
    expect(first.solidBounds.max[2]).toBeLessThanOrEqual(-32);
  }
});

test("resident world probe snapshot is sorted and matches world stats", () => {
  const world = new ProceduralResidentWorld(new ProceduralWorldGenerator(7, { chunkSize: 16 }), {
    horizontalRadiusChunks: 1,
  });
  world.updateResidencyAround([0, 1400, 0]);

  const snapshot = summarizeResidentWorld(world);
  const keys = snapshot.chunks.map((chunk) => [chunk.coord.x, chunk.coord.y, chunk.coord.z]);
  const sortedKeys = [...keys].sort((a, b) => a[0] - b[0] || a[1] - b[1] || a[2] - b[2]);

  expect(snapshot.chunkCount).toBe(world.getStats().chunkCount);
  expect(snapshot.solidVoxelCount).toBe(world.getStats().solidVoxelCount);
  expect(keys).toEqual(sortedKeys);
});

test("diffChunkCoords reports deterministic entered and evicted sets", () => {
  const diff = diffChunkCoords(
    [
      { x: 0, y: 10, z: 0 },
      { x: 1, y: 10, z: 0 },
      { x: -1, y: 10, z: 0 },
    ],
    [
      { x: -1, y: 10, z: 0 },
      { x: 2, y: 10, z: 0 },
      { x: 1, y: 11, z: 0 },
    ],
  );

  expect(diff.entered).toEqual([
    { x: 1, y: 11, z: 0 },
    { x: 2, y: 10, z: 0 },
  ]);
  expect(diff.evicted).toEqual([
    { x: 0, y: 10, z: 0 },
    { x: 1, y: 10, z: 0 },
  ]);
});
