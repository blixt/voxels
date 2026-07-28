import { execFileSync } from "node:child_process";
import type { Page } from "playwright";
import { ScenarioArguments } from "../lib/arguments.ts";
import { BrowserCapability, chromeWebGpuLaunchOptions } from "../lib/browser.ts";
import { type EngineClient, SNAPSHOT, snapshotValue } from "../lib/engine.ts";
import { analyzeDiagnosticSky } from "../lib/image.ts";
import { frameSamples, gpuFrameSamples } from "../lib/render-metrics.ts";
import { percentile } from "../lib/metrics.ts";
import { defineScenario, type ScenarioContext } from "../lib/scenario.ts";
import { startWorldStack } from "../lib/world.ts";
import type { WorldSource } from "../lib/world.ts";

const FAILURE =
  /panic|unreachable|runtimeerror|wgpu|webgpu|shader|sqlite|opfs|syncaccesshandle|nomodificationallowed|web lock request failed|no persistence leader|persistence .*failed/i;
// This fixture deliberately fills the view with the worst-case valley mesh. The end-to-end 120 Hz
// and 7.5 ms total-GPU gates remain authoritative; 5 ms still catches a world-pass regression
// without rejecting the established 4.2-4.8 ms cost of this denser correctness scene.
const WORLD_GPU_P95_BUDGET_MS = 5;
type Vector2 = readonly [number, number];
type Vector3 = readonly [number, number, number];
type BoundaryCentres = readonly Vector2[];

interface LodTimings {
  readonly frameIntervals: number[];
  readonly gpu: Map<number, { readonly total: number; readonly world: number }>;
  readonly incompleteTransitionEdges: number[];
}

interface PresentedFrameCapture {
  readonly frames: readonly Buffer[];
  readonly timestamps: readonly (number | null)[];
  readonly observedFrames: number;
  readonly overflowFrames: number;
  readonly firstTimestamp: number | null;
  readonly lastTimestamp: number | null;
}

interface PresentedFrameCaptureControl {
  readonly observedFrames: () => number;
}

interface CentreChangeCapture {
  readonly crossed: readonly number[];
  readonly snapshots: readonly (readonly number[])[];
  readonly observedEngineFrames: number;
  readonly skippedEngineFrames: number;
}

const MAX_PRESENTED_FRAME_CAPTURES = 300;

function required(values: ArrayLike<number>, index: number, label: string): number {
  const value = values[index];
  if (value === undefined) throw new Error(`${label} omitted value ${index}`);
  return value;
}

async function capturePresentedFrames(
  page: Page,
  action: (capture: PresentedFrameCaptureControl) => Promise<void>,
): Promise<PresentedFrameCapture> {
  const session = await page.context().newCDPSession(page);
  const frames: Buffer[] = [];
  const timestamps: Array<number | null> = [];
  const acknowledgements: Promise<void>[] = [];
  let observedFrames = 0;
  let overflowFrames = 0;
  let firstTimestamp: number | null = null;
  let lastTimestamp: number | null = null;
  let acknowledgementError: unknown;
  const onFrame = (event: {
    readonly data: string;
    readonly sessionId: number;
    readonly metadata?: { readonly timestamp?: number };
  }): void => {
    observedFrames += 1;
    const timestamp = event.metadata?.timestamp;
    if (timestamp !== undefined && Number.isFinite(timestamp)) {
      firstTimestamp ??= timestamp;
      lastTimestamp = timestamp;
    }
    if (frames.length < MAX_PRESENTED_FRAME_CAPTURES) {
      frames.push(Buffer.from(event.data, "base64"));
      timestamps.push(timestamp !== undefined && Number.isFinite(timestamp) ? timestamp : null);
    } else {
      overflowFrames += 1;
    }
    acknowledgements.push(
      session
        .send("Page.screencastFrameAck", { sessionId: event.sessionId })
        .then(() => undefined)
        .catch((error: unknown) => {
          acknowledgementError ??= error;
        }),
    );
  };
  session.on("Page.screencastFrame", onFrame);
  try {
    await session.send("Page.startScreencast", {
      format: "png",
      everyNthFrame: 1,
    });
    await action({ observedFrames: () => observedFrames });
    // Let Chromium deliver and acknowledge the final compositor frame before stopping capture.
    await page.waitForTimeout(32);
    await session.send("Page.stopScreencast");
    await Promise.all(acknowledgements);
    if (acknowledgementError !== undefined) throw acknowledgementError;
  } finally {
    session.off("Page.screencastFrame", onFrame);
    await session.detach().catch(() => undefined);
  }
  return {
    frames,
    timestamps,
    observedFrames,
    overflowFrames,
    firstTimestamp,
    lastTimestamp,
  };
}

function boundaryCentres(snapshot: readonly number[]): BoundaryCentres {
  return Array.from(
    { length: 8 },
    (_, index) =>
      [
        required(snapshot, SNAPSHOT.lodBoundary0X + index * 2, "LOD boundary"),
        required(snapshot, SNAPSHOT.lodBoundary0Z + index * 2, "LOD boundary"),
      ] as const,
  );
}

function sameCentres(left: BoundaryCentres, right: BoundaryCentres): boolean {
  return (
    left.length === right.length &&
    left.every((centre, index) => {
      const candidate = right[index];
      return candidate !== undefined && centre[0] === candidate[0] && centre[1] === candidate[1];
    })
  );
}

function cameraPosition(snapshot: readonly number[]): Vector3 {
  return [
    snapshotValue(snapshot, "cameraX"),
    snapshotValue(snapshot, "cameraY"),
    snapshotValue(snapshot, "cameraZ"),
  ];
}

function virtualTerrainState(snapshot: readonly number[]) {
  return {
    mode: snapshotValue(snapshot, "virtualTerrainMode"),
    registeredRegions: snapshotValue(snapshot, "virtualTerrainRegisteredRegions"),
    directoryInFlight: snapshotValue(snapshot, "virtualTerrainDirectoryInFlight"),
    directoryNodes: snapshotValue(snapshot, "virtualTerrainDirectoryNodes"),
    residentPages: snapshotValue(snapshot, "virtualTerrainResidentPages"),
    residentMiB: snapshotValue(snapshot, "virtualTerrainResidentMiB"),
    residentPrimitives: snapshotValue(snapshot, "virtualTerrainResidentPrimitives"),
    selectedPages: snapshotValue(snapshot, "virtualTerrainSelectedPages"),
    requestedPages: snapshotValue(snapshot, "virtualTerrainRequestedPages"),
    ownerlessRoots: snapshotValue(snapshot, "virtualTerrainOwnerlessRoots"),
    gpuMatchesCpuCut: snapshotValue(snapshot, "virtualTerrainGpuMatchesCpuCut") === 1,
    streamPending: snapshotValue(snapshot, "virtualTerrainStreamPending"),
    streamInFlight: snapshotValue(snapshot, "virtualTerrainStreamInFlight"),
    cancellationWasteMiB: snapshotValue(snapshot, "virtualTerrainCancellationWasteMiB"),
    cachePages: snapshotValue(snapshot, "virtualTerrainCachePages"),
    cacheMiB: snapshotValue(snapshot, "virtualTerrainCacheMiB"),
    columns: snapshotValue(snapshot, "virtualTerrainColumns"),
    columnInFlight: snapshotValue(snapshot, "virtualTerrainColumnInFlight"),
    columnRevisionFloors: snapshotValue(snapshot, "virtualTerrainColumnRevisionFloors"),
    columnAccepted: snapshotValue(snapshot, "virtualTerrainColumnAccepted"),
    columnSubmitDeferred: snapshotValue(snapshot, "virtualTerrainColumnSubmitDeferred"),
    columnPreempted: snapshotValue(snapshot, "virtualTerrainColumnPreempted"),
    columnTimedOut: snapshotValue(snapshot, "virtualTerrainColumnTimedOut"),
    columnOtherFailed: snapshotValue(snapshot, "virtualTerrainColumnOtherFailed"),
    directoryAccepted: snapshotValue(snapshot, "virtualTerrainDirectoryAccepted"),
    directorySubmitDeferred: snapshotValue(snapshot, "virtualTerrainDirectorySubmitDeferred"),
    directoryPreempted: snapshotValue(snapshot, "virtualTerrainDirectoryPreempted"),
    directoryTimedOut: snapshotValue(snapshot, "virtualTerrainDirectoryTimedOut"),
    directoryOtherFailed: snapshotValue(snapshot, "virtualTerrainDirectoryOtherFailed"),
  };
}

