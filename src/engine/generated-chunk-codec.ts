import type { GeneratedChunk } from "./procedural-generator.ts";
import {
  GENERATED_CHUNK_RENDER_CELL_SIZE,
  NO_GENERATED_SURFACE_HEIGHT,
  NO_GENERATED_WATER_HEIGHT,
  type GeneratedChunkRenderSummary,
} from "./generated-chunk-render-summary.ts";
import type { ChunkCoordinate } from "./types.ts";

const CODEC_MAGIC = 0x3158_4356;
const CODEC_VERSION = 3;
const SUBCHUNK_SIZE = 8;
const SUBCHUNK_VOLUME = SUBCHUNK_SIZE * SUBCHUNK_SIZE * SUBCHUNK_SIZE;
const HEADER_BYTES = 34;

const SUBCHUNK_EMPTY = 0;
const SUBCHUNK_UNIFORM = 1;
const SUBCHUNK_PALETTE = 2;
const SUBCHUNK_DENSE = 3;

export interface GeneratedChunkCodecStats {
  readonly byteLength: number;
  readonly subchunkCount: number;
  readonly emptySubchunkCount: number;
  readonly uniformSubchunkCount: number;
  readonly paletteSubchunkCount: number;
  readonly denseSubchunkCount: number;
}

export interface EncodedGeneratedChunk {
  readonly buffer: ArrayBuffer;
  readonly stats: GeneratedChunkCodecStats;
}

export interface DecodedGeneratedChunkSummary {
  readonly coord: ChunkCoordinate;
  readonly chunkSize: number;
  readonly solidCount: number;
  readonly solidBounds: GeneratedChunk["solidBounds"];
  readonly renderSummary: GeneratedChunkRenderSummary;
}

export function encodeGeneratedChunk(chunk: GeneratedChunk): EncodedGeneratedChunk {
  const chunkSize = inferChunkSize(chunk.data);
  if (chunkSize % SUBCHUNK_SIZE !== 0) {
    throw new Error(`Chunk size ${chunkSize} is not divisible by ${SUBCHUNK_SIZE}`);
  }
  const subchunksPerAxis = chunkSize / SUBCHUNK_SIZE;
  const subchunkCount = subchunksPerAxis * subchunksPerAxis * subchunksPerAxis;
  const chunkArea = chunkSize * chunkSize;
  const summaryMacroCellsPerAxis = chunkSize / GENERATED_CHUNK_RENDER_CELL_SIZE;
  const summaryMacroCellCount = summaryMacroCellsPerAxis * summaryMacroCellsPerAxis * summaryMacroCellsPerAxis;
  const summaryFaceMaskLength = summaryMacroCellsPerAxis * summaryMacroCellsPerAxis * 6;
  const maxBytes = HEADER_BYTES
    + subchunkCount * (1 + SUBCHUNK_VOLUME * 2)
    + 4
    + Math.ceil(chunkArea / 8)
    + chunkArea * 12
    + summaryMacroCellCount
    + summaryFaceMaskLength;
  const buffer = new ArrayBuffer(maxBytes);
  const byteView = new Uint8Array(buffer);
  const view = new DataView(buffer);

  writeHeader(view, chunk.coord, chunkSize, chunk.solidCount, chunk.solidBounds);
  let byteOffset = HEADER_BYTES;
  let emptySubchunkCount = 0;
  let uniformSubchunkCount = 0;
  let paletteSubchunkCount = 0;
  let denseSubchunkCount = 0;

  for (let subchunkZ = 0; subchunkZ < subchunksPerAxis; subchunkZ += 1) {
    for (let subchunkY = 0; subchunkY < subchunksPerAxis; subchunkY += 1) {
      for (let subchunkX = 0; subchunkX < subchunksPerAxis; subchunkX += 1) {
        const materials = readSubchunkMaterials(chunk.data, chunkSize, subchunkX, subchunkY, subchunkZ);
        const kind = classifySubchunk(materials);
        byteView[byteOffset] = kind.kind;
        byteOffset += 1;
        switch (kind.kind) {
          case SUBCHUNK_EMPTY:
            emptySubchunkCount += 1;
            break;
          case SUBCHUNK_UNIFORM:
            view.setUint16(byteOffset, kind.material, true);
            byteOffset += 2;
            uniformSubchunkCount += 1;
            break;
          case SUBCHUNK_PALETTE:
            byteOffset = writePaletteSubchunk(view, byteView, byteOffset, materials, kind.palette);
            paletteSubchunkCount += 1;
            break;
          case SUBCHUNK_DENSE:
            for (let index = 0; index < SUBCHUNK_VOLUME; index += 1) {
              view.setUint16(byteOffset, materials[index]!, true);
              byteOffset += 2;
            }
            denseSubchunkCount += 1;
            break;
        }
      }
    }
  }

  byteOffset = writeRenderSummary(view, byteView, byteOffset, chunk.renderSummary, chunkSize);

  return {
    buffer: buffer.slice(0, byteOffset),
    stats: {
      byteLength: byteOffset,
      subchunkCount,
      emptySubchunkCount,
      uniformSubchunkCount,
      paletteSubchunkCount,
      denseSubchunkCount,
    },
  };
}

