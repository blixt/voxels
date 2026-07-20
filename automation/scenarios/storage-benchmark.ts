import { execFile } from "node:child_process";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { promisify } from "node:util";
import { ScenarioArguments } from "../lib/arguments.ts";
import { defineScenario, type ScenarioContext } from "../lib/scenario.ts";
import { rustTool } from "../../scripts/build-wasm.ts";

const execFileAsync = promisify(execFile);
const SCHEMA_VERSION = 4;
const BINARY = "voxels-storage-benchmark";

type Profile = "clustered" | "frontier";

interface StorageBenchmarkOptions {
  readonly operations: number;
  readonly players: number;
  readonly profiles: readonly Profile[];
}

interface BenchmarkResult {
  readonly schemaVersion: number;
  readonly worldSource: "procedural-v16";
  readonly profile: Profile;
  readonly operations: number;
  readonly mutations: number;
  readonly editMapLogicalBytes: number;
  readonly editorCommitBytes: number;
  readonly publicCommitBytes: number;
  readonly operationLatencyMicros: {
    readonly median: number;
    readonly p95: number;
    readonly p99: number;
    readonly maximum: number;
  };
  readonly operationLatencyProgressQuartilesMicros: readonly {
    readonly median: number;
    readonly p95: number;
    readonly p99: number;
    readonly maximum: number;
  }[];
  readonly playerInitializationMs: number;
  readonly checkpointMs: number;
  readonly restartMs: number;
  readonly retryVerificationMs: number;
  readonly databaseBeforeCheckpoint: {
    readonly mainBytes: number;
    readonly walBytes: number;
    readonly shmBytes: number;
    readonly totalBytes: number;
  };
  readonly databaseAfterCheckpoint: {
    readonly mainBytes: number;
    readonly walBytes: number;
    readonly shmBytes: number;
    readonly totalBytes: number;
  };
  readonly tables: Readonly<
    Record<
      string,
      {
        readonly rows: number;
        readonly storageBytes: number;
        readonly payloadBytes: number;
        readonly unusedBytes: number;
      }
    >
  >;
}

function parseArguments(values: readonly string[]): StorageBenchmarkOptions {
  const arguments_ = new ScenarioArguments(values);
  const profiles = arguments_.choice("profile", ["all", "clustered", "frontier"] as const, "all");
  const options = {
    operations:
      arguments_.number("operations", {
        fallback: 2_000,
        integer: true,
        minimum: 1,
        maximum: 1_000_000,
      }) ?? 2_000,
    players:
      arguments_.number("players", {
        fallback: 100,
        integer: true,
        minimum: 1,
        maximum: 1_024,
      }) ?? 100,
    profiles: profiles === "all" ? (["clustered", "frontier"] as const) : ([profiles] as const),
  };
  if (options.players > options.operations) {
    throw new Error("--players cannot exceed --operations");
  }
  arguments_.assertEmpty();
  return options;
}

function executablePath(): string {
  return path.resolve(
    process.env.CARGO_TARGET_DIR ?? "target",
    "worldgen",
    process.platform === "win32" ? `${BINARY}.exe` : BINARY,
  );
}

function assertBenchmarkResult(value: unknown): asserts value is BenchmarkResult {
  if (
    typeof value !== "object" ||
    value === null ||
    !("schemaVersion" in value) ||
    value.schemaVersion !== SCHEMA_VERSION ||
    !("worldSource" in value) ||
    value.worldSource !== "procedural-v16"
  ) {
    throw new Error("native storage benchmark returned an incompatible result");
  }
  if ("error" in value) {
    throw new Error(`native storage benchmark failed: ${String(value.error)}`);
  }
}

function mebibytes(bytes: number): string {
  return (bytes / (1024 * 1024)).toFixed(2);
}

