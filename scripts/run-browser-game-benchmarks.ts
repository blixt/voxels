import { join } from "node:path";

import type {
  BootstrapExperienceBenchmark,
  RouteExperienceBenchmark,
  RouteExperienceFrameSample,
} from "../src/client/game-controller.ts";
import type { BootstrapBenchmarkSample } from "../src/engine/game-bootstrap-benchmark.ts";
import {
  createBenchmarkOutputDir,
  readGitShortHead,
  resolveChromeBinary,
  runBrowserBenchmarkScenario,
  sanitizeFileStem,
  summarizeMemorySamples,
  timestampForFile,
  withBrowserGameSession,
  writeBenchmarkArtifacts,
  type BenchmarkArtifactPaths,
  type BenchmarkHarnessOptions,
  type BrowserGameSession,
  type BrowserBenchmarkIterationResult,
} from "./lib/browser-game-benchmark-harness.ts";

export {};

interface CliOptions {
  label: string | null;
  outputDir: string | null;
  chromeBinary: string;
  headless: boolean;
  appPort: number | null;
  viewportWidth: number;
  viewportHeight: number;
  skipBuild: boolean;
  startupWarmup: number;
  startupIterations: number;
  startupTimeoutMs: number;
  startupSampleIntervalMs: number;
  walkWarmup: number;
  walkIterations: number;
  walkTimeoutMs: number;
  walkSampleIntervalMs: number;
  walkDurationSeconds: number;
  walkSettleSeconds: number;
  walkSampleHz: number;
  walkSpeedMetersPerSecond: number;
  walkYawRadians: number;
  walkSeamProbeStrideFrames: number;
  lodPersistenceIterations: number;
  lodPersistenceTimeoutMs: number;
  lodPersistenceMaxFrames: number;
  lodPersistenceChunkDelta: number;
}

interface BrowserGameBenchmarksReport {
  generatedAt: string;
  label: string | null;
  commit: string | null;
  outputDir: string;
  appUrl: string;
  chromeBinary: string;
  build: BenchmarkHarnessOptions["skipBuild"] extends true ? null : unknown;
  options: Record<string, unknown>;
  scenarios: Record<string, {
    description: string;
    iterationCount: number;
    measuredIterationCount: number;
    csvPaths?: BenchmarkArtifactPaths | null;
    artifactPaths?: Record<string, string>;
    aggregate: Record<string, number | string | boolean | null>;
  }>;
}

interface LodPersistencePhaseSummary {
  label: string;
  frameCount: number;
  settled: boolean;
  elapsedMs: number;
  totalGenerated: number;
  totalMemoryCacheHits: number;
  totalEmptyCacheHits: number;
  totalDiskCacheHits: number;
  totalDiskCacheMisses: number;
  totalScheduledDiskRequests: number;
  totalScheduledDiskStores: number;
  totalCompletedDiskStores: number;
  totalDownsampleMs: number;
  totalMeshMs: number;
  totalGeneratedByLevel: readonly number[];
  totalMemoryCacheHitsByLevel: readonly number[];
  totalEmptyCacheHitsByLevel: readonly number[];
  totalDiskCacheHitsByLevel: readonly number[];
  maxLodChunkMs: number;
  maxWorstRecentFrameMs: number;
  maxRecentHitchCount: number;
  maxRecentDroppedFrameEstimate: number;
  finalPendingChunks: number;
  finalLodPendingChunks: number;
  finalLodPendingPlanning: number;
  finalLodPendingDiskCache: number;
  finalLodPendingDiskCacheByLevel: readonly number[];
  finalLodPendingGenerationBudget: number;
  finalLodPendingGenerationBudgetByLevel: readonly number[];
  finalLodPendingPartialBuild: number;
  finalLodPendingPartialBuildByLevel: readonly number[];
  finalLodPendingPrepared: number;
  finalLodPendingPreparedByLevel: readonly number[];
  finalLodPendingInvalidatedEviction: number;
  finalLodChunkCount: number;
  finalLodChunkCountByLevel: readonly number[];
  finalLodDrawCalls: number;
  finalLodDrawCallsByLevel: readonly number[];
  finalLodGeneratedByLevel: readonly number[];
  finalLodCacheHitsByLevel: readonly number[];
  finalLodEmptyCacheHitsByLevel: readonly number[];
  finalLodDiskCacheHitsByLevel: readonly number[];
  finalCumulativeGenerated: number;
  finalCumulativeDiskCacheHits: number;
  finalCumulativeDiskCacheMisses: number;
  finalCumulativeScheduledDiskRequests: number;
  finalCumulativeScheduledDiskStores: number;
  finalCumulativeCompletedDiskStores: number;
  finalCoverage: {
    sampleCount: number;
    uncoveredGapCount: number;
    handoffHoleCount: number;
    residentOverlapCount: number;
    bandOverlapCount: number;
    waterOverlapCount: number;
    maxSeamGapMeters: number;
    maxLodOverlapMeters: number;
  };
  lastHitchCause: string;
  lastHitchWallMs: number;
}

interface LodPersistenceIteration {
  iteration: number;
  originPosition: readonly number[];
  farPosition: readonly number[];
  coldOrigin: LodPersistencePhaseSummary;
  farEviction: LodPersistencePhaseSummary;
  storeFlush: LodPersistencePhaseSummary;
  reloadOrigin: LodPersistencePhaseSummary;
  pass: boolean;
  failures: string[];
}

interface LodPersistenceScenarioResult {
  scenarioId: "lod-idb-persistence-reload";
  description: string;
  iterations: LodPersistenceIteration[];
  aggregate: Record<string, number | string | boolean | null>;
  artifactPath: string;
}

const options = parseCli(Bun.argv);
const runStamp = timestampForFile(new Date());
const runName = `${runStamp}${options.label ? `-${sanitizeFileStem(options.label)}` : ""}`;
const baseOutputDir = await createBenchmarkOutputDir(options.outputDir, "voxels-browser-game-bench");
const outputDir = options.outputDir ? join(baseOutputDir, runName) : baseOutputDir;
await Bun.$`mkdir -p ${outputDir}`.quiet();

