import { expect, test } from "bun:test";

import {
  LOD_SUBVOXEL_CLIP_MASK_FULL,
  buildLodSubvoxelClipMask,
  classifyLodSubvoxelClipMask,
  isLodSubvoxelClipped,
  listUnclippedLodSubvoxelBoxes,
  lodSubvoxelBit,
} from "../src/engine/lod-clip-mask.ts";

test("LOD subvoxel clip masks classify none, partial, and full coverage", () => {
  expect(classifyLodSubvoxelClipMask(0)).toBe("none");
  expect(classifyLodSubvoxelClipMask(lodSubvoxelBit(0, 1, 0))).toBe("partial");
  expect(classifyLodSubvoxelClipMask(LOD_SUBVOXEL_CLIP_MASK_FULL)).toBe("full");
});

test("LOD subvoxel clip masks encode stable x/y/z bit positions", () => {
  expect(lodSubvoxelBit(0, 0, 0)).toBe(1);
  expect(lodSubvoxelBit(1, 0, 0)).toBe(2);
  expect(lodSubvoxelBit(0, 1, 0)).toBe(4);
  expect(lodSubvoxelBit(0, 0, 1)).toBe(16);
});

test("LOD subvoxel clip masks build from ownership predicates", () => {
  const mask = buildLodSubvoxelClipMask((localX, localY, localZ) =>
    localY === 0 && (localX === 0 || localZ === 1),
  );

  expect(classifyLodSubvoxelClipMask(mask)).toBe("partial");
  expect(isLodSubvoxelClipped(mask, 0, 0, 0)).toBe(true);
  expect(isLodSubvoxelClipped(mask, 1, 0, 0)).toBe(false);
  expect(isLodSubvoxelClipped(mask, 1, 0, 1)).toBe(true);
  expect(isLodSubvoxelClipped(mask, 0, 1, 1)).toBe(false);
});

test("LOD subvoxel clip masks list normalized boxes left for the coarse mesher", () => {
  const mask = lodSubvoxelBit(0, 0, 0) | lodSubvoxelBit(1, 1, 1);

  expect(listUnclippedLodSubvoxelBoxes(mask)).toEqual([
    { min: [0.5, 0, 0], max: [1, 0.5, 0.5] },
    { min: [0, 0.5, 0], max: [0.5, 1, 0.5] },
    { min: [0.5, 0.5, 0], max: [1, 1, 0.5] },
    { min: [0, 0, 0.5], max: [0.5, 0.5, 1] },
    { min: [0.5, 0, 0.5], max: [1, 0.5, 1] },
    { min: [0, 0.5, 0.5], max: [0.5, 1, 1] },
  ]);
});

test("LOD subvoxel clip masks reject invalid subvoxel coordinates", () => {
  expect(() => lodSubvoxelBit(2, 0, 0)).toThrow(RangeError);
  expect(() => lodSubvoxelBit(0, -1, 0)).toThrow(RangeError);
  expect(() => lodSubvoxelBit(0, 0, 0.5)).toThrow(RangeError);
});
