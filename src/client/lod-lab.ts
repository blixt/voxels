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
  timingStrip: string;
  worstBucketSummary: string;
  lastFrameAttribution: LodLabFrameAttribution;
  lastHitchAttribution: LodLabFrameAttribution;
}

interface LodLabFrameAttribution {
  frame: number;
  wallMs: number;
  planMs: number;
  completeMs: number;
  handoffMs: number;
  renderMs: number;
  hudMs: number;
  completedChunks: number;
  pendingChunks: number;
  cause: string;
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
const timingStripBucketCount = 48;
const maxPendingCompletionsPerFrame = 1;
let lastFrameAttribution = createZeroLodLabFrameAttribution();
let lastHitchAttribution = createZeroLodLabFrameAttribution();

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
  if (delta >= timing.hitchThresholdMs) {
    lastHitchAttribution = {
      ...lastFrameAttribution,
      wallMs: delta,
      cause: lastFrameAttribution.cause === "none" ? "browser or idle" : lastFrameAttribution.cause,
    };
  }
  lastTimestamp = timestamp;
  fps = fps * 0.92 + (1000 / delta) * 0.08;
  frame += 1;

  const planStartedAt = performance.now();
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
  const planMs = performance.now() - planStartedAt;

  const completeStartedAt = performance.now();
  const completedChunks = world.completePending(maxPendingCompletionsPerFrame);
  const completeMs = performance.now() - completeStartedAt;

  const handoffStartedAt = performance.now();
  if (stagedRequest && world.reportCoverage(stagedRequest).pendingChunks === 0) {
    visibleRequest = stagedRequest;
    stagedRequest = null;
    world.requestCoverage(visibleRequest);
  }
  const handoffMs = performance.now() - handoffStartedAt;
  orbitCamera(camera, 0.0025, 0);

  const aspect = Math.max(1, viewportCanvas.width) / Math.max(1, viewportCanvas.height);
  const matrices = buildCameraMatrices(camera, aspect);
  const renderStartedAt = performance.now();
  const stats = renderer.render(world, {
    viewProjection: matrices.viewProjection,
    position: matrices.eye,
  });
  const renderMs = performance.now() - renderStartedAt;
  const snapshot = buildSnapshot(stats.drawCalls, stats.triangles);
  const hudStartedAt = performance.now();
  hudElement.textContent = [
    `LOD Lab | ${snapshot.fps.toFixed(1)} fps | max ${timing.worstRecentFrameMs.toFixed(0)}ms | hitches ${timing.recentHitchCount} | stalled ${timing.recentStalledMs.toFixed(0)}ms | dropped ${timing.recentDroppedFrameEstimate}`,
    `8Hz buckets ${snapshot.timingStrip} | ${snapshot.worstBucketSummary}`,
    formatHitchAttribution(snapshot),
    `chunks ${snapshot.activeChunks} pending ${snapshot.pendingChunks} | gaps ${snapshot.gaps} overlaps ${snapshot.overlaps} | draws ${snapshot.drawCalls}`,
  ].join("\n");
  const hudMs = performance.now() - hudStartedAt;
  lastFrameAttribution = createLodLabFrameAttribution({
    frame,
    wallMs: delta,
    planMs,
    completeMs,
    handoffMs,
    renderMs,
    hudMs,
    completedChunks,
    pendingChunks: snapshot.pendingChunks,
  });

  animationId = requestAnimationFrame(tick);
}

function buildSnapshot(drawCalls: number, triangles: number): LodLabSnapshot {
  const report = world.reportCoverage(visibleRequest);
  const timing = frameTimings.snapshot();
  return {
    frame,
    fps,
    activeChunks: report.activeChunks,
    pendingChunks: report.pendingChunks,
    gaps: report.gaps,
    overlaps: report.overlaps,
    drawCalls,
    triangles,
    timing,
    timingStrip: formatTimingStrip(timing),
    worstBucketSummary: formatWorstBucketSummary(timing),
    lastFrameAttribution: { ...lastFrameAttribution },
    lastHitchAttribution: { ...lastHitchAttribution },
  };
}

function formatTimingStrip(timing: FrameTimingSnapshot): string {
  const buckets = [...timing.recent, timing.current].slice(-timingStripBucketCount);
  return buckets.map(formatTimingBucketGlyph).join("");
}

function formatTimingBucketGlyph(bucket: { maxFrameMs: number; stalledMs: number; hitchCount: number }): string {
  const blockedMs = Math.max(bucket.maxFrameMs, bucket.stalledMs);
  if (blockedMs >= 250) {
    return "!";
  }
  if (bucket.hitchCount > 0 || bucket.stalledMs > 0) {
    return "#";
  }
  if (blockedMs >= 34) {
    return "*";
  }
  if (blockedMs >= 20) {
    return "+";
  }
  if (blockedMs > 0) {
    return ".";
  }
  return " ";
}

function formatWorstBucketSummary(timing: FrameTimingSnapshot): string {
  const buckets = [...timing.recent, timing.current];
  let worstBucket = buckets[0];
  for (const bucket of buckets) {
    if (!worstBucket || Math.max(bucket.maxFrameMs, bucket.stalledMs) > Math.max(worstBucket.maxFrameMs, worstBucket.stalledMs)) {
      worstBucket = bucket;
    }
  }
  if (!worstBucket) {
    return "worst bucket none";
  }
  const secondsAgo = Math.max(0, (timing.current.endMs - worstBucket.endMs) / 1000);
  const blockedMs = Math.max(worstBucket.maxFrameMs, worstBucket.stalledMs);
  return `worst bucket ${blockedMs.toFixed(0)}ms ${secondsAgo.toFixed(1)}s ago (${worstBucket.hitchCount} hitch, ${worstBucket.stalledMs.toFixed(0)}ms stalled)`;
}

function formatHitchAttribution(snapshot: LodLabSnapshot): string {
  if (snapshot.timing.recentHitchCount === 0) {
    return "last hitch none in recent window";
  }
  const hitch = snapshot.lastHitchAttribution;
  return `last hitch ${hitch.cause} ${hitch.wallMs.toFixed(0)}ms | complete ${hitch.completeMs.toFixed(1)} handoff ${hitch.handoffMs.toFixed(1)} render ${hitch.renderMs.toFixed(1)}`;
}

function createZeroLodLabFrameAttribution(): LodLabFrameAttribution {
  return createLodLabFrameAttribution({
    frame: 0,
    wallMs: 0,
    planMs: 0,
    completeMs: 0,
    handoffMs: 0,
    renderMs: 0,
    hudMs: 0,
    completedChunks: 0,
    pendingChunks: 0,
  });
}

function createLodLabFrameAttribution(
  input: Omit<LodLabFrameAttribution, "cause">,
): LodLabFrameAttribution {
  const candidates: Array<[cause: string, ms: number]> = [
    ["complete", input.completeMs],
    ["handoff", input.handoffMs],
    ["render", input.renderMs],
    ["plan", input.planMs],
    ["HUD", input.hudMs],
  ];
  candidates.sort((left, right) => right[1] - left[1]);
  const [cause, ms] = candidates[0] ?? ["none", 0];
  return {
    ...input,
    cause: ms > 0.05 ? cause : "none",
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
