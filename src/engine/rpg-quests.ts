import {
  findAtlasRegion,
  findAtlasRoute,
  type AtlasRegionId,
  type AtlasRouteId,
} from "./world-atlas.ts";
import { describeDiscovery } from "./discovery-catalog.ts";
import type { LandmarkId } from "./procedural-generator.ts";
import type { SkillId } from "./skill-journal.ts";
import type {
  TravelGoalDefinition,
  TravelGoalStepDefinition,
  TravelGoalStepKind,
} from "./travel-goals.ts";

export const RPG_QUESTS_VERSION = 1;

export type RpgQuestHookKind =
  | "pilgrimage"
  | "cave-rumor"
  | "faction-errand"
  | "environmental-mystery";

export type RpgQuestObjectiveKind = "visit" | "inspect" | "listen" | "interpret" | "report";
export type RpgQuestDependencyKind = "region" | "route" | "landmark" | "quest" | "objective";

export interface RpgQuestPlanningInput {
  regionId: AtlasRegionId;
  routeId?: AtlasRouteId | null;
  landmarkId?: LandmarkId | string | null;
}

export interface RpgQuestDependency {
  id: string;
  kind: RpgQuestDependencyKind;
  label: string;
}

export interface RpgQuestObjectiveSeed {
  id: string;
  kind: RpgQuestObjectiveKind;
  targetId: string;
  label: string;
  journalText: string;
  trainsSkillId: SkillId;
  dependsOn: readonly string[];
}

export interface RpgQuestHookSeed {
  id: string;
  kind: RpgQuestHookKind;
  regionId: AtlasRegionId;
  routeId: AtlasRouteId | null;
  landmarkId: string | null;
  title: string;
  rumorText: string;
  mood: string;
  faction: string | null;
  dependencies: readonly RpgQuestDependency[];
  objectives: readonly RpgQuestObjectiveSeed[];
}

export interface RpgQuestPlan {
  version: typeof RPG_QUESTS_VERSION;
  context: {
    regionId: AtlasRegionId;
    routeId: AtlasRouteId | null;
    landmarkId: string | null;
  };
  hooks: readonly RpgQuestHookSeed[];
}

export interface RpgQuestExplorationSignal {
  nearCave: boolean;
  hasFaction: boolean;
  hasLandmark: boolean;
  completedObjectiveIdsByHookId?: Readonly<Record<string, readonly string[]>>;
}

export interface RpgQuestHookSummary {
  hookId: string;
  kind: RpgQuestHookKind;
  title: string;
  rumorText: string;
  mood: string;
  faction: string | null;
  objectiveId: string;
  objectiveKind: RpgQuestObjectiveKind;
  objectiveTargetId: string;
  objectiveLabel: string;
  objectiveJournalText: string;
}

interface RegionQuestProfile {
  name: string;
  mood: string;
  faction: string;
  routeNoun: string;
  caveNoun: string;
  mysteryNoun: string;
  sensoryDetail: string;
}

const REGION_PROFILES: Record<AtlasRegionId, RegionQuestProfile> = {
  "inner-sea": {
    name: "Inner Sea",
    mood: "silt-mist, wet stone, and half-remembered shrine vows",
    faction: "Temple waykeepers",
    routeNoun: "shelf road",
    caveNoun: "silt undercroft",
    mysteryNoun: "tidal omen",
    sensoryDetail: "brackish wind moves over low moor water",
  },
  "red-mountain": {
    name: "Red Mountain",
    mood: "ash pressure, red light, and devotional dread",
    faction: "ash shrine custodians",
    routeNoun: "caldera pilgrim road",
    caveNoun: "lava tube",
    mysteryNoun: "ashfall sign",
    sensoryDetail: "warm grit taps against black basalt",
  },
  "ashen-badlands": {
    name: "Ashen Badlands",
    mood: "dry ash, exposed bones, and hard caravan choices",
    faction: "badlands caravan stewards",
    routeNoun: "ash trail",
    caveNoun: "kwama ravine",
    mysteryNoun: "buried road sign",
    sensoryDetail: "dust hisses through rib arches and cairns",
  },
  "bitter-coast": {
    name: "Bitter Coast",
    mood: "blackwater hush, root shadow, and watchful reeds",
    faction: "coast marshwardens",
    routeNoun: "wetland crossing",
    caveNoun: "root grotto",
    mysteryNoun: "blackwater reflection",
    sensoryDetail: "reed beds click while peat water swallows footsteps",
  },
  grazelands: {
    name: "Grazelands",
    mood: "sun-dried grass, open distance, and tense hospitality",
    faction: "grassland camp speakers",
    routeNoun: "eastward camp trail",
    caveNoun: "wind-cut shelter",
    mysteryNoun: "flowersea mirage",
    sensoryDetail: "yellow grass bends around cairns in long waves",
  },
  "salt-marsh-basin": {
    name: "Salt Marsh Basin",
    mood: "white glare, cracked salt, and quiet disorientation",
    faction: "causeway surveyors",
    routeNoun: "salt causeway",
    caveNoun: "saline sinkhole",
    mysteryNoun: "mirror-crust echo",
    sensoryDetail: "the flats throw every footstep back as pale light",
  },
  "glass-shard-coast": {
    name: "Glass Shard Coast",
    mood: "cold glass, coastal warning cairns, and brittle silence",
    faction: "glass coast wardens",
    routeNoun: "hazard cairn chain",
    caveNoun: "crystal cavern",
    mysteryNoun: "singing shard field",
    sensoryDetail: "shards ring softly under a hard coastal wind",
  },
  "west-gash": {
    name: "West Gash",
    mood: "redleaf shade, ravine stone, and sheltered rumors",
    faction: "ravine pass guides",
    routeNoun: "switchback pass",
    caveNoun: "granitic cave",
    mysteryNoun: "redleaf stone mark",
    sensoryDetail: "red leaves collect where the ravine wind drops",
  },
};

