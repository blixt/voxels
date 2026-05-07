import {
  GameController,
  type GameHudSnapshot,
  type TargetingOverlaySnapshot,
  type TargetingSnapshot,
} from "./game-controller.ts";
import {
  describeExplorationObjectives,
  type ExplorationObjectiveSnapshot,
} from "../engine/exploration-objectives.ts";
import { describeHotbarWindow } from "../engine/hotbar-layout.ts";
import { INVENTORY_STACK_SIZE } from "../engine/inventory.ts";
import { materialToHexColor } from "../engine/procedural-generator.ts";
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
      getDiscoveryJournal(): ExplorationJournalSnapshot;
      resetDiscoveryJournal(): ExplorationJournalSnapshot;
      getSkillJournal(): ReturnType<GameController["getSkillJournalSnapshot"]>;
      exportProgressState(): ReturnType<GameController["exportProgressState"]>;
      importProgressState(state: Parameters<GameController["importProgressState"]>[0]): ReturnType<GameController["importProgressState"]>;
      saveProgressState(): boolean;
      loadProgressState(): boolean;
      clearProgressState(): boolean;
      getExplorationObjectives(): ExplorationObjectiveSnapshot;
      getInventory(): ReturnType<GameController["getInventorySnapshot"]>;
      getInventoryPanelOpen(): ReturnType<GameController["getInventoryPanelOpen"]>;
      setInventoryPanelOpen(open: boolean): ReturnType<GameController["setInventoryPanelOpen"]>;
      toggleInventoryPanel(): ReturnType<GameController["toggleInventoryPanel"]>;
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

interface InventoryPanelView {
  update(snapshot: GameHudSnapshot, inventory: ReturnType<GameController["getInventorySnapshot"]>): void;
}

interface ObjectivePanelView {
  update(snapshot: GameHudSnapshot): void;
}

interface TargetingOverlayView {
  update(snapshot: TargetingOverlaySnapshot): void;
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
const HOTBAR_VISIBLE_SLOT_COUNT = 9;
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
  const inventoryPanelRoot = appRoot.querySelector<HTMLElement>("[data-role='inventory-panel']");
  const targetOverlayRoot = appRoot.querySelector<SVGSVGElement>("[data-role='target-overlay']");

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
    || !inventoryPanelRoot
    || !targetOverlayRoot
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
  const inventoryPanelView = createInventoryPanelView(inventoryPanelRoot);
  const targetingOverlayView = createTargetingOverlayView(targetOverlayRoot);
  let targetingOverlayFrameId = 0;
  let lastProgressSignature = "";

