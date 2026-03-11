import { expect, test } from "bun:test";

import { fnv1a } from "../src/engine/math.ts";
import {
  buildHexColorPalette,
  buildProceduralPalette,
  hexColorToMaterial,
  isProceduralWaterMaterial,
  materialToHexColor,
  ProceduralWorldGenerator,
  PROCEDURAL_WORLD_MAX_Y,
} from "../src/engine/procedural-generator.ts";

const UNDERWATER_ORGANIC_SURFACE_MATERIALS = new Set<number>([
  "#6A5",
  "#7B6",
  "#8B6",
  "#9B6",
  "#6B7",
  "#7C8",
  "#7A8",
  "#486",
  "#5A8",
  "#6A8",
].map((code) => hexColorToMaterial(code)));

function findRepresentativeLandmarkRoot(
  generator: ProceduralWorldGenerator,
  landmarkId: string,
): { x: number; z: number; probe: ReturnType<ProceduralWorldGenerator["sampleBiomeProbe"]> } | null {
  for (let coarseZ = -8192; coarseZ <= 8192; coarseZ += 32) {
    for (let coarseX = -8192; coarseX <= 8192; coarseX += 32) {
      const coarseProbe = generator.sampleBiomeProbe(coarseX, coarseZ);
      if (coarseProbe.landmarkId !== landmarkId) {
        continue;
      }
      for (let z = coarseZ - 24; z <= coarseZ + 24; z += 1) {
        for (let x = coarseX - 24; x <= coarseX + 24; x += 1) {
          const probe = generator.sampleBiomeProbe(x, z);
          if (probe.landmarkId !== landmarkId) {
            continue;
          }
          if (generator.sampleMaterial(x, probe.surfaceY + 1, z) !== 0) {
            return { x, z, probe };
          }
        }
      }
    }
  }
  return null;
}

function measureMaxCrossSection(
  generator: ProceduralWorldGenerator,
  x: number,
  z: number,
  yStart: number,
  yEnd: number,
  radius: number,
): {
  maxCount: number;
  maxWidthX: number;
  maxWidthZ: number;
} {
  let maxCount = 0;
  let maxWidthX = 0;
  let maxWidthZ = 0;
  for (let y = yStart; y <= yEnd; y += 1) {
    let minX = Infinity;
    let maxX = -Infinity;
    let minZ = Infinity;
    let maxZ = -Infinity;
    let count = 0;
    for (let dz = -radius; dz <= radius; dz += 1) {
      for (let dx = -radius; dx <= radius; dx += 1) {
        if (generator.sampleMaterial(x + dx, y, z + dz) === 0) {
          continue;
        }
        count += 1;
        minX = Math.min(minX, dx);
        maxX = Math.max(maxX, dx);
        minZ = Math.min(minZ, dz);
        maxZ = Math.max(maxZ, dz);
      }
    }
    if (count > maxCount) {
      maxCount = count;
      maxWidthX = count === 0 ? 0 : maxX - minX + 1;
      maxWidthZ = count === 0 ? 0 : maxZ - minZ + 1;
    }
  }
  return { maxCount, maxWidthX, maxWidthZ };
}

test("hex color palette covers all #RGB materials", () => {
  const palette = buildHexColorPalette();
  const material = hexColorToMaterial("#ABC");

  expect(palette).toHaveLength(4097);
  expect(materialToHexColor(material)).toBe("#ABC");
  expect(palette[material]).toBe(0xffccbbaa);
});

test("procedural water materials are classified and use translucent palette entries", () => {
  const palette = buildProceduralPalette();
  const material = hexColorToMaterial("#49B");

  expect(isProceduralWaterMaterial(material)).toBe(true);
  expect((palette[material]! >>> 24)).toBeLessThan(0xff);
});

test("procedural generator is deterministic per chunk coordinate", () => {
  const generator = new ProceduralWorldGenerator(4242);

  const a = generator.generateChunk(18, 4, -7);
  const b = generator.generateChunk(18, 4, -7);

  expect(a.solidCount).toBe(b.solidCount);
  expect(fnv1a(new Uint8Array(a.data.buffer))).toBe(fnv1a(new Uint8Array(b.data.buffer)));
});

