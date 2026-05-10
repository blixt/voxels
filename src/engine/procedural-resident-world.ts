import type { ChunkCoordinate, ChunkMeshData, Vec3, WorldStats } from "./types.ts";
import type {
  AsyncChunkGenerationQueue,
  AsyncDerivedLodChunkCacheKey,
  AsyncEncodedDerivedLodChunk,
} from "./async-chunk-generation.ts";
import { toAsyncLodChunkKey } from "./async-chunk-generation.ts";
import { decodeDerivedLodChunk, encodeDerivedLodChunk } from "./derived-lod-chunk-codec.ts";
import {
  ProceduralWorldGenerator,
  isProceduralWaterMaterial,
  type GeneratedChunk,
} from "./procedural-generator.ts";
import {
  buildOpaqueChunkMeshFromInput,
  createMeshMaterialLut,
  type MeshMaterialLut,
  type OpaqueChunkNeighborFaceSnapshot,
} from "./opaque-chunk-mesher.ts";
import {
  NO_GENERATED_SURFACE_HEIGHT,
  NO_GENERATED_WATER_HEIGHT,
  summarizeGeneratedChunkRender,
  type GeneratedChunkRenderSummary,
} from "./generated-chunk-render-summary.ts";
import {
  summarizeGeneratedRenderColumn,
  type GeneratedRenderColumnSummary,
} from "./generated-render-column-summary.ts";
import {
  GENERATED_RENDER_SUMMARY_REGION_SIZE_CHUNKS,
  getGeneratedRenderSummaryRegionCoord,
  type GeneratedRenderSummaryRegion,
} from "./generated-render-summary-region.ts";
import { deriveLodChunkData } from "./lod-chunk-derivation.ts";
import { FOG_END_DISTANCE } from "./render-constants.ts";
import { metersToWorldUnits } from "./scale.ts";
import { setChunkMeshDirtyState, type MutableResidentChunkWorld, type VoxelChunk } from "./world.ts";

const DEFAULT_HORIZONTAL_RADIUS_CHUNKS = 9;
const DEFAULT_UNDERGROUND_PADDING_CHUNKS = 3;
const DEFAULT_AIR_PADDING_CHUNKS = 2;
const LOD_VERTEX_STRIDE = 20;
const LOD_NORMAL_SCALE = 127;
const MAX_RETAINED_LOD_CHUNKS = 2048;
const MAX_RETAINED_EMPTY_LOD_KEYS = 8192;
const MAX_LOD_CACHE_REQUESTS_PER_UPDATE = 64;
const MAX_LOD_WORKER_GENERATE_REQUESTS_PER_UPDATE = 16;
const MAX_LOD_CACHE_STORES_PER_UPDATE = 4;
const LOD_FOG_COVERAGE_RADIUS = FOG_END_DISTANCE + metersToWorldUnits(32);

// LOD rings must cover the full fog distance (4800 world units).
// Each ring's outer edge in world units = radiusChunks * chunkSize * stride.
// LOD 1: 8 * 32 * 2  =   512   LOD 2: 8 * 32 * 4  = 1024
// LOD 3: 6 * 32 * 8  = 1536   LOD 4: 10 * 32 * 16 = 5120 (covers fog end)
const LOD_RINGS = [
  { level: 1, radiusChunks: 8 },
  { level: 2, radiusChunks: 8 },
  { level: 3, radiusChunks: 6 },
  { level: 4, radiusChunks: 10 },
] as const;
const MAX_LOD_LEVEL = LOD_RINGS[LOD_RINGS.length - 1]!.level;
const SPAWN_FOOTPRINT_RADIUS = metersToWorldUnits(0.8);
const SPAWN_SCAN_DEPTH = metersToWorldUnits(3.2);
const SPAWN_MAX_SURFACE_DROP = metersToWorldUnits(1.2);
const SPAWN_HEADROOM = metersToWorldUnits(1.8);
const EXPLORATION_START_CANDIDATES: readonly Vec3[] = [
  [metersToWorldUnits(236), 0, metersToWorldUnits(-4624)],
  [metersToWorldUnits(-1387.2), 0, metersToWorldUnits(-2468.8)],
  [metersToWorldUnits(-1427.2), 0, metersToWorldUnits(-2408.8)],
  [metersToWorldUnits(66.4), 0, metersToWorldUnits(-1997.6)],
];

export interface ResidencyPhaseMetrics {
  surfaceSampleMs: number;
  yRangeMs: number;
  chunkGenerationMs: number;
  chunkDispatchMs: number;
  chunkDrainMs: number;
  summaryDrainMs: number;
  chunkAdoptionMs: number;
  evictionMs: number;
  neighborDirtyMs: number;
  inFlightChunks: number;
  completedChunkCacheHits: number;
  completedGeneratedChunks: number;
  completedSummaryCacheHits: number;
  completedGeneratedSummaries: number;
  completedRegionSummaryCacheHits: number;
  missingRegionSummaries: number;
  readyGeneratedChunkBacklog: number;
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

export interface LodResidencyUpdateSummary {
  generated: number;
  generatedByLevel: readonly number[];
  cacheHits: number;
  cacheHitsByLevel: readonly number[];
  emptyCacheHits: number;
  emptyCacheHitsByLevel: readonly number[];
  pending: number;
  pendingPlanning: number;
  pendingDiskCache: number;
  pendingDiskCacheByLevel: readonly number[];
  pendingGenerationBudget: number;
  pendingGenerationBudgetByLevel: readonly number[];
  pendingPartialBuild: number;
  pendingPartialBuildByLevel: readonly number[];
  pendingPrepared: number;
  pendingPreparedByLevel: readonly number[];
  pendingInvalidatedEviction: number;
  totalChunks: number;
  totalChunksByLevel: readonly number[];
  cachedChunks: number;
  cachedEmptyKeys: number;
  elapsedMs: number;
  yRangeMs: number;
  downsampleMs: number;
  meshMs: number;
  commitMs: number;
  maxChunkMs: number;
  maxChunkLevel: number;
  maxChunkKey: string | null;
  neededKeyCount: number;
  neededKeyCacheHit: boolean;
  scheduledRegionSummaryRequests: number;
  lodDiskCacheHits: number;
  lodDiskCacheHitsByLevel: readonly number[];
  lodDiskCacheMisses: number;
  lodWorkerGenerated: number;
  lodWorkerGeneratedByLevel: readonly number[];
  scheduledLodWorkerRequests: number;
  scheduledLodDiskRequests: number;
  scheduledLodDiskStores: number;
  completedLodDiskStores: number;
}

export interface ResidencyUpdateOptions {
  maxGenerateChunks?: number;
  maxEvictChunks?: number;
  maxPlanMs?: number;
}

interface LodNeededKeyPlanningState {
  signature: string;
  neededKeys: Set<string>;
  ringIndex: number;
  dx: number;
  dz: number;
}

export interface WorldEditRecord {
  sequence: number;
  x: number;
  y: number;
  z: number;
  material: number;
  previousMaterial: number;
  chunk: ChunkCoordinate;
  localIndex: number;
}

export interface LodChunkDebugState {
  key: string;
  active: boolean;
  stale: boolean;
  coveragePunched: boolean;
  prepared: boolean;
  retained: boolean;
  retainedEmpty: boolean;
  empty: boolean;
  coveredEmpty: boolean;
}

export interface LodVisibleColumnState {
  covered: boolean;
  water: boolean;
  minY: number | null;
  maxY: number | null;
}

interface ChunkYRange {
  minCy: number;
  maxCy: number;
}

interface PartialLodChunkBuild {
  key: string;
  cx: number;
  cy: number;
  cz: number;
  level: number;
  stride: number;
  worldSize: number;
  data: Uint16Array;
  solidCount: number;
  minX: number;
  minY: number;
  minZ: number;
  maxX: number;
  maxY: number;
  maxZ: number;
  sourceComplete: boolean;
  skippedFinerCoverage: boolean;
  nextColumn: number;
}

export class ProceduralResidentWorld implements MutableResidentChunkWorld {
  readonly chunkSize: number;
  readonly minY = 0;
  readonly maxYExclusive: number;
  readonly palette: number[];

  horizontalRadiusChunks: number;
  undergroundPaddingChunks: number;
  airPaddingChunks: number;
  lastResidency: ResidencyUpdateSummary;

  private readonly chunks = new Map<string, VoxelChunk>();
  private readonly lodChunks = new Map<string, VoxelChunk>();
  private readonly preparedLodChunks = new Map<string, VoxelChunk>();
  private readonly retainedLodChunks = new Map<string, VoxelChunk>();
  private readonly staleLodKeys = new Set<string>();
  private readonly emptyLodKeys = new Set<string>();
  private readonly retainedEmptyLodKeys = new Set<string>();
  private readonly coveredEmptyLodKeys = new Set<string>();
  private readonly coveragePunchedLodKeys = new Set<string>();
  private readonly lodRenderClipMasks = new Map<string, Uint8Array>();
  private readonly missingLodDiskCacheKeys = new Set<string>();
  private readonly queuedLodDiskStoreKeys = new Set<string>();
  private readonly lodDiskStoreQueue: string[] = [];
  private readonly pendingLodNeighborRemeshKeys = new Set<string>();
  private readonly missingGeneratedRenderSummaryRegionKeys = new Set<string>();
  private coveredEmptyLodSignature: string | null = null;
  private activeLodBuildJob: PartialLodChunkBuild | null = null;
  private meshMaterialLut: MeshMaterialLut | null = null;
  private readonly emptyChunkKeys = new Set<string>();
  private readonly editOverlays = new Map<string, Map<number, number>>();
  private readonly editLog: WorldEditRecord[] = [];
  private readonly residentColumnCounts = new Map<string, number>();
  private readonly renderReadyColumnCounts = new Map<string, number>();
  private readonly renderReadyColumnKeys = new Set<string>();
  private readonly generatedRenderSummaries = new Map<string, GeneratedChunkRenderSummary>();
  private readonly generatedRenderChunkKeysByColumn = new Map<string, Set<string>>();
  private readonly generatedRenderColumnSummaries = new Map<string, GeneratedRenderColumnSummary>();
  private readonly columnChunkYRanges = new Map<string, ChunkYRange>();
  private readonly dirtyChunkKeys = new Set<string>();
  private readonly readyGeneratedChunks = new Map<string, GeneratedChunk>();
  private readonly asyncChunkGeneration: AsyncChunkGenerationQueue | null;
  private cachedSpawnPosition: Vec3 | null = null;
  private residentColumnRevision = 0;
  private renderReadyColumnRevision = 0;
  private lodNeededKeyCache: { signature: string; keys: string[] } | null = null;
  private lodNeededKeyPlanningState: LodNeededKeyPlanningState | null = null;
  private readonly lodWorldColumnCoverageCache = new Map<string, boolean>();
  private lastAnchorSignature = "";
  private lastAnchorComplete = true;
  private nextEditSequence = 1;
  constructor(
    readonly generator: ProceduralWorldGenerator,
    options: {
      horizontalRadiusChunks?: number;
      undergroundPaddingChunks?: number;
      airPaddingChunks?: number;
      asyncChunkGeneration?: AsyncChunkGenerationQueue | null;
    } = {},
  ) {
    this.chunkSize = generator.chunkSize;
    this.maxYExclusive = generator.maxYExclusive;
    this.palette = generator.palette;
    this.horizontalRadiusChunks = options.horizontalRadiusChunks ?? DEFAULT_HORIZONTAL_RADIUS_CHUNKS;
    this.undergroundPaddingChunks = options.undergroundPaddingChunks ?? DEFAULT_UNDERGROUND_PADDING_CHUNKS;
    this.airPaddingChunks = options.airPaddingChunks ?? DEFAULT_AIR_PADDING_CHUNKS;
    this.asyncChunkGeneration = options.asyncChunkGeneration ?? null;
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
    return this.dirtyChunkKeys.size;
  }

  hasResidentColumn(cx: number, cz: number): boolean {
    return (this.residentColumnCounts.get(toColumnKey(cx, cz)) ?? 0) > 0;
  }

  isColumnRenderReady(cx: number, cz: number): boolean {
    return this.renderReadyColumnKeys.has(toColumnKey(cx, cz));
  }

  getPaletteColor(materialIndex: number): number {
    return this.palette[materialIndex] ?? 0;
  }

  isCollisionMaterial(materialIndex: number): boolean {
    return materialIndex !== 0 && !isProceduralWaterMaterial(materialIndex);
  }

  isWaterMaterial(materialIndex: number): boolean {
    return isProceduralWaterMaterial(materialIndex);
  }

  getVoxel(x: number, y: number, z: number): number {
    if (y < this.minY || y >= this.maxYExclusive) {
      return 0;
    }
    const cx = Math.floor(x / this.chunkSize);
    const cy = Math.floor(y / this.chunkSize);
    const cz = Math.floor(z / this.chunkSize);
    const localIndex = toLocalVoxelIndex(x, y, z, cx, cy, cz, this.chunkSize);
    const overlay = this.editOverlays.get(toChunkKey(cx, cy, cz));
    if (overlay?.has(localIndex)) {
      return overlay.get(localIndex) ?? 0;
    }
    const chunk = this.getResidentChunk(cx, cy, cz);
    if (!chunk) {
      return 0;
    }
    return chunk.data[localIndex] ?? 0;
  }

  setVoxel(x: number, y: number, z: number, materialIndex: number): boolean {
    if (y < this.minY || y >= this.maxYExclusive) {
      return false;
    }
    const cx = Math.floor(x / this.chunkSize);
    const cy = Math.floor(y / this.chunkSize);
    const cz = Math.floor(z / this.chunkSize);
    const key = toChunkKey(cx, cy, cz);
    const lx = x - cx * this.chunkSize;
    const ly = y - cy * this.chunkSize;
    const lz = z - cz * this.chunkSize;
    const localIndex = lx + ly * this.chunkSize + lz * this.chunkSize * this.chunkSize;
    const previousMaterial = this.getVoxel(x, y, z);
    if (previousMaterial === materialIndex) {
      return false;
    }

    const overlay = this.editOverlays.get(key) ?? new Map<number, number>();
    const baseMaterial = this.generator.sampleMaterial(x, y, z);
    if (materialIndex === baseMaterial) {
      overlay.delete(localIndex);
    } else {
      overlay.set(localIndex, materialIndex);
    }
    if (overlay.size === 0) {
      this.editOverlays.delete(key);
    } else if (!this.editOverlays.has(key)) {
      this.editOverlays.set(key, overlay);
    }
    this.noteColumnChunkYRange(cx, cz, cy);
    this.emptyChunkKeys.delete(key);

    const residentChunk = this.getResidentChunk(cx, cy, cz);
    if (residentChunk) {
      updateResidentChunkVoxel(residentChunk, localIndex, lx, ly, lz, materialIndex);
      markResidentChunkDirty(this, residentChunk);
      this.recordResidentChunkRenderSummary(residentChunk);
      if (lx === 0) {
        this.markChunkDirtyByCoord(cx - 1, cy, cz);
      }
      if (ly === 0) {
        this.markChunkDirtyByCoord(cx, cy - 1, cz);
      }
      if (lz === 0) {
        this.markChunkDirtyByCoord(cx, cy, cz - 1);
      }
      if (lx === this.chunkSize - 1) {
        this.markChunkDirtyByCoord(cx + 1, cy, cz);
      }
      if (ly === this.chunkSize - 1) {
        this.markChunkDirtyByCoord(cx, cy + 1, cz);
      }
      if (lz === this.chunkSize - 1) {
        this.markChunkDirtyByCoord(cx, cy, cz + 1);
      }
    }

    this.editLog.push({
      sequence: this.nextEditSequence++,
      x,
      y,
      z,
      material: materialIndex,
      previousMaterial,
      chunk: { x: cx, y: cy, z: cz },
      localIndex,
    });
    // Invalidate LOD chunks covering this world position
    this.clearRetainedLodCache();
    this.invalidateLodChunksAt(x, y, z);
    return true;
  }

  getEditLogSnapshot(): WorldEditRecord[] {
    return this.editLog.map((record) => ({
      ...record,
      chunk: { ...record.chunk },
    }));
  }

  getResidentChunk(cx: number, cy: number, cz: number): VoxelChunk | null {
    return this.chunks.get(toChunkKey(cx, cy, cz)) ?? null;
  }

  hasResidentChunk(cx: number, cy: number, cz: number): boolean {
    return this.chunks.has(toChunkKey(cx, cy, cz));
  }

  getLodChunkDebugState(chunk: VoxelChunk): LodChunkDebugState {
    const key = toLodChunkKey(chunk.lodLevel, chunk.coord.x, chunk.coord.y, chunk.coord.z);
    return {
      key,
      active: this.lodChunks.get(key) === chunk,
      stale: this.staleLodKeys.has(key),
      coveragePunched: this.coveragePunchedLodKeys.has(key),
      prepared: this.preparedLodChunks.has(key),
      retained: this.retainedLodChunks.has(key),
      retainedEmpty: this.retainedEmptyLodKeys.has(key),
      empty: this.emptyLodKeys.has(key),
      coveredEmpty: this.coveredEmptyLodKeys.has(key),
    };
  }

  classifyVisibleLodColumn(chunk: VoxelChunk, worldX: number, worldZ: number): LodVisibleColumnState {
    const stride = Math.max(1, chunk.voxelStride);
    const worldSize = this.chunkSize * stride;
    const localX = Math.floor((worldX - chunk.coord.x * worldSize) / stride);
    const localZ = Math.floor((worldZ - chunk.coord.z * worldSize) / stride);
    if (localX < 0 || localX >= this.chunkSize || localZ < 0 || localZ >= this.chunkSize) {
      return { covered: false, water: false, minY: null, maxY: null };
    }

    const chunkArea = this.chunkSize * this.chunkSize;
    const columnOffset = localX + localZ * chunkArea;
    let covered = false;
    let water = false;
    let minY: number | null = null;
    let maxY: number | null = null;
    for (let localY = 0; localY < this.chunkSize; localY += 1) {
      const index = columnOffset + localY * this.chunkSize;
      const material = this.isLodChunkLocalVoxelVisible(chunk, index) ? chunk.data[index]! : 0;
      if (material === 0) {
        continue;
      }
      covered = true;
      const worldMinY = chunk.coord.y * worldSize + localY * stride;
      const worldMaxY = worldMinY + stride;
      minY = minY === null ? worldMinY : Math.min(minY, worldMinY);
      maxY = maxY === null ? worldMaxY : Math.max(maxY, worldMaxY);
      if (isProceduralWaterMaterial(material)) {
        const aboveIndex = index + this.chunkSize;
        const above = localY + 1 < this.chunkSize
          ? this.isLodChunkLocalVoxelVisible(chunk, aboveIndex)
            ? chunk.data[aboveIndex]!
            : 0
          : material;
        water ||= !isProceduralWaterMaterial(above);
      }
    }
    return { covered, water, minY, maxY };
  }

  *iterateResidentChunks(): Iterable<VoxelChunk> {
    // Yield LOD chunks coarsest-first so depth bias resolves correctly:
    // LOD 4 (most biased) draws first, then LOD 3, 2, 1, then LOD 0 (no bias).
    // Finer levels always win via lower depth bias.
    for (let level = 4; level >= 1; level--) {
      for (const chunk of this.lodChunks.values()) {
        if (chunk.lodLevel === level) yield chunk;
      }
    }
    for (const chunk of this.chunks.values()) {
      yield chunk;
    }
  }

  *iterateDirtyResidentChunks(): Iterable<VoxelChunk> {
    for (const key of this.dirtyChunkKeys) {
      const chunk = this.chunks.get(key);
      if (chunk) {
        yield chunk;
      }
    }
  }

