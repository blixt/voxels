import { expect, test } from "bun:test";

import { shouldPumpWorldWork, shouldRefreshResidency } from "../src/engine/stream-work.ts";

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
});
