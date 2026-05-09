import { clamp, hashUint32 } from "./math.ts";
import { worldUnitsToMeters } from "./scale.ts";
import {
  WORLD_ATLAS,
  sampleWorldAtlasMeters,
  type AtlasCaveSystemId,
  type AtlasRegionId,
  type AtlasRouteId,
  type WorldAtlas,
} from "./world-atlas.ts";

export const RPG_ENCOUNTERS_VERSION = 1;

export type RpgEncounterMoodId =
  | "ash-pilgrimage"
  | "ash-wind-watch"
  | "blackwater-fog"
  | "open-grass-patrol"
  | "salt-mirage"
  | "glass-cairn-tension"
  | "redleaf-ravine-watch"
  | "silt-road-calm"
  | "road-truce"
  | "cave-threshold";

export type RpgEncounterFactionId =
  | "temple-pilgrims"
  | "ashlander-scouts"
  | "kwama-brood"
  | "marsh-foragers"
  | "salt-causeway-wardens"
  | "glass-cairn-keepers"
  | "redleaf-guides"
  | "inner-sea-traders"
  | "opportunist-bandits"
  | "wild-beasts";

export interface RpgFactionHint {
  factionId: RpgEncounterFactionId;
  weight: number;
}

export interface RpgEncounterZoneDefinition {
  regionId: AtlasRegionId;
  moodId: RpgEncounterMoodId;
  pressureBase: number;
  wildernessRisk: number;
  factionHints: readonly RpgFactionHint[];
  flavorTags: readonly string[];
}

export interface RpgRouteEncounterModifier {
  routeId: AtlasRouteId;
  safety: number;
  moodId: RpgEncounterMoodId;
  factionHints: readonly RpgFactionHint[];
  flavorTags: readonly string[];
}

export interface RpgCaveEncounterModifier {
  caveSystemId: AtlasCaveSystemId;
  pressureBonus: number;
  factionHints: readonly RpgFactionHint[];
  flavorTags: readonly string[];
}

export interface RpgEncounterSample {
  xM: number;
  zM: number;
  regionId: AtlasRegionId | null;
  routeId: AtlasRouteId | null;
  caveSystemId: AtlasCaveSystemId | null;
  caveAnchorId: string | null;
  moodId: RpgEncounterMoodId;
  pressure: number;
  routeSafety: number;
  wildernessRisk: number;
  cavePressure: number;
  factionHints: readonly RpgFactionHint[];
  flavorTags: readonly string[];
}

export const RPG_ENCOUNTER_ZONES: readonly RpgEncounterZoneDefinition[] = [
  {
    regionId: "red-mountain",
    moodId: "ash-pilgrimage",
    pressureBase: 0.68,
    wildernessRisk: 0.82,
    factionHints: [
      { factionId: "temple-pilgrims", weight: 0.42 },
      { factionId: "ashlander-scouts", weight: 0.24 },
      { factionId: "wild-beasts", weight: 0.22 },
    ],
    flavorTags: ["caldera", "ashfall", "sacred-road"],
  },
  {
    regionId: "ashen-badlands",
    moodId: "ash-wind-watch",
    pressureBase: 0.6,
    wildernessRisk: 0.74,
    factionHints: [
      { factionId: "ashlander-scouts", weight: 0.34 },
      { factionId: "kwama-brood", weight: 0.26 },
      { factionId: "opportunist-bandits", weight: 0.2 },
    ],
    flavorTags: ["ash-apron", "ravines", "windbreak-camps"],
  },
  {
    regionId: "bitter-coast",
    moodId: "blackwater-fog",
    pressureBase: 0.46,
    wildernessRisk: 0.64,
    factionHints: [
      { factionId: "marsh-foragers", weight: 0.36 },
      { factionId: "opportunist-bandits", weight: 0.2 },
      { factionId: "wild-beasts", weight: 0.24 },
    ],
    flavorTags: ["blackwater", "root-grottos", "reed-bridges"],
  },
  {
    regionId: "grazelands",
    moodId: "open-grass-patrol",
    pressureBase: 0.34,
    wildernessRisk: 0.44,
    factionHints: [
      { factionId: "ashlander-scouts", weight: 0.3 },
      { factionId: "inner-sea-traders", weight: 0.22 },
      { factionId: "wild-beasts", weight: 0.18 },
    ],
    flavorTags: ["open-plains", "stone-camps", "grass-markers"],
  },
  {
    regionId: "salt-marsh-basin",
    moodId: "salt-mirage",
    pressureBase: 0.48,
    wildernessRisk: 0.58,
    factionHints: [
      { factionId: "salt-causeway-wardens", weight: 0.34 },
      { factionId: "inner-sea-traders", weight: 0.22 },
      { factionId: "wild-beasts", weight: 0.16 },
    ],
    flavorTags: ["mirror-salt", "dead-causeway", "sunken-pylons"],
  },
  {
    regionId: "glass-shard-coast",
    moodId: "glass-cairn-tension",
    pressureBase: 0.62,
    wildernessRisk: 0.76,
    factionHints: [
      { factionId: "glass-cairn-keepers", weight: 0.4 },
      { factionId: "opportunist-bandits", weight: 0.22 },
      { factionId: "wild-beasts", weight: 0.18 },
    ],
    flavorTags: ["glass-ridges", "coastal-cairns", "cold-shelters"],
  },
  {
    regionId: "west-gash",
    moodId: "redleaf-ravine-watch",
    pressureBase: 0.5,
    wildernessRisk: 0.62,
    factionHints: [
      { factionId: "redleaf-guides", weight: 0.36 },
      { factionId: "ashlander-scouts", weight: 0.2 },
      { factionId: "wild-beasts", weight: 0.2 },
    ],
    flavorTags: ["redleaf", "ravine-pass", "stone-overlooks"],
  },
  {
    regionId: "inner-sea",
    moodId: "silt-road-calm",
    pressureBase: 0.3,
    wildernessRisk: 0.38,
    factionHints: [
      { factionId: "inner-sea-traders", weight: 0.38 },
      { factionId: "temple-pilgrims", weight: 0.28 },
      { factionId: "marsh-foragers", weight: 0.14 },
    ],
    flavorTags: ["moor-shelf", "old-roads", "low-ruins"],
  },
] as const;

