import type { Vec3 } from "./types.ts";
import { metersToWorldUnits } from "./scale.ts";

const MIN_UNDERGROUND_EYE_DEPTH = metersToWorldUnits(0.8);
const MAX_OVERHEAD_SCAN_DISTANCE = metersToWorldUnits(7);

export interface UndergroundDiscoveryWorld {
  getVoxel(x: number, y: number, z: number): number;
  isCollisionMaterial(material: number): boolean;
}

export function resolveObservedUndergroundBiomeId(
  world: UndergroundDiscoveryWorld,
  eyePosition: Vec3,
  surfaceY: number,
  undergroundBiomeId: string | null,
): string | null {
  if (!undergroundBiomeId) {
    return null;
  }
  if (eyePosition[1] > surfaceY - MIN_UNDERGROUND_EYE_DEPTH) {
    return null;
  }
  const sampleX = Math.floor(eyePosition[0]);
  const sampleZ = Math.floor(eyePosition[2]);
  const scanStartY = Math.max(Math.floor(eyePosition[1] + 1), 0);
  const scanEndY = Math.min(
    surfaceY - 1,
    Math.floor(eyePosition[1] + MAX_OVERHEAD_SCAN_DISTANCE),
  );
  for (let y = scanStartY; y <= scanEndY; y += 1) {
    if (world.isCollisionMaterial(world.getVoxel(sampleX, y, sampleZ))) {
      return undergroundBiomeId;
    }
  }
  return null;
}
