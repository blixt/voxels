import type { GeneratedChunkRenderSummary } from "./generated-chunk-render-summary.ts";
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
