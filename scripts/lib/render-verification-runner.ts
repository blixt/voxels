import { readdir, readFile } from "node:fs/promises";
import { dirname, join, relative } from "node:path";

import { summarizeFrameDeltas } from "./render-verification-metrics.ts";

export type RenderVerificationSourceId = "route-atlas" | "live-forward" | "lod-persistence" | "view-atlas";
export type RenderVerificationGateStatus = "pass" | "fail" | "warn" | "skip";

export interface RenderVerificationThresholds {
  readonly minRouteSamples: number;
  readonly minLiveForwardSamples: number;
  readonly minViewCount: number;
  readonly minLodIterations: number;
  readonly maxP95FrameMs: number;
  readonly maxFrameMs: number;
  readonly maxFpsErrorRatio: number;
  readonly requireArtifacts: boolean;
}

export const DEFAULT_RENDER_VERIFICATION_THRESHOLDS: RenderVerificationThresholds = {
  minRouteSamples: 100,
  minLiveForwardSamples: 120,
  minViewCount: 1,
  minLodIterations: 1,
  maxP95FrameMs: 16.67,
  maxFrameMs: 50,
  maxFpsErrorRatio: 0.10,
  requireArtifacts: false,
};

export interface RenderVerificationArtifactPaths {
  readonly routeAtlasReport?: string | null;
  readonly liveForwardReport?: string | null;
  readonly liveForwardSamples?: string | null;
  readonly lodReport?: string | null;
  readonly viewAtlasReport?: string | null;
}

export interface RenderVerificationArtifacts {
  readonly routeAtlasReport?: unknown;
  readonly liveForwardReport?: unknown;
  readonly liveForwardSamples?: readonly unknown[];
  readonly lodReport?: unknown;
  readonly viewAtlasReport?: unknown;
}

export interface RenderVerificationCommandManifestEntry {
  readonly id: RenderVerificationSourceId;
  readonly label: string;
  readonly command: readonly string[];
  readonly artifactPath: string | null;
  readonly samplePath?: string | null;
  readonly present: boolean;
}

export interface RenderVerificationInputSummary {
  readonly id: RenderVerificationSourceId;
  readonly path: string | null;
  readonly samplePath?: string | null;
  readonly present: boolean;
  readonly generatedAt: string | null;
  readonly label: string | null;
  readonly commit: string | null;
}

export interface RenderVerificationGate {
  readonly id: string;
  readonly label: string;
  readonly source: RenderVerificationSourceId;
  readonly status: RenderVerificationGateStatus;
  readonly value: number | string | boolean | null;
  readonly unit?: string;
  readonly threshold?: string;
  readonly details?: Record<string, unknown>;
}

export interface RenderVerificationFailure {
  readonly groupId: string;
  readonly gateId: string;
  readonly message: string;
  readonly value: number | string | boolean | null;
  readonly threshold?: string;
}

export interface RenderVerificationGateGroup {
  readonly id: string;
  readonly label: string;
  readonly source: RenderVerificationSourceId;
  readonly gates: RenderVerificationGate[];
  readonly failures: RenderVerificationFailure[];
  readonly artifacts?: Record<string, string>;
}

export interface RenderVerificationReport {
  readonly schemaVersion: 1;
  readonly generatedAt: string;
  readonly label: string | null;
  readonly commit: string | null;
  readonly thresholds: RenderVerificationThresholds;
  readonly commandManifest: RenderVerificationCommandManifestEntry[];
  readonly inputs: RenderVerificationInputSummary[];
  readonly gateGroups: RenderVerificationGateGroup[];
  readonly gateSummary: {
    readonly total: number;
    readonly pass: number;
    readonly fail: number;
    readonly warn: number;
    readonly skip: number;
  };
  readonly failures: RenderVerificationFailure[];
}

