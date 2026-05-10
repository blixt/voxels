import { average, maxValue, percentile } from "../engine/benchmark-metrics.ts";
import {
  analyzeSettledReferenceDiff,
  analyzeBottomCenterVoid,
  buildDefaultRouteBenchmarkPlan,
  buildForwardRouteBenchmarkPlan,
  countRouteSeamFrameClasses,
  summarizeRouteFrameAccounting,
  summarizeRouteSeamCoverage,
  type BottomCenterVoidProbe,
  type RouteBenchmarkFrameTarget,
} from "../engine/game-route-benchmark.ts";
import {
  summarizeBootstrapBenchmark,
  type BootstrapBenchmarkSample,
  type BootstrapBenchmarkSummary,
} from "../engine/game-bootstrap-benchmark.ts";
import {
  buildFirstPersonCameraMatrices,
  createFirstPersonCamera,
  rotateFirstPersonCamera,
  type FirstPersonCameraState,
} from "../engine/first-person-camera.ts";
import {
  ExplorationJournal,
  type DiscoveryEvent,
  type ExplorationJournalState,
  type ExplorationJournalSnapshot,
  type ExplorationObservation,
} from "../engine/exploration-journal.ts";
import {
  resolveExplorationInteractionTarget,
  type ExplorationInteractionCandidate,
  type ExplorationInteractionVerb,
  type ResolvedExplorationInteractionTarget,
} from "../engine/exploration-interactions.ts";
import {
  ExplorationEventLog,
  type ExplorationEventLogSnapshot,
  type ExplorationEventLogState,
} from "../engine/exploration-events.ts";
import { summarizeFieldKit } from "../engine/field-kit.ts";
import { summarizeBestiary } from "../engine/bestiary-journal.ts";
import {
  getLootJournalCandidateState,
  summarizeLootJournal,
  type LootJournalCandidateState,
} from "../engine/loot-journal.ts";
import { findSafeCaveEntryFeetPosition } from "../engine/cave-traversal.ts";
import { describeNavigationBearing } from "../engine/navigation-bearing.ts";
import {
  samplePassiveMobSightingsWorldUnits,
  type PassiveMobSighting,
} from "../engine/passive-mob-sim.ts";
import {
  SkillJournal,
  type SkillId,
  type SkillJournalState,
  type SkillJournalSnapshot,
} from "../engine/skill-journal.ts";
import {
  RouteJournal,
  type TravelGoalProgressInput,
  type TravelGoalProgressResult,
  type RouteJournalSnapshot,
  type RouteJournalState,
  type TravelGoalDefinition,
  type TravelGoalSnapshot,
  type TravelGoalStepKind,
} from "../engine/travel-goals.ts";
import {
  buildTravelGoalFromQuestHook,
  planRpgQuestHooks,
  selectRpgQuestHookForExploration,
  type RpgQuestHookSummary,
} from "../engine/rpg-quests.ts";
import {
  WORLD_ATLAS,
} from "../engine/world-atlas.ts";
import {
  describeExplorationSkillEffects,
  type ExplorationSkillEffects,
} from "../engine/exploration-skill-effects.ts";
import {
  FrameTimingBuckets,
  type FrameTimingSnapshot,
} from "../engine/frame-timing-buckets.ts";
import {
  buildChunkMesh,
  buildChunkMeshFromOpaqueGeometry,
  collectDirtyChunks,
  createOpaqueChunkMeshingInput,
  rebuildDirtyMeshes,
} from "../engine/mesher.ts";
import { createMeshMaterialLut } from "../engine/opaque-chunk-mesher.ts";
import { createAsyncChunkMeshing } from "./async-chunk-meshing.ts";
import { createAsyncProceduralChunkGeneration } from "./async-procedural-chunk-generation.ts";
import {
  createPlayerState,
  getPlayerEyePosition,
  PLAYER_EYE_HEIGHT,
  stepPlayer,
  teleportPlayerToEyePosition,
  type PlayerState,
} from "../engine/player-physics.ts";
import {
  diffChunkCoords,
  summarizeResidentWorld,
  type ResidentWorldProbeSnapshot,
} from "../engine/procedural-probes.ts";
import {
  ProceduralResidentWorld,
  type LodChunkDebugState,
  type LodVisibleColumnState,
  type LodResidencyUpdateSummary,
  type ResidencyUpdateSummary,
  type WorldEditRecord,
} from "../engine/procedural-resident-world.ts";
import {
  isProceduralWaterMaterial,
  ProceduralWorldGenerator,
  type ProceduralBiomeProbe,
} from "../engine/procedural-generator.ts";
import {
  WebGpuVoxelRenderer,
  type RenderStats,
} from "../engine/renderer.ts";
import { setChunkMeshDirtyState, type VoxelChunk } from "../engine/world.ts";
import { metersToWorldUnits, worldUnitsToMeters } from "../engine/scale.ts";
import {
  shouldPumpWorldWork,
  shouldRefreshResidency,
} from "../engine/stream-work.ts";
import {
  buildStreamAnchorPosition,
  resolveStreamAnchor,
  type StreamAnchor,
} from "../engine/stream-anchor.ts";
import type { Vec3 } from "../engine/types.ts";
import {
  buildAmbientRenderEnvironment,
  resolveAmbientWorldProfile,
  type AmbientWorldProfile,
} from "../engine/ambient-environment.ts";
import {
  applyWorldAtmosphere,
  sampleWorldSystems,
  type WorldSystemSnapshot,
} from "../engine/world-systems.ts";
import {
  buildUnderwaterRenderEnvironment,
  type RenderEnvironment,
} from "../engine/water-visuals.ts";
import { resolveObservedUndergroundBiomeId } from "../engine/underground-discovery.ts";
import type { MeshBuildSummary } from "../engine/mesher.ts";
import {
  describeDiscovery,
  formatDiscoveryName,
  type DiscoveryRole,
} from "../engine/discovery-catalog.ts";
import {
  describeRpgEncounterFaction,
  describeRpgEncounterMood,
  describeRpgEncounterScoutResult,
  sampleRpgEncounterWorldUnits,
  type RpgEncounterSample,
} from "../engine/rpg-encounters.ts";
import { sampleRpgEncounterSiteWorldUnits } from "../engine/rpg-encounter-sites.ts";
import { sampleForageSiteWorldUnits } from "../engine/forage-sites.ts";

const MAX_DELTA_SECONDS = 0.05;
const HUD_PUSH_INTERVAL_MS = 120;
const STREAM_ANCHOR_MARGIN_CHUNKS = 1;
const DEFAULT_MAX_GENERATED_CHUNKS_PER_UPDATE = 7;
const DEFAULT_MAX_MESH_REBUILDS_PER_FRAME = 6;
const DEFAULT_MAX_LOD_CHUNKS_PER_FRAME = 1;
const DEFAULT_MAX_LOD_ADOPTIONS_PER_FRAME = 1;
const DEFAULT_MAX_LOD_PLAN_MS_PER_FRAME = 3;
const DEFAULT_MAX_LOD_WORK_MS_PER_FRAME = 8;
const PASSIVE_MOB_SIGHTING_RADIUS_WORLD_UNITS = metersToWorldUnits(96);
const PASSIVE_MOB_SIGHTING_CAP = 6;
const MOVING_LOD_UPDATE_INTERVAL_FRAMES = 4;
const MOVING_MAX_RESIDENCY_PLAN_MS_PER_FRAME = 5;
const MOVING_MAX_EVICT_CHUNKS_PER_FRAME = 32;
const MOVING_MAX_MESH_REBUILDS_PER_FRAME = 4;
const MOVING_MAX_LOD_CHUNKS_PER_FRAME = 1;
const MOVING_MAX_LOD_ADOPTIONS_PER_FRAME = 1;
const MOVING_MAX_LOD_PLAN_MS_PER_FRAME = 0.75;
const MOVING_MAX_LOD_WORK_MS_PER_FRAME = 4;
const MAX_SYNC_NEAR_MESH_REBUILDS_PER_FRAME = 6;
const SYNC_NEAR_MESH_RADIUS_CHUNKS = 3;
const BOOTSTRAP_PLAYABLE_COLUMN_RADIUS_CHUNKS = 2;
const CAVE_MOUTH_INTERACTION_CORE_THRESHOLD = 0.55;
const DISCOVERY_SAMPLE_INTERVAL_MS = 250;
const DISCOVERY_SAMPLE_MOVE_THRESHOLD_WORLD_UNITS = metersToWorldUnits(0.8);
const TRAVEL_CONTEXT_SAMPLE_INTERVAL_MS = 250;
const TRAVEL_CONTEXT_SAMPLE_MOVE_THRESHOLD_WORLD_UNITS = metersToWorldUnits(0.8);
const ANCIENT_LANDMARK_IDS = new Set([
  "ancestor_pillar",
  "ash_marker",
  "glass_cairn",
  "silt_shell",
  "velothi_shrine",
  "pilgrim_cairn",
  "velothi_ziggurat",
  "ash_obelisk",
  "rib_arch",
  "old_road_causeway",
  "pilgrim_lantern",
  "bone_chimes",
]);
const ROUTE_LANDMARK_IDS = new Set([
  "ancestor_pillar",
  "ash_marker",
  "glass_cairn",
  "silt_shell",
  "velothi_shrine",
  "pilgrim_cairn",
  "old_road_causeway",
  "pilgrim_lantern",
  "bone_chimes",
  "ashlander_travel_pack",
]);
const DEFAULT_TRAVEL_GOAL_ID = "first-bearings";
const BASE_TRAVEL_GOALS = [
  {
    id: DEFAULT_TRAVEL_GOAL_ID,
    routeId: "pilgrim-road",
    title: "First Bearings",
    journalText: "Get your bearings on the pilgrim road.",
    steps: [
      { id: "inspect-causeway", kind: "inspect", targetId: "old_road_causeway", label: "Inspect the old causeway" },
      { id: "read-shrine", kind: "read", targetId: "velothi_shrine", label: "Read the wayshrine" },
      { id: "use-pack", kind: "use", targetId: "ashlander_travel_pack", label: "Use the travel pack", optional: true },
    ],
  },
  {
    id: "ash-road",
    routeId: "ash-road",
    title: "Ash Road",
    journalText: "Follow the old ash markers into harsher country.",
    steps: [
      { id: "visit-ash-marker", kind: "visit", targetId: "ash_marker", label: "Reach an ash marker" },
    ],
  },
] as const satisfies readonly TravelGoalDefinition[];
const TRAVEL_GOALS: readonly TravelGoalDefinition[] = [
  ...BASE_TRAVEL_GOALS,
  ...buildQuestTravelGoals(),
];
const LANDMARK_SAMPLE_OFFSET_CACHE = new Map<string, ReadonlyArray<readonly [number, number]>>();

export interface GameHudSnapshot {
  status: string;
  pointerLocked: boolean;
  position: Vec3;
  feetPosition: Vec3;
  playerChunk: [number, number, number];
  streamAnchorChunk: [number, number];
  grounded: boolean;
  bodyInWater: boolean;
  eyeInWater: boolean;
  yawDegrees: number;
  pitchDegrees: number;
  solidVoxelCount: number;
  chunkCount: number;
  paletteCount: number;
  streamMs: number;
  streamGeneratedChunks: number;
  streamEvictedChunks: number;
  streamPendingChunks: number;
  streamEmptyChunksSkipped: number;
  streamCachedEmptyChunkHits: number;
  streamCompletedChunkCacheHits: number;
  streamCompletedGeneratedChunks: number;
  generationWorkerCount: number;
  streamCompletedSummaryCacheHits: number;
  streamCompletedGeneratedSummaries: number;
  streamCompletedRegionSummaryCacheHits: number;
  streamMissingRegionSummaries: number;
  streamDirtyResidentChunks: number;
  residencyRadiusChunks: number;
  surfaceY: number;
  worldRegionId: string | null;
  worldRegionStrength: number;
  biomeId: string | null;
  undergroundBiomeId: string | null;
  regionalVariantId: string | null;
  landmarkId: string | null;
  ambientProfileId: string;
  ambientProfileLabel: string;
  ambientFogEndMeters: number;
  timeOfDayLabel: string;
  worldClockLabel: string;
  worldDay: number;
  worldDaylight: number;
  weatherLabel: string;
  weatherIntensity: number;
  floraLabel: string;
  faunaLabel: string;
  lootSignalLabel: string;
  hazardLabel: string;
  areaCoherenceLabel: string;
  encounterMoodLabel: string;
  encounterPressureLabel: string;
  encounterFactionLabel: string;
  encounterFlavorLabel: string;
  passiveMobSightingCount: number;
  passiveMobNearestId: string | null;
  passiveMobNearestLabel: string;
  passiveMobNearestDetailLabel: string;
  passiveMobNearestDistanceMeters: number | null;
  passiveMobNearestFactionLabel: string;
  passiveMobNearestMoodLabel: string;
  bestiarySightingCount: number;
  bestiaryEntryCount: number;
  bestiarySummaryLabel: string;
  bestiaryLastSightingLabel: string;
  bestiaryLastNoteLabel: string;
  bestiaryDominantFactionLabel: string;
  activePlaceName: string;
  activeRouteName: string;
  activeRouteProgressLabel: string;
  activeTravelGoalTitle: string;
  activeTravelGoalStepLabel: string;
  activeTravelGoalProgressRatio: number;
  activeQuestHookId: string | null;
  activeQuestHookKind: string | null;
  activeQuestObjectiveKind: string | null;
  activeQuestTitle: string;
  activeQuestObjectiveLabel: string;
  activeQuestRumorText: string;
  activeQuestMoodLabel: string;
  activeQuestFactionLabel: string;
  travelContext: "surface" | "underground";
  travelContextLabel: string;
  interactionTargetName: string;
  interactionPromptLabel: string;
  interactionPromptDescription: string;
  interactionPromptVerb: ExplorationInteractionVerb | null;
  navigationTargetId: string | null;
  navigationTargetName: string | null;
  navigationSource: "interaction-target" | null;
  navigationDistanceMeters: number | null;
  navigationDistanceLabel: string | null;
  navigationCompassLabel: string | null;
  navigationBearingLabel: string | null;
  navigationTurnLabel: string | null;
  lastInteractionLabel: string;
  discoveredBiomeCount: number;
  discoveredUndergroundBiomeCount: number;
  discoveredRegionalVariantCount: number;
  discoveredLandmarkCount: number;
  discoveredAncientLandmarkCount: number;
  scoutedMobTrailCount: number;
  lootedCacheCount: number;
  scoutedCaveMouthCount: number;
  fieldKitFindCount: number;
  fieldKitSummaryLabel: string;
  fieldKitLastFindLabel: string;
  fieldKitLastNoteLabel: string;
  fieldKitDominantCategoryLabel: string;
  lootJournalCollectedCacheCount: number;
  lootJournalRevisitedCacheCount: number;
  lootJournalRevisitEventCount: number;
  lootJournalStateLabel: string;
  landmarkScanRadiusMeters: number;
  landmarkScanSampleCount: number;
  surfaceTravelSpeedMultiplier: number;
  undergroundTravelSpeedMultiplier: number;
  recentDiscoveries: DiscoveryEvent[];
  lastDiscoveryLabel: string;
  focusSkillName: string;
  focusSkillLevel: number;
  focusSkillProgressRatio: number;
  totalSkillTravelMeters: number;
  totalSkillLevel: number;
  bootstrapPlayableReady: boolean;
  bootstrapVisualReady: boolean;
  bootstrapElapsedMs: number;
  bootstrapRequiredColumns: number;
  bootstrapReadyColumns: number;
  bootstrapUrgentDirtyMeshlessChunks: number;
  bootstrapPendingMeshJobs: number;
  meshMs: number;
  meshNewChunks: number;
  meshRemeshChunks: number;
  drawCalls: number;
  triangles: number;
  lastFrameWallMs: number;
  frameTiming: FrameTimingSnapshot;
  lastHitchAttribution: GameFrameAttribution;
  lastGameplayFrameMs: number;
  lastFrameCpuMs: number;
  avgFrameWallMs: number;
  lastFrameSyncMs: number;
  lastFrameUploadMs: number;
  lastFrameUploadChunks: number;
  lastFrameUploadBytes: number;
  lastFrameEncodeMs: number;
  avgFrameCpuMs: number;
  maxGeneratedChunksPerUpdate: number;
  maxMeshRebuildsPerFrame: number;
  lodChunkCount: number;
  lodChunkCountByLevel: readonly number[];
  lodPendingChunks: number;
  lodPendingPlanning: number;
  lodPendingDiskCache: number;
  lodPendingDiskCacheByLevel: readonly number[];
  lodPendingGenerationBudget: number;
  lodPendingGenerationBudgetByLevel: readonly number[];
  lodPendingPartialBuild: number;
  lodPendingPartialBuildByLevel: readonly number[];
  lodPendingPrepared: number;
  lodPendingPreparedByLevel: readonly number[];
  lodPendingInvalidatedEviction: number;
  lodGeneratedChunks: number;
  lodGeneratedChunksByLevel: readonly number[];
  lodCacheHits: number;
  lodCacheHitsByLevel: readonly number[];
  lodEmptyCacheHits: number;
  lodEmptyCacheHitsByLevel: readonly number[];
  lodCachedChunks: number;
  lodCachedEmptyKeys: number;
  lodElapsedMs: number;
  lodYRangeMs: number;
  lodDownsampleMs: number;
  lodMeshMs: number;
  lodCommitMs: number;
  lodMaxChunkMs: number;
  lodMaxChunkLevel: number;
  lodMaxChunkKey: string | null;
  lodNeededKeyCount: number;
  lodNeededKeyCacheHit: boolean;
  lodScheduledRegionSummaryRequests: number;
  lodDiskCacheHits: number;
  lodDiskCacheHitsByLevel: readonly number[];
  lodDiskCacheMisses: number;
  lodWorkerGenerated: number;
  lodWorkerGeneratedByLevel: readonly number[];
  lodScheduledWorkerRequests: number;
  lodScheduledDiskRequests: number;
  lodScheduledDiskStores: number;
  lodCompletedDiskStores: number;
  cumulativeLodGeneratedChunks: number;
  cumulativeLodWorkerGenerated: number;
  cumulativeLodDiskCacheHits: number;
  cumulativeLodDiskCacheMisses: number;
  cumulativeLodScheduledDiskRequests: number;
  cumulativeLodScheduledDiskStores: number;
  cumulativeLodCompletedDiskStores: number;
  lodDrawCalls: number;
  lodDrawCallsByLevel: readonly number[];
  frustumCulledChunks: number;
  fogCulledChunks: number;
}

export interface GameFrameAttribution {
  frame: number;
  wallMs: number;
  gameplayMs: number;
  movementMs: number;
  streamMs: number;
  meshMs: number;
  lodMs: number;
  renderCpuMs: number;
  renderSyncMs: number;
  renderUploadMs: number;
  renderEncodeMs: number;
  cause: string;
}

interface CurrentWorldProbeContext {
  probe: ProceduralBiomeProbe;
  observedUndergroundBiomeId: string | null;
  ambientProfile: AmbientWorldProfile;
}

export interface ProgressStateSnapshot {
  version: 1;
  discovery: ExplorationJournalState;
  events?: ExplorationEventLogState;
  skills: SkillJournalState;
  routes: RouteJournalState;
}

interface ActiveExplorationHudState {
  activePlaceName: string;
  activeRouteName: string;
  activeRouteProgressLabel: string;
  activeTravelGoalTitle: string;
  activeTravelGoalStepLabel: string;
  activeTravelGoalProgressRatio: number;
  interactionTargetName: string;
  interactionPromptLabel: string;
  interactionPromptDescription: string;
  interactionPromptVerb: ExplorationInteractionVerb | null;
  navigationTargetId: string | null;
  navigationTargetName: string | null;
  navigationSource: "interaction-target" | null;
  navigationDistanceMeters: number | null;
  navigationDistanceLabel: string | null;
  navigationCompassLabel: string | null;
  navigationBearingLabel: string | null;
  navigationTurnLabel: string | null;
}

interface BootstrapReadiness {
  dirtyResidentMeshes: ReturnType<typeof summarizeDirtyResidentMeshes>;
  pendingMeshJobs: number;
  playableReady: boolean;
  visualReady: boolean;
  requiredColumns: number;
  readyColumns: number;
  urgentDirtyMeshlessChunks: number;
}

export interface GameRenderProbe {
  frameCpuMs: number;
  syncResourcesMs: number;
  uploadMs: number;
  uploadChunks: number;
  uploadBytes: number;
  encodeMs: number;
  drawCalls: number;
  triangles: number;
}

export interface ResidencyTransitionProbe {
  before: ResidentWorldProbeSnapshot;
  after: ResidentWorldProbeSnapshot;
  enteredChunkCoords: Array<[number, number, number]>;
  evictedChunkCoords: Array<[number, number, number]>;
  generatedChunkCoords: Array<[number, number, number]>;
  residency: ResidencyUpdateSummary;
  mesh: MeshBuildSummary;
  render: GameRenderProbe;
  settleFrames: number;
  settled: boolean;
}

export interface ChunkBoundaryBenchmarkSample {
  step: number;
  targetEyePosition: Vec3;
  targetChunk: [number, number, number];
  changed: boolean;
  generatedChunks: number;
  evictedChunks: number;
  streamMs: number;
  meshMs: number;
  meshNewChunks: number;
  meshRemeshChunks: number;
  frameCpuMs: number;
  syncMs: number;
  uploadMs: number;
  uploadChunks: number;
  uploadBytes: number;
  encodeMs: number;
}

export interface ChunkBoundaryBenchmarkSummary {
  sampleCount: number;
  changedCount: number;
  avgStreamMs: number;
  p95StreamMs: number;
  maxStreamMs: number;
  avgMeshMs: number;
  p95MeshMs: number;
  maxMeshMs: number;
  avgFrameCpuMs: number;
  p95FrameCpuMs: number;
  maxFrameCpuMs: number;
  avgSyncMs: number;
  p95SyncMs: number;
  maxSyncMs: number;
  avgUploadMs: number;
  p95UploadMs: number;
  maxUploadMs: number;
  avgUploadChunks: number;
  maxUploadChunks: number;
  avgUploadBytes: number;
  maxUploadBytes: number;
}

export interface ChunkBoundaryBenchmark {
  iterations: number;
  chunkDelta: number;
  radiusChunks: number;
  samples: ChunkBoundaryBenchmarkSample[];
  summary: ChunkBoundaryBenchmarkSummary;
}

export interface ChunkCacheReuseLegSummary {
  targetChunk: [number, number, number];
  frameCount: number;
  settled: boolean;
  totalStreamMs: number;
  totalMeshMs: number;
  totalGeneratedChunks: number;
  totalPersistedChunkHits: number;
  totalPersistedSummaryHits: number;
  totalPersistedRegionSummaryHits: number;
  totalMissingRegionSummaries: number;
  totalWorkerGeneratedChunks: number;
  maxPendingChunks: number;
  residentChunks: number;
}

export interface ChunkCacheReuseBenchmark {
  chunkDelta: number;
  radiusChunks: number;
  populate: ChunkCacheReuseLegSummary;
  revisit: ChunkCacheReuseLegSummary;
}

export interface IncrementalCrossingSample {
  frame: number;
  phase: "move" | "settle";
  leg: number;
  changed: boolean;
  complete: boolean;
  pendingChunks: number;
  generatedChunks: number;
  evictedChunks: number;
  streamMs: number;
  meshMs: number;
  meshCount: number;
  residentNearSamples: number;
  renderReadyNearSamples: number;
  residentNotReadyNearSamples: number;
  frameCpuMs: number;
  syncMs: number;
  uploadMs: number;
  uploadChunks: number;
  uploadBytes: number;
  encodeMs: number;
}

export interface IncrementalCrossingSummary {
  sampleCount: number;
  workFrameCount: number;
  changedCount: number;
  incompleteFrameCount: number;
  avgWorkMs: number;
  p95WorkMs: number;
  maxWorkMs: number;
  avgResidentNotReadyNearSamples: number;
  maxResidentNotReadyNearSamples: number;
  avgStreamMs: number;
  p95StreamMs: number;
  maxStreamMs: number;
  avgMeshMs: number;
  p95MeshMs: number;
  maxMeshMs: number;
  avgFrameCpuMs: number;
  p95FrameCpuMs: number;
  maxFrameCpuMs: number;
  avgUploadMs: number;
  p95UploadMs: number;
  maxUploadMs: number;
  maxPendingChunks: number;
}

export interface IncrementalCrossingBenchmark {
  iterations: number;
  chunkDelta: number;
  stepsPerLeg: number;
  settleFrames: number;
  radiusChunks: number;
  samples: IncrementalCrossingSample[];
  summary: IncrementalCrossingSummary;
}

export interface StreamingBudgets {
  maxGeneratedChunksPerUpdate: number;
  maxMeshRebuildsPerFrame: number;
}

interface LodUpdateBudget {
  maxGenerateLodChunks: number;
  maxAdoptCompletedLodChunks: number;
  maxPlanMs: number;
  maxWorkMs: number;
}

export interface LodCoverageIssueSample {
  worldX: number;
  worldZ: number;
  distanceMeters: number;
  bands: string[];
  sampleStrideMeters: number[];
  ownerChunks?: string[];
  ownerStates?: LodCoverageOwnerState[];
  verticalRanges?: LodCoverageVerticalRange[];
}

export interface LodCoverageProbe {
  center: Vec3;
  sampleRadiusMeters: number;
  sampleStepMeters: number;
  sampleCount: number;
  residentSampleCount: number;
  renderReadySampleCount: number;
  visibleLod0OwnerSampleCount: number;
  coveredSampleCount: number;
  residentOverlapCount: number;
  uncoveredGapCount: number;
  handoffHoleCount: number;
  bandOverlapCount: number;
  waterOverlapCount: number;
  wrongBandCount: number;
  residentOverlapSamples: LodCoverageIssueSample[];
  uncoveredGapSamples: LodCoverageIssueSample[];
  handoffHoleSamples: LodCoverageIssueSample[];
  bandOverlapSamples: LodCoverageIssueSample[];
  waterOverlapSamples: LodCoverageIssueSample[];
  wrongBandSamples: LodCoverageIssueSample[];
}

interface LodCoverageSpan {
  label: string;
  strideMeters: number;
  chunk: VoxelChunk;
  classifyColumn: (worldX: number, worldZ: number) => LodVisibleColumnState;
  state: LodChunkDebugState;
  chunkSize: number;
  minX: number;
  maxX: number;
  minZ: number;
  maxZ: number;
}

interface LodCoverageOwnerState extends LodChunkDebugState {
  ownerChunk: string;
}

interface LodCoverageVerticalRange {
  band: string;
  ownerChunk: string;
  minY: number;
  maxY: number;
}

export interface RenderReadyCoverageProbe {
  center: Vec3;
  sampleRadiusMeters: number;
  sampleStepMeters: number;
  sampleCount: number;
  residentSampleCount: number;
  renderReadySampleCount: number;
  residentNotReadyCount: number;
  missingResidentCount: number;
}

export interface VisibleGroundCoverageIssueSample {
  worldX: number;
  worldZ: number;
  forwardMeters: number;
  lateralMeters: number;
  resident: boolean;
  renderReady: boolean;
}

export interface VisibleGroundCoverageProbe {
  center: Vec3;
  yawRadians: number;
  sampleForwardMeters: number;
  sampleLateralMeters: number;
  sampleStepMeters: number;
  sampleCount: number;
  renderReadyCount: number;
  uncoveredCount: number;
  residentNotReadyCount: number;
  uncoveredSamples: VisibleGroundCoverageIssueSample[];
}

export interface SurfaceContinuityIssueSample {
  worldX: number;
  worldZ: number;
  neighborWorldX: number;
  neighborWorldZ: number;
  deltaMeters: number;
  renderReady: boolean;
  neighborRenderReady: boolean;
}

export interface SurfaceContinuityProbe {
  center: Vec3;
  yawRadians: number;
  sampleForwardMeters: number;
  sampleLateralMeters: number;
  sampleStepMeters: number;
  maxSmoothStepMeters: number;
  sampleCount: number;
  edgeCount: number;
  smoothEdgeCount: number;
  missingSmoothEdgeCount: number;
  abruptEdgeCount: number;
  maxExpectedStepMeters: number;
  issueSamples: SurfaceContinuityIssueSample[];
}

export interface RouteExperienceBenchmarkOptions {
  durationSeconds?: number;
  settleSeconds?: number;
  sampleHz?: number;
  speedMetersPerSecond?: number;
  seamProbeStrideFrames?: number;
  captureStrideFrames?: number;
  captureWidth?: number;
  captureHeight?: number;
  referenceDiffStrideFrames?: number;
  referenceDiffLimit?: number;
}

