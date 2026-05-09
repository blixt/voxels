import { clamp } from "./math.ts";
import { metersToWorldUnits, worldUnitsToMeters } from "./scale.ts";
import type { AmbientProfileId } from "./ambient-environment.ts";
import type { BiomeId, RegionalVariantId } from "./procedural-generator.ts";

export type AtlasRegionId =
  | "inner-sea"
  | "red-mountain"
  | "ashen-badlands"
  | "bitter-coast"
  | "grazelands"
  | "salt-marsh-basin"
  | "glass-shard-coast"
  | "west-gash";

export type AtlasSurfaceClass = "land" | "shoreline" | "coastal-shelf" | "deep-ocean";
export type AtlasWaterBiomeId = "ocean" | "deep-ocean";
export type AtlasBiomeId = BiomeId | AtlasWaterBiomeId;

export interface AtlasPointMeters {
  x: number;
  z: number;
}

export interface AtlasRadiusMeters {
  x: number;
  z: number;
}

export interface IslandEnvelope {
  origin: AtlasPointMeters;
  radius: AtlasRadiusMeters;
}

export interface AtlasRegionDefinition {
  id: AtlasRegionId;
  center: AtlasPointMeters;
  radius: AtlasRadiusMeters;
  biomeId: BiomeId;
  regionalVariantId: RegionalVariantId | null;
  ambientProfileId: AmbientProfileId;
}

export interface AtlasRegionEdgeDefinition {
  id: `${AtlasRegionId}->${AtlasRegionId}`;
  from: AtlasRegionId;
  to: AtlasRegionId;
  transitionWidthM: number;
  routeBehavior: string;
  materialBridgePalette: readonly string[];
  terrainBlendRule: string;
  validationAnchor: AtlasPointMeters;
}

export interface WorldAtlas {
  version: string;
  island: IslandEnvelope;
  regions: readonly AtlasRegionDefinition[];
  regionEdges: readonly AtlasRegionEdgeDefinition[];
}

export interface IslandMaskSample {
  islandInterior: number;
  shorelineBand: number;
  coastalShelf: number;
  deepOcean: number;
  interiorDistance: number;
  normalizedIslandDistance: number;
  edgeNormalX: number;
  edgeNormalZ: number;
  surfaceClass: AtlasSurfaceClass;
}

export interface AtlasRegionSample {
  primaryRegionId: AtlasRegionId | null;
  secondaryRegionId: AtlasRegionId | null;
  regionStrength: number;
  regionBlend: number;
  regionEdgeId: AtlasRegionEdgeDefinition["id"] | null;
  regionLocalX: number;
  regionLocalZ: number;
  regionDistance: number;
  primaryBiomeId: AtlasBiomeId;
  regionalVariantId: RegionalVariantId | null;
  ambientProfileId: AmbientProfileId | null;
}

export interface WorldAtlasSample extends IslandMaskSample, AtlasRegionSample {
  xM: number;
  zM: number;
}

interface RegionScore {
  region: AtlasRegionDefinition;
  distance: number;
  score: number;
}

const SHORELINE_CENTER_DISTANCE = 0.985;
const LAND_CUTOFF_INTERIOR = 0.08;

