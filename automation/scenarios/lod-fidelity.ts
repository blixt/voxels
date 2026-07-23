import { execFileSync } from "node:child_process";
import type { Page } from "playwright";
import { ScenarioArguments } from "../lib/arguments.ts";
import { BrowserCapability, chromeWebGpuLaunchOptions } from "../lib/browser.ts";
import { type EngineClient, type LodBoundaryHalfExtents, snapshotValue } from "../lib/engine.ts";
import {
  analyzeDiagnosticSky,
  compareRenderedImages,
  type DiagnosticSkyAnalysis,
  type RenderedImageComparison,
} from "../lib/image.ts";
import {
  sampleRenderSnapshots,
  summarizeRenderPhase,
  type RenderPhaseSummary,
} from "../lib/render-metrics.ts";
import { defineScenario, type ScenarioContext } from "../lib/scenario.ts";
import { startWorldStack, type WorldSource } from "../lib/world.ts";
import type { WasmBuildProfile } from "../../scripts/build-wasm.ts";

const FAILURE =
  /panic|unreachable|runtimeerror|wgpu|webgpu|shader|sqlite|opfs|syncaccesshandle|nomodificationallowed|web lock request failed|no persistence leader|persistence .*failed/i;
const GEOMETRY_ROI = Object.freeze({ x0: 0.02, x1: 0.98, y0: 0.14, y1: 0.94 });
const GROUND_ROI = Object.freeze({ x0: 0.02, x1: 0.98, y0: 0.5, y1: 0.96 });

interface LodProfile {
  readonly id: string;
  readonly family: "uniform" | "near-biased";
  readonly boundaries: LodBoundaryHalfExtents;
}

const LOD_PROFILES = Object.freeze([
  {
    id: "compact",
    family: "uniform",
    boundaries: [96, 256, 512, 1_024, 2_048, 3_072, 6_144, 12_288],
  },
  {
    id: "baseline",
    family: "uniform",
    boundaries: [128, 320, 640, 1_280, 2_560, 4_096, 8_192, 16_384],
  },
  {
    id: "uniform-125",
    family: "uniform",
    boundaries: [160, 416, 832, 1_664, 3_328, 5_120, 10_240, 20_480],
  },
  {
    id: "near-biased",
    family: "near-biased",
    boundaries: [192, 480, 832, 1_536, 3_072, 5_120, 10_240, 20_480],
  },
  {
    id: "uniform-150",
    family: "uniform",
    boundaries: [192, 480, 960, 1_920, 3_840, 6_144, 12_288, 24_576],
  },
] as const satisfies readonly LodProfile[]);

type ArenaId = "forest" | "valley";

interface Arena {
  readonly id: ArenaId;
  readonly spawn: readonly [number, number];
  readonly look: readonly [number, number];
  readonly headingOffsets: readonly number[];
  readonly pillarHeight: number;
  readonly pillarRadius: number;
}

const ARENAS = Object.freeze({
  forest: {
    id: "forest",
    spawn: [1_614, 294],
    look: [3.326_412_741_337_916, -0.312_000_215_053_558],
    headingOffsets: [-0.36, 0, 0.36],
    pillarHeight: 1,
    pillarRadius: 3,
  },
  valley: {
    id: "valley",
    spawn: [4_194, 6_034],
    look: [2.074_606, -0.371_797],
    headingOffsets: [-0.3, 0, 0.3],
    pillarHeight: 40,
    pillarRadius: 3,
  },
} as const satisfies Readonly<Record<ArenaId, Arena>>);

interface Options {
  readonly arena: Arena;
  readonly source: WorldSource;
  readonly viewport: { readonly width: number; readonly height: number };
  readonly deviceScaleFactor: number;
  readonly measureMs: number;
  readonly buildProfile: WasmBuildProfile;
}

interface HeadingCapture {
  readonly heading: number;
  readonly normal: Buffer;
  readonly diagnostic: Buffer;
  readonly sky: DiagnosticSkyAnalysis;
}

