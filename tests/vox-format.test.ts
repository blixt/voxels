import { expect, test } from "bun:test";

import { importMagicaVoxel } from "../src/engine/vox-format.ts";

test("MagicaVoxel importer reads a minimal single-model scene", () => {
  const bytes = buildMinimalVox();
  const { world, warnings } = importMagicaVoxel(bytes);

  expect(warnings).toEqual([]);
  expect(world.width).toBe(4);
  expect(world.height).toBe(4);
  expect(world.depth).toBe(4);
  expect(world.getVoxel(1, 3, 2)).toBe(1);
});

function buildMinimalVox(): Uint8Array {
  const sizeChunk = chunk("SIZE", ints([4, 4, 4]));
  const xyziChunk = chunk("XYZI", ints([1, 1 | (2 << 8) | (3 << 16) | (1 << 24)]));
  const rgba = new Uint8Array(255 * 4);
  rgba.set([255, 0, 0, 255], 0);
  const rgbaChunk = chunk("RGBA", rgba);
  const children = concat(sizeChunk, xyziChunk, rgbaChunk);
  const main = concat(text("MAIN"), ints([0, children.length]), children);
  return concat(text("VOX "), ints([150]), main);
}

function chunk(id: string, content: Uint8Array): Uint8Array {
  return concat(text(id), ints([content.length, 0]), content);
}

function ints(values: number[]): Uint8Array {
  const bytes = new Uint8Array(values.length * 4);
  const view = new DataView(bytes.buffer);
  values.forEach((value, index) => view.setInt32(index * 4, value, true));
  return bytes;
}

function text(value: string): Uint8Array {
  return new TextEncoder().encode(value);
}

function concat(...parts: Uint8Array[]): Uint8Array {
  const total = parts.reduce((sum, part) => sum + part.length, 0);
  const out = new Uint8Array(total);
  let offset = 0;
  for (const part of parts) {
    out.set(part, offset);
    offset += part.length;
  }
  return out;
}
