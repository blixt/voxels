import type {
  TransferredGeneratedChunk,
  TransferredGeneratedChunkRenderSummary,
  TransferredGeneratedRenderColumnSummary,
} from "../engine/generated-chunk-transfer.ts";
import {
  deserializeGeneratedChunkRenderSummary,
  deserializeGeneratedRenderColumnSummary,
  serializeGeneratedRenderColumnSummary,
} from "../engine/generated-chunk-transfer.ts";
import { mergeGeneratedRenderColumnSummary } from "../engine/generated-render-column-summary.ts";
import { PROCEDURAL_WORLD_GENERATION_VERSION } from "../engine/procedural-generator.ts";
import type { ChunkCoordinate, ColumnCoordinate } from "../engine/types.ts";

const DATABASE_NAME = "voxels-procedural-generated-chunks";
const DATABASE_VERSION = 3;
const CHUNK_STORE_NAME = "chunks";
const SUMMARY_STORE_NAME = "chunk_summaries";
const COLUMN_SUMMARY_STORE_NAME = "column_summaries";

interface StoredGeneratedChunkRecord {
  key: string;
  encodedBuffer: ArrayBuffer;
  encodedByteLength: number;
  storedAt: number;
}

interface StoredGeneratedChunkSummaryRecord {
  key: string;
  summary: TransferredGeneratedChunkRenderSummary;
  storedAt: number;
}

interface StoredGeneratedRenderColumnSummaryRecord {
  key: string;
  summary: TransferredGeneratedRenderColumnSummary;
  storedAt: number;
}

