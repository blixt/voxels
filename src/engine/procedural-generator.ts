import { clamp, packRgba } from "./math.ts";
import { fbm2D2, fbm2D3, fbm2D4, fbm2D5, hashNoise3D } from "./noise.ts";
import type { ChunkBounds, ChunkCoordinate } from "./types.ts";

export const HEX_COLOR_COUNT = 0x1000;
export const PROCEDURAL_WORLD_MAX_Y = 16_384;

type BiomeId = "verdant" | "dunes" | "badlands" | "tundra" | "ember";

interface BiomeProfile {
  id: BiomeId;
  heightBias: number;
  reliefScale: number;
  ridgeScale: number;
  detailScale: number;
  snowLine: number;
  surface: number;
  subsurface: number;
  stone: number;
  deepStone: number;
  accent: number;
  water: number;
  snow: number;
}

export interface ProceduralColumnSample {
  biomeId: BiomeId;
  surfaceY: number;
  waterTopY: number | null;
  surfaceMaterial: number;
}

export interface GeneratedChunk {
  coord: ChunkCoordinate;
  data: Uint16Array;
  solidCount: number;
  solidBounds: ChunkBounds | null;
}

interface ColumnMaterialState {
  biome: BiomeProfile;
  surfaceY: number;
  waterTopY: number;
  surfaceMaterial: number;
  strataOffset: number;
  worldXDiv3: number;
  worldZDiv3: number;
  accentSeed: number;
}

interface ChunkGenerationScratch {
  capacity: number;
  surfaceY: Int32Array;
  waterTopY: Int32Array;
  surfaceMaterial: Uint16Array;
  waterMaterial: Uint16Array;
  subsurfaceMaterial: Uint16Array;
  stoneMaterial: Uint16Array;
  deepStoneMaterial: Uint16Array;
  accentMaterial: Uint16Array;
  strataOffset: Float32Array;
  worldXDiv3: Int32Array;
  worldZDiv3: Int32Array;
  accentSeed: Int32Array;
}

const CONTINENT_SCALE = 1 / 2600;
const HILLS_SCALE = 1 / 900;
const DETAIL_SCALE = 1 / 180;
const RIDGE_SCALE = 1 / 480;
const BASIN_SCALE = 1 / 1400;
const STRATA_SCALE = 1 / 52;
const BIOME_TEMPERATURE_SCALE = 1 / 1400;
const BIOME_MOISTURE_SCALE = 1 / 1400;
const BIOME_WEIRDNESS_SCALE = 1 / 900;
const BIOME_SELECTOR_SCALE = 1 / 2200;
const ONE_THIRD = 1 / 3;
const STRATA_BAND_SCALE = 1 / 160;
const NO_WATER = -1;
const CHUNK_GENERATION_SCRATCH_POOL_LIMIT = 4;

const BIOMES: BiomeProfile[] = [
  createBiome("verdant", 20, 1.0, 0.9, 1.0, 1560, "#6A5", "#754", "#677", "#445", "#E97", "#4BF", "#DDE"),
  createBiome("dunes", -30, 0.55, 0.35, 0.7, 1700, "#DB6", "#B85", "#786", "#554", "#F9C", "#3AD", "#EDC"),
  createBiome("badlands", 10, 0.8, 0.8, 1.1, 1660, "#C75", "#A54", "#755", "#433", "#FE9", "#49B", "#ECC"),
  createBiome("tundra", 45, 1.1, 1.05, 0.85, 1480, "#BCC", "#99A", "#788", "#566", "#8DF", "#7AD", "#EEF"),
  createBiome("ember", -5, 0.9, 1.2, 1.25, 1780, "#543", "#654", "#433", "#322", "#F63", "#39B", "#DCC"),
];

