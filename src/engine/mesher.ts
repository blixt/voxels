import type { ChunkMeshData, Vec3 } from "./types.ts";
import {
  cloneOpaqueChunkMeshingInput,
  type OpaqueChunkMeshingInput,
  type OpaqueChunkMeshGeometry,
} from "./opaque-chunk-mesher.ts";
import { applyWaterDepthTint } from "./water-visuals.ts";
import { setChunkMeshDirtyState, type ResidentChunkWorld } from "./world.ts";

const VERTEX_STRIDE = 20;
const NORMAL_SCALE = 127;
const QUAD_STRIDE = 11;
const MAX_MESHER_SCRATCH_POOL = 2;
const WATER_DEPTH_BAND_SHIFT = 16;
const WATER_DEPTH_BAND_MASK = 0xff;
const WATER_DEPTH_BAND_WORLD_UNITS = 4;

interface MesherScratch {
  quads: number[];
  waterQuads: number[];
  mask: Int32Array;
}

interface ResolvedChunkNeighbor {
  data: Uint16Array | null;
  solidBounds: {
    min: [number, number, number];
    max: [number, number, number];
  } | null;
}

const mesherScratchPool: MesherScratch[] = [];

function createEmptyChunkMesh(bounds: { min: [number, number, number]; max: [number, number, number] }): ChunkMeshData {
  return {
    vertexData: new ArrayBuffer(0),
    vertexCount: 0,
    indexData: new Uint32Array(0),
    indexCount: 0,
    waterVertexData: new ArrayBuffer(0),
    waterVertexCount: 0,
    waterIndexData: new Uint32Array(0),
    waterIndexCount: 0,
    waterTriangleCount: 0,
    triangleCount: 0,
    bounds,
  };
}

export interface MeshBuildSummary {
  meshCount: number;
  newMeshCount: number;
  remeshCount: number;
  triangleCount: number;
  elapsedMs: number;
}

export interface MeshBuildOptions {
  priorityPosition?: Vec3;
}

