import type {
  AsyncChunkMeshingQueue,
  CompletedChunkMeshingJob,
} from "../engine/async-chunk-meshing.ts";
import { toAsyncChunkMeshingKey } from "../engine/async-chunk-meshing.ts";
import type {
  MeshMaterialLut,
  OpaqueChunkMeshingInput,
  OpaqueChunkMeshGeometry,
} from "../engine/opaque-chunk-mesher.ts";
import type { ChunkCoordinate } from "../engine/types.ts";

interface WorkerReadyMessage {
  type: "ready";
}

interface WorkerMeshedMessage {
  type: "meshed";
  requestId: number;
  coord: ChunkCoordinate;
  meshRevision: number;
  opaqueMesh: OpaqueChunkMeshGeometry;
}

type WorkerMessage = WorkerReadyMessage | WorkerMeshedMessage;

interface PendingRequest {
  key: string;
  workerIndex: number;
}

interface WorkerSlot {
  worker: Worker;
  pendingCount: number;
}

const DEFAULT_MAX_PENDING_JOBS = 128;
const CHUNK_MESHING_WORKER_ASSET_URL = "/assets/chunk-meshing-worker.js";

export function createAsyncChunkMeshing(
  materialLut: MeshMaterialLut,
  options: {
    workerCount?: number;
    maxPendingJobs?: number;
  } = {},
): AsyncChunkMeshingQueue | null {
  if (typeof Worker === "undefined") {
    return null;
  }
  const workerCount = Math.max(
    1,
    Math.min(
      2,
      Math.floor(options.workerCount ?? Math.max(1, (globalThis.navigator?.hardwareConcurrency ?? 4) - 3)),
    ),
  );
  const maxPendingJobs = options.maxPendingJobs ?? DEFAULT_MAX_PENDING_JOBS;
  const workerSlots: WorkerSlot[] = [];
  const pendingRequests = new Map<number, PendingRequest>();
  const pendingKeys = new Set<string>();
  const completedMeshes: CompletedChunkMeshingJob[] = [];
  let nextRequestId = 1;

  const handleMessage = (slotIndex: number, event: MessageEvent<WorkerMessage>) => {
    const message = event.data;
    if (message.type === "ready") {
      return;
    }
    const pending = pendingRequests.get(message.requestId);
    if (!pending) {
      return;
    }
    pendingRequests.delete(message.requestId);
    pendingKeys.delete(pending.key);
    workerSlots[slotIndex]!.pendingCount = Math.max(0, workerSlots[slotIndex]!.pendingCount - 1);
    completedMeshes.push({
      coord: message.coord,
      meshRevision: message.meshRevision,
      opaqueMesh: message.opaqueMesh,
    });
  };

  for (let index = 0; index < workerCount; index += 1) {
    const worker = new Worker(CHUNK_MESHING_WORKER_ASSET_URL, {
      type: "module",
      name: `chunk-meshing-${index}`,
    });
    worker.addEventListener("message", (event) => handleMessage(index, event));
    worker.postMessage({
      type: "init",
      colors: materialLut.colors,
      opaqueMask: materialLut.opaqueMask,
    });
    workerSlots.push({
      worker,
      pendingCount: 0,
    });
  }

  return {
    requestChunk(input: OpaqueChunkMeshingInput, meshRevision: number): boolean {
      const key = toAsyncChunkMeshingKey(input.coord.x, input.coord.y, input.coord.z);
      if (pendingKeys.has(key) || pendingKeys.size >= maxPendingJobs) {
        return false;
      }
      let bestWorkerIndex = 0;
      let bestPendingCount = workerSlots[0]!.pendingCount;
      for (let index = 1; index < workerSlots.length; index += 1) {
        const pendingCount = workerSlots[index]!.pendingCount;
        if (pendingCount < bestPendingCount) {
          bestPendingCount = pendingCount;
          bestWorkerIndex = index;
        }
      }
      const requestId = nextRequestId++;
      pendingKeys.add(key);
      pendingRequests.set(requestId, { key, workerIndex: bestWorkerIndex });
      workerSlots[bestWorkerIndex]!.pendingCount += 1;
      workerSlots[bestWorkerIndex]!.worker.postMessage(
        {
          type: "mesh",
          requestId,
          meshRevision,
          input,
        },
        collectMeshingInputTransferables(input),
      );
      return true;
    },
    hasPendingChunk(cx: number, cy: number, cz: number): boolean {
      return pendingKeys.has(toAsyncChunkMeshingKey(cx, cy, cz));
    },
    getPendingCount(): number {
      return pendingKeys.size;
    },
    drainCompletedMeshes(): CompletedChunkMeshingJob[] {
      if (completedMeshes.length === 0) {
        return [];
      }
      return completedMeshes.splice(0, completedMeshes.length);
    },
    dispose(): void {
      for (const slot of workerSlots) {
        slot.worker.terminate();
      }
      workerSlots.length = 0;
      pendingRequests.clear();
      pendingKeys.clear();
      completedMeshes.length = 0;
    },
  };
}

function collectMeshingInputTransferables(input: OpaqueChunkMeshingInput): Transferable[] {
  const transferables: Transferable[] = [input.chunkData.buffer];
  for (const [negativeNeighbor, positiveNeighbor] of input.neighbors) {
    if (negativeNeighbor.faceData) {
      transferables.push(negativeNeighbor.faceData.buffer);
    }
    if (positiveNeighbor.faceData) {
      transferables.push(positiveNeighbor.faceData.buffer);
    }
  }
  return transferables;
}
