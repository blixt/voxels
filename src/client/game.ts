import {
  GameController,
  type GameHudSnapshot,
  type TargetingOverlaySnapshot,
  type TargetingSnapshot,
} from "./game-controller.ts";
import { formatDiscoveryInline } from "../engine/discovery-catalog.ts";
import { describeHotbarWindow } from "../engine/hotbar-layout.ts";
import { INVENTORY_STACK_SIZE } from "../engine/inventory.ts";
import { materialToHexColor } from "../engine/procedural-generator.ts";
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
      probeNearFarSeamGaps(): ReturnType<GameController["probeNearFarSeamGaps"]>;
      probeFarFieldSurfaceGaps(): ReturnType<GameController["probeFarFieldSurfaceGaps"]>;
      getDiscoveryJournal(): ExplorationJournalSnapshot;
      resetDiscoveryJournal(): ExplorationJournalSnapshot;
      getInventory(): ReturnType<GameController["getInventorySnapshot"]>;
      getTargeting(): ReturnType<GameController["getTargetingSnapshot"]>;
      getTargetingOverlay(
        viewportWidth?: number,
        viewportHeight?: number,
      ): ReturnType<GameController["getTargetingOverlaySnapshot"]>;
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

interface InteractionHudView {
  update(snapshot: GameHudSnapshot, inventory: ReturnType<GameController["getInventorySnapshot"]>, targeting: TargetingSnapshot): void;
}

interface TargetingOverlayView {
  update(snapshot: TargetingOverlaySnapshot): void;
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
const TELEMETRY_HISTORY_LIMIT = 64;
const TELEMETRY_STORAGE_KEY = "voxels.telemetry.collapsed";
const TELEMETRY_SUMMARY_LABELS = ["FPS", "Frame", "Stream", "Mesh", "Far Build", "Render"] as const;
const HOTBAR_VISIBLE_SLOT_COUNT = 9;

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
  "Chunk Cache Hits",
  "Worker Generated",
  "Gen Workers",
  "Summary Cache Hits",
  "Summaries Built",
  "Region Cache Hits",
  "Region Cache Misses",
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
  const searchParams = new URLSearchParams(window.location.search);
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
  const captureStatus = appRoot.querySelector<HTMLElement>("[data-role='capture-status']");
  const captureTitle = appRoot.querySelector<HTMLElement>("[data-role='capture-title']");
  const captureSubtitle = appRoot.querySelector<HTMLElement>("[data-role='capture-subtitle']");
  const captureDetail = appRoot.querySelector<HTMLElement>("[data-role='capture-detail']");
  const captureProgress = appRoot.querySelector<HTMLElement>("[data-role='capture-progress']");
  const captureProgressBar = appRoot.querySelector<HTMLElement>("[data-role='capture-progress-bar']");
  const achievementElement = appRoot.querySelector<HTMLElement>("[data-role='discovery-achievements']");
  const interactionHudRoot = appRoot.querySelector<HTMLElement>("[data-role='interaction-hud']");
  const targetOverlayRoot = appRoot.querySelector<SVGSVGElement>("[data-role='target-overlay']");

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
    || !captureStatus
    || !captureTitle
    || !captureSubtitle
    || !captureDetail
    || !captureProgress
    || !captureProgressBar
    || !achievementElement
    || !interactionHudRoot
    || !targetOverlayRoot
  ) {
    throw new Error("Game UI is incomplete");
  }

  const controller = new GameController(canvas, {
    eagerBootstrapBenchmark: searchParams.get("benchmarkBootstrap") === "1",
  });
  const handleCaptureClick = async () => {
    await controller.requestPointerLock();
  };
  const telemetryValues = createTelemetryValues(telemetryElement);
  const lastTelemetryValues = new Array<string>(telemetryValues.length).fill("");
  const telemetrySummaryView = createTelemetrySummaryView(telemetrySummaryElement, telemetryChart);
  const achievementPresenter = createAchievementPresenter(achievementElement);
  const interactionHudView = createInteractionHudView(interactionHudRoot);
  const targetingOverlayView = createTargetingOverlayView(targetOverlayRoot);
  let telemetryCollapsed = loadTelemetryCollapsed();
  let targetingOverlayFrameId = 0;

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
    interactionHudView.update(snapshot, controller.getInventorySnapshot(), controller.getTargetingSnapshot());
    const captureState = buildCaptureOverlayState(snapshot);
    captureStatus.textContent = captureState.status;
    captureTitle.textContent = captureState.title;
    captureSubtitle.textContent = captureState.subtitle;
    captureDetail.textContent = captureState.detail;
    captureProgressBar.style.transform = `scaleX(${captureState.progressRatio})`;
    captureProgress.hidden = !captureState.loading;
    captureButton.disabled = captureState.loading;
    captureButton.classList.toggle("is-loading", captureState.loading);
    captureButton.hidden = snapshot.pointerLocked;
  };

  captureButton.addEventListener("click", handleCaptureClick);
  telemetryToggle.addEventListener("click", handleTelemetryToggle);

  const updateTargetingOverlay = () => {
    const viewportWidth = Math.max(1, canvas.clientWidth || canvas.width || 1);
    const viewportHeight = Math.max(1, canvas.clientHeight || canvas.height || 1);
    targetingOverlayView.update(controller.getTargetingOverlaySnapshot(viewportWidth, viewportHeight));
    targetingOverlayFrameId = requestAnimationFrame(updateTargetingOverlay);
  };
  targetingOverlayFrameId = requestAnimationFrame(updateTargetingOverlay);

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
    probeNearFarSeamGaps: () => controller.probeNearFarSeamGaps(),
    probeFarFieldSurfaceGaps: () => controller.probeFarFieldSurfaceGaps(),
    getDiscoveryJournal: () => controller.getDiscoveryJournalSnapshot(),
    resetDiscoveryJournal: () => controller.resetDiscoveryJournal(),
    getInventory: () => controller.getInventorySnapshot(),
    getTargeting: () => controller.getTargetingSnapshot(),
    getTargetingOverlay: (viewportWidth = canvas.clientWidth || canvas.width || 1, viewportHeight = canvas.clientHeight || canvas.height || 1) =>
      controller.getTargetingOverlaySnapshot(viewportWidth, viewportHeight),
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
      cancelAnimationFrame(targetingOverlayFrameId);
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

