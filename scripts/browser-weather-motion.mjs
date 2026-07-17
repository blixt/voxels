import { execFileSync } from "node:child_process";
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { chromium } from "playwright";
import {
  assertSnapshotSchema,
  chromeWebGpuLaunchOptions,
  isBrowserConsoleFailure,
  reserveEphemeralPort,
  SNAPSHOT,
} from "./browser-harness.mjs";
import { prepareBrowserWorldFixture, startBrowserWorldService } from "./browser-world-fixture.mjs";

const FAILURE =
  /panic|unreachable|runtimeerror|wgpu|webgpu|shader|sqlite|opfs|syncaccesshandle|nomodificationallowed|web lock request failed|no persistence leader|persistence .*failed/i;
const VIEWPORT = { width: 1280, height: 720 };
const OUTPUT_DIRECTORY = path.resolve(
  process.env.VOXELS_WEATHER_MOTION_OUTPUT ?? "target/weather-motion",
);
const WEATHER_CYCLE_SECONDS = 120;
const CLOUD_LAYERING_ONLY = process.env.VOXELS_CLOUD_LAYERING_ONLY === "1";

function wrappedFraction(value) {
  return ((value % 1) + 1) % 1;
}

function median(values) {
  const sorted = values.toSorted((left, right) => left - right);
  return sorted[Math.floor(sorted.length / 2)] ?? 0;
}

async function waitForEngine(page) {
  await page.waitForFunction(
    () => typeof globalThis.__VOXELS__?.snapshot === "function",
    undefined,
    { timeout: 20_000 },
  );
  const deadline = performance.now() + 60_000;
  let latest;
  while (performance.now() < deadline) {
    latest = assertSnapshotSchema(await page.evaluate(() => globalThis.__VOXELS__.snapshot()));
    if (
      latest[SNAPSHOT.allLodsReady] === 1 &&
      latest[SNAPSHOT.pendingJobs] === 0 &&
      latest[SNAPSHOT.residentChunks] > 0
    ) {
      return latest;
    }
    await page.waitForTimeout(50);
  }
  throw new Error(`weather motion fixture did not settle: ${JSON.stringify(latest)}`);
}

async function waitForWeatherFraction(page, target, tolerance = 0.008) {
  const deadline = performance.now() + WEATHER_CYCLE_SECONDS * 1_100;
  while (performance.now() < deadline) {
    const snapshot = assertSnapshotSchema(
      await page.evaluate(() => globalThis.__VOXELS__.snapshot()),
    );
    const distance = Math.abs(snapshot[SNAPSHOT.weatherFraction] - target);
    if (Math.min(distance, 1 - distance) <= tolerance) return snapshot;
    await page.waitForTimeout(25);
  }
  throw new Error(`timed out waiting for weather fraction ${target}`);
}

async function setCameraLook(page, targetYaw, targetPitch) {
  const sensitivity = 0.0022;
  const current = await page.evaluate(() => globalThis.__VOXELS__.snapshot());
  const wrappedYawDelta = Math.atan2(
    Math.sin(targetYaw - current[SNAPSHOT.yaw]),
    Math.cos(targetYaw - current[SNAPSHOT.yaw]),
  );
  await page.evaluate(({ deltaX, deltaY }) => globalThis.__VOXELS__.look(deltaX, deltaY), {
    deltaX: wrappedYawDelta / sensitivity,
    deltaY: (current[SNAPSHOT.pitch] - targetPitch) / sensitivity,
  });
  await page.waitForFunction(
    async ({ yaw, pitch, yawIndex, pitchIndex }) => {
      const snapshot = await globalThis.__VOXELS__.snapshot();
      const yawError = Math.atan2(
        Math.sin(snapshot[yawIndex] - yaw),
        Math.cos(snapshot[yawIndex] - yaw),
      );
      return Math.abs(yawError) < 0.001 && Math.abs(snapshot[pitchIndex] - pitch) < 0.001;
    },
    { yaw: targetYaw, pitch: targetPitch, yawIndex: SNAPSHOT.yaw, pitchIndex: SNAPSHOT.pitch },
    { timeout: 5_000 },
  );
}

