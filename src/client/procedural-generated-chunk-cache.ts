import type {
  TransferredGeneratedChunkRenderSummary,
  TransferredGeneratedRenderSummaryRegion,
} from "../engine/generated-chunk-transfer.ts";
import {
  deserializeGeneratedChunkRenderSummary,
  deserializeGeneratedRenderSummaryRegion,
  serializeGeneratedRenderSummaryRegion,
} from "../engine/generated-chunk-transfer.ts";
import { mergeGeneratedRenderColumnSummary } from "../engine/generated-render-column-summary.ts";
import {
  GENERATED_RENDER_SUMMARY_REGION_SIZE_CHUNKS,
  upsertGeneratedRenderSummaryRegion,
} from "../engine/generated-render-summary-region.ts";
import {
  CANONICAL_CHUNK_STORE_SCHEMA_VERSION,
  createCanonicalChunkRecordMetadata,
  formatCanonicalChunkKey,
  formatCanonicalChunkSummaryKey,
  formatCanonicalRegionSummaryKey,
  formatCanonicalWorldKey,
  formatChunkEditJournalKey,
  isCanonicalChunkRecordUsable,
  type CanonicalWorldKey,
} from "../engine/canonical-chunk-store.ts";
import type { EncodedGeneratedChunk } from "../engine/generated-chunk-codec.ts";
import type { EncodedDerivedLodChunk } from "../engine/derived-lod-chunk-codec.ts";
import type { AsyncDerivedLodChunkCacheKey } from "../engine/async-chunk-generation.ts";
import { PROCEDURAL_WORLD_GENERATION_VERSION } from "../engine/procedural-generator.ts";
import type { ChunkCoordinate, RenderSummaryRegionCoordinate } from "../engine/types.ts";

const DATABASE_NAME = "voxels-procedural-generated-chunks";
const DATABASE_VERSION = 5;
const CHUNK_STORE_NAME = "chunks";
const SUMMARY_STORE_NAME = "chunk_summaries";
const REGION_SUMMARY_STORE_NAME = "render_summary_regions";
const LOD_CHUNK_STORE_NAME = "lod_chunks";

interface StoredGeneratedChunkRecord {
  key: string;
  encodedBuffer: ArrayBuffer;
  encodedByteLength: number;
  storedAt: number;
  schemaVersion?: typeof CANONICAL_CHUNK_STORE_SCHEMA_VERSION;
  worldKey?: string;
  generationVersion?: string;
  coord?: ChunkCoordinate;
  canonicalRevision?: number;
}

export interface CachedEncodedGeneratedChunk {
  buffer: ArrayBuffer;
  byteLength: number;
}

interface StoredGeneratedChunkSummaryRecord {
  key: string;
  summary: TransferredGeneratedChunkRenderSummary;
  storedAt: number;
  worldKey?: string;
  coord?: ChunkCoordinate;
  canonicalRevision?: number;
}

interface StoredGeneratedRenderSummaryRegionRecord {
  key: string;
  summary: TransferredGeneratedRenderSummaryRegion;
  storedAt: number;
  worldKey?: string;
  coord?: RenderSummaryRegionCoordinate;
  canonicalRevision?: number;
}

interface StoredDerivedLodChunkRecord {
  key: string;
  encodedBuffer: ArrayBuffer;
  encodedByteLength: number;
  storedAt: number;
}

export interface ProceduralGeneratedChunkCache {
  getChunk(coord: ChunkCoordinate): Promise<CachedEncodedGeneratedChunk | null>;
  getChunkSummary(coord: ChunkCoordinate): Promise<TransferredGeneratedChunkRenderSummary | null>;
  getRegionSummary(coord: RenderSummaryRegionCoordinate): Promise<TransferredGeneratedRenderSummaryRegion | null>;
  getLodChunk(key: AsyncDerivedLodChunkCacheKey): Promise<CachedEncodedDerivedLodChunk | null>;
  putChunk(
    coord: ChunkCoordinate,
    chunk: EncodedGeneratedChunk,
    summary: TransferredGeneratedChunkRenderSummary,
  ): Promise<void>;
  putChunkSummary(coord: ChunkCoordinate, summary: TransferredGeneratedChunkRenderSummary): Promise<void>;
  putLodChunk(key: AsyncDerivedLodChunkCacheKey, chunk: EncodedDerivedLodChunk): Promise<void>;
  close(): void;
}

