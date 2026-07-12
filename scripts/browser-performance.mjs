import { createServer as createNetServer } from "node:net";
import { chromium } from "playwright";
import { build, preview } from "vite-plus";

const FAILURE =
  /panic|unreachable|runtimeerror|wgpu|webgpu|shader|sqlite|opfs|syncaccesshandle|nomodificationallowed|web lock request failed|no persistence leader|persistence .*failed|landmark tour/i;
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
  profilePhase: 56,
  profileElapsedSeconds: 57,
  profileDistanceMetres: 58,
  profileComplete: 59,
  profileTrackedHigh: 60,
  profileSurfaceHigh: 61,
  profilePendingHigh: 62,
  profilePendingMeshHigh: 63,
  profileArenaCapacityHighMiB: 64,
  profileWasmHighMiB: 65,
  profileEvictions: 66,
  materialDetail: 67,
  daylightPhase: 68,
  surfaceRegion: 69,
  cloudCoverage: 70,
  screenSpaceAmbientOcclusion: 71,
  gpuDepthPrepassMs: 72,
  gpuAmbientOcclusionMs: 73,
  ambientOcclusionMiB: 74,
  depthPrepassDrawCalls: 75,
  enclosure: 76,
  interiorExposure: 77,
  caveHeadlamp: 78,
  enclosureProbeUs: 79,
  schemaVersion: 80,
  sampleCount: 81,
  droppedSamples: 82,
};
const FRAME_SAMPLE_WIDTH = 5;
const FRAME_SAMPLE_START = 83;
const EDIT_SAMPLE_WIDTH = 6;

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
  const editStart = FRAME_SAMPLE_START + count * FRAME_SAMPLE_WIDTH;
  const editCount = snapshot[editStart + 9];
  const editSamples = [];
  for (let index = 0; index < editCount; index += 1) {
    const start = editStart + 11 + index * EDIT_SAMPLE_WIDTH;
    editSamples.push(snapshot.slice(start, start + EDIT_SAMPLE_WIDTH));
  }
  return {
    snapshot,
    samples,
    dropped: snapshot[SNAPSHOT.droppedSamples],
    edit: {
      phase: snapshot[editStart],
      nextOperation: snapshot[editStart + 1],
      totalOperations: snapshot[editStart + 2],
      restored: snapshot[editStart + 3] === 1,
      baselineEdits: snapshot[editStart + 4],
      currentEdits: snapshot[editStart + 5],
      inFlight: snapshot[editStart + 6],
      completed: snapshot[editStart + 7],
      superseded: snapshot[editStart + 8],
      dropped: snapshot[editStart + 10],
      samples: editSamples,
    },
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

async function editProfile(page) {
  await page.evaluate(() => globalThis.__VOXELS__.profile(2));
  const captures = [];
  const samples = [];
  const deadline = Date.now() + 60_000;
  while (Date.now() < deadline) {
    const next = await capture(page);
    captures.push(next);
    samples.push(...next.edit.samples);
    if (next.edit.phase === 3 || next.edit.phase === 4) break;
    await page.waitForTimeout(50);
  }
  const latest = captures.at(-1);
  if (!latest || latest.edit.phase !== 3) {
    throw new Error(`Rust edit profile did not converge: ${JSON.stringify(latest?.edit)}`);
  }
  const column = (index) => samples.map((sample) => sample[index]);
  const result = {
    operations: samples.length,
    frame: phaseSummary(captures).frameMs,
    enqueueMs: summary(column(3)),
    canonicalMs: summary(column(4)),
    fullConvergenceMs: summary(column(5)),
    final: {
      restored: latest.edit.restored,
      baselineEdits: latest.edit.baselineEdits,
      currentEdits: latest.edit.currentEdits,
      inFlight: latest.edit.inFlight,
      completed: latest.edit.completed,
      superseded: latest.edit.superseded,
      droppedSamples: captures.reduce((total, capture) => total + capture.edit.dropped, 0),
      pendingJobs: latest.snapshot[SNAPSHOT.pendingJobs],
      pendingMeshMiB: latest.snapshot[SNAPSHOT.pendingMeshMiB],
    },
  };
  const violations = [];
  const ordinals = column(0);
  if (samples.length !== 40) violations.push("did not record exactly 40 operations");
  if (!ordinals.every((ordinal, index) => ordinal === index + 1)) {
    violations.push("operation sequence is incomplete or reordered");
  }
  for (const targetClass of [1, 2]) {
    for (const operation of [1, 2]) {
      const countForKind = samples.filter(
        (sample) => sample[1] === targetClass && sample[2] === operation,
      ).length;
      if (countForKind !== 10) {
        violations.push(`class ${targetClass} operation ${operation} count was ${countForKind}`);
      }
    }
  }
  if (!result.final.restored || result.final.currentEdits !== result.final.baselineEdits) {
    violations.push("sparse edit map was not restored to its pristine baseline");
  }
  if (result.final.inFlight !== 0) violations.push("edit tracker did not drain");
  if (result.final.completed !== 40) violations.push("completion count was not 40");
  if (result.final.superseded !== 0) violations.push("an edit was superseded");
  if (result.final.droppedSamples !== 0) violations.push("edit samples were dropped");
  if (result.final.pendingJobs !== 0) violations.push("streaming queues did not drain");
  if (result.final.pendingMeshMiB !== 0) violations.push("pending mesh payload did not drain");
  if (result.enqueueMs.p95 > 8 || result.enqueueMs.max > 25) {
    violations.push("persistence enqueue latency exceeded 8ms p95 or 25ms max");
  }
  if (result.canonicalMs.p95 > 100 || result.canonicalMs.max > 200) {
    violations.push("canonical replacement exceeded 100ms p95 or 200ms max");
  }
  if (result.fullConvergenceMs.p95 > 150 || result.fullConvergenceMs.max > 250) {
    violations.push("full LOD convergence exceeded 150ms p95 or 250ms max");
  }
  if (result.frame.p95 > 16.67 || result.frame.above33_33ms > 0) {
    violations.push("edit phase missed the interactive frame-time gate");
  }
  if (violations.length > 0) {
    throw new Error(`edit profile violations: ${violations.join(", ")}; ${JSON.stringify(result)}`);
  }
  return result;
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
  await page.mouse.click(viewportWidth - 57, 639);
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
  // The seventh Rust context row advances the deterministic landmark catalog. TypeScript only
  // clicks the canvas; kind lookup, teleport placement, camera aim, and streaming remain Rust-owned.
  await page.mouse.click(viewportWidth - 171, 338);
  await page.keyboard.press("F3");
  await waitForEngine(page);
  await page.waitForTimeout(500);
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
      snapshot[SNAPSHOT.schemaVersion] === 10 &&
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
const sustained = process.argv.includes("--sustained");
const edits = process.argv.includes("--edits");
const materials = process.argv.includes("--materials");
const atmosphere = process.argv.includes("--atmosphere");
const ambientOcclusion = process.argv.includes("--gtao");
const semanticHeroes = process.argv.includes("--heroes");
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

  let scenarios;
  if (edits) {
    scenarios = { edits: await editProfile(page) };
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
