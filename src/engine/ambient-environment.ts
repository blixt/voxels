import { clamp } from "./math.ts";
import { metersToWorldUnits } from "./scale.ts";
import {
  DEFAULT_SKY_WEATHER_ENVIRONMENT,
  type Rgba,
  type SkyWeatherEnvironment,
} from "./render-constants.ts";
import {
  DEFAULT_RENDER_ENVIRONMENT,
  type RenderEnvironment,
} from "./water-visuals.ts";
import {
  hexColorToMaterial,
  type BiomeId,
  type LandmarkId,
  type ProceduralBiomeProbe,
  type RegionalVariantId,
} from "./procedural-generator.ts";
import { WORLD_REGION_AUTHORITY_THRESHOLD } from "./worldgen-region.ts";

export type AmbientProfileId =
  | "open-air"
  | "green-canopy"
  | "dry-haze"
  | "ashfall"
  | "salt-marsh"
  | "silt-mist"
  | "wetlands"
  | "fungal-lantern"
  | "glass-coast"
  | "cold-glass"
  | "subterranean";

export interface AmbientWorldProfile extends RenderEnvironment, SkyWeatherEnvironment {
  id: AmbientProfileId;
  label: string;
  intensity: number;
}

export interface AmbientWorldProbe {
  regionAmbientProfileId?: AmbientProfileId;
  regionStrength?: number;
  biomeId: BiomeId;
  undergroundBiomeId: string | null;
  regionalVariantId: RegionalVariantId | null;
  regionalVariantStrength: number;
  specialStrength: number;
  surfaceY: number;
  landmarkId?: LandmarkId | null;
  surfaceMaterial?: number;
  pilgrimRouteInfluence?: number;
  fields: ProceduralBiomeProbe["fields"];
}

interface AmbientProfileDefinition extends Omit<SkyWeatherEnvironment, "rainfallIntensity"> {
  id: AmbientProfileId;
  label: string;
  clearColorRgba: Rgba;
  fogColorRgba: Rgba;
  fogStartDistance: number;
  fogEndDistance: number;
  rainfallIntensity?: number;
}

const SURFACE_FOG_MIN_END_DISTANCE = metersToWorldUnits(224);
const SURFACE_FOG_MAX_END_DISTANCE = DEFAULT_RENDER_ENVIRONMENT.fogEndDistance;
const UNDERGROUND_FOG_MIN_END_DISTANCE = metersToWorldUnits(64);
const UNDERGROUND_FOG_MAX_END_DISTANCE = metersToWorldUnits(192);
const PILGRIM_ROUTE_SURFACE_MATERIALS = new Set<number>(
  ["#655", "#887", "#433", "#544", "#BBA"].map((code) => hexColorToMaterial(code)),
);
const PILGRIM_ROUTE_LANDMARKS = new Set<LandmarkId>([
  "ancestor_pillar",
  "ash_marker",
  "bone_chimes",
  "old_road_causeway",
  "pilgrim_lantern",
  "rib_arch",
  "rib_remains",
  "velothi_shrine",
  "velothi_ziggurat",
  "ash_obelisk",
]);

