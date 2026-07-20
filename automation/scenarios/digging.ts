import { execFileSync } from "node:child_process";
import type { Page } from "playwright";
import { ScenarioArguments } from "../lib/arguments.ts";
import { BrowserCapability, chromeWebGpuLaunchOptions } from "../lib/browser.ts";
import { type EngineClient, snapshotValue } from "../lib/engine.ts";
import { analyzeDiagnosticSky, type DiagnosticSkyAnalysis } from "../lib/image.ts";
import { percentile } from "../lib/metrics.ts";
import {
  captureRenderSnapshot,
  sampleRenderSnapshots,
  summarizeRenderPhase,
  type RenderSnapshotCapture,
} from "../lib/render-metrics.ts";
import { defineScenario, type ScenarioContext } from "../lib/scenario.ts";
import { startWorldStack, type WorldSource } from "../lib/world.ts";

const FAILURE =
  /panic|unreachable|runtimeerror|wgpu|webgpu|shader|sqlite|opfs|syncaccesshandle|nomodificationallowed|web lock request failed|no persistence leader|persistence .*failed|server rejected edit/iu;
const VOXELS_PER_METRE = 10;
const EDIT_TIMEOUT_MS = 30_000;

interface DigContinuity {
  readonly target: readonly [number, number, number];
  readonly latencyMs: number;
  readonly mutations: number;
  readonly observedTrackedReplacement: boolean;
  readonly maximumRequiredChunks: number;
  readonly minimumRenderableFraction: number;
  readonly minimumOwnedFraction: number;
  readonly maximumPresentedStrideVoxels: number;
  readonly captures: readonly RenderSnapshotCapture[];
}

interface Options {
  readonly source: WorldSource;
  readonly buildProfile: "debug" | "wasm-dev" | "release";
  readonly shaftDigs: number;
  readonly tunnelSteps: number;
  readonly performanceSeconds: number;
  readonly recordVideo: boolean;
}

function parseOptions(arguments_: readonly string[]): Options {
  const reader = new ScenarioArguments(arguments_);
  const options: Options = {
    source: reader.choice(
      "source",
      ["procedural-v16", "terrain-diffusion-30m"] as const,
      "procedural-v16",
    ),
    buildProfile: reader.choice("build", ["debug", "wasm-dev", "release"] as const, "release"),
    shaftDigs:
      reader.number("shaft-digs", {
        fallback: 4,
        integer: true,
        minimum: 2,
        maximum: 12,
      }) ?? 4,
    tunnelSteps:
      reader.number("tunnel-steps", {
        fallback: 32,
        integer: true,
        minimum: 4,
        maximum: 48,
      }) ?? 32,
    performanceSeconds:
      reader.number("performance-seconds", {
        fallback: 5,
        minimum: 2,
        maximum: 30,
      }) ?? 5,
    recordVideo: reader.flag("video"),
  };
  reader.assertEmpty();
  return options;
}

async function waitForSettledWorld(engine: EngineClient): Promise<readonly number[]> {
  return engine.waitForSnapshot(
    (snapshot) =>
      snapshotValue(snapshot, "quads") > 0 &&
      snapshotValue(snapshot, "pendingJobs") === 0 &&
      snapshotValue(snapshot, "surfaceInFlight") === 0 &&
      snapshotValue(snapshot, "allLodsReady") === 1,
    {
      timeoutMs: 90_000,
      intervalMs: 50,
      description: "digging fixture did not settle",
    },
  );
}

async function moveDistance(
  page: Page,
  engine: EngineClient,
  distanceMetres: number,
  key: "KeyW" | "KeyS",
  sprint = false,
): Promise<readonly number[]> {
  const before = await engine.snapshot();
  let latest = before;
  const started = performance.now();
  if (sprint) await page.keyboard.down("ShiftLeft");
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
    if (sprint) await page.keyboard.up("ShiftLeft");
  }
  throw new Error(
    `${key} movement covered less than ${distanceMetres}m: ${JSON.stringify({
      before: [snapshotValue(before, "cameraX"), snapshotValue(before, "cameraZ")],
      after: [snapshotValue(latest, "cameraX"), snapshotValue(latest, "cameraZ")],
    })}`,
  );
}

