import type { ExplorationEvent, ExplorationEventLogSnapshot } from "./exploration-events.ts";

export const FIELD_KIT_CATEGORY_IDS = [
  "supplies",
  "reagents",
  "relics",
  "salvage",
] as const;

export type FieldKitCategoryId = typeof FIELD_KIT_CATEGORY_IDS[number];

export interface FieldKitEntry {
  lootId: string;
  name: string;
  categoryId: FieldKitCategoryId;
  categoryLabel: string;
  count: number;
}

export interface FieldKitSnapshot {
  totalFinds: number;
  categoryCounts: Record<FieldKitCategoryId, number>;
  entries: readonly FieldKitEntry[];
  summaryLabel: string;
  lastFindLabel: string;
  lastFieldNoteLabel: string;
  dominantCategoryLabel: string;
}

interface LootDefinition {
  name: string;
  categoryId: FieldKitCategoryId;
}

const CATEGORY_LABELS = {
  supplies: "supplies",
  reagents: "reagents",
  relics: "relics",
  salvage: "salvage",
} as const satisfies Record<FieldKitCategoryId, string>;

const LOOT_DEFINITIONS = new Map<string, LootDefinition>([
  ["travel-pack-cache", { name: "Travel pack cache", categoryId: "supplies" }],
  ["lost-cave-pack", { name: "Lost cave pack", categoryId: "supplies" }],
  ["ashlander-cache", { name: "Trail clan cache", categoryId: "supplies" }],
  ["trail-forage", { name: "Trail forage", categoryId: "supplies" }],
  ["berry-bush-forage", { name: "Berry bush forage", categoryId: "supplies" }],
  ["cactus-pulp", { name: "Cactus pulp", categoryId: "supplies" }],
  ["wetland-reagents", { name: "Wetland reagents", categoryId: "reagents" }],
  ["glowcap-reagents", { name: "Glowcap reagents", categoryId: "reagents" }],
  ["glass-alchemy-trace", { name: "Glass alchemy trace", categoryId: "reagents" }],
  ["flower-nectar", { name: "Flower nectar", categoryId: "reagents" }],
  ["mangrove-cuttings", { name: "Mangrove cuttings", categoryId: "reagents" }],
  ["crystal-reed-shards", { name: "Crystal reed shards", categoryId: "reagents" }],
  ["shrine-offerings", { name: "Shrine offerings", categoryId: "relics" }],
]);

export function summarizeFieldKit(snapshot: ExplorationEventLogSnapshot): FieldKitSnapshot {
  const categoryCounts = createEmptyCategoryCounts();
  const entriesByLootId = new Map<string, FieldKitEntry>();
  let lastFind: FieldKitEntry | null = null;
  let lastFieldNote: string | null = null;

  const lootEvents = snapshot.events
    .filter(isLootCacheEvent)
    .sort((left, right) => left.sequence - right.sequence);
  for (const event of lootEvents) {
    const lootId = readLootId(event);
    const definition = LOOT_DEFINITIONS.get(lootId) ?? {
      name: formatUnknownLootName(lootId),
      categoryId: "salvage" as const,
    };
    const categoryLabel = CATEGORY_LABELS[definition.categoryId];
    const existing = entriesByLootId.get(lootId);
    const entry = existing
      ? { ...existing, count: existing.count + 1 }
      : {
        lootId,
        name: definition.name,
        categoryId: definition.categoryId,
        categoryLabel,
        count: 1,
      };
    entriesByLootId.set(lootId, entry);
    categoryCounts[definition.categoryId] += 1;
    lastFind = entry;
    lastFieldNote = readFieldNote(event);
  }

  const entries = [...entriesByLootId.values()].sort((left, right) => {
    const categoryOrder = FIELD_KIT_CATEGORY_IDS.indexOf(left.categoryId) - FIELD_KIT_CATEGORY_IDS.indexOf(right.categoryId);
    return categoryOrder !== 0 ? categoryOrder : left.name.localeCompare(right.name);
  });
  const totalFinds = lootEvents.length;
  const dominantCategoryId = resolveDominantCategoryId(categoryCounts);

  return {
    totalFinds,
    categoryCounts,
    entries,
    summaryLabel: totalFinds === 0 ? "Field kit empty" : `${totalFinds} field ${totalFinds === 1 ? "find" : "finds"}`,
    lastFindLabel: lastFind ? `Last find: ${lastFind.name}` : "No field finds yet",
    lastFieldNoteLabel: lastFieldNote ? `Field note: ${lastFieldNote}` : "No field note yet",
    dominantCategoryLabel: dominantCategoryId ? `Mostly ${CATEGORY_LABELS[dominantCategoryId]}` : "No kit pattern yet",
  };
}

function isLootCacheEvent(event: ExplorationEvent): boolean {
  return event.subjectType === "object" && event.role === "loot-cache";
}

function readLootId(event: ExplorationEvent): string {
  const payloadLootId = readPayloadLootId(event.payload);
  if (payloadLootId) {
    return payloadLootId;
  }
  const subjectLootId = event.subjectId.split(":")[0]?.trim();
  return subjectLootId && subjectLootId.length > 0 ? subjectLootId : "unknown-cache";
}

function readPayloadLootId(payload: ExplorationEvent["payload"]): string | null {
  if (!payload || typeof payload !== "object" || Array.isArray(payload)) {
    return null;
  }
  const lootId = (payload as Record<string, ExplorationEvent["payload"]>).lootId;
  return typeof lootId === "string" && lootId.trim().length > 0 ? lootId : null;
}

function readFieldNote(event: ExplorationEvent): string | null {
  const payload = event.payload;
  if (payload && typeof payload === "object" && !Array.isArray(payload)) {
    const fieldNote = (payload as Record<string, ExplorationEvent["payload"]>).fieldNote;
    if (typeof fieldNote === "string" && fieldNote.trim().length > 0) {
      return fieldNote;
    }
  }
  return event.flavorText && event.flavorText.trim().length > 0 ? event.flavorText : null;
}

function createEmptyCategoryCounts(): Record<FieldKitCategoryId, number> {
  return {
    supplies: 0,
    reagents: 0,
    relics: 0,
    salvage: 0,
  };
}

function resolveDominantCategoryId(counts: Record<FieldKitCategoryId, number>): FieldKitCategoryId | null {
  let dominant: FieldKitCategoryId | null = null;
  let dominantCount = 0;
  for (const categoryId of FIELD_KIT_CATEGORY_IDS) {
    if (counts[categoryId] > dominantCount) {
      dominant = categoryId;
      dominantCount = counts[categoryId];
    }
  }
  return dominant;
}

function formatUnknownLootName(lootId: string): string {
  const words = lootId
    .split(/[-_\s]+/g)
    .map((word) => word.trim())
    .filter(Boolean);
  if (words.length === 0) {
    return "Unknown cache";
  }
  return words
    .map((word, index) => index === 0 ? capitalize(word) : word)
    .join(" ");
}

function capitalize(value: string): string {
  return value.length === 0 ? value : `${value[0]!.toUpperCase()}${value.slice(1)}`;
}
