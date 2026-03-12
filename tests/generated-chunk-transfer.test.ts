import { expect, test } from "bun:test";

import {
  deserializeGeneratedChunkRenderSummary,
  deserializeGeneratedRenderColumnSummary,
  deserializeGeneratedRenderSummaryRegion,
  serializeGeneratedChunkRenderSummary,
  serializeGeneratedRenderColumnSummary,
  serializeGeneratedRenderSummaryRegion,
} from "../src/engine/generated-chunk-transfer.ts";
import { summarizeGeneratedChunkRender } from "../src/engine/generated-chunk-render-summary.ts";
import { summarizeGeneratedRenderColumn } from "../src/engine/generated-render-column-summary.ts";
import { upsertGeneratedRenderSummaryRegion } from "../src/engine/generated-render-summary-region.ts";
import { isProceduralWaterMaterial } from "../src/engine/procedural-generator.ts";

test("generated chunk render summary transfer round-trips typed summary payloads", () => {
  const chunkSize = 32;
  const data = new Uint16Array(chunkSize ** 3);
  data.fill(7);
  data[0] = 0;
  const summary = summarizeGeneratedChunkRender({ x: 2, y: 3, z: 4 }, data, chunkSize, isProceduralWaterMaterial);

  const transferred = serializeGeneratedChunkRenderSummary(summary);
  const restored = deserializeGeneratedChunkRenderSummary(transferred.summary);

  expect(restored.coord).toEqual(summary.coord);
  expect(restored.coveredColumnCount).toBe(summary.coveredColumnCount);
  expect(restored.macroCellSize).toBe(summary.macroCellSize);
  expect(restored.macroCellsPerAxis).toBe(summary.macroCellsPerAxis);
  expect(Array.from(restored.macroCellStates)).toEqual(Array.from(summary.macroCellStates));
  expect(Array.from(restored.faceOpenMask)).toEqual(Array.from(summary.faceOpenMask));
});

test("generated render column summary transfer round-trips typed summary payloads", () => {
  const chunkSize = 32;
  const data = new Uint16Array(chunkSize ** 3);
  data[3 + 5 * chunkSize + 6 * chunkSize * chunkSize] = 91;
  const chunkSummary = summarizeGeneratedChunkRender({ x: 4, y: 8, z: 9 }, data, chunkSize, isProceduralWaterMaterial);
  const summary = summarizeGeneratedRenderColumn(4, 9, [chunkSummary], chunkSize);

  expect(summary).not.toBeNull();
  const transferred = serializeGeneratedRenderColumnSummary(summary!);
  const restored = deserializeGeneratedRenderColumnSummary(transferred.summary);

  expect(restored.chunkX).toBe(summary!.chunkX);
  expect(restored.chunkZ).toBe(summary!.chunkZ);
  expect(restored.coveredColumnCount).toBe(summary!.coveredColumnCount);
  expect(Array.from(restored.surfaceY)).toEqual(Array.from(summary!.surfaceY));
  expect(Array.from(restored.surfaceMaterial)).toEqual(Array.from(summary!.surfaceMaterial));
  expect(restored.minKnownCy).toBe(summary!.minKnownCy);
  expect(restored.maxKnownCy).toBe(summary!.maxKnownCy);
});

test("generated render summary region transfer round-trips nested column summaries", () => {
  const chunkSize = 32;
  const lowerData = new Uint16Array(chunkSize ** 3);
  lowerData[2 + 4 * chunkSize + 5 * chunkSize * chunkSize] = 15;
  const upperData = new Uint16Array(chunkSize ** 3);
  upperData[7 + 10 * chunkSize + 11 * chunkSize * chunkSize] = 29;
  const lowerChunkSummary = summarizeGeneratedChunkRender({ x: -3, y: 1, z: 6 }, lowerData, chunkSize, isProceduralWaterMaterial);
  const upperChunkSummary = summarizeGeneratedChunkRender({ x: -2, y: 2, z: 6 }, upperData, chunkSize, isProceduralWaterMaterial);
  const lowerColumnSummary = summarizeGeneratedRenderColumn(-3, 6, [lowerChunkSummary], chunkSize);
  const upperColumnSummary = summarizeGeneratedRenderColumn(-2, 6, [upperChunkSummary], chunkSize);

  expect(lowerColumnSummary).not.toBeNull();
  expect(upperColumnSummary).not.toBeNull();

  const region = upsertGeneratedRenderSummaryRegion(
    upsertGeneratedRenderSummaryRegion(null, lowerColumnSummary!),
    upperColumnSummary!,
  );
  const transferred = serializeGeneratedRenderSummaryRegion(region);
  const restored = deserializeGeneratedRenderSummaryRegion(transferred.summary);

  expect(restored.regionX).toBe(region.regionX);
  expect(restored.regionZ).toBe(region.regionZ);
  expect(restored.regionSizeChunks).toBe(region.regionSizeChunks);
  expect(restored.columns).toHaveLength(2);
  expect(restored.columns.map((entry) => [entry.chunkX, entry.chunkZ])).toEqual(
    region.columns.map((entry) => [entry.chunkX, entry.chunkZ]),
  );
  expect(Array.from(restored.columns[0]!.summary.surfaceY)).toEqual(Array.from(region.columns[0]!.summary.surfaceY));
  expect(Array.from(restored.columns[1]!.summary.surfaceMaterial)).toEqual(
    Array.from(region.columns[1]!.summary.surfaceMaterial),
  );
});
