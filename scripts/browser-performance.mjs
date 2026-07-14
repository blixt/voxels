import { chromium } from "playwright";
import { build, preview } from "vite-plus";
import {
  chromeWebGpuLaunchOptions,
  isBrowserConsoleFailure,
  reserveEphemeralPort,
  SNAPSHOT,
  SNAPSHOT_SCHEMA_VERSION,
} from "./browser-harness.mjs";

const FAILURE =
  /panic|unreachable|runtimeerror|wgpu|webgpu|shader|sqlite|opfs|syncaccesshandle|nomodificationallowed|web lock request failed|no persistence leader|persistence .*failed|landmark tour/i;
const FRAME_SAMPLE_WIDTH = 5;
const FRAME_SAMPLE_START = 104;

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
      depthPrepassMs: gpuSamples.length > 0 ? summary(gpuColumn("depthPrepass")) : null,
      ambientOcclusionMs: gpuSamples.length > 0 ? summary(gpuColumn("ambientOcclusion")) : null,
      uiMs: gpuSamples.length > 0 ? summary(gpuColumn("ui")) : null,
    },
    residentChunks: latest[SNAPSHOT.residentChunks],
    visibleChunks: latest[SNAPSHOT.visibleChunks],
    pendingJobs: latest[SNAPSHOT.pendingJobs],
    quads: latest[SNAPSHOT.quads],
    waterQuads: latest[SNAPSHOT.waterQuads],
    waterDrawCalls: latest[SNAPSHOT.waterDrawCalls],
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
  return {
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
            depthPrepass: snapshot[SNAPSHOT.gpuDepthPrepassMs],
            ambientOcclusion: snapshot[SNAPSHOT.gpuAmbientOcclusionMs],
            ui: snapshot[SNAPSHOT.gpuUiMs],
          }
        : null,
  };
}

async function waitForSnapshot(page, label, predicate, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs;
  let latest;
  while (Date.now() < deadline) {
    latest = await capture(page);
    if (predicate(latest)) return latest;
    await page.waitForTimeout(50);
  }
  throw new Error(`${label} did not converge: ${JSON.stringify(latest?.snapshot)}`);
}

async function cavePortalEditPersistenceProfile(page, context) {
  const baseline = await capture(page);
  if (
    baseline.edit.currentEdits !== 0 ||
    baseline.snapshot[SNAPSHOT.openCinderPortals] !== 7 ||
    baseline.snapshot[SNAPSHOT.cinderPortalRevision] !== 0
  ) {
    throw new Error(
      `Cinder portal edit fixture was not pristine: ${JSON.stringify(baseline.edit)}`,
    );
  }

  const observer = await context.newPage();
  observePageErrors(observer);
  await observer.goto(page.url(), { waitUntil: "domcontentloaded" });
  await waitForEngine(observer);

  await page.evaluate(() => globalThis.__VOXELS__.profile(3));
  const sealedLeader = await waitForSnapshot(
    page,
    "Cinder mouth seal on persistence leader",
    (next) =>
      next.edit.currentEdits === 25 &&
      next.edit.inFlight === 0 &&
      next.snapshot[SNAPSHOT.openCinderPortals] === 6 &&
      next.snapshot[SNAPSHOT.cinderPortalRevision] >= 1 &&
      next.snapshot[SNAPSHOT.pendingJobs] === 0,
  );
  const sealedObserver = await waitForSnapshot(
    observer,
    "Cinder mouth seal in observer tab",
    (next) =>
      next.edit.currentEdits === 25 &&
      next.edit.inFlight === 0 &&
      next.snapshot[SNAPSHOT.openCinderPortals] === 6 &&
      next.snapshot[SNAPSHOT.cinderPortalRevision] >= 1 &&
      next.snapshot[SNAPSHOT.pendingJobs] === 0,
  );
  await observer.close();

  await page.reload({ waitUntil: "domcontentloaded" });
  await waitForEngine(page);
  const sealedReload = await waitForSnapshot(
    page,
    "persisted Cinder mouth seal after reload",
    (next) =>
      next.edit.currentEdits === 25 &&
      next.snapshot[SNAPSHOT.openCinderPortals] === 6 &&
      next.snapshot[SNAPSHOT.cinderPortalRevision] === 0,
  );

  await page.evaluate(() => globalThis.__VOXELS__.profile(4));
  const restored = await waitForSnapshot(
    page,
    "Cinder mouth restoration",
    (next) =>
      next.edit.currentEdits === 0 &&
      next.edit.inFlight === 0 &&
      next.snapshot[SNAPSHOT.openCinderPortals] === 7 &&
      next.snapshot[SNAPSHOT.cinderPortalRevision] >= 1 &&
      next.snapshot[SNAPSHOT.pendingJobs] === 0,
  );

  await page.reload({ waitUntil: "domcontentloaded" });
  await waitForEngine(page);
  const restoredReload = await waitForSnapshot(
    page,
    "persisted Cinder mouth restoration after reload",
    (next) =>
      next.edit.currentEdits === 0 &&
      next.snapshot[SNAPSHOT.openCinderPortals] === 7 &&
      next.snapshot[SNAPSHOT.cinderPortalRevision] === 0,
  );

  return {
    editedVoxels: 25,
    sealed: {
      leaderOpenPortals: sealedLeader.snapshot[SNAPSHOT.openCinderPortals],
      observerOpenPortals: sealedObserver.snapshot[SNAPSHOT.openCinderPortals],
      reloadOpenPortals: sealedReload.snapshot[SNAPSHOT.openCinderPortals],
      persistedEdits: sealedReload.edit.currentEdits,
    },
    restored: {
      liveOpenPortals: restored.snapshot[SNAPSHOT.openCinderPortals],
      reloadOpenPortals: restoredReload.snapshot[SNAPSHOT.openCinderPortals],
      persistedEdits: restoredReload.edit.currentEdits,
    },
  };
}

