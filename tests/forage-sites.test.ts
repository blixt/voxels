import { expect, test } from "bun:test";

import { sampleForageSiteWorldUnits } from "../src/engine/forage-sites.ts";
import { metersToWorldUnits } from "../src/engine/scale.ts";

test("forage sites are deterministic anchors inside local loot cells", () => {
  const area = { lootId: "trail-forage", lootName: "Trail Forage" };
  const worldX = metersToWorldUnits(114);
  const worldZ = metersToWorldUnits(-72);
  const first = sampleForageSiteWorldUnits(worldX, worldZ, area);
  const second = sampleForageSiteWorldUnits(worldX, worldZ, area);
  const cellSize = metersToWorldUnits(32);

  expect(second).toEqual(first);
  expect(first.x).toBeGreaterThanOrEqual(first.cellX * cellSize);
  expect(first.x).toBeLessThan((first.cellX + 1) * cellSize);
  expect(first.z).toBeGreaterThanOrEqual(first.cellZ * cellSize);
  expect(first.z).toBeLessThan((first.cellZ + 1) * cellSize);
  expect(first.x).not.toBe(worldX);
  expect(first.z).not.toBe(worldZ);
  expect(first.role).toBe("forage-patch");
  expect(first.name).toBe("Trail Forage Patch");
  expect(first.clueLabel).toBe("edible field sign");
  expect(first.fieldNote).toContain("Trail Forage");
});

test("forage site roles keep reagents caches relics and salvage distinct", () => {
  const samples = [
    sampleForageSiteWorldUnits(0, 0, { lootId: "wetland-reagents", lootName: "Wetland Reagents" }),
    sampleForageSiteWorldUnits(0, 0, { lootId: "ashlander-cache", lootName: "Trail Clan Cache" }),
    sampleForageSiteWorldUnits(0, 0, { lootId: "shrine-offerings", lootName: "Shrine Offerings" }),
    sampleForageSiteWorldUnits(0, 0, { lootId: "glass-alchemy-trace", lootName: "Glass Alchemy Trace" }),
  ];

  expect(samples.map((site) => site.role)).toEqual([
    "reagent-patch",
    "supply-cache",
    "relic-offering",
    "reagent-patch",
  ]);
  expect(samples.map((site) => site.clueLabel)).toEqual([
    "reagent field sign",
    "cache field sign",
    "ritual field sign",
    "reagent field sign",
  ]);
  expect(samples.map((site) => site.fieldNote)).toEqual([
    "Wetland Reagents clusters around damp shade and disturbed mineral seams.",
    "Trail Clan Cache is hidden by a repeated traveler habit, not by random chance.",
    "Shrine Offerings has been left deliberately, tucked away from weather and casual hands.",
    "Glass Alchemy Trace clusters around damp shade and disturbed mineral seams.",
  ]);
  expect(new Set(samples.map((site) => site.id)).size).toBe(samples.length);
});

test("vegetation forage sites anchor to visible source landmarks", () => {
  const worldX = metersToWorldUnits(12);
  const worldZ = metersToWorldUnits(-18);
  const first = sampleForageSiteWorldUnits(worldX, worldZ, {
    lootId: "berry-bush-forage",
    lootName: "Berry Bush Forage",
    forageSourceLandmarkId: "berry_bush",
  });
  const second = sampleForageSiteWorldUnits(worldX, worldZ, {
    lootId: "berry-bush-forage",
    lootName: "Berry Bush Forage",
    forageSourceLandmarkId: "berry_bush",
  });
  const generic = sampleForageSiteWorldUnits(worldX, worldZ, {
    lootId: "berry-bush-forage",
    lootName: "Berry Bush Forage",
  });

  expect(second).toEqual(first);
  expect(first).toMatchObject({
    id: `berry-bush-forage:berry_bush:${first.cellX}:${first.cellZ}`,
    role: "vegetation-forage",
    name: "Berry Bush Forage",
    x: worldX,
    z: worldZ,
    sourceLandmarkId: "berry_bush",
    anchoredToVisibleLandmark: true,
    clueLabel: "berry bush field sign",
  });
  expect(first.fieldNote).toContain("visible on the berry bush");
  expect(generic.id).not.toBe(first.id);
  expect(generic.role).toBe("forage-patch");
  expect(generic.anchoredToVisibleLandmark).toBe(false);
});
