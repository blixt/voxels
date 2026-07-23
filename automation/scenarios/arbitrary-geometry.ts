import { execFileSync } from "node:child_process";
import type { Page } from "playwright";
import { ScenarioArguments } from "../lib/arguments.ts";
import { BrowserCapability, chromeWebGpuLaunchOptions } from "../lib/browser.ts";
import { type EngineClient, snapshotValue } from "../lib/engine.ts";
import {
  analyzeDiagnosticSky,
  compareRenderedImages,
  type NormalizedImageRegion,
  type RenderedImageComparison,
} from "../lib/image.ts";
import {
  sampleRenderSnapshots,
  summarizeRenderPhase,
  type RenderPhaseSummary,
} from "../lib/render-metrics.ts";
import { defineScenario, type ScenarioContext } from "../lib/scenario.ts";
import { startWorldStack, type WorldSource } from "../lib/world.ts";

const FAILURE =
  /panic|unreachable|runtimeerror|wgpu|webgpu|shader|sqlite|opfs|syncaccesshandle|nomodificationallowed|web lock request failed|no persistence leader|persistence .*failed|server rejected edit/iu;
const VOXELS_PER_METRE = 10;
const EDIT_TIMEOUT_MS = 30_000;
const EXACT_BOUNDARIES = [192, 480, 960, 1_920, 3_840, 6_144, 12_288, 24_576] as const;
// The smallest legal canonical square ends at 3.2 m. Fixtures are centred at least 4.2 m away,
// making this a deterministic exact-to-stride-two handoff without moving the camera.
const FAR_BOUNDARIES = [32, 64, 128, 256, 512, 1_024, 2_048, 4_096] as const;
const STRUCTURE_ROI = Object.freeze({ x0: 0.22, x1: 0.78, y0: 0.15, y1: 0.88 });
const TUNNEL_ROI = Object.freeze({ x0: 0.08, x1: 0.92, y0: 0.08, y1: 0.92 });
const MAX_TOPOLOGY_COMPONENT_PIXELS = 4;

type Vector3 = readonly [number, number, number];
type VoxelVector3 = readonly [number, number, number];

interface Options {
  readonly source: WorldSource;
  readonly buildProfile: "debug" | "wasm-dev" | "release";
  readonly viewport: { readonly width: number; readonly height: number };
  readonly deviceScaleFactor: number;
  readonly performanceSeconds: number;
}

interface OwnershipCapture {
  readonly label: string;
  readonly exactSnapshot: readonly number[];
  readonly farSnapshot: readonly number[];
  readonly exactDiagnosticSkyPixels: number;
  readonly farDiagnosticSkyPixels: number;
  readonly geometry: RenderedImageComparison;
  readonly appearance: RenderedImageComparison;
  readonly performance: RenderPhaseSummary;
}

function parseOptions(arguments_: readonly string[]): Options {
  const reader = new ScenarioArguments(arguments_);
  const viewport = reader.pair("viewport", {
    fallback: [1280, 720],
    separator: "x",
    integer: true,
    minimum: 240,
  });
  if (viewport === undefined) throw new Error("viewport default is missing");
  const options: Options = {
    source: reader.choice(
      "source",
      ["procedural-v16", "terrain-diffusion-30m"] as const,
      "procedural-v16",
    ),
    buildProfile: reader.choice("build", ["debug", "wasm-dev", "release"] as const, "release"),
    viewport: { width: viewport[0], height: viewport[1] },
    deviceScaleFactor: reader.number("dpr", { fallback: 1, minimum: 0.5, maximum: 4 }) ?? 1,
    performanceSeconds:
      reader.number("performance-seconds", { fallback: 3, minimum: 1, maximum: 30 }) ?? 3,
  };
  reader.assertEmpty();
  return options;
}

function cameraPosition(snapshot: readonly number[]): Vector3 {
  return [
    snapshotValue(snapshot, "cameraX"),
    snapshotValue(snapshot, "cameraY"),
    snapshotValue(snapshot, "cameraZ"),
  ];
}

function required(values: readonly number[], index: number, label: string): number {
  const value = values[index];
  if (value === undefined) throw new Error(`${label} omitted value ${index}`);
  return value;
}

