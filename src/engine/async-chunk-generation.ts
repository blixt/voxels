import type { GeneratedChunkRenderSummary } from "./generated-chunk-render-summary.ts";
import type { GeneratedRenderColumnSummary } from "./generated-render-column-summary.ts";
import type { GeneratedChunk } from "./procedural-generator.ts";
import type { ColumnCoordinate } from "./types.ts";

export interface AsyncChunkGenerationCompletionStats {
  cacheHits: number;
  generated: number;
}

export interface AsyncChunkSummaryCompletionStats {
  cacheHits: number;
  generated: number;
}

export interface AsyncChunkGenerationQueue {
  requestChunk(cx: number, cy: number, cz: number): boolean;
  requestSummary(cx: number, cy: number, cz: number): boolean;
  requestColumnSummary(cx: number, cz: number): boolean;
  hasPendingChunk(cx: number, cy: number, cz: number): boolean;
  hasPendingColumnSummary(cx: number, cz: number): boolean;
  getPendingCount(): number;
  drainCompletedChunks(): GeneratedChunk[];
  drainCompletedSummaries(): GeneratedChunkRenderSummary[];
  drainCompletedColumnSummaries(): GeneratedRenderColumnSummary[];
  drainMissingColumnSummaries(): ColumnCoordinate[];
  drainCompletionStats(): AsyncChunkGenerationCompletionStats;
  drainSummaryCompletionStats(): AsyncChunkSummaryCompletionStats;
  dispose(): void;
}

export function toAsyncChunkGenerationKey(cx: number, cy: number, cz: number): string {
  return `${cx}:${cy}:${cz}`;
}

export function toAsyncColumnSummaryKey(cx: number, cz: number): string {
  return `column:${cx}:${cz}`;
}
