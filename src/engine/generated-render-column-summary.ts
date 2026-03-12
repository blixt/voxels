import {
  NO_GENERATED_SURFACE_HEIGHT,
  NO_GENERATED_WATER_HEIGHT,
  type GeneratedChunkRenderSummary,
  type GeneratedRenderColumnSample,
} from "./generated-chunk-render-summary.ts";

export interface GeneratedRenderColumnSummary {
  readonly chunkX: number;
  readonly chunkZ: number;
  readonly coveredColumnCount: number;
  readonly surfaceY: Int32Array;
  readonly surfaceMaterial: Uint16Array;
  readonly waterTopY: Int32Array;
  readonly waterMaterial: Uint16Array;
  readonly minKnownCy: number;
  readonly maxKnownCy: number;
  readonly minNonEmptyCy: number | null;
  readonly maxNonEmptyCy: number | null;
}

export function summarizeGeneratedRenderColumn(
  chunkX: number,
  chunkZ: number,
  chunkSummaries: Iterable<GeneratedChunkRenderSummary>,
  chunkSize: number,
): GeneratedRenderColumnSummary | null {
  const chunkArea = chunkSize * chunkSize;
  const surfaceY = new Int32Array(chunkArea);
  const surfaceMaterial = new Uint16Array(chunkArea);
  const waterTopY = new Int32Array(chunkArea);
  const waterMaterial = new Uint16Array(chunkArea);
  surfaceY.fill(NO_GENERATED_SURFACE_HEIGHT);
  waterTopY.fill(NO_GENERATED_WATER_HEIGHT);

  let coveredColumnCount = 0;
  let minKnownCy = Number.POSITIVE_INFINITY;
  let maxKnownCy = Number.NEGATIVE_INFINITY;
  let minNonEmptyCy = Number.POSITIVE_INFINITY;
  let maxNonEmptyCy = Number.NEGATIVE_INFINITY;

  for (const summary of chunkSummaries) {
    minKnownCy = Math.min(minKnownCy, summary.coord.y);
    maxKnownCy = Math.max(maxKnownCy, summary.coord.y);
    const nonEmpty = summary.macroCellStates.some((state) => state !== 0);
    if (nonEmpty) {
      minNonEmptyCy = Math.min(minNonEmptyCy, summary.coord.y);
      maxNonEmptyCy = Math.max(maxNonEmptyCy, summary.coord.y);
    }
    if (summary.coveredColumnCount === 0) {
      continue;
    }
    for (let columnIndex = 0; columnIndex < chunkArea; columnIndex += 1) {
      const sampledSurfaceY = summary.surfaceY[columnIndex] ?? NO_GENERATED_SURFACE_HEIGHT;
      if (sampledSurfaceY !== NO_GENERATED_SURFACE_HEIGHT) {
        if (surfaceY[columnIndex] === NO_GENERATED_SURFACE_HEIGHT) {
          coveredColumnCount += 1;
        }
        if (sampledSurfaceY > surfaceY[columnIndex]!) {
          surfaceY[columnIndex] = sampledSurfaceY;
          surfaceMaterial[columnIndex] = summary.surfaceMaterial[columnIndex] ?? 0;
        }
      }
      const sampledWaterTopY = summary.waterTopY[columnIndex] ?? NO_GENERATED_WATER_HEIGHT;
      if (sampledWaterTopY > waterTopY[columnIndex]!) {
        waterTopY[columnIndex] = sampledWaterTopY;
        waterMaterial[columnIndex] = summary.waterMaterial[columnIndex] ?? 0;
      }
    }
  }

  if (!Number.isFinite(minKnownCy) || !Number.isFinite(maxKnownCy)) {
    return null;
  }

  return {
    chunkX,
    chunkZ,
    coveredColumnCount,
    surfaceY: coveredColumnCount === 0 ? new Int32Array(0) : surfaceY,
    surfaceMaterial: coveredColumnCount === 0 ? new Uint16Array(0) : surfaceMaterial,
    waterTopY: coveredColumnCount === 0 ? new Int32Array(0) : waterTopY,
    waterMaterial: coveredColumnCount === 0 ? new Uint16Array(0) : waterMaterial,
    minKnownCy,
    maxKnownCy,
    minNonEmptyCy: Number.isFinite(minNonEmptyCy) ? minNonEmptyCy : null,
    maxNonEmptyCy: Number.isFinite(maxNonEmptyCy) ? maxNonEmptyCy : null,
  };
}

