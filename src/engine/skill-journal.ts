import type { DiscoveryEvent } from "./exploration-journal.ts";
import { describeDiscovery, type DiscoveryRole } from "./discovery-catalog.ts";

export type SkillId = "cartography" | "naturalist" | "spelunking" | "lore";

export interface SkillSnapshot {
  id: SkillId;
  name: string;
  totalXp: number;
  level: number;
  currentLevelXp: number;
  nextLevelXp: number;
  progressRatio: number;
}

export interface SkillJournalSnapshot {
  skills: SkillSnapshot[];
  focusSkill: SkillSnapshot;
  totalLevel: number;
  travelMeters: number;
  lastProcessedDiscoverySequence: number;
}

export interface SkillJournalState {
  xpBySkill: Partial<Record<SkillId, number>>;
  travelMetersBySkill: Partial<Record<SkillId, number>>;
  travelRemainderMetersBySkill: Partial<Record<SkillId, number>>;
  lastProcessedDiscoverySequence: number;
}

export type TravelSkillContext = "surface" | "underground";

interface SkillDefinition {
  id: SkillId;
  name: string;
}

const SKILL_DEFINITIONS: readonly SkillDefinition[] = [
  { id: "cartography", name: "Cartography" },
  { id: "naturalist", name: "Naturalist" },
  { id: "spelunking", name: "Spelunking" },
  { id: "lore", name: "Lore" },
];

const DISCOVERY_XP: Record<DiscoveryEvent["category"], { skillId: SkillId; xp: number }> = {
  biome: { skillId: "cartography", xp: 55 },
  underground: { skillId: "spelunking", xp: 90 },
  "regional-variant": { skillId: "lore", xp: 120 },
  landmark: { skillId: "naturalist", xp: 35 },
};
const LANDMARK_ROLE_XP: Partial<Record<DiscoveryRole, readonly { skillId: SkillId; xp: number }[]>> = {
  "old-road": [
    { skillId: "cartography", xp: 35 },
    { skillId: "lore", xp: 20 },
  ],
  shrine: [
    { skillId: "lore", xp: 55 },
  ],
};
const TRAVEL_METERS_PER_XP = 24;
const UNDERGROUND_TRAVEL_XP_MULTIPLIER = 1.35;

export class SkillJournal {
  private readonly xpBySkill = new Map<SkillId, number>(
    SKILL_DEFINITIONS.map((definition) => [definition.id, 0]),
  );
  private readonly travelMetersBySkill = new Map<SkillId, number>(
    SKILL_DEFINITIONS.map((definition) => [definition.id, 0]),
  );
  private readonly travelRemainderMetersBySkill = new Map<SkillId, number>(
    SKILL_DEFINITIONS.map((definition) => [definition.id, 0]),
  );
  private lastProcessedDiscoverySequence = 0;

  observeDiscoveries(discoveries: readonly DiscoveryEvent[]): SkillJournalSnapshot {
    const newDiscoveries = discoveries
      .filter((discovery) => discovery.sequence > this.lastProcessedDiscoverySequence)
      .sort((left, right) => left.sequence - right.sequence);
    for (const discovery of newDiscoveries) {
      for (const award of awardsForDiscovery(discovery)) {
        this.addXp(award.skillId, award.xp);
      }
      this.lastProcessedDiscoverySequence = Math.max(this.lastProcessedDiscoverySequence, discovery.sequence);
    }
    return this.getSnapshot();
  }

  observeTravel(distanceMeters: number, context: TravelSkillContext): SkillJournalSnapshot {
    if (!Number.isFinite(distanceMeters) || distanceMeters <= 0) {
      return this.getSnapshot();
    }
    const skillId: SkillId = context === "underground" ? "spelunking" : "cartography";
    const scaledDistanceMeters = distanceMeters * (context === "underground" ? UNDERGROUND_TRAVEL_XP_MULTIPLIER : 1);
    this.travelMetersBySkill.set(skillId, (this.travelMetersBySkill.get(skillId) ?? 0) + distanceMeters);
    const nextRemainder = (this.travelRemainderMetersBySkill.get(skillId) ?? 0) + scaledDistanceMeters;
    const gainedXp = Math.floor(nextRemainder / TRAVEL_METERS_PER_XP);
    this.travelRemainderMetersBySkill.set(skillId, nextRemainder - gainedXp * TRAVEL_METERS_PER_XP);
    if (gainedXp > 0) {
      this.addXp(skillId, gainedXp);
    }
    return this.getSnapshot();
  }

