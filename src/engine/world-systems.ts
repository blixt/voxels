import { clamp } from "./math.ts";
import type { Rgba, SkyWeatherEnvironment } from "./render-constants.ts";
import type { RenderEnvironment } from "./water-visuals.ts";
import type { AmbientWorldProfile } from "./ambient-environment.ts";
import type {
  BiomeId,
  ProceduralBiomeProbe,
  RegionalVariantId,
  UndergroundBiomeId,
} from "./procedural-generator.ts";
import type { RpgEncounterSample } from "./rpg-encounters.ts";
import type { SkillId } from "./skill-journal.ts";

export type TimeOfDayPhaseId = "dawn" | "morning" | "day" | "dusk" | "night";
export type WorldWeatherId = "clear" | "silt-mist" | "ash-squall" | "blackwater-rain" | "glass-haze" | "sporeglow";

export interface WorldClockSnapshot {
  day: number;
  hour: number;
  minute: number;
  phaseId: TimeOfDayPhaseId;
  phaseLabel: string;
  clockLabel: string;
  daylight: number;
  night: number;
  dawnDusk: number;
  dayProgress: number;
}

export interface WorldWeatherSnapshot {
  id: WorldWeatherId;
  label: string;
  intensity: number;
  cloudCoverageBonus: number;
  fogMultiplier: number;
  ashfallBonus: number;
  fungalGlowBonus: number;
}

export interface WorldAreaActivitySnapshot {
  floraLabel: string;
  faunaLabel: string;
  lootId: string;
  lootName: string;
  lootSignalLabel: string;
  lootInteractionLabel: string;
  lootSkillId: SkillId;
  forageSourceLandmarkId: string | null;
  hazardLabel: string;
  coherenceLabel: string;
}

export interface WorldSystemSnapshot {
  clock: WorldClockSnapshot;
  weather: WorldWeatherSnapshot;
  area: WorldAreaActivitySnapshot;
}

export const WORLD_DAY_LENGTH_SECONDS = 24 * 60;
const START_HOUR = 6.75;

const WEATHER_LABELS: Record<WorldWeatherId, string> = {
  clear: "Clear",
  "silt-mist": "Silt Mist",
  "ash-squall": "Ash Squall",
  "blackwater-rain": "Blackwater Rain",
  "glass-haze": "Glass Haze",
  sporeglow: "Sporeglow",
};

const FLORA_BY_BIOME: Record<BiomeId, string> = {
  verdant: "oak, redleaf, and fern cover",
  savanna: "flower grass, acacia, and low scrub",
  steppe: "wind grass, shrubs, and standing stones",
  dunes: "palms, cactus, and glassy salt grass",
  badlands: "dead snags, thorn brush, and ash scrub",
  highland: "fir, redleaf, and stone tor cover",
  moor: "heather, bog shrubs, and dark reeds",
  tundra: "fir, frost shrub, and blue-ice spires",
  marsh: "mangrove, cypress, and reed beds",
  firefly: "lantern trees and glowing reeds",
  saltflat: "salt spires and mirror-crust reeds",
  fern: "giant fern, root stumps, and cenote moss",
  fungal: "glowcap towers and mycelial shelves",
  ember: "basalt spires and ember scrub",
  bloom: "giant blooms and prism groundcover",
  shardlands: "crystal reeds and glass cairns",
};

const UNDERGROUND_FLORA: Record<UndergroundBiomeId, string> = {
  rooted: "root curtains and pale moss",
  sedimentary: "lichen seams and dry shelf fungus",
  sandy: "sand lichen and brittle cave grass",
  granitic: "redleaf roots and stone lichen",
  froststone: "frost lichen and cold glass veins",
  basaltic: "ash lichen and hot vent crust",
  peaty: "peat moss and blackwater roots",
  saline: "salt roots and pale brine reeds",
  mycelial: "glowcaps and mycelial veils",
  crystalline: "crystal reeds and glass shelf growth",
};

