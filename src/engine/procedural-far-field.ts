import { metersToWorldUnits, worldUnitsToMeters } from "./scale.ts";
import { NO_GENERATED_SURFACE_HEIGHT } from "./generated-chunk-render-summary.ts";
import type { FarFieldSource } from "./far-field-source.ts";
import type { ChunkMeshData, Vec3 } from "./types.ts";
import { applyWaterDepthTint } from "./water-visuals.ts";

const FLOAT32_BYTES = 4;
const NORMAL_SCALE = 127;
const VERTEX_STRIDE = 20;
const NO_WATER_HEIGHT = -0x7fff_ffff;
const UNKNOWN_BOUNDARY_MIN_SURFACE_Y = 0x7fff_ffff;
const MASKED_SEAM_SKIRT_FACTOR = 0.5;
const MIN_MASKED_SEAM_SKIRT_DEPTH = metersToWorldUnits(0.2);

interface FarFieldBandConfig {
  readonly label: string;
  readonly innerRadius: number;
  readonly outerRadius: number;
  readonly sampleStride: number;
  readonly anchorStride: number;
  readonly centerStride?: number;
}

export interface FarFieldExclusionMask {
  revision: number;
  maxAffectedRadiusWorldUnits?: number;
  excludesCell(minX: number, maxXExclusive: number, minZ: number, maxZExclusive: number): boolean;
}

export interface FarFieldBandRenderable {
  readonly label: string;
  readonly innerRadius: number;
  readonly outerRadius: number;
  readonly sampleStride: number;
  readonly centerStride: number;
  anchorX: number;
  anchorZ: number;
  centerX: number;
  centerZ: number;
  clearRadiusWorldUnits: number;
  maskRevision: number;
  mesh: ChunkMeshData | null;
  gpuDirty: boolean;
  triangleCount: number;
}

interface FarFieldBandSampleCache {
  readonly anchorX: number;
  readonly anchorZ: number;
  readonly sampleRadius: number;
  readonly sampleSpan: number;
  readonly heights: Int32Array;
  readonly colors: Uint32Array;
  readonly waterHeights: Int32Array;
  readonly waterColors: Uint32Array;
  readonly eastBoundaryMinSurfaceY: Int32Array;
  readonly southBoundaryMinSurfaceY: Int32Array;
}

interface FarFieldBandState extends FarFieldBandRenderable {
  sampleCache: FarFieldBandSampleCache | null;
}

interface MeshBuilder {
  vertexBuffer: ArrayBuffer;
  vertexView: DataView;
  indexData: Uint32Array;
  vertexCount: number;
  indexCount: number;
  minX: number;
  minY: number;
  minZ: number;
  maxX: number;
  maxY: number;
  maxZ: number;
}

export interface FarFieldCoverage {
  readonly label: string;
  readonly sampleStride: number;
  readonly innerRadiusWorldUnits: number;
  readonly outerRadiusWorldUnits: number;
}

export interface FarFieldUpdateSummary {
  changed: boolean;
  builtBands: number;
  pendingBands: number;
  meshCount: number;
  triangleCount: number;
  elapsedMs: number;
  sampledCellCount: number;
  sampleCacheMs: number;
  meshBuildMs: number;
  clearRadiusWorldUnits: number;
  clearRadiusMeters: number;
  maxRadiusWorldUnits: number;
  maxRadiusMeters: number;
  bandBuilds: FarFieldBandBuildSummary[];
}

export interface FarFieldBandBuildSummary {
  label: string;
  sampledCellCount: number;
  sampleCacheMs: number;
  meshBuildMs: number;
  elapsedMs: number;
  triangleCount: number;
}

export interface FarFieldMaskedSeamProbe {
  boundaryCount: number;
  gapCount: number;
  maxGapDepthWorldUnits: number;
  maxGapDepthMeters: number;
  samples: Array<{
    band: string;
    direction: "east" | "west" | "south" | "north";
    worldX: number;
    worldZ: number;
    gapDepthWorldUnits: number;
    gapDepthMeters: number;
  }>;
}

export interface FarFieldSurfaceGapProbe {
  boundaryCount: number;
  gapCount: number;
  maxGapDepthWorldUnits: number;
  maxGapDepthMeters: number;
  samples: Array<{
    band: string;
    kind: "masked" | "downward";
    direction: "east" | "west" | "south" | "north";
    worldX: number;
    worldZ: number;
    gapDepthWorldUnits: number;
    gapDepthMeters: number;
  }>;
}

const DEFAULT_BANDS: readonly FarFieldBandConfig[] = [
  {
    label: "near-transition",
    innerRadius: 0,
    outerRadius: metersToWorldUnits(48),
    sampleStride: metersToWorldUnits(0.4),
    anchorStride: metersToWorldUnits(25.6),
    centerStride: metersToWorldUnits(25.6),
  },
  {
    label: "transition",
    innerRadius: metersToWorldUnits(48),
    outerRadius: metersToWorldUnits(96),
    sampleStride: metersToWorldUnits(0.8),
    anchorStride: metersToWorldUnits(25.6),
    centerStride: metersToWorldUnits(25.6),
  },
  {
    label: "mid",
    innerRadius: metersToWorldUnits(96),
    outerRadius: metersToWorldUnits(160),
    sampleStride: metersToWorldUnits(1.6),
    anchorStride: metersToWorldUnits(51.2),
    centerStride: metersToWorldUnits(51.2),
  },
  {
    label: "far",
    innerRadius: metersToWorldUnits(160),
    outerRadius: metersToWorldUnits(272),
    sampleStride: metersToWorldUnits(3.2),
    anchorStride: metersToWorldUnits(102.4),
    centerStride: metersToWorldUnits(102.4),
  },
  {
    label: "horizon",
    innerRadius: metersToWorldUnits(272),
    outerRadius: metersToWorldUnits(416),
    sampleStride: metersToWorldUnits(6.4),
    anchorStride: metersToWorldUnits(204.8),
    centerStride: metersToWorldUnits(204.8),
  },
] as const;

export class ProceduralFarField {
  private readonly bands: FarFieldBandState[];
  lastUpdate: FarFieldUpdateSummary;
  private readonly bandConfigs: readonly FarFieldBandConfig[];

