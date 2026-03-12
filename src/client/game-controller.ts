import { average, maxValue, percentile } from "../engine/benchmark-metrics.ts";
import {
  analyzeSettledReferenceDiff,
  analyzeBottomCenterVoid,
  buildDefaultRouteBenchmarkPlan,
  buildForwardRouteBenchmarkPlan,
  summarizeRouteFrameAccounting,
  type BottomCenterVoidProbe,
  type RouteBenchmarkFrameTarget,
} from "../engine/game-route-benchmark.ts";
import {
  summarizeBootstrapBenchmark,
  type BootstrapBenchmarkSample,
  type BootstrapBenchmarkSummary,
} from "../engine/game-bootstrap-benchmark.ts";
import {
  buildFirstPersonCameraMatrices,
  createFirstPersonCamera,
  createForwardRay,
  rotateFirstPersonCamera,
  type FirstPersonCameraState,
} from "../engine/first-person-camera.ts";
import {
  ExplorationJournal,
  type DiscoveryEvent,
  type ExplorationJournalSnapshot,
  type ExplorationObservation,
} from "../engine/exploration-journal.ts";
import {
  breakVoxelAlongRay,
  placeSelectedVoxelAlongRay,
} from "../engine/interaction-loop.ts";
import {
  countUsedInventoryStacks,
  createInventoryState,
  cycleInventorySlot,
  getSelectedInventoryStack,
  selectInventorySlot,
  type InventoryState,
} from "../engine/inventory.ts";
import {
  buildChunkMesh,
  buildChunkMeshFromOpaqueGeometry,
  collectDirtyChunks,
  createOpaqueChunkMeshingInput,
  rebuildDirtyMeshes,
} from "../engine/mesher.ts";
import { createMeshMaterialLut } from "../engine/opaque-chunk-mesher.ts";
import { createAsyncChunkMeshing } from "./async-chunk-meshing.ts";
import { createAsyncProceduralChunkGeneration } from "./async-procedural-chunk-generation.ts";
import {
  createPlayerState,
  getPlayerEyePosition,
  PLAYER_EYE_HEIGHT,
  stepPlayer,
  teleportPlayerToEyePosition,
  type PlayerState,
} from "../engine/player-physics.ts";
import { ProceduralFarField, type FarFieldUpdateSummary } from "../engine/procedural-far-field.ts";
import {
  diffChunkCoords,
  summarizeResidentWorld,
  type ResidentWorldProbeSnapshot,
} from "../engine/procedural-probes.ts";
import {
  ProceduralResidentWorld,
  type ResidencyUpdateSummary,
  type WorldEditRecord,
} from "../engine/procedural-resident-world.ts";
import { ProceduralWorldGenerator, materialToHexColor } from "../engine/procedural-generator.ts";
import {
  WebGpuVoxelRenderer,
  type FarFieldRenderMask,
  type RenderStats,
} from "../engine/renderer.ts";
import { metersToWorldUnits, worldUnitsToMeters } from "../engine/scale.ts";
import {
  shouldAllowFarFieldCatchupWhileMoving,
  shouldPumpWorldWork,
  shouldRefreshResidency,
} from "../engine/stream-work.ts";
import {
  buildStreamAnchorPosition,
  resolveStreamAnchor,
  type StreamAnchor,
} from "../engine/stream-anchor.ts";
import type { Vec3 } from "../engine/types.ts";
import {
  buildUnderwaterRenderEnvironment,
  DEFAULT_RENDER_ENVIRONMENT,
  type RenderEnvironment,
} from "../engine/water-visuals.ts";
import { resolveObservedUndergroundBiomeId } from "../engine/underground-discovery.ts";
import type { MeshBuildSummary } from "../engine/mesher.ts";

const MAX_DELTA_SECONDS = 0.05;
const HUD_PUSH_INTERVAL_MS = 120;
const STREAM_ANCHOR_MARGIN_CHUNKS = 1;
const DEFAULT_MAX_GENERATED_CHUNKS_PER_UPDATE = 7;
const DEFAULT_MAX_MESH_REBUILDS_PER_FRAME = 6;
const DEFAULT_MAX_FAR_FIELD_BAND_REBUILDS_PER_FRAME = 1;
const DEFAULT_MAX_FAR_FIELD_SURFACE_PREFETCH_CHUNKS_PER_FRAME = 8;
const MAX_SYNC_NEAR_MESH_REBUILDS_PER_FRAME = 6;
const SYNC_NEAR_MESH_RADIUS_CHUNKS = 3;
const BOOTSTRAP_PLAYABLE_COLUMN_RADIUS_CHUNKS = 2;
const MOVEMENT_FAR_FIELD_CATCHUP_CADENCE_FRAMES = 6;
const FAR_FIELD_RENDER_MASK_SPAN_CHUNKS = 32;
const DISCOVERY_SAMPLE_INTERVAL_MS = 250;
const DISCOVERY_SAMPLE_MOVE_THRESHOLD_WORLD_UNITS = metersToWorldUnits(0.8);
const INTERACTION_REACH_WORLD_UNITS = metersToWorldUnits(5);
const DISCOVERY_LANDMARK_SAMPLE_OFFSETS: ReadonlyArray<readonly [number, number]> = [
  [0, 0],
  [metersToWorldUnits(1.2), 0],
  [-metersToWorldUnits(1.2), 0],
  [0, metersToWorldUnits(1.2)],
  [0, -metersToWorldUnits(1.2)],
  [metersToWorldUnits(1.2), metersToWorldUnits(1.2)],
  [metersToWorldUnits(1.2), -metersToWorldUnits(1.2)],
  [-metersToWorldUnits(1.2), metersToWorldUnits(1.2)],
  [-metersToWorldUnits(1.2), -metersToWorldUnits(1.2)],
  [metersToWorldUnits(2.4), 0],
  [-metersToWorldUnits(2.4), 0],
  [0, metersToWorldUnits(2.4)],
  [0, -metersToWorldUnits(2.4)],
];

export interface GameHudSnapshot {
  status: string;
  pointerLocked: boolean;
  position: Vec3;
  feetPosition: Vec3;
  playerChunk: [number, number, number];
  streamAnchorChunk: [number, number];
  grounded: boolean;
  bodyInWater: boolean;
  eyeInWater: boolean;
  yawDegrees: number;
  pitchDegrees: number;
  solidVoxelCount: number;
  chunkCount: number;
  paletteCount: number;
  streamMs: number;
  streamGeneratedChunks: number;
  streamEvictedChunks: number;
  streamPendingChunks: number;
  streamEmptyChunksSkipped: number;
  streamCachedEmptyChunkHits: number;
  streamCompletedChunkCacheHits: number;
  streamCompletedGeneratedChunks: number;
  generationWorkerCount: number;
  streamCompletedSummaryCacheHits: number;
  streamCompletedGeneratedSummaries: number;
  streamCompletedRegionSummaryCacheHits: number;
  streamMissingRegionSummaries: number;
  streamDirtyResidentChunks: number;
  residencyRadiusChunks: number;
  surfaceY: number;
  biomeId: string | null;
  undergroundBiomeId: string | null;
  regionalVariantId: string | null;
  landmarkId: string | null;
  discoveredBiomeCount: number;
  discoveredUndergroundBiomeCount: number;
  discoveredRegionalVariantCount: number;
  discoveredLandmarkCount: number;
  recentDiscoveries: DiscoveryEvent[];
  lastDiscoveryLabel: string;
  selectedInventorySlot: number;
  selectedInventoryMaterial: string;
  selectedInventoryCount: number;
  usedInventoryStacks: number;
  bootstrapPlayableReady: boolean;
  bootstrapVisualReady: boolean;
  bootstrapElapsedMs: number;
  farFieldMs: number;
  farFieldBuiltBands: number;
  farFieldPendingBands: number;
  farFieldTriangles: number;
  farFieldMaxRadiusMeters: number;
  meshMs: number;
  meshNewChunks: number;
  meshRemeshChunks: number;
  drawCalls: number;
  triangles: number;
  lastFrameCpuMs: number;
  lastFrameSyncMs: number;
  lastFrameUploadMs: number;
  lastFrameUploadChunks: number;
  lastFrameUploadBytes: number;
  lastFrameEncodeMs: number;
  avgFrameCpuMs: number;
  maxGeneratedChunksPerUpdate: number;
  maxMeshRebuildsPerFrame: number;
  maxFarFieldBandRebuildsPerFrame: number;
}

export interface GameRenderProbe {
  frameCpuMs: number;
  syncResourcesMs: number;
  uploadMs: number;
  uploadChunks: number;
  uploadBytes: number;
  encodeMs: number;
  drawCalls: number;
  triangles: number;
}

export interface ResidencyTransitionProbe {
  before: ResidentWorldProbeSnapshot;
  after: ResidentWorldProbeSnapshot;
  enteredChunkCoords: Array<[number, number, number]>;
  evictedChunkCoords: Array<[number, number, number]>;
  generatedChunkCoords: Array<[number, number, number]>;
  residency: ResidencyUpdateSummary;
  mesh: MeshBuildSummary;
  render: GameRenderProbe;
  settleFrames: number;
  settled: boolean;
}

export interface ChunkBoundaryBenchmarkSample {
  step: number;
  targetEyePosition: Vec3;
  targetChunk: [number, number, number];
  changed: boolean;
  generatedChunks: number;
  evictedChunks: number;
  streamMs: number;
  meshMs: number;
  meshNewChunks: number;
  meshRemeshChunks: number;
  frameCpuMs: number;
  syncMs: number;
  uploadMs: number;
  uploadChunks: number;
  uploadBytes: number;
  encodeMs: number;
}

export interface ChunkBoundaryBenchmarkSummary {
  sampleCount: number;
  changedCount: number;
  avgStreamMs: number;
  p95StreamMs: number;
  maxStreamMs: number;
  avgMeshMs: number;
  p95MeshMs: number;
  maxMeshMs: number;
  avgFrameCpuMs: number;
  p95FrameCpuMs: number;
  maxFrameCpuMs: number;
  avgSyncMs: number;
  p95SyncMs: number;
  maxSyncMs: number;
  avgUploadMs: number;
  p95UploadMs: number;
  maxUploadMs: number;
  avgUploadChunks: number;
  maxUploadChunks: number;
  avgUploadBytes: number;
  maxUploadBytes: number;
}

export interface ChunkBoundaryBenchmark {
  iterations: number;
  chunkDelta: number;
  radiusChunks: number;
  samples: ChunkBoundaryBenchmarkSample[];
  summary: ChunkBoundaryBenchmarkSummary;
}

export interface ChunkCacheReuseLegSummary {
  targetChunk: [number, number, number];
  frameCount: number;
  settled: boolean;
  totalStreamMs: number;
  totalMeshMs: number;
  totalFarFieldMs: number;
  totalGeneratedChunks: number;
  totalPersistedChunkHits: number;
  totalPersistedSummaryHits: number;
  totalPersistedRegionSummaryHits: number;
  totalMissingRegionSummaries: number;
  totalWorkerGeneratedChunks: number;
  maxPendingChunks: number;
  residentChunks: number;
}

export interface ChunkCacheReuseBenchmark {
  chunkDelta: number;
  radiusChunks: number;
  populate: ChunkCacheReuseLegSummary;
  revisit: ChunkCacheReuseLegSummary;
}

export interface IncrementalCrossingSample {
  frame: number;
  phase: "move" | "settle";
  leg: number;
  changed: boolean;
  complete: boolean;
  pendingChunks: number;
  generatedChunks: number;
  evictedChunks: number;
  streamMs: number;
  meshMs: number;
  meshCount: number;
  farFieldMs: number;
  farFieldBuiltBands: number;
  farFieldPendingBands: number;
  residentNearSamples: number;
  renderReadyNearSamples: number;
  residentNotReadyNearSamples: number;
  frameCpuMs: number;
  syncMs: number;
  uploadMs: number;
  uploadChunks: number;
  uploadBytes: number;
  encodeMs: number;
}

export interface IncrementalCrossingSummary {
  sampleCount: number;
  workFrameCount: number;
  changedCount: number;
  incompleteFrameCount: number;
  avgWorkMs: number;
  p95WorkMs: number;
  maxWorkMs: number;
  avgFarFieldMs: number;
  p95FarFieldMs: number;
  maxFarFieldMs: number;
  avgResidentNotReadyNearSamples: number;
  maxResidentNotReadyNearSamples: number;
  avgStreamMs: number;
  p95StreamMs: number;
  maxStreamMs: number;
  avgMeshMs: number;
  p95MeshMs: number;
  maxMeshMs: number;
  avgFrameCpuMs: number;
  p95FrameCpuMs: number;
  maxFrameCpuMs: number;
  avgUploadMs: number;
  p95UploadMs: number;
  maxUploadMs: number;
  maxPendingChunks: number;
}

export interface IncrementalCrossingBenchmark {
  iterations: number;
  chunkDelta: number;
  stepsPerLeg: number;
  settleFrames: number;
  radiusChunks: number;
  samples: IncrementalCrossingSample[];
  summary: IncrementalCrossingSummary;
}

export interface StreamingBudgets {
  maxGeneratedChunksPerUpdate: number;
  maxMeshRebuildsPerFrame: number;
  maxFarFieldBandRebuildsPerFrame: number;
}

export interface LodCoverageIssueSample {
  worldX: number;
  worldZ: number;
  distanceMeters: number;
  bands: string[];
  sampleStrideMeters: number[];
}

export interface LodCoverageProbe {
  center: Vec3;
  sampleRadiusMeters: number;
  sampleStepMeters: number;
  sampleCount: number;
  residentSampleCount: number;
  renderReadySampleCount: number;
  coveredSampleCount: number;
  residentOverlapCount: number;
  uncoveredGapCount: number;
  handoffHoleCount: number;
  bandOverlapCount: number;
  wrongBandCount: number;
  residentOverlapSamples: LodCoverageIssueSample[];
  uncoveredGapSamples: LodCoverageIssueSample[];
  handoffHoleSamples: LodCoverageIssueSample[];
  bandOverlapSamples: LodCoverageIssueSample[];
  wrongBandSamples: LodCoverageIssueSample[];
}

export interface RenderReadyCoverageProbe {
  center: Vec3;
  sampleRadiusMeters: number;
  sampleStepMeters: number;
  sampleCount: number;
  residentSampleCount: number;
  renderReadySampleCount: number;
  residentNotReadyCount: number;
  missingResidentCount: number;
}

export interface NearFarSeamProbe {
  boundaryCount: number;
  gapCount: number;
  maxGapDepthWorldUnits: number;
  maxGapDepthMeters: number;
  samples: Array<{
    band: string;
    direction: "east" | "west" | "south" | "north";
    worldX: number;
    worldZ: number;
    gapDepthWorldUnits: number;
    gapDepthMeters: number;
  }>;
}

export interface FarFieldSurfaceGapProbe {
  boundaryCount: number;
  gapCount: number;
  maxGapDepthWorldUnits: number;
  maxGapDepthMeters: number;
  samples: Array<{
    band: string;
    kind: "masked" | "downward";
    direction: "east" | "west" | "south" | "north";
    worldX: number;
    worldZ: number;
    gapDepthWorldUnits: number;
    gapDepthMeters: number;
  }>;
}

export interface VisibleGroundCoverageIssueSample {
  worldX: number;
  worldZ: number;
  forwardMeters: number;
  lateralMeters: number;
  resident: boolean;
  renderReady: boolean;
  farCovered: boolean;
}

export interface VisibleGroundCoverageProbe {
  center: Vec3;
  yawRadians: number;
  sampleForwardMeters: number;
  sampleLateralMeters: number;
  sampleStepMeters: number;
  sampleCount: number;
  renderReadyCount: number;
  farOnlyCount: number;
  uncoveredCount: number;
  residentNotReadyCount: number;
  uncoveredSamples: VisibleGroundCoverageIssueSample[];
}

export interface RouteExperienceBenchmarkOptions {
  durationSeconds?: number;
  settleSeconds?: number;
  sampleHz?: number;
  speedMetersPerSecond?: number;
  seamProbeStrideFrames?: number;
  captureStrideFrames?: number;
  captureWidth?: number;
  captureHeight?: number;
  referenceDiffStrideFrames?: number;
  referenceDiffLimit?: number;
}

