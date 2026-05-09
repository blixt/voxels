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
    lodRenderClipMasks: Map<string, Uint8Array>;
    reviveRetainedLodChunk(key: string): boolean;
  };

  worldWithInternals.lodChunks.set("L1:0:0:0", createTestChunk(0, 0, 0, 1, 2, true, true));
  worldWithInternals.retainedLodChunks.set("L2:0:0:0", createTestChunk(0, 0, 0, 2, 4, true, true));

  expect(worldWithInternals.reviveRetainedLodChunk("L2:0:0:0")).toBe(true);

  const revived = worldWithInternals.lodChunks.get("L2:0:0:0");
  expect(revived).toBeDefined();
  expect(worldWithInternals.coveragePunchedLodKeys.has("L2:0:0:0")).toBe(true);
  const clipMask = expectClipMask(worldWithInternals, "L2:0:0:0");
  expect(clipMask[0]).toBe(1);
  expect(clipMask[15 + 15 * CHUNK_SIZE + 15 * CHUNK_SIZE * CHUNK_SIZE]).toBe(1);
  expect(clipMask[16 * CHUNK_SIZE]).toBeFalsy();
  expect(columnHasVisibleMaterialInYRange(revived!, clipMask, 0, 0, 0, CHUNK_SIZE / 2)).toBe(false);
  expect(columnHasVisibleMaterialInYRange(revived!, clipMask, 15, 15, 0, CHUNK_SIZE / 2)).toBe(false);
  expect(columnHasVisibleMaterialInYRange(revived!, clipMask, 0, 0, CHUNK_SIZE / 2, CHUNK_SIZE)).toBe(true);
  expect(columnHasVisibleMaterial(revived!, clipMask, 16, 0)).toBe(true);
  expect(columnHasVisibleMaterial(revived!, clipMask, 20, 20)).toBe(true);
});

test("render-ready resident columns punch stale active LOD coverage instead of overlapping it", () => {
  const world = new ProceduralResidentWorld(new ProceduralWorldGenerator(1337), { horizontalRadiusChunks: 1 });
  const residentChunk = createTestChunk(0, 0, 0, 0, 1, false, true);
  const worldWithInternals = world as unknown as {
    lodChunks: Map<string, VoxelChunk>;
    staleLodKeys: Set<string>;
    coveragePunchedLodKeys: Set<string>;
    lodRenderClipMasks: Map<string, Uint8Array>;
    adoptResidentChunk(key: string, chunk: VoxelChunk): void;
  };

  worldWithInternals.lodChunks.set("L1:0:0:0", createTestChunk(0, 0, 0, 1, 2, true, true));
  worldWithInternals.adoptResidentChunk("0:0:0", residentChunk);
  world.noteResidentChunkRenderReadyState(residentChunk, true);

  const punched = worldWithInternals.lodChunks.get("L1:0:0:0");
  expect(punched).toBeDefined();
  expect(worldWithInternals.staleLodKeys.has("L1:0:0:0")).toBe(true);
  expect(worldWithInternals.coveragePunchedLodKeys.has("L1:0:0:0")).toBe(true);
  const clipMask = expectClipMask(worldWithInternals, "L1:0:0:0");
  expect(columnHasVisibleMaterial(punched!, clipMask, 0, 0)).toBe(false);
  expect(columnHasVisibleMaterial(punched!, clipMask, 15, 15)).toBe(false);
  expect(columnHasVisibleMaterial(punched!, clipMask, 16, 0)).toBe(true);
  expect(world.classifyVisibleLodColumn(punched!, 0, 0)).toEqual({
    covered: false,
    water: false,
    minY: null,
    maxY: null,
  });
  expect(world.classifyVisibleLodColumn(punched!, 32, 0).covered).toBe(true);
});

