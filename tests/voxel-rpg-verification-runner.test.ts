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
  expect(rpgReport.profile.id).toBe("rpg-render-gate");
  expect(rpgReport.rpgGateGroups.map((group) => group.id)).toEqual([
    "rpg-evidence-freshness",
    "rpg-hitch-gates",
  ]);
});

test("RPG verification failure classifier keeps LOD correctness distinct from frame budgets", () => {
  expect(classifyVoxelRpgVerificationFailure({ groupId: "live-forward", gateId: "live_forward.p95_frame_ms" }))
    .toBe("performance");
  expect(classifyVoxelRpgVerificationFailure({ groupId: "rpg-hitch-gates", gateId: "hitch.frames_over_budget" }))
    .toBe("performance");
  expect(classifyVoxelRpgVerificationFailure({ groupId: "live-forward", gateId: "live_forward.lod_band_overlap_count" }))
    .toBe("correctness");
  expect(classifyVoxelRpgVerificationFailure({ groupId: "view-settle", gateId: "view.settle_unsettled_count" }))
    .toBe("settle");
  expect(classifyVoxelRpgVerificationFailure({ groupId: "artifact-inventory", gateId: "artifact.route_atlas_present" }))
    .toBe("artifact");
  expect(classifyVoxelRpgVerificationFailure({ groupId: "rpg-evidence-freshness", gateId: "evidence.stale_artifact_count" }))
    .toBe("artifact");
});

test("RPG verification fails stale and mismatched evidence as artifact failures", () => {
  const liveForwardSamples = Array.from({ length: 120 }, (_, index) => sample(index + 1, index / 60, 8));
  const renderReport = buildRenderVerificationRunnerReport({
    generatedAt: "2026-05-09T12:00:00.000Z",
    label: "unit",
    commit: "new123",
    paths: {
      routeAtlasReport: "route/report.json",
      liveForwardReport: "live/report.json",
      liveForwardSamples: "live/samples.json",
    },
    artifacts: {
      routeAtlasReport: {
        generatedAt: "2026-05-09T09:30:00.000Z",
        commit: "new123",
        aggregate: { sampleCount: 320 },
        failures: [],
      },
      liveForwardReport: {
        generatedAt: "2026-05-09T11:45:00.000Z",
        commit: "old456",
        routeOptions: { sampleHz: 60 },
        benchmark: { sampleCount: 120, summary: { sampleHz: 60 } },
      },
      liveForwardSamples,
    },
  });

  const rpgReport = buildVoxelRpgVerificationReport(renderReport, { liveForwardSamples });
  const evidenceGroup = rpgReport.rpgGateGroups.find((group) => group.id === "rpg-evidence-freshness");

  expect(evidenceGroup?.failures.map((failure) => failure.gateId)).toEqual([
    "evidence.stale_artifact_count",
    "evidence.commit_mismatch_count",
  ]);
  expect(rpgReport.evidenceBundle.artifacts.find((artifact) => artifact.id === "route-atlas")?.status).toBe("stale");
  expect(rpgReport.summary.artifactFailures.failures.map((failure) => failure.gateId)).toEqual(
    expect.arrayContaining(["evidence.stale_artifact_count", "evidence.commit_mismatch_count"]),
  );
});

test("RPG verification aggregates live-forward hitch gates by movement, settle, and LOD work", () => {
  const liveForwardSamples = [
    sample(1, 0, 8, { phase: "move", lodMs: 2, lodMaxChunkMs: 2 }),
    sample(2, 1 / 60, 64, { phase: "move", lodMs: 14, lodMaxChunkMs: 4 }),
    sample(3, 2 / 60, 72, { phase: "settle", lodMs: 3, lodMaxChunkMs: 16 }),
  ];
  const renderReport = buildRenderVerificationRunnerReport({
    generatedAt: "2026-05-09T12:00:00.000Z",
    label: "unit",
    commit: "abc123",
    artifacts: {
      liveForwardReport: {
        generatedAt: "2026-05-09T11:59:00.000Z",
        commit: "abc123",
        routeOptions: { sampleHz: 60 },
      },
      liveForwardSamples,
    },
    thresholds: {
      minLiveForwardSamples: 1,
      maxFrameMs: 100,
      maxP95FrameMs: 100,
    },
  });

  const rpgReport = buildVoxelRpgVerificationReport(renderReport, { liveForwardSamples });
  const hitchGroup = rpgReport.rpgGateGroups.find((group) => group.id === "rpg-hitch-gates");

  expect(rpgReport.hitchSummary).toMatchObject({
    sampleCount: 3,
    hitchFrames: 2,
    movementHitchFrames: 1,
    settleHitchFrames: 1,
    lodWorkHitchFrames: 2,
  });
  expect(hitchGroup?.failures.map((failure) => failure.gateId)).toEqual([
    "hitch.frames_over_budget",
    "hitch.movement_frames_over_budget",
    "hitch.settle_frames_over_budget",
    "hitch.dropped_frame_estimate",
    "hitch.lod_work_frames_over_budget",
  ]);
  expect(rpgReport.summary.performanceFailures.failures.map((failure) => failure.gateId)).toEqual(
    expect.arrayContaining(["hitch.frames_over_budget", "hitch.lod_work_frames_over_budget"]),
  );
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
