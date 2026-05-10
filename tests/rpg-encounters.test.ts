import { expect, test } from "bun:test";

import {
  RPG_CAVE_ENCOUNTER_MODIFIERS,
  RPG_ENCOUNTER_ZONES,
  RPG_ROUTE_ENCOUNTER_MODIFIERS,
  describeRpgEncounterFaction,
  describeRpgEncounterMood,
  describeRpgEncounterPressure,
  describeRpgEncounterScoutResult,
  getRpgEncounterZoneDefinitions,
  sampleRpgEncounterMeters,
  type RpgEncounterSample,
} from "../src/engine/rpg-encounters.ts";
import { sampleRpgEncounterSiteWorldUnits } from "../src/engine/rpg-encounter-sites.ts";
import { metersToWorldUnits } from "../src/engine/scale.ts";
import {
  WORLD_ATLAS,
  type AtlasCaveSystemId,
} from "../src/engine/world-atlas.ts";

test("encounter definitions cover every atlas region, route, and cave system", () => {
  expect(getRpgEncounterZoneDefinitions()).toBe(RPG_ENCOUNTER_ZONES);
  expect(RPG_ENCOUNTER_ZONES.map((zone) => zone.regionId).sort()).toEqual(
    WORLD_ATLAS.regions.map((region) => region.id).sort(),
  );
  expect(RPG_ROUTE_ENCOUNTER_MODIFIERS.map((modifier) => modifier.routeId).sort()).toEqual(
    WORLD_ATLAS.routes.map((route) => route.id).sort(),
  );
  expect(RPG_CAVE_ENCOUNTER_MODIFIERS.map((modifier) => modifier.caveSystemId).sort()).toEqual(
    WORLD_ATLAS.caveSystems.map((caveSystem) => caveSystem.id).sort(),
  );

  for (const zone of RPG_ENCOUNTER_ZONES) {
    expect(zone.pressureBase).toBeGreaterThanOrEqual(0);
    expect(zone.pressureBase).toBeLessThanOrEqual(1);
    expect(zone.factionHints.length).toBeGreaterThanOrEqual(3);
    expect(zone.flavorTags.length).toBeGreaterThanOrEqual(3);
  }
});

test("encounter sampling is deterministic for fixed world-meter coordinates", () => {
  const first = sampleRpgEncounterMeters(-1_240, -2_600);
  const second = sampleRpgEncounterMeters(-1_240, -2_600);

  expect(second).toEqual(first);
  expect(first.pressure).toBeGreaterThanOrEqual(0);
  expect(first.pressure).toBeLessThanOrEqual(1);
  expect(first.factionHints[0]?.weight).toBeGreaterThan(first.factionHints[1]?.weight ?? 0);
});

test("encounter presentation labels are player-facing and deterministic", () => {
  expect(describeRpgEncounterMood("road-truce")).toBe("Road Truce");
  expect(describeRpgEncounterMood("cave-threshold")).toBe("Cave Threshold");
  expect(describeRpgEncounterFaction("temple-pilgrims")).toBe("Temple Pilgrims");
  expect(describeRpgEncounterFaction("opportunist-bandits")).toBe("Road Bandits");
  expect(describeRpgEncounterPressure(0.8)).toBe("High pressure");
  expect(describeRpgEncounterPressure(0.36)).toBe("Low pressure");
});

test("scout results turn pressure and faction data into field notes", () => {
  const sample = sampleRpgEncounterMeters(-1_240, -2_600);
  const result = describeRpgEncounterScoutResult(sample);

  expect(result.label).toMatch(/^Scout /);
  expect(result.pressureLabel).toBe(describeRpgEncounterPressure(sample.pressure));
  expect(result.factionLabel.length).toBeGreaterThan(0);
  expect(result.detail).toContain(result.pressureLabel);
  expect(result.detail).toMatch(/Signs: /);
});

test("regional samples keep distinct pressure, mood, and faction identity", () => {
  const redMountain = sampleRpgEncounterMeters(-120, -650);
  const bitterCoast = sampleRpgEncounterMeters(-3_600, 1_350);
  const grazelands = sampleRpgEncounterMeters(3_600, -2_600);

  expect(redMountain.regionId).toBe("red-mountain");
  expect(bitterCoast.regionId).toBe("bitter-coast");
  expect(grazelands.regionId).toBe("grazelands");
  expect(redMountain.moodId).toBe("ash-pilgrimage");
  expect(bitterCoast.moodId).toBe("blackwater-fog");
  expect(grazelands.moodId).toBe("open-grass-patrol");
  expect(redMountain.flavorTags).toContain("caldera");
  expect(bitterCoast.flavorTags).toContain("blackwater");
  expect(grazelands.flavorTags).toContain("open-plains");
  expect(topFaction(redMountain)).toBe("temple-pilgrims");
  expect(topFaction(bitterCoast)).toBe("marsh-foragers");
  expect(topFaction(grazelands)).toBe("ashlander-scouts");
  expect(redMountain.pressure).toBeGreaterThan(grazelands.pressure);
});

