import type { SkillId } from "./skill-journal.ts";

export const EXPLORATION_EVENT_VERSION = 1;

export const EXPLORATION_EVENT_KINDS = [
  "discover",
  "inspect",
  "read",
  "use",
  "enter-zone",
  "complete-travel-goal",
  "encounter",
] as const;

export type ExplorationEventKind = typeof EXPLORATION_EVENT_KINDS[number];

export const EXPLORATION_EVENT_SUBJECT_TYPES = [
  "biome",
  "underground",
  "regional-variant",
  "landmark",
  "route",
  "zone",
  "npc",
  "mob",
  "object",
] as const;

export type ExplorationEventSubjectType = typeof EXPLORATION_EVENT_SUBJECT_TYPES[number];

export type ExplorationEventPayload =
  | null
  | boolean
  | number
  | string
  | readonly ExplorationEventPayload[]
  | { readonly [key: string]: ExplorationEventPayload };

export interface ExplorationSkillAwardMetadata {
  skillId: SkillId;
  xp: number;
  reason: string;
  awardKey: string;
  onceOnly: boolean;
}

export interface ExplorationEventKeyParts {
  kind: ExplorationEventKind;
  subjectType: ExplorationEventSubjectType;
  subjectId: string;
  role?: string | null;
  occurrenceId?: string | null;
}

export interface ExplorationEventInput extends ExplorationEventKeyParts {
  key?: string;
  name?: string;
  flavorText?: string | null;
  worldPosition?: readonly [number, number, number] | null;
  repeatable?: boolean;
  skillAwards?: readonly Partial<ExplorationSkillAwardMetadata>[];
  payload?: ExplorationEventPayload;
}

export interface ExplorationEvent extends Required<Omit<ExplorationEventInput, "occurrenceId" | "skillAwards" | "payload" | "worldPosition">> {
  version: typeof EXPLORATION_EVENT_VERSION;
  sequence: number;
  flavorText: string | null;
  worldPosition?: readonly [number, number, number];
  skillAwards: readonly ExplorationSkillAwardMetadata[];
  payload?: ExplorationEventPayload;
}

export interface ExplorationEventLogSnapshot {
  events: readonly ExplorationEvent[];
  lastEvent: ExplorationEvent | null;
  nextSequence: number;
  awardedSkillKeys: readonly string[];
}

export interface ExplorationEventLogState extends ExplorationEventLogSnapshot {
  version: typeof EXPLORATION_EVENT_VERSION;
}

export type ExplorationEventReplayEntry =
  | { accepted: true; event: ExplorationEvent }
  | { accepted: false; key: string | null; reason: "duplicate" | "invalid" | "unknown-kind" };

export interface ExplorationEventReplayResult {
  acceptedEvents: readonly ExplorationEvent[];
  duplicateKeys: readonly string[];
  rejectedEvents: readonly ExplorationEventReplayEntry[];
}

export interface ExplorationEventImportResult {
  snapshot: ExplorationEventLogSnapshot;
  importedEvents: number;
  ignoredDuplicateKeys: readonly string[];
  ignoredInvalidEvents: number;
  ignoredUnknownKinds: number;
}

const EVENT_KIND_SET = new Set<string>(EXPLORATION_EVENT_KINDS);
const SUBJECT_TYPE_SET = new Set<string>(EXPLORATION_EVENT_SUBJECT_TYPES);
const SKILL_ID_SET = new Set<string>(["cartography", "naturalist", "spelunking", "lore"] satisfies SkillId[]);

// Version 1 intentionally preserves only known event fields plus the explicit payload bag.
// Unknown event kinds are ignored on import so future clients cannot execute unknown gameplay.
export class ExplorationEventLog {
  private events: ExplorationEvent[] = [];
  private readonly seenKeys = new Set<string>();
  private readonly awardedSkillKeys = new Set<string>();
  private nextSequence = 1;

  record(input: ExplorationEventInput): ExplorationEventReplayEntry {
    const normalized = normalizeInput(input);
    if (!normalized) {
      return {
        accepted: false,
        key: typeof input.key === "string" ? input.key : null,
        reason: "invalid",
      };
    }
    if (this.seenKeys.has(normalized.key)) {
      return { accepted: false, key: normalized.key, reason: "duplicate" };
    }

