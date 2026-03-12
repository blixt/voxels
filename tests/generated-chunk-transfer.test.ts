import { expect, test } from "bun:test";

import { ProceduralWorldGenerator } from "../src/engine/procedural-generator.ts";
import {
  deserializeGeneratedChunk,
  serializeGeneratedChunk,
} from "../src/engine/generated-chunk-transfer.ts";

test("generated chunk transfer preserves coord, payload, and bounds", () => {
  const generator = new ProceduralWorldGenerator(1337);
  const generated = generator.generateChunk(0, Math.floor(generator.seaLevel / generator.chunkSize), 0);
  const serialized = serializeGeneratedChunk(generated);
  const restored = deserializeGeneratedChunk(serialized.chunk);

  expect(restored.coord).toEqual(generated.coord);
  expect(restored.solidCount).toBe(generated.solidCount);
  expect(restored.solidBounds).toEqual(generated.solidBounds);
  expect(restored.data.length).toBe(generated.data.length);
  expect(restored.data[0]).toBe(generated.data[0]);
  expect(restored.data[Math.floor(restored.data.length / 2)]).toBe(generated.data[Math.floor(generated.data.length / 2)]);
  expect(restored.data[restored.data.length - 1]).toBe(generated.data[generated.data.length - 1]);
});
