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

const RESULT_SCHEMA_VERSION = 1;
const PREVIEW_HOST = "127.0.0.1";
const VIEWPORT = { width: 1280, height: 720 };
const SAMPLE_INTERVAL_MS = 50;
const READY_STABLE_SAMPLES = 3;
const SCENARIO_TIMEOUT_MS = 90_000;
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

function byteSummary(stats) {
  const world = stats.paths["/v3/world"] ?? {
    upstream: { streamBytes: 0, vxwpPayloadBytes: 0 },
    downstream: { streamBytes: 0, vxwpPayloadBytes: 0 },
  };
  const presence = stats.paths["/v3/presence"] ?? {
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

async function snapshot(page) {
  return assertSnapshotSchema(await page.evaluate(() => globalThis.__VOXELS__.snapshot()));
}

function observePage(page, label, errors) {
  page.on("pageerror", (error) => errors.push(`${label} pageerror: ${error.message}`));
  page.on("console", (message) => {
    if (isBrowserConsoleFailure(message.type(), message.text(), FAILURE)) {
      errors.push(`${label} ${message.type()}: ${message.text()}`);
    }
  });
}

async function turn(page, radians) {
  const before = await snapshot(page);
  await page.evaluate((movementX) => {
    globalThis.__VOXELS__.look(movementX, 0);
  }, radians / 0.0022);
  await page.waitForTimeout(80);
  const after = await snapshot(page);
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

async function walk(page, durationMs, sprint = false, key = "KeyW") {
  const before = await snapshot(page);
  if (sprint) await page.keyboard.down("ShiftLeft");
  await page.keyboard.down(key);
  await page.waitForTimeout(durationMs);
  await page.keyboard.up(key);
  if (sprint) await page.keyboard.up("ShiftLeft");
  const after = await snapshot(page);
  return {
    requestedDurationMs: durationMs,
    distanceMetres: Math.hypot(
      after[SNAPSHOT.cameraX] - before[SNAPSHOT.cameraX],
      after[SNAPSHOT.cameraZ] - before[SNAPSHOT.cameraZ],
    ),
  };
}

async function runScenario({ name, page, link, action, errors, timeoutMs = SCENARIO_TIMEOUT_MS }) {
  link.reset();
  const started = performance.now();
  let actionResult;
  let actionFinishedMs = null;
  let actionError;
  const actionPromise = Promise.resolve()
    .then(action)
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
      current = await snapshot(page);
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
      allLodsReady: current[SNAPSHOT.allLodsReady] === 1,
      bytes: wire,
    });
    const ready =
      actionFinishedMs !== null &&
      current[SNAPSHOT.allLodsReady] === 1 &&
      current[SNAPSHOT.visibleChunks] > 0 &&
      current[SNAPSHOT.quads] > 0;
    consecutiveReady = ready ? consecutiveReady + 1 : 0;
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
  const final = samples.at(-1);
  const informed = firstPermanentlyFinalSample(samples, actionFinishedMs);
  const firstUseful = samples.find((sample) => sample.visibleChunks > 0 && sample.quads > 0);
  const stats = link.finishStats();
  return {
    name,
    action: actionResult ?? {},
    actionFinishedMs: rounded(actionFinishedMs),
    firstUsefulMs: firstUseful ? rounded(firstUseful.elapsedMs) : null,
    viewportFullyInformedMs: rounded(informed.elapsedMs - actionFinishedMs),
    viewportFullyInformedFromStartMs: rounded(informed.elapsedMs),
    fullCoverageSettledMs: rounded(final.elapsedMs - actionFinishedMs),
    fullCoverageSettledFromStartMs: rounded(final.elapsedMs),
    bytesAtViewportInformed: informed.bytes,
    bytesAtFullCoverage: final.bytes,
    finalViewport: {
      signature: final.signature,
      visibleChunks: final.visibleChunks,
      quads: final.quads,
    },
    messages: stats.messages,
    compression: stats.compression,
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
      const coverage = scenarios.map((scenario) => scenario.fullCoverageSettledMs);
      const bytes = scenarios.map((scenario) => scenario.bytesAtFullCoverage.total);
      const down = scenarios.map((scenario) => scenario.bytesAtFullCoverage.downstream);
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
          fullCoverageSettledMs: {
            median: rounded(percentile(coverage, 0.5)),
            p95: rounded(percentile(coverage, 0.95)),
          },
          streamBytes: {
            medianTotal: percentile(bytes, 0.5),
            medianDownstream: percentile(down, 0.5),
          },
        },
      ];
    }),
  );
}