  reset(): void {
    for (const definition of SKILL_DEFINITIONS) {
      this.xpBySkill.set(definition.id, 0);
      this.travelMetersBySkill.set(definition.id, 0);
      this.travelRemainderMetersBySkill.set(definition.id, 0);
    }
    this.lastProcessedDiscoverySequence = 0;
  }

  exportState(): SkillJournalState {
    return {
      xpBySkill: Object.fromEntries(
        SKILL_DEFINITIONS.map((definition) => [definition.id, this.xpBySkill.get(definition.id) ?? 0]),
      ) as Record<SkillId, number>,
      travelMetersBySkill: Object.fromEntries(
        SKILL_DEFINITIONS.map((definition) => [definition.id, this.travelMetersBySkill.get(definition.id) ?? 0]),
      ) as Record<SkillId, number>,
      travelRemainderMetersBySkill: Object.fromEntries(
        SKILL_DEFINITIONS.map((definition) => [definition.id, this.travelRemainderMetersBySkill.get(definition.id) ?? 0]),
      ) as Record<SkillId, number>,
      lastProcessedDiscoverySequence: this.lastProcessedDiscoverySequence,
    };
  }

  importState(state: Partial<SkillJournalState>): SkillJournalSnapshot {
    for (const definition of SKILL_DEFINITIONS) {
      const totalXp = state.xpBySkill?.[definition.id];
      this.xpBySkill.set(definition.id, typeof totalXp === "number" && Number.isFinite(totalXp)
        ? Math.max(0, Math.floor(totalXp))
        : 0);
      const travelMeters = state.travelMetersBySkill?.[definition.id];
      this.travelMetersBySkill.set(definition.id, typeof travelMeters === "number" && Number.isFinite(travelMeters)
        ? Math.max(0, travelMeters)
        : 0);
      const travelRemainderMeters = state.travelRemainderMetersBySkill?.[definition.id];
      this.travelRemainderMetersBySkill.set(definition.id, typeof travelRemainderMeters === "number" && Number.isFinite(travelRemainderMeters)
        ? Math.max(0, travelRemainderMeters)
        : 0);
    }
    const sequence = state.lastProcessedDiscoverySequence;
    this.lastProcessedDiscoverySequence = typeof sequence === "number" && Number.isInteger(sequence) && sequence >= 0
      ? sequence
      : 0;
    return this.getSnapshot();
  }

  getSnapshot(): SkillJournalSnapshot {
    const skills = SKILL_DEFINITIONS.map((definition) => buildSkillSnapshot(
      definition,
      this.xpBySkill.get(definition.id) ?? 0,
    ));
    const focusSkill = [...skills].sort((left, right) => {
      if (right.level !== left.level) {
        return right.level - left.level;
      }
      if (right.totalXp !== left.totalXp) {
        return right.totalXp - left.totalXp;
      }
      return left.name.localeCompare(right.name);
    })[0]!;
    return {
      skills,
      focusSkill,
      totalLevel: skills.reduce((total, skill) => total + skill.level, 0),
      travelMeters: [...this.travelMetersBySkill.values()].reduce((total, value) => total + value, 0),
      lastProcessedDiscoverySequence: this.lastProcessedDiscoverySequence,
    };
  }

  private addXp(skillId: SkillId, xp: number): void {
    this.xpBySkill.set(skillId, (this.xpBySkill.get(skillId) ?? 0) + xp);
  }
}

function awardsForDiscovery(discovery: DiscoveryEvent): readonly { skillId: SkillId; xp: number }[] {
  const baseAward = DISCOVERY_XP[discovery.category];
  if (discovery.category !== "landmark") {
    return [baseAward];
  }
  const role = describeDiscovery(discovery.category, discovery.id).role;
  const roleAwards = LANDMARK_ROLE_XP[role] ?? [];
  return [baseAward, ...roleAwards];
}

function buildSkillSnapshot(definition: SkillDefinition, totalXp: number): SkillSnapshot {
  const level = resolveLevel(totalXp);
  const currentLevelXp = xpForLevel(level);
  const nextLevelXp = xpForLevel(level + 1);
  return {
    id: definition.id,
    name: definition.name,
    totalXp,
    level,
    currentLevelXp,
    nextLevelXp,
    progressRatio: (totalXp - currentLevelXp) / Math.max(1, nextLevelXp - currentLevelXp),
  };
}

function resolveLevel(totalXp: number): number {
  let level = 1;
  while (totalXp >= xpForLevel(level + 1)) {
    level += 1;
  }
  return level;
}

function xpForLevel(level: number): number {
  if (level <= 1) {
    return 0;
  }
  return Math.round(80 * (level - 1) ** 1.55);
}
