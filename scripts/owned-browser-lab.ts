import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createServer } from "node:net";
import { inflateSync } from "node:zlib";

export {};

type ServerMode = "dev" | "prod" | "existing";

interface CliOptions {
  label: string | null;
  outputDir: string;
  serverMode: ServerMode;
  port: number | null;
  chromeBinary: string;
  headless: boolean;
  settleMaxFrames: number;
  lodRadiusMeters: number;
  lodStepMeters: number;
  renderRadiusMeters: number;
  renderStepMeters: number;
  visibleForwardMeters: number;
  visibleLateralMeters: number;
  visibleStepMeters: number;
}

interface CommandResult {
  command: string[];
  exitCode: number;
  elapsedMs: number;
  stdout: string;
  stderr: string;
}

interface CdpMessage {
  id?: number;
  method?: string;
  params?: Record<string, unknown>;
  result?: Record<string, unknown>;
  error?: { code: number; message: string };
}

interface CdpConnection {
  close(): Promise<void>;
  send(method: string, params?: Record<string, unknown>): Promise<Record<string, unknown>>;
  waitForEvent(method: string, timeoutMs: number): Promise<Record<string, unknown>>;
  evaluate<T>(expression: string): Promise<T>;
}

interface DevToolsVersionResponse {
  webSocketDebuggerUrl: string;
  Browser?: string;
}

interface DevToolsTargetResponse {
  id: string;
  webSocketDebuggerUrl: string;
}

const PERFORMANCE_BUDGETS = {
  maxAvgFrameCpuMs: 8,
  maxLastFrameCpuMs: 12,
  maxRenderStageMs: 12,
  maxDrawCalls: 1_200,
  maxTriangles: 1_500_000,
  maxTraversalP95GameplayFrameMs: 120,
  maxTraversalMaxGameplayFrameMs: 66,
  maxTraversalP95MeasuredWorkMs: 24,
  maxTraversalP95RenderCpuMs: 12,
  maxTraversalP95StreamMs: 12,
  maxTraversalP95MeshMs: 12,
  maxTraversalP95LodMs: 12,
  maxRouteP95GameplayFrameMs: 120,
  maxRouteMaxGameplayFrameMs: 66,
  maxRouteP95MeasuredWorkMs: 24,
  maxRouteP95RenderCpuMs: 12,
  maxRouteP95StreamMs: 12,
  maxRouteP95MeshMs: 12,
  maxRouteP95LodMs: 12,
  minTraversalScreenCaptures: 1,
  minRouteScreenCaptures: 2,
  maxBootstrapPlayableMs: 3_000,
  maxBootstrapVisualMs: 3_000,
  maxBootstrapP95GameplayFrameMs: 220,
} as const;

const options = parseCli(Bun.argv);
const runStamp = timestampForFile(new Date());
const runName = `${runStamp}${options.label ? `-${sanitizeFileStem(options.label)}` : ""}`;
const outputDir = join(options.outputDir, runName);
const reportPath = join(outputDir, "report.json");
const screenshotPath = join(outputDir, "settled-page.png");
const appPort = options.port ?? (options.serverMode === "existing" ? 3000 : await findFreePort());
const appUrl = `http://127.0.0.1:${appPort}/?benchmarkBootstrap=1`;
const devToolsPort = await findFreePort();

await Bun.$`mkdir -p ${outputDir}`.quiet();

let serverProcess: Bun.Subprocess | null = null;
let chromeProcess: Bun.Subprocess | null = null;
let chromeProfileDir: string | null = null;
let cdp: CdpConnection | null = null;

try {
  let build: CommandResult | null = null;
  if (options.serverMode === "prod") {
    build = runCommand(["bun", "run", "build"]);
    if (build.exitCode !== 0) {
      throw new Error(`Production build failed:\n${build.stderr || build.stdout}`);
    }
  }

  if (options.serverMode !== "existing") {
    serverProcess = Bun.spawn(options.serverMode === "prod" ? ["bun", "run", "start"] : ["bun", "run", "dev"], {
      cwd: process.cwd(),
      env: {
        ...process.env,
        NODE_ENV: options.serverMode === "prod" ? "production" : "development",
        PORT: String(appPort),
      },
      stdout: "pipe",
      stderr: "pipe",
    });
  }
  await waitForHttp(`http://127.0.0.1:${appPort}/`, 20_000);

  chromeProfileDir = await mkdtemp(join(tmpdir(), "voxels-owned-browser-"));
  chromeProcess = Bun.spawn(buildChromeCommand(options.chromeBinary, devToolsPort, chromeProfileDir, options.headless), {
    cwd: process.cwd(),
    stdout: "ignore",
    stderr: "ignore",
  });

  const version = await waitForJsonEndpoint<DevToolsVersionResponse>(
    `http://127.0.0.1:${devToolsPort}/json/version`,
    20_000,
  );
  const target = await createDevToolsTarget(devToolsPort);
  cdp = await connectCdp(target.webSocketDebuggerUrl);
  await cdp.send("Page.enable");
  await cdp.send("Runtime.enable");
  await cdp.send("Performance.enable");

  const loadEvent = cdp.waitForEvent("Page.loadEventFired", 30_000);
  await cdp.send("Page.navigate", { url: appUrl });
  await loadEvent;
  await waitForGameApi(cdp, 30_000);
  await waitForGameWorldReady(cdp, 120_000);

  const hudSmoke = await runHudSmoke(cdp);
  const pageReport = await cdp.evaluate<Record<string, unknown>>(buildPageProbeExpression(options));
  const screenshot = await cdp.send("Page.captureScreenshot", {
    format: "png",
    fromSurface: true,
  });
  const screenshotData = readStringField(screenshot, "data");
  let visualIdentity: Record<string, unknown> | null = null;
  if (screenshotData) {
    const screenshotBytes = Buffer.from(screenshotData, "base64");
    await Bun.write(screenshotPath, screenshotBytes);
    visualIdentity = analyzeScreenshotVisualIdentity(screenshotBytes);
  }

  const failures = findFailures(pageReport, hudSmoke);
  const report = {
    generatedAt: new Date().toISOString(),
    commit: readGitShortHead(),
    appUrl,
    outputDir,
    screenshotPath: screenshotData ? screenshotPath : null,
    chromeBinary: options.chromeBinary,
    browserVersion: version.Browser ?? null,
    serverMode: options.serverMode,
    build,
    options,
    hudSmoke,
    visualIdentity,
    page: pageReport,
    failures,
  };
  await Bun.write(reportPath, `${JSON.stringify(report, null, 2)}\n`);
  printSummary(reportPath, report);

  if (failures.length > 0) {
    process.exitCode = 1;
  }
} finally {
  await cdp?.close();
  chromeProcess?.kill();
  serverProcess?.kill();
  if (chromeProfileDir) {
    await rm(chromeProfileDir, { recursive: true, force: true });
  }
}