export function planRpgQuestHooks(input: RpgQuestPlanningInput): RpgQuestPlan {
  const region = findAtlasRegion(input.regionId);
  const route = input.routeId ? findAtlasRoute(input.routeId) : null;
  const landmarkId = input.landmarkId ?? pickDefaultLandmark(input.regionId);
  const profile = REGION_PROFILES[input.regionId];
  const routeId = route?.id ?? null;
  const contextDependencies = buildContextDependencies(input.regionId, routeId, landmarkId);
  const hooks = [
    buildPilgrimageHook(input.regionId, routeId, landmarkId, profile, contextDependencies),
    buildCaveRumorHook(input.regionId, routeId, landmarkId, profile, contextDependencies),
    buildFactionErrandHook(input.regionId, routeId, landmarkId, profile, contextDependencies),
    buildEnvironmentalMysteryHook(input.regionId, routeId, landmarkId, profile, contextDependencies),
  ];

  return {
    version: RPG_QUESTS_VERSION,
    context: {
      regionId: region.id,
      routeId,
      landmarkId,
    },
    hooks,
  };
}

export function buildTravelGoalFromQuestHook(hook: RpgQuestHookSeed): TravelGoalDefinition | null {
  const steps = hook.objectives.flatMap((objective): TravelGoalStepDefinition[] => {
    const kind = travelGoalStepKindForQuestObjective(objective.kind);
    if (!kind) {
      return [];
    }
    return [{
      id: objective.id,
      kind,
      targetId: objective.targetId,
      label: objective.label,
    }];
  });
  if (steps.length === 0) {
    return null;
  }
  return {
    id: hook.id,
    routeId: hook.routeId ?? hook.regionId,
    title: hook.title,
    journalText: hook.rumorText,
    steps,
  };
}

export function selectRpgQuestHookForExploration(
  plan: RpgQuestPlan,
  signal: RpgQuestExplorationSignal,
): RpgQuestHookSummary | null {
  const preferredKind: RpgQuestHookKind = signal.nearCave
    ? "cave-rumor"
    : signal.hasFaction
    ? "faction-errand"
    : signal.hasLandmark
    ? "environmental-mystery"
    : "pilgrimage";
  const hook = plan.hooks.find((candidate) => candidate.kind === preferredKind)
    ?? plan.hooks.find((candidate) => candidate.kind === "pilgrimage")
    ?? plan.hooks[0]
    ?? null;
  if (!hook) {
    return null;
  }
  const completedObjectiveIds = new Set(signal.completedObjectiveIdsByHookId?.[hook.id] ?? []);
  const objective = hook.objectives.find((candidate) => !completedObjectiveIds.has(candidate.id))
    ?? hook.objectives[hook.objectives.length - 1]
    ?? null;
  return {
    hookId: hook.id,
    kind: hook.kind,
    title: hook.title,
    rumorText: hook.rumorText,
    mood: hook.mood,
    faction: hook.faction,
    objectiveId: objective?.id ?? `${hook.id}:local-sign`,
    objectiveKind: objective?.kind ?? "inspect",
    objectiveTargetId: objective?.targetId ?? hook.landmarkId ?? hook.routeId ?? hook.regionId,
    objectiveLabel: objective?.label ?? "Read the local signs",
    objectiveJournalText: objective?.journalText ?? hook.rumorText,
  };
}

