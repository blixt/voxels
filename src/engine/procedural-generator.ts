import { clamp, packRgba } from "./math.ts";
import { fbm2D2, fbm2D3, fbm2D4, fbm2D5, hashNoise3D } from "./noise.ts";
import {
  summarizeGeneratedChunkRender,
  type GeneratedChunkRenderSummary,
} from "./generated-chunk-render-summary.ts";
import type { ChunkBounds, ChunkCoordinate } from "./types.ts";
import {
  sampleWorldRegion,
  WORLD_REGION_AUTHORITY_THRESHOLD,
  type WorldRegionId,
} from "./worldgen-region.ts";
import {
  sampleAtlasCaveAnchorMeters,
  sampleAtlasRegionMeters,
  sampleIslandMaskMeters,
  WORLD_ATLAS,
} from "./world-atlas.ts";
import type { AmbientProfileId } from "./ambient-environment.ts";

export const HEX_COLOR_COUNT = 0x1000;
export const PROCEDURAL_WORLD_MAX_Y = 16_384;
export const PROCEDURAL_WORLD_GENERATION_VERSION = "20260508-region-authority-lod-shell-v2";

export type BaseBiomeId = "verdant" | "savanna" | "steppe" | "dunes" | "badlands" | "highland" | "moor" | "tundra";
export type SpecialBiomeId = "marsh" | "firefly" | "saltflat" | "fern" | "fungal" | "ember" | "bloom" | "shardlands";
export type BiomeId = BaseBiomeId | SpecialBiomeId;
export type UndergroundBiomeId =
  | "rooted"
  | "sedimentary"
  | "sandy"
  | "granitic"
  | "froststone"
  | "basaltic"
  | "peaty"
  | "saline"
  | "mycelial"
  | "crystalline";
export type RegionalVariantId =
  | "verdant_karst"
  | "savanna_flowersea"
  | "steppe_monolith"
  | "dunes_glass"
  | "badlands_crater"
  | "ash_wastes"
  | "highland_redleaf"
  | "moor_shadowglass"
  | "tundra_blue_ice"
  | "marsh_blackwater"
  | "firefly_lantern"
  | "saltflat_mirror"
  | "fern_cenote"
  | "fungal_moonlit"
  | "ember_caldera"
  | "bloom_prism";
export type LandmarkId =
  | "oak"
  | "canopy_tree"
  | "birch"
  | "redleaf_tree"
  | "willow"
  | "blossom_tree"
  | "fruit_tree"
  | "giant_flower"
  | "redwood"
  | "dead_tree"
  | "thorn_tree"
  | "berry_bush"
  | "giant_fern"
  | "lantern_tree"
  | "salt_spire"
  | "boulder"
  | "standing_stone"
  | "shrub"
  | "flower_patch"
  | "palm"
  | "acacia"
  | "cactus"
  | "dead_snag"
  | "hoodoo"
  | "fir"
  | "tall_fir"
  | "ice_spire"
  | "frost_shrub"
  | "cypress"
  | "mangrove"
  | "reed_cluster"
  | "basalt_spire"
  | "crystal_cluster"
  | "glowcap"
  | "mega_glowcap"
  | "root_stump"
  | "stone_tor"
  | "ancestor_pillar"
  | "ash_marker"
  | "glass_cairn"
  | "silt_shell"
  | "velothi_shrine"
  | "kwama_mound"
  | "pilgrim_cairn"
  | "velothi_ziggurat"
  | "ash_obelisk"
  | "rib_arch"
  | "old_road_causeway"
  | "paver_debris"
  | "scree_fan"
  | "shrine_debris"
  | "buried_ribs"
  | "pilgrim_lantern"
  | "bone_chimes"
  | "ashlander_travel_pack"
  | "crystal_reeds"
  | "fungal_bridge"
  | "rib_remains";

interface BaseBiomeProfile {
  id: BaseBiomeId;
  temperature: number;
  moisture: number;
  uplift: number;
  drainage: number;
  heightCenter: number;
  heightRange: number;
  heightBias: number;
  reliefScale: number;
  ridgeScale: number;
  detailScale: number;
  basinScale: number;
  terraceScale: number;
  microRelief: number;
  snowLine: number;
  surface: number;
  transitionSurface: number;
  surfaceVariant: number;
  surfaceAccent: number;
  surfaceRock: number;
  subsurface: number;
  subsurfaceVariant: number;
  water: number;
  snow: number;
}

interface SpecialBiomeProfile {
  id: SpecialBiomeId;
  surface: number;
  transitionSurface: number;
  surfaceVariant: number;
  surfaceAccent: number;
  surfaceRock: number;
  subsurface: number;
  subsurfaceVariant: number;
  water: number;
  snow: number;
  softTransition: boolean;
}

interface UndergroundBiomeProfile {
  id: UndergroundBiomeId;
  stone: number;
  deepStone: number;
  accent: number;
}

interface LandmarkProfile {
  id: LandmarkId;
  cellSize: number;
  radius: number;
  chance: number;
  scale: number;
  variant: number;
}

interface SurfaceFieldSample {
  regionId: WorldRegionId;
  secondaryRegionId: WorldRegionId;
  regionStrength: number;
  regionBlend: number;
  regionBiomeId: BiomeId;
  regionAmbientProfileId: AmbientProfileId;
  regionVariantId: RegionalVariantId | null;
  temperature: number;
  moisture: number;
  uplift: number;
  drainage: number;
  volcanism: number;
  magic: number;
  globalHeight: number;
  mountainness: number;
  oceanness: number;
  continentalness: number;
  hills: number;
  detail: number;
  ridge: number;
  basin: number;
  channel: number;
  dune: number;
  mesa: number;
  grove: number;
  oldGrowth: number;
  orchard: number;
  desolation: number;
  strata: number;
  surfacePatch: number;
  surfaceGrain: number;
  scatter: number;
  peakness: number;
  islandInterior: number;
  shorelineBand: number;
  coastalShelf: number;
  deepOcean: number;
  atlasRouteInfluence: number;
  atlasRouteCore: number;
  atlasRouteShoulder: number;
  atlasCaveInfluence: number;
  atlasCaveCore: number;
  volcanicHeart: number;
  ashRing: number;
  westWetlands: number;
  northeastGrazelands: number;
  southernSaltBasin: number;
  easternShardCoast: number;
}

interface CaveFieldSample {
  caveRibbon: number;
  cavePocket: number;
  caveDepth: number;
  caveOpenings: number;
}

function createSurfaceFieldSample(): SurfaceFieldSample {
  return {
    regionId: "inner-sea", secondaryRegionId: "inner-sea", regionStrength: 0,
    regionBlend: 1, regionBiomeId: "moor", regionAmbientProfileId: "silt-mist",
    regionVariantId: null,
    temperature: 0, moisture: 0, uplift: 0, drainage: 0, volcanism: 0, magic: 0,
    globalHeight: 0, mountainness: 0, oceanness: 0, continentalness: 0, hills: 0,
    detail: 0, ridge: 0, basin: 0, channel: 0, dune: 0, mesa: 0, grove: 0,
    oldGrowth: 0, orchard: 0, desolation: 0, strata: 0, surfacePatch: 0,
    surfaceGrain: 0, scatter: 0, peakness: 0, islandInterior: 0, coastalShelf: 0,
    shorelineBand: 0, deepOcean: 0, atlasRouteInfluence: 0, atlasRouteCore: 0,
    atlasRouteShoulder: 0, atlasCaveInfluence: 0, atlasCaveCore: 0, volcanicHeart: 0,
    ashRing: 0, westWetlands: 0, northeastGrazelands: 0, southernSaltBasin: 0,
    easternShardCoast: 0,
  };
}

function createCaveFieldSample(): CaveFieldSample {
  return { caveRibbon: 0, cavePocket: 0, caveDepth: 0, caveOpenings: 0 };
}

interface MutableColumnState {
  regionId: WorldRegionId;
  secondaryRegionId: WorldRegionId;
  regionStrength: number;
  regionAmbientProfileId: AmbientProfileId;
  biomeId: BiomeId;
  hostBiomeId: BaseBiomeId;
  secondaryBiomeId: BaseBiomeId;
  undergroundBiomeId: UndergroundBiomeId;
  regionalVariantId: RegionalVariantId | null;
  regionalVariantStrength: number;
  landmarkId: LandmarkId | null;
  temperature: number;
  moisture: number;
  uplift: number;
  drainage: number;
  volcanism: number;
  magic: number;
  globalHeight: number;
  mountainness: number;
  oceanness: number;
  surfaceY: number;
  waterTopY: number;
  pilgrimRouteInfluence: number;
  surfaceMaterialPrimary: number;
  surfaceMaterialSecondary: number;
  subsurfacePrimary: number;
  subsurfaceSecondary: number;
  waterMaterial: number;
  snowMaterial: number;
  stoneMaterial: number;
  deepStoneMaterial: number;
  accentMaterial: number;
  transitionThreshold: number;
  specialStrength: number;
  caveMainField: number;
  caveMainStrength: number;
  caveMainCenterY: number;
  caveMainHalfHeight: number;
  caveUpperField: number;
  caveUpperStrength: number;
  caveUpperCenterY: number;
  caveUpperHalfHeight: number;
  caveEntranceField: number;
  caveEntranceStrength: number;
  caveEntranceCenterY: number;
  caveEntranceHalfHeight: number;
  strataOffset: number;
  worldXDiv3: number;
  worldZDiv3: number;
  ditherSeed: number;
  accentSeed: number;
  featureKind: number;
  featureHeight: number;
  featureRadius: number;
  featureExtra: number;
  featureDeltaX: number;
  featureDeltaZ: number;
  featureMaterialPrimary: number;
  featureMaterialSecondary: number;
  featureMaterialAccent: number;
}

export interface ProceduralColumnSample {
  biomeId: BiomeId;
  hostBiomeId: BaseBiomeId;
  undergroundBiomeId: UndergroundBiomeId;
  regionalVariantId: RegionalVariantId | null;
  landmarkId: LandmarkId | null;
  surfaceY: number;
  topY: number;
  waterTopY: number | null;
  pilgrimRouteInfluence: number;
  surfaceMaterial: number;
}

export interface ProceduralSurfaceColumnSample {
  biomeId: BiomeId;
  surfaceY: number;
  topY: number;
  waterTopY: number | null;
  pilgrimRouteInfluence: number;
  surfaceMaterial: number;
  waterMaterial: number | null;
}

export interface ProceduralTopColumnMaterialBucketSample extends ProceduralSurfaceColumnSample {
  bucketIndex: number;
  material: number;
}

export interface ProceduralBiomeProbe extends ProceduralColumnSample {
  regionId: WorldRegionId;
  secondaryRegionId: WorldRegionId;
  regionStrength: number;
  regionAmbientProfileId: AmbientProfileId;
  secondaryBiomeId: BaseBiomeId;
  transitionThreshold: number;
  specialStrength: number;
  regionalVariantStrength: number;
  fields: {
    temperature: number;
    moisture: number;
    uplift: number;
    drainage: number;
    volcanism: number;
    magic: number;
    globalHeight: number;
    mountainness: number;
    oceanness: number;
    ridge: number;
    mesa: number;
    desolation: number;
    strata: number;
    surfacePatch: number;
    surfaceGrain: number;
    scatter: number;
    peakness: number;
    islandInterior: number;
    shorelineBand?: number;
    coastalShelf: number;
    deepOcean?: number;
  };
}

export interface GeneratedChunk {
  coord: ChunkCoordinate;
  data: Uint16Array;
  solidCount: number;
  solidBounds: ChunkBounds | null;
  renderSummary: GeneratedChunkRenderSummary;
}

interface BaseBiomeBlendSelection {
  primary: BaseBiomeProfile;
  secondary: BaseBiomeProfile;
  primaryWeight: number;
}

interface BiomeClassificationSelection {
  biomeId: BiomeId;
  specialStrength: number;
  biomeCore: number;
}

interface ResolvedSurfaceMaterials {
  surfacePrimary: number;
  surfaceSecondary: number;
  subsurfacePrimary: number;
  subsurfaceSecondary: number;
  water: number;
  snow: number;
  transitionThreshold: number;
}

interface ChunkGenerationScratch {
  capacity: number;
  surfaceY: Int32Array;
  waterTopY: Int32Array;
  surfacePrimary: Uint16Array;
  surfaceSecondary: Uint16Array;
  subsurfacePrimary: Uint16Array;
  subsurfaceSecondary: Uint16Array;
  waterMaterial: Uint16Array;
  snowMaterial: Uint16Array;
  stoneMaterial: Uint16Array;
  deepStoneMaterial: Uint16Array;
  accentMaterial: Uint16Array;
  transitionThreshold: Float32Array;
  caveMainField: Float32Array;
  caveMainStrength: Float32Array;
  caveMainCenterY: Int16Array;
  caveMainHalfHeight: Int16Array;
  caveUpperField: Float32Array;
  caveUpperStrength: Float32Array;
  caveUpperCenterY: Int16Array;
  caveUpperHalfHeight: Int16Array;
  caveEntranceField: Float32Array;
  caveEntranceStrength: Float32Array;
  caveEntranceCenterY: Int16Array;
  caveEntranceHalfHeight: Int16Array;
  strataOffset: Float32Array;
  worldXDiv3: Int32Array;
  worldZDiv3: Int32Array;
  ditherSeed: Int32Array;
  accentSeed: Int32Array;
  featureKind: Uint8Array;
  featureHeight: Int16Array;
  featureRadius: Int16Array;
  featureExtra: Int16Array;
  featureDeltaX: Int16Array;
  featureDeltaZ: Int16Array;
  featureMaterialPrimary: Uint16Array;
  featureMaterialSecondary: Uint16Array;
  featureMaterialAccent: Uint16Array;
}

interface RegionalVariantSelection {
  id: RegionalVariantId;
  strength: number;
}

const CONTINENT_SCALE = 1 / 5200;
const UPLIFT_SCALE = 1 / 3000;
const HILLS_SCALE = 1 / 1200;
const DETAIL_SCALE = 1 / 180;
const RIDGE_SCALE = 1 / 640;
const BASIN_SCALE = 1 / 1500;
const DRAINAGE_SCALE = 1 / 1900;
const TEMPERATURE_SCALE = 1 / 4600;
const MOISTURE_SCALE = 1 / 4200;
const VOLCANISM_SCALE = 1 / 3400;
const MAGIC_SCALE = 1 / 5600;
const PEAK_SCALE = 1 / 11800;
const CHANNEL_SCALE = 1 / 1200;
const DUNE_SCALE = 1 / 320;
const MESA_SCALE = 1 / 900;
const GROVE_SCALE = 1 / 2100;
const OLD_GROWTH_SCALE = 1 / 2800;
const ORCHARD_SCALE = 1 / 1700;
const DESOLATION_SCALE = 1 / 2400;
const STRATA_SCALE = 1 / 54;
const SURFACE_PATCH_SCALE = 1 / 48;
const SURFACE_GRAIN_SCALE = 1 / 14;
const SURFACE_SCATTER_SCALE = 1 / 26;
const CAVE_RIBBON_SCALE = 1 / 520;
const CAVE_POCKET_SCALE = 1 / 900;
const CAVE_DEPTH_SCALE = 1 / 1700;
const CAVE_OPENING_SCALE = 1 / 760;
const STRATA_BAND_SCALE = 1 / 160;
const ONE_THIRD = 1 / 3;
const SURFACE_MATERIAL_DITHER_SCALE = 1 / 7;
const WORLD_UNITS_PER_METER = 10;
const NO_WATER = -1;
const FEATURE_NONE = 0;
const FEATURE_OAK = 1;
const FEATURE_STANDING_STONE = 2;
const FEATURE_PALM = 3;
const FEATURE_HOODOO = 4;
const FEATURE_FIR = 5;
const FEATURE_ICE_SPIRE = 6;
const FEATURE_CYPRESS = 7;
const FEATURE_BASALT_SPIRE = 8;
const FEATURE_GLOWCAP = 9;
const FEATURE_BOULDER = 10;
const FEATURE_BUSH = 11;
const FEATURE_CACTUS = 12;
const FEATURE_REEDS = 13;
const FEATURE_CRYSTAL = 14;
const FEATURE_REDWOOD = 15;
const FEATURE_DEAD_TREE = 16;
const FEATURE_MEGASTRUCTURE = 17;
const FEATURE_RIB_ARCH = 18;
const FEATURE_CAUSEWAY = 19;
const FEATURE_ROAD_DEBRIS = 20;
const FEATURE_BURIED_RIBS = 21;
const FEATURE_TRAVEL_PACK = 22;
const CHUNK_GENERATION_SCRATCH_POOL_LIMIT = 4;

interface PilgrimRouteBand {
  startX: number;
  startZ: number;
  directionX: number;
  directionZ: number;
  length: number;
  halfWidth: number;
}

interface PilgrimRouteSurfaceInfluence {
  core: number;
  shoulder: number;
  fracture: number;
  lateralRatio: number;
}

interface PilgrimRouteCoordinates {
  along: number;
  lateral: number;
  lateralRatio: number;
}

interface PilgrimRouteSetPiece {
  profile: LandmarkProfile;
  deltaAlong: number;
  deltaLateral: number;
}

interface GeneratorAtlasFields {
  islandInterior: number;
  shorelineBand: number;
  coastalShelf: number;
  deepOcean: number;
  primaryRegionId: WorldRegionId | null;
  secondaryRegionId: WorldRegionId | null;
  regionStrength: number;
  regionBlend: number;
  regionDistance: number;
  primaryBiomeId: BiomeId | "ocean" | "deep-ocean";
  regionalVariantId: RegionalVariantId | null;
  ambientProfileId: AmbientProfileId | null;
  routeInfluence: number;
  routeCore: number;
  routeShoulder: number;
  distanceToRouteM: number;
  caveInfluence: number;
  caveCore: number;
  distanceToCaveAnchorM: number;
}

const PILGRIM_ROUTE_BANDS: readonly PilgrimRouteBand[] = [
  createPilgrimRouteBand(0, 0, 8, 820, 58),
  createPilgrimRouteBand(0, 0, 126, 1800, 70),
  createPilgrimRouteBand(220, -340, 54, 2200, 64),
  createPilgrimRouteBand(-540, 420, 315, 2200, 64),
  createPilgrimRouteBand(960, -780, 202, 2600, 74),
  createPilgrimRouteBand(236, -4624, 58, 3400, 82),
  createPilgrimRouteBand(-1880, -2860, 42, 3200, 86),
  createPilgrimRouteBand(-6100, 700, 10, 3000, 74),
  createPilgrimRouteBand(-3000, 700, 352, 2250, 76),
  createPilgrimRouteBand(-2100, 3600, 10, 4000, 84),
  createPilgrimRouteBand(-600, -700, 27, 3150, 82),
  createPilgrimRouteBand(900, -1800, 333, 3800, 82),
  createPilgrimRouteBand(3000, 900, 32, 3050, 80),
  createPilgrimRouteBand(-4200, -4200, 37, 3150, 78),
];

const BASE_BIOMES: readonly BaseBiomeProfile[] = [
  createBaseBiome("verdant", 0.56, 0.78, 0.28, 0.74, 0.42, 0.18, -10, 0.48, 0.18, 0.40, 0.28, 0.00, 4.4, 1548, "#6A5", "#7B6", "#8B6", "#592", "#677", "#754", "#865", "#49B", "#DDE"),
  createBaseBiome("savanna", 0.72, 0.54, 0.32, 0.56, 0.46, 0.16, -2, 0.50, 0.20, 0.36, 0.18, 0.00, 6.4, 1640, "#BA6", "#CB7", "#DB8", "#C86", "#887", "#986", "#A97", "#5AB", "#EED"),
  createBaseBiome("steppe", 0.62, 0.42, 0.36, 0.52, 0.48, 0.14, 0, 0.54, 0.22, 0.32, 0.18, 0.00, 4.8, 1608, "#9B6", "#CB7", "#BA6", "#CA7", "#887", "#875", "#986", "#4AA", "#DDD"),
  createBaseBiome("dunes", 0.84, 0.16, 0.18, 0.28, 0.30, 0.12, -16, 0.32, 0.10, 0.54, 0.42, 0.00, 8.8, 1710, "#DB6", "#EC9", "#EC7", "#CA5", "#B96", "#B85", "#C96", "#5BC", "#EDC"),
  createBaseBiome("badlands", 0.72, 0.20, 0.58, 0.36, 0.58, 0.16, 18, 0.72, 0.64, 0.38, 0.06, 0.28, 10.8, 1670, "#A65", "#B87", "#A76", "#854", "#654", "#743", "#854", "#386", "#CAB"),
  createBaseBiome("highland", 0.40, 0.56, 0.72, 0.46, 0.72, 0.16, 44, 0.88, 0.62, 0.24, 0.10, 0.06, 7.8, 1518, "#6B7", "#7C8", "#7A8", "#8C7", "#778", "#667", "#889", "#5AD", "#EEF"),
  createBaseBiome("moor", 0.28, 0.68, 0.48, 0.28, 0.54, 0.16, 6, 0.34, 0.16, 0.22, 0.30, 0.00, 5.4, 1532, "#758", "#869", "#97A", "#546", "#667", "#564", "#675", "#357", "#DDE"),
  createBaseBiome("tundra", 0.18, 0.42, 0.86, 0.40, 0.82, 0.12, 78, 0.98, 0.82, 0.16, 0.02, 0.04, 6.2, 1452, "#BCC", "#CDD", "#DDE", "#ABB", "#889", "#99A", "#AAB", "#8CD", "#EEF"),
] as const;
const BASE_BIOMES_BY_ID = new Map<BaseBiomeId, BaseBiomeProfile>(
  BASE_BIOMES.map((biome) => [biome.id, biome]),
);

const SPECIAL_BIOMES: Record<SpecialBiomeId, SpecialBiomeProfile> = {
  marsh: createSpecialBiome("marsh", "#486", "#5A8", "#597", "#2A6", "#576", "#564", "#675", "#276", "#DDE", true),
  firefly: createSpecialBiome("firefly", "#465", "#576", "#6A6", "#FC8", "#566", "#354", "#465", "#245", "#DDE", true),
  saltflat: createSpecialBiome("saltflat", "#EED", "#FDC", "#DEF", "#BBA", "#CBA", "#BBA", "#CCB", "#8CD", "#FFF", true),
  fern: createSpecialBiome("fern", "#7C5", "#8D6", "#9D7", "#6A4", "#677", "#675", "#786", "#4AB", "#EEF", true),
  fungal: createSpecialBiome("fungal", "#576", "#6A8", "#7AB", "#9CF", "#556", "#445", "#667", "#47A", "#DFF", true),
  ember: createSpecialBiome("ember", "#543", "#754", "#764", "#F74", "#433", "#654", "#765", "#36A", "#DCC", false),
  bloom: createSpecialBiome("bloom", "#6A8", "#8CF", "#7BA", "#BDF", "#668", "#557", "#668", "#4CF", "#EEF", true),
  shardlands: createSpecialBiome("shardlands", "#BCA", "#CED", "#DFF", "#A7C", "#889", "#667", "#88A", "#6BE", "#FFF", false),
};

const PROCEDURAL_WATER_ALPHA = 168;
const PROCEDURAL_WATER_MATERIALS = new Set<number>([
  ...BASE_BIOMES.map((biome) => biome.water),
  ...Object.values(SPECIAL_BIOMES).map((biome) => biome.water),
  "#134",
  "#245",
  "#58C",
  "#5BD",
  "#6DF",
  "#9CF",
  "#9DF",
].map((material) => typeof material === "number" ? material : hexColorToMaterial(material)));

const PILGRIM_ROUTE_CORE_MATERIAL = hexColorToMaterial("#655");
const PILGRIM_ROUTE_WORN_MATERIAL = hexColorToMaterial("#887");
const PILGRIM_ROUTE_DARK_MATERIAL = hexColorToMaterial("#433");
const PILGRIM_ROUTE_DUST_MATERIAL = hexColorToMaterial("#544");
const PILGRIM_ROUTE_SALT_MATERIAL = hexColorToMaterial("#BBA");
const PILGRIM_ROUTE_WARP_MAX = 18 * WORLD_UNITS_PER_METER;
const PILGRIM_ROUTE_WARP_FADE_DISTANCE = 140 * WORLD_UNITS_PER_METER;
const PILGRIM_ROUTE_SET_PIECE_START = 180 * WORLD_UNITS_PER_METER;
const PILGRIM_ROUTE_SET_PIECE_SPACING = 240 * WORLD_UNITS_PER_METER;
const PILGRIM_ROUTE_SET_PIECE_END_MARGIN = 140 * WORLD_UNITS_PER_METER;

const UNDERGROUND_BIOMES: Record<UndergroundBiomeId, UndergroundBiomeProfile> = {
  rooted: createUndergroundBiome("rooted", "#586", "#354", "#9C6"),
  sedimentary: createUndergroundBiome("sedimentary", "#866", "#644", "#DA7"),
  sandy: createUndergroundBiome("sandy", "#977", "#655", "#EDC"),
  granitic: createUndergroundBiome("granitic", "#889", "#556", "#BDE"),
  froststone: createUndergroundBiome("froststone", "#9AB", "#667", "#DFF"),
  basaltic: createUndergroundBiome("basaltic", "#544", "#322", "#F74"),
  peaty: createUndergroundBiome("peaty", "#564", "#342", "#8A6"),
  saline: createUndergroundBiome("saline", "#BBA", "#887", "#EED"),
  mycelial: createUndergroundBiome("mycelial", "#576", "#354", "#8CF"),
  crystalline: createUndergroundBiome("crystalline", "#789", "#567", "#CEF"),
};

const LANDMARKS: Record<LandmarkId, LandmarkProfile> = {
  oak: createLandmark("oak", 176, 11, 0.34, 1.0, 0),
  canopy_tree: createLandmark("canopy_tree", 224, 16, 0.20, 1.2, 2),
  birch: createLandmark("birch", 164, 9, 0.30, 1.0, 1),
  redleaf_tree: createLandmark("redleaf_tree", 184, 12, 0.22, 1.0, 9),
  willow: createLandmark("willow", 188, 14, 0.22, 1.0, 0),
  blossom_tree: createLandmark("blossom_tree", 168, 11, 0.24, 1.0, 7),
  fruit_tree: createLandmark("fruit_tree", 160, 10, 0.20, 1.0, 8),
  giant_flower: createLandmark("giant_flower", 148, 9, 0.18, 1.0, 1),
  redwood: createLandmark("redwood", 272, 20, 0.18, 1.0, 0),
  dead_tree: createLandmark("dead_tree", 172, 8, 0.20, 1.0, 0),
  thorn_tree: createLandmark("thorn_tree", 176, 9, 0.18, 1.0, 1),
  berry_bush: createLandmark("berry_bush", 88, 5, 0.42, 1.0, 5),
  giant_fern: createLandmark("giant_fern", 176, 12, 0.24, 1.0, 0),
  lantern_tree: createLandmark("lantern_tree", 180, 11, 0.22, 1.0, 10),
  salt_spire: createLandmark("salt_spire", 164, 6, 0.26, 1.0, 0),
  boulder: createLandmark("boulder", 120, 5, 0.34, 1.0, 0),
  standing_stone: createLandmark("standing_stone", 160, 5, 0.24, 1.0, 0),
  shrub: createLandmark("shrub", 92, 4, 0.42, 1.0, 0),
  flower_patch: createLandmark("flower_patch", 76, 5, 0.50, 1.0, 0),
  palm: createLandmark("palm", 184, 12, 0.26, 1.0, 0),
  acacia: createLandmark("acacia", 168, 12, 0.28, 1.0, 3),
  cactus: createLandmark("cactus", 128, 4, 0.34, 1.0, 1),
  dead_snag: createLandmark("dead_snag", 156, 4, 0.24, 1.0, 0),
  hoodoo: createLandmark("hoodoo", 184, 8, 0.24, 1.0, 0),
  fir: createLandmark("fir", 156, 9, 0.28, 1.0, 0),
  tall_fir: createLandmark("tall_fir", 208, 12, 0.18, 1.15, 0),
  ice_spire: createLandmark("ice_spire", 184, 8, 0.22, 1.0, 0),
  frost_shrub: createLandmark("frost_shrub", 104, 4, 0.44, 1.0, 0),
  cypress: createLandmark("cypress", 164, 10, 0.34, 1.0, 0),
  mangrove: createLandmark("mangrove", 208, 14, 0.24, 1.1, 0),
  reed_cluster: createLandmark("reed_cluster", 92, 4, 0.54, 1.0, 1),
  basalt_spire: createLandmark("basalt_spire", 184, 7, 0.18, 1.0, 0),
  crystal_cluster: createLandmark("crystal_cluster", 132, 5, 0.28, 1.0, 1),
  glowcap: createLandmark("glowcap", 164, 12, 0.30, 1.0, 2),
  mega_glowcap: createLandmark("mega_glowcap", 232, 18, 0.16, 1.2, 2),
  root_stump: createLandmark("root_stump", 132, 7, 0.22, 1.0, 0),
  stone_tor: createLandmark("stone_tor", 176, 7, 0.18, 1.0, 0),
  ancestor_pillar: createLandmark("ancestor_pillar", 192, 6, 0.18, 1.0, 0),
  ash_marker: createLandmark("ash_marker", 160, 5, 0.20, 1.0, 0),
  glass_cairn: createLandmark("glass_cairn", 128, 5, 0.24, 1.0, 2),
  silt_shell: createLandmark("silt_shell", 176, 8, 0.18, 1.0, 1),
  velothi_shrine: createLandmark("velothi_shrine", 164, 5, 0.18, 1.0, 1),
  kwama_mound: createLandmark("kwama_mound", 132, 6, 0.26, 1.0, 1),
  pilgrim_cairn: createLandmark("pilgrim_cairn", 144, 5, 0.22, 1.0, 1),
  velothi_ziggurat: createLandmark("velothi_ziggurat", 292, 15, 0.14, 1.0, 1),
  ash_obelisk: createLandmark("ash_obelisk", 212, 8, 0.18, 1.0, 2),
  rib_arch: createLandmark("rib_arch", 196, 13, 0.18, 1.0, 1),
  old_road_causeway: createLandmark("old_road_causeway", 128, 13, 0.30, 1.0, 1),
  paver_debris: createLandmark("paver_debris", 96, 12, 0.38, 1.0, 0),
  scree_fan: createLandmark("scree_fan", 104, 13, 0.34, 1.0, 1),
  shrine_debris: createLandmark("shrine_debris", 124, 15, 0.28, 1.0, 2),
  buried_ribs: createLandmark("buried_ribs", 144, 12, 0.24, 1.0, 3),
  pilgrim_lantern: createLandmark("pilgrim_lantern", 124, 5, 0.28, 1.0, 4),
  bone_chimes: createLandmark("bone_chimes", 156, 7, 0.24, 1.0, 5),
  ashlander_travel_pack: createLandmark("ashlander_travel_pack", 132, 7, 0.14, 1.0, 1),
  crystal_reeds: createLandmark("crystal_reeds", 112, 5, 0.34, 1.0, 2),
  fungal_bridge: createLandmark("fungal_bridge", 156, 16, 0.26, 1.0, 1),
  rib_remains: createLandmark("rib_remains", 152, 10, 0.24, 1.0, 1),
};

const BASE_BIOME_LANDMARKS: Record<BaseBiomeId, readonly LandmarkProfile[]> = {
  verdant: [
    landmarkPlacement("canopy_tree", { chance: 0.24, scale: 1.26 }),
    landmarkPlacement("oak", { scale: 1.14, chance: 0.34 }),
    landmarkPlacement("birch", { chance: 0.28, scale: 0.94 }),
    landmarkPlacement("flower_patch", { chance: 0.34, scale: 1.08, variant: 1 }),
    landmarkPlacement("shrub", { chance: 0.30, scale: 1.12 }),
    landmarkPlacement("boulder", { chance: 0.20, scale: 0.92 }),
  ],
  savanna: [
    landmarkPlacement("acacia", { chance: 0.36, scale: 1.14 }),
    landmarkPlacement("thorn_tree", { chance: 0.30, scale: 1.06, cellSize: 148, radius: 10 }),
    landmarkPlacement("flower_patch", { chance: 0.48, scale: 1.16, variant: 2, cellSize: 68, radius: 5 }),
    landmarkPlacement("fruit_tree", { chance: 0.18, scale: 0.98, cellSize: 156, radius: 11 }),
    landmarkPlacement("standing_stone", { chance: 0.24, scale: 1.10 }),
    landmarkPlacement("boulder", { chance: 0.16, scale: 0.92 }),
  ],
  steppe: [
    landmarkPlacement("acacia", { chance: 0.30, scale: 1.12 }),
    landmarkPlacement("dead_snag", { chance: 0.24, scale: 1.08 }),
    landmarkPlacement("shrub", { chance: 0.24, scale: 1.0, variant: 2 }),
    landmarkPlacement("flower_patch", { chance: 0.16, scale: 0.88, variant: 2 }),
    landmarkPlacement("standing_stone", { chance: 0.28, scale: 1.12 }),
    landmarkPlacement("ancestor_pillar", { chance: 0.14, scale: 1.10, cellSize: 176, radius: 6 }),
    landmarkPlacement("pilgrim_cairn", { chance: 0.12, scale: 1.06, cellSize: 168, radius: 5 }),
    landmarkPlacement("old_road_causeway", { chance: 0.16, scale: 1.04, cellSize: 152, radius: 12 }),
    landmarkPlacement("pilgrim_lantern", { chance: 0.12, scale: 1.04, cellSize: 148, radius: 5 }),
    landmarkPlacement("boulder", { chance: 0.18, scale: 0.9 }),
  ],
  dunes: [
    landmarkPlacement("palm", { chance: 0.34, scale: 1.22 }),
    landmarkPlacement("cactus", { chance: 0.36, scale: 1.12 }),
    landmarkPlacement("cactus", { chance: 0.24, scale: 1.65, variant: 2, cellSize: 168, radius: 6 }),
    landmarkPlacement("silt_shell", { chance: 0.14, scale: 1.04, cellSize: 188, radius: 8 }),
    landmarkPlacement("kwama_mound", { chance: 0.12, scale: 1.06, cellSize: 172, radius: 6 }),
    landmarkPlacement("dead_snag", { chance: 0.18, scale: 0.9 }),
    landmarkPlacement("boulder", { chance: 0.16, scale: 0.86 }),
  ],
  badlands: [
    landmarkPlacement("hoodoo", { chance: 0.30, scale: 1.22 }),
    landmarkPlacement("hoodoo", { chance: 0.24, scale: 0.86, variant: 1, cellSize: 156, radius: 6 }),
    landmarkPlacement("dead_snag", { chance: 0.28, scale: 1.24 }),
    landmarkPlacement("standing_stone", { chance: 0.20, scale: 1.18 }),
    landmarkPlacement("velothi_ziggurat", { chance: 0.08, scale: 1.08, cellSize: 280, radius: 15 }),
    landmarkPlacement("ash_obelisk", { chance: 0.12, scale: 1.10, cellSize: 204, radius: 8 }),
    landmarkPlacement("rib_arch", { chance: 0.14, scale: 1.08, cellSize: 184, radius: 13 }),
    landmarkPlacement("old_road_causeway", { chance: 0.22, scale: 1.06, cellSize: 132, radius: 13 }),
    landmarkPlacement("pilgrim_lantern", { chance: 0.16, scale: 1.10, cellSize: 128, radius: 5 }),
    landmarkPlacement("ash_marker", { chance: 0.16, scale: 1.12, cellSize: 168, radius: 5 }),
    landmarkPlacement("velothi_shrine", { chance: 0.14, scale: 1.12, cellSize: 184, radius: 5 }),
    landmarkPlacement("kwama_mound", { chance: 0.14, scale: 1.10, cellSize: 164, radius: 6 }),
    landmarkPlacement("pilgrim_cairn", { chance: 0.12, scale: 1.08, cellSize: 172, radius: 5 }),
    landmarkPlacement("cactus", { chance: 0.18, scale: 0.96 }),
    landmarkPlacement("boulder", { chance: 0.20, scale: 0.94 }),
  ],
  highland: [
    landmarkPlacement("tall_fir", { chance: 0.24, scale: 1.22 }),
    landmarkPlacement("fir", { chance: 0.32, scale: 1.08 }),
    landmarkPlacement("standing_stone", { chance: 0.20, scale: 1.26 }),
    landmarkPlacement("crystal_cluster", { chance: 0.14, scale: 1.08, variant: 2 }),
    landmarkPlacement("boulder", { chance: 0.28, scale: 1.08 }),
  ],
  moor: [
    landmarkPlacement("dead_tree", { chance: 0.28, scale: 1.10, cellSize: 148, radius: 8 }),
    landmarkPlacement("willow", { chance: 0.24, scale: 0.98, cellSize: 148, radius: 14 }),
    landmarkPlacement("frost_shrub", { chance: 0.40, scale: 1.04 }),
    landmarkPlacement("standing_stone", { chance: 0.32, scale: 1.18 }),
    landmarkPlacement("ancestor_pillar", { chance: 0.16, scale: 1.08, cellSize: 172, radius: 6 }),
    landmarkPlacement("glowcap", { chance: 0.14, scale: 0.96, cellSize: 156, radius: 12 }),
    landmarkPlacement("boulder", { chance: 0.22, scale: 1.00 }),
  ],
  tundra: [
    landmarkPlacement("ice_spire", { chance: 0.30, scale: 1.20 }),
    landmarkPlacement("tall_fir", { chance: 0.20, scale: 1.06 }),
    landmarkPlacement("frost_shrub", { chance: 0.52, scale: 1.04 }),
    landmarkPlacement("boulder", { chance: 0.26, scale: 1.04 }),
  ],
};