function buildPageProbeExpression(options: CliOptions): string {
  return `(${async function pageProbe(rawOptions: Record<string, number>) {
    const startedAt = performance.now();
    const game = window.__VOXELS_GAME__;
    if (!game) {
      return {
        hasGameApi: false,
        elapsedMs: performance.now() - startedAt,
      };
    }
    const before = game.snapshot();
    const bootstrapBenchmark = summarizeBootstrapBenchmark(game.getBootstrapBenchmark());
    const settleOptions = {
      radiusChunks: before.residencyRadiusChunks,
      maxFrames: rawOptions.settleMaxFrames,
    } as Record<string, number>;
    const settled = summarizeTransitionProbe(await game.teleportAndSettle(
      before.position[0],
      before.position[1],
      before.position[2],
      settleOptions,
    ));
    const forcedResidency = game.forceResidencyUpdate();
    const internalAfterForce = summarizeInternalWorld(game);
    await new Promise((resolve) => requestAnimationFrame(() => resolve(null)));
    const after = game.snapshot();
    const progressPersistence = verifyProgressPersistence(game);
    const stagedProgressFixtures = verifyStagedProgressFixtures(game);
    const traversalBudget = summarizeTraversalBenchmark(await game.benchmarkLiveForwardWalkExperience({
      durationSeconds: 2,
      settleSeconds: 1,
      sampleHz: 20,
      seamProbeStrideFrames: 20,
      captureStrideFrames: 60,
      referenceDiffStrideFrames: 0,
      referenceDiffLimit: 0,
    }));
    const routeBudget = summarizeRouteBenchmark(await game.benchmarkLiveForwardWalkExperience({
      durationSeconds: 8,
      settleSeconds: 2,
      sampleHz: 20,
      seamProbeStrideFrames: 20,
      captureStrideFrames: 120,
      referenceDiffStrideFrames: 0,
      referenceDiffLimit: 0,
      yawDriftRadians: 0.34,
      yawDriftPeriodSeconds: 6,
      sprint: true,
    }));
    await game.teleportAndSettle(after.position[0], after.position[1], after.position[2], settleOptions);
    const renderReady = game.probeRenderReadyCoverage(rawOptions.renderRadiusMeters, rawOptions.renderStepMeters);
    const visibleGround = game.probeVisibleGroundCoverage(
      rawOptions.visibleForwardMeters,
      rawOptions.visibleLateralMeters,
      rawOptions.visibleStepMeters,
    );
    const lodCoverage = game.probeLodCoverage(rawOptions.lodRadiusMeters, rawOptions.lodStepMeters);
    const traversalSkillMovement = await verifyTraversalSkillMovement(game);
    return {
      hasGameApi: true,
      title: document.title,
      url: location.href,
      webgpuAvailable: Boolean(navigator.gpu),
      elapsedMs: performance.now() - startedAt,
      before,
      bootstrapBenchmark,
      settled,
      forcedResidency,
      internalAfterForce,
      after,
      progressPersistence,
      stagedProgressFixtures,
      traversalSkillMovement,
      objectiveBreadcrumb: await verifyObjectiveBreadcrumb(game, after.position, settleOptions),
      traversalBudget,
      routeBudget,
      renderReady,
      visibleGround,
      lodCoverage,
    };

    function summarizeInternalWorld(gameApi: NonNullable<typeof window.__VOXELS_GAME__>) {
      const levels: Record<string, number> = {};
      let renderableLodChunks = 0;
      for (const chunk of gameApi.controller.world.iterateResidentChunks()) {
        levels[String(chunk.lodLevel)] = (levels[String(chunk.lodLevel)] ?? 0) + 1;
        if (chunk.lodLevel > 0 && chunk.renderReady && chunk.mesh && chunk.mesh.indexCount > 0) {
          renderableLodChunks += 1;
        }
      }
      return { levels, renderableLodChunks };
    }

    function verifyProgressPersistence(gameApi: NonNullable<typeof window.__VOXELS_GAME__>) {
      const exported = gameApi.exportProgressState();
      const before = summarizeProgress(exported);
      gameApi.resetDiscoveryJournal();
      const afterReset = summarizeProgress(gameApi.exportProgressState());
      const imported = summarizeProgress(gameApi.importProgressState(exported));
      const saved = gameApi.saveProgressState();
      const savedPayload = window.localStorage.getItem("voxels.progress.v1");
      gameApi.resetDiscoveryJournal();
      if (savedPayload) {
        window.localStorage.setItem("voxels.progress.v1", savedPayload);
      }
      const loaded = gameApi.loadProgressState();
      const afterLoad = summarizeProgress(gameApi.exportProgressState());
      const flavoredState = structuredClone(exported);
      flavoredState.discovery.discoveredLandmarkIds = sortedUnique([
        ...flavoredState.discovery.discoveredLandmarkIds,
        "silt_shell",
      ]);
      flavoredState.discovery.recentDiscoveries = [
        {
          category: "landmark",
          id: "silt_shell",
          name: "Silt Strider Shell",
          flavorText: "A hollow carapace rests half-buried in windblown dust.",
          identifier: "silt_shell",
          categoryLabel: "Landmark",
          label: "Landmark: Silt Strider Shell [silt_shell]",
          sequence: 1000,
        },
        ...flavoredState.discovery.recentDiscoveries,
      ];
      flavoredState.discovery.pendingSkillDiscoveries = [];
      flavoredState.discovery.nextDiscoverySequence = Math.max(flavoredState.discovery.nextDiscoverySequence, 1001);
      const flavoredImport = summarizeProgress(gameApi.importProgressState(flavoredState));
      gameApi.importProgressState(exported);
      return {
        before,
        afterReset,
        imported,
        saved,
        loaded,
        afterLoad,
        flavoredImport,
        importRoundTripMatches: progressSummariesMatch(before, imported),
        storageRoundTripMatches: progressSummariesMatch(before, afterLoad),
        flavorRoundTripMatches: flavoredImport.recentDiscoveries.some((discovery) =>
          discovery.id === "silt_shell"
          && discovery.flavorText === "A hollow carapace rests half-buried in windblown dust."),
      };
    }

    function verifyStagedProgressFixtures(gameApi: NonNullable<typeof window.__VOXELS_GAME__>) {
      const exported = gameApi.exportProgressState();
      const before = summarizeProgress(exported);
      const fixtureCases = [
        summarizeFixtureCase(
          "frontier-old-road-1",
          gameApi.importProgressState(createExplorationProgressFixture(exported, "frontier-old-road-1")),
          "ancient-signs-2",
        ),
        summarizeFixtureCase(
          "deep-old-road-3",
          gameApi.importProgressState(createExplorationProgressFixture(exported, "deep-old-road-3")),
          "ancient-signs-4",
        ),
      ];
      gameApi.importProgressState(exported);
      return {
        cases: fixtureCases,
        restoredMatches: progressSummariesMatch(before, summarizeProgress(gameApi.exportProgressState())),
      };

      function summarizeFixtureCase(
        id: string,
        imported: ReturnType<NonNullable<typeof window.__VOXELS_GAME__>["importProgressState"]>,
        oldRoadObjectiveId: string,
      ) {
        const objectives = gameApi.getExplorationObjectives();
        const snapshot = gameApi.snapshot();
        return {
          id,
          stageId: objectives.stageId,
          oldRoadObjective: summarizeObjective(objectives, oldRoadObjectiveId),
          discoveredBiomeCount: imported.discovery.discoveredBiomeIds.length,
          discoveredLandmarkCount: imported.discovery.discoveredLandmarkIds.length,
          discoveredAncientLandmarkCount: snapshot.discoveredAncientLandmarkCount,
          landmarkScanRadiusMeters: snapshot.landmarkScanRadiusMeters,
          landmarkScanSampleCount: snapshot.landmarkScanSampleCount,
          surfaceTravelSpeedMultiplier: snapshot.surfaceTravelSpeedMultiplier,
          undergroundTravelSpeedMultiplier: snapshot.undergroundTravelSpeedMultiplier,
        };
      }
    }

    async function verifyTraversalSkillMovement(gameApi: NonNullable<typeof window.__VOXELS_GAME__>) {
      const exported = gameApi.exportProgressState();
      const before = summarizeProgress(exported);
      let result: {
        novice: Record<string, number>;
        veteran: Record<string, number>;
        restoredMatches: boolean;
      } | null = null;
      try {
        gameApi.importProgressState(createExplorationProgressFixture(exported, "frontier-old-road-1"));
        const noviceSnapshot = gameApi.snapshot();
        const novice = summarizeTraversalBenchmark(await gameApi.benchmarkLiveForwardWalkExperience({
          durationSeconds: 1.5,
          settleSeconds: 1,
          sampleHz: 20,
          seamProbeStrideFrames: 999999,
          captureStrideFrames: 999999,
          referenceDiffStrideFrames: 0,
          referenceDiffLimit: 0,
        }));
        gameApi.importProgressState(createExplorationProgressFixture(exported, "deep-old-road-3"));
        const veteranSnapshot = gameApi.snapshot();
        const veteran = summarizeTraversalBenchmark(await gameApi.benchmarkLiveForwardWalkExperience({
          durationSeconds: 1.5,
          settleSeconds: 1,
          sampleHz: 20,
          seamProbeStrideFrames: 999999,
          captureStrideFrames: 999999,
          referenceDiffStrideFrames: 0,
          referenceDiffLimit: 0,
        }));
        result = {
          novice: {
            surfaceTravelSpeedMultiplier: noviceSnapshot.surfaceTravelSpeedMultiplier,
            totalDistanceMeters: novice.totalDistanceMeters,
            p95GameplayFrameMs: novice.p95GameplayFrameMs,
            p95MeasuredWorkMs: novice.p95MeasuredWorkMs,
          },
          veteran: {
            surfaceTravelSpeedMultiplier: veteranSnapshot.surfaceTravelSpeedMultiplier,
            totalDistanceMeters: veteran.totalDistanceMeters,
            p95GameplayFrameMs: veteran.p95GameplayFrameMs,
            p95MeasuredWorkMs: veteran.p95MeasuredWorkMs,
          },
          restoredMatches: false,
        };
      } finally {
        gameApi.importProgressState(exported);
      }
      return result
        ? {
            ...result,
            restoredMatches: progressSummariesMatch(before, summarizeProgress(gameApi.exportProgressState())),
          }
        : {
            novice: {},
            veteran: {},
            restoredMatches: progressSummariesMatch(before, summarizeProgress(gameApi.exportProgressState())),
          };
    }

    function summarizeProgress(progress: ReturnType<NonNullable<typeof window.__VOXELS_GAME__>["exportProgressState"]>) {
      return {
        version: progress.version,
        discoveredBiomeCount: progress.discovery.discoveredBiomeIds.length,
        discoveredUndergroundBiomeCount: progress.discovery.discoveredUndergroundBiomeIds.length,
        discoveredRegionalVariantCount: progress.discovery.discoveredRegionalVariantIds.length,
        discoveredLandmarkCount: progress.discovery.discoveredLandmarkIds.length,
        recentDiscoveries: progress.discovery.recentDiscoveries.map((discovery) => ({
          category: discovery.category,
          id: discovery.id,
          flavorText: discovery.flavorText ?? null,
          sequence: discovery.sequence,
        })),
        skillXp: Object.fromEntries(Object.entries(progress.skills.xpBySkill).sort()),
        travelMetersBySkill: Object.fromEntries(Object.entries(progress.skills.travelMetersBySkill).sort()),
        travelRemainderMetersBySkill: Object.fromEntries(Object.entries(progress.skills.travelRemainderMetersBySkill).sort()),
        lastProcessedDiscoverySequence: progress.skills.lastProcessedDiscoverySequence,
      };
    }

    async function verifyObjectiveBreadcrumb(
      gameApi: NonNullable<typeof window.__VOXELS_GAME__>,
      restorePosition: readonly number[],
      settleOptions: Record<string, number>,
    ) {
      const exported = gameApi.exportProgressState();
      gameApi.importProgressState(createExplorationProgressFixture(exported, "frontier-old-road-1"));
      const beforeObjectives = gameApi.getExplorationObjectives();
      const beforeAncient = gameApi.snapshot().discoveredAncientLandmarkCount;
      const targetX = -18_400;
      const targetZ = -23_232;
      const targetSurfaceY = gameApi.controller.generator.sampleColumn(targetX, targetZ).surfaceY;
      const targetEyeY = targetSurfaceY + gameApi.controller.player.eyeHeight;
      await gameApi.teleportAndSettle(targetX, targetEyeY, targetZ, settleOptions);
      const afterSnapshot = gameApi.snapshot();
      const afterObjectives = gameApi.getExplorationObjectives();
      await gameApi.teleportAndSettle(restorePosition[0] ?? 0, restorePosition[1] ?? 0, restorePosition[2] ?? 0, settleOptions);
      gameApi.importProgressState(exported);
      return {
        beforeStageId: beforeObjectives.stageId,
        afterStageId: afterObjectives.stageId,
        beforeAncient,
        afterAncient: afterSnapshot.discoveredAncientLandmarkCount,
        currentLandmarkId: afterSnapshot.landmarkId,
        beforeObjective: summarizeObjective(beforeObjectives, "ancient-signs-2"),
        afterObjective: summarizeObjective(afterObjectives, "ancient-signs-2"),
      };
    }

    function summarizeObjective(
      snapshot: ReturnType<NonNullable<typeof window.__VOXELS_GAME__>["getExplorationObjectives"]>,
      id: string,
    ) {
      const objective = snapshot.objectives.find((entry) => entry.id === id) ?? null;
      return objective
        ? {
            id: objective.id,
            progress: objective.progress,
            target: objective.target,
            completed: objective.completed,
          }
        : null;
    }

    function sortedUnique(values: readonly string[]) {
      return [...new Set(values)].sort();
    }

    function createExplorationProgressFixture(
      base: ReturnType<NonNullable<typeof window.__VOXELS_GAME__>["exportProgressState"]>,
      fixtureId: "frontier-old-road-1" | "deep-old-road-3",
    ) {
      const fixture = structuredClone(base);
      fixture.discovery.currentBiomeId = null;
      fixture.discovery.currentUndergroundBiomeId = null;
      fixture.discovery.currentRegionalVariantId = null;
      fixture.discovery.currentLandmarkId = null;
      fixture.discovery.recentDiscoveries = [];
      fixture.discovery.pendingSkillDiscoveries = [];
      fixture.discovery.nextDiscoverySequence = Math.max(fixture.discovery.nextDiscoverySequence, 3000);
      if (fixtureId === "frontier-old-road-1") {
        fixture.skills.xpBySkill = {
          cartography: 0,
          naturalist: 0,
          spelunking: 0,
          lore: 0,
        };
        fixture.discovery.discoveredBiomeIds = sortedUnique(["verdant", "savanna", "dunes"]);
        fixture.discovery.discoveredUndergroundBiomeIds = [];
        fixture.discovery.discoveredRegionalVariantIds = ["savanna_flowersea"];
        fixture.discovery.discoveredLandmarkIds = sortedUnique(["oak", "standing_stone", "ancestor_pillar"]);
        return fixture;
      }
      fixture.skills.xpBySkill = {
        cartography: 700,
        naturalist: 900,
        spelunking: 700,
        lore: 700,
      };
      fixture.discovery.discoveredBiomeIds = sortedUnique([
        "badlands",
        "bloom",
        "dunes",
        "fern",
        "firefly",
        "fungal",
        "highland",
        "marsh",
        "moor",
        "savanna",
      ]);
      fixture.discovery.discoveredUndergroundBiomeIds = sortedUnique(["rooted", "sedimentary", "crystalline"]);
      fixture.discovery.discoveredRegionalVariantIds = sortedUnique([
        "verdant_karst",
        "savanna_flowersea",
        "dunes_glass",
        "badlands_crater",
      ]);
      fixture.discovery.discoveredLandmarkIds = sortedUnique([
        "oak",
        "standing_stone",
        "acacia",
        "hoodoo",
        "boulder",
        "ancestor_pillar",
        "ash_marker",
        "glass_cairn",
      ]);
      return fixture;
    }

    function progressSummariesMatch(left: ReturnType<typeof summarizeProgress>, right: ReturnType<typeof summarizeProgress>) {
      return JSON.stringify(left) === JSON.stringify(right);
    }

    function summarizeTraversalBenchmark(benchmark: Awaited<ReturnType<NonNullable<typeof window.__VOXELS_GAME__>["benchmarkLiveForwardWalkExperience"]>>) {
      return {
        durationSeconds: benchmark.durationSeconds,
        settleSeconds: benchmark.settleSeconds,
        sampleCount: benchmark.summary.sampleCount,
        totalDistanceMeters: benchmark.summary.totalDistanceMeters,
        speedMetersPerSecond: benchmark.summary.speedMetersPerSecond,
        p95GameplayFrameMs: benchmark.summary.p95GameplayFrameMs,
        maxGameplayFrameMs: benchmark.summary.maxGameplayFrameMs,
        p95MovementMs: benchmark.summary.p95MovementMs,
        avgDiagnosticsMs: benchmark.summary.sampleCount > 0
          ? benchmark.summary.totalDiagnosticsMs / benchmark.summary.sampleCount
          : 0,
        totalDiagnosticsMs: benchmark.summary.totalDiagnosticsMs,
        totalCaptureDiagnosticsMs: benchmark.summary.totalCaptureDiagnosticsMs,
        p95MeasuredWorkMs: benchmark.summary.p95MeasuredWorkMs,
        p95RenderCpuMs: benchmark.summary.p95RenderCpuMs,
        p95StreamMs: benchmark.summary.p95StreamMs,
        p95MeshMs: benchmark.summary.p95MeshMs,
        p95LodMs: benchmark.summary.p95LodMs,
        framesWithHoleSignals: benchmark.summary.framesWithHoleSignals,
        framesWithVisibleGroundGaps: benchmark.summary.framesWithVisibleGroundGaps,
        framesWithSeamGaps: benchmark.summary.framesWithSeamGaps,
        framesWithLodOverlaps: benchmark.summary.framesWithLodOverlaps,
        blockingSeamGapCount: countBlockingSeamGaps(benchmark.samples),
        blockingLodOverlapCount: countBlockingLodOverlaps(benchmark.samples),
        maxSeamGapMeters: benchmark.summary.maxSeamGapMeters,
        maxLodOverlapMeters: benchmark.summary.maxLodOverlapMeters,
        screenVoidCaptureCount: benchmark.summary.screenVoidCaptureCount,
        framesWithScreenVoidSignals: benchmark.summary.framesWithScreenVoidSignals,
        maxPendingChunks: benchmark.summary.maxPendingChunks,
        maxPendingMeshJobs: benchmark.summary.maxPendingMeshJobs,
        maxDirtyResidentChunks: benchmark.summary.maxDirtyResidentChunks,
        seamGapSamples: summarizeSeamGapSamples(benchmark.samples),
        slowFrameSamples: summarizeSlowFrameSamples(benchmark.samples),
      };
    }

    function summarizeBootstrapBenchmark(benchmark: ReturnType<NonNullable<typeof window.__VOXELS_GAME__>["getBootstrapBenchmark"]>) {
      return {
        completed: benchmark.completed,
        sampleCount: benchmark.summary.sampleCount,
        playableReadyElapsedMs: benchmark.summary.playableReadyElapsedMs,
        visualReadyElapsedMs: benchmark.summary.visualReadyElapsedMs,
        lodCompleteElapsedMs: benchmark.summary.lodCompleteElapsedMs,
        p95GameplayFrameMs: benchmark.summary.p95GameplayFrameMs,
        maxGameplayFrameMs: benchmark.summary.maxGameplayFrameMs,
        p95RenderCpuMs: benchmark.summary.p95RenderCpuMs,
        p95StreamMs: benchmark.summary.p95StreamMs,
        p95MeshMs: benchmark.summary.p95MeshMs,
        p95LodMs: benchmark.summary.p95LodMs,
        maxLodMs: benchmark.summary.maxLodMs,
        totalLodYRangeMs: benchmark.summary.totalLodYRangeMs,
        totalLodDownsampleMs: benchmark.summary.totalLodDownsampleMs,
        totalLodMeshMs: benchmark.summary.totalLodMeshMs,
        maxPendingChunks: benchmark.summary.maxPendingChunks,
        maxPendingMeshJobs: benchmark.summary.maxPendingMeshJobs,
        maxDirtyResidentChunks: benchmark.summary.maxDirtyResidentChunks,
        maxLodPendingChunks: benchmark.summary.maxLodPendingChunks,
        framesOver33_33Ms: benchmark.summary.framesOver33_33Ms,
      };
    }

    function summarizeRouteBenchmark(benchmark: Awaited<ReturnType<NonNullable<typeof window.__VOXELS_GAME__>["benchmarkRouteExperience"]>>) {
      return {
        durationSeconds: benchmark.durationSeconds,
        settleSeconds: benchmark.settleSeconds,
        sampleCount: benchmark.summary.sampleCount,
        moveFrameCount: benchmark.summary.moveFrameCount,
        settleFrameCount: benchmark.summary.settleFrameCount,
        incompleteFrameCount: benchmark.summary.incompleteFrameCount,
        totalDistanceMeters: benchmark.summary.totalDistanceMeters,
        speedMetersPerSecond: benchmark.summary.speedMetersPerSecond,
        p95GameplayFrameMs: benchmark.summary.p95GameplayFrameMs,
        maxGameplayFrameMs: benchmark.summary.maxGameplayFrameMs,
        p95MovementMs: benchmark.summary.p95MovementMs,
        avgDiagnosticsMs: benchmark.summary.sampleCount > 0
          ? benchmark.summary.totalDiagnosticsMs / benchmark.summary.sampleCount
          : 0,
        totalDiagnosticsMs: benchmark.summary.totalDiagnosticsMs,
        totalCaptureDiagnosticsMs: benchmark.summary.totalCaptureDiagnosticsMs,
        p95MeasuredWorkMs: benchmark.summary.p95MeasuredWorkMs,
        p95RenderCpuMs: benchmark.summary.p95RenderCpuMs,
        p95StreamMs: benchmark.summary.p95StreamMs,
        p95MeshMs: benchmark.summary.p95MeshMs,
        p95LodMs: benchmark.summary.p95LodMs,
        framesWithHoleSignals: benchmark.summary.framesWithHoleSignals,
        framesWithVisibleGroundGaps: benchmark.summary.framesWithVisibleGroundGaps,
        framesWithSeamGaps: benchmark.summary.framesWithSeamGaps,
        framesWithLodOverlaps: benchmark.summary.framesWithLodOverlaps,
        blockingSeamGapCount: countBlockingSeamGaps(benchmark.samples),
        blockingLodOverlapCount: countBlockingLodOverlaps(benchmark.samples),
        maxSeamGapMeters: benchmark.summary.maxSeamGapMeters,
        maxLodOverlapMeters: benchmark.summary.maxLodOverlapMeters,
        screenVoidCaptureCount: benchmark.summary.screenVoidCaptureCount,
        framesWithScreenVoidSignals: benchmark.summary.framesWithScreenVoidSignals,
        framesWithSettledReferenceHoleSignals: benchmark.summary.framesWithSettledReferenceHoleSignals,
        maxVisibleGroundUncoveredCount: benchmark.summary.maxVisibleGroundUncoveredCount,
        maxPendingChunks: benchmark.summary.maxPendingChunks,
        maxPendingMeshJobs: benchmark.summary.maxPendingMeshJobs,
        maxDirtyResidentChunks: benchmark.summary.maxDirtyResidentChunks,
        settleFramesUntilComplete: benchmark.summary.settleFramesUntilComplete,
        seamGapSamples: summarizeSeamGapSamples(benchmark.samples),
        slowFrameSamples: summarizeSlowFrameSamples(benchmark.samples),
      };
    }

    function summarizeSeamGapSamples(samples: readonly {
      frame: number;
      phase: "move" | "settle";
      simTimeSeconds: number;
      routeDistanceMeters: number;
      feetPosition: readonly number[];
      pendingChunks: number;
      pendingMeshJobs: number;
      dirtyResidentChunks: number;
      lodPendingChunks: number;
      seamGapCount: number;
      uncoveredLodGapCount: number;
      handoffLodHoleCount: number;
      maxSeamGapMeters: number;
      lodOverlapCount: number;
      lodResidentOverlapCount: number;
      lodBandOverlapCount: number;
      maxLodOverlapMeters: number;
      visibleGroundUncoveredCount: number;
      screenVoidSuspicious: boolean;
    }[]) {
      return samples
        .filter((sample) => sample.seamGapCount > 0)
        .slice(0, 4)
        .map((sample) => ({
          frame: sample.frame,
          phase: sample.phase,
          simTimeSeconds: sample.simTimeSeconds,
          routeDistanceMeters: sample.routeDistanceMeters,
          feetPosition: sample.feetPosition.slice(0, 3),
          pendingChunks: sample.pendingChunks,
          pendingMeshJobs: sample.pendingMeshJobs,
          dirtyResidentChunks: sample.dirtyResidentChunks,
          lodPendingChunks: sample.lodPendingChunks,
          seamGapCount: sample.seamGapCount,
          uncoveredLodGapCount: sample.uncoveredLodGapCount,
          handoffLodHoleCount: sample.handoffLodHoleCount,
          maxSeamGapMeters: sample.maxSeamGapMeters,
          lodOverlapCount: sample.lodOverlapCount,
          lodResidentOverlapCount: sample.lodResidentOverlapCount,
          lodBandOverlapCount: sample.lodBandOverlapCount,
          maxLodOverlapMeters: sample.maxLodOverlapMeters,
          visibleGroundUncoveredCount: sample.visibleGroundUncoveredCount,
          screenVoidSuspicious: sample.screenVoidSuspicious,
        }));
    }

    function countBlockingSeamGaps(samples: readonly {
      phase: "move" | "settle";
      pendingChunks: number;
      pendingMeshJobs: number;
      dirtyResidentChunks: number;
      lodPendingChunks: number;
      seamGapCount: number;
      lodOverlapCount: number;
      visibleGroundUncoveredCount: number;
      screenVoidSuspicious: boolean;
    }[]) {
      return samples.filter((sample) =>
        sample.seamGapCount > 0
        && (
          (
            sample.phase === "settle"
            && sample.pendingChunks === 0
            && sample.pendingMeshJobs === 0
            && sample.dirtyResidentChunks === 0
            && sample.lodPendingChunks === 0
          )
          || sample.visibleGroundUncoveredCount > 0
          || sample.screenVoidSuspicious
        )).length;
    }

    function countBlockingLodOverlaps(samples: readonly {
      phase: "move" | "settle";
      pendingChunks: number;
      pendingMeshJobs: number;
      dirtyResidentChunks: number;
      lodPendingChunks: number;
      lodOverlapCount: number;
    }[]) {
      return samples.filter((sample) =>
        sample.lodOverlapCount > 0
        && (
          sample.pendingChunks === 0
          && sample.pendingMeshJobs === 0
          && sample.dirtyResidentChunks === 0
          && sample.lodPendingChunks === 0
        )).length;
    }

    function summarizeSlowFrameSamples(samples: readonly {
      frame: number;
      phase: "move" | "settle";
      simTimeSeconds: number;
      routeDistanceMeters: number;
      feetPosition: readonly number[];
      gameplayFrameMs: number;
      accountedFrameMs: number;
      unmeasuredFrameMs: number;
      movementMs: number;
      streamMs: number;
      meshMs: number;
      lodMs: number;
      renderCpuMs: number;
      renderSyncMs: number;
      renderUploadMs: number;
      renderEncodeMs: number;
      pendingChunks: number;
      pendingMeshJobs: number;
      dirtyResidentChunks: number;
      lodPendingChunks: number;
    }[]) {
      return [...samples]
        .sort((left, right) => right.gameplayFrameMs - left.gameplayFrameMs)
        .slice(0, 6)
        .map((sample) => ({
          frame: sample.frame,
          phase: sample.phase,
          simTimeSeconds: sample.simTimeSeconds,
          routeDistanceMeters: sample.routeDistanceMeters,
          feetPosition: sample.feetPosition.slice(0, 3),
          gameplayFrameMs: sample.gameplayFrameMs,
          accountedFrameMs: sample.accountedFrameMs,
          unmeasuredFrameMs: sample.unmeasuredFrameMs,
          movementMs: sample.movementMs,
          streamMs: sample.streamMs,
          meshMs: sample.meshMs,
          lodMs: sample.lodMs,
          renderCpuMs: sample.renderCpuMs,
          renderSyncMs: sample.renderSyncMs,
          renderUploadMs: sample.renderUploadMs,
          renderEncodeMs: sample.renderEncodeMs,
          pendingChunks: sample.pendingChunks,
          pendingMeshJobs: sample.pendingMeshJobs,
          dirtyResidentChunks: sample.dirtyResidentChunks,
          lodPendingChunks: sample.lodPendingChunks,
        }));
    }

    function summarizeTransitionProbe(rawProbe: unknown) {
      const probe = rawProbe as Record<string, unknown>;
      return {
        before: summarizeResidentWorldProbe(probe.before as Record<string, unknown> | undefined),
        after: summarizeResidentWorldProbe(probe.after as Record<string, unknown> | undefined),
        generatedChunkCount: Array.isArray(probe.generatedChunkCoords) ? probe.generatedChunkCoords.length : 0,
        evictedChunkCount: Array.isArray(probe.evictedChunkCoords) ? probe.evictedChunkCoords.length : 0,
        generatedChunkCoords: Array.isArray(probe.generatedChunkCoords) ? probe.generatedChunkCoords.slice(0, 64) : [],
        evictedChunkCoords: Array.isArray(probe.evictedChunkCoords) ? probe.evictedChunkCoords.slice(0, 64) : [],
        residency: probe.residency,
        mesh: probe.mesh,
        render: probe.render,
        settleFrames: probe.settleFrames,
        settled: probe.settled,
      };
    }

    function summarizeResidentWorldProbe(snapshot: Record<string, unknown> | undefined) {
      if (!snapshot) {
        return null;
      }
      const chunks = Array.isArray(snapshot.chunks) ? snapshot.chunks : [];
      let solidChunkCount = 0;
      let minSolidCount = Infinity;
      let maxSolidCount = 0;
      for (const chunk of chunks) {
        if (!chunk || typeof chunk !== "object") continue;
        const solidCount = Number((chunk as Record<string, unknown>).solidCount ?? 0);
        if (solidCount > 0) {
          solidChunkCount += 1;
          minSolidCount = Math.min(minSolidCount, solidCount);
          maxSolidCount = Math.max(maxSolidCount, solidCount);
        }
      }
      return {
        chunkSize: snapshot.chunkSize,
        solidVoxelCount: snapshot.solidVoxelCount,
        chunkCount: snapshot.chunkCount,
        solidChunkCount,
        emptyChunkCount: chunks.length - solidChunkCount,
        minSolidCount: Number.isFinite(minSolidCount) ? minSolidCount : 0,
        maxSolidCount,
        sampleChunks: chunks.slice(0, 8),
      };
    }
  }})(${JSON.stringify({
    settleMaxFrames: options.settleMaxFrames,
    lodRadiusMeters: options.lodRadiusMeters,
    lodStepMeters: options.lodStepMeters,
    renderRadiusMeters: options.renderRadiusMeters,
    renderStepMeters: options.renderStepMeters,
    visibleForwardMeters: options.visibleForwardMeters,
    visibleLateralMeters: options.visibleLateralMeters,
    visibleStepMeters: options.visibleStepMeters,
  })})`;
}

