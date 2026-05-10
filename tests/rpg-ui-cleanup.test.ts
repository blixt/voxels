import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";

import { expect, test } from "bun:test";

const repoRoot = new URL("..", import.meta.url).pathname;

test("RPG game page has no block-editing HUD surfaces", () => {
  const gameHtml = readFileSync(join(repoRoot, "src/pages/game.html"), "utf8");
  const gameClient = readFileSync(join(repoRoot, "src/client/game.ts"), "utf8");
  const combined = `${gameHtml}\n${gameClient}`;

  expect(combined).not.toContain("game-hotbar");
  expect(combined).not.toContain("game-inventory-panel");
  expect(combined).not.toContain("target-overlay");
  expect(combined).not.toMatch(/\bHotbar\b|\bInventory\b|\bTargeting\b|No voxel in reach/);
});

test("RPG HUD renders place route interaction and skill snapshot fields", () => {
  const gameController = readFileSync(join(repoRoot, "src/client/game-controller.ts"), "utf8");
  const gameClient = readFileSync(join(repoRoot, "src/client/game.ts"), "utf8");
  const styles = readFileSync(join(repoRoot, "public/styles.css"), "utf8");

  for (const field of [
    "activePlaceName",
    "activeRouteName",
    "activeRouteProgressLabel",
    "activeTravelGoalStepLabel",
    "activeQuestTitle",
    "activeQuestObjectiveKind",
    "activeQuestObjectiveLabel",
    "activeQuestRumorText",
    "activeQuestMoodLabel",
    "activeQuestFactionLabel",
    "interactionPromptLabel",
    "navigationTargetId",
    "navigationTargetName",
    "navigationSource",
    "navigationDistanceMeters",
    "navigationBearingLabel",
    "navigationDistanceLabel",
    "navigationCompassLabel",
    "navigationTurnLabel",
    "travelContextLabel",
    "encounterMoodLabel",
    "encounterPressureLabel",
    "encounterFactionLabel",
    "passiveMobSightingCount",
    "passiveMobNearestId",
    "passiveMobNearestLabel",
    "passiveMobNearestDetailLabel",
    "passiveMobNearestDistanceMeters",
    "passiveMobNearestFactionLabel",
    "passiveMobNearestMoodLabel",
    "bestiarySightingCount",
    "bestiaryEntryCount",
    "bestiarySummaryLabel",
    "bestiaryLastSightingLabel",
    "bestiaryLastNoteLabel",
    "bestiaryDominantFactionLabel",
    "lastInteractionLabel",
    "scoutedCaveMouthCount",
    "fieldKitFindCount",
    "fieldKitSummaryLabel",
    "fieldKitLastFindLabel",
    "fieldKitLastNoteLabel",
    "fieldKitDominantCategoryLabel",
    "lootJournalCollectedCacheCount",
    "lootJournalRevisitedCacheCount",
    "lootJournalRevisitEventCount",
    "lootJournalStateLabel",
  ]) {
    expect(gameController).toContain(field);
    expect(gameClient).toContain(`snapshot.${field}`);
  }

  for (const className of [
    "game-rpg-place",
    "game-rpg-route",
    "game-rpg-interaction",
    "game-rpg-skill",
    "game-rpg-route-progress",
  ]) {
    expect(gameClient).toContain(className);
    expect(styles).toContain(`.${className}`);
  }

  for (const pressureClassName of [
    "is-pressure-high",
    "is-pressure-watchful",
    "is-pressure-low",
    "is-pressure-quiet",
  ]) {
    expect(gameClient).toContain(pressureClassName);
    expect(styles).toContain(`.${pressureClassName}`);
  }
});

