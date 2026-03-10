import { expect, test } from "bun:test";

import { rebuildDirtyMeshes } from "../src/engine/mesher.ts";
import { VoxelWorld } from "../src/engine/world.ts";

test("fillColumn spans chunk boundaries and updates counts", () => {
  const world = new VoxelWorld({ width: 8, height: 64, depth: 8 }, 32, [0, 0xffffffff]);
  world.fillColumn(3, 4, 0, 40, 1);

  expect(world.getVoxel(3, 0, 4)).toBe(1);
  expect(world.getVoxel(3, 31, 4)).toBe(1);
  expect(world.getVoxel(3, 32, 4)).toBe(1);
  expect(world.getVoxel(3, 39, 4)).toBe(1);
  expect(world.getVoxel(3, 40, 4)).toBe(0);
  expect(world.getStats().solidVoxelCount).toBe(40);
  expect(world.chunks.size).toBe(2);
});

test("fillColumn marks adjacent chunks dirty when a boundary voxel changes", () => {
  const world = new VoxelWorld({ width: 64, height: 32, depth: 32 }, 32, [0, 0xffffffff]);
  world.fillColumn(31, 10, 10, 11, 1);
  world.setVoxel(32, 10, 10, 1);
  rebuildDirtyMeshes(world);

  const rightKey = world.resolveChunkKey(1, 0, 0);
  expect(rightKey).not.toBeNull();
  const rightChunk = world.chunks.get(rightKey!);
  expect(rightChunk?.meshDirty).toBe(false);

  world.fillColumn(31, 10, 10, 11, 0);

  expect(world.chunks.get(rightKey!)?.meshDirty).toBe(true);
});
