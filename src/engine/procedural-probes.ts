import { fnv1a } from "./math.ts";
import type {
  GeneratedChunk,
  ProceduralColumnSample,
  ProceduralWorldGenerator,
} from "./procedural-generator.ts";
import type { ChunkBounds, ChunkCoordinate, WorldStats } from "./types.ts";
import type { VoxelChunk } from "./world.ts";

export interface ChunkProbeSummary {
  coord: ChunkCoordinate;
  solidCount: number;
  checksum: string;
  solidBounds: ChunkBounds | null;
}

export interface GeneratedChunkProbeSummary extends ChunkProbeSummary {
  centerColumn: ProceduralColumnSample;
}

export interface ResidentWorldProbeSnapshot {
  chunkSize: number;
  solidVoxelCount: number;
  chunkCount: number;
  chunks: ChunkProbeSummary[];
}

export interface ResidentWorldProbeSource {
  chunkSize: number;
  getStats(): WorldStats;
  iterateResidentChunks(): Iterable<VoxelChunk>;
}

export function summarizeGeneratedChunk(
  generator: ProceduralWorldGenerator,
  chunk: GeneratedChunk,
): GeneratedChunkProbeSummary {
  const centerWorldX = chunk.coord.x * generator.chunkSize + (generator.chunkSize >> 1);
  const centerWorldZ = chunk.coord.z * generator.chunkSize + (generator.chunkSize >> 1);
  return {
    ...summarizeChunkData(chunk.coord, chunk.data, generator.chunkSize, chunk.solidCount),
    centerColumn: generator.sampleColumn(centerWorldX, centerWorldZ),
  };
}

export function summarizeResidentWorld(source: ResidentWorldProbeSource): ResidentWorldProbeSnapshot {
  const chunks = [...source.iterateResidentChunks()]
    .map((chunk) => summarizeChunkData(chunk.coord, chunk.data, source.chunkSize, chunk.solidCount))
    .sort(compareChunkProbeSummary);
  const stats = source.getStats();
  return {
    chunkSize: source.chunkSize,
    solidVoxelCount: stats.solidVoxelCount,
    chunkCount: stats.chunkCount,
    chunks,
  };
}

export function diffChunkCoords(
  before: readonly ChunkCoordinate[],
  after: readonly ChunkCoordinate[],
): {
  entered: ChunkCoordinate[];
  evicted: ChunkCoordinate[];
} {
  const beforeKeys = new Set(before.map(chunkCoordKey));
  const afterKeys = new Set(after.map(chunkCoordKey));
  const entered = after.filter((coord) => !beforeKeys.has(chunkCoordKey(coord))).sort(compareChunkCoordinate);
  const evicted = before.filter((coord) => !afterKeys.has(chunkCoordKey(coord))).sort(compareChunkCoordinate);
  return { entered, evicted };
}

export function chunkCoordKey(coord: ChunkCoordinate): string {
  return `${coord.x}:${coord.y}:${coord.z}`;
}

function summarizeChunkData(
  coord: ChunkCoordinate,
  data: Uint16Array,
  chunkSize: number,
  solidCount: number,
): ChunkProbeSummary {
  return {
    coord: { ...coord },
    solidCount,
    checksum: fnv1a(new Uint8Array(data.buffer, data.byteOffset, data.byteLength)),
    solidBounds: computeWorldSpaceSolidBounds(coord, data, chunkSize),
  };
}

function computeWorldSpaceSolidBounds(
  coord: ChunkCoordinate,
  data: Uint16Array,
  chunkSize: number,
): ChunkBounds | null {
  if (data.length === 0) {
    return null;
  }
  const chunkArea = chunkSize * chunkSize;
  let minX = chunkSize;
  let minY = chunkSize;
  let minZ = chunkSize;
  let maxX = 0;
  let maxY = 0;
  let maxZ = 0;
  let hasSolid = false;
  for (let index = 0; index < data.length; index += 1) {
    if (data[index] === 0) {
      continue;
    }
    hasSolid = true;
    const x = index % chunkSize;
    const y = Math.floor(index / chunkSize) % chunkSize;
    const z = Math.floor(index / chunkArea);
    minX = Math.min(minX, x);
    minY = Math.min(minY, y);
    minZ = Math.min(minZ, z);
    maxX = Math.max(maxX, x + 1);
    maxY = Math.max(maxY, y + 1);
    maxZ = Math.max(maxZ, z + 1);
  }
  if (!hasSolid) {
    return null;
  }
  const worldOriginX = coord.x * chunkSize;
  const worldOriginY = coord.y * chunkSize;
  const worldOriginZ = coord.z * chunkSize;
  return {
    min: [worldOriginX + minX, worldOriginY + minY, worldOriginZ + minZ],
    max: [worldOriginX + maxX, worldOriginY + maxY, worldOriginZ + maxZ],
  };
}

function compareChunkCoordinate(a: ChunkCoordinate, b: ChunkCoordinate): number {
  return a.x - b.x || a.y - b.y || a.z - b.z;
}

function compareChunkProbeSummary(a: ChunkProbeSummary, b: ChunkProbeSummary): number {
  return compareChunkCoordinate(a.coord, b.coord);
}
