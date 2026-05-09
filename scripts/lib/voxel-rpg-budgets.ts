import type {
  RenderVerificationFailure,
  RenderVerificationGateStatus,
  RenderVerificationReport,
  RenderVerificationThresholds,
} from "./render-verification-runner.ts";
import { summarizeFrameDeltas } from "./render-verification-metrics.ts";

export type VoxelRpgVerificationProfileId = "rpg-render-gate" | "rpg-render-smoke";

export interface VoxelRpgEvidenceBudgets {
  readonly maxArtifactAgeMs: number;
  readonly maxClockSkewMs: number;
  readonly requireGeneratedAt: boolean;
  readonly requireMatchingCommit: boolean;
}

export interface VoxelRpgHitchBudgets {
  readonly hitchFrameMs: number;
  readonly maxHitchFrames: number;
  readonly maxMovementHitchFrames: number;
  readonly maxSettleHitchFrames: number;
  readonly maxDroppedFrameEstimate: number;
  readonly maxLodWorkMs: number;
  readonly maxLodWorkHitchFrames: number;
}

export interface VoxelRpgVerificationBudgetProfile {
  readonly id: VoxelRpgVerificationProfileId;
  readonly label: string;
  readonly renderThresholds: RenderVerificationThresholds;
  readonly evidence: VoxelRpgEvidenceBudgets;
  readonly hitches: VoxelRpgHitchBudgets;
}

const DEFAULT_RENDER_THRESHOLDS = {
  minRouteSamples: 100,
  minLiveForwardSamples: 120,
  minViewCount: 1,
  minLodIterations: 1,
  maxP95FrameMs: 16.67,
  maxFrameMs: 50,
  maxFpsErrorRatio: 0.10,
  requireArtifacts: true,
} satisfies RenderVerificationThresholds;

export const VOXEL_RPG_VERIFICATION_PROFILES = {
  "rpg-render-gate": {
    id: "rpg-render-gate",
    label: "RPG render gate",
    renderThresholds: DEFAULT_RENDER_THRESHOLDS,
    evidence: {
      maxArtifactAgeMs: 2 * 60 * 60 * 1000,
      maxClockSkewMs: 5 * 60 * 1000,
      requireGeneratedAt: true,
      requireMatchingCommit: true,
    },
    hitches: {
      hitchFrameMs: 50,
      maxHitchFrames: 0,
      maxMovementHitchFrames: 0,
      maxSettleHitchFrames: 0,
      maxDroppedFrameEstimate: 0,
      maxLodWorkMs: 12,
      maxLodWorkHitchFrames: 0,
    },
  },
  "rpg-render-smoke": {
    id: "rpg-render-smoke",
    label: "RPG render smoke",
    renderThresholds: {
      ...DEFAULT_RENDER_THRESHOLDS,
      minRouteSamples: 20,
      minLiveForwardSamples: 20,
    },
    evidence: {
      maxArtifactAgeMs: 24 * 60 * 60 * 1000,
      maxClockSkewMs: 5 * 60 * 1000,
      requireGeneratedAt: true,
      requireMatchingCommit: true,
    },
    hitches: {
      hitchFrameMs: 50,
      maxHitchFrames: 0,
      maxMovementHitchFrames: 0,
      maxSettleHitchFrames: 0,
      maxDroppedFrameEstimate: 0,
      maxLodWorkMs: 12,
      maxLodWorkHitchFrames: 0,
    },
  },
} satisfies Record<VoxelRpgVerificationProfileId, VoxelRpgVerificationBudgetProfile>;

export const DEFAULT_VOXEL_RPG_VERIFICATION_PROFILE_ID = "rpg-render-gate" satisfies VoxelRpgVerificationProfileId;

export const VOXEL_RPG_VERIFICATION_BUDGETS =
  VOXEL_RPG_VERIFICATION_PROFILES[DEFAULT_VOXEL_RPG_VERIFICATION_PROFILE_ID].renderThresholds;

export type VoxelRpgVerificationFailureCategory =
  | "artifact"
  | "correctness"
  | "performance"
  | "settle";

export interface VoxelRpgFailureCategorySummary {
  readonly count: number;
  readonly failures: RenderVerificationFailure[];
}

export interface VoxelRpgVerificationSummary {
  readonly status: "pass" | "fail";
  readonly failureCount: number;
  readonly correctnessFailures: VoxelRpgFailureCategorySummary;
  readonly performanceFailures: VoxelRpgFailureCategorySummary;
  readonly settleFailures: VoxelRpgFailureCategorySummary;
  readonly artifactFailures: VoxelRpgFailureCategorySummary;
}