  constructor(
    readonly source: FarFieldSource,
    bandConfigs: readonly FarFieldBandConfig[] = DEFAULT_BANDS,
  ) {
    this.bandConfigs = bandConfigs;
    this.bands = bandConfigs.map((config) => ({
      label: config.label,
      innerRadius: config.innerRadius,
      outerRadius: config.outerRadius,
      sampleStride: config.sampleStride,
      centerStride: config.centerStride ?? config.sampleStride,
      anchorX: Number.NaN,
      anchorZ: Number.NaN,
      centerX: Number.NaN,
      centerZ: Number.NaN,
      clearRadiusWorldUnits: Number.NaN,
      maskRevision: -1,
      mesh: null,
      gpuDirty: false,
      triangleCount: 0,
      sampleCache: null,
    }));
    this.lastUpdate = {
      changed: false,
      builtBands: 0,
      pendingBands: 0,
      meshCount: this.bands.length,
      triangleCount: 0,
      elapsedMs: 0,
      sampledCellCount: 0,
      sampleCacheMs: 0,
      meshBuildMs: 0,
      clearRadiusWorldUnits: 0,
      clearRadiusMeters: 0,
      maxRadiusWorldUnits: Math.max(...bandConfigs.map((config) => config.outerRadius)),
      maxRadiusMeters: worldUnitsToMeters(Math.max(...bandConfigs.map((config) => config.outerRadius))),
      bandBuilds: [],
    };
  }

  getRenderables(): readonly FarFieldBandRenderable[] {
    return this.bands;
  }

  getMaxRadiusWorldUnits(): number {
    return Math.max(...this.bands.map((band) => band.outerRadius));
  }

  updateAround(
    position: Vec3,
    clearRadiusWorldUnits = 0,
    exclusionMask: FarFieldExclusionMask | null = null,
    maxBuiltBands = Number.POSITIVE_INFINITY,
  ): FarFieldUpdateSummary {
    const startedAt = performance.now();
    let changed = false;
    let builtBands = 0;
    let pendingBands = 0;
    let triangleCount = 0;
    let sampledCellCount = 0;
    let sampleCacheMs = 0;
    let meshBuildMs = 0;
    const bandBuilds: FarFieldBandBuildSummary[] = [];
    const maxAffectedRadiusWorldUnits = exclusionMask?.maxAffectedRadiusWorldUnits ?? Number.POSITIVE_INFINITY;
    for (let bandIndex = 0; bandIndex < this.bands.length; bandIndex += 1) {
      const band = this.bands[bandIndex]!;
      const config = this.bandConfigs[bandIndex]!;
      const anchorX = snapToStride(Math.floor(position[0]), config.anchorStride);
      const anchorZ = snapToStride(Math.floor(position[2]), config.anchorStride);
      const centerStride = config.centerStride ?? config.sampleStride;
      const centerX = snapToNearestStride(position[0], centerStride);
      const centerZ = snapToNearestStride(position[2], centerStride);
      const effectiveInnerRadius = Math.max(config.innerRadius, clearRadiusWorldUnits);
      const maskRevision = effectiveInnerRadius < maxAffectedRadiusWorldUnits
        ? exclusionMask?.revision ?? 0
        : 0;
      if (
        band.anchorX === anchorX
        && band.anchorZ === anchorZ
        && band.centerX === centerX
        && band.centerZ === centerZ
        && band.clearRadiusWorldUnits === effectiveInnerRadius
        && band.maskRevision === maskRevision
        && band.mesh
      ) {
        triangleCount += band.triangleCount;
        continue;
      }
      if (builtBands >= maxBuiltBands && band.mesh) {
        pendingBands += 1;
        triangleCount += band.triangleCount;
        continue;
      }
      band.anchorX = anchorX;
      band.anchorZ = anchorZ;
      band.centerX = centerX;
      band.centerZ = centerZ;
      band.clearRadiusWorldUnits = effectiveInnerRadius;
      band.maskRevision = maskRevision;
      const bandStartedAt = performance.now();
      const sampleCacheStartedAt = performance.now();
      const sampleCache = ensureBandSampleCache(
        band,
        config,
        anchorX,
        anchorZ,
        this.source,
      );
      const bandSampleCacheMs = performance.now() - sampleCacheStartedAt;
      sampledCellCount += sampleCache.sampledCellCount;
      sampleCacheMs += bandSampleCacheMs;
      const meshBuildStartedAt = performance.now();
      band.mesh = buildBandMesh(
        this.source,
        config,
        effectiveInnerRadius,
        exclusionMask,
        sampleCache.cache,
        centerX,
        centerZ,
      );
      const bandMeshBuildMs = performance.now() - meshBuildStartedAt;
      meshBuildMs += bandMeshBuildMs;
      band.gpuDirty = true;
      band.triangleCount = band.mesh.triangleCount;
      triangleCount += band.triangleCount;
      builtBands += 1;
      changed = true;
      bandBuilds.push({
        label: band.label,
        sampledCellCount: sampleCache.sampledCellCount,
        sampleCacheMs: bandSampleCacheMs,
        meshBuildMs: bandMeshBuildMs,
        elapsedMs: performance.now() - bandStartedAt,
        triangleCount: band.triangleCount,
      });
    }

    this.lastUpdate = {
      changed,
      builtBands,
      pendingBands,
      meshCount: this.bands.length,
      triangleCount,
      elapsedMs: performance.now() - startedAt,
      sampledCellCount,
      sampleCacheMs,
      meshBuildMs,
      clearRadiusWorldUnits,
      clearRadiusMeters: worldUnitsToMeters(clearRadiusWorldUnits),
      maxRadiusWorldUnits: Math.max(...this.bands.map((band) => band.outerRadius)),
      maxRadiusMeters: worldUnitsToMeters(Math.max(...this.bands.map((band) => band.outerRadius))),
      bandBuilds,
    };
    return this.lastUpdate;
  }

  classifyCoverageAt(
    worldX: number,
    worldZ: number,
    exclusionMask: FarFieldExclusionMask | null = null,
  ): FarFieldCoverage | null {
    return this.getCoverageAt(worldX, worldZ, exclusionMask)[0] ?? null;
  }

  getCoverageAt(
    worldX: number,
    worldZ: number,
    exclusionMask: FarFieldExclusionMask | null = null,
  ): FarFieldCoverage[] {
    const coverage: FarFieldCoverage[] = [];
    for (const band of this.bands) {
      if (!band.mesh) {
        continue;
      }
      const cellX = Math.floor((worldX - band.anchorX) / band.sampleStride);
      const cellZ = Math.floor((worldZ - band.anchorZ) / band.sampleStride);
      const cellMinX = band.anchorX + cellX * band.sampleStride;
      const cellMinZ = band.anchorZ + cellZ * band.sampleStride;
      if (!isCellInBand(
        cellMinX,
        cellMinZ,
        band.sampleStride,
        band.centerX,
        band.centerZ,
        band.clearRadiusWorldUnits,
        band.outerRadius,
      )) {
        continue;
      }
      if (exclusionMask?.excludesCell(cellMinX, cellMinX + band.sampleStride, cellMinZ, cellMinZ + band.sampleStride)) {
        continue;
      }
      coverage.push({
        label: band.label,
        sampleStride: band.sampleStride,
        innerRadiusWorldUnits: band.clearRadiusWorldUnits,
        outerRadiusWorldUnits: band.outerRadius,
      });
    }
    return coverage;
  }

