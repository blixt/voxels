import { expect, test } from "bun:test";

import { measureImageDifference } from "../src/engine/reference-render.ts";

test("image diff metrics pass for identical buffers", () => {
  const pixels = new Uint8ClampedArray([
    209, 224, 240, 255,
    120, 100, 80, 255,
    140, 120, 100, 255,
    209, 224, 240, 255,
  ]);
  const metrics = measureImageDifference(
    { width: 2, height: 2, pixels },
    { width: 2, height: 2, pixels: new Uint8ClampedArray(pixels) },
  );

  expect(metrics.meanAbsoluteError).toBe(0);
  expect(metrics.coverageMismatchRatio).toBe(0);
  expect(metrics.visualPass).toBe(true);
});

test("image diff metrics fail on large coverage mismatch", () => {
  const actual = new Uint8ClampedArray([
    209, 224, 240, 255,
    209, 224, 240, 255,
    209, 224, 240, 255,
    209, 224, 240, 255,
  ]);
  const reference = new Uint8ClampedArray([
    209, 224, 240, 255,
    80, 60, 40, 255,
    80, 60, 40, 255,
    209, 224, 240, 255,
  ]);
  const metrics = measureImageDifference(
    { width: 2, height: 2, pixels: actual },
    { width: 2, height: 2, pixels: reference },
  );

  expect(metrics.coverageMismatchRatio).toBe(0.5);
  expect(metrics.visualPass).toBe(false);
});
