import {
  GameController,
  type GameHudSnapshot,
} from "./game-controller.ts";
import {
  describeExplorationObjectives,
  type ExplorationObjectiveSnapshot,
} from "../engine/exploration-objectives.ts";
import type {
  DiscoveryEvent,
  ExplorationJournalSnapshot,
} from "../engine/exploration-journal.ts";
import {
  describeDiscovery,
  formatDiscoveryName,
  type DiscoveryCategory,
} from "../engine/discovery-catalog.ts";

declare global {
  interface Window {
    __VOXELS_GAME__?: {
      controller: GameController;
      snapshot(): ReturnType<GameController["getDebugSnapshot"]>;
      snapshotResidentWorld(): ReturnType<GameController["snapshotResidentWorld"]>;
      requestPointerLock(): Promise<void>;
      teleport(x: number, y: number, z: number): void;
      setViewDistance(chunks: number): void;
      getStreamingBudgets(): ReturnType<GameController["getStreamingBudgets"]>;
      setStreamingBudgets(
        maxGeneratedChunksPerUpdate: number,
        maxMeshRebuildsPerFrame: number,
      ): ReturnType<GameController["setStreamingBudgets"]>;
      forceResidencyUpdate(): ReturnType<GameController["forceResidencyUpdate"]>;
      teleportAndSettle(
        x: number,
        y: number,
        z: number,
        options?: { radiusChunks?: number; maxFrames?: number },
      ): ReturnType<GameController["teleportAndSettle"]>;
      setCameraPoseAndSettle(
        x: number,
        y: number,
        z: number,
        yawRadians: number,
        pitchRadians: number,
        options?: { radiusChunks?: number; maxFrames?: number },
      ): ReturnType<GameController["setCameraPoseAndSettle"]>;
      benchmarkChunkCrossing(
        iterations: number,
        chunkDelta?: number,
      ): ReturnType<GameController["benchmarkChunkCrossing"]>;
      benchmarkChunkCacheReuse(
        chunkDelta?: number,
        maxFramesPerLeg?: number,
      ): ReturnType<GameController["benchmarkChunkCacheReuse"]>;
      benchmarkIncrementalCrossing(
        iterations: number,
        chunkDelta?: number,
        stepsPerLeg?: number,
        settleFrames?: number,
      ): ReturnType<GameController["benchmarkIncrementalCrossing"]>;
      benchmarkRouteExperience(
        options?: Parameters<GameController["benchmarkRouteExperience"]>[0],
      ): ReturnType<GameController["benchmarkRouteExperience"]>;
      benchmarkForwardWalkExperience(
        options?: Parameters<GameController["benchmarkForwardWalkExperience"]>[0],
      ): ReturnType<GameController["benchmarkForwardWalkExperience"]>;
      benchmarkLiveForwardWalkExperience(
        options?: Parameters<GameController["benchmarkLiveForwardWalkExperience"]>[0],
      ): ReturnType<GameController["benchmarkLiveForwardWalkExperience"]>;
      getBootstrapBenchmark(): ReturnType<GameController["getBootstrapBenchmark"]>;
      probeLodCoverage(
        sampleRadiusMeters?: number,
        sampleStepMeters?: number,
      ): ReturnType<GameController["probeLodCoverage"]>;
      probeRenderReadyCoverage(
        sampleRadiusMeters?: number,
        sampleStepMeters?: number,
      ): ReturnType<GameController["probeRenderReadyCoverage"]>;
      probeVisibleGroundCoverage(
        sampleForwardMeters?: number,
        sampleLateralMeters?: number,
        sampleStepMeters?: number,
      ): ReturnType<GameController["probeVisibleGroundCoverage"]>;
      getDiscoveryJournal(): ExplorationJournalSnapshot;
      resetDiscoveryJournal(): ExplorationJournalSnapshot;
      getSkillJournal(): ReturnType<GameController["getSkillJournalSnapshot"]>;
      exportProgressState(): ReturnType<GameController["exportProgressState"]>;
      importProgressState(state: Parameters<GameController["importProgressState"]>[0]): ReturnType<GameController["importProgressState"]>;
      saveProgressState(): boolean;
      loadProgressState(): boolean;
      clearProgressState(): boolean;
      getExplorationObjectives(): ExplorationObjectiveSnapshot;
      getEditLog(): ReturnType<GameController["getEditLogSnapshot"]>;
    };
  }
}

