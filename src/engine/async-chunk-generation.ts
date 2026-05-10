import type { GeneratedChunkRenderSummary } from "./generated-chunk-render-summary.ts";
import type { GeneratedRenderSummaryRegion } from "./generated-render-summary-region.ts";
import type { GeneratedChunk } from "./procedural-generator.ts";
import type { ChunkCoordinate, RenderSummaryRegionCoordinate } from "./types.ts";
import { formatChunkCoordinateKey } from "./coordinate-keys.ts";

export interface AsyncChunkGenerationCompletionStats {
  cacheHits: number;
  generated: number;
}

export interface AsyncChunkSummaryCompletionStats {
  cacheHits: number;
  generated: number;
}

export interface AsyncDerivedLodChunkCacheKey {
  readonly lodLevel: number;
  readonly coord: ChunkCoordinate;
  readonly editRevision: number;
}

export interface AsyncEncodedDerivedLodChunk {
  readonly key: AsyncDerivedLodChunkCacheKey;
  readonly source?: "cache" | "generated";
  readonly buffer: ArrayBuffer;
  readonly byteLength: number;
}

export interface AsyncDerivedLodChunkCompletionStats {
  cacheHits: number;
  generated: number;
  missing: number;
  stored: number;
}

export interface AsyncChunkGenerationQueue {
  requestChunk(cx: number, cy: number, cz: number): boolean;
  requestSummary(cx: number, cy: number, cz: number): boolean;
  requestRegionSummary(regionX: number, regionZ: number): boolean;
  requestLodChunk(key: AsyncDerivedLodChunkCacheKey): boolean;
  requestGeneratedLodChunk(key: AsyncDerivedLodChunkCacheKey): boolean;
  storeLodChunk(chunk: AsyncEncodedDerivedLodChunk): boolean;
  hasPendingChunk(cx: number, cy: number, cz: number): boolean;
  hasPendingRegionSummary(regionX: number, regionZ: number): boolean;
  hasPendingLodChunk(key: AsyncDerivedLodChunkCacheKey): boolean;
  hasPendingGeneratedLodChunk(key: AsyncDerivedLodChunkCacheKey): boolean;
  getWorkerCount?(): number;
  getPendingCount(): number;
  drainCompletedChunks(): GeneratedChunk[];
  drainCompletedSummaries(): GeneratedChunkRenderSummary[];
  drainCompletedRegionSummaries(): GeneratedRenderSummaryRegion[];
  drainMissingRegionSummaries(): RenderSummaryRegionCoordinate[];
  drainCompletedLodChunks(maxCount?: number): AsyncEncodedDerivedLodChunk[];
  drainMissingLodChunks(): AsyncDerivedLodChunkCacheKey[];
  getCompletedLodChunkCount?(): number;
  drainCompletionStats(): AsyncChunkGenerationCompletionStats;
  drainSummaryCompletionStats(): AsyncChunkSummaryCompletionStats;
  drainLodChunkCompletionStats(): AsyncDerivedLodChunkCompletionStats;
  dispose(): void;
}

export function toAsyncChunkGenerationKey(cx: number, cy: number, cz: number): string {
  return formatChunkCoordinateKey(cx, cy, cz);
}

export function toAsyncRegionSummaryKey(regionX: number, regionZ: number): string {
  return `region:${regionX}:${regionZ}`;
}

export function toAsyncLodChunkKey(key: AsyncDerivedLodChunkCacheKey): string {
  return `lod:${key.editRevision}:${key.lodLevel}:${key.coord.x}:${key.coord.y}:${key.coord.z}`;
}

export function toAsyncGeneratedLodChunkKey(key: AsyncDerivedLodChunkCacheKey): string {
  return `generate:${toAsyncLodChunkKey(key)}`;
}

export function toAsyncStoredLodChunkKey(key: AsyncDerivedLodChunkCacheKey): string {
  return `store:${toAsyncLodChunkKey(key)}`;
}

export function cloneAsyncDerivedLodChunkCacheKey(
  key: AsyncDerivedLodChunkCacheKey,
): AsyncDerivedLodChunkCacheKey {
  return {
    lodLevel: key.lodLevel,
    editRevision: key.editRevision,
    coord: { ...key.coord },
  };
}
