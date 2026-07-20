import type { Page } from "playwright";

export interface NormalizedImageRegion {
  readonly x0: number;
  readonly x1: number;
  readonly y0: number;
  readonly y1: number;
}

export interface PixelRegion {
  readonly x0: number;
  readonly x1: number;
  readonly y0: number;
  readonly y1: number;
}

export interface DiagnosticSkyAnalysis {
  readonly roi: PixelRegion;
  readonly sampledPixels: number;
  readonly diagnosticSkyPixels: number;
  readonly diagnosticSkyFraction: number;
  readonly largestComponentPixels: number;
  readonly largestComponentFraction: number;
  readonly sampleCoordinates: readonly (readonly [number, number])[];
}

const FULL_IMAGE: NormalizedImageRegion = Object.freeze({ x0: 0, x1: 1, y0: 0, y1: 1 });

function validateRegion(region: NormalizedImageRegion): void {
  for (const [name, value] of Object.entries(region)) {
    if (!Number.isFinite(value) || value < 0 || value > 1) {
      throw new Error(`image region ${name} must be finite and within 0..=1`);
    }
  }
  if (region.x0 >= region.x1 || region.y0 >= region.y1) {
    throw new Error("image region must have positive width and height");
  }
}

/**
 * Counts the renderer's diagnostic-magenta sky inside a screenshot region.
 *
 * The HDR scene is tone-mapped before capture, so configured `[255, 0, 255]` arrives near
 * `[241, 34, 241]`. The narrow predicate tolerates browser color conversion while remaining
 * disjoint from every terrain material. Validation fixtures also suppress clouds and precipitation.
 */
export async function analyzeDiagnosticSky(
  page: Page,
  screenshot: Buffer,
  region: NormalizedImageRegion = FULL_IMAGE,
): Promise<DiagnosticSkyAnalysis> {
  validateRegion(region);
  return page.evaluate(
    async ({ base64, normalizedRegion }) => {
      const response = await fetch(`data:image/png;base64,${base64}`);
      const bitmap = await createImageBitmap(await response.blob());
      const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
      const context = canvas.getContext("2d", { willReadFrequently: true });
      if (context === null) throw new Error("diagnostic sky analysis canvas is unavailable");
      context.drawImage(bitmap, 0, 0);
      const pixels = context.getImageData(0, 0, bitmap.width, bitmap.height).data;
      const roi = {
        x0: Math.floor(bitmap.width * normalizedRegion.x0),
        x1: Math.ceil(bitmap.width * normalizedRegion.x1),
        y0: Math.floor(bitmap.height * normalizedRegion.y0),
        y1: Math.ceil(bitmap.height * normalizedRegion.y1),
      };
      const width = roi.x1 - roi.x0;
      const height = roi.y1 - roi.y0;
      const mask = new Uint8Array(width * height);
      const coordinates: (readonly [number, number])[] = [];
      let diagnosticSkyPixels = 0;
      for (let y = roi.y0; y < roi.y1; y += 1) {
        for (let x = roi.x0; x < roi.x1; x += 1) {
          const source = (x + y * bitmap.width) * 4;
          const red = pixels[source] ?? 0;
          const green = pixels[source + 1] ?? 0;
          const blue = pixels[source + 2] ?? 0;
          if (red < 232 || green > 48 || blue < 232) continue;
          mask[x - roi.x0 + (y - roi.y0) * width] = 1;
          diagnosticSkyPixels += 1;
          if (coordinates.length < 32) coordinates.push([x, y]);
        }
      }

      const visited = new Uint8Array(mask.length);
      let largestComponentPixels = 0;
      for (let start = 0; start < mask.length; start += 1) {
        if (mask[start] === 0 || visited[start] !== 0) continue;
        const stack = [start];
        visited[start] = 1;
        let componentPixels = 0;
        while (stack.length > 0) {
          const current = stack.pop();
          if (current === undefined) break;
          componentPixels += 1;
          const x = current % width;
          const y = Math.floor(current / width);
          const neighbors = [
            x > 0 ? current - 1 : -1,
            x + 1 < width ? current + 1 : -1,
            y > 0 ? current - width : -1,
            y + 1 < height ? current + width : -1,
          ];
          for (const neighbor of neighbors) {
            if (neighbor < 0 || mask[neighbor] === 0 || visited[neighbor] !== 0) continue;
            visited[neighbor] = 1;
            stack.push(neighbor);
          }
        }
        largestComponentPixels = Math.max(largestComponentPixels, componentPixels);
      }
      const sampledPixels = mask.length;
      return {
        roi,
        sampledPixels,
        diagnosticSkyPixels,
        diagnosticSkyFraction: diagnosticSkyPixels / sampledPixels,
        largestComponentPixels,
        largestComponentFraction: largestComponentPixels / sampledPixels,
        sampleCoordinates: coordinates,
      };
    },
    { base64: screenshot.toString("base64"), normalizedRegion: region },
  );
}