export const RPG_ROUTE_ENCOUNTER_MODIFIERS: readonly RpgRouteEncounterModifier[] = [
  routeModifier("pilgrim-spine-red", 0.76, "temple-pilgrims", ["wayshrines", "ash-markers"]),
  routeModifier("ash-gash-pass", 0.52, "redleaf-guides", ["switchbacks", "pass-watch"]),
  routeModifier("badlands-east-trail", 0.36, "ashlander-scouts", ["caravan-camps", "windbreaks"]),
  routeModifier("bitter-inner-crossing", 0.62, "marsh-foragers", ["reed-bridges", "hummocks"]),
  routeModifier("salt-causeway", 0.66, "salt-causeway-wardens", ["salt-ribs", "pylons"]),
  routeModifier("inner-sea-shelf-road", 0.72, "inner-sea-traders", ["road-slabs", "silt-markers"]),
  routeModifier("grazelands-glass-road", 0.4, "glass-cairn-keepers", ["warning-cairns", "shelters"]),
  routeModifier("glass-coastal-cairns", 0.42, "glass-cairn-keepers", ["coastal-warnings", "shard-vistas"]),
] as const;

export const RPG_CAVE_ENCOUNTER_MODIFIERS: readonly RpgCaveEncounterModifier[] = [
  caveModifier("red-caldera-tubes", 0.32, "temple-pilgrims", ["lava-tube", "vent-heat", "basalt-shrine"]),
  caveModifier("ash-kwama-ravines", 0.38, "kwama-brood", ["kwama-mine", "egg-scent", "buried-ribs"]),
  caveModifier("bitter-root-grottos", 0.26, "marsh-foragers", ["root-grotto", "blackwater-drip", "dry-hummock"]),
  caveModifier("west-gash-ravine-caves", 0.3, "redleaf-guides", ["granitic-cave", "ravine-echo", "redleaf-roots"]),
  caveModifier("glass-crystal-caverns", 0.36, "glass-cairn-keepers", ["crystal-cavern", "cold-glass", "cairn-vigil"]),
  caveModifier("salt-crust-sinkholes", 0.28, "salt-causeway-wardens", ["saline-sinkhole", "mirror-crust", "sunken-air"]),
] as const;

const DEFAULT_MOOD_ID: RpgEncounterMoodId = "silt-road-calm";

