import { execFileSync } from "node:child_process";
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { chromium } from "playwright";
import {
  assertSnapshotSchema,
  chromeWebGpuLaunchOptions,
  FRAME_SAMPLE_START,
  FRAME_SAMPLE_WIDTH,
  gpuSampleStart,
  GPU_SAMPLE_WIDTH,
  isBrowserConsoleFailure,
  reserveEphemeralPort,
  SNAPSHOT,
} from "./browser-harness.mjs";
import { prepareBrowserWorldFixture, startBrowserWorldService } from "./browser-world-fixture.mjs";

const FAILURE =
  /panic|unreachable|runtimeerror|wgpu|webgpu|shader|sqlite|opfs|syncaccesshandle|nomodificationallowed|web lock request failed|no persistence leader|persistence .*failed/i;
const WATERTIGHT = process.argv.includes("--watertight");
const SOURCE = process.env.VOXELS_LOD_TEST_SOURCE ?? "terrain-diffusion-30m";
const SPAWN = (process.env.VOXELS_LOD_TEST_SPAWN ?? (WATERTIGHT ? "4194,6034" : "4208,6082"))
  .split(",")
  .map((value) => Number.parseInt(value.trim(), 10));
const LOOK = (process.env.VOXELS_LOD_TEST_LOOK ?? "2.074606,-0.371797")
  .split(",")
  .map((value) => Number.parseFloat(value.trim()));
const VIEWPORT = { width: 1280, height: 720 };
const OUTPUT_DIRECTORY = path.resolve(
  process.env.VOXELS_LOD_TEST_OUTPUT ??
    (WATERTIGHT ? "target/lod-watertight" : "target/lod-transition"),
);

if (SPAWN.length !== 2 || !SPAWN.every(Number.isInteger)) {
  throw new Error("VOXELS_LOD_TEST_SPAWN must be two comma-separated canonical voxel coordinates");
}
if (LOOK.length !== 2 || !LOOK.every(Number.isFinite)) {
  throw new Error("VOXELS_LOD_TEST_LOOK must be comma-separated yaw,pitch radians");
}

function percentile(values, fraction) {
  if (values.length === 0) return 0;
  const sorted = values.toSorted((left, right) => left - right);
  return sorted[Math.min(sorted.length - 1, Math.ceil(sorted.length * fraction) - 1)];
}

function boundaryCentres(snapshot) {
  return Array.from({ length: 6 }, (_, index) => [
    snapshot[SNAPSHOT.lodBoundary0X + index * 2],
    snapshot[SNAPSHOT.lodBoundary0Z + index * 2],
  ]);
}

function sameCentres(left, right) {
  return left.every(
    (centre, index) => centre[0] === right[index][0] && centre[1] === right[index][1],
  );
}

function cameraPosition(snapshot) {
  return [snapshot[SNAPSHOT.cameraX], snapshot[SNAPSHOT.cameraY], snapshot[SNAPSHOT.cameraZ]];
}

function planarDistance(left, right) {
  return Math.hypot(left[0] - right[0], left[2] - right[2]);
}

function spatialDistance(left, right) {
  return Math.hypot(left[0] - right[0], left[1] - right[1], left[2] - right[2]);
}

function collectTiming(snapshot, timings) {
  const sampleCount = snapshot[SNAPSHOT.sampleCount] ?? 0;
  for (let index = 0; index < sampleCount; index += 1) {
    const start = FRAME_SAMPLE_START + index * FRAME_SAMPLE_WIDTH;
    timings.frameIntervals.push(snapshot[start]);
  }
  const start = gpuSampleStart(snapshot);
  const gpuCount = snapshot[start] ?? 0;
  for (let index = 0; index < gpuCount; index += 1) {
    const sample = start + 2 + index * GPU_SAMPLE_WIDTH;
    timings.gpu.set(snapshot[sample], {
      total: snapshot[sample + 1],
      world: snapshot[sample + 8],
    });
  }
}

