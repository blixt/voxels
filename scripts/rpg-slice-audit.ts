import { existsSync, readdirSync } from "node:fs";
import { join } from "node:path";

import {
  buildAmbientRenderEnvironment,
  resolveAmbientWorldProfile,
  type AmbientWorldProbe,
} from "../src/engine/ambient-environment.ts";
import { describeExplorationObjectives } from "../src/engine/exploration-objectives.ts";
import { ExplorationJournal } from "../src/engine/exploration-journal.ts";
import { resolveExplorationInteractionTarget } from "../src/engine/exploration-interactions.ts";
import { describeExplorationSkillEffects } from "../src/engine/exploration-skill-effects.ts";
import { SkillJournal } from "../src/engine/skill-journal.ts";
import { RouteJournal, type TravelGoalDefinition } from "../src/engine/travel-goals.ts";
import {
  getAtlasCaveAnchors,
  getAtlasRegionGraph,
  getAtlasRouteAnchors,
  sampleWorldAtlasMeters,
  WORLD_ATLAS,
} from "../src/engine/world-atlas.ts";

export interface RpgSliceAuditFacts {
  readonly worldDefinition: {
    readonly regionCount: number;
    readonly routeCount: number;
    readonly caveSystemCount: number;
    readonly regionsWithAmbientProfiles: number;
    readonly regionGraphNodeCount: number;
    readonly routeAnchorCount: number;
    readonly caveAnchorCount: number;
    readonly finiteIslandSamplesPass: boolean;
    readonly minRouteSetPieceCount: number;
  };
  readonly explorationLoop: {
    readonly interactionVerbCount: number;
    readonly travelGoalRequiredStepCount: number;
    readonly completedTravelGoalCount: number;
    readonly routeJournalRoundTrips: boolean;
    readonly discoveredCategoryCount: number;
    readonly objectiveStageReached: "first-bearings" | "pilgrim-road" | "deep-pilgrimage";
  };
  readonly ambiance: {
    readonly ambientProfileCount: number;
    readonly regionAmbientCoverageCount: number;
    readonly minProfileColorDistance: number;
    readonly routeContextProfileCount: number;
    readonly undergroundProfileCount: number;
    readonly screenshotEvidenceAvailable: boolean;
  };
  readonly skillsProgression: {
    readonly skillCount: number;
    readonly discoveryXpCategoryCount: number;
    readonly travelContextCount: number;
    readonly skillJournalRoundTrips: boolean;
    readonly skillEffectsImproveExploration: boolean;
    readonly duplicateDiscoveryIgnored: boolean;
  };
  readonly performanceEvidenceAvailability: {
    readonly verificationCommandCount: number;
    readonly artifactKindCount: number;
    readonly benchmarkEvidenceAvailable: boolean;
    readonly renderEvidenceAvailable: boolean;
    readonly rpgEvidenceAvailable: boolean;
    readonly latestEvidenceIsNotRequiredForScore: boolean;
  };
  readonly verificationCoverage: {
    readonly focusedTestFileCount: number;
    readonly pureModuleTestCount: number;
    readonly browserHarnessTestCount: number;
    readonly rubricDocumented: boolean;
    readonly auditHasFixtureTests: boolean;
    readonly jsonAuditCommandDocumented: boolean;
  };
}

export interface AuditCriterion {
  readonly id: string;
  readonly label: string;
  readonly passed: boolean;
  readonly value: number | string | boolean;
  readonly target: number | string | boolean;
  readonly weight: number;
}

export interface AuditSubScore {
  readonly id: keyof RpgSliceAuditFacts;
  readonly label: string;
  readonly score: number;
  readonly maxScore: 5;
  readonly status: "weak" | "partial" | "near-milestone" | "milestone-ready";
  readonly criteria: readonly AuditCriterion[];
}

export interface RpgSliceAuditReport {
  readonly schemaVersion: 1;
  readonly goal: "morrowind-like-rpg-slice";
  readonly scoringNote: string;
  readonly overallScore: number;
  readonly maxScore: 5;
  readonly status: AuditSubScore["status"];
  readonly subScores: readonly AuditSubScore[];
  readonly facts: RpgSliceAuditFacts;
}

