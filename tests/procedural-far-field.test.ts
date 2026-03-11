import { expect, test } from "bun:test";

import { ProceduralFarField } from "../src/engine/procedural-far-field.ts";
import { ProceduralWorldGenerator } from "../src/engine/procedural-generator.ts";
import { metersToWorldUnits } from "../src/engine/scale.ts";
import type { ChunkMeshData } from "../src/engine/types.ts";

test("procedural far field covers a few hundred meters with three render bands", () => {
  const farField = new ProceduralFarField(new ProceduralWorldGenerator(1337));

  const summary = farField.updateAround([0, 0, 0]);

  expect(summary.changed).toBe(true);
  expect(summary.meshCount).toBe(3);
  expect(summary.builtBands).toBe(3);
  expect(summary.maxRadiusMeters).toBeGreaterThanOrEqual(300);
  expect(summary.triangleCount).toBeGreaterThan(0);
  expect(farField.getRenderables().every((band) => band.mesh !== null)).toBe(true);
});

test("procedural far field reuses meshes while staying inside the current anchor window", () => {
  const farField = new ProceduralFarField(new ProceduralWorldGenerator(1337));
  farField.updateAround([0, 0, 0]);

  const summary = farField.updateAround([4, 0, 4]);

  expect(summary.changed).toBe(false);
  expect(summary.builtBands).toBe(0);
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
    createTestGenerator((worldX, worldZ) => (worldX === 0 && worldZ === 0 ? 8 : 1)),
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

function extractTopCellKeys(mesh: ChunkMeshData | null): string[] {
  if (!mesh) {
    return [];
  }
  const view = new DataView(mesh.vertexData);
  const keys: string[] = [];
  for (let vertexIndex = 0; vertexIndex < mesh.vertexCount; vertexIndex += 4) {
    const normalY = Math.sign(view.getInt8(vertexIndex * 20 + 13));
    if (normalY !== 1) {
      continue;
    }
    let minX = Number.POSITIVE_INFINITY;
    let minZ = Number.POSITIVE_INFINITY;
    for (let corner = 0; corner < 4; corner += 1) {
      const byteOffset = (vertexIndex + corner) * 20;
      minX = Math.min(minX, view.getFloat32(byteOffset, true));
      minZ = Math.min(minZ, view.getFloat32(byteOffset + 8, true));
    }
    keys.push(`${minX}:${minZ}`);
  }
  return keys;
}