interface GameRuntime {
  controller: GameController;
  ready: Promise<void>;
  dispose(): void;
}

interface AchievementPresenter {
  observe(recentDiscoveries: readonly DiscoveryEvent[]): void;
  dispose(): void;
}

interface InteractionHudView {
  update(snapshot: GameHudSnapshot): void;
}

interface ObjectivePanelView {
  update(snapshot: GameHudSnapshot): void;
}

interface PerformanceStripView {
  update(snapshot: GameHudSnapshot): void;
}

interface CaptureOverlayState {
  status: string;
  title: string;
  subtitle: string;
  detail: string;
  progressRatio: number;
  loading: boolean;
}

const ACHIEVEMENT_VISIBLE_MS = 3400;
const ACHIEVEMENT_EXIT_MS = 320;
const PROGRESS_STORAGE_KEY = "voxels.progress.v1";

const runtime = mountGame();
reportAsyncFailure(runtime.ready);

if (import.meta.hot) {
  import.meta.hot.accept();
  import.meta.hot.dispose(() => {
    runtime.dispose();
  });
}

function mountGame(): GameRuntime {
  const searchParams = new URLSearchParams(window.location.search);
  const appRoot = document.querySelector<HTMLElement>("[data-app='game']");
  if (!appRoot) {
    throw new Error("Game root not found");
  }

  const canvas = appRoot.querySelector<HTMLCanvasElement>("[data-role='viewport']");
  const captureButton = appRoot.querySelector<HTMLButtonElement>("[data-role='capture']");
  const captureStatus = appRoot.querySelector<HTMLElement>("[data-role='capture-status']");
  const captureTitle = appRoot.querySelector<HTMLElement>("[data-role='capture-title']");
  const captureSubtitle = appRoot.querySelector<HTMLElement>("[data-role='capture-subtitle']");
  const captureDetail = appRoot.querySelector<HTMLElement>("[data-role='capture-detail']");
  const captureProgress = appRoot.querySelector<HTMLElement>("[data-role='capture-progress']");
  const captureProgressBar = appRoot.querySelector<HTMLElement>("[data-role='capture-progress-bar']");
  const achievementElement = appRoot.querySelector<HTMLElement>("[data-role='discovery-achievements']");
  const objectivePanelRoot = appRoot.querySelector<HTMLElement>("[data-role='objective-panel']");
  const performanceStripRoot = appRoot.querySelector<HTMLElement>("[data-role='performance-strip']");
  const interactionHudRoot = appRoot.querySelector<HTMLElement>("[data-role='interaction-hud']");

  if (
    !canvas
    || !captureButton
    || !captureStatus
    || !captureTitle
    || !captureSubtitle
    || !captureDetail
    || !captureProgress
    || !captureProgressBar
    || !achievementElement
    || !objectivePanelRoot
    || !performanceStripRoot
    || !interactionHudRoot
  ) {
    throw new Error("Game UI is incomplete");
  }

  const controller = new GameController(canvas, {
    eagerBootstrapBenchmark: searchParams.get("benchmarkBootstrap") === "1",
  });
  const unregisterCapturePointerLockTarget = controller.registerPointerLockTarget(captureButton);
  const requestCapture = async (target: HTMLElement = captureButton) => {
    await controller.requestPointerLock(target);
  };
  const handleCapturePointerDown = (event: PointerEvent) => {
    if (event.button !== 0 || captureButton.disabled) {
      return;
    }
    event.preventDefault();
    void requestCapture(captureButton);
  };
  const handleCaptureClick = () => {
    if (controller.pointerLocked) {
      return;
    }
    void requestCapture(captureButton);
  };
  const achievementPresenter = createAchievementPresenter(achievementElement);
  const objectivePanelView = createObjectivePanelView(objectivePanelRoot);
  const performanceStripView = createPerformanceStripView(performanceStripRoot);
  const interactionHudView = createInteractionHudView(interactionHudRoot);
  let lastProgressSignature = "";

  controller.onHudUpdate = (snapshot) => {
    achievementPresenter.observe(snapshot.recentDiscoveries);
    objectivePanelView.update(snapshot);
    performanceStripView.update(snapshot);
    interactionHudView.update(snapshot);
    const captureState = buildCaptureOverlayState(snapshot);
    captureStatus.textContent = captureState.status;
    captureTitle.textContent = captureState.title;
    captureSubtitle.textContent = captureState.subtitle;
    captureDetail.textContent = captureState.detail;
    captureProgressBar.style.transform = `scaleX(${captureState.progressRatio})`;
    captureProgress.hidden = !captureState.loading;
    captureButton.disabled = captureState.loading;
    captureButton.classList.toggle("is-loading", captureState.loading);
    captureButton.classList.toggle("is-captured", snapshot.pointerLocked);
    captureButton.tabIndex = snapshot.pointerLocked ? -1 : 0;
    captureButton.setAttribute("aria-hidden", snapshot.pointerLocked ? "true" : "false");
    captureButton.setAttribute("aria-label", captureState.title);
    if (snapshot.pointerLocked && document.activeElement === captureButton) {
      canvas.focus({ preventScroll: true });
    }
    const progressSignature = buildProgressSignature(snapshot);
    if (progressSignature !== lastProgressSignature) {
      lastProgressSignature = progressSignature;
      saveProgressState(controller);
    }
  };

  captureButton.addEventListener("pointerdown", handleCapturePointerDown);
  captureButton.addEventListener("click", handleCaptureClick);

  window.__VOXELS_GAME__ = {
    controller,
    snapshot: () => controller.getDebugSnapshot(),
    snapshotResidentWorld: () => controller.snapshotResidentWorld(),
    requestPointerLock: () => controller.requestPointerLock(),
    teleport: (x, y, z) => controller.teleport([x, y, z]),
    setViewDistance: (chunks) => controller.setResidencyRadiusChunks(chunks),
    getStreamingBudgets: () => controller.getStreamingBudgets(),
    setStreamingBudgets: (maxGeneratedChunksPerUpdate, maxMeshRebuildsPerFrame) =>
      controller.setStreamingBudgets(
        maxGeneratedChunksPerUpdate,
        maxMeshRebuildsPerFrame,
      ),
    forceResidencyUpdate: () => controller.forceResidencyUpdate(),
    teleportAndSettle: (x, y, z, options) => controller.teleportAndSettle([x, y, z], options),
    setCameraPoseAndSettle: (x, y, z, yawRadians, pitchRadians, options) =>
      controller.setCameraPoseAndSettle([x, y, z], yawRadians, pitchRadians, options),
    benchmarkChunkCrossing: (iterations, chunkDelta) => controller.benchmarkChunkCrossing(iterations, chunkDelta),
    benchmarkChunkCacheReuse: (chunkDelta, maxFramesPerLeg) =>
      controller.benchmarkChunkCacheReuse(chunkDelta, maxFramesPerLeg),
    benchmarkIncrementalCrossing: (iterations, chunkDelta, stepsPerLeg, settleFrames) =>
      controller.benchmarkIncrementalCrossing(iterations, chunkDelta, stepsPerLeg, settleFrames),
    benchmarkRouteExperience: (options) => controller.benchmarkRouteExperience(options),
    benchmarkForwardWalkExperience: (options) => controller.benchmarkForwardWalkExperience(options),
    benchmarkLiveForwardWalkExperience: (options) => controller.benchmarkLiveForwardWalkExperience(options),
    getBootstrapBenchmark: () => controller.getBootstrapBenchmark(),
    probeLodCoverage: (sampleRadiusMeters, sampleStepMeters) =>
      controller.probeLodCoverage(sampleRadiusMeters, sampleStepMeters),
    probeRenderReadyCoverage: (sampleRadiusMeters, sampleStepMeters) =>
      controller.probeRenderReadyCoverage(sampleRadiusMeters, sampleStepMeters),
    probeVisibleGroundCoverage: (sampleForwardMeters, sampleLateralMeters, sampleStepMeters) =>
      controller.probeVisibleGroundCoverage(sampleForwardMeters, sampleLateralMeters, sampleStepMeters),
    getDiscoveryJournal: () => controller.getDiscoveryJournalSnapshot(),
    resetDiscoveryJournal: () => controller.resetDiscoveryJournal(),
    getSkillJournal: () => controller.getSkillJournalSnapshot(),
    exportProgressState: () => controller.exportProgressState(),
    importProgressState: (state) => controller.importProgressState(state),
    saveProgressState: () => saveProgressState(controller),
    loadProgressState: () => loadProgressState(controller),
    clearProgressState: () => clearProgressState(),
    getExplorationObjectives: () => describeExplorationObjectives(snapshotToObjectiveSource(controller.getDebugSnapshot())),
    getEditLog: () => controller.getEditLogSnapshot(),
  };

  const ready = controller.init().then(() => {
    loadProgressState(controller);
  });

  return {
    controller,
    ready,
    dispose() {
      captureButton.removeEventListener("pointerdown", handleCapturePointerDown);
      captureButton.removeEventListener("click", handleCaptureClick);
      controller.onHudUpdate = null;
      achievementPresenter.dispose();
      unregisterCapturePointerLockTarget();
      controller.dispose();
      delete window.__VOXELS_GAME__;
    },
  };
}

