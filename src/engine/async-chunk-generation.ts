import type { GeneratedChunk } from "./procedural-generator.ts";

export interface AsyncChunkGenerationCompletionStats {
  cacheHits: number;
  generated: number;
}

export interface AsyncChunkGenerationQueue {
  requestChunk(cx: number, cy: number, cz: number): boolean;
  hasPendingChunk(cx: number, cy: number, cz: number): boolean;
  getPendingCount(): number;
  drainCompletedChunks(): GeneratedChunk[];
  drainCompletionStats(): AsyncChunkGenerationCompletionStats;
  dispose(): void;
}

export function toAsyncChunkGenerationKey(cx: number, cy: number, cz: number): string {
  return `${cx}:${cy}:${cz}`;
}
