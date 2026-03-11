import {
  buildFirstPersonCameraMatrices,
  createFirstPersonCamera,
  moveFirstPersonCamera,
  rotateFirstPersonCamera,
  type FirstPersonCameraState,
} from "../engine/first-person-camera.ts";
import { rebuildDirtyMeshes } from "../engine/mesher.ts";
import {
  diffChunkCoords,
  summarizeResidentWorld,
  type ResidentWorldProbeSnapshot,
} from "../engine/procedural-probes.ts";
import { ProceduralResidentWorld, type ResidencyUpdateSummary } from "../engine/procedural-resident-world.ts";
import { ProceduralWorldGenerator } from "../engine/procedural-generator.ts";
import { WebGpuVoxelRenderer } from "../engine/renderer.ts";
import type { Vec3 } from "../engine/types.ts";
import type { MeshBuildSummary } from "../engine/mesher.ts";

const BASE_MOVE_SPEED = 30;
const FAST_MOVE_MULTIPLIER = 3;
const SLOW_MOVE_MULTIPLIER = 0.35;
const MAX_DELTA_SECONDS = 0.05;
const HUD_PUSH_INTERVAL_MS = 120;

export interface GameHudSnapshot {
  status: string;
  pointerLocked: boolean;
  position: Vec3;
  playerChunk: [number, number, number];
  yawDegrees: number;
  pitchDegrees: number;
  solidVoxelCount: number;
  chunkCount: number;
  paletteCount: number;
  streamMs: number;
  streamGeneratedChunks: number;
  streamEvictedChunks: number;
  streamEmptyChunksSkipped: number;
  streamDirtyResidentChunks: number;
  residencyRadiusChunks: number;
  surfaceY: number;
  meshMs: number;
  meshNewChunks: number;
  meshRemeshChunks: number;
  drawCalls: number;
  triangles: number;
  avgFrameCpuMs: number;
}

export interface ResidencyTransitionProbe {
  before: ResidentWorldProbeSnapshot;
  after: ResidentWorldProbeSnapshot;
  enteredChunkCoords: Array<[number, number, number]>;
  evictedChunkCoords: Array<[number, number, number]>;
  generatedChunkCoords: Array<[number, number, number]>;
  residency: ResidencyUpdateSummary;
  mesh: MeshBuildSummary;
}

export class GameController {
  readonly canvas: HTMLCanvasElement;
  readonly world = new ProceduralResidentWorld(new ProceduralWorldGenerator(1337));

  renderer: WebGpuVoxelRenderer | null = null;
  camera: FirstPersonCameraState = createFirstPersonCamera([0.5, 1500, 0.5]);
  meshMs = 0;
  drawCalls = 0;
  triangles = 0;
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
      playerChunk: [
        Math.floor(this.camera.position[0] / this.world.chunkSize),
        Math.floor(this.camera.position[1] / this.world.chunkSize),
        Math.floor(this.camera.position[2] / this.world.chunkSize),
      ],
      yawDegrees: toDegrees(this.camera.yaw),
      pitchDegrees: toDegrees(this.camera.pitch),
      solidVoxelCount: stats.solidVoxelCount,
      chunkCount: stats.chunkCount,
      paletteCount: stats.paletteCount,
      streamMs: this.world.lastResidency.elapsedMs,
      streamGeneratedChunks: this.world.lastResidency.generatedChunks,
      streamEvictedChunks: this.world.lastResidency.evictedChunks,
      streamEmptyChunksSkipped: this.world.lastResidency.emptyChunksSkipped,
      streamDirtyResidentChunks: this.world.lastResidency.dirtyResidentChunks,
      residencyRadiusChunks: this.world.lastResidency.radiusChunks,
      surfaceY: this.world.lastResidency.surfaceY,
      meshMs: this.meshMs,
      meshNewChunks: this.lastMeshBuildSummary.newMeshCount,
      meshRemeshChunks: this.lastMeshBuildSummary.remeshCount,
      drawCalls: this.drawCalls,
      triangles: this.triangles,
      avgFrameCpuMs: this.avgFrameCpuMs,
    };
  }

  teleport(position: Vec3): void {
    this.camera.position = [...position];
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
    this.camera.position = [...position];
    this.syncWorldAroundPlayer();
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
    };
  }

  private loadBootstrapWorld(): void {
    const spawn = this.world.getSpawnPosition();
    this.camera = createFirstPersonCamera(spawn, 0.8, -0.32);
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
      ? "Pointer locked: WASD move, Space rise, Shift descend, Ctrl sprint"
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
    if (!this.pointerLocked) {
      return;
    }
    const forward = (this.isPressed("KeyW") ? 1 : 0) - (this.isPressed("KeyS") ? 1 : 0);
    const strafe = (this.isPressed("KeyD") ? 1 : 0) - (this.isPressed("KeyA") ? 1 : 0);
    const vertical = (this.isPressed("Space") ? 1 : 0) - (this.isPressed("ShiftLeft", "ShiftRight") ? 1 : 0);
    if (forward === 0 && strafe === 0 && vertical === 0) {
      return;
    }
    let speed = BASE_MOVE_SPEED;
    if (this.isPressed("ControlLeft", "ControlRight")) {
      speed *= FAST_MOVE_MULTIPLIER;
    }
    if (this.isPressed("AltLeft", "AltRight")) {
      speed *= SLOW_MOVE_MULTIPLIER;
    }
    moveFirstPersonCamera(
      this.camera,
      { forward, strafe, vertical },
      deltaSeconds,
      speed,
    );
    this.syncWorldAroundPlayer();
  }

  private renderInteractiveFrame(): void {
    if (!this.renderer) {
      return;
    }
    this.renderer.configureCanvas(this.canvas);
    const aspect = this.canvas.width / this.canvas.height;
    const cameraMatrices = buildFirstPersonCameraMatrices(this.camera, aspect);
    const cpuStartedAt = performance.now();
    const frameStats = this.renderer.render(this.world, cameraMatrices);
    const frameCpuMs = performance.now() - cpuStartedAt;
    this.drawCalls = frameStats.drawCalls;
    this.triangles = frameStats.triangles;
    this.avgFrameCpuMs = this.avgFrameCpuMs === 0
      ? frameCpuMs
      : this.avgFrameCpuMs * 0.9 + frameCpuMs * 0.1;
    this.pushHud();
  }

  private syncWorldAroundPlayer(): void {
    const residency = this.world.updateResidencyAround(this.camera.position);
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

function isMovementKey(code: string): boolean {
  return code === "KeyW"
    || code === "KeyA"
    || code === "KeyS"
    || code === "KeyD"
    || code === "Space"
    || code === "ShiftLeft"
    || code === "ShiftRight"
    || code === "ControlLeft"
    || code === "ControlRight"
    || code === "AltLeft"
    || code === "AltRight";
}
