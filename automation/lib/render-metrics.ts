import {
  type EngineClient,
  FRAME_SAMPLE_WIDTH,
  gpuSampleStart,
  GPU_SAMPLE_WIDTH,
  SNAPSHOT,
  snapshotValue,
} from "./engine.ts";
import { percentile } from "./metrics.ts";

const FRAME_SAMPLE_START = SNAPSHOT.droppedSamples + 1;

export interface GpuFrameSample {
  readonly frameId: number;
  readonly total: number;
  readonly shadow: number;
  readonly shadowCascade0: number;
  readonly shadowCascade1: number;
  readonly shadowCascade2: number;
  readonly depthPrepass: number;
  readonly ambientOcclusion: number;
  readonly world: number;
  readonly water: number;
  readonly cloud: number;
  readonly weather: number;
  readonly ui: number;
}

export interface RenderSnapshotCapture {
  readonly snapshot: readonly number[];
  readonly samples: readonly (readonly number[])[];
  readonly dropped: number;
  readonly gpuSamples: readonly GpuFrameSample[];
  readonly gpuDropped: number;
}

export interface DistributionSummary {
  readonly median: number;
  readonly p95: number;
  readonly p99: number;
  readonly max: number;
}

function required(values: readonly number[], index: number, label: string): number {
  const value = values[index];
  if (value === undefined) throw new Error(`${label} omitted value ${index}`);
  return value;
}

function summary(values: readonly number[]): DistributionSummary {
  return {
    median: percentile(values, 0.5),
    p95: percentile(values, 0.95),
    p99: percentile(values, 0.99),
    max: Math.max(...values, 0),
  };
}

export async function captureRenderSnapshot(engine: EngineClient): Promise<RenderSnapshotCapture> {
  const snapshot = await engine.snapshot();
  const samples: number[][] = [];
  const count = snapshotValue(snapshot, "sampleCount");
  for (let index = 0; index < count; index += 1) {
    const start = FRAME_SAMPLE_START + index * FRAME_SAMPLE_WIDTH;
    samples.push(snapshot.slice(start, start + FRAME_SAMPLE_WIDTH));
  }
  const gpuStart = gpuSampleStart(snapshot);
  const gpuCount = required(snapshot, gpuStart, "GPU sample header");
  const gpuDropped = required(snapshot, gpuStart + 1, "GPU sample header");
  const gpuSamples: GpuFrameSample[] = [];
  for (let index = 0; index < gpuCount; index += 1) {
    const start = gpuStart + 2 + index * GPU_SAMPLE_WIDTH;
    const values = snapshot.slice(start, start + GPU_SAMPLE_WIDTH);
    gpuSamples.push({
      frameId: required(values, 0, "GPU sample"),
      total: required(values, 1, "GPU sample"),
      shadow: required(values, 2, "GPU sample"),
      shadowCascade0: required(values, 3, "GPU sample"),
      shadowCascade1: required(values, 4, "GPU sample"),
      shadowCascade2: required(values, 5, "GPU sample"),
      depthPrepass: required(values, 6, "GPU sample"),
      ambientOcclusion: required(values, 7, "GPU sample"),
      world: required(values, 8, "GPU sample"),
      water: required(values, 9, "GPU sample"),
      cloud: required(values, 10, "GPU sample"),
      weather: required(values, 11, "GPU sample"),
      ui: required(values, 12, "GPU sample"),
    });
  }
  return {
    snapshot,
    samples,
    dropped: snapshotValue(snapshot, "droppedSamples"),
    gpuSamples,
    gpuDropped,
  };
}

export async function sampleRenderSnapshots(
  engine: EngineClient,
  durationMs: number,
  intervalMs = 250,
): Promise<RenderSnapshotCapture[]> {
  const captures: RenderSnapshotCapture[] = [];
  const deadline = Date.now() + durationMs;
  while (Date.now() < deadline) {
    captures.push(await captureRenderSnapshot(engine));
    if (Date.now() < deadline) await engine.wait(intervalMs);
  }
  return captures;
}

