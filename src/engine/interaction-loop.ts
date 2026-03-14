import {
  getInventoryInsertCapacity,
  getSelectedInventoryStack,
  insertInventoryMaterial,
  removeSelectedInventoryMaterial,
  type InventoryState,
} from "./inventory.ts";
import { raycastResidentWorld, type VoxelRayHit } from "./voxel-raycast.ts";
import type { Vec3 } from "./types.ts";
import type { MutableResidentChunkWorld } from "./world.ts";

export interface VoxelInteractionResult {
  changed: boolean;
  hit: VoxelRayHit | null;
  material: number | null;
}

const BREAK_SPHERE_RADIUS = 5;

export function breakVoxelAlongRay(
  world: MutableResidentChunkWorld,
  inventory: InventoryState,
  origin: Vec3,
  direction: Vec3,
  maxDistance = 96,
): VoxelInteractionResult {
  const hit = raycastResidentWorld(world, origin, direction, maxDistance);
  if (!hit) {
    return { changed: false, hit: null, material: null };
  }
  const centerX = hit.voxel[0];
  const centerY = hit.voxel[1];
  const centerZ = hit.voxel[2];
  const centerMaterial = world.getVoxel(centerX, centerY, centerZ);
  if (centerMaterial === 0) {
    return { changed: false, hit, material: null };
  }
  // Break a sphere of voxels around the hit point
  const r = BREAK_SPHERE_RADIUS;
  const r2 = r * r;
  let changed = false;
  for (let dz = -r; dz <= r; dz++) {
    for (let dy = -r; dy <= r; dy++) {
      for (let dx = -r; dx <= r; dx++) {
        if (dx * dx + dy * dy + dz * dz > r2) continue;
        const vx = centerX + dx;
        const vy = centerY + dy;
        const vz = centerZ + dz;
        const material = world.getVoxel(vx, vy, vz);
        if (material === 0 || world.isWaterMaterial(material)) continue;
        if (getInventoryInsertCapacity(inventory, material) < 1) continue;
        if (world.setVoxel(vx, vy, vz, 0)) {
          insertInventoryMaterial(inventory, material, 1);
          changed = true;
        }
      }
    }
  }
  return { changed, hit, material: centerMaterial };
}

const PLACE_SPHERE_RADIUS = 5;

export function placeSelectedVoxelAlongRay(
  world: MutableResidentChunkWorld,
  inventory: InventoryState,
  origin: Vec3,
  direction: Vec3,
  maxDistance = 96,
): VoxelInteractionResult {
  const selected = getSelectedInventoryStack(inventory);
  if (!selected || selected.count <= 0) {
    return { changed: false, hit: null, material: null };
  }
  const hit = raycastResidentWorld(world, origin, direction, maxDistance);
  if (!hit) {
    return { changed: false, hit: null, material: null };
  }
  const centerX = hit.adjacent[0];
  const centerY = hit.adjacent[1];
  const centerZ = hit.adjacent[2];
  // Place a sphere of voxels around the adjacent point
  const r = PLACE_SPHERE_RADIUS;
  const r2 = r * r;
  let changed = false;
  for (let dz = -r; dz <= r; dz++) {
    for (let dy = -r; dy <= r; dy++) {
      for (let dx = -r; dx <= r; dx++) {
        if (dx * dx + dy * dy + dz * dz > r2) continue;
        const vx = centerX + dx;
        const vy = centerY + dy;
        const vz = centerZ + dz;
        if (world.getVoxel(vx, vy, vz) !== 0) continue;
        if (selected.count <= 0) break;
        if (world.setVoxel(vx, vy, vz, selected.material)) {
          removeSelectedInventoryMaterial(inventory, 1);
          changed = true;
        }
      }
    }
  }
  return { changed, hit, material: selected.material };
}
