import { expect, test } from "bun:test";

import { resolveAmbientWorldProfile, buildAmbientRenderEnvironment } from "../src/engine/ambient-environment.ts";
import { ProceduralWorldGenerator, type LandmarkId } from "../src/engine/procedural-generator.ts";
import { sampleRpgEncounterWorldUnits } from "../src/engine/rpg-encounters.ts";
import {
  applyWorldAtmosphere,
  resolveWorldClock,
  sampleWorldSystems,
  WORLD_DAY_LENGTH_SECONDS,
} from "../src/engine/world-systems.ts";

test("world clock advances through a full day and exposes readable labels", () => {
  const start = resolveWorldClock(0);
  const later = resolveWorldClock(WORLD_DAY_LENGTH_SECONDS / 2);
  const nextDay = resolveWorldClock(WORLD_DAY_LENGTH_SECONDS);

  expect(start.day).toBe(1);
  expect(start.clockLabel).toMatch(/^\d\d:\d\d$/);
  expect(later.clockLabel).not.toBe(start.clockLabel);
  expect(nextDay.day).toBe(2);
  expect(nextDay.clockLabel).toBe(start.clockLabel);
});

test("world systems produce coherent exploration cues from a generator probe", () => {
  const generator = new ProceduralWorldGenerator(1337);
  const probe = generator.sampleBiomeProbe(0, 0);
  const ambient = resolveAmbientWorldProfile(probe);
  const encounter = sampleRpgEncounterWorldUnits(0, 0);

  const systems = sampleWorldSystems(180, probe, ambient, encounter, "surface");

  expect(systems.weather.label.length).toBeGreaterThan(0);
  expect(systems.area.floraLabel.length).toBeGreaterThan(0);
  expect(systems.area.faunaLabel.length).toBeGreaterThan(0);
  expect(systems.area.lootId.length).toBeGreaterThan(0);
  expect(systems.area.lootName.length).toBeGreaterThan(0);
  expect(systems.area.lootSignalLabel.length).toBeGreaterThan(0);
  expect(["cartography", "naturalist", "spelunking", "lore"]).toContain(systems.area.lootSkillId);
  expect(systems.area.coherenceLabel).toContain(systems.area.floraLabel);
});

test("visible vegetation landmarks produce specific forage signals", () => {
  const generator = new ProceduralWorldGenerator(1337);
  const baseProbe = generator.sampleBiomeProbe(0, 0);
  const probe = {
    ...baseProbe,
    landmarkId: "berry_bush" as LandmarkId,
  };
  const ambient = resolveAmbientWorldProfile(probe);
  const encounter = sampleRpgEncounterWorldUnits(0, 0);

  const systems = sampleWorldSystems(180, probe, ambient, encounter, "surface");

  expect(systems.area.lootId).toBe("berry-bush-forage");
  expect(systems.area.lootName).toBe("Berry Bush Forage");
  expect(systems.area.lootSkillId).toBe("naturalist");
  expect(systems.area.forageSourceLandmarkId).toBe("berry_bush");
  expect(systems.area.lootInteractionLabel).toBe("Pick berry bush forage");
});

test("vegetation forage signals stay tied to their visible landmark source", () => {
  const generator = new ProceduralWorldGenerator(1337);
  const baseProbe = generator.sampleBiomeProbe(0, 0);
  const ambient = resolveAmbientWorldProfile(baseProbe);
  const encounter = sampleRpgEncounterWorldUnits(0, 0);

  const glowcap = sampleWorldSystems(180, {
    ...baseProbe,
    landmarkId: "glowcap" as LandmarkId,
  }, ambient, encounter, "surface");
  const cactus = sampleWorldSystems(180, {
    ...baseProbe,
    landmarkId: "cactus" as LandmarkId,
  }, ambient, encounter, "surface");

  expect(glowcap.area.forageSourceLandmarkId).toBe("glowcap");
  expect(glowcap.area.lootId).toBe("glowcap-reagents");
  expect(cactus.area.forageSourceLandmarkId).toBe("cactus");
  expect(cactus.area.lootId).toBe("cactus-pulp");
});

test("world atmosphere darkens and tightens fog at night without expanding fog culling distance", () => {
  const generator = new ProceduralWorldGenerator(1337);
  const probe = generator.sampleBiomeProbe(0, 0);
  const ambient = resolveAmbientWorldProfile(probe);
  const baseEnvironment = buildAmbientRenderEnvironment(ambient);
  const encounter = sampleRpgEncounterWorldUnits(0, 0);
  const systems = sampleWorldSystems(WORLD_DAY_LENGTH_SECONDS * 0.72, probe, ambient, encounter, "surface");
  const nightClock = resolveWorldClock(WORLD_DAY_LENGTH_SECONDS * 0.72);

  const environment = applyWorldAtmosphere(baseEnvironment, nightClock, systems.weather);

  expect(environment.fogEndDistance).toBeLessThanOrEqual(baseEnvironment.fogEndDistance);
  expect(environment.fogEndDistance).toBeGreaterThan(environment.fogStartDistance);
  expect(environment.skyCloudCoverage).toBeGreaterThanOrEqual(baseEnvironment.skyCloudCoverage);
  expect(environment.lightingTerms?.[1]).toBeLessThan(0.5);
  expect(environment.lightDirection?.[1]).toBeLessThan(0);
  expect(environment.moonGlowIntensity).toBeGreaterThan(0);
  expect(environment.starIntensity).toBeGreaterThan(0);
  expect(environment.sunGlowIntensity).toBeLessThan(0.25);
});

test("world atmosphere exposes celestial cues that weather can obscure", () => {
  const generator = new ProceduralWorldGenerator(1337);
  const probe = generator.sampleBiomeProbe(0, 0);
  const ambient = resolveAmbientWorldProfile(probe);
  const baseEnvironment = buildAmbientRenderEnvironment(ambient);
  const dayClock = resolveWorldClock(WORLD_DAY_LENGTH_SECONDS * 0.10);
  const nightClock = resolveWorldClock(WORLD_DAY_LENGTH_SECONDS * 0.72);
  const clearWeather = {
    id: "clear" as const,
    label: "Clear",
    intensity: 0,
    cloudCoverageBonus: 0,
    fogMultiplier: 1,
    ashfallBonus: 0,
    fungalGlowBonus: 0,
  };
  const stormWeather = {
    ...clearWeather,
    id: "ash-squall" as const,
    label: "Ash Squall",
    intensity: 1,
    cloudCoverageBonus: 0.82,
    fogMultiplier: 1.5,
    ashfallBonus: 1,
  };

  const day = applyWorldAtmosphere(baseEnvironment, dayClock, clearWeather);
  const clearNight = applyWorldAtmosphere(baseEnvironment, nightClock, clearWeather);
  const stormNight = applyWorldAtmosphere(baseEnvironment, nightClock, stormWeather);

  expect(day.sunGlowIntensity).toBeGreaterThan(0.25);
  expect(day.starIntensity).toBe(0);
  expect(clearNight.moonGlowIntensity).toBeGreaterThan(0.05);
  expect(clearNight.starIntensity).toBeGreaterThan(0.02);
  expect(stormNight.moonGlowIntensity).toBeLessThan(clearNight.moonGlowIntensity!);
  expect(stormNight.starIntensity).toBeLessThan(clearNight.starIntensity!);
  expect(stormNight.fogEndDistance).toBeLessThanOrEqual(baseEnvironment.fogEndDistance);
});