export function sampleWorldSystems(
  elapsedSeconds: number,
  probe: ProceduralBiomeProbe,
  ambientProfile: AmbientWorldProfile,
  encounter: RpgEncounterSample,
  travelContext: "surface" | "underground",
): WorldSystemSnapshot {
  const clock = resolveWorldClock(elapsedSeconds);
  const weather = resolveWorldWeather(probe, ambientProfile, clock, travelContext);
  return {
    clock,
    weather,
    area: resolveWorldAreaActivity(probe, encounter, weather, travelContext),
  };
}

export function resolveWorldClock(elapsedSeconds: number): WorldClockSnapshot {
  const safeElapsed = Number.isFinite(elapsedSeconds) ? Math.max(0, elapsedSeconds) : 0;
  const totalHours = START_HOUR + safeElapsed / WORLD_DAY_LENGTH_SECONDS * 24;
  const day = 1 + Math.floor(totalHours / 24);
  const hourFloat = totalHours % 24;
  const hour = Math.floor(hourFloat);
  const minute = Math.floor((hourFloat - hour) * 60);
  const daylight = smoothstep(5.25, 7.25, hourFloat) * (1 - smoothstep(18.25, 20.25, hourFloat));
  const dawn = 1 - Math.abs(hourFloat - 6.25) / 1.35;
  const dusk = 1 - Math.abs(hourFloat - 19.1) / 1.55;
  const dawnDusk = clamp(Math.max(dawn, dusk), 0, 1);
  const night = 1 - daylight;
  const phaseId = resolveTimeOfDayPhase(hourFloat, daylight, dawnDusk);
  return {
    day,
    hour,
    minute,
    phaseId,
    phaseLabel: phaseLabel(phaseId),
    clockLabel: `${hour.toString().padStart(2, "0")}:${minute.toString().padStart(2, "0")}`,
    daylight,
    night,
    dawnDusk,
    dayProgress: hourFloat / 24,
  };
}

