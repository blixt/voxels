import { expect, test } from "bun:test";

import { ProceduralFarField } from "../src/engine/procedural-far-field.ts";
import { ProceduralWorldGenerator } from "../src/engine/procedural-generator.ts";
import { ProceduralResidentWorld } from "../src/engine/procedural-resident-world.ts";
import { metersToWorldUnits } from "../src/engine/scale.ts";
import type { Vec3 } from "../src/engine/types.ts";

test("procedural far field coverage stays continuous around settled streamed anchors", () => {
  const generator = new ProceduralWorldGenerator(1337);
  const world = new ProceduralResidentWorld(generator);
  const farField = new ProceduralFarField(world, [
    { label: "near", innerRadius: 0, outerRadius: metersToWorldUnits(24), sampleStride: metersToWorldUnits(0.8), anchorStride: metersToWorldUnits(25.6) },
    { label: "mid", innerRadius: metersToWorldUnits(24), outerRadius: metersToWorldUnits(48), sampleStride: metersToWorldUnits(1.6), anchorStride: metersToWorldUnits(25.6) },
    { label: "far", innerRadius: metersToWorldUnits(48), outerRadius: metersToWorldUnits(64), sampleStride: metersToWorldUnits(3.2), anchorStride: metersToWorldUnits(51.2) },
  ]);
  const spawn = world.getSpawnPosition();
  const chunkSize = world.chunkSize;
  const positions: Vec3[] = [
    spawn,
    [spawn[0] + chunkSize, spawn[1], spawn[2]],
    [spawn[0] + 2 * chunkSize, spawn[1], spawn[2]],
    [spawn[0], spawn[1], spawn[2] + 2 * chunkSize],
  ];

  for (const position of positions) {
    const nearProbe = settleAndProbeCoverage(world, farField, position, 24, 0.8);
    expect(nearProbe.residentOverlapCount).toBe(0);
    expect(nearProbe.uncoveredGapCount).toBe(0);
    expect(nearProbe.bandOverlapCount).toBe(0);
    expect(nearProbe.wrongBandCount).toBe(0);

    const farProbe = settleAndProbeCoverage(world, farField, position, 56, 3.2);
    expect(farProbe.residentOverlapCount).toBe(0);
    expect(farProbe.uncoveredGapCount).toBe(0);
  }
}, { timeout: 20_000 });

function settleAndProbeCoverage(
  world: ProceduralResidentWorld,
  farField: ProceduralFarField,
  position: Vec3,
  sampleRadiusMeters: number,
  sampleStepMeters: number,
): {
  residentOverlapCount: number;
  uncoveredGapCount: number;
  bandOverlapCount: number;
  wrongBandCount: number;
} {
  world.updateResidencyAround(position, { maxGenerateChunks: Number.POSITIVE_INFINITY });
  world.prefetchFarFieldSummariesAround(position, farField.getMaxRadiusWorldUnits(), 512);
  farField.updateAround(position, 0, world.getFarFieldExclusionMask());
  const sampleRadiusWorldUnits = metersToWorldUnits(sampleRadiusMeters);
  const sampleStepWorldUnits = metersToWorldUnits(sampleStepMeters);
  const maxRadiusWorldUnits = farField.lastUpdate.maxRadiusWorldUnits;
  let residentOverlapCount = 0;
  let uncoveredGapCount = 0;
  let bandOverlapCount = 0;
  let wrongBandCount = 0;

  for (let offsetZ = -sampleRadiusWorldUnits; offsetZ <= sampleRadiusWorldUnits; offsetZ += sampleStepWorldUnits) {
    for (let offsetX = -sampleRadiusWorldUnits; offsetX <= sampleRadiusWorldUnits; offsetX += sampleStepWorldUnits) {
      const distanceWorldUnits = Math.max(Math.abs(offsetX), Math.abs(offsetZ));
      if (distanceWorldUnits > maxRadiusWorldUnits) {
        continue;
      }
      if (isBandBoundaryZone(distanceWorldUnits, farField.getRenderables(), sampleStepWorldUnits)) {
        continue;
      }
      const worldX = position[0] + offsetX;
      const worldZ = position[2] + offsetZ;
      const resident = world.hasResidentColumn(
        Math.floor(worldX / world.chunkSize),
        Math.floor(worldZ / world.chunkSize),
      );
      const coverage = farField.getCoverageAt(worldX, worldZ, world.getFarFieldExclusionMask());
      if (resident && coverage.length > 0) {
        residentOverlapCount += 1;
      }
      if (!resident && coverage.length === 0) {
        uncoveredGapCount += 1;
      }
      if (coverage.length > 1) {
        bandOverlapCount += 1;
      }
      if (coverage.some((band) => distanceWorldUnits < band.innerRadiusWorldUnits || distanceWorldUnits >= band.outerRadiusWorldUnits)) {
        wrongBandCount += 1;
      }
    }
  }

  return {
    residentOverlapCount,
    uncoveredGapCount,
    bandOverlapCount,
    wrongBandCount,
  };
}

function isBandBoundaryZone(
  distanceWorldUnits: number,
  bands: readonly ReturnType<ProceduralFarField["getRenderables"]>[number][],
  toleranceWorldUnits: number,
): boolean {
  return bands.some((band) =>
    Math.abs(distanceWorldUnits - band.innerRadius) < toleranceWorldUnits
    || Math.abs(distanceWorldUnits - band.outerRadius) < toleranceWorldUnits);
}
