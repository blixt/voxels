import type { ExplorationEventPayload } from "./exploration-events.ts";
import type { ExplorationInteractionCandidate } from "./exploration-interactions.ts";
import type { PassiveMobSighting, PassiveMobSpeciesId } from "./passive-mob-sim.ts";
import { metersToWorldUnits, worldUnitsToMeters } from "./scale.ts";

export interface PassiveMobInteractionOptions {
  surfaceY: number;
  maxCount?: number;
  feedingTrail?: PassiveMobFeedingTrailSource | null;
}

export interface PassiveMobFeedingTrailSource {
  forageSiteId: string;
  forageSiteName: string;
  forageSiteRole: string;
  forageSitePosition: readonly [number, number, number];
  lootId: string;
  clueLabel: string;
  fieldNote: string;
}

export interface PassiveMobFeedingTrailClue {
  sourceMobSightingId: string;
  forageSiteId: string;
  forageSiteName: string;
  forageSiteRole: string;
  lootId: string;
  feedingTrailLabel: string;
  fieldNote: string;
  distanceMeters: number;
}

export function buildPassiveMobInteractionCandidates(
  sightings: readonly PassiveMobSighting[],
  options: PassiveMobInteractionOptions,
): readonly ExplorationInteractionCandidate[] {
  const maxCount = Math.max(0, Math.floor(options.maxCount ?? 3));
  return sightings.slice(0, maxCount).map((sighting, index) => {
    const distanceMeters = worldUnitsToMeters(sighting.distanceWorldUnits);
    const feedingTrail = buildPassiveMobFeedingTrailClue(sighting, options.feedingTrail ?? null);
    const feedingTrailPayload: ExplorationEventPayload = feedingTrail
      ? {
          sourceMobSightingId: feedingTrail.sourceMobSightingId,
          forageSiteId: feedingTrail.forageSiteId,
          forageSiteName: feedingTrail.forageSiteName,
          forageSiteRole: feedingTrail.forageSiteRole,
          lootId: feedingTrail.lootId,
          feedingTrailLabel: feedingTrail.feedingTrailLabel,
          fieldNote: feedingTrail.fieldNote,
          distanceMeters: feedingTrail.distanceMeters,
        }
      : null;
    const trailSuffix = feedingTrail ? ` ${feedingTrail.feedingTrailLabel}.` : "";
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
        description: `${sighting.label} is moving ${formatPassiveMobDistance(distanceMeters)} away.${trailSuffix}`,
      }],
      flavorText: `${sighting.label} is moving through the nearby terrain.${trailSuffix}`,
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
        feedingTrail: feedingTrailPayload,
      },
    };
  });
}

export function buildPassiveMobFeedingTrailClue(
  sighting: Pick<PassiveMobSighting, "id" | "speciesId" | "position">,
  source: PassiveMobFeedingTrailSource | null,
): PassiveMobFeedingTrailClue | null {
  if (!source || !supportsFeedingTrail(sighting.speciesId)) {
    return null;
  }
  const distanceMeters = worldUnitsToMeters(Math.hypot(
    source.forageSitePosition[0] - sighting.position[0],
    source.forageSitePosition[2] - sighting.position[2],
  ));
  if (distanceMeters > worldUnitsToMeters(metersToWorldUnits(220))) {
    return null;
  }
  return {
    sourceMobSightingId: sighting.id,
    forageSiteId: source.forageSiteId,
    forageSiteName: source.forageSiteName,
    forageSiteRole: source.forageSiteRole,
    lootId: source.lootId,
    feedingTrailLabel: `${speciesTrailNoun(sighting.speciesId)} points toward ${source.forageSiteName}`,
    fieldNote: `${source.fieldNote} ${source.clueLabel} is reinforced by fresh feeding sign.`,
    distanceMeters: Number(distanceMeters.toFixed(1)),
  };
}

function supportsFeedingTrail(speciesId: PassiveMobSpeciesId): boolean {
  return speciesId === "kwama-forager"
    || speciesId === "marsh-forager"
    || speciesId === "wild-grazer"
    || speciesId === "pack-guar"
    || speciesId === "salt-strider";
}

function speciesTrailNoun(speciesId: PassiveMobSpeciesId): string {
  switch (speciesId) {
    case "kwama-forager":
      return "kwama feeding sign";
    case "marsh-forager":
      return "marsh forager tracks";
    case "pack-guar":
      return "pack guar browse";
    case "salt-strider":
      return "salt strider scrape";
    default:
      return "grazing tracks";
  }
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
