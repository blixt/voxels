/// <reference lib="webworker" />

import {
  openProceduralGeneratedChunkCache,
  type CachedEncodedGeneratedChunk,
  type CachedEncodedDerivedLodChunk,
  type ProceduralGeneratedChunkCache,
} from "./procedural-generated-chunk-cache.ts";
import {
  clearDeferredProceduralPersistenceQueue,
  countDeferredProceduralPersistenceJobs,
  createDeferredProceduralPersistenceQueue,
  enqueueDeferredProceduralPersistenceJob,
  shiftDeferredProceduralPersistenceJob,
} from "./procedural-deferred-persistence.ts";
import { decodeGeneratedChunk, decodeGeneratedChunkSummary, encodeGeneratedChunk } from "../engine/generated-chunk-codec.ts";
import { encodeDerivedLodChunk } from "../engine/derived-lod-chunk-codec.ts";
import { deriveLodChunkData } from "../engine/lod-chunk-derivation.ts";
import type { AsyncDerivedLodChunkCacheKey } from "../engine/async-chunk-generation.ts";
import { ProceduralWorldGenerator, type GeneratedChunk } from "../engine/procedural-generator.ts";
import {
  serializeGeneratedChunk,
  serializeGeneratedChunkRenderSummary,
  type TransferredGeneratedChunk,
  type TransferredGeneratedChunkRenderSummary,
  type TransferredGeneratedRenderSummaryRegion,
} from "../engine/generated-chunk-transfer.ts";
import type { ChunkCoordinate, RenderSummaryRegionCoordinate } from "../engine/types.ts";

type WorkerRequest =
  | {
      type: "init";
      seed: number;
      seaLevel: number;
      chunkSize: number;
      maxYExclusive: number;
    }
  | {
      type: "generate";
      requestId: number;
      coord: ChunkCoordinate;
    }
  | {
      type: "summarize";
      requestId: number;
      coord: ChunkCoordinate;
    }
  | {
      type: "summarize-region";
      requestId: number;
      coord: RenderSummaryRegionCoordinate;
    }
  | {
      type: "get-lod-chunk";
      requestId: number;
      key: AsyncDerivedLodChunkCacheKey;
    }
  | {
      type: "derive-lod-chunk";
      requestId: number;
      key: AsyncDerivedLodChunkCacheKey;
    }
  | {
      type: "put-lod-chunk";
      requestId: number;
      key: AsyncDerivedLodChunkCacheKey;
      chunk: {
        buffer: ArrayBuffer;
        byteLength: number;
      };
    };

type WorkerResponse =
  | {
      type: "ready";
    }
  | {
      type: "generated";
      requestId: number;
      source: "cache" | "generated";
      chunk: TransferredGeneratedChunk;
    }
  | {
      type: "summarized";
      requestId: number;
      source: "cache" | "generated";
      summary: TransferredGeneratedChunkRenderSummary;
    }
  | {
      type: "region-summarized";
      requestId: number;
      source: "cache" | "missing";
      coord: RenderSummaryRegionCoordinate;
      summary: TransferredGeneratedRenderSummaryRegion | null;
    }
  | {
      type: "lod-chunk";
      requestId: number;
      source: "cache" | "generated" | "missing";
      key: AsyncDerivedLodChunkCacheKey;
      stored?: boolean;
      chunk: {
        buffer: ArrayBuffer;
        byteLength: number;
      } | null;
    }
  | {
      type: "lod-chunk-stored";
      requestId: number;
      key: AsyncDerivedLodChunkCacheKey;
      stored: boolean;
    };

let generator: ProceduralWorldGenerator | null = null;
let chunkCache: ProceduralGeneratedChunkCache | null = null;
const reportedCacheFailures = new Set<string>();
const DEFERRED_PERSISTENCE_FLUSH_DELAY_MS = 250;
const MAX_PERSISTENCE_JOBS_PER_FLUSH = 2;

const pendingPersistenceJobs = createDeferredProceduralPersistenceQueue();
let persistenceFlushTimer = 0;
let persistenceFlushInProgress = false;

self.onmessage = (event: MessageEvent<WorkerRequest>) => {
  void handleMessage(event.data);
};

