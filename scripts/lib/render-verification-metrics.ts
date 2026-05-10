export type RenderMetricStatus = "pass" | "fail" | "warn";

export interface RenderMetric {
  readonly id: string;
  readonly label: string;
  readonly value: number | string | boolean | null;
  readonly unit?: string;
  readonly status: RenderMetricStatus;
  readonly threshold?: string;
  readonly details?: Record<string, unknown>;
}

export interface RenderVerificationFailure {
  readonly scenarioId: string;
  readonly metricId: string;
  readonly message: string;
  readonly value: number | string | boolean | null;
  readonly threshold?: string;
}

export interface RenderScenarioResult {
  readonly id: string;
  readonly label: string;
  readonly metrics: RenderMetric[];
  readonly failures: RenderVerificationFailure[];
  readonly artifacts?: Record<string, string>;
}

export interface RenderVerificationReport {
  readonly generatedAt: string;
  readonly label: string | null;
  readonly commit: string | null;
  readonly scenarios: RenderScenarioResult[];
  readonly failures: RenderVerificationFailure[];
}

export interface VisualIdentityMetrics {
  readonly avgLuma: number;
  readonly lumaStdDev: number;
  readonly quantizedColorCount: number;
}

export interface VisualIdentityThresholds {
  readonly minAvgLuma: number;
  readonly minLumaStdDev: number;
  readonly minQuantizedColorCount: number;
}

export const DEFAULT_VISUAL_IDENTITY_THRESHOLDS: VisualIdentityThresholds = {
  minAvgLuma: 8,
  minLumaStdDev: 6,
  minQuantizedColorCount: 8,
};

export interface LodCoverageMetrics {
  readonly uncoveredGapCount: number;
  readonly handoffHoleCount: number;
  readonly residentOverlapCount: number;
  readonly bandOverlapCount: number;
  readonly waterOverlapCount: number;
}

export interface FrameDeltaSummary {
  readonly frameCount: number;
  readonly avgMs: number;
  readonly medianMs: number;
  readonly p95Ms: number;
  readonly maxMs: number;
  readonly framesOver50Ms: number;
  readonly droppedFrameEstimate: number;
}

export interface FpsTruthMetrics {
  readonly reportedFps: number;
  readonly computedMedianFps: number;
  readonly errorRatio: number;
}

export interface FpsTruthThresholds {
  readonly maxErrorRatio: number;
}

export const DEFAULT_FPS_TRUTH_THRESHOLDS: FpsTruthThresholds = {
  maxErrorRatio: 0.10,
};

export function evaluateVisualIdentity(
  scenarioId: string,
  metrics: VisualIdentityMetrics,
  thresholds: VisualIdentityThresholds = DEFAULT_VISUAL_IDENTITY_THRESHOLDS,
): RenderScenarioResult {
  const renderMetrics: RenderMetric[] = [
    numericMinimumMetric("visual.avg_luma", "Average luma", metrics.avgLuma, thresholds.minAvgLuma),
    numericMinimumMetric("visual.luma_stddev", "Luma standard deviation", metrics.lumaStdDev, thresholds.minLumaStdDev),
    numericMinimumMetric(
      "visual.quantized_color_count",
      "Quantized color count",
      metrics.quantizedColorCount,
      thresholds.minQuantizedColorCount,
    ),
  ];
  return buildScenarioResult(scenarioId, "Visual identity", renderMetrics);
}

export function evaluateLodCoverage(scenarioId: string, metrics: LodCoverageMetrics): RenderScenarioResult {
  const renderMetrics: RenderMetric[] = [
    zeroMetric("lod.uncovered_gap_count", "LOD uncovered gaps", metrics.uncoveredGapCount),
    zeroMetric("lod.handoff_hole_count", "LOD handoff holes", metrics.handoffHoleCount),
    zeroMetric("lod.resident_overlap_count", "Resident/LOD overlaps", metrics.residentOverlapCount),
    zeroMetric("lod.band_overlap_count", "LOD band overlaps", metrics.bandOverlapCount),
    zeroMetric("lod.water_overlap_count", "Water ownership overlaps", metrics.waterOverlapCount),
  ];
  return buildScenarioResult(scenarioId, "LOD coverage", renderMetrics);
}

