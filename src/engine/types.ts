export type Vec3 = [number, number, number];
export type PackedColor = number;

export interface ChunkCoordinate {
  x: number;
  y: number;
  z: number;
}

export interface RenderSummaryRegionCoordinate {
  x: number;
  z: number;
}

export interface ChunkBounds {
  min: Vec3;
  max: Vec3;
}

export interface ChunkMeshData {
  vertexData: ArrayBuffer;
  vertexCount: number;
  indexData: Uint32Array;
  indexCount: number;
  waterVertexData: ArrayBuffer;
  waterVertexCount: number;
  waterIndexData: Uint32Array;
  waterIndexCount: number;
  waterTriangleCount: number;
  triangleCount: number;
  bounds: ChunkBounds;
}

export interface WorldStats {
  solidVoxelCount: number;
  chunkCount: number;
  paletteCount: number;
}
