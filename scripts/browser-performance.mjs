import { execFileSync } from "node:child_process";
import { mkdir, open, writeFile } from "node:fs/promises";
import path from "node:path";
import { chromium } from "playwright";
import {
  chromeWebGpuLaunchOptions,
  FRAME_SAMPLE_START,
  FRAME_SAMPLE_WIDTH,
  gpuSampleStart,
  GPU_SAMPLE_WIDTH,
  isBrowserConsoleFailure,
  reserveEphemeralPort,
  SNAPSHOT,
  SNAPSHOT_SCHEMA_VERSION,
} from "./browser-harness.mjs";
import { prepareBrowserWorldFixture, startBrowserWorldService } from "./browser-world-fixture.mjs";

const FAILURE =
  /panic|unreachable|runtimeerror|wgpu|webgpu|shader|sqlite|opfs|syncaccesshandle|nomodificationallowed|web lock request failed|no persistence leader|persistence .*failed/i;

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
      captures.flatMap((capture) => capture.gpuSamples).map((sample) => [sample.frameId, sample]),
    ).values(),
  );
  const column = (index) => samples.map((sample) => sample[index]);
  const frameIntervals = column(0);
  const frameIds = new Set(samples.map((sample) => sample[5]));
  const coveredGpuSamples = gpuSamples.filter((sample) => frameIds.has(sample.frameId));
  const unattributedCpu = samples.map((sample) =>
    Math.max(0, sample[1] - sample[2] - sample[3] - sample[4]),
  );
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
    unattributedCpuMs: summary(unattributedCpu),
    renderCpu: {
      cullMs: summary(column(6)),
      encodeMs: summary(column(7)),
      submitMs: summary(column(8)),
      testedSlices: summary(column(9)),
      selectedSlices: summary(column(10)),
    },
    gpu: {
      available: gpuSamples.length > 0,
      samples: gpuSamples.length,
      droppedSamples: captures.reduce((total, capture) => total + capture.gpuDropped, 0),
      frameCoverage: frameIds.size > 0 ? coveredGpuSamples.length / frameIds.size : 0,
      totalMs: gpuSamples.length > 0 ? summary(gpuColumn("total")) : null,
      shadowMs: gpuSamples.length > 0 ? summary(gpuColumn("shadow")) : null,
      shadowCascadeMs:
        gpuSamples.length > 0
          ? [
              summary(gpuColumn("shadowCascade0")),
              summary(gpuColumn("shadowCascade1")),
              summary(gpuColumn("shadowCascade2")),
            ]
          : null,
      worldMs: gpuSamples.length > 0 ? summary(gpuColumn("world")) : null,
      waterMs: gpuSamples.length > 0 ? summary(gpuColumn("water")) : null,
      depthPrepassMs: gpuSamples.length > 0 ? summary(gpuColumn("depthPrepass")) : null,
      ambientOcclusionMs: gpuSamples.length > 0 ? summary(gpuColumn("ambientOcclusion")) : null,
      uiMs: gpuSamples.length > 0 ? summary(gpuColumn("ui")) : null,
    },
    residentChunks: latest[SNAPSHOT.residentChunks],
    surfaceTiles: latest[SNAPSHOT.surfaceTiles],
    horizonTiles: {
      stride32: latest[SNAPSHOT.stride32Tiles],
      stride64: latest[SNAPSHOT.stride64Tiles],
    },
    interactiveLodsReady: latest[SNAPSHOT.interactiveLodsReady] === 1,
    allLodsReady: latest[SNAPSHOT.allLodsReady] === 1,
    visibleChunks: latest[SNAPSHOT.visibleChunks],
    pendingJobs: latest[SNAPSHOT.pendingJobs],
    quads: latest[SNAPSHOT.quads],
    waterQuads: latest[SNAPSHOT.waterQuads],
    waterDrawCalls: latest[SNAPSHOT.waterDrawCalls],
    drawCalls: latest[SNAPSHOT.drawCalls],
    shadowDrawCalls: latest[SNAPSHOT.shadowDrawCalls],
    framebuffer: {
      width: latest[SNAPSHOT.surfaceWidth],
      height: latest[SNAPSHOT.surfaceHeight],
      devicePixelRatio: latest[SNAPSHOT.devicePixelRatio],
      pixels: latest[SNAPSHOT.surfaceWidth] * latest[SNAPSHOT.surfaceHeight],
    },
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
    materialDetail: latest[SNAPSHOT.materialDetail] === 1,
    screenSpaceAmbientOcclusion: latest[SNAPSHOT.screenSpaceAmbientOcclusion] === 1,
    ambientOcclusionMiB: latest[SNAPSHOT.ambientOcclusionMiB],
    depthPrepassDrawCalls: latest[SNAPSHOT.depthPrepassDrawCalls],
    atmosphere: {
      daylightPhase: latest[SNAPSHOT.daylightPhase],
      surfaceRegion: latest[SNAPSHOT.surfaceRegion],
      cloudCoverage: latest[SNAPSHOT.cloudCoverage],
    },
    cave: {
      enclosure: latest[SNAPSHOT.enclosure],
      exposure: latest[SNAPSHOT.interiorExposure],
      headlamp: latest[SNAPSHOT.caveHeadlamp] === 1,
      probeUs: latest[SNAPSHOT.enclosureProbeUs],
    },
    localLights: {
      candidates: latest[SNAPSHOT.localLightCandidates],
      active: latest[SNAPSHOT.activeLocalLights],
      clipped: latest[SNAPSHOT.clippedLocalLights],
      occluded: latest[SNAPSHOT.occludedLocalLights],
      portalRejected: latest[SNAPSHOT.portalRejectedLocalLights],
      visibilityTests: latest[SNAPSHOT.localLightVisibilityTests],
      enabled: latest[SNAPSHOT.localLighting] === 1,
    },
    cinderPortals: {
      open: latest[SNAPSHOT.openCinderPortals],
      revision: latest[SNAPSHOT.cinderPortalRevision],
    },
    portalStreaming: {
      requested: latest[SNAPSHOT.streamInterestRequested],
      normalized: latest[SNAPSHOT.streamInterestNormalized],
      desired: latest[SNAPSHOT.streamInterestDesired],
      truncated: latest[SNAPSHOT.streamInterestTruncated],
      planOverflow: latest[SNAPSHOT.streamPlanOverflow] === 1,
      activeChunks: latest[SNAPSHOT.portalActiveChunks],
      activeColumns: latest[SNAPSHOT.portalActiveColumns],
      unreachableActive: latest[SNAPSHOT.unreachablePortalActive],
    },
    placementMaterial: latest[SNAPSHOT.placementMaterial],
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

