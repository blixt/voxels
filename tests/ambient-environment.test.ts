import { expect, test } from "bun:test";

import {
  buildAmbientRenderEnvironment,
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
  ridge: 0.5,
  mesa: 0.5,
  desolation: 0.5,
  strata: 0.5,
  surfacePatch: 0.5,
  surfaceGrain: 0.5,
  scatter: 0.5,
  peakness: 0.5,
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

test("ashland and fungal ambience carry cheap sky/weather shader controls", () => {
  const ash = resolveAmbientWorldProfile(buildProbe({
    biomeId: "steppe",
    regionalVariantId: "ember_caldera",
    regionalVariantStrength: 1,
    specialStrength: 1,
    fields: {
      ...BASE_FIELDS,
      volcanism: 0.92,
    },
  }));
  const fungal = resolveAmbientWorldProfile(buildProbe({
    biomeId: "fungal",
    regionalVariantId: "fungal_moonlit",
    regionalVariantStrength: 1,
    specialStrength: 0.8,
    fields: {
      ...BASE_FIELDS,
      magic: 0.90,
      moisture: 0.72,
    },
  }));
  const ashEnvironment = buildAmbientRenderEnvironment(ash);

  expect(ash.id).toBe("ashfall");
  expect(ash.label).toBe("Ash Storm");
  expect(ash.skyTopColorRgba[0]).toBeLessThan(ash.skyHorizonColorRgba[0]);
  expect(ash.skyTopColorRgba[2]).toBeLessThan(90);
  expect(ash.skyCloudCoverage).toBeGreaterThan(0.78);
  expect(ash.ashfallIntensity).toBeGreaterThan(0.78);
  expect(ashEnvironment.ashfallIntensity).toBe(ash.ashfallIntensity);
  expect(fungal.id).toBe("fungal-lantern");
  expect(fungal.fungalGlowIntensity).toBeGreaterThan(0.65);
  expect(fungal.ashfallIntensity).toBeLessThan(ash.ashfallIntensity);
});

test("strong weather skies keep browser-lab-safe luma and color separation", () => {
  const ash = resolveAmbientWorldProfile(buildProbe({
    biomeId: "ember",
    regionalVariantId: "ember_caldera",
    regionalVariantStrength: 1,
    specialStrength: 1,
    fields: {
      ...BASE_FIELDS,
      volcanism: 0.95,
    },
  }));
  const fungal = resolveAmbientWorldProfile(buildProbe({
    biomeId: "fungal",
    regionalVariantId: "fungal_moonlit",
    regionalVariantStrength: 1,
    specialStrength: 1,
    fields: {
      ...BASE_FIELDS,
      magic: 0.96,
      moisture: 0.78,
    },
  }));

  expect(rgbaLuma(ash.skyTopColorRgba)).toBeGreaterThan(40);
  expect(rgbaLuma(ash.skyHorizonColorRgba) - rgbaLuma(ash.skyTopColorRgba)).toBeGreaterThan(55);
  expect(rgbaLuma(ash.fogColorRgba)).toBeGreaterThan(105);
  expect(ash.skyCloudCoverage).toBeGreaterThanOrEqual(0.90);
  expect(ash.skyCloudBand).toBeLessThanOrEqual(0.45);

  expect(fungal.skyHorizonColorRgba[1]).toBeGreaterThan(fungal.skyHorizonColorRgba[0] + 45);
  expect(fungal.skyCloudColorRgba[2]).toBeGreaterThan(fungal.skyCloudColorRgba[0] + 45);
  expect(rgbaLuma(fungal.skyTopColorRgba)).toBeGreaterThan(55);
  expect(rgbaLuma(fungal.fogColorRgba) - rgbaLuma(fungal.skyTopColorRgba)).toBeGreaterThan(85);
  expect(fungal.skyCloudCoverage).toBeGreaterThanOrEqual(0.56);
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

function rgbaLuma(color: readonly [number, number, number, number]): number {
  return color[0] * 0.2126 + color[1] * 0.7152 + color[2] * 0.0722;
}
