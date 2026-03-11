import type { ChunkCoordinate, Vec3, WorldStats } from "./types.ts";
import {
  ProceduralWorldGenerator,
  type GeneratedChunk,
} from "./procedural-generator.ts";
import type { FarFieldExclusionMask } from "./procedural-far-field.ts";
import { metersToWorldUnits } from "./scale.ts";
import type { ResidentChunkWorld, VoxelChunk } from "./world.ts";

const DEFAULT_HORIZONTAL_RADIUS_CHUNKS = 8;
const DEFAULT_UNDERGROUND_PADDING_CHUNKS = 3;
const DEFAULT_AIR_PADDING_CHUNKS = 2;
const SPAWN_FOOTPRINT_RADIUS = metersToWorldUnits(0.8);

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
  complete: boolean;
  centerChunkX: number;
  centerChunkZ: number;
  radiusChunks: number;
  generatedChunks: number;
  evictedChunks: number;
  pendingChunks: number;
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
  private readonly residentColumnCounts = new Map<string, number>();
  private residentColumnRevision = 0;
  private lastAnchorSignature = "";
  private lastAnchorComplete = true;

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
      complete: true,
      centerChunkX: 0,
      centerChunkZ: 0,
      radiusChunks: this.horizontalRadiusChunks,
      generatedChunks: 0,
      evictedChunks: 0,
      pendingChunks: 0,
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
    this.lastAnchorComplete = false;
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

  countDirtyResidentChunks(): number {
    return countDirtyResidentChunks(this.chunks.values());
  }

  hasResidentColumn(cx: number, cz: number): boolean {
    return (this.residentColumnCounts.get(toColumnKey(cx, cz)) ?? 0) > 0;
  }

  hasRenderReadyColumn(cx: number, cz: number): boolean {
    return this.buildRenderReadyColumnKeys().has(toColumnKey(cx, cz));
  }

  intersectsResidentColumns(
    minX: number,
    maxXExclusive: number,
    minZ: number,
    maxZExclusive: number,
  ): boolean {
    const minChunkX = Math.floor(minX / this.chunkSize);
    const maxChunkX = Math.floor((maxXExclusive - 1) / this.chunkSize);
    const minChunkZ = Math.floor(minZ / this.chunkSize);
    const maxChunkZ = Math.floor((maxZExclusive - 1) / this.chunkSize);
    for (let cz = minChunkZ; cz <= maxChunkZ; cz += 1) {
      for (let cx = minChunkX; cx <= maxChunkX; cx += 1) {
        if (this.hasResidentColumn(cx, cz)) {
          return true;
        }
      }
    }
    return false;
  }

  getFarFieldExclusionMask(
    mode: "resident" | "render-ready" = "resident",
    revisionOverride?: number,
  ): FarFieldExclusionMask {
    const renderReadyColumnKeys = mode === "render-ready"
      ? this.buildRenderReadyColumnKeys()
      : null;
    return {
      revision: revisionOverride ?? this.residentColumnRevision,
      maxAffectedRadiusWorldUnits: (this.horizontalRadiusChunks + 1) * this.chunkSize,
      excludesCell: (minX, maxXExclusive, minZ, maxZExclusive) => {
        if (mode === "render-ready" && renderReadyColumnKeys) {
          return intersectsColumnKeySet(
            renderReadyColumnKeys,
            minX,
            maxXExclusive,
            minZ,
            maxZExclusive,
            this.chunkSize,
          );
        }
        return this.intersectsResidentColumns(minX, maxXExclusive, minZ, maxZExclusive);
      },
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

  updateResidencyAround(
    position: Vec3,
    options: {
      maxGenerateChunks?: number;
    } = {},
  ): ResidencyUpdateSummary {
    const centerChunkX = Math.floor(position[0] / this.chunkSize);
    const centerChunkZ = Math.floor(position[2] / this.chunkSize);
    const radiusChunks = this.horizontalRadiusChunks;
    const anchorSignature = `${centerChunkX}:${centerChunkZ}:${radiusChunks}`;
    const maxGenerateChunks = options.maxGenerateChunks ?? Number.POSITIVE_INFINITY;
    const surfaceStartedAt = performance.now();
    const surfaceY = this.generator.sampleColumn(Math.floor(position[0]), Math.floor(position[2])).surfaceY;
    const surfaceSampleMs = performance.now() - surfaceStartedAt;
    if (anchorSignature === this.lastAnchorSignature && this.lastAnchorComplete) {
      this.lastResidency = {
        ...this.lastResidency,
        changed: false,
        complete: true,
        centerChunkX,
        centerChunkZ,
        radiusChunks,
        surfaceY,
        elapsedMs: 0,
        generatedChunks: 0,
        evictedChunks: 0,
        pendingChunks: 0,
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
    let pendingChunks = 0;
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

    for (const [dx, dz] of prioritizedColumnOffsets(radiusChunks)) {
      const cx = centerChunkX + dx;
      const cz = centerChunkZ + dz;
      const yRangeStartedAt = performance.now();
      const [minCy, maxCy] = this.computeChunkYRange(cx, cz);
      yRangeMs += performance.now() - yRangeStartedAt;
      const preferredCy = dx === 0 && dz === 0
        ? Math.floor(surfaceY / this.chunkSize)
        : Math.floor((minCy + maxCy) * 0.5);
      for (const cy of prioritizedChunkYRange(minCy, maxCy, preferredCy)) {
        const key = toChunkKey(cx, cy, cz);
        neededKeys.add(key);
        if (this.chunks.has(key)) {
          continue;
        }
        if (this.emptyChunkKeys.has(key)) {
          cachedEmptyChunkHits += 1;
          continue;
        }
        if (generatedChunks >= maxGenerateChunks) {
          pendingChunks += 1;
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
        this.adoptResidentChunk(key, chunk);
        generatedChunks += 1;
        generatedChunkCoords.push({ x: cx, y: cy, z: cz });
        chunkAdoptionMs += performance.now() - adoptionStartedAt;
        const dirtyStartedAt = performance.now();
        touchedNeighborChunks += this.markAdjacentChunksDirty(cx, cy, cz);
        neighborDirtyMs += performance.now() - dirtyStartedAt;
      }
    }

    for (const [key, chunk] of [...this.chunks.entries()]) {
      if (neededKeys.has(key)) {
        continue;
      }
      const evictionStartedAt = performance.now();
      this.evictResidentChunk(key, chunk);
      evictedChunks += 1;
      evictedChunkCoords.push({ ...chunk.coord });
      evictionMs += performance.now() - evictionStartedAt;
      const dirtyStartedAt = performance.now();
      touchedNeighborChunks += this.markAdjacentChunksDirty(chunk.coord.x, chunk.coord.y, chunk.coord.z);
      neighborDirtyMs += performance.now() - dirtyStartedAt;
    }

    this.lastAnchorSignature = anchorSignature;
    this.lastAnchorComplete = pendingChunks === 0;
    this.lastResidency = {
      changed: generatedChunks > 0 || evictedChunks > 0,
      complete: pendingChunks === 0,
      centerChunkX,
      centerChunkZ,
      radiusChunks,
      generatedChunks,
      evictedChunks,
      pendingChunks,
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

  private adoptResidentChunk(key: string, chunk: VoxelChunk): void {
    this.chunks.set(key, chunk);
    const columnKey = toColumnKey(chunk.coord.x, chunk.coord.z);
    const previous = this.residentColumnCounts.get(columnKey) ?? 0;
    this.residentColumnCounts.set(columnKey, previous + 1);
    if (previous === 0) {
      this.residentColumnRevision += 1;
    }
  }

  private evictResidentChunk(key: string, chunk: VoxelChunk): void {
    this.chunks.delete(key);
    const columnKey = toColumnKey(chunk.coord.x, chunk.coord.z);
    const previous = this.residentColumnCounts.get(columnKey) ?? 0;
    if (previous <= 1) {
      this.residentColumnCounts.delete(columnKey);
      if (previous === 1) {
        this.residentColumnRevision += 1;
      }
      return;
    }
    this.residentColumnCounts.set(columnKey, previous - 1);
  }

  private buildRenderReadyColumnKeys(): Set<string> {
    const readyCounts = new Map<string, number>();
    for (const chunk of this.chunks.values()) {
      if (!chunk.meshBuilt || chunk.meshDirty) {
        continue;
      }
      const columnKey = toColumnKey(chunk.coord.x, chunk.coord.z);
      readyCounts.set(columnKey, (readyCounts.get(columnKey) ?? 0) + 1);
    }

    const readyKeys = new Set<string>();
    for (const [columnKey, residentCount] of this.residentColumnCounts) {
      if ((readyCounts.get(columnKey) ?? 0) === residentCount) {
        readyKeys.add(columnKey);
      }
    }
    return readyKeys;
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
const PRIORITIZED_COLUMN_OFFSETS = new Map<number, Array<[number, number]>>();

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

function prioritizedColumnOffsets(radiusChunks: number): Array<[number, number]> {
  const cached = PRIORITIZED_COLUMN_OFFSETS.get(radiusChunks);
  if (cached) {
    return cached;
  }
  const offsets: Array<[number, number]> = [];
  for (let dz = -radiusChunks; dz <= radiusChunks; dz += 1) {
    for (let dx = -radiusChunks; dx <= radiusChunks; dx += 1) {
      if (dx * dx + dz * dz > radiusChunks * radiusChunks) {
        continue;
      }
      offsets.push([dx, dz]);
    }
  }
  offsets.sort((left, right) => {
    const leftDistance = left[0] * left[0] + left[1] * left[1];
    const rightDistance = right[0] * right[0] + right[1] * right[1];
    return leftDistance - rightDistance || Math.abs(left[1]) - Math.abs(right[1]) || Math.abs(left[0]) - Math.abs(right[0]);
  });
  PRIORITIZED_COLUMN_OFFSETS.set(radiusChunks, offsets);
  return offsets;
}

function prioritizedChunkYRange(minCy: number, maxCy: number, preferredCy: number): number[] {
  if (maxCy < minCy) {
    return [];
  }
  const centerCy = Math.min(maxCy, Math.max(minCy, preferredCy));
  const ordered: number[] = [centerCy];
  for (let step = 1; centerCy - step >= minCy || centerCy + step <= maxCy; step += 1) {
    const upper = centerCy + step;
    if (upper <= maxCy) {
      ordered.push(upper);
    }
    const lower = centerCy - step;
    if (lower >= minCy) {
      ordered.push(lower);
    }
  }
  return ordered;
}

function toChunkKey(cx: number, cy: number, cz: number): string {
  return `${cx}:${cy}:${cz}`;
}

function toColumnKey(cx: number, cz: number): string {
  return `${cx}:${cz}`;
}

function intersectsColumnKeySet(
  columnKeys: ReadonlySet<string>,
  minX: number,
  maxXExclusive: number,
  minZ: number,
  maxZExclusive: number,
  chunkSize: number,
): boolean {
  const minChunkX = Math.floor(minX / chunkSize);
  const maxChunkX = Math.floor((maxXExclusive - 1) / chunkSize);
  const minChunkZ = Math.floor(minZ / chunkSize);
  const maxChunkZ = Math.floor((maxZExclusive - 1) / chunkSize);
  for (let cz = minChunkZ; cz <= maxChunkZ; cz += 1) {
    for (let cx = minChunkX; cx <= maxChunkX; cx += 1) {
      if (columnKeys.has(toColumnKey(cx, cz))) {
        return true;
      }
    }
  }
  return false;
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