function resetTimings(timings) {
  timings.frameIntervals.length = 0;
  timings.gpu.clear();
}

async function sampleStablePerformance(page, timings, duration) {
  await readSnapshot(page, timings);
  resetTimings(timings);
  const deadline = Date.now() + duration;
  while (Date.now() < deadline) {
    await page.waitForTimeout(100);
    await readSnapshot(page, timings);
  }
}

async function readSnapshot(page, timings) {
  const snapshot = assertSnapshotSchema(
    await page.evaluate(() => globalThis.__VOXELS__.snapshot()),
  );
  collectTiming(snapshot, timings);
  return snapshot;
}

async function waitForEngine(page, timings) {
  await page.waitForFunction(() => typeof globalThis.__VOXELS__?.snapshot === "function", null, {
    timeout: 20_000,
  });
  const deadline = Date.now() + 60_000;
  let latest = [];
  while (Date.now() < deadline) {
    latest = await readSnapshot(page, timings);
    if (
      latest[SNAPSHOT.quads] > 0 &&
      latest[SNAPSHOT.pendingJobs] === 0 &&
      latest[SNAPSHOT.surfaceInFlight] === 0 &&
      latest[SNAPSHOT.allLodsReady] === 1 &&
      latest[SNAPSHOT.lodTransitionQuads] > 0
    ) {
      return latest;
    }
    await page.waitForTimeout(25);
  }
  throw new Error(`LOD browser fixture did not settle: ${JSON.stringify(latest)}`);
}

async function setCameraLook(page, targetYaw, targetPitch, timings) {
  const sensitivity = 0.0022;
  const current = await readSnapshot(page, timings);
  const yawDelta = Math.atan2(
    Math.sin(targetYaw - current[SNAPSHOT.yaw]),
    Math.cos(targetYaw - current[SNAPSHOT.yaw]),
  );
  await page.evaluate(({ x, y }) => globalThis.__VOXELS__.look(x, y), {
    x: yawDelta / sensitivity,
    y: (current[SNAPSHOT.pitch] - targetPitch) / sensitivity,
  });
  const deadline = Date.now() + 5_000;
  while (Date.now() < deadline) {
    const snapshot = await readSnapshot(page, timings);
    const yawError = Math.atan2(
      Math.sin(snapshot[SNAPSHOT.yaw] - targetYaw),
      Math.cos(snapshot[SNAPSHOT.yaw] - targetYaw),
    );
    if (Math.abs(yawError) < 0.001 && Math.abs(snapshot[SNAPSHOT.pitch] - targetPitch) < 0.001) {
      return snapshot;
    }
    await page.waitForTimeout(10);
  }
  throw new Error("camera look did not reach the requested regression pose");
}

async function waitForCentreChange(page, initialCentres, outboundKey, timings) {
  const deadline = Date.now() + 4_000;
  await page.keyboard.down(outboundKey);
  try {
    while (Date.now() < deadline) {
      const snapshot = await readSnapshot(page, timings);
      if (!sameCentres(boundaryCentres(snapshot), initialCentres)) return snapshot;
      await page.waitForTimeout(8);
    }
  } finally {
    await page.keyboard.up(outboundKey);
  }
  throw new Error("walking forward did not cross an LOD snap boundary");
}

