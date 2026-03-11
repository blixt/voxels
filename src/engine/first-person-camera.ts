import {
  addVec3,
  clamp,
  createLookAtMatrix,
  createPerspectiveMatrix,
  crossVec3,
  degToRad,
  multiplyMatrices,
  normalizeVec3,
} from "./math.ts";
import type { Vec3 } from "./types.ts";

const LOOK_YAW_PER_PIXEL = 0.0032;
const LOOK_PITCH_PER_PIXEL = 0.0024;
const MIN_PITCH = degToRad(-89);
const MAX_PITCH = degToRad(89);

export interface FirstPersonCameraState {
  position: Vec3;
  yaw: number;
  pitch: number;
  fovY: number;
  near: number;
  far: number;
}

export interface FirstPersonCameraMatrices {
  position: Vec3;
  forward: Vec3;
  right: Vec3;
  up: Vec3;
  view: Float32Array;
  projection: Float32Array;
  viewProjection: Float32Array;
}

export function createFirstPersonCamera(
  position: Vec3,
  yaw = degToRad(45),
  pitch = degToRad(-18),
): FirstPersonCameraState {
  return {
    position: [...position],
    yaw,
    pitch,
    fovY: degToRad(68),
    near: 0.1,
    far: 20000,
  };
}

export function buildFirstPersonCameraMatrices(
  camera: FirstPersonCameraState,
  aspect: number,
): FirstPersonCameraMatrices {
  const forward = normalizeVec3([
    Math.cos(camera.pitch) * Math.cos(camera.yaw),
    Math.sin(camera.pitch),
    Math.cos(camera.pitch) * Math.sin(camera.yaw),
  ]);
  const right = normalizeVec3(crossVec3(forward, [0, 1, 0]));
  const up = normalizeVec3(crossVec3(right, forward));
  const view = createLookAtMatrix(camera.position, addVec3(camera.position, forward), up);
  const projection = createPerspectiveMatrix(camera.fovY, aspect, camera.near, camera.far);
  return {
    position: [...camera.position],
    forward,
    right,
    up,
    view,
    projection,
    viewProjection: multiplyMatrices(projection, view),
  };
}

export function rotateFirstPersonCamera(
  camera: FirstPersonCameraState,
  deltaX: number,
  deltaY: number,
): void {
  camera.yaw += deltaX * LOOK_YAW_PER_PIXEL;
  camera.pitch = clamp(camera.pitch - deltaY * LOOK_PITCH_PER_PIXEL, MIN_PITCH, MAX_PITCH);
}

export function createForwardRay(camera: FirstPersonCameraState): {
  origin: Vec3;
  direction: Vec3;
} {
  const matrices = buildFirstPersonCameraMatrices(camera, 1);
  return {
    origin: [...camera.position],
    direction: matrices.forward,
  };
}