export function decodeGeneratedChunk(buffer: ArrayBuffer): GeneratedChunk {
  const view = new DataView(buffer);
  const header = readHeader(view);
  const chunkArea = header.chunkSize * header.chunkSize;
  const data = new Uint16Array(header.chunkSize * chunkArea);
  const subchunksPerAxis = header.chunkSize / SUBCHUNK_SIZE;
  let byteOffset = HEADER_BYTES;

  for (let subchunkZ = 0; subchunkZ < subchunksPerAxis; subchunkZ += 1) {
    for (let subchunkY = 0; subchunkY < subchunksPerAxis; subchunkY += 1) {
      for (let subchunkX = 0; subchunkX < subchunksPerAxis; subchunkX += 1) {
        const kind = view.getUint8(byteOffset);
        byteOffset += 1;
        switch (kind) {
          case SUBCHUNK_EMPTY:
            writeUniformSubchunk(data, header.chunkSize, subchunkX, subchunkY, subchunkZ, 0);
            break;
          case SUBCHUNK_UNIFORM: {
            const material = view.getUint16(byteOffset, true);
            byteOffset += 2;
            writeUniformSubchunk(data, header.chunkSize, subchunkX, subchunkY, subchunkZ, material);
            break;
          }
          case SUBCHUNK_PALETTE:
            byteOffset = readPaletteSubchunk(view, data, header.chunkSize, subchunkX, subchunkY, subchunkZ, byteOffset);
            break;
          case SUBCHUNK_DENSE:
            byteOffset = readDenseSubchunk(view, data, header.chunkSize, subchunkX, subchunkY, subchunkZ, byteOffset);
            break;
          default:
            throw new Error(`Unknown generated chunk subchunk kind ${kind}`);
        }
      }
    }
  }

  const decodedRenderSummary = readRenderSummary(view, byteOffset, header.coord, header.chunkSize);
  byteOffset = decodedRenderSummary.byteOffset;

  return {
    coord: header.coord,
    data,
    solidCount: header.solidCount,
    solidBounds: header.solidBounds,
    renderSummary: decodedRenderSummary.summary,
  };
}

export function decodeGeneratedChunkSummary(buffer: ArrayBuffer): DecodedGeneratedChunkSummary {
  const view = new DataView(buffer);
  const header = readHeader(view);
  const byteOffset = skipEncodedSubchunks(view, HEADER_BYTES, header.chunkSize);
  const decodedRenderSummary = readRenderSummary(view, byteOffset, header.coord, header.chunkSize);
  return {
    coord: header.coord,
    chunkSize: header.chunkSize,
    solidCount: header.solidCount,
    solidBounds: header.solidBounds,
    renderSummary: decodedRenderSummary.summary,
  };
}

function inferChunkSize(data: Uint16Array): number {
  const chunkSize = Math.round(Math.cbrt(data.length));
  if (chunkSize * chunkSize * chunkSize !== data.length) {
    throw new Error(`Expected cubic chunk data, received length ${data.length}`);
  }
  return chunkSize;
}

