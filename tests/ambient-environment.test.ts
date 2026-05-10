import { expect, test } from "bun:test";

import {
  buildAmbientRenderEnvironment,
  resolveAmbientWorldProfile,
  type AmbientProfileId,
  type AmbientWorldProbe,
} from "../src/engine/ambient-environment.ts";
import { DEFAULT_RENDER_ENVIRONMENT } from "../src/engine/water-visuals.ts";
import { hexColorToMaterial, ProceduralWorldGenerator, type BiomeId } from "../src/engine/procedural-generator.ts";
import { metersToWorldUnits } from "../src/engine/scale.ts";
import { WORLD_ATLAS } from "../src/engine/world-atlas.ts";
import { WORLD_REGION_AUTHORITY_THRESHOLD } from "../src/engine/worldgen-region.ts";

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
  islandInterior: 1,
  coastalShelf: 0,
  shorelineBand: 0,
  deepOcean: 0,
};

test("ambient profiles map major biome moods to distinct rendering environments", () => {
  const cases: Array<[BiomeId, AmbientWorldProbe["regionalVariantId"], AmbientProfileId]> = [
    ["verdant", null, "green-canopy"],
    ["savanna", null, "dry-haze"],
    ["marsh", null, "wetlands"],
    ["saltflat", null, "salt-marsh"],
    ["fungal", null, "fungal-lantern"],
    ["ember", null, "ashfall"],
    ["tundra", null, "cold-glass"],
    ["shardlands", null, "glass-coast"],
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
  expect(ash.label).toBe("Ashlands");
  expect(ash.skyTopColorRgba[0]).toBeLessThan(ash.skyHorizonColorRgba[0]);
  expect(ash.skyTopColorRgba[2]).toBeLessThan(90);
  expect(ash.skyCloudCoverage).toBeGreaterThan(0.78);
  expect(ash.ashfallIntensity).toBeGreaterThan(0.78);
  expect(ashEnvironment.ashfallIntensity).toBe(ash.ashfallIntensity);
  expect(fungal.id).toBe("fungal-lantern");
  expect(fungal.fungalGlowIntensity).toBeGreaterThan(0.65);
  expect(fungal.ashfallIntensity).toBeLessThan(ash.ashfallIntensity);
});

test("old-road materials and landmarks pull dry routes into ash haze", () => {
  const routeSurface = resolveAmbientWorldProfile(buildProbe({
    biomeId: "steppe",
    surfaceMaterial: hexColorToMaterial("#655"),
    fields: {
      ...BASE_FIELDS,
      desolation: 0.42,
      strata: 0.60,
    },
  }));
  const routeLandmark = resolveAmbientWorldProfile(buildProbe({
    biomeId: "savanna",
    landmarkId: "pilgrim_lantern",
  }));
  const wetRoute = resolveAmbientWorldProfile(buildProbe({
    biomeId: "saltflat",
    surfaceMaterial: hexColorToMaterial("#887"),
  }));

  expect(routeSurface.id).toBe("ashfall");
  expect(routeSurface.fogEndDistance).toBeGreaterThan(metersToWorldUnits(340));
  expect(routeSurface.fogEndDistance).toBeLessThan(metersToWorldUnits(360));
  expect(routeSurface.ashfallIntensity).toBeGreaterThan(0.50);
  expect(routeLandmark.id).toBe("ashfall");
  expect(wetRoute.id).toBe("salt-marsh");
});

test("regional atmosphere profiles are meaningfully distinct without extra runtime effects", () => {
  const profiles = [
    resolveAmbientWorldProfile(buildProbe({
      biomeId: "ember",
      regionalVariantId: "ember_caldera",
      regionalVariantStrength: 1,
      specialStrength: 1,
    })),
    resolveAmbientWorldProfile(buildProbe({
      biomeId: "saltflat",
      regionalVariantId: "saltflat_mirror",
      regionalVariantStrength: 1,
      fields: { ...BASE_FIELDS, moisture: 0.76, oceanness: 0.72 },
    })),
    resolveAmbientWorldProfile(buildProbe({
      biomeId: "marsh",
      regionalVariantId: "marsh_blackwater",
      regionalVariantStrength: 1,
      fields: { ...BASE_FIELDS, moisture: 0.88, drainage: 0.22 },
    })),
    resolveAmbientWorldProfile(buildProbe({
      biomeId: "shardlands",
      regionalVariantId: "dunes_glass",
      regionalVariantStrength: 1,
      fields: { ...BASE_FIELDS, oceanness: 0.68, moisture: 0.38 },
    })),
  ];

  expect(profiles.map((profile) => profile.id)).toEqual([
    "ashfall",
    "salt-marsh",
    "wetlands",
    "glass-coast",
  ]);
  for (let leftIndex = 0; leftIndex < profiles.length; leftIndex += 1) {
    for (let rightIndex = leftIndex + 1; rightIndex < profiles.length; rightIndex += 1) {
      expect(profileDistance(profiles[leftIndex]!, profiles[rightIndex]!)).toBeGreaterThan(96);
    }
  }
  expect(profiles[0]!.fogEndDistance).toBeLessThan(profiles[3]!.fogEndDistance);
  expect(profiles[2]!.skyCloudCoverage).toBeGreaterThan(profiles[3]!.skyCloudCoverage + 0.35);
  expect(profiles[3]!.skyHorizonColorRgba[2]).toBeGreaterThan(profiles[0]!.skyHorizonColorRgba[2] + 90);
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

  for (const [x, z] of [
    [34200, -30800],
    [-43600, 9200],
    [-5200, -10800],
    [47400, 21800],
    [-27200, -32600],
  ]) {
    profileIds.add(resolveAmbientWorldProfile(generator.sampleBiomeProbe(x, z)).id);
  }

  expect(profileIds.size).toBeGreaterThanOrEqual(3);
  expect(shortestSurfaceFog).toBeGreaterThanOrEqual(metersToWorldUnits(224));
});

test("authored macro region atmospheres stay separated in clear color, fog, and horizon", () => {
  const profiles = new Map(WORLD_ATLAS.regions.map((region) => {
    const profile = resolveAmbientWorldProfile({
      biomeId: region.biomeId,
      undergroundBiomeId: null,
      regionalVariantId: region.regionalVariantId,
      regionalVariantStrength: region.regionalVariantId ? 1 : 0,
      specialStrength: 0.5,
      surfaceY: 0,
      regionAmbientProfileId: region.ambientProfileId,
      regionStrength: 1,
      fields: BASE_FIELDS,
    });
    return [profile.id, profile];
  }));
  const resolvedProfiles = [...profiles.values()];
  let minColorSeparation = Infinity;

  for (let leftIndex = 0; leftIndex < resolvedProfiles.length; leftIndex += 1) {
    for (let rightIndex = leftIndex + 1; rightIndex < resolvedProfiles.length; rightIndex += 1) {
      const left = resolvedProfiles[leftIndex]!;
      const right = resolvedProfiles[rightIndex]!;
      minColorSeparation = Math.min(minColorSeparation, euclideanColorDistance(left.clearColorRgba, right.clearColorRgba));
      minColorSeparation = Math.min(minColorSeparation, euclideanColorDistance(left.fogColorRgba, right.fogColorRgba));
      minColorSeparation = Math.min(
        minColorSeparation,
        euclideanColorDistance(left.skyHorizonColorRgba, right.skyHorizonColorRgba),
      );
    }
  }

  expect([...profiles.keys()].sort()).toEqual(["ashfall", "cold-glass", "dry-haze", "green-canopy", "silt-mist"]);
  expect(minColorSeparation).toBeGreaterThan(80);
});

test("strong macro regions stabilize sky profile across local biome noise", () => {
  const generator = new ProceduralWorldGenerator(1337);
  const center = generator.sampleBiomeProbe(-13_814, -24_678);
  const centerProfile = resolveAmbientWorldProfile(center);
  const profileIds = new Set<string>();
  const regionIds = new Set<string>();
  const biomeIds = new Set<string>();

  for (let z = -80; z <= 80; z += 16) {
    for (let x = -80; x <= 80; x += 16) {
      const probe = generator.sampleBiomeProbe(-13_814 + x, -24_678 + z);
      profileIds.add(resolveAmbientWorldProfile(probe).id);
      regionIds.add(probe.regionId);
      biomeIds.add(probe.biomeId);
    }
  }

  expect(center.regionStrength).toBeGreaterThan(WORLD_REGION_AUTHORITY_THRESHOLD);
  expect(profileIds).toEqual(new Set([centerProfile.id]));
  expect(regionIds.size).toBeLessThanOrEqual(2);
  expect(biomeIds.size).toBeLessThanOrEqual(3);
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

function profileDistance(
  left: ReturnType<typeof resolveAmbientWorldProfile>,
  right: ReturnType<typeof resolveAmbientWorldProfile>,
): number {
  return colorDistance(left.skyTopColorRgba, right.skyTopColorRgba)
    + colorDistance(left.skyHorizonColorRgba, right.skyHorizonColorRgba)
    + colorDistance(left.fogColorRgba, right.fogColorRgba)
    + Math.abs(left.skyCloudCoverage - right.skyCloudCoverage) * 100
    + Math.abs(left.ashfallIntensity - right.ashfallIntensity) * 100
    + Math.abs(left.fungalGlowIntensity - right.fungalGlowIntensity) * 100;
}

function colorDistance(
  left: readonly [number, number, number, number],
  right: readonly [number, number, number, number],
): number {
  return Math.abs(left[0] - right[0])
    + Math.abs(left[1] - right[1])
    + Math.abs(left[2] - right[2]);
}

function euclideanColorDistance(
  left: readonly [number, number, number, number],
  right: readonly [number, number, number, number],
): number {
  return Math.hypot(left[0] - right[0], left[1] - right[1], left[2] - right[2]);
}
