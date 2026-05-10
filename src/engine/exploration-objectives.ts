export interface ExplorationObjectiveSource {
  discoveredBiomeCount: number;
  discoveredUndergroundBiomeCount: number;
  discoveredRegionalVariantCount: number;
  discoveredLandmarkCount: number;
  discoveredAncientLandmarkCount: number;
  scoutedMobTrailCount?: number;
  lootedCacheCount?: number;
  scoutedCaveMouthCount?: number;
  traversedCavePassageCount?: number;
}

export interface ExplorationObjective {
  id: string;
  label: string;
  journalText: string;
  progress: number;
  target: number;
  completed: boolean;
}

export interface ExplorationObjectiveSnapshot {
  stageId: string;
  title: string;
  subtitle: string;
  journalText: string;
  progressionHint: string;
  completedCount: number;
  totalCount: number;
  objectives: ExplorationObjective[];
}

export function describeExplorationObjectives(
  source: ExplorationObjectiveSource,
): ExplorationObjectiveSnapshot {
  const surveyObjectives = [
    buildObjective(
      "biomes-3",
      "Map 3 regions",
      "Make a first map from the land, not a menu.",
      source.discoveredBiomeCount,
      3,
    ),
    buildObjective(
      "old-road-1",
      "Find an old road sign",
      "Look for causeways, lanterns, shrines, cairns, and ash markers.",
      source.discoveredAncientLandmarkCount,
      1,
    ),
    buildObjective(
      "landmarks-3",
      "Catalog 3 landmarks",
      "Every landmark fixes another point on the route.",
      source.discoveredLandmarkCount,
      3,
    ),
    buildObjective(
      "mob-trails-1",
      "Scout 1 local trail sign",
      "Tracks, spoor, and camp traces make nearby pressure readable.",
      readObjectiveCount(source.scoutedMobTrailCount),
      1,
    ),
  ];
  if (!allObjectivesComplete(surveyObjectives)) {
    return buildSnapshot(
      "first-bearings",
      "First Bearings",
      "Read the land by roads, shrines, and landmarks.",
      "The first useful map is a chain of remembered places.",
      "Travel trains Cartography; discoveries train exploration skills.",
      surveyObjectives,
    );
  }

  const frontierObjectives = [
    buildObjective(
      "old-road-2",
      "Trace 2 old road signs",
      "Follow ziggurats, arches, and road stones until they form a route.",
      source.discoveredAncientLandmarkCount,
      2,
    ),
    buildObjective(
      "landmarks-6",
      "Catalog 6 landmarks",
      "Landmarks are the fast-travel system before fast travel.",
      source.discoveredLandmarkCount,
      6,
    ),
    buildObjective(
      "variants-2",
      "Find 2 strange regions",
      "Odd borders often hide older paths.",
      source.discoveredRegionalVariantCount,
      2,
    ),
    buildObjective(
      "loot-caches-2",
      "Recover 2 field finds",
      "Use caches, forage, and salvage to turn wandering into supplies.",
      readObjectiveCount(source.lootedCacheCount),
      2,
    ),
    buildObjective(
      "cave-mouths-1",
      "Scout 1 cave mouth",
      "Surface tells mark where authored cave systems begin.",
      readObjectiveCount(source.scoutedCaveMouthCount),
      1,
    ),
    buildObjective(
      "underground-1",
      "Enter an undercroft",
      "Some roads continue below the surface.",
      source.discoveredUndergroundBiomeCount,
      1,
    ),
  ];
  if (!allObjectivesComplete(frontierObjectives)) {
    return buildSnapshot(
      "pilgrim-road",
      "Pilgrim Road",
      "Use old signs to push past familiar ground.",
      "Shrines and cairns turn wandering into a route.",
      "Landmark discoveries train Naturalist; strange regions train Lore.",
      frontierObjectives,
    );
  }

  const deepObjectives = [
    buildObjective(
      "old-road-4",
      "Trace 4 pilgrim signs",
      "The old road is a story told in missing stones.",
      source.discoveredAncientLandmarkCount,
      4,
    ),
    buildObjective(
      "biomes-10",
      "Map 10 regions",
      "Range far enough for the world to stop repeating itself.",
      source.discoveredBiomeCount,
      10,
    ),
    buildObjective(
      "variants-4",
      "Find 4 strange regions",
      "Record the places that do not match their neighbors.",
      source.discoveredRegionalVariantCount,
      4,
    ),
    buildObjective(
      "underground-3",
      "Enter 3 underground biomes",
      "Descend until the underground map has its own regions.",
      source.discoveredUndergroundBiomeCount,
      3,
    ),
    buildObjective(
      "cave-passages-1",
      "Follow 1 cave passage",
      "Use cave links as routes, not only as isolated entrances.",
      readObjectiveCount(source.traversedCavePassageCount),
      1,
    ),
    buildObjective(
      "mob-trails-5",
      "Scout 5 encounter signs",
      "Learn which factions and beasts own the routes before pushing deeper.",
      readObjectiveCount(source.scoutedMobTrailCount),
      5,
    ),
    buildObjective(
      "loot-caches-5",
      "Recover 5 field finds",
      "Build a practical field kit from local caches, forage, and salvage.",
      readObjectiveCount(source.lootedCacheCount),
      5,
    ),
    buildObjective(
      "cave-mouths-3",
      "Scout 3 cave mouths",
      "Read cave thresholds before committing to the dark routes below.",
      readObjectiveCount(source.scoutedCaveMouthCount),
      3,
    ),
    buildObjective(
      "landmarks-12",
      "Catalog 12 landmarks",
      "A full journal should read like a route across the island.",
      source.discoveredLandmarkCount,
      12,
    ),
  ];
  return buildSnapshot(
    "deep-pilgrimage",
    "Deep Pilgrimage",
    "Connect roads, ruins, caves, and distant landmarks.",
    "The route is no longer a line; it is a memory palace.",
    "Cartography, Naturalist, Lore, and Spelunking all grow through use.",
    deepObjectives,
  );
}

function buildObjective(
  id: string,
  label: string,
  journalText: string,
  progress: number,
  target: number,
): ExplorationObjective {
  const clampedProgress = Math.max(0, Math.min(progress, target));
  return {
    id,
    label,
    journalText,
    progress: clampedProgress,
    target,
    completed: clampedProgress >= target,
  };
}

function buildSnapshot(
  stageId: string,
  title: string,
  subtitle: string,
  journalText: string,
  progressionHint: string,
  objectives: ExplorationObjective[],
): ExplorationObjectiveSnapshot {
  const completedCount = objectives.reduce((count, objective) => count + (objective.completed ? 1 : 0), 0);
  return {
    stageId,
    title,
    subtitle,
    journalText,
    progressionHint,
    completedCount,
    totalCount: objectives.length,
    objectives,
  };
}

function allObjectivesComplete(objectives: readonly ExplorationObjective[]): boolean {
  return objectives.every((objective) => objective.completed);
}

function readObjectiveCount(value: number | undefined): number {
  return typeof value === "number" && Number.isFinite(value) ? Math.max(0, Math.floor(value)) : 0;
}
