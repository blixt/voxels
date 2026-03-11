import { expect, test } from "bun:test";

import { buildStreamAnchorPosition, resolveStreamAnchor } from "../src/engine/stream-anchor.ts";

test("stream anchor initializes to the player chunk", () => {
  const result = resolveStreamAnchor(null, -26, -24, 1);

  expect(result.changed).toBe(true);
  expect(result.anchor).toEqual({ chunkX: -26, chunkZ: -24 });
});

test("stream anchor stays put within the configured chunk margin", () => {
  const result = resolveStreamAnchor({ chunkX: -26, chunkZ: -24 }, -25, -25, 1);

  expect(result.changed).toBe(false);
  expect(result.anchor).toEqual({ chunkX: -26, chunkZ: -24 });
});

test("stream anchor shifts once the player exceeds the configured chunk margin", () => {
  const result = resolveStreamAnchor({ chunkX: -26, chunkZ: -24 }, -24, -22, 1);

  expect(result.changed).toBe(true);
  expect(result.anchor).toEqual({ chunkX: -24, chunkZ: -22 });
});

test("stream anchor positions target the center of a chunk column", () => {
  expect(buildStreamAnchorPosition({ chunkX: -26, chunkZ: 3 }, 32, 1615)).toEqual([-816, 1615, 112]);
});
