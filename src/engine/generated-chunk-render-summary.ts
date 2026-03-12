import type { ChunkCoordinate } from "./types.ts";

export const NO_GENERATED_SURFACE_HEIGHT = -0x7fff_ffff;
export const NO_GENERATED_WATER_HEIGHT = -0x7fff_ffff;
export const GENERATED_CHUNK_RENDER_CELL_SIZE = 8;

export const GENERATED_CHUNK_RENDER_CELL_EMPTY = 0;
export const GENERATED_CHUNK_RENDER_CELL_MIXED = 1;
export const GENERATED_CHUNK_RENDER_CELL_SOLID = 2;

export type GeneratedChunkRenderCellState =
  | typeof GENERATED_CHUNK_RENDER_CELL_EMPTY
  | typeof GENERATED_CHUNK_RENDER_CELL_MIXED
  | typeof GENERATED_CHUNK_RENDER_CELL_SOLID;

export type GeneratedChunkRenderFace = "west" | "east" | "down" | "up" | "north" | "south";

export interface GeneratedChunkRenderSummary {
  readonly coord: ChunkCoordinate;
  readonly coveredColumnCount: number;
  readonly surfaceY: Int32Array;
  readonly surfaceMaterial: Uint16Array;
  readonly waterTopY: Int32Array;
  readonly waterMaterial: Uint16Array;
  readonly macroCellSize: number;
  readonly macroCellsPerAxis: number;
  readonly macroCellStates: Uint8Array;
  readonly faceOpenMask: Uint8Array;
}

export interface GeneratedRenderColumnSample {
  readonly surfaceY: number;
  readonly surfaceMaterial: number;
  readonly waterTopY: number | null;
  readonly waterMaterial: number | null;
}

