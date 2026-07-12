import { createServer as createNetServer } from "node:net";

export const SNAPSHOT_SCHEMA_VERSION = 15;

export const SNAPSHOT = Object.freeze({
  pitch: 4,
  quads: 6,
  edits: 7,
  residentChunks: 8,
  trackedChunks: 9,
  visibleChunks: 10,
  drawCalls: 11,
  arenaAllocatedMiB: 13,
  arenaCapacityMiB: 14,
  pendingJobs: 15,
  frameMs: 17,
  loadP95Frames: 20,
  loadMaxFrames: 21,
  remeshP95Frames: 22,
  remeshMaxFrames: 23,
  waterQuads: 28,
  refractionCopyMiB: 30,
  immersion: 31,
  eyesSubmerged: 33,
  swimming: 34,
  targetPresent: 38,
  coreGpuMiB: 39,
  cpuMs: 40,
  simulationMs: 41,
  streamMs: 42,
  renderMs: 43,
  gpuSampleId: 44,
  gpuTotalMs: 45,
  gpuShadowMs: 46,
  gpuWorldMs: 47,
  gpuWaterMs: 48,
  gpuUiMs: 49,
  wasmCommittedMiB: 50,
  canonicalVoxelMiB: 51,
  pendingMeshMiB: 52,
  editLogicalMiB: 53,
  totalEvictions: 54,
  staleCompletions: 55,
  profilePhase: 56,
  profileElapsedSeconds: 57,
  profileDistanceMetres: 58,
  profileComplete: 59,
  profileTrackedHigh: 60,
  profileSurfaceHigh: 61,
  profilePendingHigh: 62,
  profilePendingMeshHigh: 63,
  profileArenaCapacityHighMiB: 64,
  profileWasmHighMiB: 65,
  profileEvictions: 66,
  materialDetail: 67,
  daylightPhase: 68,
  surfaceRegion: 69,
  cloudCoverage: 70,
  screenSpaceAmbientOcclusion: 71,
  gpuDepthPrepassMs: 72,
  gpuAmbientOcclusionMs: 73,
  ambientOcclusionMiB: 74,
  depthPrepassDrawCalls: 75,
  enclosure: 76,
  interiorExposure: 77,
  caveHeadlamp: 78,
  enclosureProbeUs: 79,
  localLightCandidates: 80,
  activeLocalLights: 81,
  clippedLocalLights: 82,
  occludedLocalLights: 83,
  portalRejectedLocalLights: 84,
  localLightVisibilityTests: 85,
  openCinderPortals: 86,
  cinderPortalRevision: 87,
  localLighting: 88,
  placementMaterial: 89,
  streamInterestRequested: 90,
  streamInterestNormalized: 91,
  streamInterestDesired: 92,
  streamInterestTruncated: 93,
  streamPlanOverflow: 94,
  portalActiveChunks: 95,
  portalActiveColumns: 96,
  unreachablePortalActive: 97,
  schemaVersion: 98,
  sampleCount: 99,
  droppedSamples: 100,
});

export function assertSnapshotSchema(snapshot) {
  const actual = snapshot[SNAPSHOT.schemaVersion];
  if (actual !== SNAPSHOT_SCHEMA_VERSION) {
    throw new Error(`snapshot schema ${actual} does not match ${SNAPSHOT_SCHEMA_VERSION}`);
  }
  return snapshot;
}

export async function reserveEphemeralPort() {
  const probe = createNetServer();
  await new Promise((resolve, reject) => {
    probe.once("error", reject);
    probe.listen(0, "127.0.0.1", resolve);
  });
  const address = probe.address();
  if (!address || typeof address === "string") throw new Error("could not reserve a TCP port");
  await new Promise((resolve, reject) =>
    probe.close((error) => (error ? reject(error) : resolve())),
  );
  return address.port;
}

export function chromeWebGpuLaunchOptions() {
  return {
    channel: "chrome",
    headless: false,
    args: [
      "--headless=new",
      "--enable-unsafe-webgpu",
      "--enable-features=WebGPU",
      "--no-sandbox",
      "--hide-scrollbars",
    ],
  };
}