export function summarizeRenderPhase(captures: readonly RenderSnapshotCapture[]) {
  const latest = captures.at(-1)?.snapshot;
  if (latest === undefined) throw new Error("render phase did not capture any samples");
  const samples = captures.flatMap((capture) => capture.samples);
  const gpuSamples = [
    ...new Map(
      captures.flatMap((capture) => capture.gpuSamples).map((sample) => [sample.frameId, sample]),
    ).values(),
  ];
  const column = (index: number): number[] =>
    samples.map((sample) => required(sample, index, "frame sample"));
  const frameIntervals = column(0);
  const frameIds = new Set(samples.map((sample) => required(sample, 5, "frame sample")));
  const coveredGpuSamples = gpuSamples.filter((sample) => frameIds.has(sample.frameId));
  const unattributedCpu = samples.map((sample) =>
    Math.max(
      0,
      required(sample, 1, "frame sample") -
        required(sample, 2, "frame sample") -
        required(sample, 3, "frame sample") -
        required(sample, 4, "frame sample"),
    ),
  );
  const gpuColumn = (key: keyof Omit<GpuFrameSample, "frameId">): number[] =>
    gpuSamples.map((sample) => sample[key]);
  const gpuSummary = (key: keyof Omit<GpuFrameSample, "frameId">) =>
    gpuSamples.length > 0 ? summary(gpuColumn(key)) : null;
  return {
    samples: samples.length,
    droppedSamples: captures.reduce((total, capture) => total + capture.dropped, 0),
    frameMs: {
      ...summary(frameIntervals),
      above16_67ms: frameIntervals.filter((value) => value > 16.67).length,
      above33_33ms: frameIntervals.filter((value) => value > 33.33).length,
    },
    cpuMs: summary(column(1)),
    simulationMs: summary(column(2)),
    streamingMs: summary(column(3)),
    renderSubmissionMs: summary(column(4)),
    unattributedCpuMs: summary(unattributedCpu),
    renderCpu: {
      cullMs: summary(column(6)),
      encodeMs: summary(column(7)),
      submitMs: summary(column(8)),
      testedSlices: summary(column(9)),
      selectedSlices: summary(column(10)),
    },
    gpu: {
      available: gpuSamples.length > 0,
      samples: gpuSamples.length,
      droppedSamples: captures.reduce((total, capture) => total + capture.gpuDropped, 0),
      frameCoverage: frameIds.size > 0 ? coveredGpuSamples.length / frameIds.size : 0,
      totalMs: gpuSummary("total"),
      shadowMs: gpuSummary("shadow"),
      shadowCascadeMs:
        gpuSamples.length > 0
          ? [
              summary(gpuColumn("shadowCascade0")),
              summary(gpuColumn("shadowCascade1")),
              summary(gpuColumn("shadowCascade2")),
            ]
          : null,
      worldMs: gpuSummary("world"),
      waterMs: gpuSummary("water"),
      depthPrepassMs: gpuSummary("depthPrepass"),
      ambientOcclusionMs: gpuSummary("ambientOcclusion"),
      cloudMs: gpuSummary("cloud"),
      weatherMs: gpuSummary("weather"),
      uiMs: gpuSummary("ui"),
    },
    residentChunks: snapshotValue(latest, "residentChunks"),
    surfaceTiles: snapshotValue(latest, "surfaceTiles"),
    horizonTiles: {
      stride32: snapshotValue(latest, "stride32Tiles"),
      stride64: snapshotValue(latest, "stride64Tiles"),
    },
    interactiveLodsReady: snapshotValue(latest, "interactiveLodsReady") === 1,
    allLodsReady: snapshotValue(latest, "allLodsReady") === 1,
    visibleChunks: snapshotValue(latest, "visibleChunks"),
    pendingJobs: snapshotValue(latest, "pendingJobs"),
    quads: snapshotValue(latest, "quads"),
    waterQuads: snapshotValue(latest, "waterQuads"),
    waterDrawCalls: snapshotValue(latest, "waterDrawCalls"),
    drawCalls: snapshotValue(latest, "drawCalls"),
    shadowDrawCalls: snapshotValue(latest, "shadowDrawCalls"),
    shadowCascades: snapshotValue(latest, "shadowCascades"),
    framebuffer: {
      width: snapshotValue(latest, "surfaceWidth"),
      height: snapshotValue(latest, "surfaceHeight"),
      devicePixelRatio: snapshotValue(latest, "devicePixelRatio"),
      pixels: snapshotValue(latest, "surfaceWidth") * snapshotValue(latest, "surfaceHeight"),
    },
    coreGpuMiB: snapshotValue(latest, "coreGpuMiB"),
    meshArenaAllocatedMiB: snapshotValue(latest, "arenaAllocatedMiB"),
    meshArenaCapacityMiB: snapshotValue(latest, "arenaCapacityMiB"),
    refractionCopyMiB: snapshotValue(latest, "refractionCopyMiB"),
    memory: {
      wasmCommittedMiB: snapshotValue(latest, "wasmCommittedMiB"),
      canonicalVoxelMiB: snapshotValue(latest, "canonicalVoxelMiB"),
      pendingMeshMiB: snapshotValue(latest, "pendingMeshMiB"),
      editLogicalMiB: snapshotValue(latest, "editLogicalMiB"),
    },
    totalEvictions: snapshotValue(latest, "totalEvictions"),
    staleCompletions: snapshotValue(latest, "staleCompletions"),
    materialDetail: snapshotValue(latest, "materialDetail") === 1,
    screenSpaceAmbientOcclusion: snapshotValue(latest, "screenSpaceAmbientOcclusion") === 1,
    ambientOcclusionMiB: snapshotValue(latest, "ambientOcclusionMiB"),
    depthPrepassDrawCalls: snapshotValue(latest, "depthPrepassDrawCalls"),
    atmosphere: {
      daylightPhase: snapshotValue(latest, "daylightPhase"),
      dayFraction: snapshotValue(latest, "dayFraction"),
      localSolarDayFraction: snapshotValue(latest, "localSolarDayFraction"),
      yearFraction: snapshotValue(latest, "yearFraction"),
      moonOrbitFraction: snapshotValue(latest, "moonOrbitFraction"),
      twinklePhase: snapshotValue(latest, "twinklePhase"),
      latitudeDegrees: snapshotValue(latest, "latitudeDegrees"),
      longitudeDegrees: snapshotValue(latest, "longitudeDegrees"),
      localSiderealAngleRadians: snapshotValue(latest, "localSiderealAngleRadians"),
      moonIlluminatedFraction: snapshotValue(latest, "moonIlluminatedFraction"),
      celestialRevision: snapshotValue(latest, "celestialRevision"),
      sunDirection: [
        snapshotValue(latest, "sunDirectionX"),
        snapshotValue(latest, "sunDirectionY"),
        snapshotValue(latest, "sunDirectionZ"),
      ] as const,
      moonDirection: [
        snapshotValue(latest, "moonDirectionX"),
        snapshotValue(latest, "moonDirectionY"),
        snapshotValue(latest, "moonDirectionZ"),
      ] as const,
      shadowStrength: snapshotValue(latest, "shadowStrength"),
      surfaceRegion: snapshotValue(latest, "surfaceRegion"),
      cloudCoverage: snapshotValue(latest, "cloudCoverage"),
      cloudOffset: [
        snapshotValue(latest, "cloudOffsetX"),
        snapshotValue(latest, "cloudOffsetZ"),
      ] as const,
      cloudVelocity: [
        snapshotValue(latest, "cloudVelocityX"),
        snapshotValue(latest, "cloudVelocityZ"),
      ] as const,
      weatherRevision: snapshotValue(latest, "weatherRevision"),
      weatherKind: snapshotValue(latest, "weatherKind"),
      weatherFraction: snapshotValue(latest, "weatherFraction"),
      precipitation: snapshotValue(latest, "precipitation"),
      storminess: snapshotValue(latest, "storminess"),
      lightning: snapshotValue(latest, "lightning"),
      cloudDensity: snapshotValue(latest, "cloudDensity"),
      cloudLayer: [
        snapshotValue(latest, "cloudBaseMetres"),
        snapshotValue(latest, "cloudTopMetres"),
      ] as const,
      cloudRenderResolution: [
        snapshotValue(latest, "cloudRenderWidth"),
        snapshotValue(latest, "cloudRenderHeight"),
      ] as const,
      cloudSteps: [
        snapshotValue(latest, "cloudViewSteps"),
        snapshotValue(latest, "cloudLightSteps"),
      ] as const,
      fogDensity: snapshotValue(latest, "fogDensity"),
      exposure: snapshotValue(latest, "outdoorExposure"),
    },
    cave: {
      enclosure: snapshotValue(latest, "enclosure"),
      exposure: snapshotValue(latest, "interiorExposure"),
      headlamp: snapshotValue(latest, "caveHeadlamp") === 1,
      probeUs: snapshotValue(latest, "enclosureProbeUs"),
    },
    localLights: {
      candidates: snapshotValue(latest, "localLightCandidates"),
      active: snapshotValue(latest, "activeLocalLights"),
      clipped: snapshotValue(latest, "clippedLocalLights"),
      occluded: snapshotValue(latest, "occludedLocalLights"),
      portalRejected: snapshotValue(latest, "portalRejectedLocalLights"),
      visibilityTests: snapshotValue(latest, "localLightVisibilityTests"),
      enabled: snapshotValue(latest, "localLighting") === 1,
    },
    cinderPortals: {
      open: snapshotValue(latest, "openCinderPortals"),
      revision: snapshotValue(latest, "cinderPortalRevision"),
    },
    portalStreaming: {
      requested: snapshotValue(latest, "streamInterestRequested"),
      normalized: snapshotValue(latest, "streamInterestNormalized"),
      desired: snapshotValue(latest, "streamInterestDesired"),
      truncated: snapshotValue(latest, "streamInterestTruncated"),
      planOverflow: snapshotValue(latest, "streamPlanOverflow") === 1,
      activeChunks: snapshotValue(latest, "portalActiveChunks"),
      activeColumns: snapshotValue(latest, "portalActiveColumns"),
      unreachableActive: snapshotValue(latest, "unreachablePortalActive"),
    },
    placementMaterial: snapshotValue(latest, "placementMaterial"),
    loadLatencyFrames: {
      p95: snapshotValue(latest, "loadP95Frames"),
      max: snapshotValue(latest, "loadMaxFrames"),
    },
    remeshLatencyFrames: {
      p95: snapshotValue(latest, "remeshP95Frames"),
      max: snapshotValue(latest, "remeshMaxFrames"),
    },
  };
}

export type RenderPhaseSummary = ReturnType<typeof summarizeRenderPhase>;
