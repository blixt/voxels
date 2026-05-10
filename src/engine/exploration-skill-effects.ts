export interface ExplorationSkillEffectSource {
  cartographyLevel: number;
  naturalistLevel: number;
  spelunkingLevel: number;
}

export interface ExplorationSkillEffects {
  landmarkScanRadiusMeters: number;
  landmarkScanSampleStepMeters: number;
  surfaceTravelSpeedMultiplier: number;
  undergroundTravelSpeedMultiplier: number;
}

export const BASE_LANDMARK_SCAN_RADIUS_METERS = 2.4;
export const LANDMARK_SCAN_SAMPLE_STEP_METERS = 1.2;
const LANDMARK_SCAN_RADIUS_BONUS_PER_LEVEL_METERS = 0.35;
const MAX_LANDMARK_SCAN_RADIUS_METERS = 4.8;
const TRAVEL_SPEED_BONUS_PER_LEVEL = 0.02;
const MAX_TRAVEL_SPEED_MULTIPLIER = 1.14;

export function describeExplorationSkillEffects(
  source: ExplorationSkillEffectSource,
): ExplorationSkillEffects {
  const cartographyLevel = readPositiveLevel(source.cartographyLevel);
  const naturalistLevel = readPositiveLevel(source.naturalistLevel);
  const spelunkingLevel = readPositiveLevel(source.spelunkingLevel);
  const bonusMeters = (naturalistLevel - 1) * LANDMARK_SCAN_RADIUS_BONUS_PER_LEVEL_METERS;
  return {
    landmarkScanRadiusMeters: Math.min(
      MAX_LANDMARK_SCAN_RADIUS_METERS,
      BASE_LANDMARK_SCAN_RADIUS_METERS + bonusMeters,
    ),
    landmarkScanSampleStepMeters: LANDMARK_SCAN_SAMPLE_STEP_METERS,
    surfaceTravelSpeedMultiplier: resolveTravelSpeedMultiplier(cartographyLevel),
    undergroundTravelSpeedMultiplier: resolveTravelSpeedMultiplier(spelunkingLevel),
  };
}

function resolveTravelSpeedMultiplier(level: number): number {
  return Math.min(MAX_TRAVEL_SPEED_MULTIPLIER, 1 + (level - 1) * TRAVEL_SPEED_BONUS_PER_LEVEL);
}

function readPositiveLevel(value: number): number {
  return Number.isFinite(value)
    ? Math.max(1, Math.floor(value))
    : 1;
}
