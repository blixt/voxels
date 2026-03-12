import type { ChunkBounds, ChunkCoordinate } from "./types.ts";

const FLOAT32_BYTES = 4;
const VERTEX_STRIDE = 20;
const NORMAL_SCALE = 127;
const QUAD_STRIDE = 11;
const MAX_MESHER_SCRATCH_POOL = 2;

interface MesherScratch {
  quads: number[];
  mask: Int32Array;
}

export interface LocalChunkBounds {
  min: [number, number, number];
  max: [number, number, number];
}

export interface OpaqueChunkNeighborSnapshot {
  data: Uint16Array | null;
  solidBounds: LocalChunkBounds | null;
}

export interface OpaqueChunkMeshingInput {
  chunkSize: number;
  coord: ChunkCoordinate;
  chunkData: Uint16Array;
  solidCount: number;
  solidBounds: LocalChunkBounds | null;
  neighbors: [
    [OpaqueChunkNeighborSnapshot, OpaqueChunkNeighborSnapshot],
    [OpaqueChunkNeighborSnapshot, OpaqueChunkNeighborSnapshot],
    [OpaqueChunkNeighborSnapshot, OpaqueChunkNeighborSnapshot],
  ];
}

export interface MeshMaterialLut {
  colors: Uint32Array;
  opaqueMask: Uint8Array;
}

export interface OpaqueChunkMeshGeometry {
  vertexData: ArrayBuffer;
  vertexCount: number;
  indexData: Uint32Array;
  indexCount: number;
  triangleCount: number;
  bounds: ChunkBounds | null;
}

const mesherScratchPool: MesherScratch[] = [];

export function createMeshMaterialLut(
  palette: readonly number[],
  isWaterMaterial: (materialIndex: number) => boolean,
): MeshMaterialLut {
  const colors = new Uint32Array(palette.length);
  const opaqueMask = new Uint8Array(palette.length);
  for (let materialIndex = 0; materialIndex < palette.length; materialIndex += 1) {
    colors[materialIndex] = palette[materialIndex] ?? 0;
    opaqueMask[materialIndex] = materialIndex !== 0 && !isWaterMaterial(materialIndex) ? 1 : 0;
  }
  return { colors, opaqueMask };
}

export function cloneOpaqueChunkMeshingInput(input: OpaqueChunkMeshingInput): OpaqueChunkMeshingInput {
  return {
    chunkSize: input.chunkSize,
    coord: { ...input.coord },
    chunkData: input.chunkData.slice(),
    solidCount: input.solidCount,
    solidBounds: cloneLocalBounds(input.solidBounds),
    neighbors: [
      [cloneNeighbor(input.neighbors[0][0]), cloneNeighbor(input.neighbors[0][1])],
      [cloneNeighbor(input.neighbors[1][0]), cloneNeighbor(input.neighbors[1][1])],
      [cloneNeighbor(input.neighbors[2][0]), cloneNeighbor(input.neighbors[2][1])],
    ],
  };
}

