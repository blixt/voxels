import { expect, test } from "bun:test";

import { buildRenderVerificationRunnerReport } from "../scripts/lib/render-verification-runner.ts";
import {
  buildVoxelRpgVerificationReport,
  classifyVoxelRpgVerificationFailure,
  summarizeVoxelRpgVerificationReport,
} from "../scripts/lib/voxel-rpg-budgets.ts";

test("RPG verification summary separates correctness, performance, settle, and artifact failures", () => {
  const renderReport = buildRenderVerificationRunnerReport({
    generatedAt: "2026-05-09T00:00:00.000Z",
    label: "unit",
    commit: "abc123",
    artifacts: {
      liveForwardReport: {
        routeOptions: { sampleHz: 60 },
      },
      liveForwardSamples: [
        sample(1, 0, 8),
        sample(2, 0.5, 90, { uncoveredLodGapCount: 2 }),
      ],
      viewAtlasReport: {
        views: [
          {
            id: "pending",
            settled: { settled: false },
            snapshot: { lodPendingChunks: 1 },
            visual: { diagnosis: { blankish: false } },
          },
        ],
      },
    },
    thresholds: {
      requireArtifacts: true,
      minLiveForwardSamples: 10,
      maxFrameMs: 50,
    },
  });

  const summary = summarizeVoxelRpgVerificationReport(renderReport);

  expect(summary.status).toBe("fail");
  expect(summary.artifactFailures.count).toBeGreaterThan(0);
  expect(summary.performanceFailures.failures.map((failure) => failure.gateId)).toContain("live_forward.samples_min");
  expect(summary.performanceFailures.failures.map((failure) => failure.gateId)).toContain("live_forward.max_frame_ms");
  expect(summary.correctnessFailures.failures.map((failure) => failure.gateId)).toContain(
    "live_forward.lod_uncovered_gap_count",
  );
  expect(summary.settleFailures.failures.map((failure) => failure.gateId)).toEqual([
    "view.settle_unsettled_count",
    "view.settle_pending_lod_count",
  ]);
});

test("RPG verification report wraps the render gate output with centralized budgets", () => {
  const renderReport = buildRenderVerificationRunnerReport({
    generatedAt: "2026-05-09T00:00:00.000Z",
    label: "unit",
    commit: "abc123",
    artifacts: {},
  });

  const rpgReport = buildVoxelRpgVerificationReport(renderReport);

  expect(rpgReport.schemaVersion).toBe(1);
  expect(rpgReport.generatedAt).toBe(renderReport.generatedAt);
  expect(rpgReport.renderVerification).toBe(renderReport);
  expect(rpgReport.budgets.requireArtifacts).toBe(true);
});

test("RPG verification failure classifier keeps LOD correctness distinct from frame budgets", () => {
  expect(classifyVoxelRpgVerificationFailure({ groupId: "live-forward", gateId: "live_forward.p95_frame_ms" }))
    .toBe("performance");
  expect(classifyVoxelRpgVerificationFailure({ groupId: "live-forward", gateId: "live_forward.lod_band_overlap_count" }))
    .toBe("correctness");
  expect(classifyVoxelRpgVerificationFailure({ groupId: "view-settle", gateId: "view.settle_unsettled_count" }))
    .toBe("settle");
  expect(classifyVoxelRpgVerificationFailure({ groupId: "artifact-inventory", gateId: "artifact.route_atlas_present" }))
    .toBe("artifact");
});

function sample(
  frame: number,
  simTimeSeconds: number,
  gameplayFrameMs: number,
  overrides: Record<string, unknown> = {},
) {
  return {
    frame,
    simTimeSeconds,
    gameplayFrameMs,
    uncoveredLodGapCount: 0,
    handoffLodHoleCount: 0,
    lodResidentOverlapCount: 0,
    lodBandOverlapCount: 0,
    waterOverlapCount: 0,
    ...overrides,
  };
}
