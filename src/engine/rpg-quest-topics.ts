import type { ExplorationInteractionCandidate } from "./exploration-interactions.ts";
import type { RpgQuestHookSummary } from "./rpg-quests.ts";

export interface RpgQuestTopicInteractionInput {
  quest: RpgQuestHookSummary | null;
  worldPosition: readonly [number, number, number];
  regionId: string | null;
  routeId: string | null;
}

export function buildRpgQuestTopicInteractionCandidate(
  input: RpgQuestTopicInteractionInput,
): ExplorationInteractionCandidate | null {
  const quest = input.quest;
  if (!quest || !isQuestTopicVerb(quest.objectiveKind)) {
    return null;
  }
  const id = buildRpgQuestTopicCandidateId(quest);
  return {
    id,
    subjectType: quest.objectiveKind === "report" ? "npc" : "route",
    name: quest.objectiveKind === "report" && quest.faction ? quest.faction : quest.title,
    role: "quest-topic",
    worldPosition: [...input.worldPosition],
    priority: priorityForQuestTopic(quest.objectiveKind),
    prompts: [{
      verb: quest.objectiveKind,
      label: quest.objectiveLabel,
      description: quest.objectiveJournalText,
    }],
    flavorText: quest.rumorText,
    skillAwards: [{
      skillId: quest.objectiveTrainsSkillId,
      xp: 18,
      reason: `Quest topic: ${quest.objectiveLabel}`,
      awardKey: `quest-topic:${quest.hookId}:${quest.objectiveId}`,
      onceOnly: true,
    }],
    payload: {
      hookId: quest.hookId,
      hookKind: quest.kind,
      objectiveId: quest.objectiveId,
      objectiveKind: quest.objectiveKind,
      objectiveTargetId: quest.objectiveTargetId,
      routeId: input.routeId,
      regionId: input.regionId,
      faction: quest.faction,
      trainsSkillId: quest.objectiveTrainsSkillId,
    },
  };
}

export function buildRpgQuestTopicCandidateId(quest: Pick<RpgQuestHookSummary, "hookId" | "objectiveId">): string {
  return `quest-topic:${quest.hookId}:${quest.objectiveId}`;
}

function isQuestTopicVerb(value: RpgQuestHookSummary["objectiveKind"]): value is "listen" | "interpret" | "report" {
  return value === "listen" || value === "interpret" || value === "report";
}

function priorityForQuestTopic(verb: "listen" | "interpret" | "report"): number {
  switch (verb) {
    case "listen":
      return 6;
    case "interpret":
      return 10;
    case "report":
      return 14;
  }
}
