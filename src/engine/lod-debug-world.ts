import {
  buildOpaqueChunkMeshFromInput,
  createMeshMaterialLut,
  type MeshMaterialLut,
  type OpaqueChunkNeighborFaceSnapshot,
} from "./opaque-chunk-mesher.ts";
import type { ChunkBounds, ChunkMeshData, PackedColor, WorldStats } from "./types.ts";
import { type ResidentChunkWorld, type VoxelChunk } from "./world.ts";

export const LOD_DEBUG_MATERIAL = {
  lod0: 1,
  lod1: 2,
  edited: 3,
} as const;

export interface LodDebugCoverageRequest {
  centerChunkX: number;
  centerChunkZ: number;
  nearRadiusLod1Chunks: number;
  farRadiusLod1Chunks: number;
}

export interface LodDebugCoverageReport {
  checkedColumns: number;
  gaps: number;
  overlaps: number;
  activeChunks: number;
  pendingChunks: number;
}

type LodChunkKey = `L${number}:${number}:${number}:${number}`;

interface ParsedLodChunkKey {
  level: number;
  cx: number;
  cy: number;
  cz: number;
}

export class LodDebugWorld implements ResidentChunkWorld {
  readonly minY = 0;
  readonly maxYExclusive: number;
  readonly palette: PackedColor[];

  private readonly chunks = new Map<LodChunkKey, VoxelChunk>();
  private readonly activeKeys = new Set<LodChunkKey>();
  private readonly pendingKeys = new Set<LodChunkKey>();
  private readonly readyInactiveKeys = new Set<LodChunkKey>();
  private readonly editMaterials = new Map<string, number>();
  private readonly materialLut: MeshMaterialLut;

  constructor(
    readonly chunkSize = 16,
    readonly horizontalRadiusChunks = 12,
  ) {
    this.maxYExclusive = chunkSize * 2;
    this.palette = [
      0x00000000,
      0xff79a7ff,
      0xffffb347,
      0xffff4fd8,
    ];
    this.materialLut = createMeshMaterialLut(this.palette, () => false);
  }

  getVoxel(x: number, y: number, z: number): number {
    return this.sampleBaseMaterial(Math.floor(x), Math.floor(y), Math.floor(z), LOD_DEBUG_MATERIAL.lod0);
  }

  getPaletteColor(materialIndex: number): PackedColor {
    return this.palette[materialIndex] ?? 0;
  }

  isCollisionMaterial(materialIndex: number): boolean {
    return materialIndex !== 0;
  }

  isWaterMaterial(_materialIndex: number): boolean {
    return false;
  }

  getResidentChunk(cx: number, cy: number, cz: number): VoxelChunk | null {
    for (const key of this.activeKeys) {
      const parsed = parseLodChunkKey(key);
      if (parsed && parsed.cx === cx && parsed.cy === cy && parsed.cz === cz) {
        return this.chunks.get(key) ?? null;
      }
    }
    return null;
  }

  hasResidentChunk(cx: number, cy: number, cz: number): boolean {
    return this.getResidentChunk(cx, cy, cz) !== null;
  }

  *iterateResidentChunks(): Iterable<VoxelChunk> {
    const sortedKeys = [...this.activeKeys].sort(compareLodChunkKeys);
    for (const key of sortedKeys) {
      const chunk = this.chunks.get(key);
      if (chunk) {
        yield chunk;
      }
    }
  }

  getChunkSolidBounds(cx: number, cy: number, cz: number): ChunkBounds | null {
    const chunk = this.getResidentChunk(cx, cy, cz);
    return chunk?.solidBounds
      ? { min: [...chunk.solidBounds.min], max: [...chunk.solidBounds.max] }
      : null;
  }

  requestCoverage(request: LodDebugCoverageRequest): void {
    const desiredKeys = this.computeDesiredKeys(request);
    this.enqueueMissingDesiredChunks(desiredKeys);

    for (const key of [...this.activeKeys]) {
      if (desiredKeys.has(key)) {
        continue;
      }
      const replacementPending = [...desiredKeys].some((desiredKey) => chunksOverlap(key, desiredKey));
      if (!replacementPending) {
        this.activeKeys.delete(key);
      }
    }
    for (const key of [...this.pendingKeys]) {
      if (!desiredKeys.has(key)) {
        this.pendingKeys.delete(key);
        this.chunks.delete(key);
      }
    }
    for (const key of [...this.readyInactiveKeys]) {
      if (!desiredKeys.has(key)) {
        this.readyInactiveKeys.delete(key);
        this.chunks.delete(key);
      }
    }
  }

