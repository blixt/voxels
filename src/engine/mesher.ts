import type { ChunkMeshData } from "./types.ts";
import type { ResidentChunkWorld } from "./world.ts";

const FLOAT32_BYTES = 4;
const VERTEX_STRIDE = 20;
const NORMAL_SCALE = 127;
const QUAD_STRIDE = 11;

export interface MeshBuildSummary {
  meshCount: number;
  newMeshCount: number;
  remeshCount: number;
  triangleCount: number;
  elapsedMs: number;
}

export function rebuildDirtyMeshes(world: ResidentChunkWorld, maxChunks = Number.POSITIVE_INFINITY): MeshBuildSummary {
  const startedAt = performance.now();
  let meshCount = 0;
  let newMeshCount = 0;
  let remeshCount = 0;
  let triangleCount = 0;
  for (const chunk of world.iterateResidentChunks()) {
    if (!chunk.meshDirty) {
      continue;
    }
    if (meshCount >= maxChunks) {
      break;
    }
    chunk.mesh = buildChunkMesh(world, chunk.coord.x, chunk.coord.y, chunk.coord.z);
    chunk.meshDirty = false;
    chunk.gpuDirty = true;
    meshCount += 1;
    if (chunk.meshBuilt) {
      remeshCount += 1;
    } else {
      newMeshCount += 1;
      chunk.meshBuilt = true;
    }
    triangleCount += chunk.mesh?.triangleCount ?? 0;
  }
  return {
    meshCount,
    newMeshCount,
    remeshCount,
    triangleCount,
    elapsedMs: performance.now() - startedAt,
  };
}

