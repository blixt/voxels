import type {
  RenderVerificationFailure,
  RenderVerificationReport,
  RenderVerificationThresholds,
} from "./render-verification-runner.ts";

export const VOXEL_RPG_VERIFICATION_BUDGETS = {
  minRouteSamples: 100,
  minLiveForwardSamples: 120,
  minViewCount: 1,
  minLodIterations: 1,
  maxP95FrameMs: 16.67,
  maxFrameMs: 50,
  maxFpsErrorRatio: 0.10,
  requireArtifacts: true,
} satisfies RenderVerificationThresholds;

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

export interface VoxelRpgVerificationReport {
  readonly schemaVersion: 1;
  readonly generatedAt: string;
  readonly label: string | null;
  readonly commit: string | null;
  readonly budgets: RenderVerificationThresholds;
  readonly summary: VoxelRpgVerificationSummary;
  readonly renderVerification: RenderVerificationReport;
}

export function buildVoxelRpgVerificationReport(
  renderVerification: RenderVerificationReport,
): VoxelRpgVerificationReport {
  return {
    schemaVersion: 1,
    generatedAt: renderVerification.generatedAt,
    label: renderVerification.label,
    commit: renderVerification.commit,
    budgets: VOXEL_RPG_VERIFICATION_BUDGETS,
    summary: summarizeVoxelRpgVerificationReport(renderVerification),
    renderVerification,
  };
}

export function summarizeVoxelRpgVerificationReport(
  report: RenderVerificationReport,
): VoxelRpgVerificationSummary {
  const byCategory: Record<VoxelRpgVerificationFailureCategory, RenderVerificationFailure[]> = {
    artifact: [],
    correctness: [],
    performance: [],
    settle: [],
  };
  for (const failure of report.failures) {
    byCategory[classifyVoxelRpgVerificationFailure(failure)].push(failure);
  }

  const failureCount = report.failures.length;
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
  if (failure.groupId === "artifact-inventory" || failure.gateId.endsWith(".artifact_missing")) {
    return "artifact";
  }
  if (failure.groupId === "view-settle" || failure.gateId.startsWith("view.settle_")) {
    return "settle";
  }
  if (
    failure.gateId.includes(".p95_")
    || failure.gateId.includes(".max_")
    || failure.gateId.includes(".fps_")
    || failure.gateId.endsWith(".samples_min")
  ) {
    return "performance";
  }
  return "correctness";
}

function categorySummary(failures: RenderVerificationFailure[]): VoxelRpgFailureCategorySummary {
  return {
    count: failures.length,
    failures,
  };
}