test("client wires pure exploration interaction and travel goal systems", () => {
  const gameController = readFileSync(join(repoRoot, "src/client/game-controller.ts"), "utf8");
  const gameClient = readFileSync(join(repoRoot, "src/client/game.ts"), "utf8");

  expect(gameController).toContain("resolveExplorationInteractionTarget");
  expect(gameController).toContain("describeNavigationBearing");
  expect(gameController).toContain("samplePassiveMobSightingsWorldUnits");
  expect(gameController).toContain("navigation ? \"interaction-target\" : null");
  expect(gameClient).toContain("snapshot.navigationBearingLabel ? ` • ${snapshot.navigationBearingLabel}` : \"\"");
  expect(gameClient).toContain("snapshot.passiveMobSightingCount > 0 ? ` • ${snapshot.passiveMobNearestLabel}` : \"\"");
  expect(gameController).toContain("new ExplorationEventLog");
  expect(gameController).toContain("explorationEventLog.record");
  expect(gameController).toContain("observeSkillAwards");
  expect(gameController).toContain("subjectType: \"mob\"");
  expect(gameController).toContain("role: encounterSite.role");
  expect(gameController).toContain("countMobSignEvents");
  expect(gameController).toContain("sampleRpgEncounterSiteWorldUnits");
  expect(gameController).toContain("clueLabel: encounterSite.clueLabel");
  expect(gameController).toContain("fieldNote: encounterSite.fieldNote");
  expect(gameController).toContain("lootSkillId");
  expect(gameController).toContain("sampleForageSiteWorldUnits");
  expect(gameController).toContain("forageSiteRole");
  expect(gameController).toContain("clueLabel: forageSite.clueLabel");
  expect(gameController).toContain("fieldNote: forageSite.fieldNote");
  expect(gameController).toContain("summarizeFieldKit");
  expect(gameController).toContain("summarizeLootJournal");
  expect(gameController).toContain("getLootJournalCandidateState");
  expect(gameController).toContain("describeInteractionSkillGates");
  expect(gameController).toContain("skillGateHint: skillGates.forageHint");
  expect(gameController).toContain("skillGateHint: skillGates.caveRouteHint");
  expect(gameController).toContain("occurrenceId: exactLootRevisit ? `revisit-${lootState.eventCount + 1}` : null");
  expect(gameController).toContain("summarizeBestiary");
  expect(gameController).toContain("factionId: encounterSite.factionId");
  expect(gameController).toContain("role: \"cave-mouth\"");
  expect(gameController).toContain("skillId: \"spelunking\"");
  expect(gameController).toContain("new RouteJournal");
  expect(gameController).toContain("planRpgQuestHooks");
  expect(gameController).toContain("selectRpgQuestHookForExploration");
  expect(gameController).toContain("selectActiveQuestHook");
  expect(gameController).toContain("observeActiveQuestStep");
  expect(gameController).toContain("buildTravelGoalFromQuestHook");
  expect(gameController).toContain("buildQuestTravelGoals");
  expect(gameController).toContain("observeTravelGoalProgress");
  expect(gameController).toContain("complete-travel-goal");
  expect(gameController).toContain("observeProgress");
  expect(gameController).toContain("observeTravel");
  expect(gameController).toContain("TRAVEL_GOALS");
  expect(gameController).toContain("describeRpgEncounterScoutResult");
  expect(gameController).toContain("sampleRpgEncounterWorldUnits(this.player.feetPosition[0], this.player.feetPosition[2])");
  expect(gameController).toContain("interactionPromptVerb: prompt?.verb ?? \"inspect\"");
  expect(gameClient).toContain("snapshot.activeQuestTitle");
  expect(gameClient).toContain("snapshot.activeQuestRumorText");
});

test("progress storage hydrates after init and honors fresh-game launches", () => {
  const gameClient = readFileSync(join(repoRoot, "src/client/game.ts"), "utf8");

  expect(gameClient).toContain("const startFresh = searchParams.has(\"freshGame\")");
  expect(gameClient).toContain("if (startFresh)");
  expect(gameClient).toContain("clearProgressState()");
  expect(gameClient).toContain("let progressHydrated = false");
  expect(gameClient).toContain("progressHydrated && progressSignature !== lastProgressSignature");
  expect(gameClient).toContain("if (!startFresh)");
  expect(gameClient).toContain("loadProgressState(controller)");
  expect(gameClient).toContain("progressHydrated = true");
});

test("legacy material gathering and placement modules stay removed", () => {
  for (const path of [
    "src/engine/inventory.ts",
    "src/engine/hotbar-layout.ts",
    "src/engine/interaction-loop.ts",
    "src/engine/targeting-overlay.ts",
    "src/engine/voxel-raycast.ts",
  ]) {
    expect(existsSync(join(repoRoot, path))).toBe(false);
  }
});
