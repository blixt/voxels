import { execFile } from "node:child_process";
import { readFile, stat } from "node:fs/promises";
import path from "node:path";
import { promisify } from "node:util";
import {
  prepareWorldFixture,
  routeWorldClient,
  startWebPreview,
  startWorldService,
} from "../lib/world.ts";
import { BrowserCapability, type BrowserViewport, reserveEphemeralPort } from "../lib/browser.ts";
import { ScenarioArguments } from "../lib/arguments.ts";
import { FRAME_SAMPLE_WIDTH, SNAPSHOT, snapshotValue } from "../lib/engine.ts";
import {
  numericSummary,
  sampleProcess,
  summarizeProcess,
  type ProcessSample,
} from "../lib/metrics.ts";
import { createShapedLink, type ShapedLink, VXWP_KIND } from "../lib/network.ts";
import { runProcess, startProcess } from "../lib/process.ts";
import { PRESENCE_PATH, WORLD_PATH, WORLD_SUBPROTOCOL } from "../lib/protocol.ts";
import { defineScenario, type ScenarioContext } from "../lib/scenario.ts";
import type { WorldFixture, WorldService, WorldSource } from "../lib/world.ts";
import { rustTool } from "../../scripts/build-wasm.ts";
import {
  worldServiceBuildCargoArgs,
  type WorldServiceCargoProfile,
} from "../../scripts/world-service-command.ts";

const execFileAsync = promisify(execFile);
const RESULT_SCHEMA_VERSION = 5;
const SAMPLE_INTERVAL_MS = 250;
const OBSERVER_SAMPLE_INTERVAL_MS = 500;
const FRAME_SAMPLE_START = SNAPSHOT.droppedSamples + 1;
const VIEWPORT = { width: 960, height: 540 };
const BOT_SPAWN_PILLAR_HEIGHT_VOXELS = 7;
const BOT_SPAWN_PROTECTION_RADIUS_VOXELS = 3;
const BROWSER_FAILURE =
  /panic|unreachable|runtimeerror|wgpu|webgpu|shader|sqlite|websocket|presence|protocol|world service/iu;
const NETWORK_PROFILE = Object.freeze({
  name: "unshaped-accounting-link",
  oneWayLatencyMs: 0,
  upstreamMegabitsPerSecond: 100_000,
  downstreamMegabitsPerSecond: 100_000,
  quantumBytes: 64 * 1_024,
});

type BotLayout = "dense" | "mixed";
type BotLoadMode = "scale" | "growth";

interface BotLoadOptions {
  counts: number[];
  durationSeconds: number;
  layout: BotLayout;
  source: WorldSource;
  mode: BotLoadMode;
  serviceProfile: WorldServiceCargoProfile;
  botProfile: WorldServiceCargoProfile;
  generationWorkers: number | undefined;
  browser: boolean;
  recordVideo: boolean;
}

interface BotClientReport {
  readonly chunkLatency: { readonly samples: number; readonly p95Ms: number };
  readonly editLatency: { readonly samples: number; readonly p95Ms: number };
  readonly maxVisiblePlayers: number;
  readonly resyncs: number;
  readonly protocolErrors: number;
  readonly errorSamples: readonly string[];
  readonly finalOutboundRateBytesPerSecond: number;
  readonly maxOutboundRateBytesPerSecond: number;
  readonly traffic: {
    readonly receivedPayloadBytes: number;
    readonly maxReceivedFrameBytes?: number;
    readonly receivedByKind?: Readonly<Record<string, { readonly payloadBytes?: number }>>;
  };
}

interface BotHarnessReport {
  readonly wallTimeMs: number;
  readonly connectionCount: number;
  readonly maxVisiblePlayers: number;
  readonly editsAccepted: number;
  readonly editsRejected: number;
  readonly editConflicts: number;
  readonly mutationsCommitted: number;
  readonly behaviors: Readonly<Record<string, number | undefined>>;
  readonly reports: readonly BotClientReport[];
}

interface DatabaseFiles {
  readonly mainBytes: number;
  readonly walBytes: number;
  readonly shmBytes: number;
  readonly totalBytes: number;
}

interface MutableDone {
  value: boolean;
}

interface Observer {
  readonly viewport: BrowserViewport;
}

