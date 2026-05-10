import { expect, test } from "bun:test";

import {
  buildRpgQuestTopicCandidateId,
  buildRpgQuestTopicInteractionCandidate,
} from "../src/engine/rpg-quest-topics.ts";
import {
  planRpgQuestHooks,
  selectRpgQuestHookForExploration,
} from "../src/engine/rpg-quests.ts";

test("quest topic interaction candidates use deterministic ids and concrete listen prompts", () => {
  const plan = planRpgQuestHooks({
    regionId: "red-mountain",
    routeId: "pilgrim-spine-red",
    landmarkId: "ash_obelisk",
  });
  const quest = selectRpgQuestHookForExploration(plan, {
    nearCave: true,
    hasFaction: false,
    hasLandmark: true,
  });

  const candidate = buildRpgQuestTopicInteractionCandidate({
    quest,
    worldPosition: [1, 2, 3],
    regionId: "red-mountain",
    routeId: "pilgrim-spine-red",
  });

  expect(candidate).toMatchObject({
    id: buildRpgQuestTopicCandidateId({
      hookId: "rpgq-cave-rumor-red-mountain-pilgrim-spine-red-ash-obelisk",
      objectiveId: "rpgq-cave-rumor-red-mountain-pilgrim-spine-red-ash-obelisk-ask-about-mouth",
    }),
    subjectType: "route",
    role: "quest-topic",
    name: "Rumor of a Lava Tube",
    worldPosition: [1, 2, 3],
    prompts: [{
      verb: "listen",
      label: "Listen for the lava tube rumor",
      description: "Compare the local cave story against the route and landmark record.",
    }],
    skillAwards: [{
      skillId: "spelunking",
      xp: 18,
      reason: "Quest topic: Listen for the lava tube rumor",
      awardKey: "quest-topic:rpgq-cave-rumor-red-mountain-pilgrim-spine-red-ash-obelisk:rpgq-cave-rumor-red-mountain-pilgrim-spine-red-ash-obelisk-ask-about-mouth",
      onceOnly: true,
    }],
    payload: {
      hookId: "rpgq-cave-rumor-red-mountain-pilgrim-spine-red-ash-obelisk",
      hookKind: "cave-rumor",
      objectiveId: "rpgq-cave-rumor-red-mountain-pilgrim-spine-red-ash-obelisk-ask-about-mouth",
      objectiveKind: "listen",
      objectiveTargetId: "red-mountain",
      routeId: "pilgrim-spine-red",
      regionId: "red-mountain",
      trainsSkillId: "spelunking",
    },
  });
});

test("quest topic interaction candidates carry faction report payloads", () => {
  const plan = planRpgQuestHooks({
    regionId: "bitter-coast",
    routeId: "bitter-inner-crossing",
    landmarkId: "mangrove",
  });
  const factionHook = plan.hooks.find((hook) => hook.kind === "faction-errand")!;
  const quest = selectRpgQuestHookForExploration(plan, {
    nearCave: false,
    hasFaction: true,
    hasLandmark: true,
    completedObjectiveIdsByHookId: {
      [factionHook.id]: [factionHook.objectives[0]!.id],
    },
  });

  const candidate = buildRpgQuestTopicInteractionCandidate({
    quest,
    worldPosition: [4, 5, 6],
    regionId: "bitter-coast",
    routeId: "bitter-inner-crossing",
  });

  expect(candidate).toMatchObject({
    subjectType: "npc",
    name: "coast marshwardens",
    role: "quest-topic",
    prompts: [{
      verb: "report",
      label: "Report along the wetland crossing",
    }],
    skillAwards: [{
      skillId: "cartography",
      awardKey: "quest-topic:rpgq-faction-errand-bitter-coast-bitter-inner-crossing-mangrove:rpgq-faction-errand-bitter-coast-bitter-inner-crossing-mangrove-deliver-warning",
    }],
    payload: {
      hookId: "rpgq-faction-errand-bitter-coast-bitter-inner-crossing-mangrove",
      objectiveId: "rpgq-faction-errand-bitter-coast-bitter-inner-crossing-mangrove-deliver-warning",
      objectiveTargetId: "bitter-inner-crossing",
      routeId: "bitter-inner-crossing",
      regionId: "bitter-coast",
      faction: "coast marshwardens",
      trainsSkillId: "cartography",
    },
  });
});

test("quest topic interaction candidates ignore non-topic objectives", () => {
  const plan = planRpgQuestHooks({
    regionId: "west-gash",
    routeId: "ash-gash-pass",
    landmarkId: "redleaf_tree",
  });
  const quest = selectRpgQuestHookForExploration(plan, {
    nearCave: false,
    hasFaction: false,
    hasLandmark: false,
  });

  expect(quest?.objectiveKind).toBe("visit");
  expect(buildRpgQuestTopicInteractionCandidate({
    quest,
    worldPosition: [0, 0, 0],
    regionId: "west-gash",
    routeId: "ash-gash-pass",
  })).toBeNull();
});
