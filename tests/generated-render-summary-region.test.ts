import { expect, test } from "bun:test";

import { summarizeGeneratedChunkRender } from "../src/engine/generated-chunk-render-summary.ts";
import { summarizeGeneratedRenderColumn } from "../src/engine/generated-render-column-summary.ts";
import {
  GENERATED_RENDER_SUMMARY_REGION_SIZE_CHUNKS,
  getGeneratedRenderSummaryRegionCoord,
  upsertGeneratedRenderSummaryRegion,
} from "../src/engine/generated-render-summary-region.ts";
import { isProceduralWaterMaterial } from "../src/engine/procedural-generator.ts";

test("render summary regions bucket chunk columns by fixed-size region coordinates", () => {
  expect(getGeneratedRenderSummaryRegionCoord(0, 0)).toEqual({ x: 0, z: 0 });
  expect(getGeneratedRenderSummaryRegionCoord(3, 3)).toEqual({ x: 0, z: 0 });
  expect(getGeneratedRenderSummaryRegionCoord(4, 0)).toEqual({ x: 1, z: 0 });
  expect(getGeneratedRenderSummaryRegionCoord(-1, -1)).toEqual({ x: -1, z: -1 });
  expect(getGeneratedRenderSummaryRegionCoord(-4, -4)).toEqual({ x: -1, z: -1 });
});

test("upserting a render summary region replaces an existing column summary in place", () => {
  const chunkSize = 32;
  const initialData = new Uint16Array(chunkSize ** 3);
  initialData[1 + 3 * chunkSize + 5 * chunkSize * chunkSize] = 7;
  const replacementData = new Uint16Array(chunkSize ** 3);
  replacementData[1 + 9 * chunkSize + 5 * chunkSize * chunkSize] = 13;
  const initialChunkSummary = summarizeGeneratedChunkRender({ x: 2, y: 1, z: -3 }, initialData, chunkSize, isProceduralWaterMaterial);
  const replacementChunkSummary = summarizeGeneratedChunkRender(
    { x: 2, y: 2, z: -3 },
    replacementData,
    chunkSize,
    isProceduralWaterMaterial,
  );
  const initialColumnSummary = summarizeGeneratedRenderColumn(2, -3, [initialChunkSummary], chunkSize);
  const replacementColumnSummary = summarizeGeneratedRenderColumn(2, -3, [replacementChunkSummary], chunkSize);

  expect(initialColumnSummary).not.toBeNull();
  expect(replacementColumnSummary).not.toBeNull();

  const once = upsertGeneratedRenderSummaryRegion(null, initialColumnSummary!);
  const twice = upsertGeneratedRenderSummaryRegion(once, replacementColumnSummary!);

  expect(twice.regionSizeChunks).toBe(GENERATED_RENDER_SUMMARY_REGION_SIZE_CHUNKS);
  expect(twice.columns).toHaveLength(1);
  expect(twice.columns[0]!.chunkX).toBe(2);
  expect(twice.columns[0]!.chunkZ).toBe(-3);
  expect(Array.from(twice.columns[0]!.summary.surfaceY)).toEqual(Array.from(replacementColumnSummary!.surfaceY));
});
