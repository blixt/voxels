import { rebuildDirtyMeshes } from "../src/engine/mesher.ts";
import { ProceduralFarField } from "../src/engine/procedural-far-field.ts";
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

const scenarios = [
  {
    id: `crossing-d${chunkDelta}`,
    run: () => simulateChunkCrossing(chunkDelta),
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
  const farTotals: number[] = [];
  const maxFrameWork: number[] = [];
  const maxPending: number[] = [];
  const generatedTotals: number[] = [];
  const remeshTotals: number[] = [];
  const uploadableDirtyTotals: number[] = [];
  const yRangeTotals: number[] = [];
  const generationTotals: number[] = [];
  const emptyChunkTotals: number[] = [];
  let residentChunks = 0;
  let farTriangles = 0;
  let anchorChanged = false;

  for (let run = 0; run < iterations; run += 1) {
    const result = scenario.run();
    frameCounts.push(result.frames.length);
    streamTotals.push(result.totalStreamMs);
    meshTotals.push(result.totalMeshMs);
    farTotals.push(result.totalFarFieldMs);
    maxFrameWork.push(result.maxFrameWorkMs);
    maxPending.push(result.maxPendingChunks);
    generatedTotals.push(result.totalGeneratedChunks);
    remeshTotals.push(result.totalRemeshChunks);
    uploadableDirtyTotals.push(result.maxDirtyResidentChunks);
    yRangeTotals.push(result.totalYRangeMs);
    generationTotals.push(result.totalChunkGenerationMs);
    emptyChunkTotals.push(result.totalEmptyChunksSkipped);
    residentChunks = result.residentChunks;
    farTriangles = result.farTriangles;
    anchorChanged = result.anchorChanged;
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
    totalFarFieldMs: summarize(farTotals),
    maxFrameWorkMs: summarize(maxFrameWork),
    maxPendingChunks: summarize(maxPending),
    totalGeneratedChunks: summarize(generatedTotals),
    totalRemeshChunks: summarize(remeshTotals),
    maxDirtyResidentChunks: summarize(uploadableDirtyTotals),
    totalYRangeMs: summarize(yRangeTotals),
    totalChunkGenerationMs: summarize(generationTotals),
    totalEmptyChunksSkipped: summarize(emptyChunkTotals),
    residentChunks,
    farTriangles,
  }));
}