export interface RouteExperienceFrameSample {
  frame: number;
  phase: "move" | "settle";
  simTimeSeconds: number;
  routeDistanceMeters: number;
  feetPosition: Vec3;
  yaw: number;
  pitch: number;
  changed: boolean;
  complete: boolean;
  pendingChunks: number;
  pendingMeshJobs: number;
  dirtyResidentChunks: number;
  dirtyMeshlessResidentChunks: number;
  dirtyRetainedMeshResidentChunks: number;
  generatedChunks: number;
  evictedChunks: number;
  movementMs: number;
  streamMs: number;
  meshMs: number;
  meshCount: number;
  farFieldMs: number;
  farFieldSampleCacheMs: number;
  farFieldMeshBuildMs: number;
  farFieldSampledCellCount: number;
  farFieldMaxBandMs: number;
  farFieldMaxBandLabel: string | null;
  farFieldBuiltBands: number;
  farFieldPendingBands: number;
  gameplayFrameMs: number;
  accountedFrameMs: number;
  unmeasuredFrameMs: number;
  diagnosticsMs: number;
  captureDiagnosticsMs: number;
  renderCpuMs: number;
  renderSyncMs: number;
  renderUploadMs: number;
  renderEncodeMs: number;
  renderOtherMs: number;
  uploadChunks: number;
  uploadBytes: number;
  drawCalls: number;
  triangles: number;
  residentNearSamples: number;
  renderReadyNearSamples: number;
  residentNotReadyNearSamples: number;
  visibleGroundSampleCount: number;
  visibleGroundUncoveredCount: number;
  visibleGroundResidentNotReadyCount: number;
  visibleGroundFarOnlyCount: number;
  seamGapCount: number;
  maxSeamGapMeters: number;
  screenVoidRatio: number | null;
  screenVoidMaxRunRatio: number | null;
  screenVoidSuspicious: boolean;
  settledReferenceChangedRatio: number | null;
  settledReferenceClearToFilledRatio: number | null;
  settledReferenceMaxClearToFilledRunRatio: number | null;
  settledReferenceSuspiciousHole: boolean;
  suspiciousHole: boolean;
}

export interface RouteExperienceBenchmarkSummary {
  sampleCount: number;
  moveFrameCount: number;
  settleFrameCount: number;
  incompleteFrameCount: number;
  totalDistanceMeters: number;
  sampleHz: number;
  speedMetersPerSecond: number;
  totalGameplayFrameMs: number;
  totalAccountedFrameMs: number;
  totalUnmeasuredFrameMs: number;
  unmeasuredFrameRatio: number;
  totalDiagnosticsMs: number;
  totalCaptureDiagnosticsMs: number;
  avgGameplayFrameMs: number;
  p95GameplayFrameMs: number;
  maxGameplayFrameMs: number;
  avgMeasuredWorkMs: number;
  p95MeasuredWorkMs: number;
  maxMeasuredWorkMs: number;
  avgUnmeasuredFrameMs: number;
  p95UnmeasuredFrameMs: number;
  maxUnmeasuredFrameMs: number;
  avgStreamMs: number;
  p95StreamMs: number;
  maxStreamMs: number;
  avgMeshMs: number;
  p95MeshMs: number;
  maxMeshMs: number;
  avgFarFieldMs: number;
  p95FarFieldMs: number;
  maxFarFieldMs: number;
  avgFarFieldSampleCacheMs: number;
  maxFarFieldSampleCacheMs: number;
  avgFarFieldMeshBuildMs: number;
  maxFarFieldMeshBuildMs: number;
  avgFarFieldSampledCellCount: number;
  maxFarFieldSampledCellCount: number;
  maxFarFieldBandBuildMs: number;
  maxFarFieldBandLabel: string | null;
  avgRenderCpuMs: number;
  p95RenderCpuMs: number;
  maxRenderCpuMs: number;
  avgRenderOtherMs: number;
  maxRenderOtherMs: number;
  avgResidentNotReadyNearSamples: number;
  maxResidentNotReadyNearSamples: number;
  avgVisibleGroundUncoveredCount: number;
  maxVisibleGroundUncoveredCount: number;
  avgVisibleGroundResidentNotReadyCount: number;
  maxVisibleGroundResidentNotReadyCount: number;
  framesWithVisibleGroundGaps: number;
  framesWithSeamGaps: number;
  framesWithScreenVoidSignals: number;
  framesWithSettledReferenceHoleSignals: number;
  framesWithHoleSignals: number;
  maxScreenVoidRatio: number;
  maxSettledReferenceChangedRatio: number;
  maxSettledReferenceClearToFilledRatio: number;
  maxSettledReferenceClearToFilledRunRatio: number;
  maxPendingChunks: number;
  maxPendingMeshJobs: number;
  maxDirtyResidentChunks: number;
  maxDirtyMeshlessResidentChunks: number;
  maxDirtyRetainedMeshResidentChunks: number;
  settleFramesUntilComplete: number | null;
}

export interface RouteExperienceBenchmark {
  seed: number;
  radiusChunks: number;
  captureStrideFrames: number;
  seamProbeStrideFrames: number;
  referenceDiffStrideFrames: number;
  referenceDiffLimit: number;
  durationSeconds: number;
  settleSeconds: number;
  totalDistanceMeters: number;
  sampleHz: number;
  speedMetersPerSecond: number;
  samples: RouteExperienceFrameSample[];
  summary: RouteExperienceBenchmarkSummary;
}

export interface BootstrapExperienceBenchmark {
  completed: boolean;
  startedAtMs: number;
  samples: BootstrapBenchmarkSample[];
  summary: BootstrapBenchmarkSummary;
}

interface GameControllerOptions {
  eagerBootstrapBenchmark?: boolean;
}

interface CapturedBenchmarkFrame {
  sampleIndex: number;
  target: Pick<RouteBenchmarkFrameTarget, "frame" | "simTimeSeconds" | "distanceMeters" | "feetPosition" | "yaw" | "pitch">
    & { phase: "move" | "settle" };
  image: {
    width: number;
    height: number;
    pixels: Uint8ClampedArray;
  };
}

export class GameController {
  readonly canvas: HTMLCanvasElement;
  readonly generator = new ProceduralWorldGenerator(1337);
  readonly asyncChunkGeneration = createAsyncProceduralChunkGeneration(this.generator);
  readonly world = new ProceduralResidentWorld(this.generator, {
    asyncChunkGeneration: this.asyncChunkGeneration,
  });
  readonly asyncChunkMeshing = createAsyncChunkMeshing(
    createMeshMaterialLut(this.world.palette, (materialIndex) => this.world.isWaterMaterial(materialIndex)),
  );
  readonly farField = new ProceduralFarField(this.world);
  readonly explorationJournal = new ExplorationJournal();
  readonly inventory: InventoryState = createInventoryState();

  renderer: WebGpuVoxelRenderer | null = null;
  camera: FirstPersonCameraState = createFirstPersonCamera([0.5, 1500, 0.5]);
  player: PlayerState = createPlayerState([0.5, 1500 - PLAYER_EYE_HEIGHT, 0.5]);
  meshMs = 0;
  drawCalls = 0;
  triangles = 0;
  lastFrameCpuMs = 0;
  lastGameplayFrameMs = 0;
  avgFrameCpuMs = 0;
  status = "Booting";
  pointerLocked = false;
  onHudUpdate: ((snapshot: GameHudSnapshot) => void) | null = null;
  private lastMeshBuildSummary: MeshBuildSummary = {
    meshCount: 0,
    newMeshCount: 0,
    remeshCount: 0,
    triangleCount: 0,
    elapsedMs: 0,
  };
  private lastRenderStats: RenderStats = zeroRenderStats();
  private lastStreamSummary: ResidencyUpdateSummary = cloneResidencySummary(this.world.lastResidency);
  private lastFarFieldSummary: FarFieldUpdateSummary = this.farField.lastUpdate;
  private streamAnchor: StreamAnchor | null = null;
  private farFieldReadyMaskRevision = 0;
  private presentedFarFieldReadyMaskRevision = 0;
  private streamingBudgets: StreamingBudgets = {
    maxGeneratedChunksPerUpdate: DEFAULT_MAX_GENERATED_CHUNKS_PER_UPDATE,
    maxMeshRebuildsPerFrame: DEFAULT_MAX_MESH_REBUILDS_PER_FRAME,
    maxFarFieldBandRebuildsPerFrame: DEFAULT_MAX_FAR_FIELD_BAND_REBUILDS_PER_FRAME,
  };

  private rafId = 0;
  private lastFrameTime = 0;
  private lastHudPushAt = 0;
  private interactiveFrameNumber = 0;
  private cachedFarFieldRenderMaskRevision = -1;
  private cachedFarFieldRenderMaskOriginChunkX = Number.NaN;
  private cachedFarFieldRenderMaskOriginChunkZ = Number.NaN;
  private cachedFarFieldRenderMask: FarFieldRenderMask | null = null;
  private readonly pressedKeys = new Set<string>();
  private lastDiscoverySampleAt = 0;
  private lastDiscoverySampleFeetPosition: Vec3 | null = null;
  private lastDiscoverySnapshot: ExplorationJournalSnapshot = this.explorationJournal.getSnapshot();
  private readonly bootstrapBenchmarkStartedAt = performance.now();
  private readonly bootstrapBenchmarkSamples: BootstrapBenchmarkSample[] = [];
  private bootstrapPlayableReady = false;
  private bootstrapBenchmarkComplete = false;
  private readonly eagerBootstrapBenchmark: boolean;

  constructor(canvas: HTMLCanvasElement, options: GameControllerOptions = {}) {
    this.canvas = canvas;
    this.eagerBootstrapBenchmark = options.eagerBootstrapBenchmark ?? false;
  }

  async init(): Promise<void> {
    this.renderer = await WebGpuVoxelRenderer.create(this.canvas);
    this.loadBootstrapWorld();
    this.attachInteractions();
    if (this.eagerBootstrapBenchmark) {
      await this.drainBootstrapBenchmark();
    }
    this.start();
  }

  dispose(): void {
    cancelAnimationFrame(this.rafId);
    if (document.pointerLockElement === this.canvas) {
      document.exitPointerLock();
    }
    this.pointerLocked = false;
    this.pressedKeys.clear();
    this.asyncChunkGeneration?.dispose();
    this.asyncChunkMeshing?.dispose();
    this.renderer?.dispose();
    this.canvas.removeEventListener("click", this.handleCanvasClick);
    this.canvas.removeEventListener("mousedown", this.handleCanvasMouseDown);
    this.canvas.removeEventListener("contextmenu", this.handleCanvasContextMenu);
    this.canvas.removeEventListener("wheel", this.handleCanvasWheel);
    document.removeEventListener("pointerlockchange", this.handlePointerLockChange);
    document.removeEventListener("mousemove", this.handleMouseMove);
    window.removeEventListener("keydown", this.handleKeyDown);
    window.removeEventListener("keyup", this.handleKeyUp);
    window.removeEventListener("blur", this.handleBlur);
    document.removeEventListener("visibilitychange", this.handleVisibilityChange);
  }

  async requestPointerLock(): Promise<void> {
    try {
      await this.canvas.requestPointerLock();
    } catch {
      this.status = "Pointer lock request was blocked";
      this.pushHud(true);
    }
  }

  start(): void {
    cancelAnimationFrame(this.rafId);
    const tick = (now: number) => {
      const gameplayStartedAt = performance.now();
      const deltaSeconds = this.lastFrameTime === 0
        ? 1 / 60
        : Math.min((now - this.lastFrameTime) / 1000, MAX_DELTA_SECONDS);
      this.lastFrameTime = now;
      this.interactiveFrameNumber += 1;
      const hasMovementIntent = this.hasMovementIntent();
      const moved = this.updateMovement(deltaSeconds);
      const dirtyResidentChunks = this.world.countDirtyResidentChunks();
      if (
        shouldPumpWorldWork(
          hasMovementIntent || moved,
          this.lastStreamSummary.pendingChunks,
          dirtyResidentChunks,
          this.lastFarFieldSummary.pendingBands,
        )
      ) {
        this.syncWorldAroundPlayer(
          false,
          shouldAllowFarFieldCatchupWhileMoving(
            hasMovementIntent,
            this.lastStreamSummary.pendingChunks,
            dirtyResidentChunks,
            this.lastFarFieldSummary.pendingBands,
            this.interactiveFrameNumber,
            MOVEMENT_FAR_FIELD_CATCHUP_CADENCE_FRAMES,
          ),
        );
      }
      this.renderInteractiveFrame();
      this.lastGameplayFrameMs = performance.now() - gameplayStartedAt;
      this.recordBootstrapBenchmarkSample(this.lastGameplayFrameMs);
      this.rafId = requestAnimationFrame(tick);
    };
    this.rafId = requestAnimationFrame(tick);
  }

  stop(): void {
    cancelAnimationFrame(this.rafId);
  }

  getDebugSnapshot(): GameHudSnapshot {
    const stats = this.world.getStats();
    const discovery = this.refreshDiscoveryJournal();
    const bootstrap = this.getBootstrapReadiness();
    return {
      status: this.status,
      pointerLocked: this.pointerLocked,
      position: [...this.camera.position],
      feetPosition: [...this.player.feetPosition] as Vec3,
      playerChunk: [
        Math.floor(this.player.feetPosition[0] / this.world.chunkSize),
        Math.floor(this.player.feetPosition[1] / this.world.chunkSize),
        Math.floor(this.player.feetPosition[2] / this.world.chunkSize),
      ],
      streamAnchorChunk: this.streamAnchor
        ? [this.streamAnchor.chunkX, this.streamAnchor.chunkZ]
        : [
            Math.floor(this.player.feetPosition[0] / this.world.chunkSize),
            Math.floor(this.player.feetPosition[2] / this.world.chunkSize),
          ],
      grounded: this.player.grounded,
      bodyInWater: this.player.bodyInWater,
      eyeInWater: this.player.eyeInWater,
      yawDegrees: toDegrees(this.camera.yaw),
      pitchDegrees: toDegrees(this.camera.pitch),
      solidVoxelCount: stats.solidVoxelCount,
      chunkCount: stats.chunkCount,
      paletteCount: stats.paletteCount,
      streamMs: this.lastStreamSummary.elapsedMs,
      streamGeneratedChunks: this.lastStreamSummary.generatedChunks,
      streamEvictedChunks: this.lastStreamSummary.evictedChunks,
      streamPendingChunks: this.lastStreamSummary.pendingChunks,
      streamEmptyChunksSkipped: this.lastStreamSummary.emptyChunksSkipped,
      streamCachedEmptyChunkHits: this.lastStreamSummary.cachedEmptyChunkHits,
      streamCompletedChunkCacheHits: this.lastStreamSummary.phaseMs.completedChunkCacheHits,
      streamCompletedGeneratedChunks: this.lastStreamSummary.phaseMs.completedGeneratedChunks,
      generationWorkerCount: this.asyncChunkGeneration?.getWorkerCount?.() ?? 0,
      streamCompletedSummaryCacheHits: this.lastStreamSummary.phaseMs.completedSummaryCacheHits,
      streamCompletedGeneratedSummaries: this.lastStreamSummary.phaseMs.completedGeneratedSummaries,
      streamCompletedRegionSummaryCacheHits: this.lastStreamSummary.phaseMs.completedRegionSummaryCacheHits,
      streamMissingRegionSummaries: this.lastStreamSummary.phaseMs.missingRegionSummaries,
      streamDirtyResidentChunks: this.world.countDirtyResidentChunks(),
      residencyRadiusChunks: this.lastStreamSummary.radiusChunks,
      surfaceY: this.lastStreamSummary.surfaceY,
      biomeId: discovery.currentBiomeId,
      undergroundBiomeId: discovery.currentUndergroundBiomeId,
      regionalVariantId: discovery.currentRegionalVariantId,
      landmarkId: discovery.currentLandmarkId,
      discoveredBiomeCount: discovery.discoveredBiomeIds.length,
      discoveredUndergroundBiomeCount: discovery.discoveredUndergroundBiomeIds.length,
      discoveredRegionalVariantCount: discovery.discoveredRegionalVariantIds.length,
      discoveredLandmarkCount: discovery.discoveredLandmarkIds.length,
      recentDiscoveries: discovery.recentDiscoveries.map((event) => ({ ...event })),
      lastDiscoveryLabel: discovery.lastDiscovery?.label ?? "None",
      selectedInventorySlot: this.inventory.selectedSlot,
      selectedInventoryMaterial: formatInventoryMaterial(getSelectedInventoryStack(this.inventory)?.material ?? null),
      selectedInventoryCount: getSelectedInventoryStack(this.inventory)?.count ?? 0,
      usedInventoryStacks: countUsedInventoryStacks(this.inventory),
      bootstrapPlayableReady: bootstrap.playableReady,
      bootstrapVisualReady: bootstrap.visualReady,
      bootstrapElapsedMs: performance.now() - this.bootstrapBenchmarkStartedAt,
      farFieldMs: this.lastFarFieldSummary.elapsedMs,
      farFieldBuiltBands: this.lastFarFieldSummary.builtBands,
      farFieldPendingBands: this.lastFarFieldSummary.pendingBands,
      farFieldTriangles: this.lastFarFieldSummary.triangleCount,
      farFieldMaxRadiusMeters: this.lastFarFieldSummary.maxRadiusMeters,
      meshMs: this.meshMs,
      meshNewChunks: this.lastMeshBuildSummary.newMeshCount,
      meshRemeshChunks: this.lastMeshBuildSummary.remeshCount,
      drawCalls: this.drawCalls,
      triangles: this.triangles,
      lastFrameCpuMs: this.lastFrameCpuMs,
      lastFrameSyncMs: this.lastRenderStats.syncResourcesMs,
      lastFrameUploadMs: this.lastRenderStats.uploadMs,
      lastFrameUploadChunks: this.lastRenderStats.uploadChunks,
      lastFrameUploadBytes: this.lastRenderStats.uploadBytes,
      lastFrameEncodeMs: this.lastRenderStats.encodeMs,
      avgFrameCpuMs: this.avgFrameCpuMs,
      maxGeneratedChunksPerUpdate: this.streamingBudgets.maxGeneratedChunksPerUpdate,
      maxMeshRebuildsPerFrame: this.streamingBudgets.maxMeshRebuildsPerFrame,
      maxFarFieldBandRebuildsPerFrame: this.streamingBudgets.maxFarFieldBandRebuildsPerFrame,
    };
  }

