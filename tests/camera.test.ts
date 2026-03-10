import { expect, test } from "bun:test";

import { buildCameraMatrices, orbitDeltaFromDrag, raycastWorld } from "../src/engine/camera.ts";
import { VoxelWorld } from "../src/engine/world.ts";

test("raycast returns the empty adjacent cell before the hit voxel", () => {
  const world = new VoxelWorld({ width: 4, height: 4, depth: 4 }, 4, [0, 0xffcc8844]);
  world.setVoxel(1, 1, 1, 1);

  const hit = raycastWorld(world, [-1.5, 1.5, 1.5], [1, 0, 0]);

  expect(hit).not.toBeNull();
  expect(hit?.voxel).toEqual([1, 1, 1]);
  expect(hit?.adjacent).toEqual([0, 1, 1]);
});

test("camera matrices map nearer points to smaller clip-space depth", () => {
  const camera = {
    target: [18, 6, 18] as [number, number, number],
    yaw: Math.PI / 4,
    pitch: -35.264 * Math.PI / 180,
    distance: 100,
    zoom: 22,
  };

  const matrices = buildCameraMatrices(camera, 1);
  const nearPoint = [
    camera.target[0] - matrices.forward[0] * 10,
    camera.target[1] - matrices.forward[1] * 10,
    camera.target[2] - matrices.forward[2] * 10,
  ] as [number, number, number];
  const farPoint = [
    camera.target[0] + matrices.forward[0] * 10,
    camera.target[1] + matrices.forward[1] * 10,
    camera.target[2] + matrices.forward[2] * 10,
  ] as [number, number, number];

  expect(projectDepth(matrices.viewProjection, nearPoint)).toBeLessThan(projectDepth(matrices.viewProjection, farPoint));
});

test("dragging downward requests an upward scene tilt", () => {
  const downward = orbitDeltaFromDrag(0, 24);
  const upward = orbitDeltaFromDrag(0, -24);

  expect(downward.yaw).toBe(0);
  expect(downward.pitch).toBeLessThan(0);
  expect(upward.pitch).toBeGreaterThan(0);
});

function projectDepth(matrix: Float32Array, point: [number, number, number]): number {
  const clipZ = matrix[2]! * point[0] + matrix[6]! * point[1] + matrix[10]! * point[2] + matrix[14]!;
  const clipW = matrix[3]! * point[0] + matrix[7]! * point[1] + matrix[11]! * point[2] + matrix[15]!;
  return clipZ / clipW;
}
