import type { AsyncChunkGenerationQueue } from "../engine/async-chunk-generation.ts";
import { toAsyncChunkGenerationKey } from "../engine/async-chunk-generation.ts";
import {
  deserializeGeneratedChunk,
  type TransferredGeneratedChunk,
} from "../engine/generated-chunk-transfer.ts";
import type { GeneratedChunk, ProceduralWorldGenerator } from "../engine/procedural-generator.ts";
import type { ChunkCoordinate } from "../engine/types.ts";

interface WorkerReadyMessage {
  type: "ready";
}

interface WorkerGeneratedMessage {
  type: "generated";
  requestId: number;
  chunk: TransferredGeneratedChunk;
}

type WorkerMessage = WorkerReadyMessage | WorkerGeneratedMessage;

interface PendingRequest {
  key: string;
  workerIndex: number;
}

interface WorkerSlot {
  worker: Worker;
  pendingCount: number;
}

const DEFAULT_MAX_PENDING_JOBS = 128;
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
  const workerCount = Math.max(
    1,
    Math.min(
      2,
      Math.floor(options.workerCount ?? Math.max(1, (globalThis.navigator?.hardwareConcurrency ?? 4) - 1)),
    ),
  );
  const maxPendingJobs = options.maxPendingJobs ?? DEFAULT_MAX_PENDING_JOBS;
  const workerSlots: WorkerSlot[] = [];
  const pendingRequests = new Map<number, PendingRequest>();
  const pendingKeys = new Set<string>();
  const completedChunks: GeneratedChunk[] = [];
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
    completedChunks.push(deserializeGeneratedChunk(message.chunk));
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
      pendingRequests.set(requestId, { key, workerIndex: bestWorkerIndex });
      workerSlots[bestWorkerIndex]!.pendingCount += 1;
      const coord: ChunkCoordinate = { x: cx, y: cy, z: cz };
      workerSlots[bestWorkerIndex]!.worker.postMessage({
        type: "generate",
        requestId,
        coord,
      });
      return true;
    },
    hasPendingChunk(cx: number, cy: number, cz: number): boolean {
      return pendingKeys.has(toAsyncChunkGenerationKey(cx, cy, cz));
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
    dispose(): void {
      for (const slot of workerSlots) {
        slot.worker.terminate();
      }
      workerSlots.length = 0;
      pendingRequests.clear();
      pendingKeys.clear();
      completedChunks.length = 0;
    },
  };
}
