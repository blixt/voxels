import { average, maxValue, percentile } from "../engine/benchmark-metrics.ts";
import {
  buildFirstPersonCameraMatrices,
  createFirstPersonCamera,
  rotateFirstPersonCamera,
  type FirstPersonCameraState,
} from "../engine/first-person-camera.ts";
import { rebuildDirtyMeshes } from "../engine/mesher.ts";
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
import { ProceduralResidentWorld, type ResidencyUpdateSummary } from "../engine/procedural-resident-world.ts";
import { ProceduralWorldGenerator } from "../engine/procedural-generator.ts";
import { WebGpuVoxelRenderer, type RenderStats } from "../engine/renderer.ts";
import { metersToWorldUnits, worldUnitsToMeters } from "../engine/scale.ts";
import { shouldPumpWorldWork, shouldRefreshResidency } from "../engine/stream-work.ts";
import {
  buildStreamAnchorPosition,
  resolveStreamAnchor,
  type StreamAnchor,
} from "../engine/stream-anchor.ts";
import type { Vec3 } from "../engine/types.ts";
import type { MeshBuildSummary } from "../engine/mesher.ts";

const MAX_DELTA_SECONDS = 0.05;
const HUD_PUSH_INTERVAL_MS = 120;
const STREAM_ANCHOR_MARGIN_CHUNKS = 1;
const DEFAULT_MAX_GENERATED_CHUNKS_PER_UPDATE = 6;
const DEFAULT_MAX_MESH_REBUILDS_PER_FRAME = 4;

export interface GameHudSnapshot {
  status: string;
  pointerLocked: boolean;
  position: Vec3;
  feetPosition: Vec3;
  playerChunk: [number, number, number];
  streamAnchorChunk: [number, number];
  grounded: boolean;
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
  streamDirtyResidentChunks: number;
  residencyRadiusChunks: number;
  surfaceY: number;
  farFieldMs: number;
  farFieldBuiltBands: number;
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
  coveredSampleCount: number;
  residentOverlapCount: number;
  uncoveredGapCount: number;
  bandOverlapCount: number;
  wrongBandCount: number;
  residentOverlapSamples: LodCoverageIssueSample[];
  uncoveredGapSamples: LodCoverageIssueSample[];
  bandOverlapSamples: LodCoverageIssueSample[];
  wrongBandSamples: LodCoverageIssueSample[];
}

export class GameController {
  readonly canvas: HTMLCanvasElement;
  readonly generator = new ProceduralWorldGenerator(1337);
  readonly world = new ProceduralResidentWorld(this.generator);
  readonly farField = new ProceduralFarField(this.generator);

  renderer: WebGpuVoxelRenderer | null = null;
  camera: FirstPersonCameraState = createFirstPersonCamera([0.5, 1500, 0.5]);
  player: PlayerState = createPlayerState([0.5, 1500 - PLAYER_EYE_HEIGHT, 0.5]);
  meshMs = 0;
  drawCalls = 0;
  triangles = 0;
  lastFrameCpuMs = 0;
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
  private streamingBudgets: StreamingBudgets = {
    maxGeneratedChunksPerUpdate: DEFAULT_MAX_GENERATED_CHUNKS_PER_UPDATE,
    maxMeshRebuildsPerFrame: DEFAULT_MAX_MESH_REBUILDS_PER_FRAME,
  };

  private rafId = 0;
  private lastFrameTime = 0;
  private lastHudPushAt = 0;
  private readonly pressedKeys = new Set<string>();

  constructor(canvas: HTMLCanvasElement) {
    this.canvas = canvas;
  }

  async init(): Promise<void> {
    this.renderer = await WebGpuVoxelRenderer.create(this.canvas);
    this.loadBootstrapWorld();
    this.attachInteractions();
    this.start();
  }

