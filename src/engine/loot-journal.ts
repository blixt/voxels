import type { ExplorationEvent, ExplorationEventLogSnapshot } from "./exploration-events.ts";

export type LootJournalCandidateMatch = "subject" | "loot-id" | "category" | "none";

export interface LootJournalCacheEntry {
  subjectId: string;
  lootId: string;
  categoryId: string | null;
  collected: boolean;
  revisited: boolean;
  collectCount: number;
  revisitCount: number;
  eventCount: number;
  firstSequence: number;
  lastSequence: number;
  lastNote: string | null;
}

export interface LootJournalSnapshot {
  totalCollectedCaches: number;
  totalRevisitedCaches: number;
  totalCollectEvents: number;
  totalRevisitEvents: number;
  entries: readonly LootJournalCacheEntry[];
}

export interface LootJournalCandidate {
  subjectId: string;
  lootId?: string | null;
  categoryId?: string | null;
}

export interface LootJournalCandidateState {
  subjectId: string;
  lootId: string | null;
  categoryId: string | null;
  collected: boolean;
  revisited: boolean;
  collectCount: number;
  revisitCount: number;
  eventCount: number;
  lastNote: string | null;
  matchedSubjectId: string | null;
  match: LootJournalCandidateMatch;
}

export function summarizeLootJournal(snapshot: ExplorationEventLogSnapshot): LootJournalSnapshot {
  const entriesBySubjectId = new Map<string, LootJournalCacheEntry>();
  const lootEvents = snapshot.events
    .filter(isLootCacheEvent)
    .sort((left, right) => left.sequence - right.sequence);

  for (const event of lootEvents) {
    const existing = entriesBySubjectId.get(event.subjectId);
    const eventCount = (existing?.eventCount ?? 0) + 1;
    entriesBySubjectId.set(event.subjectId, {
      subjectId: event.subjectId,
      lootId: existing?.lootId ?? readLootId(event),
      categoryId: existing?.categoryId ?? readCategoryId(event),
      collected: true,
      revisited: eventCount > 1,
      collectCount: 1,
      revisitCount: Math.max(0, eventCount - 1),
      eventCount,
      firstSequence: existing?.firstSequence ?? event.sequence,
      lastSequence: event.sequence,
      lastNote: readNote(event) ?? existing?.lastNote ?? null,
    });
  }

  const entries = [...entriesBySubjectId.values()].sort((left, right) => {
    const sequenceOrder = right.lastSequence - left.lastSequence;
    return sequenceOrder !== 0 ? sequenceOrder : left.subjectId.localeCompare(right.subjectId);
  });

  return {
    totalCollectedCaches: entries.length,
    totalRevisitedCaches: entries.filter((entry) => entry.revisited).length,
    totalCollectEvents: entries.reduce((total, entry) => total + entry.collectCount, 0),
    totalRevisitEvents: entries.reduce((total, entry) => total + entry.revisitCount, 0),
    entries,
  };
}

export function getLootJournalCandidateState(
  source: ExplorationEventLogSnapshot | LootJournalSnapshot,
  candidate: LootJournalCandidate,
): LootJournalCandidateState {
  const journal = isLootJournalSnapshot(source) ? source : summarizeLootJournal(source);
  const subjectId = normalizeString(candidate.subjectId) ?? "";
  const lootId = normalizeString(candidate.lootId) ?? readLootIdFromSubject(subjectId);
  const categoryId = normalizeString(candidate.categoryId);
  const subjectMatch = journal.entries.find((entry) => entry.subjectId === subjectId);
  const lootMatch = subjectMatch ?? (lootId ? journal.entries.find((entry) => entry.lootId === lootId) : null);
  const match = lootMatch ?? (categoryId ? journal.entries.find((entry) => entry.categoryId === categoryId) : null);

  if (match) {
    return {
      subjectId,
      lootId,
      categoryId,
      collected: match.collected,
      revisited: match.revisited,
      collectCount: match.collectCount,
      revisitCount: match.revisitCount,
      eventCount: match.eventCount,
      lastNote: match.lastNote,
      matchedSubjectId: match.subjectId,
      match: subjectMatch ? "subject" : lootMatch ? "loot-id" : "category",
    };
  }

  return {
    subjectId,
    lootId,
    categoryId,
    collected: false,
    revisited: false,
    collectCount: 0,
    revisitCount: 0,
    eventCount: 0,
    lastNote: null,
    matchedSubjectId: null,
    match: "none",
  };
}

function isLootCacheEvent(event: ExplorationEvent): boolean {
  return event.subjectType === "object" && event.role === "loot-cache";
}

function readLootId(event: ExplorationEvent): string {
  return readPayloadString(event.payload, "lootId") ?? readLootIdFromSubject(event.subjectId) ?? "unknown-cache";
}

function readCategoryId(event: ExplorationEvent): string | null {
  return readPayloadString(event.payload, "categoryId") ?? readPayloadString(event.payload, "forageSiteRole");
}

function readNote(event: ExplorationEvent): string | null {
  return readPayloadString(event.payload, "fieldNote") ?? (event.flavorText && event.flavorText.trim().length > 0
    ? event.flavorText
    : null);
}

function readPayloadString(payload: ExplorationEvent["payload"], key: string): string | null {
  if (!payload || typeof payload !== "object" || Array.isArray(payload)) {
    return null;
  }
  return normalizeString((payload as Record<string, ExplorationEvent["payload"]>)[key]);
}

function readLootIdFromSubject(subjectId: string): string | null {
  const subjectLootId = subjectId.split(":")[0];
  return normalizeString(subjectLootId);
}

function normalizeString(value: ExplorationEvent["payload"] | string | null | undefined): string | null {
  return typeof value === "string" && value.trim().length > 0 ? value.trim() : null;
}

function isLootJournalSnapshot(source: ExplorationEventLogSnapshot | LootJournalSnapshot): source is LootJournalSnapshot {
  return "totalCollectedCaches" in source;
}