interface ObserverSample {
  readonly atUnixMs: number;
  readonly remoteAvatars: number;
  readonly avatarParts: number;
  readonly avatarDrawCalls: number;
  readonly frameMs: number;
  readonly cpuMs: number;
  readonly gpuTotalMs: number;
  readonly wasmCommittedMiB: number;
  readonly coreGpuMiB: number;
  readonly residentChunks: number;
  readonly visibleChunks: number;
  readonly drawCalls: number;
  readonly pendingJobs: number;
  readonly surfaceInFlight: number;
  readonly interactiveLodsReady: boolean;
  readonly allLodsReady: boolean;
}

function parseArguments(values: readonly string[]): BotLoadOptions {
  const arguments_ = new ScenarioArguments(values);
  const countsSource = arguments_.string("counts", "4,8,16,32,64") ?? "";
  const counts = countsSource.split(",").map((value) => Number(value.trim()));
  if (
    counts.length === 0 ||
    counts.some((count) => !Number.isInteger(count) || count < 1 || count > 1_024)
  ) {
    throw new Error("--counts must contain integers in 1..=1024");
  }
  const browser = !arguments_.flag("no-browser");
  const recordVideo = arguments_.flag("video");
  if (recordVideo && !browser) throw new Error("--video requires the browser observer");
  const options: BotLoadOptions = {
    counts,
    durationSeconds:
      arguments_.number("duration", {
        fallback: 10,
        minimum: 1,
        maximum: 86_400,
      }) ?? 10,
    layout: arguments_.choice("layout", ["dense", "mixed"], "mixed"),
    source: arguments_.choice(
      "source",
      ["procedural-v16", "terrain-diffusion-30m"] as const,
      "procedural-v16",
    ),
    mode: arguments_.flag("growth") ? "growth" : "scale",
    serviceProfile: arguments_.choice("service-profile", ["worldgen", "worldgen-dev"], "worldgen"),
    botProfile: arguments_.choice("bot-profile", ["worldgen", "worldgen-dev"], "worldgen-dev"),
    generationWorkers: arguments_.number("generation-workers", {
      minimum: 3,
      maximum: 256,
    }),
    browser,
    recordVideo,
  };
  arguments_.assertEmpty();
  return options;
}

function executablePath(profile: WorldServiceCargoProfile, binary: string): string {
  return path.resolve(
    process.env.CARGO_TARGET_DIR ?? "target",
    profile,
    process.platform === "win32" ? `${binary}.exe` : binary,
  );
}

async function fileBytes(file: string): Promise<number> {
  try {
    return (await stat(file)).size;
  } catch (error) {
    if (error instanceof Error && "code" in error && error.code === "ENOENT") return 0;
    throw error;
  }
}

async function databaseFiles(databasePath: string): Promise<DatabaseFiles> {
  const [mainBytes, walBytes, shmBytes] = await Promise.all([
    fileBytes(databasePath),
    fileBytes(`${databasePath}-wal`),
    fileBytes(`${databasePath}-shm`),
  ]);
  return {
    mainBytes,
    walBytes,
    shmBytes,
    totalBytes: mainBytes + walBytes + shmBytes,
  };
}

async function databaseContents(databasePath: string): Promise<Record<string, unknown> | null> {
  if ((await fileBytes(databasePath)) === 0) return null;
  const query = `
    SELECT
      (SELECT page_count FROM pragma_page_count()) AS pageCount,
      (SELECT page_size FROM pragma_page_size()) AS pageSize,
      (SELECT freelist_count FROM pragma_freelist_count()) AS freePages,
      (SELECT revision FROM metadata WHERE singleton = 1) AS revision,
      (SELECT COUNT(*) FROM players) AS players,
      (SELECT COUNT(*) FROM player_inventory) AS inventoryRows,
      (SELECT COUNT(*) FROM voxel_edits) AS voxelEdits,
      (SELECT COUNT(*) FROM edit_operations) AS editOperations,
      (SELECT COUNT(*) FROM edit_operation_mutations) AS operationMutations,
      (SELECT COUNT(*) FROM edit_operation_chunks) AS operationChunks,
      (SELECT COUNT(*) FROM edit_operation_surfaces) AS operationSurfaces;
  `;
  const { stdout } = await execFileAsync("sqlite3", ["-json", databasePath, query]);
  const parsed = JSON.parse(stdout) as unknown;
  if (!Array.isArray(parsed)) throw new Error("sqlite database summary was not an array");
  const first = parsed[0];
  if (first === undefined) return null;
  if (typeof first !== "object" || first === null || Array.isArray(first)) {
    throw new Error("sqlite database summary row was not an object");
  }
  return first as Record<string, unknown>;
}