async function performDig(
  engine: EngineClient,
  target: readonly [number, number, number],
): Promise<DigContinuity> {
  const before = await engine.snapshot();
  const editsBefore = snapshotValue(before, "edits");
  if (!(await engine.submitDig(target[0], target[1], target[2], "sphere"))) {
    throw new Error(`dig submission was backpressured at ${target.join(",")}`);
  }

  const captures: RenderSnapshotCapture[] = [];
  const started = performance.now();
  let observedCommit = false;
  let observedTrackedReplacement = false;
  let maximumRequiredChunks = 0;
  let minimumRenderableFraction = 1;
  let minimumOwnedFraction = 1;
  let maximumPresentedStrideVoxels = 1;
  let stableCompleteSamples = 0;
  let latest = before;
  while (performance.now() - started < EDIT_TIMEOUT_MS) {
    const capture = await captureRenderSnapshot(engine);
    captures.push(capture);
    latest = capture.snapshot;
    const edits = snapshotValue(latest, "edits");
    const required = snapshotValue(latest, "editCanonicalRequired");
    const renderable = snapshotValue(latest, "editCanonicalRenderable");
    const owned = snapshotValue(latest, "editCanonicalOwned");
    maximumPresentedStrideVoxels = Math.max(
      maximumPresentedStrideVoxels,
      snapshotValue(latest, "presentedLodStrideVoxels"),
    );
    observedCommit ||= edits > editsBefore;
    if (required > 0) {
      observedTrackedReplacement = true;
      maximumRequiredChunks = Math.max(maximumRequiredChunks, required);
      minimumRenderableFraction = Math.min(minimumRenderableFraction, renderable / required);
      minimumOwnedFraction = Math.min(minimumOwnedFraction, owned / required);
    }
    const complete =
      observedCommit &&
      required === 0 &&
      snapshotValue(latest, "pendingJobs") === 0 &&
      snapshotValue(latest, "surfaceInFlight") === 0;
    stableCompleteSamples = complete ? stableCompleteSamples + 1 : 0;
    if (stableCompleteSamples >= 3) break;
    await engine.wait(8);
  }

  const mutations = snapshotValue(latest, "edits") - editsBefore;
  if (!observedCommit || mutations <= 0) {
    throw new Error(
      `dig did not commit solid voxels at ${target.join(",")}: ${JSON.stringify({
        editsBefore,
        editsAfter: snapshotValue(latest, "edits"),
        required: snapshotValue(latest, "editCanonicalRequired"),
      })}`,
    );
  }
  if (stableCompleteSamples < 3) {
    throw new Error(`dig replacement did not settle at ${target.join(",")}`);
  }
  return {
    target,
    latencyMs: performance.now() - started,
    mutations,
    observedTrackedReplacement,
    maximumRequiredChunks,
    minimumRenderableFraction,
    minimumOwnedFraction,
    maximumPresentedStrideVoxels,
    captures,
  };
}

async function waitForFall(engine: EngineClient, previousEyeY: number): Promise<readonly number[]> {
  return engine.waitForSnapshot(
    (snapshot) =>
      snapshotValue(snapshot, "cameraY") < previousEyeY - 0.35 &&
      snapshotValue(snapshot, "grounded") === 1,
    {
      timeoutMs: 8_000,
      intervalMs: 16,
      description: "player did not settle into the freshly dug shaft",
    },
  );
}

function continuitySummary(digs: readonly DigContinuity[]) {
  return {
    digs: digs.length,
    trackedReplacements: digs.filter((dig) => dig.observedTrackedReplacement).length,
    latencyMs: {
      median: percentile(
        digs.map((dig) => dig.latencyMs),
        0.5,
      ),
      p95: percentile(
        digs.map((dig) => dig.latencyMs),
        0.95,
      ),
      max: Math.max(...digs.map((dig) => dig.latencyMs), 0),
    },
    mutations: digs.reduce((sum, dig) => sum + dig.mutations, 0),
    maximumRequiredChunks: Math.max(...digs.map((dig) => dig.maximumRequiredChunks), 0),
    minimumRenderableFraction: Math.min(...digs.map((dig) => dig.minimumRenderableFraction), 1),
    minimumOwnedFraction: Math.min(...digs.map((dig) => dig.minimumOwnedFraction), 1),
    maximumPresentedStrideVoxels: Math.max(
      ...digs.map((dig) => dig.maximumPresentedStrideVoxels),
      1,
    ),
    renderDuringEdits: summarizeRenderPhase(digs.flatMap((dig) => dig.captures)),
  };
}

