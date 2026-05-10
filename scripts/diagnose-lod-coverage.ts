import { ProceduralWorldGenerator } from "../src/engine/procedural-generator.ts";
import { ProceduralResidentWorld } from "../src/engine/procedural-resident-world.ts";
import type { VoxelChunk } from "../src/engine/world.ts";

const SEED = 1337;
const CHUNK_SIZE = 32;

function parseNumberArg(name: string, fallback: number): number {
  const prefix = `--${name}=`;
  const arg = Bun.argv.find((value) => value.startsWith(prefix));
  if (!arg) return fallback;
  const parsed = Number(arg.slice(prefix.length));
  return Number.isFinite(parsed) ? parsed : fallback;
}

function isColumnCoveredByChunk(chunk: VoxelChunk, worldX: number, worldZ: number): boolean {
  const stride = Math.max(1, chunk.voxelStride);
  const worldSize = CHUNK_SIZE * stride;
  const localX = Math.floor((worldX - chunk.coord.x * worldSize) / stride);
  const localZ = Math.floor((worldZ - chunk.coord.z * worldSize) / stride);
  if (localX < 0 || localX >= CHUNK_SIZE || localZ < 0 || localZ >= CHUNK_SIZE) {
    return false;
  }
  const chunkArea = CHUNK_SIZE * CHUNK_SIZE;
  for (let localY = 0; localY < CHUNK_SIZE; localY += 1) {
    if (chunk.data[localX + localY * CHUNK_SIZE + localZ * chunkArea] !== 0) {
      return true;
    }
  }
  return false;
}

function overlappingLodChunks(chunks: VoxelChunk[], worldX: number, worldZ: number): string[] {
  const result: string[] = [];
  for (const chunk of chunks) {
    const stride = Math.max(1, chunk.voxelStride);
    const worldSize = CHUNK_SIZE * stride;
    const minX = chunk.coord.x * worldSize;
    const minZ = chunk.coord.z * worldSize;
    if (worldX < minX || worldX >= minX + worldSize || worldZ < minZ || worldZ >= minZ + worldSize) {
      continue;
    }
    const covered = isColumnCoveredByChunk(chunk, worldX, worldZ);
    result.push(
      `L${chunk.lodLevel}:${chunk.coord.x}:${chunk.coord.y}:${chunk.coord.z}` +
        ` stride=${stride} solid=${chunk.solidCount} covered=${covered}`,
    );
  }
  return result;
}

const radiusWorldUnits = parseNumberArg("radius", 4160);
const stepWorldUnits = parseNumberArg("step", 80);
const sampleLimit = parseNumberArg("samples", 12);

const generator = new ProceduralWorldGenerator(SEED);
const world = new ProceduralResidentWorld(generator, { horizontalRadiusChunks: 8 });
const spawnPos = world.getSpawnPosition() as [number, number, number];

const residency = world.updateResidencyAround(spawnPos, { maxGenerateChunks: Number.POSITIVE_INFINITY });
const lod = world.updateLodResidencyAround(spawnPos, { maxGenerateLodChunks: Number.POSITIVE_INFINITY });

const lodChunks: VoxelChunk[] = [];
for (const chunk of world.iterateResidentChunks()) {
  if (chunk.lodLevel > 0 && chunk.renderReady && chunk.solidCount > 0) {
    lodChunks.push(chunk);
  }
}

let uncovered = 0;
let total = 0;
const samples: Array<Record<string, unknown>> = [];
for (let dz = -radiusWorldUnits; dz <= radiusWorldUnits; dz += stepWorldUnits) {
  for (let dx = -radiusWorldUnits; dx <= radiusWorldUnits; dx += stepWorldUnits) {
    const worldX = spawnPos[0] + dx;
    const worldZ = spawnPos[2] + dz;
    const chunkX = Math.floor(worldX / CHUNK_SIZE);
    const chunkZ = Math.floor(worldZ / CHUNK_SIZE);
    const covered = world.isColumnRenderReady(chunkX, chunkZ)
      || lodChunks.some((chunk) => isColumnCoveredByChunk(chunk, worldX, worldZ));
    if (!covered) {
      uncovered += 1;
      if (samples.length < sampleLimit) {
        const surface = generator.sampleSurfaceColumn(Math.floor(worldX), Math.floor(worldZ));
        samples.push({
          worldX,
          worldZ,
          chunkX,
          chunkZ,
          surfaceY: surface.surfaceY,
          topY: surface.topY,
          waterTopY: surface.waterTopY,
          overlapping: overlappingLodChunks(lodChunks, worldX, worldZ),
        });
      }
    }
    total += 1;
  }
}

console.log(JSON.stringify({
  spawnPos,
  residency: {
    complete: residency.complete,
    generatedChunks: residency.generatedChunks,
    residentChunks: residency.residentChunks,
    pendingChunks: residency.pendingChunks,
    elapsedMs: residency.elapsedMs,
    phaseMs: residency.phaseMs,
  },
  lod,
  lodChunkCount: lodChunks.length,
  total,
  uncovered,
  samples,
}, null, 2));
