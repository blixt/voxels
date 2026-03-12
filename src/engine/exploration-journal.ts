export type DiscoveryCategory = "biome" | "underground" | "regional-variant" | "landmark";

export interface ExplorationObservation {
  biomeId: string;
  undergroundBiomeId: string;
  regionalVariantId: string | null;
  landmarkIds: readonly string[];
  currentLandmarkId: string | null;
}

export interface DiscoveryEvent {
  category: DiscoveryCategory;
  id: string;
  label: string;
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

const MAX_RECENT_DISCOVERIES = 8;

export class ExplorationJournal {
  private readonly discoveredBiomeIds = new Set<string>();
  private readonly discoveredUndergroundBiomeIds = new Set<string>();
  private readonly discoveredRegionalVariantIds = new Set<string>();
  private readonly discoveredLandmarkIds = new Set<string>();
  private recentDiscoveries: DiscoveryEvent[] = [];
  private currentBiomeId: string | null = null;
  private currentUndergroundBiomeId: string | null = null;
  private currentRegionalVariantId: string | null = null;
  private currentLandmarkId: string | null = null;

  observe(observation: ExplorationObservation): ExplorationJournalSnapshot {
    this.currentBiomeId = observation.biomeId;
    this.currentUndergroundBiomeId = observation.undergroundBiomeId;
    this.currentRegionalVariantId = observation.regionalVariantId;
    this.currentLandmarkId = observation.currentLandmarkId;

    this.recordDiscovery("biome", observation.biomeId, this.discoveredBiomeIds);
    this.recordDiscovery("underground", observation.undergroundBiomeId, this.discoveredUndergroundBiomeIds);
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
    this.currentBiomeId = null;
    this.currentUndergroundBiomeId = null;
    this.currentRegionalVariantId = null;
    this.currentLandmarkId = null;
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

  private recordDiscovery(
    category: DiscoveryCategory,
    id: string,
    registry: Set<string>,
  ): void {
    if (registry.has(id)) {
      return;
    }
    registry.add(id);
    const event = {
      category,
      id,
      label: `${categoryLabel(category)}: ${id}`,
    } satisfies DiscoveryEvent;
    this.recentDiscoveries.unshift(event);
    if (this.recentDiscoveries.length > MAX_RECENT_DISCOVERIES) {
      this.recentDiscoveries.length = MAX_RECENT_DISCOVERIES;
    }
  }
}

function categoryLabel(category: DiscoveryCategory): string {
  switch (category) {
    case "biome":
      return "Biome";
    case "underground":
      return "Underground";
    case "regional-variant":
      return "Variant";
    case "landmark":
      return "Landmark";
  }
}

function sortIds(values: ReadonlySet<string>): string[] {
  return [...values].sort((left, right) => left.localeCompare(right));
}
