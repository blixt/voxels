import { toAsyncChunkGenerationKey } from "../engine/async-chunk-generation.ts";
import type { TransferredGeneratedChunkRenderSummary } from "../engine/generated-chunk-transfer.ts";
import type { GeneratedChunk } from "../engine/procedural-generator.ts";
import type { ChunkCoordinate } from "../engine/types.ts";

export type DeferredProceduralPersistenceJob =
  | {
      type: "chunk";
      coord: ChunkCoordinate;
      chunk: GeneratedChunk;
    }
  | {
      type: "summary";
      coord: ChunkCoordinate;
      summary: TransferredGeneratedChunkRenderSummary;
    };

export interface DeferredProceduralPersistenceQueue {
  readonly order: string[];
  readonly jobsByKey: Map<string, DeferredProceduralPersistenceJob>;
}

export function createDeferredProceduralPersistenceQueue(): DeferredProceduralPersistenceQueue {
  return {
    order: [],
    jobsByKey: new Map(),
  };
}

export function enqueueDeferredProceduralPersistenceJob(
  queue: DeferredProceduralPersistenceQueue,
  job: DeferredProceduralPersistenceJob,
): void {
  const key = toDeferredProceduralPersistenceKey(job.coord);
  const existing = queue.jobsByKey.get(key);
  if (!existing) {
    queue.jobsByKey.set(key, job);
    queue.order.push(key);
    return;
  }
  if (existing.type === "chunk" && job.type === "summary") {
    return;
  }
  queue.jobsByKey.set(key, job);
}

export function shiftDeferredProceduralPersistenceJob(
  queue: DeferredProceduralPersistenceQueue,
): DeferredProceduralPersistenceJob | null {
  while (queue.order.length > 0) {
    const key = queue.order.shift()!;
    const job = queue.jobsByKey.get(key);
    if (!job) {
      continue;
    }
    queue.jobsByKey.delete(key);
    return job;
  }
  return null;
}

export function clearDeferredProceduralPersistenceQueue(queue: DeferredProceduralPersistenceQueue): void {
  queue.order.length = 0;
  queue.jobsByKey.clear();
}

export function countDeferredProceduralPersistenceJobs(queue: DeferredProceduralPersistenceQueue): number {
  return queue.jobsByKey.size;
}

function toDeferredProceduralPersistenceKey(coord: ChunkCoordinate): string {
  return toAsyncChunkGenerationKey(coord.x, coord.y, coord.z);
}
