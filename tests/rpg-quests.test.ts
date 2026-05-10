import { expect, test } from "bun:test";

import {
  buildTravelGoalFromQuestHook,
  planRpgQuestHooks,
  selectRpgQuestHookForExploration,
  type RpgQuestHookSeed,
} from "../src/engine/rpg-quests.ts";

const FORBIDDEN_MATERIAL_WORDS = /\b(collect|gather|mine|craft|ore|log|plank|cobblestone|diamond|iron|coal)\b/i;

test("rpg quest planner creates stable deterministic hook ids from region route and landmark", () => {
  const input = {
    regionId: "red-mountain" as const,
    routeId: "pilgrim-spine-red" as const,
    landmarkId: "velothi_shrine",
  };

  const first = planRpgQuestHooks(input);
  const second = planRpgQuestHooks(input);

  expect(first).toEqual(second);
  expect(first.hooks.map((hook) => hook.id)).toEqual([
    "rpgq-pilgrimage-red-mountain-pilgrim-spine-red-velothi-shrine",
    "rpgq-cave-rumor-red-mountain-pilgrim-spine-red-velothi-shrine",
    "rpgq-faction-errand-red-mountain-pilgrim-spine-red-velothi-shrine",
    "rpgq-environmental-mystery-red-mountain-pilgrim-spine-red-velothi-shrine",
  ]);
  expect(first.hooks.map((hook) => hook.kind)).toEqual([
    "pilgrimage",
    "cave-rumor",
    "faction-errand",
    "environmental-mystery",
  ]);
});

test("rpg quest planner emits coherent hook and objective dependencies", () => {
  const plan = planRpgQuestHooks({
    regionId: "ashen-badlands",
    routeId: "ash-gash-pass",
    landmarkId: "kwama_mound",
  });
  const hookIds = new Set(plan.hooks.map((hook) => hook.id));

  for (const hook of plan.hooks) {
    const availableIds = new Set<string>([
      ...hook.dependencies.map((dependency) => dependency.id),
    ]);
    for (const objective of hook.objectives) {
      for (const dependencyId of objective.dependsOn) {
        expect(
          availableIds.has(dependencyId) || hookIds.has(dependencyId),
          `${objective.id} depends on missing ${dependencyId}`,
        ).toBe(true);
      }
      availableIds.add(objective.id);
    }
  }

  const factionErrand = requireHook(plan.hooks, "faction-errand");
  expect(factionErrand.dependencies.map((dependency) => dependency.id)).toContain(
    "rpgq-pilgrimage-ashen-badlands-ash-gash-pass-kwama-mound",
  );
  const mystery = requireHook(plan.hooks, "environmental-mystery");
  expect(mystery.dependencies.map((dependency) => dependency.id)).toContain(
    "rpgq-cave-rumor-ashen-badlands-ash-gash-pass-kwama-mound",
  );
});

test("rpg quest objectives avoid material gathering loops", () => {
  const plan = planRpgQuestHooks({
    regionId: "glass-shard-coast",
    routeId: "glass-coastal-cairns",
    landmarkId: "glass_cairn",
  });

  for (const hook of plan.hooks) {
    for (const objective of hook.objectives) {
      const objectiveText = `${objective.kind} ${objective.label} ${objective.journalText}`;
      expect(objectiveText).not.toMatch(FORBIDDEN_MATERIAL_WORDS);
      expect(["visit", "inspect", "listen", "interpret", "report"]).toContain(objective.kind);
    }
  }
});

test("rpg quest text and mood are region specific", () => {
  const redMountain = planRpgQuestHooks({
    regionId: "red-mountain",
    routeId: "pilgrim-spine-red",
    landmarkId: "ash_obelisk",
  });
  const bitterCoast = planRpgQuestHooks({
    regionId: "bitter-coast",
    routeId: "bitter-inner-crossing",
    landmarkId: "mangrove",
  });

  expect(redMountain.hooks[0]?.rumorText).toContain("Red Mountain");
  expect(redMountain.hooks[0]?.mood).toContain("ash");
  expect(redMountain.hooks[1]?.rumorText).toContain("lava tube");
  expect(bitterCoast.hooks[0]?.rumorText).toContain("Bitter Coast");
  expect(bitterCoast.hooks[0]?.mood).toContain("blackwater");
  expect(bitterCoast.hooks[1]?.rumorText).toContain("root grotto");
  expect(redMountain.hooks[0]?.mood).not.toBe(bitterCoast.hooks[0]?.mood);
});

