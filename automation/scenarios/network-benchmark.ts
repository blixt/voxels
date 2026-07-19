import { spawn, execFileSync } from "node:child_process";
import type { ChildProcess } from "node:child_process";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { cpus, platform, release, tmpdir } from "node:os";
import path from "node:path";
import { connect } from "node:net";
import { chromium } from "playwright";
import type { Browser, Page } from "playwright";
import {
  chromeWebGpuLaunchOptions,
  isBrowserConsoleFailure,
  reserveEphemeralPort,
} from "../lib/browser.ts";
import {
  assertSnapshotSchema,
  FRAME_SAMPLE_WIDTH,
  SNAPSHOT,
  SNAPSHOT_SCHEMA_VERSION,
  snapshotValue,
} from "../lib/engine.ts";
import { rustTool } from "../../scripts/build-wasm.ts";
import { createShapedLink, type LinkStats, type ShapedLink } from "../lib/network.ts";
import { PRESENCE_PATH, VXWP_VERSION, WORLD_PATH } from "../lib/protocol.ts";
import { defineScenario, type ScenarioContext } from "../lib/scenario.ts";
import {
  worldServiceBuildCargoArgs,
  worldServiceCargoArgs,
} from "../../scripts/world-service-command.ts";

const RESULT_SCHEMA_VERSION = 4;
const FIXTURE_VERSION = 2;
const PREVIEW_HOST = "127.0.0.1";
const VIEWPORT = { width: 1280, height: 720 };
const SAMPLE_INTERVAL_MS = 16;
const FRAME_SAMPLE_START = SNAPSHOT.droppedSamples + 1;
const READY_STABLE_SAMPLES = 3;
const SCENARIO_TIMEOUT_MS = 90_000;
const RESIDENT_WALK_METRES = 2;
const STREAMING_WALK_METRES = 35;
const TURN_RADIANS = Math.PI;
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

interface NumberSummary {
  readonly median: number | null;
  readonly p95: number | null;
  readonly max: number | null;
}

interface ByteSummary {
  readonly total: number;
  readonly upstream: number;
  readonly downstream: number;
  readonly worldUpstream: number;
  readonly worldDownstream: number;
  readonly presenceUpstream: number;
  readonly presenceDownstream: number;
  readonly vxwpUpstream: number;
  readonly vxwpDownstream: number;
}

interface ViewportSample {
  readonly elapsedMs: number;
  readonly signature: string;
  readonly visibleChunks: number;
  readonly quads: number;
  readonly pendingJobs: number;
  readonly surfaceInFlight: number;
  readonly interactiveLodsReady: boolean;
  readonly stride32Tiles: number;
  readonly stride64Tiles: number;
  readonly allLodsReady: boolean;
  readonly bytes: ByteSummary;
}

interface FrameState {
  samples: number[][];
  droppedSamples: number;
}

interface BenchmarkActionContext {
  readonly capture: () => Promise<readonly number[]>;
  readonly markMeasurementStart: () => number;
}

function argumentValue(arguments_: readonly string[], name: string, fallback: string): string {
  const prefix = `--${name}=`;
  const value = arguments_.find((argument) => argument.startsWith(prefix))?.slice(prefix.length);
  return value ?? fallback;
}

function positiveInteger(value: string, name: string): number {
  const parsed = Number.parseInt(value, 10);
  if (!Number.isSafeInteger(parsed) || parsed <= 0) throw new Error(`${name} must be positive`);
  return parsed;
}

function positiveNumber(value: string, name: string): number {
  const parsed = Number.parseFloat(value);
  if (!Number.isFinite(parsed) || parsed <= 0) throw new Error(`${name} must be positive`);
  return parsed;
}

function requiredTomlString(contents: string, key: string): string {
  const match = contents.match(new RegExp(`^${key}\\s*=\\s*"([^"]+)"\\s*$`, "mu"));
  if (match === null) throw new Error(`missing string ${key} in world-service config`);
  const value = match[1];
  if (value === undefined) throw new Error(`string ${key} had no capture`);
  return value;
}

function sleep(milliseconds: number): Promise<void> {
  return new Promise<void>((resolve) => setTimeout(resolve, milliseconds));
}

