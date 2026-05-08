import type { ChunkCoordinate } from "./types.ts";

const CODEC_MAGIC = 0x3144_4f4c;
const CODEC_VERSION = 1;
const HEADER_BYTES = 42;

export interface DerivedLodChunkPayload {
  readonly coord: ChunkCoordinate;
  readonly lodLevel: number;
  readonly voxelStride: number;
  readonly data: Uint16Array;
  readonly solidCount: number;
  readonly solidBounds: {
    readonly min: [number, number, number];
    readonly max: [number, number, number];
  } | null;
}

export interface DerivedLodChunkCodecStats {
  readonly byteLength: number;
  readonly chunkSize: number;
  readonly voxelCount: number;
  readonly runCount: number;
  readonly zeroRunCount: number;
}

export interface EncodedDerivedLodChunk {
  readonly buffer: ArrayBuffer;
  readonly stats: DerivedLodChunkCodecStats;
}

interface DecodedHeader {
  coord: ChunkCoordinate;
  chunkSize: number;
  lodLevel: number;
  voxelStride: number;
  solidCount: number;
  solidBounds: DerivedLodChunkPayload["solidBounds"];
  voxelCount: number;
  runCount: number;
}

export function encodeDerivedLodChunk(chunk: DerivedLodChunkPayload): EncodedDerivedLodChunk {
  const chunkSize = inferChunkSize(chunk.data);
  const runs = collectRuns(chunk.data);
  const maxBytes = HEADER_BYTES + runs.length * 6;
  const buffer = new ArrayBuffer(maxBytes);
  const view = new DataView(buffer);
  writeHeader(view, chunk, chunkSize, runs.length);
  let byteOffset = HEADER_BYTES;
  let zeroRunCount = 0;
  for (const run of runs) {
    view.setUint16(byteOffset, run.material, true);
    byteOffset += 2;
    view.setUint32(byteOffset, run.length, true);
    byteOffset += 4;
    if (run.material === 0) {
      zeroRunCount += 1;
    }
  }
  return {
    buffer: buffer.slice(0, byteOffset),
    stats: {
      byteLength: byteOffset,
      chunkSize,
      voxelCount: chunk.data.length,
      runCount: runs.length,
      zeroRunCount,
    },
  };
}

export function decodeDerivedLodChunk(buffer: ArrayBuffer): DerivedLodChunkPayload {
  const view = new DataView(buffer);
  const header = readHeader(view);
  const data = new Uint16Array(header.voxelCount);
  let byteOffset = HEADER_BYTES;
  let cursor = 0;
  for (let runIndex = 0; runIndex < header.runCount; runIndex += 1) {
    const material = view.getUint16(byteOffset, true);
    byteOffset += 2;
    const length = view.getUint32(byteOffset, true);
    byteOffset += 4;
    if (cursor + length > data.length) {
      throw new Error("Derived LOD chunk run data exceeds declared voxel count");
    }
    data.fill(material, cursor, cursor + length);
    cursor += length;
  }
  if (cursor !== data.length) {
    throw new Error(`Derived LOD chunk decoded ${cursor} voxels, expected ${data.length}`);
  }
  return {
    coord: header.coord,
    lodLevel: header.lodLevel,
    voxelStride: header.voxelStride,
    data,
    solidCount: header.solidCount,
    solidBounds: header.solidBounds,
  };
}

function writeHeader(
  view: DataView,
  chunk: DerivedLodChunkPayload,
  chunkSize: number,
  runCount: number,
): void {
  view.setUint32(0, CODEC_MAGIC, true);
  view.setUint16(4, CODEC_VERSION, true);
  view.setUint16(6, chunkSize, true);
  view.setUint8(8, chunk.lodLevel);
  view.setUint8(9, chunk.solidBounds ? 1 : 0);
  view.setUint16(10, chunk.voxelStride, true);
  view.setInt32(12, chunk.coord.x, true);
  view.setInt32(16, chunk.coord.y, true);
  view.setInt32(20, chunk.coord.z, true);
  view.setUint32(24, chunk.solidCount, true);
  view.setUint32(28, chunk.data.length, true);
  view.setUint32(32, runCount, true);
  if (!chunk.solidBounds) {
    for (let index = 0; index < 6; index += 1) {
      view.setUint8(36 + index, 0);
    }
    return;
  }
  view.setUint8(36, chunk.solidBounds.min[0]);
  view.setUint8(37, chunk.solidBounds.min[1]);
  view.setUint8(38, chunk.solidBounds.min[2]);
  view.setUint8(39, chunk.solidBounds.max[0]);
  view.setUint8(40, chunk.solidBounds.max[1]);
  view.setUint8(41, chunk.solidBounds.max[2]);
}

function readHeader(view: DataView): DecodedHeader {
  const magic = view.getUint32(0, true);
  if (magic !== CODEC_MAGIC) {
    throw new Error(`Unexpected derived LOD chunk codec magic ${magic.toString(16)}`);
  }
  const version = view.getUint16(4, true);
  if (version !== CODEC_VERSION) {
    throw new Error(`Unsupported derived LOD chunk codec version ${version}`);
  }
  const chunkSize = view.getUint16(6, true);
  const hasSolidBounds = view.getUint8(9) === 1;
  return {
    coord: {
      x: view.getInt32(12, true),
      y: view.getInt32(16, true),
      z: view.getInt32(20, true),
    },
    chunkSize,
    lodLevel: view.getUint8(8),
    voxelStride: view.getUint16(10, true),
    solidCount: view.getUint32(24, true),
    voxelCount: view.getUint32(28, true),
    runCount: view.getUint32(32, true),
    solidBounds: hasSolidBounds
      ? {
          min: [view.getUint8(36), view.getUint8(37), view.getUint8(38)],
          max: [view.getUint8(39), view.getUint8(40), view.getUint8(41)],
        }
      : null,
  };
}

function collectRuns(data: Uint16Array): Array<{ material: number; length: number }> {
  if (data.length === 0) {
    return [];
  }
  const runs: Array<{ material: number; length: number }> = [];
  let material = data[0]!;
  let length = 1;
  for (let index = 1; index < data.length; index += 1) {
    const next = data[index]!;
    if (next === material) {
      length += 1;
      continue;
    }
    runs.push({ material, length });
    material = next;
    length = 1;
  }
  runs.push({ material, length });
  return runs;
}

function inferChunkSize(data: Uint16Array): number {
  const chunkSize = Math.round(Math.cbrt(data.length));
  if (chunkSize * chunkSize * chunkSize !== data.length) {
    throw new Error(`Expected cubic derived LOD chunk data, received length ${data.length}`);
  }
  return chunkSize;
}