export interface CachedEncodedDerivedLodChunk {
  readonly buffer: ArrayBuffer;
  readonly byteLength: number;
}

export async function openProceduralGeneratedChunkCache(context: {
  seed: number;
  seaLevel: number;
  chunkSize: number;
  maxYExclusive: number;
}): Promise<ProceduralGeneratedChunkCache | null> {
  if (typeof indexedDB === "undefined") {
    return null;
  }
  const database = await openDatabase();
  const worldKey = createProceduralGeneratedChunkCacheWorldKey(context);
  const keyPrefix = formatCanonicalWorldKey(worldKey);
  return {
    async getChunk(coord) {
      const record = await requestToPromise<StoredGeneratedChunkRecord | undefined>(
        database
          .transaction(CHUNK_STORE_NAME, "readonly")
          .objectStore(CHUNK_STORE_NAME)
          .get(formatCanonicalChunkKey(worldKey, coord)),
      );
      const resolvedRecord = record ?? await requestToPromise<StoredGeneratedChunkRecord | undefined>(
        database
          .transaction(CHUNK_STORE_NAME, "readonly")
          .objectStore(CHUNK_STORE_NAME)
          .get(toLegacyChunkKey(keyPrefix, coord)),
      );
      if (!resolvedRecord) {
        return null;
      }
      if (resolvedRecord.schemaVersion !== undefined && !isStoredGeneratedChunkRecordUsable(resolvedRecord, worldKey, coord)) {
        return null;
      }
      return {
        buffer: resolvedRecord.encodedBuffer,
        byteLength: resolvedRecord.encodedByteLength,
      };
    },
    async getChunkSummary(coord) {
      const record = await requestToPromise<StoredGeneratedChunkSummaryRecord | undefined>(
        database
          .transaction(SUMMARY_STORE_NAME, "readonly")
          .objectStore(SUMMARY_STORE_NAME)
          .get(formatCanonicalChunkSummaryKey(worldKey, coord)),
      );
      const resolvedRecord = record ?? await requestToPromise<StoredGeneratedChunkSummaryRecord | undefined>(
        database
          .transaction(SUMMARY_STORE_NAME, "readonly")
          .objectStore(SUMMARY_STORE_NAME)
          .get(toLegacyChunkKey(keyPrefix, coord)),
      );
      return resolvedRecord?.summary ?? null;
    },
    async getRegionSummary(coord) {
      const record = await requestToPromise<StoredGeneratedRenderSummaryRegionRecord | undefined>(
        database
          .transaction(REGION_SUMMARY_STORE_NAME, "readonly")
          .objectStore(REGION_SUMMARY_STORE_NAME)
          .get(formatCanonicalRegionSummaryKey(worldKey, coord)),
      );
      const resolvedRecord = record ?? await requestToPromise<StoredGeneratedRenderSummaryRegionRecord | undefined>(
        database
          .transaction(REGION_SUMMARY_STORE_NAME, "readonly")
          .objectStore(REGION_SUMMARY_STORE_NAME)
          .get(toLegacyRegionKey(keyPrefix, coord)),
      );
      return resolvedRecord?.summary ?? null;
    },
    async getLodChunk(key) {
      const record = await requestToPromise<StoredDerivedLodChunkRecord | undefined>(
        database
          .transaction(LOD_CHUNK_STORE_NAME, "readonly")
          .objectStore(LOD_CHUNK_STORE_NAME)
          .get(toLodChunkKey(keyPrefix, key)),
      );
      if (!record) {
        return null;
      }
      return {
        buffer: record.encodedBuffer,
        byteLength: record.encodedByteLength,
      };
    },
    async putChunk(coord, chunk, summary) {
      const transaction = database.transaction([CHUNK_STORE_NAME, SUMMARY_STORE_NAME, REGION_SUMMARY_STORE_NAME], "readwrite");
      const key = formatCanonicalChunkKey(worldKey, coord);
      const summaryKey = formatCanonicalChunkSummaryKey(worldKey, coord);
      const storedAt = Date.now();
      const canonicalRevision = 0;
      transaction.objectStore(CHUNK_STORE_NAME).put({
        ...createCanonicalChunkRecordMetadata({
          worldKey,
          coord,
          canonicalRevision,
          encodedByteLength: chunk.stats.byteLength,
          storedAt,
        }),
        key,
        encodedBuffer: chunk.buffer.slice(0),
        encodedByteLength: chunk.stats.byteLength,
        storedAt,
      } satisfies StoredGeneratedChunkRecord);
      transaction.objectStore(SUMMARY_STORE_NAME).put({
        key: summaryKey,
        worldKey: keyPrefix,
        coord: { ...coord },
        canonicalRevision,
        summary: cloneTransferredSummary(summary),
        storedAt,
      } satisfies StoredGeneratedChunkSummaryRecord);
      await putMergedRegionSummary(
        transaction.objectStore(REGION_SUMMARY_STORE_NAME),
        worldKey,
        keyPrefix,
        summary,
        storedAt,
        canonicalRevision,
      );
      await transactionToPromise(transaction);
    },
    async putChunkSummary(coord, summary) {
      const transaction = database.transaction([SUMMARY_STORE_NAME, REGION_SUMMARY_STORE_NAME], "readwrite");
      const key = formatCanonicalChunkSummaryKey(worldKey, coord);
      const storedAt = Date.now();
      const canonicalRevision = 0;
      transaction.objectStore(SUMMARY_STORE_NAME).put({
        key,
        worldKey: keyPrefix,
        coord: { ...coord },
        canonicalRevision,
        summary: cloneTransferredSummary(summary),
        storedAt,
      } satisfies StoredGeneratedChunkSummaryRecord);
      await putMergedRegionSummary(
        transaction.objectStore(REGION_SUMMARY_STORE_NAME),
        worldKey,
        keyPrefix,
        summary,
        storedAt,
        canonicalRevision,
      );
      await transactionToPromise(transaction);
    },
    async putLodChunk(key, chunk) {
      const storedAt = Date.now();
      const lodKey = toLodChunkKey(keyPrefix, key);
      const transaction = database.transaction(LOD_CHUNK_STORE_NAME, "readwrite");
      transaction.objectStore(LOD_CHUNK_STORE_NAME).put({
        key: lodKey,
        encodedBuffer: chunk.buffer.slice(0),
        encodedByteLength: chunk.stats.byteLength,
        storedAt,
      } satisfies StoredDerivedLodChunkRecord);
      await transactionToPromise(transaction);
    },
    close() {
      database.close();
    },
  };
}

