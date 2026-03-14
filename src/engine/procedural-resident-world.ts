import type { ChunkCoordinate, ChunkMeshData, Vec3, WorldStats } from "./types.ts";
import type { AsyncChunkGenerationQueue } from "./async-chunk-generation.ts";
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
  summarizeGeneratedChunkRender,
  type GeneratedChunkRenderSummary,
} from "./generated-chunk-render-summary.ts";
import {
  summarizeGeneratedRenderColumn,
  type GeneratedRenderColumnSummary,
} from "./generated-render-column-summary.ts";
import {
  type GeneratedRenderSummaryRegion,
} from "./generated-render-summary-region.ts";
import { metersToWorldUnits } from "./scale.ts";
import { setChunkMeshDirtyState, type MutableResidentChunkWorld, type VoxelChunk } from "./world.ts";

const DEFAULT_HORIZONTAL_RADIUS_CHUNKS = 8;
const DEFAULT_UNDERGROUND_PADDING_CHUNKS = 3;
const DEFAULT_AIR_PADDING_CHUNKS = 2;

const LOD_RINGS = [
  { level: 1, radiusChunks: 8 },
  { level: 2, radiusChunks: 5 },
  { level: 3, radiusChunks: 4 },
  { level: 4, radiusChunks: 3 },
] as const;
const SPAWN_FOOTPRINT_RADIUS = metersToWorldUnits(0.8);
const SPAWN_SCAN_DEPTH = metersToWorldUnits(3.2);
const SPAWN_MAX_SURFACE_DROP = metersToWorldUnits(1.2);
const SPAWN_HEADROOM = metersToWorldUnits(1.8);

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

interface ChunkYRange {
  minCy: number;
  maxCy: number;
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
  private readonly emptyLodKeys = new Set<string>();
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

