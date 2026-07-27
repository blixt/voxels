import { inflateSync } from "node:zlib";
import { readPngBinary, readPngText } from "../../web/png-metadata.ts";

const ATTACHMENT_TYPE = "vpDI";
const ATTACHMENT_MAGIC = 0x56545031;
const ATTACHMENT_HEADER_BYTES = 20;
const ATTACHMENT_CHANNELS = 5;
const PIXEL_BYTES = ATTACHMENT_CHANNELS * 4;
const FNV_OFFSET = 2166136261;
const FNV_PRIME = 16777619;

export interface TerrainDiagnosticPixel {
  readonly ownerIdHash: readonly [number, number];
  readonly primitiveFaceHash: number;
  readonly representationSource: number;
  readonly hierarchyDepth: number;
  readonly face: number;
  readonly materialId: number;
  readonly reverseZDepth: number;
  readonly worldMetres: readonly [number, number, number] | undefined;
}

interface ReproductionAttachmentMetadata {
  readonly schema: string;
  readonly image: {
    readonly pixelWidth: number;
    readonly pixelHeight: number;
  };
  readonly attachments: {
    readonly terrainPixelOwnership: {
      readonly schema: string;
      readonly inverseViewProjectionColumns?: readonly number[];
      readonly worldPositionReconstruction: {
        readonly inverseViewProjectionColumns: readonly number[];
      };
    };
  };
  readonly presentation: {
    readonly selectedCut: {
      readonly current: ReproductionCut;
      readonly outgoing: ReproductionCut | null;
    };
  };
}

interface ReproductionCut {
  readonly surfacePatches: readonly {
    readonly hierarchyDepth: number;
    readonly x: number;
    readonly z: number;
  }[];
  readonly canonicalChunks: readonly (readonly [number, number, number])[];
  readonly enclosedViewChunks: readonly (readonly [number, number, number])[];
  readonly transitionMeshKey: readonly [number, number, number, number] | null;
}

export interface TerrainDiagnosticAudit {
  readonly ownedPixels: number;
  readonly ownerIds: number;
  readonly declaredOwnerIds: number;
  readonly unmappedOwnerIds: readonly string[];
  /** Background pixels bracketed by the same projected primitive on a row or column. */
  readonly impossiblePrimitiveGapPixels: number;
  readonly impossiblePrimitiveGapSamples: readonly (readonly [number, number])[];
}

function hashStep(hash: number, value: number): number {
  return Math.imul((hash ^ value) >>> 0, FNV_PRIME) >>> 0;
}

function secondaryHashStep(hash: number, value: number): number {
  let mixed = (hash + value) >>> 0;
  mixed = (mixed + (mixed << 10)) >>> 0;
  return (mixed ^ (mixed >>> 6)) >>> 0;
}

function secondaryHashFinish(hash: number): number {
  let mixed = (hash + (hash << 3)) >>> 0;
  mixed = (mixed ^ (mixed >>> 11)) >>> 0;
  return (mixed + (mixed << 15)) >>> 0;
}

/** Mirrors the shader's stable signed-page identity, including negative coordinates. */
export function terrainDiagnosticOwnerId(
  representationKind: 1 | 2 | 3,
  hierarchyDepth: number,
  pageX: number,
  pageY: number,
  pageZ: number,
): readonly [number, number] {
  let low = FNV_OFFSET;
  let high = 0;
  for (const value of [representationKind, hierarchyDepth, pageX, pageY, pageZ]) {
    low = hashStep(low, value >>> 0);
    high = secondaryHashStep(high, value >>> 0);
  }
  high = secondaryHashFinish(high);
  return low === 0 && high === 0 ? [1, 0] : [low, high];
}

function ownerIdKey(owner: readonly [number, number]): string {
  return `${owner[0].toString(16).padStart(8, "0")}${owner[1].toString(16).padStart(8, "0")}`;
}

function parseMetadata(png: Uint8Array): ReproductionAttachmentMetadata {
  const text = readPngText(png, "voxels.reproduction");
  if (text === undefined) throw new Error("PNG has no voxels.reproduction metadata");
  const metadata = JSON.parse(text) as ReproductionAttachmentMetadata;
  if (
    metadata.schema !== "voxels.reproduction.v2" ||
    metadata.attachments?.terrainPixelOwnership?.schema !== "voxels.terrain-pixel-ownership.v1"
  ) {
    throw new Error("PNG has no supported terrain ownership attachment contract");
  }
  return metadata;
}

