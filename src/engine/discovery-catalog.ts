import type {
  BiomeId,
  LandmarkId,
  RegionalVariantId,
  UndergroundBiomeId,
} from "./procedural-generator.ts";

export type DiscoveryCategory = "biome" | "underground" | "regional-variant" | "landmark";

export interface DiscoveryPresentation {
  category: DiscoveryCategory;
  categoryLabel: string;
  id: string;
  name: string;
  flavorText: string | null;
  inlineLabel: string;
  fullLabel: string;
}

const DISCOVERY_CATEGORY_LABELS: Record<DiscoveryCategory, string> = {
  biome: "Biome",
  underground: "Underground",
  "regional-variant": "Regional Variant",
  landmark: "Landmark",
};

const BIOME_NAMES = {
  verdant: "Verdant Reach",
  savanna: "Sunspoke Savanna",
  steppe: "Ridgewind Steppe",
  dunes: "Amberglass Dunes",
  badlands: "Ashbarrow Badlands",
  highland: "Crownfall Highlands",
  moor: "Gloam Moor",
  tundra: "Bluefrost Tundra",
  marsh: "Siltmire Marsh",
  firefly: "Lantern Fen",
  saltflat: "Mirror Salt Flats",
  fern: "Cenote Fernwild",
  fungal: "Mooncap Wilds",
  ember: "Ember Wastes",
  bloom: "Prism Bloom",
  shardlands: "Shattershard Expanse",
} satisfies Record<BiomeId, string>;

const UNDERGROUND_NAMES = {
  rooted: "Rootweb Caverns",
  sedimentary: "Layerstone Deep",
  sandy: "Sandveil Hollows",
  granitic: "Granitebone Vaults",
  froststone: "Hoarfrost Veins",
  basaltic: "Basalt Furnace",
  peaty: "Peatmuck Hollows",
  saline: "Saltroot Strata",
  mycelial: "Mycelium Weald",
  crystalline: "Crystalheart Grotto",
} satisfies Record<UndergroundBiomeId, string>;

const REGIONAL_VARIANT_NAMES = {
  verdant_karst: "Karst Bloom",
  savanna_flowersea: "Flowersea Verge",
  steppe_monolith: "Monolith March",
  dunes_glass: "Glasswind Basin",
  badlands_crater: "Crater Maze",
  ash_wastes: "Ash Wastes",
  highland_redleaf: "Redleaf Crown",
  moor_shadowglass: "Shadowglass Bog",
  tundra_blue_ice: "Blue-Ice Shelf",
  marsh_blackwater: "Blackwater Channel",
  firefly_lantern: "Lanternwake",
  saltflat_mirror: "Mirror Pan",
  fern_cenote: "Cenote Steps",
  fungal_moonlit: "Moonlit Mycelia",
  ember_caldera: "Caldera Heart",
  bloom_prism: "Prism Garden",
} satisfies Record<RegionalVariantId, string>;

const LANDMARK_NAMES = {
  oak: "Oakspire Tree",
  canopy_tree: "Canopy Giant",
  birch: "Silver Birch",
  redleaf_tree: "Redleaf Tree",
  willow: "River Willow",
  blossom_tree: "Blossom Tree",
  fruit_tree: "Fruit Tree",
  giant_flower: "Giant Bloom",
  redwood: "Skyroot Redwood",
  dead_tree: "Dead Finger Tree",
  thorn_tree: "Thornback Tree",
  berry_bush: "Berry Bush",
  giant_fern: "Giant Fern",
  lantern_tree: "Lantern Tree",
  salt_spire: "Salt Spire",
  boulder: "Weathered Boulder",
  standing_stone: "Standing Stone",
  shrub: "Windworn Shrub",
  flower_patch: "Flower Patch",
  palm: "Palm Tree",
  acacia: "Acacia Tree",
  cactus: "Cactus",
  dead_snag: "Dead Snag",
  hoodoo: "Hoodoo Pillar",
  fir: "Fir Tree",
  tall_fir: "Tall Fir",
  ice_spire: "Ice Spire",
  frost_shrub: "Frost Shrub",
  cypress: "Bog Cypress",
  mangrove: "Mangrove",
  reed_cluster: "Reed Cluster",
  basalt_spire: "Basalt Spire",
  crystal_cluster: "Crystal Cluster",
  glowcap: "Glowcap",
  mega_glowcap: "Great Glowcap",
  root_stump: "Root Stump",
  stone_tor: "Stone Tor",
  ancestor_pillar: "Ancestor Pillar",
  ash_marker: "Ash Marker",
  glass_cairn: "Glass Cairn",
  silt_shell: "Silt Strider Shell",
  velothi_shrine: "Velothi Wayshrine",
  kwama_mound: "Kwama Egg Mound",
  pilgrim_cairn: "Pilgrim Cairn",
} satisfies Record<LandmarkId, string>;

const LANDMARK_FLAVOR_TEXT: Partial<Record<LandmarkId, string>> = {
  ancestor_pillar: "Weathered stonework suggests old roads beneath the grass.",
  ash_marker: "Charred stones point toward a harsher volcanic country.",
  glass_cairn: "Pale shards catch the fog like frozen lightning.",
  silt_shell: "A hollow carapace rests half-buried in windblown dust.",
  velothi_shrine: "A small shrine watches the road with worn amber light.",
  kwama_mound: "Packed clay rises around a clutch of amber-shelled hollows.",
  pilgrim_cairn: "Stacked stones mark a footpath older than the ash.",
};

export function describeDiscovery(category: DiscoveryCategory, id: string): DiscoveryPresentation {
  const categoryLabel = DISCOVERY_CATEGORY_LABELS[category];
  const name = resolveDiscoveryName(category, id);
  const flavorText = resolveDiscoveryFlavorText(category, id);
  const inlineLabel = `${name} [${id}]`;
  return {
    category,
    categoryLabel,
    id,
    name,
    flavorText,
    inlineLabel,
    fullLabel: `${categoryLabel}: ${inlineLabel}`,
  };
}

export function formatDiscoveryInline(category: DiscoveryCategory, id: string | null, fallback = "None"): string {
  if (!id) {
    return fallback;
  }
  return describeDiscovery(category, id).inlineLabel;
}

export function formatDiscoveryLabel(category: DiscoveryCategory, id: string): string {
  return describeDiscovery(category, id).fullLabel;
}

function resolveDiscoveryName(category: DiscoveryCategory, id: string): string {
  switch (category) {
    case "biome":
      return BIOME_NAMES[id as BiomeId] ?? titleCaseIdentifier(id);
    case "underground":
      return UNDERGROUND_NAMES[id as UndergroundBiomeId] ?? titleCaseIdentifier(id);
    case "regional-variant":
      return REGIONAL_VARIANT_NAMES[id as RegionalVariantId] ?? titleCaseIdentifier(id);
    case "landmark":
      return LANDMARK_NAMES[id as LandmarkId] ?? titleCaseIdentifier(id);
  }
}

function resolveDiscoveryFlavorText(category: DiscoveryCategory, id: string): string | null {
  if (category !== "landmark") {
    return null;
  }
  return LANDMARK_FLAVOR_TEXT[id as LandmarkId] ?? null;
}

function titleCaseIdentifier(value: string): string {
  return value
    .split(/[_-]+/g)
    .filter((part) => part.length > 0)
    .map((part) => `${part[0]!.toUpperCase()}${part.slice(1)}`)
    .join(" ");
}