export interface RouteExperienceFrameSample {
  frame: number;
  phase: "move" | "settle";
  simTimeSeconds: number;
  routeDistanceMeters: number;
  feetPosition: Vec3;
  yaw: number;
  pitch: number;
  changed: boolean;
  complete: boolean;
  pendingChunks: number;
  pendingMeshJobs: number;
  dirtyResidentChunks: number;
  dirtyMeshlessResidentChunks: number;
  dirtyRetainedMeshResidentChunks: number;
  generatedChunks: number;
  evictedChunks: number;
  movementMs: number;
  streamMs: number;
  meshMs: number;
  meshCount: number;
  gameplayFrameMs: number;
  accountedFrameMs: number;
  unmeasuredFrameMs: number;
  diagnosticsMs: number;
  captureDiagnosticsMs: number;
  renderCpuMs: number;
  renderSyncMs: number;
  renderUploadMs: number;
  renderEncodeMs: number;
  renderOtherMs: number;
  uploadChunks: number;
  uploadBytes: number;
  drawCalls: number;
  triangles: number;
  residentNearSamples: number;
  renderReadyNearSamples: number;
  residentNotReadyNearSamples: number;
  visibleGroundSampleCount: number;
  visibleGroundUncoveredCount: number;
  visibleGroundResidentNotReadyCount: number;
  surfaceContinuityEdgeCount: number;
  surfaceContinuityGapCount: number;
  abruptSurfaceEdgeCount: number;
  maxSurfaceContinuityStepMeters: number;
  lodMs: number;
  lodGeneratedChunks: number;
  lodPendingChunks: number;
  lodMaxChunkMs: number;
  lodMaxChunkLevel: number;
  lodMaxChunkKey: string | null;
  farLodCoverageGapCount: number;
  uncoveredFarLodGapCount: number;
  handoffFarLodHoleCount: number;
  maxFarLodCoverageGapMeters: number;
  seamGapCount: number;
  uncoveredLodGapCount: number;
  handoffLodHoleCount: number;
  maxSeamGapMeters: number;
  lodOverlapCount: number;
  lodResidentOverlapCount: number;
  lodBandOverlapCount: number;
  maxLodOverlapMeters: number;
  screenVoidRatio: number | null;
  screenVoidMaxRunRatio: number | null;
  screenVoidSuspicious: boolean;
  settledReferenceChangedRatio: number | null;
  settledReferenceClearToFilledRatio: number | null;
  settledReferenceMaxClearToFilledRunRatio: number | null;
  settledReferenceSuspiciousHole: boolean;
  suspiciousHole: boolean;
}

export interface RouteExperienceBenchmarkSummary {
  sampleCount: number;
  moveFrameCount: number;
  settleFrameCount: number;
  incompleteFrameCount: number;
  totalDistanceMeters: number;
  sampleHz: number;
  speedMetersPerSecond: number;
  totalGameplayFrameMs: number;
  totalAccountedFrameMs: number;
  totalUnmeasuredFrameMs: number;
  unmeasuredFrameRatio: number;
  totalDiagnosticsMs: number;
  totalCaptureDiagnosticsMs: number;
  avgGameplayFrameMs: number;
  p95GameplayFrameMs: number;
  maxGameplayFrameMs: number;
  framesOver16_67Ms: number;
  framesOver33_33Ms: number;
  framesOver50Ms: number;
  moveFramesOver50Ms: number;
  settleFramesOver50Ms: number;
  avgMovementMs: number;
  p95MovementMs: number;
  maxMovementMs: number;
  avgMeasuredWorkMs: number;
  p95MeasuredWorkMs: number;
  maxMeasuredWorkMs: number;
  avgUnmeasuredFrameMs: number;
  p95UnmeasuredFrameMs: number;
  maxUnmeasuredFrameMs: number;
  avgStreamMs: number;
  p95StreamMs: number;
  maxStreamMs: number;
  avgMeshMs: number;
  p95MeshMs: number;
  maxMeshMs: number;
  avgLodMs: number;
  p95LodMs: number;
  maxLodMs: number;
  p95LodChunkMs: number;
  maxLodChunkMs: number;
  avgRenderCpuMs: number;
  p95RenderCpuMs: number;
  maxRenderCpuMs: number;
  avgRenderOtherMs: number;
  maxRenderOtherMs: number;
  avgResidentNotReadyNearSamples: number;
  maxResidentNotReadyNearSamples: number;
  avgVisibleGroundUncoveredCount: number;
  maxVisibleGroundUncoveredCount: number;
  avgVisibleGroundResidentNotReadyCount: number;
  maxVisibleGroundResidentNotReadyCount: number;
  maxSurfaceContinuityGapCount: number;
  framesWithVisibleGroundGaps: number;
  framesWithSurfaceContinuityGaps: number;
  framesWithFarLodCoverageGaps: number;
  framesWithSeamGaps: number;
  framesWithBlockingSeamGaps: number;
  framesWithTransitionSeamGaps: number;
  framesWithLodOverlaps: number;
  maxSeamGapMeters: number;
  maxSurfaceContinuityStepMeters: number;
  maxFarLodCoverageGapMeters: number;
  maxLodOverlapMeters: number;
  screenVoidCaptureCount: number;
  framesWithScreenVoidSignals: number;
  framesWithSettledReferenceHoleSignals: number;
  framesWithHoleSignals: number;
  maxScreenVoidRatio: number;
  maxSettledReferenceChangedRatio: number;
  maxSettledReferenceClearToFilledRatio: number;
  maxSettledReferenceClearToFilledRunRatio: number;
  maxPendingChunks: number;
  maxPendingMeshJobs: number;
  maxDirtyResidentChunks: number;
  maxDirtyMeshlessResidentChunks: number;
  maxDirtyRetainedMeshResidentChunks: number;
  settleFramesUntilComplete: number | null;
}

export interface RouteExperienceBenchmark {
  seed: number;
  radiusChunks: number;
  captureStrideFrames: number;
  seamProbeStrideFrames: number;
  referenceDiffStrideFrames: number;
  referenceDiffLimit: number;
  durationSeconds: number;
  settleSeconds: number;
  totalDistanceMeters: number;
  sampleHz: number;
  speedMetersPerSecond: number;
  samples: RouteExperienceFrameSample[];
  summary: RouteExperienceBenchmarkSummary;
}

export interface BootstrapExperienceBenchmark {
  completed: boolean;
  startedAtMs: number;
  samples: BootstrapBenchmarkSample[];
  summary: BootstrapBenchmarkSummary;
}

export interface BenchmarkWorldPumpOptions {
  maxFrames?: number;
  maxGenerateLodChunks?: number;
  maxAdoptCompletedLodChunks?: number;
  maxLodPlanMs?: number;
  maxLodWorkMs?: number;
  maxEvictChunks?: number;
  maxResidencyPlanMs?: number;
  maxMeshRebuilds?: number;
  stopWhenSettled?: boolean;
}

export interface BenchmarkWorldPumpSummary {
  frameCount: number;
  settled: boolean;
  elapsedMs: number;
  totalGenerated: number;
  totalGeneratedByLevel: readonly number[];
  totalMemoryCacheHits: number;
  totalMemoryCacheHitsByLevel: readonly number[];
  totalEmptyCacheHits: number;
  totalEmptyCacheHitsByLevel: readonly number[];
  totalDiskCacheHits: number;
  totalDiskCacheHitsByLevel: readonly number[];
  totalDiskCacheMisses: number;
  totalWorkerGenerated: number;
  totalWorkerGeneratedByLevel: readonly number[];
  totalScheduledWorkerRequests: number;
  totalScheduledDiskRequests: number;
  totalScheduledDiskStores: number;
  totalCompletedDiskStores: number;
  totalDownsampleMs: number;
  totalMeshMs: number;
  maxLodChunkMs: number;
  maxWorstRecentFrameMs: number;
  maxRecentHitchCount: number;
  maxRecentDroppedFrameEstimate: number;
  finalSnapshot: GameHudSnapshot;
}

interface GameControllerOptions {
  eagerBootstrapBenchmark?: boolean;
}

interface CapturedBenchmarkFrame {
  sampleIndex: number;
  target: Pick<RouteBenchmarkFrameTarget, "frame" | "simTimeSeconds" | "distanceMeters" | "feetPosition" | "yaw" | "pitch">
    & { phase: "move" | "settle" };
  image: {
    width: number;
    height: number;
    pixels: Uint8ClampedArray;
  };
}

export class GameController {
  readonly canvas: HTMLCanvasElement;
  readonly generator = new ProceduralWorldGenerator(1337);
  readonly asyncChunkGeneration = createAsyncProceduralChunkGeneration(this.generator);
  readonly world = new ProceduralResidentWorld(this.generator, {
    asyncChunkGeneration: this.asyncChunkGeneration,
  });
  readonly asyncChunkMeshing = createAsyncChunkMeshing(
    createMeshMaterialLut(this.world.palette, (materialIndex) => this.world.isWaterMaterial(materialIndex)),
  );
  readonly explorationJournal = new ExplorationJournal();
  readonly explorationEventLog = new ExplorationEventLog();
  readonly skillJournal = new SkillJournal();
  readonly routeJournal = new RouteJournal(TRAVEL_GOALS);

  renderer: WebGpuVoxelRenderer | null = null;
  camera: FirstPersonCameraState = createFirstPersonCamera([0.5, 1500, 0.5]);
  player: PlayerState = createPlayerState([0.5, 1500 - PLAYER_EYE_HEIGHT, 0.5]);
  meshMs = 0;
  drawCalls = 0;
  triangles = 0;
  lastFrameCpuMs = 0;
  lastFrameWallMs = 0;
  lastGameplayFrameMs = 0;
  lastFrameLodMs = 0;
  avgFrameWallMs = 0;
  avgFrameCpuMs = 0;
  private readonly frameTimingBuckets = new FrameTimingBuckets(125, 50, 96);
  private lastCompletedFrameAttribution: GameFrameAttribution = createZeroFrameAttribution();
  private lastHitchAttribution: GameFrameAttribution = createZeroFrameAttribution();
  status = "Booting";
  pointerLocked = false;
  onHudUpdate: ((snapshot: GameHudSnapshot) => void) | null = null;
  private readonly pointerLockTargets = new Set<HTMLElement>();
  private pointerLockFallbackActive = false;
  private lastMeshBuildSummary: MeshBuildSummary = {
    meshCount: 0,
    newMeshCount: 0,
    remeshCount: 0,
    triangleCount: 0,
    elapsedMs: 0,
  };
  private lastRenderStats: RenderStats = zeroRenderStats();
  private lastLodSummary: LodResidencyUpdateSummary = {
    generated: 0,
    generatedByLevel: [0, 0, 0, 0, 0],
    cacheHits: 0,
    cacheHitsByLevel: [0, 0, 0, 0, 0],
    emptyCacheHits: 0,
    emptyCacheHitsByLevel: [0, 0, 0, 0, 0],
    pending: 0,
    pendingPlanning: 0,
    pendingDiskCache: 0,
    pendingDiskCacheByLevel: [0, 0, 0, 0, 0],
    pendingGenerationBudget: 0,
    pendingGenerationBudgetByLevel: [0, 0, 0, 0, 0],
    pendingPartialBuild: 0,
    pendingPartialBuildByLevel: [0, 0, 0, 0, 0],
    pendingPrepared: 0,
    pendingPreparedByLevel: [0, 0, 0, 0, 0],
    pendingInvalidatedEviction: 0,
    totalChunks: 0,
    totalChunksByLevel: [0, 0, 0, 0, 0],
    cachedChunks: 0,
    cachedEmptyKeys: 0,
    elapsedMs: 0,
    yRangeMs: 0,
    downsampleMs: 0,
    meshMs: 0,
    commitMs: 0,
    maxChunkMs: 0,
    maxChunkLevel: 0,
    maxChunkKey: null,
    neededKeyCount: 0,
    neededKeyCacheHit: false,
    scheduledRegionSummaryRequests: 0,
    lodDiskCacheHits: 0,
    lodDiskCacheHitsByLevel: [0, 0, 0, 0, 0],
    lodDiskCacheMisses: 0,
    lodWorkerGenerated: 0,
    lodWorkerGeneratedByLevel: [0, 0, 0, 0, 0],
    scheduledLodWorkerRequests: 0,
    scheduledLodDiskRequests: 0,
    scheduledLodDiskStores: 0,
    completedLodDiskStores: 0,
  };
  private cumulativeLodGeneratedChunks = 0;
  private cumulativeLodWorkerGenerated = 0;
  private cumulativeLodDiskCacheHits = 0;
  private cumulativeLodDiskCacheMisses = 0;
  private cumulativeLodScheduledDiskRequests = 0;
  private cumulativeLodScheduledDiskStores = 0;
  private cumulativeLodCompletedDiskStores = 0;
  private lastStreamSummary: ResidencyUpdateSummary = cloneResidencySummary(this.world.lastResidency);
  private streamAnchor: StreamAnchor | null = null;
  private streamingBudgets: StreamingBudgets = {
    maxGeneratedChunksPerUpdate: DEFAULT_MAX_GENERATED_CHUNKS_PER_UPDATE,
    maxMeshRebuildsPerFrame: DEFAULT_MAX_MESH_REBUILDS_PER_FRAME,
  };

  private rafId = 0;
  private lastFrameTime = 0;
  private lastHudPushAt = 0;
  private interactiveFrameNumber = 0;
  private readonly pressedKeys = new Set<string>();
  private lastDiscoverySampleAt = 0;
  private lastDiscoverySampleFeetPosition: Vec3 | null = null;
  private lastDiscoverySnapshot: ExplorationJournalSnapshot = this.explorationJournal.getSnapshot();
  private lastTravelContextSampleAt = 0;
  private lastTravelContextFeetPosition: Vec3 | null = null;
  private lastTravelContext: "surface" | "underground" = "surface";
  private lastInteractionLabel = "No interaction yet";
  private caveReturnFeetPosition: Vec3 | null = null;
  private readonly bootstrapBenchmarkStartedAt = performance.now();
  private readonly worldClockStartedAt = performance.now();
  private readonly bootstrapBenchmarkSamples: BootstrapBenchmarkSample[] = [];
  private bootstrapPlayableReady = false;
  private bootstrapBenchmarkComplete = false;
  private readonly eagerBootstrapBenchmark: boolean;

  constructor(canvas: HTMLCanvasElement, options: GameControllerOptions = {}) {
    this.canvas = canvas;
    this.eagerBootstrapBenchmark = options.eagerBootstrapBenchmark ?? false;
    this.pointerLockTargets.add(canvas);
    this.routeJournal.startGoal(DEFAULT_TRAVEL_GOAL_ID);
  }

  registerPointerLockTarget(target: HTMLElement): () => void {
    this.pointerLockTargets.add(target);
    return () => {
      this.pointerLockTargets.delete(target);
    };
  }

  async init(): Promise<void> {
    this.renderer = await WebGpuVoxelRenderer.create(this.canvas);
    this.loadBootstrapWorld();
    this.attachInteractions();
    if (this.eagerBootstrapBenchmark) {
      await this.drainBootstrapBenchmark();
    }
    this.start();
  }

  dispose(): void {
    cancelAnimationFrame(this.rafId);
    if (this.ownsPointerLockElement(document.pointerLockElement)) {
      document.exitPointerLock();
    }
    this.pointerLocked = false;
    this.pointerLockFallbackActive = false;
    this.pressedKeys.clear();
    this.asyncChunkGeneration?.dispose();
    this.asyncChunkMeshing?.dispose();
    this.renderer?.dispose();
    this.canvas.removeEventListener("click", this.handleCanvasClick);
    document.removeEventListener("contextmenu", this.handleDocumentContextMenu);
    document.removeEventListener("pointerlockchange", this.handlePointerLockChange);
    document.removeEventListener("mousemove", this.handleMouseMove);
    window.removeEventListener("keydown", this.handleKeyDown);
    window.removeEventListener("keyup", this.handleKeyUp);
    window.removeEventListener("blur", this.handleBlur);
    document.removeEventListener("visibilitychange", this.handleVisibilityChange);
  }

  async requestPointerLock(target: HTMLElement = this.canvas): Promise<void> {
    try {
      target.focus({ preventScroll: true });
      await target.requestPointerLock();
    } catch (error) {
      this.activatePointerLockFallback(error);
    }
  }

  start(): void {
    cancelAnimationFrame(this.rafId);
    const tick = (now: number) => {
      this.advanceInteractiveFrame(now);
      this.rafId = requestAnimationFrame(tick);
    };
    this.rafId = requestAnimationFrame(tick);
  }

  stop(): void {
    cancelAnimationFrame(this.rafId);
  }

  getDebugSnapshot(): GameHudSnapshot {
    const stats = this.world.getStats();
    const currentWorld = this.sampleCurrentWorldContext();
    const discovery = this.refreshDiscoveryJournal();
    const skills = this.skillJournal.observeDiscoveries(this.explorationJournal.drainPendingSkillDiscoveries());
    const explorationSkillEffects = resolveExplorationSkillEffects(skills);
    const routeSnapshot = this.routeJournal.getSnapshot();
    const encounter = sampleRpgEncounterWorldUnits(this.player.feetPosition[0], this.player.feetPosition[2]);
    const worldSystems = this.sampleWorldSystems(currentWorld, encounter);
    const primaryFaction = encounter.factionHints[0]?.factionId ?? null;
    const scoutResult = describeRpgEncounterScoutResult(encounter);
    const eventLog = this.explorationEventLog.getSnapshot();
    const fieldKit = summarizeFieldKit(eventLog);
    const lootJournal = summarizeLootJournal(eventLog);
    const bestiary = summarizeBestiary(eventLog);
    const passiveMobSightings = this.samplePassiveMobSightings();
    const nearestPassiveMob = passiveMobSightings[0] ?? null;
    const activeExplorationHud = this.buildActiveExplorationHudState(currentWorld, discovery, routeSnapshot, encounter);
    const activeQuest = this.selectActiveQuestHook(currentWorld, discovery, routeSnapshot, encounter, primaryFaction);
    const bootstrap = this.getBootstrapReadiness();
    const ambientProfile = currentWorld.ambientProfile;
    const travelContextLabel = this.lastTravelContext === "underground" ? "Underground route" : "Surface route";
    return {
      status: this.status,
      pointerLocked: this.pointerLocked,
      position: [...this.camera.position],
      feetPosition: [...this.player.feetPosition] as Vec3,
      playerChunk: [
        Math.floor(this.player.feetPosition[0] / this.world.chunkSize),
        Math.floor(this.player.feetPosition[1] / this.world.chunkSize),
        Math.floor(this.player.feetPosition[2] / this.world.chunkSize),
      ],
      streamAnchorChunk: this.streamAnchor
        ? [this.streamAnchor.chunkX, this.streamAnchor.chunkZ]
        : [
            Math.floor(this.player.feetPosition[0] / this.world.chunkSize),
            Math.floor(this.player.feetPosition[2] / this.world.chunkSize),
          ],
      grounded: this.player.grounded,
      bodyInWater: this.player.bodyInWater,
      eyeInWater: this.player.eyeInWater,
      yawDegrees: toDegrees(this.camera.yaw),
      pitchDegrees: toDegrees(this.camera.pitch),
      solidVoxelCount: stats.solidVoxelCount,
      chunkCount: stats.chunkCount,
      paletteCount: stats.paletteCount,
      streamMs: this.lastStreamSummary.elapsedMs,
      streamGeneratedChunks: this.lastStreamSummary.generatedChunks,
      streamEvictedChunks: this.lastStreamSummary.evictedChunks,
      streamPendingChunks: this.lastStreamSummary.pendingChunks,
      streamEmptyChunksSkipped: this.lastStreamSummary.emptyChunksSkipped,
      streamCachedEmptyChunkHits: this.lastStreamSummary.cachedEmptyChunkHits,
      streamCompletedChunkCacheHits: this.lastStreamSummary.phaseMs.completedChunkCacheHits,
      streamCompletedGeneratedChunks: this.lastStreamSummary.phaseMs.completedGeneratedChunks,
      generationWorkerCount: this.asyncChunkGeneration?.getWorkerCount?.() ?? 0,
      streamCompletedSummaryCacheHits: this.lastStreamSummary.phaseMs.completedSummaryCacheHits,
      streamCompletedGeneratedSummaries: this.lastStreamSummary.phaseMs.completedGeneratedSummaries,
      streamCompletedRegionSummaryCacheHits: this.lastStreamSummary.phaseMs.completedRegionSummaryCacheHits,
      streamMissingRegionSummaries: this.lastStreamSummary.phaseMs.missingRegionSummaries,
      streamDirtyResidentChunks: this.world.countDirtyResidentChunks(),
      residencyRadiusChunks: this.lastStreamSummary.radiusChunks,
      surfaceY: currentWorld.probe.surfaceY,
      worldRegionId: currentWorld.probe.regionId ?? null,
      worldRegionStrength: currentWorld.probe.regionStrength ?? 0,
      biomeId: currentWorld.probe.biomeId,
      undergroundBiomeId: currentWorld.observedUndergroundBiomeId,
      regionalVariantId: currentWorld.probe.regionalVariantId,
      landmarkId: currentWorld.probe.landmarkId ?? discovery.currentLandmarkId,
      ambientProfileId: ambientProfile.id,
      ambientProfileLabel: ambientProfile.label,
      ambientFogEndMeters: worldUnitsToMeters(ambientProfile.fogEndDistance),
      timeOfDayLabel: worldSystems.clock.phaseLabel,
      worldClockLabel: worldSystems.clock.clockLabel,
      worldDay: worldSystems.clock.day,
      worldDaylight: worldSystems.clock.daylight,
      weatherLabel: worldSystems.weather.label,
      weatherIntensity: worldSystems.weather.intensity,
      floraLabel: worldSystems.area.floraLabel,
      faunaLabel: worldSystems.area.faunaLabel,
      lootSignalLabel: worldSystems.area.lootSignalLabel,
      hazardLabel: worldSystems.area.hazardLabel,
      areaCoherenceLabel: worldSystems.area.coherenceLabel,
      encounterMoodLabel: formatEncounterMoodForTravelContext(
        describeRpgEncounterMood(encounter.moodId),
        this.lastTravelContext,
      ),
      encounterPressureLabel: scoutResult.pressureLabel,
      encounterFactionLabel: primaryFaction ? describeRpgEncounterFaction(primaryFaction) : "No dominant faction",
      encounterFlavorLabel: encounter.flavorTags.slice(0, 2).map(formatEncounterFlavorTag).join(" • "),
      passiveMobSightingCount: passiveMobSightings.length,
      passiveMobNearestId: nearestPassiveMob?.id ?? null,
      passiveMobNearestLabel: formatPassiveMobPresenceLabel(nearestPassiveMob),
      passiveMobNearestDetailLabel: nearestPassiveMob?.label ?? "No passive mob nearby",
      passiveMobNearestDistanceMeters: nearestPassiveMob ? worldUnitsToMeters(nearestPassiveMob.distanceWorldUnits) : null,
      passiveMobNearestFactionLabel: nearestPassiveMob?.factionName ?? "No nearby faction",
      passiveMobNearestMoodLabel: nearestPassiveMob?.moodName ?? "No nearby mob mood",
      bestiarySightingCount: bestiary.totalSightings,
      bestiaryEntryCount: bestiary.entryCount,
      bestiarySummaryLabel: bestiary.summaryLabel,
      bestiaryLastSightingLabel: bestiary.lastSightingLabel,
      bestiaryLastNoteLabel: bestiary.lastFieldNoteLabel,
      bestiaryDominantFactionLabel: bestiary.dominantFactionLabel,
      activePlaceName: activeExplorationHud.activePlaceName,
      activeRouteName: activeExplorationHud.activeRouteName,
      activeRouteProgressLabel: activeExplorationHud.activeRouteProgressLabel,
      activeTravelGoalTitle: activeExplorationHud.activeTravelGoalTitle,
      activeTravelGoalStepLabel: activeExplorationHud.activeTravelGoalStepLabel,
      activeTravelGoalProgressRatio: activeExplorationHud.activeTravelGoalProgressRatio,
      activeQuestHookId: activeQuest?.hookId ?? null,
      activeQuestHookKind: activeQuest?.kind ?? null,
      activeQuestObjectiveKind: activeQuest?.objectiveKind ?? null,
      activeQuestTitle: activeQuest?.title ?? "No local rumor",
      activeQuestObjectiveLabel: activeQuest?.objectiveLabel ?? "Keep moving",
      activeQuestRumorText: activeQuest?.rumorText ?? "No local rumor is strong enough to follow yet.",
      activeQuestMoodLabel: activeQuest?.mood ?? "No quest mood",
      activeQuestFactionLabel: activeQuest?.faction ?? "No quest faction",
      travelContext: this.lastTravelContext,
      travelContextLabel,
      interactionTargetName: activeExplorationHud.interactionTargetName,
      interactionPromptLabel: activeExplorationHud.interactionPromptLabel,
      interactionPromptDescription: activeExplorationHud.interactionPromptDescription,
      interactionPromptVerb: activeExplorationHud.interactionPromptVerb,
      navigationTargetId: activeExplorationHud.navigationTargetId,
      navigationTargetName: activeExplorationHud.navigationTargetName,
      navigationSource: activeExplorationHud.navigationSource,
      navigationDistanceMeters: activeExplorationHud.navigationDistanceMeters,
      navigationBearingLabel: activeExplorationHud.navigationBearingLabel,
      navigationDistanceLabel: activeExplorationHud.navigationDistanceLabel,
      navigationCompassLabel: activeExplorationHud.navigationCompassLabel,
      navigationTurnLabel: activeExplorationHud.navigationTurnLabel,
      lastInteractionLabel: this.lastInteractionLabel,
      discoveredBiomeCount: discovery.discoveredBiomeIds.length,
      discoveredUndergroundBiomeCount: discovery.discoveredUndergroundBiomeIds.length,
      discoveredRegionalVariantCount: discovery.discoveredRegionalVariantIds.length,
      discoveredLandmarkCount: discovery.discoveredLandmarkIds.length,
      discoveredAncientLandmarkCount: discovery.discoveredLandmarkIds
        .filter((landmarkId) => ANCIENT_LANDMARK_IDS.has(landmarkId))
        .length,
      scoutedMobTrailCount: countMobSignEvents(eventLog),
      lootedCacheCount: countEventsBySubjectRole(eventLog, "object", "loot-cache"),
      scoutedCaveMouthCount: countEventsBySubjectRole(eventLog, "zone", "cave-mouth"),
      fieldKitFindCount: fieldKit.totalFinds,
      fieldKitSummaryLabel: fieldKit.summaryLabel,
      fieldKitLastFindLabel: fieldKit.lastFindLabel,
      fieldKitLastNoteLabel: fieldKit.lastFieldNoteLabel,
      fieldKitDominantCategoryLabel: fieldKit.dominantCategoryLabel,
      lootJournalCollectedCacheCount: lootJournal.totalCollectedCaches,
      lootJournalRevisitedCacheCount: lootJournal.totalRevisitedCaches,
      lootJournalRevisitEventCount: lootJournal.totalRevisitEvents,
      lootJournalStateLabel: formatLootJournalStateLabel(lootJournal.totalCollectedCaches, lootJournal.totalRevisitedCaches),
      landmarkScanRadiusMeters: explorationSkillEffects.landmarkScanRadiusMeters,
      landmarkScanSampleCount: buildLandmarkSampleOffsets(explorationSkillEffects).length,
      surfaceTravelSpeedMultiplier: explorationSkillEffects.surfaceTravelSpeedMultiplier,
      undergroundTravelSpeedMultiplier: explorationSkillEffects.undergroundTravelSpeedMultiplier,
      recentDiscoveries: discovery.recentDiscoveries.map((event) => ({ ...event })),
      lastDiscoveryLabel: discovery.lastDiscovery?.label ?? "None",
      focusSkillName: skills.focusSkill.name,
      focusSkillLevel: skills.focusSkill.level,
      focusSkillProgressRatio: skills.focusSkill.progressRatio,
      totalSkillTravelMeters: skills.travelMeters,
      totalSkillLevel: skills.totalLevel,
      bootstrapPlayableReady: bootstrap.playableReady,
      bootstrapVisualReady: bootstrap.visualReady,
      bootstrapElapsedMs: performance.now() - this.bootstrapBenchmarkStartedAt,
      bootstrapRequiredColumns: bootstrap.requiredColumns,
      bootstrapReadyColumns: bootstrap.readyColumns,
      bootstrapUrgentDirtyMeshlessChunks: bootstrap.urgentDirtyMeshlessChunks,
      bootstrapPendingMeshJobs: bootstrap.pendingMeshJobs,
      meshMs: this.meshMs,
      meshNewChunks: this.lastMeshBuildSummary.newMeshCount,
      meshRemeshChunks: this.lastMeshBuildSummary.remeshCount,
      drawCalls: this.drawCalls,
      triangles: this.triangles,
      lastFrameWallMs: this.lastFrameWallMs,
      frameTiming: this.frameTimingBuckets.snapshot(),
      lastHitchAttribution: { ...this.lastHitchAttribution },
      lastGameplayFrameMs: this.lastGameplayFrameMs,
      lastFrameCpuMs: this.lastFrameCpuMs,
      avgFrameWallMs: this.avgFrameWallMs,
      lastFrameSyncMs: this.lastRenderStats.syncResourcesMs,
      lastFrameUploadMs: this.lastRenderStats.uploadMs,
      lastFrameUploadChunks: this.lastRenderStats.uploadChunks,
      lastFrameUploadBytes: this.lastRenderStats.uploadBytes,
      lastFrameEncodeMs: this.lastRenderStats.encodeMs,
      avgFrameCpuMs: this.avgFrameCpuMs,
      maxGeneratedChunksPerUpdate: this.streamingBudgets.maxGeneratedChunksPerUpdate,
      maxMeshRebuildsPerFrame: this.streamingBudgets.maxMeshRebuildsPerFrame,
      lodChunkCount: this.lastLodSummary.totalChunks,
      lodChunkCountByLevel: [...this.lastLodSummary.totalChunksByLevel],
      lodPendingChunks: this.lastLodSummary.pending,
      lodPendingPlanning: this.lastLodSummary.pendingPlanning,
      lodPendingDiskCache: this.lastLodSummary.pendingDiskCache,
      lodPendingDiskCacheByLevel: [...this.lastLodSummary.pendingDiskCacheByLevel],
      lodPendingGenerationBudget: this.lastLodSummary.pendingGenerationBudget,
      lodPendingGenerationBudgetByLevel: [...this.lastLodSummary.pendingGenerationBudgetByLevel],
      lodPendingPartialBuild: this.lastLodSummary.pendingPartialBuild,
      lodPendingPartialBuildByLevel: [...this.lastLodSummary.pendingPartialBuildByLevel],
      lodPendingPrepared: this.lastLodSummary.pendingPrepared,
      lodPendingPreparedByLevel: [...this.lastLodSummary.pendingPreparedByLevel],
      lodPendingInvalidatedEviction: this.lastLodSummary.pendingInvalidatedEviction,
      lodGeneratedChunks: this.lastLodSummary.generated,
      lodGeneratedChunksByLevel: [...this.lastLodSummary.generatedByLevel],
      lodCacheHits: this.lastLodSummary.cacheHits,
      lodCacheHitsByLevel: [...this.lastLodSummary.cacheHitsByLevel],
      lodEmptyCacheHits: this.lastLodSummary.emptyCacheHits,
      lodEmptyCacheHitsByLevel: [...this.lastLodSummary.emptyCacheHitsByLevel],
      lodCachedChunks: this.lastLodSummary.cachedChunks,
      lodCachedEmptyKeys: this.lastLodSummary.cachedEmptyKeys,
      lodElapsedMs: this.lastLodSummary.elapsedMs,
      lodYRangeMs: this.lastLodSummary.yRangeMs,
      lodDownsampleMs: this.lastLodSummary.downsampleMs,
      lodMeshMs: this.lastLodSummary.meshMs,
      lodCommitMs: this.lastLodSummary.commitMs,
      lodMaxChunkMs: this.lastLodSummary.maxChunkMs,
      lodMaxChunkLevel: this.lastLodSummary.maxChunkLevel,
      lodMaxChunkKey: this.lastLodSummary.maxChunkKey,
      lodNeededKeyCount: this.lastLodSummary.neededKeyCount,
      lodNeededKeyCacheHit: this.lastLodSummary.neededKeyCacheHit,
      lodScheduledRegionSummaryRequests: this.lastLodSummary.scheduledRegionSummaryRequests,
      lodDiskCacheHits: this.lastLodSummary.lodDiskCacheHits,
      lodDiskCacheHitsByLevel: [...this.lastLodSummary.lodDiskCacheHitsByLevel],
      lodDiskCacheMisses: this.lastLodSummary.lodDiskCacheMisses,
      lodWorkerGenerated: this.lastLodSummary.lodWorkerGenerated,
      lodWorkerGeneratedByLevel: [...this.lastLodSummary.lodWorkerGeneratedByLevel],
      lodScheduledWorkerRequests: this.lastLodSummary.scheduledLodWorkerRequests,
      lodScheduledDiskRequests: this.lastLodSummary.scheduledLodDiskRequests,
      lodScheduledDiskStores: this.lastLodSummary.scheduledLodDiskStores,
      lodCompletedDiskStores: this.lastLodSummary.completedLodDiskStores,
      cumulativeLodGeneratedChunks: this.cumulativeLodGeneratedChunks,
      cumulativeLodWorkerGenerated: this.cumulativeLodWorkerGenerated,
      cumulativeLodDiskCacheHits: this.cumulativeLodDiskCacheHits,
      cumulativeLodDiskCacheMisses: this.cumulativeLodDiskCacheMisses,
      cumulativeLodScheduledDiskRequests: this.cumulativeLodScheduledDiskRequests,
      cumulativeLodScheduledDiskStores: this.cumulativeLodScheduledDiskStores,
      cumulativeLodCompletedDiskStores: this.cumulativeLodCompletedDiskStores,
      lodDrawCalls: this.lastRenderStats.lodDrawCalls,
      lodDrawCallsByLevel: [...this.lastRenderStats.lodDrawCallsByLevel],
      frustumCulledChunks: this.lastRenderStats.frustumCulledChunks,
      fogCulledChunks: this.lastRenderStats.fogCulledChunks,
    };
  }