function findFailures(pageReport: Record<string, unknown>, hudSmoke: Record<string, unknown>): string[] {
  const failures: string[] = [];
  if (hudSmoke.passed !== true) {
    failures.push(`HUD smoke failed: ${String(hudSmoke.reason ?? "unknown")}`);
  }
  if (pageReport.hasGameApi !== true) {
    failures.push("window.__VOXELS_GAME__ was not available");
    return failures;
  }
  if (pageReport.webgpuAvailable !== true) {
    failures.push("navigator.gpu was not available");
  }
  const after = readRecord(pageReport.after);
  const bootstrapBenchmark = readRecord(pageReport.bootstrapBenchmark);
  if ((readNumber(after, "chunkCount") ?? 0) <= 0) {
    failures.push("settled page has no resident chunks");
  }
  if (!readStringField(after, "ambientProfileId")) {
    failures.push("settled page did not expose an ambient profile");
  }
  if ((readNumber(after, "ambientFogEndMeters") ?? 0) <= 0) {
    failures.push("settled page did not expose a valid ambient fog distance");
  }
  if (!readStringField(after, "focusSkillName")) {
    failures.push("settled page did not expose a focus skill");
  }
  if ((readNumber(after, "totalSkillLevel") ?? 0) <= 0) {
    failures.push("settled page did not expose valid skill levels");
  }
  if ((readNumber(after, "totalSkillTravelMeters") ?? -1) < 0) {
    failures.push("settled page did not expose valid skill travel meters");
  }
  if (bootstrapBenchmark.completed !== true) {
    failures.push("bootstrap benchmark did not complete");
  }
  const bootstrapPlayableMs = readNumber(bootstrapBenchmark, "playableReadyElapsedMs");
  const bootstrapVisualMs = readNumber(bootstrapBenchmark, "visualReadyElapsedMs");
  if (bootstrapPlayableMs === null) {
    failures.push("bootstrap benchmark did not record playable readiness");
  } else if (bootstrapPlayableMs > PERFORMANCE_BUDGETS.maxBootstrapPlayableMs) {
    failures.push(`bootstrap playable readiness ${bootstrapPlayableMs.toFixed(0)} ms exceeds ${PERFORMANCE_BUDGETS.maxBootstrapPlayableMs} ms`);
  }
  if (bootstrapVisualMs === null) {
    failures.push("bootstrap benchmark did not record visual readiness");
  } else if (bootstrapVisualMs > PERFORMANCE_BUDGETS.maxBootstrapVisualMs) {
    failures.push(`bootstrap visual readiness ${bootstrapVisualMs.toFixed(0)} ms exceeds ${PERFORMANCE_BUDGETS.maxBootstrapVisualMs} ms`);
  }
  const bootstrapP95FrameMs = readNumber(bootstrapBenchmark, "p95GameplayFrameMs") ?? 0;
  if (bootstrapP95FrameMs > PERFORMANCE_BUDGETS.maxBootstrapP95GameplayFrameMs) {
    failures.push(`bootstrap p95 gameplay frame ${bootstrapP95FrameMs.toFixed(2)} ms exceeds ${PERFORMANCE_BUDGETS.maxBootstrapP95GameplayFrameMs} ms`);
  }
  if ((readNumber(after, "lodChunkCount") ?? 0) <= 0) {
    failures.push("settled page has no renderable LOD chunks");
  }
  if ((readNumber(after, "lodPendingChunks") ?? 0) > 0) {
    failures.push(`settled page still has ${readNumber(after, "lodPendingChunks")} pending LOD chunks`);
  }
  const avgFrameCpuMs = readNumber(after, "avgFrameCpuMs") ?? 0;
  const lastFrameCpuMs = readNumber(after, "lastFrameCpuMs") ?? 0;
  const renderStageMs = (readNumber(after, "lastFrameSyncMs") ?? 0)
    + (readNumber(after, "lastFrameUploadMs") ?? 0)
    + (readNumber(after, "lastFrameEncodeMs") ?? 0);
  const drawCalls = readNumber(after, "drawCalls") ?? 0;
  const triangles = readNumber(after, "triangles") ?? 0;
  if (avgFrameCpuMs > PERFORMANCE_BUDGETS.maxAvgFrameCpuMs) {
    failures.push(`avg CPU frame ${avgFrameCpuMs.toFixed(2)} ms exceeds ${PERFORMANCE_BUDGETS.maxAvgFrameCpuMs} ms`);
  }
  if (lastFrameCpuMs > PERFORMANCE_BUDGETS.maxLastFrameCpuMs) {
    failures.push(`last CPU frame ${lastFrameCpuMs.toFixed(2)} ms exceeds ${PERFORMANCE_BUDGETS.maxLastFrameCpuMs} ms`);
  }
  if (renderStageMs > PERFORMANCE_BUDGETS.maxRenderStageMs) {
    failures.push(`render sync/upload/encode ${renderStageMs.toFixed(2)} ms exceeds ${PERFORMANCE_BUDGETS.maxRenderStageMs} ms`);
  }
  if (drawCalls > PERFORMANCE_BUDGETS.maxDrawCalls) {
    failures.push(`draw calls ${drawCalls.toLocaleString()} exceed ${PERFORMANCE_BUDGETS.maxDrawCalls.toLocaleString()}`);
  }
  if (triangles > PERFORMANCE_BUDGETS.maxTriangles) {
    failures.push(`triangles ${triangles.toLocaleString()} exceed ${PERFORMANCE_BUDGETS.maxTriangles.toLocaleString()}`);
  }
  const lodCoverage = readRecord(pageReport.lodCoverage);
  const progressPersistence = readRecord(pageReport.progressPersistence);
  const stagedProgressFixtures = readRecord(pageReport.stagedProgressFixtures);
  const traversalSkillMovement = readRecord(pageReport.traversalSkillMovement);
  const objectiveBreadcrumb = readRecord(pageReport.objectiveBreadcrumb);
  const traversalBudget = readRecord(pageReport.traversalBudget);
  const routeBudget = readRecord(pageReport.routeBudget);
  if (progressPersistence.importRoundTripMatches !== true) {
    failures.push("progress import round-trip did not restore exported state");
  }
  if (progressPersistence.storageRoundTripMatches !== true) {
    failures.push("progress storage round-trip did not restore exported state");
  }
  if (progressPersistence.flavorRoundTripMatches !== true) {
    failures.push("progress import/export did not preserve flavored discovery metadata");
  }
  if (progressPersistence.saved !== true || progressPersistence.loaded !== true) {
    failures.push("progress localStorage save/load API failed");
  }
  const fixtureCases = Array.isArray(stagedProgressFixtures.cases) ? stagedProgressFixtures.cases : [];
  const frontierFixture = readRecord(fixtureCases.find((entry) => readRecord(entry).id === "frontier-old-road-1"));
  const deepFixture = readRecord(fixtureCases.find((entry) => readRecord(entry).id === "deep-old-road-3"));
  const frontierOldRoad = readRecord(frontierFixture.oldRoadObjective);
  const deepOldRoad = readRecord(deepFixture.oldRoadObjective);
  if (stagedProgressFixtures.restoredMatches !== true) {
    failures.push("staged progress fixture probe did not restore the original progress state");
  }
  if (readStringField(frontierFixture, "stageId") !== "frontier-atlas") {
    failures.push(`frontier progress fixture reached ${readStringField(frontierFixture, "stageId") ?? "unknown"} instead of frontier-atlas`);
  }
  if ((readNumber(frontierOldRoad, "progress") ?? -1) !== 1 || (readNumber(frontierOldRoad, "target") ?? -1) !== 2) {
    failures.push(`frontier progress fixture old-road objective was ${formatNumber(readNumber(frontierOldRoad, "progress"))}/${formatNumber(readNumber(frontierOldRoad, "target"))}, expected 1/2`);
  }
  if (readStringField(deepFixture, "stageId") !== "deep-expedition") {
    failures.push(`deep progress fixture reached ${readStringField(deepFixture, "stageId") ?? "unknown"} instead of deep-expedition`);
  }
  if ((readNumber(deepOldRoad, "progress") ?? -1) !== 3 || (readNumber(deepOldRoad, "target") ?? -1) !== 4) {
    failures.push(`deep progress fixture old-road objective was ${formatNumber(readNumber(deepOldRoad, "progress"))}/${formatNumber(readNumber(deepOldRoad, "target"))}, expected 3/4`);
  }
  const frontierScanRadius = readNumber(frontierFixture, "landmarkScanRadiusMeters");
  const deepScanRadius = readNumber(deepFixture, "landmarkScanRadiusMeters");
  const frontierScanSamples = readNumber(frontierFixture, "landmarkScanSampleCount");
  const deepScanSamples = readNumber(deepFixture, "landmarkScanSampleCount");
  if (frontierScanRadius === null || deepScanRadius === null || deepScanRadius <= frontierScanRadius) {
    failures.push(`naturalist skill effect did not expand landmark scan radius (${formatNumber(frontierScanRadius)} -> ${formatNumber(deepScanRadius)})`);
  }
  if (frontierScanSamples === null || deepScanSamples === null || deepScanSamples <= frontierScanSamples) {
    failures.push(`naturalist skill effect did not expand landmark scan samples (${formatNumber(frontierScanSamples)} -> ${formatNumber(deepScanSamples)})`);
  }
  const frontierSurfaceSpeed = readNumber(frontierFixture, "surfaceTravelSpeedMultiplier");
  const deepSurfaceSpeed = readNumber(deepFixture, "surfaceTravelSpeedMultiplier");
  const frontierUndergroundSpeed = readNumber(frontierFixture, "undergroundTravelSpeedMultiplier");
  const deepUndergroundSpeed = readNumber(deepFixture, "undergroundTravelSpeedMultiplier");
  if (frontierSurfaceSpeed === null || deepSurfaceSpeed === null || deepSurfaceSpeed <= frontierSurfaceSpeed) {
    failures.push(`cartography skill effect did not expand surface travel speed (${formatNumber(frontierSurfaceSpeed)} -> ${formatNumber(deepSurfaceSpeed)})`);
  }
  if (frontierUndergroundSpeed === null || deepUndergroundSpeed === null || deepUndergroundSpeed <= frontierUndergroundSpeed) {
    failures.push(`spelunking skill effect did not expand underground travel speed (${formatNumber(frontierUndergroundSpeed)} -> ${formatNumber(deepUndergroundSpeed)})`);
  }
  const noviceSkillMovement = readRecord(traversalSkillMovement.novice);
  const veteranSkillMovement = readRecord(traversalSkillMovement.veteran);
  const noviceSkillDistance = readNumber(noviceSkillMovement, "totalDistanceMeters");
  const veteranSkillDistance = readNumber(veteranSkillMovement, "totalDistanceMeters");
  const noviceSkillSpeed = readNumber(noviceSkillMovement, "surfaceTravelSpeedMultiplier");
  const veteranSkillSpeed = readNumber(veteranSkillMovement, "surfaceTravelSpeedMultiplier");
  if (traversalSkillMovement.restoredMatches !== true) {
    failures.push("traversal skill movement probe did not restore the original progress state");
  }
  if (noviceSkillSpeed === null || veteranSkillSpeed === null || veteranSkillSpeed <= noviceSkillSpeed) {
    failures.push(`traversal skill movement probe did not expose a higher surface speed multiplier (${formatNumber(noviceSkillSpeed)} -> ${formatNumber(veteranSkillSpeed)})`);
  }
  if (noviceSkillDistance === null || veteranSkillDistance === null || veteranSkillDistance <= noviceSkillDistance * 1.03) {
    failures.push(`boosted traversal probe did not move at least 3% farther (${formatNumber(noviceSkillDistance)} m -> ${formatNumber(veteranSkillDistance)} m)`);
  }
  const beforeBreadcrumbObjective = readRecord(objectiveBreadcrumb.beforeObjective);
  const afterBreadcrumbObjective = readRecord(objectiveBreadcrumb.afterObjective);
  const beforeBreadcrumbProgress = readNumber(beforeBreadcrumbObjective, "progress");
  const afterBreadcrumbProgress = readNumber(afterBreadcrumbObjective, "progress");
  const beforeAncient = readNumber(objectiveBreadcrumb, "beforeAncient");
  const afterAncient = readNumber(objectiveBreadcrumb, "afterAncient");
  if (beforeBreadcrumbProgress === null) {
    failures.push("objective breadcrumb probe did not expose the old-road objective before travel");
  }
  if (afterBreadcrumbProgress === null) {
    failures.push("objective breadcrumb probe did not expose the old-road objective after travel");
  }
  if (beforeBreadcrumbProgress !== null && afterBreadcrumbProgress !== null && afterBreadcrumbProgress <= beforeBreadcrumbProgress) {
    failures.push(`objective breadcrumb progress did not advance (${beforeBreadcrumbProgress} -> ${afterBreadcrumbProgress})`);
  }
  if (beforeAncient !== null && afterAncient !== null && afterAncient <= beforeAncient) {
    failures.push(`ancient landmark count did not advance during objective breadcrumb probe (${beforeAncient} -> ${afterAncient})`);
  }
  if (readStringField(objectiveBreadcrumb, "currentLandmarkId") !== "silt_shell") {
    failures.push(`objective breadcrumb probe landed at ${readStringField(objectiveBreadcrumb, "currentLandmarkId") ?? "unknown"} instead of silt_shell`);
  }
  const traversalDistance = readNumber(traversalBudget, "totalDistanceMeters") ?? 0;
  const traversalP95GameplayFrameMs = readNumber(traversalBudget, "p95GameplayFrameMs") ?? 0;
  const traversalMaxGameplayFrameMs = readNumber(traversalBudget, "maxGameplayFrameMs") ?? 0;
  const traversalP95MeasuredWorkMs = readNumber(traversalBudget, "p95MeasuredWorkMs") ?? 0;
  const traversalP95RenderCpuMs = readNumber(traversalBudget, "p95RenderCpuMs") ?? 0;
  const traversalP95StreamMs = readNumber(traversalBudget, "p95StreamMs") ?? 0;
  const traversalP95MeshMs = readNumber(traversalBudget, "p95MeshMs") ?? 0;
  const traversalP95LodMs = readNumber(traversalBudget, "p95LodMs") ?? 0;
  if (traversalDistance <= 1) {
    failures.push(`traversal benchmark only moved ${traversalDistance.toFixed(2)} m`);
  }
  if (traversalP95GameplayFrameMs > PERFORMANCE_BUDGETS.maxTraversalP95GameplayFrameMs) {
    failures.push(`traversal p95 gameplay frame ${traversalP95GameplayFrameMs.toFixed(2)} ms exceeds ${PERFORMANCE_BUDGETS.maxTraversalP95GameplayFrameMs} ms`);
  }
  if (traversalMaxGameplayFrameMs > PERFORMANCE_BUDGETS.maxTraversalMaxGameplayFrameMs) {
    failures.push(`traversal max gameplay frame ${traversalMaxGameplayFrameMs.toFixed(2)} ms exceeds ${PERFORMANCE_BUDGETS.maxTraversalMaxGameplayFrameMs} ms`);
  }
  if (traversalP95MeasuredWorkMs > PERFORMANCE_BUDGETS.maxTraversalP95MeasuredWorkMs) {
    failures.push(`traversal p95 measured work ${traversalP95MeasuredWorkMs.toFixed(2)} ms exceeds ${PERFORMANCE_BUDGETS.maxTraversalP95MeasuredWorkMs} ms`);
  }
  if (traversalP95RenderCpuMs > PERFORMANCE_BUDGETS.maxTraversalP95RenderCpuMs) {
    failures.push(`traversal p95 render CPU ${traversalP95RenderCpuMs.toFixed(2)} ms exceeds ${PERFORMANCE_BUDGETS.maxTraversalP95RenderCpuMs} ms`);
  }
  if (traversalP95StreamMs > PERFORMANCE_BUDGETS.maxTraversalP95StreamMs) {
    failures.push(`traversal p95 stream ${traversalP95StreamMs.toFixed(2)} ms exceeds ${PERFORMANCE_BUDGETS.maxTraversalP95StreamMs} ms`);
  }
  if (traversalP95MeshMs > PERFORMANCE_BUDGETS.maxTraversalP95MeshMs) {
    failures.push(`traversal p95 mesh ${traversalP95MeshMs.toFixed(2)} ms exceeds ${PERFORMANCE_BUDGETS.maxTraversalP95MeshMs} ms`);
  }
  if (traversalP95LodMs > PERFORMANCE_BUDGETS.maxTraversalP95LodMs) {
    failures.push(`traversal p95 LOD ${traversalP95LodMs.toFixed(2)} ms exceeds ${PERFORMANCE_BUDGETS.maxTraversalP95LodMs} ms`);
  }
  if ((readNumber(traversalBudget, "framesWithHoleSignals") ?? 0) > 0) {
    failures.push(`traversal benchmark had ${readNumber(traversalBudget, "framesWithHoleSignals")} hole-signal frames`);
  }
  if ((readNumber(traversalBudget, "framesWithVisibleGroundGaps") ?? 0) > 0) {
    failures.push(`traversal benchmark had ${readNumber(traversalBudget, "framesWithVisibleGroundGaps")} visible-ground gap frames`);
  }
  if ((readNumber(traversalBudget, "blockingSeamGapCount") ?? 0) > 0) {
    failures.push(`traversal benchmark had ${readNumber(traversalBudget, "blockingSeamGapCount")} visible or settled seam-gap frames`);
  }
  if ((readNumber(traversalBudget, "blockingLodOverlapCount") ?? 0) > 0) {
    failures.push(`traversal benchmark had ${readNumber(traversalBudget, "blockingLodOverlapCount")} settled LOD-overlap frames`);
  }
  const traversalScreenCaptures = readNumber(traversalBudget, "screenVoidCaptureCount") ?? 0;
  if (traversalScreenCaptures < PERFORMANCE_BUDGETS.minTraversalScreenCaptures) {
    failures.push(`traversal benchmark captured ${traversalScreenCaptures} screen frames, expected at least ${PERFORMANCE_BUDGETS.minTraversalScreenCaptures}`);
  }
  const routeDistance = readNumber(routeBudget, "totalDistanceMeters") ?? 0;
  const routeP95GameplayFrameMs = readNumber(routeBudget, "p95GameplayFrameMs") ?? 0;
  const routeMaxGameplayFrameMs = readNumber(routeBudget, "maxGameplayFrameMs") ?? 0;
  const routeP95MeasuredWorkMs = readNumber(routeBudget, "p95MeasuredWorkMs") ?? 0;
  const routeP95RenderCpuMs = readNumber(routeBudget, "p95RenderCpuMs") ?? 0;
  const routeP95StreamMs = readNumber(routeBudget, "p95StreamMs") ?? 0;
  const routeP95MeshMs = readNumber(routeBudget, "p95MeshMs") ?? 0;
  const routeP95LodMs = readNumber(routeBudget, "p95LodMs") ?? 0;
  if (routeDistance < 18) {
    failures.push(`route benchmark only covered ${routeDistance.toFixed(2)} m`);
  }
  if ((readNumber(routeBudget, "moveFrameCount") ?? 0) < 120) {
    failures.push(`route benchmark only collected ${readNumber(routeBudget, "moveFrameCount") ?? 0} movement frames`);
  }
  if ((readNumber(routeBudget, "settleFrameCount") ?? 0) <= 0) {
    failures.push("route benchmark did not collect settle frames");
  }
  if (routeP95GameplayFrameMs > PERFORMANCE_BUDGETS.maxRouteP95GameplayFrameMs) {
    failures.push(`route p95 gameplay frame ${routeP95GameplayFrameMs.toFixed(2)} ms exceeds ${PERFORMANCE_BUDGETS.maxRouteP95GameplayFrameMs} ms`);
  }
  if (routeMaxGameplayFrameMs > PERFORMANCE_BUDGETS.maxRouteMaxGameplayFrameMs) {
    failures.push(`route max gameplay frame ${routeMaxGameplayFrameMs.toFixed(2)} ms exceeds ${PERFORMANCE_BUDGETS.maxRouteMaxGameplayFrameMs} ms`);
  }
  if (routeP95MeasuredWorkMs > PERFORMANCE_BUDGETS.maxRouteP95MeasuredWorkMs) {
    failures.push(`route p95 measured work ${routeP95MeasuredWorkMs.toFixed(2)} ms exceeds ${PERFORMANCE_BUDGETS.maxRouteP95MeasuredWorkMs} ms`);
  }
  if (routeP95RenderCpuMs > PERFORMANCE_BUDGETS.maxRouteP95RenderCpuMs) {
    failures.push(`route p95 render CPU ${routeP95RenderCpuMs.toFixed(2)} ms exceeds ${PERFORMANCE_BUDGETS.maxRouteP95RenderCpuMs} ms`);
  }
  if (routeP95StreamMs > PERFORMANCE_BUDGETS.maxRouteP95StreamMs) {
    failures.push(`route p95 stream ${routeP95StreamMs.toFixed(2)} ms exceeds ${PERFORMANCE_BUDGETS.maxRouteP95StreamMs} ms`);
  }
  if (routeP95MeshMs > PERFORMANCE_BUDGETS.maxRouteP95MeshMs) {
    failures.push(`route p95 mesh ${routeP95MeshMs.toFixed(2)} ms exceeds ${PERFORMANCE_BUDGETS.maxRouteP95MeshMs} ms`);
  }
  if (routeP95LodMs > PERFORMANCE_BUDGETS.maxRouteP95LodMs) {
    failures.push(`route p95 LOD ${routeP95LodMs.toFixed(2)} ms exceeds ${PERFORMANCE_BUDGETS.maxRouteP95LodMs} ms`);
  }
  if ((readNumber(routeBudget, "framesWithHoleSignals") ?? 0) > 0) {
    failures.push(`route benchmark had ${readNumber(routeBudget, "framesWithHoleSignals")} hole-signal frames`);
  }
  if ((readNumber(routeBudget, "framesWithVisibleGroundGaps") ?? 0) > 0) {
    failures.push(`route benchmark had ${readNumber(routeBudget, "framesWithVisibleGroundGaps")} visible-ground gap frames`);
  }
  if ((readNumber(routeBudget, "blockingSeamGapCount") ?? 0) > 0) {
    failures.push(`route benchmark had ${readNumber(routeBudget, "blockingSeamGapCount")} visible or settled seam-gap frames`);
  }
  if ((readNumber(routeBudget, "blockingLodOverlapCount") ?? 0) > 0) {
    failures.push(`route benchmark had ${readNumber(routeBudget, "blockingLodOverlapCount")} settled LOD-overlap frames`);
  }
  const routeScreenCaptures = readNumber(routeBudget, "screenVoidCaptureCount") ?? 0;
  if (routeScreenCaptures < PERFORMANCE_BUDGETS.minRouteScreenCaptures) {
    failures.push(`route benchmark captured ${routeScreenCaptures} screen frames, expected at least ${PERFORMANCE_BUDGETS.minRouteScreenCaptures}`);
  }
  if ((readNumber(routeBudget, "maxVisibleGroundUncoveredCount") ?? 0) > 0) {
    failures.push(`route benchmark max visible-ground uncovered samples was ${readNumber(routeBudget, "maxVisibleGroundUncoveredCount")}`);
  }
  const uncoveredGapCount = readNumber(lodCoverage, "uncoveredGapCount") ?? 0;
  const handoffHoleCount = readNumber(lodCoverage, "handoffHoleCount") ?? 0;
  const residentOverlapCount = readNumber(lodCoverage, "residentOverlapCount") ?? 0;
  const bandOverlapCount = readNumber(lodCoverage, "bandOverlapCount") ?? 0;
  if (residentOverlapCount > 0) {
    failures.push(`LOD coverage renders over ${residentOverlapCount} sampled render-ready LOD0 columns`);
  }
  if (bandOverlapCount > 0) {
    failures.push(`LOD coverage has ${bandOverlapCount} sampled overlapping LOD bands`);
  }
  if (uncoveredGapCount > 0) {
    failures.push(`LOD coverage has ${uncoveredGapCount} uncovered sampled gaps`);
  }
  if (handoffHoleCount > 0) {
    failures.push(`LOD handoff has ${handoffHoleCount} resident-but-uncovered sampled holes`);
  }
  return failures;
}

