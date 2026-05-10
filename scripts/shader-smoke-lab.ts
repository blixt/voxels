import { readFile } from "node:fs/promises";
import { join } from "node:path";

import { readGitShortHead, sanitizeFileStem, timestampForFile } from "./lib/browser-game-benchmark-harness.ts";

interface ShaderSmokeOptions {
  label: string;
  outputDir: string;
  viewId: string;
  compareTo: string | null;
  skipBuild: boolean;
  settleMaxFrames: number;
  viewportWidth: number;
  viewportHeight: number;
}

interface ShaderSmokeReport {
  generatedAt: string;
  label: string;
  commit: string | null;
  viewId: string;
  viewAtlasReportPath: string | null;
  visual: {
    avgLuma: number;
    lumaStdDev: number;
    quantizedColorCount: number;
    centerGridRiskScore: number;
    lowerGroundGridRiskScore: number;
  } | null;
  render: {
    frameCpuMs: number | null;
    lastGameplayFrameMs: number | null;
    drawCalls: number | null;
    triangles: number | null;
  } | null;
  failures: string[];
  artifacts: {
    report: string;
    summary: string;
  };
}

const DEFAULT_BUDGETS = {
  minAvgLuma: 10,
  minLumaStdDev: 8,
  minQuantizedColorCount: 16,
  maxFrameCpuMs: 8,
  maxLastGameplayFrameMs: 16,
  maxDrawCalls: 900,
  maxTriangles: 900_000,
};

const options = readOptions(Bun.argv);
const runName = `${timestampForFile(new Date())}-${sanitizeFileStem(options.label)}`;
const runDir = join(options.outputDir, runName);
const reportPath = join(runDir, "report.json");
const summaryPath = join(runDir, "summary.md");
await Bun.$`mkdir -p ${runDir}`.quiet();

const atlasArgs = [
  "run",
  "scripts/capture-view-atlas.ts",
  `--label=${options.label}`,
  `--views=${options.viewId}`,
  `--output-dir=${join(runDir, "view-atlas")}`,
  `--settle-max-frames=${options.settleMaxFrames}`,
  `--viewport-width=${options.viewportWidth}`,
  `--viewport-height=${options.viewportHeight}`,
];
if (options.compareTo) {
  atlasArgs.push(`--compare-to=${options.compareTo}`, "--enforce-comparison-budgets");
}
if (options.skipBuild) {
  atlasArgs.push("--skip-build");
}

let atlasStdout = "";
let atlasStderr = "";
let atlasExitCode = 0;
try {
  const result = await Bun.$`${["bun", ...atlasArgs]}`.quiet();
  atlasStdout = result.stdout.toString();
  atlasStderr = result.stderr.toString();
} catch (error) {
  atlasExitCode = typeof error === "object" && error !== null && "exitCode" in error
    ? Number((error as { exitCode?: unknown }).exitCode ?? 1)
    : 1;
  atlasStdout = typeof error === "object" && error !== null && "stdout" in error
    ? String((error as { stdout?: unknown }).stdout ?? "")
    : "";
  atlasStderr = typeof error === "object" && error !== null && "stderr" in error
    ? String((error as { stderr?: unknown }).stderr ?? "")
    : "";
}

const viewAtlasReportPath = parseReportPath(atlasStdout);
const viewAtlas = viewAtlasReportPath ? await readJsonObject(viewAtlasReportPath) : null;
const view = Array.isArray(viewAtlas?.views) ? readObject(viewAtlas.views[0]) : {};
const visualDiagnosis = readObject(readObject(readObject(view.visual).diagnosis));
const settled = readObject(readObject(view.settled).render);
const snapshot = readObject(view.snapshot);
const visual = viewAtlasReportPath
  ? {
    avgLuma: readNumber(visualDiagnosis.avgLuma),
    lumaStdDev: readNumber(visualDiagnosis.lumaStdDev),
    quantizedColorCount: readNumber(visualDiagnosis.quantizedColorCount),
    centerGridRiskScore: readNumber(visualDiagnosis.centerGridRiskScore),
    lowerGroundGridRiskScore: readNumber(visualDiagnosis.lowerGroundGridRiskScore),
  }
  : null;
