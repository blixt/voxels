import { execFile, execFileSync, spawn } from "node:child_process";
import { mkdir, readFile, stat } from "node:fs/promises";
import path from "node:path";
import { promisify } from "node:util";
import { chromium } from "playwright";
import { build, preview } from "vite-plus";
import { prepareBrowserWorldFixture, startBrowserWorldService } from "./browser-world-fixture.mjs";
import {
  assertSnapshotSchema,
  chromeWebGpuLaunchOptions,
  FRAME_SAMPLE_START,
  FRAME_SAMPLE_WIDTH,
  isBrowserConsoleFailure,
  reserveEphemeralPort,
  SNAPSHOT,
} from "./browser-harness.mjs";
import { rustTool } from "./build-wasm.ts";
import {
  numericSummary,
  sampleProcess,
  summarizeProcess,
  writeHarnessReport,
} from "./harness-metrics.mjs";
import { createShapedLink } from "./network-benchmark-link.mjs";
import { PRESENCE_PATH, WORLD_PATH, WORLD_SUBPROTOCOL } from "./vxwp-contract.mjs";
import { worldServiceBuildCargoArgs } from "./world-service-command.ts";

const execFileAsync = promisify(execFile);
const RESULT_SCHEMA_VERSION = 4;
const OUTPUT_DIRECTORY = path.resolve("target/harness/bots");
const SAMPLE_INTERVAL_MS = 250;
const OBSERVER_SAMPLE_INTERVAL_MS = 500;
const VIEWPORT = { width: 960, height: 540 };
const BROWSER_FAILURE =
  /panic|unreachable|runtimeerror|wgpu|webgpu|shader|sqlite|websocket|presence|protocol|world service/iu;
const NETWORK_PROFILE = Object.freeze({
  name: "unshaped-accounting-link",
  oneWayLatencyMs: 0,
  upstreamMegabitsPerSecond: 100_000,
  downstreamMegabitsPerSecond: 100_000,
  quantumBytes: 64 * 1_024,
});

function parseArguments(values) {
  const options = {
    counts: [4, 8, 16, 32, 64],
    durationSeconds: 10,
    layout: "mixed",
    source: "procedural-v16",
    mode: "scale",
    serviceProfile: "worldgen",
    botProfile: "worldgen-dev",
    browser: true,
  };
  for (const argument of values) {
    if (argument === "--") continue;
    if (argument === "--growth") {
      options.mode = "growth";
      continue;
    }
    if (argument === "--no-browser") {
      options.browser = false;
      continue;
    }
    if (argument === "--browser") {
      options.browser = true;
      continue;
    }
    const [name, value] = argument.split("=", 2);
    if (value === undefined) throw new Error(`expected --name=value, received ${argument}`);
    if (name === "--counts" || name === "--populations") {
      options.counts = value.split(",").map(Number);
    } else if (name === "--duration" || name === "--duration-seconds") {
      options.durationSeconds = Number(value);
    } else if (name === "--layout") {
      options.layout = value;
    } else if (name === "--source") {
      options.source = value;
    } else if (name === "--service-profile") {
      options.serviceProfile = value;
    } else if (name === "--bot-profile") {
      options.botProfile = value;
    } else {
      throw new Error(`unknown bot load option ${name}`);
    }
  }
  if (
    options.counts.length === 0 ||
    options.counts.some((count) => !Number.isInteger(count) || count < 1 || count > 1_024)
  ) {
    throw new Error("--counts must contain integers in 1..=1024");
  }
  if (
    !Number.isFinite(options.durationSeconds) ||
    options.durationSeconds < 1 ||
    options.durationSeconds > 86_400
  ) {
    throw new Error("--duration must be in 1..=86400 seconds");
  }
  if (!["dense", "mixed"].includes(options.layout)) {
    throw new Error("--layout must be dense or mixed");
  }
  if (!["scale", "growth"].includes(options.mode)) throw new Error("invalid mode");
  if (!["worldgen", "worldgen-dev"].includes(options.serviceProfile)) {
    throw new Error("--service-profile must be worldgen or worldgen-dev");
  }
  if (!["worldgen", "worldgen-dev"].includes(options.botProfile)) {
    throw new Error("--bot-profile must be worldgen or worldgen-dev");
  }
  return options;
}

