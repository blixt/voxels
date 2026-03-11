import { clamp, packRgba } from "./math.ts";
import { fbm2D2, fbm2D3, fbm2D4, fbm2D5, hashNoise3D } from "./noise.ts";
import type { ChunkBounds, ChunkCoordinate } from "./types.ts";

export const HEX_COLOR_COUNT = 0x1000;
export const PROCEDURAL_WORLD_MAX_Y = 16_384;

export type BaseBiomeId = "verdant" | "steppe" | "dunes" | "badlands" | "highland" | "tundra";
export type SpecialBiomeId = "marsh" | "ember" | "bloom";
export type BiomeId = BaseBiomeId | SpecialBiomeId;
export type UndergroundBiomeId = "rooted" | "sedimentary" | "sandy" | "granitic" | "froststone" | "basaltic";
export type LandmarkId =
  | "oak"
  | "canopy_tree"
  | "birch"
  | "blossom_tree"
  | "fruit_tree"
  | "redwood"
  | "dead_tree"
  | "berry_bush"
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
  | "mega_glowcap";

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
  orchard: number;
  desolation: number;
  strata: number;
  surfacePatch: number;
  surfaceGrain: number;
  scatter: number;
}

interface MutableColumnState {
  biomeId: BiomeId;
  hostBiomeId: BaseBiomeId;
  secondaryBiomeId: BaseBiomeId;
  undergroundBiomeId: UndergroundBiomeId;
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
  landmarkId: LandmarkId | null;
  surfaceY: number;
  topY: number;
  waterTopY: number | null;
  surfaceMaterial: number;
}

export interface ProceduralBiomeProbe extends ProceduralColumnSample {
  secondaryBiomeId: BaseBiomeId;
  transitionThreshold: number;
  specialStrength: number;
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
const CHANNEL_SCALE = 1 / 1200;
const DUNE_SCALE = 1 / 320;
const MESA_SCALE = 1 / 900;
const GROVE_SCALE = 1 / 2100;
const ORCHARD_SCALE = 1 / 1700;
const DESOLATION_SCALE = 1 / 2400;
const STRATA_SCALE = 1 / 54;
const SURFACE_PATCH_SCALE = 1 / 48;
const SURFACE_GRAIN_SCALE = 1 / 14;
const SURFACE_SCATTER_SCALE = 1 / 26;
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
  createBaseBiome("steppe", 0.62, 0.42, 0.36, 0.52, 0.48, 0.14, 0, 0.54, 0.22, 0.32, 0.18, 0.00, 4.8, 1608, "#9B6", "#CB7", "#BA6", "#CA7", "#887", "#875", "#986", "#4AA", "#DDD"),
  createBaseBiome("dunes", 0.84, 0.16, 0.18, 0.28, 0.30, 0.12, -16, 0.32, 0.10, 0.54, 0.42, 0.00, 8.8, 1710, "#DB6", "#EC9", "#EC7", "#CA5", "#B96", "#B85", "#C96", "#5BC", "#EDC"),
  createBaseBiome("badlands", 0.72, 0.20, 0.58, 0.36, 0.58, 0.16, 18, 0.72, 0.64, 0.38, 0.06, 0.46, 9.6, 1670, "#C75", "#D96", "#D86", "#B54", "#865", "#A54", "#965", "#49B", "#EBC"),
  createBaseBiome("highland", 0.40, 0.56, 0.72, 0.46, 0.72, 0.16, 44, 0.88, 0.62, 0.24, 0.10, 0.06, 7.8, 1518, "#6B7", "#7C8", "#7A8", "#8C7", "#778", "#667", "#889", "#5AD", "#EEF"),
  createBaseBiome("tundra", 0.18, 0.42, 0.86, 0.40, 0.82, 0.12, 78, 0.98, 0.82, 0.16, 0.02, 0.04, 6.2, 1452, "#BCC", "#CDD", "#DDE", "#ABB", "#889", "#99A", "#AAB", "#8CD", "#EEF"),
] as const;

