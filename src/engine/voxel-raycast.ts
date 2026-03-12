import type { Vec3 } from "./types.ts";
import type { ResidentChunkWorld } from "./world.ts";

export interface VoxelRayHit {
  voxel: [number, number, number];
  adjacent: [number, number, number];
  distance: number;
  normal: Vec3;
}

export function raycastResidentWorld(
  world: ResidentChunkWorld,
  origin: Vec3,
  direction: Vec3,
  maxDistance = 1024,
): VoxelRayHit | null {
  const epsilon = 1e-4;
  let x = Math.floor(origin[0]);
  let y = Math.floor(origin[1]);
  let z = Math.floor(origin[2]);
  let previous: [number, number, number] = [
    Math.floor(origin[0] - direction[0] * epsilon),
    Math.floor(origin[1] - direction[1] * epsilon),
    Math.floor(origin[2] - direction[2] * epsilon),
  ];
  let hitNormal: Vec3 = [0, 0, 0];

  const stepX = Math.sign(direction[0]) || 1;
  const stepY = Math.sign(direction[1]) || 1;
  const stepZ = Math.sign(direction[2]) || 1;
  const deltaX = direction[0] !== 0 ? Math.abs(1 / direction[0]) : Number.POSITIVE_INFINITY;
  const deltaY = direction[1] !== 0 ? Math.abs(1 / direction[1]) : Number.POSITIVE_INFINITY;
  const deltaZ = direction[2] !== 0 ? Math.abs(1 / direction[2]) : Number.POSITIVE_INFINITY;

  let sideX = direction[0] > 0
    ? (Math.floor(origin[0]) + 1 - origin[0]) * deltaX
    : (origin[0] - Math.floor(origin[0])) * deltaX;
  let sideY = direction[1] > 0
    ? (Math.floor(origin[1]) + 1 - origin[1]) * deltaY
    : (origin[1] - Math.floor(origin[1])) * deltaY;
  let sideZ = direction[2] > 0
    ? (Math.floor(origin[2]) + 1 - origin[2]) * deltaZ
    : (origin[2] - Math.floor(origin[2])) * deltaZ;

  let traveled = 0;
  while (traveled <= maxDistance) {
    if (y >= world.minY && y < world.maxYExclusive) {
      const voxel = world.getVoxel(x, y, z);
      if (voxel !== 0) {
        return {
          voxel: [x, y, z],
          adjacent: previous,
          distance: traveled,
          normal: hitNormal,
        };
      }
    }

    previous = [x, y, z];
    if (sideX <= sideY && sideX <= sideZ) {
      x += stepX;
      traveled = sideX;
      sideX += deltaX;
      hitNormal = [-stepX, 0, 0];
    } else if (sideY <= sideZ) {
      y += stepY;
      traveled = sideY;
      sideY += deltaY;
      hitNormal = [0, -stepY, 0];
    } else {
      z += stepZ;
      traveled = sideZ;
      sideZ += deltaZ;
      hitNormal = [0, 0, -stepZ];
    }
  }
  return null;
}
