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
const farBandBudget = readPositiveInt(readFlag(args, "--far-band-budget"), 1);
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
  const farTotals: number[] = [];
  const farSampleCacheTotals: number[] = [];
  const farMeshBuildTotals: number[] = [];
  const farSampledCellTotals: number[] = [];
  const maxFarBandBuild: Array<{ elapsedMs: number; label: string | null }> = [];
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
    farSampleCacheTotals.push(result.totalFarFieldSampleCacheMs);
    farMeshBuildTotals.push(result.totalFarFieldMeshBuildMs);
    farSampledCellTotals.push(result.totalFarFieldSampledCellCount);
    maxFarBandBuild.push(result.maxFarFieldBandBuild);
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
    farBandBudget,
    anchorMarginChunks,
    anchorChanged,
    frames: summarize(frameCounts),
    totalStreamMs: summarize(streamTotals),
    totalMeshMs: summarize(meshTotals),
    totalFarFieldMs: summarize(farTotals),
    totalFarFieldSampleCacheMs: summarize(farSampleCacheTotals),
    totalFarFieldMeshBuildMs: summarize(farMeshBuildTotals),
    totalFarFieldSampledCellCount: summarize(farSampledCellTotals),
    maxFarFieldBandBuildMs: summarize(maxFarBandBuild.map((entry) => entry.elapsedMs)),
    maxFarFieldBandLabel: maxFarBandBuild.reduce<{ label: string | null; elapsedMs: number }>(
      (current, entry) => entry.elapsedMs > current.elapsedMs ? entry : current,
      { label: null, elapsedMs: 0 },
    ).label,
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
  totalFarFieldSampleCacheMs: number;
  totalFarFieldMeshBuildMs: number;
  totalFarFieldSampledCellCount: number;
  totalGeneratedChunks: number;
  totalRemeshChunks: number;
  totalYRangeMs: number;
  totalChunkGenerationMs: number;
  totalEmptyChunksSkipped: number;
  maxPendingChunks: number;
  maxDirtyResidentChunks: number;
  maxFrameWorkMs: number;
  maxFarFieldBandBuild: {
    elapsedMs: number;
    label: string | null;
  };
  residentChunks: number;
  farTriangles: number;
} {
  const generator = new ProceduralWorldGenerator(seed);
  const world = new ProceduralResidentWorld(generator, { horizontalRadiusChunks: radiusChunks });
  const farField = new ProceduralFarField(world);
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
  let totalFarFieldSampleCacheMs = 0;
  let totalFarFieldMeshBuildMs = 0;
  let totalFarFieldSampledCellCount = 0;
  let totalGeneratedChunks = 0;
  let totalRemeshChunks = 0;
  let totalYRangeMs = 0;
  let totalChunkGenerationMs = 0;
  let totalEmptyChunksSkipped = 0;
  let maxPendingChunks = 0;
  let maxDirtyResidentChunks = 0;
  let maxFrameWorkMs = 0;
  let maxFarFieldBandBuild = {
    elapsedMs: 0,
    label: null as string | null,
  };
  let requestedFarFieldMaskRevision = 0;
  let presentedFarFieldMaskRevision = 0;

  if (!resolved.changed) {
    const farSummary = farField.updateAround(
      targetFeetPosition,
      0,
      world.getFarFieldExclusionMask("render-ready", presentedFarFieldMaskRevision),
      farBandBudget,
    );
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
      totalFarFieldSampleCacheMs: farSummary.sampleCacheMs,
      totalFarFieldMeshBuildMs: farSummary.meshBuildMs,
      totalFarFieldSampledCellCount: farSummary.sampledCellCount,
      totalGeneratedChunks: 0,
      totalRemeshChunks: 0,
      totalYRangeMs: 0,
      totalChunkGenerationMs: 0,
      totalEmptyChunksSkipped: 0,
      maxPendingChunks: 0,
      maxDirtyResidentChunks: dirtyResidentChunks,
      maxFrameWorkMs: farSummary.elapsedMs,
      maxFarFieldBandBuild: farSummary.bandBuilds.reduce(
        (current, band) => band.elapsedMs > current.elapsedMs
          ? { elapsedMs: band.elapsedMs, label: band.label }
          : current,
        { elapsedMs: 0, label: null as string | null },
      ),
      residentChunks: world.getStats().chunkCount,
      farTriangles: farField.lastUpdate.triangleCount,
    };
  }

  let step = 0;
  let pendingChunks = 0;
  let pendingFarFieldBands = 0;
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
    if (residency.generatedChunks > 0 || residency.evictedChunks > 0 || residency.touchedNeighborChunks > 0) {
      requestedFarFieldMaskRevision += 1;
    }
    if (mesh.meshCount > 0) {
      requestedFarFieldMaskRevision += 1;
    }
    dirtyResidentChunks = world.countDirtyResidentChunks();
    if (residency.pendingChunks === 0 && dirtyResidentChunks === 0) {
      presentedFarFieldMaskRevision = requestedFarFieldMaskRevision;
    }
    const farSummary = farField.updateAround(
      targetFeetPosition,
      0,
      world.getFarFieldExclusionMask("render-ready", presentedFarFieldMaskRevision),
      farBandBudget,
    );
    pendingChunks = residency.pendingChunks;
    pendingFarFieldBands = farSummary.pendingBands;
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
    totalFarFieldSampleCacheMs += farSummary.sampleCacheMs;
    totalFarFieldMeshBuildMs += farSummary.meshBuildMs;
    totalFarFieldSampledCellCount += farSummary.sampledCellCount;
    totalGeneratedChunks += residency.generatedChunks;
    totalRemeshChunks += mesh.remeshCount;
    totalYRangeMs += residency.phaseMs.yRangeMs;
    totalChunkGenerationMs += residency.phaseMs.chunkGenerationMs;
    totalEmptyChunksSkipped += residency.emptyChunksSkipped;
    maxPendingChunks = Math.max(maxPendingChunks, pendingChunks);
    maxDirtyResidentChunks = Math.max(maxDirtyResidentChunks, dirtyResidentChunks);
    maxFrameWorkMs = Math.max(maxFrameWorkMs, frameWorkMs);
    for (const bandBuild of farSummary.bandBuilds) {
      if (bandBuild.elapsedMs > maxFarFieldBandBuild.elapsedMs) {
        maxFarFieldBandBuild = {
          elapsedMs: bandBuild.elapsedMs,
          label: bandBuild.label,
        };
      }
    }
  } while (shouldPumpWorldWork(false, pendingChunks, dirtyResidentChunks, pendingFarFieldBands));

  return {
    anchorChanged: true,
    frames,
    totalStreamMs,
    totalMeshMs,
    totalFarFieldMs,
    totalFarFieldSampleCacheMs,
    totalFarFieldMeshBuildMs,
    totalFarFieldSampledCellCount,
    totalGeneratedChunks,
    totalRemeshChunks,
    totalYRangeMs,
    totalChunkGenerationMs,
    totalEmptyChunksSkipped,
    maxPendingChunks,
    maxDirtyResidentChunks,
    maxFrameWorkMs,
    maxFarFieldBandBuild,
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
  let farFieldMaskRevision = 0;
  world.updateResidencyAround(
    buildStreamAnchorPosition(anchor, world.chunkSize, feetPosition[1]),
    { maxGenerateChunks: Number.POSITIVE_INFINITY },
  );
  world.prefetchFarFieldSummariesAround(feetPosition, farField.getMaxRadiusWorldUnits(), Number.POSITIVE_INFINITY);
  farFieldMaskRevision += 1;
  rebuildDirtyMeshes(world, Number.POSITIVE_INFINITY, {
    priorityPosition: feetPosition,
  });
  farFieldMaskRevision += 1;
  farField.updateAround(
    feetPosition,
    0,
    world.getFarFieldExclusionMask("render-ready", farFieldMaskRevision),
    Number.POSITIVE_INFINITY,
  );
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
