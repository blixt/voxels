import { expect, test } from "bun:test";

import {
  RPG_CAVE_ENCOUNTER_MODIFIERS,
  RPG_ENCOUNTER_ZONES,
  RPG_ROUTE_ENCOUNTER_MODIFIERS,
  getRpgEncounterZoneDefinitions,
  sampleRpgEncounterMeters,
  type RpgEncounterSample,
} from "../src/engine/rpg-encounters.ts";
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

function topFaction(sample: RpgEncounterSample): string | null {
  return sample.factionHints[0]?.factionId ?? null;
}
