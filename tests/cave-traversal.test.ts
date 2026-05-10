import { expect, test } from "bun:test";

import { findSafeCaveEntryFeetPosition } from "../src/engine/cave-traversal.ts";
import { PLAYER_EYE_HEIGHT } from "../src/engine/player-physics.ts";
import { metersToWorldUnits } from "../src/engine/scale.ts";

function createCaveWorld(options: {
  solids: ReadonlySet<string>;
  waters?: ReadonlySet<string>;
}) {
  return {
    getVoxel(x: number, y: number, z: number): number {
      const key = `${x}:${y}:${z}`;
      if (options.waters?.has(key)) {
        return 2;
      }
      return options.solids.has(key) ? 1 : 0;
    },
    isCollisionMaterial(material: number): boolean {
      return material === 1;
    },
    isWaterMaterial(material: number): boolean {
      return material === 2;
    },
  };
}

function addFloor(solids: Set<string>, y: number, radius = 12): void {
  for (let z = -radius; z <= radius; z += 1) {
    for (let x = -radius; x <= radius; x += 1) {
      solids.add(`${x}:${y}:${z}`);
    }
  }
}

function addRoof(solids: Set<string>, y: number, radius = 12): void {
  for (let z = -radius; z <= radius; z += 1) {
    for (let x = -radius; x <= radius; x += 1) {
      solids.add(`${x}:${y}:${z}`);
    }
  }
}

function addWaterColumn(waters: Set<string>, y: number, radius = 12): void {
  for (let z = -radius; z <= radius; z += 1) {
    for (let x = -radius; x <= radius; x += 1) {
      waters.add(`${x}:${y}:${z}`);
    }
  }
}

test("cave traversal finds a supported covered player-sized interior", () => {
  const solids = new Set<string>();
  addFloor(solids, 99);
  addRoof(solids, 128);
  const target = findSafeCaveEntryFeetPosition({
    world: createCaveWorld({ solids }),
    anchorPosition: [0, 140, 0],
    surfaceY: 140,
  });

  expect(target).not.toBeNull();
  expect(target![1]).toBe(100);
  expect(target![1] + PLAYER_EYE_HEIGHT).toBeLessThan(140 - metersToWorldUnits(0.8));
});

test("cave traversal rejects open shafts without overhead cave cover", () => {
  const solids = new Set<string>();
  addFloor(solids, 99);

  expect(findSafeCaveEntryFeetPosition({
    world: createCaveWorld({ solids }),
    anchorPosition: [0, 140, 0],
    surfaceY: 140,
  })).toBeNull();
});

test("cave traversal rejects flooded entry cells", () => {
  const solids = new Set<string>();
  addFloor(solids, 99);
  addRoof(solids, 128);
  const waters = new Set<string>();
  addWaterColumn(waters, 101);

  expect(findSafeCaveEntryFeetPosition({
    world: createCaveWorld({ solids, waters }),
    anchorPosition: [0, 140, 0],
    surfaceY: 140,
  })).toBeNull();
});