  getBootstrapBenchmark(): BootstrapExperienceBenchmark {
    const samples = this.bootstrapBenchmarkSamples.map((sample) => ({ ...sample }));
    return {
      completed: this.bootstrapBenchmarkComplete,
      startedAtMs: this.bootstrapBenchmarkStartedAt,
      samples,
      summary: summarizeBootstrapBenchmark(samples),
    };
  }

  getStreamingBudgets(): StreamingBudgets {
    return { ...this.streamingBudgets };
  }

  getEditLogSnapshot(): WorldEditRecord[] {
    return this.world.getEditLogSnapshot();
  }

  setStreamingBudgets(
    maxGeneratedChunksPerUpdate: number,
    maxMeshRebuildsPerFrame: number,
  ): StreamingBudgets {
    this.streamingBudgets = {
      maxGeneratedChunksPerUpdate: clampPositiveInt(
        maxGeneratedChunksPerUpdate,
        DEFAULT_MAX_GENERATED_CHUNKS_PER_UPDATE,
      ),
      maxMeshRebuildsPerFrame: clampPositiveInt(
        maxMeshRebuildsPerFrame,
        DEFAULT_MAX_MESH_REBUILDS_PER_FRAME,
      ),
    };
    this.pushHud(true);
    return this.getStreamingBudgets();
  }

  teleport(position: Vec3): void {
    teleportPlayerToEyePosition(this.player, position);
    this.syncCameraToPlayer();
    this.syncWorldAroundPlayer();
    this.pushHud(true);
  }

  setResidencyRadiusChunks(radius: number): void {
    this.world.setHorizontalRadiusChunks(radius);
    this.syncWorldAroundPlayer(true);
    this.pushHud(true);
  }

  forceResidencyUpdate(): ResidencyUpdateSummary {
    this.world.setHorizontalRadiusChunks(this.world.horizontalRadiusChunks);
    this.syncWorldAroundPlayer(true);
    this.pushHud(true);
    return cloneResidencySummary(this.lastStreamSummary);
  }

  snapshotResidentWorld(): ResidentWorldProbeSnapshot {
    return summarizeResidentWorld(this.world);
  }

  getDiscoveryJournalSnapshot(): ExplorationJournalSnapshot {
    return this.refreshDiscoveryJournal(true);
  }

  getSkillJournalSnapshot(): SkillJournalSnapshot {
    const discovery = this.refreshDiscoveryJournal(true);
    void discovery;
    return this.skillJournal.observeDiscoveries(this.explorationJournal.drainPendingSkillDiscoveries());
  }

  getExplorationEventLogSnapshot(): ExplorationEventLogSnapshot {
    return this.explorationEventLog.getSnapshot();
  }

  resetDiscoveryJournal(): ExplorationJournalSnapshot {
    this.explorationJournal.reset();
    this.explorationEventLog.reset();
    this.skillJournal.reset();
    this.lastDiscoverySampleFeetPosition = null;
    this.lastDiscoverySampleAt = 0;
    const snapshot = this.refreshDiscoveryJournal(true);
    this.pushHud(true);
    return snapshot;
  }

  exportProgressState(): ProgressStateSnapshot {
    this.refreshDiscoveryJournal(true);
    this.skillJournal.observeDiscoveries(this.explorationJournal.drainPendingSkillDiscoveries());
    return {
      version: 1,
      discovery: this.explorationJournal.exportState(),
      events: this.explorationEventLog.exportState(),
      skills: this.skillJournal.exportState(),
      routes: this.routeJournal.exportState(),
    };
  }

  importProgressState(state: Partial<ProgressStateSnapshot>): ProgressStateSnapshot {
    this.explorationJournal.importState(state.discovery ?? {});
    this.explorationEventLog.importState(state.events ?? {});
    this.skillJournal.importState(state.skills ?? {});
    this.routeJournal.importState(state.routes ?? {});
    this.routeJournal.startGoal(DEFAULT_TRAVEL_GOAL_ID);
    this.lastDiscoverySampleFeetPosition = null;
    this.lastDiscoverySampleAt = 0;
    this.refreshDiscoveryJournal(true);
    this.skillJournal.observeDiscoveries(this.explorationJournal.drainPendingSkillDiscoveries());
    this.pushHud(true);
    return this.exportProgressState();
  }

  probeRenderReadyCoverage(sampleRadiusMeters = 12, sampleStepMeters = 0.8): RenderReadyCoverageProbe {
    const normalizedRadius = Math.max(1, sampleRadiusMeters);
    const normalizedStep = Math.max(0.1, sampleStepMeters);
    const sampleRadiusWorldUnits = metersToWorldUnits(normalizedRadius);
    const sampleStepWorldUnits = metersToWorldUnits(normalizedStep);
    const centerX = this.player.feetPosition[0];
    const centerZ = this.player.feetPosition[2];
    const lodSpans = this.collectRenderableLodCoverageSpans();
    let sampleCount = 0;
    let residentSampleCount = 0;
    let renderReadySampleCount = 0;
    let residentNotReadyCount = 0;

    for (let offsetZ = -sampleRadiusWorldUnits; offsetZ <= sampleRadiusWorldUnits; offsetZ += sampleStepWorldUnits) {
      for (let offsetX = -sampleRadiusWorldUnits; offsetX <= sampleRadiusWorldUnits; offsetX += sampleStepWorldUnits) {
        const worldX = centerX + offsetX;
        const worldZ = centerZ + offsetZ;
        const chunkX = Math.floor(worldX / this.world.chunkSize);
        const chunkZ = Math.floor(worldZ / this.world.chunkSize);
        const groundReadiness = this.resolveVisibleGroundReadiness(worldX, worldZ, chunkX, chunkZ);
        const lodCovered = isCoveredByLodSpans(worldX, worldZ, lodSpans);
        const resident = groundReadiness.resident || lodCovered;
        const renderReady = groundReadiness.renderReady || lodCovered;
        sampleCount += 1;
        if (resident) {
          residentSampleCount += 1;
        }
        if (renderReady) {
          renderReadySampleCount += 1;
        }
        if (resident && !renderReady) {
          residentNotReadyCount += 1;
        }
      }
    }

    return {
      center: [...this.player.feetPosition],
      sampleRadiusMeters: normalizedRadius,
      sampleStepMeters: normalizedStep,
      sampleCount,
      residentSampleCount,
      renderReadySampleCount,
      residentNotReadyCount,
      missingResidentCount: sampleCount - residentSampleCount,
    };
  }

  probeVisibleGroundCoverage(
    sampleForwardMeters = 16,
    sampleLateralMeters = 6,
    sampleStepMeters = 0.8,
  ): VisibleGroundCoverageProbe {
    const normalizedForward = Math.max(1, sampleForwardMeters);
    const normalizedLateral = Math.max(1, sampleLateralMeters);
    const normalizedStep = Math.max(0.1, sampleStepMeters);
    const forwardAxis = [Math.cos(this.camera.yaw), Math.sin(this.camera.yaw)] as const;
    const rightAxis = [-forwardAxis[1], forwardAxis[0]] as const;
    const forwardWorldUnits = metersToWorldUnits(normalizedForward);
    const lateralWorldUnits = metersToWorldUnits(normalizedLateral);
    const stepWorldUnits = metersToWorldUnits(normalizedStep);
    const lodSpans = this.collectRenderableLodCoverageSpans();
    const uncoveredSamples: VisibleGroundCoverageIssueSample[] = [];
    let sampleCount = 0;
    let renderReadyCount = 0;
    let uncoveredCount = 0;
    let residentNotReadyCount = 0;

    for (let forward = stepWorldUnits; forward <= forwardWorldUnits; forward += stepWorldUnits) {
      for (let lateral = -lateralWorldUnits; lateral <= lateralWorldUnits; lateral += stepWorldUnits) {
        const worldX = this.player.feetPosition[0] + forwardAxis[0] * forward + rightAxis[0] * lateral;
        const worldZ = this.player.feetPosition[2] + forwardAxis[1] * forward + rightAxis[1] * lateral;
        const chunkX = Math.floor(worldX / this.world.chunkSize);
        const chunkZ = Math.floor(worldZ / this.world.chunkSize);
        const groundReadiness = this.resolveVisibleGroundReadiness(worldX, worldZ, chunkX, chunkZ);
        const lodCovered = isCoveredByLodSpans(worldX, worldZ, lodSpans);
        const resident = groundReadiness.resident || lodCovered;
        const renderReady = groundReadiness.renderReady || lodCovered;
        sampleCount += 1;
        if (renderReady) {
          renderReadyCount += 1;
        } else {
          uncoveredCount += 1;
          pushVisibleGroundIssueSample(uncoveredSamples, {
            worldX,
            worldZ,
            forwardMeters: worldUnitsToMeters(forward),
            lateralMeters: worldUnitsToMeters(lateral),
            resident,
            renderReady,
          });
        }
        if (resident && !renderReady) {
          residentNotReadyCount += 1;
        }
      }
    }

    return {
      center: [...this.player.feetPosition],
      yawRadians: this.camera.yaw,
      sampleForwardMeters: normalizedForward,
      sampleLateralMeters: normalizedLateral,
      sampleStepMeters: normalizedStep,
      sampleCount,
      renderReadyCount,
      uncoveredCount,
      residentNotReadyCount,
      uncoveredSamples,
    };
  }

  probeSurfaceContinuity(
    sampleForwardMeters = 16,
    sampleLateralMeters = 6,
    sampleStepMeters = 0.8,
    maxSmoothStepMeters = 1.2,
  ): SurfaceContinuityProbe {
    const normalizedForward = Math.max(1, sampleForwardMeters);
    const normalizedLateral = Math.max(0.5, sampleLateralMeters);
    const normalizedStep = Math.max(0.1, sampleStepMeters);
    const maxSmoothStepWorldUnits = metersToWorldUnits(Math.max(0.1, maxSmoothStepMeters));
    const forwardWorldUnits = metersToWorldUnits(normalizedForward);
    const lateralWorldUnits = metersToWorldUnits(normalizedLateral);
    const stepWorldUnits = metersToWorldUnits(normalizedStep);
    const center = this.player.feetPosition;
    const yaw = this.camera.yaw;
    const forwardX = Math.cos(yaw);
    const forwardZ = Math.sin(yaw);
    const rightX = Math.cos(yaw + Math.PI * 0.5);
    const rightZ = Math.sin(yaw + Math.PI * 0.5);
    const samples: Array<{
      worldX: number;
      worldZ: number;
      surfaceY: number;
      renderReady: boolean;
    }> = [];
    const forwardCount = Math.floor(forwardWorldUnits / stepWorldUnits) + 1;
    const lateralCount = Math.floor((lateralWorldUnits * 2) / stepWorldUnits) + 1;
    const issueSamples: SurfaceContinuityIssueSample[] = [];
    let sampleCount = 0;
    let edgeCount = 0;
    let smoothEdgeCount = 0;
    let missingSmoothEdgeCount = 0;
    let abruptEdgeCount = 0;
    let maxExpectedStepMeters = 0;

    for (let forwardIndex = 0; forwardIndex < forwardCount; forwardIndex += 1) {
      const forward = forwardIndex * stepWorldUnits;
      for (let lateralIndex = 0; lateralIndex < lateralCount; lateralIndex += 1) {
        const lateral = -lateralWorldUnits + lateralIndex * stepWorldUnits;
        const worldX = center[0] + forwardX * forward + rightX * lateral;
        const worldZ = center[2] + forwardZ * forward + rightZ * lateral;
        const chunkX = Math.floor(worldX / this.world.chunkSize);
        const chunkZ = Math.floor(worldZ / this.world.chunkSize);
        const readiness = this.resolveVisibleGroundReadiness(worldX, worldZ, chunkX, chunkZ);
        samples.push({
          worldX,
          worldZ,
          surfaceY: this.generator.sampleColumn(worldX, worldZ).surfaceY,
          renderReady: readiness.renderReady,
        });
        sampleCount += 1;
      }
    }

    const compareEdge = (
      sample: typeof samples[number],
      neighbor: typeof samples[number],
    ): void => {
      const deltaWorldUnits = Math.abs(sample.surfaceY - neighbor.surfaceY);
      const deltaMeters = worldUnitsToMeters(deltaWorldUnits);
      maxExpectedStepMeters = Math.max(maxExpectedStepMeters, deltaMeters);
      edgeCount += 1;
      if (deltaWorldUnits > maxSmoothStepWorldUnits) {
        abruptEdgeCount += 1;
        return;
      }
      smoothEdgeCount += 1;
      if (!sample.renderReady || !neighbor.renderReady) {
        missingSmoothEdgeCount += 1;
        pushSurfaceContinuityIssueSample(issueSamples, {
          worldX: sample.worldX,
          worldZ: sample.worldZ,
          neighborWorldX: neighbor.worldX,
          neighborWorldZ: neighbor.worldZ,
          deltaMeters,
          renderReady: sample.renderReady,
          neighborRenderReady: neighbor.renderReady,
        });
      }
    };

    for (let forwardIndex = 0; forwardIndex < forwardCount; forwardIndex += 1) {
      for (let lateralIndex = 0; lateralIndex < lateralCount; lateralIndex += 1) {
        const index = forwardIndex * lateralCount + lateralIndex;
        const sample = samples[index]!;
        if (lateralIndex > 0) {
          compareEdge(sample, samples[index - 1]!);
        }
        if (forwardIndex > 0) {
          compareEdge(sample, samples[index - lateralCount]!);
        }
      }
    }

    return {
      center: [...center],
      yawRadians: yaw,
      sampleForwardMeters: normalizedForward,
      sampleLateralMeters: normalizedLateral,
      sampleStepMeters: normalizedStep,
      maxSmoothStepMeters,
      sampleCount,
      edgeCount,
      smoothEdgeCount,
      missingSmoothEdgeCount,
      abruptEdgeCount,
      maxExpectedStepMeters,
      issueSamples,
    };
  }

  private resolveVisibleGroundReadiness(
    worldX: number,
    worldZ: number,
    chunkX = Math.floor(worldX / this.world.chunkSize),
    chunkZ = Math.floor(worldZ / this.world.chunkSize),
  ): { resident: boolean; renderReady: boolean } {
    const surfaceY = this.generator.sampleColumn(worldX, worldZ).surfaceY;
    const centerChunkY = Math.floor(surfaceY / this.world.chunkSize);
    const chunkYs = new Set([
      centerChunkY,
      Math.floor((surfaceY + 1) / this.world.chunkSize),
      Math.floor((surfaceY - 1) / this.world.chunkSize),
    ]);
    let resident = false;
    for (const chunkY of chunkYs) {
      const chunk = this.world.getResidentChunk(chunkX, chunkY, chunkZ);
      if (!chunk) {
        continue;
      }
      resident = true;
      if (chunk.renderReady && chunk.mesh) {
        return { resident: true, renderReady: true };
      }
    }
    return { resident, renderReady: false };
  }

  probeLodCoverage(sampleRadiusMeters = 48, sampleStepMeters = 0.8): LodCoverageProbe {
    const normalizedRadius = Math.max(1, sampleRadiusMeters);
    const normalizedStep = Math.max(0.1, sampleStepMeters);
    const sampleRadiusWorldUnits = metersToWorldUnits(normalizedRadius);
    const sampleStepWorldUnits = metersToWorldUnits(normalizedStep);
    const centerX = this.player.feetPosition[0];
    const centerZ = this.player.feetPosition[2];
    const lodSpans = this.collectRenderableLodCoverageSpans();
    const uncoveredGapSamples: LodCoverageIssueSample[] = [];
    const handoffHoleSamples: LodCoverageIssueSample[] = [];
    const residentOverlapSamples: LodCoverageIssueSample[] = [];
    const bandOverlapSamples: LodCoverageIssueSample[] = [];
    const waterOverlapSamples: LodCoverageIssueSample[] = [];
    let sampleCount = 0;
    let residentSampleCount = 0;
    let renderReadySampleCount = 0;
    let visibleLod0OwnerSampleCount = 0;
    let coveredSampleCount = 0;
    let residentOverlapCount = 0;
    let uncoveredGapCount = 0;
    let handoffHoleCount = 0;
    let bandOverlapCount = 0;
    let waterOverlapCount = 0;
    let wrongBandCount = 0;

    for (let offsetZ = -sampleRadiusWorldUnits; offsetZ <= sampleRadiusWorldUnits; offsetZ += sampleStepWorldUnits) {
      for (let offsetX = -sampleRadiusWorldUnits; offsetX <= sampleRadiusWorldUnits; offsetX += sampleStepWorldUnits) {
        const worldX = centerX + offsetX;
        const worldZ = centerZ + offsetZ;
        const chunkX = Math.floor(worldX / this.world.chunkSize);
        const chunkZ = Math.floor(worldZ / this.world.chunkSize);
        const resident = this.world.hasResidentColumn(chunkX, chunkZ);
        const renderReady = this.world.isColumnRenderReady(chunkX, chunkZ);
        const visibleLod0Owner = renderReady || this.isVisibleLod0SurfaceRenderReady(worldX, worldZ, chunkX, chunkZ);
        const renderReadyWater = this.isRenderReadyWaterColumn(worldX, worldZ, chunkX, chunkZ);
        const lodOwnerStridesByBand = new Map<string, number>();
        const lodOwnerChunksByBand = new Map<string, Set<string>>();
        const lodOwnerStatesByChunk = new Map<string, LodCoverageOwnerState>();
        const lodVerticalRanges: LodCoverageVerticalRange[] = [];
        const waterBands = new Set<string>(renderReadyWater ? ["LOD0"] : []);
        for (const span of lodSpans) {
          const lodColumn = worldX >= span.minX
            && worldX < span.maxX
            && worldZ >= span.minZ
            && worldZ < span.maxZ
            ? span.classifyColumn(worldX, worldZ)
            : { covered: false, water: false, minY: null, maxY: null };
          if (
            lodColumn.covered
          ) {
            const ownerBand = formatLodOwnerBand(span.chunk);
            lodOwnerStridesByBand.set(ownerBand, span.strideMeters);
            const ownerChunks = lodOwnerChunksByBand.get(ownerBand) ?? new Set<string>();
            const ownerChunk = formatLodOwnerChunk(span.chunk);
            ownerChunks.add(ownerChunk);
            lodOwnerChunksByBand.set(ownerBand, ownerChunks);
            lodOwnerStatesByChunk.set(ownerChunk, { ownerChunk, ...span.state });
            if (lodColumn.minY !== null && lodColumn.maxY !== null) {
              lodVerticalRanges.push({
                band: ownerBand,
                ownerChunk,
                minY: lodColumn.minY,
                maxY: lodColumn.maxY,
              });
            }
            if (lodColumn.water) {
              waterBands.add(ownerBand);
            }
          }
        }
        const lodBands = [...lodOwnerStridesByBand.keys()];
        const ownerChunks = [...lodOwnerChunksByBand.values()].flatMap((chunks) => [...chunks]);
        const ownerStates = [...lodOwnerStatesByChunk.values()];
        const sampleStrideMeters = [...lodOwnerStridesByBand.values()];
        const bands = visibleLod0Owner ? ["LOD0", ...lodBands] : lodBands;
        const strides = visibleLod0Owner ? [worldUnitsToMeters(1), ...sampleStrideMeters] : sampleStrideMeters;
        const distanceWorldUnits = Math.max(Math.abs(offsetX), Math.abs(offsetZ));
        const distanceMeters = worldUnitsToMeters(distanceWorldUnits);
        const issueSample: LodCoverageIssueSample = {
          worldX,
          worldZ,
          distanceMeters,
          bands,
          sampleStrideMeters: strides,
          ownerChunks,
          ownerStates,
          verticalRanges: lodVerticalRanges,
        };
        sampleCount += 1;
        if (resident) {
          residentSampleCount += 1;
        }
        if (renderReady) {
          renderReadySampleCount += 1;
        }
        if (visibleLod0Owner) {
          visibleLod0OwnerSampleCount += 1;
        }
        if (bands.length > 0) {
          coveredSampleCount += 1;
        }
        if (visibleLod0Owner && lodBands.length > 0) {
          residentOverlapCount += 1;
          pushIssueSample(residentOverlapSamples, issueSample);
        }
        if (lodBands.length > 1 && lodCoverageRangesHaveVerticalOverlap(lodVerticalRanges)) {
          bandOverlapCount += 1;
          pushIssueSample(bandOverlapSamples, issueSample);
        }
        if (waterBands.size > 1) {
          waterOverlapCount += 1;
          pushIssueSample(waterOverlapSamples, {
            ...issueSample,
            bands: [...waterBands],
          });
        }
        if (bands.length === 0) {
          uncoveredGapCount += 1;
          pushIssueSample(uncoveredGapSamples, issueSample);
        }
        if (resident && !renderReady && lodBands.length === 0) {
          handoffHoleCount += 1;
          pushIssueSample(handoffHoleSamples, issueSample);
        }
      }
    }

    return {
      center: [...this.player.feetPosition],
      sampleRadiusMeters: normalizedRadius,
      sampleStepMeters: normalizedStep,
      sampleCount,
      residentSampleCount,
      renderReadySampleCount,
      visibleLod0OwnerSampleCount,
      coveredSampleCount,
      residentOverlapCount,
      uncoveredGapCount,
      handoffHoleCount,
      bandOverlapCount,
      waterOverlapCount,
      wrongBandCount,
      residentOverlapSamples,
      uncoveredGapSamples,
      handoffHoleSamples,
      bandOverlapSamples,
      waterOverlapSamples,
      wrongBandSamples: [],
    };
  }

  private isRenderReadyWaterColumn(
    worldX: number,
    worldZ: number,
    chunkX: number,
    chunkZ: number,
  ): boolean {
    const column = this.generator.sampleColumn(worldX, worldZ);
    if (column.waterTopY === null) {
      return false;
    }
    const minCy = Math.floor((column.surfaceY + 1) / this.world.chunkSize);
    const maxCy = Math.floor(column.waterTopY / this.world.chunkSize);
    const localX = positiveModulo(Math.floor(worldX), this.world.chunkSize);
    const localZ = positiveModulo(Math.floor(worldZ), this.world.chunkSize);
    const chunkArea = this.world.chunkSize * this.world.chunkSize;
    for (let cy = minCy; cy <= maxCy; cy += 1) {
      const chunk = this.world.getResidentChunk(chunkX, cy, chunkZ);
      if (!chunk || !chunk.renderReady) {
        continue;
      }
      for (let localY = 0; localY < this.world.chunkSize; localY += 1) {
        const material = chunk.data[localX + localY * this.world.chunkSize + localZ * chunkArea]!;
        if (!isProceduralWaterMaterial(material)) {
          continue;
        }
        const above = localY + 1 < this.world.chunkSize
          ? chunk.data[localX + (localY + 1) * this.world.chunkSize + localZ * chunkArea]!
          : 0;
        const worldTopY = cy * this.world.chunkSize + localY + 1;
        const waterContinuesAboveChunk = localY + 1 >= this.world.chunkSize
          && column.waterTopY >= worldTopY;
        if (!isProceduralWaterMaterial(above) && !waterContinuesAboveChunk) {
          return true;
        }
      }
    }
    return false;
  }

  private isVisibleLod0SurfaceRenderReady(
    worldX: number,
    worldZ: number,
    chunkX: number,
    chunkZ: number,
  ): boolean {
    const column = this.generator.sampleColumn(worldX, worldZ);
    const visibleY = column.waterTopY ?? column.surfaceY;
    const candidateChunkYs = new Set([
      Math.floor(visibleY / this.world.chunkSize),
      Math.floor(Math.max(0, visibleY - 1) / this.world.chunkSize),
      Math.floor(column.surfaceY / this.world.chunkSize),
    ]);
    for (const chunkY of candidateChunkYs) {
      const chunk = this.world.getResidentChunk(chunkX, chunkY, chunkZ);
      if (chunk?.renderReady && chunk.mesh) {
        return true;
      }
    }
    return false;
  }

  private collectRenderableLodCoverageSpans(): LodCoverageSpan[] {
    const spans: LodCoverageSpan[] = [];
    for (const chunk of this.world.iterateResidentChunks()) {
      if (
        chunk.lodLevel <= 0 ||
        !chunk.renderReady ||
        !chunk.mesh ||
        (chunk.mesh.indexCount === 0 && chunk.mesh.waterIndexCount === 0)
      ) {
        continue;
      }
      const worldSize = this.world.chunkSize * chunk.voxelStride;
      spans.push({
        label: `LOD${chunk.lodLevel}`,
        strideMeters: worldUnitsToMeters(chunk.voxelStride),
        chunk,
        classifyColumn: (worldX, worldZ) => this.world.classifyVisibleLodColumn(chunk, worldX, worldZ),
        state: this.world.getLodChunkDebugState(chunk),
        chunkSize: this.world.chunkSize,
        minX: chunk.coord.x * worldSize,
        maxX: (chunk.coord.x + 1) * worldSize,
        minZ: chunk.coord.z * worldSize,
        maxZ: (chunk.coord.z + 1) * worldSize,
      });
    }
    return spans;
  }

