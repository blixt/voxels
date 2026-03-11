import { expect, test } from "bun:test";

import { fnv1a } from "../src/engine/math.ts";
import {
  buildHexColorPalette,
  hexColorToMaterial,
  materialToHexColor,
  ProceduralWorldGenerator,
  PROCEDURAL_WORLD_MAX_Y,
} from "../src/engine/procedural-generator.ts";

test("hex color palette covers all #RGB materials", () => {
  const palette = buildHexColorPalette();
  const material = hexColorToMaterial("#ABC");

  expect(palette).toHaveLength(4097);
  expect(materialToHexColor(material)).toBe("#ABC");
  expect(palette[material]).toBe(0xffccbbaa);
});

test("procedural generator is deterministic per chunk coordinate", () => {
  const generator = new ProceduralWorldGenerator(4242);

  const a = generator.generateChunk(18, 4, -7);
  const b = generator.generateChunk(18, 4, -7);

  expect(a.solidCount).toBe(b.solidCount);
  expect(fnv1a(new Uint8Array(a.data.buffer))).toBe(fnv1a(new Uint8Array(b.data.buffer)));
});

test("generated chunk data matches direct material sampling", () => {
  const generator = new ProceduralWorldGenerator(999, { chunkSize: 16 });
  const chunk = generator.generateChunk(3, 2, -5);
  const chunkArea = generator.chunkSize * generator.chunkSize;
  const originX = chunk.coord.x * generator.chunkSize;
  const originY = chunk.coord.y * generator.chunkSize;
  const originZ = chunk.coord.z * generator.chunkSize;

  for (const [lx, ly, lz] of [[0, 0, 0], [4, 7, 3], [8, 12, 15], [15, 15, 15]] as const) {
    const worldX = originX + lx;
    const worldY = originY + ly;
    const worldZ = originZ + lz;
    const chunkValue = chunk.data[lx + ly * generator.chunkSize + lz * chunkArea];
    expect(chunkValue).toBe(generator.sampleMaterial(worldX, worldY, worldZ));
  }

  if (chunk.solidCount > 0) {
    expect(chunk.solidBounds).not.toBeNull();
    expect(chunk.solidBounds!.min[0]).toBeGreaterThanOrEqual(0);
    expect(chunk.solidBounds!.min[1]).toBeGreaterThanOrEqual(0);
    expect(chunk.solidBounds!.min[2]).toBeGreaterThanOrEqual(0);
    expect(chunk.solidBounds!.max[0]).toBeLessThanOrEqual(generator.chunkSize);
    expect(chunk.solidBounds!.max[1]).toBeLessThanOrEqual(generator.chunkSize);
    expect(chunk.solidBounds!.max[2]).toBeLessThanOrEqual(generator.chunkSize);
  }
});

test("procedural generator produces multiple biome families across distant coordinates", () => {
  const generator = new ProceduralWorldGenerator(1337);
  const biomeIds = new Set<string>();
  for (let x = -8192; x <= 8192; x += 2048) {
    for (let z = -8192; z <= 8192; z += 2048) {
      biomeIds.add(generator.sampleColumn(x, z).biomeId);
    }
  }

  expect(biomeIds.size).toBeGreaterThanOrEqual(4);
});

test("procedural generator respects the configured Y range", () => {
  const generator = new ProceduralWorldGenerator(7, { maxYExclusive: PROCEDURAL_WORLD_MAX_Y });
  const column = generator.sampleColumn(640, -1280);

  expect(column.surfaceY).toBeGreaterThanOrEqual(8);
  expect(column.surfaceY).toBeLessThan(PROCEDURAL_WORLD_MAX_Y);
  expect(generator.sampleMaterial(640, -1, -1280)).toBe(0);
  expect(generator.sampleMaterial(640, PROCEDURAL_WORLD_MAX_Y, -1280)).toBe(0);
});
