import { expect, test } from "bun:test";

import { summarizeBootstrapBenchmark } from "../src/engine/game-bootstrap-benchmark.ts";

test("bootstrap benchmark summary captures readiness, totals, and dropped frames", () => {
  const summary = summarizeBootstrapBenchmark([
    {
      frame: 0,
      elapsedMs: 5,
      gameplayFrameMs: 5,
      renderCpuMs: 0,
      renderSyncMs: 0,
      renderUploadMs: 0,
      renderEncodeMs: 0,
      uploadChunks: 0,
      uploadBytes: 0,
      drawCalls: 0,
      triangles: 0,
      streamMs: 5,
      meshMs: 0,
      lodMs: 0,
      lodYRangeMs: 0,
      lodDownsampleMs: 0,
      lodMeshMs: 0,
      pendingChunks: 8,
      pendingMeshJobs: 4,
      dirtyResidentChunks: 6,
      dirtyMeshlessResidentChunks: 6,
      dirtyRetainedMeshResidentChunks: 0,
      generatedChunks: 8,
      evictedChunks: 0,
      playableReady: false,
      visualReady: false,
      lodChunkCount: 0,
      lodPendingChunks: 10,
      lodComplete: false,
      frustumCulledChunks: 0,
      fogCulledChunks: 0,
      lodDrawCalls: 0,
    },
    {
      frame: 1,
      elapsedMs: 22,
      gameplayFrameMs: 17,
      renderCpuMs: 2,
      renderSyncMs: 0.5,
      renderUploadMs: 0.2,
      renderEncodeMs: 0.3,
      uploadChunks: 2,
      uploadBytes: 1024,
      drawCalls: 4,
      triangles: 120,
      streamMs: 8,
      meshMs: 4,
      lodMs: 3,
      lodYRangeMs: 0.5,
      lodDownsampleMs: 1,
      lodMeshMs: 1.5,
      pendingChunks: 2,
      pendingMeshJobs: 1,
      dirtyResidentChunks: 1,
      dirtyMeshlessResidentChunks: 1,
      dirtyRetainedMeshResidentChunks: 0,
      generatedChunks: 4,
      evictedChunks: 0,
      playableReady: false,
      visualReady: false,
      lodChunkCount: 5,
      lodPendingChunks: 8,
      lodComplete: false,
      frustumCulledChunks: 1,
      fogCulledChunks: 2,
      lodDrawCalls: 3,
    },
    {
      frame: 2,
      elapsedMs: 41,
      gameplayFrameMs: 19,
      renderCpuMs: 3,
      renderSyncMs: 0.6,
      renderUploadMs: 0.4,
      renderEncodeMs: 0.5,
      uploadChunks: 1,
      uploadBytes: 2048,
      drawCalls: 6,
      triangles: 240,
      streamMs: 6,
      meshMs: 5,
      lodMs: 4,
      lodYRangeMs: 0.8,
      lodDownsampleMs: 1.2,
      lodMeshMs: 2,
      pendingChunks: 0,
      pendingMeshJobs: 0,
      dirtyResidentChunks: 0,
      dirtyMeshlessResidentChunks: 0,
      dirtyRetainedMeshResidentChunks: 0,
      generatedChunks: 1,
      evictedChunks: 0,
      playableReady: true,
      visualReady: false,
      lodChunkCount: 20,
      lodPendingChunks: 5,
      lodComplete: false,
      frustumCulledChunks: 2,
      fogCulledChunks: 4,
      lodDrawCalls: 8,
    },
    {
      frame: 3,
      elapsedMs: 58,
      gameplayFrameMs: 17,
      renderCpuMs: 2.5,
      renderSyncMs: 0.4,
      renderUploadMs: 0.1,
      renderEncodeMs: 0.2,
      uploadChunks: 0,
      uploadBytes: 0,
      drawCalls: 8,
      triangles: 280,
      streamMs: 1,
      meshMs: 1,
      lodMs: 2,
      lodYRangeMs: 0.2,
      lodDownsampleMs: 0.8,
      lodMeshMs: 1,
      pendingChunks: 0,
      pendingMeshJobs: 0,
      dirtyResidentChunks: 0,
      dirtyMeshlessResidentChunks: 0,
      dirtyRetainedMeshResidentChunks: 0,
      generatedChunks: 0,
      evictedChunks: 0,
      playableReady: true,
      visualReady: true,
      lodChunkCount: 30,
      lodPendingChunks: 0,
      lodComplete: true,
      frustumCulledChunks: 3,
      fogCulledChunks: 5,
      lodDrawCalls: 12,
    },
  ]);

  expect(summary.sampleCount).toBe(4);
  expect(summary.totalElapsedMs).toBe(58);
  expect(summary.playableReadyElapsedMs).toBe(41);
  expect(summary.visualReadyElapsedMs).toBe(58);
  expect(summary.totalGameplayFrameMs).toBe(58);
  expect(summary.totalStreamMs).toBe(20);
  expect(summary.totalMeshMs).toBe(10);
  expect(summary.totalLodMs).toBe(9);
  expect(summary.totalLodYRangeMs).toBeCloseTo(1.5, 6);
  expect(summary.totalLodDownsampleMs).toBe(3);
  expect(summary.totalLodMeshMs).toBe(4.5);
  expect(summary.maxLodMs).toBe(4);
  expect(summary.totalRenderCpuMs).toBe(7.5);
  expect(summary.totalRenderSyncMs).toBe(1.5);
  expect(summary.totalRenderUploadMs).toBeCloseTo(0.7, 6);
  expect(summary.totalRenderEncodeMs).toBe(1);
  expect(summary.totalUploadChunks).toBe(3);
  expect(summary.totalUploadBytes).toBe(3072);
  expect(summary.totalGeneratedChunks).toBe(13);
  expect(summary.maxPendingChunks).toBe(8);
  expect(summary.maxPendingMeshJobs).toBe(4);
  expect(summary.maxFogCulledChunks).toBe(5);
  expect(summary.framesOver16_67Ms).toBe(3);
  expect(summary.framesOver33_33Ms).toBe(0);
  expect(summary.maxGameplayFrameMs).toBe(19);
});
