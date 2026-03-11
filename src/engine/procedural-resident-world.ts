import type { ChunkCoordinate, Vec3, WorldStats } from "./types.ts";
import {
  ProceduralWorldGenerator,
  type GeneratedChunk,
} from "./procedural-generator.ts";
import type { ResidentChunkWorld, VoxelChunk } from "./world.ts";

const DEFAULT_HORIZONTAL_RADIUS_CHUNKS = 3;
const DEFAULT_UNDERGROUND_PADDING_CHUNKS = 3;
const DEFAULT_AIR_PADDING_CHUNKS = 2;
const SPAWN_FOOTPRINT_RADIUS = 32;

export interface ResidencyPhaseMetrics {
  surfaceSampleMs: number;
  yRangeMs: number;
  chunkGenerationMs: number;
  chunkAdoptionMs: number;
  evictionMs: number;
  neighborDirtyMs: number;
}

export interface ResidencyUpdateSummary {
  changed: boolean;
  centerChunkX: number;
  centerChunkZ: number;
  radiusChunks: number;
  generatedChunks: number;
  evictedChunks: number;
  emptyChunksSkipped: number;
  cachedEmptyChunkHits: number;
  touchedNeighborChunks: number;
  residentChunks: number;
  dirtyResidentChunks: number;
  surfaceY: number;
  elapsedMs: number;
  generatedChunkCoords: ChunkCoordinate[];
  evictedChunkCoords: ChunkCoordinate[];
  phaseMs: ResidencyPhaseMetrics;
}

export class ProceduralResidentWorld implements ResidentChunkWorld {
  readonly chunkSize: number;
  readonly minY = 0;
  readonly maxYExclusive: number;
  readonly palette: number[];

  horizontalRadiusChunks: number;
  undergroundPaddingChunks: number;
  airPaddingChunks: number;
  lastResidency: ResidencyUpdateSummary;

  private readonly chunks = new Map<string, VoxelChunk>();
  private readonly emptyChunkKeys = new Set<string>();
  private lastAnchorSignature = "";

  constructor(
    readonly generator: ProceduralWorldGenerator,
    options: {
      horizontalRadiusChunks?: number;
      undergroundPaddingChunks?: number;
      airPaddingChunks?: number;
    } = {},
  ) {
    this.chunkSize = generator.chunkSize;
    this.maxYExclusive = generator.maxYExclusive;
    this.palette = generator.palette;
    this.horizontalRadiusChunks = options.horizontalRadiusChunks ?? DEFAULT_HORIZONTAL_RADIUS_CHUNKS;
    this.undergroundPaddingChunks = options.undergroundPaddingChunks ?? DEFAULT_UNDERGROUND_PADDING_CHUNKS;
    this.airPaddingChunks = options.airPaddingChunks ?? DEFAULT_AIR_PADDING_CHUNKS;
    this.lastResidency = {
      changed: false,
      centerChunkX: 0,
      centerChunkZ: 0,
      radiusChunks: this.horizontalRadiusChunks,
      generatedChunks: 0,
      evictedChunks: 0,
      emptyChunksSkipped: 0,
      cachedEmptyChunkHits: 0,
      touchedNeighborChunks: 0,
      residentChunks: 0,
      dirtyResidentChunks: 0,
      surfaceY: 0,
      elapsedMs: 0,
      generatedChunkCoords: [],
      evictedChunkCoords: [],
      phaseMs: zeroResidencyPhaseMetrics(),
    };
  }

  setHorizontalRadiusChunks(radius: number): void {
    this.horizontalRadiusChunks = Math.max(1, Math.floor(radius));
    this.lastAnchorSignature = "";
  }

  getStats(): WorldStats {
    let solidVoxelCount = 0;
    for (const chunk of this.chunks.values()) {
      solidVoxelCount += chunk.solidCount;
    }
    return {
      solidVoxelCount,
      chunkCount: this.chunks.size,
      paletteCount: this.palette.length - 1,
    };
  }