function percentile(values: readonly number[], fraction: number): number | null {
  if (values.length === 0) return null;
  const sorted = [...values].sort((left, right) => left - right);
  return sorted[Math.min(sorted.length - 1, Math.ceil(sorted.length * fraction) - 1)] ?? null;
}

function rounded(value: number, digits = 1): number {
  const scale = 10 ** digits;
  return Math.round(value * scale) / scale;
}

function numericSummary(values: readonly number[]): NumberSummary {
  if (values.length === 0) return { median: null, p95: null, max: null };
  return {
    median: rounded(percentile(values, 0.5) ?? 0),
    p95: rounded(percentile(values, 0.95) ?? 0),
    max: rounded(Math.max(...values)),
  };
}

function frameTimingSummary(samples: readonly number[][], droppedSamples: number) {
  const column = (index: number): number[] =>
    samples.flatMap((sample) => {
      const value = sample[index];
      return value === undefined ? [] : [value];
    });
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

function byteSummary(stats: LinkStats): ByteSummary {
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

function viewportSignature(snapshot: readonly number[]): string {
  return [
    snapshot[SNAPSHOT.viewportFingerprintLow24],
    snapshot[SNAPSHOT.viewportFingerprintHigh24],
    snapshot[SNAPSHOT.visibleChunks],
    snapshot[SNAPSHOT.quads],
    snapshot[SNAPSHOT.waterQuads],
  ].join(":");
}

function sameSignature(
  left: ViewportSample | undefined,
  right: ViewportSample | undefined,
): boolean {
  if (left === undefined || right === undefined) return false;
  return left.signature === right.signature;
}

function firstPermanentlyFinalSample(
  samples: readonly ViewportSample[],
  actionFinishedMs: number,
): ViewportSample | null {
  const final = samples.at(-1);
  if (!final) return null;
  for (let index = 0; index < samples.length; index += 1) {
    const sample = samples[index];
    if (sample === undefined) continue;
    if (sample.elapsedMs < actionFinishedMs || !sameSignature(sample, final)) continue;
    if (samples.slice(index).every((candidate) => sameSignature(candidate, final))) return sample;
  }
  return final;
}

async function waitForSnapshotApi(page: Page): Promise<void> {
  await page.waitForFunction(() => typeof globalThis.__VOXELS__?.snapshot === "function", null, {
    timeout: 30_000,
  });
}

async function captureSnapshot(page: Page, frameState: FrameState): Promise<readonly number[]> {
  const current = assertSnapshotSchema(
    await page.evaluate(() => globalThis.__VOXELS__!.snapshot()),
  );
  const count = snapshotValue(current, "sampleCount");
  for (let index = 0; index < count; index += 1) {
    const start = FRAME_SAMPLE_START + index * FRAME_SAMPLE_WIDTH;
    frameState.samples.push(current.slice(start, start + FRAME_SAMPLE_WIDTH));
  }
  frameState.droppedSamples += snapshotValue(current, "droppedSamples");
  return current;
}

function observePage(page: Page, label: string, errors: string[]): void {
  page.on("pageerror", (error) => errors.push(`${label} pageerror: ${error.message}`));
  page.on("console", (message) => {
    if (isBrowserConsoleFailure(message.type(), message.text(), FAILURE)) {
      errors.push(`${label} ${message.type()}: ${message.text()}`);
    }
  });
}

async function turn(
  page: Page,
  radians: number,
  { capture, markMeasurementStart }: BenchmarkActionContext,
) {
  const before = await capture();
  markMeasurementStart();
  await page.evaluate((movementX) => {
    globalThis.__VOXELS__!.look(movementX, 0);
  }, radians / 0.0022);
  await page.waitForTimeout(80);
  const after = await capture();
  const delta = Math.abs(
    Math.atan2(
      Math.sin(snapshotValue(after, "yaw") - snapshotValue(before, "yaw")),
      Math.cos(snapshotValue(after, "yaw") - snapshotValue(before, "yaw")),
    ),
  );
  if (delta < Math.abs(radians) * 0.8) {
    throw new Error(`turn fixture rotated only ${delta.toFixed(3)} radians`);
  }
  return { yawDeltaRadians: delta };
}

async function walkDistance(
  page: Page,
  targetDistanceMetres: number,
  {
    capture,
    sprint = false,
    key = "KeyW",
    timeoutMs = 15_000,
  }: BenchmarkActionContext & {
    readonly sprint?: boolean;
    readonly key?: string;
    readonly timeoutMs?: number;
  },
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
        snapshotValue(after, "cameraX") - snapshotValue(before, "cameraX"),
        snapshotValue(after, "cameraZ") - snapshotValue(before, "cameraZ"),
      );
      if (distance >= targetDistanceMetres) break;
    }
  } finally {
    await page.keyboard.up(key);
    if (sprint) await page.keyboard.up("ShiftLeft");
  }
  const distanceMetres = Math.hypot(
    snapshotValue(after, "cameraX") - snapshotValue(before, "cameraX"),
    snapshotValue(after, "cameraZ") - snapshotValue(before, "cameraZ"),
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

async function runScenario({
  name,
  page,
  link,
  action,
  errors,
  timeoutMs = SCENARIO_TIMEOUT_MS,
}: {
  readonly name: string;
  readonly page: Page;
  readonly link: ShapedLink;
  readonly action: (context: BenchmarkActionContext) => unknown;
  readonly errors: readonly string[];
  readonly timeoutMs?: number;
}) {
  link.reset();
  const started = performance.now();
  const frameState: FrameState = { samples: [], droppedSamples: 0 };
  let actionResult: unknown;
  let actionFinishedMs: number | null = null;
  let markedMeasurementStartedMs: number | null = null;
  let actionError: unknown;
  const actionContext: BenchmarkActionContext = {
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
  const samples: ViewportSample[] = [];
  let consecutiveReady = 0;
  while (performance.now() - started < timeoutMs) {
    if (actionError) throw actionError;
    await sleep(SAMPLE_INTERVAL_MS);
    let current: readonly number[];
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
      visibleChunks: snapshotValue(current, "visibleChunks"),
      quads: snapshotValue(current, "quads"),
      pendingJobs: snapshotValue(current, "pendingJobs"),
      surfaceInFlight: snapshotValue(current, "surfaceInFlight"),
      interactiveLodsReady: snapshotValue(current, "interactiveLodsReady") === 1,
      stride32Tiles: snapshotValue(current, "stride32Tiles"),
      stride64Tiles: snapshotValue(current, "stride64Tiles"),
      allLodsReady: snapshotValue(current, "allLodsReady") === 1,
      bytes: wire,
    });
    const measurementStartedMs = markedMeasurementStartedMs ?? actionFinishedMs;
    const ready =
      measurementStartedMs !== null &&
      snapshotValue(current, "allLodsReady") === 1 &&
      snapshotValue(current, "visibleChunks") > 0 &&
      snapshotValue(current, "quads") > 0;
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
  if (measurementStartedMs === null) throw new Error(`${name} action never started measurement`);
  const final = samples.at(-1);
  const fullCoverage = samples.at(-READY_STABLE_SAMPLES);
  const informed = firstPermanentlyFinalSample(samples, measurementStartedMs);
  const interactive = samples.find(
    (sample) => sample.elapsedMs >= measurementStartedMs && sample.interactiveLodsReady,
  );
  const firstUseful = samples.find((sample) => sample.visibleChunks > 0 && sample.quads > 0);
  if (
    final === undefined ||
    fullCoverage === undefined ||
    informed === null ||
    interactive === undefined ||
    actionFinishedMs === null
  ) {
    throw new Error(`${name} completed without the required stable samples`);
  }
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
    linkPressure: {
      upstream: {
        peakQueuedBytes: stats.upstream.peakQueuedBytes,
        peakQueueDelayMs: rounded(stats.upstream.peakQueueDelayMs, 3),
        backpressurePauses: stats.upstream.backpressurePauses,
      },
      downstream: {
        peakQueuedBytes: stats.downstream.peakQueuedBytes,
        peakQueueDelayMs: rounded(stats.downstream.peakQueueDelayMs, 3),
        backpressurePauses: stats.downstream.backpressurePauses,
      },
    },
    messages: stats.messages,
    frameTiming: frameTimingSummary(frameState.samples, frameState.droppedSamples),
    sampleCount: samples.length,
  };
}

