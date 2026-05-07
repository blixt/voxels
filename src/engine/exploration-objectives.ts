export interface ExplorationObjectiveSource {
  discoveredBiomeCount: number;
  discoveredUndergroundBiomeCount: number;
  discoveredRegionalVariantCount: number;
  discoveredLandmarkCount: number;
  discoveredAncientLandmarkCount: number;
  collectedMaterialCount: number;
}

export interface ExplorationObjective {
  id: string;
  label: string;
  progress: number;
  target: number;
  completed: boolean;
}

export interface ExplorationObjectiveSnapshot {
  stageId: string;
  title: string;
  subtitle: string;
  completedCount: number;
  totalCount: number;
  objectives: ExplorationObjective[];
}

export function describeExplorationObjectives(
  source: ExplorationObjectiveSource,
): ExplorationObjectiveSnapshot {
  const surveyObjectives = [
    buildObjective("biomes-3", "Survey 3 surface biomes", source.discoveredBiomeCount, 3),
    buildObjective("landmarks-3", "Catalog 3 landmarks", source.discoveredLandmarkCount, 3),
    buildObjective("colors-4", "Identify 4 field materials", source.collectedMaterialCount, 4),
  ];
  if (!allObjectivesComplete(surveyObjectives)) {
    return buildSnapshot(
      "surface-survey",
      "Surface Survey",
      "Build your first mental map of the world.",
      surveyObjectives,
    );
  }

  const frontierObjectives = [
    buildObjective("biomes-6", "Survey 6 surface biomes", source.discoveredBiomeCount, 6),
    buildObjective("variants-2", "Find 2 regional variants", source.discoveredRegionalVariantCount, 2),
    buildObjective("ancient-signs-2", "Trace 2 old road signs", source.discoveredAncientLandmarkCount, 2),
    buildObjective("colors-8", "Identify 8 field materials", source.collectedMaterialCount, 8),
    buildObjective("underground-1", "Enter the first underground biome", source.discoveredUndergroundBiomeCount, 1),
  ];
  if (!allObjectivesComplete(frontierObjectives)) {
    return buildSnapshot(
      "frontier-atlas",
      "Frontier Atlas",
      "Push outward and start tracing the world's stranger edges.",
      frontierObjectives,
    );
  }

  const deepObjectives = [
    buildObjective("biomes-10", "Survey 10 surface biomes", source.discoveredBiomeCount, 10),
    buildObjective("landmarks-8", "Catalog 8 landmarks", source.discoveredLandmarkCount, 8),
    buildObjective("variants-4", "Find 4 regional variants", source.discoveredRegionalVariantCount, 4),
    buildObjective("ancient-signs-4", "Trace 4 old road signs", source.discoveredAncientLandmarkCount, 4),
    buildObjective("underground-3", "Enter 3 underground biomes", source.discoveredUndergroundBiomeCount, 3),
    buildObjective("colors-16", "Identify 16 field materials", source.collectedMaterialCount, 16),
  ];
  return buildSnapshot(
    "deep-expedition",
    "Deep Expedition",
    "Range farther, descend deeper, and broaden the palette.",
    deepObjectives,
  );
}

function buildObjective(id: string, label: string, progress: number, target: number): ExplorationObjective {
  const clampedProgress = Math.max(0, Math.min(progress, target));
  return {
    id,
    label,
    progress: clampedProgress,
    target,
    completed: clampedProgress >= target,
  };
}

function buildSnapshot(
  stageId: string,
  title: string,
  subtitle: string,
  objectives: ExplorationObjective[],
): ExplorationObjectiveSnapshot {
  const completedCount = objectives.reduce((count, objective) => count + (objective.completed ? 1 : 0), 0);
  return {
    stageId,
    title,
    subtitle,
    completedCount,
    totalCount: objectives.length,
    objectives,
  };
}

function allObjectivesComplete(objectives: readonly ExplorationObjective[]): boolean {
  return objectives.every((objective) => objective.completed);
}