export interface BuildRenderVerificationReportInput {
  readonly generatedAt: string;
  readonly label?: string | null;
  readonly commit?: string | null;
  readonly cwd?: string;
  readonly thresholds?: Partial<RenderVerificationThresholds>;
  readonly paths?: RenderVerificationArtifactPaths;
  readonly artifacts: RenderVerificationArtifacts;
}

interface LodCoverageCounts {
  readonly uncoveredGapCount: number;
  readonly handoffHoleCount: number;
  readonly residentOverlapCount: number;
  readonly bandOverlapCount: number;
  readonly waterOverlapCount: number;
}

export function buildRenderVerificationRunnerReport(
  input: BuildRenderVerificationReportInput,
): RenderVerificationReport {
  const thresholds = {
    ...DEFAULT_RENDER_VERIFICATION_THRESHOLDS,
    ...definedThresholdOverrides(input.thresholds),
  };
  const paths = input.paths ?? {};
  const commandManifest = buildCommandManifest(paths, input.label ?? null);
  const inputs = buildInputSummaries(input.artifacts, paths);
  const gateGroups = [
    buildInventoryGateGroup(input.artifacts, paths, thresholds),
    buildRouteAtlasGateGroup(input.artifacts.routeAtlasReport, paths.routeAtlasReport ?? null, thresholds),
    buildLiveForwardGateGroup(
      input.artifacts.liveForwardReport,
      input.artifacts.liveForwardSamples ?? [],
      {
        reportPath: paths.liveForwardReport ?? null,
        samplesPath: paths.liveForwardSamples ?? null,
      },
      thresholds,
    ),
    buildLodPersistenceGateGroup(input.artifacts.lodReport, paths.lodReport ?? null, thresholds),
    buildViewVisualGateGroup(input.artifacts.viewAtlasReport, paths.viewAtlasReport ?? null, thresholds),
    buildViewSettleGateGroup(input.artifacts.viewAtlasReport, paths.viewAtlasReport ?? null),
  ];
  const allGates = gateGroups.flatMap((group) => group.gates);
  return {
    schemaVersion: 1,
    generatedAt: input.generatedAt,
    label: input.label ?? null,
    commit: input.commit ?? null,
    thresholds,
    commandManifest,
    inputs,
    gateGroups,
    gateSummary: {
      total: allGates.length,
      pass: allGates.filter((gate) => gate.status === "pass").length,
      fail: allGates.filter((gate) => gate.status === "fail").length,
      warn: allGates.filter((gate) => gate.status === "warn").length,
      skip: allGates.filter((gate) => gate.status === "skip").length,
    },
    failures: gateGroups.flatMap((group) => group.failures),
  };
}

function definedThresholdOverrides(
  thresholds: Partial<RenderVerificationThresholds> | undefined,
): Partial<RenderVerificationThresholds> {
  if (!thresholds) {
    return {};
  }
  return Object.fromEntries(
    Object.entries(thresholds).filter((entry): entry is [keyof RenderVerificationThresholds, number | boolean] =>
      entry[1] !== undefined
    ),
  ) as Partial<RenderVerificationThresholds>;
}

export async function loadRenderVerificationArtifacts(
  paths: RenderVerificationArtifactPaths,
): Promise<RenderVerificationArtifacts> {
  const liveForwardReport = paths.liveForwardReport ? await readJson(paths.liveForwardReport) : undefined;
  const inferredLiveForwardSamples = paths.liveForwardSamples
    ?? inferLiveForwardSamplesPath(liveForwardReport, paths.liveForwardReport ?? null);
  return {
    routeAtlasReport: paths.routeAtlasReport ? await readJson(paths.routeAtlasReport) : undefined,
    liveForwardReport,
    liveForwardSamples: inferredLiveForwardSamples
      ? asArray(await readJson(inferredLiveForwardSamples))
      : undefined,
    lodReport: paths.lodReport ? await readJson(paths.lodReport) : undefined,
    viewAtlasReport: paths.viewAtlasReport ? await readJson(paths.viewAtlasReport) : undefined,
  };
}