  async teleportAndSettle(
    position: Vec3,
    options: {
      radiusChunks?: number;
      maxFrames?: number;
    } = {},
  ): Promise<ResidencyTransitionProbe> {
    const before = this.snapshotResidentWorld();
    if (options.radiusChunks !== undefined) {
      this.world.setHorizontalRadiusChunks(options.radiusChunks);
    }
    teleportPlayerToEyePosition(this.player, position);
    this.syncCameraToPlayer();
    const maxFrames = Math.max(1, Math.floor(options.maxFrames ?? 240));
    let residency = this.syncWorldAroundPlayer(true);
    let render = await this.renderProbeFrame();
    let settleFrames = 1;
    while (
      settleFrames < maxFrames
      && (
        !residency.complete
        || residency.pendingChunks > 0
        || this.lastMeshBuildSummary.meshCount > 0
        || this.lastLodSummary.pending > 0
      )
    ) {
      residency = this.syncWorldAroundPlayer(true);
      render = await this.renderProbeFrame();
      settleFrames += 1;
    }
    const after = this.snapshotResidentWorld();
    const { entered, evicted } = diffChunkCoords(
      before.chunks.map((chunk) => chunk.coord),
      after.chunks.map((chunk) => chunk.coord),
    );
    this.pushHud(true);
    return {
      before,
      after,
      enteredChunkCoords: entered.map(toChunkTuple),
      evictedChunkCoords: evicted.map(toChunkTuple),
      generatedChunkCoords: residency.generatedChunkCoords.map(toChunkTuple),
      residency: {
        ...residency,
      generatedChunkCoords: residency.generatedChunkCoords.map((coord) => ({ ...coord })),
      evictedChunkCoords: residency.evictedChunkCoords.map((coord) => ({ ...coord })),
      },
      mesh: { ...this.lastMeshBuildSummary },
      render,
      settleFrames,
      settled: residency.complete
        && residency.pendingChunks === 0
        && this.lastMeshBuildSummary.meshCount === 0
        && this.lastLodSummary.pending === 0,
    };
  }

  async setCameraPoseAndSettle(
    position: Vec3,
    yawRadians: number,
    pitchRadians: number,
    options: {
      radiusChunks?: number;
      maxFrames?: number;
    } = {},
  ): Promise<ResidencyTransitionProbe & {
    snapshot: GameHudSnapshot;
  }> {
    const transition = await this.teleportAndSettle(position, options);
    this.camera.yaw = Number.isFinite(yawRadians) ? yawRadians : this.camera.yaw;
    this.camera.pitch = Number.isFinite(pitchRadians)
      ? Math.max(-Math.PI * 0.49, Math.min(Math.PI * 0.49, pitchRadians))
      : this.camera.pitch;
    this.syncCameraToPlayer();
    const render = await this.renderProbeFrame();
    this.pushHud(true);
    return {
      ...transition,
      render,
      snapshot: this.getDebugSnapshot(),
    };
  }

  async benchmarkChunkCrossing(iterations: number, chunkDelta = 1): Promise<ChunkBoundaryBenchmark> {
    const normalizedIterations = Math.max(1, Math.floor(iterations));
    const normalizedChunkDelta = Math.max(1, Math.floor(chunkDelta));
    const spawn = this.world.getSpawnPosition();
    const baseChunkX = Math.floor(spawn[0] / this.world.chunkSize);
    const baseChunkZ = Math.floor(spawn[2] / this.world.chunkSize);
    const leftTarget = buildEyePositionForChunkCenter(
      baseChunkX,
      baseChunkZ,
      spawn[1],
      this.world.chunkSize,
      this.player.eyeHeight,
    );
    const rightTarget = buildEyePositionForChunkCenter(
      baseChunkX + normalizedChunkDelta,
      baseChunkZ,
      spawn[1],
      this.world.chunkSize,
      this.player.eyeHeight,
    );
    const savedPlayer = {
      feetPosition: [...this.player.feetPosition] as Vec3,
      velocity: [...this.player.velocity] as Vec3,
      grounded: this.player.grounded,
    };
    const savedCamera = {
      position: [...this.camera.position] as Vec3,
      yaw: this.camera.yaw,
      pitch: this.camera.pitch,
      fovY: this.camera.fovY,
      near: this.camera.near,
      far: this.camera.far,
    };
    const savedStatus = this.status;
    const savedStreamAnchor = this.streamAnchor
      ? { chunkX: this.streamAnchor.chunkX, chunkZ: this.streamAnchor.chunkZ }
      : null;

    this.stop();
    try {
      await this.teleportAndSettle(leftTarget);
      const samples: ChunkBoundaryBenchmarkSample[] = [];
      for (let iteration = 0; iteration < normalizedIterations; iteration += 1) {
        for (const target of [rightTarget, leftTarget]) {
          const transition = await this.teleportAndSettle(target);
          samples.push({
            step: samples.length + 1,
            targetEyePosition: [...target],
            targetChunk: [
              Math.floor(target[0] / this.world.chunkSize),
              Math.floor((target[1] - this.player.eyeHeight) / this.world.chunkSize),
              Math.floor(target[2] / this.world.chunkSize),
            ],
            changed: transition.residency.changed,
            generatedChunks: transition.residency.generatedChunks,
            evictedChunks: transition.residency.evictedChunks,
            streamMs: transition.residency.elapsedMs,
            meshMs: transition.mesh.elapsedMs,
            meshNewChunks: transition.mesh.newMeshCount,
            meshRemeshChunks: transition.mesh.remeshCount,
            frameCpuMs: transition.render.frameCpuMs,
            syncMs: transition.render.syncResourcesMs,
            uploadMs: transition.render.uploadMs,
            uploadChunks: transition.render.uploadChunks,
            uploadBytes: transition.render.uploadBytes,
            encodeMs: transition.render.encodeMs,
          });
        }
      }
      return {
        iterations: normalizedIterations,
        chunkDelta: normalizedChunkDelta,
        radiusChunks: this.world.horizontalRadiusChunks,
        samples,
        summary: summarizeChunkBoundaryBenchmark(samples),
      };
    } finally {
      this.player.feetPosition = savedPlayer.feetPosition;
      this.player.velocity = savedPlayer.velocity;
      this.player.grounded = savedPlayer.grounded;
      this.camera = {
        position: savedCamera.position,
        yaw: savedCamera.yaw,
        pitch: savedCamera.pitch,
        fovY: savedCamera.fovY,
        near: savedCamera.near,
        far: savedCamera.far,
      };
      this.status = savedStatus;
      if (savedStreamAnchor) {
        this.syncWorldAroundAnchor(savedStreamAnchor);
      } else {
        this.streamAnchor = null;
        this.syncWorldAroundPlayer(true);
      }
      await this.renderProbeFrame();
      this.pushHud(true);
      this.start();
    }
  }

  async benchmarkChunkCacheReuse(chunkDelta = 24, maxFramesPerLeg = 240): Promise<ChunkCacheReuseBenchmark> {
    const normalizedChunkDelta = Math.max(1, Math.floor(chunkDelta));
    const normalizedMaxFrames = Math.max(1, Math.floor(maxFramesPerLeg));
    const spawn = this.world.getSpawnPosition();
    const baseChunkX = Math.floor(spawn[0] / this.world.chunkSize);
    const baseChunkZ = Math.floor(spawn[2] / this.world.chunkSize);
    const originTarget = buildEyePositionForChunkCenter(
      baseChunkX,
      baseChunkZ,
      spawn[1],
      this.world.chunkSize,
      this.player.eyeHeight,
    );
    const farTarget = buildEyePositionForChunkCenter(
      baseChunkX + normalizedChunkDelta,
      baseChunkZ,
      spawn[1],
      this.world.chunkSize,
      this.player.eyeHeight,
    );
    const savedPlayer = {
      feetPosition: [...this.player.feetPosition] as Vec3,
      velocity: [...this.player.velocity] as Vec3,
      grounded: this.player.grounded,
    };
    const savedCamera = {
      position: [...this.camera.position] as Vec3,
      yaw: this.camera.yaw,
      pitch: this.camera.pitch,
      fovY: this.camera.fovY,
      near: this.camera.near,
      far: this.camera.far,
    };
    const savedStatus = this.status;
    const savedStreamAnchor = this.streamAnchor
      ? { chunkX: this.streamAnchor.chunkX, chunkZ: this.streamAnchor.chunkZ }
      : null;

    this.stop();
    try {
      await this.teleportAndSettle(originTarget, { radiusChunks: this.world.horizontalRadiusChunks });
      const populate = await this.measureChunkCacheReuseLeg(farTarget, normalizedMaxFrames);
      const revisit = await this.measureChunkCacheReuseLeg(originTarget, normalizedMaxFrames);
      return {
        chunkDelta: normalizedChunkDelta,
        radiusChunks: this.world.horizontalRadiusChunks,
        populate,
        revisit,
      };
    } finally {
      this.player.feetPosition = savedPlayer.feetPosition;
      this.player.velocity = savedPlayer.velocity;
      this.player.grounded = savedPlayer.grounded;
      this.camera = {
        position: savedCamera.position,
        yaw: savedCamera.yaw,
        pitch: savedCamera.pitch,
        fovY: savedCamera.fovY,
        near: savedCamera.near,
        far: savedCamera.far,
      };
      this.status = savedStatus;
      if (savedStreamAnchor) {
        this.syncWorldAroundAnchor(savedStreamAnchor, true);
      } else {
        this.streamAnchor = null;
        this.syncWorldAroundPlayer(true);
      }
      await this.renderProbeFrame();
      this.pushHud(true);
      this.start();
    }
  }

  async benchmarkIncrementalCrossing(
    iterations: number,
    chunkDelta = 2,
    stepsPerLeg = 12,
    settleFrames = 16,
  ): Promise<IncrementalCrossingBenchmark> {
    const normalizedIterations = Math.max(1, Math.floor(iterations));
    const normalizedChunkDelta = Math.max(1, Math.floor(chunkDelta));
    const normalizedSteps = Math.max(2, Math.floor(stepsPerLeg));
    const normalizedSettleFrames = Math.max(1, Math.floor(settleFrames));
    const spawn = this.world.getSpawnPosition();
    const baseChunkX = Math.floor(spawn[0] / this.world.chunkSize);
    const baseChunkZ = Math.floor(spawn[2] / this.world.chunkSize);
    const leftFeet = buildFeetPositionForChunkCenter(baseChunkX, baseChunkZ, spawn[1], this.world.chunkSize);
    const rightFeet = buildFeetPositionForChunkCenter(
      baseChunkX + normalizedChunkDelta,
      baseChunkZ,
      spawn[1],
      this.world.chunkSize,
    );
    const savedPlayer = {
      feetPosition: [...this.player.feetPosition] as Vec3,
      velocity: [...this.player.velocity] as Vec3,
      grounded: this.player.grounded,
    };
    const savedCamera = {
      position: [...this.camera.position] as Vec3,
      yaw: this.camera.yaw,
      pitch: this.camera.pitch,
      fovY: this.camera.fovY,
      near: this.camera.near,
      far: this.camera.far,
    };
    const savedStatus = this.status;
    const savedStreamAnchor = this.streamAnchor
      ? { chunkX: this.streamAnchor.chunkX, chunkZ: this.streamAnchor.chunkZ }
      : null;

    this.stop();
    try {
      teleportPlayerToFeetPosition(this.player, leftFeet);
      this.player.grounded = true;
      this.syncCameraToPlayer();
      this.syncWorldAroundPlayer(true);
      await this.renderProbeFrame();

      const samples: IncrementalCrossingSample[] = [];
      let frame = 0;
      const legs: Array<readonly [Vec3, Vec3]> = [[leftFeet, rightFeet], [rightFeet, leftFeet]];
      for (let iteration = 0; iteration < normalizedIterations; iteration += 1) {
        for (const [startFeet, endFeet] of legs) {
          const legIndex = samples.length;
          for (let step = 1; step <= normalizedSteps; step += 1) {
            const t = step / normalizedSteps;
            teleportPlayerToFeetPosition(this.player, lerpVec3(startFeet, endFeet, t));
            this.player.grounded = true;
            this.syncCameraToPlayer();
            const residency = this.syncWorldAroundPlayer(false);
            const detailCoverage = this.probeRenderReadyCoverage();
            const render = await this.renderProbeFrame();
            frame += 1;
            samples.push(
              buildIncrementalSample(
                frame,
                "move",
                legIndex,
                residency,
                this.lastMeshBuildSummary,
                detailCoverage,
                render,
              ),
            );
          }
          for (let settleFrame = 0; settleFrame < normalizedSettleFrames; settleFrame += 1) {
            const residency = this.syncWorldAroundPlayer(false);
            const detailCoverage = this.probeRenderReadyCoverage();
            const render = await this.renderProbeFrame();
            frame += 1;
            samples.push(
              buildIncrementalSample(
                frame,
                "settle",
                legIndex,
                residency,
                this.lastMeshBuildSummary,
                detailCoverage,
                render,
              ),
            );
            if (
              residency.complete
              && residency.pendingChunks === 0
              && this.lastMeshBuildSummary.meshCount === 0
            ) {
              break;
            }
          }
        }
      }
      return {
        iterations: normalizedIterations,
        chunkDelta: normalizedChunkDelta,
        stepsPerLeg: normalizedSteps,
        settleFrames: normalizedSettleFrames,
        radiusChunks: this.world.horizontalRadiusChunks,
        samples,
        summary: summarizeIncrementalCrossing(samples),
      };
    } finally {
      this.player.feetPosition = savedPlayer.feetPosition;
      this.player.velocity = savedPlayer.velocity;
      this.player.grounded = savedPlayer.grounded;
      this.camera = {
        position: savedCamera.position,
        yaw: savedCamera.yaw,
        pitch: savedCamera.pitch,
        fovY: savedCamera.fovY,
        near: savedCamera.near,
        far: savedCamera.far,
      };
      this.status = savedStatus;
      if (savedStreamAnchor) {
        this.syncWorldAroundAnchor(savedStreamAnchor, true);
      } else {
        this.streamAnchor = null;
        this.syncWorldAroundPlayer(true);
      }
      await this.renderProbeFrame();
      this.pushHud(true);
      this.start();
    }
  }

  async benchmarkRouteExperience(
    options: RouteExperienceBenchmarkOptions = {},
  ): Promise<RouteExperienceBenchmark> {
    const spawnFeet = this.world.getSpawnPosition();
    const routePlan = buildDefaultRouteBenchmarkPlan(
      spawnFeet,
      (worldX, worldZ) => this.generator.sampleColumn(worldX, worldZ).surfaceY + 1,
      normalizeRouteBenchmarkPlanOptions(options),
    );
    return this.runRouteExperienceBenchmark(routePlan, options);
  }

  async benchmarkForwardWalkExperience(
    options: RouteExperienceBenchmarkOptions & {
      yawRadians?: number;
    } = {},
  ): Promise<RouteExperienceBenchmark> {
    const spawnFeet = this.world.getSpawnPosition();
    const routePlan = buildForwardRouteBenchmarkPlan(
      spawnFeet,
      (worldX, worldZ) => this.generator.sampleColumn(worldX, worldZ).surfaceY + 1,
      {
        ...normalizeRouteBenchmarkPlanOptions(options),
        yawRadians: options.yawRadians,
      },
    );
    return this.runRouteExperienceBenchmark(routePlan, options);
  }

  async benchmarkLiveForwardWalkExperience(
    options: RouteExperienceBenchmarkOptions & {
      yawRadians?: number;
      yawDriftRadians?: number;
      yawDriftPeriodSeconds?: number;
      sprint?: boolean;
    } = {},
  ): Promise<RouteExperienceBenchmark> {
    const normalizedOptions = normalizeRouteBenchmarkPlanOptions(options);
    const durationSeconds = normalizedOptions.durationSeconds;
    const settleSeconds = Math.max(1, options.settleSeconds ?? 4);
    const seamProbeStrideFrames = clampPositiveInt(
      options.seamProbeStrideFrames ?? Math.max(1, Math.round((normalizedOptions.sampleHz ?? 60) / 4)),
      Math.max(1, Math.round((normalizedOptions.sampleHz ?? 60) / 4)),
    );
    const captureStrideFrames = clampPositiveInt(
      options.captureStrideFrames ?? 999999,
      999999,
    );
    const captureWidth = clampPositiveInt(options.captureWidth ?? 128, 128);
    const captureHeight = clampPositiveInt(options.captureHeight ?? 72, 72);
    const referenceDiffStrideFrames = Math.max(0, Math.floor(options.referenceDiffStrideFrames ?? 0));
    const referenceDiffLimit = Math.max(0, Math.floor(options.referenceDiffLimit ?? 24));
    const yaw = options.yawRadians ?? 0;
    const yawDriftRadians = Math.max(0, options.yawDriftRadians ?? 0);
    const yawDriftPeriodSeconds = Math.max(0.1, options.yawDriftPeriodSeconds ?? Math.max(1, durationSeconds));
    const pitch = -0.34;
    const savedPlayer = {
      feetPosition: [...this.player.feetPosition] as Vec3,
      velocity: [...this.player.velocity] as Vec3,
      grounded: this.player.grounded,
    };
    const savedCamera = {
      position: [...this.camera.position] as Vec3,
      yaw: this.camera.yaw,
      pitch: this.camera.pitch,
      fovY: this.camera.fovY,
      near: this.camera.near,
      far: this.camera.far,
    };
    const savedStatus = this.status;
    const savedPointerLocked = this.pointerLocked;
    const savedPressedKeys = [...this.pressedKeys];
    const savedStreamAnchor = this.streamAnchor
      ? { chunkX: this.streamAnchor.chunkX, chunkZ: this.streamAnchor.chunkZ }
      : null;

    this.stop();
    try {
      const spawnFeet = this.world.getSpawnPosition();
      teleportPlayerToFeetPosition(this.player, spawnFeet);
      this.player.velocity = [0, 0, 0];
      this.player.grounded = true;
      this.camera.yaw = yaw;
      this.camera.pitch = pitch;
      this.syncCameraToPlayer();
      this.pointerLocked = true;
      this.pressedKeys.clear();
      this.pressedKeys.add("KeyW");
      if (options.sprint === true) {
        this.pressedKeys.add("ControlLeft");
      }
      this.lastFrameTime = 0;
      this.syncWorldAroundPlayer(true);
      this.renderCurrentFrame();
      await this.renderer?.waitForGpuIdle();

      const samples: RouteExperienceFrameSample[] = [];
      const capturedFrames: CapturedBenchmarkFrame[] = [];
      let totalDistanceMeters = 0;
      const startedAt = performance.now();
      let frame = 0;
      let lastFeetPosition: Vec3 = [...this.player.feetPosition];
      while (true) {
        const now = await nextAnimationFrame();
        const elapsedSeconds = (performance.now() - startedAt) / 1000;
        const phase: "move" | "settle" = elapsedSeconds < durationSeconds ? "move" : "settle";
        if (phase === "settle") {
          this.pressedKeys.delete("KeyW");
          this.pressedKeys.delete("ControlLeft");
        }
        this.camera.yaw = yaw + resolveBenchmarkYawDrift(
          elapsedSeconds,
          yawDriftRadians,
          yawDriftPeriodSeconds,
        );
        const interactiveFrame = this.advanceInteractiveFrame(now);
        frame += 1;
        totalDistanceMeters += worldUnitsToMeters(Math.hypot(
          this.player.feetPosition[0] - lastFeetPosition[0],
          this.player.feetPosition[2] - lastFeetPosition[2],
        ));
        lastFeetPosition = [...this.player.feetPosition];
        const frameResult = await this.captureRouteExperienceFrameSample(
          {
            frame,
            phase,
            simTimeSeconds: elapsedSeconds,
            distanceMeters: totalDistanceMeters,
          },
          interactiveFrame.movementMs,
          interactiveFrame.gameplayFrameMs,
          seamProbeStrideFrames,
          captureStrideFrames,
          captureWidth,
          captureHeight,
          referenceDiffStrideFrames,
          referenceDiffLimit,
          samples.length,
          capturedFrames.length,
          {
            frameStats: {
              drawCalls: interactiveFrame.render.drawCalls,
              triangles: interactiveFrame.render.triangles,
              syncResourcesMs: interactiveFrame.render.syncResourcesMs,
              uploadMs: interactiveFrame.render.uploadMs,
              uploadChunks: interactiveFrame.render.uploadChunks,
              uploadBytes: interactiveFrame.render.uploadBytes,
              encodeMs: interactiveFrame.render.encodeMs,
              frustumCulledChunks: this.lastRenderStats.frustumCulledChunks,
              fogCulledChunks: this.lastRenderStats.fogCulledChunks,
              lodDrawCalls: this.lastRenderStats.lodDrawCalls,
              lodDrawCallsByLevel: this.lastRenderStats.lodDrawCallsByLevel,
            },
            frameCpuMs: interactiveFrame.render.frameCpuMs,
          },
        );
        samples.push(frameResult.sample);
        if (frameResult.capturedFrame) {
          capturedFrames.push(frameResult.capturedFrame);
        }
        if (
          phase === "settle"
          && elapsedSeconds >= durationSeconds + settleSeconds
        ) {
          break;
        }
      }

      if (capturedFrames.length > 0) {
        await this.applySettledReferenceDiffs(samples, capturedFrames);
      }

      return {
        seed: this.generator.seed,
        radiusChunks: this.world.horizontalRadiusChunks,
        captureStrideFrames,
        seamProbeStrideFrames,
        referenceDiffStrideFrames,
        referenceDiffLimit,
        durationSeconds,
        settleSeconds,
        totalDistanceMeters,
        sampleHz: durationSeconds <= 0 ? samples.length : samples.length / Math.max(durationSeconds + settleSeconds, 0.001),
        speedMetersPerSecond: durationSeconds <= 0 ? 0 : totalDistanceMeters / durationSeconds,
        samples,
        summary: summarizeRouteExperienceBenchmark(samples, {
          totalDistanceMeters,
          sampleHz: durationSeconds <= 0 ? samples.length : samples.length / Math.max(durationSeconds + settleSeconds, 0.001),
          speedMetersPerSecond: durationSeconds <= 0 ? 0 : totalDistanceMeters / durationSeconds,
        }),
      };
    } finally {
      this.pressedKeys.clear();
      for (const code of savedPressedKeys) {
        this.pressedKeys.add(code);
      }
      this.pointerLocked = savedPointerLocked;
      this.player.feetPosition = savedPlayer.feetPosition;
      this.player.velocity = savedPlayer.velocity;
      this.player.grounded = savedPlayer.grounded;
      this.camera = {
        position: savedCamera.position,
        yaw: savedCamera.yaw,
        pitch: savedCamera.pitch,
        fovY: savedCamera.fovY,
        near: savedCamera.near,
        far: savedCamera.far,
      };
      this.status = savedStatus;
      this.lastFrameTime = 0;
      if (savedStreamAnchor) {
        this.syncWorldAroundAnchor(savedStreamAnchor, true);
      } else {
        this.streamAnchor = null;
        this.syncWorldAroundPlayer(true);
      }
      this.renderCurrentFrame();
      await this.renderer?.waitForGpuIdle();
      this.pushHud(true);
      this.start();
    }
  }

  private async runRouteExperienceBenchmark(
    routePlan: ReturnType<typeof buildDefaultRouteBenchmarkPlan>,
    options: RouteExperienceBenchmarkOptions,
  ): Promise<RouteExperienceBenchmark> {
    const settleSeconds = Math.max(1, options.settleSeconds ?? 4);
    const sampleHz = routePlan.sampleHz;
    const seamProbeStrideFrames = clampPositiveInt(
      options.seamProbeStrideFrames ?? Math.max(1, Math.round(sampleHz / 4)),
      Math.max(1, Math.round(sampleHz / 4)),
    );
    const captureStrideFrames = clampPositiveInt(
      options.captureStrideFrames ?? Math.max(1, Math.round(sampleHz / 2)),
      Math.max(1, Math.round(sampleHz / 2)),
    );
    const captureWidth = clampPositiveInt(options.captureWidth ?? 128, 128);
    const captureHeight = clampPositiveInt(options.captureHeight ?? 72, 72);
    const referenceDiffStrideFrames = Math.max(0, Math.floor(options.referenceDiffStrideFrames ?? 0));
    const referenceDiffLimit = Math.max(0, Math.floor(options.referenceDiffLimit ?? 24));
    const savedPlayer = {
      feetPosition: [...this.player.feetPosition] as Vec3,
      velocity: [...this.player.velocity] as Vec3,
      grounded: this.player.grounded,
    };
    const savedCamera = {
      position: [...this.camera.position] as Vec3,
      yaw: this.camera.yaw,
      pitch: this.camera.pitch,
      fovY: this.camera.fovY,
      near: this.camera.near,
      far: this.camera.far,
    };
    const savedStatus = this.status;
    const savedStreamAnchor = this.streamAnchor
      ? { chunkX: this.streamAnchor.chunkX, chunkZ: this.streamAnchor.chunkZ }
      : null;
    const spawnFeet = this.world.getSpawnPosition();

    this.stop();
    try {
      const initialTarget = routePlan.frames[0] ?? {
        frame: 1,
        phase: "move" as const,
        simTimeSeconds: 0,
        distanceMeters: 0,
        feetPosition: spawnFeet,
        yaw: this.camera.yaw,
        pitch: this.camera.pitch,
        segmentIndex: 0,
      };
      teleportPlayerToFeetPosition(this.player, initialTarget.feetPosition);
      this.player.grounded = true;
      this.camera.yaw = initialTarget.yaw;
      this.camera.pitch = initialTarget.pitch;
      this.syncCameraToPlayer();
      this.syncWorldAroundPlayer(true);
      this.renderCurrentFrame();
      await this.renderer?.waitForGpuIdle();

      const samples: RouteExperienceFrameSample[] = [];
      const capturedFrames: CapturedBenchmarkFrame[] = [];
      for (const target of routePlan.frames) {
        const frameResult = await this.runRouteExperienceFrame(
          target,
          seamProbeStrideFrames,
          captureStrideFrames,
          captureWidth,
          captureHeight,
          referenceDiffStrideFrames,
          referenceDiffLimit,
          samples.length,
          capturedFrames.length,
        );
        samples.push(frameResult.sample);
        if (frameResult.capturedFrame) {
          capturedFrames.push(frameResult.capturedFrame);
        }
      }

      const finalTarget = routePlan.frames[routePlan.frames.length - 1] ?? initialTarget;
      const maxSettleFrames = Math.max(1, Math.round(settleSeconds * sampleHz));
      for (let settleFrame = 1; settleFrame <= maxSettleFrames; settleFrame += 1) {
        const frameResult = await this.runRouteExperienceFrame(
          {
            ...finalTarget,
            frame: routePlan.frames.length + settleFrame,
            phase: "settle",
            simTimeSeconds: routePlan.durationSeconds + settleFrame / sampleHz,
            distanceMeters: routePlan.totalDistanceMeters,
          },
          seamProbeStrideFrames,
          captureStrideFrames,
          captureWidth,
          captureHeight,
          referenceDiffStrideFrames,
          referenceDiffLimit,
          samples.length,
          capturedFrames.length,
        );
        const sample = frameResult.sample;
        samples.push(sample);
        if (frameResult.capturedFrame) {
          capturedFrames.push(frameResult.capturedFrame);
        }
        if (
          sample.complete
          && sample.pendingChunks === 0
          && sample.dirtyResidentChunks === 0
          && sample.visibleGroundUncoveredCount === 0
        ) {
          break;
        }
      }

      if (capturedFrames.length > 0) {
        await this.applySettledReferenceDiffs(samples, capturedFrames);
      }

      return {
        seed: this.generator.seed,
        radiusChunks: this.world.horizontalRadiusChunks,
        captureStrideFrames,
        seamProbeStrideFrames,
        referenceDiffStrideFrames,
        referenceDiffLimit,
        durationSeconds: routePlan.durationSeconds,
        settleSeconds,
        totalDistanceMeters: routePlan.totalDistanceMeters,
        sampleHz: routePlan.sampleHz,
        speedMetersPerSecond: routePlan.speedMetersPerSecond,
        samples,
        summary: summarizeRouteExperienceBenchmark(samples, routePlan),
      };
    } finally {
      this.player.feetPosition = savedPlayer.feetPosition;
      this.player.velocity = savedPlayer.velocity;
      this.player.grounded = savedPlayer.grounded;
      this.camera = {
        position: savedCamera.position,
        yaw: savedCamera.yaw,
        pitch: savedCamera.pitch,
        fovY: savedCamera.fovY,
        near: savedCamera.near,
        far: savedCamera.far,
      };
      this.status = savedStatus;
      if (savedStreamAnchor) {
        this.syncWorldAroundAnchor(savedStreamAnchor, true);
      } else {
        this.streamAnchor = null;
        this.syncWorldAroundPlayer(true);
      }
      this.renderCurrentFrame();
      await this.renderer?.waitForGpuIdle();
      this.pushHud(true);
      this.start();
    }
  }

