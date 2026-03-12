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
      farFieldMs: 0,
      pendingChunks: 8,
      pendingMeshJobs: 4,
      dirtyResidentChunks: 6,
      dirtyMeshlessResidentChunks: 6,
      dirtyRetainedMeshResidentChunks: 0,
      generatedChunks: 8,
      evictedChunks: 0,
      farFieldPendingBands: 2,
      playableReady: false,
      visualReady: false,
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
      farFieldMs: 1,
      pendingChunks: 2,
      pendingMeshJobs: 1,
      dirtyResidentChunks: 1,
      dirtyMeshlessResidentChunks: 1,
      dirtyRetainedMeshResidentChunks: 0,
      generatedChunks: 4,
      evictedChunks: 0,
      farFieldPendingBands: 1,
      playableReady: false,
      visualReady: false,
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
      farFieldMs: 2,
      pendingChunks: 0,
      pendingMeshJobs: 0,
      dirtyResidentChunks: 0,
      dirtyMeshlessResidentChunks: 0,
      dirtyRetainedMeshResidentChunks: 0,
      generatedChunks: 1,
      evictedChunks: 0,
      farFieldPendingBands: 1,
      playableReady: true,
      visualReady: false,
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
      farFieldMs: 1,
      pendingChunks: 0,
      pendingMeshJobs: 0,
      dirtyResidentChunks: 0,
      dirtyMeshlessResidentChunks: 0,
      dirtyRetainedMeshResidentChunks: 0,
      generatedChunks: 0,
      evictedChunks: 0,
      farFieldPendingBands: 0,
      playableReady: true,
      visualReady: true,
    },
  ]);

  expect(summary.sampleCount).toBe(4);
  expect(summary.totalElapsedMs).toBe(58);
  expect(summary.playableReadyElapsedMs).toBe(41);
  expect(summary.visualReadyElapsedMs).toBe(58);
  expect(summary.totalGameplayFrameMs).toBe(58);
  expect(summary.totalStreamMs).toBe(20);
  expect(summary.totalMeshMs).toBe(10);
  expect(summary.totalFarFieldMs).toBe(4);
  expect(summary.totalRenderCpuMs).toBe(7.5);
  expect(summary.totalRenderSyncMs).toBe(1.5);
  expect(summary.totalRenderUploadMs).toBeCloseTo(0.7, 6);
  expect(summary.totalRenderEncodeMs).toBe(1);
  expect(summary.totalUploadChunks).toBe(3);
  expect(summary.totalUploadBytes).toBe(3072);
  expect(summary.totalGeneratedChunks).toBe(13);
  expect(summary.maxPendingChunks).toBe(8);
  expect(summary.maxPendingMeshJobs).toBe(4);
  expect(summary.framesOver16_67Ms).toBe(3);
  expect(summary.framesOver33_33Ms).toBe(0);
  expect(summary.maxGameplayFrameMs).toBe(19);
});