const SPECIAL_BIOME_LANDMARKS: Record<SpecialBiomeId, readonly LandmarkProfile[]> = {
  marsh: [
    landmarkPlacement("mangrove", { chance: 0.32, scale: 1.14 }),
    landmarkPlacement("cypress", { chance: 0.28, scale: 1.08 }),
    landmarkPlacement("reed_cluster", { chance: 0.62, scale: 1.1 }),
    landmarkPlacement("flower_patch", { chance: 0.24, scale: 0.92, variant: 3 }),
  ],
  firefly: [
    landmarkPlacement("lantern_tree", { chance: 0.34, scale: 1.10, cellSize: 136, radius: 12 }),
    landmarkPlacement("willow", { chance: 0.24, scale: 1.00, cellSize: 144, radius: 14 }),
    landmarkPlacement("reed_cluster", { chance: 0.76, scale: 1.14, cellSize: 64, radius: 4 }),
    landmarkPlacement("glowcap", { chance: 0.34, scale: 1.08, cellSize: 132, radius: 12 }),
    landmarkPlacement("flower_patch", { chance: 0.26, scale: 0.98, variant: 3, cellSize: 76, radius: 5 }),
  ],
  saltflat: [
    landmarkPlacement("salt_spire", { chance: 0.36, scale: 1.10 }),
    landmarkPlacement("crystal_cluster", { chance: 0.20, scale: 1.04, variant: 2 }),
    landmarkPlacement("glass_cairn", { chance: 0.16, scale: 1.08, cellSize: 152, radius: 5 }),
    landmarkPlacement("silt_shell", { chance: 0.16, scale: 1.06, cellSize: 184, radius: 8 }),
    landmarkPlacement("kwama_mound", { chance: 0.12, scale: 1.04, cellSize: 176, radius: 6 }),
    landmarkPlacement("dead_snag", { chance: 0.12, scale: 0.90 }),
    landmarkPlacement("shrub", { chance: 0.12, scale: 0.82, variant: 2 }),
  ],
  fern: [
    landmarkPlacement("giant_fern", { chance: 0.40, scale: 1.18, cellSize: 128, radius: 13 }),
    landmarkPlacement("canopy_tree", { chance: 0.30, scale: 1.18, cellSize: 132, radius: 16 }),
    landmarkPlacement("berry_bush", { chance: 0.48, scale: 1.08, cellSize: 72, radius: 5 }),
    landmarkPlacement("flower_patch", { chance: 0.28, scale: 1.04, variant: 1, cellSize: 72, radius: 5 }),
    landmarkPlacement("boulder", { chance: 0.14, scale: 0.92 }),
  ],
  fungal: [
    landmarkPlacement("mega_glowcap", { chance: 0.24, scale: 1.16 }),
    landmarkPlacement("glowcap", { chance: 0.40, scale: 1.08 }),
    landmarkPlacement("lantern_tree", { chance: 0.22, scale: 1.00, cellSize: 144, radius: 12 }),
    landmarkPlacement("giant_flower", { chance: 0.18, scale: 1.06, cellSize: 132, radius: 10 }),
    landmarkPlacement("berry_bush", { chance: 0.28, scale: 1.00, cellSize: 80, radius: 5 }),
  ],
  ember: [
    landmarkPlacement("basalt_spire", { chance: 0.30, scale: 1.28 }),
    landmarkPlacement("ash_marker", { chance: 0.22, scale: 1.20, cellSize: 152, radius: 5 }),
    landmarkPlacement("kwama_mound", { chance: 0.14, scale: 1.12, cellSize: 164, radius: 6 }),
    landmarkPlacement("pilgrim_cairn", { chance: 0.12, scale: 1.10, cellSize: 168, radius: 5 }),
    landmarkPlacement("ashlander_travel_pack", { chance: 0.16, scale: 1.04, cellSize: 148, radius: 7 }),
    landmarkPlacement("crystal_cluster", { chance: 0.28, scale: 1.12, variant: 3 }),
    landmarkPlacement("dead_snag", { chance: 0.26, scale: 1.16, variant: 1 }),
    landmarkPlacement("boulder", { chance: 0.24, scale: 0.94, variant: 1 }),
  ],
  bloom: [
    landmarkPlacement("mega_glowcap", { chance: 0.24, scale: 1.18 }),
    landmarkPlacement("giant_flower", { chance: 0.20, scale: 1.12, cellSize: 136, radius: 10, variant: 1 }),
    landmarkPlacement("glowcap", { chance: 0.32, scale: 1.08 }),
    landmarkPlacement("crystal_cluster", { chance: 0.20, scale: 1.08, variant: 2 }),
    landmarkPlacement("flower_patch", { chance: 0.28, scale: 1.08, variant: 4 }),
    landmarkPlacement("canopy_tree", { chance: 0.16, scale: 1.12, variant: 4 }),
  ],
  shardlands: [
    landmarkPlacement("salt_spire", { chance: 0.34, scale: 1.12, variant: 1 }),
    landmarkPlacement("crystal_cluster", { chance: 0.38, scale: 1.16, variant: 2 }),
    landmarkPlacement("glass_cairn", { chance: 0.20, scale: 1.12, cellSize: 144, radius: 5, variant: 2 }),
    landmarkPlacement("velothi_shrine", { chance: 0.16, scale: 1.14, cellSize: 176, radius: 5 }),
    landmarkPlacement("pilgrim_cairn", { chance: 0.14, scale: 1.10, cellSize: 164, radius: 5 }),
    landmarkPlacement("hoodoo", { chance: 0.20, scale: 1.02, variant: 1 }),
    landmarkPlacement("dead_tree", { chance: 0.18, scale: 1.02, cellSize: 156, radius: 8 }),
  ],
};

const VERDANT_GROVE_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("redwood", { chance: 0.34, scale: 1.08, cellSize: 196, radius: 20 }),
  landmarkPlacement("canopy_tree", { chance: 0.82, scale: 1.34, cellSize: 92, radius: 18 }),
  landmarkPlacement("oak", { chance: 0.74, scale: 1.20, cellSize: 84, radius: 12 }),
  landmarkPlacement("willow", { chance: 0.22, scale: 1.06, cellSize: 156, radius: 14 }),
  landmarkPlacement("berry_bush", { chance: 0.64, scale: 1.10, cellSize: 68, radius: 5 }),
  landmarkPlacement("flower_patch", { chance: 0.44, scale: 1.12, variant: 1, cellSize: 72, radius: 5 }),
];

const VERDANT_OLD_GROWTH_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("redwood", { chance: 0.42, scale: 1.20, cellSize: 176, radius: 24 }),
  landmarkPlacement("canopy_tree", { chance: 0.92, scale: 1.40, cellSize: 88, radius: 18 }),
  landmarkPlacement("oak", { chance: 0.86, scale: 1.26, cellSize: 76, radius: 13 }),
  landmarkPlacement("willow", { chance: 0.34, scale: 1.12, cellSize: 136, radius: 15 }),
  landmarkPlacement("berry_bush", { chance: 0.74, scale: 1.14, cellSize: 64, radius: 5 }),
  landmarkPlacement("flower_patch", { chance: 0.52, scale: 1.12, variant: 1, cellSize: 68, radius: 5 }),
];

const VERDANT_CANOPY_SEA_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("redwood", { chance: 0.48, scale: 1.28, cellSize: 156, radius: 26 }),
  landmarkPlacement("canopy_tree", { chance: 0.96, scale: 1.48, cellSize: 72, radius: 20 }),
  landmarkPlacement("oak", { chance: 0.90, scale: 1.32, cellSize: 68, radius: 13 }),
  landmarkPlacement("willow", { chance: 0.28, scale: 1.08, cellSize: 124, radius: 16 }),
  landmarkPlacement("root_stump", { chance: 0.18, scale: 1.08, cellSize: 136, radius: 9 }),
  landmarkPlacement("berry_bush", { chance: 0.76, scale: 1.12, cellSize: 60, radius: 5 }),
];

const VERDANT_KARST_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("willow", { chance: 0.26, scale: 1.04, cellSize: 148, radius: 14 }),
  landmarkPlacement("birch", { chance: 0.42, scale: 1.04, cellSize: 116, radius: 9 }),
  landmarkPlacement("standing_stone", { chance: 0.34, scale: 1.18 }),
  landmarkPlacement("boulder", { chance: 0.30, scale: 1.08 }),
  landmarkPlacement("flower_patch", { chance: 0.24, scale: 1.06, variant: 1, cellSize: 84, radius: 5 }),
];

const VERDANT_ORCHARD_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("blossom_tree", { chance: 0.48, scale: 1.12, cellSize: 108, radius: 12 }),
  landmarkPlacement("fruit_tree", { chance: 0.44, scale: 1.10, cellSize: 104, radius: 11 }),
  landmarkPlacement("berry_bush", { chance: 0.64, scale: 1.12, cellSize: 72, radius: 5 }),
  landmarkPlacement("flower_patch", { chance: 0.56, scale: 1.18, variant: 4, cellSize: 68, radius: 5 }),
  landmarkPlacement("shrub", { chance: 0.34, scale: 1.06, cellSize: 84, radius: 4 }),
];

const SAVANNA_WILDFLOWER_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("acacia", { chance: 0.40, scale: 1.16, cellSize: 136, radius: 12 }),
  landmarkPlacement("thorn_tree", { chance: 0.30, scale: 1.08, cellSize: 140, radius: 10 }),
  landmarkPlacement("flower_patch", { chance: 0.78, scale: 1.24, variant: 2, cellSize: 52, radius: 6 }),
  landmarkPlacement("fruit_tree", { chance: 0.22, scale: 1.00, cellSize: 148, radius: 11 }),
  landmarkPlacement("berry_bush", { chance: 0.34, scale: 1.04, cellSize: 72, radius: 5 }),
];

const STEPPE_ORCHARD_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("fruit_tree", { chance: 0.40, scale: 1.02, cellSize: 112, radius: 11 }),
  landmarkPlacement("blossom_tree", { chance: 0.34, scale: 1.00, cellSize: 116, radius: 11 }),
  landmarkPlacement("acacia", { chance: 0.24, scale: 1.06, cellSize: 144, radius: 12 }),
  landmarkPlacement("flower_patch", { chance: 0.54, scale: 1.08, variant: 2, cellSize: 72, radius: 5 }),
  landmarkPlacement("berry_bush", { chance: 0.44, scale: 1.04, cellSize: 80, radius: 5 }),
];

const STEPPE_DESOLATE_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("dead_tree", { chance: 0.34, scale: 1.18, cellSize: 132, radius: 8 }),
  landmarkPlacement("dead_snag", { chance: 0.28, scale: 1.14, cellSize: 140, radius: 4 }),
  landmarkPlacement("standing_stone", { chance: 0.26, scale: 1.18 }),
  landmarkPlacement("shrub", { chance: 0.18, scale: 0.94, variant: 2 }),
  landmarkPlacement("boulder", { chance: 0.16, scale: 0.92 }),
];

const STEPPE_THORN_SCRUB_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("thorn_tree", { chance: 0.34, scale: 1.10, cellSize: 132, radius: 10 }),
  landmarkPlacement("acacia", { chance: 0.28, scale: 1.08, cellSize: 144, radius: 12 }),
  landmarkPlacement("berry_bush", { chance: 0.36, scale: 1.02, cellSize: 82, radius: 5 }),
  landmarkPlacement("flower_patch", { chance: 0.20, scale: 0.96, variant: 2, cellSize: 76, radius: 5 }),
  landmarkPlacement("standing_stone", { chance: 0.20, scale: 1.12 }),
];

const STEPPE_MONOLITH_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("standing_stone", { chance: 0.44, scale: 1.28, cellSize: 132, radius: 6 }),
  landmarkPlacement("ancestor_pillar", { chance: 0.30, scale: 1.22, cellSize: 148, radius: 6 }),
  landmarkPlacement("thorn_tree", { chance: 0.28, scale: 1.12, cellSize: 140, radius: 10 }),
  landmarkPlacement("acacia", { chance: 0.26, scale: 1.08, cellSize: 152, radius: 12 }),
  landmarkPlacement("flower_patch", { chance: 0.22, scale: 0.92, variant: 2, cellSize: 76, radius: 5 }),
  landmarkPlacement("boulder", { chance: 0.18, scale: 0.96 }),
];

const PILGRIM_ROUTE_SKYLINE_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("paver_debris", { chance: 0.60, scale: 1.24, cellSize: 70, radius: 15 }),
  landmarkPlacement("scree_fan", { chance: 0.46, scale: 1.18, cellSize: 84, radius: 14 }),
  landmarkPlacement("shrine_debris", { chance: 0.34, scale: 1.14, cellSize: 112, radius: 15 }),
  landmarkPlacement("buried_ribs", { chance: 0.30, scale: 1.18, cellSize: 132, radius: 13 }),
  landmarkPlacement("old_road_causeway", { chance: 0.46, scale: 1.20, cellSize: 104, radius: 14 }),
  landmarkPlacement("pilgrim_lantern", { chance: 0.40, scale: 1.22, cellSize: 104, radius: 5 }),
  landmarkPlacement("bone_chimes", { chance: 0.34, scale: 1.18, cellSize: 124, radius: 7 }),
  landmarkPlacement("ash_obelisk", { chance: 0.30, scale: 1.30, cellSize: 164, radius: 8 }),
  landmarkPlacement("rib_arch", { chance: 0.24, scale: 1.24, cellSize: 164, radius: 14 }),
  landmarkPlacement("ancestor_pillar", { chance: 0.26, scale: 1.22, cellSize: 140, radius: 6 }),
  landmarkPlacement("basalt_spire", { chance: 0.24, scale: 1.24, cellSize: 144, radius: 7 }),
  landmarkPlacement("standing_stone", { chance: 0.32, scale: 1.24, cellSize: 128, radius: 6 }),
  landmarkPlacement("dead_tree", { chance: 0.18, scale: 1.16, cellSize: 156, radius: 8 }),
];

const PILGRIM_ROUTE_WETLAND_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("fungal_bridge", { chance: 0.48, scale: 1.30, cellSize: 112, radius: 17 }),
  landmarkPlacement("rib_remains", { chance: 0.38, scale: 1.22, cellSize: 126, radius: 11 }),
  landmarkPlacement("crystal_reeds", { chance: 0.54, scale: 1.28, cellSize: 72, radius: 6 }),
  landmarkPlacement("mega_glowcap", { chance: 0.34, scale: 1.34, cellSize: 118, radius: 20 }),
  landmarkPlacement("cypress", { chance: 0.44, scale: 1.26, cellSize: 104, radius: 11 }),
  landmarkPlacement("willow", { chance: 0.34, scale: 1.18, cellSize: 126, radius: 14 }),
  landmarkPlacement("rib_arch", { chance: 0.34, scale: 1.34, cellSize: 138, radius: 15 }),
  landmarkPlacement("pilgrim_lantern", { chance: 0.36, scale: 1.24, cellSize: 100, radius: 5 }),
];

const PILGRIM_ROUTE_SALT_BASIN_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("salt_spire", { chance: 0.58, scale: 1.82, cellSize: 78, radius: 8, variant: 2 }),
  landmarkPlacement("glass_cairn", { chance: 0.42, scale: 1.34, cellSize: 104, radius: 6, variant: 2 }),
  landmarkPlacement("crystal_cluster", { chance: 0.34, scale: 1.28, cellSize: 112, radius: 8, variant: 2 }),
  landmarkPlacement("ash_obelisk", { chance: 0.24, scale: 1.22, cellSize: 148, radius: 8 }),
  landmarkPlacement("old_road_causeway", { chance: 0.44, scale: 1.22, cellSize: 104, radius: 15 }),
  landmarkPlacement("pilgrim_lantern", { chance: 0.40, scale: 1.28, cellSize: 96, radius: 5 }),
  landmarkPlacement("rib_arch", { chance: 0.26, scale: 1.30, cellSize: 148, radius: 15 }),
  landmarkPlacement("standing_stone", { chance: 0.24, scale: 1.20, cellSize: 128, radius: 6 }),
];

const PILGRIM_ROUTE_GLASS_SHARD_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("glass_cairn", { chance: 0.58, scale: 1.38, cellSize: 88, radius: 6, variant: 2 }),
  landmarkPlacement("crystal_cluster", { chance: 0.52, scale: 1.36, cellSize: 92, radius: 8, variant: 2 }),
  landmarkPlacement("salt_spire", { chance: 0.42, scale: 1.42, cellSize: 110, radius: 7, variant: 2 }),
  landmarkPlacement("velothi_shrine", { chance: 0.26, scale: 1.24, cellSize: 148, radius: 6 }),
  landmarkPlacement("pilgrim_cairn", { chance: 0.36, scale: 1.18, cellSize: 112, radius: 5 }),
  landmarkPlacement("ash_obelisk", { chance: 0.22, scale: 1.24, cellSize: 176, radius: 8 }),
  landmarkPlacement("basalt_spire", { chance: 0.26, scale: 1.26, cellSize: 140, radius: 7 }),
];

const PILGRIM_ROUTE_GRAZELANDS_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("acacia", { chance: 0.44, scale: 1.24, cellSize: 112, radius: 13 }),
  landmarkPlacement("standing_stone", { chance: 0.46, scale: 1.48, cellSize: 96, radius: 7 }),
  landmarkPlacement("ancestor_pillar", { chance: 0.38, scale: 1.44, cellSize: 112, radius: 7 }),
  landmarkPlacement("ash_obelisk", { chance: 0.20, scale: 1.18, cellSize: 168, radius: 8 }),
  landmarkPlacement("thorn_tree", { chance: 0.32, scale: 1.16, cellSize: 128, radius: 10 }),
  landmarkPlacement("flower_patch", { chance: 0.42, scale: 1.12, variant: 2, cellSize: 70, radius: 6 }),
  landmarkPlacement("pilgrim_cairn", { chance: 0.28, scale: 1.16, cellSize: 118, radius: 5 }),
];

const PILGRIM_ROUTE_WEST_GASH_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("stone_tor", { chance: 0.46, scale: 1.34, cellSize: 102, radius: 9 }),
  landmarkPlacement("redleaf_tree", { chance: 0.56, scale: 1.28, cellSize: 84, radius: 13 }),
  landmarkPlacement("old_road_causeway", { chance: 0.36, scale: 1.20, cellSize: 104, radius: 15 }),
  landmarkPlacement("pilgrim_lantern", { chance: 0.34, scale: 1.26, cellSize: 94, radius: 5 }),
  landmarkPlacement("standing_stone", { chance: 0.30, scale: 1.26, cellSize: 118, radius: 6 }),
  landmarkPlacement("rib_arch", { chance: 0.22, scale: 1.20, cellSize: 158, radius: 14 }),
  landmarkPlacement("boulder", { chance: 0.30, scale: 1.10, cellSize: 96, radius: 6 }),
];

const DUNES_GLASS_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("crystal_cluster", { chance: 0.36, scale: 1.18, variant: 2 }),
  landmarkPlacement("glass_cairn", { chance: 0.28, scale: 1.14, cellSize: 136, radius: 5, variant: 2 }),
  landmarkPlacement("palm", { chance: 0.28, scale: 1.18, cellSize: 176, radius: 13 }),
  landmarkPlacement("cactus", { chance: 0.34, scale: 1.08 }),
  landmarkPlacement("standing_stone", { chance: 0.18, scale: 1.08 }),
  landmarkPlacement("boulder", { chance: 0.12, scale: 0.90 }),
];

const BADLANDS_DESOLATE_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("dead_tree", { chance: 0.34, scale: 1.28, cellSize: 148, radius: 8 }),
  landmarkPlacement("hoodoo", { chance: 0.28, scale: 1.18 }),
  landmarkPlacement("standing_stone", { chance: 0.22, scale: 1.20 }),
  landmarkPlacement("ashlander_travel_pack", { chance: 0.12, scale: 1.04, cellSize: 148, radius: 7 }),
  landmarkPlacement("boulder", { chance: 0.20, scale: 0.98, variant: 1 }),
];

const BADLANDS_CRATER_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("velothi_ziggurat", { chance: 0.14, scale: 1.16, cellSize: 264, radius: 15 }),
  landmarkPlacement("ash_obelisk", { chance: 0.18, scale: 1.22, cellSize: 188, radius: 8 }),
  landmarkPlacement("rib_arch", { chance: 0.16, scale: 1.18, cellSize: 176, radius: 13 }),
  landmarkPlacement("old_road_causeway", { chance: 0.24, scale: 1.10, cellSize: 128, radius: 13 }),
  landmarkPlacement("scree_fan", { chance: 0.30, scale: 1.12, cellSize: 112, radius: 13 }),
  landmarkPlacement("buried_ribs", { chance: 0.18, scale: 1.10, cellSize: 156, radius: 12 }),
  landmarkPlacement("hoodoo", { chance: 0.34, scale: 1.24 }),
  landmarkPlacement("ash_marker", { chance: 0.26, scale: 1.22, cellSize: 148, radius: 5 }),
  landmarkPlacement("ashlander_travel_pack", { chance: 0.12, scale: 1.08, cellSize: 148, radius: 7 }),
  landmarkPlacement("standing_stone", { chance: 0.26, scale: 1.22 }),
  landmarkPlacement("dead_tree", { chance: 0.30, scale: 1.18, cellSize: 156, radius: 8 }),
  landmarkPlacement("boulder", { chance: 0.20, scale: 1.04, variant: 1 }),
];

const ASH_WASTES_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("velothi_ziggurat", { chance: 0.22, scale: 1.22, cellSize: 252, radius: 16 }),
  landmarkPlacement("ash_obelisk", { chance: 0.30, scale: 1.28, cellSize: 172, radius: 8 }),
  landmarkPlacement("rib_arch", { chance: 0.28, scale: 1.24, cellSize: 160, radius: 14 }),
  landmarkPlacement("old_road_causeway", { chance: 0.38, scale: 1.16, cellSize: 104, radius: 14 }),
  landmarkPlacement("paver_debris", { chance: 0.46, scale: 1.18, cellSize: 84, radius: 13 }),
  landmarkPlacement("scree_fan", { chance: 0.42, scale: 1.18, cellSize: 92, radius: 14 }),
  landmarkPlacement("shrine_debris", { chance: 0.28, scale: 1.12, cellSize: 124, radius: 15 }),
  landmarkPlacement("buried_ribs", { chance: 0.26, scale: 1.16, cellSize: 144, radius: 13 }),
  landmarkPlacement("pilgrim_lantern", { chance: 0.42, scale: 1.22, cellSize: 108, radius: 5 }),
  landmarkPlacement("bone_chimes", { chance: 0.30, scale: 1.18, cellSize: 132, radius: 7 }),
  landmarkPlacement("ash_marker", { chance: 0.46, scale: 1.28, cellSize: 120, radius: 5 }),
  landmarkPlacement("velothi_shrine", { chance: 0.28, scale: 1.18, cellSize: 148, radius: 5 }),
  landmarkPlacement("pilgrim_cairn", { chance: 0.26, scale: 1.14, cellSize: 136, radius: 5 }),
  landmarkPlacement("kwama_mound", { chance: 0.24, scale: 1.14, cellSize: 132, radius: 6 }),
  landmarkPlacement("silt_shell", { chance: 0.18, scale: 1.08, cellSize: 164, radius: 8 }),
  landmarkPlacement("basalt_spire", { chance: 0.24, scale: 1.22, cellSize: 148, radius: 7 }),
  landmarkPlacement("dead_snag", { chance: 0.18, scale: 1.08, variant: 1, cellSize: 152, radius: 4 }),
];

const HIGHLAND_REDWOOD_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("redwood", { chance: 0.38, scale: 1.16, cellSize: 184, radius: 22 }),
  landmarkPlacement("tall_fir", { chance: 0.54, scale: 1.20, cellSize: 108, radius: 12 }),
  landmarkPlacement("fir", { chance: 0.52, scale: 1.12, cellSize: 96, radius: 10 }),
  landmarkPlacement("berry_bush", { chance: 0.28, scale: 1.00, cellSize: 84, radius: 5 }),
  landmarkPlacement("boulder", { chance: 0.18, scale: 1.02 }),
];

const HIGHLAND_REDWOOD_BASIN_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("redwood", { chance: 0.56, scale: 1.30, cellSize: 152, radius: 26 }),
  landmarkPlacement("tall_fir", { chance: 0.88, scale: 1.30, cellSize: 84, radius: 13 }),
  landmarkPlacement("fir", { chance: 0.82, scale: 1.20, cellSize: 76, radius: 11 }),
  landmarkPlacement("stone_tor", { chance: 0.16, scale: 1.08, cellSize: 144, radius: 8 }),
  landmarkPlacement("berry_bush", { chance: 0.34, scale: 1.02, cellSize: 72, radius: 5 }),
];

const HIGHLAND_OLD_GROWTH_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("redwood", { chance: 0.44, scale: 1.22, cellSize: 176, radius: 24 }),
  landmarkPlacement("tall_fir", { chance: 0.72, scale: 1.26, cellSize: 96, radius: 13 }),
  landmarkPlacement("fir", { chance: 0.68, scale: 1.18, cellSize: 88, radius: 11 }),
  landmarkPlacement("berry_bush", { chance: 0.32, scale: 1.00, cellSize: 78, radius: 5 }),
  landmarkPlacement("boulder", { chance: 0.16, scale: 1.04 }),
];

const HIGHLAND_REDLEAF_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("redleaf_tree", { chance: 0.42, scale: 1.12, cellSize: 108, radius: 12 }),
  landmarkPlacement("birch", { chance: 0.28, scale: 1.00, cellSize: 128, radius: 9 }),
  landmarkPlacement("standing_stone", { chance: 0.26, scale: 1.20 }),
  landmarkPlacement("boulder", { chance: 0.24, scale: 1.06 }),
  landmarkPlacement("flower_patch", { chance: 0.20, scale: 1.00, variant: 2, cellSize: 80, radius: 5 }),
];

const MOOR_SHADOWGLASS_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("lantern_tree", { chance: 0.26, scale: 1.00, cellSize: 144, radius: 12 }),
  landmarkPlacement("dead_tree", { chance: 0.34, scale: 1.16, cellSize: 144, radius: 8 }),
  landmarkPlacement("standing_stone", { chance: 0.38, scale: 1.22 }),
  landmarkPlacement("ancestor_pillar", { chance: 0.24, scale: 1.14, cellSize: 156, radius: 6 }),
  landmarkPlacement("glowcap", { chance: 0.26, scale: 1.02, cellSize: 128, radius: 12 }),
  landmarkPlacement("frost_shrub", { chance: 0.36, scale: 1.04 }),
];

const TUNDRA_TAIGA_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("tall_fir", { chance: 0.42, scale: 1.16, cellSize: 124, radius: 12 }),
  landmarkPlacement("fir", { chance: 0.56, scale: 1.10, cellSize: 96, radius: 10 }),
  landmarkPlacement("frost_shrub", { chance: 0.44, scale: 1.06, cellSize: 84, radius: 4 }),
  landmarkPlacement("boulder", { chance: 0.18, scale: 1.00 }),
];

const TUNDRA_OLD_GROWTH_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("tall_fir", { chance: 0.58, scale: 1.22, cellSize: 108, radius: 12 }),
  landmarkPlacement("fir", { chance: 0.72, scale: 1.14, cellSize: 88, radius: 10 }),
  landmarkPlacement("frost_shrub", { chance: 0.40, scale: 1.08, cellSize: 78, radius: 4 }),
  landmarkPlacement("boulder", { chance: 0.16, scale: 1.02 }),
];

const TUNDRA_BLUE_ICE_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("ice_spire", { chance: 0.44, scale: 1.26 }),
  landmarkPlacement("tall_fir", { chance: 0.18, scale: 1.08, cellSize: 132, radius: 12 }),
  landmarkPlacement("frost_shrub", { chance: 0.52, scale: 1.08 }),
  landmarkPlacement("boulder", { chance: 0.24, scale: 1.02 }),
];

const MARSH_THICKET_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("crystal_reeds", { chance: 0.32, scale: 1.08, cellSize: 104, radius: 5 }),
  landmarkPlacement("mangrove", { chance: 0.36, scale: 1.18, cellSize: 160, radius: 15 }),
  landmarkPlacement("cypress", { chance: 0.42, scale: 1.12, cellSize: 128, radius: 10 }),
  landmarkPlacement("reed_cluster", { chance: 0.70, scale: 1.12, cellSize: 68, radius: 4 }),
  landmarkPlacement("flower_patch", { chance: 0.18, scale: 0.96, variant: 3, cellSize: 84, radius: 5 }),
];

const MARSH_WILLOW_THICKET_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("fungal_bridge", { chance: 0.20, scale: 1.08, cellSize: 172, radius: 15 }),
  landmarkPlacement("crystal_reeds", { chance: 0.34, scale: 1.10, cellSize: 100, radius: 5 }),
  landmarkPlacement("willow", { chance: 0.34, scale: 1.10, cellSize: 136, radius: 14 }),
  landmarkPlacement("mangrove", { chance: 0.30, scale: 1.18, cellSize: 154, radius: 15 }),
  landmarkPlacement("cypress", { chance: 0.40, scale: 1.14, cellSize: 118, radius: 10 }),
  landmarkPlacement("reed_cluster", { chance: 0.76, scale: 1.14, cellSize: 64, radius: 4 }),
  landmarkPlacement("flower_patch", { chance: 0.28, scale: 1.00, variant: 3, cellSize: 78, radius: 5 }),
];

const MARSH_BLACKWATER_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("rib_remains", { chance: 0.30, scale: 1.12, cellSize: 150, radius: 10 }),
  landmarkPlacement("fungal_bridge", { chance: 0.24, scale: 1.10, cellSize: 156, radius: 16 }),
  landmarkPlacement("crystal_reeds", { chance: 0.42, scale: 1.16, cellSize: 84, radius: 5 }),
  landmarkPlacement("willow", { chance: 0.40, scale: 1.14, cellSize: 128, radius: 14 }),
  landmarkPlacement("cypress", { chance: 0.44, scale: 1.16, cellSize: 118, radius: 10 }),
  landmarkPlacement("reed_cluster", { chance: 0.80, scale: 1.16, cellSize: 60, radius: 4 }),
  landmarkPlacement("glowcap", { chance: 0.18, scale: 0.98, cellSize: 144, radius: 10 }),
];

const FIREFLY_LANTERN_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("crystal_reeds", { chance: 0.28, scale: 1.08, cellSize: 100, radius: 5 }),
  landmarkPlacement("lantern_tree", { chance: 0.42, scale: 1.12, cellSize: 124, radius: 12 }),
  landmarkPlacement("glowcap", { chance: 0.42, scale: 1.10, cellSize: 116, radius: 12 }),
  landmarkPlacement("willow", { chance: 0.26, scale: 1.04, cellSize: 136, radius: 14 }),
  landmarkPlacement("reed_cluster", { chance: 0.82, scale: 1.14, cellSize: 60, radius: 4 }),
];

const SALTFLAT_MIRROR_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("salt_spire", { chance: 0.44, scale: 1.18 }),
  landmarkPlacement("crystal_cluster", { chance: 0.28, scale: 1.08, variant: 2 }),
  landmarkPlacement("glass_cairn", { chance: 0.24, scale: 1.12, cellSize: 136, radius: 5, variant: 2 }),
  landmarkPlacement("standing_stone", { chance: 0.18, scale: 1.06 }),
];

const FERN_CENOTE_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("giant_fern", { chance: 0.46, scale: 1.24, cellSize: 120, radius: 14 }),
  landmarkPlacement("canopy_tree", { chance: 0.26, scale: 1.20, cellSize: 128, radius: 16 }),
  landmarkPlacement("glowcap", { chance: 0.22, scale: 1.00, cellSize: 136, radius: 12 }),
  landmarkPlacement("berry_bush", { chance: 0.52, scale: 1.08, cellSize: 68, radius: 5 }),
  landmarkPlacement("flower_patch", { chance: 0.30, scale: 1.06, variant: 1, cellSize: 68, radius: 5 }),
];

const FERN_OVERGROWN_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("giant_fern", { chance: 0.66, scale: 1.34, cellSize: 96, radius: 15 }),
  landmarkPlacement("canopy_tree", { chance: 0.48, scale: 1.26, cellSize: 112, radius: 18 }),
  landmarkPlacement("root_stump", { chance: 0.20, scale: 1.10, cellSize: 132, radius: 9 }),
  landmarkPlacement("glowcap", { chance: 0.30, scale: 1.06, cellSize: 124, radius: 12 }),
  landmarkPlacement("berry_bush", { chance: 0.64, scale: 1.10, cellSize: 60, radius: 5 }),
  landmarkPlacement("flower_patch", { chance: 0.34, scale: 1.08, variant: 1, cellSize: 64, radius: 5 }),
];

const FUNGAL_MOONLIT_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("fungal_bridge", { chance: 0.34, scale: 1.18, cellSize: 152, radius: 17 }),
  landmarkPlacement("crystal_reeds", { chance: 0.22, scale: 1.08, cellSize: 116, radius: 5 }),
  landmarkPlacement("mega_glowcap", { chance: 0.32, scale: 1.20 }),
  landmarkPlacement("glowcap", { chance: 0.48, scale: 1.14 }),
  landmarkPlacement("lantern_tree", { chance: 0.24, scale: 1.02, cellSize: 136, radius: 12 }),
  landmarkPlacement("giant_flower", { chance: 0.24, scale: 1.14, cellSize: 120, radius: 10, variant: 1 }),
];

const FUNGAL_SPORE_GROVE_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("fungal_bridge", { chance: 0.40, scale: 1.22, cellSize: 136, radius: 18 }),
  landmarkPlacement("rib_remains", { chance: 0.14, scale: 1.02, cellSize: 184, radius: 10 }),
  landmarkPlacement("mega_glowcap", { chance: 0.40, scale: 1.24, cellSize: 140, radius: 20 }),
  landmarkPlacement("glowcap", { chance: 0.64, scale: 1.16, cellSize: 104, radius: 13 }),
  landmarkPlacement("lantern_tree", { chance: 0.34, scale: 1.08, cellSize: 120, radius: 13 }),
  landmarkPlacement("giant_flower", { chance: 0.28, scale: 1.18, cellSize: 116, radius: 11, variant: 1 }),
  landmarkPlacement("berry_bush", { chance: 0.34, scale: 1.04, cellSize: 76, radius: 5 }),
];

const ROOTED_SURFACE_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("root_stump", { chance: 0.38, scale: 1.16, cellSize: 112, radius: 9 }),
  landmarkPlacement("canopy_tree", { chance: 0.54, scale: 1.18, cellSize: 116, radius: 17 }),
  landmarkPlacement("willow", { chance: 0.32, scale: 1.08, cellSize: 132, radius: 15 }),
  landmarkPlacement("berry_bush", { chance: 0.60, scale: 1.10, cellSize: 64, radius: 5 }),
];

const PEATY_SURFACE_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("rib_remains", { chance: 0.18, scale: 1.04, cellSize: 172, radius: 10 }),
  landmarkPlacement("fungal_bridge", { chance: 0.20, scale: 1.08, cellSize: 160, radius: 15 }),
  landmarkPlacement("root_stump", { chance: 0.26, scale: 1.04, cellSize: 128, radius: 8 }),
  landmarkPlacement("willow", { chance: 0.40, scale: 1.12, cellSize: 132, radius: 15 }),
  landmarkPlacement("dead_tree", { chance: 0.30, scale: 1.12, cellSize: 140, radius: 8 }),
  landmarkPlacement("reed_cluster", { chance: 0.82, scale: 1.14, cellSize: 60, radius: 4 }),
  landmarkPlacement("glowcap", { chance: 0.22, scale: 1.00, cellSize: 132, radius: 11 }),
];

const GRANITIC_TOR_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("stone_tor", { chance: 0.34, scale: 1.18, cellSize: 128, radius: 8 }),
  landmarkPlacement("standing_stone", { chance: 0.36, scale: 1.20, cellSize: 140, radius: 6 }),
  landmarkPlacement("boulder", { chance: 0.26, scale: 1.08, cellSize: 104, radius: 5 }),
  landmarkPlacement("fir", { chance: 0.20, scale: 1.02, cellSize: 124, radius: 10 }),
];

const SALINE_CRUST_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("salt_spire", { chance: 0.50, scale: 1.22, cellSize: 132, radius: 7 }),
  landmarkPlacement("stone_tor", { chance: 0.22, scale: 1.08, cellSize: 148, radius: 7 }),
  landmarkPlacement("crystal_cluster", { chance: 0.22, scale: 1.10, cellSize: 116, radius: 5 }),
  landmarkPlacement("shrub", { chance: 0.16, scale: 0.86, variant: 2, cellSize: 92, radius: 4 }),
];

const MYCELIAL_SURFACE_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("mega_glowcap", { chance: 0.36, scale: 1.22, cellSize: 144, radius: 19 }),
  landmarkPlacement("glowcap", { chance: 0.52, scale: 1.14, cellSize: 112, radius: 12 }),
  landmarkPlacement("lantern_tree", { chance: 0.26, scale: 1.06, cellSize: 128, radius: 12 }),
  landmarkPlacement("berry_bush", { chance: 0.30, scale: 1.00, cellSize: 80, radius: 5 }),
];

const CRYSTALLINE_SURFACE_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("crystal_cluster", { chance: 0.52, scale: 1.20, cellSize: 116, radius: 5, variant: 2 }),
  landmarkPlacement("salt_spire", { chance: 0.26, scale: 1.10, cellSize: 140, radius: 7, variant: 1 }),
  landmarkPlacement("stone_tor", { chance: 0.24, scale: 1.10, cellSize: 144, radius: 7 }),
  landmarkPlacement("boulder", { chance: 0.14, scale: 0.98, cellSize: 108, radius: 5 }),
];

const BASALTIC_SURFACE_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("basalt_spire", { chance: 0.44, scale: 1.30, cellSize: 140, radius: 7 }),
  landmarkPlacement("dead_snag", { chance: 0.32, scale: 1.14, cellSize: 144, radius: 4, variant: 1 }),
  landmarkPlacement("stone_tor", { chance: 0.18, scale: 1.08, cellSize: 156, radius: 7 }),
  landmarkPlacement("boulder", { chance: 0.20, scale: 1.00, cellSize: 108, radius: 5, variant: 1 }),
];

const EMBER_DEADLAND_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("ash_obelisk", { chance: 0.26, scale: 1.30, cellSize: 176, radius: 8 }),
  landmarkPlacement("pilgrim_lantern", { chance: 0.24, scale: 1.16, cellSize: 132, radius: 5 }),
  landmarkPlacement("dead_tree", { chance: 0.28, scale: 1.22, cellSize: 156, radius: 8 }),
  landmarkPlacement("basalt_spire", { chance: 0.28, scale: 1.30 }),
  landmarkPlacement("ash_marker", { chance: 0.24, scale: 1.22, cellSize: 144, radius: 5 }),
  landmarkPlacement("kwama_mound", { chance: 0.16, scale: 1.12, cellSize: 156, radius: 6 }),
  landmarkPlacement("ashlander_travel_pack", { chance: 0.10, scale: 1.04, cellSize: 152, radius: 7 }),
  landmarkPlacement("crystal_cluster", { chance: 0.26, scale: 1.12, variant: 3 }),
  landmarkPlacement("boulder", { chance: 0.18, scale: 0.96, variant: 1 }),
];

const EMBER_CALDERA_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("velothi_ziggurat", { chance: 0.12, scale: 1.20, cellSize: 276, radius: 16 }),
  landmarkPlacement("ash_obelisk", { chance: 0.32, scale: 1.34, cellSize: 164, radius: 8 }),
  landmarkPlacement("old_road_causeway", { chance: 0.18, scale: 1.08, cellSize: 136, radius: 13 }),
  landmarkPlacement("basalt_spire", { chance: 0.38, scale: 1.34 }),
  landmarkPlacement("ash_marker", { chance: 0.34, scale: 1.28, cellSize: 132, radius: 5 }),
  landmarkPlacement("pilgrim_cairn", { chance: 0.18, scale: 1.14, cellSize: 152, radius: 5 }),
  landmarkPlacement("ashlander_travel_pack", { chance: 0.18, scale: 1.08, cellSize: 144, radius: 7 }),
  landmarkPlacement("crystal_cluster", { chance: 0.34, scale: 1.16, variant: 3 }),
  landmarkPlacement("dead_tree", { chance: 0.24, scale: 1.18, cellSize: 164, radius: 8 }),
  landmarkPlacement("boulder", { chance: 0.18, scale: 1.00, variant: 1 }),
];

