import { clamp, packRgba } from "./math.ts";
import { fbm2D2, fbm2D3, fbm2D4, fbm2D5, hashNoise3D } from "./noise.ts";
import {
  summarizeGeneratedChunkSurface,
  type GeneratedChunkSurfaceSummary,
} from "./generated-chunk-surface-summary.ts";
import type { ChunkBounds, ChunkCoordinate } from "./types.ts";

export const HEX_COLOR_COUNT = 0x1000;
export const PROCEDURAL_WORLD_MAX_Y = 16_384;
export const PROCEDURAL_WORLD_GENERATION_VERSION = "20260312-persist-v2";

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
  | "stone_tor";

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

interface ColumnFieldSample {
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
  caveRibbon: number;
  cavePocket: number;
  caveDepth: number;
  caveOpenings: number;
}

interface MutableColumnState {
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
  surfaceMaterial: number;
}

export interface ProceduralSurfaceColumnSample {
  biomeId: BiomeId;
  surfaceY: number;
  topY: number;
  waterTopY: number | null;
  surfaceMaterial: number;
  waterMaterial: number | null;
}

export interface ProceduralBiomeProbe extends ProceduralColumnSample {
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
  };
}

export interface GeneratedChunk {
  coord: ChunkCoordinate;
  data: Uint16Array;
  solidCount: number;
  solidBounds: ChunkBounds | null;
  surfaceSummary: GeneratedChunkSurfaceSummary | null;
}

interface BaseBiomeBlendSelection {
  primary: BaseBiomeProfile;
  secondary: BaseBiomeProfile;
  primaryWeight: number;
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
const CHUNK_GENERATION_SCRATCH_POOL_LIMIT = 4;

const BASE_BIOMES: readonly BaseBiomeProfile[] = [
  createBaseBiome("verdant", 0.56, 0.78, 0.28, 0.74, 0.42, 0.18, -10, 0.48, 0.18, 0.40, 0.28, 0.00, 4.4, 1548, "#6A5", "#7B6", "#8B6", "#592", "#677", "#754", "#865", "#49B", "#DDE"),
  createBaseBiome("savanna", 0.72, 0.54, 0.32, 0.56, 0.46, 0.16, -2, 0.50, 0.20, 0.36, 0.18, 0.00, 6.4, 1640, "#BA6", "#CB7", "#DB8", "#C86", "#887", "#986", "#A97", "#5AB", "#EED"),
  createBaseBiome("steppe", 0.62, 0.42, 0.36, 0.52, 0.48, 0.14, 0, 0.54, 0.22, 0.32, 0.18, 0.00, 4.8, 1608, "#9B6", "#CB7", "#BA6", "#CA7", "#887", "#875", "#986", "#4AA", "#DDD"),
  createBaseBiome("dunes", 0.84, 0.16, 0.18, 0.28, 0.30, 0.12, -16, 0.32, 0.10, 0.54, 0.42, 0.00, 8.8, 1710, "#DB6", "#EC9", "#EC7", "#CA5", "#B96", "#B85", "#C96", "#5BC", "#EDC"),
  createBaseBiome("badlands", 0.72, 0.20, 0.58, 0.36, 0.58, 0.16, 18, 0.72, 0.64, 0.38, 0.06, 0.46, 9.6, 1670, "#C75", "#D96", "#D86", "#B54", "#865", "#A54", "#965", "#49B", "#EBC"),
  createBaseBiome("highland", 0.40, 0.56, 0.72, 0.46, 0.72, 0.16, 44, 0.88, 0.62, 0.24, 0.10, 0.06, 7.8, 1518, "#6B7", "#7C8", "#7A8", "#8C7", "#778", "#667", "#889", "#5AD", "#EEF"),
  createBaseBiome("moor", 0.28, 0.68, 0.48, 0.28, 0.54, 0.16, 6, 0.34, 0.16, 0.22, 0.30, 0.00, 5.4, 1532, "#758", "#869", "#97A", "#546", "#667", "#564", "#675", "#357", "#DDE"),
  createBaseBiome("tundra", 0.18, 0.42, 0.86, 0.40, 0.82, 0.12, 78, 0.98, 0.82, 0.16, 0.02, 0.04, 6.2, 1452, "#BCC", "#CDD", "#DDE", "#ABB", "#889", "#99A", "#AAB", "#8CD", "#EEF"),
] as const;

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
]);

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
    landmarkPlacement("boulder", { chance: 0.18, scale: 0.9 }),
  ],
  dunes: [
    landmarkPlacement("palm", { chance: 0.34, scale: 1.22 }),
    landmarkPlacement("cactus", { chance: 0.36, scale: 1.12 }),
    landmarkPlacement("cactus", { chance: 0.24, scale: 1.65, variant: 2, cellSize: 168, radius: 6 }),
    landmarkPlacement("dead_snag", { chance: 0.18, scale: 0.9 }),
    landmarkPlacement("boulder", { chance: 0.16, scale: 0.86 }),
  ],
  badlands: [
    landmarkPlacement("hoodoo", { chance: 0.30, scale: 1.22 }),
    landmarkPlacement("hoodoo", { chance: 0.24, scale: 0.86, variant: 1, cellSize: 156, radius: 6 }),
    landmarkPlacement("dead_snag", { chance: 0.28, scale: 1.24 }),
    landmarkPlacement("standing_stone", { chance: 0.20, scale: 1.18 }),
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
  landmarkPlacement("thorn_tree", { chance: 0.28, scale: 1.12, cellSize: 140, radius: 10 }),
  landmarkPlacement("acacia", { chance: 0.26, scale: 1.08, cellSize: 152, radius: 12 }),
  landmarkPlacement("flower_patch", { chance: 0.22, scale: 0.92, variant: 2, cellSize: 76, radius: 5 }),
  landmarkPlacement("boulder", { chance: 0.18, scale: 0.96 }),
];

