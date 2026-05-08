import { buildCameraMatrices, orbitCamera, type CameraState } from "../engine/camera.ts";
import { FrameTimingBuckets, type FrameTimingSnapshot } from "../engine/frame-timing-buckets.ts";
import {
  createLodDebugWorld,
  type LodDebugCoverageRequest,
} from "../engine/lod-debug-world.ts";
import { degToRad } from "../engine/math.ts";
import { WebGpuVoxelRenderer } from "../engine/renderer.ts";

interface LodLabSnapshot {
  frame: number;
  fps: number;
  activeChunks: number;
  pendingChunks: number;
  gaps: number;
  overlaps: number;
  drawCalls: number;
  triangles: number;
  timing: FrameTimingSnapshot;
}

const canvas = document.querySelector<HTMLCanvasElement>('[data-role="viewport"]');
const hud = document.querySelector<HTMLElement>('[data-role="hud"]');

if (!canvas || !hud) {
  throw new Error("LOD lab page is missing required elements");
}
const viewportCanvas = canvas;
const hudElement = hud;

const world = createLodDebugWorld();
let visibleRequest: LodDebugCoverageRequest = {
  centerChunkX: 0,
  centerChunkZ: 0,
  nearRadiusLod1Chunks: 1,
  farRadiusLod1Chunks: 5,
};
let stagedRequest: LodDebugCoverageRequest | null = null;
const camera: CameraState = {
  target: [0, 10, 0],
  yaw: degToRad(45),
  pitch: degToRad(-42),
  distance: 180,
  zoom: 96,
};

let renderer: WebGpuVoxelRenderer | null = null;
let frame = 0;
let lastTimestamp = performance.now();
let fps = 0;
let animationId = 0;
let nextSweepFrame = 0;
const frameTimings = new FrameTimingBuckets(125, 50, 96);

void boot();

async function boot(): Promise<void> {
  renderer = await WebGpuVoxelRenderer.create(viewportCanvas);
  window.__VOXELS_LOD_LAB__ = {
    snapshot: () => buildSnapshot(0, 0),
    applyEdit: () => {
      world.setEdit(64, 6, 64);
      return buildSnapshot(0, 0);
    },
    requestCoverage: (centerChunkX: number, centerChunkZ: number) => {
      visibleRequest = { ...visibleRequest, centerChunkX, centerChunkZ };
      stagedRequest = null;
      world.requestCoverage(visibleRequest);
      return buildSnapshot(0, 0);
    },
    completePending: (limit?: number) => world.completePending(limit),
  };
  viewportCanvas.addEventListener("pointerdown", handlePointerDown);
  animationId = requestAnimationFrame(tick);
}

function tick(timestamp: number): void {
  if (!renderer) {
    return;
  }
  const delta = Math.max(1, timestamp - lastTimestamp);
  const timing = frameTimings.record(timestamp);
  lastTimestamp = timestamp;
  fps = fps * 0.92 + (1000 / delta) * 0.08;
  frame += 1;

  const reportBeforePlan = world.reportCoverage(visibleRequest);
  if (!stagedRequest && reportBeforePlan.pendingChunks === 0 && frame >= nextSweepFrame) {
    const sweep = Math.sin(frame * 0.018) * 4;
    stagedRequest = {
      ...visibleRequest,
      centerChunkX: Math.floor(sweep),
      centerChunkZ: Math.floor(Math.cos(frame * 0.014) * 3),
    };
    world.prepareCoverage(stagedRequest);
    nextSweepFrame = frame + 90;
  }
  world.completePending(2);
  if (stagedRequest && world.reportCoverage(stagedRequest).pendingChunks === 0) {
    visibleRequest = stagedRequest;
    stagedRequest = null;
    world.requestCoverage(visibleRequest);
  }
  orbitCamera(camera, 0.0025, 0);

  const aspect = Math.max(1, viewportCanvas.width) / Math.max(1, viewportCanvas.height);
  const matrices = buildCameraMatrices(camera, aspect);
  const stats = renderer.render(world, {
    viewProjection: matrices.viewProjection,
    position: matrices.eye,
  });
  const snapshot = buildSnapshot(stats.drawCalls, stats.triangles);
  hudElement.textContent = `LOD Lab | ${snapshot.fps.toFixed(1)} fps | max ${timing.worstRecentFrameMs.toFixed(0)}ms | hitches ${timing.recentHitchCount} | dropped ${timing.recentDroppedFrameEstimate} | chunks ${snapshot.activeChunks} pending ${snapshot.pendingChunks} | gaps ${snapshot.gaps} overlaps ${snapshot.overlaps} | draws ${snapshot.drawCalls}`;

  animationId = requestAnimationFrame(tick);
}

function buildSnapshot(drawCalls: number, triangles: number): LodLabSnapshot {
  const report = world.reportCoverage(visibleRequest);
  return {
    frame,
    fps,
    activeChunks: report.activeChunks,
    pendingChunks: report.pendingChunks,
    gaps: report.gaps,
    overlaps: report.overlaps,
    drawCalls,
    triangles,
    timing: frameTimings.snapshot(),
  };
}

function handlePointerDown(): void {
  world.setEdit(64, 6, 64);
}

window.addEventListener("beforeunload", () => {
  cancelAnimationFrame(animationId);
  renderer?.dispose();
});

declare global {
  interface Window {
    __VOXELS_LOD_LAB__?: {
      snapshot(): LodLabSnapshot;
      applyEdit(): LodLabSnapshot;
      requestCoverage(centerChunkX: number, centerChunkZ: number): LodLabSnapshot;
      completePending(limit?: number): number;
    };
  }
}