await withBrowserGameSession({
  chromeBinary: options.chromeBinary,
  headless: options.headless,
  appPort: options.appPort,
  viewportWidth: options.viewportWidth,
  viewportHeight: options.viewportHeight,
  skipBuild: options.skipBuild,
  outputDir,
}, async (session) => {
  const scenarioReports: BrowserGameBenchmarksReport["scenarios"] = {};

  let startupResults: BrowserBenchmarkIterationResult<BootstrapExperienceBenchmark>[] = [];
  let startupArtifacts: BenchmarkArtifactPaths | null = null;
  if (options.startupWarmup + options.startupIterations > 0) {
    startupResults = await runBrowserBenchmarkScenario(session, {
      id: "startup-entry",
      description: "Cold-start bootstrap until the player can enter the world",
      warmupIterations: options.startupWarmup,
      measuredIterations: options.startupIterations,
      timeoutMs: options.startupTimeoutMs,
      sampleIntervalMs: options.startupSampleIntervalMs,
      benchmarkStartsDuringPrepare: true,
      async prepareIteration(benchmarkSession) {
        await benchmarkSession.navigateToGame({
          clearStorage: true,
          query: {
            benchmarkBootstrap: 1,
          },
        });
      },
      async readIteration(benchmarkSession) {
        const state = await benchmarkSession.evaluate<{
          benchmark: BootstrapExperienceBenchmark;
          ready: boolean;
        } | null>(`
          (() => {
            const game = window.__VOXELS_GAME__;
            if (!game || typeof game.getBootstrapBenchmark !== "function") {
              return null;
            }
            const benchmark = game.getBootstrapBenchmark();
            const snapshot = game.snapshot();
            const ready = snapshot.chunkCount > 0
              && snapshot.bootstrapPlayableReady === true
              && benchmark.summary.playableReadyElapsedMs !== null;
            return { benchmark, ready };
          })()
        `);
        if (!state || !state.ready) {
          return { done: false, result: null };
        }
        return { done: true, result: state.benchmark };
      },
    });

    startupArtifacts = await writeBenchmarkArtifacts(
      outputDir,
      "startup-entry",
      startupResults,
      {
        buildIterationRow(iteration, memory) {
          const drops = countFrameDrops(iteration.result.samples.map((sample) => sample.gameplayFrameMs));
          const { maxLodDrawCallsByLevel, ...summary } = iteration.result.summary;
          return {
            scenarioId: iteration.scenarioId,
            warmup: iteration.warmup,
            iteration: iteration.iteration,
            globalIndex: iteration.globalIndex,
            setupElapsedMs: iteration.setupElapsedMs,
            benchmarkElapsedMs: iteration.benchmarkElapsedMs,
            ...summary,
            maxLodDrawCallsByLevel: maxLodDrawCallsByLevel.join("/"),
            framesOver16_67Ms: drops.framesOver16_67Ms,
            framesOver33_33Ms: drops.framesOver33_33Ms,
            framesOver50Ms: drops.framesOver50Ms,
            ...memory,
          };
        },
        buildSampleRows(iteration) {
          return iteration.result.samples.map((sample) => serializeBootstrapSample(iteration, sample));
        },
        buildReport(iterationRows) {
          return aggregateIterationRows(iterationRows, [
            "benchmarkElapsedMs",
            "playableReadyElapsedMs",
            "visualReadyElapsedMs",
            "totalGeneratedChunks",
            "totalGameplayFrameMs",
            "totalStreamMs",
            "totalMeshMs",
            "peakGenerationWorkerCount",
            "peakJsHeapUsedSizeBytes",
            "peakRuntimeHeapUsedBytes",
            "deltaTaskDurationMs",
            "framesOver16_67Ms",
            "framesOver33_33Ms",
          ]);
        },
      },
    );

    scenarioReports["startup-entry"] = {
      description: "Cold-start bootstrap until the player can enter the world",
      iterationCount: startupResults.length,
      measuredIterationCount: countMeasuredIterations(startupResults),
      csvPaths: startupArtifacts,
      aggregate: aggregateIterationRows(
        startupResults.map((iteration) => buildStartupAggregateRow(iteration)),
        [
          "benchmarkElapsedMs",
          "playableReadyElapsedMs",
          "visualReadyElapsedMs",
          "totalGeneratedChunks",
          "peakGenerationWorkerCount",
          "peakJsHeapUsedSizeBytes",
          "peakRuntimeHeapUsedBytes",
          "deltaTaskDurationMs",
        ],
      ),
    };
  }

  let walkResults: BrowserBenchmarkIterationResult<RouteExperienceBenchmark>[] = [];
  let walkArtifacts: BenchmarkArtifactPaths | null = null;
  if (options.walkWarmup + options.walkIterations > 0) {
    const walkTokens = new Map<number, string>();
    walkResults = await runBrowserBenchmarkScenario(session, {
      id: "forward-walk-10s",
      description: "Fresh-load real-time 10-second held-forward walk through streaming terrain",
      warmupIterations: options.walkWarmup,
      measuredIterations: options.walkIterations,
      timeoutMs: options.walkTimeoutMs,
      sampleIntervalMs: options.walkSampleIntervalMs,
      async prepareIteration(benchmarkSession, run) {
        await benchmarkSession.navigateToGame({ clearStorage: true });
        await benchmarkSession.waitForBootstrapBenchmarkComplete(options.startupTimeoutMs);
        await benchmarkSession.waitForGameReady(options.startupTimeoutMs);
        const token = await benchmarkSession.startAsyncWindowBenchmark(
          `window.__VOXELS_GAME__.benchmarkLiveForwardWalkExperience(${JSON.stringify({
            durationSeconds: options.walkDurationSeconds,
            settleSeconds: options.walkSettleSeconds,
            sampleHz: options.walkSampleHz,
            speedMetersPerSecond: options.walkSpeedMetersPerSecond,
            yawRadians: options.walkYawRadians,
            seamProbeStrideFrames: options.walkSeamProbeStrideFrames,
            captureStrideFrames: 999999,
            referenceDiffStrideFrames: 0,
            referenceDiffLimit: 0,
          })})`,
        );
        walkTokens.set(run.globalIndex, token);
      },
      async readIteration(benchmarkSession, run) {
        const token = walkTokens.get(run.globalIndex);
        if (!token) {
          throw new Error(`Missing forward-walk benchmark token for iteration ${run.globalIndex}`);
        }
        const state = await benchmarkSession.getAsyncWindowBenchmarkState<RouteExperienceBenchmark>(token);
        if (!state || state.status === "running") {
          return { done: false, result: null };
        }
        if (state.status === "failed") {
          throw new Error(state.error ?? `Forward-walk benchmark ${token} failed`);
        }
        return { done: true, result: state.result };
      },
    });

    walkArtifacts = await writeBenchmarkArtifacts(
      outputDir,
      "forward-walk-10s",
      walkResults,
      {
        buildIterationRow(iteration, memory) {
          const drops = countFrameDrops(iteration.result.samples.map((sample) => sample.gameplayFrameMs));
          return {
            scenarioId: iteration.scenarioId,
            warmup: iteration.warmup,
            iteration: iteration.iteration,
            globalIndex: iteration.globalIndex,
            setupElapsedMs: iteration.setupElapsedMs,
            benchmarkElapsedMs: iteration.benchmarkElapsedMs,
            seed: iteration.result.seed,
            radiusChunks: iteration.result.radiusChunks,
            durationSeconds: iteration.result.durationSeconds,
            settleSeconds: iteration.result.settleSeconds,
            ...iteration.result.summary,
            framesOver16_67Ms: drops.framesOver16_67Ms,
            framesOver33_33Ms: drops.framesOver33_33Ms,
            framesOver50Ms: drops.framesOver50Ms,
            ...memory,
          };
        },
        buildSampleRows(iteration) {
          return iteration.result.samples.map((sample) => serializeRouteSample(iteration, sample));
        },
        buildReport(iterationRows) {
          return aggregateIterationRows(iterationRows, [
            "benchmarkElapsedMs",
            "avgGameplayFrameMs",
            "p95GameplayFrameMs",
            "maxGameplayFrameMs",
            "avgMeasuredWorkMs",
            "p95MeasuredWorkMs",
            "maxMeasuredWorkMs",
            "avgStreamMs",
            "avgMeshMs",
            "framesWithHoleSignals",
            "framesWithVisibleGroundGaps",
            "framesOver16_67Ms",
            "framesOver33_33Ms",
            "peakGenerationWorkerCount",
            "peakJsHeapUsedSizeBytes",
            "peakRuntimeHeapUsedBytes",
            "deltaTaskDurationMs",
          ]);
        },
      },
    );

    scenarioReports["forward-walk-10s"] = {
      description: "Fresh-load deterministic 10-second forward walk through streaming terrain",
      iterationCount: walkResults.length,
      measuredIterationCount: countMeasuredIterations(walkResults),
      csvPaths: walkArtifacts,
      aggregate: aggregateIterationRows(
        walkResults.map((iteration) => buildWalkAggregateRow(iteration)),
        [
          "benchmarkElapsedMs",
          "avgGameplayFrameMs",
          "p95GameplayFrameMs",
          "maxGameplayFrameMs",
          "framesWithHoleSignals",
          "framesOver16_67Ms",
          "framesOver33_33Ms",
          "peakGenerationWorkerCount",
          "peakJsHeapUsedSizeBytes",
          "peakRuntimeHeapUsedBytes",
          "deltaTaskDurationMs",
        ],
      ),
    };
  }

  let lodPersistenceResult: LodPersistenceScenarioResult | null = null;
  if (options.lodPersistenceIterations > 0) {
    lodPersistenceResult = await runLodPersistenceScenario(session, options, outputDir);
    scenarioReports[lodPersistenceResult.scenarioId] = {
      description: lodPersistenceResult.description,
      iterationCount: lodPersistenceResult.iterations.length,
      measuredIterationCount: lodPersistenceResult.iterations.length,
      artifactPaths: {
        reportJsonPath: lodPersistenceResult.artifactPath,
      },
      aggregate: lodPersistenceResult.aggregate,
    };
  }

  const reportPath = join(outputDir, "report.json");
  const report: BrowserGameBenchmarksReport = {
    generatedAt: new Date().toISOString(),
    label: options.label,
    commit: readGitShortHead(),
    outputDir,
    appUrl: session.appUrl,
    chromeBinary: options.chromeBinary,
    build: session.build,
    options: {
      startupWarmup: options.startupWarmup,
      startupIterations: options.startupIterations,
      startupTimeoutMs: options.startupTimeoutMs,
      startupSampleIntervalMs: options.startupSampleIntervalMs,
      walkWarmup: options.walkWarmup,
      walkIterations: options.walkIterations,
      walkTimeoutMs: options.walkTimeoutMs,
      walkSampleIntervalMs: options.walkSampleIntervalMs,
      walkDurationSeconds: options.walkDurationSeconds,
      walkSettleSeconds: options.walkSettleSeconds,
      walkSampleHz: options.walkSampleHz,
      walkSpeedMetersPerSecond: options.walkSpeedMetersPerSecond,
      walkYawRadians: options.walkYawRadians,
      walkSeamProbeStrideFrames: options.walkSeamProbeStrideFrames,
      lodPersistenceIterations: options.lodPersistenceIterations,
      lodPersistenceTimeoutMs: options.lodPersistenceTimeoutMs,
      lodPersistenceMaxFrames: options.lodPersistenceMaxFrames,
      lodPersistenceChunkDelta: options.lodPersistenceChunkDelta,
      viewportWidth: options.viewportWidth,
      viewportHeight: options.viewportHeight,
      headless: options.headless,
      skipBuild: options.skipBuild,
    },
    scenarios: scenarioReports,
  };
  await Bun.write(reportPath, `${JSON.stringify(report, null, 2)}\n`);

  printSummary(outputDir, reportPath, startupArtifacts, walkArtifacts, startupResults, walkResults, lodPersistenceResult);
  if (lodPersistenceResult && lodPersistenceResult.aggregate.pass !== true) {
    process.exitCode = 1;
  }
});

