import type { BrowserPlayerSession } from "./local-player.ts";

export const AUTOMATION_CONTRACT_VERSION = 2;
export const SNAPSHOT_SCHEMA_VERSION = 31;
export const FRAME_SAMPLE_WIDTH = 11;
export const GPU_SAMPLE_WIDTH = 13;

// This map is the compact Rust snapshot ABI. Scenario code imports it from the typed engine
// capability; it must not maintain private numeric indices.
export const SNAPSHOT = Object.freeze({
  cameraX: 0,
  cameraY: 1,
  cameraZ: 2,
  yaw: 3,
  pitch: 4,
  grounded: 5,
  quads: 6,
  edits: 7,
  residentChunks: 8,
  trackedChunks: 9,
  visibleChunks: 10,
  drawCalls: 11,
  arenaPages: 12,
  arenaAllocatedMiB: 13,
  arenaCapacityMiB: 14,
  pendingJobs: 15,
  surfaceTiles: 16,
  frameMs: 17,
  shadowDrawCalls: 18,
  shadowCascades: 19,
  loadP95Frames: 20,
  loadMaxFrames: 21,
  remeshP95Frames: 22,
  remeshMaxFrames: 23,
  stride2Tiles: 24,
  stride4Tiles: 25,
  stride8Tiles: 26,
  stride16Tiles: 27,
  waterQuads: 28,
  waterDrawCalls: 29,
  refractionCopyMiB: 30,
  immersion: 31,
  eyeDepthMetres: 32,
  eyesSubmerged: 33,
  swimming: 34,
  targetVoxelX: 35,
  targetVoxelY: 36,
  targetVoxelZ: 37,
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
  remoteAvatars: 98,
  avatarParts: 99,
  avatarDrawCalls: 100,
  viewportFingerprintLow24: 101,
  viewportFingerprintHigh24: 102,
  allLodsReady: 103,
  surfaceInFlight: 104,
  interactiveLodsReady: 105,
  stride32Tiles: 106,
  stride64Tiles: 107,
  stride128Tiles: 108,
  stride256Tiles: 109,
  renderCullMs: 110,
  renderEncodeMs: 111,
  renderSubmitMs: 112,
  drawListTestedSlices: 113,
  drawListSelectedSlices: 114,
  surfaceWidth: 115,
  surfaceHeight: 116,
  devicePixelRatio: 117,
  lodTransitionQuads: 118,
  lodBoundary0X: 119,
  lodBoundary0Z: 120,
  lodBoundary1X: 121,
  lodBoundary1Z: 122,
  lodBoundary2X: 123,
  lodBoundary2Z: 124,
  lodBoundary3X: 125,
  lodBoundary3Z: 126,
  lodBoundary4X: 127,
  lodBoundary4Z: 128,
  lodBoundary5X: 129,
  lodBoundary5Z: 130,
  lodBoundary6X: 131,
  lodBoundary6Z: 132,
  lodBoundary7X: 133,
  lodBoundary7Z: 134,
  dayFraction: 135,
  localSolarDayFraction: 136,
  yearFraction: 137,
  moonOrbitFraction: 138,
  twinklePhase: 139,
  latitudeDegrees: 140,
  longitudeDegrees: 141,
  localSiderealAngleRadians: 142,
  moonIlluminatedFraction: 143,
  celestialRevision: 144,
  sunDirectionX: 145,
  sunDirectionY: 146,
  sunDirectionZ: 147,
  moonDirectionX: 148,
  moonDirectionY: 149,
  moonDirectionZ: 150,
  shadowStrength: 151,
  cloudOffsetX: 152,
  cloudOffsetZ: 153,
  cloudVelocityX: 154,
  cloudVelocityZ: 155,
  weatherRevision: 156,
  weatherKind: 157,
  weatherFraction: 158,
  precipitation: 159,
  storminess: 160,
  lightning: 161,
  cloudDensity: 162,
  cloudBaseMetres: 163,
  cloudTopMetres: 164,
  cloudRenderWidth: 165,
  cloudRenderHeight: 166,
  cloudViewSteps: 167,
  cloudLightSteps: 168,
  fogDensity: 169,
  outdoorExposure: 170,
  spectatorActive: 171,
  presentedLodStrideVoxels: 172,
  lodFocusLagVoxels: 173,
  canonicalImmediateResident: 174,
  canonicalImmediateRequired: 175,
  canonicalSurfaceCellsResident: 176,
  canonicalSurfaceCellsRequired: 177,
  schemaVersion: 178,
  sampleCount: 179,
  droppedSamples: 180,
} as const);

export type SnapshotField = keyof typeof SNAPSHOT;

export interface EngineAutomationContract {
  readonly version: number;
  readonly snapshotVersion: number;
  readonly frameSampleWidth: number;
  readonly gpuSampleWidth: number;
  readonly semantics: {
    readonly playerEyeHeightMetres: number;
    readonly playerHeightMetres: number;
    readonly playerRadiusMetres: number;
    readonly editCubeEdgeVoxels: number;
    readonly editCubeVolumeVoxels: number;
    readonly editSphereRadiusVoxels: number;
    readonly editSphereVolumeVoxels: number;
  };
  readonly snapshotFields: readonly string[];
}

export type AutomationEditShape = "sphere" | "cube";

