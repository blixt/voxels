import { mkdtemp, readFile } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";

import { expect, test } from "bun:test";

import { ProceduralWorldGenerator } from "../src/engine/procedural-generator.ts";
import {
  OBJECT_LAB_ROUTE_LANDMARK_IDS,
  findRepresentativeLandmarkRoot,
  runObjectLab,
  runObjectLabBatch,
} from "../scripts/object-lab.ts";

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
  expect(report.sample.diagnostics.scale.boundsSize).not.toBeNull();
  expect(report.sample.diagnostics.scale.verticalSpan).toBeGreaterThan(0);
  expect(report.sample.diagnostics.scale.maxHorizontalSpan).toBeGreaterThan(0);
  expect(report.sample.diagnostics.scale.occupiedColumnCount).toBe(
    report.sample.diagnostics.silhouette.top.occupiedPixels,
  );
  expect(report.sample.diagnostics.scale.boundsVolume).toBeGreaterThanOrEqual(report.sample.solidVoxelCount);
  expect(report.sample.diagnostics.scale.fillRatio).toBeGreaterThan(0);
  expect(report.sample.diagnostics.scale.solidVoxelBudget).not.toBe("empty");
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
  expect(await readFile(report.artifacts.summary, "utf8")).toContain("## Scale And Cost Diagnostics");
  expect(await readFile(report.artifacts.summary, "utf8")).toContain("## Sample Fit Diagnostics");
  expect(await readFile(report.artifacts.contactSheet, "utf8")).toContain("<svg");
  expect(await readFile(report.artifacts.contactSheet, "utf8")).toContain("Material legend");
  expect(await readFile(report.artifacts.contactSheet, "utf8")).toContain("fit x");
  expect(await readFile(report.artifacts.contactSheet, "utf8")).toContain("budget");
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
  const pilgrimLantern = await runObjectLab({
    landmarkId: "pilgrim_lantern",
    seed: 1337,
    outputDir,
    label: "Pilgrim Lantern Shape Test",
    timestamp: new Date("2026-05-08T12:35:03.000Z"),
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
  expect(kwamaMound.sample.diagnostics.warnings).not.toContain("dominant-material");
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

  expect(pilgrimLantern.sample.solidVoxelCount).toBeGreaterThan(350);
  expect(pilgrimLantern.sample.diagnostics.materialVariety).toBe(3);
  expect(pilgrimLantern.sample.diagnostics.dominantMaterialShare).toBeLessThan(0.75);
  expect(pilgrimLantern.sample.diagnostics.sampleFit.touchesSampleEdge).toBe(false);
  expect(pilgrimLantern.sample.diagnostics.sampleFit.touchesTop).toBe(false);
  expect(pilgrimLantern.sample.diagnostics.silhouette.front.occupiedRows).toBeGreaterThan(20);
  expect(pilgrimLantern.sample.diagnostics.silhouette.front.occupiedPixels).toBeGreaterThan(100);
  expect(pilgrimLantern.sample.diagnostics.warnings).not.toContain("sample-touches-horizontal-edge");
});

test("representative roots prefer centered salt-marsh set piece columns", async () => {
  const outputDir = await mkdtemp(join(tmpdir(), "voxels-object-lab-salt-marsh-"));
  const report = await runObjectLab({
    landmarkId: "fungal_bridge",
    seed: 1337,
    outputDir,
    label: "Fungal Bridge Root Selection Test",
    timestamp: new Date("2026-05-08T12:36:00.000Z"),
    scanRadius: 32_768,
    coarseStep: 64,
    sampleRadius: 24,
    heightPadding: 4,
  });

  expect(report.root.probe.landmarkId).toBe("fungal_bridge");
  expect(report.root.x).toBe(8464);
  expect(report.root.z).toBe(-28998);
  expect(report.sample.diagnostics.sampleFit.touchesSampleEdge).toBe(false);
  expect(report.sample.diagnostics.warnings).not.toContain("sample-touches-horizontal-edge");
  expect(report.sample.diagnostics.warnings).not.toContain("root-off-center");
  expect(report.sample.diagnostics.warnings).not.toContain("top-projection-touches-edge");
});

