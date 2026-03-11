import { expect, test } from "bun:test";

import { ProceduralFarField } from "../src/engine/procedural-far-field.ts";
import { ProceduralWorldGenerator } from "../src/engine/procedural-generator.ts";
import { metersToWorldUnits } from "../src/engine/scale.ts";

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

  const summary = farField.updateAround([40, 0, 40]);

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
