import { rebuildDirtyMeshes } from "../src/engine/mesher.ts";
import { ProceduralResidentWorld } from "../src/engine/procedural-resident-world.ts";
import { ProceduralWorldGenerator } from "../src/engine/procedural-generator.ts";

const args = Bun.argv.slice(2);
const iterations = readPositiveInt(readFlag(args, "--iterations"), 3);
const warmupRuns = readPositiveInt(readFlag(args, "--warmup"), 1);
const seed = readPositiveInt(readFlag(args, "--seed"), 1337);
const nearRadius = readPositiveInt(readFlag(args, "--near-radius"), 2);
const farRadius = readPositiveInt(readFlag(args, "--far-radius"), 3);

const scenarios = [
  {
    id: `bootstrap-r${farRadius}`,
    run: () => {
      const world = new ProceduralResidentWorld(new ProceduralWorldGenerator(seed), {
        horizontalRadiusChunks: farRadius,
      });
      const spawn = world.getSpawnPosition();
      const residency = world.updateResidencyAround(spawn);
      const mesh = rebuildDirtyMeshes(world);
      return {
        streamMs: residency.elapsedMs,
        meshMs: mesh.elapsedMs,
        generatedChunks: residency.generatedChunks,
        evictedChunks: residency.evictedChunks,
        residentChunks: world.getStats().chunkCount,
        solidVoxelCount: world.getStats().solidVoxelCount,
      };
    },
  },
  {
    id: `widen-r${nearRadius}-to-r${farRadius}`,
    run: () => {
      const world = new ProceduralResidentWorld(new ProceduralWorldGenerator(seed), {
        horizontalRadiusChunks: nearRadius,
      });
      const spawn = world.getSpawnPosition();
      world.updateResidencyAround(spawn);
      rebuildDirtyMeshes(world);
      world.setHorizontalRadiusChunks(farRadius);
      const residency = world.updateResidencyAround(spawn);
      const mesh = rebuildDirtyMeshes(world);
      return {
        streamMs: residency.elapsedMs,
        meshMs: mesh.elapsedMs,
        generatedChunks: residency.generatedChunks,
        evictedChunks: residency.evictedChunks,
        residentChunks: world.getStats().chunkCount,
        solidVoxelCount: world.getStats().solidVoxelCount,
      };
    },
  },
  {
    id: `shrink-r${farRadius}-to-r${nearRadius}`,
    run: () => {
      const world = new ProceduralResidentWorld(new ProceduralWorldGenerator(seed), {
        horizontalRadiusChunks: farRadius,
      });
      const spawn = world.getSpawnPosition();
      world.updateResidencyAround(spawn);
      rebuildDirtyMeshes(world);
      world.setHorizontalRadiusChunks(nearRadius);
      const residency = world.updateResidencyAround(spawn);
      const mesh = rebuildDirtyMeshes(world);
      return {
        streamMs: residency.elapsedMs,
        meshMs: mesh.elapsedMs,
        generatedChunks: residency.generatedChunks,
        evictedChunks: residency.evictedChunks,
        residentChunks: world.getStats().chunkCount,
        solidVoxelCount: world.getStats().solidVoxelCount,
      };
    },
  },
];

for (const scenario of scenarios) {
  for (let run = 0; run < warmupRuns; run += 1) {
    scenario.run();
  }

  const streamSamples: number[] = [];
  const meshSamples: number[] = [];
  const generatedSamples: number[] = [];
  const evictedSamples: number[] = [];
  let residentChunks = 0;
  let solidVoxelCount = 0;

  for (let run = 0; run < iterations; run += 1) {
    const result = scenario.run();
    streamSamples.push(result.streamMs);
    meshSamples.push(result.meshMs);
    generatedSamples.push(result.generatedChunks);
    evictedSamples.push(result.evictedChunks);
    residentChunks = result.residentChunks;
    solidVoxelCount = result.solidVoxelCount;
  }

  console.log(JSON.stringify({
    scenario: scenario.id,
    iterations,
    warmupRuns,
    seed,
    nearRadius,
    farRadius,
    stream: summarize(streamSamples),
    mesh: summarize(meshSamples),
    generatedChunks: summarize(generatedSamples),
    evictedChunks: summarize(evictedSamples),
    residentChunks,
    solidVoxelCount,
  }));
}

function summarize(values: number[]) {
  return {
    avgMs: average(values),
    minMs: Math.min(...values),
    maxMs: Math.max(...values),
    samples: values,
  };
}

function average(values: number[]): number {
  let total = 0;
  for (const value of values) {
    total += value;
  }
  return total / values.length;
}

function readFlag(args: string[], flag: string): string | null {
  const exact = args.find((arg) => arg.startsWith(`${flag}=`));
  if (exact) {
    return exact.slice(flag.length + 1);
  }
  const index = args.indexOf(flag);
  if (index === -1) {
    return null;
  }
  return args[index + 1] ?? null;
}

function readPositiveInt(raw: string | null, fallback: number): number {
  const parsed = Number.parseInt(raw ?? "", 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}