function buildCaptureOverlayState(snapshot: GameHudSnapshot): CaptureOverlayState {
  const readyColumns = Math.max(0, snapshot.bootstrapReadyColumns);
  const requiredColumns = Math.max(1, snapshot.bootstrapRequiredColumns);
  const columnRatio = readyColumns / requiredColumns;
  const urgentMeshRatio = snapshot.bootstrapUrgentDirtyMeshlessChunks === 0
    ? 1
    : Math.max(0, 1 - snapshot.bootstrapUrgentDirtyMeshlessChunks / requiredColumns);
  const farRatio = snapshot.bootstrapVisualReady
    ? 1
    : snapshot.bootstrapPlayableReady
    ? Math.max(0.82, 1 - snapshot.farFieldPendingBands / Math.max(snapshot.farFieldPendingBands + snapshot.farFieldBuiltBands, 1))
    : 0;
  const progressRatio = snapshot.bootstrapVisualReady
    ? 1
    : snapshot.bootstrapPlayableReady
    ? 0.85 + farRatio * 0.15
    : Math.min(0.82, columnRatio * 0.68 + urgentMeshRatio * 0.18 + (snapshot.bootstrapPendingMeshJobs === 0 ? 0.14 : 0));
  if (snapshot.bootstrapPlayableReady) {
    return {
      status: snapshot.bootstrapVisualReady ? "World Ready" : "Finishing Horizon",
      title: "Click To Enter The World",
      subtitle: "WASD move • Space jump • Left break • Right place • Wheel or 1-9 switch slots",
      detail: snapshot.bootstrapVisualReady
        ? `${snapshot.chunkCount.toLocaleString()} resident chunks • ${snapshot.farFieldTriangles.toLocaleString()} far tris`
        : `${snapshot.farFieldPendingBands.toLocaleString()} far band(s) still settling`,
      progressRatio,
      loading: false,
    };
  }
  return {
    status: "Preparing World",
    title: `${readyColumns.toLocaleString()} / ${requiredColumns.toLocaleString()} nearby columns ready`,
    subtitle: [
      `${snapshot.streamPendingChunks.toLocaleString()} stream pending`,
      `${snapshot.bootstrapPendingMeshJobs.toLocaleString()} mesh job(s)`,
      `${snapshot.farFieldPendingBands.toLocaleString()} far band(s)`,
    ].join(" • "),
    detail: [
      `Urgent meshless ${snapshot.bootstrapUrgentDirtyMeshlessChunks.toLocaleString()}`,
      `Elapsed ${snapshot.bootstrapElapsedMs.toFixed(0)} ms`,
    ].join(" • "),
    progressRatio: Math.max(0.04, Math.min(0.99, progressRatio)),
    loading: true,
  };
}

