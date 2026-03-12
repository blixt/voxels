import type { GeneratedChunkRenderSummary } from "./generated-chunk-render-summary.ts";
import type { GeneratedRenderSummaryRegion } from "./generated-render-summary-region.ts";
import type { GeneratedChunk } from "./procedural-generator.ts";
import type { RenderSummaryRegionCoordinate } from "./types.ts";

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
  requestRegionSummary(regionX: number, regionZ: number): boolean;
  hasPendingChunk(cx: number, cy: number, cz: number): boolean;
  hasPendingRegionSummary(regionX: number, regionZ: number): boolean;
  getPendingCount(): number;
  drainCompletedChunks(): GeneratedChunk[];
  drainCompletedSummaries(): GeneratedChunkRenderSummary[];
  drainCompletedRegionSummaries(): GeneratedRenderSummaryRegion[];
  drainMissingRegionSummaries(): RenderSummaryRegionCoordinate[];
  drainCompletionStats(): AsyncChunkGenerationCompletionStats;
  drainSummaryCompletionStats(): AsyncChunkSummaryCompletionStats;
  dispose(): void;
}

export function toAsyncChunkGenerationKey(cx: number, cy: number, cz: number): string {
  return `${cx}:${cy}:${cz}`;
}

export function toAsyncRegionSummaryKey(regionX: number, regionZ: number): string {
  return `region:${regionX}:${regionZ}`;
}
