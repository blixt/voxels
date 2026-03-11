import { clamp, packRgba } from "./math.ts";
import {
  CLEAR_COLOR_RGBA,
  FOG_COLOR_RGBA,
  FOG_END_DISTANCE,
  FOG_START_DISTANCE,
} from "./render-constants.ts";
import { metersToWorldUnits } from "./scale.ts";
import type { PackedColor } from "./types.ts";

export interface RenderEnvironment {
  clearColorRgba: readonly [number, number, number, number];
  fogColorRgba: readonly [number, number, number, number];
  fogStartDistance: number;
  fogEndDistance: number;
}

export const DEFAULT_RENDER_ENVIRONMENT: RenderEnvironment = {
  clearColorRgba: CLEAR_COLOR_RGBA,
  fogColorRgba: FOG_COLOR_RGBA,
  fogStartDistance: FOG_START_DISTANCE,
  fogEndDistance: FOG_END_DISTANCE,
};

const UNDERWATER_FOG_START_DISTANCE = metersToWorldUnits(1.5);
const UNDERWATER_FOG_END_DISTANCE = metersToWorldUnits(24);
const WATER_SURFACE_ALPHA_MIN = 176;
const WATER_SURFACE_ALPHA_MAX = 236;
const WATER_SURFACE_DEPTH_PER_ALPHA_STEP = 3;

export function buildUnderwaterRenderEnvironment(waterColor: PackedColor): RenderEnvironment {
  const [red, green, blue] = unpackRgb(waterColor);
  const tinted: [number, number, number, number] = [
    Math.round(red * 0.56 + 10),
    Math.round(green * 0.68 + 18),
    Math.round(blue * 0.78 + 24),
    255,
  ];
  return {
    clearColorRgba: tinted,
    fogColorRgba: tinted,
    fogStartDistance: UNDERWATER_FOG_START_DISTANCE,
    fogEndDistance: UNDERWATER_FOG_END_DISTANCE,
  };
}

export function applyWaterDepthTint(baseColor: PackedColor, depthWorldUnits: number): PackedColor {
  const [red, green, blue, alpha] = unpackRgba(baseColor);
  const normalizedDepth = Math.max(0, depthWorldUnits);
  const targetAlpha = clamp(
    WATER_SURFACE_ALPHA_MIN + Math.round(normalizedDepth / WATER_SURFACE_DEPTH_PER_ALPHA_STEP),
    WATER_SURFACE_ALPHA_MIN,
    WATER_SURFACE_ALPHA_MAX,
  );
  const shading = 1 - Math.min(0.18, normalizedDepth * 0.003);
  return packRgba(
    Math.max(0, Math.min(255, Math.round(red * shading))),
    Math.max(0, Math.min(255, Math.round(green * shading))),
    Math.max(0, Math.min(255, Math.round(blue * shading))),
    Math.max(alpha, targetAlpha),
  );
}

function unpackRgb(color: PackedColor): [number, number, number] {
  return [
    color & 0xff,
    (color >>> 8) & 0xff,
    (color >>> 16) & 0xff,
  ];
}

function unpackRgba(color: PackedColor): [number, number, number, number] {
  return [
    color & 0xff,
    (color >>> 8) & 0xff,
    (color >>> 16) & 0xff,
    (color >>> 24) & 0xff,
  ];
}