function messageBytes(report: BotClientReport, kind: number): number {
  return report.traffic.receivedByKind?.[String(kind)]?.payloadBytes ?? 0;
}

function summarizeTrafficBudget(report: BotHarnessReport, fixture: WorldFixture) {
  const floorBytesPerSecond = fixture.outboundBandwidthFloorBytesPerSecond;
  const ceilingBytesPerSecond = fixture.outboundBandwidthCeilingBytesPerSecond;
  const burstBytes = fixture.outboundBandwidthBurstBytes;
  const queueDelayTargetMs = fixture.outboundQueueDelayTargetMs;
  const feedbackTimeoutMs = fixture.outboundFeedbackTimeoutMs;
  const seconds = report.wallTimeMs / 1_000;
  const receivedBytes = report.reports.map((client) => client.traffic.receivedPayloadBytes);
  const bitsPerSecond = receivedBytes.map((bytes) => (bytes * 8) / seconds);
  const envelopeBytes = report.reports.map(
    (client) =>
      ceilingBytesPerSecond * seconds +
      Math.max(burstBytes, client.traffic.maxReceivedFrameBytes ?? 0) +
      1_024,
  );
  const overBudgetClients = report.reports.filter((client, index) => {
    const envelope = envelopeBytes[index];
    return envelope !== undefined && client.traffic.receivedPayloadBytes > envelope;
  }).length;
  return {
    floorBytesPerSecond,
    ceilingBytesPerSecond,
    burstBytes,
    queueDelayTargetMs,
    feedbackTimeoutMs,
    perClientReceivedBytes: numericSummary(receivedBytes),
    perClientBitsPerSecond: numericSummary(bitsPerSecond),
    selectedRateBytesPerSecond: numericSummary(
      report.reports.map((client) => client.finalOutboundRateBytesPerSecond),
    ),
    peakSelectedRateBytesPerSecond: numericSummary(
      report.reports.map((client) => client.maxOutboundRateBytesPerSecond),
    ),
    envelopeBytes: numericSummary(envelopeBytes),
    overBudgetClients,
    payloadByClass: {
      presenceBytes: report.reports.reduce(
        (sum, client) => sum + messageBytes(client, VXWP_KIND.presenceDelta),
        0,
      ),
      editBytes: report.reports.reduce(
        (sum, client) => sum + messageBytes(client, VXWP_KIND.editCommit),
        0,
      ),
      visibleWorldBytes: report.reports.reduce(
        (sum, client) =>
          sum +
          messageBytes(client, VXWP_KIND.chunkBatchResult) +
          messageBytes(client, VXWP_KIND.surfaceTileBatchResult) +
          messageBytes(client, VXWP_KIND.frameFragment),
        0,
      ),
    },
  };
}

async function collectSamples({
  servicePid,
  botPid,
  databasePath,
  done,
}: {
  readonly servicePid: number;
  readonly botPid: number;
  readonly databasePath: string;
  readonly done: MutableDone;
}) {
  const service: ProcessSample[] = [];
  const bots: ProcessSample[] = [];
  const database: ({ readonly atUnixMs: number } & DatabaseFiles)[] = [];
  while (!done.value) {
    const [serviceSample, botSample, files] = await Promise.all([
      sampleProcess(servicePid),
      sampleProcess(botPid),
      databaseFiles(databasePath),
    ]);
    if (serviceSample) service.push(serviceSample);
    if (botSample) bots.push(botSample);
    database.push({ atUnixMs: Date.now(), ...files });
    await new Promise((resolve) => setTimeout(resolve, SAMPLE_INTERVAL_MS));
  }
  return { service, bots, database };
}

