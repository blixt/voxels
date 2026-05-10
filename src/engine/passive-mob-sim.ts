import { hashUint32 } from "./math.ts";
import {
  describeRpgEncounterFaction,
  describeRpgEncounterMood,
  sampleRpgEncounterWorldUnits,
  type RpgEncounterFactionId,
  type RpgEncounterMoodId,
} from "./rpg-encounters.ts";
import { metersToWorldUnits } from "./scale.ts";
import type { Vec3 } from "./types.ts";

export type PassiveMobSpeciesId =
  | "ash-pilgrim"
  | "ashlander-runner"
  | "kwama-forager"
  | "marsh-forager"
  | "salt-strider"
  | "glass-cairn-sentry"
  | "redleaf-guide"
  | "pack-guar"
  | "road-vagrant"
  | "wild-grazer";

export interface PassiveMobSighting {
  id: string;
  name: string;
  speciesId: PassiveMobSpeciesId;
  speciesName: string;
  factionId: RpgEncounterFactionId;
  factionName: string;
  moodId: RpgEncounterMoodId;
  moodName: string;
  distanceWorldUnits: number;
  position: Vec3;
  label: string;
  regionId: string | null;
  routeId: string | null;
  caveSystemId: string | null;
  flavorTags: readonly string[];
}

export interface PassiveMobSightingOptions {
  radiusWorldUnits?: number;
  cap?: number;
  cellSizeWorldUnits?: number;
}

const DEFAULT_RADIUS_WORLD_UNITS = metersToWorldUnits(96);
const DEFAULT_CELL_SIZE_WORLD_UNITS = metersToWorldUnits(32);
const DEFAULT_CAP = 8;
const MOB_CELL_MARGIN_WORLD_UNITS = metersToWorldUnits(4);
const MOB_HEIGHT_WORLD_UNITS = 0;

export function samplePassiveMobSightingsWorldUnits(
  viewerX: number,
  viewerZ: number,
  options: PassiveMobSightingOptions = {},
): readonly PassiveMobSighting[] {
  const radiusWorldUnits = validPositive(options.radiusWorldUnits, DEFAULT_RADIUS_WORLD_UNITS);
  const cellSizeWorldUnits = validPositive(options.cellSizeWorldUnits, DEFAULT_CELL_SIZE_WORLD_UNITS);
  const cap = Math.max(0, Math.floor(Number.isFinite(options.cap) ? options.cap! : DEFAULT_CAP));
  if (cap === 0) {
    return [];
  }

  const minCellX = Math.floor((viewerX - radiusWorldUnits) / cellSizeWorldUnits);
  const maxCellX = Math.floor((viewerX + radiusWorldUnits) / cellSizeWorldUnits);
  const minCellZ = Math.floor((viewerZ - radiusWorldUnits) / cellSizeWorldUnits);
  const maxCellZ = Math.floor((viewerZ + radiusWorldUnits) / cellSizeWorldUnits);
  const sightings: PassiveMobSighting[] = [];

  for (let cellX = minCellX; cellX <= maxCellX; cellX += 1) {
    for (let cellZ = minCellZ; cellZ <= maxCellZ; cellZ += 1) {
      const sighting = samplePassiveMobCell(viewerX, viewerZ, cellX, cellZ, cellSizeWorldUnits, radiusWorldUnits);
      if (sighting) {
        sightings.push(sighting);
      }
    }
  }

  return sightings
    .sort((left, right) => left.distanceWorldUnits - right.distanceWorldUnits || left.id.localeCompare(right.id))
    .slice(0, cap);
}

