import { rebuildDirtyMeshes } from "../src/engine/mesher.ts";
import { ProceduralResidentWorld } from "../src/engine/procedural-resident-world.ts";
import { ProceduralWorldGenerator } from "../src/engine/procedural-generator.ts";
import { shouldPumpWorldWork } from "../src/engine/stream-work.ts";
import { buildStreamAnchorPosition, resolveStreamAnchor, type StreamAnchor } from "../src/engine/stream-anchor.ts";
import type { Vec3 } from "../src/engine/types.ts";

const args = Bun.argv.slice(2);
const iterations = readPositiveInt(readFlag(args, "--iterations"), 3);
const warmupRuns = readPositiveInt(readFlag(args, "--warmup"), 1);
const seed = readPositiveInt(readFlag(args, "--seed"), 1337);
const radiusChunks = readPositiveInt(readFlag(args, "--radius"), 5);
const generateBudget = readPositiveInt(readFlag(args, "--generate-budget"), 6);
const meshBudget = readPositiveInt(readFlag(args, "--mesh-budget"), 4);
const chunkDelta = readPositiveInt(readFlag(args, "--chunk-delta"), 2);
const anchorMarginChunks = readPositiveInt(readFlag(args, "--anchor-margin"), 1);
const farAnchorDeltaChunks = readPositiveInt(readFlag(args, "--far-anchor-chunk-delta"), 8);

const scenarios = [
  {
    id: `crossing-d${chunkDelta}`,
    run: () => simulateChunkCrossing(chunkDelta),
  },
  {
    id: `crossing-far-anchor-d${farAnchorDeltaChunks}`,
    run: () => simulateChunkCrossing(farAnchorDeltaChunks),
  },
  {
    id: "crossing-d1",
    run: () => simulateChunkCrossing(1),
  },
];

for (const scenario of scenarios) {
  for (let run = 0; run < warmupRuns; run += 1) {
    scenario.run();
  }

  const frameCounts: number[] = [];
  const streamTotals: number[] = [];
  const meshTotals: number[] = [];
  const maxFrameWork: number[] = [];
  const maxPending: number[] = [];
  const generatedTotals: number[] = [];
  const remeshTotals: number[] = [];
  const uploadableDirtyTotals: number[] = [];
  const yRangeTotals: number[] = [];
  const generationTotals: number[] = [];
  const emptyChunkTotals: number[] = [];
  let residentChunks = 0;
  let anchorChanged = false;

  const lodGeneratedTotals: number[] = [];
  const lodDownsampleTotals: number[] = [];
  const lodMeshTotals: number[] = [];
  const lodTotalChunksTotals: number[] = [];
  const lodPendingTotals: number[] = [];

  for (let run = 0; run < iterations; run += 1) {
    const result = scenario.run();
    frameCounts.push(result.frames.length);
    streamTotals.push(result.totalStreamMs);
    meshTotals.push(result.totalMeshMs);
    maxFrameWork.push(result.maxFrameWorkMs);
    maxPending.push(result.maxPendingChunks);
    generatedTotals.push(result.totalGeneratedChunks);
    remeshTotals.push(result.totalRemeshChunks);
    uploadableDirtyTotals.push(result.maxDirtyResidentChunks);
    yRangeTotals.push(result.totalYRangeMs);
    generationTotals.push(result.totalChunkGenerationMs);
    emptyChunkTotals.push(result.totalEmptyChunksSkipped);
    residentChunks = result.residentChunks;
    anchorChanged = result.anchorChanged;
    lodGeneratedTotals.push(result.totalLodGeneratedChunks);
    lodDownsampleTotals.push(result.totalLodDownsampleMs);
    lodMeshTotals.push(result.totalLodMeshMs);
    lodTotalChunksTotals.push(result.lodTotalChunks);
    lodPendingTotals.push(result.maxLodPendingChunks);
  }

  console.log(JSON.stringify({
    scenario: scenario.id,
    iterations,
    warmupRuns,
    seed,
    radiusChunks,
    generateBudget,
    meshBudget,
    anchorMarginChunks,
    anchorChanged,
    frames: summarize(frameCounts),
    totalStreamMs: summarize(streamTotals),
    totalMeshMs: summarize(meshTotals),
    maxFrameWorkMs: summarize(maxFrameWork),
    maxPendingChunks: summarize(maxPending),
    totalGeneratedChunks: summarize(generatedTotals),
    totalRemeshChunks: summarize(remeshTotals),
    maxDirtyResidentChunks: summarize(uploadableDirtyTotals),
    totalYRangeMs: summarize(yRangeTotals),
    totalChunkGenerationMs: summarize(generationTotals),
    totalEmptyChunksSkipped: summarize(emptyChunkTotals),
    residentChunks,
    totalLodGeneratedChunks: summarize(lodGeneratedTotals),
    totalLodDownsampleMs: summarize(lodDownsampleTotals),
    totalLodMeshMs: summarize(lodMeshTotals),
    lodTotalChunks: summarize(lodTotalChunksTotals),
    maxLodPendingChunks: summarize(lodPendingTotals),
  }));
}

