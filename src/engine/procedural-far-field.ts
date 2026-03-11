import { metersToWorldUnits, worldUnitsToMeters } from "./scale.ts";
import { ProceduralWorldGenerator } from "./procedural-generator.ts";
import type { ChunkMeshData, Vec3 } from "./types.ts";

const FLOAT32_BYTES = 4;
const NORMAL_SCALE = 127;
const VERTEX_STRIDE = 20;
const NO_WATER_HEIGHT = -0x7fff_ffff;
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
}

interface FarFieldBandState extends FarFieldBandRenderable {
  sampleCache: FarFieldBandSampleCache | null;
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
  clearRadiusWorldUnits: number;
  clearRadiusMeters: number;
  maxRadiusWorldUnits: number;
  maxRadiusMeters: number;
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

const DEFAULT_BANDS: readonly FarFieldBandConfig[] = [
  {
    label: "near-transition",
    innerRadius: metersToWorldUnits(6),
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
    readonly generator: ProceduralWorldGenerator,
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
      clearRadiusWorldUnits: 0,
      clearRadiusMeters: 0,
      maxRadiusWorldUnits: Math.max(...bandConfigs.map((config) => config.outerRadius)),
      maxRadiusMeters: worldUnitsToMeters(Math.max(...bandConfigs.map((config) => config.outerRadius))),
    };
  }

  getRenderables(): readonly FarFieldBandRenderable[] {
    return this.bands;
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
    const maxAffectedRadiusWorldUnits = exclusionMask?.maxAffectedRadiusWorldUnits ?? Number.POSITIVE_INFINITY;
    const centerStride = Math.min(...this.bandConfigs.map((config) => config.centerStride ?? config.sampleStride));
      const centerX = snapToNearestStride(position[0], centerStride);
      const centerZ = snapToNearestStride(position[2], centerStride);

    for (let bandIndex = 0; bandIndex < this.bands.length; bandIndex += 1) {
      const band = this.bands[bandIndex]!;
      const config = this.bandConfigs[bandIndex]!;
      const anchorX = snapToStride(Math.floor(position[0]), config.anchorStride);
      const anchorZ = snapToStride(Math.floor(position[2]), config.anchorStride);
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
      const sampleCache = ensureBandSampleCache(
        band,
        config,
        anchorX,
        anchorZ,
        this.generator,
      );
      sampledCellCount += sampleCache.sampledCellCount;
      band.mesh = buildBandMesh(
        this.generator,
        config,
        effectiveInnerRadius,
        exclusionMask,
        sampleCache.cache,
        centerX,
        centerZ,
      );
      band.gpuDirty = true;
      band.triangleCount = band.mesh.triangleCount;
      triangleCount += band.triangleCount;
      builtBands += 1;
      changed = true;
    }

    this.lastUpdate = {
      changed,
      builtBands,
      pendingBands,
      meshCount: this.bands.length,
      triangleCount,
      elapsedMs: performance.now() - startedAt,
      sampledCellCount,
      clearRadiusWorldUnits,
      clearRadiusMeters: worldUnitsToMeters(clearRadiusWorldUnits),
      maxRadiusWorldUnits: Math.max(...this.bands.map((band) => band.outerRadius)),
      maxRadiusMeters: worldUnitsToMeters(Math.max(...this.bands.map((band) => band.outerRadius))),
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
            this.generator,
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
            this.generator,
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
            this.generator,
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
            this.generator,
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
}

function buildBandMesh(
  generator: ProceduralWorldGenerator,
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
  const states = buildBandStates(band, anchorX, anchorZ, centerX, centerZ, innerRadius, exclusionMask, sampleRadius, sampleSpan);

  const vertices: Array<[number, number, number, number, number, number, number]> = [];
  const indices: number[] = [];

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
      const eastBottomY = bottomYForBoundary(
        generator,
        "east",
        worldX,
        worldZ,
        band.sampleStride,
        states[sampleIndex + 1]!,
        heights[sampleIndex + 1]!,
        height,
      );
      const westBottomY = bottomYForBoundary(
        generator,
        "west",
        worldX,
        worldZ,
        band.sampleStride,
        states[sampleIndex - 1]!,
        heights[sampleIndex - 1]!,
        height,
      );
      const southBottomY = bottomYForBoundary(
        generator,
        "south",
        worldX,
        worldZ,
        band.sampleStride,
        states[sampleIndex + sampleSpan]!,
        heights[sampleIndex + sampleSpan]!,
        height,
      );
      const northBottomY = bottomYForBoundary(
        generator,
        "north",
        worldX,
        worldZ,
        band.sampleStride,
        states[sampleIndex - sampleSpan]!,
        heights[sampleIndex - sampleSpan]!,
        height,
      );

      pushQuad(
        vertices,
        indices,
        [
          [worldX, topY, worldZ],
          [worldX, topY, z1],
          [x1, topY, z1],
          [x1, topY, worldZ],
        ],
        [0, 1, 0],
        color,
      );

      pushSideQuad(vertices, indices, states[sampleIndex + 1]!, heights[sampleIndex + 1]!, height, color, [
        [x1, eastBottomY, worldZ],
        [x1, topY, worldZ],
        [x1, topY, z1],
        [x1, eastBottomY, z1],
      ], [1, 0, 0]);
      pushSideQuad(vertices, indices, states[sampleIndex - 1]!, heights[sampleIndex - 1]!, height, color, [
        [worldX, westBottomY, worldZ],
        [worldX, westBottomY, z1],
        [worldX, topY, z1],
        [worldX, topY, worldZ],
      ], [-1, 0, 0]);
      pushSideQuad(vertices, indices, states[sampleIndex + sampleSpan]!, heights[sampleIndex + sampleSpan]!, height, color, [
        [worldX, southBottomY, z1],
        [x1, southBottomY, z1],
        [x1, topY, z1],
        [worldX, topY, z1],
      ], [0, 0, 1]);
      pushSideQuad(vertices, indices, states[sampleIndex - sampleSpan]!, heights[sampleIndex - sampleSpan]!, height, color, [
        [worldX, northBottomY, worldZ],
        [worldX, topY, worldZ],
        [x1, topY, worldZ],
        [x1, northBottomY, worldZ],
      ], [0, 0, -1]);

      pushWaterTopQuad(vertices, indices, worldX, worldZ, x1, z1, height, waterHeight, waterColor);
    }
  }

