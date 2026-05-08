import { clamp } from "./math.ts";
import { metersToWorldUnits } from "./scale.ts";
import type { BiomeId, RegionalVariantId } from "./procedural-generator.ts";
import type { AmbientProfileId } from "./ambient-environment.ts";

export type WorldRegionId =
  | "inner-sea"
  | "red-mountain"
  | "ashen-badlands"
  | "bitter-coast"
  | "grazelands"
  | "salt-marsh-basin"
  | "glass-shard-coast"
  | "west-gash";

export interface WorldRegionSample {
  regionId: WorldRegionId;
  secondaryRegionId: WorldRegionId;
  regionStrength: number;
  regionBlend: number;
  islandInterior: number;
  coastalShelf: number;
  volcanicHeart: number;
  ashRing: number;
  westWetlands: number;
  northeastGrazelands: number;
  southernSaltBasin: number;
  easternShardCoast: number;
  biomeId: BiomeId;
  regionalVariantId: RegionalVariantId | null;
  ambientProfileId: AmbientProfileId;
}

interface WorldRegionDefinition {
  id: WorldRegionId;
  centerX: number;
  centerZ: number;
  radiusX: number;
  radiusZ: number;
  biomeId: BiomeId;
  regionalVariantId: RegionalVariantId | null;
  ambientProfileId: AmbientProfileId;
}

export const WORLD_REGION_AUTHORITY_THRESHOLD = 0.48;

const ISLAND_CENTER_X = metersToWorldUnits(-180);
const ISLAND_CENTER_Z = metersToWorldUnits(-520);
const ISLAND_RADIUS_X = metersToWorldUnits(6_400);
const ISLAND_RADIUS_Z = metersToWorldUnits(5_850);

const WORLD_REGION_DEFINITIONS: readonly WorldRegionDefinition[] = [
  {
    id: "red-mountain",
    centerX: metersToWorldUnits(-520),
    centerZ: metersToWorldUnits(-1_080),
    radiusX: metersToWorldUnits(1_080),
    radiusZ: metersToWorldUnits(1_020),
    biomeId: "ember",
    regionalVariantId: "ember_caldera",
    ambientProfileId: "ashfall",
  },
  {
    id: "ashen-badlands",
    centerX: metersToWorldUnits(-840),
    centerZ: metersToWorldUnits(-2_360),
    radiusX: metersToWorldUnits(2_200),
    radiusZ: metersToWorldUnits(1_760),
    biomeId: "badlands",
    regionalVariantId: "ash_wastes",
    ambientProfileId: "ashfall",
  },
  {
    id: "bitter-coast",
    centerX: metersToWorldUnits(-4_360),
    centerZ: metersToWorldUnits(920),
    radiusX: metersToWorldUnits(2_320),
    radiusZ: metersToWorldUnits(2_040),
    biomeId: "marsh",
    regionalVariantId: "marsh_blackwater",
    ambientProfileId: "silt-mist",
  },
  {
    id: "grazelands",
    centerX: metersToWorldUnits(3_420),
    centerZ: metersToWorldUnits(-3_080),
    radiusX: metersToWorldUnits(2_180),
    radiusZ: metersToWorldUnits(1_880),
    biomeId: "savanna",
    regionalVariantId: "savanna_flowersea",
    ambientProfileId: "dry-haze",
  },
  {
    id: "salt-marsh-basin",
    centerX: metersToWorldUnits(-180),
    centerZ: metersToWorldUnits(4_040),
    radiusX: metersToWorldUnits(2_240),
    radiusZ: metersToWorldUnits(1_820),
    biomeId: "saltflat",
    regionalVariantId: "saltflat_mirror",
    ambientProfileId: "silt-mist",
  },
  {
    id: "glass-shard-coast",
    centerX: metersToWorldUnits(4_740),
    centerZ: metersToWorldUnits(2_180),
    radiusX: metersToWorldUnits(1_820),
    radiusZ: metersToWorldUnits(2_040),
    biomeId: "shardlands",
    regionalVariantId: "dunes_glass",
    ambientProfileId: "cold-glass",
  },
  {
    id: "west-gash",
    centerX: metersToWorldUnits(-2_720),
    centerZ: metersToWorldUnits(-3_260),
    radiusX: metersToWorldUnits(1_820),
    radiusZ: metersToWorldUnits(1_760),
    biomeId: "highland",
    regionalVariantId: "highland_redleaf",
    ambientProfileId: "green-canopy",
  },
  {
    id: "inner-sea",
    centerX: metersToWorldUnits(1_320),
    centerZ: metersToWorldUnits(120),
    radiusX: metersToWorldUnits(2_260),
    radiusZ: metersToWorldUnits(2_180),
    biomeId: "moor",
    regionalVariantId: "moor_shadowglass",
    ambientProfileId: "silt-mist",
  },
];

