import type { ChunkCoordinate, PackedColor, WorldDimensions, WorldStats } from "./types.ts";

export const EMPTY_VOXEL = 0;
export const DEFAULT_CHUNK_SIZE = 32;

export interface VoxelChunk {
  coord: ChunkCoordinate;
  data: Uint16Array;
  solidCount: number;
  solidBounds: {
    min: [number, number, number];
    max: [number, number, number];
    dirty: boolean;
  } | null;
  meshBuilt: boolean;
  meshDirty: boolean;
  gpuDirty: boolean;
  mesh: import("./types.ts").ChunkMeshData | null;
}

export interface ResidentChunkWorld {
  readonly chunkSize: number;
  readonly minY: number;
  readonly maxYExclusive: number;
  getVoxel(x: number, y: number, z: number): number;
  getPaletteColor(materialIndex: number): PackedColor;
  isCollisionMaterial(materialIndex: number): boolean;
  isWaterMaterial(materialIndex: number): boolean;
  getResidentChunk(cx: number, cy: number, cz: number): VoxelChunk | null;
  hasResidentChunk(cx: number, cy: number, cz: number): boolean;
  iterateResidentChunks(): Iterable<VoxelChunk>;
  getChunkSolidBounds(
    cx: number,
    cy: number,
    cz: number,
  ): {
    min: [number, number, number];
    max: [number, number, number];
  } | null;
}

function toChunkKey(cx: number, cy: number, cz: number, sizeX: number, sizeY: number): number {
  return cx + cy * sizeX + cz * sizeX * sizeY;
}

export class VoxelWorld implements ResidentChunkWorld {
  readonly width: number;
  readonly height: number;
  readonly depth: number;
  readonly minY = 0;
  readonly chunkSize: number;
  readonly chunkCountX: number;
  readonly chunkCountY: number;
  readonly chunkCountZ: number;
  readonly palette: PackedColor[];
  readonly colorToIndex = new Map<PackedColor, number>();
  readonly chunks = new Map<number, VoxelChunk>();

  constructor(dimensions: WorldDimensions, chunkSize = DEFAULT_CHUNK_SIZE, palette: PackedColor[] = [0]) {
    this.width = dimensions.width;
    this.height = dimensions.height;
    this.depth = dimensions.depth;
    this.chunkSize = chunkSize;
    this.chunkCountX = Math.ceil(dimensions.width / chunkSize);
    this.chunkCountY = Math.ceil(dimensions.height / chunkSize);
    this.chunkCountZ = Math.ceil(dimensions.depth / chunkSize);
    this.palette = palette.length > 0 ? [...palette] : [0];
    if (this.palette[0] !== 0) {
      this.palette.unshift(0);
    }
    this.rebuildPaletteIndex();
  }

  rebuildPaletteIndex(): void {
    this.colorToIndex.clear();
    for (let index = 1; index < this.palette.length; index += 1) {
      this.colorToIndex.set(this.palette[index]!, index);
    }
  }

  inBounds(x: number, y: number, z: number): boolean {
    return x >= 0 && y >= 0 && z >= 0 && x < this.width && y < this.height && z < this.depth;
  }

  getPaletteColor(materialIndex: number): PackedColor {
    return this.palette[materialIndex] ?? 0;
  }

  isCollisionMaterial(materialIndex: number): boolean {
    return materialIndex !== EMPTY_VOXEL;
  }

  isWaterMaterial(_materialIndex: number): boolean {
    return false;
  }

  get maxYExclusive(): number {
    return this.height;
  }

  getVoxel(x: number, y: number, z: number): number {
    if (!this.inBounds(x, y, z)) {
      return EMPTY_VOXEL;
    }
    const [chunkKey, localIndex] = this.resolveVoxelAddress(x, y, z);
    const chunk = this.chunks.get(chunkKey);
    if (!chunk) {
      return EMPTY_VOXEL;
    }
    return chunk.data[localIndex] ?? EMPTY_VOXEL;
  }

  setVoxel(x: number, y: number, z: number, materialIndex: number): boolean {
    if (!this.inBounds(x, y, z)) {
      return false;
    }
    const [chunkKey, localIndex, cx, cy, cz, lx, ly, lz] = this.resolveVoxelAddress(x, y, z);
    let chunk = this.chunks.get(chunkKey);
    if (!chunk && materialIndex === EMPTY_VOXEL) {
      return false;
    }
    if (!chunk) {
      chunk = this.createChunk(chunkKey, cx, cy, cz);
    }

    const previous = chunk.data[localIndex] ?? EMPTY_VOXEL;
    if (previous === materialIndex) {
      return false;
    }

    chunk.data[localIndex] = materialIndex;
    if (previous === EMPTY_VOXEL && materialIndex !== EMPTY_VOXEL) {
      chunk.solidCount += 1;
      this.expandChunkSolidBounds(chunk, lx, ly, lz);
    } else if (previous !== EMPTY_VOXEL && materialIndex === EMPTY_VOXEL) {
      chunk.solidCount -= 1;
      this.invalidateChunkBoundsIfNeeded(chunk, lx, ly, lz);
    }

    this.markChunkDirty(chunk);
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

    if (chunk.solidCount === 0) {
      this.chunks.delete(chunkKey);
    }
    return true;
  }

