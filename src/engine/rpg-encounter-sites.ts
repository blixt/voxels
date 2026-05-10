import { hashUint32 } from "./math.ts";
import { metersToWorldUnits } from "./scale.ts";
import {
  describeRpgEncounterFaction,
  type RpgEncounterFactionId,
  type RpgEncounterSample,
} from "./rpg-encounters.ts";

export type RpgEncounterSiteKind = "camp" | "lair" | "watch" | "ambush" | "trail";
export type RpgEncounterSiteRole = "mob-spoor" | "mob-nest" | "mob-lair";

export interface RpgEncounterSite {
  id: string;
  kind: RpgEncounterSiteKind;
  role: RpgEncounterSiteRole;
  factionId: RpgEncounterFactionId | null;
  name: string;
  x: number;
  z: number;
  cellX: number;
  cellZ: number;
  interactionRadiusWorldUnits: number;
  priority: number;
  clueLabel: string;
  fieldNote: string;
}

const ENCOUNTER_SITE_CELL_SIZE_WORLD_UNITS = metersToWorldUnits(48);
const ENCOUNTER_SITE_MARGIN_WORLD_UNITS = metersToWorldUnits(9);
const ENCOUNTER_SITE_INTERACTION_RADIUS_WORLD_UNITS = metersToWorldUnits(7);

export function sampleRpgEncounterSiteWorldUnits(
  worldX: number,
  worldZ: number,
  encounter: RpgEncounterSample,
): RpgEncounterSite {
  const cellX = Math.floor(worldX / ENCOUNTER_SITE_CELL_SIZE_WORLD_UNITS);
  const cellZ = Math.floor(worldZ / ENCOUNTER_SITE_CELL_SIZE_WORLD_UNITS);
  const factionId = encounter.factionHints[0]?.factionId ?? null;
  const kind = resolveEncounterSiteKind(factionId, encounter.pressure);
  const role = resolveEncounterSiteRole(factionId, encounter);
  const randomX = siteRandom(cellX, cellZ, 0x4d4f_4258);
  const randomZ = siteRandom(cellX, cellZ, 0x4d4f_425a);
  const usableSpan = ENCOUNTER_SITE_CELL_SIZE_WORLD_UNITS - ENCOUNTER_SITE_MARGIN_WORLD_UNITS * 2;
  const x = cellX * ENCOUNTER_SITE_CELL_SIZE_WORLD_UNITS + ENCOUNTER_SITE_MARGIN_WORLD_UNITS + randomX * usableSpan;
  const z = cellZ * ENCOUNTER_SITE_CELL_SIZE_WORLD_UNITS + ENCOUNTER_SITE_MARGIN_WORLD_UNITS + randomZ * usableSpan;
  const factionLabel = factionId ? describeRpgEncounterFaction(factionId) : "Local";
  const clueLabel = resolveEncounterSiteClueLabel(kind, role);

  return {
    id: `${factionId ?? encounter.moodId}:${role}:${cellX}:${cellZ}`,
    kind,
    role,
    factionId,
    name: `${factionLabel} ${siteKindLabel(kind)}`,
    x,
    z,
    cellX,
    cellZ,
    interactionRadiusWorldUnits: ENCOUNTER_SITE_INTERACTION_RADIUS_WORLD_UNITS,
    priority: resolveEncounterSitePriority(kind, encounter.pressure),
    clueLabel,
    fieldNote: resolveEncounterSiteFieldNote(kind, factionLabel, encounter.pressure),
  };
}

export function describeRpgEncounterSiteKind(kind: RpgEncounterSiteKind): string {
  return siteKindLabel(kind);
}

function resolveEncounterSiteKind(
  factionId: RpgEncounterFactionId | null,
  pressure: number,
): RpgEncounterSiteKind {
  switch (factionId) {
    case "kwama-brood":
    case "wild-beasts":
      return "lair";
    case "temple-pilgrims":
    case "ashlander-scouts":
    case "marsh-foragers":
    case "inner-sea-traders":
      return "camp";
    case "salt-causeway-wardens":
    case "glass-cairn-keepers":
    case "redleaf-guides":
      return "watch";
    case "opportunist-bandits":
      return pressure >= 0.48 ? "ambush" : "trail";
    default:
      return pressure >= 0.72 ? "lair" : "trail";
  }
}

function resolveEncounterSiteRole(
  factionId: RpgEncounterFactionId | null,
  encounter: RpgEncounterSample,
): RpgEncounterSiteRole {
  if (
    (encounter.caveSystemId && (factionId === "kwama-brood" || encounter.cavePressure >= 0.18))
    || factionId === "wild-beasts"
  ) {
    return "mob-lair";
  }
  return encounter.pressure >= 0.72 ? "mob-nest" : "mob-spoor";
}

function resolveEncounterSitePriority(kind: RpgEncounterSiteKind, pressure: number): number {
  const pressurePriority = pressure >= 0.72 ? 8 : pressure >= 0.48 ? 5 : 2;
  switch (kind) {
    case "ambush":
    case "lair":
      return Math.max(pressurePriority, 8);
    case "watch":
      return Math.max(pressurePriority, 6);
    case "camp":
      return Math.max(pressurePriority, 5);
    case "trail":
      return pressurePriority;
  }
}

function siteKindLabel(kind: RpgEncounterSiteKind): string {
  switch (kind) {
    case "ambush":
      return "Ambush Sign";
    case "camp":
      return "Camp Sign";
    case "lair":
      return "Lair Sign";
    case "trail":
      return "Trail Sign";
    case "watch":
      return "Watch Sign";
  }
}

function resolveEncounterSiteClueLabel(kind: RpgEncounterSiteKind, role: RpgEncounterSiteRole): string {
  switch (role) {
    case "mob-lair":
      return "fresh lair sign";
    case "mob-nest":
      return kind === "ambush" ? "staged ambush trace" : "settled nest trace";
    case "mob-spoor":
      return kind === "watch" ? "watch post spoor" : "passing spoor";
  }
}

function resolveEncounterSiteFieldNote(
  kind: RpgEncounterSiteKind,
  factionLabel: string,
  pressure: number,
): string {
  const pressureText = pressure >= 0.72 ? "fresh and crowded" : pressure >= 0.48 ? "recent but scattered" : "old and light";
  switch (kind) {
    case "ambush":
      return `${factionLabel} sign is ${pressureText}; stones and boot cuts angle toward a choke point.`;
    case "camp":
      return `${factionLabel} sign is ${pressureText}; ash-scraped fire rings and footpaths mark a temporary camp.`;
    case "lair":
      return `${factionLabel} sign is ${pressureText}; disturbed soil and shed fragments point back to a den.`;
    case "trail":
      return `${factionLabel} sign is ${pressureText}; broken scrub and repeated tracks cross the path.`;
    case "watch":
      return `${factionLabel} sign is ${pressureText}; lookout scratches face the road and the nearest high ground.`;
  }
}

function siteRandom(cellX: number, cellZ: number, salt: number): number {
  const mixed = Math.imul(cellX, 0x1f12_bb5) ^ Math.imul(cellZ, 0x5f35_6495) ^ salt;
  return hashUint32(mixed) / 0xffff_ffff;
}
