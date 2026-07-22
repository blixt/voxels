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

export interface RenderedImageComparison {
  readonly roi: PixelRegion;
  readonly sampledPixels: number;
  readonly comparisonSamples: number;
  readonly comparisonFootprintPixels: number;
  readonly ssim: number;
  readonly meanAbsoluteLinearLumaDelta: number;
  readonly relativeMeanLinearLumaDelta: number;
  readonly diagnosticGeometry: null | {
    readonly maskDisagreementPixels: number;
    readonly maskDisagreementFraction: number;
    readonly largestDisagreementComponentPixels: number;
    readonly largestDisagreementComponentFraction: number;
    readonly occupancyIntersectionPixels: number;
    readonly occupancyUnionPixels: number;
    readonly occupancyJaccard: number;
  };
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

/**
 * Compares two identically registered renderer captures in linear light.
 *
 * A small footprint suppresses intentional sub-pixel voxel texture differences while preserving
 * macro geometry and lighting deltas. With `diagnosticGeometry`, the same pass also compares the
 * exact geometry/sky silhouettes produced by the renderer's magenta diagnostic sky.
 */
export async function compareRenderedImages(
  page: Page,
  left: Buffer,
  right: Buffer,
  {
    region = FULL_IMAGE,
    footprintPixels = 4,
    diagnosticGeometry = false,
  }: {
    readonly region?: NormalizedImageRegion;
    readonly footprintPixels?: number;
    readonly diagnosticGeometry?: boolean;
  } = {},
): Promise<RenderedImageComparison> {
  validateRegion(region);
  if (!Number.isSafeInteger(footprintPixels) || footprintPixels < 1 || footprintPixels > 32) {
    throw new Error("image comparison footprint must be an integer within 1..=32");
  }
  return page.evaluate(
    async ({ leftBase64, rightBase64, normalizedRegion, footprint, compareGeometry }) => {
      const required = (values: ArrayLike<number>, index: number): number => {
        const value = values[index];
        if (value === undefined) throw new Error(`image comparison omitted value ${index}`);
        return value;
      };
      const decode = async (base64: string) => {
        const response = await fetch(`data:image/png;base64,${base64}`);
        const bitmap = await createImageBitmap(await response.blob());
        const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
        const context = canvas.getContext("2d", { willReadFrequently: true });
        if (context === null) throw new Error("image comparison canvas is unavailable");
        context.drawImage(bitmap, 0, 0);
        return {
          width: bitmap.width,
          height: bitmap.height,
          pixels: context.getImageData(0, 0, bitmap.width, bitmap.height).data,
        };
      };
      const [leftImage, rightImage] = await Promise.all([decode(leftBase64), decode(rightBase64)]);
      if (leftImage.width !== rightImage.width || leftImage.height !== rightImage.height) {
        throw new Error("comparison images have different dimensions");
      }
      const roi = {
        x0: Math.floor(leftImage.width * normalizedRegion.x0),
        x1: Math.ceil(leftImage.width * normalizedRegion.x1),
        y0: Math.floor(leftImage.height * normalizedRegion.y0),
        y1: Math.ceil(leftImage.height * normalizedRegion.y1),
      };
      const width = roi.x1 - roi.x0;
      const height = roi.y1 - roi.y0;
      const sampledPixels = width * height;
      const linear = (value: number): number => {
        const channel = value / 255;
        return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
      };
      const luma = (pixels: Uint8ClampedArray, index: number): number =>
        0.2126 * linear(required(pixels, index)) +
        0.7152 * linear(required(pixels, index + 1)) +
        0.0722 * linear(required(pixels, index + 2));
      const isDiagnosticSky = (pixels: Uint8ClampedArray, index: number): boolean =>
        required(pixels, index) >= 232 &&
        required(pixels, index + 1) <= 48 &&
        required(pixels, index + 2) >= 232;

      let comparisonSamples = 0;
      let sumLeft = 0;
      let sumRight = 0;
      let sumLeftSquared = 0;
      let sumRightSquared = 0;
      let sumProduct = 0;
      let sumAbsolute = 0;
      for (let y = roi.y0; y < roi.y1; y += footprint) {
        for (let x = roi.x0; x < roi.x1; x += footprint) {
          let leftLuma = 0;
          let rightLuma = 0;
          let footprintSampleCount = 0;
          for (let dy = 0; dy < footprint && y + dy < roi.y1; dy += 1) {
            for (let dx = 0; dx < footprint && x + dx < roi.x1; dx += 1) {
              const pixel = (x + dx + (y + dy) * leftImage.width) * 4;
              leftLuma += luma(leftImage.pixels, pixel);
              rightLuma += luma(rightImage.pixels, pixel);
              footprintSampleCount += 1;
            }
          }
          leftLuma /= footprintSampleCount;
          rightLuma /= footprintSampleCount;
          comparisonSamples += 1;
          sumLeft += leftLuma;
          sumRight += rightLuma;
          sumLeftSquared += leftLuma * leftLuma;
          sumRightSquared += rightLuma * rightLuma;
          sumProduct += leftLuma * rightLuma;
          sumAbsolute += Math.abs(leftLuma - rightLuma);
        }
      }
      const meanLeft = sumLeft / comparisonSamples;
      const meanRight = sumRight / comparisonSamples;
      const varianceLeft = sumLeftSquared / comparisonSamples - meanLeft * meanLeft;
      const varianceRight = sumRightSquared / comparisonSamples - meanRight * meanRight;
      const covariance = sumProduct / comparisonSamples - meanLeft * meanRight;
      const c1 = 0.01 ** 2;
      const c2 = 0.03 ** 2;

      let geometry: RenderedImageComparison["diagnosticGeometry"] = null;
      if (compareGeometry) {
        const disagreement = new Uint8Array(sampledPixels);
        let maskDisagreementPixels = 0;
        let occupancyIntersectionPixels = 0;
        let occupancyUnionPixels = 0;
        for (let y = roi.y0; y < roi.y1; y += 1) {
          for (let x = roi.x0; x < roi.x1; x += 1) {
            const source = (x + y * leftImage.width) * 4;
            const leftOccupied = !isDiagnosticSky(leftImage.pixels, source);
            const rightOccupied = !isDiagnosticSky(rightImage.pixels, source);
            if (leftOccupied && rightOccupied) occupancyIntersectionPixels += 1;
            if (leftOccupied || rightOccupied) occupancyUnionPixels += 1;
            if (leftOccupied === rightOccupied) continue;
            disagreement[x - roi.x0 + (y - roi.y0) * width] = 1;
            maskDisagreementPixels += 1;
          }
        }
        const visited = new Uint8Array(disagreement.length);
        let largestDisagreementComponentPixels = 0;
        for (let start = 0; start < disagreement.length; start += 1) {
          if (disagreement[start] === 0 || visited[start] !== 0) continue;
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
              if (neighbor < 0 || disagreement[neighbor] === 0 || visited[neighbor] !== 0) {
                continue;
              }
              visited[neighbor] = 1;
              stack.push(neighbor);
            }
          }
          largestDisagreementComponentPixels = Math.max(
            largestDisagreementComponentPixels,
            componentPixels,
          );
        }
        geometry = {
          maskDisagreementPixels,
          maskDisagreementFraction: maskDisagreementPixels / sampledPixels,
          largestDisagreementComponentPixels,
          largestDisagreementComponentFraction: largestDisagreementComponentPixels / sampledPixels,
          occupancyIntersectionPixels,
          occupancyUnionPixels,
          occupancyJaccard:
            occupancyUnionPixels === 0 ? 1 : occupancyIntersectionPixels / occupancyUnionPixels,
        };
      }

      return {
        roi,
        sampledPixels,
        comparisonSamples,
        comparisonFootprintPixels: footprint,
        ssim:
          ((2 * meanLeft * meanRight + c1) * (2 * covariance + c2)) /
          ((meanLeft * meanLeft + meanRight * meanRight + c1) *
            (varianceLeft + varianceRight + c2)),
        meanAbsoluteLinearLumaDelta: sumAbsolute / comparisonSamples,
        relativeMeanLinearLumaDelta: Math.abs(meanRight - meanLeft) / Math.max(meanLeft, 0.001),
        diagnosticGeometry: geometry,
      };
    },
    {
      leftBase64: left.toString("base64"),
      rightBase64: right.toString("base64"),
      normalizedRegion: region,
      footprint: footprintPixels,
      compareGeometry: diagnosticGeometry,
    },
  );
}
