import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { rustTool } from "../../scripts/build-wasm.ts";
import { ScenarioArguments } from "../lib/arguments.ts";
import { runProcess } from "../lib/process.ts";
import { defineScenario, type ScenarioContext } from "../lib/scenario.ts";

interface CandidateReport {
  readonly kind: string;
  readonly buildMs: number;
  readonly traceMs: number;
  readonly logicalBytes: number;
  readonly primitiveCount: number;
  readonly volumetricExceptionColumns: number;
  readonly rays: number;
  readonly referenceHits: number;
  readonly ownerlessReferenceHits: number;
  readonly inventedHits: number;
  readonly materialMismatches: number;
  readonly depthMismatches: number;
  readonly maximumDepthErrorVoxels: number;
}

interface PoseReport {
  readonly schema: string;
  readonly capture: { readonly pose: number };
  readonly source: { readonly identityHash: string };
  readonly candidates: readonly CandidateReport[];
  readonly clusteredIncrementalBuild: null | {
    readonly pageEdgeVoxels: number;
    readonly pageCount: number;
    readonly editSamples: number;
    readonly boundaryEditSamples: number;
    readonly maximumRebuiltPages: number;
    readonly rebuildMs: { readonly p95: number; readonly p99: number };
    readonly exactRebuildViolations: number;
  };
  readonly gpu: null | {
    readonly schema: string;
    readonly exactClusterRaster: {
      readonly schema: string;
      readonly adapter: Readonly<Record<string, unknown>>;
      readonly workload: Readonly<Record<string, unknown>>;
      readonly gpuMs: {
        readonly shadow: { readonly p95: number };
        readonly color: { readonly p95: number };
        readonly total: { readonly p95: number; readonly p99: number };
      };
      readonly allocatedBytes: Readonly<Record<string, number>>;
    };
    readonly denseVoxelRayCasterLowerBound: {
      readonly schema: string;
      readonly classification: string;
      readonly workload: Readonly<Record<string, unknown>>;
      readonly gpuMs: {
        readonly shadow: { readonly p95: number };
        readonly color: { readonly p95: number };
        readonly total: { readonly p95: number; readonly p99: number };
      };
      readonly readbackVerification: {
        readonly primaryHitPixels: number;
        readonly shadowHitPixels: number;
        readonly invalidShadowPixels: number;
        readonly fnv1a64: string;
      };
      readonly allocatedBytes: Readonly<Record<string, number>>;
    };
  };
}

function percentile(values: readonly number[], quantile: number): number {
  if (values.length === 0) return 0;
  const sorted = [...values].sort((left, right) => left - right);
  return sorted[Math.min(Math.ceil(quantile * sorted.length) - 1, sorted.length - 1)] ?? 0;
}

function candidateSummary(reports: readonly PoseReport[], kind: string) {
  const candidates = reports.map((report) => {
    const candidate = report.candidates.find((entry) => entry.kind === kind);
    if (candidate === undefined) {
      throw new Error(`pose ${report.capture.pose} omitted candidate ${kind}`);
    }
    return candidate;
  });
  const trace = candidates.map((candidate) => candidate.traceMs);
  const build = candidates.map((candidate) => candidate.buildMs);
  return {
    kind,
    traceMs: {
      p50: percentile(trace, 0.5),
      p95: percentile(trace, 0.95),
      p99: percentile(trace, 0.99),
    },
    buildMs: {
      p50: percentile(build, 0.5),
      p95: percentile(build, 0.95),
      p99: percentile(build, 0.99),
    },
    logicalBytes: {
      minimum: Math.min(...candidates.map((candidate) => candidate.logicalBytes)),
      maximum: Math.max(...candidates.map((candidate) => candidate.logicalBytes)),
    },
    primitiveCount: {
      minimum: Math.min(...candidates.map((candidate) => candidate.primitiveCount)),
      maximum: Math.max(...candidates.map((candidate) => candidate.primitiveCount)),
    },
    volumetricExceptionColumns: candidates.reduce(
      (sum, candidate) => sum + candidate.volumetricExceptionColumns,
      0,
    ),
    referenceHits: candidates.reduce((sum, candidate) => sum + candidate.referenceHits, 0),
    ownerlessReferenceHits: candidates.reduce(
      (sum, candidate) => sum + candidate.ownerlessReferenceHits,
      0,
    ),
    inventedHits: candidates.reduce((sum, candidate) => sum + candidate.inventedHits, 0),
    materialMismatches: candidates.reduce(
      (sum, candidate) => sum + candidate.materialMismatches,
      0,
    ),
    depthMismatches: candidates.reduce((sum, candidate) => sum + candidate.depthMismatches, 0),
    maximumDepthErrorVoxels: Math.max(
      ...candidates.map((candidate) => candidate.maximumDepthErrorVoxels),
    ),
  };
}