function printSummary(reportPath: string, report: {
  hudSmoke: Record<string, unknown>;
  visualIdentity?: Record<string, unknown> | null;
  page: Record<string, unknown>;
  failures: string[];
  screenshotPath: string | null;
}): void {
  const page = report.page;
  const after = readRecord(page.after);
  const lodCoverage = readRecord(page.lodCoverage);
  const renderReady = readRecord(page.renderReady);
  const bootstrapBenchmark = readRecord(page.bootstrapBenchmark);
  const progressPersistence = readRecord(page.progressPersistence);
  const stagedProgressFixtures = readRecord(page.stagedProgressFixtures);
  const traversalSkillMovement = readRecord(page.traversalSkillMovement);
  const objectiveBreadcrumb = readRecord(page.objectiveBreadcrumb);
  const traversalBudget = readRecord(page.traversalBudget);
  const routeBudget = readRecord(page.routeBudget);
  console.log(`owned-browser-lab report: ${reportPath}`);
  if (report.screenshotPath) {
    console.log(`screenshot: ${report.screenshotPath}`);
  }
  console.log(`chunks: ${formatNumber(readNumber(after, "chunkCount"))}`);
  console.log(`LOD chunks: ${formatNumber(readNumber(after, "lodChunkCount"))}`);
  console.log(`LOD pending: ${formatNumber(readNumber(after, "lodPendingChunks"))}`);
  console.log(`ambient: ${readStringField(after, "ambientProfileId") ?? "unknown"} (${formatNumber(readNumber(after, "ambientFogEndMeters"))} m fog)`);
  console.log(`focus skill: ${readStringField(after, "focusSkillName") ?? "unknown"} ${formatNumber(readNumber(after, "focusSkillLevel"))}`);
  console.log(`bootstrap playable/visual: ${formatNumber(readNumber(bootstrapBenchmark, "playableReadyElapsedMs"))}/${formatNumber(readNumber(bootstrapBenchmark, "visualReadyElapsedMs"))} ms`);
  console.log(`bootstrap p95/max frame: ${formatNumber(readNumber(bootstrapBenchmark, "p95GameplayFrameMs"))}/${formatNumber(readNumber(bootstrapBenchmark, "maxGameplayFrameMs"))} ms`);
  console.log(`bootstrap p95 LOD/stream/mesh/render: ${formatNumber(readNumber(bootstrapBenchmark, "p95LodMs"))}/${formatNumber(readNumber(bootstrapBenchmark, "p95StreamMs"))}/${formatNumber(readNumber(bootstrapBenchmark, "p95MeshMs"))}/${formatNumber(readNumber(bootstrapBenchmark, "p95RenderCpuMs"))} ms`);
  console.log(`performance CPU avg/last: ${formatNumber(readNumber(after, "avgFrameCpuMs"))}/${formatNumber(readNumber(after, "lastFrameCpuMs"))} ms`);
  console.log(`performance draw/triangles: ${formatNumber(readNumber(after, "drawCalls"))}/${formatNumber(readNumber(after, "triangles"))}`);
  if (report.visualIdentity && typeof report.visualIdentity.error !== "string") {
    console.log(`visual identity saturation/grid/color: ${formatNumber(readNumber(report.visualIdentity, "avgSaturation"))}/${formatNumber(readNumber(report.visualIdentity, "axisAlignedEdgeDominance"))}/${formatNumber(readNumber(report.visualIdentity, "quantizedColorCount"))}`);
  }
  console.log(`progress persistence: ${progressPersistence.importRoundTripMatches === true && progressPersistence.storageRoundTripMatches === true && progressPersistence.flavorRoundTripMatches === true ? "passed" : "failed"}`);
  console.log(`staged progress fixtures: ${stagedProgressFixtures.restoredMatches === true ? "passed" : "failed"}`);
  console.log(`landmark scan radius/samples: ${formatNumber(readNumber(after, "landmarkScanRadiusMeters"))} m / ${formatNumber(readNumber(after, "landmarkScanSampleCount"))}`);
  console.log(`travel speed surface/underground: ${formatNumber(readNumber(after, "surfaceTravelSpeedMultiplier"))}x / ${formatNumber(readNumber(after, "undergroundTravelSpeedMultiplier"))}x`);
  console.log(`travel skill movement: ${formatNumber(readNumber(readRecord(traversalSkillMovement.novice), "totalDistanceMeters"))} m -> ${formatNumber(readNumber(readRecord(traversalSkillMovement.veteran), "totalDistanceMeters"))} m`);
  console.log(`objective breadcrumb: ${formatNumber(readNumber(readRecord(objectiveBreadcrumb.beforeObjective), "progress"))} -> ${formatNumber(readNumber(readRecord(objectiveBreadcrumb.afterObjective), "progress"))} at ${readStringField(objectiveBreadcrumb, "currentLandmarkId") ?? "unknown"}`);
  console.log(`traversal p95/max frame: ${formatNumber(readNumber(traversalBudget, "p95GameplayFrameMs"))}/${formatNumber(readNumber(traversalBudget, "maxGameplayFrameMs"))} ms`);
  console.log(`traversal p95 work/move/render/stream/mesh/LOD: ${formatNumber(readNumber(traversalBudget, "p95MeasuredWorkMs"))}/${formatNumber(readNumber(traversalBudget, "p95MovementMs"))}/${formatNumber(readNumber(traversalBudget, "p95RenderCpuMs"))}/${formatNumber(readNumber(traversalBudget, "p95StreamMs"))}/${formatNumber(readNumber(traversalBudget, "p95MeshMs"))}/${formatNumber(readNumber(traversalBudget, "p95LodMs"))} ms`);
  console.log(`traversal diagnostics avg/total/capture: ${formatNumber(readNumber(traversalBudget, "avgDiagnosticsMs"))}/${formatNumber(readNumber(traversalBudget, "totalDiagnosticsMs"))}/${formatNumber(readNumber(traversalBudget, "totalCaptureDiagnosticsMs"))} ms`);
  console.log(`traversal distance: ${formatNumber(readNumber(traversalBudget, "totalDistanceMeters"))} m, holes/seams/blocking seams/overlaps: ${formatNumber(readNumber(traversalBudget, "framesWithHoleSignals"))}/${formatNumber(readNumber(traversalBudget, "framesWithSeamGaps"))}/${formatNumber(readNumber(traversalBudget, "blockingSeamGapCount"))}/${formatNumber(readNumber(traversalBudget, "blockingLodOverlapCount"))}`);
  console.log(`traversal screen captures: ${formatNumber(readNumber(traversalBudget, "screenVoidCaptureCount"))}`);
  console.log(`route p95/max frame: ${formatNumber(readNumber(routeBudget, "p95GameplayFrameMs"))}/${formatNumber(readNumber(routeBudget, "maxGameplayFrameMs"))} ms`);
  console.log(`route p95 work/move/render/stream/mesh/LOD: ${formatNumber(readNumber(routeBudget, "p95MeasuredWorkMs"))}/${formatNumber(readNumber(routeBudget, "p95MovementMs"))}/${formatNumber(readNumber(routeBudget, "p95RenderCpuMs"))}/${formatNumber(readNumber(routeBudget, "p95StreamMs"))}/${formatNumber(readNumber(routeBudget, "p95MeshMs"))}/${formatNumber(readNumber(routeBudget, "p95LodMs"))} ms`);
  console.log(`route diagnostics avg/total/capture: ${formatNumber(readNumber(routeBudget, "avgDiagnosticsMs"))}/${formatNumber(readNumber(routeBudget, "totalDiagnosticsMs"))}/${formatNumber(readNumber(routeBudget, "totalCaptureDiagnosticsMs"))} ms`);
  console.log(`route distance: ${formatNumber(readNumber(routeBudget, "totalDistanceMeters"))} m, holes/seams/blocking seams/overlaps: ${formatNumber(readNumber(routeBudget, "framesWithHoleSignals"))}/${formatNumber(readNumber(routeBudget, "framesWithSeamGaps"))}/${formatNumber(readNumber(routeBudget, "blockingSeamGapCount"))}/${formatNumber(readNumber(routeBudget, "blockingLodOverlapCount"))}`);
  if (Array.isArray(routeBudget.seamGapSamples) && routeBudget.seamGapSamples.length > 0) {
    console.log(`route seam sample: ${JSON.stringify(routeBudget.seamGapSamples[0])}`);
  }
  console.log(`route screen captures: ${formatNumber(readNumber(routeBudget, "screenVoidCaptureCount"))}`);
  console.log(`LOD overlap LOD0/bands: ${formatNumber(readNumber(lodCoverage, "residentOverlapCount"))}/${formatNumber(readNumber(lodCoverage, "bandOverlapCount"))}`);
  console.log(`LOD gaps: ${formatNumber(readNumber(lodCoverage, "uncoveredGapCount"))}`);
  console.log(`LOD handoff holes: ${formatNumber(readNumber(lodCoverage, "handoffHoleCount"))}`);
  console.log(`render-ready near samples: ${formatNumber(readNumber(renderReady, "renderReadySampleCount"))}/${formatNumber(readNumber(renderReady, "sampleCount"))}`);
  console.log(`HUD smoke: ${report.hudSmoke.passed === true ? "passed" : "failed"}`);
  if (report.failures.length > 0) {
    console.log(`failures: ${report.failures.join("; ")}`);
  } else {
    console.log("failures: none");
  }
}

