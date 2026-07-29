import type { Page } from "playwright";
import { BrowserCapability } from "../lib/browser.ts";
import { type EngineClient, snapshotValue } from "../lib/engine.ts";
import { analyzeDiagnosticSky } from "../lib/image.ts";
import {
  PlayerPresentationRecorder,
  type PlayerPresentationViolation,
} from "../lib/player-presentation-recorder.ts";
import { summarizeSurfaceCutAdjacency, takePlayerScreenshot } from "../lib/player-screenshot.ts";
import { defineScenario, type ScenarioContext } from "../lib/scenario.ts";
import { startDevelopmentWorldStack } from "../lib/world.ts";

// Match the full-size browser viewport used for player reports. A small 960×540 harness can hide
// selection/capacity failures because projected-error refinement requests materially fewer pages.
const VIEWPORT = { width: 1324, height: 1118 };
const COLD_START_BUDGET_MS = 60_000;
const STABLE_CUT_DURATION_MS = 10_000;
const STABILITY_TIMEOUT_MS = 60_000;

interface AuditedCapture {
  readonly exactPages: number;
  readonly cutFingerprint: string;
  readonly largestEnclosedSkyComponent: number;
  readonly settleMs: number;
}

async function preserveContinuousPresentationFailure(
  context: ScenarioContext,
  page: Page,
  violation: PlayerPresentationViolation,
): Promise<void> {
  const stem = `continuous-${violation.phase}-frame-${violation.frameSequence}`;
  const errors: unknown[] = [];
  try {
    await context.artifacts.writeJson(
      "continuous player presentation failure trace",
      `${stem}.json`,
      violation,
    );
  } catch (error) {
    errors.push(error);
  }
  try {
    const png = await page.screenshot({ type: "png" });
    await context.artifacts.write(
      "continuous player presentation failure view",
      `${stem}.png`,
      png,
      "image/png",
    );
  } catch (error) {
    errors.push(error);
  }
  if (errors.length > 0) {
    throw new AggregateError(errors, "could not preserve all continuous presentation evidence");
  }
}

async function auditCapture(
  context: ScenarioContext,
  page: Page,
  engine: EngineClient,
  label: string,
  frame: readonly number[],
  settleMs: number,
): Promise<AuditedCapture> {
  const png = await page.screenshot({ type: "png" });
  await context.artifacts.write(label, `${label}.png`, png, "image/png");
  const sky = await analyzeDiagnosticSky(page, png);
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
  const handleSnapshot = diagnostic.metadata.presentation.terrainHandleSnapshot;
  if (
    !handleSnapshot.matchesPublishedCut ||
    handleSnapshot.generation === "0" ||
    handleSnapshot.cutFingerprint !== diagnostic.metadata.presentation.selectedCutFingerprint
  ) {
    throw new Error(
      `${label} capture did not embed one self-consistent committed terrain presentation: ` +
        `${JSON.stringify(handleSnapshot)} versus selected ` +
        `${diagnostic.metadata.presentation.selectedCutFingerprint}`,
    );
  }
  const nearby = diagnostic.ownership?.summarizeNearby(diagnostic.metadata.camera.eyeMetres, 12.8);
  if (nearby === undefined || nearby.nearbyOwnedPixels === 0) {
    throw new Error(`${label} produced no machine-readable nearby terrain ownership`);
  }
  if (nearby.nearbyCoarsePixels > 0) {
    throw new Error(
      `${label} exposed ${nearby.nearbyCoarsePixels}/${nearby.nearbyOwnedPixels} coarse terrain pixels inside the exact 12.8m player vicinity; ${JSON.stringify(nearby)}`,
    );
  }
  const exactPages = snapshotValue(frame, "virtualTerrainPublishedExactPages");
  if (
    snapshotValue(frame, "terrainReady") !== 1 ||
    snapshotValue(frame, "virtualTerrainGpuMatchesCpuCut") !== 1 ||
    snapshotValue(frame, "virtualTerrainGpuEncodingOverflowFlags") !== 0 ||
    snapshotValue(frame, "virtualTerrainPresentedSnapshotMatchesCut") !== 1 ||
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
      nearbyPixels: nearby,
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
    settleMs,
  };
}

