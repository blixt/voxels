import type { GeneratedChunk } from "./procedural-generator.ts";

export interface AsyncChunkGenerationQueue {
  requestChunk(cx: number, cy: number, cz: number): boolean;
  hasPendingChunk(cx: number, cy: number, cz: number): boolean;
  getPendingCount(): number;
  drainCompletedChunks(): GeneratedChunk[];
  dispose(): void;
}

export function toAsyncChunkGenerationKey(cx: number, cy: number, cz: number): string {
  return `${cx}:${cy}:${cz}`;
}
