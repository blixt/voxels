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
export type AtlasRouteId =
  | "pilgrim-spine-red"
  | "ash-gash-pass"
  | "badlands-east-trail"
  | "bitter-inner-crossing"
  | "salt-causeway"
  | "inner-sea-shelf-road"
  | "grazelands-glass-road"
  | "glass-coastal-cairns";
export type AtlasRouteKind = "pilgrim-road" | "coastal-walk" | "causeway" | "mountain-pass" | "hazard-route";
export type AtlasRouteSegmentKind =
  | "sacred-road"
  | "ash-road"
  | "ravine-pass"
  | "caravan-trail"
  | "wetland-bridge"
  | "salt-causeway"
  | "shelf-road"
  | "hazard-cairns";
export type AtlasCaveSystemId =
  | "red-caldera-tubes"
  | "ash-kwama-ravines"
  | "bitter-root-grottos"
  | "west-gash-ravine-caves"
  | "glass-crystal-caverns"
  | "salt-crust-sinkholes";
export type AtlasCaveKind =
  | "lava-tube"
  | "kwama-mine"
  | "root-cave"
  | "granitic-cave"
  | "crystal-cavern"
  | "saline-sinkhole";
export type AtlasCaveAnchorKind = "entrance" | "chamber" | "overlook";

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

export interface AtlasRouteNodeDefinition {
  id: string;
  point: AtlasPointMeters;
  regionId: AtlasRegionId;
}

export interface AtlasRouteDefinition {
  id: AtlasRouteId;
  kind: AtlasRouteKind;
  nodes: readonly AtlasRouteNodeDefinition[];
  widthM: number;
  shoulderM: number;
  materialProfileId: string;
  expectedRegionIds: readonly AtlasRegionId[];
  landmarkCadenceM: number;
  strongVistaCadenceM: number;
  segmentKind: AtlasRouteSegmentKind;
  recommendedSetPieceIds: readonly string[];
  validationAnchor: AtlasPointMeters;
}

export interface AtlasCaveAnchorDefinition {
  id: string;
  kind: AtlasCaveAnchorKind;
  point: AtlasPointMeters;
  regionId: AtlasRegionId;
  radiusM: number;
  dryEntranceRequired: boolean;
  associatedRouteId: AtlasRouteId | null;
  landmarkMarkerIds: readonly string[];
}

export interface AtlasCaveTunnelDefinition {
  from: string;
  to: string;
  widthM: number;
  verticalBiasM: number;
  materialProfileId: string;
}

export interface AtlasCaveSystemDefinition {
  id: AtlasCaveSystemId;
  regionId: AtlasRegionId;
  kind: AtlasCaveKind;
  anchors: readonly AtlasCaveAnchorDefinition[];
  tunnels: readonly AtlasCaveTunnelDefinition[];
  materialProfileId: string;
  expectedRouteIds: readonly AtlasRouteId[];
  validationAnchor: AtlasPointMeters;
}

export interface AtlasRegionGraphNode {
  regionId: AtlasRegionId;
  center: AtlasPointMeters;
  radius: AtlasRadiusMeters;
  approximateEllipseAreaM2: number;
  edgeIds: readonly AtlasRegionEdgeDefinition["id"][];
  routeIds: readonly AtlasRouteId[];
  caveSystemIds: readonly AtlasCaveSystemId[];
}

export interface AtlasRouteAnchor {
  id: string;
  routeId: AtlasRouteId;
  kind: "node" | "validation";
  point: AtlasPointMeters;
  regionId: AtlasRegionId;
  segmentKind: AtlasRouteSegmentKind;
}

export interface AtlasCaveAnchor {
  id: string;
  caveSystemId: AtlasCaveSystemId;
  kind: AtlasCaveAnchorKind;
  point: AtlasPointMeters;
  regionId: AtlasRegionId;
  radiusM: number;
  dryEntranceRequired: boolean;
  associatedRouteId: AtlasRouteId | null;
  landmarkMarkerIds: readonly string[];
}