  probeMaskedSeamGaps(exclusionMask: FarFieldExclusionMask | null = null): FarFieldMaskedSeamProbe {
    let boundaryCount = 0;
    let gapCount = 0;
    let maxGapDepthWorldUnits = 0;
    const samples: FarFieldMaskedSeamProbe["samples"] = [];

    for (let bandIndex = 0; bandIndex < this.bands.length; bandIndex += 1) {
      const band = this.bands[bandIndex]!;
      const config = this.bandConfigs[bandIndex]!;
      const sampleCache = band.sampleCache;
      if (!band.mesh || !sampleCache) {
        continue;
      }
      const states = buildBandStates(
        config,
        sampleCache.anchorX,
        sampleCache.anchorZ,
        band.centerX,
        band.centerZ,
        band.clearRadiusWorldUnits,
        exclusionMask,
        sampleCache.sampleRadius,
        sampleCache.sampleSpan,
        sampleCache.heights,
      );
      const renderRadius = sampleCache.sampleRadius - 1;
      for (let cellZ = -renderRadius; cellZ <= renderRadius; cellZ += 1) {
        for (let cellX = -renderRadius; cellX <= renderRadius; cellX += 1) {
          const sampleIndex = cellX + sampleCache.sampleRadius + (cellZ + sampleCache.sampleRadius) * sampleCache.sampleSpan;
          if (states[sampleIndex] !== CELL_RENDERED) {
            continue;
          }
          const height = sampleCache.heights[sampleIndex]!;
          const worldX = sampleCache.anchorX + cellX * config.sampleStride;
          const worldZ = sampleCache.anchorZ + cellZ * config.sampleStride;
          evaluateMaskedSeamGap(
            this.source,
            band.label,
            "east",
            worldX,
            worldZ,
            config.sampleStride,
            states[sampleIndex + 1]!,
            sampleCache.heights[sampleIndex + 1]!,
            height,
            (gapDepthWorldUnits, sample) => {
              boundaryCount += 1;
              if (gapDepthWorldUnits > 0) {
                gapCount += 1;
                maxGapDepthWorldUnits = Math.max(maxGapDepthWorldUnits, gapDepthWorldUnits);
                if (samples.length < 8) {
                  samples.push(sample);
                }
              }
            },
          );
          evaluateMaskedSeamGap(
            this.source,
            band.label,
            "west",
            worldX,
            worldZ,
            config.sampleStride,
            states[sampleIndex - 1]!,
            sampleCache.heights[sampleIndex - 1]!,
            height,
            (gapDepthWorldUnits, sample) => {
              boundaryCount += 1;
              if (gapDepthWorldUnits > 0) {
                gapCount += 1;
                maxGapDepthWorldUnits = Math.max(maxGapDepthWorldUnits, gapDepthWorldUnits);
                if (samples.length < 8) {
                  samples.push(sample);
                }
              }
            },
          );
          evaluateMaskedSeamGap(
            this.source,
            band.label,
            "south",
            worldX,
            worldZ,
            config.sampleStride,
            states[sampleIndex + sampleCache.sampleSpan]!,
            sampleCache.heights[sampleIndex + sampleCache.sampleSpan]!,
            height,
            (gapDepthWorldUnits, sample) => {
              boundaryCount += 1;
              if (gapDepthWorldUnits > 0) {
                gapCount += 1;
                maxGapDepthWorldUnits = Math.max(maxGapDepthWorldUnits, gapDepthWorldUnits);
                if (samples.length < 8) {
                  samples.push(sample);
                }
              }
            },
          );
          evaluateMaskedSeamGap(
            this.source,
            band.label,
            "north",
            worldX,
            worldZ,
            config.sampleStride,
            states[sampleIndex - sampleCache.sampleSpan]!,
            sampleCache.heights[sampleIndex - sampleCache.sampleSpan]!,
            height,
            (gapDepthWorldUnits, sample) => {
              boundaryCount += 1;
              if (gapDepthWorldUnits > 0) {
                gapCount += 1;
                maxGapDepthWorldUnits = Math.max(maxGapDepthWorldUnits, gapDepthWorldUnits);
                if (samples.length < 8) {
                  samples.push(sample);
                }
              }
            },
          );
        }
      }
    }

    return {
      boundaryCount,
      gapCount,
      maxGapDepthWorldUnits,
      maxGapDepthMeters: worldUnitsToMeters(maxGapDepthWorldUnits),
      samples,
    };
  }

  probeSurfaceGaps(exclusionMask: FarFieldExclusionMask | null = null): FarFieldSurfaceGapProbe {
    let boundaryCount = 0;
    let gapCount = 0;
    let maxGapDepthWorldUnits = 0;
    const samples: FarFieldSurfaceGapProbe["samples"] = [];

    for (let bandIndex = 0; bandIndex < this.bands.length; bandIndex += 1) {
      const band = this.bands[bandIndex]!;
      const config = this.bandConfigs[bandIndex]!;
      const sampleCache = band.sampleCache;
      if (!band.mesh || !sampleCache) {
        continue;
      }
      const states = buildBandStates(
        config,
        sampleCache.anchorX,
        sampleCache.anchorZ,
        band.centerX,
        band.centerZ,
        band.clearRadiusWorldUnits,
        exclusionMask,
        sampleCache.sampleRadius,
        sampleCache.sampleSpan,
        sampleCache.heights,
      );
      const renderRadius = sampleCache.sampleRadius - 1;
      for (let cellZ = -renderRadius; cellZ <= renderRadius; cellZ += 1) {
        for (let cellX = -renderRadius; cellX <= renderRadius; cellX += 1) {
          const sampleIndex = cellX + sampleCache.sampleRadius + (cellZ + sampleCache.sampleRadius) * sampleCache.sampleSpan;
          if (states[sampleIndex] !== CELL_RENDERED) {
            continue;
          }
          const height = sampleCache.heights[sampleIndex]!;
          const worldX = sampleCache.anchorX + cellX * config.sampleStride;
          const worldZ = sampleCache.anchorZ + cellZ * config.sampleStride;
          evaluateSurfaceGap(
            sampleCache,
            this.source,
            band.label,
            "east",
            worldX,
            worldZ,
            config.sampleStride,
            sampleIndex,
            states[sampleIndex + 1]!,
            sampleCache.heights[sampleIndex + 1]!,
            height,
            (gapDepthWorldUnits, sample) => {
              boundaryCount += 1;
              if (gapDepthWorldUnits > 0) {
                gapCount += 1;
                maxGapDepthWorldUnits = Math.max(maxGapDepthWorldUnits, gapDepthWorldUnits);
                if (samples.length < 8) {
                  samples.push(sample);
                }
              }
            },
          );
          evaluateSurfaceGap(
            sampleCache,
            this.source,
            band.label,
            "west",
            worldX,
            worldZ,
            config.sampleStride,
            sampleIndex,
            states[sampleIndex - 1]!,
            sampleCache.heights[sampleIndex - 1]!,
            height,
            (gapDepthWorldUnits, sample) => {
              boundaryCount += 1;
              if (gapDepthWorldUnits > 0) {
                gapCount += 1;
                maxGapDepthWorldUnits = Math.max(maxGapDepthWorldUnits, gapDepthWorldUnits);
                if (samples.length < 8) {
                  samples.push(sample);
                }
              }
            },
          );
          evaluateSurfaceGap(
            sampleCache,
            this.source,
            band.label,
            "south",
            worldX,
            worldZ,
            config.sampleStride,
            sampleIndex,
            states[sampleIndex + sampleCache.sampleSpan]!,
            sampleCache.heights[sampleIndex + sampleCache.sampleSpan]!,
            height,
            (gapDepthWorldUnits, sample) => {
              boundaryCount += 1;
              if (gapDepthWorldUnits > 0) {
                gapCount += 1;
                maxGapDepthWorldUnits = Math.max(maxGapDepthWorldUnits, gapDepthWorldUnits);
                if (samples.length < 8) {
                  samples.push(sample);
                }
              }
            },
          );
          evaluateSurfaceGap(
            sampleCache,
            this.source,
            band.label,
            "north",
            worldX,
            worldZ,
            config.sampleStride,
            sampleIndex,
            states[sampleIndex - sampleCache.sampleSpan]!,
            sampleCache.heights[sampleIndex - sampleCache.sampleSpan]!,
            height,
            (gapDepthWorldUnits, sample) => {
              boundaryCount += 1;
              if (gapDepthWorldUnits > 0) {
                gapCount += 1;
                maxGapDepthWorldUnits = Math.max(maxGapDepthWorldUnits, gapDepthWorldUnits);
                if (samples.length < 8) {
                  samples.push(sample);
                }
              }
            },
          );
        }
      }
    }

    return {
      boundaryCount,
      gapCount,
      maxGapDepthWorldUnits,
      maxGapDepthMeters: worldUnitsToMeters(maxGapDepthWorldUnits),
      samples,
    };
  }
}