async function captureStepBackCoverage(
  page: Page,
  engine: EngineClient,
  distanceMetres: number,
): Promise<{
  readonly distanceMetres: number;
  readonly samples: number;
  readonly worst: DiagnosticSkyAnalysis;
  readonly captures: readonly DiagnosticSkyAnalysis[];
  readonly worstScreenshot: Buffer;
}> {
  const before = await engine.snapshot();
  const analyses: DiagnosticSkyAnalysis[] = [];
  let worstScreenshot = await page.screenshot();
  let worst = await analyzeDiagnosticSky(page, worstScreenshot, {
    x0: 0.08,
    x1: 0.92,
    y0: 0.08,
    y1: 0.82,
  });
  analyses.push(worst);
  let latest = before;
  const started = performance.now();
  await page.keyboard.down("KeyS");
  try {
    while (performance.now() - started < 15_000) {
      await page.waitForTimeout(120);
      latest = await engine.snapshot();
      const screenshot = await page.screenshot();
      const analysis = await analyzeDiagnosticSky(page, screenshot, {
        x0: 0.08,
        x1: 0.92,
        y0: 0.08,
        y1: 0.82,
      });
      analyses.push(analysis);
      if (analysis.diagnosticSkyPixels > worst.diagnosticSkyPixels) {
        worst = analysis;
        worstScreenshot = screenshot;
      }
      const distance = Math.hypot(
        snapshotValue(latest, "cameraX") - snapshotValue(before, "cameraX"),
        snapshotValue(latest, "cameraZ") - snapshotValue(before, "cameraZ"),
      );
      if (distance >= distanceMetres) {
        return {
          distanceMetres: distance,
          samples: analyses.length,
          worst,
          captures: analyses,
          worstScreenshot,
        };
      }
    }
  } finally {
    await page.keyboard.up("KeyS");
  }
  throw new Error(
    `tunnel step-back covered less than ${distanceMetres}m from ${JSON.stringify([
      snapshotValue(before, "cameraX"),
      snapshotValue(before, "cameraY"),
      snapshotValue(before, "cameraZ"),
    ])}`,
  );
}