function samplePassiveMobCell(
  viewerX: number,
  viewerZ: number,
  cellX: number,
  cellZ: number,
  cellSizeWorldUnits: number,
  radiusWorldUnits: number,
): PassiveMobSighting | null {
  const usableSpan = Math.max(1, cellSizeWorldUnits - MOB_CELL_MARGIN_WORLD_UNITS * 2);
  const x = cellX * cellSizeWorldUnits
    + MOB_CELL_MARGIN_WORLD_UNITS
    + passiveRandom(cellX, cellZ, 0x4d4f_4258) * usableSpan;
  const z = cellZ * cellSizeWorldUnits
    + MOB_CELL_MARGIN_WORLD_UNITS
    + passiveRandom(cellX, cellZ, 0x4d4f_425a) * usableSpan;
  const distanceWorldUnits = Math.hypot(x - viewerX, z - viewerZ);
  if (distanceWorldUnits > radiusWorldUnits) {
    return null;
  }

  const encounter = sampleRpgEncounterWorldUnits(x, z);
  const factionId = encounter.factionHints[0]?.factionId ?? "wild-beasts";
  const speciesId = resolvePassiveMobSpecies(factionId, encounter.flavorTags);
  const speciesName = speciesLabel(speciesId);
  const factionName = describeRpgEncounterFaction(factionId);
  const moodName = describeRpgEncounterMood(encounter.moodId);
  const name = `${speciesName} ${passiveNameSuffix(cellX, cellZ, factionId)}`;

  return {
    id: `${factionId}:${speciesId}:${cellX}:${cellZ}`,
    name,
    speciesId,
    speciesName,
    factionId,
    factionName,
    moodId: encounter.moodId,
    moodName,
    distanceWorldUnits: Number(distanceWorldUnits.toFixed(3)),
    position: [Number(x.toFixed(3)), MOB_HEIGHT_WORLD_UNITS, Number(z.toFixed(3))],
    label: `${speciesName} (${factionName}, ${moodName})`,
    regionId: encounter.regionId,
    routeId: encounter.routeId,
    caveSystemId: encounter.caveSystemId,
    flavorTags: encounter.flavorTags,
  };
}

function resolvePassiveMobSpecies(
  factionId: RpgEncounterFactionId,
  flavorTags: readonly string[],
): PassiveMobSpeciesId {
  switch (factionId) {
    case "temple-pilgrims":
      return "ash-pilgrim";
    case "ashlander-scouts":
      return "ashlander-runner";
    case "kwama-brood":
      return "kwama-forager";
    case "marsh-foragers":
      return "marsh-forager";
    case "salt-causeway-wardens":
      return "salt-strider";
    case "glass-cairn-keepers":
      return "glass-cairn-sentry";
    case "redleaf-guides":
      return "redleaf-guide";
    case "inner-sea-traders":
      return "pack-guar";
    case "opportunist-bandits":
      return "road-vagrant";
    case "wild-beasts":
      return flavorTags.includes("kwama-mine") ? "kwama-forager" : "wild-grazer";
  }
}

function speciesLabel(speciesId: PassiveMobSpeciesId): string {
  switch (speciesId) {
    case "ash-pilgrim":
      return "Ash Pilgrim";
    case "ashlander-runner":
      return "Ashlander Runner";
    case "kwama-forager":
      return "Kwama Forager";
    case "marsh-forager":
      return "Marsh Forager";
    case "salt-strider":
      return "Salt Strider";
    case "glass-cairn-sentry":
      return "Glass Cairn Sentry";
    case "redleaf-guide":
      return "Redleaf Guide";
    case "pack-guar":
      return "Pack Guar";
    case "road-vagrant":
      return "Road Vagrant";
    case "wild-grazer":
      return "Wild Grazer";
  }
}

function passiveNameSuffix(cellX: number, cellZ: number, factionId: RpgEncounterFactionId): string {
  const suffixes = ["I", "II", "III", "IV", "V", "VI"];
  let mixed = Math.imul(cellX, 0x1f12_bb5) ^ Math.imul(cellZ, 0x5f35_6495) ^ 0x4e41_4d45;
  for (let index = 0; index < factionId.length; index += 1) {
    mixed = Math.imul(mixed ^ factionId.charCodeAt(index), 0x45d9_f3b);
  }
  return suffixes[hashUint32(mixed) % suffixes.length]!;
}

function passiveRandom(cellX: number, cellZ: number, salt: number): number {
  const mixed = Math.imul(cellX, 0x27d4_eb2d) ^ Math.imul(cellZ, 0x1656_67b1) ^ salt;
  return hashUint32(mixed) / 0xffff_ffff;
}

function validPositive(value: number | undefined, fallback: number): number {
  return Number.isFinite(value) && value! > 0 ? value! : fallback;
}