function buildProgressSignature(snapshot: GameHudSnapshot): string {
  return [
    snapshot.discoveredBiomeCount,
    snapshot.discoveredUndergroundBiomeCount,
    snapshot.discoveredRegionalVariantCount,
    snapshot.discoveredLandmarkCount,
    snapshot.totalSkillLevel,
    snapshot.focusSkillName,
    snapshot.focusSkillLevel,
    snapshot.focusSkillProgressRatio.toFixed(3),
    snapshot.totalSkillTravelMeters.toFixed(1),
    snapshot.lastDiscoveryLabel,
  ].join("|");
}

function saveProgressState(controller: GameController): boolean {
  try {
    window.localStorage.setItem(PROGRESS_STORAGE_KEY, JSON.stringify(controller.exportProgressState()));
    return true;
  } catch {
    return false;
  }
}

function loadProgressState(controller: GameController): boolean {
  try {
    const stored = window.localStorage.getItem(PROGRESS_STORAGE_KEY);
    if (!stored) {
      return false;
    }
    const parsed = JSON.parse(stored) as Parameters<GameController["importProgressState"]>[0];
    controller.importProgressState(parsed);
    return true;
  } catch {
    return false;
  }
}

function clearProgressState(): boolean {
  try {
    window.localStorage.removeItem(PROGRESS_STORAGE_KEY);
    return true;
  } catch {
    return false;
  }
}