export function buildChunkMesh(world: ResidentChunkWorld, cx: number, cy: number, cz: number): ChunkMeshData {
  const chunkSize = world.chunkSize;
  const originX = cx * chunkSize;
  const originY = cy * chunkSize;
  const originZ = cz * chunkSize;
  const chunk = world.getResidentChunk(cx, cy, cz);
  if (!chunk) {
    return {
      vertexData: new ArrayBuffer(0),
      vertexCount: 0,
      indexData: new Uint32Array(0),
      indexCount: 0,
      triangleCount: 0,
      bounds: {
        min: [originX, originY, originZ],
        max: [originX + chunkSize, originY + chunkSize, originZ + chunkSize],
      },
    };
  }

  const chunkArea = chunkSize * chunkSize;
  const chunkVolume = chunkSize * chunkArea;
  if (chunk.solidCount === chunkVolume && isChunkFullyOccluded(world, cx, cy, cz, chunkSize, chunkArea)) {
    return {
      vertexData: new ArrayBuffer(0),
      vertexCount: 0,
      indexData: new Uint32Array(0),
      indexCount: 0,
      triangleCount: 0,
      bounds: {
        min: [originX, originY, originZ],
        max: [originX + chunkSize, originY + chunkSize, originZ + chunkSize],
      },
    };
  }
  const solidBounds = world.getChunkSolidBounds(cx, cy, cz);
  if (!solidBounds) {
    return {
      vertexData: new ArrayBuffer(0),
      vertexCount: 0,
      indexData: new Uint32Array(0),
      indexCount: 0,
      triangleCount: 0,
      bounds: {
        min: [originX, originY, originZ],
        max: [originX + chunkSize, originY + chunkSize, originZ + chunkSize],
      },
    };
  }

  const chunkData = chunk.data;
  const quads: number[] = [];

  for (let axis = 0; axis < 3; axis += 1) {
    const u = (axis + 1) % 3;
    const v = (axis + 2) % 3;
    const uSpan = solidBounds.max[u] - solidBounds.min[u];
    const vSpan = solidBounds.max[v] - solidBounds.min[v];
    const mask = new Int32Array(uSpan * vSpan);
    const x = [...solidBounds.min];
    const q = [0, 0, 0];
    q[axis] = 1;

    for (x[axis] = solidBounds.min[axis] - 1; x[axis] < solidBounds.max[axis]; x[axis] += 1) {
      let maskIndex = 0;
      for (x[v] = solidBounds.min[v]; x[v] < solidBounds.max[v]; x[v] += 1) {
        for (x[u] = solidBounds.min[u]; x[u] < solidBounds.max[u]; x[u] += 1) {
          const ax = originX + x[0];
          const ay = originY + x[1];
          const az = originZ + x[2];
          const bx = ax + q[0]!;
          const by = ay + q[1]!;
          const bz = az + q[2]!;
          const a = x[axis] >= 0
            ? sampleChunkVoxel(chunkData, chunkSize, chunkArea, x[0]!, x[1]!, x[2]!)
            : world.getVoxel(ax, ay, az);
          const b = x[axis] + 1 < chunkSize
            ? sampleChunkVoxel(chunkData, chunkSize, chunkArea, x[0] + q[0]!, x[1] + q[1]!, x[2] + q[2]!)
            : world.getVoxel(bx, by, bz);
          mask[maskIndex] = (a !== 0) === (b !== 0)
            ? 0
            : a !== 0
            ? a
            : -b;
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
              + (u === 0 ? solidBounds.min[u] + column : 0)
              + (v === 0 ? solidBounds.min[v] + row : 0),
            originY
              + (axis === 1 ? x[axis] + 1 : 0)
              + (u === 1 ? solidBounds.min[u] + column : 0)
              + (v === 1 ? solidBounds.min[v] + row : 0),
            originZ
              + (axis === 2 ? x[axis] + 1 : 0)
              + (u === 2 ? solidBounds.min[u] + column : 0)
              + (v === 2 ? solidBounds.min[v] + row : 0),
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
    const color = world.getPaletteColor(material);
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
    bounds: {
      min: [minX, minY, minZ],
      max: [maxX, maxY, maxZ],
    },
  };
}

function sampleChunkVoxel(
  chunkData: Uint16Array,
  chunkSize: number,
  chunkArea: number,
  x: number,
  y: number,
  z: number,
): number {
  return chunkData[x + y * chunkSize + z * chunkArea];
}

function isChunkFullyOccluded(
  world: ResidentChunkWorld,
  cx: number,
  cy: number,
  cz: number,
  chunkSize: number,
  chunkArea: number,
): boolean {
  return isNeighborFaceSolid(world, cx - 1, cy, cz, chunkSize, chunkArea, "x", chunkSize - 1)
    && isNeighborFaceSolid(world, cx + 1, cy, cz, chunkSize, chunkArea, "x", 0)
    && isNeighborFaceSolid(world, cx, cy - 1, cz, chunkSize, chunkArea, "y", chunkSize - 1)
    && isNeighborFaceSolid(world, cx, cy + 1, cz, chunkSize, chunkArea, "y", 0)
    && isNeighborFaceSolid(world, cx, cy, cz - 1, chunkSize, chunkArea, "z", chunkSize - 1)
    && isNeighborFaceSolid(world, cx, cy, cz + 1, chunkSize, chunkArea, "z", 0);
}

function isNeighborFaceSolid(
  world: ResidentChunkWorld,
  cx: number,
  cy: number,
  cz: number,
  chunkSize: number,
  chunkArea: number,
  axis: "x" | "y" | "z",
  faceIndex: number,
): boolean {
  const chunk = world.getResidentChunk(cx, cy, cz);
  if (!chunk) {
    return false;
  }
  const bounds = world.getChunkSolidBounds(cx, cy, cz);
  if (!bounds) {
    return false;
  }
  if (axis === "x" && (bounds.min[0] > faceIndex || bounds.max[0] <= faceIndex)) {
    return false;
  }
  if (axis === "y" && (bounds.min[1] > faceIndex || bounds.max[1] <= faceIndex)) {
    return false;
  }
  if (axis === "z" && (bounds.min[2] > faceIndex || bounds.max[2] <= faceIndex)) {
    return false;
  }
  const data = chunk.data;
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