export function rebuildDirtyMeshes(
  world: ResidentChunkWorld,
  maxChunks = Number.POSITIVE_INFINITY,
  options: MeshBuildOptions = {},
): MeshBuildSummary {
  const startedAt = performance.now();
  let meshCount = 0;
  let newMeshCount = 0;
  let remeshCount = 0;
  let triangleCount = 0;
  const dirtyChunks = collectDirtyChunks(world, options.priorityPosition);
  for (const chunk of dirtyChunks) {
    if (meshCount >= maxChunks) {
      break;
    }
    chunk.mesh = buildChunkMesh(world, chunk.coord.x, chunk.coord.y, chunk.coord.z);
    setChunkMeshDirtyState(world, chunk, false);
    chunk.pendingMeshRevision = null;
    chunk.gpuDirty = true;
    meshCount += 1;
    if (chunk.meshBuilt) {
      remeshCount += 1;
    } else {
      newMeshCount += 1;
      chunk.meshBuilt = true;
    }
    world.noteResidentChunkRenderReadyState?.(chunk, chunk.meshBuilt && chunk.mesh !== null);
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

export function collectDirtyChunks(world: ResidentChunkWorld, priorityPosition?: Vec3) {
  const dirtyChunks = [];
  for (const chunk of world.iterateDirtyResidentChunks?.() ?? world.iterateResidentChunks()) {
    if (!chunk.meshDirty) {
      continue;
    }
    dirtyChunks.push(chunk);
  }
  if (!priorityPosition || dirtyChunks.length <= 1) {
    return dirtyChunks;
  }
  const priorityChunkX = Math.floor(priorityPosition[0] / world.chunkSize);
  const priorityChunkY = Math.floor(priorityPosition[1] / world.chunkSize);
  const priorityChunkZ = Math.floor(priorityPosition[2] / world.chunkSize);
  dirtyChunks.sort((left, right) => {
    const builtDifference = Number(left.meshBuilt) - Number(right.meshBuilt);
    if (builtDifference !== 0) {
      return builtDifference;
    }
    const leftPlanarDistance = Math.max(
      Math.abs(left.coord.x - priorityChunkX),
      Math.abs(left.coord.z - priorityChunkZ),
    );
    const rightPlanarDistance = Math.max(
      Math.abs(right.coord.x - priorityChunkX),
      Math.abs(right.coord.z - priorityChunkZ),
    );
    if (leftPlanarDistance !== rightPlanarDistance) {
      return leftPlanarDistance - rightPlanarDistance;
    }
    const leftVerticalDistance = Math.abs(left.coord.y - priorityChunkY);
    const rightVerticalDistance = Math.abs(right.coord.y - priorityChunkY);
    if (leftVerticalDistance !== rightVerticalDistance) {
      return leftVerticalDistance - rightVerticalDistance;
    }
    return left.coord.x - right.coord.x || left.coord.y - right.coord.y || left.coord.z - right.coord.z;
  });
  return dirtyChunks;
}

export function createOpaqueChunkMeshingInput(
  world: ResidentChunkWorld,
  cx: number,
  cy: number,
  cz: number,
  options: { cloneData?: boolean } = {},
): OpaqueChunkMeshingInput | null {
  const chunk = world.getResidentChunk(cx, cy, cz);
  if (!chunk) {
    return null;
  }
  const neighbors = resolveChunkNeighbors(world, cx, cy, cz);
  const input: OpaqueChunkMeshingInput = {
    chunkSize: world.chunkSize,
    coord: { x: cx, y: cy, z: cz },
    chunkData: chunk.data,
    solidCount: chunk.solidCount,
    solidBounds: cloneLocalBounds(resolveChunkSolidBounds(world, chunk, cx, cy, cz)),
    neighbors: [
      [
        toOpaqueChunkNeighbor(neighbors[0]![0], 0, true, world.chunkSize),
        toOpaqueChunkNeighbor(neighbors[0]![1], 0, false, world.chunkSize),
      ],
      [
        toOpaqueChunkNeighbor(neighbors[1]![0], 1, true, world.chunkSize),
        toOpaqueChunkNeighbor(neighbors[1]![1], 1, false, world.chunkSize),
      ],
      [
        toOpaqueChunkNeighbor(neighbors[2]![0], 2, true, world.chunkSize),
        toOpaqueChunkNeighbor(neighbors[2]![1], 2, false, world.chunkSize),
      ],
    ],
  };
  return options.cloneData ? cloneOpaqueChunkMeshingInput(input) : input;
}

export function buildChunkMeshFromOpaqueGeometry(
  world: ResidentChunkWorld,
  cx: number,
  cy: number,
  cz: number,
  opaqueMesh: OpaqueChunkMeshGeometry,
): ChunkMeshData {
  const chunkSize = world.chunkSize;
  const fallbackBounds = {
    min: [cx * chunkSize, cy * chunkSize, cz * chunkSize] as [number, number, number],
    max: [(cx + 1) * chunkSize, (cy + 1) * chunkSize, (cz + 1) * chunkSize] as [number, number, number],
  };
  const waterMesh = buildWaterOnlyChunkMesh(world, cx, cy, cz);
  return {
    vertexData: opaqueMesh.vertexData,
    vertexCount: opaqueMesh.vertexCount,
    indexData: opaqueMesh.indexData,
    indexCount: opaqueMesh.indexCount,
    waterVertexData: waterMesh.vertexData,
    waterVertexCount: waterMesh.vertexCount,
    waterIndexData: waterMesh.indexData,
    waterIndexCount: waterMesh.indexCount,
    waterTriangleCount: waterMesh.triangleCount,
    triangleCount: opaqueMesh.triangleCount + waterMesh.triangleCount,
    bounds: combineMeshBounds(opaqueMesh.bounds, waterMesh.bounds, fallbackBounds),
  };
}

export function buildChunkMesh(world: ResidentChunkWorld, cx: number, cy: number, cz: number): ChunkMeshData {
  const chunkSize = world.chunkSize;
  const originX = cx * chunkSize;
  const originY = cy * chunkSize;
  const originZ = cz * chunkSize;
  const chunk = world.getResidentChunk(cx, cy, cz);
  if (!chunk) {
    return createEmptyChunkMesh({
      min: [originX, originY, originZ],
      max: [originX + chunkSize, originY + chunkSize, originZ + chunkSize],
    });
  }

  const neighbors = resolveChunkNeighbors(world, cx, cy, cz);
  const chunkArea = chunkSize * chunkSize;
  const chunkVolume = chunkSize * chunkArea;
  if (chunk.solidCount === chunkVolume && isChunkFullyOccluded(neighbors, chunkSize, chunkArea)) {
    return createEmptyChunkMesh({
      min: [originX, originY, originZ],
      max: [originX + chunkSize, originY + chunkSize, originZ + chunkSize],
    });
  }
  const solidBounds = resolveChunkSolidBounds(world, chunk, cx, cy, cz);
  if (!solidBounds) {
    return createEmptyChunkMesh({
      min: [originX, originY, originZ],
      max: [originX + chunkSize, originY + chunkSize, originZ + chunkSize],
    });
  }

  const chunkData = chunk.data;
  const maxMaskLength = Math.max(
    (solidBounds.max[1] - solidBounds.min[1]) * (solidBounds.max[2] - solidBounds.min[2]),
    (solidBounds.max[0] - solidBounds.min[0]) * (solidBounds.max[2] - solidBounds.min[2]),
    (solidBounds.max[0] - solidBounds.min[0]) * (solidBounds.max[1] - solidBounds.min[1]),
  );
  const scratch = acquireMesherScratch(maxMaskLength);
  const quads = scratch.quads;

  for (let axis = 0; axis < 3; axis += 1) {
    const u = (axis + 1) % 3;
    const v = (axis + 2) % 3;
    const uSpan = solidBounds.max[u] - solidBounds.min[u];
    const vSpan = solidBounds.max[v] - solidBounds.min[v];
    const mask = scratch.mask;
    const x = [...solidBounds.min];
    const q = [0, 0, 0];
    q[axis] = 1;

    for (x[axis] = solidBounds.min[axis] - 1; x[axis] < solidBounds.max[axis]; x[axis] += 1) {
      let maskIndex = 0;
      const negativeNeighbor = neighbors[axis]![0];
      const positiveNeighbor = neighbors[axis]![1];
      for (x[v] = solidBounds.min[v]; x[v] < solidBounds.max[v]; x[v] += 1) {
        for (x[u] = solidBounds.min[u]; x[u] < solidBounds.max[u]; x[u] += 1) {
          const a = x[axis] >= 0
            ? sampleChunkVoxel(chunkData, chunkSize, chunkArea, x[0]!, x[1]!, x[2]!)
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
            ? sampleChunkVoxel(chunkData, chunkSize, chunkArea, x[0] + q[0]!, x[1] + q[1]!, x[2] + q[2]!)
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
          const opaqueA = isOpaqueMaterial(world, a) ? a : 0;
          const opaqueB = isOpaqueMaterial(world, b) ? b : 0;
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

  buildWaterSurfaceQuads(world, chunkData, neighbors, chunkSize, chunkArea, solidBounds, originX, originY, originZ, scratch);
  const opaqueMesh = buildMeshGeometryFromQuads(quads, world);
  const waterMesh = buildWaterMeshGeometryFromQuads(scratch.waterQuads, world);
  const mesh = {
    vertexData: opaqueMesh.vertexData,
    vertexCount: opaqueMesh.vertexCount,
    indexData: opaqueMesh.indexData,
    indexCount: opaqueMesh.indexCount,
    waterVertexData: waterMesh.vertexData,
    waterVertexCount: waterMesh.vertexCount,
    waterIndexData: waterMesh.indexData,
    waterIndexCount: waterMesh.indexCount,
    waterTriangleCount: waterMesh.triangleCount,
    triangleCount: opaqueMesh.triangleCount + waterMesh.triangleCount,
    bounds: combineMeshBounds(
      opaqueMesh.bounds,
      waterMesh.bounds,
      {
        min: [originX, originY, originZ],
        max: [originX + chunkSize, originY + chunkSize, originZ + chunkSize],
      },
    ),
  };
  releaseMesherScratch(scratch);
  return mesh;
}

function buildWaterSurfaceQuads(
  world: ResidentChunkWorld,
  chunkData: Uint16Array,
  neighbors: ReadonlyArray<[ResolvedChunkNeighbor, ResolvedChunkNeighbor]>,
  chunkSize: number,
  chunkArea: number,
  solidBounds: {
    min: [number, number, number];
    max: [number, number, number];
  },
  originX: number,
  originY: number,
  originZ: number,
  scratch: MesherScratch,
): void {
  const mask = scratch.mask;
  const quads = scratch.waterQuads;
  const width = solidBounds.max[0] - solidBounds.min[0];
  const depth = solidBounds.max[2] - solidBounds.min[2];
  const positiveYNeighbor = neighbors[1]![1];

  for (let y = solidBounds.min[1]; y < solidBounds.max[1]; y += 1) {
    let maskIndex = 0;
    for (let z = solidBounds.min[2]; z < solidBounds.max[2]; z += 1) {
      for (let x = solidBounds.min[0]; x < solidBounds.max[0]; x += 1) {
        const material = sampleChunkVoxel(chunkData, chunkSize, chunkArea, x, y, z);
        if (!world.isWaterMaterial(material)) {
          mask[maskIndex] = 0;
          maskIndex += 1;
          continue;
        }
        const above = y + 1 < chunkSize
          ? sampleChunkVoxel(chunkData, chunkSize, chunkArea, x, y + 1, z)
          : sampleNeighborVoxel(
              positiveYNeighbor.data,
              1,
              false,
              x,
              y + 1,
              z,
              chunkSize,
              chunkArea,
            );
        if (world.isWaterMaterial(above)) {
          mask[maskIndex] = 0;
          maskIndex += 1;
          continue;
        }
        const worldX = originX + x;
        const worldY = originY + y;
        const worldZ = originZ + z;
        const depthBand = quantizeWaterDepthBand(measureWaterDepthWorldUnits(world, worldX, worldY, worldZ));
        mask[maskIndex] = encodeWaterSurfaceKey(material, depthBand);
        maskIndex += 1;
      }
    }

    maskIndex = 0;
    for (let row = 0; row < depth; row += 1) {
      for (let column = 0; column < width; ) {
        const current = mask[maskIndex];
        if (current === 0) {
          column += 1;
          maskIndex += 1;
          continue;
        }
        let quadWidth = 1;
        while (column + quadWidth < width && mask[maskIndex + quadWidth] === current) {
          quadWidth += 1;
        }

        let quadDepth = 1;
        let shouldGrow = true;
        while (row + quadDepth < depth && shouldGrow) {
          for (let offset = 0; offset < quadWidth; offset += 1) {
            if (mask[maskIndex + offset + quadDepth * width] !== current) {
              shouldGrow = false;
              break;
            }
          }
          if (shouldGrow) {
            quadDepth += 1;
          }
        }

        quads.push(
          originX + solidBounds.min[0] + column,
          originY + y + 1,
          originZ + solidBounds.min[2] + row,
          quadWidth,
          0,
          0,
          0,
          0,
          quadDepth,
          1,
          current,
        );

        for (let dz = 0; dz < quadDepth; dz += 1) {
          for (let dx = 0; dx < quadWidth; dx += 1) {
            mask[maskIndex + dx + dz * width] = 0;
          }
        }

        column += quadWidth;
        maskIndex += quadWidth;
      }
    }
  }
}

function packNormal(normalX: number, normalY: number, normalZ: number): number {
  const packedX = (normalX * NORMAL_SCALE) & 0xff;
  const packedY = (normalY * NORMAL_SCALE) & 0xff;
  const packedZ = (normalZ * NORMAL_SCALE) & 0xff;
  return (packedX | (packedY << 8) | (packedZ << 16) | (NORMAL_SCALE << 24)) >>> 0;
}

function buildMeshGeometryFromQuads(
  quads: readonly number[],
  world: ResidentChunkWorld,
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
  const quadCount = quads.length / QUAD_STRIDE;
  const vertexCount = quadCount * 4;
  const indexCount = quadCount * 6;
  const vertexData = new ArrayBuffer(vertexCount * VERTEX_STRIDE);
  const vertexFloatView = new Float32Array(vertexData);
  const vertexUintView = new Uint32Array(vertexData);
  const indexData = new Uint32Array(indexCount);

  let vertexWordOffset = 0;
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
    const packedNormal = packNormal(normalX, normalY, normalZ);

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

    vertexFloatView[vertexWordOffset] = corner0X;
    vertexFloatView[vertexWordOffset + 1] = corner0Y;
    vertexFloatView[vertexWordOffset + 2] = corner0Z;
    vertexUintView[vertexWordOffset + 3] = packedNormal;
    vertexUintView[vertexWordOffset + 4] = color;
    vertexWordOffset += 5;
    vertexFloatView[vertexWordOffset] = corner1X;
    vertexFloatView[vertexWordOffset + 1] = corner1Y;
    vertexFloatView[vertexWordOffset + 2] = corner1Z;
    vertexUintView[vertexWordOffset + 3] = packedNormal;
    vertexUintView[vertexWordOffset + 4] = color;
    vertexWordOffset += 5;
    vertexFloatView[vertexWordOffset] = corner2X;
    vertexFloatView[vertexWordOffset + 1] = corner2Y;
    vertexFloatView[vertexWordOffset + 2] = corner2Z;
    vertexUintView[vertexWordOffset + 3] = packedNormal;
    vertexUintView[vertexWordOffset + 4] = color;
    vertexWordOffset += 5;
    vertexFloatView[vertexWordOffset] = corner3X;
    vertexFloatView[vertexWordOffset + 1] = corner3Y;
    vertexFloatView[vertexWordOffset + 2] = corner3Z;
    vertexUintView[vertexWordOffset + 3] = packedNormal;
    vertexUintView[vertexWordOffset + 4] = color;
    vertexWordOffset += 5;

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

function buildWaterMeshGeometryFromQuads(
  quads: readonly number[],
  world: ResidentChunkWorld,
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
  const quadCount = quads.length / QUAD_STRIDE;
  const vertexCount = quadCount * 4;
  const indexCount = quadCount * 6;
  const vertexData = new ArrayBuffer(vertexCount * VERTEX_STRIDE);
  const vertexFloatView = new Float32Array(vertexData);
  const vertexUintView = new Uint32Array(vertexData);
  const indexData = new Uint32Array(indexCount);
  const waterNormal = packNormal(0, 1, 0);

  let vertexWordOffset = 0;
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
    const packedKey = quads[quadIndex + 10]!;
    const waterMaterial = decodeWaterSurfaceMaterial(packedKey);
    const depthWorldUnits = decodeWaterSurfaceDepthWorldUnits(packedKey);
    const color = applyWaterDepthTint(world.getPaletteColor(waterMaterial), depthWorldUnits);

    const corner0X = positionX;
    const corner0Y = positionY;
    const corner0Z = positionZ;
    const corner1X = positionX + duX;
    const corner1Y = positionY + duY;
    const corner1Z = positionZ + duZ;
    const corner2X = positionX + duX + dvX;
    const corner2Y = positionY + duY + dvY;
    const corner2Z = positionZ + duZ + dvZ;
    const corner3X = positionX + dvX;
    const corner3Y = positionY + dvY;
    const corner3Z = positionZ + dvZ;

    minX = Math.min(minX, corner0X, corner1X, corner2X, corner3X);
    minY = Math.min(minY, corner0Y, corner1Y, corner2Y, corner3Y);
    minZ = Math.min(minZ, corner0Z, corner1Z, corner2Z, corner3Z);
    maxX = Math.max(maxX, corner0X, corner1X, corner2X, corner3X);
    maxY = Math.max(maxY, corner0Y, corner1Y, corner2Y, corner3Y);
    maxZ = Math.max(maxZ, corner0Z, corner1Z, corner2Z, corner3Z);

    vertexFloatView[vertexWordOffset] = corner0X;
    vertexFloatView[vertexWordOffset + 1] = corner0Y;
    vertexFloatView[vertexWordOffset + 2] = corner0Z;
    vertexUintView[vertexWordOffset + 3] = waterNormal;
    vertexUintView[vertexWordOffset + 4] = color;
    vertexWordOffset += 5;
    vertexFloatView[vertexWordOffset] = corner1X;
    vertexFloatView[vertexWordOffset + 1] = corner1Y;
    vertexFloatView[vertexWordOffset + 2] = corner1Z;
    vertexUintView[vertexWordOffset + 3] = waterNormal;
    vertexUintView[vertexWordOffset + 4] = color;
    vertexWordOffset += 5;
    vertexFloatView[vertexWordOffset] = corner2X;
    vertexFloatView[vertexWordOffset + 1] = corner2Y;
    vertexFloatView[vertexWordOffset + 2] = corner2Z;
    vertexUintView[vertexWordOffset + 3] = waterNormal;
    vertexUintView[vertexWordOffset + 4] = color;
    vertexWordOffset += 5;
    vertexFloatView[vertexWordOffset] = corner3X;
    vertexFloatView[vertexWordOffset + 1] = corner3Y;
    vertexFloatView[vertexWordOffset + 2] = corner3Z;
    vertexUintView[vertexWordOffset + 3] = waterNormal;
    vertexUintView[vertexWordOffset + 4] = color;
    vertexWordOffset += 5;

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

function buildWaterOnlyChunkMesh(
  world: ResidentChunkWorld,
  cx: number,
  cy: number,
  cz: number,
): ReturnType<typeof buildWaterMeshGeometryFromQuads> {
  const chunk = world.getResidentChunk(cx, cy, cz);
  if (!chunk) {
    return createEmptyWaterMesh();
  }
  const solidBounds = resolveChunkSolidBounds(world, chunk, cx, cy, cz);
  if (!solidBounds) {
    return createEmptyWaterMesh();
  }
  const chunkSize = world.chunkSize;
  const chunkArea = chunkSize * chunkSize;
  const scratch = acquireMesherScratch(
    (solidBounds.max[0] - solidBounds.min[0]) * (solidBounds.max[2] - solidBounds.min[2]),
  );
  buildWaterSurfaceQuads(
    world,
    chunk.data,
    resolveChunkNeighbors(world, cx, cy, cz),
    chunkSize,
    chunkArea,
    solidBounds,
    cx * chunkSize,
    cy * chunkSize,
    cz * chunkSize,
    scratch,
  );
  const mesh = buildWaterMeshGeometryFromQuads(scratch.waterQuads, world);
  releaseMesherScratch(scratch);
  return mesh;
}

function createEmptyWaterMesh(): ReturnType<typeof buildWaterMeshGeometryFromQuads> {
  return {
    vertexData: new ArrayBuffer(0),
    vertexCount: 0,
    indexData: new Uint32Array(0),
    indexCount: 0,
    triangleCount: 0,
    bounds: null,
  };
}

function combineMeshBounds(
  opaqueBounds: { min: [number, number, number]; max: [number, number, number] } | null,
  waterBounds: { min: [number, number, number]; max: [number, number, number] } | null,
  fallback: { min: [number, number, number]; max: [number, number, number] },
): {
  min: [number, number, number];
  max: [number, number, number];
} {
  if (!opaqueBounds && !waterBounds) {
    return fallback;
  }
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

function isOpaqueMaterial(world: ResidentChunkWorld, material: number): boolean {
  return material !== 0 && !world.isWaterMaterial(material);
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
  neighbors: ReadonlyArray<[ResolvedChunkNeighbor, ResolvedChunkNeighbor]>,
  chunkSize: number,
  chunkArea: number,
): boolean {
  return isNeighborFaceSolid(neighbors[0]![0], chunkSize, chunkArea, "x", chunkSize - 1)
    && isNeighborFaceSolid(neighbors[0]![1], chunkSize, chunkArea, "x", 0)
    && isNeighborFaceSolid(neighbors[1]![0], chunkSize, chunkArea, "y", chunkSize - 1)
    && isNeighborFaceSolid(neighbors[1]![1], chunkSize, chunkArea, "y", 0)
    && isNeighborFaceSolid(neighbors[2]![0], chunkSize, chunkArea, "z", chunkSize - 1)
    && isNeighborFaceSolid(neighbors[2]![1], chunkSize, chunkArea, "z", 0);
}

function isNeighborFaceSolid(
  neighbor: ResolvedChunkNeighbor,
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

function resolveChunkNeighbors(
  world: ResidentChunkWorld,
  cx: number,
  cy: number,
  cz: number,
): Array<[ResolvedChunkNeighbor, ResolvedChunkNeighbor]> {
  return [
    [
      resolveNeighbor(world, cx - 1, cy, cz),
      resolveNeighbor(world, cx + 1, cy, cz),
    ],
    [
      resolveNeighbor(world, cx, cy - 1, cz),
      resolveNeighbor(world, cx, cy + 1, cz),
    ],
    [
      resolveNeighbor(world, cx, cy, cz - 1),
      resolveNeighbor(world, cx, cy, cz + 1),
    ],
  ];
}

function resolveNeighbor(
  world: ResidentChunkWorld,
  cx: number,
  cy: number,
  cz: number,
): ResolvedChunkNeighbor {
  const chunk = world.getResidentChunk(cx, cy, cz);
  if (!chunk) {
    return {
      data: null,
      solidBounds: null,
    };
  }
  return {
    data: chunk.data,
    solidBounds: resolveChunkSolidBounds(world, chunk, cx, cy, cz),
  };
}

function toOpaqueChunkNeighbor(
  neighbor: ResolvedChunkNeighbor,
  axis: number,
  negative: boolean,
  chunkSize: number,
) {
  return {
    faceData: extractNeighborFaceData(neighbor.data, axis, negative, chunkSize),
    solidBounds: cloneLocalBounds(neighbor.solidBounds),
  };
}

function extractNeighborFaceData(
  data: Uint16Array | null,
  axis: number,
  negative: boolean,
  chunkSize: number,
): Uint16Array | null {
  if (!data) {
    return null;
  }
  const chunkArea = chunkSize * chunkSize;
  const faceData = new Uint16Array(chunkArea);
  if (axis === 0) {
    const localX = negative ? chunkSize - 1 : 0;
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
    const localY = negative ? chunkSize - 1 : 0;
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
  const localZ = negative ? chunkSize - 1 : 0;
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

function cloneLocalBounds(
  bounds: { min: [number, number, number]; max: [number, number, number] } | null,
): { min: [number, number, number]; max: [number, number, number] } | null {
  if (!bounds) {
    return null;
  }
  return {
    min: [...bounds.min],
    max: [...bounds.max],
  };
}

function resolveChunkSolidBounds(
  world: ResidentChunkWorld,
  chunk: { solidCount: number; solidBounds: { min: [number, number, number]; max: [number, number, number]; dirty: boolean } | null },
  cx: number,
  cy: number,
  cz: number,
): {
  min: [number, number, number];
  max: [number, number, number];
} | null {
  if (chunk.solidCount === 0 || !chunk.solidBounds) {
    return null;
  }
  if (!chunk.solidBounds.dirty) {
    return chunk.solidBounds;
  }
  return world.getChunkSolidBounds(cx, cy, cz);
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

function measureWaterDepthWorldUnits(
  world: ResidentChunkWorld,
  worldX: number,
  worldY: number,
  worldZ: number,
): number {
  let depth = 0;
  let sampleY = worldY;
  while (sampleY >= world.minY) {
    const material = world.getVoxel(worldX, sampleY, worldZ);
    if (!world.isWaterMaterial(material)) {
      break;
    }
    depth += 1;
    sampleY -= 1;
  }
  return depth;
}

function quantizeWaterDepthBand(depthWorldUnits: number): number {
  if (depthWorldUnits <= 0) {
    return 0;
  }
  return Math.min(WATER_DEPTH_BAND_MASK, Math.floor((depthWorldUnits - 1) / WATER_DEPTH_BAND_WORLD_UNITS));
}

function encodeWaterSurfaceKey(material: number, depthBand: number): number {
  return material | (depthBand << WATER_DEPTH_BAND_SHIFT);
}

function decodeWaterSurfaceMaterial(key: number): number {
  return key & 0xffff;
}

function decodeWaterSurfaceDepthWorldUnits(key: number): number {
  const depthBand = (key >>> WATER_DEPTH_BAND_SHIFT) & WATER_DEPTH_BAND_MASK;
  return depthBand * WATER_DEPTH_BAND_WORLD_UNITS + 1;
}



function acquireMesherScratch(requiredMaskLength: number): MesherScratch {
  const scratch = mesherScratchPool.pop() ?? {
    quads: [],
    waterQuads: [],
    mask: new Int32Array(requiredMaskLength),
  };
  scratch.quads.length = 0;
  scratch.waterQuads.length = 0;
  if (scratch.mask.length < requiredMaskLength) {
    scratch.mask = new Int32Array(requiredMaskLength);
  }
  return scratch;
}

function releaseMesherScratch(scratch: MesherScratch): void {
  scratch.quads.length = 0;
  scratch.waterQuads.length = 0;
  if (mesherScratchPool.length >= MAX_MESHER_SCRATCH_POOL) {
    return;
  }
  mesherScratchPool.push(scratch);
}
