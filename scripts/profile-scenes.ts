import { getSceneDefinitions } from "../src/engine/scenes.ts";
import { rebuildDirtyMeshes } from "../src/engine/mesher.ts";

const args = Bun.argv.slice(2);
const iterations = readPositiveInt(readFlag(args, "--iterations"), 5);
const warmupRuns = readPositiveInt(readFlag(args, "--warmup"), 1);
const sceneIds = args.filter((arg) => !arg.startsWith("--"));
const sceneDefinitions = getSceneDefinitions();
const scenes = sceneIds.length === 0
  ? sceneDefinitions
  : sceneIds.map((id) => {
      const scene = sceneDefinitions.find((candidate) => candidate.id === id);
      if (!scene) {
        throw new Error(`Unknown scene "${id}"`);
      }
      return scene;
    });

for (const scene of scenes) {
  for (let run = 0; run < warmupRuns; run += 1) {
    const build = scene.build();
    rebuildDirtyMeshes(build.world);
  }

  const buildSamples: number[] = [];
  const meshSamples: number[] = [];
  let triangles = 0;
  let chunks = 0;
  let solidVoxels = 0;

  for (let run = 0; run < iterations; run += 1) {
    const buildStartedAt = performance.now();
    const build = scene.build();
    buildSamples.push(performance.now() - buildStartedAt);

    const meshSummary = rebuildDirtyMeshes(build.world);
    meshSamples.push(meshSummary.elapsedMs);
    triangles = meshSummary.triangleCount;
    chunks = build.world.chunks.size;
    solidVoxels = build.world.getStats().solidVoxelCount;
  }

  console.log(JSON.stringify({
    scene: scene.id,
    iterations,
    warmupRuns,
    build: summarize(buildSamples),
    mesh: summarize(meshSamples),
    triangles,
    chunks,
    solidVoxels,
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
