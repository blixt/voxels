import { createServer as createNetServer } from "node:net";
import { chromium } from "playwright";
import { build, preview } from "vite-plus";

const FAILURE =
  /panic|unreachable|runtimeerror|wgpu|webgpu|shader|sqlite|opfs|syncaccesshandle|nomodificationallowed|web lock request failed|no persistence leader|persistence .*failed/i;
const SNAPSHOT = {
  quads: 6,
  residentChunks: 8,
  visibleChunks: 10,
  drawCalls: 11,
  arenaAllocatedMiB: 13,
  arenaCapacityMiB: 14,
  pendingJobs: 15,
  frameMs: 17,
  loadP95Frames: 20,
  loadMaxFrames: 21,
  remeshP95Frames: 22,
  remeshMaxFrames: 23,
  waterQuads: 28,
  refractionCopyMiB: 30,
  immersion: 31,
  eyesSubmerged: 33,
  coreGpuMiB: 39,
  cpuMs: 40,
  simulationMs: 41,
  streamMs: 42,
  renderMs: 43,
  gpuSampleId: 44,
  gpuTotalMs: 45,
  gpuShadowMs: 46,
  gpuWorldMs: 47,
  gpuWaterMs: 48,
  gpuUiMs: 49,
  wasmCommittedMiB: 50,
  canonicalVoxelMiB: 51,
  pendingMeshMiB: 52,
  editLogicalMiB: 53,
  totalEvictions: 54,
  staleCompletions: 55,
  schemaVersion: 56,
  sampleCount: 57,
  droppedSamples: 58,
};
const FRAME_SAMPLE_WIDTH = 5;
const FRAME_SAMPLE_START = 59;

function percentile(values, fraction) {
  if (values.length === 0) return 0;
  const sorted = values.toSorted((left, right) => left - right);
  return sorted[Math.min(sorted.length - 1, Math.ceil(sorted.length * fraction) - 1)];
}

function summary(values) {
  return {
    median: percentile(values, 0.5),
    p95: percentile(values, 0.95),
    p99: percentile(values, 0.99),
    max: Math.max(...values, 0),
  };
}

function phaseSummary(captures) {
  const latest = captures.at(-1)?.snapshot;
  if (!latest) throw new Error("performance phase did not capture any samples");
  const samples = captures.flatMap((capture) => capture.samples);
  const gpuSamples = Array.from(
    new Map(
      captures
        .filter((capture) => capture.gpuSample)
        .map((capture) => [capture.gpuSample.id, capture.gpuSample]),
    ).values(),
  );
  const column = (index) => samples.map((sample) => sample[index]);
  const frameIntervals = column(0);
  const gpuColumn = (key) => gpuSamples.map((sample) => sample[key]);
  return {
    samples: samples.length,
    droppedSamples: captures.reduce((total, capture) => total + capture.dropped, 0),
    frameMs: {
      ...summary(frameIntervals),
      above16_67ms: frameIntervals.filter((value) => value > 16.67).length,
      above33_33ms: frameIntervals.filter((value) => value > 33.33).length,
    },
    cpuMs: summary(column(1)),
    simulationMs: summary(column(2)),
    streamingMs: summary(column(3)),
    renderSubmissionMs: summary(column(4)),
    gpu: {
      available: gpuSamples.length > 0,
      samples: gpuSamples.length,
      totalMs: gpuSamples.length > 0 ? summary(gpuColumn("total")) : null,
      shadowMs: gpuSamples.length > 0 ? summary(gpuColumn("shadow")) : null,
      worldMs: gpuSamples.length > 0 ? summary(gpuColumn("world")) : null,
      waterMs: gpuSamples.length > 0 ? summary(gpuColumn("water")) : null,
      uiMs: gpuSamples.length > 0 ? summary(gpuColumn("ui")) : null,
    },
    residentChunks: latest[SNAPSHOT.residentChunks],
    visibleChunks: latest[SNAPSHOT.visibleChunks],
    pendingJobs: latest[SNAPSHOT.pendingJobs],
    quads: latest[SNAPSHOT.quads],
    waterQuads: latest[SNAPSHOT.waterQuads],
    waterDrawCalls: latest[29],
    drawCalls: latest[SNAPSHOT.drawCalls],
    coreGpuMiB: latest[SNAPSHOT.coreGpuMiB],
    meshArenaAllocatedMiB: latest[SNAPSHOT.arenaAllocatedMiB],
    meshArenaCapacityMiB: latest[SNAPSHOT.arenaCapacityMiB],
    refractionCopyMiB: latest[SNAPSHOT.refractionCopyMiB],
    memory: {
      wasmCommittedMiB: latest[SNAPSHOT.wasmCommittedMiB],
      canonicalVoxelMiB: latest[SNAPSHOT.canonicalVoxelMiB],
      pendingMeshMiB: latest[SNAPSHOT.pendingMeshMiB],
      editLogicalMiB: latest[SNAPSHOT.editLogicalMiB],
    },
    totalEvictions: latest[SNAPSHOT.totalEvictions],
    staleCompletions: latest[SNAPSHOT.staleCompletions],
    loadLatencyFrames: {
      p95: latest[SNAPSHOT.loadP95Frames],
      max: latest[SNAPSHOT.loadMaxFrames],
    },
    remeshLatencyFrames: {
      p95: latest[SNAPSHOT.remeshP95Frames],
      max: latest[SNAPSHOT.remeshMaxFrames],
    },
  };
}