test("object lab labels intentional negative-space route debris separately from sparse failures", async () => {
  const outputDir = await mkdtemp(join(tmpdir(), "voxels-object-lab-negative-space-"));
  const screeFan = await runObjectLab({
    landmarkId: "scree_fan",
    seed: 1337,
    outputDir,
    label: "Scree Fan Negative Space Test",
    timestamp: new Date("2026-05-08T12:36:10.000Z"),
    scanRadius: 4096,
    coarseStep: 32,
    sampleRadius: 18,
    heightPadding: 4,
  });
  const buriedRibs = await runObjectLab({
    landmarkId: "buried_ribs",
    seed: 1337,
    outputDir,
    label: "Buried Ribs Negative Space Test",
    timestamp: new Date("2026-05-08T12:36:11.000Z"),
    scanRadius: 4096,
    coarseStep: 32,
    sampleRadius: 18,
    heightPadding: 4,
  });

  for (const report of [screeFan, buriedRibs]) {
    expect(report.sample.diagnostics.distinctiveness.formClass).toBe("negative-space");
    expect(report.sample.diagnostics.distinctiveness.intentionalNegativeSpace).toBe(true);
    expect(report.sample.diagnostics.distinctiveness.negativeSpaceRatio).toBeGreaterThan(0.8);
    expect(report.sample.diagnostics.distinctiveness.coverageBalance).toBeGreaterThan(0.5);
    expect(report.sample.diagnostics.warnings).toContain("intentional-negative-space");
    expect(report.sample.diagnostics.warnings).not.toContain("low-bounds-fill");
    expect(Array.isArray(report.sample.diagnostics.warningsSuppressed)).toBe(true);
  }

  expect(screeFan.sample.diagnostics.distinctiveness.topAsymmetry).toBeGreaterThan(0.45);
  expect(screeFan.sample.diagnostics.warnings).toContain("top-projection-touches-edge");
  expect(buriedRibs.sample.diagnostics.distinctiveness.frontAsymmetry).toBeGreaterThan(0.5);
  expect(buriedRibs.sample.diagnostics.warnings).not.toContain("top-projection-touches-edge");

  const summary = await readFile(screeFan.artifacts.summary, "utf8");
  expect(summary).toContain("## Distinctiveness Diagnostics");
  expect(summary).toContain("- Form class: negative-space");
  expect(summary).toContain("- Negative-space ratio:");
  expect(summary).toContain("- Suppressed warnings:");
});

test("object lab batch writes route landmark comparison diagnostics", async () => {
  expect(OBJECT_LAB_ROUTE_LANDMARK_IDS).toEqual([
    "ash_marker",
    "pilgrim_lantern",
    "bone_chimes",
    "paver_debris",
    "scree_fan",
    "shrine_debris",
    "buried_ribs",
    "old_road_causeway",
    "velothi_ziggurat",
    "ash_obelisk",
    "rib_arch",
    "crystal_reeds",
    "fungal_bridge",
    "rib_remains",
  ]);

  const outputDir = await mkdtemp(join(tmpdir(), "voxels-object-lab-batch-"));
  const report = await runObjectLabBatch({
    landmarkIds: ["ash_marker", "pilgrim_lantern"],
    seed: 1337,
    outputDir,
    label: "Route Landmark Comparison Test",
    timestamp: new Date("2026-05-08T12:37:00.000Z"),
    scanRadius: 4096,
    coarseStep: 32,
    sampleRadius: 18,
    heightPadding: 4,
  });

  expect(report.runDir.endsWith("2026-05-08-123700000Z-route-landmark-comparison-test")).toBe(true);
  expect(report.landmarkIds).toEqual(["ash_marker", "pilgrim_lantern"]);
  expect(report.reports.map((entry) => entry.landmarkId)).toEqual(["ash_marker", "pilgrim_lantern"]);
  expect(report.comparison).toHaveLength(2);
  expect(report.comparison[0]?.landmarkId).toBe("ash_marker");
  expect(report.comparison[0]?.solidVoxelCount).toBeGreaterThan(0);
  expect(report.comparison[0]?.boundsSize).not.toBeNull();
  expect(report.comparison[0]?.materialVariety).toBeGreaterThan(0);
  expect(report.comparison[0]?.formClass).toBe("route");
  expect(report.comparison[0]?.negativeSpaceRatio).toBeGreaterThan(0);
  expect(report.comparison[0]?.coverageBalance).toBeGreaterThan(0);
  expect(report.comparison[0]?.topAsymmetry).toBeGreaterThanOrEqual(0);
  expect(report.comparison[0]?.warningsSuppressed).toEqual([]);
  expect(report.comparison[0]?.topSilhouette.coverage).toBeGreaterThan(0);
  expect(report.comparison[0]?.frontSilhouette.normalizedHeight).toBeGreaterThan(0);
  expect(report.comparison[0]?.contactSheet.endsWith("contact-sheet.svg")).toBe(true);
  expect(report.artifacts.report.endsWith("batch-report.json")).toBe(true);
  expect(report.artifacts.summary.endsWith("comparison.md")).toBe(true);

  const batchJson = await readFile(report.artifacts.report, "utf8");
  const summary = await readFile(report.artifacts.summary, "utf8");
  expect(batchJson).toContain(`"landmarkIds": [`);
  expect(batchJson).toContain(`"ash_marker"`);
  expect(summary).toContain("# Object Lab Route Landmark Comparison");
  expect(summary).toContain("Negative Space");
  expect(summary).toContain("Coverage Balance");
  expect(summary).toContain("Suppressed");
  expect(summary).toContain("| ash_marker |");
  expect(summary).toContain("| pilgrim_lantern |");
  expect(summary).toContain("## Warning Queue");
});
