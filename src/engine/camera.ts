import {
  addVec3,
  clamp,
  createLookAtMatrix,
  createOrthographicMatrix,
  crossVec3,
  degToRad,
  normalizeVec3,
  scaleVec3,
  subVec3,
  multiplyMatrices,
} from "./math.ts";
import type { Vec3 } from "./types.ts";
import { VoxelWorld } from "./world.ts";

export interface CameraState {
  target: Vec3;
  yaw: number;
  pitch: number;
  distance: number;
  zoom: number;
}

export interface CameraMatrices {
  eye: Vec3;
  forward: Vec3;
  right: Vec3;
  up: Vec3;
  view: Float32Array;
  projection: Float32Array;
  viewProjection: Float32Array;
}

export interface RayHit {
  voxel: [number, number, number];
  adjacent: [number, number, number];
  distance: number;
  normal: Vec3;
}

export function createDefaultCamera(world: VoxelWorld): CameraState {
  return {
    target: [world.width * 0.5, world.height * 0.2, world.depth * 0.5],
    yaw: degToRad(45),
    pitch: degToRad(-35.264),
    distance: 420,
    zoom: 90,
  };
}

export function buildCameraMatrices(camera: CameraState, aspect: number): CameraMatrices {
  const directionFromTarget = normalizeVec3([
    Math.cos(camera.pitch) * Math.cos(camera.yaw),
    Math.sin(camera.pitch),
    Math.cos(camera.pitch) * Math.sin(camera.yaw),
  ]);
  const eye = subVec3(camera.target, scaleVec3(directionFromTarget, camera.distance));
  const forward = normalizeVec3(subVec3(camera.target, eye));
  const right = normalizeVec3(crossVec3(forward, [0, 1, 0]));
  const up = normalizeVec3(crossVec3(right, forward));
  const horizontal = camera.zoom * aspect;
  const projection = createOrthographicMatrix(-horizontal, horizontal, -camera.zoom, camera.zoom, 0.1, 1000);
  const view = createLookAtMatrix(eye, camera.target, up);
  return {
    eye,
    forward,
    right,
    up,
    view,
    projection,
    viewProjection: multiplyMatrices(projection, view),
  };
}

export function orbitCamera(camera: CameraState, deltaYaw: number, deltaPitch: number): void {
  camera.yaw += deltaYaw;
  camera.pitch = clamp(camera.pitch + deltaPitch, degToRad(-80), degToRad(-15));
}

export function zoomCamera(camera: CameraState, delta: number): void {
  camera.zoom = clamp(camera.zoom + delta, 16, 220);
}

export function panCamera(camera: CameraState, right: Vec3, up: Vec3, deltaX: number, deltaY: number): void {
  camera.target = addVec3(camera.target, addVec3(scaleVec3(right, deltaX), scaleVec3(up, deltaY)));
}

export function createScreenRay(
  camera: CameraState,
  aspect: number,
  canvasWidth: number,
  canvasHeight: number,
  screenX: number,
  screenY: number,
  world: VoxelWorld,
): { origin: Vec3; direction: Vec3 } {
  const matrices = buildCameraMatrices(camera, aspect);
  const ndcX = (screenX / canvasWidth) * 2 - 1;
  const ndcY = 1 - (screenY / canvasHeight) * 2;
  const worldX = ndcX * camera.zoom * aspect;
  const worldY = ndcY * camera.zoom;
  const start = addVec3(
    camera.target,
    addVec3(scaleVec3(matrices.right, worldX), scaleVec3(matrices.up, worldY)),
  );
  const worldRadius = Math.max(world.width, world.height, world.depth) * 1.5;
  return {
    origin: subVec3(start, scaleVec3(matrices.forward, worldRadius)),
    direction: matrices.forward,
  };
}

