import { deflateSync } from "node:zlib";
import { describe, expect, it } from "vite-plus/test";
import { embedPngBinary, embedPngText } from "../../web/png-metadata.ts";
import {
  auditTerrainDiagnosticAttachment,
  readTerrainDiagnosticAttachment,
  terrainDiagnosticOwnerId,
} from "./terrain-diagnostic.ts";

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
  const parts = [new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10])];
  for (const [type, data] of [
    ["IHDR", new Uint8Array(13)],
    ["IEND", new Uint8Array()],
  ] as const) {
    const chunk = new Uint8Array(data.length + 12);
    const view = new DataView(chunk.buffer);
    view.setUint32(0, data.length);
    chunk.set(new TextEncoder().encode(type), 4);
    chunk.set(data, 8);
    view.setUint32(chunk.length - 4, crc32(chunk.subarray(4, chunk.length - 4)));
    parts.push(chunk);
  }
  const png = new Uint8Array(parts.reduce((total, part) => total + part.length, 0));
  let offset = 0;
  for (const part of parts) {
    png.set(part, offset);
    offset += part.length;
  }
  return png;
}

function diagnosticPng(
  width: number,
  height: number,
  pixels: Uint8Array,
  current: {
    readonly surfacePatches: readonly { hierarchyDepth: number; x: number; z: number }[];
    readonly canonicalChunks: readonly (readonly [number, number, number])[];
    readonly enclosedViewChunks: readonly (readonly [number, number, number])[];
    readonly transitionMeshKey: readonly [number, number, number, number] | null;
  },
): Uint8Array {
  const compressed = deflateSync(pixels);
  const payload = new Uint8Array(20 + compressed.byteLength);
  const header = new DataView(payload.buffer);
  header.setUint32(0, 0x56545031);
  header.setUint16(4, 1);
  header.setUint16(6, 5);
  header.setUint32(8, width);
  header.setUint32(12, height);
  header.setUint32(16, pixels.byteLength);
  payload.set(compressed, 20);
  const metadata = JSON.stringify({
    schema: "voxels.reproduction.v2",
    image: { pixelWidth: width, pixelHeight: height },
    attachments: {
      terrainPixelOwnership: {
        schema: "voxels.terrain-pixel-ownership.v1",
        worldPositionReconstruction: {
          inverseViewProjectionColumns: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1],
        },
      },
    },
    presentation: { selectedCut: { current, outgoing: null } },
  });
  return embedPngBinary(
    embedPngText(minimalPng(), "voxels.reproduction", metadata),
    "vpDI",
    payload,
  );
}

describe("terrain diagnostic attachment", () => {
  it("decodes exact integer identity and reconstructs world position", () => {
    const pixels = new Uint8Array(20);
    const pixel = new DataView(pixels.buffer);
    const owner = terrainDiagnosticOwnerId(2, 3, -4, 0, 7);
    pixel.setUint32(0, owner[0], true);
    pixel.setUint32(4, owner[1], true);
    pixel.setUint32(8, 0x12345678, true);
    pixel.setUint32(12, 6 | (3 << 4) | (2 << 8) | (14 << 11), true);
    pixel.setFloat32(16, 0.5, true);
    const png = diagnosticPng(1, 1, pixels, {
      surfacePatches: [{ hierarchyDepth: 2, x: -4, z: 7 }],
      canonicalChunks: [],
      enclosedViewChunks: [],
      transitionMeshKey: null,
    });

    const attachment = readTerrainDiagnosticAttachment(png);
    expect(attachment.width).toBe(1);
    expect(attachment.height).toBe(1);
    expect(attachment.pixel(0, 0)).toEqual({
      ownerIdHash: owner,
      primitiveFaceHash: 0x12345678,
      representationSource: 6,
      hierarchyDepth: 3,
      face: 2,
      materialId: 14,
      reverseZDepth: 0.5,
      worldMetres: [0, 0, 0.5],
    });
    expect(() => attachment.pixel(-1, 0)).toThrow(/outside/u);
    expect(auditTerrainDiagnosticAttachment(png)).toEqual({
      ownedPixels: 1,
      ownerIds: 1,
      declaredOwnerIds: 2,
      unmappedOwnerIds: [],
      impossiblePrimitiveGapPixels: 0,
      impossiblePrimitiveGapSamples: [],
    });
  });

  it("detects a background pixel bracketed by one projected primitive", () => {
    const width = 3;
    const height = 3;
    const pixels = new Uint8Array(width * height * 20);
    const view = new DataView(pixels.buffer);
    const owner = terrainDiagnosticOwnerId(1, 0, -1, 2, -3);
    for (const x of [0, 2]) {
      const offset = (x + width) * 20;
      view.setUint32(offset, owner[0], true);
      view.setUint32(offset + 4, owner[1], true);
      view.setUint32(offset + 8, 42, true);
    }
    const png = diagnosticPng(width, height, pixels, {
      surfacePatches: [],
      canonicalChunks: [[-1, 2, -3]],
      enclosedViewChunks: [],
      transitionMeshKey: null,
    });
    const audit = auditTerrainDiagnosticAttachment(png);
    expect(audit.impossiblePrimitiveGapPixels).toBe(1);
    expect(audit.impossiblePrimitiveGapSamples).toEqual([[1, 1]]);
  });

  it("keeps signed page coordinates distinct around the world origin", () => {
    expect(terrainDiagnosticOwnerId(2, 1, -1, 0, 0)).not.toEqual(
      terrainDiagnosticOwnerId(2, 1, 0, 0, 0),
    );
    expect(terrainDiagnosticOwnerId(1, 0, -1, -1, -1)).not.toEqual([0, 0]);
  });
});
