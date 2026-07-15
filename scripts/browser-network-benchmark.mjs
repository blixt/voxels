import { spawn, execFileSync } from "node:child_process";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { cpus, platform, release, tmpdir } from "node:os";
import path from "node:path";
import { connect } from "node:net";
import { chromium } from "playwright";
import {
  assertSnapshotSchema,
  chromeWebGpuLaunchOptions,
  isBrowserConsoleFailure,
  reserveEphemeralPort,
  SNAPSHOT,
  SNAPSHOT_SCHEMA_VERSION,
} from "./browser-harness.mjs";
import { rustTool } from "./build-wasm.ts";
import { createShapedLink } from "./network-benchmark-link.mjs";
import { PRESENCE_PATH, VXWP_VERSION, WORLD_PATH } from "./vxwp-contract.mjs";
import { worldServiceCargoArgs } from "./world-service-command.ts";

const RESULT_SCHEMA_VERSION = 2;
const FIXTURE_VERSION = 2;
const PREVIEW_HOST = "127.0.0.1";
const VIEWPORT = { width: 1280, height: 720 };
const FRAME_SAMPLE_WIDTH = 5;
const FRAME_SAMPLE_START = SNAPSHOT.droppedSamples + 1;
const SAMPLE_INTERVAL_MS = 16;
const READY_STABLE_SAMPLES = 3;
const SCENARIO_TIMEOUT_MS = 90_000;
const RESIDENT_WALK_METRES = 2;
const STREAMING_WALK_METRES = 35;
const TURN_RADIANS = Math.PI;
const OUTPUT_DIRECTORY = path.resolve("target/network-benchmark");
const FAILURE =
  /panic|unreachable|runtimeerror|wgpu|webgpu|shader|sqlite|opfs|syncaccesshandle|websocket|protocol|world service/i;
const DEFAULT_PROFILE = Object.freeze({
  name: "good_remote",
  roundTripLatencyMs: 40,
  oneWayLatencyMs: 20,
  downstreamMegabitsPerSecond: 50,
  upstreamMegabitsPerSecond: 10,
  jitterMs: 0,
  packetLossPercent: 0,
});

function argumentValue(name, fallback) {
  const prefix = `--${name}=`;
  const value = process.argv.find((argument) => argument.startsWith(prefix))?.slice(prefix.length);
  return value ?? fallback;
}

function positiveInteger(value, name) {
  const parsed = Number.parseInt(value, 10);
  if (!Number.isSafeInteger(parsed) || parsed <= 0) throw new Error(`${name} must be positive`);
  return parsed;
}

