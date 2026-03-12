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
  farFieldMs: number;
  farFieldPrefetchMs: number;
  farFieldPrefetchRequestedChunks: number;
  pendingChunks: number;
  pendingMeshJobs: number;
  dirtyResidentChunks: number;
  dirtyMeshlessResidentChunks: number;
  dirtyRetainedMeshResidentChunks: number;
  generatedChunks: number;
  evictedChunks: number;
  farFieldPendingBands: number;
  playableReady: boolean;
  visualReady: boolean;
}

export interface BootstrapBenchmarkSummary {
  sampleCount: number;
  totalElapsedMs: number;
  playableReadyElapsedMs: number | null;
  visualReadyElapsedMs: number | null;
  totalGameplayFrameMs: number;
  totalStreamMs: number;
  totalMeshMs: number;
  totalFarFieldMs: number;
  totalFarFieldPrefetchMs: number;
  totalFarFieldPrefetchRequestedChunks: number;
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
  avgFarFieldMs: number;
  p95FarFieldMs: number;
  maxFarFieldMs: number;
  avgFarFieldPrefetchMs: number;
  p95FarFieldPrefetchMs: number;
  maxFarFieldPrefetchMs: number;
  avgRenderCpuMs: number;
  p95RenderCpuMs: number;
  maxRenderCpuMs: number;
  totalGeneratedChunks: number;
  totalEvictedChunks: number;
  maxPendingChunks: number;
  maxPendingMeshJobs: number;
  maxDirtyResidentChunks: number;
  maxDirtyMeshlessResidentChunks: number;
  framesOver16_67Ms: number;
  framesOver33_33Ms: number;
}

export function summarizeBootstrapBenchmark(
  samples: readonly BootstrapBenchmarkSample[],
): BootstrapBenchmarkSummary {
  const gameplayFrameMs = samples.map((sample) => sample.gameplayFrameMs);
  const streamMs = samples.map((sample) => sample.streamMs);
  const meshMs = samples.map((sample) => sample.meshMs);
  const farFieldMs = samples.map((sample) => sample.farFieldMs);
  const farFieldPrefetchMs = samples.map((sample) => sample.farFieldPrefetchMs);
  const farFieldPrefetchRequestedChunks = samples.map((sample) => sample.farFieldPrefetchRequestedChunks);
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
    totalFarFieldMs: sumNumbers(farFieldMs),
    totalFarFieldPrefetchMs: sumNumbers(farFieldPrefetchMs),
    totalFarFieldPrefetchRequestedChunks: sumNumbers(farFieldPrefetchRequestedChunks),
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
    avgFarFieldMs: average(farFieldMs),
    p95FarFieldMs: percentile(farFieldMs, 0.95),
    maxFarFieldMs: maxValue(farFieldMs),
    avgFarFieldPrefetchMs: average(farFieldPrefetchMs),
    p95FarFieldPrefetchMs: percentile(farFieldPrefetchMs, 0.95),
    maxFarFieldPrefetchMs: maxValue(farFieldPrefetchMs),
    avgRenderCpuMs: average(renderCpuMs),
    p95RenderCpuMs: percentile(renderCpuMs, 0.95),
    maxRenderCpuMs: maxValue(renderCpuMs),
    totalGeneratedChunks: samples.reduce((sum, sample) => sum + sample.generatedChunks, 0),
    totalEvictedChunks: samples.reduce((sum, sample) => sum + sample.evictedChunks, 0),
    maxPendingChunks: maxValue(samples.map((sample) => sample.pendingChunks)),
    maxPendingMeshJobs: maxValue(samples.map((sample) => sample.pendingMeshJobs)),
    maxDirtyResidentChunks: maxValue(samples.map((sample) => sample.dirtyResidentChunks)),
    maxDirtyMeshlessResidentChunks: maxValue(samples.map((sample) => sample.dirtyMeshlessResidentChunks)),
    framesOver16_67Ms: samples.filter((sample) => sample.gameplayFrameMs > 16.67).length,
    framesOver33_33Ms: samples.filter((sample) => sample.gameplayFrameMs > 33.33).length,
  };
}

function sumNumbers(values: readonly number[]): number {
  return values.reduce((sum, value) => sum + value, 0);
}

function firstElapsed(
  samples: readonly BootstrapBenchmarkSample[],
  key: "playableReady" | "visualReady",
): number | null {
  for (const sample of samples) {
    if (sample[key]) {
      return sample.elapsedMs;
    }
  }
  return null;
}