async function capture(page) {
  const snapshot = await page.evaluate(() => globalThis.__VOXELS__.snapshot());
  const count = snapshot[SNAPSHOT.sampleCount];
  const samples = [];
  for (let index = 0; index < count; index += 1) {
    const start = FRAME_SAMPLE_START + index * FRAME_SAMPLE_WIDTH;
    samples.push(snapshot.slice(start, start + FRAME_SAMPLE_WIDTH));
  }
  const gpuStart = gpuSampleStart(snapshot);
  const gpuCount = snapshot[gpuStart] ?? 0;
  const gpuDropped = snapshot[gpuStart + 1] ?? 0;
  const gpuSamples = [];
  for (let index = 0; index < gpuCount; index += 1) {
    const start = gpuStart + 2 + index * GPU_SAMPLE_WIDTH;
    const values = snapshot.slice(start, start + GPU_SAMPLE_WIDTH);
    gpuSamples.push({
      frameId: values[0],
      total: values[1],
      shadow: values[2],
      shadowCascade0: values[3],
      shadowCascade1: values[4],
      shadowCascade2: values[5],
      depthPrepass: values[6],
      ambientOcclusion: values[7],
      world: values[8],
      water: values[9],
      ui: values[10],
    });
  }
  return {
    snapshot,
    samples,
    dropped: snapshot[SNAPSHOT.droppedSamples],
    gpuSamples,
    gpuDropped,
  };
}