const render = viewAtlasReportPath
  ? {
    frameCpuMs: readOptionalNumber(settled.frameCpuMs),
    lastGameplayFrameMs: readOptionalNumber(snapshot.lastGameplayFrameMs),
    drawCalls: readOptionalNumber(settled.drawCalls ?? snapshot.drawCalls),
    triangles: readOptionalNumber(settled.triangles ?? snapshot.triangles),
  }
  : null;

const failures = [
  ...findShaderSmokeFailures(visual, render),
  ...readAtlasFailures(viewAtlas),
];
if (atlasExitCode !== 0 && readAtlasFailures(viewAtlas).length === 0) {
  failures.push(`view-atlas command exited ${atlasExitCode}${atlasStderr ? `: ${atlasStderr.trim()}` : ""}`);
}

const report: ShaderSmokeReport = {
  generatedAt: new Date().toISOString(),
  label: options.label,
  commit: readGitShortHead(),
  viewId: options.viewId,
  viewAtlasReportPath,
  visual,
  render,
  failures,
  artifacts: {
    report: reportPath,
    summary: summaryPath,
  },
};

await Bun.write(reportPath, `${JSON.stringify(report, null, 2)}\n`);
await Bun.write(summaryPath, buildSummary(report));

console.log(`shader smoke report: ${reportPath}`);
console.log(`summary: ${summaryPath}`);
console.log(`view atlas: ${viewAtlasReportPath ?? "missing"}`);
console.log(`failures: ${failures.length > 0 ? failures.join("; ") : "none"}`);
if (failures.length > 0) {
  process.exitCode = 1;
}

function findShaderSmokeFailures(
  visual: ShaderSmokeReport["visual"],
  render: ShaderSmokeReport["render"],
): string[] {
  const failures: string[] = [];
  if (!visual) {
    failures.push("view-atlas report was not available");
  } else {
    if (visual.avgLuma < DEFAULT_BUDGETS.minAvgLuma) {
      failures.push(`avg luma ${visual.avgLuma.toFixed(1)} below ${DEFAULT_BUDGETS.minAvgLuma}`);
    }
    if (visual.lumaStdDev < DEFAULT_BUDGETS.minLumaStdDev) {
      failures.push(`luma stddev ${visual.lumaStdDev.toFixed(1)} below ${DEFAULT_BUDGETS.minLumaStdDev}`);
    }
    if (visual.quantizedColorCount < DEFAULT_BUDGETS.minQuantizedColorCount) {
      failures.push(`color buckets ${visual.quantizedColorCount} below ${DEFAULT_BUDGETS.minQuantizedColorCount}`);
    }
  }
  if (!render) {
    failures.push("render metrics were not available");
  } else {
    if ((render.frameCpuMs ?? Infinity) > DEFAULT_BUDGETS.maxFrameCpuMs) {
      failures.push(`render frame CPU ${formatOptional(render.frameCpuMs)} ms exceeds ${DEFAULT_BUDGETS.maxFrameCpuMs}`);
    }
    if ((render.lastGameplayFrameMs ?? Infinity) > DEFAULT_BUDGETS.maxLastGameplayFrameMs) {
      failures.push(`last gameplay frame ${formatOptional(render.lastGameplayFrameMs)} ms exceeds ${DEFAULT_BUDGETS.maxLastGameplayFrameMs}`);
    }
    if ((render.drawCalls ?? Infinity) > DEFAULT_BUDGETS.maxDrawCalls) {
      failures.push(`draw calls ${formatOptional(render.drawCalls)} exceed ${DEFAULT_BUDGETS.maxDrawCalls}`);
    }
    if ((render.triangles ?? Infinity) > DEFAULT_BUDGETS.maxTriangles) {
      failures.push(`triangles ${formatOptional(render.triangles)} exceed ${DEFAULT_BUDGETS.maxTriangles}`);
    }
  }
  return failures;
}