interface ComparedHeading {
  readonly heading: number;
  readonly sky: DiagnosticSkyAnalysis;
  readonly appearance: RenderedImageComparison;
  readonly geometry: RenderedImageComparison;
}

interface ProfileCapture {
  readonly profile: LodProfile;
  readonly headings: readonly HeadingCapture[];
  readonly performance: RenderPhaseSummary;
}

interface ViewportCompletenessSample {
  readonly heading: number;
  readonly atTarget: ReturnType<typeof viewportState>;
  readonly firstPresented: ReturnType<typeof viewportState>;
  readonly afterCapture: ReturnType<typeof viewportState>;
  readonly settled: ReturnType<typeof viewportState>;
  readonly comparison: RenderedImageComparison;
}

function viewportState(snapshot: readonly number[]) {
  return {
    frameSequence: snapshotValue(snapshot, "frameSequence"),
    gpuSampleId: snapshotValue(snapshot, "gpuSampleId"),
    visibleChunks: snapshotValue(snapshot, "visibleChunks"),
    drawCalls: snapshotValue(snapshot, "drawCalls"),
    quads: snapshotValue(snapshot, "quads"),
    selectedSlices: snapshotValue(snapshot, "drawListSelectedSlices"),
    viewportFingerprint: [
      snapshotValue(snapshot, "viewportFingerprintLow24"),
      snapshotValue(snapshot, "viewportFingerprintHigh24"),
    ] as const,
    pendingJobs: snapshotValue(snapshot, "pendingJobs"),
    surfaceInFlight: snapshotValue(snapshot, "surfaceInFlight"),
    allLodsReady: snapshotValue(snapshot, "allLodsReady"),
    lodTransitionQuads: snapshotValue(snapshot, "lodTransitionQuads"),
  };
}

function parseOptions(arguments_: readonly string[]): Options {
  const reader = new ScenarioArguments(arguments_);
  const arenaId = reader.choice("arena", ["forest", "valley"] as const, "forest");
  const viewport = reader.pair("viewport", {
    fallback: [1_500, 1_000],
    separator: "x",
    integer: true,
    minimum: 240,
    maximum: 4_096,
  }) ?? [1_500, 1_000];
  const options: Options = {
    arena: ARENAS[arenaId],
    source: reader.choice(
      "source",
      ["procedural-v16", "terrain-diffusion-30m"] as const,
      "terrain-diffusion-30m",
    ),
    viewport: { width: viewport[0], height: viewport[1] },
    deviceScaleFactor: reader.number("dpr", { fallback: 2, minimum: 0.5, maximum: 4 }) ?? 2,
    measureMs:
      reader.number("measure", {
        fallback: 2_500,
        minimum: 500,
        maximum: 30_000,
        integer: true,
      }) ?? 2_500,
    buildProfile: reader.choice("build", ["debug", "wasm-dev", "release"] as const, "release"),
  };
  reader.assertEmpty();
  return options;
}

function requiredGpu(
  distribution: RenderPhaseSummary["gpu"]["totalMs"],
  label: string,
): NonNullable<RenderPhaseSummary["gpu"]["totalMs"]> {
  if (distribution === null) throw new Error(`${label} GPU timestamps are unavailable`);
  return distribution;
}

async function waitForSettledGeometry(engine: EngineClient): Promise<readonly number[]> {
  let stable = 0;
  let previousFingerprint: readonly [number, number] | undefined;
  return engine.waitForSnapshot(
    (snapshot) => {
      const fingerprint = [
        snapshotValue(snapshot, "viewportFingerprintLow24"),
        snapshotValue(snapshot, "viewportFingerprintHigh24"),
      ] as const;
      const ready =
        snapshotValue(snapshot, "quads") > 0 &&
        snapshotValue(snapshot, "pendingJobs") === 0 &&
        snapshotValue(snapshot, "surfaceInFlight") === 0 &&
        snapshotValue(snapshot, "allLodsReady") === 1 &&
        snapshotValue(snapshot, "lodTransitionQuads") > 0;
      stable =
        ready &&
        previousFingerprint !== undefined &&
        fingerprint[0] === previousFingerprint[0] &&
        fingerprint[1] === previousFingerprint[1]
          ? stable + 1
          : 0;
      previousFingerprint = fingerprint;
      return stable >= 10;
    },
    {
      timeoutMs: 90_000,
      intervalMs: 24,
      description: "LOD fidelity fixture did not reach stable complete geometry",
    },
  );
}

