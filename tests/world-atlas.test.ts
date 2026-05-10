import { expect, test } from "bun:test";

import {
  WORLD_ATLAS,
  atlasMetersToWorldUnits,
  atlasWorldUnitsToMeters,
  estimateAtlasRegionEllipseAreaM2,
  getAtlasCaveAnchors,
  getAtlasRegionGraph,
  getAtlasRouteAnchors,
  sampleAtlasCaveAnchorMeters,
  sampleAtlasRouteMeters,
  sampleWorldAtlasMeters,
  type AtlasCaveSystemId,
  type AtlasRegionId,
  type AtlasRouteId,
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

const EXPECTED_ROUTE_IDS = [
  "pilgrim-spine-red",
  "ash-gash-pass",
  "badlands-east-trail",
  "bitter-inner-crossing",
  "salt-causeway",
  "inner-sea-shelf-road",
  "grazelands-glass-road",
  "glass-coastal-cairns",
] as const satisfies readonly AtlasRouteId[];

const EXPECTED_CAVE_SYSTEM_IDS = [
  "red-caldera-tubes",
  "ash-kwama-ravines",
  "bitter-root-grottos",
  "west-gash-ravine-caves",
  "glass-crystal-caverns",
  "salt-crust-sinkholes",
] as const satisfies readonly AtlasCaveSystemId[];

test("world atlas defines the eight authored macro regions", () => {
  expect(WORLD_ATLAS.version).toBe("20260509-wave1-atlas-foundation");
  expect(WORLD_ATLAS.regions).toHaveLength(8);
  expect(new Set(WORLD_ATLAS.regions.map((region) => region.id)).size).toBe(8);
  expect(WORLD_ATLAS.island.origin).toEqual({ x: -180, z: -520 });
  expect(WORLD_ATLAS.island.radius).toEqual({ x: 6_400, z: 5_850 });
});

test("world atlas defines the eight initial authored routes", () => {
  expect(WORLD_ATLAS.routes.map((route) => route.id)).toEqual(Array.from(EXPECTED_ROUTE_IDS));

  for (const route of WORLD_ATLAS.routes) {
    expect(route.nodes.length).toBeGreaterThanOrEqual(3);
    expect(route.widthM).toBeGreaterThan(0);
    expect(route.shoulderM).toBeGreaterThan(route.widthM);
    expect(route.expectedRegionIds.length).toBeGreaterThanOrEqual(1);
    expect(route.recommendedSetPieceIds.length).toBeGreaterThanOrEqual(2);
  }
});

test("world atlas defines deterministic cave anchor systems", () => {
  expect(WORLD_ATLAS.caveSystems.map((caveSystem) => caveSystem.id)).toEqual(Array.from(EXPECTED_CAVE_SYSTEM_IDS));

  for (const caveSystem of WORLD_ATLAS.caveSystems) {
    expect(caveSystem.anchors.length).toBeGreaterThanOrEqual(3);
    expect(caveSystem.tunnels.length).toBeGreaterThanOrEqual(2);
    expect(caveSystem.expectedRouteIds.length).toBeGreaterThanOrEqual(1);
    expect(caveSystem.materialProfileId.length).toBeGreaterThan(0);

    const anchorIds = new Set(caveSystem.anchors.map((anchor) => anchor.id));
    for (const tunnel of caveSystem.tunnels) {
      expect(anchorIds.has(tunnel.from)).toBe(true);
      expect(anchorIds.has(tunnel.to)).toBe(true);
      expect(tunnel.widthM).toBeGreaterThan(0);
    }
  }
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

test("region graph exposes stable centers, edges, routes, caves, and meaningful area", () => {
  const graph = getAtlasRegionGraph();

  expect(graph.map((node) => node.regionId)).toEqual(WORLD_ATLAS.regions.map((region) => region.id));
  for (const node of graph) {
    const region = WORLD_ATLAS.regions.find((candidate) => candidate.id === node.regionId)!;

    expect(node.center).toEqual(region.center);
    expect(node.radius).toEqual(region.radius);
    expect(node.approximateEllipseAreaM2).toBeCloseTo(estimateAtlasRegionEllipseAreaM2(region), 5);
    expect(node.approximateEllipseAreaM2).toBeGreaterThan(3_000_000);
    expect(node.edgeIds.length).toBeGreaterThanOrEqual(1);
    expect(node.routeIds.length).toBeGreaterThanOrEqual(1);
  }

  const graphedCaveSystemIds = new Set(graph.flatMap((node) => node.caveSystemIds));
  for (const caveSystem of WORLD_ATLAS.caveSystems) {
    expect(graphedCaveSystemIds.has(caveSystem.id)).toBe(true);
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

test("finite island grid has bounded land and no biome leakage outside low interior", () => {
  const { origin, radius } = WORLD_ATLAS.island;
  const steps = 64;
  let landSamples = 0;
  let outsideSamples = 0;
  let outsideOceanSamples = 0;
  const regionHits = new Map<AtlasRegionId, number>();

  for (let xStep = 0; xStep < steps; xStep += 1) {
    for (let zStep = 0; zStep < steps; zStep += 1) {
      const xM = origin.x - radius.x + (xStep / (steps - 1)) * radius.x * 2;
      const zM = origin.z - radius.z + (zStep / (steps - 1)) * radius.z * 2;
      const sample = sampleWorldAtlasMeters(xM, zM);

      if (sample.islandInterior >= 0.08) {
        landSamples += 1;
        if (sample.primaryRegionId) {
          regionHits.set(sample.primaryRegionId, (regionHits.get(sample.primaryRegionId) ?? 0) + 1);
        }
      } else {
        outsideSamples += 1;
        if (sample.primaryBiomeId === "ocean" || sample.primaryBiomeId === "deep-ocean") {
          outsideOceanSamples += 1;
        }
      }
    }
  }

  expect(landSamples).toBeGreaterThan(steps * steps * 0.55);
  expect(landSamples).toBeLessThan(steps * steps * 0.9);
  expect(outsideOceanSamples).toBe(outsideSamples);
  for (const region of WORLD_ATLAS.regions) {
    expect(regionHits.get(region.id) ?? 0).toBeGreaterThan(40);
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
    expect(sample.routeId).toBeNull();
    expect(sample.routeInfluence).toBe(0);
  }
});

test("route anchor helper returns deterministic route nodes and validation anchors", () => {
  const anchors = getAtlasRouteAnchors();
  const expectedCount = WORLD_ATLAS.routes.reduce((count, route) => count + route.nodes.length + 1, 0);

  expect(anchors).toHaveLength(expectedCount);
  expect(anchors.map((anchor) => anchor.routeId).slice(0, 6)).toEqual([
    "pilgrim-spine-red",
    "pilgrim-spine-red",
    "pilgrim-spine-red",
    "pilgrim-spine-red",
    "pilgrim-spine-red",
    "pilgrim-spine-red",
  ]);

  for (const anchor of anchors) {
    const sample = sampleWorldAtlasMeters(anchor.point.x, anchor.point.z);

    expect(sample.surfaceClass).toBe("land");
    expect(sample.islandInterior).toBeGreaterThan(0.08);
  }
});

test("authored region-edge anchors expose plausible secondary regions", () => {
  for (const edge of WORLD_ATLAS.regionEdges) {
    const sample = sampleWorldAtlasMeters(edge.validationAnchor.x, edge.validationAnchor.z);
    const sampledRegions = new Set([sample.primaryRegionId, sample.secondaryRegionId]);

    expect(sample.regionEdgeId).toBe(edge.id);
    expect(sampledRegions.has(edge.from)).toBe(true);
    expect(sampledRegions.has(edge.to)).toBe(true);
    expect(sample.primaryRegionId).not.toBeNull();
    expect(sample.secondaryRegionId).not.toBeNull();
    expect(sample.primaryRegionId).not.toBe(sample.secondaryRegionId);
    expect(sample.regionBlend).toBeGreaterThan(0.2);
  }
});

test("cave anchors stay on finite island land and sample their owning cave system", () => {
  const anchors = getAtlasCaveAnchors();
  expect(anchors).toHaveLength(WORLD_ATLAS.caveSystems.reduce((count, caveSystem) => count + caveSystem.anchors.length, 0));

  for (const anchor of anchors) {
    const worldSample = sampleWorldAtlasMeters(anchor.point.x, anchor.point.z);
    const caveSample = sampleAtlasCaveAnchorMeters(anchor.point.x, anchor.point.z);
    const sampledRegions = new Set([worldSample.primaryRegionId, worldSample.secondaryRegionId]);

    expect(worldSample.surfaceClass).toBe("land");
    expect(worldSample.islandInterior).toBeGreaterThan(0.08);
    expect(sampledRegions.has(anchor.regionId)).toBe(true);
    expect(caveSample.caveSystemId).toBe(anchor.caveSystemId);
    expect(caveSample.caveAnchorId).toBe(anchor.id);
    expect(caveSample.caveAnchorKind).toBe(anchor.kind);
    expect(caveSample.distanceToCaveAnchorM).toBeCloseTo(0, 8);
    expect(caveSample.caveCore).toBe(1);
    expect(caveSample.caveInfluence).toBe(1);
    expect(caveSample.caveLandmarkMarkerIds).toEqual(anchor.landmarkMarkerIds);
  }
});

test("route validation anchors sample their authored routes", () => {
  for (const route of WORLD_ATLAS.routes) {
    const sample = sampleAtlasRouteMeters(route.validationAnchor.x, route.validationAnchor.z);

    expect(sample.routeId).toBe(route.id);
    expect(sample.routeSegmentKind).toBe(route.segmentKind);
    expect(sample.distanceToRouteM).toBeCloseTo(0, 8);
    expect(sample.routeCore).toBe(1);
    expect(sample.routeInfluence).toBe(1);
    expect(sample.recommendedSetPieceIds).toEqual(route.recommendedSetPieceIds);
  }
});

test("route nodes stay on finite island land and declared regions", () => {
  for (const route of WORLD_ATLAS.routes) {
    for (const node of route.nodes) {
      const sample = sampleWorldAtlasMeters(node.point.x, node.point.z);
      const sampledRegions = new Set([sample.primaryRegionId, sample.secondaryRegionId]);

      expect(sample.surfaceClass).toBe("land");
      expect(sample.islandInterior).toBeGreaterThan(0.08);
      expect(route.expectedRegionIds).toContain(node.regionId);
      expect(sampledRegions.has(node.regionId)).toBe(true);
    }
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