function sleep(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

function percentile(values, fraction) {
  if (values.length === 0) return null;
  const sorted = [...values].sort((left, right) => left - right);
  return sorted[Math.min(sorted.length - 1, Math.ceil(sorted.length * fraction) - 1)];
}

function rounded(value, digits = 1) {
  const scale = 10 ** digits;
  return Math.round(value * scale) / scale;
}

function numericSummary(values) {
  if (values.length === 0) return { median: null, p95: null, max: null };
  return {
    median: rounded(percentile(values, 0.5)),
    p95: rounded(percentile(values, 0.95)),
    max: rounded(Math.max(...values)),
  };
}

function frameTimingSummary(samples, droppedSamples) {
  const column = (index) => samples.map((sample) => sample[index]);
  const frameMs = column(0);
  return {
    samples: samples.length,
    droppedSamples,
    frameMs: {
      ...numericSummary(frameMs),
      above33_33ms: frameMs.filter((value) => value > 33.33).length,
    },
    cpuMs: numericSummary(column(1)),
    streamingMs: numericSummary(column(3)),
  };
}

function byteSummary(stats) {
  const world = stats.paths[WORLD_PATH] ?? {
    upstream: { streamBytes: 0, vxwpPayloadBytes: 0 },
    downstream: { streamBytes: 0, vxwpPayloadBytes: 0 },
  };
  const presence = stats.paths[PRESENCE_PATH] ?? {
    upstream: { streamBytes: 0, vxwpPayloadBytes: 0 },
    downstream: { streamBytes: 0, vxwpPayloadBytes: 0 },
  };
  return {
    total: stats.upstream.streamBytes + stats.downstream.streamBytes,
    upstream: stats.upstream.streamBytes,
    downstream: stats.downstream.streamBytes,
    worldUpstream: world.upstream.streamBytes,
    worldDownstream: world.downstream.streamBytes,
    presenceUpstream: presence.upstream.streamBytes,
    presenceDownstream: presence.downstream.streamBytes,
    vxwpUpstream: stats.upstream.vxwpPayloadBytes,
    vxwpDownstream: stats.downstream.vxwpPayloadBytes,
  };
}

function viewportSignature(snapshot) {
  return [
    snapshot[SNAPSHOT.viewportFingerprintLow24],
    snapshot[SNAPSHOT.viewportFingerprintHigh24],
    snapshot[SNAPSHOT.visibleChunks],
    snapshot[SNAPSHOT.quads],
    snapshot[SNAPSHOT.waterQuads],
  ].join(":");
}

function sameSignature(left, right) {
  return left.signature === right.signature;
}

function firstPermanentlyFinalSample(samples, actionFinishedMs) {
  const final = samples.at(-1);
  if (!final) return null;
  for (let index = 0; index < samples.length; index += 1) {
    const sample = samples[index];
    if (sample.elapsedMs < actionFinishedMs || !sameSignature(sample, final)) continue;
    if (samples.slice(index).every((candidate) => sameSignature(candidate, final))) return sample;
  }
  return final;
}

async function waitForSnapshotApi(page) {
  await page.waitForFunction(() => typeof globalThis.__VOXELS__?.snapshot === "function", null, {
    timeout: 30_000,
  });
}

async function captureSnapshot(page, frameState) {
  const current = assertSnapshotSchema(await page.evaluate(() => globalThis.__VOXELS__.snapshot()));
  const count = current[SNAPSHOT.sampleCount];
  for (let index = 0; index < count; index += 1) {
    const start = FRAME_SAMPLE_START + index * FRAME_SAMPLE_WIDTH;
    frameState.samples.push(current.slice(start, start + FRAME_SAMPLE_WIDTH));
  }
  frameState.droppedSamples += current[SNAPSHOT.droppedSamples];
  return current;
}

function observePage(page, label, errors) {
  page.on("pageerror", (error) => errors.push(`${label} pageerror: ${error.message}`));
  page.on("console", (message) => {
    if (isBrowserConsoleFailure(message.type(), message.text(), FAILURE)) {
      errors.push(`${label} ${message.type()}: ${message.text()}`);
    }
  });
}

async function turn(page, radians, { capture, markMeasurementStart }) {
  const before = await capture();
  markMeasurementStart();
  await page.evaluate((movementX) => {
    globalThis.__VOXELS__.look(movementX, 0);
  }, radians / 0.0022);
  await page.waitForTimeout(80);
  const after = await capture();
  const delta = Math.abs(
    Math.atan2(
      Math.sin(after[SNAPSHOT.yaw] - before[SNAPSHOT.yaw]),
      Math.cos(after[SNAPSHOT.yaw] - before[SNAPSHOT.yaw]),
    ),
  );
  if (delta < Math.abs(radians) * 0.8) {
    throw new Error(`turn fixture rotated only ${delta.toFixed(3)} radians`);
  }
  return { yawDeltaRadians: delta };
}

async function walkDistance(
  page,
  targetDistanceMetres,
  { capture, sprint = false, key = "KeyW", timeoutMs = 15_000 },
) {
  const before = await capture();
  const started = performance.now();
  if (sprint) await page.keyboard.down("ShiftLeft");
  await page.keyboard.down(key);
  let after = before;
  try {
    while (performance.now() - started < timeoutMs) {
      await page.waitForTimeout(25);
      after = await capture();
      const distance = Math.hypot(
        after[SNAPSHOT.cameraX] - before[SNAPSHOT.cameraX],
        after[SNAPSHOT.cameraZ] - before[SNAPSHOT.cameraZ],
      );
      if (distance >= targetDistanceMetres) break;
    }
  } finally {
    await page.keyboard.up(key);
    if (sprint) await page.keyboard.up("ShiftLeft");
  }
  const distanceMetres = Math.hypot(
    after[SNAPSHOT.cameraX] - before[SNAPSHOT.cameraX],
    after[SNAPSHOT.cameraZ] - before[SNAPSHOT.cameraZ],
  );
  if (distanceMetres < targetDistanceMetres) {
    throw new Error(
      `walk fixture covered ${distanceMetres.toFixed(2)} of ${targetDistanceMetres} metres`,
    );
  }
  return {
    requestedDistanceMetres: targetDistanceMetres,
    actionDurationMs: rounded(performance.now() - started),
    distanceMetres: rounded(distanceMetres, 3),
  };
}

async function runScenario({ name, page, link, action, errors, timeoutMs = SCENARIO_TIMEOUT_MS }) {
  link.reset();
  const started = performance.now();
  const frameState = { samples: [], droppedSamples: 0 };
  let actionResult;
  let actionFinishedMs = null;
  let markedMeasurementStartedMs = null;
  let actionError;
  const actionContext = {
    capture: () => captureSnapshot(page, frameState),
    markMeasurementStart: () => {
      markedMeasurementStartedMs ??= performance.now() - started;
      return markedMeasurementStartedMs;
    },
  };
  const actionPromise = Promise.resolve()
    .then(() => action(actionContext))
    .then((result) => {
      actionResult = result;
      actionFinishedMs = performance.now() - started;
    })
    .catch((error) => {
      actionError = error;
      actionFinishedMs = performance.now() - started;
    });
  const samples = [];
  let consecutiveReady = 0;
  while (performance.now() - started < timeoutMs) {
    if (actionError) throw actionError;
    await sleep(SAMPLE_INTERVAL_MS);
    let current;
    try {
      current = await captureSnapshot(page, frameState);
    } catch (error) {
      if (performance.now() - started < 30_000) continue;
      throw error;
    }
    const elapsedMs = performance.now() - started;
    const wire = byteSummary(link.snapshot());
    samples.push({
      elapsedMs,
      signature: viewportSignature(current),
      visibleChunks: current[SNAPSHOT.visibleChunks],
      quads: current[SNAPSHOT.quads],
      pendingJobs: current[SNAPSHOT.pendingJobs],
      surfaceInFlight: current[SNAPSHOT.surfaceInFlight],
      interactiveLodsReady: current[SNAPSHOT.interactiveLodsReady] === 1,
      stride32Tiles: current[SNAPSHOT.stride32Tiles],
      stride64Tiles: current[SNAPSHOT.stride64Tiles],
      allLodsReady: current[SNAPSHOT.allLodsReady] === 1,
      bytes: wire,
    });
    const measurementStartedMs = markedMeasurementStartedMs ?? actionFinishedMs;
    const ready =
      measurementStartedMs !== null &&
      current[SNAPSHOT.allLodsReady] === 1 &&
      current[SNAPSHOT.visibleChunks] > 0 &&
      current[SNAPSHOT.quads] > 0;
    consecutiveReady = ready
      ? consecutiveReady > 0 && sameSignature(samples.at(-2), samples.at(-1))
        ? consecutiveReady + 1
        : 1
      : 0;
    if (consecutiveReady >= READY_STABLE_SAMPLES) break;
  }
  await actionPromise;
  if (actionError) throw actionError;
  if (consecutiveReady < READY_STABLE_SAMPLES) {
    throw new Error(
      `${name} did not reach complete LOD coverage: ${JSON.stringify(samples.at(-1))}`,
    );
  }
  if (errors.length > 0) throw new Error(errors.join("\n"));
  const measurementStartedMs = markedMeasurementStartedMs ?? actionFinishedMs;
  const final = samples.at(-1);
  const fullCoverage = samples.at(-READY_STABLE_SAMPLES);
  const informed = firstPermanentlyFinalSample(samples, measurementStartedMs);
  const interactive = samples.find(
    (sample) => sample.elapsedMs >= measurementStartedMs && sample.interactiveLodsReady,
  );
  const firstUseful = samples.find((sample) => sample.visibleChunks > 0 && sample.quads > 0);
  const stats = link.snapshot();
  return {
    name,
    action: actionResult ?? {},
    actionFinishedMs: rounded(actionFinishedMs),
    measurementStartedFromStartMs: rounded(measurementStartedMs),
    firstUsefulMs: firstUseful ? rounded(firstUseful.elapsedMs) : null,
    interactiveCoverageReadyMs: rounded(interactive.elapsedMs - measurementStartedMs),
    viewportFullyInformedMs: rounded(informed.elapsedMs - measurementStartedMs),
    viewportFullyInformedFromStartMs: rounded(informed.elapsedMs),
    fullCoverageSettledMs: rounded(fullCoverage.elapsedMs - measurementStartedMs),
    fullCoverageSettledFromStartMs: rounded(fullCoverage.elapsedMs),
    bytesAtViewportInformed: informed.bytes,
    bytesAtInteractiveCoverage: interactive.bytes,
    bytesAtFullCoverage: fullCoverage.bytes,
    finalViewport: {
      signature: final.signature,
      visibleChunks: final.visibleChunks,
      quads: final.quads,
    },
    messages: stats.messages,
    frameTiming: frameTimingSummary(frameState.samples, frameState.droppedSamples),
    sampleCount: samples.length,
  };
}

async function waitForPort(port, child, logs) {
  const deadline = performance.now() + 90_000;
  while (performance.now() < deadline) {
    if (child.exitCode !== null) {
      throw new Error(`world service exited with ${child.exitCode}\n${logs.join("")}`);
    }
    const connected = await new Promise((resolve) => {
      const socket = connect({ host: "127.0.0.1", port });
      socket.once("connect", () => {
        socket.destroy();
        resolve(true);
      });
      socket.once("error", () => resolve(false));
    });
    if (connected) return;
    await sleep(100);
  }
  throw new Error(`world service did not listen on ${port}\n${logs.join("")}`);
}

function stopChild(child) {
  if (!child || child.exitCode !== null) return Promise.resolve();
  child.kill("SIGTERM");
  return Promise.race([
    new Promise((resolve) => child.once("exit", resolve)),
    sleep(3_000).then(() => {
      if (child.exitCode === null) child.kill("SIGKILL");
    }),
  ]);
}

function aggregateRuns(runs) {
  const grouped = Map.groupBy(runs, (run) => run.name);
  return Object.fromEntries(
    [...grouped.entries()].map(([name, scenarios]) => {
      const viewport = scenarios.map((scenario) => scenario.viewportFullyInformedMs);
      const interactive = scenarios.map((scenario) => scenario.interactiveCoverageReadyMs);
      const coverage = scenarios.map((scenario) => scenario.fullCoverageSettledMs);
      const viewportBytes = scenarios.map((scenario) => scenario.bytesAtViewportInformed.total);
      const interactiveWorldBytes = scenarios.map(
        (scenario) => scenario.bytesAtInteractiveCoverage.worldDownstream,
      );
      const viewportWorldBytes = scenarios.map(
        (scenario) => scenario.bytesAtViewportInformed.worldDownstream,
      );
      const coverageBytes = scenarios.map((scenario) => scenario.bytesAtFullCoverage.total);
      const coverageWorldBytes = scenarios.map(
        (scenario) => scenario.bytesAtFullCoverage.worldDownstream,
      );
      const frameP95 = scenarios
        .map((scenario) => scenario.frameTiming.frameMs.p95)
        .filter((value) => value !== null);
      const frameMax = scenarios
        .map((scenario) => scenario.frameTiming.frameMs.max)
        .filter((value) => value !== null);
      const streamingP95 = scenarios
        .map((scenario) => scenario.frameTiming.streamingMs.p95)
        .filter((value) => value !== null);
      return [
        name,
        {
          runs: scenarios.length,
          viewportFullyInformedMs: {
            median: rounded(percentile(viewport, 0.5)),
            p95: rounded(percentile(viewport, 0.95)),
            min: rounded(Math.min(...viewport)),
            max: rounded(Math.max(...viewport)),
          },
          interactiveCoverageReadyMs: {
            median: rounded(percentile(interactive, 0.5)),
            p95: rounded(percentile(interactive, 0.95)),
            max: rounded(Math.max(...interactive)),
          },
          fullCoverageSettledMs: {
            median: rounded(percentile(coverage, 0.5)),
            p95: rounded(percentile(coverage, 0.95)),
            max: rounded(Math.max(...coverage)),
          },
          bytesAtViewportInformed: {
            medianTotal: percentile(viewportBytes, 0.5),
            medianWorldDownstream: percentile(viewportWorldBytes, 0.5),
          },
          bytesAtInteractiveCoverage: {
            medianWorldDownstream: percentile(interactiveWorldBytes, 0.5),
          },
          bytesAtFullCoverage: {
            medianTotal: percentile(coverageBytes, 0.5),
            medianWorldDownstream: percentile(coverageWorldBytes, 0.5),
          },
          frameTiming: {
            medianP95Ms: rounded(percentile(frameP95, 0.5)),
            maxMs: rounded(Math.max(...frameMax)),
            streamingMedianP95Ms: rounded(percentile(streamingP95, 0.5)),
            above33_33ms: scenarios.reduce(
              (total, scenario) => total + scenario.frameTiming.frameMs.above33_33ms,
              0,
            ),
            samples: scenarios.reduce((total, scenario) => total + scenario.frameTiming.samples, 0),
            droppedSamples: scenarios.reduce(
              (total, scenario) => total + scenario.frameTiming.droppedSamples,
              0,
            ),
          },
        },
      ];
    }),
  );
}

function markdownReport(result) {
  const rows = Object.entries(result.summary).map(([name, summary]) => {
    const viewport = summary.viewportFullyInformedMs;
    return `| ${name} | ${summary.interactiveCoverageReadyMs.median.toFixed(1)} | ${viewport.median.toFixed(1)} | ${viewport.max.toFixed(1)} | ${summary.fullCoverageSettledMs.median.toFixed(1)} | ${summary.bytesAtInteractiveCoverage.medianWorldDownstream.toLocaleString("en-US")} | ${summary.bytesAtViewportInformed.medianWorldDownstream.toLocaleString("en-US")} | ${summary.bytesAtFullCoverage.medianTotal.toLocaleString("en-US")} |`;
  });
  const frameRows = Object.entries(result.summary).map(([name, summary]) => {
    const frame = summary.frameTiming;
    return `| ${name} | ${frame.medianP95Ms.toFixed(1)} | ${frame.maxMs.toFixed(1)} | ${frame.above33_33ms.toLocaleString("en-US")} / ${frame.samples.toLocaleString("en-US")} | ${frame.streamingMedianP95Ms.toFixed(1)} | ${frame.droppedSamples.toLocaleString("en-US")} |`;
  });
  return `# Remote world streaming benchmark\n\nGenerated ${result.generatedAt} at commit \`${result.git.commit}\`${result.git.dirty ? " (dirty)" : ""}.\n\nLink profile: ${result.link.roundTripLatencyMs} ms RTT, ${result.link.downstreamMegabitsPerSecond} Mbit/s down, ${result.link.upstreamMegabitsPerSecond} Mbit/s up, no jitter or loss. Both WebSockets share one bandwidth clock per direction. Counts are TCP stream bytes delivered by the user-space proxy; they include HTTP/WebSocket framing but exclude TCP/IP/TLS overhead.\n\n| Scenario | Interactive ready median (ms) | Viewport informed median (ms) | max (ms) | Full coverage median (ms) | World bytes at interactive | World bytes at viewport | Total bytes at full coverage |\n| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |\n${rows.join("\n")}\n\n“Interactive ready” is when canonical terrain and the original four surface rings are complete; kilometre horizon prefetch starts only afterward. “Viewport informed” is the earliest post-action sample whose presented-geometry fingerprint equals the final fully settled viewport and stays equal. “Full coverage” is the first of three consecutive matching samples where every canonical and surface LOD queue and in-flight stage is settled. Turn timing starts when look input is issued; walking covers a fixed distance.\n\n## Main-thread frame timing\n\n| Scenario | Median run p95 (ms) | Worst frame (ms) | Frames >33.33 ms | Streaming p95 (ms) | Dropped samples |\n| --- | ---: | ---: | ---: | ---: | ---: |\n${frameRows.join("\n")}\n`;
}

async function main() {
  const repetitions = positiveInteger(argumentValue("runs", "5"), "runs");
  const profile = { ...DEFAULT_PROFILE };
  const temporary = await mkdtemp(path.join(tmpdir(), "voxels-network-benchmark-"));
  const backendPort = await reserveEphemeralPort();
  const proxyPort = await reserveEphemeralPort();
  const previewPort = await reserveEphemeralPort();
  const serviceConfigPath = path.join(temporary, "world-service.toml");
  const clientConfigPath = path.join(temporary, "client.toml");
  const [serviceSource, clientSource] = await Promise.all([
    readFile("config/world-service.toml", "utf8"),
    readFile("config/client.toml", "utf8"),
  ]);
  await writeFile(
    serviceConfigPath,
    serviceSource
      .replace(/^listen = .*$/m, `listen = "127.0.0.1:${backendPort}"`)
      .replace(
        /^allowed_origins = .*$/m,
        `allowed_origins = ["http://${PREVIEW_HOST}:${previewPort}"]`,
      )
      .replace(/^database = .*$/m, 'database = "world-state-v5.sqlite3"'),
  );
  await writeFile(
    clientConfigPath,
    clientSource
      .replace(/^endpoint = .*$/m, `endpoint = "ws://127.0.0.1:${proxyPort}${WORLD_PATH}"`)
      .replace(
        /^presence_endpoint = .*$/m,
        `presence_endpoint = "ws://127.0.0.1:${proxyPort}${PRESENCE_PATH}"`,
      ),
  );
  process.env.VOXELS_CLIENT_CONFIG_PATH = clientConfigPath;

  let browser;
  let previewServer;
  let worldService;
  let link;
  const worldLogs = [];
  try {
    const { build, preview } = await import("vite-plus");
    await build({ mode: "production" });
    worldService = spawn(
      rustTool("cargo"),
      worldServiceCargoArgs({ metal: true, configPath: serviceConfigPath }),
      { stdio: ["ignore", "pipe", "pipe"] },
    );
    for (const stream of [worldService.stdout, worldService.stderr]) {
      stream.on("data", (bytes) => {
        worldLogs.push(bytes.toString());
        if (worldLogs.length > 200) worldLogs.shift();
      });
    }
    await waitForPort(backendPort, worldService, worldLogs);
    link = await createShapedLink({ listenPort: proxyPort, targetPort: backendPort, profile });
    previewServer = await preview({
      root: process.cwd(),
      preview: { host: PREVIEW_HOST, port: previewPort, strictPort: true },
    });
    browser = await chromium.launch(chromeWebGpuLaunchOptions());
    const runs = [];
    for (let repetition = 0; repetition < repetitions; repetition += 1) {
      const errors = [];
      const context = await browser.newContext({ viewport: VIEWPORT });
      const page = await context.newPage();
      observePage(page, `run-${repetition + 1}`, errors);
      const url = `http://${PREVIEW_HOST}:${previewPort}/?player=network-bench-${repetition + 1}`;
      runs.push(
        await runScenario({
          name: "cold_spawn",
          page,
          link,
          errors,
          action: async () => {
            await page.goto(url, { waitUntil: "domcontentloaded" });
            await waitForSnapshotApi(page);
          },
        }),
      );
      runs.push(
        await runScenario({
          name: "resident_walk",
          page,
          link,
          errors,
          // Spawn faces -Z from the exact chunk boundary. Walking backward stays inside the
          // already-resident spawn chunk and is the control for unexpected world traffic.
          action: (context) =>
            walkDistance(page, RESIDENT_WALK_METRES, { ...context, key: "KeyS" }),
        }),
      );
      runs.push(
        await runScenario({
          name: "cached_turn_180",
          page,
          link,
          errors,
          action: (context) => turn(page, TURN_RADIANS, context),
        }),
      );
      runs.push(
        await runScenario({
          name: "streaming_walk",
          page,
          link,
          errors,
          action: (context) =>
            walkDistance(page, STREAMING_WALK_METRES, { ...context, sprint: true }),
        }),
      );
      await context.close();

      const pivotErrors = [];
      const pivotContext = await browser.newContext({ viewport: VIEWPORT });
      const pivotPage = await pivotContext.newPage();
      observePage(pivotPage, `pivot-${repetition + 1}`, pivotErrors);
      runs.push(
        await runScenario({
          name: "turn_during_spawn",
          page: pivotPage,
          link,
          errors: pivotErrors,
          action: async (context) => {
            await pivotPage.goto(
              `http://${PREVIEW_HOST}:${previewPort}/?player=network-pivot-${repetition + 1}`,
              { waitUntil: "domcontentloaded" },
            );
            await waitForSnapshotApi(pivotPage);
            const deadline = performance.now() + 30_000;
            while (performance.now() < deadline) {
              const current = await context.capture();
              if (
                current[SNAPSHOT.visibleChunks] > 0 &&
                current[SNAPSHOT.quads] > 0 &&
                current[SNAPSHOT.allLodsReady] === 0
              ) {
                return turn(pivotPage, TURN_RADIANS, context);
              }
              await pivotPage.waitForTimeout(SAMPLE_INTERVAL_MS);
            }
            throw new Error("turn-during-spawn fixture did not observe partial coverage");
          },
        }),
      );
      await pivotContext.close();
    }

    const git = {
      commit: execFileSync("git", ["rev-parse", "--short=12", "HEAD"], { encoding: "utf8" }).trim(),
      dirty: execFileSync("git", ["status", "--porcelain"], { encoding: "utf8" }).trim() !== "",
    };
    const result = {
      schemaVersion: RESULT_SCHEMA_VERSION,
      generatedAt: new Date().toISOString(),
      git,
      environment: {
        platform: `${platform()} ${release()}`,
        cpu: cpus()[0]?.model ?? "unknown",
        logicalCpus: cpus().length,
        chrome: browser.version(),
        node: process.version,
      },
      browserSnapshotSchema: SNAPSHOT_SCHEMA_VERSION,
      fixture: {
        version: FIXTURE_VERSION,
        viewport: VIEWPORT,
        sampleIntervalMs: SAMPLE_INTERVAL_MS,
        readyStableSamples: READY_STABLE_SAMPLES,
        residentWalkMetres: RESIDENT_WALK_METRES,
        streamingWalkMetres: STREAMING_WALK_METRES,
        turnRadians: TURN_RADIANS,
      },
      protocol: {
        name: "VXWP",
        version: VXWP_VERSION,
        resultCompression: { codec: "brotli", quality: 2, windowBits: 20 },
      },
      link: { ...profile, ...link.profile },
      repetitions,
      scenarios: [
        "cold_spawn",
        "resident_walk",
        "cached_turn_180",
        "streaming_walk",
        "turn_during_spawn",
      ],
      summary: aggregateRuns(runs),
      runs,
    };
    await mkdir(OUTPUT_DIRECTORY, { recursive: true });
    const stamp = result.generatedAt.replaceAll(":", "-");
    const jsonPath = path.join(OUTPUT_DIRECTORY, `${stamp}.json`);
    const markdownPath = path.join(OUTPUT_DIRECTORY, `${stamp}.md`);
    const report = markdownReport(result);
    await Promise.all([
      writeFile(jsonPath, `${JSON.stringify(result, null, 2)}\n`),
      writeFile(markdownPath, report),
      writeFile(path.join(OUTPUT_DIRECTORY, "latest.json"), `${JSON.stringify(result, null, 2)}\n`),
      writeFile(path.join(OUTPUT_DIRECTORY, "latest.md"), report),
    ]);
    process.stdout.write(`${report}\nJSON: ${jsonPath}\nMarkdown: ${markdownPath}\n`);
  } finally {
    await browser?.close();
    await previewServer?.close();
    await link?.close();
    await stopChild(worldService);
    await rm(temporary, { recursive: true, force: true });
  }
}

await main();