function buildBandMesh(
  source: FarFieldSource,
  band: FarFieldBandConfig,
  innerRadius: number,
  exclusionMask: FarFieldExclusionMask | null,
  sampleCache: FarFieldBandSampleCache,
  centerX: number,
  centerZ: number,
): ChunkMeshData {
  const {
    anchorX,
    anchorZ,
    sampleRadius,
    sampleSpan,
    heights,
    colors,
    waterHeights,
    waterColors,
  } = sampleCache;
  const states = buildBandStates(
    band,
    anchorX,
    anchorZ,
    centerX,
    centerZ,
    innerRadius,
    exclusionMask,
    sampleRadius,
    sampleSpan,
    heights,
  );
  let renderedCellCount = 0;
  for (const state of states) {
    if (state === CELL_RENDERED) {
      renderedCellCount += 1;
    }
  }
  const mesh = createMeshBuilder(renderedCellCount * 5);
  const waterMesh = createMeshBuilder(renderedCellCount);

  const renderRadius = sampleRadius - 1;
  for (let cellZ = -renderRadius; cellZ <= renderRadius; cellZ += 1) {
    for (let cellX = -renderRadius; cellX <= renderRadius; cellX += 1) {
      const sampleIndex = cellX + sampleRadius + (cellZ + sampleRadius) * sampleSpan;
      if (states[sampleIndex] !== CELL_RENDERED) {
        continue;
      }
      const height = heights[sampleIndex]!;
      const color = colors[sampleIndex]!;
      const worldX = anchorX + cellX * band.sampleStride;
      const worldZ = anchorZ + cellZ * band.sampleStride;
      const topY = height + 1;
      const x1 = worldX + band.sampleStride;
      const z1 = worldZ + band.sampleStride;
      const waterHeight = waterHeights[sampleIndex]!;
      const waterColor = waterColors[sampleIndex]!;
      const eastState = states[sampleIndex + 1]!;
      const eastHeight = heights[sampleIndex + 1]!;
      const westState = states[sampleIndex - 1]!;
      const westHeight = heights[sampleIndex - 1]!;
      const southState = states[sampleIndex + sampleSpan]!;
      const southHeight = heights[sampleIndex + sampleSpan]!;
      const northState = states[sampleIndex - sampleSpan]!;
      const northHeight = heights[sampleIndex - sampleSpan]!;
      const eastBoundaryMinSurfaceY = eastState === CELL_MASKED || eastHeight < height
        ? getBoundaryMinSurfaceY(sampleCache, source, "east", sampleIndex, worldX, worldZ, band.sampleStride)
        : null;
      const westBoundaryMinSurfaceY = westState === CELL_MASKED || westHeight < height
        ? getBoundaryMinSurfaceY(sampleCache, source, "west", sampleIndex, worldX, worldZ, band.sampleStride)
        : null;
      const southBoundaryMinSurfaceY = southState === CELL_MASKED || southHeight < height
        ? getBoundaryMinSurfaceY(sampleCache, source, "south", sampleIndex, worldX, worldZ, band.sampleStride)
        : null;
      const northBoundaryMinSurfaceY = northState === CELL_MASKED || northHeight < height
        ? getBoundaryMinSurfaceY(sampleCache, source, "north", sampleIndex, worldX, worldZ, band.sampleStride)
        : null;
      const eastBottomY = bottomYForBoundary(
        band.sampleStride,
        eastState,
        eastHeight,
        height,
        eastBoundaryMinSurfaceY,
      );
      const westBottomY = bottomYForBoundary(
        band.sampleStride,
        westState,
        westHeight,
        height,
        westBoundaryMinSurfaceY,
      );
      const southBottomY = bottomYForBoundary(
        band.sampleStride,
        southState,
        southHeight,
        height,
        southBoundaryMinSurfaceY,
      );
      const northBottomY = bottomYForBoundary(
        band.sampleStride,
        northState,
        northHeight,
        height,
        northBoundaryMinSurfaceY,
      );

      pushQuad(
        mesh,
        worldX, topY, worldZ,
        worldX, topY, z1,
        x1, topY, z1,
        x1, topY, worldZ,
        0, 1, 0,
        color,
      );

      pushSideQuad(
        mesh,
        eastState,
        eastHeight,
        height,
        color,
        x1, eastBottomY, worldZ,
        x1, topY, worldZ,
        x1, topY, z1,
        x1, eastBottomY, z1,
        1, 0, 0,
      );
      pushSideQuad(
        mesh,
        westState,
        westHeight,
        height,
        color,
        worldX, westBottomY, worldZ,
        worldX, westBottomY, z1,
        worldX, topY, z1,
        worldX, topY, worldZ,
        -1, 0, 0,
      );
      pushSideQuad(
        mesh,
        southState,
        southHeight,
        height,
        color,
        worldX, southBottomY, z1,
        x1, southBottomY, z1,
        x1, topY, z1,
        worldX, topY, z1,
        0, 0, 1,
      );
      pushSideQuad(
        mesh,
        northState,
        northHeight,
        height,
        color,
        worldX, northBottomY, worldZ,
        worldX, topY, worldZ,
        x1, topY, worldZ,
        x1, northBottomY, worldZ,
        0, 0, -1,
      );

      pushWaterTopQuad(waterMesh, worldX, worldZ, x1, z1, height, waterHeight, waterColor);
    }
  }

  return finishCombinedMeshBuilders(mesh, waterMesh, anchorX, anchorZ);
}

