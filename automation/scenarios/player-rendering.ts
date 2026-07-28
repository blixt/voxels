import type { Page } from "playwright";
import { BrowserCapability } from "../lib/browser.ts";
import { type EngineClient, snapshotValue } from "../lib/engine.ts";
import { analyzeDiagnosticSky } from "../lib/image.ts";
import { summarizeSurfaceCutAdjacency, takePlayerScreenshot } from "../lib/player-screenshot.ts";
import { defineScenario, type ScenarioContext } from "../lib/scenario.ts";
import { startWorldStack } from "../lib/world.ts";

const VIEWPORT = { width: 960, height: 540 };
const STABLE_CUT_DURATION_MS = 4_000;
const STABILITY_POLL_MS = 200;
const STABILITY_TIMEOUT_MS = 45_000;

interface AuditedCapture {
  readonly exactPages: number;
  readonly cutFingerprint: string;
  readonly largestEnclosedSkyComponent: number;
}

async function auditCapture(
  context: ScenarioContext,
  page: Page,
  engine: EngineClient,
  label: string,
  frame: readonly number[],
): Promise<AuditedCapture> {
  const png = await page.screenshot({ type: "png" });
  await context.artifacts.write(label, `${label}.png`, png, "image/png");
  const sky = await analyzeDiagnosticSky(page, png);
  const exactPages = snapshotValue(frame, "virtualTerrainPublishedExactPages");
  if (
    snapshotValue(frame, "terrainReady") !== 1 ||
    exactPages === 0 ||
    snapshotValue(frame, "virtualTerrainPublishedMinimumLevel") !== 0
  ) {
    throw new Error(`${label} did not retain its exact playable terrain vicinity`);
  }
  if (snapshotValue(frame, "virtualTerrainPublishedExactLodDiscontinuities") !== 0) {
    throw new Error(
      `${label} published a skipped hierarchy level at the exact-player transition frontier`,
    );
  }
  if (sky.largestEnclosedComponentPixels > 0) {
    await engine.setGeometrySourceDebug(true);
    let diagnostic;
    try {
      diagnostic = await takePlayerScreenshot(page);
      await context.artifacts.write(
        `${label} source and LOD ownership`,
        `${label}-source-ownership.png`,
        diagnostic.png,
        "image/png",
      );
    } finally {
      await engine.setGeometrySourceDebug(false);
    }
    const cut = diagnostic.metadata.presentation.selectedCut.cut;
    const firstHole = sky.enclosedSampleCoordinates[0];
    const machineEvidence = {
      exactSurfaceLodDiscontinuities: cut?.exactSurfaceLodDiscontinuities,
      ownerlessRoots: cut?.ownerlessRoots.length,
      allCutAdjacency: summarizeSurfaceCutAdjacency(cut?.selectedPages ?? []),
      nearCutAdjacency: summarizeSurfaceCutAdjacency(
        cut?.selectedPages ?? [],
        diagnostic.metadata.camera.eyeMetres,
        12.8,
      ),
      nearbyPixels: diagnostic.ownership?.summarizeNearby(
        diagnostic.metadata.camera.eyeMetres,
        12.8,
      ),
      holeNeighborhood:
        firstHole === undefined
          ? []
          : diagnostic.ownership?.sampleNeighborhood(firstHole[0], firstHole[1], 3),
    };
    throw new Error(
      `${label} exposed a ${sky.largestEnclosedComponentPixels}-pixel enclosed magenta terrain hole at ${JSON.stringify(sky.enclosedSampleCoordinates)}; ${JSON.stringify(machineEvidence)}`,
    );
  }
  return {
    exactPages,
    cutFingerprint: `${snapshotValue(frame, "virtualTerrainCutFingerprintHigh24")}:${snapshotValue(frame, "virtualTerrainCutFingerprintLow24")}`,
    largestEnclosedSkyComponent: sky.largestEnclosedComponentPixels,
  };
}

