import { expect, test } from "bun:test";

import { ProceduralWorldGenerator } from "../src/engine/procedural-generator.ts";
import { ProceduralResidentWorld } from "../src/engine/procedural-resident-world.ts";
import type { VoxelChunk } from "../src/engine/world.ts";

const CHUNK_SIZE = 32;

test("adopting an unmeshed resident chunk keeps coarser LOD coverage until the resident chunk is render-ready", () => {
  const world = new ProceduralResidentWorld(new ProceduralWorldGenerator(1337), { horizontalRadiusChunks: 1 });
  const residentChunk = createTestChunk(0, 0, 0, 0, 1, false);
  const lodChunk = createTestChunk(0, 0, 0, 1, 2, true);
  const worldWithInternals = world as unknown as {
    lodChunks: Map<string, VoxelChunk>;
    staleLodKeys: Set<string>;
    adoptResidentChunk(key: string, chunk: VoxelChunk): void;
  };

  worldWithInternals.lodChunks.set("L1:0:0:0", lodChunk);
  worldWithInternals.adoptResidentChunk("0:0:0", residentChunk);

  expect(worldWithInternals.lodChunks.has("L1:0:0:0")).toBe(true);

  world.noteResidentChunkRenderReadyState(residentChunk, true);

  expect(worldWithInternals.lodChunks.has("L1:0:0:0")).toBe(true);
  expect(worldWithInternals.staleLodKeys.has("L1:0:0:0")).toBe(true);
});

test("coarser LOD source invalidation keeps stale chunks visible until replacement is ready", () => {
  const world = new ProceduralResidentWorld(new ProceduralWorldGenerator(1337), { horizontalRadiusChunks: 1 });
  const lodChunk = createTestChunk(0, 0, 0, 2, 4, true);
  const worldWithInternals = world as unknown as {
    lodChunks: Map<string, VoxelChunk>;
    staleLodKeys: Set<string>;
    invalidateCoarserLodChunksForSourceChunk(sourceLevel: number, sourceCx: number, sourceCy: number, sourceCz: number): void;
  };

  worldWithInternals.lodChunks.set("L2:0:0:0", lodChunk);
  worldWithInternals.invalidateCoarserLodChunksForSourceChunk(1, 0, 0, 0);

  expect(worldWithInternals.lodChunks.has("L2:0:0:0")).toBe(true);
  expect(worldWithInternals.staleLodKeys.has("L2:0:0:0")).toBe(true);
});

test("edit invalidation keeps active LOD visible while marking it for regeneration", () => {
  const world = new ProceduralResidentWorld(new ProceduralWorldGenerator(1337), { horizontalRadiusChunks: 1 });
  const lodChunk = createTestChunk(0, 0, 0, 1, 2, true);
  const worldWithInternals = world as unknown as {
    lodChunks: Map<string, VoxelChunk>;
    staleLodKeys: Set<string>;
    invalidateLodChunksAt(worldX: number, worldY: number, worldZ: number): void;
  };

  worldWithInternals.lodChunks.set("L1:0:0:0", lodChunk);
  worldWithInternals.invalidateLodChunksAt(1, 1, 1);

  expect(worldWithInternals.lodChunks.has("L1:0:0:0")).toBe(true);
  expect(worldWithInternals.staleLodKeys.has("L1:0:0:0")).toBe(true);
});

function createTestChunk(
  cx: number,
  cy: number,
  cz: number,
  lodLevel: number,
  voxelStride: number,
  renderReady: boolean,
): VoxelChunk {
  return {
    coord: { x: cx, y: cy, z: cz },
    lodLevel,
    voxelStride,
    data: new Uint16Array(CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE),
    solidCount: 0,
    solidBounds: null,
    meshBuilt: renderReady,
    meshDirty: !renderReady,
    renderReady,
    meshRevision: 1,
    pendingMeshRevision: null,
    gpuDirty: renderReady,
    mesh: null,
  };
}