function ensureBandSampleCache(
  band: FarFieldBandState,
  config: FarFieldBandConfig,
  anchorX: number,
  anchorZ: number,
  source: FarFieldSource,
): {
  cache: FarFieldBandSampleCache;
  sampledCellCount: number;
} {
  const sampleRadius = computeBandSampleRadius(config);
  const sampleSpan = sampleRadius * 2 + 1;
  const cached = band.sampleCache;
  if (
    cached
    && cached.sampleRadius === sampleRadius
    && cached.sampleSpan === sampleSpan
  ) {
    if (cached.anchorX === anchorX && cached.anchorZ === anchorZ) {
      return {
        cache: cached,
        sampledCellCount: 0,
      };
    }
    const shifted = tryShiftBandSampleCache(cached, config, anchorX, anchorZ, source);
    if (shifted) {
      band.sampleCache = shifted.cache;
      return shifted;
    }
  }

  const heights = new Int32Array(sampleSpan * sampleSpan);
  heights.fill(NO_GENERATED_SURFACE_HEIGHT);
  const colors = new Uint32Array(sampleSpan * sampleSpan);
  const waterHeights = new Int32Array(sampleSpan * sampleSpan);
  waterHeights.fill(NO_WATER_HEIGHT);
  const waterColors = new Uint32Array(sampleSpan * sampleSpan);
  const eastBoundaryMinSurfaceY = new Int32Array(sampleSpan * sampleSpan);
  eastBoundaryMinSurfaceY.fill(UNKNOWN_BOUNDARY_MIN_SURFACE_Y);
  const southBoundaryMinSurfaceY = new Int32Array(sampleSpan * sampleSpan);
  southBoundaryMinSurfaceY.fill(UNKNOWN_BOUNDARY_MIN_SURFACE_Y);

  for (let sampleZ = 0; sampleZ < sampleSpan; sampleZ += 1) {
    const cellZ = sampleZ - sampleRadius;
    const cellMinZ = anchorZ + cellZ * config.sampleStride;
    const rowOffset = sampleZ * sampleSpan;
    for (let sampleX = 0; sampleX < sampleSpan; sampleX += 1) {
      const cellX = sampleX - sampleRadius;
      const cellMinX = anchorX + cellX * config.sampleStride;
      const sampledCell = sampleFarFieldCell(source, cellMinX, cellMinZ, config.sampleStride);
      const sampleIndex = sampleX + rowOffset;
      heights[sampleIndex] = sampledCell.surfaceY;
      colors[sampleIndex] = sampledCell.surfaceColor;
      waterHeights[sampleIndex] = sampledCell.waterTopY ?? NO_WATER_HEIGHT;
      waterColors[sampleIndex] = sampledCell.waterColor;
    }
  }

  const sampleCache: FarFieldBandSampleCache = {
    anchorX,
    anchorZ,
    sampleRadius,
    sampleSpan,
    heights,
    colors,
    waterHeights,
    waterColors,
    eastBoundaryMinSurfaceY,
    southBoundaryMinSurfaceY,
  };
  band.sampleCache = sampleCache;
  return {
    cache: sampleCache,
    sampledCellCount: sampleSpan * sampleSpan,
  };
}

function tryShiftBandSampleCache(
  cached: FarFieldBandSampleCache,
  config: FarFieldBandConfig,
  anchorX: number,
  anchorZ: number,
  source: FarFieldSource,
): {
  cache: FarFieldBandSampleCache;
  sampledCellCount: number;
} | null {
  const deltaXCells = Math.round((anchorX - cached.anchorX) / config.sampleStride);
  const deltaZCells = Math.round((anchorZ - cached.anchorZ) / config.sampleStride);
  if (
    !Number.isFinite(deltaXCells)
    || !Number.isFinite(deltaZCells)
    || Math.abs(deltaXCells) >= cached.sampleSpan
    || Math.abs(deltaZCells) >= cached.sampleSpan
  ) {
    return null;
  }

  const expectedDeltaX = deltaXCells * config.sampleStride;
  const expectedDeltaZ = deltaZCells * config.sampleStride;
  if (
    Math.abs((anchorX - cached.anchorX) - expectedDeltaX) > 1e-6
    || Math.abs((anchorZ - cached.anchorZ) - expectedDeltaZ) > 1e-6
  ) {
    return null;
  }

  const heights = new Int32Array(cached.heights.length);
  heights.fill(NO_GENERATED_SURFACE_HEIGHT);
  const colors = new Uint32Array(cached.colors.length);
  const waterHeights = new Int32Array(cached.waterHeights.length);
  waterHeights.fill(NO_WATER_HEIGHT);
  const waterColors = new Uint32Array(cached.waterColors.length);
  const eastBoundaryMinSurfaceY = new Int32Array(cached.eastBoundaryMinSurfaceY.length);
  eastBoundaryMinSurfaceY.fill(UNKNOWN_BOUNDARY_MIN_SURFACE_Y);
  const southBoundaryMinSurfaceY = new Int32Array(cached.southBoundaryMinSurfaceY.length);
  southBoundaryMinSurfaceY.fill(UNKNOWN_BOUNDARY_MIN_SURFACE_Y);
  let sampledCellCount = 0;

  for (let sampleZ = 0; sampleZ < cached.sampleSpan; sampleZ += 1) {
    const oldSampleZ = sampleZ + deltaZCells;
    const rowOffset = sampleZ * cached.sampleSpan;
    for (let sampleX = 0; sampleX < cached.sampleSpan; sampleX += 1) {
      const sampleIndex = sampleX + rowOffset;
      const oldSampleX = sampleX + deltaXCells;
      if (
        oldSampleX >= 0
        && oldSampleX < cached.sampleSpan
        && oldSampleZ >= 0
        && oldSampleZ < cached.sampleSpan
      ) {
        const oldIndex = oldSampleX + oldSampleZ * cached.sampleSpan;
        heights[sampleIndex] = cached.heights[oldIndex]!;
        colors[sampleIndex] = cached.colors[oldIndex]!;
        waterHeights[sampleIndex] = cached.waterHeights[oldIndex]!;
        waterColors[sampleIndex] = cached.waterColors[oldIndex]!;
        eastBoundaryMinSurfaceY[sampleIndex] = cached.eastBoundaryMinSurfaceY[oldIndex]!;
        southBoundaryMinSurfaceY[sampleIndex] = cached.southBoundaryMinSurfaceY[oldIndex]!;
        continue;
      }
      const cellX = sampleX - cached.sampleRadius;
      const cellZ = sampleZ - cached.sampleRadius;
      const cellMinX = anchorX + cellX * config.sampleStride;
      const cellMinZ = anchorZ + cellZ * config.sampleStride;
      const sampledCell = sampleFarFieldCell(source, cellMinX, cellMinZ, config.sampleStride);
      heights[sampleIndex] = sampledCell.surfaceY;
      colors[sampleIndex] = sampledCell.surfaceColor;
      waterHeights[sampleIndex] = sampledCell.waterTopY ?? NO_WATER_HEIGHT;
      waterColors[sampleIndex] = sampledCell.waterColor;
      sampledCellCount += 1;
    }
  }

  return {
    cache: {
      anchorX,
      anchorZ,
      sampleRadius: cached.sampleRadius,
      sampleSpan: cached.sampleSpan,
      heights,
      colors,
      waterHeights,
      waterColors,
      eastBoundaryMinSurfaceY,
      southBoundaryMinSurfaceY,
    },
    sampledCellCount,
  };
}

