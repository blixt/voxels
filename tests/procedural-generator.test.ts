import { expect, setDefaultTimeout, test } from "bun:test";

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
  "#BA6",
  "#9B6",
  "#6B7",
  "#7C8",
  "#7A8",
  "#486",
  "#5A8",
  "#7C5",
  "#6A8",
].map((code) => hexColorToMaterial(code)));

setDefaultTimeout(15_000);

const SURFACE_BIOME_IDS = [
  "verdant",
  "savanna",
  "steppe",
  "dunes",
  "badlands",
  "highland",
  "moor",
  "tundra",
  "marsh",
  "firefly",
  "saltflat",
  "fern",
  "fungal",
  "ember",
  "bloom",
  "shardlands",
] as const;

const AUDITED_SURFACE_LANDMARK_IDS = [
  "oak",
  "canopy_tree",
  "birch",
  "redleaf_tree",
  "willow",
  "blossom_tree",
  "fruit_tree",
  "giant_flower",
  "redwood",
  "dead_tree",
  "thorn_tree",
  "berry_bush",
  "giant_fern",
  "lantern_tree",
  "salt_spire",
  "boulder",
  "standing_stone",
  "shrub",
  "flower_patch",
  "palm",
  "acacia",
  "cactus",
  "dead_snag",
  "hoodoo",
  "fir",
  "tall_fir",
  "ice_spire",
  "frost_shrub",
  "cypress",
  "mangrove",
  "reed_cluster",
  "basalt_spire",
  "crystal_cluster",
  "glowcap",
  "mega_glowcap",
  "root_stump",
  "stone_tor",
  "ancestor_pillar",
  "ash_marker",
  "glass_cairn",
  "silt_shell",
  "velothi_shrine",
  "kwama_mound",
  "pilgrim_cairn",
  "velothi_ziggurat",
  "ash_obelisk",
  "rib_arch",
  "old_road_causeway",
  "pilgrim_lantern",
  "crystal_reeds",
  "fungal_bridge",
  "rib_remains",
] as const;

const TRUNKED_LANDMARK_IDS = [
  "oak",
  "canopy_tree",
  "birch",
  "redleaf_tree",
  "willow",
  "blossom_tree",
  "fruit_tree",
  "redwood",
  "dead_tree",
  "thorn_tree",
  "lantern_tree",
  "palm",
  "acacia",
  "fir",
  "tall_fir",
  "cypress",
  "mangrove",
  "giant_fern",
  "giant_flower",
] as const;

type LandmarkRoot = { x: number; z: number; probe: ReturnType<ProceduralWorldGenerator["sampleBiomeProbe"]> };

let landmarkRootCache: Map<string, LandmarkRoot | null> | null = null;

function ensureLandmarkRootCache(generator: ProceduralWorldGenerator): Map<string, LandmarkRoot | null> {
  if (landmarkRootCache) {
    return landmarkRootCache;
  }
  const cache = new Map<string, LandmarkRoot | null>();
  const pending = new Set<string>(AUDITED_SURFACE_LANDMARK_IDS);

  for (let coarseZ = -32768; coarseZ <= 32768 && pending.size > 0; coarseZ += 64) {
    for (let coarseX = -32768; coarseX <= 32768 && pending.size > 0; coarseX += 64) {
      const coarseProbe = generator.sampleBiomeProbe(coarseX, coarseZ);
      const landmarkId = coarseProbe.landmarkId;
      if (!landmarkId || !pending.has(landmarkId)) {
        continue;
      }
      for (let z = coarseZ - 48; z <= coarseZ + 48; z += 1) {
        for (let x = coarseX - 48; x <= coarseX + 48; x += 1) {
          const probe = generator.sampleBiomeProbe(x, z);
          if (probe.landmarkId !== landmarkId) {
            continue;
          }
          const rootMaterial = generator.sampleMaterial(x, probe.surfaceY + 1, z);
          if (rootMaterial === 0 || isProceduralWaterMaterial(rootMaterial)) {
            continue;
          }
          cache.set(landmarkId, { x, z, probe });
          pending.delete(landmarkId);
          z = coarseZ + 49;
          break;
        }
      }
    }
  }

  for (const landmarkId of AUDITED_SURFACE_LANDMARK_IDS) {
    if (!cache.has(landmarkId)) {
      cache.set(landmarkId, null);
    }
  }
  landmarkRootCache = cache;
  return cache;
}