  return buildMeshData(vertices, indices, anchorX, anchorZ);
}

function ensureBandSampleCache(
  band: FarFieldBandState,
  config: FarFieldBandConfig,
  anchorX: number,
  anchorZ: number,
  generator: ProceduralWorldGenerator,
): {
  cache: FarFieldBandSampleCache;
  sampledCellCount: number;
} {
  const sampleRadius = computeBandSampleRadius(config);
  const sampleSpan = sampleRadius * 2 + 1;
  const cached = band.sampleCache;
  if (
    cached
    && cached.anchorX === anchorX
    && cached.anchorZ === anchorZ
    && cached.sampleRadius === sampleRadius
    && cached.sampleSpan === sampleSpan
  ) {
    return {
      cache: cached,
      sampledCellCount: 0,
    };
  }

  const heights = new Int32Array(sampleSpan * sampleSpan);
  const colors = new Uint32Array(sampleSpan * sampleSpan);
  const waterHeights = new Int32Array(sampleSpan * sampleSpan);
  waterHeights.fill(NO_WATER_HEIGHT);
  const waterColors = new Uint32Array(sampleSpan * sampleSpan);

  for (let sampleZ = 0; sampleZ < sampleSpan; sampleZ += 1) {
    const cellZ = sampleZ - sampleRadius;
    const cellMinZ = anchorZ + cellZ * config.sampleStride;
    const rowOffset = sampleZ * sampleSpan;
    for (let sampleX = 0; sampleX < sampleSpan; sampleX += 1) {
      const cellX = sampleX - sampleRadius;
      const cellMinX = anchorX + cellX * config.sampleStride;
      const sampledCell = sampleFarFieldCell(generator, cellMinX, cellMinZ, config.sampleStride);
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
  };
  band.sampleCache = sampleCache;
  return {
    cache: sampleCache,
    sampledCellCount: sampleSpan * sampleSpan,
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

function buildMeshData(
  vertices: Array<[number, number, number, number, number, number, number]>,
  indices: number[],
  anchorX: number,
  anchorZ: number,
): ChunkMeshData {
  const vertexData = new ArrayBuffer(vertices.length * VERTEX_STRIDE);
  const view = new DataView(vertexData);
  let minX = Number.POSITIVE_INFINITY;
  let minY = Number.POSITIVE_INFINITY;
  let minZ = Number.POSITIVE_INFINITY;
  let maxX = Number.NEGATIVE_INFINITY;
  let maxY = Number.NEGATIVE_INFINITY;
  let maxZ = Number.NEGATIVE_INFINITY;

  for (let vertexIndex = 0; vertexIndex < vertices.length; vertexIndex += 1) {
    const [x, y, z, nx, ny, nz, color] = vertices[vertexIndex]!;
    minX = Math.min(minX, x);
    minY = Math.min(minY, y);
    minZ = Math.min(minZ, z);
    maxX = Math.max(maxX, x);
    maxY = Math.max(maxY, y);
    maxZ = Math.max(maxZ, z);
    writeVertex(view, vertexIndex * VERTEX_STRIDE, x, y, z, nx, ny, nz, color);
  }

  return {
    vertexData,
    vertexCount: vertices.length,
    indexData: new Uint32Array(indices),
    indexCount: indices.length,
    triangleCount: indices.length / 3,
    bounds: vertices.length === 0
      ? {
          min: [anchorX, 0, anchorZ],
          max: [anchorX, 0, anchorZ],
        }
      : {
          min: [minX, minY, minZ],
          max: [maxX, maxY, maxZ],
        },
  };
}

function pushQuad(
  vertices: Array<[number, number, number, number, number, number, number]>,
  indices: number[],
  corners: [
    [number, number, number],
    [number, number, number],
    [number, number, number],
    [number, number, number],
  ],
  normal: [number, number, number],
  color: number,
): void {
  const baseVertex = vertices.length;
  for (const [x, y, z] of corners) {
    vertices.push([x, y, z, normal[0], normal[1], normal[2], color]);
  }
  indices.push(
    baseVertex,
    baseVertex + 1,
    baseVertex + 2,
    baseVertex,
    baseVertex + 2,
    baseVertex + 3,
  );
}

function pushSideQuad(
  vertices: Array<[number, number, number, number, number, number, number]>,
  indices: number[],
  neighborState: number,
  neighborHeight: number,
  height: number,
  color: number,
  corners: [
    [number, number, number],
    [number, number, number],
    [number, number, number],
    [number, number, number],
  ],
  normal: [number, number, number],
): void {
  if (neighborState !== CELL_MASKED && neighborHeight >= height) {
    return;
  }
  pushQuad(vertices, indices, corners, normal, color);
}

function bottomYForBoundary(
  generator: ProceduralWorldGenerator,
  direction: "east" | "west" | "south" | "north",
  worldX: number,
  worldZ: number,
  sampleStride: number,
  neighborState: number,
  neighborHeight: number,
  height: number,
): number {
  let bottomY = neighborHeight >= height ? height + 1 : neighborHeight + 1;
  if (neighborState === CELL_MASKED) {
    bottomY = Math.min(
      bottomY,
      sampleMaskedBoundaryMinSurfaceY(generator, direction, worldX, worldZ, sampleStride) + 1,
    );
    bottomY = Math.max(0, bottomY - computeMaskedSeamSkirtDepth(sampleStride));
  }
  return bottomY;
}

function pushWaterTopQuad(
  vertices: Array<[number, number, number, number, number, number, number]>,
  indices: number[],
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
  pushQuad(
    vertices,
    indices,
    [
      [minX, topY, minZ],
      [minX, topY, maxZ],
      [maxX, topY, maxZ],
      [maxX, topY, minZ],
    ],
    [0, 1, 0],
    waterColor,
  );
}

function sampleFarFieldCell(
  generator: ProceduralWorldGenerator,
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
  const centerColumn = generator.sampleColumn(centerX, centerZ);
  let waterTopY: number | null = centerColumn.waterTopY;
  let waterColor = waterTopY !== null
    ? generator.palette[generator.sampleMaterial(centerX, waterTopY, centerZ)] ?? 0
    : 0;

  if (sampleStride <= metersToWorldUnits(1.6)) {
    for (const [offsetX, offsetZ] of FEATURE_SAMPLE_OFFSETS) {
      const sampleX = cellMinX + sampleStride * offsetX;
      const sampleZ = cellMinZ + sampleStride * offsetZ;
      const column = generator.sampleColumn(sampleX, sampleZ);
      if (column.waterTopY === null) {
        continue;
      }
      if (waterTopY === null || column.waterTopY > waterTopY) {
        waterTopY = column.waterTopY;
        waterColor = generator.palette[generator.sampleMaterial(sampleX, column.waterTopY, sampleZ)] ?? waterColor;
      }
    }
  }

  return {
    surfaceY: centerColumn.surfaceY,
    surfaceColor: generator.palette[centerColumn.surfaceMaterial] ?? 0,
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
  generator: ProceduralWorldGenerator,
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
  const wallBottomY = bottomYForBoundary(
    generator,
    direction,
    worldX,
    worldZ,
    sampleStride,
    neighborState,
    neighborHeight,
    height,
  );
  const boundaryMinSurfaceY = sampleMaskedBoundaryMinSurfaceY(generator, direction, worldX, worldZ, sampleStride);
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

function sampleMaskedBoundaryMinSurfaceY(
  generator: ProceduralWorldGenerator,
  direction: "east" | "west" | "south" | "north",
  worldX: number,
  worldZ: number,
  sampleStride: number,
): number {
  const sampleStep = 1;
  let minSurfaceY = Number.POSITIVE_INFINITY;
  const sampleCount = Math.max(1, Math.floor(sampleStride / sampleStep));
  for (let sampleIndex = 0; sampleIndex < sampleCount; sampleIndex += 1) {
    const offset = Math.min(sampleStride - 0.5, sampleIndex + 0.5);
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
    minSurfaceY = Math.min(minSurfaceY, generator.sampleColumn(sampleX, sampleZ).surfaceY);
  }
  return minSurfaceY;
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