function buildBandStates(
  band: FarFieldBandConfig,
  anchorX: number,
  anchorZ: number,
  centerX: number,
  centerZ: number,
  innerRadius: number,
  exclusionMask: FarFieldExclusionMask | null,
  sampleRadius: number,
  sampleSpan: number,
  heights: Int32Array,
): Uint8Array {
  const states = new Uint8Array(sampleSpan * sampleSpan);
  for (let sampleZ = 0; sampleZ < sampleSpan; sampleZ += 1) {
    const cellZ = sampleZ - sampleRadius;
    const cellMinZ = anchorZ + cellZ * band.sampleStride;
    const rowOffset = sampleZ * sampleSpan;
    for (let sampleX = 0; sampleX < sampleSpan; sampleX += 1) {
      const cellX = sampleX - sampleRadius;
      const cellMinX = anchorX + cellX * band.sampleStride;
      const sampleIndex = sampleX + rowOffset;
      if (heights[sampleIndex] === NO_GENERATED_SURFACE_HEIGHT) {
        states[sampleIndex] = CELL_OMITTED;
        continue;
      }
      if (!isCellInBand(cellMinX, cellMinZ, band.sampleStride, centerX, centerZ, innerRadius, band.outerRadius)) {
        states[sampleIndex] = CELL_OMITTED;
        continue;
      }
      states[sampleIndex] = exclusionMask?.excludesCell(
        cellMinX,
        cellMinX + band.sampleStride,
        cellMinZ,
        cellMinZ + band.sampleStride,
      )
        ? CELL_MASKED
        : CELL_RENDERED;
    }
  }
  return states;
}

function createMeshBuilder(maxQuadCount: number): MeshBuilder {
  const vertexBuffer = new ArrayBuffer(maxQuadCount * 4 * VERTEX_STRIDE);
  return {
    vertexBuffer,
    vertexView: new DataView(vertexBuffer),
    indexData: new Uint32Array(maxQuadCount * 6),
    vertexCount: 0,
    indexCount: 0,
    minX: Number.POSITIVE_INFINITY,
    minY: Number.POSITIVE_INFINITY,
    minZ: Number.POSITIVE_INFINITY,
    maxX: Number.NEGATIVE_INFINITY,
    maxY: Number.NEGATIVE_INFINITY,
    maxZ: Number.NEGATIVE_INFINITY,
  };
}

function finishMeshBuilder(
  mesh: MeshBuilder,
): {
  vertexData: ArrayBuffer;
  vertexCount: number;
  indexData: Uint32Array;
  indexCount: number;
  triangleCount: number;
  bounds: {
    min: [number, number, number];
    max: [number, number, number];
  } | null;
} {
  const usedVertexBytes = mesh.vertexCount * VERTEX_STRIDE;
  const vertexData = usedVertexBytes === mesh.vertexBuffer.byteLength
    ? mesh.vertexBuffer
    : mesh.vertexBuffer.slice(0, usedVertexBytes);
  const indexData = mesh.indexCount === mesh.indexData.length
    ? mesh.indexData
    : mesh.indexData.slice(0, mesh.indexCount);
  return {
    vertexData,
    vertexCount: mesh.vertexCount,
    indexData,
    indexCount: mesh.indexCount,
    triangleCount: mesh.indexCount / 3,
    bounds: mesh.vertexCount === 0
      ? null
      : {
          min: [mesh.minX, mesh.minY, mesh.minZ],
          max: [mesh.maxX, mesh.maxY, mesh.maxZ],
        },
  };
}

function finishCombinedMeshBuilders(
  opaqueMesh: MeshBuilder,
  waterMesh: MeshBuilder,
  anchorX: number,
  anchorZ: number,
): ChunkMeshData {
  const opaque = finishMeshBuilder(opaqueMesh);
  const water = finishMeshBuilder(waterMesh);
  return {
    vertexData: opaque.vertexData,
    vertexCount: opaque.vertexCount,
    indexData: opaque.indexData,
    indexCount: opaque.indexCount,
    waterVertexData: water.vertexData,
    waterVertexCount: water.vertexCount,
    waterIndexData: water.indexData,
    waterIndexCount: water.indexCount,
    waterTriangleCount: water.triangleCount,
    triangleCount: opaque.triangleCount + water.triangleCount,
    bounds: opaque.bounds || water.bounds
      ? combineBounds(
          opaque.bounds,
          water.bounds,
        )
      : {
          min: [anchorX, 0, anchorZ],
          max: [anchorX, 0, anchorZ],
        }
  };
}

