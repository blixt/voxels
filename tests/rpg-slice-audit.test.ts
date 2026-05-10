import { expect, test } from "bun:test";

import {
  buildRpgSliceAuditReport,
  collectProjectAuditFacts,
  type RpgSliceAuditFacts,
} from "../scripts/rpg-slice-audit.ts";

test("RPG slice audit scores a complete fixture as milestone-ready", () => {
  const report = buildRpgSliceAuditReport(completeFacts());

  expect(report.schemaVersion).toBe(1);
  expect(report.status).toBe("milestone-ready");
  expect(report.overallScore).toBe(5);
  expect(report.subScores.map((score) => score.id)).toEqual([
    "worldDefinition",
    "explorationLoop",
    "ambiance",
    "skillsProgression",
    "performanceEvidenceAvailability",
    "verificationCoverage",
  ]);
  expect(report.scoringNote).toContain("LOD-specific data");
});

test("RPG slice audit exposes low sub-scores for missing loop and evidence facts", () => {
  const report = buildRpgSliceAuditReport({
    ...completeFacts(),
    explorationLoop: {
      interactionVerbCount: 1,
      travelGoalRequiredStepCount: 1,
      completedTravelGoalCount: 0,
      routeJournalRoundTrips: false,
      discoveredCategoryCount: 1,
      objectiveStageReached: "first-bearings",
    },
    performanceEvidenceAvailability: {
      verificationCommandCount: 2,
      artifactKindCount: 0,
      benchmarkEvidenceAvailable: false,
      renderEvidenceAvailable: false,
      rpgEvidenceAvailable: true,
      latestEvidenceIsNotRequiredForScore: true,
    },
  });

  const exploration = report.subScores.find((score) => score.id === "explorationLoop")!;
  const evidence = report.subScores.find((score) => score.id === "performanceEvidenceAvailability")!;

  expect(report.status).toBe("near-milestone");
  expect(exploration.status).toBe("weak");
  expect(exploration.criteria.filter((criterion) => criterion.passed)).toHaveLength(0);
  expect(evidence.status).toBe("weak");
  expect(evidence.criteria.find((criterion) => criterion.id === "not-latest")?.passed).toBe(true);
});

test("RPG slice audit collector is deterministic for module-derived facts", () => {
  const first = collectProjectAuditFacts({ artifactRoot: "__missing_artifacts__" });
  const second = collectProjectAuditFacts({ artifactRoot: "__missing_artifacts__" });

  expect(second).toEqual(first);
  expect(first.worldDefinition.regionCount).toBeGreaterThanOrEqual(8);
  expect(first.explorationLoop.interactionVerbCount).toBe(3);
  expect(first.explorationLoop.objectiveStageReached).toBe("deep-pilgrimage");
  expect(first.skillsProgression.skillCount).toBe(4);
  expect(first.performanceEvidenceAvailability.latestEvidenceIsNotRequiredForScore).toBe(true);
});

function completeFacts(): RpgSliceAuditFacts {
  return {
    worldDefinition: {
      regionCount: 8,
      routeCount: 8,
      caveSystemCount: 6,
      regionsWithAmbientProfiles: 8,
      regionGraphNodeCount: 8,
      routeAnchorCount: 24,
      caveAnchorCount: 18,
      finiteIslandSamplesPass: true,
      minRouteSetPieceCount: 2,
    },
    explorationLoop: {
      interactionVerbCount: 3,
      travelGoalRequiredStepCount: 3,
      completedTravelGoalCount: 1,
      routeJournalRoundTrips: true,
      discoveredCategoryCount: 4,
      objectiveStageReached: "deep-pilgrimage",
    },
    ambiance: {
      ambientProfileCount: 5,
      regionAmbientCoverageCount: 8,
      minProfileColorDistance: 96,
      routeContextProfileCount: 5,
      undergroundProfileCount: 2,
      screenshotEvidenceAvailable: true,
    },
    skillsProgression: {
      skillCount: 4,
      discoveryXpCategoryCount: 4,
      travelContextCount: 2,
      skillJournalRoundTrips: true,
      skillEffectsImproveExploration: true,
      duplicateDiscoveryIgnored: true,
    },
    performanceEvidenceAvailability: {
      verificationCommandCount: 6,
      artifactKindCount: 4,
      benchmarkEvidenceAvailable: true,
      renderEvidenceAvailable: true,
      rpgEvidenceAvailable: true,
      latestEvidenceIsNotRequiredForScore: true,
    },
    verificationCoverage: {
      focusedTestFileCount: 12,
      pureModuleTestCount: 10,
      browserHarnessTestCount: 1,
      rubricDocumented: true,
      auditHasFixtureTests: true,
      jsonAuditCommandDocumented: true,
    },
  };
}