function reportAsyncFailure(promise: Promise<unknown>): void {
  promise.catch((error) => {
    queueMicrotask(() => {
      throw error;
    });
  });
}

function buildCaptureOverlayState(snapshot: GameHudSnapshot): CaptureOverlayState {
  if (snapshot.status.startsWith("Pointer lock blocked")) {
    return {
      status: "Pointer lock blocked",
      title: "Click to retry",
      subtitle: "",
      detail: "",
      progressRatio: 1,
      loading: false,
    };
  }
  const readyColumns = Math.max(0, snapshot.bootstrapReadyColumns);
  const requiredColumns = Math.max(1, snapshot.bootstrapRequiredColumns);
  const columnRatio = readyColumns / requiredColumns;
  const urgentMeshRatio = snapshot.bootstrapUrgentDirtyMeshlessChunks === 0
    ? 1
    : Math.max(0, 1 - snapshot.bootstrapUrgentDirtyMeshlessChunks / requiredColumns);
  const progressRatio = snapshot.bootstrapVisualReady
    ? 1
    : Math.min(0.82, columnRatio * 0.68 + urgentMeshRatio * 0.18 + (snapshot.bootstrapPendingMeshJobs === 0 ? 0.14 : 0));
  if (snapshot.bootstrapPlayableReady) {
    return {
      status: snapshot.bootstrapVisualReady ? "Ready" : "Finishing",
      title: "Click to play",
      subtitle: "",
      detail: `${snapshot.chunkCount.toLocaleString()} chunks`,
      progressRatio,
      loading: false,
    };
  }
  return {
    status: "Loading",
    title: `${readyColumns.toLocaleString()} / ${requiredColumns.toLocaleString()} columns`,
    subtitle: [
      `${snapshot.streamPendingChunks.toLocaleString()} pending`,
      `${snapshot.bootstrapPendingMeshJobs.toLocaleString()} meshes`,
    ].join(" • "),
    detail: [
      `${snapshot.bootstrapUrgentDirtyMeshlessChunks.toLocaleString()} urgent`,
      `${snapshot.bootstrapElapsedMs.toFixed(0)} ms`,
    ].join(" • "),
    progressRatio: Math.max(0.04, Math.min(0.99, progressRatio)),
    loading: true,
  };
}

