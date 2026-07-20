import { execFileSync } from "node:child_process";
import { cpus, platform, release } from "node:os";
import type { Page } from "playwright";
import { BrowserCapability, type BrowserFailure, reserveEphemeralPort } from "../lib/browser.ts";
import { ScenarioArguments } from "../lib/arguments.ts";
import { type EngineClient, SNAPSHOT_SCHEMA_VERSION, snapshotValue } from "../lib/engine.ts";
import { frameSamples, type FrameSample } from "../lib/render-metrics.ts";
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

const RESULT_SCHEMA_VERSION = 7;
const FIXTURE_VERSION = 3;
const PREVIEW_HOST = "127.0.0.1";
const VIEWPORT = { width: 1280, height: 720 };
const SAMPLE_INTERVAL_MS = 16;
const READY_STABLE_SAMPLES = 3;
const SCENARIO_TIMEOUT_MS = 90_000;
const RESIDENT_WALK_METRES = 2;
const STREAMING_WALK_METRES = 35;
const MOVEMENT_PROGRESS_EPSILON_METRES = 0.025;
const MAX_MOVEMENT_NO_PROGRESS_MS = 150;
const MAX_STREAMING_POSE_INTERVAL_MS = 50;
const MAX_STREAMING_UPSTREAM_QUEUE_DELAY_MS = 10;
const MAX_STREAMING_PROXY_RTT_OVER_LINK_MS = 50;
const MAX_STREAMING_RTT_OVER_LINK_MS = 100;
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
  samples: FrameSample[];
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