  getBootstrapBenchmark(): BootstrapExperienceBenchmark {
    const samples = this.bootstrapBenchmarkSamples.map((sample) => ({ ...sample }));
    return {
      completed: this.bootstrapBenchmarkComplete,
      startedAtMs: this.bootstrapBenchmarkStartedAt,
      samples,
      summary: summarizeBootstrapBenchmark(samples),
    };
  }

  getStreamingBudgets(): StreamingBudgets {
    return { ...this.streamingBudgets };
  }

  getInventorySnapshot(): {
    selectedSlot: number;
    usedStacks: number;
    slots: Array<{ material: number; count: number } | null>;
  } {
    return {
      selectedSlot: this.inventory.selectedSlot,
      usedStacks: countUsedInventoryStacks(this.inventory),
      slots: this.inventory.slots.map((stack) => stack ? { ...stack } : null),
    };
  }

  getEditLogSnapshot(): WorldEditRecord[] {
    return this.world.getEditLogSnapshot();
  }

  breakTargetVoxel(): boolean {
    const ray = createForwardRay(this.camera);
    const result = breakVoxelAlongRay(
      this.world,
      this.inventory,
      ray.origin,
      ray.direction,
      INTERACTION_REACH_WORLD_UNITS,
    );
    if (!result.changed || !result.material) {
      return false;
    }
    this.farFieldReadyMaskRevision += 1;
    this.flushMeshBuildBudget(1);
    this.status = `Collected ${formatInventoryMaterial(result.material)}`;
    this.pushHud(true);
    return true;
  }

  placeSelectedVoxel(): boolean {
    const ray = createForwardRay(this.camera);
    const result = placeSelectedVoxelAlongRay(
      this.world,
      this.inventory,
      ray.origin,
      ray.direction,
      INTERACTION_REACH_WORLD_UNITS,
    );
    if (!result.changed || !result.material) {
      return false;
    }
    this.farFieldReadyMaskRevision += 1;
    this.flushMeshBuildBudget(1);
    this.status = `Placed ${formatInventoryMaterial(result.material)}`;
    this.pushHud(true);
    return true;
  }

  setStreamingBudgets(
    maxGeneratedChunksPerUpdate: number,
    maxMeshRebuildsPerFrame: number,
    maxFarFieldBandRebuildsPerFrame = this.streamingBudgets.maxFarFieldBandRebuildsPerFrame,
  ): StreamingBudgets {
    this.streamingBudgets = {
      maxGeneratedChunksPerUpdate: clampPositiveInt(
        maxGeneratedChunksPerUpdate,
        DEFAULT_MAX_GENERATED_CHUNKS_PER_UPDATE,
      ),
      maxMeshRebuildsPerFrame: clampPositiveInt(
        maxMeshRebuildsPerFrame,
        DEFAULT_MAX_MESH_REBUILDS_PER_FRAME,
      ),
      maxFarFieldBandRebuildsPerFrame: clampPositiveInt(
        maxFarFieldBandRebuildsPerFrame,
        DEFAULT_MAX_FAR_FIELD_BAND_REBUILDS_PER_FRAME,
      ),
    };
    this.pushHud(true);
    return this.getStreamingBudgets();
  }

  teleport(position: Vec3): void {
    teleportPlayerToEyePosition(this.player, position);
    this.syncCameraToPlayer();
    this.syncWorldAroundPlayer();
    this.pushHud(true);
  }

  setResidencyRadiusChunks(radius: number): void {
    this.world.setHorizontalRadiusChunks(radius);
    this.syncWorldAroundPlayer(true);
    this.pushHud(true);
  }

  forceResidencyUpdate(): ResidencyUpdateSummary {
    this.world.setHorizontalRadiusChunks(this.world.horizontalRadiusChunks);
    this.syncWorldAroundPlayer(true);
    this.pushHud(true);
    return cloneResidencySummary(this.lastStreamSummary);
  }

  snapshotResidentWorld(): ResidentWorldProbeSnapshot {
    return summarizeResidentWorld(this.world);
  }

  getDiscoveryJournalSnapshot(): ExplorationJournalSnapshot {
    return this.refreshDiscoveryJournal(true);
  }

  resetDiscoveryJournal(): ExplorationJournalSnapshot {
    this.explorationJournal.reset();
    this.lastDiscoverySampleFeetPosition = null;
    this.lastDiscoverySampleAt = 0;
    const snapshot = this.refreshDiscoveryJournal(true);
    this.pushHud(true);
    return snapshot;
  }

  probeRenderReadyCoverage(sampleRadiusMeters = 12, sampleStepMeters = 0.8): RenderReadyCoverageProbe {
    const normalizedRadius = Math.max(1, sampleRadiusMeters);
    const normalizedStep = Math.max(0.1, sampleStepMeters);
    const sampleRadiusWorldUnits = metersToWorldUnits(normalizedRadius);
    const sampleStepWorldUnits = metersToWorldUnits(normalizedStep);
    const centerX = this.player.feetPosition[0];
    const centerZ = this.player.feetPosition[2];
    const renderReadyMask = this.getRenderReadyFarFieldMask();
    let sampleCount = 0;
    let residentSampleCount = 0;
    let renderReadySampleCount = 0;
    let residentNotReadyCount = 0;

    for (let offsetZ = -sampleRadiusWorldUnits; offsetZ <= sampleRadiusWorldUnits; offsetZ += sampleStepWorldUnits) {
      for (let offsetX = -sampleRadiusWorldUnits; offsetX <= sampleRadiusWorldUnits; offsetX += sampleStepWorldUnits) {
        const worldX = centerX + offsetX;
        const worldZ = centerZ + offsetZ;
        const chunkX = Math.floor(worldX / this.world.chunkSize);
        const chunkZ = Math.floor(worldZ / this.world.chunkSize);
        const resident = this.world.hasResidentColumn(chunkX, chunkZ);
        const renderReady = renderReadyMask.excludesCell(
          chunkX * this.world.chunkSize,
          (chunkX + 1) * this.world.chunkSize,
          chunkZ * this.world.chunkSize,
          (chunkZ + 1) * this.world.chunkSize,
        );
        sampleCount += 1;
        if (resident) {
          residentSampleCount += 1;
        }
        if (renderReady) {
          renderReadySampleCount += 1;
        }
        if (resident && !renderReady) {
          residentNotReadyCount += 1;
        }
      }
    }

    return {
      center: [...this.player.feetPosition],
      sampleRadiusMeters: normalizedRadius,
      sampleStepMeters: normalizedStep,
      sampleCount,
      residentSampleCount,
      renderReadySampleCount,
      residentNotReadyCount,
      missingResidentCount: sampleCount - residentSampleCount,
    };
  }

  probeNearFarSeamGaps(): NearFarSeamProbe {
    return this.farField.probeMaskedSeamGaps(this.getRenderReadyFarFieldMask());
  }

  probeFarFieldSurfaceGaps(): FarFieldSurfaceGapProbe {
    return this.farField.probeSurfaceGaps(this.getRenderReadyFarFieldMask());
  }

  probeVisibleGroundCoverage(
    sampleForwardMeters = 16,
    sampleLateralMeters = 6,
    sampleStepMeters = 0.8,
  ): VisibleGroundCoverageProbe {
    const normalizedForward = Math.max(1, sampleForwardMeters);
    const normalizedLateral = Math.max(1, sampleLateralMeters);
    const normalizedStep = Math.max(0.1, sampleStepMeters);
    const renderReadyMask = this.getRenderReadyFarFieldMask();
    const forwardAxis = [Math.cos(this.camera.yaw), Math.sin(this.camera.yaw)] as const;
    const rightAxis = [-forwardAxis[1], forwardAxis[0]] as const;
    const forwardWorldUnits = metersToWorldUnits(normalizedForward);
    const lateralWorldUnits = metersToWorldUnits(normalizedLateral);
    const stepWorldUnits = metersToWorldUnits(normalizedStep);
    const uncoveredSamples: VisibleGroundCoverageIssueSample[] = [];
    let sampleCount = 0;
    let renderReadyCount = 0;
    let farOnlyCount = 0;
    let uncoveredCount = 0;
    let residentNotReadyCount = 0;

    for (let forward = stepWorldUnits; forward <= forwardWorldUnits; forward += stepWorldUnits) {
      for (let lateral = -lateralWorldUnits; lateral <= lateralWorldUnits; lateral += stepWorldUnits) {
        const worldX = this.player.feetPosition[0] + forwardAxis[0] * forward + rightAxis[0] * lateral;
        const worldZ = this.player.feetPosition[2] + forwardAxis[1] * forward + rightAxis[1] * lateral;
        const chunkX = Math.floor(worldX / this.world.chunkSize);
        const chunkZ = Math.floor(worldZ / this.world.chunkSize);
        const resident = this.world.hasResidentColumn(chunkX, chunkZ);
        const renderReady = renderReadyMask.excludesCell(worldX, worldX + 1, worldZ, worldZ + 1);
        const farCovered = this.farField.getCoverageAt(worldX, worldZ, renderReadyMask).length > 0;
        sampleCount += 1;
        if (renderReady) {
          renderReadyCount += 1;
        } else if (farCovered) {
          farOnlyCount += 1;
        } else {
          uncoveredCount += 1;
          pushVisibleGroundIssueSample(uncoveredSamples, {
            worldX,
            worldZ,
            forwardMeters: worldUnitsToMeters(forward),
            lateralMeters: worldUnitsToMeters(lateral),
            resident,
            renderReady,
            farCovered,
          });
        }
        if (resident && !renderReady) {
          residentNotReadyCount += 1;
        }
      }
    }

    return {
      center: [...this.player.feetPosition],
      yawRadians: this.camera.yaw,
      sampleForwardMeters: normalizedForward,
      sampleLateralMeters: normalizedLateral,
      sampleStepMeters: normalizedStep,
      sampleCount,
      renderReadyCount,
      farOnlyCount,
      uncoveredCount,
      residentNotReadyCount,
      uncoveredSamples,
    };
  }

  probeLodCoverage(sampleRadiusMeters = 48, sampleStepMeters = 0.8): LodCoverageProbe {
    const normalizedRadius = Math.max(1, sampleRadiusMeters);
    const normalizedStep = Math.max(0.1, sampleStepMeters);
    const sampleRadiusWorldUnits = metersToWorldUnits(normalizedRadius);
    const sampleStepWorldUnits = metersToWorldUnits(normalizedStep);
    const centerX = this.player.feetPosition[0];
    const centerZ = this.player.feetPosition[2];
    const maxRadiusWorldUnits = this.lastFarFieldSummary.maxRadiusWorldUnits;
    const renderReadyMask = this.getRenderReadyFarFieldMask();
    const residentOverlapSamples: LodCoverageIssueSample[] = [];
    const uncoveredGapSamples: LodCoverageIssueSample[] = [];
    const handoffHoleSamples: LodCoverageIssueSample[] = [];
    const bandOverlapSamples: LodCoverageIssueSample[] = [];
    const wrongBandSamples: LodCoverageIssueSample[] = [];
    let sampleCount = 0;
    let residentSampleCount = 0;
    let renderReadySampleCount = 0;
    let coveredSampleCount = 0;
    let residentOverlapCount = 0;
    let uncoveredGapCount = 0;
    let handoffHoleCount = 0;
    let bandOverlapCount = 0;
    let wrongBandCount = 0;

    for (let offsetZ = -sampleRadiusWorldUnits; offsetZ <= sampleRadiusWorldUnits; offsetZ += sampleStepWorldUnits) {
      for (let offsetX = -sampleRadiusWorldUnits; offsetX <= sampleRadiusWorldUnits; offsetX += sampleStepWorldUnits) {
        const distanceWorldUnits = Math.max(Math.abs(offsetX), Math.abs(offsetZ));
        if (distanceWorldUnits > maxRadiusWorldUnits) {
          continue;
        }
        const worldX = centerX + offsetX;
        const worldZ = centerZ + offsetZ;
        const chunkX = Math.floor(worldX / this.world.chunkSize);
        const chunkZ = Math.floor(worldZ / this.world.chunkSize);
        const resident = this.world.hasResidentColumn(chunkX, chunkZ);
        const renderReady = renderReadyMask.excludesCell(worldX, worldX + 1, worldZ, worldZ + 1);
        const coverage = this.farField.getCoverageAt(worldX, worldZ, renderReadyMask);
        const distanceMeters = worldUnitsToMeters(distanceWorldUnits);
        const issueSample = coverageToIssueSample(worldX, worldZ, distanceMeters, coverage);
        sampleCount += 1;
        if (resident) {
          residentSampleCount += 1;
        }
        if (renderReady) {
          renderReadySampleCount += 1;
        }
        if (coverage.length > 0) {
          coveredSampleCount += 1;
        }
        if (renderReady && coverage.length > 0) {
          residentOverlapCount += 1;
          pushIssueSample(residentOverlapSamples, issueSample);
        }
        if (!renderReady && coverage.length === 0) {
          uncoveredGapCount += 1;
          pushIssueSample(uncoveredGapSamples, issueSample);
        }
        if (resident && !renderReady && coverage.length === 0) {
          handoffHoleCount += 1;
          pushIssueSample(handoffHoleSamples, issueSample);
        }
        if (coverage.length > 1) {
          bandOverlapCount += 1;
          pushIssueSample(bandOverlapSamples, issueSample);
        }
        if (
          coverage.length > 1
          || coverage.some((band) => distanceWorldUnits < band.innerRadiusWorldUnits || distanceWorldUnits >= band.outerRadiusWorldUnits)
        ) {
          wrongBandCount += 1;
          pushIssueSample(wrongBandSamples, issueSample);
        }
      }
    }

    return {
      center: [...this.player.feetPosition],
      sampleRadiusMeters: normalizedRadius,
      sampleStepMeters: normalizedStep,
      sampleCount,
      residentSampleCount,
      renderReadySampleCount,
      coveredSampleCount,
      residentOverlapCount,
      uncoveredGapCount,
      handoffHoleCount,
      bandOverlapCount,
      wrongBandCount,
      residentOverlapSamples,
      uncoveredGapSamples,
      handoffHoleSamples,
      bandOverlapSamples,
      wrongBandSamples,
    };
  }