  dispose(): void {
    cancelAnimationFrame(this.rafId);
    if (document.pointerLockElement === this.canvas) {
      document.exitPointerLock();
    }
    this.pointerLocked = false;
    this.pressedKeys.clear();
    this.renderer?.dispose();
    this.canvas.removeEventListener("click", this.handleCanvasClick);
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
      const deltaSeconds = this.lastFrameTime === 0
        ? 1 / 60
        : Math.min((now - this.lastFrameTime) / 1000, MAX_DELTA_SECONDS);
      this.lastFrameTime = now;
      const moved = this.updateMovement(deltaSeconds);
      if (shouldPumpWorldWork(moved, this.lastStreamSummary.pendingChunks, this.world.countDirtyResidentChunks())) {
        this.syncWorldAroundPlayer();
      }
      this.renderInteractiveFrame();
      this.rafId = requestAnimationFrame(tick);
    };
    this.rafId = requestAnimationFrame(tick);
  }

  stop(): void {
    cancelAnimationFrame(this.rafId);
  }

  getDebugSnapshot(): GameHudSnapshot {
    const stats = this.world.getStats();
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
      streamDirtyResidentChunks: this.world.countDirtyResidentChunks(),
      residencyRadiusChunks: this.lastStreamSummary.radiusChunks,
      surfaceY: this.lastStreamSummary.surfaceY,
      farFieldMs: this.lastFarFieldSummary.elapsedMs,
      farFieldBuiltBands: this.lastFarFieldSummary.builtBands,
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
    };
  }

  getStreamingBudgets(): StreamingBudgets {
    return { ...this.streamingBudgets };
  }

  setStreamingBudgets(
    maxGeneratedChunksPerUpdate: number,
    maxMeshRebuildsPerFrame: number,
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

  probeLodCoverage(sampleRadiusMeters = 48, sampleStepMeters = 0.8): LodCoverageProbe {
    const normalizedRadius = Math.max(1, sampleRadiusMeters);
    const normalizedStep = Math.max(0.1, sampleStepMeters);
    const sampleRadiusWorldUnits = metersToWorldUnits(normalizedRadius);
    const sampleStepWorldUnits = metersToWorldUnits(normalizedStep);
    const centerX = this.player.feetPosition[0];
    const centerZ = this.player.feetPosition[2];
    const maxRadiusWorldUnits = this.lastFarFieldSummary.maxRadiusWorldUnits;
    const exclusionMask = this.world.getFarFieldExclusionMask();
    const residentOverlapSamples: LodCoverageIssueSample[] = [];
    const uncoveredGapSamples: LodCoverageIssueSample[] = [];
    const bandOverlapSamples: LodCoverageIssueSample[] = [];
    const wrongBandSamples: LodCoverageIssueSample[] = [];
    let sampleCount = 0;
    let residentSampleCount = 0;
    let coveredSampleCount = 0;
    let residentOverlapCount = 0;
    let uncoveredGapCount = 0;
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
        const coverage = this.farField.getCoverageAt(worldX, worldZ, exclusionMask);
        const distanceMeters = worldUnitsToMeters(distanceWorldUnits);
        const issueSample = coverageToIssueSample(worldX, worldZ, distanceMeters, coverage);
        sampleCount += 1;
        if (resident) {
          residentSampleCount += 1;
        }
        if (coverage.length > 0) {
          coveredSampleCount += 1;
        }
        if (resident && coverage.length > 0) {
          residentOverlapCount += 1;
          pushIssueSample(residentOverlapSamples, issueSample);
        }
        if (!resident && coverage.length === 0) {
          uncoveredGapCount += 1;
          pushIssueSample(uncoveredGapSamples, issueSample);
        }
        if (coverage.length > 1) {
          bandOverlapCount += 1;
          pushIssueSample(bandOverlapSamples, issueSample);
        }
        if (coverage.some((band) => distanceWorldUnits < band.innerRadiusWorldUnits || distanceWorldUnits >= band.outerRadiusWorldUnits)) {
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
      coveredSampleCount,
      residentOverlapCount,
      uncoveredGapCount,
      bandOverlapCount,
      wrongBandCount,
      residentOverlapSamples,
      uncoveredGapSamples,
      bandOverlapSamples,
      wrongBandSamples,
    };
  }

  async teleportAndSettle(
    position: Vec3,
    options: {
      radiusChunks?: number;
    } = {},
  ): Promise<ResidencyTransitionProbe> {
    const before = this.snapshotResidentWorld();
    if (options.radiusChunks !== undefined) {
      this.world.setHorizontalRadiusChunks(options.radiusChunks);
    }
    teleportPlayerToEyePosition(this.player, position);
    this.syncCameraToPlayer();
    const residency = this.syncWorldAroundPlayer(true);
    const render = await this.renderProbeFrame();
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
            const residency = this.syncWorldAroundPlayer(false);
            const render = await this.renderProbeFrame();
            frame += 1;
            samples.push(buildIncrementalSample(frame, "move", legIndex, residency, this.lastMeshBuildSummary, render));
          }
          for (let settleFrame = 0; settleFrame < normalizedSettleFrames; settleFrame += 1) {
            const residency = this.syncWorldAroundPlayer(false);
            const render = await this.renderProbeFrame();
            frame += 1;
            samples.push(buildIncrementalSample(frame, "settle", legIndex, residency, this.lastMeshBuildSummary, render));
            if (residency.complete && residency.pendingChunks === 0 && this.lastMeshBuildSummary.meshCount === 0) {
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

  private loadBootstrapWorld(): void {
    const spawn = this.world.getSpawnPosition();
    this.player = createPlayerState(spawn, { grounded: true });
    this.camera = createFirstPersonCamera(getPlayerEyePosition(this.player), 0.8, -0.32);
    this.streamAnchor = null;
    this.syncWorldAroundPlayer(true);
    this.status = "Click once to capture cursor";
    this.pushHud(true);
  }

  private attachInteractions(): void {
    this.canvas.addEventListener("click", this.handleCanvasClick);
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

  private syncWorldAroundPlayer(force = false): ResidencyUpdateSummary {
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
      this.lastFarFieldSummary = this.farField.updateAround(
        this.player.feetPosition,
        0,
        this.world.getFarFieldExclusionMask(),
      );
      this.flushMeshBuildBudget();
      this.lastStreamSummary = createIdleResidencySummary(
        this.lastStreamSummary,
        this.streamAnchor ?? resolved.anchor,
        this.world.horizontalRadiusChunks,
        this.world.getStats().chunkCount,
        this.world.countDirtyResidentChunks(),
      );
      return cloneResidencySummary(this.lastStreamSummary);
    }
    return this.syncWorldAroundAnchor(resolved.anchor);
  }

  private syncWorldAroundAnchor(anchor: StreamAnchor, settle = false): ResidencyUpdateSummary {
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
    this.lastFarFieldSummary = this.farField.updateAround(
      this.player.feetPosition,
      0,
      this.world.getFarFieldExclusionMask(),
    );
    this.flushMeshBuildBudget(
      settle ? Number.POSITIVE_INFINITY : this.streamingBudgets.maxMeshRebuildsPerFrame,
    );
    this.status = residency.pendingChunks > 0
      ? `Streaming ${residency.pendingChunks} pending chunk(s)`
      : residency.generatedChunks > 0 || residency.evictedChunks > 0
      ? `Streamed ${residency.generatedChunks} chunk(s), evicted ${residency.evictedChunks}`
      : "Residency updated";
    return cloneResidencySummary(this.lastStreamSummary);
  }

  private isPressed(...codes: string[]): boolean {
    return codes.some((code) => this.pressedKeys.has(code));
  }

  private syncCameraToPlayer(): void {
    this.camera.position = getPlayerEyePosition(this.player);
  }

  private flushMeshBuildBudget(maxChunks = DEFAULT_MAX_MESH_REBUILDS_PER_FRAME): void {
    const meshSummary = rebuildDirtyMeshes(this.world, maxChunks);
    this.lastMeshBuildSummary = meshSummary;
    this.meshMs = meshSummary.elapsedMs;
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
    const cpuStartedAt = performance.now();
    const frameStats = this.renderer.render(this.world, cameraMatrices, null, 0, this.farField.getRenderables());
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

  private pushHud(force = false): void {
    const now = performance.now();
    if (!force && now - this.lastHudPushAt < HUD_PUSH_INTERVAL_MS) {
      return;
    }
    this.lastHudPushAt = now;
    this.onHudUpdate?.(this.getDebugSnapshot());
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
  const workSamples = samples.map((sample) => sample.streamMs + sample.meshMs + sample.frameCpuMs);
  const streamSamples = samples.map((sample) => sample.streamMs);
  const meshSamples = samples.map((sample) => sample.meshMs);
  const frameCpuSamples = samples.map((sample) => sample.frameCpuMs);
  const uploadSamples = samples.map((sample) => sample.uploadMs);
  return {
    sampleCount: samples.length,
    workFrameCount: samples.filter((sample) => sample.streamMs > 0 || sample.meshMs > 0 || sample.uploadChunks > 0).length,
    changedCount: samples.filter((sample) => sample.changed).length,
    incompleteFrameCount: samples.filter((sample) => !sample.complete || sample.pendingChunks > 0).length,
    avgWorkMs: average(workSamples),
    p95WorkMs: percentile(workSamples, 0.95),
    maxWorkMs: maxValue(workSamples),
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

function zeroResidencyPhaseMetrics(): ResidencyUpdateSummary["phaseMs"] {
  return {
    surfaceSampleMs: 0,
    yRangeMs: 0,
    chunkGenerationMs: 0,
    chunkAdoptionMs: 0,
    evictionMs: 0,
    neighborDirtyMs: 0,
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
