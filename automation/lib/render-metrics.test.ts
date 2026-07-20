import { describe, expect, it } from "vite-plus/test";
import { SNAPSHOT } from "./engine.ts";
import {
  frameSamples,
  summarizeRenderPhase,
  type RenderSnapshotCapture,
} from "./render-metrics.ts";

describe("render metrics", () => {
  it("summarizes frame and GPU telemetry with one shared schema", () => {
    const snapshot = Array.from({ length: SNAPSHOT.droppedSamples + 1 }, () => 0);
    snapshot[SNAPSHOT.residentChunks] = 12;
    snapshot[SNAPSHOT.quads] = 345;
    const capture: RenderSnapshotCapture = {
      snapshot,
      samples: [
        {
          intervalMs: 10,
          cpuMs: 5,
          simulationMs: 1,
          streamingMs: 1,
          renderSubmissionMs: 1,
          frameId: 42,
          renderCullMs: 0.2,
          renderEncodeMs: 0.3,
          renderSubmitMs: 0.1,
          testedSlices: 20,
          selectedSlices: 10,
        },
      ],
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

  it("attributes delayed GPU timings only to frames captured in the measured phase", () => {
    const snapshot = Array.from({ length: SNAPSHOT.droppedSamples + 1 }, () => 0);
    const capture: RenderSnapshotCapture = {
      snapshot,
      samples: [
        {
          intervalMs: 8,
          cpuMs: 2,
          simulationMs: 0.2,
          streamingMs: 0.3,
          renderSubmissionMs: 0.5,
          frameId: 8,
          renderCullMs: 0.1,
          renderEncodeMs: 0.2,
          renderSubmitMs: 0.1,
          testedSlices: 20,
          selectedSlices: 10,
        },
      ],
      dropped: 0,
      gpuSamples: [
        {
          frameId: 7,
          total: 100,
          shadow: 100,
          shadowCascade0: 100,
          shadowCascade1: 100,
          shadowCascade2: 100,
          depthPrepass: 100,
          ambientOcclusion: 100,
          world: 100,
          water: 100,
          cloud: 100,
          weather: 100,
          ui: 100,
        },
        {
          frameId: 8,
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
    expect(summary.gpu.samples).toBe(1);
    expect(summary.gpu.totalMs?.p95).toBe(4);
    expect(summary.gpu.frameCoverage).toBe(1);
  });

  it("decodes the Rust-owned frame sample layout once for every scenario", () => {
    const snapshot = Array.from({ length: SNAPSHOT.droppedSamples + 1 + 11 }, () => 0);
    snapshot[SNAPSHOT.sampleCount] = 1;
    snapshot[SNAPSHOT.droppedSamples + 1] = 8.25;
    snapshot[SNAPSHOT.droppedSamples + 6] = 123;

    expect(frameSamples(snapshot)).toMatchObject([{ intervalMs: 8.25, frameId: 123 }]);
  });
});
