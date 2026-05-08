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

test("prepared finer LOD chunks do not become visible until they fully replace the covered coarser chunk", () => {
  const world = new ProceduralResidentWorld(new ProceduralWorldGenerator(1337), { horizontalRadiusChunks: 1 });
  const coarserChunk = createTestChunk(0, 0, 0, 2, 4, true, true);
  const worldWithInternals = world as unknown as {
    lodChunks: Map<string, VoxelChunk>;
    preparedLodChunks: Map<string, VoxelChunk>;
    commitPreparedLodChunks(): void;
  };

  worldWithInternals.lodChunks.set("L2:0:0:0", coarserChunk);
  worldWithInternals.preparedLodChunks.set("L1:0:0:0", createTestChunk(0, 0, 0, 1, 2, true, true));

  worldWithInternals.commitPreparedLodChunks();

  expect(worldWithInternals.lodChunks.has("L2:0:0:0")).toBe(true);
  expect(worldWithInternals.lodChunks.has("L1:0:0:0")).toBe(false);
  expect(worldWithInternals.preparedLodChunks.has("L1:0:0:0")).toBe(true);
});

test("prepared finer LOD chunks atomically replace the covered coarser chunk once complete", () => {
  const world = new ProceduralResidentWorld(new ProceduralWorldGenerator(1337), { horizontalRadiusChunks: 1 });
  const worldWithInternals = world as unknown as {
    lodChunks: Map<string, VoxelChunk>;
    preparedLodChunks: Map<string, VoxelChunk>;
    commitPreparedLodChunks(): void;
  };

  worldWithInternals.lodChunks.set("L2:0:0:0", createTestChunk(0, 0, 0, 2, 4, true, true));
  worldWithInternals.preparedLodChunks.set("L1:0:0:0", createTestChunk(0, 0, 0, 1, 2, true, true));
  worldWithInternals.preparedLodChunks.set("L1:1:0:0", createTestChunk(1, 0, 0, 1, 2, true, true));
  worldWithInternals.preparedLodChunks.set("L1:0:0:1", createTestChunk(0, 0, 1, 1, 2, true, true));
  worldWithInternals.preparedLodChunks.set("L1:1:0:1", createTestChunk(1, 0, 1, 1, 2, true, true));

  worldWithInternals.commitPreparedLodChunks();

  expect(worldWithInternals.lodChunks.has("L2:0:0:0")).toBe(false);
  expect(worldWithInternals.preparedLodChunks.size).toBe(0);
  expect(worldWithInternals.lodChunks.has("L1:0:0:0")).toBe(true);
  expect(worldWithInternals.lodChunks.has("L1:1:0:0")).toBe(true);
  expect(worldWithInternals.lodChunks.has("L1:0:0:1")).toBe(true);
  expect(worldWithInternals.lodChunks.has("L1:1:0:1")).toBe(true);
});

test("revived retained coarser LOD chunks are punched against active finer coverage", () => {
  const world = new ProceduralResidentWorld(new ProceduralWorldGenerator(1337), { horizontalRadiusChunks: 1 });
  const worldWithInternals = world as unknown as {
    lodChunks: Map<string, VoxelChunk>;
    retainedLodChunks: Map<string, VoxelChunk>;
    coveragePunchedLodKeys: Set<string>;
    reviveRetainedLodChunk(key: string): boolean;
  };

  worldWithInternals.lodChunks.set("L1:0:0:0", createTestChunk(0, 0, 0, 1, 2, true, true));
  worldWithInternals.retainedLodChunks.set("L2:0:0:0", createTestChunk(0, 0, 0, 2, 4, true, true));

  expect(worldWithInternals.reviveRetainedLodChunk("L2:0:0:0")).toBe(true);

  const revived = worldWithInternals.lodChunks.get("L2:0:0:0");
  expect(revived).toBeDefined();
  expect(worldWithInternals.coveragePunchedLodKeys.has("L2:0:0:0")).toBe(true);
  expect(columnHasMaterial(revived!, 0, 0)).toBe(false);
  expect(columnHasMaterial(revived!, 15, 15)).toBe(false);
  expect(columnHasMaterial(revived!, 16, 0)).toBe(true);
  expect(columnHasMaterial(revived!, 20, 20)).toBe(true);
});

test("render-ready resident columns punch stale active LOD coverage instead of overlapping it", () => {
  const world = new ProceduralResidentWorld(new ProceduralWorldGenerator(1337), { horizontalRadiusChunks: 1 });
  const residentChunk = createTestChunk(0, 0, 0, 0, 1, false, true);
  const worldWithInternals = world as unknown as {
    lodChunks: Map<string, VoxelChunk>;
    staleLodKeys: Set<string>;
    coveragePunchedLodKeys: Set<string>;
    adoptResidentChunk(key: string, chunk: VoxelChunk): void;
  };

  worldWithInternals.lodChunks.set("L1:0:0:0", createTestChunk(0, 0, 0, 1, 2, true, true));
  worldWithInternals.adoptResidentChunk("0:0:0", residentChunk);
  world.noteResidentChunkRenderReadyState(residentChunk, true);

  const punched = worldWithInternals.lodChunks.get("L1:0:0:0");
  expect(punched).toBeDefined();
  expect(worldWithInternals.staleLodKeys.has("L1:0:0:0")).toBe(true);
  expect(worldWithInternals.coveragePunchedLodKeys.has("L1:0:0:0")).toBe(true);
  expect(columnHasMaterial(punched!, 0, 0)).toBe(false);
  expect(columnHasMaterial(punched!, 15, 15)).toBe(false);
  expect(columnHasMaterial(punched!, 16, 0)).toBe(true);
});

function createTestChunk(
  cx: number,
  cy: number,
  cz: number,
  lodLevel: number,
  voxelStride: number,
  renderReady: boolean,
  solid = false,
): VoxelChunk {
  const data = new Uint16Array(CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE);
  if (solid) {
    data.fill(1);
  }
  return {
    coord: { x: cx, y: cy, z: cz },
    lodLevel,
    voxelStride,
    data,
    solidCount: solid ? data.length : 0,
    solidBounds: solid
      ? { min: [0, 0, 0], max: [CHUNK_SIZE, CHUNK_SIZE, CHUNK_SIZE], dirty: false }
      : null,
    meshBuilt: renderReady,
    meshDirty: !renderReady,
    renderReady,
    meshRevision: 1,
    pendingMeshRevision: null,
    gpuDirty: renderReady,
    mesh: null,
  };
}

function columnHasMaterial(chunk: VoxelChunk, localX: number, localZ: number): boolean {
  const chunkArea = CHUNK_SIZE * CHUNK_SIZE;
  for (let localY = 0; localY < CHUNK_SIZE; localY += 1) {
    if (chunk.data[localX + localY * CHUNK_SIZE + localZ * chunkArea] !== 0) {
      return true;
    }
  }
  return false;
}
