import { readFile } from "node:fs/promises";
import { inflateSync } from "node:zlib";
import type { Download, Page } from "playwright";
import { readPngBinary, readPngText } from "../../web/png-metadata.ts";

const ATTACHMENT_TYPE = "vpDI";
const ATTACHMENT_MAGIC = 0x56545031;
const ATTACHMENT_HEADER_BYTES = 20;
const ATTACHMENT_CHANNELS = 5;
const PIXEL_BYTES = ATTACHMENT_CHANNELS * 4;
const LEVEL_ZERO_SURFACE_PAGE_METRES = 3.2;

export interface PlayerScreenshotMetadata {
  readonly schema: "voxels.reproduction.v2";
  readonly frameSequence: number;
  readonly image: {
    readonly pixelWidth: number;
    readonly pixelHeight: number;
    readonly cssWidth: number;
    readonly cssHeight: number;
    readonly devicePixelRatio: number;
  };
  readonly camera: {
    readonly eyeMetres: readonly [number, number, number];
    readonly yawRadians: number;
    readonly pitchRadians: number;
  };
  readonly presentation: {
    readonly selectedCutFingerprint: string;
    readonly terrainHandleSnapshot: {
      readonly generation: string;
      readonly cutFingerprint: string;
      readonly matchesPublishedCut: boolean;
    };
    readonly selectedCut: {
      readonly kind: "virtualTerrain";
      readonly cut: {
        readonly selectedPages: readonly {
          readonly level: number;
          readonly coord: readonly [number, number, number];
        }[];
        readonly refinementRoots: readonly {
          readonly level: number;
          readonly coord: readonly [number, number, number];
        }[];
        readonly exactSurfaceLodDiscontinuities: number;
        readonly ownerlessRoots: readonly unknown[];
      } | null;
    };
  };
  readonly streaming: {
    readonly virtualStream: {
      readonly pendingPages: number;
      readonly inFlightPages: number;
      readonly obsoleteInFlightPages: number;
      readonly cancelledPendingPages: string;
      readonly cancellationWasteBytes: string;
      readonly failedPages: string;
    };
  };
  readonly render: {
    readonly worldLabOpen: boolean;
    readonly geometrySourceDebug: boolean;
  };
  readonly attachments: {
    readonly terrainPixelOwnership: {
      readonly schema: "voxels.terrain-pixel-ownership.v1";
      readonly populated: boolean;
      readonly worldPositionReconstruction: {
        readonly inverseViewProjectionColumns: readonly number[];
      };
    };
  };
}

export interface PlayerScreenshot {
  readonly filename: string;
  readonly png: Buffer;
  readonly metadata: PlayerScreenshotMetadata;
  readonly ownership: TerrainOwnershipAttachment | null;
}

export interface TerrainOwnershipSummary {
  readonly ownedPixels: number;
  readonly nearbyOwnedPixels: number;
  readonly nearbyExactPixels: number;
  readonly nearbyCoarsePixels: number;
  readonly nearbyCoarseFraction: number;
  readonly nearbyLevelPixels: Readonly<Record<string, number>>;
  readonly nearbySourcePixels: Readonly<Record<string, number>>;
}

export interface TerrainOwnershipPixel {
  readonly x: number;
  readonly y: number;
  readonly ownerLow: number;
  readonly ownerHigh: number;
  readonly source: number;
  readonly level: number;
  readonly reverseZDepth: number;
}

export interface TerrainCutAdjacencySummary {
  readonly adjacentEdges: number;
  readonly maximumLevelDelta: number;
  readonly discontinuousEdges: number;
  readonly discontinuitySamples: readonly string[];
}

type SelectedTerrainPage = NonNullable<
  PlayerScreenshotMetadata["presentation"]["selectedCut"]["cut"]
>["selectedPages"][number];

/**
 * Audits the published surface quadtree in integer level-0 page coordinates.
 *
 * A balanced cut may only change by one hierarchy level across a shared edge. Larger jumps are
 * exactly the condition that puts metre-scale geometry directly beside 10 cm terrain.
 */