async function shortPlayerStep(
  page: Page,
  capturePosition: () => Promise<readonly number[]>,
): Promise<number> {
  const before = await capturePosition();
  await page.keyboard.down("KeyW");
  await page.waitForTimeout(180);
  await page.keyboard.up("KeyW");
  await page.waitForTimeout(100);
  const after = await capturePosition();
  const distance = Math.hypot(
    snapshotValue(after, "cameraX") - snapshotValue(before, "cameraX"),
    snapshotValue(after, "cameraZ") - snapshotValue(before, "cameraZ"),
  );
  if (distance < 0.02) {
    throw new Error("the real W-key step was ignored because the player was not yet playable");
  }
  return distance;
}

async function walkBeyondProtectedPedestal(
  page: Page,
  capturePosition: () => Promise<readonly number[]>,
  targetMetres: number,
): Promise<number> {
  const before = await capturePosition();
  await page.keyboard.down("ShiftLeft");
  await page.keyboard.down("KeyW");
  let distance = 0;
  try {
    const deadline = performance.now() + 20_000;
    while (performance.now() < deadline) {
      await page.waitForTimeout(50);
      const current = await capturePosition();
      distance = Math.hypot(
        snapshotValue(current, "cameraX") - snapshotValue(before, "cameraX"),
        snapshotValue(current, "cameraZ") - snapshotValue(before, "cameraZ"),
      );
      if (distance >= targetMetres) break;
    }
  } finally {
    await page.keyboard.up("KeyW");
    await page.keyboard.up("ShiftLeft");
  }
  if (distance < targetMetres) {
    throw new Error(
      `player moved only ${distance.toFixed(2)}m of the requested ${targetMetres.toFixed(2)}m travel`,
    );
  }
  return distance;
}

async function stablePhaseCapture(
  context: ScenarioContext,
  page: Page,
  phase: string,
  engine: EngineClient,
  assertHealthy: () => void,
): Promise<AuditedCapture> {
  const started = performance.now();
  let stableSince = started;
  let lastLog = 0;
  let previousFingerprint = "";
  while (performance.now() - started < STABILITY_TIMEOUT_MS) {
    const current = await engine.snapshot();
    assertHealthy();
    const exactPages = snapshotValue(current, "virtualTerrainPublishedExactPages");
    const fingerprint = `${snapshotValue(current, "virtualTerrainCutFingerprintHigh24")}:${snapshotValue(current, "virtualTerrainCutFingerprintLow24")}`;
    if (fingerprint !== previousFingerprint) {
      previousFingerprint = fingerprint;
      stableSince = performance.now();
    }
    const state = {
      frameSequence: snapshotValue(current, "frameSequence"),
      terrainReady: snapshotValue(current, "terrainReady"),
      renderMode: snapshotValue(current, "virtualTerrainMode"),
      registeredRegions: snapshotValue(current, "virtualTerrainRegisteredRegions"),
      directoryNodes: snapshotValue(current, "virtualTerrainDirectoryNodes"),
      residentPages: snapshotValue(current, "virtualTerrainResidentPages"),
      residentMiB: snapshotValue(current, "virtualTerrainResidentMiB"),
      residentPrimitives: snapshotValue(current, "virtualTerrainResidentPrimitives"),
      publishedPages: snapshotValue(current, "virtualTerrainPublishedPages"),
      exactPages,
      selectedPages: snapshotValue(current, "virtualTerrainSelectedPages"),
      requestedPages: snapshotValue(current, "virtualTerrainRequestedPages"),
      ownerlessRoots: snapshotValue(current, "virtualTerrainOwnerlessRoots"),
      pendingPages: snapshotValue(current, "virtualTerrainStreamPending"),
      inFlightPages: snapshotValue(current, "virtualTerrainStreamInFlight"),
      columns: snapshotValue(current, "virtualTerrainColumns"),
      columnInFlight: snapshotValue(current, "virtualTerrainColumnInFlight"),
      columnRevisionFloors: snapshotValue(current, "virtualTerrainColumnRevisionFloors"),
      currentColumnKnown: snapshotValue(current, "virtualTerrainCurrentColumnKnown"),
      currentColumnRoots: snapshotValue(current, "virtualTerrainCurrentColumnRoots"),
      currentColumnRegisteredRoots: snapshotValue(
        current,
        "virtualTerrainCurrentColumnRegisteredRoots",
      ),
      nearestRegisteredRootMetres: snapshotValue(
        current,
        "virtualTerrainNearestRegisteredRootMetres",
      ),
      gpuMatchesCpu: snapshotValue(current, "virtualTerrainGpuMatchesCpuCut"),
      arenaAllocatedMiB: snapshotValue(current, "arenaAllocatedMiB"),
      arenaCapacityMiB: snapshotValue(current, "arenaCapacityMiB"),
      editCanonicalRequired: snapshotValue(current, "editCanonicalRequired"),
      editCanonicalRenderable: snapshotValue(current, "editCanonicalRenderable"),
      editCanonicalOwned: snapshotValue(current, "editCanonicalOwned"),
    };
    if (performance.now() - lastLog >= 5_000) {
      context.log(`${phase} convergence ${JSON.stringify(state)}`);
      lastLog = performance.now();
    }
    if (
      state.terrainReady === 1 &&
      state.exactPages > 0 &&
      state.columnRevisionFloors === 0 &&
      state.currentColumnRegisteredRoots > 0 &&
      state.editCanonicalRenderable >= state.editCanonicalRequired &&
      state.editCanonicalOwned >= state.editCanonicalRequired &&
      performance.now() - stableSince >= STABLE_CUT_DURATION_MS
    ) {
      const capture = await auditCapture(context, page, engine, phase, current);
      assertHealthy();
      return capture;
    }
    await page.waitForTimeout(STABILITY_POLL_MS);
  }
  const failure = await takePlayerScreenshot(page);
  await context.artifacts.write(
    `${phase} convergence failure reproduction`,
    `${phase}-convergence-failure.png`,
    failure.png,
    "image/png",
  );
  const cut = failure.metadata.presentation.selectedCut.cut;
  throw new Error(
    `${phase} did not reach a quiescent published cut within 45 seconds; ` +
      `published adjacency ${JSON.stringify(summarizeSurfaceCutAdjacency(cut?.selectedPages ?? []))}`,
  );
}

