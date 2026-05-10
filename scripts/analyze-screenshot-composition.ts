import { readdir, readFile } from "node:fs/promises";
import { join } from "node:path";
import { inflateSync } from "node:zlib";

interface PngImage {
  width: number;
  height: number;
  pixels: Uint8Array;
}

interface RegionSpec {
  id: string;
  label: string;
  x0: number;
  y0: number;
  x1: number;
  y1: number;
}

interface RegionMetrics {
  id: string;
  label: string;
  bounds: {
    x: [number, number];
    y: [number, number];
  };
  sampleCount: number;
  avgLuma: number;
  lumaStdDev: number;
  avgSaturation: number;
  quantizedColorCount: number;
  horizontalEdgeRate: number;
  verticalEdgeRate: number;
  axisEdgeRate: number;
  diagonalEdgeRate: number;
  axisAlignedEdgeDominance: number;
  colorEdgeRate: number;
  warmRatio: number;
  coolRatio: number;
}

interface CompositionReport {
  generatedAt: string;
  imagePath: string;
  image: {
    width: number;
    height: number;
  };
  regions: RegionMetrics[];
  diagnosis: {
    highestAxisDominanceRegion: string | null;
    highestAxisDominance: number;
    highestColorRegion: string | null;
    highestColorCount: number;
    likelyGridDriver: string | null;
    groundToSkyAxisRatio: number | null;
    lowerGroundGridDominance: number | null;
    centerGridDominance: number | null;
  };
  artifacts: {
    report: string;
    summary: string;
  };
}

const args = Bun.argv.slice(2);
const imagePath = readFlag(args, "--image") ?? await findLatestOwnedBrowserScreenshot("artifacts/owned-browser-lab");
if (!imagePath) {
  throw new Error("No screenshot found. Pass --image <path> or run owned-browser-lab first.");
}

const label = sanitizeFileStem(readFlag(args, "--label") ?? "screenshot-composition");
const outputDir = readFlag(args, "--output-dir") ?? "artifacts/screenshot-composition";
const runDir = join(outputDir, `${timestampForFile(new Date())}-${label}`);
const reportPath = join(runDir, "report.json");
const summaryPath = join(runDir, "summary.md");

await Bun.$`mkdir -p ${runDir}`.quiet();

const image = decodePng(await readFile(imagePath));
const regions = buildRegionSpecs().map((region) => analyzeRegion(image, region));
const report: CompositionReport = {
  generatedAt: new Date().toISOString(),
  imagePath,
  image: {
    width: image.width,
    height: image.height,
  },
  regions,
  diagnosis: diagnoseComposition(regions),
  artifacts: {
    report: reportPath,
    summary: summaryPath,
  },
};

await Bun.write(reportPath, `${JSON.stringify(report, null, 2)}\n`);
await Bun.write(summaryPath, buildMarkdownSummary(report));

console.log(`screenshot composition report: ${reportPath}`);
console.log(`summary: ${summaryPath}`);
console.log(`image: ${imagePath}`);
console.log(`likely grid driver: ${report.diagnosis.likelyGridDriver ?? "none"}`);
console.log(`highest axis dominance: ${report.diagnosis.highestAxisDominanceRegion ?? "none"} ${report.diagnosis.highestAxisDominance.toFixed(2)}`);

function buildRegionSpecs(): RegionSpec[] {
  return [
    { id: "full_subject", label: "Full Subject", x0: 0.08, y0: 0.18, x1: 0.92, y1: 0.78 },
    { id: "sky", label: "Sky", x0: 0.08, y0: 0.00, x1: 0.92, y1: 0.32 },
    { id: "horizon", label: "Horizon", x0: 0.08, y0: 0.28, x1: 0.92, y1: 0.52 },
    { id: "center_view", label: "Center View", x0: 0.20, y0: 0.24, x1: 0.80, y1: 0.68 },
    { id: "lower_ground", label: "Lower Ground", x0: 0.08, y0: 0.55, x1: 0.92, y1: 0.90 },
    { id: "bottom_center", label: "Bottom Center", x0: 0.30, y0: 0.62, x1: 0.70, y1: 0.90 },
    { id: "lower_left", label: "Lower Left", x0: 0.08, y0: 0.55, x1: 0.36, y1: 0.90 },
    { id: "lower_right", label: "Lower Right", x0: 0.64, y0: 0.55, x1: 0.92, y1: 0.90 },
  ];
}