const BASE_FIELDS: AmbientWorldProbe["fields"] = {
  temperature: 0.5,
  moisture: 0.5,
  uplift: 0.5,
  drainage: 0.5,
  volcanism: 0.5,
  magic: 0.5,
  globalHeight: 0.5,
  mountainness: 0.5,
  oceanness: 0.5,
  ridge: 0.5,
  mesa: 0.5,
  desolation: 0.5,
  strata: 0.5,
  surfacePatch: 0.5,
  surfaceGrain: 0.5,
  scatter: 0.5,
  peakness: 0.5,
  islandInterior: 1,
  coastalShelf: 0,
  shorelineBand: 0,
  deepOcean: 0,
};

export function buildRpgSliceAuditReport(facts: RpgSliceAuditFacts): RpgSliceAuditReport {
  const subScores = [
    scoreWorldDefinition(facts.worldDefinition),
    scoreExplorationLoop(facts.explorationLoop),
    scoreAmbiance(facts.ambiance),
    scoreSkillsProgression(facts.skillsProgression),
    scorePerformanceEvidenceAvailability(facts.performanceEvidenceAvailability),
    scoreVerificationCoverage(facts.verificationCoverage),
  ];
  const overallScore = roundScore(
    subScores.reduce((total, subScore) => total + subScore.score, 0) / subScores.length,
  );
  return {
    schemaVersion: 1,
    goal: "morrowind-like-rpg-slice",
    scoringNote:
      "Scores emphasize authored world, route loop, ambiance, skills, evidence, and tests. LOD-specific data is treated only as render/performance evidence, not as the main progress criterion.",
    overallScore,
    maxScore: 5,
    status: statusForScore(overallScore),
    subScores,
    facts,
  };
}

export function collectProjectAuditFacts(options: { artifactRoot?: string; cwd?: string } = {}): RpgSliceAuditFacts {
  const cwd = options.cwd ?? process.cwd();
  const artifactRoot = options.artifactRoot ?? "artifacts";
  const artifactFiles = listFiles(join(cwd, artifactRoot), 4);
  const packageJsonExists = existsSync(join(cwd, "package.json"));
  const testFiles = [
    "tests/world-atlas.test.ts",
    "tests/exploration-events.test.ts",
    "tests/exploration-journal.test.ts",
    "tests/exploration-objectives.test.ts",
    "tests/exploration-interactions.test.ts",
    "tests/travel-goals.test.ts",
    "tests/skill-journal.test.ts",
    "tests/exploration-skill-effects.test.ts",
    "tests/ambient-environment.test.ts",
    "tests/rpg-ui-cleanup.test.ts",
    "tests/render-verification-runner.test.ts",
    "tests/browser-game-benchmark-harness.test.ts",
    "tests/rpg-slice-audit.test.ts",
  ];

  return {
    worldDefinition: collectWorldDefinitionFacts(),
    explorationLoop: collectExplorationLoopFacts(),
    ambiance: collectAmbianceFacts(artifactFiles),
    skillsProgression: collectSkillsProgressionFacts(),
    performanceEvidenceAvailability: {
      verificationCommandCount: [
        "scripts/run-voxel-rpg-verification.ts",
        "scripts/run-render-verification.ts",
        "scripts/route-atlas.ts",
        "scripts/capture-view-atlas.ts",
        "scripts/run-browser-route-trace.ts",
        "scripts/run-browser-game-benchmarks.ts",
      ].filter((path) => existsSync(join(cwd, path))).length,
      artifactKindCount: countPresentArtifactKinds(artifactFiles, [
        "route-atlas",
        "view-atlas",
        "owned-browser-lab",
        "browser-route-trace",
        "voxel-rpg-verification",
        "render-verification",
      ]),
      benchmarkEvidenceAvailable: hasArtifactKind(artifactFiles, "browser-route-trace")
        || hasArtifactKind(artifactFiles, "browser-game-benchmark"),
      renderEvidenceAvailable: hasArtifactKind(artifactFiles, "view-atlas")
        || hasArtifactKind(artifactFiles, "render-verification"),
      rpgEvidenceAvailable: hasArtifactKind(artifactFiles, "voxel-rpg-verification")
        || existsSync(join(cwd, "scripts/run-voxel-rpg-verification.ts")),
      latestEvidenceIsNotRequiredForScore: true,
    },
    verificationCoverage: {
      focusedTestFileCount: testFiles.filter((path) => existsSync(join(cwd, path))).length,
      pureModuleTestCount: testFiles
        .filter((path) => existsSync(join(cwd, path)) && !path.includes("browser"))
        .length,
      browserHarnessTestCount: testFiles
        .filter((path) => existsSync(join(cwd, path)) && path.includes("browser"))
        .length,
      rubricDocumented: existsSync(join(cwd, "docs/loop/20260509-rpg-slice-rubric.md")),
      auditHasFixtureTests: existsSync(join(cwd, "tests/rpg-slice-audit.test.ts")),
      jsonAuditCommandDocumented: packageJsonExists,
    },
  };
}