export function applyWorldAtmosphere(
  environment: RenderEnvironment & SkyWeatherEnvironment,
  clock: WorldClockSnapshot,
  weather: WorldWeatherSnapshot,
): RenderEnvironment & SkyWeatherEnvironment {
  const nightAmount = clock.night * (1 - clock.dawnDusk * 0.48);
  const duskAmount = clock.dawnDusk;
  const fogMultiplier = weather.fogMultiplier * (1 + nightAmount * 0.18);
  const fogEndDistance = clamp(
    environment.fogEndDistance / fogMultiplier,
    environment.fogStartDistance + 16,
    environment.fogEndDistance,
  );
  const fogStartDistance = clamp(
    environment.fogStartDistance / Math.max(1, weather.fogMultiplier * 0.72),
    8,
    fogEndDistance - 8,
  );
  const nightSkyTop = [9, 14, 24, 255] as const;
  const nightHorizon = [29, 36, 48, 255] as const;
  const duskSky = [138, 82, 74, 255] as const;
  const duskHorizon = [218, 135, 82, 255] as const;
  const cloudVeil = clamp(environment.skyCloudCoverage + weather.cloudCoverageBonus + weather.intensity * 0.18, 0, 1);
  const sunAngle = clock.dayProgress * Math.PI * 2 - Math.PI * 0.52;
  const sunHeight = Math.max(0.08, Math.sin(sunAngle) * 0.88 + 0.12);
  const warmLight = clock.dawnDusk * 0.22;
  const ambientLight = 0.15 + clock.daylight * 0.25 + weather.intensity * 0.02;
  const directionalLight = 0.12 + clock.daylight * 0.54 - weather.cloudCoverageBonus * 0.18;
  const hemisphereLight = 0.16 + clock.daylight * 0.18 + clock.night * 0.03;
  return {
    ...environment,
    lightDirection: [
      Math.cos(sunAngle) * 0.55,
      -sunHeight,
      Math.sin(sunAngle) * 0.55,
    ],
    lightingTerms: [
      clamp(ambientLight + warmLight * 0.05, 0.12, 0.48),
      clamp(directionalLight, 0.08, 0.66),
      clamp(hemisphereLight + warmLight * 0.04, 0.10, 0.34),
    ],
    clearColorRgba: mixRgba(
      mixRgba(environment.clearColorRgba, duskHorizon, duskAmount * 0.22),
      nightHorizon,
      nightAmount * 0.72,
    ),
    fogColorRgba: mixRgba(
      mixRgba(environment.fogColorRgba, duskHorizon, duskAmount * 0.24),
      nightHorizon,
      nightAmount * 0.68,
    ),
    fogStartDistance,
    fogEndDistance,
    skyTopColorRgba: mixRgba(
      mixRgba(environment.skyTopColorRgba, duskSky, duskAmount * 0.42),
      nightSkyTop,
      nightAmount * 0.82,
    ),
    skyHorizonColorRgba: mixRgba(
      mixRgba(environment.skyHorizonColorRgba, duskHorizon, duskAmount * 0.48),
      nightHorizon,
      nightAmount * 0.74,
    ),
    skyCloudColorRgba: mixRgba(environment.skyCloudColorRgba, [118, 128, 143, 255], nightAmount * 0.42),
    skyCloudCoverage: clamp(environment.skyCloudCoverage + weather.cloudCoverageBonus + nightAmount * 0.06, 0, 1),
    skyCloudBand: clamp(environment.skyCloudBand - weather.intensity * 0.08 + duskAmount * 0.04, 0.18, 0.82),
    ashfallIntensity: clamp(environment.ashfallIntensity + weather.ashfallBonus, 0, 1),
    fungalGlowIntensity: clamp(environment.fungalGlowIntensity + weather.fungalGlowBonus + nightAmount * 0.10, 0, 1),
    sunGlowIntensity: clamp((clock.daylight * 0.68 + clock.dawnDusk * 0.74) * (1 - cloudVeil * 0.50), 0, 1),
    moonGlowIntensity: clamp((clock.night - clock.dawnDusk * 0.34) * (1 - cloudVeil * 0.72), 0, 1),
    starIntensity: clamp(clock.night ** 1.35 * (1 - clock.dawnDusk) * (1 - cloudVeil * 0.94), 0, 1),
  };
}

function resolveWorldWeather(
  probe: ProceduralBiomeProbe,
  ambientProfile: AmbientWorldProfile,
  clock: WorldClockSnapshot,
  travelContext: "surface" | "underground",
): WorldWeatherSnapshot {
  const fields = probe.fields;
  const seed = hash2(Math.floor(probe.surfaceY * 0.17), Math.floor(clock.day * 13 + clock.hour / 3));
  const pulse = 0.5 + 0.5 * Math.sin(clock.dayProgress * Math.PI * 2 + seed * Math.PI * 2);
  const wetPressure = fields.moisture * 0.62 + fields.oceanness * 0.34 + pulse * 0.22;
  const ashPressure = fields.volcanism * 0.78 + fields.desolation * 0.28 + ambientProfile.ashfallIntensity * 0.52;
  const magicPressure = fields.magic * 0.64 + ambientProfile.fungalGlowIntensity * 0.58 + (clock.night * 0.18);

  if (travelContext === "underground") {
    if (probe.undergroundBiomeId === "mycelial" || probe.undergroundBiomeId === "crystalline" || magicPressure > 0.74) {
      return weather("sporeglow", 0.46 + magicPressure * 0.38);
    }
    return weather("silt-mist", 0.22 + Math.max(fields.moisture, fields.oceanness) * 0.24);
  }
  if (ashPressure > 0.82) {
    return weather("ash-squall", 0.34 + ashPressure * 0.48);
  }
  if (probe.biomeId === "marsh" || probe.biomeId === "moor" || probe.biomeId === "saltflat" || wetPressure > 0.86) {
    return weather("blackwater-rain", 0.30 + wetPressure * 0.42);
  }
  if (probe.biomeId === "shardlands" || probe.regionalVariantId === "dunes_glass" || probe.regionalVariantId === "saltflat_mirror") {
    return weather("glass-haze", 0.34 + Math.max(fields.magic, fields.oceanness) * 0.34);
  }
  if (magicPressure > 0.88) {
    return weather("sporeglow", 0.32 + magicPressure * 0.38);
  }
  if (wetPressure > 0.66 || ambientProfile.id === "silt-mist") {
    return weather("silt-mist", 0.26 + wetPressure * 0.32);
  }
  return weather("clear", 0.08 + pulse * 0.12);
}