  getPaletteColor(materialIndex: number): number {
    return this.palette[materialIndex] ?? 0;
  }

  getVoxel(x: number, y: number, z: number): number {
    if (y < this.minY || y >= this.maxYExclusive) {
      return 0;
    }
    const cx = Math.floor(x / this.chunkSize);
    const cy = Math.floor(y / this.chunkSize);
    const cz = Math.floor(z / this.chunkSize);
    const chunk = this.getResidentChunk(cx, cy, cz);
    if (!chunk) {
      return 0;
    }
    const lx = x - cx * this.chunkSize;
    const ly = y - cy * this.chunkSize;
    const lz = z - cz * this.chunkSize;
    return chunk.data[lx + ly * this.chunkSize + lz * this.chunkSize * this.chunkSize] ?? 0;
  }

  getResidentChunk(cx: number, cy: number, cz: number): VoxelChunk | null {
    return this.chunks.get(toChunkKey(cx, cy, cz)) ?? null;
  }

  hasResidentChunk(cx: number, cy: number, cz: number): boolean {
    return this.chunks.has(toChunkKey(cx, cy, cz));
  }

  *iterateResidentChunks(): Iterable<VoxelChunk> {
    for (const chunk of this.chunks.values()) {
      yield chunk;
    }
  }

  getChunkSolidBounds(
    cx: number,
    cy: number,
    cz: number,
  ): {
    min: [number, number, number];
    max: [number, number, number];
  } | null {
    const chunk = this.getResidentChunk(cx, cy, cz);
    if (!chunk || chunk.solidCount === 0 || !chunk.solidBounds) {
      return null;
    }
    if (chunk.solidBounds.dirty) {
      recomputeChunkSolidBounds(chunk, this.chunkSize);
    }
    if (!chunk.solidBounds) {
      return null;
    }
    return {
      min: [...chunk.solidBounds.min],
      max: [...chunk.solidBounds.max],
    };
  }

  getSpawnPosition(): Vec3 {
    const preferredBiomes = new Set(["verdant", "dunes", "badlands"]);
    let fallback = this.sampleSpawnCandidate(0, 0);
    let fallbackPosition: Vec3 = [0.5, fallback.standY, 0.5];
    for (let ring = 0; ring <= 24; ring += 1) {
      for (const [offsetX, offsetZ] of spawnSearchOffsets(ring)) {
        const worldX = offsetX * this.chunkSize * 2;
        const worldZ = offsetZ * this.chunkSize * 2;
        const candidate = this.sampleSpawnCandidate(worldX, worldZ);
        if (
          preferredBiomes.has(candidate.column.biomeId)
          && candidate.column.surfaceY >= this.generator.seaLevel - 48
          && candidate.column.surfaceY <= this.generator.seaLevel + 320
          && candidate.surfaceSpread <= 12
        ) {
          return [worldX + 0.5, candidate.standY, worldZ + 0.5];
        }
        if (
          candidate.surfaceSpread < fallback.surfaceSpread
          || (
            candidate.surfaceSpread === fallback.surfaceSpread
            && Math.abs(candidate.column.surfaceY - this.generator.seaLevel)
              < Math.abs(fallback.column.surfaceY - this.generator.seaLevel)
          )
        ) {
          fallback = candidate;
          fallbackPosition = [worldX + 0.5, candidate.standY, worldZ + 0.5];
        }
      }
    }
    return fallbackPosition;
  }

  private sampleSpawnCandidate(worldX: number, worldZ: number): {
    column: ReturnType<ProceduralWorldGenerator["sampleColumn"]>;
    standY: number;
    surfaceSpread: number;
  } {
    const column = this.generator.sampleColumn(worldX, worldZ);
    let minSurfaceY = column.surfaceY;
    let maxSurfaceY = column.surfaceY;
    for (const [offsetX, offsetZ] of spawnFootprintOffsets()) {
      const sampled = this.generator.sampleColumn(worldX + offsetX, worldZ + offsetZ);
      minSurfaceY = Math.min(minSurfaceY, sampled.surfaceY);
      maxSurfaceY = Math.max(maxSurfaceY, sampled.surfaceY);
    }
    return {
      column,
      standY: maxSurfaceY + 1,
      surfaceSpread: maxSurfaceY - minSurfaceY,
    };
  }