async function sample(page, durationMs) {
  const captures = [];
  const deadline = Date.now() + durationMs;
  while (Date.now() < deadline) {
    const snapshot = await page.evaluate(() => globalThis.__VOXELS__.snapshot());
    const count = snapshot[SNAPSHOT.sampleCount];
    const samples = [];
    for (let index = 0; index < count; index += 1) {
      const start = FRAME_SAMPLE_START + index * FRAME_SAMPLE_WIDTH;
      samples.push(snapshot.slice(start, start + FRAME_SAMPLE_WIDTH));
    }
    captures.push({
      snapshot,
      samples,
      dropped: snapshot[SNAPSHOT.droppedSamples],
      gpuSample:
        snapshot[SNAPSHOT.gpuSampleId] > 0
          ? {
              id: snapshot[SNAPSHOT.gpuSampleId],
              total: snapshot[SNAPSHOT.gpuTotalMs],
              shadow: snapshot[SNAPSHOT.gpuShadowMs],
              world: snapshot[SNAPSHOT.gpuWorldMs],
              water: snapshot[SNAPSHOT.gpuWaterMs],
              ui: snapshot[SNAPSHOT.gpuUiMs],
            }
          : null,
    });
    await page.waitForTimeout(250);
  }
  return captures;
}

async function waitForEngine(page) {
  await page.waitForFunction(() => typeof globalThis.__VOXELS__?.snapshot === "function", null, {
    timeout: 20_000,
  });
  const deadline = Date.now() + 60_000;
  let lastSnapshot = [];
  while (Date.now() < deadline) {
    const snapshot = await page.evaluate(() => globalThis.__VOXELS__.snapshot());
    lastSnapshot = snapshot;
    if (
      snapshot[SNAPSHOT.schemaVersion] === 4 &&
      snapshot[SNAPSHOT.quads] > 0 &&
      snapshot[SNAPSHOT.residentChunks] > 0 &&
      snapshot[SNAPSHOT.pendingJobs] === 0
    ) {
      return snapshot;
    }
    await page.waitForTimeout(50);
  }
  throw new Error(`release engine did not settle: ${JSON.stringify(lastSnapshot)}`);
}

async function enterUnderwaterShowcase(page, viewportWidth) {
  await page.keyboard.press("Escape");
  await page.keyboard.press("F3");
  await page.waitForTimeout(100);
  await page.mouse.click(viewportWidth - 83, 90);
  await page.waitForTimeout(100);
  await page.mouse.click(viewportWidth - 171, 168);
  await page.waitForFunction(
    async () => {
      const snapshot = await globalThis.__VOXELS__.snapshot();
      return snapshot[31] > 0.5 && snapshot[33] === 1;
    },
    null,
    { timeout: 20_000 },
  );
  await page.keyboard.press("F3");
  await page.waitForTimeout(500);
}

async function reserveEphemeralPort() {
  const probe = createNetServer();
  await new Promise((resolve, reject) => {
    probe.once("error", reject);
    probe.listen(0, "127.0.0.1", resolve);
  });
  const address = probe.address();
  if (!address || typeof address === "string") throw new Error("could not reserve a TCP port");
  await new Promise((resolve, reject) =>
    probe.close((error) => (error ? reject(error) : resolve())),
  );
  return address.port;
}

const viewport = { width: 1280, height: 720 };
const errors = [];
const port = await reserveEphemeralPort();
let browser;
let server;

try {
  await build({ logLevel: "warn" });
  server = await preview({
    logLevel: "warn",
    preview: { host: "127.0.0.1", port, strictPort: true },
  });
  browser = await chromium.launch({
    channel: "chrome",
    headless: false,
    args: [
      "--headless=new",
      "--enable-unsafe-webgpu",
      "--enable-features=WebGPU",
      "--no-sandbox",
      "--hide-scrollbars",
    ],
  });
  const context = await browser.newContext({ viewport, deviceScaleFactor: 1 });
  const page = await context.newPage();
  page.on("pageerror", (error) => errors.push(`pageerror: ${error.message}`));
  page.on("console", (message) => {
    if (
      (message.type() === "error" || message.type() === "warning") &&
      FAILURE.test(message.text())
    ) {
      errors.push(`${message.type()}: ${message.text()}`);
    }
  });
  const navigationStarted = performance.now();
  await page.goto(`http://127.0.0.1:${port}`, { waitUntil: "domcontentloaded" });
  await waitForEngine(page);
  const settledMilliseconds = performance.now() - navigationStarted;

  const steady = phaseSummary(await sample(page, 4_000));

  await page.keyboard.down("KeyW");
  const traversalSamples = await sample(page, 6_000);
  await page.keyboard.up("KeyW");
  const traversal = phaseSummary(traversalSamples);

  await enterUnderwaterShowcase(page, viewport.width);
  const underwater = phaseSummary(await sample(page, 4_000));
  if (process.env.SCREENSHOT) {
    await page.screenshot({ path: process.env.SCREENSHOT });
  }

  if (errors.length > 0) throw new Error(errors.join("\n"));
  console.log(
    JSON.stringify(
      {
        ok: true,
        schemaVersion: 1,
        build: "release",
        viewport,
        startup: { settledMilliseconds },
        steady,
        traversal,
        underwater,
        errors: 0,
      },
      null,
      2,
    ),
  );
} catch (error) {
  console.error(JSON.stringify({ ok: false, error: String(error), errors }, null, 2));
  process.exitCode = 1;
} finally {
  await browser?.close();
  await server?.close();
}