function parseCli(argv: readonly string[]): CliOptions {
  const args = argv.slice(2);
  return {
    label: readFlag(args, "--label"),
    outputDir: readFlag(args, "--output-dir"),
    chromeBinary: readFlag(args, "--chrome-binary") ?? resolveChromeBinary(),
    headless: readBooleanFlag(args, "--headless", true),
    appPort: readOptionalPositiveInt(readFlag(args, "--port")),
    viewportWidth: readPositiveInt(readFlag(args, "--width"), 1440),
    viewportHeight: readPositiveInt(readFlag(args, "--height"), 900),
    skipBuild: readBooleanFlag(args, "--skip-build", false),
    startupWarmup: readNonNegativeInt(readFlag(args, "--startup-warmup"), 1),
    startupIterations: readNonNegativeInt(readFlag(args, "--startup-iterations"), 3),
    startupTimeoutMs: readPositiveInt(readFlag(args, "--startup-timeout-ms"), 180_000),
    startupSampleIntervalMs: readPositiveInt(readFlag(args, "--startup-sample-interval-ms"), 100),
    walkWarmup: readNonNegativeInt(readFlag(args, "--walk-warmup"), 1),
    walkIterations: readNonNegativeInt(readFlag(args, "--walk-iterations"), 3),
    walkTimeoutMs: readPositiveInt(readFlag(args, "--walk-timeout-ms"), 120_000),
    walkSampleIntervalMs: readPositiveInt(readFlag(args, "--walk-sample-interval-ms"), 100),
    walkDurationSeconds: readPositiveFloat(readFlag(args, "--walk-duration"), 10),
    walkSettleSeconds: readPositiveFloat(readFlag(args, "--walk-settle"), 4),
    walkSampleHz: readPositiveInt(readFlag(args, "--walk-sample-hz"), 60),
    walkSpeedMetersPerSecond: readPositiveFloat(readFlag(args, "--walk-speed"), 4.6),
    walkYawRadians: readFloat(readFlag(args, "--walk-yaw"), 0),
    walkSeamProbeStrideFrames: readPositiveInt(readFlag(args, "--walk-seam-stride"), 15),
    lodPersistenceIterations: readNonNegativeInt(readFlag(args, "--lod-persistence-iterations"), 0),
    lodPersistenceTimeoutMs: readPositiveInt(readFlag(args, "--lod-persistence-timeout-ms"), 180_000),
    lodPersistenceMaxFrames: readPositiveInt(readFlag(args, "--lod-persistence-max-frames"), 240),
    lodPersistenceChunkDelta: readNonNegativeInt(readFlag(args, "--lod-persistence-chunk-delta"), 0),
  };
}

async function runLodPersistenceScenario(
  session: BrowserGameSession,
  options: CliOptions,
  outputDir: string,
): Promise<LodPersistenceScenarioResult> {
  const scenarioId = "lod-idb-persistence-reload";
  const description = "Derived LOD IndexedDB persistence across reload after JS memory is cleared";
  const iterations: LodPersistenceIteration[] = [];
  for (let iteration = 1; iteration <= options.lodPersistenceIterations; iteration += 1) {
    console.log(`[${scenarioId}] starting measured iteration ${iteration}/${options.lodPersistenceIterations}`);
    await session.navigateToGame({
      clearStorage: true,
      query: { benchmarkBootstrap: 1 },
    });
    await session.waitForBootstrapBenchmarkComplete(options.lodPersistenceTimeoutMs);
    await session.waitForGameReady(options.lodPersistenceTimeoutMs);
    const populate = await session.evaluate<Omit<LodPersistenceIteration, "iteration" | "reloadOrigin" | "pass" | "failures">>(
      buildLodPersistencePopulateExpression({
        maxFrames: options.lodPersistenceMaxFrames,
        chunkDelta: options.lodPersistenceChunkDelta,
      }),
    );

    await session.navigateToGame({
      clearStorage: false,
    });
    await session.waitForGameReady(options.lodPersistenceTimeoutMs);
    const reloadOrigin = await session.evaluate<LodPersistencePhaseSummary>(
      buildLodPersistenceReloadExpression({
        maxFrames: options.lodPersistenceMaxFrames,
        originPosition: populate.originPosition,
      }),
    );
    const failures = validateLodPersistenceIteration({
      iteration,
      ...populate,
      reloadOrigin,
      pass: true,
      failures: [],
    });
    iterations.push({
      iteration,
      ...populate,
      reloadOrigin,
      pass: failures.length === 0,
      failures,
    });
  }

  const artifactPath = join(outputDir, `${scenarioId}.json`);
  const aggregate = aggregateLodPersistenceIterations(iterations);
  const result: LodPersistenceScenarioResult = {
    scenarioId,
    description,
    iterations,
    aggregate,
    artifactPath,
  };
  await Bun.write(artifactPath, `${JSON.stringify(result, null, 2)}\n`);
  return result;
}

function buildLodPersistencePopulateExpression(options: {
  maxFrames: number;
  chunkDelta: number;
}): string {
  return `(async () => {
    ${buildLodPersistencePageHarnessSource()}
    return await lodPersistencePopulatePageProbe(${JSON.stringify(options)});
  })()`;
}

function buildLodPersistenceReloadExpression(options: {
  maxFrames: number;
  originPosition: readonly number[];
}): string {
  return `(async () => {
    ${buildLodPersistencePageHarnessSource()}
    return await lodPersistenceReloadPageProbe(${JSON.stringify(options)});
  })()`;
}

