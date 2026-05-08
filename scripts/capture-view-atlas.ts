import { readdir, readFile } from "node:fs/promises";
import { join } from "node:path";
import { inflateSync } from "node:zlib";

import {
  readGitShortHead,
  sanitizeFileStem,
  timestampForFile,
  withBrowserGameSession,
} from "./lib/browser-game-benchmark-harness.ts";
import { metersToWorldUnits } from "../src/engine/scale.ts";

interface ViewSpec {
  id: string;
  label: string;
  eyeMeters: readonly [number, number];
  lookAtMeters: readonly [number, number];
  pitchDegrees: number;
}

interface CliOptions {
  label: string | null;
  outputDir: string;
  compareTo: string | null;
  selectedViewIds: string[] | null;
  chromeBinary?: string;
  headless: boolean;
  appPort: number | null;
  viewportWidth: number;
  viewportHeight: number;
  skipBuild: boolean;
  settleMaxFrames: number;
}

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
  axisEdgeRate: number;
  diagonalEdgeRate: number;
  axisAlignedEdgeDominance: number;
  colorEdgeRate: number;
  gridRiskScore: number;
}

interface ViewCapture {
  id: string;
  label: string;
  screenshotPath: string;
  screenshotRelativePath: string;
  eyeMeters: readonly [number, number];
  lookAtMeters: readonly [number, number];
  yawDegrees: number;
  pitchDegrees: number;
  settled: Record<string, unknown>;
  snapshot: Record<string, unknown>;
  visual: {
    width: number;
    height: number;
    regions: RegionMetrics[];
    diagnosis: {
      avgLuma: number;
      lumaStdDev: number;
      quantizedColorCount: number;
      centerGridRiskScore: number;
      lowerGroundGridRiskScore: number;
      blankish: boolean;
    };
  };
}

interface ViewAtlasReport {
  generatedAt: string;
  label: string | null;
  commit: string | null;
  appUrl: string;
  outputDir: string;
  summaryPath: string;
  compareTo: string | null;
  comparison: ViewAtlasComparison | null;
  options: Omit<CliOptions, "chromeBinary" | "selectedViewIds"> & {
    selectedViewIds: string[] | null;
    chromeBinary: string | null;
  };
  views: ViewCapture[];
  failures: string[];
}

interface ViewAtlasComparison {
  baselinePath: string;
  baselineGeneratedAt: string | null;
  viewDeltas: Array<{
    id: string;
    avgLuma: number;
    lumaStdDev: number;
    quantizedColorCount: number;
    centerGridRiskScore: number;
    lowerGroundGridRiskScore: number;
  }>;
}

const VIEW_SPECS: ViewSpec[] = [
  {
    id: "origin-overlook",
    label: "Origin Overlook",
    eyeMeters: [0, 0],
    lookAtMeters: [90, 12],
    pitchDegrees: -18,
  },
  {
    id: "ancestor-pillar-vista",
    label: "Ancestor Pillar Vista",
    eyeMeters: [66.4, -2057.6],
    lookAtMeters: [66.4, -1997.6],
    pitchDegrees: -16,
  },
  {
    id: "ash-marker-vista",
    label: "Ash Marker Vista",
    eyeMeters: [236, -4664],
    lookAtMeters: [236, -4604],
    pitchDegrees: -16,
  },
  {
    id: "silt-shell-vista",
    label: "Silt Shell Vista",
    eyeMeters: [-1960, -2323.2],
    lookAtMeters: [-1900, -2323.2],
    pitchDegrees: -15,
  },
  {
    id: "ziggurat-vista",
    label: "Ziggurat Vista",
    eyeMeters: [-1764.8, -2536.9],
    lookAtMeters: [-1704.8, -2536.9],
    pitchDegrees: -14,
  },
];

const options = parseCli(Bun.argv);
const selectedViews = selectViews(options.selectedViewIds);
const runStamp = timestampForFile(new Date());
const runName = `${runStamp}${options.label ? `-${sanitizeFileStem(options.label)}` : ""}`;
const runDir = join(options.outputDir, runName);
const screenshotsDir = join(runDir, "screenshots");
const reportPath = join(runDir, "report.json");
const summaryPath = join(runDir, "summary.md");