  controller.onHudUpdate = (snapshot) => {
    achievementPresenter.observe(snapshot.recentDiscoveries);
    objectivePanelView.update(snapshot);
    performanceStripView.update(snapshot);
    const inventorySnapshot = controller.getInventorySnapshot();
    interactionHudView.update(snapshot, inventorySnapshot, controller.getTargetingSnapshot());
    inventoryPanelView.update(snapshot, inventorySnapshot);
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
    setStreamingBudgets: (maxGeneratedChunksPerUpdate, maxMeshRebuildsPerFrame) =>
      controller.setStreamingBudgets(
        maxGeneratedChunksPerUpdate,
        maxMeshRebuildsPerFrame,
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
    getDiscoveryJournal: () => controller.getDiscoveryJournalSnapshot(),
    resetDiscoveryJournal: () => controller.resetDiscoveryJournal(),
    getSkillJournal: () => controller.getSkillJournalSnapshot(),
    exportProgressState: () => controller.exportProgressState(),
    importProgressState: (state) => controller.importProgressState(state),
    saveProgressState: () => saveProgressState(controller),
    loadProgressState: () => loadProgressState(controller),
    clearProgressState: () => clearProgressState(),
    getExplorationObjectives: () => describeExplorationObjectives(snapshotToObjectiveSource(controller.getDebugSnapshot())),
    getInventory: () => controller.getInventorySnapshot(),
    getInventoryPanelOpen: () => controller.getInventoryPanelOpen(),
    setInventoryPanelOpen: (open) => controller.setInventoryPanelOpen(open),
    toggleInventoryPanel: () => controller.toggleInventoryPanel(),
    getTargeting: () => controller.getTargetingSnapshot(),
    getTargetingOverlay: (viewportWidth = canvas.clientWidth || canvas.width || 1, viewportHeight = canvas.clientHeight || canvas.height || 1) =>
      controller.getTargetingOverlaySnapshot(viewportWidth, viewportHeight),
    getEditLog: () => controller.getEditLogSnapshot(),
    breakTargetVoxel: () => controller.breakTargetVoxel(),
    placeSelectedVoxel: () => controller.placeSelectedVoxel(),
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
      cancelAnimationFrame(targetingOverlayFrameId);
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
  const targetTitle = document.createElement("h2");
  targetTitle.className = "game-target-title";
  const targetMeta = document.createElement("p");
  targetMeta.className = "game-target-meta";
  const targetCard = document.createElement("section");
  targetCard.className = "game-target-card";
  targetCard.hidden = true;
  targetCard.append(targetTitle, targetMeta);

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

  root.replaceChildren(targetCard, hotbarElement);

  return {
    update(_snapshot, inventory, targeting) {
      if (!targeting.hit) {
        targetCard.hidden = true;
      } else {
        targetCard.hidden = false;
        targetTitle.textContent = targeting.targetMaterial;
        targetMeta.textContent = `${targeting.distanceMeters.toFixed(1)} m`;
      }

      const windowLayout = describeHotbarWindow(
        inventory.selectedSlot,
        inventory.slots.length,
        HOTBAR_VISIBLE_SLOT_COUNT,
      );

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
        slot.root.title = stack
          ? `Slot ${slotIndex + 1}: ${materialHex}, ${stack.count} / ${INVENTORY_STACK_SIZE}`
          : `Slot ${slotIndex + 1}: empty`;
      }
    },
  };
}

function createInventoryPanelView(root: HTMLElement): InventoryPanelView {
  const title = document.createElement("h2");
  title.className = "game-inventory-panel-title";
  title.textContent = "Inventory";
  const summary = document.createElement("p");
  summary.className = "game-inventory-panel-summary";
  const grid = document.createElement("div");
  grid.className = "game-inventory-grid";

  const slots: Array<{
    root: HTMLElement;
    index: HTMLElement;
    swatch: HTMLElement;
    material: HTMLElement;
    count: HTMLElement;
    fill: HTMLElement;
  }> = [];

  for (let slotIndex = 0; slotIndex < 32; slotIndex += 1) {
    const index = document.createElement("span");
    index.className = "game-inventory-slot-index";
    const swatch = document.createElement("span");
    swatch.className = "game-inventory-slot-swatch";
    const material = document.createElement("strong");
    material.className = "game-inventory-slot-material";
    const count = document.createElement("span");
    count.className = "game-inventory-slot-count";
    const fill = document.createElement("span");
    fill.className = "game-inventory-slot-fill";
    const slot = document.createElement("div");
    slot.className = "game-inventory-slot";
    slot.append(index, swatch, material, count, fill);
    grid.append(slot);
    slots.push({ root: slot, index, swatch, material, count, fill });
  }

  root.replaceChildren(title, summary, grid);

  return {
    update(snapshot, inventory) {
      root.toggleAttribute("hidden", !snapshot.inventoryPanelOpen);
      if (!snapshot.inventoryPanelOpen) {
        return;
      }
      summary.textContent = [
        `Used ${inventory.usedStacks.toLocaleString()} / ${inventory.slots.length} stacks`,
        `Selected slot ${inventory.selectedSlot + 1}`,
        snapshot.selectedInventoryMaterial === "Empty"
          ? "Selected empty"
          : `${snapshot.selectedInventoryMaterial} ${snapshot.selectedInventoryCount.toLocaleString()} / ${INVENTORY_STACK_SIZE.toLocaleString()}`,
      ].join(" • ");
      for (let slotIndex = 0; slotIndex < slots.length; slotIndex += 1) {
        const slotView = slots[slotIndex]!;
        const stack = inventory.slots[slotIndex];
        const materialHex = stack ? materialToHexColor(stack.material) : "Empty";
        slotView.root.classList.toggle("is-selected", slotIndex === inventory.selectedSlot);
        slotView.root.classList.toggle("is-empty", stack === null);
        slotView.index.textContent = String(slotIndex + 1);
        slotView.material.textContent = materialHex;
        slotView.count.textContent = stack ? stack.count.toLocaleString() : "";
        slotView.fill.style.transform = `scaleX(${stack ? stack.count / INVENTORY_STACK_SIZE : 0})`;
        slotView.swatch.style.background = stack ? materialToCssColor(materialHex) : "rgba(255, 246, 214, 0.08)";
      }
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
      stage.textContent = `${snapshot.ambientProfileLabel} • ${objectiveSnapshot.completedCount}/${objectiveSnapshot.totalCount}`;
      root.title = [
        objectiveSnapshot.subtitle,
        `${objectiveSnapshot.title}: ${objectiveSnapshot.completedCount}/${objectiveSnapshot.totalCount}`,
        `${snapshot.focusSkillName} ${snapshot.focusSkillLevel}`,
        `Fog ${snapshot.ambientFogEndMeters.toFixed(0)} m`,
      ].join(" • ");
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

function snapshotToObjectiveSource(snapshot: GameHudSnapshot) {
  return {
    discoveredBiomeCount: snapshot.discoveredBiomeCount,
    discoveredUndergroundBiomeCount: snapshot.discoveredUndergroundBiomeCount,
    discoveredRegionalVariantCount: snapshot.discoveredRegionalVariantCount,
    discoveredLandmarkCount: snapshot.discoveredLandmarkCount,
    discoveredAncientLandmarkCount: snapshot.discoveredAncientLandmarkCount,
    collectedMaterialCount: snapshot.collectedMaterialCount,
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
    categoryElement.textContent = `${event.categoryLabel} Discovered`;
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
  const fps = document.createElement("strong");
  fps.className = "game-performance-value";
  const chunks = document.createElement("span");
  chunks.className = "game-performance-value";
  const draws = document.createElement("span");
  draws.className = "game-performance-value";
  root.replaceChildren(fps, chunks, draws);

  const lastValues = ["", "", ""];
  return {
    update(snapshot) {
      const nextValues = [
        `${formatFps(snapshot.avgFrameWallMs)} FPS`,
        `${formatCompactCount(snapshot.chunkCount)} chunks`,
        `${formatCompactCount(snapshot.drawCalls)} draws`,
      ];
      for (let index = 0; index < nextValues.length; index += 1) {
        if (nextValues[index] === lastValues[index]) {
          continue;
        }
        root.children[index]!.textContent = nextValues[index]!;
        lastValues[index] = nextValues[index]!;
      }
      root.title = [
        `Frame ${snapshot.lastFrameWallMs.toFixed(1)} ms`,
        `CPU ${snapshot.lastFrameCpuMs.toFixed(1)} ms`,
        `Stream ${snapshot.streamMs.toFixed(1)} ms`,
        `Mesh ${snapshot.meshMs.toFixed(1)} ms`,
        `LOD ${snapshot.lodChunkCount.toLocaleString()}`,
        `Fog culled ${snapshot.fogCulledChunks.toLocaleString()}`,
      ].join(" • ");
    },
  };
}

function formatFps(avgFrameWallMs: number): string {
  if (!Number.isFinite(avgFrameWallMs) || avgFrameWallMs <= 0) {
    return "0";
  }
  const value = 1000 / avgFrameWallMs;
  return value >= 100 ? value.toFixed(0) : value.toFixed(1);
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
