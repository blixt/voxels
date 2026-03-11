import { expect, test } from "bun:test";

import { fnv1a } from "../src/engine/math.ts";
import { ProceduralFarField } from "../src/engine/procedural-far-field.ts";
import { ProceduralWorldGenerator } from "../src/engine/procedural-generator.ts";
import { metersToWorldUnits } from "../src/engine/scale.ts";
import type { ChunkMeshData } from "../src/engine/types.ts";

test("procedural far field covers a few hundred meters with five render bands", () => {
  const farField = new ProceduralFarField(new ProceduralWorldGenerator(1337));

  const summary = farField.updateAround([0, 0, 0]);

  expect(summary.changed).toBe(true);
  expect(summary.meshCount).toBe(5);
  expect(summary.builtBands).toBe(5);
  expect(summary.sampledCellCount).toBeGreaterThan(0);
  expect(summary.maxRadiusMeters).toBeGreaterThanOrEqual(300);
  expect(summary.triangleCount).toBeGreaterThan(0);
  expect(farField.getRenderables().every((band) => band.mesh !== null)).toBe(true);
});

test("procedural far field reuses meshes while staying inside the current anchor window", () => {
  const farField = new ProceduralFarField(new ProceduralWorldGenerator(1337));
  farField.updateAround([0, 0, 0]);

  const summary = farField.updateAround([3, 0, 3]);

  expect(summary.changed).toBe(false);
  expect(summary.builtBands).toBe(0);
  expect(summary.pendingBands).toBe(0);
  expect(summary.sampledCellCount).toBe(0);
});

test("procedural far field default bands stay stable across a few meters of movement", () => {
  const farField = new ProceduralFarField(new ProceduralWorldGenerator(1337));
  farField.updateAround([0, 0, 0]);

  const summary = farField.updateAround([24, 0, 0]);

  expect(summary.changed).toBe(false);
  expect(summary.builtBands).toBe(0);
  expect(summary.pendingBands).toBe(0);
});

test("procedural far field rebuilds when crossing the nearest anchor stride", () => {
  const farField = new ProceduralFarField(new ProceduralWorldGenerator(1337));
  farField.updateAround([0, 0, 0]);

  const summary = farField.updateAround([320, 0, 0]);

  expect(summary.changed).toBe(true);
  expect(summary.builtBands).toBeGreaterThanOrEqual(1);
});

test("procedural far field rebuilds the near band when the clear radius grows in place", () => {
  const farField = new ProceduralFarField(new ProceduralWorldGenerator(1337));
  farField.updateAround([0, 0, 0]);
  const midBand = farField.getRenderables()[0]!;
  const initialTriangleCount = midBand.triangleCount;

  const summary = farField.updateAround([0, 0, 0], metersToWorldUnits(40));

  expect(summary.changed).toBe(true);
  expect(summary.builtBands).toBe(1);
  expect(summary.clearRadiusMeters).toBe(40);
  expect(midBand.triangleCount).toBeLessThan(initialTriangleCount);
});

test("procedural far field clears out the near resident radius to avoid overlap", () => {
  const generator = new ProceduralWorldGenerator(1337);
  const baseline = new ProceduralFarField(generator, [
    { label: "near", innerRadius: 0, outerRadius: 96, sampleStride: 8, anchorStride: 128 },
  ]);
  const cleared = new ProceduralFarField(generator, [
    { label: "near", innerRadius: 0, outerRadius: 96, sampleStride: 8, anchorStride: 128 },
  ]);

  const baselineSummary = baseline.updateAround([0, 0, 0]);
  const clearedSummary = cleared.updateAround([0, 0, 0], 48);

  expect(baselineSummary.triangleCount).toBeGreaterThan(0);
  expect(clearedSummary.clearRadiusWorldUnits).toBe(48);
  expect(clearedSummary.clearRadiusMeters).toBeCloseTo(4.8, 5);
  expect(clearedSummary.triangleCount).toBeLessThan(baselineSummary.triangleCount);
});

test("procedural far field keeps the inner hole centered on the player inside one anchor window", () => {
  const farField = new ProceduralFarField(createTestGenerator(() => 12), [
    { label: "near", innerRadius: 64, outerRadius: 192, sampleStride: 16, anchorStride: 256 },
  ]);

  farField.updateAround([120, 0, 0]);

  expect(farField.classifyCoverageAt(120, 0)).toBeNull();
  expect(farField.classifyCoverageAt(232, 0)).not.toBeNull();
});