function markdownReport(result) {
  const rows = Object.entries(result.summary).map(([name, summary]) => {
    const viewport = summary.viewportFullyInformedMs;
    return `| ${name} | ${viewport.median.toFixed(1)} | ${viewport.p95.toFixed(1)} | ${summary.fullCoverageSettledMs.median.toFixed(1)} | ${summary.streamBytes.medianTotal.toLocaleString("en-US")} | ${summary.streamBytes.medianDownstream.toLocaleString("en-US")} |`;
  });
  const compression = {};
  for (const run of result.runs) {
    for (const [kind, values] of Object.entries(run.compression)) {
      compression[kind] ??= {
        rawBytes: 0,
        zstdLevel1Bytes: 0,
        brotliQuality4Bytes: 0,
        deflateLevel1Bytes: 0,
      };
      for (const field of Object.keys(compression[kind])) compression[kind][field] += values[field];
    }
  }
  const compressionRows = Object.entries(compression).map(([kind, values]) => {
    const ratio = values.rawBytes === 0 ? 0 : (1 - values.zstdLevel1Bytes / values.rawBytes) * 100;
    return `| ${kind} | ${values.rawBytes.toLocaleString("en-US")} | ${values.zstdLevel1Bytes.toLocaleString("en-US")} | ${ratio.toFixed(1)}% | ${values.brotliQuality4Bytes.toLocaleString("en-US")} | ${values.deflateLevel1Bytes.toLocaleString("en-US")} |`;
  });
  return `# Remote world streaming benchmark\n\nGenerated ${result.generatedAt} at commit \`${result.git.commit}\`${result.git.dirty ? " (dirty)" : ""}.\n\nLink profile: ${result.link.roundTripLatencyMs} ms RTT, ${result.link.downstreamMegabitsPerSecond} Mbit/s down, ${result.link.upstreamMegabitsPerSecond} Mbit/s up, no jitter or loss. Counts are TCP stream bytes delivered by the user-space proxy; they include HTTP/WebSocket framing but exclude TCP/IP/TLS overhead.\n\n| Scenario | Viewport informed median (ms) | p95 (ms) | Full coverage after action median (ms) | Total stream bytes | Downstream bytes |\n| --- | ---: | ---: | ---: | ---: | ---: |\n${rows.join("\n")}\n\n“Viewport informed” is the earliest post-action sample whose presented-geometry fingerprint equals the final fully settled viewport and stays equal. “Full coverage” additionally requires every canonical and surface LOD queue and in-flight stage to settle.\n\n## Compression headroom\n\nThese are offline, independently compressed VXWP result messages; compression time is outside scenario timing.\n\n| Message | Current bytes | zstd level 1 | zstd reduction | Brotli q4 | Deflate level 1 |\n| --- | ---: | ---: | ---: | ---: | ---: |\n${compressionRows.join("\n")}\n`;
}

async function main() {
  const repetitions = positiveInteger(argumentValue("runs", "3"), "runs");
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
      ),
  );
  await writeFile(
    clientConfigPath,
    clientSource
      .replace(/^endpoint = .*$/m, `endpoint = "ws://127.0.0.1:${proxyPort}/v3/world"`)
      .replace(
        /^presence_endpoint = .*$/m,
        `presence_endpoint = "ws://127.0.0.1:${proxyPort}/v3/presence"`,
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
      [
        "run",
        "--profile",
        "worldgen",
        "-p",
        "voxels-world-service",
        "--bin",
        "voxels-worldd",
        "--",
        serviceConfigPath,
      ],
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
          action: () => walk(page, 500, false, "KeyS"),
        }),
      );
      runs.push(
        await runScenario({
          name: "cached_turn_180",
          page,
          link,
          errors,
          action: () => turn(page, Math.PI),
        }),
      );
      runs.push(
        await runScenario({
          name: "streaming_walk",
          page,
          link,
          errors,
          action: () => walk(page, 5_000, true),
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
          action: async () => {
            await pivotPage.goto(
              `http://${PREVIEW_HOST}:${previewPort}/?player=network-pivot-${repetition + 1}`,
              { waitUntil: "domcontentloaded" },
            );
            await waitForSnapshotApi(pivotPage);
            await pivotPage.waitForFunction(
              ({ visible, quads, ready }) =>
                globalThis.__VOXELS__
                  .snapshot()
                  .then(
                    (values) => values[visible] > 0 && values[quads] > 0 && values[ready] === 0,
                  ),
              {
                visible: SNAPSHOT.visibleChunks,
                quads: SNAPSHOT.quads,
                ready: SNAPSHOT.allLodsReady,
              },
              { timeout: 30_000 },
            );
            return turn(pivotPage, Math.PI);
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
      link: { ...profile, quantumBytes: link.profile.quantumBytes },
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
