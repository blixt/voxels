import {
  buildCameraMatrices,
  createDefaultCamera,
  createScreenRay,
  orbitDeltaFromDrag,
  orbitCamera,
  panCamera,
  raycastWorld,
  zoomCamera,
  type CameraState,
} from "../engine/camera.ts";
import { rebuildDirtyMeshes } from "../engine/mesher.ts";
import { createDefaultScene, getSceneDefinitions } from "../engine/scenes.ts";
import { decodeWorld, encodeWorld, hashWorld } from "../engine/scene-format.ts";
import { compareRenderedImages, renderReferenceImage } from "../engine/reference-render.ts";
import { VALIDATION_RENDER_SIZE } from "../engine/render-constants.ts";
import { WebGpuVoxelRenderer } from "../engine/renderer.ts";
import type {
  RenderValidationArtifacts,
  SceneBenchmarkSample,
  SceneCameraPreset,
} from "../engine/types.ts";
import { VoxelWorld } from "../engine/world.ts";
import { importMagicaVoxel } from "../engine/vox-format.ts";

const sceneDefinitions = getSceneDefinitions();
const sceneById = new Map(sceneDefinitions.map((definition) => [definition.id, definition]));
const stressSceneDefinitions = sceneDefinitions.filter((definition) => definition.stress);

export interface HudSnapshot {
  sceneName: string;
  solidVoxelCount: number;
  chunkCount: number;
  paletteCount: number;
  buildMs: number;
  meshMs: number;
  drawCalls: number;
  triangles: number;
  avgFrameCpuMs: number;
  status: string;
}

export class EngineController {
  readonly canvas: HTMLCanvasElement;
  renderer: WebGpuVoxelRenderer | null = null;
  world: VoxelWorld = createDefaultScene().world;
  camera: CameraState = createDefaultCamera(this.world);
  sceneName = "terrain256";
  buildMs = 0;
  meshMs = 0;
  drawCalls = 0;
  triangles = 0;
  avgFrameCpuMs = 0;
  status = "Booting";
  onHudUpdate: ((snapshot: HudSnapshot) => void) | null = null;
  lastValidationArtifacts: RenderValidationArtifacts | null = null;
  private rafId = 0;
  private pointerState:
    | {
        button: number;
        x: number;
        y: number;
      }
    | null = null;

  constructor(canvas: HTMLCanvasElement) {
    this.canvas = canvas;
  }

  static getSceneDefinitions() {
    return sceneDefinitions;
  }

  async init(initialSceneId = "terrain256"): Promise<void> {
    this.renderer = await WebGpuVoxelRenderer.create(this.canvas);
    this.loadScene(initialSceneId);
    this.attachInteractions();
    this.start();
  }

  dispose(): void {
    cancelAnimationFrame(this.rafId);
    this.renderer?.dispose();
  }

  start(): void {
    cancelAnimationFrame(this.rafId);
    const tick = () => {
      this.renderInteractiveFrame();
      this.rafId = requestAnimationFrame(tick);
    };
    this.rafId = requestAnimationFrame(tick);
  }

  stop(): void {
    cancelAnimationFrame(this.rafId);
  }

  loadScene(sceneId: string): void {
    const scene = sceneById.get(sceneId) ?? sceneById.get("terrain256");
    if (!scene) {
      throw new Error(`Unknown scene "${sceneId}"`);
    }
    const startedAt = performance.now();
    const build = scene.build();
    this.world = build.world;
    this.sceneName = build.name;
    this.camera = applyCameraPreset(this.world, build.camera);
    this.buildMs = performance.now() - startedAt;
    const meshSummary = rebuildDirtyMeshes(this.world);
    this.meshMs = meshSummary.elapsedMs;
    this.status = build.notes?.join(" | ") ?? "Scene ready";
    this.pushHud();
  }

  loadWorld(world: VoxelWorld, sceneName = "custom"): void {
    this.world = world;
    this.sceneName = sceneName;
    this.camera = createDefaultCamera(world);
    const meshSummary = rebuildDirtyMeshes(this.world);
    this.buildMs = 0;
    this.meshMs = meshSummary.elapsedMs;
    this.status = "Custom scene loaded";
    this.pushHud();
  }

  async importSceneFile(file: File): Promise<string[]> {
    const bytes = new Uint8Array(await file.arrayBuffer());
    if (file.name.toLowerCase().endsWith(".vox")) {
      const result = importMagicaVoxel(bytes);
      this.loadWorld(result.world, file.name);
      this.status = result.warnings.length > 0 ? result.warnings.join(" | ") : "Imported VOX scene";
      this.pushHud();
      return result.warnings;
    }
    const world = decodeWorld(bytes);
    this.loadWorld(world, file.name);
    return [];
  }

  exportScene(): Blob {
    const bytes = encodeWorld(this.world);
    const copy = new Uint8Array(bytes.byteLength);
    copy.set(bytes);
    return new Blob([copy], {
      type: "application/octet-stream",
    });
  }