export function buildOpaqueChunkMeshFromInput(
  input: OpaqueChunkMeshingInput,
  materialLut: MeshMaterialLut,
): OpaqueChunkMeshGeometry {
  const chunkSize = input.chunkSize;
  const originX = input.coord.x * chunkSize;
  const originY = input.coord.y * chunkSize;
  const originZ = input.coord.z * chunkSize;
  const fallbackBounds = {
    min: [originX, originY, originZ] as [number, number, number],
    max: [originX + chunkSize, originY + chunkSize, originZ + chunkSize] as [number, number, number],
  };
  if (input.solidCount === 0 || !input.solidBounds) {
    return createEmptyOpaqueChunkMesh(null);
  }

  const chunkArea = chunkSize * chunkSize;
  const chunkVolume = chunkSize * chunkArea;
  if (input.solidCount === chunkVolume && isChunkFullyOccluded(input.neighbors, chunkSize, chunkArea)) {
    return createEmptyOpaqueChunkMesh(null);
  }

  const scratch = acquireMesherScratch(Math.max(
    (input.solidBounds.max[1] - input.solidBounds.min[1]) * (input.solidBounds.max[2] - input.solidBounds.min[2]),
    (input.solidBounds.max[0] - input.solidBounds.min[0]) * (input.solidBounds.max[2] - input.solidBounds.min[2]),
    (input.solidBounds.max[0] - input.solidBounds.min[0]) * (input.solidBounds.max[1] - input.solidBounds.min[1]),
  ));
  const quads = scratch.quads;
  const mask = scratch.mask;

  for (let axis = 0; axis < 3; axis += 1) {
    const u = (axis + 1) % 3;
    const v = (axis + 2) % 3;
    const uSpan = input.solidBounds.max[u] - input.solidBounds.min[u];
    const vSpan = input.solidBounds.max[v] - input.solidBounds.min[v];
    const x = [...input.solidBounds.min];
    const q = [0, 0, 0];
    q[axis] = 1;

    for (x[axis] = input.solidBounds.min[axis] - 1; x[axis] < input.solidBounds.max[axis]; x[axis] += 1) {
      let maskIndex = 0;
      const negativeNeighbor = input.neighbors[axis][0];
      const positiveNeighbor = input.neighbors[axis][1];
      for (x[v] = input.solidBounds.min[v]; x[v] < input.solidBounds.max[v]; x[v] += 1) {
        for (x[u] = input.solidBounds.min[u]; x[u] < input.solidBounds.max[u]; x[u] += 1) {
          const a = x[axis] >= 0
            ? sampleChunkVoxel(input.chunkData, chunkSize, chunkArea, x[0]!, x[1]!, x[2]!)
            : sampleNeighborVoxel(
                negativeNeighbor.data,
                axis,
                true,
                x[0]!,
                x[1]!,
                x[2]!,
                chunkSize,
                chunkArea,
              );
          const b = x[axis] + 1 < chunkSize
            ? sampleChunkVoxel(input.chunkData, chunkSize, chunkArea, x[0] + q[0]!, x[1] + q[1]!, x[2] + q[2]!)
            : sampleNeighborVoxel(
                positiveNeighbor.data,
                axis,
                false,
                x[0] + q[0]!,
                x[1] + q[1]!,
                x[2] + q[2]!,
                chunkSize,
                chunkArea,
              );
          const opaqueA = isOpaqueMaterial(materialLut, a) ? a : 0;
          const opaqueB = isOpaqueMaterial(materialLut, b) ? b : 0;
          mask[maskIndex] = (opaqueA !== 0) === (opaqueB !== 0)
            ? 0
            : opaqueA !== 0
            ? opaqueA
            : -opaqueB;
          maskIndex += 1;
        }
      }

      maskIndex = 0;
      for (let row = 0; row < vSpan; row += 1) {
        for (let column = 0; column < uSpan; ) {
          const current = mask[maskIndex];
          if (current === 0) {
            column += 1;
            maskIndex += 1;
            continue;
          }

          let width = 1;
          while (column + width < uSpan && mask[maskIndex + width] === current) {
            width += 1;
          }

          let height = 1;
          let shouldGrow = true;
          while (row + height < vSpan && shouldGrow) {
            for (let offset = 0; offset < width; offset += 1) {
              if (mask[maskIndex + offset + height * uSpan] !== current) {
                shouldGrow = false;
                break;
              }
            }
            if (shouldGrow) {
              height += 1;
            }
          }

          quads.push(
            originX
              + (axis === 0 ? x[axis] + 1 : 0)
              + (u === 0 ? input.solidBounds.min[u] + column : 0)
              + (v === 0 ? input.solidBounds.min[v] + row : 0),
            originY
              + (axis === 1 ? x[axis] + 1 : 0)
              + (u === 1 ? input.solidBounds.min[u] + column : 0)
              + (v === 1 ? input.solidBounds.min[v] + row : 0),
            originZ
              + (axis === 2 ? x[axis] + 1 : 0)
              + (u === 2 ? input.solidBounds.min[u] + column : 0)
              + (v === 2 ? input.solidBounds.min[v] + row : 0),
            u === 0 ? width : 0,
            u === 1 ? width : 0,
            u === 2 ? width : 0,
            v === 0 ? height : 0,
            v === 1 ? height : 0,
            v === 2 ? height : 0,
            axis,
            current,
          );

          for (let dy = 0; dy < height; dy += 1) {
            for (let dx = 0; dx < width; dx += 1) {
              mask[maskIndex + dx + dy * uSpan] = 0;
            }
          }

          column += width;
          maskIndex += width;
        }
      }
    }
  }

  const geometry = buildMeshGeometryFromQuads(quads, materialLut);
  releaseMesherScratch(scratch);
  return geometry.bounds
    ? geometry
    : createEmptyOpaqueChunkMesh(fallbackBounds);
}

function createEmptyOpaqueChunkMesh(bounds: ChunkBounds | null): OpaqueChunkMeshGeometry {
  return {
    vertexData: new ArrayBuffer(0),
    vertexCount: 0,
    indexData: new Uint32Array(0),
    indexCount: 0,
    triangleCount: 0,
    bounds,
  };
}

