import { expect, test } from "bun:test";

import { VoxelWorld } from "../src/engine/world.ts";

test("resident chunk helpers expose populated chunk coordinates", () => {
  const world = new VoxelWorld({ width: 64, height: 32, depth: 32 }, 32, [0, 0xffffffff]);
  world.setVoxel(0, 0, 0, 1);
  world.setVoxel(33, 0, 0, 1);

  const coords = Array.from(world.iterateResidentChunks(), (chunk) => chunk.coord)
    .map((coord) => [coord.x, coord.y, coord.z]);

  expect(coords).toEqual([
    [0, 0, 0],
    [1, 0, 0],
  ]);
  expect(world.hasResidentChunk(0, 0, 0)).toBe(true);
  expect(world.hasResidentChunk(1, 0, 0)).toBe(true);
  expect(world.hasResidentChunk(2, 0, 0)).toBe(false);
  expect(world.getResidentChunk(1, 0, 0)?.solidCount).toBe(1);
});
