import { expect, test } from "bun:test";

import {
  applyWaterDepthTint,
  buildUnderwaterRenderEnvironment,
  DEFAULT_RENDER_ENVIRONMENT,
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
