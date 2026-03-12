import { expect, test } from "bun:test";

import {
  deserializeGeneratedChunkRenderSummary,
  serializeGeneratedChunkRenderSummary,
} from "../src/engine/generated-chunk-transfer.ts";
import { summarizeGeneratedChunkRender } from "../src/engine/generated-chunk-render-summary.ts";
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
