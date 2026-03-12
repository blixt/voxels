import { expect, test } from "bun:test";

import {
  buildChunkMesh,
  buildChunkMeshFromOpaqueGeometry,
  createOpaqueChunkMeshingInput,
} from "../src/engine/mesher.ts";
import {
  buildOpaqueChunkMeshFromInput,
  createMeshMaterialLut,
} from "../src/engine/opaque-chunk-mesher.ts";
import { VoxelWorld } from "../src/engine/world.ts";

test("opaque chunk meshing input clones chunk data when requested", () => {
  const world = new VoxelWorld({ width: 16, height: 16, depth: 16 }, 16, [0, 0xff88aa44]);
  world.setVoxel(1, 1, 1, 1);

  const input = createOpaqueChunkMeshingInput(world, 0, 0, 0, { cloneData: true });
  expect(input).not.toBeNull();

  world.setVoxel(1, 1, 1, 0);

  expect(input?.chunkData[1 + 1 * 16 + 1 * 16 * 16]).toBe(1);
});

test("worker-friendly opaque meshing matches the synchronous opaque mesh", () => {
  const world = new VoxelWorld({ width: 32, height: 16, depth: 16 }, 16, [0, 0xff88aa44, 0xff4466aa]);
  world.setVoxel(1, 1, 1, 1);
  world.setVoxel(2, 1, 1, 1);
  world.setVoxel(15, 1, 1, 1);
  world.setVoxel(16, 1, 1, 2);
  world.setVoxel(16, 2, 1, 2);

  const syncMesh = buildChunkMesh(world, 0, 0, 0);
  const input = createOpaqueChunkMeshingInput(world, 0, 0, 0, { cloneData: true });
  expect(input).not.toBeNull();

  const opaqueMesh = buildOpaqueChunkMeshFromInput(
    input!,
    createMeshMaterialLut(world.palette, (materialIndex) => world.isWaterMaterial(materialIndex)),
  );
  const combinedMesh = buildChunkMeshFromOpaqueGeometry(world, 0, 0, 0, opaqueMesh);

  expect(opaqueMesh.vertexCount).toBe(syncMesh.vertexCount);
  expect(opaqueMesh.indexCount).toBe(syncMesh.indexCount);
  expect(opaqueMesh.triangleCount).toBe(syncMesh.triangleCount);
  expect(new Uint8Array(opaqueMesh.vertexData)).toEqual(new Uint8Array(syncMesh.vertexData));
  expect(opaqueMesh.indexData).toEqual(syncMesh.indexData);
  expect(combinedMesh.vertexCount).toBe(syncMesh.vertexCount);
  expect(combinedMesh.indexCount).toBe(syncMesh.indexCount);
  expect(combinedMesh.waterVertexCount).toBe(syncMesh.waterVertexCount);
  expect(combinedMesh.waterIndexCount).toBe(syncMesh.waterIndexCount);
  expect(combinedMesh.triangleCount).toBe(syncMesh.triangleCount);
  expect(combinedMesh.bounds).toEqual(syncMesh.bounds);
});