async function waitForSettledWorld(
  engine: EngineClient,
  description: string,
  timeoutMs = 90_000,
): Promise<readonly number[]> {
  let stableSamples = 0;
  let previousFingerprint: string | undefined;
  const settled = await engine.waitForSnapshot(
    (snapshot) => {
      const fingerprint = `${snapshotValue(snapshot, "viewportFingerprintHigh24")}:${snapshotValue(snapshot, "viewportFingerprintLow24")}:${snapshotValue(snapshot, "quads")}`;
      const ready =
        snapshotValue(snapshot, "quads") > 0 &&
        snapshotValue(snapshot, "canonicalImmediateResident") ===
          snapshotValue(snapshot, "canonicalImmediateRequired") &&
        snapshotValue(snapshot, "canonicalSurfaceCellsResident") ===
          snapshotValue(snapshot, "canonicalSurfaceCellsRequired") &&
        snapshotValue(snapshot, "surfaceQueued") === 0 &&
        snapshotValue(snapshot, "surfaceDirty") === 0 &&
        snapshotValue(snapshot, "surfaceInFlight") === 0 &&
        snapshotValue(snapshot, "lodIncompleteTransitionEdges") === 0;
      stableSamples = ready && fingerprint === previousFingerprint ? stableSamples + 1 : 0;
      previousFingerprint = fingerprint;
      return stableSamples >= 3;
    },
    { timeoutMs, intervalMs: 25, description },
  );
  // Retire the deliberately short old-cut overlay before comparing ownership policies.
  await engine.wait(300);
  return settled;
}

async function moveDistance(
  page: Page,
  engine: EngineClient,
  distanceMetres: number,
  key: "KeyW" | "KeyS",
): Promise<readonly number[]> {
  const before = await engine.snapshot();
  let latest = before;
  const started = performance.now();
  await page.keyboard.down(key);
  try {
    while (performance.now() - started < 20_000) {
      await page.waitForTimeout(25);
      latest = await engine.snapshot();
      const distance = Math.hypot(
        snapshotValue(latest, "cameraX") - snapshotValue(before, "cameraX"),
        snapshotValue(latest, "cameraZ") - snapshotValue(before, "cameraZ"),
      );
      if (distance >= distanceMetres) return latest;
    }
  } finally {
    await page.keyboard.up(key);
  }
  throw new Error(
    `${key} covered less than ${distanceMetres}m: ${JSON.stringify({
      before: cameraPosition(before),
      after: cameraPosition(latest),
    })}`,
  );
}

async function waitForEdit(
  engine: EngineClient,
  editsBefore: number,
  label: string,
): Promise<readonly number[]> {
  return engine.waitForSnapshot(
    (snapshot) => {
      const required = snapshotValue(snapshot, "editCanonicalRequired");
      return (
        snapshotValue(snapshot, "edits") > editsBefore &&
        snapshotValue(snapshot, "editCanonicalRenderable") === required &&
        snapshotValue(snapshot, "editCanonicalOwned") === required &&
        snapshotValue(snapshot, "pendingJobs") === 0 &&
        snapshotValue(snapshot, "surfaceInFlight") === 0
      );
    },
    { timeoutMs: EDIT_TIMEOUT_MS, intervalMs: 25, description: `${label} did not converge` },
  );
}

async function dig(
  engine: EngineClient,
  target: VoxelVector3,
  shape: "sphere" | "cube",
  label: string,
): Promise<readonly number[]> {
  const committed = await submitDigIfSolid(engine, target, shape, label);
  if (committed === undefined) {
    throw new Error(`${label} did not commit solid voxels at ${target.join(",")}`);
  }
  return committed;
}