function parseCli(argv: readonly string[]): CliOptions {
  const args = argv.slice(2);
  return {
    label: readFlag(args, "--label"),
    outputDir: readFlag(args, "--output-dir") ?? "artifacts/owned-browser-lab",
    serverMode: readServerMode(readFlag(args, "--server")),
    port: readOptionalPositiveInt(readFlag(args, "--port")),
    chromeBinary: readFlag(args, "--chrome-binary") ?? resolveChromeBinary(),
    headless: readBooleanFlag(args, "--headless", true),
    settleMaxFrames: readPositiveInt(readFlag(args, "--settle-max-frames"), 300),
    lodRadiusMeters: readPositiveFloat(readFlag(args, "--lod-radius"), 48),
    lodStepMeters: readPositiveFloat(readFlag(args, "--lod-step"), 1.6),
    renderRadiusMeters: readPositiveFloat(readFlag(args, "--render-radius"), 12),
    renderStepMeters: readPositiveFloat(readFlag(args, "--render-step"), 0.8),
    visibleForwardMeters: readPositiveFloat(readFlag(args, "--visible-forward"), 16),
    visibleLateralMeters: readPositiveFloat(readFlag(args, "--visible-lateral"), 6),
    visibleStepMeters: readPositiveFloat(readFlag(args, "--visible-step"), 0.8),
  };
}

