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
  const chunkKey = world.resolveChunkKey(cx, cy, cz);
  const originX = cx * chunkSize;
  const originY = cy * chunkSize;
  const originZ = cz * chunkSize;
  const chunk = chunkKey === null ? null : world.chunks.get(chunkKey);
  const solidBounds = world.getChunkSolidBounds(cx, cy, cz);
  if (!chunk || !solidBounds) {
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
  const chunkData = chunk.data;
  const quads: Quad[] = [];

  for (let axis = 0; axis < 3; axis += 1) {
    const u = (axis + 1) % 3;
    const v = (axis + 2) % 3;
    const uSpan = solidBounds.max[u] - solidBounds.min[u];
    const vSpan = solidBounds.max[v] - solidBounds.min[v];
    const mask = new Array<FaceCell | null>(uSpan * vSpan);
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
      for (let row = 0; row < vSpan; row += 1) {
        for (let column = 0; column < uSpan; ) {
          const current = mask[maskIndex];
          if (!current) {
            column += 1;
            maskIndex += 1;
            continue;
          }

          let width = 1;
          while (
            column + width < uSpan &&
            isSameFace(mask[maskIndex + width], current)
          ) {
            width += 1;
          }

          let height = 1;
          let shouldGrow = true;
          while (row + height < vSpan && shouldGrow) {
            for (let offset = 0; offset < width; offset += 1) {
              if (!isSameFace(mask[maskIndex + offset + height * uSpan], current)) {
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
          position[u] += solidBounds.min[u] + column;
          position[v] += solidBounds.min[v] + row;

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
              mask[maskIndex + dx + dy * uSpan] = null;
            }
          }

          column += width;
          maskIndex += width;
        }
      }
    }
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
