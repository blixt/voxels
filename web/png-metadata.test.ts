import { describe, expect, it } from "vite-plus/test";
import { embedPngBinary, embedPngText, readPngBinary, readPngText } from "./png-metadata.ts";

function crc32(bytes: Uint8Array): number {
  let crc = 0xffffffff;
  for (const byte of bytes) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit += 1) {
      crc = (crc >>> 1) ^ (0xedb88320 & -(crc & 1));
    }
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function minimalPng(): Uint8Array {
  const signature = [137, 80, 78, 71, 13, 10, 26, 10];
  const chunks = [
    { type: "IHDR", data: new Uint8Array(13) },
    { type: "IEND", data: new Uint8Array() },
  ];
  const bytes = [new Uint8Array(signature)];
  for (const { type, data } of chunks) {
    const chunk = new Uint8Array(data.length + 12);
    new DataView(chunk.buffer).setUint32(0, data.length);
    chunk.set(new TextEncoder().encode(type), 4);
    chunk.set(data, 8);
    new DataView(chunk.buffer).setUint32(
      chunk.length - 4,
      crc32(chunk.subarray(4, chunk.length - 4)),
    );
    bytes.push(chunk);
  }
  const length = bytes.reduce((sum, part) => sum + part.length, 0);
  const png = new Uint8Array(length);
  let offset = 0;
  for (const part of bytes) {
    png.set(part, offset);
    offset += part.length;
  }
  return png;
}

describe("PNG screenshot metadata", () => {
  it("embeds round-trippable JSON in a CRC-valid tEXt chunk", () => {
    const source = minimalPng();
    const json = '{"schema":"voxels.reproduction.v3","pixelWidth":1280}';
    const encoded = embedPngText(source, "voxels.reproduction", json);

    expect(readPngText(encoded, "voxels.reproduction")).toBe(json);
    expect(encoded.subarray(0, source.length)).not.toEqual(source);
    const ihdrEnd = 8 + 12 + 13;
    const textLength = new DataView(encoded.buffer, encoded.byteOffset).getUint32(ihdrEnd);
    const typeAndData = encoded.subarray(ihdrEnd + 4, ihdrEnd + 8 + textLength);
    const storedCrc = new DataView(
      encoded.buffer,
      encoded.byteOffset + ihdrEnd + 8 + textLength,
      4,
    ).getUint32(0);
    expect(storedCrc).toBe(crc32(typeAndData));
  });

  it("rejects invalid PNGs and non-ASCII metadata", () => {
    expect(() => embedPngText(new Uint8Array(), "voxels.reproduction", "{}")).toThrow(
      /invalid PNG signature/u,
    );
    expect(() => embedPngText(minimalPng(), "voxels.reproduction", '{"place":"Málaga"}')).toThrow(
      /must be ASCII/u,
    );
  });

  it("embeds a CRC-valid private binary attachment without a textual sidecar", () => {
    const source = minimalPng();
    const diagnostic = new Uint8Array([0, 255, 17, 0, 88, 42]);
    const encoded = embedPngBinary(source, "vpDI", diagnostic);

    expect(readPngBinary(encoded, "vpDI")).toEqual(diagnostic);
    expect(readPngBinary(encoded, "vpXX")).toBeUndefined();
    expect(() => embedPngBinary(source, "VPDI", diagnostic)).toThrow(/must match aaAa/u);
    expect(() => embedPngBinary(source, "vpdI", diagnostic)).toThrow(/must match aaAa/u);
  });
});