  async teleportAndSettle(
    position: Vec3,
    options: {
      radiusChunks?: number;
      maxFrames?: number;
    } = {},
  ): Promise<ResidencyTransitionProbe> {
    const before = this.snapshotResidentWorld();
    if (options.radiusChunks !== undefined) {
      this.world.setHorizontalRadiusChunks(options.radiusChunks);
    }
    teleportPlayerToEyePosition(this.player, position);
    this.syncCameraToPlayer();
    const maxFrames = Math.max(1, Math.floor(options.maxFrames ?? 240));
    let residency = this.syncWorldAroundPlayer(true);
    let render = await this.renderProbeFrame();
    let settleFrames = 1;
    while (
      settleFrames < maxFrames
      && (
        !residency.complete
        || residency.pendingChunks > 0
        || this.lastMeshBuildSummary.meshCount > 0
        || this.lastFarFieldSummary.pendingBands > 0
      )
    ) {
      residency = this.syncWorldAroundPlayer(true);
      render = await this.renderProbeFrame();
      settleFrames += 1;
    }
    const after = this.snapshotResidentWorld();
    const { entered, evicted } = diffChunkCoords(
      before.chunks.map((chunk) => chunk.coord),
      after.chunks.map((chunk) => chunk.coord),
    );
    this.pushHud(true);
    return {
      before,
      after,
      enteredChunkCoords: entered.map(toChunkTuple),
      evictedChunkCoords: evicted.map(toChunkTuple),
      generatedChunkCoords: residency.generatedChunkCoords.map(toChunkTuple),
      residency: {
        ...residency,
      generatedChunkCoords: residency.generatedChunkCoords.map((coord) => ({ ...coord })),
      evictedChunkCoords: residency.evictedChunkCoords.map((coord) => ({ ...coord })),
      },
      mesh: { ...this.lastMeshBuildSummary },
      render,
      settleFrames,
      settled: residency.complete
        && residency.pendingChunks === 0
        && this.lastMeshBuildSummary.meshCount === 0
        && this.lastFarFieldSummary.pendingBands === 0,
    };
  }

  async benchmarkChunkCrossing(iterations: number, chunkDelta = 1): Promise<ChunkBoundaryBenchmark> {
    const normalizedIterations = Math.max(1, Math.floor(iterations));
    const normalizedChunkDelta = Math.max(1, Math.floor(chunkDelta));
    const spawn = this.world.getSpawnPosition();
    const baseChunkX = Math.floor(spawn[0] / this.world.chunkSize);
    const baseChunkZ = Math.floor(spawn[2] / this.world.chunkSize);
    const leftTarget = buildEyePositionForChunkCenter(
      baseChunkX,
      baseChunkZ,
      spawn[1],
      this.world.chunkSize,
      this.player.eyeHeight,
    );
    const rightTarget = buildEyePositionForChunkCenter(
      baseChunkX + normalizedChunkDelta,
      baseChunkZ,
      spawn[1],
      this.world.chunkSize,
      this.player.eyeHeight,
    );
    const savedPlayer = {
      feetPosition: [...this.player.feetPosition] as Vec3,
      velocity: [...this.player.velocity] as Vec3,
      grounded: this.player.grounded,
    };
    const savedCamera = {
      position: [...this.camera.position] as Vec3,
      yaw: this.camera.yaw,
      pitch: this.camera.pitch,
      fovY: this.camera.fovY,
      near: this.camera.near,
      far: this.camera.far,
    };
    const savedStatus = this.status;
    const savedStreamAnchor = this.streamAnchor
      ? { chunkX: this.streamAnchor.chunkX, chunkZ: this.streamAnchor.chunkZ }
      : null;

    this.stop();
    try {
      await this.teleportAndSettle(leftTarget);
      const samples: ChunkBoundaryBenchmarkSample[] = [];
      for (let iteration = 0; iteration < normalizedIterations; iteration += 1) {
        for (const target of [rightTarget, leftTarget]) {
          const transition = await this.teleportAndSettle(target);
          samples.push({
            step: samples.length + 1,
            targetEyePosition: [...target],
            targetChunk: [
              Math.floor(target[0] / this.world.chunkSize),
              Math.floor((target[1] - this.player.eyeHeight) / this.world.chunkSize),
              Math.floor(target[2] / this.world.chunkSize),
            ],
            changed: transition.residency.changed,
            generatedChunks: transition.residency.generatedChunks,
            evictedChunks: transition.residency.evictedChunks,
            streamMs: transition.residency.elapsedMs,
            meshMs: transition.mesh.elapsedMs,
            meshNewChunks: transition.mesh.newMeshCount,
            meshRemeshChunks: transition.mesh.remeshCount,
            frameCpuMs: transition.render.frameCpuMs,
            syncMs: transition.render.syncResourcesMs,
            uploadMs: transition.render.uploadMs,
            uploadChunks: transition.render.uploadChunks,
            uploadBytes: transition.render.uploadBytes,
            encodeMs: transition.render.encodeMs,
          });
        }
      }
      return {
        iterations: normalizedIterations,
        chunkDelta: normalizedChunkDelta,
        radiusChunks: this.world.horizontalRadiusChunks,
        samples,
        summary: summarizeChunkBoundaryBenchmark(samples),
      };
    } finally {
      this.player.feetPosition = savedPlayer.feetPosition;
      this.player.velocity = savedPlayer.velocity;
      this.player.grounded = savedPlayer.grounded;
      this.camera = {
        position: savedCamera.position,
        yaw: savedCamera.yaw,
        pitch: savedCamera.pitch,
        fovY: savedCamera.fovY,
        near: savedCamera.near,
        far: savedCamera.far,
      };
      this.status = savedStatus;
      if (savedStreamAnchor) {
        this.syncWorldAroundAnchor(savedStreamAnchor);
      } else {
        this.streamAnchor = null;
        this.syncWorldAroundPlayer(true);
      }
      await this.renderProbeFrame();
      this.pushHud(true);
      this.start();
    }
  }

  async benchmarkChunkCacheReuse(chunkDelta = 24, maxFramesPerLeg = 240): Promise<ChunkCacheReuseBenchmark> {
    const normalizedChunkDelta = Math.max(1, Math.floor(chunkDelta));
    const normalizedMaxFrames = Math.max(1, Math.floor(maxFramesPerLeg));
    const spawn = this.world.getSpawnPosition();
    const baseChunkX = Math.floor(spawn[0] / this.world.chunkSize);
    const baseChunkZ = Math.floor(spawn[2] / this.world.chunkSize);
    const originTarget = buildEyePositionForChunkCenter(
      baseChunkX,
      baseChunkZ,
      spawn[1],
      this.world.chunkSize,
      this.player.eyeHeight,
    );
    const farTarget = buildEyePositionForChunkCenter(
      baseChunkX + normalizedChunkDelta,
      baseChunkZ,
      spawn[1],
      this.world.chunkSize,
      this.player.eyeHeight,
    );
    const savedPlayer = {
      feetPosition: [...this.player.feetPosition] as Vec3,
      velocity: [...this.player.velocity] as Vec3,
      grounded: this.player.grounded,
    };
    const savedCamera = {
      position: [...this.camera.position] as Vec3,
      yaw: this.camera.yaw,
      pitch: this.camera.pitch,
      fovY: this.camera.fovY,
      near: this.camera.near,
      far: this.camera.far,
    };
    const savedStatus = this.status;
    const savedStreamAnchor = this.streamAnchor
      ? { chunkX: this.streamAnchor.chunkX, chunkZ: this.streamAnchor.chunkZ }
      : null;

    this.stop();
    try {
      await this.teleportAndSettle(originTarget, { radiusChunks: this.world.horizontalRadiusChunks });
      const populate = await this.measureChunkCacheReuseLeg(farTarget, normalizedMaxFrames);
      const revisit = await this.measureChunkCacheReuseLeg(originTarget, normalizedMaxFrames);
      return {
        chunkDelta: normalizedChunkDelta,
        radiusChunks: this.world.horizontalRadiusChunks,
        populate,
        revisit,
      };
    } finally {
      this.player.feetPosition = savedPlayer.feetPosition;
      this.player.velocity = savedPlayer.velocity;
      this.player.grounded = savedPlayer.grounded;
      this.camera = {
        position: savedCamera.position,
        yaw: savedCamera.yaw,
        pitch: savedCamera.pitch,
        fovY: savedCamera.fovY,
        near: savedCamera.near,
        far: savedCamera.far,
      };
      this.status = savedStatus;
      if (savedStreamAnchor) {
        this.syncWorldAroundAnchor(savedStreamAnchor, true);
      } else {
        this.streamAnchor = null;
        this.syncWorldAroundPlayer(true);
      }
      await this.renderProbeFrame();
      this.pushHud(true);
      this.start();
    }
  }

  async benchmarkIncrementalCrossing(
    iterations: number,
    chunkDelta = 2,
    stepsPerLeg = 12,
    settleFrames = 16,
  ): Promise<IncrementalCrossingBenchmark> {
    const normalizedIterations = Math.max(1, Math.floor(iterations));
    const normalizedChunkDelta = Math.max(1, Math.floor(chunkDelta));
    const normalizedSteps = Math.max(2, Math.floor(stepsPerLeg));
    const normalizedSettleFrames = Math.max(1, Math.floor(settleFrames));
    const spawn = this.world.getSpawnPosition();
    const baseChunkX = Math.floor(spawn[0] / this.world.chunkSize);
    const baseChunkZ = Math.floor(spawn[2] / this.world.chunkSize);
    const leftFeet = buildFeetPositionForChunkCenter(baseChunkX, baseChunkZ, spawn[1], this.world.chunkSize);
    const rightFeet = buildFeetPositionForChunkCenter(
      baseChunkX + normalizedChunkDelta,
      baseChunkZ,
      spawn[1],
      this.world.chunkSize,
    );
    const savedPlayer = {
      feetPosition: [...this.player.feetPosition] as Vec3,
      velocity: [...this.player.velocity] as Vec3,
      grounded: this.player.grounded,
    };
    const savedCamera = {
      position: [...this.camera.position] as Vec3,
      yaw: this.camera.yaw,
      pitch: this.camera.pitch,
      fovY: this.camera.fovY,
      near: this.camera.near,
      far: this.camera.far,
    };
    const savedStatus = this.status;
    const savedStreamAnchor = this.streamAnchor
      ? { chunkX: this.streamAnchor.chunkX, chunkZ: this.streamAnchor.chunkZ }
      : null;

    this.stop();
    try {
      teleportPlayerToFeetPosition(this.player, leftFeet);
      this.player.grounded = true;
      this.syncCameraToPlayer();
      this.syncWorldAroundPlayer(true);
      await this.renderProbeFrame();

      const samples: IncrementalCrossingSample[] = [];
      let frame = 0;
      const legs: Array<readonly [Vec3, Vec3]> = [[leftFeet, rightFeet], [rightFeet, leftFeet]];
      for (let iteration = 0; iteration < normalizedIterations; iteration += 1) {
        for (const [startFeet, endFeet] of legs) {
          const legIndex = samples.length;
          for (let step = 1; step <= normalizedSteps; step += 1) {
            const t = step / normalizedSteps;
            teleportPlayerToFeetPosition(this.player, lerpVec3(startFeet, endFeet, t));
            this.player.grounded = true;
            this.syncCameraToPlayer();
            const residency = this.syncWorldAroundPlayer(false, false);
            const detailCoverage = this.probeRenderReadyCoverage();
            const render = await this.renderProbeFrame();
            frame += 1;
            samples.push(
              buildIncrementalSample(
                frame,
                "move",
                legIndex,
                residency,
                this.lastMeshBuildSummary,
                this.lastFarFieldSummary,
                detailCoverage,
                render,
              ),
            );
          }
          for (let settleFrame = 0; settleFrame < normalizedSettleFrames; settleFrame += 1) {
            const residency = this.syncWorldAroundPlayer(false, true);
            const detailCoverage = this.probeRenderReadyCoverage();
            const render = await this.renderProbeFrame();
            frame += 1;
            samples.push(
              buildIncrementalSample(
                frame,
                "settle",
                legIndex,
                residency,
                this.lastMeshBuildSummary,
                this.lastFarFieldSummary,
                detailCoverage,
                render,
              ),
            );
            if (
              residency.complete
              && residency.pendingChunks === 0
              && this.lastMeshBuildSummary.meshCount === 0
              && this.lastFarFieldSummary.pendingBands === 0
            ) {
              break;
            }
          }
        }
      }
      return {
        iterations: normalizedIterations,
        chunkDelta: normalizedChunkDelta,
        stepsPerLeg: normalizedSteps,
        settleFrames: normalizedSettleFrames,
        radiusChunks: this.world.horizontalRadiusChunks,
        samples,
        summary: summarizeIncrementalCrossing(samples),
      };
    } finally {
      this.player.feetPosition = savedPlayer.feetPosition;
      this.player.velocity = savedPlayer.velocity;
      this.player.grounded = savedPlayer.grounded;
      this.camera = {
        position: savedCamera.position,
        yaw: savedCamera.yaw,
        pitch: savedCamera.pitch,
        fovY: savedCamera.fovY,
        near: savedCamera.near,
        far: savedCamera.far,
      };
      this.status = savedStatus;
      if (savedStreamAnchor) {
        this.syncWorldAroundAnchor(savedStreamAnchor, true);
      } else {
        this.streamAnchor = null;
        this.syncWorldAroundPlayer(true);
      }
      await this.renderProbeFrame();
      this.pushHud(true);
      this.start();
    }
  }

  async benchmarkRouteExperience(
    options: RouteExperienceBenchmarkOptions = {},
  ): Promise<RouteExperienceBenchmark> {
    const spawnFeet = this.world.getSpawnPosition();
    const routePlan = buildDefaultRouteBenchmarkPlan(
      spawnFeet,
      (worldX, worldZ) => this.generator.sampleColumn(worldX, worldZ).surfaceY + 1,
      normalizeRouteBenchmarkPlanOptions(options),
    );
    return this.runRouteExperienceBenchmark(routePlan, options);
  }

  async benchmarkForwardWalkExperience(
    options: RouteExperienceBenchmarkOptions & {
      yawRadians?: number;
    } = {},
  ): Promise<RouteExperienceBenchmark> {
    const spawnFeet = this.world.getSpawnPosition();
    const routePlan = buildForwardRouteBenchmarkPlan(
      spawnFeet,
      (worldX, worldZ) => this.generator.sampleColumn(worldX, worldZ).surfaceY + 1,
      {
        ...normalizeRouteBenchmarkPlanOptions(options),
        yawRadians: options.yawRadians,
      },
    );
    return this.runRouteExperienceBenchmark(routePlan, options);
  }