const DUNES_GLASS_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("crystal_cluster", { chance: 0.36, scale: 1.18, variant: 2 }),
  landmarkPlacement("palm", { chance: 0.28, scale: 1.18, cellSize: 176, radius: 13 }),
  landmarkPlacement("cactus", { chance: 0.34, scale: 1.08 }),
  landmarkPlacement("standing_stone", { chance: 0.18, scale: 1.08 }),
  landmarkPlacement("boulder", { chance: 0.12, scale: 0.90 }),
];

const BADLANDS_DESOLATE_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("dead_tree", { chance: 0.34, scale: 1.28, cellSize: 148, radius: 8 }),
  landmarkPlacement("hoodoo", { chance: 0.28, scale: 1.18 }),
  landmarkPlacement("standing_stone", { chance: 0.22, scale: 1.20 }),
  landmarkPlacement("boulder", { chance: 0.20, scale: 0.98, variant: 1 }),
];

const BADLANDS_CRATER_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("hoodoo", { chance: 0.34, scale: 1.24 }),
  landmarkPlacement("standing_stone", { chance: 0.26, scale: 1.22 }),
  landmarkPlacement("dead_tree", { chance: 0.30, scale: 1.18, cellSize: 156, radius: 8 }),
  landmarkPlacement("boulder", { chance: 0.20, scale: 1.04, variant: 1 }),
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
  landmarkPlacement("mangrove", { chance: 0.36, scale: 1.18, cellSize: 160, radius: 15 }),
  landmarkPlacement("cypress", { chance: 0.42, scale: 1.12, cellSize: 128, radius: 10 }),
  landmarkPlacement("reed_cluster", { chance: 0.70, scale: 1.12, cellSize: 68, radius: 4 }),
  landmarkPlacement("flower_patch", { chance: 0.18, scale: 0.96, variant: 3, cellSize: 84, radius: 5 }),
];

const MARSH_WILLOW_THICKET_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("willow", { chance: 0.34, scale: 1.10, cellSize: 136, radius: 14 }),
  landmarkPlacement("mangrove", { chance: 0.30, scale: 1.18, cellSize: 154, radius: 15 }),
  landmarkPlacement("cypress", { chance: 0.40, scale: 1.14, cellSize: 118, radius: 10 }),
  landmarkPlacement("reed_cluster", { chance: 0.76, scale: 1.14, cellSize: 64, radius: 4 }),
  landmarkPlacement("flower_patch", { chance: 0.28, scale: 1.00, variant: 3, cellSize: 78, radius: 5 }),
];

const MARSH_BLACKWATER_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("willow", { chance: 0.40, scale: 1.14, cellSize: 128, radius: 14 }),
  landmarkPlacement("cypress", { chance: 0.44, scale: 1.16, cellSize: 118, radius: 10 }),
  landmarkPlacement("reed_cluster", { chance: 0.80, scale: 1.16, cellSize: 60, radius: 4 }),
  landmarkPlacement("glowcap", { chance: 0.18, scale: 0.98, cellSize: 144, radius: 10 }),
];

const FIREFLY_LANTERN_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("lantern_tree", { chance: 0.42, scale: 1.12, cellSize: 124, radius: 12 }),
  landmarkPlacement("glowcap", { chance: 0.42, scale: 1.10, cellSize: 116, radius: 12 }),
  landmarkPlacement("willow", { chance: 0.26, scale: 1.04, cellSize: 136, radius: 14 }),
  landmarkPlacement("reed_cluster", { chance: 0.82, scale: 1.14, cellSize: 60, radius: 4 }),
];

const SALTFLAT_MIRROR_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("salt_spire", { chance: 0.44, scale: 1.18 }),
  landmarkPlacement("crystal_cluster", { chance: 0.28, scale: 1.08, variant: 2 }),
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
  landmarkPlacement("mega_glowcap", { chance: 0.32, scale: 1.20 }),
  landmarkPlacement("glowcap", { chance: 0.48, scale: 1.14 }),
  landmarkPlacement("lantern_tree", { chance: 0.24, scale: 1.02, cellSize: 136, radius: 12 }),
  landmarkPlacement("giant_flower", { chance: 0.24, scale: 1.14, cellSize: 120, radius: 10, variant: 1 }),
];