function markdownReport(results: readonly BenchmarkResult[]): string {
  const rows = results.map((result) => {
    const disk = result.databaseAfterCheckpoint.mainBytes;
    const bytesPerMutation = disk / result.mutations;
    const memoryBytesPerMutation = result.editMapLogicalBytes / result.mutations;
    const publicWireBytesPerMutation = result.publicCommitBytes / result.mutations;
    const first = result.operationLatencyProgressQuartilesMicros.at(0);
    const last = result.operationLatencyProgressQuartilesMicros.at(-1);
    if (first === undefined || last === undefined) {
      throw new Error("native storage benchmark omitted operation-order latency");
    }
    const progressRatio = first.median === 0 ? 0 : last.median / first.median;
    return `| ${result.profile} | ${result.operations.toLocaleString()} | ${result.mutations.toLocaleString()} | ${result.operationLatencyMicros.median} / ${result.operationLatencyMicros.p95} / ${result.operationLatencyMicros.p99} | ${first.median} → ${last.median} (${progressRatio.toFixed(2)}×) | ${result.restartMs.toFixed(1)} | ${result.checkpointMs.toFixed(1)} | ${mebibytes(result.databaseBeforeCheckpoint.walBytes)} | ${mebibytes(disk)} | ${bytesPerMutation.toFixed(1)} | ${memoryBytesPerMutation.toFixed(1)} | ${publicWireBytesPerMutation.toFixed(1)} |`;
  });
  const tableRows = results.flatMap((result) =>
    Object.entries(result.tables).map(
      ([table, stats]) =>
        `| ${result.profile} | ${table} | ${stats.rows.toLocaleString()} | ${mebibytes(stats.storageBytes)} | ${mebibytes(stats.payloadBytes)} | ${mebibytes(stats.unusedBytes)} |`,
    ),
  );
  return `# Durable world storage benchmark

The native fixture executes the production edit planner and SQLite transaction path against the
deterministic \`${results[0]?.worldSource ?? "unknown"}\` source, checkpoints the WAL, reopens the
database, verifies its revision, and retries one durable operation per player. It deliberately
excludes sockets, request queues, broadcasts, client work, rendering, and Terrain Diffusion provider
cost; those belong to the protocol, provider, and browser scenarios. The clustered corpus models
dense collaborative construction/excavation; frontier models long-lived exploration spread across
many spatial regions. Operation-order quartiles expose latency that grows as the durable edit
journal fills.

| Profile | Operations | Mutations | Commit median / p95 / p99 (µs) | First → last quartile median | Restart (ms) | Checkpoint (ms) | Peak WAL MiB | Durable MiB | Disk B / mutation | Logical RAM B / mutation | Public wire B / mutation |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
${rows.join("\n")}

| Profile | SQLite b-tree | Rows | Storage MiB | Payload MiB | Unused MiB |
| --- | --- | ---: | ---: | ---: | ---: |
${tableRows.join("\n")}
`;
}

async function runNative(
  context: ScenarioContext,
  directory: string,
  profile: Profile,
  options: StorageBenchmarkOptions,
): Promise<BenchmarkResult> {
  const requestPath = path.join(directory, `${profile}-request.json`);
  const responsePath = path.join(directory, `${profile}-response.json`);
  await writeFile(
    requestPath,
    `${JSON.stringify(
      {
        schemaVersion: SCHEMA_VERSION,
        databasePath: path.join(directory, `${profile}.sqlite3`),
        operations: options.operations,
        players: options.players,
        profile,
      },
      null,
      2,
    )}\n`,
  );
  context.log(`running ${profile} durability corpus`);
  await execFileAsync(executablePath(), [requestPath, responsePath], {
    cwd: process.cwd(),
    maxBuffer: 16 * 1024 * 1024,
  });
  const result: unknown = JSON.parse(await readFile(responsePath, "utf8"));
  assertBenchmarkResult(result);
  return result;
}

async function main(context: ScenarioContext, values: readonly string[]) {
  const options = parseArguments(values);
  await execFileAsync(
    rustTool("cargo"),
    [
      "build",
      "--profile",
      "worldgen",
      "-p",
      "voxels-world-service",
      "--features",
      "automation-fixture",
      "--bin",
      BINARY,
    ],
    { cwd: process.cwd(), maxBuffer: 16 * 1024 * 1024 },
  );
  const directory = await mkdtemp(path.join(tmpdir(), "voxels-storage-benchmark-"));
  context.defer("storage benchmark temporary databases", () =>
    rm(directory, { force: true, recursive: true }),
  );
  const results = [];
  for (const profile of options.profiles) {
    results.push(await runNative(context, directory, profile, options));
  }
  await Promise.all([
    context.artifacts.writeJson("storage benchmark", "report.json", {
      schemaVersion: SCHEMA_VERSION,
      generatedAt: new Date().toISOString(),
      options,
      results,
    }),
    context.artifacts.writeText(
      "storage benchmark",
      "report.md",
      markdownReport(results),
      "text/markdown",
    ),
  ]);
  return {
    summary: `Completed ${results.length} native durable-world storage profile${results.length === 1 ? "" : "s"}.`,
    metrics: {
      profiles: results.length,
      operations: options.operations,
      players: options.players,
    },
    details: results,
  };
}

export default defineScenario({
  id: "storage-benchmark",
  kind: "benchmark",
  summary:
    "Measures durable edit latency, disk amplification, WAL checkpointing, restart, and retries.",
  uses: { metrics: true, rust: true },
  timeoutMs: 1_800_000,
  run: main,
});
