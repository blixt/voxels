import { expect, test } from "bun:test";

import {
  WORLD_ATLAS,
  atlasMetersToWorldUnits,
  atlasWorldUnitsToMeters,
  sampleWorldAtlasMeters,
  type AtlasRegionId,
} from "../src/engine/world-atlas.ts";

const EXPECTED_REGION_METADATA = {
  "red-mountain": {
    biomeId: "ember",
    regionalVariantId: "ember_caldera",
    ambientProfileId: "ashfall",
  },
  "ashen-badlands": {
    biomeId: "badlands",
    regionalVariantId: "ash_wastes",
    ambientProfileId: "ashfall",
  },
  "bitter-coast": {
    biomeId: "marsh",
    regionalVariantId: "marsh_blackwater",
    ambientProfileId: "silt-mist",
  },
  grazelands: {
    biomeId: "savanna",
    regionalVariantId: "savanna_flowersea",
    ambientProfileId: "dry-haze",
  },
  "salt-marsh-basin": {
    biomeId: "saltflat",
    regionalVariantId: "saltflat_mirror",
    ambientProfileId: "silt-mist",
  },
  "glass-shard-coast": {
    biomeId: "shardlands",
    regionalVariantId: "dunes_glass",
    ambientProfileId: "cold-glass",
  },
  "west-gash": {
    biomeId: "highland",
    regionalVariantId: "highland_redleaf",
    ambientProfileId: "green-canopy",
  },
  "inner-sea": {
    biomeId: "moor",
    regionalVariantId: "moor_shadowglass",
    ambientProfileId: "silt-mist",
  },
} as const satisfies Record<
  AtlasRegionId,
  {
    biomeId: string;
    regionalVariantId: string;
    ambientProfileId: string;
  }
>;

test("world atlas defines the eight authored macro regions", () => {
  expect(WORLD_ATLAS.version).toBe("20260509-wave1-atlas-foundation");
  expect(WORLD_ATLAS.regions).toHaveLength(8);
  expect(new Set(WORLD_ATLAS.regions.map((region) => region.id)).size).toBe(8);
  expect(WORLD_ATLAS.island.origin).toEqual({ x: -180, z: -520 });
  expect(WORLD_ATLAS.island.radius).toEqual({ x: 6_400, z: 5_850 });
});

test("region centers sample their expected primary metadata", () => {
  for (const region of WORLD_ATLAS.regions) {
    const sample = sampleWorldAtlasMeters(region.center.x, region.center.z);
    const expected = EXPECTED_REGION_METADATA[region.id];

    expect(sample.primaryRegionId).toBe(region.id);
    expect(sample.secondaryRegionId).not.toBe(region.id);
    expect(sample.primaryBiomeId).toBe(expected.biomeId);
    expect(sample.regionalVariantId).toBe(expected.regionalVariantId);
    expect(sample.ambientProfileId).toBe(expected.ambientProfileId);
    expect(sample.islandInterior).toBeGreaterThan(0.8);
    expect(sample.surfaceClass).toBe("land");
  }
});

test("each region owns at least seventy percent of its inner ellipse", () => {
  for (const region of WORLD_ATLAS.regions) {
    let samples = 0;
    let primaryHits = 0;

    for (let xStep = -6; xStep <= 6; xStep += 1) {
      for (let zStep = -6; zStep <= 6; zStep += 1) {
        const localX = (xStep / 6) * 0.65;
        const localZ = (zStep / 6) * 0.65;
        if (localX * localX + localZ * localZ > 0.65 * 0.65) {
          continue;
        }

        samples += 1;
        const sample = sampleWorldAtlasMeters(
          region.center.x + localX * region.radius.x,
          region.center.z + localZ * region.radius.z,
        );
        if (sample.primaryRegionId === region.id) {
          primaryHits += 1;
        }
      }
    }

    expect(primaryHits / samples).toBeGreaterThanOrEqual(0.7);
  }
});

test("outside the finite island returns ocean classifications instead of land biomes", () => {
  const { origin, radius } = WORLD_ATLAS.island;
  const waterSamples = [
    { x: origin.x + radius.x * 1.12, z: origin.z, surfaceClass: "coastal-shelf", biomeId: "ocean" },
    { x: origin.x - radius.x * 1.18, z: origin.z, surfaceClass: "coastal-shelf", biomeId: "ocean" },
    { x: origin.x, z: origin.z + radius.z * 1.18, surfaceClass: "coastal-shelf", biomeId: "ocean" },
    { x: origin.x + radius.x * 1.75, z: origin.z, surfaceClass: "deep-ocean", biomeId: "deep-ocean" },
    { x: origin.x, z: origin.z - radius.z * 1.75, surfaceClass: "deep-ocean", biomeId: "deep-ocean" },
  ] as const;

  for (const point of waterSamples) {
    const sample = sampleWorldAtlasMeters(point.x, point.z);

    expect(sample.islandInterior).toBeLessThan(0.08);
    expect(sample.primaryRegionId).toBeNull();
    expect(sample.secondaryRegionId).toBeNull();
    expect(sample.surfaceClass).toBe(point.surfaceClass);
    expect(sample.primaryBiomeId).toBe(point.biomeId);
    expect(sample.regionalVariantId).toBeNull();
    expect(sample.ambientProfileId).toBeNull();
  }
});

test("authored region-edge anchors expose plausible secondary regions", () => {
  for (const edge of WORLD_ATLAS.regionEdges) {
    const sample = sampleWorldAtlasMeters(edge.validationAnchor.x, edge.validationAnchor.z);
    const sampledRegions = new Set([sample.primaryRegionId, sample.secondaryRegionId]);

    expect(sample.regionEdgeId).toBe(edge.id);
    expect(sampledRegions.has(edge.from)).toBe(true);
    expect(sampledRegions.has(edge.to)).toBe(true);
    expect(sample.regionBlend).toBeGreaterThan(0.2);
  }
});

test("atlas coordinate conversion helpers round-trip meters and world units", () => {
  const pointM = { x: 123.5, z: -456.25 };
  const pointWorld = atlasMetersToWorldUnits(pointM);
  const roundTrip = atlasWorldUnitsToMeters(pointWorld);

  expect(pointWorld).toEqual({ x: 1_235, z: -4_562.5 });
  expect(roundTrip.x).toBeCloseTo(pointM.x, 8);
  expect(roundTrip.z).toBeCloseTo(pointM.z, 8);
});
