import type { BrowserPlayerSession } from "./local-player.ts";

export const AUTOMATION_CONTRACT_VERSION = 8;
export const SNAPSHOT_SCHEMA_VERSION = 60;
export const FRAME_SAMPLE_WIDTH = 22;
export const GPU_SAMPLE_WIDTH = 15;

// This map is the compact Rust snapshot ABI. Scenario code imports it from the typed engine
// capability; it must not maintain private numeric indices.
export const SNAPSHOT_FIELD_NAMES = [
  "cameraX",
  "cameraY",
  "cameraZ",
  "yaw",
  "pitch",
  "grounded",
  "quads",
  "edits",
  "residentChunks",
  "trackedChunks",
  "visibleChunks",
  "drawCalls",
  "arenaPages",
  "arenaAllocatedMiB",
  "arenaCapacityMiB",
  "pendingJobs",
  "frameMs",
  "shadowDrawCalls",
  "shadowCascades",
  "loadP95Frames",
  "loadMaxFrames",
  "remeshP95Frames",
  "remeshMaxFrames",
  "waterQuads",
  "waterDrawCalls",
  "refractionCopyMiB",
  "immersion",
  "eyeDepthMetres",
  "eyesSubmerged",
  "swimming",
  "targetVoxelX",
  "targetVoxelY",
  "targetVoxelZ",
  "targetPresent",
  "coreGpuMiB",
  "cpuMs",
  "simulationMs",
  "streamMs",
  "renderMs",
  "gpuSampleId",
  "gpuTotalMs",
  "gpuShadowMs",
  "gpuWorldMs",
  "gpuWaterMs",
  "gpuUiMs",
  "wasmCommittedMiB",
  "canonicalVoxelMiB",
  "pendingMeshMiB",
  "editLogicalMiB",
  "totalEvictions",
  "staleCompletions",
  "profilePhase",
  "profileElapsedSeconds",
  "profileDistanceMetres",
  "profileComplete",
  "profileTrackedHigh",
  "profilePendingHigh",
  "profilePendingMeshHigh",
  "profileArenaCapacityHighMiB",
  "profileWasmHighMiB",
  "profileEvictions",
  "materialDetail",
  "daylightPhase",
  "surfaceRegion",
  "cloudCoverage",
  "screenSpaceAmbientOcclusion",
  "gpuDepthPrepassMs",
  "gpuAmbientOcclusionMs",
  "ambientOcclusionMiB",
  "depthPrepassDrawCalls",
  "enclosure",
  "interiorExposure",
  "caveHeadlamp",
  "enclosureProbeUs",
  "localLightCandidates",
  "activeLocalLights",
  "clippedLocalLights",
  "occludedLocalLights",
  "portalRejectedLocalLights",
  "localLightVisibilityTests",
  "openCinderPortals",
  "cinderPortalRevision",
  "localLighting",
  "placementMaterial",
  "streamInterestRequested",
  "streamInterestNormalized",
  "streamInterestDesired",
  "streamInterestTruncated",
  "streamPlanOverflow",
  "portalActiveChunks",
  "portalActiveColumns",
  "unreachablePortalActive",
  "remoteAvatars",
  "avatarParts",
  "avatarDrawCalls",
  "viewportFingerprintLow24",
  "viewportFingerprintHigh24",
  "terrainReady",
  "renderCullMs",
  "renderEncodeMs",
  "renderSubmitMs",
  "drawListTestedSlices",
  "drawListSelectedSlices",
  "surfaceWidth",
  "surfaceHeight",
  "devicePixelRatio",
  "dayFraction",
  "localSolarDayFraction",
  "yearFraction",
  "moonOrbitFraction",
  "twinklePhase",
  "latitudeDegrees",
  "longitudeDegrees",
  "localSiderealAngleRadians",
  "moonIlluminatedFraction",
  "celestialRevision",
  "sunDirectionX",
  "sunDirectionY",
  "sunDirectionZ",
  "moonDirectionX",
  "moonDirectionY",
  "moonDirectionZ",
  "shadowStrength",
  "cloudOffsetX",
  "cloudOffsetZ",
  "cloudVelocityX",
  "cloudVelocityZ",
  "weatherRevision",
  "weatherKind",
  "weatherFraction",
  "precipitation",
  "storminess",
  "lightning",
  "cloudDensity",
  "cloudBaseMetres",
  "cloudTopMetres",
  "cloudRenderWidth",
  "cloudRenderHeight",
  "cloudViewSteps",
  "cloudLightSteps",
  "fogDensity",
  "outdoorExposure",
  "spectatorActive",
  "reproductionActive",
  "reproductionInvalidated",
  "canonicalLatticePresented",
  "canonicalImmediateResident",
  "canonicalImmediateRequired",
  "terrainColumnCellsOwned",
  "terrainColumnCellsRequired",
  "generationQueued",
  "generationInFlight",
  "meshingQueued",
  "meshingInFlight",
  "uploadQueued",
  "uploadInFlight",
  "loadCompleted",
  "loadInFlight",
  "acceptedCompletions",
  "collisionImmediateResident",
  "collisionImmediateRequired",
  "collisionLookaheadResident",
  "collisionLookaheadRequired",
  "collisionLookaheadSeconds",
  "editCanonicalRequired",
  "editCanonicalRenderable",
  "editCanonicalOwned",
  "enclosedViewResident",
  "enclosedViewRequired",
  "enclosedViewRenderable",
  "enclosedViewOwned",
  "virtualTerrainMode",
  "virtualTerrainExactDomainComplete",
  "virtualTerrainExactDomainRequiredLeaves",
  "virtualTerrainExactDomainCurrentCoverage",
  "virtualTerrainExactDomainFingerprintLow24",
  "virtualTerrainExactDomainFingerprintMid24",
  "virtualTerrainExactDomainFingerprintHigh16",
  "virtualTerrainExactCoreComplete",
  "virtualTerrainExactCoreRequiredLeaves",
  "virtualTerrainExactCoreCurrentCoverage",
  "virtualTerrainExactPredictionComplete",
  "virtualTerrainExactPredictionRequiredLeaves",
  "virtualTerrainExactPredictionCurrentCoverage",
  "virtualTerrainRegisteredRegions",
  "virtualTerrainDirectoryInFlight",
  "virtualTerrainDirectoryNodes",
  "virtualTerrainResidentPages",
  "virtualTerrainResidentMiB",
  "virtualTerrainResidentPrimitives",
  "virtualTerrainSelectedPages",
  "virtualTerrainRequestedPages",
  "virtualTerrainOwnerlessRoots",
  "virtualTerrainGpuMatchesCpuCut",
  "virtualTerrainGpuEncodingOverflowFlags",
  "virtualTerrainGpuEncodedPages",
  "virtualTerrainGpuOwnerlessRoots",
  "virtualTerrainStreamPending",
  "virtualTerrainStreamInFlight",
  "virtualTerrainCancellationWasteMiB",
  "virtualTerrainCachePages",
  "virtualTerrainCacheMiB",
  "virtualTerrainColumns",
  "virtualTerrainColumnInFlight",
  "virtualTerrainColumnRevisionFloors",
  "virtualTerrainCurrentColumnKnown",
  "virtualTerrainCurrentColumnRoots",
  "virtualTerrainCurrentColumnRegisteredRoots",
  "virtualTerrainNearestRegisteredRootMetres",
  "virtualTerrainColumnAccepted",
  "virtualTerrainColumnSubmitDeferred",
  "virtualTerrainColumnPreempted",
  "virtualTerrainColumnTimedOut",
  "virtualTerrainColumnOtherFailed",
  "virtualTerrainDirectoryAccepted",
  "virtualTerrainDirectorySubmitDeferred",
  "virtualTerrainDirectoryPreempted",
  "virtualTerrainDirectoryTimedOut",
  "virtualTerrainDirectoryOtherFailed",
  "virtualTerrainPageSubmitDeferred",
  "virtualTerrainPagePreempted",
  "virtualTerrainPageTimedOut",
  "virtualTerrainPageOtherFailed",
  "virtualTerrainPageUnavailable",
  "virtualTerrainPageStaleRevision",
  "virtualTerrainPageGenerationFailed",
  "virtualTerrainPageUploadFailed",
  "virtualTerrainPublishedPages",
  "virtualTerrainPublishedExactPages",
  "virtualTerrainPublishedMinimumLevel",
  "virtualTerrainPublishedMaximumLevel",
  "virtualTerrainPublishedExactLodDiscontinuities",
  "virtualTerrainCutFingerprintLow24",
  "virtualTerrainCutFingerprintHigh24",
  "virtualTerrainPresentedSnapshotGenerationLow24",
  "virtualTerrainPresentedSnapshotGenerationHigh24",
  "virtualTerrainPresentedSnapshotFingerprintLow24",
  "virtualTerrainPresentedSnapshotFingerprintHigh24",
  "virtualTerrainPresentedSnapshotMatchesCut",
  "virtualTerrainPresentedCoverageGapFrames",
  "virtualTerrainDesiredEnvelopeComplete",
  "virtualTerrainDesiredEnvelopeFingerprintLow24",
  "virtualTerrainDesiredEnvelopeFingerprintMid24",
  "virtualTerrainDesiredEnvelopeFingerprintHigh16",
  "virtualTerrainDesiredSafetyLeaves",
  "virtualTerrainDesiredHorizonRoots",
  "virtualTerrainDesiredLocusMinimumLeafX",
  "virtualTerrainDesiredLocusMinimumLeafZ",
  "virtualTerrainDesiredLocusMaximumLeafExclusiveX",
  "virtualTerrainDesiredLocusMaximumLeafExclusiveZ",
  "virtualTerrainCommittedEnvelopeFingerprintLow24",
  "virtualTerrainCommittedEnvelopeFingerprintMid24",
  "virtualTerrainCommittedEnvelopeFingerprintHigh16",
  "virtualTerrainCommittedSafetyLeaves",
  "virtualTerrainCommittedSafetyCoverage",
  "virtualTerrainCommittedHorizonRoots",
  "virtualTerrainCommittedHorizonCoverage",
  "virtualTerrainCommittedLocusMinimumLeafX",
  "virtualTerrainCommittedLocusMinimumLeafZ",
  "virtualTerrainCommittedLocusMaximumLeafExclusiveX",
  "virtualTerrainCommittedLocusMaximumLeafExclusiveZ",
  "presentedCameraInsideCommittedEnvelope",
  "presentationTargetX",
  "presentationTargetY",
  "presentationTargetZ",
  "presentationGateActive",
  "presentationGateStepsLow24",
  "presentationGateStepsMid24",
  "presentationGateStepsHigh16",
  "presentationGateFramesLow24",
  "presentationGateFramesMid24",
  "presentationGateFramesHigh16",
  "clientViewGoalKind",
  "clientViewAttemptPresent",
  "clientViewAttemptCanonicalReady",
  "clientViewAttemptTerrainStatus",
  "virtualTerrainPublicationInFlight",
  "virtualTerrainPlanLastSelection",
  "virtualTerrainPlanLastInvalidation",
  "virtualTerrainPlanLastInvalidationLine",
  "virtualTerrainPublicationLastAbortLine",
  "frameSequence",
  "schemaVersion",
  "sampleCount",
  "droppedSamples",
] as const;

export const SNAPSHOT = Object.freeze(
  Object.fromEntries(SNAPSHOT_FIELD_NAMES.map((name, index) => [name, index])),
) as Readonly<Record<(typeof SNAPSHOT_FIELD_NAMES)[number], number>>;

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
  applyReproduction(metadata: string): Promise<void>;
  clearReproduction(): void;
  spectator(active: boolean): Promise<boolean>;
  diagnosticSky(rgb: readonly [number, number, number] | null): Promise<boolean>;
  geometrySourceDebug(enabled: boolean): Promise<boolean>;
  materialDetail(enabled: boolean): Promise<boolean>;
  exactVolumePresented(x: number, y: number, z: number): Promise<boolean>;
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
  readonly player: BrowserPlayerSession;
  playerUrl(name: string): string;
}

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