async function sample(page, durationMs) {
  const captures = [];
  const deadline = Date.now() + durationMs;
  while (Date.now() < deadline) {
    captures.push(await capture(page));
    await page.waitForTimeout(250);
  }
  return captures;
}

async function mark(page, name) {
  await page.evaluate((value) => performance.mark(value), name);
}

async function startChromiumTrace(context, page) {
  const session = await context.newCDPSession(page);
  await session.send("Performance.enable");
  await session.send("Tracing.start", {
    transferMode: "ReturnAsStream",
    traceConfig: {
      recordMode: "recordContinuously",
      enableSampling: true,
      includedCategories: [
        "blink.user_timing",
        "cc",
        "devtools.timeline",
        "disabled-by-default-devtools.timeline",
        "disabled-by-default-v8.cpu_profiler",
        "gpu",
        "renderer.scheduler",
        "toplevel",
        "v8",
        "viz",
      ],
    },
  });
  return session;
}

async function stopChromiumTrace(session, outputPath) {
  const completed = new Promise((resolve) => session.once("Tracing.tracingComplete", resolve));
  await session.send("Tracing.end");
  const { stream } = await completed;
  if (!stream) throw new Error("Chromium trace completed without a readable stream");
  await mkdir(path.dirname(outputPath), { recursive: true });
  const file = await open(outputPath, "w");
  try {
    while (true) {
      const chunk = await session.send("IO.read", { handle: stream });
      const bytes = chunk.base64Encoded ? Buffer.from(chunk.data, "base64") : chunk.data;
      await file.write(bytes);
      if (chunk.eof) break;
    }
  } finally {
    await file.close();
    await session.send("IO.close", { handle: stream });
  }
  const metrics = await session.send("Performance.getMetrics");
  await session.detach();
  return Object.fromEntries(metrics.metrics.map(({ name, value }) => [name, value]));
}

async function setMaterialDetail(page, enabled, viewportWidth) {
  const current = await page.evaluate(
    (index) => globalThis.__VOXELS__.snapshot().then((snapshot) => snapshot[index] === 1),
    SNAPSHOT.materialDetail,
  );
  if (current === enabled) return;
  await page.keyboard.press("F3");
  await page.waitForTimeout(200);
  // Material detail is the eighth Rust-owned feature row. The click targets the toggle,
  // exercising the same canvas hit-testing path as a human rather than a JavaScript render option.
  await page.mouse.click(viewportWidth - 57, 607);
  await page.waitForFunction(
    async ({ index, expected }) => {
      const snapshot = await globalThis.__VOXELS__.snapshot();
      return (snapshot[index] === 1) === expected;
    },
    { index: SNAPSHOT.materialDetail, expected: enabled },
    { timeout: 5_000 },
  );
  await page.keyboard.press("F3");
  await page.waitForTimeout(800);
}

async function materialDetailProfile(page, viewportWidth) {
  await setMaterialDetail(page, false, viewportWidth);
  const off = phaseSummary(await sample(page, 5_000));
  await setMaterialDetail(page, true, viewportWidth);
  const on = phaseSummary(await sample(page, 5_000));
  const delta = {
    worldP95Ms: on.gpu.worldMs.p95 - off.gpu.worldMs.p95,
    totalP95Ms: on.gpu.totalMs.p95 - off.gpu.totalMs.p95,
    frameP95Ms: on.frameMs.p95 - off.frameMs.p95,
    coreGpuMiB: on.coreGpuMiB - off.coreGpuMiB,
  };
  const invariantKeys = [
    "residentChunks",
    "quads",
    "waterQuads",
    "waterDrawCalls",
    "drawCalls",
    "meshArenaAllocatedMiB",
    "meshArenaCapacityMiB",
    "refractionCopyMiB",
  ];
  const changed = invariantKeys.filter((key) => on[key] !== off[key]);
  const violations = [];
  if (!off.gpu.available || !on.gpu.available) violations.push("GPU timestamps unavailable");
  if (off.materialDetail || !on.materialDetail)
    violations.push("Rust UI toggle state was not observed");
  if (changed.length > 0)
    violations.push(`geometry/resource invariants changed: ${changed.join(", ")}`);
  if (delta.worldP95Ms > 0.5) violations.push("world GPU p95 increased by more than 0.50ms");
  if (delta.totalP95Ms > 0.75) violations.push("active GPU p95 increased by more than 0.75ms");
  if (off.frameMs.p95 > 12 || on.frameMs.p95 > 12) violations.push("frame p95 exceeded 12ms");
  if (off.frameMs.above33_33ms > 0 || on.frameMs.above33_33ms > 0) {
    violations.push("a paired profile frame exceeded 33.33ms");
  }
  if (off.droppedSamples > 0 || on.droppedSamples > 0)
    violations.push("frame samples were dropped");
  const result = { off, on, delta, invariantKeys };
  if (violations.length > 0) {
    throw new Error(
      `material detail profile violations: ${violations.join(", ")}; ${JSON.stringify(result)}`,
    );
  }
  return result;
}