function cloneNeighbor(neighbor: OpaqueChunkNeighborSnapshot): OpaqueChunkNeighborSnapshot {
  return {
    data: neighbor.data?.slice() ?? null,
    solidBounds: cloneLocalBounds(neighbor.solidBounds),
  };
}

function cloneLocalBounds(bounds: LocalChunkBounds | null): LocalChunkBounds | null {
  if (!bounds) {
    return null;
  }
  return {
    min: [...bounds.min],
    max: [...bounds.max],
  };
}

function isOpaqueMaterial(materialLut: MeshMaterialLut, material: number): boolean {
  return materialLut.opaqueMask[material] === 1;
}

function sampleChunkVoxel(
  chunkData: Uint16Array,
  chunkSize: number,
  chunkArea: number,
  x: number,
  y: number,
  z: number,
): number {
  return chunkData[x + y * chunkSize + z * chunkArea] ?? 0;
}

function isChunkFullyOccluded(
  neighbors: OpaqueChunkMeshingInput["neighbors"],
  chunkSize: number,
  chunkArea: number,
): boolean {
  return isNeighborFaceSolid(neighbors[0][0], chunkSize, chunkArea, "x", chunkSize - 1)
    && isNeighborFaceSolid(neighbors[0][1], chunkSize, chunkArea, "x", 0)
    && isNeighborFaceSolid(neighbors[1][0], chunkSize, chunkArea, "y", chunkSize - 1)
    && isNeighborFaceSolid(neighbors[1][1], chunkSize, chunkArea, "y", 0)
    && isNeighborFaceSolid(neighbors[2][0], chunkSize, chunkArea, "z", chunkSize - 1)
    && isNeighborFaceSolid(neighbors[2][1], chunkSize, chunkArea, "z", 0);
}

function isNeighborFaceSolid(
  neighbor: OpaqueChunkNeighborSnapshot,
  chunkSize: number,
  chunkArea: number,
  axis: "x" | "y" | "z",
  faceIndex: number,
): boolean {
  if (!neighbor.data || !neighbor.solidBounds) {
    return false;
  }
  const bounds = neighbor.solidBounds;
  if (axis === "x" && (bounds.min[0] > faceIndex || bounds.max[0] <= faceIndex)) {
    return false;
  }
  if (axis === "y" && (bounds.min[1] > faceIndex || bounds.max[1] <= faceIndex)) {
    return false;
  }
  if (axis === "z" && (bounds.min[2] > faceIndex || bounds.max[2] <= faceIndex)) {
    return false;
  }
  const data = neighbor.data;
  if (axis === "x") {
    for (let z = 0; z < chunkSize; z += 1) {
      const planeOffset = z * chunkArea;
      for (let y = 0; y < chunkSize; y += 1) {
        if (data[faceIndex + y * chunkSize + planeOffset] === 0) {
          return false;
        }
      }
    }
    return true;
  }
  if (axis === "y") {
    const planeOffset = faceIndex * chunkSize;
    for (let z = 0; z < chunkSize; z += 1) {
      const rowOffset = z * chunkArea + planeOffset;
      for (let x = 0; x < chunkSize; x += 1) {
        if (data[x + rowOffset] === 0) {
          return false;
        }
      }
    }
    return true;
  }
  for (let y = 0; y < chunkSize; y += 1) {
    const rowOffset = y * chunkSize + faceIndex * chunkArea;
    for (let x = 0; x < chunkSize; x += 1) {
      if (data[x + rowOffset] === 0) {
        return false;
      }
    }
  }
  return true;
}

function sampleNeighborVoxel(
  data: Uint16Array | null,
  axis: number,
  negative: boolean,
  x: number,
  y: number,
  z: number,
  chunkSize: number,
  chunkArea: number,
): number {
  if (!data) {
    return 0;
  }
  if (axis === 0) {
    const localX = negative ? chunkSize - 1 : 0;
    return data[localX + y * chunkSize + z * chunkArea] ?? 0;
  }
  if (axis === 1) {
    const localY = negative ? chunkSize - 1 : 0;
    return data[x + localY * chunkSize + z * chunkArea] ?? 0;
  }
  const localZ = negative ? chunkSize - 1 : 0;
  return data[x + y * chunkSize + localZ * chunkArea] ?? 0;
}