export function sampleRpgEncounterMeters(xM: number, zM: number, atlas: WorldAtlas = WORLD_ATLAS): RpgEncounterSample {
  const atlasSample = sampleWorldAtlasMeters(xM, zM, atlas);
  const zone = atlasSample.primaryRegionId ? findEncounterZone(atlasSample.primaryRegionId) : null;
  const route = atlasSample.routeId ? findRouteModifier(atlasSample.routeId) : null;
  const cave = atlasSample.caveSystemId ? findCaveModifier(atlasSample.caveSystemId) : null;
  const coordinateJitter = deterministicSignedJitter(xM, zM) * 0.035;

  const routeSafety = route ? route.safety * (atlasSample.routeCore * 0.9 + atlasSample.routeShoulder * 0.45) : 0;
  const wildernessRisk = (zone?.wildernessRisk ?? 0) * (1 - atlasSample.routeInfluence * 0.58);
  const cavePressure = cave ? cave.pressureBonus * (atlasSample.caveCore * 0.75 + atlasSample.caveInfluence * 0.45) : 0;
  const pressure = clamp(
    (zone?.pressureBase ?? 0) +
      wildernessRisk * 0.28 -
      routeSafety * 0.42 +
      cavePressure +
      atlasSample.shorelineBand * 0.04 +
      coordinateJitter,
    0,
    1,
  );
  const moodId = cave
    ? "cave-threshold"
    : route && atlasSample.routeCore > 0.35
      ? route.moodId
      : zone?.moodId ?? DEFAULT_MOOD_ID;

  return {
    xM,
    zM,
    regionId: atlasSample.primaryRegionId,
    routeId: atlasSample.routeId,
    caveSystemId: atlasSample.caveSystemId,
    caveAnchorId: atlasSample.caveAnchorId,
    moodId,
    pressure,
    routeSafety,
    wildernessRisk,
    cavePressure,
    factionHints: normalizeFactionHints([
      ...(zone?.factionHints ?? []),
      ...scaleFactionHints(route?.factionHints ?? [], atlasSample.routeInfluence * 0.75),
      ...scaleFactionHints(cave?.factionHints ?? [], atlasSample.caveInfluence),
    ]),
    flavorTags: uniqueStrings([
      ...(zone?.flavorTags ?? []),
      ...(route && atlasSample.routeInfluence > 0 ? route.flavorTags : []),
      ...(cave && atlasSample.caveInfluence > 0 ? cave.flavorTags : []),
    ]),
  };
}

export function sampleRpgEncounterWorldUnits(
  worldX: number,
  worldZ: number,
  atlas: WorldAtlas = WORLD_ATLAS,
): RpgEncounterSample {
  return sampleRpgEncounterMeters(worldUnitsToMeters(worldX), worldUnitsToMeters(worldZ), atlas);
}

export function getRpgEncounterZoneDefinitions(): readonly RpgEncounterZoneDefinition[] {
  return RPG_ENCOUNTER_ZONES;
}

function routeModifier(
  routeId: AtlasRouteId,
  safety: number,
  factionId: RpgEncounterFactionId,
  flavorTags: readonly string[],
): RpgRouteEncounterModifier {
  return {
    routeId,
    safety,
    moodId: "road-truce",
    factionHints: [{ factionId, weight: 0.34 }],
    flavorTags,
  };
}

function caveModifier(
  caveSystemId: AtlasCaveSystemId,
  pressureBonus: number,
  factionId: RpgEncounterFactionId,
  flavorTags: readonly string[],
): RpgCaveEncounterModifier {
  return {
    caveSystemId,
    pressureBonus,
    factionHints: [{ factionId, weight: 0.46 }],
    flavorTags,
  };
}

function findEncounterZone(regionId: AtlasRegionId): RpgEncounterZoneDefinition {
  const zone = RPG_ENCOUNTER_ZONES.find((candidate) => candidate.regionId === regionId);
  if (!zone) {
    throw new Error(`Missing RPG encounter zone for atlas region: ${regionId}`);
  }
  return zone;
}

function findRouteModifier(routeId: AtlasRouteId): RpgRouteEncounterModifier | null {
  return RPG_ROUTE_ENCOUNTER_MODIFIERS.find((candidate) => candidate.routeId === routeId) ?? null;
}

function findCaveModifier(caveSystemId: AtlasCaveSystemId): RpgCaveEncounterModifier | null {
  return RPG_CAVE_ENCOUNTER_MODIFIERS.find((candidate) => candidate.caveSystemId === caveSystemId) ?? null;
}

function scaleFactionHints(hints: readonly RpgFactionHint[], scale: number): readonly RpgFactionHint[] {
  return hints.map((hint) => ({
    factionId: hint.factionId,
    weight: hint.weight * scale,
  }));
}

function normalizeFactionHints(hints: readonly RpgFactionHint[]): readonly RpgFactionHint[] {
  const weights = new Map<RpgEncounterFactionId, number>();
  for (const hint of hints) {
    weights.set(hint.factionId, (weights.get(hint.factionId) ?? 0) + hint.weight);
  }

  const total = [...weights.values()].reduce((sum, weight) => sum + weight, 0);
  if (total <= 0) {
    return [];
  }

  return [...weights.entries()]
    .map(([factionId, weight]) => ({
      factionId,
      weight: Number((weight / total).toFixed(4)),
    }))
    .sort((left, right) => right.weight - left.weight || left.factionId.localeCompare(right.factionId));
}

function uniqueStrings(values: readonly string[]): readonly string[] {
  return [...new Set(values)];
}

function deterministicSignedJitter(xM: number, zM: number): number {
  const cellX = Math.floor(xM / 96);
  const cellZ = Math.floor(zM / 96);
  const packed = Math.imul(cellX, 73_856_093) ^ Math.imul(cellZ, 19_349_663) ^ 0x9e3779b9;
  return hashUint32(packed) / 0xffffffff * 2 - 1;
}