async function cycleDaylight(page, viewportWidth, expectedPhase) {
  await page.keyboard.press("Escape");
  await page.keyboard.press("F3");
  await page.waitForTimeout(100);
  // TIME is an explicit Mission Control header action. This intentionally exercises Rust canvas
  // hit-testing rather than adding a browser-side debug setter for atmosphere state.
  await page.mouse.click(viewportWidth - 134, 88);
  await page.waitForFunction(
    async ({ index, expected }) => {
      const snapshot = await globalThis.__VOXELS__.snapshot();
      return snapshot[index] === expected;
    },
    { index: SNAPSHOT.daylightPhase, expected: expectedPhase },
    { timeout: 5_000 },
  );
  await page.keyboard.press("F3");
  await page.waitForTimeout(1_200);
}

async function atmosphereProfile(page, viewportWidth) {
  const names = ["dawn", "clearDay", "goldenHour", "blueHour"];
  const phases = {};
  for (let captured = 0; captured < names.length; captured += 1) {
    const before = await capture(page);
    const phase = before.snapshot[SNAPSHOT.daylightPhase];
    phases[names[phase]] = phaseSummary(await sample(page, 4_000));
    await page.screenshot({ path: `target/atmosphere-${names[phase]}.png` });
    if (captured + 1 < names.length) {
      await cycleDaylight(page, viewportWidth, (phase + 1) % names.length);
    }
  }

  const values = Object.values(phases);
  const violations = [];
  if (values.length !== names.length) violations.push("did not observe all four daylight phases");
  const reference = values[0];
  for (const [name, phase] of Object.entries(phases)) {
    if (phase.frameMs.p95 > 12 || phase.frameMs.above16_67ms > 0) {
      violations.push(`${name} missed the 120Hz frame gate`);
    }
    if (phase.gpu.available && phase.gpu.worldMs.p95 > 2.0) {
      violations.push(`${name} world GPU p95 exceeded 2ms`);
    }
    if (phase.gpu.available && phase.gpu.totalMs.p95 > 7.5) {
      violations.push(`${name} active GPU p95 exceeded 7.5ms`);
    }
    if (phase.atmosphere.cloudCoverage < 0.08 || phase.atmosphere.cloudCoverage > 0.94) {
      violations.push(`${name} cloud coverage escaped its normalized visual range`);
    }
    if (
      reference &&
      (phase.quads !== reference.quads ||
        phase.visibleChunks !== reference.visibleChunks ||
        phase.drawCalls !== reference.drawCalls ||
        phase.meshArenaCapacityMiB !== reference.meshArenaCapacityMiB)
    ) {
      violations.push(`${name} changed geometry or mesh residency`);
    }
    if (phase.droppedSamples > 0) violations.push(`${name} dropped frame samples`);
  }
  if (violations.length > 0) {
    throw new Error(
      `atmosphere profile violations: ${violations.join(", ")}; ${JSON.stringify(phases)}`,
    );
  }
  return phases;
}