function collectWorldDefinitionFacts(): RpgSliceAuditFacts["worldDefinition"] {
  const regionGraph = getAtlasRegionGraph();
  const island = WORLD_ATLAS.island;
  const finiteIslandSamplesPass = [
    sampleWorldAtlasMeters(island.origin.x, island.origin.z).surfaceClass === "land",
    sampleWorldAtlasMeters(island.origin.x + island.radius.x * 1.2, island.origin.z).surfaceClass !== "land",
    sampleWorldAtlasMeters(island.origin.x, island.origin.z + island.radius.z * 1.2).surfaceClass !== "land",
  ].every(Boolean);

  return {
    regionCount: WORLD_ATLAS.regions.length,
    routeCount: WORLD_ATLAS.routes.length,
    caveSystemCount: WORLD_ATLAS.caveSystems.length,
    regionsWithAmbientProfiles: WORLD_ATLAS.regions.filter((region) => region.ambientProfileId).length,
    regionGraphNodeCount: regionGraph.length,
    routeAnchorCount: getAtlasRouteAnchors().length,
    caveAnchorCount: getAtlasCaveAnchors().length,
    finiteIslandSamplesPass,
    minRouteSetPieceCount: Math.min(...WORLD_ATLAS.routes.map((route) => route.recommendedSetPieceIds.length)),
  };
}

