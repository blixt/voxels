import { expect, test } from "bun:test";

import {
  buildRenderVerificationReport,
  evaluateFpsTruth,
  evaluateLodCoverage,
  evaluateVisualIdentity,
  summarizeFrameDeltas,
} from "../scripts/lib/render-verification-metrics.ts";

test("visual identity metrics flag blankish frames", () => {
  const scenario = evaluateVisualIdentity("static-origin", {
    avgLuma: 3,
    lumaStdDev: 2,
    quantizedColorCount: 4,
  });

  expect(scenario.failures.map((failure) => failure.metricId)).toEqual([
    "visual.avg_luma",
    "visual.luma_stddev",
    "visual.quantized_color_count",
  ]);
});

test("LOD coverage metrics require zero gaps and overlaps", () => {
  const scenario = evaluateLodCoverage("lod-boundary", {
    uncoveredGapCount: 0,
    handoffHoleCount: 1,
    residentOverlapCount: 0,
    bandOverlapCount: 2,
    waterOverlapCount: 0,
  });

  expect(scenario.failures.map((failure) => failure.metricId)).toEqual([
    "lod.handoff_hole_count",
    "lod.band_overlap_count",
  ]);
});

test("frame delta summary computes useful hitch buckets", () => {
  const summary = summarizeFrameDeltas([16, 17, 18, 67, 10]);

  expect(summary.frameCount).toBe(5);
  expect(summary.medianMs).toBe(17);
  expect(summary.p95Ms).toBe(67);
  expect(summary.maxMs).toBe(67);
  expect(summary.framesOver50Ms).toBe(1);
  expect(summary.droppedFrameEstimate).toBeGreaterThan(0);
});

test("FPS truth metric fails when reported FPS disagrees with RAF cadence", () => {
  const scenario = evaluateFpsTruth("route-smoke", 60, [33.33, 33.33, 33.33, 33.33]);

  expect(scenario.failures).toHaveLength(1);
  expect(scenario.failures[0]?.metricId).toBe("fps.error_ratio");
});

test("render verification report flattens scenario failures", () => {
  const scenario = evaluateLodCoverage("lod-boundary", {
    uncoveredGapCount: 1,
    handoffHoleCount: 0,
    residentOverlapCount: 0,
    bandOverlapCount: 0,
    waterOverlapCount: 0,
  });
  const report = buildRenderVerificationReport({
    generatedAt: "2026-05-09T00:00:00.000Z",
    label: "unit",
    commit: "abc123",
    scenarios: [scenario],
  });

  expect(report.failures).toHaveLength(1);
  expect(report.failures[0]?.scenarioId).toBe("lod-boundary");
});