async function handleMessage(message: WorkerRequest): Promise<void> {
  if (message.type === "init") {
    generator = new ProceduralWorldGenerator(message.seed, {
      seaLevel: message.seaLevel,
      chunkSize: message.chunkSize,
      maxYExclusive: message.maxYExclusive,
    });
    try {
      chunkCache = await openProceduralGeneratedChunkCache({
        seed: message.seed,
        seaLevel: message.seaLevel,
        chunkSize: message.chunkSize,
        maxYExclusive: message.maxYExclusive,
      });
    } catch (error) {
      reportCacheFailure("open", error);
      chunkCache = null;
    }
    const response: WorkerResponse = { type: "ready" };
    self.postMessage(response);
    return;
  }
  if (!generator) {
    throw new Error("Procedural generation worker received a request before initialization");
  }
  if (message.type === "get-lod-chunk") {
    let cachedLodChunk: CachedEncodedDerivedLodChunk | null = null;
    try {
      cachedLodChunk = await chunkCache?.getLodChunk(message.key) ?? null;
    } catch (error) {
      reportCacheFailure("read", error);
      chunkCache?.close();
      chunkCache = null;
      cachedLodChunk = null;
    }
    const response: WorkerResponse = {
      type: "lod-chunk",
      requestId: message.requestId,
      source: cachedLodChunk ? "cache" : "missing",
      key: cloneLodChunkCacheKey(message.key),
      chunk: cachedLodChunk
        ? {
            buffer: cachedLodChunk.buffer,
            byteLength: cachedLodChunk.byteLength,
          }
        : null,
    };
    self.postMessage(response, {
      transfer: cachedLodChunk ? [cachedLodChunk.buffer] : [],
    });
    return;
  }
  if (message.type === "derive-lod-chunk") {
    const encoded = deriveGeneratedLodChunk(generator, message.key);
    let stored = false;
    try {
      if (chunkCache) {
        await chunkCache.putLodChunk(message.key, encoded);
        stored = true;
      }
    } catch (error) {
      reportCacheFailure("write", error);
      chunkCache?.close();
      chunkCache = null;
    }
    const response: WorkerResponse = {
      type: "lod-chunk",
      requestId: message.requestId,
      source: "generated",
      key: cloneLodChunkCacheKey(message.key),
      stored,
      chunk: {
        buffer: encoded.buffer,
        byteLength: encoded.stats.byteLength,
      },
    };
    self.postMessage(response, {
      transfer: [encoded.buffer],
    });
    return;
  }
  if (message.type === "put-lod-chunk") {
    let stored = false;
    try {
      if (chunkCache) {
        await chunkCache.putLodChunk(message.key, {
          buffer: message.chunk.buffer,
          stats: {
            byteLength: message.chunk.byteLength,
            chunkSize: 0,
            voxelCount: 0,
            runCount: 0,
            zeroRunCount: 0,
          },
        });
        stored = true;
      }
    } catch (error) {
      reportCacheFailure("write", error);
      chunkCache?.close();
      chunkCache = null;
    }
    const response: WorkerResponse = {
      type: "lod-chunk-stored",
      requestId: message.requestId,
      key: cloneLodChunkCacheKey(message.key),
      stored,
    };
    self.postMessage(response);
    return;
  }
  if (message.type === "summarize-region") {
    let cachedRegionSummary: TransferredGeneratedRenderSummaryRegion | null = null;
    try {
      cachedRegionSummary = await chunkCache?.getRegionSummary(message.coord) ?? null;
    } catch (error) {
      reportCacheFailure("read", error);
      chunkCache?.close();
      chunkCache = null;
      cachedRegionSummary = null;
    }
    const response: WorkerResponse = {
      type: "region-summarized",
      requestId: message.requestId,
      source: cachedRegionSummary ? "cache" : "missing",
      coord: { ...message.coord },
      summary: cachedRegionSummary,
    };
    self.postMessage(response, {
      transfer: cachedRegionSummary
        ? serializeGeneratedRenderSummaryRegionTransfer(cachedRegionSummary)
        : [],
    });
    return;
  }
  if (message.type === "summarize") {
    let cachedSummary: TransferredGeneratedChunkRenderSummary | null = null;
    try {
      cachedSummary = await chunkCache?.getChunkSummary(message.coord) ?? null;
    } catch (error) {
      reportCacheFailure("read", error);
      chunkCache?.close();
      chunkCache = null;
      cachedSummary = null;
    }
    if (cachedSummary) {
      const response: WorkerResponse = {
        type: "summarized",
        requestId: message.requestId,
        source: "cache",
        summary: cachedSummary,
      };
      self.postMessage(response, {
        transfer: [
          cachedSummary.surfaceY.buffer,
          cachedSummary.surfaceMaterial.buffer,
          cachedSummary.waterTopY.buffer,
          cachedSummary.waterMaterial.buffer,
          cachedSummary.macroCellStates.buffer,
          cachedSummary.faceOpenMask.buffer,
        ],
      });
      return;
    }
  }
  let cachedChunk: CachedEncodedGeneratedChunk | null = null;
  try {
    cachedChunk = await chunkCache?.getChunk(message.coord) ?? null;
  } catch (error) {
    reportCacheFailure("read", error);
    chunkCache?.close();
    chunkCache = null;
    cachedChunk = null;
  }
  if (cachedChunk) {
    if (message.type === "generate") {
      const transferredChunk = serializeGeneratedChunk(decodeGeneratedChunk(cachedChunk.buffer));
      const response: WorkerResponse = {
        type: "generated",
        requestId: message.requestId,
        source: "cache",
        chunk: transferredChunk.chunk,
      };
      self.postMessage(response, { transfer: transferredChunk.transfer });
      return;
    }
    const summary = serializeGeneratedChunkRenderSummary(decodeGeneratedChunkSummary(cachedChunk.buffer).renderSummary);
    if (chunkCache) {
      enqueueDeferredProceduralPersistenceJob(pendingPersistenceJobs, {
        type: "summary",
        coord: { ...message.coord },
        summary: cloneTransferredSummary(summary.summary),
      });
      scheduleDeferredPersistenceFlush();
    }
    const response: WorkerResponse = {
      type: "summarized",
      requestId: message.requestId,
      source: "cache",
      summary: summary.summary,
    };
    self.postMessage(response, { transfer: summary.transfer });
    return;
  }
  const generated = generator.generateChunk(message.coord.x, message.coord.y, message.coord.z);
  if (message.type === "generate") {
    if (chunkCache) {
      enqueueDeferredProceduralPersistenceJob(pendingPersistenceJobs, {
        type: "chunk",
        chunk: cloneGeneratedChunkForDeferredPersistence(generated),
        coord: { ...generated.coord },
      });
      scheduleDeferredPersistenceFlush();
    }
    const transferredChunk = serializeGeneratedChunk(generated);
    const response: WorkerResponse = {
      type: "generated",
      requestId: message.requestId,
      source: "generated",
      chunk: transferredChunk.chunk,
    };
    self.postMessage(response, { transfer: transferredChunk.transfer });
    return;
  }
  const summary = serializeGeneratedChunkRenderSummary(generated.renderSummary);
  if (chunkCache) {
    enqueueDeferredProceduralPersistenceJob(pendingPersistenceJobs, {
      type: "summary",
      coord: { ...generated.coord },
      summary: cloneTransferredSummary(summary.summary),
    });
    scheduleDeferredPersistenceFlush();
  }
  const response: WorkerResponse = {
    type: "summarized",
    requestId: message.requestId,
    source: "generated",
    summary: summary.summary,
  };
  self.postMessage(response, { transfer: summary.transfer });
}

