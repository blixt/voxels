import { clamp } from "./math.ts";
import { metersToWorldUnits } from "./scale.ts";
import {
  DEFAULT_RENDER_ENVIRONMENT,
  type RenderEnvironment,
} from "./water-visuals.ts";
import type {
  BiomeId,
  ProceduralBiomeProbe,
  RegionalVariantId,
} from "./procedural-generator.ts";

export type AmbientProfileId =
  | "open-air"
  | "green-canopy"
  | "dry-haze"
  | "ashfall"
  | "silt-mist"
  | "fungal-lantern"
  | "cold-glass"
  | "subterranean";

type Rgba = readonly [number, number, number, number];

export interface AmbientWorldProfile extends RenderEnvironment {
  id: AmbientProfileId;
  label: string;
  intensity: number;
}

export interface AmbientWorldProbe {
  biomeId: BiomeId;
  undergroundBiomeId: string | null;
  regionalVariantId: RegionalVariantId | null;
  regionalVariantStrength: number;
  specialStrength: number;
  surfaceY: number;
  fields: ProceduralBiomeProbe["fields"];
}

interface AmbientProfileDefinition {
  id: AmbientProfileId;
  label: string;
  clearColorRgba: Rgba;
  fogColorRgba: Rgba;
  fogStartDistance: number;
  fogEndDistance: number;
}

const SURFACE_FOG_MIN_END_DISTANCE = metersToWorldUnits(224);
const SURFACE_FOG_MAX_END_DISTANCE = DEFAULT_RENDER_ENVIRONMENT.fogEndDistance;
const UNDERGROUND_FOG_MIN_END_DISTANCE = metersToWorldUnits(64);
const UNDERGROUND_FOG_MAX_END_DISTANCE = metersToWorldUnits(192);

const PROFILE_DEFINITIONS: Record<AmbientProfileId, AmbientProfileDefinition> = {
  "open-air": {
    id: "open-air",
    label: "Open Air",
    clearColorRgba: [209, 224, 240, 255],
    fogColorRgba: [209, 224, 240, 255],
    fogStartDistance: metersToWorldUnits(96),
    fogEndDistance: metersToWorldUnits(416),
  },
  "green-canopy": {
    id: "green-canopy",
    label: "Canopy Light",
    clearColorRgba: [199, 221, 207, 255],
    fogColorRgba: [177, 204, 184, 255],
    fogStartDistance: metersToWorldUnits(78),
    fogEndDistance: metersToWorldUnits(360),
  },
  "dry-haze": {
    id: "dry-haze",
    label: "Dry Haze",
    clearColorRgba: [226, 214, 190, 255],
    fogColorRgba: [215, 197, 164, 255],
    fogStartDistance: metersToWorldUnits(88),
    fogEndDistance: metersToWorldUnits(396),
  },
  ashfall: {
    id: "ashfall",
    label: "Ashfall",
    clearColorRgba: [191, 181, 168, 255],
    fogColorRgba: [163, 143, 127, 255],
    fogStartDistance: metersToWorldUnits(58),
    fogEndDistance: metersToWorldUnits(320),
  },
  "silt-mist": {
    id: "silt-mist",
    label: "Silt Mist",
    clearColorRgba: [190, 205, 202, 255],
    fogColorRgba: [157, 178, 171, 255],
    fogStartDistance: metersToWorldUnits(54),
    fogEndDistance: metersToWorldUnits(300),
  },
  "fungal-lantern": {
    id: "fungal-lantern",
    label: "Fungal Glow",
    clearColorRgba: [184, 197, 219, 255],
    fogColorRgba: [143, 170, 191, 255],
    fogStartDistance: metersToWorldUnits(56),
    fogEndDistance: metersToWorldUnits(304),
  },
  "cold-glass": {
    id: "cold-glass",
    label: "Cold Glass",
    clearColorRgba: [201, 220, 236, 255],
    fogColorRgba: [180, 209, 228, 255],
    fogStartDistance: metersToWorldUnits(74),
    fogEndDistance: metersToWorldUnits(384),
  },
  subterranean: {
    id: "subterranean",
    label: "Underdeep",
    clearColorRgba: [35, 44, 48, 255],
    fogColorRgba: [48, 58, 58, 255],
    fogStartDistance: metersToWorldUnits(18),
    fogEndDistance: metersToWorldUnits(128),
  },
};

export function resolveAmbientWorldProfile(
  probe: AmbientWorldProbe,
  options: { observedUndergroundBiomeId?: string | null } = {},
): AmbientWorldProfile {
  const undergroundBiomeId = options.observedUndergroundBiomeId ?? null;
  const profileId = undergroundBiomeId
    ? resolveUndergroundProfileId(undergroundBiomeId)
    : resolveSurfaceProfileId(probe.biomeId, probe.regionalVariantId);
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
    clearColorRgba: mixRgba(DEFAULT_RENDER_ENVIRONMENT.clearColorRgba, definition.clearColorRgba, intensity * 0.72),
    fogColorRgba: mixRgba(DEFAULT_RENDER_ENVIRONMENT.fogColorRgba, definition.fogColorRgba, intensity),
    fogStartDistance,
    fogEndDistance,
  };
}

export function buildAmbientRenderEnvironment(profile: AmbientWorldProfile): RenderEnvironment {
  return {
    clearColorRgba: profile.clearColorRgba,
    fogColorRgba: profile.fogColorRgba,
    fogStartDistance: profile.fogStartDistance,
    fogEndDistance: profile.fogEndDistance,
  };
}

function resolveSurfaceProfileId(biomeId: BiomeId, regionalVariantId: RegionalVariantId | null): AmbientProfileId {
  switch (regionalVariantId) {
    case "badlands_crater":
    case "ember_caldera":
      return "ashfall";
    case "marsh_blackwater":
    case "moor_shadowglass":
    case "saltflat_mirror":
      return "silt-mist";
    case "firefly_lantern":
    case "fungal_moonlit":
    case "bloom_prism":
      return "fungal-lantern";
    case "dunes_glass":
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

  switch (biomeId) {
    case "verdant":
    case "fern":
    case "bloom":
      return "green-canopy";
    case "marsh":
    case "moor":
    case "saltflat":
      return "silt-mist";
    case "firefly":
    case "fungal":
      return "fungal-lantern";
    case "ember":
    case "badlands":
      return "ashfall";
    case "tundra":
    case "highland":
    case "shardlands":
      return "cold-glass";
    case "savanna":
    case "steppe":
    case "dunes":
      return "dry-haze";
  }
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
  const profilePressure = probe.specialStrength * 0.22 + probe.regionalVariantStrength * 0.20;
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