function executablePath(profile, binary) {
  return path.resolve(
    process.env.CARGO_TARGET_DIR ?? "target",
    profile,
    process.platform === "win32" ? `${binary}.exe` : binary,
  );
}

function waitForChild(child, label, logs) {
  return new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("exit", (code, signal) => {
      if (code === 0) resolve();
      else {
        reject(
          new Error(
            `${label} exited with ${signal ? `signal ${signal}` : `status ${code}`}\n${logs.join("")}`,
          ),
        );
      }
    });
  });
}

async function fileBytes(file) {
  try {
    return (await stat(file)).size;
  } catch (error) {
    if (error.code === "ENOENT") return 0;
    throw error;
  }
}

async function databaseFiles(databasePath) {
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

async function databaseContents(databasePath) {
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
  return JSON.parse(stdout)[0] ?? null;
}

function requiredConfigInteger(toml, key) {
  const match = toml.match(new RegExp(`^${key} = ([0-9]+)$`, "mu"));
  if (match === null) throw new Error(`missing integer ${key} in world-service config`);
  return Number(match[1]);
}

function messageBytes(report, kind) {
  return report.traffic.receivedByKind?.[String(kind)]?.payloadBytes ?? 0;
}

function summarizeTrafficBudget(report, serviceConfig) {
  const floorBytesPerSecond = requiredConfigInteger(
    serviceConfig,
    "outbound_bandwidth_floor_bytes_per_second",
  );
  const ceilingBytesPerSecond = requiredConfigInteger(
    serviceConfig,
    "outbound_bandwidth_ceiling_bytes_per_second",
  );
  const burstBytes = requiredConfigInteger(serviceConfig, "outbound_bandwidth_burst_bytes");
  const queueDelayTargetMs = requiredConfigInteger(serviceConfig, "outbound_queue_delay_target_ms");
  const feedbackTimeoutMs = requiredConfigInteger(serviceConfig, "outbound_feedback_timeout_ms");
  const seconds = report.wallTimeMs / 1_000;
  const receivedBytes = report.reports.map((client) => client.traffic.receivedPayloadBytes);
  const bitsPerSecond = receivedBytes.map((bytes) => (bytes * 8) / seconds);
  const envelopeBytes = report.reports.map(
    (client) =>
      ceilingBytesPerSecond * seconds +
      Math.max(burstBytes, client.traffic.maxReceivedFrameBytes ?? 0) +
      1_024,
  );
  const overBudgetClients = report.reports.filter(
    (client, index) => client.traffic.receivedPayloadBytes > envelopeBytes[index],
  ).length;
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
      presenceBytes: report.reports.reduce((sum, client) => sum + messageBytes(client, 16), 0),
      editBytes: report.reports.reduce((sum, client) => sum + messageBytes(client, 12), 0),
      visibleWorldBytes: report.reports.reduce(
        (sum, client) =>
          sum + messageBytes(client, 4) + messageBytes(client, 8) + messageBytes(client, 18),
        0,
      ),
    },
  };
}

