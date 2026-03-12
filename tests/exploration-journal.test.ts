import { expect, test } from "bun:test";

import { ExplorationJournal } from "../src/engine/exploration-journal.ts";

test("exploration journal records biome, underground, variant, and landmark discoveries once", () => {
  const journal = new ExplorationJournal();

  journal.observe({
    biomeId: "verdant",
    undergroundBiomeId: "rooted",
    regionalVariantId: "verdant_karst",
    landmarkIds: ["redwood", "berry_bush", "redwood"],
    currentLandmarkId: "redwood",
  });
  journal.observe({
    biomeId: "verdant",
    undergroundBiomeId: "rooted",
    regionalVariantId: "verdant_karst",
    landmarkIds: ["berry_bush"],
    currentLandmarkId: "berry_bush",
  });

  const snapshot = journal.getSnapshot();

  expect(snapshot.currentBiomeId).toBe("verdant");
  expect(snapshot.currentUndergroundBiomeId).toBe("rooted");
  expect(snapshot.currentRegionalVariantId).toBe("verdant_karst");
  expect(snapshot.currentLandmarkId).toBe("berry_bush");
  expect(snapshot.discoveredBiomeIds).toEqual(["verdant"]);
  expect(snapshot.discoveredUndergroundBiomeIds).toEqual(["rooted"]);
  expect(snapshot.discoveredRegionalVariantIds).toEqual(["verdant_karst"]);
  expect(snapshot.discoveredLandmarkIds).toEqual(["berry_bush", "redwood"]);
  expect(snapshot.recentDiscoveries).toHaveLength(5);
  expect(snapshot.lastDiscovery?.label).toBe("Landmark: Berry Bush [berry_bush]");
  expect(snapshot.lastDiscovery?.name).toBe("Berry Bush");
  expect(snapshot.lastDiscovery?.sequence).toBe(5);
});

test("exploration journal reset clears discovered state", () => {
  const journal = new ExplorationJournal();

  journal.observe({
    biomeId: "savanna",
    undergroundBiomeId: "saline",
    regionalVariantId: null,
    landmarkIds: ["acacia"],
    currentLandmarkId: "acacia",
  });
  journal.reset();

  const snapshot = journal.getSnapshot();

  expect(snapshot.currentBiomeId).toBeNull();
  expect(snapshot.currentUndergroundBiomeId).toBeNull();
  expect(snapshot.currentRegionalVariantId).toBeNull();
  expect(snapshot.currentLandmarkId).toBeNull();
  expect(snapshot.discoveredBiomeIds).toEqual([]);
  expect(snapshot.discoveredUndergroundBiomeIds).toEqual([]);
  expect(snapshot.discoveredRegionalVariantIds).toEqual([]);
  expect(snapshot.discoveredLandmarkIds).toEqual([]);
  expect(snapshot.recentDiscoveries).toEqual([]);
  expect(snapshot.lastDiscovery).toBeNull();
});
