import { mkdir, writeFile } from "node:fs/promises";
import { dirname } from "node:path";

import {
  buildRenderVerificationRunnerReport,
  loadRenderVerificationArtifacts,
  resolveDefaultRenderVerificationArtifactPaths,
  type RenderVerificationArtifactPaths,
  type RenderVerificationThresholds,
} from "./lib/render-verification-runner.ts";

interface CliOptions {
  readonly label: string | null;
  readonly output: string | null;
  readonly artifactRoot: string;
  readonly useLatestArtifacts: boolean;
  readonly paths: RenderVerificationArtifactPaths;
  readonly thresholds: Partial<RenderVerificationThresholds>;
}

const options = parseCli(Bun.argv);
const defaultPaths = options.useLatestArtifacts
  ? await resolveDefaultRenderVerificationArtifactPaths(options.artifactRoot)
  : {};
const paths = mergePaths(defaultPaths, options.paths);
const artifacts = await loadRenderVerificationArtifacts(paths);
const report = buildRenderVerificationRunnerReport({
  generatedAt: new Date().toISOString(),
  label: options.label,
  commit: readGitShortHead(),
  thresholds: options.thresholds,
  paths,
  artifacts,
});

const serialized = `${JSON.stringify(report, null, 2)}\n`;
if (options.output) {
  await mkdir(dirname(options.output), { recursive: true });
  await writeFile(options.output, serialized);
} else {
  console.log(serialized.trimEnd());
}

if (report.failures.length > 0) {
  process.exitCode = 1;
}

function parseCli(argv: readonly string[]): CliOptions {
  const args = argv.slice(2);
  return {
    label: readFlag(args, "--label"),
    output: readFlag(args, "--output"),
    artifactRoot: readFlag(args, "--artifact-root") ?? "artifacts",
    useLatestArtifacts: readBooleanFlag(args, "--latest-artifacts", true),
    paths: {
      routeAtlasReport: readFlag(args, "--route-atlas-report"),
      liveForwardReport: readFlag(args, "--live-forward-report"),
      liveForwardSamples: readFlag(args, "--live-forward-samples"),
      lodReport: readFlag(args, "--lod-report"),
      viewAtlasReport: readFlag(args, "--view-atlas-report"),
    },
    thresholds: {
      minRouteSamples: readOptionalNonNegativeInt(args, "--min-route-samples"),
      minLiveForwardSamples: readOptionalNonNegativeInt(args, "--min-live-forward-samples"),
      minViewCount: readOptionalNonNegativeInt(args, "--min-view-count"),
      minLodIterations: readOptionalNonNegativeInt(args, "--min-lod-iterations"),
      maxP95FrameMs: readOptionalPositiveFloat(args, "--max-p95-frame-ms"),
      maxFrameMs: readOptionalPositiveFloat(args, "--max-frame-ms"),
      maxFpsErrorRatio: readOptionalPositiveFloat(args, "--max-fps-error-ratio"),
      requireArtifacts: readOptionalBooleanFlag(args, "--require-artifacts"),
    },
  };
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

function readOptionalBooleanFlag(args: readonly string[], name: string): boolean | undefined {
  if (!args.some((arg) => arg === name || arg.startsWith(`${name}=`))) {
    return undefined;
  }
  return readBooleanFlag(args, name, true);
}

function readOptionalNonNegativeInt(args: readonly string[], name: string): number | undefined {
  const value = readFlag(args, name);
  if (value === null) {
    return undefined;
  }
  const parsed = Number(value);
  if (!Number.isInteger(parsed) || parsed < 0) {
    throw new Error(`${name} must be a non-negative integer`);
  }
  return parsed;
}

function readOptionalPositiveFloat(args: readonly string[], name: string): number | undefined {
  const value = readFlag(args, name);
  if (value === null) {
    return undefined;
  }
  const parsed = Number(value);
  if (!Number.isFinite(parsed) || parsed <= 0) {
    throw new Error(`${name} must be a positive number`);
  }
  return parsed;
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