const PROFILE_DEFINITIONS: Record<AmbientProfileId, AmbientProfileDefinition> = {
  "open-air": {
    id: "open-air",
    label: "Open Air",
    clearColorRgba: [207, 223, 238, 255],
    fogColorRgba: [202, 219, 233, 255],
    fogStartDistance: metersToWorldUnits(96),
    fogEndDistance: metersToWorldUnits(480),
    skyTopColorRgba: [78, 124, 172, 255],
    skyHorizonColorRgba: [210, 224, 236, 255],
    skyCloudColorRgba: [226, 231, 230, 255],
    skyCloudCoverage: 0.16,
    skyCloudBand: 0.70,
    ashfallIntensity: 0,
    fungalGlowIntensity: 0,
  },
  "green-canopy": {
    id: "green-canopy",
    label: "Canopy Light",
    clearColorRgba: [90, 205, 115, 255],
    fogColorRgba: [48, 160, 74, 255],
    fogStartDistance: metersToWorldUnits(70),
    fogEndDistance: metersToWorldUnits(372),
    skyTopColorRgba: [57, 105, 113, 255],
    skyHorizonColorRgba: [150, 230, 130, 255],
    skyCloudColorRgba: [136, 205, 145, 255],
    skyCloudCoverage: 0.30,
    skyCloudBand: 0.66,
    ashfallIntensity: 0,
    fungalGlowIntensity: 0.06,
  },
  "dry-haze": {
    id: "dry-haze",
    label: "Mineral Haze",
    clearColorRgba: [225, 200, 160, 255],
    fogColorRgba: [203, 188, 154, 255],
    fogStartDistance: metersToWorldUnits(84),
    fogEndDistance: metersToWorldUnits(420),
    skyTopColorRgba: [112, 124, 137, 255],
    skyHorizonColorRgba: [222, 200, 162, 255],
    skyCloudColorRgba: [174, 166, 145, 255],
    skyCloudCoverage: 0.28,
    skyCloudBand: 0.54,
    ashfallIntensity: 0.08,
    fungalGlowIntensity: 0,
  },
  ashfall: {
    id: "ashfall",
    label: "Ashlands",
    clearColorRgba: [137, 116, 101, 255],
    fogColorRgba: [126, 101, 85, 255],
    fogStartDistance: metersToWorldUnits(38),
    fogEndDistance: metersToWorldUnits(304),
    skyTopColorRgba: [48, 48, 52, 255],
    skyHorizonColorRgba: [185, 115, 78, 255],
    skyCloudColorRgba: [75, 67, 62, 255],
    skyCloudCoverage: 0.96,
    skyCloudBand: 0.40,
    ashfallIntensity: 0.98,
    fungalGlowIntensity: 0,
  },
  "salt-marsh": {
    id: "salt-marsh",
    label: "Salt Marsh",
    clearColorRgba: [196, 207, 197, 255],
    fogColorRgba: [190, 198, 179, 255],
    fogStartDistance: metersToWorldUnits(48),
    fogEndDistance: metersToWorldUnits(316),
    skyTopColorRgba: [86, 107, 112, 255],
    skyHorizonColorRgba: [218, 218, 193, 255],
    skyCloudColorRgba: [176, 185, 166, 255],
    skyCloudCoverage: 0.66,
    skyCloudBand: 0.48,
    ashfallIntensity: 0.08,
    fungalGlowIntensity: 0.05,
  },
  "silt-mist": {
    id: "silt-mist",
    label: "Silt Mist",
    clearColorRgba: [130, 155, 190, 255],
    fogColorRgba: [50, 98, 145, 255],
    fogStartDistance: metersToWorldUnits(50),
    fogEndDistance: metersToWorldUnits(336),
    skyTopColorRgba: [65, 91, 100, 255],
    skyHorizonColorRgba: [96, 150, 198, 255],
    skyCloudColorRgba: [64, 112, 154, 255],
    skyCloudCoverage: 0.62,
    skyCloudBand: 0.45,
    ashfallIntensity: 0.16,
    fungalGlowIntensity: 0.06,
  },
  wetlands: {
    id: "wetlands",
    label: "Blackwater Wetlands",
    clearColorRgba: [146, 177, 172, 255],
    fogColorRgba: [92, 128, 123, 255],
    fogStartDistance: metersToWorldUnits(36),
    fogEndDistance: metersToWorldUnits(276),
    skyTopColorRgba: [34, 62, 70, 255],
    skyHorizonColorRgba: [124, 175, 157, 255],
    skyCloudColorRgba: [58, 95, 91, 255],
    skyCloudCoverage: 0.74,
    skyCloudBand: 0.36,
    ashfallIntensity: 0.04,
    fungalGlowIntensity: 0.18,
  },
  "fungal-lantern": {
    id: "fungal-lantern",
    label: "Fungal Glow",
    clearColorRgba: [169, 190, 211, 255],
    fogColorRgba: [102, 170, 184, 255],
    fogStartDistance: metersToWorldUnits(48),
    fogEndDistance: metersToWorldUnits(324),
    skyTopColorRgba: [40, 66, 94, 255],
    skyHorizonColorRgba: [82, 184, 185, 255],
    skyCloudColorRgba: [40, 119, 132, 255],
    skyCloudCoverage: 0.64,
    skyCloudBand: 0.35,
    ashfallIntensity: 0.03,
    fungalGlowIntensity: 0.86,
  },
  "glass-coast": {
    id: "glass-coast",
    label: "Glass Coast",
    clearColorRgba: [191, 218, 226, 255],
    fogColorRgba: [155, 207, 219, 255],
    fogStartDistance: metersToWorldUnits(82),
    fogEndDistance: metersToWorldUnits(448),
    skyTopColorRgba: [39, 101, 145, 255],
    skyHorizonColorRgba: [211, 236, 230, 255],
    skyCloudColorRgba: [151, 198, 212, 255],
    skyCloudCoverage: 0.22,
    skyCloudBand: 0.68,
    ashfallIntensity: 0,
    fungalGlowIntensity: 0.02,
  },
  "cold-glass": {
    id: "cold-glass",
    label: "Cold Glass",
    clearColorRgba: [190, 230, 255, 255],
    fogColorRgba: [155, 200, 238, 255],
    fogStartDistance: metersToWorldUnits(70),
    fogEndDistance: metersToWorldUnits(408),
    skyTopColorRgba: [55, 100, 147, 255],
    skyHorizonColorRgba: [199, 225, 239, 255],
    skyCloudColorRgba: [161, 195, 218, 255],
    skyCloudCoverage: 0.38,
    skyCloudBand: 0.60,
    ashfallIntensity: 0,
    fungalGlowIntensity: 0.03,
  },
  subterranean: {
    id: "subterranean",
    label: "Underdeep",
    clearColorRgba: [30, 41, 45, 255],
    fogColorRgba: [42, 56, 55, 255],
    fogStartDistance: metersToWorldUnits(16),
    fogEndDistance: metersToWorldUnits(116),
    skyTopColorRgba: [16, 25, 29, 255],
    skyHorizonColorRgba: [39, 51, 50, 255],
    skyCloudColorRgba: [26, 36, 37, 255],
    skyCloudCoverage: 0.34,
    skyCloudBand: 0.32,
    ashfallIntensity: 0.10,
    fungalGlowIntensity: 0.34,
  },
};