export async function resolveDefaultRenderVerificationArtifactPaths(
  artifactRoot = "artifacts",
): Promise<RenderVerificationArtifactPaths> {
  const liveForwardReport = await findLatestReport(join(artifactRoot, "browser-route-trace"));
  return {
    routeAtlasReport: await findLatestReport(join(artifactRoot, "route-atlas")),
    liveForwardReport,
    liveForwardSamples: liveForwardReport ? inferLiveForwardSamplesPath(await readJson(liveForwardReport), liveForwardReport) : null,
    lodReport: await findLatestFile(artifactRoot, "lod-idb-persistence-reload.json"),
    viewAtlasReport: await findLatestReport(join(artifactRoot, "view-atlas")),
  };
}

export function buildCommandManifest(
  paths: RenderVerificationArtifactPaths,
  label: string | null,
): RenderVerificationCommandManifestEntry[] {
  const labelArg = label ? [`--label=${label}`] : [];
  return [
    {
      id: "route-atlas",
      label: "Route atlas baseline inventory",
      command: ["bun", "run", "atlas:routes", "--", ...labelArg],
      artifactPath: paths.routeAtlasReport ?? null,
      present: Boolean(paths.routeAtlasReport),
    },
    {
      id: "live-forward",
      label: "Live-forward browser route trace",
      command: ["bun", "run", "trace:route", "--", "--benchmark=live-forward", ...labelArg],
      artifactPath: paths.liveForwardReport ?? null,
      samplePath: paths.liveForwardSamples ?? null,
      present: Boolean(paths.liveForwardReport),
    },
    {
      id: "lod-persistence",
      label: "LOD persistence coverage probe",
      command: ["bun", "run", "bench:lod-persistence", "--", ...labelArg],
      artifactPath: paths.lodReport ?? null,
      present: Boolean(paths.lodReport),
    },
    {
      id: "view-atlas",
      label: "View atlas visual and settle capture",
      command: ["bun", "run", "atlas:views", "--", ...labelArg],
      artifactPath: paths.viewAtlasReport ?? null,
      present: Boolean(paths.viewAtlasReport),
    },
  ];
}

function buildInventoryGateGroup(
  artifacts: RenderVerificationArtifacts,
  paths: RenderVerificationArtifactPaths,
  thresholds: RenderVerificationThresholds,
): RenderVerificationGateGroup {
  const gate = (id: string, label: string, source: RenderVerificationSourceId, present: boolean): RenderVerificationGate => ({
    id,
    label,
    source,
    status: present ? "pass" : thresholds.requireArtifacts ? "fail" : "warn",
    value: present,
    threshold: thresholds.requireArtifacts ? "present" : "present for full gate coverage",
  });
  return buildGateGroup(
    "artifact-inventory",
    "Artifact inventory",
    "route-atlas",
    [
      gate("artifact.route_atlas_present", "Route atlas report present", "route-atlas", Boolean(artifacts.routeAtlasReport)),
      gate("artifact.live_forward_present", "Live-forward report present", "live-forward", Boolean(artifacts.liveForwardReport)),
      gate("artifact.live_forward_samples_present", "Live-forward samples present", "live-forward", Boolean(artifacts.liveForwardSamples)),
      gate("artifact.lod_persistence_present", "LOD persistence report present", "lod-persistence", Boolean(artifacts.lodReport)),
      gate("artifact.view_atlas_present", "View atlas report present", "view-atlas", Boolean(artifacts.viewAtlasReport)),
    ],
    compactArtifacts({
      routeAtlasReport: paths.routeAtlasReport,
      liveForwardReport: paths.liveForwardReport,
      liveForwardSamples: paths.liveForwardSamples,
      lodReport: paths.lodReport,
      viewAtlasReport: paths.viewAtlasReport,
    }),
  );
}