function analyzeRegion(image: PngImage, region: RegionSpec): RegionMetrics {
  const xStart = Math.max(0, Math.floor(image.width * region.x0));
  const xEnd = Math.min(image.width, Math.ceil(image.width * region.x1));
  const yStart = Math.max(0, Math.floor(image.height * region.y0));
  const yEnd = Math.min(image.height, Math.ceil(image.height * region.y1));
  const colorBuckets = new Set<number>();
  let sampleCount = 0;
  let saturationTotal = 0;
  let lumaTotal = 0;
  let lumaSquaredTotal = 0;
  let warmSampleCount = 0;
  let coolSampleCount = 0;
  let horizontalEdges = 0;
  let horizontalStrongEdges = 0;
  let verticalEdges = 0;
  let verticalStrongEdges = 0;
  let diagonalEdges = 0;
  let diagonalStrongEdges = 0;
  let colorStrongEdges = 0;

  for (let y = yStart; y < yEnd; y += 2) {
    for (let x = xStart; x < xEnd; x += 2) {
      const rgb = readRgb(image, x, y);
      const luma = luminance(rgb);
      saturationTotal += rgbSaturation(rgb);
      lumaTotal += luma;
      lumaSquaredTotal += luma * luma;
      colorBuckets.add(quantizeRgb(rgb));
      if (rgb[0] > rgb[2] * 1.12 && rgb[0] > rgb[1] * 0.82) {
        warmSampleCount += 1;
      }
      if (rgb[2] > rgb[0] * 1.08 && rgb[1] > rgb[0] * 0.86) {
        coolSampleCount += 1;
      }
      sampleCount += 1;

      if (x + 2 < xEnd) {
        horizontalEdges += 1;
        const other = readRgb(image, x + 2, y);
        if (Math.abs(luma - luminance(other)) >= 18) {
          horizontalStrongEdges += 1;
        }
        if (rgbDistance(rgb, other) >= 42) {
          colorStrongEdges += 1;
        }
      }
      if (y + 2 < yEnd) {
        verticalEdges += 1;
        const other = readRgb(image, x, y + 2);
        if (Math.abs(luma - luminance(other)) >= 18) {
          verticalStrongEdges += 1;
        }
        if (rgbDistance(rgb, other) >= 42) {
          colorStrongEdges += 1;
        }
      }
      if (x + 2 < xEnd && y + 2 < yEnd) {
        diagonalEdges += 1;
        const other = readRgb(image, x + 2, y + 2);
        if (Math.abs(luma - luminance(other)) >= 18) {
          diagonalStrongEdges += 1;
        }
      }
      if (x - 2 >= xStart && y + 2 < yEnd) {
        diagonalEdges += 1;
        const other = readRgb(image, x - 2, y + 2);
        if (Math.abs(luma - luminance(other)) >= 18) {
          diagonalStrongEdges += 1;
        }
      }
    }
  }

  const avgLuma = sampleCount === 0 ? 0 : lumaTotal / sampleCount;
  const lumaVariance = sampleCount === 0 ? 0 : Math.max(0, lumaSquaredTotal / sampleCount - avgLuma * avgLuma);
  const axisStrongEdges = horizontalStrongEdges + verticalStrongEdges;
  const axisEdges = horizontalEdges + verticalEdges;
  const axisEdgeRate = ratio(axisStrongEdges, axisEdges);
  const diagonalEdgeRate = ratio(diagonalStrongEdges, diagonalEdges);
  return {
    id: region.id,
    label: region.label,
    bounds: {
      x: [xStart, xEnd],
      y: [yStart, yEnd],
    },
    sampleCount,
    avgLuma,
    lumaStdDev: Math.sqrt(lumaVariance),
    avgSaturation: sampleCount === 0 ? 0 : saturationTotal / sampleCount,
    quantizedColorCount: colorBuckets.size,
    horizontalEdgeRate: ratio(horizontalStrongEdges, horizontalEdges),
    verticalEdgeRate: ratio(verticalStrongEdges, verticalEdges),
    axisEdgeRate,
    diagonalEdgeRate,
    axisAlignedEdgeDominance: axisEdgeRate / Math.max(0.001, diagonalEdgeRate),
    colorEdgeRate: ratio(colorStrongEdges, axisEdges),
    warmRatio: ratio(warmSampleCount, sampleCount),
    coolRatio: ratio(coolSampleCount, sampleCount),
  };
}

