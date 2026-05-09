import { expect, test } from "bun:test";

import {
  createProceduralGeneratedChunkCacheKeys,
  createProceduralGeneratedChunkCacheWorldKey,
} from "../src/client/procedural-generated-chunk-cache.ts";
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
