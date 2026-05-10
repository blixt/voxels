import { expect, test } from "bun:test";

import {
  findViewAtlasComparisonBudgetFailures,
  type ViewAtlasComparison,
} from "../scripts/lib/view-atlas-budgets.ts";

test("view atlas comparison budgets pass small visual deltas", () => {
  const comparison: ViewAtlasComparison = {
    baselinePath: "baseline.json",
    baselineGeneratedAt: "2026-05-08T00:00:00.000Z",
    viewDeltas: [
      {
        id: "origin-overlook",
        baselinePresent: true,
        avgLuma: -2.5,
        lumaStdDev: 0.4,
        quantizedColorCount: 3,
        horizonGridRiskScore: 0.003,
        centerGridRiskScore: 0.002,
        lowerGroundGridRiskScore: 0.001,
      },
    ],
  };

  expect(findViewAtlasComparisonBudgetFailures(comparison)).toEqual([]);
});

test("view atlas comparison budgets flag missing baselines and grid regressions", () => {
  const comparison: ViewAtlasComparison = {
    baselinePath: "baseline.json",
    baselineGeneratedAt: null,
    viewDeltas: [
      {
        id: "missing-view",
        baselinePresent: false,
        avgLuma: 0,
        lumaStdDev: 0,
        quantizedColorCount: 0,
        horizonGridRiskScore: 0,
        centerGridRiskScore: 0,
        lowerGroundGridRiskScore: 0,
      },
      {
        id: "approach",
        baselinePresent: true,
        avgLuma: -20,
        lumaStdDev: -1,
        quantizedColorCount: -20,
        horizonGridRiskScore: 0.012,
        centerGridRiskScore: 0.009,
        lowerGroundGridRiskScore: 0.006,
      },
    ],
  };

  expect(findViewAtlasComparisonBudgetFailures(comparison)).toEqual([
    "missing-view is missing from baseline baseline.json",
    "approach luma dropped -20.0, budget -18.0",
    "approach color buckets changed -20, budget -16",
    "approach horizon grid regressed +0.012, budget +0.010",
    "approach center grid regressed +0.009, budget +0.006",
    "approach lower-ground grid regressed +0.006, budget +0.005",
  ]);
});