export function mergeGeneratedRenderColumnSummary(
  existing: GeneratedRenderColumnSummary | null,
  chunkSummary: GeneratedChunkRenderSummary,
  chunkSize: number,
): GeneratedRenderColumnSummary {
  const chunkArea = chunkSize * chunkSize;
  const surfaceY = existing?.coveredColumnCount
    ? existing.surfaceY.slice()
    : new Int32Array(chunkArea).fill(NO_GENERATED_SURFACE_HEIGHT);
  const surfaceMaterial = existing?.coveredColumnCount ? existing.surfaceMaterial.slice() : new Uint16Array(chunkArea);
  const waterTopY = existing?.coveredColumnCount
    ? existing.waterTopY.slice()
    : new Int32Array(chunkArea).fill(NO_GENERATED_WATER_HEIGHT);
  const waterMaterial = existing?.coveredColumnCount ? existing.waterMaterial.slice() : new Uint16Array(chunkArea);

  let coveredColumnCount = existing?.coveredColumnCount ?? 0;
  let minKnownCy = existing ? Math.min(existing.minKnownCy, chunkSummary.coord.y) : chunkSummary.coord.y;
  let maxKnownCy = existing ? Math.max(existing.maxKnownCy, chunkSummary.coord.y) : chunkSummary.coord.y;
  let minNonEmptyCy = existing?.minNonEmptyCy ?? null;
  let maxNonEmptyCy = existing?.maxNonEmptyCy ?? null;
  const nonEmpty = chunkSummary.macroCellStates.some((state) => state !== 0);
  if (nonEmpty) {
    minNonEmptyCy = minNonEmptyCy === null ? chunkSummary.coord.y : Math.min(minNonEmptyCy, chunkSummary.coord.y);
    maxNonEmptyCy = maxNonEmptyCy === null ? chunkSummary.coord.y : Math.max(maxNonEmptyCy, chunkSummary.coord.y);
  }

  if (chunkSummary.coveredColumnCount > 0) {
    for (let columnIndex = 0; columnIndex < chunkArea; columnIndex += 1) {
      const sampledSurfaceY = chunkSummary.surfaceY[columnIndex] ?? NO_GENERATED_SURFACE_HEIGHT;
      if (sampledSurfaceY !== NO_GENERATED_SURFACE_HEIGHT) {
        if (surfaceY[columnIndex] === NO_GENERATED_SURFACE_HEIGHT) {
          coveredColumnCount += 1;
        }
        if (sampledSurfaceY > surfaceY[columnIndex]!) {
          surfaceY[columnIndex] = sampledSurfaceY;
          surfaceMaterial[columnIndex] = chunkSummary.surfaceMaterial[columnIndex] ?? 0;
        }
      }
      const sampledWaterTopY = chunkSummary.waterTopY[columnIndex] ?? NO_GENERATED_WATER_HEIGHT;
      if (sampledWaterTopY > waterTopY[columnIndex]!) {
        waterTopY[columnIndex] = sampledWaterTopY;
        waterMaterial[columnIndex] = chunkSummary.waterMaterial[columnIndex] ?? 0;
      }
    }
  }

  return {
    chunkX: existing?.chunkX ?? chunkSummary.coord.x,
    chunkZ: existing?.chunkZ ?? chunkSummary.coord.z,
    coveredColumnCount,
    surfaceY: coveredColumnCount === 0 ? new Int32Array(0) : surfaceY,
    surfaceMaterial: coveredColumnCount === 0 ? new Uint16Array(0) : surfaceMaterial,
    waterTopY: coveredColumnCount === 0 ? new Int32Array(0) : waterTopY,
    waterMaterial: coveredColumnCount === 0 ? new Uint16Array(0) : waterMaterial,
    minKnownCy,
    maxKnownCy,
    minNonEmptyCy,
    maxNonEmptyCy,
  };
}

export function sampleGeneratedRenderColumnSummary(
  summary: GeneratedRenderColumnSummary,
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
