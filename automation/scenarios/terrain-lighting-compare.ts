import { readFile } from "node:fs/promises";
import { BrowserCapability } from "../lib/browser.ts";
import { defineScenario } from "../lib/scenario.ts";

interface ImageSample {
  readonly width: number;
  readonly height: number;
  readonly pixels: Uint8ClampedArray;
}

export interface LightingSummary {
  readonly blockMean: number;
  readonly blockP10: number;
  readonly blockP90: number;
  readonly blockP90P10: number;
  readonly blockP90ToP10: number;
  readonly coarseGradientRms: number;
  readonly coarseGradientRelativeRms: number;
}

interface LightingComparison {
  readonly roi: {
    readonly x0: number;
    readonly x1: number;
    readonly y0: number;
    readonly y1: number;
  };
  readonly reference: LightingSummary;
  readonly candidate: LightingSummary;
  readonly ratios: Omit<LightingSummary, "blockMean" | "blockP10" | "blockP90">;
}

export function lightingRatios(
  reference: LightingSummary,
  candidate: LightingSummary,
): LightingComparison["ratios"] {
  const ratio = (label: string, numerator: number, denominator: number): number => {
    if (!Number.isFinite(numerator)) {
      throw new Error(`candidate terrain lighting ${label} must be finite`);
    }
    if (!Number.isFinite(denominator) || denominator <= 0) {
      throw new Error(`reference terrain lighting has no measurable ${label}`);
    }
    return numerator / denominator;
  };
  return {
    blockP90P10: ratio("block contrast", candidate.blockP90P10, reference.blockP90P10),
    blockP90ToP10: ratio("contrast ratio", candidate.blockP90ToP10, reference.blockP90ToP10),
    coarseGradientRms: ratio(
      "coarse gradient",
      candidate.coarseGradientRms,
      reference.coarseGradientRms,
    ),
    coarseGradientRelativeRms: ratio(
      "relative coarse gradient",
      candidate.coarseGradientRelativeRms,
      reference.coarseGradientRelativeRms,
    ),
  };
}

export default defineScenario({
  id: "terrain-lighting-compare",
  kind: "analysis",
  summary: "Compares coarse terrain illumination in two equal-size PNG captures.",
  uses: { browser: true, metrics: true },
  timeoutMs: 60_000,
  async run(context, arguments_) {
    const [referencePath, candidatePath, ...extra] = arguments_;
    if (referencePath === undefined || candidatePath === undefined || extra.length > 0) {
      throw new Error("terrain-lighting-compare requires REFERENCE.png CANDIDATE.png");
    }
    const browser = await BrowserCapability.start(context);
    const viewport = await browser.open({
      url: "about:blank",
      label: "image-analysis",
      engine: false,
    });
    const images = await Promise.all(
      [referencePath, candidatePath].map(async (file) => (await readFile(file)).toString("base64")),
    );
    const analysis = await viewport.page.evaluate(
      async ([reference, candidate]): Promise<Omit<LightingComparison, "ratios">> => {
        const decode = async (base64: string): Promise<ImageSample> => {
          const response = await fetch(`data:image/png;base64,${base64}`);
          const bitmap = await createImageBitmap(await response.blob());
          const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
          const canvasContext = canvas.getContext("2d", { willReadFrequently: true });
          if (canvasContext === null) throw new Error("2D image analysis context is unavailable");
          canvasContext.drawImage(bitmap, 0, 0);
          return {
            width: bitmap.width,
            height: bitmap.height,
            pixels: canvasContext.getImageData(0, 0, bitmap.width, bitmap.height).data,
          };
        };
        const [first, second] = await Promise.all([decode(reference), decode(candidate)]);
        if (first.width !== second.width || first.height !== second.height) {
          throw new Error("terrain-lighting images must have identical dimensions");
        }
        const linear = (value: number): number =>
          value <= 0.040_45 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
        const luma = (image: ImageSample, x: number, y: number): number => {
          const index = (x + y * image.width) * 4;
          const red = image.pixels[index];
          const green = image.pixels[index + 1];
          const blue = image.pixels[index + 2];
          if (red === undefined || green === undefined || blue === undefined) {
            throw new Error("image sample is outside the decoded buffer");
          }
          return (
            linear(red / 255) * 0.2126 + linear(green / 255) * 0.7152 + linear(blue / 255) * 0.0722
          );
        };
        const roi = {
          x0: Math.floor(first.width * 0.02),
          x1: Math.floor(first.width * 0.46),
          y0: Math.floor(first.height * 0.28),
          y1: Math.floor(first.height * 0.58),
        };
        const block = 8;
        const summarize = (image: ImageSample): LightingSummary => {
          const width = Math.floor((roi.x1 - roi.x0) / block);
          const height = Math.floor((roi.y1 - roi.y0) / block);
          const lag = 4;
          if (width <= lag || height <= lag) {
            throw new Error(
              "terrain-lighting images are too small for 8-pixel blocks and a 4-block gradient lag",
            );
          }
          const values = new Float32Array(width * height);
          for (let blockY = 0; blockY < height; blockY += 1) {
            for (let blockX = 0; blockX < width; blockX += 1) {
              let sum = 0;
              for (let y = 0; y < block; y += 1) {
                for (let x = 0; x < block; x += 1) {
                  sum += luma(image, roi.x0 + blockX * block + x, roi.y0 + blockY * block + y);
                }
              }
              values[blockX + blockY * width] = sum / (block * block);
            }
          }
          const gradients: number[] = [];
          for (let y = 0; y < height - lag; y += 1) {
            for (let x = 0; x < width - lag; x += 1) {
              const center = values[x + y * width];
              const horizontal = values[x + lag + y * width];
              const vertical = values[x + (y + lag) * width];
              if (center === undefined || horizontal === undefined || vertical === undefined) {
                throw new Error("coarse lighting grid is incomplete");
              }
              gradients.push(Math.abs(center - horizontal), Math.abs(center - vertical));
            }
          }
          const sorted = [...values].sort((left, right) => left - right);
          const p10 = sorted[Math.floor(sorted.length * 0.1)] ?? 0;
          const p90 = sorted[Math.floor(sorted.length * 0.9)] ?? 0;
          const mean = sorted.reduce((sum, value) => sum + value, 0) / sorted.length;
          const gradientRms = Math.sqrt(
            gradients.reduce((sum, value) => sum + value * value, 0) / gradients.length,
          );
          return {
            blockMean: mean,
            blockP10: p10,
            blockP90: p90,
            blockP90P10: p90 - p10,
            blockP90ToP10: p90 / Math.max(p10, 0.000_001),
            coarseGradientRms: gradientRms,
            coarseGradientRelativeRms: gradientRms / Math.max(mean, 0.000_001),
          };
        };
        const referenceMetrics = summarize(first);
        const candidateMetrics = summarize(second);
        return {
          roi,
          reference: referenceMetrics,
          candidate: candidateMetrics,
        };
      },
      images as [string, string],
    );
    browser.assertHealthy();
    const metrics: LightingComparison = {
      ...analysis,
      ratios: lightingRatios(analysis.reference, analysis.candidate),
    };
    return {
      summary: "Terrain lighting comparison complete.",
      metrics: { ...metrics },
    };
  },
});