async function waitForPortalStreaming(page, label, expectedActive) {
  return waitForSnapshot(
    page,
    label,
    (next) => {
      const snapshot = next.snapshot;
      const requested = snapshot[SNAPSHOT.streamInterestRequested];
      const desired = snapshot[SNAPSHOT.streamInterestDesired];
      const active = snapshot[SNAPSHOT.portalActiveChunks];
      return (
        snapshot[SNAPSHOT.pendingJobs] === 0 &&
        snapshot[SNAPSHOT.streamPlanOverflow] === 0 &&
        snapshot[SNAPSHOT.streamInterestTruncated] === 0 &&
        snapshot[SNAPSHOT.unreachablePortalActive] === 0 &&
        requested === snapshot[SNAPSHOT.streamInterestNormalized] &&
        desired === requested &&
        active === desired &&
        (expectedActive ? active > 0 : active === 0)
      );
    },
    60_000,
  );
}

async function cavePortalStreamingProfile(page, viewportWidth) {
  const far = await waitForPortalStreaming(page, "far exterior portal streaming", false);

  await visitCinderVault(page, viewportWidth);
  const approach = await waitForPortalStreaming(page, "Cinder approach portal streaming", true);
  await visitCinderVault(page, viewportWidth);
  const descent = await waitForPortalStreaming(page, "Cinder descent portal streaming", true);
  await visitCinderVault(page, viewportWidth);
  const chamber = await waitForPortalStreaming(page, "Cinder chamber portal streaming", true);
  await visitCinderVault(page, viewportWidth);
  const overhead = await waitForPortalStreaming(page, "Cinder overhead portal streaming", true);

  await page.evaluate(() => globalThis.__VOXELS__.profile(3));
  const sealedOutside = await waitForSnapshot(
    page,
    "sealed exterior portal streaming",
    (next) =>
      next.edit.currentEdits === 25 &&
      next.edit.inFlight === 0 &&
      next.snapshot[SNAPSHOT.openCinderPortals] === 6 &&
      next.snapshot[SNAPSHOT.streamInterestRequested] === 0 &&
      next.snapshot[SNAPSHOT.portalActiveChunks] === 0 &&
      next.snapshot[SNAPSHOT.unreachablePortalActive] === 0 &&
      next.snapshot[SNAPSHOT.pendingJobs] === 0,
    60_000,
  );

  for (let stop = 0; stop < 3; stop += 1) await visitCinderVault(page, viewportWidth);
  const sealedInside = await waitForPortalStreaming(
    page,
    "sealed chamber internal portal streaming",
    true,
  );
  await page.evaluate(() => globalThis.__VOXELS__.profile(4));
  const restoredInside = await waitForSnapshot(
    page,
    "restored chamber portal streaming",
    (next) =>
      next.edit.currentEdits === 0 &&
      next.edit.inFlight === 0 &&
      next.snapshot[SNAPSHOT.openCinderPortals] === 7 &&
      next.snapshot[SNAPSHOT.streamPlanOverflow] === 0 &&
      next.snapshot[SNAPSHOT.streamInterestTruncated] === 0 &&
      next.snapshot[SNAPSHOT.unreachablePortalActive] === 0 &&
      next.snapshot[SNAPSHOT.portalActiveChunks] ===
        next.snapshot[SNAPSHOT.streamInterestDesired] &&
      next.snapshot[SNAPSHOT.streamInterestDesired] ===
        next.snapshot[SNAPSHOT.streamInterestRequested] &&
      next.snapshot[SNAPSHOT.pendingJobs] === 0,
    60_000,
  );

  const snapshots = {
    far: far.snapshot,
    approach: approach.snapshot,
    descent: descent.snapshot,
    chamber: chamber.snapshot,
    overhead: overhead.snapshot,
    sealedOutside: sealedOutside.snapshot,
    sealedInside: sealedInside.snapshot,
    restoredInside: restoredInside.snapshot,
  };
  const summarize = (snapshot) => ({
    tracked: snapshot[SNAPSHOT.trackedChunks],
    requested: snapshot[SNAPSHOT.streamInterestRequested],
    desired: snapshot[SNAPSHOT.streamInterestDesired],
    activeChunks: snapshot[SNAPSHOT.portalActiveChunks],
    activeColumns: snapshot[SNAPSHOT.portalActiveColumns],
    openPortals: snapshot[SNAPSHOT.openCinderPortals],
    revision: snapshot[SNAPSHOT.cinderPortalRevision],
    truncated: snapshot[SNAPSHOT.streamInterestTruncated],
    unreachableActive: snapshot[SNAPSHOT.unreachablePortalActive],
  });
  const result = Object.fromEntries(
    Object.entries(snapshots).map(([name, snapshot]) => [name, summarize(snapshot)]),
  );
  const performance = phaseSummary(await sample(page, 4_000));
  const violations = [];
  for (const [name, phase] of Object.entries(result)) {
    if (phase.tracked > 320) violations.push(`${name}: tracked chunk bound exceeded`);
    if (phase.truncated !== 0) violations.push(`${name}: stream interest was truncated`);
    if (phase.unreachableActive !== 0) {
      violations.push(`${name}: unreachable portal chunks remained active`);
    }
  }
  if (performance.frameMs.p95 > 12) violations.push("settled frame p95 exceeded 12ms");
  if (performance.frameMs.above33_33ms > 0) violations.push("a settled frame exceeded 33.33ms");
  if (performance.droppedSamples > 0) violations.push("settled frame samples were dropped");
  if (violations.length > 0) {
    throw new Error(
      `portal streaming profile violations: ${violations.join(", ")}; ${JSON.stringify({ phases: result, performance })}`,
    );
  }
  return { phases: result, performance };
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

async function setScreenSpaceAmbientOcclusion(page, enabled, viewportWidth) {
  const current = await page.evaluate(
    (index) => globalThis.__VOXELS__.snapshot().then((snapshot) => snapshot[index] === 1),
    SNAPSHOT.screenSpaceAmbientOcclusion,
  );
  if (current === enabled) return;
  await page.keyboard.press("Escape");
  await page.keyboard.press("F3");
  await page.waitForTimeout(200);
  // The third Rust-owned feature row is screen-space contact AO. This deliberately exercises the
  // same canvas hit-testing path as a human and does not expose a JavaScript renderer setter.
  await page.mouse.click(viewportWidth - 57, 444);
  await page.waitForFunction(
    async ({ index, expected }) => {
      const snapshot = await globalThis.__VOXELS__.snapshot();
      return (snapshot[index] === 1) === expected;
    },
    { index: SNAPSHOT.screenSpaceAmbientOcclusion, expected: enabled },
    { timeout: 5_000 },
  );
  await page.keyboard.press("F3");
  await page.waitForTimeout(800);
}

async function visitFinalRouteMark(page, viewportWidth) {
  for (let ordinal = 0; ordinal < 5; ordinal += 1) {
    await page.keyboard.press("Escape");
    await page.keyboard.press("F3");
    await page.waitForTimeout(80);
    await page.mouse.click(viewportWidth - 83, 90);
    await page.waitForTimeout(80);
    // The sixth context row follows the next canonical pilgrim-road landmark.
    await page.mouse.click(viewportWidth - 171, 304);
    await page.keyboard.press("F3");
    await page.waitForTimeout(120);
  }
  await waitForEngine(page);
  await page.waitForTimeout(800);
}

async function ambientOcclusionProfile(page, viewportWidth) {
  await visitFinalRouteMark(page, viewportWidth);
  await setScreenSpaceAmbientOcclusion(page, false, viewportWidth);
  const off = phaseSummary(await sample(page, 5_000));
  await page.screenshot({ path: "target/spatial-ao-off.png" });
  await setScreenSpaceAmbientOcclusion(page, true, viewportWidth);
  const on = phaseSummary(await sample(page, 5_000));
  await page.screenshot({ path: "target/spatial-ao-on.png" });
  const delta = {
    frameP95Ms: on.frameMs.p95 - off.frameMs.p95,
    worldP95Ms: on.gpu.worldMs.p95 - off.gpu.worldMs.p95,
    totalP95Ms: on.gpu.totalMs.p95 - off.gpu.totalMs.p95,
    coreGpuMiB: on.coreGpuMiB - off.coreGpuMiB,
  };
  const invariantKeys = [
    "residentChunks",
    "visibleChunks",
    "quads",
    "waterQuads",
    "waterDrawCalls",
    "drawCalls",
    "meshArenaAllocatedMiB",
    "meshArenaCapacityMiB",
    "refractionCopyMiB",
    "ambientOcclusionMiB",
  ];
  const changed = invariantKeys.filter((key) => on[key] !== off[key]);
  const violations = [];
  if (!off.gpu.available || !on.gpu.available) violations.push("GPU timestamps unavailable");
  if (off.screenSpaceAmbientOcclusion || !on.screenSpaceAmbientOcclusion) {
    violations.push("Rust UI AO toggle state was not observed");
  }
  if (off.depthPrepassDrawCalls !== 0) violations.push("disabled AO submitted a depth prepass");
  if (on.depthPrepassDrawCalls !== on.drawCalls) {
    violations.push("depth ownership draw count did not match the opaque scene");
  }
  if (off.gpu.depthPrepassMs.p95 !== 0 || off.gpu.ambientOcclusionMs.p95 !== 0) {
    violations.push("disabled AO reported active GPU passes");
  }
  if (on.gpu.depthPrepassMs.p95 <= 0 || on.gpu.ambientOcclusionMs.p95 <= 0) {
    violations.push("enabled AO did not report both GPU stages");
  }
  if (on.gpu.depthPrepassMs.p95 + on.gpu.ambientOcclusionMs.p95 > 1.75) {
    violations.push("depth plus spatial AO GPU p95 exceeded 1.75ms");
  }
  if (on.gpu.totalMs.p95 > 7.5) violations.push("active GPU p95 exceeded 7.5ms");
  if (delta.worldP95Ms > 0.4) violations.push("world GPU p95 increased by more than 0.4ms");
  if (off.frameMs.p95 > 12 || on.frameMs.p95 > 12) violations.push("frame p95 exceeded 12ms");
  if (off.frameMs.above16_67ms > 0 || on.frameMs.above16_67ms > 0) {
    violations.push("paired AO profile missed the 120Hz frame gate");
  }
  if (off.droppedSamples > 0 || on.droppedSamples > 0) violations.push("frame samples dropped");
  if (changed.length > 0) {
    violations.push(`geometry/resource invariants changed: ${changed.join(", ")}`);
  }
  const result = { off, on, delta, invariantKeys };
  if (violations.length > 0) {
    throw new Error(
      `spatial AO profile violations: ${violations.join(", ")}; ${JSON.stringify(result)}`,
    );
  }
  return result;
}

async function visitNextLandmark(page, viewportWidth) {
  await page.keyboard.press("Escape");
  await page.keyboard.press("F3");
  await page.waitForTimeout(80);
  await page.mouse.click(viewportWidth - 83, 90);
  await page.waitForTimeout(80);
  // The eighth Rust context row advances the deterministic landmark catalog. TypeScript only
  // clicks the canvas; kind lookup, teleport placement, camera aim, and streaming remain Rust-owned.
  await page.mouse.click(viewportWidth - 171, 372);
  await page.keyboard.press("F3");
  await waitForEngine(page);
  await page.waitForTimeout(500);
}

async function visitCinderVault(page, viewportWidth) {
  const before = await capture(page);
  const previousPosition = [
    before.snapshot[SNAPSHOT.cameraX],
    before.snapshot[SNAPSHOT.cameraY],
    before.snapshot[SNAPSHOT.cameraZ],
  ];
  await page.keyboard.press("Escape");
  await page.keyboard.press("F3");
  await page.waitForTimeout(80);
  await page.mouse.click(viewportWidth - 83, 90);
  await page.waitForTimeout(80);
  // The seventh Rust context row advances the three-stop Cinder Vault tour. TypeScript only
  // supplies canvas input; Rust owns the tour state, poses, enclosure probe, and rendering.
  await page.mouse.click(viewportWidth - 171, 338);
  await page.keyboard.press("F3");
  await page.waitForFunction(
    async ({ indices, previous }) => {
      const snapshot = await globalThis.__VOXELS__.snapshot();
      const dx = snapshot[indices.x] - previous[0];
      const dy = snapshot[indices.y] - previous[1];
      const dz = snapshot[indices.z] - previous[2];
      return dx * dx + dy * dy + dz * dz > 1;
    },
    {
      indices: { x: SNAPSHOT.cameraX, y: SNAPSHOT.cameraY, z: SNAPSHOT.cameraZ },
      previous: previousPosition,
    },
    { timeout: 5_000 },
  );
  // Let the new camera pose schedule its first streaming pass before checking for a drained queue.
  await page.waitForTimeout(150);
  await waitForEngine(page);
}

async function waitForCaveAdaptation(page, minimumEnclosure, minimumExposure) {
  await page.waitForFunction(
    async ({ enclosureIndex, exposureIndex, minimumEnclosure, minimumExposure }) => {
      const snapshot = await globalThis.__VOXELS__.snapshot();
      return (
        snapshot[enclosureIndex] >= minimumEnclosure && snapshot[exposureIndex] >= minimumExposure
      );
    },
    {
      enclosureIndex: SNAPSHOT.enclosure,
      exposureIndex: SNAPSHOT.interiorExposure,
      minimumEnclosure,
      minimumExposure,
    },
    { timeout: 12_000 },
  );
}

async function caveProfile(page, viewportWidth) {
  const stops = [
    { name: "approach", minimumEnclosure: 0, minimumExposure: 0.99, headlamp: false },
    { name: "descent", minimumEnclosure: 0.18, minimumExposure: 1.15, headlamp: false },
    { name: "chamber", minimumEnclosure: 0.75, minimumExposure: 1.35, headlamp: true },
  ];
  const phases = {};
  const violations = [];

  for (const stop of stops) {
    await visitCinderVault(page, viewportWidth);
    await waitForCaveAdaptation(page, stop.minimumEnclosure, stop.minimumExposure);
    // At least 20 timestamp samples keep p95 distinct from a single scheduling outlier.
    const captures = await sample(page, 6_000);
    const phase = phaseSummary(captures);
    phase.cave.probe = summary(
      captures.map((capture) => capture.snapshot[SNAPSHOT.enclosureProbeUs]),
    );
    phase.streaming = {
      maxPendingJobs: Math.max(
        ...captures.map((capture) => capture.snapshot[SNAPSHOT.pendingJobs]),
        0,
      ),
      maxPendingMeshMiB: Math.max(
        ...captures.map((capture) => capture.snapshot[SNAPSHOT.pendingMeshMiB]),
        0,
      ),
      maxStaleCompletions: Math.max(
        ...captures.map((capture) => capture.snapshot[SNAPSHOT.staleCompletions]),
        0,
      ),
    };
    phases[stop.name] = phase;
    await page.screenshot({ path: `target/cinder-vault-${stop.name}.png` });

    if (phase.cave.headlamp !== stop.headlamp) {
      violations.push(`${stop.name}: automatic headlamp transition was incorrect`);
    }
    if (phase.pendingJobs !== 0 || phase.streaming.maxPendingJobs !== 0) {
      violations.push(`${stop.name}: streaming queues did not stay drained`);
    }
    if (phase.memory.pendingMeshMiB !== 0 || phase.streaming.maxPendingMeshMiB !== 0) {
      violations.push(`${stop.name}: pending mesh payload did not stay drained`);
    }
    if (phase.staleCompletions !== 0 || phase.streaming.maxStaleCompletions !== 0) {
      violations.push(`${stop.name}: stale streaming work completed`);
    }
    if (phase.droppedSamples !== 0) violations.push(`${stop.name}: frame samples were dropped`);
    if (phase.frameMs.p95 > 12) violations.push(`${stop.name}: frame p95 exceeded 12ms`);
    if (phase.frameMs.above33_33ms > 0) {
      violations.push(`${stop.name}: a frame exceeded 33.33ms`);
    }
    if (phase.gpu.available && phase.gpu.totalMs.p95 > 7.5) {
      violations.push(`${stop.name}: active GPU p95 exceeded 7.5ms`);
    }
    if (phase.cave.probe.max > 1_000) {
      violations.push(`${stop.name}: enclosure probe exceeded 1ms`);
    }
  }

  const { approach, descent, chamber } = phases;
  if (approach.cave.enclosure > 0.15) {
    violations.push("approach: enclosure did not return to an outdoor state");
  }
  if (approach.cave.exposure < 0.98 || approach.cave.exposure > 1.08) {
    violations.push("approach: interior exposure did not return to its outdoor range");
  }
  if (descent.cave.enclosure < 0.18 || descent.cave.exposure < 1.15) {
    violations.push("descent: tunnel did not engage enclosure adaptation");
  }
  if (chamber.cave.enclosure < 0.75 || chamber.cave.exposure < 1.35) {
    violations.push("chamber: deep interior adaptation did not converge");
  }
  if (chamber.cave.enclosure + 0.05 < descent.cave.enclosure) {
    violations.push("chamber: enclosure regressed materially from the descent");
  }
  if (approach.localLights.active !== 0 || descent.localLights.active !== 0) {
    violations.push("approach/descent: out-of-range voxel lights became active");
  }
  if (chamber.localLights.active < 2 || chamber.localLights.active > 16) {
    violations.push("chamber: active voxel light count left the bounded 2..16 range");
  }
  if (chamber.localLights.clipped !== 0) {
    violations.push("chamber: natural authored lights exceeded the active budget");
  }
  if (chamber.localLights.occluded !== 0) {
    violations.push("chamber: connected authored lights were rejected by voxel visibility");
  }

  if (violations.length > 0) {
    throw new Error(
      `Cinder Vault profile violations: ${violations.join(", ")}; ${JSON.stringify(phases)}`,
    );
  }
  return phases;
}

async function setCaveHeadlamp(page, enabled, viewportWidth) {
  const current = await page.evaluate(
    (index) => globalThis.__VOXELS__.snapshot().then((snapshot) => snapshot[index] === 1),
    SNAPSHOT.caveHeadlamp,
  );
  if (current === enabled) return;
  await page.keyboard.press("Escape");
  await page.keyboard.press("F3");
  await page.waitForTimeout(200);
  await page.mouse.click(viewportWidth - 57, 639);
  await page.waitForFunction(
    async ({ index, expected }) => {
      const snapshot = await globalThis.__VOXELS__.snapshot();
      return (snapshot[index] === 1) === expected;
    },
    { index: SNAPSHOT.caveHeadlamp, expected: enabled },
    { timeout: 5_000 },
  );
  await page.keyboard.press("F3");
  await page.waitForTimeout(500);
}

async function setLocalLighting(page, enabled, viewportWidth) {
  const current = await page.evaluate(
    (index) => globalThis.__VOXELS__.snapshot().then((snapshot) => snapshot[index] === 1),
    SNAPSHOT.localLighting,
  );
  if (current === enabled) return;
  await page.keyboard.press("Escape");
  await page.keyboard.press("F3");
  await page.waitForTimeout(200);
  await page.mouse.click(viewportWidth - 57, 672);
  await page.waitForFunction(
    async ({ index, expected }) => {
      const snapshot = await globalThis.__VOXELS__.snapshot();
      return (snapshot[index] === 1) === expected;
    },
    { index: SNAPSHOT.localLighting, expected: enabled },
    { timeout: 5_000 },
  );
  await page.keyboard.press("F3");
  await page.waitForTimeout(500);
}

async function localLightProfile(page, viewportWidth) {
  for (let stop = 0; stop < 3; stop += 1) await visitCinderVault(page, viewportWidth);
  await waitForCaveAdaptation(page, 0.75, 1.35);
  await setCaveHeadlamp(page, false, viewportWidth);
  await setLocalLighting(page, false, viewportWidth);
  const off = phaseSummary(await sample(page, 6_000));
  await page.screenshot({ path: "target/cinder-vault-local-lights-off.png" });
  await setLocalLighting(page, true, viewportWidth);
  const on = phaseSummary(await sample(page, 6_000));
  await page.screenshot({ path: "target/cinder-vault-local-lights-on.png" });

  const invariantKeys = [
    "residentChunks",
    "visibleChunks",
    "quads",
    "waterQuads",
    "drawCalls",
    "meshArenaAllocatedMiB",
    "meshArenaCapacityMiB",
  ];
  const changed = invariantKeys.filter((key) => off[key] !== on[key]);
  const delta = {
    worldP95Ms: on.gpu.worldMs.p95 - off.gpu.worldMs.p95,
    totalP95Ms: on.gpu.totalMs.p95 - off.gpu.totalMs.p95,
    frameP95Ms: on.frameMs.p95 - off.frameMs.p95,
  };
  const violations = [];
  if (off.cave.headlamp || on.cave.headlamp) violations.push("headlamp was not isolated off");
  if (off.localLights.enabled || off.localLights.active !== 0) {
    violations.push("disabled local lights remained active");
  }
  if (!on.localLights.enabled || on.localLights.active < 2 || on.localLights.active > 16) {
    violations.push("enabled active light count left the bounded 2..16 range");
  }
  if (off.localLights.candidates !== on.localLights.candidates || on.localLights.candidates < 2) {
    violations.push("resident light candidates changed across the render-only toggle");
  }
  if (on.localLights.clipped !== 0) violations.push("authored chamber lights were clipped");
  if (on.localLights.occluded !== 0) {
    violations.push("connected chamber lights were rejected by voxel visibility");
  }
  if (changed.length > 0)
    violations.push(`geometry/resource invariants changed: ${changed.join(", ")}`);
  if (!off.gpu.available || !on.gpu.available) violations.push("GPU timestamps unavailable");
  if (delta.worldP95Ms > 1.25) violations.push("world GPU p95 increased by more than 1.25ms");
  if (delta.totalP95Ms > 1.5 || on.gpu.totalMs.p95 > 7.5) {
    violations.push("active GPU local-light budget was exceeded");
  }
  if (off.frameMs.p95 > 12 || on.frameMs.p95 > 12) violations.push("frame p95 exceeded 12ms");
  if (off.frameMs.above33_33ms > 0 || on.frameMs.above33_33ms > 0) {
    violations.push("a paired local-light frame exceeded 33.33ms");
  }
  if (off.droppedSamples > 0 || on.droppedSamples > 0) violations.push("frame samples dropped");
  const result = { off, on, delta, invariantKeys };
  if (violations.length > 0) {
    throw new Error(
      `local-light profile violations: ${violations.join(", ")}; ${JSON.stringify(result)}`,
    );
  }
  return result;
}

async function cavePortalProfile(page, viewportWidth) {
  for (let stop = 0; stop < 4; stop += 1) await visitCinderVault(page, viewportWidth);
  const phase = phaseSummary(await sample(page, 6_000));
  await page.screenshot({ path: "target/cinder-vault-overhead-portal-gate.png" });
  const violations = [];
  if (phase.cinderPortals.open !== 7 || phase.cinderPortals.revision !== 0) {
    violations.push("pristine Cinder portal topology was not fully open and revision-zero");
  }
  if (phase.localLights.candidates < 2) {
    violations.push("overhead view did not retain chamber light candidates");
  }
  if (phase.localLights.active !== 0) {
    violations.push("chamber lights crossed the exterior shell");
  }
  if (phase.localLights.portalRejected < 2) {
    violations.push("portal geodesic did not reject chamber candidates");
  }
  if (phase.localLights.visibilityTests > 32 || phase.localLights.clipped !== 0) {
    violations.push("portal visibility escaped its bounded test/light budget");
  }
  if (phase.pendingJobs !== 0 || phase.memory.pendingMeshMiB !== 0) {
    violations.push("overhead portal profile did not settle streaming");
  }
  if (phase.droppedSamples !== 0 || phase.frameMs.p95 > 12 || phase.frameMs.above33_33ms > 0) {
    violations.push("overhead portal profile missed its frame gate");
  }
  if (phase.gpu.available && phase.gpu.totalMs.p95 > 7.5) {
    violations.push("overhead portal profile exceeded the active GPU gate");
  }
  if (violations.length > 0) {
    throw new Error(
      `Cinder portal profile violations: ${violations.join(", ")}; ${JSON.stringify(phase)}`,
    );
  }
  return phase;
}

async function semanticHeroProfile(page, viewportWidth) {
  // Skip the six regional background forms and three route forms. The append-only hero ids occupy
  // the final six positions in the Rust catalog.
  for (let index = 0; index < 9; index += 1) {
    await visitNextLandmark(page, viewportWidth);
  }
  const slugs = [
    "elder-canopy",
    "tor-circle",
    "needle-gate",
    "buried-ribs",
    "buried-colonnade",
    "basalt-crown",
  ];
  const heroes = {};
  const violations = [];
  for (const slug of slugs) {
    await visitNextLandmark(page, viewportWidth);
    const phase = phaseSummary(await sample(page, 3_000));
    heroes[slug] = phase;
    await page.screenshot({ path: `target/semantic-hero-${slug}.png` });
    if (!phase.gpu.available) violations.push(`${slug}: GPU timestamps unavailable`);
    if (phase.pendingJobs !== 0) violations.push(`${slug}: streaming did not settle`);
    if (phase.droppedSamples > 0) violations.push(`${slug}: frame samples were dropped`);
    if (phase.frameMs.p95 > 12) violations.push(`${slug}: frame p95 exceeded 12ms`);
    if (phase.frameMs.above33_33ms > 0) violations.push(`${slug}: a frame exceeded 33.33ms`);
    if (phase.gpu.available && phase.gpu.totalMs.p95 > 7.5) {
      violations.push(`${slug}: active GPU p95 exceeded 7.5ms`);
    }
    if (phase.depthPrepassDrawCalls + phase.waterDrawCalls !== phase.drawCalls) {
      violations.push(`${slug}: AO depth ownership did not match opaque draws`);
    }
    if (phase.staleCompletions !== 0) violations.push(`${slug}: stale work completed`);
  }
  if (violations.length > 0) {
    throw new Error(
      `semantic hero profile violations: ${violations.join(", ")}; ${JSON.stringify(heroes)}`,
    );
  }
  return heroes;
}

async function cycleDaylight(page, viewportWidth, expectedPhase) {
  await page.keyboard.press("Escape");
  await page.keyboard.press("F3");
  await page.waitForTimeout(100);
  await page.mouse.click(viewportWidth - 83, 90);
  await page.waitForTimeout(100);
  // The fifth Rust context row is "Cycle regional daylight". This intentionally exercises canvas
  // hit-testing rather than adding a browser-side debug setter for atmosphere state.
  await page.mouse.click(viewportWidth - 171, 270);
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

async function enterUnderwaterShowcase(page, viewportWidth) {
  await page.keyboard.press("Escape");
  await page.keyboard.press("F3");
  await page.waitForTimeout(100);
  await page.mouse.click(viewportWidth - 83, 90);
  await page.waitForTimeout(100);
  await page.mouse.click(viewportWidth - 171, 168);
  await page.waitForFunction(
    async (indices) => {
      const snapshot = await globalThis.__VOXELS__.snapshot();
      return snapshot[indices.immersion] > 0.5 && snapshot[indices.eyesSubmerged] === 1;
    },
    { immersion: SNAPSHOT.immersion, eyesSubmerged: SNAPSHOT.eyesSubmerged },
    { timeout: 20_000 },
  );
  await page.keyboard.press("F3");
  await page.waitForTimeout(500);
}

const viewport = { width: 1280, height: 720 };
const sustained = process.argv.includes("--sustained");
const materials = process.argv.includes("--materials");
const atmosphere = process.argv.includes("--atmosphere");
const ambientOcclusion = process.argv.includes("--gtao");
const semanticHeroes = process.argv.includes("--heroes");
const caves = process.argv.includes("--caves");
const localLights = process.argv.includes("--lights");
const cavePortals = process.argv.includes("--portals");
const cavePortalEdits = process.argv.includes("--portal-edits");
const cavePortalStreaming = process.argv.includes("--portal-streaming");
const errors = [];
const port = await reserveEphemeralPort();
let browser;
let server;

function observePageErrors(page) {
  page.on("pageerror", (error) => errors.push(`pageerror: ${error.message}`));
  page.on("console", (message) => {
    if (isBrowserConsoleFailure(message.type(), message.text(), FAILURE)) {
      errors.push(`${message.type()}: ${message.text()}`);
    }
  });
}

try {
  await build({ logLevel: "warn" });
  server = await preview({
    logLevel: "warn",
    preview: { host: "127.0.0.1", port, strictPort: true },
  });
  browser = await chromium.launch(chromeWebGpuLaunchOptions());
  const context = await browser.newContext({ viewport, deviceScaleFactor: 1 });
  const page = await context.newPage();
  observePageErrors(page);
  const navigationStarted = performance.now();
  await page.goto(`http://127.0.0.1:${port}`, { waitUntil: "domcontentloaded" });
  await waitForEngine(page);
  const settledMilliseconds = performance.now() - navigationStarted;

  let scenarios;
  if (cavePortalEdits) {
    scenarios = { cavePortalEdits: await cavePortalEditPersistenceProfile(page, context) };
  } else if (cavePortalStreaming) {
    scenarios = { cavePortalStreaming: await cavePortalStreamingProfile(page, viewport.width) };
  } else if (sustained) {
    scenarios = { sustained: await sustainedProfile(page) };
  } else if (materials) {
    scenarios = { materials: await materialDetailProfile(page, viewport.width) };
  } else if (atmosphere) {
    scenarios = { atmosphere: await atmosphereProfile(page, viewport.width) };
  } else if (ambientOcclusion) {
    scenarios = { ambientOcclusion: await ambientOcclusionProfile(page, viewport.width) };
  } else if (semanticHeroes) {
    scenarios = { semanticHeroes: await semanticHeroProfile(page, viewport.width) };
  } else if (caves) {
    scenarios = { caves: await caveProfile(page, viewport.width) };
  } else if (localLights) {
    scenarios = { localLights: await localLightProfile(page, viewport.width) };
  } else if (cavePortals) {
    scenarios = { cavePortals: await cavePortalProfile(page, viewport.width) };
  } else {
    const steady = phaseSummary(await sample(page, 4_000));
    await page.keyboard.down("KeyW");
    const traversalSamples = await sample(page, 6_000);
    await page.keyboard.up("KeyW");
    const traversal = phaseSummary(traversalSamples);
    await enterUnderwaterShowcase(page, viewport.width);
    const underwater = phaseSummary(await sample(page, 4_000));
    scenarios = { steady, traversal, underwater };
  }
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
        ...scenarios,
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