test("generated chunk data matches direct material sampling", () => {
  const generator = new ProceduralWorldGenerator(999, { chunkSize: 16 });
  const chunk = generator.generateChunk(3, 2, -5);
  const chunkArea = generator.chunkSize * generator.chunkSize;
  const originX = chunk.coord.x * generator.chunkSize;
  const originY = chunk.coord.y * generator.chunkSize;
  const originZ = chunk.coord.z * generator.chunkSize;

  for (const [lx, ly, lz] of [[0, 0, 0], [4, 7, 3], [8, 12, 15], [15, 15, 15]] as const) {
    const worldX = originX + lx;
    const worldY = originY + ly;
    const worldZ = originZ + lz;
    const chunkValue = chunk.data[lx + ly * generator.chunkSize + lz * chunkArea];
    expect(chunkValue).toBe(generator.sampleMaterial(worldX, worldY, worldZ));
  }

  if (chunk.solidCount > 0) {
    expect(chunk.solidBounds).not.toBeNull();
    expect(chunk.solidBounds!.min[0]).toBeGreaterThanOrEqual(0);
    expect(chunk.solidBounds!.min[1]).toBeGreaterThanOrEqual(0);
    expect(chunk.solidBounds!.min[2]).toBeGreaterThanOrEqual(0);
    expect(chunk.solidBounds!.max[0]).toBeLessThanOrEqual(generator.chunkSize);
    expect(chunk.solidBounds!.max[1]).toBeLessThanOrEqual(generator.chunkSize);
    expect(chunk.solidBounds!.max[2]).toBeLessThanOrEqual(generator.chunkSize);
  }
});

test("procedural biome probe is deterministic for surface, fields, and landmarks", () => {
  const generator = new ProceduralWorldGenerator(1337);
  const a = generator.sampleBiomeProbe(1440, -960);
  const b = generator.sampleBiomeProbe(1440, -960);

  expect(a).toEqual(b);
});

test("procedural generator produces a broad biome roster across distant coordinates", () => {
  const generator = new ProceduralWorldGenerator(1337);
  const biomeIds = new Set<string>();
  const undergroundIds = new Set<string>();
  for (let x = -8192; x <= 8192; x += 256) {
    for (let z = -8192; z <= 8192; z += 256) {
      const probe = generator.sampleBiomeProbe(x, z);
      biomeIds.add(probe.biomeId);
      undergroundIds.add(probe.undergroundBiomeId);
    }
  }

  expect(biomeIds.size).toBeGreaterThanOrEqual(6);
  expect(undergroundIds.size).toBeGreaterThanOrEqual(4);
});

test("special biomes obey their host-biome rules", () => {
  const generator = new ProceduralWorldGenerator(1337);
  let marshCount = 0;
  let emberCount = 0;
  let bloomCount = 0;

  for (let x = -8192; x <= 8192; x += 64) {
    for (let z = -8192; z <= 8192; z += 64) {
      const probe = generator.sampleBiomeProbe(x, z);
      if (probe.biomeId === "marsh") {
        marshCount += 1;
        expect(["verdant", "steppe"]).toContain(probe.hostBiomeId);
      }
      if (probe.biomeId === "ember") {
        emberCount += 1;
        expect(["badlands", "highland"]).toContain(probe.hostBiomeId);
      }
      if (probe.biomeId === "bloom") {
        bloomCount += 1;
        expect(["verdant", "highland"]).toContain(probe.hostBiomeId);
      }
    }
  }

  expect(marshCount).toBeGreaterThan(0);
  expect(emberCount).toBeGreaterThan(0);
  expect(bloomCount).toBeGreaterThan(0);
});

test("procedural generator avoids forbidden direct biome adjacencies", () => {
  const generator = new ProceduralWorldGenerator(1337);
  const forbiddenPairs = new Set([
    "dunes|tundra",
    "badlands|marsh",
    "ember|marsh",
    "bloom|dunes",
  ]);

  for (let x = -4096; x <= 4096; x += 64) {
    for (let z = -4096; z <= 4096; z += 64) {
      const center = generator.sampleBiomeProbe(x, z);
      const right = generator.sampleBiomeProbe(x + 64, z);
      const forward = generator.sampleBiomeProbe(x, z + 64);
      const rightPair = [center.biomeId, right.biomeId].sort().join("|");
      const forwardPair = [center.biomeId, forward.biomeId].sort().join("|");
      expect(forbiddenPairs.has(rightPair)).toBe(false);
      expect(forbiddenPairs.has(forwardPair)).toBe(false);
    }
  }
});