test("rpg quest planner adapts hooks into travel goals", () => {
  const plan = planRpgQuestHooks({
    regionId: "red-mountain",
    routeId: "pilgrim-spine-red",
    landmarkId: "ash_obelisk",
  });

  const pilgrimage = requireHook(plan.hooks, "pilgrimage");
  const caveRumor = requireHook(plan.hooks, "cave-rumor");
  const goal = buildTravelGoalFromQuestHook(pilgrimage);
  const caveGoal = buildTravelGoalFromQuestHook(caveRumor);

  expect(goal).toMatchObject({
    id: "rpgq-pilgrimage-red-mountain-pilgrim-spine-red-ash-obelisk",
    routeId: "pilgrim-spine-red",
    title: "Red Mountain Pilgrim Bearings",
  });
  expect(goal?.steps.map((step) => [step.kind, step.targetId])).toEqual([
    ["visit", "pilgrim-spine-red"],
    ["inspect", "ash_obelisk"],
  ]);
  expect(goal?.steps.map((step) => step.kind)).not.toContain("listen");
  expect(goal?.steps.map((step) => step.kind)).not.toContain("read");
  expect(goal?.steps.map((step) => step.kind)).not.toContain("use");
  expect(caveGoal).toMatchObject({
    id: "rpgq-cave-rumor-red-mountain-pilgrim-spine-red-ash-obelisk",
    routeId: "pilgrim-spine-red",
    title: "Rumor of a Lava Tube",
  });
  expect(caveGoal?.steps.map((step) => [step.kind, step.targetId])).toEqual([
    ["listen", "red-mountain"],
    ["inspect", "ash_obelisk"],
  ]);
});

test("rpg quest exploration selector picks context-relevant hooks", () => {
  const plan = planRpgQuestHooks({
    regionId: "ashen-badlands",
    routeId: "ash-gash-pass",
    landmarkId: "kwama_mound",
  });

  expect(selectRpgQuestHookForExploration(plan, {
    nearCave: true,
    hasFaction: true,
    hasLandmark: true,
  })?.kind).toBe("cave-rumor");
  expect(selectRpgQuestHookForExploration(plan, {
    nearCave: false,
    hasFaction: true,
    hasLandmark: true,
  })?.kind).toBe("faction-errand");
  expect(selectRpgQuestHookForExploration(plan, {
    nearCave: false,
    hasFaction: false,
    hasLandmark: true,
  })?.kind).toBe("environmental-mystery");
  const fallback = selectRpgQuestHookForExploration(plan, {
    nearCave: false,
    hasFaction: false,
    hasLandmark: false,
  });

  expect(fallback?.kind).toBe("pilgrimage");
  expect(fallback?.objectiveLabel.length).toBeGreaterThan(0);
  expect(fallback?.rumorText).toContain("Ashen Badlands");
  expect(fallback?.objectiveTrainsSkillId).toBe("cartography");
});

test("rpg quest exploration selector advances to the next incomplete objective", () => {
  const plan = planRpgQuestHooks({
    regionId: "ashen-badlands",
    routeId: "ash-gash-pass",
    landmarkId: "kwama_mound",
  });
  const caveRumor = requireHook(plan.hooks, "cave-rumor");

  const selected = selectRpgQuestHookForExploration(plan, {
    nearCave: true,
    hasFaction: false,
    hasLandmark: true,
    completedObjectiveIdsByHookId: {
      [caveRumor.id]: [caveRumor.objectives[0]!.id],
    },
  });

  expect(selected).toMatchObject({
    kind: "cave-rumor",
    objectiveKind: "inspect",
    objectiveTargetId: "kwama_mound",
    objectiveTrainsSkillId: "naturalist",
  });
});

test("rpg quest summaries expose the skill trained by the active objective", () => {
  const plan = planRpgQuestHooks({
    regionId: "red-mountain",
    routeId: "pilgrim-spine-red",
    landmarkId: "ash_obelisk",
  });
  const caveRumor = requireHook(plan.hooks, "cave-rumor");
  const factionErrand = requireHook(plan.hooks, "faction-errand");

  const caveSummary = selectRpgQuestHookForExploration(plan, {
    nearCave: true,
    hasFaction: false,
    hasLandmark: true,
  });
  const factionSummary = selectRpgQuestHookForExploration(plan, {
    nearCave: false,
    hasFaction: true,
    hasLandmark: true,
    completedObjectiveIdsByHookId: {
      [factionErrand.id]: [factionErrand.objectives[0]!.id],
    },
  });

  expect(caveSummary).toMatchObject({
    hookId: caveRumor.id,
    objectiveKind: "listen",
    objectiveTrainsSkillId: "spelunking",
  });
  expect(factionSummary).toMatchObject({
    hookId: factionErrand.id,
    objectiveKind: "report",
    objectiveTrainsSkillId: "cartography",
  });
});

function requireHook(
  hooks: readonly RpgQuestHookSeed[],
  kind: RpgQuestHookSeed["kind"],
): RpgQuestHookSeed {
  const hook = hooks.find((candidate) => candidate.kind === kind);
  if (!hook) {
    throw new Error(`Missing hook: ${kind}`);
  }
  return hook;
}