    const skillAwards = this.resolveSkillAwards(normalized);
    const event: ExplorationEvent = {
      ...normalized,
      version: EXPLORATION_EVENT_VERSION,
      sequence: this.nextSequence++,
      skillAwards,
    };
    this.events.push(event);
    this.seenKeys.add(event.key);
    for (const award of skillAwards) {
      if (award.onceOnly) {
        this.awardedSkillKeys.add(award.awardKey);
      }
    }
    return { accepted: true, event: cloneEvent(event) };
  }

  replay(inputs: readonly ExplorationEventInput[]): ExplorationEventReplayResult {
    const acceptedEvents: ExplorationEvent[] = [];
    const duplicateKeys: string[] = [];
    const rejectedEvents: ExplorationEventReplayEntry[] = [];
    for (const input of inputs) {
      const result = this.record(input);
      if (result.accepted) {
        acceptedEvents.push(result.event);
      } else {
        rejectedEvents.push(result);
        if (result.reason === "duplicate" && result.key) {
          duplicateKeys.push(result.key);
        }
      }
    }
    return { acceptedEvents, duplicateKeys, rejectedEvents };
  }

  reset(): void {
    this.events = [];
    this.seenKeys.clear();
    this.awardedSkillKeys.clear();
    this.nextSequence = 1;
  }

  getSnapshot(): ExplorationEventLogSnapshot {
    return {
      events: this.events.map(cloneEvent),
      lastEvent: this.events.length > 0 ? cloneEvent(this.events[this.events.length - 1]!) : null,
      nextSequence: this.nextSequence,
      awardedSkillKeys: [...this.awardedSkillKeys].sort((left, right) => left.localeCompare(right)),
    };
  }

  exportState(): ExplorationEventLogState {
    return {
      version: EXPLORATION_EVENT_VERSION,
      ...this.getSnapshot(),
    };
  }

  importState(state: Partial<ExplorationEventLogState>): ExplorationEventImportResult {
    this.reset();

    const imported: ExplorationEvent[] = [];
    const seenImportKeys = new Set<string>();
    const ignoredDuplicateKeys: string[] = [];
    let ignoredInvalidEvents = 0;
    let ignoredUnknownKinds = 0;

    for (const raw of Array.isArray(state.events) ? state.events : []) {
      const event = normalizeImportedEvent(raw);
      if (!event) {
        if (isUnknownKindRecord(raw)) {
          ignoredUnknownKinds += 1;
        } else {
          ignoredInvalidEvents += 1;
        }
        continue;
      }
      if (seenImportKeys.has(event.key)) {
        ignoredDuplicateKeys.push(event.key);
        continue;
      }
      seenImportKeys.add(event.key);
      imported.push(event);
    }

    imported.sort((left, right) => left.sequence - right.sequence);
    this.events = imported.map(cloneEvent);
    for (const event of this.events) {
      this.seenKeys.add(event.key);
      for (const award of event.skillAwards) {
        if (award.onceOnly) {
          this.awardedSkillKeys.add(award.awardKey);
        }
      }
    }
    for (const key of readStringArray(state.awardedSkillKeys)) {
      this.awardedSkillKeys.add(key);
    }

    const maxSequence = Math.max(0, ...this.events.map((event) => event.sequence));
    this.nextSequence = Math.max(readPositiveInteger(state.nextSequence) ?? 1, maxSequence + 1);

    return {
      snapshot: this.getSnapshot(),
      importedEvents: this.events.length,
      ignoredDuplicateKeys,
      ignoredInvalidEvents,
      ignoredUnknownKinds,
    };
  }

  private resolveSkillAwards(input: NormalizedExplorationEventInput): ExplorationSkillAwardMetadata[] {
    const awards = normalizeSkillAwards(input.skillAwards, input);
    const acceptedAwards: ExplorationSkillAwardMetadata[] = [];
    for (const award of awards) {
      if (award.onceOnly && this.awardedSkillKeys.has(award.awardKey)) {
        continue;
      }
      acceptedAwards.push(award);
    }
    return acceptedAwards;
  }
}

