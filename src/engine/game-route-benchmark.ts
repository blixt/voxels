import { average, maxValue, percentile } from "./benchmark-metrics.ts";
import { CLEAR_COLOR_RGBA } from "./render-constants.ts";
import { metersToWorldUnits } from "./scale.ts";
import type { Vec3 } from "./types.ts";

const DEFAULT_ROUTE_SEGMENTS = [
  { yawRadians: 0.06, distanceMeters: 12 },
  { yawRadians: 0.22, distanceMeters: 10 },
  { yawRadians: -0.18, distanceMeters: 10 },
  { yawRadians: 0.18, distanceMeters: 10 },
  { yawRadians: -0.12, distanceMeters: 10 },
  { yawRadians: 0.08, distanceMeters: 12 },
] as const;

export interface RouteBenchmarkFrameTarget {
  frame: number;
  phase: "move";
  simTimeSeconds: number;
  distanceMeters: number;
  feetPosition: Vec3;
  yaw: number;
  pitch: number;
  segmentIndex: number;
}

export interface RouteBenchmarkPlan {
  durationSeconds: number;
  sampleHz: number;
  speedMetersPerSecond: number;
  totalDistanceMeters: number;
  frames: RouteBenchmarkFrameTarget[];
}

export interface RouteBenchmarkPlanOptions {
  durationSeconds?: number;
  sampleHz?: number;
  speedMetersPerSecond?: number;
  pitchRadians?: number;
}

export interface BottomCenterVoidProbe {
  sampleCount: number;
  clearPixelCount: number;
  clearRatio: number;
  maxClearRun: number;
  maxClearRunRatio: number;
  suspicious: boolean;
}

export interface SettledReferenceDiffProbe {
  sampleCount: number;
  changedPixelCount: number;
  changedRatio: number;
  clearToFilledPixelCount: number;
  clearToFilledRatio: number;
  filledToClearPixelCount: number;
  filledToClearRatio: number;
  maxClearToFilledRun: number;
  maxClearToFilledRunRatio: number;
  suspiciousHole: boolean;
}

export interface RouteFrameAccountingSample {
  gameplayFrameMs: number;
  movementMs: number;
  streamMs: number;
  meshMs: number;
  lodMs?: number;
  renderCpuMs: number;
}

export interface RouteFrameAccountingSummary {
  sampleCount: number;
  totalGameplayFrameMs: number;
  totalMovementMs: number;
  totalStreamMs: number;
  totalMeshMs: number;
  totalLodMs: number;
  totalRenderCpuMs: number;
  totalAccountedMs: number;
  totalUnmeasuredMs: number;
  avgGameplayFrameMs: number;
  p95GameplayFrameMs: number;
  maxGameplayFrameMs: number;
  avgMovementMs: number;
  p95MovementMs: number;
  maxMovementMs: number;
  avgUnmeasuredMs: number;
  p95UnmeasuredMs: number;
  maxUnmeasuredMs: number;
  avgMeasuredWorkMs: number;
  p95MeasuredWorkMs: number;
  maxMeasuredWorkMs: number;
}

export interface RouteSeamCoverageIssue {
  distanceMeters: number;
}

export interface RouteSeamCoverageProbe {
  uncoveredGapCount: number;
  handoffHoleCount: number;
  uncoveredGapSamples?: readonly RouteSeamCoverageIssue[];
  handoffHoleSamples?: readonly RouteSeamCoverageIssue[];
}

export interface RouteSeamCoverageSummary {
  seamGapCount: number;
  maxSeamGapMeters: number;
}

