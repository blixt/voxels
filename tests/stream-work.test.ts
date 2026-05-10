import { expect, test } from "bun:test";

import {
  shouldPumpWorldWork,
  shouldRefreshResidency,
  shouldRunMovingLodUpdate,
} from "../src/engine/stream-work.ts";

test("residency refresh continues on the same anchor while chunks are still pending", () => {
  expect(shouldRefreshResidency(false, false, 1)).toBe(true);
  expect(shouldRefreshResidency(false, false, 0)).toBe(false);
  expect(shouldRefreshResidency(false, true, 0)).toBe(true);
  expect(shouldRefreshResidency(true, false, 0)).toBe(true);
});

test("background world work keeps pumping while dirty meshes, pending chunks, or pending LOD remain", () => {
  expect(shouldPumpWorldWork(false, 0, 0)).toBe(false);
  expect(shouldPumpWorldWork(true, 0, 0)).toBe(true);
  expect(shouldPumpWorldWork(false, 2, 0)).toBe(true);
  expect(shouldPumpWorldWork(false, 0, 3)).toBe(true);
  expect(shouldPumpWorldWork(false, 0, 0, 4)).toBe(true);
});

test("moving LOD updates keep progressing pending LOD even while resident stream work is busy", () => {
  expect(shouldRunMovingLodUpdate({
    movementActive: true,
    frameNumber: 8,
    intervalFrames: 4,
    pendingChunks: 25,
    dirtyResidentChunks: 7,
    pendingLodChunks: 12,
  })).toBe(true);
  expect(shouldRunMovingLodUpdate({
    movementActive: true,
    frameNumber: 8,
    intervalFrames: 4,
    pendingChunks: 25,
    dirtyResidentChunks: 7,
    pendingLodChunks: 0,
  })).toBe(false);
  expect(shouldRunMovingLodUpdate({
    movementActive: true,
    frameNumber: 8,
    intervalFrames: 4,
    pendingChunks: 0,
    dirtyResidentChunks: 0,
    pendingLodChunks: 0,
  })).toBe(true);
  expect(shouldRunMovingLodUpdate({
    movementActive: true,
    frameNumber: 9,
    intervalFrames: 4,
    pendingChunks: 0,
    dirtyResidentChunks: 0,
    pendingLodChunks: 12,
  })).toBe(false);
  expect(shouldRunMovingLodUpdate({
    movementActive: false,
    frameNumber: 8,
    intervalFrames: 4,
    pendingChunks: 0,
    dirtyResidentChunks: 0,
    pendingLodChunks: 12,
  })).toBe(false);
});