export const WORLD_ATLAS: WorldAtlas = {
  version: "20260509-wave1-atlas-foundation",
  island: {
    origin: { x: -180, z: -520 },
    radius: { x: 6_400, z: 5_850 },
  },
  regions: [
    {
      id: "red-mountain",
      center: { x: -520, z: -1_080 },
      radius: { x: 1_080, z: 1_020 },
      biomeId: "ember",
      regionalVariantId: "ember_caldera",
      ambientProfileId: "ashfall",
    },
    {
      id: "ashen-badlands",
      center: { x: -840, z: -2_360 },
      radius: { x: 2_200, z: 1_760 },
      biomeId: "badlands",
      regionalVariantId: "ash_wastes",
      ambientProfileId: "ashfall",
    },
    {
      id: "bitter-coast",
      center: { x: -4_360, z: 920 },
      radius: { x: 2_320, z: 2_040 },
      biomeId: "marsh",
      regionalVariantId: "marsh_blackwater",
      ambientProfileId: "silt-mist",
    },
    {
      id: "grazelands",
      center: { x: 3_420, z: -3_080 },
      radius: { x: 2_180, z: 1_880 },
      biomeId: "savanna",
      regionalVariantId: "savanna_flowersea",
      ambientProfileId: "dry-haze",
    },
    {
      id: "salt-marsh-basin",
      center: { x: -180, z: 4_040 },
      radius: { x: 2_640, z: 1_820 },
      biomeId: "saltflat",
      regionalVariantId: "saltflat_mirror",
      ambientProfileId: "silt-mist",
    },
    {
      id: "glass-shard-coast",
      center: { x: 4_740, z: 2_180 },
      radius: { x: 1_820, z: 2_040 },
      biomeId: "shardlands",
      regionalVariantId: "dunes_glass",
      ambientProfileId: "cold-glass",
    },
    {
      id: "west-gash",
      center: { x: -2_720, z: -3_260 },
      radius: { x: 1_820, z: 1_760 },
      biomeId: "highland",
      regionalVariantId: "highland_redleaf",
      ambientProfileId: "green-canopy",
    },
    {
      id: "inner-sea",
      center: { x: 1_320, z: 120 },
      radius: { x: 2_260, z: 2_180 },
      biomeId: "moor",
      regionalVariantId: "moor_shadowglass",
      ambientProfileId: "silt-mist",
    },
  ],
  regionEdges: [
    {
      id: "inner-sea->red-mountain",
      from: "inner-sea",
      to: "red-mountain",
      transitionWidthM: 860,
      routeBehavior: "pilgrimage approach with ashfall visible from wet lowlands",
      materialBridgePalette: ["moor peat", "wet basalt", "ash dust"],
      terrainBlendRule: "low shelf rises into volcanic foothills",
      validationAnchor: { x: 400, z: -520 },
    },
    {
      id: "red-mountain->ashen-badlands",
      from: "red-mountain",
      to: "ashen-badlands",
      transitionWidthM: 780,
      routeBehavior: "caldera apron, ash roads, cave mouths",
      materialBridgePalette: ["basalt", "red ash", "black pumice"],
      terrainBlendRule: "caldera cone breaks into ash apron ravines",
      validationAnchor: { x: -680, z: -1_720 },
    },
    {
      id: "ashen-badlands->west-gash",
      from: "ashen-badlands",
      to: "west-gash",
      transitionWidthM: 900,
      routeBehavior: "highland pass and ravine route",
      materialBridgePalette: ["ash gravel", "redleaf loam", "weathered stone"],
      terrainBlendRule: "ash apron cuts into highland ravine walls",
      validationAnchor: { x: -1_780, z: -2_810 },
    },
    {
      id: "ashen-badlands->grazelands",
      from: "ashen-badlands",
      to: "grazelands",
      transitionWidthM: 980,
      routeBehavior: "dry eastward trail with camp cadence",
      materialBridgePalette: ["ash loam", "yellow grass", "trail dust"],
      terrainBlendRule: "ash flats thin into rolling dry grass",
      validationAnchor: { x: 1_290, z: -2_720 },
    },
    {
      id: "bitter-coast->inner-sea",
      from: "bitter-coast",
      to: "inner-sea",
      transitionWidthM: 1_000,
      routeBehavior: "wetland to moor crossing with bridges and low islands",
      materialBridgePalette: ["blackwater mud", "reed peat", "old stone"],
      terrainBlendRule: "drowned hummocks merge into central moor shelf",
      validationAnchor: { x: -1_520, z: 520 },
    },
    {
      id: "inner-sea->salt-marsh-basin",
      from: "inner-sea",
      to: "salt-marsh-basin",
      transitionWidthM: 1_060,
      routeBehavior: "shelf road to mirror flats",
      materialBridgePalette: ["moor silt", "salt crust", "causeway stone"],
      terrainBlendRule: "wet shelf drains into bright flat basin",
      validationAnchor: { x: 570, z: 2_080 },
    },
    {
      id: "salt-marsh-basin->glass-shard-coast",
      from: "salt-marsh-basin",
      to: "glass-shard-coast",
      transitionWidthM: 940,
      routeBehavior: "salt and glass mineral transition",
      materialBridgePalette: ["white salt", "pale sand", "glass fragments"],
      terrainBlendRule: "flat salt ribs sharpen into shard ridges",
      validationAnchor: { x: 2_280, z: 3_110 },
    },
    {
      id: "grazelands->glass-shard-coast",
      from: "grazelands",
      to: "glass-shard-coast",
      transitionWidthM: 920,
      routeBehavior: "open route into hazardous shard coast",
      materialBridgePalette: ["dry grass", "coastal gravel", "glass cairns"],
      terrainBlendRule: "rolling grass loses soil over cold glass shelves",
      validationAnchor: { x: 4_080, z: -450 },
    },
  ],
};

export function atlasMetersToWorldUnits(point: AtlasPointMeters): AtlasPointMeters {
  return {
    x: metersToWorldUnits(point.x),
    z: metersToWorldUnits(point.z),
  };
}

export function atlasWorldUnitsToMeters(point: AtlasPointMeters): AtlasPointMeters {
  return {
    x: worldUnitsToMeters(point.x),
    z: worldUnitsToMeters(point.z),
  };
}