function deriveGeneratedLodChunk(
  generator: ProceduralWorldGenerator,
  key: AsyncDerivedLodChunkCacheKey,
): ReturnType<typeof encodeDerivedLodChunk> {
  const level = Math.max(1, Math.floor(key.lodLevel));
  const chunkSize = generator.chunkSize;
  const stride = 1 << level;
  const worldSize = chunkSize * stride;
  const originX = key.coord.x * worldSize;
  const originY = key.coord.y * worldSize;
  const originZ = key.coord.z * worldSize;
  const derived = deriveLodChunkData({
    chunkSize,
    originX,
    originY,
    originZ,
    level,
    stride,
    useGeneratedTopMaterial: level >= 2,
    isOutputColumnCoveredByFiner: () => false,
    sampleSourceMaterial: () => ({ material: 0, complete: false }),
    sampleGeneratedTopMaterial: (ox, oz, minOy, maxOyExclusive) => {
      const worldX = originX + ox * stride;
      const worldZ = originZ + oz * stride;
      return generator.sampleTopColumnMaterialBucket(
        worldX,
        worldZ,
        originY + minOy * stride,
        stride,
        maxOyExclusive - minOy,
        stride * 3,
      );
    },
    sampleSurfaceYRange: (ox, oz) => {
      const worldX = originX + ox * stride;
      const worldZ = originZ + oz * stride;
      const column = generator.sampleSurfaceColumn(worldX, worldZ);
      return {
        minY: column.surfaceY,
        maxY: Math.max(column.topY, column.waterTopY ?? column.surfaceY),
      };
    },
    sampleGeneratedColumnMaterials: (ox, oz, startOy, endOyInclusive) => {
      const worldX = originX + ox * stride;
      const worldZ = originZ + oz * stride;
      return generator.sampleColumnMaterialBuckets(
        worldX,
        worldZ,
        originY + startOy * stride,
        stride,
        endOyInclusive - startOy + 1,
      );
    },
  });
  return encodeDerivedLodChunk({
    coord: { ...key.coord },
    lodLevel: level,
    voxelStride: stride,
    data: derived.data,
    solidCount: derived.solidCount,
    solidBounds: derived.solidBounds,
  });
}

