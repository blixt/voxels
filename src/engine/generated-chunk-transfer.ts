import type { ChunkCoordinate } from "./types.ts";
import type { GeneratedChunk } from "./procedural-generator.ts";

export interface TransferredGeneratedChunk {
  coord: ChunkCoordinate;
  dataBuffer: ArrayBuffer;
  solidCount: number;
  solidBounds: {
    min: [number, number, number];
    max: [number, number, number];
  } | null;
}

export function serializeGeneratedChunk(chunk: GeneratedChunk): {
  chunk: TransferredGeneratedChunk;
  transfer: Transferable[];
} {
  const dataBytes = new Uint8Array(chunk.data.byteLength);
  dataBytes.set(new Uint8Array(chunk.data.buffer, chunk.data.byteOffset, chunk.data.byteLength));
  const dataBuffer = dataBytes.buffer;
  return {
    chunk: {
      coord: { ...chunk.coord },
      dataBuffer,
      solidCount: chunk.solidCount,
      solidBounds: chunk.solidBounds
        ? {
            min: [...chunk.solidBounds.min],
            max: [...chunk.solidBounds.max],
          }
        : null,
    },
    transfer: [dataBuffer],
  };
}

export function deserializeGeneratedChunk(chunk: TransferredGeneratedChunk): GeneratedChunk {
  return {
    coord: { ...chunk.coord },
    data: new Uint16Array(chunk.dataBuffer),
    solidCount: chunk.solidCount,
    solidBounds: chunk.solidBounds
      ? {
          min: [...chunk.solidBounds.min],
          max: [...chunk.solidBounds.max],
        }
      : null,
  };
}