await Bun.$`mkdir -p ${screenshotsDir}`.quiet();

const baselinePath = options.compareTo ?? await findPreviousReportPath(options.outputDir, runName);
let appUrl = "";
const captures = await withBrowserGameSession(
  {
    chromeBinary: options.chromeBinary,
    headless: options.headless,
    appPort: options.appPort,
    viewportWidth: options.viewportWidth,
    viewportHeight: options.viewportHeight,
    skipBuild: options.skipBuild,
    outputDir: runDir,
  },
  async (session) => {
    appUrl = session.appUrl;
    await session.navigateToGame({
      clearStorage: true,
      query: { benchmarkBootstrap: 1 },
    });
    await session.waitForGameReady(120_000);

    const viewCaptures: ViewCapture[] = [];
    for (const view of selectedViews) {
      console.log(`[view-atlas] capturing ${view.id}`);
      const yawDegrees = yawDegreesForView(view);
      const settled = await session.evaluate<Record<string, unknown>>(buildSettleViewExpression(view, yawDegrees, options.settleMaxFrames));
      const screenshotBytes = await session.captureScreenshotPng();
      const screenshotPath = join(screenshotsDir, `${view.id}.png`);
      await Bun.write(screenshotPath, screenshotBytes);
      const image = decodePng(screenshotBytes);
      const regions = buildRegionSpecs().map((region) => analyzeRegion(image, region));
      const snapshot = readObject(settled.snapshot);
      viewCaptures.push({
        id: view.id,
        label: view.label,
        screenshotPath,
        screenshotRelativePath: `screenshots/${view.id}.png`,
        eyeMeters: view.eyeMeters,
        lookAtMeters: view.lookAtMeters,
        yawDegrees,
        pitchDegrees: view.pitchDegrees,
        settled: readObject(settled.settled),
        snapshot,
        visual: {
          width: image.width,
          height: image.height,
          regions,
          diagnosis: diagnoseVisual(regions),
        },
      });
    }
    return viewCaptures;
  },
);

const comparison = baselinePath ? await compareWithBaseline(baselinePath, captures) : null;
const failures = captures
  .filter((capture) => capture.visual.diagnosis.blankish)
  .map((capture) => `${capture.id} looks blankish: luma=${capture.visual.diagnosis.avgLuma.toFixed(1)}, colors=${capture.visual.diagnosis.quantizedColorCount}`);
const report: ViewAtlasReport = {
  generatedAt: new Date().toISOString(),
  label: options.label,
  commit: readGitShortHead(),
  appUrl,
  outputDir: runDir,
  summaryPath,
  compareTo: baselinePath,
  comparison,
  options: {
    ...options,
    selectedViewIds: options.selectedViewIds,
    chromeBinary: options.chromeBinary ?? null,
  },
  views: captures,
  failures,
};

await Bun.write(reportPath, `${JSON.stringify(report, null, 2)}\n`);
await Bun.write(summaryPath, buildMarkdownSummary(report));

console.log(`view atlas report: ${reportPath}`);
console.log(`summary: ${summaryPath}`);
console.log(`screenshots: ${screenshotsDir}`);
console.log(`views: ${captures.map((capture) => capture.id).join(", ")}`);
console.log(`failures: ${failures.length > 0 ? failures.join("; ") : "none"}`);
if (failures.length > 0) {
  process.exitCode = 1;
}