export function summarizeGeneratedChunkRender(
  coord: ChunkCoordinate,
  data: Uint16Array,
  chunkSize: number,
  isWaterMaterial: (materialIndex: number) => boolean,
  macroCellSize = GENERATED_CHUNK_RENDER_CELL_SIZE,
): GeneratedChunkRenderSummary {
  if (chunkSize % macroCellSize !== 0) {
    throw new Error(`Chunk size ${chunkSize} is not divisible by render summary cell size ${macroCellSize}`);
  }
  const chunkArea = chunkSize * chunkSize;
  const macroCellsPerAxis = chunkSize / macroCellSize;
  const macroCellVolume = macroCellsPerAxis * macroCellsPerAxis * macroCellsPerAxis;
  const macroFaceArea = macroCellsPerAxis * macroCellsPerAxis;
  const surfaceY = new Int32Array(chunkArea);
  const surfaceMaterial = new Uint16Array(chunkArea);
  const waterTopY = new Int32Array(chunkArea);
  const waterMaterial = new Uint16Array(chunkArea);
  const macroCellStates = new Uint8Array(macroCellVolume);
  const faceOpenMask = new Uint8Array(macroFaceArea * GENERATED_CHUNK_FACE_COUNT);
  surfaceY.fill(NO_GENERATED_SURFACE_HEIGHT);
  waterTopY.fill(NO_GENERATED_WATER_HEIGHT);
  const worldOriginY = coord.y * chunkSize;
  let coveredColumnCount = 0;

  for (let localZ = 0; localZ < chunkSize; localZ += 1) {
    for (let localX = 0; localX < chunkSize; localX += 1) {
      const columnIndex = localX + localZ * chunkSize;
      let topSolidWorldY = NO_GENERATED_SURFACE_HEIGHT;
      let topSolidMaterial = 0;
      let topWaterWorldY = NO_GENERATED_WATER_HEIGHT;
      let topWaterMaterial = 0;
      for (let localY = chunkSize - 1; localY >= 0; localY -= 1) {
        const materialIndex = data[localX + localY * chunkSize + localZ * chunkArea] ?? 0;
        if (materialIndex === 0) {
          continue;
        }
        const worldY = worldOriginY + localY;
        if (isWaterMaterial(materialIndex)) {
          if (topWaterWorldY === NO_GENERATED_WATER_HEIGHT) {
            topWaterWorldY = worldY;
            topWaterMaterial = materialIndex;
          }
          continue;
        }
        topSolidWorldY = worldY;
        topSolidMaterial = materialIndex;
        break;
      }
      if (topSolidWorldY === NO_GENERATED_SURFACE_HEIGHT && topWaterWorldY === NO_GENERATED_WATER_HEIGHT) {
        continue;
      }
      coveredColumnCount += 1;
      surfaceY[columnIndex] = topSolidWorldY;
      surfaceMaterial[columnIndex] = topSolidMaterial;
      waterTopY[columnIndex] = topWaterWorldY;
      waterMaterial[columnIndex] = topWaterMaterial;
    }
  }

  const macroVoxelVolume = macroCellSize * macroCellSize * macroCellSize;
  for (let cellZ = 0; cellZ < macroCellsPerAxis; cellZ += 1) {
    const startZ = cellZ * macroCellSize;
    for (let cellY = 0; cellY < macroCellsPerAxis; cellY += 1) {
      const startY = cellY * macroCellSize;
      for (let cellX = 0; cellX < macroCellsPerAxis; cellX += 1) {
        const startX = cellX * macroCellSize;
        let occupiedVoxelCount = 0;
        let westOpen = false;
        let eastOpen = false;
        let downOpen = false;
        let upOpen = false;
        let northOpen = false;
        let southOpen = false;
        for (let localZ = 0; localZ < macroCellSize; localZ += 1) {
          const voxelZ = startZ + localZ;
          for (let localY = 0; localY < macroCellSize; localY += 1) {
            const voxelY = startY + localY;
            const rowOffset = voxelY * chunkSize + voxelZ * chunkArea;
            for (let localX = 0; localX < macroCellSize; localX += 1) {
              const voxelX = startX + localX;
              const materialIndex = data[voxelX + rowOffset] ?? 0;
              const occupied = materialIndex !== 0;
              if (occupied) {
                occupiedVoxelCount += 1;
                continue;
              }
              if (cellX === 0 && localX === 0) {
                westOpen = true;
              }
              if (cellX === macroCellsPerAxis - 1 && localX === macroCellSize - 1) {
                eastOpen = true;
              }
              if (cellY === 0 && localY === 0) {
                downOpen = true;
              }
              if (cellY === macroCellsPerAxis - 1 && localY === macroCellSize - 1) {
                upOpen = true;
              }
              if (cellZ === 0 && localZ === 0) {
                northOpen = true;
              }
              if (cellZ === macroCellsPerAxis - 1 && localZ === macroCellSize - 1) {
                southOpen = true;
              }
            }
          }
        }
        const macroCellIndex = toMacroCellIndex(cellX, cellY, cellZ, macroCellsPerAxis);
        macroCellStates[macroCellIndex] = occupiedVoxelCount === 0
          ? GENERATED_CHUNK_RENDER_CELL_EMPTY
          : (occupiedVoxelCount === macroVoxelVolume ? GENERATED_CHUNK_RENDER_CELL_SOLID : GENERATED_CHUNK_RENDER_CELL_MIXED);
        if (westOpen) {
          faceOpenMask[faceMaskOffset("west", macroCellsPerAxis) + faceCellIndex(cellY, cellZ, macroCellsPerAxis)] = 1;
        }
        if (eastOpen) {
          faceOpenMask[faceMaskOffset("east", macroCellsPerAxis) + faceCellIndex(cellY, cellZ, macroCellsPerAxis)] = 1;
        }
        if (downOpen) {
          faceOpenMask[faceMaskOffset("down", macroCellsPerAxis) + faceCellIndex(cellX, cellZ, macroCellsPerAxis)] = 1;
        }
        if (upOpen) {
          faceOpenMask[faceMaskOffset("up", macroCellsPerAxis) + faceCellIndex(cellX, cellZ, macroCellsPerAxis)] = 1;
        }
        if (northOpen) {
          faceOpenMask[faceMaskOffset("north", macroCellsPerAxis) + faceCellIndex(cellX, cellY, macroCellsPerAxis)] = 1;
        }
        if (southOpen) {
          faceOpenMask[faceMaskOffset("south", macroCellsPerAxis) + faceCellIndex(cellX, cellY, macroCellsPerAxis)] = 1;
        }
      }
    }
  }

  return {
    coord: { ...coord },
    coveredColumnCount,
    surfaceY: coveredColumnCount === 0 ? new Int32Array(0) : surfaceY,
    surfaceMaterial: coveredColumnCount === 0 ? new Uint16Array(0) : surfaceMaterial,
    waterTopY: coveredColumnCount === 0 ? new Int32Array(0) : waterTopY,
    waterMaterial: coveredColumnCount === 0 ? new Uint16Array(0) : waterMaterial,
    macroCellSize,
    macroCellsPerAxis,
    macroCellStates,
    faceOpenMask,
  };
}