async function waitForPort(
  port: number,
  child: ChildProcess,
  logs: readonly string[],
): Promise<void> {
  const deadline = performance.now() + 90_000;
  while (performance.now() < deadline) {
    if (child.exitCode !== null) {
      throw new Error(`world service exited with ${child.exitCode}\n${logs.join("")}`);
    }
    const connected = await new Promise<boolean>((resolve) => {
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

function stopChild(child: ChildProcess | undefined): Promise<unknown> {
  if (!child || child.exitCode !== null) return Promise.resolve();
  child.kill("SIGTERM");
  return Promise.race([
    new Promise<void>((resolve) => {
      child.once("exit", () => resolve());
    }),
    sleep(3_000).then(() => {
      if (child.exitCode === null) child.kill("SIGKILL");
    }),
  ]);
}

type NetworkRun = Awaited<ReturnType<typeof runScenario>>;

function aggregateRuns(runs: readonly NetworkRun[]) {
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
      const downstreamQueueDelay = scenarios.map(
        (scenario) => scenario.linkPressure.downstream.peakQueueDelayMs,
      );
      const downstreamQueuedBytes = scenarios.map(
        (scenario) => scenario.linkPressure.downstream.peakQueuedBytes,
      );
      return [
        name,
        {
          runs: scenarios.length,
          viewportFullyInformedMs: {
            median: rounded(percentile(viewport, 0.5) ?? 0),
            p95: rounded(percentile(viewport, 0.95) ?? 0),
            min: rounded(Math.min(...viewport)),
            max: rounded(Math.max(...viewport)),
          },
          interactiveCoverageReadyMs: {
            median: rounded(percentile(interactive, 0.5) ?? 0),
            p95: rounded(percentile(interactive, 0.95) ?? 0),
            max: rounded(Math.max(...interactive)),
          },
          fullCoverageSettledMs: {
            median: rounded(percentile(coverage, 0.5) ?? 0),
            p95: rounded(percentile(coverage, 0.95) ?? 0),
            max: rounded(Math.max(...coverage)),
          },
          bytesAtViewportInformed: {
            medianTotal: percentile(viewportBytes, 0.5) ?? 0,
            medianWorldDownstream: percentile(viewportWorldBytes, 0.5) ?? 0,
          },
          bytesAtInteractiveCoverage: {
            medianWorldDownstream: percentile(interactiveWorldBytes, 0.5) ?? 0,
          },
          bytesAtFullCoverage: {
            medianTotal: percentile(coverageBytes, 0.5) ?? 0,
            medianWorldDownstream: percentile(coverageWorldBytes, 0.5) ?? 0,
          },
          frameTiming: {
            medianP95Ms: rounded(percentile(frameP95, 0.5) ?? 0),
            maxMs: rounded(Math.max(...frameMax)),
            streamingMedianP95Ms: rounded(percentile(streamingP95, 0.5) ?? 0),
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
          linkPressure: {
            downstreamPeakQueueDelayMs: {
              median: rounded(percentile(downstreamQueueDelay, 0.5) ?? 0, 3),
              max: rounded(Math.max(...downstreamQueueDelay), 3),
            },
            downstreamPeakQueuedBytes: {
              median: percentile(downstreamQueuedBytes, 0.5) ?? 0,
              max: Math.max(...downstreamQueuedBytes),
            },
            downstreamBackpressurePauses: scenarios.reduce(
              (total, scenario) => total + scenario.linkPressure.downstream.backpressurePauses,
              0,
            ),
          },
        },
      ];
    }),
  );
}

type NetworkSummary = ReturnType<typeof aggregateRuns>;

interface NetworkMarkdownReport {
  readonly generatedAt: string;
  readonly git: { readonly commit: string; readonly dirty: boolean };
  readonly world: { readonly source: string };
  readonly repetitions: number;
  readonly environment: {
    readonly cpu: string;
    readonly platform: string;
    readonly chrome: string;
    readonly node: string;
  };
  readonly link: {
    readonly roundTripLatencyMs: number;
    readonly downstreamMegabitsPerSecond: number;
    readonly upstreamMegabitsPerSecond: number;
  };
  readonly summary: NetworkSummary;
}

function markdownReport(result: NetworkMarkdownReport): string {
  const rows = Object.entries(result.summary).map(([name, summary]) => {
    const viewport = summary.viewportFullyInformedMs;
    return `| ${name} | ${summary.interactiveCoverageReadyMs.median.toFixed(1)} | ${viewport.median.toFixed(1)} | ${viewport.max.toFixed(1)} | ${summary.fullCoverageSettledMs.median.toFixed(1)} | ${summary.bytesAtInteractiveCoverage.medianWorldDownstream.toLocaleString("en-US")} | ${summary.bytesAtViewportInformed.medianWorldDownstream.toLocaleString("en-US")} | ${summary.bytesAtFullCoverage.medianTotal.toLocaleString("en-US")} |`;
  });
  const frameRows = Object.entries(result.summary).map(([name, summary]) => {
    const frame = summary.frameTiming;
    return `| ${name} | ${frame.medianP95Ms.toFixed(1)} | ${frame.maxMs.toFixed(1)} | ${frame.above33_33ms.toLocaleString("en-US")} / ${frame.samples.toLocaleString("en-US")} | ${frame.streamingMedianP95Ms.toFixed(1)} | ${frame.droppedSamples.toLocaleString("en-US")} |`;
  });
  const pressureRows = Object.entries(result.summary).map(([name, summary]) => {
    const pressure = summary.linkPressure;
    return `| ${name} | ${pressure.downstreamPeakQueueDelayMs.median.toFixed(3)} / ${pressure.downstreamPeakQueueDelayMs.max.toFixed(3)} | ${pressure.downstreamPeakQueuedBytes.median.toLocaleString("en-US")} / ${pressure.downstreamPeakQueuedBytes.max.toLocaleString("en-US")} | ${pressure.downstreamBackpressurePauses.toLocaleString("en-US")} |`;
  });
  return `# Remote world streaming benchmark\n\nGenerated ${result.generatedAt} at commit \`${result.git.commit}\`${result.git.dirty ? " (dirty)" : ""}.\n\nWorld source: \`${result.world.source}\`; repetitions: ${result.repetitions}; environment: ${result.environment.cpu}, ${result.environment.platform}, Chrome ${result.environment.chrome}, Node ${result.environment.node}.\n\nLink profile: ${result.link.roundTripLatencyMs} ms RTT, ${result.link.downstreamMegabitsPerSecond} Mbit/s down, ${result.link.upstreamMegabitsPerSecond} Mbit/s up, no jitter or loss. Both WebSockets share one bandwidth clock per direction. Counts are TCP stream bytes delivered by the user-space proxy; they include HTTP/WebSocket framing but exclude TCP/IP/TLS overhead.\n\n| Scenario | Interactive ready median (ms) | Viewport informed median (ms) | max (ms) | Full coverage median (ms) | World bytes at interactive | World bytes at viewport | Total bytes at full coverage |\n| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |\n${rows.join("\n")}\n\n“Interactive ready” is when canonical terrain and the original four surface rings are complete; kilometre horizon prefetch starts only afterward. “Viewport informed” is the earliest post-action sample whose presented-geometry fingerprint equals the final fully settled viewport and stays equal. “Full coverage” is the first of three consecutive matching samples where every canonical and surface LOD queue and in-flight stage is settled. Turn timing starts when look input is issued; walking covers a fixed distance.\n\n## Link pressure\n\n| Scenario | Downstream peak queue delay median/max (ms) | Downstream peak queued median/max (bytes) | Source pauses |\n| --- | ---: | ---: | ---: |\n${pressureRows.join("\n")}\n\nQueue delay excludes configured propagation and each pacing quantum's own serialization. A source pause means the proxy's bounded queue applied TCP backpressure.\n\n## Main-thread frame timing\n\n| Scenario | Median run p95 (ms) | Worst frame (ms) | Frames >33.33 ms | Streaming p95 (ms) | Dropped samples |\n| --- | ---: | ---: | ---: | ---: | ---: |\n${frameRows.join("\n")}\n`;
}

async function main(context: ScenarioContext, arguments_: readonly string[]) {
  const value = (name: string, fallback: string): string =>
    argumentValue(arguments_, name, fallback);
  const repetitions = positiveInteger(value("runs", "5"), "runs");
  const roundTripLatencyMs = positiveNumber(
    value("rtt-ms", String(DEFAULT_PROFILE.roundTripLatencyMs)),
    "rtt-ms",
  );
  const profile = {
    ...DEFAULT_PROFILE,
    name: value("profile", DEFAULT_PROFILE.name),
    roundTripLatencyMs,
    oneWayLatencyMs: roundTripLatencyMs / 2,
    downstreamMegabitsPerSecond: positiveNumber(
      value("downstream-mbps", String(DEFAULT_PROFILE.downstreamMegabitsPerSecond)),
      "downstream-mbps",
    ),
    upstreamMegabitsPerSecond: positiveNumber(
      value("upstream-mbps", String(DEFAULT_PROFILE.upstreamMegabitsPerSecond)),
      "upstream-mbps",
    ),
  };
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
  const worldSource = requiredTomlString(serviceSource, "source");
  await writeFile(
    serviceConfigPath,
    serviceSource
      .replace(/^listen = .*$/m, `listen = "127.0.0.1:${backendPort}"`)
      .replace(
        /^allowed_origins = .*$/m,
        `allowed_origins = ["http://${PREVIEW_HOST}:${previewPort}"]`,
      )
      .replace(/^database = .*$/m, 'database = "world-state.sqlite3"'),
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
  const previousClientConfig = process.env.VOXELS_CLIENT_CONFIG_PATH;
  process.env.VOXELS_CLIENT_CONFIG_PATH = clientConfigPath;

  let browser: Browser | undefined;
  let previewServer: Awaited<ReturnType<(typeof import("vite-plus"))["preview"]>> | undefined;
  let worldService: ChildProcess | undefined;
  let link: ShapedLink | undefined;
  const worldLogs: string[] = [];
  try {
    const { build, preview } = await import("vite-plus");
    await build({ mode: "production" });
    // Keep readiness about daemon startup rather than compilation. In particular, a feature-set
    // change can otherwise outlive the readiness deadline and leave no benchmark result.
    execFileSync(rustTool("cargo"), worldServiceBuildCargoArgs({ metal: true }), {
      stdio: "inherit",
    });
    worldService = spawn(
      rustTool("cargo"),
      worldServiceCargoArgs({ metal: true, configPath: serviceConfigPath }),
      { stdio: ["ignore", "pipe", "pipe"] },
    );
    for (const stream of [worldService.stdout, worldService.stderr]) {
      if (stream === null) throw new Error("world service did not expose output pipes");
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
    const runs: NetworkRun[] = [];
    for (let repetition = 0; repetition < repetitions; repetition += 1) {
      const errors: string[] = [];
      const browserContext = await browser.newContext({ viewport: VIEWPORT });
      const page = await browserContext.newPage();
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
      await browserContext.close();

      const pivotErrors: string[] = [];
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
                snapshotValue(current, "visibleChunks") > 0 &&
                snapshotValue(current, "quads") > 0 &&
                snapshotValue(current, "allLodsReady") === 0
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
      world: { source: worldSource },
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
    const report = markdownReport(result);
    await Promise.all([
      context.artifacts.writeJson("network benchmark", "report.json", result),
      context.artifacts.writeText("network benchmark", "report.md", report, "text/markdown"),
    ]);
    return {
      summary: `Completed ${runs.length} network benchmark runs.`,
      metrics: result.summary,
      details: result,
    };
  } catch (error) {
    const reason = error instanceof Error ? error.stack : String(error);
    const serviceLog = worldLogs.join("").trim();
    throw new Error(
      `${reason}\n\nNative world-service output:\n${serviceLog || "(no output captured)"}`,
      { cause: error },
    );
  } finally {
    await browser?.close();
    await previewServer?.close();
    await link?.close();
    await stopChild(worldService);
    if (previousClientConfig === undefined) delete process.env.VOXELS_CLIENT_CONFIG_PATH;
    else process.env.VOXELS_CLIENT_CONFIG_PATH = previousClientConfig;
    await rm(temporary, { recursive: true, force: true });
  }
}

export default defineScenario({
  id: "network-benchmark",
  kind: "benchmark",
  summary: "Measures prioritized world streaming through a shaped bidirectional remote link.",
  uses: {
    world: true,
    browser: true,
    viewport: "browser",
    network: true,
    metrics: true,
    rust: true,
  },
  timeoutMs: 1_800_000,
  run: main,
});