function simulateChunkCrossing(deltaChunks: number): {
  anchorChanged: boolean;
  frames: Array<{
    step: number;
    streamMs: number;
    meshMs: number;
    generatedChunks: number;
    pendingChunks: number;
    dirtyResidentChunks: number;
    remeshChunks: number;
    yRangeMs: number;
    chunkGenerationMs: number;
    emptyChunksSkipped: number;
    lodGeneratedChunks: number;
    lodDownsampleMs: number;
    lodMeshMs: number;
    lodTotalChunks: number;
    lodPendingChunks: number;
  }>;
  totalStreamMs: number;
  totalMeshMs: number;
  totalGeneratedChunks: number;
  totalRemeshChunks: number;
  totalYRangeMs: number;
  totalChunkGenerationMs: number;
  totalEmptyChunksSkipped: number;
  totalLodGeneratedChunks: number;
  totalLodDownsampleMs: number;
  totalLodMeshMs: number;
  maxPendingChunks: number;
  maxDirtyResidentChunks: number;
  maxFrameWorkMs: number;
  maxLodPendingChunks: number;
  lodTotalChunks: number;
  residentChunks: number;
} {
  const generator = new ProceduralWorldGenerator(seed);
  const world = new ProceduralResidentWorld(generator, { horizontalRadiusChunks: radiusChunks });
  const spawn = world.getSpawnPosition();
  const initialAnchor = resolveAnchor(spawn, world.chunkSize);

  settleWorld(world, spawn, initialAnchor);
  world.updateLodResidencyAround(spawn, { maxGenerateLodChunks: Number.POSITIVE_INFINITY });

  const targetFeetPosition: Vec3 = [
    spawn[0] + deltaChunks * world.chunkSize,
    spawn[1],
    spawn[2],
  ];
  const targetChunkX = Math.floor(targetFeetPosition[0] / world.chunkSize);
  const targetChunkZ = Math.floor(targetFeetPosition[2] / world.chunkSize);
  const resolved = resolveStreamAnchor(initialAnchor, targetChunkX, targetChunkZ, anchorMarginChunks);
  const targetAnchor = resolved.anchor;

  const frames: Array<{
    step: number;
    streamMs: number;
    meshMs: number;
    generatedChunks: number;
    pendingChunks: number;
    dirtyResidentChunks: number;
    remeshChunks: number;
    yRangeMs: number;
    chunkGenerationMs: number;
    emptyChunksSkipped: number;
    lodGeneratedChunks: number;
    lodDownsampleMs: number;
    lodMeshMs: number;
    lodTotalChunks: number;
    lodPendingChunks: number;
  }> = [];

  let totalStreamMs = 0;
  let totalMeshMs = 0;
  let totalGeneratedChunks = 0;
  let totalRemeshChunks = 0;
  let totalYRangeMs = 0;
  let totalChunkGenerationMs = 0;
  let totalEmptyChunksSkipped = 0;
  let totalLodGeneratedChunks = 0;
  let totalLodDownsampleMs = 0;
  let totalLodMeshMs = 0;
  let maxPendingChunks = 0;
  let maxDirtyResidentChunks = 0;
  let maxFrameWorkMs = 0;
  let maxLodPendingChunks = 0;
  let lodTotalChunks = 0;

  if (!resolved.changed) {
    const dirtyResidentChunks = world.countDirtyResidentChunks();
    return {
      anchorChanged: false,
      frames: [
        {
          step: 1,
          streamMs: 0,
          meshMs: 0,
          generatedChunks: 0,
          pendingChunks: 0,
          dirtyResidentChunks,
          remeshChunks: 0,
          yRangeMs: 0,
          chunkGenerationMs: 0,
          emptyChunksSkipped: 0,
          lodGeneratedChunks: 0,
          lodDownsampleMs: 0,
          lodMeshMs: 0,
          lodTotalChunks: 0,
          lodPendingChunks: 0,
        },
      ],
      totalStreamMs: 0,
      totalMeshMs: 0,
      totalGeneratedChunks: 0,
      totalRemeshChunks: 0,
      totalYRangeMs: 0,
      totalChunkGenerationMs: 0,
      totalEmptyChunksSkipped: 0,
      totalLodGeneratedChunks: 0,
      totalLodDownsampleMs: 0,
      totalLodMeshMs: 0,
      maxPendingChunks: 0,
      maxDirtyResidentChunks: dirtyResidentChunks,
      maxFrameWorkMs: 0,
      maxLodPendingChunks: 0,
      lodTotalChunks: 0,
      residentChunks: world.getStats().chunkCount,
    };
  }

  let step = 0;
  let pendingChunks = 0;
  let dirtyResidentChunks = world.countDirtyResidentChunks();
  do {
    step += 1;
    const residency = world.updateResidencyAround(
      buildStreamAnchorPosition(targetAnchor, world.chunkSize, targetFeetPosition[1]),
      { maxGenerateChunks: generateBudget },
    );
    const mesh = rebuildDirtyMeshes(world, meshBudget, {
      priorityPosition: targetFeetPosition,
    });
    const lodResult = world.updateLodResidencyAround(targetFeetPosition);
    dirtyResidentChunks = world.countDirtyResidentChunks();
    pendingChunks = residency.pendingChunks;
    const frameWorkMs = residency.elapsedMs + mesh.elapsedMs + lodResult.elapsedMs;
    frames.push({
      step,
      streamMs: residency.elapsedMs,
      meshMs: mesh.elapsedMs,
      generatedChunks: residency.generatedChunks,
      pendingChunks,
      dirtyResidentChunks,
      remeshChunks: mesh.remeshCount,
      yRangeMs: residency.phaseMs.yRangeMs,
      chunkGenerationMs: residency.phaseMs.chunkGenerationMs,
      emptyChunksSkipped: residency.emptyChunksSkipped,
      lodGeneratedChunks: lodResult.generated,
      lodDownsampleMs: lodResult.downsampleMs,
      lodMeshMs: lodResult.meshMs,
      lodTotalChunks: lodResult.totalChunks,
      lodPendingChunks: lodResult.pending,
    });
    totalStreamMs += residency.elapsedMs;
    totalMeshMs += mesh.elapsedMs;
    totalGeneratedChunks += residency.generatedChunks;
    totalRemeshChunks += mesh.remeshCount;
    totalYRangeMs += residency.phaseMs.yRangeMs;
    totalChunkGenerationMs += residency.phaseMs.chunkGenerationMs;
    totalEmptyChunksSkipped += residency.emptyChunksSkipped;
    totalLodGeneratedChunks += lodResult.generated;
    totalLodDownsampleMs += lodResult.downsampleMs;
    totalLodMeshMs += lodResult.meshMs;
    maxPendingChunks = Math.max(maxPendingChunks, pendingChunks);
    maxDirtyResidentChunks = Math.max(maxDirtyResidentChunks, dirtyResidentChunks);
    maxFrameWorkMs = Math.max(maxFrameWorkMs, frameWorkMs);
    maxLodPendingChunks = Math.max(maxLodPendingChunks, lodResult.pending);
    lodTotalChunks = lodResult.totalChunks;
  } while (shouldPumpWorldWork(false, pendingChunks, dirtyResidentChunks));

  return {
    anchorChanged: true,
    frames,
    totalStreamMs,
    totalMeshMs,
    totalGeneratedChunks,
    totalRemeshChunks,
    totalYRangeMs,
    totalChunkGenerationMs,
    totalEmptyChunksSkipped,
    totalLodGeneratedChunks,
    totalLodDownsampleMs,
    totalLodMeshMs,
    maxPendingChunks,
    maxDirtyResidentChunks,
    maxFrameWorkMs,
    maxLodPendingChunks,
    lodTotalChunks,
    residentChunks: world.getStats().chunkCount,
  };
}

function settleWorld(
  world: ProceduralResidentWorld,
  feetPosition: Vec3,
  anchor: StreamAnchor,
): void {
  world.updateResidencyAround(
    buildStreamAnchorPosition(anchor, world.chunkSize, feetPosition[1]),
    { maxGenerateChunks: Number.POSITIVE_INFINITY },
  );
  rebuildDirtyMeshes(world, Number.POSITIVE_INFINITY, {
    priorityPosition: feetPosition,
  });
}

function resolveAnchor(feetPosition: Vec3, chunkSize: number): StreamAnchor {
  return {
    chunkX: Math.floor(feetPosition[0] / chunkSize),
    chunkZ: Math.floor(feetPosition[2] / chunkSize),
  };
}

function summarize(values: number[]) {
  return {
    avg: average(values),
    min: Math.min(...values),
    max: Math.max(...values),
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