async function collectObserverSamples(
  observer: Observer | null,
  expectedBots: number,
  started: number,
  done: MutableDone,
) {
  if (observer === null) return null;
  const samples: ObserverSample[] = [];
  const frameMilliseconds: number[] = [];
  let rosterReadyMs: number | null = null;
  let interactiveWorldReadyMs: number | null = null;
  let fullWorldReadyMs: number | null = null;
  while (!done.value) {
    const values = await observer.viewport.engine.snapshot();
    const remoteAvatars = snapshotValue(values, "remoteAvatars");
    const elapsedMs = performance.now() - started;
    if (rosterReadyMs === null && remoteAvatars === expectedBots) {
      rosterReadyMs = elapsedMs;
    }
    if (interactiveWorldReadyMs === null && snapshotValue(values, "interactiveLodsReady") === 1) {
      interactiveWorldReadyMs = elapsedMs;
    }
    if (
      fullWorldReadyMs === null &&
      snapshotValue(values, "allLodsReady") === 1 &&
      snapshotValue(values, "surfaceInFlight") === 0 &&
      snapshotValue(values, "pendingJobs") === 0
    ) {
      fullWorldReadyMs = elapsedMs;
    }
    for (let index = 0; index < snapshotValue(values, "sampleCount"); index += 1) {
      const frame = values[FRAME_SAMPLE_START + index * FRAME_SAMPLE_WIDTH];
      if (frame !== undefined) frameMilliseconds.push(frame);
    }
    samples.push({
      atUnixMs: Date.now(),
      remoteAvatars,
      avatarParts: snapshotValue(values, "avatarParts"),
      avatarDrawCalls: snapshotValue(values, "avatarDrawCalls"),
      frameMs: snapshotValue(values, "frameMs"),
      cpuMs: snapshotValue(values, "cpuMs"),
      gpuTotalMs: snapshotValue(values, "gpuTotalMs"),
      wasmCommittedMiB: snapshotValue(values, "wasmCommittedMiB"),
      coreGpuMiB: snapshotValue(values, "coreGpuMiB"),
      residentChunks: snapshotValue(values, "residentChunks"),
      visibleChunks: snapshotValue(values, "visibleChunks"),
      drawCalls: snapshotValue(values, "drawCalls"),
      pendingJobs: snapshotValue(values, "pendingJobs"),
      surfaceInFlight: snapshotValue(values, "surfaceInFlight"),
      interactiveLodsReady: snapshotValue(values, "interactiveLodsReady") === 1,
      allLodsReady: snapshotValue(values, "allLodsReady") === 1,
    });
    await observer.viewport.page.waitForTimeout(OBSERVER_SAMPLE_INTERVAL_MS);
  }
  const final = samples.at(-1);
  return {
    rosterReadyMs,
    interactiveWorldReadyMs,
    fullWorldReadyMs,
    maxRemoteAvatars: Math.max(0, ...samples.map((sample) => sample.remoteAvatars)),
    frameMs: numericSummary(frameMilliseconds),
    cpuMs: numericSummary(samples.map((sample) => sample.cpuMs)),
    gpuTotalMs: numericSummary(
      samples.map((sample) => sample.gpuTotalMs).filter((value) => value > 0),
    ),
    wasmCommittedMiB: numericSummary(samples.map((sample) => sample.wasmCommittedMiB)),
    coreGpuMiB: numericSummary(samples.map((sample) => sample.coreGpuMiB)),
    finalWorld:
      final === undefined
        ? null
        : {
            residentChunks: final.residentChunks,
            visibleChunks: final.visibleChunks,
            pendingJobs: final.pendingJobs,
            surfaceInFlight: final.surfaceInFlight,
            interactiveLodsReady: final.interactiveLodsReady,
            allLodsReady: final.allLodsReady,
          },
    samples,
    errors: observer.viewport.failures.map((failure) => `${failure.source}: ${failure.message}`),
  };
}

function summarizeDatabase(
  samples: readonly ({ readonly atUnixMs: number } & DatabaseFiles)[],
  before: DatabaseFiles,
  after: DatabaseFiles,
  contents: Readonly<Record<string, unknown>> | null,
) {
  return {
    before,
    after,
    deltaBytes: after.totalBytes - before.totalBytes,
    peakTotalBytes: Math.max(
      before.totalBytes,
      after.totalBytes,
      ...samples.map((value) => value.totalBytes),
    ),
    samples,
    contents,
  };
}