function openDatabase(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(DATABASE_NAME, DATABASE_VERSION);
    request.onerror = () => reject(request.error ?? new Error("Failed to open procedural chunk cache"));
    request.onupgradeneeded = () => {
      const database = request.result;
      if (!database.objectStoreNames.contains(CHUNK_STORE_NAME)) {
        database.createObjectStore(CHUNK_STORE_NAME, { keyPath: "key" });
      }
      if (!database.objectStoreNames.contains(SUMMARY_STORE_NAME)) {
        database.createObjectStore(SUMMARY_STORE_NAME, { keyPath: "key" });
      }
      if (database.objectStoreNames.contains("column_summaries")) {
        database.deleteObjectStore("column_summaries");
      }
      if (!database.objectStoreNames.contains(REGION_SUMMARY_STORE_NAME)) {
        database.createObjectStore(REGION_SUMMARY_STORE_NAME, { keyPath: "key" });
      }
      if (!database.objectStoreNames.contains(LOD_CHUNK_STORE_NAME)) {
        database.createObjectStore(LOD_CHUNK_STORE_NAME, { keyPath: "key" });
      }
    };
    request.onsuccess = () => resolve(request.result);
  });
}

function requestToPromise<T>(request: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    request.onerror = () => reject(request.error ?? new Error("IndexedDB request failed"));
    request.onsuccess = () => resolve(request.result);
  });
}