function multiplyColumnMajorMatrixVector(
  matrix: readonly number[],
  vector: readonly [number, number, number, number],
): [number, number, number, number] {
  if (matrix.length !== 16 || !matrix.every(Number.isFinite)) {
    throw new Error("terrain attachment inverse view-projection matrix is invalid");
  }
  return [0, 1, 2, 3].map((row) =>
    vector.reduce((sum, value, column) => sum + matrix[column * 4 + row]! * value, 0),
  ) as [number, number, number, number];
}

export class TerrainDiagnosticAttachment {
  readonly width: number;
  readonly height: number;
  readonly pixels: Uint8Array;
  readonly inverseViewProjection: readonly number[];

  constructor(
    width: number,
    height: number,
    pixels: Uint8Array,
    inverseViewProjection: readonly number[],
  ) {
    if (
      !Number.isSafeInteger(width) ||
      !Number.isSafeInteger(height) ||
      width <= 0 ||
      height <= 0 ||
      pixels.byteLength !== width * height * PIXEL_BYTES
    ) {
      throw new Error("terrain diagnostic attachment dimensions do not match its pixels");
    }
    this.width = width;
    this.height = height;
    this.pixels = pixels;
    this.inverseViewProjection = inverseViewProjection;
  }

  pixel(x: number, y: number): TerrainDiagnosticPixel {
    if (
      !Number.isSafeInteger(x) ||
      !Number.isSafeInteger(y) ||
      x < 0 ||
      y < 0 ||
      x >= this.width ||
      y >= this.height
    ) {
      throw new Error(`terrain diagnostic pixel ${x},${y} is outside the attachment`);
    }
    const offset = (x + y * this.width) * PIXEL_BYTES;
    const view = new DataView(this.pixels.buffer, this.pixels.byteOffset + offset, PIXEL_BYTES);
    const ownerIdHash = [view.getUint32(0, true), view.getUint32(4, true)] as const;
    const primitiveFaceHash = view.getUint32(8, true);
    const descriptor = view.getUint32(12, true);
    const reverseZDepth = view.getFloat32(16, true);
    let worldMetres: [number, number, number] | undefined;
    if (
      (ownerIdHash[0] !== 0 || ownerIdHash[1] !== 0) &&
      Number.isFinite(reverseZDepth) &&
      reverseZDepth > 0
    ) {
      const ndcX = ((x + 0.5) / this.width) * 2 - 1;
      const ndcY = 1 - ((y + 0.5) / this.height) * 2;
      const homogeneous = multiplyColumnMajorMatrixVector(this.inverseViewProjection, [
        ndcX,
        ndcY,
        reverseZDepth,
        1,
      ]);
      if (Number.isFinite(homogeneous[3]) && Math.abs(homogeneous[3]) > Number.EPSILON) {
        worldMetres = [
          homogeneous[0] / homogeneous[3],
          homogeneous[1] / homogeneous[3],
          homogeneous[2] / homogeneous[3],
        ];
      }
    }
    return {
      ownerIdHash,
      primitiveFaceHash,
      representationSource: descriptor & 0xf,
      hierarchyDepth: (descriptor >>> 4) & 0xf,
      face: (descriptor >>> 8) & 0x7,
      materialId: (descriptor >>> 11) & 0xffff,
      reverseZDepth,
      worldMetres,
    };
  }
}

/** Decodes the compressed machine-readable ownership buffer embedded in a mission screenshot. */
export function readTerrainDiagnosticAttachment(png: Uint8Array): TerrainDiagnosticAttachment {
  const metadata = parseMetadata(png);
  const payload = readPngBinary(png, ATTACHMENT_TYPE);
  if (payload === undefined || payload.byteLength < ATTACHMENT_HEADER_BYTES) {
    throw new Error("PNG has no complete vpDI terrain ownership attachment");
  }
  const header = new DataView(payload.buffer, payload.byteOffset, ATTACHMENT_HEADER_BYTES);
  if (
    header.getUint32(0) !== ATTACHMENT_MAGIC ||
    header.getUint16(4) !== 1 ||
    header.getUint16(6) !== ATTACHMENT_CHANNELS
  ) {
    throw new Error("PNG terrain ownership attachment header is unsupported");
  }
  const width = header.getUint32(8);
  const height = header.getUint32(12);
  const expectedBytes = header.getUint32(16);
  if (
    width !== metadata.image.pixelWidth ||
    height !== metadata.image.pixelHeight ||
    expectedBytes !== width * height * PIXEL_BYTES
  ) {
    throw new Error("PNG terrain ownership attachment disagrees with reproduction metadata");
  }
  const pixels = new Uint8Array(inflateSync(payload.subarray(ATTACHMENT_HEADER_BYTES)));
  if (pixels.byteLength !== expectedBytes) {
    throw new Error("PNG terrain ownership attachment decompressed to the wrong size");
  }
  const inverse =
    metadata.attachments.terrainPixelOwnership.worldPositionReconstruction
      .inverseViewProjectionColumns;
  return new TerrainDiagnosticAttachment(width, height, pixels, inverse);
}