function buildRouteAtlasGateGroup(
  report: unknown,
  reportPath: string | null,
  thresholds: RenderVerificationThresholds,
): RenderVerificationGateGroup {
  if (!report) {
    return skippedGroup("route-atlas", "Route atlas gates", "route-atlas", reportPath);
  }
  const record = asRecord(report);
  const sampleCount = readNumber(record, ["aggregate", "sampleCount"])
    ?? sumNumbers(asArray(record.routes).map((route) => readNumber(route, ["sampleCount"]) ?? 0));
  const failures = asArray(record.failures).length;
  return buildGateGroup(
    "route-atlas",
    "Route atlas gates",
    "route-atlas",
    [
      minGate("route.samples_min", "Route atlas sample minimum", "route-atlas", sampleCount, thresholds.minRouteSamples),
      zeroGate("route.failures", "Route atlas failure count", "route-atlas", failures),
    ],
    compactArtifacts({ report: reportPath }),
  );
}

function buildLiveForwardGateGroup(
  report: unknown,
  samples: readonly unknown[],
  paths: { readonly reportPath: string | null; readonly samplesPath: string | null },
  thresholds: RenderVerificationThresholds,
): RenderVerificationGateGroup {
  if (!report && samples.length === 0) {
    return skippedGroup("live-forward", "Live-forward FPS and LOD gates", "live-forward", paths.reportPath);
  }
  const reportRecord = asRecord(report);
  const frameDeltas = samples
    .map((sample) => readNumber(sample, ["gameplayFrameMs"]))
    .filter((value): value is number => value !== null);
  const frameSummary = summarizeFrameDeltas(frameDeltas);
  const sampleCount = samples.length > 0 ? samples.length : readNumber(reportRecord, ["benchmark", "sampleCount"]) ?? 0;
  const fpsTruth = calculateFpsTruth(reportRecord, samples);
  const lodCoverage = summarizeLiveForwardLodCoverage(samples);
  return buildGateGroup(
    "live-forward",
    "Live-forward FPS and LOD gates",
    "live-forward",
    [
      minGate("live_forward.samples_min", "Live-forward sample minimum", "live-forward", sampleCount, thresholds.minLiveForwardSamples),
      maxGate("live_forward.p95_frame_ms", "P95 gameplay frame", "live-forward", frameSummary.p95Ms, thresholds.maxP95FrameMs, "ms"),
      maxGate("live_forward.max_frame_ms", "Max gameplay frame", "live-forward", frameSummary.maxMs, thresholds.maxFrameMs, "ms"),
      {
        id: "live_forward.fps_truth_error_ratio",
        label: "FPS truth error ratio",
        source: "live-forward",
        status: fpsTruth.errorRatio === null
          ? "skip"
          : fpsTruth.errorRatio <= thresholds.maxFpsErrorRatio ? "pass" : "fail",
        value: fpsTruth.errorRatio === null ? null : roundMetric(fpsTruth.errorRatio),
        threshold: `<= ${thresholds.maxFpsErrorRatio}`,
        details: fpsTruth,
      },
      ...lodCoverageGates("live_forward", "live-forward", lodCoverage),
    ],
    compactArtifacts({ report: paths.reportPath, samples: paths.samplesPath }),
  );
}