  private async runRouteExperienceBenchmark(
    routePlan: ReturnType<typeof buildDefaultRouteBenchmarkPlan>,
    options: RouteExperienceBenchmarkOptions,
  ): Promise<RouteExperienceBenchmark> {
    const settleSeconds = Math.max(1, options.settleSeconds ?? 4);
    const sampleHz = routePlan.sampleHz;
    const seamProbeStrideFrames = clampPositiveInt(
      options.seamProbeStrideFrames ?? Math.max(1, Math.round(sampleHz / 4)),
      Math.max(1, Math.round(sampleHz / 4)),
    );
    const captureStrideFrames = clampPositiveInt(
      options.captureStrideFrames ?? Math.max(1, Math.round(sampleHz / 2)),
      Math.max(1, Math.round(sampleHz / 2)),
    );
    const captureWidth = clampPositiveInt(options.captureWidth ?? 128, 128);
    const captureHeight = clampPositiveInt(options.captureHeight ?? 72, 72);
    const referenceDiffStrideFrames = Math.max(0, Math.floor(options.referenceDiffStrideFrames ?? 0));
    const referenceDiffLimit = Math.max(0, Math.floor(options.referenceDiffLimit ?? 24));
    const savedPlayer = {
      feetPosition: [...this.player.feetPosition] as Vec3,
      velocity: [...this.player.velocity] as Vec3,
      grounded: this.player.grounded,
    };
    const savedCamera = {
      position: [...this.camera.position] as Vec3,
      yaw: this.camera.yaw,
      pitch: this.camera.pitch,
      fovY: this.camera.fovY,
      near: this.camera.near,
      far: this.camera.far,
    };
    const savedStatus = this.status;
    const savedStreamAnchor = this.streamAnchor
      ? { chunkX: this.streamAnchor.chunkX, chunkZ: this.streamAnchor.chunkZ }
      : null;
    const spawnFeet = this.world.getSpawnPosition();

    this.stop();
    try {
      const initialTarget = routePlan.frames[0] ?? {
        frame: 1,
        phase: "move" as const,
        simTimeSeconds: 0,
        distanceMeters: 0,
        feetPosition: spawnFeet,
        yaw: this.camera.yaw,
        pitch: this.camera.pitch,
        segmentIndex: 0,
      };
      teleportPlayerToFeetPosition(this.player, initialTarget.feetPosition);
      this.player.grounded = true;
      this.camera.yaw = initialTarget.yaw;
      this.camera.pitch = initialTarget.pitch;
      this.syncCameraToPlayer();
      this.syncWorldAroundPlayer(true);
      this.renderCurrentFrame();
      await this.renderer?.waitForGpuIdle();

      const samples: RouteExperienceFrameSample[] = [];
      const capturedFrames: CapturedBenchmarkFrame[] = [];
      for (const target of routePlan.frames) {
        const frameResult = await this.runRouteExperienceFrame(
          target,
          seamProbeStrideFrames,
          captureStrideFrames,
          captureWidth,
          captureHeight,
          referenceDiffStrideFrames,
          referenceDiffLimit,
          samples.length,
          capturedFrames.length,
        );
        samples.push(frameResult.sample);
        if (frameResult.capturedFrame) {
          capturedFrames.push(frameResult.capturedFrame);
        }
      }

      const finalTarget = routePlan.frames[routePlan.frames.length - 1] ?? initialTarget;
      const maxSettleFrames = Math.max(1, Math.round(settleSeconds * sampleHz));
      for (let settleFrame = 1; settleFrame <= maxSettleFrames; settleFrame += 1) {
        const frameResult = await this.runRouteExperienceFrame(
          {
            ...finalTarget,
            frame: routePlan.frames.length + settleFrame,
            phase: "settle",
            simTimeSeconds: routePlan.durationSeconds + settleFrame / sampleHz,
            distanceMeters: routePlan.totalDistanceMeters,
          },
          seamProbeStrideFrames,
          captureStrideFrames,
          captureWidth,
          captureHeight,
          referenceDiffStrideFrames,
          referenceDiffLimit,
          samples.length,
          capturedFrames.length,
        );
        const sample = frameResult.sample;
        samples.push(sample);
        if (frameResult.capturedFrame) {
          capturedFrames.push(frameResult.capturedFrame);
        }
        if (
          sample.complete
          && sample.pendingChunks === 0
          && sample.dirtyResidentChunks === 0
          && sample.farFieldPendingBands === 0
          && sample.visibleGroundUncoveredCount === 0
        ) {
          break;
        }
      }

      if (capturedFrames.length > 0) {
        await this.applySettledReferenceDiffs(samples, capturedFrames);
      }

      return {
        seed: this.generator.seed,
        radiusChunks: this.world.horizontalRadiusChunks,
        captureStrideFrames,
        seamProbeStrideFrames,
        referenceDiffStrideFrames,
        referenceDiffLimit,
        durationSeconds: routePlan.durationSeconds,
        settleSeconds,
        totalDistanceMeters: routePlan.totalDistanceMeters,
        sampleHz: routePlan.sampleHz,
        speedMetersPerSecond: routePlan.speedMetersPerSecond,
        samples,
        summary: summarizeRouteExperienceBenchmark(samples, routePlan),
      };
    } finally {
      this.player.feetPosition = savedPlayer.feetPosition;
      this.player.velocity = savedPlayer.velocity;
      this.player.grounded = savedPlayer.grounded;
      this.camera = {
        position: savedCamera.position,
        yaw: savedCamera.yaw,
        pitch: savedCamera.pitch,
        fovY: savedCamera.fovY,
        near: savedCamera.near,
        far: savedCamera.far,
      };
      this.status = savedStatus;
      if (savedStreamAnchor) {
        this.syncWorldAroundAnchor(savedStreamAnchor, true);
      } else {
        this.streamAnchor = null;
        this.syncWorldAroundPlayer(true);
      }
      this.renderCurrentFrame();
      await this.renderer?.waitForGpuIdle();
      this.pushHud(true);
      this.start();
    }
  }

  private async measureChunkCacheReuseLeg(
    targetEyePosition: Vec3,
    maxFrames: number,
  ): Promise<ChunkCacheReuseLegSummary> {
    teleportPlayerToEyePosition(this.player, targetEyePosition);
    this.player.grounded = true;
    this.syncCameraToPlayer();
    let frameCount = 0;
    let totalStreamMs = 0;
    let totalMeshMs = 0;
    let totalFarFieldMs = 0;
    let totalGeneratedChunks = 0;
    let totalPersistedChunkHits = 0;
    let totalPersistedSummaryHits = 0;
    let totalPersistedRegionSummaryHits = 0;
    let totalMissingRegionSummaries = 0;
    let totalWorkerGeneratedChunks = 0;
    let maxPendingChunks = 0;
    for (; frameCount < maxFrames; frameCount += 1) {
      const residency = this.syncWorldAroundPlayer(frameCount === 0, true);
      await this.renderProbeFrame();
      totalStreamMs += residency.elapsedMs;
      totalMeshMs += this.lastMeshBuildSummary.elapsedMs;
      totalFarFieldMs += this.lastFarFieldSummary.elapsedMs;
      totalGeneratedChunks += residency.generatedChunks;
      totalPersistedChunkHits += residency.phaseMs.completedChunkCacheHits;
      totalPersistedSummaryHits += residency.phaseMs.completedSummaryCacheHits;
      totalPersistedRegionSummaryHits += residency.phaseMs.completedRegionSummaryCacheHits;
      totalMissingRegionSummaries += residency.phaseMs.missingRegionSummaries;
      totalWorkerGeneratedChunks += residency.phaseMs.completedGeneratedChunks;
      maxPendingChunks = Math.max(maxPendingChunks, residency.pendingChunks);
      if (
        residency.complete
        && residency.pendingChunks === 0
        && this.lastMeshBuildSummary.meshCount === 0
        && this.lastFarFieldSummary.pendingBands === 0
      ) {
        break;
      }
    }
    return {
      targetChunk: [
        Math.floor(targetEyePosition[0] / this.world.chunkSize),
        Math.floor((targetEyePosition[1] - this.player.eyeHeight) / this.world.chunkSize),
        Math.floor(targetEyePosition[2] / this.world.chunkSize),
      ],
      frameCount: frameCount + 1,
      settled: frameCount < maxFrames,
      totalStreamMs,
      totalMeshMs,
      totalFarFieldMs,
      totalGeneratedChunks,
      totalPersistedChunkHits,
      totalPersistedSummaryHits,
      totalPersistedRegionSummaryHits,
      totalMissingRegionSummaries,
      totalWorkerGeneratedChunks,
      maxPendingChunks,
      residentChunks: this.world.getStats().chunkCount,
    };
  }

  private loadBootstrapWorld(): void {
    const spawn = this.world.getSpawnPosition();
    this.player = createPlayerState(spawn, { grounded: true });
    this.camera = createFirstPersonCamera(getPlayerEyePosition(this.player), 0.8, -0.32);
    this.streamAnchor = null;
    const bootstrapStartedAt = performance.now();
    this.syncWorldAroundPlayer(false);
    this.lastGameplayFrameMs = performance.now() - bootstrapStartedAt;
    this.recordBootstrapBenchmarkSample(this.lastGameplayFrameMs);
    this.status = this.bootstrapPlayableReady ? "Click once to capture cursor" : "Preparing world";
    this.pushHud(true);
  }

  private async drainBootstrapBenchmark(maxFrames = 600): Promise<void> {
    if (this.bootstrapPlayableReady) {
      return;
    }
    const normalizedMaxFrames = Math.max(1, Math.floor(maxFrames));
    for (let frame = 0; frame < normalizedMaxFrames; frame += 1) {
      const gameplayStartedAt = performance.now();
      if (
        this.lastStreamSummary.pendingChunks > 0
        || this.world.countDirtyResidentChunks() > 0
        || this.lastFarFieldSummary.pendingBands > 0
      ) {
        this.syncWorldAroundPlayer(true);
      }
      this.renderCurrentFrame();
      this.lastGameplayFrameMs = performance.now() - gameplayStartedAt;
      this.recordBootstrapBenchmarkSample(this.lastGameplayFrameMs);
      if (this.bootstrapPlayableReady) {
        return;
      }
      await new Promise<void>((resolve) => {
        setTimeout(resolve, 0);
      });
    }
    throw new Error("Bootstrap benchmark did not complete within the allotted deterministic drain frames");
  }

  private attachInteractions(): void {
    this.canvas.addEventListener("click", this.handleCanvasClick);
    this.canvas.addEventListener("mousedown", this.handleCanvasMouseDown);
    this.canvas.addEventListener("contextmenu", this.handleCanvasContextMenu);
    this.canvas.addEventListener("wheel", this.handleCanvasWheel, { passive: false });
    document.addEventListener("pointerlockchange", this.handlePointerLockChange);
    document.addEventListener("mousemove", this.handleMouseMove);
    window.addEventListener("keydown", this.handleKeyDown);
    window.addEventListener("keyup", this.handleKeyUp);
    window.addEventListener("blur", this.handleBlur);
    document.addEventListener("visibilitychange", this.handleVisibilityChange);
  }

  private readonly handleCanvasClick = () => {
    if (!this.pointerLocked) {
      void this.requestPointerLock();
    }
  };

  private readonly handleCanvasMouseDown = (event: MouseEvent) => {
    if (!this.pointerLocked) {
      if (event.button === 0) {
        void this.requestPointerLock();
      }
      return;
    }
    if (event.button === 0) {
      event.preventDefault();
      this.breakTargetVoxel();
    } else if (event.button === 2) {
      event.preventDefault();
      this.placeSelectedVoxel();
    }
  };

  private readonly handleCanvasContextMenu = (event: MouseEvent) => {
    event.preventDefault();
  };

  private readonly handleCanvasWheel = (event: WheelEvent) => {
    if (!this.pointerLocked) {
      return;
    }
    event.preventDefault();
    cycleInventorySlot(this.inventory, event.deltaY > 0 ? 1 : -1);
    this.pushHud(true);
  };

  private readonly handlePointerLockChange = () => {
    this.pointerLocked = document.pointerLockElement === this.canvas;
    this.status = this.pointerLocked
      ? "Pointer locked: WASD move, Space jump, Ctrl sprint, Alt slow"
      : "Click once to capture cursor";
    this.pushHud(true);
  };

  private readonly handleMouseMove = (event: MouseEvent) => {
    if (!this.pointerLocked) {
      return;
    }
    rotateFirstPersonCamera(this.camera, event.movementX, event.movementY);
  };

  private readonly handleKeyDown = (event: KeyboardEvent) => {
    if (this.pointerLocked && isMovementKey(event.code)) {
      event.preventDefault();
    }
    if (event.code.startsWith("Digit")) {
      const digit = Number.parseInt(event.code.slice(5), 10);
      if (digit >= 1 && digit <= 9) {
        selectInventorySlot(this.inventory, digit - 1);
        this.pushHud(true);
      }
    }
    this.pressedKeys.add(event.code);
  };

  private readonly handleKeyUp = (event: KeyboardEvent) => {
    this.pressedKeys.delete(event.code);
  };

  private readonly handleBlur = () => {
    this.pressedKeys.clear();
  };

  private readonly handleVisibilityChange = () => {
    if (document.hidden) {
      this.pressedKeys.clear();
    }
  };

  private updateMovement(deltaSeconds: number): boolean {
    const input = this.pointerLocked
      ? {
          forward: (this.isPressed("KeyW") ? 1 : 0) - (this.isPressed("KeyS") ? 1 : 0),
          strafe: (this.isPressed("KeyD") ? 1 : 0) - (this.isPressed("KeyA") ? 1 : 0),
          jump: this.isPressed("Space"),
          sprint: this.isPressed("ControlLeft", "ControlRight"),
          precision: this.isPressed("AltLeft", "AltRight"),
        }
      : {
          forward: 0,
          strafe: 0,
          jump: false,
          sprint: false,
          precision: false,
        };
    const result = stepPlayer(
      this.world,
      this.player,
      this.camera.yaw,
      input,
      deltaSeconds,
    );
    this.syncCameraToPlayer();
    return result.moved;
  }

  private hasMovementIntent(): boolean {
    if (!this.pointerLocked) {
      return false;
    }
    return this.isPressed("KeyW", "KeyS", "KeyA", "KeyD", "Space", "ControlLeft", "ControlRight");
  }

  private renderInteractiveFrame(): void {
    const rendered = this.renderCurrentFrame();
    if (!rendered) {
      return;
    }
    const { frameCpuMs } = rendered;
    this.avgFrameCpuMs = this.avgFrameCpuMs === 0
      ? frameCpuMs
      : this.avgFrameCpuMs * 0.9 + frameCpuMs * 0.1;
    this.pushHud();
  }

  private syncWorldAroundPlayer(force = false, allowFarFieldRebuild = true): ResidencyUpdateSummary {
    const playerChunkX = Math.floor(this.player.feetPosition[0] / this.world.chunkSize);
    const playerChunkZ = Math.floor(this.player.feetPosition[2] / this.world.chunkSize);
    const resolved = force
      ? {
          anchor: { chunkX: playerChunkX, chunkZ: playerChunkZ },
          changed: true,
        }
      : resolveStreamAnchor(this.streamAnchor, playerChunkX, playerChunkZ, STREAM_ANCHOR_MARGIN_CHUNKS);
    if (force) {
      return this.syncWorldAroundAnchor(resolved.anchor, true);
    }
    if (!shouldRefreshResidency(false, resolved.changed, this.lastStreamSummary.pendingChunks)) {
      this.prefetchFarFieldSummaries(false);
      this.flushMeshBuildBudget();
      this.syncPresentedFarFieldMaskRevision();
      this.lastFarFieldSummary = this.farField.updateAround(
        this.player.feetPosition,
        0,
        null,
        this.resolveFarFieldRebuildBudget(false, allowFarFieldRebuild),
      );
      this.lastStreamSummary = createIdleResidencySummary(
        this.lastStreamSummary,
        this.streamAnchor ?? resolved.anchor,
        this.world.horizontalRadiusChunks,
        this.world.getStats().chunkCount,
        this.world.countDirtyResidentChunks(),
      );
      return cloneResidencySummary(this.lastStreamSummary);
    }
    return this.syncWorldAroundAnchor(resolved.anchor, false, allowFarFieldRebuild);
  }

  private syncWorldAroundAnchor(anchor: StreamAnchor, settle = false, allowFarFieldRebuild = true): ResidencyUpdateSummary {
    this.streamAnchor = anchor;
    const residency = this.world.updateResidencyAround(
      buildStreamAnchorPosition(anchor, this.world.chunkSize, this.player.feetPosition[1]),
      {
        maxGenerateChunks: settle
          ? Number.POSITIVE_INFINITY
          : this.streamingBudgets.maxGeneratedChunksPerUpdate,
      },
    );
    this.lastStreamSummary = cloneResidencySummary(residency);
    if (residency.generatedChunks > 0 || residency.evictedChunks > 0 || residency.touchedNeighborChunks > 0) {
      this.farFieldReadyMaskRevision += 1;
    }
    this.prefetchFarFieldSummaries(settle);
    this.flushMeshBuildBudget(
      settle ? Number.POSITIVE_INFINITY : this.streamingBudgets.maxMeshRebuildsPerFrame,
    );
    this.syncPresentedFarFieldMaskRevision(settle);
    this.lastFarFieldSummary = this.farField.updateAround(
      this.player.feetPosition,
      0,
      null,
      this.resolveFarFieldRebuildBudget(settle, allowFarFieldRebuild),
    );
    this.status = residency.pendingChunks > 0
      ? `Streaming ${residency.pendingChunks} pending chunk(s)`
      : residency.generatedChunks > 0 || residency.evictedChunks > 0
      ? `Streamed ${residency.generatedChunks} chunk(s), evicted ${residency.evictedChunks}`
      : "Residency updated";
    return cloneResidencySummary(this.lastStreamSummary);
  }

  private prefetchFarFieldSummaries(settle: boolean): void {
    const maxGenerateChunks = settle
      ? Number.POSITIVE_INFINITY
      : this.lastStreamSummary.pendingChunks > 0
      ? 0
      : DEFAULT_MAX_FAR_FIELD_SURFACE_PREFETCH_CHUNKS_PER_FRAME;
    if (maxGenerateChunks <= 0) {
      return;
    }
    this.world.prefetchFarFieldSummariesAround(
      this.player.feetPosition,
      this.farField.getMaxRadiusWorldUnits(),
      maxGenerateChunks,
    );
  }

  private isPressed(...codes: string[]): boolean {
    return codes.some((code) => this.pressedKeys.has(code));
  }

  private syncCameraToPlayer(): void {
    this.camera.position = getPlayerEyePosition(this.player);
  }

