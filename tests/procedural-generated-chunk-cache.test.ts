import { expect, test } from "bun:test";

import {
  createProceduralGeneratedChunkCacheKeys,
  createProceduralGeneratedChunkCacheWorldKey,
  createProceduralGeneratedChunkEditJournalRecord,
} from "../src/client/procedural-generated-chunk-cache.ts";
import { packChunkEditDelta } from "../src/engine/chunk-edit-journal.ts";
import { PROCEDURAL_WORLD_GENERATION_VERSION } from "../src/engine/procedural-generator.ts";

const CONTEXT = {
  seed: 1337,
  seaLevel: 20,
  chunkSize: 32,
  maxYExclusive: 3072,
};

test("procedural generated chunk cache uses canonical world identity", () => {
  expect(createProceduralGeneratedChunkCacheWorldKey(CONTEXT)).toEqual({
    generationVersion: PROCEDURAL_WORLD_GENERATION_VERSION,
    seed: 1337,
    seaLevel: 20,
    chunkSize: 32,
    maxYExclusive: 3072,
  });
});

test("procedural generated chunk cache builds canonical edit journal records", () => {
  const worldKey = createProceduralGeneratedChunkCacheWorldKey(CONTEXT);
  const coord = { x: -4, y: 2, z: -9 };
  const first = createProceduralGeneratedChunkEditJournalRecord({
    worldKey,
    coord,
    chunkSize: CONTEXT.chunkSize,
    deltas: [
      packChunkEditDelta(1, [
        { x: 3, y: 0, z: 0, material: 9 },
        { x: 1, y: 0, z: 0, material: 4 },
      ], CONTEXT.chunkSize),
    ],
    revision: 1,
    storedAt: 10,
  });

  expect(first).toEqual({
    key: `${PROCEDURAL_WORLD_GENERATION_VERSION}:1337:20:32:3072:edits:-4:2:-9`,
    worldKey: `${PROCEDURAL_WORLD_GENERATION_VERSION}:1337:20:32:3072`,
    coord,
    baseGenerationVersion: PROCEDURAL_WORLD_GENERATION_VERSION,
    minRevision: 1,
    maxRevision: 1,
    deltas: [{ revision: 1, voxelIndices: [1, 3], materials: [4, 9] }],
    storedAt: 10,
  });

  const merged = createProceduralGeneratedChunkEditJournalRecord({
    worldKey,
    coord,
    chunkSize: CONTEXT.chunkSize,
    existing: first,
    deltas: [packChunkEditDelta(3, [{ x: 1, y: 0, z: 0, material: 12 }], CONTEXT.chunkSize)],
    revision: 3,
    storedAt: 20,
  });

  expect(merged?.minRevision).toBe(1);
  expect(merged?.maxRevision).toBe(3);
  expect(merged?.deltas).toEqual([
    { revision: 1, voxelIndices: [1, 3], materials: [4, 9] },
    { revision: 3, voxelIndices: [1], materials: [12] },
  ]);
  expect(merged?.storedAt).toBe(20);
});

test("procedural generated chunk edit journals reject stale append revisions", () => {
  const worldKey = createProceduralGeneratedChunkCacheWorldKey(CONTEXT);

  expect(() => createProceduralGeneratedChunkEditJournalRecord({
    worldKey,
    coord: { x: 0, y: 0, z: 0 },
    chunkSize: CONTEXT.chunkSize,
    deltas: [packChunkEditDelta(4, [{ x: 0, y: 0, z: 0, material: 2 }], CONTEXT.chunkSize)],
    revision: 3,
    storedAt: 1,
  })).toThrow("exceeds append revision");
});

test("procedural generated chunk cache exposes canonical and legacy keys", () => {
  const keys = createProceduralGeneratedChunkCacheKeys(CONTEXT, { x: -4, y: 2, z: -9 });
  const prefix = `${PROCEDURAL_WORLD_GENERATION_VERSION}:1337:20:32:3072`;

  expect(keys).toEqual({
    worldKey: prefix,
    chunkKey: `${prefix}:chunk:-4:2:-9`,
    legacyChunkKey: `${prefix}:-4:2:-9`,
    chunkSummaryKey: `${prefix}:summary:-4:2:-9`,
    legacyChunkSummaryKey: `${prefix}:-4:2:-9`,
    editJournalKey: `${prefix}:edits:-4:2:-9`,
  });
});
