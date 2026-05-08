import { mkdtemp, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { expect, test } from "bun:test";

import {
  analyzeSurfaceGrid,
  runTerrainSurfaceLab,
  type TerrainPatchSpec,
  type TerrainSurfaceSample,
} from "../scripts/terrain-surface-lab.ts";

const TEST_PATCH: TerrainPatchSpec = {
  id: "synthetic-road",
  label: "Synthetic Road",
  source: "route-atlas",
  centerMeters: [0, 0],
  focusLandmarkIds: ["old_road_causeway"],
};

test("terrain surface grid analysis reports diversity, modulo, flatness, and grid-likeness", () => {
  const samples = buildSyntheticGrid([
    [100, 100, 100],
    [100, 104, 104],
    [100, 104, 108],
  ], [
    [1, 1, 2],
    [1, 2, 2],
    [3, 3, 2],
  ]);

  const analysis = analyzeSurfaceGrid(TEST_PATCH, samples, { radiusMeters: 4, stepMeters: 1 });

  expect(analysis.sampleCount).toBe(9);
  expect(analysis.gridSize).toEqual({ width: 3, height: 3 });
  expect(analysis.surfaceMeters).toEqual({ min: 10, max: 10.8, mean: 10.222, range: 0.8 });
  expect(analysis.materialDiversity.distinctMaterialCount).toBe(3);
  expect(analysis.materialDiversity.dominantMaterial?.material).toBe(2);
  expect(analysis.materialDiversity.dominantMaterial?.share).toBe(0.444);
  expect(analysis.heightModulo.modulo).toBe(8);
  expect(analysis.heightModulo.buckets.find((bucket) => bucket.remainder === 4)?.count).toBe(6);
  expect(analysis.flatness.averageSlope).toBeGreaterThan(0);
  expect(analysis.flatness.buckets.map((bucket) => bucket.id)).toEqual(["flat", "gentle", "rolling", "steep", "cliff"]);
  expect(analysis.gridLikeness.axisEdgeRate).toBeGreaterThan(0);
  expect(analysis.gridLikeness.axisEdgeCount).toBeGreaterThan(0);
  expect(analysis.distinctBiomes).toEqual(["test"]);
  expect(analysis.directLandmarks).toEqual(["old_road_causeway"]);
  expect(analysis.missingFocusLandmarkIds).toEqual([]);
});

test("terrain surface lab writes report and markdown summary artifacts", async () => {
  const outputDir = await mkdtemp(join(tmpdir(), "voxels-terrain-lab-"));
  const report = await runTerrainSurfaceLab({
    outputDir,
    label: "Terrain Lab Shape Test",
    timestamp: new Date("2026-05-08T12:38:00.000Z"),
    patchIds: ["ash-marker-vista"],
    radiusMeters: 12,
    stepMeters: 6,
  });

  expect(report.runDir.endsWith("20260508T123800Z-Terrain-Lab-Shape-Test")).toBe(true);
  expect(report.artifacts.report.endsWith("report.json")).toBe(true);
  expect(report.artifacts.summary.endsWith("summary.md")).toBe(true);
  expect(report.aggregate.patchCount).toBe(1);
  expect(report.aggregate.sampleCount).toBe(25);
  expect(report.patches[0]?.id).toBe("ash-marker-vista");
  expect(report.patches[0]?.materialDiversity.distinctMaterialCount).toBeGreaterThan(0);
  expect(report.patches[0]?.heightModulo.buckets).toHaveLength(8);
  expect(report.patches[0]?.flatness.buckets).toHaveLength(5);
  expect(report.patches[0]?.gridLikeness.score).toBeGreaterThanOrEqual(0);

  const json = await readFile(report.artifacts.report, "utf8");
  const summary = await readFile(report.artifacts.summary, "utf8");
  expect(json).toContain(`"patchCount": 1`);
  expect(json).toContain(`"materialDiversity"`);
  expect(summary).toContain("# Terrain Surface Lab Summary");
  expect(summary).toContain("| Ash Marker Vista |");
  expect(summary).toContain("## Height Modulo");
});

test("terrain surface lab compare mode writes aggregate and per-patch deltas", async () => {
  const outputDir = await mkdtemp(join(tmpdir(), "voxels-terrain-lab-compare-"));
  const baseline = await runTerrainSurfaceLab({
    outputDir,
    label: "Terrain Lab Baseline",
    timestamp: new Date("2026-05-08T12:40:00.000Z"),
    patchIds: ["ash-marker-vista", "causeway-road"],
    radiusMeters: 12,
    stepMeters: 6,
  });
  const comparison = await runTerrainSurfaceLab({
    outputDir,
    label: "Terrain Lab Comparison",
    timestamp: new Date("2026-05-08T12:41:00.000Z"),
    patchIds: ["ash-marker-vista", "causeway-road"],
    radiusMeters: 12,
    stepMeters: 6,
    compareTo: baseline.artifacts.report,
  });

  expect(comparison.comparison?.baselinePath).toBe(baseline.artifacts.report);
  expect(comparison.comparison?.baselineGeneratedAt).toBe("2026-05-08T12:40:00.000Z");
  expect(comparison.comparison?.aggregateDeltas.averageMaterialCount).toBe(0);
  expect(comparison.comparison?.aggregateDeltas.averageDominantMaterialShare).toBe(0);
  expect(comparison.comparison?.aggregateDeltas.maxSurfaceRangeMeters).toBe(0);
  expect(comparison.comparison?.aggregateDeltas.averageFlatnessShareDeltas.flat).toBe(0);
  expect(comparison.comparison?.patchDeltas).toHaveLength(2);
  expect(comparison.comparison?.patchDeltas[0]?.baselinePresent).toBe(true);
  expect(comparison.comparison?.patchDeltas[0]?.materialCount).toBe(0);
  expect(comparison.comparison?.patchDeltas[0]?.dominantMaterialShare).toBe(0);
  expect(comparison.comparison?.patchDeltas[0]?.surfaceRangeMeters).toBe(0);
  expect(comparison.comparison?.patchDeltas[0]?.gridLikenessScore).toBe(0);
  expect(comparison.comparison?.patchDeltas[0]?.flatnessShareDeltas.flat).toBe(0);

  const json = await readFile(comparison.artifacts.report, "utf8");
  const summary = await readFile(comparison.artifacts.summary, "utf8");
  expect(json).toContain(`"comparison"`);
  expect(json).toContain(`"patchDeltas"`);
  expect(summary).toContain("## Comparison");
  expect(summary).toContain("### Patch Deltas");
  expect(summary).toContain("| Average material count | +0.00 |");
});

function buildSyntheticGrid(
  heights: readonly (readonly number[])[],
  materials: readonly (readonly number[])[],
): TerrainSurfaceSample[] {
  const samples: TerrainSurfaceSample[] = [];
  for (let z = 0; z < heights.length; z += 1) {
    for (let x = 0; x < (heights[z]?.length ?? 0); x += 1) {
      samples.push({
        gridX: x,
        gridZ: z,
        worldX: x,
        worldZ: z,
        surfaceY: heights[z]![x]!,
        surfaceMaterial: materials[z]![x]!,
        biomeId: "test",
        undergroundBiomeId: "under-test",
        regionalVariantId: z === 0 ? "ridge" : null,
        landmarkId: x === 1 && z === 1 ? "old_road_causeway" : null,
      });
    }
  }
  return samples;
}