test("activating a finer LOD chunk punches active coarser columns in the same footprint", () => {
  const world = new ProceduralResidentWorld(new ProceduralWorldGenerator(1337), { horizontalRadiusChunks: 1 });
  const worldWithInternals = world as unknown as {
    lodChunks: Map<string, VoxelChunk>;
    coveragePunchedLodKeys: Set<string>;
    lodRenderClipMasks: Map<string, Uint8Array>;
    punchActiveCoarserLodChunksCoveredBy(finer: VoxelChunk): void;
  };
  const coarser = createTestChunk(0, 0, 0, 2, 4, true, true);
  const finer = createTestChunk(0, 0, 0, 1, 2, true, true);

  worldWithInternals.lodChunks.set("L2:0:0:0", coarser);
  worldWithInternals.lodChunks.set("L1:0:0:0", finer);
  worldWithInternals.punchActiveCoarserLodChunksCoveredBy(finer);

  const punched = worldWithInternals.lodChunks.get("L2:0:0:0");
  expect(punched).toBeDefined();
  expect(worldWithInternals.coveragePunchedLodKeys.has("L2:0:0:0")).toBe(true);
  const clipMask = expectClipMask(worldWithInternals, "L2:0:0:0");
  expect(columnHasVisibleMaterialInYRange(punched!, clipMask, 0, 0, 0, CHUNK_SIZE / 2)).toBe(false);
  expect(columnHasVisibleMaterialInYRange(punched!, clipMask, 15, 15, 0, CHUNK_SIZE / 2)).toBe(false);
  expect(columnHasVisibleMaterialInYRange(punched!, clipMask, 0, 0, CHUNK_SIZE / 2, CHUNK_SIZE)).toBe(true);
  expect(columnHasVisibleMaterial(punched!, clipMask, 16, 0)).toBe(true);
  expect(columnHasVisibleMaterial(punched!, clipMask, 20, 20)).toBe(true);
});

test("activating negative-coordinate finer LOD chunks punches the matching coarser boundary row", () => {
  const world = new ProceduralResidentWorld(new ProceduralWorldGenerator(1337), { horizontalRadiusChunks: 1 });
  const worldWithInternals = world as unknown as {
    lodChunks: Map<string, VoxelChunk>;
    coveragePunchedLodKeys: Set<string>;
    lodRenderClipMasks: Map<string, Uint8Array>;
    punchActiveCoarserLodChunksCoveredBy(finer: VoxelChunk): void;
  };
  const coarser = createTestChunk(-98, 0, -188, 2, 4, true, true);
  const finerLeft = createTestChunk(-196, 0, -376, 1, 2, true, true);
  const finerRight = createTestChunk(-195, 0, -376, 1, 2, true, true);

  worldWithInternals.lodChunks.set("L2:-98:0:-188", coarser);
  worldWithInternals.lodChunks.set("L1:-196:0:-376", finerLeft);
  worldWithInternals.lodChunks.set("L1:-195:0:-376", finerRight);
  worldWithInternals.punchActiveCoarserLodChunksCoveredBy(finerLeft);
  worldWithInternals.punchActiveCoarserLodChunksCoveredBy(finerRight);

  const punched = worldWithInternals.lodChunks.get("L2:-98:0:-188");
  expect(punched).toBeDefined();
  expect(worldWithInternals.coveragePunchedLodKeys.has("L2:-98:0:-188")).toBe(true);
  const clipMask = expectClipMask(worldWithInternals, "L2:-98:0:-188");
  for (let localZ = 0; localZ < CHUNK_SIZE / 2; localZ += 1) {
    for (let localX = 0; localX < CHUNK_SIZE; localX += 1) {
      expect(columnHasVisibleMaterialInYRange(punched!, clipMask, localX, localZ, 0, CHUNK_SIZE / 2)).toBe(false);
      expect(columnHasVisibleMaterialInYRange(punched!, clipMask, localX, localZ, CHUNK_SIZE / 2, CHUNK_SIZE)).toBe(true);
    }
  }
  expect(columnHasVisibleMaterial(punched!, clipMask, 0, 16)).toBe(true);
});

