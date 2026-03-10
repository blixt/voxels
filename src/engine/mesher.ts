import type { ChunkMeshData } from "./types.ts";
import { VoxelWorld } from "./world.ts";

const FLOAT32_BYTES = 4;
const VERTEX_STRIDE = 20;
const NORMAL_SCALE = 127;

interface FaceCell {
  material: number;
  normal: 1 | -1;
}

interface Quad {
  position: [number, number, number];
  du: [number, number, number];
  dv: [number, number, number];
  axis: number;
  normal: 1 | -1;
  material: number;
}

export interface MeshBuildSummary {
  meshCount: number;
  triangleCount: number;
  elapsedMs: number;
}

export function rebuildDirtyMeshes(world: VoxelWorld): MeshBuildSummary {
  const startedAt = performance.now();
  let meshCount = 0;
  let triangleCount = 0;
  for (const chunk of world.chunks.values()) {
    if (!chunk.meshDirty) {
      continue;
    }
    chunk.mesh = buildChunkMesh(world, chunk.coord.x, chunk.coord.y, chunk.coord.z);
    chunk.meshDirty = false;
    chunk.gpuDirty = true;
    meshCount += 1;
    triangleCount += chunk.mesh?.triangleCount ?? 0;
  }
  return {
    meshCount,
    triangleCount,
    elapsedMs: performance.now() - startedAt,
  };
}

export function buildChunkMesh(world: VoxelWorld, cx: number, cy: number, cz: number): ChunkMeshData {
  const chunkSize = world.chunkSize;
  const originX = cx * chunkSize;
  const originY = cy * chunkSize;
  const originZ = cz * chunkSize;
  const quads: Quad[] = [];
  const mask = new Array<FaceCell | null>(chunkSize * chunkSize);

  for (let axis = 0; axis < 3; axis += 1) {
    const u = (axis + 1) % 3;
    const v = (axis + 2) % 3;
    const x = [0, 0, 0];
    const q = [0, 0, 0];
    q[axis] = 1;

    for (x[axis] = -1; x[axis] < chunkSize; x[axis] += 1) {
      let maskIndex = 0;
      for (x[v] = 0; x[v] < chunkSize; x[v] += 1) {
        for (x[u] = 0; x[u] < chunkSize; x[u] += 1) {
          const ax = originX + x[0];
          const ay = originY + x[1];
          const az = originZ + x[2];
          const bx = ax + q[0]!;
          const by = ay + q[1]!;
          const bz = az + q[2]!;
          const a = x[axis] >= 0 ? world.getVoxel(ax, ay, az) : 0;
          const b = x[axis] < chunkSize - 1 ? world.getVoxel(bx, by, bz) : 0;
          if ((a !== 0) === (b !== 0)) {
            mask[maskIndex] = null;
          } else if (a !== 0) {
            mask[maskIndex] = { material: a, normal: 1 };
          } else {
            mask[maskIndex] = { material: b, normal: -1 };
          }
          maskIndex += 1;
        }
      }

      maskIndex = 0;
      for (let row = 0; row < chunkSize; row += 1) {
        for (let column = 0; column < chunkSize; ) {
          const current = mask[maskIndex];
          if (!current) {
            column += 1;
            maskIndex += 1;
            continue;
          }

          let width = 1;
          while (
            column + width < chunkSize &&
            isSameFace(mask[maskIndex + width], current)
          ) {
            width += 1;
          }

          let height = 1;
          let shouldGrow = true;
          while (row + height < chunkSize && shouldGrow) {
            for (let offset = 0; offset < width; offset += 1) {
              if (!isSameFace(mask[maskIndex + offset + height * chunkSize], current)) {
                shouldGrow = false;
                break;
              }
            }
            if (shouldGrow) {
              height += 1;
            }
          }

          const position: [number, number, number] = [originX, originY, originZ];
          position[axis] += x[axis] + 1;
          position[u] += column;
          position[v] += row;

          const du: [number, number, number] = [0, 0, 0];
          const dv: [number, number, number] = [0, 0, 0];
          du[u] = width;
          dv[v] = height;

          quads.push({
            position,
            du,
            dv,
            axis,
            normal: current.normal,
            material: current.material,
          });

          for (let dy = 0; dy < height; dy += 1) {
            for (let dx = 0; dx < width; dx += 1) {
              mask[maskIndex + dx + dy * chunkSize] = null;
            }
          }

          column += width;
          maskIndex += width;
        }
      }
    }
  }

  if (quads.length === 0) {
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

  const vertexCount = quads.length * 4;
  const indexCount = quads.length * 6;
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

  for (const quad of quads) {
    const color = world.getPaletteColor(quad.material);
    const normal = axisNormal(quad.axis, quad.normal);
    const corners = quad.normal === 1
      ? [
          quad.position,
          addPoint(quad.position, quad.du),
          addPoint(addPoint(quad.position, quad.du), quad.dv),
          addPoint(quad.position, quad.dv),
        ]
      : [
          quad.position,
          addPoint(quad.position, quad.dv),
          addPoint(addPoint(quad.position, quad.dv), quad.du),
          addPoint(quad.position, quad.du),
        ];

    for (const corner of corners) {
      minX = Math.min(minX, corner[0]);
      minY = Math.min(minY, corner[1]);
      minZ = Math.min(minZ, corner[2]);
      maxX = Math.max(maxX, corner[0]);
      maxY = Math.max(maxY, corner[1]);
      maxZ = Math.max(maxZ, corner[2]);
      writeVertex(vertexView, vertexOffset, corner, normal, color);
      vertexOffset += VERTEX_STRIDE;
    }

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
    triangleCount: quads.length * 2,
    bounds: {
      min: [minX, minY, minZ],
      max: [maxX, maxY, maxZ],
    },
  };
}

function isSameFace(a: FaceCell | null | undefined, b: FaceCell): boolean {
  return !!a && a.material === b.material && a.normal === b.normal;
}

function addPoint(
  a: [number, number, number],
  b: [number, number, number],
): [number, number, number] {
  return [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
}

function axisNormal(axis: number, direction: 1 | -1): [number, number, number] {
  const out: [number, number, number] = [0, 0, 0];
  out[axis] = direction;
  return out;
}

function writeVertex(
  view: DataView,
  byteOffset: number,
  position: [number, number, number],
  normal: [number, number, number],
  color: number,
): void {
  view.setFloat32(byteOffset + 0 * FLOAT32_BYTES, position[0], true);
  view.setFloat32(byteOffset + 1 * FLOAT32_BYTES, position[1], true);
  view.setFloat32(byteOffset + 2 * FLOAT32_BYTES, position[2], true);
  view.setInt8(byteOffset + 12, normal[0] * NORMAL_SCALE);
  view.setInt8(byteOffset + 13, normal[1] * NORMAL_SCALE);
  view.setInt8(byteOffset + 14, normal[2] * NORMAL_SCALE);
  view.setInt8(byteOffset + 15, NORMAL_SCALE);
  view.setUint32(byteOffset + 16, color, true);
}
