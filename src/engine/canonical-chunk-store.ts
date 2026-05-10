import type { PackedChunkEditDelta } from "./chunk-edit-journal.ts";
import type { EncodedGeneratedChunk } from "./generated-chunk-codec.ts";
import type {
  TransferredGeneratedChunkRenderSummary,
  TransferredGeneratedRenderSummaryRegion,
} from "./generated-chunk-transfer.ts";
import type { ChunkCoordinate, RenderSummaryRegionCoordinate } from "./types.ts";

export const CANONICAL_CHUNK_STORE_SCHEMA_VERSION = 1;

export interface CanonicalWorldKey {
  readonly generationVersion: string;
  readonly seed: number;
  readonly seaLevel: number;
  readonly chunkSize: number;
  readonly maxYExclusive: number;
}

export interface CanonicalChunkRecordMetadata {
  readonly schemaVersion: typeof CANONICAL_CHUNK_STORE_SCHEMA_VERSION;
  readonly key: string;
  readonly worldKey: string;
  readonly generationVersion: string;
  readonly coord: ChunkCoordinate;
  readonly canonicalRevision: number;
  readonly encodedByteLength: number;
  readonly storedAt: number;
}

export interface CachedCanonicalChunk {
  readonly buffer: ArrayBuffer;
  readonly byteLength: number;
  readonly metadata: CanonicalChunkRecordMetadata;
}

export interface CanonicalChunkSummaryRecord {
  readonly key: string;
  readonly worldKey: string;
  readonly coord: ChunkCoordinate;
  readonly canonicalRevision: number;
  readonly summary: TransferredGeneratedChunkRenderSummary;
  readonly storedAt: number;
}

export interface CanonicalRenderSummaryRegionRecord {
  readonly key: string;
  readonly worldKey: string;
  readonly coord: RenderSummaryRegionCoordinate;
  readonly canonicalRevision: number;
  readonly summary: TransferredGeneratedRenderSummaryRegion;
  readonly storedAt: number;
}

export interface ChunkEditJournalRecord {
  readonly key: string;
  readonly worldKey: string;
  readonly coord: ChunkCoordinate;
  readonly baseGenerationVersion: string;
  readonly minRevision: number | null;
  readonly maxRevision: number | null;
  readonly deltas: readonly PackedChunkEditDelta[];
  readonly storedAt: number;
}

export interface CanonicalChunkStore {
  getCanonicalChunk(coord: ChunkCoordinate): Promise<CachedCanonicalChunk | null>;
  putCanonicalChunk(
    coord: ChunkCoordinate,
    chunk: EncodedGeneratedChunk,
    summary: TransferredGeneratedChunkRenderSummary,
    canonicalRevision: number,
  ): Promise<void>;
  getChunkSummary(coord: ChunkCoordinate): Promise<CanonicalChunkSummaryRecord | null>;
  putChunkSummary(
    coord: ChunkCoordinate,
    summary: TransferredGeneratedChunkRenderSummary,
    canonicalRevision: number,
  ): Promise<void>;
  getRegionSummary(coord: RenderSummaryRegionCoordinate): Promise<CanonicalRenderSummaryRegionRecord | null>;
  getEditJournal(coord: ChunkCoordinate): Promise<ChunkEditJournalRecord | null>;
  appendEditDeltas(coord: ChunkCoordinate, deltas: readonly PackedChunkEditDelta[], revision: number): Promise<void>;
  close(): void;
}

export function formatCanonicalWorldKey(worldKey: CanonicalWorldKey): string {
  assertCanonicalWorldKey(worldKey);
  return [
    worldKey.generationVersion,
    worldKey.seed,
    worldKey.seaLevel,
    worldKey.chunkSize,
    worldKey.maxYExclusive,
  ].join(":");
}

export function formatCanonicalChunkKey(worldKey: CanonicalWorldKey, coord: ChunkCoordinate): string {
  return `${formatCanonicalWorldKey(worldKey)}:chunk:${formatChunkCoord(coord)}`;
}

export function formatCanonicalChunkSummaryKey(worldKey: CanonicalWorldKey, coord: ChunkCoordinate): string {
  return `${formatCanonicalWorldKey(worldKey)}:summary:${formatChunkCoord(coord)}`;
}