  private flushMeshBuildBudget(maxChunks = DEFAULT_MAX_MESH_REBUILDS_PER_FRAME): void {
    if (this.asyncChunkMeshing) {
      const startedAt = performance.now();
      let meshCount = 0;
      let newMeshCount = 0;
      let remeshCount = 0;
      let triangleCount = 0;
      const dirtyChunks = collectDirtyChunks(this.world, this.player.feetPosition);
      const priorityChunkX = Math.floor(this.player.feetPosition[0] / this.world.chunkSize);
      const priorityChunkY = Math.floor(this.player.feetPosition[1] / this.world.chunkSize);
      const priorityChunkZ = Math.floor(this.player.feetPosition[2] / this.world.chunkSize);

      for (const completed of this.asyncChunkMeshing.drainCompletedMeshes()) {
        const chunk = this.world.getResidentChunk(completed.coord.x, completed.coord.y, completed.coord.z);
        if (!chunk || chunk.meshRevision !== completed.meshRevision) {
          continue;
        }
        chunk.mesh = buildChunkMeshFromOpaqueGeometry(
          this.world,
          completed.coord.x,
          completed.coord.y,
          completed.coord.z,
          completed.opaqueMesh,
        );
        chunk.meshDirty = false;
        chunk.pendingMeshRevision = null;
        chunk.gpuDirty = true;
        meshCount += 1;
        if (chunk.meshBuilt) {
          remeshCount += 1;
        } else {
          newMeshCount += 1;
          chunk.meshBuilt = true;
        }
        triangleCount += chunk.mesh?.triangleCount ?? 0;
      }

      let syncBuiltCount = 0;
      for (const chunk of dirtyChunks) {
        if (syncBuiltCount >= Math.min(MAX_SYNC_NEAR_MESH_REBUILDS_PER_FRAME, maxChunks)) {
          break;
        }
        if (!chunk.meshDirty) {
          continue;
        }
        if (!shouldSyncBuildUrgentChunk(chunk, priorityChunkX, priorityChunkY, priorityChunkZ)) {
          continue;
        }
        const hasPendingJob = this.asyncChunkMeshing.hasPendingChunk(chunk.coord.x, chunk.coord.y, chunk.coord.z);
        const wasBuilt = chunk.meshBuilt;
        chunk.mesh = buildChunkMesh(this.world, chunk.coord.x, chunk.coord.y, chunk.coord.z);
        chunk.meshDirty = false;
        chunk.gpuDirty = true;
        if (hasPendingJob) {
          chunk.pendingMeshRevision = chunk.meshRevision;
        } else {
          chunk.pendingMeshRevision = null;
        }
        syncBuiltCount += 1;
        meshCount += 1;
        if (wasBuilt) {
          remeshCount += 1;
        } else {
          newMeshCount += 1;
          chunk.meshBuilt = true;
        }
        triangleCount += chunk.mesh?.triangleCount ?? 0;
      }

      let scheduledCount = 0;
      for (const chunk of dirtyChunks) {
        if (scheduledCount >= maxChunks) {
          break;
        }
        if (!chunk.meshDirty) {
          continue;
        }
        if (chunk.pendingMeshRevision === chunk.meshRevision) {
          continue;
        }
        const input = createOpaqueChunkMeshingInput(
          this.world,
          chunk.coord.x,
          chunk.coord.y,
          chunk.coord.z,
          { cloneData: true },
        );
        if (!input) {
          continue;
        }
        if (!this.asyncChunkMeshing.requestChunk(input, chunk.meshRevision)) {
          break;
        }
        chunk.pendingMeshRevision = chunk.meshRevision;
        scheduledCount += 1;
      }

      const elapsedMs = performance.now() - startedAt;
      this.lastMeshBuildSummary = {
        meshCount,
        newMeshCount,
        remeshCount,
        triangleCount,
        elapsedMs,
      };
      this.meshMs = elapsedMs;
      if (meshCount > 0) {
        this.farFieldReadyMaskRevision += 1;
      }
      return;
    }
    const meshSummary = rebuildDirtyMeshes(this.world, maxChunks, {
      priorityPosition: this.player.feetPosition,
    });
    this.lastMeshBuildSummary = meshSummary;
    this.meshMs = meshSummary.elapsedMs;
    if (meshSummary.meshCount > 0) {
      this.farFieldReadyMaskRevision += 1;
    }
  }

  private getRenderReadyFarFieldMask() {
    return this.world.getFarFieldExclusionMask("render-ready", this.presentedFarFieldReadyMaskRevision);
  }

  private buildFarFieldRenderMask(): FarFieldRenderMask {
    const playerChunkX = Math.floor(this.player.feetPosition[0] / this.world.chunkSize);
    const playerChunkZ = Math.floor(this.player.feetPosition[2] / this.world.chunkSize);
    const halfSpan = FAR_FIELD_RENDER_MASK_SPAN_CHUNKS / 2;
    const originChunkX = playerChunkX - halfSpan;
    const originChunkZ = playerChunkZ - halfSpan;
    if (
      this.cachedFarFieldRenderMask
      && this.cachedFarFieldRenderMaskRevision === this.farFieldReadyMaskRevision
      && this.cachedFarFieldRenderMaskOriginChunkX === originChunkX
      && this.cachedFarFieldRenderMaskOriginChunkZ === originChunkZ
    ) {
      return this.cachedFarFieldRenderMask;
    }
    const renderReadyMask = this.world.getFarFieldExclusionMask("render-ready");
    const words = new Uint32Array((FAR_FIELD_RENDER_MASK_SPAN_CHUNKS * FAR_FIELD_RENDER_MASK_SPAN_CHUNKS) / 32);
    for (let localZ = 0; localZ < FAR_FIELD_RENDER_MASK_SPAN_CHUNKS; localZ += 1) {
      const chunkZ = originChunkZ + localZ;
      for (let localX = 0; localX < FAR_FIELD_RENDER_MASK_SPAN_CHUNKS; localX += 1) {
        const chunkX = originChunkX + localX;
        if (!renderReadyMask.excludesCell(
          chunkX * this.world.chunkSize,
          (chunkX + 1) * this.world.chunkSize,
          chunkZ * this.world.chunkSize,
          (chunkZ + 1) * this.world.chunkSize,
        )) {
          continue;
        }
        const bitIndex = localX + localZ * FAR_FIELD_RENDER_MASK_SPAN_CHUNKS;
        words[bitIndex >>> 5] |= (1 << (bitIndex & 31)) >>> 0;
      }
    }
    const mask = {
      originChunkX,
      originChunkZ,
      spanChunks: FAR_FIELD_RENDER_MASK_SPAN_CHUNKS,
      chunkSizeWorldUnits: this.world.chunkSize,
      words,
    };
    this.cachedFarFieldRenderMaskRevision = this.farFieldReadyMaskRevision;
    this.cachedFarFieldRenderMaskOriginChunkX = originChunkX;
    this.cachedFarFieldRenderMaskOriginChunkZ = originChunkZ;
    this.cachedFarFieldRenderMask = mask;
    return mask;
  }

  private resolveFarFieldRebuildBudget(settle: boolean, allowFarFieldRebuild: boolean): number {
    if (settle) {
      return Number.POSITIVE_INFINITY;
    }
    if (!allowFarFieldRebuild) {
      return 0;
    }
    if (this.lastStreamSummary.pendingChunks > 0 || this.world.countDirtyResidentChunks() > 0) {
      return 0;
    }
    return this.streamingBudgets.maxFarFieldBandRebuildsPerFrame;
  }

  private syncPresentedFarFieldMaskRevision(force = false): void {
    if (force || (this.lastStreamSummary.pendingChunks === 0 && this.world.countDirtyResidentChunks() === 0)) {
      this.presentedFarFieldReadyMaskRevision = this.farFieldReadyMaskRevision;
    }
  }

  private renderCurrentFrame(): {
    frameStats: RenderStats;
    frameCpuMs: number;
  } | null {
    if (!this.renderer) {
      return null;
    }
    this.renderer.configureCanvas(this.canvas);
    const aspect = this.canvas.width / this.canvas.height;
    const cameraMatrices = buildFirstPersonCameraMatrices(this.camera, aspect);
    const renderEnvironment = this.resolveRenderEnvironment();
    const cpuStartedAt = performance.now();
    const frameStats = this.renderer.render(
      this.world,
      cameraMatrices,
      null,
      0,
      this.farField.getRenderables(),
      this.buildFarFieldRenderMask(),
      renderEnvironment,
    );
    const frameCpuMs = performance.now() - cpuStartedAt;
    this.lastRenderStats = frameStats;
    this.lastFrameCpuMs = frameCpuMs;
    this.drawCalls = frameStats.drawCalls;
    this.triangles = frameStats.triangles;
    return {
      frameStats,
      frameCpuMs,
    };
  }

  private async runRouteExperienceFrame(
    target: Pick<RouteBenchmarkFrameTarget, "frame" | "simTimeSeconds" | "distanceMeters" | "feetPosition" | "yaw" | "pitch">
      & { phase: "move" | "settle" },
    seamProbeStrideFrames: number,
    captureStrideFrames: number,
    captureWidth: number,
    captureHeight: number,
    referenceDiffStrideFrames: number,
    referenceDiffLimit: number,
    sampleIndex: number,
    capturedFrameCount: number,
  ): Promise<{
    sample: RouteExperienceFrameSample;
    capturedFrame: CapturedBenchmarkFrame | null;
  }> {
    const gameplayStartedAt = performance.now();
    const movementStartedAt = performance.now();
    teleportPlayerToFeetPosition(this.player, target.feetPosition);
    this.player.grounded = true;
    this.camera.yaw = target.yaw;
    this.camera.pitch = target.pitch;
    this.syncCameraToPlayer();
    const movementMs = performance.now() - movementStartedAt;
    const dirtyResidentChunks = this.world.countDirtyResidentChunks();
    const residency = this.syncWorldAroundPlayer(
      false,
      shouldAllowFarFieldCatchupWhileMoving(
        target.phase !== "move",
        this.lastStreamSummary.pendingChunks,
        dirtyResidentChunks,
        this.lastFarFieldSummary.pendingBands,
        target.frame,
        MOVEMENT_FAR_FIELD_CATCHUP_CADENCE_FRAMES,
      ),
    );
    const render = this.renderCurrentFrame();
    const gameplayFrameMs = performance.now() - gameplayStartedAt;
    const dirtyResidentMeshes = summarizeDirtyResidentMeshes(this.world);
    const frameProbe = render ?? {
      frameStats: zeroRenderStats(),
      frameCpuMs: 0,
    };
    const diagnosticsStartedAt = performance.now();
    const detailCoverage = this.probeRenderReadyCoverage();
    const visibleGround = this.probeVisibleGroundCoverage();
    const seamProbe = target.frame % seamProbeStrideFrames === 0 || visibleGround.uncoveredCount > 0
      ? this.probeNearFarSeamGaps()
      : null;
    let screenVoid: BottomCenterVoidProbe | null = null;
    let capturedFrame: CapturedBenchmarkFrame | null = null;
    let captureDiagnosticsMs = 0;
    const shouldCaptureVoid = target.frame % captureStrideFrames === 0 || visibleGround.uncoveredCount > 0;
    const shouldCaptureReference = referenceDiffStrideFrames > 0
      && capturedFrameCount < referenceDiffLimit
      && target.frame % referenceDiffStrideFrames === 0;
    if (shouldCaptureVoid || shouldCaptureReference) {
      const captureStartedAt = performance.now();
      const image = await this.captureRouteFrameImage(captureWidth, captureHeight);
      screenVoid = image ? analyzeBottomCenterVoid(image) : null;
      if (image && shouldCaptureReference) {
        capturedFrame = {
          sampleIndex,
          target: {
            frame: target.frame,
            phase: target.phase,
            simTimeSeconds: target.simTimeSeconds,
            distanceMeters: target.distanceMeters,
            feetPosition: [...target.feetPosition],
            yaw: target.yaw,
            pitch: target.pitch,
          },
          image,
        };
      }
      captureDiagnosticsMs = performance.now() - captureStartedAt;
    }
    const diagnosticsMs = performance.now() - diagnosticsStartedAt;
    const renderOtherMs = Math.max(
      0,
      frameProbe.frameCpuMs
        - frameProbe.frameStats.syncResourcesMs
        - frameProbe.frameStats.uploadMs
        - frameProbe.frameStats.encodeMs,
    );
    const accountedFrameMs = movementMs
      + residency.elapsedMs
      + this.lastMeshBuildSummary.elapsedMs
      + this.lastFarFieldSummary.elapsedMs
      + frameProbe.frameCpuMs;
    const unmeasuredFrameMs = Math.max(0, gameplayFrameMs - accountedFrameMs);
    const maxFarFieldBand = this.lastFarFieldSummary.bandBuilds.reduce<{
      label: string | null;
      elapsedMs: number;
    }>((current, band) => band.elapsedMs > current.elapsedMs
      ? { label: band.label, elapsedMs: band.elapsedMs }
      : current, { label: null, elapsedMs: 0 });
    const suspiciousHole = visibleGround.uncoveredCount > 0
      || (seamProbe?.gapCount ?? 0) > 0
      || (screenVoid?.suspicious ?? false);

    return {
      sample: {
        frame: target.frame,
        phase: target.phase,
        simTimeSeconds: target.simTimeSeconds,
        routeDistanceMeters: target.distanceMeters,
        feetPosition: [...this.player.feetPosition],
        yaw: this.camera.yaw,
        pitch: this.camera.pitch,
        changed: residency.changed,
        complete: residency.complete,
        pendingChunks: residency.pendingChunks,
        pendingMeshJobs: this.asyncChunkMeshing?.getPendingCount() ?? 0,
        dirtyResidentChunks: dirtyResidentMeshes.dirtyResidentChunks,
        dirtyMeshlessResidentChunks: dirtyResidentMeshes.dirtyMeshlessResidentChunks,
        dirtyRetainedMeshResidentChunks: dirtyResidentMeshes.dirtyRetainedMeshResidentChunks,
        generatedChunks: residency.generatedChunks,
        evictedChunks: residency.evictedChunks,
        movementMs,
        streamMs: residency.elapsedMs,
        meshMs: this.lastMeshBuildSummary.elapsedMs,
        meshCount: this.lastMeshBuildSummary.meshCount,
        farFieldMs: this.lastFarFieldSummary.elapsedMs,
        farFieldSampleCacheMs: this.lastFarFieldSummary.sampleCacheMs,
        farFieldMeshBuildMs: this.lastFarFieldSummary.meshBuildMs,
        farFieldSampledCellCount: this.lastFarFieldSummary.sampledCellCount,
        farFieldMaxBandMs: maxFarFieldBand.elapsedMs,
        farFieldMaxBandLabel: maxFarFieldBand.label,
        farFieldBuiltBands: this.lastFarFieldSummary.builtBands,
        farFieldPendingBands: this.lastFarFieldSummary.pendingBands,
        gameplayFrameMs,
        accountedFrameMs,
        unmeasuredFrameMs,
        diagnosticsMs,
        captureDiagnosticsMs,
        renderCpuMs: frameProbe.frameCpuMs,
        renderSyncMs: frameProbe.frameStats.syncResourcesMs,
        renderUploadMs: frameProbe.frameStats.uploadMs,
        renderEncodeMs: frameProbe.frameStats.encodeMs,
        renderOtherMs,
        uploadChunks: frameProbe.frameStats.uploadChunks,
        uploadBytes: frameProbe.frameStats.uploadBytes,
        drawCalls: frameProbe.frameStats.drawCalls,
        triangles: frameProbe.frameStats.triangles,
        residentNearSamples: detailCoverage.residentSampleCount,
        renderReadyNearSamples: detailCoverage.renderReadySampleCount,
        residentNotReadyNearSamples: detailCoverage.residentNotReadyCount,
        visibleGroundSampleCount: visibleGround.sampleCount,
        visibleGroundUncoveredCount: visibleGround.uncoveredCount,
        visibleGroundResidentNotReadyCount: visibleGround.residentNotReadyCount,
        visibleGroundFarOnlyCount: visibleGround.farOnlyCount,
        seamGapCount: seamProbe?.gapCount ?? 0,
        maxSeamGapMeters: seamProbe?.maxGapDepthMeters ?? 0,
        screenVoidRatio: screenVoid?.clearRatio ?? null,
        screenVoidMaxRunRatio: screenVoid?.maxClearRunRatio ?? null,
        screenVoidSuspicious: screenVoid?.suspicious ?? false,
        settledReferenceChangedRatio: null,
        settledReferenceClearToFilledRatio: null,
        settledReferenceMaxClearToFilledRunRatio: null,
        settledReferenceSuspiciousHole: false,
        suspiciousHole,
      },
      capturedFrame,
    };
  }

