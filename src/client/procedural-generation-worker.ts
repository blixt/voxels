/// <reference lib="webworker" />

import { openProceduralGeneratedChunkCache, type ProceduralGeneratedChunkCache } from "./procedural-generated-chunk-cache.ts";
import { ProceduralWorldGenerator } from "../engine/procedural-generator.ts";
import { serializeGeneratedChunk, type TransferredGeneratedChunk } from "../engine/generated-chunk-transfer.ts";
import type { ChunkCoordinate } from "../engine/types.ts";

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
    throw new Error("Procedural generation worker received a generate request before initialization");
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
    const response: WorkerResponse = {
      type: "generated",
      requestId: message.requestId,
      source: "cache",
      chunk: cachedChunk,
    };
    self.postMessage(response, { transfer: [cachedChunk.encodedBuffer] });
    return;
  }
  const generated = generator.generateChunk(message.coord.x, message.coord.y, message.coord.z);
  const serialized = serializeGeneratedChunk(generated);
  try {
    await chunkCache?.putChunk(message.coord, serialized.chunk);
  } catch (error) {
    reportCacheFailure("write", error);
    chunkCache?.close();
    chunkCache = null;
  }
  const response: WorkerResponse = {
    type: "generated",
    requestId: message.requestId,
    source: "generated",
    chunk: serialized.chunk,
  };
  self.postMessage(response, { transfer: serialized.transfer });
}

function reportCacheFailure(stage: "open" | "read" | "write", error: unknown): void {
  if (reportedCacheFailures.has(stage)) {
    return;
  }
  reportedCacheFailures.add(stage);
  console.warn(`[procedural-generation-worker] persistent chunk cache ${stage} failed; disabling cache`, error);
}

export {};
