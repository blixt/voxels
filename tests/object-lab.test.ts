import { mkdtemp, readFile } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";

import { expect, test } from "bun:test";

import { ProceduralWorldGenerator } from "../src/engine/procedural-generator.ts";
import { findRepresentativeLandmarkRoot, runObjectLab } from "../scripts/object-lab.ts";

test("object lab finds a representative landmark and writes isolated artifacts", async () => {
  const generator = new ProceduralWorldGenerator(1337);
  const root = findRepresentativeLandmarkRoot(generator, "oak", {
    scanRadius: 1024,
    coarseStep: 32,
    refineRadius: 48,
  });

  expect(root?.probe.landmarkId).toBe("oak");
  expect(root?.probe.surfaceY).toBeGreaterThan(0);

  const outputDir = await mkdtemp(join(tmpdir(), "voxels-object-lab-"));
  const report = await runObjectLab({
    landmarkId: "oak",
    seed: 1337,
    outputDir,
    label: "Oak Test",
    timestamp: new Date("2026-05-08T12:34:56.000Z"),
    sampleRadius: 10,
    heightPadding: 2,
    worldX: root!.x,
    worldZ: root!.z,
  });

  expect(report.runDir.endsWith("2026-05-08-123456000Z-oak-test")).toBe(true);
  expect(report.sample.solidVoxelCount).toBeGreaterThan(0);
  expect(report.sample.materialCounts.length).toBeGreaterThan(0);
  expect(report.sample.diagnostics.materialVariety).toBe(report.sample.materialCounts.length);
  expect(report.sample.diagnostics.dominantMaterialShare).toBeGreaterThan(0);
  expect(report.sample.diagnostics.sampleFit.centerOffset).not.toBeNull();
  expect(typeof report.sample.diagnostics.sampleFit.touchesSampleEdge).toBe("boolean");
  expect(report.sample.diagnostics.silhouette.front.occupiedPixels).toBeGreaterThan(0);
  expect(report.sample.diagnostics.silhouette.front.occupiedRows).toBeGreaterThan(0);
  expect(report.sample.diagnostics.silhouette.front.occupiedColumns).toBeGreaterThan(0);
  expect(report.sample.diagnostics.silhouette.front.centerOffset).not.toBeNull();
  expect(report.sample.diagnostics.silhouette.front.aspectRatio).not.toBeNull();
  expect(await readFile(report.artifacts.report, "utf8")).toContain(`"landmarkId": "oak"`);
  expect(await readFile(report.artifacts.summary, "utf8")).toContain("# Object Lab: oak");
  expect(await readFile(report.artifacts.summary, "utf8")).toContain("## Silhouette Diagnostics");
  expect(await readFile(report.artifacts.summary, "utf8")).toContain("## Sample Fit Diagnostics");
  expect(await readFile(report.artifacts.contactSheet, "utf8")).toContain("<svg");
  expect(await readFile(report.artifacts.contactSheet, "utf8")).toContain("Material legend");
  expect(await readFile(report.artifacts.contactSheet, "utf8")).toContain("fit x");
  expect(await readFile(report.artifacts.topProjection, "utf8")).toStartWith("P3\n");
  expect(await readFile(report.artifacts.frontProjection, "utf8")).toStartWith("P3\n");
  expect(await readFile(report.artifacts.sideProjection, "utf8")).toStartWith("P3\n");
});

test("ashland prop lab diagnostics keep snag branches and squat kwama mound accents", async () => {
  const outputDir = await mkdtemp(join(tmpdir(), "voxels-object-lab-ashland-"));
  const deadSnag = await runObjectLab({
    landmarkId: "dead_snag",
    seed: 1337,
    outputDir,
    label: "Dead Snag Shape Test",
    timestamp: new Date("2026-05-08T12:35:00.000Z"),
    scanRadius: 4096,
    coarseStep: 32,
    sampleRadius: 14,
    heightPadding: 4,
  });
  const kwamaMound = await runObjectLab({
    landmarkId: "kwama_mound",
    seed: 1337,
    outputDir,
    label: "Kwama Mound Shape Test",
    timestamp: new Date("2026-05-08T12:35:01.000Z"),
    scanRadius: 4096,
    coarseStep: 32,
    sampleRadius: 18,
    heightPadding: 4,
  });
  const ashMarker = await runObjectLab({
    landmarkId: "ash_marker",
    seed: 1337,
    outputDir,
    label: "Ash Marker Shape Test",
    timestamp: new Date("2026-05-08T12:35:02.000Z"),
    scanRadius: 4096,
    coarseStep: 32,
    sampleRadius: 18,
    heightPadding: 4,
  });

  expect(deadSnag.sample.solidVoxelCount).toBeGreaterThan(450);
  expect(deadSnag.sample.diagnostics.materialVariety).toBeGreaterThanOrEqual(2);
  expect(deadSnag.sample.diagnostics.dominantMaterialShare).toBeLessThan(0.8);
  expect(deadSnag.sample.diagnostics.silhouette.front.occupiedPixels).toBeGreaterThan(180);
  expect(deadSnag.sample.diagnostics.silhouette.side.occupiedPixels).toBeGreaterThan(180);

  expect(kwamaMound.sample.solidVoxelCount).toBeGreaterThan(900);
  expect(kwamaMound.sample.diagnostics.materialVariety).toBeGreaterThanOrEqual(2);
  expect(kwamaMound.sample.diagnostics.dominantMaterialShare).toBeLessThan(0.9);
  expect(kwamaMound.sample.diagnostics.silhouette.front.normalizedHeight).toBeLessThan(0.65);
  expect(kwamaMound.sample.diagnostics.silhouette.front.aspectRatio).toBeGreaterThan(1.1);

  expect(ashMarker.sample.solidVoxelCount).toBeGreaterThan(1300);
  expect(ashMarker.sample.diagnostics.materialVariety).toBe(2);
  expect(ashMarker.sample.diagnostics.dominantMaterialShare).toBeLessThan(0.65);
  expect(ashMarker.sample.diagnostics.sampleFit.touchesSampleEdge).toBe(false);
  expect(ashMarker.sample.diagnostics.sampleFit.touchesTop).toBe(false);
  expect(ashMarker.sample.diagnostics.silhouette.front.occupiedRows).toBeGreaterThan(35);
  expect(ashMarker.sample.diagnostics.silhouette.front.occupiedPixels).toBeGreaterThan(270);
  expect(ashMarker.sample.diagnostics.warnings).not.toContain("sample-touches-horizontal-edge");
});