async function submitDigIfSolid(
  engine: EngineClient,
  target: VoxelVector3,
  shape: "sphere" | "cube",
  label: string,
): Promise<readonly number[] | undefined> {
  const before = await engine.snapshot();
  const editsBefore = snapshotValue(before, "edits");
  if (!(await engine.submitDig(target[0], target[1], target[2], shape))) {
    throw new Error(`${label} was backpressured at ${target.join(",")}`);
  }
  const deadline = performance.now() + 2_000;
  while (performance.now() < deadline) {
    const after = await engine.snapshot();
    if (snapshotValue(after, "edits") > editsBefore) {
      return waitForEdit(engine, editsBefore, label);
    }
    if (
      snapshotValue(after, "editCanonicalRenderable") ===
        snapshotValue(after, "editCanonicalRequired") &&
      snapshotValue(after, "editCanonicalOwned") ===
        snapshotValue(after, "editCanonicalRequired") &&
      snapshotValue(after, "pendingJobs") === 0 &&
      snapshotValue(after, "surfaceInFlight") === 0
    ) {
      await engine.wait(250);
      const confirmed = await engine.snapshot();
      if (
        snapshotValue(confirmed, "edits") === editsBefore &&
        snapshotValue(confirmed, "editCanonicalRenderable") ===
          snapshotValue(confirmed, "editCanonicalRequired") &&
        snapshotValue(confirmed, "editCanonicalOwned") ===
          snapshotValue(confirmed, "editCanonicalRequired") &&
        snapshotValue(confirmed, "pendingJobs") === 0 &&
        snapshotValue(confirmed, "surfaceInFlight") === 0
      ) {
        return undefined;
      }
    }
    await engine.wait(25);
  }
  throw new Error(`${label} neither remained empty nor converged`);
}

async function clearPlacement(
  engine: EngineClient,
  target: VoxelVector3,
  label: string,
): Promise<void> {
  // An already-empty volume has no commit. A non-empty volume must completely converge before the
  // corresponding placement, otherwise the test would race two authoritative edit operations.
  await submitDigIfSolid(engine, target, "cube", `${label} clearance`);
}

async function place(
  engine: EngineClient,
  target: VoxelVector3,
  materialId: number,
  label: string,
): Promise<readonly number[]> {
  const before = await engine.snapshot();
  const editsBefore = snapshotValue(before, "edits");
  if (!(await engine.submitPlace(target[0], target[1], target[2], materialId, "cube"))) {
    throw new Error(`${label} was backpressured at ${target.join(",")}`);
  }
  return waitForEdit(engine, editsBefore, label);
}

function fixtureTarget(
  origin: readonly number[],
  forwardVoxels: number,
  rightVoxels: number,
  y: number,
): VoxelVector3 {
  const yaw = snapshotValue(origin, "yaw");
  const originX = Math.round(snapshotValue(origin, "cameraX") * VOXELS_PER_METRE);
  const originZ = Math.round(snapshotValue(origin, "cameraZ") * VOXELS_PER_METRE);
  return [
    Math.round(originX + Math.sin(yaw) * forwardVoxels + Math.cos(yaw) * rightVoxels),
    y,
    Math.round(originZ - Math.cos(yaw) * forwardVoxels + Math.sin(yaw) * rightVoxels),
  ];
}

async function aimAt(
  engine: EngineClient,
  target: Vector3,
  description: string,
): Promise<readonly number[]> {
  const snapshot = await engine.snapshot();
  const origin = cameraPosition(snapshot);
  const dx = target[0] - origin[0];
  const dy = target[1] - origin[1];
  const dz = target[2] - origin[2];
  return engine.setCameraLook(Math.atan2(dx, -dz), Math.atan2(dy, Math.hypot(dx, dz)), {
    timeoutMs: 10_000,
    intervalMs: 10,
    description,
  });
}

function voxelMetres(target: VoxelVector3): Vector3 {
  return target.map((value) => value / VOXELS_PER_METRE) as [number, number, number];
}

function topologyMetrics(comparison: RenderedImageComparison) {
  const geometry = comparison.diagnosticGeometry;
  if (geometry === null) throw new Error("diagnostic geometry comparison was not requested");
  return {
    falseSkyPixels: geometry.rightOnlyOccupancyPixels,
    largestFalseSkyComponentPixels: geometry.largestRightOnlyComponentPixels,
    opaqueFillPixels: geometry.leftOnlyOccupancyPixels,
    largestOpaqueFillComponentPixels: geometry.largestLeftOnlyComponentPixels,
    occupancyJaccard: geometry.occupancyJaccard,
  };
}