/**
 * Proves that every non-background diagnostic owner can be resolved through the exact selected
 * cut. Any returned unmapped ID is actionable renderer output from outside that ownership graph.
 */
export function auditTerrainDiagnosticAttachment(png: Uint8Array): TerrainDiagnosticAudit {
  const metadata = parseMetadata(png);
  const attachment = readTerrainDiagnosticAttachment(png);
  const declared = new Set<string>();
  for (const cut of [
    metadata.presentation.selectedCut.current,
    metadata.presentation.selectedCut.outgoing,
  ]) {
    if (cut === null) continue;
    for (const patch of cut.surfacePatches) {
      declared.add(
        ownerIdKey(terrainDiagnosticOwnerId(2, patch.hierarchyDepth + 1, patch.x, 0, patch.z)),
      );
    }
    for (const [x, y, z] of [...cut.canonicalChunks, ...cut.enclosedViewChunks]) {
      declared.add(ownerIdKey(terrainDiagnosticOwnerId(1, 0, x, y, z)));
    }
    if (cut.transitionMeshKey !== null) {
      const [depth, x, y, z] = cut.transitionMeshKey;
      declared.add(ownerIdKey(terrainDiagnosticOwnerId(3, depth, x, y, z)));
    }
  }
  // The opaque exact-volume frontier is one explicit renderer product with a stable mesh key.
  declared.add(ownerIdKey(terrainDiagnosticOwnerId(3, 255, 2, 0, 0)));

  const seen = new Set<string>();
  let ownedPixels = 0;
  const view = new DataView(
    attachment.pixels.buffer,
    attachment.pixels.byteOffset,
    attachment.pixels.byteLength,
  );
  for (let offset = 0; offset < attachment.pixels.byteLength; offset += PIXEL_BYTES) {
    const owner = [view.getUint32(offset, true), view.getUint32(offset + 4, true)] as const;
    if (owner[0] === 0 && owner[1] === 0) continue;
    ownedPixels += 1;
    seen.add(ownerIdKey(owner));
  }
  const identityAt = (x: number, y: number) => {
    const offset = (x + y * attachment.width) * PIXEL_BYTES;
    return [
      view.getUint32(offset, true),
      view.getUint32(offset + 4, true),
      view.getUint32(offset + 8, true),
    ] as const;
  };
  const isOwned = (identity: readonly number[]) => identity[0] !== 0 || identity[1] !== 0;
  const samePrimitive = (left: readonly number[], right: readonly number[]) =>
    isOwned(left) && left[0] === right[0] && left[1] === right[1] && left[2] === right[2];
  let impossiblePrimitiveGapPixels = 0;
  const impossiblePrimitiveGapSamples: (readonly [number, number])[] = [];
  for (let y = 1; y + 1 < attachment.height; y += 1) {
    for (let x = 1; x + 1 < attachment.width; x += 1) {
      if (isOwned(identityAt(x, y))) continue;
      if (
        !samePrimitive(identityAt(x - 1, y), identityAt(x + 1, y)) &&
        !samePrimitive(identityAt(x, y - 1), identityAt(x, y + 1))
      ) {
        continue;
      }
      impossiblePrimitiveGapPixels += 1;
      if (impossiblePrimitiveGapSamples.length < 32) {
        impossiblePrimitiveGapSamples.push([x, y]);
      }
    }
  }
  return {
    ownedPixels,
    ownerIds: seen.size,
    declaredOwnerIds: declared.size,
    unmappedOwnerIds: [...seen].filter((owner) => !declared.has(owner)).sort(),
    impossiblePrimitiveGapPixels,
    impossiblePrimitiveGapSamples,
  };
}