async function captureHeading(
  context: ScenarioContext,
  page: Page,
  engine: EngineClient,
  profile: LodProfile,
  heading: number,
  pitch: number,
  headingIndex: number,
): Promise<HeadingCapture> {
  await engine.setCameraLook(heading, pitch, {
    timeoutMs: 10_000,
    intervalMs: 10,
    description: `${profile.id} heading did not settle`,
  });
  await waitForSettledGeometry(engine);
  await engine.setDiagnosticSky([255, 0, 255]);
  const diagnostic = await page.screenshot();
  const diagnosticName = `${profile.id}-heading-${headingIndex + 1}-diagnostic.png`;
  await context.artifacts.write(
    `${profile.id} heading ${headingIndex + 1} diagnostic`,
    diagnosticName,
    diagnostic,
    "image/png",
  );
  const sky = await analyzeDiagnosticSky(page, diagnostic, GROUND_ROI);
  await engine.setDiagnosticSky(null);
  const normal = await page.screenshot();
  await context.artifacts.write(
    `${profile.id} heading ${headingIndex + 1}`,
    `${profile.id}-heading-${headingIndex + 1}.png`,
    normal,
    "image/png",
  );
  return { heading, normal, diagnostic, sky };
}

async function captureProfile(
  context: ScenarioContext,
  page: Page,
  engine: EngineClient,
  arena: Arena,
  profile: LodProfile,
  measureMs: number,
): Promise<ProfileCapture> {
  context.log(`capturing ${profile.id}: ${profile.boundaries.join(",")}`);
  await engine.setLodBoundaryHalfExtents(profile.boundaries);
  await page.waitForTimeout(500);
  await waitForSettledGeometry(engine);
  const headings: HeadingCapture[] = [];
  for (const [index, offset] of arena.headingOffsets.entries()) {
    headings.push(
      await captureHeading(
        context,
        page,
        engine,
        profile,
        arena.look[0] + offset,
        arena.look[1],
        index,
      ),
    );
  }
  await engine.setCameraLook(arena.look[0], arena.look[1]);
  await waitForSettledGeometry(engine);
  const performance = summarizeRenderPhase(await sampleRenderSnapshots(engine, measureMs, 125));
  return { profile, headings, performance };
}

async function compareProfile(
  page: Page,
  capture: ProfileCapture,
  reference: ProfileCapture,
): Promise<readonly ComparedHeading[]> {
  return Promise.all(
    capture.headings.map(async (heading, index) => {
      const referenceHeading = reference.headings[index];
      if (referenceHeading === undefined || referenceHeading.heading !== heading.heading) {
        throw new Error(`${capture.profile.id} omitted registered reference heading ${index}`);
      }
      const [appearance, geometry] = await Promise.all([
        compareRenderedImages(page, heading.normal, referenceHeading.normal, {
          region: GEOMETRY_ROI,
          footprintPixels: 4,
        }),
        compareRenderedImages(page, heading.diagnostic, referenceHeading.diagnostic, {
          region: GEOMETRY_ROI,
          footprintPixels: 4,
          diagnosticGeometry: true,
        }),
      ]);
      return { heading: heading.heading, sky: heading.sky, appearance, geometry };
    }),
  );
}