export interface VoxelRpgEvidenceArtifact {
  readonly id: string;
  readonly label: string;
  readonly command: readonly string[];
  readonly path: string | null;
  readonly samplePath?: string | null;
  readonly present: boolean;
  readonly generatedAt: string | null;
  readonly commit: string | null;
  readonly ageMs: number | null;
  readonly status: "fresh" | "stale" | "future" | "missing-generated-at" | "missing";
}

export interface VoxelRpgEvidenceBundle {
  readonly generatedAt: string;
  readonly commit: string | null;
  readonly maxArtifactAgeMs: number;
  readonly maxClockSkewMs: number;
  readonly artifacts: VoxelRpgEvidenceArtifact[];
}

export interface VoxelRpgHitchSummary {
  readonly sampleCount: number;
  readonly hitchFrameMs: number;
  readonly frameCount: number;
  readonly p95FrameMs: number;
  readonly maxFrameMs: number;
  readonly hitchFrames: number;
  readonly movementHitchFrames: number;
  readonly settleHitchFrames: number;
  readonly droppedFrameEstimate: number;
  readonly maxLodWorkMs: number;
  readonly lodWorkHitchFrames: number;
}

export interface VoxelRpgVerificationGate {
  readonly id: string;
  readonly label: string;
  readonly status: RenderVerificationGateStatus;
  readonly value: number | string | boolean | null;
  readonly unit?: string;
  readonly threshold?: string;
  readonly details?: Record<string, unknown>;
}

export interface VoxelRpgVerificationGateGroup {
  readonly id: string;
  readonly label: string;
  readonly gates: VoxelRpgVerificationGate[];
  readonly failures: RenderVerificationFailure[];
}

export interface BuildVoxelRpgVerificationReportOptions {
  readonly profile?: VoxelRpgVerificationProfileId | VoxelRpgVerificationBudgetProfile;
  readonly liveForwardSamples?: readonly unknown[];
}

export interface VoxelRpgVerificationReport {
  readonly schemaVersion: 1;
  readonly generatedAt: string;
  readonly label: string | null;
  readonly commit: string | null;
  readonly profile: VoxelRpgVerificationBudgetProfile;
  readonly budgets: RenderVerificationThresholds;
  readonly evidenceBundle: VoxelRpgEvidenceBundle;
  readonly hitchSummary: VoxelRpgHitchSummary;
  readonly rpgGateGroups: VoxelRpgVerificationGateGroup[];
  readonly summary: VoxelRpgVerificationSummary;
  readonly renderVerification: RenderVerificationReport;
}

export function buildVoxelRpgVerificationReport(
  renderVerification: RenderVerificationReport,
  options: BuildVoxelRpgVerificationReportOptions = {},
): VoxelRpgVerificationReport {
  const profile = resolveVoxelRpgVerificationProfile(options.profile);
  const evidenceBundle = buildVoxelRpgEvidenceBundle(renderVerification, profile.evidence);
  const hitchSummary = summarizeVoxelRpgHitches(options.liveForwardSamples ?? [], profile.hitches);
  const rpgGateGroups = [
    buildEvidenceFreshnessGateGroup(evidenceBundle, profile.evidence),
    buildHitchGateGroup(hitchSummary, profile.hitches, options.liveForwardSamples ?? []),
  ];
  const rpgFailures = rpgGateGroups.flatMap((group) => group.failures);
  return {
    schemaVersion: 1,
    generatedAt: renderVerification.generatedAt,
    label: renderVerification.label,
    commit: renderVerification.commit,
    profile,
    budgets: profile.renderThresholds,
    evidenceBundle,
    hitchSummary,
    rpgGateGroups,
    summary: summarizeVoxelRpgVerificationReport(renderVerification, rpgFailures),
    renderVerification,
  };
}

export function summarizeVoxelRpgVerificationReport(
  report: RenderVerificationReport,
  extraFailures: readonly RenderVerificationFailure[] = [],
): VoxelRpgVerificationSummary {
  const byCategory: Record<VoxelRpgVerificationFailureCategory, RenderVerificationFailure[]> = {
    artifact: [],
    correctness: [],
    performance: [],
    settle: [],
  };
  const failures = [...report.failures, ...extraFailures];
  for (const failure of failures) {
    byCategory[classifyVoxelRpgVerificationFailure(failure)].push(failure);
  }

  const failureCount = failures.length;
  return {
    status: failureCount === 0 ? "pass" : "fail",
    failureCount,
    correctnessFailures: categorySummary(byCategory.correctness),
    performanceFailures: categorySummary(byCategory.performance),
    settleFailures: categorySummary(byCategory.settle),
    artifactFailures: categorySummary(byCategory.artifact),
  };
}