function planarDistance(left: Vector3, right: Vector3): number {
  return Math.hypot(left[0] - right[0], left[2] - right[2]);
}

function spatialDistance(left: Vector3, right: Vector3): number {
  return Math.hypot(left[0] - right[0], left[1] - right[1], left[2] - right[2]);
}

function summarizePresentedStrides(samples: readonly { readonly presentedStrideVoxels: number }[]) {
  const strides = samples.map((sample) => sample.presentedStrideVoxels);
  const sorted = [...strides].sort((left, right) => left - right);
  const count = strides.length;
  const histogram = [...new Set(sorted)].map((stride) => ({
    stride,
    samples: strides.filter((candidate) => candidate === stride).length,
  }));
  return {
    samples: count,
    histogram,
    canonicalFraction: strides.filter((stride) => stride === 1).length / Math.max(count, 1),
    strideAtMost8Fraction: strides.filter((stride) => stride <= 8).length / Math.max(count, 1),
    stride128PlusFraction: strides.filter((stride) => stride >= 128).length / Math.max(count, 1),
    medianStride: sorted[Math.floor(count * 0.5)] ?? 0,
    p90Stride: sorted[Math.min(Math.floor(count * 0.9), count - 1)] ?? 0,
    meanLodExponent:
      strides.reduce((sum, stride) => sum + Math.log2(Math.max(stride, 1)), 0) / Math.max(count, 1),
  };
}

function summarizeTravelLodQuality(samples: readonly { readonly presentedStrideVoxels: number }[]) {
  const third = Math.ceil(samples.length / 3);
  return {
    overall: summarizePresentedStrides(samples),
    early: summarizePresentedStrides(samples.slice(0, third)),
    middle: summarizePresentedStrides(samples.slice(third, third * 2)),
    late: summarizePresentedStrides(samples.slice(third * 2)),
  };
}

function collectTiming(snapshot: readonly number[], timings: LodTimings): void {
  timings.frameIntervals.push(...frameSamples(snapshot).map((sample) => sample.intervalMs));
  timings.incompleteTransitionEdges.push(snapshotValue(snapshot, "lodIncompleteTransitionEdges"));
  for (const sample of gpuFrameSamples(snapshot).samples) {
    timings.gpu.set(sample.frameId, {
      total: sample.total,
      world: sample.world,
    });
  }
}

function resetTimings(timings: LodTimings): void {
  timings.frameIntervals.length = 0;
  timings.gpu.clear();
  timings.incompleteTransitionEdges.length = 0;
}

async function sampleStablePerformance(
  page: Page,
  engine: EngineClient,
  timings: LodTimings,
  duration: number,
): Promise<void> {
  await readSnapshot(engine, timings);
  resetTimings(timings);
  const deadline = Date.now() + duration;
  while (Date.now() < deadline) {
    await page.waitForTimeout(100);
    await readSnapshot(engine, timings);
  }
}

async function readSnapshot(engine: EngineClient, timings: LodTimings): Promise<readonly number[]> {
  const snapshot = await engine.snapshot();
  collectTiming(snapshot, timings);
  return snapshot;
}

async function waitForEngine(
  engine: EngineClient,
  timings: LodTimings,
): Promise<readonly number[]> {
  return engine.waitForSnapshot(
    (snapshot) =>
      snapshotValue(snapshot, "quads") > 0 &&
      snapshotValue(snapshot, "pendingJobs") === 0 &&
      snapshotValue(snapshot, "surfaceInFlight") === 0 &&
      snapshotValue(snapshot, "allLodsReady") === 1 &&
      snapshotValue(snapshot, "lodTransitionQuads") > 0,
    {
      timeoutMs: 60_000,
      description: "LOD browser fixture did not settle",
      onSnapshot: (snapshot) => collectTiming(snapshot, timings),
    },
  );
}

async function setCameraLook(
  engine: EngineClient,
  targetYaw: number,
  targetPitch: number,
  timings: LodTimings,
): Promise<readonly number[]> {
  return engine.setCameraLook(targetYaw, targetPitch, {
    intervalMs: 10,
    description: "camera look did not reach the requested regression pose",
    onSnapshot: (snapshot) => collectTiming(snapshot, timings),
  });
}

async function waitForCentreChange(
  page: Page,
  engine: EngineClient,
  initialCentres: BoundaryCentres,
  outboundKey: string,
  timings: LodTimings,
  afterCentreChangeFrames: number,
  onCentreChange: () => void,
): Promise<CentreChangeCapture> {
  const deadline = Date.now() + 4_000;
  let previousFrame = snapshotValue(await readSnapshot(engine, timings), "frameSequence");
  let crossed: readonly number[] | undefined;
  let framesAfterCrossing = 0;
  let observedEngineFrames = 0;
  let skippedEngineFrames = 0;
  const snapshots: Array<readonly number[]> = [];
  await page.keyboard.down(outboundKey);
  try {
    while (Date.now() < deadline) {
      const snapshot = await engine.waitForFrameAfter(previousFrame, {
        timeoutMs: 2_000,
        intervalMs: 1,
        description: "renderer stopped while walking toward an LOD snap boundary",
        onSnapshot: (sample) => collectTiming(sample, timings),
      });
      const frame = snapshotValue(snapshot, "frameSequence");
      skippedEngineFrames += Math.max(0, frame - previousFrame - 1);
      previousFrame = frame;
      observedEngineFrames += 1;
      snapshots.push(snapshot);
      if (crossed === undefined && !sameCentres(boundaryCentres(snapshot), initialCentres)) {
        crossed = snapshot;
        onCentreChange();
      } else if (crossed !== undefined) {
        framesAfterCrossing += 1;
      }
      if (crossed !== undefined && framesAfterCrossing >= afterCentreChangeFrames) {
        return {
          crossed,
          snapshots,
          observedEngineFrames,
          skippedEngineFrames,
        };
      }
    }
  } finally {
    await page.keyboard.up(outboundKey);
  }
  if (crossed === undefined) {
    throw new Error("walking forward did not cross an LOD snap boundary");
  }
  throw new Error(
    `renderer produced only ${framesAfterCrossing}/${afterCentreChangeFrames} requested frames after the LOD boundary`,
  );
}

async function waitForStableFrame(
  page: Page,
  engine: EngineClient,
  expectedCentres: BoundaryCentres,
  timings: LodTimings,
): Promise<readonly number[]> {
  let latest: readonly number[] = [];
  let previousPosition: Vector3 | undefined;
  let stable = 0;
  const deadline = Date.now() + 10_000;
  while (stable < 12 && Date.now() < deadline) {
    latest = await readSnapshot(engine, timings);
    const position = cameraPosition(latest);
    const settled =
      sameCentres(boundaryCentres(latest), expectedCentres) &&
      snapshotValue(latest, "grounded") === 1 &&
      previousPosition !== undefined &&
      spatialDistance(position, previousPosition) < 0.0015 &&
      snapshotValue(latest, "pendingJobs") === 0 &&
      snapshotValue(latest, "surfaceInFlight") === 0 &&
      snapshotValue(latest, "allLodsReady") === 1 &&
      snapshotValue(latest, "lodTransitionQuads") > 0;
    stable = settled ? stable + 1 : 0;
    previousPosition = position;
    await page.waitForTimeout(16);
  }
  if (stable < 12) {
    throw new Error(
      `LOD frame did not stabilize at ${JSON.stringify(expectedCentres)}; latest ${JSON.stringify(boundaryCentres(latest))}`,
    );
  }
  return latest;
}

