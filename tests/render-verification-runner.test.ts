import { expect, test } from "bun:test";

import {
  buildCommandManifest,
  buildRenderVerificationRunnerReport,
} from "../scripts/lib/render-verification-runner.ts";

test("render verification runner aggregates artifact gates into a reproducible schema", () => {
  const report = buildRenderVerificationRunnerReport({
    generatedAt: "2026-05-09T00:00:00.000Z",
    label: "unit",
    commit: "abc123",
    paths: {
      routeAtlasReport: "artifacts/route-atlas/run/report.json",
      liveForwardReport: "artifacts/browser-route-trace/run/report.json",
      liveForwardSamples: "artifacts/browser-route-trace/run/benchmark-samples.json",
      lodReport: "artifacts/voxels-browser-game-bench/run/lod-idb-persistence-reload.json",
      viewAtlasReport: "artifacts/view-atlas/run/report.json",
    },
    artifacts: {
      routeAtlasReport: routeAtlasReport(),
      liveForwardReport: liveForwardReport(),
      liveForwardSamples: liveForwardSamples(180, 1 / 60),
      lodReport: lodPersistenceReport(),
      viewAtlasReport: viewAtlasReport(),
    },
  });

  expect(report.schemaVersion).toBe(1);
  expect(report.commandManifest.map((entry) => entry.id)).toEqual([
    "route-atlas",
    "live-forward",
    "lod-persistence",
    "view-atlas",
  ]);
  expect(report.inputs.every((input) => input.present)).toBe(true);
  expect(report.failures).toEqual([]);
  expect(groupIds(report)).toContain("view-visual");
  expect(groupIds(report)).toContain("view-settle");
});

test("render verification runner reports live-forward sample, frame, FPS, and LOD failures", () => {
  const report = buildRenderVerificationRunnerReport({
    generatedAt: "2026-05-09T00:00:00.000Z",
    artifacts: {
      liveForwardReport: liveForwardReport({ sampleHz: 60 }),
      liveForwardSamples: [
        sample(1, 0, 12),
        sample(2, 0.5, 80, { handoffLodHoleCount: 1 }),
        sample(3, 1.0, 90, { lodResidentOverlapCount: 2 }),
      ],
    },
    thresholds: {
      minLiveForwardSamples: 10,
      maxP95FrameMs: 16.67,
      maxFrameMs: 50,
      maxFpsErrorRatio: 0.10,
    },
  });

  const failureIds = report.failures.map((failure) => failure.gateId);
  expect(failureIds).toContain("live_forward.samples_min");
  expect(failureIds).toContain("live_forward.p95_frame_ms");
  expect(failureIds).toContain("live_forward.max_frame_ms");
  expect(failureIds).toContain("live_forward.fps_truth_error_ratio");
  expect(failureIds).toContain("live_forward.lod_handoff_hole_count");
  expect(failureIds).toContain("live_forward.lod_resident_overlap_count");
});

test("render verification runner keeps visual and full-settle view gates separate", () => {
  const report = buildRenderVerificationRunnerReport({
    generatedAt: "2026-05-09T00:00:00.000Z",
    artifacts: {
      viewAtlasReport: {
        generatedAt: "2026-05-09T00:00:00.000Z",
        views: [
          {
            id: "blank-unsettled",
            settled: { settled: false },
            snapshot: { lodPendingChunks: 3 },
            visual: {
              diagnosis: {
                blankish: true,
              },
            },
          },
        ],
      },
    },
  });

  const visualGroup = report.gateGroups.find((group) => group.id === "view-visual");
  const settleGroup = report.gateGroups.find((group) => group.id === "view-settle");

  expect(visualGroup?.failures.map((failure) => failure.gateId)).toEqual(["view.visual_blankish_count"]);
  expect(settleGroup?.failures.map((failure) => failure.gateId)).toEqual([
    "view.settle_unsettled_count",
    "view.settle_pending_lod_count",
  ]);
});