function combineBounds(
  opaqueBounds: {
    min: [number, number, number];
    max: [number, number, number];
  } | null,
  waterBounds: {
    min: [number, number, number];
    max: [number, number, number];
  } | null,
): {
  min: [number, number, number];
  max: [number, number, number];
} {
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

function pushQuad(
  mesh: MeshBuilder,
  x0: number,
  y0: number,
  z0: number,
  x1: number,
  y1: number,
  z1: number,
  x2: number,
  y2: number,
  z2: number,
  x3: number,
  y3: number,
  z3: number,
  normalX: number,
  normalY: number,
  normalZ: number,
  color: number,
): void {
  const baseVertex = mesh.vertexCount;
  writeVertex(mesh.vertexView, baseVertex * VERTEX_STRIDE, x0, y0, z0, normalX, normalY, normalZ, color);
  writeVertex(mesh.vertexView, (baseVertex + 1) * VERTEX_STRIDE, x1, y1, z1, normalX, normalY, normalZ, color);
  writeVertex(mesh.vertexView, (baseVertex + 2) * VERTEX_STRIDE, x2, y2, z2, normalX, normalY, normalZ, color);
  writeVertex(mesh.vertexView, (baseVertex + 3) * VERTEX_STRIDE, x3, y3, z3, normalX, normalY, normalZ, color);
  mesh.vertexCount += 4;

  mesh.indexData[mesh.indexCount + 0] = baseVertex;
  mesh.indexData[mesh.indexCount + 1] = baseVertex + 1;
  mesh.indexData[mesh.indexCount + 2] = baseVertex + 2;
  mesh.indexData[mesh.indexCount + 3] = baseVertex;
  mesh.indexData[mesh.indexCount + 4] = baseVertex + 2;
  mesh.indexData[mesh.indexCount + 5] = baseVertex + 3;
  mesh.indexCount += 6;

  mesh.minX = Math.min(mesh.minX, x0, x1, x2, x3);
  mesh.minY = Math.min(mesh.minY, y0, y1, y2, y3);
  mesh.minZ = Math.min(mesh.minZ, z0, z1, z2, z3);
  mesh.maxX = Math.max(mesh.maxX, x0, x1, x2, x3);
  mesh.maxY = Math.max(mesh.maxY, y0, y1, y2, y3);
  mesh.maxZ = Math.max(mesh.maxZ, z0, z1, z2, z3);
}

function pushSideQuad(
  mesh: MeshBuilder,
  neighborState: number,
  neighborHeight: number,
  height: number,
  color: number,
  x0: number,
  y0: number,
  z0: number,
  x1: number,
  y1: number,
  z1: number,
  x2: number,
  y2: number,
  z2: number,
  x3: number,
  y3: number,
  z3: number,
  normalX: number,
  normalY: number,
  normalZ: number,
): void {
  if (neighborState !== CELL_MASKED && neighborHeight >= height) {
    return;
  }
  pushQuad(
    mesh,
    x0, y0, z0,
    x1, y1, z1,
    x2, y2, z2,
    x3, y3, z3,
    normalX,
    normalY,
    normalZ,
    color,
  );
}

function bottomYForBoundary(
  sampleStride: number,
  neighborState: number,
  neighborHeight: number,
  height: number,
  boundaryMinSurfaceY: number | null,
): number {
  let bottomY = neighborHeight >= height ? height + 1 : neighborHeight + 1;
  if (boundaryMinSurfaceY !== null) {
    bottomY = Math.min(bottomY, boundaryMinSurfaceY + 1);
  }
  if (neighborState === CELL_MASKED) {
    bottomY = Math.max(0, bottomY - computeMaskedSeamSkirtDepth(sampleStride));
  }
  return bottomY;
}

function getBoundaryMinSurfaceY(
  sampleCache: FarFieldBandSampleCache,
  source: FarFieldSource,
  direction: "east" | "west" | "south" | "north",
  sampleIndex: number,
  worldX: number,
  worldZ: number,
  sampleStride: number,
): number | null {
  switch (direction) {
    case "east": {
      let cached = sampleCache.eastBoundaryMinSurfaceY[sampleIndex]!;
      if (cached === UNKNOWN_BOUNDARY_MIN_SURFACE_Y) {
        cached = sampleBoundaryMinSurfaceY(source, "east", worldX, worldZ, sampleStride) ?? NO_GENERATED_SURFACE_HEIGHT;
        sampleCache.eastBoundaryMinSurfaceY[sampleIndex] = cached;
      }
      return cached === NO_GENERATED_SURFACE_HEIGHT ? null : cached;
    }
    case "west": {
      const westIndex = sampleIndex - 1;
      if (westIndex < 0) {
        return null;
      }
      let cached = sampleCache.eastBoundaryMinSurfaceY[westIndex]!;
      if (cached === UNKNOWN_BOUNDARY_MIN_SURFACE_Y) {
        cached = sampleBoundaryMinSurfaceY(source, "west", worldX, worldZ, sampleStride) ?? NO_GENERATED_SURFACE_HEIGHT;
        sampleCache.eastBoundaryMinSurfaceY[westIndex] = cached;
      }
      return cached === NO_GENERATED_SURFACE_HEIGHT ? null : cached;
    }
    case "south": {
      let cached = sampleCache.southBoundaryMinSurfaceY[sampleIndex]!;
      if (cached === UNKNOWN_BOUNDARY_MIN_SURFACE_Y) {
        cached = sampleBoundaryMinSurfaceY(source, "south", worldX, worldZ, sampleStride) ?? NO_GENERATED_SURFACE_HEIGHT;
        sampleCache.southBoundaryMinSurfaceY[sampleIndex] = cached;
      }
      return cached === NO_GENERATED_SURFACE_HEIGHT ? null : cached;
    }
    case "north": {
      const northIndex = sampleIndex - sampleCache.sampleSpan;
      if (northIndex < 0) {
        return null;
      }
      let cached = sampleCache.southBoundaryMinSurfaceY[northIndex]!;
      if (cached === UNKNOWN_BOUNDARY_MIN_SURFACE_Y) {
        cached = sampleBoundaryMinSurfaceY(source, "north", worldX, worldZ, sampleStride) ?? NO_GENERATED_SURFACE_HEIGHT;
        sampleCache.southBoundaryMinSurfaceY[northIndex] = cached;
      }
      return cached === NO_GENERATED_SURFACE_HEIGHT ? null : cached;
    }
  }
}

function pushWaterTopQuad(
  mesh: MeshBuilder,
  minX: number,
  minZ: number,
  maxX: number,
  maxZ: number,
  terrainHeight: number,
  waterHeight: number,
  waterColor: number,
): void {
  if (waterHeight === NO_WATER_HEIGHT || waterHeight <= terrainHeight || waterColor === 0) {
    return;
  }
  const topY = waterHeight + 1;
  const tintedWaterColor = applyWaterDepthTint(waterColor, waterHeight - terrainHeight);
  pushQuad(
    mesh,
    minX, topY, minZ,
    minX, topY, maxZ,
    maxX, topY, maxZ,
    maxX, topY, minZ,
    0, 1, 0,
    tintedWaterColor,
  );
}

function sampleFarFieldCell(
  source: FarFieldSource,
  cellMinX: number,
  cellMinZ: number,
  sampleStride: number,
): {
  surfaceY: number;
  surfaceColor: number;
  waterTopY: number | null;
  waterColor: number;
} {
  const centerX = cellMinX + sampleStride * 0.5;
  const centerZ = cellMinZ + sampleStride * 0.5;
  const centerColumn = source.sampleFarFieldColumn(centerX, centerZ);
  if (!centerColumn) {
    return {
      surfaceY: NO_GENERATED_SURFACE_HEIGHT,
      surfaceColor: 0,
      waterTopY: null,
      waterColor: 0,
    };
  }
  let waterTopY: number | null = centerColumn.waterTopY;
  let waterColor = centerColumn.waterMaterial !== null
    ? source.palette[centerColumn.waterMaterial] ?? 0
    : 0;

  if (sampleStride <= metersToWorldUnits(1.6)) {
    for (const [offsetX, offsetZ] of FEATURE_SAMPLE_OFFSETS) {
      const sampleX = cellMinX + sampleStride * offsetX;
      const sampleZ = cellMinZ + sampleStride * offsetZ;
      const column = source.sampleFarFieldColumn(sampleX, sampleZ);
      if (!column || column.waterTopY === null) {
        continue;
      }
      if (waterTopY === null || column.waterTopY > waterTopY) {
        waterTopY = column.waterTopY;
        waterColor = column.waterMaterial !== null
          ? source.palette[column.waterMaterial] ?? waterColor
          : waterColor;
      }
    }
  }

  return {
    surfaceY: centerColumn.surfaceY,
    surfaceColor: source.palette[centerColumn.surfaceMaterial] ?? 0,
    waterTopY,
    waterColor,
  };
}

function writeVertex(
  view: DataView,
  byteOffset: number,
  x: number,
  y: number,
  z: number,
  normalX: number,
  normalY: number,
  normalZ: number,
  color: number,
): void {
  view.setFloat32(byteOffset + 0 * FLOAT32_BYTES, x, true);
  view.setFloat32(byteOffset + 1 * FLOAT32_BYTES, y, true);
  view.setFloat32(byteOffset + 2 * FLOAT32_BYTES, z, true);
  view.setInt8(byteOffset + 12, normalX * NORMAL_SCALE);
  view.setInt8(byteOffset + 13, normalY * NORMAL_SCALE);
  view.setInt8(byteOffset + 14, normalZ * NORMAL_SCALE);
  view.setInt8(byteOffset + 15, NORMAL_SCALE);
  view.setUint32(byteOffset + 16, color, true);
}

function isCellInBand(
  cellMinX: number,
  cellMinZ: number,
  sampleStride: number,
  centerX: number,
  centerZ: number,
  innerRadius: number,
  outerRadius: number,
): boolean {
  const distance = Math.max(
    Math.abs(cellMinX + sampleStride * 0.5 - centerX),
    Math.abs(cellMinZ + sampleStride * 0.5 - centerZ),
  );
  return distance >= innerRadius && distance < outerRadius;
}

function computeMaskedSeamSkirtDepth(sampleStride: number): number {
  return Math.max(MIN_MASKED_SEAM_SKIRT_DEPTH, sampleStride * MASKED_SEAM_SKIRT_FACTOR);
}

function evaluateMaskedSeamGap(
  source: FarFieldSource,
  bandLabel: string,
  direction: "east" | "west" | "south" | "north",
  worldX: number,
  worldZ: number,
  sampleStride: number,
  neighborState: number,
  neighborHeight: number,
  height: number,
  record: (
    gapDepthWorldUnits: number,
    sample: FarFieldMaskedSeamProbe["samples"][number],
  ) => void,
): void {
  if (neighborState !== CELL_MASKED) {
    return;
  }
  const boundaryMinSurfaceY = sampleBoundaryMinSurfaceY(source, direction, worldX, worldZ, sampleStride);
  if (boundaryMinSurfaceY === null) {
    return;
  }
  const wallBottomY = bottomYForBoundary(
    sampleStride,
    neighborState,
    neighborHeight,
    height,
    boundaryMinSurfaceY,
  );
  const gapDepthWorldUnits = wallBottomY - (boundaryMinSurfaceY + 1);
  record(gapDepthWorldUnits, {
    band: bandLabel,
    direction,
    worldX,
    worldZ,
    gapDepthWorldUnits,
    gapDepthMeters: worldUnitsToMeters(gapDepthWorldUnits),
  });
}

function evaluateSurfaceGap(
  sampleCache: FarFieldBandSampleCache,
  source: FarFieldSource,
  bandLabel: string,
  direction: "east" | "west" | "south" | "north",
  worldX: number,
  worldZ: number,
  sampleStride: number,
  sampleIndex: number,
  neighborState: number,
  neighborHeight: number,
  height: number,
  record: (
    gapDepthWorldUnits: number,
    sample: FarFieldSurfaceGapProbe["samples"][number],
  ) => void,
): void {
  if (neighborState !== CELL_MASKED && neighborHeight >= height) {
    return;
  }
  const boundaryMinSurfaceY = getBoundaryMinSurfaceY(
    sampleCache,
    source,
    direction,
    sampleIndex,
    worldX,
    worldZ,
    sampleStride,
  );
  if (boundaryMinSurfaceY === null) {
    return;
  }
  const wallBottomY = bottomYForBoundary(
    sampleStride,
    neighborState,
    neighborHeight,
    height,
    boundaryMinSurfaceY,
  );
  const gapDepthWorldUnits = wallBottomY - (boundaryMinSurfaceY + 1);
  record(gapDepthWorldUnits, {
    band: bandLabel,
    kind: neighborState === CELL_MASKED ? "masked" : "downward",
    direction,
    worldX,
    worldZ,
    gapDepthWorldUnits,
    gapDepthMeters: worldUnitsToMeters(gapDepthWorldUnits),
  });
}

function sampleBoundaryMinSurfaceY(
  source: FarFieldSource,
  direction: "east" | "west" | "south" | "north",
  worldX: number,
  worldZ: number,
  sampleStride: number,
): number | null {
  let minSurfaceY = Number.POSITIVE_INFINITY;
  const sampleOffsets = sampleStride <= 4
    ? [0.5]
    : [0.125, 0.375, 0.625, 0.875].map((value) => Math.min(sampleStride - 0.5, sampleStride * value));
  for (const offset of sampleOffsets) {
    const sampleX = direction === "east"
      ? worldX + sampleStride + 0.5
      : direction === "west"
      ? worldX - 0.5
      : worldX + offset;
    const sampleZ = direction === "south"
      ? worldZ + sampleStride + 0.5
      : direction === "north"
      ? worldZ - 0.5
      : worldZ + offset;
    const sample = source.sampleFarFieldColumn(sampleX, sampleZ);
    if (!sample) {
      continue;
    }
    minSurfaceY = Math.min(minSurfaceY, sample.surfaceY);
  }
  return Number.isFinite(minSurfaceY) ? minSurfaceY : null;
}

const CELL_OMITTED = 0;
const CELL_MASKED = 1;
const CELL_RENDERED = 2;
const FEATURE_SAMPLE_OFFSETS: ReadonlyArray<readonly [number, number]> = [
  [0.2, 0.2],
  [0.8, 0.2],
  [0.2, 0.8],
  [0.8, 0.8],
];

function snapToStride(worldCoordinate: number, stride: number): number {
  return Math.floor(worldCoordinate / stride) * stride;
}

function snapToNearestStride(worldCoordinate: number, stride: number): number {
  return Math.round(worldCoordinate / stride) * stride;
}

function computeBandSampleRadius(band: FarFieldBandConfig): number {
  const outerCells = Math.ceil(band.outerRadius / band.sampleStride);
  return outerCells + Math.ceil(band.anchorStride / band.sampleStride) + 1;
}
