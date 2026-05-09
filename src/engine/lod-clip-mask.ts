export const LOD_SUBVOXEL_CLIP_MASK_FULL = 0xff;

export type LodSubvoxelCoverage = "none" | "partial" | "full";

export interface LodSubvoxelBox {
  readonly min: readonly [number, number, number];
  readonly max: readonly [number, number, number];
}

export function lodSubvoxelBit(localX: number, localY: number, localZ: number): number {
  if (!isValidSubvoxelCoord(localX) || !isValidSubvoxelCoord(localY) || !isValidSubvoxelCoord(localZ)) {
    throw new RangeError(`LOD subvoxel coords must be 0 or 1, received ${localX},${localY},${localZ}`);
  }
  return 1 << (localX + localY * 2 + localZ * 4);
}

export function buildLodSubvoxelClipMask(
  isSubvoxelOwned: (localX: number, localY: number, localZ: number) => boolean,
): number {
  let mask = 0;
  for (let localZ = 0; localZ < 2; localZ += 1) {
    for (let localY = 0; localY < 2; localY += 1) {
      for (let localX = 0; localX < 2; localX += 1) {
        if (isSubvoxelOwned(localX, localY, localZ)) {
          mask |= lodSubvoxelBit(localX, localY, localZ);
        }
      }
    }
  }
  return mask;
}

export function classifyLodSubvoxelClipMask(mask: number): LodSubvoxelCoverage {
  const normalized = normalizeLodSubvoxelClipMask(mask);
  if (normalized === 0) {
    return "none";
  }
  return normalized === LOD_SUBVOXEL_CLIP_MASK_FULL ? "full" : "partial";
}

export function isLodSubvoxelClipped(mask: number, localX: number, localY: number, localZ: number): boolean {
  return (normalizeLodSubvoxelClipMask(mask) & lodSubvoxelBit(localX, localY, localZ)) !== 0;
}

export function listUnclippedLodSubvoxelBoxes(mask: number): LodSubvoxelBox[] {
  const normalized = normalizeLodSubvoxelClipMask(mask);
  const boxes: LodSubvoxelBox[] = [];
  for (let localZ = 0; localZ < 2; localZ += 1) {
    for (let localY = 0; localY < 2; localY += 1) {
      for (let localX = 0; localX < 2; localX += 1) {
        if ((normalized & lodSubvoxelBit(localX, localY, localZ)) !== 0) {
          continue;
        }
        boxes.push({
          min: [localX / 2, localY / 2, localZ / 2],
          max: [(localX + 1) / 2, (localY + 1) / 2, (localZ + 1) / 2],
        });
      }
    }
  }
  return boxes;
}

function normalizeLodSubvoxelClipMask(mask: number): number {
  return mask & LOD_SUBVOXEL_CLIP_MASK_FULL;
}

function isValidSubvoxelCoord(coord: number): boolean {
  return Number.isInteger(coord) && coord >= 0 && coord <= 1;
}