function buildSettleViewExpression(view: ViewSpec, yawDegrees: number, settleMaxFrames: number): string {
  const worldX = metersToWorldUnits(view.eyeMeters[0]);
  const worldZ = metersToWorldUnits(view.eyeMeters[1]);
  const yawRadians = yawDegrees * Math.PI / 180;
  const pitchRadians = view.pitchDegrees * Math.PI / 180;
  return `(${async function settleView(
    rawWorldX: number,
    rawWorldZ: number,
    rawYaw: number,
    rawPitch: number,
    rawSettleMaxFrames: number,
  ) {
    const game = window.__VOXELS_GAME__;
    if (!game) {
      throw new Error("window.__VOXELS_GAME__ is not ready");
    }
    const surfaceY = game.controller.generator.sampleColumn(rawWorldX, rawWorldZ).surfaceY;
    const eyeY = surfaceY + game.controller.player.eyeHeight;
    const settled = await game.setCameraPoseAndSettle(rawWorldX, eyeY, rawWorldZ, rawYaw, rawPitch, {
      radiusChunks: game.snapshot().residencyRadiusChunks,
      maxFrames: rawSettleMaxFrames,
    });
    await new Promise((resolve) => requestAnimationFrame(() => resolve(null)));
    await new Promise((resolve) => requestAnimationFrame(() => resolve(null)));
    return {
      settled,
      snapshot: game.snapshot(),
    };
  }})(${JSON.stringify(worldX)}, ${JSON.stringify(worldZ)}, ${JSON.stringify(yawRadians)}, ${JSON.stringify(pitchRadians)}, ${JSON.stringify(settleMaxFrames)})`;
}