const BLOOM_ORCHARD_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("mega_glowcap", { chance: 0.18, scale: 1.22 }),
  landmarkPlacement("giant_flower", { chance: 0.22, scale: 1.18, cellSize: 124, radius: 10, variant: 1 }),
  landmarkPlacement("blossom_tree", { chance: 0.28, scale: 1.18, cellSize: 128, radius: 12 }),
  landmarkPlacement("fruit_tree", { chance: 0.22, scale: 1.10, cellSize: 128, radius: 11 }),
  landmarkPlacement("glowcap", { chance: 0.30, scale: 1.06 }),
  landmarkPlacement("flower_patch", { chance: 0.34, scale: 1.12, variant: 4, cellSize: 72, radius: 5 }),
];

const BLOOM_FLOWER_GROVE_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("giant_flower", { chance: 0.56, scale: 1.34, cellSize: 92, radius: 12, variant: 1 }),
  landmarkPlacement("blossom_tree", { chance: 0.50, scale: 1.24, cellSize: 98, radius: 13 }),
  landmarkPlacement("fruit_tree", { chance: 0.26, scale: 1.12, cellSize: 122, radius: 11 }),
  landmarkPlacement("glowcap", { chance: 0.44, scale: 1.12, cellSize: 132, radius: 12 }),
  landmarkPlacement("flower_patch", { chance: 0.72, scale: 1.22, variant: 4, cellSize: 56, radius: 6 }),
];

const BLOOM_PRISM_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("giant_flower", { chance: 0.34, scale: 1.26, cellSize: 116, radius: 11, variant: 1 }),
  landmarkPlacement("crystal_cluster", { chance: 0.42, scale: 1.18, variant: 2 }),
  landmarkPlacement("glowcap", { chance: 0.36, scale: 1.12 }),
  landmarkPlacement("blossom_tree", { chance: 0.24, scale: 1.14, cellSize: 122, radius: 12 }),
  landmarkPlacement("flower_patch", { chance: 0.44, scale: 1.18, variant: 4, cellSize: 64, radius: 5 }),
];

const chunkGenerationScratchPool: ChunkGenerationScratch[] = [];

export function buildHexColorPalette(): number[] {
  const palette = new Array<number>(HEX_COLOR_COUNT + 1);
  palette[0] = 0;
  for (let value = 0; value < HEX_COLOR_COUNT; value += 1) {
    const r = ((value >> 8) & 0xf) * 17;
    const g = ((value >> 4) & 0xf) * 17;
    const b = (value & 0xf) * 17;
    palette[value + 1] = packRgba(r, g, b);
  }
  return palette;
}

export function buildProceduralPalette(): number[] {
  const palette = buildHexColorPalette();
  for (const material of PROCEDURAL_WATER_MATERIALS) {
    palette[material] = (palette[material]! & 0x00ff_ffff) | (PROCEDURAL_WATER_ALPHA << 24);
  }
  return palette;
}

export function isProceduralWaterMaterial(materialIndex: number): boolean {
  return PROCEDURAL_WATER_MATERIALS.has(materialIndex);
}

export function hexColorToMaterial(code: string): number {
  const clean = code.startsWith("#") ? code.slice(1) : code;
  if (!/^[0-9a-fA-F]{3}$/.test(clean)) {
    throw new Error(`Expected a #RGB color code, received "${code}"`);
  }
  return Number.parseInt(clean, 16) + 1;
}

export function materialToHexColor(materialIndex: number): string {
  if (materialIndex <= 0 || materialIndex > HEX_COLOR_COUNT) {
    throw new Error(`Material index ${materialIndex} is outside the #RGB palette`);
  }
  return `#${(materialIndex - 1).toString(16).padStart(3, "0").toUpperCase()}`;
}

export class ProceduralWorldGenerator {
  readonly palette = buildProceduralPalette();
  readonly seaLevel: number;
  readonly chunkSize: number;
  readonly maxYExclusive: number;
  private readonly continentSeed: number;
  private readonly upliftSeed: number;
  private readonly hillsSeed: number;
  private readonly detailSeed: number;
  private readonly ridgeSeed: number;
  private readonly basinSeed: number;
  private readonly drainageSeed: number;
  private readonly temperatureSeed: number;
  private readonly moistureSeed: number;
  private readonly volcanismSeed: number;
  private readonly magicSeed: number;
  private readonly peakSeed: number;
  private readonly channelSeed: number;
  private readonly duneSeed: number;
  private readonly mesaSeed: number;
  private readonly groveSeed: number;
  private readonly oldGrowthSeed: number;
  private readonly orchardSeed: number;
  private readonly desolationSeed: number;
  private readonly strataSeed: number;
  private readonly surfacePatchSeed: number;
  private readonly surfaceGrainSeed: number;
  private readonly surfaceScatterSeed: number;
  private readonly caveRibbonSeed: number;
  private readonly cavePocketSeed: number;
  private readonly caveDepthSeed: number;
  private readonly caveOpeningSeed: number;
  private readonly transitionSeed: number;
  private readonly featureSeed: number;
  private readonly columnSampleState = createMutableColumnState();
  private readonly biomeProbeState = createMutableColumnState();
  private readonly surfaceSampleState = createMutableColumnState();
  private readonly materialSampleState = createMutableColumnState();
  private readonly reusableSurfaceFields = createSurfaceFieldSample();
  private readonly reusableCaveFields = createCaveFieldSample();
  private readonly reusableBaseBiomeBlend: BaseBiomeBlendSelection = {
    primary: BASE_BIOMES[0]!,
    secondary: BASE_BIOMES[1]!,
    primaryWeight: 1,
  };
  private readonly biomeClassificationState: BiomeClassificationSelection = {
    biomeId: "verdant",
    specialStrength: 0,
    biomeCore: 0,
  };
  private readonly resolvedSurfaceMaterials: ResolvedSurfaceMaterials = {
    surfacePrimary: 0,
    surfaceSecondary: 0,
    subsurfacePrimary: 0,
    subsurfaceSecondary: 0,
    water: 0,
    snow: 0,
    transitionThreshold: 1,
  };

  constructor(
    readonly seed = 1337,
    options: {
      seaLevel?: number;
      chunkSize?: number;
      maxYExclusive?: number;
    } = {},
  ) {
    this.seaLevel = options.seaLevel ?? 1400;
    this.chunkSize = options.chunkSize ?? 32;
    this.maxYExclusive = options.maxYExclusive ?? PROCEDURAL_WORLD_MAX_Y;
    this.continentSeed = seed + 101;
    this.upliftSeed = seed + 151;
    this.hillsSeed = seed + 199;
    this.detailSeed = seed + 251;
    this.ridgeSeed = seed + 307;
    this.basinSeed = seed + 353;
    this.drainageSeed = seed + 401;
    this.temperatureSeed = seed + 457;
    this.moistureSeed = seed + 503;
    this.volcanismSeed = seed + 557;
    this.magicSeed = seed + 601;
    this.peakSeed = seed + 617;
    this.channelSeed = seed + 653;
    this.duneSeed = seed + 709;
    this.mesaSeed = seed + 761;
    this.groveSeed = seed + 787;
    this.oldGrowthSeed = seed + 811;
    this.orchardSeed = seed + 829;
    this.desolationSeed = seed + 881;
    this.strataSeed = seed + 809;
    this.surfacePatchSeed = seed + 863;
    this.surfaceGrainSeed = seed + 911;
    this.surfaceScatterSeed = seed + 967;
    this.transitionSeed = seed + 1013;
    this.featureSeed = seed + 1061;
    this.caveRibbonSeed = seed + 1109;
    this.cavePocketSeed = seed + 1157;
    this.caveDepthSeed = seed + 1201;
    this.caveOpeningSeed = seed + 1253;
  }

  sampleColumn(worldX: number, worldZ: number): ProceduralColumnSample {
    const state = this.columnSampleState;
    this.fillSurfaceColumnState(worldX, worldZ, state);
    return columnSampleFromState(state);
  }

  sampleBiomeProbe(worldX: number, worldZ: number): ProceduralBiomeProbe {
    const state = this.biomeProbeState;
    this.fillSurfaceColumnState(worldX, worldZ, state);
    return {
      ...columnSampleFromState(state),
      regionId: state.regionId,
      secondaryRegionId: state.secondaryRegionId,
      regionStrength: state.regionStrength,
      regionAmbientProfileId: state.regionAmbientProfileId,
      secondaryBiomeId: state.secondaryBiomeId,
      transitionThreshold: state.transitionThreshold,
      specialStrength: state.specialStrength,
      regionalVariantStrength: state.regionalVariantStrength,
      fields: {
        temperature: state.temperature,
        moisture: state.moisture,
        uplift: state.uplift,
        drainage: state.drainage,
        volcanism: state.volcanism,
        magic: state.magic,
        globalHeight: state.globalHeight,
        mountainness: state.mountainness,
        oceanness: state.oceanness,
        ridge: this.lastFillSurfaceFields.ridge,
        mesa: this.lastFillSurfaceFields.mesa,
        desolation: this.lastFillSurfaceFields.desolation,
        strata: this.lastFillSurfaceFields.strata,
        surfacePatch: this.lastFillSurfaceFields.surfacePatch,
        surfaceGrain: this.lastFillSurfaceFields.surfaceGrain,
        scatter: this.lastFillSurfaceFields.scatter,
        peakness: this.lastFillSurfaceFields.peakness,
        islandInterior: this.lastFillSurfaceFields.islandInterior,
        shorelineBand: this.lastFillSurfaceFields.shorelineBand,
        coastalShelf: this.lastFillSurfaceFields.coastalShelf,
        deepOcean: this.lastFillSurfaceFields.deepOcean,
      },
    };
  }

  sampleSurfaceColumn(worldX: number, worldZ: number): ProceduralSurfaceColumnSample {
    const state = this.surfaceSampleState;
    this.fillSurfaceColumnState(worldX, worldZ, state);
    return surfaceColumnSampleFromState(state);
  }

  sampleMaterial(worldX: number, worldY: number, worldZ: number): number {
    if (worldY < 0 || worldY >= this.maxYExclusive) {
      return 0;
    }
    const state = this.materialSampleState;
    this.fillColumnState(worldX, worldZ, state);
    return this.sampleMaterialFromColumn(state, worldY);
  }

  sampleColumnMaterialBuckets(
    worldX: number,
    worldZ: number,
    firstBucketMinY: number,
    bucketSize: number,
    bucketCount: number,
  ): Uint16Array {
    const count = Math.max(0, Math.floor(bucketCount));
    const materials = new Uint16Array(count);
    if (count === 0) {
      return materials;
    }
    const state = this.materialSampleState;
    this.fillColumnState(worldX, worldZ, state);
    return this.sampleMaterialBucketsFromColumnState(state, firstBucketMinY, bucketSize, count);
  }

  sampleTopColumnMaterialBucket(
    worldX: number,
    worldZ: number,
    firstBucketMinY: number,
    bucketSize: number,
    bucketCount: number,
    shellPaddingY: number,
  ): ProceduralTopColumnMaterialBucketSample | null {
    const count = Math.max(0, Math.floor(bucketCount));
    if (count === 0) {
      return null;
    }
    const state = this.materialSampleState;
    this.fillColumnState(worldX, worldZ, state);
    const stride = Math.max(1, Math.floor(bucketSize));
    const minSurfaceWorldY = state.surfaceY - Math.max(0, Math.floor(shellPaddingY));
    const maxSurfaceWorldY = Math.max(topYFromState(state), nullableWaterTopY(state.waterTopY) ?? state.surfaceY);
    const startBucket = Math.max(0, Math.floor((minSurfaceWorldY - firstBucketMinY) / stride));
    const endBucket = Math.min(count - 1, Math.floor((maxSurfaceWorldY - firstBucketMinY) / stride));
    if (startBucket > endBucket) {
      return null;
    }
    for (let bucketIndex = endBucket; bucketIndex >= startBucket; bucketIndex -= 1) {
      const minWorldY = Math.max(0, Math.floor(firstBucketMinY + bucketIndex * stride));
      const maxWorldY = Math.min(this.maxYExclusive - 1, minWorldY + stride - 1);
      if (maxWorldY < minWorldY) {
        continue;
      }
      let waterMaterial = 0;
      for (let worldY = maxWorldY; worldY >= minWorldY; worldY -= 1) {
        const material = this.sampleMaterialFromColumn(state, worldY);
        if (material === 0) {
          continue;
        }
        if (!isProceduralWaterMaterial(material)) {
          return {
            ...surfaceColumnSampleFromState(state),
            bucketIndex,
            material,
          };
        }
        if (waterMaterial === 0) {
          waterMaterial = material;
        }
      }
      if (waterMaterial !== 0) {
        return {
          ...surfaceColumnSampleFromState(state),
          bucketIndex,
          material: waterMaterial,
        };
      }
    }
    return null;
  }

  private sampleMaterialBucketsFromColumnState(
    state: MutableColumnState,
    firstBucketMinY: number,
    bucketSize: number,
    bucketCount: number,
  ): Uint16Array {
    const count = Math.max(0, Math.floor(bucketCount));
    const materials = new Uint16Array(count);
    if (count === 0) {
      return materials;
    }
    const stride = Math.max(1, Math.floor(bucketSize));
    for (let bucketIndex = 0; bucketIndex < count; bucketIndex += 1) {
      const minWorldY = Math.max(0, Math.floor(firstBucketMinY + bucketIndex * stride));
      const maxWorldY = Math.min(this.maxYExclusive - 1, minWorldY + stride - 1);
      if (maxWorldY < minWorldY) {
        continue;
      }
      let waterMaterial = 0;
      for (let worldY = maxWorldY; worldY >= minWorldY; worldY -= 1) {
        const material = this.sampleMaterialFromColumn(state, worldY);
        if (material === 0) {
          continue;
        }
        if (!isProceduralWaterMaterial(material)) {
          materials[bucketIndex] = material;
          break;
        }
        if (waterMaterial === 0) {
          waterMaterial = material;
        }
      }
      if (materials[bucketIndex] === 0) {
        materials[bucketIndex] = waterMaterial;
      }
    }
    return materials;
  }

  generateChunk(cx: number, cy: number, cz: number): GeneratedChunk {
    const chunkArea = this.chunkSize * this.chunkSize;
    const data = new Uint16Array(this.chunkSize * chunkArea);
    const originX = cx * this.chunkSize;
    const originY = cy * this.chunkSize;
    const originZ = cz * this.chunkSize;
    const scratch = acquireChunkGenerationScratch(chunkArea);
    const columnState = createMutableColumnState();
    for (let z = 0; z < this.chunkSize; z += 1) {
        const worldZ = originZ + z;
        const rowOffset = z * this.chunkSize;
      for (let x = 0; x < this.chunkSize; x += 1) {
        this.fillColumnState(originX + x, worldZ, columnState);
        this.writeChunkColumnState(scratch, x + rowOffset, columnState);
      }
    }

    // Compute per-column max Y to enable early exit in the voxel loop.
    // For columns entirely below originY, or entirely above originY + chunkSize,
    // we can skip Y iterations.
    let columnMaxTopY = 0;
    for (let i = 0; i < chunkArea; i += 1) {
      const surfY = scratch.surfaceY[i]!;
      const waterY = scratch.waterTopY[i]!;
      const featureH = scratch.featureHeight[i]!;
      let topY = surfY;
      if (waterY !== NO_WATER && waterY > topY) topY = waterY;
      if (featureH > 0) topY = surfY + featureH;
      if (topY > columnMaxTopY) columnMaxTopY = topY;
    }
    // If the entire chunk is above the maximum column top, skip the voxel loop
    const maxRelevantY = Math.min(originY + this.chunkSize - 1, columnMaxTopY);

    let solidCount = 0;
    let minX = this.chunkSize;
    let minY = this.chunkSize;
    let minZ = this.chunkSize;
    let maxX = 0;
    let maxY = 0;
    let maxZ = 0;
    for (let z = 0; z < this.chunkSize; z += 1) {
      const rowOffset = z * this.chunkSize;
      for (let y = 0; y < this.chunkSize; y += 1) {
        const worldY = originY + y;
        if (worldY > maxRelevantY) break;
        const worldYDiv3 = Math.floor(worldY * ONE_THIRD);
        const worldYBandBase = worldY * STRATA_BAND_SCALE;
        const planeOffset = y * this.chunkSize + z * chunkArea;
        for (let x = 0; x < this.chunkSize; x += 1) {
          const columnIndex = x + rowOffset;
          const surfaceY = scratch.surfaceY[columnIndex]!;
          // Fast path: above surface with no water
          if (worldY > surfaceY) {
            const waterTopY = scratch.waterTopY[columnIndex]!;
            if (waterTopY === NO_WATER || worldY > waterTopY) {
              // Check feature above surface
              if (scratch.featureKind[columnIndex] !== FEATURE_NONE) {
                const relativeY = worldY - (surfaceY + 1);
                if (relativeY >= 0 && relativeY <= scratch.featureHeight[columnIndex]!) {
                  const featureMat = sampleFeatureMaterial(
                    scratch.featureKind[columnIndex]!,
                    scratch.featureHeight[columnIndex]!,
                    scratch.featureRadius[columnIndex]!,
                    scratch.featureExtra[columnIndex]!,
                    scratch.featureDeltaX[columnIndex]!,
                    scratch.featureDeltaZ[columnIndex]!,
                    scratch.featureMaterialPrimary[columnIndex]!,
                    scratch.featureMaterialSecondary[columnIndex]!,
                    scratch.featureMaterialAccent[columnIndex]!,
                    surfaceY,
                    worldY,
                  );
                  if (featureMat !== 0) {
                    data[x + planeOffset] = featureMat;
                    solidCount += 1;
                    if (x < minX) minX = x;
                    if (y < minY) minY = y;
                    if (z < minZ) minZ = z;
                    if (x + 1 > maxX) maxX = x + 1;
                    if (y + 1 > maxY) maxY = y + 1;
                    if (z + 1 > maxZ) maxZ = z + 1;
                  }
                }
              }
              continue;
            }
            // Water above surface
            data[x + planeOffset] = scratch.waterMaterial[columnIndex]!;
            solidCount += 1;
            if (x < minX) minX = x;
            if (y < minY) minY = y;
            if (z < minZ) minZ = z;
            if (x + 1 > maxX) maxX = x + 1;
            if (y + 1 > maxY) maxY = y + 1;
            if (z + 1 > maxZ) maxZ = z + 1;
            continue;
          }
          const material = sampleMaterialFromScratch(
            scratch,
            columnIndex,
            worldY,
            worldYDiv3,
            worldYBandBase,
          );
          if (material === 0) {
            continue;
          }
          data[x + planeOffset] = material;
          solidCount += 1;
          if (x < minX) minX = x;
          if (y < minY) minY = y;
          if (z < minZ) minZ = z;
          if (x + 1 > maxX) maxX = x + 1;
          if (y + 1 > maxY) maxY = y + 1;
          if (z + 1 > maxZ) maxZ = z + 1;
        }
      }
    }
    releaseChunkGenerationScratch(scratch);

    return {
      coord: { x: cx, y: cy, z: cz },
      data,
      solidCount,
      solidBounds: solidCount === 0
        ? null
        : {
            min: [minX, minY, minZ],
            max: [maxX, maxY, maxZ],
          },
      renderSummary: summarizeGeneratedChunkRender(
        { x: cx, y: cy, z: cz },
        data,
        this.chunkSize,
        isProceduralWaterMaterial,
      ),
    };
  }

  private fillSurfaceColumnState(
    worldX: number,
    worldZ: number,
    out: MutableColumnState,
  ): void {
    const fields = this.sampleSurfaceFields(worldX, worldZ);
    const baseBlend = this.selectBaseBiomes(fields);
    const terrainProfile = blendTerrainProfile(baseBlend.primary, baseBlend.secondary, baseBlend.primaryWeight);
    const biomeSelection = this.selectBiomeClassification(fields, baseBlend);
    const biomeCore = biomeSelection.biomeCore;
    let surfaceY = this.sampleSurfaceY(fields, terrainProfile, biomeCore);
    const biomeId = biomeSelection.biomeId;
    const specialStrength = biomeSelection.specialStrength;
    surfaceY = adjustSpecialBiomeSurfaceY(this.seaLevel, biomeId, specialStrength, fields, biomeCore, surfaceY);

    const regionalVariant = fields.regionStrength > WORLD_REGION_AUTHORITY_THRESHOLD && fields.regionVariantId
      ? { id: fields.regionVariantId, strength: fields.regionStrength }
      : selectRegionalVariant(biomeId, fields);
    if (regionalVariant) {
      surfaceY += sampleRegionalVariantSurfaceDelta(regionalVariant.id, regionalVariant.strength, fields, biomeCore);
    }
    const routeSurface = samplePilgrimRouteSurfaceInfluence(worldX, worldZ, biomeId, fields);
    if (routeSurface) {
      surfaceY += samplePilgrimRouteSurfaceDelta(routeSurface, fields, biomeCore);
    }
    const pilgrimRouteInfluence = routeSurface
      ? clamp(Math.max(routeSurface.core, routeSurface.shoulder) + routeSurface.fracture * 0.18, 0, 1)
      : 0;
    surfaceY = clamp(surfaceY, 8, this.maxYExclusive - 2);
    const hostBiomeId = resolveHostBiomeId(biomeId, baseBlend.primary.id, baseBlend.secondary.id);
    const snowLine = terrainProfile.snowLine - Math.round((fields.temperature - 0.5) * 90);
    const undergroundBiomeId = this.selectUndergroundBiome(biomeId, hostBiomeId, fields);
    const underground = UNDERGROUND_BIOMES[undergroundBiomeId];
    const surfaceMaterials = this.resolveSurfaceMaterials(
      biomeId,
      baseBlend.primary,
      baseBlend.secondary,
      baseBlend.primaryWeight,
      specialStrength,
      fields,
      biomeCore,
      surfaceY,
    );
    if (regionalVariant) {
      applyRegionalVariantMaterialOverrides(surfaceMaterials, regionalVariant.id);
    }
    applyAtlasSurfaceMaterialOverrides(surfaceMaterials, fields);
    if (routeSurface) {
      applyPilgrimRouteSurfaceMaterials(surfaceMaterials, routeSurface, fields);
    }
    const waterTopY = this.resolveWaterTopY(
      biomeId,
      surfaceY,
      fields,
      specialStrength,
      regionalVariant?.id ?? null,
      regionalVariant?.strength ?? 0,
    );
    const submergedSurface = hasStandingWater(surfaceY, waterTopY);
    if (submergedSurface) {
      surfaceMaterials.surfacePrimary = surfaceMaterials.subsurfacePrimary;
      surfaceMaterials.surfaceSecondary = surfaceMaterials.subsurfaceSecondary;
    }
    const landmarkId = this.resolveLandmark(
      worldX,
      worldZ,
      biomeId,
      undergroundBiomeId,
      regionalVariant?.id ?? null,
      surfaceY,
      waterTopY,
      fields,
      out,
    );

    out.biomeId = biomeId;
    out.regionId = fields.regionId;
    out.secondaryRegionId = fields.secondaryRegionId;
    out.regionStrength = fields.regionStrength;
    out.regionAmbientProfileId = fields.regionAmbientProfileId;
    out.hostBiomeId = hostBiomeId;
    out.secondaryBiomeId = baseBlend.secondary.id;
    out.undergroundBiomeId = undergroundBiomeId;
    out.regionalVariantId = regionalVariant?.id ?? null;
    out.regionalVariantStrength = regionalVariant?.strength ?? 0;
    out.landmarkId = landmarkId;
    out.pilgrimRouteInfluence = pilgrimRouteInfluence;
    out.temperature = fields.temperature;
    out.moisture = fields.moisture;
    out.uplift = fields.uplift;
    out.drainage = fields.drainage;
    out.volcanism = fields.volcanism;
    out.magic = fields.magic;
    out.globalHeight = fields.globalHeight;
    out.mountainness = fields.mountainness;
    out.oceanness = fields.oceanness;
    out.surfaceY = surfaceY;
    out.waterTopY = waterTopY;
    out.surfaceMaterialPrimary = surfaceY >= snowLine && biomeId !== "ember" ? surfaceMaterials.snow : surfaceMaterials.surfacePrimary;
    out.surfaceMaterialSecondary = surfaceY >= snowLine && biomeId !== "ember" ? surfaceMaterials.snow : surfaceMaterials.surfaceSecondary;
    out.subsurfacePrimary = surfaceMaterials.subsurfacePrimary;
    out.subsurfaceSecondary = surfaceMaterials.subsurfaceSecondary;
    out.waterMaterial = surfaceMaterials.water;
    out.snowMaterial = surfaceMaterials.snow;
    out.stoneMaterial = underground.stone;
    out.deepStoneMaterial = underground.deepStone;
    out.accentMaterial = underground.accent;
    out.transitionThreshold = surfaceMaterials.transitionThreshold;
    out.specialStrength = specialStrength;
    out.strataOffset = fields.strata * 5;
    out.worldXDiv3 = Math.floor(worldX * SURFACE_MATERIAL_DITHER_SCALE);
    out.worldZDiv3 = Math.floor(worldZ * SURFACE_MATERIAL_DITHER_SCALE);
    out.ditherSeed = this.transitionSeed + baseBlend.primary.surface + baseBlend.secondary.surface;
    out.accentSeed = this.seed + underground.accent;
    this.lastFillSurfaceFields = fields;
    this.lastFillSurfaceBiomePrimaryWeight = baseBlend.primaryWeight;
  }

  private lastFillSurfaceFields: SurfaceFieldSample = createSurfaceFieldSample();
  private lastFillSurfaceBiomePrimaryWeight = 1;

  private fillColumnState(
    worldX: number,
    worldZ: number,
    out: MutableColumnState,
  ): void {
    this.fillSurfaceColumnState(worldX, worldZ, out);
    const caveFields = this.sampleCaveFields(worldX, worldZ);
    this.configureCaveState(
      out.biomeId,
      out.hostBiomeId,
      out.undergroundBiomeId,
      out.regionalVariantId,
      out.surfaceY,
      out.waterTopY,
      this.lastFillSurfaceFields,
      caveFields,
      this.lastFillSurfaceBiomePrimaryWeight,
      out,
    );
  }

  private sampleSurfaceFields(worldX: number, worldZ: number): SurfaceFieldSample {
    const out = this.reusableSurfaceFields;
    const province = sampleWorldRegion(worldX, worldZ);
    const atlas = sampleGeneratorAtlasFields(worldX, worldZ);
    const atlasRegionId = atlas.primaryRegionId ?? province.regionId;
    const atlasSecondaryRegionId = atlas.secondaryRegionId ?? atlasRegionId;
    const atlasBiomeId = atlas.primaryBiomeId === "ocean" || atlas.primaryBiomeId === "deep-ocean"
      ? province.biomeId
      : atlas.primaryBiomeId;
    const atlasRegionStrength = atlas.primaryRegionId
      ? clamp(atlas.regionStrength + atlas.islandInterior * 0.08, 0, 1)
      : 0;
    const atlasRegionAuthority = atlas.primaryRegionId
      ? atlasRegionStrength * smoothstep(0.72, 0.18, atlas.regionBlend)
      : 0;
    const atlasRegionCore = atlas.primaryRegionId
      ? smoothstep(1.18, 0.12, atlas.regionDistance) * atlas.islandInterior * atlasRegionAuthority
      : 0;
    const continentalness = fbm2D5(worldX * CONTINENT_SCALE, worldZ * CONTINENT_SCALE, this.continentSeed) - 0.5;
    const uplift = fbm2D4(worldX * UPLIFT_SCALE, worldZ * UPLIFT_SCALE, this.upliftSeed);
    const hills = fbm2D4(worldX * HILLS_SCALE, worldZ * HILLS_SCALE, this.hillsSeed) - 0.5;
    const detail = fbm2D4(worldX * DETAIL_SCALE, worldZ * DETAIL_SCALE, this.detailSeed) - 0.5;
    const ridge = 1 - Math.abs(fbm2D3(worldX * RIDGE_SCALE, worldZ * RIDGE_SCALE, this.ridgeSeed) * 2 - 1);
    const basin = fbm2D3(worldX * BASIN_SCALE, worldZ * BASIN_SCALE, this.basinSeed) - 0.5;
    const peakness = fbm2D3(worldX * PEAK_SCALE, worldZ * PEAK_SCALE, this.peakSeed);
    const mountainness = smoothstep(0.56, 0.84, uplift) * smoothstep(0.42, 0.78, ridge);
    const oceanness = smoothstep(-0.40, -0.08, continentalness) * smoothstep(-0.28, 0.06, -basin);
    const globalHeight = saturate(
      0.26
        + (continentalness + 0.5) * 0.34
        + uplift * 0.14
        - oceanness * 0.14
        - smoothstep(0.46, 0.82, -basin) * 0.05,
    );
    out.temperature = clamp(
      fbm2D4(worldX * TEMPERATURE_SCALE, worldZ * TEMPERATURE_SCALE, this.temperatureSeed)
        + province.southernSaltBasin * 0.10
        + province.northeastGrazelands * 0.04
        - province.westWetlands * 0.05
        - atlas.coastalShelf * 0.05,
      0,
      1,
    );
    out.moisture = clamp(
      fbm2D4(worldX * MOISTURE_SCALE, worldZ * MOISTURE_SCALE, this.moistureSeed)
        + province.westWetlands * 0.34
        + atlas.coastalShelf * 0.08
        - province.ashRing * 0.22
        - province.volcanicHeart * 0.34
        - province.southernSaltBasin * 0.28,
      0,
      1,
    );
    out.uplift = clamp(uplift + province.volcanicHeart * 0.12 + province.ashRing * 0.04 + sampleAtlasUpliftBias(atlasRegionId, atlasRegionCore) - atlas.coastalShelf * 0.16, 0, 1);
    out.drainage = clamp(
      fbm2D3(worldX * DRAINAGE_SCALE, worldZ * DRAINAGE_SCALE, this.drainageSeed)
        + province.westWetlands * 0.18
        + province.southernSaltBasin * 0.12
        - province.volcanicHeart * 0.12,
      0,
      1,
    );
    out.volcanism = clamp(
      fbm2D3(worldX * VOLCANISM_SCALE, worldZ * VOLCANISM_SCALE, this.volcanismSeed)
        + province.volcanicHeart * 0.52
        + province.ashRing * 0.26
        + province.easternShardCoast * 0.16,
      0,
      1,
    );
    out.magic = clamp(
      fbm2D3(worldX * MAGIC_SCALE, worldZ * MAGIC_SCALE, this.magicSeed)
        + province.westWetlands * 0.12
        + province.easternShardCoast * 0.22
        + province.southernSaltBasin * 0.10,
      0,
      1,
    );
    out.globalHeight = clamp(globalHeight * (0.52 + atlas.islandInterior * 0.58) - atlas.coastalShelf * 0.30 - atlas.deepOcean * 0.18 + sampleAtlasGlobalHeightBias(atlasRegionId, atlasRegionCore), 0, 1);
    out.mountainness = clamp(mountainness + sampleAtlasMountainnessBias(atlasRegionId, atlasRegionCore), 0, 1);
    out.oceanness = clamp(Math.max(oceanness, 1 - atlas.islandInterior + atlas.coastalShelf * 0.16 + atlas.deepOcean * 0.34), 0, 1);
    out.continentalness = continentalness;
    out.hills = hills;
    out.detail = detail;
    out.ridge = clamp(ridge + province.volcanicHeart * 0.18 + province.easternShardCoast * 0.12, 0, 1);
    out.basin = clamp(basin - province.southernSaltBasin * 0.42 - province.westWetlands * 0.18, -1, 1);
    out.channel = clamp(1 - Math.abs(fbm2D2(worldX * CHANNEL_SCALE, worldZ * CHANNEL_SCALE, this.channelSeed) * 2 - 1) + province.westWetlands * 0.16 + province.southernSaltBasin * 0.18, 0, 1);
    out.dune = clamp(1 - Math.abs(fbm2D2(worldX * DUNE_SCALE, worldZ * DUNE_SCALE, this.duneSeed) * 2 - 1) + province.southernSaltBasin * 0.24, 0, 1);
    out.mesa = clamp(smoothstep(0.54, 0.84, fbm2D2(worldX * MESA_SCALE, worldZ * MESA_SCALE, this.mesaSeed)) + province.ashRing * 0.22 + province.volcanicHeart * 0.28, 0, 1);
    out.grove = clamp(fbm2D3(worldX * GROVE_SCALE, worldZ * GROVE_SCALE, this.groveSeed) + province.westWetlands * 0.22 + province.northeastGrazelands * 0.10 - province.ashRing * 0.18, 0, 1);
    out.oldGrowth = clamp(fbm2D3(worldX * OLD_GROWTH_SCALE, worldZ * OLD_GROWTH_SCALE, this.oldGrowthSeed) + province.westWetlands * 0.20 - province.volcanicHeart * 0.26, 0, 1);
    out.orchard = clamp(fbm2D3(worldX * ORCHARD_SCALE, worldZ * ORCHARD_SCALE, this.orchardSeed) + province.northeastGrazelands * 0.22, 0, 1);
    out.desolation = clamp(
      fbm2D3(worldX * DESOLATION_SCALE, worldZ * DESOLATION_SCALE, this.desolationSeed)
        + province.volcanicHeart * 0.42
        + province.ashRing * 0.34
        + province.southernSaltBasin * 0.18
        - province.westWetlands * 0.22,
      0,
      1,
    );
    out.strata = clamp(fbm2D2(worldX * STRATA_SCALE, worldZ * STRATA_SCALE, this.strataSeed) + province.ashRing * 0.22 + province.volcanicHeart * 0.18, 0, 1);
    out.surfacePatch = clamp(fbm2D3(worldX * SURFACE_PATCH_SCALE, worldZ * SURFACE_PATCH_SCALE, this.surfacePatchSeed) + province.ashRing * 0.18 + province.southernSaltBasin * 0.12, 0, 1);
    out.surfaceGrain = clamp(fbm2D2(worldX * SURFACE_GRAIN_SCALE, worldZ * SURFACE_GRAIN_SCALE, this.surfaceGrainSeed) + province.ashRing * 0.16 + province.easternShardCoast * 0.14, 0, 1);
    out.scatter = clamp(fbm2D2(worldX * SURFACE_SCATTER_SCALE, worldZ * SURFACE_SCATTER_SCALE, this.surfaceScatterSeed) + province.ashRing * 0.12 + province.westWetlands * 0.10, 0, 1);
    out.peakness = clamp(peakness + province.volcanicHeart * 0.56 + province.ashRing * 0.10, 0, 1);
    out.regionId = atlasRegionId;
    out.secondaryRegionId = atlasSecondaryRegionId;
    out.regionStrength = atlasRegionStrength;
    out.regionBlend = atlas.primaryRegionId ? atlas.regionBlend : 1;
    out.regionBiomeId = atlasBiomeId;
    out.regionAmbientProfileId = atlas.ambientProfileId ?? province.ambientProfileId;
    out.regionVariantId = atlas.regionalVariantId ?? province.regionalVariantId;
    out.islandInterior = atlas.islandInterior;
    out.shorelineBand = atlas.shorelineBand;
    out.coastalShelf = atlas.coastalShelf;
    out.deepOcean = atlas.deepOcean;
    out.atlasRouteInfluence = atlas.routeInfluence;
    out.atlasRouteCore = atlas.routeCore;
    out.atlasRouteShoulder = atlas.routeShoulder;
    out.atlasCaveInfluence = atlas.caveInfluence;
    out.atlasCaveCore = atlas.caveCore;
    out.volcanicHeart = Math.max(province.volcanicHeart * 0.35, atlasRegionId === "red-mountain" ? atlasRegionCore : 0);
    out.ashRing = Math.max(
      province.ashRing * 0.35,
      atlasRegionId === "ashen-badlands" ? atlasRegionCore : atlasRegionId === "red-mountain" ? atlasRegionCore * 0.38 : 0,
    );
    out.westWetlands = Math.max(
      province.westWetlands * 0.35,
      atlasRegionId === "bitter-coast" ? atlasRegionCore : atlasRegionId === "inner-sea" ? atlasRegionCore * 0.30 : 0,
    );
    out.northeastGrazelands = Math.max(province.northeastGrazelands * 0.35, atlasRegionId === "grazelands" ? atlasRegionCore : 0);
    out.southernSaltBasin = Math.max(province.southernSaltBasin * 0.35, atlasRegionId === "salt-marsh-basin" ? atlasRegionCore : 0);
    out.easternShardCoast = Math.max(province.easternShardCoast * 0.35, atlasRegionId === "glass-shard-coast" ? atlasRegionCore : 0);
    return out;
  }

  private sampleCaveFields(worldX: number, worldZ: number): CaveFieldSample {
    const out = this.reusableCaveFields;
    out.caveRibbon = 1 - Math.abs(fbm2D2(worldX * CAVE_RIBBON_SCALE, worldZ * CAVE_RIBBON_SCALE, this.caveRibbonSeed) * 2 - 1);
    out.cavePocket = fbm2D3(worldX * CAVE_POCKET_SCALE, worldZ * CAVE_POCKET_SCALE, this.cavePocketSeed);
    out.caveDepth = fbm2D3(worldX * CAVE_DEPTH_SCALE, worldZ * CAVE_DEPTH_SCALE, this.caveDepthSeed);
    out.caveOpenings = fbm2D2(worldX * CAVE_OPENING_SCALE, worldZ * CAVE_OPENING_SCALE, this.caveOpeningSeed);
    return out;
  }

  private selectBaseBiomes(fields: SurfaceFieldSample): BaseBiomeBlendSelection {
    let primary = BASE_BIOMES[0]!;
    let secondary = BASE_BIOMES[1]!;
    let primaryScore = -1;
    let secondaryScore = -1;
    for (const biome of BASE_BIOMES) {
      const score = scoreBaseBiome(fields, biome);
      if (score > primaryScore) {
        secondary = primary;
        secondaryScore = primaryScore;
        primary = biome;
        primaryScore = score;
      } else if (score > secondaryScore) {
        secondary = biome;
        secondaryScore = score;
      }
    }
    const regionBaseBiome = BASE_BIOMES_BY_ID.get(fields.regionBiomeId as BaseBiomeId) ?? null;
    if (regionBaseBiome && fields.regionStrength > 0.58) {
      if (primary.id !== regionBaseBiome.id) {
        secondary = primary;
        secondaryScore = primaryScore;
        primary = regionBaseBiome;
        primaryScore = Math.max(primaryScore, secondaryScore, 0.001) * (1.12 + fields.regionStrength * 0.42);
      } else {
        primaryScore *= 1.10 + fields.regionStrength * 0.24;
      }
    }
    const total = primaryScore + secondaryScore;
    const out = this.reusableBaseBiomeBlend;
    out.primary = primary;
    out.secondary = secondary;
    out.primaryWeight = total <= 0 ? 1 : primaryScore / total;
    return out;
  }