function buildPilgrimageHook(
  regionId: AtlasRegionId,
  routeId: AtlasRouteId | null,
  landmarkId: string | null,
  profile: RegionQuestProfile,
  dependencies: readonly RpgQuestDependency[],
): RpgQuestHookSeed {
  const id = hookId("pilgrimage", regionId, routeId, landmarkId);
  const firstObjectiveId = objectiveId(id, "read-sign");
  return {
    id,
    kind: "pilgrimage",
    regionId,
    routeId,
    landmarkId,
    title: `${profile.name} Pilgrim Bearings`,
    rumorText: `${profile.faction} say the ${profile.routeNoun} through ${profile.name} can still be followed by patient eyes. ${profile.sensoryDetail}.`,
    mood: profile.mood,
    faction: profile.faction,
    dependencies,
    objectives: [
      {
        id: firstObjectiveId,
        kind: "visit",
        targetId: routeId ?? regionId,
        label: `Walk the ${profile.routeNoun}`,
        journalText: `Trace the ${profile.routeNoun} until the local pilgrimage logic becomes legible.`,
        trainsSkillId: "cartography",
        dependsOn: dependencyIds(dependencies, ["region", "route"]),
      },
      {
        id: objectiveId(id, "read-landmark"),
        kind: "inspect",
        targetId: landmarkId ?? regionId,
        label: `Interpret the ${landmarkName(landmarkId)}`,
        journalText: `Read the landmark as a signpost instead of a resource node.`,
        trainsSkillId: "lore",
        dependsOn: [firstObjectiveId],
      },
    ],
  };
}

function buildCaveRumorHook(
  regionId: AtlasRegionId,
  routeId: AtlasRouteId | null,
  landmarkId: string | null,
  profile: RegionQuestProfile,
  dependencies: readonly RpgQuestDependency[],
): RpgQuestHookSeed {
  const id = hookId("cave-rumor", regionId, routeId, landmarkId);
  const firstObjectiveId = objectiveId(id, "ask-about-mouth");
  return {
    id,
    kind: "cave-rumor",
    regionId,
    routeId,
    landmarkId,
    title: `Rumor of a ${titleCaseIdentifier(profile.caveNoun)}`,
    rumorText: `Travelers in ${profile.name} lower their voices around the ${profile.caveNoun}; the story changes, but the entrance always lies near a known sign.`,
    mood: profile.mood,
    faction: null,
    dependencies,
    objectives: [
      {
        id: firstObjectiveId,
        kind: "listen",
        targetId: regionId,
        label: `Listen for the ${profile.caveNoun} rumor`,
        journalText: `Compare the local cave story against the route and landmark record.`,
        trainsSkillId: "spelunking",
        dependsOn: dependencyIds(dependencies, ["region"]),
      },
      {
        id: objectiveId(id, "inspect-threshold"),
        kind: "inspect",
        targetId: landmarkId ?? routeId ?? regionId,
        label: `Inspect signs around the suspected ${profile.caveNoun}`,
        journalText: `Look for airflow, echo, dampness, or shrine marks that make the rumor physically plausible.`,
        trainsSkillId: "naturalist",
        dependsOn: [firstObjectiveId],
      },
    ],
  };
}

function buildFactionErrandHook(
  regionId: AtlasRegionId,
  routeId: AtlasRouteId | null,
  landmarkId: string | null,
  profile: RegionQuestProfile,
  dependencies: readonly RpgQuestDependency[],
): RpgQuestHookSeed {
  const id = hookId("faction-errand", regionId, routeId, landmarkId);
  const pilgrimageId = hookId("pilgrimage", regionId, routeId, landmarkId);
  const firstObjectiveId = objectiveId(id, "take-message");
  return {
    id,
    kind: "faction-errand",
    regionId,
    routeId,
    landmarkId,
    title: `${titleCaseIdentifier(profile.faction)} Errand`,
    rumorText: `${profile.faction} need a reliable traveler to carry a warning through ${profile.name}, not another hand hired to haul supplies.`,
    mood: profile.mood,
    faction: profile.faction,
    dependencies: [
      ...dependencies,
      {
        id: pilgrimageId,
        kind: "quest",
        label: "Pilgrim bearings established",
      },
    ],
    objectives: [
      {
        id: firstObjectiveId,
        kind: "listen",
        targetId: profile.faction,
        label: `Hear the ${profile.faction} request`,
        journalText: `Learn who is waiting, what sign proves the route, and why the errand matters.`,
        trainsSkillId: "lore",
        dependsOn: [pilgrimageId],
      },
      {
        id: objectiveId(id, "deliver-warning"),
        kind: "report",
        targetId: routeId ?? landmarkId ?? regionId,
        label: `Report along the ${profile.routeNoun}`,
        journalText: `Carry the warning by route knowledge and local judgment.`,
        trainsSkillId: "cartography",
        dependsOn: [firstObjectiveId],
      },
    ],
  };
}