function createInteractionHudView(root: HTMLElement): InteractionHudView {
  const targetLabel = document.createElement("p");
  targetLabel.className = "game-target-label";
  const targetTitle = document.createElement("h2");
  targetTitle.className = "game-target-title";
  const targetMeta = document.createElement("p");
  targetMeta.className = "game-target-meta";
  const targetActions = document.createElement("p");
  targetActions.className = "game-target-actions";
  const targetCard = document.createElement("section");
  targetCard.className = "game-target-card";
  targetCard.append(targetLabel, targetTitle, targetMeta, targetActions);

  const hotbarSummary = document.createElement("p");
  hotbarSummary.className = "game-hotbar-summary";

  const hotbarElement = document.createElement("div");
  hotbarElement.className = "game-hotbar";
  const hotbarSlots: Array<{
    root: HTMLElement;
    index: HTMLElement;
    swatch: HTMLElement;
    material: HTMLElement;
    count: HTMLElement;
    fill: HTMLElement;
  }> = [];
  for (let slotIndex = 0; slotIndex < HOTBAR_VISIBLE_SLOT_COUNT; slotIndex += 1) {
    const index = document.createElement("span");
    index.className = "game-hotbar-slot-index";
    const swatch = document.createElement("span");
    swatch.className = "game-hotbar-slot-swatch";
    const material = document.createElement("strong");
    material.className = "game-hotbar-slot-material";
    const count = document.createElement("span");
    count.className = "game-hotbar-slot-count";
    const fill = document.createElement("span");
    fill.className = "game-hotbar-slot-fill";
    const slot = document.createElement("div");
    slot.className = "game-hotbar-slot";
    slot.append(index, swatch, material, count, fill);
    hotbarElement.append(slot);
    hotbarSlots.push({ root: slot, index, swatch, material, count, fill });
  }

  root.replaceChildren(targetCard, hotbarSummary, hotbarElement);

  return {
    update(snapshot, inventory, targeting) {
      if (!targeting.hit) {
        targetLabel.textContent = snapshot.pointerLocked ? "Target" : "Targeting";
        targetTitle.textContent = "No voxel in reach";
        targetMeta.textContent = targeting.placeMaterial === "Empty"
          ? "Look at a nearby block to break it or select a stack to place."
          : `Selected ${targeting.placeMaterial} • look at a nearby surface to place.`;
      } else {
        targetLabel.textContent = "Target";
        targetTitle.textContent = `${targeting.targetMaterial} • ${targeting.distanceMeters.toFixed(1)} m`;
        targetMeta.textContent = [
          `Voxel ${formatCoords(targeting.voxel)}`,
          `Adjacent ${formatCoords(targeting.adjacent)}`,
        ].join(" • ");
      }
      targetActions.textContent = [
        targeting.breakActionLabel,
        targeting.placeActionLabel,
      ].join(" • ");

      const windowLayout = describeHotbarWindow(
        inventory.selectedSlot,
        inventory.slots.length,
        HOTBAR_VISIBLE_SLOT_COUNT,
      );
      const selectedStack = inventory.slots[inventory.selectedSlot];
      const selectedLabel = selectedStack
        ? `Selected ${materialToHexColor(selectedStack.material)} ${selectedStack.count.toLocaleString()} / ${INVENTORY_STACK_SIZE.toLocaleString()}`
        : "Selected empty";
      hotbarSummary.textContent = [
        `Slots ${windowLayout.startSlot + 1}-${windowLayout.endSlotExclusive} of ${inventory.slots.length}`,
        `Stacks ${inventory.usedStacks.toLocaleString()} / ${inventory.slots.length}`,
        selectedLabel,
      ].join(" • ");

      for (let slotOffset = 0; slotOffset < HOTBAR_VISIBLE_SLOT_COUNT; slotOffset += 1) {
        const slotIndex = windowLayout.startSlot + slotOffset;
        const slot = hotbarSlots[slotOffset]!;
        const stack = inventory.slots[slotIndex];
        const materialHex = stack ? materialToHexColor(stack.material) : "Empty";
        if (slotIndex >= inventory.slots.length) {
          slot.root.setAttribute("hidden", "");
          continue;
        }
        slot.root.removeAttribute("hidden");
        slot.root.classList.toggle("is-selected", slotIndex === inventory.selectedSlot);
        slot.root.classList.toggle("is-empty", stack === null);
        slot.index.textContent = String(slotIndex + 1);
        slot.material.textContent = materialHex;
        slot.count.textContent = stack ? stack.count.toLocaleString() : "";
        slot.fill.style.transform = `scaleX(${stack ? stack.count / INVENTORY_STACK_SIZE : 0})`;
        slot.swatch.style.background = stack ? materialToCssColor(materialHex) : "rgba(255, 246, 214, 0.08)";
      }
    },
  };
}