  private selectBiomeClassification(
    fields: SurfaceFieldSample,
    baseBlend: BaseBiomeBlendSelection,
  ): BiomeClassificationSelection {
    const biomeCore = smoothstep(0.60, 0.88, baseBlend.primaryWeight);
    const flatness = saturate(1 - (
      fields.ridge * 0.7
      + Math.abs(fields.detail) * 1.4
      + Math.abs(fields.hills) * 0.55
      + fields.uplift * 0.25
    ));
    const wetLowlandHost = hostSetStrength(baseBlend.primary.id, baseBlend.secondary.id, baseBlend.primaryWeight, [
      "verdant",
      "savanna",
      "steppe",
    ]);
    const warmLushHost = hostSetStrength(baseBlend.primary.id, baseBlend.secondary.id, baseBlend.primaryWeight, [
      "verdant",
      "savanna",
      "highland",
    ]);
    const moorBloomHost = hostSetStrength(baseBlend.primary.id, baseBlend.secondary.id, baseBlend.primaryWeight, [
      "verdant",
      "highland",
      "moor",
    ]);
    const dryRuggedHost = hostSetStrength(baseBlend.primary.id, baseBlend.secondary.id, baseBlend.primaryWeight, [
      "badlands",
      "highland",
    ]);
    const dryLowlandHost = hostSetStrength(baseBlend.primary.id, baseBlend.secondary.id, baseBlend.primaryWeight, [
      "savanna",
      "steppe",
      "dunes",
    ]);
    const aridShardHost = hostSetStrength(baseBlend.primary.id, baseBlend.secondary.id, baseBlend.primaryWeight, [
      "dunes",
      "badlands",
      "highland",
      "tundra",
    ]);
    const marshStrength = avg5(
      wetLowlandHost,
      smoothstep(0.56, 0.82, fields.moisture),
      smoothstep(0.48, 0.80, fields.drainage + Math.max(0, -fields.basin) * 0.38),
      smoothstep(0.36, 0.76, fields.oceanness + Math.max(0, -fields.basin) * 0.42 + fields.channel * 0.12),
      smoothstep(0.22, 0.82, flatness),
    ) * smoothstep(0.34, 0.78, 1 - (fields.magic * 0.58 + fields.volcanism * 0.36));
    const fireflyStrength = avg5(
      wetLowlandHost,
      smoothstep(0.56, 0.80, fields.moisture),
      smoothstep(0.48, 0.74, fields.magic),
      smoothstep(0.42, 0.74, fields.grove + fields.channel * 0.25),
      smoothstep(0.20, 0.82, flatness),
    );
    const saltflatStrength = avg5(
      dryLowlandHost,
      smoothstep(0.34, 0.72, fields.oceanness + Math.max(0, -fields.basin) * 0.45 + fields.channel * 0.18),
      smoothstep(0.42, 0.74, 1 - fields.moisture),
      smoothstep(0.18, 0.82, flatness),
      smoothstep(0.36, 0.72, 1 - fields.globalHeight + fields.oceanness * 0.35),
    );
    const fernStrength = avg5(
      warmLushHost,
      smoothstep(0.50, 0.76, fields.temperature),
      smoothstep(0.56, 0.82, fields.moisture),
      smoothstep(0.42, 0.76, 1 - fields.drainage + Math.max(0, -fields.basin) * 0.55 + fields.channel * 0.15),
      smoothstep(0.24, 0.84, flatness),
    );
    const fungalStrength = avg5(
      moorBloomHost,
      smoothstep(0.46, 0.74, fields.magic),
      smoothstep(0.56, 0.82, fields.moisture),
      smoothstep(0.46, 0.76, fields.grove + fields.oldGrowth * 0.25),
      smoothstep(0.20, 0.82, flatness),
    );
    const emberStrength = saturate(
      dryRuggedHost
      * smoothstep(0.62, 0.84, fields.volcanism)
      * smoothstep(0.16, 0.58, 1 - fields.moisture),
    );
    const bloomStrength = saturate(
      moorBloomHost
      * smoothstep(0.54, 0.74, fields.magic)
      * smoothstep(0.38, 0.62, fields.moisture)
      * (0.55 + smoothstep(0.14, 0.66, 1 - fields.volcanism) * 0.45),
    );
    const shardlandsStrength = avg5(
      aridShardHost,
      smoothstep(0.40, 0.72, fields.magic + fields.volcanism * 0.20),
      smoothstep(0.46, 0.78, fields.volcanism + fields.ridge * 0.16),
      smoothstep(0.34, 0.72, 1 - fields.moisture),
      smoothstep(0.44, 0.78, fields.surfacePatch + fields.dune * 0.25 + fields.mesa * 0.20),
    );

    let biomeId: BiomeId = baseBlend.primary.id;
    let specialStrength = 0;
    if (marshStrength > 0.56) {
      biomeId = "marsh";
      specialStrength = marshStrength;
    }
    if (fireflyStrength > 0.56 && fireflyStrength > specialStrength) {
      biomeId = "firefly";
      specialStrength = fireflyStrength;
    }
    if (saltflatStrength > 0.78 && saltflatStrength > specialStrength) {
      biomeId = "saltflat";
      specialStrength = saltflatStrength;
    }
    if (fernStrength > 0.58 && fernStrength > specialStrength) {
      biomeId = "fern";
      specialStrength = fernStrength;
    }
    if (fungalStrength > 0.58 && fungalStrength > specialStrength) {
      biomeId = "fungal";
      specialStrength = fungalStrength;
    }
    if (emberStrength > 0.54 && emberStrength > specialStrength && fields.moisture < 0.58) {
      biomeId = "ember";
      specialStrength = emberStrength;
    }
    if (bloomStrength > 0.42 && bloomStrength > specialStrength) {
      biomeId = "bloom";
      specialStrength = bloomStrength;
    }
    if (shardlandsStrength > 0.76 && shardlandsStrength > specialStrength) {
      biomeId = "shardlands";
      specialStrength = shardlandsStrength;
    }
    if (fields.southernSaltBasin > 0.62 && saltflatStrength > 0.58 && saltflatStrength >= specialStrength * 0.86) {
      biomeId = "saltflat";
      specialStrength = Math.max(specialStrength, saltflatStrength, fields.southernSaltBasin);
    }
    if (fields.southernSaltBasin > 0.66 && (fields.surfacePatch > 0.56 || fields.dune > 0.52)) {
      biomeId = "dunes";
      specialStrength = 0;
    }
    if (fields.southernSaltBasin > 0.94 && fields.coastalShelf > 0.54) {
      biomeId = "saltflat";
      specialStrength = Math.max(specialStrength, fields.southernSaltBasin);
    }
    if (fields.southernSaltBasin > 0.82 && fields.strata > 0.72 && fields.dune > 0.42) {
      biomeId = "tundra";
      specialStrength = 0;
    }
    if (fields.westWetlands > 0.64 && marshStrength > 0.48 && marshStrength >= specialStrength * 0.78) {
      biomeId = fields.magic > 0.58 ? "fungal" : fields.magic > 0.50 ? "firefly" : "marsh";
      specialStrength = Math.max(specialStrength, marshStrength, fields.westWetlands);
    }
    if (fields.westWetlands > 0.78 && fields.drainage > 0.58 && fields.magic < 0.74) {
      biomeId = "marsh";
      specialStrength = Math.max(specialStrength, fields.westWetlands);
    }
    if (fields.westWetlands > 0.70 && fields.grove > 0.72 && fields.temperature > 0.42) {
      biomeId = "fern";
      specialStrength = Math.max(specialStrength, fields.westWetlands * 0.86);
    }
    if (fields.westWetlands > 0.92 && fields.drainage > 0.48) {
      biomeId = "marsh";
      specialStrength = Math.max(specialStrength, fields.westWetlands);
    }
    if (fields.easternShardCoast > 0.58 && shardlandsStrength > 0.56 && shardlandsStrength >= specialStrength * 0.82) {
      biomeId = "shardlands";
      specialStrength = Math.max(specialStrength, shardlandsStrength, fields.easternShardCoast);
    }
    if (fields.volcanicHeart > 0.44 && fields.volcanism > 0.62 && fields.moisture < 0.58) {
      biomeId = "ember";
      specialStrength = Math.max(specialStrength, fields.volcanicHeart, emberStrength);
    }
    const shouldLockRegionBiome = fields.regionStrength > 0.64 && (
      fields.regionBiomeId === "ember"
      || fields.regionBiomeId === "marsh"
      || fields.regionBiomeId === "saltflat"
      || fields.regionBiomeId === "shardlands"
      || fields.regionStrength > 0.88
    );
    const preserveRegionSubBiome = (
      (fields.regionBiomeId === "saltflat" && biomeId === "dunes" && fields.regionBlend > 0.22)
      || (fields.regionBiomeId === "marsh" && (biomeId === "fern" || biomeId === "fungal" || biomeId === "firefly"))
      || (fields.regionId === "west-gash" && (biomeId === "tundra" || biomeId === "highland"))
    );
    if (shouldLockRegionBiome && !preserveRegionSubBiome) {
      biomeId = fields.regionBiomeId;
      specialStrength = Math.max(specialStrength, fields.regionStrength);
    }
    const result = this.biomeClassificationState;
    result.biomeId = biomeId;
    result.specialStrength = specialStrength;
    result.biomeCore = Math.max(biomeCore, fields.regionStrength);
    return result;
  }

  private sampleSurfaceY(
    fields: SurfaceFieldSample,
    terrainProfile: {
      heightBias: number;
      reliefScale: number;
      ridgeScale: number;
      detailScale: number;
      basinScale: number;
      terraceScale: number;
      microRelief: number;
      snowLine: number;
    },
    biomeCore: number,
  ): number {
    const globalBaseHeight = this.seaLevel
      - 96
      + fields.globalHeight * 460
      - fields.oceanness * 82;
    const massifRelief = smoothstep(0.56, 0.86, fields.globalHeight)
      * smoothstep(0.50, 0.82, fields.uplift)
      * (18 + fields.mountainness * 112 + fields.ridge * 44);
    const peakProvince = smoothstep(0.54, 0.72, fields.peakness)
      * smoothstep(0.54, 0.82, fields.globalHeight)
      * smoothstep(0.56, 0.82, fields.uplift);
    const peakProvinceLift = peakProvince * (66 + fields.globalHeight * 202 + fields.uplift * 112);
    const peakCrown = peakProvince
      * smoothstep(0.62, 0.88, fields.ridge)
      * (28 + fields.mountainness * 68);
    const sharedRelief = fields.hills * (28 + fields.globalHeight * 36)
      + (fields.ridge * fields.ridge - 0.30) * (18 + fields.mountainness * 68)
      + fields.basin * 26
      + fields.detail * 8
      + massifRelief
      + peakProvinceLift
      + peakCrown;
    const localWeight = 0.06 + biomeCore * biomeCore * 0.74;
    const localHeight = terrainProfile.heightBias * localWeight
      + fields.hills * 88 * terrainProfile.reliefScale * localWeight
      + (fields.ridge * fields.ridge - 0.34) * 72 * terrainProfile.ridgeScale * localWeight
      + fields.detail * 16 * terrainProfile.detailScale * localWeight
      + fields.basin * 34 * terrainProfile.basinScale * localWeight
      + (fields.dune - 0.5) * 24 * terrainProfile.detailScale * localWeight
      + (fields.mesa - 0.5) * 20 * terrainProfile.terraceScale * localWeight;
    const preTerrace = globalBaseHeight + sharedRelief + localHeight;
    const terracedHeight = terrainProfile.terraceScale <= 0
      ? preTerrace
      : (() => {
          const terraceInfluence = terrainProfile.terraceScale * localWeight;
          const strataWarp = (fields.strata - 0.5) * 10 + (fields.surfacePatch - 0.5) * 4;
          const warpedTerraceInput = preTerrace + strataWarp * terraceInfluence;
          const warpedTerrace = Math.round(warpedTerraceInput / 8) * 8 - strataWarp * terraceInfluence * 0.72;
          return lerp(preTerrace, warpedTerrace, terraceInfluence);
        })();
    const microRelief = Math.round(
      (fields.surfaceGrain - 0.5) * terrainProfile.microRelief * (0.35 + biomeCore * 0.65),
    );
    const crustBreakup = sampleTerrainCrustBreakup(fields, terrainProfile, biomeCore);
    const islandCoastDelta = -Math.round(
      fields.coastalShelf * 96
        + fields.shorelineBand * 18
        + (1 - fields.islandInterior) * (220 + fields.deepOcean * 260),
    );
    const volcanicLift = Math.round(fields.volcanicHeart * (96 + fields.peakness * 104) + fields.ashRing * 18);
    const saltBasinDrop = Math.round(fields.southernSaltBasin * (18 + fields.coastalShelf * 28));
    const atlasRegionDelta = sampleAtlasRegionSurfaceDelta(fields);
    return Math.floor(clamp(
      terracedHeight + microRelief + crustBreakup + islandCoastDelta + volcanicLift + atlasRegionDelta - saltBasinDrop,
      8,
      this.maxYExclusive - 2,
    ));
  }

  private resolveSurfaceMaterials(
    biomeId: BiomeId,
    primary: BaseBiomeProfile,
    secondary: BaseBiomeProfile,
    primaryWeight: number,
    specialStrength: number,
    fields: SurfaceFieldSample,
    biomeCore: number,
    surfaceY: number,
  ): ResolvedSurfaceMaterials {
    const materials = this.resolvedSurfaceMaterials;
    const baseBiomeOverride = biomeId !== primary.id ? BASE_BIOMES_BY_ID.get(biomeId as BaseBiomeId) : null;
    if (baseBiomeOverride) {
      primary = baseBiomeOverride;
      primaryWeight = Math.max(primaryWeight, 0.82);
    }
    if (biomeId === primary.id) {
      const primarySurface = selectSurfaceMaterial(primary, fields, biomeCore, surfaceY);
      const primarySubsurface = selectSubsurfaceMaterial(primary, fields, biomeCore, surfaceY);
      const secondarySurface = selectSurfaceMaterial(secondary, fields, 1 - biomeCore * 0.5, surfaceY);
      const secondarySubsurface = selectSubsurfaceMaterial(secondary, fields, 1 - biomeCore * 0.5, surfaceY);
      materials.surfacePrimary = primarySurface;
      materials.surfaceSecondary = primary === secondary ? primarySurface : secondarySurface;
      materials.subsurfacePrimary = primarySubsurface;
      materials.subsurfaceSecondary = primary === secondary ? primarySubsurface : secondarySubsurface;
      materials.water = primary.water;
      materials.snow = primary.snow;
      materials.transitionThreshold = primary === secondary
        ? 1
        : clamp(0.56 + biomeCore * 0.24 + (primaryWeight - 0.5) * 0.30, 0.52, 0.90);
      return materials;
    }

    const special = SPECIAL_BIOMES[biomeId as SpecialBiomeId];
    const specialBiomeId = biomeId as SpecialBiomeId;
    const primarySurface = selectSpecialSurfaceMaterial(special, specialBiomeId, fields, biomeCore, specialStrength, surfaceY);
    const primarySubsurface = selectSpecialSubsurfaceMaterial(special, specialBiomeId, fields, biomeCore, specialStrength);
    const hostSurface = selectSurfaceMaterial(primary, fields, biomeCore, surfaceY);
    const hostSubsurface = selectSubsurfaceMaterial(primary, fields, biomeCore, surfaceY);
    const specialThreshold = special.softTransition
      ? clamp(0.58 + specialStrength * 0.24 + biomeCore * 0.10, 0.58, 0.92)
      : 1;
    materials.surfacePrimary = primarySurface;
    materials.surfaceSecondary = special.softTransition ? hostSurface : primarySurface;
    materials.subsurfacePrimary = primarySubsurface;
    materials.subsurfaceSecondary = special.softTransition ? hostSubsurface : primarySubsurface;
    materials.water = special.water;
    materials.snow = special.snow;
    materials.transitionThreshold = specialThreshold;
    return materials;
  }

  private resolveWaterTopY(
    biomeId: BiomeId,
    surfaceY: number,
    fields: SurfaceFieldSample,
    specialStrength: number,
    regionalVariantId: RegionalVariantId | null,
    regionalVariantStrength: number,
  ): number {
    let waterTopY = surfaceY < this.seaLevel ? this.seaLevel : NO_WATER;
    if (biomeId === "marsh") {
      const marshWetPocket = avg3(
        smoothstep(0.58, 0.84, fields.moisture),
        smoothstep(0.50, 0.82, fields.channel),
        smoothstep(0.38, 0.76, Math.max(0, -fields.basin) + fields.oceanness * 0.28),
      );
      if (marshWetPocket > 0.58) {
        const extraWaterDepth = 1 + Math.round(
          lerp(0, 2, clamp((marshWetPocket - 0.58) / 0.42, 0, 1) * (0.42 + specialStrength * 0.58)),
        );
        waterTopY = Math.max(waterTopY, surfaceY + extraWaterDepth);
      }
    } else if (biomeId === "firefly") {
      if (fields.channel > 0.68 || fields.basin < -0.14) {
        const extraWaterDepth = Math.round(lerp(1, 2, specialStrength));
        waterTopY = Math.max(waterTopY, surfaceY + extraWaterDepth);
      }
    } else if (biomeId === "saltflat") {
      if (fields.channel > 0.54 || fields.surfacePatch > 0.66) {
        waterTopY = Math.max(waterTopY, surfaceY + 1);
      }
    } else if (biomeId === "fern") {
      if (fields.channel > 0.74 && fields.basin < -0.08) {
        waterTopY = Math.max(waterTopY, surfaceY + 1);
      }
    } else if (biomeId === "fungal" && fields.magic > 0.70 && fields.moisture > 0.68) {
      waterTopY = Math.max(waterTopY, surfaceY + 1);
    } else if (biomeId === "dunes" && fields.channel > 0.78 && fields.basin < -0.08) {
      waterTopY = Math.max(waterTopY, surfaceY + 1);
    } else if (biomeId === "bloom" && fields.magic > 0.82 && fields.moisture > 0.62) {
      waterTopY = Math.max(waterTopY, surfaceY + 1);
    }
    if (regionalVariantId === "marsh_blackwater") {
      waterTopY = Math.max(waterTopY, surfaceY + Math.max(2, Math.round(lerp(2, 4, regionalVariantStrength))));
    } else if (regionalVariantId === "firefly_lantern") {
      waterTopY = Math.max(waterTopY, surfaceY + 2);
    } else if (regionalVariantId === "saltflat_mirror") {
      waterTopY = Math.max(waterTopY, surfaceY + 1);
    } else if (regionalVariantId === "fern_cenote") {
      waterTopY = Math.max(waterTopY, surfaceY + 2);
    } else if (regionalVariantId === "bloom_prism" && fields.magic > 0.76) {
      waterTopY = Math.max(waterTopY, surfaceY + 1);
    }
    return waterTopY;
  }

  private selectUndergroundBiome(
    biomeId: BiomeId,
    hostBiomeId: BaseBiomeId,
    fields: SurfaceFieldSample,
  ): UndergroundBiomeId {
    if (fields.atlasCaveInfluence > 0.38) {
      switch (fields.regionId) {
        case "red-mountain":
        case "ashen-badlands":
          return "basaltic";
        case "bitter-coast":
          return "rooted";
        case "salt-marsh-basin":
          return "saline";
        case "glass-shard-coast":
          return "crystalline";
        case "west-gash":
          return "granitic";
        default:
          break;
      }
    }
    if (biomeId === "saltflat") {
      return "saline";
    }
    if (biomeId === "fungal") {
      return "mycelial";
    }
    if (biomeId === "firefly" || biomeId === "moor") {
      return "peaty";
    }
    if (biomeId === "shardlands") {
      return fields.volcanism > 0.72 ? "basaltic" : "crystalline";
    }
    if (biomeId === "ember" || fields.volcanism > 0.78) {
      return "basaltic";
    }
    if (biomeId === "tundra" || (hostBiomeId === "highland" && fields.temperature < 0.3)) {
      return "froststone";
    }
    if (
      biomeId === "marsh"
      || biomeId === "fern"
      || biomeId === "bloom"
      || ((hostBiomeId === "verdant" || hostBiomeId === "savanna") && fields.moisture > 0.6)
    ) {
      return "rooted";
    }
    if (biomeId === "dunes" || (hostBiomeId === "steppe" && fields.moisture < 0.32)) {
      return "sandy";
    }
    if (biomeId === "badlands" || hostBiomeId === "badlands") {
      return "sedimentary";
    }
    return hostBiomeId === "highland" ? "granitic" : "sedimentary";
  }

  private resolveLandmark(
    worldX: number,
    worldZ: number,
    biomeId: BiomeId,
    undergroundBiomeId: UndergroundBiomeId,
    regionalVariantId: RegionalVariantId | null,
    surfaceY: number,
    waterTopY: number,
    fields: SurfaceFieldSample,
    out: MutableColumnState,
  ): LandmarkId | null {
    out.featureKind = FEATURE_NONE;
    out.featureHeight = 0;
    out.featureRadius = 0;
    out.featureExtra = 0;
    out.featureDeltaX = 0;
    out.featureDeltaZ = 0;
    out.featureMaterialPrimary = 0;
    out.featureMaterialSecondary = 0;
    out.featureMaterialAccent = 0;
    const setPiece = samplePilgrimRouteSetPiece(worldX, worldZ, biomeId, fields);
    if (setPiece) {
      out.featureDeltaX = setPiece.deltaAlong;
      out.featureDeltaZ = setPiece.deltaLateral;
      if (configureLandmarkFeature(setPiece.profile, surfaceY, waterTopY, fields, out)) {
        return setPiece.profile.id;
      }
    }
    const roster = selectPilgrimRouteRoster(worldX, worldZ, biomeId, fields)
      ?? selectLandmarkRoster(biomeId, undergroundBiomeId, regionalVariantId, fields);
    if (roster.length === 0) {
      return null;
    }

    const selectorCellX = Math.floor(worldX / 64);
    const selectorCellZ = Math.floor(worldZ / 64);
    const startIndex = Math.floor(
      hashNoise3D(selectorCellX, 19, selectorCellZ, this.featureSeed + biomeId.length) * roster.length,
    );
    for (let attempt = 0; attempt < roster.length; attempt += 1) {
      const profile = roster[(startIndex + attempt) % roster.length]!;
      const cellX = Math.floor(worldX / profile.cellSize);
      const cellZ = Math.floor(worldZ / profile.cellSize);
      const chance = hashNoise3D(cellX, 1, cellZ, this.featureSeed + profile.cellSize);
      if (chance > profile.chance) {
        continue;
      }
      const cellOriginX = cellX * profile.cellSize;
      const cellOriginZ = cellZ * profile.cellSize;
      const margin = profile.radius + 2;
      const span = Math.max(1, profile.cellSize - margin * 2);
      const anchorX = cellOriginX + margin + Math.floor(hashNoise3D(cellX, 2, cellZ, this.featureSeed + profile.radius) * span);
      const anchorZ = cellOriginZ + margin + Math.floor(hashNoise3D(cellX, 3, cellZ, this.featureSeed + profile.radius * 2) * span);
      const deltaX = worldX - anchorX;
      const deltaZ = worldZ - anchorZ;
      if (Math.abs(deltaX) > profile.radius || Math.abs(deltaZ) > profile.radius) {
        continue;
      }
      out.featureDeltaX = deltaX;
      out.featureDeltaZ = deltaZ;
      if (configureLandmarkFeature(profile, surfaceY, waterTopY, fields, out)) {
        return profile.id;
      }
    }
    return null;
  }

  private configureCaveState(
    biomeId: BiomeId,
    hostBiomeId: BaseBiomeId,
    undergroundBiomeId: UndergroundBiomeId,
    regionalVariantId: RegionalVariantId | null,
    surfaceY: number,
    waterTopY: number,
    fields: SurfaceFieldSample,
    caveFields: CaveFieldSample,
    biomePrimaryWeight: number,
    out: MutableColumnState,
  ): void {
    const caveInterior = 0.35 + smoothstep(0.68, 0.88, biomePrimaryWeight) * 0.65;
    const ruggedness = saturate(
      fields.uplift * 0.28
      + fields.ridge * 0.34
      + fields.mountainness * 0.28
      + Math.max(0, fields.detail) * 0.12
      + fields.peakness * 0.10,
    );
    const basinness = smoothstep(0.48, 0.84, -fields.basin);
    const subterraneanDryness = smoothstep(0.36, 0.76, 1 - fields.moisture + fields.drainage * 0.18);
    const waterSuppression = hasStandingWater(surfaceY, waterTopY) ? 0.44 : 1;
    const caveEntranceCliff = smoothstep(
      0.38,
      0.82,
      ruggedness
        + Math.max(0, fields.detail) * 0.24
        + fields.channel * 0.18
        + Math.max(0, fields.surfaceGrain - 0.5) * 0.14,
    );
    const deepAffinity = resolveDeepCaveAffinity(biomeId, hostBiomeId, undergroundBiomeId, regionalVariantId);
    const upperAffinity = resolveUpperCaveAffinity(biomeId, hostBiomeId, undergroundBiomeId, regionalVariantId);
    let mainField = avg4(
      smoothstep(0.52, 0.82, caveFields.caveRibbon),
      smoothstep(0.42, 0.78, caveFields.cavePocket),
      smoothstep(0.34, 0.72, caveFields.caveDepth + fields.drainage * 0.18 + basinness * 0.20),
      smoothstep(0.24, 0.72, subterraneanDryness + ruggedness * 0.18),
    );
    let upperField = avg3(
      smoothstep(0.56, 0.84, caveFields.caveOpenings),
      smoothstep(0.36, 0.76, ruggedness + basinness * 0.22 + fields.channel * 0.16),
      smoothstep(0.36, 0.74, caveFields.caveRibbon + caveFields.cavePocket * 0.18),
    );
    let entranceField = avg3(
      smoothstep(0.50, 0.82, caveFields.caveOpenings),
      smoothstep(0.42, 0.80, ruggedness + fields.channel * 0.22 + basinness * 0.12),
      smoothstep(0.34, 0.72, caveFields.caveRibbon + caveFields.cavePocket * 0.24),
    );
    if (fields.atlasCaveInfluence > 0) {
      const atlasCaveField = smoothstep(0.05, 0.92, fields.atlasCaveInfluence);
      mainField = Math.max(mainField, 0.48 + atlasCaveField * 0.42);
      upperField = Math.max(upperField, 0.34 + fields.atlasCaveCore * 0.54 + atlasCaveField * 0.18);
      entranceField = Math.max(entranceField, 0.30 + fields.atlasCaveCore * 0.56 + atlasCaveField * 0.16);
    }
    const mainStrength = saturate(mainField * (0.62 + deepAffinity * 0.58 + fields.atlasCaveInfluence * 0.34) * waterSuppression);
    const upperStrength = saturate(
      (
        upperField * (0.34 + upperAffinity * caveInterior * 1.22 + fields.atlasCaveInfluence * 0.28)
        + ruggedness * 0.18 * caveInterior
      ) * waterSuppression,
    );
    const boundaryFactor = resolveBiomeBoundarySuppression(biomePrimaryWeight);
    const entranceStrength = saturate(
      entranceField
      * caveEntranceCliff
      * (0.62 + upperAffinity * 1.45 + caveInterior * 0.90 + fields.atlasCaveInfluence * 0.45)
      * boundaryFactor
      * waterSuppression,
    );
    const mainCenterY = clamp(
      surfaceY - Math.round(lerp(20, 104, caveFields.caveDepth) + fields.globalHeight * 12 + deepAffinity * 14),
      36,
      surfaceY - 10,
    );
    const mainHalfHeight = clamp(
      Math.round(lerp(4, 18, caveFields.cavePocket) + deepAffinity * 8 + fields.magic * 2),
      0,
      30,
    );
    const upperDepth = clamp(
      Math.round(
        lerp(12, 2, caveFields.caveOpenings)
        + (1 - ruggedness) * 4
        + (1 - upperAffinity) * 3
        - caveInterior * 2,
      ),
      2,
      18,
    );
    const upperCenterY = clamp(surfaceY - upperDepth, 20, surfaceY - 2);
    const upperHalfHeight = clamp(
      Math.round(lerp(6, 14, caveFields.cavePocket) + upperAffinity * 6 + ruggedness * 4),
      0,
      22,
    );
    const entranceDepth = clamp(
      Math.round(
        lerp(10, 3, caveEntranceCliff)
        + (1 - caveFields.caveOpenings) * 2
        + (1 - upperAffinity) * 2
        - caveInterior,
      ),
      2,
      12,
    );
    const entranceCenterY = clamp(surfaceY - entranceDepth, 20, surfaceY - 2);
    const entranceHalfHeight = clamp(
      Math.round(lerp(5, 9, caveFields.cavePocket) + upperAffinity * 4 + ruggedness * 2 + caveEntranceCliff * 5),
      0,
      16,
    );

    out.caveMainField = mainField;
    out.caveMainStrength = mainStrength;
    out.caveMainCenterY = mainStrength >= 0.50 && mainHalfHeight > 0 ? mainCenterY : 0;
    out.caveMainHalfHeight = mainStrength >= 0.50 ? mainHalfHeight : 0;
    out.caveUpperField = upperField;
    out.caveUpperStrength = upperStrength;
    out.caveUpperCenterY = upperStrength >= 0.36 && upperHalfHeight > 0 ? upperCenterY : 0;
    out.caveUpperHalfHeight = upperStrength >= 0.36 ? upperHalfHeight : 0;
    out.caveEntranceField = entranceField;
    out.caveEntranceStrength = entranceStrength;
    out.caveEntranceCenterY = entranceStrength >= 0.30 && entranceHalfHeight > 0 ? entranceCenterY : 0;
    out.caveEntranceHalfHeight = entranceStrength >= 0.30 ? entranceHalfHeight : 0;
  }

  private writeChunkColumnState(
    scratch: ChunkGenerationScratch,
    columnIndex: number,
    state: MutableColumnState,
  ): void {
    scratch.surfaceY[columnIndex] = state.surfaceY;
    scratch.waterTopY[columnIndex] = state.waterTopY;
    scratch.surfacePrimary[columnIndex] = state.surfaceMaterialPrimary;
    scratch.surfaceSecondary[columnIndex] = state.surfaceMaterialSecondary;
    scratch.subsurfacePrimary[columnIndex] = state.subsurfacePrimary;
    scratch.subsurfaceSecondary[columnIndex] = state.subsurfaceSecondary;
    scratch.waterMaterial[columnIndex] = state.waterMaterial;
    scratch.snowMaterial[columnIndex] = state.snowMaterial;
    scratch.stoneMaterial[columnIndex] = state.stoneMaterial;
    scratch.deepStoneMaterial[columnIndex] = state.deepStoneMaterial;
    scratch.accentMaterial[columnIndex] = state.accentMaterial;
    scratch.transitionThreshold[columnIndex] = state.transitionThreshold;
    scratch.caveMainField[columnIndex] = state.caveMainField;
    scratch.caveMainStrength[columnIndex] = state.caveMainStrength;
    scratch.caveMainCenterY[columnIndex] = state.caveMainCenterY;
    scratch.caveMainHalfHeight[columnIndex] = state.caveMainHalfHeight;
    scratch.caveUpperField[columnIndex] = state.caveUpperField;
    scratch.caveUpperStrength[columnIndex] = state.caveUpperStrength;
    scratch.caveUpperCenterY[columnIndex] = state.caveUpperCenterY;
    scratch.caveUpperHalfHeight[columnIndex] = state.caveUpperHalfHeight;
    scratch.caveEntranceField[columnIndex] = state.caveEntranceField;
    scratch.caveEntranceStrength[columnIndex] = state.caveEntranceStrength;
    scratch.caveEntranceCenterY[columnIndex] = state.caveEntranceCenterY;
    scratch.caveEntranceHalfHeight[columnIndex] = state.caveEntranceHalfHeight;
    scratch.strataOffset[columnIndex] = state.strataOffset;
    scratch.worldXDiv3[columnIndex] = state.worldXDiv3;
    scratch.worldZDiv3[columnIndex] = state.worldZDiv3;
    scratch.ditherSeed[columnIndex] = state.ditherSeed;
    scratch.accentSeed[columnIndex] = state.accentSeed;
    scratch.featureKind[columnIndex] = state.featureKind;
    scratch.featureHeight[columnIndex] = state.featureHeight;
    scratch.featureRadius[columnIndex] = state.featureRadius;
    scratch.featureExtra[columnIndex] = state.featureExtra;
    scratch.featureDeltaX[columnIndex] = state.featureDeltaX;
    scratch.featureDeltaZ[columnIndex] = state.featureDeltaZ;
    scratch.featureMaterialPrimary[columnIndex] = state.featureMaterialPrimary;
    scratch.featureMaterialSecondary[columnIndex] = state.featureMaterialSecondary;
    scratch.featureMaterialAccent[columnIndex] = state.featureMaterialAccent;
  }

  private sampleMaterialFromColumn(context: MutableColumnState, worldY: number): number {
    const featureMaterial = sampleFeatureMaterial(
      context.featureKind,
      context.featureHeight,
      context.featureRadius,
      context.featureExtra,
      context.featureDeltaX,
      context.featureDeltaZ,
      context.featureMaterialPrimary,
      context.featureMaterialSecondary,
      context.featureMaterialAccent,
      context.surfaceY,
      worldY,
    );
    if (featureMaterial !== 0) {
      return featureMaterial;
    }
    if (worldY > context.surfaceY) {
      return context.waterTopY !== NO_WATER && worldY <= context.waterTopY ? context.waterMaterial : 0;
    }
    if (worldY === context.surfaceY) {
      if (
        sampleCaveSurfaceBreach(
          context.surfaceY,
          context.caveEntranceField,
          context.caveEntranceStrength,
          context.caveEntranceCenterY,
          context.caveEntranceHalfHeight,
        )
      ) {
        return 0;
      }
      return resolveSurfaceTransitionMaterial(
        context.surfaceMaterialPrimary,
        context.surfaceMaterialSecondary,
        context.transitionThreshold,
        context.worldXDiv3,
        Math.floor(worldY * ONE_THIRD),
        context.worldZDiv3,
        context.ditherSeed,
      );
    }
    if (sampleCaveVoid(
      context.surfaceY,
      worldY,
      context.caveMainField,
      context.caveMainStrength,
      context.caveMainCenterY,
      context.caveMainHalfHeight,
      context.caveUpperField,
      context.caveUpperStrength,
      context.caveUpperCenterY,
      context.caveUpperHalfHeight,
      context.caveEntranceField,
      context.caveEntranceStrength,
      context.caveEntranceCenterY,
      context.caveEntranceHalfHeight,
    )) {
      return 0;
    }
    if (worldY >= context.surfaceY - 4) {
      return resolveTransitionMaterial(
        context.subsurfacePrimary,
        context.subsurfaceSecondary,
        context.transitionThreshold,
        context.worldXDiv3,
        Math.floor(worldY * ONE_THIRD),
        context.worldZDiv3,
        context.ditherSeed + 17,
      );
    }
    if (worldY < 24) {
      return context.deepStoneMaterial;
    }
    const accentNoise = hashNoise3D(
      context.worldXDiv3,
      Math.floor(worldY * ONE_THIRD),
      context.worldZDiv3,
      context.accentSeed,
    );
    if (worldY < context.surfaceY - 18 && accentNoise > 0.992) {
      return context.accentMaterial;
    }
    const band = Math.abs(Math.floor(worldY * STRATA_BAND_SCALE + context.strataOffset)) % 3;
    if (band === 0) {
      return context.stoneMaterial;
    }
    if (band === 1) {
      return context.deepStoneMaterial;
    }
    return context.subsurfacePrimary;
  }
}

function createBaseBiome(
  id: BaseBiomeId,
  temperature: number,
  moisture: number,
  uplift: number,
  drainage: number,
  heightCenter: number,
  heightRange: number,
  heightBias: number,
  reliefScale: number,
  ridgeScale: number,
  detailScale: number,
  basinScale: number,
  terraceScale: number,
  microRelief: number,
  snowLine: number,
  surface: string,
  transitionSurface: string,
  surfaceVariant: string,
  surfaceAccent: string,
  surfaceRock: string,
  subsurface: string,
  subsurfaceVariant: string,
  water: string,
  snow: string,
): BaseBiomeProfile {
  return {
    id,
    temperature,
    moisture,
    uplift,
    drainage,
    heightCenter,
    heightRange,
    heightBias,
    reliefScale,
    ridgeScale,
    detailScale,
    basinScale,
    terraceScale,
    microRelief,
    snowLine,
    surface: hexColorToMaterial(surface),
    transitionSurface: hexColorToMaterial(transitionSurface),
    surfaceVariant: hexColorToMaterial(surfaceVariant),
    surfaceAccent: hexColorToMaterial(surfaceAccent),
    surfaceRock: hexColorToMaterial(surfaceRock),
    subsurface: hexColorToMaterial(subsurface),
    subsurfaceVariant: hexColorToMaterial(subsurfaceVariant),
    water: hexColorToMaterial(water),
    snow: hexColorToMaterial(snow),
  };
}

function createSpecialBiome(
  id: SpecialBiomeId,
  surface: string,
  transitionSurface: string,
  surfaceVariant: string,
  surfaceAccent: string,
  surfaceRock: string,
  subsurface: string,
  subsurfaceVariant: string,
  water: string,
  snow: string,
  softTransition: boolean,
): SpecialBiomeProfile {
  return {
    id,
    surface: hexColorToMaterial(surface),
    transitionSurface: hexColorToMaterial(transitionSurface),
    surfaceVariant: hexColorToMaterial(surfaceVariant),
    surfaceAccent: hexColorToMaterial(surfaceAccent),
    surfaceRock: hexColorToMaterial(surfaceRock),
    subsurface: hexColorToMaterial(subsurface),
    subsurfaceVariant: hexColorToMaterial(subsurfaceVariant),
    water: hexColorToMaterial(water),
    snow: hexColorToMaterial(snow),
    softTransition,
  };
}

function createUndergroundBiome(
  id: UndergroundBiomeId,
  stone: string,
  deepStone: string,
  accent: string,
): UndergroundBiomeProfile {
  return {
    id,
    stone: hexColorToMaterial(stone),
    deepStone: hexColorToMaterial(deepStone),
    accent: hexColorToMaterial(accent),
  };
}

function createLandmark(
  id: LandmarkId,
  cellSize: number,
  radius: number,
  chance: number,
  scale: number,
  variant: number,
): LandmarkProfile {
  return {
    id,
    cellSize,
    radius,
    chance,
    scale,
    variant,
  };
}

function landmarkPlacement(
  id: LandmarkId,
  overrides: Partial<Omit<LandmarkProfile, "id">> = {},
): LandmarkProfile {
  const base = LANDMARKS[id];
  return {
    id,
    cellSize: overrides.cellSize ?? base.cellSize,
    radius: overrides.radius ?? base.radius,
    chance: overrides.chance ?? base.chance,
    scale: overrides.scale ?? base.scale,
    variant: overrides.variant ?? base.variant,
  };
}

