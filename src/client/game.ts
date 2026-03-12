import { GameController, type GameHudSnapshot } from "./game-controller.ts";
import { formatDiscoveryInline } from "../engine/discovery-catalog.ts";
import { worldUnitsToMeters } from "../engine/scale.ts";
import type {
  DiscoveryEvent,
  ExplorationJournalSnapshot,
} from "../engine/exploration-journal.ts";

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
        maxFarFieldBandRebuildsPerFrame?: number,
      ): ReturnType<GameController["setStreamingBudgets"]>;
      forceResidencyUpdate(): ReturnType<GameController["forceResidencyUpdate"]>;
      teleportAndSettle(
        x: number,
        y: number,
        z: number,
        options?: { radiusChunks?: number },
      ): ReturnType<GameController["teleportAndSettle"]>;
      benchmarkChunkCrossing(
        iterations: number,
        chunkDelta?: number,
      ): ReturnType<GameController["benchmarkChunkCrossing"]>;
      benchmarkIncrementalCrossing(
        iterations: number,
        chunkDelta?: number,
        stepsPerLeg?: number,
        settleFrames?: number,
      ): ReturnType<GameController["benchmarkIncrementalCrossing"]>;
      benchmarkRouteExperience(
        options?: Parameters<GameController["benchmarkRouteExperience"]>[0],
      ): ReturnType<GameController["benchmarkRouteExperience"]>;
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
      probeNearFarSeamGaps(): ReturnType<GameController["probeNearFarSeamGaps"]>;
      probeFarFieldSurfaceGaps(): ReturnType<GameController["probeFarFieldSurfaceGaps"]>;
      getDiscoveryJournal(): ExplorationJournalSnapshot;
      resetDiscoveryJournal(): ExplorationJournalSnapshot;
      getInventory(): ReturnType<GameController["getInventorySnapshot"]>;
      getEditLog(): ReturnType<GameController["getEditLogSnapshot"]>;
      breakTargetVoxel(): ReturnType<GameController["breakTargetVoxel"]>;
      placeSelectedVoxel(): ReturnType<GameController["placeSelectedVoxel"]>;
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

interface TelemetrySummaryView {
  update(snapshot: GameHudSnapshot): void;
}

interface TelemetryHistorySample {
  fps: number;
  frameMs: number;
  streamMs: number;
  meshMs: number;
  farFieldMs: number;
  renderMs: number;
  otherMs: number;
}

const ACHIEVEMENT_VISIBLE_MS = 3400;
const ACHIEVEMENT_EXIT_MS = 320;
const TELEMETRY_HISTORY_LIMIT = 64;
const TELEMETRY_STORAGE_KEY = "voxels.telemetry.collapsed";
const TELEMETRY_SUMMARY_LABELS = ["FPS", "Frame", "Stream", "Mesh", "Far", "Render"] as const;

const TELEMETRY_LABELS = [
  "Position",
  "Feet",
  "Player Chunk",
  "Stream Anchor",
  "Grounded",
  "Body Water",
  "Eye Water",
  "Yaw",
  "Pitch",
  "Resident Chunks",
  "Dirty Resident",
  "Radius",
  "Surface Y",
  "Far Field",
  "Far Build",
  "Far Bands",
  "Far Pending",
  "Far Tris",
  "Voxels",
  "Palette",
  "Stream",
  "Generated",
  "Evicted",
  "Pending",
  "Empty Skipped",
  "Empty Cache Hits",
  "Biome",
  "Underground",
  "Variant",
  "Landmark",
  "Discoveries",
  "Last Discovery",
  "Selected Slot",
  "Selected Material",
  "Selected Count",
  "Used Stacks",
  "Gen Budget",
  "Mesh Budget",
  "Far Budget",
  "Mesh",
  "New Meshes",
  "Remeshes",
  "Sync",
  "Upload",
  "Upload Chunks",
  "Draw Calls",
  "Triangles",
  "Frame CPU",
  "Avg Frame CPU",
] as const;

const runtime = mountGame();
reportAsyncFailure(runtime.ready);

if (import.meta.hot) {
  import.meta.hot.accept();
  import.meta.hot.dispose(() => {
    runtime.dispose();
  });
}

