export interface InteractionSkillGateSource {
  cartographyLevel: number;
  naturalistLevel: number;
  spelunkingLevel: number;
  loreLevel: number;
}

export interface InteractionSkillGateHints {
  canReadRoadMarks: boolean;
  canIdentifyForageQuality: boolean;
  canAssessCaveRoute: boolean;
  canInterpretLore: boolean;
  roadMarkHint: string;
  forageHint: string;
  caveRouteHint: string;
  loreHint: string;
}

export function describeInteractionSkillGates(
  source: InteractionSkillGateSource,
): InteractionSkillGateHints {
  const cartographyLevel = readLevel(source.cartographyLevel);
  const naturalistLevel = readLevel(source.naturalistLevel);
  const spelunkingLevel = readLevel(source.spelunkingLevel);
  const loreLevel = readLevel(source.loreLevel);
  const canReadRoadMarks = cartographyLevel >= 2;
  const canIdentifyForageQuality = naturalistLevel >= 2;
  const canAssessCaveRoute = spelunkingLevel >= 2;
  const canInterpretLore = loreLevel >= 2;
  return {
    canReadRoadMarks,
    canIdentifyForageQuality,
    canAssessCaveRoute,
    canInterpretLore,
    roadMarkHint: canReadRoadMarks
      ? "Cartography reads the older route marks clearly."
      : "Cartography 2 would decode older route marks.",
    forageHint: canIdentifyForageQuality
      ? "Naturalist identifies the safest cut and likely yield."
      : "Naturalist 2 would identify quality and safe harvest.",
    caveRouteHint: canAssessCaveRoute
      ? "Spelunking reads the safer descent and return path."
      : "Spelunking 2 would judge the safest cave route.",
    loreHint: canInterpretLore
      ? "Lore interprets the pilgrim meaning behind the marks."
      : "Lore 2 would interpret the deeper pilgrim meaning.",
  };
}

function readLevel(value: number): number {
  return Number.isFinite(value) ? Math.max(1, Math.floor(value)) : 1;
}