function buildLodPersistenceGateGroup(
  report: unknown,
  reportPath: string | null,
  thresholds: RenderVerificationThresholds,
): RenderVerificationGateGroup {
  if (!report) {
    return skippedGroup("lod-persistence", "LOD persistence coverage gates", "lod-persistence", reportPath);
  }
  const record = asRecord(report);
  const iterations = asArray(record.iterations);
  const aggregate = asRecord(record.aggregate);
  const coverage = iterations.length > 0
    ? summarizeLodPersistenceCoverage(iterations)
    : summarizeLodPersistenceAggregateCoverage(aggregate);
  const farSettle = iterations.length > 0
    ? summarizeLodPersistenceFarSettle(iterations)
    : summarizeLodPersistenceAggregateFarSettle(aggregate);
  const iterationCount = iterations.length > 0 ? iterations.length : readNumber(aggregate, ["iterationCount"]) ?? 0;
  const artifactFailures = readNumber(aggregate, ["failureCount"]) ?? asArray(record.failures).length;
  return buildGateGroup(
    "lod-persistence",
    "LOD persistence coverage gates",
    "lod-persistence",
    [
      minGate("lod_persistence.iterations_min", "LOD persistence iteration minimum", "lod-persistence", iterationCount, thresholds.minLodIterations),
      zeroGate("lod_persistence.artifact_failures", "LOD persistence artifact failure count", "lod-persistence", artifactFailures),
      ...lodCoverageGates("lod_persistence", "lod-persistence", coverage),
      {
        id: "lod_persistence.far_unsettled_count",
        label: "Far transition unsettled phases",
        source: "lod-persistence",
        status: farSettle.unsettledCount === 0 ? "pass" : "warn",
        value: farSettle.unsettledCount,
        threshold: "=== 0",
        details: {
          maxPendingChunks: farSettle.maxPendingChunks,
        },
      },
    ],
    compactArtifacts({ report: reportPath }),
  );
}

function buildViewVisualGateGroup(
  report: unknown,
  reportPath: string | null,
  thresholds: RenderVerificationThresholds,
): RenderVerificationGateGroup {
  if (!report) {
    return skippedGroup("view-visual", "View visual gates", "view-atlas", reportPath);
  }
  const views = asArray(asRecord(report).views);
  const blankishViews = views
    .filter((view) => readBoolean(view, ["visual", "diagnosis", "blankish"]) === true)
    .map((view) => readString(view, ["id"]) ?? "unknown");
  return buildGateGroup(
    "view-visual",
    "View visual gates",
    "view-atlas",
    [
      minGate("view.count_min", "View capture minimum", "view-atlas", views.length, thresholds.minViewCount),
      zeroGate("view.visual_blankish_count", "Blankish visual count", "view-atlas", blankishViews.length, {
        blankishViews,
      }),
    ],
    compactArtifacts({ report: reportPath }),
  );
}

function buildViewSettleGateGroup(
  report: unknown,
  reportPath: string | null,
): RenderVerificationGateGroup {
  if (!report) {
    return skippedGroup("view-settle", "View full-settle gates", "view-atlas", reportPath);
  }
  const views = asArray(asRecord(report).views);
  const unsettledViews = views
    .filter((view) => readBoolean(view, ["settled", "settled"]) === false)
    .map((view) => readString(view, ["id"]) ?? "unknown");
  const pendingLodViews = views
    .filter((view) => (readNumber(view, ["snapshot", "lodPendingChunks"]) ?? 0) > 0)
    .map((view) => readString(view, ["id"]) ?? "unknown");
  return buildGateGroup(
    "view-settle",
    "View full-settle gates",
    "view-atlas",
    [
      zeroGate("view.settle_unsettled_count", "Unsettled view count", "view-atlas", unsettledViews.length, {
        unsettledViews,
      }),
      zeroGate("view.settle_pending_lod_count", "Views with pending LOD after settle", "view-atlas", pendingLodViews.length, {
        pendingLodViews,
      }),
    ],
    compactArtifacts({ report: reportPath }),
  );
}

function buildGateGroup(
  id: string,
  label: string,
  source: RenderVerificationSourceId,
  gates: readonly RenderVerificationGate[],
  artifacts: Record<string, string> = {},
): RenderVerificationGateGroup {
  const failures = gates
    .filter((gate) => gate.status === "fail")
    .map((gate) => ({
      groupId: id,
      gateId: gate.id,
      message: `${label}: ${gate.label} failed`,
      value: gate.value,
      threshold: gate.threshold,
    }));
  return {
    id,
    label,
    source,
    gates: [...gates],
    failures,
    artifacts,
  };
}

