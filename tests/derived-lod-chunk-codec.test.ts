import { expect, test } from "bun:test";

import {
  decodeDerivedLodChunk,
  encodeDerivedLodChunk,
  type DerivedLodChunkPayload,
} from "../src/engine/derived-lod-chunk-codec.ts";

test("derived LOD chunk codec round-trips sparse shell data", () => {
  const chunk = createSparseLodChunk();

  const encoded = encodeDerivedLodChunk(chunk);
  const decoded = decodeDerivedLodChunk(encoded.buffer);

  expect(decoded.coord).toEqual(chunk.coord);
  expect(decoded.lodLevel).toBe(chunk.lodLevel);
  expect(decoded.voxelStride).toBe(chunk.voxelStride);
  expect(decoded.solidCount).toBe(chunk.solidCount);
  expect(decoded.solidBounds).toEqual(chunk.solidBounds);
  expect(Array.from(decoded.data)).toEqual(Array.from(chunk.data));
  expect(encoded.stats.runCount).toBeLessThan(chunk.data.length / 4);
});

test("derived LOD chunk codec compresses empty chunks to a tiny payload", () => {
  const data = new Uint16Array(16 * 16 * 16);
  const chunk: DerivedLodChunkPayload = {
    coord: { x: 0, y: 999, z: 0 },
    lodLevel: 4,
    voxelStride: 16,
    data,
    solidCount: 0,
    solidBounds: null,
  };

  const encoded = encodeDerivedLodChunk(chunk);
  const decoded = decodeDerivedLodChunk(encoded.buffer);

  expect(encoded.stats.runCount).toBe(1);
  expect(encoded.stats.zeroRunCount).toBe(1);
  expect(encoded.stats.byteLength).toBeLessThan(64);
  expect(decoded.solidCount).toBe(0);
  expect(decoded.solidBounds).toBeNull();
  expect(decoded.data.every((material) => material === 0)).toBe(true);
});

test("derived LOD chunk codec rejects non-cubic payloads", () => {
  const chunk: DerivedLodChunkPayload = {
    coord: { x: 1, y: 2, z: 3 },
    lodLevel: 1,
    voxelStride: 2,
    data: new Uint16Array(17),
    solidCount: 0,
    solidBounds: null,
  };

  expect(() => encodeDerivedLodChunk(chunk)).toThrow("Expected cubic derived LOD chunk data");
});

function createSparseLodChunk(): DerivedLodChunkPayload {
  const chunkSize = 16;
  const data = new Uint16Array(chunkSize * chunkSize * chunkSize);
  const chunkArea = chunkSize * chunkSize;
  let solidCount = 0;
  for (let z = 0; z < chunkSize; z += 1) {
    for (let x = 0; x < chunkSize; x += 1) {
      const y = 4 + ((x + z) % 3);
      const material = (x + z) % 5 === 0 ? 12 : 7;
      data[x + y * chunkSize + z * chunkArea] = material;
      solidCount += 1;
    }
  }
  return {
    coord: { x: -11, y: 3, z: 19 },
    lodLevel: 3,
    voxelStride: 8,
    data,
    solidCount,
    solidBounds: {
      min: [0, 4, 0],
      max: [16, 7, 16],
    },
  };
}