function createTargetingOverlayView(root: SVGSVGElement): TargetingOverlayView {
  const namespace = "http://www.w3.org/2000/svg";
  const face = document.createElementNS(namespace, "polygon");
  face.setAttribute("class", "target-overlay-face");
  const previewFace = document.createElementNS(namespace, "polygon");
  previewFace.setAttribute("class", "target-overlay-preview-face");
  const lines = Array.from({ length: 12 }, () => {
    const line = document.createElementNS(namespace, "line");
    line.setAttribute("class", "target-overlay-line");
    return line;
  });
  const previewLines = Array.from({ length: 12 }, () => {
    const line = document.createElementNS(namespace, "line");
    line.setAttribute("class", "target-overlay-preview-line");
    return line;
  });
  root.replaceChildren(previewFace, face, ...previewLines, ...lines);

  return {
    update(snapshot) {
      root.toggleAttribute("hidden", !snapshot.visible && !snapshot.previewVisible);
      root.classList.toggle("is-breakable", snapshot.breakable);
      root.setAttribute("viewBox", `0 0 ${snapshot.viewportWidth} ${snapshot.viewportHeight}`);
      if (!snapshot.visible && !snapshot.previewVisible) {
        face.setAttribute("points", "");
        previewFace.setAttribute("points", "");
        for (const line of lines) {
          line.setAttribute("visibility", "hidden");
        }
        for (const line of previewLines) {
          line.setAttribute("visibility", "hidden");
        }
        return;
      }
      face.setAttribute("points", snapshot.visible
        ? snapshot.facePolygon.map(([x, y]) => `${x},${y}`).join(" ")
        : "");
      for (let index = 0; index < lines.length; index += 1) {
        const line = lines[index]!;
        const segment = snapshot.visible ? snapshot.outlineSegments[index] : undefined;
        if (!segment) {
          line.setAttribute("visibility", "hidden");
          continue;
        }
        line.setAttribute("x1", segment.from[0].toFixed(2));
        line.setAttribute("y1", segment.from[1].toFixed(2));
        line.setAttribute("x2", segment.to[0].toFixed(2));
        line.setAttribute("y2", segment.to[1].toFixed(2));
        line.setAttribute("visibility", "visible");
      }
      previewFace.setAttribute("points", snapshot.previewVisible
        ? snapshot.previewFacePolygon.map(([x, y]) => `${x},${y}`).join(" ")
        : "");
      const previewColor = materialToCssColor(snapshot.previewMaterial);
      previewFace.style.fill = snapshot.previewVisible ? colorWithAlpha(previewColor, 0.12) : "transparent";
      previewFace.style.stroke = snapshot.previewVisible ? colorWithAlpha(previewColor, 0.72) : "transparent";
      for (let index = 0; index < previewLines.length; index += 1) {
        const line = previewLines[index]!;
        const segment = snapshot.previewVisible ? snapshot.previewOutlineSegments[index] : undefined;
        if (!segment) {
          line.setAttribute("visibility", "hidden");
          continue;
        }
        line.style.stroke = colorWithAlpha(previewColor, 0.95);
        line.setAttribute("x1", segment.from[0].toFixed(2));
        line.setAttribute("y1", segment.from[1].toFixed(2));
        line.setAttribute("x2", segment.to[0].toFixed(2));
        line.setAttribute("y2", segment.to[1].toFixed(2));
        line.setAttribute("visibility", "visible");
      }
    },
  };
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

function materialToCssColor(hex: string): string {
  if (!/^#[0-9A-F]{3}$/i.test(hex)) {
    return "rgba(255, 246, 214, 0.08)";
  }
  const [r, g, b] = hex.slice(1).split("").map((digit) => Number.parseInt(digit + digit, 16));
  return `rgb(${r} ${g} ${b})`;
}

function colorWithAlpha(color: string, alpha: number): string {
  const match = color.match(/^rgb\((\d+) (\d+) (\d+)\)$/);
  if (!match) {
    return color;
  }
  return `rgba(${match[1]}, ${match[2]}, ${match[3]}, ${alpha})`;
}

function formatCoords(coords: readonly number[] | null): string {
  if (!coords) {
    return "none";
  }
  return `${coords[0]}, ${coords[1]}, ${coords[2]}`;
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
    snapshot.streamCompletedChunkCacheHits.toLocaleString(),
    snapshot.streamCompletedGeneratedChunks.toLocaleString(),
    snapshot.generationWorkerCount.toLocaleString(),
    snapshot.streamCompletedSummaryCacheHits.toLocaleString(),
    snapshot.streamCompletedGeneratedSummaries.toLocaleString(),
    snapshot.streamCompletedRegionSummaryCacheHits.toLocaleString(),
    snapshot.streamMissingRegionSummaries.toLocaleString(),
    formatDiscoveryInline("biome", snapshot.biomeId, "Unknown"),
    formatDiscoveryInline("underground", snapshot.undergroundBiomeId, "Surface"),
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
