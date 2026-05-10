import {
  describeDiscovery,
  type DiscoveryCategory,
} from "./discovery-catalog.ts";

export interface ExplorationObservation {
  biomeId: string;
  undergroundBiomeId: string | null;
  regionalVariantId: string | null;
  landmarkIds: readonly string[];
  currentLandmarkId: string | null;
}

export interface DiscoveryEvent {
  category: DiscoveryCategory;
  id: string;
  name: string;
  flavorText: string | null;
  identifier: string;
  categoryLabel: string;
  label: string;
  sequence: number;
}

export interface ExplorationJournalSnapshot {
  currentBiomeId: string | null;
  currentUndergroundBiomeId: string | null;
  currentRegionalVariantId: string | null;
  currentLandmarkId: string | null;
  discoveredBiomeIds: string[];
  discoveredUndergroundBiomeIds: string[];
  discoveredRegionalVariantIds: string[];
  discoveredLandmarkIds: string[];
  recentDiscoveries: DiscoveryEvent[];
  lastDiscovery: DiscoveryEvent | null;
}

export interface ExplorationJournalState extends ExplorationJournalSnapshot {
  pendingSkillDiscoveries: DiscoveryEvent[];
  nextDiscoverySequence: number;
}

const MAX_RECENT_DISCOVERIES = 8;
const VALID_DISCOVERY_CATEGORIES = new Set<DiscoveryCategory>([
  "biome",
  "underground",
  "regional-variant",
  "landmark",
]);

export class ExplorationJournal {
  private readonly discoveredBiomeIds = new Set<string>();
  private readonly discoveredUndergroundBiomeIds = new Set<string>();
  private readonly discoveredRegionalVariantIds = new Set<string>();
  private readonly discoveredLandmarkIds = new Set<string>();
  private recentDiscoveries: DiscoveryEvent[] = [];
  private pendingSkillDiscoveries: DiscoveryEvent[] = [];
  private currentBiomeId: string | null = null;
  private currentUndergroundBiomeId: string | null = null;
  private currentRegionalVariantId: string | null = null;
  private currentLandmarkId: string | null = null;
  private nextDiscoverySequence = 1;

  observe(observation: ExplorationObservation): ExplorationJournalSnapshot {
    this.currentBiomeId = observation.biomeId;
    this.currentUndergroundBiomeId = observation.undergroundBiomeId;
    this.currentRegionalVariantId = observation.regionalVariantId;
    this.currentLandmarkId = observation.currentLandmarkId;

    this.recordDiscovery("biome", observation.biomeId, this.discoveredBiomeIds);
    if (observation.undergroundBiomeId) {
      this.recordDiscovery("underground", observation.undergroundBiomeId, this.discoveredUndergroundBiomeIds);
    }
    if (observation.regionalVariantId) {
      this.recordDiscovery(
        "regional-variant",
        observation.regionalVariantId,
        this.discoveredRegionalVariantIds,
      );
    }
    for (const landmarkId of new Set(observation.landmarkIds)) {
      this.recordDiscovery("landmark", landmarkId, this.discoveredLandmarkIds);
    }

    return this.getSnapshot();
  }

  reset(): void {
    this.discoveredBiomeIds.clear();
    this.discoveredUndergroundBiomeIds.clear();
    this.discoveredRegionalVariantIds.clear();
    this.discoveredLandmarkIds.clear();
    this.recentDiscoveries = [];
    this.pendingSkillDiscoveries = [];
    this.currentBiomeId = null;
    this.currentUndergroundBiomeId = null;
    this.currentRegionalVariantId = null;
    this.currentLandmarkId = null;
    this.nextDiscoverySequence = 1;
  }

  getSnapshot(): ExplorationJournalSnapshot {
    return {
      currentBiomeId: this.currentBiomeId,
      currentUndergroundBiomeId: this.currentUndergroundBiomeId,
      currentRegionalVariantId: this.currentRegionalVariantId,
      currentLandmarkId: this.currentLandmarkId,
      discoveredBiomeIds: sortIds(this.discoveredBiomeIds),
      discoveredUndergroundBiomeIds: sortIds(this.discoveredUndergroundBiomeIds),
      discoveredRegionalVariantIds: sortIds(this.discoveredRegionalVariantIds),
      discoveredLandmarkIds: sortIds(this.discoveredLandmarkIds),
      recentDiscoveries: this.recentDiscoveries.map((event) => ({ ...event })),
      lastDiscovery: this.recentDiscoveries[0] ? { ...this.recentDiscoveries[0] } : null,
    };
  }

  drainPendingSkillDiscoveries(): DiscoveryEvent[] {
    const discoveries = this.pendingSkillDiscoveries.map((event) => ({ ...event }));
    this.pendingSkillDiscoveries = [];
    return discoveries;
  }