  private async captureRouteFrameImage(width: number, height: number): Promise<{
    width: number;
    height: number;
    pixels: Uint8ClampedArray;
  } | null> {
    if (!this.renderer) {
      return null;
    }
    const cameraMatrices = buildFirstPersonCameraMatrices(this.camera, width / height);
    const renderEnvironment = this.resolveRenderEnvironment();
    return await this.renderer.captureImage(
      this.world,
      cameraMatrices,
      width,
      height,
      this.farField.getRenderables(),
      this.buildFarFieldRenderMask(),
      renderEnvironment,
    );
  }

  private resolveRenderEnvironment(): RenderEnvironment {
    if (!this.player.eyeInWater) {
      return DEFAULT_RENDER_ENVIRONMENT;
    }
    const eye = getPlayerEyePosition(this.player);
    const material = this.world.getVoxel(
      Math.floor(eye[0]),
      Math.floor(eye[1]),
      Math.floor(eye[2]),
    );
    if (!this.world.isWaterMaterial(material)) {
      return DEFAULT_RENDER_ENVIRONMENT;
    }
    return buildUnderwaterRenderEnvironment(this.world.getPaletteColor(material));
  }

  private async applySettledReferenceDiffs(
    samples: RouteExperienceFrameSample[],
    capturedFrames: readonly CapturedBenchmarkFrame[],
  ): Promise<void> {
    for (const capturedFrame of capturedFrames) {
      const referenceImage = await this.captureSettledReferenceFrame(capturedFrame.target, capturedFrame.image.width, capturedFrame.image.height);
      if (!referenceImage) {
        continue;
      }
      const diff = analyzeSettledReferenceDiff(capturedFrame.image, referenceImage);
      const sample = samples[capturedFrame.sampleIndex];
      if (!sample) {
        continue;
      }
      sample.settledReferenceChangedRatio = diff.changedRatio;
      sample.settledReferenceClearToFilledRatio = diff.clearToFilledRatio;
      sample.settledReferenceMaxClearToFilledRunRatio = diff.maxClearToFilledRunRatio;
      sample.settledReferenceSuspiciousHole = diff.suspiciousHole;
      sample.suspiciousHole = sample.suspiciousHole || diff.suspiciousHole;
    }
  }

  private async captureSettledReferenceFrame(
    target: CapturedBenchmarkFrame["target"],
    width: number,
    height: number,
  ): Promise<{
    width: number;
    height: number;
    pixels: Uint8ClampedArray;
  } | null> {
    teleportPlayerToFeetPosition(this.player, target.feetPosition);
    this.player.grounded = true;
    this.camera.yaw = target.yaw;
    this.camera.pitch = target.pitch;
    this.syncCameraToPlayer();
    this.syncWorldAroundPlayer(true);
    return await this.captureRouteFrameImage(width, height);
  }

  private async renderProbeFrame(): Promise<GameRenderProbe> {
    const rendered = this.renderCurrentFrame();
    if (!rendered || !this.renderer) {
      return zeroGameRenderProbe();
    }
    await this.renderer.waitForGpuIdle();
    return {
      frameCpuMs: rendered.frameCpuMs,
      syncResourcesMs: rendered.frameStats.syncResourcesMs,
      uploadMs: rendered.frameStats.uploadMs,
      uploadChunks: rendered.frameStats.uploadChunks,
      uploadBytes: rendered.frameStats.uploadBytes,
      encodeMs: rendered.frameStats.encodeMs,
      drawCalls: rendered.frameStats.drawCalls,
      triangles: rendered.frameStats.triangles,
    };
  }

  private recordBootstrapBenchmarkSample(gameplayFrameMs: number): void {
    if (this.bootstrapBenchmarkComplete) {
      return;
    }
    const bootstrap = this.getBootstrapReadiness();
    this.bootstrapBenchmarkSamples.push({
      frame: this.bootstrapBenchmarkSamples.length,
      elapsedMs: performance.now() - this.bootstrapBenchmarkStartedAt,
      gameplayFrameMs,
      renderCpuMs: this.lastFrameCpuMs,
      renderSyncMs: this.lastRenderStats.syncResourcesMs,
      renderUploadMs: this.lastRenderStats.uploadMs,
      renderEncodeMs: this.lastRenderStats.encodeMs,
      uploadChunks: this.lastRenderStats.uploadChunks,
      uploadBytes: this.lastRenderStats.uploadBytes,
      drawCalls: this.drawCalls,
      triangles: this.triangles,
      streamMs: this.lastStreamSummary.elapsedMs,
      meshMs: this.lastMeshBuildSummary.elapsedMs,
      farFieldMs: this.lastFarFieldSummary.elapsedMs,
      pendingChunks: this.lastStreamSummary.pendingChunks,
      pendingMeshJobs: bootstrap.pendingMeshJobs,
      dirtyResidentChunks: bootstrap.dirtyResidentMeshes.dirtyResidentChunks,
      dirtyMeshlessResidentChunks: bootstrap.dirtyResidentMeshes.dirtyMeshlessResidentChunks,
      dirtyRetainedMeshResidentChunks: bootstrap.dirtyResidentMeshes.dirtyRetainedMeshResidentChunks,
      generatedChunks: this.lastStreamSummary.generatedChunks,
      evictedChunks: this.lastStreamSummary.evictedChunks,
      farFieldPendingBands: this.lastFarFieldSummary.pendingBands,
      playableReady: bootstrap.playableReady,
      visualReady: bootstrap.visualReady,
    });
    if (bootstrap.playableReady) {
      this.bootstrapPlayableReady = true;
    }
    if (bootstrap.visualReady) {
      this.bootstrapBenchmarkComplete = true;
    }
  }

  private getBootstrapReadiness(): {
    dirtyResidentMeshes: ReturnType<typeof summarizeDirtyResidentMeshes>;
    pendingMeshJobs: number;
    playableReady: boolean;
    visualReady: boolean;
  } {
    const dirtyResidentMeshes = summarizeDirtyResidentMeshes(this.world);
    const pendingMeshJobs = this.asyncChunkMeshing?.getPendingCount() ?? 0;
    const hasResidentChunks = this.world.getStats().chunkCount > 0;
    const playerChunkX = Math.floor(this.player.feetPosition[0] / this.world.chunkSize);
    const playerChunkY = Math.floor(this.player.feetPosition[1] / this.world.chunkSize);
    const playerChunkZ = Math.floor(this.player.feetPosition[2] / this.world.chunkSize);
    const localMissingColumns = countMissingResidentColumnsAround(
      this.world,
      playerChunkX,
      playerChunkZ,
      BOOTSTRAP_PLAYABLE_COLUMN_RADIUS_CHUNKS,
    );
    const urgentDirtyMeshlessChunks = countUrgentDirtyMeshlessChunks(
      this.world,
      playerChunkX,
      playerChunkY,
      playerChunkZ,
    );
    const supportChunk = this.world.getResidentChunk(playerChunkX, Math.floor((this.player.feetPosition[1] - 1) / this.world.chunkSize), playerChunkZ);
    const playableReady = hasResidentChunks
      && supportChunk !== null
      && localMissingColumns === 0
      && urgentDirtyMeshlessChunks === 0;
    const visualReady = playableReady && this.lastFarFieldSummary.pendingBands === 0;
    return {
      dirtyResidentMeshes,
      pendingMeshJobs,
      playableReady,
      visualReady,
    };
  }

  private pushHud(force = false): void {
    const now = performance.now();
    if (!force && now - this.lastHudPushAt < HUD_PUSH_INTERVAL_MS) {
      return;
    }
    this.lastHudPushAt = now;
    this.onHudUpdate?.(this.getDebugSnapshot());
  }

  private refreshDiscoveryJournal(force = false): ExplorationJournalSnapshot {
    const now = performance.now();
    const currentFeetPosition = this.player.feetPosition;
    if (!force && this.lastDiscoverySampleFeetPosition) {
      const deltaX = currentFeetPosition[0] - this.lastDiscoverySampleFeetPosition[0];
      const deltaZ = currentFeetPosition[2] - this.lastDiscoverySampleFeetPosition[2];
      const movedFarEnough = Math.hypot(deltaX, deltaZ) >= DISCOVERY_SAMPLE_MOVE_THRESHOLD_WORLD_UNITS;
      if (!movedFarEnough && now - this.lastDiscoverySampleAt < DISCOVERY_SAMPLE_INTERVAL_MS) {
        return this.lastDiscoverySnapshot;
      }
    }
    this.lastDiscoverySampleAt = now;
    this.lastDiscoverySampleFeetPosition = [...currentFeetPosition] as Vec3;
    this.lastDiscoverySnapshot = this.explorationJournal.observe(this.sampleExplorationObservation());
    return this.lastDiscoverySnapshot;
  }

  private sampleExplorationObservation(): ExplorationObservation {
    const centerX = Math.floor(this.player.feetPosition[0]);
    const centerZ = Math.floor(this.player.feetPosition[2]);
    const centerProbe = this.generator.sampleBiomeProbe(centerX, centerZ);
    const observedUndergroundBiomeId = resolveObservedUndergroundBiomeId(
      this.world,
      this.camera.position,
      centerProbe.surfaceY,
      centerProbe.undergroundBiomeId,
    );
    const landmarkIds: string[] = [];
    let currentLandmarkId: string | null = centerProbe.landmarkId;
    if (centerProbe.landmarkId) {
      landmarkIds.push(centerProbe.landmarkId);
    }
    for (const [offsetX, offsetZ] of DISCOVERY_LANDMARK_SAMPLE_OFFSETS) {
      if (offsetX === 0 && offsetZ === 0) {
        continue;
      }
      const probe = this.generator.sampleBiomeProbe(centerX + offsetX, centerZ + offsetZ);
      if (!probe.landmarkId) {
        continue;
      }
      if (currentLandmarkId === null) {
        currentLandmarkId = probe.landmarkId;
      }
      landmarkIds.push(probe.landmarkId);
    }
    return {
      biomeId: centerProbe.biomeId,
      undergroundBiomeId: observedUndergroundBiomeId,
      regionalVariantId: centerProbe.regionalVariantId,
      landmarkIds,
      currentLandmarkId,
    };
  }
}

function toDegrees(value: number): number {
  return value * 180 / Math.PI;
}

function toChunkTuple(coord: { x: number; y: number; z: number }): [number, number, number] {
  return [coord.x, coord.y, coord.z];
}

function zeroRenderStats(): RenderStats {
  return {
    drawCalls: 0,
    triangles: 0,
    syncResourcesMs: 0,
    uploadMs: 0,
    uploadChunks: 0,
    uploadBytes: 0,
    encodeMs: 0,
  };
}

function zeroGameRenderProbe(): GameRenderProbe {
  return {
    frameCpuMs: 0,
    syncResourcesMs: 0,
    uploadMs: 0,
    uploadChunks: 0,
    uploadBytes: 0,
    encodeMs: 0,
    drawCalls: 0,
    triangles: 0,
  };
}

function formatInventoryMaterial(material: number | null): string {
  if (!material) {
    return "Empty";
  }
  return materialToHexColor(material);
}

function shouldSyncBuildUrgentChunk(
  chunk: { coord: { x: number; y: number; z: number }; meshBuilt: boolean },
  priorityChunkX: number,
  priorityChunkY: number,
  priorityChunkZ: number,
): boolean {
  if (chunk.meshBuilt) {
    return false;
  }
  const planarDistance = Math.max(
    Math.abs(chunk.coord.x - priorityChunkX),
    Math.abs(chunk.coord.z - priorityChunkZ),
  );
  if (planarDistance > SYNC_NEAR_MESH_RADIUS_CHUNKS) {
    return false;
  }
  return Math.abs(chunk.coord.y - priorityChunkY) <= 1;
}

function buildEyePositionForChunkCenter(
  chunkX: number,
  chunkZ: number,
  feetY: number,
  chunkSize: number,
  eyeHeight: number,
): Vec3 {
  return [
    chunkX * chunkSize + chunkSize * 0.5,
    feetY + eyeHeight,
    chunkZ * chunkSize + chunkSize * 0.5,
  ];
}

function buildFeetPositionForChunkCenter(
  chunkX: number,
  chunkZ: number,
  feetY: number,
  chunkSize: number,
): Vec3 {
  return [
    chunkX * chunkSize + chunkSize * 0.5,
    feetY,
    chunkZ * chunkSize + chunkSize * 0.5,
  ];
}

function teleportPlayerToFeetPosition(player: PlayerState, feetPosition: Vec3): void {
  player.feetPosition = [...feetPosition];
  player.velocity = [0, 0, 0];
}

function lerpVec3(from: Vec3, to: Vec3, t: number): Vec3 {
  return [
    from[0] + (to[0] - from[0]) * t,
    from[1] + (to[1] - from[1]) * t,
    from[2] + (to[2] - from[2]) * t,
  ];
}

function buildIncrementalSample(
  frame: number,
  phase: "move" | "settle",
  leg: number,
  residency: ResidencyUpdateSummary,
  mesh: MeshBuildSummary,
  farField: FarFieldUpdateSummary,
  detailCoverage: RenderReadyCoverageProbe,
  render: GameRenderProbe,
): IncrementalCrossingSample {
  return {
    frame,
    phase,
    leg,
    changed: residency.changed,
    complete: residency.complete,
    pendingChunks: residency.pendingChunks,
    generatedChunks: residency.generatedChunks,
    evictedChunks: residency.evictedChunks,
    streamMs: residency.elapsedMs,
    meshMs: mesh.elapsedMs,
    meshCount: mesh.meshCount,
    farFieldMs: farField.elapsedMs,
    farFieldBuiltBands: farField.builtBands,
    farFieldPendingBands: farField.pendingBands,
    residentNearSamples: detailCoverage.residentSampleCount,
    renderReadyNearSamples: detailCoverage.renderReadySampleCount,
    residentNotReadyNearSamples: detailCoverage.residentNotReadyCount,
    frameCpuMs: render.frameCpuMs,
    syncMs: render.syncResourcesMs,
    uploadMs: render.uploadMs,
    uploadChunks: render.uploadChunks,
    uploadBytes: render.uploadBytes,
    encodeMs: render.encodeMs,
  };
}

function coverageToIssueSample(
  worldX: number,
  worldZ: number,
  distanceMeters: number,
  coverage: ReturnType<ProceduralFarField["getCoverageAt"]>,
): LodCoverageIssueSample {
  return {
    worldX,
    worldZ,
    distanceMeters,
    bands: coverage.map((band) => band.label),
    sampleStrideMeters: coverage.map((band) => worldUnitsToMeters(band.sampleStride)),
  };
}

function pushIssueSample(target: LodCoverageIssueSample[], sample: LodCoverageIssueSample): void {
  if (target.length >= 8) {
    return;
  }
  target.push(sample);
}

function cloneResidencySummary(summary: ResidencyUpdateSummary): ResidencyUpdateSummary {
  return {
    ...summary,
    generatedChunkCoords: summary.generatedChunkCoords.map((coord) => ({ ...coord })),
    evictedChunkCoords: summary.evictedChunkCoords.map((coord) => ({ ...coord })),
    phaseMs: { ...summary.phaseMs },
  };
}

function createIdleResidencySummary(
  summary: ResidencyUpdateSummary,
  anchor: StreamAnchor,
  radiusChunks: number,
  residentChunks: number,
  dirtyResidentChunks: number,
): ResidencyUpdateSummary {
  return {
    ...summary,
    changed: false,
    complete: true,
    centerChunkX: anchor.chunkX,
    centerChunkZ: anchor.chunkZ,
    radiusChunks,
    generatedChunks: 0,
    evictedChunks: 0,
    pendingChunks: 0,
    emptyChunksSkipped: 0,
    cachedEmptyChunkHits: 0,
    touchedNeighborChunks: 0,
    residentChunks,
    dirtyResidentChunks,
    elapsedMs: 0,
    generatedChunkCoords: [],
    evictedChunkCoords: [],
    phaseMs: zeroResidencyPhaseMetrics(),
  };
}