function diagnoseComposition(regions: readonly RegionMetrics[]): CompositionReport["diagnosis"] {
  const highestAxis = maxBy(regions, (region) => region.axisAlignedEdgeDominance);
  const highestColor = maxBy(regions, (region) => region.quantizedColorCount);
  const sky = regions.find((region) => region.id === "sky") ?? null;
  const lowerGround = regions.find((region) => region.id === "lower_ground") ?? null;
  const center = regions.find((region) => region.id === "center_view") ?? null;
  const likelyGridDriver = maxBy(
    regions.filter((region) => region.id !== "sky"),
    (region) => region.axisAlignedEdgeDominance * Math.max(0.01, region.axisEdgeRate),
  );
  return {
    highestAxisDominanceRegion: highestAxis?.id ?? null,
    highestAxisDominance: highestAxis?.axisAlignedEdgeDominance ?? 0,
    highestColorRegion: highestColor?.id ?? null,
    highestColorCount: highestColor?.quantizedColorCount ?? 0,
    likelyGridDriver: likelyGridDriver?.id ?? null,
    groundToSkyAxisRatio: sky && lowerGround ? lowerGround.axisAlignedEdgeDominance / Math.max(0.001, sky.axisAlignedEdgeDominance) : null,
    lowerGroundGridDominance: lowerGround?.axisAlignedEdgeDominance ?? null,
    centerGridDominance: center?.axisAlignedEdgeDominance ?? null,
  };
}

function buildMarkdownSummary(report: CompositionReport): string {
  return [
    "# Screenshot Composition Summary",
    "",
    `- Generated: ${report.generatedAt}`,
    `- Image: ${report.imagePath}`,
    `- Size: ${report.image.width}x${report.image.height}`,
    `- Likely grid driver: ${report.diagnosis.likelyGridDriver ?? "none"}`,
    `- Highest axis dominance: ${report.diagnosis.highestAxisDominanceRegion ?? "none"} ${report.diagnosis.highestAxisDominance.toFixed(2)}`,
    `- Ground/sky axis ratio: ${formatNullable(report.diagnosis.groundToSkyAxisRatio)}`,
    "",
    "| Region | Luma | Stddev | Sat | Colors | H Edge | V Edge | Axis | Diagonal | Axis/Diag | Color Edge | Warm | Cool |",
    "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
    ...report.regions.map((region) =>
      `| ${region.label} | ${region.avgLuma.toFixed(1)} | ${region.lumaStdDev.toFixed(1)} | ${region.avgSaturation.toFixed(2)} | ${region.quantizedColorCount} | ${formatPercent(region.horizontalEdgeRate)} | ${formatPercent(region.verticalEdgeRate)} | ${formatPercent(region.axisEdgeRate)} | ${formatPercent(region.diagonalEdgeRate)} | ${region.axisAlignedEdgeDominance.toFixed(2)} | ${formatPercent(region.colorEdgeRate)} | ${formatPercent(region.warmRatio)} | ${formatPercent(region.coolRatio)} |`
    ),
    "",
  ].join("\n");
}

