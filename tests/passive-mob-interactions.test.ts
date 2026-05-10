import { expect, test } from "bun:test";

import { buildPassiveMobInteractionCandidates } from "../src/engine/passive-mob-interactions.ts";
import { samplePassiveMobSightingsWorldUnits } from "../src/engine/passive-mob-sim.ts";
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