function skippedGroup(
  id: string,
  label: string,
  source: RenderVerificationSourceId,
  reportPath: string | null,
): RenderVerificationGateGroup {
  return buildGateGroup(id, label, source, [
    {
      id: `${id}.artifact_missing`,
      label: "Artifact not provided",
      source,
      status: "skip",
      value: false,
      threshold: "artifact path provided",
    },
  ], compactArtifacts({ report: reportPath }));
}

function buildInputSummaries(
  artifacts: RenderVerificationArtifacts,
  paths: RenderVerificationArtifactPaths,
): RenderVerificationInputSummary[] {
  return [
    inputSummary("route-atlas", paths.routeAtlasReport ?? null, artifacts.routeAtlasReport),
    inputSummary(
      "live-forward",
      paths.liveForwardReport ?? null,
      artifacts.liveForwardReport,
      paths.liveForwardSamples ?? null,
    ),
    inputSummary("lod-persistence", paths.lodReport ?? null, artifacts.lodReport),
    inputSummary("view-atlas", paths.viewAtlasReport ?? null, artifacts.viewAtlasReport),
  ];
}

function inputSummary(
  id: RenderVerificationSourceId,
  path: string | null,
  artifact: unknown,
  samplePath?: string | null,
): RenderVerificationInputSummary {
  const record = asRecord(artifact);
  return {
    id,
    path,
    samplePath,
    present: Boolean(artifact),
    generatedAt: readString(record, ["generatedAt"]),
    label: readString(record, ["label"]),
    commit: readString(record, ["commit"]),
  };
}

function minGate(
  id: string,
  label: string,
  source: RenderVerificationSourceId,
  value: number,
  minimum: number,
): RenderVerificationGate {
  return {
    id,
    label,
    source,
    status: value >= minimum ? "pass" : "fail",
    value,
    threshold: `>= ${minimum}`,
  };
}

function maxGate(
  id: string,
  label: string,
  source: RenderVerificationSourceId,
  value: number,
  maximum: number,
  unit?: string,
): RenderVerificationGate {
  return {
    id,
    label,
    source,
    status: value <= maximum ? "pass" : "fail",
    value: roundMetric(value),
    unit,
    threshold: `<= ${maximum}`,
  };
}

function zeroGate(
  id: string,
  label: string,
  source: RenderVerificationSourceId,
  value: number,
  details?: Record<string, unknown>,
): RenderVerificationGate {
  return {
    id,
    label,
    source,
    status: value === 0 ? "pass" : "fail",
    value,
    threshold: "=== 0",
    details,
  };
}

function lodCoverageGates(
  prefix: string,
  source: RenderVerificationSourceId,
  counts: LodCoverageCounts,
): RenderVerificationGate[] {
  return [
    zeroGate(`${prefix}.lod_uncovered_gap_count`, "LOD uncovered gaps", source, counts.uncoveredGapCount),
    zeroGate(`${prefix}.lod_handoff_hole_count`, "LOD handoff holes", source, counts.handoffHoleCount),
    zeroGate(`${prefix}.lod_resident_overlap_count`, "Resident/LOD overlaps", source, counts.residentOverlapCount),
    zeroGate(`${prefix}.lod_band_overlap_count`, "LOD band overlaps", source, counts.bandOverlapCount),
    zeroGate(`${prefix}.lod_water_overlap_count`, "Water ownership overlaps", source, counts.waterOverlapCount),
  ];
}

