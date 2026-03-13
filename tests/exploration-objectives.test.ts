import { expect, test } from "bun:test";

import { describeExplorationObjectives } from "../src/engine/exploration-objectives.ts";

test("exploration objectives start in the surface survey stage", () => {
  const snapshot = describeExplorationObjectives({
    discoveredBiomeCount: 1,
    discoveredUndergroundBiomeCount: 0,
    discoveredRegionalVariantCount: 0,
    discoveredLandmarkCount: 1,
    collectedMaterialCount: 0,
  });

  expect(snapshot.stageId).toBe("surface-survey");
  expect(snapshot.completedCount).toBe(0);
  expect(snapshot.objectives.map((objective) => objective.id)).toEqual([
    "biomes-3",
    "landmarks-3",
    "colors-4",
  ]);
});

test("exploration objectives advance to frontier atlas after the first survey stage", () => {
  const snapshot = describeExplorationObjectives({
    discoveredBiomeCount: 4,
    discoveredUndergroundBiomeCount: 0,
    discoveredRegionalVariantCount: 0,
    discoveredLandmarkCount: 4,
    collectedMaterialCount: 5,
  });

  expect(snapshot.stageId).toBe("frontier-atlas");
  expect(snapshot.objectives.map((objective) => objective.id)).toEqual([
    "biomes-6",
    "variants-2",
    "colors-8",
    "underground-1",
  ]);
});

test("exploration objectives eventually settle into the deep expedition stage", () => {
  const snapshot = describeExplorationObjectives({
    discoveredBiomeCount: 9,
    discoveredUndergroundBiomeCount: 2,
    discoveredRegionalVariantCount: 3,
    discoveredLandmarkCount: 8,
    collectedMaterialCount: 12,
  });

  expect(snapshot.stageId).toBe("deep-expedition");
  expect(snapshot.objectives).toHaveLength(5);
  expect(snapshot.completedCount).toBe(1);
});
