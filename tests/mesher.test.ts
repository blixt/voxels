import { expect, test } from "bun:test";

import { buildChunkMesh } from "../src/engine/mesher.ts";
import { VoxelWorld } from "../src/engine/world.ts";

function collectPositions(mesh: ReturnType<typeof buildChunkMesh>): Array<[number, number, number]> {
  const view = new DataView(mesh.vertexData);
  const positions: Array<[number, number, number]> = [];
  for (let index = 0; index < mesh.vertexCount; index += 1) {
    const byteOffset = index * 20;
    positions.push([
      view.getFloat32(byteOffset + 0, true),
      view.getFloat32(byteOffset + 4, true),
      view.getFloat32(byteOffset + 8, true),
    ]);
  }
  return positions;
}

test("greedy mesher collapses a solid box into six quads", () => {
  const world = new VoxelWorld({ width: 32, height: 32, depth: 32 }, 32, [0, 0xff8899aa]);
  world.fillBox(10, 10, 10, 12, 12, 12, 1);

  const mesh = buildChunkMesh(world, 0, 0, 0);

  expect(mesh.vertexCount).toBe(24);
  expect(mesh.indexCount).toBe(36);
  expect(mesh.triangleCount).toBe(12);
  expect(mesh.bounds.min).toEqual([10, 10, 10]);
  expect(mesh.bounds.max).toEqual([12, 12, 12]);
});

test("greedy mesher skips completely empty chunks", () => {
  const world = new VoxelWorld({ width: 32, height: 32, depth: 32 }, 32, [0, 0xff8899aa]);

  const mesh = buildChunkMesh(world, 0, 0, 0);

  expect(mesh.vertexCount).toBe(0);
  expect(mesh.indexCount).toBe(0);
});

test("single voxel mesh stays inside that voxel bounds", () => {
  const world = new VoxelWorld({ width: 8, height: 8, depth: 8 }, 8, [0, 0xff8899aa]);
  world.setVoxel(1, 1, 1, 1);

  const mesh = buildChunkMesh(world, 0, 0, 0);
  const positions = collectPositions(mesh);

  expect(mesh.bounds.min).toEqual([1, 1, 1]);
  expect(mesh.bounds.max).toEqual([2, 2, 2]);
  for (const position of positions) {
    expect(position[0]).toBeGreaterThanOrEqual(1);
    expect(position[0]).toBeLessThanOrEqual(2);
    expect(position[1]).toBeGreaterThanOrEqual(1);
    expect(position[1]).toBeLessThanOrEqual(2);
    expect(position[2]).toBeGreaterThanOrEqual(1);
    expect(position[2]).toBeLessThanOrEqual(2);
  }
});