function createMutableColumnState(): MutableColumnState {
  return {
    regionId: "inner-sea",
    secondaryRegionId: "inner-sea",
    regionStrength: 0,
    regionAmbientProfileId: "silt-mist",
    biomeId: "verdant",
    hostBiomeId: "verdant",
    secondaryBiomeId: "steppe",
    undergroundBiomeId: "rooted",
    regionalVariantId: null,
    regionalVariantStrength: 0,
    landmarkId: null,
    temperature: 0,
    moisture: 0,
    uplift: 0,
    drainage: 0,
    volcanism: 0,
    magic: 0,
    globalHeight: 0,
    mountainness: 0,
    oceanness: 0,
    surfaceY: 0,
    waterTopY: NO_WATER,
    pilgrimRouteInfluence: 0,
    surfaceMaterialPrimary: 0,
    surfaceMaterialSecondary: 0,
    subsurfacePrimary: 0,
    subsurfaceSecondary: 0,
    waterMaterial: 0,
    snowMaterial: 0,
    stoneMaterial: 0,
    deepStoneMaterial: 0,
    accentMaterial: 0,
    transitionThreshold: 1,
    specialStrength: 0,
    caveMainField: 0,
    caveMainStrength: 0,
    caveMainCenterY: 0,
    caveMainHalfHeight: 0,
    caveUpperField: 0,
    caveUpperStrength: 0,
    caveUpperCenterY: 0,
    caveUpperHalfHeight: 0,
    caveEntranceField: 0,
    caveEntranceStrength: 0,
    caveEntranceCenterY: 0,
    caveEntranceHalfHeight: 0,
    strataOffset: 0,
    worldXDiv3: 0,
    worldZDiv3: 0,
    ditherSeed: 0,
    accentSeed: 0,
    featureKind: FEATURE_NONE,
    featureHeight: 0,
    featureRadius: 0,
    featureExtra: 0,
    featureDeltaX: 0,
    featureDeltaZ: 0,
    featureMaterialPrimary: 0,
    featureMaterialSecondary: 0,
    featureMaterialAccent: 0,
  };
}

function topYFromState(state: MutableColumnState): number {
  return Math.max(state.surfaceY, state.surfaceY + state.featureHeight + (state.featureKind === FEATURE_NONE ? 0 : 1));
}

function nullableWaterTopY(waterTopY: number): number | null {
  return waterTopY === NO_WATER ? null : waterTopY;
}

function columnSampleFromState(state: MutableColumnState): ProceduralColumnSample {
  return {
    biomeId: state.biomeId,
    hostBiomeId: state.hostBiomeId,
    undergroundBiomeId: state.undergroundBiomeId,
    regionalVariantId: state.regionalVariantId,
    landmarkId: state.landmarkId,
    surfaceY: state.surfaceY,
    topY: topYFromState(state),
    waterTopY: nullableWaterTopY(state.waterTopY),
    pilgrimRouteInfluence: state.pilgrimRouteInfluence,
    surfaceMaterial: state.surfaceMaterialPrimary,
  };
}

function surfaceColumnSampleFromState(state: MutableColumnState): ProceduralSurfaceColumnSample {
  return {
    biomeId: state.biomeId,
    surfaceY: state.surfaceY,
    topY: topYFromState(state),
    waterTopY: nullableWaterTopY(state.waterTopY),
    pilgrimRouteInfluence: state.pilgrimRouteInfluence,
    surfaceMaterial: state.surfaceMaterialPrimary,
    waterMaterial: state.waterTopY === NO_WATER ? null : state.waterMaterial,
  };
}

function scoreBaseBiome(fields: SurfaceFieldSample, biome: BaseBiomeProfile): number {
  let score = (
    scoreField(fields.temperature, biome.temperature, 0.28) * 0.20
    + scoreField(fields.moisture, biome.moisture, 0.30) * 0.20
    + scoreField(fields.uplift, biome.uplift, 0.32) * 0.12
    + scoreField(fields.drainage, biome.drainage, 0.34) * 0.10
    + scoreField(fields.globalHeight, biome.heightCenter, biome.heightRange) * 0.30
    + scoreField(fields.magic, biome.id === "highland" ? 0.56 : 0.44, 0.56) * 0.04
    + scoreField(fields.volcanism, biome.id === "badlands" ? 0.52 : 0.32, 0.72) * 0.04
  );
  switch (biome.id) {
    case "verdant":
      score *= 0.72 + smoothstep(0.52, 0.80, fields.moisture) * 0.48;
      break;
    case "savanna":
      score *= 0.62
        + smoothstep(0.58, 0.84, fields.temperature) * 0.38
        + scoreField(fields.moisture, 0.54, 0.18) * 0.20;
      break;
    case "steppe":
      score *= 0.68 + scoreField(fields.moisture, 0.42, 0.18) * 0.32;
      break;
    case "dunes":
      score *= 0.40
        + smoothstep(0.60, 0.86, fields.temperature) * 0.62
        + smoothstep(0.50, 0.82, 1 - fields.moisture) * 0.50;
      break;
    case "badlands":
      score *= 0.58
        + smoothstep(0.46, 0.76, fields.uplift) * 0.34
        + smoothstep(0.46, 0.78, fields.volcanism) * 0.22;
      break;
    case "highland":
      score *= 0.52 + smoothstep(0.58, 0.82, fields.uplift) * 0.42;
      break;
    case "moor":
      score *= 0.62
        + smoothstep(0.54, 0.82, fields.moisture) * 0.26
        + smoothstep(0.54, 0.84, 1 - fields.drainage) * 0.32;
      break;
    case "tundra":
      score *= 0.50
        + smoothstep(0.46, 0.78, fields.uplift) * 0.24
        + smoothstep(0.40, 0.82, 1 - fields.temperature) * 0.54;
      break;
  }
  score *= scoreIslandProvinceBaseBiomeBias(fields, biome.id);
  return score;
}

function scoreIslandProvinceBaseBiomeBias(fields: SurfaceFieldSample, biomeId: BaseBiomeId): number {
  let bias = 0.82;
  switch (biomeId) {
    case "badlands":
      bias += fields.ashRing * 0.72 + fields.volcanicHeart * 0.44 + fields.easternShardCoast * 0.14;
      break;
    case "dunes":
      bias += fields.southernSaltBasin * 0.48 + fields.coastalShelf * 0.10;
      break;
    case "savanna":
      bias += fields.northeastGrazelands * 0.54 + fields.southernSaltBasin * 0.10;
      break;
    case "steppe":
      bias += fields.northeastGrazelands * 0.42 + fields.ashRing * 0.12;
      break;
    case "moor":
      bias += fields.westWetlands * 0.40 + fields.coastalShelf * 0.14;
      break;
    case "verdant":
      bias += fields.westWetlands * 0.26 + fields.northeastGrazelands * 0.18 - fields.ashRing * 0.34;
      break;
    case "highland":
      bias += fields.volcanicHeart * 0.20 + fields.easternShardCoast * 0.22;
      break;
    case "tundra":
      bias += fields.volcanicHeart * 0.08 + fields.coastalShelf * 0.06;
      break;
  }
  return clamp(bias, 0.28, 1.76);
}

function blendTerrainProfile(primary: BaseBiomeProfile, secondary: BaseBiomeProfile, primaryWeight: number): {
  heightBias: number;
  reliefScale: number;
  ridgeScale: number;
  detailScale: number;
  basinScale: number;
  terraceScale: number;
  microRelief: number;
  snowLine: number;
} {
  const secondaryWeight = 1 - primaryWeight;
  return {
    heightBias: primary.heightBias * primaryWeight + secondary.heightBias * secondaryWeight,
    reliefScale: primary.reliefScale * primaryWeight + secondary.reliefScale * secondaryWeight,
    ridgeScale: primary.ridgeScale * primaryWeight + secondary.ridgeScale * secondaryWeight,
    detailScale: primary.detailScale * primaryWeight + secondary.detailScale * secondaryWeight,
    basinScale: primary.basinScale * primaryWeight + secondary.basinScale * secondaryWeight,
    terraceScale: primary.terraceScale * primaryWeight + secondary.terraceScale * secondaryWeight,
    microRelief: primary.microRelief * primaryWeight + secondary.microRelief * secondaryWeight,
    snowLine: primary.snowLine * primaryWeight + secondary.snowLine * secondaryWeight,
  };
}

function hostSetStrength(
  primary: BaseBiomeId,
  secondary: BaseBiomeId,
  primaryWeight: number,
  ids: readonly BaseBiomeId[],
): number {
  let strength = 0;
  if (ids.includes(primary)) {
    strength += primaryWeight;
  }
  if (ids.includes(secondary)) {
    strength += 1 - primaryWeight;
  }
  return strength;
}

function resolveHostBiomeId(
  biomeId: BiomeId,
  primary: BaseBiomeId,
  secondary: BaseBiomeId,
): BaseBiomeId {
  if (biomeId === "marsh") {
    return primary === "verdant" || primary === "savanna" || primary === "steppe"
      ? primary
      : secondary === "verdant" || secondary === "savanna" || secondary === "steppe"
      ? secondary
      : "verdant";
  }
  if (biomeId === "firefly") {
    return primary === "verdant" || primary === "savanna" || primary === "steppe" || primary === "moor" || primary === "tundra"
      ? primary
      : secondary === "verdant" || secondary === "savanna" || secondary === "steppe" || secondary === "moor" || secondary === "tundra"
      ? secondary
      : "moor";
  }
  if (biomeId === "saltflat") {
    return primary === "savanna" || primary === "steppe" || primary === "dunes"
      ? primary
      : secondary === "savanna" || secondary === "steppe" || secondary === "dunes"
      ? secondary
      : "dunes";
  }
  if (biomeId === "fern") {
    return primary === "verdant" || primary === "savanna" || primary === "highland"
      ? primary
      : secondary === "verdant" || secondary === "savanna" || secondary === "highland"
      ? secondary
      : "verdant";
  }
  if (biomeId === "fungal") {
    return primary === "verdant" || primary === "highland" || primary === "moor"
      ? primary
      : secondary === "verdant" || secondary === "highland" || secondary === "moor"
      ? secondary
      : "moor";
  }
  if (biomeId === "ember") {
    return primary === "badlands" || primary === "highland" || primary === "steppe" || primary === "savanna"
      ? primary
      : secondary === "badlands" || secondary === "highland" || secondary === "steppe" || secondary === "savanna"
      ? secondary
      : "badlands";
  }
  if (biomeId === "bloom") {
    return primary === "verdant" || primary === "highland" || primary === "moor"
      ? primary
      : secondary === "verdant" || secondary === "highland" || secondary === "moor"
      ? secondary
      : "verdant";
  }
  if (biomeId === "shardlands") {
    return primary === "dunes" || primary === "badlands" || primary === "highland" || primary === "tundra"
      ? primary
      : secondary === "dunes" || secondary === "badlands" || secondary === "highland" || secondary === "tundra"
      ? secondary
      : "dunes";
  }
  return primary;
}

function resolveBiomeBoundarySuppression(biomePrimaryWeight: number): number {
  return 0.25 + smoothstep(0.72, 0.92, biomePrimaryWeight) * 0.75;
}

function selectSurfaceMaterial(
  biome: BaseBiomeProfile,
  fields: SurfaceFieldSample,
  biomeCore: number,
  surfaceY: number,
): number {
  return pickSurfaceMaterial(
    biome.surface,
    biome.transitionSurface,
    biome.surfaceVariant,
    biome.surfaceAccent,
    biome.surfaceRock,
    fields,
    biomeCore,
    surfaceY,
    0,
  );
}

function selectSubsurfaceMaterial(
  biome: BaseBiomeProfile,
  fields: SurfaceFieldSample,
  biomeCore: number,
  surfaceY: number,
): number {
  return pickSubsurfaceMaterial(
    biome.subsurface,
    biome.subsurfaceVariant,
    biome.surfaceRock,
    fields,
    biomeCore,
    surfaceY,
    0,
  );
}

function selectSpecialSurfaceMaterial(
  biome: SpecialBiomeProfile,
  biomeId: SpecialBiomeId,
  fields: SurfaceFieldSample,
  biomeCore: number,
  specialStrength: number,
  surfaceY: number,
): number {
  const extraBias = biomeId === "ember"
    ? 0.24 + specialStrength * 0.34
    : biomeId === "marsh"
    ? -0.10 + fields.moisture * 0.18
    : fields.magic * 0.18 + specialStrength * 0.16;
  return pickSurfaceMaterial(
    biome.surface,
    biome.transitionSurface,
    biome.surfaceVariant,
    biome.surfaceAccent,
    biome.surfaceRock,
    fields,
    biomeCore,
    surfaceY,
    extraBias,
  );
}

function selectSpecialSubsurfaceMaterial(
  biome: SpecialBiomeProfile,
  biomeId: SpecialBiomeId,
  fields: SurfaceFieldSample,
  biomeCore: number,
  specialStrength: number,
): number {
  const extraBias = biomeId === "ember"
    ? 0.20 + specialStrength * 0.28
    : biomeId === "bloom"
    ? fields.magic * 0.14
    : fields.moisture * 0.10;
  return pickSubsurfaceMaterial(
    biome.subsurface,
    biome.subsurfaceVariant,
    biome.surfaceRock,
    fields,
    biomeCore,
    0,
    extraBias,
  );
}

function pickSurfaceMaterial(
  surface: number,
  transitionSurface: number,
  surfaceVariant: number,
  surfaceAccent: number,
  surfaceRock: number,
  fields: SurfaceFieldSample,
  biomeCore: number,
  surfaceY: number,
  extraBias: number,
): number {
  const shoreBias = smoothstep(
    0.32,
    0.84,
    fields.oceanness + fields.channel * 0.30 + Math.max(0, 0.46 - fields.globalHeight) * 0.55,
  );
  const rockyBias = smoothstep(
    0.44,
    0.92,
    fields.mountainness + Math.max(0, surfaceY - 1540) * 0.0012 + extraBias,
  );
  const patchBias = smoothstep(0.54, 0.84, fields.surfacePatch);
  const accentBias = smoothstep(0.58, 0.90, fields.surfaceGrain + fields.scatter * 0.32 + extraBias * 0.35);
  if (rockyBias > 0.72 && fields.surfaceGrain > 0.48) {
    return surfaceRock;
  }
  if (shoreBias > 0.62 && fields.surfacePatch < 0.60) {
    return transitionSurface;
  }
  if (accentBias > 0.80 && biomeCore > 0.44) {
    return surfaceAccent;
  }
  if (patchBias > 0.70 || fields.scatter > 0.70) {
    return surfaceVariant;
  }
  if (fields.surfacePatch < 0.18 && shoreBias > 0.42) {
    return transitionSurface;
  }
  return surface;
}

function pickSubsurfaceMaterial(
  subsurface: number,
  subsurfaceVariant: number,
  surfaceRock: number,
  fields: SurfaceFieldSample,
  biomeCore: number,
  surfaceY: number,
  extraBias: number,
): number {
  const rockyBias = smoothstep(
    0.48,
    0.90,
    fields.mountainness + Math.max(0, surfaceY - 1560) * 0.001 + extraBias,
  );
  if (rockyBias > 0.80 && biomeCore > 0.40) {
    return surfaceRock;
  }
  if (fields.surfacePatch > 0.62 || fields.surfaceGrain > 0.72 || fields.scatter > 0.66) {
    return subsurfaceVariant;
  }
  return subsurface;
}

function createPilgrimRouteBand(
  startMetersX: number,
  startMetersZ: number,
  headingDegrees: number,
  lengthMeters: number,
  halfWidthMeters: number,
): PilgrimRouteBand {
  const heading = headingDegrees * Math.PI / 180;
  return {
    startX: startMetersX * WORLD_UNITS_PER_METER,
    startZ: startMetersZ * WORLD_UNITS_PER_METER,
    directionX: Math.cos(heading),
    directionZ: Math.sin(heading),
    length: lengthMeters * WORLD_UNITS_PER_METER,
    halfWidth: halfWidthMeters * WORLD_UNITS_PER_METER,
  };
}

function samplePilgrimRouteSurfaceInfluence(
  worldX: number,
  worldZ: number,
  biomeId: BiomeId,
  fields: SurfaceFieldSample,
): PilgrimRouteSurfaceInfluence | null {
  const wetlandRoute = isWetlandPilgrimRouteField(fields);
  const atlasRoute = fields.atlasRouteInfluence > 0.08 && fields.atlasRouteCore > 0.64;
  if (
    !wetlandRoute
    && !atlasRoute
    && (
      biomeId === "verdant"
      || biomeId === "fern"
      || biomeId === "bloom"
      || (biomeId === "highland" && fields.oldGrowth > 0.62 && fields.moisture > 0.48)
    )
  ) {
    return null;
  }

  let best: PilgrimRouteSurfaceInfluence | null = atlasRoute
    ? {
        core: fields.atlasRouteCore,
        shoulder: fields.atlasRouteShoulder,
        fracture: saturate(
          fields.atlasRouteInfluence * 0.16
            + fields.atlasRouteShoulder * 0.28
            + fields.surfacePatch * 0.22
            + fields.strata * 0.18
            + fields.desolation * 0.16,
        ),
        lateralRatio: fields.atlasRouteCore > 0 ? 0 : 0.72,
      }
    : null;
  for (const band of PILGRIM_ROUTE_BANDS) {
    const route = samplePilgrimRouteCoordinates(worldX, worldZ, band, fields);
    if (!route) {
      continue;
    }

    const core = 1 - smoothstep(0.08, 0.38, route.lateralRatio);
    const shoulder = smoothstep(0.22, 0.88, route.lateralRatio) * (1 - smoothstep(0.84, 1.02, route.lateralRatio));
    const worldXDiv12 = Math.floor(worldX / 12);
    const worldZDiv12 = Math.floor(worldZ / 12);
    const diagonalA = diagonalStripeStrength(worldXDiv12, worldZDiv12, 83, 19, 5, 3);
    const diagonalB = diagonalStripeStrength(worldXDiv12, worldZDiv12, 131, 29, -4, 7);
    const crackField = Math.max(
      smoothstep(0.70, 0.96, diagonalA),
      smoothstep(0.76, 0.98, diagonalB) * 0.86,
    );
    const harshness = avg3(
      smoothstep(0.42, 0.82, fields.surfacePatch),
      smoothstep(0.40, 0.80, fields.scatter),
      smoothstep(0.42, 0.82, fields.strata + fields.desolation * 0.24),
    );
    const fracture = saturate(crackField * (0.42 + harshness * 0.58) + shoulder * harshness * 0.26);
    const candidate = { core, shoulder, fracture, lateralRatio: route.lateralRatio };
    if (!best || candidate.core + candidate.shoulder > best.core + best.shoulder) {
      best = candidate;
    }
  }
  return best && best.core + best.shoulder > 0.08 ? best : null;
}

function samplePilgrimRouteCoordinates(
  worldX: number,
  worldZ: number,
  band: PilgrimRouteBand,
  fields: SurfaceFieldSample,
): PilgrimRouteCoordinates | null {
  const deltaX = worldX - band.startX;
  const deltaZ = worldZ - band.startZ;
  const along = deltaX * band.directionX + deltaZ * band.directionZ;
  if (along < -(band.halfWidth + PILGRIM_ROUTE_WARP_MAX) || along > band.length + band.halfWidth + PILGRIM_ROUTE_WARP_MAX) {
    return null;
  }
  const rawLateral = deltaX * -band.directionZ + deltaZ * band.directionX;
  const lateral = rawLateral - samplePilgrimRouteLateralWarp(along, band, fields);
  const lateralRatio = Math.abs(lateral) / band.halfWidth;
  if (lateralRatio > 1) {
    return null;
  }
  return { along, lateral, lateralRatio };
}

function samplePilgrimRouteLateralWarp(
  along: number,
  band: PilgrimRouteBand,
  fields: SurfaceFieldSample,
): number {
  const edgeFade = smoothstep(0, PILGRIM_ROUTE_WARP_FADE_DISTANCE, along)
    * (1 - smoothstep(band.length - PILGRIM_ROUTE_WARP_FADE_DISTANCE, band.length, along));
  if (edgeFade <= 0) {
    return 0;
  }
  const alongMeters = along / WORLD_UNITS_PER_METER;
  const seed = band.startX * 0.000_037 + band.startZ * 0.000_053 + band.directionX * 11.7 + band.directionZ * 17.3;
  const broadBend = Math.sin(alongMeters * 0.014 + seed) * 0.56
    + Math.sin(alongMeters * 0.033 + seed * 1.61) * 0.26;
  const groundDrift = (fields.surfaceGrain - 0.5) * 0.30
    + (fields.strata - 0.5) * 0.22
    + (fields.scatter - 0.5) * 0.18;
  return clamp((broadBend + groundDrift) * PILGRIM_ROUTE_WARP_MAX, -PILGRIM_ROUTE_WARP_MAX, PILGRIM_ROUTE_WARP_MAX) * edgeFade;
}

function samplePilgrimRouteSurfaceDelta(
  route: PilgrimRouteSurfaceInfluence,
  fields: SurfaceFieldSample,
  biomeCore: number,
): number {
  const centerWear = route.core * (0.8 + fields.strata * 1.25);
  const shoulderCollapse = route.shoulder * (0.8 + route.fracture * 2.2 + fields.scatter * 0.55);
  const raisedBrokenSlab = route.fracture > 0.68 && route.core > 0.22 ? 1 : 0;
  const atlasConform = fields.atlasRouteInfluence * (1.2 + fields.atlasRouteCore * 2.4 + fields.atlasRouteShoulder * 1.1);
  return -Math.round((centerWear + shoulderCollapse + atlasConform) * (0.42 + biomeCore * 0.58)) + raisedBrokenSlab;
}

function applyPilgrimRouteSurfaceMaterials(
  materials: ResolvedSurfaceMaterials,
  route: PilgrimRouteSurfaceInfluence,
  fields: SurfaceFieldSample,
): void {
  const desolateRoute = fields.desolation > 0.54 || fields.volcanism > 0.54;
  const routeDeposit = route.fracture + route.core * 0.34 + route.shoulder * 0.22;
  if (route.shoulder > route.core + 0.12 && routeDeposit > 0.50) {
    materials.surfacePrimary = desolateRoute ? PILGRIM_ROUTE_DUST_MATERIAL : PILGRIM_ROUTE_WORN_MATERIAL;
    materials.surfaceSecondary = route.fracture > 0.46 ? PILGRIM_ROUTE_DARK_MATERIAL : PILGRIM_ROUTE_CORE_MATERIAL;
    materials.transitionThreshold = clamp(0.50 + route.fracture * 0.18, 0.50, 0.72);
    return;
  }
  if (route.core > 0.28 || routeDeposit > 0.64) {
    materials.surfacePrimary = route.fracture > 0.72 ? PILGRIM_ROUTE_DARK_MATERIAL : PILGRIM_ROUTE_CORE_MATERIAL;
    materials.surfaceSecondary = route.fracture > 0.50
      ? (desolateRoute ? PILGRIM_ROUTE_DUST_MATERIAL : PILGRIM_ROUTE_SALT_MATERIAL)
      : PILGRIM_ROUTE_WORN_MATERIAL;
    materials.transitionThreshold = clamp(0.58 + route.core * 0.16 - route.fracture * 0.10, 0.48, 0.78);
  }
}

function selectPilgrimRouteRoster(
  worldX: number,
  worldZ: number,
  biomeId: BiomeId,
  fields: SurfaceFieldSample,
): readonly LandmarkProfile[] | null {
  const wetlandRoute = isWetlandPilgrimRouteField(fields);
  if (
    !wetlandRoute
    && (
      biomeId === "verdant"
      || biomeId === "fern"
      || biomeId === "bloom"
      || (biomeId === "highland" && fields.oldGrowth > 0.62 && fields.moisture > 0.48)
    )
  ) {
    return null;
  }
  for (const band of PILGRIM_ROUTE_BANDS) {
    const route = samplePilgrimRouteCoordinates(worldX, worldZ, band, fields);
    if (!route) {
      continue;
    }
    if (route.lateralRatio <= 0.86) {
      if (fields.easternShardCoast > 0.48) {
        return PILGRIM_ROUTE_GLASS_SHARD_LANDMARKS;
      }
      if (fields.southernSaltBasin > 0.48) {
        return PILGRIM_ROUTE_SALT_BASIN_LANDMARKS;
      }
      if (fields.regionId === "west-gash") {
        return PILGRIM_ROUTE_WEST_GASH_LANDMARKS;
      }
      if (fields.northeastGrazelands > 0.48) {
        return PILGRIM_ROUTE_GRAZELANDS_LANDMARKS;
      }
      if (fields.westWetlands > 0.48 || fields.regionId === "bitter-coast") {
        return PILGRIM_ROUTE_WETLAND_LANDMARKS;
      }
    }
    const brokenRouteEdge = route.lateralRatio > 0.52 && (fields.surfacePatch > 0.42 || fields.scatter > 0.46);
    const routeCore = route.lateralRatio <= 0.72 && fields.desolation + fields.ridge + fields.strata > 1.20;
    if (brokenRouteEdge || routeCore) {
      return PILGRIM_ROUTE_SKYLINE_LANDMARKS;
    }
  }
  return null;
}

function samplePilgrimRouteSetPiece(
  worldX: number,
  worldZ: number,
  biomeId: BiomeId,
  fields: SurfaceFieldSample,
): PilgrimRouteSetPiece | null {
  const wetlandRoute = isWetlandPilgrimRouteField(fields);
  if (!wetlandRoute && (biomeId === "verdant" || biomeId === "bloom")) {
    return null;
  }
  let best: PilgrimRouteSetPiece | null = null;
  let bestDistance = Number.POSITIVE_INFINITY;
  for (let bandIndex = 0; bandIndex < PILGRIM_ROUTE_BANDS.length; bandIndex += 1) {
    const band = PILGRIM_ROUTE_BANDS[bandIndex]!;
    const deltaX = worldX - band.startX;
    const deltaZ = worldZ - band.startZ;
    const along = deltaX * band.directionX + deltaZ * band.directionZ;
    const earlyWetlandCadence = wetlandRoute && bandIndex === 8;
    const setPieceStart = earlyWetlandCadence ? 48 * WORLD_UNITS_PER_METER : PILGRIM_ROUTE_SET_PIECE_START;
    const setPieceSpacing = earlyWetlandCadence ? 180 * WORLD_UNITS_PER_METER : PILGRIM_ROUTE_SET_PIECE_SPACING;
    if (
      along < setPieceStart - band.halfWidth
      || along > band.length - PILGRIM_ROUTE_SET_PIECE_END_MARGIN + band.halfWidth
    ) {
      continue;
    }
    const rawLateral = deltaX * -band.directionZ + deltaZ * band.directionX;
    const lateral = rawLateral - samplePilgrimRouteLateralWarp(along, band, fields);
    const anchorIndex = Math.max(0, Math.round((along - setPieceStart) / setPieceSpacing));
    const anchorAlong = setPieceStart + anchorIndex * setPieceSpacing;
    if (anchorAlong > band.length - PILGRIM_ROUTE_SET_PIECE_END_MARGIN) {
      continue;
    }
    const side = (anchorIndex + bandIndex) % 2 === 0 ? 1 : -1;
    const anchorLateral = side * band.halfWidth * 0.62;
    const deltaAlong = along - anchorAlong;
    const deltaLateral = lateral - anchorLateral;
    const profile = selectPilgrimRouteSetPieceProfile(anchorIndex, bandIndex, fields);
    const radius = profile.radius + 3;
    if (Math.abs(deltaAlong) > radius || Math.abs(deltaLateral) > radius) {
      continue;
    }
    const distance = Math.hypot(deltaAlong, deltaLateral);
    if (distance < bestDistance) {
      bestDistance = distance;
      best = { profile, deltaAlong, deltaLateral };
    }
  }
  return best;
}

function isWetlandPilgrimRouteField(fields: SurfaceFieldSample): boolean {
  return fields.regionId === "bitter-coast" || fields.westWetlands > 0.48;
}

function selectPilgrimRouteSetPieceProfile(
  anchorIndex: number,
  bandIndex: number,
  fields: SurfaceFieldSample,
): LandmarkProfile {
  const sequence = (anchorIndex + bandIndex * 3) % 8;
  if (fields.easternShardCoast > 0.48) {
    switch (sequence) {
      case 0:
      case 5:
        return landmarkPlacement("glass_cairn", { chance: 1, scale: 1.34, cellSize: 1, radius: 6, variant: 2 });
      case 1:
      case 6:
        return landmarkPlacement("crystal_cluster", { chance: 1, scale: 1.30, cellSize: 1, radius: 7, variant: 2 });
      case 2:
        return landmarkPlacement("salt_spire", { chance: 1, scale: 1.30, cellSize: 1, radius: 6, variant: 2 });
      case 3:
        return landmarkPlacement("velothi_shrine", { chance: 1, scale: 1.20, cellSize: 1, radius: 6 });
      default:
        return landmarkPlacement("pilgrim_cairn", { chance: 1, scale: 1.18, cellSize: 1, radius: 5 });
    }
  }
  if (fields.southernSaltBasin > 0.48) {
    switch (sequence) {
      case 0:
        return landmarkPlacement("salt_spire", { chance: 1, scale: 1.90, cellSize: 1, radius: 8, variant: 2 });
      case 5:
        return landmarkPlacement("old_road_causeway", { chance: 1, scale: 1.30, cellSize: 1, radius: 17 });
      case 1:
      case 6:
        return landmarkPlacement("ash_obelisk", { chance: 1, scale: 1.24, cellSize: 1, radius: 8 });
      case 2:
        return landmarkPlacement("glass_cairn", { chance: 1, scale: 1.22, cellSize: 1, radius: 6, variant: 2 });
      case 3:
        return landmarkPlacement("crystal_cluster", { chance: 1, scale: 1.16, cellSize: 1, radius: 7, variant: 2 });
      default:
        return landmarkPlacement("pilgrim_lantern", { chance: 1, scale: 1.28, cellSize: 1, radius: 5 });
    }
  }
  if (fields.regionId === "west-gash") {
    switch (sequence) {
      case 0:
      case 4:
        return landmarkPlacement("stone_tor", { chance: 1, scale: 1.30, cellSize: 1, radius: 9 });
      case 1:
      case 5:
        return landmarkPlacement("redleaf_tree", { chance: 1, scale: 1.24, cellSize: 1, radius: 13 });
      case 2:
        return landmarkPlacement("old_road_causeway", { chance: 1, scale: 1.22, cellSize: 1, radius: 15 });
      case 3:
        return landmarkPlacement("pilgrim_lantern", { chance: 1, scale: 1.28, cellSize: 1, radius: 5 });
      default:
        return landmarkPlacement("standing_stone", { chance: 1, scale: 1.24, cellSize: 1, radius: 6 });
    }
  }
  if (fields.northeastGrazelands > 0.48) {
    switch (sequence) {
      case 0:
      case 4:
        return landmarkPlacement("standing_stone", { chance: 1, scale: 1.52, cellSize: 1, radius: 7 });
      case 1:
      case 5:
        return landmarkPlacement("acacia", { chance: 1, scale: 1.18, cellSize: 1, radius: 13 });
      case 2:
        return landmarkPlacement("ancestor_pillar", { chance: 1, scale: 1.48, cellSize: 1, radius: 7 });
      case 3:
        return landmarkPlacement("flower_patch", { chance: 1, scale: 1.10, variant: 2, cellSize: 1, radius: 6 });
      default:
        return landmarkPlacement("pilgrim_cairn", { chance: 1, scale: 1.12, cellSize: 1, radius: 5 });
    }
  }
  if (fields.volcanicHeart > 0.42 || fields.ashRing > 0.48) {
    if (sequence === 0 || sequence === 5) {
      return landmarkPlacement("velothi_ziggurat", { chance: 1, scale: 1.34, cellSize: 1, radius: 18 });
    }
    if (sequence === 2 || sequence === 6) {
      return landmarkPlacement("ash_obelisk", { chance: 1, scale: 1.42, cellSize: 1, radius: 9 });
    }
  }
  if (fields.regionId === "bitter-coast" || fields.westWetlands > 0.50 || fields.magic > 0.62) {
    if (fields.regionId === "bitter-coast" && (sequence === 0 || sequence === 4)) {
      return landmarkPlacement("rib_arch", { chance: 1, scale: 1.70, cellSize: 1, radius: 17 });
    }
    if (fields.regionId === "bitter-coast" && (sequence === 2 || sequence === 6)) {
      return landmarkPlacement("crystal_reeds", { chance: 1, scale: 1.66, cellSize: 1, radius: 8 });
    }
    if (fields.regionId === "bitter-coast" && sequence === 5) {
      return landmarkPlacement("rib_remains", { chance: 1, scale: 1.58, cellSize: 1, radius: 12 });
    }
    if (sequence === 1 || sequence === 6) {
      return landmarkPlacement("fungal_bridge", { chance: 1, scale: 1.34, cellSize: 1, radius: 18 });
    }
    if (sequence === 3) {
      return landmarkPlacement("crystal_reeds", { chance: 1, scale: 1.24, cellSize: 1, radius: 7 });
    }
  }
  switch (sequence) {
    case 0:
      return landmarkPlacement("old_road_causeway", { chance: 1, scale: 1.34, cellSize: 1, radius: 17 });
    case 1:
      return landmarkPlacement("pilgrim_lantern", { chance: 1, scale: 1.42, cellSize: 1, radius: 6 });
    case 2:
      return landmarkPlacement("rib_arch", { chance: 1, scale: 1.34, cellSize: 1, radius: 16 });
    case 3:
      return landmarkPlacement("ash_obelisk", { chance: 1, scale: 1.32, cellSize: 1, radius: 9 });
    case 4:
      return landmarkPlacement("bone_chimes", { chance: 1, scale: 1.24, cellSize: 1, radius: 8 });
    case 5:
      return landmarkPlacement("velothi_shrine", { chance: 1, scale: 1.28, cellSize: 1, radius: 6 });
    case 6:
      return landmarkPlacement("buried_ribs", { chance: 1, scale: 1.28, cellSize: 1, radius: 14 });
    default:
      return landmarkPlacement("shrine_debris", { chance: 1, scale: 1.22, cellSize: 1, radius: 16 });
  }
}

function selectLandmarkRoster(
  biomeId: BiomeId,
  undergroundBiomeId: UndergroundBiomeId,
  regionalVariantId: RegionalVariantId | null,
  fields: SurfaceFieldSample,
): readonly LandmarkProfile[] {
  switch (regionalVariantId) {
    case "verdant_karst":
      return VERDANT_KARST_LANDMARKS;
    case "savanna_flowersea":
      return SAVANNA_WILDFLOWER_LANDMARKS;
    case "steppe_monolith":
      return STEPPE_MONOLITH_LANDMARKS;
    case "dunes_glass":
      return DUNES_GLASS_LANDMARKS;
    case "badlands_crater":
      return BADLANDS_CRATER_LANDMARKS;
    case "ash_wastes":
      return ASH_WASTES_LANDMARKS;
    case "highland_redleaf":
      return HIGHLAND_REDLEAF_LANDMARKS;
    case "moor_shadowglass":
      return MOOR_SHADOWGLASS_LANDMARKS;
    case "tundra_blue_ice":
      return TUNDRA_BLUE_ICE_LANDMARKS;
    case "marsh_blackwater":
      return MARSH_BLACKWATER_LANDMARKS;
    case "firefly_lantern":
      return FIREFLY_LANTERN_LANDMARKS;
    case "saltflat_mirror":
      return SALTFLAT_MIRROR_LANDMARKS;
    case "fern_cenote":
      return FERN_CENOTE_LANDMARKS;
    case "fungal_moonlit":
      return FUNGAL_MOONLIT_LANDMARKS;
    case "ember_caldera":
      return EMBER_CALDERA_LANDMARKS;
    case "bloom_prism":
      return BLOOM_PRISM_LANDMARKS;
    default:
      break;
  }
  const undergroundSurfaceRoster = selectUndergroundSurfaceRoster(undergroundBiomeId, biomeId, fields);
  switch (biomeId) {
    case "verdant":
      if (fields.oldGrowth > 0.78 && fields.grove > 0.66 && fields.moisture > 0.68) {
        return VERDANT_CANOPY_SEA_LANDMARKS;
      }
      if (fields.oldGrowth > 0.70 && fields.moisture > 0.60 && fields.grove > 0.54) {
        return VERDANT_OLD_GROWTH_LANDMARKS;
      }
      if (fields.grove > 0.64 && fields.moisture > 0.62 && fields.drainage > 0.56) {
        return VERDANT_GROVE_LANDMARKS;
      }
      if (fields.orchard > 0.68 && fields.temperature > 0.50 && fields.moisture > 0.56) {
        return VERDANT_ORCHARD_LANDMARKS;
      }
      if (undergroundSurfaceRoster) {
        return undergroundSurfaceRoster;
      }
      return BASE_BIOME_LANDMARKS.verdant;
    case "savanna":
      return fields.scatter > 0.58 && fields.moisture > 0.46
        ? SAVANNA_WILDFLOWER_LANDMARKS
        : BASE_BIOME_LANDMARKS.savanna;
    case "steppe":
      if (fields.orchard > 0.70 && fields.moisture > 0.42 && fields.temperature > 0.56) {
        return STEPPE_ORCHARD_LANDMARKS;
      }
      if (fields.desolation > 0.64 && fields.temperature > 0.56 && fields.moisture < 0.48) {
        return STEPPE_THORN_SCRUB_LANDMARKS;
      }
      if (fields.desolation > 0.72 && fields.moisture < 0.42) {
        return STEPPE_DESOLATE_LANDMARKS;
      }
      return BASE_BIOME_LANDMARKS.steppe;
    case "badlands":
      return fields.desolation > 0.58 ? BADLANDS_DESOLATE_LANDMARKS : BASE_BIOME_LANDMARKS.badlands;
    case "highland":
      if (fields.oldGrowth > 0.74 && fields.grove > 0.62 && fields.uplift > 0.72) {
        return HIGHLAND_REDWOOD_BASIN_LANDMARKS;
      }
      if (fields.oldGrowth > 0.66 && fields.moisture > 0.48 && fields.uplift > 0.60) {
        return HIGHLAND_OLD_GROWTH_LANDMARKS;
      }
      if (undergroundSurfaceRoster) {
        return undergroundSurfaceRoster;
      }
      return fields.grove > 0.68 && fields.moisture > 0.50 && fields.uplift > 0.62
        ? HIGHLAND_REDWOOD_LANDMARKS
        : BASE_BIOME_LANDMARKS.highland;
    case "moor":
      return fields.magic > 0.56 && fields.desolation > 0.46
        ? MOOR_SHADOWGLASS_LANDMARKS
        : BASE_BIOME_LANDMARKS.moor;
    case "tundra":
      if (fields.oldGrowth > 0.62 && fields.moisture > 0.34 && fields.uplift > 0.60) {
        return TUNDRA_OLD_GROWTH_LANDMARKS;
      }
      return fields.grove > 0.62 && fields.moisture > 0.36
        ? TUNDRA_TAIGA_LANDMARKS
        : BASE_BIOME_LANDMARKS.tundra;
    case "marsh":
      if (fields.grove > 0.66 && fields.moisture > 0.70) {
        return MARSH_WILLOW_THICKET_LANDMARKS;
      }
      return fields.grove > 0.60 ? MARSH_THICKET_LANDMARKS : SPECIAL_BIOME_LANDMARKS.marsh;
    case "firefly":
      return fields.magic > 0.66 && fields.grove > 0.56
        ? FIREFLY_LANTERN_LANDMARKS
        : SPECIAL_BIOME_LANDMARKS.firefly;
    case "saltflat":
      if (undergroundSurfaceRoster) {
        return undergroundSurfaceRoster;
      }
      return fields.surfacePatch > 0.66 || fields.channel > 0.62
        ? SALTFLAT_MIRROR_LANDMARKS
        : SPECIAL_BIOME_LANDMARKS.saltflat;
    case "fern":
      if (fields.grove > 0.62 && fields.moisture > 0.72 && fields.channel > 0.54) {
        return FERN_OVERGROWN_LANDMARKS;
      }
      if (undergroundSurfaceRoster) {
        return undergroundSurfaceRoster;
      }
      return fields.channel > 0.64 || fields.basin < -0.10
        ? FERN_CENOTE_LANDMARKS
        : SPECIAL_BIOME_LANDMARKS.fern;
    case "fungal":
      if (fields.magic > 0.74 && fields.moisture > 0.62) {
        return FUNGAL_SPORE_GROVE_LANDMARKS;
      }
      if (undergroundSurfaceRoster) {
        return undergroundSurfaceRoster;
      }
      return fields.magic > 0.68
        ? FUNGAL_MOONLIT_LANDMARKS
        : SPECIAL_BIOME_LANDMARKS.fungal;
    case "ember":
      if (undergroundSurfaceRoster) {
        return undergroundSurfaceRoster;
      }
      return fields.desolation > 0.54 ? EMBER_DEADLAND_LANDMARKS : SPECIAL_BIOME_LANDMARKS.ember;
    case "bloom":
      if (fields.magic > 0.62 && (fields.orchard > 0.52 || fields.grove > 0.56)) {
        return BLOOM_FLOWER_GROVE_LANDMARKS;
      }
      if (undergroundSurfaceRoster) {
        return undergroundSurfaceRoster;
      }
      return fields.orchard > 0.64 ? BLOOM_ORCHARD_LANDMARKS : SPECIAL_BIOME_LANDMARKS.bloom;
    case "shardlands":
      if (undergroundSurfaceRoster) {
        return undergroundSurfaceRoster;
      }
      return fields.magic > 0.62 ? SALTFLAT_MIRROR_LANDMARKS : SPECIAL_BIOME_LANDMARKS.shardlands;
    default:
      return BASE_BIOME_LANDMARKS[biomeId];
  }
}