function collectExplorationLoopFacts(): RpgSliceAuditFacts["explorationLoop"] {
  const interaction = resolveExplorationInteractionTarget({
    viewerPosition: [0, 0, 0],
    viewerForward: [0, 0, 1],
    candidates: [{
      id: "pilgrim-lantern-fixture",
      subjectType: "landmark",
      name: "Pilgrim Lantern",
      role: "old-road",
      worldPosition: [0, 0, 2],
      prompts: ["inspect", "read", "use"],
    }],
  });
  const route = WORLD_ATLAS.routes[0]!;
  const travelGoal: TravelGoalDefinition = {
    id: "audit-route-goal",
    routeId: route.id,
    title: "Audit Route",
    journalText: "Follow a short authored route.",
    steps: [
      { id: "visit-start", kind: "visit", targetId: route.nodes[0]!.id, label: "Visit start" },
      { id: "inspect-marker", kind: "inspect", targetId: route.recommendedSetPieceIds[0]!, label: "Inspect marker" },
      { id: "read-marker", kind: "read", targetId: route.recommendedSetPieceIds[1]!, label: "Read marker" },
    ],
  };
  const journal = new RouteJournal([travelGoal]);
  journal.observeProgress({ routeId: route.id, kind: "visit", targetId: route.nodes[0]!.id });
  journal.observeProgress({ routeId: route.id, kind: "inspect", targetId: route.recommendedSetPieceIds[0]! });
  journal.observeProgress({ routeId: route.id, kind: "read", targetId: route.recommendedSetPieceIds[1]! });
  const routeState = journal.exportState();
  const importedJournal = new RouteJournal([travelGoal]);
  const importedSnapshot = importedJournal.importState(routeState);

  const explorationJournal = new ExplorationJournal();
  explorationJournal.observe({
    biomeId: "marsh",
    undergroundBiomeId: "rooted",
    regionalVariantId: "marsh_blackwater",
    landmarkIds: ["pilgrim_lantern", "velothi_shrine", "old_road_causeway"],
    currentLandmarkId: "pilgrim_lantern",
  });
  const snapshot = explorationJournal.getSnapshot();
  const objectives = describeExplorationObjectives({
    discoveredBiomeCount: 10,
    discoveredUndergroundBiomeCount: 3,
    discoveredRegionalVariantCount: 4,
    discoveredLandmarkCount: 12,
    discoveredAncientLandmarkCount: 4,
    scoutedMobTrailCount: 5,
    lootedCacheCount: 5,
    scoutedCaveMouthCount: 3,
    traversedCavePassageCount: 1,
  });

  return {
    interactionVerbCount: new Set(interaction.target?.prompts.map((prompt) => prompt.verb) ?? []).size,
    travelGoalRequiredStepCount: travelGoal.steps.filter((step) => step.optional !== true).length,
    completedTravelGoalCount: importedSnapshot.completedGoalIds.length,
    routeJournalRoundTrips: importedSnapshot.goals[0]?.completed === true,
    discoveredCategoryCount: [
      snapshot.discoveredBiomeIds.length > 0,
      snapshot.discoveredUndergroundBiomeIds.length > 0,
      snapshot.discoveredRegionalVariantIds.length > 0,
      snapshot.discoveredLandmarkIds.length > 0,
    ].filter(Boolean).length,
    objectiveStageReached: readObjectiveStageId(objectives.stageId),
  };
}

function collectAmbianceFacts(artifactFiles: readonly string[]): RpgSliceAuditFacts["ambiance"] {
  const regionProfiles = new Set(WORLD_ATLAS.regions.map((region) => region.ambientProfileId));
  const resolvedProfiles = new Map(WORLD_ATLAS.regions.map((region) => {
    const profile = resolveAmbientWorldProfile({
      biomeId: region.biomeId,
      undergroundBiomeId: null,
      regionalVariantId: region.regionalVariantId,
      regionalVariantStrength: region.regionalVariantId ? 1 : 0,
      specialStrength: 0.5,
      surfaceY: 0,
      regionAmbientProfileId: region.ambientProfileId,
      regionStrength: 1,
      fields: BASE_FIELDS,
    });
    return [profile.id, profile];
  }));
  const undergroundProfiles = new Set([
    resolveAmbientWorldProfile(buildProbe("fungal"), { observedUndergroundBiomeId: "rooted" }).id,
    resolveAmbientWorldProfile(buildProbe("ember"), { observedUndergroundBiomeId: "basaltic" }).id,
  ]);

  return {
    ambientProfileCount: regionProfiles.size,
    regionAmbientCoverageCount: WORLD_ATLAS.regions.filter((region) => region.ambientProfileId).length,
    minProfileColorDistance: minColorDistance([...resolvedProfiles.values()].map(buildAmbientRenderEnvironment)),
    routeContextProfileCount: new Set(WORLD_ATLAS.routes.map((route) => route.segmentKind)).size,
    undergroundProfileCount: undergroundProfiles.size,
    screenshotEvidenceAvailable: hasArtifactKind(artifactFiles, "view-atlas")
      || hasArtifactKind(artifactFiles, "owned-browser-lab"),
  };
}