test("prepared replacement keeps stale active finer ownership punched out of coarser chunks", () => {
  const world = new ProceduralResidentWorld(new ProceduralWorldGenerator(1337), { horizontalRadiusChunks: 1 });
  const worldWithInternals = world as unknown as {
    lodChunks: Map<string, VoxelChunk>;
    preparedLodChunks: Map<string, VoxelChunk>;
    staleLodKeys: Set<string>;
    coveragePunchedLodKeys: Set<string>;
    lodRenderClipMasks: Map<string, Uint8Array>;
    prepareLodChunkBlockedByActiveCoarser(key: string, chunk: VoxelChunk): void;
  };
  const coarser = createTestChunk(-98, 11, -188, 2, 4, true, true);
  const staleActiveFiner = createTestChunk(-196, 22, -376, 1, 2, true, true);
  const preparedReplacement = createTestChunk(-196, 22, -376, 1, 2, true, true);

  worldWithInternals.lodChunks.set("L2:-98:11:-188", coarser);
  worldWithInternals.lodChunks.set("L1:-196:22:-376", staleActiveFiner);
  worldWithInternals.staleLodKeys.add("L1:-196:22:-376");

  worldWithInternals.prepareLodChunkBlockedByActiveCoarser("L1:-196:22:-376", preparedReplacement);

  const punched = worldWithInternals.lodChunks.get("L2:-98:11:-188");
  expect(worldWithInternals.preparedLodChunks.get("L1:-196:22:-376")).toBe(preparedReplacement);
  expect(punched).toBeDefined();
  expect(worldWithInternals.coveragePunchedLodKeys.has("L2:-98:11:-188")).toBe(true);
  const clipMask = expectClipMask(worldWithInternals, "L2:-98:11:-188");
  for (let localZ = 0; localZ < CHUNK_SIZE / 2; localZ += 1) {
    for (let localX = 0; localX < CHUNK_SIZE / 2; localX += 1) {
      expect(columnHasVisibleMaterialInYRange(punched!, clipMask, localX, localZ, 0, CHUNK_SIZE / 2)).toBe(false);
      expect(columnHasVisibleMaterialInYRange(punched!, clipMask, localX, localZ, CHUNK_SIZE / 2, CHUNK_SIZE)).toBe(true);
    }
    expect(columnHasVisibleMaterial(punched!, clipMask, CHUNK_SIZE / 2, localZ)).toBe(true);
  }
});

test("prepared same-key replacement swaps into an already owned finer slot", () => {
  const world = new ProceduralResidentWorld(new ProceduralWorldGenerator(1337), { horizontalRadiusChunks: 1 });
  const worldWithInternals = world as unknown as {
    lodChunks: Map<string, VoxelChunk>;
    preparedLodChunks: Map<string, VoxelChunk>;
    staleLodKeys: Set<string>;
    coveragePunchedLodKeys: Set<string>;
    lodRenderClipMasks: Map<string, Uint8Array>;
    commitPreparedLodChunks(): void;
  };
  const activeFiner = createTestChunkWithVerticalRange(-196, 22, -376, 1, 2, true, 0, CHUNK_SIZE / 2);
  const preparedReplacement = createTestChunkWithVerticalRange(-196, 22, -376, 1, 2, true, CHUNK_SIZE / 2, CHUNK_SIZE);
  const partiallyPunchedCoarser = createTestChunk(-98, 11, -188, 2, 4, true, true);

  worldWithInternals.lodChunks.set("L1:-196:22:-376", activeFiner);
  worldWithInternals.staleLodKeys.add("L1:-196:22:-376");
  worldWithInternals.preparedLodChunks.set("L1:-196:22:-376", preparedReplacement);
  worldWithInternals.lodChunks.set("L2:-98:11:-188", partiallyPunchedCoarser);
  worldWithInternals.coveragePunchedLodKeys.add("L2:-98:11:-188");

  worldWithInternals.commitPreparedLodChunks();

  expect(worldWithInternals.preparedLodChunks.size).toBe(0);
  expect(worldWithInternals.lodChunks.get("L1:-196:22:-376")).toBe(preparedReplacement);
  expect(worldWithInternals.staleLodKeys.has("L1:-196:22:-376")).toBe(false);
  const punched = worldWithInternals.lodChunks.get("L2:-98:11:-188");
  expect(punched).toBeDefined();
  expect(worldWithInternals.coveragePunchedLodKeys.has("L2:-98:11:-188")).toBe(true);
  const clipMask = expectClipMask(worldWithInternals, "L2:-98:11:-188");
  for (let localZ = 0; localZ < CHUNK_SIZE / 2; localZ += 1) {
    for (let localX = 0; localX < CHUNK_SIZE / 2; localX += 1) {
      expect(columnHasVisibleMaterialInYRange(punched!, clipMask, localX, localZ, CHUNK_SIZE / 4, CHUNK_SIZE / 2)).toBe(false);
    }
  }
  expect(columnHasVisibleMaterialInYRange(punched!, clipMask, 0, 0, 0, CHUNK_SIZE / 4)).toBe(true);
  expect(columnHasVisibleMaterialInYRange(punched!, clipMask, CHUNK_SIZE / 2, 0, CHUNK_SIZE / 4, CHUNK_SIZE / 2)).toBe(true);
});