test("procedural far field default bands are ordered without overlap", () => {
  const farField = new ProceduralFarField(new ProceduralWorldGenerator(1337));
  const bands = farField.getRenderables();

  for (let index = 1; index < bands.length; index += 1) {
    expect(bands[index]!.innerRadius).toBeGreaterThanOrEqual(bands[index - 1]!.outerRadius);
  }
});

test("procedural far field emits vertical faces in all four directions", () => {
  const farField = new ProceduralFarField(
    createTestGenerator((worldX, worldZ) => (worldX >= 0 && worldX < 8 && worldZ >= 0 && worldZ < 8 ? 8 : 1)),
    [
      { label: "test", innerRadius: 0, outerRadius: 24, sampleStride: 8, anchorStride: 32 },
    ],
  );

  farField.updateAround([0, 0, 0]);

  const normals = extractNormalSet(farField.getRenderables()[0]!.mesh);
  expect(normals).toContain("1,0,0");
  expect(normals).toContain("-1,0,0");
  expect(normals).toContain("0,0,1");
  expect(normals).toContain("0,0,-1");
});

test("procedural far field exclusion masks remove overlapping top cells", () => {
  const generator = createTestGenerator(() => 4);
  const farField = new ProceduralFarField(generator, [
    { label: "test", innerRadius: 0, outerRadius: 16, sampleStride: 8, anchorStride: 32 },
  ]);
  const exclusionMask = {
    revision: 1,
    excludesCell: (minX: number, _maxXExclusive: number, minZ: number) => minX === 0 && minZ === 0,
  };

  farField.updateAround([0, 0, 0]);
  const baselineMesh = farField.getRenderables()[0]!.mesh;
  const baselineCoverage = farField.classifyCoverageAt(4, 4);
  const baselineTopCells = extractTopCellKeys(baselineMesh);
  farField.updateAround([0, 0, 0], 0, exclusionMask);
  const maskedMesh = farField.getRenderables()[0]!.mesh;
  const maskedCoverage = farField.classifyCoverageAt(4, 4, exclusionMask);
  const maskedTopCells = extractTopCellKeys(maskedMesh);

  expect(baselineCoverage).not.toBeNull();
  expect(maskedCoverage).toBeNull();
  expect(baselineTopCells).toContain("0:0");
  expect(maskedTopCells).not.toContain("0:0");
});

test("procedural far field reuses sampled terrain when only the exclusion mask changes", () => {
  const generator = createTestGenerator(() => 4);
  const farField = new ProceduralFarField(generator, [
    { label: "test", innerRadius: 0, outerRadius: 16, sampleStride: 8, anchorStride: 32 },
  ]);
  const firstMask = {
    revision: 1,
    excludesCell: (_minX: number, _maxXExclusive: number, _minZ: number, _maxZExclusive: number) => false,
  };
  const secondMask = {
    revision: 2,
    excludesCell: (minX: number, _maxXExclusive: number, minZ: number) => minX === 0 && minZ === 0,
  };

  const initial = farField.updateAround([0, 0, 0], 0, firstMask);
  const updated = farField.updateAround([0, 0, 0], 0, secondMask);

  expect(initial.sampledCellCount).toBeGreaterThan(0);
  expect(updated.changed).toBe(true);
  expect(updated.builtBands).toBe(1);
  expect(updated.pendingBands).toBe(0);
  expect(updated.sampledCellCount).toBe(0);
});

test("procedural far field reuses most sampled cells when shifting one anchor window", () => {
  const config = [
    { label: "test", innerRadius: 0, outerRadius: 24, sampleStride: 8, anchorStride: 32, centerStride: 32 },
  ] as const;
  const generator = new ProceduralWorldGenerator(1337);
  const reused = new ProceduralFarField(generator, config);
  const fresh = new ProceduralFarField(generator, config);

  const initial = reused.updateAround([0, 0, 0]);
  const shifted = reused.updateAround([32, 0, 0]);
  const direct = fresh.updateAround([32, 0, 0]);

  expect(initial.sampledCellCount).toBeGreaterThan(0);
  expect(shifted.sampledCellCount).toBeGreaterThan(0);
  expect(shifted.sampledCellCount).toBeLessThan(direct.sampledCellCount);
  expect(meshChecksum(reused.getRenderables()[0]!.mesh)).toBe(meshChecksum(fresh.getRenderables()[0]!.mesh));
});