function createInteractionHudView(root: HTMLElement): InteractionHudView {
  const region = document.createElement("strong");
  region.className = "game-rpg-region";
  const landmark = document.createElement("span");
  landmark.className = "game-rpg-landmark";
  const skill = document.createElement("span");
  skill.className = "game-rpg-skill";
  const discovery = document.createElement("span");
  discovery.className = "game-rpg-discovery";
  const card = document.createElement("section");
  card.className = "game-rpg-hud";
  card.append(region, landmark, skill, discovery);
  root.replaceChildren(card);

  return {
    update(snapshot) {
      const recentDiscovery = snapshot.recentDiscoveries[0] ?? null;
      region.textContent = formatCurrentRegionName(snapshot);
      landmark.textContent = snapshot.landmarkId
        ? `Near ${formatDiscoveryName("landmark", snapshot.landmarkId)}`
        : snapshot.undergroundBiomeId
        ? formatDiscoveryName("underground", snapshot.undergroundBiomeId)
        : snapshot.ambientProfileLabel;
      skill.textContent = `${snapshot.focusSkillName} ${snapshot.focusSkillLevel}`;
      discovery.textContent = recentDiscovery
        ? `Found ${recentDiscovery.name}`
        : snapshot.lastDiscoveryLabel === "None"
        ? `${snapshot.discoveredLandmarkCount.toLocaleString()} landmarks cataloged`
        : snapshot.lastDiscoveryLabel;
      card.classList.toggle("is-captured", snapshot.pointerLocked);
      card.title = [
        snapshot.status,
        `${snapshot.discoveredBiomeCount} regions`,
        `${snapshot.discoveredRegionalVariantCount} strange regions`,
        `${snapshot.discoveredAncientLandmarkCount} old road signs`,
        `${snapshot.focusSkillName} ${(snapshot.focusSkillProgressRatio * 100).toFixed(0)}%`,
        recentDiscovery?.flavorText ?? null,
      ].filter(Boolean).join(" • ");
    },
  };
}