async function runPopulation({
  context,
  count,
  options,
  fixture,
  service,
  link,
  botBinary,
  runIndex,
  observer,
}: {
  readonly context: ScenarioContext;
  readonly count: number;
  readonly options: BotLoadOptions;
  readonly fixture: WorldFixture;
  readonly service: WorldService;
  readonly link: ShapedLink;
  readonly botBinary: string;
  readonly runIndex: number;
  readonly observer: Observer | null;
}) {
  const reportPath = path.join(fixture.directory, `bots-${count}-${runIndex}.json`);
  const before = await databaseFiles(fixture.databasePath);
  if (observer !== null) {
    await observer.viewport.engine.snapshot();
  }
  link.reset();
  const logs: string[] = [];
  const botProcess = startProcess(
    context,
    botBinary,
    [
      `--world-url=ws://127.0.0.1:${link.port}${WORLD_PATH}`,
      `--presence-url=ws://127.0.0.1:${link.port}${PRESENCE_PATH}`,
      `--origin=http://127.0.0.1:${fixture.originPort}`,
      `--subprotocol=${WORLD_SUBPROTOCOL}`,
      `--auth-token=${fixture.authToken}`,
      `--bots=${count}`,
      `--duration-seconds=${options.durationSeconds}`,
      `--seed=${0x5eedcafe}`,
      `--layout=${options.layout}`,
      `--report=${reportPath}`,
    ],
    {
      label: `bots (${count})`,
      cwd: process.cwd(),
      env: process.env,
      stdio: ["ignore", "pipe", "pipe"],
    },
  );
  const { child: bot } = botProcess;
  for (const stream of [bot.stdout, bot.stderr]) {
    stream?.on("data", (bytes) => {
      logs.push(bytes.toString());
      if (logs.length > 200) logs.shift();
    });
  }
  const done = { value: false };
  if (service.child.pid === undefined || bot.pid === undefined) {
    throw new Error("bot load processes did not expose process IDs");
  }
  const samplesPromise = collectSamples({
    servicePid: service.child.pid,
    botPid: bot.pid,
    databasePath: fixture.databasePath,
    done,
  });
  const started = performance.now();
  const observerPromise = collectObserverSamples(observer, count, started, done);
  try {
    try {
      await botProcess.completed;
    } catch (error) {
      const serviceLogs = service.logs?.join("") ?? "";
      throw new Error(
        `${error instanceof Error ? error.message : String(error)}\n${logs.join("")}${
          serviceLogs.length > 0 ? `\nworld service:\n${serviceLogs}` : ""
        }`,
        { cause: error },
      );
    }
  } finally {
    done.value = true;
  }
  const wallTimeMs = performance.now() - started;
  const samples = await samplesPromise;
  const observerReport = await observerPromise;
  const report = JSON.parse(await readFile(reportPath, "utf8")) as BotHarnessReport;
  const after = await databaseFiles(fixture.databasePath);
  const contents = await databaseContents(fixture.databasePath);
  return {
    count,
    generationWorkers: fixture.generationWorkers,
    wallTimeMs,
    botReport: report,
    trafficBudget: summarizeTrafficBudget(report, fixture),
    process: {
      service: summarizeProcess(samples.service),
      bots: summarizeProcess(samples.bots),
    },
    network: link.snapshot(),
    database: summarizeDatabase(samples.database, before, after, contents),
    observer: observerReport,
  };
}

type BotLoadStageBase = Awaited<ReturnType<typeof runPopulation>>;
type BotLoadStage = BotLoadStageBase & { readonly violations: readonly string[] };

interface BotLoadResult {
  readonly schemaVersion: number;
  readonly generatedAt: string;
  readonly host: {
    readonly platform: string;
    readonly architecture: string;
    readonly node: string;
  };
  readonly options: BotLoadOptions;
  readonly stages: BotLoadStage[];
  readonly violations: string[];
}