test("procedural far field can defer excess band rebuilds behind a per-frame budget", () => {
  const farField = new ProceduralFarField(new ProceduralWorldGenerator(1337), [
    { label: "near", innerRadius: 0, outerRadius: 64, sampleStride: 8, anchorStride: 32, centerStride: 32 },
    { label: "mid", innerRadius: 64, outerRadius: 128, sampleStride: 16, anchorStride: 64, centerStride: 64 },
  ]);

  farField.updateAround([0, 0, 0]);
  const constrained = farField.updateAround([80, 0, 0], 0, null, 1);

  expect(constrained.changed).toBe(true);
  expect(constrained.builtBands).toBe(1);
  expect(constrained.pendingBands).toBe(1);

  const settled = farField.updateAround([80, 0, 0], 0, null, 1);
  expect(settled.builtBands).toBe(1);
  expect(settled.pendingBands).toBe(0);
});

test("procedural far field keeps a seam wall against lower masked neighbors", () => {
  const farField = new ProceduralFarField(
    createTestGenerator((worldX, worldZ) => (worldX < 8 && worldZ < 8 ? 8 : 1)),
    [
      { label: "test", innerRadius: 0, outerRadius: 24, sampleStride: 8, anchorStride: 32 },
    ],
  );
  const exclusionMask = {
    revision: 1,
    excludesCell: (minX: number, _maxXExclusive: number, minZ: number) => minX === 8 && minZ === 0,
  };

  farField.updateAround([0, 0, 0], 0, exclusionMask);

  const quads = extractQuads(farField.getRenderables()[0]!.mesh);
  expect(quads.some((quad) =>
    quad.normal.join(",") === "1,0,0"
    && quad.min[0] === 8
    && quad.min[1] <= 0
    && quad.min[2] === 0
    && quad.max[1] === 9
    && quad.max[2] === 8)).toBe(true);
});

test("procedural far field adds a downward skirt against flat masked seams", () => {
  const farField = new ProceduralFarField(createTestGenerator(() => 8), [
    { label: "test", innerRadius: 0, outerRadius: 24, sampleStride: 8, anchorStride: 32 },
  ]);
  const exclusionMask = {
    revision: 1,
    excludesCell: (minX: number, _maxXExclusive: number, minZ: number) => minX === 8 && minZ === 0,
  };

  farField.updateAround([0, 0, 0], 0, exclusionMask);

  const quads = extractQuads(farField.getRenderables()[0]!.mesh);
  expect(quads.some((quad) =>
    quad.normal.join(",") === "1,0,0"
    && quad.min[0] === 8
    && quad.min[1] < 9
    && quad.max[1] === 9
    && quad.min[2] === 0
    && quad.max[2] === 8)).toBe(true);
});

test("procedural far field seam probe reports no masked-edge terrain gaps on a flat seam", () => {
  const farField = new ProceduralFarField(createTestGenerator(() => 8), [
    { label: "test", innerRadius: 0, outerRadius: 24, sampleStride: 8, anchorStride: 32 },
  ]);
  const exclusionMask = {
    revision: 1,
    excludesCell: (minX: number, _maxXExclusive: number, minZ: number) => minX === 8 && minZ === 0,
  };

  farField.updateAround([0, 0, 0], 0, exclusionMask);
  const seamProbe = farField.probeMaskedSeamGaps(exclusionMask);

  expect(seamProbe.boundaryCount).toBeGreaterThan(0);
  expect(seamProbe.gapCount).toBe(0);
  expect(seamProbe.maxGapDepthWorldUnits).toBe(0);
});