export function summarizeSurfaceCutAdjacency(
  pages: readonly SelectedTerrainPage[],
  eyeMetres?: readonly [number, number, number],
  radiusMetres = Number.POSITIVE_INFINITY,
): TerrainCutAdjacencySummary {
  if (
    eyeMetres !== undefined &&
    (!eyeMetres.every(Number.isFinite) || !Number.isFinite(radiusMetres) || radiusMetres <= 0)
  ) {
    throw new Error("surface cut adjacency audit received an invalid eye or radius");
  }
  const eyePage =
    eyeMetres === undefined
      ? undefined
      : [
          eyeMetres[0] / LEVEL_ZERO_SURFACE_PAGE_METRES,
          eyeMetres[2] / LEVEL_ZERO_SURFACE_PAGE_METRES,
        ];
  const surface = pages
    .filter((page) => page.coord[1] === -2_147_483_648)
    .map((page) => {
      const scale = 2 ** page.level;
      return {
        page,
        minimumX: page.coord[0] * scale,
        maximumX: (page.coord[0] + 1) * scale,
        minimumZ: page.coord[2] * scale,
        maximumZ: (page.coord[2] + 1) * scale,
      };
    });
  let adjacentEdges = 0;
  let maximumLevelDelta = 0;
  let discontinuousEdges = 0;
  const discontinuitySamples: string[] = [];
  for (let leftIndex = 0; leftIndex < surface.length; leftIndex += 1) {
    const left = surface[leftIndex]!;
    for (let rightIndex = leftIndex + 1; rightIndex < surface.length; rightIndex += 1) {
      const right = surface[rightIndex]!;
      const verticalEdge =
        (left.maximumX === right.minimumX || right.maximumX === left.minimumX) &&
        Math.max(left.minimumZ, right.minimumZ) < Math.min(left.maximumZ, right.maximumZ);
      const horizontalEdge =
        (left.maximumZ === right.minimumZ || right.maximumZ === left.minimumZ) &&
        Math.max(left.minimumX, right.minimumX) < Math.min(left.maximumX, right.maximumX);
      if (!verticalEdge && !horizontalEdge) continue;
      if (eyePage !== undefined) {
        const edgeX = verticalEdge
          ? left.maximumX === right.minimumX
            ? left.maximumX
            : left.minimumX
          : Math.max(left.minimumX, Math.min(eyePage[0]!, left.maximumX));
        const edgeZ = horizontalEdge
          ? left.maximumZ === right.minimumZ
            ? left.maximumZ
            : left.minimumZ
          : Math.max(
              Math.max(left.minimumZ, right.minimumZ),
              Math.min(eyePage[1]!, Math.min(left.maximumZ, right.maximumZ)),
            );
        const horizontalEdgeX = horizontalEdge
          ? Math.max(
              Math.max(left.minimumX, right.minimumX),
              Math.min(eyePage[0]!, Math.min(left.maximumX, right.maximumX)),
            )
          : edgeX;
        if (
          Math.hypot(horizontalEdgeX - eyePage[0]!, edgeZ - eyePage[1]!) *
            LEVEL_ZERO_SURFACE_PAGE_METRES >
          radiusMetres
        ) {
          continue;
        }
      }
      adjacentEdges += 1;
      const delta = Math.abs(left.page.level - right.page.level);
      maximumLevelDelta = Math.max(maximumLevelDelta, delta);
      if (delta <= 1) continue;
      discontinuousEdges += 1;
      if (discontinuitySamples.length < 8) {
        discontinuitySamples.push(
          `L${left.page.level}@${left.page.coord[0]},${left.page.coord[2]} <> L${right.page.level}@${right.page.coord[0]},${right.page.coord[2]}`,
        );
      }
    }
  }
  return {
    adjacentEdges,
    maximumLevelDelta,
    discontinuousEdges,
    discontinuitySamples,
  };
}

function parseMetadata(png: Uint8Array): PlayerScreenshotMetadata {
  const text = readPngText(png, "voxels.reproduction");
  if (text === undefined) throw new Error("player screenshot omitted voxels.reproduction metadata");
  const metadata = JSON.parse(text) as PlayerScreenshotMetadata;
  if (
    metadata.schema !== "voxels.reproduction.v2" ||
    metadata.attachments?.terrainPixelOwnership?.schema !== "voxels.terrain-pixel-ownership.v1" ||
    metadata.presentation?.selectedCut?.kind !== "virtualTerrain" ||
    typeof metadata.presentation?.terrainHandleSnapshot?.matchesPublishedCut !== "boolean" ||
    typeof metadata.presentation.terrainHandleSnapshot.generation !== "string" ||
    typeof metadata.presentation.terrainHandleSnapshot.cutFingerprint !== "string" ||
    !Array.isArray(metadata.presentation.selectedCut.cut?.selectedPages) ||
    !Array.isArray(metadata.presentation.selectedCut.cut?.refinementRoots) ||
    !Array.isArray(metadata.camera?.eyeMetres) ||
    metadata.camera.eyeMetres.length !== 3 ||
    !metadata.camera.eyeMetres.every(Number.isFinite)
  ) {
    throw new Error(`player screenshot metadata contract is incomplete: ${text}`);
  }
  return metadata;
}