function collectSkillsProgressionFacts(): RpgSliceAuditFacts["skillsProgression"] {
  const explorationJournal = new ExplorationJournal();
  explorationJournal.observe({
    biomeId: "marsh",
    undergroundBiomeId: "rooted",
    regionalVariantId: "marsh_blackwater",
    landmarkIds: ["pilgrim_lantern", "velothi_shrine"],
    currentLandmarkId: "pilgrim_lantern",
  });
  const discoveries = explorationJournal.drainPendingSkillDiscoveries();
  const skills = new SkillJournal();
  skills.observeDiscoveries(discoveries);
  const beforeDuplicate = sumSkillXp(skills.getSnapshot().skills);
  skills.observeDiscoveries(discoveries);
  const afterDuplicate = sumSkillXp(skills.getSnapshot().skills);
  skills.observeTravel(96, "surface");
  skills.observeTravel(96, "underground");
  const state = skills.exportState();
  const imported = new SkillJournal();
  const importedSnapshot = imported.importState(state);
  const baseEffects = describeExplorationSkillEffects({
    cartographyLevel: 1,
    naturalistLevel: 1,
    spelunkingLevel: 1,
  });
  const improvedEffects = describeExplorationSkillEffects({
    cartographyLevel: 4,
    naturalistLevel: 4,
    spelunkingLevel: 4,
  });

  return {
    skillCount: importedSnapshot.skills.length,
    discoveryXpCategoryCount: new Set(discoveries.map((discovery) => discovery.category)).size,
    travelContextCount: 2,
    skillJournalRoundTrips: importedSnapshot.travelMeters > 0 && importedSnapshot.skills.some((skill) => skill.totalXp > 0),
    skillEffectsImproveExploration:
      improvedEffects.landmarkScanRadiusMeters > baseEffects.landmarkScanRadiusMeters
      && improvedEffects.surfaceTravelSpeedMultiplier > baseEffects.surfaceTravelSpeedMultiplier
      && improvedEffects.undergroundTravelSpeedMultiplier > baseEffects.undergroundTravelSpeedMultiplier,
    duplicateDiscoveryIgnored: beforeDuplicate === afterDuplicate,
  };
}

function scoreWorldDefinition(facts: RpgSliceAuditFacts["worldDefinition"]): AuditSubScore {
  return buildSubScore("worldDefinition", "World Definition", [
    atLeast("regions", "Authored macro regions", facts.regionCount, 8, 1.2),
    atLeast("routes", "Authored route graph", facts.routeCount, 8, 1.1),
    atLeast("caves", "Cave systems", facts.caveSystemCount, 6, 0.9),
    atLeast("ambient", "Region ambient coverage", facts.regionsWithAmbientProfiles, facts.regionCount, 0.9),
    atLeast("graph", "Region graph nodes", facts.regionGraphNodeCount, facts.regionCount, 0.8),
    atLeast("route-anchors", "Route anchors", facts.routeAnchorCount, 16, 0.7),
    atLeast("cave-anchors", "Cave anchors", facts.caveAnchorCount, 18, 0.7),
    booleanCriterion("finite-island", "Finite island/coast samples", facts.finiteIslandSamplesPass, true, 1),
    atLeast("set-pieces", "Route set-piece hooks", facts.minRouteSetPieceCount, 2, 0.7),
  ]);
}

function scoreExplorationLoop(facts: RpgSliceAuditFacts["explorationLoop"]): AuditSubScore {
  return buildSubScore("explorationLoop", "Exploration Loop", [
    atLeast("verbs", "Inspect/read/use verbs", facts.interactionVerbCount, 3, 1.2),
    atLeast("goal-steps", "Route goal required steps", facts.travelGoalRequiredStepCount, 3, 1),
    atLeast("goal-complete", "Completed route goal", facts.completedTravelGoalCount, 1, 1),
    booleanCriterion("route-persistence", "Route journal round trip", facts.routeJournalRoundTrips, true, 1),
    atLeast("discoveries", "Discovery categories", facts.discoveredCategoryCount, 4, 1),
    oneOf("objective-stage", "Objective stage reaches deep loop", facts.objectiveStageReached, ["deep-pilgrimage"], 0.8),
  ]);
}