const SPECIAL_BIOMES: Record<SpecialBiomeId, SpecialBiomeProfile> = {
  marsh: createSpecialBiome("marsh", "#486", "#5A8", "#597", "#2A6", "#576", "#564", "#675", "#276", "#DDE", true),
  ember: createSpecialBiome("ember", "#543", "#754", "#764", "#F74", "#433", "#654", "#765", "#36A", "#DCC", false),
  bloom: createSpecialBiome("bloom", "#6A8", "#8CF", "#7BA", "#BDF", "#668", "#557", "#668", "#4CF", "#EEF", true),
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
};

const LANDMARKS: Record<LandmarkId, LandmarkProfile> = {
  oak: createLandmark("oak", 176, 11, 0.34, 1.0, 0),
  canopy_tree: createLandmark("canopy_tree", 224, 16, 0.20, 1.2, 2),
  birch: createLandmark("birch", 164, 9, 0.30, 1.0, 1),
  blossom_tree: createLandmark("blossom_tree", 168, 11, 0.24, 1.0, 7),
  fruit_tree: createLandmark("fruit_tree", 160, 10, 0.20, 1.0, 8),
  redwood: createLandmark("redwood", 272, 20, 0.18, 1.0, 0),
  dead_tree: createLandmark("dead_tree", 172, 8, 0.20, 1.0, 0),
  berry_bush: createLandmark("berry_bush", 88, 5, 0.42, 1.0, 5),
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
  ember: [
    landmarkPlacement("basalt_spire", { chance: 0.30, scale: 1.28 }),
    landmarkPlacement("crystal_cluster", { chance: 0.28, scale: 1.12, variant: 3 }),
    landmarkPlacement("dead_snag", { chance: 0.26, scale: 1.16, variant: 1 }),
    landmarkPlacement("boulder", { chance: 0.24, scale: 0.94, variant: 1 }),
  ],
  bloom: [
    landmarkPlacement("mega_glowcap", { chance: 0.24, scale: 1.18 }),
    landmarkPlacement("glowcap", { chance: 0.32, scale: 1.08 }),
    landmarkPlacement("crystal_cluster", { chance: 0.20, scale: 1.08, variant: 2 }),
    landmarkPlacement("flower_patch", { chance: 0.28, scale: 1.08, variant: 4 }),
    landmarkPlacement("canopy_tree", { chance: 0.16, scale: 1.12, variant: 4 }),
  ],
};

const VERDANT_GROVE_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("redwood", { chance: 0.34, scale: 1.08, cellSize: 196, radius: 20 }),
  landmarkPlacement("canopy_tree", { chance: 0.82, scale: 1.34, cellSize: 92, radius: 18 }),
  landmarkPlacement("oak", { chance: 0.74, scale: 1.20, cellSize: 84, radius: 12 }),
  landmarkPlacement("berry_bush", { chance: 0.64, scale: 1.10, cellSize: 68, radius: 5 }),
  landmarkPlacement("flower_patch", { chance: 0.44, scale: 1.12, variant: 1, cellSize: 72, radius: 5 }),
];

const VERDANT_ORCHARD_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("blossom_tree", { chance: 0.48, scale: 1.12, cellSize: 108, radius: 12 }),
  landmarkPlacement("fruit_tree", { chance: 0.44, scale: 1.10, cellSize: 104, radius: 11 }),
  landmarkPlacement("berry_bush", { chance: 0.64, scale: 1.12, cellSize: 72, radius: 5 }),
  landmarkPlacement("flower_patch", { chance: 0.56, scale: 1.18, variant: 4, cellSize: 68, radius: 5 }),
  landmarkPlacement("shrub", { chance: 0.34, scale: 1.06, cellSize: 84, radius: 4 }),
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

const BADLANDS_DESOLATE_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("dead_tree", { chance: 0.34, scale: 1.28, cellSize: 148, radius: 8 }),
  landmarkPlacement("hoodoo", { chance: 0.28, scale: 1.18 }),
  landmarkPlacement("standing_stone", { chance: 0.22, scale: 1.20 }),
  landmarkPlacement("boulder", { chance: 0.20, scale: 0.98, variant: 1 }),
];