  editAtCanvasPoint(screenX: number, screenY: number, addVoxel: boolean): boolean {
    const aspect = this.canvas.width / this.canvas.height;
    const rect = this.canvas.getBoundingClientRect();
    const { origin, direction } = createScreenRay(
      this.camera,
      aspect,
      rect.width,
      rect.height,
      screenX,
      screenY,
      this.world,
    );
    const hit = raycastWorld(this.world, origin, direction);
    if (!hit) {
      return false;
    }
    const target = addVoxel ? hit.adjacent : hit.voxel;
    if (!this.world.inBounds(target[0], target[1], target[2])) {
      return false;
    }
    const material = addVoxel ? 9 : 0;
    const changed = this.world.setVoxel(target[0], target[1], target[2], material);
    if (!changed) {
      return false;
    }
    const meshSummary = rebuildDirtyMeshes(this.world);
    this.meshMs = meshSummary.elapsedMs;
    this.status = addVoxel ? "Placed accent voxel" : "Removed voxel";
    this.pushHud();
    return true;
  }

  async runBenchmark(sceneId: string, iterations: number, frameCount: number): Promise<SceneBenchmarkSample[]> {
    if (!this.renderer) {
      throw new Error("Renderer not initialized");
    }
    this.stop();
    const results: SceneBenchmarkSample[] = [];
    for (let iteration = 0; iteration < iterations; iteration += 1) {
      const scene = sceneById.get(sceneId);
      if (!scene) {
        throw new Error(`Unknown benchmark scene "${sceneId}"`);
      }
      const startedAt = performance.now();
      const build = scene.build();
      const buildMs = performance.now() - startedAt;
      this.world = build.world;
      this.sceneName = build.name;
      this.camera = applyCameraPreset(this.world, build.camera);
      const meshSummary = rebuildDirtyMeshes(this.world);

      const initialHash = hashWorld(this.world);
      const roundtripHash = hashWorld(decodeWorld(encodeWorld(this.world)));
      const editPass = this.runSyntheticEditCheck();
      const cpuSamples: number[] = [];
      const timer = this.renderer.createFrameTimer(frameCount);
      let drawCalls = 0;
      let triangles = 0;

      for (let frameIndex = 0; frameIndex < frameCount; frameIndex += 1) {
        await nextFrame();
        const cpuStartedAt = performance.now();
        const frameStats = this.renderer.render(this.world, this.camera, timer, frameIndex);
        cpuSamples.push(performance.now() - cpuStartedAt);
        drawCalls = frameStats.drawCalls;
        triangles = frameStats.triangles;
      }
      await this.renderer.waitForGpuIdle();
      const gpuSamples = timer ? await timer.readResults() : null;
      timer?.destroy();
      const validation = scene.kind === "validation"
        ? await this.validateCurrentFrame()
        : null;
      if (!validation) {
        this.lastValidationArtifacts = null;
      }

      const avgFrameCpuMs = average(cpuSamples);
      const avgFrameGpuMs = gpuSamples ? average(gpuSamples) : null;
      this.buildMs = buildMs;
      this.meshMs = meshSummary.elapsedMs;
      this.drawCalls = drawCalls;
      this.triangles = triangles;
      this.avgFrameCpuMs = avgFrameCpuMs;
      this.status = `Benchmarked ${sceneId}`;
      this.pushHud();

      results.push({
        sceneName: sceneId,
        sceneKind: scene.kind,
        iteration: iteration + 1,
        buildMs,
        meshMs: meshSummary.elapsedMs,
        avgFrameCpuMs,
        avgFrameGpuMs,
        triangles,
        drawCalls,
        solidVoxelCount: this.world.getStats().solidVoxelCount,
        checksum: initialHash,
        correctnessPass: initialHash === roundtripHash && editPass && (validation?.metrics.visualPass ?? true),
        visualPass: validation?.metrics.visualPass ?? null,
        meanAbsoluteError: validation?.metrics.meanAbsoluteError ?? null,
        maxAbsoluteError: validation?.metrics.maxAbsoluteError ?? null,
        coverageMismatchRatio: validation?.metrics.coverageMismatchRatio ?? null,
        renderChecksum: validation?.metrics.renderChecksum ?? null,
        referenceChecksum: validation?.metrics.referenceChecksum ?? null,
      });
    }
    this.start();
    return results;
  }

  getLastValidationArtifacts(): RenderValidationArtifacts | null {
    return this.lastValidationArtifacts;
  }

  private renderInteractiveFrame(): void {
    if (!this.renderer) {
      return;
    }
    const cpuStartedAt = performance.now();
    const frameStats = this.renderer.render(this.world, this.camera);
    this.avgFrameCpuMs = this.avgFrameCpuMs * 0.9 + (performance.now() - cpuStartedAt) * 0.1;
    this.drawCalls = frameStats.drawCalls;
    this.triangles = frameStats.triangles;
    this.pushHud();
  }

