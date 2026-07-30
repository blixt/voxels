import type { Page } from "playwright";
import { BrowserCapability } from "../lib/browser.ts";
import { type EngineClient, snapshotValue } from "../lib/engine.ts";
import { analyzeDiagnosticSky, compareRenderedImages } from "../lib/image.ts";
import {
  PlayerPresentationRecorder,
  type PlayerPresentationViolation,
} from "../lib/player-presentation-recorder.ts";
import {
  type PlayerScreenshot,
  summarizeSurfaceCutAdjacency,
  takePlayerScreenshot,
} from "../lib/player-screenshot.ts";
import { defineScenario, type ScenarioContext } from "../lib/scenario.ts";
import { startDevelopmentWorldStack } from "../lib/world.ts";

// Match the full-size browser viewport used for player reports. A small 960×540 harness can hide
// selection/capacity failures because projected-error refinement requests materially fewer pages.
const VIEWPORT = { width: 1324, height: 1118 };
const COLD_START_BUDGET_MS = 60_000;
const STABLE_CUT_DURATION_MS = 10_000;
const STABILITY_TIMEOUT_MS = 60_000;
const MOVEMENT_PROGRESS_EPSILON_METRES = 0.005;
const MAX_PLAYER_NO_PROGRESS_MS = 350;
const MAX_PLAYER_EXACT_QUALITY_DEBT_MS = 1_500;
const MAX_SPECTATOR_NO_PROGRESS_MS = 250;
const SPECTATOR_TRAVEL_MS = 18_000;
const JOURNEY_SCREENSHOT_TIMEOUT_MS = 45_000;

