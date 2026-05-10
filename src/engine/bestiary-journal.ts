import type { ExplorationEvent, ExplorationEventLogSnapshot } from "./exploration-events.ts";

export interface BestiaryEntry {
  id: string;
  label: string;
  count: number;
  lastRole: string;
  lastNote: string;
}

export interface BestiarySnapshot {
  totalSightings: number;
  entryCount: number;
  entries: readonly BestiaryEntry[];
  summaryLabel: string;
  lastSightingLabel: string;
  lastFieldNoteLabel: string;
  dominantFactionLabel: string;
}

export function summarizeBestiary(snapshot: ExplorationEventLogSnapshot): BestiarySnapshot {
  const entriesById = new Map<string, BestiaryEntry>();
  let lastEvent: ExplorationEvent | null = null;

  const mobEvents = snapshot.events
    .filter((event) => event.subjectType === "mob")
    .sort((left, right) => left.sequence - right.sequence);

  for (const event of mobEvents) {
    const id = readFactionId(event) ?? readMoodId(event) ?? event.subjectId;
    const label = formatBestiaryLabel(id);
    const existing = entriesById.get(id);
    const note = readFieldNote(event) ?? event.flavorText ?? "No field note recorded.";
    entriesById.set(id, {
      id,
      label,
      count: (existing?.count ?? 0) + 1,
      lastRole: event.role ?? "mob-sign",
      lastNote: note,
    });
    lastEvent = event;
  }

  const entries = [...entriesById.values()].sort((left, right) => {
    const countOrder = right.count - left.count;
    return countOrder !== 0 ? countOrder : left.label.localeCompare(right.label);
  });
  const dominant = entries[0] ?? null;
  const lastId = lastEvent ? readFactionId(lastEvent) ?? readMoodId(lastEvent) ?? lastEvent.subjectId : null;
  const lastNote = lastEvent ? readFieldNote(lastEvent) ?? lastEvent.flavorText : null;

  return {
    totalSightings: mobEvents.length,
    entryCount: entries.length,
    entries,
    summaryLabel: mobEvents.length === 0
      ? "Bestiary empty"
      : `${mobEvents.length} mob ${mobEvents.length === 1 ? "sighting" : "sightings"}`,
    lastSightingLabel: lastEvent && lastId
      ? `Last sign: ${formatBestiaryLabel(lastId)}`
      : "No mob signs yet",
    lastFieldNoteLabel: lastNote ? `Mob note: ${lastNote}` : "No mob field note yet",
    dominantFactionLabel: dominant ? `Most signs: ${dominant.label}` : "No dominant mob sign",
  };
}

function readFactionId(event: ExplorationEvent): string | null {
  const payloadFactionId = readPayloadString(event.payload, "factionId");
  if (payloadFactionId) {
    return payloadFactionId;
  }
  return looksLikeFactionId(event.subjectId) ? event.subjectId : null;
}

function readMoodId(event: ExplorationEvent): string | null {
  return readPayloadString(event.payload, "moodId");
}

function readFieldNote(event: ExplorationEvent): string | null {
  return readPayloadString(event.payload, "fieldNote");
}

function readPayloadString(payload: ExplorationEvent["payload"], key: string): string | null {
  if (!payload || typeof payload !== "object" || Array.isArray(payload)) {
    return null;
  }
  const value = (payload as Record<string, ExplorationEvent["payload"]>)[key];
  return typeof value === "string" && value.trim().length > 0 ? value : null;
}

function looksLikeFactionId(value: string): boolean {
  return value.includes("-") && !value.includes(":");
}

function formatBestiaryLabel(id: string): string {
  return id
    .split(/[-_\s:]+/g)
    .filter(Boolean)
    .map((part) => `${part[0]?.toUpperCase() ?? ""}${part.slice(1)}`)
    .join(" ");
}