const BIOME_BY_ID: Record<BiomeId, BiomeProfile> = {
  verdant: BIOMES[0]!,
  dunes: BIOMES[1]!,
  badlands: BIOMES[2]!,
  tundra: BIOMES[3]!,
  ember: BIOMES[4]!,
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
  private readonly hillsSeed: number;
  private readonly detailSeed: number;
  private readonly ridgeSeed: number;
  private readonly basinSeed: number;
  private readonly strataSeed: number;
  private readonly temperatureSeed: number;
  private readonly moistureSeed: number;
  private readonly weirdnessSeed: number;
  private readonly selectorSeed: number;

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
    this.hillsSeed = seed + 163;
    this.detailSeed = seed + 211;
    this.ridgeSeed = seed + 307;
    this.basinSeed = seed + 401;
    this.strataSeed = seed + 503;
    this.temperatureSeed = seed + 607;
    this.moistureSeed = seed + 701;
    this.weirdnessSeed = seed + 809;
    this.selectorSeed = seed + 911;
  }

  sampleColumn(worldX: number, worldZ: number): ProceduralColumnSample {
    const biome = this.sampleBiome(worldX, worldZ);
    const surfaceY = this.sampleSurfaceY(worldX, worldZ, biome);
    return {
      biomeId: biome.id,
      surfaceY,
      waterTopY: surfaceY < this.seaLevel ? this.seaLevel : null,
      surfaceMaterial: surfaceY >= biome.snowLine ? biome.snow : biome.surface,
    };
  }

  sampleMaterial(worldX: number, worldY: number, worldZ: number): number {
    if (worldY < 0 || worldY >= this.maxYExclusive) {
      return 0;
    }
    return this.sampleMaterialFromColumn(this.buildColumnMaterialState(worldX, worldZ), worldY);
  }

  generateChunk(cx: number, cy: number, cz: number): GeneratedChunk {
    const chunkArea = this.chunkSize * this.chunkSize;
    const data = new Uint16Array(this.chunkSize * chunkArea);
    const originX = cx * this.chunkSize;
    const originY = cy * this.chunkSize;
    const originZ = cz * this.chunkSize;
    const scratch = acquireChunkGenerationScratch(chunkArea);
    for (let z = 0; z < this.chunkSize; z += 1) {
      const worldZ = originZ + z;
      const rowOffset = z * this.chunkSize;
      for (let x = 0; x < this.chunkSize; x += 1) {
        const columnIndex = x + rowOffset;
        const worldX = originX + x;
        this.writeChunkColumnState(scratch, columnIndex, worldX, worldZ);
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
          const surfaceY = scratch.surfaceY[columnIndex]!;
          let material = 0;
          if (worldY > surfaceY) {
            const waterTopY = scratch.waterTopY[columnIndex]!;
            material = waterTopY !== NO_WATER && worldY <= waterTopY
              ? scratch.waterMaterial[columnIndex]!
              : 0;
          } else if (worldY === surfaceY) {
            material = scratch.surfaceMaterial[columnIndex]!;
          } else if (worldY >= surfaceY - 4) {
            material = scratch.subsurfaceMaterial[columnIndex]!;
          } else if (worldY < 24) {
            material = scratch.deepStoneMaterial[columnIndex]!;
          } else {
            const accentNoise = hashNoise3D(
              scratch.worldXDiv3[columnIndex]!,
              worldYDiv3,
              scratch.worldZDiv3[columnIndex]!,
              scratch.accentSeed[columnIndex]!,
            );
            if (worldY < surfaceY - 18 && accentNoise > 0.992) {
              material = scratch.accentMaterial[columnIndex]!;
            } else {
              const band = Math.abs(Math.floor(worldYBandBase + scratch.strataOffset[columnIndex]!)) % 3;
              if (band === 0) {
                material = scratch.stoneMaterial[columnIndex]!;
              } else if (band === 1) {
                material = scratch.deepStoneMaterial[columnIndex]!;
              } else {
                material = scratch.subsurfaceMaterial[columnIndex]!;
              }
            }
          }
          if (material !== 0) {
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
    }
    releaseChunkGenerationScratch(scratch);
    const solidBounds = solidCount === 0
      ? null
      : {
          min: [minX, minY, minZ] as [number, number, number],
          max: [maxX, maxY, maxZ] as [number, number, number],
        };
    return {
      coord: { x: cx, y: cy, z: cz },
      data,
      solidCount,
      solidBounds,
    };
  }

  private buildColumnMaterialState(worldX: number, worldZ: number): ColumnMaterialState {
    const biome = this.sampleBiome(worldX, worldZ);
    const surfaceY = this.sampleSurfaceY(worldX, worldZ, biome);
    return {
      biome,
      surfaceY,
      waterTopY: surfaceY < this.seaLevel ? this.seaLevel : NO_WATER,
      surfaceMaterial: surfaceY >= biome.snowLine ? biome.snow : biome.surface,
      strataOffset: fbm2D2(worldX * STRATA_SCALE, worldZ * STRATA_SCALE, this.strataSeed) * 5,
      worldXDiv3: Math.floor(worldX * ONE_THIRD),
      worldZDiv3: Math.floor(worldZ * ONE_THIRD),
      accentSeed: this.seed + biome.accent,
    };
  }

  private sampleMaterialFromColumn(context: ColumnMaterialState, worldY: number): number {
    const { biome, surfaceY, waterTopY, surfaceMaterial, strataOffset, worldXDiv3, worldZDiv3, accentSeed } = context;
    if (worldY > surfaceY) {
      return waterTopY !== NO_WATER && worldY <= waterTopY ? biome.water : 0;
    }
    if (worldY === surfaceY) {
      return surfaceMaterial;
    }
    if (worldY >= surfaceY - 4) {
      return biome.subsurface;
    }
    if (worldY < 24) {
      return biome.deepStone;
    }
    const accentNoise = hashNoise3D(
      worldXDiv3,
      Math.floor(worldY * ONE_THIRD),
      worldZDiv3,
      accentSeed,
    );
    if (worldY < surfaceY - 18 && accentNoise > 0.992) {
      return biome.accent;
    }
    const band = Math.abs(Math.floor(worldY * STRATA_BAND_SCALE + strataOffset)) % 3;
    if (band === 0) {
      return biome.stone;
    }
    if (band === 1) {
      return biome.deepStone;
    }
    return biome.subsurface;
  }

  private sampleSurfaceY(worldX: number, worldZ: number, biome: BiomeProfile): number {
    const scaledContinentX = worldX * CONTINENT_SCALE;
    const scaledContinentZ = worldZ * CONTINENT_SCALE;
    const scaledHillsX = worldX * HILLS_SCALE;
    const scaledHillsZ = worldZ * HILLS_SCALE;
    const scaledDetailX = worldX * DETAIL_SCALE;
    const scaledDetailZ = worldZ * DETAIL_SCALE;
    const scaledRidgeX = worldX * RIDGE_SCALE;
    const scaledRidgeZ = worldZ * RIDGE_SCALE;
    const scaledBasinX = worldX * BASIN_SCALE;
    const scaledBasinZ = worldZ * BASIN_SCALE;
    const continent = fbm2D5(scaledContinentX, scaledContinentZ, this.continentSeed) - 0.5;
    const hills = fbm2D4(scaledHillsX, scaledHillsZ, this.hillsSeed) - 0.5;
    const detail = fbm2D4(scaledDetailX, scaledDetailZ, this.detailSeed) - 0.5;
    const ridge = 1 - Math.abs(fbm2D3(scaledRidgeX, scaledRidgeZ, this.ridgeSeed) * 2 - 1);
    const basin = fbm2D3(scaledBasinX, scaledBasinZ, this.basinSeed) - 0.5;
    return clamp(
      Math.floor(
        this.seaLevel
          - 30
          + biome.heightBias
          + continent * 220
          + hills * 110 * biome.reliefScale
          + (ridge * ridge - 0.3) * 95 * biome.ridgeScale
          + detail * 18 * biome.detailScale
          + basin * 90,
      ),
      8,
      this.maxYExclusive - 2,
    );
  }

  private writeChunkColumnState(
    scratch: ChunkGenerationScratch,
    columnIndex: number,
    worldX: number,
    worldZ: number,
  ): void {
    const biome = this.sampleBiome(worldX, worldZ);
    const surfaceY = this.sampleSurfaceY(worldX, worldZ, biome);
    scratch.surfaceY[columnIndex] = surfaceY;
    scratch.waterTopY[columnIndex] = surfaceY < this.seaLevel ? this.seaLevel : NO_WATER;
    scratch.surfaceMaterial[columnIndex] = surfaceY >= biome.snowLine ? biome.snow : biome.surface;
    scratch.waterMaterial[columnIndex] = biome.water;
    scratch.subsurfaceMaterial[columnIndex] = biome.subsurface;
    scratch.stoneMaterial[columnIndex] = biome.stone;
    scratch.deepStoneMaterial[columnIndex] = biome.deepStone;
    scratch.accentMaterial[columnIndex] = biome.accent;
    scratch.strataOffset[columnIndex] = fbm2D2(worldX * STRATA_SCALE, worldZ * STRATA_SCALE, this.strataSeed) * 5;
    scratch.worldXDiv3[columnIndex] = Math.floor(worldX * ONE_THIRD);
    scratch.worldZDiv3[columnIndex] = Math.floor(worldZ * ONE_THIRD);
    scratch.accentSeed[columnIndex] = this.seed + biome.accent;
  }

  private sampleBiome(worldX: number, worldZ: number): BiomeProfile {
    const temperature = fbm2D4(worldX * BIOME_TEMPERATURE_SCALE, worldZ * BIOME_TEMPERATURE_SCALE, this.temperatureSeed);
    const moisture = fbm2D4(worldX * BIOME_MOISTURE_SCALE, worldZ * BIOME_MOISTURE_SCALE, this.moistureSeed);
    const weirdness = fbm2D3(worldX * BIOME_WEIRDNESS_SCALE, worldZ * BIOME_WEIRDNESS_SCALE, this.weirdnessSeed);
    const selector = fbm2D4(worldX * BIOME_SELECTOR_SCALE, worldZ * BIOME_SELECTOR_SCALE, this.selectorSeed);

    let biomeId: BiomeId;
    if (selector < 0.18) {
      biomeId = "dunes";
    } else if (selector < 0.38) {
      biomeId = "badlands";
    } else if (selector < 0.62) {
      biomeId = "verdant";
    } else if (selector < 0.82) {
      biomeId = "tundra";
    } else {
      biomeId = "ember";
    }

    if (temperature < 0.18) {
      biomeId = "tundra";
    } else if (temperature > 0.82 && moisture < 0.55) {
      biomeId = "dunes";
    } else if (weirdness > 0.84 && temperature > 0.35) {
      biomeId = "ember";
    } else if (moisture > 0.72 && biomeId === "badlands") {
      biomeId = "verdant";
    } else if (moisture < 0.24 && biomeId === "verdant") {
      biomeId = "badlands";
    }

    return BIOME_BY_ID[biomeId];
  }
}

function createBiome(
  id: BiomeId,
  heightBias: number,
  reliefScale: number,
  ridgeScale: number,
  detailScale: number,
  snowLine: number,
  surface: string,
  subsurface: string,
  stone: string,
  deepStone: string,
  accent: string,
  water: string,
  snow: string,
): BiomeProfile {
  return {
    id,
    heightBias,
    reliefScale,
    ridgeScale,
    detailScale,
    snowLine,
    surface: hexColorToMaterial(surface),
    subsurface: hexColorToMaterial(subsurface),
    stone: hexColorToMaterial(stone),
    deepStone: hexColorToMaterial(deepStone),
    accent: hexColorToMaterial(accent),
    water: hexColorToMaterial(water),
    snow: hexColorToMaterial(snow),
  };
}

function acquireChunkGenerationScratch(capacity: number): ChunkGenerationScratch {
  const scratch = chunkGenerationScratchPool.pop();
  if (!scratch || scratch.capacity < capacity) {
    return {
      capacity,
      surfaceY: new Int32Array(capacity),
      waterTopY: new Int32Array(capacity),
      surfaceMaterial: new Uint16Array(capacity),
      waterMaterial: new Uint16Array(capacity),
      subsurfaceMaterial: new Uint16Array(capacity),
      stoneMaterial: new Uint16Array(capacity),
      deepStoneMaterial: new Uint16Array(capacity),
      accentMaterial: new Uint16Array(capacity),
      strataOffset: new Float32Array(capacity),
      worldXDiv3: new Int32Array(capacity),
      worldZDiv3: new Int32Array(capacity),
      accentSeed: new Int32Array(capacity),
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
