import { expect, test } from "bun:test";

import { decodeGeneratedChunk, encodeGeneratedChunk } from "../src/engine/generated-chunk-codec.ts";
import { summarizeGeneratedChunkSurface } from "../src/engine/generated-chunk-surface-summary.ts";
import type { GeneratedChunk } from "../src/engine/procedural-generator.ts";
import { isProceduralWaterMaterial, ProceduralWorldGenerator } from "../src/engine/procedural-generator.ts";

test("generated chunk codec round-trips procedural chunk data", () => {
  const generator = new ProceduralWorldGenerator(1337, { chunkSize: 16 });
  const chunk = generator.generateChunk(4, 2, -3);

  const encoded = encodeGeneratedChunk(chunk);
  const decoded = decodeGeneratedChunk(encoded.buffer);

  expect(decoded.coord).toEqual(chunk.coord);
  expect(decoded.solidCount).toBe(chunk.solidCount);
  expect(decoded.solidBounds).toEqual(chunk.solidBounds);
  expect(decoded.surfaceSummary?.coveredColumnCount).toBe(chunk.surfaceSummary?.coveredColumnCount ?? 0);
  expect(Array.from(decoded.data)).toEqual(Array.from(chunk.data));
});

test("generated chunk codec compresses empty chunks to a tiny payload", () => {
  const chunk = createUniformChunk(32, { x: 0, y: 999, z: 0 }, 0);

  const encoded = encodeGeneratedChunk(chunk);

  expect(encoded.stats.emptySubchunkCount).toBe(64);
  expect(encoded.stats.byteLength).toBeLessThan(256);
});

test("generated chunk codec compresses uniform chunks well", () => {
  const chunk = createUniformChunk(32, { x: 1, y: 2, z: 3 }, 77);

  const encoded = encodeGeneratedChunk(chunk);
  const decoded = decodeGeneratedChunk(encoded.buffer);

  expect(encoded.stats.uniformSubchunkCount).toBe(64);
  expect(encoded.stats.byteLength).toBeLessThan(chunk.data.byteLength / 4);
  expect(Array.from(decoded.data)).toEqual(Array.from(chunk.data));
  expect(decoded.surfaceSummary?.coveredColumnCount).toBe(chunk.surfaceSummary?.coveredColumnCount ?? 0);
});

function createUniformChunk(chunkSize: number, coord: { x: number; y: number; z: number }, material: number): GeneratedChunk {
  const length = chunkSize * chunkSize * chunkSize;
  const data = new Uint16Array(length);
  data.fill(material);
  return {
    coord,
    data,
    solidCount: material === 0 ? 0 : length,
    solidBounds: material === 0
      ? null
      : {
          min: [0, 0, 0],
          max: [chunkSize, chunkSize, chunkSize],
        },
    surfaceSummary: summarizeGeneratedChunkSurface(coord, data, chunkSize, isProceduralWaterMaterial),
  };
}
