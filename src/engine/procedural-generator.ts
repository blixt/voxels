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
  | "birch"
  | "boulder"
  | "standing_stone"
  | "shrub"
  | "palm"
  | "cactus"
  | "hoodoo"
  | "fir"
  | "ice_spire"
  | "cypress"
  | "reed_cluster"
  | "basalt_spire"
  | "crystal_cluster"
  | "glowcap";

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
const CHUNK_GENERATION_SCRATCH_POOL_LIMIT = 4;

const BASE_BIOMES: readonly BaseBiomeProfile[] = [
  createBaseBiome("verdant", 0.56, 0.78, 0.28, 0.74, 0.42, 0.18, -10, 0.48, 0.18, 0.40, 0.28, 0.00, 2.0, 1548, "#6A5", "#7B6", "#8B6", "#592", "#677", "#754", "#865", "#49B", "#DDE"),
  createBaseBiome("steppe", 0.62, 0.42, 0.36, 0.52, 0.48, 0.14, 0, 0.54, 0.22, 0.32, 0.18, 0.00, 1.8, 1608, "#9B6", "#CB7", "#BA6", "#CA7", "#887", "#875", "#986", "#4AA", "#DDD"),
  createBaseBiome("dunes", 0.84, 0.16, 0.18, 0.28, 0.30, 0.12, -16, 0.32, 0.10, 0.54, 0.42, 0.00, 2.8, 1710, "#DB6", "#EC9", "#EC7", "#CA5", "#B96", "#B85", "#C96", "#5BC", "#EDC"),
  createBaseBiome("badlands", 0.72, 0.20, 0.58, 0.36, 0.58, 0.16, 18, 0.72, 0.64, 0.38, 0.06, 0.46, 2.4, 1670, "#C75", "#D96", "#D86", "#B54", "#865", "#A54", "#965", "#49B", "#EBC"),
  createBaseBiome("highland", 0.40, 0.56, 0.72, 0.46, 0.72, 0.16, 44, 0.88, 0.62, 0.24, 0.10, 0.06, 2.6, 1518, "#6B7", "#7C8", "#7A8", "#8C7", "#778", "#667", "#889", "#5AD", "#EEF"),
  createBaseBiome("tundra", 0.18, 0.42, 0.86, 0.40, 0.82, 0.12, 78, 0.98, 0.82, 0.16, 0.02, 0.04, 2.2, 1452, "#BCC", "#CDD", "#DDE", "#ABB", "#889", "#99A", "#AAB", "#8CD", "#EEF"),
] as const;

const SPECIAL_BIOMES: Record<SpecialBiomeId, SpecialBiomeProfile> = {
  marsh: createSpecialBiome("marsh", "#486", "#5A8", "#597", "#2A6", "#576", "#564", "#675", "#276", "#DDE", true),
  ember: createSpecialBiome("ember", "#543", "#754", "#764", "#F74", "#433", "#654", "#765", "#36A", "#DCC", false),
  bloom: createSpecialBiome("bloom", "#6A8", "#8CF", "#7BA", "#BDF", "#668", "#557", "#668", "#4CF", "#EEF", true),
};

const UNDERGROUND_BIOMES: Record<UndergroundBiomeId, UndergroundBiomeProfile> = {
  rooted: createUndergroundBiome("rooted", "#586", "#354", "#9C6"),
  sedimentary: createUndergroundBiome("sedimentary", "#866", "#644", "#DA7"),
  sandy: createUndergroundBiome("sandy", "#977", "#655", "#EDC"),
  granitic: createUndergroundBiome("granitic", "#889", "#556", "#BDE"),
  froststone: createUndergroundBiome("froststone", "#9AB", "#667", "#DFF"),
  basaltic: createUndergroundBiome("basaltic", "#544", "#322", "#F74"),
};

const LANDMARKS: Record<LandmarkId, LandmarkProfile> = {
  oak: { id: "oak", cellSize: 88, radius: 5, chance: 0.32 },
  birch: { id: "birch", cellSize: 84, radius: 4, chance: 0.30 },
  boulder: { id: "boulder", cellSize: 72, radius: 4, chance: 0.34 },
  standing_stone: { id: "standing_stone", cellSize: 104, radius: 3, chance: 0.24 },
  shrub: { id: "shrub", cellSize: 64, radius: 3, chance: 0.42 },
  palm: { id: "palm", cellSize: 96, radius: 6, chance: 0.3 },
  cactus: { id: "cactus", cellSize: 84, radius: 3, chance: 0.34 },
  hoodoo: { id: "hoodoo", cellSize: 104, radius: 5, chance: 0.26 },
  fir: { id: "fir", cellSize: 80, radius: 4, chance: 0.34 },
  ice_spire: { id: "ice_spire", cellSize: 104, radius: 4, chance: 0.24 },
  cypress: { id: "cypress", cellSize: 88, radius: 5, chance: 0.42 },
  reed_cluster: { id: "reed_cluster", cellSize: 68, radius: 3, chance: 0.48 },
  basalt_spire: { id: "basalt_spire", cellSize: 104, radius: 4, chance: 0.22 },
  crystal_cluster: { id: "crystal_cluster", cellSize: 76, radius: 3, chance: 0.32 },
  glowcap: { id: "glowcap", cellSize: 80, radius: 6, chance: 0.4 },
};