async function findLatestOwnedBrowserScreenshot(outputDir: string): Promise<string | null> {
  let entries: string[];
  try {
    entries = await readdir(outputDir);
  } catch {
    return null;
  }
  const sorted = entries.filter((entry) => /^\d{8}T\d{6}Z/.test(entry)).sort((left, right) => right.localeCompare(left));
  for (const entry of sorted) {
    const candidate = join(outputDir, entry, "settled-page.png");
    try {
      await readFile(candidate);
      return candidate;
    } catch {
      // Keep looking for the most recent completed browser lab screenshot.
    }
  }
  return null;
}

function decodePng(bytes: Uint8Array): PngImage {
  const signature = [137, 80, 78, 71, 13, 10, 26, 10];
  for (let index = 0; index < signature.length; index += 1) {
    if (bytes[index] !== signature[index]) {
      throw new Error("not a PNG file");
    }
  }
  let offset = 8;
  let width = 0;
  let height = 0;
  let bitDepth = 0;
  let colorType = 0;
  const idatChunks: Uint8Array[] = [];
  while (offset + 8 <= bytes.length) {
    const length = readUInt32Be(bytes, offset);
    const type = String.fromCharCode(bytes[offset + 4] ?? 0, bytes[offset + 5] ?? 0, bytes[offset + 6] ?? 0, bytes[offset + 7] ?? 0);
    const dataStart = offset + 8;
    const dataEnd = dataStart + length;
    if (dataEnd + 4 > bytes.length) {
      throw new Error(`truncated PNG chunk ${type}`);
    }
    const data = bytes.subarray(dataStart, dataEnd);
    if (type === "IHDR") {
      width = readUInt32Be(data, 0);
      height = readUInt32Be(data, 4);
      bitDepth = data[8] ?? 0;
      colorType = data[9] ?? 0;
    } else if (type === "IDAT") {
      idatChunks.push(data);
    } else if (type === "IEND") {
      break;
    }
    offset = dataEnd + 4;
  }
  if (width <= 0 || height <= 0) {
    throw new Error("PNG missing IHDR");
  }
  if (bitDepth !== 8 || (colorType !== 2 && colorType !== 6)) {
    throw new Error(`unsupported PNG format bitDepth=${bitDepth} colorType=${colorType}`);
  }
  const sourceBytesPerPixel = colorType === 6 ? 4 : 3;
  const scanlineLength = width * sourceBytesPerPixel;
  const raw = new Uint8Array(inflateSync(concatUint8Arrays(idatChunks)));
  const unfiltered = new Uint8Array(scanlineLength * height);
  let sourceOffset = 0;
  let targetOffset = 0;
  for (let y = 0; y < height; y += 1) {
    const filter = raw[sourceOffset] ?? 0;
    sourceOffset += 1;
    for (let x = 0; x < scanlineLength; x += 1) {
      const rawValue = raw[sourceOffset + x] ?? 0;
      const left = x >= sourceBytesPerPixel ? unfiltered[targetOffset + x - sourceBytesPerPixel] ?? 0 : 0;
      const up = y > 0 ? unfiltered[targetOffset + x - scanlineLength] ?? 0 : 0;
      const upLeft = y > 0 && x >= sourceBytesPerPixel ? unfiltered[targetOffset + x - scanlineLength - sourceBytesPerPixel] ?? 0 : 0;
      unfiltered[targetOffset + x] = unfilterPngByte(filter, rawValue, left, up, upLeft);
    }
    sourceOffset += scanlineLength;
    targetOffset += scanlineLength;
  }
  const pixels = new Uint8Array(width * height * 4);
  for (let index = 0; index < width * height; index += 1) {
    const src = index * sourceBytesPerPixel;
    const dst = index * 4;
    pixels[dst] = unfiltered[src] ?? 0;
    pixels[dst + 1] = unfiltered[src + 1] ?? 0;
    pixels[dst + 2] = unfiltered[src + 2] ?? 0;
    pixels[dst + 3] = colorType === 6 ? unfiltered[src + 3] ?? 255 : 255;
  }
  return { width, height, pixels };
}

