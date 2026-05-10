import { expect, test } from "bun:test";

import {
  buildFirstPersonCameraMatrices,
  createFirstPersonCamera,
  createForwardRay,
  FIRST_PERSON_FAR_DISTANCE,
  FIRST_PERSON_NEAR_DISTANCE,
  rotateFirstPersonCamera,
} from "../src/engine/first-person-camera.ts";
import { FOG_END_DISTANCE } from "../src/engine/render-constants.ts";

test("moving the mouse downward pitches the first-person camera downward", () => {
  const camera = createFirstPersonCamera([0, 0, 0]);
  const originalPitch = camera.pitch;

  rotateFirstPersonCamera(camera, 0, 24);

  expect(camera.pitch).toBeLessThan(originalPitch);
});

test("forward ray uses the camera position and a normalized direction", () => {
  const camera = createFirstPersonCamera([5, 7, 11], 0.35, -0.45);

  const ray = createForwardRay(camera);

  expect(ray.origin).toEqual([5, 7, 11]);
  expect(Math.hypot(ray.direction[0], ray.direction[1], ray.direction[2])).toBeCloseTo(1, 5);
});

test("first-person projection keeps enough depth precision through the fog range", () => {
  const camera = createFirstPersonCamera([24, 18, 12], 0.65, -0.2);

  expect(camera.near).toBe(FIRST_PERSON_NEAR_DISTANCE);
  expect(camera.far).toBe(FIRST_PERSON_FAR_DISTANCE);
  expect(camera.far / camera.near).toBeLessThan(1500);

  const matrices = buildFirstPersonCameraMatrices(camera, 16 / 9);
  const fogEdgeDistance = FOG_END_DISTANCE - 1;
  const fogEdgePoint = [
    camera.position[0] + matrices.forward[0] * fogEdgeDistance,
    camera.position[1] + matrices.forward[1] * fogEdgeDistance,
    camera.position[2] + matrices.forward[2] * fogEdgeDistance,
  ] as [number, number, number];
  const oneVoxelFartherPoint = [
    camera.position[0] + matrices.forward[0] * (fogEdgeDistance + 1),
    camera.position[1] + matrices.forward[1] * (fogEdgeDistance + 1),
    camera.position[2] + matrices.forward[2] * (fogEdgeDistance + 1),
  ] as [number, number, number];
  const depth24Step = 1 / 16_777_216;

  expect(projectDepth(matrices.viewProjection, oneVoxelFartherPoint) - projectDepth(matrices.viewProjection, fogEdgePoint))
    .toBeGreaterThan(depth24Step * 2);
});

test("perspective camera matrices map nearer points to smaller clip-space depth", () => {
  const camera = createFirstPersonCamera([24, 18, 12], 0.65, -0.2);
  const matrices = buildFirstPersonCameraMatrices(camera, 16 / 9);
  const nearPoint = [
    camera.position[0] + matrices.forward[0] * 4,
    camera.position[1] + matrices.forward[1] * 4,
    camera.position[2] + matrices.forward[2] * 4,
  ] as [number, number, number];
  const farPoint = [
    camera.position[0] + matrices.forward[0] * 40,
    camera.position[1] + matrices.forward[1] * 40,
    camera.position[2] + matrices.forward[2] * 40,
  ] as [number, number, number];

  expect(projectDepth(matrices.viewProjection, nearPoint)).toBeLessThan(projectDepth(matrices.viewProjection, farPoint));
});

function projectDepth(matrix: Float32Array, point: [number, number, number]): number {
  const clipZ = matrix[2]! * point[0] + matrix[6]! * point[1] + matrix[10]! * point[2] + matrix[14]!;
  const clipW = matrix[3]! * point[0] + matrix[7]! * point[1] + matrix[11]! * point[2] + matrix[15]!;
  return clipZ / clipW;
}
