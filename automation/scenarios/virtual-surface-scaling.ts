import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { rustTool } from "../../scripts/build-wasm.ts";
import { runProcess } from "../lib/process.ts";
import { defineScenario, type ScenarioContext } from "../lib/scenario.ts";

interface ScalingReport {
  readonly schema: string;
  readonly region: {
    readonly bounds: {
      readonly shape: readonly [number, number, number];
    };
  };
  readonly volume: {
    readonly logicalBytes: number;
    readonly sampleMs: number;
  };
  readonly candidates: readonly [
    {
      readonly kind: string;
      readonly buildMs: number;
      readonly primitiveCount: number;
      readonly ownerlessReferenceHits: number;
      readonly inventedHits: number;
      readonly materialMismatches: number;
      readonly depthMismatches: number;
    },
  ];
  readonly gpu: {
    readonly schema: string;
    readonly exactClusterRaster: {
      readonly workload: {
        readonly quadCount: number;
        readonly measuredFrames: number;
        readonly state: string;
      };
      readonly gpuMs: {
        readonly total: { readonly p50: number; readonly p95: number; readonly p99: number };
      };
      readonly readbackVerification: {
        readonly primaryHitPixels: number;
        readonly shadowHitPixels: number;
        readonly invalidShadowPixels: number;
        readonly fnv1a64: string;
      };
    };
  };
}

async function run(context: ScenarioContext) {
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
      label: "virtual surface scaling build",
      stdio: "inherit",
    },
  );
  const executable = resolve("target/worldgen/voxels-virtual-surface-bakeoff");
  const measurements = [];
  for (const edge of [128, 256, 512]) {
    context.log(`measuring exact clustered raster over a ${edge / 10} metre source window`);
    const output = context.artifacts.resolve(`cluster-scale-${edge}.json`);
    await runProcess(
      context,
      executable,
      [
        "--pose=2",
        `--edge=${edge}`,
        "--rays=16x9",
        "--cluster-only",
        "--gpu-raster-only",
        `--output=${output}`,
      ],
      {
        label: `virtual surface ${edge}-voxel scaling`,
        stdio: "inherit",
      },
    );
    context.artifacts.record(`Virtual surface ${edge}-voxel scaling`, output, "application/json");
    const report = JSON.parse(await readFile(output, "utf8")) as ScalingReport;
    const candidate = report.candidates[0];
    const raster = report.gpu.exactClusterRaster;
    const violations =
      candidate.ownerlessReferenceHits +
      candidate.inventedHits +
      candidate.materialMismatches +
      candidate.depthMismatches;
    if (
      report.schema !== "voxels.virtual-surface-bakeoff.v1" ||
      report.gpu.schema !== "voxels.virtual-surface-gpu-raster-scaling.v1" ||
      candidate.kind !== "clustered-virtual-geometry" ||
      violations !== 0 ||
      raster.workload.measuredFrames < 32 ||
      raster.workload.state !== "night-rain-wet-material-shadow-sampled" ||
      raster.readbackVerification.primaryHitPixels === 0 ||
      raster.readbackVerification.shadowHitPixels === 0 ||
      raster.readbackVerification.invalidShadowPixels !== 0 ||
      !Number.isFinite(raster.gpuMs.total.p95)
    ) {
      throw new Error(`${edge}-voxel clustered scaling measurement is invalid`);
    }
    if (raster.gpuMs.total.p95 > 4) {
      throw new Error(
        `${edge}-voxel exact clustered raster exceeded the 4ms terrain budget: ${raster.gpuMs.total.p95}ms p95`,
      );
    }
    measurements.push({
      edgeVoxels: edge,
      shapeVoxels: report.region.bounds.shape,
      volumeLogicalBytes: report.volume.logicalBytes,
      sourceSampleMs: report.volume.sampleMs,
      clusterBuildMs: candidate.buildMs,
      quadCount: raster.workload.quadCount,
      gpuMs: raster.gpuMs.total,
      readback: raster.readbackVerification,
    });
  }
  const summary = {
    schema: "voxels.virtual-surface-scaling-summary.v1",
    pose: 2,
    measurements,
    decision:
      "Exact clustered raster remains inside the 4ms p95 terrain budget through the 51.2m supplied-world window; monolithic builds are rejected in favor of page-local construction.",
  };
  await context.artifacts.writeJson("Virtual surface scaling summary", "summary.json", summary);
  const largest = measurements.at(-1);
  return {
    summary: "Exact clustered geometry passed the three-size 4K Metal scaling gate.",
    metrics: {
      largestEdgeVoxels: largest?.edgeVoxels ?? 0,
      largestQuadCount: largest?.quadCount ?? 0,
      largestGpuP95Ms: largest?.gpuMs.p95 ?? 0,
      largestBuildMs: largest?.clusterBuildMs ?? 0,
    },
    details: summary,
  };
}

export default defineScenario({
  id: "virtual-surface-scaling",
  kind: "benchmark",
  summary:
    "Scale exact 10 cm clustered raster through 51.2m with stored and sampled shadows at 4K.",
  uses: { world: true, metrics: true, rust: true },
  timeoutMs: 900_000,
  run,
});
