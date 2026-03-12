import type { AsyncChunkGenerationQueue } from "../engine/async-chunk-generation.ts";
import { toAsyncChunkGenerationKey, toAsyncRegionSummaryKey } from "../engine/async-chunk-generation.ts";
import {
  deserializeGeneratedChunk,
  deserializeGeneratedChunkRenderSummary,
  type TransferredGeneratedChunk,
  type TransferredGeneratedChunkRenderSummary,
  deserializeGeneratedRenderSummaryRegion,
  type TransferredGeneratedRenderSummaryRegion,
} from "../engine/generated-chunk-transfer.ts";
import type { GeneratedChunkRenderSummary } from "../engine/generated-chunk-render-summary.ts";
import type { GeneratedRenderSummaryRegion } from "../engine/generated-render-summary-region.ts";
import type { GeneratedChunk, ProceduralWorldGenerator } from "../engine/procedural-generator.ts";
import type { ChunkCoordinate, RenderSummaryRegionCoordinate } from "../engine/types.ts";

interface WorkerReadyMessage {
  type: "ready";
}

interface WorkerGeneratedMessage {
  type: "generated";
  requestId: number;
  source: "cache" | "generated";
  chunk: TransferredGeneratedChunk;
}

interface WorkerSummarizedMessage {
  type: "summarized";
  requestId: number;
  source: "cache" | "generated";
  summary: TransferredGeneratedChunkRenderSummary;
}

interface WorkerRegionSummarizedMessage {
  type: "region-summarized";
  requestId: number;
  source: "cache" | "missing";
  coord: RenderSummaryRegionCoordinate;
  summary: TransferredGeneratedRenderSummaryRegion | null;
}

type WorkerMessage =
  | WorkerReadyMessage
  | WorkerGeneratedMessage
  | WorkerSummarizedMessage
  | WorkerRegionSummarizedMessage;

type PendingRequestMode = "chunk" | "summary" | "region-summary";

interface PendingRequest {
  key: string;
  mode: PendingRequestMode;
}

interface WorkerSlot {
  worker: Worker;
  pendingCount: number;
}

const DEFAULT_MAX_PENDING_JOBS = 128;
const DEFAULT_MAX_WORKERS = 2;
const DEFAULT_RESERVED_THREADS = 2;
const PROCEDURAL_WORKER_ASSET_URL = "/assets/procedural-generation-worker.js";