function mountGame(): GameRuntime {
  const appRoot = document.querySelector<HTMLElement>("[data-app='game']");
  if (!appRoot) {
    throw new Error("Game root not found");
  }

  const canvas = appRoot.querySelector<HTMLCanvasElement>("[data-role='viewport']");
  const telemetryPanel = appRoot.querySelector<HTMLElement>("[data-role='telemetry-panel']");
  const telemetryToggle = appRoot.querySelector<HTMLButtonElement>("[data-role='telemetry-toggle']");
  const telemetryToggleLabel = appRoot.querySelector<HTMLElement>("[data-role='telemetry-toggle-label']");
  const telemetrySummaryElement = appRoot.querySelector<HTMLElement>("[data-role='telemetry-summary']");
  const telemetryChart = appRoot.querySelector<HTMLCanvasElement>("[data-role='telemetry-chart']");
  const telemetryDetails = appRoot.querySelector<HTMLElement>("[data-role='telemetry-details']");
  const telemetryElement = appRoot.querySelector<HTMLElement>("[data-role='telemetry']");
  const captureButton = appRoot.querySelector<HTMLButtonElement>("[data-role='capture']");
  const achievementElement = appRoot.querySelector<HTMLElement>("[data-role='discovery-achievements']");

  if (
    !canvas
    || !telemetryPanel
    || !telemetryToggle
    || !telemetryToggleLabel
    || !telemetrySummaryElement
    || !telemetryChart
    || !telemetryDetails
    || !telemetryElement
    || !captureButton
    || !achievementElement
  ) {
    throw new Error("Game UI is incomplete");
  }

  const controller = new GameController(canvas);
  const handleCaptureClick = async () => {
    await controller.requestPointerLock();
  };
  const telemetryValues = createTelemetryValues(telemetryElement);
  const lastTelemetryValues = new Array<string>(telemetryValues.length).fill("");
  const telemetrySummaryView = createTelemetrySummaryView(telemetrySummaryElement, telemetryChart);
  const achievementPresenter = createAchievementPresenter(achievementElement);
  let telemetryCollapsed = loadTelemetryCollapsed();

  const applyTelemetryCollapsed = () => {
    telemetryPanel.classList.toggle("is-collapsed", telemetryCollapsed);
    telemetryDetails.hidden = telemetryCollapsed;
    telemetryToggle.setAttribute("aria-expanded", telemetryCollapsed ? "false" : "true");
    telemetryToggleLabel.textContent = telemetryCollapsed ? "Show Debug" : "Hide Debug";
    storeTelemetryCollapsed(telemetryCollapsed);
  };
  const handleTelemetryToggle = () => {
    telemetryCollapsed = !telemetryCollapsed;
    applyTelemetryCollapsed();
  };
  applyTelemetryCollapsed();

  controller.onHudUpdate = (snapshot) => {
    const nextValues = buildTelemetryValues(snapshot);
    for (let index = 0; index < nextValues.length; index += 1) {
      const value = nextValues[index]!;
      if (value === lastTelemetryValues[index]) {
        continue;
      }
      telemetryValues[index]!.textContent = value;
      lastTelemetryValues[index] = value;
    }
    telemetrySummaryView.update(snapshot);
    achievementPresenter.observe(snapshot.recentDiscoveries);
    captureButton.hidden = snapshot.pointerLocked;
  };

  captureButton.addEventListener("click", handleCaptureClick);
  telemetryToggle.addEventListener("click", handleTelemetryToggle);
  window.__VOXELS_GAME__ = {
    controller,
    snapshot: () => controller.getDebugSnapshot(),
    snapshotResidentWorld: () => controller.snapshotResidentWorld(),
    requestPointerLock: () => controller.requestPointerLock(),
    teleport: (x, y, z) => controller.teleport([x, y, z]),
    setViewDistance: (chunks) => controller.setResidencyRadiusChunks(chunks),
    getStreamingBudgets: () => controller.getStreamingBudgets(),
    setStreamingBudgets: (maxGeneratedChunksPerUpdate, maxMeshRebuildsPerFrame, maxFarFieldBandRebuildsPerFrame) =>
      controller.setStreamingBudgets(
        maxGeneratedChunksPerUpdate,
        maxMeshRebuildsPerFrame,
        maxFarFieldBandRebuildsPerFrame,
      ),
    forceResidencyUpdate: () => controller.forceResidencyUpdate(),
    teleportAndSettle: (x, y, z, options) => controller.teleportAndSettle([x, y, z], options),
    benchmarkChunkCrossing: (iterations, chunkDelta) => controller.benchmarkChunkCrossing(iterations, chunkDelta),
    benchmarkIncrementalCrossing: (iterations, chunkDelta, stepsPerLeg, settleFrames) =>
      controller.benchmarkIncrementalCrossing(iterations, chunkDelta, stepsPerLeg, settleFrames),
    benchmarkRouteExperience: (options) => controller.benchmarkRouteExperience(options),
    probeLodCoverage: (sampleRadiusMeters, sampleStepMeters) =>
      controller.probeLodCoverage(sampleRadiusMeters, sampleStepMeters),
    probeRenderReadyCoverage: (sampleRadiusMeters, sampleStepMeters) =>
      controller.probeRenderReadyCoverage(sampleRadiusMeters, sampleStepMeters),
    probeVisibleGroundCoverage: (sampleForwardMeters, sampleLateralMeters, sampleStepMeters) =>
      controller.probeVisibleGroundCoverage(sampleForwardMeters, sampleLateralMeters, sampleStepMeters),
    probeNearFarSeamGaps: () => controller.probeNearFarSeamGaps(),
    probeFarFieldSurfaceGaps: () => controller.probeFarFieldSurfaceGaps(),
    getDiscoveryJournal: () => controller.getDiscoveryJournalSnapshot(),
    resetDiscoveryJournal: () => controller.resetDiscoveryJournal(),
    getInventory: () => controller.getInventorySnapshot(),
    getEditLog: () => controller.getEditLogSnapshot(),
    breakTargetVoxel: () => controller.breakTargetVoxel(),
    placeSelectedVoxel: () => controller.placeSelectedVoxel(),
  };

  const ready = controller.init();

  return {
    controller,
    ready,
    dispose() {
      captureButton.removeEventListener("click", handleCaptureClick);
      telemetryToggle.removeEventListener("click", handleTelemetryToggle);
      controller.onHudUpdate = null;
      achievementPresenter.dispose();
      controller.dispose();
      delete window.__VOXELS_GAME__;
    },
  };
}