  updateResidencyAround(position: Vec3): ResidencyUpdateSummary {
    const centerChunkX = Math.floor(position[0] / this.chunkSize);
    const centerChunkZ = Math.floor(position[2] / this.chunkSize);
    const radiusChunks = this.horizontalRadiusChunks;
    const anchorSignature = `${centerChunkX}:${centerChunkZ}:${radiusChunks}`;
    const surfaceStartedAt = performance.now();
    const surfaceY = this.generator.sampleColumn(Math.floor(position[0]), Math.floor(position[2])).surfaceY;
    const surfaceSampleMs = performance.now() - surfaceStartedAt;
    if (anchorSignature === this.lastAnchorSignature) {
      this.lastResidency = {
        ...this.lastResidency,
        changed: false,
        centerChunkX,
        centerChunkZ,
        radiusChunks,
        surfaceY,
        elapsedMs: 0,
        generatedChunks: 0,
        evictedChunks: 0,
        emptyChunksSkipped: 0,
        cachedEmptyChunkHits: 0,
        touchedNeighborChunks: 0,
        residentChunks: this.chunks.size,
        dirtyResidentChunks: countDirtyResidentChunks(this.chunks.values()),
        generatedChunkCoords: [],
        evictedChunkCoords: [],
        phaseMs: {
          ...zeroResidencyPhaseMetrics(),
          surfaceSampleMs,
        },
      };
      return this.lastResidency;
    }

    const startedAt = performance.now();
    const neededKeys = new Set<string>();
    let generatedChunks = 0;
    let evictedChunks = 0;
    let emptyChunksSkipped = 0;
    let cachedEmptyChunkHits = 0;
    let touchedNeighborChunks = 0;
    const generatedChunkCoords: ChunkCoordinate[] = [];
    const evictedChunkCoords: ChunkCoordinate[] = [];
    let yRangeMs = 0;
    let chunkGenerationMs = 0;
    let chunkAdoptionMs = 0;
    let evictionMs = 0;
    let neighborDirtyMs = 0;

    for (let dz = -radiusChunks; dz <= radiusChunks; dz += 1) {
      for (let dx = -radiusChunks; dx <= radiusChunks; dx += 1) {
        if (dx * dx + dz * dz > radiusChunks * radiusChunks) {
          continue;
        }
        const cx = centerChunkX + dx;
        const cz = centerChunkZ + dz;
        const yRangeStartedAt = performance.now();
        const [minCy, maxCy] = this.computeChunkYRange(cx, cz);
        yRangeMs += performance.now() - yRangeStartedAt;
        for (let cy = minCy; cy <= maxCy; cy += 1) {
          const key = toChunkKey(cx, cy, cz);
          neededKeys.add(key);
          if (this.chunks.has(key)) {
            continue;
          }
          if (this.emptyChunkKeys.has(key)) {
            cachedEmptyChunkHits += 1;
            continue;
          }
          const generationStartedAt = performance.now();
          const generated = this.generator.generateChunk(cx, cy, cz);
          chunkGenerationMs += performance.now() - generationStartedAt;
          if (generated.solidCount === 0) {
            emptyChunksSkipped += 1;
            this.emptyChunkKeys.add(key);
            continue;
          }
          const adoptionStartedAt = performance.now();
          const chunk = createResidentChunk(generated, this.chunkSize);
          this.emptyChunkKeys.delete(key);
          this.chunks.set(key, chunk);
          generatedChunks += 1;
          generatedChunkCoords.push({ x: cx, y: cy, z: cz });
          chunkAdoptionMs += performance.now() - adoptionStartedAt;
          const dirtyStartedAt = performance.now();
          touchedNeighborChunks += this.markAdjacentChunksDirty(cx, cy, cz);
          neighborDirtyMs += performance.now() - dirtyStartedAt;
        }
      }
    }

    for (const [key, chunk] of [...this.chunks.entries()]) {
      if (neededKeys.has(key)) {
        continue;
      }
      const evictionStartedAt = performance.now();
      this.chunks.delete(key);
      evictedChunks += 1;
      evictedChunkCoords.push({ ...chunk.coord });
      evictionMs += performance.now() - evictionStartedAt;
      const dirtyStartedAt = performance.now();
      touchedNeighborChunks += this.markAdjacentChunksDirty(chunk.coord.x, chunk.coord.y, chunk.coord.z);
      neighborDirtyMs += performance.now() - dirtyStartedAt;
    }

    this.lastAnchorSignature = anchorSignature;
    this.lastResidency = {
      changed: true,
      centerChunkX,
      centerChunkZ,
      radiusChunks,
      generatedChunks,
      evictedChunks,
      emptyChunksSkipped,
      cachedEmptyChunkHits,
      touchedNeighborChunks,
      residentChunks: this.chunks.size,
      dirtyResidentChunks: countDirtyResidentChunks(this.chunks.values()),
      surfaceY,
      elapsedMs: performance.now() - startedAt,
      generatedChunkCoords,
      evictedChunkCoords,
      phaseMs: {
        surfaceSampleMs,
        yRangeMs,
        chunkGenerationMs,
        chunkAdoptionMs,
        evictionMs,
        neighborDirtyMs,
      },
    };
    return this.lastResidency;
  }