  private async measureChunkCacheReuseLeg(
    targetEyePosition: Vec3,
    maxFrames: number,
  ): Promise<ChunkCacheReuseLegSummary> {
    teleportPlayerToEyePosition(this.player, targetEyePosition);
    this.player.grounded = true;
    this.syncCameraToPlayer();
    let frameCount = 0;
    let totalStreamMs = 0;
    let totalMeshMs = 0;
    let totalGeneratedChunks = 0;
    let totalPersistedChunkHits = 0;
    let totalPersistedSummaryHits = 0;
    let totalPersistedRegionSummaryHits = 0;
    let totalMissingRegionSummaries = 0;
    let totalWorkerGeneratedChunks = 0;
    let maxPendingChunks = 0;
    for (; frameCount < maxFrames; frameCount += 1) {
      const residency = this.syncWorldAroundPlayer(frameCount === 0);
      await this.renderProbeFrame();
      totalStreamMs += residency.elapsedMs;
      totalMeshMs += this.lastMeshBuildSummary.elapsedMs;
      totalGeneratedChunks += residency.generatedChunks;
      totalPersistedChunkHits += residency.phaseMs.completedChunkCacheHits;
      totalPersistedSummaryHits += residency.phaseMs.completedSummaryCacheHits;
      totalPersistedRegionSummaryHits += residency.phaseMs.completedRegionSummaryCacheHits;
      totalMissingRegionSummaries += residency.phaseMs.missingRegionSummaries;
      totalWorkerGeneratedChunks += residency.phaseMs.completedGeneratedChunks;
      maxPendingChunks = Math.max(maxPendingChunks, residency.pendingChunks);
      if (
        residency.complete
        && residency.pendingChunks === 0
        && this.lastMeshBuildSummary.meshCount === 0
      ) {
        break;
      }
    }
    return {
      targetChunk: [
        Math.floor(targetEyePosition[0] / this.world.chunkSize),
        Math.floor((targetEyePosition[1] - this.player.eyeHeight) / this.world.chunkSize),
        Math.floor(targetEyePosition[2] / this.world.chunkSize),
      ],
      frameCount: frameCount + 1,
      settled: frameCount < maxFrames,
      totalStreamMs,
      totalMeshMs,
      totalGeneratedChunks,
      totalPersistedChunkHits,
      totalPersistedSummaryHits,
      totalPersistedRegionSummaryHits,
      totalMissingRegionSummaries,
      totalWorkerGeneratedChunks,
      maxPendingChunks,
      residentChunks: this.world.getStats().chunkCount,
    };
  }

  async pumpWorldForBenchmark(
    position?: Vec3,
    options: BenchmarkWorldPumpOptions = {},
  ): Promise<BenchmarkWorldPumpSummary> {
    const maxFrames = Math.max(1, Math.floor(options.maxFrames ?? 120));
    const stopWhenSettled = options.stopWhenSettled ?? true;
    const lodBudget = {
      maxGenerateLodChunks: Math.max(0, Math.floor(options.maxGenerateLodChunks ?? DEFAULT_MAX_LOD_CHUNKS_PER_FRAME)),
      maxAdoptCompletedLodChunks: Math.max(1, Math.floor(options.maxAdoptCompletedLodChunks ?? DEFAULT_MAX_LOD_ADOPTIONS_PER_FRAME)),
      maxPlanMs: Math.max(0.1, options.maxLodPlanMs ?? DEFAULT_MAX_LOD_PLAN_MS_PER_FRAME),
      maxWorkMs: Math.max(0.1, options.maxLodWorkMs ?? DEFAULT_MAX_LOD_WORK_MS_PER_FRAME),
    };
    const residencyBudget = {
      maxEvictChunks: Math.max(1, Math.floor(options.maxEvictChunks ?? MOVING_MAX_EVICT_CHUNKS_PER_FRAME)),
      maxPlanMs: Math.max(0.1, options.maxResidencyPlanMs ?? MOVING_MAX_RESIDENCY_PLAN_MS_PER_FRAME),
    };
    const meshBudget = Math.max(0, Math.floor(options.maxMeshRebuilds ?? DEFAULT_MAX_MESH_REBUILDS_PER_FRAME));

    if (position) {
      teleportPlayerToEyePosition(this.player, position);
      this.player.grounded = true;
      this.syncCameraToPlayer();
    }

    const startedAt = performance.now();
    let frameCount = 0;
    let totalGenerated = 0;
    const totalGeneratedByLevel = [0, 0, 0, 0, 0];
    let totalMemoryCacheHits = 0;
    const totalMemoryCacheHitsByLevel = [0, 0, 0, 0, 0];
    let totalEmptyCacheHits = 0;
    const totalEmptyCacheHitsByLevel = [0, 0, 0, 0, 0];
    let totalDiskCacheHits = 0;
    const totalDiskCacheHitsByLevel = [0, 0, 0, 0, 0];
    let totalDiskCacheMisses = 0;
    let totalWorkerGenerated = 0;
    const totalWorkerGeneratedByLevel = [0, 0, 0, 0, 0];
    let totalScheduledWorkerRequests = 0;
    let totalScheduledDiskRequests = 0;
    let totalScheduledDiskStores = 0;
    let totalCompletedDiskStores = 0;
    let totalDownsampleMs = 0;
    let totalMeshMs = 0;
    let maxLodChunkMs = 0;
    let maxWorstRecentFrameMs = 0;
    let maxRecentHitchCount = 0;
    let maxRecentDroppedFrameEstimate = 0;
    let settled = false;
    let finalSnapshot = this.getDebugSnapshot();

    for (; frameCount < maxFrames; frameCount += 1) {
      await nextAnimationFrame();
      const residency = this.syncWorldAroundPlayer(false, true, lodBudget, residencyBudget, meshBudget);
      await this.renderProbeFrame();
      finalSnapshot = this.getDebugSnapshot();
      totalGenerated += this.lastLodSummary.generated;
      addLodLevelCounts(totalGeneratedByLevel, this.lastLodSummary.generatedByLevel);
      totalMemoryCacheHits += this.lastLodSummary.cacheHits;
      addLodLevelCounts(totalMemoryCacheHitsByLevel, this.lastLodSummary.cacheHitsByLevel);
      totalEmptyCacheHits += this.lastLodSummary.emptyCacheHits;
      addLodLevelCounts(totalEmptyCacheHitsByLevel, this.lastLodSummary.emptyCacheHitsByLevel);
      totalDiskCacheHits += this.lastLodSummary.lodDiskCacheHits;
      addLodLevelCounts(totalDiskCacheHitsByLevel, this.lastLodSummary.lodDiskCacheHitsByLevel);
      totalDiskCacheMisses += this.lastLodSummary.lodDiskCacheMisses;
      totalWorkerGenerated += this.lastLodSummary.lodWorkerGenerated;
      addLodLevelCounts(totalWorkerGeneratedByLevel, this.lastLodSummary.lodWorkerGeneratedByLevel);
      totalScheduledWorkerRequests += this.lastLodSummary.scheduledLodWorkerRequests;
      totalScheduledDiskRequests += this.lastLodSummary.scheduledLodDiskRequests;
      totalScheduledDiskStores += this.lastLodSummary.scheduledLodDiskStores;
      totalCompletedDiskStores += this.lastLodSummary.completedLodDiskStores;
      totalDownsampleMs += this.lastLodSummary.downsampleMs;
      totalMeshMs += this.lastLodSummary.meshMs;
      maxLodChunkMs = Math.max(maxLodChunkMs, this.lastLodSummary.maxChunkMs);
      maxWorstRecentFrameMs = Math.max(maxWorstRecentFrameMs, finalSnapshot.frameTiming.worstRecentFrameMs);
      maxRecentHitchCount = Math.max(maxRecentHitchCount, finalSnapshot.frameTiming.recentHitchCount);
      maxRecentDroppedFrameEstimate = Math.max(
        maxRecentDroppedFrameEstimate,
        finalSnapshot.frameTiming.recentDroppedFrameEstimate,
      );
      settled = residency.complete
        && residency.pendingChunks === 0
        && this.lastMeshBuildSummary.meshCount === 0
        && this.lastLodSummary.pending === 0
        && (this.asyncChunkGeneration?.getPendingCount() ?? 0) === 0
        && (this.asyncChunkMeshing?.getPendingCount() ?? 0) === 0;
      if (settled && stopWhenSettled && frameCount >= 2) {
        frameCount += 1;
        break;
      }
    }

    const summary = {
      frameCount,
      settled,
      elapsedMs: performance.now() - startedAt,
      totalGenerated,
      totalGeneratedByLevel,
      totalMemoryCacheHits,
      totalMemoryCacheHitsByLevel,
      totalEmptyCacheHits,
      totalEmptyCacheHitsByLevel,
      totalDiskCacheHits,
      totalDiskCacheHitsByLevel,
      totalDiskCacheMisses,
      totalWorkerGenerated,
      totalWorkerGeneratedByLevel,
      totalScheduledWorkerRequests,
      totalScheduledDiskRequests,
      totalScheduledDiskStores,
      totalCompletedDiskStores,
      totalDownsampleMs,
      totalMeshMs,
      maxLodChunkMs,
      maxWorstRecentFrameMs,
      maxRecentHitchCount,
      maxRecentDroppedFrameEstimate,
      finalSnapshot,
    };
    this.pushHud(true);
    return summary;
  }

  private loadBootstrapWorld(): void {
    const spawn = this.world.getSpawnPosition();
    this.player = createPlayerState(spawn, { grounded: true });
    this.camera = createFirstPersonCamera(getPlayerEyePosition(this.player), 0.8, -0.32);
    this.streamAnchor = null;
    const bootstrapStartedAt = performance.now();
    this.syncWorldAroundPlayer(false);
    this.lastGameplayFrameMs = performance.now() - bootstrapStartedAt;
    this.recordBootstrapBenchmarkSample(this.lastGameplayFrameMs);
    this.status = this.bootstrapPlayableReady ? "Click once to capture cursor" : "Preparing world";
    this.pushHud(true);
  }

  private async drainBootstrapBenchmark(maxFrames = 600): Promise<void> {
    if (this.bootstrapPlayableReady) {
      return;
    }
    const normalizedMaxFrames = Math.max(1, Math.floor(maxFrames));
    for (let frame = 0; frame < normalizedMaxFrames; frame += 1) {
      const gameplayStartedAt = performance.now();
      if (
        this.lastStreamSummary.pendingChunks > 0
        || this.world.countDirtyResidentChunks() > 0
        || this.lastLodSummary.pending > 0
      ) {
        this.syncWorldAroundPlayer(false);
      }
      this.renderCurrentFrame();
      this.lastGameplayFrameMs = performance.now() - gameplayStartedAt;
      this.recordBootstrapBenchmarkSample(this.lastGameplayFrameMs);
      if (this.bootstrapPlayableReady) {
        return;
      }
      await new Promise<void>((resolve) => {
        setTimeout(resolve, 0);
      });
    }
    throw new Error("Bootstrap benchmark did not complete within the allotted deterministic drain frames");
  }

  private attachInteractions(): void {
    this.canvas.addEventListener("click", this.handleCanvasClick);
    document.addEventListener("contextmenu", this.handleDocumentContextMenu);
    document.addEventListener("pointerlockchange", this.handlePointerLockChange);
    document.addEventListener("mousemove", this.handleMouseMove);
    window.addEventListener("keydown", this.handleKeyDown);
    window.addEventListener("keyup", this.handleKeyUp);
    window.addEventListener("blur", this.handleBlur);
    document.addEventListener("visibilitychange", this.handleVisibilityChange);
  }

  private performFocusedInteraction(): void {
    const currentWorld = this.sampleCurrentWorldContext();
    const discovery = this.refreshDiscoveryJournal(true);
    const target = this.resolveCurrentInteraction(currentWorld, discovery).target;
    const prompt = target?.prompts.find((candidate) => !candidate.disabled) ?? null;
    if (!target || !prompt) {
      const encounter = sampleRpgEncounterWorldUnits(this.player.feetPosition[0], this.player.feetPosition[2]);
      const primaryFaction = encounter.factionHints[0]?.factionId ?? null;
      const scoutResult = describeRpgEncounterScoutResult(encounter);
      const nearestPassiveMob = this.samplePassiveMobSightings()[0] ?? null;
      this.explorationEventLog.record({
        kind: "encounter",
        subjectType: "mob",
        subjectId: nearestPassiveMob ? nearestPassiveMob.id : encounter.factionHints[0]?.factionId ?? encounter.moodId,
        role: nearestPassiveMob ? "passive-sighting" : encounter.moodId,
        name: nearestPassiveMob?.name ?? scoutResult.label,
        flavorText: nearestPassiveMob
          ? `${nearestPassiveMob.label} is moving through the nearby terrain.`
          : scoutResult.detail,
        worldPosition: nearestPassiveMob
          ? [nearestPassiveMob.position[0], this.player.feetPosition[1], nearestPassiveMob.position[2]]
          : [...this.player.feetPosition],
        repeatable: true,
        payload: {
          factionId: nearestPassiveMob?.factionId ?? encounter.factionHints[0]?.factionId ?? null,
          speciesId: nearestPassiveMob?.speciesId ?? null,
          speciesName: nearestPassiveMob?.speciesName ?? null,
          distanceMeters: nearestPassiveMob ? Number(worldUnitsToMeters(nearestPassiveMob.distanceWorldUnits).toFixed(1)) : null,
          fieldNote: nearestPassiveMob
            ? `${nearestPassiveMob.label} sighted ${formatPassiveMobDistance(worldUnitsToMeters(nearestPassiveMob.distanceWorldUnits))} away.`
            : scoutResult.detail,
          pressure: Number(encounter.pressure.toFixed(3)),
          moodId: nearestPassiveMob?.moodId ?? encounter.moodId,
          regionId: nearestPassiveMob?.regionId ?? encounter.regionId,
          routeId: nearestPassiveMob?.routeId ?? encounter.routeId,
          caveSystemId: nearestPassiveMob?.caveSystemId ?? encounter.caveSystemId,
        },
      });
      this.observeActiveQuestStep(currentWorld, discovery, encounter, primaryFaction, null, null, ["listen"]);
      this.lastInteractionLabel = nearestPassiveMob
        ? `Sighted ${nearestPassiveMob.speciesName}`
        : `${scoutResult.label}: ${scoutResult.pressureLabel}`;
      this.status = nearestPassiveMob
        ? `${nearestPassiveMob.label} ${formatPassiveMobDistance(worldUnitsToMeters(nearestPassiveMob.distanceWorldUnits))} away.`
        : scoutResult.detail;
      this.pushHud(true);
      return;
    }

    const eventResult = this.explorationEventLog.record(prompt.eventInput);
    if (eventResult.accepted) {
      this.skillJournal.observeSkillAwards(eventResult.event.skillAwards);
    }
    if (target.role === "cave-mouth" && prompt.verb === "use") {
      this.enterCaveMouth(target, currentWorld);
      this.pushHud(true);
      return;
    }
    if (target.role === "cave-exit" && prompt.verb === "use") {
      this.exitCaveMouth();
      this.pushHud(true);
      return;
    }
    const routeId = readPayloadRouteId(prompt.eventInput.payload) ?? routeIdForLandmark(target.id);
    const result = this.observeTravelGoalProgress({
      routeId,
      kind: prompt.verb,
      targetId: target.id,
    });
    const encounter = sampleRpgEncounterWorldUnits(this.player.feetPosition[0], this.player.feetPosition[2]);
    this.observeActiveQuestStep(
      currentWorld,
      discovery,
      encounter,
      encounter.factionHints[0]?.factionId ?? null,
      target.id,
      routeId,
      ["listen", "inspect", "interpret", "report"],
    );
    const completedTitle = result.completedGoalIds
      .map((goalId) => TRAVEL_GOALS.find((goal) => goal.id === goalId)?.title ?? goalId)
      .join(", ");
    this.lastInteractionLabel = completedTitle
      ? `Completed ${completedTitle}`
      : result.changed
      ? prompt.label
      : `${prompt.label} noted`;
    this.status = this.lastInteractionLabel;
    this.pushHud(true);
  }

  private buildActiveExplorationHudState(
    currentWorld: CurrentWorldProbeContext,
    discovery: ExplorationJournalSnapshot,
    routeSnapshot: RouteJournalSnapshot,
    encounter: RpgEncounterSample,
  ): ActiveExplorationHudState {
    const target = this.resolveCurrentInteraction(currentWorld, discovery).target;
    const activeGoal = selectActiveTravelGoal(routeSnapshot);
    const nextStep = activeGoal ? findNextTravelGoalStep(activeGoal) : null;
    const navigation = target
      ? describeNavigationBearing({
          viewerPosition: this.camera.position,
          viewerYawRadians: this.camera.yaw,
          targetPosition: target.worldPosition,
          targetName: target.name,
        })
      : null;
    const regionName = formatDiscoveryName("biome", currentWorld.probe.biomeId, "Unknown region");
    const placeName = target?.name
      ?? (discovery.currentLandmarkId
        ? formatDiscoveryName("landmark", discovery.currentLandmarkId)
        : currentWorld.probe.regionalVariantId
        ? formatDiscoveryName("regional-variant", currentWorld.probe.regionalVariantId)
        : regionName);
    const prompt = target?.prompts.find((candidate) => !candidate.disabled) ?? null;
    const routeName = activeGoal ? formatRouteName(activeGoal.routeId) : "Open road";
    return {
      activePlaceName: placeName,
      activeRouteName: routeName,
      activeRouteProgressLabel: activeGoal
        ? `${activeGoal.completedRequiredStepCount}/${activeGoal.requiredStepCount} steps`
        : "No route tracked",
      activeTravelGoalTitle: activeGoal?.title ?? "Wander",
      activeTravelGoalStepLabel: nextStep?.label ?? (activeGoal?.completed ? "Route complete" : "Find a road sign"),
      activeTravelGoalProgressRatio: activeGoal?.progress ?? 0,
      interactionTargetName: target?.name ?? "No nearby focus",
      interactionPromptLabel: prompt?.label ?? describeRpgEncounterScoutResult(encounter).label,
      interactionPromptDescription: prompt?.description
        ?? (target ? `${target.distanceMeters.toFixed(1)} m away` : describeRpgEncounterScoutResult(encounter).detail),
      interactionPromptVerb: prompt?.verb ?? "inspect",
      navigationTargetId: target?.id ?? null,
      navigationTargetName: navigation?.targetName ?? null,
      navigationSource: navigation ? "interaction-target" : null,
      navigationDistanceMeters: navigation?.distanceMeters ?? null,
      navigationBearingLabel: navigation?.bearingLabel ?? null,
      navigationDistanceLabel: navigation?.distanceLabel ?? null,
      navigationCompassLabel: navigation?.compassLabel ?? null,
      navigationTurnLabel: navigation?.turnLabel ?? null,
    };
  }

  private resolveCurrentInteraction(
    currentWorld: CurrentWorldProbeContext,
    discovery: ExplorationJournalSnapshot,
  ): ReturnType<typeof resolveExplorationInteractionTarget> {
    return resolveExplorationInteractionTarget({
      viewerPosition: this.camera.position,
      viewerForward: buildFirstPersonCameraMatrices(this.camera, 1).forward,
      maxDistanceMeters: metersToWorldUnits(8),
      candidates: this.buildExplorationInteractionCandidates(currentWorld, discovery),
    });
  }

  private samplePassiveMobSightings(): readonly PassiveMobSighting[] {
    return samplePassiveMobSightingsWorldUnits(
      this.player.feetPosition[0],
      this.player.feetPosition[2],
      {
        radiusWorldUnits: PASSIVE_MOB_SIGHTING_RADIUS_WORLD_UNITS,
        cap: PASSIVE_MOB_SIGHTING_CAP,
      },
    );
  }

  private buildExplorationInteractionCandidates(
    currentWorld: CurrentWorldProbeContext,
    discovery: ExplorationJournalSnapshot,
  ): ExplorationInteractionCandidate[] {
    const landmarkIds = [
      currentWorld.probe.landmarkId,
      discovery.currentLandmarkId,
    ].filter((landmarkId): landmarkId is string => typeof landmarkId === "string" && landmarkId.length > 0);
    const uniqueLandmarkIds = [...new Set(landmarkIds)];
    const candidates: ExplorationInteractionCandidate[] = [];
    const forward = buildFirstPersonCameraMatrices(this.camera, 1).forward;
    const encounter = sampleRpgEncounterWorldUnits(this.player.feetPosition[0], this.player.feetPosition[2]);
    const caveExitCandidate = this.buildCaveExitInteractionCandidate(forward);
    if (caveExitCandidate) {
      candidates.push(caveExitCandidate);
    }
    const caveMouthCandidate = buildCaveMouthInteractionCandidate(currentWorld, encounter);
    if (caveMouthCandidate) {
      candidates.push(caveMouthCandidate);
    }
    for (const landmarkId of uniqueLandmarkIds) {
      const presentation = describeDiscovery("landmark", landmarkId);
      const routeId = routeIdForLandmark(landmarkId);
      candidates.push({
        id: landmarkId,
        subjectType: landmarkId === "ashlander_travel_pack" ? "object" : "landmark",
        name: presentation.name,
        role: presentation.role,
        worldPosition: [
          this.player.feetPosition[0] + forward[0] * metersToWorldUnits(1.4),
          currentWorld.probe.surfaceY,
          this.player.feetPosition[2] + forward[2] * metersToWorldUnits(1.4),
        ],
        interactionRadiusMeters: metersToWorldUnits(5),
        priority: ROUTE_LANDMARK_IDS.has(landmarkId) ? 20 : 4,
        prompts: buildLandmarkInteractionPrompts(landmarkId, presentation.role),
        flavorText: presentation.flavorText,
        payload: routeId ? { routeId } : undefined,
      });
    }
    const encounterSite = sampleRpgEncounterSiteWorldUnits(
      this.player.feetPosition[0],
      this.player.feetPosition[2],
      encounter,
    );
    candidates.push({
      id: encounterSite.id,
      subjectType: "mob",
      name: encounterSite.name,
      role: encounterSite.role,
      worldPosition: [
        encounterSite.x,
        currentWorld.probe.surfaceY,
        encounterSite.z,
      ],
      interactionRadiusMeters: encounterSite.interactionRadiusWorldUnits,
      priority: encounterSite.priority,
      prompts: [{
        verb: "inspect",
        label: `Scout ${encounterSite.name}`,
        description: encounterSite.fieldNote,
      }],
      flavorText: encounterSite.fieldNote,
      skillAwards: [{
        skillId: "naturalist",
        xp: 16,
        reason: "First local encounter site",
        awardKey: `encounter-site:${encounterSite.id}`,
        onceOnly: true,
      }],
      payload: {
        siteKind: encounterSite.kind,
        factionId: encounterSite.factionId,
        clueLabel: encounterSite.clueLabel,
        fieldNote: encounterSite.fieldNote,
        pressure: Number(encounter.pressure.toFixed(3)),
        moodId: encounter.moodId,
        regionId: encounter.regionId,
        routeId: encounter.routeId,
        caveSystemId: encounter.caveSystemId,
      },
    });
    const worldSystems = sampleWorldSystems(
      (performance.now() - this.worldClockStartedAt) / 1000,
      currentWorld.probe,
      currentWorld.ambientProfile,
      encounter,
      this.lastTravelContext,
    );
    const forageSite = sampleForageSiteWorldUnits(
      this.player.feetPosition[0],
      this.player.feetPosition[2],
      worldSystems.area,
    );
    const lootState = getLootJournalCandidateState(this.explorationEventLog.getSnapshot(), {
      subjectId: forageSite.id,
      lootId: worldSystems.area.lootId,
      categoryId: forageSite.role,
    });
    const exactLootRevisit = lootState.match === "subject" && lootState.collected;
    const forageProbe = this.generator.sampleBiomeProbe(forageSite.x, forageSite.z);
    candidates.push({
      id: forageSite.id,
      subjectType: "object",
      name: forageSite.name,
      role: "loot-cache",
      worldPosition: [
        forageSite.x,
        forageProbe.surfaceY,
        forageSite.z,
      ],
      interactionRadiusMeters: forageSite.interactionRadiusWorldUnits,
      priority: 1,
      prompts: [{
        verb: "use",
        label: exactLootRevisit ? `Revisit ${forageSite.name}` : worldSystems.area.lootInteractionLabel,
        description: describeLootCandidatePrompt(forageSite.fieldNote, lootState),
      }],
      flavorText: forageSite.fieldNote,
      occurrenceId: exactLootRevisit ? `revisit-${lootState.eventCount + 1}` : null,
      repeatable: exactLootRevisit,
      skillAwards: [{
        skillId: worldSystems.area.lootSkillId,
        xp: 18,
        reason: "First local find",
        awardKey: `loot:${forageSite.id}`,
        onceOnly: true,
      }],
      payload: {
        lootId: worldSystems.area.lootId,
        categoryId: forageSite.role,
        forageSiteRole: forageSite.role,
        clueLabel: forageSite.clueLabel,
        fieldNote: forageSite.fieldNote,
        collectedBefore: exactLootRevisit,
        lootJournalMatch: lootState.match,
        previousFindNote: lootState.lastNote,
        forageSourceLandmarkId: worldSystems.area.forageSourceLandmarkId,
        weather: worldSystems.weather.id,
        hazard: worldSystems.area.hazardLabel,
      },
    });
    return candidates;
  }

  private buildCaveExitInteractionCandidate(forward: readonly [number, number, number]): ExplorationInteractionCandidate | null {
    if (!this.caveReturnFeetPosition || this.lastTravelContext !== "underground") {
      return null;
    }
    return {
      id: "cave-exit:return",
      subjectType: "zone",
      name: "Cave Mouth Return",
      role: "cave-exit",
      worldPosition: [
        this.player.feetPosition[0] + forward[0] * metersToWorldUnits(1.4),
        this.player.feetPosition[1] + PLAYER_EYE_HEIGHT * 0.5,
        this.player.feetPosition[2] + forward[2] * metersToWorldUnits(1.4),
      ],
      interactionRadiusMeters: metersToWorldUnits(5),
      priority: 18,
      prompts: [{
        verb: "use",
        label: "Exit to cave mouth",
        description: "Climb back toward the last surface entrance.",
      }],
      flavorText: "A memorized return path leads back to the surface.",
      payload: {
        caveTraversal: "exit",
      },
    };
  }

  private enterCaveMouth(
    target: ResolvedExplorationInteractionTarget,
    currentWorld: CurrentWorldProbeContext,
  ): void {
    const entryFeet = findSafeCaveEntryFeetPosition({
      world: this.world,
      anchorPosition: target.worldPosition,
      surfaceY: currentWorld.probe.surfaceY,
    });
    if (!entryFeet) {
      this.lastInteractionLabel = `${target.name} is blocked`;
      this.status = "No safe passage is open here yet.";
      return;
    }
    this.caveReturnFeetPosition = [...this.player.feetPosition] as Vec3;
    teleportPlayerToFeetPosition(this.player, entryFeet);
    this.player.grounded = true;
    this.lastTravelContext = "underground";
    this.lastTravelContextFeetPosition = null;
    this.lastTravelContextSampleAt = 0;
    this.syncCameraToPlayer();
    this.syncWorldAroundPlayer(true);
    this.lastInteractionLabel = `Entered ${target.name}`;
    this.status = `${target.name}: underground route`;
  }

  private exitCaveMouth(): void {
    const returnFeet = this.caveReturnFeetPosition;
    if (!returnFeet) {
      this.lastInteractionLabel = "No return path remembered";
      this.status = this.lastInteractionLabel;
      return;
    }
    teleportPlayerToFeetPosition(this.player, returnFeet);
    this.player.grounded = true;
    this.caveReturnFeetPosition = null;
    this.lastTravelContext = "surface";
    this.lastTravelContextFeetPosition = null;
    this.lastTravelContextSampleAt = 0;
    this.syncCameraToPlayer();
    this.syncWorldAroundPlayer(true);
    this.lastInteractionLabel = "Exited to cave mouth";
    this.status = this.lastInteractionLabel;
  }

  private readonly handleCanvasClick = () => {
    if (!this.pointerLocked) {
      void this.requestPointerLock();
    }
  };

  private readonly handleDocumentContextMenu = (event: MouseEvent) => {
    if (this.pointerLocked || event.target === this.canvas) {
      event.preventDefault();
    }
  };

  private readonly handlePointerLockChange = () => {
    const browserPointerLocked = this.ownsPointerLockElement(document.pointerLockElement);
    if (browserPointerLocked) {
      this.pointerLockFallbackActive = false;
    }
    this.pointerLocked = browserPointerLocked || this.pointerLockFallbackActive;
    this.status = browserPointerLocked
      ? "Exploring: WASD move, Space jump, Ctrl sprint, Alt slow"
      : this.pointerLockFallbackActive
      ? "Input captured: WASD move"
      : "Click once to capture cursor";
    this.pushHud(true);
  };

  private ownsPointerLockElement(element: Element | null): boolean {
    return element instanceof HTMLElement && this.pointerLockTargets.has(element);
  }