export function sampleGeneratedChunkRenderSurface(
  summary: GeneratedChunkRenderSummary,
  localX: number,
  localZ: number,
  chunkSize: number,
): GeneratedRenderColumnSample | null {
  if (summary.coveredColumnCount === 0) {
    return null;
  }
  const columnIndex = localX + localZ * chunkSize;
  const sampledSurfaceY = summary.surfaceY[columnIndex] ?? NO_GENERATED_SURFACE_HEIGHT;
  const sampledWaterTopY = summary.waterTopY[columnIndex] ?? NO_GENERATED_WATER_HEIGHT;
  if (sampledSurfaceY === NO_GENERATED_SURFACE_HEIGHT) {
    return null;
  }
  return {
    surfaceY: sampledSurfaceY,
    surfaceMaterial: summary.surfaceMaterial[columnIndex] ?? 0,
    waterTopY: sampledWaterTopY === NO_GENERATED_WATER_HEIGHT ? null : sampledWaterTopY,
    waterMaterial: sampledWaterTopY === NO_GENERATED_WATER_HEIGHT ? null : (summary.waterMaterial[columnIndex] ?? 0),
  };
}

export function getGeneratedChunkRenderCellState(
  summary: GeneratedChunkRenderSummary,
  cellX: number,
  cellY: number,
  cellZ: number,
): GeneratedChunkRenderCellState {
  return (summary.macroCellStates[toMacroCellIndex(cellX, cellY, cellZ, summary.macroCellsPerAxis)]
    ?? GENERATED_CHUNK_RENDER_CELL_EMPTY) as GeneratedChunkRenderCellState;
}

export function isGeneratedChunkFaceOpen(
  summary: GeneratedChunkRenderSummary,
  face: GeneratedChunkRenderFace,
  faceU: number,
  faceV: number,
): boolean {
  const offset = faceMaskOffset(face, summary.macroCellsPerAxis);
  return summary.faceOpenMask[offset + faceCellIndex(faceU, faceV, summary.macroCellsPerAxis)] !== 0;
}

const GENERATED_CHUNK_FACE_ORDER: readonly GeneratedChunkRenderFace[] = ["west", "east", "down", "up", "north", "south"] as const;
const GENERATED_CHUNK_FACE_COUNT = GENERATED_CHUNK_FACE_ORDER.length;

function toMacroCellIndex(cellX: number, cellY: number, cellZ: number, macroCellsPerAxis: number): number {
  return cellX + cellY * macroCellsPerAxis + cellZ * macroCellsPerAxis * macroCellsPerAxis;
}

function faceCellIndex(faceU: number, faceV: number, macroCellsPerAxis: number): number {
  return faceU + faceV * macroCellsPerAxis;
}

function faceMaskOffset(face: GeneratedChunkRenderFace, macroCellsPerAxis: number): number {
  const faceIndex = GENERATED_CHUNK_FACE_ORDER.indexOf(face);
  if (faceIndex < 0) {
    throw new Error(`Unknown generated chunk render face ${face}`);
  }
  return faceIndex * macroCellsPerAxis * macroCellsPerAxis;
}