function readServerMode(value: string | null): ServerMode {
  if (value === null) return "prod";
  if (value === "dev" || value === "prod" || value === "existing") return value;
  throw new Error(`Unsupported --server value: ${value}`);
}

function readFlag(args: readonly string[], flag: string): string | null {
  const exact = args.find((arg) => arg.startsWith(`${flag}=`));
  if (exact) return exact.slice(flag.length + 1);
  const index = args.indexOf(flag);
  return index === -1 ? null : args[index + 1] ?? null;
}

function readBooleanFlag(args: readonly string[], flag: string, fallback: boolean): boolean {
  const value = readFlag(args, flag);
  if (value === null) return fallback;
  if (value === "1" || value === "true") return true;
  if (value === "0" || value === "false") return false;
  throw new Error(`Expected ${flag}=true|false, got ${value}`);
}

function readOptionalPositiveInt(raw: string | null): number | null {
  if (raw === null) return null;
  return readPositiveInt(raw, 0);
}

function readPositiveInt(raw: string | null, fallback: number): number {
  const parsed = Number.parseInt(raw ?? "", 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

function readPositiveFloat(raw: string | null, fallback: number): number {
  const parsed = Number.parseFloat(raw ?? "");
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

function runCommand(command: string[]): CommandResult {
  const startedAt = performance.now();
  const result = Bun.spawnSync(command, {
    cwd: process.cwd(),
    stdout: "pipe",
    stderr: "pipe",
    env: process.env,
  });
  return {
    command,
    exitCode: result.exitCode,
    elapsedMs: Number((performance.now() - startedAt).toFixed(2)),
    stdout: new TextDecoder().decode(result.stdout),
    stderr: new TextDecoder().decode(result.stderr),
  };
}

function resolveChromeBinary(): string {
  const candidates = [
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    Bun.which("google-chrome"),
    Bun.which("chrome"),
    Bun.which("chromium"),
    Bun.which("chromium-browser"),
  ];
  for (const candidate of candidates) {
    if (candidate) return candidate;
  }
  throw new Error("Could not find Chrome. Pass --chrome-binary=/path/to/chrome");
}

function buildChromeCommand(chromeBinary: string, devToolsPort: number, profileDir: string, headless: boolean): string[] {
  return [
    chromeBinary,
    ...(headless ? ["--headless=new"] : []),
    `--remote-debugging-port=${devToolsPort}`,
    `--user-data-dir=${profileDir}`,
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-background-networking",
    "--disable-component-update",
    "--disable-sync",
    "--disable-features=DialMediaRouteProvider",
    "--enable-unsafe-webgpu",
    "--enable-features=Vulkan,UseSkiaRenderer",
    "about:blank",
  ];
}

async function findFreePort(): Promise<number> {
  return await new Promise((resolve, reject) => {
    const server = createServer();
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      if (!address || typeof address === "string") {
        server.close();
        reject(new Error("Could not allocate an ephemeral port"));
        return;
      }
      const port = address.port;
      server.close(() => resolve(port));
    });
    server.on("error", reject);
  });
}

async function waitForHttp(url: string, timeoutMs: number): Promise<void> {
  await pollUntil(async () => {
    try {
      const response = await fetch(url, { cache: "no-store" });
      return response.ok;
    } catch {
      return false;
    }
  }, timeoutMs, `Timed out waiting for ${url}`);
}

async function waitForJsonEndpoint<T>(url: string, timeoutMs: number): Promise<T> {
  let lastError: unknown = null;
  return await pollUntil(async () => {
    try {
      const response = await fetch(url);
      if (!response.ok) return null;
      return await response.json() as T;
    } catch (error) {
      lastError = error;
      return null;
    }
  }, timeoutMs, `Timed out waiting for ${url}: ${lastError instanceof Error ? lastError.message : "unknown error"}`);
}

async function createDevToolsTarget(port: number): Promise<DevToolsTargetResponse> {
  const endpoint = `http://127.0.0.1:${port}/json/new?about:blank`;
  const put = await fetch(endpoint, { method: "PUT" });
  if (put.ok) return await put.json() as DevToolsTargetResponse;
  const get = await fetch(endpoint);
  if (!get.ok) throw new Error(`Failed to create DevTools target: PUT ${put.status}, GET ${get.status}`);
  return await get.json() as DevToolsTargetResponse;
}

async function waitForGameApi(cdp: CdpConnection, timeoutMs: number): Promise<void> {
  await pollUntil(async () => {
    return await cdp.evaluate<boolean>("Boolean(window.__VOXELS_GAME__ && window.__VOXELS_GAME__.snapshot)");
  }, timeoutMs, "Timed out waiting for window.__VOXELS_GAME__");
}

async function waitForGameWorldReady(cdp: CdpConnection, timeoutMs: number): Promise<void> {
  await pollUntil(async () => {
    return await cdp.evaluate<boolean>(`(() => {
      const game = window.__VOXELS_GAME__;
      if (!game || typeof game.snapshot !== "function") return false;
      const snapshot = game.snapshot();
      return snapshot.chunkCount > 0 && snapshot.bootstrapPlayableReady === true;
    })()`);
  }, timeoutMs, "Timed out waiting for playable resident world state");
}

async function runHudSmoke(cdp: CdpConnection): Promise<Record<string, unknown>> {
  const before = await cdp.evaluate<Record<string, unknown>>(`(() => {
    const capture = document.querySelector("[data-role='capture']");
    const canvas = document.querySelector("[data-role='viewport']");
    if (
      canvas instanceof HTMLCanvasElement
      && capture instanceof HTMLElement
      && !window.__VOXELS_POINTER_LOCK_PROBE__
    ) {
      window.__VOXELS_POINTER_LOCK_PROBE__ = {
        called: false,
        targetRole: null,
        rejected: false,
        rejectionName: null,
      };
      patchPointerLock(canvas, "canvas");
      patchPointerLock(capture, "capture");
      function patchPointerLock(element, role) {
        const originalRequestPointerLock = element.requestPointerLock.bind(element);
        element.requestPointerLock = async function requestPointerLockProbe(...args) {
          window.__VOXELS_POINTER_LOCK_PROBE__.called = true;
          window.__VOXELS_POINTER_LOCK_PROBE__.targetRole = role;
          try {
            return await originalRequestPointerLock(...args);
          } catch (error) {
            window.__VOXELS_POINTER_LOCK_PROBE__.rejected = true;
            window.__VOXELS_POINTER_LOCK_PROBE__.rejectionName = error instanceof DOMException ? error.name : String(error);
            throw error;
          }
        };
      };
    }
    const text = document.body.innerText;
    return {
      capturePresent: capture instanceof HTMLElement,
      promptVisible: capture instanceof HTMLElement && capture.innerText.includes("Click to play"),
      canvasPresent: canvas instanceof HTMLCanvasElement,
      legacyTextFound: /Show Debug|Hide Debug|No voxel in reach|Targeting|Inventory|Hotbar/.test(text),
      minecraftHudFound: Boolean(
        document.querySelector(".game-hotbar, .game-inventory-panel, .target-overlay"),
      ),
      clickX: Math.round(window.innerWidth / 2),
      clickY: Math.round(window.innerHeight / 2),
    };
  })()`);
  if (before.capturePresent !== true) {
    return { passed: false, reason: "capture overlay missing", before };
  }

  const clickX = readNumber(before, "clickX") ?? 400;
  const clickY = readNumber(before, "clickY") ?? 300;
  await cdp.send("Input.dispatchMouseEvent", {
    type: "mousePressed",
    x: clickX,
    y: clickY,
    button: "left",
    clickCount: 1,
  });
  await cdp.send("Input.dispatchMouseEvent", {
    type: "mouseReleased",
    x: clickX,
    y: clickY,
    button: "left",
    clickCount: 1,
  });
  await Bun.sleep(300);

  const after = await cdp.evaluate<Record<string, unknown>>(`(() => {
    const capture = document.querySelector("[data-role='capture']");
    const game = window.__VOXELS_GAME__;
    const snapshot = game?.snapshot?.();
    const style = capture instanceof HTMLElement ? getComputedStyle(capture) : null;
    const probe = window.__VOXELS_POINTER_LOCK_PROBE__ ?? {};
    return {
      capturePresent: capture instanceof HTMLElement,
      promptVisible: capture instanceof HTMLElement && capture.innerText.includes("Click to play"),
      pointerLocked: snapshot?.pointerLocked === true,
      documentPointerLockElementRole: document.pointerLockElement === capture
        ? "capture"
        : document.pointerLockElement === document.querySelector("[data-role='viewport']")
        ? "canvas"
        : null,
      pointerEvents: style?.pointerEvents ?? null,
      opacity: style?.opacity ?? null,
      legacyTextFound: /Show Debug|Hide Debug|No voxel in reach|Targeting|Inventory|Hotbar/.test(document.body.innerText),
      minecraftHudFound: Boolean(
        document.querySelector(".game-hotbar, .game-inventory-panel, .target-overlay"),
      ),
      requestPointerLockCalled: probe.called === true,
      requestPointerLockTargetRole: probe.targetRole ?? null,
      requestPointerLockRejected: probe.rejected === true,
      requestPointerLockRejectionName: probe.rejectionName ?? null,
      positionBeforeMove: snapshot?.feetPosition ?? null,
      skillTravelMetersBeforeMove: snapshot?.totalSkillTravelMeters ?? null,
    };
  })()`);
  await cdp.send("Input.dispatchKeyEvent", {
    type: "rawKeyDown",
    windowsVirtualKeyCode: 87,
    nativeVirtualKeyCode: 87,
    code: "KeyW",
    key: "w",
  });
  await Bun.sleep(450);
  await cdp.send("Input.dispatchKeyEvent", {
    type: "keyUp",
    windowsVirtualKeyCode: 87,
    nativeVirtualKeyCode: 87,
    code: "KeyW",
    key: "w",
  });
  await Bun.sleep(100);
  const movement = await cdp.evaluate<Record<string, unknown>>(`(() => {
    const game = window.__VOXELS_GAME__;
    const snapshot = game?.snapshot?.();
    const before = ${JSON.stringify(after.positionBeforeMove)};
    const afterPosition = snapshot?.feetPosition ?? null;
    const movedMeters = Array.isArray(before) && Array.isArray(afterPosition)
      ? Math.hypot(afterPosition[0] - before[0], afterPosition[2] - before[2]) / 16
      : 0;
    const beforeSkillTravelMeters = Number(${JSON.stringify(after.skillTravelMetersBeforeMove)});
    const afterSkillTravelMeters = Number(snapshot?.totalSkillTravelMeters ?? 0);
    return {
      positionAfterMove: afterPosition,
      movedMeters,
      skillTravelMetersBeforeMove: Number.isFinite(beforeSkillTravelMeters) ? beforeSkillTravelMeters : null,
      skillTravelMetersAfterMove: Number.isFinite(afterSkillTravelMeters) ? afterSkillTravelMeters : null,
      skillTravelMetersDelta: Number.isFinite(beforeSkillTravelMeters) && Number.isFinite(afterSkillTravelMeters)
        ? afterSkillTravelMeters - beforeSkillTravelMeters
        : null,
      pointerLockedAfterMove: snapshot?.pointerLocked === true,
    };
  })()`);
  const passed = before.legacyTextFound !== true
    && after.legacyTextFound !== true
    && before.minecraftHudFound !== true
    && after.minecraftHudFound !== true
    && after.requestPointerLockCalled === true
    && (after.requestPointerLockTargetRole === "capture" || after.requestPointerLockTargetRole === "canvas")
    && (
      after.documentPointerLockElementRole !== null
      || (after.pointerLocked === true && after.promptVisible !== true && after.pointerEvents === "none")
    )
    && (readNumber(movement, "movedMeters") ?? 0) > 0.01
    && (readNumber(movement, "skillTravelMetersDelta") ?? 0) > 0.01;
  return {
    passed,
    reason: passed ? null : "click gate did not capture input and move on WASD",
    before,
    after,
    movement,
  };
}

async function pollUntil<T>(fn: () => Promise<T | false | null>, timeoutMs: number, errorMessage: string): Promise<T> {
  const startedAt = performance.now();
  while (performance.now() - startedAt < timeoutMs) {
    const result = await fn();
    if (result) return result;
    await Bun.sleep(100);
  }
  throw new Error(errorMessage);
}

async function connectCdp(webSocketUrl: string): Promise<CdpConnection> {
  const socket = new WebSocket(webSocketUrl);
  const pending = new Map<number, {
    resolve(value: Record<string, unknown>): void;
    reject(error: Error): void;
  }>();
  const eventWaiters: Array<{
    method: string;
    resolve(value: Record<string, unknown>): void;
    reject(error: Error): void;
    timeout: ReturnType<typeof setTimeout>;
  }> = [];
  let nextId = 1;

  await new Promise<void>((resolve, reject) => {
    socket.addEventListener("open", () => resolve(), { once: true });
    socket.addEventListener("error", () => reject(new Error("CDP websocket failed to open")), { once: true });
  });

  socket.addEventListener("message", (event) => {
    const message = JSON.parse(String(event.data)) as CdpMessage;
    if (message.id !== undefined) {
      const waiter = pending.get(message.id);
      if (!waiter) return;
      pending.delete(message.id);
      if (message.error) {
        waiter.reject(new Error(`${message.error.code}: ${message.error.message}`));
      } else {
        waiter.resolve(message.result ?? {});
      }
      return;
    }
    if (!message.method) return;
    for (let index = 0; index < eventWaiters.length; index += 1) {
      const waiter = eventWaiters[index]!;
      if (waiter.method !== message.method) continue;
      clearTimeout(waiter.timeout);
      eventWaiters.splice(index, 1);
      waiter.resolve(message.params ?? {});
      return;
    }
  });

  const send = (method: string, params: Record<string, unknown> = {}) => {
    const id = nextId++;
    socket.send(JSON.stringify({ id, method, params }));
    return new Promise<Record<string, unknown>>((resolve, reject) => {
      pending.set(id, { resolve, reject });
    });
  };

  return {
    async close() {
      socket.close();
      for (const waiter of pending.values()) {
        waiter.reject(new Error("CDP connection closed"));
      }
      pending.clear();
    },
    send,
    waitForEvent(method: string, timeoutMs: number) {
      return new Promise<Record<string, unknown>>((resolve, reject) => {
        const timeout = setTimeout(() => {
          const index = eventWaiters.findIndex((waiter) => waiter.method === method && waiter.reject === reject);
          if (index !== -1) eventWaiters.splice(index, 1);
          reject(new Error(`Timed out waiting for CDP event ${method}`));
        }, timeoutMs);
        eventWaiters.push({ method, resolve, reject, timeout });
      });
    },
    async evaluate<T>(expression: string): Promise<T> {
      const response = await send("Runtime.evaluate", {
        expression,
        awaitPromise: true,
        returnByValue: true,
      });
      const result = readRecord(response.result);
      if (response.exceptionDetails) {
        throw new Error(`Runtime.evaluate failed: ${JSON.stringify(response.exceptionDetails)}`);
      }
      return result.value as T;
    },
  };
}

function readRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : {};
}

