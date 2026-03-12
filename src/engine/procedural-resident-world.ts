import type { ChunkCoordinate, Vec3, WorldStats } from "./types.ts";
import type { AsyncChunkGenerationQueue } from "./async-chunk-generation.ts";
import {
  ProceduralWorldGenerator,
  isProceduralWaterMaterial,
  type GeneratedChunk,
} from "./procedural-generator.ts";
import type { FarFieldColumnSample, FarFieldSource } from "./far-field-source.ts";
import type { FarFieldExclusionMask } from "./procedural-far-field.ts";
import {
  NO_GENERATED_WATER_HEIGHT,
  summarizeGeneratedChunkRender,
  type GeneratedChunkRenderSummary,
} from "./generated-chunk-render-summary.ts";
import {
  sampleGeneratedRenderColumnSummary,
  summarizeGeneratedRenderColumn,
  type GeneratedRenderColumnSummary,
} from "./generated-render-column-summary.ts";
import {
  GENERATED_RENDER_SUMMARY_REGION_SIZE_CHUNKS,
  getGeneratedRenderSummaryRegionCoord,
  type GeneratedRenderSummaryRegion,
} from "./generated-render-summary-region.ts";
import { metersToWorldUnits } from "./scale.ts";
import type { MutableResidentChunkWorld, VoxelChunk } from "./world.ts";

const DEFAULT_HORIZONTAL_RADIUS_CHUNKS = 8;
const DEFAULT_UNDERGROUND_PADDING_CHUNKS = 3;
const DEFAULT_AIR_PADDING_CHUNKS = 2;
const FAR_FIELD_SUMMARY_DISCOVERY_VERTICAL_PADDING_CHUNKS = 1;
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

