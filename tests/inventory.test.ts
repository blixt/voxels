import { expect, test } from "bun:test";

import {
  countUsedInventoryStacks,
  createInventoryState,
  getInventoryInsertCapacity,
  getSelectedInventoryStack,
  insertInventoryMaterial,
  removeSelectedInventoryMaterial,
  selectInventorySlot,
} from "../src/engine/inventory.ts";

test("inventory merges matching materials before opening new stacks", () => {
  const inventory = createInventoryState();

  expect(insertInventoryMaterial(inventory, 0x123, 900)).toEqual({ added: 900, leftover: 0 });
  expect(insertInventoryMaterial(inventory, 0x123, 200)).toEqual({ added: 200, leftover: 0 });

  expect(inventory.slots[0]).toEqual({ material: 0x123, count: 1024 });
  expect(inventory.slots[1]).toEqual({ material: 0x123, count: 76 });
  expect(countUsedInventoryStacks(inventory)).toBe(2);
});

test("inventory reports capacity and removes from the selected stack", () => {
  const inventory = createInventoryState();
  insertInventoryMaterial(inventory, 0x222, 10);
  selectInventorySlot(inventory, 0);

  expect(getInventoryInsertCapacity(inventory, 0x222)).toBe((32 * 1024) - 10);
  expect(removeSelectedInventoryMaterial(inventory, 4)).toEqual({ removed: 4, emptySlot: false });
  expect(getSelectedInventoryStack(inventory)).toEqual({ material: 0x222, count: 6 });
  expect(removeSelectedInventoryMaterial(inventory, 6)).toEqual({ removed: 6, emptySlot: true });
  expect(getSelectedInventoryStack(inventory)).toBeNull();
});
