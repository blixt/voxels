import type { GeneratedChunk } from "./procedural-generator.ts";
import type { ChunkCoordinate } from "./types.ts";

const CODEC_MAGIC = 0x3158_4356;
const CODEC_VERSION = 1;
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

export function encodeGeneratedChunk(chunk: GeneratedChunk): EncodedGeneratedChunk {
  const chunkSize = inferChunkSize(chunk.data);
  if (chunkSize % SUBCHUNK_SIZE !== 0) {
    throw new Error(`Chunk size ${chunkSize} is not divisible by ${SUBCHUNK_SIZE}`);
  }
  const subchunksPerAxis = chunkSize / SUBCHUNK_SIZE;
  const subchunkCount = subchunksPerAxis * subchunksPerAxis * subchunksPerAxis;
  const maxBytes = HEADER_BYTES + subchunkCount * (1 + SUBCHUNK_VOLUME * 2);
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

  return {
    coord: header.coord,
    data,
    solidCount: header.solidCount,
    solidBounds: header.solidBounds,
  };
}

function inferChunkSize(data: Uint16Array): number {
  const chunkSize = Math.round(Math.cbrt(data.length));
  if (chunkSize * chunkSize * chunkSize !== data.length) {
    throw new Error(`Expected cubic chunk data, received length ${data.length}`);
  }
  return chunkSize;
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