async function captureBurst(page, count, intervalMilliseconds, prefix) {
  const frames = [];
  for (let index = 0; index < count; index += 1) {
    const started = performance.now();
    const png = await page.screenshot();
    const finished = performance.now();
    frames.push({
      base64: png.toString("base64"),
      midpointMilliseconds: (started + finished) * 0.5,
    });
    if (index === 0 || index === Math.floor(count / 2) || index === count - 1) {
      await writeFile(path.join(OUTPUT_DIRECTORY, `${prefix}-${index}.png`), png);
    }
    if (index + 1 < count) await page.waitForTimeout(intervalMilliseconds);
  }
  return frames;
}

async function analyzeRainMotion(page, frames) {
  return page.evaluate(async (serializedFrames) => {
    const decoded = [];
    for (const frame of serializedFrames) {
      const response = await fetch(`data:image/png;base64,${frame.base64}`);
      const bitmap = await createImageBitmap(await response.blob());
      const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
      const context = canvas.getContext("2d", { willReadFrequently: true });
      context.drawImage(bitmap, 0, 0);
      decoded.push({
        width: bitmap.width,
        height: bitmap.height,
        pixels: context.getImageData(0, 0, bitmap.width, bitmap.height).data,
        midpointMilliseconds: frame.midpointMilliseconds,
      });
    }
    const step = 4;
    const x0 = Math.floor(decoded[0].width * 0.08);
    const x1 = Math.floor(decoded[0].width * 0.78);
    const y0 = Math.floor(decoded[0].height * 0.05);
    const y1 = Math.floor(decoded[0].height * 0.82);
    const width = Math.floor((x1 - x0) / step);
    const height = Math.floor((y1 - y0) / step);
    const lumaFrames = decoded.map((frame) => {
      const luma = new Float32Array(width * height);
      for (let y = 0; y < height; y += 1) {
        for (let x = 0; x < width; x += 1) {
          const source = (x0 + x * step + (y0 + y * step) * frame.width) * 4;
          const red = frame.pixels[source] / 255;
          const green = frame.pixels[source + 1] / 255;
          const blue = frame.pixels[source + 2] / 255;
          luma[x + y * width] = red * 0.2126 + green * 0.7152 + blue * 0.0722;
        }
      }
      return luma;
    });
    const background = new Float32Array(width * height);
    for (let index = 0; index < background.length; index += 1) {
      const values = lumaFrames.map((frame) => frame[index]).sort((left, right) => left - right);
      background[index] = values[Math.floor(values.length / 2)];
    }
    const signals = lumaFrames.map((luma) => {
      const signal = new Float32Array(luma.length);
      for (let index = 0; index < luma.length; index += 1) {
        // Rain may either brighten a dark surface or attenuate a bright sky. Measuring only
        // positive luminance changes made the motion gate blind to physically tinted streaks
        // and allowed unrelated highlights to dominate the correlation.
        // A one-pixel physically tinted streak is often sub-pixel after the four-pixel
        // analysis decimation. Keep that weak coverage in the signal; the normalized
        // temporal correlation below rejects stationary terrain and sky detail.
        signal[index] = Math.max(Math.abs(luma[index] - background[index]) - 0.006, 0);
      }
      return signal;
    });
    const pairs = [];
    for (let frameIndex = 0; frameIndex + 1 < signals.length; frameIndex += 1) {
      const first = signals[frameIndex];
      const second = signals[frameIndex + 1];
      let best = { score: -1, dx: 0, dy: 0 };
      let bestNegative = -1;
      let bestPositive = -1;
      for (let dy = -30; dy <= 30; dy += 1) {
        for (let dx = -3; dx <= 3; dx += 1) {
          let product = 0;
          let firstEnergy = 0;
          let secondEnergy = 0;
          for (let y = 30; y < height - 30; y += 1) {
            for (let x = 3; x < width - 3; x += 1) {
              const left = first[x + y * width];
              const right = second[x + dx + (y + dy) * width];
              product += left * right;
              firstEnergy += left * left;
              secondEnergy += right * right;
            }
          }
          const score = product / Math.max(Math.sqrt(firstEnergy * secondEnergy), 0.000001);
          if (score > best.score) best = { score, dx, dy };
          if (dy < 0) bestNegative = Math.max(bestNegative, score);
          if (dy > 0) bestPositive = Math.max(bestPositive, score);
        }
      }
      const elapsedSeconds =
        (decoded[frameIndex + 1].midpointMilliseconds - decoded[frameIndex].midpointMilliseconds) /
        1_000;
      pairs.push({
        ...best,
        dxPixels: best.dx * step,
        dyPixels: best.dy * step,
        velocityYPixelsPerSecond: (best.dy * step) / elapsedSeconds,
        positiveAdvantage: bestPositive - bestNegative,
      });
    }
    const occupied = signals.map(
      (signal) => signal.filter((value) => value > 0).length / signal.length,
    );
    return { roi: { x0, x1, y0, y1, step }, occupied, pairs };
  }, frames);
}