async function waitForStableChangedFrame(
  page: Page,
  engine: EngineClient,
  initialCentres: BoundaryCentres,
  timings: LodTimings,
): Promise<readonly number[]> {
  let latest: readonly number[] = [];
  let latestCentres: BoundaryCentres = [];
  let previousCentres: BoundaryCentres | undefined;
  let previousPosition: Vector3 | undefined;
  let stable = 0;
  const deadline = Date.now() + 10_000;
  while (stable < 12 && Date.now() < deadline) {
    latest = await readSnapshot(engine, timings);
    latestCentres = boundaryCentres(latest);
    const position = cameraPosition(latest);
    const settled =
      !sameCentres(latestCentres, initialCentres) &&
      snapshotValue(latest, "grounded") === 1 &&
      previousPosition !== undefined &&
      spatialDistance(position, previousPosition) < 0.0015 &&
      snapshotValue(latest, "pendingJobs") === 0 &&
      snapshotValue(latest, "surfaceInFlight") === 0 &&
      snapshotValue(latest, "allLodsReady") === 1 &&
      snapshotValue(latest, "lodTransitionQuads") > 0;
    stable =
      settled && previousCentres !== undefined && sameCentres(latestCentres, previousCentres)
        ? stable + 1
        : 0;
    previousCentres = latestCentres;
    previousPosition = position;
    await page.waitForTimeout(16);
  }
  if (stable < 12) {
    throw new Error(
      `changed LOD frame did not stabilize; initial ${JSON.stringify(initialCentres)}, latest ${JSON.stringify(latestCentres)}`,
    );
  }
  return latest;
}

async function compareScreenshots(page: Page, before: Buffer, after: Buffer) {
  return page.evaluate(
    async ({ beforeBase64, afterBase64 }) => {
      const at = (values: ArrayLike<number>, index: number): number => {
        const value = values[index];
        if (value === undefined) throw new Error(`image analysis omitted value ${index}`);
        return value;
      };
      const decode = async (base64: string) => {
        const response = await fetch(`data:image/png;base64,${base64}`);
        const bitmap = await createImageBitmap(await response.blob());
        const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
        const context = canvas.getContext("2d", { willReadFrequently: true });
        if (context === null) throw new Error("LOD comparison canvas is unavailable");
        context.drawImage(bitmap, 0, 0);
        return {
          width: bitmap.width,
          height: bitmap.height,
          pixels: context.getImageData(0, 0, bitmap.width, bitmap.height).data,
        };
      };
      const [left, right] = await Promise.all([decode(beforeBase64), decode(afterBase64)]);
      if (left.width !== right.width || left.height !== right.height) {
        throw new Error("LOD comparison screenshots have different dimensions");
      }
      const roi = {
        x0: Math.floor(left.width * 0.02),
        x1: Math.ceil(left.width * 0.46),
        y0: Math.floor(left.height * 0.28),
        y1: Math.ceil(left.height * 0.58),
      };
      const linear = (value: number): number => {
        const channel = value / 255;
        return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
      };
      const luma = (pixels: Uint8ClampedArray, index: number): number =>
        0.2126 * linear(at(pixels, index)) +
        0.7152 * linear(at(pixels, index + 1)) +
        0.0722 * linear(at(pixels, index + 2));
      let count = 0;
      let sumLeft = 0;
      let sumRight = 0;
      let sumLeftSquared = 0;
      let sumRightSquared = 0;
      let sumProduct = 0;
      let sumAbsolute = 0;
      let catastrophic = 0;
      let leftNearBlackPixels = 0;
      let rightNearBlackPixels = 0;
      // LOD ownership intentionally changes sub-pixel grass and the exact edges of decimated
      // steps. Compare 4x4 linear-light footprints so this gate measures the low-frequency valley
      // lighting regression it was built for; the separate watertight gate owns terrain cracks.
      const footprint = 4;
      for (let y = roi.y0; y < roi.y1; y += footprint) {
        for (let x = roi.x0; x < roi.x1; x += footprint) {
          let leftLuma = 0;
          let rightLuma = 0;
          let footprintPixels = 0;
          for (let offsetY = 0; offsetY < footprint && y + offsetY < roi.y1; offsetY += 1) {
            for (let offsetX = 0; offsetX < footprint && x + offsetX < roi.x1; offsetX += 1) {
              const index = (x + offsetX + (y + offsetY) * left.width) * 4;
              if (
                at(left.pixels, index) <= 2 &&
                at(left.pixels, index + 1) <= 2 &&
                at(left.pixels, index + 2) <= 2
              ) {
                leftNearBlackPixels += 1;
              }
              if (
                at(right.pixels, index) <= 2 &&
                at(right.pixels, index + 1) <= 2 &&
                at(right.pixels, index + 2) <= 2
              ) {
                rightNearBlackPixels += 1;
              }
              leftLuma += luma(left.pixels, index);
              rightLuma += luma(right.pixels, index);
              footprintPixels += 1;
            }
          }
          leftLuma /= footprintPixels;
          rightLuma /= footprintPixels;
          count += 1;
          sumLeft += leftLuma;
          sumRight += rightLuma;
          sumLeftSquared += leftLuma * leftLuma;
          sumRightSquared += rightLuma * rightLuma;
          sumProduct += leftLuma * rightLuma;
          sumAbsolute += Math.abs(leftLuma - rightLuma);
          if (
            Math.max(leftLuma, rightLuma) > 0.03 &&
            Math.min(leftLuma, rightLuma) < Math.max(leftLuma, rightLuma) * 0.5
          ) {
            catastrophic += 1;
          }
        }
      }
      const meanLeft = sumLeft / count;
      const meanRight = sumRight / count;
      const varianceLeft = sumLeftSquared / count - meanLeft * meanLeft;
      const varianceRight = sumRightSquared / count - meanRight * meanRight;
      const covariance = sumProduct / count - meanLeft * meanRight;
      const c1 = 0.01 ** 2;
      const c2 = 0.03 ** 2;
      const pixels = (roi.x1 - roi.x0) * (roi.y1 - roi.y0);
      return {
        roi,
        pixels,
        comparisonSamples: count,
        comparisonFootprintPixels: footprint,
        nearBlackPixelFraction: {
          before: leftNearBlackPixels / pixels,
          after: rightNearBlackPixels / pixels,
        },
        meanLinearLuma: { before: meanLeft, after: meanRight },
        relativeMeanLumaDelta: Math.abs(meanRight - meanLeft) / Math.max(meanLeft, 0.001),
        meanAbsoluteLinearLumaDelta: sumAbsolute / count,
        catastrophicDarkFraction: catastrophic / count,
        ssim:
          ((2 * meanLeft * meanRight + c1) * (2 * covariance + c2)) /
          ((meanLeft * meanLeft + meanRight * meanRight + c1) *
            (varianceLeft + varianceRight + c2)),
      };
    },
    { beforeBase64: before.toString("base64"), afterBase64: after.toString("base64") },
  );
}

