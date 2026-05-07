import { average, maxValue, percentile } from "./benchmark-metrics.ts";

export interface BootstrapBenchmarkSample {
  frame: number;
  elapsedMs: number;
  gameplayFrameMs: number;
  renderCpuMs: number;
  renderSyncMs: number;
  renderUploadMs: number;
  renderEncodeMs: number;
  uploadChunks: number;
  uploadBytes: number;
  drawCalls: number;
  triangles: number;
  streamMs: number;
  meshMs: number;
  lodMs: number;
  lodYRangeMs: number;
  lodDownsampleMs: number;
  lodMeshMs: number;
  pendingChunks: number;
  pendingMeshJobs: number;
  dirtyResidentChunks: number;
  dirtyMeshlessResidentChunks: number;
  dirtyRetainedMeshResidentChunks: number;
  generatedChunks: number;
  evictedChunks: number;
  playableReady: boolean;
  visualReady: boolean;
  lodChunkCount: number;
  lodPendingChunks: number;
  lodComplete: boolean;
  frustumCulledChunks: number;
  fogCulledChunks: number;
  lodDrawCalls: number;
}

export interface BootstrapBenchmarkSummary {
  sampleCount: number;
  totalElapsedMs: number;
  playableReadyElapsedMs: number | null;
  visualReadyElapsedMs: number | null;
  lodCompleteElapsedMs: number | null;
  totalGameplayFrameMs: number;
  totalStreamMs: number;
  totalMeshMs: number;
  totalLodMs: number;
  totalLodYRangeMs: number;
  totalLodDownsampleMs: number;
  totalLodMeshMs: number;
  totalRenderCpuMs: number;
  totalRenderSyncMs: number;
  totalRenderUploadMs: number;
  totalRenderEncodeMs: number;
  totalUploadChunks: number;
  totalUploadBytes: number;
  avgGameplayFrameMs: number;
  p95GameplayFrameMs: number;
  maxGameplayFrameMs: number;
  avgStreamMs: number;
  p95StreamMs: number;
  maxStreamMs: number;
  avgMeshMs: number;
  p95MeshMs: number;
  maxMeshMs: number;
  avgLodMs: number;
  p95LodMs: number;
  maxLodMs: number;
  avgRenderCpuMs: number;
  p95RenderCpuMs: number;
  maxRenderCpuMs: number;
  totalGeneratedChunks: number;
  totalEvictedChunks: number;
  maxPendingChunks: number;
  maxPendingMeshJobs: number;
  maxDirtyResidentChunks: number;
  maxDirtyMeshlessResidentChunks: number;
  maxLodChunkCount: number;
  maxLodPendingChunks: number;
  maxFrustumCulledChunks: number;
  maxFogCulledChunks: number;
  maxLodDrawCalls: number;
  framesOver16_67Ms: number;
  framesOver33_33Ms: number;
}