  noteResidentChunkMeshDirtyState(chunk: VoxelChunk, dirty: boolean): void {
    const key = toChunkKey(chunk.coord.x, chunk.coord.y, chunk.coord.z);
    if (dirty) {
      this.dirtyChunkKeys.add(key);
      return;
    }
    this.dirtyChunkKeys.delete(key);
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
    if (this.cachedSpawnPosition) {
      return [...this.cachedSpawnPosition];
    }
    for (const candidatePosition of EXPLORATION_START_CANDIDATES) {
      const candidate = this.sampleSpawnCandidate(candidatePosition[0], candidatePosition[2]);
      if (
        candidate.unsupportedSamples === 0
        && candidate.surfaceSpread <= 12
        && candidate.column.surfaceY >= this.generator.seaLevel - 96
      ) {
        this.cachedSpawnPosition = [candidatePosition[0] + 0.5, candidate.standY, candidatePosition[2] + 0.5];
        return [...this.cachedSpawnPosition];
      }
    }
    const preferredBiomes = new Set(["verdant", "steppe", "highland", "bloom"]);
    let fallback = this.sampleSpawnCandidate(0, 0);
    let fallbackPosition: Vec3 = [0.5, fallback.standY, 0.5];
    for (let ring = 0; ring <= 24; ring += 1) {
      for (const [offsetX, offsetZ] of spawnSearchOffsets(ring)) {
        const worldX = offsetX * this.chunkSize * 2;
        const worldZ = offsetZ * this.chunkSize * 2;
        const candidate = this.sampleSpawnCandidate(worldX, worldZ);
        if (
          preferredBiomes.has(candidate.column.biomeId)
          && candidate.unsupportedSamples === 0
          && candidate.column.surfaceY >= this.generator.seaLevel - 48
          && candidate.column.surfaceY <= this.generator.seaLevel + 320
          && candidate.surfaceSpread <= 12
        ) {
          this.cachedSpawnPosition = [worldX + 0.5, candidate.standY, worldZ + 0.5];
          return [...this.cachedSpawnPosition];
        }
        if (
          candidate.unsupportedSamples < fallback.unsupportedSamples
          || (
            candidate.unsupportedSamples === fallback.unsupportedSamples
            && candidate.surfaceSpread < fallback.surfaceSpread
          )
          || (
            candidate.unsupportedSamples === fallback.unsupportedSamples
            && candidate.surfaceSpread === fallback.surfaceSpread
            && Math.abs(candidate.column.surfaceY - this.generator.seaLevel)
              < Math.abs(fallback.column.surfaceY - this.generator.seaLevel)
          )
        ) {
          fallback = candidate;
          fallbackPosition = [worldX + 0.5, candidate.standY, worldZ + 0.5];
        }
      }
    }
    this.cachedSpawnPosition = fallbackPosition;
    return [...this.cachedSpawnPosition];
  }

  private sampleSpawnCandidate(worldX: number, worldZ: number): {
    column: ReturnType<ProceduralWorldGenerator["sampleColumn"]>;
    standY: number;
    surfaceSpread: number;
    unsupportedSamples: number;
  } {
    const column = this.generator.sampleColumn(worldX, worldZ);
    const standableSurfaceY = this.findSpawnStandableSurfaceY(worldX, worldZ, column);
    let minStandableY = standableSurfaceY ?? column.surfaceY;
    let maxStandableY = standableSurfaceY ?? column.surfaceY;
    let unsupportedSamples = standableSurfaceY === null ? 1 : 0;
    for (const [offsetX, offsetZ] of spawnFootprintOffsets()) {
      const sampledColumn = this.generator.sampleColumn(worldX + offsetX, worldZ + offsetZ);
      const sampledStandableY = this.findSpawnStandableSurfaceY(worldX + offsetX, worldZ + offsetZ, sampledColumn);
      if (sampledStandableY === null) {
        unsupportedSamples += 1;
        continue;
      }
      minStandableY = Math.min(minStandableY, sampledStandableY);
      maxStandableY = Math.max(maxStandableY, sampledStandableY);
    }
    return {
      column,
      standY: (unsupportedSamples === 0 ? maxStandableY : (standableSurfaceY ?? column.surfaceY)) + 1,
      surfaceSpread: unsupportedSamples === 0 ? maxStandableY - minStandableY : Number.POSITIVE_INFINITY,
      unsupportedSamples,
    };
  }

  private findSpawnStandableSurfaceY(
    worldX: number,
    worldZ: number,
    column: Pick<ReturnType<ProceduralWorldGenerator["sampleColumn"]>, "surfaceY">,
  ): number | null {
    const minWorldY = Math.max(this.minY, column.surfaceY - SPAWN_SCAN_DEPTH);
    for (let worldY = column.surfaceY; worldY >= minWorldY; worldY -= 1) {
      if (!this.isCollisionMaterial(this.generator.sampleMaterial(worldX, worldY, worldZ))) {
        continue;
      }
      if (worldY < column.surfaceY - SPAWN_MAX_SURFACE_DROP) {
        return null;
      }
      if (this.generator.sampleMaterial(worldX, worldY + 1, worldZ) !== 0) {
        continue;
      }
      if (this.generator.sampleMaterial(worldX, worldY + SPAWN_HEADROOM - 1, worldZ) !== 0) {
        continue;
      }
      return worldY;
    }
    return null;
  }