function resolveWorldAreaActivity(
  probe: ProceduralBiomeProbe,
  encounter: RpgEncounterSample,
  weather: WorldWeatherSnapshot,
  travelContext: "surface" | "underground",
): WorldAreaActivitySnapshot {
  const underground = travelContext === "underground";
  const floraLabel = underground
    ? UNDERGROUND_FLORA[probe.undergroundBiomeId]
    : FLORA_BY_BIOME[probe.biomeId];
  const faunaLabel = resolveFaunaLabel(probe, encounter, underground);
  const loot = resolveLootSignal(probe.regionalVariantId, probe.landmarkId, underground, weather);
  const hazardLabel = resolveHazardLabel(probe, encounter, weather, underground);
  return {
    floraLabel,
    faunaLabel,
    lootId: loot.id,
    lootName: loot.name,
    lootSignalLabel: loot.label,
    lootInteractionLabel: loot.interactionLabel,
    lootSkillId: loot.skillId,
    forageSourceLandmarkId: loot.forageSourceLandmarkId,
    hazardLabel,
    coherenceLabel: underground
      ? `cave ecology: ${floraLabel}; ${faunaLabel}`
      : `surface ecology: ${floraLabel}; ${faunaLabel}`,
  };
}

function resolveFaunaLabel(
  probe: ProceduralBiomeProbe,
  encounter: RpgEncounterSample,
  underground: boolean,
): string {
  const factionId = encounter.factionHints[0]?.factionId ?? null;
  if (underground) {
    if (factionId === "kwama-brood") {
      return "kwama tunnels and brood sign";
    }
    if (probe.undergroundBiomeId === "mycelial") {
      return "glowmoths and cave foragers";
    }
    if (probe.undergroundBiomeId === "crystalline") {
      return "glass beetles and shard keepers";
    }
    return "cave skitterers and old pilgrim traces";
  }
  switch (probe.biomeId) {
    case "marsh":
    case "moor":
      return "scrib, mudcrabs, and marsh foragers";
    case "badlands":
    case "ember":
      return "ash hounds and pilgrim scouts";
    case "fungal":
    case "firefly":
      return "netch calves, glowmoths, and spore gatherers";
    case "shardlands":
      return "glass beetles and cairn keepers";
    case "tundra":
    case "highland":
      return "cliff racers and redleaf guides";
    default:
      return factionId ? "scouts, pack guar, and road watchers" : "small game and distant road traffic";
  }
}