export function sampleWorldAtlasMeters(xM: number, zM: number, atlas = WORLD_ATLAS): WorldAtlasSample {
  const islandMask = sampleIslandMaskMeters(xM, zM, atlas);
  const regionSample = sampleAtlasRegionMeters(xM, zM, islandMask, atlas);

  return {
    xM,
    zM,
    ...islandMask,
    ...regionSample,
  };
}

export function sampleWorldAtlasWorldUnits(worldX: number, worldZ: number, atlas = WORLD_ATLAS): WorldAtlasSample {
  return sampleWorldAtlasMeters(worldUnitsToMeters(worldX), worldUnitsToMeters(worldZ), atlas);
}

export function sampleIslandMaskMeters(xM: number, zM: number, atlas = WORLD_ATLAS): IslandMaskSample {
  const localX = (xM - atlas.island.origin.x) / atlas.island.radius.x;
  const localZ = (zM - atlas.island.origin.z) / atlas.island.radius.z;
  const angle = Math.atan2(localZ, localX);
  const shorelineScale = islandShorelineScale(angle);
  const normalizedIslandDistance = Math.hypot(localX, localZ) / shorelineScale;
  const islandInterior = 1 - smoothstep(0.89, 1.045, normalizedIslandDistance);
  const shorelineBand =
    (1 - smoothstep(0, 0.105, Math.abs(normalizedIslandDistance - SHORELINE_CENTER_DISTANCE))) *
    (1 - smoothstep(1.22, 1.36, normalizedIslandDistance));
  const coastalShelf =
    smoothstep(0.90, 1.08, normalizedIslandDistance) *
    (1 - smoothstep(1.30, 1.58, normalizedIslandDistance));
  const deepOcean = smoothstep(1.24, 1.56, normalizedIslandDistance);
  const interiorDistance = clamp(1 - normalizedIslandDistance, 0, 1);
  const normalLength = Math.hypot(localX / atlas.island.radius.x, localZ / atlas.island.radius.z);
  const edgeNormalX = normalLength === 0 ? 0 : (localX / atlas.island.radius.x) / normalLength;
  const edgeNormalZ = normalLength === 0 ? 0 : (localZ / atlas.island.radius.z) / normalLength;

  return {
    islandInterior,
    shorelineBand,
    coastalShelf,
    deepOcean,
    interiorDistance,
    normalizedIslandDistance,
    edgeNormalX,
    edgeNormalZ,
    surfaceClass: classifySurface(islandInterior, shorelineBand, deepOcean),
  };
}

export function findAtlasRegion(regionId: AtlasRegionId, atlas = WORLD_ATLAS): AtlasRegionDefinition {
  const region = atlas.regions.find((candidate) => candidate.id === regionId);
  if (!region) {
    throw new Error(`Unknown atlas region: ${regionId}`);
  }
  return region;
}

export function findAtlasRegionEdge(
  edgeId: AtlasRegionEdgeDefinition["id"],
  atlas = WORLD_ATLAS,
): AtlasRegionEdgeDefinition {
  const edge = atlas.regionEdges.find((candidate) => candidate.id === edgeId);
  if (!edge) {
    throw new Error(`Unknown atlas region edge: ${edgeId}`);
  }
  return edge;
}

function sampleAtlasRegionMeters(
  xM: number,
  zM: number,
  islandMask: IslandMaskSample,
  atlas: WorldAtlas,
): AtlasRegionSample {
  if (islandMask.islandInterior < LAND_CUTOFF_INTERIOR) {
    return {
      primaryRegionId: null,
      secondaryRegionId: null,
      regionStrength: 0,
      regionBlend: 0,
      regionEdgeId: null,
      regionLocalX: 0,
      regionLocalZ: 0,
      regionDistance: Infinity,
      primaryBiomeId: islandMask.deepOcean > 0.52 ? "deep-ocean" : "ocean",
      regionalVariantId: null,
      ambientProfileId: null,
    };
  }

  const scores = atlas.regions
    .map((region) => scoreRegion(region, xM, zM, atlas))
    .sort((a, b) => b.score - a.score);
  const primary = scores[0]!;
  const secondary = scores[1]!;
  const localX = (xM - primary.region.center.x) / primary.region.radius.x;
  const localZ = (zM - primary.region.center.z) / primary.region.radius.z;
  const edge = resolveRegionEdge(primary.region.id, secondary.region.id, xM, zM, atlas);
  const blend = clamp(secondary.score / Math.max(0.001, primary.score), 0, 1);

  return {
    primaryRegionId: primary.region.id,
    secondaryRegionId: secondary.region.id,
    regionStrength: clamp((1 - blend) * 1.42 + islandMask.islandInterior * 0.16, 0, 1),
    regionBlend: blend,
    regionEdgeId: edge?.id ?? null,
    regionLocalX: localX,
    regionLocalZ: localZ,
    regionDistance: primary.distance,
    primaryBiomeId: primary.region.biomeId,
    regionalVariantId: primary.region.regionalVariantId,
    ambientProfileId: primary.region.ambientProfileId,
  };
}

