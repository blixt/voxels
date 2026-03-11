import { clamp, packRgba } from "./math.ts";
import { fbm2D, hashNoise3D } from "./noise.ts";
import type { ChunkBounds, ChunkCoordinate } from "./types.ts";

export const HEX_COLOR_COUNT = 0x1000;
export const PROCEDURAL_WORLD_MAX_Y = 16_384;

type BiomeId = "verdant" | "dunes" | "badlands" | "tundra" | "ember";

interface BiomeProfile {
  id: BiomeId;
  baseHeight: number;
  relief: number;
  ridge: number;
  detail: number;
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

interface ColumnContext {
  biome: BiomeProfile;
  column: ProceduralColumnSample;
  strataNoise: number;
}

const BIOMES: BiomeProfile[] = [
  createBiome("verdant", 1320, 360, 260, 52, 2100, "#6A5", "#754", "#677", "#445", "#E97", "#4BF", "#DDE"),
  createBiome("dunes", 1180, 190, 80, 36, 2300, "#DB6", "#B85", "#786", "#554", "#F9C", "#3AD", "#EDC"),
  createBiome("badlands", 1460, 320, 210, 60, 2500, "#C75", "#A54", "#755", "#433", "#FE9", "#49B", "#ECC"),
  createBiome("tundra", 1620, 380, 460, 48, 1750, "#BCC", "#99A", "#788", "#566", "#8DF", "#7AD", "#EEF"),
  createBiome("ember", 980, 520, 920, 76, 3000, "#543", "#654", "#433", "#322", "#F63", "#39B", "#DCC"),
];

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
  }

  sampleColumn(worldX: number, worldZ: number): ProceduralColumnSample {
    return this.buildColumnContext(worldX, worldZ).column;
  }

  sampleMaterial(worldX: number, worldY: number, worldZ: number): number {
    if (worldY < 0 || worldY >= this.maxYExclusive) {
      return 0;
    }
    return this.sampleMaterialFromColumn(this.buildColumnContext(worldX, worldZ), worldX, worldY, worldZ);
  }

  generateChunk(cx: number, cy: number, cz: number): GeneratedChunk {
    const chunkArea = this.chunkSize * this.chunkSize;
    const data = new Uint16Array(this.chunkSize * chunkArea);
    const originX = cx * this.chunkSize;
    const originY = cy * this.chunkSize;
    const originZ = cz * this.chunkSize;
    const columnContexts = new Array<ColumnContext>(chunkArea);
    const worldXs = new Int32Array(chunkArea);
    const worldZs = new Int32Array(chunkArea);
    for (let z = 0; z < this.chunkSize; z += 1) {
      const worldZ = originZ + z;
      const rowOffset = z * this.chunkSize;
      for (let x = 0; x < this.chunkSize; x += 1) {
        const columnIndex = x + rowOffset;
        const worldX = originX + x;
        columnContexts[columnIndex] = this.buildColumnContext(worldX, worldZ);
        worldXs[columnIndex] = worldX;
        worldZs[columnIndex] = worldZ;
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
        const planeOffset = y * this.chunkSize + z * chunkArea;
        for (let x = 0; x < this.chunkSize; x += 1) {
          const columnIndex = x + rowOffset;
          const material = this.sampleMaterialFromColumn(
            columnContexts[columnIndex]!,
            worldXs[columnIndex]!,
            worldY,
            worldZs[columnIndex]!,
          );
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

  private buildColumnContext(worldX: number, worldZ: number): ColumnContext {
    const biome = this.sampleBiome(worldX, worldZ);
    const macro = fbm2D(worldX / 1024, worldZ / 1024, 5, this.seed + 101);
    const detail = fbm2D(worldX / 144, worldZ / 144, 4, this.seed + 211) - 0.5;
    const ridge = 1 - Math.abs(fbm2D(worldX / 280, worldZ / 280, 3, this.seed + 307) * 2 - 1);
    const basin = fbm2D(worldX / 560, worldZ / 560, 3, this.seed + 401) - 0.5;
    const surfaceY = clamp(
      Math.floor(
        biome.baseHeight
          + macro * biome.relief
          + ridge * ridge * biome.ridge
          + detail * biome.detail
          + basin * 140,
      ),
      8,
      this.maxYExclusive - 2,
    );
    return {
      biome,
      column: {
        biomeId: biome.id,
        surfaceY,
        waterTopY: surfaceY < this.seaLevel ? this.seaLevel : null,
        surfaceMaterial: surfaceY >= biome.snowLine ? biome.snow : biome.surface,
      },
      strataNoise: fbm2D(worldX / 52, worldZ / 52, 2, this.seed + 503),
    };
  }

  private sampleMaterialFromColumn(
    context: ColumnContext,
    worldX: number,
    worldY: number,
    worldZ: number,
  ): number {
    const { biome, column, strataNoise } = context;
    if (worldY > column.surfaceY) {
      return column.waterTopY !== null && worldY <= column.waterTopY ? biome.water : 0;
    }
    if (worldY === column.surfaceY) {
      return column.surfaceMaterial;
    }
    if (worldY >= column.surfaceY - 4) {
      return biome.subsurface;
    }
    if (worldY < 24) {
      return biome.deepStone;
    }
    const accentNoise = hashNoise3D(
      Math.floor(worldX / 3),
      Math.floor(worldY / 3),
      Math.floor(worldZ / 3),
      this.seed + biome.accent,
    );
    if (worldY < column.surfaceY - 18 && accentNoise > 0.992) {
      return biome.accent;
    }
    const band = Math.abs(Math.floor((worldY / 160) + strataNoise * 5)) % 3;
    if (band === 0) {
      return biome.stone;
    }
    if (band === 1) {
      return biome.deepStone;
    }
    return biome.subsurface;
  }

  private sampleBiome(worldX: number, worldZ: number): BiomeProfile {
    const temperature = fbm2D(worldX / 1400, worldZ / 1400, 4, this.seed + 607);
    const moisture = fbm2D(worldX / 1400, worldZ / 1400, 4, this.seed + 701);
    const weirdness = fbm2D(worldX / 900, worldZ / 900, 3, this.seed + 809);
    const selector = fbm2D(worldX / 2200, worldZ / 2200, 4, this.seed + 911);

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

    return biomeById(biomeId);
  }
}

function createBiome(
  id: BiomeId,
  baseHeight: number,
  relief: number,
  ridge: number,
  detail: number,
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
    baseHeight,
    relief,
    ridge,
    detail,
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

function biomeById(id: BiomeId): BiomeProfile {
  const biome = BIOMES.find((candidate) => candidate.id === id);
  if (!biome) {
    throw new Error(`Unknown biome "${id}"`);
  }
  return biome;
}
