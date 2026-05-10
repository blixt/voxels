import { expect, test } from "bun:test";

import {
  applyWaterDepthTint,
  buildUnderwaterRenderEnvironment,
  DEFAULT_RENDER_ENVIRONMENT,
  resolveWaterVisualParameters,
  type WaterVisualProfileId,
} from "../src/engine/water-visuals.ts";

test("depth tint increases water surface opacity with depth", () => {
  const shallow = applyWaterDepthTint(0xaaee8844, 1);
  const deep = applyWaterDepthTint(0xaaee8844, 64);

  expect(shallow >>> 24).toBeGreaterThanOrEqual(176);
  expect(deep >>> 24).toBeGreaterThan(shallow >>> 24);
  expect(deep >>> 24).toBeLessThanOrEqual(236);
});

test("underwater render environment uses a tinted shorter-range fog", () => {
  const environment = buildUnderwaterRenderEnvironment(0xaaee8844);

  expect(environment.fogEndDistance).toBeLessThan(DEFAULT_RENDER_ENVIRONMENT.fogEndDistance);
  expect(environment.fogStartDistance).toBeLessThan(environment.fogEndDistance);
  expect(environment.clearColorRgba).toEqual(environment.fogColorRgba);
  expect(environment.fogColorRgba).not.toEqual(DEFAULT_RENDER_ENVIRONMENT.fogColorRgba);
});

test("regional water presets separate murky, brackish, and glassy reads", () => {
  const profiles: WaterVisualProfileId[] = ["ashlands", "salt-marsh", "wetlands", "glass-coast"];
  const environments = profiles.map((profile) => buildUnderwaterRenderEnvironment(0xcc8c6a42, profile));
  const deepColors = profiles.map((profile) => applyWaterDepthTint(0xaa8c6a42, 48, profile));

  expect(environments[2]!.fogEndDistance).toBeLessThan(environments[0]!.fogEndDistance);
  expect(environments[3]!.fogEndDistance).toBeGreaterThan(environments[1]!.fogEndDistance);
  expect(environments[3]!.fogColorRgba[2]).toBeGreaterThan(environments[0]!.fogColorRgba[2] + 30);
  expect(deepColors[2]! >>> 24).toBeGreaterThan(deepColors[3]! >>> 24);

  for (let leftIndex = 0; leftIndex < environments.length; leftIndex += 1) {
    for (let rightIndex = leftIndex + 1; rightIndex < environments.length; rightIndex += 1) {
      expect(colorDistance(environments[leftIndex]!.fogColorRgba, environments[rightIndex]!.fogColorRgba))
        .toBeGreaterThan(18);
    }
  }
});

test("water visual parameters stay bounded and cheap to apply", () => {
  const profiles: WaterVisualProfileId[] = ["default", "ashlands", "salt-marsh", "wetlands", "glass-coast"];
  for (const profile of profiles) {
    const parameters = resolveWaterVisualParameters(profile);
    const shallow = applyWaterDepthTint(0x806ab8ce, 0, profile);
    const deep = applyWaterDepthTint(0x806ab8ce, 96, profile);

    expect(parameters.underwaterFogStartDistance).toBeLessThan(parameters.underwaterFogEndDistance);
    expect(parameters.surfaceAlphaMin).toBeGreaterThanOrEqual(160);
    expect(parameters.surfaceAlphaMax).toBeLessThanOrEqual(244);
    expect(deep >>> 24).toBeGreaterThanOrEqual(shallow >>> 24);
    expect(deep >>> 24).toBeLessThanOrEqual(parameters.surfaceAlphaMax);
  }
});

function colorDistance(
  left: readonly [number, number, number, number],
  right: readonly [number, number, number, number],
): number {
  return Math.abs(left[0] - right[0])
    + Math.abs(left[1] - right[1])
    + Math.abs(left[2] - right[2]);
}