export function buildDefaultRouteBenchmarkPlan(
  startFeetPosition: Vec3,
  sampleFeetYAt: (worldX: number, worldZ: number) => number,
  options: RouteBenchmarkPlanOptions = {},
): RouteBenchmarkPlan {
  const durationSeconds = clampPositive(options.durationSeconds ?? 10, 10);
  const sampleHz = clampPositive(options.sampleHz ?? 60, 60);
  const speedMetersPerSecond = clampPositive(options.speedMetersPerSecond ?? 4.6, 4.6);
  const pitch = options.pitchRadians ?? -0.34;
  const frameCount = Math.max(1, Math.round(durationSeconds * sampleHz));
  const distancePerFrameWorldUnits = metersToWorldUnits(speedMetersPerSecond) / sampleHz;
  const segments = DEFAULT_ROUTE_SEGMENTS.map((segment) => ({
    yawRadians: segment.yawRadians,
    distanceWorldUnits: metersToWorldUnits(segment.distanceMeters),
    distanceMeters: segment.distanceMeters,
  }));

  let segmentIndex = 0;
  let segmentDistanceWorldUnits = 0;
  let totalDistanceMeters = 0;
  let worldX = startFeetPosition[0];
  let worldZ = startFeetPosition[2];
  const frames: RouteBenchmarkFrameTarget[] = [];

  for (let frame = 1; frame <= frameCount; frame += 1) {
    let remainingDistanceWorldUnits = distancePerFrameWorldUnits;
    while (remainingDistanceWorldUnits > 0) {
      const currentSegment = segments[Math.min(segmentIndex, segments.length - 1)]!;
      const segmentRemainingWorldUnits = currentSegment.distanceWorldUnits - segmentDistanceWorldUnits;
      const stepWorldUnits = Math.min(remainingDistanceWorldUnits, segmentRemainingWorldUnits);
      worldX += Math.cos(currentSegment.yawRadians) * stepWorldUnits;
      worldZ += Math.sin(currentSegment.yawRadians) * stepWorldUnits;
      segmentDistanceWorldUnits += stepWorldUnits;
      remainingDistanceWorldUnits -= stepWorldUnits;
      if (
        segmentIndex < segments.length - 1
        && segmentDistanceWorldUnits >= currentSegment.distanceWorldUnits - 1e-6
      ) {
        segmentIndex += 1;
        segmentDistanceWorldUnits = 0;
      } else if (segmentIndex >= segments.length - 1 && remainingDistanceWorldUnits > 0) {
        segmentDistanceWorldUnits = currentSegment.distanceWorldUnits;
      }
    }
    totalDistanceMeters += speedMetersPerSecond / sampleHz;
    frames.push({
      frame,
      phase: "move",
      simTimeSeconds: frame / sampleHz,
      distanceMeters: totalDistanceMeters,
      feetPosition: [worldX, sampleFeetYAt(worldX, worldZ), worldZ],
      yaw: segments[Math.min(segmentIndex, segments.length - 1)]!.yawRadians,
      pitch,
      segmentIndex,
    });
  }

  return {
    durationSeconds,
    sampleHz,
    speedMetersPerSecond,
    totalDistanceMeters,
    frames,
  };
}

export function buildForwardRouteBenchmarkPlan(
  startFeetPosition: Vec3,
  sampleFeetYAt: (worldX: number, worldZ: number) => number,
  options: RouteBenchmarkPlanOptions & {
    yawRadians?: number;
  } = {},
): RouteBenchmarkPlan {
  const durationSeconds = clampPositive(options.durationSeconds ?? 10, 10);
  const sampleHz = clampPositive(options.sampleHz ?? 60, 60);
  const speedMetersPerSecond = clampPositive(options.speedMetersPerSecond ?? 4.6, 4.6);
  const pitch = options.pitchRadians ?? -0.34;
  const yaw = options.yawRadians ?? 0;
  const frameCount = Math.max(1, Math.round(durationSeconds * sampleHz));
  const distancePerFrameWorldUnits = metersToWorldUnits(speedMetersPerSecond) / sampleHz;
  let totalDistanceMeters = 0;
  let worldX = startFeetPosition[0];
  let worldZ = startFeetPosition[2];
  const frames: RouteBenchmarkFrameTarget[] = [];

  for (let frame = 1; frame <= frameCount; frame += 1) {
    worldX += Math.cos(yaw) * distancePerFrameWorldUnits;
    worldZ += Math.sin(yaw) * distancePerFrameWorldUnits;
    totalDistanceMeters += speedMetersPerSecond / sampleHz;
    frames.push({
      frame,
      phase: "move",
      simTimeSeconds: frame / sampleHz,
      distanceMeters: totalDistanceMeters,
      feetPosition: [worldX, sampleFeetYAt(worldX, worldZ), worldZ],
      yaw,
      pitch,
      segmentIndex: 0,
    });
  }

  return {
    durationSeconds,
    sampleHz,
    speedMetersPerSecond,
    totalDistanceMeters,
    frames,
  };
}