function scoreRegion(region: AtlasRegionDefinition, xM: number, zM: number, atlas: WorldAtlas): RegionScore {
  const dx = (xM - region.center.x) / region.radius.x;
  const dz = (zM - region.center.z) / region.radius.z;
  const distance = Math.hypot(dx, dz);
  const broadScore = 1 / (0.18 + distance * distance);
  const graphScore = strongestEdgeEndpointBoost(region.id, xM, zM, atlas);
  const centralBias = region.id === "inner-sea" ? 0.07 : 0;
  const volcanicBias = region.id === "red-mountain" ? 0.04 : 0;

  return {
    region,
    distance,
    score: broadScore + graphScore + centralBias + volcanicBias,
  };
}

function strongestEdgeEndpointBoost(regionId: AtlasRegionId, xM: number, zM: number, atlas: WorldAtlas): number {
  let strongest = 0;
  for (const edge of atlas.regionEdges) {
    if (edge.from !== regionId && edge.to !== regionId) {
      continue;
    }
    const endpointA = findAtlasRegion(edge.from, atlas).center;
    const endpointB = findAtlasRegion(edge.to, atlas).center;
    const segment = projectPointToSegment(xM, zM, endpointA, endpointB);
    const normalizedDistance = segment.distance / edge.transitionWidthM;
    const alongBalance = 1 - Math.abs(segment.t - 0.5) * 1.1;
    strongest = Math.max(strongest, smoothstep(1.0, 0.0, normalizedDistance) * clamp(alongBalance, 0, 1) * 0.28);
  }
  return strongest;
}

function resolveRegionEdge(
  primaryRegionId: AtlasRegionId,
  secondaryRegionId: AtlasRegionId,
  xM: number,
  zM: number,
  atlas: WorldAtlas,
): AtlasRegionEdgeDefinition | null {
  const directEdge = atlas.regionEdges.find((edge) =>
    (edge.from === primaryRegionId && edge.to === secondaryRegionId) ||
    (edge.from === secondaryRegionId && edge.to === primaryRegionId)
  );
  if (directEdge) {
    return directEdge;
  }

  let nearestEdge: { edge: AtlasRegionEdgeDefinition; normalizedDistance: number } | null = null;
  for (const edge of atlas.regionEdges) {
    if (edge.from !== primaryRegionId && edge.to !== primaryRegionId) {
      continue;
    }
    const endpointA = findAtlasRegion(edge.from, atlas).center;
    const endpointB = findAtlasRegion(edge.to, atlas).center;
    const projected = projectPointToSegment(xM, zM, endpointA, endpointB);
    const normalizedDistance = projected.distance / edge.transitionWidthM;
    if (projected.t < 0.12 || projected.t > 0.88 || normalizedDistance > 0.72) {
      continue;
    }
    if (!nearestEdge || normalizedDistance < nearestEdge.normalizedDistance) {
      nearestEdge = { edge, normalizedDistance };
    }
  }

  return nearestEdge?.edge ?? null;
}

function projectPointToSegment(
  x: number,
  z: number,
  a: AtlasPointMeters,
  b: AtlasPointMeters,
): { t: number; distance: number } {
  const abX = b.x - a.x;
  const abZ = b.z - a.z;
  const lengthSquared = abX * abX + abZ * abZ;
  if (lengthSquared === 0) {
    return { t: 0, distance: Math.hypot(x - a.x, z - a.z) };
  }
  const t = clamp(((x - a.x) * abX + (z - a.z) * abZ) / lengthSquared, 0, 1);
  const projectedX = a.x + abX * t;
  const projectedZ = a.z + abZ * t;
  return { t, distance: Math.hypot(x - projectedX, z - projectedZ) };
}

function islandShorelineScale(angle: number): number {
  return 1
    + Math.sin(angle * 3.0 + 0.7) * 0.08
    + Math.sin(angle * 5.0 - 1.6) * 0.055
    + Math.sin(angle * 9.0 + 0.2) * 0.028;
}

function classifySurface(
  islandInterior: number,
  shorelineBand: number,
  deepOcean: number,
): AtlasSurfaceClass {
  if (islandInterior >= LAND_CUTOFF_INTERIOR) {
    return shorelineBand > 0.42 ? "shoreline" : "land";
  }
  return deepOcean > 0.52 ? "deep-ocean" : "coastal-shelf";
}

function smoothstep(edge0: number, edge1: number, value: number): number {
  if (edge0 === edge1) {
    return value < edge0 ? 0 : 1;
  }
  const t = clamp((value - edge0) / (edge1 - edge0), 0, 1);
  return t * t * (3 - 2 * t);
}
