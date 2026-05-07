import { join } from "node:path";

interface SmokeCommand {
  label: string;
  command: string[];
}

interface SmokeResult {
  label: string;
  command: string[];
  exitCode: number;
  elapsedMs: number;
  stdout: string;
  stderr: string;
}

const args = Bun.argv.slice(2);
const label = readFlag(args, "--label") ?? "verify-smoke";
const includeBrowser = readBooleanFlag(args, "--browser", true);
const outputDir = readFlag(args, "--output-dir") ?? "artifacts/verify-smoke";
const runStamp = timestampForFile(new Date());
const runName = `${runStamp}-${sanitizeFileStem(label)}`;
const runDir = join(outputDir, runName);
const reportPath = join(runDir, "report.json");

await Bun.$`mkdir -p ${runDir}`.quiet();

const commands: SmokeCommand[] = [
  { label: "typecheck", command: ["bun", "run", "typecheck"] },
  {
    label: "focused-tests",
    command: [
      "bun",
      "test",
      "tests/ambient-environment.test.ts",
      "tests/water-visuals.test.ts",
      "tests/skill-journal.test.ts",
      "tests/exploration-skill-effects.test.ts",
      "tests/exploration-journal.test.ts",
      "tests/exploration-objectives.test.ts",
      "tests/benchmark-metrics.test.ts",
      "tests/game-bootstrap-benchmark.test.ts",
      "tests/game-route-benchmark.test.ts",
    ],
  },
  { label: "build", command: ["bun", "run", "build"] },
  { label: "route-atlas", command: ["bun", "run", "scripts/route-atlas.ts", `--label=${label}`] },
];

if (includeBrowser) {
  commands.push({
    label: "browser-lab",
    command: ["bun", "run", "scripts/owned-browser-lab.ts", `--label=${label}`],
  });
}

const startedAt = new Date();
const results: SmokeResult[] = [];
for (const command of commands) {
  console.log(`[verify-smoke] ${command.label}: ${command.command.join(" ")}`);
  const result = runCommand(command);
  results.push(result);
  if (result.exitCode !== 0) {
    break;
  }
}

const failures = results
  .filter((result) => result.exitCode !== 0)
  .map((result) => `${result.label} exited ${result.exitCode}`);
const report = {
  generatedAt: new Date().toISOString(),
  startedAt: startedAt.toISOString(),
  elapsedMs: Date.now() - startedAt.getTime(),
  label,
  includeBrowser,
  results,
  failures,
};
await Bun.write(reportPath, `${JSON.stringify(report, null, 2)}\n`);

console.log(`[verify-smoke] report: ${reportPath}`);
for (const result of results) {
  console.log(`[verify-smoke] ${result.label}: ${result.exitCode === 0 ? "passed" : "failed"} (${result.elapsedMs.toFixed(0)} ms)`);
}
if (failures.length > 0) {
  console.log(`[verify-smoke] failures: ${failures.join("; ")}`);
  process.exitCode = 1;
} else {
  console.log("[verify-smoke] failures: none");
}

function runCommand(smokeCommand: SmokeCommand): SmokeResult {
  const startedAt = performance.now();
  const result = Bun.spawnSync(smokeCommand.command, {
    cwd: process.cwd(),
    stdout: "pipe",
    stderr: "pipe",
  });
  const stdout = result.stdout.toString();
  const stderr = result.stderr.toString();
  if (stdout.trim().length > 0) {
    console.log(stdout.trim());
  }
  if (stderr.trim().length > 0) {
    console.error(stderr.trim());
  }
  return {
    label: smokeCommand.label,
    command: smokeCommand.command,
    exitCode: result.exitCode,
    elapsedMs: performance.now() - startedAt,
    stdout,
    stderr,
  };
}

function readFlag(args: readonly string[], flag: string): string | null {
  const exact = args.find((arg) => arg.startsWith(`${flag}=`));
  if (exact) return exact.slice(flag.length + 1);
  const index = args.indexOf(flag);
  if (index === -1) return null;
  return args[index + 1] ?? null;
}

function readBooleanFlag(args: readonly string[], flag: string, fallback: boolean): boolean {
  const value = readFlag(args, flag);
  if (value === null) return fallback;
  return value === "1" || value === "true" || value === "yes";
}

function timestampForFile(date: Date): string {
  return date.toISOString().replace(/[-:]/g, "").replace(/\.\d{3}Z$/, "Z");
}

function sanitizeFileStem(value: string): string {
  return value.toLowerCase().replace(/[^a-z0-9._-]+/g, "-").replace(/^-+|-+$/g, "");
}
