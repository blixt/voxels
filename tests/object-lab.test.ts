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
  expect(report.sample.diagnostics.silhouette.front.occupiedPixels).toBeGreaterThan(0);
  expect(report.sample.diagnostics.silhouette.front.aspectRatio).not.toBeNull();
  expect(await readFile(report.artifacts.report, "utf8")).toContain(`"landmarkId": "oak"`);
  expect(await readFile(report.artifacts.summary, "utf8")).toContain("# Object Lab: oak");
  expect(await readFile(report.artifacts.summary, "utf8")).toContain("## Silhouette Diagnostics");
  expect(await readFile(report.artifacts.contactSheet, "utf8")).toContain("<svg");
  expect(await readFile(report.artifacts.contactSheet, "utf8")).toContain("Material legend");
  expect(await readFile(report.artifacts.topProjection, "utf8")).toStartWith("P3\n");
  expect(await readFile(report.artifacts.frontProjection, "utf8")).toStartWith("P3\n");
  expect(await readFile(report.artifacts.sideProjection, "utf8")).toStartWith("P3\n");
});