function selectUndergroundSurfaceRoster(
  undergroundBiomeId: UndergroundBiomeId,
  biomeId: BiomeId,
  fields: SurfaceFieldSample,
): readonly LandmarkProfile[] | null {
  switch (undergroundBiomeId) {
    case "rooted":
      if (
        (biomeId === "verdant" || biomeId === "fern" || biomeId === "bloom")
        && fields.oldGrowth > 0.66
        && fields.moisture > 0.62
      ) {
        return ROOTED_SURFACE_LANDMARKS;
      }
      return null;
    case "peaty":
      return fields.moisture > 0.68 && (fields.channel > 0.50 || fields.grove > 0.56)
        ? PEATY_SURFACE_LANDMARKS
        : null;
    case "granitic":
      return fields.uplift > 0.64 && fields.scatter > 0.44
        ? GRANITIC_TOR_LANDMARKS
        : null;
    case "saline":
      return fields.surfacePatch > 0.64 || fields.channel > 0.60
        ? SALINE_CRUST_LANDMARKS
        : null;
    case "mycelial":
      return fields.magic > 0.64 && fields.moisture > 0.54
        ? MYCELIAL_SURFACE_LANDMARKS
        : null;
    case "crystalline":
      return fields.magic > 0.60 || fields.scatter > 0.54
        ? CRYSTALLINE_SURFACE_LANDMARKS
        : null;
    case "basaltic":
      return fields.volcanism > 0.60 || fields.desolation > 0.56
        ? BASALTIC_SURFACE_LANDMARKS
        : null;
    default:
      return null;
  }
}

function resolveDeepCaveAffinity(
  biomeId: BiomeId,
  hostBiomeId: BaseBiomeId,
  undergroundBiomeId: UndergroundBiomeId,
  regionalVariantId: RegionalVariantId | null,
): number {
  let affinity = hostBiomeId === "highland" ? 0.44 : 0.24;
  switch (biomeId) {
    case "verdant":
      affinity += 0.10;
      break;
    case "savanna":
      affinity += 0.04;
      break;
    case "steppe":
      affinity += 0.08;
      break;
    case "dunes":
      affinity += 0.06;
      break;
    case "badlands":
      affinity += 0.34;
      break;
    case "highland":
      affinity += 0.44;
      break;
    case "moor":
      affinity += 0.12;
      break;
    case "tundra":
      affinity += 0.20;
      break;
    case "marsh":
      affinity += 0.04;
      break;
    case "firefly":
      affinity += 0.10;
      break;
    case "saltflat":
      affinity += 0.04;
      break;
    case "fern":
      affinity += 0.24;
      break;
    case "fungal":
      affinity += 0.30;
      break;
    case "ember":
      affinity += 0.40;
      break;
    case "bloom":
      affinity += 0.14;
      break;
    case "shardlands":
      affinity += 0.38;
      break;
  }
  switch (undergroundBiomeId) {
    case "granitic":
    case "crystalline":
    case "basaltic":
      affinity += 0.14;
      break;
    case "mycelial":
      affinity += 0.18;
      break;
    case "sandy":
    case "peaty":
      affinity -= 0.06;
      break;
    default:
      break;
  }
  switch (regionalVariantId) {
    case "verdant_karst":
    case "badlands_crater":
    case "ember_caldera":
    case "fern_cenote":
      affinity += 0.18;
      break;
    case "moor_shadowglass":
    case "tundra_blue_ice":
    case "fungal_moonlit":
      affinity += 0.10;
      break;
    case "saltflat_mirror":
      affinity -= 0.04;
      break;
    default:
      break;
  }
  return saturate(affinity);
}

function resolveUpperCaveAffinity(
  biomeId: BiomeId,
  hostBiomeId: BaseBiomeId,
  undergroundBiomeId: UndergroundBiomeId,
  regionalVariantId: RegionalVariantId | null,
): number {
  let affinity = hostBiomeId === "highland" ? 0.16 : 0.04;
  switch (biomeId) {
    case "verdant":
      affinity += 0.08;
      break;
    case "savanna":
      affinity += 0.02;
      break;
    case "steppe":
      affinity += 0.06;
      break;
    case "dunes":
      affinity += 0.12;
      break;
    case "badlands":
      affinity += 0.34;
      break;
    case "highland":
      affinity += 0.46;
      break;
    case "moor":
      affinity += 0.04;
      break;
    case "tundra":
      affinity += 0.16;
      break;
    case "marsh":
      affinity += 0.02;
      break;
    case "firefly":
      affinity += 0.04;
      break;
    case "saltflat":
      affinity += 0.06;
      break;
    case "fern":
      affinity += 0.26;
      break;
    case "fungal":
      affinity += 0.08;
      break;
    case "ember":
      affinity += 0.26;
      break;
    case "bloom":
      affinity += 0.06;
      break;
    case "shardlands":
      affinity += 0.34;
      break;
  }
  switch (undergroundBiomeId) {
    case "granitic":
    case "crystalline":
      affinity += 0.08;
      break;
    case "rooted":
      affinity += 0.04;
      break;
    case "peaty":
      affinity -= 0.04;
      break;
    default:
      break;
  }
  switch (regionalVariantId) {
    case "verdant_karst":
    case "fern_cenote":
      affinity += 0.32;
      break;
    case "badlands_crater":
    case "ember_caldera":
    case "steppe_monolith":
      affinity += 0.14;
      break;
    case "saltflat_mirror":
    case "savanna_flowersea":
      affinity -= 0.02;
      break;
    default:
      break;
  }
  return saturate(affinity);
}

function selectRegionalVariant(biomeId: BiomeId, fields: SurfaceFieldSample): RegionalVariantSelection | null {
  let strength = 0;
  let id: RegionalVariantId | null = null;
  switch (biomeId) {
    case "verdant":
      strength = avg4(
        smoothstep(0.68, 0.84, fields.oldGrowth),
        smoothstep(0.58, 0.78, fields.drainage),
        smoothstep(0.48, 0.74, fields.globalHeight),
        smoothstep(0.52, 0.80, fields.channel),
      );
      id = strength > 0.56 ? "verdant_karst" : null;
      break;
    case "savanna":
      strength = avg4(
        smoothstep(0.62, 0.84, fields.temperature),
        smoothstep(0.58, 0.84, fields.scatter),
        smoothstep(0.48, 0.72, fields.moisture),
        smoothstep(0.48, 0.76, fields.grove + fields.orchard * 0.2),
      );
      id = strength > 0.60 ? "savanna_flowersea" : null;
      break;
    case "steppe":
      strength = avg4(
        smoothstep(0.58, 0.80, fields.uplift),
        smoothstep(0.56, 0.82, fields.ridge),
        smoothstep(0.50, 0.76, fields.peakness),
        smoothstep(0.48, 0.76, fields.temperature),
      );
      id = strength > 0.82 ? "steppe_monolith" : null;
      break;
    case "dunes":
      strength = avg4(
        smoothstep(0.76, 0.90, fields.temperature),
        smoothstep(0.64, 0.88, fields.dune),
        smoothstep(0.48, 0.78, fields.volcanism + fields.magic * 0.2),
        smoothstep(0.56, 0.84, 1 - fields.moisture),
      );
      id = strength > 0.64 ? "dunes_glass" : null;
      break;
    case "badlands":
      strength = avg4(
        smoothstep(0.54, 0.80, fields.uplift),
        smoothstep(0.54, 0.82, fields.mesa),
        smoothstep(0.48, 0.80, fields.volcanism),
        smoothstep(0.48, 0.78, fields.peakness),
      );
      {
        const ashStrength = avg4(
          smoothstep(0.50, 0.78, fields.volcanism),
          smoothstep(0.40, 0.76, fields.desolation),
          smoothstep(0.36, 0.74, fields.mesa),
          smoothstep(0.48, 0.78, 1 - fields.moisture),
        );
        if (ashStrength > 0.56) {
          strength = ashStrength;
          id = "ash_wastes";
        } else {
          id = strength > 0.62 ? "badlands_crater" : null;
        }
      }
      break;
    case "highland":
      strength = avg4(
        smoothstep(0.64, 0.84, fields.oldGrowth),
        scoreField(fields.temperature, 0.54, 0.18),
        smoothstep(0.46, 0.76, fields.moisture),
        smoothstep(0.54, 0.80, fields.uplift),
      );
      id = strength > 0.50 ? "highland_redleaf" : null;
      break;
    case "moor":
      strength = avg3(
        smoothstep(0.58, 0.82, fields.magic),
        smoothstep(0.54, 0.80, fields.desolation + fields.scatter * 0.15),
        smoothstep(0.58, 0.84, fields.moisture),
      );
      id = strength > 0.50 ? "moor_shadowglass" : null;
      break;
    case "tundra":
      strength = avg3(
        smoothstep(0.70, 0.88, fields.uplift),
        smoothstep(0.56, 0.82, fields.peakness + fields.ridge * 0.2),
        smoothstep(0.52, 0.84, 1 - fields.temperature),
      );
      id = strength > 0.78 ? "tundra_blue_ice" : null;
      break;
    case "marsh":
      strength = avg3(
        smoothstep(0.70, 0.88, fields.moisture),
        smoothstep(0.66, 0.88, fields.channel),
        smoothstep(0.54, 0.80, fields.grove),
      );
      id = strength > 0.46 ? "marsh_blackwater" : null;
      break;
    case "firefly":
      strength = avg3(
        smoothstep(0.66, 0.86, fields.magic),
        smoothstep(0.70, 0.90, fields.moisture),
        smoothstep(0.58, 0.84, fields.channel + fields.grove * 0.2),
      );
      id = strength > 0.46 ? "firefly_lantern" : null;
      break;
    case "saltflat":
      strength = avg3(
        smoothstep(0.60, 0.84, fields.oceanness + Math.max(0, -fields.basin) * 0.25),
        smoothstep(0.56, 0.82, fields.surfacePatch),
        smoothstep(0.58, 0.84, 1 - fields.moisture),
      );
      id = strength > 0.52 ? "saltflat_mirror" : null;
      break;
    case "fern":
      strength = avg3(
        smoothstep(0.64, 0.86, fields.temperature),
        smoothstep(0.68, 0.88, fields.moisture),
        smoothstep(0.58, 0.86, fields.channel + Math.max(0, -fields.basin) * 0.30),
      );
      id = strength > 0.52 ? "fern_cenote" : null;
      break;
    case "fungal":
      strength = avg3(
        smoothstep(0.68, 0.88, fields.magic),
        smoothstep(0.68, 0.88, fields.moisture),
        smoothstep(0.56, 0.82, fields.oldGrowth + fields.grove * 0.2),
      );
      id = strength > 0.46 ? "fungal_moonlit" : null;
      break;
    case "ember":
      strength = avg3(
        smoothstep(0.66, 0.86, fields.volcanism),
        smoothstep(0.58, 0.82, fields.peakness + fields.mesa * 0.3),
        smoothstep(0.50, 0.80, fields.ridge),
      );
      {
        const ashStrength = avg4(
          smoothstep(0.54, 0.82, fields.volcanism),
          smoothstep(0.44, 0.78, fields.desolation),
          smoothstep(0.42, 0.76, fields.surfacePatch + fields.scatter * 0.2),
          smoothstep(0.48, 0.78, 1 - fields.moisture),
        );
        if (ashStrength > 0.58 && strength < 0.72) {
          strength = ashStrength;
          id = "ash_wastes";
        } else {
          id = strength > 0.52 ? "ember_caldera" : null;
        }
      }
      break;
    case "bloom":
      strength = avg3(
        smoothstep(0.68, 0.86, fields.magic),
        smoothstep(0.50, 0.78, fields.orchard + fields.grove * 0.2),
        smoothstep(0.44, 0.74, fields.moisture),
      );
      id = strength > 0.64 ? "bloom_prism" : null;
      break;
  }
  if (id === null) return null;
  reusableRegionalVariant.id = id;
  reusableRegionalVariant.strength = strength;
  return reusableRegionalVariant;
}

const reusableRegionalVariant: RegionalVariantSelection = { id: "verdant_karst", strength: 0 };

function sampleGeneratorAtlasFields(worldX: number, worldZ: number): GeneratorAtlasFields {
  const xM = worldX / WORLD_UNITS_PER_METER;
  const zM = worldZ / WORLD_UNITS_PER_METER;
  const island = sampleIslandMaskMeters(xM, zM, WORLD_ATLAS);
  const region = sampleAtlasRegionMeters(xM, zM, island, WORLD_ATLAS);
  const route = sampleGeneratorAtlasRouteFields(xM, zM, island.islandInterior);
  const cave = sampleAtlasCaveAnchorMeters(xM, zM, island, WORLD_ATLAS);
  return {
    islandInterior: island.islandInterior,
    shorelineBand: island.shorelineBand,
    coastalShelf: island.coastalShelf,
    deepOcean: island.deepOcean,
    primaryRegionId: region.primaryRegionId,
    secondaryRegionId: region.secondaryRegionId,
    regionStrength: region.regionStrength,
    regionBlend: region.regionBlend,
    regionDistance: region.regionDistance,
    primaryBiomeId: region.primaryBiomeId,
    regionalVariantId: region.regionalVariantId,
    ambientProfileId: region.ambientProfileId,
    ...route,
    caveInfluence: cave.caveInfluence,
    caveCore: cave.caveCore,
    distanceToCaveAnchorM: cave.distanceToCaveAnchorM,
  };
}

function sampleGeneratorAtlasRouteFields(
  xM: number,
  zM: number,
  islandInterior: number,
): Pick<GeneratorAtlasFields, "routeInfluence" | "routeCore" | "routeShoulder" | "distanceToRouteM"> {
  if (islandInterior < 0.08) {
    return { routeInfluence: 0, routeCore: 0, routeShoulder: 0, distanceToRouteM: Infinity };
  }
  let nearestDistance = Infinity;
  for (const route of WORLD_ATLAS.routes) {
    for (let index = 0; index < route.nodes.length - 1; index += 1) {
      const a = route.nodes[index]!.point;
      const b = route.nodes[index + 1]!.point;
      nearestDistance = Math.min(nearestDistance, distancePointToSegmentMeters(xM, zM, a.x, a.z, b.x, b.z));
    }
  }
  const core = 1 - smoothstep(38, 68, nearestDistance);
  const shoulder = smoothstep(38, 68, nearestDistance) * (1 - smoothstep(72, 104, nearestDistance));
  const influence = Math.max(core, shoulder);
  return { routeInfluence: influence, routeCore: core, routeShoulder: shoulder, distanceToRouteM: nearestDistance };
}

function distancePointToSegmentMeters(
  x: number,
  z: number,
  ax: number,
  az: number,
  bx: number,
  bz: number,
): number {
  const abX = bx - ax;
  const abZ = bz - az;
  const lengthSquared = abX * abX + abZ * abZ;
  if (lengthSquared <= 0) {
    return Math.hypot(x - ax, z - az);
  }
  const t = clamp(((x - ax) * abX + (z - az) * abZ) / lengthSquared, 0, 1);
  return Math.hypot(x - (ax + abX * t), z - (az + abZ * t));
}

function sampleAtlasUpliftBias(regionId: WorldRegionId, regionCore: number): number {
  switch (regionId) {
    case "red-mountain":
      return regionCore * 0.46;
    case "west-gash":
      return regionCore * 0.28;
    case "glass-shard-coast":
      return regionCore * 0.12;
    case "bitter-coast":
    case "salt-marsh-basin":
      return -regionCore * 0.08;
    default:
      return regionCore * 0.03;
  }
}

function sampleAtlasGlobalHeightBias(regionId: WorldRegionId, regionCore: number): number {
  switch (regionId) {
    case "red-mountain":
      return regionCore * 0.20;
    case "west-gash":
      return regionCore * 0.13;
    case "ashen-badlands":
      return regionCore * 0.07;
    case "glass-shard-coast":
      return regionCore * 0.05;
    case "bitter-coast":
      return -regionCore * 0.10;
    case "salt-marsh-basin":
      return -regionCore * 0.14;
    case "inner-sea":
      return -regionCore * 0.04;
    default:
      return 0;
  }
}

function sampleAtlasMountainnessBias(regionId: WorldRegionId, regionCore: number): number {
  switch (regionId) {
    case "red-mountain":
      return regionCore * 0.58;
    case "west-gash":
      return regionCore * 0.36;
    case "ashen-badlands":
      return regionCore * 0.16;
    case "glass-shard-coast":
      return regionCore * 0.20;
    default:
      return 0;
  }
}

function sampleAtlasRegionSurfaceDelta(fields: SurfaceFieldSample): number {
  const strength = fields.regionStrength * fields.islandInterior;
  switch (fields.regionId) {
    case "red-mountain":
      return Math.round((92 + fields.peakness * 132 + fields.ridge * 42) * strength);
    case "west-gash":
      return Math.round((54 + fields.ridge * 60 + Math.max(0, fields.hills) * 36) * strength);
    case "ashen-badlands":
      return Math.round((18 + fields.mesa * 34 + fields.strata * 18) * strength);
    case "glass-shard-coast":
      return Math.round((22 + fields.ridge * 34 + fields.surfaceGrain * 18) * strength);
    case "grazelands":
      return Math.round((6 + fields.hills * 26) * strength);
    case "bitter-coast":
      return -Math.round((26 + fields.channel * 24) * strength);
    case "inner-sea":
      return -Math.round((16 + Math.max(0, -fields.basin) * 22) * strength);
    case "salt-marsh-basin":
      return -Math.round((38 + fields.channel * 18 + fields.coastalShelf * 28) * strength);
    default:
      return 0;
  }
}

function sampleTerrainCrustBreakup(
  fields: SurfaceFieldSample,
  terrainProfile: { terraceScale: number; microRelief: number },
  biomeCore: number,
): number {
  const terraceHost = smoothstep(0.18, 0.70, terrainProfile.terraceScale);
  const harshSurface = avg4(
    smoothstep(0.46, 0.82, fields.surfacePatch),
    smoothstep(0.44, 0.82, fields.scatter),
    smoothstep(0.44, 0.84, fields.strata),
    smoothstep(0.40, 0.82, fields.desolation + fields.mesa * 0.20),
  );
  const diagonalLift = (
    (fields.strata - 0.5) * 2.0
    + (fields.surfacePatch - 0.5) * 1.2
    - (fields.scatter - 0.5) * 0.9
  );
  const erodedLip = (
    smoothstep(0.56, 0.86, fields.surfacePatch + fields.mesa * 0.18)
    - smoothstep(0.52, 0.84, fields.channel + fields.basin * 0.14)
  );
  const amplitude = (1.2 + terrainProfile.microRelief * 0.18)
    * (0.22 + biomeCore * 0.48)
    * (0.45 + terraceHost * 0.55)
    * (0.50 + harshSurface * 0.50);
  return Math.round((diagonalLift * 0.62 + erodedLip * 0.72) * amplitude);
}

function adjustSpecialBiomeSurfaceY(
  seaLevel: number,
  biomeId: BiomeId,
  specialStrength: number,
  fields: SurfaceFieldSample,
  biomeCore: number,
  surfaceY: number,
): number {
  switch (biomeId) {
    case "marsh":
      return surfaceY - Math.round(lerp(1, 7, specialStrength) * (0.35 + biomeCore * 0.65));
    case "firefly":
      return surfaceY - Math.round(lerp(0, 4, specialStrength) * (0.30 + biomeCore * 0.50));
    case "saltflat": {
      const saltTarget = seaLevel - 10 + Math.round((fields.surfacePatch - 0.5) * 6);
      return Math.round(lerp(surfaceY, saltTarget, clamp(0.36 + specialStrength * 0.42, 0, 0.86)));
    }
    case "fern":
      return surfaceY - Math.round(lerp(0, 3, specialStrength) * (0.25 + biomeCore * 0.55));
    case "fungal":
      return surfaceY + Math.round((fields.detail - 0.5) * lerp(6, 14, specialStrength) * (0.30 + biomeCore * 0.70));
    case "bloom":
      return surfaceY + Math.round(lerp(0, 4, specialStrength) * (fields.magic - 0.5) * (0.3 + biomeCore * 0.7));
    case "ember":
      return surfaceY + Math.round(lerp(2, 11, specialStrength) * (0.25 + fields.mesa) * (0.35 + biomeCore * 0.65));
    case "shardlands":
      return surfaceY + Math.round(lerp(3, 12, specialStrength) * (0.35 + fields.ridge * 0.65) * (0.30 + biomeCore * 0.70));
    default:
      return surfaceY;
  }
}

function sampleRegionalVariantSurfaceDelta(
  regionalVariantId: RegionalVariantId,
  strength: number,
  fields: SurfaceFieldSample,
  biomeCore: number,
): number {
  const weight = 0.28 + biomeCore * 0.72;
  switch (regionalVariantId) {
    case "verdant_karst":
      return -Math.round(lerp(6, 18, strength) * (0.6 + fields.channel * 0.4) * weight);
    case "savanna_flowersea":
      return Math.round((fields.hills - 0.2) * (8 + strength * 10) * weight);
    case "steppe_monolith":
      return Math.round(lerp(4, 16, strength) * (0.46 + fields.ridge * 0.42 + fields.surfacePatch * 0.12) * weight)
        + Math.round(((fields.strata - 0.5) * 5 + (fields.scatter - 0.5) * 3) * weight);
    case "dunes_glass":
      return Math.round((fields.dune - 0.52) * (12 + strength * 18) * weight);
    case "badlands_crater":
      return Math.round((fields.mesa - 0.52) * (16 + strength * 20) * weight);
    case "ash_wastes":
      return Math.round((fields.mesa - 0.58) * (6 + strength * 10) * weight)
        - Math.round(lerp(2, 7, strength) * (0.45 + Math.max(0, fields.desolation - 0.5)) * weight)
        + Math.round(((fields.strata - 0.5) * 12 + (fields.surfaceGrain - 0.5) * 6 + (fields.scatter - 0.5) * 5) * weight);
    case "highland_redleaf":
      return Math.round(lerp(4, 12, strength) * (0.55 + fields.hills * 0.15 + 0.30) * weight);
    case "moor_shadowglass":
      return -Math.round(lerp(2, 8, strength) * (0.6 + Math.max(0, -fields.basin) * 0.4) * weight);
    case "tundra_blue_ice":
      return Math.round(lerp(6, 18, strength) * (0.5 + fields.ridge * 0.5) * weight);
    case "marsh_blackwater":
      return -Math.round(lerp(4, 12, strength) * (0.65 + fields.channel * 0.35) * weight);
    case "firefly_lantern":
      return -Math.round(lerp(2, 8, strength) * (0.55 + fields.channel * 0.45) * weight);
    case "saltflat_mirror":
      return -Math.round(lerp(2, 6, strength) * (0.6 + Math.max(0, -fields.basin) * 0.4) * weight);
    case "fern_cenote":
      return -Math.round(lerp(4, 10, strength) * (0.55 + fields.channel * 0.45) * weight);
    case "fungal_moonlit":
      return Math.round((fields.detail - 0.5) * (8 + strength * 12) * weight);
    case "ember_caldera":
      return Math.round(lerp(6, 18, strength) * (0.45 + fields.volcanism * 0.55) * weight);
    case "bloom_prism":
      return Math.round(lerp(2, 8, strength) * (0.45 + fields.magic * 0.55) * weight);
    default:
      return 0;
  }
}

function applyRegionalVariantMaterialOverrides(
  materials: {
    surfacePrimary: number;
    surfaceSecondary: number;
    subsurfacePrimary: number;
    subsurfaceSecondary: number;
    water: number;
    snow: number;
    transitionThreshold: number;
  },
  regionalVariantId: RegionalVariantId,
): void {
  switch (regionalVariantId) {
    case "verdant_karst":
      materials.surfacePrimary = hexColorToMaterial("#8B7");
      materials.surfaceSecondary = hexColorToMaterial("#BBC");
      materials.subsurfacePrimary = hexColorToMaterial("#887");
      materials.subsurfaceSecondary = hexColorToMaterial("#998");
      break;
    case "savanna_flowersea":
      materials.surfacePrimary = hexColorToMaterial("#CB7");
      materials.surfaceSecondary = hexColorToMaterial("#DA8");
      materials.subsurfacePrimary = hexColorToMaterial("#A86");
      materials.subsurfaceSecondary = hexColorToMaterial("#B97");
      break;
    case "steppe_monolith":
      materials.surfacePrimary = hexColorToMaterial("#BA7");
      materials.surfaceSecondary = hexColorToMaterial("#CBA");
      materials.subsurfacePrimary = hexColorToMaterial("#987");
      materials.subsurfaceSecondary = hexColorToMaterial("#A98");
      break;
    case "dunes_glass":
      materials.surfacePrimary = hexColorToMaterial("#EDC");
      materials.surfaceSecondary = hexColorToMaterial("#CDD");
      materials.subsurfacePrimary = hexColorToMaterial("#BAA");
      materials.subsurfaceSecondary = hexColorToMaterial("#CBB");
      break;
    case "badlands_crater":
      materials.surfacePrimary = hexColorToMaterial("#965");
      materials.surfaceSecondary = hexColorToMaterial("#B75");
      materials.subsurfacePrimary = hexColorToMaterial("#754");
      materials.subsurfaceSecondary = hexColorToMaterial("#965");
      break;
    case "ash_wastes":
      materials.surfacePrimary = hexColorToMaterial("#655");
      materials.surfaceSecondary = hexColorToMaterial("#887");
      materials.subsurfacePrimary = hexColorToMaterial("#433");
      materials.subsurfaceSecondary = hexColorToMaterial("#544");
      materials.transitionThreshold = Math.min(materials.transitionThreshold, 0.66);
      break;
    case "highland_redleaf":
      materials.surfacePrimary = hexColorToMaterial("#A86");
      materials.surfaceSecondary = hexColorToMaterial("#C97");
      materials.subsurfacePrimary = hexColorToMaterial("#875");
      materials.subsurfaceSecondary = hexColorToMaterial("#986");
      break;
    case "moor_shadowglass":
      materials.surfacePrimary = hexColorToMaterial("#546");
      materials.surfaceSecondary = hexColorToMaterial("#768");
      materials.subsurfacePrimary = hexColorToMaterial("#435");
      materials.subsurfaceSecondary = hexColorToMaterial("#657");
      materials.water = hexColorToMaterial("#245");
      break;
    case "tundra_blue_ice":
      materials.surfacePrimary = hexColorToMaterial("#CDD");
      materials.surfaceSecondary = hexColorToMaterial("#DFF");
      materials.subsurfacePrimary = hexColorToMaterial("#AAB");
      materials.subsurfaceSecondary = hexColorToMaterial("#BCD");
      materials.water = hexColorToMaterial("#9DF");
      break;
    case "marsh_blackwater":
      materials.surfacePrimary = hexColorToMaterial("#354");
      materials.surfaceSecondary = hexColorToMaterial("#465");
      materials.subsurfacePrimary = hexColorToMaterial("#243");
      materials.subsurfaceSecondary = hexColorToMaterial("#354");
      materials.water = hexColorToMaterial("#134");
      break;
    case "firefly_lantern":
      materials.surfacePrimary = hexColorToMaterial("#465");
      materials.surfaceSecondary = hexColorToMaterial("#687");
      materials.subsurfacePrimary = hexColorToMaterial("#354");
      materials.subsurfaceSecondary = hexColorToMaterial("#576");
      materials.water = hexColorToMaterial("#245");
      break;
    case "saltflat_mirror":
      materials.surfacePrimary = hexColorToMaterial("#FFF");
      materials.surfaceSecondary = hexColorToMaterial("#DEF");
      materials.subsurfacePrimary = hexColorToMaterial("#CCB");
      materials.subsurfaceSecondary = hexColorToMaterial("#EED");
      materials.water = hexColorToMaterial("#9CF");
      break;
    case "fern_cenote":
      materials.surfacePrimary = hexColorToMaterial("#8D6");
      materials.surfaceSecondary = hexColorToMaterial("#6B8");
      materials.subsurfacePrimary = hexColorToMaterial("#786");
      materials.subsurfaceSecondary = hexColorToMaterial("#576");
      materials.water = hexColorToMaterial("#5BD");
      break;
    case "fungal_moonlit":
      materials.surfacePrimary = hexColorToMaterial("#687");
      materials.surfaceSecondary = hexColorToMaterial("#8AF");
      materials.subsurfacePrimary = hexColorToMaterial("#556");
      materials.subsurfaceSecondary = hexColorToMaterial("#678");
      materials.water = hexColorToMaterial("#58C");
      break;
    case "ember_caldera":
      materials.surfacePrimary = hexColorToMaterial("#433");
      materials.surfaceSecondary = hexColorToMaterial("#654");
      materials.subsurfacePrimary = hexColorToMaterial("#322");
      materials.subsurfaceSecondary = hexColorToMaterial("#433");
      break;
    case "bloom_prism":
      materials.surfacePrimary = hexColorToMaterial("#8CF");
      materials.surfaceSecondary = hexColorToMaterial("#BDF");
      materials.subsurfacePrimary = hexColorToMaterial("#668");
      materials.subsurfaceSecondary = hexColorToMaterial("#79B");
      materials.water = hexColorToMaterial("#6DF");
      break;
    default:
      break;
  }
}

function applyAtlasSurfaceMaterialOverrides(
  materials: ResolvedSurfaceMaterials,
  fields: SurfaceFieldSample,
): void {
  if (fields.deepOcean > 0.34) {
    materials.surfacePrimary = hexColorToMaterial("#245");
    materials.surfaceSecondary = hexColorToMaterial("#134");
    materials.subsurfacePrimary = hexColorToMaterial("#345");
    materials.subsurfaceSecondary = hexColorToMaterial("#234");
    materials.water = hexColorToMaterial("#134");
    materials.transitionThreshold = 0.68;
    return;
  }

  if (fields.coastalShelf > 0.46 && fields.islandInterior < 0.38) {
    materials.surfacePrimary = hexColorToMaterial("#887");
    materials.surfaceSecondary = hexColorToMaterial("#655");
    materials.subsurfacePrimary = hexColorToMaterial("#665");
    materials.subsurfaceSecondary = hexColorToMaterial("#554");
    materials.water = hexColorToMaterial("#58C");
    materials.transitionThreshold = 0.58;
  }

  if (fields.atlasCaveInfluence <= 0.48) {
    return;
  }

  switch (fields.regionId) {
    case "red-mountain":
    case "ashen-badlands":
      materials.subsurfacePrimary = hexColorToMaterial("#433");
      materials.subsurfaceSecondary = hexColorToMaterial("#654");
      break;
    case "bitter-coast":
      materials.subsurfacePrimary = hexColorToMaterial("#342");
      materials.subsurfaceSecondary = hexColorToMaterial("#564");
      break;
    case "salt-marsh-basin":
      materials.subsurfacePrimary = hexColorToMaterial("#BBA");
      materials.subsurfaceSecondary = hexColorToMaterial("#887");
      break;
    case "glass-shard-coast":
      materials.subsurfacePrimary = hexColorToMaterial("#667");
      materials.subsurfaceSecondary = hexColorToMaterial("#CEF");
      break;
    case "west-gash":
      materials.subsurfacePrimary = hexColorToMaterial("#667");
      materials.subsurfaceSecondary = hexColorToMaterial("#889");
      break;
    default:
      break;
  }
}

