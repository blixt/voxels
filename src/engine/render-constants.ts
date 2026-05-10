import { metersToWorldUnits } from "./scale.ts";
import type { Vec3 } from "./types.ts";

export type Rgba = readonly [number, number, number, number];

export interface SkyWeatherEnvironment {
  lightDirection?: Vec3;
  lightingTerms?: readonly [number, number, number];
  skyTopColorRgba: Rgba;
  skyHorizonColorRgba: Rgba;
  skyCloudColorRgba: Rgba;
  skyCloudCoverage: number;
  skyCloudBand: number;
  ashfallIntensity: number;
  rainfallIntensity: number;
  fungalGlowIntensity: number;
  sunGlowIntensity?: number;
  moonGlowIntensity?: number;
  starIntensity?: number;
}

export const CLEAR_COLOR_RGBA = [207, 223, 238, 255] as const;
export const LIGHT_DIRECTION: Vec3 = [0.55, -0.85, 0.45];
export const LIGHTING_TERMS = [0.40, 0.66, 0.28] as const;
export const FOG_COLOR_RGBA = CLEAR_COLOR_RGBA;
export const FOG_START_DISTANCE = metersToWorldUnits(96);
export const FOG_END_DISTANCE = metersToWorldUnits(480);
export const VALIDATION_RENDER_SIZE = 128;

export const DEFAULT_SKY_WEATHER_ENVIRONMENT: SkyWeatherEnvironment = {
  skyTopColorRgba: [78, 124, 172, 255],
  skyHorizonColorRgba: CLEAR_COLOR_RGBA,
  skyCloudColorRgba: [226, 231, 230, 255],
  skyCloudCoverage: 0.08,
  skyCloudBand: 0.64,
  ashfallIntensity: 0,
  rainfallIntensity: 0,
  fungalGlowIntensity: 0,
  sunGlowIntensity: 0.72,
  moonGlowIntensity: 0,
  starIntensity: 0,
};