async function captureOwnershipPair(
  context: ScenarioContext,
  page: Page,
  engine: EngineClient,
  label: string,
  target: Vector3,
  roi: NormalizedImageRegion,
  performanceSeconds: number,
): Promise<OwnershipCapture> {
  await engine.setLodBoundaryHalfExtents(EXACT_BOUNDARIES);
  await aimAt(engine, target, `${label} exact view did not align`);
  const exactSnapshot = await waitForSettledWorld(engine, `${label} exact view did not settle`);
  const exactNormal = await page.screenshot();
  await context.artifacts.write(`${label} exact`, `${label}-exact.png`, exactNormal, "image/png");
  await engine.setDiagnosticSky([255, 0, 255]);
  const exactDiagnostic = await page.screenshot();
  await context.artifacts.write(
    `${label} exact diagnostic`,
    `${label}-exact-diagnostic.png`,
    exactDiagnostic,
    "image/png",
  );
  const exactSky = await analyzeDiagnosticSky(page, exactDiagnostic, roi);

  await engine.setLodBoundaryHalfExtents(FAR_BOUNDARIES);
  const farSnapshot = await waitForSettledWorld(engine, `${label} far view did not settle`);
  const farDiagnostic = await page.screenshot();
  await context.artifacts.write(
    `${label} far diagnostic`,
    `${label}-far-diagnostic.png`,
    farDiagnostic,
    "image/png",
  );
  const farSky = await analyzeDiagnosticSky(page, farDiagnostic, roi);
  const geometry = await compareRenderedImages(page, farDiagnostic, exactDiagnostic, {
    region: roi,
    footprintPixels: 1,
    diagnosticGeometry: true,
  });

  await engine.setDiagnosticSky(null);
  const farNormal = await page.screenshot();
  await context.artifacts.write(`${label} far`, `${label}-far.png`, farNormal, "image/png");
  const appearance = await compareRenderedImages(page, farNormal, exactNormal, {
    region: roi,
    footprintPixels: 2,
  });
  const performance = summarizeRenderPhase(
    await sampleRenderSnapshots(engine, performanceSeconds * 1_000, 200),
  );
  return {
    label,
    exactSnapshot,
    farSnapshot,
    exactDiagnosticSkyPixels: exactSky.diagnosticSkyPixels,
    farDiagnosticSkyPixels: farSky.diagnosticSkyPixels,
    geometry,
    appearance,
    performance,
  };
}

function captureReport(capture: OwnershipCapture) {
  return {
    label: capture.label,
    exact: {
      camera: cameraPosition(capture.exactSnapshot),
      presentedStrideVoxels: snapshotValue(capture.exactSnapshot, "presentedLodStrideVoxels"),
      incompleteTransitionEdges: snapshotValue(
        capture.exactSnapshot,
        "lodIncompleteTransitionEdges",
      ),
      enclosedViewRequired: snapshotValue(capture.exactSnapshot, "enclosedViewRequired"),
      enclosedViewOwned: snapshotValue(capture.exactSnapshot, "enclosedViewOwned"),
      diagnosticSkyPixels: capture.exactDiagnosticSkyPixels,
    },
    far: {
      camera: cameraPosition(capture.farSnapshot),
      presentedStrideVoxels: snapshotValue(capture.farSnapshot, "presentedLodStrideVoxels"),
      incompleteTransitionEdges: snapshotValue(capture.farSnapshot, "lodIncompleteTransitionEdges"),
      enclosedViewRequired: snapshotValue(capture.farSnapshot, "enclosedViewRequired"),
      enclosedViewOwned: snapshotValue(capture.farSnapshot, "enclosedViewOwned"),
      diagnosticSkyPixels: capture.farDiagnosticSkyPixels,
    },
    topology: topologyMetrics(capture.geometry),
    appearance: {
      ssim: capture.appearance.ssim,
      meanAbsoluteLinearLumaDelta: capture.appearance.meanAbsoluteLinearLumaDelta,
    },
    performance: capture.performance,
  };
}