function configureLandmarkFeature(
  profile: LandmarkProfile,
  surfaceY: number,
  waterTopY: number,
  fields: SurfaceFieldSample,
  out: MutableColumnState,
): boolean {
  const submergedSurface = hasStandingWater(surfaceY, waterTopY);
  switch (profile.id) {
    case "oak":
      if (submergedSurface) {
        return false;
      }
      configureTreeFeature(
        out,
        FEATURE_OAK,
        scaledFeatureHeight(30, 16, fields.moisture, profile.scale),
        scaledFeatureRadius(10, 3, fields.moisture, profile.scale),
        "#653",
        profile.variant >= 4 ? "#8CF" : "#5B4",
      );
      out.featureExtra = profile.variant;
      return true;
    case "canopy_tree":
      if (submergedSurface) {
        return false;
      }
      configureTreeFeature(
        out,
        FEATURE_OAK,
        scaledFeatureHeight(44, 22, fields.moisture, profile.scale),
        scaledFeatureRadius(15, 4, fields.moisture, profile.scale),
        "#653",
        profile.variant >= 4 ? "#9EF" : "#4A5",
      );
      out.featureExtra = profile.variant;
      return true;
    case "birch":
      if (submergedSurface) {
        return false;
      }
      configureTreeFeature(
        out,
        FEATURE_OAK,
        scaledFeatureHeight(32, 14, fields.moisture, profile.scale),
        scaledFeatureRadius(8, 2, fields.moisture, profile.scale),
        "#EEC",
        "#9C7",
      );
      out.featureExtra = 1;
      return true;
    case "redleaf_tree":
      if (submergedSurface) {
        return false;
      }
      configureTreeFeature(
        out,
        FEATURE_OAK,
        scaledFeatureHeight(32, 18, fields.moisture + fields.temperature * 0.2, profile.scale),
        scaledFeatureRadius(11, 3, fields.moisture, profile.scale),
        "#754",
        "#C75",
        "#E97",
      );
      out.featureExtra = 9;
      return true;
    case "willow":
      if (submergedSurface) {
        return false;
      }
      configureTreeFeature(
        out,
        FEATURE_CYPRESS,
        scaledFeatureHeight(34, 20, fields.moisture + fields.drainage * 0.2, profile.scale),
        scaledFeatureRadius(12, 5, fields.moisture, profile.scale),
        "#754",
        "#7A8",
      );
      out.featureExtra = 1;
      return true;
    case "blossom_tree":
      if (submergedSurface) {
        return false;
      }
      configureTreeFeature(
        out,
        FEATURE_OAK,
        scaledFeatureHeight(28, 16, fields.moisture, profile.scale),
        scaledFeatureRadius(10, 3, fields.moisture, profile.scale),
        "#754",
        "#FCD",
        "#FFF",
      );
      out.featureExtra = 7;
      return true;
    case "fruit_tree":
      if (submergedSurface) {
        return false;
      }
      configureTreeFeature(
        out,
        FEATURE_OAK,
        scaledFeatureHeight(26, 14, fields.moisture, profile.scale),
        scaledFeatureRadius(9, 3, fields.moisture, profile.scale),
        "#754",
        "#7A5",
        fields.temperature > 0.62 ? "#F84" : "#C33",
      );
      out.featureExtra = 8;
      return true;
    case "giant_flower":
      if (submergedSurface) {
        return false;
      }
      configureTreeFeature(
        out,
        FEATURE_GLOWCAP,
        scaledFeatureHeight(18, 18, fields.magic + fields.moisture * 0.2, profile.scale),
        scaledFeatureRadius(9, 4, fields.magic + fields.moisture * 0.2, profile.scale),
        "#7A5",
        profile.variant > 0 ? "#FD7" : "#D7F",
      );
      out.featureExtra = 4;
      return true;
    case "redwood":
      if (submergedSurface) {
        return false;
      }
      configureTreeFeature(
        out,
        FEATURE_REDWOOD,
        scaledFeatureHeight(118, 52, fields.uplift + fields.moisture * 0.5, profile.scale),
        scaledFeatureRadius(14, 6, fields.moisture, profile.scale),
        "#643",
        "#586",
      );
      return true;
    case "dead_tree":
      if (submergedSurface) {
        return false;
      }
      configureTreeFeature(
        out,
        FEATURE_DEAD_TREE,
        scaledFeatureHeight(32, 24, fields.uplift + fields.desolation * 0.4, profile.scale),
        scaledFeatureRadius(7, 3, fields.scatter, profile.scale),
        profile.variant > 0 ? "#322" : "#544",
        profile.variant > 0 ? "#655" : "#765",
      );
      return true;
    case "thorn_tree":
      if (submergedSurface) {
        return false;
      }
      configureTreeFeature(
        out,
        FEATURE_DEAD_TREE,
        scaledFeatureHeight(28, 18, fields.temperature + fields.desolation * 0.5, profile.scale),
        scaledFeatureRadius(8, 4, fields.scatter + fields.temperature * 0.2, profile.scale),
        "#543",
        "#875",
      );
      out.featureExtra = 2;
      return true;
    case "berry_bush":
      if (submergedSurface) {
        return false;
      }
      configureTreeFeature(
        out,
        FEATURE_BUSH,
        scaledFeatureHeight(4, 4, fields.moisture, profile.scale),
        scaledFeatureRadius(5, 2, fields.moisture, profile.scale),
        "#653",
        "#486",
        "#C35",
      );
      out.featureExtra = 5;
      return true;
    case "giant_fern":
      if (submergedSurface) {
        return false;
      }
      configureTreeFeature(
        out,
        FEATURE_PALM,
        scaledFeatureHeight(24, 18, fields.moisture + fields.temperature * 0.2, profile.scale),
        scaledFeatureRadius(11, 4, fields.moisture + fields.temperature * 0.2, profile.scale),
        "#675",
        "#8B6",
      );
      return true;
    case "lantern_tree":
      if (submergedSurface) {
        return false;
      }
      configureTreeFeature(
        out,
        FEATURE_OAK,
        scaledFeatureHeight(24, 14, fields.magic + fields.moisture * 0.2, profile.scale),
        scaledFeatureRadius(10, 3, fields.magic + fields.moisture * 0.2, profile.scale),
        "#543",
        "#7A8",
        "#FC8",
      );
      out.featureExtra = 10;
      return true;
    case "salt_spire":
      configureSpireFeature(
        out,
        FEATURE_CRYSTAL,
        scaledFeatureHeight(18, 20, Math.max(fields.surfacePatch, 1 - fields.moisture), profile.scale),
        scaledFeatureRadius(4, 3, fields.surfacePatch + fields.scatter * 0.2, profile.scale),
        "#CCD",
        "#FFF",
      );
      out.featureExtra = 1;
      return true;
    case "boulder":
      configureSpireFeature(
        out,
        FEATURE_BOULDER,
        scaledFeatureHeight(4, 4, fields.scatter, profile.scale),
        scaledFeatureRadius(4, 2, fields.scatter, profile.scale),
        profile.variant > 0 ? "#665" : "#889",
        profile.variant > 0 ? "#887" : "#BBC",
      );
      out.featureExtra = 0;
      return true;
    case "standing_stone":
      configureSpireFeature(
        out,
        FEATURE_STANDING_STONE,
        scaledFeatureHeight(22, 14, fields.uplift, profile.scale),
        scaledFeatureRadius(3, 2, fields.uplift, profile.scale),
        "#998",
        "#CBA",
      );
      return true;
    case "ancestor_pillar":
      configureSpireFeature(
        out,
        FEATURE_STANDING_STONE,
        scaledFeatureHeight(28, 18, fields.uplift + fields.magic * 0.2, profile.scale),
        scaledFeatureRadius(4, 2, fields.uplift + fields.scatter * 0.2, profile.scale),
        "#776",
        "#DBC",
      );
      out.featureExtra = 1;
      return true;
    case "shrub":
      if (submergedSurface) {
        return false;
      }
      configureTreeFeature(
        out,
        FEATURE_BUSH,
        scaledFeatureHeight(3, 5, fields.moisture, profile.scale),
        scaledFeatureRadius(4, 2, fields.moisture, profile.scale),
        profile.variant === 2 ? "#986" : "#764",
        profile.variant === 2 ? "#BA7" : "#7B6",
      );
      return true;
    case "flower_patch":
      if (submergedSurface) {
        return false;
      }
      configureTreeFeature(
        out,
        FEATURE_BUSH,
        scaledFeatureHeight(2, 2, fields.scatter, profile.scale),
        scaledFeatureRadius(5, 3, fields.moisture + fields.magic * 0.3, profile.scale),
        "#784",
        profile.variant === 4
          ? "#BDF"
          : profile.variant === 3
          ? "#6DB"
          : profile.variant === 2
          ? "#DA7"
          : "#D97",
      );
      return true;
    case "palm":
      if (submergedSurface || (waterTopY === NO_WATER && fields.channel < 0.68)) {
        return false;
      }
      configureTreeFeature(
        out,
        FEATURE_PALM,
        scaledFeatureHeight(32, 12, fields.temperature, profile.scale),
        scaledFeatureRadius(9, 4, fields.moisture, profile.scale),
        "#864",
        "#7B6",
      );
      return true;
    case "acacia":
      if (submergedSurface) {
        return false;
      }
      configureTreeFeature(
        out,
        FEATURE_OAK,
        scaledFeatureHeight(26, 14, fields.temperature, profile.scale),
        scaledFeatureRadius(13, 4, fields.moisture, profile.scale),
        "#754",
        "#8A5",
      );
      out.featureExtra = 3;
      return true;
    case "cactus":
      if (submergedSurface) {
        return false;
      }
      configureTreeFeature(
        out,
        FEATURE_CACTUS,
        scaledFeatureHeight(14, 18, fields.temperature, profile.scale),
        scaledFeatureRadius(2, 1, fields.surfacePatch, profile.scale),
        "#596",
        "#7B8",
      );
      out.featureExtra = profile.variant > 1 ? 2 : 1 + Math.floor(fields.surfacePatch * 2);
      return true;
    case "dead_snag":
      if (submergedSurface) {
        return false;
      }
      configureTreeFeature(
        out,
        FEATURE_DEAD_TREE,
        scaledFeatureHeight(22, 18, fields.uplift + fields.surfacePatch * 0.5, profile.scale),
        scaledFeatureRadius(6, 3, fields.scatter + fields.desolation * 0.3, profile.scale),
        profile.variant > 0 ? "#433" : "#654",
        profile.variant > 0 ? "#654" : "#876",
      );
      out.featureExtra = profile.variant > 0 ? 3 : 1;
      return true;
    case "hoodoo":
      configureSpireFeature(
        out,
        FEATURE_HOODOO,
        scaledFeatureHeight(24, 28, fields.mesa, profile.scale),
        scaledFeatureRadius(6, 4, fields.mesa, profile.scale),
        profile.variant > 0 ? "#A65" : "#B75",
        profile.variant > 0 ? "#D97" : "#EBA",
      );
      return true;
    case "fir":
      if (submergedSurface) {
        return false;
      }
      configureTreeFeature(
        out,
        FEATURE_FIR,
        scaledFeatureHeight(36, 24, fields.uplift, profile.scale),
        scaledFeatureRadius(8, 4, fields.moisture, profile.scale),
        "#764",
        "#6A7",
      );
      return true;
    case "tall_fir":
      if (submergedSurface) {
        return false;
      }
      configureTreeFeature(
        out,
        FEATURE_FIR,
        scaledFeatureHeight(54, 32, fields.uplift, profile.scale),
        scaledFeatureRadius(10, 5, fields.moisture, profile.scale),
        "#654",
        "#6B7",
      );
      return true;
    case "ice_spire":
      configureSpireFeature(
        out,
        FEATURE_ICE_SPIRE,
        scaledFeatureHeight(28, 22, 1 - fields.temperature, profile.scale),
        scaledFeatureRadius(5, 3, fields.uplift, profile.scale),
        "#CDE",
        "#EFF",
      );
      return true;
    case "frost_shrub":
      if (submergedSurface) {
        return false;
      }
      configureTreeFeature(
        out,
        FEATURE_BUSH,
        scaledFeatureHeight(3, 4, 1 - fields.temperature, profile.scale),
        scaledFeatureRadius(4, 2, fields.moisture, profile.scale),
        "#88A",
        "#CDE",
      );
      return true;
    case "cypress":
      if (submergedSurface) {
        return false;
      }
      configureTreeFeature(
        out,
        FEATURE_CYPRESS,
        scaledFeatureHeight(30, 18, fields.drainage, profile.scale),
        scaledFeatureRadius(7, 4, fields.moisture, profile.scale),
        "#554",
        "#486",
      );
      return true;
    case "mangrove":
      if (submergedSurface || (waterTopY === NO_WATER && fields.channel < 0.56)) {
        return false;
      }
      configureTreeFeature(
        out,
        FEATURE_CYPRESS,
        scaledFeatureHeight(34, 22, fields.moisture, profile.scale),
        scaledFeatureRadius(10, 5, fields.moisture, profile.scale),
        "#654",
        "#597",
      );
      return true;
    case "reed_cluster":
      if (waterTopY === NO_WATER && fields.channel < 0.58) {
        return false;
      }
      configureTreeFeature(
        out,
        FEATURE_REEDS,
        scaledFeatureHeight(8, 8, fields.moisture, profile.scale),
        scaledFeatureRadius(2, 1, fields.moisture, profile.scale),
        "#684",
        "#8A6",
      );
      out.featureExtra = 1;
      return true;
    case "crystal_reeds":
      if (waterTopY === NO_WATER && fields.channel < 0.54 && fields.moisture < 0.64) {
        return false;
      }
      configureSpireFeature(
        out,
        FEATURE_CRYSTAL,
        scaledFeatureHeight(15, 16, fields.magic + fields.moisture * 0.25, profile.scale),
        scaledFeatureRadius(5, 3, fields.channel + fields.magic * 0.2, profile.scale),
        "#68A",
        "#CEF",
        "#DFF",
      );
      out.featureExtra = 3;
      return true;
    case "basalt_spire":
      configureSpireFeature(
        out,
        FEATURE_BASALT_SPIRE,
        scaledFeatureHeight(28, 30, fields.volcanism, profile.scale),
        scaledFeatureRadius(5, 3, fields.volcanism, profile.scale),
        "#433",
        "#F74",
      );
      return true;
    case "ash_marker":
      configureSpireFeature(
        out,
        FEATURE_BASALT_SPIRE,
        scaledFeatureHeight(20, 22, fields.volcanism + fields.desolation * 0.2, profile.scale),
        scaledFeatureRadius(3, 2, fields.volcanism + fields.scatter * 0.2, profile.scale),
        "#544",
        "#C86",
      );
      out.featureExtra = 3;
      return true;
    case "pilgrim_lantern":
      configureSpireFeature(
        out,
        FEATURE_BASALT_SPIRE,
        scaledFeatureHeight(22, 18, fields.volcanism + fields.magic * 0.25, profile.scale),
        scaledFeatureRadius(3, 2, fields.scatter + fields.magic * 0.2, profile.scale),
        "#544",
        "#DA8",
        "#FC6",
      );
      out.featureExtra = 4;
      return true;
    case "bone_chimes":
      configureSpireFeature(
        out,
        FEATURE_BASALT_SPIRE,
        scaledFeatureHeight(28, 20, fields.desolation + fields.magic * 0.2, profile.scale),
        scaledFeatureRadius(6, 3, fields.scatter + fields.surfacePatch * 0.2, profile.scale),
        "#322",
        "#CBA",
        "#DA8",
      );
      out.featureExtra = 5;
      return true;
    case "ashlander_travel_pack":
      if (submergedSurface) {
        return false;
      }
      configureSpireFeature(
        out,
        FEATURE_TRAVEL_PACK,
        scaledFeatureHeight(12, 8, fields.desolation + fields.surfacePatch * 0.35, profile.scale),
        scaledFeatureRadius(6, 2, fields.scatter + fields.desolation * 0.22, profile.scale),
        "#764",
        "#BA8",
        "#322",
      );
      out.featureExtra = profile.variant;
      return true;
    case "paver_debris":
      if (submergedSurface) {
        return false;
      }
      configureSpireFeature(
        out,
        FEATURE_ROAD_DEBRIS,
        scaledFeatureHeight(3, 4, fields.scatter + fields.surfacePatch * 0.25, profile.scale),
        scaledFeatureRadius(10, 5, fields.scatter + fields.desolation * 0.2, profile.scale),
        "#655",
        "#887",
        "#433",
      );
      out.featureExtra = profile.variant;
      return true;
    case "scree_fan":
      if (submergedSurface) {
        return false;
      }
      configureSpireFeature(
        out,
        FEATURE_ROAD_DEBRIS,
        scaledFeatureHeight(4, 5, fields.scatter + fields.desolation * 0.22, profile.scale),
        scaledFeatureRadius(12, 6, fields.scatter + fields.uplift * 0.18, profile.scale),
        "#433",
        "#765",
        "#B75",
      );
      out.featureExtra = 1;
      return true;
    case "shrine_debris":
      if (submergedSurface) {
        return false;
      }
      configureSpireFeature(
        out,
        FEATURE_ROAD_DEBRIS,
        scaledFeatureHeight(5, 5, fields.magic + fields.surfacePatch * 0.2, profile.scale),
        scaledFeatureRadius(8, 5, fields.scatter + fields.magic * 0.2, profile.scale),
        "#544",
        "#A87",
        "#DA8",
      );
      out.featureExtra = 2;
      return true;
    case "buried_ribs":
      if (submergedSurface) {
        return false;
      }
      configureSpireFeature(
        out,
        FEATURE_BURIED_RIBS,
        scaledFeatureHeight(8, 8, fields.desolation + fields.surfacePatch * 0.25, profile.scale),
        scaledFeatureRadius(11, 5, fields.scatter + fields.desolation * 0.2, profile.scale),
        "#665",
        "#CBA",
        "#432",
      );
      out.featureExtra = profile.variant;
      return true;
    case "ash_obelisk":
      configureSpireFeature(
        out,
        FEATURE_MEGASTRUCTURE,
        scaledFeatureHeight(42, 24, fields.volcanism + fields.uplift * 0.25, profile.scale),
        scaledFeatureRadius(8, 3, fields.volcanism + fields.surfacePatch * 0.2, profile.scale),
        "#322",
        "#F74",
        "#DA8",
      );
      out.featureExtra = 2;
      return true;
    case "crystal_cluster":
      configureSpireFeature(
        out,
        FEATURE_CRYSTAL,
        scaledFeatureHeight(14, 18, fields.magic, profile.scale),
        scaledFeatureRadius(4, 3, fields.magic, profile.scale),
        profile.variant >= 3 ? "#A7C" : "#79B",
        profile.variant >= 2 ? "#EFF" : "#CEF",
      );
      out.featureExtra = 1;
      return true;
    case "glass_cairn":
      configureSpireFeature(
        out,
        FEATURE_CRYSTAL,
        scaledFeatureHeight(8, 12, fields.magic + fields.surfacePatch * 0.2, profile.scale),
        scaledFeatureRadius(5, 2, fields.magic + fields.surfacePatch * 0.3, profile.scale),
        "#8AC",
        "#EFF",
      );
      out.featureExtra = 2;
      return true;
    case "silt_shell":
      configureSpireFeature(
        out,
        FEATURE_HOODOO,
        scaledFeatureHeight(10, 10, fields.desolation + fields.surfacePatch * 0.3, profile.scale),
        scaledFeatureRadius(8, 4, fields.scatter + fields.desolation * 0.2, profile.scale),
        "#876",
        "#CBA",
      );
      out.featureExtra = 2;
      return true;
    case "rib_arch":
      configureSpireFeature(
        out,
        FEATURE_RIB_ARCH,
        scaledFeatureHeight(20, 16, fields.desolation + fields.surfacePatch * 0.25, profile.scale),
        scaledFeatureRadius(12, 5, fields.scatter + fields.desolation * 0.2, profile.scale),
        "#665",
        "#CBA",
        "#432",
      );
      out.featureExtra = 1;
      return true;
    case "rib_remains":
      configureSpireFeature(
        out,
        FEATURE_RIB_ARCH,
        scaledFeatureHeight(12, 10, fields.channel + fields.surfacePatch * 0.2, profile.scale),
        scaledFeatureRadius(9, 4, fields.scatter + fields.moisture * 0.2, profile.scale),
        "#776",
        "#CBA",
        "#544",
      );
      out.featureExtra = 2;
      return true;
    case "velothi_shrine":
      configureSpireFeature(
        out,
        FEATURE_STANDING_STONE,
        scaledFeatureHeight(18, 14, fields.uplift + fields.magic * 0.25, profile.scale),
        scaledFeatureRadius(4, 2, fields.surfacePatch + fields.magic * 0.2, profile.scale),
        "#665",
        "#DA8",
      );
      out.featureExtra = 2;
      return true;
    case "velothi_ziggurat":
      configureSpireFeature(
        out,
        FEATURE_MEGASTRUCTURE,
        scaledFeatureHeight(46, 32, fields.uplift + fields.magic * 0.25, profile.scale),
        scaledFeatureRadius(14, 5, fields.surfacePatch + fields.scatter * 0.2, profile.scale),
        "#433",
        "#DA8",
        "#C86",
      );
      out.featureExtra = 1;
      return true;
    case "kwama_mound":
      configureSpireFeature(
        out,
        FEATURE_BOULDER,
        scaledFeatureHeight(6, 8, fields.desolation + fields.surfacePatch * 0.3, profile.scale),
        scaledFeatureRadius(7, 3, fields.scatter + fields.surfacePatch * 0.3, profile.scale),
        "#765",
        "#DB8",
      );
      out.featureExtra = 2;
      return true;
    case "pilgrim_cairn":
      configureSpireFeature(
        out,
        FEATURE_STANDING_STONE,
        scaledFeatureHeight(12, 10, fields.uplift + fields.magic * 0.2, profile.scale),
        scaledFeatureRadius(5, 2, fields.scatter + fields.surfacePatch * 0.2, profile.scale),
        "#776",
        "#EDB",
      );
      out.featureExtra = 1;
      return true;
    case "old_road_causeway":
      configureSpireFeature(
        out,
        FEATURE_CAUSEWAY,
        scaledFeatureHeight(3, 3, fields.surfacePatch + fields.scatter * 0.2, profile.scale),
        scaledFeatureRadius(12, 5, fields.scatter + fields.uplift * 0.15, profile.scale),
        "#766",
        "#BA8",
      );
      out.featureExtra = profile.variant;
      return true;
    case "fungal_bridge":
      configureSpireFeature(
        out,
        FEATURE_CAUSEWAY,
        scaledFeatureHeight(7, 4, fields.magic + fields.moisture * 0.25, profile.scale),
        scaledFeatureRadius(14, 6, fields.channel + fields.grove * 0.25, profile.scale),
        "#465",
        "#8CF",
        "#6A8",
      );
      out.featureExtra = 3;
      return true;
    case "glowcap":
      if (submergedSurface) {
        return false;
      }
      configureTreeFeature(
        out,
        FEATURE_GLOWCAP,
        scaledFeatureHeight(20, 20, fields.magic, profile.scale),
        scaledFeatureRadius(10, 5, fields.magic, profile.scale),
        "#79B",
        "#8CF",
      );
      out.featureExtra = 2;
      return true;
    case "mega_glowcap":
      if (submergedSurface) {
        return false;
      }
      configureTreeFeature(
        out,
        FEATURE_GLOWCAP,
        scaledFeatureHeight(38, 28, fields.magic, profile.scale),
        scaledFeatureRadius(16, 6, fields.magic, profile.scale),
        "#68A",
        "#AEF",
      );
      out.featureExtra = 3;
      return true;
    case "root_stump":
      if (submergedSurface) {
        return false;
      }
      configureSpireFeature(
        out,
        FEATURE_BOULDER,
        scaledFeatureHeight(7, 6, fields.oldGrowth + fields.moisture * 0.2, profile.scale),
        scaledFeatureRadius(6, 2, fields.oldGrowth + fields.scatter * 0.2, profile.scale),
        "#654",
        "#875",
      );
      out.featureExtra = 0;
      return true;
    case "stone_tor":
      configureSpireFeature(
        out,
        FEATURE_HOODOO,
        scaledFeatureHeight(18, 18, fields.uplift + fields.scatter * 0.25, profile.scale),
        scaledFeatureRadius(5, 3, fields.uplift + fields.scatter * 0.2, profile.scale),
        "#889",
        "#BBC",
      );
      return true;
    default:
      return false;
  }
}

function configureTreeFeature(
  out: MutableColumnState,
  featureKind: number,
  height: number,
  radius: number,
  materialPrimary: string,
  materialSecondary: string,
  materialAccent?: string,
): void {
  out.featureKind = featureKind;
  out.featureHeight = height;
  out.featureRadius = radius;
  out.featureExtra = 0;
  out.featureMaterialPrimary = hexColorToMaterial(materialPrimary);
  out.featureMaterialSecondary = hexColorToMaterial(materialSecondary);
  out.featureMaterialAccent = materialAccent ? hexColorToMaterial(materialAccent) : 0;
}

function configureSpireFeature(
  out: MutableColumnState,
  featureKind: number,
  height: number,
  radius: number,
  materialPrimary: string,
  materialSecondary: string,
  materialAccent?: string,
): void {
  out.featureKind = featureKind;
  out.featureHeight = height;
  out.featureRadius = radius;
  out.featureExtra = 1;
  out.featureMaterialPrimary = hexColorToMaterial(materialPrimary);
  out.featureMaterialSecondary = hexColorToMaterial(materialSecondary);
  out.featureMaterialAccent = materialAccent ? hexColorToMaterial(materialAccent) : 0;
}

function scaledFeatureHeight(base: number, jitterRange: number, signal: number, scale: number): number {
  return Math.max(1, Math.round((base + Math.floor(signal * jitterRange)) * scale));
}

function scaledFeatureRadius(base: number, jitterRange: number, signal: number, scale: number): number {
  return Math.max(1, Math.round((base + signal * jitterRange) * scale));
}

function hasStandingWater(surfaceY: number, waterTopY: number): boolean {
  return waterTopY !== NO_WATER && waterTopY > surfaceY;
}

function shouldUseFeatureAccent(
  featureDeltaX: number,
  featureDeltaZ: number,
  relativeY: number,
  featureExtra: number,
): boolean {
  const hash = Math.abs(
    featureDeltaX * 31
      + featureDeltaZ * 17
      + relativeY * 13
      + featureExtra * 19,
  );
  const cadence = featureExtra >= 8 ? 7 : featureExtra >= 7 ? 5 : 6;
  return hash % cadence === 0;
}

function sampleMaterialFromScratch(
  scratch: ChunkGenerationScratch,
  columnIndex: number,
  worldY: number,
  worldYDiv3: number,
  worldYBandBase: number,
): number {
  const featureMaterial = sampleFeatureMaterial(
    scratch.featureKind[columnIndex]!,
    scratch.featureHeight[columnIndex]!,
    scratch.featureRadius[columnIndex]!,
    scratch.featureExtra[columnIndex]!,
    scratch.featureDeltaX[columnIndex]!,
    scratch.featureDeltaZ[columnIndex]!,
    scratch.featureMaterialPrimary[columnIndex]!,
    scratch.featureMaterialSecondary[columnIndex]!,
    scratch.featureMaterialAccent[columnIndex]!,
    scratch.surfaceY[columnIndex]!,
    worldY,
  );
  if (featureMaterial !== 0) {
    return featureMaterial;
  }
  const surfaceY = scratch.surfaceY[columnIndex]!;
  if (worldY > surfaceY) {
    const waterTopY = scratch.waterTopY[columnIndex]!;
    return waterTopY !== NO_WATER && worldY <= waterTopY ? scratch.waterMaterial[columnIndex]! : 0;
  }
  if (worldY === surfaceY) {
    if (
      sampleCaveSurfaceBreach(
        surfaceY,
        scratch.caveEntranceField[columnIndex]!,
        scratch.caveEntranceStrength[columnIndex]!,
        scratch.caveEntranceCenterY[columnIndex]!,
        scratch.caveEntranceHalfHeight[columnIndex]!,
      )
    ) {
      return 0;
    }
    return resolveSurfaceTransitionMaterial(
      scratch.surfacePrimary[columnIndex]!,
      scratch.surfaceSecondary[columnIndex]!,
      scratch.transitionThreshold[columnIndex]!,
      scratch.worldXDiv3[columnIndex]!,
      worldYDiv3,
      scratch.worldZDiv3[columnIndex]!,
      scratch.ditherSeed[columnIndex]!,
    );
  }
  if (sampleCaveVoid(
    surfaceY,
    worldY,
    scratch.caveMainField[columnIndex]!,
    scratch.caveMainStrength[columnIndex]!,
    scratch.caveMainCenterY[columnIndex]!,
    scratch.caveMainHalfHeight[columnIndex]!,
    scratch.caveUpperField[columnIndex]!,
    scratch.caveUpperStrength[columnIndex]!,
    scratch.caveUpperCenterY[columnIndex]!,
    scratch.caveUpperHalfHeight[columnIndex]!,
    scratch.caveEntranceField[columnIndex]!,
    scratch.caveEntranceStrength[columnIndex]!,
    scratch.caveEntranceCenterY[columnIndex]!,
    scratch.caveEntranceHalfHeight[columnIndex]!,
  )) {
    return 0;
  }
  if (worldY >= surfaceY - 4) {
    return resolveTransitionMaterial(
      scratch.subsurfacePrimary[columnIndex]!,
      scratch.subsurfaceSecondary[columnIndex]!,
      scratch.transitionThreshold[columnIndex]!,
      scratch.worldXDiv3[columnIndex]!,
      worldYDiv3,
      scratch.worldZDiv3[columnIndex]!,
      scratch.ditherSeed[columnIndex]! + 17,
    );
  }
  if (worldY < 24) {
    return scratch.deepStoneMaterial[columnIndex]!;
  }
  const accentNoise = hashNoise3D(
    scratch.worldXDiv3[columnIndex]!,
    worldYDiv3,
    scratch.worldZDiv3[columnIndex]!,
    scratch.accentSeed[columnIndex]!,
  );
  if (worldY < surfaceY - 18 && accentNoise > 0.992) {
    return scratch.accentMaterial[columnIndex]!;
  }
  const band = Math.abs(Math.floor(worldYBandBase + scratch.strataOffset[columnIndex]!)) % 3;
  if (band === 0) {
    return scratch.stoneMaterial[columnIndex]!;
  }
  if (band === 1) {
    return scratch.deepStoneMaterial[columnIndex]!;
  }
  return scratch.subsurfacePrimary[columnIndex]!;
}

function sampleCaveVoid(
  surfaceY: number,
  worldY: number,
  caveMainField: number,
  caveMainStrength: number,
  caveMainCenterY: number,
  caveMainHalfHeight: number,
  caveUpperField: number,
  caveUpperStrength: number,
  caveUpperCenterY: number,
  caveUpperHalfHeight: number,
  caveEntranceField: number,
  caveEntranceStrength: number,
  caveEntranceCenterY: number,
  caveEntranceHalfHeight: number,
): boolean {
  if (worldY <= 24 || worldY >= surfaceY) {
    return false;
  }
  if (
    sampleCaveLayer(
      surfaceY,
      worldY,
      caveMainField,
      caveMainStrength,
      caveMainCenterY,
      caveMainHalfHeight,
      8,
      0,
    )
  ) {
    return true;
  }
  return sampleCaveLayer(
    surfaceY,
    worldY,
    caveEntranceField,
    caveEntranceStrength,
    caveEntranceCenterY,
    caveEntranceHalfHeight,
    5,
    -0.24,
  ) || sampleCaveLayer(
    surfaceY,
    worldY,
    caveUpperField,
    caveUpperStrength,
    caveUpperCenterY,
    caveUpperHalfHeight,
    5,
    -0.12,
  );
}

function sampleCaveSurfaceBreach(
  surfaceY: number,
  field: number,
  strength: number,
  centerY: number,
  halfHeight: number,
): boolean {
  if (strength <= 0 || halfHeight <= 0 || centerY <= 0) {
    return false;
  }
  const depthToCenter = surfaceY - centerY;
  if (depthToCenter < 0) {
    return false;
  }
  const reach = halfHeight + 1.5;
  if (depthToCenter > reach) {
    return false;
  }
  const verticalShape = 1 - depthToCenter / Math.max(1, reach);
  const breachScore = field + strength * 0.38 + verticalShape * 0.58;
  const breachThreshold = 1.02 - strength * 0.52;
  return breachScore >= breachThreshold;
}

function sampleCaveLayer(
  surfaceY: number,
  worldY: number,
  field: number,
  strength: number,
  centerY: number,
  halfHeight: number,
  minimumRoofDepth: number,
  thresholdBias: number,
): boolean {
  if (strength <= 0 || halfHeight <= 0 || surfaceY - worldY < minimumRoofDepth) {
    return false;
  }
  const verticalDistance = Math.abs(worldY - centerY);
  if (verticalDistance > halfHeight) {
    return false;
  }
  const verticalShape = 1 - verticalDistance / Math.max(1, halfHeight + 0.75);
  const carveThreshold = 0.90 - strength * 0.20 + thresholdBias;
  const carveScore = field + verticalShape * (0.34 + strength * 0.22);
  return carveScore >= carveThreshold;
}