async function completedDownload(download: Download): Promise<{
  readonly filename: string;
  readonly png: Buffer;
}> {
  const failure = await download.failure();
  const temporaryPath = await download.path();
  if (failure !== null || temporaryPath === null) {
    throw new Error(`F2 screenshot download failed: ${failure ?? "missing temporary file"}`);
  }
  return {
    filename: download.suggestedFilename(),
    png: await readFile(temporaryPath),
  };
}

/** Uses the actual gameplay key binding and downloaded PNG, with World Lab remaining closed. */
export async function takePlayerScreenshot(page: Page): Promise<PlayerScreenshot> {
  // The diagnostic attachment reads five full-resolution GPU channels and deflates them into the
  // PNG. SwiftShader in headless CI is deliberately much slower than a player's hardware GPU.
  const pending = page.waitForEvent("download", { timeout: 240_000 });
  await page.keyboard.press("F2");
  const { filename, png } = await completedDownload(await pending);
  return readPlayerScreenshot(png, filename);
}

/** Reads the same embedded contracts from an existing player capture for offline investigation. */
export function readPlayerScreenshot(png: Buffer, filename = "capture.png"): PlayerScreenshot {
  const metadata = parseMetadata(png);
  if (metadata.render.worldLabOpen) {
    throw new Error("F2 player screenshot unexpectedly opened World Lab");
  }
  return {
    filename,
    png,
    metadata,
    ownership: readTerrainOwnershipAttachment(png, metadata),
  };
}

