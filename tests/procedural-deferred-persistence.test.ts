import { expect, test } from "bun:test";

import {
  countDeferredProceduralPersistenceJobs,
  createDeferredProceduralPersistenceQueue,
  enqueueDeferredProceduralPersistenceJob,
  shiftDeferredProceduralPersistenceJob,
} from "../src/client/procedural-deferred-persistence.ts";
import type { GeneratedChunkRenderSummary } from "../src/engine/generated-chunk-render-summary.ts";
import type { TransferredGeneratedChunkRenderSummary } from "../src/engine/generated-chunk-transfer.ts";
import type { GeneratedChunk } from "../src/engine/procedural-generator.ts";

function createRenderSummary(x: number, y: number, z: number): GeneratedChunkRenderSummary {
  return {
    coord: { x, y, z },
    coveredColumnCount: 1,
    surfaceY: new Int32Array([16]),
    surfaceMaterial: new Uint16Array([123]),
    waterTopY: new Int32Array([0]),
    waterMaterial: new Uint16Array([0]),
    macroCellSize: 4,
    macroCellsPerAxis: 1,
    macroCellStates: new Uint8Array([1]),
    faceOpenMask: new Uint8Array([0, 0, 0, 0, 0, 0]),
  };
}

function createSummary(x: number, y: number, z: number): TransferredGeneratedChunkRenderSummary {
  return createRenderSummary(x, y, z);
}

function createChunk(x: number, y: number, z: number): GeneratedChunk {
  return {
    coord: { x, y, z },
    data: new Uint16Array([1, 2, 3, 4, 5, 6, 7, 8]),
    solidCount: 8,
    solidBounds: {
      min: [0, 0, 0],
      max: [2, 2, 2],
    },
    renderSummary: createRenderSummary(x, y, z),
  };
}

test("deferred persistence queue dedupes repeated summary writes", () => {
  const queue = createDeferredProceduralPersistenceQueue();

  enqueueDeferredProceduralPersistenceJob(queue, {
    type: "summary",
    coord: { x: 1, y: 2, z: 3 },
    summary: createSummary(1, 2, 3),
  });
  enqueueDeferredProceduralPersistenceJob(queue, {
    type: "summary",
    coord: { x: 1, y: 2, z: 3 },
    summary: createSummary(1, 2, 3),
  });

  expect(countDeferredProceduralPersistenceJobs(queue)).toBe(1);
  const job = shiftDeferredProceduralPersistenceJob(queue);
  expect(job?.type).toBe("summary");
  expect(countDeferredProceduralPersistenceJobs(queue)).toBe(0);
});

test("chunk persistence supersedes an earlier summary write for the same coord", () => {
  const queue = createDeferredProceduralPersistenceQueue();

  enqueueDeferredProceduralPersistenceJob(queue, {
    type: "summary",
    coord: { x: 4, y: 5, z: 6 },
    summary: createSummary(4, 5, 6),
  });
  enqueueDeferredProceduralPersistenceJob(queue, {
    type: "chunk",
    coord: { x: 4, y: 5, z: 6 },
    chunk: createChunk(4, 5, 6),
  });

  expect(countDeferredProceduralPersistenceJobs(queue)).toBe(1);
  const job = shiftDeferredProceduralPersistenceJob(queue);
  expect(job?.type).toBe("chunk");
  expect(job?.coord).toEqual({ x: 4, y: 5, z: 6 });
});

test("summary persistence cannot replace an already queued chunk write", () => {
  const queue = createDeferredProceduralPersistenceQueue();

  enqueueDeferredProceduralPersistenceJob(queue, {
    type: "chunk",
    coord: { x: 7, y: 8, z: 9 },
    chunk: createChunk(7, 8, 9),
  });
  enqueueDeferredProceduralPersistenceJob(queue, {
    type: "summary",
    coord: { x: 7, y: 8, z: 9 },
    summary: createSummary(7, 8, 9),
  });

  expect(countDeferredProceduralPersistenceJobs(queue)).toBe(1);
  const job = shiftDeferredProceduralPersistenceJob(queue);
  expect(job?.type).toBe("chunk");
  expect(job?.coord).toEqual({ x: 7, y: 8, z: 9 });
});

test("queue order is preserved when a queued summary is upgraded to a chunk", () => {
  const queue = createDeferredProceduralPersistenceQueue();

  enqueueDeferredProceduralPersistenceJob(queue, {
    type: "summary",
    coord: { x: 1, y: 1, z: 1 },
    summary: createSummary(1, 1, 1),
  });
  enqueueDeferredProceduralPersistenceJob(queue, {
    type: "summary",
    coord: { x: 2, y: 2, z: 2 },
    summary: createSummary(2, 2, 2),
  });
  enqueueDeferredProceduralPersistenceJob(queue, {
    type: "chunk",
    coord: { x: 1, y: 1, z: 1 },
    chunk: createChunk(1, 1, 1),
  });

  const first = shiftDeferredProceduralPersistenceJob(queue);
  const second = shiftDeferredProceduralPersistenceJob(queue);

  expect(first?.type).toBe("chunk");
  expect(first?.coord).toEqual({ x: 1, y: 1, z: 1 });
  expect(second?.type).toBe("summary");
  expect(second?.coord).toEqual({ x: 2, y: 2, z: 2 });
});