function analyzeScreenshotVisualIdentity(bytes: Uint8Array): Record<string, unknown> {
  try {
    const image = decodePng(bytes);
    return analyzeImageVisualIdentity(image);
  } catch (error) {
    return {
      error: error instanceof Error ? error.message : String(error),
    };
  }
}

function analyzeImageVisualIdentity(image: {
  width: number;
  height: number;
  pixels: Uint8Array;
}): Record<string, unknown> {
  const xStart = Math.max(0, Math.floor(image.width * 0.08));
  const xEnd = Math.min(image.width, Math.ceil(image.width * 0.92));
  const yStart = Math.max(0, Math.floor(image.height * 0.18));
  const yEnd = Math.min(image.height, Math.ceil(image.height * 0.78));
  const colorBuckets = new Set<number>();
  let sampleCount = 0;
  let saturationTotal = 0;
  let lumaTotal = 0;
  let lumaSquaredTotal = 0;
  let warmSampleCount = 0;
  let coolSampleCount = 0;
  let axisStrongEdges = 0;
  let axisEdges = 0;
  let diagonalStrongEdges = 0;
  let diagonalEdges = 0;
  let colorStrongEdges = 0;

  for (let y = yStart; y < yEnd; y += 2) {
    for (let x = xStart; x < xEnd; x += 2) {
      const rgb = readRgb(image, x, y);
      const luma = luminance(rgb);
      const saturation = rgbSaturation(rgb);
      saturationTotal += saturation;
      lumaTotal += luma;
      lumaSquaredTotal += luma * luma;
      colorBuckets.add(quantizeRgb(rgb));
      if (rgb[0] > rgb[2] * 1.12 && rgb[0] > rgb[1] * 0.82) {
        warmSampleCount += 1;
      }
      if (rgb[2] > rgb[0] * 1.08 && rgb[1] > rgb[0] * 0.86) {
        coolSampleCount += 1;
      }
      sampleCount += 1;

      if (x + 2 < xEnd) {
        axisEdges += 1;
        const other = readRgb(image, x + 2, y);
        if (Math.abs(luma - luminance(other)) >= 18) {
          axisStrongEdges += 1;
        }
        if (rgbDistance(rgb, other) >= 42) {
          colorStrongEdges += 1;
        }
      }
      if (y + 2 < yEnd) {
        axisEdges += 1;
        const other = readRgb(image, x, y + 2);
        if (Math.abs(luma - luminance(other)) >= 18) {
          axisStrongEdges += 1;
        }
        if (rgbDistance(rgb, other) >= 42) {
          colorStrongEdges += 1;
        }
      }
      if (x + 2 < xEnd && y + 2 < yEnd) {
        diagonalEdges += 1;
        const other = readRgb(image, x + 2, y + 2);
        if (Math.abs(luma - luminance(other)) >= 18) {
          diagonalStrongEdges += 1;
        }
      }
      if (x - 2 >= xStart && y + 2 < yEnd) {
        diagonalEdges += 1;
        const other = readRgb(image, x - 2, y + 2);
        if (Math.abs(luma - luminance(other)) >= 18) {
          diagonalStrongEdges += 1;
        }
      }
    }
  }

  const avgLuma = sampleCount === 0 ? 0 : lumaTotal / sampleCount;
  const lumaVariance = sampleCount === 0
    ? 0
    : Math.max(0, lumaSquaredTotal / sampleCount - avgLuma * avgLuma);
  const axisEdgeRate = axisEdges === 0 ? 0 : axisStrongEdges / axisEdges;
  const diagonalEdgeRate = diagonalEdges === 0 ? 0 : diagonalStrongEdges / diagonalEdges;
  return {
    width: image.width,
    height: image.height,
    sampleCount,
    avgSaturation: sampleCount === 0 ? 0 : saturationTotal / sampleCount,
    avgLuma,
    lumaStdDev: Math.sqrt(lumaVariance),
    quantizedColorCount: colorBuckets.size,
    axisEdgeRate,
    diagonalEdgeRate,
    axisAlignedEdgeDominance: axisEdgeRate / Math.max(0.001, diagonalEdgeRate),
    colorEdgeRate: axisEdges === 0 ? 0 : colorStrongEdges / axisEdges,
    warmRatio: sampleCount === 0 ? 0 : warmSampleCount / sampleCount,
    coolRatio: sampleCount === 0 ? 0 : coolSampleCount / sampleCount,
  };
}

