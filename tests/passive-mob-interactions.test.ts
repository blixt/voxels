import { expect, test } from "bun:test";

import {
  buildPassiveMobFeedingTrailClue,
  buildPassiveMobInteractionCandidates,
} from "../src/engine/passive-mob-interactions.ts";
import {
  samplePassiveMobSightingsWorldUnits,
  type PassiveMobSighting,
} from "../src/engine/passive-mob-sim.ts";
import { metersToWorldUnits } from "../src/engine/scale.ts";

test("passive mob interaction candidates expose stable bestiary payloads", () => {
  const sightings = samplePassiveMobSightingsWorldUnits(
    metersToWorldUnits(-120),
    metersToWorldUnits(-650),
    { cap: 3 },
  );
  const candidates = buildPassiveMobInteractionCandidates(sightings, {
    surfaceY: metersToWorldUnits(42),
    maxCount: 2,
  });

  expect(candidates).toHaveLength(2);
  expect(candidates[0]).toMatchObject({
    id: sightings[0]!.id,
    subjectType: "mob",
    name: sightings[0]!.name,
    role: "passive-sighting",
    worldPosition: [
      sightings[0]!.position[0],
      metersToWorldUnits(42),
      sightings[0]!.position[2],
    ],
    priority: 6.75,
    prompts: [{
      verb: "inspect",
      label: `Observe ${sightings[0]!.speciesName}`,
    }],
    skillAwards: [{
      skillId: "naturalist",
      xp: 12,
      reason: "Passive mob sighting",
      awardKey: `passive-mob:${sightings[0]!.id}`,
      onceOnly: true,
    }],
    payload: {
      factionId: sightings[0]!.factionId,
      speciesId: sightings[0]!.speciesId,
      speciesName: sightings[0]!.speciesName,
      moodId: sightings[0]!.moodId,
      regionId: sightings[0]!.regionId,
      routeId: sightings[0]!.routeId,
      caveSystemId: sightings[0]!.caveSystemId,
      flavorTags: [...sightings[0]!.flavorTags],
    },
  });
  expect(candidates[0]!.prompts[0]).toMatchObject({
    description: expect.stringContaining(sightings[0]!.label),
  });
  expect(candidates[0]!.payload).toMatchObject({
    fieldNote: expect.stringContaining("sighted"),
  });
  expect(candidates.map((candidate) => candidate.priority)).toEqual([6.75, 6.7]);
});

test("passive mob interaction candidate caps can suppress all sightings", () => {
  const sightings = samplePassiveMobSightingsWorldUnits(0, 0, { cap: 3 });

  expect(buildPassiveMobInteractionCandidates(sightings, {
    surfaceY: 0,
    maxCount: 0,
  })).toEqual([]);
});

test("passive foragers can point naturalists toward a nearby forage site", () => {
  const source = {
    forageSiteId: "trail-forage:forage-patch:0:0",
    forageSiteName: "Trail Forage Patch",
    forageSiteRole: "forage-patch",
    forageSitePosition: [metersToWorldUnits(12), 0, 0] as const,
    lootId: "trail-forage",
    clueLabel: "edible field sign",
    fieldNote: "Trail forage grows where travel has thinned the scrub.",
  };

  const clue = buildPassiveMobFeedingTrailClue(sighting({
    id: "kwama-brood:kwama-forager:0:0",
    speciesId: "kwama-forager",
  }), source);
  const unsupported = buildPassiveMobFeedingTrailClue(sighting({
    id: "temple-pilgrims:ash-pilgrim:0:0",
    speciesId: "ash-pilgrim",
  }), source);

  expect(clue).toMatchObject({
    sourceMobSightingId: "kwama-brood:kwama-forager:0:0",
    forageSiteId: "trail-forage:forage-patch:0:0",
    forageSiteName: "Trail Forage Patch",
    forageSiteRole: "forage-patch",
    lootId: "trail-forage",
    feedingTrailLabel: "kwama feeding sign points toward Trail Forage Patch",
    distanceMeters: 12,
  });
  expect(clue?.fieldNote).toContain("fresh feeding sign");
  expect(unsupported).toBeNull();
});

test("passive mob candidates carry feeding-trail payloads for supported species", () => {
  const candidates = buildPassiveMobInteractionCandidates([
    sighting({
      id: "kwama-brood:kwama-forager:0:0",
      speciesId: "kwama-forager",
      speciesName: "Kwama Forager",
    }),
    sighting({
      id: "temple-pilgrims:ash-pilgrim:0:0",
      speciesId: "ash-pilgrim",
      speciesName: "Ash Pilgrim",
    }),
  ], {
    surfaceY: 0,
    feedingTrail: {
      forageSiteId: "wetland-reagents:reagent-patch:0:0",
      forageSiteName: "Wetland Reagents Patch",
      forageSiteRole: "reagent-patch",
      forageSitePosition: [metersToWorldUnits(8), 0, 0],
      lootId: "wetland-reagents",
      clueLabel: "reagent field sign",
      fieldNote: "Wetland reagents cluster around damp shade.",
    },
  });

  expect(candidates[0]?.prompts[0]).toMatchObject({
    description: expect.stringContaining("points toward Wetland Reagents Patch"),
  });
  expect(candidates[0]?.payload).toMatchObject({
    feedingTrail: {
      sourceMobSightingId: "kwama-brood:kwama-forager:0:0",
      forageSiteId: "wetland-reagents:reagent-patch:0:0",
      lootId: "wetland-reagents",
    },
  });
  expect(candidates[1]?.payload).toMatchObject({
    feedingTrail: null,
  });
});

function sighting(overrides: Partial<PassiveMobSighting>): PassiveMobSighting {
  return {
    id: "wild-beasts:wild-grazer:0:0",
    name: "Wild Grazer I",
    speciesId: "wild-grazer",
    speciesName: "Wild Grazer",
    factionId: "wild-beasts",
    factionName: "Wild Beasts",
    moodId: "open-grass-patrol",
    moodName: "Open Grass Patrol",
    distanceWorldUnits: metersToWorldUnits(6),
    position: [0, 0, 0],
    label: "Wild Grazer (Wild Beasts, Open Country)",
    regionId: "bitter-coast",
    routeId: "pilgrim-road",
    caveSystemId: null,
    flavorTags: ["browse"],
    ...overrides,
  };
}