function frameTimingSummary(samples: readonly FrameSample[], droppedSamples: number) {
  const frameMs = samples.map((sample) => sample.intervalMs);
  return {
    samples: samples.length,
    droppedSamples,
    frameMs: {
      ...numericSummary(frameMs),
      above33_33ms: frameMs.filter((value) => value > 33.33).length,
    },
    cpuMs: numericSummary(samples.map((sample) => sample.cpuMs)),
    streamingMs: numericSummary(samples.map((sample) => sample.streamingMs)),
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

function worldProductPriorityFrames(stats: LinkStats): Record<string, number> {
  const totals: Record<string, number> = {};
  for (const message of Object.values(stats.messages)) {
    for (const [priority, frames] of Object.entries(message.worldProductPriorityFrames ?? {})) {
      totals[priority] = (totals[priority] ?? 0) + frames;
    }
  }
  return totals;
}

function viewportSignature(snapshot: readonly number[]): string {
  return [
    snapshotValue(snapshot, "viewportFingerprintLow24"),
    snapshotValue(snapshot, "viewportFingerprintHigh24"),
    snapshotValue(snapshot, "visibleChunks"),
    snapshotValue(snapshot, "quads"),
    snapshotValue(snapshot, "waterQuads"),
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
  frameState.samples.push(...frameSamples(current));
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
  let previous = before;
  let lastProgressAt = started;
  let longestNoProgressMs = 0;
  try {
    while (performance.now() - started < timeoutMs) {
      await page.waitForTimeout(25);
      after = await capture();
      const observedAt = performance.now();
      const stepDistance = Math.hypot(
        snapshotValue(after, "cameraX") - snapshotValue(previous, "cameraX"),
        snapshotValue(after, "cameraZ") - snapshotValue(previous, "cameraZ"),
      );
      if (stepDistance >= MOVEMENT_PROGRESS_EPSILON_METRES) {
        lastProgressAt = observedAt;
      } else {
        longestNoProgressMs = Math.max(longestNoProgressMs, observedAt - lastProgressAt);
      }
      previous = after;
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
  if (longestNoProgressMs > MAX_MOVEMENT_NO_PROGRESS_MS) {
    throw new Error(
      `walk fixture stopped progressing for ${longestNoProgressMs.toFixed(1)} ms while movement remained held`,
    );
  }
  return {
    requestedDistanceMetres: targetDistanceMetres,
    actionDurationMs: rounded(performance.now() - started),
    distanceMetres: rounded(distanceMetres, 3),
    longestNoProgressMs: rounded(longestNoProgressMs),
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
  const priorities = worldProductPriorityFrames(stats);
  const poseFrames = stats.messages["upstream:player_pose"]?.frames ?? 0;
  const realtime = {
    poseFrames,
    meanPoseIntervalMs: poseFrames === 0 ? null : rounded(fullCoverage.elapsedMs / poseFrames, 3),
    maxObservedRoundTripMs:
      stats.messages["upstream:presence_ping"]?.maxObservedRoundTripMs ?? null,
    proxyRoundTrip: stats.presenceProxyRoundTrip,
    presenceUpstreamPeakQueueDelayMs: rounded(
      stats.paths[PRESENCE_PATH]?.upstream.peakQueueDelayMs ?? 0,
      3,
    ),
    presenceUpstreamBackpressurePauses:
      stats.paths[PRESENCE_PATH]?.upstream.backpressurePauses ?? 0,
    collisionRequestFrames: priorities.collision_critical ?? 0,
    ordinaryRequestFrames: Object.entries(priorities).reduce(
      (total, [priority, frames]) => (priority === "collision_critical" ? total : total + frames),
      0,
    ),
    canceledRequestFrames: stats.messages["upstream:cancel"]?.frames ?? 0,
    worldProductPriorityFrames: priorities,
  };
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
    realtime,
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
      const longestNoProgressMs = scenarios.flatMap((scenario) => {
        if (typeof scenario.action !== "object" || scenario.action === null) return [];
        const value = (scenario.action as Record<string, unknown>).longestNoProgressMs;
        return typeof value === "number" && Number.isFinite(value) ? [value] : [];
      });
      const poseIntervals = scenarios.flatMap((scenario) =>
        scenario.realtime.meanPoseIntervalMs === null ? [] : [scenario.realtime.meanPoseIntervalMs],
      );
      const observedRoundTrips = scenarios.flatMap((scenario) =>
        scenario.realtime.maxObservedRoundTripMs === null
          ? []
          : [scenario.realtime.maxObservedRoundTripMs],
      );
      const proxyRoundTripP95 = scenarios.flatMap((scenario) =>
        scenario.realtime.proxyRoundTrip.p95Ms === null
          ? []
          : [scenario.realtime.proxyRoundTrip.p95Ms],
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
          movement:
            longestNoProgressMs.length > 0
              ? { longestNoProgressMs: numericSummary(longestNoProgressMs) }
              : null,
          realtime: {
            meanPoseIntervalMs: numericSummary(poseIntervals),
            maxObservedRoundTripMs: numericSummary(observedRoundTrips),
            proxyRoundTripP95Ms: numericSummary(proxyRoundTripP95),
            presenceUpstreamPeakQueueDelayMs: numericSummary(
              scenarios.map((scenario) => scenario.realtime.presenceUpstreamPeakQueueDelayMs),
            ),
            presenceUpstreamBackpressurePauses: scenarios.reduce(
              (total, scenario) => total + scenario.realtime.presenceUpstreamBackpressurePauses,
              0,
            ),
            collisionRequestFrames: scenarios.reduce(
              (total, scenario) => total + scenario.realtime.collisionRequestFrames,
              0,
            ),
            ordinaryRequestFrames: scenarios.reduce(
              (total, scenario) => total + scenario.realtime.ordinaryRequestFrames,
              0,
            ),
            canceledRequestFrames: scenarios.reduce(
              (total, scenario) => total + scenario.realtime.canceledRequestFrames,
              0,
            ),
          },
        },
      ];
    }),
  );
}

type NetworkSummary = ReturnType<typeof aggregateRuns>;

function streamingGuardViolations(
  runs: readonly NetworkRun[],
  configuredRoundTripMs: number,
): string[] {
  const controlErrors = runs.flatMap((run) =>
    Object.entries(run.messages).flatMap(([key, message]) =>
      Object.entries(message.controlErrors ?? {}).map(
        ([reason, count]) => `${run.name}: ${key} reported ${count} control errors: ${reason}`,
      ),
    ),
  );
  const streaming = runs
    .filter((run) => run.name === "streaming_walk")
    .flatMap((run, index) => {
      const label = `streaming_walk run ${index + 1}`;
      const violations: string[] = [];
      if (
        run.realtime.meanPoseIntervalMs === null ||
        run.realtime.meanPoseIntervalMs > MAX_STREAMING_POSE_INTERVAL_MS
      ) {
        violations.push(
          `${label}: mean pose interval ${run.realtime.meanPoseIntervalMs ?? "n/a"} ms exceeds ${MAX_STREAMING_POSE_INTERVAL_MS} ms`,
        );
      }
      if (run.realtime.presenceUpstreamPeakQueueDelayMs > MAX_STREAMING_UPSTREAM_QUEUE_DELAY_MS) {
        violations.push(
          `${label}: presence upload queued for ${run.realtime.presenceUpstreamPeakQueueDelayMs} ms`,
        );
      }
      if (run.realtime.presenceUpstreamBackpressurePauses !== 0) {
        violations.push(
          `${label}: presence upload hit ${run.realtime.presenceUpstreamBackpressurePauses} backpressure pauses`,
        );
      }
      if (
        run.realtime.proxyRoundTrip.p95Ms === null ||
        run.realtime.proxyRoundTrip.p95Ms >
          configuredRoundTripMs + MAX_STREAMING_PROXY_RTT_OVER_LINK_MS
      ) {
        violations.push(
          `${label}: proxy-observed p95 RTT ${run.realtime.proxyRoundTrip.p95Ms ?? "n/a"} ms exceeds link RTT plus ${MAX_STREAMING_PROXY_RTT_OVER_LINK_MS} ms`,
        );
      }
      if (
        run.realtime.maxObservedRoundTripMs === null ||
        run.realtime.maxObservedRoundTripMs > configuredRoundTripMs + MAX_STREAMING_RTT_OVER_LINK_MS
      ) {
        violations.push(
          `${label}: observed RTT ${run.realtime.maxObservedRoundTripMs ?? "n/a"} ms exceeds link RTT plus ${MAX_STREAMING_RTT_OVER_LINK_MS} ms`,
        );
      }
      if (run.realtime.collisionRequestFrames === 0) {
        violations.push(`${label}: sent no collision-critical world requests`);
      }
      return violations;
    });
  return [...controlErrors, ...streaming];
}

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
  readonly guards: {
    readonly passed: boolean;
    readonly violations: readonly string[];
  };
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
  const movementRows = Object.entries(result.summary).flatMap(([name, summary]) => {
    const movement = summary.movement;
    if (movement === null) return [];
    const noProgress = movement.longestNoProgressMs;
    return [
      `| ${name} | ${noProgress.median?.toFixed(1) ?? "n/a"} | ${noProgress.max?.toFixed(1) ?? "n/a"} |`,
    ];
  });
  const realtimeRows = Object.entries(result.summary).map(([name, summary]) => {
    const realtime = summary.realtime;
    return `| ${name} | ${realtime.meanPoseIntervalMs.median?.toFixed(1) ?? "n/a"} | ${realtime.proxyRoundTripP95Ms.max?.toFixed(1) ?? "n/a"} | ${realtime.maxObservedRoundTripMs.max?.toFixed(1) ?? "n/a"} | ${realtime.presenceUpstreamPeakQueueDelayMs.max?.toFixed(3) ?? "n/a"} | ${realtime.collisionRequestFrames.toLocaleString("en-US")} / ${realtime.ordinaryRequestFrames.toLocaleString("en-US")} | ${realtime.canceledRequestFrames.toLocaleString("en-US")} |`;
  });
  const guardSummary = result.guards.passed
    ? "Passed."
    : `Failed:\n${result.guards.violations.map((violation) => `- ${violation}`).join("\n")}`;
  return `# Remote world streaming benchmark\n\nGenerated ${result.generatedAt} at commit \`${result.git.commit}\`${result.git.dirty ? " (dirty)" : ""}.\n\nWorld source: \`${result.world.source}\`; repetitions: ${result.repetitions}; environment: ${result.environment.cpu}, ${result.environment.platform}, Chrome ${result.environment.chrome}, Node ${result.environment.node}.\n\nLink profile: ${result.link.roundTripLatencyMs} ms RTT, ${result.link.downstreamMegabitsPerSecond} Mbit/s down, ${result.link.upstreamMegabitsPerSecond} Mbit/s up, no jitter or loss. Both WebSockets share one bandwidth clock per direction. Counts are TCP stream bytes delivered by the user-space proxy; they include HTTP/WebSocket framing but exclude TCP/IP/TLS overhead.\n\nStreaming guards: ${guardSummary}\n\n| Scenario | Interactive ready median (ms) | Viewport informed median (ms) | max (ms) | Full coverage median (ms) | World bytes at interactive | World bytes at viewport | Total bytes at full coverage |\n| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |\n${rows.join("\n")}\n\n“Interactive ready” is when canonical terrain and the original four surface rings are complete; kilometre horizon prefetch starts only afterward. “Viewport informed” is the earliest post-action sample whose presented-geometry fingerprint equals the final fully settled viewport and stays equal. “Full coverage” is the first of three consecutive matching samples where every canonical and surface LOD queue and in-flight stage is settled. Turn timing starts when look input is issued; walking covers a fixed distance.\n\n## Movement continuity\n\n| Scenario | Longest no-progress median (ms) | max (ms) |\n| --- | ---: | ---: |\n${movementRows.join("\n")}\n\nA movement sample counts as progress after at least ${MOVEMENT_PROGRESS_EPSILON_METRES} metres of horizontal displacement. Long no-progress intervals while movement input remains held expose collision stalls caused by missing canonical chunks; the fixed route fails at more than ${MAX_MOVEMENT_NO_PROGRESS_MS} ms.\n\n## Realtime control and collision priority\n\n| Scenario | Mean pose interval (ms) | Proxy p95 RTT (ms) | Client max RTT (ms) | Presence upload queue max (ms) | Collision / ordinary requests | Canceled requests |\n| --- | ---: | ---: | ---: | ---: | ---: | ---: |\n${realtimeRows.join("\n")}\n\nPose traffic uses a dedicated WebSocket and coalesces stale samples instead of queueing them. Proxy RTT measures ping-to-pong across the shaped link and server without client scheduling after delivery. Collision requests identify the current support volume and intended movement corridor; canceled requests show collision-critical work preempting an already-full ordinary request window.\n\n## Link pressure\n\n| Scenario | Downstream peak queue delay median/max (ms) | Downstream peak queued median/max (bytes) | Source pauses |\n| --- | ---: | ---: | ---: |\n${pressureRows.join("\n")}\n\nQueue delay excludes configured propagation and each pacing quantum's own serialization. A source pause means the proxy's bounded queue applied TCP backpressure.\n\n## Main-thread frame timing\n\n| Scenario | Median run p95 (ms) | Worst frame (ms) | Frames >33.33 ms | Streaming p95 (ms) | Dropped samples |\n| --- | ---: | ---: | ---: | ---: | ---: |\n${frameRows.join("\n")}\n`;
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
    const guardViolations = streamingGuardViolations(runs, profile.roundTripLatencyMs);
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
        movementProgressEpsilonMetres: MOVEMENT_PROGRESS_EPSILON_METRES,
        maximumMovementNoProgressMs: MAX_MOVEMENT_NO_PROGRESS_MS,
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
      guards: {
        passed: guardViolations.length === 0,
        violations: guardViolations,
        limits: {
          maximumMeanPoseIntervalMs: MAX_STREAMING_POSE_INTERVAL_MS,
          maximumPresenceUpstreamQueueDelayMs: MAX_STREAMING_UPSTREAM_QUEUE_DELAY_MS,
          maximumProxyRoundTripOverConfiguredLinkMs: MAX_STREAMING_PROXY_RTT_OVER_LINK_MS,
          maximumRoundTripOverConfiguredLinkMs: MAX_STREAMING_RTT_OVER_LINK_MS,
          requireCollisionCriticalRequests: true,
          requireNoPresenceUpstreamBackpressure: true,
          requireNoControlErrors: true,
        },
      },
      summary: aggregateRuns(runs),
      runs,
    };
    const report = markdownReport(result);
    await Promise.all([
      context.artifacts.writeJson("network benchmark", "report.json", result),
      context.artifacts.writeText("network benchmark", "report.md", report, "text/markdown"),
    ]);
    if (!result.guards.passed) {
      throw new Error(`streaming guards failed:\n${guardViolations.join("\n")}`);
    }
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