test("procedural far field can preserve water tops inside coarse cells", () => {
  const terrainColor = 0xff_aa_88_ff;
  const waterColor = 0xff_44_aa_ff;
  const farField = new ProceduralFarField({
    palette: [0, terrainColor, waterColor],
    sampleColumn(worldX: number, worldZ: number) {
      const hasWater = worldX >= 2 && worldX <= 6 && worldZ >= 2 && worldZ <= 6;
      return {
        biomeId: "verdant",
        surfaceY: hasWater ? 2 : 6,
        waterTopY: hasWater ? 6 : null,
        surfaceMaterial: 1,
      };
    },
    sampleMaterial(_worldX: number, worldY: number) {
      return worldY >= 6 ? 2 : 1;
    },
  } as unknown as ProceduralWorldGenerator, [
    { label: "test", innerRadius: 0, outerRadius: 24, sampleStride: 8, anchorStride: 32 },
  ]);

  farField.updateAround([0, 0, 0]);

  const topColors = extractQuads(farField.getRenderables()[0]!.mesh)
    .filter((quad) => quad.normal.join(",") === "0,1,0")
    .map((quad) => quad.color);
  expect(topColors).toContain(terrainColor);
  expect(topColors).toContain(waterColor);
});

function createTestGenerator(
  surfaceYForWorldPosition: (worldX: number, worldZ: number) => number,
): ProceduralWorldGenerator {
  return {
    palette: [0, 0xff_aa_88_ff],
    sampleColumn(worldX: number, worldZ: number) {
      return {
        biomeId: "verdant",
        surfaceY: surfaceYForWorldPosition(worldX, worldZ),
        waterTopY: null,
        surfaceMaterial: 1,
      };
    },
  } as unknown as ProceduralWorldGenerator;
}

function extractNormalSet(mesh: ChunkMeshData | null): Set<string> {
  if (!mesh) {
    return new Set();
  }
  const normals = new Set<string>();
  const view = new DataView(mesh.vertexData);
  for (let vertexIndex = 0; vertexIndex < mesh.vertexCount; vertexIndex += 1) {
    normals.add([
      Math.sign(view.getInt8(vertexIndex * 20 + 12)),
      Math.sign(view.getInt8(vertexIndex * 20 + 13)),
      Math.sign(view.getInt8(vertexIndex * 20 + 14)),
    ].join(","));
  }
  return normals;
}

function meshChecksum(mesh: ChunkMeshData | null): string {
  if (!mesh) {
    return "null";
  }
  return [
    fnv1a(new Uint8Array(mesh.vertexData.slice(0))),
    fnv1a(new Uint8Array(mesh.indexData.buffer.slice(0))),
    String(mesh.triangleCount),
  ].join(":");
}

function extractTopCellKeys(mesh: ChunkMeshData | null): string[] {
  if (!mesh) {
    return [];
  }
  return extractQuads(mesh)
    .filter((quad) => quad.normal[1] === 1)
    .map((quad) => `${quad.min[0]}:${quad.min[2]}`);
}

function extractQuads(mesh: ChunkMeshData | null): Array<{
  normal: [number, number, number];
  min: [number, number, number];
  max: [number, number, number];
  color: number;
}> {
  if (!mesh) {
    return [];
  }
  const view = new DataView(mesh.vertexData);
  const quads: Array<{
    normal: [number, number, number];
    min: [number, number, number];
    max: [number, number, number];
    color: number;
  }> = [];
  for (let vertexIndex = 0; vertexIndex < mesh.vertexCount; vertexIndex += 4) {
    const byteOffset = vertexIndex * 20;
    const normal: [number, number, number] = [
      Math.sign(view.getInt8(byteOffset + 12)),
      Math.sign(view.getInt8(byteOffset + 13)),
      Math.sign(view.getInt8(byteOffset + 14)),
    ];
    let minX = Number.POSITIVE_INFINITY;
    let minY = Number.POSITIVE_INFINITY;
    let minZ = Number.POSITIVE_INFINITY;
    let maxX = Number.NEGATIVE_INFINITY;
    let maxY = Number.NEGATIVE_INFINITY;
    let maxZ = Number.NEGATIVE_INFINITY;
    for (let corner = 0; corner < 4; corner += 1) {
      const cornerOffset = (vertexIndex + corner) * 20;
      const x = view.getFloat32(cornerOffset, true);
      const y = view.getFloat32(cornerOffset + 4, true);
      const z = view.getFloat32(cornerOffset + 8, true);
      minX = Math.min(minX, x);
      minY = Math.min(minY, y);
      minZ = Math.min(minZ, z);
      maxX = Math.max(maxX, x);
      maxY = Math.max(maxY, y);
      maxZ = Math.max(maxZ, z);
    }
    quads.push({
      normal,
      min: [minX, minY, minZ],
      max: [maxX, maxY, maxZ],
      color: view.getUint32(byteOffset + 16, true),
    });
  }
  return quads;
}