export function raycastWorld(
  world: VoxelWorld,
  origin: Vec3,
  direction: Vec3,
  maxDistance = 1024,
): RayHit | null {
  const boundsMin: Vec3 = [0, 0, 0];
  const boundsMax: Vec3 = [world.width, world.height, world.depth];
  const range = intersectRayAabb(origin, direction, boundsMin, boundsMax);
  if (!range) {
    return null;
  }

  const epsilon = 1e-4;
  let t = Math.max(range[0], 0) + epsilon;
  const entryPoint: Vec3 = [
    origin[0] + direction[0] * t,
    origin[1] + direction[1] * t,
    origin[2] + direction[2] * t,
  ];
  const previousPoint: Vec3 = [
    origin[0] + direction[0] * Math.max(t - epsilon * 2, 0),
    origin[1] + direction[1] * Math.max(t - epsilon * 2, 0),
    origin[2] + direction[2] * Math.max(t - epsilon * 2, 0),
  ];

  let x = Math.floor(entryPoint[0]);
  let y = Math.floor(entryPoint[1]);
  let z = Math.floor(entryPoint[2]);
  let previous: [number, number, number] = [
    Math.floor(previousPoint[0]),
    Math.floor(previousPoint[1]),
    Math.floor(previousPoint[2]),
  ];
  let hitNormal: Vec3 = [0, 0, 0];

  const stepX = Math.sign(direction[0]) || 1;
  const stepY = Math.sign(direction[1]) || 1;
  const stepZ = Math.sign(direction[2]) || 1;
  const deltaX = direction[0] !== 0 ? Math.abs(1 / direction[0]) : Number.POSITIVE_INFINITY;
  const deltaY = direction[1] !== 0 ? Math.abs(1 / direction[1]) : Number.POSITIVE_INFINITY;
  const deltaZ = direction[2] !== 0 ? Math.abs(1 / direction[2]) : Number.POSITIVE_INFINITY;

  let sideX = direction[0] > 0 ? (Math.floor(entryPoint[0]) + 1 - entryPoint[0]) * deltaX : (entryPoint[0] - Math.floor(entryPoint[0])) * deltaX;
  let sideY = direction[1] > 0 ? (Math.floor(entryPoint[1]) + 1 - entryPoint[1]) * deltaY : (entryPoint[1] - Math.floor(entryPoint[1])) * deltaY;
  let sideZ = direction[2] > 0 ? (Math.floor(entryPoint[2]) + 1 - entryPoint[2]) * deltaZ : (entryPoint[2] - Math.floor(entryPoint[2])) * deltaZ;

  while (t <= range[1] && t <= maxDistance) {
    if (world.inBounds(x, y, z) && world.getVoxel(x, y, z) !== 0) {
      return {
        voxel: [x, y, z],
        adjacent: previous,
        distance: t,
        normal: hitNormal,
      };
    }
    if (sideX < sideY && sideX < sideZ) {
      previous = [x, y, z];
      x += stepX;
      t = sideX;
      sideX += deltaX;
      hitNormal = [-stepX, 0, 0];
    } else if (sideY < sideZ) {
      previous = [x, y, z];
      y += stepY;
      t = sideY;
      sideY += deltaY;
      hitNormal = [0, -stepY, 0];
    } else {
      previous = [x, y, z];
      z += stepZ;
      t = sideZ;
      sideZ += deltaZ;
      hitNormal = [0, 0, -stepZ];
    }
  }

  return null;
}

function intersectRayAabb(origin: Vec3, direction: Vec3, min: Vec3, max: Vec3): [number, number] | null {
  let tMin = -Infinity;
  let tMax = Infinity;
  for (let axis = 0; axis < 3; axis += 1) {
    const value = origin[axis];
    const delta = direction[axis];
    if (delta === 0) {
      if (value < min[axis] || value > max[axis]) {
        return null;
      }
      continue;
    }
    const inv = 1 / delta;
    let t0 = (min[axis] - value) * inv;
    let t1 = (max[axis] - value) * inv;
    if (t0 > t1) {
      [t0, t1] = [t1, t0];
    }
    tMin = Math.max(tMin, t0);
    tMax = Math.min(tMax, t1);
    if (tMin > tMax) {
      return null;
    }
  }
  return [tMin, tMax];
}
