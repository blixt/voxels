import { hashUint32 } from "./math.ts";
import { metersToWorldUnits } from "./scale.ts";
import type { WorldAreaActivitySnapshot } from "./world-systems.ts";

export type ForageSiteRole = "forage-patch" | "reagent-patch" | "supply-cache" | "relic-offering" | "salvage-trace";

export interface ForageSite {
  id: string;
  role: ForageSiteRole;
  name: string;
  x: number;
  z: number;
  cellX: number;
  cellZ: number;
  interactionRadiusWorldUnits: number;
  clueLabel: string;
  fieldNote: string;
}

const FORAGE_SITE_CELL_SIZE_WORLD_UNITS = metersToWorldUnits(32);
const FORAGE_SITE_MARGIN_WORLD_UNITS = metersToWorldUnits(5);
const FORAGE_SITE_INTERACTION_RADIUS_WORLD_UNITS = metersToWorldUnits(4);

export function sampleForageSiteWorldUnits(
  worldX: number,
  worldZ: number,
  area: Pick<WorldAreaActivitySnapshot, "lootId" | "lootName">,
): ForageSite {
  const cellX = Math.floor(worldX / FORAGE_SITE_CELL_SIZE_WORLD_UNITS);
  const cellZ = Math.floor(worldZ / FORAGE_SITE_CELL_SIZE_WORLD_UNITS);
  const randomX = siteRandom(cellX, cellZ, area.lootId, 0x464f_5247);
  const randomZ = siteRandom(cellX, cellZ, area.lootId, 0x464f_525a);
  const usableSpan = FORAGE_SITE_CELL_SIZE_WORLD_UNITS - FORAGE_SITE_MARGIN_WORLD_UNITS * 2;
  const role = resolveForageSiteRole(area.lootId);
  const roleLabel = forageRoleLabel(role);
  return {
    id: `${area.lootId}:${role}:${cellX}:${cellZ}`,
    role,
    name: `${area.lootName} ${roleLabel}`,
    x: cellX * FORAGE_SITE_CELL_SIZE_WORLD_UNITS + FORAGE_SITE_MARGIN_WORLD_UNITS + randomX * usableSpan,
    z: cellZ * FORAGE_SITE_CELL_SIZE_WORLD_UNITS + FORAGE_SITE_MARGIN_WORLD_UNITS + randomZ * usableSpan,
    cellX,
    cellZ,
    interactionRadiusWorldUnits: FORAGE_SITE_INTERACTION_RADIUS_WORLD_UNITS,
    clueLabel: resolveForageClueLabel(role),
    fieldNote: resolveForageFieldNote(role, area.lootName),
  };
}

function resolveForageSiteRole(lootId: string): ForageSiteRole {
  if (lootId.includes("reagent") || lootId.includes("alchemy")) {
    return "reagent-patch";
  }
  if (lootId.includes("pack") || lootId.includes("cache")) {
    return "supply-cache";
  }
  if (lootId.includes("offering") || lootId.includes("shrine")) {
    return "relic-offering";
  }
  if (lootId.includes("trace") || lootId.includes("scrap")) {
    return "salvage-trace";
  }
  return "forage-patch";
}

function forageRoleLabel(role: ForageSiteRole): string {
  switch (role) {
    case "forage-patch":
      return "Patch";
    case "reagent-patch":
      return "Patch";
    case "relic-offering":
      return "Offering";
    case "salvage-trace":
      return "Trace";
    case "supply-cache":
      return "Cache";
  }
}

function resolveForageClueLabel(role: ForageSiteRole): string {
  switch (role) {
    case "forage-patch":
      return "edible field sign";
    case "reagent-patch":
      return "reagent field sign";
    case "relic-offering":
      return "ritual field sign";
    case "salvage-trace":
      return "salvage field sign";
    case "supply-cache":
      return "cache field sign";
  }
}

function resolveForageFieldNote(role: ForageSiteRole, lootName: string): string {
  switch (role) {
    case "forage-patch":
      return `${lootName} grows where travel has thinned the scrub; take only the ripe pieces.`;
    case "reagent-patch":
      return `${lootName} clusters around damp shade and disturbed mineral seams.`;
    case "relic-offering":
      return `${lootName} has been left deliberately, tucked away from weather and casual hands.`;
    case "salvage-trace":
      return `${lootName} follows a scrape of broken gear, old glass, and wind-polished fragments.`;
    case "supply-cache":
      return `${lootName} is hidden by a repeated traveler habit, not by random chance.`;
  }
}

function siteRandom(cellX: number, cellZ: number, lootId: string, salt: number): number {
  let mixed = Math.imul(cellX, 0x27d4_eb2d) ^ Math.imul(cellZ, 0x1656_67b1) ^ salt;
  for (let index = 0; index < lootId.length; index += 1) {
    mixed = Math.imul(mixed ^ lootId.charCodeAt(index), 0x45d9_f3b);
  }
  return hashUint32(mixed) / 0xffff_ffff;
}
