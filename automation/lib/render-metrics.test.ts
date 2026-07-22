import { describe, expect, it } from "vite-plus/test";
import { FRAME_SAMPLE_WIDTH, SNAPSHOT } from "./engine.ts";
import {
  frameSamples,
  summarizeRenderPhase,
  summarizeStreamingPressure,
  type RenderSnapshotCapture,
} from "./render-metrics.ts";

describe("render metrics", () => {
  it("summarizes frame and GPU telemetry with one shared schema", () => {
    const snapshot = Array.from({ length: SNAPSHOT.droppedSamples + 1 }, () => 0);
    snapshot[SNAPSHOT.residentChunks] = 12;
    snapshot[SNAPSHOT.quads] = 345;
    const capture: RenderSnapshotCapture = {
      capturedAtMs: 10,
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
          renderLodPlanMs: 0.05,
          lodPlanRebuildReason: 2,
          renderEncodeMs: 0.3,
          renderSubmitMs: 0.1,
          lodOwnershipRefreshes: 2,
          testedSlices: 20,
          selectedSlices: 10,
          streamRemoteMs: 0.1,
          streamPlanMs: 0.2,
          streamMeshMs: 0.3,
          streamPublishMs: 0.1,
          streamSurfaceMs: 0.2,
          streamPresenceMs: 0.1,
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
      capturedAtMs: 10,
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
          renderLodPlanMs: 0,
          lodPlanRebuildReason: 0,
          renderEncodeMs: 0.2,
          renderSubmitMs: 0.1,
          lodOwnershipRefreshes: 0,
          testedSlices: 20,
          selectedSlices: 10,
          streamRemoteMs: 0.05,
          streamPlanMs: 0.05,
          streamMeshMs: 0.05,
          streamPublishMs: 0.05,
          streamSurfaceMs: 0.05,
          streamPresenceMs: 0.05,
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
    const snapshot = Array.from(
      { length: SNAPSHOT.droppedSamples + 1 + FRAME_SAMPLE_WIDTH },
      () => 0,
    );
    snapshot[SNAPSHOT.sampleCount] = 1;
    snapshot[SNAPSHOT.droppedSamples + 1] = 8.25;
    snapshot[SNAPSHOT.droppedSamples + 6] = 123;

    expect(frameSamples(snapshot)).toMatchObject([{ intervalMs: 8.25, frameId: 123 }]);
  });

  it("reports stage pressure and continuous presentation degradation", () => {
    const snapshot = Array.from({ length: SNAPSHOT.droppedSamples + 1 }, () => 0);
    snapshot[SNAPSHOT.pendingJobs] = 7;
    snapshot[SNAPSHOT.generationQueued] = 3;
    snapshot[SNAPSHOT.generationInFlight] = 2;
    snapshot[SNAPSHOT.surfaceInFlight] = 2;
    snapshot[SNAPSHOT.loadCompleted] = 10;
    snapshot[SNAPSHOT.acceptedCompletions] = 20;
    snapshot[SNAPSHOT.canonicalImmediateRequired] = 15;
    snapshot[SNAPSHOT.canonicalImmediateResident] = 14;
    snapshot[SNAPSHOT.collisionImmediateRequired] = 8;
    snapshot[SNAPSHOT.collisionImmediateResident] = 8;
    snapshot[SNAPSHOT.collisionLookaheadRequired] = 20;
    snapshot[SNAPSHOT.collisionLookaheadResident] = 19;
    snapshot[SNAPSHOT.canonicalSurfaceCellsRequired] = 1_024;
    snapshot[SNAPSHOT.canonicalSurfaceCellsResident] = 1_024;
    snapshot[SNAPSHOT.presentedLodStrideVoxels] = 2;
    const settled = [...snapshot];
    settled[SNAPSHOT.pendingJobs] = 0;
    settled[SNAPSHOT.generationQueued] = 0;
    settled[SNAPSHOT.generationInFlight] = 0;
    settled[SNAPSHOT.surfaceInFlight] = 0;
    settled[SNAPSHOT.loadCompleted] = 13;
    settled[SNAPSHOT.acceptedCompletions] = 25;
    settled[SNAPSHOT.canonicalImmediateResident] = 15;
    settled[SNAPSHOT.collisionLookaheadResident] = 20;
    settled[SNAPSHOT.presentedLodStrideVoxels] = 1;
    const capture = (capturedAtMs: number, values: readonly number[]): RenderSnapshotCapture => ({
      capturedAtMs,
      snapshot: values,
      samples: [],
      dropped: 0,
      gpuSamples: [],
      gpuDropped: 0,
    });

    const pressure = summarizeStreamingPressure([
      capture(10, snapshot),
      capture(260, snapshot),
      capture(510, settled),
    ]);
    expect(pressure.pendingJobs.max).toBe(7);
    expect(pressure.generation.queued.max).toBe(3);
    expect(pressure.surface.inFlight.max).toBe(2);
    expect(pressure.completions.initialLoads).toBe(3);
    expect(pressure.completions.accepted).toBe(5);
    expect(pressure.readiness.canonicalImmediateRatio).toBeCloseTo(1 / 3);
    expect(pressure.readiness.canonicalPresentationRatio).toBeCloseTo(1 / 3);
    expect(pressure.readiness.collisionImmediateRatio).toBe(1);
    expect(pressure.readiness.collisionLookaheadRatio).toBeCloseTo(1 / 3);
    expect(pressure.readiness.longestDegradedPresentationMs).toBe(500);
    expect(pressure.readiness.longestCollisionImmediateGapMs).toBe(0);
    expect(pressure.readiness.longestCollisionLookaheadGapMs).toBe(500);
  });
});