function buildSummary(report: ShaderSmokeReport): string {
  return [
    "# Shader Smoke Lab",
    "",
    `Generated: ${report.generatedAt}`,
    `Commit: ${report.commit ?? "unknown"}`,
    `View: ${report.viewId}`,
    `View atlas: ${report.viewAtlasReportPath ?? "missing"}`,
    "",
    "## Visual",
    "",
    `- Luma: ${report.visual?.avgLuma.toFixed(1) ?? "n/a"}`,
    `- Stddev: ${report.visual?.lumaStdDev.toFixed(1) ?? "n/a"}`,
    `- Colors: ${report.visual?.quantizedColorCount ?? "n/a"}`,
    `- Center grid: ${report.visual?.centerGridRiskScore.toFixed(3) ?? "n/a"}`,
    `- Lower-ground grid: ${report.visual?.lowerGroundGridRiskScore.toFixed(3) ?? "n/a"}`,
    "",
    "## Render",
    "",
    `- Frame CPU: ${formatOptional(report.render?.frameCpuMs)} ms`,
    `- Last gameplay frame: ${formatOptional(report.render?.lastGameplayFrameMs)} ms`,
    `- Draw calls: ${formatOptional(report.render?.drawCalls)}`,
    `- Triangles: ${formatOptional(report.render?.triangles)}`,
    "",
    `Failures: ${report.failures.length > 0 ? report.failures.join("; ") : "none"}`,
    "",
  ].join("\n");
}

function readOptions(argv: readonly string[]): ShaderSmokeOptions {
  const args = argv.slice(2);
  return {
    label: readFlag(args, "--label") ?? "shader-smoke",
    outputDir: readFlag(args, "--output-dir") ?? "artifacts/shader-smoke-lab",
    viewId: readFlag(args, "--view") ?? "origin-overlook",
    compareTo: readFlag(args, "--compare-to"),
    skipBuild: readSwitchFlag(args, "--skip-build"),
    settleMaxFrames: readPositiveInt(readFlag(args, "--settle-max-frames"), 90),
    viewportWidth: readPositiveInt(readFlag(args, "--viewport-width"), 960),
    viewportHeight: readPositiveInt(readFlag(args, "--viewport-height"), 600),
  };
}

function parseReportPath(stdout: string): string | null {
  return stdout.match(/view atlas report: (.+)/)?.[1]?.trim() ?? null;
}

async function readJsonObject(path: string): Promise<Record<string, unknown> | null> {
  try {
    return readObject(JSON.parse(await readFile(path, "utf8")));
  } catch {
    return null;
  }
}

function readAtlasFailures(report: Record<string, unknown> | null): string[] {
  return Array.isArray(report?.failures) ? report.failures.map(String) : [];
}

function readObject(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : {};
}

function readOptionalNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function readNumber(value: unknown): number {
  return readOptionalNumber(value) ?? 0;
}

function formatOptional(value: number | null | undefined): string {
  return typeof value === "number" && Number.isFinite(value) ? value.toFixed(value >= 100 ? 0 : 2) : "n/a";
}

function readFlag(args: readonly string[], name: string): string | null {
  const prefix = `${name}=`;
  const equals = args.find((arg) => arg.startsWith(prefix));
  if (equals) {
    return equals.slice(prefix.length);
  }
  const index = args.indexOf(name);
  return index >= 0 ? args[index + 1] ?? null : null;
}

function readSwitchFlag(args: readonly string[], name: string): boolean {
  if (!args.some((arg) => arg === name || arg.startsWith(`${name}=`))) {
    return false;
  }
  const value = readFlag(args, name);
  return value === null || value === "" || value === "1" || value === "true" || value === "yes";
}

function readPositiveInt(value: string | null, fallback: number): number {
  const parsed = Number(value);
  return Number.isFinite(parsed) && parsed > 0 ? Math.floor(parsed) : fallback;
}
