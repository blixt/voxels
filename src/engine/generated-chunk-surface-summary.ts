import type { ChunkCoordinate } from "./types.ts";

export const NO_GENERATED_SURFACE_HEIGHT = -0x7fff_ffff;
export const NO_GENERATED_WATER_HEIGHT = -0x7fff_ffff;

export interface GeneratedChunkSurfaceSummary {
  readonly coord: ChunkCoordinate;
  readonly coveredColumnCount: number;
  readonly surfaceY: Int32Array;
  readonly surfaceMaterial: Uint16Array;
  readonly waterTopY: Int32Array;
  readonly waterMaterial: Uint16Array;
}

export interface GeneratedSurfaceColumnSample {
  readonly surfaceY: number;
  readonly surfaceMaterial: number;
  readonly waterTopY: number | null;
  readonly waterMaterial: number | null;
}

export function summarizeGeneratedChunkSurface(
  coord: ChunkCoordinate,
  data: Uint16Array,
  chunkSize: number,
  isWaterMaterial: (materialIndex: number) => boolean,
): GeneratedChunkSurfaceSummary | null {
  const chunkArea = chunkSize * chunkSize;
  const surfaceY = new Int32Array(chunkArea);
  const surfaceMaterial = new Uint16Array(chunkArea);
  const waterTopY = new Int32Array(chunkArea);
  const waterMaterial = new Uint16Array(chunkArea);
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

  if (coveredColumnCount === 0) {
    return null;
  }
  return {
    coord: { ...coord },
    coveredColumnCount,
    surfaceY,
    surfaceMaterial,
    waterTopY,
    waterMaterial,
  };
}

export function sampleGeneratedChunkSurface(
  summary: GeneratedChunkSurfaceSummary,
  localX: number,
  localZ: number,
  chunkSize: number,
): GeneratedSurfaceColumnSample | null {
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