function unfilterPngByte(filter: number, rawValue: number, left: number, up: number, upLeft: number): number {
  switch (filter) {
    case 0:
      return rawValue;
    case 1:
      return (rawValue + left) & 0xff;
    case 2:
      return (rawValue + up) & 0xff;
    case 3:
      return (rawValue + Math.floor((left + up) / 2)) & 0xff;
    case 4:
      return (rawValue + paethPredictor(left, up, upLeft)) & 0xff;
    default:
      throw new Error(`unsupported PNG filter ${filter}`);
  }
}

function paethPredictor(left: number, up: number, upLeft: number): number {
  const estimate = left + up - upLeft;
  const leftDistance = Math.abs(estimate - left);
  const upDistance = Math.abs(estimate - up);
  const upLeftDistance = Math.abs(estimate - upLeft);
  if (leftDistance <= upDistance && leftDistance <= upLeftDistance) {
    return left;
  }
  return upDistance <= upLeftDistance ? up : upLeft;
}

function readRgb(image: PngImage, x: number, y: number): [number, number, number] {
  const index = (y * image.width + x) * 4;
  return [image.pixels[index] ?? 0, image.pixels[index + 1] ?? 0, image.pixels[index + 2] ?? 0];
}

function luminance(rgb: readonly [number, number, number]): number {
  return rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722;
}

function rgbSaturation(rgb: readonly [number, number, number]): number {
  const max = Math.max(rgb[0], rgb[1], rgb[2]);
  const min = Math.min(rgb[0], rgb[1], rgb[2]);
  return max === 0 ? 0 : (max - min) / max;
}

function rgbDistance(left: readonly [number, number, number], right: readonly [number, number, number]): number {
  return Math.hypot(left[0] - right[0], left[1] - right[1], left[2] - right[2]);
}

function quantizeRgb(rgb: readonly [number, number, number]): number {
  return ((rgb[0] >> 4) << 8) | ((rgb[1] >> 4) << 4) | (rgb[2] >> 4);
}

function concatUint8Arrays(chunks: readonly Uint8Array[]): Uint8Array {
  const output = new Uint8Array(chunks.reduce((total, chunk) => total + chunk.length, 0));
  let offset = 0;
  for (const chunk of chunks) {
    output.set(chunk, offset);
    offset += chunk.length;
  }
  return output;
}

function readUInt32Be(bytes: Uint8Array, offset: number): number {
  return (((bytes[offset] ?? 0) * 0x1000000) + ((bytes[offset + 1] ?? 0) << 16) + ((bytes[offset + 2] ?? 0) << 8) + (bytes[offset + 3] ?? 0)) >>> 0;
}

function maxBy<T>(values: readonly T[], score: (value: T) => number): T | null {
  let best: T | null = null;
  let bestScore = -Infinity;
  for (const value of values) {
    const valueScore = score(value);
    if (valueScore > bestScore) {
      best = value;
      bestScore = valueScore;
    }
  }
  return best;
}

function ratio(numerator: number, denominator: number): number {
  return denominator === 0 ? 0 : numerator / denominator;
}

function readFlag(args: readonly string[], name: string): string | null {
  const equalsPrefix = `${name}=`;
  const equalsValue = args.find((arg) => arg.startsWith(equalsPrefix));
  if (equalsValue) {
    return equalsValue.slice(equalsPrefix.length);
  }
  const index = args.indexOf(name);
  return index >= 0 ? args[index + 1] ?? null : null;
}

function timestampForFile(date: Date): string {
  return date.toISOString().replace(/[-:]/g, "").replace(/\.\d{3}Z$/, "Z");
}

function sanitizeFileStem(value: string): string {
  return value.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || "run";
}

function formatPercent(value: number): string {
  return `${(value * 100).toFixed(1)}%`;
}

function formatNullable(value: number | null): string {
  return value === null ? "n/a" : value.toFixed(2);
}