function transactionToPromise(transaction: IDBTransaction): Promise<void> {
  return new Promise((resolve, reject) => {
    transaction.onerror = () => reject(transaction.error ?? new Error("IndexedDB transaction failed"));
    transaction.onabort = () => reject(transaction.error ?? new Error("IndexedDB transaction aborted"));
    transaction.oncomplete = () => resolve();
  });
}

export function createProceduralGeneratedChunkCacheWorldKey(context: {
  seed: number;
  seaLevel: number;
  chunkSize: number;
  maxYExclusive: number;
}): CanonicalWorldKey {
  return {
    generationVersion: PROCEDURAL_WORLD_GENERATION_VERSION,
    seed: context.seed,
    seaLevel: context.seaLevel,
    chunkSize: context.chunkSize,
    maxYExclusive: context.maxYExclusive,
  };
}

export function createProceduralGeneratedChunkCacheKeys(
  context: {
    seed: number;
    seaLevel: number;
    chunkSize: number;
    maxYExclusive: number;
  },
  coord: ChunkCoordinate,
): {
  worldKey: string;
  chunkKey: string;
  legacyChunkKey: string;
  chunkSummaryKey: string;
  legacyChunkSummaryKey: string;
  editJournalKey: string;
} {
  const worldKey = createProceduralGeneratedChunkCacheWorldKey(context);
  const keyPrefix = formatCanonicalWorldKey(worldKey);
  return {
    worldKey: keyPrefix,
    chunkKey: formatCanonicalChunkKey(worldKey, coord),
    legacyChunkKey: toLegacyChunkKey(keyPrefix, coord),
    chunkSummaryKey: formatCanonicalChunkSummaryKey(worldKey, coord),
    legacyChunkSummaryKey: toLegacyChunkKey(keyPrefix, coord),
    editJournalKey: formatChunkEditJournalKey(worldKey, coord),
  };
}

function isStoredGeneratedChunkRecordUsable(
  record: StoredGeneratedChunkRecord,
  worldKey: CanonicalWorldKey,
  coord: ChunkCoordinate,
): boolean {
  if (record.schemaVersion === undefined) {
    return true;
  }
  if (record.worldKey === undefined || record.generationVersion === undefined || record.coord === undefined) {
    return false;
  }
  return isCanonicalChunkRecordUsable({
    schemaVersion: record.schemaVersion,
    key: record.key,
    worldKey: record.worldKey,
    generationVersion: record.generationVersion,
    coord: record.coord,
    canonicalRevision: record.canonicalRevision ?? 0,
    encodedByteLength: record.encodedByteLength,
    storedAt: record.storedAt,
  }, worldKey, coord);
}

function toLegacyChunkKey(prefix: string, coord: ChunkCoordinate): string {
  return `${prefix}:${coord.x}:${coord.y}:${coord.z}`;
}

function toLegacyRegionKey(prefix: string, coord: RenderSummaryRegionCoordinate): string {
  return `${prefix}:${coord.x}:${coord.z}`;
}

function toLodChunkKey(prefix: string, key: AsyncDerivedLodChunkCacheKey): string {
  return `${prefix}:lod:${key.editRevision}:${key.lodLevel}:${key.coord.x}:${key.coord.y}:${key.coord.z}`;
}

function cloneTransferredSummary(summary: TransferredGeneratedChunkRenderSummary): TransferredGeneratedChunkRenderSummary {
  return {
    coord: { ...summary.coord },
    coveredColumnCount: summary.coveredColumnCount,
    surfaceY: summary.surfaceY.slice(),
    surfaceMaterial: summary.surfaceMaterial.slice(),
    waterTopY: summary.waterTopY.slice(),
    waterMaterial: summary.waterMaterial.slice(),
    macroCellSize: summary.macroCellSize,
    macroCellsPerAxis: summary.macroCellsPerAxis,
    macroCellStates: summary.macroCellStates.slice(),
    faceOpenMask: summary.faceOpenMask.slice(),
  };
}