async function analyzeCloudRotation(page, baselineFrames, rotatedFrames, returnedFrames, cameras) {
  return page.evaluate(
    async ({ baselineFrames, rotatedFrames, returnedFrames, cameras }) => {
      async function medianLuma(frames) {
        const decoded = [];
        for (const frame of frames) {
          const response = await fetch(`data:image/png;base64,${frame.base64}`);
          const bitmap = await createImageBitmap(await response.blob());
          const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
          const context = canvas.getContext("2d", { willReadFrequently: true });
          context.drawImage(bitmap, 0, 0);
          decoded.push({
            width: bitmap.width,
            height: bitmap.height,
            pixels: context.getImageData(0, 0, bitmap.width, bitmap.height).data,
          });
        }
        const luma = new Float32Array(decoded[0].width * decoded[0].height);
        for (let index = 0; index < luma.length; index += 1) {
          const source = index * 4;
          const values = decoded
            .map((image) => {
              const red = image.pixels[source] / 255;
              const green = image.pixels[source + 1] / 255;
              const blue = image.pixels[source + 2] / 255;
              return red * 0.2126 + green * 0.7152 + blue * 0.0722;
            })
            .sort((left, right) => left - right);
          luma[index] = values[Math.floor(values.length / 2)];
        }
        return { width: decoded[0].width, height: decoded[0].height, luma };
      }
      function basis({ yaw, pitch }) {
        const forward = [
          Math.sin(yaw) * Math.cos(pitch),
          Math.sin(pitch),
          -Math.cos(yaw) * Math.cos(pitch),
        ];
        const right = [Math.cos(yaw), 0, Math.sin(yaw)];
        const up = [
          -Math.sin(yaw) * Math.sin(pitch),
          Math.cos(pitch),
          Math.cos(yaw) * Math.sin(pitch),
        ];
        return { forward, right, up };
      }
      function dot(left, right) {
        return left[0] * right[0] + left[1] * right[1] + left[2] * right[2];
      }
      function normalize(vector) {
        const length = Math.sqrt(dot(vector, vector));
        return vector.map((value) => value / length);
      }
      function bilinear(image, x, y) {
        const x0 = Math.floor(x);
        const y0 = Math.floor(y);
        const x1 = Math.min(x0 + 1, image.width - 1);
        const y1 = Math.min(y0 + 1, image.height - 1);
        const tx = x - x0;
        const ty = y - y0;
        const top =
          image.luma[x0 + y0 * image.width] * (1 - tx) + image.luma[x1 + y0 * image.width] * tx;
        const bottom =
          image.luma[x0 + y1 * image.width] * (1 - tx) + image.luma[x1 + y1 * image.width] * tx;
        return top * (1 - ty) + bottom * ty;
      }
      function directMetrics(first, second) {
        let count = 0;
        let absolute = 0;
        let catastrophic = 0;
        for (let y = 0; y < first.height * 0.68; y += 2) {
          for (let x = 0; x < first.width * 0.78; x += 2) {
            const left = first.luma[x + y * first.width];
            const right = second.luma[x + y * second.width];
            absolute += Math.abs(left - right);
            catastrophic += Number(
              Math.max(left, right) > 0.05 && Math.min(left, right) < Math.max(left, right) * 0.5,
            );
            count += 1;
          }
        }
        return { meanAbsoluteError: absolute / count, catastrophicFraction: catastrophic / count };
      }
      function layeringMetrics(image) {
        const x0 = Math.floor(image.width * 0.04);
        const x1 = Math.floor(image.width * 0.78);
        const y0 = Math.floor(image.height * 0.06);
        const y1 = Math.floor(image.height * 0.68);
        let horizontalCoherent = 0;
        let horizontalTotal = 0;
        for (let y = y0 + 2; y < y1 - 2; y += 2) {
          let signed = 0;
          let absolute = 0;
          let samples = 0;
          for (let x = x0; x < x1; x += 2) {
            const curvature =
              image.luma[x + (y - 2) * image.width] -
              image.luma[x + y * image.width] * 2 +
              image.luma[x + (y + 2) * image.width];
            signed += curvature;
            absolute += Math.abs(curvature);
            samples += 1;
          }
          horizontalCoherent += Math.abs(signed / samples);
          horizontalTotal += absolute / samples;
        }
        let verticalCoherent = 0;
        let verticalTotal = 0;
        for (let x = x0 + 2; x < x1 - 2; x += 2) {
          let signed = 0;
          let absolute = 0;
          let samples = 0;
          for (let y = y0; y < y1; y += 2) {
            const curvature =
              image.luma[x - 2 + y * image.width] -
              image.luma[x + y * image.width] * 2 +
              image.luma[x + 2 + y * image.width];
            signed += curvature;
            absolute += Math.abs(curvature);
            samples += 1;
          }
          verticalCoherent += Math.abs(signed / samples);
          verticalTotal += absolute / samples;
        }
        const rowMean = [];
        for (let y = y0; y < y1; y += 1) {
          let total = 0;
          for (let x = x0; x < x1; x += 2) total += image.luma[x + y * image.width];
          rowMean.push(total / Math.ceil((x1 - x0) / 2));
        }
        const residual = rowMean.map((value, index) => {
          let local = 0;
          let samples = 0;
          for (
            let neighbor = Math.max(0, index - 12);
            neighbor <= Math.min(rowMean.length - 1, index + 12);
            neighbor += 1
          ) {
            local += rowMean[neighbor];
            samples += 1;
          }
          return value - local / samples;
        });
        let maximumAutocorrelation = -1;
        let maximumAutocorrelationLagPixels = 0;
        const rowAutocorrelations = [];
        for (let lag = 4; lag <= 40; lag += 1) {
          let product = 0;
          let firstEnergy = 0;
          let secondEnergy = 0;
          for (let index = 0; index + lag < residual.length; index += 1) {
            const first = residual[index];
            const second = residual[index + lag];
            product += first * second;
            firstEnergy += first * first;
            secondEnergy += second * second;
          }
          const correlation = product / Math.max(Math.sqrt(firstEnergy * secondEnergy), 0.000001);
          rowAutocorrelations.push({ lag, correlation });
          if (correlation > maximumAutocorrelation) {
            maximumAutocorrelation = correlation;
            maximumAutocorrelationLagPixels = lag;
          }
        }
        // A repeated integration shelf creates a narrow correlation spike. A large natural cloud
        // creates a broad hill across neighboring lags, so raw maximum correlation incorrectly
        // penalizes coherent morphology. Peak prominence separates those two cases.
        const offPeakCorrelations = rowAutocorrelations
          .filter(({ lag }) => Math.abs(lag - maximumAutocorrelationLagPixels) >= 3)
          .map(({ correlation }) => correlation)
          .sort((left, right) => left - right);
        const offPeakReference =
          offPeakCorrelations[Math.floor((offPeakCorrelations.length - 1) * 0.9)] ?? 0;
        const horizontalCoherence = horizontalCoherent / Math.max(horizontalTotal, 0.000001);
        const verticalCoherence = verticalCoherent / Math.max(verticalTotal, 0.000001);
        return {
          roi: { x0, x1, y0, y1 },
          horizontalCoherence,
          verticalCoherence,
          directionalExcess: horizontalCoherence / Math.max(verticalCoherence, 0.000001),
          maximumRowAutocorrelation: maximumAutocorrelation,
          maximumRowAutocorrelationLagPixels: maximumAutocorrelationLagPixels,
          maximumRowAutocorrelationProminence: maximumAutocorrelation - offPeakReference,
        };
      }
      const baseline = await medianLuma(baselineFrames);
      const rotated = await medianLuma(rotatedFrames);
      const returned = await medianLuma(returnedFrames);
      const sourceBasis = basis(cameras.baseline);
      const targetBasis = basis(cameras.rotated);
      const aspect = baseline.width / baseline.height;
      const tangent = 0.6745085;
      let count = 0;
      let weightedCount = 0;
      let absolute = 0;
      let weightedAbsolute = 0;
      let product = 0;
      let sourceEnergy = 0;
      let targetEnergy = 0;
      let catastrophic = 0;
      for (let y = 10; y < baseline.height * 0.68; y += 2) {
        for (let x = 10; x < baseline.width * 0.78; x += 2) {
          const ndcX = ((x + 0.5) / baseline.width) * 2 - 1;
          const ndcY = (1 - (y + 0.5) / baseline.height) * 2 - 1;
          const ray = normalize([
            sourceBasis.forward[0] +
              sourceBasis.right[0] * ndcX * aspect * tangent +
              sourceBasis.up[0] * ndcY * tangent,
            sourceBasis.forward[1] +
              sourceBasis.right[1] * ndcX * aspect * tangent +
              sourceBasis.up[1] * ndcY * tangent,
            sourceBasis.forward[2] +
              sourceBasis.right[2] * ndcX * aspect * tangent +
              sourceBasis.up[2] * ndcY * tangent,
          ]);
          const targetDepth = dot(ray, targetBasis.forward);
          if (ray[1] <= 0.03 || targetDepth <= 0.01) continue;
          const targetNdcX = dot(ray, targetBasis.right) / (targetDepth * aspect * tangent);
          const targetNdcY = dot(ray, targetBasis.up) / (targetDepth * tangent);
          const targetX = (targetNdcX + 1) * 0.5 * rotated.width - 0.5;
          const targetY = (1 - targetNdcY) * 0.5 * rotated.height - 0.5;
          if (
            targetX < 4 ||
            targetY < 4 ||
            targetX >= rotated.width - 4 ||
            targetY >= rotated.height - 4
          ) {
            continue;
          }
          const source = baseline.luma[x + y * baseline.width];
          const target = bilinear(rotated, targetX, targetY);
          const blur =
            (baseline.luma[x - 8 + y * baseline.width] +
              baseline.luma[x + 8 + y * baseline.width] +
              baseline.luma[x + (y - 8) * baseline.width] +
              baseline.luma[x + (y + 8) * baseline.width]) *
            0.25;
          const weight = Math.min(Math.abs(source - blur) / 0.015, 1);
          const delta = Math.abs(source - target);
          absolute += delta;
          weightedAbsolute += delta * weight;
          product += source * target * weight;
          sourceEnergy += source * source * weight;
          targetEnergy += target * target * weight;
          catastrophic += Number(
            Math.max(source, target) > 0.05 &&
              Math.min(source, target) < Math.max(source, target) * 0.5,
          );
          weightedCount += weight;
          count += 1;
        }
      }
      return {
        layering: layeringMetrics(baseline),
        rotation: {
          samples: count,
          weightedSamples: weightedCount,
          meanAbsoluteError: absolute / count,
          weightedMeanAbsoluteError: weightedAbsolute / Math.max(weightedCount, 1),
          weightedCorrelation: product / Math.max(Math.sqrt(sourceEnergy * targetEnergy), 0.000001),
          catastrophicFraction: catastrophic / count,
        },
        returned: directMetrics(baseline, returned),
      };
    },
    { baselineFrames, rotatedFrames, returnedFrames, cameras },
  );
}

