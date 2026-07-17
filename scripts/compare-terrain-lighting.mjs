import { readFile } from "node:fs/promises";
import { chromium } from "playwright";
import { chromeWebGpuLaunchOptions } from "./browser-harness.mjs";

const [referencePath, candidatePath] = process.argv.slice(2);
if (!referencePath || !candidatePath) {
  throw new Error("usage: node scripts/compare-terrain-lighting.mjs REFERENCE.png CANDIDATE.png");
}

const browser = await chromium.launch(chromeWebGpuLaunchOptions());
try {
  const context = await browser.newContext();
  const page = await context.newPage();
  const images = await Promise.all(
    [referencePath, candidatePath].map(async (file) => (await readFile(file)).toString("base64")),
  );
  const metrics = await page.evaluate(async ([reference, candidate]) => {
    const decode = async (base64) => {
      const response = await fetch(`data:image/png;base64,${base64}`);
      const bitmap = await createImageBitmap(await response.blob());
      const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
      const context = canvas.getContext("2d", { willReadFrequently: true });
      context.drawImage(bitmap, 0, 0);
      return {
        width: bitmap.width,
        height: bitmap.height,
        pixels: context.getImageData(0, 0, bitmap.width, bitmap.height).data,
      };
    };
    const [first, second] = await Promise.all([decode(reference), decode(candidate)]);
    if (first.width !== second.width || first.height !== second.height) {
      throw new Error("terrain-lighting images must have identical dimensions");
    }
    const linear = (value) => (value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4);
    const luma = (image, x, y) => {
      const index = (x + y * image.width) * 4;
      return (
        linear(image.pixels[index] / 255) * 0.2126 +
        linear(image.pixels[index + 1] / 255) * 0.7152 +
        linear(image.pixels[index + 2] / 255) * 0.0722
      );
    };
    // This is the same grounded valley ROI used by the LOD transition gate. Eight-pixel means
    // remove voxel/material grain; 32-pixel gradients retain landscape-scale illumination.
    const roi = {
      x0: Math.floor(first.width * 0.02),
      x1: Math.floor(first.width * 0.46),
      y0: Math.floor(first.height * 0.28),
      y1: Math.floor(first.height * 0.58),
    };
    const block = 8;
    const summarize = (image) => {
      const width = Math.floor((roi.x1 - roi.x0) / block);
      const height = Math.floor((roi.y1 - roi.y0) / block);
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
      const gradients = [];
      const lag = 4;
      for (let y = 0; y < height - lag; y += 1) {
        for (let x = 0; x < width - lag; x += 1) {
          const center = values[x + y * width];
          gradients.push(Math.abs(center - values[x + lag + y * width]));
          gradients.push(Math.abs(center - values[x + (y + lag) * width]));
        }
      }
      const sorted = [...values].sort((left, right) => left - right);
      const gradientRms = Math.sqrt(
        gradients.reduce((sum, value) => sum + value * value, 0) / gradients.length,
      );
      return {
        blockMean: sorted.reduce((sum, value) => sum + value, 0) / sorted.length,
        blockP10: sorted[Math.floor(sorted.length * 0.1)],
        blockP90: sorted[Math.floor(sorted.length * 0.9)],
        blockP90P10:
          sorted[Math.floor(sorted.length * 0.9)] - sorted[Math.floor(sorted.length * 0.1)],
        blockP90ToP10:
          sorted[Math.floor(sorted.length * 0.9)] /
          Math.max(sorted[Math.floor(sorted.length * 0.1)], 0.000_001),
        coarseGradientRms: gradientRms,
        coarseGradientRelativeRms:
          gradientRms /
          Math.max(sorted.reduce((sum, value) => sum + value, 0) / sorted.length, 0.000_001),
      };
    };
    const referenceMetrics = summarize(first);
    const candidateMetrics = summarize(second);
    return {
      roi,
      reference: referenceMetrics,
      candidate: candidateMetrics,
      ratios: {
        blockP90P10: candidateMetrics.blockP90P10 / referenceMetrics.blockP90P10,
        blockP90ToP10: candidateMetrics.blockP90ToP10 / referenceMetrics.blockP90ToP10,
        coarseGradientRms: candidateMetrics.coarseGradientRms / referenceMetrics.coarseGradientRms,
        coarseGradientRelativeRms:
          candidateMetrics.coarseGradientRelativeRms / referenceMetrics.coarseGradientRelativeRms,
      },
    };
  }, images);
  console.log(JSON.stringify(metrics, null, 2));
} finally {
  await browser.close();
}