export interface EngineAutomationApi {
  contract(): Promise<EngineAutomationContract>;
  snapshot(): Promise<number[]>;
  profile(profileId: number): void;
  spectator(active: boolean): Promise<boolean>;
  look(deltaX: number, deltaY: number): void;
  submitPlace(
    x: number,
    y: number,
    z: number,
    materialId: number,
    shape: AutomationEditShape,
  ): Promise<boolean>;
  submitDig(x: number, y: number, z: number, shape: AutomationEditShape): Promise<boolean>;
  inventory(): Promise<number[]>;
  surfaceEditState(stride: number, x: number, z: number): Promise<number[]>;
  readonly player: BrowserPlayerSession;
  playerUrl(name: string): string;
}

export const SNAPSHOT_FIELD_NAMES: readonly string[] = (() => {
  const result = Array.from<string>({ length: SNAPSHOT.droppedSamples + 1 });
  for (const [name, index] of Object.entries(SNAPSHOT)) result[index] = name;
  const missing = result.findIndex((name) => name === undefined);
  if (missing >= 0) throw new Error(`TypeScript snapshot schema omits field ${missing}`);
  return Object.freeze(result);
})();

export function parseAutomationContract(value: string): EngineAutomationContract {
  const [version, snapshotVersion, frameSampleWidth, gpuSampleWidth, semantics, fields, ...extra] =
    value.split("\n");
  if (
    version === undefined ||
    snapshotVersion === undefined ||
    frameSampleWidth === undefined ||
    gpuSampleWidth === undefined ||
    semantics === undefined ||
    fields === undefined ||
    extra.length > 0
  ) {
    throw new Error("Rust automation contract has an invalid envelope");
  }
  const [
    playerEyeHeightMetres,
    playerHeightMetres,
    playerRadiusMetres,
    editCubeEdgeVoxels,
    editCubeVolumeVoxels,
    editSphereRadiusVoxels,
    editSphereVolumeVoxels,
    ...extraSemantics
  ] = semantics.split(",").map(Number);
  if (
    playerEyeHeightMetres === undefined ||
    playerHeightMetres === undefined ||
    playerRadiusMetres === undefined ||
    editCubeEdgeVoxels === undefined ||
    editCubeVolumeVoxels === undefined ||
    editSphereRadiusVoxels === undefined ||
    editSphereVolumeVoxels === undefined ||
    extraSemantics.length > 0
  ) {
    throw new Error("Rust automation contract has invalid gameplay semantics");
  }
  return Object.freeze({
    version: Number(version),
    snapshotVersion: Number(snapshotVersion),
    frameSampleWidth: Number(frameSampleWidth),
    gpuSampleWidth: Number(gpuSampleWidth),
    semantics: Object.freeze({
      playerEyeHeightMetres,
      playerHeightMetres,
      playerRadiusMetres,
      editCubeEdgeVoxels,
      editCubeVolumeVoxels,
      editSphereRadiusVoxels,
      editSphereVolumeVoxels,
    }),
    snapshotFields: Object.freeze(fields.split(",")),
  });
}

export function assertAutomationContract(contract: EngineAutomationContract): void {
  if (contract.version !== AUTOMATION_CONTRACT_VERSION) {
    throw new Error(
      `automation contract ${contract.version} does not match ${AUTOMATION_CONTRACT_VERSION}`,
    );
  }
  if (contract.snapshotVersion !== SNAPSHOT_SCHEMA_VERSION) {
    throw new Error(
      `snapshot schema ${contract.snapshotVersion} does not match ${SNAPSHOT_SCHEMA_VERSION}`,
    );
  }
  if (contract.frameSampleWidth !== FRAME_SAMPLE_WIDTH) {
    throw new Error(
      `frame sample width ${contract.frameSampleWidth} does not match ${FRAME_SAMPLE_WIDTH}`,
    );
  }
  if (contract.gpuSampleWidth !== GPU_SAMPLE_WIDTH) {
    throw new Error(
      `GPU sample width ${contract.gpuSampleWidth} does not match ${GPU_SAMPLE_WIDTH}`,
    );
  }
  const semantics = contract.semantics;
  if (
    typeof semantics !== "object" ||
    semantics === null ||
    !Object.values(semantics).every(Number.isFinite) ||
    semantics.playerEyeHeightMetres <= 0 ||
    semantics.playerHeightMetres < semantics.playerEyeHeightMetres ||
    semantics.playerRadiusMetres <= 0 ||
    !Number.isInteger(semantics.editCubeEdgeVoxels) ||
    semantics.editCubeEdgeVoxels <= 0 ||
    semantics.editCubeVolumeVoxels !== semantics.editCubeEdgeVoxels ** 3 ||
    semantics.editSphereRadiusVoxels <= 0 ||
    !Number.isInteger(semantics.editSphereVolumeVoxels) ||
    semantics.editSphereVolumeVoxels <= 0
  ) {
    throw new Error("Rust automation contract has invalid gameplay semantics");
  }
  if (
    contract.snapshotFields.length !== SNAPSHOT_FIELD_NAMES.length ||
    contract.snapshotFields.some((name, index) => name !== SNAPSHOT_FIELD_NAMES[index])
  ) {
    throw new Error("Rust and TypeScript snapshot field layouts differ");
  }
}

export function assertSnapshotSchema(snapshot: readonly number[]): readonly number[] {
  const actual = snapshot[SNAPSHOT.schemaVersion];
  if (actual !== SNAPSHOT_SCHEMA_VERSION) {
    throw new Error(`snapshot schema ${actual} does not match ${SNAPSHOT_SCHEMA_VERSION}`);
  }
  return snapshot;
}

declare global {
  var __VOXELS__: EngineAutomationApi | undefined;
}