const BASE_BIOME_LANDMARKS: Record<BaseBiomeId, readonly LandmarkId[]> = {
  verdant: ["oak", "birch", "shrub", "boulder"],
  steppe: ["standing_stone", "shrub", "boulder", "birch"],
  dunes: ["palm", "cactus", "boulder"],
  badlands: ["hoodoo", "standing_stone", "boulder", "cactus"],
  highland: ["fir", "boulder", "birch", "standing_stone"],
  tundra: ["ice_spire", "fir", "boulder", "shrub"],
};

const SPECIAL_BIOME_LANDMARKS: Record<SpecialBiomeId, readonly LandmarkId[]> = {
  marsh: ["cypress", "reed_cluster", "shrub"],
  ember: ["basalt_spire", "crystal_cluster", "boulder"],
  bloom: ["glowcap", "birch", "crystal_cluster", "shrub"],
};

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
  readonly palette = buildHexColorPalette();
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
    const landmarkId = this.resolveLandmark(worldX, worldZ, biomeId, waterTopY, fields, out);

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
    const sharedRelief = fields.hills * (28 + fields.globalHeight * 36)
      + (fields.ridge * fields.ridge - 0.30) * (18 + fields.mountainness * 68)
      + fields.basin * 26
      + fields.detail * 8;
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
    const roster = selectLandmarkRoster(biomeId);
    if (roster.length === 0) {
      return null;
    }

    const selectorCellX = Math.floor(worldX / 64);
    const selectorCellZ = Math.floor(worldZ / 64);
    const startIndex = Math.floor(
      hashNoise3D(selectorCellX, 19, selectorCellZ, this.featureSeed + biomeId.length) * roster.length,
    );
    for (let attempt = 0; attempt < roster.length; attempt += 1) {
      const landmarkId = roster[(startIndex + attempt) % roster.length]!;
      const profile = LANDMARKS[landmarkId];
      const cellX = Math.floor(worldX / profile.cellSize);
      const cellZ = Math.floor(worldZ / profile.cellSize);
      const chance = hashNoise3D(cellX, 1, cellZ, this.featureSeed + profile.cellSize);
      if (chance > profile.chance) {
        continue;
      }
      const cellOriginX = cellX * profile.cellSize;
      const cellOriginZ = cellZ * profile.cellSize;
      const margin = profile.radius + 2;
      const span = profile.cellSize - margin * 2;
      const anchorX = cellOriginX + margin + Math.floor(hashNoise3D(cellX, 2, cellZ, this.featureSeed + profile.radius) * span);
      const anchorZ = cellOriginZ + margin + Math.floor(hashNoise3D(cellX, 3, cellZ, this.featureSeed + profile.radius * 2) * span);
      const deltaX = worldX - anchorX;
      const deltaZ = worldZ - anchorZ;
      if (Math.abs(deltaX) > profile.radius || Math.abs(deltaZ) > profile.radius) {
        continue;
      }
      out.featureDeltaX = deltaX;
      out.featureDeltaZ = deltaZ;
      if (configureLandmarkFeature(landmarkId, waterTopY, fields, out)) {
        return landmarkId;
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

function selectLandmarkRoster(biomeId: BiomeId): readonly LandmarkId[] {
  switch (biomeId) {
    case "marsh":
    case "ember":
    case "bloom":
      return SPECIAL_BIOME_LANDMARKS[biomeId];
    default:
      return BASE_BIOME_LANDMARKS[biomeId];
  }
}

function configureLandmarkFeature(
  landmarkId: LandmarkId,
  waterTopY: number,
  fields: ColumnFieldSample,
  out: MutableColumnState,
): boolean {
  switch (landmarkId) {
    case "oak":
      configureTreeFeature(out, FEATURE_OAK, 8 + Math.floor(fields.moisture * 5), 4, "#653", "#5B4");
      return true;
    case "birch":
      configureTreeFeature(out, FEATURE_OAK, 9 + Math.floor(fields.moisture * 4), 3, "#EEC", "#9C7");
      out.featureExtra = 1;
      return true;
    case "boulder":
      configureSpireFeature(out, FEATURE_BOULDER, 2 + Math.floor(fields.scatter * 2), 3, "#889", "#BBC");
      out.featureExtra = 0;
      return true;
    case "standing_stone":
      configureSpireFeature(out, FEATURE_STANDING_STONE, 7 + Math.floor(fields.uplift * 4), 2, "#998", "#CBA");
      return true;
    case "shrub":
      configureTreeFeature(out, FEATURE_BUSH, 2 + Math.floor(fields.moisture * 2), 2, "#764", "#7B6");
      return true;
    case "palm":
      if (waterTopY === NO_WATER && fields.channel < 0.68) {
        return false;
      }
      configureTreeFeature(out, FEATURE_PALM, 9 + Math.floor(fields.temperature * 4), 4, "#864", "#7B6");
      return true;
    case "cactus":
      configureTreeFeature(out, FEATURE_CACTUS, 5 + Math.floor(fields.temperature * 3), 2, "#596", "#7B8");
      out.featureExtra = 1 + Math.floor(fields.surfacePatch * 2);
      return true;
    case "hoodoo":
      configureSpireFeature(out, FEATURE_HOODOO, 8 + Math.floor(fields.mesa * 6), 3, "#B75", "#EBA");
      return true;
    case "fir":
      configureTreeFeature(out, FEATURE_FIR, 10 + Math.floor(fields.uplift * 5), 4, "#764", "#6A7");
      return true;
    case "ice_spire":
      configureSpireFeature(out, FEATURE_ICE_SPIRE, 8 + Math.floor((1 - fields.temperature) * 5), 3, "#CDE", "#EFF");
      return true;
    case "cypress":
      configureTreeFeature(out, FEATURE_CYPRESS, 8 + Math.floor(fields.drainage * 4), 3, "#554", "#486");
      return true;
    case "reed_cluster":
      if (waterTopY === NO_WATER && fields.channel < 0.58) {
        return false;
      }
      configureTreeFeature(out, FEATURE_REEDS, 3 + Math.floor(fields.moisture * 2), 2, "#684", "#8A6");
      out.featureExtra = 1;
      return true;
    case "basalt_spire":
      configureSpireFeature(out, FEATURE_BASALT_SPIRE, 9 + Math.floor(fields.volcanism * 6), 3, "#433", "#F74");
      return true;
    case "crystal_cluster":
      configureSpireFeature(out, FEATURE_CRYSTAL, 4 + Math.floor(fields.magic * 3), 3, "#79B", "#CEF");
      out.featureExtra = 1;
      return true;
    case "glowcap":
      configureTreeFeature(out, FEATURE_GLOWCAP, 8 + Math.floor(fields.magic * 5), 5, "#79B", "#8CF");
      out.featureExtra = 2;
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
): void {
  out.featureKind = featureKind;
  out.featureHeight = height;
  out.featureRadius = radius;
  out.featureExtra = 0;
  out.featureMaterialPrimary = hexColorToMaterial(materialPrimary);
  out.featureMaterialSecondary = hexColorToMaterial(materialSecondary);
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
    case FEATURE_OAK:
      if (relativeY <= featureHeight - 4) {
        const trunkRadius = featureExtra > 0 ? 0.55 : 0.75;
        return absX <= trunkRadius && absZ <= trunkRadius ? materialPrimary : 0;
      }
      return radial <= Math.max(featureExtra > 0 ? 1.25 : 1.5, featureRadius - Math.abs(relativeY - (featureHeight - 2)) * (featureExtra > 0 ? 0.7 : 0.9))
        ? materialSecondary
        : 0;
    case FEATURE_BOULDER:
      if (relativeY === featureHeight && radial <= Math.max(0.8, featureRadius - 0.6)) {
        return materialSecondary;
      }
      return radial <= Math.max(1, featureRadius - relativeY * 0.75) ? materialPrimary : 0;
    case FEATURE_BUSH:
      if (relativeY === 0 && absX <= 0.55 && absZ <= 0.55) {
        return materialPrimary;
      }
      return radial <= Math.max(1.1, featureRadius - relativeY * 0.6) ? materialSecondary : 0;
    case FEATURE_STANDING_STONE:
      return radial <= Math.max(1.1, featureRadius - relativeY * 0.2) ? materialPrimary : 0;
    case FEATURE_PALM:
      if (relativeY <= featureHeight - 2) {
        return absX <= 0.75 && absZ <= 0.75 ? materialPrimary : 0;
      }
      return relativeY === featureHeight - 1
        ? absX + absZ <= featureRadius + 0.5 ? materialSecondary : 0
        : radial <= 1.2 ? materialSecondary : 0;
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
    case FEATURE_FIR:
      if (relativeY <= 2) {
        return absX <= 0.75 && absZ <= 0.75 ? materialPrimary : 0;
      }
      return radial <= Math.max(1, featureRadius - (relativeY - 2) * 0.45) ? materialSecondary : 0;
    case FEATURE_ICE_SPIRE:
      return radial <= Math.max(0.8, featureRadius - relativeY * 0.35) ? materialSecondary : 0;
    case FEATURE_CYPRESS:
      if (relativeY <= 2) {
        return absX <= 0.75 && absZ <= 0.75 ? materialPrimary : 0;
      }
      return radial <= Math.max(1, featureRadius - Math.abs(relativeY - featureHeight * 0.6) * 0.25)
        ? materialSecondary
        : 0;
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
    case FEATURE_GLOWCAP:
      if (relativeY <= featureHeight - 3) {
        return absX <= 0.75 && absZ <= 0.75 ? materialPrimary : 0;
      }
      return relativeY <= featureHeight - 1
        ? radial <= Math.max(1.5, featureRadius - Math.abs(relativeY - (featureHeight - 2)) * 0.8)
          ? materialSecondary
          : 0
        : radial <= featureRadius + 0.5 ? materialSecondary : 0;
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