interface AuditedCapture {
  readonly exactPages: number;
  readonly cutFingerprint: string;
  readonly terrainRequest: number | null;
  readonly terrainGeneration: number | null;
  readonly terrainRevisionDigest: string;
  readonly largestEnclosedSkyComponent: number;
  readonly terrainInteriorSkyPixels: number;
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
  settleMs: number,
  terrainInteriorY0 = 0.48,
  requireVisibleNearField = true,
  requireLowerTerrainCoverage = true,
): Promise<AuditedCapture> {
  const png = await page.screenshot({ type: "png" });
  await context.artifacts.write(label, `${label}.png`, png, "image/png");
  const [sky, terrainInteriorSky] = await Promise.all([
    analyzeDiagnosticSky(page, png),
    analyzeDiagnosticSky(page, png, {
      x0: 0.04,
      x1: 0.96,
      y0: terrainInteriorY0,
      y1: 0.98,
    }),
  ]);
  await engine.setGeometrySourceDebug(true);
  let diagnostic;
  try {
    diagnostic = await takePlayerScreenshot(page, {
      timeoutMs: JOURNEY_SCREENSHOT_TIMEOUT_MS,
    });
    if (!diagnostic.metadata.render.geometrySourceDebug) {
      throw new Error(`${label} source-ownership capture lost its requested renderer state`);
    }
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
  if (nearby === undefined || nearby.ownedPixels === 0) {
    throw new Error(`${label} produced no machine-readable visible terrain ownership`);
  }
  if (requireVisibleNearField && nearby.nearbyOwnedPixels === 0) {
    throw new Error(`${label} produced no machine-readable nearby terrain ownership`);
  }
  if (nearby.nearbyCoarsePixels > 0) {
    throw new Error(
      `${label} exposed ${nearby.nearbyCoarsePixels}/${nearby.nearbyOwnedPixels} coarse terrain pixels inside the exact 12.8m player vicinity; ${JSON.stringify(nearby)}`,
    );
  }
  const cut = diagnostic.metadata.presentation.selectedCut.cut;
  if (cut === null) {
    throw new Error(`${label} capture omitted its published virtual terrain cut`);
  }
  const exactPages = cut.selectedPages.filter(
    (page) => page.level === 0 && page.coord[1] === -2_147_483_648,
  ).length;
  const exactDomain = diagnostic.metadata.presentation.virtualTerrain.exactSurfaceDomain;
  if (
    diagnostic.metadata.presentation.virtualTerrain.mode !== "visible" ||
    !cut.renderable ||
    cut.ownerlessRoots.length !== 0 ||
    cut.feedbackOverflow ||
    cut.selectionOverflow ||
    cut.traversalOverflow ||
    exactPages === 0 ||
    !exactDomain.coreComplete ||
    exactDomain.coreRequiredLeaves === 0 ||
    exactDomain.coreCurrentCoverage !== exactDomain.coreRequiredLeaves
  ) {
    throw new Error(`${label} did not retain its exact playable terrain vicinity`);
  }
  if (cut.exactSurfaceLodDiscontinuities !== 0) {
    throw new Error(
      `${label} published a skipped hierarchy level at the exact-player transition frontier`,
    );
  }
  if (sky.largestEnclosedComponentPixels > 0) {
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
  if (requireLowerTerrainCoverage && terrainInteriorSky.diagnosticSkyPixels > 0) {
    throw new Error(
      `${label} exposed ${terrainInteriorSky.diagnosticSkyPixels} diagnostic-sky pixels below the conservative terrain silhouette at ${JSON.stringify(terrainInteriorSky.sampleCoordinates)}; boundary-connected cracks are terrain holes too`,
    );
  }
  return {
    exactPages,
    cutFingerprint: diagnostic.metadata.presentation.selectedCutFingerprint,
    terrainRequest: diagnostic.metadata.presentation.publishedClientView.terrainRequest,
    terrainGeneration: diagnostic.metadata.presentation.publishedClientView.terrainGeneration,
    terrainRevisionDigest: diagnostic.metadata.presentation.publishedClientView.revisionDigest,
    largestEnclosedSkyComponent: sky.largestEnclosedComponentPixels,
    terrainInteriorSkyPixels: terrainInteriorSky.diagnosticSkyPixels,
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
): Promise<{
  readonly distanceMetres: number;
  readonly longestNoProgressMs: number;
  readonly longestFrameWaitMs: number;
  readonly longestExactQualityDebtMs: number;
  readonly longestExactQualityDebtMetres: number;
}> {
  const before = recorder.latestSnapshot;
  const forward = [
    Math.sin(snapshotValue(before, "yaw")),
    -Math.cos(snapshotValue(before, "yaw")),
  ] as const;
  const right = [-forward[1], forward[0]] as const;
  const coverageGapBaseline = snapshotValue(before, "virtualTerrainPresentedCoverageGapFrames");
  await page.keyboard.down("ShiftLeft");
  await page.keyboard.down("KeyW");
  let distance = 0;
  let previous = before;
  let lastProgressAt = performance.now();
  let longestNoProgressMs = 0;
  let longestFrameWaitMs = 0;
  let exactQualityDebtStartedAt: number | undefined;
  let exactQualityDebtStartedMetres = 0;
  let longestExactQualityDebtMs = 0;
  let longestExactQualityDebtMetres = 0;
  try {
    const deadline = performance.now() + 20_000;
    let previousFrame = snapshotValue(recorder.latestSnapshot, "frameSequence");
    while (performance.now() < deadline) {
      const frameWaitStarted = performance.now();
      const current = await recorder.waitForFrameAfter(previousFrame, {
        timeoutMs: Math.max(1, deadline - performance.now()),
        description: "renderer stopped advancing during real player sprint",
      });
      longestFrameWaitMs = Math.max(longestFrameWaitMs, performance.now() - frameWaitStarted);
      previousFrame = snapshotValue(current, "frameSequence");
      const observedAt = performance.now();
      const stepX = snapshotValue(current, "cameraX") - snapshotValue(previous, "cameraX");
      const stepZ = snapshotValue(current, "cameraZ") - snapshotValue(previous, "cameraZ");
      const forwardStep = stepX * forward[0] + stepZ * forward[1];
      if (forwardStep >= MOVEMENT_PROGRESS_EPSILON_METRES) {
        lastProgressAt = observedAt;
      } else {
        longestNoProgressMs = Math.max(longestNoProgressMs, observedAt - lastProgressAt);
      }
      previous = current;
      const displacementX = snapshotValue(current, "cameraX") - snapshotValue(before, "cameraX");
      const displacementZ = snapshotValue(current, "cameraZ") - snapshotValue(before, "cameraZ");
      distance = displacementX * forward[0] + displacementZ * forward[1];
      if (
        snapshotValue(current, "virtualTerrainExactCoreComplete") === 1 &&
        snapshotValue(current, "virtualTerrainExactCoreRequiredLeaves") > 0 &&
        snapshotValue(current, "virtualTerrainExactCoreCurrentCoverage") ===
          snapshotValue(current, "virtualTerrainExactCoreRequiredLeaves")
      ) {
        if (exactQualityDebtStartedAt !== undefined) {
          longestExactQualityDebtMs = Math.max(
            longestExactQualityDebtMs,
            observedAt - exactQualityDebtStartedAt,
          );
          longestExactQualityDebtMetres = Math.max(
            longestExactQualityDebtMetres,
            distance - exactQualityDebtStartedMetres,
          );
          exactQualityDebtStartedAt = undefined;
        }
      } else {
        if (exactQualityDebtStartedAt === undefined) {
          exactQualityDebtStartedAt = observedAt;
          exactQualityDebtStartedMetres = distance;
        }
      }
      if (
        snapshotValue(current, "virtualTerrainPresentedCoverageGapFrames") > coverageGapBaseline
      ) {
        throw new Error(
          `renderer presented ${snapshotValue(current, "virtualTerrainPresentedCoverageGapFrames") - coverageGapBaseline} frame(s) after the camera outran its complete terrain horizon`,
        );
      }
      if (distance >= targetMetres) break;
    }
  } finally {
    await page.keyboard.up("KeyW");
    await page.keyboard.up("ShiftLeft");
  }
  // Full-resolution screenshot readback inside the held-key loop gives streaming extra wall-clock
  // headroom. Audit the resulting view only after the uninstrumented movement interval ends.
  const postSprintPng = await page.screenshot({ type: "png" });
  const [magenta, terrainInteriorSky, black] = await Promise.all([
    analyzeDiagnosticSky(page, postSprintPng),
    analyzeDiagnosticSky(page, postSprintPng, { x0: 0.04, x1: 0.96, y0: 0.58, y1: 0.98 }),
    analyzeDiagnosticSky(page, postSprintPng, { x0: 0.05, x1: 0.95, y0: 0.08, y1: 0.58 }, "black"),
  ]);
  if (
    magenta.largestEnclosedComponentPixels > 0 ||
    terrainInteriorSky.diagnosticSkyPixels > 0 ||
    black.largestComponentPixels >= 16
  ) {
    await context.artifacts.write(
      "terrain hole immediately after sprint",
      "post-sprint-terrain-hole.png",
      postSprintPng,
      "image/png",
    );
    throw new Error(
      `terrain exposed a hole after ${distance.toFixed(2)}m of uninstrumented sprinting: ` +
        `${magenta.largestEnclosedComponentPixels} enclosed magenta pixels, ` +
        `${terrainInteriorSky.diagnosticSkyPixels} below-silhouette magenta pixels, ` +
        `${black.largestComponentPixels} contiguous black pixels`,
    );
  }
  if (exactQualityDebtStartedAt !== undefined) {
    longestExactQualityDebtMs = Math.max(
      longestExactQualityDebtMs,
      performance.now() - exactQualityDebtStartedAt,
    );
    longestExactQualityDebtMetres = Math.max(
      longestExactQualityDebtMetres,
      distance - exactQualityDebtStartedMetres,
    );
  }
  if (distance < targetMetres) {
    const evidence = {
      requestedMetres: targetMetres,
      travelledMetres: distance,
      latest: recorder.latestFrame,
      trace: recorder.trace(),
    };
    await context.artifacts.writeJson(
      "incomplete real player travel trace",
      "incomplete-player-travel.json",
      evidence,
    );
    throw new Error(
      `player moved only ${distance.toFixed(2)}m of the requested ${targetMetres.toFixed(2)}m travel: ${JSON.stringify(recorder.latestFrame)}`,
    );
  }
  const finalDisplacementX = snapshotValue(previous, "cameraX") - snapshotValue(before, "cameraX");
  const finalDisplacementZ = snapshotValue(previous, "cameraZ") - snapshotValue(before, "cameraZ");
  const lateralDriftMetres = Math.abs(
    finalDisplacementX * right[0] + finalDisplacementZ * right[1],
  );
  if (lateralDriftMetres > 2) {
    throw new Error(
      `held forward sprint drifted ${lateralDriftMetres.toFixed(2)}m laterally while making ${distance.toFixed(2)}m forward progress`,
    );
  }
  if (longestNoProgressMs > MAX_PLAYER_NO_PROGRESS_MS) {
    throw new Error(
      `player stopped progressing for ${longestNoProgressMs.toFixed(1)}ms while sprint input remained held`,
    );
  }
  if (longestFrameWaitMs > MAX_PLAYER_NO_PROGRESS_MS) {
    throw new Error(
      `renderer produced no frame for ${longestFrameWaitMs.toFixed(1)}ms during held sprint input`,
    );
  }
  if (longestExactQualityDebtMs > MAX_PLAYER_EXACT_QUALITY_DEBT_MS) {
    await context.artifacts.writeJson(
      "camera-local exact terrain debt trace",
      "player-exact-quality-debt.json",
      {
        longestExactQualityDebtMs,
        longestExactQualityDebtMetres,
        travelledMetres: distance,
        latest: recorder.latestFrame,
        trace: recorder.trace(),
      },
    );
    const png = await page.screenshot({ type: "png" });
    await context.artifacts.write(
      "camera-local exact terrain debt view",
      "player-exact-quality-debt.png",
      png,
      "image/png",
    );
    throw new Error(
      `camera-local 10 cm terrain ownership fell behind for ${longestExactQualityDebtMs.toFixed(1)}ms / ${longestExactQualityDebtMetres.toFixed(2)}m during ordinary sprint`,
    );
  }
  return {
    distanceMetres: distance,
    longestNoProgressMs,
    longestFrameWaitMs,
    longestExactQualityDebtMs,
    longestExactQualityDebtMetres,
  };
}

async function jumpAndLand(page: Page, recorder: PlayerPresentationRecorder): Promise<number> {
  const groundY = snapshotValue(recorder.latestSnapshot, "cameraY");
  await page.keyboard.down("Space");
  try {
    await recorder.guard(page.waitForTimeout(120));
  } finally {
    await page.keyboard.up("Space");
  }
  const airborne = await recorder.waitFor(
    (snapshot) =>
      snapshotValue(snapshot, "grounded") === 0 &&
      snapshotValue(snapshot, "cameraY") > groundY + 0.1,
    { timeoutMs: 2_000, description: "real Space-key jump never left the ground" },
  );
  const ascentMetres = snapshotValue(airborne, "cameraY") - groundY;
  await recorder.waitFor((snapshot) => snapshotValue(snapshot, "grounded") === 1, {
    timeoutMs: 5_000,
    description: "ordinary player did not land after a real jump",
  });
  return ascentMetres;
}

async function sustainedSpectatorTravel(
  page: Page,
  recorder: PlayerPresentationRecorder,
  durationMs = SPECTATOR_TRAVEL_MS,
  minimumDistanceMetres = 100,
): Promise<{
  readonly distanceMetres: number;
  readonly longestNoProgressMs: number;
  readonly longestFrameWaitMs: number;
  readonly frames: number;
  readonly cutTransitions: number;
  readonly exactLocusTransitions: number;
  readonly committedExactEpochTransitions: number;
}> {
  const before = recorder.latestSnapshot;
  const yaw = snapshotValue(before, "yaw");
  const pitch = snapshotValue(before, "pitch");
  const cosPitch = Math.cos(pitch);
  const forward = [Math.sin(yaw) * cosPitch, Math.sin(pitch), -Math.cos(yaw) * cosPitch] as const;
  const coverageGapBaseline = snapshotValue(before, "virtualTerrainPresentedCoverageGapFrames");
  let previous = before;
  let previousFrame = snapshotValue(before, "frameSequence");
  let lastProgressAt = performance.now();
  let longestNoProgressMs = 0;
  let longestFrameWaitMs = 0;
  let frames = 0;
  let cutTransitions = 0;
  let exactLocusTransitions = 0;
  let cut = [
    snapshotValue(before, "virtualTerrainCutFingerprintHigh24"),
    snapshotValue(before, "virtualTerrainCutFingerprintLow24"),
  ].join(":");
  let exactLocus = [
    snapshotValue(before, "virtualTerrainCommittedLocusMinimumLeafX"),
    snapshotValue(before, "virtualTerrainCommittedLocusMinimumLeafZ"),
    snapshotValue(before, "virtualTerrainCommittedLocusMaximumLeafExclusiveX"),
    snapshotValue(before, "virtualTerrainCommittedLocusMaximumLeafExclusiveZ"),
  ].join(":");
  let committedExactEpoch = `${exactLocus}|${cut}`;
  let committedExactEpochTransitions = 0;
  const deadline = performance.now() + durationMs;
  await page.keyboard.down("KeyW");
  try {
    while (performance.now() < deadline) {
      const frameWaitStarted = performance.now();
      const current = await recorder.waitForFrameAfter(previousFrame, {
        timeoutMs: Math.max(1, deadline - performance.now() + 1_000),
        description: "renderer stopped advancing during sustained spectator flight",
      });
      longestFrameWaitMs = Math.max(longestFrameWaitMs, performance.now() - frameWaitStarted);
      previousFrame = snapshotValue(current, "frameSequence");
      frames += 1;
      const currentCut = [
        snapshotValue(current, "virtualTerrainCutFingerprintHigh24"),
        snapshotValue(current, "virtualTerrainCutFingerprintLow24"),
      ].join(":");
      if (currentCut !== cut) {
        cutTransitions += 1;
        cut = currentCut;
      }
      const currentExactLocus = [
        snapshotValue(current, "virtualTerrainCommittedLocusMinimumLeafX"),
        snapshotValue(current, "virtualTerrainCommittedLocusMinimumLeafZ"),
        snapshotValue(current, "virtualTerrainCommittedLocusMaximumLeafExclusiveX"),
        snapshotValue(current, "virtualTerrainCommittedLocusMaximumLeafExclusiveZ"),
      ].join(":");
      if (currentExactLocus !== exactLocus) {
        exactLocusTransitions += 1;
        exactLocus = currentExactLocus;
      }
      const currentCommittedExactEpoch = `${currentExactLocus}|${currentCut}`;
      if (currentCommittedExactEpoch !== committedExactEpoch) {
        committedExactEpochTransitions += 1;
        committedExactEpoch = currentCommittedExactEpoch;
      }
      const observedAt = performance.now();
      const forwardStep =
        (snapshotValue(current, "cameraX") - snapshotValue(previous, "cameraX")) * forward[0] +
        (snapshotValue(current, "cameraY") - snapshotValue(previous, "cameraY")) * forward[1] +
        (snapshotValue(current, "cameraZ") - snapshotValue(previous, "cameraZ")) * forward[2];
      if (forwardStep >= MOVEMENT_PROGRESS_EPSILON_METRES) {
        lastProgressAt = observedAt;
      } else {
        longestNoProgressMs = Math.max(longestNoProgressMs, observedAt - lastProgressAt);
      }
      previous = current;
      if (
        snapshotValue(current, "virtualTerrainPresentedCoverageGapFrames") > coverageGapBaseline
      ) {
        throw new Error(
          `spectator outran the complete committed terrain horizon for ${snapshotValue(current, "virtualTerrainPresentedCoverageGapFrames") - coverageGapBaseline} frame(s)`,
        );
      }
    }
  } finally {
    await page.keyboard.up("KeyW");
  }
  const displacement = [
    snapshotValue(previous, "cameraX") - snapshotValue(before, "cameraX"),
    snapshotValue(previous, "cameraY") - snapshotValue(before, "cameraY"),
    snapshotValue(previous, "cameraZ") - snapshotValue(before, "cameraZ"),
  ] as const;
  const distanceMetres =
    displacement[0] * forward[0] + displacement[1] * forward[1] + displacement[2] * forward[2];
  const totalDisplacement = Math.hypot(...displacement);
  const lateralDriftMetres = Math.sqrt(
    Math.max(0, totalDisplacement * totalDisplacement - distanceMetres * distanceMetres),
  );
  if (distanceMetres < minimumDistanceMetres) {
    throw new Error(
      `spectator covered only ${distanceMetres.toFixed(2)}m during ${durationMs / 1_000}s of held forward input`,
    );
  }
  if (longestNoProgressMs > MAX_SPECTATOR_NO_PROGRESS_MS) {
    throw new Error(
      `spectator stopped progressing for ${longestNoProgressMs.toFixed(1)}ms while forward input remained held`,
    );
  }
  if (longestFrameWaitMs > MAX_SPECTATOR_NO_PROGRESS_MS) {
    throw new Error(
      `renderer produced no frame for ${longestFrameWaitMs.toFixed(1)}ms during held spectator input`,
    );
  }
  if (lateralDriftMetres > 2) {
    throw new Error(
      `held spectator-forward input drifted ${lateralDriftMetres.toFixed(2)}m away from its commanded ray`,
    );
  }
  if (exactLocusTransitions === 0 || cutTransitions === 0 || committedExactEpochTransitions === 0) {
    throw new Error(
      `spectator movement did not exercise a new committed exact terrain epoch: ` +
        `${exactLocusTransitions} committed locus transitions, ${cutTransitions} cut transitions, ` +
        `${committedExactEpochTransitions} paired epoch transitions`,
    );
  }
  return {
    distanceMetres,
    longestNoProgressMs,
    longestFrameWaitMs,
    frames,
    cutTransitions,
    exactLocusTransitions,
    committedExactEpochTransitions,
  };
}

async function waitForInventoryRevision(
  engine: EngineClient,
  recorder: PlayerPresentationRecorder,
  previousRevision: number,
  description: string,
  timeoutMs = 20_000,
): Promise<readonly number[]> {
  const deadline = performance.now() + timeoutMs;
  let sequence = snapshotValue(recorder.latestSnapshot, "frameSequence");
  while (performance.now() < deadline) {
    const inventory = await recorder.guard(engine.inventory());
    if ((inventory[0] ?? 0) > previousRevision) return inventory;
    const next = await recorder.waitForFrameAfter(sequence, {
      timeoutMs: Math.max(1, deadline - performance.now()),
      description,
    });
    sequence = snapshotValue(next, "frameSequence");
  }
  throw new Error(description);
}

function mostStockedMaterial(inventory: readonly number[]): {
  readonly materialId: number;
  readonly count: number;
} {
  let materialId = 0;
  let count = 0;
  for (let candidate = 1; candidate + 1 < inventory.length; candidate += 1) {
    const candidateCount = inventory[candidate + 1] ?? 0;
    if (candidateCount > count) {
      materialId = candidate;
      count = candidateCount;
    }
  }
  return { materialId, count };
}

async function verifyCaptureReplayAgainstCurrentWorld(
  context: ScenarioContext,
  page: Page,
  engine: EngineClient,
  recorder: PlayerPresentationRecorder,
  capture: PlayerScreenshot,
): Promise<number> {
  const metadataText = JSON.stringify(capture.metadata);
  const expectedFingerprint = BigInt(`0x${capture.metadata.presentation.selectedCutFingerprint}`);
  const expectedLow24 = Number(expectedFingerprint & 0xff_ff_ffn);
  const expectedHigh24 = Number((expectedFingerprint >> 24n) & 0xff_ff_ffn);
  const before = recorder.latestSnapshot;
  let applied = false;
  try {
    await recorder.guard(
      engine.applyReproduction(metadataText, {
        timeoutMs: 30_000,
        description: "current authoritative world did not accept its own screenshot metadata",
      }),
    );
    applied = true;
    const committed = await recorder.waitFor(
      (snapshot) =>
        snapshotValue(snapshot, "clientViewGoalKind") === 0 &&
        snapshotValue(snapshot, "reproductionActive") === 1 &&
        snapshotValue(snapshot, "terrainReady") === 1 &&
        snapshotValue(snapshot, "virtualTerrainPresentedSnapshotMatchesCut") === 1 &&
        snapshotValue(snapshot, "virtualTerrainCutFingerprintLow24") === expectedLow24 &&
        snapshotValue(snapshot, "virtualTerrainCutFingerprintHigh24") === expectedHigh24,
      {
        timeoutMs: 30_000,
        description:
          "screenshot metadata did not restore its exact cut in the same authoritative world",
      },
    );
    await recorder.waitForFrameAfter(snapshotValue(committed, "frameSequence"), {
      timeoutMs: 5_000,
      description: "renderer did not present a frame from the restored screenshot state",
    });
    const replayed = await recorder.guard(
      takePlayerScreenshot(page, { timeoutMs: JOURNEY_SCREENSHOT_TIMEOUT_MS }),
    );
    if (
      replayed.metadata.presentation.selectedCutFingerprint !==
        capture.metadata.presentation.selectedCutFingerprint ||
      JSON.stringify(replayed.metadata.presentation.selectedCut.cut?.selectedPages) !==
        JSON.stringify(capture.metadata.presentation.selectedCut.cut?.selectedPages)
    ) {
      throw new Error(
        "same-world screenshot replay restored a different cut or selected-page identity",
      );
    }
    const imageComparison = await compareRenderedImages(page, capture.png, replayed.png, {
      region: { x0: 0.03, x1: 0.97, y0: 0.06, y1: 0.9 },
      footprintPixels: 4,
      diagnosticGeometry: true,
    });
    const originalOwnership = capture.ownership;
    const replayedOwnership = replayed.ownership;
    const ownershipAttachmentMatches =
      originalOwnership !== null &&
      replayedOwnership !== null &&
      originalOwnership.width === replayedOwnership.width &&
      originalOwnership.height === replayedOwnership.height &&
      Buffer.from(originalOwnership.pixels).equals(Buffer.from(replayedOwnership.pixels));
    await context.artifacts.writeJson(
      "same-world screenshot replay comparison",
      "same-world-replay-comparison.json",
      { ...imageComparison, ownershipAttachmentMatches },
    );
    await context.artifacts.write(
      "same-world screenshot metadata replay",
      "same-world-replayed.png",
      replayed.png,
      "image/png",
    );
    const geometry = imageComparison.diagnosticGeometry;
    if (
      geometry === null ||
      !ownershipAttachmentMatches ||
      geometry.largestDisagreementComponentPixels > 16 ||
      geometry.occupancyJaccard < 0.9999 ||
      imageComparison.ssim < 0.99 ||
      imageComparison.meanAbsoluteLinearRgbDelta > 0.005 ||
      imageComparison.meanAbsoluteLinearLumaDelta > 0.005 ||
      imageComparison.relativeMeanLinearLumaDelta > 0.02
    ) {
      throw new Error(
        `same-world screenshot replay changed exact terrain ownership, material, depth, geometry, or appearance: ${JSON.stringify({ ...imageComparison, ownershipAttachmentMatches })}`,
      );
    }
    return replayed.png.byteLength;
  } finally {
    if (applied) {
      const sequence = snapshotValue(recorder.latestSnapshot, "frameSequence");
      await recorder.guard(engine.clearReproduction());
      await recorder.waitFor(
        (snapshot) =>
          snapshotValue(snapshot, "frameSequence") > sequence &&
          snapshotValue(snapshot, "clientViewGoalKind") === 0 &&
          snapshotValue(snapshot, "reproductionActive") === 0 &&
          snapshotValue(snapshot, "terrainReady") === 1 &&
          Math.hypot(
            snapshotValue(snapshot, "cameraX") - snapshotValue(before, "cameraX"),
            snapshotValue(snapshot, "cameraY") - snapshotValue(before, "cameraY"),
            snapshotValue(snapshot, "cameraZ") - snapshotValue(before, "cameraZ"),
          ) <= 0.001,
        {
          timeoutMs: 30_000,
          description: "clearing screenshot replay did not restore the live player view",
        },
      );
    }
  }
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
  let previousFrame = snapshotValue(recorder.latestSnapshot, "frameSequence");
  const failureBaseline = {
    columnOtherFailed: snapshotValue(recorder.latestSnapshot, "virtualTerrainColumnOtherFailed"),
    directoryOtherFailed: snapshotValue(
      recorder.latestSnapshot,
      "virtualTerrainDirectoryOtherFailed",
    ),
    pageOtherFailed: snapshotValue(recorder.latestSnapshot, "virtualTerrainPageOtherFailed"),
    pageUnavailable: snapshotValue(recorder.latestSnapshot, "virtualTerrainPageUnavailable"),
    pageGenerationFailed: snapshotValue(
      recorder.latestSnapshot,
      "virtualTerrainPageGenerationFailed",
    ),
    pageUploadFailed: snapshotValue(recorder.latestSnapshot, "virtualTerrainPageUploadFailed"),
  };
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
      planLastSelection: snapshotValue(current, "virtualTerrainPlanLastSelection"),
      planLastInvalidation: snapshotValue(current, "virtualTerrainPlanLastInvalidation"),
      planLastInvalidationLine: snapshotValue(current, "virtualTerrainPlanLastInvalidationLine"),
      publicationLastAbortLine: snapshotValue(current, "virtualTerrainPublicationLastAbortLine"),
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
      state.editCanonicalOwned >= state.editCanonicalRequired &&
      state.columnOtherFailed === failureBaseline.columnOtherFailed &&
      state.directoryOtherFailed === failureBaseline.directoryOtherFailed &&
      state.pageOtherFailed === failureBaseline.pageOtherFailed &&
      state.pageUnavailable === failureBaseline.pageUnavailable &&
      state.pageGenerationFailed === failureBaseline.pageGenerationFailed &&
      state.pageUploadFailed === failureBaseline.pageUploadFailed;
    if (!presentable || fingerprint !== previousFingerprint) {
      stableSince = undefined;
    } else {
      stableSince ??= performance.now();
    }
    previousFingerprint = fingerprint;
    if (stableSince !== undefined && performance.now() - stableSince >= STABLE_CUT_DURATION_MS) {
      const settleMs = performance.now() - started;
      const capture = await recorder.guard(auditCapture(context, page, engine, phase, settleMs));
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
  const automationContract = await engine.ready();
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

    // Exercise the reported failure in its coldest real state: enter spectator immediately after
    // the first playable frame, before the ten-second stability window, ordinary movement, edits,
    // screenshots, or a long route can warm the destination page cache.
    recorder.setPhase("fresh-spectator-travel");
    const bodyBeforeFreshSpectator = await recorder.guard(engine.setSpectator(true));
    // Fly perpendicular to the later yaw-zero walking route so this probe cannot warm its pages.
    await recorder.guard(engine.setCameraLook(Math.PI / 2, 0));
    const freshSpectatorMotion = await sustainedSpectatorTravel(page, recorder, 5_000, 40);
    const freshSpectatorEndpoint = await auditCapture(
      context,
      page,
      engine,
      "fresh-spectator-endpoint",
      0,
      0.48,
      false,
    );
    recorder.setPhase("fresh-spectator-restore");
    const freshRestoredBody = await recorder.guard(engine.setSpectator(false));
    const freshSpectatorBodyRestoreErrorMetres = Math.hypot(
      snapshotValue(freshRestoredBody, "cameraX") -
        snapshotValue(bodyBeforeFreshSpectator, "cameraX"),
      snapshotValue(freshRestoredBody, "cameraY") -
        snapshotValue(bodyBeforeFreshSpectator, "cameraY"),
      snapshotValue(freshRestoredBody, "cameraZ") -
        snapshotValue(bodyBeforeFreshSpectator, "cameraZ"),
    );
    if (freshSpectatorBodyRestoreErrorMetres > 0.001) {
      throw new Error(
        `fresh spectator flight restored the player body ${freshSpectatorBodyRestoreErrorMetres.toFixed(4)}m from its saved position`,
      );
    }
    await recorder.waitFor(
      (snapshot) =>
        snapshotValue(snapshot, "terrainReady") === 1 &&
        snapshotValue(snapshot, "canonicalImmediateRequired") > 0 &&
        snapshotValue(snapshot, "canonicalImmediateResident") >=
          snapshotValue(snapshot, "canonicalImmediateRequired") &&
        snapshotValue(snapshot, "grounded") === 1,
      {
        timeoutMs: 30_000,
        description:
          "gameplay body did not recover exact collision terrain after immediate spectator flight",
      },
    );

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
    const travelMotion = await walkBeyondProtectedPedestal(context, page, recorder, 32);
    await recorder.waitFor((snapshot) => snapshotValue(snapshot, "grounded") === 1, {
      timeoutMs: 15_000,
      description: "player did not land after leaving the spawn pedestal",
    });
    await recorder.guard(engine.setCameraLook(0, -0.48));
    recorder.setPhase("jump");
    const jumpAscentMetres = await jumpAndLand(page, recorder);

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
    let inventory = await recorder.guard(engine.inventory());
    const inventoryRevisionBeforeDig = inventory[0] ?? 0;

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
    inventory = await waitForInventoryRevision(
      engine,
      recorder,
      inventoryRevisionBeforeDig,
      "real primary-button dig did not become authoritative",
    );
    const requiredPlacementStock = automationContract.semantics.editSphereVolumeVoxels;
    let digPasses = 1;
    let previousTarget: readonly [number, number, number] = [
      snapshotValue(targeted, "targetVoxelX"),
      snapshotValue(targeted, "targetVoxelY"),
      snapshotValue(targeted, "targetVoxelZ"),
    ];
    while (mostStockedMaterial(inventory).count < requiredPlacementStock && digPasses < 6) {
      const afterEditSequence = snapshotValue(recorder.latestSnapshot, "frameSequence");
      const nextTarget = await recorder.waitFor(
        (snapshot) =>
          snapshotValue(snapshot, "frameSequence") > afterEditSequence &&
          snapshotValue(snapshot, "targetPresent") === 1 &&
          snapshotValue(snapshot, "pendingJobs") === 0 &&
          Math.hypot(
            snapshotValue(snapshot, "targetVoxelX") - previousTarget[0],
            snapshotValue(snapshot, "targetVoxelY") - previousTarget[1],
            snapshotValue(snapshot, "targetVoxelZ") - previousTarget[2],
          ) >= 4,
        {
          timeoutMs: 15_000,
          description: "ordinary player ray did not advance through the dug tunnel",
        },
      );
      previousTarget = [
        snapshotValue(nextTarget, "targetVoxelX"),
        snapshotValue(nextTarget, "targetVoxelY"),
        snapshotValue(nextTarget, "targetVoxelZ"),
      ];
      const inventoryRevisionBeforePass = inventory[0] ?? 0;
      await recorder.guard(page.mouse.click(VIEWPORT.width / 2, VIEWPORT.height / 2));
      inventory = await waitForInventoryRevision(
        engine,
        recorder,
        inventoryRevisionBeforePass,
        "ordinary follow-up dig did not become authoritative",
      );
      digPasses += 1;
    }
    const { materialId: placementMaterial, count: placementStock } = mostStockedMaterial(inventory);
    if (placementStock < requiredPlacementStock) {
      throw new Error(
        `${digPasses} collinear real digs yielded only ${placementStock}/${requiredPlacementStock} voxels of the most abundant material`,
      );
    }
    recorder.setPhase("dig-settle");
    const edited = await stablePhaseCapture(
      context,
      page,
      "after-player-dig",
      engine,
      recorder,
      () => browser.assertHealthy(),
    );
    if (
      edited.terrainRequest === travel.terrainRequest ||
      edited.terrainGeneration === travel.terrainGeneration ||
      edited.terrainRevisionDigest === travel.terrainRevisionDigest
    ) {
      throw new Error(
        "the authoritative player dig never promoted a newly requested, newly encoded terrain revision",
      );
    }
    recorder.setPhase("place");
    for (let step = 0; step < inventory.length - 2; step += 1) {
      const selected = snapshotValue(recorder.latestSnapshot, "placementMaterial");
      if (selected === placementMaterial) break;
      const previousSequence = snapshotValue(recorder.latestSnapshot, "frameSequence");
      await recorder.guard(page.mouse.wheel(0, 120));
      await recorder.waitFor(
        (snapshot) =>
          snapshotValue(snapshot, "frameSequence") > previousSequence &&
          snapshotValue(snapshot, "placementMaterial") !== selected,
        {
          timeoutMs: 2_000,
          description: "real inventory wheel input did not advance material selection",
        },
      );
    }
    if (snapshotValue(recorder.latestSnapshot, "placementMaterial") !== placementMaterial) {
      throw new Error(`real inventory input could not select material ${placementMaterial}`);
    }
    // Placement is a player action, not an automation edit shortcut. Search a small set of real
    // view directions because the metre-scale sphere can legitimately intersect the player or
    // occupied terrain on one slope. Every attempt still goes through targeting, right-button
    // input, server validation, and authoritative inventory debit.
    const placementYaws = [
      Math.PI / 2,
      -Math.PI / 2,
      Math.PI,
      Math.PI / 4,
      -Math.PI / 4,
      (3 * Math.PI) / 4,
      (-3 * Math.PI) / 4,
      0,
    ];
    let inventoryBeforePlace: readonly number[] | undefined;
    let inventoryAfterPlace: readonly number[] | undefined;
    for (const yaw of placementYaws) {
      await recorder.guard(engine.setCameraLook(yaw, -0.55));
      await recorder.waitFor(
        (snapshot) =>
          snapshotValue(snapshot, "targetPresent") === 1 &&
          snapshotValue(snapshot, "canonicalImmediateRequired") > 0 &&
          snapshotValue(snapshot, "canonicalImmediateResident") >=
            snapshotValue(snapshot, "canonicalImmediateRequired") &&
          (snapshotValue(snapshot, "targetVoxelX") !== snapshotValue(targeted, "targetVoxelX") ||
            snapshotValue(snapshot, "targetVoxelZ") !== snapshotValue(targeted, "targetVoxelZ")),
        { timeoutMs: 5_000, description: "ordinary player could not target terrain to place" },
      );
      const candidateInventory = await recorder.guard(engine.inventory());
      const candidateRevision = candidateInventory[0] ?? 0;
      const attemptDescription = `real secondary-button placement at yaw ${yaw.toFixed(3)} did not become authoritative`;
      await recorder.guard(page.mouse.down({ button: "right" }));
      try {
        await recorder.guard(page.waitForTimeout(100));
      } finally {
        await recorder.guard(page.mouse.up({ button: "right" }));
      }
      try {
        inventoryAfterPlace = await waitForInventoryRevision(
          engine,
          recorder,
          candidateRevision,
          attemptDescription,
          3_000,
        );
        inventoryBeforePlace = candidateInventory;
        break;
      } catch (error) {
        if (!(error instanceof Error) || !error.message.startsWith(attemptDescription)) {
          throw error;
        }
      }
    }
    if (inventoryBeforePlace === undefined || inventoryAfterPlace === undefined) {
      throw new Error(
        `real secondary-button placement was rejected in every unobstructed view direction: ${JSON.stringify(recorder.latestFrame)}`,
      );
    }
    const placementCountBefore = inventoryBeforePlace[placementMaterial + 1] ?? 0;
    const placementCountAfter = inventoryAfterPlace[placementMaterial + 1] ?? 0;
    if (placementCountBefore - placementCountAfter !== requiredPlacementStock) {
      throw new Error(
        `real placement debited ${placementCountBefore - placementCountAfter} voxels instead of ${requiredPlacementStock}`,
      );
    }
    recorder.setPhase("place-settle");
    const placed = await stablePhaseCapture(
      context,
      page,
      "after-player-place",
      engine,
      recorder,
      () => browser.assertHealthy(),
    );
    if (
      placed.terrainRequest === edited.terrainRequest ||
      placed.terrainGeneration === edited.terrainGeneration ||
      placed.terrainRevisionDigest === edited.terrainRevisionDigest
    ) {
      throw new Error(
        "the authoritative player placement never promoted a newly requested, newly encoded terrain revision",
      );
    }
    const reproductionCapture = await recorder.guard(
      takePlayerScreenshot(page, { timeoutMs: JOURNEY_SCREENSHOT_TIMEOUT_MS }),
    );
    await context.artifacts.write(
      "F2 gameplay capture with reproduction metadata",
      reproductionCapture.filename,
      reproductionCapture.png,
      "image/png",
    );
    const reproductionReplayScreenshotBytes = await verifyCaptureReplayAgainstCurrentWorld(
      context,
      page,
      engine,
      recorder,
      reproductionCapture,
    );
    browser.assertHealthy();

    // Run the long spectator excursion after all gameplay convergence checks. Restoring a saved
    // body should not make the test wait for speculative far-flight pages to drain before it can
    // validate dig/place correctness at that body.
    recorder.setPhase("spectator-ascent");
    const bodyBeforeSpectator = await recorder.guard(engine.setSpectator(true));
    const spectatorStartY = snapshotValue(recorder.latestSnapshot, "cameraY");
    await page.keyboard.down("Space");
    try {
      // Gain enough altitude that the long collisionless route exercises terrain streaming above
      // the landscape instead of ending inside a mountain where a lower-screen silhouette has no
      // useful meaning.
      await recorder.guard(page.waitForTimeout(2_000));
    } finally {
      await page.keyboard.up("Space");
    }
    const spectatorAscended = await recorder.waitFor(
      (snapshot) => snapshotValue(snapshot, "cameraY") > spectatorStartY + 0.5,
      { timeoutMs: 2_000, description: "spectator could not ascend with held Space input" },
    );
    const spectatorAscentMetres = snapshotValue(spectatorAscended, "cameraY") - spectatorStartY;
    recorder.setPhase("spectator-travel");
    await recorder.guard(engine.setCameraLook(0, 0));
    const spectatorMotion = await sustainedSpectatorTravel(page, recorder);
    const spectatorReady = await recorder.waitFor(
      (snapshot) =>
        snapshotValue(snapshot, "terrainReady") === 1 &&
        snapshotValue(snapshot, "virtualTerrainGpuMatchesCpuCut") === 1 &&
        snapshotValue(snapshot, "virtualTerrainPresentedSnapshotMatchesCut") === 1 &&
        snapshotValue(snapshot, "virtualTerrainExactCoreComplete") === 1 &&
        snapshotValue(snapshot, "virtualTerrainExactCoreCurrentCoverage") ===
          snapshotValue(snapshot, "virtualTerrainExactCoreRequiredLeaves"),
      {
        timeoutMs: 30_000,
        description: "spectator endpoint never acquired an exact camera-local terrain presentation",
      },
    );
    await recorder.guard(engine.setCameraLook(0, -0.62));
    await recorder.waitForFrameAfter(snapshotValue(spectatorReady, "frameSequence"), {
      timeoutMs: 5_000,
      description: "spectator endpoint did not present its downward terrain audit view",
    });
    const spectator = await auditCapture(
      context,
      page,
      engine,
      "after-spectator-flight",
      0,
      0.55,
      false,
      true,
    );
    recorder.setPhase("spectator-restore");
    const restoredBody = await recorder.guard(engine.setSpectator(false));
    const bodyRestoreErrorMetres = Math.hypot(
      snapshotValue(restoredBody, "cameraX") - snapshotValue(bodyBeforeSpectator, "cameraX"),
      snapshotValue(restoredBody, "cameraY") - snapshotValue(bodyBeforeSpectator, "cameraY"),
      snapshotValue(restoredBody, "cameraZ") - snapshotValue(bodyBeforeSpectator, "cameraZ"),
    );
    if (bodyRestoreErrorMetres > 0.001) {
      throw new Error(
        `leaving spectator restored the player body ${bodyRestoreErrorMetres.toFixed(4)}m from its saved position`,
      );
    }
    await recorder.guard(engine.setCameraLook(Math.PI, -0.48));
    await recorder.waitFor(
      (snapshot) =>
        snapshotValue(snapshot, "terrainReady") === 1 &&
        snapshotValue(snapshot, "canonicalImmediateRequired") > 0 &&
        snapshotValue(snapshot, "canonicalImmediateResident") >=
          snapshotValue(snapshot, "canonicalImmediateRequired") &&
        snapshotValue(snapshot, "grounded") === 1 &&
        snapshotValue(snapshot, "targetPresent") === 1,
      {
        timeoutMs: 30_000,
        description:
          "restored gameplay body did not regain exact terrain, collision, grounding, and targeting",
      },
    );
    const postSpectatorStepMetres = await shortPlayerStep(page, recorder);
    const postSpectatorJumpAscentMetres = await jumpAndLand(page, recorder);
    browser.assertHealthy();

    return {
      summary:
        "Default spawn, immediate and sustained spectator flight, walking, jumping, dig, place, and capture retained continuous movement and exact gap-free near terrain.",
      metrics: {
        freshSpectatorTravelMetres: freshSpectatorMotion.distanceMetres,
        freshSpectatorLongestNoProgressMs: freshSpectatorMotion.longestNoProgressMs,
        freshSpectatorLongestFrameWaitMs: freshSpectatorMotion.longestFrameWaitMs,
        freshSpectatorTravelFrames: freshSpectatorMotion.frames,
        freshSpectatorCutTransitions: freshSpectatorMotion.cutTransitions,
        freshSpectatorExactLocusTransitions: freshSpectatorMotion.exactLocusTransitions,
        freshSpectatorCommittedExactEpochTransitions:
          freshSpectatorMotion.committedExactEpochTransitions,
        freshSpectatorEndpointExactPages: freshSpectatorEndpoint.exactPages,
        freshSpectatorEndpointCut: freshSpectatorEndpoint.cutFingerprint,
        freshSpectatorBodyRestoreErrorMetres,
        walkedMetres: travelMotion.distanceMetres,
        playerLongestNoProgressMs: travelMotion.longestNoProgressMs,
        playerLongestFrameWaitMs: travelMotion.longestFrameWaitMs,
        playerLongestExactQualityDebtMs: travelMotion.longestExactQualityDebtMs,
        playerLongestExactQualityDebtMetres: travelMotion.longestExactQualityDebtMetres,
        jumpAscentMetres,
        digPasses,
        spectatorAscentMetres,
        spectatorTravelMetres: spectatorMotion.distanceMetres,
        spectatorLongestNoProgressMs: spectatorMotion.longestNoProgressMs,
        spectatorLongestFrameWaitMs: spectatorMotion.longestFrameWaitMs,
        spectatorTravelFrames: spectatorMotion.frames,
        spectatorExactPages: spectator.exactPages,
        spectatorBodyRestoreErrorMetres: bodyRestoreErrorMetres,
        postSpectatorStepMetres,
        postSpectatorJumpAscentMetres,
        pedestalStepMetres,
        pedestalExactPages: pedestal.exactPages,
        editedExactPages: edited.exactPages,
        placedExactPages: placed.exactPages,
        travelExactPages: travel.exactPages,
        pedestalSelectedCut: pedestal.cutFingerprint,
        travelSelectedCut: travel.cutFingerprint,
        editedSelectedCut: edited.cutFingerprint,
        travelTerrainRequest: travel.terrainRequest,
        editedTerrainRequest: edited.terrainRequest,
        travelTerrainGeneration: travel.terrainGeneration,
        editedTerrainGeneration: edited.terrainGeneration,
        travelTerrainRevisionDigest: travel.terrainRevisionDigest,
        editedTerrainRevisionDigest: edited.terrainRevisionDigest,
        largestEnclosedSkyComponent: Math.max(
          ...[freshSpectatorEndpoint, pedestal, travel, edited, placed, spectator].map(
            (entry) => entry.largestEnclosedSkyComponent,
          ),
        ),
        largestTerrainInteriorSkyPixels: Math.max(
          ...[freshSpectatorEndpoint, pedestal, travel, edited, placed].map(
            (entry) => entry.terrainInteriorSkyPixels,
          ),
        ),
        exactLodDiscontinuities: 0,
        continuousRendererFrames: recorder.observedFrames,
        firstPlayableFrameSequence: recorder.firstPlayableFrameSequence ?? 0,
        reproductionScreenshotBytes: reproductionCapture.png.byteLength,
        reproductionReplayScreenshotBytes,
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