  updateResidencyAround(
    position: Vec3,
    options: ResidencyUpdateOptions = {},
  ): ResidencyUpdateSummary {
    const centerChunkX = Math.floor(position[0] / this.chunkSize);
    const centerChunkZ = Math.floor(position[2] / this.chunkSize);
    const radiusChunks = this.horizontalRadiusChunks;
    const anchorSignature = `${centerChunkX}:${centerChunkZ}:${radiusChunks}`;
    const maxGenerateChunks = options.maxGenerateChunks ?? Number.POSITIVE_INFINITY;
    const maxEvictChunks = options.maxEvictChunks ?? Number.POSITIVE_INFINITY;
    const maxPlanMs = options.maxPlanMs ?? Number.POSITIVE_INFINITY;
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
        dirtyResidentChunks: this.dirtyChunkKeys.size,
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
    let chunkDispatchMs = 0;
    let chunkDrainMs = 0;
    let summaryDrainMs = 0;
    let chunkAdoptionMs = 0;
    let evictionMs = 0;
    let neighborDirtyMs = 0;
    let inFlightChunks = this.asyncChunkGeneration?.getPendingCount() ?? 0;
    let planBudgetExhausted = false;
    const hasPlanBudgetExpired = (): boolean => {
      if (!Number.isFinite(maxPlanMs) || performance.now() - startedAt < maxPlanMs) {
        return false;
      }
      planBudgetExhausted = true;
      return true;
    };
    const drainStartedAt = performance.now();
    const completedGeneratedChunks = this.asyncChunkGeneration?.drainCompletedChunks() ?? [];
    const completedGenerationStats = this.asyncChunkGeneration?.drainCompletionStats() ?? { cacheHits: 0, generated: 0 };
    chunkDrainMs = performance.now() - drainStartedAt;
    const summaryDrainStartedAt = performance.now();
    const completedRenderSummaries = this.asyncChunkGeneration?.drainCompletedSummaries() ?? [];
    const completedSummaryStats = this.asyncChunkGeneration?.drainSummaryCompletionStats() ?? { cacheHits: 0, generated: 0 };
    summaryDrainMs = performance.now() - summaryDrainStartedAt;
    const completedRegionSummaries = this.asyncChunkGeneration?.drainCompletedRegionSummaries() ?? [];
    const missingRegionSummaries = this.asyncChunkGeneration?.drainMissingRegionSummaries() ?? [];
    for (const missing of missingRegionSummaries) {
      this.missingGeneratedRenderSummaryRegionKeys.add(toRegionSummaryKey(missing.x, missing.z));
    }
    for (const summary of completedRegionSummaries) {
      this.recordPersistedRegionRenderSummary(summary);
    }
    for (const summary of completedRenderSummaries) {
      this.recordChunkRenderSummary(toChunkKey(summary.coord.x, summary.coord.y, summary.coord.z), summary);
    }
    for (const generated of completedGeneratedChunks) {
      const key = toChunkKey(generated.coord.x, generated.coord.y, generated.coord.z);
      this.applyOverlayToGeneratedChunk(key, generated);
      this.recordGeneratedChunkRenderSummary(key, generated);
      this.readyGeneratedChunks.set(key, generated);
    }
    let scheduledChunks = 0;

    for (const [dx, dz] of prioritizedColumnOffsets(radiusChunks)) {
      if (hasPlanBudgetExpired()) {
        break;
      }
      const cx = centerChunkX + dx;
      const cz = centerChunkZ + dz;
      const yRangeStartedAt = performance.now();
      const [minCy, maxCy] = this.computeChunkYRange(cx, cz);
      yRangeMs += performance.now() - yRangeStartedAt;
      const preferredCy = dx === 0 && dz === 0
        ? Math.floor(surfaceY / this.chunkSize)
        : Math.floor((minCy + maxCy) * 0.5);
      for (const cy of prioritizedChunkYRange(minCy, maxCy, preferredCy)) {
        if (hasPlanBudgetExpired()) {
          break;
        }
        const key = toChunkKey(cx, cy, cz);
        neededKeys.add(key);
        if (this.chunks.has(key)) {
          continue;
        }
        const readyGenerated = this.readyGeneratedChunks.get(key);
        if (readyGenerated) {
          if (generatedChunks >= maxGenerateChunks) {
            pendingChunks += 1;
            continue;
          }
          this.readyGeneratedChunks.delete(key);
          if (readyGenerated.solidCount === 0) {
            emptyChunksSkipped += 1;
            this.emptyChunkKeys.add(key);
            continue;
          }
          const adoptionStartedAt = performance.now();
          const chunk = createResidentChunk(readyGenerated, this.chunkSize);
          this.emptyChunkKeys.delete(key);
          this.adoptResidentChunk(key, chunk);
          generatedChunks += 1;
          generatedChunkCoords.push({ x: cx, y: cy, z: cz });
          chunkAdoptionMs += performance.now() - adoptionStartedAt;
          const dirtyStartedAt = performance.now();
          touchedNeighborChunks += this.markAdjacentChunksDirty(cx, cy, cz);
          neighborDirtyMs += performance.now() - dirtyStartedAt;
          continue;
        }
        if (this.emptyChunkKeys.has(key)) {
          cachedEmptyChunkHits += 1;
          continue;
        }
        if (this.asyncChunkGeneration) {
          if (this.asyncChunkGeneration.hasPendingChunk(cx, cy, cz)) {
            pendingChunks += 1;
            continue;
          }
          if (scheduledChunks >= maxGenerateChunks) {
            pendingChunks += 1;
            continue;
          }
          const dispatchStartedAt = performance.now();
          const requested = this.asyncChunkGeneration.requestChunk(cx, cy, cz);
          chunkDispatchMs += performance.now() - dispatchStartedAt;
          if (requested) {
            scheduledChunks += 1;
          }
          pendingChunks += 1;
          continue;
        }
        if (generatedChunks >= maxGenerateChunks) {
          pendingChunks += 1;
          continue;
        }
        const generationStartedAt = performance.now();
        const generated = this.generator.generateChunk(cx, cy, cz);
        this.applyOverlayToGeneratedChunk(key, generated);
        this.recordGeneratedChunkRenderSummary(key, generated);
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

    if (!planBudgetExhausted) {
      for (const [key, generated] of this.readyGeneratedChunks) {
        if (neededKeys.has(key)) {
          continue;
        }
        this.readyGeneratedChunks.delete(key);
        if (generated.solidCount === 0) {
          this.emptyChunkKeys.add(key);
        }
      }

      for (const [key, chunk] of [...this.chunks.entries()]) {
        if (neededKeys.has(key)) {
          continue;
        }
        if (evictedChunks >= maxEvictChunks) {
          pendingChunks = Math.max(pendingChunks, 1);
          break;
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
    } else {
      pendingChunks = Math.max(pendingChunks, 1, this.lastResidency.pendingChunks);
    }

    this.lastAnchorSignature = anchorSignature;
    this.lastAnchorComplete = pendingChunks === 0 && !planBudgetExhausted;
    inFlightChunks = this.asyncChunkGeneration?.getPendingCount() ?? 0;
    this.lastResidency = {
      changed: generatedChunks > 0 || evictedChunks > 0,
      complete: pendingChunks === 0 && !planBudgetExhausted,
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
      dirtyResidentChunks: this.dirtyChunkKeys.size,
      surfaceY,
      elapsedMs: performance.now() - startedAt,
      generatedChunkCoords,
      evictedChunkCoords,
      phaseMs: {
        surfaceSampleMs,
        yRangeMs,
        chunkGenerationMs,
        chunkDispatchMs,
        chunkDrainMs,
        summaryDrainMs,
        chunkAdoptionMs,
        evictionMs,
        neighborDirtyMs,
        inFlightChunks,
        completedChunkCacheHits: completedGenerationStats.cacheHits,
        completedGeneratedChunks: completedGenerationStats.generated,
        completedSummaryCacheHits: completedSummaryStats.cacheHits,
        completedGeneratedSummaries: completedSummaryStats.generated,
        completedRegionSummaryCacheHits: completedRegionSummaries.length,
        missingRegionSummaries: missingRegionSummaries.length,
        readyGeneratedChunkBacklog: this.readyGeneratedChunks.size,
      },
    };
    return this.lastResidency;
  }

  updateLodResidencyAround(
    position: Vec3,
    options?: {
      maxGenerateLodChunks?: number;
      maxAdoptCompletedLodChunks?: number;
      maxPlanMs?: number;
      maxWorkMs?: number;
    },
  ): LodResidencyUpdateSummary {
    const startedAt = performance.now();
    const maxGenerate = options?.maxGenerateLodChunks ?? 32;
    const maxAdoptCompleted = options?.maxAdoptCompletedLodChunks
      ?? (options ? Math.max(1, maxGenerate) : Number.POSITIVE_INFINITY);
    const maxPlanMs = options?.maxPlanMs ?? Number.POSITIVE_INFINITY;
    const maxWorkMs = options?.maxWorkMs ?? Number.POSITIVE_INFINITY;

    if (!this.meshMaterialLut) {
      this.meshMaterialLut = createMeshMaterialLut(this.palette, isProceduralWaterMaterial);
    }
    this.lodWorldColumnCoverageCache.clear();

    // Phase 1: compute all needed LOD chunk keys.
    //
    // LOD 1 chunks are excluded only when their footprint is actually covered
    // by render-ready LOD 0 columns. Coarser levels are planned conservatively;
    // the downsample step punches out finer coverage per column.
    const neededKeySignature = this.createLodNeededKeySignature(position);
    if (this.coveredEmptyLodSignature !== neededKeySignature) {
      this.coveredEmptyLodKeys.clear();
      this.coveredEmptyLodSignature = neededKeySignature;
    }
    const cachedNeededKeys = this.lodNeededKeyCache?.signature === neededKeySignature
      ? this.lodNeededKeyCache.keys
      : null;
    const neededKeys = new Set<string>();
    let neededKeyCacheHit = cachedNeededKeys !== null;
    let yRangeMs = 0;
    let scheduledRegionSummaryRequests = 0;

    if (cachedNeededKeys) {
      for (const key of cachedNeededKeys) {
        neededKeys.add(key);
      }
    } else {
      const planning = this.planLodNeededKeys(position, neededKeySignature, maxPlanMs);
      yRangeMs = planning.yRangeMs;
      scheduledRegionSummaryRequests = planning.scheduledRegionSummaryRequests;
      if (!planning.complete) {
        return {
          generated: 0,
          generatedByLevel: createLodLevelCounts(),
          pending: 1,
          pendingPlanning: 1,
          pendingDiskCache: 0,
          pendingDiskCacheByLevel: createLodLevelCounts(),
          pendingGenerationBudget: 0,
          pendingGenerationBudgetByLevel: createLodLevelCounts(),
          pendingPartialBuild: 0,
          pendingPartialBuildByLevel: createLodLevelCounts(),
          pendingPrepared: 0,
          pendingPreparedByLevel: createLodLevelCounts(),
          pendingInvalidatedEviction: 0,
          totalChunks: this.lodChunks.size,
          totalChunksByLevel: countLodChunksByLevel(this.lodChunks.values()),
          cachedChunks: this.retainedLodChunks.size,
          cachedEmptyKeys: this.retainedEmptyLodKeys.size,
          elapsedMs: performance.now() - startedAt,
          yRangeMs,
          downsampleMs: 0,
          meshMs: 0,
          commitMs: 0,
          maxChunkMs: 0,
          maxChunkLevel: 0,
          maxChunkKey: null,
          neededKeyCount: planning.neededKeyCount,
          neededKeyCacheHit,
          cacheHits: 0,
          cacheHitsByLevel: createLodLevelCounts(),
          emptyCacheHits: 0,
          emptyCacheHitsByLevel: createLodLevelCounts(),
          scheduledRegionSummaryRequests,
          lodDiskCacheHits: 0,
          lodDiskCacheHitsByLevel: createLodLevelCounts(),
          lodDiskCacheMisses: 0,
          lodWorkerGenerated: 0,
          lodWorkerGeneratedByLevel: createLodLevelCounts(),
          scheduledLodWorkerRequests: 0,
          scheduledLodDiskRequests: 0,
          scheduledLodDiskStores: 0,
          completedLodDiskStores: 0,
        };
      }
      for (const key of planning.keys) {
        neededKeys.add(key);
      }
    }

    // Phase 2: downsample and mesh LOD chunks with a budget.
    // Process levels in ascending order so LOD N-1 is available when building LOD N.
    let generated = 0;
    const generatedByLevel = createLodLevelCounts();
    let cacheHits = 0;
    const cacheHitsByLevel = createLodLevelCounts();
    let emptyCacheHits = 0;
    const emptyCacheHitsByLevel = createLodLevelCounts();
    let pending = 0;
    let pendingDiskCache = 0;
    const pendingDiskCacheByLevel = createLodLevelCounts();
    let pendingGenerationBudget = 0;
    const pendingGenerationBudgetByLevel = createLodLevelCounts();
    let pendingPartialBuild = 0;
    const pendingPartialBuildByLevel = createLodLevelCounts();
    let pendingInvalidatedEviction = 0;
    let downsampleMs = 0;
    let meshMs = 0;
    let commitMs = 0;
    let lodDiskCacheHits = 0;
    const lodDiskCacheHitsByLevel = createLodLevelCounts();
    let lodWorkerGenerated = 0;
    const lodWorkerGeneratedByLevel = createLodLevelCounts();
    let scheduledLodDiskRequests = 0;
    let scheduledLodWorkerRequests = 0;
    let scheduledLodDiskStores = 0;
    const lodDiskStats = this.asyncChunkGeneration?.drainLodChunkCompletionStats() ?? { cacheHits: 0, generated: 0, missing: 0, stored: 0 };
    const completedLodChunks = this.asyncChunkGeneration?.drainCompletedLodChunks(maxAdoptCompleted) ?? [];
    const missingLodDiskChunks = this.asyncChunkGeneration?.drainMissingLodChunks() ?? [];
    for (const missing of missingLodDiskChunks) {
      this.missingLodDiskCacheKeys.add(toAsyncLodChunkKey(missing));
    }
    const adoptedLod = this.adoptCompletedLodChunks(completedLodChunks, neededKeys);
    lodDiskCacheHits += adoptedLod.cacheHits;
    lodWorkerGenerated += adoptedLod.workerGenerated;
    for (let index = 0; index < lodDiskCacheHitsByLevel.length; index += 1) {
      lodDiskCacheHitsByLevel[index] += adoptedLod.cacheHitsByLevel[index] ?? 0;
      lodWorkerGeneratedByLevel[index] += adoptedLod.workerGeneratedByLevel[index] ?? 0;
    }
    meshMs += adoptedLod.meshMs;
    const pendingCompletedLodAdoptions = this.asyncChunkGeneration?.getCompletedLodChunkCount?.() ?? 0;
    let maxChunkMs = 0;
    let maxChunkLevel = 0;
    let maxChunkKey: string | null = null;
    let workBudgetExhausted = false;
    if (this.activeLodBuildJob && !neededKeys.has(this.activeLodBuildJob.key)) {
      this.activeLodBuildJob = null;
    }
    for (const ring of LOD_RINGS) {
      const stride = 1 << ring.level;
      const worldSize = this.chunkSize * stride;
      const keyPrefix = `L${ring.level}:`;
      for (const key of neededKeys) {
        if (!key.startsWith(keyPrefix)) continue;
        const activeLodChunk = this.lodChunks.get(key);
        if (activeLodChunk && !this.staleLodKeys.has(key)) {
          this.punchActiveCoarserLodChunksCoveredBy(activeLodChunk, { includeAlreadyPunched: false });
          continue;
        }
        if (this.preparedLodChunks.has(key) || this.coveredEmptyLodKeys.has(key)) continue;
        if (this.emptyLodKeys.has(key)) continue;
        if (this.reviveRetainedEmptyLodKey(key)) {
          emptyCacheHits += 1;
          emptyCacheHitsByLevel[ring.level] += 1;
          continue;
        }
        if (this.reviveRetainedLodChunk(key)) {
          cacheHits += 1;
          cacheHitsByLevel[ring.level] += 1;
          continue;
        }
        const parts = key.split(":");
        const cx = parseInt(parts[1]!);
        const cy = parseInt(parts[2]!);
        const cz = parseInt(parts[3]!);
        const diskRequest = this.scheduleLodDiskChunkRequest(ring.level, cx, cy, cz, scheduledLodDiskRequests);
        if (diskRequest !== "skip") {
          if (diskRequest === "scheduled") {
            scheduledLodDiskRequests += 1;
          }
          pending++;
          pendingDiskCache++;
          pendingDiskCacheByLevel[ring.level] += 1;
          continue;
        }
        const workerRequest = this.scheduleGeneratedLodChunkRequest(ring.level, cx, cy, cz, scheduledLodWorkerRequests);
        if (workerRequest !== "skip") {
          if (workerRequest === "scheduled") {
            scheduledLodWorkerRequests += 1;
          }
          pending++;
          pendingGenerationBudget++;
          pendingGenerationBudgetByLevel[ring.level] += 1;
          continue;
        }
        if (
          workBudgetExhausted ||
          generated >= maxGenerate ||
          (generated > 0 && performance.now() - startedAt >= maxWorkMs)
        ) {
          pending++;
          pendingGenerationBudget++;
          pendingGenerationBudgetByLevel[ring.level] += 1;
          workBudgetExhausted = true;
          continue;
        }

        const chunkStartedAt = performance.now();
        const dsStartedAt = performance.now();
        const partialBuildAllowed = ring.level >= 3 && this.editOverlays.size === 0;
        let data: {
          data: Uint16Array;
          solidCount: number;
          solidBounds: { min: [number, number, number]; max: [number, number, number] } | null;
          sourceComplete: boolean;
          skippedFinerCoverage: boolean;
        } | null = null;
        if (partialBuildAllowed) {
          if (this.activeLodBuildJob && this.activeLodBuildJob.key !== key) {
            pending++;
            pendingGenerationBudget++;
            pendingGenerationBudgetByLevel[ring.level] += 1;
            workBudgetExhausted = true;
            continue;
          }
          const job = this.activeLodBuildJob ?? this.createPartialLodChunkBuild(key, cx, cy, cz, ring.level, stride, worldSize);
          this.activeLodBuildJob = job;
          const complete = this.processPartialLodChunkBuild(job, startedAt + maxWorkMs);
          downsampleMs += performance.now() - dsStartedAt;
          if (!complete) {
            const chunkMs = performance.now() - chunkStartedAt;
            if (chunkMs > maxChunkMs) {
              maxChunkMs = chunkMs;
              maxChunkLevel = ring.level;
              maxChunkKey = key;
            }
            pending++;
            pendingPartialBuild++;
            pendingPartialBuildByLevel[ring.level] += 1;
            workBudgetExhausted = true;
            continue;
          }
          data = this.finalizePartialLodChunkBuild(job);
          this.activeLodBuildJob = null;
        } else {
          data = this.downsampleLodChunkData(cx, cy, cz, ring.level, stride, worldSize);
          downsampleMs += performance.now() - dsStartedAt;
        }
        if (data.solidCount === 0) {
          if (data.skippedFinerCoverage) {
            this.coveredEmptyLodKeys.add(key);
          } else {
            this.emptyLodKeys.add(key);
          }
          this.lodChunks.delete(key);
          this.preparedLodChunks.delete(key);
          this.staleLodKeys.delete(key);
          this.retainedLodChunks.delete(key);
          this.retainedEmptyLodKeys.delete(key);
          this.clearLodCoveragePunchedKey(key);
          this.invalidateCoarserLodChunksForSourceChunk(ring.level, cx, cy, cz, {
            clearNeededKeyCache: false,
          });
          generated++;
          generatedByLevel[ring.level] += 1;
          const chunkMs = performance.now() - chunkStartedAt;
          if (chunkMs > maxChunkMs) {
            maxChunkMs = chunkMs;
            maxChunkLevel = ring.level;
            maxChunkKey = key;
          }
          continue;
        }
        const chunk = createLodResidentChunk(
          { coord: { x: cx, y: cy, z: cz }, data: data.data, solidCount: data.solidCount, solidBounds: data.solidBounds, renderSummary: null! },
          ring.level, stride,
        );
        const meshStartedAt = performance.now();
        const mesh = this.buildLodChunkMesh(chunk, cx, cy, cz, ring.level, stride, worldSize);
        meshMs += performance.now() - meshStartedAt;
        chunk.mesh = mesh;
        chunk.meshBuilt = true;
        chunk.meshDirty = false;
        chunk.renderReady = true;
        chunk.gpuDirty = true;
        this.retainedLodChunks.delete(key);
        this.retainedEmptyLodKeys.delete(key);
        if (data.skippedFinerCoverage) {
          this.coveragePunchedLodKeys.add(key);
        } else {
          this.clearLodCoveragePunchedKey(key);
        }
        if (this.lodChunkHasCoveredActiveCoarserOverlap(chunk)) {
          this.prepareLodChunkBlockedByActiveCoarser(key, chunk);
        } else {
          this.lodChunks.set(key, chunk);
          this.lodWorldColumnCoverageCache.clear();
          this.staleLodKeys.delete(key);
          this.punchActiveCoarserLodChunksCoveredBy(chunk);
        }
        if (!data.skippedFinerCoverage) {
          this.enqueueLodDiskStore(key);
        }
        this.queueAdjacentLodChunkRemeshes(ring.level, cx, cy, cz);
        this.invalidateCoarserLodChunksForSourceChunk(ring.level, cx, cy, cz, {
          clearNeededKeyCache: false,
        });
        generated++;
        generatedByLevel[ring.level] += 1;
        const chunkMs = performance.now() - chunkStartedAt;
        if (chunkMs > maxChunkMs) {
          maxChunkMs = chunkMs;
          maxChunkLevel = ring.level;
          maxChunkKey = key;
        }
        if (performance.now() - startedAt >= maxWorkMs) {
          workBudgetExhausted = true;
        }
      }
    }
    const commitStartedAt = performance.now();
    this.commitPreparedLodChunks();
    this.punchActiveCoarserLodChunksCoveredByPreparedActiveFiner();
    commitMs = performance.now() - commitStartedAt;

    const neighborRemeshStartedAt = performance.now();
    const neighborRemeshBudget = Number.isFinite(maxWorkMs)
      ? Math.max(0, startedAt + maxWorkMs - neighborRemeshStartedAt)
      : Number.POSITIVE_INFINITY;
    const neighborRemesh = this.processPendingLodNeighborRemeshes({
      maxMs: neighborRemeshBudget,
      maxCount: Number.isFinite(maxWorkMs) ? 1 : Number.POSITIVE_INFINITY,
    });
    meshMs += neighborRemesh.meshMs;

    // Phase 3: evict LOD chunks no longer needed.
    // Only evict when there's no pending work — otherwise old LOD chunks
    // provide coverage while new ones are still being generated.
    let invalidatedCoarserDuringEviction = false;
    if (pending === 0) {
      for (const [key, chunk] of [...this.lodChunks.entries()]) {
        if (!neededKeys.has(key)) {
          this.lodChunks.delete(key);
          this.staleLodKeys.delete(key);
          this.retainLodChunk(key, chunk);
          this.invalidateCoarserLodChunksForSourceChunk(chunk.lodLevel, chunk.coord.x, chunk.coord.y, chunk.coord.z, {
            clearNeededKeyCache: false,
          });
          invalidatedCoarserDuringEviction = true;
        }
      }
      for (const [key] of [...this.preparedLodChunks.entries()]) {
        if (!neededKeys.has(key)) {
          this.retainLodChunk(key, this.preparedLodChunks.get(key)!);
          this.preparedLodChunks.delete(key);
        }
      }
      for (const key of [...this.emptyLodKeys]) {
        if (!neededKeys.has(key)) {
          this.emptyLodKeys.delete(key);
          this.retainEmptyLodKey(key);
          const parts = parseLodKey(key);
          if (parts) {
            this.invalidateCoarserLodChunksForSourceChunk(parts.level, parts.cx, parts.cy, parts.cz, {
              clearNeededKeyCache: false,
            });
            invalidatedCoarserDuringEviction = true;
          }
        }
      }
      for (const key of [...this.coveredEmptyLodKeys]) {
        if (!neededKeys.has(key)) {
          this.coveredEmptyLodKeys.delete(key);
          const parts = parseLodKey(key);
          if (parts) {
            this.invalidateCoarserLodChunksForSourceChunk(parts.level, parts.cx, parts.cy, parts.cz, {
              clearNeededKeyCache: false,
            });
            invalidatedCoarserDuringEviction = true;
          }
        }
      }
    }
    if (invalidatedCoarserDuringEviction) {
      pending += 1;
      pendingInvalidatedEviction = 1;
    }
    scheduledLodDiskStores += this.flushQueuedLodDiskStores();
    const pendingPrepared = this.preparedLodChunks.size;
    const pendingPreparedByLevel = countLodChunksByLevel(this.preparedLodChunks.values());
    pending += pendingPrepared + pendingCompletedLodAdoptions + this.pendingLodNeighborRemeshKeys.size;
    if (!neededKeyCacheHit) {
      this.lodNeededKeyCache = {
        signature: neededKeySignature,
        keys: [...neededKeys],
      };
    }

    return {
      generated,
      generatedByLevel,
      cacheHits,
      cacheHitsByLevel,
      emptyCacheHits,
      emptyCacheHitsByLevel,
      pending,
      pendingPlanning: 0,
      pendingDiskCache,
      pendingDiskCacheByLevel,
      pendingGenerationBudget,
      pendingGenerationBudgetByLevel,
      pendingPartialBuild,
      pendingPartialBuildByLevel,
      pendingPrepared,
      pendingPreparedByLevel,
      pendingInvalidatedEviction,
      totalChunks: this.lodChunks.size,
      totalChunksByLevel: countLodChunksByLevel(this.lodChunks.values()),
      cachedChunks: this.retainedLodChunks.size,
      cachedEmptyKeys: this.retainedEmptyLodKeys.size,
      elapsedMs: performance.now() - startedAt,
      yRangeMs,
      downsampleMs,
      meshMs,
      commitMs,
      maxChunkMs,
      maxChunkLevel,
      maxChunkKey,
      neededKeyCount: neededKeys.size,
      neededKeyCacheHit,
      scheduledRegionSummaryRequests,
      lodDiskCacheHits,
      lodDiskCacheHitsByLevel,
      lodDiskCacheMisses: lodDiskStats.missing,
      lodWorkerGenerated,
      lodWorkerGeneratedByLevel,
      scheduledLodWorkerRequests,
      scheduledLodDiskRequests,
      scheduledLodDiskStores,
      completedLodDiskStores: lodDiskStats.stored,
    };
  }

  private estimateLodColumnYRange(
    cx: number,
    cz: number,
    level: number,
    stride: number,
    worldSize: number,
  ): { minCy: number; maxCy: number } | null {
    // For LOD 1, derive Y range from source LOD 0 chunks' solidBounds
    // when available — avoids expensive generator.sampleColumn() calls.
    if (level === 1) {
      const yRange = this.estimateYRangeFromSolidBounds(cx, cz, stride, worldSize);
      if (yRange) return yRange;
    } else {
      const summaryRange = this.estimateYRangeFromColumnSummaries(cx, cz, stride, worldSize);
      if (summaryRange) return summaryRange;
      // For LOD 2+, derive from LOD N-1 chunks' solidBounds
      const yRange = this.estimateYRangeFromLodSolidBounds(cx, cz, level, stride, worldSize);
      if (yRange) return yRange;
    }
    const summaryRange = this.estimateYRangeFromColumnSummaries(cx, cz, stride, worldSize);
    if (summaryRange) return summaryRange;

    return this.estimateYRangeFromSurfaceSamples(cx, cz, stride, worldSize);
  }

  private schedulePersistedRegionSummariesForLodFootprint(
    lodCx: number,
    lodCz: number,
    stride: number,
  ): number {
    if (!this.asyncChunkGeneration) {
      return 0;
    }
    const minCol0X = lodCx * stride;
    const maxCol0X = minCol0X + stride - 1;
    const minCol0Z = lodCz * stride;
    const maxCol0Z = minCol0Z + stride - 1;
    const regions = new Map<string, { x: number; z: number }>();
    for (let cz = minCol0Z; cz <= maxCol0Z; cz += 1) {
      for (let cx = minCol0X; cx <= maxCol0X; cx += 1) {
        if (this.generatedRenderColumnSummaries.has(toColumnKey(cx, cz))) {
          continue;
        }
        const region = getGeneratedRenderSummaryRegionCoord(
          cx,
          cz,
          GENERATED_RENDER_SUMMARY_REGION_SIZE_CHUNKS,
        );
        const key = toRegionSummaryKey(region.x, region.z);
        regions.set(key, region);
      }
    }

    let requested = 0;
    for (const [key, region] of regions) {
      if (
        this.missingGeneratedRenderSummaryRegionKeys.has(key)
        || this.asyncChunkGeneration.hasPendingRegionSummary(region.x, region.z)
      ) {
        continue;
      }
      if (this.asyncChunkGeneration.requestRegionSummary(region.x, region.z)) {
        requested += 1;
      }
    }
    return requested;
  }

  private adoptCompletedLodChunks(
    chunks: AsyncEncodedDerivedLodChunk[],
    neededKeys: Set<string>,
  ): { cacheHits: number; cacheHitsByLevel: readonly number[]; workerGenerated: number; workerGeneratedByLevel: readonly number[]; meshMs: number } {
    let cacheHits = 0;
    const cacheHitsByLevel = createLodLevelCounts();
    let workerGenerated = 0;
    const workerGeneratedByLevel = createLodLevelCounts();
    let meshMs = 0;
    for (const encoded of chunks) {
      this.missingLodDiskCacheKeys.delete(toAsyncLodChunkKey(encoded.key));
      const key = toLodChunkKey(encoded.key.lodLevel, encoded.key.coord.x, encoded.key.coord.y, encoded.key.coord.z);
      const activeIsFresh = this.lodChunks.has(key) && !this.staleLodKeys.has(key);
      if (!neededKeys.has(key) || activeIsFresh || this.preparedLodChunks.has(key)) {
        continue;
      }
      const decoded = decodeDerivedLodChunk(encoded.buffer);
      const source = encoded.source ?? "cache";
      if (decoded.solidCount === 0) {
        this.emptyLodKeys.add(key);
        this.lodChunks.delete(key);
        this.staleLodKeys.delete(key);
        this.clearLodCoveragePunchedKey(key);
        if (source === "generated") {
          workerGenerated += 1;
          workerGeneratedByLevel[decoded.lodLevel] += 1;
        } else {
          cacheHits += 1;
          cacheHitsByLevel[decoded.lodLevel] += 1;
        }
        continue;
      }
      const chunk = createLodResidentChunk(
        {
          coord: decoded.coord,
          data: decoded.data,
          solidCount: decoded.solidCount,
          solidBounds: decoded.solidBounds,
          renderSummary: null!,
        },
        decoded.lodLevel,
        decoded.voxelStride,
      );
      const activation = this.punchLodChunkCoveredByActiveFiner(chunk);
      if (activation.visibleSolidCount === 0) {
        this.coveredEmptyLodKeys.add(key);
        this.lodChunks.delete(key);
        this.staleLodKeys.delete(key);
        this.retainedLodChunks.delete(key);
        this.retainedEmptyLodKeys.delete(key);
        this.clearLodCoveragePunchedKey(key);
        if (source === "generated") {
          workerGenerated += 1;
          workerGeneratedByLevel[decoded.lodLevel] += 1;
        } else {
          cacheHits += 1;
          cacheHitsByLevel[decoded.lodLevel] += 1;
        }
        continue;
      }
      if (activation.skippedFinerCoverage) {
        this.coveragePunchedLodKeys.add(key);
      } else {
        this.clearLodCoveragePunchedKey(key);
      }
      const worldSize = this.chunkSize * decoded.voxelStride;
      const meshStartedAt = performance.now();
      const mesh = this.buildLodChunkMesh(
        activation.chunk,
        decoded.coord.x,
        decoded.coord.y,
        decoded.coord.z,
        decoded.lodLevel,
        decoded.voxelStride,
        worldSize,
      );
      meshMs += performance.now() - meshStartedAt;
      activation.chunk.mesh = mesh;
      activation.chunk.meshBuilt = true;
      activation.chunk.meshDirty = false;
      activation.chunk.renderReady = true;
      activation.chunk.gpuDirty = true;
      this.retainedLodChunks.delete(key);
      this.retainedEmptyLodKeys.delete(key);
      if (this.lodChunkHasCoveredActiveCoarserOverlap(activation.chunk)) {
        this.prepareLodChunkBlockedByActiveCoarser(key, activation.chunk);
      } else {
        this.lodChunks.set(key, activation.chunk);
        this.lodWorldColumnCoverageCache.clear();
        this.staleLodKeys.delete(key);
        this.punchActiveCoarserLodChunksCoveredBy(activation.chunk);
      }
      this.queueAdjacentLodChunkRemeshes(decoded.lodLevel, decoded.coord.x, decoded.coord.y, decoded.coord.z);
      this.invalidateCoarserLodChunksForSourceChunk(decoded.lodLevel, decoded.coord.x, decoded.coord.y, decoded.coord.z, {
        clearNeededKeyCache: false,
      });
      if (source === "generated") {
        workerGenerated += 1;
        workerGeneratedByLevel[decoded.lodLevel] += 1;
      } else {
        cacheHits += 1;
        cacheHitsByLevel[decoded.lodLevel] += 1;
      }
    }
    return { cacheHits, cacheHitsByLevel, workerGenerated, workerGeneratedByLevel, meshMs };
  }

  private scheduleLodDiskChunkRequest(
    lodLevel: number,
    cx: number,
    cy: number,
    cz: number,
    scheduledThisUpdate: number,
  ): "scheduled" | "pending" | "skip" {
    if (!this.asyncChunkGeneration || this.editOverlays.size > 0) {
      return "skip";
    }
    const cacheKey = this.toLodDiskCacheKey(lodLevel, cx, cy, cz);
    const requestKey = toAsyncLodChunkKey(cacheKey);
    if (this.missingLodDiskCacheKeys.has(requestKey)) {
      return "skip";
    }
    if (this.asyncChunkGeneration.hasPendingLodChunk(cacheKey)) {
      return "pending";
    }
    if (scheduledThisUpdate >= MAX_LOD_CACHE_REQUESTS_PER_UPDATE) {
      return "skip";
    }
    return this.asyncChunkGeneration.requestLodChunk(cacheKey) ? "scheduled" : "pending";
  }

  private scheduleGeneratedLodChunkRequest(
    lodLevel: number,
    cx: number,
    cy: number,
    cz: number,
    scheduledThisUpdate: number,
  ): "scheduled" | "pending" | "skip" {
    if (!this.asyncChunkGeneration || this.editOverlays.size > 0 || lodLevel < 1) {
      return "skip";
    }
    if (scheduledThisUpdate >= MAX_LOD_WORKER_GENERATE_REQUESTS_PER_UPDATE) {
      return "pending";
    }
    const cacheKey = this.toLodDiskCacheKey(lodLevel, cx, cy, cz);
    if (this.asyncChunkGeneration.hasPendingGeneratedLodChunk(cacheKey)) {
      return "pending";
    }
    return this.asyncChunkGeneration.requestGeneratedLodChunk(cacheKey) ? "scheduled" : "pending";
  }

  private reviveRetainedLodChunk(key: string): boolean {
    const chunk = this.retainedLodChunks.get(key);
    if (!chunk) {
      return false;
    }
    if (this.staleLodKeys.has(key) || this.coveragePunchedLodKeys.has(key)) {
      this.retainedLodChunks.delete(key);
      this.clearLodCoveragePunchedKey(key);
      return false;
    }
    this.retainedLodChunks.delete(key);
    this.retainedEmptyLodKeys.delete(key);
    const activation = this.punchLodChunkCoveredByActiveFiner(chunk);
    if (activation.visibleSolidCount === 0) {
      this.coveredEmptyLodKeys.add(key);
      this.clearLodCoveragePunchedKey(key);
      return true;
    }
    if (activation.skippedFinerCoverage) {
      this.coveragePunchedLodKeys.add(key);
      const stride = Math.max(1, activation.chunk.voxelStride);
      const worldSize = this.chunkSize * stride;
      if (!this.meshMaterialLut) {
        this.meshMaterialLut = createMeshMaterialLut(this.palette, isProceduralWaterMaterial);
      }
      activation.chunk.mesh = this.buildLodChunkMesh(
        activation.chunk,
        activation.chunk.coord.x,
        activation.chunk.coord.y,
        activation.chunk.coord.z,
        activation.chunk.lodLevel,
        stride,
        worldSize,
      );
      activation.chunk.meshBuilt = true;
      activation.chunk.meshDirty = false;
      activation.chunk.renderReady = true;
    } else {
      this.clearLodCoveragePunchedKey(key);
    }
    activation.chunk.gpuDirty = true;
    if (this.lodChunkHasCoveredActiveCoarserOverlap(activation.chunk)) {
      this.prepareLodChunkBlockedByActiveCoarser(key, activation.chunk);
    } else {
      this.lodChunks.set(key, activation.chunk);
      this.lodWorldColumnCoverageCache.clear();
      this.staleLodKeys.delete(key);
      this.punchActiveCoarserLodChunksCoveredBy(activation.chunk);
    }
    return true;
  }

  private reviveRetainedEmptyLodKey(key: string): boolean {
    if (!this.retainedEmptyLodKeys.has(key)) {
      return false;
    }
    this.retainedEmptyLodKeys.delete(key);
    this.retainedLodChunks.delete(key);
    this.clearLodCoveragePunchedKey(key);
    this.emptyLodKeys.add(key);
    return true;
  }

  private retainLodChunk(key: string, chunk: VoxelChunk): void {
    if (chunk.solidCount === 0 || this.staleLodKeys.has(key) || this.coveragePunchedLodKeys.has(key)) {
      this.retainedLodChunks.delete(key);
      this.clearLodCoveragePunchedKey(key);
      return;
    }
    this.retainedEmptyLodKeys.delete(key);
    this.retainedLodChunks.delete(key);
    this.retainedLodChunks.set(key, chunk);
    this.enqueueLodDiskStore(key);
    while (this.retainedLodChunks.size > MAX_RETAINED_LOD_CHUNKS) {
      const oldestKey = this.retainedLodChunks.keys().next().value as string | undefined;
      if (!oldestKey) break;
      this.retainedLodChunks.delete(oldestKey);
      this.queuedLodDiskStoreKeys.delete(oldestKey);
    }
  }

  private retainEmptyLodKey(key: string): void {
    this.retainedLodChunks.delete(key);
    this.clearLodCoveragePunchedKey(key);
    this.retainedEmptyLodKeys.delete(key);
    this.retainedEmptyLodKeys.add(key);
    while (this.retainedEmptyLodKeys.size > MAX_RETAINED_EMPTY_LOD_KEYS) {
      const oldestKey = this.retainedEmptyLodKeys.values().next().value as string | undefined;
      if (!oldestKey) break;
      this.retainedEmptyLodKeys.delete(oldestKey);
    }
  }

  private deleteRetainedLodKey(key: string): void {
    this.retainedLodChunks.delete(key);
    this.retainedEmptyLodKeys.delete(key);
    this.clearLodCoveragePunchedKey(key);
    this.queuedLodDiskStoreKeys.delete(key);
  }

  private clearRetainedLodCache(): void {
    this.retainedLodChunks.clear();
    this.retainedEmptyLodKeys.clear();
    this.coveragePunchedLodKeys.clear();
    this.lodRenderClipMasks.clear();
    this.queuedLodDiskStoreKeys.clear();
    this.lodDiskStoreQueue.length = 0;
    this.pendingLodNeighborRemeshKeys.clear();
  }

  private enqueueLodDiskStore(key: string): void {
    if (!this.asyncChunkGeneration || this.queuedLodDiskStoreKeys.has(key)) {
      return;
    }
    this.queuedLodDiskStoreKeys.add(key);
    this.lodDiskStoreQueue.push(key);
  }

  private flushQueuedLodDiskStores(): number {
    if (!this.asyncChunkGeneration || this.editOverlays.size > 0) {
      return 0;
    }
    let scheduled = 0;
    while (scheduled < MAX_LOD_CACHE_STORES_PER_UPDATE && this.lodDiskStoreQueue.length > 0) {
      const key = this.lodDiskStoreQueue.shift()!;
      this.queuedLodDiskStoreKeys.delete(key);
      const chunk = this.retainedLodChunks.get(key) ?? this.lodChunks.get(key) ?? this.preparedLodChunks.get(key);
      if (
        !chunk
        || chunk.solidCount === 0
        || chunk.lodLevel <= 0
        || this.coveragePunchedLodKeys.has(key)
        || this.staleLodKeys.has(key)
      ) {
        continue;
      }
      const encoded = encodeDerivedLodChunk({
        coord: { ...chunk.coord },
        lodLevel: chunk.lodLevel,
        voxelStride: chunk.voxelStride,
        data: chunk.data,
        solidCount: chunk.solidCount,
        solidBounds: chunk.solidBounds
          ? {
              min: [...chunk.solidBounds.min],
              max: [...chunk.solidBounds.max],
            }
          : null,
      });
      if (!this.asyncChunkGeneration.storeLodChunk({
        key: this.toLodDiskCacheKey(chunk.lodLevel, chunk.coord.x, chunk.coord.y, chunk.coord.z),
        buffer: encoded.buffer,
        byteLength: encoded.stats.byteLength,
      })) {
        this.queuedLodDiskStoreKeys.add(key);
        this.lodDiskStoreQueue.unshift(key);
        break;
      }
      this.missingLodDiskCacheKeys.delete(toAsyncLodChunkKey(this.toLodDiskCacheKey(chunk.lodLevel, chunk.coord.x, chunk.coord.y, chunk.coord.z)));
      scheduled += 1;
    }
    return scheduled;
  }

  private toLodDiskCacheKey(lodLevel: number, cx: number, cy: number, cz: number): AsyncDerivedLodChunkCacheKey {
    return {
      lodLevel,
      coord: { x: cx, y: cy, z: cz },
      editRevision: this.nextEditSequence - 1,
    };
  }

  private planLodNeededKeys(
    position: Vec3,
    signature: string,
    maxPlanMs: number,
  ): {
    complete: boolean;
    keys: string[];
    neededKeyCount: number;
    yRangeMs: number;
    scheduledRegionSummaryRequests: number;
  } {
    let state = this.lodNeededKeyPlanningState;
    if (!state || state.signature !== signature) {
      state = {
        signature,
        neededKeys: new Set<string>(),
        ringIndex: 0,
        dx: -LOD_RINGS[0]!.radiusChunks,
        dz: -LOD_RINGS[0]!.radiusChunks,
      };
      this.lodNeededKeyPlanningState = state;
    }

    const startedAt = performance.now();
    let yRangeMs = 0;
    let scheduledRegionSummaryRequests = 0;
    while (state.ringIndex < LOD_RINGS.length) {
      const ring = LOD_RINGS[state.ringIndex]!;
      const stride = 1 << ring.level;
      const worldSize = this.chunkSize * stride;
      const lcx = Math.floor(position[0] / worldSize);
      const lcz = Math.floor(position[2] / worldSize);

      while (state.dz <= ring.radiusChunks) {
        while (state.dx <= ring.radiusChunks) {
          const dx = state.dx;
          const dz = state.dz;
          state.dx += 1;
          const cx = lcx + dx;
          const cz = lcz + dz;
          if (!lodFootprintIntersectsRadius(position[0], position[2], cx, cz, worldSize, LOD_FOG_COVERAGE_RADIUS)) {
            if (performance.now() - startedAt >= maxPlanMs) {
              return {
                complete: false,
                keys: [],
                neededKeyCount: state.neededKeys.size,
                yRangeMs,
                scheduledRegionSummaryRequests,
              };
            }
            continue;
          }
          if (state.ringIndex > 0) {
            const finerRing = LOD_RINGS[state.ringIndex - 1]!;
            const finerCoverageRadius = finerRing.radiusChunks * this.chunkSize * (1 << finerRing.level);
            if (lodFootprintFullyInsideRadius(position[0], position[2], cx, cz, worldSize, finerCoverageRadius)) {
              if (performance.now() - startedAt >= maxPlanMs) {
                return {
                  complete: false,
                  keys: [],
                  neededKeyCount: state.neededKeys.size,
                  yRangeMs,
                  scheduledRegionSummaryRequests,
                };
              }
              continue;
            }
          }

          const coveredByFinerLod = ring.level === 1
            ? this.isLodChunkFullyCoveredByRenderReadyColumns(cx, cz, stride)
            : false;

          if (!coveredByFinerLod) {
            scheduledRegionSummaryRequests += this.schedulePersistedRegionSummariesForLodFootprint(cx, cz, stride);
            const yRangeStartedAt = performance.now();
            const yRange = this.estimateLodColumnYRange(cx, cz, ring.level, stride, worldSize);
            yRangeMs += performance.now() - yRangeStartedAt;
            if (yRange) {
              for (let cy = yRange.minCy; cy <= yRange.maxCy; cy += 1) {
                state.neededKeys.add(`L${ring.level}:${cx}:${cy}:${cz}`);
              }
            }
          }

          if (performance.now() - startedAt >= maxPlanMs) {
            return {
              complete: false,
              keys: [],
              neededKeyCount: state.neededKeys.size,
              yRangeMs,
              scheduledRegionSummaryRequests,
            };
          }
        }
        state.dx = -ring.radiusChunks;
        state.dz += 1;
      }

      state.ringIndex += 1;
      if (state.ringIndex < LOD_RINGS.length) {
        const nextRing = LOD_RINGS[state.ringIndex]!;
        state.dx = -nextRing.radiusChunks;
        state.dz = -nextRing.radiusChunks;
      }
    }

    const keys = [...state.neededKeys];
    this.lodNeededKeyPlanningState = null;
    this.lodNeededKeyCache = { signature, keys };
    return {
      complete: true,
      keys,
      neededKeyCount: keys.length,
      yRangeMs,
      scheduledRegionSummaryRequests,
    };
  }

  private estimateYRangeFromSolidBounds(
    lodCx: number,
    lodCz: number,
    stride: number,
    worldSize: number,
  ): { minCy: number; maxCy: number } | null {
    // Scan all LOD 0 columns in this LOD chunk's footprint
    const minCol0X = lodCx * stride;
    const maxCol0X = minCol0X + stride - 1;
    const minCol0Z = lodCz * stride;
    const maxCol0Z = minCol0Z + stride - 1;
    let globalMinY = Infinity;
    let globalMaxY = -Infinity;
    let foundAny = false;
    for (let cz = minCol0Z; cz <= maxCol0Z; cz++) {
      for (let cx = minCol0X; cx <= maxCol0X; cx++) {
        const yRange = this.columnChunkYRanges.get(toColumnKey(cx, cz));
        if (!yRange) continue;
        // Scan resident chunks in this column for solidBounds
        for (let cy = yRange.minCy; cy <= yRange.maxCy; cy++) {
          const chunk = this.getResidentChunk(cx, cy, cz);
          if (!chunk || chunk.solidCount === 0 || !chunk.solidBounds) continue;
          const worldMinY = cy * this.chunkSize + chunk.solidBounds.min[1];
          const worldMaxY = cy * this.chunkSize + chunk.solidBounds.max[1];
          globalMinY = Math.min(globalMinY, worldMinY);
          globalMaxY = Math.max(globalMaxY, worldMaxY);
          foundAny = true;
        }
      }
    }
    if (!foundAny) return null;
    const shellPaddingY = Math.max(stride * 2, this.chunkSize);
    const paddedMinY = Math.max(0, globalMinY - shellPaddingY);
    const paddedMaxY = Math.min(this.maxYExclusive - 1, globalMaxY + this.airPaddingChunks * this.chunkSize);
    return {
      minCy: Math.max(0, Math.floor(paddedMinY / worldSize)),
      maxCy: Math.max(0, Math.floor(paddedMaxY / worldSize)),
    };
  }

  private estimateYRangeFromLodSolidBounds(
    lodCx: number,
    lodCz: number,
    level: number,
    _stride: number,
    worldSize: number,
  ): { minCy: number; maxCy: number } | null {
    // Scan LOD (level-1) chunks that overlap this LOD chunk's XZ footprint
    const sourceStride = 1 << (level - 1);
    const sourceWorldSize = this.chunkSize * sourceStride;
    const srcMinCx = Math.floor((lodCx * worldSize) / sourceWorldSize);
    const srcMaxCx = Math.floor(((lodCx + 1) * worldSize - 1) / sourceWorldSize);
    const srcMinCz = Math.floor((lodCz * worldSize) / sourceWorldSize);
    const srcMaxCz = Math.floor(((lodCz + 1) * worldSize - 1) / sourceWorldSize);
    let globalMinY = Infinity;
    let globalMaxY = -Infinity;
    let foundAny = false;
    for (let scz = srcMinCz; scz <= srcMaxCz; scz++) {
      for (let scx = srcMinCx; scx <= srcMaxCx; scx++) {
        // Scan a reasonable Y range for source LOD chunks
        for (let scy = 0; scy < Math.ceil(this.maxYExclusive / sourceWorldSize); scy++) {
          const srcChunk = this.lodChunks.get(`L${level - 1}:${scx}:${scy}:${scz}`);
          if (!srcChunk || srcChunk.solidCount === 0 || !srcChunk.solidBounds) continue;
          const worldMinY = scy * sourceWorldSize + srcChunk.solidBounds.min[1] * sourceStride;
          const worldMaxY = scy * sourceWorldSize + srcChunk.solidBounds.max[1] * sourceStride;
          globalMinY = Math.min(globalMinY, worldMinY);
          globalMaxY = Math.max(globalMaxY, worldMaxY);
          foundAny = true;
        }
      }
    }
    if (!foundAny) return null;
    const shellPaddingY = Math.max(sourceStride * 2, this.chunkSize);
    const paddedMinY = Math.max(0, globalMinY - shellPaddingY);
    const paddedMaxY = Math.min(this.maxYExclusive - 1, globalMaxY + this.airPaddingChunks * this.chunkSize);
    return {
      minCy: Math.max(0, Math.floor(paddedMinY / worldSize)),
      maxCy: Math.max(0, Math.floor(paddedMaxY / worldSize)),
    };
  }

  private estimateYRangeFromColumnSummaries(
    lodCx: number,
    lodCz: number,
    stride: number,
    worldSize: number,
  ): { minCy: number; maxCy: number } | null {
    const minCol0X = lodCx * stride;
    const maxCol0X = minCol0X + stride - 1;
    const minCol0Z = lodCz * stride;
    const maxCol0Z = minCol0Z + stride - 1;
    let minWorldY = Number.POSITIVE_INFINITY;
    let maxWorldY = Number.NEGATIVE_INFINITY;

    for (let cz = minCol0Z; cz <= maxCol0Z; cz += 1) {
      for (let cx = minCol0X; cx <= maxCol0X; cx += 1) {
        const summary = this.generatedRenderColumnSummaries.get(toColumnKey(cx, cz));
        if (!summary) continue;
        const range = estimateColumnSummaryWorldYRange(summary, this.chunkSize);
        if (!range) continue;
        minWorldY = Math.min(minWorldY, range.minY);
        maxWorldY = Math.max(maxWorldY, range.maxY);
      }
    }

    if (!Number.isFinite(minWorldY) || !Number.isFinite(maxWorldY)) {
      return null;
    }
    return this.toPaddedLodYRange(minWorldY, maxWorldY, stride, worldSize);
  }

  private estimateYRangeFromSurfaceSamples(
    cx: number,
    cz: number,
    stride: number,
    worldSize: number,
  ): { minCy: number; maxCy: number } {
    const points = [
      [cx * worldSize, cz * worldSize],
      [cx * worldSize + worldSize - 1, cz * worldSize],
      [cx * worldSize, cz * worldSize + worldSize - 1],
      [cx * worldSize + worldSize - 1, cz * worldSize + worldSize - 1],
      [cx * worldSize + (worldSize >> 1), cz * worldSize + (worldSize >> 1)],
    ];
    let minSurfaceY = Infinity;
    let maxWorldY = -Infinity;
    for (const [px, pz] of points) {
      const column = this.generator.sampleSurfaceColumn(px!, pz!);
      minSurfaceY = Math.min(minSurfaceY, column.surfaceY);
      maxWorldY = Math.max(maxWorldY, column.surfaceY, column.topY, column.waterTopY ?? column.surfaceY);
    }
    return this.toPaddedLodYRange(minSurfaceY, maxWorldY, stride, worldSize);
  }

  private toPaddedLodYRange(
    minWorldY: number,
    maxWorldY: number,
    stride: number,
    worldSize: number,
  ): { minCy: number; maxCy: number } {
    const shellPaddingY = stride * 3;
    const paddedMinY = Math.max(this.minY, minWorldY - shellPaddingY);
    const paddedMaxY = Math.min(this.maxYExclusive - 1, maxWorldY);
    return {
      minCy: Math.max(0, Math.floor(paddedMinY / worldSize)),
      maxCy: Math.max(0, Math.floor(paddedMaxY / worldSize)),
    };
  }

  private createPartialLodChunkBuild(
    key: string,
    cx: number,
    cy: number,
    cz: number,
    level: number,
    stride: number,
    worldSize: number,
  ): PartialLodChunkBuild {
    const cs = this.chunkSize;
    return {
      key,
      cx,
      cy,
      cz,
      level,
      stride,
      worldSize,
      data: new Uint16Array(cs * cs * cs),
      solidCount: 0,
      minX: cs,
      minY: cs,
      minZ: cs,
      maxX: 0,
      maxY: 0,
      maxZ: 0,
      sourceComplete: true,
      skippedFinerCoverage: false,
      nextColumn: 0,
    };
  }

  private processPartialLodChunkBuild(job: PartialLodChunkBuild, deadlineMs: number): boolean {
    const cs = this.chunkSize;
    const totalColumns = cs * cs;
    const originX = job.cx * job.worldSize;
    const originY = job.cy * job.worldSize;
    const originZ = job.cz * job.worldSize;
    while (job.nextColumn < totalColumns) {
      const column = job.nextColumn;
      job.nextColumn += 1;
      const ox = column % cs;
      const oz = Math.floor(column / cs);
      if (this.isOutputLodColumnCoveredByFiner(job.level, originX, originZ, job.stride, ox, oz)) {
        job.skippedFinerCoverage = true;
      } else {
        const topBucket = this.generator.sampleTopColumnMaterialBucket(
          originX + ox * job.stride,
          originZ + oz * job.stride,
          originY,
          job.stride,
          cs,
          job.stride * 3,
        );
        if (topBucket) {
          this.recordPartialLodMaterial(job, ox, topBucket.bucketIndex, oz, topBucket.material);
        }
      }
      if (job.nextColumn < totalColumns && performance.now() >= deadlineMs) {
        return false;
      }
    }
    return true;
  }

  private recordPartialLodMaterial(
    job: PartialLodChunkBuild,
    ox: number,
    oy: number,
    oz: number,
    material: number,
  ): void {
    const cs = this.chunkSize;
    const chunkArea = cs * cs;
    job.data[ox + oy * cs + oz * chunkArea] = material;
    job.solidCount += 1;
    if (ox < job.minX) job.minX = ox;
    if (oy < job.minY) job.minY = oy;
    if (oz < job.minZ) job.minZ = oz;
    if (ox + 1 > job.maxX) job.maxX = ox + 1;
    if (oy + 1 > job.maxY) job.maxY = oy + 1;
    if (oz + 1 > job.maxZ) job.maxZ = oz + 1;
  }

  private finalizePartialLodChunkBuild(job: PartialLodChunkBuild): {
    data: Uint16Array;
    solidCount: number;
    solidBounds: { min: [number, number, number]; max: [number, number, number] } | null;
    sourceComplete: boolean;
    skippedFinerCoverage: boolean;
  } {
    return {
      data: job.data,
      solidCount: job.solidCount,
      solidBounds: job.solidCount === 0
        ? null
        : { min: [job.minX, job.minY, job.minZ], max: [job.maxX, job.maxY, job.maxZ] },
      sourceComplete: job.sourceComplete,
      skippedFinerCoverage: job.skippedFinerCoverage,
    };
  }

  private isOutputLodColumnCoveredByFiner(
    level: number,
    originX: number,
    originZ: number,
    stride: number,
    ox: number,
    oz: number,
  ): boolean {
    const minWorldX = originX + ox * stride;
    const maxWorldX = minWorldX + stride - 1;
    const minWorldZ = originZ + oz * stride;
    const maxWorldZ = minWorldZ + stride - 1;
    const minChunkX = Math.floor(minWorldX / this.chunkSize);
    const maxChunkX = Math.floor(maxWorldX / this.chunkSize);
    const minChunkZ = Math.floor(minWorldZ / this.chunkSize);
    const maxChunkZ = Math.floor(maxWorldZ / this.chunkSize);
    for (let chunkZ = minChunkZ; chunkZ <= maxChunkZ; chunkZ += 1) {
      for (let chunkX = minChunkX; chunkX <= maxChunkX; chunkX += 1) {
        if (this.isColumnRenderReady(chunkX, chunkZ)) {
          return true;
        }
      }
    }
    for (let finerLevel = 1; finerLevel < level; finerLevel += 1) {
      const finerStride = 1 << finerLevel;
      const finerWorldSize = this.chunkSize * finerStride;
      const minLodX = Math.floor(minWorldX / finerWorldSize);
      const maxLodX = Math.floor(maxWorldX / finerWorldSize);
      const minLodZ = Math.floor(minWorldZ / finerWorldSize);
      const maxLodZ = Math.floor(maxWorldZ / finerWorldSize);
      for (let lodZ = minLodZ; lodZ <= maxLodZ; lodZ += 1) {
        for (let lodX = minLodX; lodX <= maxLodX; lodX += 1) {
          if (this.isLodWorldColumnCovered(
            finerLevel,
            lodX,
            lodZ,
            minWorldX,
            maxWorldX,
            minWorldZ,
            maxWorldZ,
          )) {
            return true;
          }
        }
      }
    }
    return false;
  }

  private downsampleLodChunkData(
    lodCx: number,
    lodCy: number,
    lodCz: number,
    level: number,
    stride: number,
    worldSize: number,
  ): {
    data: Uint16Array;
    solidCount: number;
    solidBounds: { min: [number, number, number]; max: [number, number, number] } | null;
    sourceComplete: boolean;
    skippedFinerCoverage: boolean;
  } {
    const cs = this.chunkSize;
    const chunkArea = cs * cs;

    // World-space origin of this LOD chunk
    const originX = lodCx * worldSize;
    const originY = lodCy * worldSize;
    const originZ = lodCz * worldSize;
    const sourceLevel = level - 1;
    const sourceStride = 1 << sourceLevel;
    const sourceWorldSize = cs * sourceStride;
    const sourceVoxelSpan = Math.max(1, Math.floor(stride / sourceStride));

    const isOutputColumnCoveredByFiner = (ox: number, oz: number): boolean => {
      const minWorldX = originX + ox * stride;
      const maxWorldX = minWorldX + stride - 1;
      const minWorldZ = originZ + oz * stride;
      const maxWorldZ = minWorldZ + stride - 1;
      const minChunkX = Math.floor(minWorldX / this.chunkSize);
      const maxChunkX = Math.floor(maxWorldX / this.chunkSize);
      const minChunkZ = Math.floor(minWorldZ / this.chunkSize);
      const maxChunkZ = Math.floor(maxWorldZ / this.chunkSize);
      for (let chunkZ = minChunkZ; chunkZ <= maxChunkZ; chunkZ += 1) {
        for (let chunkX = minChunkX; chunkX <= maxChunkX; chunkX += 1) {
          if (this.isColumnRenderReady(chunkX, chunkZ)) {
            return true;
          }
        }
      }
      for (let finerLevel = 1; finerLevel < level; finerLevel += 1) {
        const finerStride = 1 << finerLevel;
        const finerWorldSize = this.chunkSize * finerStride;
        const minLodX = Math.floor(minWorldX / finerWorldSize);
        const maxLodX = Math.floor(maxWorldX / finerWorldSize);
        const minLodZ = Math.floor(minWorldZ / finerWorldSize);
        const maxLodZ = Math.floor(maxWorldZ / finerWorldSize);
        for (let lodZ = minLodZ; lodZ <= maxLodZ; lodZ += 1) {
          for (let lodX = minLodX; lodX <= maxLodX; lodX += 1) {
            if (this.isLodWorldColumnCovered(
              finerLevel,
              lodX,
              lodZ,
              minWorldX,
              maxWorldX,
              minWorldZ,
              maxWorldZ,
            )) {
              return true;
            }
          }
        }
      }
      return false;
    };
    const sampleSourceMaterial = (
      ox: number,
      oy: number,
      oz: number,
    ): { material: number; complete: boolean } => {
      const baseWorldX = originX + ox * stride;
      const baseWorldY = originY + oy * stride;
      const baseWorldZ = originZ + oz * stride;
      let complete = true;
      let waterMaterial = 0;

      for (let dy = sourceVoxelSpan - 1; dy >= 0; dy -= 1) {
        for (let dz = 0; dz < sourceVoxelSpan; dz += 1) {
          for (let dx = 0; dx < sourceVoxelSpan; dx += 1) {
            const sourceWorldX = baseWorldX + dx * sourceStride;
            const sourceWorldY = baseWorldY + dy * sourceStride;
            const sourceWorldZ = baseWorldZ + dz * sourceStride;
            const sourceCx = Math.floor(sourceWorldX / sourceWorldSize);
            const sourceCy = Math.floor(sourceWorldY / sourceWorldSize);
            const sourceCz = Math.floor(sourceWorldZ / sourceWorldSize);
            const sourceChunk = sourceLevel === 0
              ? this.chunks.get(toChunkKey(sourceCx, sourceCy, sourceCz))
              : this.lodChunks.get(`L${sourceLevel}:${sourceCx}:${sourceCy}:${sourceCz}`);
            if (!sourceChunk) {
              const sourceKnownEmpty = sourceLevel === 0
                ? this.emptyChunkKeys.has(toChunkKey(sourceCx, sourceCy, sourceCz))
                : this.emptyLodKeys.has(`L${sourceLevel}:${sourceCx}:${sourceCy}:${sourceCz}`)
                  || this.coveredEmptyLodKeys.has(`L${sourceLevel}:${sourceCx}:${sourceCy}:${sourceCz}`);
              if (!sourceKnownEmpty) {
                complete = false;
              }
              continue;
            }
            const localX = Math.floor((sourceWorldX - sourceCx * sourceWorldSize) / sourceStride);
            const localY = Math.floor((sourceWorldY - sourceCy * sourceWorldSize) / sourceStride);
            const localZ = Math.floor((sourceWorldZ - sourceCz * sourceWorldSize) / sourceStride);
            const material = sourceChunk.data[localX + localY * cs + localZ * chunkArea] ?? 0;
            if (material === 0) {
              continue;
            }
            if (!isProceduralWaterMaterial(material)) {
              return { material, complete };
            }
            if (waterMaterial === 0) {
              waterMaterial = material;
            }
          }
        }
      }
      return { material: waterMaterial, complete };
    };
    return deriveLodChunkData({
      chunkSize: cs,
      originX,
      originY,
      originZ,
      level,
      stride,
      useGeneratedTopMaterial: level >= 2 && this.editOverlays.size === 0,
      sampleSourceMaterial,
      isOutputColumnCoveredByFiner,
      sampleGeneratedTopMaterial: (ox, oz, minOy, maxOyExclusive) => {
        const worldX = originX + ox * stride;
        const worldZ = originZ + oz * stride;
        return this.generator.sampleTopColumnMaterialBucket(
          worldX,
          worldZ,
          originY + minOy * stride,
          stride,
          maxOyExclusive - minOy,
          stride * 3,
        );
      },
      sampleSurfaceYRange: (ox, oz) => {
        const worldX = originX + ox * stride;
        const worldZ = originZ + oz * stride;
        const column = this.generator.sampleSurfaceColumn(worldX, worldZ);
        return {
          minY: column.surfaceY,
          maxY: Math.max(column.topY, column.waterTopY ?? column.surfaceY),
        };
      },
      sampleGeneratedColumnMaterials: (ox, oz, startOy, endOyInclusive) => {
        const worldX = originX + ox * stride;
        const worldZ = originZ + oz * stride;
        return this.generator.sampleColumnMaterialBuckets(
          worldX,
          worldZ,
          originY + startOy * stride,
          stride,
          endOyInclusive - startOy + 1,
        );
      },
    });
  }

  private invalidateLodChunksAt(worldX: number, worldY: number, worldZ: number): void {
    this.clearLodNeededKeyCache();
    for (const ring of LOD_RINGS) {
      const stride = 1 << ring.level;
      const worldSize = this.chunkSize * stride;
      const key = `L${ring.level}:${Math.floor(worldX / worldSize)}:${Math.floor(worldY / worldSize)}:${Math.floor(worldZ / worldSize)}`;
      if (this.lodChunks.has(key)) {
        this.staleLodKeys.add(key);
      }
      this.preparedLodChunks.delete(key);
      this.emptyLodKeys.delete(key);
      this.coveredEmptyLodKeys.delete(key);
      this.deleteRetainedLodKey(key);
    }
  }

  private invalidateCoarserLodChunksForSourceChunk(
    sourceLevel: number,
    sourceCx: number,
    sourceCy: number,
    sourceCz: number,
    options: { clearNeededKeyCache?: boolean } = {},
  ): void {
    if (options.clearNeededKeyCache !== false) {
      this.clearLodNeededKeyCache();
    }
    const sourceStride = 1 << sourceLevel;
    const sourceWorldSize = this.chunkSize * sourceStride;
    const sourceWorldX = sourceCx * sourceWorldSize;
    const sourceWorldY = sourceCy * sourceWorldSize;
    const sourceWorldZ = sourceCz * sourceWorldSize;
    for (const ring of LOD_RINGS) {
      if (ring.level <= sourceLevel) {
        continue;
      }
      const stride = 1 << ring.level;
      const worldSize = this.chunkSize * stride;
      const key = `L${ring.level}:${Math.floor(sourceWorldX / worldSize)}:${Math.floor(sourceWorldY / worldSize)}:${Math.floor(sourceWorldZ / worldSize)}`;
      if (this.lodChunks.has(key)) {
        this.staleLodKeys.add(key);
        this.punchActiveLodChunkCoveredByFiner(key);
      }
      this.preparedLodChunks.delete(key);
      this.emptyLodKeys.delete(key);
      this.coveredEmptyLodKeys.delete(key);
      this.deleteRetainedLodKey(key);
    }
  }

  private clearLodNeededKeyCache(): void {
    this.lodNeededKeyCache = null;
    this.lodNeededKeyPlanningState = null;
    this.coveredEmptyLodKeys.clear();
    this.coveredEmptyLodSignature = null;
  }

  private createLodNeededKeySignature(position: Vec3): string {
    const quantizedX = Math.floor(position[0] / this.chunkSize);
    const quantizedZ = Math.floor(position[2] / this.chunkSize);
    const parts = [
      `x${quantizedX}`,
      `z${quantizedZ}`,
      `resident${this.residentColumnRevision}`,
      `ready${this.renderReadyColumnRevision}`,
    ];
    for (const ring of LOD_RINGS) {
      const worldSize = this.chunkSize * (1 << ring.level);
      parts.push(
        `L${ring.level}:${Math.floor(position[0] / worldSize)}:${Math.floor(position[2] / worldSize)}`,
      );
    }
    return parts.join("|");
  }

  private isLodChunkFullyCoveredByRenderReadyColumns(
    lodCx: number,
    lodCz: number,
    stride: number,
  ): boolean {
    // Check if ALL LOD 0 columns within this LOD chunk's XZ footprint are render-ready.
    const minCol0X = lodCx * stride;
    const maxCol0X = minCol0X + stride - 1;
    const minCol0Z = lodCz * stride;
    const maxCol0Z = minCol0Z + stride - 1;
    for (let cz = minCol0Z; cz <= maxCol0Z; cz++) {
      for (let cx = minCol0X; cx <= maxCol0X; cx++) {
        if (!this.isColumnRenderReady(cx, cz)) {
          return false;
        }
      }
    }
    return true;
  }

  private isLodWorldColumnCovered(
    level: number,
    cx: number,
    cz: number,
    minWorldX: number,
    maxWorldX: number,
    minWorldZ: number,
    maxWorldZ: number,
  ): boolean {
    const stride = 1 << level;
    const worldSize = this.chunkSize * stride;
    const minLocalX = Math.max(0, Math.floor((minWorldX - cx * worldSize) / stride));
    const maxLocalX = Math.min(this.chunkSize - 1, Math.floor((maxWorldX - cx * worldSize) / stride));
    const minLocalZ = Math.max(0, Math.floor((minWorldZ - cz * worldSize) / stride));
    const maxLocalZ = Math.min(this.chunkSize - 1, Math.floor((maxWorldZ - cz * worldSize) / stride));
    if (minLocalX > maxLocalX || minLocalZ > maxLocalZ) {
      return false;
    }
    const cacheKey = `${level}:${cx}:${cz}:${minLocalX}:${maxLocalX}:${minLocalZ}:${maxLocalZ}`;
    const cached = this.lodWorldColumnCoverageCache.get(cacheKey);
    if (cached !== undefined) {
      return cached;
    }
    const chunkArea = this.chunkSize * this.chunkSize;
    const maxCyExclusive = Math.ceil(this.maxYExclusive / worldSize);
    for (let cy = 0; cy < maxCyExclusive; cy += 1) {
      const chunk = this.lodChunks.get(`L${level}:${cx}:${cy}:${cz}`);
      if (!chunk?.renderReady || chunk.solidCount === 0) {
        continue;
      }
      for (let localZ = minLocalZ; localZ <= maxLocalZ; localZ += 1) {
        for (let localX = minLocalX; localX <= maxLocalX; localX += 1) {
          const columnOffset = localX + localZ * chunkArea;
          for (let localY = 0; localY < this.chunkSize; localY += 1) {
            if (this.isLodChunkLocalVoxelVisible(chunk, columnOffset + localY * this.chunkSize)) {
              this.lodWorldColumnCoverageCache.set(cacheKey, true);
              return true;
            }
          }
        }
      }
    }
    this.lodWorldColumnCoverageCache.set(cacheKey, false);
    return false;
  }

  private isOutputLodVoxelCoveredByFiner(
    level: number,
    originX: number,
    originY: number,
    originZ: number,
    stride: number,
    ox: number,
    oy: number,
    oz: number,
  ): boolean {
    const minWorldX = originX + ox * stride;
    const maxWorldX = minWorldX + stride - 1;
    const minWorldY = originY + oy * stride;
    const maxWorldY = minWorldY + stride - 1;
    const minWorldZ = originZ + oz * stride;
    const maxWorldZ = minWorldZ + stride - 1;
    const minChunkX = Math.floor(minWorldX / this.chunkSize);
    const maxChunkX = Math.floor(maxWorldX / this.chunkSize);
    const minChunkZ = Math.floor(minWorldZ / this.chunkSize);
    const maxChunkZ = Math.floor(maxWorldZ / this.chunkSize);
    for (let chunkZ = minChunkZ; chunkZ <= maxChunkZ; chunkZ += 1) {
      for (let chunkX = minChunkX; chunkX <= maxChunkX; chunkX += 1) {
        if (this.isColumnRenderReady(chunkX, chunkZ)) {
          return true;
        }
      }
    }
    for (let finerLevel = 1; finerLevel < level; finerLevel += 1) {
      const finerStride = 1 << finerLevel;
      const finerWorldSize = this.chunkSize * finerStride;
      const minLodX = Math.floor(minWorldX / finerWorldSize);
      const maxLodX = Math.floor(maxWorldX / finerWorldSize);
      const minLodZ = Math.floor(minWorldZ / finerWorldSize);
      const maxLodZ = Math.floor(maxWorldZ / finerWorldSize);
      for (let lodZ = minLodZ; lodZ <= maxLodZ; lodZ += 1) {
        for (let lodX = minLodX; lodX <= maxLodX; lodX += 1) {
          if (this.isLodWorldVoxelRangeCovered(
            finerLevel,
            lodX,
            lodZ,
            minWorldX,
            maxWorldX,
            minWorldY,
            maxWorldY,
            minWorldZ,
            maxWorldZ,
          )) {
            return true;
          }
        }
      }
    }
    return false;
  }

  private isLodWorldVoxelRangeCovered(
    level: number,
    cx: number,
    cz: number,
    minWorldX: number,
    maxWorldX: number,
    minWorldY: number,
    maxWorldY: number,
    minWorldZ: number,
    maxWorldZ: number,
  ): boolean {
    const stride = 1 << level;
    const worldSize = this.chunkSize * stride;
    const minLocalX = Math.max(0, Math.floor((minWorldX - cx * worldSize) / stride));
    const maxLocalX = Math.min(this.chunkSize - 1, Math.floor((maxWorldX - cx * worldSize) / stride));
    const minLocalZ = Math.max(0, Math.floor((minWorldZ - cz * worldSize) / stride));
    const maxLocalZ = Math.min(this.chunkSize - 1, Math.floor((maxWorldZ - cz * worldSize) / stride));
    const minCy = Math.max(0, Math.floor(minWorldY / worldSize));
    const maxCy = Math.min(Math.ceil(this.maxYExclusive / worldSize) - 1, Math.floor(maxWorldY / worldSize));
    if (minLocalX > maxLocalX || minLocalZ > maxLocalZ || minCy > maxCy) {
      return false;
    }
    const chunkArea = this.chunkSize * this.chunkSize;
    for (let cy = minCy; cy <= maxCy; cy += 1) {
      const chunk = this.lodChunks.get(`L${level}:${cx}:${cy}:${cz}`);
      if (!chunk?.renderReady || chunk.solidCount === 0) {
        continue;
      }
      const minLocalY = Math.max(0, Math.floor((minWorldY - cy * worldSize) / stride));
      const maxLocalY = Math.min(this.chunkSize - 1, Math.floor((maxWorldY - cy * worldSize) / stride));
      for (let localZ = minLocalZ; localZ <= maxLocalZ; localZ += 1) {
        for (let localX = minLocalX; localX <= maxLocalX; localX += 1) {
          const columnOffset = localX + localZ * chunkArea;
          for (let localY = minLocalY; localY <= maxLocalY; localY += 1) {
            if (this.isLodChunkLocalVoxelVisible(chunk, columnOffset + localY * this.chunkSize)) {
              return true;
            }
          }
        }
      }
    }
    return false;
  }

  private lodChunkHasCoveredActiveCoarserOverlap(chunk: VoxelChunk): boolean {
    if (chunk.lodLevel <= 0 || chunk.solidCount === 0) {
      return false;
    }
    for (const coarser of this.lodChunks.values()) {
      if (coarser.lodLevel <= chunk.lodLevel || coarser.solidCount === 0) {
        continue;
      }
      if (this.lodChunksHaveCoveredColumnOverlap(coarser, chunk)) {
        return true;
      }
    }
    return false;
  }

  private prepareLodChunkBlockedByActiveCoarser(key: string, chunk: VoxelChunk): void {
    this.preparedLodChunks.set(key, chunk);
    const activeSameKey = this.lodChunks.get(key);
    if (
      activeSameKey?.renderReady
      && activeSameKey.solidCount > 0
      && activeSameKey.lodLevel === chunk.lodLevel
    ) {
      this.punchActiveCoarserLodChunksCoveredBy(activeSameKey);
    }
  }

  private punchActiveCoarserLodChunksCoveredByPreparedActiveFiner(): void {
    if (this.preparedLodChunks.size === 0) {
      return;
    }
    const coarserCandidateKeys = this.collectPreparedCoarserCandidateKeys();
    if (coarserCandidateKeys.size === 0) {
      return;
    }
    const activeFinerChunks = [...this.lodChunks.values()]
      .filter((chunk) => chunk.renderReady && chunk.lodLevel > 0 && chunk.solidCount > 0)
      .sort((left, right) => left.lodLevel - right.lodLevel);
    for (const coarserKey of coarserCandidateKeys) {
      const coarser = this.lodChunks.get(coarserKey);
      if (!coarser || coarser.solidCount === 0) {
        continue;
      }
      for (const finer of activeFinerChunks) {
        if (finer.lodLevel >= coarser.lodLevel) {
          continue;
        }
        if (
          lodChunkFootprintsOverlap(coarser, finer, this.chunkSize)
          && this.lodChunksHaveCoveredColumnOverlap(coarser, finer)
        ) {
          this.punchActiveLodChunkCoveredByFiner(coarserKey);
          break;
        }
      }
    }
  }

  private commitPreparedLodChunks(): void {
    if (this.preparedLodChunks.size === 0) {
      return;
    }
    this.commitPreparedSameKeyReplacements();
    let changed = true;
    while (changed) {
      changed = false;
      const coarserCandidateKeys = this.collectPreparedCoarserCandidateKeys();
      for (const [coarserKey, coarser] of [...this.lodChunks.entries()].sort((left, right) => right[1].lodLevel - left[1].lodLevel)) {
        if (!coarserCandidateKeys.has(coarserKey)) {
          continue;
        }
        if (coarser.lodLevel <= 0 || coarser.solidCount === 0) {
          continue;
        }
        if (!this.isLodChunkFullyCoveredByFinerCandidates(coarser)) {
          continue;
        }
        for (const [preparedKey, prepared] of [...this.preparedLodChunks.entries()]) {
          if (
            prepared.lodLevel < coarser.lodLevel &&
            lodChunkFootprintsOverlap(coarser, prepared, this.chunkSize) &&
            this.lodChunksHaveCoveredColumnOverlap(coarser, prepared)
          ) {
            this.preparedLodChunks.delete(preparedKey);
            this.lodChunks.set(preparedKey, prepared);
            this.lodWorldColumnCoverageCache.clear();
            this.staleLodKeys.delete(preparedKey);
            this.punchActiveCoarserLodChunksCoveredBy(prepared);
          }
        }
        this.lodChunks.delete(coarserKey);
        this.lodWorldColumnCoverageCache.clear();
        this.staleLodKeys.delete(coarserKey);
        changed = true;
      }

      for (const [preparedKey, prepared] of [...this.preparedLodChunks.entries()]) {
        if (this.lodChunkHasCoveredActiveCoarserOverlap(prepared)) {
          continue;
        }
        this.preparedLodChunks.delete(preparedKey);
        this.lodChunks.set(preparedKey, prepared);
        this.lodWorldColumnCoverageCache.clear();
        this.staleLodKeys.delete(preparedKey);
        this.punchActiveCoarserLodChunksCoveredBy(prepared);
        changed = true;
      }
    }
  }

  private commitPreparedSameKeyReplacements(): void {
    for (const [preparedKey, prepared] of [...this.preparedLodChunks.entries()]) {
      const activeSameKey = this.lodChunks.get(preparedKey);
      if (
        !activeSameKey?.renderReady
        || activeSameKey.lodLevel !== prepared.lodLevel
        || activeSameKey.voxelStride !== prepared.voxelStride
      ) {
        continue;
      }
      this.preparedLodChunks.delete(preparedKey);
      this.lodChunks.set(preparedKey, prepared);
      this.lodWorldColumnCoverageCache.clear();
      this.staleLodKeys.delete(preparedKey);
      this.punchActiveCoarserLodChunksCoveredBy(prepared);
    }
  }

  private collectPreparedCoarserCandidateKeys(): Set<string> {
    const candidates = new Set<string>();
    for (const prepared of this.preparedLodChunks.values()) {
      for (const [coarserKey, coarser] of this.lodChunks.entries()) {
        if (coarser.lodLevel <= prepared.lodLevel || coarser.solidCount === 0) {
          continue;
        }
        if (lodChunkFootprintsOverlap(coarser, prepared, this.chunkSize)) {
          candidates.add(coarserKey);
        }
      }
    }
    return candidates;
  }

  private isLodChunkFullyCoveredByFinerCandidates(coarser: VoxelChunk): boolean {
    const coarserStride = Math.max(1, coarser.voxelStride);
    const coarserWorldSize = this.chunkSize * coarserStride;
    for (let localZ = 0; localZ < this.chunkSize; localZ += 1) {
      for (let localX = 0; localX < this.chunkSize; localX += 1) {
        if (!this.lodChunkCoversLocalColumn(coarser, localX, localZ)) {
          continue;
        }
        const worldX = coarser.coord.x * coarserWorldSize + localX * coarserStride;
        const worldZ = coarser.coord.z * coarserWorldSize + localZ * coarserStride;
        if (!this.isWorldColumnCoveredByFinerLodCandidate(coarser.lodLevel, worldX, worldZ)) {
          return false;
        }
      }
    }
    return true;
  }

  private isWorldColumnCoveredByFinerLodCandidate(coarserLevel: number, worldX: number, worldZ: number): boolean {
    for (const chunk of this.lodChunks.values()) {
      if (chunk.lodLevel >= coarserLevel) {
        continue;
      }
      if (this.lodChunkCoversWorldColumn(chunk, worldX, worldZ)) {
        return true;
      }
    }
    for (const chunk of this.preparedLodChunks.values()) {
      if (chunk.lodLevel >= coarserLevel) {
        continue;
      }
      if (this.lodChunkCoversWorldColumn(chunk, worldX, worldZ)) {
        return true;
      }
    }
    return false;
  }

  private lodChunksHaveCoveredColumnOverlap(coarser: VoxelChunk, finer: VoxelChunk): boolean {
    if (!lodChunkFootprintsOverlap(coarser, finer, this.chunkSize)) {
      return false;
    }
    const finerStride = Math.max(1, finer.voxelStride);
    const finerWorldSize = this.chunkSize * finerStride;
    for (let localZ = 0; localZ < this.chunkSize; localZ += 1) {
      for (let localX = 0; localX < this.chunkSize; localX += 1) {
        if (!this.lodChunkCoversLocalColumn(finer, localX, localZ)) {
          continue;
        }
        const worldX = finer.coord.x * finerWorldSize + localX * finerStride;
        const worldZ = finer.coord.z * finerWorldSize + localZ * finerStride;
        if (this.lodChunkCoversWorldColumn(coarser, worldX, worldZ)) {
          return true;
        }
      }
    }
    return false;
  }

  private lodChunkCoversWorldColumn(chunk: VoxelChunk, worldX: number, worldZ: number): boolean {
    const stride = Math.max(1, chunk.voxelStride);
    const worldSize = this.chunkSize * stride;
    const localX = Math.floor((worldX - chunk.coord.x * worldSize) / stride);
    const localZ = Math.floor((worldZ - chunk.coord.z * worldSize) / stride);
    if (localX < 0 || localX >= this.chunkSize || localZ < 0 || localZ >= this.chunkSize) {
      return false;
    }
    return this.lodChunkCoversLocalColumn(chunk, localX, localZ);
  }

  private lodChunkCoversLocalColumn(chunk: VoxelChunk, localX: number, localZ: number): boolean {
    const chunkArea = this.chunkSize * this.chunkSize;
    const columnOffset = localX + localZ * chunkArea;
    for (let localY = 0; localY < this.chunkSize; localY += 1) {
      if (this.isLodChunkLocalVoxelVisible(chunk, columnOffset + localY * this.chunkSize)) {
        return true;
      }
    }
    return false;
  }

  private isLodChunkLocalVoxelVisible(chunk: VoxelChunk, localIndex: number): boolean {
    if (chunk.data[localIndex] === 0) {
      return false;
    }
    const clipMask = this.lodRenderClipMasks.get(toLodChunkKey(chunk.lodLevel, chunk.coord.x, chunk.coord.y, chunk.coord.z));
    return !clipMask || clipMask[localIndex] === 0;
  }

  private punchActiveCoarserLodChunksCoveredBy(
    finer: VoxelChunk,
    options: { includeAlreadyPunched?: boolean } = {},
  ): void {
    if (finer.lodLevel <= 0 || finer.solidCount === 0) {
      return;
    }
    const includeAlreadyPunched = options.includeAlreadyPunched ?? true;
    const finerStride = Math.max(1, finer.voxelStride);
    const finerWorldSize = this.chunkSize * finerStride;
    const minWorldX = finer.coord.x * finerWorldSize;
    const maxWorldXExclusive = (finer.coord.x + 1) * finerWorldSize;
    const minWorldZ = finer.coord.z * finerWorldSize;
    const maxWorldZExclusive = (finer.coord.z + 1) * finerWorldSize;
    for (let coarserLevel = finer.lodLevel + 1; coarserLevel <= MAX_LOD_LEVEL; coarserLevel += 1) {
      const coarserStride = 1 << coarserLevel;
      const coarserWorldSize = this.chunkSize * coarserStride;
      const minCx = Math.floor(minWorldX / coarserWorldSize);
      const maxCx = Math.floor((maxWorldXExclusive - 1) / coarserWorldSize);
      const minCz = Math.floor(minWorldZ / coarserWorldSize);
      const maxCz = Math.floor((maxWorldZExclusive - 1) / coarserWorldSize);
      const maxCyExclusive = Math.ceil(this.maxYExclusive / coarserWorldSize);
      for (let cz = minCz; cz <= maxCz; cz += 1) {
        for (let cx = minCx; cx <= maxCx; cx += 1) {
          for (let cy = 0; cy < maxCyExclusive; cy += 1) {
            const coarserKey = toLodChunkKey(coarserLevel, cx, cy, cz);
            const coarser = this.lodChunks.get(coarserKey);
            if (!coarser || coarser === finer || coarser.solidCount === 0) {
              continue;
            }
            if (!includeAlreadyPunched && this.coveragePunchedLodKeys.has(coarserKey)) {
              continue;
            }
            this.punchActiveLodChunkCoveredByFiner(coarserKey);
          }
        }
      }
    }
  }

  private punchLodChunkCoveredByActiveFiner(
    chunk: VoxelChunk,
  ): { chunk: VoxelChunk; skippedFinerCoverage: boolean; visibleSolidCount: number } {
    if (chunk.lodLevel <= 0 || chunk.solidCount === 0) {
      return { chunk, skippedFinerCoverage: false, visibleSolidCount: chunk.solidCount };
    }
    const stride = Math.max(1, chunk.voxelStride);
    const originX = chunk.coord.x * this.chunkSize * stride;
    const originY = chunk.coord.y * this.chunkSize * stride;
    const originZ = chunk.coord.z * this.chunkSize * stride;
    let skippedFinerCoverage = false;
    const chunkArea = this.chunkSize * this.chunkSize;
    let clipMask: Uint8Array | null = null;
    let visibleSolidCount = 0;
    for (let oz = 0; oz < this.chunkSize; oz += 1) {
      for (let ox = 0; ox < this.chunkSize; ox += 1) {
        if (!this.isOutputLodColumnCoveredByFiner(chunk.lodLevel, originX, originZ, stride, ox, oz)) {
          const columnOffset = ox + oz * chunkArea;
          for (let oy = 0; oy < this.chunkSize; oy += 1) {
            const index = columnOffset + oy * this.chunkSize;
            const material = chunk.data[index] ?? 0;
            if (material === 0) {
              continue;
            }
            visibleSolidCount += 1;
          }
          continue;
        }
        for (let oy = 0; oy < this.chunkSize; oy += 1) {
          const index = ox + oy * this.chunkSize + oz * chunkArea;
          const material = chunk.data[index] ?? 0;
          if (material === 0) {
            continue;
          }
          if (this.isOutputLodVoxelCoveredByFiner(chunk.lodLevel, originX, originY, originZ, stride, ox, oy, oz)) {
            skippedFinerCoverage = true;
            clipMask ??= new Uint8Array(chunk.data.length);
            clipMask[index] = 1;
            continue;
          }
          visibleSolidCount += 1;
        }
      }
    }
    if (!skippedFinerCoverage) {
      return { chunk, skippedFinerCoverage: false, visibleSolidCount: chunk.solidCount };
    }
    if (clipMask) {
      this.lodRenderClipMasks.set(toLodChunkKey(chunk.lodLevel, chunk.coord.x, chunk.coord.y, chunk.coord.z), clipMask);
    }
    return { chunk, skippedFinerCoverage: true, visibleSolidCount };
  }

  private punchActiveLodChunkCoveredByFiner(key: string): void {
    const chunk = this.lodChunks.get(key);
    if (!chunk || chunk.lodLevel <= 0 || chunk.solidCount === 0) {
      return;
    }
    const activation = this.punchLodChunkCoveredByActiveFiner(chunk);
    if (!activation.skippedFinerCoverage) {
      return;
    }
    if (activation.visibleSolidCount === 0) {
      this.lodChunks.delete(key);
      this.staleLodKeys.delete(key);
      this.clearLodCoveragePunchedKey(key);
      this.coveredEmptyLodKeys.add(key);
      return;
    }
    const stride = Math.max(1, activation.chunk.voxelStride);
    const worldSize = this.chunkSize * stride;
    if (!this.meshMaterialLut) {
      this.meshMaterialLut = createMeshMaterialLut(this.palette, isProceduralWaterMaterial);
    }
    activation.chunk.mesh = this.buildLodChunkMesh(
      activation.chunk,
      activation.chunk.coord.x,
      activation.chunk.coord.y,
      activation.chunk.coord.z,
      activation.chunk.lodLevel,
      stride,
      worldSize,
    );
    activation.chunk.meshBuilt = true;
    activation.chunk.meshDirty = false;
    activation.chunk.renderReady = true;
    activation.chunk.gpuDirty = true;
    this.lodChunks.set(key, activation.chunk);
    this.lodWorldColumnCoverageCache.clear();
    this.coveragePunchedLodKeys.add(key);
  }

  private clearLodCoveragePunchedKey(key: string): void {
    this.coveragePunchedLodKeys.delete(key);
    this.lodRenderClipMasks.delete(key);
    this.lodWorldColumnCoverageCache.clear();
  }

  private buildLodChunkMesh(
    chunk: VoxelChunk,
    cx: number,
    cy: number,
    cz: number,
    level: number,
    stride: number,
    worldSize: number,
  ): ChunkMeshData {
    const clipMask = this.lodRenderClipMasks.get(toLodChunkKey(level, cx, cy, cz)) ?? null;
    const opaqueMesh = buildOpaqueChunkMeshFromInput(
      {
        chunkSize: this.chunkSize,
        coord: { x: cx, y: cy, z: cz },
        chunkData: chunk.data,
        clipMask,
        solidCount: chunk.solidCount,
        solidBounds: chunk.solidBounds
          ? { min: [...chunk.solidBounds.min], max: [...chunk.solidBounds.max] }
          : null,
        neighbors: [
          [
            this.resolveLodOpaqueNeighborFace(level, cx - 1, cy, cz, 0, true),
            this.resolveLodOpaqueNeighborFace(level, cx + 1, cy, cz, 0, false),
          ],
          [
            this.resolveLodOpaqueNeighborFace(level, cx, cy - 1, cz, 1, true),
            this.resolveLodOpaqueNeighborFace(level, cx, cy + 1, cz, 1, false),
          ],
          [
            this.resolveLodOpaqueNeighborFace(level, cx, cy, cz - 1, 2, true),
            this.resolveLodOpaqueNeighborFace(level, cx, cy, cz + 1, 2, false),
          ],
        ],
      },
      this.meshMaterialLut!,
    );

    // Scale vertex positions from mesher-local coords to world coords.
    // The mesher places vertices at (cx*chunkSize .. (cx+1)*chunkSize).
    // We need them at (cx*worldSize .. (cx+1)*worldSize).
    // Opaque LOD voxels use a full-stride downward Y bias so a coarse cell
    // never renders above the detailed bucket it represents.
    if (stride > 1 && opaqueMesh.vertexCount > 0) {
      const mesherOriginX = cx * this.chunkSize;
      const mesherOriginY = cy * this.chunkSize;
      const mesherOriginZ = cz * this.chunkSize;
      const worldOriginX = cx * worldSize;
      const worldOriginY = cy * worldSize;
      const worldOriginZ = cz * worldSize;
      scaleLodVertexPositions(
        opaqueMesh.vertexData, opaqueMesh.vertexCount,
        mesherOriginX, mesherOriginY, mesherOriginZ,
        worldOriginX, worldOriginY, worldOriginZ,
        stride,
        stride,
      );
      if (opaqueMesh.bounds) {
        opaqueMesh.bounds.min[0] = worldOriginX + (opaqueMesh.bounds.min[0] - mesherOriginX) * stride;
        opaqueMesh.bounds.min[1] = worldOriginY + (opaqueMesh.bounds.min[1] - mesherOriginY) * stride - stride;
        opaqueMesh.bounds.min[2] = worldOriginZ + (opaqueMesh.bounds.min[2] - mesherOriginZ) * stride;
        opaqueMesh.bounds.max[0] = worldOriginX + (opaqueMesh.bounds.max[0] - mesherOriginX) * stride;
        opaqueMesh.bounds.max[1] = worldOriginY + (opaqueMesh.bounds.max[1] - mesherOriginY) * stride - stride;
        opaqueMesh.bounds.max[2] = worldOriginZ + (opaqueMesh.bounds.max[2] - mesherOriginZ) * stride;
      }
    }
    const waterMesh = this.buildLodWaterTopMesh(chunk, cx, cy, cz, stride, worldSize, clipMask);

    return {
      vertexData: opaqueMesh.vertexData,
      vertexCount: opaqueMesh.vertexCount,
      indexData: opaqueMesh.indexData,
      indexCount: opaqueMesh.indexCount,
      waterVertexData: waterMesh.vertexData,
      waterVertexCount: waterMesh.vertexCount,
      waterIndexData: waterMesh.indexData,
      waterIndexCount: waterMesh.indexCount,
      waterTriangleCount: waterMesh.triangleCount,
      triangleCount: opaqueMesh.triangleCount + waterMesh.triangleCount,
      bounds: combineLodMeshBounds(
        opaqueMesh.bounds,
        waterMesh.bounds,
        {
          min: [cx * worldSize, cy * worldSize, cz * worldSize],
          max: [(cx + 1) * worldSize, (cy + 1) * worldSize, (cz + 1) * worldSize],
        },
      ),
    };
  }

  private queueAdjacentLodChunkRemeshes(level: number, cx: number, cy: number, cz: number): void {
    for (const [dx, dy, dz] of ADJACENT_CHUNK_OFFSETS) {
      const neighborCx = cx + dx;
      const neighborCy = cy + dy;
      const neighborCz = cz + dz;
      const key = toLodChunkKey(level, neighborCx, neighborCy, neighborCz);
      const neighbor = this.lodChunks.get(key);
      if (!neighbor || !neighbor.renderReady || neighbor.solidCount === 0) {
        continue;
      }
      this.pendingLodNeighborRemeshKeys.add(key);
    }
  }

  private processPendingLodNeighborRemeshes(options: {
    maxMs: number;
    maxCount: number;
  }): { remeshed: number; meshMs: number } {
    if (this.pendingLodNeighborRemeshKeys.size === 0 || options.maxCount <= 0 || options.maxMs <= 0) {
      return { remeshed: 0, meshMs: 0 };
    }
    const startedAt = performance.now();
    let remeshed = 0;
    let meshMs = 0;
    for (const key of [...this.pendingLodNeighborRemeshKeys]) {
      if (
        remeshed >= options.maxCount
        || (remeshed > 0 && performance.now() - startedAt >= options.maxMs)
      ) {
        break;
      }
      this.pendingLodNeighborRemeshKeys.delete(key);
      const parsed = parseLodKey(key);
      if (!parsed) {
        continue;
      }
      const neighbor = this.lodChunks.get(key);
      if (!neighbor || !neighbor.renderReady || neighbor.solidCount === 0) {
        continue;
      }
      const stride = 1 << parsed.level;
      const worldSize = this.chunkSize * stride;
      const meshStartedAt = performance.now();
      neighbor.mesh = this.buildLodChunkMesh(neighbor, parsed.cx, parsed.cy, parsed.cz, parsed.level, stride, worldSize);
      meshMs += performance.now() - meshStartedAt;
      neighbor.meshBuilt = true;
      neighbor.meshDirty = false;
      neighbor.gpuDirty = true;
      remeshed += 1;
    }
    return { remeshed, meshMs };
  }

  private resolveLodOpaqueNeighborFace(
    level: number,
    cx: number,
    cy: number,
    cz: number,
    axis: number,
    negativeSide: boolean,
  ): OpaqueChunkNeighborFaceSnapshot {
    const key = toLodChunkKey(level, cx, cy, cz);
    const chunk = this.lodChunks.get(key);
    if (!chunk || chunk.solidCount === 0) {
      return { faceData: null, solidBounds: null };
    }
    return {
      faceData: extractLodNeighborFaceData(
        chunk.data,
        axis,
        negativeSide,
        this.chunkSize,
        this.lodRenderClipMasks.get(key) ?? null,
      ),
      solidBounds: chunk.solidBounds
        ? { min: [...chunk.solidBounds.min], max: [...chunk.solidBounds.max] }
        : null,
    };
  }

  private buildLodWaterTopMesh(
    chunk: VoxelChunk,
    cx: number,
    cy: number,
    cz: number,
    stride: number,
    worldSize: number,
    clipMask: Uint8Array | null,
  ): {
    vertexData: ArrayBuffer;
    vertexCount: number;
    indexData: Uint32Array;
    indexCount: number;
    triangleCount: number;
    bounds: { min: [number, number, number]; max: [number, number, number] } | null;
  } {
    const solidBounds = chunk.solidBounds;
    if (!solidBounds) {
      return createEmptyLodWaterMesh();
    }

    const cs = this.chunkSize;
    const chunkArea = cs * cs;
    const width = solidBounds.max[0] - solidBounds.min[0];
    const depth = solidBounds.max[2] - solidBounds.min[2];
    if (width <= 0 || depth <= 0) {
      return createEmptyLodWaterMesh();
    }

    const mask = new Uint16Array(width * depth);
    const quads: number[] = [];

    for (let y = solidBounds.min[1]; y < solidBounds.max[1]; y += 1) {
      let maskIndex = 0;
      for (let z = solidBounds.min[2]; z < solidBounds.max[2]; z += 1) {
        for (let x = solidBounds.min[0]; x < solidBounds.max[0]; x += 1) {
          const index = x + y * cs + z * chunkArea;
          const material = clipMask?.[index] ? 0 : chunk.data[index]!;
          if (!isProceduralWaterMaterial(material)) {
            mask[maskIndex++] = 0;
            continue;
          }
          const aboveIndex = x + (y + 1) * cs + z * chunkArea;
          const above = y + 1 < cs
            ? clipMask?.[aboveIndex]
              ? 0
              : chunk.data[aboveIndex]!
            : this.isLodWaterContinuingAboveChunk(cx, cy, cz, x, z, y, stride, worldSize)
              ? material
              : 0;
          mask[maskIndex++] = isProceduralWaterMaterial(above) ? 0 : material;
        }
      }

      maskIndex = 0;
      for (let row = 0; row < depth; row += 1) {
        for (let column = 0; column < width; ) {
          const material = mask[maskIndex]!;
          if (material === 0) {
            column += 1;
            maskIndex += 1;
            continue;
          }
          let quadWidth = 1;
          while (column + quadWidth < width && mask[maskIndex + quadWidth] === material) {
            quadWidth += 1;
          }
          let quadDepth = 1;
          let shouldGrow = true;
          while (row + quadDepth < depth && shouldGrow) {
            for (let offset = 0; offset < quadWidth; offset += 1) {
              if (mask[maskIndex + offset + quadDepth * width] !== material) {
                shouldGrow = false;
                break;
              }
            }
            if (shouldGrow) {
              quadDepth += 1;
            }
          }

          quads.push(
            solidBounds.min[0] + column,
            y + 1,
            solidBounds.min[2] + row,
            quadWidth,
            quadDepth,
            material,
          );

          for (let dz = 0; dz < quadDepth; dz += 1) {
            for (let dx = 0; dx < quadWidth; dx += 1) {
              mask[maskIndex + dx + dz * width] = 0;
            }
          }
          column += quadWidth;
          maskIndex += quadWidth;
        }
      }
    }

    const quadCount = quads.length / 6;
    if (quadCount === 0) {
      return createEmptyLodWaterMesh();
    }

    const vertexCount = quadCount * 4;
    const indexCount = quadCount * 6;
    const vertexData = new ArrayBuffer(vertexCount * LOD_VERTEX_STRIDE);
    const vertexFloatView = new Float32Array(vertexData);
    const vertexUintView = new Uint32Array(vertexData);
    const indexData = new Uint32Array(indexCount);
    const normal = packLodNormal(0, 1, 0);
    const worldOriginX = cx * worldSize;
    const worldOriginY = cy * worldSize;
    const worldOriginZ = cz * worldSize;
    let vertexWordOffset = 0;
    let indexOffset = 0;
    let baseVertex = 0;
    let minX = Infinity, minY = Infinity, minZ = Infinity;
    let maxX = -Infinity, maxY = -Infinity, maxZ = -Infinity;

    for (let quadIndex = 0; quadIndex < quads.length; quadIndex += 6) {
      const localX = quads[quadIndex]!;
      const localY = quads[quadIndex + 1]!;
      const localZ = quads[quadIndex + 2]!;
      const quadWidth = quads[quadIndex + 3]!;
      const quadDepth = quads[quadIndex + 4]!;
      const material = quads[quadIndex + 5]!;
      const color = this.getPaletteColor(material);
      const x0 = worldOriginX + localX * stride;
      const y = worldOriginY + localY * stride;
      const z0 = worldOriginZ + localZ * stride;
      const x1 = x0 + quadWidth * stride;
      const z1 = z0 + quadDepth * stride;
      minX = Math.min(minX, x0, x1);
      minY = Math.min(minY, y);
      minZ = Math.min(minZ, z0, z1);
      maxX = Math.max(maxX, x0, x1);
      maxY = Math.max(maxY, y);
      maxZ = Math.max(maxZ, z0, z1);

      writeLodWaterVertex(vertexFloatView, vertexUintView, vertexWordOffset, x0, y, z0, normal, color);
      vertexWordOffset += 5;
      writeLodWaterVertex(vertexFloatView, vertexUintView, vertexWordOffset, x1, y, z0, normal, color);
      vertexWordOffset += 5;
      writeLodWaterVertex(vertexFloatView, vertexUintView, vertexWordOffset, x1, y, z1, normal, color);
      vertexWordOffset += 5;
      writeLodWaterVertex(vertexFloatView, vertexUintView, vertexWordOffset, x0, y, z1, normal, color);
      vertexWordOffset += 5;

      indexData[indexOffset] = baseVertex;
      indexData[indexOffset + 1] = baseVertex + 1;
      indexData[indexOffset + 2] = baseVertex + 2;
      indexData[indexOffset + 3] = baseVertex;
      indexData[indexOffset + 4] = baseVertex + 2;
      indexData[indexOffset + 5] = baseVertex + 3;
      baseVertex += 4;
      indexOffset += 6;
    }

    return {
      vertexData,
      vertexCount,
      indexData,
      indexCount,
      triangleCount: quadCount * 2,
      bounds: { min: [minX, minY, minZ], max: [maxX, maxY, maxZ] },
    };
  }

  private isLodWaterContinuingAboveChunk(
    cx: number,
    cy: number,
    cz: number,
    localX: number,
    localZ: number,
    localY: number,
    stride: number,
    worldSize: number,
  ): boolean {
    const worldX = cx * worldSize + localX * stride;
    const worldZ = cz * worldSize + localZ * stride;
    const topPlaneY = cy * worldSize + (localY + 1) * stride;
    const column = this.generator.sampleSurfaceColumn(worldX, worldZ);
    return column.waterTopY !== null && column.waterTopY >= topPlaneY;
  }

  private computeChunkYRange(cx: number, cz: number): [number, number] {
    const columnKey = toColumnKey(cx, cz);
    const cached = this.columnChunkYRanges.get(columnKey);
    if (cached) {
      return [cached.minCy, cached.maxCy];
    }
    const chunkOriginX = cx * this.chunkSize;
    const chunkOriginZ = cz * this.chunkSize;
    let minSurfaceY = Number.POSITIVE_INFINITY;
    let maxSurfaceY = Number.NEGATIVE_INFINITY;
    let maxTopY = Number.NEGATIVE_INFINITY;
    let maxWaterTopY = Number.NEGATIVE_INFINITY;
    for (const [offsetX, offsetZ] of sampleOffsets(this.chunkSize)) {
      const column = this.generator.sampleColumn(chunkOriginX + offsetX, chunkOriginZ + offsetZ);
      minSurfaceY = Math.min(minSurfaceY, column.surfaceY);
      maxSurfaceY = Math.max(maxSurfaceY, column.surfaceY);
      maxTopY = Math.max(maxTopY, column.topY);
      maxWaterTopY = Math.max(maxWaterTopY, column.waterTopY ?? column.surfaceY);
    }
    const minWorldY = Math.max(this.minY, minSurfaceY - this.undergroundPaddingChunks * this.chunkSize);
    const maxWorldY = Math.min(
      this.maxYExclusive - 1,
      Math.max(maxSurfaceY, maxTopY, maxWaterTopY) + this.airPaddingChunks * this.chunkSize,
    );
    const range: [number, number] = [
      Math.max(0, Math.floor(minWorldY / this.chunkSize)),
      Math.max(0, Math.floor(maxWorldY / this.chunkSize)),
    ];
    this.columnChunkYRanges.set(columnKey, {
      minCy: range[0],
      maxCy: range[1],
    });
    return range;
  }

  private markAdjacentChunksDirty(cx: number, cy: number, cz: number): number {
    let touched = 0;
    for (const [dx, dy, dz] of ADJACENT_CHUNK_OFFSETS) {
      const chunk = this.getResidentChunk(cx + dx, cy + dy, cz + dz);
      if (!chunk) {
        continue;
      }
      markResidentChunkDirty(this, chunk);
      touched += 1;
    }
    return touched;
  }

  private adoptResidentChunk(key: string, chunk: VoxelChunk): void {
    this.chunks.set(key, chunk);
    this.noteResidentChunkMeshDirtyState(chunk, chunk.meshDirty);
    const columnKey = toColumnKey(chunk.coord.x, chunk.coord.z);
    const previous = this.residentColumnCounts.get(columnKey) ?? 0;
    this.residentColumnCounts.set(columnKey, previous + 1);
    if (chunk.renderReady) {
      this.renderReadyColumnCounts.set(columnKey, (this.renderReadyColumnCounts.get(columnKey) ?? 0) + 1);
    }
    this.syncRenderReadyColumnKey(columnKey);
    if (previous === 0) {
      this.residentColumnRevision += 1;
    }
    if (chunk.renderReady) {
      this.invalidateCoarserLodChunksForSourceChunk(0, chunk.coord.x, chunk.coord.y, chunk.coord.z);
    }
  }

  private evictResidentChunk(key: string, chunk: VoxelChunk): void {
    this.clearLodNeededKeyCache();
    this.chunks.delete(key);
    this.dirtyChunkKeys.delete(key);
    const columnKey = toColumnKey(chunk.coord.x, chunk.coord.z);
    if (chunk.renderReady) {
      const previousRenderReady = this.renderReadyColumnCounts.get(columnKey) ?? 0;
      if (previousRenderReady <= 1) {
        this.renderReadyColumnCounts.delete(columnKey);
      } else {
        this.renderReadyColumnCounts.set(columnKey, previousRenderReady - 1);
      }
    }
    const previous = this.residentColumnCounts.get(columnKey) ?? 0;
    if (previous <= 1) {
      this.residentColumnCounts.delete(columnKey);
      this.renderReadyColumnKeys.delete(columnKey);
      if (previous === 1) {
        this.residentColumnRevision += 1;
      }
    } else {
      this.residentColumnCounts.set(columnKey, previous - 1);
      this.syncRenderReadyColumnKey(columnKey);
    }
    this.invalidateCoarserLodChunksForSourceColumn(0, chunk.coord.x, chunk.coord.z);
  }

  noteResidentChunkRenderReadyState(chunk: VoxelChunk, renderReady: boolean): void {
    if (chunk.renderReady === renderReady) {
      return;
    }
    chunk.renderReady = renderReady;
    const columnKey = toColumnKey(chunk.coord.x, chunk.coord.z);
    if (renderReady) {
      this.renderReadyColumnCounts.set(columnKey, (this.renderReadyColumnCounts.get(columnKey) ?? 0) + 1);
    } else {
      const previous = this.renderReadyColumnCounts.get(columnKey) ?? 0;
      if (previous <= 1) {
        this.renderReadyColumnCounts.delete(columnKey);
      } else {
        this.renderReadyColumnCounts.set(columnKey, previous - 1);
      }
    }
    this.syncRenderReadyColumnKey(columnKey);
    this.invalidateCoarserLodChunksForSourceColumn(0, chunk.coord.x, chunk.coord.z);
  }

  private syncRenderReadyColumnKey(columnKey: string): void {
    const wasReady = this.renderReadyColumnKeys.has(columnKey);
    const residentCount = this.residentColumnCounts.get(columnKey) ?? 0;
    let isReady = false;
    if (residentCount === 0) {
      this.renderReadyColumnKeys.delete(columnKey);
    } else if ((this.renderReadyColumnCounts.get(columnKey) ?? 0) === residentCount) {
      this.renderReadyColumnKeys.add(columnKey);
      isReady = true;
    } else {
      this.renderReadyColumnKeys.delete(columnKey);
    }
    if (wasReady !== isReady) {
      this.renderReadyColumnRevision += 1;
      this.clearLodNeededKeyCache();
    }
  }

  private invalidateCoarserLodChunksForSourceColumn(
    sourceLevel: number,
    sourceCx: number,
    sourceCz: number,
    options: { clearNeededKeyCache?: boolean } = {},
  ): void {
    if (options.clearNeededKeyCache !== false) {
      this.clearLodNeededKeyCache();
    }
    const sourceStride = 1 << sourceLevel;
    const sourceWorldSize = this.chunkSize * sourceStride;
    const sourceWorldX = sourceCx * sourceWorldSize;
    const sourceWorldZ = sourceCz * sourceWorldSize;
    for (const ring of LOD_RINGS) {
      if (ring.level <= sourceLevel) {
        continue;
      }
      const stride = 1 << ring.level;
      const worldSize = this.chunkSize * stride;
      const lodCx = Math.floor(sourceWorldX / worldSize);
      const lodCz = Math.floor(sourceWorldZ / worldSize);
      const keyPrefix = `L${ring.level}:${lodCx}:`;
      for (const key of [...this.lodChunks.keys()]) {
        if (!key.startsWith(keyPrefix)) {
          continue;
        }
        const parts = key.split(":");
        if (Number.parseInt(parts[3]!, 10) === lodCz) {
          this.staleLodKeys.add(key);
          this.punchActiveLodChunkCoveredByFiner(key);
        }
      }
      for (const key of [...this.preparedLodChunks.keys()]) {
        if (!key.startsWith(keyPrefix)) {
          continue;
        }
        const parts = key.split(":");
        if (Number.parseInt(parts[3]!, 10) === lodCz) {
          this.preparedLodChunks.delete(key);
        }
      }
      for (const key of [...this.emptyLodKeys]) {
        if (!key.startsWith(keyPrefix)) {
          continue;
        }
        const parts = key.split(":");
        if (Number.parseInt(parts[3]!, 10) === lodCz) {
          this.emptyLodKeys.delete(key);
        }
      }
      for (const key of [...this.coveredEmptyLodKeys]) {
        if (!key.startsWith(keyPrefix)) {
          continue;
        }
        const parts = key.split(":");
        if (Number.parseInt(parts[3]!, 10) === lodCz) {
          this.coveredEmptyLodKeys.delete(key);
        }
      }
      for (const key of [...this.retainedLodChunks.keys()]) {
        if (!key.startsWith(keyPrefix)) {
          continue;
        }
        const parts = key.split(":");
        if (Number.parseInt(parts[3]!, 10) === lodCz) {
          this.deleteRetainedLodKey(key);
        }
      }
      for (const key of [...this.retainedEmptyLodKeys]) {
        if (!key.startsWith(keyPrefix)) {
          continue;
        }
        const parts = key.split(":");
        if (Number.parseInt(parts[3]!, 10) === lodCz) {
          this.deleteRetainedLodKey(key);
        }
      }
    }
  }

  private applyOverlayToGeneratedChunk(key: string, generated: GeneratedChunk): void {
    const overlay = this.editOverlays.get(key);
    if (!overlay || overlay.size === 0) {
      return;
    }
    if (generated.data.length === 0) {
      generated.data = new Uint16Array(this.chunkSize ** 3);
    }
    for (const [localIndex, material] of overlay) {
      generated.data[localIndex] = material;
    }
    recomputeGeneratedChunkSolidBounds(generated, this.chunkSize);
    generated.renderSummary = summarizeGeneratedChunkRender(
      generated.coord,
      generated.data,
      this.chunkSize,
      isProceduralWaterMaterial,
    );
  }

  private markChunkDirtyByCoord(cx: number, cy: number, cz: number): void {
    const chunk = this.getResidentChunk(cx, cy, cz);
    if (!chunk) {
      return;
    }
    markResidentChunkDirty(this, chunk);
  }

  private recordGeneratedChunkRenderSummary(key: string, generated: GeneratedChunk): void {
    this.recordChunkRenderSummary(key, generated.renderSummary);
  }

  private recordChunkRenderSummary(key: string, summary: GeneratedChunkRenderSummary): void {
    this.generatedRenderSummaries.set(key, summary);
    const columnKey = toColumnKey(summary.coord.x, summary.coord.z);
    const chunkKeys = this.generatedRenderChunkKeysByColumn.get(columnKey) ?? new Set<string>();
    chunkKeys.add(key);
    this.generatedRenderChunkKeysByColumn.set(columnKey, chunkKeys);
    this.rebuildColumnRenderSummary(summary.coord.x, summary.coord.z);
  }

  private recordPersistedRegionRenderSummary(region: GeneratedRenderSummaryRegion): void {
    this.missingGeneratedRenderSummaryRegionKeys.delete(toRegionSummaryKey(region.regionX, region.regionZ));
    for (const entry of region.columns) {
      this.recordPersistedColumnRenderSummary(entry.summary);
    }
  }

  private recordPersistedColumnRenderSummary(summary: GeneratedRenderColumnSummary): void {
    const columnKey = toColumnKey(summary.chunkX, summary.chunkZ);
    if (this.generatedRenderChunkKeysByColumn.has(columnKey)) {
      return;
    }
    this.generatedRenderColumnSummaries.set(columnKey, summary);
  }

  private recordResidentChunkRenderSummary(chunk: VoxelChunk): void {
    const key = toChunkKey(chunk.coord.x, chunk.coord.y, chunk.coord.z);
    this.recordGeneratedChunkRenderSummary(key, {
      coord: chunk.coord,
      data: chunk.data,
      solidCount: chunk.solidCount,
      solidBounds: chunk.solidBounds
        ? {
            min: [...chunk.solidBounds.min],
            max: [...chunk.solidBounds.max],
          }
        : null,
      renderSummary: summarizeGeneratedChunkRender(
        chunk.coord,
        chunk.data,
        this.chunkSize,
        isProceduralWaterMaterial,
      ),
    });
  }

  private rebuildColumnRenderSummary(cx: number, cz: number): void {
    const columnKey = toColumnKey(cx, cz);
    const chunkKeys = this.generatedRenderChunkKeysByColumn.get(columnKey);
    if (!chunkKeys || chunkKeys.size === 0) {
      this.generatedRenderColumnSummaries.delete(columnKey);
      return;
    }
    const summaries: GeneratedChunkRenderSummary[] = [];
    for (const chunkKey of chunkKeys) {
      const summary = this.generatedRenderSummaries.get(chunkKey);
      if (summary) {
        summaries.push(summary);
      }
    }
    const columnSummary = summarizeGeneratedRenderColumn(cx, cz, summaries, this.chunkSize);
    if (!columnSummary) {
      this.generatedRenderColumnSummaries.delete(columnKey);
      return;
    }
    this.generatedRenderColumnSummaries.set(columnKey, columnSummary);
  }

  private noteColumnChunkYRange(cx: number, cz: number, cy: number): void {
    const columnKey = toColumnKey(cx, cz);
    const existing = this.columnChunkYRanges.get(columnKey);
    if (!existing) {
      this.columnChunkYRanges.set(columnKey, { minCy: cy, maxCy: cy });
      return;
    }
    existing.minCy = Math.min(existing.minCy, cy);
    existing.maxCy = Math.max(existing.maxCy, cy);
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

function createLodLevelCounts(): number[] {
  return Array.from({ length: MAX_LOD_LEVEL + 1 }, () => 0);
}

function countLodChunksByLevel(chunks: Iterable<VoxelChunk>): number[] {
  const counts = createLodLevelCounts();
  for (const chunk of chunks) {
    if (chunk.lodLevel >= 0 && chunk.lodLevel < counts.length) {
      counts[chunk.lodLevel] += 1;
    }
  }
  return counts;
}

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
    offsets.push([value, -ring], [value, ring]);
  }
  for (let value = -ring + 1; value <= ring - 1; value += 1) {
    offsets.push([-ring, value], [ring, value]);
  }
  return offsets;
}

function spawnFootprintOffsets(): Array<[number, number]> {
  return [
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

function toLodChunkKey(level: number, cx: number, cy: number, cz: number): string {
  return `L${level}:${cx}:${cy}:${cz}`;
}

function lodChunkFootprintsOverlap(left: VoxelChunk, right: VoxelChunk, chunkSize: number): boolean {
  const leftStride = Math.max(1, left.voxelStride);
  const rightStride = Math.max(1, right.voxelStride);
  const leftWorldSize = chunkSize * leftStride;
  const rightWorldSize = chunkSize * rightStride;
  const leftMinX = left.coord.x * leftWorldSize;
  const leftMaxX = (left.coord.x + 1) * leftWorldSize;
  const leftMinZ = left.coord.z * leftWorldSize;
  const leftMaxZ = (left.coord.z + 1) * leftWorldSize;
  const rightMinX = right.coord.x * rightWorldSize;
  const rightMaxX = (right.coord.x + 1) * rightWorldSize;
  const rightMinZ = right.coord.z * rightWorldSize;
  const rightMaxZ = (right.coord.z + 1) * rightWorldSize;
  return leftMinX < rightMaxX && leftMaxX > rightMinX && leftMinZ < rightMaxZ && leftMaxZ > rightMinZ;
}

function lodFootprintIntersectsRadius(
  centerX: number,
  centerZ: number,
  cx: number,
  cz: number,
  worldSize: number,
  radius: number,
): boolean {
  const minX = cx * worldSize;
  const maxX = (cx + 1) * worldSize;
  const minZ = cz * worldSize;
  const maxZ = (cz + 1) * worldSize;
  const closestX = Math.min(maxX, Math.max(minX, centerX));
  const closestZ = Math.min(maxZ, Math.max(minZ, centerZ));
  const dx = closestX - centerX;
  const dz = closestZ - centerZ;
  return dx * dx + dz * dz <= radius * radius;
}

function lodFootprintFullyInsideRadius(
  centerX: number,
  centerZ: number,
  cx: number,
  cz: number,
  worldSize: number,
  radius: number,
): boolean {
  const minX = cx * worldSize;
  const maxX = (cx + 1) * worldSize;
  const minZ = cz * worldSize;
  const maxZ = (cz + 1) * worldSize;
  const farthestX = Math.max(Math.abs(minX - centerX), Math.abs(maxX - centerX));
  const farthestZ = Math.max(Math.abs(minZ - centerZ), Math.abs(maxZ - centerZ));
  return farthestX * farthestX + farthestZ * farthestZ <= radius * radius;
}

function toLocalVoxelIndex(
  x: number,
  y: number,
  z: number,
  cx: number,
  cy: number,
  cz: number,
  chunkSize: number,
): number {
  const lx = x - cx * chunkSize;
  const ly = y - cy * chunkSize;
  const lz = z - cz * chunkSize;
  return lx + ly * chunkSize + lz * chunkSize * chunkSize;
}

function toColumnKey(cx: number, cz: number): string {
  return `${cx}:${cz}`;
}

function toRegionSummaryKey(regionX: number, regionZ: number): string {
  return `${regionX}:${regionZ}`;
}

function extractLodNeighborFaceData(
  data: Uint16Array,
  axis: number,
  negativeSide: boolean,
  chunkSize: number,
  clipMask: Uint8Array | null = null,
): Uint16Array {
  const chunkArea = chunkSize * chunkSize;
  const faceData = new Uint16Array(chunkArea);
  if (axis === 0) {
    const localX = negativeSide ? chunkSize - 1 : 0;
    for (let z = 0; z < chunkSize; z += 1) {
      const sourcePlaneOffset = localX + z * chunkArea;
      const faceRowOffset = z * chunkSize;
      for (let y = 0; y < chunkSize; y += 1) {
        const sourceIndex = sourcePlaneOffset + y * chunkSize;
        faceData[y + faceRowOffset] = clipMask?.[sourceIndex] ? 0 : data[sourceIndex]!;
      }
    }
    return faceData;
  }
  if (axis === 1) {
    const localY = negativeSide ? chunkSize - 1 : 0;
    const sourceRowOffset = localY * chunkSize;
    for (let z = 0; z < chunkSize; z += 1) {
      const sourcePlaneOffset = z * chunkArea + sourceRowOffset;
      const faceRowOffset = z * chunkSize;
      for (let x = 0; x < chunkSize; x += 1) {
        const sourceIndex = sourcePlaneOffset + x;
        faceData[x + faceRowOffset] = clipMask?.[sourceIndex] ? 0 : data[sourceIndex]!;
      }
    }
    return faceData;
  }
  const localZ = negativeSide ? chunkSize - 1 : 0;
  const sourcePlaneOffset = localZ * chunkArea;
  for (let y = 0; y < chunkSize; y += 1) {
    const sourceRowOffset = sourcePlaneOffset + y * chunkSize;
    const faceRowOffset = y * chunkSize;
    for (let x = 0; x < chunkSize; x += 1) {
      const sourceIndex = sourceRowOffset + x;
      faceData[x + faceRowOffset] = clipMask?.[sourceIndex] ? 0 : data[sourceIndex]!;
    }
  }
  return faceData;
}

function createResidentChunk(generated: GeneratedChunk, chunkSize: number): VoxelChunk {
  const chunk: VoxelChunk = {
    coord: generated.coord,
    lodLevel: 0,
    voxelStride: 1,
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
    renderReady: false,
    meshRevision: 1,
    pendingMeshRevision: null,
    gpuDirty: true,
    mesh: null,
  };
  if (chunk.solidCount > 0 && !chunk.solidBounds) {
    recomputeChunkSolidBounds(chunk, chunkSize);
  }
  return chunk;
}

function markResidentChunkDirty(world: MutableResidentChunkWorld, chunk: VoxelChunk): void {
  setChunkMeshDirtyState(world, chunk, true);
  chunk.meshRevision += 1;
  chunk.pendingMeshRevision = null;
}

function updateResidentChunkVoxel(
  chunk: VoxelChunk,
  localIndex: number,
  lx: number,
  ly: number,
  lz: number,
  materialIndex: number,
): void {
  const previous = chunk.data[localIndex] ?? 0;
  if (previous === materialIndex) {
    return;
  }
  chunk.data[localIndex] = materialIndex;
  if (previous === 0 && materialIndex !== 0) {
    chunk.solidCount += 1;
    expandResidentChunkSolidBounds(chunk, lx, ly, lz);
  } else if (previous !== 0 && materialIndex === 0) {
    chunk.solidCount -= 1;
    invalidateResidentChunkBoundsIfNeeded(chunk, lx, ly, lz);
  }
}

function expandResidentChunkSolidBounds(chunk: VoxelChunk, lx: number, ly: number, lz: number): void {
  if (!chunk.solidBounds) {
    chunk.solidBounds = {
      min: [lx, ly, lz],
      max: [lx + 1, ly + 1, lz + 1],
      dirty: false,
    };
    return;
  }
  chunk.solidBounds.min[0] = Math.min(chunk.solidBounds.min[0], lx);
  chunk.solidBounds.min[1] = Math.min(chunk.solidBounds.min[1], ly);
  chunk.solidBounds.min[2] = Math.min(chunk.solidBounds.min[2], lz);
  chunk.solidBounds.max[0] = Math.max(chunk.solidBounds.max[0], lx + 1);
  chunk.solidBounds.max[1] = Math.max(chunk.solidBounds.max[1], ly + 1);
  chunk.solidBounds.max[2] = Math.max(chunk.solidBounds.max[2], lz + 1);
}

function invalidateResidentChunkBoundsIfNeeded(chunk: VoxelChunk, lx: number, ly: number, lz: number): void {
  if (!chunk.solidBounds) {
    return;
  }
  if (chunk.solidCount === 0) {
    chunk.solidBounds = null;
    return;
  }
  if (
    lx === chunk.solidBounds.min[0]
    || ly === chunk.solidBounds.min[1]
    || lz === chunk.solidBounds.min[2]
    || lx + 1 === chunk.solidBounds.max[0]
    || ly + 1 === chunk.solidBounds.max[1]
    || lz + 1 === chunk.solidBounds.max[2]
  ) {
    chunk.solidBounds.dirty = true;
  }
}

function zeroResidencyPhaseMetrics(): ResidencyPhaseMetrics {
  return {
    surfaceSampleMs: 0,
    yRangeMs: 0,
    chunkGenerationMs: 0,
    chunkDispatchMs: 0,
    chunkDrainMs: 0,
    summaryDrainMs: 0,
    chunkAdoptionMs: 0,
    evictionMs: 0,
    neighborDirtyMs: 0,
    inFlightChunks: 0,
    completedChunkCacheHits: 0,
    completedGeneratedChunks: 0,
    completedSummaryCacheHits: 0,
    completedGeneratedSummaries: 0,
    completedRegionSummaryCacheHits: 0,
    missingRegionSummaries: 0,
    readyGeneratedChunkBacklog: 0,
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

function recomputeGeneratedChunkSolidBounds(chunk: GeneratedChunk, chunkSize: number): void {
  let solidCount = 0;
  let minX = chunkSize;
  let minY = chunkSize;
  let minZ = chunkSize;
  let maxX = 0;
  let maxY = 0;
  let maxZ = 0;
  const chunkArea = chunkSize * chunkSize;

  for (let lz = 0; lz < chunkSize; lz += 1) {
    for (let ly = 0; ly < chunkSize; ly += 1) {
      for (let lx = 0; lx < chunkSize; lx += 1) {
        const localIndex = lx + ly * chunkSize + lz * chunkArea;
        if (chunk.data[localIndex] === 0) {
          continue;
        }
        solidCount += 1;
        minX = Math.min(minX, lx);
        minY = Math.min(minY, ly);
        minZ = Math.min(minZ, lz);
        maxX = Math.max(maxX, lx + 1);
        maxY = Math.max(maxY, ly + 1);
        maxZ = Math.max(maxZ, lz + 1);
      }
    }
  }

  chunk.solidCount = solidCount;
  chunk.solidBounds = solidCount === 0
    ? null
    : {
        min: [minX, minY, minZ],
        max: [maxX, maxY, maxZ],
      };
}

function createLodResidentChunk(generated: GeneratedChunk, lodLevel: number, voxelStride: number): VoxelChunk {
  return {
    coord: generated.coord,
    lodLevel,
    voxelStride,
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
    meshDirty: false,
    renderReady: false,
    meshRevision: 1,
    pendingMeshRevision: null,
    gpuDirty: true,
    mesh: null,
  };
}

function estimateColumnSummaryWorldYRange(
  summary: GeneratedRenderColumnSummary,
  chunkSize: number,
): { minY: number; maxY: number } | null {
  let minY = Number.POSITIVE_INFINITY;
  let maxY = Number.NEGATIVE_INFINITY;
  if (summary.minNonEmptyCy !== null && summary.maxNonEmptyCy !== null) {
    minY = Math.min(minY, summary.minNonEmptyCy * chunkSize);
    maxY = Math.max(maxY, (summary.maxNonEmptyCy + 1) * chunkSize - 1);
  }
  for (let index = 0; index < summary.surfaceY.length; index += 1) {
    const surfaceY = summary.surfaceY[index] ?? NO_GENERATED_SURFACE_HEIGHT;
    if (surfaceY !== NO_GENERATED_SURFACE_HEIGHT) {
      minY = Math.min(minY, surfaceY);
      maxY = Math.max(maxY, surfaceY);
    }
    const waterTopY = summary.waterTopY[index] ?? NO_GENERATED_WATER_HEIGHT;
    if (waterTopY !== NO_GENERATED_WATER_HEIGHT) {
      maxY = Math.max(maxY, waterTopY);
    }
  }
  if (!Number.isFinite(minY) || !Number.isFinite(maxY)) {
    return null;
  }
  return { minY, maxY };
}

function parseLodKey(key: string): { level: number; cx: number; cy: number; cz: number } | null {
  const parts = key.split(":");
  if (parts.length !== 4 || !parts[0]?.startsWith("L")) {
    return null;
  }
  const level = Number.parseInt(parts[0].slice(1), 10);
  const cx = Number.parseInt(parts[1]!, 10);
  const cy = Number.parseInt(parts[2]!, 10);
  const cz = Number.parseInt(parts[3]!, 10);
  if ([level, cx, cy, cz].some((value) => !Number.isFinite(value))) {
    return null;
  }
  return { level, cx, cy, cz };
}

function scaleLodVertexPositions(
  vertexData: ArrayBuffer,
  vertexCount: number,
  mesherOriginX: number,
  mesherOriginY: number,
  mesherOriginZ: number,
  worldOriginX: number,
  worldOriginY: number,
  worldOriginZ: number,
  stride: number,
  yBias: number,
): void {
  const floats = new Float32Array(vertexData);
  for (let i = 0; i < vertexCount; i++) {
    const base = i * 5; // 20 bytes = 5 float32s per vertex
    floats[base + 0] = worldOriginX + (floats[base + 0]! - mesherOriginX) * stride;
    floats[base + 1] = worldOriginY + (floats[base + 1]! - mesherOriginY) * stride - yBias;
    floats[base + 2] = worldOriginZ + (floats[base + 2]! - mesherOriginZ) * stride;
  }
}

function createEmptyLodWaterMesh(): {
  vertexData: ArrayBuffer;
  vertexCount: number;
  indexData: Uint32Array;
  indexCount: number;
  triangleCount: number;
  bounds: null;
} {
  return {
    vertexData: new ArrayBuffer(0),
    vertexCount: 0,
    indexData: new Uint32Array(0),
    indexCount: 0,
    triangleCount: 0,
    bounds: null,
  };
}

function writeLodWaterVertex(
  floatView: Float32Array,
  uintView: Uint32Array,
  wordOffset: number,
  x: number,
  y: number,
  z: number,
  packedNormal: number,
  color: number,
): void {
  floatView[wordOffset] = x;
  floatView[wordOffset + 1] = y;
  floatView[wordOffset + 2] = z;
  uintView[wordOffset + 3] = packedNormal;
  uintView[wordOffset + 4] = color;
}

function packLodNormal(normalX: number, normalY: number, normalZ: number): number {
  const packedX = (normalX * LOD_NORMAL_SCALE) & 0xff;
  const packedY = (normalY * LOD_NORMAL_SCALE) & 0xff;
  const packedZ = (normalZ * LOD_NORMAL_SCALE) & 0xff;
  return (packedX | (packedY << 8) | (packedZ << 16) | (LOD_NORMAL_SCALE << 24)) >>> 0;
}

function combineLodMeshBounds(
  opaqueBounds: { min: [number, number, number]; max: [number, number, number] } | null,
  waterBounds: { min: [number, number, number]; max: [number, number, number] } | null,
  fallbackBounds: { min: [number, number, number]; max: [number, number, number] },
): { min: [number, number, number]; max: [number, number, number] } {
  if (!opaqueBounds && !waterBounds) {
    return fallbackBounds;
  }
  if (!opaqueBounds) {
    return waterBounds!;
  }
  if (!waterBounds) {
    return opaqueBounds;
  }
  return {
    min: [
      Math.min(opaqueBounds.min[0], waterBounds.min[0]),
      Math.min(opaqueBounds.min[1], waterBounds.min[1]),
      Math.min(opaqueBounds.min[2], waterBounds.min[2]),
    ],
    max: [
      Math.max(opaqueBounds.max[0], waterBounds.max[0]),
      Math.max(opaqueBounds.max[1], waterBounds.max[1]),
      Math.max(opaqueBounds.max[2], waterBounds.max[2]),
    ],
  };
}
