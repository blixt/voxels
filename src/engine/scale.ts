export const VOXEL_SIZE_METERS = 0.1;
export const VOXELS_PER_METER = 1 / VOXEL_SIZE_METERS;

export function metersToWorldUnits(meters: number): number {
  return meters * VOXELS_PER_METER;
}

export function worldUnitsToMeters(worldUnits: number): number {
  return worldUnits * VOXEL_SIZE_METERS;
}