function findRepresentativeLandmarkRoot(
  generator: ProceduralWorldGenerator,
  landmarkId: string,
): LandmarkRoot | null {
  return ensureLandmarkRootCache(generator).get(landmarkId) ?? null;
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

function measureCrossSection(
  generator: ProceduralWorldGenerator,
  x: number,
  z: number,
  y: number,
  radius: number,
): {
  count: number;
  widthX: number;
  widthZ: number;
} {
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
  return {
    count,
    widthX: count === 0 ? 0 : maxX - minX + 1,
    widthZ: count === 0 ? 0 : maxZ - minZ + 1,
  };
}

function measureLandmarkObject(
  generator: ProceduralWorldGenerator,
  root: LandmarkRoot,
  radius: number,
  heightPadding: number,
): {
  solidVoxelCount: number;
  materialVariety: number;
  dominantMaterialShare: number;
  boundsSize: [number, number, number];
} {
  const yMin = root.probe.surfaceY + 1;
  const yMax = root.probe.topY + heightPadding;
  const materialCounts = new Map<number, number>();
  let solidVoxelCount = 0;
  let minX = Infinity;
  let minY = Infinity;
  let minZ = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  let maxZ = -Infinity;

  for (let dz = -radius; dz <= radius; dz += 1) {
    for (let dx = -radius; dx <= radius; dx += 1) {
      const worldX = root.x + dx;
      const worldZ = root.z + dz;
      const localProbe = generator.sampleBiomeProbe(worldX, worldZ);
      if (localProbe.landmarkId !== root.probe.landmarkId) {
        continue;
      }
      for (let y = Math.max(yMin, localProbe.surfaceY + 1); y <= yMax; y += 1) {
        const material = generator.sampleMaterial(worldX, y, worldZ);
        if (material === 0 || isProceduralWaterMaterial(material)) {
          continue;
        }
        solidVoxelCount += 1;
        materialCounts.set(material, (materialCounts.get(material) ?? 0) + 1);
        minX = Math.min(minX, worldX);
        minY = Math.min(minY, y);
        minZ = Math.min(minZ, worldZ);
        maxX = Math.max(maxX, worldX + 1);
        maxY = Math.max(maxY, y + 1);
        maxZ = Math.max(maxZ, worldZ + 1);
      }
    }
  }

  const dominantCount = materialCounts.size === 0 ? 0 : Math.max(...materialCounts.values());
  return {
    solidVoxelCount,
    materialVariety: materialCounts.size,
    dominantMaterialShare: solidVoxelCount === 0 ? 0 : dominantCount / solidVoxelCount,
    boundsSize: solidVoxelCount === 0 ? [0, 0, 0] : [maxX - minX, maxY - minY, maxZ - minZ],
  };
}

function measureContiguousVerticalSupport(
  generator: ProceduralWorldGenerator,
  x: number,
  z: number,
  startY: number,
  endY: number,
): number {
  let support = 0;
  for (let y = startY; y <= endY; y += 1) {
    if (generator.sampleMaterial(x, y, z) === 0) {
      break;
    }
    support += 1;
  }
  return support;
}

function findBestVerticalSupportAnchor(
  generator: ProceduralWorldGenerator,
  root: LandmarkRoot,
  radius = 6,
): {
  x: number;
  z: number;
  supportHeight: number;
  baseWidthX: number;
  baseWidthZ: number;
} {
  let best = {
    x: root.x,
    z: root.z,
    supportHeight: 0,
    baseWidthX: 0,
    baseWidthZ: 0,
  };

  for (let z = root.z - radius; z <= root.z + radius; z += 1) {
    for (let x = root.x - radius; x <= root.x + radius; x += 1) {
      const supportHeight = measureContiguousVerticalSupport(
        generator,
        x,
        z,
        root.probe.surfaceY + 1,
        root.probe.topY,
      );
      if (supportHeight === 0) {
        continue;
      }
      const base = measureCrossSection(generator, x, z, root.probe.surfaceY + 1, 6);
      if (
        supportHeight > best.supportHeight
        || (supportHeight === best.supportHeight && base.count > best.baseWidthX * best.baseWidthZ)
      ) {
        best = {
          x,
          z,
          supportHeight,
          baseWidthX: base.widthX,
          baseWidthZ: base.widthZ,
        };
      }
    }
  }

  return best;
}

function collectBiomePatchStats(
  generator: ProceduralWorldGenerator,
  minCoord: number,
  maxCoord: number,
  step: number,
): Map<string, {
  cells: number;
  decentCells: number;
  maxWidthM: number;
  maxHeightM: number;
  componentCount: number;
}> {
  const width = Math.floor((maxCoord - minCoord) / step) + 1;
  const biomes = Array.from({ length: width * width }, () => "");
  for (let gz = 0; gz < width; gz += 1) {
    const z = minCoord + gz * step;
    for (let gx = 0; gx < width; gx += 1) {
      const x = minCoord + gx * step;
      biomes[gx + gz * width] = generator.sampleBiomeProbe(x, z).biomeId;
    }
  }

  const visited = new Uint8Array(width * width);
  const stats = new Map<string, {
    cells: number;
    decentCells: number;
    maxWidthM: number;
    maxHeightM: number;
    componentCount: number;
  }>();

  for (let gz = 0; gz < width; gz += 1) {
    for (let gx = 0; gx < width; gx += 1) {
      const index = gx + gz * width;
      if (visited[index]) {
        continue;
      }
      visited[index] = 1;
      const biomeId = biomes[index]!;
      const queue = [index];
      let queueIndex = 0;
      let count = 0;
      let minGX = gx;
      let maxGX = gx;
      let minGZ = gz;
      let maxGZ = gz;

      while (queueIndex < queue.length) {
        const current = queue[queueIndex++]!;
        const cx = current % width;
        const cz = (current / width) | 0;
        count += 1;
        minGX = Math.min(minGX, cx);
        maxGX = Math.max(maxGX, cx);
        minGZ = Math.min(minGZ, cz);
        maxGZ = Math.max(maxGZ, cz);

        for (const [nx, nz] of [
          [cx - 1, cz],
          [cx + 1, cz],
          [cx, cz - 1],
          [cx, cz + 1],
        ] as const) {
          if (nx < 0 || nz < 0 || nx >= width || nz >= width) {
            continue;
          }
          const neighborIndex = nx + nz * width;
          if (visited[neighborIndex] || biomes[neighborIndex] !== biomeId) {
            continue;
          }
          visited[neighborIndex] = 1;
          queue.push(neighborIndex);
        }
      }

      const widthM = (maxGX - minGX + 1) * step * 0.1;
      const heightM = (maxGZ - minGZ + 1) * step * 0.1;
      const entry = stats.get(biomeId) ?? {
        cells: 0,
        decentCells: 0,
        maxWidthM: 0,
        maxHeightM: 0,
        componentCount: 0,
      };
      entry.cells += count;
      entry.componentCount += 1;
      entry.maxWidthM = Math.max(entry.maxWidthM, widthM);
      entry.maxHeightM = Math.max(entry.maxHeightM, heightM);
      if (widthM >= 10 && heightM >= 10) {
        entry.decentCells += count;
      }
      stats.set(biomeId, entry);
    }
  }

  return stats;
}

function hasSubsurfaceVoid(
  generator: ProceduralWorldGenerator,
  x: number,
  z: number,
  minDepth = 8,
  maxDepth = 96,
): boolean {
  const column = generator.sampleColumn(x, z);
  for (let depth = minDepth; depth <= maxDepth; depth += 2) {
    const worldY = column.surfaceY - depth;
    if (worldY <= 24) {
      break;
    }
    if (generator.sampleMaterial(x, worldY, z) === 0) {
      return true;
    }
  }
  return false;
}

function hasExposedCaveOpening(
  generator: ProceduralWorldGenerator,
  x: number,
  z: number,
  minDepth = 4,
  maxDepth = 28,
): boolean {
  const column = generator.sampleColumn(x, z);
  if (generator.sampleMaterial(x, column.surfaceY, z) === 0) {
    return true;
  }
  const neighborOffsets = [
    [1, 0],
    [-1, 0],
    [0, 1],
    [0, -1],
    [2, 0],
    [-2, 0],
    [0, 2],
    [0, -2],
  ] as const;

  for (let depth = minDepth; depth <= maxDepth; depth += 2) {
    const worldY = column.surfaceY - depth;
    if (worldY <= 24 || generator.sampleMaterial(x, worldY, z) !== 0) {
      continue;
    }
    for (const [offsetX, offsetZ] of neighborOffsets) {
      const neighborSurfaceY = generator.sampleColumn(x + offsetX, z + offsetZ).surfaceY;
      if (neighborSurfaceY <= worldY + 1) {
        return true;
      }
    }
  }
  return false;
}

function isNearBiomeBoundary(
  generator: ProceduralWorldGenerator,
  x: number,
  z: number,
  step = 32,
): boolean {
  const centerBiome = generator.sampleBiomeProbe(x, z).biomeId;
  for (const [offsetX, offsetZ] of [
    [step, 0],
    [-step, 0],
    [0, step],
    [0, -step],
  ] as const) {
    if (generator.sampleBiomeProbe(x + offsetX, z + offsetZ).biomeId !== centerBiome) {
      return true;
    }
  }
  return false;
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

test("surface column sampling matches the full column fields used by far-field rendering", () => {
  const generator = new ProceduralWorldGenerator(1337);
  for (const [x, z] of [
    [0, 0],
    [512, -384],
    [-1440, 960],
    [4096, 2048],
  ] as const) {
    const full = generator.sampleColumn(x, z);
    const surface = generator.sampleSurfaceColumn(x, z);
    expect(surface.surfaceY).toBe(full.surfaceY);
    expect(surface.waterTopY).toBe(full.waterTopY);
    expect(surface.surfaceMaterial).toBe(full.surfaceMaterial);
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

  expect(biomeIds.size).toBeGreaterThanOrEqual(8);
  expect(biomeIds.has("savanna")).toBe(true);
  expect(biomeIds.has("moor")).toBe(true);
  expect(undergroundIds.size).toBeGreaterThanOrEqual(6);
});

test("special biomes obey their host-biome rules", () => {
  const generator = new ProceduralWorldGenerator(1337);
  let marshCount = 0;
  let fireflyCount = 0;
  let saltflatCount = 0;
  let fernCount = 0;
  let fungalCount = 0;
  let emberCount = 0;
  let bloomCount = 0;
  let shardlandsCount = 0;

  for (let x = -8192; x <= 8192; x += 64) {
    for (let z = -8192; z <= 8192; z += 64) {
      const probe = generator.sampleBiomeProbe(x, z);
      if (probe.biomeId === "marsh") {
        marshCount += 1;
        expect(["verdant", "savanna", "steppe"]).toContain(probe.hostBiomeId);
      }
      if (probe.biomeId === "firefly") {
        fireflyCount += 1;
        expect(["verdant", "savanna", "moor", "tundra"]).toContain(probe.hostBiomeId);
      }
      if (probe.biomeId === "saltflat") {
        saltflatCount += 1;
        expect(["savanna", "steppe", "dunes"]).toContain(probe.hostBiomeId);
      }
      if (probe.biomeId === "fern") {
        fernCount += 1;
        expect(["verdant", "savanna", "highland"]).toContain(probe.hostBiomeId);
      }
      if (probe.biomeId === "fungal") {
        fungalCount += 1;
        expect(["verdant", "highland", "moor"]).toContain(probe.hostBiomeId);
      }
      if (probe.biomeId === "ember") {
        emberCount += 1;
        expect(["badlands", "highland"]).toContain(probe.hostBiomeId);
      }
      if (probe.biomeId === "bloom") {
        bloomCount += 1;
        expect(["verdant", "highland", "moor"]).toContain(probe.hostBiomeId);
      }
      if (probe.biomeId === "shardlands") {
        shardlandsCount += 1;
        expect(probe.fields.moisture).toBeLessThanOrEqual(0.56);
        expect(Math.max(probe.fields.magic, probe.fields.volcanism)).toBeGreaterThanOrEqual(0.48);
      }
    }
  }

  expect(marshCount).toBeGreaterThan(0);
  expect(fireflyCount).toBeGreaterThan(0);
  expect(saltflatCount).toBeGreaterThan(0);
  expect(fernCount).toBeGreaterThan(0);
  expect(fungalCount).toBeGreaterThan(0);
  expect(emberCount).toBeGreaterThan(0);
  expect(bloomCount).toBeGreaterThan(0);
  expect(shardlandsCount).toBeGreaterThan(0);
});

test("surface biomes mostly occupy patches at least 10m by 10m", () => {
  const generator = new ProceduralWorldGenerator(1337);
  const stats = collectBiomePatchStats(generator, -16384, 16384, 64);

  for (const biomeId of SURFACE_BIOME_IDS) {
    const entry = stats.get(biomeId);
    expect(entry).toBeDefined();
    expect(entry!.maxWidthM).toBeGreaterThanOrEqual(10);
    expect(entry!.maxHeightM).toBeGreaterThanOrEqual(10);
    expect(entry!.decentCells / entry!.cells).toBeGreaterThanOrEqual(0.8);
  }
});

test("procedural generator avoids forbidden direct biome adjacencies", () => {
  const generator = new ProceduralWorldGenerator(1337);
  const forbiddenPairs = new Set([
    "dunes|tundra",
    "badlands|marsh",
    "ember|marsh",
    "bloom|dunes",
    "saltflat|tundra",
    "firefly|dunes",
    "fungal|dunes",
    "marsh|shardlands",
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

test("rare shared peak fields create tall mountains without dominating the world", () => {
  const generator = new ProceduralWorldGenerator(1337);
  let maxSurfaceY = -Infinity;
  let over1700 = 0;
  let over1760 = 0;
  let sampleCount = 0;

  for (let z = -8192; z <= 8192; z += 64) {
    for (let x = -8192; x <= 8192; x += 64) {
      const surfaceY = generator.sampleColumn(x, z).surfaceY;
      maxSurfaceY = Math.max(maxSurfaceY, surfaceY);
      if (surfaceY >= 1700) {
        over1700 += 1;
      }
      if (surfaceY >= 1760) {
        over1760 += 1;
      }
      sampleCount += 1;
    }
  }

  expect(maxSurfaceY).toBeGreaterThanOrEqual(1760);
  expect(over1700).toBeGreaterThan(0);
  expect(over1760).toBeGreaterThan(0);
  expect(over1700 / sampleCount).toBeLessThanOrEqual(0.03);
  expect(over1760 / sampleCount).toBeLessThanOrEqual(0.01);
});

test("rare regional extremes appear across biome families without taking over the world", () => {
  const generator = new ProceduralWorldGenerator(1337);
  const expected = {
    verdant: "verdant_karst",
    savanna: "savanna_flowersea",
    steppe: "steppe_monolith",
    dunes: "dunes_glass",
    badlands: "badlands_crater",
    ash: "ash_wastes",
    highland: "highland_redleaf",
    moor: "moor_shadowglass",
    tundra: "tundra_blue_ice",
    firefly: "firefly_lantern",
    saltflat: "saltflat_mirror",
    fern: "fern_cenote",
    fungal: "fungal_moonlit",
    ember: "ember_caldera",
    bloom: "bloom_prism",
  } as const;
  const totalSamples = { count: 0 };
  const counts = new Map<string, number>();

  for (let z = -8192; z <= 8192; z += 64) {
    for (let x = -8192; x <= 8192; x += 64) {
      const probe = generator.sampleBiomeProbe(x, z);
      if (probe.regionalVariantId) {
        counts.set(probe.regionalVariantId, (counts.get(probe.regionalVariantId) ?? 0) + 1);
      }
      totalSamples.count += 1;
    }
  }

  for (const regionalVariantId of Object.values(expected)) {
    const count = counts.get(regionalVariantId) ?? 0;
    expect(count).toBeGreaterThan(0);
    expect(count / totalSamples.count).toBeLessThanOrEqual(0.02);
  }
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

  expect(landmarkIds.size).toBeGreaterThanOrEqual(13);
  expect(landmarkIds.has("oak")).toBe(true);
  expect(landmarkIds.has("canopy_tree")).toBe(true);
  expect(landmarkIds.has("acacia")).toBe(true);
  expect(landmarkIds.has("redleaf_tree")).toBe(true);
  expect(landmarkIds.has("willow")).toBe(true);
  expect(landmarkIds.has("blossom_tree")).toBe(true);
  expect(landmarkIds.has("fruit_tree")).toBe(true);
  expect(landmarkIds.has("giant_flower")).toBe(true);
  expect(landmarkIds.has("giant_fern")).toBe(true);
  expect(landmarkIds.has("lantern_tree")).toBe(true);
  expect(landmarkIds.has("salt_spire")).toBe(true);
  expect(landmarkIds.has("redwood")).toBe(true);
  expect(landmarkIds.has("dead_tree")).toBe(true);
  expect(landmarkIds.has("thorn_tree")).toBe(true);
  expect(landmarkIds.has("root_stump")).toBe(true);
  expect(landmarkIds.has("stone_tor")).toBe(true);
  expect(landmarkIds.has("hoodoo")).toBe(true);
  expect(landmarkIds.has("ice_spire")).toBe(true);
  expect(
    landmarkIds.has("palm")
    || landmarkIds.has("glowcap")
    || landmarkIds.has("basalt_spire")
    || landmarkIds.has("mega_glowcap"),
  ).toBe(true);
});

test("audited surface landmarks all appear in the generated world", () => {
  const generator = new ProceduralWorldGenerator(1337);

  for (const landmarkId of AUDITED_SURFACE_LANDMARK_IDS) {
    expect(findRepresentativeLandmarkRoot(generator, landmarkId)).not.toBeNull();
  }
});

test("landmark scale now regularly exceeds player height", () => {
  const generator = new ProceduralWorldGenerator(1337);
  let tallestFeature = 0;
  let tallFeatureCount = 0;
  let tallestRedwood = 0;

  for (let z = -8192; z <= 8192; z += 16) {
    for (let x = -8192; x <= 8192; x += 16) {
      const probe = generator.sampleBiomeProbe(x, z);
      const featureHeight = probe.topY - probe.surfaceY;
      tallestFeature = Math.max(tallestFeature, featureHeight);
      if (probe.landmarkId === "redwood") {
        tallestRedwood = Math.max(tallestRedwood, featureHeight);
      }
      if (featureHeight >= 24) {
        tallFeatureCount += 1;
      }
    }
  }

  expect(tallestFeature).toBeGreaterThanOrEqual(72);
  expect(tallestRedwood).toBeGreaterThanOrEqual(160);
  expect(tallFeatureCount).toBeGreaterThan(100);
});

test("the world now contains dense forest plus orchard and flower-grove landmark pockets", () => {
  const generator = new ProceduralWorldGenerator(1337);
  const forestLandmarks = new Set(["redwood", "tall_fir", "fir", "canopy_tree", "oak", "willow"]);
  const orchardLandmarks = new Set(["blossom_tree", "fruit_tree", "berry_bush"]);
  const gladeLandmarks = new Set(["giant_flower", "blossom_tree", "glowcap", "flower_patch"]);
  let maxForestRatio = 0;
  let maxOrchardRatio = 0;
  let maxGladeRatio = 0;
  let maxFernJungleRatio = 0;

  for (let centerZ = -8192; centerZ <= 8192; centerZ += 192) {
    for (let centerX = -8192; centerX <= 8192; centerX += 192) {
      let forestCount = 0;
      let orchardCount = 0;
      let gladeCount = 0;
      let fernJungleCount = 0;
      let total = 0;
      for (let dz = -48; dz <= 48; dz += 8) {
        for (let dx = -48; dx <= 48; dx += 8) {
          const probe = generator.sampleBiomeProbe(centerX + dx, centerZ + dz);
          const featureHeight = probe.topY - probe.surfaceY;
          if (forestLandmarks.has(probe.landmarkId ?? "") && featureHeight >= 32) {
            forestCount += 1;
          }
          if (orchardLandmarks.has(probe.landmarkId ?? "") && featureHeight >= 10) {
            orchardCount += 1;
          }
          if (gladeLandmarks.has(probe.landmarkId ?? "") && featureHeight >= 2) {
            gladeCount += 1;
          }
          if ((probe.landmarkId === "giant_fern" || probe.landmarkId === "canopy_tree") && featureHeight >= 18) {
            fernJungleCount += 1;
          }
          total += 1;
        }
      }
      maxForestRatio = Math.max(maxForestRatio, forestCount / total);
      maxOrchardRatio = Math.max(maxOrchardRatio, orchardCount / total);
      maxGladeRatio = Math.max(maxGladeRatio, gladeCount / total);
      maxFernJungleRatio = Math.max(maxFernJungleRatio, fernJungleCount / total);
    }
  }

  expect(maxForestRatio).toBeGreaterThanOrEqual(0.38);
  expect(maxOrchardRatio).toBeGreaterThanOrEqual(0.15);
  expect(maxGladeRatio).toBeGreaterThanOrEqual(0.13);
  expect(maxFernJungleRatio).toBeGreaterThanOrEqual(0.24);
});

test("underground families leak distinct landmark signatures onto the surface", () => {
  const generator = new ProceduralWorldGenerator(1337);
  const landmarksByUnderground = new Map<string, Set<string>>();

  for (let z = -8192; z <= 8192; z += 16) {
    for (let x = -8192; x <= 8192; x += 16) {
      const probe = generator.sampleBiomeProbe(x, z);
      if (!probe.landmarkId) {
        continue;
      }
      const set = landmarksByUnderground.get(probe.undergroundBiomeId) ?? new Set<string>();
      set.add(probe.landmarkId);
      landmarksByUnderground.set(probe.undergroundBiomeId, set);
    }
  }

  expect(landmarksByUnderground.get("rooted")?.has("root_stump")).toBe(true);
  expect(
    (landmarksByUnderground.get("granitic")?.has("stone_tor") ?? false)
    || (landmarksByUnderground.get("granitic")?.has("standing_stone") ?? false),
  ).toBe(true);
  expect(landmarksByUnderground.get("saline")?.has("salt_spire")).toBe(true);
  expect(
    (landmarksByUnderground.get("mycelial")?.has("mega_glowcap") ?? false)
    || (landmarksByUnderground.get("mycelial")?.has("glowcap") ?? false),
  ).toBe(true);
  expect(landmarksByUnderground.get("crystalline")?.has("crystal_cluster")).toBe(true);
  expect(landmarksByUnderground.get("basaltic")?.has("basalt_spire")).toBe(true);
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
  expect(materialsByBiome.get("savanna")?.size ?? 0).toBeGreaterThanOrEqual(4);
  expect(materialsByBiome.get("steppe")?.size ?? 0).toBeGreaterThanOrEqual(4);
  expect(materialsByBiome.get("dunes")?.size ?? 0).toBeGreaterThanOrEqual(3);
  expect(materialsByBiome.get("badlands")?.size ?? 0).toBeGreaterThanOrEqual(4);
  expect(materialsByBiome.get("highland")?.size ?? 0).toBeGreaterThanOrEqual(4);
  expect(materialsByBiome.get("moor")?.size ?? 0).toBeGreaterThanOrEqual(3);
});

test("new biome families expose distinct landmark identities", () => {
  const generator = new ProceduralWorldGenerator(1337);
  const landmarksByBiome = new Map<string, Set<string>>();

  for (let z = -8192; z <= 8192; z += 16) {
    for (let x = -8192; x <= 8192; x += 16) {
      const probe = generator.sampleBiomeProbe(x, z);
      if (!probe.landmarkId) {
        continue;
      }
      const set = landmarksByBiome.get(probe.biomeId) ?? new Set<string>();
      set.add(probe.landmarkId);
      landmarksByBiome.set(probe.biomeId, set);
    }
  }

  expect(landmarksByBiome.get("savanna")?.has("acacia") || landmarksByBiome.get("savanna")?.has("thorn_tree")).toBe(true);
  expect(landmarksByBiome.get("moor")?.has("standing_stone") || landmarksByBiome.get("moor")?.has("dead_tree")).toBe(true);
  expect(landmarksByBiome.get("saltflat")?.has("salt_spire")).toBe(true);
  expect(landmarksByBiome.get("fern")?.has("giant_fern")).toBe(true);
  expect(landmarksByBiome.get("fungal")?.has("mega_glowcap") || landmarksByBiome.get("fungal")?.has("lantern_tree")).toBe(true);
  expect(landmarksByBiome.get("firefly")?.has("lantern_tree")).toBe(true);
  expect(landmarksByBiome.get("shardlands")?.has("salt_spire") || landmarksByBiome.get("shardlands")?.has("crystal_cluster")).toBe(true);
});

test("salt-marsh and fungal regions expose basin set-piece landmarks", () => {
  const generator = new ProceduralWorldGenerator(1337);

  const crystalReeds = findRepresentativeLandmarkRoot(generator, "crystal_reeds");
  const fungalBridge = findRepresentativeLandmarkRoot(generator, "fungal_bridge");
  const ribRemains = findRepresentativeLandmarkRoot(generator, "rib_remains");

  expect(crystalReeds).not.toBeNull();
  expect(fungalBridge).not.toBeNull();
  expect(ribRemains).not.toBeNull();
  expect(new Set(["marsh", "firefly", "fungal"]).has(crystalReeds!.probe.biomeId)).toBe(true);
  expect(new Set(["marsh", "fungal"]).has(fungalBridge!.probe.biomeId)).toBe(true);
  expect(new Set(["marsh", "fungal"]).has(ribRemains!.probe.biomeId)).toBe(true);
});

test("salt-marsh basin set pieces keep readable silhouettes and material variety", () => {
  const generator = new ProceduralWorldGenerator(1337);

  const crystalReeds = findRepresentativeLandmarkRoot(generator, "crystal_reeds");
  const fungalBridge = findRepresentativeLandmarkRoot(generator, "fungal_bridge");

  expect(crystalReeds).not.toBeNull();
  expect(fungalBridge).not.toBeNull();

  const crystalSample = measureLandmarkObject(generator, crystalReeds!, 18, 8);
  const bridgeSample = measureLandmarkObject(generator, fungalBridge!, 24, 8);

  expect(crystalSample.solidVoxelCount).toBeGreaterThanOrEqual(500);
  expect(crystalSample.materialVariety).toBeGreaterThanOrEqual(3);
  expect(crystalSample.dominantMaterialShare).toBeLessThan(0.70);
  expect(crystalSample.boundsSize[1]).toBeGreaterThanOrEqual(24);
  expect(crystalSample.boundsSize[0]).toBeGreaterThanOrEqual(8);
  expect(crystalSample.boundsSize[2]).toBeGreaterThanOrEqual(8);

  expect(bridgeSample.solidVoxelCount).toBeGreaterThanOrEqual(1200);
  expect(bridgeSample.solidVoxelCount).toBeLessThan(3500);
  expect(bridgeSample.materialVariety).toBeGreaterThanOrEqual(3);
  expect(bridgeSample.dominantMaterialShare).toBeLessThan(0.65);
  expect(bridgeSample.boundsSize[0]).toBeGreaterThanOrEqual(28);
  expect(bridgeSample.boundsSize[2]).toBeGreaterThanOrEqual(12);
  expect(bridgeSample.boundsSize[1]).toBeGreaterThanOrEqual(8);
});

test("ancient route landmarks appear in harsh and uncanny regions", () => {
  const generator = new ProceduralWorldGenerator(1337);
  const seen = new Set<string>();

  for (let z = -12288; z <= 12288; z += 24) {
    for (let x = -12288; x <= 12288; x += 24) {
      const landmarkId = generator.sampleBiomeProbe(x, z).landmarkId;
      if (landmarkId) {
        seen.add(landmarkId);
      }
    }
  }

  expect(seen.has("ancestor_pillar")).toBe(true);
  expect(seen.has("ash_marker")).toBe(true);
  expect(seen.has("glass_cairn")).toBe(true);
  expect(seen.has("old_road_causeway")).toBe(true);
  expect(seen.has("pilgrim_lantern")).toBe(true);
});

test("ashland exploration landmarks add Morrowind-like silhouettes", () => {
  const generator = new ProceduralWorldGenerator(1337);
  const seen = new Set<string>();

  for (let z = -24_000; z <= 24_000; z += 32) {
    for (let x = -24_000; x <= 24_000; x += 32) {
      const landmarkId = generator.sampleBiomeProbe(x, z).landmarkId;
      if (landmarkId) {
        seen.add(landmarkId);
      }
    }
  }

  expect(seen.has("silt_shell")).toBe(true);
  expect(seen.has("velothi_shrine")).toBe(true);
  expect(seen.has("kwama_mound")).toBe(true);
  expect(seen.has("pilgrim_cairn")).toBe(true);
  expect(seen.has("velothi_ziggurat")).toBe(true);
  expect(seen.has("ash_obelisk")).toBe(true);
  expect(seen.has("rib_arch")).toBe(true);
  expect(seen.has("old_road_causeway")).toBe(true);
  expect(seen.has("pilgrim_lantern")).toBe(true);
});

test("ash wastes regional pockets favor ancient ashland silhouettes over generic desert props", () => {
  const generator = new ProceduralWorldGenerator(1337);
  const ashlandLandmarks = new Set([
    "ash_marker",
    "velothi_shrine",
    "pilgrim_cairn",
    "kwama_mound",
    "silt_shell",
    "basalt_spire",
    "velothi_ziggurat",
    "ash_obelisk",
    "rib_arch",
    "old_road_causeway",
    "pilgrim_lantern",
  ]);
  const genericDesertLandmarks = new Set(["cactus", "palm", "shrub"]);
  const ashMaterials = new Set(["#655", "#887", "#433", "#544"].map((code) => hexColorToMaterial(code)));
  let ashSamples = 0;
  let ashlandLandmarkSamples = 0;
  let genericDesertSamples = 0;
  let ashSurfaceMaterialSamples = 0;
  const ashMaterialVariety = new Set<number>();
  const ashSurfaceModuloBuckets = new Map<number, number>();

  for (let z = -24_000; z <= 24_000; z += 32) {
    for (let x = -24_000; x <= 24_000; x += 32) {
      const probe = generator.sampleBiomeProbe(x, z);
      if (probe.regionalVariantId !== "ash_wastes") {
        continue;
      }
      ashSamples += 1;
      if (ashlandLandmarks.has(probe.landmarkId ?? "")) {
        ashlandLandmarkSamples += 1;
      }
      if (genericDesertLandmarks.has(probe.landmarkId ?? "")) {
        genericDesertSamples += 1;
      }
      if (ashMaterials.has(probe.surfaceMaterial)) {
        ashSurfaceMaterialSamples += 1;
      }
      ashSurfaceModuloBuckets.set(probe.surfaceY % 8, (ashSurfaceModuloBuckets.get(probe.surfaceY % 8) ?? 0) + 1);
      for (const y of [probe.surfaceY, probe.surfaceY - 1, probe.surfaceY - 2]) {
        const material = generator.sampleMaterial(x, y, z);
        if (ashMaterials.has(material)) {
          ashMaterialVariety.add(material);
        }
      }
    }
  }
  const dominantSurfaceModuloShare = Math.max(...ashSurfaceModuloBuckets.values()) / ashSamples;

  expect(ashSamples).toBeGreaterThan(0);
  expect(ashSamples / ((48_000 / 32 + 1) ** 2)).toBeLessThanOrEqual(0.02);
  expect(ashlandLandmarkSamples).toBeGreaterThan(genericDesertSamples);
  expect(ashSurfaceMaterialSamples / ashSamples).toBeGreaterThanOrEqual(0.55);
  expect(ashMaterialVariety.size).toBeGreaterThanOrEqual(4);
  expect(dominantSurfaceModuloShare).toBeLessThanOrEqual(0.22);
});

test("ashland and old-road landmarks render shaped caps instead of block columns", () => {
  const generator = new ProceduralWorldGenerator(1337);

  for (const landmarkId of ["ash_marker", "velothi_shrine"] as const) {
    const root = findRepresentativeLandmarkRoot(generator, landmarkId);
    expect(root).not.toBeNull();
    expect(generator.sampleMaterial(root!.x, root!.probe.surfaceY + 1, root!.z)).not.toBe(0);

    const featureHeight = root!.probe.topY - root!.probe.surfaceY;
    const shaftY = root!.probe.surfaceY + 1 + Math.floor(featureHeight * 0.45);
    const capY = root!.probe.surfaceY + 1 + Math.floor(featureHeight * 0.88);
    const shaft = measureCrossSection(generator, root!.x, root!.z, shaftY, 10);
    const cap = measureCrossSection(generator, root!.x, root!.z, capY, 10);

    expect(shaft.count).toBeGreaterThan(0);
    expect(cap.count).toBeGreaterThan(shaft.count);
    expect(cap.widthX).toBeGreaterThanOrEqual(shaft.widthX + 3);
    expect(cap.widthX).toBeGreaterThan(cap.widthZ);
  }

  const lantern = findRepresentativeLandmarkRoot(generator, "pilgrim_lantern");
  expect(lantern).not.toBeNull();
  expect(generator.sampleMaterial(lantern!.x, lantern!.probe.surfaceY + 1, lantern!.z)).not.toBe(0);
  const lanternHeight = lantern!.probe.topY - lantern!.probe.surfaceY;
  const lanternBaseHeight = Math.min(4, Math.max(2, Math.round(lanternHeight * 0.12)));
  const lanternCageY = lantern!.probe.surfaceY + 1 + Math.max(lanternBaseHeight + 2, lanternHeight - 13);
  const lanternShaftY = lantern!.probe.surfaceY + 1 + Math.floor(lanternHeight * 0.45);
  const lanternShaft = measureCrossSection(generator, lantern!.x, lantern!.z, lanternShaftY, 10);
  const lanternUpper = measureMaxCrossSection(
    generator,
    lantern!.x,
    lantern!.z,
    lantern!.probe.surfaceY + 1 + lanternBaseHeight,
    lantern!.probe.topY,
    12,
  );
  const lanternCage = measureCrossSection(generator, lantern!.x, lantern!.z, lanternCageY, 10);

  expect(lanternShaft.count).toBeGreaterThan(0);
  expect(lanternUpper.maxCount).toBeGreaterThan(lanternShaft.count);
  expect(lanternUpper.maxWidthX).toBeGreaterThanOrEqual(lanternShaft.widthX + 2);
  expect(lanternCage.count).toBeGreaterThan(0);
  expect(lanternCage.widthX).toBeGreaterThanOrEqual(lanternShaft.widthX);
});

test("ashland megastructures have distinctive large silhouettes", () => {
  const generator = new ProceduralWorldGenerator(1337);

  const ziggurat = findRepresentativeLandmarkRoot(generator, "velothi_ziggurat");
  const obelisk = findRepresentativeLandmarkRoot(generator, "ash_obelisk");
  const ribArch = findRepresentativeLandmarkRoot(generator, "rib_arch");
  const causeway = findRepresentativeLandmarkRoot(generator, "old_road_causeway");

  expect(ziggurat).not.toBeNull();
  expect(obelisk).not.toBeNull();
  expect(ribArch).not.toBeNull();
  expect(causeway).not.toBeNull();

  const zigguratHeight = ziggurat!.probe.topY - ziggurat!.probe.surfaceY;
  const obeliskHeight = obelisk!.probe.topY - obelisk!.probe.surfaceY;
  const zigguratBase = measureCrossSection(generator, ziggurat!.x, ziggurat!.z, ziggurat!.probe.surfaceY + 2, 24);
  const obeliskMid = measureCrossSection(generator, obelisk!.x, obelisk!.z, obelisk!.probe.surfaceY + Math.floor(obeliskHeight * 0.55), 18);
  const ribTop = measureMaxCrossSection(generator, ribArch!.x, ribArch!.z, ribArch!.probe.surfaceY + 4, ribArch!.probe.topY, 18);
  const causewaySlab = measureCrossSection(generator, causeway!.x, causeway!.z, causeway!.probe.surfaceY + 1, 24);

  expect(zigguratHeight).toBeGreaterThanOrEqual(50);
  expect(obeliskHeight).toBeGreaterThanOrEqual(48);
  expect(zigguratBase.widthX).toBeGreaterThanOrEqual(24);
  expect(zigguratBase.widthZ).toBeGreaterThanOrEqual(14);
  expect(obeliskMid.widthX).toBeLessThan(zigguratBase.widthX);
  expect(obeliskMid.widthZ).toBeGreaterThan(0);
  expect(ribTop.maxWidthX).toBeGreaterThanOrEqual(12);
  expect(ribTop.maxCount).toBeGreaterThanOrEqual(12);
  expect(causewaySlab.widthX).toBeGreaterThanOrEqual(18);
  expect(causewaySlab.widthZ).toBeGreaterThanOrEqual(4);
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
    "willow",
    "palm",
    "acacia",
    "thorn_tree",
    "giant_fern",
    "lantern_tree",
    "giant_flower",
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
    { landmarkId: "willow", minWidth: 14, minCount: 120 },
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

test("tree-like landmarks keep a continuous trunk or stem from the ground", () => {
  const generator = new ProceduralWorldGenerator(1337);

  for (const landmarkId of TRUNKED_LANDMARK_IDS) {
    const root = findRepresentativeLandmarkRoot(generator, landmarkId);
    expect(root).not.toBeNull();
    const anchor = findBestVerticalSupportAnchor(generator, root!);
    const minSupportHeight = Math.max(6, Math.min(24, Math.floor((root!.probe.topY - root!.probe.surfaceY) * 0.18)));

    expect(anchor.supportHeight).toBeGreaterThanOrEqual(minSupportHeight);
    expect(Math.max(anchor.baseWidthX, anchor.baseWidthZ)).toBeGreaterThanOrEqual(1);
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

test("procedural generator now creates subterranean voids across the world", () => {
  const generator = new ProceduralWorldGenerator(1337);
  let voidColumns = 0;
  let sampledColumns = 0;

  for (let z = -4096; z <= 4096; z += 48) {
    for (let x = -4096; x <= 4096; x += 48) {
      sampledColumns += 1;
      if (hasSubsurfaceVoid(generator, x, z)) {
        voidColumns += 1;
      }
    }
  }

  expect(voidColumns).toBeGreaterThan(3000);
  expect(voidColumns / sampledColumns).toBeGreaterThanOrEqual(0.08);
});

test("rugged and cave-prone biomes expose more natural cave mouths than flatter biomes", () => {
  const generator = new ProceduralWorldGenerator(1337);
  const ruggedBiomes = new Set(["highland", "badlands", "fern", "ember", "shardlands"]);
  const flatterBiomes = new Set(["savanna", "steppe", "marsh", "saltflat"]);
  let ruggedSamples = 0;
  let ruggedOpenings = 0;
  let flatSamples = 0;
  let flatOpenings = 0;

  for (let z = -6144; z <= 6144; z += 32) {
    for (let x = -6144; x <= 6144; x += 32) {
      const probe = generator.sampleBiomeProbe(x, z);
      if (ruggedBiomes.has(probe.biomeId)) {
        ruggedSamples += 1;
        if (hasExposedCaveOpening(generator, x, z)) {
          ruggedOpenings += 1;
        }
      } else if (flatterBiomes.has(probe.biomeId)) {
        flatSamples += 1;
        if (hasExposedCaveOpening(generator, x, z)) {
          flatOpenings += 1;
        }
      }
    }
  }

  const ruggedRatio = ruggedOpenings / ruggedSamples;
  const flatRatio = flatOpenings / flatSamples;

  expect(ruggedSamples).toBeGreaterThan(2000);
  expect(flatSamples).toBeGreaterThan(2000);
  expect(ruggedRatio).toBeGreaterThanOrEqual(0.04);
  expect(ruggedRatio).toBeGreaterThan(flatRatio * 1.6);
});

test("biome-specific cave mouths are suppressed near direct biome boundaries", () => {
  const generator = new ProceduralWorldGenerator(1337);
  const caveBiomes = new Set(["highland", "badlands", "fern", "ember", "shardlands"]);
  let interiorSamples = 0;
  let interiorOpenings = 0;
  let boundarySamples = 0;
  let boundaryOpenings = 0;

  for (let z = -4096; z <= 4096; z += 32) {
    for (let x = -4096; x <= 4096; x += 32) {
      const probe = generator.sampleBiomeProbe(x, z);
      if (!caveBiomes.has(probe.biomeId)) {
        continue;
      }
      if (isNearBiomeBoundary(generator, x, z, 32)) {
        boundarySamples += 1;
        if (hasExposedCaveOpening(generator, x, z)) {
          boundaryOpenings += 1;
        }
      } else {
        interiorSamples += 1;
        if (hasExposedCaveOpening(generator, x, z)) {
          interiorOpenings += 1;
        }
      }
    }
  }

  expect(interiorSamples).toBeGreaterThan(1000);
  expect(boundarySamples).toBeGreaterThan(200);
  expect(interiorOpenings / interiorSamples).toBeGreaterThan(boundaryOpenings / boundarySamples);
});

test("procedural generator respects the configured Y range", () => {
  const generator = new ProceduralWorldGenerator(7, { maxYExclusive: PROCEDURAL_WORLD_MAX_Y });
  const column = generator.sampleColumn(640, -1280);

  expect(column.surfaceY).toBeGreaterThanOrEqual(8);
  expect(column.surfaceY).toBeLessThan(PROCEDURAL_WORLD_MAX_Y);
  expect(generator.sampleMaterial(640, -1, -1280)).toBe(0);
  expect(generator.sampleMaterial(640, PROCEDURAL_WORLD_MAX_Y, -1280)).toBe(0);
});