function simulateChunkCrossing(deltaChunks: number): {
  anchorChanged: boolean;
  frames: Array<{
    step: number;
    streamMs: number;
    meshMs: number;
    farFieldMs: number;
    generatedChunks: number;
    pendingChunks: number;
    dirtyResidentChunks: number;
    remeshChunks: number;
    yRangeMs: number;
    chunkGenerationMs: number;
    emptyChunksSkipped: number;
  }>;
  totalStreamMs: number;
  totalMeshMs: number;
  totalFarFieldMs: number;
  totalGeneratedChunks: number;
  totalRemeshChunks: number;
  totalYRangeMs: number;
  totalChunkGenerationMs: number;
  totalEmptyChunksSkipped: number;
  maxPendingChunks: number;
  maxDirtyResidentChunks: number;
  maxFrameWorkMs: number;
  residentChunks: number;
  farTriangles: number;
} {
  const generator = new ProceduralWorldGenerator(seed);
  const world = new ProceduralResidentWorld(generator, { horizontalRadiusChunks: radiusChunks });
  const farField = new ProceduralFarField(generator);
  const spawn = world.getSpawnPosition();
  const initialAnchor = resolveAnchor(spawn, world.chunkSize);

  settleWorld(world, farField, spawn, initialAnchor);

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
    farFieldMs: number;
    generatedChunks: number;
    pendingChunks: number;
    dirtyResidentChunks: number;
    remeshChunks: number;
    yRangeMs: number;
    chunkGenerationMs: number;
    emptyChunksSkipped: number;
  }> = [];

  let totalStreamMs = 0;
  let totalMeshMs = 0;
  let totalFarFieldMs = 0;
  let totalGeneratedChunks = 0;
  let totalRemeshChunks = 0;
  let totalYRangeMs = 0;
  let totalChunkGenerationMs = 0;
  let totalEmptyChunksSkipped = 0;
  let maxPendingChunks = 0;
  let maxDirtyResidentChunks = 0;
  let maxFrameWorkMs = 0;

  if (!resolved.changed) {
    const farSummary = farField.updateAround(targetFeetPosition, getNearClearRadiusWorldUnits(world));
    const dirtyResidentChunks = world.countDirtyResidentChunks();
    return {
      anchorChanged: false,
      frames: [
        {
          step: 1,
          streamMs: 0,
          meshMs: 0,
          farFieldMs: farSummary.elapsedMs,
          generatedChunks: 0,
          pendingChunks: 0,
          dirtyResidentChunks,
          remeshChunks: 0,
          yRangeMs: 0,
          chunkGenerationMs: 0,
          emptyChunksSkipped: 0,
        },
      ],
      totalStreamMs: 0,
      totalMeshMs: 0,
      totalFarFieldMs: farSummary.elapsedMs,
      totalGeneratedChunks: 0,
      totalRemeshChunks: 0,
      totalYRangeMs: 0,
      totalChunkGenerationMs: 0,
      totalEmptyChunksSkipped: 0,
      maxPendingChunks: 0,
      maxDirtyResidentChunks: dirtyResidentChunks,
      maxFrameWorkMs: farSummary.elapsedMs,
      residentChunks: world.getStats().chunkCount,
      farTriangles: farField.lastUpdate.triangleCount,
    };
  }

  let step = 0;
  let pendingChunks = 0;
  let dirtyResidentChunks = world.countDirtyResidentChunks();
  do {
    step += 1;
    const farSummary = farField.updateAround(targetFeetPosition, getNearClearRadiusWorldUnits(world));
    const residency = world.updateResidencyAround(
      buildStreamAnchorPosition(targetAnchor, world.chunkSize, targetFeetPosition[1]),
      { maxGenerateChunks: generateBudget },
    );
    const mesh = rebuildDirtyMeshes(world, meshBudget);
    dirtyResidentChunks = world.countDirtyResidentChunks();
    pendingChunks = residency.pendingChunks;
    const frameWorkMs = farSummary.elapsedMs + residency.elapsedMs + mesh.elapsedMs;
    frames.push({
      step,
      streamMs: residency.elapsedMs,
      meshMs: mesh.elapsedMs,
      farFieldMs: farSummary.elapsedMs,
      generatedChunks: residency.generatedChunks,
      pendingChunks,
      dirtyResidentChunks,
      remeshChunks: mesh.remeshCount,
      yRangeMs: residency.phaseMs.yRangeMs,
      chunkGenerationMs: residency.phaseMs.chunkGenerationMs,
      emptyChunksSkipped: residency.emptyChunksSkipped,
    });
    totalStreamMs += residency.elapsedMs;
    totalMeshMs += mesh.elapsedMs;
    totalFarFieldMs += farSummary.elapsedMs;
    totalGeneratedChunks += residency.generatedChunks;
    totalRemeshChunks += mesh.remeshCount;
    totalYRangeMs += residency.phaseMs.yRangeMs;
    totalChunkGenerationMs += residency.phaseMs.chunkGenerationMs;
    totalEmptyChunksSkipped += residency.emptyChunksSkipped;
    maxPendingChunks = Math.max(maxPendingChunks, pendingChunks);
    maxDirtyResidentChunks = Math.max(maxDirtyResidentChunks, dirtyResidentChunks);
    maxFrameWorkMs = Math.max(maxFrameWorkMs, frameWorkMs);
  } while (shouldPumpWorldWork(false, pendingChunks, dirtyResidentChunks));

  return {
    anchorChanged: true,
    frames,
    totalStreamMs,
    totalMeshMs,
    totalFarFieldMs,
    totalGeneratedChunks,
    totalRemeshChunks,
    totalYRangeMs,
    totalChunkGenerationMs,
    totalEmptyChunksSkipped,
    maxPendingChunks,
    maxDirtyResidentChunks,
    maxFrameWorkMs,
    residentChunks: world.getStats().chunkCount,
    farTriangles: farField.lastUpdate.triangleCount,
  };
}

function settleWorld(
  world: ProceduralResidentWorld,
  farField: ProceduralFarField,
  feetPosition: Vec3,
  anchor: StreamAnchor,
): void {
  farField.updateAround(feetPosition, getNearClearRadiusWorldUnits(world));
  world.updateResidencyAround(
    buildStreamAnchorPosition(anchor, world.chunkSize, feetPosition[1]),
    { maxGenerateChunks: Number.POSITIVE_INFINITY },
  );
  rebuildDirtyMeshes(world, Number.POSITIVE_INFINITY);
}

function resolveAnchor(feetPosition: Vec3, chunkSize: number): StreamAnchor {
  return {
    chunkX: Math.floor(feetPosition[0] / chunkSize),
    chunkZ: Math.floor(feetPosition[2] / chunkSize),
  };
}

function getNearClearRadiusWorldUnits(world: ProceduralResidentWorld): number {
  return (world.horizontalRadiusChunks + 1) * world.chunkSize;
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