function skipEncodedSubchunks(view: DataView, byteOffset: number, chunkSize: number): number {
  const subchunksPerAxis = chunkSize / SUBCHUNK_SIZE;
  for (let subchunkZ = 0; subchunkZ < subchunksPerAxis; subchunkZ += 1) {
    for (let subchunkY = 0; subchunkY < subchunksPerAxis; subchunkY += 1) {
      for (let subchunkX = 0; subchunkX < subchunksPerAxis; subchunkX += 1) {
        const kind = view.getUint8(byteOffset);
        byteOffset += 1;
        switch (kind) {
          case SUBCHUNK_EMPTY:
            break;
          case SUBCHUNK_UNIFORM:
            byteOffset += 2;
            break;
          case SUBCHUNK_PALETTE: {
            const paletteLength = view.getUint16(byteOffset, true);
            byteOffset += 2;
            const bitsPerIndex = view.getUint16(byteOffset, true);
            byteOffset += 2;
            byteOffset += paletteLength * 2;
            byteOffset += Math.ceil((SUBCHUNK_VOLUME * bitsPerIndex) / 8);
            break;
          }
          case SUBCHUNK_DENSE:
            byteOffset += SUBCHUNK_VOLUME * 2;
            break;
          default:
            throw new Error(`Unknown generated chunk subchunk kind ${kind}`);
        }
      }
    }
  }
  return byteOffset;
}

function writeHeader(
  view: DataView,
  coord: ChunkCoordinate,
  chunkSize: number,
  solidCount: number,
  solidBounds: GeneratedChunk["solidBounds"],
): void {
  view.setUint32(0, CODEC_MAGIC, true);
  view.setUint16(4, CODEC_VERSION, true);
  view.setUint16(6, chunkSize, true);
  view.setInt32(8, coord.x, true);
  view.setInt32(12, coord.y, true);
  view.setInt32(16, coord.z, true);
  view.setUint32(20, solidCount, true);
  view.setUint8(24, solidBounds ? 1 : 0);
  view.setUint8(25, SUBCHUNK_SIZE);
  view.setUint16(26, 0, true);
  if (!solidBounds) {
    for (let offset = 0; offset < 6; offset += 1) {
      view.setUint8(28 + offset, 0);
    }
    return;
  }
  view.setUint8(28, solidBounds.min[0]);
  view.setUint8(29, solidBounds.min[1]);
  view.setUint8(30, solidBounds.min[2]);
  view.setUint8(31, solidBounds.max[0]);
  view.setUint8(32, solidBounds.max[1]);
  view.setUint8(33, solidBounds.max[2]);
}

function readHeader(view: DataView): {
  coord: ChunkCoordinate;
  chunkSize: number;
  solidCount: number;
  solidBounds: GeneratedChunk["solidBounds"];
} {
  const magic = view.getUint32(0, true);
  if (magic !== CODEC_MAGIC) {
    throw new Error(`Unexpected generated chunk codec magic ${magic}`);
  }
  const version = view.getUint16(4, true);
  if (version !== CODEC_VERSION) {
    throw new Error(`Unsupported generated chunk codec version ${version}`);
  }
  const chunkSize = view.getUint16(6, true);
  const subchunkSize = view.getUint8(25);
  if (subchunkSize !== SUBCHUNK_SIZE) {
    throw new Error(`Unsupported generated chunk subchunk size ${subchunkSize}`);
  }
  const hasBounds = view.getUint8(24) !== 0;
  return {
    coord: {
      x: view.getInt32(8, true),
      y: view.getInt32(12, true),
      z: view.getInt32(16, true),
    },
    chunkSize,
    solidCount: view.getUint32(20, true),
    solidBounds: hasBounds
      ? {
          min: [view.getUint8(28), view.getUint8(29), view.getUint8(30)],
          max: [view.getUint8(31), view.getUint8(32), view.getUint8(33)],
        }
      : null,
  };
}

function readSubchunkMaterials(
  data: Uint16Array,
  chunkSize: number,
  subchunkX: number,
  subchunkY: number,
  subchunkZ: number,
): Uint16Array {
  const subchunk = new Uint16Array(SUBCHUNK_VOLUME);
  const chunkArea = chunkSize * chunkSize;
  const startX = subchunkX * SUBCHUNK_SIZE;
  const startY = subchunkY * SUBCHUNK_SIZE;
  const startZ = subchunkZ * SUBCHUNK_SIZE;
  let writeIndex = 0;
  for (let localZ = 0; localZ < SUBCHUNK_SIZE; localZ += 1) {
    const z = startZ + localZ;
    for (let localY = 0; localY < SUBCHUNK_SIZE; localY += 1) {
      const y = startY + localY;
      const rowOffset = y * chunkSize + z * chunkArea;
      for (let localX = 0; localX < SUBCHUNK_SIZE; localX += 1) {
        subchunk[writeIndex] = data[startX + localX + rowOffset] ?? 0;
        writeIndex += 1;
      }
    }
  }
  return subchunk;
}

