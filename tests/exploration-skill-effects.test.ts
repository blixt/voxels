import { expect, test } from "bun:test";

import {
  BASE_LANDMARK_SCAN_RADIUS_METERS,
  describeExplorationSkillEffects,
} from "../src/engine/exploration-skill-effects.ts";

test("exploration skill effects keep novice landmark sensing at the baseline", () => {
  const effects = describeExplorationSkillEffects({
    cartographyLevel: 1,
    naturalistLevel: 1,
    spelunkingLevel: 1,
  });

  expect(effects.landmarkScanRadiusMeters).toBe(BASE_LANDMARK_SCAN_RADIUS_METERS);
  expect(effects.landmarkScanSampleStepMeters).toBe(1.2);
  expect(effects.surfaceTravelSpeedMultiplier).toBe(1);
  expect(effects.undergroundTravelSpeedMultiplier).toBe(1);
});

test("exploration skill effects expand landmark sensing with naturalist level", () => {
  const novice = describeExplorationSkillEffects({
    cartographyLevel: 1,
    naturalistLevel: 1,
    spelunkingLevel: 1,
  });
  const veteran = describeExplorationSkillEffects({
    cartographyLevel: 1,
    naturalistLevel: 8,
    spelunkingLevel: 1,
  });

  expect(veteran.landmarkScanRadiusMeters).toBeGreaterThan(novice.landmarkScanRadiusMeters);
  expect(veteran.landmarkScanRadiusMeters).toBeLessThanOrEqual(4.8);
});

test("exploration skill effects improve matching travel speed with traversal skills", () => {
  const novice = describeExplorationSkillEffects({
    cartographyLevel: 1,
    naturalistLevel: 1,
    spelunkingLevel: 1,
  });
  const veteran = describeExplorationSkillEffects({
    cartographyLevel: 7,
    naturalistLevel: 1,
    spelunkingLevel: 8,
  });

  expect(veteran.surfaceTravelSpeedMultiplier).toBeGreaterThan(novice.surfaceTravelSpeedMultiplier);
  expect(veteran.undergroundTravelSpeedMultiplier).toBeGreaterThan(novice.undergroundTravelSpeedMultiplier);
  expect(veteran.surfaceTravelSpeedMultiplier).toBeLessThanOrEqual(1.14);
  expect(veteran.undergroundTravelSpeedMultiplier).toBeLessThanOrEqual(1.14);
});

test("exploration skill effects clamp invalid levels to a safe baseline", () => {
  expect(describeExplorationSkillEffects({
    cartographyLevel: Number.NaN,
    naturalistLevel: Number.NaN,
    spelunkingLevel: Number.NaN,
  }).landmarkScanRadiusMeters)
    .toBe(BASE_LANDMARK_SCAN_RADIUS_METERS);
  expect(describeExplorationSkillEffects({
    cartographyLevel: -10,
    naturalistLevel: -10,
    spelunkingLevel: -10,
  }).landmarkScanRadiusMeters)
    .toBe(BASE_LANDMARK_SCAN_RADIUS_METERS);
});