async function returnToPose(page, target, initialCentres, timings) {
  const correctionHistory = [];
  const brake = async (duration) => {
    const keys = ["KeyW", "KeyS", "KeyA", "KeyD"];
    for (const key of keys) await page.keyboard.down(key);
    try {
      const deadline = Date.now() + duration;
      while (Date.now() < deadline) {
        await readSnapshot(page, timings);
        await page.waitForTimeout(16);
      }
    } finally {
      for (const key of keys) await page.keyboard.up(key);
    }
  };
  await brake(300);
  let latest = await readSnapshot(page, timings);
  for (let correction = 0; correction < 80; correction += 1) {
    const position = cameraPosition(latest);
    const distance = planarDistance(position, target);
    correctionHistory.push({ correction, distance, position });
    if (distance <= 0.008) {
      await brake(200);
      latest = await readSnapshot(page, timings);
      if (planarDistance(cameraPosition(latest), target) <= 0.015) break;
      continue;
    }
    const yaw = latest[SNAPSHOT.yaw];
    const forward = [Math.sin(yaw), -Math.cos(yaw)];
    const right = [-forward[1], forward[0]];
    const error = [target[0] - position[0], target[2] - position[2]];
    const candidates = [
      ["KeyW", forward],
      ["KeyS", forward.map((value) => -value)],
      ["KeyD", right],
      ["KeyA", right.map((value) => -value)],
    ];
    const [key] = candidates.toSorted(
      (left, rightCandidate) =>
        rightCandidate[1][0] * error[0] +
        rightCandidate[1][1] * error[1] -
        (left[1][0] * error[0] + left[1][1] * error[1]),
    )[0];
    await page.keyboard.down(key);
    try {
      const deadline = Date.now() + Math.min(40, Math.max(8, distance * 80));
      while (Date.now() < deadline) {
        latest = await readSnapshot(page, timings);
        await page.waitForTimeout(4);
      }
    } finally {
      await page.keyboard.up(key);
    }
    await brake(48);
    latest = await readSnapshot(page, timings);
  }
  const error = planarDistance(cameraPosition(latest), target);
  if (error > 0.015) {
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

async function waitForStableFrame(page, expectedCentres, timings) {
  let latest;
  let previousPosition;
  let stable = 0;
  const deadline = Date.now() + 10_000;
  while (stable < 12 && Date.now() < deadline) {
    latest = await readSnapshot(page, timings);
    const position = cameraPosition(latest);
    const settled =
      sameCentres(boundaryCentres(latest), expectedCentres) &&
      latest[SNAPSHOT.grounded] === 1 &&
      previousPosition !== undefined &&
      spatialDistance(position, previousPosition) < 0.0015 &&
      latest[SNAPSHOT.pendingJobs] === 0 &&
      latest[SNAPSHOT.surfaceInFlight] === 0 &&
      latest[SNAPSHOT.allLodsReady] === 1 &&
      latest[SNAPSHOT.lodTransitionQuads] > 0;
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

async function waitForStableChangedFrame(page, initialCentres, timings) {
  let latest;
  let latestCentres;
  let previousCentres;
  let previousPosition;
  let stable = 0;
  const deadline = Date.now() + 10_000;
  while (stable < 12 && Date.now() < deadline) {
    latest = await readSnapshot(page, timings);
    latestCentres = boundaryCentres(latest);
    const position = cameraPosition(latest);
    const settled =
      !sameCentres(latestCentres, initialCentres) &&
      latest[SNAPSHOT.grounded] === 1 &&
      previousPosition !== undefined &&
      spatialDistance(position, previousPosition) < 0.0015 &&
      latest[SNAPSHOT.pendingJobs] === 0 &&
      latest[SNAPSHOT.surfaceInFlight] === 0 &&
      latest[SNAPSHOT.allLodsReady] === 1 &&
      latest[SNAPSHOT.lodTransitionQuads] > 0;
    stable =
      settled && previousCentres && sameCentres(latestCentres, previousCentres) ? stable + 1 : 0;
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

async function compareScreenshots(page, before, after) {
  return page.evaluate(
    async ({ beforeBase64, afterBase64 }) => {
      const decode = async (base64) => {
        const response = await fetch(`data:image/png;base64,${base64}`);
        const bitmap = await createImageBitmap(await response.blob());
        const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
        const context = canvas.getContext("2d", { willReadFrequently: true });
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
      const linear = (value) => {
        const channel = value / 255;
        return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
      };
      const luma = (pixels, index) =>
        0.2126 * linear(pixels[index]) +
        0.7152 * linear(pixels[index + 1]) +
        0.0722 * linear(pixels[index + 2]);
      let count = 0;
      let sumLeft = 0;
      let sumRight = 0;
      let sumLeftSquared = 0;
      let sumRightSquared = 0;
      let sumProduct = 0;
      let sumAbsolute = 0;
      let catastrophic = 0;
      for (let y = roi.y0; y < roi.y1; y += 1) {
        for (let x = roi.x0; x < roi.x1; x += 1) {
          const index = (x + y * left.width) * 4;
          const leftLuma = luma(left.pixels, index);
          const rightLuma = luma(right.pixels, index);
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
      return {
        roi,
        pixels: count,
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

async function analyzeWatertightTerrain(page, screenshot) {
  return page.evaluate(async (base64) => {
    const response = await fetch(`data:image/png;base64,${base64}`);
    const bitmap = await createImageBitmap(await response.blob());
    const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
    const context = canvas.getContext("2d", { willReadFrequently: true });
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
    let skyLikePixels = 0;
    for (let y = roi.y0; y < roi.y1; y += 1) {
      for (let x = roi.x0; x < roi.x1; x += 1) {
        const source = (x + y * bitmap.width) * 4;
        const red = pixels[source];
        const green = pixels[source + 1];
        const blue = pixels[source + 2];
        const exposedSky = red > 25 && red > green * 1.35 && red > blue * 1.08;
        if (!exposedSky) continue;
        skyLike[x - roi.x0 + (y - roi.y0) * width] = 1;
        skyLikePixels += 1;
      }
    }
    const visited = new Uint8Array(skyLike.length);
    let largestSkyLikeComponent = 0;
    for (let start = 0; start < skyLike.length; start += 1) {
      if (skyLike[start] === 0 || visited[start] !== 0) continue;
      const stack = [start];
      visited[start] = 1;
      let component = 0;
      while (stack.length > 0) {
        const current = stack.pop();
        component += 1;
        const x = current % width;
        const y = Math.floor(current / width);
        const neighbors = [
          x > 0 ? current - 1 : -1,
          x + 1 < width ? current + 1 : -1,
          y > 0 ? current - width : -1,
          y + 1 < height ? current + width : -1,
        ];
        for (const neighbor of neighbors) {
          if (neighbor < 0 || skyLike[neighbor] === 0 || visited[neighbor] !== 0) continue;
          visited[neighbor] = 1;
          stack.push(neighbor);
        }
      }
      largestSkyLikeComponent = Math.max(largestSkyLikeComponent, component);
    }
    const sampledPixels = skyLike.length;
    return {
      roi,
      sampledPixels,
      skyLikePixels,
      skyLikeFraction: skyLikePixels / sampledPixels,
      largestSkyLikeComponent,
      largestSkyLikeFraction: largestSkyLikeComponent / sampledPixels,
    };
  }, screenshot.toString("base64"));
}

function summarizePerformance(timings) {
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

const timings = { frameIntervals: [], gpu: new Map() };
const errors = [];
const port = await reserveEphemeralPort();
let browser;
let server;
let fixture;
let worldService;

try {
  await mkdir(OUTPUT_DIRECTORY, { recursive: true });
  fixture = await prepareBrowserWorldFixture({
    browserPort: port,
    prefix: "voxels-lod-transition-",
    source: SOURCE,
    spawnVoxels: SPAWN,
    cascadedShadows: true,
    screenSpaceAmbientOcclusion: true,
    dayLengthSeconds: 0,
    dayFractionAtUnixEpoch: 0.5,
    weatherCycleSeconds: 0,
    weatherFractionAtUnixEpoch: 0.08,
    cloudVelocityMetresPerSecond: [0, 0],
  });
  process.env.VOXELS_BROWSER_BUILD_PROFILE = process.env.VOXELS_LOD_TEST_BUILD ?? "release";
  const { build, preview } = await import("vite-plus");
  await build({ logLevel: "warn" });
  worldService = await startBrowserWorldService(fixture, {
    metal: SOURCE === "terrain-diffusion-30m",
  });
  server = await preview({
    logLevel: "warn",
    preview: { host: "127.0.0.1", port, strictPort: true },
  });
  browser = await chromium.launch(chromeWebGpuLaunchOptions());
  const context = await browser.newContext({ viewport: VIEWPORT, deviceScaleFactor: 1 });
  const page = await context.newPage();
  page.on("pageerror", (error) => errors.push(`pageerror: ${error.message}`));
  page.on("console", (message) => {
    if (isBrowserConsoleFailure(message.type(), message.text(), FAILURE)) {
      errors.push(`${message.type()}: ${message.text()}`);
    }
  });
  await page.goto(`http://127.0.0.1:${port}`, { waitUntil: "domcontentloaded" });
  let beforeSnapshot = await waitForEngine(page, timings);
  beforeSnapshot = await setCameraLook(page, LOOK[0], LOOK[1], timings);
  const initialCentres = boundaryCentres(beforeSnapshot);
  beforeSnapshot = await waitForStableFrame(page, initialCentres, timings);
  const beforePose = cameraPosition(beforeSnapshot);
  const before = await page.screenshot({ path: path.join(OUTPUT_DIRECTORY, "before.png") });

  if (WATERTIGHT) {
    await sampleStablePerformance(page, timings, 2_000);
    const image = await analyzeWatertightTerrain(page, before);
    const performance = summarizePerformance(timings);
    const violations = [];
    if (image.skyLikeFraction > 0.001)
      violations.push("terrain-only ROI exposed more than 0.1% sky-colored pixels");
    if (image.largestSkyLikeComponent > 32)
      violations.push("terrain-only ROI contains a connected sky-colored crack");
    if (performance.frameP95Ms > 12) violations.push("frame p95 exceeded 12ms");
    if (performance.fractionAbove16_67Ms > 0.01)
      violations.push("over 1% of measured frames exceeded 16.67ms");
    if (performance.frameMaxMs > 25) violations.push("a measured frame exceeded 25ms");
    if (performance.worldGpuP95Ms > 2) violations.push("world GPU p95 exceeded 2ms");
    if (performance.totalGpuP95Ms > 7.5) violations.push("total GPU p95 exceeded 7.5ms");
    if (errors.length > 0) violations.push(...errors);
    const result = {
      ok: violations.length === 0,
      mode: "watertight",
      commit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(),
      dirty: execFileSync("git", ["status", "--porcelain"], { encoding: "utf8" }).trim() !== "",
      source: SOURCE,
      spawnVoxels: SPAWN,
      look: LOOK,
      browser: browser.version(),
      pose: beforePose,
      lod: {
        centres: initialCentres,
        transitionQuads: beforeSnapshot[SNAPSHOT.lodTransitionQuads],
        viewportFingerprint: [
          beforeSnapshot[SNAPSHOT.viewportFingerprintLow24],
          beforeSnapshot[SNAPSHOT.viewportFingerprintHigh24],
        ],
      },
      image,
      performance,
      violations,
    };
    await writeFile(
      path.join(OUTPUT_DIRECTORY, "report.json"),
      `${JSON.stringify(result, null, 2)}\n`,
    );
    console.log(JSON.stringify(result, null, 2));
    if (!result.ok) process.exitCode = 1;
  } else {
    const cameraVoxelX = beforePose[0] / 0.1;
    const desiredXDirection = Math.sign(cameraVoxelX - initialCentres[0][0]) || 1;
    const forwardXDirection = Math.sign(Math.sin(beforeSnapshot[SNAPSHOT.yaw])) || 1;
    const outboundKey = desiredXDirection === forwardXDirection ? "KeyW" : "KeyS";
    const crossedSnapshot = await waitForCentreChange(page, initialCentres, outboundKey, timings);
    const crossedPose = cameraPosition(crossedSnapshot);
    if (planarDistance(crossedPose, beforePose) <= 0) {
      throw new Error("LOD focus changed without measurable player movement");
    }
    await returnToPose(page, beforePose, initialCentres, timings);
    const afterSnapshot = await waitForStableChangedFrame(page, initialCentres, timings);
    const afterCentres = boundaryCentres(afterSnapshot);
    const afterPose = cameraPosition(afterSnapshot);
    const after = await page.screenshot({ path: path.join(OUTPUT_DIRECTORY, "after.png") });
    await sampleStablePerformance(page, timings, 2_000);
    const image = await compareScreenshots(page, before, after);
    const performance = summarizePerformance(timings);
    const planarPoseErrorMetres = planarDistance(beforePose, afterPose);
    const poseErrorMetres = spatialDistance(beforePose, afterPose);
    const violations = [];
    // Ground height follows the returned X/Z position. A few centimetres on a steep voxel slope
    // can legitimately move Y farther, while the screenshots remain horizontally registered.
    if (planarPoseErrorMetres > 0.015)
      violations.push("camera did not return to the same horizontal pose");
    if (image.relativeMeanLumaDelta > 0.04)
      violations.push("valley mean luminance changed by over 4%");
    if (image.meanAbsoluteLinearLumaDelta > 0.025)
      violations.push("valley mean absolute luminance delta exceeded 0.025");
    if (image.catastrophicDarkFraction > 0.01)
      violations.push("over 1% of valley pixels changed luminance by at least 2x");
    if (image.ssim < 0.97) violations.push("valley SSIM fell below 0.97");
    if (performance.frameP95Ms > 12) violations.push("frame p95 exceeded 12ms");
    if (performance.fractionAbove16_67Ms > 0.01)
      violations.push("over 1% of measured frames exceeded 16.67ms");
    if (performance.frameMaxMs > 25) violations.push("a measured frame exceeded 25ms");
    if (performance.worldGpuP95Ms > 2) violations.push("world GPU p95 exceeded 2ms");
    if (performance.totalGpuP95Ms > 7.5) violations.push("total GPU p95 exceeded 7.5ms");
    if (errors.length > 0) violations.push(...errors);

    const result = {
      ok: violations.length === 0,
      mode: "transition",
      commit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(),
      dirty: execFileSync("git", ["status", "--porcelain"], { encoding: "utf8" }).trim() !== "",
      source: SOURCE,
      spawnVoxels: SPAWN,
      look: LOOK,
      browser: browser.version(),
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
        transitionQuadsBefore: beforeSnapshot[SNAPSHOT.lodTransitionQuads],
        transitionQuadsAfter: afterSnapshot[SNAPSHOT.lodTransitionQuads],
        viewportFingerprintBefore: [
          beforeSnapshot[SNAPSHOT.viewportFingerprintLow24],
          beforeSnapshot[SNAPSHOT.viewportFingerprintHigh24],
        ],
        viewportFingerprintAfter: [
          afterSnapshot[SNAPSHOT.viewportFingerprintLow24],
          afterSnapshot[SNAPSHOT.viewportFingerprintHigh24],
        ],
      },
      image,
      performance,
      violations,
    };
    await writeFile(
      path.join(OUTPUT_DIRECTORY, "report.json"),
      `${JSON.stringify(result, null, 2)}\n`,
    );
    console.log(JSON.stringify(result, null, 2));
    if (!result.ok) process.exitCode = 1;
  }
} catch (error) {
  console.error(JSON.stringify({ ok: false, error: String(error), errors }, null, 2));
  process.exitCode = 1;
} finally {
  await browser?.close();
  await server?.close();
  await worldService?.close();
  await fixture?.cleanup();
}