function calculateFpsTruth(
  report: Record<string, unknown>,
  samples: readonly unknown[],
): Record<string, number | null> {
  const expectedFps = readNumber(report, ["routeOptions", "sampleHz"])
    ?? readNumber(report, ["benchmark", "sampleHz"])
    ?? readNumber(report, ["benchmark", "summary", "sampleHz"]);
  const firstSimSeconds = readNumber(samples[0], ["simTimeSeconds"]);
  const lastSimSeconds = readNumber(samples[samples.length - 1], ["simTimeSeconds"]);
  const computedFps = firstSimSeconds !== null && lastSimSeconds !== null && lastSimSeconds > firstSimSeconds
    ? samples.length / (lastSimSeconds - firstSimSeconds)
    : readNumber(report, ["benchmark", "summary", "sampleHz"]);
  const errorRatio = expectedFps !== null && computedFps !== null && computedFps > 0
    ? Math.abs(expectedFps - computedFps) / computedFps
    : null;
  return {
    expectedFps: expectedFps === null ? null : roundMetric(expectedFps),
    computedFps: computedFps === null ? null : roundMetric(computedFps),
    errorRatio: errorRatio === null ? null : roundMetric(errorRatio),
  };
}

function summarizeLiveForwardLodCoverage(samples: readonly unknown[]): LodCoverageCounts {
  return {
    uncoveredGapCount: maxNumbers(samples.flatMap((sample) => [
      readNumber(sample, ["uncoveredLodGapCount"]),
      readNumber(sample, ["uncoveredFarLodGapCount"]),
      readNumber(sample, ["farLodCoverageGapCount"]),
    ])),
    handoffHoleCount: maxNumbers(samples.flatMap((sample) => [
      readNumber(sample, ["handoffLodHoleCount"]),
      readNumber(sample, ["handoffFarLodHoleCount"]),
    ])),
    residentOverlapCount: maxNumbers(samples.map((sample) => readNumber(sample, ["lodResidentOverlapCount"]))),
    bandOverlapCount: maxNumbers(samples.map((sample) => readNumber(sample, ["lodBandOverlapCount"]))),
    waterOverlapCount: maxNumbers(samples.map((sample) => readNumber(sample, ["waterOverlapCount"]))),
  };
}

function summarizeLodPersistenceCoverage(iterations: readonly unknown[]): LodCoverageCounts {
  const phases = iterations.flatMap((iteration) => [
    readValue(iteration, ["coldOrigin"]),
    readValue(iteration, ["farEviction"]),
    readValue(iteration, ["storeFlush"]),
    readValue(iteration, ["reloadOrigin"]),
  ]);
  const coverages = phases.map((phase) => readValue(phase, ["finalCoverage"]));
  return {
    uncoveredGapCount: maxNumbers(coverages.map((coverage) => readNumber(coverage, ["uncoveredGapCount"]))),
    handoffHoleCount: maxNumbers(coverages.map((coverage) => readNumber(coverage, ["handoffHoleCount"]))),
    residentOverlapCount: maxNumbers(coverages.map((coverage) => readNumber(coverage, ["residentOverlapCount"]))),
    bandOverlapCount: maxNumbers(coverages.map((coverage) => readNumber(coverage, ["bandOverlapCount"]))),
    waterOverlapCount: maxNumbers(coverages.map((coverage) => readNumber(coverage, ["waterOverlapCount"]))),
  };
}

function summarizeLodPersistenceAggregateCoverage(aggregate: Record<string, unknown>): LodCoverageCounts {
  return {
    uncoveredGapCount: readNumber(aggregate, ["maxReloadCoverageGaps"]) ?? 0,
    handoffHoleCount: 0,
    residentOverlapCount: readNumber(aggregate, ["maxReloadCoverageOverlaps"]) ?? 0,
    bandOverlapCount: 0,
    waterOverlapCount: 0,
  };
}

function summarizeLodPersistenceFarSettle(iterations: readonly unknown[]): { unsettledCount: number; maxPendingChunks: number } {
  const farPhases = iterations
    .map((iteration) => readValue(iteration, ["farEviction"]))
    .filter((phase) => readString(phase, ["label"]) !== "far-eviction-skipped");
  return {
    unsettledCount: farPhases.filter((phase) => readBoolean(phase, ["settled"]) === false).length,
    maxPendingChunks: maxNumbers(farPhases.map((phase) => readNumber(phase, ["finalLodPendingChunks"]))),
  };
}