function markdownReport(result: BotLoadResult): string {
  const lines = [
    "# Voxels bot population benchmark",
    "",
    `Mode: **${result.options.mode}** · layout: **${result.options.layout}** · duration: **${result.options.durationSeconds}s per population** · source: **${result.options.source}** · generation workers: **${result.options.generationWorkers ?? "config default"}**`,
    "",
    "| Bots | Workers | Server CPU p95 | Server RSS peak | Bot CPU p95 | TCP down/up | VXWP down/up | DB growth | Edits accepted/conflicts | Mutations | Chunk p95 | Edit p95 | Visible | Roster ready | Observer LOD | Observer frame p95 |",
    "| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
  ];
  for (const stage of result.stages) {
    const bot = stage.botReport;
    const chunkP95 = numericSummary(
      bot.reports.flatMap((report) =>
        report.chunkLatency.samples > 0 ? [report.chunkLatency.p95Ms] : [],
      ),
    ).p95;
    const editP95 = numericSummary(
      bot.reports.flatMap((report) =>
        report.editLatency.samples > 0 ? [report.editLatency.p95Ms] : [],
      ),
    ).p95;
    const observerFrame =
      stage.observer === null ? "off" : `${stage.observer.frameMs.p95.toFixed(1)} ms`;
    const rosterReady =
      stage.observer === null
        ? "off"
        : stage.observer.rosterReadyMs === null
          ? "missing"
          : `${stage.observer.rosterReadyMs.toFixed(0)} ms`;
    const observerLod =
      stage.observer === null
        ? "off"
        : stage.observer.fullWorldReadyMs !== null
          ? `full ${stage.observer.fullWorldReadyMs.toFixed(0)} ms`
          : stage.observer.interactiveWorldReadyMs !== null
            ? `interactive ${stage.observer.interactiveWorldReadyMs.toFixed(0)} ms`
            : `partial ${stage.observer.finalWorld?.residentChunks ?? 0} resident`;
    lines.push(
      `| ${stage.count} | ${stage.generationWorkers} | ${stage.process.service.cpuPercent.p95.toFixed(1)}% | ${stage.process.service.rssMiB.max.toFixed(1)} MiB | ${stage.process.bots.cpuPercent.p95.toFixed(1)}% | ${(stage.network.downstream.streamBytes / 1_048_576).toFixed(2)} / ${(stage.network.upstream.streamBytes / 1_048_576).toFixed(2)} MiB | ${(stage.network.downstream.vxwpPayloadBytes / 1_048_576).toFixed(2)} / ${(stage.network.upstream.vxwpPayloadBytes / 1_048_576).toFixed(2)} MiB | ${(stage.database.deltaBytes / 1_024).toFixed(1)} KiB | ${bot.editsAccepted}/${bot.editConflicts} | ${bot.mutationsCommitted} | ${chunkP95.toFixed(1)} ms | ${editP95.toFixed(1)} ms | ${bot.maxVisiblePlayers} | ${rosterReady} | ${observerLod} | ${observerFrame} |`,
    );
  }
  lines.push(
    "",
    "Generated terrain is deterministic and RAM-cached, not persisted. Explorer traffic therefore increases CPU and bandwidth; durable disk growth comes from player resumes, inventories, idempotent operation history, and sparse voxel edits.",
    "",
    "CPU percentages use `ps` semantics (100% is one fully occupied logical core). Stream bytes include HTTP upgrades and WebSocket framing; VXWP bytes are binary protocol payloads.",
    "",
    "## Per-client traffic budgets",
    "",
    "| Bots | Adaptive floor/ceiling | Selected p95/max | Queue target | Burst | Client down p95/max | Presence | Edits | Visible world | Ceiling violations |",
    "| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
  );
  for (const stage of result.stages) {
    const budget = stage.trafficBudget;
    lines.push(
      `| ${stage.count} | ${((budget.floorBytesPerSecond * 8) / 1_000_000).toFixed(3)} / ${((budget.ceilingBytesPerSecond * 8) / 1_000_000).toFixed(3)} Mbit/s | ${((budget.selectedRateBytesPerSecond.p95 * 8) / 1_000_000).toFixed(3)} / ${((budget.peakSelectedRateBytesPerSecond.max * 8) / 1_000_000).toFixed(3)} Mbit/s | ${budget.queueDelayTargetMs} ms | ${(budget.burstBytes / 1_024).toFixed(0)} KiB | ${(budget.perClientBitsPerSecond.p95 / 1_000_000).toFixed(3)} / ${(budget.perClientBitsPerSecond.max / 1_000_000).toFixed(3)} Mbit/s | ${(budget.payloadByClass.presenceBytes / 1_048_576).toFixed(2)} MiB | ${(budget.payloadByClass.editBytes / 1_048_576).toFixed(2)} MiB | ${(budget.payloadByClass.visibleWorldBytes / 1_048_576).toFixed(2)} MiB | ${budget.overBudgetClients} |`,
    );
  }
  if (result.violations.length > 0) {
    lines.push("", "## Violations", "", ...result.violations.map((violation) => `- ${violation}`));
  }
  return `${lines.join("\n")}\n`;
}