export function sampleWorldRegion(worldX: number, worldZ: number): WorldRegionSample {
  const localX = (worldX - ISLAND_CENTER_X) / ISLAND_RADIUS_X;
  const localZ = (worldZ - ISLAND_CENTER_Z) / ISLAND_RADIUS_Z;
  const angle = Math.atan2(localZ, localX);
  const shorelineIrregularity = 1
    + Math.sin(angle * 3.0 + 0.7) * 0.08
    + Math.sin(angle * 5.0 - 1.6) * 0.055
    + Math.sin(angle * 9.0 + 0.2) * 0.028;
  const islandDistance = Math.hypot(localX, localZ) / shorelineIrregularity;
  const islandInterior = 1 - smoothstep(0.86, 1.08, islandDistance);
  const coastalShelf = smoothstep(0.62, 1.03, islandDistance) * islandInterior;

  let primary = WORLD_REGION_DEFINITIONS[0]!;
  let secondary = WORLD_REGION_DEFINITIONS[1]!;
  let primaryScore = -Infinity;
  let secondaryScore = -Infinity;
  for (const region of WORLD_REGION_DEFINITIONS) {
    const dx = (worldX - region.centerX) / region.radiusX;
    const dz = (worldZ - region.centerZ) / region.radiusZ;
    const distance = Math.hypot(dx, dz);
    const broadScore = 1 / (0.22 + distance * distance);
    const directionalBias = region.id === "inner-sea" ? 0.10 : 0;
    const shorelinePenalty = region.id === "red-mountain" || region.id === "ashen-badlands"
      ? 0
      : coastalShelf * 0.08;
    const score = broadScore + directionalBias - shorelinePenalty;
    if (score > primaryScore) {
      secondary = primary;
      secondaryScore = primaryScore;
      primary = region;
      primaryScore = score;
    } else if (score > secondaryScore) {
      secondary = region;
      secondaryScore = score;
    }
  }

  if (islandInterior < 0.08) {
    primary = WORLD_REGION_DEFINITIONS.find((region) => region.id === "inner-sea")!;
    secondary = primary;
    primaryScore = 1;
    secondaryScore = 0;
  }

  const regionBlend = clamp(secondaryScore / Math.max(0.001, primaryScore), 0, 1);
  const regionStrength = clamp((1 - regionBlend) * 1.55 + islandInterior * 0.20, 0, 1);
  const volcanicHeart = regionInfluence("red-mountain", worldX, worldZ) * islandInterior;
  const ashRing = Math.max(
    regionInfluence("ashen-badlands", worldX, worldZ),
    smoothstep(0.34, 1.0, regionInfluence("red-mountain", worldX, worldZ)) * 0.62,
  ) * islandInterior;

  return {
    regionId: primary.id,
    secondaryRegionId: secondary.id,
    regionStrength,
    regionBlend,
    islandInterior,
    coastalShelf,
    volcanicHeart,
    ashRing,
    westWetlands: Math.max(regionInfluence("bitter-coast", worldX, worldZ), regionInfluence("inner-sea", worldX, worldZ) * 0.34) * islandInterior,
    northeastGrazelands: regionInfluence("grazelands", worldX, worldZ) * islandInterior,
    southernSaltBasin: regionInfluence("salt-marsh-basin", worldX, worldZ) * islandInterior,
    easternShardCoast: regionInfluence("glass-shard-coast", worldX, worldZ) * islandInterior,
    biomeId: primary.biomeId,
    regionalVariantId: primary.regionalVariantId,
    ambientProfileId: primary.ambientProfileId,
  };
}

function regionInfluence(regionId: WorldRegionId, worldX: number, worldZ: number): number {
  const region = WORLD_REGION_DEFINITIONS.find((candidate) => candidate.id === regionId)!;
  const dx = (worldX - region.centerX) / region.radiusX;
  const dz = (worldZ - region.centerZ) / region.radiusZ;
  return clamp(1 - smoothstep(0.12, 1.28, Math.hypot(dx, dz)), 0, 1);
}

function smoothstep(edge0: number, edge1: number, value: number): number {
  const t = clamp((value - edge0) / (edge1 - edge0), 0, 1);
  return t * t * (3 - 2 * t);
}
