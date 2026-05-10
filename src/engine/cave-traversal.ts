import {
  PLAYER_EYE_HEIGHT,
  PLAYER_HEIGHT,
  PLAYER_RADIUS,
} from "./player-physics.ts";
import { metersToWorldUnits } from "./scale.ts";
import type { Vec3 } from "./types.ts";

export interface CaveTraversalWorld {
  getVoxel(x: number, y: number, z: number): number;
  isCollisionMaterial(material: number): boolean;
  isWaterMaterial(material: number): boolean;
}

export interface CaveEntrySearchInput {
  world: CaveTraversalWorld;
  anchorPosition: readonly [number, number, number];
  surfaceY: number;
  maxHorizontalDistance?: number;
  minDepth?: number;
  maxDepth?: number;
}

const DEFAULT_MAX_HORIZONTAL_DISTANCE = metersToWorldUnits(3.6);
const DEFAULT_MIN_DEPTH = metersToWorldUnits(1.2);
const DEFAULT_MAX_DEPTH = metersToWorldUnits(22);
const MAX_OVERHEAD_SCAN_DISTANCE = metersToWorldUnits(7);

export function findSafeCaveEntryFeetPosition(input: CaveEntrySearchInput): Vec3 | null {
  const maxHorizontalDistance = sanitizePositive(input.maxHorizontalDistance) ?? DEFAULT_MAX_HORIZONTAL_DISTANCE;
  const minDepth = sanitizePositive(input.minDepth) ?? DEFAULT_MIN_DEPTH;
  const maxDepth = Math.max(minDepth, sanitizePositive(input.maxDepth) ?? DEFAULT_MAX_DEPTH);
  const anchorX = Math.floor(input.anchorPosition[0]);
  const anchorZ = Math.floor(input.anchorPosition[2]);
  const shallowestFeetY = Math.floor(input.surfaceY - minDepth - PLAYER_EYE_HEIGHT);
  const deepestFeetY = Math.floor(input.surfaceY - maxDepth);

  for (const [offsetX, offsetZ] of buildSearchOffsets(maxHorizontalDistance)) {
    const feetX = anchorX + offsetX + 0.5;
    const feetZ = anchorZ + offsetZ + 0.5;
    for (let feetY = shallowestFeetY; feetY >= deepestFeetY; feetY -= 1) {
      const feet: Vec3 = [feetX, feetY, feetZ];
      if (canStandInCave(input.world, feet, input.surfaceY)) {
        return feet;
      }
    }
  }
  return null;
}

function canStandInCave(world: CaveTraversalWorld, feet: Vec3, surfaceY: number): boolean {
  const eyeY = feet[1] + PLAYER_EYE_HEIGHT;
  if (eyeY > surfaceY - metersToWorldUnits(0.8)) {
    return false;
  }
  if (!hasSolidFloor(world, feet)) {
    return false;
  }
  if (bodyCollides(world, feet)) {
    return false;
  }
  if (isWaterAt(world, feet[0], feet[1] + 1, feet[2]) || isWaterAt(world, feet[0], eyeY, feet[2])) {
    return false;
  }
  return hasOverheadCaveMaterial(world, feet, surfaceY);
}

function hasSolidFloor(world: CaveTraversalWorld, feet: Vec3): boolean {
  return world.isCollisionMaterial(world.getVoxel(
    Math.floor(feet[0]),
    Math.floor(feet[1] - 1),
    Math.floor(feet[2]),
  ));
}

function bodyCollides(world: CaveTraversalWorld, feet: Vec3): boolean {
  const minX = Math.floor(feet[0] - PLAYER_RADIUS + 0.001);
  const maxX = Math.floor(feet[0] + PLAYER_RADIUS - 0.001);
  const minY = Math.floor(feet[1] + 0.001);
  const maxY = Math.floor(feet[1] + PLAYER_HEIGHT - 0.001);
  const minZ = Math.floor(feet[2] - PLAYER_RADIUS + 0.001);
  const maxZ = Math.floor(feet[2] + PLAYER_RADIUS - 0.001);
  for (let z = minZ; z <= maxZ; z += 1) {
    for (let y = minY; y <= maxY; y += 1) {
      for (let x = minX; x <= maxX; x += 1) {
        if (world.isCollisionMaterial(world.getVoxel(x, y, z))) {
          return true;
        }
      }
    }
  }
  return false;
}

function isWaterAt(world: CaveTraversalWorld, x: number, y: number, z: number): boolean {
  return world.isWaterMaterial(world.getVoxel(Math.floor(x), Math.floor(y), Math.floor(z)));
}

function hasOverheadCaveMaterial(world: CaveTraversalWorld, feet: Vec3, surfaceY: number): boolean {
  const x = Math.floor(feet[0]);
  const z = Math.floor(feet[2]);
  const scanStartY = Math.floor(feet[1] + PLAYER_EYE_HEIGHT + 1);
  const scanEndY = Math.min(Math.floor(surfaceY - 1), Math.floor(scanStartY + MAX_OVERHEAD_SCAN_DISTANCE));
  for (let y = scanStartY; y <= scanEndY; y += 1) {
    if (world.isCollisionMaterial(world.getVoxel(x, y, z))) {
      return true;
    }
  }
  return false;
}

function buildSearchOffsets(maxHorizontalDistance: number): Array<[number, number]> {
  const limit = Math.max(0, Math.floor(maxHorizontalDistance));
  const offsets: Array<[number, number]> = [];
  for (let z = -limit; z <= limit; z += 1) {
    for (let x = -limit; x <= limit; x += 1) {
      if (Math.hypot(x, z) <= maxHorizontalDistance) {
        offsets.push([x, z]);
      }
    }
  }
  offsets.sort((left, right) => Math.hypot(left[0], left[1]) - Math.hypot(right[0], right[1])
    || left[1] - right[1]
    || left[0] - right[0]);
  return offsets;
}

function sanitizePositive(value: number | undefined): number | null {
  return typeof value === "number" && Number.isFinite(value) && value > 0 ? value : null;
}