test("procedural generator keeps the terrain envelope near sea level with visible lowlands", () => {
  const generator = new ProceduralWorldGenerator(1337);
  let minSurfaceY = Infinity;
  let maxSurfaceY = -Infinity;
  let totalSurfaceY = 0;
  let underwaterCount = 0;
  let sampleCount = 0;
  let maxAdjacentStep = 0;

  for (let z = -4096; z <= 4096; z += 256) {
    for (let x = -4096; x <= 4096; x += 256) {
      const center = generator.sampleColumn(x, z);
      const right = generator.sampleColumn(x + 1, z);
      const forward = generator.sampleColumn(x, z + 1);
      minSurfaceY = Math.min(minSurfaceY, center.surfaceY);
      maxSurfaceY = Math.max(maxSurfaceY, center.surfaceY);
      totalSurfaceY += center.surfaceY;
      sampleCount += 1;
      if ((center.waterTopY ?? center.surfaceY) > center.surfaceY) {
        underwaterCount += 1;
      }
      maxAdjacentStep = Math.max(
        maxAdjacentStep,
        Math.abs(right.surfaceY - center.surfaceY),
        Math.abs(forward.surfaceY - center.surfaceY),
      );
    }
  }

  const averageSurfaceY = totalSurfaceY / sampleCount;
  const underwaterRatio = underwaterCount / sampleCount;

  expect(minSurfaceY).toBeGreaterThanOrEqual(generator.seaLevel - 240);
  expect(maxSurfaceY).toBeLessThanOrEqual(generator.seaLevel + 420);
  expect(maxSurfaceY).toBeGreaterThanOrEqual(generator.seaLevel + 220);
  expect(averageSurfaceY).toBeGreaterThanOrEqual(generator.seaLevel - 40);
  expect(averageSurfaceY).toBeLessThanOrEqual(generator.seaLevel + 220);
  expect(underwaterRatio).toBeGreaterThanOrEqual(0.04);
  expect(underwaterRatio).toBeLessThanOrEqual(0.38);
  expect(maxAdjacentStep).toBeLessThanOrEqual(72);
});

test("soft biome edges stay within a walkable transition budget", () => {
  const generator = new ProceduralWorldGenerator(1337);
  const softPairs = new Set([
    "highland|tundra",
    "steppe|verdant",
    "dunes|steppe",
    "highland|verdant",
    "marsh|steppe",
    "marsh|verdant",
    "bloom|highland",
    "bloom|verdant",
  ]);
  let maxSoftBoundaryJump = 0;
  let sawSoftBoundary = false;

  for (let z = -4096; z <= 4096; z += 64) {
    for (let x = -4096; x <= 4096; x += 64) {
      const center = generator.sampleBiomeProbe(x, z);
      const right = generator.sampleBiomeProbe(x + 64, z);
      const forward = generator.sampleBiomeProbe(x, z + 64);
      const rightPair = [center.biomeId, right.biomeId].sort().join("|");
      const forwardPair = [center.biomeId, forward.biomeId].sort().join("|");
      if (center.biomeId !== right.biomeId && softPairs.has(rightPair)) {
        sawSoftBoundary = true;
        maxSoftBoundaryJump = Math.max(maxSoftBoundaryJump, Math.abs(right.surfaceY - center.surfaceY));
      }
      if (center.biomeId !== forward.biomeId && softPairs.has(forwardPair)) {
        sawSoftBoundary = true;
        maxSoftBoundaryJump = Math.max(maxSoftBoundaryJump, Math.abs(forward.surfaceY - center.surfaceY));
      }
    }
  }

  expect(sawSoftBoundary).toBe(true);
  expect(maxSoftBoundaryJump).toBeLessThanOrEqual(56);
});

