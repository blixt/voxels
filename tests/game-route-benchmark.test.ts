import { expect, test } from "bun:test";

import {
  analyzeBottomCenterVoid,
  buildDefaultRouteBenchmarkPlan,
  summarizeRouteFrameAccounting,
} from "../src/engine/game-route-benchmark.ts";
import { metersToWorldUnits } from "../src/engine/scale.ts";

test("default route benchmark plan stays within the requested per-frame travel budget", () => {
  const start: [number, number, number] = [100, 250, -40];
  const plan = buildDefaultRouteBenchmarkPlan(
    start,
    () => 250,
    {
      durationSeconds: 2,
      sampleHz: 20,
      speedMetersPerSecond: 4,
    },
  );

  expect(plan.frames).toHaveLength(40);
  expect(plan.totalDistanceMeters).toBeCloseTo(8, 6);
  const maxStepWorldUnits = metersToWorldUnits(4 / 20);
  let previous = start;
  for (const frame of plan.frames) {
    const delta = Math.hypot(
      frame.feetPosition[0] - previous[0],
      frame.feetPosition[2] - previous[2],
    );
    expect(delta).toBeLessThanOrEqual(maxStepWorldUnits + 1e-6);
    expect(frame.feetPosition[1]).toBe(250);
    previous = frame.feetPosition;
  }
});

test("bottom-center void analysis ignores clear pixels outside the sampled window", () => {
  const width = 10;
  const height = 10;
  const pixels = new Uint8ClampedArray(width * height * 4).fill(0);
  for (let index = 0; index < pixels.length; index += 4) {
    pixels[index + 0] = 40;
    pixels[index + 1] = 50;
    pixels[index + 2] = 60;
    pixels[index + 3] = 255;
  }
  for (let y = 0; y < 3; y += 1) {
    for (let x = 0; x < width; x += 1) {
      const index = (y * width + x) * 4;
      pixels[index + 0] = 209;
      pixels[index + 1] = 224;
      pixels[index + 2] = 240;
    }
  }

  const probe = analyzeBottomCenterVoid({ width, height, pixels });

  expect(probe.clearRatio).toBe(0);
  expect(probe.suspicious).toBeFalse();
});

test("bottom-center void analysis flags a clear-color slit in the lower center window", () => {
  const width = 20;
  const height = 20;
  const pixels = new Uint8ClampedArray(width * height * 4).fill(0);
  for (let index = 0; index < pixels.length; index += 4) {
    pixels[index + 0] = 40;
    pixels[index + 1] = 50;
    pixels[index + 2] = 60;
    pixels[index + 3] = 255;
  }
  for (let y = 12; y < 18; y += 1) {
    for (let x = 7; x < 13; x += 1) {
      const index = (y * width + x) * 4;
      pixels[index + 0] = 209;
      pixels[index + 1] = 224;
      pixels[index + 2] = 240;
    }
  }

  const probe = analyzeBottomCenterVoid({ width, height, pixels });

  expect(probe.clearRatio).toBeGreaterThan(0.1);
  expect(probe.maxClearRunRatio).toBeGreaterThan(0.12);
  expect(probe.suspicious).toBeTrue();
});

test("route frame accounting exposes measured and unmeasured time explicitly", () => {
  const summary = summarizeRouteFrameAccounting([
    {
      gameplayFrameMs: 12,
      movementMs: 1,
      streamMs: 2,
      meshMs: 3,
      farFieldMs: 1,
      renderCpuMs: 4,
    },
    {
      gameplayFrameMs: 9,
      movementMs: 1,
      streamMs: 1,
      meshMs: 1,
      farFieldMs: 1,
      renderCpuMs: 2,
    },
  ]);

  expect(summary.totalGameplayFrameMs).toBe(21);
  expect(summary.totalAccountedMs).toBe(17);
  expect(summary.totalUnmeasuredMs).toBe(4);
  expect(summary.maxUnmeasuredMs).toBe(3);
});