  fillBox(
    startX: number,
    startY: number,
    startZ: number,
    endXExclusive: number,
    endYExclusive: number,
    endZExclusive: number,
    materialIndex: number,
  ): void {
    for (let z = startZ; z < endZExclusive; z += 1) {
      for (let x = startX; x < endXExclusive; x += 1) {
        this.fillColumn(x, z, startY, endYExclusive, materialIndex);
      }
    }
  }

  fillColumn(
    x: number,
    z: number,
    startY: number,
    endYExclusive: number,
    materialIndex: number,
  ): void {
    if (x < 0 || z < 0 || x >= this.width || z >= this.depth) {
      return;
    }
    const clippedStartY = Math.max(0, startY);
    const clippedEndY = Math.min(this.height, endYExclusive);
    if (clippedStartY >= clippedEndY) {
      return;
    }

    const cx = Math.floor(x / this.chunkSize);
    const cz = Math.floor(z / this.chunkSize);
    const lx = x - cx * this.chunkSize;
    const lz = z - cz * this.chunkSize;
    const columnBase = lx + lz * this.chunkSize * this.chunkSize;

    for (let y = clippedStartY; y < clippedEndY; ) {
      const cy = Math.floor(y / this.chunkSize);
      const localStartY = y - cy * this.chunkSize;
      const localEndY = Math.min(this.chunkSize, localStartY + clippedEndY - y);
      const chunkKey = toChunkKey(cx, cy, cz, this.chunkCountX, this.chunkCountY);
      let chunk = this.chunks.get(chunkKey);
      if (!chunk && materialIndex === EMPTY_VOXEL) {
        y += localEndY - localStartY;
        continue;
      }
      if (!chunk) {
        chunk = this.createChunk(chunkKey, cx, cy, cz);
      }

      let chunkChanged = false;
      let touchedBottomBoundary = false;
      let touchedTopBoundary = false;
      for (let ly = localStartY; ly < localEndY; ly += 1) {
        const localIndex = columnBase + ly * this.chunkSize;
        const previous = chunk.data[localIndex] ?? EMPTY_VOXEL;
        if (previous === materialIndex) {
          continue;
        }
        chunk.data[localIndex] = materialIndex;
        if (previous === EMPTY_VOXEL && materialIndex !== EMPTY_VOXEL) {
          chunk.solidCount += 1;
          this.expandChunkSolidBounds(chunk, lx, ly, lz);
        } else if (previous !== EMPTY_VOXEL && materialIndex === EMPTY_VOXEL) {
          chunk.solidCount -= 1;
          this.invalidateChunkBoundsIfNeeded(chunk, lx, ly, lz);
        }
        chunkChanged = true;
        touchedBottomBoundary ||= ly === 0;
        touchedTopBoundary ||= ly === this.chunkSize - 1;
      }

      if (chunkChanged) {
        this.markChunkDirty(chunk);
        if (lx === 0) {
          this.markChunkDirtyByCoord(cx - 1, cy, cz);
        }
        if (lx === this.chunkSize - 1) {
          this.markChunkDirtyByCoord(cx + 1, cy, cz);
        }
        if (lz === 0) {
          this.markChunkDirtyByCoord(cx, cy, cz - 1);
        }
        if (lz === this.chunkSize - 1) {
          this.markChunkDirtyByCoord(cx, cy, cz + 1);
        }
        if (touchedBottomBoundary) {
          this.markChunkDirtyByCoord(cx, cy - 1, cz);
        }
        if (touchedTopBoundary) {
          this.markChunkDirtyByCoord(cx, cy + 1, cz);
        }
        if (chunk.solidCount === 0) {
          this.chunks.delete(chunkKey);
        }
      }

      y += localEndY - localStartY;
    }
  }

  clear(): void {
    this.chunks.clear();
  }

  getResidentChunk(cx: number, cy: number, cz: number): VoxelChunk | null {
    const key = this.resolveChunkKey(cx, cy, cz);
    return key === null ? null : this.chunks.get(key) ?? null;
  }

  hasResidentChunk(cx: number, cy: number, cz: number): boolean {
    return this.getResidentChunk(cx, cy, cz) !== null;
  }

