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
  const material = world.getVoxel(hit.voxel[0], hit.voxel[1], hit.voxel[2]);
  if (material === 0) {
    return { changed: false, hit, material: null };
  }
  if (getInventoryInsertCapacity(inventory, material) < 1) {
    return { changed: false, hit, material };
  }
  const changed = world.setVoxel(hit.voxel[0], hit.voxel[1], hit.voxel[2], 0);
  if (!changed) {
    return { changed: false, hit, material };
  }
  insertInventoryMaterial(inventory, material, 1);
  return { changed: true, hit, material };
}

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
  const target = hit.adjacent;
  if (world.getVoxel(target[0], target[1], target[2]) !== 0) {
    return { changed: false, hit, material: selected.material };
  }
  const changed = world.setVoxel(target[0], target[1], target[2], selected.material);
  if (!changed) {
    return { changed: false, hit, material: selected.material };
  }
  removeSelectedInventoryMaterial(inventory, 1);
  return { changed: true, hit, material: selected.material };
}