test("prepared finer backlog repairs active coarser chunks refreshed after the finer became active", () => {
  const world = new ProceduralResidentWorld(new ProceduralWorldGenerator(1337), { horizontalRadiusChunks: 1 });
  const worldWithInternals = world as unknown as {
    lodChunks: Map<string, VoxelChunk>;
    preparedLodChunks: Map<string, VoxelChunk>;
    coveragePunchedLodKeys: Set<string>;
    lodRenderClipMasks: Map<string, Uint8Array>;
    punchActiveCoarserLodChunksCoveredByPreparedActiveFiner(): void;
  };
  const activeFiner = createTestChunk(-196, 22, -376, 1, 2, true, true);
  const preparedReplacement = createTestChunk(-196, 22, -376, 1, 2, true, true);
  const refreshedCoarser = createTestChunk(-98, 11, -188, 2, 4, true, true);

  worldWithInternals.lodChunks.set("L1:-196:22:-376", activeFiner);
  worldWithInternals.preparedLodChunks.set("L1:-196:22:-376", preparedReplacement);
  worldWithInternals.lodChunks.set("L2:-98:11:-188", refreshedCoarser);

  worldWithInternals.punchActiveCoarserLodChunksCoveredByPreparedActiveFiner();

  const punched = worldWithInternals.lodChunks.get("L2:-98:11:-188");
  expect(punched).toBeDefined();
  expect(worldWithInternals.coveragePunchedLodKeys.has("L2:-98:11:-188")).toBe(true);
  const clipMask = expectClipMask(worldWithInternals, "L2:-98:11:-188");
  for (let localZ = 0; localZ < CHUNK_SIZE / 2; localZ += 1) {
    for (let localX = 0; localX < CHUNK_SIZE / 2; localX += 1) {
      expect(columnHasVisibleMaterialInYRange(punched!, clipMask, localX, localZ, 0, CHUNK_SIZE / 2)).toBe(false);
      expect(columnHasVisibleMaterialInYRange(punched!, clipMask, localX, localZ, CHUNK_SIZE / 2, CHUNK_SIZE)).toBe(true);
    }
    expect(columnHasVisibleMaterial(punched!, clipMask, CHUNK_SIZE / 2, localZ)).toBe(true);
  }
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

function createTestChunkWithVerticalRange(
  cx: number,
  cy: number,
  cz: number,
  lodLevel: number,
  voxelStride: number,
  renderReady: boolean,
  minLocalY: number,
  maxLocalYExclusive: number,
): VoxelChunk {
  const chunk = createTestChunk(cx, cy, cz, lodLevel, voxelStride, renderReady);
  let solidCount = 0;
  const chunkArea = CHUNK_SIZE * CHUNK_SIZE;
  for (let localZ = 0; localZ < CHUNK_SIZE; localZ += 1) {
    for (let localX = 0; localX < CHUNK_SIZE; localX += 1) {
      const columnOffset = localX + localZ * chunkArea;
      for (let localY = minLocalY; localY < maxLocalYExclusive; localY += 1) {
        chunk.data[columnOffset + localY * CHUNK_SIZE] = 1;
        solidCount += 1;
      }
    }
  }
  chunk.solidCount = solidCount;
  chunk.solidBounds = solidCount === 0
    ? null
    : { min: [0, minLocalY, 0], max: [CHUNK_SIZE, maxLocalYExclusive, CHUNK_SIZE], dirty: false };
  return chunk;
}

function columnHasVisibleMaterial(
  chunk: VoxelChunk,
  clipMask: Uint8Array | null,
  localX: number,
  localZ: number,
): boolean {
  return columnHasVisibleMaterialInYRange(chunk, clipMask, localX, localZ, 0, CHUNK_SIZE);
}

function columnHasVisibleMaterialInYRange(
  chunk: VoxelChunk,
  clipMask: Uint8Array | null,
  localX: number,
  localZ: number,
  minLocalY: number,
  maxLocalYExclusive: number,
): boolean {
  const chunkArea = CHUNK_SIZE * CHUNK_SIZE;
  for (let localY = minLocalY; localY < maxLocalYExclusive; localY += 1) {
    const index = localX + localY * CHUNK_SIZE + localZ * chunkArea;
    if (chunk.data[index] !== 0 && clipMask?.[index] !== 1) {
      return true;
    }
  }
  return false;
}

function expectClipMask(
  world: { lodRenderClipMasks: Map<string, Uint8Array> },
  key: string,
): Uint8Array {
  const clipMask = world.lodRenderClipMasks.get(key);
  expect(clipMask).toBeDefined();
  return clipMask!;
}
