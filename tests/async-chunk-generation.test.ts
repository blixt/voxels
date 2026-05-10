import { expect, test } from "bun:test";

import { toAsyncLodChunkKey } from "../src/engine/async-chunk-generation.ts";

test("derived LOD cache keys include edit revision, level, and full coord", () => {
  expect(toAsyncLodChunkKey({
    editRevision: 7,
    lodLevel: 3,
    coord: { x: -11, y: 5, z: 19 },
  })).toBe("lod:7:3:-11:5:19");

  expect(toAsyncLodChunkKey({
    editRevision: 8,
    lodLevel: 3,
    coord: { x: -11, y: 5, z: 19 },
  })).not.toBe(toAsyncLodChunkKey({
    editRevision: 7,
    lodLevel: 3,
    coord: { x: -11, y: 5, z: 19 },
  }));
});
