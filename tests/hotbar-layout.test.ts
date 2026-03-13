import { expect, test } from "bun:test";

import { describeHotbarWindow } from "../src/engine/hotbar-layout.ts";

test("hotbar window centers the selected slot when possible", () => {
  expect(describeHotbarWindow(5, 32, 9)).toEqual({
    startSlot: 1,
    endSlotExclusive: 10,
    leftHiddenCount: 1,
    rightHiddenCount: 22,
  });
});

test("hotbar window clamps to the start and end edges", () => {
  expect(describeHotbarWindow(0, 32, 9)).toEqual({
    startSlot: 0,
    endSlotExclusive: 9,
    leftHiddenCount: 0,
    rightHiddenCount: 23,
  });
  expect(describeHotbarWindow(31, 32, 9)).toEqual({
    startSlot: 23,
    endSlotExclusive: 32,
    leftHiddenCount: 23,
    rightHiddenCount: 0,
  });
});

test("hotbar window handles small or empty inventories", () => {
  expect(describeHotbarWindow(0, 4, 9)).toEqual({
    startSlot: 0,
    endSlotExclusive: 4,
    leftHiddenCount: 0,
    rightHiddenCount: 0,
  });
  expect(describeHotbarWindow(0, 0, 9)).toEqual({
    startSlot: 0,
    endSlotExclusive: 0,
    leftHiddenCount: 0,
    rightHiddenCount: 0,
  });
});
