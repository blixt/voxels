import { expect, test } from "bun:test";

import {
  breakVoxelAlongRay,
  placeSelectedVoxelAlongRay,
} from "../src/engine/interaction-loop.ts";
import { createInventoryState, insertInventoryMaterial } from "../src/engine/inventory.ts";
import { VoxelWorld } from "../src/engine/world.ts";

test("breaking a voxel along a ray clears the world and inserts into inventory", () => {
  const world = new VoxelWorld({ width: 8, height: 8, depth: 8 }, 8, [0, 0xffaa8844]);
  world.setVoxel(1, 1, 1, 1);
  const inventory = createInventoryState();

  const result = breakVoxelAlongRay(world, inventory, [-1.5, 1.5, 1.5], [1, 0, 0], 16);

  expect(result.changed).toBe(true);
  expect(result.hit?.voxel).toEqual([1, 1, 1]);
  expect(world.getVoxel(1, 1, 1)).toBe(0);
  expect(inventory.slots[0]).toEqual({ material: 1, count: 1 });
});

test("breaking a voxel does nothing when the inventory cannot accept it", () => {
  const world = new VoxelWorld({ width: 8, height: 8, depth: 8 }, 8, [0, 0xffaa8844]);
  world.setVoxel(1, 1, 1, 1);
  const inventory = createInventoryState();
  for (let slot = 0; slot < inventory.slots.length; slot += 1) {
    inventory.slots[slot] = { material: slot + 1, count: 1024 };
  }

  const result = breakVoxelAlongRay(world, inventory, [-1.5, 1.5, 1.5], [1, 0, 0], 16);

  expect(result.changed).toBe(false);
  expect(world.getVoxel(1, 1, 1)).toBe(1);
});

test("placing voxels uses the adjacent empty cell sphere and consumes items", () => {
  const world = new VoxelWorld({ width: 8, height: 8, depth: 8 }, 8, [0, 0xffaa8844, 0xff44aa88]);
  world.setVoxel(1, 1, 1, 1);
  const inventory = createInventoryState();
  insertInventoryMaterial(inventory, 2, 1024);

  const result = placeSelectedVoxelAlongRay(world, inventory, [-1.5, 1.5, 1.5], [1, 0, 0], 16);

  expect(result.changed).toBe(true);
  // The hit adjacent is (0, 1, 1) — sphere centered there
  expect(world.getVoxel(0, 1, 1)).toBe(2);
  // Inventory should have consumed some items
  expect(inventory.slots[0]!.count).toBeLessThan(1024);
});
