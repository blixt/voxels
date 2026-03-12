import {
  buildOpaqueChunkMeshFromInput,
  type MeshMaterialLut,
  type OpaqueChunkMeshingInput,
  type OpaqueChunkMeshGeometry,
} from "../engine/opaque-chunk-mesher.ts";
import type { ChunkCoordinate } from "../engine/types.ts";

interface InitMessage {
  type: "init";
  colors: Uint32Array;
  opaqueMask: Uint8Array;
}

interface MeshMessage {
  type: "mesh";
  requestId: number;
  meshRevision: number;
  input: OpaqueChunkMeshingInput;
}

type WorkerRequestMessage = InitMessage | MeshMessage;

interface ReadyMessage {
  type: "ready";
}

interface MeshedMessage {
  type: "meshed";
  requestId: number;
  coord: ChunkCoordinate;
  meshRevision: number;
  opaqueMesh: OpaqueChunkMeshGeometry;
}

type WorkerResponseMessage = ReadyMessage | MeshedMessage;

let materialLut: MeshMaterialLut | null = null;

self.onmessage = (event: MessageEvent<WorkerRequestMessage>) => {
  const message = event.data;
  if (message.type === "init") {
    materialLut = {
      colors: message.colors,
      opaqueMask: message.opaqueMask,
    };
    const ready: WorkerResponseMessage = { type: "ready" };
    self.postMessage(ready);
    return;
  }
  if (!materialLut) {
    throw new Error("Chunk meshing worker received a mesh request before initialization");
  }
  const opaqueMesh = buildOpaqueChunkMeshFromInput(message.input, materialLut);
  const response: WorkerResponseMessage = {
    type: "meshed",
    requestId: message.requestId,
    coord: message.input.coord,
    meshRevision: message.meshRevision,
    opaqueMesh,
  };
  self.postMessage(response, [opaqueMesh.vertexData, opaqueMesh.indexData.buffer]);
};

export {};
