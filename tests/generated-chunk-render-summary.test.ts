import { expect, test } from "bun:test";

import {
  GENERATED_CHUNK_RENDER_CELL_EMPTY,
  GENERATED_CHUNK_RENDER_CELL_MIXED,
  GENERATED_CHUNK_RENDER_CELL_SOLID,
  isGeneratedChunkFaceOpen,
  summarizeGeneratedChunkRender,
} from "../src/engine/generated-chunk-render-summary.ts";
import { isProceduralWaterMaterial } from "../src/engine/procedural-generator.ts";

test("generated chunk render summary retains empty chunks for future volumetric far rendering", () => {
  const chunkSize = 32;
  const summary = summarizeGeneratedChunkRender({ x: 0, y: 0, z: 0 }, new Uint16Array(chunkSize ** 3), chunkSize, isProceduralWaterMaterial);

  expect(summary.coveredColumnCount).toBe(0);
  expect(summary.surfaceY.length).toBe(0);
  expect(summary.waterTopY.length).toBe(0);
  expect(summary.macroCellStates.every((state) => state === GENERATED_CHUNK_RENDER_CELL_EMPTY)).toBe(true);
  expect(isGeneratedChunkFaceOpen(summary, "west", 0, 0)).toBe(true);
  expect(isGeneratedChunkFaceOpen(summary, "up", 0, 0)).toBe(true);
});

test("generated chunk render summary classifies macro cells as solid, mixed, or empty", () => {
  const chunkSize = 32;
  const data = new Uint16Array(chunkSize ** 3);
  data.fill(17);
  data[0] = 0;
  const summary = summarizeGeneratedChunkRender({ x: 0, y: 0, z: 0 }, data, chunkSize, isProceduralWaterMaterial);

  expect(summary.macroCellStates[0]).toBe(GENERATED_CHUNK_RENDER_CELL_MIXED);
  expect(summary.macroCellStates[1]).toBe(GENERATED_CHUNK_RENDER_CELL_SOLID);
  expect(summary.macroCellStates[summary.macroCellStates.length - 1]).toBe(GENERATED_CHUNK_RENDER_CELL_SOLID);
});
