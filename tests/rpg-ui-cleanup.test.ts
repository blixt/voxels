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
    "interactionPromptLabel",
    "travelContextLabel",
    "lastInteractionLabel",
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
});

test("client wires pure exploration interaction and travel goal systems", () => {
  const gameController = readFileSync(join(repoRoot, "src/client/game-controller.ts"), "utf8");

  expect(gameController).toContain("resolveExplorationInteractionTarget");
  expect(gameController).toContain("new RouteJournal");
  expect(gameController).toContain("observeProgress");
  expect(gameController).toContain("observeTravel");
  expect(gameController).toContain("TRAVEL_GOALS");
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