async function analyzePresentedFrameContinuity(
  page: Page,
  frames: readonly Buffer[],
  timestamps: readonly (number | null)[],
) {
  if (frames.length < 2 || frames.length !== timestamps.length) {
    throw new Error(
      `LOD continuity requires at least two aligned frames; received ${frames.length} frames and ${timestamps.length} timestamps`,
    );
  }
  const pairs = await page.evaluate(
    async ({ base64Frames, frameTimestamps }) => {
      const required = <T>(values: ArrayLike<T>, index: number): T => {
        const value = values[index];
        if (value === undefined) throw new Error(`LOD continuity omitted value ${index}`);
        return value;
      };
      const decode = async (base64: string) => {
        const response = await fetch(`data:image/png;base64,${base64}`);
        const bitmap = await createImageBitmap(await response.blob());
        const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
        const context = canvas.getContext("2d", { willReadFrequently: true });
        if (context === null) throw new Error("LOD continuity canvas is unavailable");
        context.drawImage(bitmap, 0, 0);
        return {
          width: bitmap.width,
          height: bitmap.height,
          pixels: context.getImageData(0, 0, bitmap.width, bitmap.height).data,
        };
      };
      const linear = (value: number): number => {
        const channel = value / 255;
        return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
      };
      const luma = (pixels: Uint8ClampedArray, index: number): number =>
        0.2126 * linear(required(pixels, index)) +
        0.7152 * linear(required(pixels, index + 1)) +
        0.0722 * linear(required(pixels, index + 2));
      const results: Array<{
        readonly pair: number;
        readonly intervalMs: number | null;
        readonly meanAbsoluteLinearLumaDelta: number;
        readonly normalizedMeanAbsoluteLinearLumaDelta: number;
        readonly relativeMeanLinearLumaDelta: number;
        readonly catastrophicDarkFraction: number;
      }> = [];
      let left = await decode(required(base64Frames, 0));
      for (let pair = 0; pair + 1 < base64Frames.length; pair += 1) {
        const right = await decode(required(base64Frames, pair + 1));
        if (left.width !== right.width || left.height !== right.height) {
          throw new Error("LOD continuity frames have different dimensions");
        }
        const roi = {
          x0: Math.floor(left.width * 0.02),
          x1: Math.ceil(left.width * 0.98),
          y0: Math.floor(left.height * 0.55),
          y1: Math.ceil(left.height * 0.98),
        };
        const footprint = 4;
        let samples = 0;
        let leftSum = 0;
        let rightSum = 0;
        let absoluteSum = 0;
        let catastrophic = 0;
        for (let y = roi.y0; y < roi.y1; y += footprint) {
          for (let x = roi.x0; x < roi.x1; x += footprint) {
            let leftLuma = 0;
            let rightLuma = 0;
            let footprintSamples = 0;
            for (let dy = 0; dy < footprint && y + dy < roi.y1; dy += 1) {
              for (let dx = 0; dx < footprint && x + dx < roi.x1; dx += 1) {
                const pixel = (x + dx + (y + dy) * left.width) * 4;
                leftLuma += luma(left.pixels, pixel);
                rightLuma += luma(right.pixels, pixel);
                footprintSamples += 1;
              }
            }
            leftLuma /= footprintSamples;
            rightLuma /= footprintSamples;
            leftSum += leftLuma;
            rightSum += rightLuma;
            absoluteSum += Math.abs(leftLuma - rightLuma);
            if (
              Math.max(leftLuma, rightLuma) > 0.03 &&
              Math.min(leftLuma, rightLuma) < Math.max(leftLuma, rightLuma) * 0.5
            ) {
              catastrophic += 1;
            }
            samples += 1;
          }
        }
        const leftTimestamp = required(frameTimestamps, pair);
        const rightTimestamp = required(frameTimestamps, pair + 1);
        const intervalMs =
          leftTimestamp === null || rightTimestamp === null
            ? null
            : Math.max(0, (rightTimestamp - leftTimestamp) * 1_000);
        const meanAbsoluteLinearLumaDelta = absoluteSum / samples;
        results.push({
          pair,
          intervalMs,
          meanAbsoluteLinearLumaDelta,
          normalizedMeanAbsoluteLinearLumaDelta:
            intervalMs === null || intervalMs < 1
              ? meanAbsoluteLinearLumaDelta
              : (meanAbsoluteLinearLumaDelta * (1000 / 60)) / intervalMs,
          relativeMeanLinearLumaDelta:
            Math.abs(rightSum - leftSum) / Math.max(leftSum, samples * 0.001),
          catastrophicDarkFraction: catastrophic / samples,
        });
        left = right;
      }
      return results;
    },
    {
      base64Frames: frames.map((frame) => frame.toString("base64")),
      frameTimestamps: timestamps,
    },
  );
  const deltas = pairs.map((pair) => pair.normalizedMeanAbsoluteLinearLumaDelta);
  const catastrophic = pairs.map((pair) => pair.catastrophicDarkFraction);
  const maximum = Math.max(...deltas);
  const maximumPair = pairs.find((pair) => pair.normalizedMeanAbsoluteLinearLumaDelta === maximum);
  const median = percentile(deltas, 0.5);
  return {
    pairs,
    samples: pairs.length,
    normalizedMeanAbsoluteLinearLumaDelta: {
      median,
      p95: percentile(deltas, 0.95),
      maximum,
      maximumOverMedian: maximum / Math.max(median, 0.000_1),
      maximumPair: maximumPair?.pair ?? 0,
    },
    catastrophicDarkFraction: {
      p95: percentile(catastrophic, 0.95),
      maximum: Math.max(...catastrophic),
    },
  };
}

async function analyzeWatertightTerrain(
  page: Page,
  screenshot: Buffer,
  target: "magenta" | "black" = "magenta",
) {
  // This fixed camera points 21 degrees below the horizon. Keep the region below the tree-lined
  // silhouette: magenta pockets between distant trunks are legitimate sky, whereas an enclosed
  // component in this lower ground band is missing terrain coverage.
  return analyzeDiagnosticSky(
    page,
    screenshot,
    {
      x0: 0.02,
      x1: 0.98,
      y0: 0.55,
      y1: 0.98,
    },
    target,
  );
}

function summarizePerformance(timings: LodTimings) {
  const gpu = [...timings.gpu.values()];
  return {
    samples: timings.frameIntervals.length,
    frameP95Ms: percentile(timings.frameIntervals, 0.95),
    frameMaxMs: Math.max(...timings.frameIntervals, 0),
    framesAbove16_67Ms: timings.frameIntervals.filter((value) => value > 16.67).length,
    fractionAbove16_67Ms:
      timings.frameIntervals.filter((value) => value > 16.67).length /
      Math.max(timings.frameIntervals.length, 1),
    gpuSamples: gpu.length,
    worldGpuP95Ms: percentile(
      gpu.map((sample) => sample.world),
      0.95,
    ),
    totalGpuP95Ms: percentile(
      gpu.map((sample) => sample.total),
      0.95,
    ),
    maximumIncompleteTransitionEdges: Math.max(0, ...timings.incompleteTransitionEdges),
  };
}

type LodMode =
  | "transition"
  | "watertight"
  | "boundary-coverage"
  | "travel-coverage"
  | "descent-coverage";

interface LodOptions {
  readonly mode: LodMode;
  readonly source: WorldSource;
  readonly spawn: readonly [number, number];
  readonly look: readonly [number, number];
  readonly pillarHeight: number;
  readonly pillarRadius: number;
  readonly openWorldLab: boolean;
  readonly stepOffPillar: boolean;
  readonly viewport: { readonly width: number; readonly height: number };
  readonly deviceScaleFactor: number;
  readonly cascadedShadows: boolean;
  readonly screenSpaceAmbientOcclusion: boolean;
  readonly recordVideo: boolean;
  readonly geometrySourceTravel: boolean;
  readonly travelSeconds: number;
  readonly buildProfile: "debug" | "wasm-dev" | "release";
  readonly environment: "day-clear" | "night-rain";
}

function parseOptions(arguments_: readonly string[]): LodOptions {
  const argumentsReader = new ScenarioArguments(arguments_);
  const mode = argumentsReader.choice(
    "mode",
    [
      "transition",
      "watertight",
      "boundary-coverage",
      "travel-coverage",
      "descent-coverage",
    ] as const,
    "transition",
  );
  const boundaryCoverage = mode === "boundary-coverage";
  const watertight = mode !== "transition";
  const spawn = argumentsReader.pair("spawn", {
    fallback: boundaryCoverage ? [1614, 294] : watertight ? [4194, 6034] : [4208, 6082],
    integer: true,
    minimum: -2_147_483_648,
    maximum: 2_147_483_647,
  }) ?? [0, 0];
  const look = argumentsReader.pair("look", {
    fallback: boundaryCoverage
      ? [3.326_412_741_337_916, -0.312_000_215_053_558]
      : [2.074_606, -0.371_797],
    minimum: -Math.PI * 2,
    maximum: Math.PI * 2,
  }) ?? [0, 0];
  if (look[1] < -Math.PI / 2 || look[1] > Math.PI / 2) {
    throw new Error("--look pitch must be in -pi/2..=pi/2");
  }
  const viewport = argumentsReader.pair("viewport", {
    fallback: boundaryCoverage ? [1848, 1345] : [1280, 720],
    separator: "x",
    integer: true,
    minimum: 240,
  }) ?? [1280, 720];
  const openWorldLab =
    argumentsReader.flag("world-lab") ||
    (boundaryCoverage && !argumentsReader.flag("no-world-lab"));
  const stepOffPillar =
    argumentsReader.flag("step-off-pillar") ||
    (boundaryCoverage && !argumentsReader.flag("no-step-off-pillar"));
  const shadows = argumentsReader.choice("shadows", ["on", "off"] as const, "on");
  const ambientOcclusion = argumentsReader.choice("ssao", ["on", "off"] as const, "off");
  const options: LodOptions = {
    mode,
    source: argumentsReader.choice(
      "source",
      ["procedural-v16", "terrain-diffusion-30m"] as const,
      "terrain-diffusion-30m",
    ),
    spawn,
    look,
    pillarHeight:
      argumentsReader.number("pillar-height", {
        // Keep the moving gate on the surrounding terrain. A four-metre isolated pedestal exposes
        // legitimate open air in the lower-screen ROI as the camera moves, which is not a cut hole.
        fallback: boundaryCoverage ? 1 : watertight ? 40 : 1,
        integer: true,
        minimum: 1,
        maximum: 1_000,
      }) ?? 1,
    pillarRadius:
      argumentsReader.number("pillar-radius", {
        // The transition fixture needs room to cross an eight-voxel snap threshold.
        fallback: boundaryCoverage ? 1 : watertight ? 3 : 12,
        integer: true,
        minimum: 1,
        maximum: 32,
      }) ?? 1,
    openWorldLab,
    stepOffPillar,
    viewport: { width: viewport[0], height: viewport[1] },
    deviceScaleFactor:
      argumentsReader.number("dpr", {
        fallback: boundaryCoverage ? 1.360_930_735_930_736 : 1,
        minimum: 0.5,
        maximum: 4,
      }) ?? 1,
    cascadedShadows: shadows === "on",
    screenSpaceAmbientOcclusion: ambientOcclusion === "on",
    recordVideo: argumentsReader.flag("video"),
    geometrySourceTravel: argumentsReader.flag("geometry-source-travel"),
    travelSeconds:
      argumentsReader.number("travel-seconds", {
        fallback: 30,
        minimum: 1,
        maximum: 300,
      }) ?? 30,
    buildProfile: argumentsReader.choice(
      "build",
      ["debug", "wasm-dev", "release"] as const,
      "release",
    ),
    environment: argumentsReader.choice(
      "environment",
      ["day-clear", "night-rain"] as const,
      "day-clear",
    ),
  };
  argumentsReader.assertEmpty();
  return options;
}

