import { expect, test } from "bun:test";

import { createFirstPersonCamera } from "../src/engine/first-person-camera.ts";
import { buildTargetingOverlayGeometry } from "../src/engine/targeting-overlay.ts";
import type { VoxelRayHit } from "../src/engine/voxel-raycast.ts";

test("targeting overlay projects a reachable front face into screen space", () => {
  const camera = createFirstPersonCamera([0.5, 0.5, -4], Math.PI * 0.5, 0);
  const hit: VoxelRayHit = {
    voxel: [0, 0, 0],
    adjacent: [0, 0, -1],
    distance: 4,
    normal: [0, 0, -1],
  };

  const overlay = buildTargetingOverlayGeometry(camera, hit, 1000, 800);

  expect(overlay.visible).toBe(true);
  expect(overlay.outlineSegments).toHaveLength(4);
  expect(overlay.facePolygon).toHaveLength(4);
  for (const [x, y] of overlay.facePolygon) {
    expect(x).toBeGreaterThan(0);
    expect(x).toBeLessThan(1000);
    expect(y).toBeGreaterThan(0);
    expect(y).toBeLessThan(800);
  }
});

test("targeting overlay hides voxels that are behind the camera", () => {
  const camera = createFirstPersonCamera([0.5, 0.5, -4], -Math.PI * 0.5, 0);
  const hit: VoxelRayHit = {
    voxel: [0, 0, 0],
    adjacent: [0, 0, -1],
    distance: 4,
    normal: [0, 0, -1],
  };

  const overlay = buildTargetingOverlayGeometry(camera, hit, 1000, 800);

  expect(overlay.visible).toBe(false);
  expect(overlay.outlineSegments).toHaveLength(0);
  expect(overlay.facePolygon).toHaveLength(0);
});