function writeRenderSummary(
  view: DataView,
  byteView: Uint8Array,
  byteOffset: number,
  summary: GeneratedChunkRenderSummary,
  chunkSize: number,
): number {
  const chunkArea = chunkSize * chunkSize;
  view.setUint8(byteOffset, summary.macroCellSize);
  byteOffset += 1;
  view.setUint8(byteOffset, summary.macroCellsPerAxis);
  byteOffset += 1;
  view.setUint16(byteOffset, summary.coveredColumnCount, true);
  byteOffset += 2;
  const maskOffset = byteOffset;
  const maskByteLength = Math.ceil(chunkArea / 8);
  byteView.fill(0, maskOffset, maskOffset + maskByteLength);
  byteOffset += maskByteLength;
  for (let columnIndex = 0; columnIndex < chunkArea; columnIndex += 1) {
    const surfaceY = summary.surfaceY[columnIndex] ?? NO_GENERATED_SURFACE_HEIGHT;
    const waterTopY = summary.waterTopY[columnIndex] ?? NO_GENERATED_WATER_HEIGHT;
    if (surfaceY === NO_GENERATED_SURFACE_HEIGHT && waterTopY === NO_GENERATED_WATER_HEIGHT) {
      continue;
    }
    byteView[maskOffset + (columnIndex >>> 3)]! |= 1 << (columnIndex & 7);
    view.setInt32(byteOffset, surfaceY, true);
    byteOffset += 4;
    view.setUint16(byteOffset, summary.surfaceMaterial[columnIndex] ?? 0, true);
    byteOffset += 2;
    view.setInt32(byteOffset, waterTopY, true);
    byteOffset += 4;
    view.setUint16(byteOffset, summary.waterMaterial[columnIndex] ?? 0, true);
    byteOffset += 2;
  }
  byteView.set(summary.macroCellStates, byteOffset);
  byteOffset += summary.macroCellStates.length;
  byteView.set(summary.faceOpenMask, byteOffset);
  byteOffset += summary.faceOpenMask.length;
  return byteOffset;
}

function readRenderSummary(
  view: DataView,
  byteOffset: number,
  coord: ChunkCoordinate,
  chunkSize: number,
): {
  byteOffset: number;
  summary: GeneratedChunkRenderSummary;
} {
  const chunkArea = chunkSize * chunkSize;
  const macroCellSize = view.getUint8(byteOffset);
  byteOffset += 1;
  const macroCellsPerAxis = view.getUint8(byteOffset);
  byteOffset += 1;
  const coveredColumnCount = view.getUint16(byteOffset, true);
  byteOffset += 2;
  const maskByteLength = Math.ceil(chunkArea / 8);
  const maskOffset = byteOffset;
  byteOffset += maskByteLength;
  const surfaceY = coveredColumnCount === 0 ? new Int32Array(0) : new Int32Array(chunkArea);
  const surfaceMaterial = coveredColumnCount === 0 ? new Uint16Array(0) : new Uint16Array(chunkArea);
  const waterTopY = coveredColumnCount === 0 ? new Int32Array(0) : new Int32Array(chunkArea);
  const waterMaterial = coveredColumnCount === 0 ? new Uint16Array(0) : new Uint16Array(chunkArea);
  if (coveredColumnCount > 0) {
    surfaceY.fill(NO_GENERATED_SURFACE_HEIGHT);
    waterTopY.fill(NO_GENERATED_WATER_HEIGHT);
    for (let columnIndex = 0; columnIndex < chunkArea; columnIndex += 1) {
      const masked = (view.getUint8(maskOffset + (columnIndex >>> 3)) & (1 << (columnIndex & 7))) !== 0;
      if (!masked) {
        continue;
      }
      surfaceY[columnIndex] = view.getInt32(byteOffset, true);
      byteOffset += 4;
      surfaceMaterial[columnIndex] = view.getUint16(byteOffset, true);
      byteOffset += 2;
      waterTopY[columnIndex] = view.getInt32(byteOffset, true);
      byteOffset += 4;
      waterMaterial[columnIndex] = view.getUint16(byteOffset, true);
      byteOffset += 2;
    }
  }
  const macroCellCount = macroCellsPerAxis * macroCellsPerAxis * macroCellsPerAxis;
  const macroCellStates = new Uint8Array(macroCellCount);
  macroCellStates.set(new Uint8Array(view.buffer, view.byteOffset + byteOffset, macroCellCount));
  byteOffset += macroCellCount;
  const faceOpenMaskLength = macroCellsPerAxis * macroCellsPerAxis * 6;
  const faceOpenMask = new Uint8Array(faceOpenMaskLength);
  faceOpenMask.set(new Uint8Array(view.buffer, view.byteOffset + byteOffset, faceOpenMaskLength));
  byteOffset += faceOpenMaskLength;
  return {
    byteOffset,
    summary: {
      coord: { ...coord },
      coveredColumnCount,
      surfaceY,
      surfaceMaterial,
      waterTopY,
      waterMaterial,
      macroCellSize,
      macroCellsPerAxis,
      macroCellStates,
      faceOpenMask,
    },
  };
}