test("authored roads reduce encounter pressure compared with nearby wilderness", () => {
  const road = sampleRpgEncounterMeters(570, 2_080);
  const wilderness = sampleRpgEncounterMeters(1_240, 1_780);

  expect(road.regionId).toBe("inner-sea");
  expect(road.routeId).toBe("inner-sea-shelf-road");
  expect(road.routeSafety).toBeGreaterThan(0.5);
  expect(road.moodId).toBe("road-truce");
  expect(wilderness.regionId).toBe("inner-sea");
  expect(wilderness.routeSafety).toBeLessThan(0.1);
  expect(wilderness.pressure).toBeGreaterThan(road.pressure);
});

test("hazard routes remain safer than wilderness but less safe than pilgrim roads", () => {
  const pilgrimRoad = sampleRpgEncounterMeters(-60, -800);
  const glassHazardRoute = sampleRpgEncounterMeters(4_080, -450);

  expect(pilgrimRoad.routeId).toBe("pilgrim-spine-red");
  expect(glassHazardRoute.routeId).toBe("grazelands-glass-road");
  expect(pilgrimRoad.routeSafety).toBeGreaterThan(glassHazardRoute.routeSafety);
  expect(glassHazardRoute.flavorTags).toContain("warning-cairns");
});

test("cave anchors add cave-specific pressure, mood, factions, and flavor", () => {
  const samples = new Map<AtlasCaveSystemId, RpgEncounterSample>(
    WORLD_ATLAS.caveSystems.map((caveSystem) => [
      caveSystem.id,
      sampleRpgEncounterMeters(caveSystem.validationAnchor.x, caveSystem.validationAnchor.z),
    ]),
  );

  expect(samples.get("ash-kwama-ravines")).toMatchObject({
    caveSystemId: "ash-kwama-ravines",
    moodId: "cave-threshold",
  });
  expect(samples.get("ash-kwama-ravines")?.flavorTags).toContain("kwama-mine");
  expect(topFaction(samples.get("ash-kwama-ravines")!)).toBe("kwama-brood");

  expect(samples.get("glass-crystal-caverns")?.flavorTags).toContain("crystal-cavern");
  expect(samples.get("salt-crust-sinkholes")?.flavorTags).toContain("saline-sinkhole");

  for (const sample of samples.values()) {
    expect(sample.cavePressure).toBeGreaterThan(0.1);
    expect(sample.pressure).toBeGreaterThan(0.45);
  }
});

test("mob encounter sites are deterministic anchored signs inside local cells", () => {
  const worldX = metersToWorldUnits(-1_240);
  const worldZ = metersToWorldUnits(-2_600);
  const encounter = sampleRpgEncounterMeters(-1_240, -2_600);
  const first = sampleRpgEncounterSiteWorldUnits(worldX, worldZ, encounter);
  const second = sampleRpgEncounterSiteWorldUnits(worldX, worldZ, encounter);
  const cellSize = metersToWorldUnits(48);

  expect(second).toEqual(first);
  expect(first.x).toBeGreaterThanOrEqual(first.cellX * cellSize);
  expect(first.x).toBeLessThan((first.cellX + 1) * cellSize);
  expect(first.z).toBeGreaterThanOrEqual(first.cellZ * cellSize);
  expect(first.z).toBeLessThan((first.cellZ + 1) * cellSize);
  expect(first.x).not.toBe(worldX);
  expect(first.z).not.toBe(worldZ);
  expect(["mob-spoor", "mob-nest", "mob-lair"]).toContain(first.role);
  expect(first.name).toContain("Sign");
  expect(first.clueLabel.length).toBeGreaterThan(0);
  expect(first.fieldNote).toContain("sign is");
});

test("mob encounter sites turn cave kwama pressure into lair signs", () => {
  const cave = WORLD_ATLAS.caveSystems.find((system) => system.id === "ash-kwama-ravines")!;
  const encounter = sampleRpgEncounterMeters(cave.validationAnchor.x, cave.validationAnchor.z);
  const site = sampleRpgEncounterSiteWorldUnits(
    metersToWorldUnits(cave.validationAnchor.x),
    metersToWorldUnits(cave.validationAnchor.z),
    encounter,
  );

  expect(topFaction(encounter)).toBe("kwama-brood");
  expect(site.role).toBe("mob-lair");
  expect(site.name).toContain("Lair Sign");
  expect(site.clueLabel).toBe("fresh lair sign");
  expect(site.fieldNote).toContain("Kwama Brood");
  expect(site.fieldNote).toContain("den");
  expect(site.priority).toBeGreaterThanOrEqual(8);
});

test("low-pressure encounter sites stay as spoor instead of lairs", () => {
  const encounter = sampleRpgEncounterMeters(570, 2_080);
  const site = sampleRpgEncounterSiteWorldUnits(metersToWorldUnits(570), metersToWorldUnits(2_080), encounter);

  expect(encounter.routeId).toBe("inner-sea-shelf-road");
  expect(encounter.pressure).toBeLessThan(0.72);
  expect(site.role).toBe("mob-spoor");
  expect(site.clueLabel).toBe("passing spoor");
});

function topFaction(sample: RpgEncounterSample): string | null {
  return sample.factionHints[0]?.factionId ?? null;
}