function resolveLootSignal(
  variantId: RegionalVariantId | null,
  landmarkId: string | null,
  underground: boolean,
  weather: WorldWeatherSnapshot,
): { id: string; name: string; label: string; interactionLabel: string; skillId: SkillId; forageSourceLandmarkId: string | null } {
  const vegetationForage = resolveVegetationForageSignal(landmarkId);
  if (vegetationForage) {
    return vegetationForage;
  }
  if (landmarkId === "ashlander_travel_pack") {
    return loot("travel-pack-cache", "Travel Pack Cache", "travel pack cache nearby", "Check the travel pack", "cartography");
  }
  if (landmarkId?.includes("shrine") || landmarkId?.includes("velothi")) {
    return loot("shrine-offerings", "Shrine Offerings", "shrine offerings and route notes", "Read the route offerings", "lore");
  }
  if (underground) {
    return weather.id === "sporeglow"
      ? loot("glowcap-reagents", "Glowcap Reagents", "glowcap reagents and old tools", "Gather glowcap reagents", "naturalist")
      : loot("lost-cave-pack", "Lost Cave Pack", "mineral seams and lost packs", "Search the lost pack", "spelunking");
  }
  switch (variantId) {
    case "dunes_glass":
    case "saltflat_mirror":
    case "bloom_prism":
      return loot("glass-alchemy-trace", "Glass Alchemy Trace", "glass shards and alchemy traces", "Collect glass traces", "lore");
    case "ember_caldera":
    case "badlands_crater":
    case "ash_wastes":
      return loot("ashlander-cache", "Ashlander Cache", "ashlander caches and basalt scrap", "Scavenge the ash cache", "cartography");
    case "marsh_blackwater":
    case "fungal_moonlit":
      return loot("wetland-reagents", "Wetland Reagents", "reagents, spores, and wetland salvage", "Forage wetland reagents", "naturalist");
    default:
      return loot("trail-forage", "Trail Forage", "forage, road debris, and landmark notes", "Forage the trail", "naturalist");
  }
}

function resolveVegetationForageSignal(
  landmarkId: string | null,
): { id: string; name: string; label: string; interactionLabel: string; skillId: SkillId; forageSourceLandmarkId: string | null } | null {
  switch (landmarkId) {
    case "berry_bush":
      return loot("berry-bush-forage", "Berry Bush Forage", "ripe berries on the bush", "Pick berry bush forage", "naturalist", landmarkId);
    case "glowcap":
    case "mega_glowcap":
      return loot("glowcap-reagents", "Glowcap Reagents", "glowcap caps and luminous spores", "Gather glowcap reagents", "naturalist", landmarkId);
    case "flower_patch":
    case "giant_flower":
      return loot("flower-nectar", "Flower Nectar", "nectar bulbs and bright petals", "Gather flower nectar", "naturalist", landmarkId);
    case "cactus":
      return loot("cactus-pulp", "Cactus Pulp", "water-rich cactus pulp", "Cut cactus pulp", "naturalist", landmarkId);
    case "mangrove":
      return loot("mangrove-cuttings", "Mangrove Cuttings", "mangrove roots and brackish cuttings", "Take mangrove cuttings", "naturalist", landmarkId);
    case "crystal_reeds":
      return loot("crystal-reed-shards", "Crystal Reed Shards", "resonant crystal reed shards", "Collect crystal reed shards", "lore", landmarkId);
    default:
      return null;
  }
}

function loot(
  id: string,
  name: string,
  label: string,
  interactionLabel: string,
  skillId: SkillId,
  forageSourceLandmarkId: string | null = null,
): { id: string; name: string; label: string; interactionLabel: string; skillId: SkillId; forageSourceLandmarkId: string | null } {
  return { id, name, label, interactionLabel, skillId, forageSourceLandmarkId };
}

function resolveHazardLabel(
  probe: ProceduralBiomeProbe,
  encounter: RpgEncounterSample,
  weather: WorldWeatherSnapshot,
  underground: boolean,
): string {
  if (encounter.pressure >= 0.72) {
    return "high encounter pressure";
  }
  if (weather.id === "ash-squall") {
    return "low visibility ash";
  }
  if (weather.id === "blackwater-rain") {
    return "slick ground and deep channels";
  }
  if (underground) {
    return "tight passages and blind drops";
  }
  if (probe.fields.volcanism > 0.7) {
    return "hot stone and unstable scree";
  }
  return "open exploration";
}

