import { expect, test } from "bun:test";

import { decodeWorld, encodeWorld, hashWorld } from "../src/engine/scene-format.ts";
import { VoxelWorld } from "../src/engine/world.ts";

test("scene format roundtrips sparse worlds", () => {
  const world = new VoxelWorld({ width: 64, height: 64, depth: 64 }, 32, [0, 0xff443322, 0xff22aa44]);
  world.setVoxel(1, 2, 3, 1);
  world.setVoxel(33, 15, 7, 2);
  world.setVoxel(60, 40, 31, 1);

  const bytes = encodeWorld(world);
  const decoded = decodeWorld(bytes);

  expect(hashWorld(decoded)).toBe(hashWorld(world));
  expect(decoded.getVoxel(1, 2, 3)).toBe(1);
  expect(decoded.getVoxel(33, 15, 7)).toBe(2);
  expect(decoded.getVoxel(60, 40, 31)).toBe(1);
  expect(decoded.getStats()).toEqual(world.getStats());
});