function reportCacheFailure(stage: "open" | "read" | "write", error: unknown): void {
  if (reportedCacheFailures.has(stage)) {
    return;
  }
  reportedCacheFailures.add(stage);
  console.warn(`[procedural-generation-worker] persistent chunk cache ${stage} failed; disabling cache`, error);
}

function scheduleDeferredPersistenceFlush(): void {
  if (!chunkCache || persistenceFlushTimer !== 0 || persistenceFlushInProgress || countDeferredProceduralPersistenceJobs(pendingPersistenceJobs) === 0) {
    return;
  }
  persistenceFlushTimer = self.setTimeout(() => {
    persistenceFlushTimer = 0;
    void flushDeferredPersistenceJobs();
  }, DEFERRED_PERSISTENCE_FLUSH_DELAY_MS);
}

async function flushDeferredPersistenceJobs(): Promise<void> {
  if (!chunkCache || persistenceFlushInProgress || countDeferredProceduralPersistenceJobs(pendingPersistenceJobs) === 0) {
    return;
  }
  persistenceFlushInProgress = true;
  try {
    let processedJobs = 0;
    while (chunkCache && countDeferredProceduralPersistenceJobs(pendingPersistenceJobs) > 0 && processedJobs < MAX_PERSISTENCE_JOBS_PER_FLUSH) {
      const job = shiftDeferredProceduralPersistenceJob(pendingPersistenceJobs);
      if (!job) {
        break;
      }
      if (job.type === "summary") {
        await chunkCache.putChunkSummary(job.coord, job.summary);
      } else {
        const summary = cloneTransferredSummary(serializeGeneratedChunkRenderSummary(job.chunk.renderSummary).summary);
        await chunkCache.putChunk(job.chunk.coord, encodeGeneratedChunk(job.chunk), summary);
      }
      processedJobs += 1;
    }
  } catch (error) {
    reportCacheFailure("write", error);
    chunkCache?.close();
    chunkCache = null;
    clearDeferredProceduralPersistenceQueue(pendingPersistenceJobs);
  } finally {
    persistenceFlushInProgress = false;
    scheduleDeferredPersistenceFlush();
  }
}

function cloneGeneratedChunkForDeferredPersistence(chunk: GeneratedChunk): GeneratedChunk {
  return {
    coord: { ...chunk.coord },
    data: chunk.data.slice(),
    solidCount: chunk.solidCount,
    solidBounds: chunk.solidBounds
      ? {
          min: [...chunk.solidBounds.min],
          max: [...chunk.solidBounds.max],
        }
      : null,
    renderSummary: cloneGeneratedChunkRenderSummary(chunk.renderSummary),
  };
}

function cloneGeneratedChunkRenderSummary(
  summary: import("../engine/generated-chunk-render-summary.ts").GeneratedChunkRenderSummary,
): import("../engine/generated-chunk-render-summary.ts").GeneratedChunkRenderSummary {
  return {
    coord: { ...summary.coord },
    coveredColumnCount: summary.coveredColumnCount,
    surfaceY: summary.surfaceY.slice(),
    surfaceMaterial: summary.surfaceMaterial.slice(),
    waterTopY: summary.waterTopY.slice(),
    waterMaterial: summary.waterMaterial.slice(),
    macroCellSize: summary.macroCellSize,
    macroCellsPerAxis: summary.macroCellsPerAxis,
    macroCellStates: summary.macroCellStates.slice(),
    faceOpenMask: summary.faceOpenMask.slice(),
  };
}

function cloneTransferredSummary(summary: TransferredGeneratedChunkRenderSummary): TransferredGeneratedChunkRenderSummary {
  return {
    coord: { ...summary.coord },
    coveredColumnCount: summary.coveredColumnCount,
    surfaceY: summary.surfaceY.slice(),
    surfaceMaterial: summary.surfaceMaterial.slice(),
    waterTopY: summary.waterTopY.slice(),
    waterMaterial: summary.waterMaterial.slice(),
    macroCellSize: summary.macroCellSize,
    macroCellsPerAxis: summary.macroCellsPerAxis,
    macroCellStates: summary.macroCellStates.slice(),
    faceOpenMask: summary.faceOpenMask.slice(),
  };
}

function cloneLodChunkCacheKey(key: AsyncDerivedLodChunkCacheKey): AsyncDerivedLodChunkCacheKey {
  return {
    lodLevel: key.lodLevel,
    editRevision: key.editRevision,
    coord: { ...key.coord },
  };
}

function serializeGeneratedRenderSummaryRegionTransfer(
  summary: TransferredGeneratedRenderSummaryRegion,
): Transferable[] {
  return summary.columns.flatMap((entry) => [
    entry.summary.surfaceY.buffer,
    entry.summary.surfaceMaterial.buffer,
    entry.summary.waterTopY.buffer,
    entry.summary.waterMaterial.buffer,
  ]);
}

export {};