function buildLodPersistencePageHarnessSource(): string {
  return [
    lodPersistencePopulatePageProbe,
    lodPersistenceReloadPageProbe,
    driveLodPersistencePhase,
    pumpLodPersistencePhase,
    buildLodPersistencePhaseSummary,
    buildCurrentLodPersistencePhaseSummary,
    isLodPersistenceSettled,
    summarizeLodCoverageForPersistence,
  ].map((fn) => fn.toString()).join("\n");
}

async function lodPersistencePopulatePageProbe(rawOptions: {
  maxFrames: number;
  chunkDelta: number;
}): Promise<Omit<LodPersistenceIteration, "iteration" | "reloadOrigin" | "pass" | "failures">> {
  const game = window.__VOXELS_GAME__;
  if (!game) {
    throw new Error("window.__VOXELS_GAME__ is not available");
  }
  const controller = game.controller;
  controller.stop();
  try {
    game.setViewDistance(1);
    const originSnapshot = game.snapshot();
    const originPosition = [...originSnapshot.position];
    const chunkSize = Math.max(1, controller.world.chunkSize);
    const chunkDelta = Math.max(0, Math.floor(rawOptions.chunkDelta));
    const farPosition = [
      originPosition[0] + chunkSize * chunkDelta,
      originPosition[1],
      originPosition[2] + chunkSize * chunkDelta,
    ];
    const maxFrames = Math.max(1, Math.floor(rawOptions.maxFrames));
    const coldOrigin = await driveLodPersistencePhase(game, "cold-origin", originPosition, maxFrames, () => false);
    const farEviction = chunkDelta > 0
      ? await driveLodPersistencePhase(game, "far-eviction", farPosition, maxFrames, () => false)
      : buildCurrentLodPersistencePhaseSummary(game, "far-eviction-skipped");
    const storeFlush = await pumpLodPersistencePhase(game, "store-flush", maxFrames, (phase) =>
      phase.totalCompletedDiskStores > 0 || phase.totalScheduledDiskStores > 0,
    );
    return {
      originPosition,
      farPosition,
      coldOrigin,
      farEviction,
      storeFlush,
    };
  } finally {
    controller.start();
  }
}

async function lodPersistenceReloadPageProbe(rawOptions: {
  maxFrames: number;
  originPosition: readonly number[];
}): Promise<LodPersistencePhaseSummary> {
  const game = window.__VOXELS_GAME__;
  if (!game) {
    throw new Error("window.__VOXELS_GAME__ is not available");
  }
  const controller = game.controller;
  controller.stop();
  try {
    game.setViewDistance(1);
    const origin = rawOptions.originPosition;
    return await driveLodPersistencePhase(
      game,
      "reload-origin",
      [Number(origin[0]), Number(origin[1]), Number(origin[2])],
      Math.max(1, Math.floor(rawOptions.maxFrames)),
      (phase) => phase.totalDiskCacheHits > 0,
    );
  } finally {
    controller.start();
  }
}

async function driveLodPersistencePhase(
  game: NonNullable<Window["__VOXELS_GAME__"]>,
  label: string,
  position: readonly number[],
  maxFrames: number,
  earlyExit: (phase: LodPersistencePhaseSummary) => boolean,
): Promise<LodPersistencePhaseSummary> {
  return await pumpLodPersistencePhase(game, label, maxFrames, earlyExit, [
    Number(position[0]),
    Number(position[1]),
    Number(position[2]),
  ]);
}

async function pumpLodPersistencePhase(
  game: NonNullable<Window["__VOXELS_GAME__"]>,
  label: string,
  maxFrames: number,
  earlyExit: (phase: LodPersistencePhaseSummary) => boolean,
  position?: readonly number[],
): Promise<LodPersistencePhaseSummary> {
  const startedAt = performance.now();
  let frameCount = 0;
  let totalGenerated = 0;
  let totalMemoryCacheHits = 0;
  let totalEmptyCacheHits = 0;
  let totalDiskCacheHits = 0;
  let totalDiskCacheMisses = 0;
  let totalScheduledDiskRequests = 0;
  let totalScheduledDiskStores = 0;
  let totalCompletedDiskStores = 0;
  let totalDownsampleMs = 0;
  let totalMeshMs = 0;
  const totalGeneratedByLevel = [0, 0, 0, 0, 0];
  const totalMemoryCacheHitsByLevel = [0, 0, 0, 0, 0];
  const totalEmptyCacheHitsByLevel = [0, 0, 0, 0, 0];
  const totalDiskCacheHitsByLevel = [0, 0, 0, 0, 0];
  let maxLodChunkMs = 0;
  let maxWorstRecentFrameMs = 0;
  let maxRecentHitchCount = 0;
  let maxRecentDroppedFrameEstimate = 0;
  let finalSnapshot = game.snapshot();
  const batchFrames = 4;
  const addLevelCounts = (target: number[], source: readonly number[]): void => {
    for (let index = 0; index < target.length; index += 1) {
      target[index] += source[index] ?? 0;
    }
  };

  while (frameCount < maxFrames) {
    const remainingFrames = maxFrames - frameCount;
    const pump = await game.pumpWorldForBenchmark(
      position && frameCount === 0 ? [Number(position[0]), Number(position[1]), Number(position[2])] : undefined,
      {
        maxFrames: Math.min(batchFrames, remainingFrames),
        maxGenerateLodChunks: 12,
        maxLodPlanMs: 12,
        maxLodWorkMs: 12,
        maxEvictChunks: 48,
        maxMeshRebuilds: 4,
        stopWhenSettled: false,
      },
    );
    frameCount += pump.frameCount;
    finalSnapshot = pump.finalSnapshot;
    totalGenerated += pump.totalGenerated;
    totalMemoryCacheHits += pump.totalMemoryCacheHits;
    totalEmptyCacheHits += pump.totalEmptyCacheHits;
    totalDiskCacheHits += pump.totalDiskCacheHits;
    totalDiskCacheMisses += pump.totalDiskCacheMisses;
    totalScheduledDiskRequests += pump.totalScheduledDiskRequests;
    totalScheduledDiskStores += pump.totalScheduledDiskStores;
    totalCompletedDiskStores += pump.totalCompletedDiskStores;
    totalDownsampleMs += pump.totalDownsampleMs;
    totalMeshMs += pump.totalMeshMs;
    addLevelCounts(totalGeneratedByLevel, pump.totalGeneratedByLevel);
    addLevelCounts(totalMemoryCacheHitsByLevel, pump.totalMemoryCacheHitsByLevel);
    addLevelCounts(totalEmptyCacheHitsByLevel, pump.totalEmptyCacheHitsByLevel);
    addLevelCounts(totalDiskCacheHitsByLevel, pump.totalDiskCacheHitsByLevel);
    maxLodChunkMs = Math.max(maxLodChunkMs, pump.maxLodChunkMs);
    maxWorstRecentFrameMs = Math.max(maxWorstRecentFrameMs, pump.maxWorstRecentFrameMs);
    maxRecentHitchCount = Math.max(maxRecentHitchCount, pump.maxRecentHitchCount);
    maxRecentDroppedFrameEstimate = Math.max(maxRecentDroppedFrameEstimate, pump.maxRecentDroppedFrameEstimate);
    const partial = buildLodPersistencePhaseSummary({
      label,
      frameCount,
      settled: pump.settled,
      elapsedMs: performance.now() - startedAt,
      totalGenerated,
      totalMemoryCacheHits,
      totalEmptyCacheHits,
      totalDiskCacheHits,
      totalDiskCacheMisses,
      totalScheduledDiskRequests,
      totalScheduledDiskStores,
      totalCompletedDiskStores,
      totalDownsampleMs,
      totalMeshMs,
      totalGeneratedByLevel,
      totalMemoryCacheHitsByLevel,
      totalEmptyCacheHitsByLevel,
      totalDiskCacheHitsByLevel,
      maxLodChunkMs,
      maxWorstRecentFrameMs,
      maxRecentHitchCount,
      maxRecentDroppedFrameEstimate,
      finalSnapshot,
      finalCoverage: summarizeLodCoverageForPersistence(game),
    });
    if (partial.settled && earlyExit(partial)) {
      return partial;
    }
    if (partial.settled && frameCount >= 3 && !earlyExit(partial)) {
      if (label !== "store-flush" && label !== "reload-origin") {
        return partial;
      }
    }
  }

  return buildLodPersistencePhaseSummary({
    label,
    frameCount,
    settled: isLodPersistenceSettled(finalSnapshot),
    elapsedMs: performance.now() - startedAt,
    totalGenerated,
    totalMemoryCacheHits,
    totalEmptyCacheHits,
    totalDiskCacheHits,
    totalDiskCacheMisses,
    totalScheduledDiskRequests,
    totalScheduledDiskStores,
    totalCompletedDiskStores,
    totalDownsampleMs,
    totalMeshMs,
    totalGeneratedByLevel,
    totalMemoryCacheHitsByLevel,
    totalEmptyCacheHitsByLevel,
    totalDiskCacheHitsByLevel,
    maxLodChunkMs,
    maxWorstRecentFrameMs,
    maxRecentHitchCount,
    maxRecentDroppedFrameEstimate,
    finalSnapshot,
    finalCoverage: summarizeLodCoverageForPersistence(game),
  });
}