await mkdir(OUTPUT_DIRECTORY, { recursive: true });
const errors = [];
const port = await reserveEphemeralPort();
const currentUnixSeconds = Date.now() / 1_000;
const weatherFractionAtUnixEpoch = wrappedFraction(
  0.3 - currentUnixSeconds / WEATHER_CYCLE_SECONDS,
);
let browser;
let server;
let fixture;
let worldService;

try {
  fixture = await prepareBrowserWorldFixture({
    browserPort: port,
    prefix: "voxels-weather-motion-",
    source: "procedural-v16",
    cascadedShadows: true,
    screenSpaceAmbientOcclusion: true,
    dayLengthSeconds: 0,
    dayFractionAtUnixEpoch: 0.5,
    weatherCycleSeconds: CLOUD_LAYERING_ONLY ? 0 : WEATHER_CYCLE_SECONDS,
    weatherFractionAtUnixEpoch: CLOUD_LAYERING_ONLY ? 0.32 : weatherFractionAtUnixEpoch,
    cloudVelocityMetresPerSecond: [0, 0],
  });
  const { build, preview } = await import("vite-plus");
  await build({ logLevel: "warn" });
  worldService = await startBrowserWorldService(fixture);
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
  const settled = await waitForEngine(page);

  const baselineCamera = { yaw: 0.35, pitch: 0.32 };
  const rotatedCamera = { yaw: 0.39, pitch: 0.32 };
  await waitForWeatherFraction(page, 0.32);
  await setCameraLook(page, baselineCamera.yaw, baselineCamera.pitch);
  const cloudBaseline = await captureBurst(page, 5, 18, "cloud-baseline");
  await setCameraLook(page, rotatedCamera.yaw, rotatedCamera.pitch);
  const cloudRotated = await captureBurst(page, 5, 18, "cloud-rotated");
  await setCameraLook(page, baselineCamera.yaw, baselineCamera.pitch);
  const cloudReturned = await captureBurst(page, 5, 18, "cloud-returned");
  const cloud = await analyzeCloudRotation(page, cloudBaseline, cloudRotated, cloudReturned, {
    baseline: baselineCamera,
    rotated: rotatedCamera,
  });

  let rain;
  if (!CLOUD_LAYERING_ONLY) {
    await waitForWeatherFraction(page, 0.5);
    await setCameraLook(page, 0.35, 0.05);
    const rainFrames = await captureBurst(page, 10, 28, "rain");
    rain = await analyzeRainMotion(page, rainFrames);
  }
  const finalSnapshot = assertSnapshotSchema(
    await page.evaluate(() => globalThis.__VOXELS__.snapshot()),
  );

  const correlatedRainPairs = rain?.pairs.filter((pair) => pair.score >= 0.08) ?? [];
  // The analysis samples every fourth pixel. Displacements below one sampled interval per
  // ordinary frame are indistinguishable from stationary background correlation, so they do
  // not carry directional evidence.
  const rainMotionPairs = correlatedRainPairs.filter(
    (pair) => Math.abs(pair.velocityYPixelsPerSecond) >= 80,
  );
  const downwardPairs = rainMotionPairs.filter((pair) => pair.dyPixels > 0);
  const rainMedianVelocity = median(rainMotionPairs.map((pair) => pair.velocityYPixelsPerSecond));
  const rainMedianAdvantage = median(rainMotionPairs.map((pair) => pair.positiveAdvantage));
  const violations = [];
  if (rain) {
    if (rainMotionPairs.length < 4) violations.push("too few measurable rain motion pairs");
    // Individual sparse pairs can alias to a different streak in the periodic particle
    // lattice. Direction is therefore gated by the median best displacement and by the
    // median advantage of the complete downward search half over the upward half below,
    // rather than by requiring every pair's single strongest alias to agree.
    if (rainMedianVelocity < 80 || rainMedianVelocity > 2_500) {
      violations.push(`rain vertical velocity was implausible: ${rainMedianVelocity}`);
    }
    if (rainMedianAdvantage < 0.01) {
      violations.push("downward rain correlation did not beat upward correlation");
    }
    if (median(rain.occupied) < 0.00025 || median(rain.occupied) > 0.2) {
      violations.push("rain signal occupancy was outside the useful regression range");
    }
  }
  if (cloud.rotation.weightedSamples < cloud.rotation.samples * 0.03) {
    violations.push("cloud comparison did not contain enough visible detail");
  }
  if (cloud.rotation.weightedCorrelation < 0.95) {
    violations.push(`cloud rotation correlation was ${cloud.rotation.weightedCorrelation}`);
  }
  if (cloud.rotation.weightedMeanAbsoluteError > 0.035) {
    violations.push(
      `cloud rotation weighted error was ${cloud.rotation.weightedMeanAbsoluteError}`,
    );
  }
  if (cloud.rotation.catastrophicFraction > 0.005) {
    violations.push("cloud rotation produced catastrophic luminance changes");
  }
  if (cloud.returned.meanAbsoluteError > 0.015) {
    violations.push(`returned cloud view error was ${cloud.returned.meanAbsoluteError}`);
  }
  if (cloud.returned.catastrophicFraction > 0.002) {
    violations.push("returned cloud view retained unstable pixels");
  }
  if (CLOUD_LAYERING_ONLY && cloud.layering.directionalExcess > 0.93) {
    violations.push(`cloud horizontal layer excess was ${cloud.layering.directionalExcess}`);
  }
  if (CLOUD_LAYERING_ONLY && cloud.layering.maximumRowAutocorrelationProminence > 0.25) {
    violations.push(
      `cloud row autocorrelation prominence was ${cloud.layering.maximumRowAutocorrelationProminence}`,
    );
  }
  if (
    finalSnapshot[SNAPSHOT.quads] !== settled[SNAPSHOT.quads] ||
    finalSnapshot[SNAPSHOT.residentChunks] !== settled[SNAPSHOT.residentChunks]
  ) {
    violations.push("weather motion test changed terrain geometry or residency");
  }
  if (errors.length > 0) violations.push(...errors);

  const result = {
    ok: violations.length === 0,
    commit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(),
    dirty: execFileSync("git", ["status", "--porcelain"], { encoding: "utf8" }).length > 0,
    browser: browser.version(),
    weather: {
      fraction: finalSnapshot[SNAPSHOT.weatherFraction],
      precipitation: finalSnapshot[SNAPSHOT.precipitation],
    },
    rain: rain
      ? {
          medianOccupiedFraction: median(rain.occupied),
          correlatedPairs: correlatedRainPairs.length,
          motionPairs: rainMotionPairs.length,
          downwardPairs: downwardPairs.length,
          medianVelocityYPixelsPerSecond: rainMedianVelocity,
          medianPositiveCorrelationAdvantage: rainMedianAdvantage,
          pairs: rain.pairs,
        }
      : null,
    cloud,
    geometry: {
      quads: finalSnapshot[SNAPSHOT.quads],
      residentChunks: finalSnapshot[SNAPSHOT.residentChunks],
      visibleChunks: finalSnapshot[SNAPSHOT.visibleChunks],
    },
    violations,
  };
  await writeFile(
    path.join(OUTPUT_DIRECTORY, "report.json"),
    `${JSON.stringify(result, null, 2)}\n`,
  );
  console.log(JSON.stringify(result, null, 2));
  if (!result.ok) process.exitCode = 1;
} finally {
  await browser?.close();
  await server?.close();
  await worldService?.close();
  await fixture?.cleanup();
}