function summarizeLodPersistenceAggregateFarSettle(
  aggregate: Record<string, unknown>,
): { unsettledCount: number; maxPendingChunks: number } {
  return {
    unsettledCount: readNumber(aggregate, ["farUnsettledCount"]) ?? 0,
    maxPendingChunks: readNumber(aggregate, ["maxFarLodPendingChunks"]) ?? 0,
  };
}

function inferLiveForwardSamplesPath(report: unknown, reportPath: string | null): string | null {
  const samplePath = readString(report, ["benchmarkSamplesPath"]);
  if (!samplePath) {
    return null;
  }
  if (samplePath.startsWith("/") || !reportPath) {
    return samplePath;
  }
  return join(dirname(reportPath), relative(dirname(reportPath), samplePath));
}

async function findLatestReport(dir: string): Promise<string | null> {
  const entries = await readDirectoryEntries(dir);
  for (const entry of entries.sort((left, right) => right.name.localeCompare(left.name))) {
    if (!entry.isDirectory()) {
      continue;
    }
    const reportPath = join(dir, entry.name, "report.json");
    try {
      await readFile(reportPath);
      return reportPath;
    } catch {
      continue;
    }
  }
  return null;
}

async function findLatestFile(root: string, fileName: string): Promise<string | null> {
  const matches: string[] = [];
  await collectMatchingFiles(root, fileName, matches, 3);
  return matches.sort((left, right) => right.localeCompare(left))[0] ?? null;
}

async function collectMatchingFiles(
  dir: string,
  fileName: string,
  matches: string[],
  depthRemaining: number,
): Promise<void> {
  if (depthRemaining < 0) {
    return;
  }
  for (const entry of await readDirectoryEntries(dir)) {
    const path = join(dir, entry.name);
    if (entry.isFile() && entry.name === fileName) {
      matches.push(path);
    } else if (entry.isDirectory()) {
      await collectMatchingFiles(path, fileName, matches, depthRemaining - 1);
    }
  }
}

async function readDirectoryEntries(dir: string) {
  try {
    return await readdir(dir, { withFileTypes: true });
  } catch {
    return [];
  }
}

async function readJson(path: string): Promise<unknown> {
  return JSON.parse(await readFile(path, "utf8")) as unknown;
}

function compactArtifacts(input: Record<string, string | null | undefined>): Record<string, string> {
  const result: Record<string, string> = {};
  for (const [key, value] of Object.entries(input)) {
    if (value) {
      result[key] = value;
    }
  }
  return result;
}

function asRecord(value: unknown): Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : {};
}

function asArray(value: unknown): unknown[] {
  return Array.isArray(value) ? value : [];
}

function readValue(value: unknown, path: readonly string[]): unknown {
  let current = value;
  for (const part of path) {
    current = asRecord(current)[part];
  }
  return current;
}

function readNumber(value: unknown, path: readonly string[]): number | null {
  const found = readValue(value, path);
  return typeof found === "number" && Number.isFinite(found) ? found : null;
}

function readString(value: unknown, path: readonly string[]): string | null {
  const found = readValue(value, path);
  return typeof found === "string" ? found : null;
}

function readBoolean(value: unknown, path: readonly string[]): boolean | null {
  const found = readValue(value, path);
  return typeof found === "boolean" ? found : null;
}

function maxNumbers(values: readonly (number | null)[]): number {
  const finite = values.filter((value): value is number => typeof value === "number" && Number.isFinite(value));
  return finite.length === 0 ? 0 : Math.max(...finite);
}

function sumNumbers(values: readonly number[]): number {
  return values.reduce((total, value) => total + value, 0);
}

function roundMetric(value: number): number {
  return Number.isFinite(value) ? Math.round(value * 1000) / 1000 : value;
}
