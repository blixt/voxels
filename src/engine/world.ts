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
  meshDirty: boolean;
  gpuDirty: boolean;
  mesh: import("./types.ts").ChunkMeshData | null;
}

function toChunkKey(cx: number, cy: number, cz: number, sizeX: number, sizeY: number): number {
  return cx + cy * sizeX + cz * sizeX * sizeY;
}

export class VoxelWorld {
  readonly width: number;
  readonly height: number;
  readonly depth: number;
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
      chunk = {
        coord: { x: cx, y: cy, z: cz },
        data: new Uint16Array(this.chunkSize * this.chunkSize * this.chunkSize),
        solidCount: 0,
        solidBounds: null,
        meshDirty: true,
        gpuDirty: true,
        mesh: null,
      };
      this.chunks.set(chunkKey, chunk);
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
      for (let y = startY; y < endYExclusive; y += 1) {
        for (let x = startX; x < endXExclusive; x += 1) {
          this.setVoxel(x, y, z, materialIndex);
        }
      }
    }
  }

  clear(): void {
    this.chunks.clear();
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
