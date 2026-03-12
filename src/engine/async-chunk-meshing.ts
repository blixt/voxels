import type { OpaqueChunkMeshingInput, OpaqueChunkMeshGeometry } from "./opaque-chunk-mesher.ts";
import type { ChunkCoordinate } from "./types.ts";

export interface CompletedChunkMeshingJob {
  coord: ChunkCoordinate;
  meshRevision: number;
  opaqueMesh: OpaqueChunkMeshGeometry;
}

export interface AsyncChunkMeshingQueue {
  requestChunk(input: OpaqueChunkMeshingInput, meshRevision: number): boolean;
  hasPendingChunk(cx: number, cy: number, cz: number): boolean;
  getPendingCount(): number;
  drainCompletedMeshes(): CompletedChunkMeshingJob[];
  dispose(): void;
}

export function toAsyncChunkMeshingKey(cx: number, cy: number, cz: number): string {
  return `${cx}:${cy}:${cz}`;
}
