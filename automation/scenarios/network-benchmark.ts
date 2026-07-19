import { execFileSync } from "node:child_process";
import { cpus, platform, release } from "node:os";
import type { Page } from "playwright";
import { BrowserCapability, type BrowserFailure, reserveEphemeralPort } from "../lib/browser.ts";
import { ScenarioArguments } from "../lib/arguments.ts";
import {
  type EngineClient,
  FRAME_SAMPLE_WIDTH,
  SNAPSHOT,
  SNAPSHOT_SCHEMA_VERSION,
  snapshotValue,
} from "../lib/engine.ts";
import { createShapedLink, type LinkStats, type ShapedLink } from "../lib/network.ts";
import { percentileOrNull as percentile, rounded } from "../lib/metrics.ts";
import { PRESENCE_PATH, VXWP_VERSION, WORLD_PATH } from "../lib/protocol.ts";
import { defineScenario, type ScenarioContext } from "../lib/scenario.ts";
import {
  prepareWorldFixture,
  routeWorldClient,
  startWebPreview,
  startWorldService,
} from "../lib/world.ts";
import type { WorldSource } from "../lib/world.ts";

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

function sleep(milliseconds: number): Promise<void> {
  return new Promise<void>((resolve) => setTimeout(resolve, milliseconds));
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

async function waitForSnapshotApi(engine: EngineClient): Promise<void> {
  await engine.ready();
}

async function captureSnapshot(
  engine: EngineClient,
  frameState: FrameState,
): Promise<readonly number[]> {
  const current = await engine.snapshot();
  const count = snapshotValue(current, "sampleCount");
  for (let index = 0; index < count; index += 1) {
    const start = FRAME_SAMPLE_START + index * FRAME_SAMPLE_WIDTH;
    frameState.samples.push(current.slice(start, start + FRAME_SAMPLE_WIDTH));
  }
  frameState.droppedSamples += snapshotValue(current, "droppedSamples");
  return current;
}

async function turn(
  engine: EngineClient,
  radians: number,
  { capture, markMeasurementStart }: BenchmarkActionContext,
) {
  const before = await capture();
  markMeasurementStart();
  await engine.look(radians / 0.0022, 0);
  await engine.wait(80);
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
  engine,
  link,
  action,
  errors,
  timeoutMs = SCENARIO_TIMEOUT_MS,
}: {
  readonly name: string;
  readonly engine: EngineClient;
  readonly link: ShapedLink;
  readonly action: (context: BenchmarkActionContext) => unknown;
  readonly errors: readonly BrowserFailure[];
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
    capture: () => captureSnapshot(engine, frameState),
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
      current = await captureSnapshot(engine, frameState);
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
  if (errors.length > 0) {
    throw new Error(
      errors.map((error) => `${error.page} ${error.source}: ${error.message}`).join("\n"),
    );
  }
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
  const options = new ScenarioArguments(arguments_);
  const repetitions =
    options.number("runs", { fallback: 5, integer: true, minimum: 1, maximum: 100 }) ?? 5;
  const roundTripLatencyMs =
    options.number("rtt-ms", {
      fallback: DEFAULT_PROFILE.roundTripLatencyMs,
      minimum: 0,
      maximum: 10_000,
    }) ?? DEFAULT_PROFILE.roundTripLatencyMs;
  const profile = {
    ...DEFAULT_PROFILE,
    name: options.string("profile", DEFAULT_PROFILE.name) ?? DEFAULT_PROFILE.name,
    roundTripLatencyMs,
    oneWayLatencyMs: roundTripLatencyMs / 2,
    downstreamMegabitsPerSecond:
      options.number("downstream-mbps", {
        fallback: DEFAULT_PROFILE.downstreamMegabitsPerSecond,
        minimum: 0.001,
      }) ?? DEFAULT_PROFILE.downstreamMegabitsPerSecond,
    upstreamMegabitsPerSecond:
      options.number("upstream-mbps", {
        fallback: DEFAULT_PROFILE.upstreamMegabitsPerSecond,
        minimum: 0.001,
      }) ?? DEFAULT_PROFILE.upstreamMegabitsPerSecond,
  };
  const worldSource = options.choice(
    "source",
    ["procedural-v16", "terrain-diffusion-30m"] as const satisfies readonly WorldSource[],
    "terrain-diffusion-30m",
  );
  options.assertEmpty();
  const proxyPort = await reserveEphemeralPort();
  const previewPort = await reserveEphemeralPort();
  const fixture = await prepareWorldFixture({
    originPort: previewPort,
    clientPorts: [proxyPort],
    prefix: "voxels-network-benchmark-",
    source: worldSource,
  });
  context.defer("network benchmark fixture", () => fixture.cleanup());
  await startWebPreview(context, {
    port: previewPort,
    buildProfile: "release",
  });
  const service = await startWorldService(context, fixture, {
    metal: worldSource === "terrain-diffusion-30m",
  });
  const link = await createShapedLink({
    listenPort: proxyPort,
    targetPort: fixture.backendPort,
    profile,
  });
  context.defer("network benchmark shaped link", () => link.close());
  const clientRoute = routeWorldClient(fixture, 0);
  const browser = await BrowserCapability.start(context, { warningPattern: FAILURE });
  try {
    const runs: NetworkRun[] = [];
    for (let repetition = 0; repetition < repetitions; repetition += 1) {
      const viewport = await browser.open({
        url: "about:blank",
        label: `run-${repetition + 1}`,
        viewport: VIEWPORT,
        engine: false,
        ...clientRoute,
      });
      const page = viewport.page;
      const { engine } = viewport;
      const url = `http://${PREVIEW_HOST}:${previewPort}/?player=network-bench-${repetition + 1}`;
      runs.push(
        await runScenario({
          name: "cold_spawn",
          engine,
          link,
          errors: viewport.failures,
          action: async () => {
            await page.goto(url, { waitUntil: "domcontentloaded" });
            await waitForSnapshotApi(engine);
          },
        }),
      );
      runs.push(
        await runScenario({
          name: "resident_walk",
          engine,
          link,
          errors: viewport.failures,
          // Spawn faces -Z from the exact chunk boundary. Walking backward stays inside the
          // already-resident spawn chunk and is the control for unexpected world traffic.
          action: (context) =>
            walkDistance(page, RESIDENT_WALK_METRES, { ...context, key: "KeyS" }),
        }),
      );
      runs.push(
        await runScenario({
          name: "cached_turn_180",
          engine,
          link,
          errors: viewport.failures,
          action: (context) => turn(engine, TURN_RADIANS, context),
        }),
      );
      runs.push(
        await runScenario({
          name: "streaming_walk",
          engine,
          link,
          errors: viewport.failures,
          action: (context) =>
            walkDistance(page, STREAMING_WALK_METRES, { ...context, sprint: true }),
        }),
      );
      await viewport.close();

      const pivotViewport = await browser.open({
        url: "about:blank",
        label: `pivot-${repetition + 1}`,
        viewport: VIEWPORT,
        engine: false,
        ...clientRoute,
      });
      const pivotPage = pivotViewport.page;
      const pivotEngine = pivotViewport.engine;
      runs.push(
        await runScenario({
          name: "turn_during_spawn",
          engine: pivotEngine,
          link,
          errors: pivotViewport.failures,
          action: async (context) => {
            await pivotPage.goto(
              `http://${PREVIEW_HOST}:${previewPort}/?player=network-pivot-${repetition + 1}`,
              { waitUntil: "domcontentloaded" },
            );
            await waitForSnapshotApi(pivotEngine);
            const deadline = performance.now() + 30_000;
            while (performance.now() < deadline) {
              const current = await context.capture();
              if (
                snapshotValue(current, "visibleChunks") > 0 &&
                snapshotValue(current, "quads") > 0 &&
                snapshotValue(current, "allLodsReady") === 0
              ) {
                return turn(pivotEngine, TURN_RADIANS, context);
              }
              await pivotPage.waitForTimeout(SAMPLE_INTERVAL_MS);
            }
            throw new Error("turn-during-spawn fixture did not observe partial coverage");
          },
        }),
      );
      await pivotViewport.close();
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
        chrome: browser.version,
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
    const serviceLog = service.logs.join("").trim();
    throw new Error(
      `${reason}\n\nNative world-service output:\n${serviceLog || "(no output captured)"}`,
      { cause: error },
    );
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
