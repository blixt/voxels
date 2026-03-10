import { expect, test } from "bun:test";

import { average, averageWarm, firstValue } from "../src/engine/benchmark-metrics.ts";

test("benchmark metric helpers split first and warm samples correctly", () => {
  expect(firstValue([4, 2, 1])).toBe(4);
  expect(average([4, 2, 1])).toBeCloseTo(7 / 3, 6);
  expect(averageWarm([4, 2, 1])).toBeCloseTo(1.5, 6);
});

test("benchmark metric helpers handle missing warm samples", () => {
  expect(firstValue<number>([])).toBeNull();
  expect(average([])).toBe(0);
  expect(averageWarm([])).toBeNull();
  expect(averageWarm([9])).toBeNull();
});
