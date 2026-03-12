/// <reference lib="webworker" />

import { ProceduralWorldGenerator } from "../engine/procedural-generator.ts";
import { serializeGeneratedChunk, type TransferredGeneratedChunk } from "../engine/generated-chunk-transfer.ts";
import type { ChunkCoordinate } from "../engine/types.ts";

type WorkerRequest =
  | {
      type: "init";
      seed: number;
      seaLevel: number;
      chunkSize: number;
      maxYExclusive: number;
    }
  | {
      type: "generate";
      requestId: number;
      coord: ChunkCoordinate;
    };

type WorkerResponse =
  | {
      type: "ready";
    }
  | {
      type: "generated";
      requestId: number;
      chunk: TransferredGeneratedChunk;
    };

let generator: ProceduralWorldGenerator | null = null;

self.onmessage = (event: MessageEvent<WorkerRequest>) => {
  const message = event.data;
  if (message.type === "init") {
    generator = new ProceduralWorldGenerator(message.seed, {
      seaLevel: message.seaLevel,
      chunkSize: message.chunkSize,
      maxYExclusive: message.maxYExclusive,
    });
    const response: WorkerResponse = { type: "ready" };
    self.postMessage(response);
    return;
  }
  if (!generator) {
    throw new Error("Procedural generation worker received a generate request before initialization");
  }
  const generated = generator.generateChunk(message.coord.x, message.coord.y, message.coord.z);
  const serialized = serializeGeneratedChunk(generated);
  const response: WorkerResponse = {
    type: "generated",
    requestId: message.requestId,
    chunk: serialized.chunk,
  };
  self.postMessage(response, { transfer: serialized.transfer });
};

export {};