  private runSyntheticEditCheck(): boolean {
    const probeX = Math.floor(this.world.width * 0.5);
    const probeY = Math.floor(this.world.height * 0.33);
    const probeZ = Math.floor(this.world.depth * 0.5);
    const original = this.world.getVoxel(probeX, probeY, probeZ);
    const next = original === 0 ? 9 : 0;
    this.world.setVoxel(probeX, probeY, probeZ, next);
    const changed = this.world.getVoxel(probeX, probeY, probeZ) === next;
    this.world.setVoxel(probeX, probeY, probeZ, original);
    rebuildDirtyMeshes(this.world);
    return changed && this.world.getVoxel(probeX, probeY, probeZ) === original;
  }

  private async validateCurrentFrame(): Promise<{
    artifacts: RenderValidationArtifacts;
    metrics: ReturnType<typeof compareRenderedImages>["metrics"];
  }> {
    if (!this.renderer) {
      throw new Error("Renderer not initialized");
    }
    const actual = await this.renderer.captureImage(
      this.world,
      this.camera,
      VALIDATION_RENDER_SIZE,
      VALIDATION_RENDER_SIZE,
    );
    const reference = renderReferenceImage(
      this.world,
      this.camera,
      VALIDATION_RENDER_SIZE,
      VALIDATION_RENDER_SIZE,
    );
    const validation = compareRenderedImages(actual, reference);
    this.lastValidationArtifacts = validation.artifacts;
    return validation;
  }

  private attachInteractions(): void {
    this.canvas.addEventListener("pointerdown", (event) => {
      this.canvas.setPointerCapture(event.pointerId);
      this.pointerState = { button: event.button, x: event.clientX, y: event.clientY };
    });

    this.canvas.addEventListener("pointermove", (event) => {
      if (!this.pointerState) {
        return;
      }
      const deltaX = event.clientX - this.pointerState.x;
      const deltaY = event.clientY - this.pointerState.y;
      this.pointerState.x = event.clientX;
      this.pointerState.y = event.clientY;
      if (this.pointerState.button === 0 && !event.shiftKey && !event.altKey) {
        const orbitDelta = orbitDeltaFromDrag(deltaX, deltaY);
        orbitCamera(this.camera, orbitDelta.yaw, orbitDelta.pitch);
      } else if (this.pointerState.button === 1 || event.altKey) {
        const aspect = this.canvas.width / this.canvas.height;
        const matrices = buildCameraMatrices(this.camera, aspect);
        panCamera(this.camera, matrices.right, matrices.up, -deltaX * 0.14, deltaY * 0.14);
      }
    });

    this.canvas.addEventListener("pointerup", (event) => {
      if (!this.pointerState) {
        return;
      }
      const traveled = Math.hypot(event.clientX - this.pointerState.x, event.clientY - this.pointerState.y);
      const rect = this.canvas.getBoundingClientRect();
      const x = event.clientX - rect.left;
      const y = event.clientY - rect.top;
      if (traveled < 4) {
        this.editAtCanvasPoint(x, y, event.shiftKey);
      }
      this.pointerState = null;
    });

    this.canvas.addEventListener("wheel", (event) => {
      event.preventDefault();
      zoomCamera(this.camera, Math.sign(event.deltaY) * 6);
    }, { passive: false });
  }

  private pushHud(): void {
    this.onHudUpdate?.({
      sceneName: this.sceneName,
      solidVoxelCount: this.world.getStats().solidVoxelCount,
      chunkCount: this.world.getStats().chunkCount,
      paletteCount: this.world.getStats().paletteCount,
      buildMs: this.buildMs,
      meshMs: this.meshMs,
      drawCalls: this.drawCalls,
      triangles: this.triangles,
      avgFrameCpuMs: this.avgFrameCpuMs,
      status: this.status,
    });
  }
}

function average(values: number[]): number {
  if (values.length === 0) {
    return 0;
  }
  return values.reduce((sum, value) => sum + value, 0) / values.length;
}

function nextFrame(): Promise<number> {
  return new Promise((resolve) => requestAnimationFrame(resolve));
}

function applyCameraPreset(world: VoxelWorld, preset?: SceneCameraPreset): CameraState {
  if (!preset) {
    return createDefaultCamera(world);
  }
  return { ...preset };
}

export function getSceneOptions(): Array<{ id: string; label: string; description: string }> {
  return sceneDefinitions.map((definition) => ({
    id: definition.id,
    label: definition.label,
    description: definition.describe(),
  }));
}

export function getStressSceneOptions(): Array<{ id: string; label: string; description: string }> {
  return stressSceneDefinitions.map((definition) => ({
    id: definition.id,
    label: definition.label,
    description: definition.describe(),
  }));
}
