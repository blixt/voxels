import { expect, test } from "bun:test";

import {
  CANONICAL_CHUNK_STORE_SCHEMA_VERSION,
  createCanonicalChunkRecordMetadata,
  formatCanonicalChunkKey,
  formatCanonicalChunkSummaryKey,
  formatCanonicalRegionSummaryKey,
  formatCanonicalWorldKey,
  formatChunkEditJournalKey,
  isCanonicalChunkRecordUsable,
  type CanonicalWorldKey,
} from "../src/engine/canonical-chunk-store.ts";

const WORLD_KEY: CanonicalWorldKey = {
  generationVersion: "proc-v42",
  seed: 1337,
  seaLevel: 20,
  chunkSize: 32,
  maxYExclusive: 3072,
};

test("canonical world and chunk keys include generation identity and negative coordinates", () => {
  expect(formatCanonicalWorldKey(WORLD_KEY)).toBe("proc-v42:1337:20:32:3072");
  expect(formatCanonicalChunkKey(WORLD_KEY, { x: -12, y: 3, z: -99 }))
    .toBe("proc-v42:1337:20:32:3072:chunk:-12:3:-99");
  expect(formatCanonicalChunkSummaryKey(WORLD_KEY, { x: -12, y: 3, z: -99 }))
    .toBe("proc-v42:1337:20:32:3072:summary:-12:3:-99");
  expect(formatCanonicalRegionSummaryKey(WORLD_KEY, { x: -4, z: 7 }))
    .toBe("proc-v42:1337:20:32:3072:region-summary:-4:7");
  expect(formatChunkEditJournalKey(WORLD_KEY, { x: -12, y: 3, z: -99 }))
    .toBe("proc-v42:1337:20:32:3072:edits:-12:3:-99");
});

test("canonical chunk metadata records revision, byte size, and schema", () => {
  const metadata = createCanonicalChunkRecordMetadata({
    worldKey: WORLD_KEY,
    coord: { x: 1, y: 2, z: 3 },
    canonicalRevision: 17,
    encodedByteLength: 4096,
    storedAt: 1778310000000,
  });

  expect(metadata).toEqual({
    schemaVersion: CANONICAL_CHUNK_STORE_SCHEMA_VERSION,
    key: "proc-v42:1337:20:32:3072:chunk:1:2:3",
    worldKey: "proc-v42:1337:20:32:3072",
    generationVersion: "proc-v42",
    coord: { x: 1, y: 2, z: 3 },
    canonicalRevision: 17,
    encodedByteLength: 4096,
    storedAt: 1778310000000,
  });
});

test("canonical chunk records reject stale generation and coordinate mismatches", () => {
  const metadata = createCanonicalChunkRecordMetadata({
    worldKey: WORLD_KEY,
    coord: { x: -1, y: 0, z: 2 },
    canonicalRevision: 0,
    encodedByteLength: 128,
    storedAt: 1,
  });

  expect(isCanonicalChunkRecordUsable(metadata, WORLD_KEY, { x: -1, y: 0, z: 2 })).toBe(true);
  expect(isCanonicalChunkRecordUsable(
    metadata,
    { ...WORLD_KEY, generationVersion: "proc-v43" },
    { x: -1, y: 0, z: 2 },
  )).toBe(false);
  expect(isCanonicalChunkRecordUsable(metadata, WORLD_KEY, { x: -1, y: 0, z: 3 })).toBe(false);
  expect(isCanonicalChunkRecordUsable(
    { ...metadata, encodedByteLength: 0 },
    WORLD_KEY,
    { x: -1, y: 0, z: 2 },
  )).toBe(false);
});

test("canonical key helpers reject malformed world keys", () => {
  expect(() => formatCanonicalWorldKey({ ...WORLD_KEY, generationVersion: "" })).toThrow();
  expect(() => formatCanonicalWorldKey({ ...WORLD_KEY, chunkSize: 0 })).toThrow(RangeError);
  expect(() => formatCanonicalChunkKey(WORLD_KEY, { x: 1.5, y: 0, z: 0 })).toThrow(RangeError);
  expect(() => createCanonicalChunkRecordMetadata({
    worldKey: WORLD_KEY,
    coord: { x: 0, y: 0, z: 0 },
    canonicalRevision: -1,
    encodedByteLength: 1,
    storedAt: 1,
  })).toThrow(RangeError);
});
