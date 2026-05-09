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

export type WaterVisualProfileId =
  | "default"
  | "ashlands"
  | "salt-marsh"
  | "glass-coast"
  | "wetlands";

export interface WaterVisualParameters {
  underwaterFogStartDistance: number;
  underwaterFogEndDistance: number;
  surfaceAlphaMin: number;
  surfaceAlphaMax: number;
  surfaceDepthPerAlphaStep: number;
  surfaceShadePerWorldUnit: number;
  surfaceMaxShadeLoss: number;
  redTint: number;
  greenTint: number;
  blueTint: number;
  redBias: number;
  greenBias: number;
  blueBias: number;
}

const WATER_VISUAL_PARAMETERS: Record<WaterVisualProfileId, WaterVisualParameters> = {
  default: {
    underwaterFogStartDistance: UNDERWATER_FOG_START_DISTANCE,
    underwaterFogEndDistance: UNDERWATER_FOG_END_DISTANCE,
    surfaceAlphaMin: WATER_SURFACE_ALPHA_MIN,
    surfaceAlphaMax: WATER_SURFACE_ALPHA_MAX,
    surfaceDepthPerAlphaStep: WATER_SURFACE_DEPTH_PER_ALPHA_STEP,
    surfaceShadePerWorldUnit: 0.003,
    surfaceMaxShadeLoss: 0.18,
    redTint: 0.56,
    greenTint: 0.68,
    blueTint: 0.78,
    redBias: 10,
    greenBias: 18,
    blueBias: 24,
  },
  ashlands: {
    underwaterFogStartDistance: metersToWorldUnits(1.2),
    underwaterFogEndDistance: metersToWorldUnits(15),
    surfaceAlphaMin: 192,
    surfaceAlphaMax: 242,
    surfaceDepthPerAlphaStep: 2.4,
    surfaceShadePerWorldUnit: 0.0042,
    surfaceMaxShadeLoss: 0.28,
    redTint: 0.68,
    greenTint: 0.48,
    blueTint: 0.42,
    redBias: 20,
    greenBias: 12,
    blueBias: 8,
  },
  "salt-marsh": {
    underwaterFogStartDistance: metersToWorldUnits(1),
    underwaterFogEndDistance: metersToWorldUnits(18),
    surfaceAlphaMin: 186,
    surfaceAlphaMax: 238,
    surfaceDepthPerAlphaStep: 2.6,
    surfaceShadePerWorldUnit: 0.0034,
    surfaceMaxShadeLoss: 0.22,
    redTint: 0.62,
    greenTint: 0.70,
    blueTint: 0.58,
    redBias: 18,
    greenBias: 24,
    blueBias: 12,
  },
  "glass-coast": {
    underwaterFogStartDistance: metersToWorldUnits(2),
    underwaterFogEndDistance: metersToWorldUnits(34),
    surfaceAlphaMin: 164,
    surfaceAlphaMax: 224,
    surfaceDepthPerAlphaStep: 3.8,
    surfaceShadePerWorldUnit: 0.0022,
    surfaceMaxShadeLoss: 0.12,
    redTint: 0.48,
    greenTint: 0.72,
    blueTint: 0.92,
    redBias: 8,
    greenBias: 20,
    blueBias: 34,
  },
  wetlands: {
    underwaterFogStartDistance: metersToWorldUnits(0.8),
    underwaterFogEndDistance: metersToWorldUnits(12),
    surfaceAlphaMin: 202,
    surfaceAlphaMax: 244,
    surfaceDepthPerAlphaStep: 2.1,
    surfaceShadePerWorldUnit: 0.0048,
    surfaceMaxShadeLoss: 0.32,
    redTint: 0.42,
    greenTint: 0.58,
    blueTint: 0.52,
    redBias: 8,
    greenBias: 18,
    blueBias: 14,
  },
};

export function resolveWaterVisualParameters(profileId: WaterVisualProfileId = "default"): WaterVisualParameters {
  return WATER_VISUAL_PARAMETERS[profileId];
}

export function buildUnderwaterRenderEnvironment(
  waterColor: PackedColor,
  profileId: WaterVisualProfileId = "default",
): RenderEnvironment {
  const parameters = resolveWaterVisualParameters(profileId);
  const [red, green, blue] = unpackRgb(waterColor);
  const tinted: [number, number, number, number] = [
    clampColor(Math.round(red * parameters.redTint + parameters.redBias)),
    clampColor(Math.round(green * parameters.greenTint + parameters.greenBias)),
    clampColor(Math.round(blue * parameters.blueTint + parameters.blueBias)),
    255,
  ];
  return {
    clearColorRgba: tinted,
    fogColorRgba: tinted,
    fogStartDistance: parameters.underwaterFogStartDistance,
    fogEndDistance: parameters.underwaterFogEndDistance,
  };
}

export function applyWaterDepthTint(
  baseColor: PackedColor,
  depthWorldUnits: number,
  profileId: WaterVisualProfileId = "default",
): PackedColor {
  const parameters = resolveWaterVisualParameters(profileId);
  const [red, green, blue, alpha] = unpackRgba(baseColor);
  const normalizedDepth = Math.max(0, depthWorldUnits);
  const targetAlpha = clamp(
    parameters.surfaceAlphaMin + Math.round(normalizedDepth / parameters.surfaceDepthPerAlphaStep),
    parameters.surfaceAlphaMin,
    parameters.surfaceAlphaMax,
  );
  const shading = 1 - Math.min(parameters.surfaceMaxShadeLoss, normalizedDepth * parameters.surfaceShadePerWorldUnit);
  return packRgba(
    clampColor(Math.round(red * shading)),
    clampColor(Math.round(green * shading)),
    clampColor(Math.round(blue * shading)),
    Math.max(alpha, targetAlpha),
  );
}

function clampColor(channel: number): number {
  return Math.max(0, Math.min(255, channel));
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