export function buildExplorationEventKey(parts: ExplorationEventKeyParts): string {
  const role = parts.role && parts.role.trim().length > 0 ? parts.role.trim() : "default";
  const segments = [
    parts.kind,
    parts.subjectType,
    encodeStableKeyPart(parts.subjectId),
    encodeStableKeyPart(role),
  ];
  if (parts.occurrenceId && parts.occurrenceId.trim().length > 0) {
    segments.push(encodeStableKeyPart(parts.occurrenceId.trim()));
  }
  return segments.join(":");
}

export function buildFirstReadUseSkillAwards(input: ExplorationEventKeyParts): readonly ExplorationSkillAwardMetadata[] {
  if (input.kind !== "read" && input.kind !== "use") {
    return [];
  }
  const reason = input.kind === "read" ? "First read" : "First use";
  const skillId = input.kind === "use" && input.subjectType === "route" ? "cartography" : "lore";
  const xp = input.kind === "read" ? 25 : 20;
  return [{
    skillId,
    xp,
    reason,
    awardKey: `${input.kind}:${input.subjectType}:${input.subjectId}`,
    onceOnly: true,
  }];
}

export function isKnownExplorationEventKind(value: unknown): value is ExplorationEventKind {
  return typeof value === "string" && EVENT_KIND_SET.has(value);
}

type NormalizedExplorationEventInput = Required<Omit<ExplorationEventInput, "occurrenceId" | "skillAwards" | "payload" | "worldPosition">> & {
  flavorText: string | null;
  worldPosition?: readonly [number, number, number];
  skillAwards?: readonly Partial<ExplorationSkillAwardMetadata>[];
  payload?: ExplorationEventPayload;
};

function normalizeInput(input: ExplorationEventInput): NormalizedExplorationEventInput | null {
  if (!isKnownExplorationEventKind(input.kind) || !isKnownSubjectType(input.subjectType)) {
    return null;
  }
  const subjectId = readNonEmptyString(input.subjectId);
  if (!subjectId) {
    return null;
  }
  const role = readNonEmptyString(input.role) ?? "default";
  const key = readNonEmptyString(input.key) ?? buildExplorationEventKey({ ...input, subjectId, role });
  const name = readNonEmptyString(input.name) ?? subjectId;
  const worldPosition = readWorldPosition(input.worldPosition);
  const payload = sanitizePayload(input.payload);
  return {
    key,
    kind: input.kind,
    subjectId,
    subjectType: input.subjectType,
    role,
    name,
    flavorText: typeof input.flavorText === "string" ? input.flavorText : null,
    repeatable: input.repeatable === true,
    ...(worldPosition ? { worldPosition } : {}),
    ...(input.skillAwards ? { skillAwards: input.skillAwards } : {}),
    ...(payload !== undefined ? { payload } : {}),
  };
}

function normalizeImportedEvent(value: unknown): ExplorationEvent | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return null;
  }
  const record = value as Record<string, unknown>;
  if (!isKnownExplorationEventKind(record.kind) || !isKnownSubjectType(record.subjectType)) {
    return null;
  }
  const key = readNonEmptyString(record.key);
  const subjectId = readNonEmptyString(record.subjectId);
  const sequence = readPositiveInteger(record.sequence);
  if (!key || !subjectId || !sequence) {
    return null;
  }
  const role = readNonEmptyString(record.role) ?? "default";
  const name = readNonEmptyString(record.name) ?? subjectId;
  const worldPosition = readWorldPosition(record.worldPosition);
  const payload = sanitizePayload(record.payload);
  const imported: ExplorationEvent = {
    version: EXPLORATION_EVENT_VERSION,
    key,
    kind: record.kind,
    subjectId,
    subjectType: record.subjectType,
    role,
    name,
    flavorText: typeof record.flavorText === "string" ? record.flavorText : null,
    sequence,
    repeatable: record.repeatable === true,
    skillAwards: normalizeSkillAwards(record.skillAwards, {
      kind: record.kind,
      subjectType: record.subjectType,
      subjectId,
      role,
    }, false),
    ...(worldPosition ? { worldPosition } : {}),
    ...(payload !== undefined ? { payload } : {}),
  };
  return imported;
}