  prepareCoverage(request: LodDebugCoverageRequest): void {
    this.enqueueMissingDesiredChunks(this.computeDesiredKeys(request));
  }

  private enqueueMissingDesiredChunks(desiredKeys: Set<LodChunkKey>): void {
    for (const key of desiredKeys) {
      if (this.activeKeys.has(key) || this.pendingKeys.has(key) || this.readyInactiveKeys.has(key)) {
        continue;
      }
      this.pendingKeys.add(key);
    }
  }

  completePending(limit = Number.POSITIVE_INFINITY): number {
    let completed = 0;
    for (const key of [...this.pendingKeys].sort(compareLodChunkKeys)) {
      if (completed >= limit) {
        break;
      }
      const chunk = this.chunks.get(key) ?? this.createOrRegenerateChunk(key, false);
      this.meshChunk(chunk);
      chunk.renderReady = true;
      this.pendingKeys.delete(key);
      this.readyInactiveKeys.add(key);
      completed += 1;
      this.activateReadyChunks();
    }
    this.activateReadyChunks();
    return completed;
  }

  setEdit(x: number, y: number, z: number, material = LOD_DEBUG_MATERIAL.edited): void {
    this.editMaterials.set(toVoxelKey(x, y, z), material);
    const baseCx = Math.floor(x / this.chunkSize);
    const baseCy = Math.floor(y / this.chunkSize);
    const baseCz = Math.floor(z / this.chunkSize);
    const affectedKeys = [...this.chunks.keys()].filter((key) => {
      const parsed = parseLodChunkKey(key);
      if (!parsed) {
        return false;
      }
      const stride = 1 << parsed.level;
      const worldSize = this.chunkSize * stride;
      const minCx = Math.floor(parsed.cx * worldSize / this.chunkSize);
      const maxCx = Math.floor(((parsed.cx + 1) * worldSize - 1) / this.chunkSize);
      const minCy = Math.floor(parsed.cy * worldSize / this.chunkSize);
      const maxCy = Math.floor(((parsed.cy + 1) * worldSize - 1) / this.chunkSize);
      const minCz = Math.floor(parsed.cz * worldSize / this.chunkSize);
      const maxCz = Math.floor(((parsed.cz + 1) * worldSize - 1) / this.chunkSize);
      return baseCx >= minCx && baseCx <= maxCx && baseCy >= minCy && baseCy <= maxCy && baseCz >= minCz && baseCz <= maxCz;
    });
    for (const key of affectedKeys) {
      const wasActive = this.activeKeys.has(key);
      this.createOrRegenerateChunk(key, wasActive);
    }
  }

  sampleActiveMaterialAtWorldVoxel(x: number, y: number, z: number): number {
    for (const key of [...this.activeKeys].sort(compareLodChunkKeys)) {
      const parsed = parseLodChunkKey(key);
      const chunk = this.chunks.get(key);
      if (!parsed || !chunk) {
        continue;
      }
      const stride = 1 << parsed.level;
      const worldSize = this.chunkSize * stride;
      const localX = Math.floor((x - parsed.cx * worldSize) / stride);
      const localY = Math.floor((y - parsed.cy * worldSize) / stride);
      const localZ = Math.floor((z - parsed.cz * worldSize) / stride);
      if (
        localX < 0 || localX >= this.chunkSize ||
        localY < 0 || localY >= this.chunkSize ||
        localZ < 0 || localZ >= this.chunkSize
      ) {
        continue;
      }
      const material = chunk.data[localX + localY * this.chunkSize + localZ * this.chunkSize * this.chunkSize] ?? 0;
      if (material !== 0) {
        return material;
      }
    }
    return 0;
  }