export function formatCanonicalRegionSummaryKey(
  worldKey: CanonicalWorldKey,
  coord: RenderSummaryRegionCoordinate,
): string {
  assertInteger("region x", coord.x);
  assertInteger("region z", coord.z);
  return `${formatCanonicalWorldKey(worldKey)}:region-summary:${coord.x}:${coord.z}`;
}

export function formatChunkEditJournalKey(worldKey: CanonicalWorldKey, coord: ChunkCoordinate): string {
  return `${formatCanonicalWorldKey(worldKey)}:edits:${formatChunkCoord(coord)}`;
}

export function createCanonicalChunkRecordMetadata(input: {
  readonly worldKey: CanonicalWorldKey;
  readonly coord: ChunkCoordinate;
  readonly canonicalRevision: number;
  readonly encodedByteLength: number;
  readonly storedAt: number;
}): CanonicalChunkRecordMetadata {
  assertRevision(input.canonicalRevision);
  assertPositiveInteger("encodedByteLength", input.encodedByteLength);
  assertFiniteNumber("storedAt", input.storedAt);
  return {
    schemaVersion: CANONICAL_CHUNK_STORE_SCHEMA_VERSION,
    key: formatCanonicalChunkKey(input.worldKey, input.coord),
    worldKey: formatCanonicalWorldKey(input.worldKey),
    generationVersion: input.worldKey.generationVersion,
    coord: cloneChunkCoord(input.coord),
    canonicalRevision: input.canonicalRevision,
    encodedByteLength: input.encodedByteLength,
    storedAt: input.storedAt,
  };
}

export function isCanonicalChunkRecordUsable(
  metadata: CanonicalChunkRecordMetadata,
  worldKey: CanonicalWorldKey,
  coord: ChunkCoordinate,
): boolean {
  return metadata.schemaVersion === CANONICAL_CHUNK_STORE_SCHEMA_VERSION
    && metadata.worldKey === formatCanonicalWorldKey(worldKey)
    && metadata.generationVersion === worldKey.generationVersion
    && metadata.key === formatCanonicalChunkKey(worldKey, coord)
    && sameChunkCoord(metadata.coord, coord)
    && metadata.encodedByteLength > 0;
}

function formatChunkCoord(coord: ChunkCoordinate): string {
  assertInteger("chunk x", coord.x);
  assertInteger("chunk y", coord.y);
  assertInteger("chunk z", coord.z);
  return `${coord.x}:${coord.y}:${coord.z}`;
}

function cloneChunkCoord(coord: ChunkCoordinate): ChunkCoordinate {
  return { x: coord.x, y: coord.y, z: coord.z };
}

function sameChunkCoord(left: ChunkCoordinate, right: ChunkCoordinate): boolean {
  return left.x === right.x && left.y === right.y && left.z === right.z;
}

function assertCanonicalWorldKey(worldKey: CanonicalWorldKey): void {
  if (worldKey.generationVersion.length === 0) {
    throw new Error("Canonical world key generationVersion must not be empty");
  }
  assertInteger("seed", worldKey.seed);
  assertInteger("seaLevel", worldKey.seaLevel);
  assertPositiveInteger("chunkSize", worldKey.chunkSize);
  assertPositiveInteger("maxYExclusive", worldKey.maxYExclusive);
}

function assertRevision(value: number): void {
  if (!Number.isInteger(value) || value < 0) {
    throw new RangeError(`Expected a non-negative canonical revision, received ${value}`);
  }
}

function assertPositiveInteger(label: string, value: number): void {
  if (!Number.isInteger(value) || value <= 0) {
    throw new RangeError(`Expected ${label} to be a positive integer, received ${value}`);
  }
}

function assertInteger(label: string, value: number): void {
  if (!Number.isInteger(value)) {
    throw new RangeError(`Expected ${label} to be an integer, received ${value}`);
  }
}

function assertFiniteNumber(label: string, value: number): void {
  if (!Number.isFinite(value)) {
    throw new RangeError(`Expected ${label} to be finite, received ${value}`);
  }
}