const HIGHLAND_REDWOOD_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("redwood", { chance: 0.38, scale: 1.16, cellSize: 184, radius: 22 }),
  landmarkPlacement("tall_fir", { chance: 0.54, scale: 1.20, cellSize: 108, radius: 12 }),
  landmarkPlacement("fir", { chance: 0.52, scale: 1.12, cellSize: 96, radius: 10 }),
  landmarkPlacement("berry_bush", { chance: 0.28, scale: 1.00, cellSize: 84, radius: 5 }),
  landmarkPlacement("boulder", { chance: 0.18, scale: 1.02 }),
];

const TUNDRA_TAIGA_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("tall_fir", { chance: 0.42, scale: 1.16, cellSize: 124, radius: 12 }),
  landmarkPlacement("fir", { chance: 0.56, scale: 1.10, cellSize: 96, radius: 10 }),
  landmarkPlacement("frost_shrub", { chance: 0.44, scale: 1.06, cellSize: 84, radius: 4 }),
  landmarkPlacement("boulder", { chance: 0.18, scale: 1.00 }),
];

const MARSH_THICKET_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("mangrove", { chance: 0.36, scale: 1.18, cellSize: 160, radius: 15 }),
  landmarkPlacement("cypress", { chance: 0.42, scale: 1.12, cellSize: 128, radius: 10 }),
  landmarkPlacement("reed_cluster", { chance: 0.70, scale: 1.12, cellSize: 68, radius: 4 }),
  landmarkPlacement("flower_patch", { chance: 0.18, scale: 0.96, variant: 3, cellSize: 84, radius: 5 }),
];

const EMBER_DEADLAND_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("dead_tree", { chance: 0.28, scale: 1.22, cellSize: 156, radius: 8 }),
  landmarkPlacement("basalt_spire", { chance: 0.28, scale: 1.30 }),
  landmarkPlacement("crystal_cluster", { chance: 0.26, scale: 1.12, variant: 3 }),
  landmarkPlacement("boulder", { chance: 0.18, scale: 0.96, variant: 1 }),
];