  *iterateResidentChunks(): Iterable<VoxelChunk> {
    for (const chunk of this.chunks.values()) {
      yield chunk;
    }
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

  resolveVoxelAddress(
    x: number,
    y: number,
    z: number,
  ): [chunkKey: number, localIndex: number, cx: number, cy: number, cz: number, lx: number, ly: number, lz: number] {
    const cx = Math.floor(x / this.chunkSize);
    const cy = Math.floor(y / this.chunkSize);
    const cz = Math.floor(z / this.chunkSize);
    const lx = x - cx * this.chunkSize;
    const ly = y - cy * this.chunkSize;
    const lz = z - cz * this.chunkSize;
    const localIndex = lx + ly * this.chunkSize + lz * this.chunkSize * this.chunkSize;
    const chunkKey = toChunkKey(cx, cy, cz, this.chunkCountX, this.chunkCountY);
    return [chunkKey, localIndex, cx, cy, cz, lx, ly, lz];
  }

  resolveChunkKey(cx: number, cy: number, cz: number): number | null {
    if (
      cx < 0 ||
      cy < 0 ||
      cz < 0 ||
      cx >= this.chunkCountX ||
      cy >= this.chunkCountY ||
      cz >= this.chunkCountZ
    ) {
      return null;
    }
    return toChunkKey(cx, cy, cz, this.chunkCountX, this.chunkCountY);
  }

  markChunkDirtyByCoord(cx: number, cy: number, cz: number): void {
    const key = this.resolveChunkKey(cx, cy, cz);
    if (key === null) {
      return;
    }
    const chunk = this.chunks.get(key);
    if (chunk) {
      this.markChunkDirty(chunk);
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
    const key = this.resolveChunkKey(cx, cy, cz);
    if (key === null) {
      return null;
    }
    const chunk = this.chunks.get(key);
    if (!chunk || chunk.solidCount === 0) {
      return null;
    }
    if (!chunk.solidBounds) {
      this.recomputeChunkSolidBounds(chunk);
    } else if (chunk.solidBounds.dirty) {
      this.recomputeChunkSolidBounds(chunk);
    }
    if (!chunk.solidBounds) {
      return null;
    }
    return {
      min: [...chunk.solidBounds.min],
      max: [...chunk.solidBounds.max],
    };
  }

  private createChunk(chunkKey: number, cx: number, cy: number, cz: number): VoxelChunk {
    const chunk = {
      coord: { x: cx, y: cy, z: cz },
      data: new Uint16Array(this.chunkSize * this.chunkSize * this.chunkSize),
      solidCount: 0,
      solidBounds: null,
      meshBuilt: false,
      meshDirty: true,
      gpuDirty: true,
      mesh: null,
    };
    this.chunks.set(chunkKey, chunk);
    return chunk;
  }

  private markChunkDirty(chunk: VoxelChunk): void {
    chunk.meshDirty = true;
    chunk.gpuDirty = true;
    chunk.mesh = null;
  }

  private expandChunkSolidBounds(chunk: VoxelChunk, lx: number, ly: number, lz: number): void {
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

  private invalidateChunkBoundsIfNeeded(chunk: VoxelChunk, lx: number, ly: number, lz: number): void {
    if (!chunk.solidBounds) {
      return;
    }
    if (chunk.solidCount === 0) {
      chunk.solidBounds = null;
      return;
    }
    if (
      lx === chunk.solidBounds.min[0] ||
      ly === chunk.solidBounds.min[1] ||
      lz === chunk.solidBounds.min[2] ||
      lx + 1 === chunk.solidBounds.max[0] ||
      ly + 1 === chunk.solidBounds.max[1] ||
      lz + 1 === chunk.solidBounds.max[2]
    ) {
      chunk.solidBounds.dirty = true;
    }
  }

  private recomputeChunkSolidBounds(chunk: VoxelChunk): void {
    let minX = this.chunkSize;
    let minY = this.chunkSize;
    let minZ = this.chunkSize;
    let maxX = 0;
    let maxY = 0;
    let maxZ = 0;

    for (let lz = 0; lz < this.chunkSize; lz += 1) {
      for (let ly = 0; ly < this.chunkSize; ly += 1) {
        for (let lx = 0; lx < this.chunkSize; lx += 1) {
          const localIndex = lx + ly * this.chunkSize + lz * this.chunkSize * this.chunkSize;
          if (chunk.data[localIndex] === EMPTY_VOXEL) {
            continue;
          }
          minX = Math.min(minX, lx);
          minY = Math.min(minY, ly);
          minZ = Math.min(minZ, lz);
          maxX = Math.max(maxX, lx + 1);
          maxY = Math.max(maxY, ly + 1);
          maxZ = Math.max(maxZ, lz + 1);
        }
      }
    }

    chunk.solidBounds = chunk.solidCount === 0
      ? null
      : {
          min: [minX, minY, minZ],
          max: [maxX, maxY, maxZ],
          dirty: false,
        };
  }
}
