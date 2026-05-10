export interface ViewAtlasComparisonDelta {
  id: string;
  baselinePresent: boolean;
  avgLuma: number;
  lumaStdDev: number;
  quantizedColorCount: number;
  horizonGridRiskScore: number;
  centerGridRiskScore: number;
  lowerGroundGridRiskScore: number;
}

export interface ViewAtlasComparison {
  baselinePath: string;
  baselineGeneratedAt: string | null;
  viewDeltas: ViewAtlasComparisonDelta[];
}

export interface ViewAtlasComparisonBudgetThresholds {
  maxLumaDrop: number;
  minColorCountDelta: number;
  maxHorizonGridRegression: number;
  maxCenterGridRegression: number;
  maxLowerGroundGridRegression: number;
}

export const DEFAULT_VIEW_ATLAS_COMPARISON_BUDGETS: ViewAtlasComparisonBudgetThresholds = {
  maxLumaDrop: 18,
  minColorCountDelta: -16,
  maxHorizonGridRegression: 0.010,
  maxCenterGridRegression: 0.006,
  maxLowerGroundGridRegression: 0.005,
};

export function findViewAtlasComparisonBudgetFailures(
  comparison: ViewAtlasComparison,
  thresholds: ViewAtlasComparisonBudgetThresholds = DEFAULT_VIEW_ATLAS_COMPARISON_BUDGETS,
): string[] {
  const failures: string[] = [];
  for (const delta of comparison.viewDeltas) {
    if (!delta.baselinePresent) {
      failures.push(`${delta.id} is missing from baseline ${comparison.baselinePath}`);
      continue;
    }
    if (delta.avgLuma < -thresholds.maxLumaDrop) {
      failures.push(`${delta.id} luma dropped ${formatSigned(delta.avgLuma, 1)}, budget -${thresholds.maxLumaDrop.toFixed(1)}`);
    }
    if (delta.quantizedColorCount < thresholds.minColorCountDelta) {
      failures.push(`${delta.id} color buckets changed ${formatSigned(delta.quantizedColorCount)}, budget ${thresholds.minColorCountDelta}`);
    }
    if (delta.horizonGridRiskScore > thresholds.maxHorizonGridRegression) {
      failures.push(`${delta.id} horizon grid regressed ${formatSigned(delta.horizonGridRiskScore, 3)}, budget +${thresholds.maxHorizonGridRegression.toFixed(3)}`);
    }
    if (delta.centerGridRiskScore > thresholds.maxCenterGridRegression) {
      failures.push(`${delta.id} center grid regressed ${formatSigned(delta.centerGridRiskScore, 3)}, budget +${thresholds.maxCenterGridRegression.toFixed(3)}`);
    }
    if (delta.lowerGroundGridRiskScore > thresholds.maxLowerGroundGridRegression) {
      failures.push(`${delta.id} lower-ground grid regressed ${formatSigned(delta.lowerGroundGridRiskScore, 3)}, budget +${thresholds.maxLowerGroundGridRegression.toFixed(3)}`);
    }
  }
  return failures;
}

function formatSigned(value: number, digits = 0): string {
  const prefix = value > 0 ? "+" : "";
  return `${prefix}${value.toFixed(digits)}`;
}