  private activatePointerLockFallback(error: unknown): void {
    this.pointerLockFallbackActive = true;
    this.pointerLocked = true;
    this.status = error instanceof Error
      ? `Input captured; browser pointer lock unavailable (${error.name})`
      : "Input captured; browser pointer lock unavailable";
    this.pushHud(true);
  }

  private readonly handleMouseMove = (event: MouseEvent) => {
    if (!this.pointerLocked) {
      return;
    }
    rotateFirstPersonCamera(this.camera, event.movementX, event.movementY);
  };

  private readonly handleKeyDown = (event: KeyboardEvent) => {
    if (event.code === "Escape" && this.pointerLockFallbackActive) {
      event.preventDefault();
      this.pointerLockFallbackActive = false;
      this.pointerLocked = false;
      this.pressedKeys.clear();
      this.status = "Click once to capture cursor";
      this.pushHud(true);
      return;
    }
    if (this.pointerLocked && isMovementKey(event.code)) {
      event.preventDefault();
    }
    if (this.pointerLocked && (event.code === "KeyE" || event.code === "Enter")) {
      event.preventDefault();
      this.performFocusedInteraction();
      return;
    }
    this.pressedKeys.add(event.code);
  };

  private readonly handleKeyUp = (event: KeyboardEvent) => {
    this.pressedKeys.delete(event.code);
  };

  private readonly handleBlur = () => {
    this.pressedKeys.clear();
  };

  private readonly handleVisibilityChange = () => {
    if (document.hidden) {
      this.pressedKeys.clear();
    }
  };

  private updateMovement(deltaSeconds: number): { moved: boolean; distanceMeters: number; travelContext: "surface" | "underground" } {
    const travelContext = this.resolveTravelSkillContext();
    const explorationSkillEffects = resolveExplorationSkillEffects(this.skillJournal.getSnapshot());
    const input = this.pointerLocked
      ? {
          forward: (this.isPressed("KeyW") ? 1 : 0) - (this.isPressed("KeyS") ? 1 : 0),
          strafe: (this.isPressed("KeyD") ? 1 : 0) - (this.isPressed("KeyA") ? 1 : 0),
          jump: this.isPressed("Space"),
          sprint: this.isPressed("ControlLeft", "ControlRight"),
          precision: this.isPressed("AltLeft", "AltRight"),
          speedMultiplier: travelContext === "underground"
            ? explorationSkillEffects.undergroundTravelSpeedMultiplier
            : explorationSkillEffects.surfaceTravelSpeedMultiplier,
        }
      : {
          forward: 0,
          strafe: 0,
          jump: false,
          sprint: false,
          precision: false,
          speedMultiplier: 1,
        };
    const prevFeet: [number, number, number] = [...this.player.feetPosition];
    const result = stepPlayer(
      this.world,
      this.player,
      this.camera.yaw,
      input,
      deltaSeconds,
    );
    // Prevent player from entering chunks without collision data (LOD-only territory).
    // Check if the target position has a resident LOD 0 chunk column.
    const targetCx = Math.floor(this.player.feetPosition[0] / this.world.chunkSize);
    const targetCz = Math.floor(this.player.feetPosition[2] / this.world.chunkSize);
    if (!this.world.hasResidentColumn(targetCx, targetCz)) {
      this.player.feetPosition[0] = prevFeet[0];
      this.player.feetPosition[2] = prevFeet[2];
      this.player.velocity[0] = 0;
      this.player.velocity[2] = 0;
    }
    this.syncCameraToPlayer();
    const deltaX = this.player.feetPosition[0] - prevFeet[0];
    const deltaZ = this.player.feetPosition[2] - prevFeet[2];
    return {
      moved: result.moved,
      distanceMeters: worldUnitsToMeters(Math.hypot(deltaX, deltaZ)),
      travelContext,
    };
  }

  private hasMovementIntent(): boolean {
    if (!this.pointerLocked) {
      return false;
    }
    return this.isPressed("KeyW", "KeyS", "KeyA", "KeyD", "Space", "ControlLeft", "ControlRight");
  }

  private renderInteractiveFrame(): GameRenderProbe {
    const rendered = this.renderCurrentFrame();
    if (!rendered) {
      return zeroGameRenderProbe();
    }
    const { frameCpuMs } = rendered;
    this.avgFrameCpuMs = this.avgFrameCpuMs === 0
      ? frameCpuMs
      : this.avgFrameCpuMs * 0.9 + frameCpuMs * 0.1;
    this.pushHud();
    return {
      frameCpuMs: rendered.frameCpuMs,
      syncResourcesMs: rendered.frameStats.syncResourcesMs,
      uploadMs: rendered.frameStats.uploadMs,
      uploadChunks: rendered.frameStats.uploadChunks,
      uploadBytes: rendered.frameStats.uploadBytes,
      encodeMs: rendered.frameStats.encodeMs,
      drawCalls: rendered.frameStats.drawCalls,
      triangles: rendered.frameStats.triangles,
    };
  }

  private advanceInteractiveFrame(now: number): {
    movementMs: number;
    render: GameRenderProbe;
    gameplayFrameMs: number;
  } {
    const gameplayStartedAt = performance.now();
    const rawDeltaMs = this.lastFrameTime === 0
      ? 1000 / 60
      : Math.max(0, now - this.lastFrameTime);
    const frameTiming = this.frameTimingBuckets.record(now);
    if (rawDeltaMs >= frameTiming.hitchThresholdMs) {
      this.lastHitchAttribution = {
        ...this.lastCompletedFrameAttribution,
        wallMs: rawDeltaMs,
        cause: this.lastCompletedFrameAttribution.cause === "none"
          ? "browser or idle"
          : this.lastCompletedFrameAttribution.cause,
      };
    }
    this.lastFrameWallMs = rawDeltaMs;
    this.avgFrameWallMs = this.avgFrameWallMs === 0
      ? rawDeltaMs
      : this.avgFrameWallMs * 0.9 + rawDeltaMs * 0.1;
    const deltaSeconds = Math.min(rawDeltaMs / 1000, MAX_DELTA_SECONDS);
    this.lastFrameTime = now;
    this.interactiveFrameNumber += 1;
    this.lastFrameLodMs = 0;
    const hasMovementIntent = this.hasMovementIntent();
    const movementStartedAt = performance.now();
    const movement = this.updateMovement(deltaSeconds);
    if (movement.distanceMeters > 0) {
      this.skillJournal.observeTravel(movement.distanceMeters, movement.travelContext);
    }
    const movementMs = performance.now() - movementStartedAt;
    const dirtyResidentChunks = this.world.countDirtyResidentChunks();
    const movementActive = hasMovementIntent || movement.moved;
    let streamMs = 0;
    let meshMs = 0;
    let lodMs = 0;
    const movingLodBudget: LodUpdateBudget = {
      maxGenerateLodChunks: MOVING_MAX_LOD_CHUNKS_PER_FRAME,
      maxAdoptCompletedLodChunks: MOVING_MAX_LOD_ADOPTIONS_PER_FRAME,
      maxPlanMs: MOVING_MAX_LOD_PLAN_MS_PER_FRAME,
      maxWorkMs: MOVING_MAX_LOD_WORK_MS_PER_FRAME,
    };
    const shouldRunMovingLodUpdate = movementActive
      && this.interactiveFrameNumber % MOVING_LOD_UPDATE_INTERVAL_FRAMES === 0
      && this.lastStreamSummary.pendingChunks === 0
      && dirtyResidentChunks === 0;
    const allowLodUpdate = !movementActive || shouldRunMovingLodUpdate;
    const lodBudget = shouldRunMovingLodUpdate ? movingLodBudget : undefined;
    if (
      shouldPumpWorldWork(
        movementActive,
        this.lastStreamSummary.pendingChunks,
        dirtyResidentChunks,
        this.lastLodSummary.pending,
      )
    ) {
      const residency = this.syncWorldAroundPlayer(
        false,
        allowLodUpdate,
        lodBudget,
        movementActive
          ? {
              maxEvictChunks: MOVING_MAX_EVICT_CHUNKS_PER_FRAME,
              maxPlanMs: MOVING_MAX_RESIDENCY_PLAN_MS_PER_FRAME,
            }
          : undefined,
        movementActive ? MOVING_MAX_MESH_REBUILDS_PER_FRAME : undefined,
      );
      streamMs = residency.elapsedMs;
      meshMs = this.meshMs;
      lodMs = this.lastFrameLodMs;
    }
    const render = this.renderInteractiveFrame();
    const gameplayFrameMs = performance.now() - gameplayStartedAt;
    this.lastGameplayFrameMs = gameplayFrameMs;
    this.lastCompletedFrameAttribution = createFrameAttribution({
      frame: this.interactiveFrameNumber,
      wallMs: rawDeltaMs,
      gameplayMs: gameplayFrameMs,
      movementMs,
      streamMs,
      meshMs,
      lodMs,
      renderCpuMs: render.frameCpuMs,
      renderSyncMs: render.syncResourcesMs,
      renderUploadMs: render.uploadMs,
      renderEncodeMs: render.encodeMs,
    });
    this.recordBootstrapBenchmarkSample(gameplayFrameMs);
    return {
      movementMs,
      render,
      gameplayFrameMs,
    };
  }

  private resolveTravelSkillContext(): "surface" | "underground" {
    const now = performance.now();
    const currentFeetPosition = this.player.feetPosition;
    if (this.lastTravelContextFeetPosition) {
      const deltaX = currentFeetPosition[0] - this.lastTravelContextFeetPosition[0];
      const deltaZ = currentFeetPosition[2] - this.lastTravelContextFeetPosition[2];
      const movedFarEnough = Math.hypot(deltaX, deltaZ) >= TRAVEL_CONTEXT_SAMPLE_MOVE_THRESHOLD_WORLD_UNITS;
      if (!movedFarEnough && now - this.lastTravelContextSampleAt < TRAVEL_CONTEXT_SAMPLE_INTERVAL_MS) {
        return this.lastTravelContext;
      }
    }
    this.lastTravelContextSampleAt = now;
    this.lastTravelContextFeetPosition = [...currentFeetPosition] as Vec3;
    const centerX = Math.floor(this.player.feetPosition[0]);
    const centerZ = Math.floor(this.player.feetPosition[2]);
    const probe = this.generator.sampleBiomeProbe(centerX, centerZ);
    const observedUndergroundBiomeId = resolveObservedUndergroundBiomeId(
      this.world,
      this.camera.position,
      probe.surfaceY,
      probe.undergroundBiomeId,
    );
    this.lastTravelContext = observedUndergroundBiomeId ? "underground" : "surface";
    return this.lastTravelContext;
  }

  private syncWorldAroundPlayer(
    force = false,
    allowLodUpdate = true,
    lodBudget?: LodUpdateBudget,
    residencyBudget?: { maxEvictChunks?: number; maxPlanMs?: number },
    meshBudget?: number,
  ): ResidencyUpdateSummary {
    const playerChunkX = Math.floor(this.player.feetPosition[0] / this.world.chunkSize);
    const playerChunkZ = Math.floor(this.player.feetPosition[2] / this.world.chunkSize);
    const resolved = force
      ? {
          anchor: { chunkX: playerChunkX, chunkZ: playerChunkZ },
          changed: true,
        }
      : resolveStreamAnchor(this.streamAnchor, playerChunkX, playerChunkZ, STREAM_ANCHOR_MARGIN_CHUNKS);
    if (force) {
      return this.syncWorldAroundAnchor(resolved.anchor, true, true);
    }
    if (!shouldRefreshResidency(false, resolved.changed, this.lastStreamSummary.pendingChunks)) {
      this.flushMeshBuildBudget(meshBudget);
      if (allowLodUpdate) {
        this.recordLodSummary(this.world.updateLodResidencyAround(this.player.feetPosition, {
          maxGenerateLodChunks: lodBudget?.maxGenerateLodChunks ?? DEFAULT_MAX_LOD_CHUNKS_PER_FRAME,
          maxAdoptCompletedLodChunks: lodBudget?.maxAdoptCompletedLodChunks ?? DEFAULT_MAX_LOD_ADOPTIONS_PER_FRAME,
          maxPlanMs: lodBudget?.maxPlanMs ?? DEFAULT_MAX_LOD_PLAN_MS_PER_FRAME,
          maxWorkMs: lodBudget?.maxWorkMs ?? DEFAULT_MAX_LOD_WORK_MS_PER_FRAME,
        }));
        this.lastFrameLodMs = this.lastLodSummary.elapsedMs;
      } else {
        this.deferLodWork();
      }
      this.lastStreamSummary = createIdleResidencySummary(
        this.lastStreamSummary,
        this.streamAnchor ?? resolved.anchor,
        this.world.horizontalRadiusChunks,
        this.world.getStats().chunkCount,
        this.world.countDirtyResidentChunks(),
      );
      return cloneResidencySummary(this.lastStreamSummary);
    }
    return this.syncWorldAroundAnchor(resolved.anchor, false, allowLodUpdate, lodBudget, residencyBudget, meshBudget);
  }

  private syncWorldAroundAnchor(
    anchor: StreamAnchor,
    settle = false,
    allowLodUpdate = true,
    lodBudget?: LodUpdateBudget,
    residencyBudget?: { maxEvictChunks?: number; maxPlanMs?: number },
    meshBudget?: number,
  ): ResidencyUpdateSummary {
    this.streamAnchor = anchor;
    const residency = this.world.updateResidencyAround(
      buildStreamAnchorPosition(anchor, this.world.chunkSize, this.player.feetPosition[1]),
      {
        maxGenerateChunks: settle
          ? Number.POSITIVE_INFINITY
          : this.streamingBudgets.maxGeneratedChunksPerUpdate,
        maxEvictChunks: settle
          ? Number.POSITIVE_INFINITY
          : residencyBudget?.maxEvictChunks,
        maxPlanMs: settle
          ? Number.POSITIVE_INFINITY
          : residencyBudget?.maxPlanMs,
      },
    );
    this.lastStreamSummary = cloneResidencySummary(residency);
    this.flushMeshBuildBudget(
      settle ? Number.POSITIVE_INFINITY : meshBudget ?? this.streamingBudgets.maxMeshRebuildsPerFrame,
    );
    if (settle || allowLodUpdate) {
      this.recordLodSummary(this.world.updateLodResidencyAround(this.player.feetPosition, {
        maxGenerateLodChunks: settle
          ? Number.POSITIVE_INFINITY
          : lodBudget?.maxGenerateLodChunks ?? DEFAULT_MAX_LOD_CHUNKS_PER_FRAME,
        maxAdoptCompletedLodChunks: settle
          ? Number.POSITIVE_INFINITY
          : lodBudget?.maxAdoptCompletedLodChunks ?? DEFAULT_MAX_LOD_ADOPTIONS_PER_FRAME,
        maxPlanMs: settle
          ? Number.POSITIVE_INFINITY
          : lodBudget?.maxPlanMs ?? DEFAULT_MAX_LOD_PLAN_MS_PER_FRAME,
        maxWorkMs: settle
          ? Number.POSITIVE_INFINITY
          : lodBudget?.maxWorkMs ?? DEFAULT_MAX_LOD_WORK_MS_PER_FRAME,
      }));
      this.lastFrameLodMs = this.lastLodSummary.elapsedMs;
    } else {
      this.deferLodWork();
    }
    this.status = residency.pendingChunks > 0
      ? `Streaming ${residency.pendingChunks} pending chunk(s)`
      : residency.generatedChunks > 0 || residency.evictedChunks > 0
      ? `Streamed ${residency.generatedChunks} chunk(s), evicted ${residency.evictedChunks}`
      : "Residency updated";
    return cloneResidencySummary(this.lastStreamSummary);
  }

  private deferLodWork(): void {
    this.lastFrameLodMs = 0;
    this.lastLodSummary = {
      ...this.lastLodSummary,
      generated: 0,
      generatedByLevel: [0, 0, 0, 0, 0],
      cacheHits: 0,
      cacheHitsByLevel: [0, 0, 0, 0, 0],
      emptyCacheHits: 0,
      emptyCacheHitsByLevel: [0, 0, 0, 0, 0],
      pending: Math.max(1, this.lastLodSummary.pending),
      elapsedMs: 0,
      yRangeMs: 0,
      downsampleMs: 0,
      meshMs: 0,
      commitMs: 0,
      maxChunkMs: 0,
      maxChunkLevel: 0,
      maxChunkKey: null,
      neededKeyCacheHit: false,
      scheduledRegionSummaryRequests: 0,
    lodDiskCacheHits: 0,
    lodDiskCacheHitsByLevel: [0, 0, 0, 0, 0],
    lodDiskCacheMisses: 0,
    lodWorkerGenerated: 0,
    lodWorkerGeneratedByLevel: [0, 0, 0, 0, 0],
    scheduledLodWorkerRequests: 0,
    scheduledLodDiskRequests: 0,
    scheduledLodDiskStores: 0,
    completedLodDiskStores: 0,
    };
  }

  private recordLodSummary(summary: LodResidencyUpdateSummary): void {
    this.lastLodSummary = summary;
    this.cumulativeLodGeneratedChunks += summary.generated;
    this.cumulativeLodWorkerGenerated += summary.lodWorkerGenerated;
    this.cumulativeLodDiskCacheHits += summary.lodDiskCacheHits;
    this.cumulativeLodDiskCacheMisses += summary.lodDiskCacheMisses;
    this.cumulativeLodScheduledDiskRequests += summary.scheduledLodDiskRequests;
    this.cumulativeLodScheduledDiskStores += summary.scheduledLodDiskStores;
    this.cumulativeLodCompletedDiskStores += summary.completedLodDiskStores;
  }

  private isPressed(...codes: string[]): boolean {
    return codes.some((code) => this.pressedKeys.has(code));
  }

  private syncCameraToPlayer(): void {
    this.camera.position = getPlayerEyePosition(this.player);
  }

  private flushMeshBuildBudget(maxChunks = DEFAULT_MAX_MESH_REBUILDS_PER_FRAME): void {
    if (this.asyncChunkMeshing) {
      const startedAt = performance.now();
      const chunkBudget = Number.isFinite(maxChunks) ? Math.max(0, Math.floor(maxChunks)) : Number.POSITIVE_INFINITY;
      let meshCount = 0;
      let newMeshCount = 0;
      let remeshCount = 0;
      let triangleCount = 0;
      const dirtyChunks = collectDirtyChunks(this.world, this.player.feetPosition);
      const priorityChunkX = Math.floor(this.player.feetPosition[0] / this.world.chunkSize);
      const priorityChunkY = Math.floor(this.player.feetPosition[1] / this.world.chunkSize);
      const priorityChunkZ = Math.floor(this.player.feetPosition[2] / this.world.chunkSize);

      for (const completed of this.asyncChunkMeshing.drainCompletedMeshes(chunkBudget)) {
        const chunk = this.world.getResidentChunk(completed.coord.x, completed.coord.y, completed.coord.z);
        if (!chunk || chunk.meshRevision !== completed.meshRevision) {
          continue;
        }
        chunk.mesh = buildChunkMeshFromOpaqueGeometry(
          this.world,
          completed.coord.x,
          completed.coord.y,
          completed.coord.z,
          completed.opaqueMesh,
        );
        setChunkMeshDirtyState(this.world, chunk, false);
        chunk.pendingMeshRevision = null;
        chunk.gpuDirty = true;
        meshCount += 1;
        if (chunk.meshBuilt) {
          remeshCount += 1;
        } else {
          newMeshCount += 1;
          chunk.meshBuilt = true;
        }
        this.world.noteResidentChunkRenderReadyState(chunk, chunk.meshBuilt && chunk.mesh !== null);
        triangleCount += chunk.mesh?.triangleCount ?? 0;
      }

      let syncBuiltCount = 0;
      for (const chunk of dirtyChunks) {
        const remainingMeshBudget = chunkBudget - meshCount;
        if (remainingMeshBudget <= 0 || syncBuiltCount >= Math.min(MAX_SYNC_NEAR_MESH_REBUILDS_PER_FRAME, remainingMeshBudget)) {
          break;
        }
        if (!chunk.meshDirty) {
          continue;
        }
        if (!shouldSyncBuildUrgentChunk(chunk, priorityChunkX, priorityChunkY, priorityChunkZ)) {
          continue;
        }
        const hasPendingJob = this.asyncChunkMeshing.hasPendingChunk(chunk.coord.x, chunk.coord.y, chunk.coord.z);
        const wasBuilt = chunk.meshBuilt;
        chunk.mesh = buildChunkMesh(this.world, chunk.coord.x, chunk.coord.y, chunk.coord.z);
        setChunkMeshDirtyState(this.world, chunk, false);
        chunk.gpuDirty = true;
        if (hasPendingJob) {
          chunk.pendingMeshRevision = chunk.meshRevision;
        } else {
          chunk.pendingMeshRevision = null;
        }
        syncBuiltCount += 1;
        meshCount += 1;
        if (wasBuilt) {
          remeshCount += 1;
        } else {
          newMeshCount += 1;
          chunk.meshBuilt = true;
        }
        this.world.noteResidentChunkRenderReadyState(chunk, chunk.meshBuilt && chunk.mesh !== null);
        triangleCount += chunk.mesh?.triangleCount ?? 0;
      }

      let scheduledCount = 0;
      for (const chunk of dirtyChunks) {
        const remainingScheduleBudget = chunkBudget - scheduledCount;
        if (remainingScheduleBudget <= 0) {
          break;
        }
        if (!chunk.meshDirty) {
          continue;
        }
        if (chunk.pendingMeshRevision === chunk.meshRevision) {
          continue;
        }
        const input = createOpaqueChunkMeshingInput(
          this.world,
          chunk.coord.x,
          chunk.coord.y,
          chunk.coord.z,
          { cloneData: true },
        );
        if (!input) {
          continue;
        }
        if (!this.asyncChunkMeshing.requestChunk(input, chunk.meshRevision)) {
          break;
        }
        chunk.pendingMeshRevision = chunk.meshRevision;
        scheduledCount += 1;
      }

      const elapsedMs = performance.now() - startedAt;
      this.lastMeshBuildSummary = {
        meshCount,
        newMeshCount,
        remeshCount,
        triangleCount,
        elapsedMs,
      };
      this.meshMs = elapsedMs;
      return;
    }
    const meshSummary = rebuildDirtyMeshes(this.world, maxChunks, {
      priorityPosition: this.player.feetPosition,
    });
    this.lastMeshBuildSummary = meshSummary;
    this.meshMs = meshSummary.elapsedMs;
  }

  private renderCurrentFrame(): {
    frameStats: RenderStats;
    frameCpuMs: number;
  } | null {
    if (!this.renderer) {
      return null;
    }
    this.renderer.configureCanvas(this.canvas);
    const aspect = this.canvas.width / this.canvas.height;
    const cameraMatrices = buildFirstPersonCameraMatrices(this.camera, aspect);
    const renderEnvironment = this.resolveRenderEnvironment();
    const cpuStartedAt = performance.now();
    const frameStats = this.renderer.render(
      this.world,
      cameraMatrices,
      null,
      0,
      renderEnvironment,
    );
    const frameCpuMs = performance.now() - cpuStartedAt;
    this.lastRenderStats = frameStats;
    this.lastFrameCpuMs = frameCpuMs;
    this.drawCalls = frameStats.drawCalls;
    this.triangles = frameStats.triangles;
    return {
      frameStats,
      frameCpuMs,
    };
  }

  private async runRouteExperienceFrame(
    target: Pick<RouteBenchmarkFrameTarget, "frame" | "simTimeSeconds" | "distanceMeters" | "feetPosition" | "yaw" | "pitch">
      & { phase: "move" | "settle" },
    seamProbeStrideFrames: number,
    captureStrideFrames: number,
    captureWidth: number,
    captureHeight: number,
    referenceDiffStrideFrames: number,
    referenceDiffLimit: number,
    sampleIndex: number,
    capturedFrameCount: number,
  ): Promise<{
    sample: RouteExperienceFrameSample;
    capturedFrame: CapturedBenchmarkFrame | null;
  }> {
    const gameplayStartedAt = performance.now();
    const movementStartedAt = performance.now();
    teleportPlayerToFeetPosition(this.player, target.feetPosition);
    this.player.grounded = true;
    this.camera.yaw = target.yaw;
    this.camera.pitch = target.pitch;
    this.syncCameraToPlayer();
    const movementMs = performance.now() - movementStartedAt;
    this.syncWorldAroundPlayer(false);
    const render = this.renderCurrentFrame();
    const gameplayFrameMs = performance.now() - gameplayStartedAt;
    return this.captureRouteExperienceFrameSample(
      target,
      movementMs,
      gameplayFrameMs,
      seamProbeStrideFrames,
      captureStrideFrames,
      captureWidth,
      captureHeight,
      referenceDiffStrideFrames,
      referenceDiffLimit,
      sampleIndex,
      capturedFrameCount,
      render,
    );
  }

  private async captureRouteExperienceFrameSample(
    target: {
      frame: number;
      phase: "move" | "settle";
      simTimeSeconds: number;
      distanceMeters: number;
    },
    movementMs: number,
    gameplayFrameMs: number,
    seamProbeStrideFrames: number,
    captureStrideFrames: number,
    captureWidth: number,
    captureHeight: number,
    referenceDiffStrideFrames: number,
    referenceDiffLimit: number,
    sampleIndex: number,
    capturedFrameCount: number,
    render: {
      frameStats: RenderStats;
      frameCpuMs: number;
    } | null = null,
  ): Promise<{
    sample: RouteExperienceFrameSample;
    capturedFrame: CapturedBenchmarkFrame | null;
  }> {
    const residency = this.lastStreamSummary;
    const dirtyResidentMeshes = summarizeDirtyResidentMeshes(this.world);
    const frameProbe = render ?? {
      frameStats: zeroRenderStats(),
      frameCpuMs: 0,
    };
    const diagnosticsStartedAt = performance.now();
    const detailCoverage = this.probeRenderReadyCoverage();
    const visibleGround = this.probeVisibleGroundCoverage();
    const shouldProbeSeams = target.frame % seamProbeStrideFrames === 0;
    const surfaceContinuity = shouldProbeSeams
      ? this.probeSurfaceContinuity()
      : {
          edgeCount: 0,
          missingSmoothEdgeCount: 0,
          abruptEdgeCount: 0,
          maxExpectedStepMeters: 0,
        };
    let screenVoid: BottomCenterVoidProbe | null = null;
    let capturedFrame: CapturedBenchmarkFrame | null = null;
    let captureDiagnosticsMs = 0;
    const shouldCaptureVoid = target.frame % captureStrideFrames === 0 || visibleGround.uncoveredCount > 0;
    const seamCoverage = shouldProbeSeams
      ? summarizeRouteSeamCoverage(this.probeLodCoverage(48, 1.6))
      : {
          seamGapCount: 0,
          uncoveredGapCount: 0,
          handoffHoleCount: 0,
          lodOverlapCount: 0,
          residentOverlapCount: 0,
          bandOverlapCount: 0,
          maxSeamGapMeters: 0,
          maxLodOverlapMeters: 0,
        };
    const shouldCaptureReference = referenceDiffStrideFrames > 0
      && capturedFrameCount < referenceDiffLimit
      && target.frame % referenceDiffStrideFrames === 0;
    if (shouldCaptureVoid || shouldCaptureReference) {
      const captureStartedAt = performance.now();
      const image = await this.captureRouteFrameImage(captureWidth, captureHeight);
      screenVoid = image ? analyzeBottomCenterVoid(image) : null;
      if (image && shouldCaptureReference) {
        capturedFrame = {
          sampleIndex,
          target: {
            frame: target.frame,
            phase: target.phase,
            simTimeSeconds: target.simTimeSeconds,
            distanceMeters: target.distanceMeters,
            feetPosition: [...this.player.feetPosition],
            yaw: this.camera.yaw,
            pitch: this.camera.pitch,
          },
          image,
        };
      }
      captureDiagnosticsMs = performance.now() - captureStartedAt;
    }
    const diagnosticsMs = performance.now() - diagnosticsStartedAt;
    const renderOtherMs = Math.max(
      0,
      frameProbe.frameCpuMs
        - frameProbe.frameStats.syncResourcesMs
        - frameProbe.frameStats.uploadMs
        - frameProbe.frameStats.encodeMs,
    );
    const accountedFrameMs = movementMs
      + residency.elapsedMs
      + this.lastMeshBuildSummary.elapsedMs
      + this.lastFrameLodMs
      + frameProbe.frameCpuMs;
    const unmeasuredFrameMs = Math.max(0, gameplayFrameMs - accountedFrameMs);
    const suspiciousHole = visibleGround.uncoveredCount > 0
      || (screenVoid?.suspicious ?? false);

    return {
      sample: {
        frame: target.frame,
        phase: target.phase,
        simTimeSeconds: target.simTimeSeconds,
        routeDistanceMeters: target.distanceMeters,
        feetPosition: [...this.player.feetPosition],
        yaw: this.camera.yaw,
        pitch: this.camera.pitch,
        changed: residency.changed,
        complete: residency.complete,
        pendingChunks: residency.pendingChunks,
        pendingMeshJobs: this.asyncChunkMeshing?.getPendingCount() ?? 0,
        dirtyResidentChunks: dirtyResidentMeshes.dirtyResidentChunks,
        dirtyMeshlessResidentChunks: dirtyResidentMeshes.dirtyMeshlessResidentChunks,
        dirtyRetainedMeshResidentChunks: dirtyResidentMeshes.dirtyRetainedMeshResidentChunks,
        generatedChunks: residency.generatedChunks,
        evictedChunks: residency.evictedChunks,
        movementMs,
        streamMs: residency.elapsedMs,
        meshMs: this.lastMeshBuildSummary.elapsedMs,
        meshCount: this.lastMeshBuildSummary.meshCount,
        lodMs: this.lastFrameLodMs,
        lodGeneratedChunks: this.lastLodSummary.generated,
        lodPendingChunks: this.lastLodSummary.pending,
        gameplayFrameMs,
        accountedFrameMs,
        unmeasuredFrameMs,
        diagnosticsMs,
        captureDiagnosticsMs,
        renderCpuMs: frameProbe.frameCpuMs,
        renderSyncMs: frameProbe.frameStats.syncResourcesMs,
        renderUploadMs: frameProbe.frameStats.uploadMs,
        renderEncodeMs: frameProbe.frameStats.encodeMs,
        renderOtherMs,
        uploadChunks: frameProbe.frameStats.uploadChunks,
        uploadBytes: frameProbe.frameStats.uploadBytes,
        drawCalls: frameProbe.frameStats.drawCalls,
        triangles: frameProbe.frameStats.triangles,
        residentNearSamples: detailCoverage.residentSampleCount,
        renderReadyNearSamples: detailCoverage.renderReadySampleCount,
        residentNotReadyNearSamples: detailCoverage.residentNotReadyCount,
        visibleGroundSampleCount: visibleGround.sampleCount,
        visibleGroundUncoveredCount: visibleGround.uncoveredCount,
        visibleGroundResidentNotReadyCount: visibleGround.residentNotReadyCount,
        surfaceContinuityEdgeCount: surfaceContinuity.edgeCount,
        surfaceContinuityGapCount: surfaceContinuity.missingSmoothEdgeCount,
        abruptSurfaceEdgeCount: surfaceContinuity.abruptEdgeCount,
        maxSurfaceContinuityStepMeters: surfaceContinuity.maxExpectedStepMeters,
        farLodCoverageGapCount: seamCoverage.seamGapCount,
        lodMaxChunkMs: this.lastLodSummary.maxChunkMs,
        lodMaxChunkLevel: this.lastLodSummary.maxChunkLevel,
        lodMaxChunkKey: this.lastLodSummary.maxChunkKey,
        uncoveredFarLodGapCount: seamCoverage.uncoveredGapCount,
        handoffFarLodHoleCount: seamCoverage.handoffHoleCount,
        maxFarLodCoverageGapMeters: seamCoverage.maxSeamGapMeters,
        seamGapCount: seamCoverage.seamGapCount,
        uncoveredLodGapCount: seamCoverage.uncoveredGapCount,
        handoffLodHoleCount: seamCoverage.handoffHoleCount,
        maxSeamGapMeters: seamCoverage.maxSeamGapMeters,
        lodOverlapCount: seamCoverage.lodOverlapCount,
        lodResidentOverlapCount: seamCoverage.residentOverlapCount,
        lodBandOverlapCount: seamCoverage.bandOverlapCount,
        maxLodOverlapMeters: seamCoverage.maxLodOverlapMeters,
        screenVoidRatio: screenVoid?.clearRatio ?? null,
        screenVoidMaxRunRatio: screenVoid?.maxClearRunRatio ?? null,
        screenVoidSuspicious: screenVoid?.suspicious ?? false,
        settledReferenceChangedRatio: null,
        settledReferenceClearToFilledRatio: null,
        settledReferenceMaxClearToFilledRunRatio: null,
        settledReferenceSuspiciousHole: false,
        suspiciousHole,
      },
      capturedFrame,
    };
  }

