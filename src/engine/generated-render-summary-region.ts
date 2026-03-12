import type { GeneratedRenderColumnSummary } from "./generated-render-column-summary.ts";

export const GENERATED_RENDER_SUMMARY_REGION_SIZE_CHUNKS = 4;

export interface GeneratedRenderSummaryRegionColumn {
  readonly chunkX: number;
  readonly chunkZ: number;
  readonly summary: GeneratedRenderColumnSummary;
}

export interface GeneratedRenderSummaryRegion {
  readonly regionX: number;
  readonly regionZ: number;
  readonly regionSizeChunks: number;
  readonly columns: readonly GeneratedRenderSummaryRegionColumn[];
}

export function getGeneratedRenderSummaryRegionCoord(
  chunkX: number,
  chunkZ: number,
  regionSizeChunks = GENERATED_RENDER_SUMMARY_REGION_SIZE_CHUNKS,
): {
  x: number;
  z: number;
} {
  return {
    x: floorDiv(chunkX, regionSizeChunks),
    z: floorDiv(chunkZ, regionSizeChunks),
  };
}

export function upsertGeneratedRenderSummaryRegion(
  existing: GeneratedRenderSummaryRegion | null,
  summary: GeneratedRenderColumnSummary,
  regionSizeChunks = GENERATED_RENDER_SUMMARY_REGION_SIZE_CHUNKS,
): GeneratedRenderSummaryRegion {
  const regionCoord = getGeneratedRenderSummaryRegionCoord(summary.chunkX, summary.chunkZ, regionSizeChunks);
  if (
    existing
    && (
      existing.regionX !== regionCoord.x
      || existing.regionZ !== regionCoord.z
      || existing.regionSizeChunks !== regionSizeChunks
    )
  ) {
    throw new Error(
      `Cannot merge column ${summary.chunkX}:${summary.chunkZ} into render summary region `
        + `${existing.regionX}:${existing.regionZ} sized ${existing.regionSizeChunks}`,
    );
  }

  const nextColumns = existing ? [...existing.columns] : [];
  const nextEntry: GeneratedRenderSummaryRegionColumn = {
    chunkX: summary.chunkX,
    chunkZ: summary.chunkZ,
    summary,
  };
  const index = nextColumns.findIndex((entry) => entry.chunkX === summary.chunkX && entry.chunkZ === summary.chunkZ);
  if (index >= 0) {
    nextColumns[index] = nextEntry;
  } else {
    nextColumns.push(nextEntry);
    nextColumns.sort((left, right) => left.chunkZ - right.chunkZ || left.chunkX - right.chunkX);
  }

  return {
    regionX: regionCoord.x,
    regionZ: regionCoord.z,
    regionSizeChunks,
    columns: nextColumns,
  };
}

function floorDiv(value: number, divisor: number): number {
  return Math.floor(value / divisor);
}