function decodePng(bytes: Uint8Array): {
  width: number;
  height: number;
  pixels: Uint8Array;
} {
  const signature = [137, 80, 78, 71, 13, 10, 26, 10];
  for (let index = 0; index < signature.length; index += 1) {
    if (bytes[index] !== signature[index]) {
      throw new Error("not a PNG file");
    }
  }
  let offset = 8;
  let width = 0;
  let height = 0;
  let bitDepth = 0;
  let colorType = 0;
  const idatChunks: Uint8Array[] = [];
  while (offset + 8 <= bytes.length) {
    const length = readUInt32Be(bytes, offset);
    const type = String.fromCharCode(
      bytes[offset + 4] ?? 0,
      bytes[offset + 5] ?? 0,
      bytes[offset + 6] ?? 0,
      bytes[offset + 7] ?? 0,
    );
    const dataStart = offset + 8;
    const dataEnd = dataStart + length;
    if (dataEnd + 4 > bytes.length) {
      throw new Error(`truncated PNG chunk ${type}`);
    }
    const data = bytes.subarray(dataStart, dataEnd);
    if (type === "IHDR") {
      width = readUInt32Be(data, 0);
      height = readUInt32Be(data, 4);
      bitDepth = data[8] ?? 0;
      colorType = data[9] ?? 0;
    } else if (type === "IDAT") {
      idatChunks.push(data);
    } else if (type === "IEND") {
      break;
    }
    offset = dataEnd + 4;
  }
  if (width <= 0 || height <= 0) {
    throw new Error("PNG missing IHDR");
  }
  if (bitDepth !== 8 || (colorType !== 2 && colorType !== 6)) {
    throw new Error(`unsupported PNG format bitDepth=${bitDepth} colorType=${colorType}`);
  }
  const sourceBytesPerPixel = colorType === 6 ? 4 : 3;
  const scanlineLength = width * sourceBytesPerPixel;
  const compressed = concatUint8Arrays(idatChunks);
  const inflated = inflateSync(compressed);
  const raw = new Uint8Array(inflated.buffer, inflated.byteOffset, inflated.byteLength);
  const expectedLength = (scanlineLength + 1) * height;
  if (raw.length < expectedLength) {
    throw new Error("PNG data is shorter than expected");
  }
  const unfiltered = new Uint8Array(scanlineLength * height);
  let sourceOffset = 0;
  let targetOffset = 0;
  for (let y = 0; y < height; y += 1) {
    const filter = raw[sourceOffset] ?? 0;
    sourceOffset += 1;
    for (let x = 0; x < scanlineLength; x += 1) {
      const rawValue = raw[sourceOffset + x] ?? 0;
      const left = x >= sourceBytesPerPixel ? unfiltered[targetOffset + x - sourceBytesPerPixel] ?? 0 : 0;
      const up = y > 0 ? unfiltered[targetOffset + x - scanlineLength] ?? 0 : 0;
      const upLeft = y > 0 && x >= sourceBytesPerPixel
        ? unfiltered[targetOffset + x - scanlineLength - sourceBytesPerPixel] ?? 0
        : 0;
      unfiltered[targetOffset + x] = unfilterPngByte(filter, rawValue, left, up, upLeft);
    }
    sourceOffset += scanlineLength;
    targetOffset += scanlineLength;
  }
  const pixels = new Uint8Array(width * height * 4);
  for (let index = 0; index < width * height; index += 1) {
    const src = index * sourceBytesPerPixel;
    const dst = index * 4;
    pixels[dst] = unfiltered[src] ?? 0;
    pixels[dst + 1] = unfiltered[src + 1] ?? 0;
    pixels[dst + 2] = unfiltered[src + 2] ?? 0;
    pixels[dst + 3] = colorType === 6 ? unfiltered[src + 3] ?? 255 : 255;
  }
  return { width, height, pixels };
}

function unfilterPngByte(filter: number, rawValue: number, left: number, up: number, upLeft: number): number {
  switch (filter) {
    case 0:
      return rawValue;
    case 1:
      return (rawValue + left) & 0xff;
    case 2:
      return (rawValue + up) & 0xff;
    case 3:
      return (rawValue + Math.floor((left + up) / 2)) & 0xff;
    case 4:
      return (rawValue + paethPredictor(left, up, upLeft)) & 0xff;
    default:
      throw new Error(`unsupported PNG filter ${filter}`);
  }
}

function paethPredictor(left: number, up: number, upLeft: number): number {
  const estimate = left + up - upLeft;
  const leftDistance = Math.abs(estimate - left);
  const upDistance = Math.abs(estimate - up);
  const upLeftDistance = Math.abs(estimate - upLeft);
  if (leftDistance <= upDistance && leftDistance <= upLeftDistance) {
    return left;
  }
  return upDistance <= upLeftDistance ? up : upLeft;
}

function concatUint8Arrays(chunks: readonly Uint8Array[]): Uint8Array {
  const totalLength = chunks.reduce((total, chunk) => total + chunk.length, 0);
  const output = new Uint8Array(totalLength);
  let offset = 0;
  for (const chunk of chunks) {
    output.set(chunk, offset);
    offset += chunk.length;
  }
  return output;
}

function readUInt32Be(bytes: Uint8Array, offset: number): number {
  return (
    ((bytes[offset] ?? 0) * 0x1000000)
    + ((bytes[offset + 1] ?? 0) << 16)
    + ((bytes[offset + 2] ?? 0) << 8)
    + (bytes[offset + 3] ?? 0)
  ) >>> 0;
}

function readRgb(image: { width: number; pixels: Uint8Array }, x: number, y: number): [number, number, number] {
  const index = (y * image.width + x) * 4;
  return [
    image.pixels[index] ?? 0,
    image.pixels[index + 1] ?? 0,
    image.pixels[index + 2] ?? 0,
  ];
}

function luminance(rgb: readonly [number, number, number]): number {
  return rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722;
}

function rgbSaturation(rgb: readonly [number, number, number]): number {
  const max = Math.max(rgb[0], rgb[1], rgb[2]);
  const min = Math.min(rgb[0], rgb[1], rgb[2]);
  return max === 0 ? 0 : (max - min) / max;
}

function rgbDistance(left: readonly [number, number, number], right: readonly [number, number, number]): number {
  return Math.hypot(left[0] - right[0], left[1] - right[1], left[2] - right[2]);
}

function quantizeRgb(rgb: readonly [number, number, number]): number {
  return ((rgb[0] >> 4) << 8) | ((rgb[1] >> 4) << 4) | (rgb[2] >> 4);
}

function readNumber(record: Record<string, unknown>, key: string): number | null {
  const value = record[key];
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function readStringField(record: Record<string, unknown>, key: string): string | null {
  const value = record[key];
  return typeof value === "string" ? value : null;
}

function readGitShortHead(): string | null {
  const result = Bun.spawnSync(["git", "rev-parse", "--short", "HEAD"], {
    cwd: process.cwd(),
    stdout: "pipe",
    stderr: "ignore",
  });
  if (result.exitCode !== 0) return null;
  return new TextDecoder().decode(result.stdout).trim() || null;
}

function timestampForFile(date: Date): string {
  return date.toISOString().replace(/[-:]/g, "").replace(/\.\d{3}Z$/, "Z");
}

function sanitizeFileStem(value: string): string {
  return value.toLowerCase().replace(/[^a-z0-9._-]+/g, "-").replace(/^-+|-+$/g, "") || "run";
}

function formatNumber(value: number | null): string {
  if (value === null) return "n/a";
  return Number.isInteger(value) ? String(value) : value.toFixed(2);
}
