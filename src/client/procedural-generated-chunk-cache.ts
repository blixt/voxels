import type {
  TransferredGeneratedChunk,
  TransferredGeneratedChunkRenderSummary,
} from "../engine/generated-chunk-transfer.ts";
import { PROCEDURAL_WORLD_GENERATION_VERSION } from "../engine/procedural-generator.ts";
import type { ChunkCoordinate } from "../engine/types.ts";

const DATABASE_NAME = "voxels-procedural-generated-chunks";
const DATABASE_VERSION = 2;
const CHUNK_STORE_NAME = "chunks";
const SUMMARY_STORE_NAME = "chunk_summaries";

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

export interface ProceduralGeneratedChunkCache {
  getChunk(coord: ChunkCoordinate): Promise<TransferredGeneratedChunk | null>;
  getChunkSummary(coord: ChunkCoordinate): Promise<TransferredGeneratedChunkRenderSummary | null>;
  putChunk(
    coord: ChunkCoordinate,
    chunk: TransferredGeneratedChunk,
    summary: TransferredGeneratedChunkRenderSummary,
  ): Promise<void>;
  putChunkSummary(coord: ChunkCoordinate, summary: TransferredGeneratedChunkRenderSummary): Promise<void>;
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
    async putChunk(coord, chunk, summary) {
      const transaction = database.transaction([CHUNK_STORE_NAME, SUMMARY_STORE_NAME], "readwrite");
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
      await transactionToPromise(transaction);
    },
    async putChunkSummary(coord, summary) {
      const key = toChunkKey(keyPrefix, coord);
      await requestToPromise(
        database
          .transaction(SUMMARY_STORE_NAME, "readwrite")
          .objectStore(SUMMARY_STORE_NAME)
          .put({
            key,
            summary: cloneTransferredSummary(summary),
            storedAt: Date.now(),
          } satisfies StoredGeneratedChunkSummaryRecord),
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
