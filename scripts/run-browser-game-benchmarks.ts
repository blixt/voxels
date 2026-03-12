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
    csvPaths: BenchmarkArtifactPaths;
    aggregate: Record<string, number | null>;
  }>;
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
          return {
            scenarioId: iteration.scenarioId,
            warmup: iteration.warmup,
            iteration: iteration.iteration,
            globalIndex: iteration.globalIndex,
            setupElapsedMs: iteration.setupElapsedMs,
            benchmarkElapsedMs: iteration.benchmarkElapsedMs,
            ...iteration.result.summary,
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
            "totalFarFieldMs",
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
      description: "Fresh-load deterministic 10-second forward walk through streaming terrain",
      warmupIterations: options.walkWarmup,
      measuredIterations: options.walkIterations,
      timeoutMs: options.walkTimeoutMs,
      sampleIntervalMs: options.walkSampleIntervalMs,
      async prepareIteration(benchmarkSession, run) {
        await benchmarkSession.navigateToGame({ clearStorage: true });
        await benchmarkSession.waitForBootstrapBenchmarkComplete(options.startupTimeoutMs);
        await benchmarkSession.waitForGameReady(options.startupTimeoutMs);
        const token = await benchmarkSession.startAsyncWindowBenchmark(
          `window.__VOXELS_GAME__.benchmarkForwardWalkExperience(${JSON.stringify({
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
            "avgFarFieldMs",
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
      viewportWidth: options.viewportWidth,
      viewportHeight: options.viewportHeight,
      headless: options.headless,
      skipBuild: options.skipBuild,
    },
    scenarios: scenarioReports,
  };
  await Bun.write(reportPath, `${JSON.stringify(report, null, 2)}\n`);

  printSummary(outputDir, reportPath, startupArtifacts, walkArtifacts, startupResults, walkResults);
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
  return {
    scenarioId: iteration.scenarioId,
    warmup: iteration.warmup,
    iteration: iteration.iteration,
    globalIndex: iteration.globalIndex,
    ...sample,
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
    farFieldMs: sample.farFieldMs,
    farFieldPrefetchMs: sample.farFieldPrefetchMs,
    farFieldPrefetchRequestedChunks: sample.farFieldPrefetchRequestedChunks,
    farFieldSampleCacheMs: sample.farFieldSampleCacheMs,
    farFieldMeshBuildMs: sample.farFieldMeshBuildMs,
    farFieldSampledCellCount: sample.farFieldSampledCellCount,
    farFieldMaxBandMs: sample.farFieldMaxBandMs,
    farFieldMaxBandLabel: sample.farFieldMaxBandLabel,
    farFieldBuiltBands: sample.farFieldBuiltBands,
    farFieldPendingBands: sample.farFieldPendingBands,
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
    visibleGroundFarOnlyCount: sample.visibleGroundFarOnlyCount,
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
}

function averageNumber(values: readonly (number | null)[]): number | null {
  const numericValues = values.filter((value): value is number => typeof value === "number" && Number.isFinite(value));
  if (numericValues.length === 0) {
    return null;
  }
  return roundNumber(sumNumbers(numericValues) / numericValues.length);
}
