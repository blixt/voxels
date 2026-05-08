import type {
  BiomeId,
  LandmarkId,
  RegionalVariantId,
  UndergroundBiomeId,
} from "./procedural-generator.ts";

export type DiscoveryCategory = "biome" | "underground" | "regional-variant" | "landmark";
export type DiscoveryRole = "region" | "deep-place" | "strange-border" | "old-road" | "shrine" | "landmark";

export interface DiscoveryPresentation {
  category: DiscoveryCategory;
  categoryLabel: string;
  id: string;
  name: string;
  role: DiscoveryRole;
  roleLabel: string;
  flavorText: string | null;
  progressionHint: string;
  inlineLabel: string;
  fullLabel: string;
}

const DISCOVERY_CATEGORY_LABELS: Record<DiscoveryCategory, string> = {
  biome: "Biome",
  underground: "Underground",
  "regional-variant": "Regional Variant",
  landmark: "Landmark",
};

const DISCOVERY_ROLE_LABELS: Record<DiscoveryRole, string> = {
  region: "Region",
  "deep-place": "Deep Place",
  "strange-border": "Strange Border",
  "old-road": "Old Road",
  shrine: "Shrine",
  landmark: "Landmark",
};

const DISCOVERY_PROGRESSION_HINTS: Record<DiscoveryCategory, string> = {
  biome: "Travel and region discovery train Cartography.",
  underground: "Underground discovery trains Spelunking.",
  "regional-variant": "Strange region discovery trains Lore.",
  landmark: "Landmark discovery trains Naturalist.",
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
  velothi_ziggurat: "Velothi Ziggurat",
  rib_arch: "Rib Arch",
  ash_obelisk: "Ash Obelisk",
  old_road_causeway: "Old Road Causeway",
  pilgrim_lantern: "Pilgrim Lantern",
  bone_chimes: "Bone Wind Chimes",
  crystal_reeds: "Crystal Reeds",
  fungal_bridge: "Fungal Bridge",
  rib_remains: "Rib Remains",
} satisfies Record<LandmarkId, string> & Record<string, string>;

const LANDMARK_FLAVOR_TEXT: Partial<Record<LandmarkId, string>> & Partial<Record<string, string>> = {
  ancestor_pillar: "Weathered stonework suggests old roads beneath the grass.",
  ash_marker: "Charred stones point toward a harsher volcanic country.",
  glass_cairn: "Pale shards catch the fog like frozen lightning.",
  silt_shell: "A hollow carapace rests half-buried in windblown dust.",
  velothi_shrine: "A small shrine watches the road with worn amber light.",
  kwama_mound: "Packed clay rises around a clutch of amber-shelled hollows.",
  pilgrim_cairn: "Stacked stones mark a footpath older than the ash.",
  velothi_ziggurat: "Tiered stone rises where the old road turns toward temple country.",
  rib_arch: "Great ribs frame the trail like the remains of a forgotten silt beast.",
  ash_obelisk: "A black obelisk leans into the ash wind and refuses to fall.",
  old_road_causeway: "Raised stones cross the low ground, worn smooth by pilgrim feet.",
  pilgrim_lantern: "A hooded lantern keeps its watch over a road almost lost.",
  bone_chimes: "A wind rack of black posts and pale bones clatters over the pilgrim stones.",
  crystal_reeds: "Glass-bright reeds grow from brackish pools and hum in the silt wind.",
  fungal_bridge: "A shelf of old fungus spans the wet ground like a living footbridge.",
  rib_remains: "Low bones break the blackwater, more warning than shelter.",
};

const OLD_ROAD_LANDMARK_IDS = new Set<string>([
  "ancestor_pillar",
  "ash_marker",
  "glass_cairn",
  "silt_shell",
  "pilgrim_cairn",
  "rib_arch",
  "ash_obelisk",
  "old_road_causeway",
  "pilgrim_lantern",
  "bone_chimes",
]);

const SHRINE_LANDMARK_IDS = new Set<string>([
  "velothi_shrine",
  "velothi_ziggurat",
]);

const CATEGORY_ROLES: Record<Exclude<DiscoveryCategory, "landmark">, DiscoveryRole> = {
  biome: "region",
  underground: "deep-place",
  "regional-variant": "strange-border",
};

export function describeDiscovery(category: DiscoveryCategory, id: string): DiscoveryPresentation {
  const categoryLabel = DISCOVERY_CATEGORY_LABELS[category];
  const name = resolveDiscoveryName(category, id);
  const role = resolveDiscoveryRole(category, id);
  const flavorText = resolveDiscoveryFlavorText(category, id);
  const inlineLabel = `${name} [${id}]`;
  return {
    category,
    categoryLabel,
    id,
    name,
    role,
    roleLabel: DISCOVERY_ROLE_LABELS[role],
    flavorText,
    progressionHint: DISCOVERY_PROGRESSION_HINTS[category],
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

export function formatDiscoveryName(category: DiscoveryCategory, id: string | null, fallback = "Unknown"): string {
  return id ? describeDiscovery(category, id).name : fallback;
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

function resolveDiscoveryRole(category: DiscoveryCategory, id: string): DiscoveryRole {
  if (category !== "landmark") {
    return CATEGORY_ROLES[category];
  }
  if (SHRINE_LANDMARK_IDS.has(id as LandmarkId)) {
    return "shrine";
  }
  if (OLD_ROAD_LANDMARK_IDS.has(id as LandmarkId)) {
    return "old-road";
  }
  return "landmark";
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