export function summarizeBootstrapBenchmark(
  samples: readonly BootstrapBenchmarkSample[],
): BootstrapBenchmarkSummary {
  const gameplayFrameMs = samples.map((sample) => sample.gameplayFrameMs);
  const streamMs = samples.map((sample) => sample.streamMs);
  const meshMs = samples.map((sample) => sample.meshMs);
  const lodMs = samples.map((sample) => sample.lodMs);
  const lodYRangeMs = samples.map((sample) => sample.lodYRangeMs);
  const lodDownsampleMs = samples.map((sample) => sample.lodDownsampleMs);
  const lodMeshMs = samples.map((sample) => sample.lodMeshMs);
  const renderCpuMs = samples.map((sample) => sample.renderCpuMs);
  const renderSyncMs = samples.map((sample) => sample.renderSyncMs);
  const renderUploadMs = samples.map((sample) => sample.renderUploadMs);
  const renderEncodeMs = samples.map((sample) => sample.renderEncodeMs);
  const uploadChunks = samples.map((sample) => sample.uploadChunks);
  const uploadBytes = samples.map((sample) => sample.uploadBytes);
  return {
    sampleCount: samples.length,
    totalElapsedMs: samples[samples.length - 1]?.elapsedMs ?? 0,
    playableReadyElapsedMs: firstElapsed(samples, "playableReady"),
    visualReadyElapsedMs: firstElapsed(samples, "visualReady"),
    totalGameplayFrameMs: sumNumbers(gameplayFrameMs),
    totalStreamMs: sumNumbers(streamMs),
    totalMeshMs: sumNumbers(meshMs),
    totalLodMs: sumNumbers(lodMs),
    totalLodYRangeMs: sumNumbers(lodYRangeMs),
    totalLodDownsampleMs: sumNumbers(lodDownsampleMs),
    totalLodMeshMs: sumNumbers(lodMeshMs),
    totalRenderCpuMs: sumNumbers(renderCpuMs),
    totalRenderSyncMs: sumNumbers(renderSyncMs),
    totalRenderUploadMs: sumNumbers(renderUploadMs),
    totalRenderEncodeMs: sumNumbers(renderEncodeMs),
    totalUploadChunks: sumNumbers(uploadChunks),
    totalUploadBytes: sumNumbers(uploadBytes),
    avgGameplayFrameMs: average(gameplayFrameMs),
    p95GameplayFrameMs: percentile(gameplayFrameMs, 0.95),
    maxGameplayFrameMs: maxValue(gameplayFrameMs),
    avgStreamMs: average(streamMs),
    p95StreamMs: percentile(streamMs, 0.95),
    maxStreamMs: maxValue(streamMs),
    avgMeshMs: average(meshMs),
    p95MeshMs: percentile(meshMs, 0.95),
    maxMeshMs: maxValue(meshMs),
    avgLodMs: average(lodMs),
    p95LodMs: percentile(lodMs, 0.95),
    maxLodMs: maxValue(lodMs),
    avgRenderCpuMs: average(renderCpuMs),
    p95RenderCpuMs: percentile(renderCpuMs, 0.95),
    maxRenderCpuMs: maxValue(renderCpuMs),
    lodCompleteElapsedMs: firstElapsed(samples, "lodComplete"),
    totalGeneratedChunks: samples.reduce((sum, sample) => sum + sample.generatedChunks, 0),
    totalEvictedChunks: samples.reduce((sum, sample) => sum + sample.evictedChunks, 0),
    maxPendingChunks: maxValue(samples.map((sample) => sample.pendingChunks)),
    maxPendingMeshJobs: maxValue(samples.map((sample) => sample.pendingMeshJobs)),
    maxDirtyResidentChunks: maxValue(samples.map((sample) => sample.dirtyResidentChunks)),
    maxDirtyMeshlessResidentChunks: maxValue(samples.map((sample) => sample.dirtyMeshlessResidentChunks)),
    maxLodChunkCount: maxValue(samples.map((sample) => sample.lodChunkCount)),
    maxLodPendingChunks: maxValue(samples.map((sample) => sample.lodPendingChunks)),
    maxFrustumCulledChunks: maxValue(samples.map((sample) => sample.frustumCulledChunks)),
    maxFogCulledChunks: maxValue(samples.map((sample) => sample.fogCulledChunks)),
    maxLodDrawCalls: maxValue(samples.map((sample) => sample.lodDrawCalls)),
    framesOver16_67Ms: samples.filter((sample) => sample.gameplayFrameMs > 16.67).length,
    framesOver33_33Ms: samples.filter((sample) => sample.gameplayFrameMs > 33.33).length,
  };
}

function sumNumbers(values: readonly number[]): number {
  return values.reduce((sum, value) => sum + value, 0);
}

function firstElapsed(
  samples: readonly BootstrapBenchmarkSample[],
  key: "playableReady" | "visualReady" | "lodComplete",
): number | null {
  for (const sample of samples) {
    if (sample[key]) {
      return sample.elapsedMs;
    }
  }
  return null;
}