export function analyzeBottomCenterVoid(
  image: {
    width: number;
    height: number;
    pixels: Uint8ClampedArray;
  },
  options: {
    xMinFraction?: number;
    xMaxFraction?: number;
    yMinFraction?: number;
    yMaxFraction?: number;
  } = {},
): BottomCenterVoidProbe {
  const xMin = Math.max(0, Math.floor(image.width * (options.xMinFraction ?? 0.2)));
  const xMaxExclusive = Math.min(image.width, Math.ceil(image.width * (options.xMaxFraction ?? 0.8)));
  const yMin = Math.max(0, Math.floor(image.height * (options.yMinFraction ?? 0.58)));
  const yMaxExclusive = Math.min(image.height, Math.ceil(image.height * (options.yMaxFraction ?? 0.94)));
  let sampleCount = 0;
  let clearPixelCount = 0;
  let maxClearRun = 0;

  for (let y = yMin; y < yMaxExclusive; y += 1) {
    let currentRun = 0;
    for (let x = xMin; x < xMaxExclusive; x += 1) {
      const index = (y * image.width + x) * 4;
      const clear = isClearPixel(
        image.pixels[index + 0] ?? 0,
        image.pixels[index + 1] ?? 0,
        image.pixels[index + 2] ?? 0,
      );
      sampleCount += 1;
      if (clear) {
        clearPixelCount += 1;
        currentRun += 1;
        maxClearRun = Math.max(maxClearRun, currentRun);
      } else {
        currentRun = 0;
      }
    }
  }

  const windowWidth = Math.max(1, xMaxExclusive - xMin);
  const clearRatio = sampleCount === 0 ? 0 : clearPixelCount / sampleCount;
  const maxClearRunRatio = maxClearRun / windowWidth;
  return {
    sampleCount,
    clearPixelCount,
    clearRatio,
    maxClearRun,
    maxClearRunRatio,
    suspicious: clearRatio >= 0.01 || maxClearRunRatio >= 0.12,
  };
}

export function summarizeRouteFrameAccounting(
  samples: readonly RouteFrameAccountingSample[],
): RouteFrameAccountingSummary {
  const gameplayFrameSamples = samples.map((sample) => sample.gameplayFrameMs);
  const movementSamples = samples.map((sample) => sample.movementMs);
  const streamSamples = samples.map((sample) => sample.streamMs);
  const meshSamples = samples.map((sample) => sample.meshMs);
  const lodSamples = samples.map((sample) => sample.lodMs ?? 0);
  const renderCpuSamples = samples.map((sample) => sample.renderCpuMs);
  const measuredWorkSamples = samples.map((sample) =>
    sample.movementMs
    + sample.streamMs
    + sample.meshMs
    + (sample.lodMs ?? 0)
    + sample.renderCpuMs);
  const unmeasuredSamples = gameplayFrameSamples.map((value, index) =>
    Math.max(0, value - (measuredWorkSamples[index] ?? 0)));
  return {
    sampleCount: samples.length,
    totalGameplayFrameMs: sum(gameplayFrameSamples),
    totalMovementMs: sum(movementSamples),
    totalStreamMs: sum(streamSamples),
    totalMeshMs: sum(meshSamples),
    totalLodMs: sum(lodSamples),
    totalRenderCpuMs: sum(renderCpuSamples),
    totalAccountedMs: sum(measuredWorkSamples),
    totalUnmeasuredMs: sum(unmeasuredSamples),
    avgGameplayFrameMs: average(gameplayFrameSamples),
    p95GameplayFrameMs: percentile(gameplayFrameSamples, 0.95),
    maxGameplayFrameMs: maxValue(gameplayFrameSamples),
    avgMovementMs: average(movementSamples),
    p95MovementMs: percentile(movementSamples, 0.95),
    maxMovementMs: maxValue(movementSamples),
    avgUnmeasuredMs: average(unmeasuredSamples),
    p95UnmeasuredMs: percentile(unmeasuredSamples, 0.95),
    maxUnmeasuredMs: maxValue(unmeasuredSamples),
    avgMeasuredWorkMs: average(measuredWorkSamples),
    p95MeasuredWorkMs: percentile(measuredWorkSamples, 0.95),
    maxMeasuredWorkMs: maxValue(measuredWorkSamples),
  };
}

export function summarizeRouteSeamCoverage(
  probe: RouteSeamCoverageProbe,
): RouteSeamCoverageSummary {
  const issueSamples = [
    ...(probe.uncoveredGapSamples ?? []),
    ...(probe.handoffHoleSamples ?? []),
  ];
  return {
    seamGapCount: Math.max(0, probe.uncoveredGapCount) + Math.max(0, probe.handoffHoleCount),
    maxSeamGapMeters: maxValue(issueSamples.map((sample) => sample.distanceMeters)),
  };
}