  reportCoverage(request: LodDebugCoverageRequest): LodDebugCoverageReport {
    let checkedColumns = 0;
    let gaps = 0;
    let overlaps = 0;
    const centerLod1X = Math.floor(request.centerChunkX / 2);
    const centerLod1Z = Math.floor(request.centerChunkZ / 2);
    const minLod1X = centerLod1X - request.farRadiusLod1Chunks;
    const maxLod1X = centerLod1X + request.farRadiusLod1Chunks;
    const minLod1Z = centerLod1Z - request.farRadiusLod1Chunks;
    const maxLod1Z = centerLod1Z + request.farRadiusLod1Chunks;

    for (let baseCz = minLod1Z * 2; baseCz <= maxLod1Z * 2 + 1; baseCz += 1) {
      for (let baseCx = minLod1X * 2; baseCx <= maxLod1X * 2 + 1; baseCx += 1) {
        checkedColumns += 1;
        let owners = 0;
        for (const key of this.activeKeys) {
          if (chunkCoversBaseColumn(key, baseCx, baseCz, this.chunkSize)) {
            owners += 1;
          }
        }
        if (owners === 0) {
          gaps += 1;
        } else if (owners > 1) {
          overlaps += 1;
        }
      }
    }

    return {
      checkedColumns,
      gaps,
      overlaps,
      activeChunks: this.activeKeys.size,
      pendingChunks: this.pendingKeys.size + this.readyInactiveKeys.size,
    };
  }

  getStats(): WorldStats {
    let solidVoxelCount = 0;
    for (const chunk of this.iterateResidentChunks()) {
      solidVoxelCount += chunk.solidCount;
    }
    return {
      solidVoxelCount,
      chunkCount: this.activeKeys.size,
      paletteCount: this.palette.length - 1,
    };
  }

  private computeDesiredKeys(request: LodDebugCoverageRequest): Set<LodChunkKey> {
    const desired = new Set<LodChunkKey>();
    const centerLod1X = Math.floor(request.centerChunkX / 2);
    const centerLod1Z = Math.floor(request.centerChunkZ / 2);

    for (let lz = centerLod1Z - request.farRadiusLod1Chunks; lz <= centerLod1Z + request.farRadiusLod1Chunks; lz += 1) {
      for (let lx = centerLod1X - request.farRadiusLod1Chunks; lx <= centerLod1X + request.farRadiusLod1Chunks; lx += 1) {
        const distance = Math.max(Math.abs(lx - centerLod1X), Math.abs(lz - centerLod1Z));
        if (distance <= request.nearRadiusLod1Chunks) {
          for (let dz = 0; dz < 2; dz += 1) {
            for (let dx = 0; dx < 2; dx += 1) {
              desired.add(toLodChunkKey(0, lx * 2 + dx, 0, lz * 2 + dz));
            }
          }
        } else {
          desired.add(toLodChunkKey(1, lx, 0, lz));
        }
      }
    }
    return desired;
  }

