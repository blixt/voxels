import type {
  TransferredGeneratedChunk,
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
import { PROCEDURAL_WORLD_GENERATION_VERSION } from "../engine/procedural-generator.ts";
import type { ChunkCoordinate, RenderSummaryRegionCoordinate } from "../engine/types.ts";

const DATABASE_NAME = "voxels-procedural-generated-chunks";
const DATABASE_VERSION = 4;
const CHUNK_STORE_NAME = "chunks";
const SUMMARY_STORE_NAME = "chunk_summaries";
const REGION_SUMMARY_STORE_NAME = "render_summary_regions";

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

interface StoredGeneratedRenderSummaryRegionRecord {
  key: string;
  summary: TransferredGeneratedRenderSummaryRegion;
  storedAt: number;
}

export interface ProceduralGeneratedChunkCache {
  getChunk(coord: ChunkCoordinate): Promise<TransferredGeneratedChunk | null>;
  getChunkSummary(coord: ChunkCoordinate): Promise<TransferredGeneratedChunkRenderSummary | null>;
  getRegionSummary(coord: RenderSummaryRegionCoordinate): Promise<TransferredGeneratedRenderSummaryRegion | null>;
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
    async getRegionSummary(coord) {
      const record = await requestToPromise<StoredGeneratedRenderSummaryRegionRecord | undefined>(
        database
          .transaction(REGION_SUMMARY_STORE_NAME, "readonly")
          .objectStore(REGION_SUMMARY_STORE_NAME)
          .get(toRegionKey(keyPrefix, coord)),
      );
      return record?.summary ?? null;
    },
    async putChunk(coord, chunk, summary) {
      const transaction = database.transaction([CHUNK_STORE_NAME, SUMMARY_STORE_NAME, REGION_SUMMARY_STORE_NAME], "readwrite");
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
      await putMergedRegionSummary(transaction.objectStore(REGION_SUMMARY_STORE_NAME), keyPrefix, summary, storedAt);
      await transactionToPromise(transaction);
    },
    async putChunkSummary(coord, summary) {
      const transaction = database.transaction([SUMMARY_STORE_NAME, REGION_SUMMARY_STORE_NAME], "readwrite");
      const key = toChunkKey(keyPrefix, coord);
      const storedAt = Date.now();
      transaction.objectStore(SUMMARY_STORE_NAME).put({
        key,
        summary: cloneTransferredSummary(summary),
        storedAt,
      } satisfies StoredGeneratedChunkSummaryRecord);
      await putMergedRegionSummary(transaction.objectStore(REGION_SUMMARY_STORE_NAME), keyPrefix, summary, storedAt);
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

function toRegionKey(prefix: string, coord: RenderSummaryRegionCoordinate): string {
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

async function putMergedRegionSummary(
  regionStore: IDBObjectStore,
  keyPrefix: string,
  summary: TransferredGeneratedChunkRenderSummary,
  storedAt: number,
): Promise<void> {
  const regionCoord = getRegionCoord(summary);
  const existingRecord = await requestToPromise<StoredGeneratedRenderSummaryRegionRecord | undefined>(
    regionStore.get(toRegionKey(keyPrefix, regionCoord)),
  );
  const chunkSize = summary.surfaceY.length > 0
    ? Math.round(Math.sqrt(summary.surfaceY.length))
    : summary.macroCellSize * summary.macroCellsPerAxis;
  const existingRegion = existingRecord ? deserializeGeneratedRenderSummaryRegion(existingRecord.summary) : null;
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
    key: toRegionKey(keyPrefix, regionCoord),
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
