import { metersToWorldUnits, worldUnitsToMeters } from "./scale.ts";
import { ProceduralWorldGenerator } from "./procedural-generator.ts";
import type { ChunkMeshData, Vec3 } from "./types.ts";

const FLOAT32_BYTES = 4;
const NORMAL_SCALE = 127;
const VERTEX_STRIDE = 20;

interface FarFieldBandConfig {
  readonly label: string;
  readonly innerRadius: number;
  readonly outerRadius: number;
  readonly sampleStride: number;
  readonly anchorStride: number;
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

export interface FarFieldCoverage {
  readonly label: string;
  readonly sampleStride: number;
  readonly innerRadiusWorldUnits: number;
  readonly outerRadiusWorldUnits: number;
}

export interface FarFieldUpdateSummary {
  changed: boolean;
  builtBands: number;
  meshCount: number;
  triangleCount: number;
  elapsedMs: number;
  clearRadiusWorldUnits: number;
  clearRadiusMeters: number;
  maxRadiusWorldUnits: number;
  maxRadiusMeters: number;
}

const DEFAULT_BANDS: readonly FarFieldBandConfig[] = [
  {
    label: "mid",
    innerRadius: metersToWorldUnits(6),
    outerRadius: metersToWorldUnits(96),
    sampleStride: metersToWorldUnits(1.6),
    anchorStride: metersToWorldUnits(25.6),
  },
  {
    label: "far",
    innerRadius: metersToWorldUnits(96),
    outerRadius: metersToWorldUnits(224),
    sampleStride: metersToWorldUnits(4.8),
    anchorStride: metersToWorldUnits(76.8),
  },
  {
    label: "horizon",
    innerRadius: metersToWorldUnits(224),
    outerRadius: metersToWorldUnits(416),
    sampleStride: metersToWorldUnits(12.8),
    anchorStride: metersToWorldUnits(204.8),
  },
] as const;

export class ProceduralFarField {
  readonly bands: FarFieldBandRenderable[];
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
      anchorX: Number.NaN,
      anchorZ: Number.NaN,
      centerX: Number.NaN,
      centerZ: Number.NaN,
      clearRadiusWorldUnits: Number.NaN,
      maskRevision: -1,
      mesh: null,
      gpuDirty: false,
      triangleCount: 0,
    }));
    this.lastUpdate = {
      changed: false,
      builtBands: 0,
      meshCount: this.bands.length,
      triangleCount: 0,
      elapsedMs: 0,
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
  ): FarFieldUpdateSummary {
    const startedAt = performance.now();
    let changed = false;
    let builtBands = 0;
    let triangleCount = 0;
    const maxAffectedRadiusWorldUnits = exclusionMask?.maxAffectedRadiusWorldUnits ?? Number.POSITIVE_INFINITY;

    for (let bandIndex = 0; bandIndex < this.bands.length; bandIndex += 1) {
      const band = this.bands[bandIndex]!;
      const config = this.bandConfigs[bandIndex]!;
      const anchorX = snapToStride(Math.floor(position[0]), config.anchorStride);
      const anchorZ = snapToStride(Math.floor(position[2]), config.anchorStride);
      const effectiveInnerRadius = Math.max(config.innerRadius, clearRadiusWorldUnits);
      const centerX = snapToNearestStride(position[0], config.sampleStride);
      const centerZ = snapToNearestStride(position[2], config.sampleStride);
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
      band.anchorX = anchorX;
      band.anchorZ = anchorZ;
      band.centerX = centerX;
      band.centerZ = centerZ;
      band.clearRadiusWorldUnits = effectiveInnerRadius;
      band.maskRevision = maskRevision;
      band.mesh = buildBandMesh(
        this.generator,
        config,
        anchorX,
        anchorZ,
        centerX,
        centerZ,
        effectiveInnerRadius,
        exclusionMask,
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
      meshCount: this.bands.length,
      triangleCount,
      elapsedMs: performance.now() - startedAt,
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
}

function buildBandMesh(
  generator: ProceduralWorldGenerator,
  band: FarFieldBandConfig,
  anchorX: number,
  anchorZ: number,
  centerX: number,
  centerZ: number,
  innerRadius: number,
  exclusionMask: FarFieldExclusionMask | null,
): ChunkMeshData {
  const outerCells = Math.ceil(band.outerRadius / band.sampleStride);
  const sampleRadius = outerCells
    + Math.ceil(Math.max(Math.abs(centerX - anchorX), Math.abs(centerZ - anchorZ)) / band.sampleStride)
    + 1;
  const sampleSpan = sampleRadius * 2 + 1;
  const heights = new Int32Array(sampleSpan * sampleSpan);
  const colors = new Uint32Array(sampleSpan * sampleSpan);
  const states = new Uint8Array(sampleSpan * sampleSpan);

  for (let sampleZ = 0; sampleZ < sampleSpan; sampleZ += 1) {
    const cellZ = sampleZ - sampleRadius;
    const worldZ = anchorZ + cellZ * band.sampleStride;
    const rowOffset = sampleZ * sampleSpan;
    for (let sampleX = 0; sampleX < sampleSpan; sampleX += 1) {
      const cellX = sampleX - sampleRadius;
      const worldX = anchorX + cellX * band.sampleStride;
      const column = generator.sampleColumn(worldX, worldZ);
      const sampleIndex = sampleX + rowOffset;
      heights[sampleIndex] = column.surfaceY;
      colors[sampleIndex] = generator.palette[column.surfaceMaterial] ?? 0;
      if (!isCellInBand(worldX, worldZ, band.sampleStride, centerX, centerZ, innerRadius, band.outerRadius)) {
        states[sampleIndex] = CELL_OMITTED;
        continue;
      }
      states[sampleIndex] = exclusionMask?.excludesCell(
        worldX,
        worldX + band.sampleStride,
        worldZ,
        worldZ + band.sampleStride,
      )
        ? CELL_MASKED
        : CELL_RENDERED;
    }
  }

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
      const eastBottomY = bottomYForNeighbor(states[sampleIndex + 1]!, heights[sampleIndex + 1]!, height);
      const westBottomY = bottomYForNeighbor(states[sampleIndex - 1]!, heights[sampleIndex - 1]!, height);
      const southBottomY = bottomYForNeighbor(states[sampleIndex + sampleSpan]!, heights[sampleIndex + sampleSpan]!, height);
      const northBottomY = bottomYForNeighbor(states[sampleIndex - sampleSpan]!, heights[sampleIndex - sampleSpan]!, height);

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
    }
  }

  return buildMeshData(vertices, indices, anchorX, anchorZ);
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
  if (neighborState === CELL_MASKED) {
    return;
  }
  if (neighborHeight >= height) {
    return;
  }
  pushQuad(vertices, indices, corners, normal, color);
}

function bottomYForNeighbor(neighborState: number, neighborHeight: number, height: number): number {
  if (neighborState === CELL_MASKED || neighborHeight >= height) {
    return height + 1;
  }
  return neighborHeight + 1;
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

const CELL_OMITTED = 0;
const CELL_MASKED = 1;
const CELL_RENDERED = 2;

function snapToStride(worldCoordinate: number, stride: number): number {
  return Math.floor(worldCoordinate / stride) * stride;
}

function snapToNearestStride(worldCoordinate: number, stride: number): number {
  return Math.round(worldCoordinate / stride) * stride;
}
