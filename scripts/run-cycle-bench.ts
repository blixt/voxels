export {};

interface CliOptions {
  label: string | null;
  outputDir: string;
}

interface StepDefinition {
  id: string;
  description: string;
  command: string[];
  kind: "check" | "build" | "profile-scenes" | "profile-stream" | "profile-game-stream";
}

interface StepReport {
  id: string;
  description: string;
  command: string[];
  kind: StepDefinition["kind"];
  exitCode: number;
  elapsedMs: number;
  stdout: string;
  stderr: string;
  jsonLines: unknown[];
}

interface CycleReport {
  generatedAt: string;
  label: string | null;
  commit: string | null;
  bunVersion: string;
  cwd: string;
  outputPath: string;
  steps: StepReport[];
  summary: Record<string, unknown>;
}

const options = parseCli(Bun.argv);
const outputDir = options.outputDir;
const outputName = `${timestampForFile(new Date())}${options.label ? `-${sanitizeFileStem(options.label)}` : ""}.json`;
const outputPath = `${outputDir}/${outputName}`;

await Bun.$`mkdir -p ${outputDir}`.quiet();

const steps: StepDefinition[] = [
  {
    id: "check",
    description: "Type checks and unit tests",
    command: ["bun", "run", "check"],
    kind: "check",
  },
  {
    id: "build",
    description: "Production build",
    command: ["bun", "run", "build"],
    kind: "build",
  },
  {
    id: "profile-scenes",
    description: "Local scene build/mesh profile",
    command: [
      "bun",
      "run",
      "profile",
      "--",
      "--iterations=2",
      "--warmup=1",
      "terrain256",
      "stressMicroCubes256",
      "stressScreens256",
    ],
    kind: "profile-scenes",
  },
  {
    id: "profile-stream",
    description: "Procedural residency profile",
    command: [
      "bun",
      "run",
      "profile:stream",
      "--",
      "--iterations=2",
      "--warmup=1",
      "--seed=1337",
      "--near-radius=2",
      "--far-radius=3",
    ],
    kind: "profile-stream",
  },
  {
    id: "profile-game-stream",
    description: "Incremental game streaming profile",
    command: [
      "bun",
      "run",
      "profile:game-stream",
      "--",
      "--iterations=2",
      "--warmup=1",
      "--seed=1337",
      "--radius=5",
      "--generate-budget=6",
      "--mesh-budget=4",
      "--far-band-budget=1",
      "--chunk-delta=2",
    ],
    kind: "profile-game-stream",
  },
];

const stepReports: StepReport[] = [];
let hadFailure = false;

for (const step of steps) {
  if (hadFailure) {
    break;
  }
  const report = runStep(step);
  stepReports.push(report);
  if (report.exitCode !== 0) {
    hadFailure = true;
  }
}

const report: CycleReport = {
  generatedAt: new Date().toISOString(),
  label: options.label,
  commit: await readGitShortHead(),
  bunVersion: Bun.version,
  cwd: process.cwd(),
  outputPath,
  steps: stepReports,
  summary: buildSummary(stepReports),
};

await Bun.write(outputPath, `${JSON.stringify(report, null, 2)}\n`);
printSummary(report);

if (hadFailure) {
  process.exit(1);
}

function parseCli(argv: readonly string[]): CliOptions {
  const args = argv.slice(2);
  return {
    label: readFlag(args, "--label"),
    outputDir: readFlag(args, "--output-dir") ?? "artifacts/cycle-bench",
  };
}

function readFlag(args: readonly string[], flag: string): string | null {
  const exact = args.find((arg) => arg.startsWith(`${flag}=`));
  if (exact) {
    return exact.slice(flag.length + 1);
  }
  const index = args.indexOf(flag);
  if (index === -1) {
    return null;
  }
  return args[index + 1] ?? null;
}

function runStep(step: StepDefinition): StepReport {
  const startedAt = performance.now();
  const subprocess = Bun.spawnSync(step.command, {
    cwd: process.cwd(),
    stdout: "pipe",
    stderr: "pipe",
    env: process.env,
  });
  const elapsedMs = performance.now() - startedAt;
  const stdout = new TextDecoder().decode(subprocess.stdout);
  const stderr = new TextDecoder().decode(subprocess.stderr);
  const jsonLines = extractJsonLines(stdout);

  return {
    id: step.id,
    description: step.description,
    command: step.command,
    kind: step.kind,
    exitCode: subprocess.exitCode,
    elapsedMs: Number(elapsedMs.toFixed(2)),
    stdout,
    stderr,
    jsonLines,
  };
}

function extractJsonLines(stdout: string): unknown[] {
  const parsed: unknown[] = [];
  for (const line of stdout.split(/\r?\n/)) {
    const trimmed = line.trim();
    if (!trimmed.startsWith("{") || !trimmed.endsWith("}")) {
      continue;
    }
    try {
      parsed.push(JSON.parse(trimmed));
    } catch {
      // Keep raw stdout in the report; malformed lines are ignored here.
    }
  }
  return parsed;
}

