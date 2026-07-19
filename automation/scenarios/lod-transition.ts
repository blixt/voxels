import { execFileSync } from "node:child_process";
import type { Page } from "playwright";
import { ScenarioArguments } from "../lib/arguments.ts";
import { BrowserCapability, chromeWebGpuLaunchOptions } from "../lib/browser.ts";
import {
  type EngineClient,
  FRAME_SAMPLE_WIDTH,
  gpuSampleStart,
  GPU_SAMPLE_WIDTH,
  SNAPSHOT,
  snapshotValue,
} from "../lib/engine.ts";
import { percentile } from "../lib/metrics.ts";
import { defineScenario, type ScenarioContext } from "../lib/scenario.ts";
import { startWorldStack } from "../lib/world.ts";
import type { WorldSource } from "../lib/world.ts";

const FRAME_SAMPLE_START = SNAPSHOT.droppedSamples + 1;
const FAILURE =
  /panic|unreachable|runtimeerror|wgpu|webgpu|shader|sqlite|opfs|syncaccesshandle|nomodificationallowed|web lock request failed|no persistence leader|persistence .*failed/i;
type Vector2 = readonly [number, number];
type Vector3 = readonly [number, number, number];
type BoundaryCentres = readonly Vector2[];

interface LodTimings {
  readonly frameIntervals: number[];
  readonly gpu: Map<number, { readonly total: number; readonly world: number }>;
}

function required(values: ArrayLike<number>, index: number, label: string): number {
  const value = values[index];
  if (value === undefined) throw new Error(`${label} omitted value ${index}`);
  return value;
}