  private async captureRouteFrameImage(width: number, height: number): Promise<{
    width: number;
    height: number;
    pixels: Uint8ClampedArray;
  } | null> {
    if (!this.renderer) {
      return null;
    }
    const cameraMatrices = buildFirstPersonCameraMatrices(this.camera, width / height);
    const renderEnvironment = this.resolveRenderEnvironment();
    return await this.renderer.captureImage(
      this.world,
      cameraMatrices,
      width,
      height,
      renderEnvironment,
    );
  }

  private resolveRenderEnvironment(): RenderEnvironment {
    const currentWorld = this.sampleCurrentWorldContext();
    const ambientEnvironment = buildAmbientRenderEnvironment(currentWorld.ambientProfile);
    if (!this.player.eyeInWater) {
      const encounter = sampleRpgEncounterWorldUnits(this.player.feetPosition[0], this.player.feetPosition[2]);
      const worldSystems = this.sampleWorldSystems(currentWorld, encounter);
      return applyWorldAtmosphere(ambientEnvironment, worldSystems.clock, worldSystems.weather);
    }
    const eye = getPlayerEyePosition(this.player);
    const material = this.world.getVoxel(
      Math.floor(eye[0]),
      Math.floor(eye[1]),
      Math.floor(eye[2]),
    );
    if (!this.world.isWaterMaterial(material)) {
      return ambientEnvironment;
    }
    return buildUnderwaterRenderEnvironment(this.world.getPaletteColor(material));
  }

  private sampleWorldSystems(
    currentWorld: CurrentWorldProbeContext,
    encounter: RpgEncounterSample,
  ): WorldSystemSnapshot {
    return sampleWorldSystems(
      (performance.now() - this.worldClockStartedAt) / 1000,
      currentWorld.probe,
      currentWorld.ambientProfile,
      encounter,
      this.lastTravelContext,
    );
  }

  private selectActiveQuestHook(
    currentWorld: CurrentWorldProbeContext,
    discovery: ExplorationJournalSnapshot,
    routeSnapshot: RouteJournalSnapshot,
    encounter: RpgEncounterSample,
    primaryFaction: string | null,
  ): RpgQuestHookSummary | null {
    if (!encounter.regionId) {
      return null;
    }
    const landmarkId = currentWorld.probe.landmarkId ?? discovery.currentLandmarkId;
    const plan = planRpgQuestHooks({
      regionId: encounter.regionId,
      routeId: encounter.routeId,
      landmarkId,
    });
    return selectRpgQuestHookForExploration(plan, {
      nearCave: encounter.caveSystemId !== null || this.lastTravelContext === "underground",
      hasFaction: primaryFaction !== null,
      hasLandmark: landmarkId !== null,
      completedObjectiveIdsByHookId: Object.fromEntries(routeSnapshot.goals.map((goal) => [
        goal.id,
        goal.completedStepIds,
      ])),
    });
  }

  private sampleCurrentWorldContext(): CurrentWorldProbeContext {
    const centerX = Math.floor(this.player.feetPosition[0]);
    const centerZ = Math.floor(this.player.feetPosition[2]);
    const probe = this.generator.sampleBiomeProbe(centerX, centerZ);
    const observedUndergroundBiomeId = resolveObservedUndergroundBiomeId(
      this.world,
      this.camera.position,
      probe.surfaceY,
      probe.undergroundBiomeId,
    );
    return {
      probe,
      observedUndergroundBiomeId,
      ambientProfile: resolveAmbientWorldProfile(probe, { observedUndergroundBiomeId }),
    };
  }

  private async applySettledReferenceDiffs(
    samples: RouteExperienceFrameSample[],
    capturedFrames: readonly CapturedBenchmarkFrame[],
  ): Promise<void> {
    for (const capturedFrame of capturedFrames) {
      const referenceImage = await this.captureSettledReferenceFrame(capturedFrame.target, capturedFrame.image.width, capturedFrame.image.height);
      if (!referenceImage) {
        continue;
      }
      const diff = analyzeSettledReferenceDiff(capturedFrame.image, referenceImage);
      const sample = samples[capturedFrame.sampleIndex];
      if (!sample) {
        continue;
      }
      sample.settledReferenceChangedRatio = diff.changedRatio;
      sample.settledReferenceClearToFilledRatio = diff.clearToFilledRatio;
      sample.settledReferenceMaxClearToFilledRunRatio = diff.maxClearToFilledRunRatio;
      sample.settledReferenceSuspiciousHole = diff.suspiciousHole;
      sample.suspiciousHole = sample.suspiciousHole || diff.suspiciousHole;
    }
  }

  private async captureSettledReferenceFrame(
    target: CapturedBenchmarkFrame["target"],
    width: number,
    height: number,
  ): Promise<{
    width: number;
    height: number;
    pixels: Uint8ClampedArray;
  } | null> {
    teleportPlayerToFeetPosition(this.player, target.feetPosition);
    this.player.grounded = true;
    this.camera.yaw = target.yaw;
    this.camera.pitch = target.pitch;
    this.syncCameraToPlayer();
    this.syncWorldAroundPlayer(true);
    return await this.captureRouteFrameImage(width, height);
  }

  private async renderProbeFrame(): Promise<GameRenderProbe> {
    const rendered = this.renderCurrentFrame();
    if (!rendered || !this.renderer) {
      return zeroGameRenderProbe();
    }
    await this.renderer.waitForGpuIdle();
    return {
      frameCpuMs: rendered.frameCpuMs,
      syncResourcesMs: rendered.frameStats.syncResourcesMs,
      uploadMs: rendered.frameStats.uploadMs,
      uploadChunks: rendered.frameStats.uploadChunks,
      uploadBytes: rendered.frameStats.uploadBytes,
      encodeMs: rendered.frameStats.encodeMs,
      drawCalls: rendered.frameStats.drawCalls,
      triangles: rendered.frameStats.triangles,
    };
  }

  private recordBootstrapBenchmarkSample(gameplayFrameMs: number): void {
    if (this.bootstrapBenchmarkComplete) {
      return;
    }
    const bootstrap = this.getBootstrapReadiness();
    const becamePlayableReady = !this.bootstrapPlayableReady && bootstrap.playableReady;
    const becameVisualReady = !this.bootstrapBenchmarkComplete && bootstrap.visualReady;
    this.bootstrapBenchmarkSamples.push({
      frame: this.bootstrapBenchmarkSamples.length,
      elapsedMs: performance.now() - this.bootstrapBenchmarkStartedAt,
      gameplayFrameMs,
      renderCpuMs: this.lastFrameCpuMs,
      renderSyncMs: this.lastRenderStats.syncResourcesMs,
      renderUploadMs: this.lastRenderStats.uploadMs,
      renderEncodeMs: this.lastRenderStats.encodeMs,
      uploadChunks: this.lastRenderStats.uploadChunks,
      uploadBytes: this.lastRenderStats.uploadBytes,
      drawCalls: this.drawCalls,
      triangles: this.triangles,
      streamMs: this.lastStreamSummary.elapsedMs,
      meshMs: this.lastMeshBuildSummary.elapsedMs,
      lodMs: this.lastLodSummary.elapsedMs,
      lodYRangeMs: this.lastLodSummary.yRangeMs,
      lodDownsampleMs: this.lastLodSummary.downsampleMs,
      lodMeshMs: this.lastLodSummary.meshMs,
      pendingChunks: this.lastStreamSummary.pendingChunks,
      pendingMeshJobs: bootstrap.pendingMeshJobs,
      dirtyResidentChunks: bootstrap.dirtyResidentMeshes.dirtyResidentChunks,
      dirtyMeshlessResidentChunks: bootstrap.dirtyResidentMeshes.dirtyMeshlessResidentChunks,
      dirtyRetainedMeshResidentChunks: bootstrap.dirtyResidentMeshes.dirtyRetainedMeshResidentChunks,
      generatedChunks: this.lastStreamSummary.generatedChunks,
      evictedChunks: this.lastStreamSummary.evictedChunks,
      playableReady: bootstrap.playableReady,
      visualReady: bootstrap.visualReady,
      lodChunkCount: this.lastLodSummary.totalChunks,
      lodPendingChunks: this.lastLodSummary.pending,
      lodComplete: this.lastLodSummary.pending === 0 && this.lastLodSummary.totalChunks > 0,
      frustumCulledChunks: this.lastRenderStats.frustumCulledChunks,
      fogCulledChunks: this.lastRenderStats.fogCulledChunks,
      lodDrawCalls: this.lastRenderStats.lodDrawCalls,
      lodDrawCallsByLevel: this.lastRenderStats.lodDrawCallsByLevel,
    });
    if (bootstrap.playableReady) {
      this.bootstrapPlayableReady = true;
    }
    if (bootstrap.visualReady) {
      this.bootstrapBenchmarkComplete = true;
    }
    if (becamePlayableReady || becameVisualReady) {
      this.status = this.pointerLocked
        ? "Pointer locked: WASD move, Space jump, Ctrl sprint, Alt slow"
        : "Click once to capture cursor";
      this.pushHud(true);
    }
  }

  private getBootstrapReadiness(): BootstrapReadiness {
    const dirtyResidentMeshes = summarizeDirtyResidentMeshes(this.world);
    const pendingMeshJobs = this.asyncChunkMeshing?.getPendingCount() ?? 0;
    const hasResidentChunks = this.world.getStats().chunkCount > 0;
    const playerChunkX = Math.floor(this.player.feetPosition[0] / this.world.chunkSize);
    const playerChunkY = Math.floor(this.player.feetPosition[1] / this.world.chunkSize);
    const playerChunkZ = Math.floor(this.player.feetPosition[2] / this.world.chunkSize);
    const localMissingColumns = countMissingResidentColumnsAround(
      this.world,
      playerChunkX,
      playerChunkZ,
      BOOTSTRAP_PLAYABLE_COLUMN_RADIUS_CHUNKS,
    );
    const requiredColumns = countColumnsWithinRadius(BOOTSTRAP_PLAYABLE_COLUMN_RADIUS_CHUNKS);
    const urgentDirtyMeshlessChunks = countUrgentDirtyMeshlessChunks(
      this.world,
      playerChunkX,
      playerChunkY,
      playerChunkZ,
    );
    const supportChunk = this.world.getResidentChunk(playerChunkX, Math.floor((this.player.feetPosition[1] - 1) / this.world.chunkSize), playerChunkZ);
    const playableReady = hasResidentChunks
      && supportChunk !== null
      && localMissingColumns === 0
      && urgentDirtyMeshlessChunks === 0;
    const visualReady = playableReady;
    return {
      dirtyResidentMeshes,
      pendingMeshJobs,
      playableReady,
      visualReady,
      requiredColumns,
      readyColumns: Math.max(0, requiredColumns - localMissingColumns),
      urgentDirtyMeshlessChunks,
    };
  }

  private pushHud(force = false): void {
    const now = performance.now();
    if (!force && now - this.lastHudPushAt < HUD_PUSH_INTERVAL_MS) {
      return;
    }
    this.lastHudPushAt = now;
    this.onHudUpdate?.(this.getDebugSnapshot());
  }

  private refreshDiscoveryJournal(force = false): ExplorationJournalSnapshot {
    const now = performance.now();
    const currentFeetPosition = this.player.feetPosition;
    if (!force && this.lastDiscoverySampleFeetPosition) {
      const deltaX = currentFeetPosition[0] - this.lastDiscoverySampleFeetPosition[0];
      const deltaZ = currentFeetPosition[2] - this.lastDiscoverySampleFeetPosition[2];
      const movedFarEnough = Math.hypot(deltaX, deltaZ) >= DISCOVERY_SAMPLE_MOVE_THRESHOLD_WORLD_UNITS;
      if (!movedFarEnough && now - this.lastDiscoverySampleAt < DISCOVERY_SAMPLE_INTERVAL_MS) {
        return this.lastDiscoverySnapshot;
      }
    }
    this.lastDiscoverySampleAt = now;
    this.lastDiscoverySampleFeetPosition = [...currentFeetPosition] as Vec3;
    this.lastDiscoverySnapshot = this.explorationJournal.observe(this.sampleExplorationObservation());
    this.observePassiveRouteProgress(this.lastDiscoverySnapshot);
    return this.lastDiscoverySnapshot;
  }

  private observePassiveRouteProgress(discovery: ExplorationJournalSnapshot): void {
    const encounter = sampleRpgEncounterWorldUnits(this.player.feetPosition[0], this.player.feetPosition[2]);
    if (encounter.routeId) {
      this.observeTravelGoalProgress({
        routeId: encounter.routeId,
        kind: "visit",
        targetId: encounter.routeId,
      });
    }

    const landmarkId = discovery.currentLandmarkId;
    if (!landmarkId) {
      return;
    }
    const routeId = routeIdForLandmark(landmarkId);
    if (!routeId) {
      return;
    }
    this.observeTravelGoalProgress({
      routeId,
      kind: "visit",
      targetId: landmarkId,
    });
  }

  private observeTravelGoalProgress(input: TravelGoalProgressInput): TravelGoalProgressResult {
    const routeScopedResult = this.routeJournal.observeProgress(input);
    const result = input.routeId && !input.goalId
      ? mergeTravelGoalProgressResults(routeScopedResult, this.routeJournal.observeProgress({
        ...input,
        routeId: null,
      }))
      : routeScopedResult;
    this.recordCompletedTravelGoals(result);
    return result;
  }

  private observeActiveQuestStep(
    currentWorld: CurrentWorldProbeContext,
    discovery: ExplorationJournalSnapshot,
    encounter: RpgEncounterSample,
    primaryFaction: string | null,
    targetId: string | null,
    routeId: string | null,
    allowedKinds: readonly TravelGoalStepKind[],
  ): TravelGoalProgressResult | null {
    const activeQuest = this.selectActiveQuestHook(
      currentWorld,
      discovery,
      this.routeJournal.getSnapshot(),
      encounter,
      primaryFaction,
    );
    if (!activeQuest || !allowedKinds.includes(activeQuest.objectiveKind)) {
      return null;
    }
    if (!questObjectiveMatchesInteraction(activeQuest, targetId, routeId)) {
      return null;
    }
    return this.observeTravelGoalProgress({
      goalId: activeQuest.hookId,
      kind: activeQuest.objectiveKind,
      targetId: activeQuest.objectiveTargetId,
    });
  }

  private recordCompletedTravelGoals(result: TravelGoalProgressResult): void {
    for (const goalId of result.completedGoalIds) {
      const definition = TRAVEL_GOALS.find((goal) => goal.id === goalId);
      this.explorationEventLog.record({
        kind: "complete-travel-goal",
        subjectType: "route",
        subjectId: goalId,
        role: "travel-goal",
        name: definition?.title ?? goalId,
        flavorText: definition?.journalText ?? null,
        payload: {
          routeId: definition?.routeId ?? null,
          completedStepIds: result.completedStepIds,
        },
      });
    }
  }

  private sampleExplorationObservation(): ExplorationObservation {
    const centerX = Math.floor(this.player.feetPosition[0]);
    const centerZ = Math.floor(this.player.feetPosition[2]);
    const centerProbe = this.generator.sampleBiomeProbe(centerX, centerZ);
    const observedUndergroundBiomeId = resolveObservedUndergroundBiomeId(
      this.world,
      this.camera.position,
      centerProbe.surfaceY,
      centerProbe.undergroundBiomeId,
    );
    const explorationSkillEffects = resolveExplorationSkillEffects(this.skillJournal.getSnapshot());
    const landmarkIds: string[] = [];
    let currentLandmarkId: string | null = centerProbe.landmarkId;
    if (centerProbe.landmarkId) {
      landmarkIds.push(centerProbe.landmarkId);
    }
    for (const [offsetX, offsetZ] of buildLandmarkSampleOffsets(explorationSkillEffects)) {
      if (offsetX === 0 && offsetZ === 0) {
        continue;
      }
      const probe = this.generator.sampleBiomeProbe(centerX + offsetX, centerZ + offsetZ);
      if (!probe.landmarkId) {
        continue;
      }
      if (currentLandmarkId === null) {
        currentLandmarkId = probe.landmarkId;
      }
      landmarkIds.push(probe.landmarkId);
    }
    return {
      biomeId: centerProbe.biomeId,
      undergroundBiomeId: observedUndergroundBiomeId,
      regionalVariantId: centerProbe.regionalVariantId,
      landmarkIds,
      currentLandmarkId,
    };
  }
}

function toDegrees(value: number): number {
  return value * 180 / Math.PI;
}

function resolveExplorationSkillEffects(snapshot: SkillJournalSnapshot): ExplorationSkillEffects {
  return describeExplorationSkillEffects({
    cartographyLevel: readSkillLevel(snapshot, "cartography"),
    naturalistLevel: readSkillLevel(snapshot, "naturalist"),
    spelunkingLevel: readSkillLevel(snapshot, "spelunking"),
  });
}

function readSkillLevel(snapshot: SkillJournalSnapshot, skillId: SkillId): number {
  return snapshot.skills.find((skill) => skill.id === skillId)?.level ?? 1;
}

function buildLandmarkSampleOffsets(effects: ExplorationSkillEffects): ReadonlyArray<readonly [number, number]> {
  const cacheKey = `${effects.landmarkScanRadiusMeters.toFixed(3)}:${effects.landmarkScanSampleStepMeters.toFixed(3)}`;
  const cached = LANDMARK_SAMPLE_OFFSET_CACHE.get(cacheKey);
  if (cached) {
    return cached;
  }
  const radiusWorldUnits = metersToWorldUnits(effects.landmarkScanRadiusMeters);
  const stepWorldUnits = metersToWorldUnits(effects.landmarkScanSampleStepMeters);
  const maxStep = Math.max(0, Math.ceil(radiusWorldUnits / Math.max(1, stepWorldUnits)));
  const offsets: Array<readonly [number, number]> = [];
  for (let zStep = -maxStep; zStep <= maxStep; zStep += 1) {
    for (let xStep = -maxStep; xStep <= maxStep; xStep += 1) {
      const offsetX = xStep * stepWorldUnits;
      const offsetZ = zStep * stepWorldUnits;
      if (Math.hypot(offsetX, offsetZ) <= radiusWorldUnits + 0.001) {
        offsets.push([offsetX, offsetZ]);
      }
    }
  }
  const sorted = offsets.sort((left, right) => Math.hypot(left[0], left[1]) - Math.hypot(right[0], right[1]));
  LANDMARK_SAMPLE_OFFSET_CACHE.set(cacheKey, sorted);
  return sorted;
}

function buildLandmarkInteractionPrompts(
  landmarkId: string,
  role: DiscoveryRole,
): ExplorationInteractionCandidate["prompts"] {
  if (landmarkId === "velothi_shrine" || role === "shrine") {
    return [
      "inspect",
      { verb: "read", label: "Read the shrine etching", description: "Trace the pilgrim marks for a route clue." },
      { verb: "use", label: "Offer thanks", description: "Mark the shrine in your journal." },
    ];
  }
  if (landmarkId === "ashlander_travel_pack") {
    return [
      "inspect",
      { verb: "use", label: "Check the travel pack", description: "Look for a route note or useful bearing." },
    ];
  }
  if (role === "old-road") {
    return [
      { verb: "inspect", label: `Inspect ${formatDiscoveryName("landmark", landmarkId)}`, description: "Study the old road sign for your route journal." },
      { verb: "read", label: "Read the road marks", description: "Decode scratches left by earlier travelers." },
    ];
  }
  return ["inspect"];
}

function routeIdForLandmark(landmarkId: string): string | null {
  if (landmarkId === "ash_marker") {
    return "ash-road";
  }
  return ROUTE_LANDMARK_IDS.has(landmarkId) ? "pilgrim-road" : null;
}

function formatEncounterMoodForTravelContext(moodLabel: string, travelContext: "surface" | "underground"): string {
  if (travelContext === "surface" && moodLabel === "Cave Threshold") {
    return "Cave Rumor";
  }
  return moodLabel;
}