function buildLodPersistencePhaseSummary(input: {
  label: string;
  frameCount: number;
  settled: boolean;
  elapsedMs: number;
  totalGenerated: number;
  totalMemoryCacheHits: number;
  totalEmptyCacheHits: number;
  totalDiskCacheHits: number;
  totalDiskCacheMisses: number;
  totalScheduledDiskRequests: number;
  totalScheduledDiskStores: number;
  totalCompletedDiskStores: number;
  totalDownsampleMs: number;
  totalMeshMs: number;
  totalGeneratedByLevel: readonly number[];
  totalMemoryCacheHitsByLevel: readonly number[];
  totalEmptyCacheHitsByLevel: readonly number[];
  totalDiskCacheHitsByLevel: readonly number[];
  maxLodChunkMs: number;
  maxWorstRecentFrameMs: number;
  maxRecentHitchCount: number;
  maxRecentDroppedFrameEstimate: number;
  finalSnapshot: ReturnType<NonNullable<Window["__VOXELS_GAME__"]>["snapshot"]>;
  finalCoverage: LodPersistencePhaseSummary["finalCoverage"];
}): LodPersistencePhaseSummary {
  return {
    label: input.label,
    frameCount: input.frameCount,
    settled: input.settled,
    elapsedMs: input.elapsedMs,
    totalGenerated: input.totalGenerated,
    totalMemoryCacheHits: input.totalMemoryCacheHits,
    totalEmptyCacheHits: input.totalEmptyCacheHits,
    totalDiskCacheHits: input.totalDiskCacheHits,
    totalDiskCacheMisses: input.totalDiskCacheMisses,
    totalScheduledDiskRequests: input.totalScheduledDiskRequests,
    totalScheduledDiskStores: input.totalScheduledDiskStores,
    totalCompletedDiskStores: input.totalCompletedDiskStores,
    totalDownsampleMs: input.totalDownsampleMs,
    totalMeshMs: input.totalMeshMs,
    totalGeneratedByLevel: [...input.totalGeneratedByLevel],
    totalMemoryCacheHitsByLevel: [...input.totalMemoryCacheHitsByLevel],
    totalEmptyCacheHitsByLevel: [...input.totalEmptyCacheHitsByLevel],
    totalDiskCacheHitsByLevel: [...input.totalDiskCacheHitsByLevel],
    maxLodChunkMs: input.maxLodChunkMs,
    maxWorstRecentFrameMs: input.maxWorstRecentFrameMs,
    maxRecentHitchCount: input.maxRecentHitchCount,
    maxRecentDroppedFrameEstimate: input.maxRecentDroppedFrameEstimate,
    finalPendingChunks: input.finalSnapshot.streamPendingChunks,
    finalLodPendingChunks: input.finalSnapshot.lodPendingChunks,
    finalLodPendingPlanning: input.finalSnapshot.lodPendingPlanning,
    finalLodPendingDiskCache: input.finalSnapshot.lodPendingDiskCache,
    finalLodPendingDiskCacheByLevel: [...input.finalSnapshot.lodPendingDiskCacheByLevel],
    finalLodPendingGenerationBudget: input.finalSnapshot.lodPendingGenerationBudget,
    finalLodPendingGenerationBudgetByLevel: [...input.finalSnapshot.lodPendingGenerationBudgetByLevel],
    finalLodPendingPartialBuild: input.finalSnapshot.lodPendingPartialBuild,
    finalLodPendingPartialBuildByLevel: [...input.finalSnapshot.lodPendingPartialBuildByLevel],
    finalLodPendingPrepared: input.finalSnapshot.lodPendingPrepared,
    finalLodPendingPreparedByLevel: [...input.finalSnapshot.lodPendingPreparedByLevel],
    finalLodPendingInvalidatedEviction: input.finalSnapshot.lodPendingInvalidatedEviction,
    finalLodChunkCount: input.finalSnapshot.lodChunkCount,
    finalLodChunkCountByLevel: [...input.finalSnapshot.lodChunkCountByLevel],
    finalLodDrawCalls: input.finalSnapshot.lodDrawCalls,
    finalLodDrawCallsByLevel: [...input.finalSnapshot.lodDrawCallsByLevel],
    finalLodGeneratedByLevel: [...input.finalSnapshot.lodGeneratedChunksByLevel],
    finalLodCacheHitsByLevel: [...input.finalSnapshot.lodCacheHitsByLevel],
    finalLodEmptyCacheHitsByLevel: [...input.finalSnapshot.lodEmptyCacheHitsByLevel],
    finalLodDiskCacheHitsByLevel: [...input.finalSnapshot.lodDiskCacheHitsByLevel],
    finalCumulativeGenerated: input.finalSnapshot.cumulativeLodGeneratedChunks,
    finalCumulativeDiskCacheHits: input.finalSnapshot.cumulativeLodDiskCacheHits,
    finalCumulativeDiskCacheMisses: input.finalSnapshot.cumulativeLodDiskCacheMisses,
    finalCumulativeScheduledDiskRequests: input.finalSnapshot.cumulativeLodScheduledDiskRequests,
    finalCumulativeScheduledDiskStores: input.finalSnapshot.cumulativeLodScheduledDiskStores,
    finalCumulativeCompletedDiskStores: input.finalSnapshot.cumulativeLodCompletedDiskStores,
    finalCoverage: input.finalCoverage,
    lastHitchCause: input.finalSnapshot.lastHitchAttribution.cause,
    lastHitchWallMs: input.finalSnapshot.lastHitchAttribution.wallMs,
  };
}

function buildCurrentLodPersistencePhaseSummary(
  game: NonNullable<Window["__VOXELS_GAME__"]>,
  label: string,
): LodPersistencePhaseSummary {
  const snapshot = game.snapshot();
  return buildLodPersistencePhaseSummary({
    label,
    frameCount: 0,
    settled: isLodPersistenceSettled(snapshot),
    elapsedMs: 0,
    totalGenerated: 0,
    totalMemoryCacheHits: 0,
    totalEmptyCacheHits: 0,
    totalDiskCacheHits: 0,
    totalDiskCacheMisses: 0,
    totalScheduledDiskRequests: 0,
    totalScheduledDiskStores: 0,
    totalCompletedDiskStores: 0,
    totalDownsampleMs: 0,
    totalMeshMs: 0,
    totalGeneratedByLevel: [0, 0, 0, 0, 0],
    totalMemoryCacheHitsByLevel: [0, 0, 0, 0, 0],
    totalEmptyCacheHitsByLevel: [0, 0, 0, 0, 0],
    totalDiskCacheHitsByLevel: [0, 0, 0, 0, 0],
    maxLodChunkMs: 0,
    maxWorstRecentFrameMs: snapshot.frameTiming.worstRecentFrameMs,
    maxRecentHitchCount: snapshot.frameTiming.recentHitchCount,
    maxRecentDroppedFrameEstimate: snapshot.frameTiming.recentDroppedFrameEstimate,
    finalSnapshot: snapshot,
    finalCoverage: summarizeLodCoverageForPersistence(game),
  });
}