function reportAsyncFailure(promise: Promise<unknown>): void {
  promise.catch((error) => {
    queueMicrotask(() => {
      throw error;
    });
  });
}

function createTelemetryValues(root: HTMLElement): HTMLSpanElement[] {
  const fragment = document.createDocumentFragment();
  const values: HTMLSpanElement[] = [];
  for (const label of TELEMETRY_LABELS) {
    const metric = document.createElement("div");
    metric.className = "game-metric";
    const labelElement = document.createElement("span");
    labelElement.textContent = label;
    const valueElement = document.createElement("strong");
    metric.append(labelElement, valueElement);
    fragment.append(metric);
    values.push(valueElement);
  }
  root.replaceChildren(fragment);
  return values;
}

function createAchievementPresenter(root: HTMLElement): AchievementPresenter {
  const categoryElement = document.createElement("p");
  categoryElement.className = "discovery-achievement-category";
  const nameElement = document.createElement("h2");
  nameElement.className = "discovery-achievement-name";
  const identifierElement = document.createElement("p");
  identifierElement.className = "discovery-achievement-identifier";
  const card = document.createElement("section");
  card.className = "discovery-achievement";
  card.hidden = true;
  card.append(categoryElement, nameElement, identifierElement);
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
    categoryElement.textContent = `${event.categoryLabel} Discovered`;
    nameElement.textContent = event.name;
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

function createTelemetrySummaryView(root: HTMLElement, canvas: HTMLCanvasElement): TelemetrySummaryView {
  const fragment = document.createDocumentFragment();
  const valueElements: HTMLSpanElement[] = [];
  for (const label of TELEMETRY_SUMMARY_LABELS) {
    const metric = document.createElement("div");
    metric.className = "game-telemetry-summary-metric";
    const labelElement = document.createElement("span");
    labelElement.textContent = label;
    const valueElement = document.createElement("strong");
    metric.append(labelElement, valueElement);
    fragment.append(metric);
    valueElements.push(valueElement);
  }
  root.replaceChildren(fragment);

  const lastValues = new Array<string>(valueElements.length).fill("");
  const history: TelemetryHistorySample[] = [];

  return {
    update(snapshot) {
      const renderMs = snapshot.lastFrameSyncMs + snapshot.lastFrameUploadMs + snapshot.lastFrameEncodeMs;
      const frameMs = Math.max(0, snapshot.lastFrameCpuMs);
      const fps = frameMs > 0 ? Math.min(240, 1000 / frameMs) : 0;
      const historySample: TelemetryHistorySample = {
        fps,
        frameMs,
        streamMs: Math.max(0, snapshot.streamMs),
        meshMs: Math.max(0, snapshot.meshMs),
        farFieldMs: Math.max(0, snapshot.farFieldMs),
        renderMs: Math.max(0, renderMs),
        otherMs: Math.max(0, frameMs - snapshot.streamMs - snapshot.meshMs - snapshot.farFieldMs - renderMs),
      };
      history.push(historySample);
      if (history.length > TELEMETRY_HISTORY_LIMIT) {
        history.shift();
      }

      const summaryValues = [
        formatSummaryFps(snapshot.avgFrameCpuMs > 0 ? 1000 / snapshot.avgFrameCpuMs : fps),
        formatSummaryMs(frameMs),
        formatSummaryMs(snapshot.streamMs),
        formatSummaryMs(snapshot.meshMs),
        formatSummaryMs(snapshot.farFieldMs),
        formatSummaryMs(renderMs),
      ];
      for (let index = 0; index < summaryValues.length; index += 1) {
        const value = summaryValues[index]!;
        if (value === lastValues[index]) {
          continue;
        }
        valueElements[index]!.textContent = value;
        lastValues[index] = value;
      }

      drawTelemetrySummaryChart(canvas, history);
    },
  };
}

function buildTelemetryValues(snapshot: GameHudSnapshot): string[] {
  return [
    formatPosition(snapshot.position),
    formatPosition(snapshot.feetPosition),
    snapshot.playerChunk.join(", "),
    snapshot.streamAnchorChunk.join(", "),
    snapshot.grounded ? "Yes" : "No",
    snapshot.bodyInWater ? "Yes" : "No",
    snapshot.eyeInWater ? "Yes" : "No",
    `${snapshot.yawDegrees.toFixed(1)}°`,
    `${snapshot.pitchDegrees.toFixed(1)}°`,
    snapshot.chunkCount.toLocaleString(),
    snapshot.streamDirtyResidentChunks.toLocaleString(),
    `${snapshot.residencyRadiusChunks} chunks`,
    `${worldUnitsToMeters(snapshot.surfaceY).toFixed(1)} m`,
    `${snapshot.farFieldMaxRadiusMeters.toFixed(0)} m`,
    `${snapshot.farFieldMs.toFixed(1)} ms`,
    snapshot.farFieldBuiltBands.toLocaleString(),
    snapshot.farFieldPendingBands.toLocaleString(),
    snapshot.farFieldTriangles.toLocaleString(),
    snapshot.solidVoxelCount.toLocaleString(),
    snapshot.paletteCount.toLocaleString(),
    `${snapshot.streamMs.toFixed(1)} ms`,
    snapshot.streamGeneratedChunks.toLocaleString(),
    snapshot.streamEvictedChunks.toLocaleString(),
    snapshot.streamPendingChunks.toLocaleString(),
    snapshot.streamEmptyChunksSkipped.toLocaleString(),
    snapshot.streamCachedEmptyChunkHits.toLocaleString(),
    formatDiscoveryInline("biome", snapshot.biomeId, "Unknown"),
    formatDiscoveryInline("underground", snapshot.undergroundBiomeId, "Unknown"),
    formatDiscoveryInline("regional-variant", snapshot.regionalVariantId),
    formatDiscoveryInline("landmark", snapshot.landmarkId),
    `B ${snapshot.discoveredBiomeCount} / U ${snapshot.discoveredUndergroundBiomeCount} / V ${snapshot.discoveredRegionalVariantCount} / L ${snapshot.discoveredLandmarkCount}`,
    snapshot.lastDiscoveryLabel,
    snapshot.selectedInventorySlot.toLocaleString(),
    snapshot.selectedInventoryMaterial,
    snapshot.selectedInventoryCount.toLocaleString(),
    snapshot.usedInventoryStacks.toLocaleString(),
    snapshot.maxGeneratedChunksPerUpdate.toLocaleString(),
    snapshot.maxMeshRebuildsPerFrame.toLocaleString(),
    snapshot.maxFarFieldBandRebuildsPerFrame.toLocaleString(),
    `${snapshot.meshMs.toFixed(1)} ms`,
    snapshot.meshNewChunks.toLocaleString(),
    snapshot.meshRemeshChunks.toLocaleString(),
    `${snapshot.lastFrameSyncMs.toFixed(2)} ms`,
    `${snapshot.lastFrameUploadMs.toFixed(2)} ms`,
    snapshot.lastFrameUploadChunks.toLocaleString(),
    snapshot.drawCalls.toLocaleString(),
    snapshot.triangles.toLocaleString(),
    `${snapshot.lastFrameCpuMs.toFixed(2)} ms`,
    `${snapshot.avgFrameCpuMs.toFixed(2)} ms`,
  ];
}

function formatPosition(position: [number, number, number]): string {
  return position.map((value) => `${worldUnitsToMeters(value).toFixed(1)}m`).join(", ");
}

function loadTelemetryCollapsed(): boolean {
  try {
    const stored = window.localStorage.getItem(TELEMETRY_STORAGE_KEY);
    return stored === null ? true : stored === "true";
  } catch {
    return true;
  }
}

function storeTelemetryCollapsed(collapsed: boolean): void {
  try {
    window.localStorage.setItem(TELEMETRY_STORAGE_KEY, collapsed ? "true" : "false");
  } catch {
    // Ignore storage failures; the default collapsed behavior still works.
  }
}

function formatSummaryFps(value: number): string {
  if (!Number.isFinite(value) || value <= 0) {
    return "0";
  }
  return value >= 100 ? value.toFixed(0) : value.toFixed(1);
}

function formatSummaryMs(value: number): string {
  return `${Math.max(0, value).toFixed(1)} ms`;
}

function drawTelemetrySummaryChart(
  canvas: HTMLCanvasElement,
  samples: readonly TelemetryHistorySample[],
): void {
  const context = canvas.getContext("2d");
  if (!context) {
    return;
  }
  const rect = canvas.getBoundingClientRect();
  const devicePixelRatio = window.devicePixelRatio || 1;
  const width = Math.max(1, Math.round(rect.width * devicePixelRatio));
  const height = Math.max(1, Math.round(rect.height * devicePixelRatio));
  if (canvas.width !== width || canvas.height !== height) {
    canvas.width = width;
    canvas.height = height;
  }

  context.clearRect(0, 0, width, height);
  context.fillStyle = "rgba(6, 10, 12, 0.6)";
  context.fillRect(0, 0, width, height);

  if (samples.length === 0) {
    return;
  }

  const plotBottom = height - 6 * devicePixelRatio;
  const plotTop = 24 * devicePixelRatio;
  const plotHeight = Math.max(1, plotBottom - plotTop);
  const frameScaleMax = Math.max(16.7, ...samples.map((sample) => sample.frameMs));
  const columnWidth = width / samples.length;
  const colors = {
    stream: "rgba(92, 144, 154, 0.7)",
    mesh: "rgba(168, 133, 84, 0.74)",
    far: "rgba(118, 102, 152, 0.68)",
    render: "rgba(98, 128, 98, 0.68)",
    other: "rgba(82, 86, 98, 0.6)",
  } as const;

  context.strokeStyle = "rgba(255, 244, 213, 0.12)";
  context.lineWidth = 1 * devicePixelRatio;
  context.beginPath();
  context.moveTo(0, plotTop + plotHeight * 0.5);
  context.lineTo(width, plotTop + plotHeight * 0.5);
  context.stroke();

  for (let index = 0; index < samples.length; index += 1) {
    const sample = samples[index]!;
    const x = index * columnWidth;
    let top = plotBottom;
    const segments = [
      [sample.otherMs, colors.other],
      [sample.renderMs, colors.render],
      [sample.farFieldMs, colors.far],
      [sample.meshMs, colors.mesh],
      [sample.streamMs, colors.stream],
    ] as const;
    for (const [value, color] of segments) {
      if (value <= 0) {
        continue;
      }
      const segmentHeight = Math.max(1, (Math.min(value, frameScaleMax) / frameScaleMax) * plotHeight);
      const nextTop = Math.max(plotTop, top - segmentHeight);
      const visibleHeight = top - nextTop;
      top = nextTop;
      if (visibleHeight <= 0) {
        continue;
      }
      context.fillStyle = color;
      context.fillRect(x, top, Math.max(1, columnWidth - devicePixelRatio * 0.5), visibleHeight);
    }
  }

  const fpsScaleMax = Math.max(60, ...samples.map((sample) => sample.fps));
  context.strokeStyle = "#fff4d5";
  context.lineWidth = Math.max(1.5, devicePixelRatio * 1.2);
  context.beginPath();
  for (let index = 0; index < samples.length; index += 1) {
    const sample = samples[index]!;
    const x = samples.length === 1
      ? width / 2
      : (index / (samples.length - 1)) * width;
    const clampedFps = Math.max(0, Math.min(sample.fps, fpsScaleMax));
    const y = plotBottom - (clampedFps / fpsScaleMax) * plotHeight;
    if (index === 0) {
      context.moveTo(x, y);
    } else {
      context.lineTo(x, y);
    }
  }
  context.stroke();
}