function captureViolations(capture: OwnershipCapture): string[] {
  const topology = topologyMetrics(capture.geometry);
  const violations: string[] = [];
  if (topology.largestFalseSkyComponentPixels > MAX_TOPOLOGY_COMPONENT_PIXELS) {
    violations.push(
      `${capture.label} lost exact geometry to ${topology.largestFalseSkyComponentPixels}-pixel false sky`,
    );
  }
  if (topology.largestOpaqueFillComponentPixels > MAX_TOPOLOGY_COMPONENT_PIXELS) {
    violations.push(
      `${capture.label} filled exact air with a ${topology.largestOpaqueFillComponentPixels}-pixel opaque region`,
    );
  }
  if (capture.performance.frameMs.p95 > 12) {
    violations.push(`${capture.label} frame p95 exceeded 12ms`);
  }
  if ((capture.performance.gpu.totalMs?.p95 ?? 0) > 7.5) {
    violations.push(`${capture.label} total GPU p95 exceeded 7.5ms`);
  }
  return violations;
}

async function prepareInventory(
  engine: EngineClient,
  origin: readonly number[],
  groundY: number,
  requiredUnits: number,
): Promise<{ readonly materialId: number; readonly inventory: readonly number[] }> {
  const findMaterial = (inventory: readonly number[]) =>
    Array.from({ length: inventory.length - 2 }, (_unused, index) => index + 1)
      .filter((id) => required(inventory, id + 1, "material inventory") >= requiredUnits)
      .sort(
        (left, right) =>
          required(inventory, right + 1, "material inventory") -
          required(inventory, left + 1, "material inventory"),
      )[0];
  let inventory = await engine.inventory();
  let materialId = findMaterial(inventory);
  for (const forward of [-12, -24, -36, -48] as const) {
    for (const right of [-18, -6, 6, 18] as const) {
      await submitDigIfSolid(
        engine,
        fixtureTarget(origin, forward, right, groundY - 3),
        "cube",
        `inventory dig ${forward}/${right}`,
      );
      inventory = await engine.inventory();
      materialId = findMaterial(inventory);
      if (materialId !== undefined) return { materialId, inventory };
    }
  }
  if (materialId === undefined) {
    throw new Error(
      `inventory preparation did not produce ${requiredUnits} units of one material: ${JSON.stringify(inventory)}`,
    );
  }
  return { materialId, inventory };
}

