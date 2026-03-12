import type { GeneratedChunkRenderSummary } from "./generated-chunk-render-summary.ts";
import type { GeneratedRenderColumnSummary } from "./generated-render-column-summary.ts";
import type {
  GeneratedRenderSummaryRegion,
  GeneratedRenderSummaryRegionColumn,
} from "./generated-render-summary-region.ts";
import type { GeneratedChunk } from "./procedural-generator.ts";
import { decodeGeneratedChunk, encodeGeneratedChunk, type GeneratedChunkCodecStats } from "./generated-chunk-codec.ts";

export interface TransferredGeneratedChunk {
  encodedBuffer: ArrayBuffer;
  encodedByteLength: number;
  codecStats?: GeneratedChunkCodecStats;
}

export interface TransferredGeneratedChunkRenderSummary {
  coord: GeneratedChunkRenderSummary["coord"];
  coveredColumnCount: number;
  surfaceY: Int32Array;
  surfaceMaterial: Uint16Array;
  waterTopY: Int32Array;
  waterMaterial: Uint16Array;
  macroCellSize: number;
  macroCellsPerAxis: number;
  macroCellStates: Uint8Array;
  faceOpenMask: Uint8Array;
}

export interface TransferredGeneratedRenderColumnSummary {
  chunkX: number;
  chunkZ: number;
  coveredColumnCount: number;
  surfaceY: Int32Array;
  surfaceMaterial: Uint16Array;
  waterTopY: Int32Array;
  waterMaterial: Uint16Array;
  minKnownCy: number;
  maxKnownCy: number;
  minNonEmptyCy: number | null;
  maxNonEmptyCy: number | null;
}

export interface TransferredGeneratedRenderSummaryRegionColumn {
  chunkX: number;
  chunkZ: number;
  summary: TransferredGeneratedRenderColumnSummary;
}

export interface TransferredGeneratedRenderSummaryRegion {
  regionX: number;
  regionZ: number;
  regionSizeChunks: number;
  columns: TransferredGeneratedRenderSummaryRegionColumn[];
}

export function serializeGeneratedChunk(chunk: GeneratedChunk): {
  chunk: TransferredGeneratedChunk;
  transfer: Transferable[];
} {
  const encoded = encodeGeneratedChunk(chunk);
  return {
    chunk: {
      encodedBuffer: encoded.buffer,
      encodedByteLength: encoded.stats.byteLength,
      codecStats: encoded.stats,
    },
    transfer: [encoded.buffer],
  };
}

export function deserializeGeneratedChunk(chunk: TransferredGeneratedChunk): GeneratedChunk {
  return decodeGeneratedChunk(chunk.encodedBuffer);
}

export function serializeGeneratedChunkRenderSummary(summary: GeneratedChunkRenderSummary): {
  summary: TransferredGeneratedChunkRenderSummary;
  transfer: Transferable[];
} {
  return {
    summary: {
      coord: { ...summary.coord },
      coveredColumnCount: summary.coveredColumnCount,
      surfaceY: summary.surfaceY,
      surfaceMaterial: summary.surfaceMaterial,
      waterTopY: summary.waterTopY,
      waterMaterial: summary.waterMaterial,
      macroCellSize: summary.macroCellSize,
      macroCellsPerAxis: summary.macroCellsPerAxis,
      macroCellStates: summary.macroCellStates,
      faceOpenMask: summary.faceOpenMask,
    },
    transfer: [
      summary.surfaceY.buffer,
      summary.surfaceMaterial.buffer,
      summary.waterTopY.buffer,
      summary.waterMaterial.buffer,
      summary.macroCellStates.buffer,
      summary.faceOpenMask.buffer,
    ],
  };
}

export function deserializeGeneratedChunkRenderSummary(
  summary: TransferredGeneratedChunkRenderSummary,
): GeneratedChunkRenderSummary {
  return {
    coord: { ...summary.coord },
    coveredColumnCount: summary.coveredColumnCount,
    surfaceY: summary.surfaceY,
    surfaceMaterial: summary.surfaceMaterial,
    waterTopY: summary.waterTopY,
    waterMaterial: summary.waterMaterial,
    macroCellSize: summary.macroCellSize,
    macroCellsPerAxis: summary.macroCellsPerAxis,
    macroCellStates: summary.macroCellStates,
    faceOpenMask: summary.faceOpenMask,
  };
}

export function serializeGeneratedRenderColumnSummary(summary: GeneratedRenderColumnSummary): {
  summary: TransferredGeneratedRenderColumnSummary;
  transfer: Transferable[];
} {
  return {
    summary: {
      chunkX: summary.chunkX,
      chunkZ: summary.chunkZ,
      coveredColumnCount: summary.coveredColumnCount,
      surfaceY: summary.surfaceY,
      surfaceMaterial: summary.surfaceMaterial,
      waterTopY: summary.waterTopY,
      waterMaterial: summary.waterMaterial,
      minKnownCy: summary.minKnownCy,
      maxKnownCy: summary.maxKnownCy,
      minNonEmptyCy: summary.minNonEmptyCy,
      maxNonEmptyCy: summary.maxNonEmptyCy,
    },
    transfer: [
      summary.surfaceY.buffer,
      summary.surfaceMaterial.buffer,
      summary.waterTopY.buffer,
      summary.waterMaterial.buffer,
    ],
  };
}

export function deserializeGeneratedRenderColumnSummary(
  summary: TransferredGeneratedRenderColumnSummary,
): GeneratedRenderColumnSummary {
  return {
    chunkX: summary.chunkX,
    chunkZ: summary.chunkZ,
    coveredColumnCount: summary.coveredColumnCount,
    surfaceY: summary.surfaceY,
    surfaceMaterial: summary.surfaceMaterial,
    waterTopY: summary.waterTopY,
    waterMaterial: summary.waterMaterial,
    minKnownCy: summary.minKnownCy,
    maxKnownCy: summary.maxKnownCy,
    minNonEmptyCy: summary.minNonEmptyCy,
    maxNonEmptyCy: summary.maxNonEmptyCy,
  };
}

export function serializeGeneratedRenderSummaryRegion(region: GeneratedRenderSummaryRegion): {
  summary: TransferredGeneratedRenderSummaryRegion;
  transfer: Transferable[];
} {
  const columns: TransferredGeneratedRenderSummaryRegionColumn[] = [];
  const transfer: Transferable[] = [];
  for (const entry of region.columns) {
    const serialized = serializeGeneratedRenderColumnSummary(entry.summary);
    columns.push({
      chunkX: entry.chunkX,
      chunkZ: entry.chunkZ,
      summary: serialized.summary,
    });
    transfer.push(...serialized.transfer);
  }
  return {
    summary: {
      regionX: region.regionX,
      regionZ: region.regionZ,
      regionSizeChunks: region.regionSizeChunks,
      columns,
    },
    transfer,
  };
}

export function deserializeGeneratedRenderSummaryRegion(
  region: TransferredGeneratedRenderSummaryRegion,
): GeneratedRenderSummaryRegion {
  const columns: GeneratedRenderSummaryRegionColumn[] = region.columns.map((entry) => ({
    chunkX: entry.chunkX,
    chunkZ: entry.chunkZ,
    summary: deserializeGeneratedRenderColumnSummary(entry.summary),
  }));
  return {
    regionX: region.regionX,
    regionZ: region.regionZ,
    regionSizeChunks: region.regionSizeChunks,
    columns,
  };
}
