import {
  buildFirstPersonCameraMatrices,
  createFirstPersonCamera,
  moveFirstPersonCamera,
  rotateFirstPersonCamera,
  type FirstPersonCameraState,
} from "../engine/first-person-camera.ts";
import { rebuildDirtyMeshes } from "../engine/mesher.ts";
import { createDefaultScene } from "../engine/scenes.ts";
import { WebGpuVoxelRenderer } from "../engine/renderer.ts";
import type { Vec3 } from "../engine/types.ts";
import { VoxelWorld } from "../engine/world.ts";

const BASE_MOVE_SPEED = 30;
const FAST_MOVE_MULTIPLIER = 3;
const SLOW_MOVE_MULTIPLIER = 0.35;
const MAX_DELTA_SECONDS = 0.05;
const HUD_PUSH_INTERVAL_MS = 120;

export interface GameHudSnapshot {
  status: string;
  pointerLocked: boolean;
  position: Vec3;
  yawDegrees: number;
  pitchDegrees: number;
  solidVoxelCount: number;
  chunkCount: number;
  paletteCount: number;
  buildMs: number;
  meshMs: number;
  drawCalls: number;
  triangles: number;
  avgFrameCpuMs: number;
}

export class GameController {
  readonly canvas: HTMLCanvasElement;
  renderer: WebGpuVoxelRenderer | null = null;
  world: VoxelWorld = createDefaultScene().world;
  camera: FirstPersonCameraState = createFirstPersonCamera([128, 72, 128]);
  buildMs = 0;
  meshMs = 0;
  drawCalls = 0;
  triangles = 0;
  avgFrameCpuMs = 0;
  status = "Booting";
  pointerLocked = false;
  onHudUpdate: ((snapshot: GameHudSnapshot) => void) | null = null;

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
      yawDegrees: toDegrees(this.camera.yaw),
      pitchDegrees: toDegrees(this.camera.pitch),
      solidVoxelCount: stats.solidVoxelCount,
      chunkCount: stats.chunkCount,
      paletteCount: stats.paletteCount,
      buildMs: this.buildMs,
      meshMs: this.meshMs,
      drawCalls: this.drawCalls,
      triangles: this.triangles,
      avgFrameCpuMs: this.avgFrameCpuMs,
    };
  }

  teleport(position: Vec3): void {
    this.camera.position = [...position];
    this.pushHud(true);
  }

  private loadBootstrapWorld(): void {
    const startedAt = performance.now();
    const build = createDefaultScene();
    this.world = build.world;
    this.buildMs = performance.now() - startedAt;
    const meshSummary = rebuildDirtyMeshes(this.world);
    this.meshMs = meshSummary.elapsedMs;
    const spawn = findSpawnPoint(this.world);
    this.camera = createFirstPersonCamera(spawn);
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

function findSpawnPoint(world: VoxelWorld): Vec3 {
  const x = Math.floor(world.width * 0.5);
  const z = Math.floor(world.depth * 0.5);
  const surfaceY = findSurface(world, x, z);
  return [x + 0.5, surfaceY + 12, z + 0.5];
}

function findSurface(world: VoxelWorld, x: number, z: number): number {
  for (let y = world.height - 1; y >= 0; y -= 1) {
    if (world.getVoxel(x, y, z) !== 0) {
      return y;
    }
  }
  return 0;
}

function toDegrees(value: number): number {
  return value * 180 / Math.PI;
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
