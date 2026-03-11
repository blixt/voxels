import { expect, test } from "bun:test";

import {
  shouldAllowFarFieldCatchupWhileMoving,
  shouldPumpWorldWork,
  shouldRefreshResidency,
} from "../src/engine/stream-work.ts";

test("residency refresh continues on the same anchor while chunks are still pending", () => {
  expect(shouldRefreshResidency(false, false, 1)).toBe(true);
  expect(shouldRefreshResidency(false, false, 0)).toBe(false);
  expect(shouldRefreshResidency(false, true, 0)).toBe(true);
  expect(shouldRefreshResidency(true, false, 0)).toBe(true);
});

test("background world work keeps pumping while dirty meshes or pending chunks remain", () => {
  expect(shouldPumpWorldWork(false, 0, 0)).toBe(false);
  expect(shouldPumpWorldWork(true, 0, 0)).toBe(true);
  expect(shouldPumpWorldWork(false, 2, 0)).toBe(true);
  expect(shouldPumpWorldWork(false, 0, 3)).toBe(true);
  expect(shouldPumpWorldWork(false, 0, 0, 1)).toBe(true);
});

test("far-field catch-up while moving is throttled to quiet frames on a cadence", () => {
  expect(shouldAllowFarFieldCatchupWhileMoving(false, 0, 0, 5, 1)).toBe(true);
  expect(shouldAllowFarFieldCatchupWhileMoving(true, 2, 0, 5, 6)).toBe(false);
  expect(shouldAllowFarFieldCatchupWhileMoving(true, 0, 3, 5, 6)).toBe(false);
  expect(shouldAllowFarFieldCatchupWhileMoving(true, 0, 0, 0, 6)).toBe(false);
  expect(shouldAllowFarFieldCatchupWhileMoving(true, 0, 0, 5, 5)).toBe(false);
  expect(shouldAllowFarFieldCatchupWhileMoving(true, 0, 0, 5, 6)).toBe(true);
});