export class ProceduralResidentWorld implements MutableResidentChunkWorld, FarFieldSource {
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
  private readonly editOverlays = new Map<string, Map<number, number>>();
  private readonly editLog: WorldEditRecord[] = [];
  private readonly residentColumnCounts = new Map<string, number>();
  private readonly generatedRenderSummaries = new Map<string, GeneratedChunkRenderSummary>();
  private readonly generatedRenderChunkKeysByColumn = new Map<string, Set<string>>();
  private readonly generatedRenderColumnSummaries = new Map<string, GeneratedRenderColumnSummary>();
  private readonly pendingFarFieldColumnRanges = new Map<string, ChunkYRange>();
  private readonly missingPersistedFarFieldRegionSummaries = new Set<string>();
  private readonly asyncChunkGeneration: AsyncChunkGenerationQueue | null;
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
    return countDirtyResidentChunks(this.chunks.values());
  }

  hasResidentColumn(cx: number, cz: number): boolean {
    return (this.residentColumnCounts.get(toColumnKey(cx, cz)) ?? 0) > 0;
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
    if (minChunkX === maxChunkX && minChunkZ === maxChunkZ) {
      return this.hasResidentColumn(minChunkX, minChunkZ);
    }
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

  isCollisionMaterial(materialIndex: number): boolean {
    return materialIndex !== 0 && !isProceduralWaterMaterial(materialIndex);
  }

  isWaterMaterial(materialIndex: number): boolean {
    return isProceduralWaterMaterial(materialIndex);
  }

  sampleFarFieldColumn(worldX: number, worldZ: number): FarFieldColumnSample | null {
    const voxelX = Math.floor(worldX);
    const voxelZ = Math.floor(worldZ);
    const cx = Math.floor(voxelX / this.chunkSize);
    const cz = Math.floor(voxelZ / this.chunkSize);
    const columnSummary = this.generatedRenderColumnSummaries.get(toColumnKey(cx, cz));
    if (!columnSummary) {
      return null;
    }
    const localX = voxelX - cx * this.chunkSize;
    const localZ = voxelZ - cz * this.chunkSize;
    const sampled = sampleGeneratedRenderColumnSummary(columnSummary, localX, localZ, this.chunkSize);
    if (!sampled) {
      return null;
    }
    let waterTopY = sampled.waterTopY ?? NO_GENERATED_WATER_HEIGHT;
    let waterMaterial = sampled.waterMaterial ?? 0;
    const surfaceY = sampled.surfaceY;
    const surfaceMaterial = sampled.surfaceMaterial;
    if (waterTopY <= surfaceY) {
      waterTopY = NO_GENERATED_WATER_HEIGHT;
      waterMaterial = 0;
    }
    return {
      surfaceY,
      surfaceMaterial,
      waterTopY: waterTopY === NO_GENERATED_WATER_HEIGHT ? null : waterTopY,
      waterMaterial: waterTopY === NO_GENERATED_WATER_HEIGHT ? null : waterMaterial,
    };
  }

  getFarFieldChunkSummary(cx: number, cy: number, cz: number): GeneratedChunkRenderSummary | null {
    return this.generatedRenderSummaries.get(toChunkKey(cx, cy, cz)) ?? null;
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
    this.emptyChunkKeys.delete(key);

    const residentChunk = this.getResidentChunk(cx, cy, cz);
    if (residentChunk) {
      updateResidentChunkVoxel(residentChunk, localIndex, lx, ly, lz, materialIndex);
      markResidentChunkDirty(residentChunk);
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
          return [worldX + 0.5, candidate.standY, worldZ + 0.5];
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
    return fallbackPosition;
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
    const completedGeneratedChunksByKey = new Map<string, GeneratedChunk>();
    for (const summary of completedRegionSummaries) {
      this.recordPersistedRegionRenderSummary(summary);
    }
    for (const coord of missingRegionSummaries) {
      this.missingPersistedFarFieldRegionSummaries.add(toRegionKey(coord.x, coord.z));
    }
    for (const summary of completedRenderSummaries) {
      this.recordChunkRenderSummary(toChunkKey(summary.coord.x, summary.coord.y, summary.coord.z), summary);
    }
    for (const generated of completedGeneratedChunks) {
      this.recordGeneratedChunkRenderSummary(toChunkKey(generated.coord.x, generated.coord.y, generated.coord.z), generated);
      completedGeneratedChunksByKey.set(toChunkKey(generated.coord.x, generated.coord.y, generated.coord.z), generated);
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
        const completed = completedGeneratedChunksByKey.get(key);
        if (completed) {
          completedGeneratedChunksByKey.delete(key);
          this.applyOverlayToGeneratedChunk(key, completed);
          if (completed.solidCount === 0) {
            emptyChunksSkipped += 1;
            this.emptyChunkKeys.add(key);
            continue;
          }
          const adoptionStartedAt = performance.now();
          const chunk = createResidentChunk(completed, this.chunkSize);
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

    for (const [key, generated] of completedGeneratedChunksByKey) {
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
      dirtyResidentChunks: countDirtyResidentChunks(this.chunks.values()),
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
      },
    };
    return this.lastResidency;
  }

  prefetchFarFieldSummariesAround(
    position: Vec3,
    outerRadiusWorldUnits: number,
    maxGenerateChunks: number,
  ): number {
    if (maxGenerateChunks <= 0) {
      return 0;
    }
    const centerChunkX = Math.floor(position[0] / this.chunkSize);
    const centerChunkZ = Math.floor(position[2] / this.chunkSize);
    const outerRadiusChunks = Math.max(this.horizontalRadiusChunks, Math.ceil(outerRadiusWorldUnits / this.chunkSize));
    const residentRadiusSquared = this.horizontalRadiusChunks * this.horizontalRadiusChunks;
    let requestedChunks = 0;

    for (const [dx, dz] of prioritizedColumnOffsets(outerRadiusChunks)) {
      if (requestedChunks >= maxGenerateChunks) {
        break;
      }
      if (dx * dx + dz * dz <= residentRadiusSquared) {
        continue;
      }
      const cx = centerChunkX + dx;
      const cz = centerChunkZ + dz;
      if (!this.generatedRenderColumnSummaries.has(toColumnKey(cx, cz)) && !this.pendingFarFieldColumnRanges.has(toColumnKey(cx, cz))) {
        this.requestPersistedFarFieldRegionSummaryForColumn(cx, cz);
      }
      const estimatedRange = this.estimateFarFieldSummaryChunkYRange(cx, cz);
      if (!estimatedRange) {
        continue;
      }
      const { minCy, maxCy } = estimatedRange;
      for (const cy of prioritizeFarFieldSurfaceChunkYRange(minCy, maxCy)) {
        if (requestedChunks >= maxGenerateChunks) {
          break;
        }
        const key = toChunkKey(cx, cy, cz);
        if (
          this.chunks.has(key)
          || this.generatedRenderSummaries.has(key)
          || this.emptyChunkKeys.has(key)
          || this.asyncChunkGeneration?.hasPendingChunk(cx, cy, cz)
        ) {
          continue;
        }
        this.notePendingFarFieldColumnRange(cx, cz, cy);
        if (this.asyncChunkGeneration) {
          if (this.asyncChunkGeneration.requestSummary(cx, cy, cz)) {
            requestedChunks += 1;
          }
          continue;
        }
        const generated = this.generator.generateChunk(cx, cy, cz);
        this.applyOverlayToGeneratedChunk(key, generated);
        this.recordGeneratedChunkRenderSummary(key, generated);
        if (generated.solidCount === 0) {
          this.emptyChunkKeys.add(key);
        }
        requestedChunks += 1;
      }
    }

    return requestedChunks;
  }

  private computeChunkYRange(cx: number, cz: number): [number, number] {
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
    return [
      Math.max(0, Math.floor(minWorldY / this.chunkSize)),
      Math.max(0, Math.floor(maxWorldY / this.chunkSize)),
    ];
  }

  private estimateFarFieldSummaryChunkYRange(cx: number, cz: number): ChunkYRange | null {
    const columnKey = toColumnKey(cx, cz);
    let minCy = Number.POSITIVE_INFINITY;
    let maxCy = Number.NEGATIVE_INFINITY;

    const existingRange = this.getKnownFarFieldColumnRange(cx, cz);
    if (existingRange) {
      minCy = Math.min(minCy, existingRange.minCy);
      maxCy = Math.max(maxCy, existingRange.maxCy);
    }

    for (const [offsetX, offsetZ] of FAR_FIELD_DISCOVERY_NEIGHBOR_OFFSETS) {
      const neighborRange = this.getKnownFarFieldColumnRange(cx + offsetX, cz + offsetZ);
      if (!neighborRange) {
        continue;
      }
      minCy = Math.min(minCy, neighborRange.minCy - FAR_FIELD_SUMMARY_DISCOVERY_VERTICAL_PADDING_CHUNKS);
      maxCy = Math.max(maxCy, neighborRange.maxCy + FAR_FIELD_SUMMARY_DISCOVERY_VERTICAL_PADDING_CHUNKS);
    }

    const pendingRange = this.pendingFarFieldColumnRanges.get(columnKey);
    if (pendingRange) {
      minCy = Math.min(minCy, pendingRange.minCy);
      maxCy = Math.max(maxCy, pendingRange.maxCy);
    }

    if (!Number.isFinite(minCy) || !Number.isFinite(maxCy)) {
      return null;
    }
    return {
      minCy: clampChunkY(Math.min(minCy, maxCy), this.maxYExclusive, this.chunkSize),
      maxCy: clampChunkY(Math.max(minCy, maxCy), this.maxYExclusive, this.chunkSize),
    };
  }

  private markAdjacentChunksDirty(cx: number, cy: number, cz: number): number {
    let touched = 0;
    for (const [dx, dy, dz] of ADJACENT_CHUNK_OFFSETS) {
      const chunk = this.getResidentChunk(cx + dx, cy + dy, cz + dz);
      if (!chunk) {
        continue;
      }
      markResidentChunkDirty(chunk);
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
      if (!chunk.meshBuilt || !chunk.mesh) {
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
    markResidentChunkDirty(chunk);
  }

  private recordGeneratedChunkRenderSummary(key: string, generated: GeneratedChunk): void {
    this.recordChunkRenderSummary(key, generated.renderSummary);
  }

  private recordChunkRenderSummary(key: string, summary: GeneratedChunkRenderSummary): void {
    this.generatedRenderSummaries.set(key, summary);
    const columnKey = toColumnKey(summary.coord.x, summary.coord.z);
    const regionCoord = getGeneratedRenderSummaryRegionCoord(
      summary.coord.x,
      summary.coord.z,
      GENERATED_RENDER_SUMMARY_REGION_SIZE_CHUNKS,
    );
    this.missingPersistedFarFieldRegionSummaries.delete(toRegionKey(regionCoord.x, regionCoord.z));
    const chunkKeys = this.generatedRenderChunkKeysByColumn.get(columnKey) ?? new Set<string>();
    chunkKeys.add(key);
    this.generatedRenderChunkKeysByColumn.set(columnKey, chunkKeys);
    this.rebuildColumnRenderSummary(summary.coord.x, summary.coord.z);
  }

  private recordPersistedRegionRenderSummary(region: GeneratedRenderSummaryRegion): void {
    this.missingPersistedFarFieldRegionSummaries.delete(toRegionKey(region.regionX, region.regionZ));
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

  private getKnownFarFieldColumnRange(cx: number, cz: number): ChunkYRange | null {
    const summary = this.generatedRenderColumnSummaries.get(toColumnKey(cx, cz));
    if (summary) {
      const preferredMinCy = summary.minNonEmptyCy ?? summary.minKnownCy;
      const preferredMaxCy = summary.maxNonEmptyCy ?? summary.maxKnownCy;
      return {
        minCy: preferredMinCy,
        maxCy: preferredMaxCy,
      };
    }
    return this.pendingFarFieldColumnRanges.get(toColumnKey(cx, cz)) ?? null;
  }

  private notePendingFarFieldColumnRange(cx: number, cz: number, cy: number): void {
    const columnKey = toColumnKey(cx, cz);
    const existing = this.pendingFarFieldColumnRanges.get(columnKey);
    if (!existing) {
      this.pendingFarFieldColumnRanges.set(columnKey, { minCy: cy, maxCy: cy });
      return;
    }
    existing.minCy = Math.min(existing.minCy, cy);
    existing.maxCy = Math.max(existing.maxCy, cy);
  }

  private requestPersistedFarFieldRegionSummaryForColumn(cx: number, cz: number): void {
    if (!this.asyncChunkGeneration) {
      return;
    }
    const regionCoord = getGeneratedRenderSummaryRegionCoord(cx, cz, GENERATED_RENDER_SUMMARY_REGION_SIZE_CHUNKS);
    const regionKey = toRegionKey(regionCoord.x, regionCoord.z);
    if (
      this.missingPersistedFarFieldRegionSummaries.has(regionKey)
      || this.asyncChunkGeneration.hasPendingRegionSummary(regionCoord.x, regionCoord.z)
    ) {
      return;
    }
    this.asyncChunkGeneration.requestRegionSummary(regionCoord.x, regionCoord.z);
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
const FAR_FIELD_DISCOVERY_NEIGHBOR_OFFSETS: ReadonlyArray<readonly [number, number]> = [
  [-1, -1],
  [0, -1],
  [1, -1],
  [-1, 0],
  [1, 0],
  [-1, 1],
  [0, 1],
  [1, 1],
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

function clampChunkY(cy: number, maxYExclusive: number, chunkSize: number): number {
  return Math.max(0, Math.min(Math.floor((maxYExclusive - 1) / chunkSize), cy));
}

function prioritizeFarFieldSurfaceChunkYRange(minCy: number, maxCy: number): number[] {
  if (maxCy < minCy) {
    return [];
  }
  const ordered: number[] = [minCy];
  for (let cy = maxCy; cy > minCy; cy -= 1) {
    ordered.push(cy);
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

function toRegionKey(regionX: number, regionZ: number): string {
  return `${regionX}:${regionZ}`;
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
  if (minChunkX === maxChunkX && minChunkZ === maxChunkZ) {
    return columnKeys.has(toColumnKey(minChunkX, minChunkZ));
  }
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

function markResidentChunkDirty(chunk: VoxelChunk): void {
  chunk.meshDirty = true;
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
