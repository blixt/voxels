import { fnv1a } from "./math.ts";
import { VoxelWorld } from "./world.ts";

const MAGIC = 0x43535856;
const VERSION = 1;

class ByteWriter {
  private readonly bytes: number[] = [];

  writeU16(value: number): void {
    this.bytes.push(value & 0xff, (value >>> 8) & 0xff);
  }

  writeU32(value: number): void {
    this.bytes.push(value & 0xff, (value >>> 8) & 0xff, (value >>> 16) & 0xff, (value >>> 24) & 0xff);
  }

  finish(): Uint8Array {
    return Uint8Array.from(this.bytes);
  }
}

class ByteReader {
  private offset = 0;

  constructor(private readonly bytes: Uint8Array) {}

  readU8(): number {
    const value = this.bytes[this.offset];
    if (value === undefined) {
      throw new Error("Unexpected end of buffer while reading u8");
    }
    this.offset += 1;
    return value;
  }

  readU16(): number {
    const value = this.readU8() | (this.readU8() << 8);
    return value >>> 0;
  }

  readU32(): number {
    return (this.readU8() | (this.readU8() << 8) | (this.readU8() << 16) | (this.readU8() << 24)) >>> 0;
  }
}

export function encodeWorld(world: VoxelWorld): Uint8Array {
  const writer = new ByteWriter();
  writer.writeU32(MAGIC);
  writer.writeU16(VERSION);
  writer.writeU16(world.chunkSize);
  writer.writeU16(world.width);
  writer.writeU16(world.height);
  writer.writeU16(world.depth);
  writer.writeU16(world.palette.length);
  for (const color of world.palette) {
    writer.writeU32(color);
  }

  writer.writeU32(world.chunks.size);
  for (const chunk of world.chunks.values()) {
    writer.writeU16(chunk.coord.x);
    writer.writeU16(chunk.coord.y);
    writer.writeU16(chunk.coord.z);
    const data = chunk.data;
    let index = 0;
    while (index < data.length) {
      const value = data[index]!;
      let runLength = 1;
      while (index + runLength < data.length && data[index + runLength] === value && runLength < 0xffff) {
        runLength += 1;
      }
      writer.writeU16(runLength);
      writer.writeU16(value);
      index += runLength;
    }
    writer.writeU16(0);
  }

  return writer.finish();
}

export function decodeWorld(bytes: Uint8Array | ArrayBuffer): VoxelWorld {
  const source = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  const reader = new ByteReader(source);
  const magic = reader.readU32();
  if (magic !== MAGIC) {
    throw new Error("Invalid VXSC scene file");
  }
  const version = reader.readU16();
  if (version !== VERSION) {
    throw new Error(`Unsupported VXSC version: ${version}`);
  }
  const chunkSize = reader.readU16();
  const width = reader.readU16();
  const height = reader.readU16();
  const depth = reader.readU16();
  const paletteLength = reader.readU16();
  const palette: number[] = [];
  for (let index = 0; index < paletteLength; index += 1) {
    palette.push(reader.readU32());
  }
  const world = new VoxelWorld({ width, height, depth }, chunkSize, palette);
  const chunkCount = reader.readU32();
  for (let chunkIndex = 0; chunkIndex < chunkCount; chunkIndex += 1) {
    const cx = reader.readU16();
    const cy = reader.readU16();
    const cz = reader.readU16();
    const baseX = cx * chunkSize;
    const baseY = cy * chunkSize;
    const baseZ = cz * chunkSize;
    let localIndex = 0;
    while (true) {
      const runLength = reader.readU16();
      if (runLength === 0) {
        break;
      }
      const value = reader.readU16();
      for (let count = 0; count < runLength; count += 1) {
        if (value !== 0) {
          const lx = localIndex % chunkSize;
          const ly = Math.floor(localIndex / chunkSize) % chunkSize;
          const lz = Math.floor(localIndex / (chunkSize * chunkSize));
          world.setVoxel(baseX + lx, baseY + ly, baseZ + lz, value);
        }
        localIndex += 1;
      }
    }
  }
  return world;
}

export function hashWorld(world: VoxelWorld): string {
  return fnv1a(encodeWorld(world));
}