function createObjectivePanelView(root: HTMLElement): ObjectivePanelView {
  const stage = document.createElement("span");
  stage.className = "game-status-stage";
  const task = document.createElement("strong");
  task.className = "game-status-task";
  const progress = document.createElement("span");
  progress.className = "game-status-progress";
  const fill = document.createElement("span");
  fill.className = "game-status-progress-fill";
  progress.append(fill);
  root.replaceChildren(stage, task, progress);

  return {
    update(snapshot) {
      const objectiveSnapshot = describeExplorationObjectives(snapshotToObjectiveSource(snapshot));
      const currentObjective = objectiveSnapshot.objectives.find((objective) => !objective.completed)
        ?? objectiveSnapshot.objectives[objectiveSnapshot.objectives.length - 1]
        ?? null;
      stage.textContent = `${objectiveSnapshot.title} • ${objectiveSnapshot.completedCount}/${objectiveSnapshot.totalCount}`;
      root.title = [
        objectiveSnapshot.subtitle,
        objectiveSnapshot.journalText,
        currentObjective?.journalText ?? null,
        objectiveSnapshot.progressionHint,
        `${objectiveSnapshot.title}: ${objectiveSnapshot.completedCount}/${objectiveSnapshot.totalCount}`,
        `${snapshot.focusSkillName} ${snapshot.focusSkillLevel}`,
        `Fog ${snapshot.ambientFogEndMeters.toFixed(0)} m`,
      ].filter(Boolean).join(" • ");
      if (!currentObjective) {
        task.textContent = "Expedition complete";
        fill.style.transform = "scaleX(1)";
        return;
      }
      task.textContent = `${currentObjective.label} ${currentObjective.progress}/${currentObjective.target}`;
      fill.style.transform = `scaleX(${currentObjective.target > 0 ? currentObjective.progress / currentObjective.target : 0})`;
    },
  };
}

function snapshotToObjectiveSource(snapshot: GameHudSnapshot) {
  return {
    discoveredBiomeCount: snapshot.discoveredBiomeCount,
    discoveredUndergroundBiomeCount: snapshot.discoveredUndergroundBiomeCount,
    discoveredRegionalVariantCount: snapshot.discoveredRegionalVariantCount,
    discoveredLandmarkCount: snapshot.discoveredLandmarkCount,
    discoveredAncientLandmarkCount: snapshot.discoveredAncientLandmarkCount,
  };
}

function createAchievementPresenter(root: HTMLElement): AchievementPresenter {
  const categoryElement = document.createElement("p");
  categoryElement.className = "discovery-achievement-category";
  const nameElement = document.createElement("h2");
  nameElement.className = "discovery-achievement-name";
  const flavorElement = document.createElement("p");
  flavorElement.className = "discovery-achievement-flavor";
  const identifierElement = document.createElement("p");
  identifierElement.className = "discovery-achievement-identifier";
  const card = document.createElement("section");
  card.className = "discovery-achievement";
  card.hidden = true;
  card.append(categoryElement, nameElement, flavorElement, identifierElement);
  root.replaceChildren(card);

  const queuedSequences = new Set<number>();
  const queue: DiscoveryEvent[] = [];
  let lastSeenSequence = 0;
  let activeSequence = 0;
  let timerId = 0;
  let exitTimerId = 0;

  const scheduleNext = () => {
    if (activeSequence !== 0 || queue.length === 0) {
      return;
    }
    const event = queue.shift()!;
    activeSequence = event.sequence;
    card.hidden = false;
    categoryElement.textContent = `${formatDiscoveryCategoryLabel(event.category, event.id)} Discovered`;
    nameElement.textContent = event.name;
    flavorElement.textContent = event.flavorText ?? "";
    flavorElement.hidden = !event.flavorText;
    identifierElement.textContent = `[${event.identifier}]`;
    card.classList.remove("is-active");
    void card.offsetWidth;
    card.classList.add("is-active");
    timerId = window.setTimeout(() => {
      card.classList.remove("is-active");
      exitTimerId = window.setTimeout(() => {
        activeSequence = 0;
        if (queue.length === 0) {
          card.hidden = true;
        }
        scheduleNext();
      }, ACHIEVEMENT_EXIT_MS);
    }, ACHIEVEMENT_VISIBLE_MS);
  };

  return {
    observe(recentDiscoveries) {
      const unseen = recentDiscoveries
        .filter((event) => event.sequence > lastSeenSequence && !queuedSequences.has(event.sequence))
        .sort((left, right) => left.sequence - right.sequence);
      if (unseen.length === 0) {
        return;
      }
      lastSeenSequence = Math.max(lastSeenSequence, unseen[unseen.length - 1]!.sequence);
      for (const event of unseen) {
        queuedSequences.add(event.sequence);
        queue.push(event);
      }
      scheduleNext();
    },
    dispose() {
      if (timerId !== 0) {
        window.clearTimeout(timerId);
      }
      if (exitTimerId !== 0) {
        window.clearTimeout(exitTimerId);
      }
      queue.length = 0;
      queuedSequences.clear();
      activeSequence = 0;
      card.hidden = true;
      card.classList.remove("is-active");
    },
  };
}