function stageViolations(stage: BotLoadStageBase, browserEnabled: boolean): string[] {
  const violations: string[] = [];
  const expectedVisible = browserEnabled ? stage.count : stage.count - 1;
  if (stage.botReport.connectionCount !== stage.count) {
    violations.push(
      `${stage.count} bots: only ${stage.botReport.connectionCount} protocol clients connected`,
    );
  }
  if (stage.botReport.maxVisiblePlayers !== expectedVisible) {
    violations.push(
      `${stage.count} bots: native clients saw ${stage.botReport.maxVisiblePlayers}, expected ${expectedVisible}`,
    );
  }
  const incompleteRosters = stage.botReport.reports.filter(
    (report) => report.maxVisiblePlayers !== expectedVisible,
  ).length;
  if (incompleteRosters > 0) {
    violations.push(
      `${stage.count} bots: ${incompleteRosters} clients did not receive the complete ${expectedVisible}-peer roster`,
    );
  }
  const rejected = stage.botReport.editsRejected;
  const expectedConflicts = stage.botReport.editConflicts ?? 0;
  const editorCount = ["digger", "builder", "follower"].reduce(
    (sum, behavior) => sum + (stage.botReport.behaviors[behavior] ?? 0),
    0,
  );
  const resyncs = stage.botReport.reports.reduce((sum, report) => sum + report.resyncs, 0);
  const protocolErrors = stage.botReport.reports.reduce(
    (sum, report) => sum + report.protocolErrors,
    0,
  );
  const nativeErrors = [
    ...new Set(stage.botReport.reports.flatMap((report) => report.errorSamples)),
  ];
  if (rejected > expectedConflicts) {
    violations.push(
      `${stage.count} bots: ${rejected - expectedConflicts} unexpected edits were rejected`,
    );
  }
  if (editorCount > 0 && stage.botReport.editsAccepted === 0) {
    violations.push(`${stage.count} bots: editor clients completed no accepted edits`);
  }
  if (editorCount > 0 && stage.botReport.mutationsCommitted === 0) {
    violations.push(`${stage.count} bots: accepted edits produced no voxel mutations`);
  }
  if (resyncs > 0) violations.push(`${stage.count} bots: ${resyncs} clients required resync`);
  if (protocolErrors > 0) {
    violations.push(`${stage.count} bots: ${protocolErrors} protocol errors were received`);
  }
  if (nativeErrors.length > 0) {
    violations.push(
      `${stage.count} bots: native client errors: ${nativeErrors.slice(0, 3).join("; ")}`,
    );
  }
  if (stage.trafficBudget.overBudgetClients > 0) {
    violations.push(
      `${stage.count} bots: ${stage.trafficBudget.overBudgetClients} clients exceeded the configured payload envelope`,
    );
  }
  if (stage.observer !== null) {
    if (stage.observer.rosterReadyMs === null || stage.observer.maxRemoteAvatars !== stage.count) {
      violations.push(
        `${stage.count} bots: browser observed at most ${stage.observer.maxRemoteAvatars} avatars`,
      );
    }
    for (const error of stage.observer.errors) {
      violations.push(`${stage.count} bots: browser ${error}`);
    }
  }
  return violations;
}

async function startObserver(
  browser: BrowserCapability,
  previewPort: number,
  fixture: WorldFixture,
  count: number,
  recordVideo: boolean,
): Promise<Observer> {
  const viewport = await browser.open({
    url: `http://127.0.0.1:${previewPort}`,
    label: `bot-observer-${count}`,
    viewport: VIEWPORT,
    recordVideo,
    videoFilename: `bot-observer-${count}.webm`,
    ...routeWorldClient(fixture, 0),
  });
  await viewport.engine.waitForSnapshot(
    (snapshot) =>
      snapshotValue(snapshot, "quads") > 0 && snapshotValue(snapshot, "residentChunks") > 0,
    {
      timeoutMs: 30_000,
      intervalMs: 100,
      description: `browser observer did not load terrain; errors: ${JSON.stringify(viewport.failures)}`,
    },
  );
  return { viewport };
}