  exportState(): ExplorationJournalState {
    return {
      ...this.getSnapshot(),
      pendingSkillDiscoveries: this.pendingSkillDiscoveries.map((event) => ({ ...event })),
      nextDiscoverySequence: this.nextDiscoverySequence,
    };
  }

  importState(state: Partial<ExplorationJournalState>): ExplorationJournalSnapshot {
    this.discoveredBiomeIds.clear();
    this.discoveredUndergroundBiomeIds.clear();
    this.discoveredRegionalVariantIds.clear();
    this.discoveredLandmarkIds.clear();
    for (const id of readStringArray(state.discoveredBiomeIds)) {
      this.discoveredBiomeIds.add(id);
    }
    for (const id of readStringArray(state.discoveredUndergroundBiomeIds)) {
      this.discoveredUndergroundBiomeIds.add(id);
    }
    for (const id of readStringArray(state.discoveredRegionalVariantIds)) {
      this.discoveredRegionalVariantIds.add(id);
    }
    for (const id of readStringArray(state.discoveredLandmarkIds)) {
      this.discoveredLandmarkIds.add(id);
    }
    this.currentBiomeId = readNullableString(state.currentBiomeId);
    this.currentUndergroundBiomeId = readNullableString(state.currentUndergroundBiomeId);
    this.currentRegionalVariantId = readNullableString(state.currentRegionalVariantId);
    this.currentLandmarkId = readNullableString(state.currentLandmarkId);
    this.recentDiscoveries = readDiscoveryEvents(state.recentDiscoveries)
      .sort((left, right) => right.sequence - left.sequence)
      .slice(0, MAX_RECENT_DISCOVERIES);
    this.pendingSkillDiscoveries = readDiscoveryEvents(state.pendingSkillDiscoveries);
    const maxSequence = Math.max(
      0,
      ...this.recentDiscoveries.map((event) => event.sequence),
      ...this.pendingSkillDiscoveries.map((event) => event.sequence),
    );
    this.nextDiscoverySequence = Math.max(
      readPositiveInteger(state.nextDiscoverySequence) ?? 1,
      maxSequence + 1,
    );
    return this.getSnapshot();
  }

  private recordDiscovery(
    category: DiscoveryCategory,
    id: string,
    registry: Set<string>,
  ): void {
    if (registry.has(id)) {
      return;
    }
    registry.add(id);
    const presentation = describeDiscovery(category, id);
    const event = {
      category,
      id,
      name: presentation.name,
      flavorText: presentation.flavorText,
      identifier: id,
      categoryLabel: presentation.categoryLabel,
      label: presentation.fullLabel,
      sequence: this.nextDiscoverySequence++,
    } satisfies DiscoveryEvent;
    this.recentDiscoveries.unshift(event);
    this.pendingSkillDiscoveries.push(event);
    if (this.recentDiscoveries.length > MAX_RECENT_DISCOVERIES) {
      this.recentDiscoveries.length = MAX_RECENT_DISCOVERIES;
    }
  }
}

function sortIds(values: ReadonlySet<string>): string[] {
  return [...values].sort((left, right) => left.localeCompare(right));
}

function readStringArray(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((entry): entry is string => typeof entry === "string") : [];
}

function readNullableString(value: unknown): string | null {
  return typeof value === "string" ? value : null;
}

function readPositiveInteger(value: unknown): number | null {
  return typeof value === "number" && Number.isInteger(value) && value > 0 ? value : null;
}

function readDiscoveryEvents(value: unknown): DiscoveryEvent[] {
  if (!Array.isArray(value)) {
    return [];
  }
  const events: DiscoveryEvent[] = [];
  for (const item of value) {
    if (!item || typeof item !== "object" || Array.isArray(item)) {
      continue;
    }
    const record = item as Record<string, unknown>;
    const category = record.category;
    const id = record.id;
    const sequence = record.sequence;
    if (
      typeof category !== "string"
      || !VALID_DISCOVERY_CATEGORIES.has(category as DiscoveryCategory)
      || typeof id !== "string"
      || typeof sequence !== "number"
      || !Number.isInteger(sequence)
      || sequence <= 0
    ) {
      continue;
    }
    const presentation = describeDiscovery(category as DiscoveryCategory, id);
    events.push({
      category: category as DiscoveryCategory,
      id,
      name: typeof record.name === "string" ? record.name : presentation.name,
      flavorText: typeof record.flavorText === "string" ? record.flavorText : presentation.flavorText,
      identifier: typeof record.identifier === "string" ? record.identifier : id,
      categoryLabel: typeof record.categoryLabel === "string" ? record.categoryLabel : presentation.categoryLabel,
      label: typeof record.label === "string" ? record.label : presentation.fullLabel,
      sequence,
    });
  }
  return events.sort((left, right) => left.sequence - right.sequence);
}
