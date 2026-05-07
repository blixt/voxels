import { expect, test } from "bun:test";

import {
  resolveAmbientWorldProfile,
  type AmbientProfileId,
  type AmbientWorldProbe,
} from "../src/engine/ambient-environment.ts";
import { DEFAULT_RENDER_ENVIRONMENT } from "../src/engine/water-visuals.ts";
import { ProceduralWorldGenerator, type BiomeId } from "../src/engine/procedural-generator.ts";
import { metersToWorldUnits } from "../src/engine/scale.ts";

const BASE_FIELDS: AmbientWorldProbe["fields"] = {
  temperature: 0.5,
  moisture: 0.5,
  uplift: 0.5,
  drainage: 0.5,
  volcanism: 0.5,
  magic: 0.5,
  globalHeight: 0.5,
  mountainness: 0.5,
  oceanness: 0.5,
};

test("ambient profiles map major biome moods to distinct rendering environments", () => {
  const cases: Array<[BiomeId, AmbientWorldProbe["regionalVariantId"], AmbientProfileId]> = [
    ["verdant", null, "green-canopy"],
    ["savanna", null, "dry-haze"],
    ["marsh", null, "silt-mist"],
    ["fungal", null, "fungal-lantern"],
    ["ember", null, "ashfall"],
    ["tundra", null, "cold-glass"],
    ["steppe", "ember_caldera", "ashfall"],
  ];

  for (const [biomeId, regionalVariantId, expectedId] of cases) {
    const profile = resolveAmbientWorldProfile(buildProbe({ biomeId, regionalVariantId }));
    expect(profile.id).toBe(expectedId);
    expect(profile.label.length).toBeGreaterThan(0);
    expect(profile.fogStartDistance).toBeLessThan(profile.fogEndDistance);
    expect(profile.fogEndDistance).toBeGreaterThanOrEqual(metersToWorldUnits(224));
    expect(profile.fogEndDistance).toBeLessThanOrEqual(DEFAULT_RENDER_ENVIRONMENT.fogEndDistance);
  }
});

test("underground ambience is stronger but still bounded for culling", () => {
  const surface = resolveAmbientWorldProfile(buildProbe({ biomeId: "verdant" }));
  const rooted = resolveAmbientWorldProfile(buildProbe({ biomeId: "verdant" }), {
    observedUndergroundBiomeId: "rooted",
  });
  const basaltic = resolveAmbientWorldProfile(buildProbe({ biomeId: "verdant" }), {
    observedUndergroundBiomeId: "basaltic",
  });

  expect(rooted.id).toBe("subterranean");
  expect(basaltic.id).toBe("ashfall");
  expect(rooted.fogEndDistance).toBeLessThan(surface.fogEndDistance);
  expect(rooted.fogEndDistance).toBeGreaterThanOrEqual(metersToWorldUnits(64));
  expect(rooted.fogEndDistance).toBeLessThanOrEqual(metersToWorldUnits(192));
});

test("procedural ambient profiles are deterministic and varied across the world", () => {
  const generator = new ProceduralWorldGenerator(1337);
  const first = resolveAmbientWorldProfile(generator.sampleBiomeProbe(1440, -960));
  const second = resolveAmbientWorldProfile(generator.sampleBiomeProbe(1440, -960));
  expect(second).toEqual(first);

  const profileIds = new Set<string>();
  let shortestSurfaceFog = Infinity;
  for (let z = -7; z <= 7; z += 1) {
    for (let x = -7; x <= 7; x += 1) {
      const profile = resolveAmbientWorldProfile(generator.sampleBiomeProbe(x * 768, z * 768));
      profileIds.add(profile.id);
      shortestSurfaceFog = Math.min(shortestSurfaceFog, profile.fogEndDistance);
    }
  }

  expect(profileIds.size).toBeGreaterThanOrEqual(5);
  expect(shortestSurfaceFog).toBeGreaterThanOrEqual(metersToWorldUnits(224));
});

function buildProbe(overrides: Partial<AmbientWorldProbe>): AmbientWorldProbe {
  return {
    biomeId: "verdant",
    undergroundBiomeId: "rooted",
    regionalVariantId: null,
    regionalVariantStrength: 0,
    specialStrength: 0,
    surfaceY: 100,
    fields: BASE_FIELDS,
    ...overrides,
  };
}
