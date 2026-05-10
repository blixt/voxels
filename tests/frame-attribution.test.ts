import { expect, test } from "bun:test";

import { classifyFrameAttributionCause } from "../src/engine/frame-attribution.ts";

test("frame attribution keeps real LOD work distinct", () => {
  expect(classifyFrameAttributionCause({
    wallMs: 72,
    gameplayMs: 58,
    movementMs: 1,
    streamMs: 2,
    meshMs: 4,
    lodMs: 51,
    renderCpuMs: 6,
    renderSyncMs: 1,
    renderUploadMs: 1,
  })).toBe("LOD");
});

test("frame attribution does not blame tiny LOD work for browser wall gaps", () => {
  expect(classifyFrameAttributionCause({
    wallMs: 66.6,
    gameplayMs: 5,
    movementMs: 0.1,
    streamMs: 0,
    meshMs: 0,
    lodMs: 4.3,
    renderCpuMs: 0.6,
    renderSyncMs: 0,
    renderUploadMs: 0,
  })).toBe("browser or idle");
});

test("frame attribution reports none for empty frames", () => {
  expect(classifyFrameAttributionCause({
    wallMs: 16,
    gameplayMs: 0,
    movementMs: 0,
    streamMs: 0,
    meshMs: 0,
    lodMs: 0,
    renderCpuMs: 0,
    renderSyncMs: 0,
    renderUploadMs: 0,
  })).toBe("none");
});
