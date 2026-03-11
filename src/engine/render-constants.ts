import { metersToWorldUnits } from "./scale.ts";
import type { Vec3 } from "./types.ts";

export const CLEAR_COLOR_RGBA = [209, 224, 240, 255] as const;
export const LIGHT_DIRECTION: Vec3 = [0.55, -0.85, 0.45];
export const LIGHTING_TERMS = [0.28, 0.62, 0.2] as const;
export const FOG_COLOR_RGBA = CLEAR_COLOR_RGBA;
export const FOG_START_DISTANCE = metersToWorldUnits(96);
export const FOG_END_DISTANCE = metersToWorldUnits(416);
export const VALIDATION_RENDER_SIZE = 128;