export interface WorldAtlas {
  version: string;
  island: IslandEnvelope;
  regions: readonly AtlasRegionDefinition[];
  regionEdges: readonly AtlasRegionEdgeDefinition[];
  routes: readonly AtlasRouteDefinition[];
  caveSystems: readonly AtlasCaveSystemDefinition[];
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

export interface AtlasRouteSample {
  routeId: AtlasRouteId | null;
  routeInfluence: number;
  routeCore: number;
  routeShoulder: number;
  distanceAlongM: number;
  distanceToRouteM: number;
  routeSegmentKind: AtlasRouteSegmentKind | null;
  recommendedSetPieceIds: readonly string[];
}

export interface AtlasCaveAnchorSample {
  caveSystemId: AtlasCaveSystemId | null;
  caveAnchorId: string | null;
  caveAnchorKind: AtlasCaveAnchorKind | null;
  caveAnchorPoint: AtlasPointMeters | null;
  caveDryEntranceRequired: boolean;
  caveInfluence: number;
  caveCore: number;
  distanceToCaveAnchorM: number;
  caveRegionId: AtlasRegionId | null;
  associatedRouteId: AtlasRouteId | null;
  caveMaterialProfileId: string | null;
  caveLandmarkMarkerIds: readonly string[];
}

export interface WorldAtlasSample extends IslandMaskSample, AtlasRegionSample, AtlasRouteSample, AtlasCaveAnchorSample {
  xM: number;
  zM: number;
}

interface RegionScore {
  region: AtlasRegionDefinition;
  distance: number;
  score: number;
}

interface RouteProjection {
  route: AtlasRouteDefinition;
  distanceToRouteM: number;
  distanceAlongM: number;
  segmentIndex: number;
  t: number;
}

interface CaveAnchorProjection {
  system: AtlasCaveSystemDefinition;
  anchor: AtlasCaveAnchorDefinition;
  distanceToAnchorM: number;
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
  routes: [
    {
      id: "pilgrim-spine-red",
      kind: "pilgrim-road",
      nodes: [
        { id: "inner-sea-shrine-road", point: { x: 1_320, z: 120 }, regionId: "inner-sea" },
        { id: "wet-ash-approach", point: { x: 400, z: -520 }, regionId: "inner-sea" },
        { id: "red-caldera-gate", point: { x: -520, z: -1_080 }, regionId: "red-mountain" },
        { id: "ash-apron-road", point: { x: -680, z: -1_720 }, regionId: "ashen-badlands" },
        { id: "badlands-pilgrim-end", point: { x: -840, z: -2_360 }, regionId: "ashen-badlands" },
      ],
      widthM: 72,
      shoulderM: 180,
      materialProfileId: "ash-basalt-pilgrim-road",
      expectedRegionIds: ["inner-sea", "red-mountain", "ashen-badlands"],
      landmarkCadenceM: 240,
      strongVistaCadenceM: 720,
      segmentKind: "sacred-road",
      recommendedSetPieceIds: ["road-shrine", "ash-marker", "caldera-vista"],
      validationAnchor: { x: -60, z: -800 },
    },
    {
      id: "ash-gash-pass",
      kind: "mountain-pass",
      nodes: [
        { id: "ash-ravine-mouth", point: { x: -840, z: -2_360 }, regionId: "ashen-badlands" },
        { id: "ash-gash-saddle", point: { x: -1_780, z: -2_810 }, regionId: "ashen-badlands" },
        { id: "west-gash-redleaf-pass", point: { x: -2_720, z: -3_260 }, regionId: "west-gash" },
      ],
      widthM: 54,
      shoulderM: 150,
      materialProfileId: "ash-stone-ravine-pass",
      expectedRegionIds: ["ashen-badlands", "west-gash"],
      landmarkCadenceM: 260,
      strongVistaCadenceM: 780,
      segmentKind: "ravine-pass",
      recommendedSetPieceIds: ["ravine-switchback", "kwama-mine-mouth", "redleaf-tor"],
      validationAnchor: { x: -1_780, z: -2_810 },
    },
    {
      id: "badlands-east-trail",
      kind: "hazard-route",
      nodes: [
        { id: "badlands-east-camp", point: { x: -840, z: -2_360 }, regionId: "ashen-badlands" },
        { id: "eastward-ash-trail", point: { x: 1_290, z: -2_720 }, regionId: "ashen-badlands" },
        { id: "grazelands-west-camp", point: { x: 3_420, z: -3_080 }, regionId: "grazelands" },
      ],
      widthM: 58,
      shoulderM: 190,
      materialProfileId: "ash-grass-caravan-trail",
      expectedRegionIds: ["ashen-badlands", "grazelands"],
      landmarkCadenceM: 420,
      strongVistaCadenceM: 1_050,
      segmentKind: "caravan-trail",
      recommendedSetPieceIds: ["trail-camp", "ash-windbreak", "grass-marker"],
      validationAnchor: { x: 1_290, z: -2_720 },
    },
    {
      id: "bitter-inner-crossing",
      kind: "causeway",
      nodes: [
        { id: "bitter-blackwater-dock", point: { x: -4_360, z: 920 }, regionId: "bitter-coast" },
        { id: "wetland-bridge-chain", point: { x: -1_520, z: 520 }, regionId: "bitter-coast" },
        { id: "inner-sea-west-bank", point: { x: 1_320, z: 120 }, regionId: "inner-sea" },
      ],
      widthM: 64,
      shoulderM: 210,
      materialProfileId: "blackwater-peat-bridge",
      expectedRegionIds: ["bitter-coast", "inner-sea"],
      landmarkCadenceM: 300,
      strongVistaCadenceM: 840,
      segmentKind: "wetland-bridge",
      recommendedSetPieceIds: ["reed-bridge", "dry-hummock", "old-stone-marker"],
      validationAnchor: { x: -1_520, z: 520 },
    },
    {
      id: "salt-causeway",
      kind: "causeway",
      nodes: [
        { id: "salt-west-rib", point: { x: -1_860, z: 4_040 }, regionId: "salt-marsh-basin" },
        { id: "salt-mirror-center", point: { x: -180, z: 4_040 }, regionId: "salt-marsh-basin" },
        { id: "salt-glass-rib", point: { x: 2_280, z: 3_110 }, regionId: "glass-shard-coast" },
      ],
      widthM: 80,
      shoulderM: 240,
      materialProfileId: "white-salt-dead-causeway",
      expectedRegionIds: ["salt-marsh-basin", "glass-shard-coast"],
      landmarkCadenceM: 320,
      strongVistaCadenceM: 960,
      segmentKind: "salt-causeway",
      recommendedSetPieceIds: ["salt-rib", "sunken-pylon", "glass-warning-marker"],
      validationAnchor: { x: -180, z: 4_040 },
    },
    {
      id: "inner-sea-shelf-road",
      kind: "coastal-walk",
      nodes: [
        { id: "inner-north-shelf", point: { x: 1_320, z: 120 }, regionId: "inner-sea" },
        { id: "inner-south-shelf", point: { x: 570, z: 2_080 }, regionId: "inner-sea" },
        { id: "salt-basin-north-road", point: { x: -180, z: 4_040 }, regionId: "salt-marsh-basin" },
      ],
      widthM: 66,
      shoulderM: 190,
      materialProfileId: "moor-shelf-old-road",
      expectedRegionIds: ["inner-sea", "salt-marsh-basin"],
      landmarkCadenceM: 280,
      strongVistaCadenceM: 760,
      segmentKind: "shelf-road",
      recommendedSetPieceIds: ["old-road-slab", "low-island", "silt-marker"],
      validationAnchor: { x: 570, z: 2_080 },
    },
    {
      id: "grazelands-glass-road",
      kind: "hazard-route",
      nodes: [
        { id: "grazelands-flower-road", point: { x: 3_420, z: -3_080 }, regionId: "grazelands" },
        { id: "glass-road-warning", point: { x: 4_080, z: -450 }, regionId: "grazelands" },
        { id: "glass-shard-inland-gate", point: { x: 4_740, z: 2_180 }, regionId: "glass-shard-coast" },
      ],
      widthM: 62,
      shoulderM: 220,
      materialProfileId: "dry-grass-glass-road",
      expectedRegionIds: ["grazelands", "glass-shard-coast"],
      landmarkCadenceM: 450,
      strongVistaCadenceM: 1_000,
      segmentKind: "hazard-cairns",
      recommendedSetPieceIds: ["grass-cairn", "glass-warning-marker", "shard-vista"],
      validationAnchor: { x: 4_080, z: -450 },
    },
    {
      id: "glass-coastal-cairns",
      kind: "coastal-walk",
      nodes: [
        { id: "glass-north-cairns", point: { x: 5_140, z: 520 }, regionId: "glass-shard-coast" },
        { id: "glass-shard-coastal-spine", point: { x: 4_740, z: 2_180 }, regionId: "glass-shard-coast" },
        { id: "glass-salt-cairns", point: { x: 2_280, z: 3_110 }, regionId: "glass-shard-coast" },
      ],
      widthM: 50,
      shoulderM: 180,
      materialProfileId: "glass-coastal-cairns",
      expectedRegionIds: ["glass-shard-coast", "salt-marsh-basin"],
      landmarkCadenceM: 300,
      strongVistaCadenceM: 800,
      segmentKind: "hazard-cairns",
      recommendedSetPieceIds: ["glass-cairn", "coastal-warning", "shard-shelter"],
      validationAnchor: { x: 4_940, z: 1_350 },
    },
  ],
  caveSystems: [
    {
      id: "red-caldera-tubes",
      regionId: "red-mountain",
      kind: "lava-tube",
      anchors: [
        {
          id: "red-caldera-gate-tube",
          kind: "entrance",
          point: { x: -520, z: -1_080 },
          regionId: "red-mountain",
          radiusM: 120,
          dryEntranceRequired: true,
          associatedRouteId: "pilgrim-spine-red",
          landmarkMarkerIds: ["caldera-vista", "basalt-spire"],
        },
        {
          id: "red-rim-chamber",
          kind: "chamber",
          point: { x: -260, z: -1_260 },
          regionId: "red-mountain",
          radiusM: 170,
          dryEntranceRequired: true,
          associatedRouteId: "pilgrim-spine-red",
          landmarkMarkerIds: ["ash-marker", "rim-vent"],
        },
        {
          id: "red-ash-apron-mouth",
          kind: "entrance",
          point: { x: -680, z: -1_720 },
          regionId: "ashen-badlands",
          radiusM: 130,
          dryEntranceRequired: true,
          associatedRouteId: "pilgrim-spine-red",
          landmarkMarkerIds: ["ash-marker", "black-pumice"],
        },
      ],
      tunnels: [
        {
          from: "red-caldera-gate-tube",
          to: "red-rim-chamber",
          widthM: 68,
          verticalBiasM: -42,
          materialProfileId: "basalt-lava-tube",
        },
        {
          from: "red-caldera-gate-tube",
          to: "red-ash-apron-mouth",
          widthM: 54,
          verticalBiasM: -58,
          materialProfileId: "ash-basalt-vent",
        },
      ],
      materialProfileId: "basalt-lava-tube",
      expectedRouteIds: ["pilgrim-spine-red"],
      validationAnchor: { x: -520, z: -1_080 },
    },
    {
      id: "ash-kwama-ravines",
      regionId: "ashen-badlands",
      kind: "kwama-mine",
      anchors: [
        {
          id: "ash-pilgrim-mine-mouth",
          kind: "entrance",
          point: { x: -840, z: -2_360 },
          regionId: "ashen-badlands",
          radiusM: 150,
          dryEntranceRequired: true,
          associatedRouteId: "ash-gash-pass",
          landmarkMarkerIds: ["kwama-mound", "scree-fan"],
        },
        {
          id: "ash-gash-saddle-mine",
          kind: "entrance",
          point: { x: -1_780, z: -2_810 },
          regionId: "ashen-badlands",
          radiusM: 125,
          dryEntranceRequired: true,
          associatedRouteId: "ash-gash-pass",
          landmarkMarkerIds: ["ravine-switchback", "kwama-mound"],
        },
        {
          id: "ash-deep-brood-chamber",
          kind: "chamber",
          point: { x: -1_240, z: -2_600 },
          regionId: "ashen-badlands",
          radiusM: 190,
          dryEntranceRequired: true,
          associatedRouteId: "badlands-east-trail",
          landmarkMarkerIds: ["ash-windbreak", "buried-ribs"],
        },
      ],
      tunnels: [
        {
          from: "ash-pilgrim-mine-mouth",
          to: "ash-deep-brood-chamber",
          widthM: 62,
          verticalBiasM: -36,
          materialProfileId: "ash-kwama-mine",
        },
        {
          from: "ash-deep-brood-chamber",
          to: "ash-gash-saddle-mine",
          widthM: 58,
          verticalBiasM: -24,
          materialProfileId: "ash-ravine-mine",
        },
      ],
      materialProfileId: "ash-kwama-mine",
      expectedRouteIds: ["ash-gash-pass", "badlands-east-trail"],
      validationAnchor: { x: -1_240, z: -2_600 },
    },
    {
      id: "bitter-root-grottos",
      regionId: "bitter-coast",
      kind: "root-cave",
      anchors: [
        {
          id: "bitter-blackwater-root-mouth",
          kind: "entrance",
          point: { x: -4_360, z: 920 },
          regionId: "bitter-coast",
          radiusM: 140,
          dryEntranceRequired: true,
          associatedRouteId: "bitter-inner-crossing",
          landmarkMarkerIds: ["reed-bridge", "root-arch"],
        },
        {
          id: "bitter-hummock-grotto",
          kind: "chamber",
          point: { x: -3_080, z: 760 },
          regionId: "bitter-coast",
          radiusM: 175,
          dryEntranceRequired: true,
          associatedRouteId: "bitter-inner-crossing",
          landmarkMarkerIds: ["dry-hummock", "mangrove-root"],
        },
        {
          id: "bitter-bridge-root-mouth",
          kind: "entrance",
          point: { x: -1_520, z: 520 },
          regionId: "bitter-coast",
          radiusM: 115,
          dryEntranceRequired: true,
          associatedRouteId: "bitter-inner-crossing",
          landmarkMarkerIds: ["old-stone-marker", "root-arch"],
        },
      ],
      tunnels: [
        {
          from: "bitter-blackwater-root-mouth",
          to: "bitter-hummock-grotto",
          widthM: 52,
          verticalBiasM: -18,
          materialProfileId: "blackwater-root-grotto",
        },
        {
          from: "bitter-hummock-grotto",
          to: "bitter-bridge-root-mouth",
          widthM: 48,
          verticalBiasM: -16,
          materialProfileId: "peat-root-tunnel",
        },
      ],
      materialProfileId: "blackwater-root-grotto",
      expectedRouteIds: ["bitter-inner-crossing"],
      validationAnchor: { x: -3_080, z: 760 },
    },
    {
      id: "west-gash-ravine-caves",
      regionId: "west-gash",
      kind: "granitic-cave",
      anchors: [
        {
          id: "west-gash-redleaf-mouth",
          kind: "entrance",
          point: { x: -2_720, z: -3_260 },
          regionId: "west-gash",
          radiusM: 135,
          dryEntranceRequired: true,
          associatedRouteId: "ash-gash-pass",
          landmarkMarkerIds: ["redleaf-tor", "stone-overlook"],
        },
        {
          id: "west-switchback-overlook",
          kind: "overlook",
          point: { x: -2_240, z: -3_020 },
          regionId: "west-gash",
          radiusM: 110,
          dryEntranceRequired: true,
          associatedRouteId: "ash-gash-pass",
          landmarkMarkerIds: ["ravine-switchback", "redleaf-tor"],
        },
        {
          id: "west-ravine-chamber",
          kind: "chamber",
          point: { x: -2_480, z: -3_460 },
          regionId: "west-gash",
          radiusM: 185,
          dryEntranceRequired: true,
          associatedRouteId: "ash-gash-pass",
          landmarkMarkerIds: ["weathered-stone", "redleaf-root"],
        },
      ],
      tunnels: [
        {
          from: "west-gash-redleaf-mouth",
          to: "west-ravine-chamber",
          widthM: 56,
          verticalBiasM: -34,
          materialProfileId: "granitic-ravine-cave",
        },
        {
          from: "west-switchback-overlook",
          to: "west-ravine-chamber",
          widthM: 44,
          verticalBiasM: -46,
          materialProfileId: "redleaf-stone-cavern",
        },
      ],
      materialProfileId: "granitic-ravine-cave",
      expectedRouteIds: ["ash-gash-pass"],
      validationAnchor: { x: -2_720, z: -3_260 },
    },
    {
      id: "glass-crystal-caverns",
      regionId: "glass-shard-coast",
      kind: "crystal-cavern",
      anchors: [
        {
          id: "glass-inland-crystal-mouth",
          kind: "entrance",
          point: { x: 4_740, z: 2_180 },
          regionId: "glass-shard-coast",
          radiusM: 145,
          dryEntranceRequired: true,
          associatedRouteId: "grazelands-glass-road",
          landmarkMarkerIds: ["shard-vista", "glass-warning-marker"],
        },
        {
          id: "glass-coastal-spine-cavern",
          kind: "chamber",
          point: { x: 4_940, z: 1_350 },
          regionId: "glass-shard-coast",
          radiusM: 180,
          dryEntranceRequired: true,
          associatedRouteId: "glass-coastal-cairns",
          landmarkMarkerIds: ["glass-cairn", "crystal-ridge"],
        },
        {
          id: "glass-north-cairn-mouth",
          kind: "entrance",
          point: { x: 5_140, z: 520 },
          regionId: "glass-shard-coast",
          radiusM: 115,
          dryEntranceRequired: true,
          associatedRouteId: "glass-coastal-cairns",
          landmarkMarkerIds: ["coastal-warning", "glass-cairn"],
        },
      ],
      tunnels: [
        {
          from: "glass-inland-crystal-mouth",
          to: "glass-coastal-spine-cavern",
          widthM: 60,
          verticalBiasM: -28,
          materialProfileId: "cold-crystal-cavern",
        },
        {
          from: "glass-coastal-spine-cavern",
          to: "glass-north-cairn-mouth",
          widthM: 46,
          verticalBiasM: -22,
          materialProfileId: "glass-ridge-cavern",
        },
      ],
      materialProfileId: "cold-crystal-cavern",
      expectedRouteIds: ["grazelands-glass-road", "glass-coastal-cairns"],
      validationAnchor: { x: 4_940, z: 1_350 },
    },
    {
      id: "salt-crust-sinkholes",
      regionId: "salt-marsh-basin",
      kind: "saline-sinkhole",
      anchors: [
        {
          id: "salt-west-rib-sinkhole",
          kind: "entrance",
          point: { x: -1_860, z: 4_040 },
          regionId: "salt-marsh-basin",
          radiusM: 120,
          dryEntranceRequired: true,
          associatedRouteId: "salt-causeway",
          landmarkMarkerIds: ["salt-rib", "sunken-pylon"],
        },
        {
          id: "salt-mirror-crust-chamber",
          kind: "chamber",
          point: { x: -180, z: 4_040 },
          regionId: "salt-marsh-basin",
          radiusM: 210,
          dryEntranceRequired: true,
          associatedRouteId: "salt-causeway",
          landmarkMarkerIds: ["salt-rib", "mirror-crust"],
        },
        {
          id: "salt-glass-rib-sinkhole",
          kind: "entrance",
          point: { x: 2_280, z: 3_110 },
          regionId: "salt-marsh-basin",
          radiusM: 130,
          dryEntranceRequired: true,
          associatedRouteId: "salt-causeway",
          landmarkMarkerIds: ["glass-warning-marker", "salt-rib"],
        },
      ],
      tunnels: [
        {
          from: "salt-west-rib-sinkhole",
          to: "salt-mirror-crust-chamber",
          widthM: 66,
          verticalBiasM: -20,
          materialProfileId: "saline-crust-cave",
        },
        {
          from: "salt-mirror-crust-chamber",
          to: "salt-glass-rib-sinkhole",
          widthM: 62,
          verticalBiasM: -18,
          materialProfileId: "salt-glass-sinkhole",
        },
      ],
      materialProfileId: "saline-crust-cave",
      expectedRouteIds: ["salt-causeway"],
      validationAnchor: { x: -180, z: 4_040 },
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
  const routeSample = sampleAtlasRouteMeters(xM, zM, islandMask, atlas);
  const caveAnchorSample = sampleAtlasCaveAnchorMeters(xM, zM, islandMask, atlas);

  return {
    xM,
    zM,
    ...islandMask,
    ...regionSample,
    ...routeSample,
    ...caveAnchorSample,
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

export function findAtlasRoute(routeId: AtlasRouteId, atlas = WORLD_ATLAS): AtlasRouteDefinition {
  const route = atlas.routes.find((candidate) => candidate.id === routeId);
  if (!route) {
    throw new Error(`Unknown atlas route: ${routeId}`);
  }
  return route;
}

export function findAtlasCaveSystem(caveSystemId: AtlasCaveSystemId, atlas = WORLD_ATLAS): AtlasCaveSystemDefinition {
  const caveSystem = atlas.caveSystems.find((candidate) => candidate.id === caveSystemId);
  if (!caveSystem) {
    throw new Error(`Unknown atlas cave system: ${caveSystemId}`);
  }
  return caveSystem;
}

export function estimateAtlasRegionEllipseAreaM2(region: AtlasRegionDefinition): number {
  return Math.PI * region.radius.x * region.radius.z;
}

export function getAtlasRegionGraph(atlas = WORLD_ATLAS): readonly AtlasRegionGraphNode[] {
  return atlas.regions.map((region) => ({
    regionId: region.id,
    center: region.center,
    radius: region.radius,
    approximateEllipseAreaM2: estimateAtlasRegionEllipseAreaM2(region),
    edgeIds: atlas.regionEdges
      .filter((edge) => edge.from === region.id || edge.to === region.id)
      .map((edge) => edge.id),
    routeIds: atlas.routes
      .filter((route) => route.expectedRegionIds.includes(region.id))
      .map((route) => route.id),
    caveSystemIds: atlas.caveSystems
      .filter((caveSystem) =>
        caveSystem.regionId === region.id ||
        caveSystem.anchors.some((anchor) => anchor.regionId === region.id)
      )
      .map((caveSystem) => caveSystem.id),
  }));
}

export function getAtlasRouteAnchors(atlas = WORLD_ATLAS): readonly AtlasRouteAnchor[] {
  return atlas.routes.flatMap((route) => [
    ...route.nodes.map((node): AtlasRouteAnchor => ({
      id: node.id,
      routeId: route.id,
      kind: "node",
      point: node.point,
      regionId: node.regionId,
      segmentKind: route.segmentKind,
    })),
    {
      id: `${route.id}-validation`,
      routeId: route.id,
      kind: "validation" as const,
      point: route.validationAnchor,
      regionId: route.expectedRegionIds[0]!,
      segmentKind: route.segmentKind,
    },
  ]);
}

export function getAtlasCaveAnchors(atlas = WORLD_ATLAS): readonly AtlasCaveAnchor[] {
  return atlas.caveSystems.flatMap((caveSystem) =>
    caveSystem.anchors.map((anchor): AtlasCaveAnchor => ({
      id: anchor.id,
      caveSystemId: caveSystem.id,
      kind: anchor.kind,
      point: anchor.point,
      regionId: anchor.regionId,
      radiusM: anchor.radiusM,
      dryEntranceRequired: anchor.dryEntranceRequired,
      associatedRouteId: anchor.associatedRouteId,
      landmarkMarkerIds: anchor.landmarkMarkerIds,
    })),
  );
}

export function sampleAtlasRouteMeters(
  xM: number,
  zM: number,
  islandMask = sampleIslandMaskMeters(xM, zM),
  atlas = WORLD_ATLAS,
): AtlasRouteSample {
  if (islandMask.islandInterior < LAND_CUTOFF_INTERIOR) {
    return emptyRouteSample();
  }

  const projection = findNearestRouteProjection(xM, zM, atlas);
  if (!projection) {
    return emptyRouteSample();
  }

  const routeReachM = projection.route.widthM + projection.route.shoulderM;
  const routeInfluence = 1 - smoothstep(projection.route.widthM, routeReachM, projection.distanceToRouteM);
  const routeCore = 1 - smoothstep(projection.route.widthM * 0.58, projection.route.widthM, projection.distanceToRouteM);
  const routeShoulder =
    smoothstep(projection.route.widthM * 0.58, projection.route.widthM, projection.distanceToRouteM) *
    (1 - smoothstep(projection.route.widthM, routeReachM, projection.distanceToRouteM));

  if (routeInfluence <= 0) {
    return {
      ...emptyRouteSample(),
      distanceToRouteM: projection.distanceToRouteM,
      distanceAlongM: projection.distanceAlongM,
    };
  }

  return {
    routeId: projection.route.id,
    routeInfluence,
    routeCore,
    routeShoulder,
    distanceAlongM: projection.distanceAlongM,
    distanceToRouteM: projection.distanceToRouteM,
    routeSegmentKind: projection.route.segmentKind,
    recommendedSetPieceIds: projection.route.recommendedSetPieceIds,
  };
}

export function sampleAtlasCaveAnchorMeters(
  xM: number,
  zM: number,
  islandMask = sampleIslandMaskMeters(xM, zM),
  atlas = WORLD_ATLAS,
): AtlasCaveAnchorSample {
  if (islandMask.islandInterior < LAND_CUTOFF_INTERIOR) {
    return emptyCaveAnchorSample();
  }

  const projection = findNearestCaveAnchorProjection(xM, zM, atlas);
  if (!projection) {
    return emptyCaveAnchorSample();
  }

  const outerRadiusM = projection.anchor.radiusM * 2.35;
  const caveInfluence = 1 - smoothstep(projection.anchor.radiusM, outerRadiusM, projection.distanceToAnchorM);
  const caveCore = 1 - smoothstep(projection.anchor.radiusM * 0.45, projection.anchor.radiusM, projection.distanceToAnchorM);

  if (caveInfluence <= 0) {
    return {
      ...emptyCaveAnchorSample(),
      distanceToCaveAnchorM: projection.distanceToAnchorM,
    };
  }

  return {
    caveSystemId: projection.system.id,
    caveAnchorId: projection.anchor.id,
    caveAnchorKind: projection.anchor.kind,
    caveAnchorPoint: projection.anchor.point,
    caveDryEntranceRequired: projection.anchor.dryEntranceRequired,
    caveInfluence,
    caveCore,
    distanceToCaveAnchorM: projection.distanceToAnchorM,
    caveRegionId: projection.anchor.regionId,
    associatedRouteId: projection.anchor.associatedRouteId,
    caveMaterialProfileId: projection.system.materialProfileId,
    caveLandmarkMarkerIds: projection.anchor.landmarkMarkerIds,
  };
}

export function sampleAtlasRegionMeters(
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

function emptyRouteSample(): AtlasRouteSample {
  return {
    routeId: null,
    routeInfluence: 0,
    routeCore: 0,
    routeShoulder: 0,
    distanceAlongM: Infinity,
    distanceToRouteM: Infinity,
    routeSegmentKind: null,
    recommendedSetPieceIds: [],
  };
}

function emptyCaveAnchorSample(): AtlasCaveAnchorSample {
  return {
    caveSystemId: null,
    caveAnchorId: null,
    caveAnchorKind: null,
    caveAnchorPoint: null,
    caveDryEntranceRequired: false,
    caveInfluence: 0,
    caveCore: 0,
    distanceToCaveAnchorM: Infinity,
    caveRegionId: null,
    associatedRouteId: null,
    caveMaterialProfileId: null,
    caveLandmarkMarkerIds: [],
  };
}

function findNearestRouteProjection(xM: number, zM: number, atlas: WorldAtlas): RouteProjection | null {
  let nearest: RouteProjection | null = null;
  for (const route of atlas.routes) {
    let distanceBeforeSegmentM = 0;
    for (let index = 0; index < route.nodes.length - 1; index += 1) {
      const a = route.nodes[index]!.point;
      const b = route.nodes[index + 1]!.point;
      const segment = projectPointToSegment(xM, zM, a, b);
      const segmentLengthM = distanceBetweenPoints(a, b);
      const projection: RouteProjection = {
        route,
        distanceToRouteM: segment.distance,
        distanceAlongM: distanceBeforeSegmentM + segmentLengthM * segment.t,
        segmentIndex: index,
        t: segment.t,
      };

      if (!nearest || projection.distanceToRouteM < nearest.distanceToRouteM) {
        nearest = projection;
      }

      distanceBeforeSegmentM += segmentLengthM;
    }
  }
  return nearest;
}

function findNearestCaveAnchorProjection(xM: number, zM: number, atlas: WorldAtlas): CaveAnchorProjection | null {
  let nearest: CaveAnchorProjection | null = null;
  for (const system of atlas.caveSystems) {
    for (const anchor of system.anchors) {
      const distanceToAnchorM = Math.hypot(xM - anchor.point.x, zM - anchor.point.z);
      if (!nearest || distanceToAnchorM < nearest.distanceToAnchorM) {
        nearest = { system, anchor, distanceToAnchorM };
      }
    }
  }
  return nearest;
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

function distanceBetweenPoints(a: AtlasPointMeters, b: AtlasPointMeters): number {
  return Math.hypot(b.x - a.x, b.z - a.z);
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