const BLOOM_ORCHARD_LANDMARKS: readonly LandmarkProfile[] = [
  landmarkPlacement("mega_glowcap", { chance: 0.18, scale: 1.22 }),
  landmarkPlacement("blossom_tree", { chance: 0.28, scale: 1.18, cellSize: 128, radius: 12 }),
  landmarkPlacement("fruit_tree", { chance: 0.22, scale: 1.10, cellSize: 128, radius: 11 }),
  landmarkPlacement("glowcap", { chance: 0.30, scale: 1.06 }),
  landmarkPlacement("flower_patch", { chance: 0.34, scale: 1.12, variant: 4, cellSize: 72, radius: 5 }),
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
  private readonly channelSeed: number;
  private readonly duneSeed: number;
  private readonly mesaSeed: number;
  private readonly groveSeed: number;
  private readonly orchardSeed: number;
  private readonly desolationSeed: number;
  private readonly strataSeed: number;
  private readonly surfacePatchSeed: number;
  private readonly surfaceGrainSeed: number;
  private readonly surfaceScatterSeed: number;
  private readonly transitionSeed: number;
  private readonly featureSeed: number;

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
    this.channelSeed = seed + 653;
    this.duneSeed = seed + 709;
    this.mesaSeed = seed + 761;
    this.groveSeed = seed + 787;
    this.orchardSeed = seed + 829;
    this.desolationSeed = seed + 881;
    this.strataSeed = seed + 809;
    this.surfacePatchSeed = seed + 863;
    this.surfaceGrainSeed = seed + 911;
    this.surfaceScatterSeed = seed + 967;
    this.transitionSeed = seed + 1013;
    this.featureSeed = seed + 1061;
  }

  sampleColumn(worldX: number, worldZ: number): ProceduralColumnSample {
    const state = createMutableColumnState();
    this.fillColumnState(worldX, worldZ, state);
    return columnSampleFromState(state);
  }

  sampleBiomeProbe(worldX: number, worldZ: number): ProceduralBiomeProbe {
    const state = createMutableColumnState();
    this.fillColumnState(worldX, worldZ, state);
    return {
      ...columnSampleFromState(state),
      secondaryBiomeId: state.secondaryBiomeId,
      transitionThreshold: state.transitionThreshold,
      specialStrength: state.specialStrength,
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

  sampleMaterial(worldX: number, worldY: number, worldZ: number): number {
    if (worldY < 0 || worldY >= this.maxYExclusive) {
      return 0;
    }
    const state = createMutableColumnState();
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
    };
  }

  private fillColumnState(worldX: number, worldZ: number, out: MutableColumnState): void {
    const fields = this.sampleFields(worldX, worldZ);
    const baseBlend = this.selectBaseBiomes(fields);
    const terrainProfile = blendTerrainProfile(baseBlend.primary, baseBlend.secondary, baseBlend.primaryWeight);
    const biomeCore = smoothstep(0.60, 0.88, baseBlend.primaryWeight);
    let surfaceY = this.sampleSurfaceY(fields, terrainProfile, biomeCore);

    const flatness = saturate(1 - (
      fields.ridge * 0.7
      + Math.abs(fields.detail) * 1.4
      + Math.abs(fields.hills) * 0.55
      + fields.uplift * 0.25
    ));
    const verdantSteppeHost = hostBlendStrength(baseBlend.primary.id, baseBlend.secondary.id, baseBlend.primaryWeight, "verdant", "steppe");
    const badlandsHighlandHost = hostBlendStrength(baseBlend.primary.id, baseBlend.secondary.id, baseBlend.primaryWeight, "badlands", "highland");
    const verdantHighlandHost = hostBlendStrength(baseBlend.primary.id, baseBlend.secondary.id, baseBlend.primaryWeight, "verdant", "highland");
    const marshStrength = saturate(
      verdantSteppeHost
      * smoothstep(0.50, 0.74, fields.moisture)
      * smoothstep(0.44, 0.68, fields.drainage)
      * smoothstep(0.32, 0.72, fields.channel)
      * smoothstep(0.18, 0.82, flatness),
    );
    const emberStrength = saturate(
      badlandsHighlandHost
      * smoothstep(0.62, 0.84, fields.volcanism)
      * smoothstep(0.16, 0.58, 1 - fields.moisture),
    );
    const bloomStrength = saturate(
      verdantHighlandHost
      * smoothstep(0.54, 0.74, fields.magic)
      * smoothstep(0.38, 0.62, fields.moisture)
      * (0.55 + smoothstep(0.14, 0.66, 1 - fields.volcanism) * 0.45),
    );

    let biomeId: BiomeId = baseBlend.primary.id;
    let specialStrength = 0;
    if (marshStrength > 0.40 && marshStrength >= emberStrength && marshStrength >= bloomStrength) {
      biomeId = "marsh";
      specialStrength = marshStrength;
      surfaceY -= Math.round(lerp(1, 7, marshStrength) * (0.35 + biomeCore * 0.65));
    } else if (bloomStrength > 0.34 && bloomStrength >= emberStrength) {
      biomeId = "bloom";
      specialStrength = bloomStrength;
      surfaceY += Math.round(lerp(0, 4, bloomStrength) * (fields.magic - 0.5) * (0.3 + biomeCore * 0.7));
    } else if (emberStrength > 0.54) {
      biomeId = "ember";
      specialStrength = emberStrength;
      surfaceY += Math.round(lerp(2, 11, emberStrength) * (0.25 + fields.mesa) * (0.35 + biomeCore * 0.65));
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
    const waterTopY = this.resolveWaterTopY(biomeId, surfaceY, fields, specialStrength);
    const submergedSurface = hasStandingWater(surfaceY, waterTopY);
    if (submergedSurface) {
      surfaceMaterials.surfacePrimary = surfaceMaterials.subsurfacePrimary;
      surfaceMaterials.surfaceSecondary = surfaceMaterials.subsurfaceSecondary;
    }
    const landmarkId = this.resolveLandmark(worldX, worldZ, biomeId, surfaceY, waterTopY, fields, out);

    out.biomeId = biomeId;
    out.hostBiomeId = hostBiomeId;
    out.secondaryBiomeId = baseBlend.secondary.id;
    out.undergroundBiomeId = undergroundBiomeId;
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
    out.strataOffset = fields.strata * 5;
    out.worldXDiv3 = Math.floor(worldX * ONE_THIRD);
    out.worldZDiv3 = Math.floor(worldZ * ONE_THIRD);
    out.ditherSeed = this.transitionSeed + baseBlend.primary.surface + baseBlend.secondary.surface;
    out.accentSeed = this.seed + underground.accent;
  }

  private sampleFields(worldX: number, worldZ: number): ColumnFieldSample {
    const continentalness = fbm2D5(worldX * CONTINENT_SCALE, worldZ * CONTINENT_SCALE, this.continentSeed) - 0.5;
    const uplift = fbm2D4(worldX * UPLIFT_SCALE, worldZ * UPLIFT_SCALE, this.upliftSeed);
    const hills = fbm2D4(worldX * HILLS_SCALE, worldZ * HILLS_SCALE, this.hillsSeed) - 0.5;
    const detail = fbm2D4(worldX * DETAIL_SCALE, worldZ * DETAIL_SCALE, this.detailSeed) - 0.5;
    const ridge = 1 - Math.abs(fbm2D3(worldX * RIDGE_SCALE, worldZ * RIDGE_SCALE, this.ridgeSeed) * 2 - 1);
    const basin = fbm2D3(worldX * BASIN_SCALE, worldZ * BASIN_SCALE, this.basinSeed) - 0.5;
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
      orchard: fbm2D3(worldX * ORCHARD_SCALE, worldZ * ORCHARD_SCALE, this.orchardSeed),
      desolation: fbm2D3(worldX * DESOLATION_SCALE, worldZ * DESOLATION_SCALE, this.desolationSeed),
      strata: fbm2D2(worldX * STRATA_SCALE, worldZ * STRATA_SCALE, this.strataSeed),
      surfacePatch: fbm2D3(worldX * SURFACE_PATCH_SCALE, worldZ * SURFACE_PATCH_SCALE, this.surfacePatchSeed),
      surfaceGrain: fbm2D2(worldX * SURFACE_GRAIN_SCALE, worldZ * SURFACE_GRAIN_SCALE, this.surfaceGrainSeed),
      scatter: fbm2D2(worldX * SURFACE_SCATTER_SCALE, worldZ * SURFACE_SCATTER_SCALE, this.surfaceScatterSeed),
    };
  }

  private selectBaseBiomes(fields: ColumnFieldSample): {
    primary: BaseBiomeProfile;
    secondary: BaseBiomeProfile;
    primaryWeight: number;
  } {
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
    const sharedRelief = fields.hills * (28 + fields.globalHeight * 36)
      + (fields.ridge * fields.ridge - 0.30) * (18 + fields.mountainness * 68)
      + fields.basin * 26
      + fields.detail * 8
      + massifRelief;
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
  ): number {
    let waterTopY = surfaceY < this.seaLevel ? this.seaLevel : NO_WATER;
    if (biomeId === "marsh") {
      const extraWaterDepth = Math.round(lerp(1, 3, specialStrength));
      waterTopY = Math.max(waterTopY, surfaceY + extraWaterDepth);
    } else if (biomeId === "dunes" && fields.channel > 0.78 && fields.basin < -0.08) {
      waterTopY = Math.max(waterTopY, surfaceY + 1);
    } else if (biomeId === "bloom" && fields.magic > 0.82 && fields.moisture > 0.62) {
      waterTopY = Math.max(waterTopY, surfaceY + 1);
    }
    return waterTopY;
  }

  private selectUndergroundBiome(
    biomeId: BiomeId,
    hostBiomeId: BaseBiomeId,
    fields: ColumnFieldSample,
  ): UndergroundBiomeId {
    if (biomeId === "ember" || fields.volcanism > 0.78) {
      return "basaltic";
    }
    if (biomeId === "tundra" || (hostBiomeId === "highland" && fields.temperature < 0.3)) {
      return "froststone";
    }
    if (biomeId === "marsh" || biomeId === "bloom" || (hostBiomeId === "verdant" && fields.moisture > 0.6)) {
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
    const roster = selectLandmarkRoster(biomeId, fields);
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

function columnSampleFromState(state: MutableColumnState): ProceduralColumnSample {
  return {
    biomeId: state.biomeId,
    hostBiomeId: state.hostBiomeId,
    undergroundBiomeId: state.undergroundBiomeId,
    landmarkId: state.landmarkId,
    surfaceY: state.surfaceY,
    topY: Math.max(state.surfaceY, state.surfaceY + state.featureHeight + (state.featureKind === FEATURE_NONE ? 0 : 1)),
    waterTopY: state.waterTopY === NO_WATER ? null : state.waterTopY,
    surfaceMaterial: state.surfaceMaterialPrimary,
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

function hostBlendStrength(
  primary: BaseBiomeId,
  secondary: BaseBiomeId,
  primaryWeight: number,
  a: BaseBiomeId,
  b: BaseBiomeId,
): number {
  let strength = 0;
  if (primary === a || primary === b) {
    strength += primaryWeight;
  }
  if (secondary === a || secondary === b) {
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
    return primary === "verdant" || primary === "steppe" ? primary : secondary;
  }
  if (biomeId === "ember") {
    return primary === "badlands" || primary === "highland" ? primary : secondary;
  }
  if (biomeId === "bloom") {
    return primary === "verdant" || primary === "highland" ? primary : secondary;
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

function selectLandmarkRoster(biomeId: BiomeId, fields: ColumnFieldSample): readonly LandmarkProfile[] {
  switch (biomeId) {
    case "verdant":
      if (fields.grove > 0.64 && fields.moisture > 0.62 && fields.drainage > 0.56) {
        return VERDANT_GROVE_LANDMARKS;
      }
      if (fields.orchard > 0.68 && fields.temperature > 0.50 && fields.moisture > 0.56) {
        return VERDANT_ORCHARD_LANDMARKS;
      }
      return BASE_BIOME_LANDMARKS.verdant;
    case "steppe":
      if (fields.orchard > 0.70 && fields.moisture > 0.42 && fields.temperature > 0.56) {
        return STEPPE_ORCHARD_LANDMARKS;
      }
      if (fields.desolation > 0.72 && fields.moisture < 0.42) {
        return STEPPE_DESOLATE_LANDMARKS;
      }
      return BASE_BIOME_LANDMARKS.steppe;
    case "badlands":
      return fields.desolation > 0.58 ? BADLANDS_DESOLATE_LANDMARKS : BASE_BIOME_LANDMARKS.badlands;
    case "highland":
      return fields.grove > 0.68 && fields.moisture > 0.50 && fields.uplift > 0.62
        ? HIGHLAND_REDWOOD_LANDMARKS
        : BASE_BIOME_LANDMARKS.highland;
    case "tundra":
      return fields.grove > 0.62 && fields.moisture > 0.36
        ? TUNDRA_TAIGA_LANDMARKS
        : BASE_BIOME_LANDMARKS.tundra;
    case "marsh":
      return fields.grove > 0.60 ? MARSH_THICKET_LANDMARKS : SPECIAL_BIOME_LANDMARKS.marsh;
    case "ember":
      return fields.desolation > 0.54 ? EMBER_DEADLAND_LANDMARKS : SPECIAL_BIOME_LANDMARKS.ember;
    case "bloom":
      return fields.orchard > 0.64 ? BLOOM_ORCHARD_LANDMARKS : SPECIAL_BIOME_LANDMARKS.bloom;
    default:
      return BASE_BIOME_LANDMARKS[biomeId];
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
      const trunkHeight = Math.max(3, Math.round(featureHeight * 0.18));
      const trunkRadius = Math.min(1.1, 0.72 + featureRadius * 0.04);
      if (relativeY <= trunkHeight) {
        return absX <= trunkRadius && absZ <= trunkRadius ? materialPrimary : 0;
      }
      const crownCenter = featureHeight * 0.62;
      const crownRadius = featureRadius - Math.abs(relativeY - crownCenter) * 0.18;
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
      const branchStart = Math.max(4, Math.round(featureHeight * 0.36));
      if (relativeY < branchStart) {
        return 0;
      }
      const branchSeed = (relativeY + featureExtra * 3) % 6;
      const branchExtent = Math.max(2, Math.round(featureRadius * (relativeY > featureHeight * 0.72 ? 0.55 : 0.40)));
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
      if (relativeY <= featureHeight - (featureExtra >= 3 ? 5 : 3)) {
        const stemRadius = Math.min(1.2, 0.75 + featureRadius * 0.04);
        return absX <= stemRadius && absZ <= stemRadius ? materialPrimary : 0;
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
