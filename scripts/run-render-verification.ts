import { mkdir, writeFile } from "node:fs/promises";
import { dirname } from "node:path";

import {
  buildRenderVerificationReport,
  buildScenarioResult,
  type RenderMetric,
} from "./lib/render-verification-metrics.ts";

interface CliOptions {
  label: string | null;
  output: string | null;
}

const options = parseCli(Bun.argv);
const report = buildRenderVerificationReport({
  generatedAt: new Date().toISOString(),
  label: options.label,
  commit: readGitShortHead(),
  scenarios: [
    buildScenarioResult("smoke-skeleton", "Render verification smoke skeleton", [
      {
        id: "skeleton.report_shape",
        label: "Report shape emitted",
        value: true,
        status: "pass",
      } satisfies RenderMetric,
    ]),
  ],
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