function isLodPersistenceSettled(snapshot: ReturnType<NonNullable<Window["__VOXELS_GAME__"]>["snapshot"]>): boolean {
  return snapshot.streamPendingChunks === 0
    && snapshot.lodPendingChunks === 0
    && snapshot.meshNewChunks === 0
    && snapshot.meshRemeshChunks === 0
    && snapshot.bootstrapPlayableReady === true;
}

function summarizeLodCoverageForPersistence(
  game: NonNullable<Window["__VOXELS_GAME__"]>,
): LodPersistencePhaseSummary["finalCoverage"] {
  const coverage = game.probeLodCoverage(48, 1.6);
  const maxSeamGapMeters = Math.max(
    0,
    ...coverage.uncoveredGapSamples.map((sample) => sample.distanceMeters),
    ...coverage.handoffHoleSamples.map((sample) => sample.distanceMeters),
  );
  const maxLodOverlapMeters = Math.max(
    0,
    ...coverage.residentOverlapSamples.map((sample) => sample.distanceMeters),
    ...coverage.bandOverlapSamples.map((sample) => sample.distanceMeters),
    ...coverage.waterOverlapSamples.map((sample) => sample.distanceMeters),
  );
  return {
    sampleCount: coverage.sampleCount,
    uncoveredGapCount: coverage.uncoveredGapCount,
    handoffHoleCount: coverage.handoffHoleCount,
    residentOverlapCount: coverage.residentOverlapCount,
    bandOverlapCount: coverage.bandOverlapCount,
    waterOverlapCount: coverage.waterOverlapCount,
    maxSeamGapMeters,
    maxLodOverlapMeters,
  };
}

function validateLodPersistenceIteration(iteration: LodPersistenceIteration): string[] {
  const failures: string[] = [];
  if (!iteration.coldOrigin.settled && iteration.coldOrigin.finalLodPendingChunks > 8) {
    failures.push("cold origin phase did not settle");
  }
  if (iteration.farEviction.label !== "far-eviction-skipped" && !iteration.farEviction.settled) {
    failures.push("far eviction phase did not settle");
  }
  if (iteration.storeFlush.totalCompletedDiskStores <= 0 && iteration.storeFlush.totalScheduledDiskStores <= 0) {
    failures.push("populate run did not schedule or complete derived LOD disk stores");
  }
  if (!iteration.reloadOrigin.settled && iteration.reloadOrigin.finalLodPendingChunks > 8) {
    failures.push("reload origin phase did not settle");
  }
  if (iteration.reloadOrigin.totalScheduledDiskRequests <= 0 && iteration.reloadOrigin.finalCumulativeScheduledDiskRequests <= 0) {
    failures.push("reload origin did not schedule derived LOD disk reads");
  }
  if (iteration.reloadOrigin.totalDiskCacheHits <= 0 && iteration.reloadOrigin.finalCumulativeDiskCacheHits <= 0) {
    failures.push("reload origin did not adopt any derived LOD chunks from IndexedDB");
  }
  if (iteration.reloadOrigin.totalGenerated >= iteration.coldOrigin.totalGenerated && iteration.reloadOrigin.finalCumulativeDiskCacheHits <= 0) {
    failures.push("reload origin generated at least as many LOD chunks as cold origin");
  }
  for (const phase of [iteration.coldOrigin, iteration.reloadOrigin]) {
    if (phase.finalCoverage.uncoveredGapCount > 0) {
      failures.push(`${phase.label} has ${phase.finalCoverage.uncoveredGapCount} uncovered LOD sample gaps`);
    }
    if (phase.finalCoverage.handoffHoleCount > 0) {
      failures.push(`${phase.label} has ${phase.finalCoverage.handoffHoleCount} LOD handoff holes`);
    }
    if (phase.finalCoverage.residentOverlapCount > 0 || phase.finalCoverage.bandOverlapCount > 0) {
      failures.push(`${phase.label} has LOD overlap samples`);
    }
  }
  if (
    iteration.farEviction.label !== "far-eviction-skipped"
    && (iteration.farEviction.finalCoverage.residentOverlapCount > 0 || iteration.farEviction.finalCoverage.bandOverlapCount > 0)
  ) {
    failures.push("far-eviction has LOD overlap samples");
  }
  return failures;
}

function aggregateLodPersistenceIterations(
  iterations: readonly LodPersistenceIteration[],
): Record<string, number | string | boolean | null> {
  const reloads = iterations.map((iteration) => iteration.reloadOrigin);
  const cold = iterations.map((iteration) => iteration.coldOrigin);
  const stores = iterations.map((iteration) => iteration.storeFlush);
  const far = iterations
    .map((iteration) => iteration.farEviction)
    .filter((phase) => phase.label !== "far-eviction-skipped");
  const failures = iterations.flatMap((iteration) => iteration.failures);
  return {
    pass: failures.length === 0,
    failureCount: failures.length,
    iterationCount: iterations.length,
    totalReloadDiskCacheHits: sumNumbers(reloads.map((phase) => phase.totalDiskCacheHits)),
    totalReloadCumulativeDiskCacheHits: sumNumbers(reloads.map((phase) => phase.finalCumulativeDiskCacheHits)),
    totalReloadDiskCacheMisses: sumNumbers(reloads.map((phase) => phase.totalDiskCacheMisses)),
    totalReloadScheduledDiskRequests: sumNumbers(reloads.map((phase) => phase.totalScheduledDiskRequests)),
    totalReloadCumulativeScheduledDiskRequests: sumNumbers(reloads.map((phase) => phase.finalCumulativeScheduledDiskRequests)),
    totalStoreScheduledDiskStores: sumNumbers(stores.map((phase) => phase.totalScheduledDiskStores)),
    totalStoreCompletedDiskStores: sumNumbers(stores.map((phase) => phase.totalCompletedDiskStores)),
    avgColdGenerated: averageNumber(cold.map((phase) => phase.totalGenerated)),
    avgReloadGenerated: averageNumber(reloads.map((phase) => phase.totalGenerated)),
    avgColdDownsampleMs: averageNumber(cold.map((phase) => phase.totalDownsampleMs)),
    avgReloadDownsampleMs: averageNumber(reloads.map((phase) => phase.totalDownsampleMs)),
    maxReloadLodChunkMs: maxNumber(reloads.map((phase) => phase.maxLodChunkMs)),
    maxReloadWorstRecentFrameMs: maxNumber(reloads.map((phase) => phase.maxWorstRecentFrameMs)),
    maxReloadCoverageGaps: maxNumber(reloads.map((phase) =>
      phase.finalCoverage.uncoveredGapCount + phase.finalCoverage.handoffHoleCount)),
    maxReloadCoverageOverlaps: maxNumber(reloads.map((phase) =>
      phase.finalCoverage.residentOverlapCount + phase.finalCoverage.bandOverlapCount)),
    maxFarLodPendingChunks: maxNumber(far.map((phase) => phase.finalLodPendingChunks)),
    maxFarPendingDiskCache: maxNumber(far.map((phase) => phase.finalLodPendingDiskCache)),
    maxFarPendingGenerationBudget: maxNumber(far.map((phase) => phase.finalLodPendingGenerationBudget)),
    maxFarPendingPartialBuild: maxNumber(far.map((phase) => phase.finalLodPendingPartialBuild)),
    maxFarPendingPrepared: maxNumber(far.map((phase) => phase.finalLodPendingPrepared)),
    maxFarPendingInvalidatedEviction: maxNumber(far.map((phase) => phase.finalLodPendingInvalidatedEviction)),
    maxFarPendingPlanning: maxNumber(far.map((phase) => phase.finalLodPendingPlanning)),
    maxFarPendingDiskCacheByLevel: maxLevelCounts(far.map((phase) => phase.finalLodPendingDiskCacheByLevel)),
    maxFarPendingGenerationBudgetByLevel: maxLevelCounts(far.map((phase) => phase.finalLodPendingGenerationBudgetByLevel)),
    maxFarPendingPartialBuildByLevel: maxLevelCounts(far.map((phase) => phase.finalLodPendingPartialBuildByLevel)),
    maxFarPendingPreparedByLevel: maxLevelCounts(far.map((phase) => phase.finalLodPendingPreparedByLevel)),
    maxFarLodChunkCountByLevel: maxLevelCounts(far.map((phase) => phase.finalLodChunkCountByLevel)),
    maxReloadPendingGenerationBudgetByLevel: maxLevelCounts(reloads.map((phase) => phase.finalLodPendingGenerationBudgetByLevel)),
    maxReloadLodChunkCountByLevel: maxLevelCounts(reloads.map((phase) => phase.finalLodChunkCountByLevel)),
    firstFailure: failures[0] ?? null,
  };
}