  private computeChunkYRange(cx: number, cz: number): [number, number] {
    const chunkOriginX = cx * this.chunkSize;
    const chunkOriginZ = cz * this.chunkSize;
    let minSurfaceY = Number.POSITIVE_INFINITY;
    let maxSurfaceY = Number.NEGATIVE_INFINITY;
    let maxWaterTopY = Number.NEGATIVE_INFINITY;
    for (const [offsetX, offsetZ] of sampleOffsets(this.chunkSize)) {
      const column = this.generator.sampleColumn(chunkOriginX + offsetX, chunkOriginZ + offsetZ);
      minSurfaceY = Math.min(minSurfaceY, column.surfaceY);
      maxSurfaceY = Math.max(maxSurfaceY, column.surfaceY);
      maxWaterTopY = Math.max(maxWaterTopY, column.waterTopY ?? column.surfaceY);
    }
    const minWorldY = Math.max(this.minY, minSurfaceY - this.undergroundPaddingChunks * this.chunkSize);
    const maxWorldY = Math.min(
      this.maxYExclusive - 1,
      Math.max(maxSurfaceY, maxWaterTopY) + this.airPaddingChunks * this.chunkSize,
    );
    return [
      Math.max(0, Math.floor(minWorldY / this.chunkSize)),
      Math.max(0, Math.floor(maxWorldY / this.chunkSize)),
    ];
  }

  private markAdjacentChunksDirty(cx: number, cy: number, cz: number): number {
    let touched = 0;
    for (const [dx, dy, dz] of ADJACENT_CHUNK_OFFSETS) {
      const chunk = this.getResidentChunk(cx + dx, cy + dy, cz + dz);
      if (!chunk) {
        continue;
      }
      chunk.meshDirty = true;
      chunk.gpuDirty = true;
      chunk.mesh = null;
      touched += 1;
    }
    return touched;
  }
}

const ADJACENT_CHUNK_OFFSETS: ReadonlyArray<readonly [number, number, number]> = [
  [-1, 0, 0],
  [1, 0, 0],
  [0, -1, 0],
  [0, 1, 0],
  [0, 0, -1],
  [0, 0, 1],
];

