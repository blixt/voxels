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

export interface FarFieldBandRenderable {
  readonly label: string;
  readonly innerRadius: number;
  readonly outerRadius: number;
  readonly sampleStride: number;
  anchorX: number;
  anchorZ: number;
  clearRadiusWorldUnits: number;
  mesh: ChunkMeshData | null;
  gpuDirty: boolean;
  triangleCount: number;
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
    innerRadius: metersToWorldUnits(80),
    outerRadius: metersToWorldUnits(224),
    sampleStride: metersToWorldUnits(4.8),
    anchorStride: metersToWorldUnits(76.8),
  },
  {
    label: "horizon",
    innerRadius: metersToWorldUnits(192),
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
      clearRadiusWorldUnits: Number.NaN,
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

  updateAround(position: Vec3, clearRadiusWorldUnits = 0): FarFieldUpdateSummary {
    const startedAt = performance.now();
    let changed = false;
    let builtBands = 0;
    let triangleCount = 0;

    for (let bandIndex = 0; bandIndex < this.bands.length; bandIndex += 1) {
      const band = this.bands[bandIndex]!;
      const config = this.bandConfigs[bandIndex]!;
      const anchorX = snapToStride(Math.floor(position[0]), config.anchorStride);
      const anchorZ = snapToStride(Math.floor(position[2]), config.anchorStride);
      const effectiveInnerRadius = Math.max(config.innerRadius, clearRadiusWorldUnits);
      if (
        band.anchorX === anchorX
        && band.anchorZ === anchorZ
        && band.clearRadiusWorldUnits === effectiveInnerRadius
        && band.mesh
      ) {
        triangleCount += band.triangleCount;
        continue;
      }
      band.anchorX = anchorX;
      band.anchorZ = anchorZ;
      band.clearRadiusWorldUnits = effectiveInnerRadius;
      band.mesh = buildBandMesh(
        this.generator,
        config,
        anchorX,
        anchorZ,
        effectiveInnerRadius,
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
}

function buildBandMesh(
  generator: ProceduralWorldGenerator,
  band: FarFieldBandConfig,
  anchorX: number,
  anchorZ: number,
  innerRadius: number,
): ChunkMeshData {
  const outerCells = Math.ceil(band.outerRadius / band.sampleStride);
  const sampleSpan = outerCells * 2 + 1;
  const heights = new Int32Array(sampleSpan * sampleSpan);
  const colors = new Uint32Array(sampleSpan * sampleSpan);

  for (let sampleZ = 0; sampleZ < sampleSpan; sampleZ += 1) {
    const cellZ = sampleZ - outerCells;
    const worldZ = anchorZ + cellZ * band.sampleStride;
    const rowOffset = sampleZ * sampleSpan;
    for (let sampleX = 0; sampleX < sampleSpan; sampleX += 1) {
      const cellX = sampleX - outerCells;
      const worldX = anchorX + cellX * band.sampleStride;
      const column = generator.sampleColumn(worldX, worldZ);
      heights[sampleX + rowOffset] = column.surfaceY;
      colors[sampleX + rowOffset] = generator.palette[column.surfaceMaterial] ?? 0;
    }
  }

  const vertices: Array<[number, number, number, number, number, number, number]> = [];
  const indices: number[] = [];

  for (let cellZ = -outerCells; cellZ < outerCells; cellZ += 1) {
    for (let cellX = -outerCells; cellX < outerCells; cellX += 1) {
      if (!isCellInBand(cellX, cellZ, band.sampleStride, innerRadius, band.outerRadius)) {
        continue;
      }
      const sampleIndex = cellX + outerCells + (cellZ + outerCells) * sampleSpan;
      const height = heights[sampleIndex]!;
      const color = colors[sampleIndex]!;
      const worldX = anchorX + cellX * band.sampleStride;
      const worldZ = anchorZ + cellZ * band.sampleStride;
      const topY = height + 1;
      const x1 = worldX + band.sampleStride;
      const z1 = worldZ + band.sampleStride;

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

      if (isCellInBand(cellX + 1, cellZ, band.sampleStride, innerRadius, band.outerRadius)) {
        const eastHeight = heights[sampleIndex + 1]!;
        if (eastHeight < height) {
          const bottomY = eastHeight + 1;
          pushQuad(
            vertices,
            indices,
            [
              [x1, bottomY, worldZ],
              [x1, topY, worldZ],
              [x1, topY, z1],
              [x1, bottomY, z1],
            ],
            [1, 0, 0],
            color,
          );
        }
      }

      if (isCellInBand(cellX, cellZ + 1, band.sampleStride, innerRadius, band.outerRadius)) {
        const southHeight = heights[sampleIndex + sampleSpan]!;
        if (southHeight < height) {
          const bottomY = southHeight + 1;
          pushQuad(
            vertices,
            indices,
            [
              [worldX, bottomY, z1],
              [x1, bottomY, z1],
              [x1, topY, z1],
              [worldX, topY, z1],
            ],
            [0, 0, 1],
            color,
          );
        }
      }
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
  cellX: number,
  cellZ: number,
  sampleStride: number,
  innerRadius: number,
  outerRadius: number,
): boolean {
  const distance = Math.max(
    Math.abs((cellX + 0.5) * sampleStride),
    Math.abs((cellZ + 0.5) * sampleStride),
  );
  return distance > innerRadius && distance <= outerRadius;
}

function snapToStride(worldCoordinate: number, stride: number): number {
  return Math.floor(worldCoordinate / stride) * stride;
}