async function captureViewportCompleteness(
  context: ScenarioContext,
  page: Page,
  engine: EngineClient,
  arena: Arena,
  profile: LodProfile,
): Promise<readonly ViewportCompletenessSample[]> {
  await engine.setLodBoundaryHalfExtents(profile.boundaries);
  await waitForSettledGeometry(engine);
  await engine.setDiagnosticSky([255, 0, 255]);
  const samples: ViewportCompletenessSample[] = [];
  for (const [index, offset] of arena.headingOffsets.entries()) {
    const heading = arena.look[0] + offset;
    // Turn a full peripheral field away before returning. The first screenshot at the registered
    // target must already contain everything that the settled frame contains; otherwise culling or
    // view-driven streaming still depends on staring at a tree or terrain region.
    await engine.setCameraLook(heading + 1.15, arena.look[1]);
    await waitForSettledGeometry(engine);
    const targetSnapshot = await engine.setCameraLook(heading, arena.look[1]);
    const firstPresentedSnapshot = await engine.waitForFrameAfter(
      snapshotValue(targetSnapshot, "frameSequence"),
      {
        timeoutMs: 2_000,
        intervalMs: 4,
        description: "peripheral target view was not presented",
      },
    );
    const immediate = await page.screenshot();
    const afterCaptureSnapshot = await engine.snapshot();
    const settledSnapshot = await waitForSettledGeometry(engine);
    const settled = await page.screenshot();
    const comparison = await compareRenderedImages(page, immediate, settled, {
      region: GEOMETRY_ROI,
      footprintPixels: 4,
      diagnosticGeometry: true,
    });
    await context.artifacts.write(
      `viewport completeness ${index + 1} immediate`,
      `viewport-completeness-${index + 1}-immediate.png`,
      immediate,
      "image/png",
    );
    await context.artifacts.write(
      `viewport completeness ${index + 1} settled`,
      `viewport-completeness-${index + 1}-settled.png`,
      settled,
      "image/png",
    );
    samples.push({
      heading,
      atTarget: viewportState(targetSnapshot),
      firstPresented: viewportState(firstPresentedSnapshot),
      afterCapture: viewportState(afterCaptureSnapshot),
      settled: viewportState(settledSnapshot),
      comparison,
    });
  }
  await engine.setDiagnosticSky(null);
  return samples;
}

function worstHeading(comparisons: readonly ComparedHeading[]) {
  const appearanceSsim = Math.min(...comparisons.map((entry) => entry.appearance.ssim));
  const appearanceLumaDelta = Math.max(
    ...comparisons.map((entry) => entry.appearance.meanAbsoluteLinearLumaDelta),
  );
  const maskDisagreementFraction = Math.max(
    ...comparisons.map((entry) => entry.geometry.diagnosticGeometry?.maskDisagreementFraction ?? 1),
  );
  const largestMaskComponentFraction = Math.max(
    ...comparisons.map(
      (entry) => entry.geometry.diagnosticGeometry?.largestDisagreementComponentFraction ?? 1,
    ),
  );
  // The candidate is the left image and uniform-150 reference is the right image. Keep this
  // asymmetric metric separate: a disappearing tree crown, wall, or overhang is more serious than
  // an equally sized silhouette shift that merely adds candidate geometry.
  const referenceGeometryMissingFraction = Math.max(
    ...comparisons.map(
      (entry) => entry.geometry.diagnosticGeometry?.rightOnlyOccupancyFraction ?? 1,
    ),
  );
  const largestMissingComponentFraction = Math.max(
    ...comparisons.map(
      (entry) => entry.geometry.diagnosticGeometry?.largestRightOnlyComponentFraction ?? 1,
    ),
  );
  const diagnosticSkyPixels = Math.max(
    ...comparisons.map((entry) => entry.sky.diagnosticSkyPixels),
  );
  return {
    appearanceSsim,
    appearanceLumaDelta,
    maskDisagreementFraction,
    largestMaskComponentFraction,
    referenceGeometryMissingFraction,
    largestMissingComponentFraction,
    diagnosticSkyPixels,
  };
}

function performanceMetrics(performance: RenderPhaseSummary) {
  return {
    frameP95Ms: performance.frameMs.p95,
    frameP99Ms: performance.frameMs.p99,
    framesAbove16_67Ms: performance.frameMs.above16_67ms,
    cpuP95Ms: performance.cpuMs.p95,
    worldGpuP95Ms: requiredGpu(performance.gpu.worldMs, "LOD profile world").p95,
    totalGpuP95Ms: requiredGpu(performance.gpu.totalMs, "LOD profile total").p95,
    quads: performance.quads,
    drawCalls: performance.drawCalls,
    visibleChunks: performance.visibleChunks,
    residentChunks: performance.residentChunks,
    meshArenaAllocatedMiB: performance.meshArenaAllocatedMiB,
  };
}

