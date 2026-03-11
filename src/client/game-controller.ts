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
import {
  diffChunkCoords,
  summarizeResidentWorld,
  type ResidentWorldProbeSnapshot,
} from "../engine/procedural-probes.ts";
import { ProceduralResidentWorld, type ResidencyUpdateSummary } from "../engine/procedural-resident-world.ts";
import { ProceduralWorldGenerator } from "../engine/procedural-generator.ts";
import { WebGpuVoxelRenderer, type RenderStats } from "../engine/renderer.ts";
import type { Vec3 } from "../engine/types.ts";
import type { MeshBuildSummary } from "../engine/mesher.ts";

const MAX_DELTA_SECONDS = 0.05;
const HUD_PUSH_INTERVAL_MS = 120;

export interface GameHudSnapshot {
  status: string;
  pointerLocked: boolean;
  position: Vec3;
  feetPosition: Vec3;
  playerChunk: [number, number, number];
  grounded: boolean;
  yawDegrees: number;
  pitchDegrees: number;
  solidVoxelCount: number;
  chunkCount: number;
  paletteCount: number;
  streamMs: number;
  streamGeneratedChunks: number;
  streamEvictedChunks: number;
  streamEmptyChunksSkipped: number;
  streamCachedEmptyChunkHits: number;
  streamDirtyResidentChunks: number;
  residencyRadiusChunks: number;
  surfaceY: number;
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

export class GameController {
  readonly canvas: HTMLCanvasElement;
  readonly world = new ProceduralResidentWorld(new ProceduralWorldGenerator(1337));

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
      this.updateMovement(deltaSeconds);
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
      grounded: this.player.grounded,
      yawDegrees: toDegrees(this.camera.yaw),
      pitchDegrees: toDegrees(this.camera.pitch),
      solidVoxelCount: stats.solidVoxelCount,
      chunkCount: stats.chunkCount,
      paletteCount: stats.paletteCount,
      streamMs: this.world.lastResidency.elapsedMs,
      streamGeneratedChunks: this.world.lastResidency.generatedChunks,
      streamEvictedChunks: this.world.lastResidency.evictedChunks,
      streamEmptyChunksSkipped: this.world.lastResidency.emptyChunksSkipped,
      streamCachedEmptyChunkHits: this.world.lastResidency.cachedEmptyChunkHits,
      streamDirtyResidentChunks: this.world.lastResidency.dirtyResidentChunks,
      residencyRadiusChunks: this.world.lastResidency.radiusChunks,
      surfaceY: this.world.lastResidency.surfaceY,
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
    };
  }

  teleport(position: Vec3): void {
    teleportPlayerToEyePosition(this.player, position);
    this.syncCameraToPlayer();
    this.syncWorldAroundPlayer();
    this.pushHud(true);
  }

  setResidencyRadiusChunks(radius: number): void {
    this.world.setHorizontalRadiusChunks(radius);
    this.syncWorldAroundPlayer();
    this.pushHud(true);
  }

  forceResidencyUpdate(): ResidencyUpdateSummary {
    this.world.setHorizontalRadiusChunks(this.world.horizontalRadiusChunks);
    this.syncWorldAroundPlayer();
    this.pushHud(true);
    return this.world.lastResidency;
  }

  snapshotResidentWorld(): ResidentWorldProbeSnapshot {
    return summarizeResidentWorld(this.world);
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
    this.syncWorldAroundPlayer();
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
      generatedChunkCoords: this.world.lastResidency.generatedChunkCoords.map(toChunkTuple),
      residency: {
        ...this.world.lastResidency,
        generatedChunkCoords: this.world.lastResidency.generatedChunkCoords.map((coord) => ({ ...coord })),
        evictedChunkCoords: this.world.lastResidency.evictedChunkCoords.map((coord) => ({ ...coord })),
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
      this.syncWorldAroundPlayer();
      await this.renderProbeFrame();
      this.pushHud(true);
      this.start();
    }
  }

  private loadBootstrapWorld(): void {
    const spawn = this.world.getSpawnPosition();
    this.player = createPlayerState(spawn, { grounded: true });
    this.camera = createFirstPersonCamera(getPlayerEyePosition(this.player), 0.8, -0.32);
    this.syncWorldAroundPlayer();
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

  private updateMovement(deltaSeconds: number): void {
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
    if (result.moved) {
      this.syncWorldAroundPlayer();
    }
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

  private syncWorldAroundPlayer(): void {
    const residency = this.world.updateResidencyAround(this.player.feetPosition);
    if (!residency.changed) {
      return;
    }
    const meshSummary = rebuildDirtyMeshes(this.world);
    this.lastMeshBuildSummary = meshSummary;
    this.meshMs = meshSummary.elapsedMs;
    this.status = residency.generatedChunks > 0 || residency.evictedChunks > 0
      ? `Streamed ${residency.generatedChunks} chunk(s), evicted ${residency.evictedChunks}`
      : "Residency updated";
  }

  private isPressed(...codes: string[]): boolean {
    return codes.some((code) => this.pressedKeys.has(code));
  }

  private syncCameraToPlayer(): void {
    this.camera.position = getPlayerEyePosition(this.player);
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
    const frameStats = this.renderer.render(this.world, cameraMatrices);
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