export function createAsyncProceduralChunkGeneration(
  generator: ProceduralWorldGenerator,
  options: {
    workerCount?: number;
    maxPendingJobs?: number;
  } = {},
): AsyncChunkGenerationQueue | null {
  if (typeof Worker === "undefined") {
    return null;
  }
  const workerCount = resolveProceduralGenerationWorkerCount(options.workerCount);
  const maxPendingJobs = options.maxPendingJobs ?? DEFAULT_MAX_PENDING_JOBS;
  const workerSlots: WorkerSlot[] = [];
  const pendingRequests = new Map<number, PendingRequest>();
  const pendingKeys = new Set<string>();
  const completedChunks: GeneratedChunk[] = [];
  const completedSummaries: GeneratedChunkRenderSummary[] = [];
  const completedRegionSummaries: GeneratedRenderSummaryRegion[] = [];
  const missingRegionSummaries: RenderSummaryRegionCoordinate[] = [];
  let completedCacheHits = 0;
  let completedGenerated = 0;
  let completedSummaryCacheHits = 0;
  let completedSummaryGenerated = 0;
  let nextRequestId = 1;

  const handleMessage = (slotIndex: number, event: MessageEvent<WorkerMessage>) => {
    const message = event.data;
    if (message.type === "ready") {
      return;
    }
    const pending = pendingRequests.get(message.requestId);
    if (!pending) {
      return;
    }
    pendingRequests.delete(message.requestId);
    pendingKeys.delete(pending.key);
    workerSlots[slotIndex]!.pendingCount = Math.max(0, workerSlots[slotIndex]!.pendingCount - 1);
    if (pending.mode === "region-summary") {
      if (message.type !== "region-summarized") {
        throw new Error(`Expected region summary response for ${pending.key}, received ${message.type}`);
      }
      if (message.source === "cache" && message.summary) {
        completedRegionSummaries.push(deserializeGeneratedRenderSummaryRegion(message.summary));
      } else {
        missingRegionSummaries.push({ ...message.coord });
      }
      return;
    }
    if (pending.mode === "chunk") {
      if (message.source === "cache") {
        completedCacheHits += 1;
      } else {
        completedGenerated += 1;
      }
      if (message.type !== "generated") {
        throw new Error(`Expected generated chunk response for ${pending.key}, received ${message.type}`);
      }
      completedChunks.push(deserializeGeneratedChunk(message.chunk));
      return;
    }
    if (message.source === "cache") {
      completedSummaryCacheHits += 1;
    } else {
      completedSummaryGenerated += 1;
    }
    if (message.type !== "summarized") {
      throw new Error(`Expected summarized chunk response for ${pending.key}, received ${message.type}`);
    }
    completedSummaries.push(deserializeGeneratedChunkRenderSummary(message.summary));
  };

  for (let index = 0; index < workerCount; index += 1) {
    const worker = new Worker(PROCEDURAL_WORKER_ASSET_URL, {
      type: "module",
      name: `procedural-generation-${index}`,
    });
    worker.addEventListener("message", (event) => handleMessage(index, event));
    worker.postMessage({
      type: "init",
      seed: generator.seed,
      seaLevel: generator.seaLevel,
      chunkSize: generator.chunkSize,
      maxYExclusive: generator.maxYExclusive,
    });
    workerSlots.push({
      worker,
      pendingCount: 0,
    });
  }

  return {
    requestChunk(cx: number, cy: number, cz: number): boolean {
      return request("chunk", cx, cy, cz);
    },
    requestSummary(cx: number, cy: number, cz: number): boolean {
      return request("summary", cx, cy, cz);
    },
    requestRegionSummary(regionX: number, regionZ: number): boolean {
      return requestRegionSummary(regionX, regionZ);
    },
    hasPendingChunk(cx: number, cy: number, cz: number): boolean {
      return pendingKeys.has(toAsyncChunkGenerationKey(cx, cy, cz));
    },
    hasPendingRegionSummary(regionX: number, regionZ: number): boolean {
      return pendingKeys.has(toAsyncRegionSummaryKey(regionX, regionZ));
    },
    getWorkerCount(): number {
      return workerSlots.length;
    },
    getPendingCount(): number {
      return pendingKeys.size;
    },
    drainCompletedChunks(): GeneratedChunk[] {
      if (completedChunks.length === 0) {
        return [];
      }
      return completedChunks.splice(0, completedChunks.length);
    },
    drainCompletedSummaries(): GeneratedChunkRenderSummary[] {
      if (completedSummaries.length === 0) {
        return [];
      }
      return completedSummaries.splice(0, completedSummaries.length);
    },
    drainCompletedRegionSummaries(): GeneratedRenderSummaryRegion[] {
      if (completedRegionSummaries.length === 0) {
        return [];
      }
      return completedRegionSummaries.splice(0, completedRegionSummaries.length);
    },
    drainMissingRegionSummaries(): RenderSummaryRegionCoordinate[] {
      if (missingRegionSummaries.length === 0) {
        return [];
      }
      return missingRegionSummaries.splice(0, missingRegionSummaries.length);
    },
    drainCompletionStats() {
      const stats = {
        cacheHits: completedCacheHits,
        generated: completedGenerated,
      };
      completedCacheHits = 0;
      completedGenerated = 0;
      return stats;
    },
    drainSummaryCompletionStats() {
      const stats = {
        cacheHits: completedSummaryCacheHits,
        generated: completedSummaryGenerated,
      };
      completedSummaryCacheHits = 0;
      completedSummaryGenerated = 0;
      return stats;
    },
    dispose(): void {
      for (const slot of workerSlots) {
        slot.worker.terminate();
      }
      workerSlots.length = 0;
      pendingRequests.clear();
      pendingKeys.clear();
      completedChunks.length = 0;
      completedSummaries.length = 0;
      completedRegionSummaries.length = 0;
      missingRegionSummaries.length = 0;
      completedCacheHits = 0;
      completedGenerated = 0;
      completedSummaryCacheHits = 0;
      completedSummaryGenerated = 0;
    },
  };

  function request(mode: PendingRequestMode, cx: number, cy: number, cz: number): boolean {
    const key = toAsyncChunkGenerationKey(cx, cy, cz);
    if (pendingKeys.has(key) || pendingKeys.size >= maxPendingJobs) {
      return false;
    }
    let bestWorkerIndex = 0;
    let bestPendingCount = workerSlots[0]!.pendingCount;
    for (let index = 1; index < workerSlots.length; index += 1) {
      const pendingCount = workerSlots[index]!.pendingCount;
      if (pendingCount < bestPendingCount) {
        bestPendingCount = pendingCount;
        bestWorkerIndex = index;
      }
    }
    const requestId = nextRequestId++;
    pendingKeys.add(key);
    pendingRequests.set(requestId, { key, mode });
    workerSlots[bestWorkerIndex]!.pendingCount += 1;
    const coord: ChunkCoordinate = { x: cx, y: cy, z: cz };
    workerSlots[bestWorkerIndex]!.worker.postMessage({
      type: mode === "chunk" ? "generate" : "summarize",
      requestId,
      coord,
    });
    return true;
  }

  function requestRegionSummary(regionX: number, regionZ: number): boolean {
    const key = toAsyncRegionSummaryKey(regionX, regionZ);
    if (pendingKeys.has(key) || pendingKeys.size >= maxPendingJobs) {
      return false;
    }
    let bestWorkerIndex = 0;
    let bestPendingCount = workerSlots[0]!.pendingCount;
    for (let index = 1; index < workerSlots.length; index += 1) {
      const pendingCount = workerSlots[index]!.pendingCount;
      if (pendingCount < bestPendingCount) {
        bestPendingCount = pendingCount;
        bestWorkerIndex = index;
      }
    }
    const requestId = nextRequestId++;
    pendingKeys.add(key);
    pendingRequests.set(requestId, { key, mode: "region-summary" });
    workerSlots[bestWorkerIndex]!.pendingCount += 1;
    workerSlots[bestWorkerIndex]!.worker.postMessage({
      type: "summarize-region",
      requestId,
      coord: { x: regionX, z: regionZ } satisfies RenderSummaryRegionCoordinate,
    });
    return true;
  }
}

function resolveProceduralGenerationWorkerCount(requestedWorkerCount?: number): number {
  if (requestedWorkerCount !== undefined) {
    const normalized = Number.isFinite(requestedWorkerCount) ? Math.floor(requestedWorkerCount) : 1;
    return Math.max(1, normalized);
  }
  const hardwareConcurrency = Math.max(1, Math.floor(globalThis.navigator?.hardwareConcurrency ?? 4));
  return Math.max(1, Math.min(DEFAULT_MAX_WORKERS, hardwareConcurrency - DEFAULT_RESERVED_THREADS));
}