export interface ProceduralGeneratedChunkCache {
  getChunk(coord: ChunkCoordinate): Promise<TransferredGeneratedChunk | null>;
  getChunkSummary(coord: ChunkCoordinate): Promise<TransferredGeneratedChunkRenderSummary | null>;
  getColumnSummary(coord: ColumnCoordinate): Promise<TransferredGeneratedRenderColumnSummary | null>;
  putChunk(
    coord: ChunkCoordinate,
    chunk: TransferredGeneratedChunk,
    summary: TransferredGeneratedChunkRenderSummary,
  ): Promise<void>;
  putChunkSummary(coord: ChunkCoordinate, summary: TransferredGeneratedChunkRenderSummary): Promise<void>;
  putColumnSummary(coord: ColumnCoordinate, summary: TransferredGeneratedRenderColumnSummary): Promise<void>;
  close(): void;
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
  const keyPrefix = [
    PROCEDURAL_WORLD_GENERATION_VERSION,
    context.seed,
    context.seaLevel,
    context.chunkSize,
    context.maxYExclusive,
  ].join(":");
  return {
    async getChunk(coord) {
      const record = await requestToPromise<StoredGeneratedChunkRecord | undefined>(
        database
          .transaction(CHUNK_STORE_NAME, "readonly")
          .objectStore(CHUNK_STORE_NAME)
          .get(toChunkKey(keyPrefix, coord)),
      );
      if (!record) {
        return null;
      }
      return {
        encodedBuffer: record.encodedBuffer,
        encodedByteLength: record.encodedByteLength,
      };
    },
    async getChunkSummary(coord) {
      const record = await requestToPromise<StoredGeneratedChunkSummaryRecord | undefined>(
        database
          .transaction(SUMMARY_STORE_NAME, "readonly")
          .objectStore(SUMMARY_STORE_NAME)
          .get(toChunkKey(keyPrefix, coord)),
      );
      return record?.summary ?? null;
    },
    async getColumnSummary(coord) {
      const record = await requestToPromise<StoredGeneratedRenderColumnSummaryRecord | undefined>(
        database
          .transaction(COLUMN_SUMMARY_STORE_NAME, "readonly")
          .objectStore(COLUMN_SUMMARY_STORE_NAME)
          .get(toColumnKey(keyPrefix, coord)),
      );
      return record?.summary ?? null;
    },
    async putChunk(coord, chunk, summary) {
      const transaction = database.transaction([CHUNK_STORE_NAME, SUMMARY_STORE_NAME, COLUMN_SUMMARY_STORE_NAME], "readwrite");
      const key = toChunkKey(keyPrefix, coord);
      const storedAt = Date.now();
      transaction.objectStore(CHUNK_STORE_NAME).put({
        key,
        encodedBuffer: chunk.encodedBuffer.slice(0),
        encodedByteLength: chunk.encodedByteLength,
        storedAt,
      } satisfies StoredGeneratedChunkRecord);
      transaction.objectStore(SUMMARY_STORE_NAME).put({
        key,
        summary: cloneTransferredSummary(summary),
        storedAt,
      } satisfies StoredGeneratedChunkSummaryRecord);
      const columnStore = transaction.objectStore(COLUMN_SUMMARY_STORE_NAME);
      const existingColumnRecord = await requestToPromise<StoredGeneratedRenderColumnSummaryRecord | undefined>(
        columnStore.get(toColumnKey(keyPrefix, coord)),
      );
      columnStore.put({
        key: toColumnKey(keyPrefix, coord),
        summary: mergeTransferredColumnSummary(existingColumnRecord?.summary ?? null, summary),
        storedAt,
      } satisfies StoredGeneratedRenderColumnSummaryRecord);
      await transactionToPromise(transaction);
    },
    async putChunkSummary(coord, summary) {
      const transaction = database.transaction([SUMMARY_STORE_NAME, COLUMN_SUMMARY_STORE_NAME], "readwrite");
      const key = toChunkKey(keyPrefix, coord);
      const storedAt = Date.now();
      transaction.objectStore(SUMMARY_STORE_NAME).put({
        key,
        summary: cloneTransferredSummary(summary),
        storedAt,
      } satisfies StoredGeneratedChunkSummaryRecord);
      const columnStore = transaction.objectStore(COLUMN_SUMMARY_STORE_NAME);
      const existingColumnRecord = await requestToPromise<StoredGeneratedRenderColumnSummaryRecord | undefined>(
        columnStore.get(toColumnKey(keyPrefix, coord)),
      );
      columnStore.put({
        key: toColumnKey(keyPrefix, coord),
        summary: mergeTransferredColumnSummary(existingColumnRecord?.summary ?? null, summary),
        storedAt,
      } satisfies StoredGeneratedRenderColumnSummaryRecord);
      await transactionToPromise(transaction);
    },
    async putColumnSummary(coord, summary) {
      await requestToPromise(
        database
          .transaction(COLUMN_SUMMARY_STORE_NAME, "readwrite")
          .objectStore(COLUMN_SUMMARY_STORE_NAME)
          .put({
            key: toColumnKey(keyPrefix, coord),
            summary: cloneTransferredColumnSummary(summary),
            storedAt: Date.now(),
          } satisfies StoredGeneratedRenderColumnSummaryRecord),
      );
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
      if (!database.objectStoreNames.contains(COLUMN_SUMMARY_STORE_NAME)) {
        database.createObjectStore(COLUMN_SUMMARY_STORE_NAME, { keyPath: "key" });
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

function toChunkKey(prefix: string, coord: ChunkCoordinate): string {
  return `${prefix}:${coord.x}:${coord.y}:${coord.z}`;
}

function toColumnKey(prefix: string, coord: ColumnCoordinate): string {
  return `${prefix}:${coord.x}:${coord.z}`;
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

function cloneTransferredColumnSummary(
  summary: TransferredGeneratedRenderColumnSummary,
): TransferredGeneratedRenderColumnSummary {
  return {
    chunkX: summary.chunkX,
    chunkZ: summary.chunkZ,
    coveredColumnCount: summary.coveredColumnCount,
    surfaceY: summary.surfaceY.slice(),
    surfaceMaterial: summary.surfaceMaterial.slice(),
    waterTopY: summary.waterTopY.slice(),
    waterMaterial: summary.waterMaterial.slice(),
    minKnownCy: summary.minKnownCy,
    maxKnownCy: summary.maxKnownCy,
    minNonEmptyCy: summary.minNonEmptyCy,
    maxNonEmptyCy: summary.maxNonEmptyCy,
  };
}

function mergeTransferredColumnSummary(
  existing: TransferredGeneratedRenderColumnSummary | null,
  summary: TransferredGeneratedChunkRenderSummary,
): TransferredGeneratedRenderColumnSummary {
  const chunkSize = summary.surfaceY.length > 0
    ? Math.round(Math.sqrt(summary.surfaceY.length))
    : summary.macroCellSize * summary.macroCellsPerAxis;
  const merged = mergeGeneratedRenderColumnSummary(
    existing ? deserializeGeneratedRenderColumnSummary(existing) : null,
    deserializeGeneratedChunkRenderSummary(summary),
    chunkSize,
  );
  return serializeGeneratedRenderColumnSummary(merged).summary;
}