export function resolveAmbientWorldProfile(
  probe: AmbientWorldProbe,
  options: { observedUndergroundBiomeId?: string | null } = {},
): AmbientWorldProfile {
  const undergroundBiomeId = options.observedUndergroundBiomeId ?? null;
  const profileId = undergroundBiomeId
    ? resolveUndergroundProfileId(undergroundBiomeId)
    : resolveSurfaceProfileId(probe);
  const definition = PROFILE_DEFINITIONS[profileId];
  const intensity = resolveProfileIntensity(probe, Boolean(undergroundBiomeId));
  const fogEndMinimum = undergroundBiomeId ? UNDERGROUND_FOG_MIN_END_DISTANCE : SURFACE_FOG_MIN_END_DISTANCE;
  const fogEndMaximum = undergroundBiomeId ? UNDERGROUND_FOG_MAX_END_DISTANCE : SURFACE_FOG_MAX_END_DISTANCE;
  const fogEndDistance = clamp(
    blend(DEFAULT_RENDER_ENVIRONMENT.fogEndDistance, definition.fogEndDistance, intensity),
    fogEndMinimum,
    fogEndMaximum,
  );
  const fogStartDistance = clamp(
    blend(DEFAULT_RENDER_ENVIRONMENT.fogStartDistance, definition.fogStartDistance, intensity),
    metersToWorldUnits(12),
    Math.max(metersToWorldUnits(16), fogEndDistance - metersToWorldUnits(8)),
  );

  return {
    id: definition.id,
    label: definition.label,
    intensity,
    clearColorRgba: mixRgba(DEFAULT_RENDER_ENVIRONMENT.clearColorRgba, definition.clearColorRgba, intensity),
    fogColorRgba: mixRgba(DEFAULT_RENDER_ENVIRONMENT.fogColorRgba, definition.fogColorRgba, intensity),
    fogStartDistance,
    fogEndDistance,
    skyTopColorRgba: mixRgba(DEFAULT_SKY_WEATHER_ENVIRONMENT.skyTopColorRgba, definition.skyTopColorRgba, intensity),
    skyHorizonColorRgba: mixRgba(DEFAULT_SKY_WEATHER_ENVIRONMENT.skyHorizonColorRgba, definition.skyHorizonColorRgba, intensity),
    skyCloudColorRgba: mixRgba(DEFAULT_SKY_WEATHER_ENVIRONMENT.skyCloudColorRgba, definition.skyCloudColorRgba, intensity),
    skyCloudCoverage: blend(DEFAULT_SKY_WEATHER_ENVIRONMENT.skyCloudCoverage, definition.skyCloudCoverage, intensity),
    skyCloudBand: blend(DEFAULT_SKY_WEATHER_ENVIRONMENT.skyCloudBand, definition.skyCloudBand, intensity),
    ashfallIntensity: blend(DEFAULT_SKY_WEATHER_ENVIRONMENT.ashfallIntensity, definition.ashfallIntensity, intensity),
    rainfallIntensity: blend(
      DEFAULT_SKY_WEATHER_ENVIRONMENT.rainfallIntensity,
      definition.rainfallIntensity ?? DEFAULT_SKY_WEATHER_ENVIRONMENT.rainfallIntensity,
      intensity,
    ),
    fungalGlowIntensity: blend(DEFAULT_SKY_WEATHER_ENVIRONMENT.fungalGlowIntensity, definition.fungalGlowIntensity, intensity),
  };
}