function weather(id: WorldWeatherId, intensity: number): WorldWeatherSnapshot {
  const clampedIntensity = clamp(intensity, 0, 1);
  switch (id) {
    case "ash-squall":
      return {
        id,
        label: WEATHER_LABELS[id],
        intensity: clampedIntensity,
        cloudCoverageBonus: 0.18 + clampedIntensity * 0.22,
        fogMultiplier: 1.16 + clampedIntensity * 0.48,
        ashfallBonus: 0.16 + clampedIntensity * 0.32,
        fungalGlowBonus: 0,
      };
    case "blackwater-rain":
      return {
        id,
        label: WEATHER_LABELS[id],
        intensity: clampedIntensity,
        cloudCoverageBonus: 0.20 + clampedIntensity * 0.26,
        fogMultiplier: 1.10 + clampedIntensity * 0.38,
        ashfallBonus: 0,
        fungalGlowBonus: 0.03,
      };
    case "glass-haze":
      return {
        id,
        label: WEATHER_LABELS[id],
        intensity: clampedIntensity,
        cloudCoverageBonus: 0.08 + clampedIntensity * 0.14,
        fogMultiplier: 1.06 + clampedIntensity * 0.26,
        ashfallBonus: 0,
        fungalGlowBonus: 0.04 + clampedIntensity * 0.08,
      };
    case "sporeglow":
      return {
        id,
        label: WEATHER_LABELS[id],
        intensity: clampedIntensity,
        cloudCoverageBonus: 0.10 + clampedIntensity * 0.18,
        fogMultiplier: 1.08 + clampedIntensity * 0.30,
        ashfallBonus: 0,
        fungalGlowBonus: 0.18 + clampedIntensity * 0.42,
      };
    case "silt-mist":
      return {
        id,
        label: WEATHER_LABELS[id],
        intensity: clampedIntensity,
        cloudCoverageBonus: 0.08 + clampedIntensity * 0.18,
        fogMultiplier: 1.08 + clampedIntensity * 0.32,
        ashfallBonus: 0.03 + clampedIntensity * 0.08,
        fungalGlowBonus: 0,
      };
    case "clear":
      return {
        id,
        label: WEATHER_LABELS[id],
        intensity: clampedIntensity,
        cloudCoverageBonus: clampedIntensity * 0.04,
        fogMultiplier: 1,
        ashfallBonus: 0,
        fungalGlowBonus: 0,
      };
  }
}

function resolveTimeOfDayPhase(hour: number, daylight: number, dawnDusk: number): TimeOfDayPhaseId {
  if (dawnDusk > 0.38 && hour < 12) {
    return "dawn";
  }
  if (dawnDusk > 0.34 && hour >= 12) {
    return "dusk";
  }
  if (daylight < 0.28) {
    return "night";
  }
  return hour < 11 ? "morning" : "day";
}

function phaseLabel(phaseId: TimeOfDayPhaseId): string {
  switch (phaseId) {
    case "dawn":
      return "Dawn";
    case "morning":
      return "Morning";
    case "day":
      return "Day";
    case "dusk":
      return "Dusk";
    case "night":
      return "Night";
  }
}

function mixRgba(left: Rgba, right: Rgba, amount: number): [number, number, number, number] {
  const t = clamp(amount, 0, 1);
  return [
    Math.round(left[0] + (right[0] - left[0]) * t),
    Math.round(left[1] + (right[1] - left[1]) * t),
    Math.round(left[2] + (right[2] - left[2]) * t),
    Math.round(left[3] + (right[3] - left[3]) * t),
  ];
}

function smoothstep(edge0: number, edge1: number, value: number): number {
  const t = clamp((value - edge0) / (edge1 - edge0), 0, 1);
  return t * t * (3 - 2 * t);
}

function hash2(x: number, z: number): number {
  let hash = Math.imul(x | 0, 374761393) ^ Math.imul(z | 0, 668265263);
  hash = (hash ^ (hash >>> 13)) >>> 0;
  hash = Math.imul(hash, 1274126177) >>> 0;
  return ((hash ^ (hash >>> 16)) >>> 0) / 0xffffffff;
}
