import { expect, test } from "bun:test";

import { describeExplorationObjectives } from "../src/engine/exploration-objectives.ts";

test("exploration objectives start with first bearings around roads and landmarks", () => {
  const snapshot = describeExplorationObjectives({
    discoveredBiomeCount: 1,
    discoveredUndergroundBiomeCount: 0,
    discoveredRegionalVariantCount: 0,
    discoveredLandmarkCount: 1,
    discoveredAncientLandmarkCount: 0,
  });

  expect(snapshot.stageId).toBe("first-bearings");
  expect(snapshot.title).toBe("First Bearings");
  expect(snapshot.progressionHint).toContain("Cartography");
  expect(snapshot.completedCount).toBe(0);
  expect(snapshot.objectives.map((objective) => objective.id)).toEqual([
    "biomes-3",
    "old-road-1",
    "landmarks-3",
  ]);
  expect(snapshot.objectives.find((objective) => objective.id === "old-road-1")?.label).toBe(
    "Find an old road sign",
  );
});

test("exploration objectives advance to pilgrim road after first bearings", () => {
  const snapshot = describeExplorationObjectives({
    discoveredBiomeCount: 4,
    discoveredUndergroundBiomeCount: 0,
    discoveredRegionalVariantCount: 1,
    discoveredLandmarkCount: 4,
    discoveredAncientLandmarkCount: 1,
  });

  expect(snapshot.stageId).toBe("pilgrim-road");
  expect(snapshot.objectives.map((objective) => objective.id)).toEqual([
    "old-road-2",
    "landmarks-6",
    "variants-2",
    "underground-1",
  ]);
  expect(snapshot.objectives[0]?.journalText).toContain("route");
});

test("exploration objectives eventually settle into the deep pilgrimage stage", () => {
  const snapshot = describeExplorationObjectives({
    discoveredBiomeCount: 9,
    discoveredUndergroundBiomeCount: 2,
    discoveredRegionalVariantCount: 3,
    discoveredLandmarkCount: 8,
    discoveredAncientLandmarkCount: 3,
  });

  expect(snapshot.stageId).toBe("deep-pilgrimage");
  expect(snapshot.objectives).toHaveLength(5);
  expect(snapshot.objectives.map((objective) => objective.id)).toEqual([
    "old-road-4",
    "biomes-10",
    "variants-4",
    "underground-3",
    "landmarks-12",
  ]);
  expect(snapshot.completedCount).toBe(0);
});

test("exploration objective progress clamps to its target", () => {
  const snapshot = describeExplorationObjectives({
    discoveredBiomeCount: 30,
    discoveredUndergroundBiomeCount: 30,
    discoveredRegionalVariantCount: 30,
    discoveredLandmarkCount: 30,
    discoveredAncientLandmarkCount: 30,
  });

  expect(snapshot.stageId).toBe("deep-pilgrimage");
  expect(snapshot.objectives.every((objective) => objective.progress === objective.target)).toBe(true);
  expect(snapshot.completedCount).toBe(snapshot.totalCount);
});