async function main(context: ScenarioContext, arguments_: readonly string[]) {
  const options = parseArguments(arguments_);
  await runProcess(
    context,
    rustTool("cargo"),
    worldServiceBuildCargoArgs({ metal: false, profile: options.serviceProfile }),
    {
      label: "bot world-service build",
      cwd: process.cwd(),
      env: process.env,
      stdio: "inherit",
    },
  );
  await runProcess(
    context,
    rustTool("cargo"),
    ["build", "--profile", options.botProfile, "-p", "voxels-bots"],
    {
      label: "bot harness build",
      cwd: process.cwd(),
      env: process.env,
      stdio: "inherit",
    },
  );
  const botBinary = executablePath(options.botProfile, "voxels-bots");
  const result: BotLoadResult = {
    schemaVersion: RESULT_SCHEMA_VERSION,
    generatedAt: new Date().toISOString(),
    host: {
      platform: process.platform,
      architecture: process.arch,
      node: process.version,
    },
    options,
    stages: [],
    violations: [],
  };
  let growthFixture: WorldFixture | undefined;
  let growthService: WorldService | undefined;
  let growthLink: ShapedLink | undefined;
  let browser: BrowserCapability | undefined;
  let previewPort: number | undefined;
  try {
    if (options.browser) {
      previewPort = await reserveEphemeralPort();
      await startWebPreview(context, { port: previewPort, buildProfile: "release" });
      browser = await BrowserCapability.start(context, { warningPattern: BROWSER_FAILURE });
    }
    for (const [runIndex, count] of options.counts.entries()) {
      const proxyPort = growthLink?.port ?? (await reserveEphemeralPort());
      let fixture = growthFixture;
      if (fixture === undefined) {
        const createdFixture = await prepareWorldFixture({
          originPort: previewPort ?? (await reserveEphemeralPort()),
          clientPorts: [proxyPort],
          prefix: `voxels-bots-${count}-`,
          source: options.source,
          // Keep the spawn safe without lifting nearby terrain beyond ordinary interaction reach.
          spawnPillarHeightVoxels: BOT_SPAWN_PILLAR_HEIGHT_VOXELS,
          spawnProtectionRadiusVoxels: BOT_SPAWN_PROTECTION_RADIUS_VOXELS,
          generationWorkers: options.generationWorkers,
        });
        context.defer(`bot fixture ${count}`, () => createdFixture.cleanup());
        fixture = createdFixture;
      }
      const service: WorldService =
        growthService ??
        (await startWorldService(context, fixture, {
          build: false,
          metal: false,
          profile: options.serviceProfile,
        }));
      let proxy: ShapedLink | undefined = growthLink;
      if (proxy === undefined) {
        const createdProxy = await createShapedLink({
          listenPort: proxyPort,
          targetPort: fixture.backendPort,
          profile: NETWORK_PROFILE,
        });
        context.defer(`bot shaped link ${count}`, () => createdProxy.close());
        proxy = createdProxy;
      }
      if (options.mode === "growth" && growthFixture === undefined) {
        growthFixture = fixture;
        growthService = service;
        growthLink = proxy;
      }
      let observer: Observer | null = null;
      try {
        observer =
          browser === undefined || previewPort === undefined
            ? null
            : await startObserver(browser, previewPort, fixture, count, options.recordVideo);
        const stage = await runPopulation({
          context,
          count,
          options,
          fixture,
          service,
          link: proxy,
          botBinary,
          runIndex,
          observer,
        });
        const violations = stageViolations(stage, options.browser);
        result.violations.push(...violations);
        result.stages.push({ ...stage, violations });
        if (observer !== null && runIndex === options.counts.length - 1) {
          await observer.viewport.screenshot("bot observer", {
            filename: "observer.png",
          });
        }
      } finally {
        if (observer !== null) await observer.viewport.close();
        if (options.mode === "scale") {
          await proxy.close();
          await service.close();
          await fixture.cleanup();
        }
      }
    }
  } finally {
    if (growthLink) await growthLink.close();
    if (growthService) await growthService.close();
    if (growthFixture) await growthFixture.cleanup();
  }
  const markdown = markdownReport(result);
  await Promise.all([
    context.artifacts.writeJson("bot load report", "report.json", result),
    context.artifacts.writeText("bot load report", "report.md", markdown, "text/markdown"),
  ]);
  if (result.violations.length > 0) {
    throw new Error(`bot load harness found ${result.violations.length} violation(s)`);
  }
  return {
    summary: `Completed ${result.stages.length} bot population stages.`,
    metrics: {
      populations: result.stages.length,
      violations: result.violations.length,
    },
    details: result,
  };
}

export default defineScenario({
  id: "bot-load",
  kind: "bot-load",
  summary: "Runs native bot populations with optional browser observation and resource accounting.",
  uses: {
    world: true,
    browser: true,
    viewport: "browser",
    screenshots: true,
    video: true,
    bots: true,
    network: true,
    metrics: true,
    rust: true,
  },
  timeoutMs: 1_800_000,
  run: main,
});