function buildMeshGeometryFromQuads(
  quads: readonly number[],
  materialLut: MeshMaterialLut,
): OpaqueChunkMeshGeometry {
  const quadCount = quads.length / QUAD_STRIDE;
  const vertexCount = quadCount * 4;
  const indexCount = quadCount * 6;
  const vertexData = new ArrayBuffer(vertexCount * VERTEX_STRIDE);
  const vertexView = new DataView(vertexData);
  const indexData = new Uint32Array(indexCount);

  let vertexOffset = 0;
  let indexOffset = 0;
  let baseVertex = 0;
  let minX = Infinity;
  let minY = Infinity;
  let minZ = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  let maxZ = -Infinity;

  for (let quadIndex = 0; quadIndex < quads.length; quadIndex += QUAD_STRIDE) {
    const positionX = quads[quadIndex]!;
    const positionY = quads[quadIndex + 1]!;
    const positionZ = quads[quadIndex + 2]!;
    const duX = quads[quadIndex + 3]!;
    const duY = quads[quadIndex + 4]!;
    const duZ = quads[quadIndex + 5]!;
    const dvX = quads[quadIndex + 6]!;
    const dvY = quads[quadIndex + 7]!;
    const dvZ = quads[quadIndex + 8]!;
    const axis = quads[quadIndex + 9]!;
    const face = quads[quadIndex + 10]!;
    const normal = face > 0 ? 1 : -1;
    const material = normal === 1 ? face : -face;
    const color = materialLut.colors[material] ?? 0;
    const normalX = axis === 0 ? normal : 0;
    const normalY = axis === 1 ? normal : 0;
    const normalZ = axis === 2 ? normal : 0;

    const corner0X = positionX;
    const corner0Y = positionY;
    const corner0Z = positionZ;
    const corner1X = normal === 1 ? positionX + duX : positionX + dvX;
    const corner1Y = normal === 1 ? positionY + duY : positionY + dvY;
    const corner1Z = normal === 1 ? positionZ + duZ : positionZ + dvZ;
    const corner2X = positionX + duX + dvX;
    const corner2Y = positionY + duY + dvY;
    const corner2Z = positionZ + duZ + dvZ;
    const corner3X = normal === 1 ? positionX + dvX : positionX + duX;
    const corner3Y = normal === 1 ? positionY + dvY : positionY + duY;
    const corner3Z = normal === 1 ? positionZ + dvZ : positionZ + duZ;

    minX = Math.min(minX, corner0X, corner1X, corner2X, corner3X);
    minY = Math.min(minY, corner0Y, corner1Y, corner2Y, corner3Y);
    minZ = Math.min(minZ, corner0Z, corner1Z, corner2Z, corner3Z);
    maxX = Math.max(maxX, corner0X, corner1X, corner2X, corner3X);
    maxY = Math.max(maxY, corner0Y, corner1Y, corner2Y, corner3Y);
    maxZ = Math.max(maxZ, corner0Z, corner1Z, corner2Z, corner3Z);

    writeVertex(vertexView, vertexOffset, corner0X, corner0Y, corner0Z, normalX, normalY, normalZ, color);
    vertexOffset += VERTEX_STRIDE;
    writeVertex(vertexView, vertexOffset, corner1X, corner1Y, corner1Z, normalX, normalY, normalZ, color);
    vertexOffset += VERTEX_STRIDE;
    writeVertex(vertexView, vertexOffset, corner2X, corner2Y, corner2Z, normalX, normalY, normalZ, color);
    vertexOffset += VERTEX_STRIDE;
    writeVertex(vertexView, vertexOffset, corner3X, corner3Y, corner3Z, normalX, normalY, normalZ, color);
    vertexOffset += VERTEX_STRIDE;

    indexData[indexOffset + 0] = baseVertex;
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
    bounds: quadCount === 0
      ? null
      : {
          min: [minX, minY, minZ],
          max: [maxX, maxY, maxZ],
        },
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

function acquireMesherScratch(requiredMaskLength: number): MesherScratch {
  const scratch = mesherScratchPool.pop() ?? {
    quads: [],
    mask: new Int32Array(requiredMaskLength),
  };
  scratch.quads.length = 0;
  if (scratch.mask.length < requiredMaskLength) {
    scratch.mask = new Int32Array(requiredMaskLength);
  }
  return scratch;
}

function releaseMesherScratch(scratch: MesherScratch): void {
  scratch.quads.length = 0;
  if (mesherScratchPool.length >= MAX_MESHER_SCRATCH_POOL) {
    return;
  }
  mesherScratchPool.push(scratch);
}