function buildEnvironmentalMysteryHook(
  regionId: AtlasRegionId,
  routeId: AtlasRouteId | null,
  landmarkId: string | null,
  profile: RegionQuestProfile,
  dependencies: readonly RpgQuestDependency[],
): RpgQuestHookSeed {
  const id = hookId("environmental-mystery", regionId, routeId, landmarkId);
  const caveRumorId = hookId("cave-rumor", regionId, routeId, landmarkId);
  const firstObjectiveId = objectiveId(id, "observe-pattern");
  return {
    id,
    kind: "environmental-mystery",
    regionId,
    routeId,
    landmarkId,
    title: `${titleCaseIdentifier(profile.mysteryNoun)} Mystery`,
    rumorText: `Something in ${profile.name} is out of rhythm: ${profile.mysteryNoun}, ${profile.sensoryDetail}, and the landmark record do not agree.`,
    mood: profile.mood,
    faction: null,
    dependencies: [
      ...dependencies,
      {
        id: caveRumorId,
        kind: "quest",
        label: "Cave rumor cross-checked",
      },
    ],
    objectives: [
      {
        id: firstObjectiveId,
        kind: "inspect",
        targetId: landmarkId ?? regionId,
        label: `Observe the ${profile.mysteryNoun}`,
        journalText: `Treat the place as evidence: light, sound, growth, weather, and old road placement.`,
        trainsSkillId: "naturalist",
        dependsOn: dependencyIds(dependencies, ["region", "landmark"]),
      },
      {
        id: objectiveId(id, "compare-rumor"),
        kind: "interpret",
        targetId: caveRumorId,
        label: `Compare the mystery with the cave rumor`,
        journalText: `Decide whether the local story explains the environmental pattern or hides another route.`,
        trainsSkillId: "lore",
        dependsOn: [firstObjectiveId, caveRumorId],
      },
    ],
  };
}

function buildContextDependencies(
  regionId: AtlasRegionId,
  routeId: AtlasRouteId | null,
  landmarkId: string | null,
): readonly RpgQuestDependency[] {
  const dependencies: RpgQuestDependency[] = [
    {
      id: dependencyId("region", regionId),
      kind: "region",
      label: `Discover ${REGION_PROFILES[regionId].name}`,
    },
  ];
  if (routeId) {
    dependencies.push({
      id: dependencyId("route", routeId),
      kind: "route",
      label: `Trace ${titleCaseIdentifier(routeId)}`,
    });
  }
  if (landmarkId) {
    dependencies.push({
      id: dependencyId("landmark", landmarkId),
      kind: "landmark",
      label: `Notice ${landmarkName(landmarkId)}`,
    });
  }
  return dependencies;
}

function pickDefaultLandmark(regionId: AtlasRegionId): string {
  switch (regionId) {
    case "inner-sea":
      return "velothi_shrine";
    case "red-mountain":
      return "ash_obelisk";
    case "ashen-badlands":
      return "buried_ribs";
    case "bitter-coast":
      return "mangrove";
    case "grazelands":
      return "standing_stone";
    case "salt-marsh-basin":
      return "salt_spire";
    case "glass-shard-coast":
      return "glass_cairn";
    case "west-gash":
      return "redleaf_tree";
  }
}

function hookId(
  kind: RpgQuestHookKind,
  regionId: AtlasRegionId,
  routeId: AtlasRouteId | null,
  landmarkId: string | null,
): string {
  return `rpgq-${kind}-${regionId}-${routeId ?? "open"}-${slug(landmarkId ?? "wild")}`;
}

function objectiveId(hookIdValue: string, objectiveSlug: string): string {
  return `${hookIdValue}-${objectiveSlug}`;
}

function dependencyId(kind: Exclude<RpgQuestDependencyKind, "quest" | "objective">, id: string): string {
  return `${kind}:${id}`;
}

function dependencyIds(
  dependencies: readonly RpgQuestDependency[],
  kinds: readonly RpgQuestDependencyKind[],
): readonly string[] {
  return dependencies
    .filter((dependency) => kinds.includes(dependency.kind))
    .map((dependency) => dependency.id);
}

function landmarkName(landmarkId: string | null): string {
  return landmarkId ? describeDiscovery("landmark", landmarkId).name : "open country";
}

function titleCaseIdentifier(value: string): string {
  return value
    .split(/[-_\s]+/g)
    .filter(Boolean)
    .map((part) => part.slice(0, 1).toUpperCase() + part.slice(1))
    .join(" ");
}

function slug(value: string): string {
  return value.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || "seed";
}

function travelGoalStepKindForQuestObjective(kind: RpgQuestObjectiveKind): TravelGoalStepKind | null {
  switch (kind) {
    case "visit":
    case "inspect":
    case "listen":
    case "interpret":
    case "report":
      return kind;
  }
}