test("render verification runner reports far LOD unsettlement as a warning", () => {
  const report = buildRenderVerificationRunnerReport({
    generatedAt: "2026-05-09T00:00:00.000Z",
    artifacts: {
      lodReport: lodPersistenceReport({
        farEviction: {
          label: "far-eviction",
          settled: false,
          finalLodPendingChunks: 967,
        },
      }),
    },
  });

  const lodGroup = report.gateGroups.find((group) => group.id === "lod-persistence");
  const farSettleGate = lodGroup?.gates.find((gate) => gate.id === "lod_persistence.far_unsettled_count");

  expect(farSettleGate?.status).toBe("warn");
  expect(farSettleGate?.value).toBe(1);
  expect(farSettleGate?.details).toEqual({ maxPendingChunks: 967 });
  expect(report.failures).toEqual([]);
  expect(lodGroup?.gates.filter((gate) => gate.status === "warn").map((gate) => gate.id)).toEqual([
    "lod_persistence.far_unsettled_count",
  ]);
});

test("command manifest preserves explicit artifact paths and runnable commands", () => {
  const manifest = buildCommandManifest({
    routeAtlasReport: "route/report.json",
    liveForwardReport: "live/report.json",
    liveForwardSamples: "live/samples.json",
    lodReport: "lod/report.json",
    viewAtlasReport: "view/report.json",
  }, "render-check");

  expect(manifest[0]?.command).toEqual(["bun", "run", "atlas:routes", "--", "--label=render-check"]);
  expect(manifest[1]?.command).toContain("--benchmark=live-forward");
  expect(manifest[1]?.samplePath).toBe("live/samples.json");
  expect(manifest.every((entry) => entry.present)).toBe(true);
});

function groupIds(report: ReturnType<typeof buildRenderVerificationRunnerReport>): string[] {
  return report.gateGroups.map((group) => group.id);
}

function routeAtlasReport() {
  return {
    generatedAt: "2026-05-09T00:00:00.000Z",
    label: "routes",
    commit: "abc123",
    aggregate: {
      sampleCount: 320,
    },
    failures: [],
  };
}

function liveForwardReport(options: { readonly sampleHz?: number } = {}) {
  return {
    generatedAt: "2026-05-09T00:00:00.000Z",
    label: "live",
    commit: "abc123",
    benchmarkSamplesPath: "artifacts/browser-route-trace/run/benchmark-samples.json",
    routeOptions: {
      sampleHz: options.sampleHz ?? 60,
    },
    benchmark: {
      sampleCount: 180,
      summary: {
        sampleHz: options.sampleHz ?? 60,
      },
    },
  };
}

function liveForwardSamples(count: number, stepSeconds: number) {
  return Array.from({ length: count }, (_, index) => sample(index + 1, index * stepSeconds, 8));
}

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

function lodPersistenceReport(options: {
  farEviction?: Record<string, unknown>;
} = {}) {
  const coverage = {
    uncoveredGapCount: 0,
    handoffHoleCount: 0,
    residentOverlapCount: 0,
    bandOverlapCount: 0,
    waterOverlapCount: 0,
  };
  const phase = {
    label: "reload-origin",
    finalCoverage: coverage,
  };
  const farEviction = {
    ...phase,
    ...options.farEviction,
    finalCoverage: coverage,
  };
  return {
    generatedAt: "2026-05-09T00:00:00.000Z",
    commit: "abc123",
    iterations: [
      {
        coldOrigin: phase,
        farEviction,
        storeFlush: phase,
        reloadOrigin: phase,
        failures: [],
      },
    ],
    aggregate: {
      failureCount: 0,
      iterationCount: 1,
    },
  };
}

function viewAtlasReport() {
  return {
    generatedAt: "2026-05-09T00:00:00.000Z",
    label: "views",
    commit: "abc123",
    views: [
      {
        id: "origin-overlook",
        settled: {
          settled: true,
        },
        snapshot: {
          lodPendingChunks: 0,
        },
        visual: {
          diagnosis: {
            blankish: false,
          },
        },
      },
    ],
  };
}
