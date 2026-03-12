import { expect, test } from "bun:test";

import {
  deserializeGeneratedChunkRenderSummary,
  deserializeGeneratedRenderColumnSummary,
  serializeGeneratedChunkRenderSummary,
  serializeGeneratedRenderColumnSummary,
} from "../src/engine/generated-chunk-transfer.ts";
import { summarizeGeneratedChunkRender } from "../src/engine/generated-chunk-render-summary.ts";
import { summarizeGeneratedRenderColumn } from "../src/engine/generated-render-column-summary.ts";
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
