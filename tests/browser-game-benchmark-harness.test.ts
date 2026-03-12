import { expect, test } from "bun:test";

import { summarizeMemorySamples } from "../scripts/lib/browser-game-benchmark-harness.ts";

test("summarizeMemorySamples reports peaks and metric deltas", () => {
  const summary = summarizeMemorySamples([
    {
      elapsedMs: 10,
      jsHeapUsedSizeBytes: 100,
      jsHeapTotalSizeBytes: 200,
      runtimeHeapUsedBytes: 300,
      runtimeHeapTotalBytes: 400,
      taskDurationSeconds: 1,
      scriptDurationSeconds: 0.4,
      layoutDurationSeconds: 0.1,
      recalcStyleDurationSeconds: 0.05,
      documents: 1,
      nodes: 10,
      residentChunks: 12,
      pendingChunks: 8,
      dirtyResidentChunks: 5,
      farFieldPendingBands: 2,
      solidVoxelCount: 1000,
    },
    {
      elapsedMs: 40,
      jsHeapUsedSizeBytes: 160,
      jsHeapTotalSizeBytes: 240,
      runtimeHeapUsedBytes: 480,
      runtimeHeapTotalBytes: 640,
      taskDurationSeconds: 1.35,
      scriptDurationSeconds: 0.55,
      layoutDurationSeconds: 0.16,
      recalcStyleDurationSeconds: 0.08,
      documents: 1,
      nodes: 10,
      residentChunks: 20,
      pendingChunks: 3,
      dirtyResidentChunks: 2,
      farFieldPendingBands: 1,
      solidVoxelCount: 1800,
    },
  ]);

  expect(summary.sampleCount).toBe(2);
  expect(summary.firstElapsedMs).toBe(10);
  expect(summary.lastElapsedMs).toBe(40);
  expect(summary.peakJsHeapUsedSizeBytes).toBe(160);
  expect(summary.peakRuntimeHeapUsedBytes).toBe(480);
  expect(summary.peakResidentChunks).toBe(20);
  expect(summary.peakPendingChunks).toBe(8);
  expect(summary.deltaTaskDurationMs).toBe(350);
  expect(summary.deltaScriptDurationMs).toBe(150);
  expect(summary.deltaLayoutDurationMs).toBe(60);
  expect(summary.deltaRecalcStyleDurationMs).toBe(30);
});