async function excavateTunnelFixture(
  page: Page,
  engine: EngineClient,
  eyeHeightMetres: number,
): Promise<{ readonly terminator: Vector3; readonly branch: Vector3 }> {
  const editsBefore = snapshotValue(await engine.snapshot(), "edits");
  for (let shaft = 0; shaft < 3; shaft += 1) {
    const before = await engine.snapshot();
    const feetY = snapshotValue(before, "cameraY") - eyeHeightMetres;
    await dig(
      engine,
      [
        Math.floor(snapshotValue(before, "cameraX") * VOXELS_PER_METRE),
        Math.floor(feetY * VOXELS_PER_METRE) - 2,
        Math.floor(snapshotValue(before, "cameraZ") * VOXELS_PER_METRE),
      ],
      "sphere",
      `shaft dig ${shaft + 1}`,
    );
    await engine.waitForSnapshot(
      (snapshot) =>
        snapshotValue(snapshot, "cameraY") < snapshotValue(before, "cameraY") - 0.35 &&
        snapshotValue(snapshot, "grounded") === 1,
      { timeoutMs: 8_000, intervalMs: 16, description: `shaft fall ${shaft + 1} did not settle` },
    );
  }

  const tunnelYaw = Math.PI / 2;
  await engine.setCameraLook(tunnelYaw, 0);
  for (let step = 0; step < 16; step += 1) {
    const pose = await engine.snapshot();
    const feetY = snapshotValue(pose, "cameraY") - eyeHeightMetres;
    const targetX = Math.round(
      (snapshotValue(pose, "cameraX") + Math.sin(tunnelYaw) * 0.55) * VOXELS_PER_METRE,
    );
    const targetZ = Math.round(
      (snapshotValue(pose, "cameraZ") - Math.cos(tunnelYaw) * 0.55) * VOXELS_PER_METRE,
    );
    for (const heightMetres of [0.45, 1.25]) {
      await submitDigIfSolid(
        engine,
        [targetX, Math.round((feetY + heightMetres) * VOXELS_PER_METRE), targetZ],
        "sphere",
        `main tunnel ${step + 1}/${heightMetres}`,
      );
    }
    await moveDistance(page, engine, 0.35, "KeyW");
  }

  const junction = await engine.snapshot();
  const feetY = snapshotValue(junction, "cameraY") - eyeHeightMetres;
  const branchYaw = tunnelYaw - Math.PI / 2;
  for (const distance of [0.6, 1.15, 1.7, 2.25, 2.8]) {
    for (const heightMetres of [0.45, 1.25]) {
      await submitDigIfSolid(
        engine,
        [
          Math.round(
            (snapshotValue(junction, "cameraX") + Math.sin(branchYaw) * distance) *
              VOXELS_PER_METRE,
          ),
          Math.round((feetY + heightMetres) * VOXELS_PER_METRE),
          Math.round(
            (snapshotValue(junction, "cameraZ") - Math.cos(branchYaw) * distance) *
              VOXELS_PER_METRE,
          ),
        ],
        "sphere",
        `branch tunnel ${distance}/${heightMetres}`,
      );
    }
  }
  const terminator = [
    snapshotValue(junction, "cameraX") + Math.sin(tunnelYaw) * 0.9,
    feetY + 0.9,
    snapshotValue(junction, "cameraZ") - Math.cos(tunnelYaw) * 0.9,
  ] as const;
  const branch = [
    snapshotValue(junction, "cameraX") + Math.sin(branchYaw) * 1.7,
    feetY + 0.9,
    snapshotValue(junction, "cameraZ") - Math.cos(branchYaw) * 1.7,
  ] as const;
  const editsAfter = snapshotValue(await engine.snapshot(), "edits");
  if (editsAfter <= editsBefore) {
    throw new Error("branched tunnel fixture did not commit any authoritative edits");
  }
  await engine.setCameraLook(tunnelYaw, 0);
  await moveDistance(page, engine, 4.2, "KeyS");
  await aimAt(engine, terminator, "branched tunnel view did not align");
  await engine.waitForSnapshot((snapshot) => snapshotValue(snapshot, "enclosure") > 0.9, {
    timeoutMs: 15_000,
    intervalMs: 50,
    description: "branched tunnel did not become enclosed",
  });
  return { terminator, branch };
}