test("landmarks appear across the world with multiple distinct families", () => {
  const generator = new ProceduralWorldGenerator(1337);
  const landmarkIds = new Set<string>();

  for (let z = -6144; z <= 6144; z += 16) {
    for (let x = -6144; x <= 6144; x += 16) {
      const landmarkId = generator.sampleBiomeProbe(x, z).landmarkId;
      if (landmarkId) {
        landmarkIds.add(landmarkId);
      }
    }
  }

  expect(landmarkIds.size).toBeGreaterThanOrEqual(10);
  expect(landmarkIds.has("oak")).toBe(true);
  expect(landmarkIds.has("canopy_tree")).toBe(true);
  expect(landmarkIds.has("acacia")).toBe(true);
  expect(landmarkIds.has("hoodoo")).toBe(true);
  expect(landmarkIds.has("ice_spire")).toBe(true);
  expect(
    landmarkIds.has("palm")
    || landmarkIds.has("glowcap")
    || landmarkIds.has("basalt_spire")
    || landmarkIds.has("mega_glowcap"),
  ).toBe(true);
});

test("landmark scale now regularly exceeds player height", () => {
  const generator = new ProceduralWorldGenerator(1337);
  let tallestFeature = 0;
  let tallFeatureCount = 0;

  for (let z = -8192; z <= 8192; z += 16) {
    for (let x = -8192; x <= 8192; x += 16) {
      const probe = generator.sampleBiomeProbe(x, z);
      const featureHeight = probe.topY - probe.surfaceY;
      tallestFeature = Math.max(tallestFeature, featureHeight);
      if (featureHeight >= 24) {
        tallFeatureCount += 1;
      }
    }
  }

  expect(tallestFeature).toBeGreaterThanOrEqual(72);
  expect(tallFeatureCount).toBeGreaterThan(100);
});

test("surface materials vary within major biomes to support finer ground detail", () => {
  const generator = new ProceduralWorldGenerator(1337);
  const materialsByBiome = new Map<string, Set<number>>();

  for (let z = -8192; z <= 8192; z += 32) {
    for (let x = -8192; x <= 8192; x += 32) {
      const probe = generator.sampleBiomeProbe(x, z);
      if (probe.biomeId === "marsh" || probe.biomeId === "ember" || probe.biomeId === "bloom") {
        continue;
      }
      const materials = materialsByBiome.get(probe.biomeId) ?? new Set<number>();
      materials.add(probe.surfaceMaterial);
      materialsByBiome.set(probe.biomeId, materials);
    }
  }

  expect(materialsByBiome.get("verdant")?.size ?? 0).toBeGreaterThanOrEqual(4);
  expect(materialsByBiome.get("steppe")?.size ?? 0).toBeGreaterThanOrEqual(4);
  expect(materialsByBiome.get("dunes")?.size ?? 0).toBeGreaterThanOrEqual(3);
  expect(materialsByBiome.get("badlands")?.size ?? 0).toBeGreaterThanOrEqual(4);
  expect(materialsByBiome.get("highland")?.size ?? 0).toBeGreaterThanOrEqual(4);
});

test("underwater columns no longer expose grassy surface materials", () => {
  const generator = new ProceduralWorldGenerator(1337);
  let underwaterColumns = 0;
  let underwaterOrganicSurfaceColumns = 0;

  for (let z = -8192; z <= 8192; z += 16) {
    for (let x = -8192; x <= 8192; x += 16) {
      const probe = generator.sampleBiomeProbe(x, z);
      if ((probe.waterTopY ?? probe.surfaceY) <= probe.surfaceY) {
        continue;
      }
      underwaterColumns += 1;
      if (UNDERWATER_ORGANIC_SURFACE_MATERIALS.has(probe.surfaceMaterial)) {
        underwaterOrganicSurfaceColumns += 1;
      }
    }
  }

  expect(underwaterColumns).toBeGreaterThan(1000);
  expect(underwaterOrganicSurfaceColumns).toBe(0);
});