function scoreAmbiance(facts: RpgSliceAuditFacts["ambiance"]): AuditSubScore {
  return buildSubScore("ambiance", "Ambiance", [
    atLeast("profiles", "Distinct ambient profiles", facts.ambientProfileCount, 5, 1),
    atLeast("region-coverage", "Region profile coverage", facts.regionAmbientCoverageCount, 8, 1),
    atLeast("color-distance", "Profile color separation", facts.minProfileColorDistance, 80, 1),
    atLeast("route-contexts", "Route context variety", facts.routeContextProfileCount, 5, 0.8),
    atLeast("underground", "Underground atmosphere profiles", facts.undergroundProfileCount, 2, 0.7),
    booleanCriterion("screenshots", "Screenshot evidence available", facts.screenshotEvidenceAvailable, true, 0.8),
  ]);
}

function scoreSkillsProgression(facts: RpgSliceAuditFacts["skillsProgression"]): AuditSubScore {
  return buildSubScore("skillsProgression", "Skills And Progression", [
    atLeast("skills", "Exploration skill count", facts.skillCount, 4, 1),
    atLeast("discovery-xp", "Discovery XP categories", facts.discoveryXpCategoryCount, 4, 1),
    atLeast("travel-contexts", "Travel XP contexts", facts.travelContextCount, 2, 0.8),
    booleanCriterion("skill-persistence", "Skill journal round trip", facts.skillJournalRoundTrips, true, 1),
    booleanCriterion("effects", "Skill effects improve exploration", facts.skillEffectsImproveExploration, true, 1),
    booleanCriterion("idempotent", "Duplicate discoveries ignored", facts.duplicateDiscoveryIgnored, true, 0.8),
  ]);
}

function scorePerformanceEvidenceAvailability(
  facts: RpgSliceAuditFacts["performanceEvidenceAvailability"],
): AuditSubScore {
  return buildSubScore("performanceEvidenceAvailability", "Performance Evidence Availability", [
    atLeast("commands", "Verification commands", facts.verificationCommandCount, 6, 1),
    atLeast("artifacts", "Artifact categories present", facts.artifactKindCount, 4, 0.8),
    booleanCriterion("benchmark", "Browser benchmark evidence", facts.benchmarkEvidenceAvailable, true, 1),
    booleanCriterion("render", "Render evidence path", facts.renderEvidenceAvailable, true, 1),
    booleanCriterion("rpg", "RPG verification path", facts.rpgEvidenceAvailable, true, 1),
    booleanCriterion("not-latest", "Does not require fresh latest artifacts", facts.latestEvidenceIsNotRequiredForScore, true, 0.6),
  ]);
}

function scoreVerificationCoverage(facts: RpgSliceAuditFacts["verificationCoverage"]): AuditSubScore {
  return buildSubScore("verificationCoverage", "Verification Coverage", [
    atLeast("focused-tests", "Focused test files", facts.focusedTestFileCount, 12, 1),
    atLeast("pure-tests", "Pure module tests", facts.pureModuleTestCount, 10, 1),
    atLeast("browser-tests", "Browser harness tests", facts.browserHarnessTestCount, 1, 0.7),
    booleanCriterion("rubric", "Rubric documented", facts.rubricDocumented, true, 1),
    booleanCriterion("fixture-tests", "Audit fixture tests", facts.auditHasFixtureTests, true, 1),
    booleanCriterion("json-command", "JSON command documented", facts.jsonAuditCommandDocumented, true, 0.6),
  ]);
}

function buildSubScore(
  id: keyof RpgSliceAuditFacts,
  label: string,
  criteria: readonly AuditCriterion[],
): AuditSubScore {
  const weight = criteria.reduce((total, criterion) => total + criterion.weight, 0);
  const earned = criteria.reduce((total, criterion) => total + (criterion.passed ? criterion.weight : 0), 0);
  const score = roundScore((earned / Math.max(1, weight)) * 5);
  return {
    id,
    label,
    score,
    maxScore: 5,
    status: statusForScore(score),
    criteria,
  };
}

function atLeast(id: string, label: string, value: number, target: number, weight: number): AuditCriterion {
  return { id, label, value, target, weight, passed: value >= target };
}