function uniformMarginals(
  results: readonly {
    readonly id: string;
    readonly family: LodProfile["family"];
    readonly worst: ReturnType<typeof worstHeading>;
    readonly performance: ReturnType<typeof performanceMetrics>;
  }[],
) {
  const uniform = results.filter((result) => result.family === "uniform");
  return uniform.slice(1).map((current, index) => {
    const previous = uniform[index];
    if (previous === undefined) throw new Error("uniform LOD chain omitted its previous profile");
    const ssimGain = current.worst.appearanceSsim - previous.worst.appearanceSsim;
    const maskGain =
      previous.worst.maskDisagreementFraction - current.worst.maskDisagreementFraction;
    const worldGpuCostMs = current.performance.worldGpuP95Ms - previous.performance.worldGpuP95Ms;
    return {
      from: previous.id,
      to: current.id,
      ssimGain,
      maskDisagreementReduction: maskGain,
      worldGpuCostMs,
      ssimGainPerWorldGpuMs: ssimGain / Math.max(worldGpuCostMs, 0.001),
    };
  });
}

async function runLodFidelity(context: ScenarioContext, arguments_: readonly string[]) {
  const options = parseOptions(arguments_);
  const referenceProfile = LOD_PROFILES.at(-1);
  if (referenceProfile === undefined)
    throw new Error("LOD fidelity sweep has no reference profile");
  const world = await startWorldStack(context, {
    fixture: {
      prefix: "voxels-lod-fidelity-",
      source: options.source,
      spawnVoxels: options.arena.spawn,
      spawnPillarHeightVoxels: options.arena.pillarHeight,
      spawnPillarRadiusVoxels: options.arena.pillarRadius,
      lodBoundaryHalfExtentsVoxels: referenceProfile.boundaries,
      dayLengthSeconds: 0,
      dayFractionAtUnixEpoch: 0.5,
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
    label: `lod-fidelity-${options.arena.id}`,
    viewport: options.viewport,
    deviceScaleFactor: options.deviceScaleFactor,
    ...world.clientRoute,
  });
  const { engine, page } = viewport;
  await engine.setCameraLook(options.arena.look[0], options.arena.look[1]);
  await waitForSettledGeometry(engine);

  // Capture the most detailed profile first. This proves its complete hierarchy is resident before
  // lower-cost ownership policies reuse the exact same world, camera, and browser process.
  const reference = await captureProfile(
    context,
    page,
    engine,
    options.arena,
    referenceProfile,
    options.measureMs,
  );
  const captures = [reference];
  for (const profile of LOD_PROFILES.slice(0, -1)) {
    captures.push(
      await captureProfile(context, page, engine, options.arena, profile, options.measureMs),
    );
  }

  const orderedCaptures = LOD_PROFILES.map((profile) => {
    const capture = captures.find((candidate) => candidate.profile.id === profile.id);
    if (capture === undefined) throw new Error(`LOD sweep omitted ${profile.id}`);
    return capture;
  });
  const results = [];
  for (const capture of orderedCaptures) {
    const headings = await compareProfile(page, capture, reference);
    results.push({
      id: capture.profile.id,
      family: capture.profile.family,
      boundaries: capture.profile.boundaries,
      headings,
      worst: worstHeading(headings),
      performance: performanceMetrics(capture.performance),
    });
  }
  const marginals = uniformMarginals(results);
  const viewportCompleteness = await captureViewportCompleteness(
    context,
    page,
    engine,
    options.arena,
    referenceProfile,
  );
  const worstViewportMissingPixels = Math.max(
    ...viewportCompleteness.map(
      ({ comparison }) => comparison.diagnosticGeometry?.rightOnlyOccupancyPixels ?? 1,
    ),
  );
  const worstViewportMissingComponentPixels = Math.max(
    ...viewportCompleteness.map(
      ({ comparison }) => comparison.diagnosticGeometry?.largestRightOnlyComponentPixels ?? 1,
    ),
  );
  const eligible = results.filter(
    (result) =>
      result.family === "uniform" &&
      result.worst.diagnosticSkyPixels === 0 &&
      result.worst.appearanceSsim >= 0.9995 &&
      result.worst.maskDisagreementFraction <= 0.00025 &&
      result.worst.referenceGeometryMissingFraction <= 0.0001 &&
      result.performance.frameP95Ms <= 12 &&
      result.performance.totalGpuP95Ms <= 7.5,
  );
  // The uniform results are ordered from cheapest ownership policy to most detailed. Select the
  // first one that clears strict visual and frame-headroom gates instead of sorting noisy, tightly
  // clustered GPU samples and accidentally recommending the largest policy.
  const recommendation = eligible[0];
  const violations: string[] = [];
  for (const result of results) {
    if (result.worst.diagnosticSkyPixels > 0) {
      violations.push(`${result.id} exposed diagnostic sky inside the terrain ROI`);
    }
    if (result.performance.frameP95Ms > 12) {
      violations.push(`${result.id} frame p95 exceeded 12ms`);
    }
    if (result.performance.totalGpuP95Ms > 7.5) {
      violations.push(`${result.id} total GPU p95 exceeded 7.5ms`);
    }
  }
  if (recommendation === undefined) {
    violations.push("no measured LOD profile met the fidelity and 120Hz headroom criteria");
  }
  if (worstViewportMissingPixels > 0) {
    violations.push(
      `first peripheral view omitted ${worstViewportMissingPixels} settled geometry pixels`,
    );
  }
  browser.assertHealthy();
  const report = {
    ok: violations.length === 0,
    commit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(),
    dirty: execFileSync("git", ["status", "--porcelain"], { encoding: "utf8" }).trim() !== "",
    source: options.source,
    arena: options.arena,
    browser: browser.version,
    framebuffer: {
      css: options.viewport,
      deviceScaleFactor: options.deviceScaleFactor,
      pixels: {
        width: options.viewport.width * options.deviceScaleFactor,
        height: options.viewport.height * options.deviceScaleFactor,
      },
    },
    referenceProfile: referenceProfile.id,
    recommendation: recommendation?.id ?? null,
    criteria: {
      minimumWorstHeadingSsim: 0.9995,
      maximumMaskDisagreementFraction: 0.00025,
      maximumReferenceGeometryMissingFraction: 0.0001,
      maximumFrameP95Ms: 12,
      maximumTotalGpuP95Ms: 7.5,
      requireZeroDiagnosticSkyPixelsInGroundRoi: true,
      requireZeroGeometryMissingFromFirstPeripheralView: true,
    },
    results,
    uniformMarginals: marginals,
    viewportCompleteness: {
      samples: viewportCompleteness,
      worstMissingPixels: worstViewportMissingPixels,
      worstMissingComponentPixels: worstViewportMissingComponentPixels,
    },
    violations,
  };
  await context.artifacts.writeJson("LOD fidelity report", "report.json", report);
  if (!report.ok) throw new Error(`LOD fidelity violations: ${violations.join(", ")}`);
  return {
    summary: `LOD fidelity sweep recommends ${recommendation?.id}.`,
    metrics: {
      recommendation: recommendation?.id ?? null,
      profiles: results.map((result) => ({
        id: result.id,
        ...result.worst,
        ...result.performance,
      })),
    },
    details: report,
  };
}

export default defineScenario({
  id: "lod-fidelity",
  kind: "benchmark",
  summary:
    "Sweeps nonlinear LOD boundaries against a registered high-detail reference for seams, silhouettes, fidelity, and 120Hz cost.",
  uses: {
    world: true,
    browser: true,
    viewport: "browser",
    screenshots: true,
    video: false,
    metrics: true,
    rust: true,
  },
  timeoutMs: 1_800_000,
  run: runLodFidelity,
});
