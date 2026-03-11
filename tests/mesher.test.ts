import { expect, test } from "bun:test";

import { buildChunkMesh, rebuildDirtyMeshes } from "../src/engine/mesher.ts";
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

test("removing a boundary voxel recomputes sparse chunk bounds", () => {
  const world = new VoxelWorld({ width: 8, height: 8, depth: 8 }, 8, [0, 0xff8899aa]);
  world.setVoxel(1, 1, 1, 1);
  world.setVoxel(4, 4, 4, 1);
  world.setVoxel(1, 1, 1, 0);

  const mesh = buildChunkMesh(world, 0, 0, 0);
  const positions = collectPositions(mesh);

  expect(mesh.bounds.min).toEqual([4, 4, 4]);
  expect(mesh.bounds.max).toEqual([5, 5, 5]);
  for (const position of positions) {
    expect(position[0]).toBeGreaterThanOrEqual(4);
    expect(position[0]).toBeLessThanOrEqual(5);
    expect(position[1]).toBeGreaterThanOrEqual(4);
    expect(position[1]).toBeLessThanOrEqual(5);
    expect(position[2]).toBeGreaterThanOrEqual(4);
    expect(position[2]).toBeLessThanOrEqual(5);
  }
});

test("meshing skips hidden faces between adjacent chunks", () => {
  const world = new VoxelWorld({ width: 64, height: 32, depth: 32 }, 32, [0, 0xff8899aa]);
  world.fillBox(0, 0, 0, 64, 32, 32, 1);

  const leftMesh = buildChunkMesh(world, 0, 0, 0);
  const rightMesh = buildChunkMesh(world, 1, 0, 0);

  expect(leftMesh.triangleCount + rightMesh.triangleCount).toBe(20);
});

test("meshing skips a fully occluded solid chunk", () => {
  const world = new VoxelWorld({ width: 96, height: 96, depth: 96 }, 32, [0, 0xff8899aa]);
  world.fillBox(0, 0, 0, 96, 96, 96, 1);

  const mesh = buildChunkMesh(world, 1, 1, 1);

  expect(mesh.vertexCount).toBe(0);
  expect(mesh.indexCount).toBe(0);
  expect(mesh.triangleCount).toBe(0);
});

test("meshing does not skip a fully solid chunk when a neighbor face has a hole", () => {
  const world = new VoxelWorld({ width: 96, height: 96, depth: 96 }, 32, [0, 0xff8899aa]);
  world.fillBox(0, 0, 0, 96, 96, 96, 1);
  world.setVoxel(31, 32, 32, 0);

  const mesh = buildChunkMesh(world, 1, 1, 1);

  expect(mesh.triangleCount).toBe(2);
  expect(mesh.bounds.min).toEqual([32, 32, 32]);
  expect(mesh.bounds.max).toEqual([32, 33, 33]);
});

test("budgeted meshing prioritizes nearby unbuilt chunks around the focus point", () => {
  const world = new VoxelWorld({ width: 96, height: 32, depth: 32 }, 32, [0, 0xff8899aa]);
  world.setVoxel(80, 1, 1, 1);
  world.setVoxel(48, 1, 1, 1);
  world.setVoxel(16, 1, 1, 1);

  const summary = rebuildDirtyMeshes(world, 1, {
    priorityPosition: [16, 1, 1],
  });

  expect(summary.meshCount).toBe(1);
  expect(world.getResidentChunk(0, 0, 0)?.meshBuilt).toBe(true);
  expect(world.getResidentChunk(1, 0, 0)?.meshBuilt).toBe(false);
  expect(world.getResidentChunk(2, 0, 0)?.meshBuilt).toBe(false);
});
