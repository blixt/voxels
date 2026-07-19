import { describe, expect, it } from "vite-plus/test";
import { SNAPSHOT } from "./engine.ts";
import { summarizeRenderPhase, type RenderSnapshotCapture } from "./render-metrics.ts";

describe("render metrics", () => {
  it("summarizes frame and GPU telemetry with one shared schema", () => {
    const snapshot = Array.from({ length: SNAPSHOT.droppedSamples + 1 }, () => 0);
    snapshot[SNAPSHOT.residentChunks] = 12;
    snapshot[SNAPSHOT.quads] = 345;
    const capture: RenderSnapshotCapture = {
      snapshot,
      samples: [[10, 5, 1, 1, 1, 42, 0.2, 0.3, 0.1, 20, 10]],
      dropped: 0,
      gpuSamples: [
        {
          frameId: 42,
          total: 4,
          shadow: 0.5,
          shadowCascade0: 0.1,
          shadowCascade1: 0.2,
          shadowCascade2: 0.2,
          depthPrepass: 0.2,
          ambientOcclusion: 0.4,
          world: 1.5,
          water: 0.2,
          cloud: 0.6,
          weather: 0.1,
          ui: 0.1,
        },
      ],
      gpuDropped: 0,
    };

    const summary = summarizeRenderPhase([capture]);
    expect(summary.frameMs.p95).toBe(10);
    expect(summary.cpuMs.p95).toBe(5);
    expect(summary.gpu.totalMs?.p95).toBe(4);
    expect(summary.gpu.worldMs?.p95).toBe(1.5);
    expect(summary.gpu.frameCoverage).toBe(1);
    expect(summary.residentChunks).toBe(12);
    expect(summary.quads).toBe(345);
  });
});
