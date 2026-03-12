import { expect, test } from "bun:test";

import { summarizeGeneratedChunkRender } from "../src/engine/generated-chunk-render-summary.ts";
import {
  mergeGeneratedRenderColumnSummary,
  sampleGeneratedRenderColumnSummary,
  summarizeGeneratedRenderColumn,
} from "../src/engine/generated-render-column-summary.ts";
import { isProceduralWaterMaterial } from "../src/engine/procedural-generator.ts";

test("generated render column summary aggregates the top visible voxel across chunk slabs", () => {
  const chunkSize = 32;
  const lower = new Uint16Array(chunkSize ** 3);
  const upper = new Uint16Array(chunkSize ** 3);
  lower[3 + 7 * chunkSize + 5 * chunkSize * chunkSize] = 11;
  upper[3 + 2 * chunkSize + 5 * chunkSize * chunkSize] = 22;

  const summary = summarizeGeneratedRenderColumn(4, -2, [
    summarizeGeneratedChunkRender({ x: 4, y: 10, z: -2 }, lower, chunkSize, isProceduralWaterMaterial),
    summarizeGeneratedChunkRender({ x: 4, y: 11, z: -2 }, upper, chunkSize, isProceduralWaterMaterial),
  ], chunkSize);

  expect(summary).not.toBeNull();
  const sample = sampleGeneratedRenderColumnSummary(summary!, 3, 5, chunkSize);
  expect(sample).not.toBeNull();
  expect(sample!.surfaceMaterial).toBe(22);
  expect(summary!.minKnownCy).toBe(10);
  expect(summary!.maxKnownCy).toBe(11);
  expect(summary!.minNonEmptyCy).toBe(10);
  expect(summary!.maxNonEmptyCy).toBe(11);
});

test("generated render column summary tracks empty known ranges separately from non-empty ranges", () => {
  const chunkSize = 32;
  const empty = summarizeGeneratedChunkRender(
    { x: 1, y: 4, z: 2 },
    new Uint16Array(chunkSize ** 3),
    chunkSize,
    isProceduralWaterMaterial,
  );
  const solidData = new Uint16Array(chunkSize ** 3);
  solidData[0] = 7;
  const solid = summarizeGeneratedChunkRender({ x: 1, y: 5, z: 2 }, solidData, chunkSize, isProceduralWaterMaterial);

  const summary = summarizeGeneratedRenderColumn(1, 2, [empty, solid], chunkSize);

  expect(summary).not.toBeNull();
  expect(summary!.minKnownCy).toBe(4);
  expect(summary!.maxKnownCy).toBe(5);
  expect(summary!.minNonEmptyCy).toBe(5);
  expect(summary!.maxNonEmptyCy).toBe(5);
});

test("generated render column summary can be merged incrementally from chunk summaries", () => {
  const chunkSize = 32;
  const lower = new Uint16Array(chunkSize ** 3);
  const upper = new Uint16Array(chunkSize ** 3);
  lower[4 + 6 * chunkSize + 3 * chunkSize * chunkSize] = 123;
  upper[4 + 8 * chunkSize + 3 * chunkSize * chunkSize] = 456;

  const lowerSummary = summarizeGeneratedChunkRender({ x: 7, y: 12, z: -5 }, lower, chunkSize, isProceduralWaterMaterial);
  const upperSummary = summarizeGeneratedChunkRender({ x: 7, y: 13, z: -5 }, upper, chunkSize, isProceduralWaterMaterial);
  const merged = mergeGeneratedRenderColumnSummary(
    mergeGeneratedRenderColumnSummary(null, lowerSummary, chunkSize),
    upperSummary,
    chunkSize,
  );
  const recomputed = summarizeGeneratedRenderColumn(7, -5, [lowerSummary, upperSummary], chunkSize);

  expect(merged.coveredColumnCount).toBe(recomputed!.coveredColumnCount);
  expect(Array.from(merged.surfaceY)).toEqual(Array.from(recomputed!.surfaceY));
  expect(Array.from(merged.surfaceMaterial)).toEqual(Array.from(recomputed!.surfaceMaterial));
  expect(merged.minKnownCy).toBe(recomputed!.minKnownCy);
  expect(merged.maxKnownCy).toBe(recomputed!.maxKnownCy);
  expect(merged.minNonEmptyCy).toBe(recomputed!.minNonEmptyCy);
  expect(merged.maxNonEmptyCy).toBe(recomputed!.maxNonEmptyCy);
});
