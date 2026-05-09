import { mkdir, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";

import {
  buildRenderVerificationRunnerReport,
  loadRenderVerificationArtifacts,
  resolveDefaultRenderVerificationArtifactPaths,
  type RenderVerificationArtifactPaths,
} from "./lib/render-verification-runner.ts";
import {
  buildVoxelRpgVerificationReport,
  resolveVoxelRpgVerificationProfile,
  type VoxelRpgVerificationProfileId,
} from "./lib/voxel-rpg-budgets.ts";

interface CliOptions {
  readonly label: string;
  readonly output: string | null;
  readonly artifactRoot: string;
  readonly useLatestArtifacts: boolean;
  readonly profile: VoxelRpgVerificationProfileId;
  readonly paths: RenderVerificationArtifactPaths;
}

const options = parseCli(Bun.argv);
const generatedAt = new Date().toISOString();
const profile = resolveVoxelRpgVerificationProfile(options.profile);
const defaultPaths = options.useLatestArtifacts
  ? await resolveDefaultRenderVerificationArtifactPaths(options.artifactRoot)
  : {};
const paths = mergePaths(defaultPaths, options.paths);
const artifacts = await loadRenderVerificationArtifacts(paths);
const renderVerification = buildRenderVerificationRunnerReport({
  generatedAt,
  label: options.label,
  commit: readGitShortHead(),
  thresholds: profile.renderThresholds,
  paths,
  artifacts,
});
const report = buildVoxelRpgVerificationReport(renderVerification, {
  profile,
  liveForwardSamples: artifacts.liveForwardSamples,
});
const outputPath = options.output ?? defaultOutputPath(options.artifactRoot, options.label, generatedAt);

await mkdir(dirname(outputPath), { recursive: true });
await writeFile(outputPath, `${JSON.stringify(report, null, 2)}\n`);

console.log(`[verify:rpg] report: ${outputPath}`);
console.log(`[verify:rpg] profile: ${report.profile.id}`);
console.log(`[verify:rpg] status: ${report.summary.status}`);
console.log(
  `[verify:rpg] failures correctness/performance/settle/artifact: `
    + `${report.summary.correctnessFailures.count}/`
    + `${report.summary.performanceFailures.count}/`
    + `${report.summary.settleFailures.count}/`
    + `${report.summary.artifactFailures.count}`,
);
for (const failure of report.renderVerification.failures) {
  console.log(`[verify:rpg] ${failure.groupId}.${failure.gateId}: ${failure.message}`);
}
for (const failure of report.rpgGateGroups.flatMap((group) => group.failures)) {
  console.log(`[verify:rpg] ${failure.groupId}.${failure.gateId}: ${failure.message}`);
}

if (report.summary.status === "fail") {
  process.exitCode = 1;
}

function parseCli(argv: readonly string[]): CliOptions {
  const args = argv.slice(2);
  return {
    label: readFlag(args, "--label") ?? "voxel-rpg-verification",
    output: readFlag(args, "--output"),
    artifactRoot: readFlag(args, "--artifact-root") ?? "artifacts",
    useLatestArtifacts: readBooleanFlag(args, "--latest-artifacts", true),
    profile: readProfile(readFlag(args, "--profile")),
    paths: {
      routeAtlasReport: readFlag(args, "--route-atlas-report"),
      liveForwardReport: readFlag(args, "--live-forward-report"),
      liveForwardSamples: readFlag(args, "--live-forward-samples"),
      lodReport: readFlag(args, "--lod-report"),
      viewAtlasReport: readFlag(args, "--view-atlas-report"),
    },
  };
}

function readProfile(value: string | null): VoxelRpgVerificationProfileId {
  if (value === null || value === "rpg-render-gate") {
    return "rpg-render-gate";
  }
  if (value === "rpg-render-smoke") {
    return "rpg-render-smoke";
  }
  throw new Error(`Unsupported --profile value: ${value}`);
}

function mergePaths(
  defaults: RenderVerificationArtifactPaths,
  overrides: RenderVerificationArtifactPaths,
): RenderVerificationArtifactPaths {
  return {
    routeAtlasReport: overrides.routeAtlasReport ?? defaults.routeAtlasReport ?? null,
    liveForwardReport: overrides.liveForwardReport ?? defaults.liveForwardReport ?? null,
    liveForwardSamples: overrides.liveForwardSamples ?? defaults.liveForwardSamples ?? null,
    lodReport: overrides.lodReport ?? defaults.lodReport ?? null,
    viewAtlasReport: overrides.viewAtlasReport ?? defaults.viewAtlasReport ?? null,
  };
}

function readFlag(args: readonly string[], name: string): string | null {
  const prefix = `${name}=`;
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index]!;
    if (arg === name) {
      return args[index + 1] ?? null;
    }
    if (arg.startsWith(prefix)) {
      return arg.slice(prefix.length);
    }
  }
  return null;
}

function readBooleanFlag(args: readonly string[], name: string, fallback: boolean): boolean {
  const value = readFlag(args, name);
  if (value === null) {
    return fallback;
  }
  return value !== "false" && value !== "0";
}

function defaultOutputPath(artifactRoot: string, label: string, generatedAt: string): string {
  return join(
    artifactRoot,
    "voxel-rpg-verification",
    `${timestampSlug(generatedAt)}-${safeSlug(label)}`,
    "report.json",
  );
}

function timestampSlug(value: string): string {
  return value.replaceAll(":", "").replaceAll(".", "").replaceAll("T", "-").replaceAll("Z", "Z");
}

function safeSlug(value: string): string {
  return value.trim().toLowerCase().replace(/[^a-z0-9._-]+/g, "-").replace(/^-+|-+$/g, "") || "run";
}

function readGitShortHead(): string | null {
  const result = Bun.spawnSync(["git", "rev-parse", "--short", "HEAD"], {
    stdout: "pipe",
    stderr: "pipe",
  });
  if (!result.success) {
    return null;
  }
  const value = result.stdout.toString().trim();
  return value || null;
}