function formatEncounterFlavorTag(tag: string): string {
  return tag
    .split("-")
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

function readPayloadRouteId(payload: unknown): string | null {
  if (!payload || typeof payload !== "object" || Array.isArray(payload)) {
    return null;
  }
  const routeId = (payload as Record<string, unknown>).routeId;
  return typeof routeId === "string" && routeId.trim().length > 0 ? routeId : null;
}

function buildCaveMouthInteractionCandidate(
  currentWorld: CurrentWorldProbeContext,
  encounter: RpgEncounterSample,
): ExplorationInteractionCandidate | null {
  const fields = currentWorld.probe.fields;
  if (
    fields.atlasCaveAnchorKind !== "entrance"
    || (fields.atlasCaveCore ?? 0) < CAVE_MOUTH_INTERACTION_CORE_THRESHOLD
    || !fields.atlasCaveAnchorId
    || fields.atlasCaveAnchorX === null
    || fields.atlasCaveAnchorX === undefined
    || fields.atlasCaveAnchorZ === null
    || fields.atlasCaveAnchorZ === undefined
  ) {
    return null;
  }

  const caveSystemId = fields.atlasCaveSystemId ?? encounter.caveSystemId ?? "local-cave";
  const caveName = `${formatCaveSystemName(caveSystemId)} Mouth`;
  const undergroundName = formatDiscoveryName("underground", currentWorld.probe.undergroundBiomeId, "underground");
  const scoutResult = describeRpgEncounterScoutResult(encounter);
  return {
    id: `cave-mouth:${fields.atlasCaveAnchorId}`,
    subjectType: "zone",
    name: caveName,
    role: "cave-mouth",
    worldPosition: [
      fields.atlasCaveAnchorX,
      currentWorld.probe.surfaceY,
      fields.atlasCaveAnchorZ,
    ],
    interactionRadiusMeters: metersToWorldUnits(6),
    priority: 12,
    prompts: [{
      verb: "use",
      label: `Enter ${caveName}`,
      description: `${undergroundName} begins here. ${scoutResult.detail}`,
    }],
    flavorText: `${undergroundName} begins here. ${scoutResult.detail}`,
    skillAwards: [{
      skillId: "spelunking",
      xp: 24,
      reason: "Cave mouth scouted",
      awardKey: `cave-mouth:${fields.atlasCaveAnchorId}`,
      onceOnly: true,
    }],
    payload: {
      caveSystemId,
      caveAnchorId: fields.atlasCaveAnchorId,
      caveAnchorKind: fields.atlasCaveAnchorKind,
      undergroundBiomeId: currentWorld.probe.undergroundBiomeId,
      pressure: Number(encounter.pressure.toFixed(3)),
      moodId: encounter.moodId,
      regionId: encounter.regionId,
      routeId: encounter.routeId,
    },
  };
}

function countEventsBySubjectRole(
  snapshot: ExplorationEventLogSnapshot,
  subjectType: string,
  role: string,
): number {
  return snapshot.events.filter((event) => event.subjectType === subjectType && event.role === role).length;
}

function countMobSignEvents(snapshot: ExplorationEventLogSnapshot): number {
  const mobSignRoles = new Set(["mob-trail", "mob-spoor", "mob-nest", "mob-lair"]);
  return snapshot.events.filter((event) => event.subjectType === "mob" && event.role !== null && mobSignRoles.has(event.role)).length;
}

function formatPassiveMobPresenceLabel(sighting: PassiveMobSighting | null): string {
  if (!sighting) {
    return "No passive mob nearby";
  }
  return `${formatPassiveMobShortSpecies(sighting.speciesName)} ${formatPassiveMobDistance(worldUnitsToMeters(sighting.distanceWorldUnits))}`;
}

function formatPassiveMobDistance(distanceMeters: number): string {
  if (distanceMeters < 10) {
    return `${distanceMeters.toFixed(1)} m`;
  }
  if (distanceMeters < 1000) {
    return `${Math.round(distanceMeters)} m`;
  }
  return `${(distanceMeters / 1000).toFixed(1)} km`;
}

function formatPassiveMobShortSpecies(speciesName: string): string {
  if (speciesName.includes("Kwama")) return "Kwama";
  if (speciesName.includes("Guar")) return "Guar";
  if (speciesName.includes("Pilgrim")) return "Pilgrim";
  if (speciesName.includes("Runner")) return "Runner";
  if (speciesName.includes("Forager")) return "Forager";
  if (speciesName.includes("Sentry")) return "Sentry";
  if (speciesName.includes("Guide")) return "Guide";
  if (speciesName.includes("Vagrant")) return "Vagrant";
  if (speciesName.includes("Grazer")) return "Grazer";
  return speciesName;
}

function describeLootCandidatePrompt(fieldNote: string, state: LootJournalCandidateState): string {
  if (!state.collected) {
    return fieldNote;
  }
  if (state.match === "subject") {
    return state.lastNote ? `Already searched here. Last note: ${state.lastNote}` : "Already searched here.";
  }
  return `${fieldNote} Similar find recorded: ${state.lastNote ?? state.lootId ?? "known cache"}.`;
}

function formatLootJournalStateLabel(collectedCaches: number, revisitedCaches: number): string {
  if (collectedCaches === 0) {
    return "No caches collected";
  }
  return revisitedCaches > 0
    ? `${collectedCaches} caches • ${revisitedCaches} revisited`
    : `${collectedCaches} caches collected`;
}

function buildQuestTravelGoals(): readonly TravelGoalDefinition[] {
  return WORLD_ATLAS.routes.flatMap((route) => {
    const regionId = route.nodes[0]?.regionId ?? route.expectedRegionIds[0];
    if (!regionId) {
      return [];
    }
    const plan = planRpgQuestHooks({
      regionId,
      routeId: route.id,
    });
    return plan.hooks
      .map(buildTravelGoalFromQuestHook)
      .filter((goal): goal is TravelGoalDefinition => goal !== null);
  });
}

function mergeTravelGoalProgressResults(
  primary: TravelGoalProgressResult,
  secondary: TravelGoalProgressResult,
): TravelGoalProgressResult {
  return {
    changed: primary.changed || secondary.changed,
    completedGoalIds: uniqueStrings([...primary.completedGoalIds, ...secondary.completedGoalIds]),
    completedStepIds: uniqueStrings([...primary.completedStepIds, ...secondary.completedStepIds]),
    snapshot: secondary.snapshot,
  };
}

function uniqueStrings(values: readonly string[]): readonly string[] {
  return [...new Set(values)];
}

function questObjectiveMatchesInteraction(
  quest: RpgQuestHookSummary,
  targetId: string | null,
  routeId: string | null,
): boolean {
  switch (quest.objectiveKind) {
    case "listen":
      return true;
    case "inspect":
    case "interpret":
      return targetId === quest.objectiveTargetId;
    case "report":
    case "visit":
      return targetId === quest.objectiveTargetId || routeId === quest.objectiveTargetId;
  }
}

function formatCaveSystemName(caveSystemId: string): string {
  return caveSystemId
    .split(/[-_\s]+/g)
    .map((word) => word.trim())
    .filter(Boolean)
    .map((word) => `${word[0]?.toUpperCase() ?? ""}${word.slice(1)}`)
    .join(" ");
}

function selectActiveTravelGoal(snapshot: RouteJournalSnapshot): TravelGoalSnapshot | null {
  return snapshot.goals.find((goal) => goal.status === "active")
    ?? snapshot.goals.find((goal) => !goal.completed)
    ?? snapshot.goals[snapshot.goals.length - 1]
    ?? null;
}

function findNextTravelGoalStep(goal: TravelGoalSnapshot): TravelGoalDefinition["steps"][number] | null {
  const definition = TRAVEL_GOALS.find((candidate) => candidate.id === goal.id);
  if (!definition) {
    return null;
  }
  return definition.steps.find((step) => !("optional" in step && step.optional === true) && !goal.completedStepIds.includes(step.id))
    ?? definition.steps.find((step) => !goal.completedStepIds.includes(step.id))
    ?? null;
}

function formatRouteName(routeId: string): string {
  switch (routeId) {
    case "pilgrim-road":
      return "Pilgrim Road";
    case "ash-road":
      return "Ash Road";
    default:
      return titleCaseRouteId(routeId);
  }
}

function titleCaseRouteId(routeId: string): string {
  return routeId
    .split(/[-_\s]+/)
    .filter(Boolean)
    .map((part) => part[0] ? `${part[0].toUpperCase()}${part.slice(1)}` : part)
    .join(" ");
}

function toChunkTuple(coord: { x: number; y: number; z: number }): [number, number, number] {
  return [coord.x, coord.y, coord.z];
}

function zeroRenderStats(): RenderStats {
  return {
    drawCalls: 0,
    triangles: 0,
    syncResourcesMs: 0,
    uploadMs: 0,
    uploadChunks: 0,
    uploadBytes: 0,
    encodeMs: 0,
    frustumCulledChunks: 0,
    fogCulledChunks: 0,
    lodDrawCalls: 0,
    lodDrawCallsByLevel: [0, 0, 0, 0, 0],
  };
}

function zeroGameRenderProbe(): GameRenderProbe {
  return {
    frameCpuMs: 0,
    syncResourcesMs: 0,
    uploadMs: 0,
    uploadChunks: 0,
    uploadBytes: 0,
    encodeMs: 0,
    drawCalls: 0,
    triangles: 0,
  };
}

function createZeroFrameAttribution(): GameFrameAttribution {
  return createFrameAttribution({
    frame: 0,
    wallMs: 0,
    gameplayMs: 0,
    movementMs: 0,
    streamMs: 0,
    meshMs: 0,
    lodMs: 0,
    renderCpuMs: 0,
    renderSyncMs: 0,
    renderUploadMs: 0,
    renderEncodeMs: 0,
  });
}

function createFrameAttribution(input: Omit<GameFrameAttribution, "cause">): GameFrameAttribution {
  const candidates: Array<[cause: string, ms: number]> = [
    ["stream", input.streamMs],
    ["mesh", input.meshMs],
    ["LOD", input.lodMs],
    ["render", input.renderCpuMs],
    ["GPU upload", input.renderUploadMs],
    ["GPU sync", input.renderSyncMs],
    ["movement", input.movementMs],
  ];
  candidates.sort((left, right) => right[1] - left[1]);
  const [cause, ms] = candidates[0] ?? ["none", 0];
  return {
    ...input,
    cause: ms > 0.05 ? cause : "none",
  };
}

function shouldSyncBuildUrgentChunk(
  chunk: { coord: { x: number; y: number; z: number }; meshBuilt: boolean },
  priorityChunkX: number,
  priorityChunkY: number,
  priorityChunkZ: number,
): boolean {
  if (chunk.meshBuilt) {
    return false;
  }
  const planarDistance = Math.max(
    Math.abs(chunk.coord.x - priorityChunkX),
    Math.abs(chunk.coord.z - priorityChunkZ),
  );
  if (planarDistance > SYNC_NEAR_MESH_RADIUS_CHUNKS) {
    return false;
  }
  return Math.abs(chunk.coord.y - priorityChunkY) <= 1;
}

function buildEyePositionForChunkCenter(
  chunkX: number,
  chunkZ: number,
  feetY: number,
  chunkSize: number,
  eyeHeight: number,
): Vec3 {
  return [
    chunkX * chunkSize + chunkSize * 0.5,
    feetY + eyeHeight,
    chunkZ * chunkSize + chunkSize * 0.5,
  ];
}

function buildFeetPositionForChunkCenter(
  chunkX: number,
  chunkZ: number,
  feetY: number,
  chunkSize: number,
): Vec3 {
  return [
    chunkX * chunkSize + chunkSize * 0.5,
    feetY,
    chunkZ * chunkSize + chunkSize * 0.5,
  ];
}

function teleportPlayerToFeetPosition(player: PlayerState, feetPosition: Vec3): void {
  player.feetPosition = [...feetPosition];
  player.velocity = [0, 0, 0];
}

function lerpVec3(from: Vec3, to: Vec3, t: number): Vec3 {
  return [
    from[0] + (to[0] - from[0]) * t,
    from[1] + (to[1] - from[1]) * t,
    from[2] + (to[2] - from[2]) * t,
  ];
}

function buildIncrementalSample(
  frame: number,
  phase: "move" | "settle",
  leg: number,
  residency: ResidencyUpdateSummary,
  mesh: MeshBuildSummary,
  detailCoverage: RenderReadyCoverageProbe,
  render: GameRenderProbe,
): IncrementalCrossingSample {
  return {
    frame,
    phase,
    leg,
    changed: residency.changed,
    complete: residency.complete,
    pendingChunks: residency.pendingChunks,
    generatedChunks: residency.generatedChunks,
    evictedChunks: residency.evictedChunks,
    streamMs: residency.elapsedMs,
    meshMs: mesh.elapsedMs,
    meshCount: mesh.meshCount,
    residentNearSamples: detailCoverage.residentSampleCount,
    renderReadyNearSamples: detailCoverage.renderReadySampleCount,
    residentNotReadyNearSamples: detailCoverage.residentNotReadyCount,
    frameCpuMs: render.frameCpuMs,
    syncMs: render.syncResourcesMs,
    uploadMs: render.uploadMs,
    uploadChunks: render.uploadChunks,
    uploadBytes: render.uploadBytes,
    encodeMs: render.encodeMs,
  };
}

function pushIssueSample(target: LodCoverageIssueSample[], sample: LodCoverageIssueSample): void {
  if (target.length >= 8) {
    return;
  }
  target.push(sample);
}

function isCoveredByLodSpans(
  worldX: number,
  worldZ: number,
  lodSpans: readonly LodCoverageSpan[],
): boolean {
  for (const span of lodSpans) {
    if (
      worldX >= span.minX
      && worldX < span.maxX
      && worldZ >= span.minZ
      && worldZ < span.maxZ
      && span.classifyColumn(worldX, worldZ).covered
    ) {
      return true;
    }
  }
  return false;
}

function formatLodOwnerBand(chunk: VoxelChunk): string {
  return `LOD${chunk.lodLevel}:${chunk.coord.x}:${chunk.coord.z}`;
}

function formatLodOwnerChunk(chunk: VoxelChunk): string {
  return `LOD${chunk.lodLevel}:${chunk.coord.x}:${chunk.coord.y}:${chunk.coord.z}`;
}

function lodCoverageRangesHaveVerticalOverlap(ranges: readonly LodCoverageVerticalRange[]): boolean {
  for (let leftIndex = 0; leftIndex < ranges.length; leftIndex += 1) {
    const left = ranges[leftIndex]!;
    for (let rightIndex = leftIndex + 1; rightIndex < ranges.length; rightIndex += 1) {
      const right = ranges[rightIndex]!;
      if (left.band === right.band) {
        continue;
      }
      if (left.minY < right.maxY && left.maxY > right.minY) {
        return true;
      }
    }
  }
  return false;
}

function positiveModulo(value: number, divisor: number): number {
  return ((value % divisor) + divisor) % divisor;
}

function cloneResidencySummary(summary: ResidencyUpdateSummary): ResidencyUpdateSummary {
  return {
    ...summary,
    generatedChunkCoords: summary.generatedChunkCoords.map((coord) => ({ ...coord })),
    evictedChunkCoords: summary.evictedChunkCoords.map((coord) => ({ ...coord })),
    phaseMs: { ...summary.phaseMs },
  };
}

function createIdleResidencySummary(
  summary: ResidencyUpdateSummary,
  anchor: StreamAnchor,
  radiusChunks: number,
  residentChunks: number,
  dirtyResidentChunks: number,
): ResidencyUpdateSummary {
  return {
    ...summary,
    changed: false,
    complete: true,
    centerChunkX: anchor.chunkX,
    centerChunkZ: anchor.chunkZ,
    radiusChunks,
    generatedChunks: 0,
    evictedChunks: 0,
    pendingChunks: 0,
    emptyChunksSkipped: 0,
    cachedEmptyChunkHits: 0,
    touchedNeighborChunks: 0,
    residentChunks,
    dirtyResidentChunks,
    elapsedMs: 0,
    generatedChunkCoords: [],
    evictedChunkCoords: [],
    phaseMs: zeroResidencyPhaseMetrics(),
  };
}

function summarizeChunkBoundaryBenchmark(samples: readonly ChunkBoundaryBenchmarkSample[]): ChunkBoundaryBenchmarkSummary {
  const streamSamples = samples.map((sample) => sample.streamMs);
  const meshSamples = samples.map((sample) => sample.meshMs);
  const frameCpuSamples = samples.map((sample) => sample.frameCpuMs);
  const syncSamples = samples.map((sample) => sample.syncMs);
  const uploadSamples = samples.map((sample) => sample.uploadMs);
  const uploadChunkSamples = samples.map((sample) => sample.uploadChunks);
  const uploadByteSamples = samples.map((sample) => sample.uploadBytes);
  return {
    sampleCount: samples.length,
    changedCount: samples.filter((sample) => sample.changed).length,
    avgStreamMs: average(streamSamples),
    p95StreamMs: percentile(streamSamples, 0.95),
    maxStreamMs: maxValue(streamSamples),
    avgMeshMs: average(meshSamples),
    p95MeshMs: percentile(meshSamples, 0.95),
    maxMeshMs: maxValue(meshSamples),
    avgFrameCpuMs: average(frameCpuSamples),
    p95FrameCpuMs: percentile(frameCpuSamples, 0.95),
    maxFrameCpuMs: maxValue(frameCpuSamples),
    avgSyncMs: average(syncSamples),
    p95SyncMs: percentile(syncSamples, 0.95),
    maxSyncMs: maxValue(syncSamples),
    avgUploadMs: average(uploadSamples),
    p95UploadMs: percentile(uploadSamples, 0.95),
    maxUploadMs: maxValue(uploadSamples),
    avgUploadChunks: average(uploadChunkSamples),
    maxUploadChunks: maxValue(uploadChunkSamples),
    avgUploadBytes: average(uploadByteSamples),
    maxUploadBytes: maxValue(uploadByteSamples),
  };
}

function summarizeIncrementalCrossing(samples: readonly IncrementalCrossingSample[]): IncrementalCrossingSummary {
  const workSamples = samples.map((sample) => sample.streamMs + sample.meshMs + sample.frameCpuMs);
  const streamSamples = samples.map((sample) => sample.streamMs);
  const meshSamples = samples.map((sample) => sample.meshMs);
  const frameCpuSamples = samples.map((sample) => sample.frameCpuMs);
  const uploadSamples = samples.map((sample) => sample.uploadMs);
  const residentNotReadySamples = samples.map((sample) => sample.residentNotReadyNearSamples);
  return {
    sampleCount: samples.length,
    workFrameCount: samples.filter((sample) =>
      sample.streamMs > 0
      || sample.meshMs > 0
      || sample.uploadChunks > 0
    ).length,
    changedCount: samples.filter((sample) => sample.changed).length,
    incompleteFrameCount: samples.filter((sample) =>
      !sample.complete || sample.pendingChunks > 0
    ).length,
    avgWorkMs: average(workSamples),
    p95WorkMs: percentile(workSamples, 0.95),
    maxWorkMs: maxValue(workSamples),
    avgResidentNotReadyNearSamples: average(residentNotReadySamples),
    maxResidentNotReadyNearSamples: maxValue(residentNotReadySamples),
    avgStreamMs: average(streamSamples),
    p95StreamMs: percentile(streamSamples, 0.95),
    maxStreamMs: maxValue(streamSamples),
    avgMeshMs: average(meshSamples),
    p95MeshMs: percentile(meshSamples, 0.95),
    maxMeshMs: maxValue(meshSamples),
    avgFrameCpuMs: average(frameCpuSamples),
    p95FrameCpuMs: percentile(frameCpuSamples, 0.95),
    maxFrameCpuMs: maxValue(frameCpuSamples),
    avgUploadMs: average(uploadSamples),
    p95UploadMs: percentile(uploadSamples, 0.95),
    maxUploadMs: maxValue(uploadSamples),
    maxPendingChunks: maxValue(samples.map((sample) => sample.pendingChunks)),
  };
}

function summarizeRouteExperienceBenchmark(
  samples: readonly RouteExperienceFrameSample[],
  plan: {
    totalDistanceMeters: number;
    sampleHz: number;
    speedMetersPerSecond: number;
  },
): RouteExperienceBenchmarkSummary {
  const accounting = summarizeRouteFrameAccounting(samples.map((sample) => ({
    gameplayFrameMs: sample.gameplayFrameMs,
    movementMs: sample.movementMs,
    streamMs: sample.streamMs,
    meshMs: sample.meshMs,
    lodMs: sample.lodMs,
    renderCpuMs: sample.renderCpuMs,
  })));
  const streamSamples = samples.map((sample) => sample.streamMs);
  const meshSamples = samples.map((sample) => sample.meshMs);
  const lodSamples = samples.map((sample) => sample.lodMs);
  const lodChunkSamples = samples.map((sample) => sample.lodMaxChunkMs);
  const renderCpuSamples = samples.map((sample) => sample.renderCpuMs);
  const renderOtherSamples = samples.map((sample) => sample.renderOtherMs);
  const residentNotReadySamples = samples.map((sample) => sample.residentNotReadyNearSamples);
  const visibleGroundUncoveredSamples = samples.map((sample) => sample.visibleGroundUncoveredCount);
  const visibleGroundResidentNotReadySamples = samples.map((sample) => sample.visibleGroundResidentNotReadyCount);
  const surfaceContinuityGapSamples = samples.map((sample) => sample.surfaceContinuityGapCount);
  const surfaceContinuityStepSamples = samples.map((sample) => sample.maxSurfaceContinuityStepMeters);
  const farLodCoverageDistanceSamples = samples.map((sample) => sample.maxFarLodCoverageGapMeters);
  const diagnosticsSamples = samples.map((sample) => sample.diagnosticsMs);
  const captureDiagnosticsSamples = samples.map((sample) => sample.captureDiagnosticsMs);
  const settledReferenceChangedSamples = samples.map((sample) => sample.settledReferenceChangedRatio ?? 0);
  const settledReferenceClearToFilledSamples = samples.map((sample) => sample.settledReferenceClearToFilledRatio ?? 0);
  const settledReferenceClearToFilledRunSamples = samples.map((sample) =>
    sample.settledReferenceMaxClearToFilledRunRatio ?? 0);
  const seamFrameClasses = countRouteSeamFrameClasses(samples);
  const settleCompletion = samples.find((sample) =>
    sample.phase === "settle"
    && sample.complete
    && sample.pendingChunks === 0
    && sample.dirtyResidentChunks === 0
    && sample.visibleGroundUncoveredCount === 0);

  return {
    sampleCount: samples.length,
    moveFrameCount: samples.filter((sample) => sample.phase === "move").length,
    settleFrameCount: samples.filter((sample) => sample.phase === "settle").length,
    incompleteFrameCount: samples.filter((sample) =>
      !sample.complete
      || sample.pendingChunks > 0
      || sample.dirtyResidentChunks > 0).length,
    totalDistanceMeters: plan.totalDistanceMeters,
    sampleHz: plan.sampleHz,
    speedMetersPerSecond: plan.speedMetersPerSecond,
    totalGameplayFrameMs: accounting.totalGameplayFrameMs,
    totalAccountedFrameMs: accounting.totalAccountedMs,
    totalUnmeasuredFrameMs: accounting.totalUnmeasuredMs,
    unmeasuredFrameRatio: accounting.totalGameplayFrameMs === 0
      ? 0
      : accounting.totalUnmeasuredMs / accounting.totalGameplayFrameMs,
    totalDiagnosticsMs: sumNumbers(diagnosticsSamples),
    totalCaptureDiagnosticsMs: sumNumbers(captureDiagnosticsSamples),
    avgGameplayFrameMs: accounting.avgGameplayFrameMs,
    p95GameplayFrameMs: accounting.p95GameplayFrameMs,
    maxGameplayFrameMs: accounting.maxGameplayFrameMs,
    framesOver16_67Ms: samples.filter((sample) => sample.gameplayFrameMs > 16.67).length,
    framesOver33_33Ms: samples.filter((sample) => sample.gameplayFrameMs > 33.33).length,
    framesOver50Ms: samples.filter((sample) => sample.gameplayFrameMs > 50).length,
    moveFramesOver50Ms: samples.filter((sample) => sample.phase === "move" && sample.gameplayFrameMs > 50).length,
    settleFramesOver50Ms: samples.filter((sample) => sample.phase === "settle" && sample.gameplayFrameMs > 50).length,
    avgMovementMs: accounting.avgMovementMs,
    p95MovementMs: accounting.p95MovementMs,
    maxMovementMs: accounting.maxMovementMs,
    avgMeasuredWorkMs: accounting.avgMeasuredWorkMs,
    p95MeasuredWorkMs: accounting.p95MeasuredWorkMs,
    maxMeasuredWorkMs: accounting.maxMeasuredWorkMs,
    avgUnmeasuredFrameMs: accounting.avgUnmeasuredMs,
    p95UnmeasuredFrameMs: accounting.p95UnmeasuredMs,
    maxUnmeasuredFrameMs: accounting.maxUnmeasuredMs,
    avgStreamMs: average(streamSamples),
    p95StreamMs: percentile(streamSamples, 0.95),
    maxStreamMs: maxValue(streamSamples),
    avgMeshMs: average(meshSamples),
    p95MeshMs: percentile(meshSamples, 0.95),
    maxMeshMs: maxValue(meshSamples),
    avgLodMs: average(lodSamples),
    p95LodMs: percentile(lodSamples, 0.95),
    maxLodMs: maxValue(lodSamples),
    p95LodChunkMs: percentile(lodChunkSamples, 0.95),
    maxLodChunkMs: maxValue(lodChunkSamples),
    avgRenderCpuMs: average(renderCpuSamples),
    p95RenderCpuMs: percentile(renderCpuSamples, 0.95),
    maxRenderCpuMs: maxValue(renderCpuSamples),
    avgRenderOtherMs: average(renderOtherSamples),
    maxRenderOtherMs: maxValue(renderOtherSamples),
    avgResidentNotReadyNearSamples: average(residentNotReadySamples),
    maxResidentNotReadyNearSamples: maxValue(residentNotReadySamples),
    avgVisibleGroundUncoveredCount: average(visibleGroundUncoveredSamples),
    maxVisibleGroundUncoveredCount: maxValue(visibleGroundUncoveredSamples),
    avgVisibleGroundResidentNotReadyCount: average(visibleGroundResidentNotReadySamples),
    maxVisibleGroundResidentNotReadyCount: maxValue(visibleGroundResidentNotReadySamples),
    maxSurfaceContinuityGapCount: maxValue(surfaceContinuityGapSamples),
    framesWithVisibleGroundGaps: samples.filter((sample) => sample.visibleGroundUncoveredCount > 0).length,
    framesWithSurfaceContinuityGaps: samples.filter((sample) => sample.surfaceContinuityGapCount > 0).length,
    framesWithFarLodCoverageGaps: samples.filter((sample) => sample.farLodCoverageGapCount > 0).length,
    framesWithSeamGaps: samples.filter((sample) => sample.seamGapCount > 0).length,
    framesWithBlockingSeamGaps: seamFrameClasses.framesWithBlockingSeamGaps,
    framesWithTransitionSeamGaps: seamFrameClasses.framesWithTransitionSeamGaps,
    framesWithLodOverlaps: samples.filter((sample) => sample.lodOverlapCount > 0).length,
    maxSeamGapMeters: maxValue(samples.map((sample) => sample.maxSeamGapMeters)),
    maxSurfaceContinuityStepMeters: maxValue(surfaceContinuityStepSamples),
    maxFarLodCoverageGapMeters: maxValue(farLodCoverageDistanceSamples),
    maxLodOverlapMeters: maxValue(samples.map((sample) => sample.maxLodOverlapMeters)),
    screenVoidCaptureCount: samples.filter((sample) => sample.screenVoidRatio !== null).length,
    framesWithScreenVoidSignals: samples.filter((sample) => sample.screenVoidSuspicious).length,
    framesWithSettledReferenceHoleSignals: samples.filter((sample) => sample.settledReferenceSuspiciousHole).length,
    framesWithHoleSignals: samples.filter((sample) => sample.suspiciousHole).length,
    maxScreenVoidRatio: maxValue(samples.map((sample) => sample.screenVoidRatio ?? 0)),
    maxSettledReferenceChangedRatio: maxValue(settledReferenceChangedSamples),
    maxSettledReferenceClearToFilledRatio: maxValue(settledReferenceClearToFilledSamples),
    maxSettledReferenceClearToFilledRunRatio: maxValue(settledReferenceClearToFilledRunSamples),
    maxPendingChunks: maxValue(samples.map((sample) => sample.pendingChunks)),
    maxPendingMeshJobs: maxValue(samples.map((sample) => sample.pendingMeshJobs)),
    maxDirtyResidentChunks: maxValue(samples.map((sample) => sample.dirtyResidentChunks)),
    maxDirtyMeshlessResidentChunks: maxValue(samples.map((sample) => sample.dirtyMeshlessResidentChunks)),
    maxDirtyRetainedMeshResidentChunks: maxValue(samples.map((sample) => sample.dirtyRetainedMeshResidentChunks)),
    settleFramesUntilComplete: settleCompletion
      ? samples.filter((sample) => sample.phase === "settle" && sample.frame <= settleCompletion.frame).length
      : null,
  };
}

function normalizeRouteBenchmarkPlanOptions(options: RouteExperienceBenchmarkOptions): {
  durationSeconds: number;
  sampleHz: number;
  speedMetersPerSecond: number;
} {
  return {
    durationSeconds: Math.max(1, options.durationSeconds ?? 10),
    sampleHz: Math.max(1, Math.floor(options.sampleHz ?? 60)),
    speedMetersPerSecond: Math.max(0.1, options.speedMetersPerSecond ?? 4.6),
  };
}

function resolveBenchmarkYawDrift(
  elapsedSeconds: number,
  amplitudeRadians: number,
  periodSeconds: number,
): number {
  if (amplitudeRadians <= 0) {
    return 0;
  }
  const phase = (elapsedSeconds / periodSeconds) * Math.PI * 2;
  return Math.sin(phase) * amplitudeRadians
    + Math.sin(phase * 0.43) * amplitudeRadians * 0.35;
}

function zeroResidencyPhaseMetrics(): ResidencyUpdateSummary["phaseMs"] {
  return {
    surfaceSampleMs: 0,
    yRangeMs: 0,
    chunkGenerationMs: 0,
    chunkDispatchMs: 0,
    chunkDrainMs: 0,
    summaryDrainMs: 0,
    chunkAdoptionMs: 0,
    evictionMs: 0,
    neighborDirtyMs: 0,
    inFlightChunks: 0,
    completedChunkCacheHits: 0,
    completedGeneratedChunks: 0,
    completedSummaryCacheHits: 0,
    completedGeneratedSummaries: 0,
    completedRegionSummaryCacheHits: 0,
    missingRegionSummaries: 0,
    readyGeneratedChunkBacklog: 0,
  };
}

function isMovementKey(code: string): boolean {
  return code === "KeyW"
    || code === "KeyA"
    || code === "KeyS"
    || code === "KeyD"
    || code === "Space"
    || code === "ControlLeft"
    || code === "ControlRight"
    || code === "AltLeft"
    || code === "AltRight";
}

function clampPositiveInt(value: number, fallback: number): number {
  const normalized = Math.floor(value);
  return Number.isFinite(normalized) && normalized > 0 ? normalized : fallback;
}

function pushVisibleGroundIssueSample(
  target: VisibleGroundCoverageIssueSample[],
  sample: VisibleGroundCoverageIssueSample,
): void {
  if (target.length >= 8) {
    return;
  }
  target.push(sample);
}

function pushSurfaceContinuityIssueSample(
  target: SurfaceContinuityIssueSample[],
  sample: SurfaceContinuityIssueSample,
): void {
  if (target.length >= 8) {
    return;
  }
  target.push(sample);
}

function summarizeDirtyResidentMeshes(world: ProceduralResidentWorld): {
  dirtyResidentChunks: number;
  dirtyMeshlessResidentChunks: number;
  dirtyRetainedMeshResidentChunks: number;
} {
  let dirtyResidentChunks = 0;
  let dirtyMeshlessResidentChunks = 0;
  let dirtyRetainedMeshResidentChunks = 0;
  for (const chunk of world.iterateDirtyResidentChunks()) {
    if (!chunk.meshDirty) {
      continue;
    }
    dirtyResidentChunks += 1;
    if (chunk.mesh) {
      dirtyRetainedMeshResidentChunks += 1;
    } else {
      dirtyMeshlessResidentChunks += 1;
    }
  }
  return {
    dirtyResidentChunks,
    dirtyMeshlessResidentChunks,
    dirtyRetainedMeshResidentChunks,
  };
}

function countMissingResidentColumnsAround(
  world: ProceduralResidentWorld,
  centerChunkX: number,
  centerChunkZ: number,
  radiusChunks: number,
): number {
  let missingColumns = 0;
  for (let dz = -radiusChunks; dz <= radiusChunks; dz += 1) {
    for (let dx = -radiusChunks; dx <= radiusChunks; dx += 1) {
      if (dx * dx + dz * dz > radiusChunks * radiusChunks) {
        continue;
      }
      if (!world.hasResidentColumn(centerChunkX + dx, centerChunkZ + dz)) {
        missingColumns += 1;
      }
    }
  }
  return missingColumns;
}

function countColumnsWithinRadius(radiusChunks: number): number {
  let columns = 0;
  for (let dz = -radiusChunks; dz <= radiusChunks; dz += 1) {
    for (let dx = -radiusChunks; dx <= radiusChunks; dx += 1) {
      if (dx * dx + dz * dz > radiusChunks * radiusChunks) {
        continue;
      }
      columns += 1;
    }
  }
  return columns;
}

function countUrgentDirtyMeshlessChunks(
  world: ProceduralResidentWorld,
  priorityChunkX: number,
  priorityChunkY: number,
  priorityChunkZ: number,
): number {
  let urgentDirtyMeshlessChunks = 0;
  for (const chunk of world.iterateDirtyResidentChunks()) {
    if (!chunk.meshDirty || chunk.mesh) {
      continue;
    }
    if (shouldSyncBuildUrgentChunk(chunk, priorityChunkX, priorityChunkY, priorityChunkZ)) {
      urgentDirtyMeshlessChunks += 1;
    }
  }
  return urgentDirtyMeshlessChunks;
}

function nextAnimationFrame(): Promise<number> {
  return new Promise((resolve) => {
    requestAnimationFrame((now) => resolve(now));
  });
}

function addLodLevelCounts(target: number[], source: readonly number[]): void {
  for (let index = 0; index < target.length; index += 1) {
    target[index] += source[index] ?? 0;
  }
}

function sumNumbers(values: readonly number[]): number {
  let total = 0;
  for (const value of values) {
    total += value;
  }
  return total;
}