async function shortPlayerStep(page: Page, recorder: PlayerPresentationRecorder): Promise<number> {
  const before = recorder.latestSnapshot;
  await page.keyboard.down("KeyW");
  try {
    await recorder.guard(page.waitForTimeout(180));
  } finally {
    await page.keyboard.up("KeyW");
  }
  const releasedAfter = snapshotValue(recorder.latestSnapshot, "frameSequence");
  await recorder.waitForFrameAfter(releasedAfter, {
    timeoutMs: 5_000,
    description: "renderer did not observe the end of the real W-key step",
  });
  const after = recorder.latestSnapshot;
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
  context: ScenarioContext,
  page: Page,
  recorder: PlayerPresentationRecorder,
  targetMetres: number,
): Promise<number> {
  const before = recorder.latestSnapshot;
  await page.keyboard.down("ShiftLeft");
  await page.keyboard.down("KeyW");
  let distance = 0;
  let nextPixelAudit = 0;
  try {
    const deadline = performance.now() + 20_000;
    let previousFrame = snapshotValue(recorder.latestSnapshot, "frameSequence");
    while (performance.now() < deadline) {
      const current = await recorder.waitForFrameAfter(previousFrame, {
        timeoutMs: Math.max(1, deadline - performance.now()),
        description: "renderer stopped advancing during real player sprint",
      });
      previousFrame = snapshotValue(current, "frameSequence");
      distance = Math.hypot(
        snapshotValue(current, "cameraX") - snapshotValue(before, "cameraX"),
        snapshotValue(current, "cameraZ") - snapshotValue(before, "cameraZ"),
      );
      if (performance.now() >= nextPixelAudit) {
        const png = await page.screenshot({ type: "png" });
        const [magenta, black] = await Promise.all([
          analyzeDiagnosticSky(page, png),
          analyzeDiagnosticSky(page, png, { x0: 0.05, x1: 0.95, y0: 0.08, y1: 0.58 }, "black"),
        ]);
        if (magenta.largestEnclosedComponentPixels > 0 || black.largestComponentPixels >= 16) {
          await context.artifacts.write(
            "transient terrain hole during sprint",
            "during-sprint-transient-hole.png",
            png,
            "image/png",
          );
          throw new Error(
            `terrain exposed a transient hole after ${distance.toFixed(2)}m of sprinting: ` +
              `${magenta.largestEnclosedComponentPixels} enclosed magenta pixels, ` +
              `${black.largestComponentPixels} contiguous black pixels`,
          );
        }
        nextPixelAudit = performance.now() + 250;
      }
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
  recorder: PlayerPresentationRecorder,
  assertHealthy: () => void,
): Promise<AuditedCapture> {
  const started = performance.now();
  let stableSince: number | undefined;
  let lastLog = 0;
  let previousFingerprint = "";
  let previousFlow = "";
  let previousFrame = snapshotValue(recorder.latestSnapshot, "frameSequence");
  while (performance.now() - started < STABILITY_TIMEOUT_MS) {
    const current = await recorder.waitForFrameAfter(previousFrame, {
      timeoutMs: Math.max(1, STABILITY_TIMEOUT_MS - (performance.now() - started)),
      description: `${phase} renderer stopped advancing before convergence`,
    });
    previousFrame = snapshotValue(current, "frameSequence");
    assertHealthy();
    const exactPages = snapshotValue(current, "virtualTerrainPublishedExactPages");
    const fingerprint = [
      snapshotValue(current, "virtualTerrainCutFingerprintHigh24"),
      snapshotValue(current, "virtualTerrainCutFingerprintLow24"),
      snapshotValue(current, "virtualTerrainPresentedSnapshotGenerationHigh24"),
      snapshotValue(current, "virtualTerrainPresentedSnapshotGenerationLow24"),
      snapshotValue(current, "virtualTerrainPresentedSnapshotFingerprintHigh24"),
      snapshotValue(current, "virtualTerrainPresentedSnapshotFingerprintLow24"),
    ].join(":");
    const state = {
      frameSequence: snapshotValue(current, "frameSequence"),
      terrainReady: snapshotValue(current, "terrainReady"),
      renderMode: snapshotValue(current, "virtualTerrainMode"),
      registeredRegions: snapshotValue(current, "virtualTerrainRegisteredRegions"),
      directoryInFlight: snapshotValue(current, "virtualTerrainDirectoryInFlight"),
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
      gpuOverflow: snapshotValue(current, "virtualTerrainGpuEncodingOverflowFlags"),
      presentedMatchesCut: snapshotValue(current, "virtualTerrainPresentedSnapshotMatchesCut"),
      columnSubmitDeferred: snapshotValue(current, "virtualTerrainColumnSubmitDeferred"),
      columnPreempted: snapshotValue(current, "virtualTerrainColumnPreempted"),
      columnTimedOut: snapshotValue(current, "virtualTerrainColumnTimedOut"),
      columnOtherFailed: snapshotValue(current, "virtualTerrainColumnOtherFailed"),
      directorySubmitDeferred: snapshotValue(current, "virtualTerrainDirectorySubmitDeferred"),
      directoryPreempted: snapshotValue(current, "virtualTerrainDirectoryPreempted"),
      directoryTimedOut: snapshotValue(current, "virtualTerrainDirectoryTimedOut"),
      directoryOtherFailed: snapshotValue(current, "virtualTerrainDirectoryOtherFailed"),
      pageSubmitDeferred: snapshotValue(current, "virtualTerrainPageSubmitDeferred"),
      pagePreempted: snapshotValue(current, "virtualTerrainPagePreempted"),
      pageTimedOut: snapshotValue(current, "virtualTerrainPageTimedOut"),
      pageOtherFailed: snapshotValue(current, "virtualTerrainPageOtherFailed"),
      pageUnavailable: snapshotValue(current, "virtualTerrainPageUnavailable"),
      pageStaleRevision: snapshotValue(current, "virtualTerrainPageStaleRevision"),
      pageGenerationFailed: snapshotValue(current, "virtualTerrainPageGenerationFailed"),
      pageUploadFailed: snapshotValue(current, "virtualTerrainPageUploadFailed"),
      arenaAllocatedMiB: snapshotValue(current, "arenaAllocatedMiB"),
      arenaCapacityMiB: snapshotValue(current, "arenaCapacityMiB"),
      editCanonicalRequired: snapshotValue(current, "editCanonicalRequired"),
      editCanonicalRenderable: snapshotValue(current, "editCanonicalRenderable"),
      editCanonicalOwned: snapshotValue(current, "editCanonicalOwned"),
    };
    const flow = [
      state.columnSubmitDeferred,
      state.columnPreempted,
      state.columnTimedOut,
      state.columnOtherFailed,
      state.directorySubmitDeferred,
      state.directoryPreempted,
      state.directoryTimedOut,
      state.directoryOtherFailed,
      state.pageSubmitDeferred,
      state.pagePreempted,
      state.pageTimedOut,
      state.pageOtherFailed,
      state.pageUnavailable,
      state.pageStaleRevision,
      state.pageGenerationFailed,
      state.pageUploadFailed,
    ].join(":");
    if (performance.now() - lastLog >= 5_000) {
      context.log(`${phase} convergence ${JSON.stringify(state)}`);
      lastLog = performance.now();
    }
    // Player readiness is a property of the immutable presented cut, not of speculative quality
    // work outside it. Requiring every directory/page queue to become globally empty made this
    // "real player" scenario wait behind far-detail prefetch that the actual loading gate does not
    // own. The fingerprint must still remain unchanged for the full stability window, and the
    // continuous recorder independently rejects any ownership/core regression on every frame.
    const presentable =
      state.terrainReady === 1 &&
      state.exactPages > 0 &&
      state.gpuMatchesCpu === 1 &&
      state.gpuOverflow === 0 &&
      state.presentedMatchesCut === 1 &&
      state.columnRevisionFloors === 0 &&
      state.currentColumnRegisteredRoots > 0 &&
      state.editCanonicalRenderable >= state.editCanonicalRequired &&
      state.editCanonicalOwned >= state.editCanonicalRequired;
    if (
      !presentable ||
      fingerprint !== previousFingerprint ||
      (previousFlow !== "" && flow !== previousFlow)
    ) {
      stableSince = undefined;
    } else {
      stableSince ??= performance.now();
    }
    previousFingerprint = fingerprint;
    previousFlow = flow;
    if (stableSince !== undefined && performance.now() - stableSince >= STABLE_CUT_DURATION_MS) {
      const settleMs = performance.now() - started;
      const capture = await recorder.guard(
        auditCapture(context, page, engine, phase, current, settleMs),
      );
      assertHealthy();
      return capture;
    }
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
    `${phase} did not reach a quiescent published cut within ${STABILITY_TIMEOUT_MS / 1_000} seconds; ` +
      `published adjacency ${JSON.stringify(summarizeSurfaceCutAdjacency(cut?.selectedPages ?? []))}`,
  );
}

async function run(context: ScenarioContext, arguments_: readonly string[]) {
  if (arguments_.length > 0) {
    throw new Error(`player-rendering takes no arguments; received ${arguments_.join(" ")}`);
  }
  const coldStartStarted = performance.now();
  const world = await startDevelopmentWorldStack(context, {
    fixture: {
      prefix: "voxels-player-rendering-",
      source: "terrain-diffusion-30m",
      diagnosticSkyRgb: [255, 0, 255],
      dayLengthSeconds: 0,
      dayFractionAtUnixEpoch: 0.72,
      weatherCycleSeconds: 0,
      weatherFractionAtUnixEpoch: 0.08,
    },
    serviceProfile: "worldgen-dev",
    browserProfile: "wasm-dev",
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
  const recorder = new PlayerPresentationRecorder(engine, {
    initialPhase: "startup",
    onFrame: (frame) => {
      browser.assertHealthy();
      if (frame.phase !== "startup" || performance.now() - lastProgressLog < 10_000) return;
      lastProgressLog = performance.now();
      context.log(
        `startup continuous frame ${JSON.stringify({
          frameSequence: frame.frameSequence,
          renderMode: frame.renderMode,
          terrainReady: frame.terrainReady,
          exactPages: frame.published.exactPages,
          minimumLevel: frame.published.minimumLevel,
          cut: frame.published.cutFingerprint,
          bankGeneration: frame.gpu.presentedBankGeneration,
          bankMatchesCut: frame.gpu.presentedBankMatchesCut,
          streaming: frame.streaming,
          committedEnvelope: frame.committedEnvelope.fingerprint,
          committedSafety: `${frame.committedEnvelope.safetyCoverage}/${frame.committedEnvelope.safetyLeaves}`,
          committedHorizon: `${frame.committedEnvelope.horizonCoverage}/${frame.committedEnvelope.horizonRoots}`,
          target: frame.presentationTarget,
          gate: frame.presentationGate,
        })}`,
      );
    },
    onViolation: (violation) => preserveContinuousPresentationFailure(context, page, violation),
  });
  try {
    let ready: readonly number[];
    try {
      await recorder.start();
      ready = await recorder.waitFor(
        (snapshot) =>
          snapshotValue(snapshot, "terrainReady") === 1 &&
          snapshotValue(snapshot, "canonicalImmediateRequired") > 0 &&
          snapshotValue(snapshot, "canonicalImmediateResident") >=
            snapshotValue(snapshot, "canonicalImmediateRequired") &&
          snapshotValue(snapshot, "grounded") === 1 &&
          snapshotValue(snapshot, "pendingJobs") === 0 &&
          snapshotValue(snapshot, "frameSequence") > 0,
        {
          timeoutMs: COLD_START_BUDGET_MS,
          description: "default player never received a playable terrain presentation",
        },
      );
    } catch (error) {
      try {
        await context.artifacts.writeJson(
          "cold-start failure engine snapshot",
          "cold-start-failure-snapshot.json",
          {
            error: error instanceof Error ? error.message : String(error),
            snapshot: recorder.trace().at(-1),
            continuousTrace: recorder.trace(),
          },
        );
      } catch (artifactError) {
        context.log(`could not preserve cold-start snapshot evidence: ${String(artifactError)}`);
      }
      try {
        const screenshot = await page.screenshot({ type: "png" });
        await context.artifacts.write(
          "cold-start failure player view",
          "cold-start-failure.png",
          screenshot,
          "image/png",
        );
      } catch (artifactError) {
        context.log(`could not preserve cold-start screenshot evidence: ${String(artifactError)}`);
      }
      throw error;
    }
    const coldStartMs = performance.now() - coldStartStarted;
    recorder.setPhase("pedestal-step");
    await recorder.guard(engine.setCameraLook(snapshotValue(ready, "yaw"), -0.48));
    const pedestalStepMetres = await shortPlayerStep(page, recorder);
    recorder.setPhase("pedestal-settle");
    const pedestal = await stablePhaseCapture(
      context,
      page,
      "default-pedestal",
      engine,
      recorder,
      () => browser.assertHealthy(),
    );

    recorder.setPhase("travel");
    await recorder.guard(engine.setCameraLook(0, -0.22));
    const distanceMetres = await walkBeyondProtectedPedestal(context, page, recorder, 32);
    await recorder.waitFor((snapshot) => snapshotValue(snapshot, "grounded") === 1, {
      timeoutMs: 15_000,
      description: "player did not land after leaving the spawn pedestal",
    });
    await recorder.guard(engine.setCameraLook(0, -0.48));
    recorder.setPhase("travel-settle");
    const travel = await stablePhaseCapture(context, page, "after-sprint", engine, recorder, () =>
      browser.assertHealthy(),
    );

    recorder.setPhase("dig");
    await recorder.guard(engine.setCameraLook(0, -0.72));
    const targeted = await recorder.waitFor(
      (snapshot) => snapshotValue(snapshot, "targetPresent") === 1,
      { timeoutMs: 15_000, description: "ordinary player could not target terrain to dig" },
    );
    const editsBefore = snapshotValue(targeted, "edits");

    // First click acquires pointer lock, second click is the same primary-button dig action a player
    // performs. The test intentionally does not call the automation edit shortcut.
    await recorder.guard(page.mouse.click(VIEWPORT.width / 2, VIEWPORT.height / 2));
    await recorder.guard(
      page.waitForFunction(() => document.pointerLockElement instanceof HTMLCanvasElement),
    );
    await page.mouse.down();
    try {
      await recorder.guard(page.waitForTimeout(120));
    } finally {
      await page.mouse.up();
    }
    await recorder.waitFor((snapshot) => snapshotValue(snapshot, "edits") > editsBefore, {
      timeoutMs: 20_000,
      description: "real primary-button dig did not become authoritative",
    });
    recorder.setPhase("dig-settle");
    const edited = await stablePhaseCapture(
      context,
      page,
      "after-player-dig",
      engine,
      recorder,
      () => browser.assertHealthy(),
    );
    if (edited.cutFingerprint === travel.cutFingerprint) {
      throw new Error("the authoritative player dig never changed the published terrain revision");
    }
    const reproductionCapture = await recorder.guard(takePlayerScreenshot(page));
    await context.artifacts.write(
      "F2 gameplay capture with reproduction metadata",
      reproductionCapture.filename,
      reproductionCapture.png,
      "image/png",
    );
    browser.assertHealthy();

    return {
      summary:
        "Default spawn, a real player step, movement off the pedestal, and a real dig retained exact, stable, gap-free near terrain on every observed renderer frame.",
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
        continuousRendererFrames: recorder.observedFrames,
        firstPlayableFrameSequence: recorder.firstPlayableFrameSequence ?? 0,
        reproductionScreenshotBytes: reproductionCapture.png.byteLength,
        coldStartMs,
        pedestalSettleMs: pedestal.settleMs,
        travelSettleMs: travel.settleMs,
        editSettleMs: edited.settleMs,
      },
    };
  } finally {
    await recorder.stop();
  }
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
