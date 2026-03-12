import { expect, test } from "bun:test";

import { resolveObservedUndergroundBiomeId } from "../src/engine/underground-discovery.ts";
import { metersToWorldUnits } from "../src/engine/scale.ts";

function createDiscoveryWorld(solidVoxels: ReadonlySet<string>) {
  return {
    getVoxel(x: number, y: number, z: number): number {
      return solidVoxels.has(`${x}:${y}:${z}`) ? 1 : 0;
    },
    isCollisionMaterial(material: number): boolean {
      return material !== 0;
    },
  };
}

test("underground discovery does not trigger while the eye is above the local surface", () => {
  const world = createDiscoveryWorld(new Set(["0:1397:0"]));
  const eyePosition: [number, number, number] = [0.5, 1404, 0.5];

  expect(resolveObservedUndergroundBiomeId(world, eyePosition, 1400, "rooted")).toBeNull();
});

test("underground discovery requires actual overhead cover below the surface", () => {
  const openWorld = createDiscoveryWorld(new Set());
  const roofedWorld = createDiscoveryWorld(new Set(["0:1389:0", "0:1390:0"]));
  const eyePosition: [number, number, number] = [0.5, 1388, 0.5];
  const surfaceY = 1400;

  expect(resolveObservedUndergroundBiomeId(openWorld, eyePosition, surfaceY, "granitic")).toBeNull();
  expect(resolveObservedUndergroundBiomeId(roofedWorld, eyePosition, surfaceY, "granitic")).toBe("granitic");
});

test("underground discovery tolerates realistic player eye positions in caves", () => {
  const roofY = 1360 + metersToWorldUnits(1.2);
  const world = createDiscoveryWorld(new Set([`0:${roofY}:0`]));
  const eyePosition: [number, number, number] = [0.5, 1360, 0.5];

  expect(resolveObservedUndergroundBiomeId(world, eyePosition, 1382, "mycelial")).toBe("mycelial");
});