async function run(context: ScenarioContext, arguments_: readonly string[]) {
  if (arguments_.length > 0) {
    throw new Error(`player-rendering takes no arguments; received ${arguments_.join(" ")}`);
  }
  const world = await startWorldStack(context, {
    fixture: {
      prefix: "voxels-player-rendering-",
      source: "terrain-diffusion-30m",
      diagnosticSkyRgb: [255, 0, 255],
      dayLengthSeconds: 0,
      dayFractionAtUnixEpoch: 0.72,
      weatherCycleSeconds: 0,
      weatherFractionAtUnixEpoch: 0.08,
    },
    service: { metal: true, profile: "worldgen-dev" },
    web: { buildProfile: "wasm-dev" },
  });
  const browser = await BrowserCapability.start(context);
  const viewport = await browser.open({
    url: world.url,
    label: "real-default-player",
    viewport: VIEWPORT,
    ...world.clientRoute,
  });
  const { engine, page } = viewport;
  let lastProgressLog = 0;
  const ready = await engine.waitForSnapshot(
    (snapshot) =>
      snapshotValue(snapshot, "terrainReady") === 1 &&
      snapshotValue(snapshot, "canonicalImmediateRequired") > 0 &&
      snapshotValue(snapshot, "canonicalImmediateResident") >=
        snapshotValue(snapshot, "canonicalImmediateRequired") &&
      snapshotValue(snapshot, "grounded") === 1 &&
      snapshotValue(snapshot, "pendingJobs") === 0 &&
      snapshotValue(snapshot, "frameSequence") > 0,
    {
      timeoutMs: 120_000,
      description: "default player never received a playable terrain presentation",
      onSnapshot: (snapshot) => {
        browser.assertHealthy();
        if (performance.now() - lastProgressLog < 10_000) return;
        lastProgressLog = performance.now();
        context.log(
          JSON.stringify({
            exactPages: snapshotValue(snapshot, "virtualTerrainPublishedExactPages"),
            minimumLevel: snapshotValue(snapshot, "virtualTerrainPublishedMinimumLevel"),
            maximumLevel: snapshotValue(snapshot, "virtualTerrainPublishedMaximumLevel"),
            residentPages: snapshotValue(snapshot, "virtualTerrainResidentPages"),
            selectedPages: snapshotValue(snapshot, "virtualTerrainSelectedPages"),
            requestedPages: snapshotValue(snapshot, "virtualTerrainRequestedPages"),
            pendingPages: snapshotValue(snapshot, "virtualTerrainStreamPending"),
            inFlightPages: snapshotValue(snapshot, "virtualTerrainStreamInFlight"),
          }),
        );
      },
    },
  );
  await engine.setCameraLook(snapshotValue(ready, "yaw"), -0.48);
  const pedestalStepMetres = await shortPlayerStep(page, () => engine.snapshot());
  const pedestal = await stablePhaseCapture(context, page, "default-pedestal", engine, () =>
    browser.assertHealthy(),
  );

  await engine.setCameraLook(0, -0.22);
  const distanceMetres = await walkBeyondProtectedPedestal(page, () => engine.snapshot(), 32);
  await engine.waitForSnapshot((snapshot) => snapshotValue(snapshot, "grounded") === 1, {
    timeoutMs: 15_000,
    description: "player did not land after leaving the spawn pedestal",
  });
  await engine.setCameraLook(0, -0.48);
  const travel = await stablePhaseCapture(context, page, "after-sprint", engine, () =>
    browser.assertHealthy(),
  );
  await engine.setCameraLook(0, -0.72);
  const targeted = await engine.waitForSnapshot(
    (snapshot) => snapshotValue(snapshot, "targetPresent") === 1,
    { timeoutMs: 15_000, description: "ordinary player could not target terrain to dig" },
  );
  const editsBefore = snapshotValue(targeted, "edits");

  // First click acquires pointer lock, second click is the same primary-button dig action a player
  // performs. The test intentionally does not call the automation edit shortcut.
  await page.mouse.click(VIEWPORT.width / 2, VIEWPORT.height / 2);
  await page.waitForFunction(() => document.pointerLockElement instanceof HTMLCanvasElement);
  await page.mouse.down();
  await page.waitForTimeout(120);
  await page.mouse.up();
  await engine.waitForSnapshot((snapshot) => snapshotValue(snapshot, "edits") > editsBefore, {
    timeoutMs: 20_000,
    description: "real primary-button dig did not become authoritative",
  });
  const edited = await stablePhaseCapture(context, page, "after-player-dig", engine, () =>
    browser.assertHealthy(),
  );
  if (edited.cutFingerprint === travel.cutFingerprint) {
    throw new Error("the authoritative player dig never changed the published terrain revision");
  }
  const reproductionCapture = await takePlayerScreenshot(page);
  await context.artifacts.write(
    "F2 gameplay capture with reproduction metadata",
    reproductionCapture.filename,
    reproductionCapture.png,
    "image/png",
  );
  browser.assertHealthy();

  return {
    summary:
      "Default spawn, a real player step, movement off the pedestal, and a real dig retained exact, stable, gap-free near terrain.",
    metrics: {
      walkedMetres: distanceMetres,
      pedestalStepMetres,
      pedestalExactPages: pedestal.exactPages,
      editedExactPages: edited.exactPages,
      travelExactPages: travel.exactPages,
      pedestalSelectedCut: pedestal.cutFingerprint,
      travelSelectedCut: travel.cutFingerprint,
      editedSelectedCut: edited.cutFingerprint,
      largestEnclosedSkyComponent: Math.max(
        ...[pedestal, travel, edited].map((entry) => entry.largestEnclosedSkyComponent),
      ),
      exactLodDiscontinuities: 0,
      reproductionScreenshotBytes: reproductionCapture.png.byteLength,
    },
  };
}

export default defineScenario({
  id: "player-rendering",
  kind: "validation",
  summary:
    "Validates the actual default player spawn, movement, dig, near-field LOD, seams, and stationary convergence.",
  uses: {
    world: true,
    browser: true,
    viewport: "browser",
    screenshots: true,
    rust: true,
  },
  timeoutMs: 600_000,
  run,
});
