export type Vec3 = [number, number, number];
export type PackedColor = number;

export interface WorldDimensions {
  width: number;
  height: number;
  depth: number;
}

export interface ChunkCoordinate {
  x: number;
  y: number;
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

export interface SceneBuildResult {
  name: string;
  world: import("./world.ts").VoxelWorld;
  notes?: string[];
  camera?: SceneCameraPreset;
}

export interface SceneCameraPreset {
  target: Vec3;
  yaw: number;
  pitch: number;
  distance: number;
  zoom: number;
}

export type SceneKind = "performance" | "validation";

export interface RenderValidationMetrics {
  meanAbsoluteError: number;
  maxAbsoluteError: number;
  coverageMismatchRatio: number;
  renderChecksum: string;
  referenceChecksum: string;
  visualPass: boolean;
}

export interface RenderValidationArtifacts {
  actualDataUrl: string;
  referenceDataUrl: string;
  diffDataUrl: string;
  width: number;
  height: number;
}

export interface SceneBenchmarkSample {
  sceneName: string;
  sceneKind: SceneKind;
  iteration: number;
  buildMs: number;
  meshMs: number;
  firstFrameCpuMs: number;
  avgFrameCpuMs: number;
  avgWarmFrameCpuMs: number | null;
  firstFrameGpuMs: number | null;
  avgFrameGpuMs: number | null;
  avgWarmFrameGpuMs: number | null;
  firstFrameSyncMs: number;
  firstFrameUploadMs: number;
  firstFrameEncodeMs: number;
  firstFrameUploadChunks: number;
  firstFrameUploadBytes: number;
  triangles: number;
  drawCalls: number;
  solidVoxelCount: number;
  checksum: string;
  correctnessPass: boolean;
  visualPass: boolean | null;
  meanAbsoluteError: number | null;
  maxAbsoluteError: number | null;
  coverageMismatchRatio: number | null;
  renderChecksum: string | null;
  referenceChecksum: string | null;
}
