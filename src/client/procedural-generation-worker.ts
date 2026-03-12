/// <reference lib="webworker" />

import { openProceduralGeneratedChunkCache, type ProceduralGeneratedChunkCache } from "./procedural-generated-chunk-cache.ts";
import { decodeGeneratedChunkSummary } from "../engine/generated-chunk-codec.ts";
import { ProceduralWorldGenerator } from "../engine/procedural-generator.ts";
import {
  serializeGeneratedChunk,
  serializeGeneratedChunkRenderSummary,
  type TransferredGeneratedChunk,
  type TransferredGeneratedChunkRenderSummary,
  type TransferredGeneratedRenderColumnSummary,
} from "../engine/generated-chunk-transfer.ts";
import type { ChunkCoordinate, ColumnCoordinate } from "../engine/types.ts";

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
      type: "summarize-column";
      requestId: number;
      coord: ColumnCoordinate;
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
      type: "column-summarized";
      requestId: number;
      source: "cache" | "missing";
      coord: ColumnCoordinate;
      summary: TransferredGeneratedRenderColumnSummary | null;
    };

let generator: ProceduralWorldGenerator | null = null;
let chunkCache: ProceduralGeneratedChunkCache | null = null;
const reportedCacheFailures = new Set<string>();

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
  if (message.type === "summarize-column") {
    let cachedColumnSummary: TransferredGeneratedRenderColumnSummary | null = null;
    try {
      cachedColumnSummary = await chunkCache?.getColumnSummary(message.coord) ?? null;
    } catch (error) {
      reportCacheFailure("read", error);
      chunkCache?.close();
      chunkCache = null;
      cachedColumnSummary = null;
    }
    const response: WorkerResponse = {
      type: "column-summarized",
      requestId: message.requestId,
      source: cachedColumnSummary ? "cache" : "missing",
      coord: { ...message.coord },
      summary: cachedColumnSummary,
    };
    self.postMessage(response, {
      transfer: cachedColumnSummary
        ? [
            cachedColumnSummary.surfaceY.buffer,
            cachedColumnSummary.surfaceMaterial.buffer,
            cachedColumnSummary.waterTopY.buffer,
            cachedColumnSummary.waterMaterial.buffer,
          ]
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
  let cachedChunk: TransferredGeneratedChunk | null = null;
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
      const response: WorkerResponse = {
        type: "generated",
        requestId: message.requestId,
        source: "cache",
        chunk: cachedChunk,
      };
      self.postMessage(response, { transfer: [cachedChunk.encodedBuffer] });
      return;
    }
    const summary = serializeGeneratedChunkRenderSummary(decodeGeneratedChunkSummary(cachedChunk.encodedBuffer).renderSummary);
    try {
      await chunkCache?.putChunkSummary(message.coord, summary.summary);
    } catch (error) {
      reportCacheFailure("write", error);
      chunkCache?.close();
      chunkCache = null;
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
  const serialized = serializeGeneratedChunk(generated);
  const summary = serializeGeneratedChunkRenderSummary(generated.renderSummary);
  try {
    await chunkCache?.putChunk(message.coord, serialized.chunk, summary.summary);
  } catch (error) {
    reportCacheFailure("write", error);
    chunkCache?.close();
    chunkCache = null;
  }
  if (message.type === "generate") {
    const response: WorkerResponse = {
      type: "generated",
      requestId: message.requestId,
      source: "generated",
      chunk: serialized.chunk,
    };
    self.postMessage(response, { transfer: serialized.transfer });
    return;
  }
  const response: WorkerResponse = {
    type: "summarized",
    requestId: message.requestId,
    source: "generated",
    summary: summary.summary,
  };
  self.postMessage(response, { transfer: summary.transfer });
}

function reportCacheFailure(stage: "open" | "read" | "write", error: unknown): void {
  if (reportedCacheFailures.has(stage)) {
    return;
  }
  reportedCacheFailures.add(stage);
  console.warn(`[procedural-generation-worker] persistent chunk cache ${stage} failed; disabling cache`, error);
}

export {};