function boundaryCentres(snapshot: readonly number[]): BoundaryCentres {
  return Array.from(
    { length: 6 },
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

function planarDistance(left: Vector3, right: Vector3): number {
  return Math.hypot(left[0] - right[0], left[2] - right[2]);
}

function spatialDistance(left: Vector3, right: Vector3): number {
  return Math.hypot(left[0] - right[0], left[1] - right[1], left[2] - right[2]);
}

function collectTiming(snapshot: readonly number[], timings: LodTimings): void {
  const sampleCount = snapshotValue(snapshot, "sampleCount");
  for (let index = 0; index < sampleCount; index += 1) {
    const start = FRAME_SAMPLE_START + index * FRAME_SAMPLE_WIDTH;
    timings.frameIntervals.push(required(snapshot, start, "frame timing"));
  }
  const start = gpuSampleStart(snapshot);
  const gpuCount = snapshot[start] ?? 0;
  for (let index = 0; index < gpuCount; index += 1) {
    const sample = start + 2 + index * GPU_SAMPLE_WIDTH;
    timings.gpu.set(required(snapshot, sample, "GPU timing"), {
      total: required(snapshot, sample + 1, "GPU timing"),
      world: required(snapshot, sample + 8, "GPU timing"),
    });
  }
}

function resetTimings(timings: LodTimings): void {
  timings.frameIntervals.length = 0;
  timings.gpu.clear();
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
): Promise<readonly number[]> {
  const deadline = Date.now() + 4_000;
  await page.keyboard.down(outboundKey);
  try {
    while (Date.now() < deadline) {
      const snapshot = await readSnapshot(engine, timings);
      if (!sameCentres(boundaryCentres(snapshot), initialCentres)) return snapshot;
      await page.waitForTimeout(8);
    }
  } finally {
    await page.keyboard.up(outboundKey);
  }
  throw new Error("walking forward did not cross an LOD snap boundary");
}

async function returnToPose(
  page: Page,
  engine: EngineClient,
  target: Vector3,
  initialCentres: BoundaryCentres,
  timings: LodTimings,
): Promise<readonly number[]> {
  const correctionHistory: {
    readonly correction: number;
    readonly distance: number;
    readonly position: Vector3;
  }[] = [];
  const brake = async (duration: number): Promise<void> => {
    const keys = ["KeyW", "KeyS", "KeyA", "KeyD"];
    for (const key of keys) await page.keyboard.down(key);
    try {
      const deadline = Date.now() + duration;
      while (Date.now() < deadline) {
        await readSnapshot(engine, timings);
        await page.waitForTimeout(16);
      }
    } finally {
      for (const key of keys) await page.keyboard.up(key);
    }
  };
  await brake(300);
  let latest = await readSnapshot(engine, timings);
  for (let correction = 0; correction < 80; correction += 1) {
    const position = cameraPosition(latest);
    const distance = planarDistance(position, target);
    correctionHistory.push({ correction, distance, position });
    if (distance <= 0.008) {
      await brake(200);
      latest = await readSnapshot(engine, timings);
      if (planarDistance(cameraPosition(latest), target) <= 0.015) break;
      continue;
    }
    const yaw = snapshotValue(latest, "yaw");
    const forward: Vector2 = [Math.sin(yaw), -Math.cos(yaw)];
    const right: Vector2 = [-forward[1], forward[0]];
    const error: Vector2 = [target[0] - position[0], target[2] - position[2]];
    const candidates: readonly (readonly [string, Vector2])[] = [
      ["KeyW", forward],
      ["KeyS", [-forward[0], -forward[1]]],
      ["KeyD", right],
      ["KeyA", [-right[0], -right[1]]],
    ];
    const candidate = candidates.toSorted(
      (left, rightCandidate) =>
        rightCandidate[1][0] * error[0] +
        rightCandidate[1][1] * error[1] -
        (left[1][0] * error[0] + left[1][1] * error[1]),
    )[0];
    if (candidate === undefined) throw new Error("camera correction has no movement candidate");
    const [key] = candidate;
    await page.keyboard.down(key);
    try {
      const deadline = Date.now() + Math.min(40, Math.max(8, distance * 80));
      while (Date.now() < deadline) {
        latest = await readSnapshot(engine, timings);
        await page.waitForTimeout(4);
      }
    } finally {
      await page.keyboard.up(key);
    }
    await brake(48);
    latest = await readSnapshot(engine, timings);
  }
  const error = planarDistance(cameraPosition(latest), target);
  if (error > 0.025) {
    throw new Error(
      `could not return to camera pose; planar error ${error}m; final corrections ${JSON.stringify(correctionHistory.slice(-8))}`,
    );
  }
  if (sameCentres(boundaryCentres(latest), initialCentres)) {
    throw new Error(
      `LOD focus returned to its initial state at the comparison pose: ${JSON.stringify(initialCentres)}`,
    );
  }
  return latest;
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
      const isolatedSkyExposures = (pixels: Uint8ClampedArray) => {
        // Stay below the irregular tree-lined horizon: this measures pale sky samples fully
        // enclosed by the valley terrain, the one-pixel signature of an uncovered far-LOD
        // T-junction. Keep a few coordinates so a failure is immediately reproducible.
        const crackRoi = { ...roi, y0: Math.max(roi.y0, Math.floor(left.height * 0.3)) };
        const skyLike = (x: number, y: number): boolean => {
          const index = (x + y * left.width) * 4;
          const red = at(pixels, index);
          const green = at(pixels, index + 1);
          const blue = at(pixels, index + 2);
          return red > 150 && green > 160 && blue > 180 && blue > red * 1.05;
        };
        let count = 0;
        const coordinates: Vector2[] = [];
        for (let y = crackRoi.y0 + 1; y < crackRoi.y1 - 1; y += 1) {
          for (let x = crackRoi.x0 + 1; x < crackRoi.x1 - 1; x += 1) {
            if (!skyLike(x, y)) continue;
            let skyNeighbours = 0;
            for (let offsetY = -1; offsetY <= 1; offsetY += 1) {
              for (let offsetX = -1; offsetX <= 1; offsetX += 1) {
                if ((offsetX !== 0 || offsetY !== 0) && skyLike(x + offsetX, y + offsetY)) {
                  skyNeighbours += 1;
                }
              }
            }
            if (skyNeighbours === 0) {
              count += 1;
              if (coordinates.length < 16) coordinates.push([x, y]);
            }
          }
        }
        return { count, coordinates, roi: crackRoi };
      };
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
      const isolatedSkyExposurePixels = {
        before: isolatedSkyExposures(left.pixels),
        after: isolatedSkyExposures(right.pixels),
      };
      return {
        roi,
        pixels,
        comparisonSamples: count,
        comparisonFootprintPixels: footprint,
        isolatedSkyExposurePixels,
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

async function analyzeWatertightTerrain(page: Page, screenshot: Buffer) {
  return page.evaluate(async (base64: string) => {
    const at = (values: ArrayLike<number>, index: number): number => {
      const value = values[index];
      if (value === undefined) throw new Error(`watertight analysis omitted value ${index}`);
      return value;
    };
    const response = await fetch(`data:image/png;base64,${base64}`);
    const bitmap = await createImageBitmap(await response.blob());
    const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
    const context = canvas.getContext("2d", { willReadFrequently: true });
    if (context === null) throw new Error("watertight analysis canvas is unavailable");
    context.drawImage(bitmap, 0, 0);
    const pixels = context.getImageData(0, 0, bitmap.width, bitmap.height).data;
    // This fixed camera looks down onto uninterrupted terrain here. The historical crack exposed
    // the red/brown sky in one large triangle; terrain at this fixture is green or neutral.
    const roi = {
      x0: Math.floor(bitmap.width * 0.4),
      x1: Math.ceil(bitmap.width * 0.88),
      y0: Math.floor(bitmap.height * 0.52),
      y1: Math.ceil(bitmap.height * 0.605),
    };
    const width = roi.x1 - roi.x0;
    const height = roi.y1 - roi.y0;
    const skyLike = new Uint8Array(width * height);
    const coolExposure = new Uint8Array(width * height);
    let skyLikePixels = 0;
    let coolExposurePixels = 0;
    for (let y = roi.y0; y < roi.y1; y += 1) {
      for (let x = roi.x0; x < roi.x1; x += 1) {
        const source = (x + y * bitmap.width) * 4;
        const red = at(pixels, source);
        const green = at(pixels, source + 1);
        const blue = at(pixels, source + 2);
        const target = x - roi.x0 + (y - roi.y0) * width;
        const exposedSky = red > 25 && red > green * 1.35 && red > blue * 1.08;
        if (exposedSky) {
          skyLike[target] = 1;
          skyLikePixels += 1;
        }
        // The reported boundary crack exposed a cool, bright cloud/sky layer rather than the
        // reddish horizon covered by the original gate. Grass is green-dominant in this fixed
        // fixture, so this also remains insensitive to normal terrain lighting changes.
        const exposedCoolSky = (red + green + blue) / 3 > 61 && green < Math.max(red, blue) * 1.18;
        if (exposedCoolSky) {
          coolExposure[target] = 1;
          coolExposurePixels += 1;
        }
      }
    }
    const connectedComponentSummary = (mask: Uint8Array) => {
      const visited = new Uint8Array(mask.length);
      let largest = 0;
      let largestBroad = 0;
      for (let start = 0; start < mask.length; start += 1) {
        if (mask[start] === 0 || visited[start] !== 0) continue;
        const stack: number[] = [start];
        visited[start] = 1;
        let component = 0;
        let minX = width;
        let maxX = 0;
        let minY = height;
        let maxY = 0;
        while (stack.length > 0) {
          const current = stack.pop();
          if (current === undefined) break;
          component += 1;
          const x = current % width;
          const y = Math.floor(current / width);
          minX = Math.min(minX, x);
          maxX = Math.max(maxX, x);
          minY = Math.min(minY, y);
          maxY = Math.max(maxY, y);
          const neighbors = [
            x > 0 ? current - 1 : -1,
            x + 1 < width ? current + 1 : -1,
            y > 0 ? current - width : -1,
            y + 1 < height ? current + width : -1,
          ];
          for (const neighbor of neighbors) {
            if (neighbor < 0 || mask[neighbor] === 0 || visited[neighbor] !== 0) continue;
            visited[neighbor] = 1;
            stack.push(neighbor);
          }
        }
        largest = Math.max(largest, component);
        // Red/brown tree trunks share the original horizon-color predicate, but are narrow and
        // vertical. The regression this fixture owns is a broad triangular terrain opening.
        if (maxX - minX + 1 >= 32 && maxY - minY + 1 >= 4) {
          largestBroad = Math.max(largestBroad, component);
        }
      }
      return { largest, largestBroad };
    };
    const skyLikeComponents = connectedComponentSummary(skyLike);
    const coolExposureComponents = connectedComponentSummary(coolExposure);
    const sampledPixels = skyLike.length;
    return {
      roi,
      sampledPixels,
      skyLikePixels,
      skyLikeFraction: skyLikePixels / sampledPixels,
      largestSkyLikeComponent: skyLikeComponents.largest,
      largestBroadSkyLikeComponent: skyLikeComponents.largestBroad,
      largestBroadSkyLikeFraction: skyLikeComponents.largestBroad / sampledPixels,
      coolExposurePixels,
      coolExposureFraction: coolExposurePixels / sampledPixels,
      largestCoolExposureComponent: coolExposureComponents.largest,
      largestCoolExposureFraction: coolExposureComponents.largest / sampledPixels,
    };
  }, screenshot.toString("base64"));
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
  };
}

type LodMode = "transition" | "watertight" | "boundary-coverage";

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
  readonly buildProfile: "debug" | "wasm-dev" | "release";
}

function parseOptions(arguments_: readonly string[]): LodOptions {
  const argumentsReader = new ScenarioArguments(arguments_);
  const mode = argumentsReader.choice(
    "mode",
    ["transition", "watertight", "boundary-coverage"] as const,
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
        fallback: boundaryCoverage ? 1 : 40,
        integer: true,
        minimum: 1,
        maximum: 1_000,
      }) ?? 1,
    pillarRadius:
      argumentsReader.number("pillar-radius", {
        fallback: boundaryCoverage ? 1 : watertight ? 3 : 6,
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
    buildProfile: argumentsReader.choice(
      "build",
      ["debug", "wasm-dev", "release"] as const,
      "release",
    ),
  };
  argumentsReader.assertEmpty();
  return options;
}

async function runLodTransition(context: ScenarioContext, arguments_: readonly string[]) {
  const options = parseOptions(arguments_);
  const boundaryCoverage = options.mode === "boundary-coverage";
  const watertight = options.mode !== "transition";
  const timings: LodTimings = { frameIntervals: [], gpu: new Map() };
  const world = await startWorldStack(context, {
    fixture: {
      prefix: "voxels-lod-transition-",
      source: options.source,
      spawnVoxels: options.spawn,
      spawnPillarHeightVoxels: options.pillarHeight,
      spawnPillarRadiusVoxels: options.pillarRadius,
      cascadedShadows: options.cascadedShadows,
      screenSpaceAmbientOcclusion: options.screenSpaceAmbientOcclusion,
      dayLengthSeconds: 0,
      dayFractionAtUnixEpoch: 0.5,
      weatherCycleSeconds: 0,
      weatherFractionAtUnixEpoch: 0.08,
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
  const beforePose = cameraPosition(beforeSnapshot);
  const before = await page.screenshot();
  await context.artifacts.write("LOD before", "before.png", before, "image/png");
  const beforeVideoSeconds = (Date.now() - videoStartedAtMs) / 1_000;
  if (options.recordVideo && !watertight) await page.waitForTimeout(1_500);

  if (watertight) {
    const headingSamples = [
      {
        yaw: options.look[0],
        image: await analyzeWatertightTerrain(page, before),
      },
    ];
    if (boundaryCoverage) {
      for (const [index, offset] of [-0.16, -0.08, 0.08, 0.16].entries()) {
        await page.keyboard.press("F3");
        beforeSnapshot = await setCameraLook(
          engine,
          options.look[0] + offset,
          options.look[1],
          timings,
        );
        beforeSnapshot = await waitForStableFrame(page, engine, initialCentres, timings);
        await page.keyboard.press("F3");
        beforeSnapshot = await waitForStableFrame(page, engine, initialCentres, timings);
        const screenshot = await page.screenshot();
        await context.artifacts.write(
          `LOD heading ${index + 1}`,
          `heading-${index + 1}.png`,
          screenshot,
          "image/png",
        );
        headingSamples.push({
          yaw: options.look[0] + offset,
          image: await analyzeWatertightTerrain(page, screenshot),
        });
      }
    }
    await sampleStablePerformance(page, engine, timings, 2_000);
    const image = headingSamples.reduce(
      (worst, sample) =>
        sample.image.largestCoolExposureComponent > worst.largestCoolExposureComponent
          ? sample.image
          : worst,
      headingSamples[0]?.image ?? (await analyzeWatertightTerrain(page, before)),
    );
    const performance = summarizePerformance(timings);
    const violations: string[] = [];
    if (!boundaryCoverage) {
      if (image.largestBroadSkyLikeComponent > 32)
        violations.push("terrain-only ROI contains a connected sky-colored crack");
    }
    if (image.largestCoolExposureComponent > 256)
      violations.push("terrain-only ROI contains a connected cool sky/cloud exposure");
    if (performance.frameP95Ms > 12) violations.push("frame p95 exceeded 12ms");
    if (performance.fractionAbove16_67Ms > 0.01)
      violations.push("over 1% of measured frames exceeded 16.67ms");
    if (performance.frameMaxMs > 25) violations.push("a measured frame exceeded 25ms");
    const worldGpuBudgetMs = boundaryCoverage ? 3 : 2;
    if (performance.worldGpuP95Ms > worldGpuBudgetMs)
      violations.push(`world GPU p95 exceeded ${worldGpuBudgetMs}ms`);
    if (performance.totalGpuP95Ms > 7.5) violations.push("total GPU p95 exceeded 7.5ms");
    browser.assertHealthy();
    const result = {
      ok: violations.length === 0,
      mode: options.mode,
      commit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(),
      dirty: execFileSync("git", ["status", "--porcelain"], { encoding: "utf8" }).trim() !== "",
      source: options.source,
      spawnVoxels: options.spawn,
      look: options.look,
      browser: browser.version,
      pose: beforePose,
      lod: {
        centres: initialCentres,
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
    const cameraVoxelX = beforePose[0] / 0.1;
    const firstCentre = initialCentres[0];
    if (firstCentre === undefined) throw new Error("LOD fixture has no boundary centre");
    const desiredXDirection = Math.sign(cameraVoxelX - firstCentre[0]) || 1;
    const forwardXDirection = Math.sign(Math.sin(snapshotValue(beforeSnapshot, "yaw"))) || 1;
    const outboundKey = desiredXDirection === forwardXDirection ? "KeyW" : "KeyS";
    const crossedSnapshot = await waitForCentreChange(
      page,
      engine,
      initialCentres,
      outboundKey,
      timings,
    );
    const crossedPose = cameraPosition(crossedSnapshot);
    if (planarDistance(crossedPose, beforePose) <= 0) {
      throw new Error("LOD focus changed without measurable player movement");
    }
    await returnToPose(page, engine, beforePose, initialCentres, timings);
    const afterSnapshot = await waitForStableChangedFrame(page, engine, initialCentres, timings);
    const afterCentres = boundaryCentres(afterSnapshot);
    const afterPose = cameraPosition(afterSnapshot);
    const after = await page.screenshot();
    await context.artifacts.write("LOD after", "after.png", after, "image/png");
    const afterVideoSeconds = (Date.now() - videoStartedAtMs) / 1_000;
    if (options.recordVideo) await page.waitForTimeout(1_500);
    await sampleStablePerformance(page, engine, timings, 2_000);
    const image = await compareScreenshots(page, before, after);
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
      image.isolatedSkyExposurePixels.before.count > 0 ||
      image.isolatedSkyExposurePixels.after.count > 0
    ) {
      violations.push("valley terrain contains isolated sky-exposure pixels");
    }
    if (image.ssim < 0.97) violations.push("valley SSIM fell below 0.97");
    if (performance.frameP95Ms > 12) violations.push("frame p95 exceeded 12ms");
    if (performance.fractionAbove16_67Ms > 0.01)
      violations.push("over 1% of measured frames exceeded 16.67ms");
    if (performance.frameMaxMs > 25) violations.push("a measured frame exceeded 25ms");
    if (performance.worldGpuP95Ms > 2) violations.push("world GPU p95 exceeded 2ms");
    if (performance.totalGpuP95Ms > 7.5) violations.push("total GPU p95 exceeded 7.5ms");
    browser.assertHealthy();

    const result = {
      ok: violations.length === 0,
      mode: "transition",
      commit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(),
      dirty: execFileSync("git", ["status", "--porcelain"], { encoding: "utf8" }).trim() !== "",
      source: options.source,
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