function writeUniformSubchunk(
  data: Uint16Array,
  chunkSize: number,
  subchunkX: number,
  subchunkY: number,
  subchunkZ: number,
  material: number,
): void {
  const chunkArea = chunkSize * chunkSize;
  const startX = subchunkX * SUBCHUNK_SIZE;
  const startY = subchunkY * SUBCHUNK_SIZE;
  const startZ = subchunkZ * SUBCHUNK_SIZE;
  for (let localZ = 0; localZ < SUBCHUNK_SIZE; localZ += 1) {
    const z = startZ + localZ;
    for (let localY = 0; localY < SUBCHUNK_SIZE; localY += 1) {
      const y = startY + localY;
      const rowOffset = y * chunkSize + z * chunkArea;
      for (let localX = 0; localX < SUBCHUNK_SIZE; localX += 1) {
        data[startX + localX + rowOffset] = material;
      }
    }
  }
}

function classifySubchunk(materials: Uint16Array):
  | { kind: typeof SUBCHUNK_EMPTY }
  | { kind: typeof SUBCHUNK_UNIFORM; material: number }
  | { kind: typeof SUBCHUNK_PALETTE; palette: Uint16Array }
  | { kind: typeof SUBCHUNK_DENSE } {
  const first = materials[0] ?? 0;
  let uniform = true;
  const paletteValues: number[] = [first];
  const paletteLookup = new Map<number, number>([[first, 0]]);
  for (let index = 1; index < materials.length; index += 1) {
    const material = materials[index] ?? 0;
    if (material !== first) {
      uniform = false;
    }
    if (!paletteLookup.has(material)) {
      if (paletteValues.length >= 64) {
        return uniform
          ? { kind: SUBCHUNK_UNIFORM, material: first }
          : { kind: SUBCHUNK_DENSE };
      }
      paletteLookup.set(material, paletteValues.length);
      paletteValues.push(material);
    }
  }
  if (uniform) {
    return first === 0
      ? { kind: SUBCHUNK_EMPTY }
      : { kind: SUBCHUNK_UNIFORM, material: first };
  }
  const palette = Uint16Array.from(paletteValues);
  const bitsPerIndex = bitWidth(palette.length);
  const packedBytes = Math.ceil((SUBCHUNK_VOLUME * bitsPerIndex) / 8);
  const paletteBytes = 2 + 2 + palette.length * 2 + packedBytes;
  return paletteBytes < SUBCHUNK_VOLUME * 2
    ? { kind: SUBCHUNK_PALETTE, palette }
    : { kind: SUBCHUNK_DENSE };
}

function writePaletteSubchunk(
  view: DataView,
  byteView: Uint8Array,
  byteOffset: number,
  materials: Uint16Array,
  palette: Uint16Array,
): number {
  view.setUint16(byteOffset, palette.length, true);
  byteOffset += 2;
  const bitsPerIndex = bitWidth(palette.length);
  view.setUint16(byteOffset, bitsPerIndex, true);
  byteOffset += 2;
  const paletteLookup = new Map<number, number>();
  for (let index = 0; index < palette.length; index += 1) {
    const material = palette[index] ?? 0;
    view.setUint16(byteOffset, material, true);
    byteOffset += 2;
    paletteLookup.set(material, index);
  }
  const packedBytes = Math.ceil((SUBCHUNK_VOLUME * bitsPerIndex) / 8);
  byteView.fill(0, byteOffset, byteOffset + packedBytes);
  let bitOffset = 0;
  for (let index = 0; index < SUBCHUNK_VOLUME; index += 1) {
    writePackedBits(byteView, byteOffset, bitOffset, bitsPerIndex, paletteLookup.get(materials[index] ?? 0) ?? 0);
    bitOffset += bitsPerIndex;
  }
  return byteOffset + packedBytes;
}

