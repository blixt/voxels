import type { ChunkCoordinate, PackedColor } from "./types.ts";

export interface VoxelChunk {
  coord: ChunkCoordinate;
  lodLevel: number;
  voxelStride: number;
  data: Uint16Array;
  solidCount: number;
  solidBounds: {
    min: [number, number, number];
    max: [number, number, number];
    dirty: boolean;
  } | null;
  meshBuilt: boolean;
  meshDirty: boolean;
  renderReady: boolean;
  meshRevision: number;
  pendingMeshRevision: number | null;
  gpuDirty: boolean;
  mesh: import("./types.ts").ChunkMeshData | null;
}

export interface ResidentChunkWorld {
  readonly chunkSize: number;
  readonly minY: number;
  readonly maxYExclusive: number;
  getVoxel(x: number, y: number, z: number): number;
  getPaletteColor(materialIndex: number): PackedColor;
  isCollisionMaterial(materialIndex: number): boolean;
  isWaterMaterial(materialIndex: number): boolean;
  getResidentChunk(cx: number, cy: number, cz: number): VoxelChunk | null;
  hasResidentChunk(cx: number, cy: number, cz: number): boolean;
  iterateResidentChunks(): Iterable<VoxelChunk>;
  iterateDirtyResidentChunks?(): Iterable<VoxelChunk>;
  noteResidentChunkMeshDirtyState?(chunk: VoxelChunk, dirty: boolean): void;
  noteResidentChunkRenderReadyState?(chunk: VoxelChunk, renderReady: boolean): void;
  getChunkSolidBounds(
    cx: number,
    cy: number,
    cz: number,
  ): {
    min: [number, number, number];
    max: [number, number, number];
  } | null;
}

export interface MutableResidentChunkWorld extends ResidentChunkWorld {
  setVoxel(x: number, y: number, z: number, materialIndex: number): boolean;
}

export function setChunkMeshDirtyState(world: ResidentChunkWorld, chunk: VoxelChunk, dirty: boolean): void {
  if (chunk.meshDirty === dirty) {
    return;
  }
  chunk.meshDirty = dirty;
  world.noteResidentChunkMeshDirtyState?.(chunk, dirty);
}