export function buildAmbientRenderEnvironment(profile: AmbientWorldProfile): RenderEnvironment & SkyWeatherEnvironment {
  return {
    clearColorRgba: profile.clearColorRgba,
    fogColorRgba: profile.fogColorRgba,
    fogStartDistance: profile.fogStartDistance,
    fogEndDistance: profile.fogEndDistance,
    skyTopColorRgba: profile.skyTopColorRgba,
    skyHorizonColorRgba: profile.skyHorizonColorRgba,
    skyCloudColorRgba: profile.skyCloudColorRgba,
    skyCloudCoverage: profile.skyCloudCoverage,
    skyCloudBand: profile.skyCloudBand,
    ashfallIntensity: profile.ashfallIntensity,
    rainfallIntensity: profile.rainfallIntensity,
    fungalGlowIntensity: profile.fungalGlowIntensity,
  };
}

function resolveSurfaceProfileId(probe: AmbientWorldProbe): AmbientProfileId {
  const routeHaze = isPilgrimRouteHazeProbe(probe);
  if (routeHaze && probe.biomeId !== "marsh" && probe.biomeId !== "saltflat" && probe.biomeId !== "fungal") {
    return "ashfall";
  }
  if ((probe.regionStrength ?? 0) > WORLD_REGION_AUTHORITY_THRESHOLD && probe.regionAmbientProfileId) {
    return probe.regionAmbientProfileId;
  }
  switch (probe.regionalVariantId) {
    case "badlands_crater":
    case "ember_caldera":
      return "ashfall";
    case "marsh_blackwater":
      return "wetlands";
    case "saltflat_mirror":
      return "salt-marsh";
    case "moor_shadowglass":
      return "silt-mist";
    case "firefly_lantern":
    case "fungal_moonlit":
    case "bloom_prism":
      return "fungal-lantern";
    case "dunes_glass":
      return "glass-coast";
    case "tundra_blue_ice":
      return "cold-glass";
    case "verdant_karst":
    case "highland_redleaf":
    case "fern_cenote":
    case "savanna_flowersea":
      return "green-canopy";
    case "steppe_monolith":
    case null:
      break;
  }

  if (routeHaze) {
    return probe.biomeId === "saltflat" ? "salt-marsh" : "silt-mist";
  }

  switch (probe.biomeId) {
    case "verdant":
    case "fern":
    case "bloom":
      return "green-canopy";
    case "marsh":
      return "wetlands";
    case "moor":
      return "silt-mist";
    case "saltflat":
      return "salt-marsh";
    case "firefly":
    case "fungal":
      return "fungal-lantern";
    case "ember":
    case "badlands":
      return "ashfall";
    case "tundra":
    case "highland":
      return "cold-glass";
    case "shardlands":
      return "glass-coast";
    case "savanna":
    case "steppe":
    case "dunes":
      return "dry-haze";
  }
}

function isPilgrimRouteHazeProbe(probe: AmbientWorldProbe): boolean {
  if (typeof probe.surfaceMaterial === "number" && PILGRIM_ROUTE_SURFACE_MATERIALS.has(probe.surfaceMaterial)) {
    return true;
  }
  if (probe.landmarkId && PILGRIM_ROUTE_LANDMARKS.has(probe.landmarkId)) {
    return true;
  }
  if ((probe.pilgrimRouteInfluence ?? 0) > 0.18) {
    return true;
  }
  return probe.fields.desolation > 0.56 && probe.fields.strata > 0.58 && probe.fields.scatter > 0.56;
}

function resolveUndergroundProfileId(undergroundBiomeId: string): AmbientProfileId {
  switch (undergroundBiomeId) {
    case "mycelial":
    case "crystalline":
      return "fungal-lantern";
    case "froststone":
      return "cold-glass";
    case "basaltic":
      return "ashfall";
    default:
      return "subterranean";
  }
}

function resolveProfileIntensity(probe: AmbientWorldProbe, underground: boolean): number {
  const fieldPressure = (
    Math.max(0, probe.fields.moisture - 0.58) * 0.12
    + Math.max(0, probe.fields.magic - 0.56) * 0.16
    + Math.max(0, probe.fields.volcanism - 0.60) * 0.14
    + Math.max(0, probe.fields.oceanness - 0.50) * 0.08
  );
  const routePressure = !underground && isPilgrimRouteHazeProbe(probe) ? 0.18 : 0;
  const regionPressure = (probe.regionStrength ?? 0) * 0.18;
  const profilePressure = regionPressure + probe.specialStrength * 0.22 + probe.regionalVariantStrength * 0.20 + routePressure;
  return clamp((underground ? 0.84 : 0.58) + fieldPressure + profilePressure, 0.48, 1);
}

function mixRgba(left: Rgba, right: Rgba, amount: number): [number, number, number, number] {
  const t = clamp(amount, 0, 1);
  return [
    Math.round(blend(left[0], right[0], t)),
    Math.round(blend(left[1], right[1], t)),
    Math.round(blend(left[2], right[2], t)),
    Math.round(blend(left[3], right[3], t)),
  ];
}

function blend(left: number, right: number, amount: number): number {
  return left + (right - left) * amount;
}
