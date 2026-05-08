import { expect, test } from "bun:test";

import {
  createLodDebugWorld,
  LOD_DEBUG_MATERIAL,
  LodDebugWorld,
  type LodDebugCoverageRequest,
} from "../src/engine/lod-debug-world.ts";

const REQUEST: LodDebugCoverageRequest = {
  centerChunkX: 0,
  centerChunkZ: 0,
  nearRadiusLod1Chunks: 1,
  farRadiusLod1Chunks: 4,
};

test("LOD debug world covers each base chunk column with exactly one active owner", () => {
  const world = new LodDebugWorld();
  world.requestCoverage(REQUEST);
  world.completePending();

  const report = world.reportCoverage(REQUEST);

  expect(report.pendingChunks).toBe(0);
  expect(report.gaps).toBe(0);
  expect(report.overlaps).toBe(0);
  expect(report.activeChunks).toBeGreaterThan(0);
});

test("LOD debug handoff keeps old coverage until replacement chunks are ready", () => {
  const world = new LodDebugWorld();
  world.requestCoverage({ ...REQUEST, nearRadiusLod1Chunks: 0 });
  world.completePending();
  expect(world.reportCoverage({ ...REQUEST, nearRadiusLod1Chunks: 0 }).gaps).toBe(0);

  world.requestCoverage({ ...REQUEST, nearRadiusLod1Chunks: 2 });
  const pendingReport = world.reportCoverage({ ...REQUEST, nearRadiusLod1Chunks: 2 });

  expect(pendingReport.pendingChunks).toBeGreaterThan(0);
  expect(pendingReport.gaps).toBe(0);
  expect(pendingReport.overlaps).toBe(0);

  while (world.reportCoverage({ ...REQUEST, nearRadiusLod1Chunks: 2 }).pendingChunks > 0) {
    world.completePending(1);
    const incrementalReport = world.reportCoverage({ ...REQUEST, nearRadiusLod1Chunks: 2 });
    expect(incrementalReport.gaps).toBe(0);
    expect(incrementalReport.overlaps).toBe(0);
  }

  const finalReport = world.reportCoverage({ ...REQUEST, nearRadiusLod1Chunks: 2 });
  expect(finalReport.pendingChunks).toBe(0);
  expect(finalReport.gaps).toBe(0);
  expect(finalReport.overlaps).toBe(0);
});

test("LOD debug staging keeps visible coverage stable while preparing the next area", () => {
  const world = new LodDebugWorld();
  const visibleRequest = { ...REQUEST, centerChunkX: 0, centerChunkZ: 0, nearRadiusLod1Chunks: 1 };
  const nextRequest = { ...REQUEST, centerChunkX: 6, centerChunkZ: -4, nearRadiusLod1Chunks: 2 };
  world.requestCoverage(visibleRequest);
  world.completePending();

  world.prepareCoverage(nextRequest);
  expect(world.reportCoverage(visibleRequest).gaps).toBe(0);
  expect(world.reportCoverage(visibleRequest).overlaps).toBe(0);

  while (world.reportCoverage(nextRequest).pendingChunks > 0) {
    world.completePending(1);
    expect(world.reportCoverage(visibleRequest).gaps).toBe(0);
    expect(world.reportCoverage(visibleRequest).overlaps).toBe(0);
  }

  world.requestCoverage(nextRequest);
  const finalReport = world.reportCoverage(nextRequest);
  expect(finalReport.gaps).toBe(0);
  expect(finalReport.overlaps).toBe(0);
});

test("LOD debug edits propagate through the active coarse representation", () => {
  const world = new LodDebugWorld();
  world.requestCoverage({ ...REQUEST, nearRadiusLod1Chunks: 0 });
  world.completePending();

  world.setEdit(64, 6, 64, LOD_DEBUG_MATERIAL.edited);

  expect(world.sampleActiveMaterialAtWorldVoxel(64, 6, 64)).toBe(LOD_DEBUG_MATERIAL.edited);
});

test("LOD debug world is renderable through resident chunk iteration", () => {
  const world = createLodDebugWorld();
  const chunks = [...world.iterateResidentChunks()];

  expect(chunks.length).toBeGreaterThan(0);
  expect(chunks.every((chunk) => chunk.renderReady && chunk.meshBuilt && chunk.mesh)).toBe(true);
  expect(chunks.some((chunk) => chunk.lodLevel === 0)).toBe(true);
  expect(chunks.some((chunk) => chunk.lodLevel === 1)).toBe(true);
});