function normalizeSkillAwards(
  value: unknown,
  event: ExplorationEventKeyParts,
  inferDefaultAwards = true,
): ExplorationSkillAwardMetadata[] {
  const rawAwards = Array.isArray(value)
    ? value
    : inferDefaultAwards ? buildFirstReadUseSkillAwards(event) : [];
  const awards: ExplorationSkillAwardMetadata[] = [];
  for (const raw of rawAwards) {
    if (!raw || typeof raw !== "object" || Array.isArray(raw)) {
      continue;
    }
    const record = raw as Record<string, unknown>;
    if (typeof record.skillId !== "string" || !SKILL_ID_SET.has(record.skillId)) {
      continue;
    }
    const xp = typeof record.xp === "number" && Number.isFinite(record.xp)
      ? Math.max(0, Math.floor(record.xp))
      : null;
    const awardKey = readNonEmptyString(record.awardKey);
    if (xp === null || !awardKey) {
      continue;
    }
    awards.push({
      skillId: record.skillId as SkillId,
      xp,
      reason: readNonEmptyString(record.reason) ?? "Exploration event",
      awardKey,
      onceOnly: record.onceOnly !== false,
    });
  }
  return awards;
}

function cloneEvent(event: ExplorationEvent): ExplorationEvent {
  return {
    ...event,
    ...(event.worldPosition ? { worldPosition: [...event.worldPosition] as const } : {}),
    skillAwards: event.skillAwards.map((award) => ({ ...award })),
    ...(event.payload !== undefined ? { payload: clonePayload(event.payload) } : {}),
  };
}

function isKnownSubjectType(value: unknown): value is ExplorationEventSubjectType {
  return typeof value === "string" && SUBJECT_TYPE_SET.has(value);
}

function isUnknownKindRecord(value: unknown): boolean {
  return Boolean(
    value
      && typeof value === "object"
      && !Array.isArray(value)
      && typeof (value as Record<string, unknown>).kind === "string"
      && !EVENT_KIND_SET.has((value as Record<string, unknown>).kind as string),
  );
}

function readNonEmptyString(value: unknown): string | null {
  return typeof value === "string" && value.trim().length > 0 ? value.trim() : null;
}

function readStringArray(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((entry): entry is string => typeof entry === "string") : [];
}

function readPositiveInteger(value: unknown): number | null {
  return typeof value === "number" && Number.isInteger(value) && value > 0 ? value : null;
}

function readWorldPosition(value: unknown): readonly [number, number, number] | null {
  if (!Array.isArray(value) || value.length !== 3) {
    return null;
  }
  const [x, y, z] = value;
  return Number.isFinite(x) && Number.isFinite(y) && Number.isFinite(z)
    ? [x, y, z]
    : null;
}

function sanitizePayload(value: unknown): ExplorationEventPayload | undefined {
  if (value === undefined) {
    return undefined;
  }
  if (value === null || typeof value === "boolean" || typeof value === "string") {
    return value;
  }
  if (typeof value === "number") {
    return Number.isFinite(value) ? value : null;
  }
  if (Array.isArray(value)) {
    return value.map((entry) => sanitizePayload(entry) ?? null);
  }
  if (typeof value === "object") {
    const result: Record<string, ExplorationEventPayload> = {};
    for (const [key, entry] of Object.entries(value)) {
      const cleanKey = readNonEmptyString(key);
      if (!cleanKey) {
        continue;
      }
      result[cleanKey] = sanitizePayload(entry) ?? null;
    }
    return result;
  }
  return null;
}

function clonePayload(value: ExplorationEventPayload): ExplorationEventPayload {
  if (!value || typeof value !== "object") {
    return value;
  }
  if (Array.isArray(value)) {
    return value.map(clonePayload);
  }
  return Object.fromEntries(
    Object.entries(value).map(([key, entry]) => [key, clonePayload(entry)]),
  );
}

function encodeStableKeyPart(value: string): string {
  return encodeURIComponent(value);
}