  private createOrRegenerateChunk(key: LodChunkKey, renderReady: boolean): VoxelChunk {
    const parsed = parseLodChunkKey(key);
    if (!parsed) {
      throw new Error(`Invalid LOD debug chunk key ${key}`);
    }
    const stride = 1 << parsed.level;
    const data = new Uint16Array(this.chunkSize * this.chunkSize * this.chunkSize);
    const defaultMaterial = parsed.level === 0 ? LOD_DEBUG_MATERIAL.lod0 : LOD_DEBUG_MATERIAL.lod1;
    let solidCount = 0;
    let minX = this.chunkSize;
    let minY = this.chunkSize;
    let minZ = this.chunkSize;
    let maxX = 0;
    let maxY = 0;
    let maxZ = 0;

    for (let lz = 0; lz < this.chunkSize; lz += 1) {
      for (let ly = 0; ly < this.chunkSize; ly += 1) {
        for (let lx = 0; lx < this.chunkSize; lx += 1) {
          const worldX = (parsed.cx * this.chunkSize + lx) * stride;
          const worldY = (parsed.cy * this.chunkSize + ly) * stride;
          const worldZ = (parsed.cz * this.chunkSize + lz) * stride;
          const material = this.sampleFootprintMaterial(worldX, worldY, worldZ, stride, defaultMaterial);
          if (material === 0) {
            continue;
          }
          data[lx + ly * this.chunkSize + lz * this.chunkSize * this.chunkSize] = material;
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

    const chunk: VoxelChunk = {
      coord: { x: parsed.cx, y: parsed.cy, z: parsed.cz },
      lodLevel: parsed.level,
      voxelStride: stride,
      data,
      solidCount,
      solidBounds: solidCount === 0
        ? null
        : { min: [minX, minY, minZ], max: [maxX, maxY, maxZ], dirty: false },
      meshBuilt: false,
      meshDirty: true,
      renderReady: false,
      meshRevision: (this.chunks.get(key)?.meshRevision ?? 0) + 1,
      pendingMeshRevision: null,
      gpuDirty: true,
      mesh: null,
    };
    this.chunks.set(key, chunk);
    if (renderReady) {
      this.meshChunk(chunk);
      chunk.renderReady = true;
    }
    return chunk;
  }

  private sampleFootprintMaterial(worldX: number, worldY: number, worldZ: number, stride: number, defaultMaterial: number): number {
    let fallback = 0;
    for (let dz = 0; dz < stride; dz += 1) {
      for (let dy = stride - 1; dy >= 0; dy -= 1) {
        for (let dx = 0; dx < stride; dx += 1) {
          const material = this.sampleBaseMaterial(worldX + dx, worldY + dy, worldZ + dz, defaultMaterial);
          if (material === LOD_DEBUG_MATERIAL.edited) {
            return material;
          }
          if (material !== 0) {
            fallback = material;
          }
        }
      }
    }
    return fallback;
  }

  private sampleBaseMaterial(x: number, y: number, z: number, defaultMaterial: number): number {
    const edit = this.editMaterials.get(toVoxelKey(x, y, z));
    if (edit !== undefined) {
      return edit;
    }
    const height = 7
      + Math.floor((Math.sin(x * 0.11) + Math.cos(z * 0.13)) * 2)
      + (((Math.floor(x / 11) + Math.floor(z / 13)) % 3 + 3) % 3);
    return y >= 0 && y <= height ? defaultMaterial : 0;
  }

  private meshChunk(chunk: VoxelChunk): void {
    const parsed = { level: chunk.lodLevel, cx: chunk.coord.x, cy: chunk.coord.y, cz: chunk.coord.z };
    const stride = 1 << parsed.level;
    const worldSize = this.chunkSize * stride;
    const opaqueMesh = buildOpaqueChunkMeshFromInput(
      {
        chunkSize: this.chunkSize,
        coord: { x: parsed.cx, y: parsed.cy, z: parsed.cz },
        chunkData: chunk.data,
        solidCount: chunk.solidCount,
        solidBounds: chunk.solidBounds
          ? { min: [...chunk.solidBounds.min], max: [...chunk.solidBounds.max] }
          : null,
        neighbors: [
          [
            this.resolveNeighborFace(parsed.level, parsed.cx - 1, parsed.cy, parsed.cz, 0, true),
            this.resolveNeighborFace(parsed.level, parsed.cx + 1, parsed.cy, parsed.cz, 0, false),
          ],
          [
            this.resolveNeighborFace(parsed.level, parsed.cx, parsed.cy - 1, parsed.cz, 1, true),
            this.resolveNeighborFace(parsed.level, parsed.cx, parsed.cy + 1, parsed.cz, 1, false),
          ],
          [
            this.resolveNeighborFace(parsed.level, parsed.cx, parsed.cy, parsed.cz - 1, 2, true),
            this.resolveNeighborFace(parsed.level, parsed.cx, parsed.cy, parsed.cz + 1, 2, false),
          ],
        ],
      },
      this.materialLut,
    );
    if (stride > 1 && opaqueMesh.vertexCount > 0) {
      scaleVertexPositions(
        opaqueMesh.vertexData,
        opaqueMesh.vertexCount,
        parsed.cx * this.chunkSize,
        parsed.cy * this.chunkSize,
        parsed.cz * this.chunkSize,
        parsed.cx * worldSize,
        parsed.cy * worldSize,
        parsed.cz * worldSize,
        stride,
      );
      if (opaqueMesh.bounds) {
        opaqueMesh.bounds.min[0] = parsed.cx * worldSize + (opaqueMesh.bounds.min[0] - parsed.cx * this.chunkSize) * stride;
        opaqueMesh.bounds.min[1] = parsed.cy * worldSize + (opaqueMesh.bounds.min[1] - parsed.cy * this.chunkSize) * stride - stride;
        opaqueMesh.bounds.min[2] = parsed.cz * worldSize + (opaqueMesh.bounds.min[2] - parsed.cz * this.chunkSize) * stride;
        opaqueMesh.bounds.max[0] = parsed.cx * worldSize + (opaqueMesh.bounds.max[0] - parsed.cx * this.chunkSize) * stride;
        opaqueMesh.bounds.max[1] = parsed.cy * worldSize + (opaqueMesh.bounds.max[1] - parsed.cy * this.chunkSize) * stride - stride;
        opaqueMesh.bounds.max[2] = parsed.cz * worldSize + (opaqueMesh.bounds.max[2] - parsed.cz * this.chunkSize) * stride;
      }
    }

    chunk.mesh = {
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
        min: [parsed.cx * worldSize, parsed.cy * worldSize, parsed.cz * worldSize],
        max: [(parsed.cx + 1) * worldSize, (parsed.cy + 1) * worldSize, (parsed.cz + 1) * worldSize],
      },
    } satisfies ChunkMeshData;
    chunk.meshBuilt = true;
    chunk.meshDirty = false;
    chunk.gpuDirty = true;
  }

  private resolveNeighborFace(
    level: number,
    cx: number,
    cy: number,
    cz: number,
    axis: number,
    negativeSide: boolean,
  ): OpaqueChunkNeighborFaceSnapshot {
    const key = toLodChunkKey(level, cx, cy, cz);
    const chunk = this.chunks.get(key);
    if (!chunk || chunk.solidCount === 0) {
      return { faceData: null, solidBounds: null };
    }
    return {
      faceData: extractNeighborFaceData(chunk.data, axis, negativeSide, this.chunkSize),
      solidBounds: chunk.solidBounds
        ? { min: [...chunk.solidBounds.min], max: [...chunk.solidBounds.max] }
        : null,
    };
  }

  private activateReadyChunks(): void {
    let changed = true;
    while (changed) {
      changed = false;
      const activeAndReady = new Set([...this.activeKeys, ...this.readyInactiveKeys]);
      for (const coarseKey of [...this.activeKeys].sort(compareLodChunkKeys)) {
        const coarse = parseLodChunkKey(coarseKey);
        if (!coarse || coarse.level === 0) {
          continue;
        }
        if (!allBaseColumnsCoveredByFiner(coarseKey, activeAndReady, this.chunkSize)) {
          continue;
        }
        for (const readyKey of [...this.readyInactiveKeys]) {
          const ready = parseLodChunkKey(readyKey);
          if (!ready || ready.level >= coarse.level || !chunksOverlap(readyKey, coarseKey)) {
            continue;
          }
          this.readyInactiveKeys.delete(readyKey);
          this.activeKeys.add(readyKey);
        }
        this.activeKeys.delete(coarseKey);
        changed = true;
      }

      for (const readyKey of [...this.readyInactiveKeys]) {
        if (this.overlapsActiveCoarserChunk(readyKey)) {
          continue;
        }
        this.readyInactiveKeys.delete(readyKey);
        this.activeKeys.add(readyKey);
        changed = true;
      }
    }
  }

  private overlapsActiveCoarserChunk(key: LodChunkKey): boolean {
    const parsed = parseLodChunkKey(key);
    if (!parsed) {
      return false;
    }
    for (const activeKey of this.activeKeys) {
      const active = parseLodChunkKey(activeKey);
      if (!active || active.level <= parsed.level) {
        continue;
      }
      if (chunksOverlap(key, activeKey)) {
        return true;
      }
    }
    return false;
  }
}

export function createLodDebugWorld(): LodDebugWorld {
  const world = new LodDebugWorld();
  world.requestCoverage({
    centerChunkX: 0,
    centerChunkZ: 0,
    nearRadiusLod1Chunks: 1,
    farRadiusLod1Chunks: 5,
  });
  world.completePending();
  return world;
}

function toLodChunkKey(level: number, cx: number, cy: number, cz: number): LodChunkKey {
  return `L${level}:${cx}:${cy}:${cz}`;
}

function parseLodChunkKey(key: string): ParsedLodChunkKey | null {
  const match = /^L(\d+):(-?\d+):(-?\d+):(-?\d+)$/.exec(key);
  if (!match) {
    return null;
  }
  return {
    level: Number.parseInt(match[1]!, 10),
    cx: Number.parseInt(match[2]!, 10),
    cy: Number.parseInt(match[3]!, 10),
    cz: Number.parseInt(match[4]!, 10),
  };
}

function compareLodChunkKeys(left: string, right: string): number {
  const a = parseLodChunkKey(left)!;
  const b = parseLodChunkKey(right)!;
  return b.level - a.level || a.cz - b.cz || a.cx - b.cx || a.cy - b.cy;
}

function toVoxelKey(x: number, y: number, z: number): string {
  return `${x}:${y}:${z}`;
}

function chunkCoversBaseColumn(key: LodChunkKey, baseCx: number, baseCz: number, chunkSize: number): boolean {
  const parsed = parseLodChunkKey(key);
  if (!parsed) {
    return false;
  }
  const stride = 1 << parsed.level;
  const minBaseCx = parsed.cx * stride;
  const maxBaseCx = Math.floor(((parsed.cx + 1) * chunkSize * stride - 1) / chunkSize);
  const minBaseCz = parsed.cz * stride;
  const maxBaseCz = Math.floor(((parsed.cz + 1) * chunkSize * stride - 1) / chunkSize);
  return baseCx >= minBaseCx && baseCx <= maxBaseCx && baseCz >= minBaseCz && baseCz <= maxBaseCz;
}

function chunksOverlap(leftKey: LodChunkKey, rightKey: LodChunkKey): boolean {
  const left = chunkRect(leftKey);
  const right = chunkRect(rightKey);
  return left.minX < right.maxX && left.maxX > right.minX && left.minZ < right.maxZ && left.maxZ > right.minZ;
}

function chunkRect(key: LodChunkKey): { minX: number; maxX: number; minZ: number; maxZ: number } {
  const parsed = parseLodChunkKey(key)!;
  const stride = 1 << parsed.level;
  return {
    minX: parsed.cx * stride,
    maxX: (parsed.cx + 1) * stride,
    minZ: parsed.cz * stride,
    maxZ: (parsed.cz + 1) * stride,
  };
}

function allBaseColumnsCoveredByFiner(key: LodChunkKey, activeKeys: Set<LodChunkKey>, chunkSize: number): boolean {
  const parsed = parseLodChunkKey(key);
  if (!parsed) {
    return false;
  }
  const stride = 1 << parsed.level;
  for (let dz = 0; dz < stride; dz += 1) {
    for (let dx = 0; dx < stride; dx += 1) {
      const baseCx = parsed.cx * stride + dx;
      const baseCz = parsed.cz * stride + dz;
      let covered = false;
      for (const activeKey of activeKeys) {
        const active = parseLodChunkKey(activeKey);
        if (!active || active.level >= parsed.level) {
          continue;
        }
        if (chunkCoversBaseColumn(activeKey, baseCx, baseCz, chunkSize)) {
          covered = true;
          break;
        }
      }
      if (!covered) {
        return false;
      }
    }
  }
  return true;
}

function extractNeighborFaceData(
  data: Uint16Array,
  axis: number,
  negativeSide: boolean,
  chunkSize: number,
): Uint16Array {
  const chunkArea = chunkSize * chunkSize;
  const faceData = new Uint16Array(chunkArea);
  if (axis === 0) {
    const localX = negativeSide ? chunkSize - 1 : 0;
    for (let z = 0; z < chunkSize; z += 1) {
      const sourcePlaneOffset = localX + z * chunkArea;
      const faceRowOffset = z * chunkSize;
      for (let y = 0; y < chunkSize; y += 1) {
        faceData[y + faceRowOffset] = data[sourcePlaneOffset + y * chunkSize]!;
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
        faceData[x + faceRowOffset] = data[sourcePlaneOffset + x]!;
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
      faceData[x + faceRowOffset] = data[sourceRowOffset + x]!;
    }
  }
  return faceData;
}

function scaleVertexPositions(
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
  for (let i = 0; i < vertexCount; i += 1) {
    const base = i * 5;
    floats[base] = worldOriginX + (floats[base]! - mesherOriginX) * stride;
    floats[base + 1] = worldOriginY + (floats[base + 1]! - mesherOriginY) * stride - stride;
    floats[base + 2] = worldOriginZ + (floats[base + 2]! - mesherOriginZ) * stride;
  }
}
