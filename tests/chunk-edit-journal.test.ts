import { expect, test } from "bun:test";

import {
  applyPackedChunkEditDelta,
  compactChunkEditJournal,
  createChunkEditJournal,
  exportChunkEditJournal,
  importChunkEditJournal,
  packChunkEditDelta,
  replayChunkEditJournal,
  toChunkVoxelIndex,
  type ExportedChunkEditJournal,
} from "../src/engine/chunk-edit-journal.ts";

test("chunk edit journal packs sparse local voxel edits in stable index order", () => {
  const packed = packChunkEditDelta(7, [
    { x: 3, y: 0, z: 0, material: 9 },
    { x: 1, y: 0, z: 0, material: 4 },
    { x: 3, y: 0, z: 0, material: 12 },
  ], 4);

  expect(packed).toEqual({
    revision: 7,
    voxelIndices: [1, 3],
    materials: [4, 12],
  });
});

test("chunk edit journal replays revisions deterministically and latest overwrite wins", () => {
  const chunkSize = 4;
  const base = new Uint16Array(chunkSize ** 3);
  const voxelIndex = toChunkVoxelIndex({ x: 2, y: 1, z: 3 }, chunkSize);
  base[voxelIndex] = 5;
  const journal = createChunkEditJournal({ x: 0, y: 0, z: 0 }, chunkSize, [
    packChunkEditDelta(3, [{ x: 2, y: 1, z: 3, material: 20 }], chunkSize),
    packChunkEditDelta(1, [{ x: 2, y: 1, z: 3, material: 10 }], chunkSize),
    packChunkEditDelta(2, [{ x: 0, y: 0, z: 0, material: 6 }], chunkSize),
  ]);

  const replayed = replayChunkEditJournal(journal, base);

  expect(base[voxelIndex]).toBe(5);
  expect(replayed[voxelIndex]).toBe(20);
  expect(replayed[0]).toBe(6);
  expect(journal.minRevision).toBe(1);
  expect(journal.maxRevision).toBe(3);
});

test("chunk edit journal keeps input order stable for equal revisions", () => {
  const chunkSize = 4;
  const base = new Uint16Array(chunkSize ** 3);
  const journal = createChunkEditJournal({ x: 0, y: 0, z: 0 }, chunkSize, [
    packChunkEditDelta(2, [{ x: 1, y: 1, z: 1, material: 11 }], chunkSize),
    packChunkEditDelta(2, [{ x: 1, y: 1, z: 1, material: 12 }], chunkSize),
  ]);

  const replayed = replayChunkEditJournal(journal, base);

  expect(replayed[toChunkVoxelIndex({ x: 1, y: 1, z: 1 }, chunkSize)]).toBe(12);
});

test("chunk edit journal compacts repeated voxel edits while preserving winning revisions", () => {
  const chunkSize = 4;
  const base = new Uint16Array(chunkSize ** 3);
  const journal = createChunkEditJournal({ x: 2, y: 0, z: -1 }, chunkSize, [
    packChunkEditDelta(1, [
      { x: 0, y: 0, z: 0, material: 4 },
      { x: 1, y: 0, z: 0, material: 5 },
    ], chunkSize),
    packChunkEditDelta(4, [{ x: 0, y: 0, z: 0, material: 8 }], chunkSize),
    packChunkEditDelta(3, [{ x: 2, y: 0, z: 0, material: 7 }], chunkSize),
  ]);

  const compacted = compactChunkEditJournal(journal);

  expect(compacted.minRevision).toBe(1);
  expect(compacted.maxRevision).toBe(4);
  expect(compacted.deltas).toEqual([
    { revision: 1, voxelIndices: [1], materials: [5] },
    { revision: 3, voxelIndices: [2], materials: [7] },
    { revision: 4, voxelIndices: [0], materials: [8] },
  ]);
  expect(Array.from(replayChunkEditJournal(compacted, base))).toEqual(
    Array.from(replayChunkEditJournal(journal, base)),
  );
});

test("chunk edit journal supports empty journals", () => {
  const chunkSize = 4;
  const base = new Uint16Array(chunkSize ** 3);
  base[7] = 99;
  const journal = createChunkEditJournal({ x: 0, y: 0, z: 0 }, chunkSize);

  expect(journal.minRevision).toBeNull();
  expect(journal.maxRevision).toBeNull();
  expect(journal.deltas).toEqual([]);
  expect(Array.from(replayChunkEditJournal(journal, base))).toEqual(Array.from(base));
  expect(compactChunkEditJournal(journal).deltas).toEqual([]);
});

test("chunk edit journal import and export preserve negative chunk coordinates", () => {
  const chunkSize = 4;
  const journal = createChunkEditJournal({ x: -12, y: -3, z: 5 }, chunkSize, [
    packChunkEditDelta(9, [{ x: 3, y: 2, z: 1, material: 123 }], chunkSize),
  ]);

  const exported = exportChunkEditJournal(journal);
  const imported = importChunkEditJournal(exported);

  expect(exported).toEqual({
    version: 1,
    coord: { x: -12, y: -3, z: 5 },
    chunkSize,
    minRevision: 9,
    maxRevision: 9,
    deltas: [{ revision: 9, voxelIndices: [27], materials: [123] }],
  });
  expect(imported).toEqual(journal);
});

test("chunk edit journal validates import shape before replay", () => {
  const invalid = {
    version: 1,
    coord: { x: 0, y: 0, z: 0 },
    chunkSize: 4,
    minRevision: 1,
    maxRevision: 1,
    deltas: [{ revision: 1, voxelIndices: [0, 1], materials: [2] }],
  } satisfies ExportedChunkEditJournal;

  expect(() => importChunkEditJournal(invalid)).toThrow("index/material lengths");
});

test("packed chunk edit deltas apply without mutating source data", () => {
  const chunkSize = 4;
  const base = new Uint16Array(chunkSize ** 3);
  const delta = packChunkEditDelta(1, [{ x: 0, y: 1, z: 0, material: 15 }], chunkSize);

  const replayed = applyPackedChunkEditDelta(base, delta, chunkSize);

  expect(base[toChunkVoxelIndex({ x: 0, y: 1, z: 0 }, chunkSize)]).toBe(0);
  expect(replayed[toChunkVoxelIndex({ x: 0, y: 1, z: 0 }, chunkSize)]).toBe(15);
});