function sampleFeatureMaterial(
  featureKind: number,
  featureHeight: number,
  featureRadius: number,
  featureExtra: number,
  featureDeltaX: number,
  featureDeltaZ: number,
  materialPrimary: number,
  materialSecondary: number,
  materialAccent: number,
  surfaceY: number,
  worldY: number,
): number {
  if (featureKind === FEATURE_NONE) {
    return 0;
  }
  const relativeY = worldY - (surfaceY + 1);
  if (relativeY < 0 || relativeY > featureHeight) {
    return 0;
  }
  const absX = Math.abs(featureDeltaX);
  const absZ = Math.abs(featureDeltaZ);
  const radial = Math.hypot(featureDeltaX, featureDeltaZ);
  switch (featureKind) {
    case FEATURE_OAK: {
      const canopyVariant = featureExtra >= 7 ? 0 : featureExtra >= 4 ? 2 : featureExtra >= 2 ? 1 : 0;
      const slender = featureExtra === 1;
      const trunkCutoff = canopyVariant === 0
        ? featureHeight - 4
        : featureHeight - Math.max(6, Math.round(featureHeight * (canopyVariant === 1 ? 0.24 : 0.28)));
      if (relativeY <= trunkCutoff) {
        const trunkRadius = slender ? 0.75 : Math.min(1.45, 0.85 + featureRadius * 0.06 + canopyVariant * 0.12);
        return absX <= trunkRadius && absZ <= trunkRadius ? materialPrimary : 0;
      }
      const canopyCenter = canopyVariant === 0 ? featureHeight - 2 : featureHeight - Math.max(2, Math.round(featureHeight * 0.10));
      const canopyFalloff = canopyVariant === 0 ? (slender ? 0.7 : 0.9) : canopyVariant === 1 ? 0.38 : 0.50;
      const canopyBaseRadius = canopyVariant === 0 ? (slender ? 1.25 : 1.5) : featureRadius * (canopyVariant === 1 ? 0.72 : 0.78);
      if (radial > Math.max(canopyBaseRadius, featureRadius - Math.abs(relativeY - canopyCenter) * canopyFalloff)) {
        return 0;
      }
      if (materialAccent !== 0 && featureExtra >= 7 && shouldUseFeatureAccent(featureDeltaX, featureDeltaZ, relativeY, featureExtra)) {
        return materialAccent;
      }
      return materialSecondary;
    }
    case FEATURE_BOULDER: {
      if (featureExtra >= 2) {
        const moundHeight = Math.max(3, Math.round(featureHeight * 0.76));
        if (relativeY > moundHeight) {
          return 0;
        }
        const yProgress = relativeY / Math.max(1, moundHeight);
        const layerWidthX = Math.max(1.6, featureRadius * (1.18 - yProgress * 0.58));
        const layerWidthZ = Math.max(1.2, featureRadius * (0.78 - yProgress * 0.38));
        const oval = Math.hypot(featureDeltaX / layerWidthX, featureDeltaZ / layerWidthZ);
        if (oval > 1) {
          return 0;
        }
        const eggY = Math.max(1, Math.round(featureHeight * 0.36));
        const leftEgg = Math.hypot((featureDeltaX + featureRadius * 0.32) / 1.7, (featureDeltaZ - featureRadius * 0.18) / 1.25);
        const rightEgg = Math.hypot((featureDeltaX - featureRadius * 0.36) / 1.55, (featureDeltaZ + featureRadius * 0.14) / 1.2);
        const centerEgg = Math.hypot(featureDeltaX / 1.45, (featureDeltaZ + featureRadius * 0.34) / 1.15);
        if (Math.abs(relativeY - eggY) <= 1 && (leftEgg <= 1 || rightEgg <= 1 || centerEgg <= 1)) {
          return materialSecondary;
        }
        if (relativeY >= moundHeight - 1 && oval <= 0.62) {
          return materialSecondary;
        }
        return materialPrimary;
      }
      const bodyRadius = Math.max(1.1, featureRadius - Math.abs(relativeY - featureHeight * 0.45) * 0.55);
      const topCapRadius = Math.min(bodyRadius, Math.max(0.9, featureRadius * 0.58));
      if (relativeY === featureHeight && radial <= topCapRadius) {
        return materialSecondary;
      }
      return radial <= bodyRadius ? materialPrimary : 0;
    }
    case FEATURE_MEGASTRUCTURE: {
      if (featureExtra >= 2) {
        const plinthHeight = Math.max(4, Math.round(featureHeight * 0.13));
        if (relativeY <= plinthHeight) {
          const plinthRadius = featureRadius + 1.8 - relativeY * 0.38;
          if (radial > Math.max(2.2, plinthRadius)) {
            return 0;
          }
          const plinthCourse = relativeY % 2 === 0 || Math.abs(absX - absZ) <= 0.65;
          return plinthCourse ? materialSecondary : materialPrimary;
        }
        const towerY = relativeY - plinthHeight;
        const towerProgress = towerY / Math.max(1, featureHeight - plinthHeight);
        const towerRadius = Math.max(1.0, featureRadius * (0.42 - towerProgress * 0.28));
        if (radial > towerRadius) {
          return 0;
        }
        const glyphBand = towerY % 7 === 0 && absX <= towerRadius && absZ <= 0.84;
        const verticalInlay = materialAccent !== 0
          && towerY > 4
          && towerY < featureHeight - plinthHeight - 3
          && (
            (absX <= 0.72 && featureDeltaZ < 0)
            || (absZ <= 0.72 && featureDeltaX > 0)
          )
          && towerY % 3 !== 1;
        const crown = relativeY >= featureHeight - 3;
        const narrowWindow = !crown
          && !glyphBand
          && !verticalInlay
          && towerY > 7
          && towerY < featureHeight - plinthHeight - 7
          && (
            (absX <= 0.52 && absZ > 1.05 && absZ < towerRadius - 0.25)
            || (absZ <= 0.52 && featureDeltaX < 0 && absX > 1.05 && absX < towerRadius - 0.25)
          )
          && towerY % 11 >= 2
          && towerY % 11 <= 6;
        if (narrowWindow) {
          return 0;
        }
        return verticalInlay ? materialAccent : glyphBand || crown ? materialSecondary : materialPrimary;
      }

      const baseHeight = Math.max(10, Math.round(featureHeight * 0.36));
      if (relativeY <= baseHeight) {
        const stepHeight = Math.max(2, Math.round(baseHeight / 5));
        const step = Math.min(5, Math.floor(relativeY / stepHeight));
        const tierHalfX = Math.max(3.2, featureRadius + 3.4 - step * 2.15);
        const tierHalfZ = Math.max(2.4, featureRadius * 0.72 + 2.4 - step * 1.45);
        const chamfer = Math.max(2.4, 4.6 - step * 0.45);
        if (absX > tierHalfX || absZ > tierHalfZ || absX + absZ > tierHalfX + tierHalfZ - chamfer) {
          return 0;
        }
        const edgeCourse = Math.abs(absX - tierHalfX) <= 0.65 || Math.abs(absZ - tierHalfZ) <= 0.65;
        const stairCut = featureDeltaZ < 0 && absX <= Math.max(1.1, 1.8 + step * 0.35) && relativeY > stepHeight;
        const cornerBreak = step >= 1
          && relativeY > 1
          && absX > tierHalfX - 2.2
          && absZ > tierHalfZ - 1.8
          && (
            (featureDeltaX > 0 && featureDeltaZ > 0 && step % 2 === 0)
            || (featureDeltaX < 0 && featureDeltaZ < 0 && step % 3 !== 1)
          );
        const innerVoid = step >= 2
          && relativeY > stepHeight * 1.5
          && relativeY < baseHeight - 2
          && absX < tierHalfX - 2.6
          && absZ < tierHalfZ - 1.8
          && (
            featureDeltaZ > Math.max(0.2, tierHalfZ * 0.06)
            || (absX > 1.8 && absZ < tierHalfZ * 0.48 && step >= 3)
          );
        const forecourtVoid = step >= 1
          && relativeY > stepHeight
          && featureDeltaZ < -Math.max(1.6, tierHalfZ * 0.16)
          && absX < Math.max(2.4, tierHalfX * 0.32)
          && relativeY < baseHeight - 1;
        const sideButtress = step >= 1
          && relativeY <= baseHeight - 1
          && absZ <= Math.max(1.4, tierHalfZ * 0.36)
          && (
            Math.abs(absX - Math.max(2.8, tierHalfX - 2.4)) <= 1.05
            || (step >= 3 && Math.abs(absX - Math.max(2.2, tierHalfX - 4.8)) <= 0.85)
          );
        if (stairCut) {
          return 0;
        }
        if (!sideButtress && (cornerBreak || innerVoid || forecourtVoid)) {
          return 0;
        }
        const warmInlay = materialAccent !== 0
          && !edgeCourse
          && step >= 2
          && relativeY % stepHeight === Math.max(1, Math.floor(stepHeight / 2))
          && (
            Math.abs(absX - Math.max(2.4, tierHalfX - 4.2)) <= 0.55
            || Math.abs(absZ - Math.max(1.8, tierHalfZ - 3.2)) <= 0.55
          )
          && (Math.floor(absX + absZ + relativeY) % 3 !== 0);
        return warmInlay ? materialAccent : edgeCourse || relativeY % stepHeight === 0 ? materialSecondary : materialPrimary;
      }

      const towerY = relativeY - baseHeight;
      const towerHalfX = Math.max(1.9, featureRadius * 0.34 - towerY * 0.030);
      const towerHalfZ = Math.max(1.4, featureRadius * 0.24 - towerY * 0.024);
      if (absX <= towerHalfX && absZ <= towerHalfZ) {
        const slit = absX <= 0.55 && featureDeltaZ < 0 && towerY > 4 && towerY < Math.max(7, featureHeight * 0.18);
        if (slit) {
          return 0;
        }
        const crown = relativeY >= featureHeight - 4;
        const glyphBand = towerY % 8 === 0 && (absX <= 0.75 || absZ <= 0.75);
        const sideWindow = materialAccent !== 0
          && !crown
          && !glyphBand
          && towerY > 10
          && towerY < featureHeight - baseHeight - 8
          && towerY % 13 >= 4
          && towerY % 13 <= 7
          && (
            (absX <= 0.55 && absZ > 0.72 && absZ < towerHalfZ - 0.2)
            || (absZ <= 0.55 && featureDeltaX > 0 && absX > 0.72 && absX < towerHalfX - 0.2)
          );
        if (sideWindow) {
          return 0;
        }
        const warmGlyph = materialAccent !== 0 && glyphBand && towerY % 16 === 0;
        return warmGlyph ? materialAccent : crown || glyphBand ? materialSecondary : materialPrimary;
      }
      return 0;
    }
    case FEATURE_RIB_ARCH: {
      const pillarOffset = Math.max(4.2, featureRadius * 0.72);
      const archBaseY = Math.max(5, Math.round(featureHeight * 0.54));
      const archRise = Math.max(4, featureHeight - archBaseY);
      const ribPlanes = featureExtra >= 2 ? [-5, -2, 1, 4] : [-4, -1, 2, 5];
      const ribPlaneDistance = Math.min(...ribPlanes.map((ribZ) => Math.abs(featureDeltaZ - ribZ)));
      const ribThickness = featureExtra >= 2 ? 1.45 : 1.25;
      const leftPillar = Math.abs(featureDeltaX + pillarOffset) <= 1.35 && ribPlaneDistance <= ribThickness && relativeY <= archBaseY + 1;
      const rightPillar = Math.abs(featureDeltaX - pillarOffset) <= 1.35 && ribPlaneDistance <= ribThickness && relativeY <= archBaseY + 1;
      if (leftPillar || rightPillar) {
        return materialPrimary;
      }
      const normalizedX = Math.min(1, absX / Math.max(1, pillarOffset));
      const archY = archBaseY + Math.round((1 - normalizedX * normalizedX) * archRise);
      const onArch = absX <= pillarOffset + 1.85 && ribPlaneDistance <= ribThickness && Math.abs(relativeY - archY) <= 1;
      if (onArch) {
        const chippedEdge = materialAccent !== 0 && ribPlaneDistance <= 0.48 && (Math.round(absX) + relativeY) % 5 === 0;
        return chippedEdge ? materialAccent : materialSecondary;
      }
      const spine = featureExtra >= 2
        && absX <= 1.25
        && absZ <= Math.max(5.8, featureRadius * 0.58)
        && relativeY <= Math.max(3, Math.round(archBaseY * 0.52));
      const brokenRib = ribPlaneDistance <= 0.8
        && relativeY <= archBaseY
        && absX < pillarOffset - 2.2
        && (Math.round(absX) + relativeY + Math.round(absZ)) % 7 === 0;
      if (spine && materialAccent !== 0 && (Math.round(featureDeltaZ) + relativeY) % 4 === 0) {
        return materialAccent;
      }
      if (brokenRib && materialAccent !== 0 && (Math.round(absX) + relativeY) % 4 === 0) {
        return materialAccent;
      }
      return brokenRib || spine ? materialPrimary : 0;
    }
    case FEATURE_CAUSEWAY: {
      if (featureExtra >= 3) {
        const capBaseY = Math.max(2, Math.round(featureHeight * 0.36));
        const capTopY = Math.min(featureHeight, capBaseY + 3);
        const halfLength = featureRadius + 1.8;
        const halfWidth = Math.max(4.2, featureRadius * 0.40);
        const capProfile = Math.hypot(featureDeltaX / halfLength, featureDeltaZ / halfWidth);
        const centerStalk = radial <= Math.max(1.05, featureRadius * 0.10)
          && relativeY <= capBaseY + 1;
        const buttress = relativeY <= capBaseY
          && absX <= Math.max(1.6, featureRadius * 0.16 + relativeY * 0.16)
          && absZ <= Math.max(1.2, featureRadius * 0.11 + relativeY * 0.12);
        if (centerStalk || buttress) {
          return materialPrimary;
        }
        if (relativeY < capBaseY || relativeY > capTopY || capProfile > 1) {
          return 0;
        }
        const rim = capProfile > 0.74 || Math.abs(absZ - halfWidth * 0.42) <= 0.62;
        const gill = relativeY === capBaseY && Math.floor((featureDeltaX + featureRadius) / 3) % 2 === 0;
        const capSpot = materialAccent !== 0
          && relativeY >= capTopY - 1
          && Math.abs((featureDeltaX * 5 + featureDeltaZ * 7 + relativeY * 3) % 11) <= 1;
        return capSpot ? materialAccent : rim || gill ? materialSecondary : materialPrimary;
      }
      const slabHeight = Math.max(1, Math.min(featureHeight, 4));
      if (relativeY > slabHeight) {
        return 0;
      }
      const longAxis = absX <= featureRadius + 1.5 && absZ <= 2.4;
      const crossAxis = absZ <= Math.max(4.0, featureRadius * 0.42) && absX <= 2.0;
      const oldRoadShoulder = materialAccent === 0
        && relativeY <= Math.max(1, slabHeight - 1)
        && absX <= featureRadius * 0.78
        && absZ > 3.0
        && absZ <= Math.max(5.2, featureRadius * 0.44)
        && (Math.floor((featureDeltaX + featureRadius) / 4) + Math.floor(absZ)) % 3 !== 1;
      const oldRoadApproach = materialAccent === 0
        && relativeY <= Math.max(1, slabHeight - 2)
        && absX <= Math.max(3.2, featureRadius * 0.34)
        && absZ <= featureRadius * 0.72
        && absZ > Math.max(4.0, featureRadius * 0.42)
        && Math.floor((featureDeltaZ + featureRadius) / 5) % 2 === 0;
      if (!longAxis && !crossAxis && !oldRoadShoulder && !oldRoadApproach) {
        return 0;
      }
      const edge = absZ > 1.6 || absX > featureRadius - 1 || (crossAxis && absX > 1.2) || oldRoadShoulder || oldRoadApproach;
      const brokenJoint = (Math.floor(featureDeltaX / 3) + Math.floor(featureDeltaZ / 3)) % 3 === 0;
      return edge || (relativeY === slabHeight && brokenJoint) ? materialSecondary : materialPrimary;
    }
    case FEATURE_ROAD_DEBRIS: {
      const slabHeight = Math.max(1, Math.min(featureHeight, 5));
      if (relativeY > slabHeight || radial > featureRadius + 1.5) {
        return 0;
      }
      if (featureExtra === 1) {
        const forward = featureDeltaZ + featureRadius * 0.66;
        const fanWidth = Math.max(2.4, 2.2 + Math.max(0, forward) * 0.64);
        const inFan = forward >= 0
          && featureDeltaZ <= featureRadius * 0.82
          && absX <= fanWidth
          && absX + Math.max(0, -featureDeltaZ * 0.18) <= featureRadius * 0.94;
        const spur = Math.abs(featureDeltaZ + featureDeltaX * 0.36) <= 1.05
          && absX <= featureRadius * 0.76
          && featureDeltaZ > -featureRadius * 0.52;
        if (!inFan && !spur) {
          return 0;
        }
        const localNoise = hashNoise3D(
          Math.floor(featureDeltaX * 0.45),
          relativeY + 11,
          Math.floor(featureDeltaZ * 0.45),
          1231 + featureRadius * 13,
        );
        const top = Math.max(1, Math.min(slabHeight, 1 + Math.floor(localNoise * (slabHeight + 1))));
        if (relativeY > top) {
          return 0;
        }
        const gap = localNoise < 0.16 && relativeY >= top - 1;
        if (gap) {
          return 0;
        }
        const brightChip = materialAccent !== 0 && relativeY === top && localNoise > 0.78;
        return brightChip ? materialAccent : relativeY === top || spur ? materialSecondary : materialPrimary;
      }
      if (featureExtra === 2) {
        const plinthHeight = Math.max(3, Math.min(slabHeight, 4));
        const plinth = absX <= featureRadius * 0.92
          && absZ <= Math.max(4.0, featureRadius * 0.66)
          && absX + absZ <= featureRadius * 1.28
          && relativeY <= plinthHeight;
        const frontStep = featureDeltaZ < -featureRadius * 0.24
          && absX <= featureRadius * 0.70
          && absZ <= featureRadius * 0.88
          && relativeY <= 2;
        const leftShard = Math.abs(featureDeltaX + featureRadius * 0.42) <= 1.1
          && Math.abs(featureDeltaZ - featureRadius * 0.12) <= 1.0
          && relativeY <= slabHeight
          && relativeY >= Math.max(1, Math.floor(absZ * 0.25));
        const rightShard = Math.abs(featureDeltaX - featureRadius * 0.32) <= 1.0
          && Math.abs(featureDeltaZ + featureRadius * 0.20) <= 1.35
          && relativeY <= Math.max(2, slabHeight - 1);
        const rearShard = Math.abs(featureDeltaZ - featureRadius * 0.36) <= 1.1
          && absX <= 1.4
          && relativeY <= Math.max(3, slabHeight - 1);
        if (!plinth && !frontStep && !leftShard && !rightShard && !rearShard) {
          return 0;
        }
        const brokenCorner = plinth
          && relativeY === plinthHeight
          && absX > featureRadius * 0.62
          && absZ > featureRadius * 0.42
          && (Math.floor(absX + absZ) + featureExtra) % 2 === 0;
        if (brokenCorner) {
          return 0;
        }
        const inlay = materialAccent !== 0
          && relativeY >= plinthHeight - 1
          && (absX <= 0.9 || absZ <= 0.9 || leftShard || rearShard)
          && (Math.floor(absX + absZ) % 3 !== 1);
        return inlay ? materialAccent : leftShard || rightShard || rearShard || relativeY >= plinthHeight - 1 ? materialSecondary : materialPrimary;
      }
      const islandA = Math.hypot(
        (featureDeltaX + featureRadius * 0.34) / Math.max(2.6, featureRadius * 0.46),
        (featureDeltaZ - featureRadius * 0.12) / Math.max(1.9, featureRadius * 0.22),
      );
      const islandB = Math.hypot(
        (featureDeltaX - featureRadius * 0.28) / Math.max(2.4, featureRadius * 0.40),
        (featureDeltaZ + featureRadius * 0.28) / Math.max(1.8, featureRadius * 0.24),
      );
      const islandC = Math.hypot(
        (featureDeltaX + featureRadius * 0.04) / Math.max(2.0, featureRadius * 0.28),
        (featureDeltaZ + featureRadius * 0.58) / Math.max(1.6, featureRadius * 0.18),
      );
      const ribLineA = Math.abs(featureDeltaZ - featureDeltaX * 0.38) <= 1.15
        && featureDeltaX > -featureRadius * 0.70
        && featureDeltaX < featureRadius * 0.58;
      const ribLineB = featureExtra >= 2
        && Math.abs(featureDeltaZ + featureDeltaX * 0.52 - featureRadius * 0.18) <= 0.90
        && featureDeltaX > -featureRadius * 0.50
        && featureDeltaX < featureRadius * 0.74;
      const inIsland = islandA <= 1 || islandB <= 1 || islandC <= 1 || ribLineA || ribLineB;
      if (!inIsland) {
        return 0;
      }
      const localNoise = hashNoise3D(
        Math.floor(featureDeltaX * 0.5),
        featureExtra + relativeY,
        Math.floor(featureDeltaZ * 0.5),
        971 + featureRadius * 17,
      );
      const top = Math.max(1, Math.min(slabHeight, 1 + Math.floor(localNoise * (slabHeight + 1))));
      if (relativeY > top) {
        return 0;
      }
      const paverJoint = (Math.floor((featureDeltaX + featureRadius) / 3) + Math.floor((featureDeltaZ + featureRadius) / 4)) % 4 === 0;
      const chip = localNoise < 0.18 && relativeY >= top - 1;
      if (chip || (paverJoint && relativeY === top)) {
        return 0;
      }
      if (materialAccent !== 0 && (ribLineA || ribLineB || (relativeY === top && localNoise > 0.76))) {
        return materialAccent;
      }
      return relativeY === top || paverJoint ? materialSecondary : materialPrimary;
    }
    case FEATURE_TRAVEL_PACK: {
      const bodyHeight = Math.max(4, Math.round(featureHeight * 0.46));
      const bedrollY = Math.max(bodyHeight + 2, Math.round(featureHeight * 0.66));
      const frameTopY = Math.max(bedrollY + 2, featureHeight - 1);
      const bodyProgress = relativeY / Math.max(1, bodyHeight);
      const bodyHalfX = Math.max(2.6, featureRadius * (0.98 - bodyProgress * 0.24));
      const bodyHalfZ = Math.max(1.9, featureRadius * (0.58 - bodyProgress * 0.18));
      const bodyOval = Math.hypot(featureDeltaX / bodyHalfX, (featureDeltaZ + featureRadius * 0.05) / bodyHalfZ);

      if (relativeY <= bodyHeight && bodyOval <= 1) {
        const bottomMat = relativeY <= 1 || bodyOval > 0.76;
        const strap = materialAccent !== 0
          && (
            Math.abs(featureDeltaX) <= 0.55
            || Math.abs(featureDeltaX - featureRadius * 0.42) <= 0.48
            || (Math.abs(featureDeltaZ) <= 0.48 && relativeY >= 2)
          );
        return strap ? materialAccent : bottomMat ? materialPrimary : materialSecondary;
      }

      const frameOffsetX = Math.max(3.2, featureRadius * 0.66);
      const frameZ = Math.max(1.4, featureRadius * 0.36);
      const leftFrame = Math.abs(featureDeltaX + frameOffsetX) <= 0.58
        && Math.abs(featureDeltaZ - frameZ) <= 0.58
        && relativeY <= frameTopY;
      const rightFrame = Math.abs(featureDeltaX - frameOffsetX) <= 0.58
        && Math.abs(featureDeltaZ - frameZ) <= 0.58
        && relativeY <= frameTopY;
      const topFrame = Math.abs(relativeY - frameTopY) <= 0
        && absX <= frameOffsetX + 0.8
        && Math.abs(featureDeltaZ - frameZ) <= 0.58;
      if (leftFrame || rightFrame || topFrame) {
        return materialAccent || materialPrimary;
      }

      const bedrollTube = absX <= featureRadius + 1.2
        && Math.hypot((featureDeltaZ + featureRadius * 0.18) / 1.75, (relativeY - bedrollY) / 2.15) <= 1;
      if (bedrollTube) {
        const rolledEnd = absX > featureRadius - 0.8;
        const lash = materialAccent !== 0
          && (Math.abs(featureDeltaX) <= 0.48 || Math.abs(absX - featureRadius * 0.52) <= 0.48);
        return lash ? materialAccent : rolledEnd ? materialPrimary : materialSecondary;
      }

      const potX = featureRadius * 0.78;
      const potY = Math.max(3, Math.round(bodyHeight * 0.64));
      const sidePot = Math.hypot((featureDeltaX - potX) / 1.45, (featureDeltaZ + 1.2) / 1.15) <= 1
        && Math.abs(relativeY - potY) <= 2;
      if (sidePot) {
        const rim = Math.abs(relativeY - potY) === 2 || Math.abs(featureDeltaX - potX) > 1.05;
        return rim ? materialAccent || materialPrimary : materialSecondary;
      }

      const bedrollCord = materialAccent !== 0
        && relativeY > bodyHeight
        && relativeY < bedrollY
        && absZ <= 0.55
        && (Math.abs(featureDeltaX + featureRadius * 0.36) <= 0.48 || Math.abs(featureDeltaX - featureRadius * 0.28) <= 0.48);
      return bedrollCord ? materialAccent : 0;
    }
    case FEATURE_BURIED_RIBS: {
      const lowHeight = Math.max(4, Math.min(featureHeight, 14));
      if (relativeY > lowHeight) {
        return 0;
      }
      const ribPlanes = [-0.48, -0.24, 0.02, 0.28, 0.52].map((fraction) => fraction * featureRadius);
      const ribPlaneDistance = Math.min(...ribPlanes.map((ribZ) => Math.abs(featureDeltaZ - ribZ)));
      const span = Math.max(5.2, featureRadius * 0.88);
      const normalizedX = Math.min(1, absX / span);
      const archY = 1 + Math.round((1 - normalizedX * normalizedX) * lowHeight * 0.82);
      const brokenGap = hashNoise3D(
        Math.floor((featureDeltaX + 32) * 0.25),
        Math.floor(relativeY * 0.5),
        Math.floor((featureDeltaZ + 32) * 0.25),
        1559 + featureExtra,
      ) < 0.10;
      const onRib = absX <= span
        && ribPlaneDistance <= 0.95
        && Math.abs(relativeY - archY) <= 1
        && !brokenGap;
      if (onRib) {
        const darkCrack = materialAccent !== 0 && ribPlaneDistance <= 0.32 && (Math.round(absX) + relativeY) % 4 === 0;
        return darkCrack ? materialAccent : materialSecondary;
      }
      const buriedSpine = absX <= 1.25
        && absZ <= featureRadius * 0.62
        && relativeY <= Math.max(2, Math.round(lowHeight * 0.28));
      const knuckle = radial <= Math.max(1.3, featureRadius * 0.16)
        && relativeY <= Math.max(3, Math.round(lowHeight * 0.34));
      if (buriedSpine || knuckle) {
        return materialPrimary;
      }
      return 0;
    }
    case FEATURE_BUSH:
      if (relativeY === 0 && absX <= 0.55 && absZ <= 0.55) {
        return materialPrimary;
      }
      if (radial > Math.max(1.1, featureRadius - relativeY * 0.6)) {
        return 0;
      }
      if (materialAccent !== 0 && featureExtra >= 5 && shouldUseFeatureAccent(featureDeltaX, featureDeltaZ, relativeY, featureExtra)) {
        return materialAccent;
      }
      return materialSecondary;
    case FEATURE_STANDING_STONE:
      if (featureExtra >= 2) {
        const baseHeight = Math.min(4, Math.max(2, Math.round(featureHeight * 0.16)));
        if (relativeY <= baseHeight) {
          const baseRadius = featureRadius + 1.25 - relativeY * 0.28;
          return radial <= Math.max(1.4, baseRadius) ? materialPrimary : 0;
        }
        const roofBaseY = Math.max(baseHeight + 3, featureHeight - 5);
        if (relativeY >= roofBaseY) {
          const roofStep = relativeY - roofBaseY;
          const roofHalfX = Math.max(2.6, featureRadius + 2.25 - roofStep * 0.65);
          const roofHalfZ = Math.max(1.1, featureRadius * 0.48 - roofStep * 0.20);
          return absX <= roofHalfX && absZ <= roofHalfZ ? materialSecondary : 0;
        }
        const doorwayHeight = Math.min(6, Math.max(3, Math.round(featureHeight * 0.26)));
        if (relativeY <= baseHeight + doorwayHeight && absX <= 0.65 && featureDeltaZ < 0) {
          return 0;
        }
        const columnRadius = Math.max(1.0, featureRadius * 0.48 - (relativeY - baseHeight) * 0.025);
        return radial <= columnRadius ? materialPrimary : 0;
      }
      return radial <= Math.max(1.1, featureRadius - relativeY * 0.2) ? materialPrimary : 0;
    case FEATURE_PALM: {
      const trunkHeight = Math.max(4, featureHeight - Math.max(4, Math.round(featureHeight * 0.20)));
      const trunkRadius = Math.min(1.15, 0.75 + featureRadius * 0.05);
      if (relativeY <= trunkHeight) {
        return absX <= trunkRadius && absZ <= trunkRadius ? materialPrimary : 0;
      }
      const crownOffset = featureHeight - relativeY;
      if (crownOffset === 1) {
        return absX + absZ <= featureRadius + 0.9 || radial <= Math.max(1.8, featureRadius * 0.42)
          ? materialSecondary
          : 0;
      }
      if (crownOffset === 0) {
        return radial <= Math.max(1.6, featureRadius * 0.34) ? materialSecondary : 0;
      }
      return absX + absZ <= Math.max(1.8, featureRadius * 0.55) && radial <= featureRadius + 0.45
        ? materialSecondary
        : 0;
    }
    case FEATURE_CACTUS: {
      const armY = Math.max(2, featureHeight - 2);
      if (relativeY <= armY) {
        if (absX <= 0.55 && absZ <= 0.55) {
          return materialPrimary;
        }
        if (featureExtra > 0 && relativeY === armY && ((featureDeltaX === 1 && absZ <= 0.55) || (featureDeltaZ === 1 && absX <= 0.55))) {
          return materialPrimary;
        }
        return 0;
      }
      return absX <= 0.55 && absZ <= 0.55 ? materialSecondary : 0;
    }
    case FEATURE_HOODOO:
      if (relativeY === featureHeight && radial <= featureRadius + 0.7) {
        return materialSecondary;
      }
      return radial <= Math.max(1.1, featureRadius - relativeY * 0.22) ? materialPrimary : 0;
    case FEATURE_FIR: {
      const trunkHeight = Math.max(4, Math.round(featureHeight * 0.22));
      const trunkRadius = Math.min(1.2, 0.7 + featureRadius * 0.05);
      if (relativeY <= trunkHeight) {
        return absX <= trunkRadius && absZ <= trunkRadius ? materialPrimary : 0;
      }
      const crownProgress = (relativeY - trunkHeight) / Math.max(1, featureHeight - trunkHeight);
      const crownRadius = featureRadius * (1 - crownProgress * 0.82);
      const lowerSkirt = smoothstep(0, 0.28, 1 - crownProgress) * 0.8;
      return radial <= Math.max(1.2, crownRadius + lowerSkirt) ? materialSecondary : 0;
    }
    case FEATURE_ICE_SPIRE:
      return radial <= Math.max(0.8, featureRadius - relativeY * 0.35) ? materialSecondary : 0;
    case FEATURE_CYPRESS: {
      const willow = featureExtra >= 1;
      const trunkHeight = willow
        ? Math.max(4, Math.round(featureHeight * 0.22))
        : Math.max(3, Math.round(featureHeight * 0.18));
      const trunkRadius = willow
        ? Math.min(1.3, 0.78 + featureRadius * 0.04)
        : Math.min(1.1, 0.72 + featureRadius * 0.04);
      if (relativeY <= trunkHeight) {
        return absX <= trunkRadius && absZ <= trunkRadius ? materialPrimary : 0;
      }
      const crownCenter = willow ? featureHeight * 0.58 : featureHeight * 0.62;
      const crownRadius = willow
        ? featureRadius * 1.04 - Math.abs(relativeY - crownCenter) * 0.12
        : featureRadius - Math.abs(relativeY - crownCenter) * 0.18;
      if (willow && relativeY >= crownCenter && radial <= featureRadius + 0.55 && radial >= Math.max(1.8, featureRadius * 0.58)) {
        return materialSecondary;
      }
      return radial <= Math.max(1.2, crownRadius)
        ? materialSecondary
        : 0;
    }
    case FEATURE_REEDS:
      if (relativeY > featureHeight) {
        return 0;
      }
      return (
        (absX <= 0.45 && absZ <= 0.45)
        || (featureDeltaX === 1 && absZ <= 0.45)
        || (featureDeltaZ === 1 && absX <= 0.45)
      )
        ? materialSecondary
        : 0;
    case FEATURE_BASALT_SPIRE:
      if (featureExtra >= 5) {
        const baseHeight = Math.min(4, Math.max(2, Math.round(featureHeight * 0.11)));
        if (relativeY <= baseHeight) {
          const baseRadius = featureRadius + 1.9 - relativeY * 0.35;
          const lip = relativeY === baseHeight || radial > baseRadius - 0.85;
          return radial <= Math.max(1.7, baseRadius) ? (lip ? materialSecondary : materialPrimary) : 0;
        }

        const crossbarY = Math.max(baseHeight + 10, featureHeight - 8);
        const shoulderY = Math.max(baseHeight + 5, crossbarY - 6);
        const supportOffset = Math.max(3.6, featureRadius * 0.72);
        const postHeight = Math.min(featureHeight, crossbarY + 2);
        const leftPost = Math.abs(featureDeltaX + supportOffset) <= 0.95 && absZ <= 0.85 && relativeY <= postHeight;
        const rightPost = Math.abs(featureDeltaX - supportOffset) <= 0.95 && absZ <= 0.85 && relativeY <= postHeight;
        const centerPost = absX <= 0.72 && absZ <= 0.72 && relativeY <= crossbarY + 3;
        if (leftPost || rightPost || centerPost) {
          const banded = (relativeY - baseHeight) % 6 === 0;
          return banded ? materialSecondary : materialPrimary;
        }

        const crossbarHalf = featureRadius + 3.8;
        const topCrossbar = Math.abs(relativeY - crossbarY) <= 1 && absX <= crossbarHalf && absZ <= 0.85;
        const lowerCrossbar = relativeY === shoulderY && absX <= crossbarHalf * 0.82 && absZ <= 0.65;
        if (topCrossbar || lowerCrossbar) {
          return materialSecondary;
        }

        const hangerOffsets = [-5.2, -2.8, -0.8, 1.7, 4.5] as const;
        for (const offsetX of hangerOffsets) {
          const localX = featureDeltaX - offsetX;
          const localAbsX = Math.abs(localX);
          const hangerTop = crossbarY - 1;
          const hangerBottom = Math.max(baseHeight + 3, crossbarY - 10 - Math.round(Math.abs(offsetX) * 0.35));
          const nearString = localAbsX <= 0.42 && absZ <= 0.42;
          if (nearString && relativeY >= hangerBottom && relativeY <= hangerTop) {
            const bead = materialAccent !== 0 && relativeY % 4 === 0;
            return bead ? materialAccent : materialSecondary;
          }
          const boneY = hangerBottom + Math.round(Math.abs(offsetX) % 3);
          const boneBlade = Math.abs(relativeY - boneY) <= 2 && localAbsX <= 0.82 && absZ <= 0.58;
          if (boneBlade) {
            const edge = localAbsX > 0.55 || Math.abs(relativeY - boneY) === 2;
            return edge && materialAccent !== 0 ? materialAccent : materialSecondary;
          }
        }

        const windTornCord = Math.abs(absX - absZ) <= 0.45
          && absX <= featureRadius * 0.72
          && relativeY >= shoulderY - 3
          && relativeY <= crossbarY - 1
          && (Math.round(absX + relativeY) % 4 === 0);
        return windTornCord ? materialSecondary : 0;
      }
      if (featureExtra >= 4) {
        const baseHeight = Math.min(4, Math.max(2, Math.round(featureHeight * 0.12)));
        if (relativeY <= baseHeight) {
          const baseRadius = featureRadius + 1.7 - relativeY * 0.32;
          const ashLip = relativeY === baseHeight || radial > baseRadius - 0.75;
          return radial <= Math.max(1.5, baseRadius) ? (ashLip ? materialSecondary : materialPrimary) : 0;
        }

        const crossbarY = Math.max(baseHeight + 7, featureHeight - 7);
        const postTopY = Math.min(featureHeight, crossbarY + 2);
        const pole = relativeY <= postTopY && absX <= 1.05 && absZ <= 1.05;
        if (pole) {
          const metalBand = (relativeY - baseHeight) % 5 === 0;
          return metalBand ? materialSecondary : materialPrimary;
        }

        const crossbarHalf = featureRadius + 3.2;
        if (relativeY === crossbarY && absX <= crossbarHalf && absZ <= 1.05) {
          return materialSecondary;
        }
        if (
          relativeY >= crossbarY - 3
          && relativeY < crossbarY
          && absZ <= 0.85
          && Math.abs(absX - Math.max(1.6, featureRadius + 1.4)) <= 0.58
        ) {
          return materialSecondary;
        }

        const lanternX = featureRadius + 1.6;
        const lanternY = crossbarY - 5;
        const localX = featureDeltaX - lanternX;
        const localAbsX = Math.abs(localX);
        const localAbsZ = absZ;
        if (Math.abs(relativeY - (crossbarY - 2)) <= 1 && localAbsX <= 0.85 && localAbsZ <= 0.85) {
          return materialSecondary;
        }
        if (Math.abs(relativeY - lanternY) <= 2 && localAbsX <= 1.85 && localAbsZ <= 1.35) {
          const frame = localAbsX >= 1.55 || localAbsZ >= 1.15 || Math.abs(relativeY - lanternY) === 2;
          return frame ? materialSecondary : materialAccent || materialSecondary;
        }

        const rearCounterweight = Math.abs(featureDeltaX + featureRadius * 0.9) <= 0.75
          && absZ <= 0.75
          && relativeY >= crossbarY - 4
          && relativeY <= crossbarY - 2;
        return rearCounterweight ? materialSecondary : 0;
      }
      if (featureExtra >= 2) {
        const plinthHeight = Math.min(4, Math.max(2, Math.round(featureHeight * 0.12)));
        if (relativeY <= plinthHeight) {
          return radial <= Math.max(1.2, featureRadius + 1.4 - relativeY * 0.30) ? materialSecondary : 0;
        }
        const capBaseY = Math.max(plinthHeight + 4, featureHeight - 4);
        if (featureExtra >= 3) {
          const crossbarY = capBaseY - 1;
          if (relativeY === crossbarY && absZ <= 0.65 && absX <= featureRadius + 2.5) {
            return materialPrimary;
          }
          if (
            relativeY >= crossbarY - 4
            && relativeY < crossbarY
            && absZ <= 0.55
            && (
              Math.abs(absX - (featureRadius + 1)) <= 0.55
              || Math.abs(absX - Math.max(1, featureRadius - 1)) <= 0.55
            )
          ) {
            return materialSecondary;
          }
          if (
            relativeY >= crossbarY - 3
            && relativeY <= crossbarY
            && absX <= 1.0
            && absZ <= 0.8
          ) {
            return materialSecondary;
          }
        }
        if (relativeY >= capBaseY) {
          const capStep = relativeY - capBaseY;
          const capHalfX = Math.max(1.5, featureRadius + 1.1 - capStep * 0.55);
          const capHalfZ = Math.max(1.0, featureRadius * 0.42 - capStep * 0.10);
          if (absX <= capHalfX && absZ <= capHalfZ) {
            return materialSecondary;
          }
        }
        const shaftRadius = Math.max(0.9, featureRadius * 0.66 - (relativeY - plinthHeight) * 0.045);
        return radial <= shaftRadius ? materialPrimary : 0;
      }
      if (relativeY <= 1 + featureExtra && radial <= Math.max(1, featureRadius - relativeY * 0.15)) {
        return materialSecondary;
      }
      return radial <= Math.max(1, featureRadius - relativeY * 0.28) ? materialPrimary : 0;
    case FEATURE_CRYSTAL:
      if (featureExtra >= 3) {
        const reeds = [
          [0, 0, 1.00, 1.00],
          [-3, 2, 0.58, 0.62],
          [3, -2, 0.68, 0.70],
          [2, 4, 0.52, 0.55],
          [5, 1, 0.42, 0.46],
          [1, 6, 0.38, 0.42],
        ] as const;
        for (const [offsetX, offsetZ, heightScale, radiusScale] of reeds) {
          const localX = featureDeltaX - offsetX;
          const localZ = featureDeltaZ - offsetZ;
          const localAbsX = Math.abs(localX);
          const localAbsZ = Math.abs(localZ);
          const localRadial = Math.hypot(localX, localZ);
          const reedHeight = Math.max(5, Math.round(featureHeight * heightScale));
          if (relativeY > reedHeight) {
            continue;
          }
          const baseLift = offsetX === 0 && offsetZ === 0 ? 0 : Math.min(2, Math.max(1, Math.round((1 - heightScale) * 4)));
          if (relativeY < baseLift) {
            continue;
          }
          const reedProgress = (relativeY - baseLift) / Math.max(1, reedHeight - baseLift);
          const reedRadius = Math.max(0.72, featureRadius * radiusScale * (0.34 - reedProgress * 0.22));
          const shardFacet = localAbsX <= reedRadius && localAbsZ <= reedRadius * 0.72;
          const diagonalFacet = Math.abs(localAbsX - localAbsZ) <= 0.62 && localRadial <= reedRadius * 1.15;
          if (!shardFacet && !diagonalFacet) {
            continue;
          }
          const tip = relativeY >= reedHeight - 1;
          const brightEdge = materialAccent !== 0
            && (tip || ((localX + localZ + relativeY + offsetX) % 5 === 0 && localRadial >= reedRadius * 0.42));
          return brightEdge ? materialAccent : tip || localAbsX > reedRadius * 0.52 ? materialSecondary : materialPrimary;
        }
        return 0;
      }
      if (relativeY >= featureHeight - 1 && radial <= Math.max(0.8, featureRadius - relativeY * 0.25)) {
        return materialSecondary;
      }
      return radial <= Math.max(0.9, featureRadius - relativeY * 0.55) ? materialPrimary : 0;
    case FEATURE_REDWOOD: {
      const trunkHeight = Math.max(16, Math.round(featureHeight * 0.78));
      const trunkRadius = Math.max(1.6, featureRadius * 0.17);
      if (relativeY <= trunkHeight) {
        const buttressRadius = relativeY <= Math.max(5, featureHeight * 0.05)
          ? trunkRadius + (1 - relativeY / Math.max(1, featureHeight * 0.05)) * Math.max(1.2, featureRadius * 0.08)
          : trunkRadius;
        return radial <= buttressRadius ? materialPrimary : 0;
      }
      const crownProgress = (relativeY - trunkHeight) / Math.max(1, featureHeight - trunkHeight);
      const crownRadius = featureRadius * (0.70 - crownProgress * 0.38);
      const capBias = crownProgress > 0.72 ? (crownProgress - 0.72) * 4.8 : 0;
      return radial <= Math.max(1.4, crownRadius + capBias) ? materialSecondary : 0;
    }
    case FEATURE_DEAD_TREE: {
      const trunkRadius = Math.max(0.9, featureRadius * 0.16);
      if (radial <= trunkRadius) {
        return materialPrimary;
      }
      const thorny = featureExtra >= 2;
      const branchStart = thorny
        ? Math.max(3, Math.round(featureHeight * 0.24))
        : Math.max(4, Math.round(featureHeight * 0.36));
      if (relativeY < branchStart) {
        return 0;
      }
      const branchSeed = (relativeY + featureExtra * 3) % 6;
      const branchExtent = thorny
        ? Math.max(3, Math.round(featureRadius * (relativeY > featureHeight * 0.60 ? 0.85 : 0.62)))
        : Math.max(2, Math.round(featureRadius * (relativeY > featureHeight * 0.72 ? 0.55 : 0.40)));
      const branchOnX = branchSeed === 0 || branchSeed === 3;
      const branchOnZ = branchSeed === 1 || branchSeed === 4;
      const branchDiagonal = branchSeed === 2 || branchSeed === 5;
      if (branchOnX && absZ <= 0.8 && absX <= trunkRadius + branchExtent) {
        return materialSecondary;
      }
      if (branchOnZ && absX <= 0.8 && absZ <= trunkRadius + branchExtent) {
        return materialSecondary;
      }
      if (branchDiagonal && Math.abs(absX - absZ) <= 0.9 && absX + absZ <= branchExtent + trunkRadius + 1.2) {
        return materialSecondary;
      }
      return 0;
    }
    case FEATURE_GLOWCAP:
      if (relativeY <= featureHeight - (featureExtra >= 4 ? 4 : featureExtra >= 3 ? 5 : 3)) {
        const stemRadius = featureExtra >= 4
          ? Math.min(1.0, 0.62 + featureRadius * 0.03)
          : Math.min(1.2, 0.75 + featureRadius * 0.04);
        return absX <= stemRadius && absZ <= stemRadius ? materialPrimary : 0;
      }
      if (featureExtra >= 4) {
        const petalCenter = featureHeight - 1;
        const petalRadius = featureRadius + 1.2 - Math.abs(relativeY - petalCenter) * 0.75;
        return radial <= Math.max(1.8, petalRadius) ? materialSecondary : 0;
      }
      return relativeY <= featureHeight - 1
        ? radial <= Math.max(featureExtra >= 3 ? 3 : 1.5, featureRadius - Math.abs(relativeY - (featureHeight - 2)) * (featureExtra >= 3 ? 0.45 : 0.8))
          ? materialSecondary
          : 0
        : radial <= featureRadius + (featureExtra >= 3 ? 2 : 0.5) ? materialSecondary : 0;
    default:
      return 0;
  }
}

function resolveTransitionMaterial(
  primary: number,
  secondary: number,
  threshold: number,
  worldXDiv3: number,
  worldYDiv3: number,
  worldZDiv3: number,
  seed: number,
): number {
  if (primary === secondary || threshold >= 0.999) {
    return primary;
  }
  return hashNoise3D(worldXDiv3, worldYDiv3, worldZDiv3, seed) <= threshold ? primary : secondary;
}

function resolveSurfaceTransitionMaterial(
  primary: number,
  secondary: number,
  threshold: number,
  worldXDiv3: number,
  worldYDiv3: number,
  worldZDiv3: number,
  seed: number,
): number {
  if (primary === secondary || threshold >= 0.999) {
    return primary;
  }
  const diagonalA = diagonalStripeStrength(worldXDiv3, worldZDiv3, seed, 23, 5, 3);
  const diagonalB = diagonalStripeStrength(worldXDiv3, worldZDiv3, seed + 37, 31, -4, 7);
  const fracture = Math.max(
    smoothstep(0.70, 0.96, diagonalA),
    smoothstep(0.76, 0.98, diagonalB) * 0.82,
  );
  const grain = hashNoise3D(worldXDiv3, worldYDiv3 + 11, worldZDiv3, seed + 101);
  if (fracture > 0.68 && grain > 0.22) {
    return secondary;
  }
  const adjustedThreshold = clamp(threshold - fracture * 0.20, 0.30, 0.96);
  return hashNoise3D(worldXDiv3, worldYDiv3, worldZDiv3, seed) <= adjustedThreshold ? primary : secondary;
}

function diagonalStripeStrength(
  worldXDiv3: number,
  worldZDiv3: number,
  seed: number,
  period: number,
  xWeight: number,
  zWeight: number,
): number {
  const phase = positiveModulo(worldXDiv3 * xWeight + worldZDiv3 * zWeight + seed, period) / period;
  return 1 - Math.abs(phase * 2 - 1);
}

function positiveModulo(value: number, modulus: number): number {
  return ((value % modulus) + modulus) % modulus;
}

function scoreField(value: number, target: number, spread: number): number {
  return saturate(1 - Math.abs(value - target) / spread);
}

function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t;
}

function avg3(a: number, b: number, c: number): number {
  return (a + b + c) * (1 / 3);
}

function avg4(a: number, b: number, c: number, d: number): number {
  return (a + b + c + d) * 0.25;
}

function avg5(a: number, b: number, c: number, d: number, e: number): number {
  return (a + b + c + d + e) * 0.2;
}

function saturate(value: number): number {
  return clamp(value, 0, 1);
}

function smoothstep(edge0: number, edge1: number, value: number): number {
  if (edge0 === edge1) {
    return value >= edge1 ? 1 : 0;
  }
  const t = saturate((value - edge0) / (edge1 - edge0));
  return t * t * (3 - 2 * t);
}

function acquireChunkGenerationScratch(capacity: number): ChunkGenerationScratch {
  const scratch = chunkGenerationScratchPool.pop();
  if (!scratch || scratch.capacity < capacity) {
    return {
      capacity,
      surfaceY: new Int32Array(capacity),
      waterTopY: new Int32Array(capacity),
      surfacePrimary: new Uint16Array(capacity),
      surfaceSecondary: new Uint16Array(capacity),
      subsurfacePrimary: new Uint16Array(capacity),
      subsurfaceSecondary: new Uint16Array(capacity),
      waterMaterial: new Uint16Array(capacity),
      snowMaterial: new Uint16Array(capacity),
      stoneMaterial: new Uint16Array(capacity),
      deepStoneMaterial: new Uint16Array(capacity),
      accentMaterial: new Uint16Array(capacity),
      transitionThreshold: new Float32Array(capacity),
      caveMainField: new Float32Array(capacity),
      caveMainStrength: new Float32Array(capacity),
      caveMainCenterY: new Int16Array(capacity),
      caveMainHalfHeight: new Int16Array(capacity),
      caveUpperField: new Float32Array(capacity),
      caveUpperStrength: new Float32Array(capacity),
      caveUpperCenterY: new Int16Array(capacity),
      caveUpperHalfHeight: new Int16Array(capacity),
      caveEntranceField: new Float32Array(capacity),
      caveEntranceStrength: new Float32Array(capacity),
      caveEntranceCenterY: new Int16Array(capacity),
      caveEntranceHalfHeight: new Int16Array(capacity),
      strataOffset: new Float32Array(capacity),
      worldXDiv3: new Int32Array(capacity),
      worldZDiv3: new Int32Array(capacity),
      ditherSeed: new Int32Array(capacity),
      accentSeed: new Int32Array(capacity),
      featureKind: new Uint8Array(capacity),
      featureHeight: new Int16Array(capacity),
      featureRadius: new Int16Array(capacity),
      featureExtra: new Int16Array(capacity),
      featureDeltaX: new Int16Array(capacity),
      featureDeltaZ: new Int16Array(capacity),
      featureMaterialPrimary: new Uint16Array(capacity),
      featureMaterialSecondary: new Uint16Array(capacity),
      featureMaterialAccent: new Uint16Array(capacity),
    };
  }
  return scratch;
}

function releaseChunkGenerationScratch(scratch: ChunkGenerationScratch): void {
  if (chunkGenerationScratchPool.length >= CHUNK_GENERATION_SCRATCH_POOL_LIMIT) {
    return;
  }
  chunkGenerationScratchPool.push(scratch);
}
