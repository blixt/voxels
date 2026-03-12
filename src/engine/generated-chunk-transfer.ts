import type { GeneratedChunk } from "./procedural-generator.ts";
import { decodeGeneratedChunk, encodeGeneratedChunk, type GeneratedChunkCodecStats } from "./generated-chunk-codec.ts";

export interface TransferredGeneratedChunk {
  encodedBuffer: ArrayBuffer;
  encodedByteLength: number;
  codecStats?: GeneratedChunkCodecStats;
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