async function collectSamples({ servicePid, botPid, databasePath, done }) {
  const service = [];
  const bots = [];
  const database = [];
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

async function collectObserverSamples(observer, expectedBots, started, done) {
  if (observer === null) return null;
  const samples = [];
  const frameMilliseconds = [];
  let rosterReadyMs = null;
  let interactiveWorldReadyMs = null;
  let fullWorldReadyMs = null;
  while (!done.value) {
    const values = assertSnapshotSchema(
      await observer.page.evaluate(() => globalThis.__VOXELS__.snapshot()),
    );
    const remoteAvatars = values[SNAPSHOT.remoteAvatars];
    const elapsedMs = performance.now() - started;
    if (rosterReadyMs === null && remoteAvatars === expectedBots) {
      rosterReadyMs = elapsedMs;
    }
    if (interactiveWorldReadyMs === null && values[SNAPSHOT.interactiveLodsReady] === 1) {
      interactiveWorldReadyMs = elapsedMs;
    }
    if (
      fullWorldReadyMs === null &&
      values[SNAPSHOT.allLodsReady] === 1 &&
      values[SNAPSHOT.surfaceInFlight] === 0 &&
      values[SNAPSHOT.pendingJobs] === 0
    ) {
      fullWorldReadyMs = elapsedMs;
    }
    for (let index = 0; index < values[SNAPSHOT.sampleCount]; index += 1) {
      frameMilliseconds.push(values[FRAME_SAMPLE_START + index * FRAME_SAMPLE_WIDTH]);
    }
    samples.push({
      atUnixMs: Date.now(),
      remoteAvatars,
      avatarParts: values[SNAPSHOT.avatarParts],
      avatarDrawCalls: values[SNAPSHOT.avatarDrawCalls],
      frameMs: values[SNAPSHOT.frameMs],
      cpuMs: values[SNAPSHOT.cpuMs],
      gpuTotalMs: values[SNAPSHOT.gpuTotalMs],
      wasmCommittedMiB: values[SNAPSHOT.wasmCommittedMiB],
      coreGpuMiB: values[SNAPSHOT.coreGpuMiB],
      residentChunks: values[SNAPSHOT.residentChunks],
      visibleChunks: values[SNAPSHOT.visibleChunks],
      drawCalls: values[SNAPSHOT.drawCalls],
      pendingJobs: values[SNAPSHOT.pendingJobs],
      surfaceInFlight: values[SNAPSHOT.surfaceInFlight],
      interactiveLodsReady: values[SNAPSHOT.interactiveLodsReady] === 1,
      allLodsReady: values[SNAPSHOT.allLodsReady] === 1,
    });
    await observer.page.waitForTimeout(OBSERVER_SAMPLE_INTERVAL_MS);
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
    errors: [...observer.errors],
  };
}

function summarizeDatabase(samples, before, after, contents) {
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
  count,
  options,
  fixture,
  service,
  link,
  botBinary,
  runIndex,
  observer,
}) {
  const reportPath = path.join(fixture.directory, `bots-${count}-${runIndex}.json`);
  const before = await databaseFiles(fixture.databasePath);
  if (observer !== null) {
    await observer.page.evaluate(() => globalThis.__VOXELS__.snapshot());
  }
  link.reset();
  const logs = [];
  const bot = spawn(
    botBinary,
    [
      `--world-url=ws://127.0.0.1:${link.port}${WORLD_PATH}`,
      `--presence-url=ws://127.0.0.1:${link.port}${PRESENCE_PATH}`,
      `--origin=http://127.0.0.1:${fixture.browserPort}`,
      `--subprotocol=${WORLD_SUBPROTOCOL}`,
      `--auth-token=${fixture.authToken}`,
      `--bots=${count}`,
      `--duration-seconds=${options.durationSeconds}`,
      `--seed=${0x5eedcafe}`,
      `--layout=${options.layout}`,
      `--report=${reportPath}`,
    ],
    {
      cwd: process.cwd(),
      env: process.env,
      stdio: ["ignore", "pipe", "pipe"],
      detached: process.platform !== "win32",
    },
  );
  for (const stream of [bot.stdout, bot.stderr]) {
    stream.on("data", (bytes) => {
      logs.push(bytes.toString());
      if (logs.length > 200) logs.shift();
    });
  }
  const done = { value: false };
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
      await waitForChild(bot, `bots (${count})`, logs);
    } catch (error) {
      const serviceLogs = service.logs?.join("") ?? "";
      throw new Error(
        `${error instanceof Error ? error.message : String(error)}${
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
  const report = JSON.parse(await readFile(reportPath, "utf8"));
  const serviceConfig = await readFile(fixture.serviceConfigPath, "utf8");
  const after = await databaseFiles(fixture.databasePath);
  const contents = await databaseContents(fixture.databasePath);
  return {
    count,
    wallTimeMs,
    botReport: report,
    trafficBudget: summarizeTrafficBudget(report, serviceConfig),
    process: {
      service: summarizeProcess(samples.service),
      bots: summarizeProcess(samples.bots),
    },
    network: link.snapshot(),
    database: summarizeDatabase(samples.database, before, after, contents),
    observer: observerReport,
  };
}

function markdownReport(result) {
  const lines = [
    "# Voxels bot population benchmark",
    "",
    `Mode: **${result.options.mode}** · layout: **${result.options.layout}** · duration: **${result.options.durationSeconds}s per population** · source: **${result.options.source}**`,
    "",
    "| Bots | Server CPU p95 | Server RSS peak | Bot CPU p95 | TCP down/up | VXWP down/up | DB growth | Edits accepted/conflicts | Mutations | Chunk p95 | Edit p95 | Visible | Roster ready | Observer LOD | Observer frame p95 |",
    "| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
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
      `| ${stage.count} | ${stage.process.service.cpuPercent.p95.toFixed(1)}% | ${stage.process.service.rssMiB.max.toFixed(1)} MiB | ${stage.process.bots.cpuPercent.p95.toFixed(1)}% | ${(stage.network.downstream.streamBytes / 1_048_576).toFixed(2)} / ${(stage.network.upstream.streamBytes / 1_048_576).toFixed(2)} MiB | ${(stage.network.downstream.vxwpPayloadBytes / 1_048_576).toFixed(2)} / ${(stage.network.upstream.vxwpPayloadBytes / 1_048_576).toFixed(2)} MiB | ${(stage.database.deltaBytes / 1_024).toFixed(1)} KiB | ${bot.editsAccepted}/${bot.editConflicts} | ${bot.mutationsCommitted} | ${chunkP95.toFixed(1)} ms | ${editP95.toFixed(1)} ms | ${bot.maxVisiblePlayers} | ${rosterReady} | ${observerLod} | ${observerFrame} |`,
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

function stageViolations(stage, browserEnabled) {
  const violations = [];
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

async function startObserver(browser, previewPort, fixture, proxyPort) {
  const context = await browser.newContext({ viewport: VIEWPORT, deviceScaleFactor: 1 });
  const page = await context.newPage();
  const errors = [];
  const recordError = (message) => {
    if (errors.length < 32 && !errors.includes(message)) errors.push(message);
  };
  page.on("pageerror", (error) => recordError(`pageerror: ${error.message}`));
  page.on("console", (message) => {
    if (isBrowserConsoleFailure(message.type(), message.text(), BROWSER_FAILURE)) {
      recordError(`${message.type()}: ${message.text()}`);
    }
  });
  const clientConfig = (await readFile(fixture.clientConfigPath, "utf8"))
    .replace(/^endpoint = .*$/m, `endpoint = "ws://127.0.0.1:${proxyPort}${WORLD_PATH}"`)
    .replace(
      /^presence_endpoint = .*$/m,
      `presence_endpoint = "ws://127.0.0.1:${proxyPort}${PRESENCE_PATH}"`,
    );
  await page.route("**/config/client.toml", (route) =>
    route.fulfill({
      status: 200,
      contentType: "text/plain; charset=utf-8",
      body: clientConfig,
    }),
  );
  await page.goto(`http://127.0.0.1:${previewPort}`, { waitUntil: "domcontentloaded" });
  await page.waitForFunction(() => typeof globalThis.__VOXELS__?.snapshot === "function", null, {
    timeout: 30_000,
  });
  const deadline = performance.now() + 30_000;
  let latest = [];
  while (performance.now() < deadline) {
    latest = assertSnapshotSchema(await page.evaluate(() => globalThis.__VOXELS__.snapshot()));
    if (latest[SNAPSHOT.quads] > 0 && latest[SNAPSHOT.residentChunks] > 0) {
      return { context, page, errors };
    }
    await page.waitForTimeout(100);
  }
  throw new Error(
    `browser observer did not load terrain: ${JSON.stringify({
      quads: latest[SNAPSHOT.quads],
      residentChunks: latest[SNAPSHOT.residentChunks],
      trackedChunks: latest[SNAPSHOT.trackedChunks],
      pendingJobs: latest[SNAPSHOT.pendingJobs],
      allLodsReady: latest[SNAPSHOT.allLodsReady],
      interactiveLodsReady: latest[SNAPSHOT.interactiveLodsReady],
      errors,
    })}`,
  );
}

async function main() {
  const options = parseArguments(process.argv.slice(2));
  execFileSync(
    rustTool("cargo"),
    worldServiceBuildCargoArgs({ metal: false, profile: options.serviceProfile }),
    { cwd: process.cwd(), env: process.env, stdio: "inherit" },
  );
  execFileSync(rustTool("cargo"), ["build", "--profile", options.botProfile, "-p", "voxels-bots"], {
    cwd: process.cwd(),
    env: process.env,
    stdio: "inherit",
  });
  const botBinary = executablePath(options.botProfile, "voxels-bots");
  const result = {
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
  let growthFixture;
  let growthService;
  let growthLink;
  let browser;
  let previewServer;
  let previewPort;
  try {
    if (options.browser) {
      previewPort = await reserveEphemeralPort();
      const buildFixture = await prepareBrowserWorldFixture({
        browserPort: previewPort,
        prefix: "voxels-bots-browser-build-",
        source: options.source,
      });
      try {
        await build({ logLevel: "warn" });
      } finally {
        await buildFixture.cleanup();
      }
      previewServer = await preview({
        logLevel: "warn",
        preview: { host: "127.0.0.1", port: previewPort, strictPort: true },
      });
      browser = await chromium.launch(chromeWebGpuLaunchOptions());
    }
    for (const [runIndex, count] of options.counts.entries()) {
      const fixture =
        growthFixture ??
        (await prepareBrowserWorldFixture({
          browserPort: previewPort ?? (await reserveEphemeralPort()),
          prefix: `voxels-bots-${count}-`,
          source: options.source,
        }));
      const service =
        growthService ??
        (await startBrowserWorldService(fixture, {
          build: false,
          metal: false,
          profile: options.serviceProfile,
        }));
      let proxy = growthLink;
      if (proxy === undefined) {
        const proxyPort = await reserveEphemeralPort();
        proxy = await createShapedLink({
          listenPort: proxyPort,
          targetPort: fixture.backendPort,
          profile: NETWORK_PROFILE,
        });
        proxy.port = proxyPort;
      }
      if (options.mode === "growth" && growthFixture === undefined) {
        growthFixture = fixture;
        growthService = service;
        growthLink = proxy;
      }
      let observer = null;
      try {
        observer =
          browser === undefined || previewPort === undefined
            ? null
            : await startObserver(browser, previewPort, fixture, proxy.port);
        const stage = await runPopulation({
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
        stage.violations = violations;
        result.violations.push(...violations);
        result.stages.push(stage);
        if (observer !== null && runIndex === options.counts.length - 1) {
          await mkdir(OUTPUT_DIRECTORY, { recursive: true });
          await observer.page.screenshot({
            path: path.join(OUTPUT_DIRECTORY, "latest-observer.png"),
          });
        }
      } finally {
        if (observer !== null) await observer.context.close();
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
    if (browser) await browser.close();
    if (previewServer) await previewServer.close();
  }
  const markdown = markdownReport(result);
  await writeHarnessReport(OUTPUT_DIRECTORY, result, markdown);
  process.stdout.write(`${markdown}\nJSON: ${path.join(OUTPUT_DIRECTORY, "latest.json")}\n`);
  if (result.violations.length > 0) {
    throw new Error(`bot load harness found ${result.violations.length} violation(s)`);
  }
}

await main();
