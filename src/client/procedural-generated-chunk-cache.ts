import type { TransferredGeneratedChunk } from "../engine/generated-chunk-transfer.ts";
import { PROCEDURAL_WORLD_GENERATION_VERSION } from "../engine/procedural-generator.ts";
import type { ChunkCoordinate } from "../engine/types.ts";

const DATABASE_NAME = "voxels-procedural-generated-chunks";
const DATABASE_VERSION = 1;
const STORE_NAME = "chunks";

interface StoredGeneratedChunkRecord {
  key: string;
  encodedBuffer: ArrayBuffer;
  encodedByteLength: number;
  storedAt: number;
}

export interface ProceduralGeneratedChunkCache {
  getChunk(coord: ChunkCoordinate): Promise<TransferredGeneratedChunk | null>;
  putChunk(coord: ChunkCoordinate, chunk: TransferredGeneratedChunk): Promise<void>;
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
          .transaction(STORE_NAME, "readonly")
          .objectStore(STORE_NAME)
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
    async putChunk(coord, chunk) {
      await requestToPromise(
        database
          .transaction(STORE_NAME, "readwrite")
          .objectStore(STORE_NAME)
          .put({
            key: toChunkKey(keyPrefix, coord),
            encodedBuffer: chunk.encodedBuffer.slice(0),
            encodedByteLength: chunk.encodedByteLength,
            storedAt: Date.now(),
          } satisfies StoredGeneratedChunkRecord),
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
      if (!database.objectStoreNames.contains(STORE_NAME)) {
        database.createObjectStore(STORE_NAME, { keyPath: "key" });
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

function toChunkKey(prefix: string, coord: ChunkCoordinate): string {
  return `${prefix}:${coord.x}:${coord.y}:${coord.z}`;
}