export class TerrainOwnershipAttachment {
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
      pixels.byteLength !== width * height * PIXEL_BYTES ||
      inverseViewProjection.length !== 16 ||
      !inverseViewProjection.every(Number.isFinite)
    ) {
      throw new Error("terrain ownership attachment dimensions or projection are invalid");
    }
    this.width = width;
    this.height = height;
    this.pixels = pixels;
    this.inverseViewProjection = inverseViewProjection;
  }

  summarizeNearby(
    eyeMetres: readonly [number, number, number],
    radiusMetres: number,
  ): TerrainOwnershipSummary {
    if (!Number.isFinite(radiusMetres) || radiusMetres <= 0) {
      throw new Error("nearby terrain radius must be positive and finite");
    }
    const view = new DataView(this.pixels.buffer, this.pixels.byteOffset, this.pixels.byteLength);
    const matrix = this.inverseViewProjection;
    const radiusSquared = radiusMetres * radiusMetres;
    const nearbyLevelPixels: Record<string, number> = {};
    const nearbySourcePixels: Record<string, number> = {};
    let ownedPixels = 0;
    let nearbyOwnedPixels = 0;
    let nearbyExactPixels = 0;
    let nearbyCoarsePixels = 0;
    for (let y = 0; y < this.height; y += 1) {
      for (let x = 0; x < this.width; x += 1) {
        const offset = (x + y * this.width) * PIXEL_BYTES;
        const ownerLow = view.getUint32(offset, true);
        const ownerHigh = view.getUint32(offset + 4, true);
        if (ownerLow === 0 && ownerHigh === 0) continue;
        ownedPixels += 1;
        const reverseZDepth = view.getFloat32(offset + 16, true);
        if (!Number.isFinite(reverseZDepth) || reverseZDepth <= 0) continue;
        const ndcX = ((x + 0.5) / this.width) * 2 - 1;
        const ndcY = 1 - ((y + 0.5) / this.height) * 2;
        const vector = [ndcX, ndcY, reverseZDepth, 1] as const;
        const homogeneous = [0, 1, 2, 3].map((row) =>
          vector.reduce((sum, value, column) => sum + matrix[column * 4 + row]! * value, 0),
        );
        const divisor = homogeneous[3]!;
        if (!Number.isFinite(divisor) || Math.abs(divisor) <= Number.EPSILON) continue;
        const worldX = homogeneous[0]! / divisor;
        const worldZ = homogeneous[2]! / divisor;
        const deltaX = worldX - eyeMetres[0];
        const deltaZ = worldZ - eyeMetres[2];
        if (deltaX * deltaX + deltaZ * deltaZ > radiusSquared) continue;
        nearbyOwnedPixels += 1;
        const descriptor = view.getUint32(offset + 12, true);
        const source = descriptor & 0xf;
        const level = (descriptor >>> 4) & 0xf;
        nearbyLevelPixels[String(level)] = (nearbyLevelPixels[String(level)] ?? 0) + 1;
        nearbySourcePixels[String(source)] = (nearbySourcePixels[String(source)] ?? 0) + 1;
        if (level === 0) {
          nearbyExactPixels += 1;
        } else {
          nearbyCoarsePixels += 1;
        }
      }
    }
    return {
      ownedPixels,
      nearbyOwnedPixels,
      nearbyExactPixels,
      nearbyCoarsePixels,
      nearbyCoarseFraction: nearbyOwnedPixels === 0 ? 1 : nearbyCoarsePixels / nearbyOwnedPixels,
      nearbyLevelPixels: Object.freeze(nearbyLevelPixels),
      nearbySourcePixels: Object.freeze(nearbySourcePixels),
    };
  }

  sampleNeighborhood(x: number, y: number, radiusPixels = 2): readonly TerrainOwnershipPixel[] {
    if (
      !Number.isSafeInteger(x) ||
      !Number.isSafeInteger(y) ||
      !Number.isSafeInteger(radiusPixels) ||
      radiusPixels < 0 ||
      radiusPixels > 16
    ) {
      throw new Error("terrain ownership neighborhood is invalid");
    }
    const view = new DataView(this.pixels.buffer, this.pixels.byteOffset, this.pixels.byteLength);
    const samples: TerrainOwnershipPixel[] = [];
    for (
      let sampleY = Math.max(0, y - radiusPixels);
      sampleY <= Math.min(this.height - 1, y + radiusPixels);
      sampleY += 1
    ) {
      for (
        let sampleX = Math.max(0, x - radiusPixels);
        sampleX <= Math.min(this.width - 1, x + radiusPixels);
        sampleX += 1
      ) {
        const offset = (sampleX + sampleY * this.width) * PIXEL_BYTES;
        const descriptor = view.getUint32(offset + 12, true);
        samples.push({
          x: sampleX,
          y: sampleY,
          ownerLow: view.getUint32(offset, true),
          ownerHigh: view.getUint32(offset + 4, true),
          source: descriptor & 0xf,
          level: (descriptor >>> 4) & 0xf,
          reverseZDepth: view.getFloat32(offset + 16, true),
        });
      }
    }
    return samples;
  }
}

function readTerrainOwnershipAttachment(
  png: Uint8Array,
  metadata: PlayerScreenshotMetadata,
): TerrainOwnershipAttachment | null {
  if (!metadata.attachments.terrainPixelOwnership.populated) return null;
  const payload = readPngBinary(png, ATTACHMENT_TYPE);
  if (payload === undefined || payload.byteLength < ATTACHMENT_HEADER_BYTES) {
    throw new Error("player screenshot omitted the vpDI terrain ownership attachment");
  }
  const header = new DataView(payload.buffer, payload.byteOffset, ATTACHMENT_HEADER_BYTES);
  if (
    header.getUint32(0) !== ATTACHMENT_MAGIC ||
    header.getUint16(4) !== 1 ||
    header.getUint16(6) !== ATTACHMENT_CHANNELS
  ) {
    throw new Error("player screenshot terrain ownership attachment is unsupported");
  }
  const width = header.getUint32(8);
  const height = header.getUint32(12);
  const expectedBytes = header.getUint32(16);
  if (
    width !== metadata.image.pixelWidth ||
    height !== metadata.image.pixelHeight ||
    expectedBytes !== width * height * PIXEL_BYTES
  ) {
    throw new Error("player screenshot ownership attachment disagrees with its metadata");
  }
  const pixels = new Uint8Array(inflateSync(payload.subarray(ATTACHMENT_HEADER_BYTES)));
  if (pixels.byteLength !== expectedBytes) {
    throw new Error("player screenshot ownership attachment decompressed to the wrong size");
  }
  return new TerrainOwnershipAttachment(
    width,
    height,
    pixels,
    metadata.attachments.terrainPixelOwnership.worldPositionReconstruction
      .inverseViewProjectionColumns,
  );
}