  *iterateResidentChunks(): Iterable<VoxelChunk> {
    for (const chunk of this.lodChunks.values()) {
      yield chunk;
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
    inFlightChunks = this.asyncChunkGeneration?.getPendingCount() ?? 0;
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
    options?: { maxGenerateLodChunks?: number },
  ): { generated: number; pending: number } {
    const maxGenerate = options?.maxGenerateLodChunks ?? 4;

    if (!this.meshMaterialLut) {
      this.meshMaterialLut = createMeshMaterialLut(this.palette, isProceduralWaterMaterial);
    }

    // Phase 1: compute all needed LOD chunk keys.
    //
    // LOD chunks are excluded in two cases:
    // 1. For LOD 1: excluded when ALL LOD 0 columns within the chunk's
    //    footprint are render-ready. This uses actual state, not theoretical
    //    radius, so there are never holes during loading.
    // 2. For LOD 2+: excluded when fully inside the previous LOD ring's
    //    coverage (those rings are co-generated, always complete).
    //
    // LOD chunks use a depth-biased pipeline so where they DO overlap with
    // LOD 0 at boundaries, LOD 0 always wins cleanly. No z-fighting.
    const neededKeys = new Set<string>();

    // Between LOD rings, track coverage from finer LOD rings.
    // Start from 0 — LOD 1 uses render-ready checks, not coverage half-width.
    let coveredHalfWidth = 0;

    for (const ring of LOD_RINGS) {
      const stride = 1 << ring.level;
      const worldSize = this.chunkSize * stride;
      const lcx = Math.floor(position[0] / worldSize);
      const lcz = Math.floor(position[2] / worldSize);
      const offsetInChunkX = position[0] - lcx * worldSize;
      const offsetInChunkZ = position[2] - lcz * worldSize;

      for (let dz = -ring.radiusChunks; dz <= ring.radiusChunks; dz += 1) {
        for (let dx = -ring.radiusChunks; dx <= ring.radiusChunks; dx += 1) {
          const cx = lcx + dx;
          const cz = lcz + dz;

          if (ring.level === 1) {
            // LOD 1: exclude only if ALL LOD 0 columns within this chunk
            // are render-ready (actually meshed, not just in-radius).
            if (this.isLodChunkFullyCoveredByRenderReadyColumns(cx, cz, stride)) {
              continue;
            }
          } else if (coveredHalfWidth > 0) {
            // LOD 2+: exclude if fully inside the finer LOD ring's coverage
            const chunkMinX = dx * worldSize - offsetInChunkX;
            const chunkMaxX = (dx + 1) * worldSize - offsetInChunkX;
            const chunkMinZ = dz * worldSize - offsetInChunkZ;
            const chunkMaxZ = (dz + 1) * worldSize - offsetInChunkZ;
            const farthestX = Math.max(Math.abs(chunkMinX), Math.abs(chunkMaxX));
            const farthestZ = Math.max(Math.abs(chunkMinZ), Math.abs(chunkMaxZ));
            if (farthestX <= coveredHalfWidth && farthestZ <= coveredHalfWidth) {
              continue;
            }
          }

          // Estimate Y range: sample multiple points across the LOD column
          // to avoid missing terrain that's between the center sample point
          const points = [
            [cx * worldSize, cz * worldSize],
            [cx * worldSize + worldSize - 1, cz * worldSize],
            [cx * worldSize, cz * worldSize + worldSize - 1],
            [cx * worldSize + worldSize - 1, cz * worldSize + worldSize - 1],
            [cx * worldSize + (worldSize >> 1), cz * worldSize + (worldSize >> 1)],
          ];
          let minSurfaceY = Infinity;
          let maxSurfaceY = -Infinity;
          let maxTopY = -Infinity;
          let maxWaterY = -Infinity;
          for (const [px, pz] of points) {
            const col = this.generator.sampleColumn(px!, pz!);
            if (col.surfaceY < minSurfaceY) minSurfaceY = col.surfaceY;
            if (col.surfaceY > maxSurfaceY) maxSurfaceY = col.surfaceY;
            if (col.topY > maxTopY) maxTopY = col.topY;
            if ((col.waterTopY ?? col.surfaceY) > maxWaterY) maxWaterY = col.waterTopY ?? col.surfaceY;
          }
          const minWorldY = Math.max(0, minSurfaceY - this.undergroundPaddingChunks * this.chunkSize);
          const maxWorldY = Math.min(
            this.maxYExclusive - 1,
            Math.max(maxSurfaceY, maxTopY, maxWaterY)
              + this.airPaddingChunks * this.chunkSize,
          );
          const minCy = Math.max(0, Math.floor(minWorldY / worldSize));
          const maxCy = Math.max(0, Math.floor(maxWorldY / worldSize));

          for (let cy = minCy; cy <= maxCy; cy += 1) {
            neededKeys.add(`L${ring.level}:${cx}:${cy}:${cz}`);
          }
        }
      }
      // Accumulate coverage from this ring for excluding coarser rings
      coveredHalfWidth = Math.max(
        coveredHalfWidth,
        ring.radiusChunks * worldSize + Math.max(offsetInChunkX, worldSize - offsetInChunkX),
      );
    }

    // Phase 2: downsample and mesh LOD chunks with a budget.
    // Process levels in ascending order so LOD N-1 is available when building LOD N.
    let generated = 0;
    let pending = 0;
    for (const ring of LOD_RINGS) {
      const stride = 1 << ring.level;
      const worldSize = this.chunkSize * stride;
      const keyPrefix = `L${ring.level}:`;
      for (const key of neededKeys) {
        if (!key.startsWith(keyPrefix)) continue;
        if (this.lodChunks.has(key) || this.emptyLodKeys.has(key)) continue;
        if (generated >= maxGenerate) {
          pending++;
          continue;
        }
        const parts = key.split(":");
        const cx = parseInt(parts[1]!);
        const cy = parseInt(parts[2]!);
        const cz = parseInt(parts[3]!);

        const data = this.downsampleLodChunkData(cx, cy, cz, ring.level, stride, worldSize);
        if (!data || data.solidCount === 0) {
          this.emptyLodKeys.add(key);
          generated++;
          continue;
        }
        const chunk = createLodResidentChunk(
          { coord: { x: cx, y: cy, z: cz }, data: data.data, solidCount: data.solidCount, solidBounds: data.solidBounds, renderSummary: null! },
          ring.level, stride,
        );
        const mesh = this.buildLodChunkMesh(chunk, cx, cy, cz, stride, worldSize);
        chunk.mesh = mesh;
        chunk.meshBuilt = true;
        chunk.meshDirty = false;
        chunk.renderReady = true;
        chunk.gpuDirty = true;
        this.lodChunks.set(key, chunk);
        generated++;
      }
    }

    // Phase 3: evict LOD chunks no longer needed
    for (const key of this.lodChunks.keys()) {
      if (!neededKeys.has(key)) {
        this.lodChunks.delete(key);
      }
    }
    for (const key of this.emptyLodKeys) {
      if (!neededKeys.has(key)) {
        this.emptyLodKeys.delete(key);
      }
    }

    return { generated, pending };
  }

  private downsampleLodChunkData(
    lodCx: number,
    lodCy: number,
    lodCz: number,
    level: number,
    stride: number,
    worldSize: number,
  ): { data: Uint16Array; solidCount: number; solidBounds: { min: [number, number, number]; max: [number, number, number] } | null } | null {
    const cs = this.chunkSize;
    const chunkArea = cs * cs;
    const data = new Uint16Array(cs * chunkArea);

    // World-space origin of this LOD chunk
    const originX = lodCx * worldSize;
    const originY = lodCy * worldSize;
    const originZ = lodCz * worldSize;

    // Source lookup: LOD 1 reads from LOD 0 chunks, LOD 2+ from LOD (level-1) chunks.
    const sourceStride = level === 1 ? 1 : (1 << (level - 1));
    const sourceWorldSize = cs * sourceStride;
    const lookupSource = level === 1
      ? (cx: number, cy: number, cz: number) => this.chunks.get(toChunkKey(cx, cy, cz))?.data ?? null
      : (cx: number, cy: number, cz: number) => this.lodChunks.get(`L${level - 1}:${cx}:${cy}:${cz}`)?.data ?? null;

    let solidCount = 0;
    let minX = cs, minY = cs, minZ = cs;
    let maxX = 0, maxY = 0, maxZ = 0;

    // The downsample factor relative to source: always 2 (each LOD level halves resolution)
    const ds = 2;

    for (let oz = 0; oz < cs; oz++) {
      for (let oy = 0; oy < cs; oy++) {
        for (let ox = 0; ox < cs; ox++) {
          // World position of this output voxel
          const wx = originX + ox * stride;
          const wy = originY + oy * stride;
          const wz = originZ + oz * stride;

          // Find the source chunk and local coordinates
          const srcCx = Math.floor(wx / sourceWorldSize);
          const srcCy = Math.floor(wy / sourceWorldSize);
          const srcCz = Math.floor(wz / sourceWorldSize);
          const srcData = lookupSource(srcCx, srcCy, srcCz);

          if (!srcData) {
            // Source chunk not available — treat as air
            continue;
          }

          // Local coordinate within source chunk
          const srcLx = Math.floor((wx - srcCx * sourceWorldSize) / sourceStride);
          const srcLy = Math.floor((wy - srcCy * sourceWorldSize) / sourceStride);
          const srcLz = Math.floor((wz - srcCz * sourceWorldSize) / sourceStride);

          // Downsample: scan the ds×ds×ds block starting at the source local coord,
          // pick highest opaque voxel (top-down Y scan)
          let bestMaterial = 0;
          outer: for (let dy = ds - 1; dy >= 0; dy--) {
            for (let dz = 0; dz < ds; dz++) {
              for (let dx = 0; dx < ds; dx++) {
                const lx = srcLx + dx;
                const ly = srcLy + dy;
                const lz = srcLz + dz;
                if (lx >= cs || ly >= cs || lz >= cs) continue;
                const mat = srcData[lx + ly * cs + lz * chunkArea]!;
                if (mat !== 0 && !isProceduralWaterMaterial(mat)) {
                  bestMaterial = mat;
                  break outer;
                }
                if (mat !== 0 && bestMaterial === 0) {
                  bestMaterial = mat; // water fallback
                }
              }
            }
          }

          if (bestMaterial !== 0) {
            data[ox + oy * cs + oz * chunkArea] = bestMaterial;
            solidCount++;
            if (ox < minX) minX = ox;
            if (oy < minY) minY = oy;
            if (oz < minZ) minZ = oz;
            if (ox + 1 > maxX) maxX = ox + 1;
            if (oy + 1 > maxY) maxY = oy + 1;
            if (oz + 1 > maxZ) maxZ = oz + 1;
          }
        }
      }
    }

    if (solidCount === 0) return null;
    return {
      data,
      solidCount,
      solidBounds: { min: [minX, minY, minZ], max: [maxX, maxY, maxZ] },
    };
  }

  private invalidateLodChunksAt(worldX: number, worldY: number, worldZ: number): void {
    for (const [key, chunk] of this.lodChunks) {
      const stride = chunk.voxelStride;
      const worldSize = this.chunkSize * stride;
      const minX = chunk.coord.x * worldSize;
      const minY = chunk.coord.y * worldSize;
      const minZ = chunk.coord.z * worldSize;
      if (worldX >= minX && worldX < minX + worldSize &&
          worldY >= minY && worldY < minY + worldSize &&
          worldZ >= minZ && worldZ < minZ + worldSize) {
        // Remove so it gets regenerated on next updateLodResidencyAround
        this.lodChunks.delete(key);
        this.emptyLodKeys.delete(key);
      }
    }
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

  private buildLodChunkMesh(
    chunk: VoxelChunk,
    cx: number,
    cy: number,
    cz: number,
    stride: number,
    worldSize: number,
  ): ChunkMeshData {
    const nullFace: OpaqueChunkNeighborFaceSnapshot = { faceData: null, solidBounds: null };
    const opaqueMesh = buildOpaqueChunkMeshFromInput(
      {
        chunkSize: this.chunkSize,
        coord: { x: cx, y: cy, z: cz },
        chunkData: chunk.data,
        solidCount: chunk.solidCount,
        solidBounds: chunk.solidBounds
          ? { min: [...chunk.solidBounds.min], max: [...chunk.solidBounds.max] }
          : null,
        neighbors: [
          [nullFace, nullFace],
          [nullFace, nullFace],
          [nullFace, nullFace],
        ],
      },
      this.meshMaterialLut!,
    );

    // Scale vertex positions from mesher-local coords to world coords.
    // The mesher places vertices at (cx*chunkSize .. (cx+1)*chunkSize).
    // We need them at (cx*worldSize .. (cx+1)*worldSize).
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
      );
      if (opaqueMesh.bounds) {
        opaqueMesh.bounds.min[0] = worldOriginX + (opaqueMesh.bounds.min[0] - mesherOriginX) * stride;
        opaqueMesh.bounds.min[1] = worldOriginY + (opaqueMesh.bounds.min[1] - mesherOriginY) * stride;
        opaqueMesh.bounds.min[2] = worldOriginZ + (opaqueMesh.bounds.min[2] - mesherOriginZ) * stride;
        opaqueMesh.bounds.max[0] = worldOriginX + (opaqueMesh.bounds.max[0] - mesherOriginX) * stride;
        opaqueMesh.bounds.max[1] = worldOriginY + (opaqueMesh.bounds.max[1] - mesherOriginY) * stride;
        opaqueMesh.bounds.max[2] = worldOriginZ + (opaqueMesh.bounds.max[2] - mesherOriginZ) * stride;
      }
    }

    return {
      vertexData: opaqueMesh.vertexData,
      vertexCount: opaqueMesh.vertexCount,
      indexData: opaqueMesh.indexData,
      indexCount: opaqueMesh.indexCount,
      waterVertexData: new ArrayBuffer(0),
      waterVertexCount: 0,
      waterIndexData: new Uint32Array(0),
      waterIndexCount: 0,
      waterTriangleCount: 0,
      triangleCount: opaqueMesh.triangleCount,
      bounds: opaqueMesh.bounds ?? {
        min: [cx * worldSize, cy * worldSize, cz * worldSize],
        max: [(cx + 1) * worldSize, (cy + 1) * worldSize, (cz + 1) * worldSize],
      },
    };
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
  }

  private evictResidentChunk(key: string, chunk: VoxelChunk): void {
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
      return;
    }
    this.residentColumnCounts.set(columnKey, previous - 1);
    this.syncRenderReadyColumnKey(columnKey);
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
  }

  private syncRenderReadyColumnKey(columnKey: string): void {
    const residentCount = this.residentColumnCounts.get(columnKey) ?? 0;
    if (residentCount === 0) {
      this.renderReadyColumnKeys.delete(columnKey);
      return;
    }
    if ((this.renderReadyColumnCounts.get(columnKey) ?? 0) === residentCount) {
      this.renderReadyColumnKeys.add(columnKey);
      return;
    }
    this.renderReadyColumnKeys.delete(columnKey);
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
): void {
  const floats = new Float32Array(vertexData);
  for (let i = 0; i < vertexCount; i++) {
    const base = i * 5; // 20 bytes = 5 float32s per vertex
    floats[base + 0] = worldOriginX + (floats[base + 0]! - mesherOriginX) * stride;
    floats[base + 1] = worldOriginY + (floats[base + 1]! - mesherOriginY) * stride;
    floats[base + 2] = worldOriginZ + (floats[base + 2]! - mesherOriginZ) * stride;
  }
}
