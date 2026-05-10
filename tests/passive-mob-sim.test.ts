import { expect, test } from "bun:test";

import { samplePassiveMobSightingsWorldUnits } from "../src/engine/passive-mob-sim.ts";
import { metersToWorldUnits } from "../src/engine/scale.ts";
import { WORLD_ATLAS } from "../src/engine/world-atlas.ts";

test("passive mob sightings are deterministic for a fixed viewer", () => {
  const viewerX = metersToWorldUnits(-120);
  const viewerZ = metersToWorldUnits(-650);
  const first = samplePassiveMobSightingsWorldUnits(viewerX, viewerZ, { cap: 6 });
  const second = samplePassiveMobSightingsWorldUnits(viewerX, viewerZ, { cap: 6 });

  expect(second).toEqual(first);
  expect(first.length).toBe(6);
  expect(new Set(first.map((sighting) => sighting.id)).size).toBe(first.length);
  expect(first[0]).toMatchObject({
    factionId: "temple-pilgrims",
    speciesId: "ash-pilgrim",
    moodId: "ash-pilgrimage",
    factionName: "Temple Pilgrims",
    moodName: "Ash Pilgrimage",
    regionId: "red-mountain",
  });
  expect(first[0]!.label).toContain("Temple Pilgrims");
  expect(first[0]!.label).toContain("Ash Pilgrimage");
  expect(first[0]!.position).toHaveLength(3);
});

test("passive mob sightings obey cap and sort nearest first", () => {
  const viewerX = metersToWorldUnits(570);
  const viewerZ = metersToWorldUnits(2_080);
  const sightings = samplePassiveMobSightingsWorldUnits(viewerX, viewerZ, {
    cap: 3,
    radiusWorldUnits: metersToWorldUnits(160),
  });

  expect(sightings).toHaveLength(3);
  expect(sightings.map((sighting) => sighting.distanceWorldUnits)).toEqual(
    [...sightings].map((sighting) => sighting.distanceWorldUnits).sort((left, right) => left - right),
  );
  for (const sighting of sightings) {
    expect(sighting.distanceWorldUnits).toBeLessThanOrEqual(metersToWorldUnits(160));
    expect(sighting.id).toMatch(/^[a-z-]+:[a-z-]+:-?\d+:-?\d+$/);
  }

  expect(samplePassiveMobSightingsWorldUnits(viewerX, viewerZ, { cap: 0 })).toEqual([]);
});

test("passive mob sightings reflect biome and encounter flavor", () => {
  const bitterCoast = samplePassiveMobSightingsWorldUnits(
    metersToWorldUnits(-3_600),
    metersToWorldUnits(1_350),
    { cap: 8, radiusWorldUnits: metersToWorldUnits(96) },
  );
  const bitterCoastForager = bitterCoast.find((sighting) => sighting.regionId === "bitter-coast");

  expect(bitterCoastForager).toBeDefined();
  expect(bitterCoastForager).toMatchObject({
    factionId: "marsh-foragers",
    speciesId: "marsh-forager",
    moodId: "blackwater-fog",
  });
  expect(bitterCoastForager!.flavorTags).toContain("blackwater");
  expect(bitterCoastForager!.label).toContain("Blackwater Fog");

  const cave = WORLD_ATLAS.caveSystems.find((system) => system.id === "ash-kwama-ravines")!;
  const caveSightings = samplePassiveMobSightingsWorldUnits(
    metersToWorldUnits(cave.validationAnchor.x),
    metersToWorldUnits(cave.validationAnchor.z),
    { cap: 10, radiusWorldUnits: metersToWorldUnits(72) },
  );
  const kwama = caveSightings.find((sighting) => sighting.caveSystemId === "ash-kwama-ravines");

  expect(kwama).toBeDefined();
  expect(kwama).toMatchObject({
    factionId: "kwama-brood",
    speciesId: "kwama-forager",
    moodId: "cave-threshold",
  });
  expect(kwama!.flavorTags).toContain("kwama-mine");
  expect(kwama!.label).toContain("Cave Threshold");
});