function sampleOffsets(chunkSize: number): Array<[number, number]> {
  return [
    [0, 0],
    [chunkSize >> 1, chunkSize >> 1],
    [chunkSize - 1, 0],
    [0, chunkSize - 1],
    [chunkSize - 1, chunkSize - 1],
  ];
}

function spawnSearchOffsets(ring: number): Array<[number, number]> {
  if (ring === 0) {
    return [[0, 0]];
  }
  const offsets: Array<[number, number]> = [];
  for (let value = -ring; value <= ring; value += 1) {
    offsets.push([value, -ring], [value, ring], [-ring, value], [ring, value]);
  }
  return offsets;
}

function spawnFootprintOffsets(): Array<[number, number]> {
  return [
    [0, 0],
    [-SPAWN_FOOTPRINT_RADIUS, 0],
    [SPAWN_FOOTPRINT_RADIUS, 0],
    [0, -SPAWN_FOOTPRINT_RADIUS],
    [0, SPAWN_FOOTPRINT_RADIUS],
    [-SPAWN_FOOTPRINT_RADIUS, -SPAWN_FOOTPRINT_RADIUS],
    [-SPAWN_FOOTPRINT_RADIUS, SPAWN_FOOTPRINT_RADIUS],
    [SPAWN_FOOTPRINT_RADIUS, -SPAWN_FOOTPRINT_RADIUS],
    [SPAWN_FOOTPRINT_RADIUS, SPAWN_FOOTPRINT_RADIUS],
  ];
}

function toChunkKey(cx: number, cy: number, cz: number): string {
  return `${cx}:${cy}:${cz}`;
}

function createResidentChunk(generated: GeneratedChunk, chunkSize: number): VoxelChunk {
  const chunk: VoxelChunk = {
    coord: generated.coord,
    data: generated.data,
    solidCount: generated.solidCount,
    solidBounds: generated.solidBounds
      ? {
          min: [...generated.solidBounds.min],
          max: [...generated.solidBounds.max],
          dirty: false,
        }
      : null,
    meshBuilt: false,
    meshDirty: true,
    gpuDirty: true,
    mesh: null,
  };
  if (chunk.solidCount > 0 && !chunk.solidBounds) {
    recomputeChunkSolidBounds(chunk, chunkSize);
  }
  return chunk;
}

function countDirtyResidentChunks(chunks: Iterable<VoxelChunk>): number {
  let dirtyCount = 0;
  for (const chunk of chunks) {
    if (chunk.meshDirty) {
      dirtyCount += 1;
    }
  }
  return dirtyCount;
}

function zeroResidencyPhaseMetrics(): ResidencyPhaseMetrics {
  return {
    surfaceSampleMs: 0,
    yRangeMs: 0,
    chunkGenerationMs: 0,
    chunkAdoptionMs: 0,
    evictionMs: 0,
    neighborDirtyMs: 0,
  };
}

function recomputeChunkSolidBounds(chunk: VoxelChunk, chunkSize: number): void {
  if (chunk.solidCount === 0) {
    chunk.solidBounds = null;
    return;
  }
  const chunkArea = chunkSize * chunkSize;
  let minX = chunkSize;
  let minY = chunkSize;
  let minZ = chunkSize;
  let maxX = 0;
  let maxY = 0;
  let maxZ = 0;
  for (let index = 0; index < chunk.data.length; index += 1) {
    if (chunk.data[index] === 0) {
      continue;
    }
    const x = index % chunkSize;
    const y = Math.floor(index / chunkSize) % chunkSize;
    const z = Math.floor(index / chunkArea);
    minX = Math.min(minX, x);
    minY = Math.min(minY, y);
    minZ = Math.min(minZ, z);
    maxX = Math.max(maxX, x + 1);
    maxY = Math.max(maxY, y + 1);
    maxZ = Math.max(maxZ, z + 1);
  }
  chunk.solidBounds = {
    min: [minX, minY, minZ],
    max: [maxX, maxY, maxZ],
    dirty: false,
  };
}