async function sustainedProfile(page) {
  await page.evaluate(() => globalThis.__VOXELS__.profile(1));
  const captures = [];
  const deadline = Date.now() + 120_000;
  while (Date.now() < deadline) {
    const next = await capture(page);
    captures.push(next);
    if (next.snapshot[SNAPSHOT.profileComplete] === 1) break;
    await page.waitForTimeout(250);
  }
  const latest = captures.at(-1)?.snapshot;
  if (!latest || latest[SNAPSHOT.profileComplete] !== 1) {
    throw new Error(
      `sustained Rust profile did not drain: ${JSON.stringify(latest?.slice(56, 70))}`,
    );
  }
  const measured = captures.filter((capture) => capture.snapshot[SNAPSHOT.profilePhase] === 2);
  const finalTwenty = measured.filter(
    (capture) => capture.snapshot[SNAPSHOT.profileElapsedSeconds] >= 70,
  );
  const range = (values) => Math.max(...values) - Math.min(...values);
  const result = {
    measured: phaseSummary(measured),
    distanceMetres: latest[SNAPSHOT.profileDistanceMetres],
    evictions: latest[SNAPSHOT.profileEvictions],
    highWater: {
      trackedChunks: latest[SNAPSHOT.profileTrackedHigh],
      surfaceTiles: latest[SNAPSHOT.profileSurfaceHigh],
      pendingJobs: latest[SNAPSHOT.profilePendingHigh],
      pendingMeshes: latest[SNAPSHOT.profilePendingMeshHigh],
      arenaCapacityMiB: latest[SNAPSHOT.profileArenaCapacityHighMiB],
      wasmCommittedMiB: latest[SNAPSHOT.profileWasmHighMiB],
    },
    finalTwentySeconds: {
      wasmCommittedRangeMiB: range(
        finalTwenty.map((capture) => capture.snapshot[SNAPSHOT.wasmCommittedMiB]),
      ),
      arenaCapacityRangeMiB: range(
        finalTwenty.map((capture) => capture.snapshot[SNAPSHOT.arenaCapacityMiB]),
      ),
    },
    final: {
      pendingJobs: latest[SNAPSHOT.pendingJobs],
      pendingMeshMiB: latest[SNAPSHOT.pendingMeshMiB],
      canonicalVoxelMiB: latest[SNAPSHOT.canonicalVoxelMiB],
      staleCompletions: latest[SNAPSHOT.staleCompletions],
    },
  };
  const violations = [];
  if (result.distanceMetres < 1_000) violations.push("distance below 1km");
  if (result.evictions < 500) violations.push("fewer than 500 canonical evictions");
  if (result.highWater.trackedChunks > 320) violations.push("tracked chunk bound exceeded");
  if (result.highWater.surfaceTiles > 896) violations.push("surface residency bound exceeded");
  if (result.highWater.pendingMeshes > 3) violations.push("pending mesh bound exceeded");
  if (result.final.pendingJobs !== 0) violations.push("queues did not drain");
  if (result.final.pendingMeshMiB !== 0) violations.push("pending mesh payload did not drain");
  if (result.measured.frameMs.p95 > 12) violations.push("frame p95 above 12ms");
  if (result.measured.frameMs.p99 > 16.67) violations.push("frame p99 above 16.67ms");
  if (result.measured.cpuMs.p95 > 7.5) violations.push("worker CPU p95 above 7.5ms");
  if (result.measured.streamingMs.p95 > 4.5) violations.push("streaming p95 above 4.5ms");
  if (result.measured.frameMs.above33_33ms > 0) violations.push("frame exceeded 33.33ms");
  if (result.measured.droppedSamples > 0) violations.push("frame samples were dropped");
  if (result.finalTwentySeconds.wasmCommittedRangeMiB > 1) {
    violations.push("WASM committed memory did not plateau");
  }
  if (result.finalTwentySeconds.arenaCapacityRangeMiB > 4) {
    violations.push("mesh arena capacity did not plateau");
  }
  if (violations.length > 0) {
    throw new Error(
      `sustained profile violations: ${violations.join(", ")}; ${JSON.stringify(result)}`,
    );
  }
  return result;
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
      snapshot[SNAPSHOT.schemaVersion] === SNAPSHOT_SCHEMA_VERSION &&
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

const viewport = {
  width: Number.parseInt(process.env.VOXELS_PROFILE_WIDTH ?? "1280", 10),
  height: Number.parseInt(process.env.VOXELS_PROFILE_HEIGHT ?? "720", 10),
};
const deviceScaleFactor = Number.parseFloat(process.env.VOXELS_PROFILE_DPR ?? "1");
if (
  !Number.isInteger(viewport.width) ||
  !Number.isInteger(viewport.height) ||
  viewport.width < 320 ||
  viewport.height < 240 ||
  !Number.isFinite(deviceScaleFactor) ||
  deviceScaleFactor < 0.5 ||
  deviceScaleFactor > 4
) {
  throw new Error("profile viewport must be at least 320x240 with DPR in 0.5..=4");
}
const sustained = process.argv.includes("--sustained");
const materials = process.argv.includes("--materials");
const atmosphere = process.argv.includes("--atmosphere");
const stationary = process.argv.includes("--stationary");
const worldSource = process.env.VOXELS_PROFILE_SOURCE ?? "procedural-v16";
const spawnVoxels = (() => {
  const configured = process.env.VOXELS_PROFILE_SPAWN;
  if (configured === undefined) return undefined;
  const parts = configured.split(",").map((value) => value.trim());
  const values = parts.map(Number);
  if (
    parts.length !== 2 ||
    !parts.every((value) => /^-?\d+$/.test(value)) ||
    !values.every(
      (value) => Number.isInteger(value) && value >= -2_147_483_648 && value <= 2_147_483_647,
    )
  ) {
    throw new Error("VOXELS_PROFILE_SPAWN must be two comma-separated canonical voxel coordinates");
  }
  return values;
})();
const cameraLook = (() => {
  const configured = process.env.VOXELS_PROFILE_LOOK;
  if (configured === undefined) return undefined;
  const values = configured.split(",").map((value) => Number(value.trim()));
  if (
    values.length !== 2 ||
    !values.every(Number.isFinite) ||
    values[1] < -Math.PI / 2 ||
    values[1] > Math.PI / 2
  ) {
    throw new Error("VOXELS_PROFILE_LOOK must be finite comma-separated yaw,pitch radians");
  }
  return values;
})();
const cascadedShadows = (() => {
  const configured = process.env.VOXELS_PROFILE_SHADOWS;
  if (configured === undefined || configured === "on") return true;
  if (configured === "off") return false;
  throw new Error("VOXELS_PROFILE_SHADOWS must be on or off");
})();
const screenSpaceAmbientOcclusion = (() => {
  const configured = process.env.VOXELS_PROFILE_SSAO;
  if (configured === undefined || configured === "on") return true;
  if (configured === "off") return false;
  throw new Error("VOXELS_PROFILE_SSAO must be on or off");
})();
const buildProfile = process.env.VOXELS_PROFILE_BUILD ?? "release";
if (!new Set(["debug", "wasm-dev", "release"]).has(buildProfile)) {
  throw new Error("VOXELS_PROFILE_BUILD must be debug, wasm-dev, or release");
}
process.env.VOXELS_BROWSER_BUILD_PROFILE = buildProfile;
const traceEnabled = process.argv.includes("--trace") || process.env.VOXELS_PROFILE_TRACE === "1";
const tracePath = path.resolve(
  process.env.VOXELS_PROFILE_TRACE_PATH ??
    `target/render-profile-${buildProfile}-${worldSource}-${viewport.width}x${viewport.height}-dpr${deviceScaleFactor}.json`,
);
const outputPath = process.env.VOXELS_PROFILE_OUTPUT
  ? path.resolve(process.env.VOXELS_PROFILE_OUTPUT)
  : undefined;
const errors = [];
const port = await reserveEphemeralPort();
let browser;
let server;
let fixture;
let worldService;
let traceSession;
let traceMetrics;

function observePageErrors(page) {
  page.on("pageerror", (error) => errors.push(`pageerror: ${error.message}`));
  page.on("console", (message) => {
    if (isBrowserConsoleFailure(message.type(), message.text(), FAILURE)) {
      errors.push(`${message.type()}: ${message.text()}`);
    }
  });
}

try {
  fixture = await prepareBrowserWorldFixture({
    browserPort: port,
    prefix: "voxels-browser-profile-",
    source: worldSource,
    spawnVoxels,
    cascadedShadows,
    screenSpaceAmbientOcclusion,
  });
  const { build, preview } = await import("vite-plus");
  await build({ logLevel: "warn" });
  worldService = await startBrowserWorldService(fixture, {
    metal: worldSource === "terrain-diffusion-30m",
  });
  server = await preview({
    logLevel: "warn",
    preview: { host: "127.0.0.1", port, strictPort: true },
  });
  browser = await chromium.launch(chromeWebGpuLaunchOptions());
  const context = await browser.newContext({ viewport, deviceScaleFactor });
  const page = await context.newPage();
  observePageErrors(page);
  const navigationStarted = performance.now();
  await page.goto(`http://127.0.0.1:${port}`, { waitUntil: "domcontentloaded" });
  await waitForEngine(page);
  if (cameraLook) await setCameraLook(page, cameraLook[0], cameraLook[1]);
  const settledMilliseconds = performance.now() - navigationStarted;
  if (traceEnabled) traceSession = await startChromiumTrace(context, page);

  let scenarios;
  if (stationary) {
    scenarios = { steady: phaseSummary(await sample(page, 4_000)) };
  } else if (sustained) {
    scenarios = { sustained: await sustainedProfile(page) };
  } else if (materials) {
    scenarios = { materials: await materialDetailProfile(page, viewport.width) };
  } else if (atmosphere) {
    scenarios = { atmosphere: await atmosphereProfile(page, viewport.width) };
  } else {
    await mark(page, "voxels:steady:start");
    const steady = phaseSummary(await sample(page, 4_000));
    await mark(page, "voxels:steady:end");
    await mark(page, "voxels:traversal:start");
    await page.keyboard.down("KeyW");
    const traversalSamples = await sample(page, 6_000);
    await page.keyboard.up("KeyW");
    const traversal = phaseSummary(traversalSamples);
    await mark(page, "voxels:traversal:end");
    scenarios = { steady, traversal };
  }
  if (process.env.SCREENSHOT) {
    await page.screenshot({ path: process.env.SCREENSHOT });
  }
  const finalSnapshot = await page.evaluate(() => globalThis.__VOXELS__.snapshot());

  if (errors.length > 0) throw new Error(errors.join("\n"));
  if (traceSession) {
    traceMetrics = await stopChromiumTrace(traceSession, tracePath);
    traceSession = undefined;
  }
  const result = {
    ok: true,
    schemaVersion: 3,
    commit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(),
    dirty: execFileSync("git", ["status", "--porcelain"], { encoding: "utf8" }).trim() !== "",
    build: buildProfile,
    worldSource,
    spawnVoxels: fixture.spawnVoxels,
    requestedLook: cameraLook ?? null,
    cascadedShadows,
    screenSpaceAmbientOcclusion,
    finalPose: {
      x: finalSnapshot[SNAPSHOT.cameraX],
      y: finalSnapshot[SNAPSHOT.cameraY],
      z: finalSnapshot[SNAPSHOT.cameraZ],
      yaw: finalSnapshot[SNAPSHOT.yaw],
      pitch: finalSnapshot[SNAPSHOT.pitch],
    },
    viewport: { ...viewport, deviceScaleFactor },
    browser: { version: browser.version() },
    startup: { settledMilliseconds },
    ...scenarios,
    trace: traceEnabled ? { path: tracePath, performanceMetrics: traceMetrics } : null,
    errors: 0,
  };
  const json = `${JSON.stringify(result, null, 2)}\n`;
  if (outputPath) {
    await mkdir(path.dirname(outputPath), { recursive: true });
    await writeFile(outputPath, json);
  }
  console.log(json);
} catch (error) {
  console.error(JSON.stringify({ ok: false, error: String(error), errors }, null, 2));
  process.exitCode = 1;
} finally {
  if (traceSession) {
    try {
      await stopChromiumTrace(traceSession, tracePath);
    } catch {}
  }
  await browser?.close();
  await server?.close();
  await worldService?.close();
  await fixture?.cleanup();
}