async function run(context: ScenarioContext, rawArguments: readonly string[]) {
  const arguments_ = new ScenarioArguments(rawArguments);
  const edge =
    arguments_.number("edge", { fallback: 128, integer: true, minimum: 32, maximum: 512 }) ?? 128;
  const rays = arguments_.pair("rays", {
    fallback: [320, 180],
    separator: "x",
    integer: true,
    minimum: 16,
    maximum: 2048,
  }) ?? [320, 180];
  const poses = arguments_.flag("single-pose") ? [1] : [1, 2, 3];
  arguments_.assertEmpty();

  await runProcess(
    context,
    rustTool("cargo"),
    [
      "build",
      "--profile",
      "worldgen",
      "-p",
      "voxels-virtual-surface-bakeoff",
      "--features",
      "terrain-metal",
      "--bin",
      "voxels-virtual-surface-bakeoff",
    ],
    {
      label: "virtual surface bake-off build",
      stdio: "inherit",
    },
  );
  const executable = resolve("target/worldgen/voxels-virtual-surface-bakeoff");
  const reports: PoseReport[] = [];
  for (const pose of poses) {
    context.log(`sampling supplied Terrain Diffusion pose ${pose}`);
    const output = context.artifacts.resolve(`pose-${pose}.json`);
    await runProcess(
      context,
      executable,
      [
        `--pose=${pose}`,
        `--edge=${edge}`,
        `--rays=${rays[0]}x${rays[1]}`,
        ...(pose === 2 ? ["--gpu"] : []),
        `--output=${output}`,
      ],
      {
        label: `virtual surface pose ${pose}`,
        stdio: "inherit",
      },
    );
    context.artifacts.record(`Virtual surface pose ${pose}`, output, "application/json");
    const report = JSON.parse(await readFile(output, "utf8")) as PoseReport;
    if (report.schema !== "voxels.virtual-surface-bakeoff.v1" || report.capture.pose !== pose) {
      throw new Error(`pose ${pose} produced an incompatible bake-off report`);
    }
    if (report.candidates.some((candidate) => candidate.referenceHits === 0)) {
      throw new Error(`pose ${pose} did not exercise visible canonical terrain`);
    }
    reports.push(report);
  }
  context.log("sampling deterministic cave, overhang, water, and floating-voxel stress volume");
  const topologyOutput = context.artifacts.resolve("topology-stress.json");
  await runProcess(
    context,
    executable,
    ["--fixture=topology-stress", `--rays=${rays[0]}x${rays[1]}`, `--output=${topologyOutput}`],
    {
      label: "virtual surface topology stress",
      stdio: "inherit",
    },
  );
  context.artifacts.record("Virtual surface topology stress", topologyOutput, "application/json");
  const topology = JSON.parse(await readFile(topologyOutput, "utf8")) as PoseReport;
  if (topology.schema !== "voxels.virtual-surface-bakeoff.v1" || topology.capture.pose !== 0) {
    throw new Error("topology stress produced an incompatible bake-off report");
  }
  const topologyStepped = topology.candidates.find(
    (candidate) => candidate.kind === "stepped-surface",
  );
  if (
    topologyStepped === undefined ||
    topologyStepped.volumetricExceptionColumns === 0 ||
    topologyStepped.ownerlessReferenceHits === 0
  ) {
    throw new Error(
      "topology stress did not prove the pure stepped-surface candidate is incomplete",
    );
  }
  const topologyExactViolations = topology.candidates
    .filter((candidate) => candidate.kind !== "stepped-surface")
    .flatMap((candidate) => [
      candidate.ownerlessReferenceHits,
      candidate.inventedHits,
      candidate.materialMismatches,
      candidate.depthMismatches,
    ])
    .reduce((sum, value) => sum + value, 0);
  if (topologyExactViolations !== 0) {
    throw new Error(
      `topology stress found ${topologyExactViolations} errors in exact-capable candidates`,
    );
  }
  const kinds = [
    "exact-greedy",
    "stepped-surface",
    "clustered-virtual-geometry",
    "sparse-brick-ray-caster",
  ];
  const candidates = kinds.map((kind) => candidateSummary(reports, kind));
  const gpu = reports.find((report) => report.gpu !== null)?.gpu;
  const incrementalBuilds = reports.map((report) => report.clusteredIncrementalBuild);
  if (
    gpu === null ||
    gpu === undefined ||
    gpu.schema !== "voxels.virtual-surface-gpu-competition.v1" ||
    gpu.exactClusterRaster.schema !== "voxels.virtual-surface-gpu-bakeoff.v1" ||
    gpu.denseVoxelRayCasterLowerBound.schema !==
      "voxels.virtual-surface-dense-voxel-gpu-bakeoff.v1" ||
    !Number.isFinite(gpu.exactClusterRaster.gpuMs.total.p95) ||
    !Number.isFinite(gpu.denseVoxelRayCasterLowerBound.gpuMs.total.p95) ||
    gpu.denseVoxelRayCasterLowerBound.readbackVerification.primaryHitPixels === 0 ||
    gpu.denseVoxelRayCasterLowerBound.readbackVerification.shadowHitPixels === 0 ||
    gpu.denseVoxelRayCasterLowerBound.readbackVerification.invalidShadowPixels !== 0
  ) {
    throw new Error("supplied-world bake-off omitted valid, nontrivial Metal/WGPU results");
  }
  if (
    incrementalBuilds.some(
      (build) =>
        build === null ||
        build.editSamples < 16 ||
        build.boundaryEditSamples === 0 ||
        build.maximumRebuiltPages > 7 ||
        build.exactRebuildViolations !== 0 ||
        !Number.isFinite(build.rebuildMs.p95),
    )
  ) {
    throw new Error("clustered page-local edit rebuild gate failed");
  }
  const rayCasterPrimaryDecision =
    gpu.denseVoxelRayCasterLowerBound.gpuMs.total.p95 > 4
      ? "rejected: optimistic dense traversal already exceeds 4ms p95 terrain budget"
      : "continue: sparse traversal still requires a full implementation benchmark";
  const exactCandidates = candidates.filter(
    (candidate) =>
      candidate.kind !== "stepped-surface" || candidate.volumetricExceptionColumns === 0,
  );
  const correctnessViolations = exactCandidates.flatMap((candidate) => [
    ...(candidate.ownerlessReferenceHits === 0
      ? []
      : [`${candidate.kind} missed ${candidate.ownerlessReferenceHits} reference hits`]),
    ...(candidate.inventedHits === 0
      ? []
      : [`${candidate.kind} invented ${candidate.inventedHits} hits`]),
    ...(candidate.materialMismatches === 0
      ? []
      : [`${candidate.kind} confused ${candidate.materialMismatches} material hits`]),
    ...(candidate.depthMismatches === 0
      ? []
      : [`${candidate.kind} changed ${candidate.depthMismatches} hit depths`]),
  ]);
  if (correctnessViolations.length > 0) {
    throw new Error(`virtual surface correctness gate failed: ${correctnessViolations.join("; ")}`);
  }
  const summary = {
    schema: "voxels.virtual-surface-bakeoff-summary.v1",
    suppliedPoses: poses,
    regionEdgeVoxels: edge,
    rayGrid: rays,
    sourceIdentityHashes: [...new Set(reports.map((report) => report.source.identityHash))],
    candidates,
    gpu,
    decisions: {
      pureSteppedSurface: "rejected: cannot conservatively represent required topology",
      sparseBrickPrimaryRenderer: rayCasterPrimaryDecision,
      exactClusterRaster: "continue to hierarchy scaling and edit-build tests",
    },
    clusteredIncrementalBuild: incrementalBuilds,
    topologyStress: {
      steppedSurface: topologyStepped,
      exactCapableViolations: topologyExactViolations,
    },
  };
  await context.artifacts.writeJson("Virtual surface bake-off summary", "summary.json", summary);
  return {
    summary:
      "All four disposable representations ran against the same exact supplied-world oracle.",
    metrics: {
      poses: poses.length,
      raysPerPose: rays[0] * rays[1],
      candidates: candidates.length,
      correctnessViolations: correctnessViolations.length,
      steppedSurfaceExceptionColumns: topologyStepped.volumetricExceptionColumns,
      steppedSurfaceOwnerlessHits: topologyStepped.ownerlessReferenceHits,
      clusteredRasterGpuP95Ms: gpu.exactClusterRaster.gpuMs.total.p95,
      denseVoxelRayCasterGpuP95Ms: gpu.denseVoxelRayCasterLowerBound.gpuMs.total.p95,
      clusteredEditRebuildP95Ms: Math.max(
        ...incrementalBuilds.map((build) => build?.rebuildMs.p95 ?? Number.POSITIVE_INFINITY),
      ),
    },
    details: summary,
  };
}

export default defineScenario({
  id: "virtual-surface-bakeoff",
  kind: "benchmark",
  summary:
    "Compare exact greedy, stepped, clustered, and sparse-brick terrain representations on supplied captures.",
  uses: { world: true, metrics: true, rust: true },
  timeoutMs: 900_000,
  run,
});