test("vegetation landmarks do not root inside standing water", () => {
  const generator = new ProceduralWorldGenerator(1337);
  const vegetationLandmarks = new Set([
    "oak",
    "canopy_tree",
    "birch",
    "palm",
    "acacia",
    "cactus",
    "dead_snag",
    "fir",
    "tall_fir",
    "frost_shrub",
    "cypress",
    "mangrove",
    "glowcap",
    "mega_glowcap",
    "shrub",
    "flower_patch",
  ]);
  let underwaterColumns = 0;
  let rootedInWater = 0;

  for (let z = -4096; z <= 4096; z += 16) {
    for (let x = -4096; x <= 4096; x += 16) {
      const probe = generator.sampleBiomeProbe(x, z);
      if ((probe.waterTopY ?? probe.surfaceY) <= probe.surfaceY) {
        continue;
      }
      underwaterColumns += 1;
      if (!probe.landmarkId || !vegetationLandmarks.has(probe.landmarkId)) {
        continue;
      }
      for (let y = probe.surfaceY + 1; y <= Math.min(probe.surfaceY + 3, probe.topY); y += 1) {
        if (generator.sampleMaterial(x, y, z) !== 0) {
          rootedInWater += 1;
          break;
        }
      }
    }
  }

  expect(underwaterColumns).toBeGreaterThan(500);
  expect(rootedInWater).toBe(0);
});

test("representative trees keep broad crowns instead of collapsing into poles", () => {
  const generator = new ProceduralWorldGenerator(1337);
  const shapeExpectations = [
    { landmarkId: "oak", minWidth: 12, minCount: 80 },
    { landmarkId: "fir", minWidth: 10, minCount: 80 },
    { landmarkId: "tall_fir", minWidth: 12, minCount: 120 },
    { landmarkId: "palm", minWidth: 10, minCount: 80 },
  ] as const;

  for (const expectation of shapeExpectations) {
    const root = findRepresentativeLandmarkRoot(generator, expectation.landmarkId);
    expect(root).not.toBeNull();
    const crownStart = root!.probe.surfaceY + Math.max(3, Math.floor((root!.probe.topY - root!.probe.surfaceY) * 0.65));
    const crown = measureMaxCrossSection(generator, root!.x, root!.z, crownStart, root!.probe.topY, 12);
    expect(Math.max(crown.maxWidthX, crown.maxWidthZ)).toBeGreaterThanOrEqual(expectation.minWidth);
    expect(crown.maxCount).toBeGreaterThanOrEqual(expectation.minCount);
  }
});

test("representative boulders stay rounded instead of flaring at the cap", () => {
  const generator = new ProceduralWorldGenerator(1337);
  const root = findRepresentativeLandmarkRoot(generator, "boulder");

  expect(root).not.toBeNull();
  const body = measureMaxCrossSection(generator, root!.x, root!.z, root!.probe.surfaceY + 1, root!.probe.topY - 1, 10);
  const cap = measureMaxCrossSection(generator, root!.x, root!.z, root!.probe.topY, root!.probe.topY, 10);
  expect(cap.maxCount).toBeGreaterThan(0);
  expect(cap.maxWidthX).toBeLessThanOrEqual(body.maxWidthX);
  expect(cap.maxWidthZ).toBeLessThanOrEqual(body.maxWidthZ);
});

test("below-ground material identity varies across underground biome families", () => {
  const generator = new ProceduralWorldGenerator(1337);
  const deepMaterials = new Set<number>();

  for (let x = -4096; x <= 4096; x += 512) {
    for (let z = -4096; z <= 4096; z += 512) {
      const column = generator.sampleBiomeProbe(x, z);
      deepMaterials.add(generator.sampleMaterial(x, Math.max(24, column.surfaceY - 28), z));
    }
  }

  expect(deepMaterials.size).toBeGreaterThanOrEqual(4);
});

test("procedural generator respects the configured Y range", () => {
  const generator = new ProceduralWorldGenerator(7, { maxYExclusive: PROCEDURAL_WORLD_MAX_Y });
  const column = generator.sampleColumn(640, -1280);

  expect(column.surfaceY).toBeGreaterThanOrEqual(8);
  expect(column.surfaceY).toBeLessThan(PROCEDURAL_WORLD_MAX_Y);
  expect(generator.sampleMaterial(640, -1, -1280)).toBe(0);
  expect(generator.sampleMaterial(640, PROCEDURAL_WORLD_MAX_Y, -1280)).toBe(0);
});