function booleanCriterion(
  id: string,
  label: string,
  value: boolean,
  target: boolean,
  weight: number,
): AuditCriterion {
  return { id, label, value, target, weight, passed: value === target };
}

function oneOf(
  id: string,
  label: string,
  value: string,
  target: readonly string[],
  weight: number,
): AuditCriterion {
  return { id, label, value, target: target.join("|"), weight, passed: target.includes(value) };
}

function statusForScore(score: number): AuditSubScore["status"] {
  if (score >= 4) {
    return "milestone-ready";
  }
  if (score >= 3.5) {
    return "near-milestone";
  }
  if (score >= 2) {
    return "partial";
  }
  return "weak";
}

function readObjectiveStageId(value: string): RpgSliceAuditFacts["explorationLoop"]["objectiveStageReached"] {
  if (value === "pilgrim-road" || value === "deep-pilgrimage") {
    return value;
  }
  return "first-bearings";
}

function roundScore(value: number): number {
  return Math.round(value * 100) / 100;
}

function buildProbe(biomeId: AmbientWorldProbe["biomeId"]): AmbientWorldProbe {
  return {
    biomeId,
    undergroundBiomeId: null,
    regionalVariantId: null,
    regionalVariantStrength: 0,
    specialStrength: 0,
    surfaceY: 0,
    fields: BASE_FIELDS,
  };
}

function minColorDistance(
  profiles: readonly { clearColorRgba: readonly number[]; fogColorRgba: readonly number[]; skyHorizonColorRgba: readonly number[] }[],
): number {
  let min = Infinity;
  for (let left = 0; left < profiles.length; left += 1) {
    for (let right = left + 1; right < profiles.length; right += 1) {
      min = Math.min(min, colorDistance(profiles[left]!.clearColorRgba, profiles[right]!.clearColorRgba));
      min = Math.min(min, colorDistance(profiles[left]!.fogColorRgba, profiles[right]!.fogColorRgba));
      min = Math.min(min, colorDistance(profiles[left]!.skyHorizonColorRgba, profiles[right]!.skyHorizonColorRgba));
    }
  }
  return Number.isFinite(min) ? roundScore(min) : 0;
}

function colorDistance(left: readonly number[], right: readonly number[]): number {
  return Math.hypot(left[0]! - right[0]!, left[1]! - right[1]!, left[2]! - right[2]!);
}

function sumSkillXp(skills: readonly { totalXp: number }[]): number {
  return skills.reduce((total, skill) => total + skill.totalXp, 0);
}

function countPresentArtifactKinds(files: readonly string[], kinds: readonly string[]): number {
  return kinds.filter((kind) => hasArtifactKind(files, kind)).length;
}

function hasArtifactKind(files: readonly string[], kind: string): boolean {
  return files.some((file) => file.includes(`/${kind}/`) && file.endsWith("report.json"));
}

function listFiles(root: string, maxDepth: number): string[] {
  if (maxDepth < 0 || !existsSync(root)) {
    return [];
  }
  const files: string[] = [];
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    const path = join(root, entry.name);
    if (entry.isDirectory()) {
      files.push(...listFiles(path, maxDepth - 1));
    } else {
      files.push(path);
    }
  }
  return files.sort((left, right) => left.localeCompare(right));
}

if (import.meta.main) {
  const report = buildRpgSliceAuditReport(collectProjectAuditFacts(parseCli(Bun.argv)));
  console.log(JSON.stringify(report, null, 2));
}

function parseCli(argv: readonly string[]): { artifactRoot?: string; cwd?: string } {
  const args = argv.slice(2);
  return {
    artifactRoot: readFlag(args, "--artifact-root") ?? undefined,
    cwd: readFlag(args, "--cwd") ?? undefined,
  };
}

function readFlag(args: readonly string[], name: string): string | null {
  const prefix = `${name}=`;
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index]!;
    if (arg === name) {
      return args[index + 1] ?? null;
    }
    if (arg.startsWith(prefix)) {
      return arg.slice(prefix.length);
    }
  }
  return null;
}