function summarizeChunkBoundaryBenchmark(samples: readonly ChunkBoundaryBenchmarkSample[]): ChunkBoundaryBenchmarkSummary {
  const streamSamples = samples.map((sample) => sample.streamMs);
  const meshSamples = samples.map((sample) => sample.meshMs);
  const frameCpuSamples = samples.map((sample) => sample.frameCpuMs);
  const syncSamples = samples.map((sample) => sample.syncMs);
  const uploadSamples = samples.map((sample) => sample.uploadMs);
  const uploadChunkSamples = samples.map((sample) => sample.uploadChunks);
  const uploadByteSamples = samples.map((sample) => sample.uploadBytes);
  return {
    sampleCount: samples.length,
    changedCount: samples.filter((sample) => sample.changed).length,
    avgStreamMs: average(streamSamples),
    p95StreamMs: percentile(streamSamples, 0.95),
    maxStreamMs: maxValue(streamSamples),
    avgMeshMs: average(meshSamples),
    p95MeshMs: percentile(meshSamples, 0.95),
    maxMeshMs: maxValue(meshSamples),
    avgFrameCpuMs: average(frameCpuSamples),
    p95FrameCpuMs: percentile(frameCpuSamples, 0.95),
    maxFrameCpuMs: maxValue(frameCpuSamples),
    avgSyncMs: average(syncSamples),
    p95SyncMs: percentile(syncSamples, 0.95),
    maxSyncMs: maxValue(syncSamples),
    avgUploadMs: average(uploadSamples),
    p95UploadMs: percentile(uploadSamples, 0.95),
    maxUploadMs: maxValue(uploadSamples),
    avgUploadChunks: average(uploadChunkSamples),
    maxUploadChunks: maxValue(uploadChunkSamples),
    avgUploadBytes: average(uploadByteSamples),
    maxUploadBytes: maxValue(uploadByteSamples),
  };
}

function summarizeIncrementalCrossing(samples: readonly IncrementalCrossingSample[]): IncrementalCrossingSummary {
  const workSamples = samples.map((sample) => sample.streamMs + sample.meshMs + sample.farFieldMs + sample.frameCpuMs);
  const farFieldSamples = samples.map((sample) => sample.farFieldMs);
  const streamSamples = samples.map((sample) => sample.streamMs);
  const meshSamples = samples.map((sample) => sample.meshMs);
  const frameCpuSamples = samples.map((sample) => sample.frameCpuMs);
  const uploadSamples = samples.map((sample) => sample.uploadMs);
  const residentNotReadySamples = samples.map((sample) => sample.residentNotReadyNearSamples);
  return {
    sampleCount: samples.length,
    workFrameCount: samples.filter((sample) =>
      sample.streamMs > 0
      || sample.meshMs > 0
      || sample.farFieldMs > 0
      || sample.uploadChunks > 0
      || sample.farFieldPendingBands > 0
    ).length,
    changedCount: samples.filter((sample) => sample.changed).length,
    incompleteFrameCount: samples.filter((sample) =>
      !sample.complete || sample.pendingChunks > 0 || sample.farFieldPendingBands > 0
    ).length,
    avgWorkMs: average(workSamples),
    p95WorkMs: percentile(workSamples, 0.95),
    maxWorkMs: maxValue(workSamples),
    avgFarFieldMs: average(farFieldSamples),
    p95FarFieldMs: percentile(farFieldSamples, 0.95),
    maxFarFieldMs: maxValue(farFieldSamples),
    avgResidentNotReadyNearSamples: average(residentNotReadySamples),
    maxResidentNotReadyNearSamples: maxValue(residentNotReadySamples),
    avgStreamMs: average(streamSamples),
    p95StreamMs: percentile(streamSamples, 0.95),
    maxStreamMs: maxValue(streamSamples),
    avgMeshMs: average(meshSamples),
    p95MeshMs: percentile(meshSamples, 0.95),
    maxMeshMs: maxValue(meshSamples),
    avgFrameCpuMs: average(frameCpuSamples),
    p95FrameCpuMs: percentile(frameCpuSamples, 0.95),
    maxFrameCpuMs: maxValue(frameCpuSamples),
    avgUploadMs: average(uploadSamples),
    p95UploadMs: percentile(uploadSamples, 0.95),
    maxUploadMs: maxValue(uploadSamples),
    maxPendingChunks: maxValue(samples.map((sample) => sample.pendingChunks)),
  };
}

function summarizeRouteExperienceBenchmark(
  samples: readonly RouteExperienceFrameSample[],
  plan: {
    totalDistanceMeters: number;
    sampleHz: number;
    speedMetersPerSecond: number;
  },
): RouteExperienceBenchmarkSummary {
  const accounting = summarizeRouteFrameAccounting(samples.map((sample) => ({
    gameplayFrameMs: sample.gameplayFrameMs,
    movementMs: sample.movementMs,
    streamMs: sample.streamMs,
    meshMs: sample.meshMs,
    farFieldMs: sample.farFieldMs,
    renderCpuMs: sample.renderCpuMs,
  })));
  const streamSamples = samples.map((sample) => sample.streamMs);
  const meshSamples = samples.map((sample) => sample.meshMs);
  const farFieldSamples = samples.map((sample) => sample.farFieldMs);
  const farFieldSampleCacheSamples = samples.map((sample) => sample.farFieldSampleCacheMs);
  const farFieldMeshBuildSamples = samples.map((sample) => sample.farFieldMeshBuildMs);
  const farFieldSampledCellSamples = samples.map((sample) => sample.farFieldSampledCellCount);
  const renderCpuSamples = samples.map((sample) => sample.renderCpuMs);
  const renderOtherSamples = samples.map((sample) => sample.renderOtherMs);
  const residentNotReadySamples = samples.map((sample) => sample.residentNotReadyNearSamples);
  const visibleGroundUncoveredSamples = samples.map((sample) => sample.visibleGroundUncoveredCount);
  const visibleGroundResidentNotReadySamples = samples.map((sample) => sample.visibleGroundResidentNotReadyCount);
  const diagnosticsSamples = samples.map((sample) => sample.diagnosticsMs);
  const captureDiagnosticsSamples = samples.map((sample) => sample.captureDiagnosticsMs);
  const settledReferenceChangedSamples = samples.map((sample) => sample.settledReferenceChangedRatio ?? 0);
  const settledReferenceClearToFilledSamples = samples.map((sample) => sample.settledReferenceClearToFilledRatio ?? 0);
  const settledReferenceClearToFilledRunSamples = samples.map((sample) =>
    sample.settledReferenceMaxClearToFilledRunRatio ?? 0);
  const settleCompletion = samples.find((sample) =>
    sample.phase === "settle"
    && sample.complete
    && sample.pendingChunks === 0
    && sample.dirtyResidentChunks === 0
    && sample.farFieldPendingBands === 0
    && sample.visibleGroundUncoveredCount === 0);

  return {
    sampleCount: samples.length,
    moveFrameCount: samples.filter((sample) => sample.phase === "move").length,
    settleFrameCount: samples.filter((sample) => sample.phase === "settle").length,
    incompleteFrameCount: samples.filter((sample) =>
      !sample.complete
      || sample.pendingChunks > 0
      || sample.dirtyResidentChunks > 0
      || sample.farFieldPendingBands > 0).length,
    totalDistanceMeters: plan.totalDistanceMeters,
    sampleHz: plan.sampleHz,
    speedMetersPerSecond: plan.speedMetersPerSecond,
    totalGameplayFrameMs: accounting.totalGameplayFrameMs,
    totalAccountedFrameMs: accounting.totalAccountedMs,
    totalUnmeasuredFrameMs: accounting.totalUnmeasuredMs,
    unmeasuredFrameRatio: accounting.totalGameplayFrameMs === 0
      ? 0
      : accounting.totalUnmeasuredMs / accounting.totalGameplayFrameMs,
    totalDiagnosticsMs: sumNumbers(diagnosticsSamples),
    totalCaptureDiagnosticsMs: sumNumbers(captureDiagnosticsSamples),
    avgGameplayFrameMs: accounting.avgGameplayFrameMs,
    p95GameplayFrameMs: accounting.p95GameplayFrameMs,
    maxGameplayFrameMs: accounting.maxGameplayFrameMs,
    avgMeasuredWorkMs: accounting.avgMeasuredWorkMs,
    p95MeasuredWorkMs: accounting.p95MeasuredWorkMs,
    maxMeasuredWorkMs: accounting.maxMeasuredWorkMs,
    avgUnmeasuredFrameMs: accounting.avgUnmeasuredMs,
    p95UnmeasuredFrameMs: accounting.p95UnmeasuredMs,
    maxUnmeasuredFrameMs: accounting.maxUnmeasuredMs,
    avgStreamMs: average(streamSamples),
    p95StreamMs: percentile(streamSamples, 0.95),
    maxStreamMs: maxValue(streamSamples),
    avgMeshMs: average(meshSamples),
    p95MeshMs: percentile(meshSamples, 0.95),
    maxMeshMs: maxValue(meshSamples),
    avgFarFieldMs: average(farFieldSamples),
    p95FarFieldMs: percentile(farFieldSamples, 0.95),
    maxFarFieldMs: maxValue(farFieldSamples),
    avgFarFieldSampleCacheMs: average(farFieldSampleCacheSamples),
    maxFarFieldSampleCacheMs: maxValue(farFieldSampleCacheSamples),
    avgFarFieldMeshBuildMs: average(farFieldMeshBuildSamples),
    maxFarFieldMeshBuildMs: maxValue(farFieldMeshBuildSamples),
    avgFarFieldSampledCellCount: average(farFieldSampledCellSamples),
    maxFarFieldSampledCellCount: maxValue(farFieldSampledCellSamples),
    maxFarFieldBandBuildMs: maxValue(samples.map((sample) => sample.farFieldMaxBandMs)),
    maxFarFieldBandLabel: samples.reduce<{ label: string | null; elapsedMs: number }>(
      (current, sample) => sample.farFieldMaxBandMs > current.elapsedMs
        ? { label: sample.farFieldMaxBandLabel, elapsedMs: sample.farFieldMaxBandMs }
        : current,
      { label: null, elapsedMs: 0 },
    ).label,
    avgRenderCpuMs: average(renderCpuSamples),
    p95RenderCpuMs: percentile(renderCpuSamples, 0.95),
    maxRenderCpuMs: maxValue(renderCpuSamples),
    avgRenderOtherMs: average(renderOtherSamples),
    maxRenderOtherMs: maxValue(renderOtherSamples),
    avgResidentNotReadyNearSamples: average(residentNotReadySamples),
    maxResidentNotReadyNearSamples: maxValue(residentNotReadySamples),
    avgVisibleGroundUncoveredCount: average(visibleGroundUncoveredSamples),
    maxVisibleGroundUncoveredCount: maxValue(visibleGroundUncoveredSamples),
    avgVisibleGroundResidentNotReadyCount: average(visibleGroundResidentNotReadySamples),
    maxVisibleGroundResidentNotReadyCount: maxValue(visibleGroundResidentNotReadySamples),
    framesWithVisibleGroundGaps: samples.filter((sample) => sample.visibleGroundUncoveredCount > 0).length,
    framesWithSeamGaps: samples.filter((sample) => sample.seamGapCount > 0).length,
    framesWithScreenVoidSignals: samples.filter((sample) => sample.screenVoidSuspicious).length,
    framesWithSettledReferenceHoleSignals: samples.filter((sample) => sample.settledReferenceSuspiciousHole).length,
    framesWithHoleSignals: samples.filter((sample) => sample.suspiciousHole).length,
    maxScreenVoidRatio: maxValue(samples.map((sample) => sample.screenVoidRatio ?? 0)),
    maxSettledReferenceChangedRatio: maxValue(settledReferenceChangedSamples),
    maxSettledReferenceClearToFilledRatio: maxValue(settledReferenceClearToFilledSamples),
    maxSettledReferenceClearToFilledRunRatio: maxValue(settledReferenceClearToFilledRunSamples),
    maxPendingChunks: maxValue(samples.map((sample) => sample.pendingChunks)),
    maxPendingMeshJobs: maxValue(samples.map((sample) => sample.pendingMeshJobs)),
    maxDirtyResidentChunks: maxValue(samples.map((sample) => sample.dirtyResidentChunks)),
    maxDirtyMeshlessResidentChunks: maxValue(samples.map((sample) => sample.dirtyMeshlessResidentChunks)),
    maxDirtyRetainedMeshResidentChunks: maxValue(samples.map((sample) => sample.dirtyRetainedMeshResidentChunks)),
    settleFramesUntilComplete: settleCompletion
      ? samples.filter((sample) => sample.phase === "settle" && sample.frame <= settleCompletion.frame).length
      : null,
  };
}

function normalizeRouteBenchmarkPlanOptions(options: RouteExperienceBenchmarkOptions): {
  durationSeconds: number;
  sampleHz: number;
  speedMetersPerSecond: number;
} {
  return {
    durationSeconds: Math.max(1, options.durationSeconds ?? 10),
    sampleHz: Math.max(1, Math.floor(options.sampleHz ?? 60)),
    speedMetersPerSecond: Math.max(0.1, options.speedMetersPerSecond ?? 4.6),
  };
}

function zeroResidencyPhaseMetrics(): ResidencyUpdateSummary["phaseMs"] {
  return {
    surfaceSampleMs: 0,
    yRangeMs: 0,
    chunkGenerationMs: 0,
    chunkDispatchMs: 0,
    chunkDrainMs: 0,
    summaryDrainMs: 0,
    chunkAdoptionMs: 0,
    evictionMs: 0,
    neighborDirtyMs: 0,
    inFlightChunks: 0,
    completedChunkCacheHits: 0,
    completedGeneratedChunks: 0,
    completedSummaryCacheHits: 0,
    completedGeneratedSummaries: 0,
    completedRegionSummaryCacheHits: 0,
    missingRegionSummaries: 0,
    readyGeneratedChunkBacklog: 0,
  };
}

function isMovementKey(code: string): boolean {
  return code === "KeyW"
    || code === "KeyA"
    || code === "KeyS"
    || code === "KeyD"
    || code === "Space"
    || code === "ControlLeft"
    || code === "ControlRight"
    || code === "AltLeft"
    || code === "AltRight";
}

function clampPositiveInt(value: number, fallback: number): number {
  const normalized = Math.floor(value);
  return Number.isFinite(normalized) && normalized > 0 ? normalized : fallback;
}

function pushVisibleGroundIssueSample(
  target: VisibleGroundCoverageIssueSample[],
  sample: VisibleGroundCoverageIssueSample,
): void {
  if (target.length >= 8) {
    return;
  }
  target.push(sample);
}

function summarizeDirtyResidentMeshes(world: ProceduralResidentWorld): {
  dirtyResidentChunks: number;
  dirtyMeshlessResidentChunks: number;
  dirtyRetainedMeshResidentChunks: number;
} {
  let dirtyResidentChunks = 0;
  let dirtyMeshlessResidentChunks = 0;
  let dirtyRetainedMeshResidentChunks = 0;
  for (const chunk of world.iterateResidentChunks()) {
    if (!chunk.meshDirty) {
      continue;
    }
    dirtyResidentChunks += 1;
    if (chunk.mesh) {
      dirtyRetainedMeshResidentChunks += 1;
    } else {
      dirtyMeshlessResidentChunks += 1;
    }
  }
  return {
    dirtyResidentChunks,
    dirtyMeshlessResidentChunks,
    dirtyRetainedMeshResidentChunks,
  };
}

function countMissingResidentColumnsAround(
  world: ProceduralResidentWorld,
  centerChunkX: number,
  centerChunkZ: number,
  radiusChunks: number,
): number {
  let missingColumns = 0;
  for (let dz = -radiusChunks; dz <= radiusChunks; dz += 1) {
    for (let dx = -radiusChunks; dx <= radiusChunks; dx += 1) {
      if (dx * dx + dz * dz > radiusChunks * radiusChunks) {
        continue;
      }
      if (!world.hasResidentColumn(centerChunkX + dx, centerChunkZ + dz)) {
        missingColumns += 1;
      }
    }
  }
  return missingColumns;
}

function countUrgentDirtyMeshlessChunks(
  world: ProceduralResidentWorld,
  priorityChunkX: number,
  priorityChunkY: number,
  priorityChunkZ: number,
): number {
  let urgentDirtyMeshlessChunks = 0;
  for (const chunk of world.iterateResidentChunks()) {
    if (!chunk.meshDirty || chunk.mesh) {
      continue;
    }
    if (shouldSyncBuildUrgentChunk(chunk, priorityChunkX, priorityChunkY, priorityChunkZ)) {
      urgentDirtyMeshlessChunks += 1;
    }
  }
  return urgentDirtyMeshlessChunks;
}

function sumNumbers(values: readonly number[]): number {
  let total = 0;
  for (const value of values) {
    total += value;
  }
  return total;
}