function buildSummary(steps: readonly StepReport[]): Record<string, unknown> {
  const summary: Record<string, unknown> = {
    failedStepIds: steps.filter((step) => step.exitCode !== 0).map((step) => step.id),
    totalElapsedMs: Number(steps.reduce((total, step) => total + step.elapsedMs, 0).toFixed(2)),
  };

  const sceneProfile = steps.find((step) => step.kind === "profile-scenes");
  if (sceneProfile) {
    summary.sceneProfile = summarizeSceneProfile(sceneProfile.jsonLines);
  }

  const streamProfile = steps.find((step) => step.kind === "profile-stream");
  if (streamProfile) {
    summary.streamProfile = summarizeStreamProfile(streamProfile.jsonLines);
  }

  const gameStreamProfile = steps.find((step) => step.kind === "profile-game-stream");
  if (gameStreamProfile) {
    summary.gameStreamProfile = summarizeGameStreamProfile(gameStreamProfile.jsonLines);
  }

  return summary;
}

function summarizeSceneProfile(entries: readonly unknown[]): Record<string, unknown> {
  const byScene = new Map<string, Record<string, unknown>>();
  for (const entry of entries) {
    if (!isRecord(entry) || typeof entry.scene !== "string") {
      continue;
    }
    byScene.set(entry.scene, {
      buildAvgMs: readNestedNumber(entry, ["build", "avgMs"]),
      meshAvgMs: readNestedNumber(entry, ["mesh", "avgMs"]),
      triangles: typeof entry.triangles === "number" ? entry.triangles : null,
      chunks: typeof entry.chunks === "number" ? entry.chunks : null,
      solidVoxels: typeof entry.solidVoxels === "number" ? entry.solidVoxels : null,
    });
  }
  return Object.fromEntries(byScene);
}

function summarizeStreamProfile(entries: readonly unknown[]): Record<string, unknown> {
  const byScenario = new Map<string, Record<string, unknown>>();
  for (const entry of entries) {
    if (!isRecord(entry) || typeof entry.scenario !== "string") {
      continue;
    }
    byScenario.set(entry.scenario, {
      streamAvgMs: readNestedNumber(entry, ["stream", "avgMs"]),
      meshAvgMs: readNestedNumber(entry, ["mesh", "avgMs"]),
      generatedChunksAvg: readNestedNumber(entry, ["generatedChunks", "avgMs"]),
      meshRemeshChunksAvg: readNestedNumber(entry, ["meshRemeshChunks", "avgMs"]),
      chunkGenerationAvgMs: readNestedNumber(entry, ["phases", "chunkGeneration", "avgMs"]),
      residentChunks: typeof entry.residentChunks === "number" ? entry.residentChunks : null,
    });
  }
  return Object.fromEntries(byScenario);
}

function summarizeGameStreamProfile(entries: readonly unknown[]): Record<string, unknown> {
  const byScenario = new Map<string, Record<string, unknown>>();
  for (const entry of entries) {
    if (!isRecord(entry) || typeof entry.scenario !== "string") {
      continue;
    }
    byScenario.set(entry.scenario, {
      totalStreamAvgMs: readNestedNumber(entry, ["totalStreamMs", "avg"]),
      totalMeshAvgMs: readNestedNumber(entry, ["totalMeshMs", "avg"]),
      totalFarFieldAvgMs: readNestedNumber(entry, ["totalFarFieldMs", "avg"]),
      maxFrameWorkAvgMs: readNestedNumber(entry, ["maxFrameWorkMs", "avg"]),
      maxFarFieldBandLabel: typeof entry.maxFarFieldBandLabel === "string" ? entry.maxFarFieldBandLabel : null,
      generatedChunksAvg: readNestedNumber(entry, ["totalGeneratedChunks", "avg"]),
      remeshChunksAvg: readNestedNumber(entry, ["totalRemeshChunks", "avg"]),
      emptyChunksSkippedAvg: readNestedNumber(entry, ["totalEmptyChunksSkipped", "avg"]),
    });
  }
  return Object.fromEntries(byScenario);
}

function readNestedNumber(value: unknown, path: readonly string[]): number | null {
  let current: unknown = value;
  for (const segment of path) {
    if (!isRecord(current) || !(segment in current)) {
      return null;
    }
    current = current[segment];
  }
  return typeof current === "number" ? current : null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function printSummary(report: CycleReport): void {
  const lines = [
    `cycle-bench report: ${report.outputPath}`,
    `commit: ${report.commit ?? "unknown"}`,
    `total elapsed: ${report.summary.totalElapsedMs} ms`,
  ];
  for (const step of report.steps) {
    lines.push(
      `${step.id}: exit=${step.exitCode} elapsed=${step.elapsedMs.toFixed(2)} ms`,
    );
  }
  console.log(lines.join("\n"));
  console.log(JSON.stringify(report.summary, null, 2));
}

async function readGitShortHead(): Promise<string | null> {
  const result = Bun.spawnSync(["git", "rev-parse", "--short", "HEAD"], {
    cwd: process.cwd(),
    stdout: "pipe",
    stderr: "pipe",
  });
  if (result.exitCode !== 0) {
    return null;
  }
  return new TextDecoder().decode(result.stdout).trim() || null;
}

function sanitizeFileStem(value: string): string {
  return value.replace(/[^A-Za-z0-9._-]+/g, "-").replace(/-+/g, "-").replace(/^-|-$/g, "");
}

function timestampForFile(date: Date): string {
  return date.toISOString().replace(/[:-]/g, "").replace(/\.\d{3}Z$/, "Z");
}