async function putMergedRegionSummary(
  regionStore: IDBObjectStore,
  worldKey: CanonicalWorldKey,
  keyPrefix: string,
  summary: TransferredGeneratedChunkRenderSummary,
  storedAt: number,
  canonicalRevision: number,
): Promise<void> {
  const regionCoord = getRegionCoord(summary);
  const existingRecord = await requestToPromise<StoredGeneratedRenderSummaryRegionRecord | undefined>(
    regionStore.get(formatCanonicalRegionSummaryKey(worldKey, regionCoord)),
  );
  const resolvedExistingRecord = existingRecord ?? await requestToPromise<StoredGeneratedRenderSummaryRegionRecord | undefined>(
    regionStore.get(toLegacyRegionKey(keyPrefix, regionCoord)),
  );
  const chunkSize = summary.surfaceY.length > 0
    ? Math.round(Math.sqrt(summary.surfaceY.length))
    : summary.macroCellSize * summary.macroCellsPerAxis;
  const existingRegion = resolvedExistingRecord ? deserializeGeneratedRenderSummaryRegion(resolvedExistingRecord.summary) : null;
  const existingColumnSummary = existingRegion?.columns.find(
    (entry) => entry.chunkX === summary.coord.x && entry.chunkZ === summary.coord.z,
  )?.summary ?? null;
  const mergedColumnSummary = mergeGeneratedRenderColumnSummary(
    existingColumnSummary,
    deserializeGeneratedChunkRenderSummary(summary),
    chunkSize,
  );
  const mergedRegion = upsertGeneratedRenderSummaryRegion(
    existingRegion,
    mergedColumnSummary,
    GENERATED_RENDER_SUMMARY_REGION_SIZE_CHUNKS,
  );
  regionStore.put({
    key: formatCanonicalRegionSummaryKey(worldKey, regionCoord),
    worldKey: keyPrefix,
    coord: { ...regionCoord },
    canonicalRevision,
    summary: cloneTransferredRegionSummary(serializeGeneratedRenderSummaryRegion(mergedRegion).summary),
    storedAt,
  } satisfies StoredGeneratedRenderSummaryRegionRecord);
}

function cloneTransferredRegionSummary(
  summary: TransferredGeneratedRenderSummaryRegion,
): TransferredGeneratedRenderSummaryRegion {
  return {
    regionX: summary.regionX,
    regionZ: summary.regionZ,
    regionSizeChunks: summary.regionSizeChunks,
    columns: summary.columns.map((entry) => ({
      chunkX: entry.chunkX,
      chunkZ: entry.chunkZ,
      summary: {
        chunkX: entry.summary.chunkX,
        chunkZ: entry.summary.chunkZ,
        coveredColumnCount: entry.summary.coveredColumnCount,
        surfaceY: entry.summary.surfaceY.slice(),
        surfaceMaterial: entry.summary.surfaceMaterial.slice(),
        waterTopY: entry.summary.waterTopY.slice(),
        waterMaterial: entry.summary.waterMaterial.slice(),
        minKnownCy: entry.summary.minKnownCy,
        maxKnownCy: entry.summary.maxKnownCy,
        minNonEmptyCy: entry.summary.minNonEmptyCy,
        maxNonEmptyCy: entry.summary.maxNonEmptyCy,
      },
    })),
  };
}

function getRegionCoord(summary: TransferredGeneratedChunkRenderSummary): RenderSummaryRegionCoordinate {
  return {
    x: Math.floor(summary.coord.x / GENERATED_RENDER_SUMMARY_REGION_SIZE_CHUNKS),
    z: Math.floor(summary.coord.z / GENERATED_RENDER_SUMMARY_REGION_SIZE_CHUNKS),
  };
}
