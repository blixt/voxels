import { expect, test } from "bun:test";

import { deriveLodChunkData } from "../src/engine/lod-chunk-derivation.ts";

test("LOD derivation uses source material before generated fallback when source is complete", () => {
  const result = deriveLodChunkData({
    chunkSize: 4,
    originX: 0,
    originY: 0,
    originZ: 0,
    level: 1,
    stride: 2,
    useGeneratedTopMaterial: false,
    isOutputColumnCoveredByFiner: () => false,
    sampleSurfaceYRange: () => ({ minY: 0, maxY: 7 }),
    sampleSourceMaterial: (_ox, oy) => ({ material: oy === 2 ? 9 : 0, complete: true }),
    sampleGeneratedTopMaterial: () => ({ bucketIndex: 1, material: 4 }),
    sampleGeneratedColumnMaterials: () => [0, 0, 5, 0],
  });

  expect(result.sourceComplete).toBe(true);
  expect(result.solidCount).toBe(16);
  expect(result.solidBounds).toEqual({ min: [0, 2, 0], max: [4, 3, 4] });
  expect(columnMaterial(result.data, 4, 0, 2, 0)).toBe(9);
});

test("LOD derivation falls back to generated material when source is incomplete", () => {
  const result = deriveLodChunkData({
    chunkSize: 4,
    originX: 0,
    originY: 0,
    originZ: 0,
    level: 1,
    stride: 2,
    useGeneratedTopMaterial: false,
    isOutputColumnCoveredByFiner: () => false,
    sampleSurfaceYRange: () => ({ minY: 0, maxY: 7 }),
    sampleSourceMaterial: () => ({ material: 0, complete: false }),
    sampleGeneratedTopMaterial: () => null,
    sampleGeneratedColumnMaterials: () => [0, 6, 0, 0],
  });

  expect(result.sourceComplete).toBe(false);
  expect(result.solidCount).toBe(16);
  expect(result.solidBounds).toEqual({ min: [0, 1, 0], max: [4, 2, 4] });
  expect(columnMaterial(result.data, 4, 1, 1, 3)).toBe(6);
});

test("LOD derivation skips columns owned by finer coverage", () => {
  const result = deriveLodChunkData({
    chunkSize: 4,
    originX: 0,
    originY: 0,
    originZ: 0,
    level: 2,
    stride: 4,
    useGeneratedTopMaterial: true,
    isOutputColumnCoveredByFiner: (ox) => ox < 2,
    sampleSurfaceYRange: () => ({ minY: 0, maxY: 15 }),
    sampleSourceMaterial: () => ({ material: 0, complete: true }),
    sampleGeneratedTopMaterial: () => ({ bucketIndex: 1, material: 11 }),
    sampleGeneratedColumnMaterials: () => [0, 0, 0, 0],
  });

  expect(result.skippedFinerCoverage).toBe(true);
  expect(result.sourceComplete).toBe(true);
  expect(result.solidCount).toBe(8);
  expect(columnMaterial(result.data, 4, 0, 1, 0)).toBe(0);
  expect(columnMaterial(result.data, 4, 2, 1, 0)).toBe(11);
});

function columnMaterial(data: Uint16Array, chunkSize: number, x: number, y: number, z: number): number {
  return data[x + y * chunkSize + z * chunkSize * chunkSize] ?? 0;
}