async function runDigging(context: ScenarioContext, arguments_: readonly string[]) {
  const options = parseOptions(arguments_);
  const world = await startWorldStack(context, {
    fixture: {
      prefix: "voxels-digging-",
      source: options.source,
      spawnVoxels: [4208, 6082],
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
    label: "digging",
    viewport: { width: 1280, height: 720 },
    recordVideo: options.recordVideo,
    videoFilename: "digging.webm",
    ...world.clientRoute,
  });
  const { engine, page } = viewport;
  const contract = await engine.ready(90_000);
  await waitForSettledWorld(engine);
  await moveDistance(page, engine, 3, "KeyW", false);
  await engine.wait(250);

  const digs: DigContinuity[] = [];
  for (let index = 0; index < options.shaftDigs; index += 1) {
    const before = await engine.snapshot();
    const feetY = snapshotValue(before, "cameraY") - contract.semantics.playerEyeHeightMetres;
    const target = [
      Math.floor(snapshotValue(before, "cameraX") * VOXELS_PER_METRE),
      // Put the broad middle of the sphere through the support plane. Centering it one full
      // radius below the feet removes only a narrow cap and can leave the capsule on a support
      // ring even though more than one cubic metre was excavated.
      Math.floor(feetY * VOXELS_PER_METRE) - 2,
      Math.floor(snapshotValue(before, "cameraZ") * VOXELS_PER_METRE),
    ] as const;
    digs.push(await performDig(engine, target));
    await waitForFall(engine, snapshotValue(before, "cameraY"));
  }

  let tunnelPose = await engine.snapshot();
  await engine.setCameraLook(snapshotValue(tunnelPose, "yaw"), 0);
  for (let step = 0; step < options.tunnelSteps; step += 1) {
    tunnelPose = await engine.snapshot();
    const yaw = snapshotValue(tunnelPose, "yaw");
    const forwardX = Math.sin(yaw);
    const forwardZ = -Math.cos(yaw);
    const feetY = snapshotValue(tunnelPose, "cameraY") - contract.semantics.playerEyeHeightMetres;
    const targetX = Math.round(
      (snapshotValue(tunnelPose, "cameraX") + forwardX * 0.55) * VOXELS_PER_METRE,
    );
    const targetZ = Math.round(
      (snapshotValue(tunnelPose, "cameraZ") + forwardZ * 0.55) * VOXELS_PER_METRE,
    );
    for (const heightMetres of [0.45, 1.25]) {
      digs.push(
        await performDig(engine, [
          targetX,
          Math.round((feetY + heightMetres) * VOXELS_PER_METRE),
          targetZ,
        ]),
      );
    }
    await moveDistance(page, engine, 0.35, "KeyW");
  }

  await engine.setCameraLook(snapshotValue(await engine.snapshot(), "yaw"), 0);
  const enclosed = await engine.waitForSnapshot(
    (snapshot) =>
      snapshotValue(snapshot, "enclosure") > 0.94 && snapshotValue(snapshot, "pendingJobs") === 0,
    {
      timeoutMs: 20_000,
      intervalMs: 50,
      description: "finished tunnel did not become a sealed interior",
    },
  );

  // Drain edit and movement history before measuring ordinary rendering with the real atmosphere.
  await engine.snapshot();
  const performanceCaptures = await sampleRenderSnapshots(
    engine,
    options.performanceSeconds * 1_000,
    200,
  );
  const undergroundPerformance = summarizeRenderPhase(performanceCaptures);
  const normalScreenshot = await page.screenshot();
  await context.artifacts.write(
    "Underground normal rendering",
    "underground-normal.png",
    normalScreenshot,
    "image/png",
  );

  await engine.setDiagnosticSky([255, 0, 255]);
  const stepBack = await captureStepBackCoverage(page, engine, 5);
  await context.artifacts.write(
    "Worst tunnel diagnostic coverage",
    "tunnel-worst-coverage.png",
    stepBack.worstScreenshot,
    "image/png",
  );
  await engine.setDiagnosticSky(null);
  browser.assertHealthy();

  const continuity = continuitySummary(digs);
  const violations: string[] = [];
  if (continuity.trackedReplacements === 0)
    violations.push("no edit replacement remained observable long enough to validate continuity");
  if (continuity.minimumRenderableFraction < 1)
    violations.push("an edit replacement lost its renderable canonical mesh");
  if (continuity.minimumOwnedFraction < 1)
    violations.push("an edit replacement lost canonical LOD ownership");
  if (continuity.maximumPresentedStrideVoxels > 1)
    violations.push("the immediate viewport degraded to a coarse LOD during digging");
  if (stepBack.worst.diagnosticSkyPixels > 0)
    violations.push("the enclosed tunnel exposed diagnostic sky while stepping backward");
  if (undergroundPerformance.frameMs.p95 > 9.5)
    violations.push("underground frame p95 exceeded the 120 Hz tolerance of 9.5ms");
  if (undergroundPerformance.frameMs.above16_67ms > 0)
    violations.push("underground rendering produced frames slower than 60 Hz");
  if ((undergroundPerformance.gpu.totalMs?.p95 ?? 0) > 7.5)
    violations.push("underground total GPU p95 exceeded 7.5ms");
  if (undergroundPerformance.droppedSamples > 0)
    violations.push("underground frame telemetry dropped samples");

  const result = {
    ok: violations.length === 0,
    commit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(),
    dirty: execFileSync("git", ["status", "--porcelain"], { encoding: "utf8" }).trim() !== "",
    options,
    browser: browser.version,
    finalPose: {
      x: snapshotValue(enclosed, "cameraX"),
      y: snapshotValue(enclosed, "cameraY"),
      z: snapshotValue(enclosed, "cameraZ"),
      enclosure: snapshotValue(enclosed, "enclosure"),
    },
    continuity,
    undergroundPerformance,
    stepBack: {
      distanceMetres: stepBack.distanceMetres,
      samples: stepBack.samples,
      worst: stepBack.worst,
    },
    violations,
  };
  await context.artifacts.writeJson("Digging report", "report.json", result);
  if (!result.ok) throw new Error(`digging violations: ${violations.join(", ")}`);
  return {
    summary: "Digging continuity, underground performance, and tunnel coverage passed.",
    metrics: {
      editLatencyP95Ms: continuity.latencyMs.p95,
      frameP95Ms: undergroundPerformance.frameMs.p95,
      gpuP95Ms: undergroundPerformance.gpu.totalMs?.p95 ?? null,
      diagnosticSkyPixels: stepBack.worst.diagnosticSkyPixels,
    },
    details: result,
  };
}

export default defineScenario({
  id: "digging",
  kind: "validation",
  summary: "Validates edit continuity, underground performance, and tunnel LOD coverage.",
  uses: {
    world: true,
    browser: true,
    viewport: "browser",
    screenshots: true,
    video: true,
    metrics: true,
    rust: true,
  },
  timeoutMs: 1_800_000,
  run: runDigging,
});
