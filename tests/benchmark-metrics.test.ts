import { expect, test } from "bun:test";

import { average, averageWarm, firstValue, maxValue, percentile } from "../src/engine/benchmark-metrics.ts";

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
  expect(maxValue([])).toBe(0);
  expect(percentile([], 0.95)).toBe(0);
});

test("benchmark metric helpers summarize tails deterministically", () => {
  const values = [9, 1, 4, 7, 2];

  expect(maxValue(values)).toBe(9);
  expect(percentile(values, 0)).toBe(1);
  expect(percentile(values, 0.5)).toBe(4);
  expect(percentile(values, 0.95)).toBe(9);
  expect(percentile(values, 1)).toBe(9);
});