export function analyzeSettledReferenceDiff(
  transientImage: {
    width: number;
    height: number;
    pixels: Uint8ClampedArray;
  },
  settledImage: {
    width: number;
    height: number;
    pixels: Uint8ClampedArray;
  },
  options: {
    xMinFraction?: number;
    xMaxFraction?: number;
    yMinFraction?: number;
    yMaxFraction?: number;
    channelDeltaThreshold?: number;
  } = {},
): SettledReferenceDiffProbe {
  if (
    transientImage.width !== settledImage.width
    || transientImage.height !== settledImage.height
  ) {
    throw new Error("Transient and settled images must have matching dimensions.");
  }

  const width = transientImage.width;
  const height = transientImage.height;
  const xMin = Math.max(0, Math.floor(width * (options.xMinFraction ?? 0.08)));
  const xMaxExclusive = Math.min(width, Math.ceil(width * (options.xMaxFraction ?? 0.92)));
  const yMin = Math.max(0, Math.floor(height * (options.yMinFraction ?? 0.42)));
  const yMaxExclusive = Math.min(height, Math.ceil(height * (options.yMaxFraction ?? 0.96)));
  const channelDeltaThreshold = Math.max(0, options.channelDeltaThreshold ?? 18);
  let sampleCount = 0;
  let changedPixelCount = 0;
  let clearToFilledPixelCount = 0;
  let filledToClearPixelCount = 0;
  let maxClearToFilledRun = 0;

  for (let y = yMin; y < yMaxExclusive; y += 1) {
    let currentClearToFilledRun = 0;
    for (let x = xMin; x < xMaxExclusive; x += 1) {
      const index = (y * width + x) * 4;
      const transientR = transientImage.pixels[index + 0] ?? 0;
      const transientG = transientImage.pixels[index + 1] ?? 0;
      const transientB = transientImage.pixels[index + 2] ?? 0;
      const settledR = settledImage.pixels[index + 0] ?? 0;
      const settledG = settledImage.pixels[index + 1] ?? 0;
      const settledB = settledImage.pixels[index + 2] ?? 0;
      const delta = Math.abs(transientR - settledR)
        + Math.abs(transientG - settledG)
        + Math.abs(transientB - settledB);
      const transientClear = isClearPixel(transientR, transientG, transientB);
      const settledClear = isClearPixel(settledR, settledG, settledB);
      sampleCount += 1;
      if (delta >= channelDeltaThreshold) {
        changedPixelCount += 1;
      }
      if (transientClear && !settledClear) {
        clearToFilledPixelCount += 1;
        currentClearToFilledRun += 1;
        maxClearToFilledRun = Math.max(maxClearToFilledRun, currentClearToFilledRun);
      } else {
        currentClearToFilledRun = 0;
      }
      if (!transientClear && settledClear) {
        filledToClearPixelCount += 1;
      }
    }
  }

  const sampleWindowWidth = Math.max(1, xMaxExclusive - xMin);
  const changedRatio = sampleCount === 0 ? 0 : changedPixelCount / sampleCount;
  const clearToFilledRatio = sampleCount === 0 ? 0 : clearToFilledPixelCount / sampleCount;
  const filledToClearRatio = sampleCount === 0 ? 0 : filledToClearPixelCount / sampleCount;
  const maxClearToFilledRunRatio = maxClearToFilledRun / sampleWindowWidth;
  return {
    sampleCount,
    changedPixelCount,
    changedRatio,
    clearToFilledPixelCount,
    clearToFilledRatio,
    filledToClearPixelCount,
    filledToClearRatio,
    maxClearToFilledRun,
    maxClearToFilledRunRatio,
    suspiciousHole: clearToFilledRatio >= 0.008 || maxClearToFilledRunRatio >= 0.1,
  };
}

function isClearPixel(r: number, g: number, b: number): boolean {
  return r === CLEAR_COLOR_RGBA[0] && g === CLEAR_COLOR_RGBA[1] && b === CLEAR_COLOR_RGBA[2];
}

function clampPositive(value: number, fallback: number): number {
  return Number.isFinite(value) && value > 0 ? value : fallback;
}

function sum(values: readonly number[]): number {
  let total = 0;
  for (const value of values) {
    total += value;
  }
  return total;
}