function readFlag(args: readonly string[], flag: string): string | null {
  const exact = args.find((arg) => arg.startsWith(`${flag}=`));
  if (exact) {
    return exact.slice(flag.length + 1);
  }
  const index = args.indexOf(flag);
  if (index === -1) {
    return null;
  }
  return args[index + 1] ?? null;
}

function readBooleanFlag(args: readonly string[], flag: string, fallback: boolean): boolean {
  const value = readFlag(args, flag);
  if (value === null) {
    return fallback;
  }
  if (value === "true" || value === "1") {
    return true;
  }
  if (value === "false" || value === "0") {
    return false;
  }
  throw new Error(`Expected a boolean for ${flag}, received "${value}"`);
}

function readPositiveInt(value: string | null, fallback: number): number {
  if (value === null) {
    return fallback;
  }
  const parsed = Number.parseInt(value, 10);
  if (!Number.isFinite(parsed) || parsed <= 0) {
    throw new Error(`Expected a positive integer, received "${value}"`);
  }
  return parsed;
}

function readNonNegativeInt(value: string | null, fallback: number): number {
  if (value === null) {
    return fallback;
  }
  const parsed = Number.parseInt(value, 10);
  if (!Number.isFinite(parsed) || parsed < 0) {
    throw new Error(`Expected a non-negative integer, received "${value}"`);
  }
  return parsed;
}

function readPositiveFloat(value: string | null, fallback: number): number {
  if (value === null) {
    return fallback;
  }
  const parsed = Number.parseFloat(value);
  if (!Number.isFinite(parsed) || parsed <= 0) {
    throw new Error(`Expected a positive number, received "${value}"`);
  }
  return parsed;
}

function readFloat(value: string | null, fallback: number): number {
  if (value === null) {
    return fallback;
  }
  const parsed = Number.parseFloat(value);
  if (!Number.isFinite(parsed)) {
    throw new Error(`Expected a number, received "${value}"`);
  }
  return parsed;
}

function readOptionalPositiveInt(value: string | null): number | null {
  if (value === null) {
    return null;
  }
  return readPositiveInt(value, 1);
}

function countFrameDrops(frameSamplesMs: readonly number[]): {
  framesOver16_67Ms: number;
  framesOver33_33Ms: number;
  framesOver50Ms: number;
} {
  return {
    framesOver16_67Ms: frameSamplesMs.filter((value) => value > 16.67).length,
    framesOver33_33Ms: frameSamplesMs.filter((value) => value > 33.33).length,
    framesOver50Ms: frameSamplesMs.filter((value) => value > 50).length,
  };
}

function serializeBootstrapSample(
  iteration: BrowserBenchmarkIterationResult<BootstrapExperienceBenchmark>,
  sample: BootstrapBenchmarkSample,
): Record<string, string | number | boolean | null | undefined> {
  const { lodDrawCallsByLevel, ...serializableSample } = sample;
  return {
    scenarioId: iteration.scenarioId,
    warmup: iteration.warmup,
    iteration: iteration.iteration,
    globalIndex: iteration.globalIndex,
    ...serializableSample,
    lodDrawCallsByLevel: lodDrawCallsByLevel.join("/"),
  };
}

function serializeRouteSample(
  iteration: BrowserBenchmarkIterationResult<RouteExperienceBenchmark>,
  sample: RouteExperienceFrameSample,
): Record<string, string | number | boolean | null | undefined> {
  return {
    scenarioId: iteration.scenarioId,
    warmup: iteration.warmup,
    iteration: iteration.iteration,
    globalIndex: iteration.globalIndex,
    frame: sample.frame,
    phase: sample.phase,
    simTimeSeconds: sample.simTimeSeconds,
    routeDistanceMeters: sample.routeDistanceMeters,
    feetPositionX: sample.feetPosition[0],
    feetPositionY: sample.feetPosition[1],
    feetPositionZ: sample.feetPosition[2],
    yaw: sample.yaw,
    pitch: sample.pitch,
    changed: sample.changed,
    complete: sample.complete,
    pendingChunks: sample.pendingChunks,
    pendingMeshJobs: sample.pendingMeshJobs,
    dirtyResidentChunks: sample.dirtyResidentChunks,
    dirtyMeshlessResidentChunks: sample.dirtyMeshlessResidentChunks,
    dirtyRetainedMeshResidentChunks: sample.dirtyRetainedMeshResidentChunks,
    generatedChunks: sample.generatedChunks,
    evictedChunks: sample.evictedChunks,
    movementMs: sample.movementMs,
    streamMs: sample.streamMs,
    meshMs: sample.meshMs,
    meshCount: sample.meshCount,
    gameplayFrameMs: sample.gameplayFrameMs,
    accountedFrameMs: sample.accountedFrameMs,
    unmeasuredFrameMs: sample.unmeasuredFrameMs,
    diagnosticsMs: sample.diagnosticsMs,
    captureDiagnosticsMs: sample.captureDiagnosticsMs,
    renderCpuMs: sample.renderCpuMs,
    renderSyncMs: sample.renderSyncMs,
    renderUploadMs: sample.renderUploadMs,
    renderEncodeMs: sample.renderEncodeMs,
    renderOtherMs: sample.renderOtherMs,
    uploadChunks: sample.uploadChunks,
    uploadBytes: sample.uploadBytes,
    drawCalls: sample.drawCalls,
    triangles: sample.triangles,
    residentNearSamples: sample.residentNearSamples,
    renderReadyNearSamples: sample.renderReadyNearSamples,
    residentNotReadyNearSamples: sample.residentNotReadyNearSamples,
    visibleGroundSampleCount: sample.visibleGroundSampleCount,
    visibleGroundUncoveredCount: sample.visibleGroundUncoveredCount,
    visibleGroundResidentNotReadyCount: sample.visibleGroundResidentNotReadyCount,
    farLodCoverageGapCount: sample.farLodCoverageGapCount,
    maxFarLodCoverageGapMeters: sample.maxFarLodCoverageGapMeters,
    seamGapCount: sample.seamGapCount,
    maxSeamGapMeters: sample.maxSeamGapMeters,
    screenVoidRatio: sample.screenVoidRatio,
    screenVoidMaxRunRatio: sample.screenVoidMaxRunRatio,
    screenVoidSuspicious: sample.screenVoidSuspicious,
    settledReferenceChangedRatio: sample.settledReferenceChangedRatio,
    settledReferenceClearToFilledRatio: sample.settledReferenceClearToFilledRatio,
    settledReferenceMaxClearToFilledRunRatio: sample.settledReferenceMaxClearToFilledRunRatio,
    settledReferenceSuspiciousHole: sample.settledReferenceSuspiciousHole,
    suspiciousHole: sample.suspiciousHole,
  };
}