const FUNGAL_SPORE_GROVE_LANDMARKS: readonly LandmarkProfile[] = [
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
  landmarkPlacement("dead_tree", { chance: 0.28, scale: 1.22, cellSize: 156, radius: 8 }),
  landmarkPlacement("basalt_spire", { chance: 0.28, scale: 1.30 }),
  landmarkPlacement("crystal_cluster", { chance: 0.26, scale: 1.12, variant: 3 }),
  landmarkPlacement("boulder", { chance: 0.18, scale: 0.96, variant: 1 }),
];

const EMBER_CALDERA_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("basalt_spire", { chance: 0.38, scale: 1.34 }),
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
        const worldYDiv3 = Math.floor(worldY * ONE_THIRD);
        const worldYBandBase = worldY * STRATA_BAND_SCALE;
        const planeOffset = y * this.chunkSize + z * chunkArea;
        for (let x = 0; x < this.chunkSize; x += 1) {
          const columnIndex = x + rowOffset;
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
          minX = Math.min(minX, x);
          minY = Math.min(minY, y);
          minZ = Math.min(minZ, z);
          maxX = Math.max(maxX, x + 1);
          maxY = Math.max(maxY, y + 1);
          maxZ = Math.max(maxZ, z + 1);
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
      surfaceSummary: summarizeGeneratedChunkSurface(
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
  ): { fields: ColumnFieldSample; biomePrimaryWeight: number } {
    const fields = this.sampleFields(worldX, worldZ);
    const baseBlend = this.selectBaseBiomes(fields);
    const terrainProfile = blendTerrainProfile(baseBlend.primary, baseBlend.secondary, baseBlend.primaryWeight);
    const biomeSelection = this.selectBiomeClassification(fields, baseBlend);
    const biomeCore = biomeSelection.biomeCore;
    let surfaceY = this.sampleSurfaceY(fields, terrainProfile, biomeCore);
    const biomeId = biomeSelection.biomeId;
    const specialStrength = biomeSelection.specialStrength;
    surfaceY = adjustSpecialBiomeSurfaceY(this.seaLevel, biomeId, specialStrength, fields, biomeCore, surfaceY);

    const regionalVariant = selectRegionalVariant(biomeId, fields);
    if (regionalVariant) {
      surfaceY += sampleRegionalVariantSurfaceDelta(regionalVariant.id, regionalVariant.strength, fields, biomeCore);
    }
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
    out.hostBiomeId = hostBiomeId;
    out.secondaryBiomeId = baseBlend.secondary.id;
    out.undergroundBiomeId = undergroundBiomeId;
    out.regionalVariantId = regionalVariant?.id ?? null;
    out.regionalVariantStrength = regionalVariant?.strength ?? 0;
    out.landmarkId = landmarkId;
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
    out.caveMainField = 0;
    out.caveMainStrength = 0;
    out.caveMainCenterY = 0;
    out.caveMainHalfHeight = 0;
    out.caveUpperField = 0;
    out.caveUpperStrength = 0;
    out.caveUpperCenterY = 0;
    out.caveUpperHalfHeight = 0;
    out.caveEntranceField = 0;
    out.caveEntranceStrength = 0;
    out.caveEntranceCenterY = 0;
    out.caveEntranceHalfHeight = 0;
    out.strataOffset = fields.strata * 5;
    out.worldXDiv3 = Math.floor(worldX * ONE_THIRD);
    out.worldZDiv3 = Math.floor(worldZ * ONE_THIRD);
    out.ditherSeed = this.transitionSeed + baseBlend.primary.surface + baseBlend.secondary.surface;
    out.accentSeed = this.seed + underground.accent;
    return { fields, biomePrimaryWeight: baseBlend.primaryWeight };
  }

  private fillColumnState(
    worldX: number,
    worldZ: number,
    out: MutableColumnState,
  ): void {
    const surfaceContext = this.fillSurfaceColumnState(worldX, worldZ, out);
    out.caveMainField = 0;
    out.caveMainStrength = 0;
    out.caveMainCenterY = 0;
    out.caveMainHalfHeight = 0;
    out.caveUpperField = 0;
    out.caveUpperStrength = 0;
    out.caveUpperCenterY = 0;
    out.caveUpperHalfHeight = 0;
    out.caveEntranceField = 0;
    out.caveEntranceStrength = 0;
    out.caveEntranceCenterY = 0;
    out.caveEntranceHalfHeight = 0;
    this.configureCaveState(
      worldX,
      worldZ,
      out.biomeId,
      out.hostBiomeId,
      out.undergroundBiomeId,
      out.regionalVariantId,
      out.surfaceY,
      out.waterTopY,
      surfaceContext.fields,
      surfaceContext.biomePrimaryWeight,
      out,
    );
  }

  private sampleFields(worldX: number, worldZ: number): ColumnFieldSample {
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
    return {
      temperature: fbm2D4(worldX * TEMPERATURE_SCALE, worldZ * TEMPERATURE_SCALE, this.temperatureSeed),
      moisture: fbm2D4(worldX * MOISTURE_SCALE, worldZ * MOISTURE_SCALE, this.moistureSeed),
      uplift,
      drainage: fbm2D3(worldX * DRAINAGE_SCALE, worldZ * DRAINAGE_SCALE, this.drainageSeed),
      volcanism: fbm2D3(worldX * VOLCANISM_SCALE, worldZ * VOLCANISM_SCALE, this.volcanismSeed),
      magic: fbm2D3(worldX * MAGIC_SCALE, worldZ * MAGIC_SCALE, this.magicSeed),
      globalHeight,
      mountainness,
      oceanness,
      continentalness,
      hills,
      detail,
      ridge,
      basin,
      channel: 1 - Math.abs(fbm2D2(worldX * CHANNEL_SCALE, worldZ * CHANNEL_SCALE, this.channelSeed) * 2 - 1),
      dune: 1 - Math.abs(fbm2D2(worldX * DUNE_SCALE, worldZ * DUNE_SCALE, this.duneSeed) * 2 - 1),
      mesa: smoothstep(0.54, 0.84, fbm2D2(worldX * MESA_SCALE, worldZ * MESA_SCALE, this.mesaSeed)),
      grove: fbm2D3(worldX * GROVE_SCALE, worldZ * GROVE_SCALE, this.groveSeed),
      oldGrowth: fbm2D3(worldX * OLD_GROWTH_SCALE, worldZ * OLD_GROWTH_SCALE, this.oldGrowthSeed),
      orchard: fbm2D3(worldX * ORCHARD_SCALE, worldZ * ORCHARD_SCALE, this.orchardSeed),
      desolation: fbm2D3(worldX * DESOLATION_SCALE, worldZ * DESOLATION_SCALE, this.desolationSeed),
      strata: fbm2D2(worldX * STRATA_SCALE, worldZ * STRATA_SCALE, this.strataSeed),
      surfacePatch: fbm2D3(worldX * SURFACE_PATCH_SCALE, worldZ * SURFACE_PATCH_SCALE, this.surfacePatchSeed),
      surfaceGrain: fbm2D2(worldX * SURFACE_GRAIN_SCALE, worldZ * SURFACE_GRAIN_SCALE, this.surfaceGrainSeed),
      scatter: fbm2D2(worldX * SURFACE_SCATTER_SCALE, worldZ * SURFACE_SCATTER_SCALE, this.surfaceScatterSeed),
      peakness,
      caveRibbon: 1 - Math.abs(fbm2D2(worldX * CAVE_RIBBON_SCALE, worldZ * CAVE_RIBBON_SCALE, this.caveRibbonSeed) * 2 - 1),
      cavePocket: fbm2D3(worldX * CAVE_POCKET_SCALE, worldZ * CAVE_POCKET_SCALE, this.cavePocketSeed),
      caveDepth: fbm2D3(worldX * CAVE_DEPTH_SCALE, worldZ * CAVE_DEPTH_SCALE, this.caveDepthSeed),
      caveOpenings: fbm2D2(worldX * CAVE_OPENING_SCALE, worldZ * CAVE_OPENING_SCALE, this.caveOpeningSeed),
    };
  }

  private selectBaseBiomes(fields: ColumnFieldSample): BaseBiomeBlendSelection {
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
    const total = primaryScore + secondaryScore;
    return {
      primary,
      secondary,
      primaryWeight: total <= 0 ? 1 : primaryScore / total,
    };
  }

  private selectBiomeClassification(
    fields: ColumnFieldSample,
    baseBlend: BaseBiomeBlendSelection,
  ): {
    biomeId: BiomeId;
    specialStrength: number;
    biomeCore: number;
  } {
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
      "tundra",
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
    const marshStrength = saturate(
      wetLowlandHost
      * smoothstep(0.50, 0.74, fields.moisture)
      * smoothstep(0.44, 0.68, fields.drainage)
      * smoothstep(0.32, 0.72, fields.channel)
      * smoothstep(0.18, 0.82, flatness),
    );
    const fireflyStrength = averageSignal(
      wetLowlandHost,
      smoothstep(0.56, 0.80, fields.moisture),
      smoothstep(0.48, 0.74, fields.magic),
      smoothstep(0.42, 0.74, fields.grove + fields.channel * 0.25),
      smoothstep(0.20, 0.82, flatness),
    );
    const saltflatStrength = averageSignal(
      dryLowlandHost,
      smoothstep(0.34, 0.72, fields.oceanness + Math.max(0, -fields.basin) * 0.45 + fields.channel * 0.18),
      smoothstep(0.42, 0.74, 1 - fields.moisture),
      smoothstep(0.18, 0.82, flatness),
      smoothstep(0.36, 0.72, 1 - fields.globalHeight + fields.oceanness * 0.35),
    );
    const fernStrength = averageSignal(
      warmLushHost,
      smoothstep(0.50, 0.76, fields.temperature),
      smoothstep(0.56, 0.82, fields.moisture),
      smoothstep(0.42, 0.76, 1 - fields.drainage + Math.max(0, -fields.basin) * 0.55 + fields.channel * 0.15),
      smoothstep(0.24, 0.84, flatness),
    );
    const fungalStrength = averageSignal(
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
    const shardlandsStrength = averageSignal(
      aridShardHost,
      smoothstep(0.40, 0.72, fields.magic + fields.volcanism * 0.20),
      smoothstep(0.46, 0.78, fields.volcanism + fields.ridge * 0.16),
      smoothstep(0.34, 0.72, 1 - fields.moisture),
      smoothstep(0.44, 0.78, fields.surfacePatch + fields.dune * 0.25 + fields.mesa * 0.20),
    );

    let biomeId: BiomeId = baseBlend.primary.id;
    let specialStrength = 0;
    const specialCandidates = [
      { id: "marsh" as const, strength: marshStrength, threshold: 0.30 },
      { id: "firefly" as const, strength: fireflyStrength, threshold: 0.56 },
      { id: "saltflat" as const, strength: saltflatStrength, threshold: 0.78 },
      { id: "fern" as const, strength: fernStrength, threshold: 0.58 },
      { id: "fungal" as const, strength: fungalStrength, threshold: 0.58 },
      { id: "ember" as const, strength: emberStrength, threshold: 0.54 },
      { id: "bloom" as const, strength: bloomStrength, threshold: 0.42 },
      { id: "shardlands" as const, strength: shardlandsStrength, threshold: 0.76 },
    ];
    let selectedSpecial: (typeof specialCandidates)[number] | null = null;
    for (const candidate of specialCandidates) {
      if (candidate.strength <= candidate.threshold) {
        continue;
      }
      if (selectedSpecial === null || candidate.strength > selectedSpecial.strength) {
        selectedSpecial = candidate;
      }
    }
    if (selectedSpecial) {
      biomeId = selectedSpecial.id;
      specialStrength = selectedSpecial.strength;
    }
    return { biomeId, specialStrength, biomeCore };
  }

  private sampleBiomeIdQuick(worldX: number, worldZ: number): BiomeId {
    const fields = this.sampleFields(worldX, worldZ);
    const baseBlend = this.selectBaseBiomes(fields);
    return this.selectBiomeClassification(fields, baseBlend).biomeId;
  }

  private measureBiomeBoundarySuppression(worldX: number, worldZ: number, biomeId: BiomeId): number {
    const step = 32;
    let mismatches = 0;
    for (const [offsetX, offsetZ] of [
      [step, 0],
      [-step, 0],
      [0, step],
      [0, -step],
    ] as const) {
      if (this.sampleBiomeIdQuick(worldX + offsetX, worldZ + offsetZ) !== biomeId) {
        mismatches += 1;
      }
    }
    if (mismatches === 0) {
      return 1;
    }
    if (mismatches === 1) {
      return 0.45;
    }
    if (mismatches === 2) {
      return 0.2;
    }
    return 0.08;
  }

  private sampleSurfaceY(
    fields: ColumnFieldSample,
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
    const peakProvinceLift = 1.25 * peakProvince * (84 + fields.globalHeight * 280 + fields.uplift * 160);
    const peakCrown = 1.25 * peakProvince
      * smoothstep(0.62, 0.88, fields.ridge)
      * (36 + fields.mountainness * 110);
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
      : lerp(preTerrace, Math.round(preTerrace / 8) * 8, terrainProfile.terraceScale * localWeight);
    const microRelief = Math.round(
      (fields.surfaceGrain - 0.5) * terrainProfile.microRelief * (0.35 + biomeCore * 0.65),
    );
    return Math.floor(clamp(terracedHeight + microRelief, 8, this.maxYExclusive - 2));
  }

  private resolveSurfaceMaterials(
    biomeId: BiomeId,
    primary: BaseBiomeProfile,
    secondary: BaseBiomeProfile,
    primaryWeight: number,
    specialStrength: number,
    fields: ColumnFieldSample,
    biomeCore: number,
    surfaceY: number,
  ): {
    surfacePrimary: number;
    surfaceSecondary: number;
    subsurfacePrimary: number;
    subsurfaceSecondary: number;
    water: number;
    snow: number;
    transitionThreshold: number;
  } {
    if (biomeId === primary.id) {
      const primarySurface = selectSurfaceMaterial(primary, fields, biomeCore, surfaceY);
      const primarySubsurface = selectSubsurfaceMaterial(primary, fields, biomeCore, surfaceY);
      const secondarySurface = selectSurfaceMaterial(secondary, fields, 1 - biomeCore * 0.5, surfaceY);
      const secondarySubsurface = selectSubsurfaceMaterial(secondary, fields, 1 - biomeCore * 0.5, surfaceY);
      return {
        surfacePrimary: primarySurface,
        surfaceSecondary: primary === secondary ? primarySurface : secondarySurface,
        subsurfacePrimary: primarySubsurface,
        subsurfaceSecondary: primary === secondary ? primarySubsurface : secondarySubsurface,
        water: primary.water,
        snow: primary.snow,
        transitionThreshold: primary === secondary ? 1 : clamp(0.56 + biomeCore * 0.24 + (primaryWeight - 0.5) * 0.30, 0.52, 0.90),
      };
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
    return {
      surfacePrimary: primarySurface,
      surfaceSecondary: special.softTransition ? hostSurface : primarySurface,
      subsurfacePrimary: primarySubsurface,
      subsurfaceSecondary: special.softTransition ? hostSubsurface : primarySubsurface,
      water: special.water,
      snow: special.snow,
      transitionThreshold: specialThreshold,
    };
  }

  private resolveWaterTopY(
    biomeId: BiomeId,
    surfaceY: number,
    fields: ColumnFieldSample,
    specialStrength: number,
    regionalVariantId: RegionalVariantId | null,
    regionalVariantStrength: number,
  ): number {
    let waterTopY = surfaceY < this.seaLevel ? this.seaLevel : NO_WATER;
    if (biomeId === "marsh") {
      const extraWaterDepth = Math.round(lerp(1, 3, specialStrength));
      waterTopY = Math.max(waterTopY, surfaceY + extraWaterDepth);
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
    fields: ColumnFieldSample,
  ): UndergroundBiomeId {
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
    fields: ColumnFieldSample,
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
    const roster = selectLandmarkRoster(biomeId, undergroundBiomeId, regionalVariantId, fields);
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
    worldX: number,
    worldZ: number,
    biomeId: BiomeId,
    hostBiomeId: BaseBiomeId,
    undergroundBiomeId: UndergroundBiomeId,
    regionalVariantId: RegionalVariantId | null,
    surfaceY: number,
    waterTopY: number,
    fields: ColumnFieldSample,
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
    const mainField = averageSignal(
      smoothstep(0.52, 0.82, fields.caveRibbon),
      smoothstep(0.42, 0.78, fields.cavePocket),
      smoothstep(0.34, 0.72, fields.caveDepth + fields.drainage * 0.18 + basinness * 0.20),
      smoothstep(0.24, 0.72, subterraneanDryness + ruggedness * 0.18),
    );
    const upperField = averageSignal(
      smoothstep(0.56, 0.84, fields.caveOpenings),
      smoothstep(0.36, 0.76, ruggedness + basinness * 0.22 + fields.channel * 0.16),
      smoothstep(0.36, 0.74, fields.caveRibbon + fields.cavePocket * 0.18),
    );
    const entranceField = averageSignal(
      smoothstep(0.50, 0.82, fields.caveOpenings),
      smoothstep(0.42, 0.80, ruggedness + fields.channel * 0.22 + basinness * 0.12),
      smoothstep(0.34, 0.72, fields.caveRibbon + fields.cavePocket * 0.24),
    );
    const mainStrength = saturate(mainField * (0.62 + deepAffinity * 0.58) * waterSuppression);
    const upperStrength = saturate(
      (
        upperField * (0.34 + upperAffinity * caveInterior * 1.22)
        + ruggedness * 0.18 * caveInterior
      ) * waterSuppression,
    );
    const boundaryFactor = biomePrimaryWeight > 0.72
      ? this.measureBiomeBoundarySuppression(worldX, worldZ, biomeId)
      : 0.25;
    const entranceStrength = saturate(
      entranceField
      * caveEntranceCliff
      * (0.62 + upperAffinity * 1.45 + caveInterior * 0.90)
      * boundaryFactor
      * waterSuppression,
    );
    const mainCenterY = clamp(
      surfaceY - Math.round(lerp(20, 104, fields.caveDepth) + fields.globalHeight * 12 + deepAffinity * 14),
      36,
      surfaceY - 10,
    );
    const mainHalfHeight = clamp(
      Math.round(lerp(4, 18, fields.cavePocket) + deepAffinity * 8 + fields.magic * 2),
      0,
      30,
    );
    const upperDepth = clamp(
      Math.round(
        lerp(12, 2, fields.caveOpenings)
        + (1 - ruggedness) * 4
        + (1 - upperAffinity) * 3
        - caveInterior * 2,
      ),
      2,
      18,
    );
    const upperCenterY = clamp(surfaceY - upperDepth, 20, surfaceY - 2);
    const upperHalfHeight = clamp(
      Math.round(lerp(6, 14, fields.cavePocket) + upperAffinity * 6 + ruggedness * 4),
      0,
      22,
    );
    const entranceDepth = clamp(
      Math.round(
        lerp(10, 3, caveEntranceCliff)
        + (1 - fields.caveOpenings) * 2
        + (1 - upperAffinity) * 2
        - caveInterior,
      ),
      2,
      12,
    );
    const entranceCenterY = clamp(surfaceY - entranceDepth, 20, surfaceY - 2);
    const entranceHalfHeight = clamp(
      Math.round(lerp(5, 9, fields.cavePocket) + upperAffinity * 4 + ruggedness * 2 + caveEntranceCliff * 5),
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
      return resolveTransitionMaterial(
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
    surfaceMaterial: state.surfaceMaterialPrimary,
  };
}

function surfaceColumnSampleFromState(state: MutableColumnState): ProceduralSurfaceColumnSample {
  return {
    biomeId: state.biomeId,
    surfaceY: state.surfaceY,
    topY: topYFromState(state),
    waterTopY: nullableWaterTopY(state.waterTopY),
    surfaceMaterial: state.surfaceMaterialPrimary,
    waterMaterial: state.waterTopY === NO_WATER ? null : state.waterMaterial,
  };
}

function scoreBaseBiome(fields: ColumnFieldSample, biome: BaseBiomeProfile): number {
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
  return score;
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
    return primary === "verdant" || primary === "savanna" || primary === "steppe" ? primary : secondary;
  }
  if (biomeId === "firefly") {
    return primary === "verdant" || primary === "savanna" || primary === "moor" || primary === "tundra" ? primary : secondary;
  }
  if (biomeId === "saltflat") {
    return primary === "savanna" || primary === "steppe" || primary === "dunes" ? primary : secondary;
  }
  if (biomeId === "fern") {
    return primary === "verdant" || primary === "savanna" || primary === "highland" ? primary : secondary;
  }
  if (biomeId === "fungal") {
    return primary === "verdant" || primary === "highland" || primary === "moor" ? primary : secondary;
  }
  if (biomeId === "ember") {
    return primary === "badlands" || primary === "highland" ? primary : secondary;
  }
  if (biomeId === "bloom") {
    return primary === "verdant" || primary === "highland" || primary === "moor" ? primary : secondary;
  }
  if (biomeId === "shardlands") {
    return primary === "dunes" || primary === "badlands" || primary === "highland" || primary === "tundra" ? primary : secondary;
  }
  return primary;
}

function selectSurfaceMaterial(
  biome: BaseBiomeProfile,
  fields: ColumnFieldSample,
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
  fields: ColumnFieldSample,
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
  fields: ColumnFieldSample,
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
  fields: ColumnFieldSample,
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
  fields: ColumnFieldSample,
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
  fields: ColumnFieldSample,
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

function selectLandmarkRoster(
  biomeId: BiomeId,
  undergroundBiomeId: UndergroundBiomeId,
  regionalVariantId: RegionalVariantId | null,
  fields: ColumnFieldSample,
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
  fields: ColumnFieldSample,
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

function selectRegionalVariant(biomeId: BiomeId, fields: ColumnFieldSample): RegionalVariantSelection | null {
  let strength = 0;
  let id: RegionalVariantId | null = null;
  switch (biomeId) {
    case "verdant":
      strength = averageSignal(
        smoothstep(0.68, 0.84, fields.oldGrowth),
        smoothstep(0.58, 0.78, fields.drainage),
        smoothstep(0.48, 0.74, fields.globalHeight),
        smoothstep(0.52, 0.80, fields.channel),
      );
      id = strength > 0.56 ? "verdant_karst" : null;
      break;
    case "savanna":
      strength = averageSignal(
        smoothstep(0.62, 0.84, fields.temperature),
        smoothstep(0.58, 0.84, fields.scatter),
        smoothstep(0.48, 0.72, fields.moisture),
        smoothstep(0.48, 0.76, fields.grove + fields.orchard * 0.2),
      );
      id = strength > 0.60 ? "savanna_flowersea" : null;
      break;
    case "steppe":
      strength = averageSignal(
        smoothstep(0.58, 0.80, fields.uplift),
        smoothstep(0.56, 0.82, fields.ridge),
        smoothstep(0.50, 0.76, fields.peakness),
        smoothstep(0.48, 0.76, fields.temperature),
      );
      id = strength > 0.82 ? "steppe_monolith" : null;
      break;
    case "dunes":
      strength = averageSignal(
        smoothstep(0.76, 0.90, fields.temperature),
        smoothstep(0.64, 0.88, fields.dune),
        smoothstep(0.48, 0.78, fields.volcanism + fields.magic * 0.2),
        smoothstep(0.56, 0.84, 1 - fields.moisture),
      );
      id = strength > 0.64 ? "dunes_glass" : null;
      break;
    case "badlands":
      strength = averageSignal(
        smoothstep(0.54, 0.80, fields.uplift),
        smoothstep(0.54, 0.82, fields.mesa),
        smoothstep(0.48, 0.80, fields.volcanism),
        smoothstep(0.48, 0.78, fields.peakness),
      );
      id = strength > 0.62 ? "badlands_crater" : null;
      break;
    case "highland":
      strength = averageSignal(
        smoothstep(0.64, 0.84, fields.oldGrowth),
        scoreField(fields.temperature, 0.54, 0.18),
        smoothstep(0.46, 0.76, fields.moisture),
        smoothstep(0.54, 0.80, fields.uplift),
      );
      id = strength > 0.50 ? "highland_redleaf" : null;
      break;
    case "moor":
      strength = averageSignal(
        smoothstep(0.58, 0.82, fields.magic),
        smoothstep(0.54, 0.80, fields.desolation + fields.scatter * 0.15),
        smoothstep(0.58, 0.84, fields.moisture),
      );
      id = strength > 0.50 ? "moor_shadowglass" : null;
      break;
    case "tundra":
      strength = averageSignal(
        smoothstep(0.70, 0.88, fields.uplift),
        smoothstep(0.56, 0.82, fields.peakness + fields.ridge * 0.2),
        smoothstep(0.52, 0.84, 1 - fields.temperature),
      );
      id = strength > 0.78 ? "tundra_blue_ice" : null;
      break;
    case "marsh":
      strength = averageSignal(
        smoothstep(0.70, 0.88, fields.moisture),
        smoothstep(0.66, 0.88, fields.channel),
        smoothstep(0.54, 0.80, fields.grove),
      );
      id = strength > 0.46 ? "marsh_blackwater" : null;
      break;
    case "firefly":
      strength = averageSignal(
        smoothstep(0.66, 0.86, fields.magic),
        smoothstep(0.70, 0.90, fields.moisture),
        smoothstep(0.58, 0.84, fields.channel + fields.grove * 0.2),
      );
      id = strength > 0.46 ? "firefly_lantern" : null;
      break;
    case "saltflat":
      strength = averageSignal(
        smoothstep(0.60, 0.84, fields.oceanness + Math.max(0, -fields.basin) * 0.25),
        smoothstep(0.56, 0.82, fields.surfacePatch),
        smoothstep(0.58, 0.84, 1 - fields.moisture),
      );
      id = strength > 0.52 ? "saltflat_mirror" : null;
      break;
    case "fern":
      strength = averageSignal(
        smoothstep(0.64, 0.86, fields.temperature),
        smoothstep(0.68, 0.88, fields.moisture),
        smoothstep(0.58, 0.86, fields.channel + Math.max(0, -fields.basin) * 0.30),
      );
      id = strength > 0.52 ? "fern_cenote" : null;
      break;
    case "fungal":
      strength = averageSignal(
        smoothstep(0.68, 0.88, fields.magic),
        smoothstep(0.68, 0.88, fields.moisture),
        smoothstep(0.56, 0.82, fields.oldGrowth + fields.grove * 0.2),
      );
      id = strength > 0.46 ? "fungal_moonlit" : null;
      break;
    case "ember":
      strength = averageSignal(
        smoothstep(0.66, 0.86, fields.volcanism),
        smoothstep(0.58, 0.82, fields.peakness + fields.mesa * 0.3),
        smoothstep(0.50, 0.80, fields.ridge),
      );
      id = strength > 0.52 ? "ember_caldera" : null;
      break;
    case "bloom":
      strength = averageSignal(
        smoothstep(0.68, 0.86, fields.magic),
        smoothstep(0.50, 0.78, fields.orchard + fields.grove * 0.2),
        smoothstep(0.44, 0.74, fields.moisture),
      );
      id = strength > 0.64 ? "bloom_prism" : null;
      break;
  }
  return id === null ? null : { id, strength };
}

function adjustSpecialBiomeSurfaceY(
  seaLevel: number,
  biomeId: BiomeId,
  specialStrength: number,
  fields: ColumnFieldSample,
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
  fields: ColumnFieldSample,
  biomeCore: number,
): number {
  const weight = 0.28 + biomeCore * 0.72;
  switch (regionalVariantId) {
    case "verdant_karst":
      return -Math.round(lerp(6, 18, strength) * (0.6 + fields.channel * 0.4) * weight);
    case "savanna_flowersea":
      return Math.round((fields.hills - 0.2) * (8 + strength * 10) * weight);
    case "steppe_monolith":
      return Math.round(lerp(4, 14, strength) * (0.5 + fields.ridge * 0.5) * weight);
    case "dunes_glass":
      return Math.round((fields.dune - 0.52) * (12 + strength * 18) * weight);
    case "badlands_crater":
      return Math.round((fields.mesa - 0.52) * (16 + strength * 20) * weight);
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

function configureLandmarkFeature(
  profile: LandmarkProfile,
  surfaceY: number,
  waterTopY: number,
  fields: ColumnFieldSample,
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
        scaledFeatureRadius(8, 3, fields.moisture, profile.scale),
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
      configureSpireFeature(
        out,
        FEATURE_STANDING_STONE,
        scaledFeatureHeight(20, 20, fields.uplift + fields.surfacePatch * 0.5, profile.scale),
        scaledFeatureRadius(2, 1, fields.scatter, profile.scale),
        profile.variant > 0 ? "#433" : "#654",
        profile.variant > 0 ? "#654" : "#876",
      );
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
): void {
  out.featureKind = featureKind;
  out.featureHeight = height;
  out.featureRadius = radius;
  out.featureExtra = 1;
  out.featureMaterialPrimary = hexColorToMaterial(materialPrimary);
  out.featureMaterialSecondary = hexColorToMaterial(materialSecondary);
  out.featureMaterialAccent = 0;
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
    return resolveTransitionMaterial(
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
    1,
    -0.24,
  ) || sampleCaveLayer(
    surfaceY,
    worldY,
    caveUpperField,
    caveUpperStrength,
    caveUpperCenterY,
    caveUpperHalfHeight,
    1,
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
      const bodyRadius = Math.max(1.1, featureRadius - Math.abs(relativeY - featureHeight * 0.45) * 0.55);
      const topCapRadius = Math.min(bodyRadius, Math.max(0.9, featureRadius * 0.58));
      if (relativeY === featureHeight && radial <= topCapRadius) {
        return materialSecondary;
      }
      return radial <= bodyRadius ? materialPrimary : 0;
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
      if (relativeY <= 1 + featureExtra && radial <= Math.max(1, featureRadius - relativeY * 0.15)) {
        return materialSecondary;
      }
      return radial <= Math.max(1, featureRadius - relativeY * 0.28) ? materialPrimary : 0;
    case FEATURE_CRYSTAL:
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

function scoreField(value: number, target: number, spread: number): number {
  return saturate(1 - Math.abs(value - target) / spread);
}

function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t;
}

function averageSignal(...values: number[]): number {
  if (values.length === 0) {
    return 0;
  }
  let total = 0;
  for (const value of values) {
    total += value;
  }
  return total / values.length;
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