function createPerformanceStripView(root: HTMLElement): PerformanceStripView {
  const frame = document.createElement("strong");
  frame.className = "game-performance-value";
  const work = document.createElement("span");
  work.className = "game-performance-value";
  const chunks = document.createElement("span");
  chunks.className = "game-performance-value";
  root.replaceChildren(frame, work, chunks);

  const lastValues = ["", "", ""];
  return {
    update(snapshot) {
      const wallFrameMs = snapshot.avgFrameWallMs > 0 ? snapshot.avgFrameWallMs : snapshot.lastFrameWallMs;
      const nextValues = [
        formatWallFrameRate(wallFrameMs),
        `${formatFrameMs(snapshot.lastGameplayFrameMs)} work`,
        `${formatCompactCount(snapshot.chunkCount)} chunks`,
      ];
      for (let index = 0; index < nextValues.length; index += 1) {
        if (nextValues[index] === lastValues[index]) {
          continue;
        }
        root.children[index]!.textContent = nextValues[index]!;
        lastValues[index] = nextValues[index]!;
      }
      root.title = [
        `Wall frame ${snapshot.lastFrameWallMs.toFixed(1)} ms`,
        `Average wall frame ${snapshot.avgFrameWallMs.toFixed(1)} ms`,
        `Gameplay work ${snapshot.lastGameplayFrameMs.toFixed(1)} ms`,
        `Render CPU ${snapshot.lastFrameCpuMs.toFixed(1)} ms`,
        `Stream ${snapshot.streamMs.toFixed(1)} ms`,
        `Mesh ${snapshot.meshMs.toFixed(1)} ms`,
        `Pending ${snapshot.streamPendingChunks.toLocaleString()}`,
        `Dirty ${snapshot.streamDirtyResidentChunks.toLocaleString()}`,
        `Draws ${snapshot.drawCalls.toLocaleString()}`,
        `LOD ${snapshot.lodChunkCount.toLocaleString()}`,
        `Fog culled ${snapshot.fogCulledChunks.toLocaleString()}`,
      ].join(" • ");
    },
  };
}

function formatWallFrameRate(frameWallMs: number): string {
  if (!Number.isFinite(frameWallMs) || frameWallMs <= 0) {
    return "0 fps";
  }
  const fps = 1000 / frameWallMs;
  return `${fps.toFixed(fps >= 100 ? 0 : 1)} fps`;
}

function formatFrameMs(frameWallMs: number): string {
  if (!Number.isFinite(frameWallMs) || frameWallMs < 0) {
    return "0.0 ms";
  }
  return `${frameWallMs.toFixed(frameWallMs >= 100 ? 0 : 1)} ms`;
}

function formatCompactCount(value: number): string {
  const absolute = Math.abs(value);
  if (absolute >= 1_000_000) {
    return `${(value / 1_000_000).toFixed(1)}M`;
  }
  if (absolute >= 10_000) {
    return `${(value / 1_000).toFixed(0)}k`;
  }
  if (absolute >= 1_000) {
    return `${(value / 1_000).toFixed(1)}k`;
  }
  return value.toLocaleString();
}

function formatCurrentRegionName(snapshot: GameHudSnapshot): string {
  if (snapshot.regionalVariantId) {
    return formatDiscoveryName("regional-variant", snapshot.regionalVariantId);
  }
  if (snapshot.undergroundBiomeId) {
    return formatDiscoveryName("underground", snapshot.undergroundBiomeId);
  }
  if (snapshot.biomeId) {
    return formatDiscoveryName("biome", snapshot.biomeId);
  }
  return snapshot.ambientProfileLabel;
}

function formatDiscoveryCategoryLabel(category: DiscoveryCategory, id: string): string {
  const presentation = describeDiscovery(category, id);
  return presentation.role === "landmark" ? presentation.categoryLabel : presentation.roleLabel;
}