async function runLodTransition(context: ScenarioContext, arguments_: readonly string[]) {
  const options = parseOptions(arguments_);
  const boundaryCoverage = options.mode === "boundary-coverage";
  const descentCoverage = options.mode === "descent-coverage";
  const travelCoverage = options.mode === "travel-coverage" || descentCoverage;
  const watertight = options.mode !== "transition";
  const timings: LodTimings = {
    frameIntervals: [],
    gpu: new Map(),
    incompleteTransitionEdges: [],
  };
  const world = await startWorldStack(context, {
    fixture: {
      prefix: "voxels-lod-transition-",
      source: options.source,
      spawnVoxels: options.spawn,
      spawnPillarHeightVoxels: options.pillarHeight,
      spawnPillarRadiusVoxels: options.pillarRadius,
      // Sustained travel is a streaming-pressure gate. Match the checked-in production worker
      // budget so a fast local development machine cannot hide a fallback-coverage race.
      generationWorkers: travelCoverage ? 2 : undefined,
      generationWorkersPerClient: travelCoverage ? 1 : undefined,
      cascadedShadows: options.cascadedShadows,
      screenSpaceAmbientOcclusion: options.screenSpaceAmbientOcclusion,
      dayLengthSeconds: 0,
      dayFractionAtUnixEpoch: options.environment === "night-rain" ? 0 : 0.5,
      weatherCycleSeconds: 0,
      weatherFractionAtUnixEpoch: options.environment === "night-rain" ? 0.5 : 0.08,
      cloudVelocityMetresPerSecond: [0, 0],
    },
    service: { metal: options.source === "terrain-diffusion-30m" },
    web: { buildProfile: options.buildProfile },
  });
  const browser = await BrowserCapability.start(context, {
    warningPattern: FAILURE,
    launch: chromeWebGpuLaunchOptions(),
  });
  const viewport = await browser.open({
    url: world.url,
    label: "lod-transition",
    viewport: options.viewport,
    deviceScaleFactor: options.deviceScaleFactor,
    recordVideo: options.recordVideo,
    videoFilename: "transition-raw.webm",
    ...world.clientRoute,
  });
  const { engine, page } = viewport;
  const videoStartedAtMs = Date.now();
  let beforeSnapshot = await waitForEngine(engine, timings);
  beforeSnapshot = await setCameraLook(engine, options.look[0], options.look[1], timings);
  if (options.stepOffPillar) {
    await page.keyboard.down("KeyD");
    await page.waitForTimeout(160);
    await page.keyboard.up("KeyD");
  }
  const initialCentres = boundaryCentres(beforeSnapshot);
  beforeSnapshot = await waitForStableFrame(page, engine, initialCentres, timings);
  if (options.openWorldLab) {
    await page.keyboard.press("F3");
    beforeSnapshot = await waitForStableFrame(page, engine, initialCentres, timings);
  }
  await engine.setDiagnosticSky([255, 0, 255]);
  const beforePose = cameraPosition(beforeSnapshot);
  const before = await page.screenshot();
  await context.artifacts.write("LOD before", "before.png", before, "image/png");
  const beforeVideoSeconds = (Date.now() - videoStartedAtMs) / 1_000;
  if (options.recordVideo && !watertight) await page.waitForTimeout(1_500);

  if (travelCoverage) {
    await engine.setSpectator(true);
    const diagnosticTarget = options.geometrySourceTravel ? "black" : "magenta";
    if (options.geometrySourceTravel) await engine.setGeometrySourceDebug(true);
    const groundPose = cameraPosition(await readSnapshot(engine, timings));
    if (descentCoverage) {
      // Build a real continuous sky-to-ground trajectory instead of teleporting. The ascent
      // changes no X/Z ownership; the diagonal descent then exercises surface readiness, cut
      // transitions, raster seams, and view-dependent culling together.
      await page.keyboard.down("Space");
      await page.waitForTimeout(6_000);
      await page.keyboard.up("Space");
      await page.waitForTimeout(500);
      beforeSnapshot = await setCameraLook(
        engine,
        options.look[0],
        Math.min(options.look[1], -0.65),
        timings,
      );
    }
    const travelStartedAt = Date.now();
    const travelStartedPose = cameraPosition(await readSnapshot(engine, timings));
    const descentStopHeight = groundPose[1] + 30;
    const samples: Array<{
      readonly elapsedMs: number;
      readonly camera: Vector3;
      readonly diagnosticSkyPixels: number;
      readonly enclosedSkyPixels: number;
      readonly largestComponentPixels: number;
      readonly largestEnclosedComponentPixels: number;
      readonly presentedStrideVoxels: number;
      readonly incompleteTransitionEdges: number;
      readonly cutTransitionActive: boolean;
      readonly cutTransitionPhase: number;
      readonly surfaceQueued: number;
      readonly virtualTerrain: ReturnType<typeof virtualTerrainState>;
    }> = [];
    let worstScreenshot = options.geometrySourceTravel ? await page.screenshot() : before;
    let worst = await analyzeWatertightTerrain(page, worstScreenshot, diagnosticTarget);
    let failureGeometrySources: Buffer | undefined;
    await page.keyboard.down("KeyW");
    if (descentCoverage) await page.keyboard.down("ShiftLeft");
    try {
      while (Date.now() - travelStartedAt < options.travelSeconds * 1_000) {
        await page.waitForTimeout(200);
        const snapshot = await readSnapshot(engine, timings);
        const screenshot = await page.screenshot();
        const analysis = await analyzeWatertightTerrain(page, screenshot, diagnosticTarget);
        samples.push({
          elapsedMs: Date.now() - travelStartedAt,
          camera: cameraPosition(snapshot),
          diagnosticSkyPixels: analysis.diagnosticSkyPixels,
          enclosedSkyPixels: analysis.enclosedPixels,
          largestComponentPixels: analysis.largestComponentPixels,
          largestEnclosedComponentPixels: analysis.largestEnclosedComponentPixels,
          presentedStrideVoxels: snapshotValue(snapshot, "presentedLodStrideVoxels"),
          incompleteTransitionEdges: snapshotValue(snapshot, "lodIncompleteTransitionEdges"),
          cutTransitionActive: snapshotValue(snapshot, "lodCutTransitionActive") === 1,
          cutTransitionPhase: snapshotValue(snapshot, "lodCutTransitionPhase"),
          surfaceQueued: snapshotValue(snapshot, "surfaceQueued"),
          virtualTerrain: virtualTerrainState(snapshot),
        });
        if (
          analysis.enclosedPixels > worst.enclosedPixels ||
          (analysis.enclosedPixels === worst.enclosedPixels &&
            analysis.largestEnclosedComponentPixels > worst.largestEnclosedComponentPixels)
        ) {
          worst = analysis;
          worstScreenshot = screenshot;
        }
        if (
          analysis.enclosedPixels > 0 &&
          failureGeometrySources === undefined &&
          !options.geometrySourceTravel
        ) {
          await engine.setGeometrySourceDebug(true);
          failureGeometrySources = await page.screenshot();
          await engine.setGeometrySourceDebug(false);
        }
        if (descentCoverage && cameraPosition(snapshot)[1] <= descentStopHeight) break;
      }
    } finally {
      await page.keyboard.up("KeyW");
      if (descentCoverage) await page.keyboard.up("ShiftLeft");
    }
    if (descentCoverage) await page.waitForTimeout(500);
    const stoppedImmediateSnapshot = await readSnapshot(engine, timings);
    const travelFinishedPose = cameraPosition(stoppedImmediateSnapshot);
    const stoppedImmediateScreenshot = await page.screenshot();
    const stoppedImmediate = await analyzeWatertightTerrain(
      page,
      stoppedImmediateScreenshot,
      diagnosticTarget,
    );
    let settledSamples = 0;
    await engine.waitForSnapshot(
      (snapshot) => {
        collectTiming(snapshot, timings);
        const settled =
          snapshotValue(snapshot, "presentedLodStrideVoxels") === 1 &&
          snapshotValue(snapshot, "lodIncompleteTransitionEdges") === 0;
        settledSamples = settled ? settledSamples + 1 : 0;
        return settledSamples >= 3;
      },
      {
        timeoutMs: 10_000,
        intervalMs: 50,
        description: "nearby surface did not return to three stable canonical samples",
      },
    );
    // Let the short old-cut overlay retire after the canonical cut has stabilized.
    await page.waitForTimeout(250);
    const stoppedSettledSnapshot = await readSnapshot(engine, timings);
    const stoppedSettledScreenshot = await page.screenshot();
    const stoppedSettled = await analyzeWatertightTerrain(
      page,
      stoppedSettledScreenshot,
      diagnosticTarget,
    );
    let stoppedSettledGeometrySources: Buffer | undefined;
    if (stoppedSettled.enclosedPixels > 0) {
      if (options.geometrySourceTravel) {
        stoppedSettledGeometrySources = stoppedSettledScreenshot;
      } else {
        await engine.setGeometrySourceDebug(true);
        stoppedSettledGeometrySources = await page.screenshot();
        await engine.setGeometrySourceDebug(false);
      }
    }
    if (options.geometrySourceTravel) await engine.setGeometrySourceDebug(false);
    await engine.setDiagnosticSky(null);
    await context.artifacts.write(
      "Worst sustained travel coverage",
      "travel-worst-coverage.png",
      worstScreenshot,
      "image/png",
    );
    if (failureGeometrySources !== undefined) {
      await context.artifacts.write(
        "Geometry sources at sustained travel coverage failure",
        "travel-failure-geometry-sources.png",
        failureGeometrySources,
        "image/png",
      );
    }
    await context.artifacts.write(
      "Coverage immediately after stopping",
      "travel-stopped-immediate.png",
      stoppedImmediateScreenshot,
      "image/png",
    );
    await context.artifacts.write(
      "Coverage after the LOD handoff window",
      "travel-stopped-settled.png",
      stoppedSettledScreenshot,
      "image/png",
    );
    if (stoppedSettledGeometrySources !== undefined) {
      await context.artifacts.write(
        "Geometry sources after the LOD handoff window",
        "travel-stopped-settled-geometry-sources.png",
        stoppedSettledGeometrySources,
        "image/png",
      );
    }
    const uncoveredOwnerSamples = samples.filter(
      (sample) => sample.presentedStrideVoxels === 0,
    ).length;
    const violations: string[] = [];
    if (worst.enclosedPixels > 0) {
      violations.push("sustained travel exposed enclosed diagnostic sky inside the terrain ROI");
    }
    if (stoppedImmediate.enclosedPixels > 0) {
      violations.push("stopping exposed enclosed diagnostic sky inside the terrain ROI");
    }
    if (stoppedSettled.enclosedPixels > 0) {
      violations.push("settled nearby terrain retained enclosed diagnostic sky");
    }
    if (uncoveredOwnerSamples > 0) {
      violations.push("sustained travel sampled a world point without an LOD owner");
    }
    if (
      samples.some(
        (sample) =>
          sample.virtualTerrain.mode === 2 &&
          (sample.virtualTerrain.ownerlessRoots > 0 || !sample.virtualTerrain.gpuMatchesCpuCut),
      )
    ) {
      violations.push("visible virtual terrain lost exact CPU/GPU cut ownership");
    }
    if (descentCoverage && travelFinishedPose[1] > descentStopHeight + 5) {
      violations.push("spectator descent did not reach the registered near-ground handoff");
    }
    browser.assertHealthy();
    const result = {
      ok: violations.length === 0,
      mode: options.mode,
      commit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(),
      dirty: execFileSync("git", ["status", "--porcelain"], { encoding: "utf8" }).trim() !== "",
      source: options.source,
      environment: options.environment,
      browser: browser.version,
      travel: {
        trajectory: descentCoverage ? "diagonal-descent" : "horizontal-flight",
        requestedSeconds: options.travelSeconds,
        samples: samples.length,
        distanceMetres: spatialDistance(travelStartedPose, travelFinishedPose),
        startedPose: travelStartedPose,
        finishedPose: travelFinishedPose,
        altitude: {
          startedMetres: travelStartedPose[1],
          finishedMetres: travelFinishedPose[1],
          verticalDropMetres: Math.max(0, travelStartedPose[1] - travelFinishedPose[1]),
        },
        uncoveredOwnerSamples,
        maximumPresentedStrideVoxels: Math.max(
          0,
          ...samples.map((sample) => sample.presentedStrideVoxels),
        ),
        maximumIncompleteTransitionEdges: Math.max(
          0,
          ...samples.map((sample) => sample.incompleteTransitionEdges),
        ),
        surfaceQueue: {
          first: samples[0]?.surfaceQueued ?? 0,
          last: samples.at(-1)?.surfaceQueued ?? 0,
          maximum: Math.max(0, ...samples.map((sample) => sample.surfaceQueued)),
        },
        virtualTerrain: {
          visibleSamples: samples.filter((sample) => sample.virtualTerrain.mode === 2).length,
          maximumRegisteredRegions: Math.max(
            0,
            ...samples.map((sample) => sample.virtualTerrain.registeredRegions),
          ),
          maximumResidentPages: Math.max(
            0,
            ...samples.map((sample) => sample.virtualTerrain.residentPages),
          ),
          maximumSelectedPages: Math.max(
            0,
            ...samples.map((sample) => sample.virtualTerrain.selectedPages),
          ),
          maximumRequestedPages: Math.max(
            0,
            ...samples.map((sample) => sample.virtualTerrain.requestedPages),
          ),
          maximumOwnerlessRoots: Math.max(
            0,
            ...samples.map((sample) => sample.virtualTerrain.ownerlessRoots),
          ),
          gpuMismatchSamples: samples.filter(
            (sample) => sample.virtualTerrain.mode === 2 && !sample.virtualTerrain.gpuMatchesCpuCut,
          ).length,
        },
        lodQuality: summarizeTravelLodQuality(samples),
      },
      worst,
      stopped: {
        immediate: {
          image: stoppedImmediate,
          presentedStrideVoxels: snapshotValue(
            stoppedImmediateSnapshot,
            "presentedLodStrideVoxels",
          ),
          incompleteTransitionEdges: snapshotValue(
            stoppedImmediateSnapshot,
            "lodIncompleteTransitionEdges",
          ),
          surfaceQueued: snapshotValue(stoppedImmediateSnapshot, "surfaceQueued"),
          virtualTerrain: virtualTerrainState(stoppedImmediateSnapshot),
        },
        settled: {
          image: stoppedSettled,
          presentedStrideVoxels: snapshotValue(stoppedSettledSnapshot, "presentedLodStrideVoxels"),
          incompleteTransitionEdges: snapshotValue(
            stoppedSettledSnapshot,
            "lodIncompleteTransitionEdges",
          ),
          surfaceQueued: snapshotValue(stoppedSettledSnapshot, "surfaceQueued"),
          virtualTerrain: virtualTerrainState(stoppedSettledSnapshot),
        },
      },
      samples,
      violations,
    };
    await context.artifacts.writeJson("LOD report", "report.json", result);
    if (!result.ok) throw new Error(`LOD travel coverage violations: ${violations.join(", ")}`);
    return {
      summary: descentCoverage
        ? "Diagonal sky-to-ground LOD coverage passed."
        : "Sustained LOD travel coverage passed.",
      metrics: result.travel,
      details: result,
    };
  }

  if (watertight) {
    const headingSamples = [
      {
        yaw: options.look[0],
        image: await analyzeWatertightTerrain(page, before),
      },
    ];
    for (const [index, offset] of [-0.4, -0.2, 0.2, 0.4].entries()) {
      if (boundaryCoverage && options.openWorldLab) await page.keyboard.press("F3");
      beforeSnapshot = await setCameraLook(
        engine,
        options.look[0] + offset,
        options.look[1],
        timings,
      );
      beforeSnapshot = await waitForStableFrame(page, engine, initialCentres, timings);
      if (boundaryCoverage && options.openWorldLab) {
        await page.keyboard.press("F3");
        beforeSnapshot = await waitForStableFrame(page, engine, initialCentres, timings);
      }
      const screenshot = await page.screenshot();
      await context.artifacts.write(
        `LOD heading ${index + 1}`,
        `heading-${index + 1}.png`,
        screenshot,
        "image/png",
      );
      const image = await analyzeWatertightTerrain(page, screenshot);
      headingSamples.push({
        yaw: options.look[0] + offset,
        image,
      });
      if (image.diagnosticSkyPixels > 0) {
        await engine.setGeometrySourceDebug(true);
        const geometrySources = await page.screenshot();
        await context.artifacts.write(
          `LOD heading ${index + 1} geometry sources`,
          `heading-${index + 1}-geometry-sources.png`,
          geometrySources,
          "image/png",
        );
        await engine.setGeometrySourceDebug(false);
      }
    }
    await engine.setGeometrySourceDebug(true);
    const geometrySources = await page.screenshot();
    await context.artifacts.write(
      "LOD geometry sources",
      "geometry-sources.png",
      geometrySources,
      "image/png",
    );
    await engine.setGeometrySourceDebug(false);
    await engine.setDiagnosticSky(null);
    await sampleStablePerformance(page, engine, timings, 2_000);
    const image = headingSamples.reduce(
      (worst, sample) =>
        sample.image.largestComponentPixels > worst.largestComponentPixels ? sample.image : worst,
      headingSamples[0]?.image ?? (await analyzeWatertightTerrain(page, before)),
    );
    const performance = summarizePerformance(timings);
    const violations: string[] = [];
    if (image.diagnosticSkyPixels > 0)
      violations.push("terrain-only ROI exposes the diagnostic magenta sky");
    if (performance.frameP95Ms > 12) violations.push("frame p95 exceeded 12ms");
    if (performance.fractionAbove16_67Ms > 0.01)
      violations.push("over 1% of measured frames exceeded 16.67ms");
    if (performance.frameMaxMs > 25) violations.push("a measured frame exceeded 25ms");
    if (performance.worldGpuP95Ms > WORLD_GPU_P95_BUDGET_MS)
      violations.push(`world GPU p95 exceeded ${WORLD_GPU_P95_BUDGET_MS}ms`);
    if (performance.totalGpuP95Ms > 7.5) violations.push("total GPU p95 exceeded 7.5ms");
    browser.assertHealthy();
    const result = {
      ok: violations.length === 0,
      mode: options.mode,
      commit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(),
      dirty: execFileSync("git", ["status", "--porcelain"], { encoding: "utf8" }).trim() !== "",
      source: options.source,
      environment: options.environment,
      spawnVoxels: options.spawn,
      look: options.look,
      browser: browser.version,
      pose: beforePose,
      lod: {
        centres: initialCentres,
        worldQuads: snapshotValue(beforeSnapshot, "quads"),
        drawCalls: snapshotValue(beforeSnapshot, "drawCalls"),
        transitionQuads: snapshotValue(beforeSnapshot, "lodTransitionQuads"),
        viewportFingerprint: [
          snapshotValue(beforeSnapshot, "viewportFingerprintLow24"),
          snapshotValue(beforeSnapshot, "viewportFingerprintHigh24"),
        ],
      },
      image,
      headingSamples,
      performance,
      violations,
    };
    await context.artifacts.writeJson("LOD report", "report.json", result);
    if (!result.ok) throw new Error(`LOD ${options.mode} violations: ${violations.join(", ")}`);
    return {
      summary: `LOD ${options.mode} validation passed.`,
      metrics: performance,
      details: result,
    };
  } else {
    const movingCoverage = {
      samples: 0,
      framesWithHoles: 0,
      totalHolePixels: 0,
      worst: await analyzeWatertightTerrain(page, before),
      worstScreenshot: before,
    };
    const cameraVoxelX = beforePose[0] / 0.1;
    const firstCentre = initialCentres[0];
    if (firstCentre === undefined) throw new Error("LOD fixture has no boundary centre");
    const desiredXDirection = Math.sign(cameraVoxelX - firstCentre[0]) || 1;
    const forwardXDirection = Math.sign(Math.sin(snapshotValue(beforeSnapshot, "yaw"))) || 1;
    const outboundKey = desiredXDirection === forwardXDirection ? "KeyW" : "KeyS";
    let crossedSnapshot: readonly number[] | undefined;
    let capturedEngineFrames = 0;
    let skippedEngineFrames = 0;
    let crossingPresentedFrame = 0;
    let movementSnapshots: readonly (readonly number[])[] = [];
    // Spectator flight drives the same moving-camera LOD focus and cut renderer, then restores the
    // saved body pose exactly when disabled. The old keyboard braking loop varied by centimetres
    // and introduced enough parallax to hide the cut comparison behind a positioning error.
    await engine.setSpectator(true);
    const presentedFrames = await capturePresentedFrames(page, async (capture) => {
      const movement = await waitForCentreChange(
        page,
        engine,
        initialCentres,
        outboundKey,
        timings,
        12,
        () => {
          crossingPresentedFrame = capture.observedFrames();
        },
      );
      crossedSnapshot = movement.crossed;
      capturedEngineFrames = movement.observedEngineFrames;
      skippedEngineFrames = movement.skippedEngineFrames;
      movementSnapshots = movement.snapshots;
    });
    if (crossedSnapshot === undefined) {
      throw new Error("presented-frame capture completed without crossing an LOD boundary");
    }
    for (const screenshot of presentedFrames.frames) {
      const analysis = await analyzeWatertightTerrain(page, screenshot);
      movingCoverage.samples += 1;
      movingCoverage.totalHolePixels += analysis.diagnosticSkyPixels;
      if (analysis.diagnosticSkyPixels > 0) movingCoverage.framesWithHoles += 1;
      if (analysis.diagnosticSkyPixels > movingCoverage.worst.diagnosticSkyPixels) {
        movingCoverage.worst = analysis;
        movingCoverage.worstScreenshot = screenshot;
      }
    }
    const capturedPresentationDurationMs =
      presentedFrames.firstTimestamp === null || presentedFrames.lastTimestamp === null
        ? null
        : (presentedFrames.lastTimestamp - presentedFrames.firstTimestamp) * 1_000;
    const crossedPose = cameraPosition(crossedSnapshot);
    if (planarDistance(crossedPose, beforePose) <= 0) {
      throw new Error("LOD focus changed without measurable player movement");
    }
    if (presentedFrames.overflowFrames > 0) {
      throw new Error(
        `presented-frame capture exceeded ${MAX_PRESENTED_FRAME_CAPTURES} frames by ${presentedFrames.overflowFrames}`,
      );
    }
    if (movingCoverage.samples < 8) {
      throw new Error(
        `presented-frame capture observed only ${movingCoverage.samples} compositor frames`,
      );
    }
    const transitionActiveFrames = movementSnapshots.filter(
      (snapshot) => snapshotValue(snapshot, "lodCutTransitionActive") === 1,
    ).length;
    const transitionActiveObserved = transitionActiveFrames > 0;
    const continuityStart = Math.max(
      0,
      Math.min(crossingPresentedFrame - 1, presentedFrames.frames.length - 2),
    );
    const continuityFrames = presentedFrames.frames.slice(continuityStart);
    const continuityTimestamps = presentedFrames.timestamps.slice(continuityStart);
    const continuityAnalysis = await analyzePresentedFrameContinuity(
      page,
      continuityFrames,
      continuityTimestamps,
    );
    const crossingPair = continuityAnalysis.pairs[0];
    if (crossingPair === undefined) {
      throw new Error("LOD continuity capture omitted the ownership crossing pair");
    }
    const continuity = {
      ...continuityAnalysis,
      crossing: {
        ...crossingPair,
        normalizedDeltaOverMotionMedian:
          crossingPair.normalizedMeanAbsoluteLinearLumaDelta /
          Math.max(continuityAnalysis.normalizedMeanAbsoluteLinearLumaDelta.median, 0.000_1),
      },
    };
    const worstContinuityPair = continuity.normalizedMeanAbsoluteLinearLumaDelta.maximumPair;
    await context.artifacts.write(
      "Worst LOD continuity pair before",
      "moving-worst-continuity-before.png",
      continuityFrames[worstContinuityPair] ?? continuityFrames[0] ?? before,
      "image/png",
    );
    await context.artifacts.write(
      "Worst LOD continuity pair after",
      "moving-worst-continuity-after.png",
      continuityFrames[worstContinuityPair + 1] ?? continuityFrames.at(-1) ?? before,
      "image/png",
    );
    await engine.setSpectator(false);
    const afterSnapshot = await waitForStableChangedFrame(page, engine, initialCentres, timings);
    const afterCentres = boundaryCentres(afterSnapshot);
    const afterPose = cameraPosition(afterSnapshot);
    const after = await page.screenshot();
    await context.artifacts.write("LOD after", "after.png", after, "image/png");
    await context.artifacts.write(
      "Worst moving LOD coverage",
      "moving-worst-coverage.png",
      movingCoverage.worstScreenshot,
      "image/png",
    );
    const afterVideoSeconds = (Date.now() - videoStartedAtMs) / 1_000;
    if (options.recordVideo) await page.waitForTimeout(1_500);
    await engine.setDiagnosticSky(null);
    await sampleStablePerformance(page, engine, timings, 2_000);
    const [comparison, beforeSkyExposure, afterSkyExposure] = await Promise.all([
      compareScreenshots(page, before, after),
      analyzeDiagnosticSky(page, before, { x0: 0.02, x1: 0.46, y0: 0.3, y1: 0.58 }),
      analyzeDiagnosticSky(page, after, { x0: 0.02, x1: 0.46, y0: 0.3, y1: 0.58 }),
    ]);
    const image = {
      ...comparison,
      movingCoverage: {
        samples: movingCoverage.samples,
        framesWithHoles: movingCoverage.framesWithHoles,
        totalHolePixels: movingCoverage.totalHolePixels,
        observedFrames: presentedFrames.observedFrames,
        overflowFrames: presentedFrames.overflowFrames,
        capturedEngineFrames,
        skippedEngineFrames,
        captureDurationMs: capturedPresentationDurationMs,
        cutTransition: {
          activeObserved: transitionActiveObserved,
          activeEngineFrames: transitionActiveFrames,
          expectedInactive: true,
        },
        continuity,
        worst: movingCoverage.worst,
      },
      diagnosticSkyExposure: {
        before: beforeSkyExposure,
        after: afterSkyExposure,
      },
    };
    const performance = summarizePerformance(timings);
    const planarPoseErrorMetres = planarDistance(beforePose, afterPose);
    const poseErrorMetres = spatialDistance(beforePose, afterPose);
    const violations: string[] = [];
    // Ground height follows the returned X/Z position. A few centimetres on a steep voxel slope
    // can legitimately move Y farther, while the screenshots remain horizontally registered.
    if (planarPoseErrorMetres > 0.025)
      violations.push("camera did not return to the same horizontal pose");
    if (image.relativeMeanLumaDelta > 0.04)
      violations.push("valley mean luminance changed by over 4%");
    if (image.meanAbsoluteLinearLumaDelta > 0.025)
      violations.push("valley mean absolute luminance delta exceeded 0.025");
    if (image.catastrophicDarkFraction > 0.01)
      violations.push("over 1% of valley pixels changed luminance by at least 2x");
    if (image.nearBlackPixelFraction.before > 0.1 || image.nearBlackPixelFraction.after > 0.1) {
      violations.push("over 10% of valley pixels rendered near-black");
    }
    if (
      image.diagnosticSkyExposure.before.diagnosticSkyPixels > 0 ||
      image.diagnosticSkyExposure.after.diagnosticSkyPixels > 0
    ) {
      violations.push("valley terrain exposes the diagnostic magenta sky");
    }
    if (image.movingCoverage.framesWithHoles !== 0)
      violations.push(
        `moving LOD transition exposed diagnostic sky in ${image.movingCoverage.framesWithHoles} presented frames`,
      );
    if (image.movingCoverage.cutTransition.activeObserved)
      violations.push("legacy dual-cut transition overlay became active");
    if (image.movingCoverage.continuity.normalizedMeanAbsoluteLinearLumaDelta.maximum > 0.05) {
      violations.push("moving LOD continuity delta exceeded 0.05 linear luminance");
    }
    if (
      image.movingCoverage.continuity.normalizedMeanAbsoluteLinearLumaDelta.maximumOverMedian > 3
    ) {
      violations.push("moving LOD continuity produced a delta over 3x the motion median");
    }
    if (image.movingCoverage.continuity.crossing.normalizedDeltaOverMotionMedian > 3) {
      violations.push("LOD ownership crossing produced a delta over 3x the motion median");
    }
    if (image.ssim < 0.97) violations.push("valley SSIM fell below 0.97");
    if (performance.frameP95Ms > 12) violations.push("frame p95 exceeded 12ms");
    if (performance.fractionAbove16_67Ms > 0.01)
      violations.push("over 1% of measured frames exceeded 16.67ms");
    if (performance.frameMaxMs > 25) violations.push("a measured frame exceeded 25ms");
    if (performance.worldGpuP95Ms > WORLD_GPU_P95_BUDGET_MS)
      violations.push(`world GPU p95 exceeded ${WORLD_GPU_P95_BUDGET_MS}ms`);
    if (performance.totalGpuP95Ms > 7.5) violations.push("total GPU p95 exceeded 7.5ms");
    browser.assertHealthy();

    const result = {
      ok: violations.length === 0,
      mode: "transition",
      commit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(),
      dirty: execFileSync("git", ["status", "--porcelain"], { encoding: "utf8" }).trim() !== "",
      source: options.source,
      environment: options.environment,
      spawnVoxels: options.spawn,
      look: options.look,
      browser: browser.version,
      pose: {
        before: beforePose,
        crossed: crossedPose,
        after: afterPose,
        planarErrorMetres: planarPoseErrorMetres,
        errorMetres: poseErrorMetres,
      },
      lod: {
        centresBefore: initialCentres,
        centresAfter: afterCentres,
        transitionQuadsBefore: snapshotValue(beforeSnapshot, "lodTransitionQuads"),
        transitionQuadsAfter: snapshotValue(afterSnapshot, "lodTransitionQuads"),
        viewportFingerprintBefore: [
          snapshotValue(beforeSnapshot, "viewportFingerprintLow24"),
          snapshotValue(beforeSnapshot, "viewportFingerprintHigh24"),
        ],
        viewportFingerprintAfter: [
          snapshotValue(afterSnapshot, "viewportFingerprintLow24"),
          snapshotValue(afterSnapshot, "viewportFingerprintHigh24"),
        ],
      },
      image,
      performance,
      violations,
      ...(options.recordVideo
        ? {
            videoMarkers: {
              beforeSeconds: beforeVideoSeconds,
              afterSeconds: afterVideoSeconds,
            },
          }
        : {}),
    };
    await context.artifacts.writeJson("LOD report", "report.json", result);
    if (!result.ok) throw new Error(`LOD transition violations: ${violations.join(", ")}`);
    return {
      summary: "LOD transition validation passed.",
      metrics: performance,
      details: result,
    };
  }
}

export default defineScenario({
  id: "lod-transition",
  kind: "validation",
  summary: "Validates LOD continuity, terrain watertightness, and boundary coverage.",
  uses: {
    world: true,
    browser: true,
    viewport: "browser",
    screenshots: true,
    video: true,
    metrics: true,
    rust: true,
  },
  timeoutMs: 1_800_000,
  run: runLodTransition,
});
