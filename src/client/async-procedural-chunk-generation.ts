import type { AsyncChunkGenerationQueue } from "../engine/async-chunk-generation.ts";
import {
  toAsyncChunkGenerationKey,
  toAsyncLodChunkKey,
  toAsyncRegionSummaryKey,
  type AsyncDerivedLodChunkCacheKey,
  type AsyncEncodedDerivedLodChunk,
} from "../engine/async-chunk-generation.ts";
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

interface WorkerLodChunkMessage {
  type: "lod-chunk";
  requestId: number;
  source: "cache" | "missing";
  key: AsyncDerivedLodChunkCacheKey;
  chunk: {
    buffer: ArrayBuffer;
    byteLength: number;
  } | null;
}

interface WorkerLodChunkStoredMessage {
  type: "lod-chunk-stored";
  requestId: number;
  key: AsyncDerivedLodChunkCacheKey;
  stored: boolean;
}

type WorkerMessage =
  | WorkerReadyMessage
  | WorkerGeneratedMessage
  | WorkerSummarizedMessage
  | WorkerRegionSummarizedMessage
  | WorkerLodChunkMessage
  | WorkerLodChunkStoredMessage;

type PendingRequestMode = "chunk" | "summary" | "region-summary" | "lod-chunk" | "lod-store";

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
  const completedLodChunks: AsyncEncodedDerivedLodChunk[] = [];
  const missingLodChunks: AsyncDerivedLodChunkCacheKey[] = [];
  let completedCacheHits = 0;
  let completedGenerated = 0;
  let completedSummaryCacheHits = 0;
  let completedSummaryGenerated = 0;
  let completedLodCacheHits = 0;
  let completedLodMissing = 0;
  let completedLodStored = 0;
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
    if (pending.mode === "lod-store") {
      if (message.type !== "lod-chunk-stored") {
        throw new Error(`Expected LOD chunk store response for ${pending.key}, received ${message.type}`);
      }
      if (message.stored) {
        completedLodStored += 1;
      }
      return;
    }
    if (pending.mode === "lod-chunk") {
      if (message.type !== "lod-chunk") {
        throw new Error(`Expected LOD chunk response for ${pending.key}, received ${message.type}`);
      }
      if (message.source === "cache" && message.chunk) {
        completedLodCacheHits += 1;
        completedLodChunks.push({
          key: cloneLodChunkCacheKey(message.key),
          buffer: message.chunk.buffer,
          byteLength: message.chunk.byteLength,
        });
      } else {
        completedLodMissing += 1;
        missingLodChunks.push(cloneLodChunkCacheKey(message.key));
      }
      return;
    }
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
      if (message.type !== "generated") {
        throw new Error(`Expected generated chunk response for ${pending.key}, received ${message.type}`);
      }
      if (message.source === "cache") {
        completedCacheHits += 1;
      } else {
        completedGenerated += 1;
      }
      completedChunks.push(deserializeGeneratedChunk(message.chunk));
      return;
    }
    if (message.type !== "summarized") {
      throw new Error(`Expected summarized chunk response for ${pending.key}, received ${message.type}`);
    }
    if (message.source === "cache") {
      completedSummaryCacheHits += 1;
    } else {
      completedSummaryGenerated += 1;
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
    requestLodChunk(key: AsyncDerivedLodChunkCacheKey): boolean {
      return requestLodChunk(key);
    },
    storeLodChunk(chunk: AsyncEncodedDerivedLodChunk): boolean {
      return storeLodChunk(chunk);
    },
    hasPendingChunk(cx: number, cy: number, cz: number): boolean {
      return pendingKeys.has(toAsyncChunkGenerationKey(cx, cy, cz));
    },
    hasPendingRegionSummary(regionX: number, regionZ: number): boolean {
      return pendingKeys.has(toAsyncRegionSummaryKey(regionX, regionZ));
    },
    hasPendingLodChunk(key: AsyncDerivedLodChunkCacheKey): boolean {
      return pendingKeys.has(toAsyncLodChunkKey(key));
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
    drainCompletedLodChunks(): AsyncEncodedDerivedLodChunk[] {
      if (completedLodChunks.length === 0) {
        return [];
      }
      return completedLodChunks.splice(0, completedLodChunks.length);
    },
    drainMissingLodChunks(): AsyncDerivedLodChunkCacheKey[] {
      if (missingLodChunks.length === 0) {
        return [];
      }
      return missingLodChunks.splice(0, missingLodChunks.length);
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
    drainLodChunkCompletionStats() {
      const stats = {
        cacheHits: completedLodCacheHits,
        missing: completedLodMissing,
        stored: completedLodStored,
      };
      completedLodCacheHits = 0;
      completedLodMissing = 0;
      completedLodStored = 0;
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
      completedLodChunks.length = 0;
      missingLodChunks.length = 0;
      completedCacheHits = 0;
      completedGenerated = 0;
      completedSummaryCacheHits = 0;
      completedSummaryGenerated = 0;
      completedLodCacheHits = 0;
      completedLodMissing = 0;
      completedLodStored = 0;
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

  function requestLodChunk(key: AsyncDerivedLodChunkCacheKey): boolean {
    const requestKey = toAsyncLodChunkKey(key);
    if (pendingKeys.has(requestKey) || pendingKeys.size >= maxPendingJobs) {
      return false;
    }
    const workerIndex = chooseWorkerIndex();
    const requestId = nextRequestId++;
    pendingKeys.add(requestKey);
    pendingRequests.set(requestId, { key: requestKey, mode: "lod-chunk" });
    workerSlots[workerIndex]!.pendingCount += 1;
    workerSlots[workerIndex]!.worker.postMessage({
      type: "get-lod-chunk",
      requestId,
      key: cloneLodChunkCacheKey(key),
    });
    return true;
  }

  function storeLodChunk(chunk: AsyncEncodedDerivedLodChunk): boolean {
    const requestKey = `store:${toAsyncLodChunkKey(chunk.key)}`;
    if (pendingKeys.has(requestKey) || pendingKeys.size >= maxPendingJobs) {
      return false;
    }
    const workerIndex = chooseWorkerIndex();
    const requestId = nextRequestId++;
    pendingKeys.add(requestKey);
    pendingRequests.set(requestId, { key: requestKey, mode: "lod-store" });
    workerSlots[workerIndex]!.pendingCount += 1;
    workerSlots[workerIndex]!.worker.postMessage({
      type: "put-lod-chunk",
      requestId,
      key: cloneLodChunkCacheKey(chunk.key),
      chunk: {
        buffer: chunk.buffer,
        byteLength: chunk.byteLength,
      },
    });
    return true;
  }

  function chooseWorkerIndex(): number {
    let bestWorkerIndex = 0;
    let bestPendingCount = workerSlots[0]!.pendingCount;
    for (let index = 1; index < workerSlots.length; index += 1) {
      const pendingCount = workerSlots[index]!.pendingCount;
      if (pendingCount < bestPendingCount) {
        bestPendingCount = pendingCount;
        bestWorkerIndex = index;
      }
    }
    return bestWorkerIndex;
  }
}

function cloneLodChunkCacheKey(key: AsyncDerivedLodChunkCacheKey): AsyncDerivedLodChunkCacheKey {
  return {
    lodLevel: key.lodLevel,
    editRevision: key.editRevision,
    coord: { ...key.coord },
  };
}

function resolveProceduralGenerationWorkerCount(requestedWorkerCount?: number): number {
  if (requestedWorkerCount !== undefined) {
    const normalized = Number.isFinite(requestedWorkerCount) ? Math.floor(requestedWorkerCount) : 1;
    return Math.max(1, normalized);
  }
  const hardwareConcurrency = Math.max(1, Math.floor(globalThis.navigator?.hardwareConcurrency ?? 4));
  return Math.max(1, Math.min(DEFAULT_MAX_WORKERS, hardwareConcurrency - DEFAULT_RESERVED_THREADS));
}