export function classifyVoxelRpgVerificationFailure(
  failure: Pick<RenderVerificationFailure, "groupId" | "gateId">,
): VoxelRpgVerificationFailureCategory {
  if (
    failure.groupId === "artifact-inventory"
    || failure.groupId === "rpg-evidence-freshness"
    || failure.gateId.endsWith(".artifact_missing")
    || failure.gateId.startsWith("evidence.")
  ) {
    return "artifact";
  }
  if (failure.groupId === "view-settle" || failure.gateId.startsWith("view.settle_")) {
    return "settle";
  }
  if (
    failure.groupId === "rpg-hitch-gates"
    || failure.gateId.startsWith("hitch.")
    || failure.gateId.includes(".p95_")
    || failure.gateId.includes(".max_")
    || failure.gateId.includes(".fps_")
    || failure.gateId.endsWith(".samples_min")
  ) {
    return "performance";
  }
  return "correctness";
}

export function resolveVoxelRpgVerificationProfile(
  profile: VoxelRpgVerificationProfileId | VoxelRpgVerificationBudgetProfile | null | undefined,
): VoxelRpgVerificationBudgetProfile {
  if (!profile) {
    return VOXEL_RPG_VERIFICATION_PROFILES[DEFAULT_VOXEL_RPG_VERIFICATION_PROFILE_ID];
  }
  if (typeof profile !== "string") {
    return profile;
  }
  const resolved = VOXEL_RPG_VERIFICATION_PROFILES[profile];
  if (!resolved) {
    throw new Error(`Unknown RPG verification profile: ${profile}`);
  }
  return resolved;
}

export function buildVoxelRpgEvidenceBundle(
  report: RenderVerificationReport,
  budgets: VoxelRpgEvidenceBudgets,
): VoxelRpgEvidenceBundle {
  const generatedAtMs = Date.parse(report.generatedAt);
  const artifacts = report.inputs.map((input) => {
    const manifest = report.commandManifest.find((entry) => entry.id === input.id);
    const inputGeneratedAtMs = input.generatedAt ? Date.parse(input.generatedAt) : Number.NaN;
    const ageMs = Number.isFinite(generatedAtMs) && Number.isFinite(inputGeneratedAtMs)
      ? generatedAtMs - inputGeneratedAtMs
      : null;
    return {
      id: input.id,
      label: manifest?.label ?? input.id,
      command: manifest?.command ?? [],
      path: input.path,
      samplePath: input.samplePath,
      present: input.present,
      generatedAt: input.generatedAt,
      commit: input.commit,
      ageMs,
      status: evidenceArtifactStatus(input.present, input.generatedAt, ageMs, budgets),
    };
  });
  return {
    generatedAt: report.generatedAt,
    commit: report.commit,
    maxArtifactAgeMs: budgets.maxArtifactAgeMs,
    maxClockSkewMs: budgets.maxClockSkewMs,
    artifacts,
  };
}

export function summarizeVoxelRpgHitches(
  samples: readonly unknown[],
  budgets: VoxelRpgHitchBudgets,
): VoxelRpgHitchSummary {
  const frameDeltas = samples
    .map((sample) => readNumber(sample, "gameplayFrameMs"))
    .filter((value): value is number => value !== null);
  const frameSummary = summarizeFrameDeltas(frameDeltas);
  const hitchSamples = samples.filter((sample) => (readNumber(sample, "gameplayFrameMs") ?? 0) > budgets.hitchFrameMs);
  const lodWorkHitchFrames = samples.filter((sample) =>
    Math.max(readNumber(sample, "lodMs") ?? 0, readNumber(sample, "lodMaxChunkMs") ?? 0) > budgets.maxLodWorkMs
  ).length;
  return {
    sampleCount: samples.length,
    hitchFrameMs: budgets.hitchFrameMs,
    frameCount: frameSummary.frameCount,
    p95FrameMs: roundMetric(frameSummary.p95Ms),
    maxFrameMs: roundMetric(frameSummary.maxMs),
    hitchFrames: hitchSamples.length,
    movementHitchFrames: hitchSamples.filter((sample) => readString(sample, "phase") === "move").length,
    settleHitchFrames: hitchSamples.filter((sample) => readString(sample, "phase") === "settle").length,
    droppedFrameEstimate: frameSummary.droppedFrameEstimate,
    maxLodWorkMs: budgets.maxLodWorkMs,
    lodWorkHitchFrames,
  };
}

function categorySummary(failures: RenderVerificationFailure[]): VoxelRpgFailureCategorySummary {
  return {
    count: failures.length,
    failures,
  };
}

