import type { ExplorationInteractionCandidate } from "./exploration-interactions.ts";
import type { PassiveMobSighting } from "./passive-mob-sim.ts";
import { worldUnitsToMeters } from "./scale.ts";

export interface PassiveMobInteractionOptions {
  surfaceY: number;
  maxCount?: number;
}

export function buildPassiveMobInteractionCandidates(
  sightings: readonly PassiveMobSighting[],
  options: PassiveMobInteractionOptions,
): readonly ExplorationInteractionCandidate[] {
  const maxCount = Math.max(0, Math.floor(options.maxCount ?? 3));
  return sightings.slice(0, maxCount).map((sighting, index) => {
    const distanceMeters = worldUnitsToMeters(sighting.distanceWorldUnits);
    return {
      id: sighting.id,
      subjectType: "mob",
      name: sighting.name,
      role: "passive-sighting",
      worldPosition: [sighting.position[0], options.surfaceY, sighting.position[2]],
      interactionRadiusMeters: sighting.distanceWorldUnits + 1,
      priority: 6.75 - index * 0.05,
      prompts: [{
        verb: "inspect",
        label: `Observe ${sighting.speciesName}`,
        description: `${sighting.label} is moving ${formatPassiveMobDistance(distanceMeters)} away.`,
      }],
      flavorText: `${sighting.label} is moving through the nearby terrain.`,
      skillAwards: [{
        skillId: "naturalist",
        xp: 12,
        reason: "Passive mob sighting",
        awardKey: `passive-mob:${sighting.id}`,
        onceOnly: true,
      }],
      payload: {
        factionId: sighting.factionId,
        speciesId: sighting.speciesId,
        speciesName: sighting.speciesName,
        distanceMeters: Number(distanceMeters.toFixed(1)),
        fieldNote: `${sighting.label} sighted ${formatPassiveMobDistance(distanceMeters)} away.`,
        moodId: sighting.moodId,
        regionId: sighting.regionId,
        routeId: sighting.routeId,
        caveSystemId: sighting.caveSystemId,
        flavorTags: [...sighting.flavorTags],
      },
    };
  });
}

function formatPassiveMobDistance(distanceMeters: number): string {
  if (distanceMeters < 10) {
    return `${distanceMeters.toFixed(1)} m`;
  }
  if (distanceMeters < 1000) {
    return `${Math.round(distanceMeters)} m`;
  }
  return `${(distanceMeters / 1000).toFixed(1)} km`;
}