function aggregateIterationRows(
  rows: readonly Record<string, string | number | boolean | null | undefined>[],
  numericKeys: readonly string[],
): Record<string, number | null> {
  const measuredRows = rows.filter((row) => row.warmup !== true);
  const sourceRows = measuredRows.length > 0 ? measuredRows : rows;
  const aggregate: Record<string, number | null> = {
    rowCount: sourceRows.length,
  };
  for (const key of numericKeys) {
    const values = sourceRows
      .map((row) => row[key])
      .filter((value): value is number => typeof value === "number" && Number.isFinite(value));
    aggregate[`avg_${key}`] = values.length > 0 ? roundNumber(sumNumbers(values) / values.length) : null;
    aggregate[`max_${key}`] = values.length > 0 ? roundNumber(Math.max(...values)) : null;
  }
  return aggregate;
}

function buildStartupAggregateRow(
  iteration: BrowserBenchmarkIterationResult<BootstrapExperienceBenchmark>,
): Record<string, string | number | boolean | null | undefined> {
  const drops = countFrameDrops(iteration.result.samples.map((sample) => sample.gameplayFrameMs));
  const memory = summarizeMemorySamples(iteration.pollSamples);
  return {
    warmup: iteration.warmup,
    benchmarkElapsedMs: iteration.benchmarkElapsedMs,
    playableReadyElapsedMs: iteration.result.summary.playableReadyElapsedMs,
    visualReadyElapsedMs: iteration.result.summary.visualReadyElapsedMs,
    totalGeneratedChunks: iteration.result.summary.totalGeneratedChunks,
    peakGenerationWorkerCount: memory.peakGenerationWorkerCount,
    peakJsHeapUsedSizeBytes: memory.peakJsHeapUsedSizeBytes,
    peakRuntimeHeapUsedBytes: memory.peakRuntimeHeapUsedBytes,
    deltaTaskDurationMs: memory.deltaTaskDurationMs,
    framesOver16_67Ms: drops.framesOver16_67Ms,
    framesOver33_33Ms: drops.framesOver33_33Ms,
  };
}

function buildWalkAggregateRow(
  iteration: BrowserBenchmarkIterationResult<RouteExperienceBenchmark>,
): Record<string, string | number | boolean | null | undefined> {
  const drops = countFrameDrops(iteration.result.samples.map((sample) => sample.gameplayFrameMs));
  const memory = summarizeMemorySamples(iteration.pollSamples);
  return {
    warmup: iteration.warmup,
    benchmarkElapsedMs: iteration.benchmarkElapsedMs,
    avgGameplayFrameMs: iteration.result.summary.avgGameplayFrameMs,
    p95GameplayFrameMs: iteration.result.summary.p95GameplayFrameMs,
    maxGameplayFrameMs: iteration.result.summary.maxGameplayFrameMs,
    framesWithHoleSignals: iteration.result.summary.framesWithHoleSignals,
    peakGenerationWorkerCount: memory.peakGenerationWorkerCount,
    peakJsHeapUsedSizeBytes: memory.peakJsHeapUsedSizeBytes,
    peakRuntimeHeapUsedBytes: memory.peakRuntimeHeapUsedBytes,
    deltaTaskDurationMs: memory.deltaTaskDurationMs,
    framesOver16_67Ms: drops.framesOver16_67Ms,
    framesOver33_33Ms: drops.framesOver33_33Ms,
  };
}

function countMeasuredIterations<TResult>(iterations: readonly BrowserBenchmarkIterationResult<TResult>[]): number {
  return iterations.filter((iteration) => !iteration.warmup).length;
}

function sumNumbers(values: readonly number[]): number {
  return values.reduce((sum, value) => sum + value, 0);
}

function roundNumber(value: number): number {
  return Number(value.toFixed(3));
}

function printSummary(
  outputDir: string,
  reportPath: string,
  startupArtifacts: BenchmarkArtifactPaths | null,
  walkArtifacts: BenchmarkArtifactPaths | null,
  startupResults: readonly BrowserBenchmarkIterationResult<BootstrapExperienceBenchmark>[],
  walkResults: readonly BrowserBenchmarkIterationResult<RouteExperienceBenchmark>[],
  lodPersistenceResult: LodPersistenceScenarioResult | null,
): void {
  const startupMeasured = startupResults.filter((iteration) => !iteration.warmup);
  const walkMeasured = walkResults.filter((iteration) => !iteration.warmup);
  const startupPlayableAvg = averageNumber(startupMeasured.map((iteration) => iteration.result.summary.playableReadyElapsedMs));
  const startupVisualAvg = averageNumber(startupMeasured.map((iteration) => iteration.result.summary.visualReadyElapsedMs));
  const walkP95 = averageNumber(walkMeasured.map((iteration) => iteration.result.summary.p95GameplayFrameMs));
  const walkHoles = sumNumbers(walkMeasured.map((iteration) => iteration.result.summary.framesWithHoleSignals));
  console.log(`Output directory: ${outputDir}`);
  console.log(`Report JSON: ${reportPath}`);
  if (startupArtifacts) {
    console.log(`Startup iteration CSV: ${startupArtifacts.iterationCsvPath}`);
    console.log(`Startup sample CSV: ${startupArtifacts.samplesCsvPath ?? "n/a"}`);
    console.log(`Startup memory CSV: ${startupArtifacts.memoryCsvPath}`);
    console.log(`Startup avg playable-ready ms: ${startupPlayableAvg ?? "n/a"}`);
    console.log(`Startup avg visual-ready ms: ${startupVisualAvg ?? "n/a"}`);
  }
  if (walkArtifacts) {
    console.log(`Walk iteration CSV: ${walkArtifacts.iterationCsvPath}`);
    console.log(`Walk sample CSV: ${walkArtifacts.samplesCsvPath ?? "n/a"}`);
    console.log(`Walk memory CSV: ${walkArtifacts.memoryCsvPath}`);
    console.log(`Walk avg p95 gameplay frame ms: ${walkP95 ?? "n/a"}`);
    console.log(`Walk total hole-signal frames across measured iterations: ${walkHoles}`);
  }
  if (lodPersistenceResult) {
    console.log(`LOD persistence report JSON: ${lodPersistenceResult.artifactPath}`);
    console.log(`LOD persistence pass: ${lodPersistenceResult.aggregate.pass}`);
    console.log(`LOD persistence reload disk hits: ${lodPersistenceResult.aggregate.totalReloadDiskCacheHits}`);
    console.log(`LOD persistence reload cumulative disk hits: ${lodPersistenceResult.aggregate.totalReloadCumulativeDiskCacheHits}`);
    console.log(`LOD persistence first failure: ${lodPersistenceResult.aggregate.firstFailure ?? "n/a"}`);
  }
}

function averageNumber(values: readonly (number | null)[]): number | null {
  const numericValues = values.filter((value): value is number => typeof value === "number" && Number.isFinite(value));
  if (numericValues.length === 0) {
    return null;
  }
  return roundNumber(sumNumbers(numericValues) / numericValues.length);
}

function maxNumber(values: readonly number[]): number | null {
  const numericValues = values.filter((value) => Number.isFinite(value));
  if (numericValues.length === 0) {
    return null;
  }
  return roundNumber(Math.max(...numericValues));
}

function maxLevelCounts(values: readonly (readonly number[])[]): string | null {
  if (values.length === 0) {
    return null;
  }
  const levelCount = Math.max(0, ...values.map((counts) => counts.length));
  if (levelCount === 0) {
    return null;
  }
  const maxima = Array.from({ length: levelCount }, (_, level) =>
    Math.max(0, ...values.map((counts) => counts[level] ?? 0)),
  );
  return maxima.join("/");
}