function buildEvidenceFreshnessGateGroup(
  bundle: VoxelRpgEvidenceBundle,
  budgets: VoxelRpgEvidenceBudgets,
): VoxelRpgVerificationGateGroup {
  return buildRpgGateGroup("rpg-evidence-freshness", "RPG evidence freshness", [
    countGate(
      "evidence.missing_generated_at_count",
      "Artifacts missing generatedAt",
      bundle.artifacts.filter((artifact) => artifact.status === "missing-generated-at").length,
      budgets.requireGeneratedAt ? 0 : Number.POSITIVE_INFINITY,
      { artifactIds: artifactIds(bundle, "missing-generated-at") },
    ),
    countGate(
      "evidence.stale_artifact_count",
      "Stale artifact count",
      bundle.artifacts.filter((artifact) => artifact.status === "stale").length,
      0,
      { artifactIds: artifactIds(bundle, "stale") },
    ),
    countGate(
      "evidence.future_artifact_count",
      "Future-dated artifact count",
      bundle.artifacts.filter((artifact) => artifact.status === "future").length,
      0,
      { artifactIds: artifactIds(bundle, "future") },
    ),
    countGate(
      "evidence.commit_mismatch_count",
      "Artifact commit mismatch count",
      budgets.requireMatchingCommit ? commitMismatchCount(bundle) : 0,
      0,
      { artifactIds: commitMismatchArtifactIds(bundle) },
    ),
  ]);
}

function buildHitchGateGroup(
  summary: VoxelRpgHitchSummary,
  budgets: VoxelRpgHitchBudgets,
  samples: readonly unknown[],
): VoxelRpgVerificationGateGroup {
  if (samples.length === 0) {
    return buildRpgGateGroup("rpg-hitch-gates", "RPG hitch gates", [
      {
        id: "hitch.samples_missing",
        label: "Live-forward samples not provided",
        status: "skip",
        value: false,
        threshold: "live-forward samples provided",
      },
    ]);
  }
  return buildRpgGateGroup("rpg-hitch-gates", "RPG hitch gates", [
    countGate("hitch.frames_over_budget", "Frames over hitch budget", summary.hitchFrames, budgets.maxHitchFrames),
    countGate(
      "hitch.movement_frames_over_budget",
      "Movement frames over hitch budget",
      summary.movementHitchFrames,
      budgets.maxMovementHitchFrames,
    ),
    countGate(
      "hitch.settle_frames_over_budget",
      "Settle frames over hitch budget",
      summary.settleHitchFrames,
      budgets.maxSettleHitchFrames,
    ),
    countGate(
      "hitch.dropped_frame_estimate",
      "Dropped frame estimate",
      summary.droppedFrameEstimate,
      budgets.maxDroppedFrameEstimate,
    ),
    countGate(
      "hitch.lod_work_frames_over_budget",
      "LOD work frames over budget",
      summary.lodWorkHitchFrames,
      budgets.maxLodWorkHitchFrames,
    ),
  ]);
}

function buildRpgGateGroup(
  id: string,
  label: string,
  gates: readonly VoxelRpgVerificationGate[],
): VoxelRpgVerificationGateGroup {
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
    gates: [...gates],
    failures,
  };
}

function countGate(
  id: string,
  label: string,
  value: number,
  maximum: number,
  details?: Record<string, unknown>,
): VoxelRpgVerificationGate {
  return {
    id,
    label,
    status: value <= maximum ? "pass" : "fail",
    value,
    threshold: `<= ${maximum}`,
    details,
  };
}

function evidenceArtifactStatus(
  present: boolean,
  generatedAt: string | null,
  ageMs: number | null,
  budgets: VoxelRpgEvidenceBudgets,
): VoxelRpgEvidenceArtifact["status"] {
  if (!present) {
    return "missing";
  }
  if (!generatedAt || ageMs === null) {
    return "missing-generated-at";
  }
  if (ageMs > budgets.maxArtifactAgeMs) {
    return "stale";
  }
  if (ageMs < -budgets.maxClockSkewMs) {
    return "future";
  }
  return "fresh";
}

function artifactIds(bundle: VoxelRpgEvidenceBundle, status: VoxelRpgEvidenceArtifact["status"]): string[] {
  return bundle.artifacts.filter((artifact) => artifact.status === status).map((artifact) => artifact.id);
}

function commitMismatchCount(bundle: VoxelRpgEvidenceBundle): number {
  return commitMismatchArtifactIds(bundle).length;
}

function commitMismatchArtifactIds(bundle: VoxelRpgEvidenceBundle): string[] {
  if (!bundle.commit) {
    return [];
  }
  return bundle.artifacts
    .filter((artifact) => artifact.present && artifact.commit !== null && artifact.commit !== bundle.commit)
    .map((artifact) => artifact.id);
}

function readNumber(value: unknown, key: string): number | null {
  const record = asRecord(value);
  const raw = record[key];
  return typeof raw === "number" && Number.isFinite(raw) ? raw : null;
}

function readString(value: unknown, key: string): string | null {
  const record = asRecord(value);
  const raw = record[key];
  return typeof raw === "string" ? raw : null;
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" ? value as Record<string, unknown> : {};
}

function roundMetric(value: number): number {
  return Math.round(value * 1000) / 1000;
}