export function summarizeFrameDeltas(frameDeltasMs: readonly number[], targetFrameMs = 1000 / 60): FrameDeltaSummary {
  const normalized = frameDeltasMs
    .filter((value) => Number.isFinite(value) && value >= 0)
    .slice()
    .sort((left, right) => left - right);
  if (normalized.length === 0) {
    return {
      frameCount: 0,
      avgMs: 0,
      medianMs: 0,
      p95Ms: 0,
      maxMs: 0,
      framesOver50Ms: 0,
      droppedFrameEstimate: 0,
    };
  }
  return {
    frameCount: normalized.length,
    avgMs: average(normalized),
    medianMs: percentileSorted(normalized, 0.5),
    p95Ms: percentileSorted(normalized, 0.95),
    maxMs: normalized[normalized.length - 1]!,
    framesOver50Ms: normalized.filter((value) => value > 50).length,
    droppedFrameEstimate: normalized.reduce(
      (total, value) => total + Math.max(0, Math.round(value / targetFrameMs) - 1),
      0,
    ),
  };
}

export function evaluateFpsTruth(
  scenarioId: string,
  reportedFps: number,
  frameDeltasMs: readonly number[],
  thresholds: FpsTruthThresholds = DEFAULT_FPS_TRUTH_THRESHOLDS,
): RenderScenarioResult {
  const summary = summarizeFrameDeltas(frameDeltasMs);
  const computedMedianFps = summary.medianMs > 0 ? 1000 / summary.medianMs : 0;
  const errorRatio = computedMedianFps > 0
    ? Math.abs(reportedFps - computedMedianFps) / computedMedianFps
    : Number.POSITIVE_INFINITY;
  const metric: RenderMetric = {
    id: "fps.error_ratio",
    label: "Reported FPS error ratio",
    value: roundMetric(errorRatio),
    status: errorRatio <= thresholds.maxErrorRatio ? "pass" : "fail",
    threshold: `<= ${thresholds.maxErrorRatio}`,
    details: {
      reportedFps: roundMetric(reportedFps),
      computedMedianFps: roundMetric(computedMedianFps),
      medianFrameMs: roundMetric(summary.medianMs),
      frameCount: summary.frameCount,
    },
  };
  return buildScenarioResult(scenarioId, "FPS truth", [metric]);
}

export function buildScenarioResult(
  id: string,
  label: string,
  metrics: readonly RenderMetric[],
  artifacts: Record<string, string> = {},
): RenderScenarioResult {
  const failures = metrics
    .filter((metric) => metric.status === "fail")
    .map((metric) => ({
      scenarioId: id,
      metricId: metric.id,
      message: `${label}: ${metric.label} failed`,
      value: metric.value,
      threshold: metric.threshold,
    }));
  return {
    id,
    label,
    metrics: [...metrics],
    failures,
    artifacts,
  };
}

export function buildRenderVerificationReport(input: {
  generatedAt: string;
  label?: string | null;
  commit?: string | null;
  scenarios: readonly RenderScenarioResult[];
}): RenderVerificationReport {
  const scenarios = [...input.scenarios];
  return {
    generatedAt: input.generatedAt,
    label: input.label ?? null,
    commit: input.commit ?? null,
    scenarios,
    failures: scenarios.flatMap((scenario) => scenario.failures),
  };
}

function numericMinimumMetric(id: string, label: string, value: number, minimum: number): RenderMetric {
  return {
    id,
    label,
    value: roundMetric(value),
    status: value >= minimum ? "pass" : "fail",
    threshold: `>= ${minimum}`,
  };
}

function zeroMetric(id: string, label: string, value: number): RenderMetric {
  return {
    id,
    label,
    value,
    status: value === 0 ? "pass" : "fail",
    threshold: "=== 0",
  };
}

function average(values: readonly number[]): number {
  return values.reduce((total, value) => total + value, 0) / values.length;
}

function percentileSorted(values: readonly number[], percentile: number): number {
  if (values.length === 0) {
    return 0;
  }
  const index = Math.min(values.length - 1, Math.max(0, Math.ceil(values.length * percentile) - 1));
  return values[index]!;
}

function roundMetric(value: number): number {
  if (!Number.isFinite(value)) {
    return value;
  }
  return Math.round(value * 1000) / 1000;
}