function readPaletteSubchunk(
  view: DataView,
  data: Uint16Array,
  chunkSize: number,
  subchunkX: number,
  subchunkY: number,
  subchunkZ: number,
  byteOffset: number,
): number {
  const paletteLength = view.getUint16(byteOffset, true);
  byteOffset += 2;
  const bitsPerIndex = view.getUint16(byteOffset, true);
  byteOffset += 2;
  const palette = new Uint16Array(paletteLength);
  for (let index = 0; index < paletteLength; index += 1) {
    palette[index] = view.getUint16(byteOffset, true);
    byteOffset += 2;
  }
  const byteView = new Uint8Array(view.buffer);
  const packedBytes = Math.ceil((SUBCHUNK_VOLUME * bitsPerIndex) / 8);
  const chunkArea = chunkSize * chunkSize;
  const startX = subchunkX * SUBCHUNK_SIZE;
  const startY = subchunkY * SUBCHUNK_SIZE;
  const startZ = subchunkZ * SUBCHUNK_SIZE;
  let bitOffset = 0;
  for (let localZ = 0; localZ < SUBCHUNK_SIZE; localZ += 1) {
    const z = startZ + localZ;
    for (let localY = 0; localY < SUBCHUNK_SIZE; localY += 1) {
      const y = startY + localY;
      const rowOffset = y * chunkSize + z * chunkArea;
      for (let localX = 0; localX < SUBCHUNK_SIZE; localX += 1) {
        const paletteIndex = readPackedBits(byteView, byteOffset, bitOffset, bitsPerIndex);
        data[startX + localX + rowOffset] = palette[paletteIndex] ?? 0;
        bitOffset += bitsPerIndex;
      }
    }
  }
  return byteOffset + packedBytes;
}

function readDenseSubchunk(
  view: DataView,
  data: Uint16Array,
  chunkSize: number,
  subchunkX: number,
  subchunkY: number,
  subchunkZ: number,
  byteOffset: number,
): number {
  const chunkArea = chunkSize * chunkSize;
  const startX = subchunkX * SUBCHUNK_SIZE;
  const startY = subchunkY * SUBCHUNK_SIZE;
  const startZ = subchunkZ * SUBCHUNK_SIZE;
  for (let localZ = 0; localZ < SUBCHUNK_SIZE; localZ += 1) {
    const z = startZ + localZ;
    for (let localY = 0; localY < SUBCHUNK_SIZE; localY += 1) {
      const y = startY + localY;
      const rowOffset = y * chunkSize + z * chunkArea;
      for (let localX = 0; localX < SUBCHUNK_SIZE; localX += 1) {
        data[startX + localX + rowOffset] = view.getUint16(byteOffset, true);
        byteOffset += 2;
      }
    }
  }
  return byteOffset;
}

function writePackedBits(
  byteView: Uint8Array,
  byteOffset: number,
  bitOffset: number,
  bitCount: number,
  value: number,
): void {
  let remaining = bitCount;
  let writeValue = value;
  let writeBitOffset = bitOffset;
  while (remaining > 0) {
    const byteIndex = byteOffset + (writeBitOffset >> 3);
    const intraByteOffset = writeBitOffset & 7;
    const writableBits = Math.min(remaining, 8 - intraByteOffset);
    const bitMask = (1 << writableBits) - 1;
    byteView[byteIndex] |= (writeValue & bitMask) << intraByteOffset;
    writeValue >>= writableBits;
    writeBitOffset += writableBits;
    remaining -= writableBits;
  }
}

function readPackedBits(
  byteView: Uint8Array,
  byteOffset: number,
  bitOffset: number,
  bitCount: number,
): number {
  let remaining = bitCount;
  let readBitOffset = bitOffset;
  let shift = 0;
  let value = 0;
  while (remaining > 0) {
    const byteIndex = byteOffset + (readBitOffset >> 3);
    const intraByteOffset = readBitOffset & 7;
    const readableBits = Math.min(remaining, 8 - intraByteOffset);
    const bitMask = (1 << readableBits) - 1;
    value |= ((byteView[byteIndex] >> intraByteOffset) & bitMask) << shift;
    readBitOffset += readableBits;
    shift += readableBits;
    remaining -= readableBits;
  }
  return value;
}

function bitWidth(valueCount: number): number {
  return valueCount <= 1 ? 1 : 32 - Math.clz32(valueCount - 1);
}