async function runArbitraryGeometry(context: ScenarioContext, arguments_: readonly string[]) {
  const options = parseOptions(arguments_);
  const world = await startWorldStack(context, {
    fixture: {
      prefix: "voxels-arbitrary-geometry-",
      source: options.source,
      spawnVoxels: [4_208, 6_082],
      spawnPillarHeightVoxels: 1,
      spawnPillarRadiusVoxels: 1,
      spawnProtectionRadiusVoxels: 1,
      dayLengthSeconds: 0,
      dayFractionAtUnixEpoch: 0.42,
      weatherCycleSeconds: 0,
      weatherFractionAtUnixEpoch: 0.08,
      cloudVelocityMetresPerSecond: [0, 0],
    },
    service: { metal: options.source === "terrain-diffusion-30m" },
    web: { buildProfile: options.buildProfile },
  });
  const browser = await BrowserCapability.start(context, {
    warningPattern: FAILURE,
    launch: chromeWebGpuLaunchOptions(),
  });
  const viewport = await browser.open({
    url: world.url,
    label: "arbitrary-geometry",
    viewport: options.viewport,
    deviceScaleFactor: options.deviceScaleFactor,
    ...world.clientRoute,
  });
  const { engine, page } = viewport;
  const contract = await engine.ready(90_000);
  await engine.setLodBoundaryHalfExtents(EXACT_BOUNDARIES);
  await waitForSettledWorld(engine, "arbitrary-geometry world did not settle");
  await engine.setCameraLook(0, 0);
  const workOrigin = await moveDistance(page, engine, 10, "KeyW");
  const groundY = Math.round(
    (snapshotValue(workOrigin, "cameraY") - contract.semantics.playerEyeHeightMetres) *
      VOXELS_PER_METRE,
  );

  const prepared = await prepareInventory(engine, workOrigin, groundY, 4_000);
  const floating = fixtureTarget(workOrigin, 48, -12, groundY + 15);
  const build = [
    fixtureTarget(workOrigin, 48, 12, groundY + 15),
    fixtureTarget(workOrigin, 48, 12, groundY + 25),
    fixtureTarget(workOrigin, 48, 22, groundY + 25),
  ] as const;
  for (const [index, target] of [floating, ...build].entries()) {
    await clearPlacement(engine, target, `fixture ${index + 1}`);
  }
  await place(engine, floating, prepared.materialId, "floating structure");
  for (const [index, target] of build.entries()) {
    await place(engine, target, prepared.materialId, `branched build cube ${index + 1}`);
  }
  await waitForSettledWorld(engine, "outdoor edited fixtures did not settle");

  const floatingCapture = await captureOwnershipPair(
    context,
    page,
    engine,
    "floating-structure",
    voxelMetres(floating),
    STRUCTURE_ROI,
    options.performanceSeconds,
  );
  const buildAim = voxelMetres(build[1]);
  const buildCapture = await captureOwnershipPair(
    context,
    page,
    engine,
    "edited-build",
    buildAim,
    STRUCTURE_ROI,
    options.performanceSeconds,
  );

  await engine.setLodBoundaryHalfExtents(EXACT_BOUNDARIES);
  await engine.setCameraLook(Math.PI / 2, 0);
  const tunnelFixture = await excavateTunnelFixture(
    page,
    engine,
    contract.semantics.playerEyeHeightMetres,
  );
  const tunnelCapture = await captureOwnershipPair(
    context,
    page,
    engine,
    "branched-tunnel",
    tunnelFixture.terminator,
    TUNNEL_ROI,
    options.performanceSeconds,
  );
  browser.assertHealthy();

  const captures = [floatingCapture, buildCapture, tunnelCapture];
  const violations = captures.flatMap(captureViolations);
  const tunnelFarRequired = snapshotValue(tunnelCapture.farSnapshot, "enclosedViewRequired");
  const tunnelFarOwned = snapshotValue(tunnelCapture.farSnapshot, "enclosedViewOwned");
  if (tunnelFarRequired === 0 || tunnelFarOwned !== tunnelFarRequired) {
    violations.push(
      `branched tunnel did not cross the surface handoff with complete exact-volume ownership (${tunnelFarOwned}/${tunnelFarRequired})`,
    );
  }
  const result = {
    ok: violations.length === 0,
    commit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(),
    dirty: execFileSync("git", ["status", "--porcelain"], { encoding: "utf8" }).trim() !== "",
    source: options.source,
    browser: browser.version,
    options,
    ownershipPolicies: { exact: EXACT_BOUNDARIES, far: FAR_BOUNDARIES },
    topologyTolerance: { maximumConnectedComponentPixels: MAX_TOPOLOGY_COMPONENT_PIXELS },
    fixtures: {
      floating: { voxel: floating, materialId: prepared.materialId },
      build: { voxels: build, materialId: prepared.materialId },
      tunnel: tunnelFixture,
    },
    captures: captures.map(captureReport),
    violations,
  };
  await context.artifacts.writeJson("Arbitrary geometry report", "report.json", result);
  if (!result.ok) throw new Error(`arbitrary geometry violations: ${violations.join("; ")}`);
  return {
    summary: "Edited standalone structures and branched tunnel survived the exact-to-far handoff.",
    metrics: {
      maximumFalseSkyComponentPixels: Math.max(
        ...captures.map(
          (capture) => topologyMetrics(capture.geometry).largestFalseSkyComponentPixels,
        ),
      ),
      maximumOpaqueFillComponentPixels: Math.max(
        ...captures.map(
          (capture) => topologyMetrics(capture.geometry).largestOpaqueFillComponentPixels,
        ),
      ),
      maximumFrameP95Ms: Math.max(...captures.map((capture) => capture.performance.frameMs.p95)),
    },
    details: result,
  };
}

export default defineScenario({
  id: "arbitrary-geometry",
  kind: "validation",
  summary:
    "Validates standalone edited structures and branched volume across the exact-to-far handoff.",
  uses: {
    world: true,
    browser: true,
    viewport: "browser",
    screenshots: true,
    metrics: true,
    rust: true,
  },
  timeoutMs: 1_800_000,
  run: runArbitraryGeometry,
});