function buildRegionSpecs(): RegionSpec[] {
  return [
    { id: "full", label: "Full", x0: 0.06, y0: 0.08, x1: 0.94, y1: 0.90 },
    { id: "sky", label: "Sky", x0: 0.06, y0: 0.02, x1: 0.94, y1: 0.30 },
    { id: "horizon", label: "Horizon", x0: 0.06, y0: 0.25, x1: 0.94, y1: 0.52 },
    { id: "center", label: "Center", x0: 0.22, y0: 0.25, x1: 0.78, y1: 0.68 },
    { id: "lower_ground", label: "Lower Ground", x0: 0.06, y0: 0.55, x1: 0.94, y1: 0.92 },
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

  const avgLuma = ratio(lumaTotal, sampleCount);
  const lumaVariance = sampleCount === 0 ? 0 : Math.max(0, lumaSquaredTotal / sampleCount - avgLuma * avgLuma);
  const axisStrongEdges = horizontalStrongEdges + verticalStrongEdges;
  const axisEdges = horizontalEdges + verticalEdges;
  const axisEdgeRate = ratio(axisStrongEdges, axisEdges);
  const diagonalEdgeRate = ratio(diagonalStrongEdges, diagonalEdges);
  const axisAlignedEdgeDominance = axisEdgeRate / Math.max(0.001, diagonalEdgeRate);
  const colorEdgeRate = ratio(colorStrongEdges, axisEdges);
  return {
    id: region.id,
    label: region.label,
    bounds: {
      x: [xStart, xEnd],
      y: [yStart, yEnd],
    },
    sampleCount,
    avgLuma: roundMetric(avgLuma),
    lumaStdDev: roundMetric(Math.sqrt(lumaVariance)),
    avgSaturation: roundMetric(ratio(saturationTotal, sampleCount)),
    quantizedColorCount: colorBuckets.size,
    axisEdgeRate: roundMetric(axisEdgeRate),
    diagonalEdgeRate: roundMetric(diagonalEdgeRate),
    axisAlignedEdgeDominance: roundMetric(axisAlignedEdgeDominance),
    colorEdgeRate: roundMetric(colorEdgeRate),
    gridRiskScore: roundMetric(axisAlignedEdgeDominance * axisEdgeRate),
  };
}

function diagnoseVisual(regions: readonly RegionMetrics[]): ViewCapture["visual"]["diagnosis"] {
  const full = regionById(regions, "full");
  const center = regionById(regions, "center");
  const lowerGround = regionById(regions, "lower_ground");
  const avgLuma = full?.avgLuma ?? 0;
  const lumaStdDev = full?.lumaStdDev ?? 0;
  const quantizedColorCount = full?.quantizedColorCount ?? 0;
  return {
    avgLuma,
    lumaStdDev,
    quantizedColorCount,
    centerGridRiskScore: center?.gridRiskScore ?? 0,
    lowerGroundGridRiskScore: lowerGround?.gridRiskScore ?? 0,
    blankish: avgLuma < 8 || lumaStdDev < 6 || quantizedColorCount < 8,
  };
}

function buildMarkdownSummary(report: ViewAtlasReport): string {
  const lines = [
    "# View Atlas Summary",
    "",
    `Generated: ${report.generatedAt}`,
    `Commit: ${report.commit ?? "unknown"}`,
    `Output: ${report.outputDir}`,
    report.compareTo ? `Compared to: ${report.compareTo}` : "Compared to: none",
    "",
    "| View | Screenshot | Luma | Stddev | Colors | Center grid | Ground grid | Blankish |",
    "| --- | --- | ---: | ---: | ---: | ---: | ---: | --- |",
    ...report.views.map((view) => {
      const diagnosis = view.visual.diagnosis;
      return `| ${view.label} | ![${view.id}](${view.screenshotRelativePath}) | ${diagnosis.avgLuma.toFixed(1)} | ${diagnosis.lumaStdDev.toFixed(1)} | ${diagnosis.quantizedColorCount} | ${diagnosis.centerGridRiskScore.toFixed(3)} | ${diagnosis.lowerGroundGridRiskScore.toFixed(3)} | ${diagnosis.blankish ? "yes" : "no"} |`;
    }),
    "",
  ];
  if (report.comparison) {
    lines.push(
      "## Deltas",
      "",
      "| View | Luma | Stddev | Colors | Center grid | Ground grid |",
      "| --- | ---: | ---: | ---: | ---: | ---: |",
      ...report.comparison.viewDeltas.map((delta) =>
        `| ${delta.id} | ${formatSigned(delta.avgLuma, 1)} | ${formatSigned(delta.lumaStdDev, 1)} | ${formatSigned(delta.quantizedColorCount)} | ${formatSigned(delta.centerGridRiskScore, 3)} | ${formatSigned(delta.lowerGroundGridRiskScore, 3)} |`
      ),
      "",
    );
  }
  lines.push(
    "## Regions",
    "",
    "| View | Region | Luma | Stddev | Sat | Colors | Axis | Diag | Axis/Diag | Color edge | Grid score |",
    "| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
    ...report.views.flatMap((view) => view.visual.regions.map((region) =>
      `| ${view.id} | ${region.label} | ${region.avgLuma.toFixed(1)} | ${region.lumaStdDev.toFixed(1)} | ${region.avgSaturation.toFixed(3)} | ${region.quantizedColorCount} | ${formatPercent(region.axisEdgeRate)} | ${formatPercent(region.diagonalEdgeRate)} | ${region.axisAlignedEdgeDominance.toFixed(2)} | ${formatPercent(region.colorEdgeRate)} | ${region.gridRiskScore.toFixed(3)} |`
    )),
    "",
  );
  if (report.failures.length > 0) {
    lines.push("## Failures", "", ...report.failures.map((failure) => `- ${failure}`), "");
  }
  return lines.join("\n");
}

async function compareWithBaseline(
  baselinePath: string,
  captures: readonly ViewCapture[],
): Promise<ViewAtlasComparison | null> {
  let baseline: Partial<ViewAtlasReport>;
  try {
    baseline = JSON.parse(await readFile(baselinePath, "utf8")) as Partial<ViewAtlasReport>;
  } catch {
    return null;
  }
  if (!Array.isArray(baseline.views)) {
    return null;
  }
  const baselineViews = new Map(baseline.views.map((view) => [view.id, view]));
  return {
    baselinePath,
    baselineGeneratedAt: typeof baseline.generatedAt === "string" ? baseline.generatedAt : null,
    viewDeltas: captures.map((view) => {
      const baselineView = baselineViews.get(view.id);
      return {
        id: view.id,
        avgLuma: view.visual.diagnosis.avgLuma - readNumber(baselineView?.visual?.diagnosis?.avgLuma),
        lumaStdDev: view.visual.diagnosis.lumaStdDev - readNumber(baselineView?.visual?.diagnosis?.lumaStdDev),
        quantizedColorCount: view.visual.diagnosis.quantizedColorCount - readNumber(baselineView?.visual?.diagnosis?.quantizedColorCount),
        centerGridRiskScore: view.visual.diagnosis.centerGridRiskScore - readNumber(baselineView?.visual?.diagnosis?.centerGridRiskScore),
        lowerGroundGridRiskScore: view.visual.diagnosis.lowerGroundGridRiskScore - readNumber(baselineView?.visual?.diagnosis?.lowerGroundGridRiskScore),
      };
    }),
  };
}

async function findPreviousReportPath(outputDir: string, currentRunName: string): Promise<string | null> {
  let entries: string[];
  try {
    entries = await readdir(outputDir);
  } catch {
    return null;
  }
  const previousRunNames = entries
    .filter((entry) => entry !== currentRunName)
    .filter((entry) => /^\d{8}T\d{6}Z/.test(entry))
    .sort((left, right) => right.localeCompare(left));
  for (const previousRunName of previousRunNames) {
    const candidate = join(outputDir, previousRunName, "report.json");
    try {
      const parsed = JSON.parse(await readFile(candidate, "utf8")) as Partial<ViewAtlasReport>;
      if (Array.isArray(parsed.views)) {
        return candidate;
      }
    } catch {
      // Keep looking for the most recent completed atlas report.
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

function parseCli(argv: readonly string[]): CliOptions {
  const args = argv.slice(2);
  return {
    label: readFlag(args, "--label"),
    outputDir: readFlag(args, "--output-dir") ?? "artifacts/view-atlas",
    compareTo: readFlag(args, "--compare-to"),
    selectedViewIds: readListFlag(args, "--views"),
    chromeBinary: readFlag(args, "--chrome-binary") ?? undefined,
    headless: !readBooleanFlag(args, "--headed", false),
    appPort: readOptionalPositiveInt(readFlag(args, "--app-port")),
    viewportWidth: readPositiveInt(readFlag(args, "--viewport-width"), 1440),
    viewportHeight: readPositiveInt(readFlag(args, "--viewport-height"), 900),
    skipBuild: readBooleanFlag(args, "--skip-build", false),
    settleMaxFrames: readPositiveInt(readFlag(args, "--settle-max-frames"), 240),
  };
}

function selectViews(selectedViewIds: readonly string[] | null): ViewSpec[] {
  if (!selectedViewIds) {
    return VIEW_SPECS;
  }
  const viewsById = new Map(VIEW_SPECS.map((view) => [view.id, view]));
  return selectedViewIds.map((id) => {
    const view = viewsById.get(id);
    if (!view) {
      throw new Error(`Unknown view "${id}". Available: ${VIEW_SPECS.map((spec) => spec.id).join(", ")}`);
    }
    return view;
  });
}

function yawDegreesForView(view: ViewSpec): number {
  const dx = view.lookAtMeters[0] - view.eyeMeters[0];
  const dz = view.lookAtMeters[1] - view.eyeMeters[1];
  return Math.atan2(dz, dx) * 180 / Math.PI;
}

function regionById(regions: readonly RegionMetrics[], id: string): RegionMetrics | null {
  return regions.find((region) => region.id === id) ?? null;
}

function readObject(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : {};
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

function readListFlag(args: readonly string[], name: string): string[] | null {
  const value = readFlag(args, name);
  if (!value) {
    return null;
  }
  return value.split(",").map((entry) => entry.trim()).filter(Boolean);
}

function readBooleanFlag(args: readonly string[], name: string, fallback: boolean): boolean {
  const value = readFlag(args, name);
  if (value === null) {
    return fallback;
  }
  return value === "1" || value === "true" || value === "yes";
}

function readOptionalPositiveInt(value: string | null): number | null {
  if (value === null) {
    return null;
  }
  return readPositiveInt(value, 0);
}

function readPositiveInt(value: string | null, fallback: number): number {
  if (value === null) {
    return fallback;
  }
  const parsed = Number(value);
  if (!Number.isFinite(parsed) || parsed <= 0) {
    return fallback;
  }
  return Math.floor(parsed);
}

function readNumber(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

function ratio(numerator: number, denominator: number): number {
  return denominator === 0 ? 0 : numerator / denominator;
}

function roundMetric(value: number): number {
  return Number(value.toFixed(4));
}

function formatPercent(value: number): string {
  return `${(value * 100).toFixed(1)}%`;
}

function formatSigned(value: number, digits = 0): string {
  const prefix = value > 0 ? "+" : "";
  return `${prefix}${value.toFixed(digits)}`;
}
