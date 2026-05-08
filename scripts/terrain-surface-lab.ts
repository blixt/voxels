import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";

import {
  materialToHexColor,
  ProceduralWorldGenerator,
  type ProceduralBiomeProbe,
} from "../src/engine/procedural-generator.ts";
import { metersToWorldUnits, worldUnitsToMeters } from "../src/engine/scale.ts";
import { sanitizeFileStem, timestampForFile } from "./lib/browser-game-benchmark-harness.ts";

export interface TerrainSurfaceLabOptions {
  seed?: number;
  outputDir?: string;
  label?: string;
  timestamp?: Date;
  radiusMeters?: number;
  stepMeters?: number;
  patchIds?: readonly string[];
}

export interface TerrainPatchSpec {
  id: string;
  label: string;
  source: "view-atlas" | "route-atlas";
  centerMeters: readonly [number, number];
  focusLandmarkIds: readonly string[];
}

export interface TerrainSurfaceSample {
  gridX: number;
  gridZ: number;
  worldX: number;
  worldZ: number;
  surfaceY: number;
  surfaceMaterial: number;
  biomeId: string;
  undergroundBiomeId: string;
  regionalVariantId: string | null;
  landmarkId: string | null;
}

export interface TerrainMaterialDiversity {
  distinctMaterialCount: number;
  entropy: number;
  normalizedEntropy: number;
  dominantMaterial: {
    material: number;
    hex: string;
    count: number;
    share: number;
  } | null;
  materialCounts: Array<{
    material: number;
    hex: string;
    count: number;
    share: number;
  }>;
}

export interface TerrainHeightModuloDistribution {
  modulo: number;
  buckets: Array<{
    remainder: number;
    count: number;
    share: number;
  }>;
}

export interface TerrainFlatnessDistribution {
  averageSlope: number;
  maxSlope: number;
  buckets: Array<{
    id: "flat" | "gentle" | "rolling" | "steep" | "cliff";
    label: string;
    minSlope: number;
    maxSlope: number | null;
    count: number;
    share: number;
  }>;
}

export interface TerrainGridLikeness {
  score: number;
  axisEdgeRate: number;
  diagonalEdgeRate: number;
  axisAlignedEdgeDominance: number;
  heightAxisEdgeRate: number;
  materialAxisEdgeRate: number;
  axisEdgeCount: number;
  diagonalEdgeCount: number;
}

export interface TerrainPatchAnalysis {
  id: string;
  label: string;
  source: TerrainPatchSpec["source"];
  centerMeters: readonly [number, number];
  centerWorld: readonly [number, number];
  radiusMeters: number;
  stepMeters: number;
  gridSize: {
    width: number;
    height: number;
  };
  sampleCount: number;
  surfaceMeters: {
    min: number;
    max: number;
    mean: number;
    range: number;
  };
  distinctBiomes: string[];
  distinctUndergroundBiomes: string[];
  distinctRegionalVariants: string[];
  directLandmarks: string[];
  focusLandmarkIds: string[];
  missingFocusLandmarkIds: string[];
  materialDiversity: TerrainMaterialDiversity;
  heightModulo: TerrainHeightModuloDistribution;
  flatness: TerrainFlatnessDistribution;
  gridLikeness: TerrainGridLikeness;
  warnings: string[];
}

export interface TerrainSurfaceLabReport {
  generatedAt: string;
  label: string;
  seed: number;
  runDir: string;
  options: {
    radiusMeters: number;
    stepMeters: number;
    selectedPatchIds: string[] | null;
  };
  aggregate: {
    patchCount: number;
    sampleCount: number;
    distinctBiomes: string[];
    distinctLandmarks: string[];
    averageMaterialCount: number;
    averageGridLikenessScore: number;
    maxGridLikenessScore: number;
    maxSurfaceRangeMeters: number;
    warningCount: number;
  };
  patches: TerrainPatchAnalysis[];
  artifacts: {
    report: string;
    summary: string;
  };
}

const DEFAULT_SEED = 1337;
const DEFAULT_OUTPUT_DIR = "artifacts/terrain-lab";
const DEFAULT_RADIUS_METERS = 72;
const DEFAULT_STEP_METERS = 4;
const HEIGHT_MODULO = 8;

const FLATNESS_BUCKETS: TerrainFlatnessDistribution["buckets"] = [
  { id: "flat", label: "Flat", minSlope: 0, maxSlope: 0.08, count: 0, share: 0 },
  { id: "gentle", label: "Gentle", minSlope: 0.08, maxSlope: 0.25, count: 0, share: 0 },
  { id: "rolling", label: "Rolling", minSlope: 0.25, maxSlope: 0.60, count: 0, share: 0 },
  { id: "steep", label: "Steep", minSlope: 0.60, maxSlope: 1.20, count: 0, share: 0 },
  { id: "cliff", label: "Cliff", minSlope: 1.20, maxSlope: null, count: 0, share: 0 },
];

export const TERRAIN_SURFACE_PATCHES: TerrainPatchSpec[] = [
  {
    id: "ash-marker-vista",
    label: "Ash Marker Vista",
    source: "view-atlas",
    centerMeters: [236, -4604],
    focusLandmarkIds: ["ash_marker"],
  },
  {
    id: "ziggurat-vista",
    label: "Ziggurat Vista",
    source: "view-atlas",
    centerMeters: [-1704.8, -2536.9],
    focusLandmarkIds: ["velothi_ziggurat"],
  },
  {
    id: "ziggurat-approach",
    label: "Ziggurat Approach",
    source: "view-atlas",
    centerMeters: [-1064.5, -1584.6],
    focusLandmarkIds: ["velothi_ziggurat", "ash_obelisk"],
  },
  {
    id: "causeway-road",
    label: "Old Road Causeway",
    source: "route-atlas",
    centerMeters: [19.6, -3277.1],
    focusLandmarkIds: ["old_road_causeway"],
  },
  {
    id: "pilgrim-lantern-road",
    label: "Pilgrim Lantern Road",
    source: "route-atlas",
    centerMeters: [18.6, -3245.3],
    focusLandmarkIds: ["pilgrim_lantern", "old_road_causeway"],
  },
  {
    id: "ash-obelisk-road",
    label: "Ash Obelisk Road",
    source: "route-atlas",
    centerMeters: [-1658.1, -2508.8],
    focusLandmarkIds: ["ash_obelisk", "velothi_ziggurat"],
  },
  {
    id: "rib-arch-road",
    label: "Rib Arch Road",
    source: "route-atlas",
    centerMeters: [-1171.7, -2546.4],
    focusLandmarkIds: ["rib_arch"],
  },
] as const;

if (import.meta.main) {
  try {
    const options = readCliOptions(Bun.argv.slice(2));
    const report = await runTerrainSurfaceLab(options);
    console.log(`terrain surface lab report: ${report.artifacts.report}`);
    console.log(`summary: ${report.artifacts.summary}`);
    console.log(`patches: ${report.patches.map((patch) => patch.id).join(", ")}`);
    console.log(`avg grid-likeness: ${report.aggregate.averageGridLikenessScore.toFixed(3)}`);
    console.log(`warnings: ${report.aggregate.warningCount}`);
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exit(1);
  }
}

export async function runTerrainSurfaceLab(options: TerrainSurfaceLabOptions = {}): Promise<TerrainSurfaceLabReport> {
  const timestamp = options.timestamp ?? new Date();
  const label = sanitizeFileStem(options.label ?? "surface-probes");
  const runName = `${timestampForFile(timestamp)}-${label}`;
  const runDir = join(options.outputDir ?? DEFAULT_OUTPUT_DIR, runName);
  const reportPath = join(runDir, "report.json");
  const summaryPath = join(runDir, "summary.md");
  const seed = options.seed ?? DEFAULT_SEED;
  const radiusMeters = options.radiusMeters ?? DEFAULT_RADIUS_METERS;
  const stepMeters = options.stepMeters ?? DEFAULT_STEP_METERS;
  const selectedPatchIds = options.patchIds ? [...options.patchIds] : null;
  const patchSpecs = selectPatchSpecs(selectedPatchIds);
  const generator = new ProceduralWorldGenerator(seed);

  await mkdir(runDir, { recursive: true });

  const patches = patchSpecs.map((patch) => analyzeTerrainPatch(generator, patch, { radiusMeters, stepMeters }));
  const report: TerrainSurfaceLabReport = {
    generatedAt: timestamp.toISOString(),
    label,
    seed,
    runDir,
    options: {
      radiusMeters,
      stepMeters,
      selectedPatchIds,
    },
    aggregate: summarizeAggregate(patches),
    patches,
    artifacts: {
      report: reportPath,
      summary: summaryPath,
    },
  };

  await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`);
  await writeFile(summaryPath, buildMarkdownSummary(report));
  return report;
}

export function analyzeTerrainPatch(
  generator: ProceduralWorldGenerator,
  patch: TerrainPatchSpec,
  options: {
    radiusMeters: number;
    stepMeters: number;
  },
): TerrainPatchAnalysis {
  const samples = sampleTerrainPatch(generator, patch, options);
  return analyzeSurfaceGrid(patch, samples, options);
}

export function sampleTerrainPatch(
  generator: ProceduralWorldGenerator,
  patch: TerrainPatchSpec,
  options: {
    radiusMeters: number;
    stepMeters: number;
  },
): TerrainSurfaceSample[] {
  validatePositiveNumber(options.radiusMeters, "radiusMeters");
  validatePositiveNumber(options.stepMeters, "stepMeters");
  const centerWorldX = metersToWorldUnits(patch.centerMeters[0]);
  const centerWorldZ = metersToWorldUnits(patch.centerMeters[1]);
  const radiusWorldUnits = metersToWorldUnits(options.radiusMeters);
  const stepWorldUnits = metersToWorldUnits(options.stepMeters);
  const samples: TerrainSurfaceSample[] = [];
  let gridZ = 0;
  for (let dz = -radiusWorldUnits; dz <= radiusWorldUnits + 0.001; dz += stepWorldUnits) {
    let gridX = 0;
    for (let dx = -radiusWorldUnits; dx <= radiusWorldUnits + 0.001; dx += stepWorldUnits) {
      const worldX = Math.round(centerWorldX + dx);
      const worldZ = Math.round(centerWorldZ + dz);
      const probe = generator.sampleBiomeProbe(worldX, worldZ);
      const column = generator.sampleColumn(worldX, worldZ);
      samples.push(buildSurfaceSample(gridX, gridZ, worldX, worldZ, column.surfaceY, column.surfaceMaterial, probe));
      gridX += 1;
    }
    gridZ += 1;
  }
  return samples;
}

export function analyzeSurfaceGrid(
  patch: TerrainPatchSpec,
  samples: readonly TerrainSurfaceSample[],
  options: {
    radiusMeters: number;
    stepMeters: number;
  },
): TerrainPatchAnalysis {
  if (samples.length === 0) {
    throw new Error("Cannot analyze an empty surface grid.");
  }
  const width = Math.max(...samples.map((sample) => sample.gridX)) + 1;
  const height = Math.max(...samples.map((sample) => sample.gridZ)) + 1;
  if (width * height !== samples.length) {
    throw new Error(`Surface grid is incomplete: width=${width}, height=${height}, samples=${samples.length}`);
  }
  const surfacesMeters = samples.map((sample) => worldUnitsToMeters(sample.surfaceY));
  const directLandmarks = sortedDistinct(samples.map((sample) => sample.landmarkId).filter(isString));
  const materialDiversity = summarizeMaterialDiversity(samples);
  const flatness = summarizeFlatness(samples, width, height, options.stepMeters);
  const gridLikeness = summarizeGridLikeness(samples, width, height);
  const warnings = buildPatchWarnings({
    materialDiversity,
    flatness,
    gridLikeness,
    directLandmarks,
    focusLandmarkIds: patch.focusLandmarkIds,
  });
  return {
    id: patch.id,
    label: patch.label,
    source: patch.source,
    centerMeters: patch.centerMeters,
    centerWorld: [Math.round(metersToWorldUnits(patch.centerMeters[0])), Math.round(metersToWorldUnits(patch.centerMeters[1]))],
    radiusMeters: options.radiusMeters,
    stepMeters: options.stepMeters,
    gridSize: { width, height },
    sampleCount: samples.length,
    surfaceMeters: {
      min: roundMetric(Math.min(...surfacesMeters)),
      max: roundMetric(Math.max(...surfacesMeters)),
      mean: roundMetric(average(surfacesMeters)),
      range: roundMetric(Math.max(...surfacesMeters) - Math.min(...surfacesMeters)),
    },
    distinctBiomes: sortedDistinct(samples.map((sample) => sample.biomeId)),
    distinctUndergroundBiomes: sortedDistinct(samples.map((sample) => sample.undergroundBiomeId)),
    distinctRegionalVariants: sortedDistinct(samples.map((sample) => sample.regionalVariantId).filter(isString)),
    directLandmarks,
    focusLandmarkIds: [...patch.focusLandmarkIds],
    missingFocusLandmarkIds: patch.focusLandmarkIds.filter((landmarkId) => !directLandmarks.includes(landmarkId)),
    materialDiversity,
    heightModulo: summarizeHeightModulo(samples, HEIGHT_MODULO),
    flatness,
    gridLikeness,
    warnings,
  };
}

function buildSurfaceSample(
  gridX: number,
  gridZ: number,
  worldX: number,
  worldZ: number,
  surfaceY: number,
  surfaceMaterial: number,
  probe: ProceduralBiomeProbe,
): TerrainSurfaceSample {
  return {
    gridX,
    gridZ,
    worldX,
    worldZ,
    surfaceY,
    surfaceMaterial,
    biomeId: probe.biomeId,
    undergroundBiomeId: probe.undergroundBiomeId,
    regionalVariantId: probe.regionalVariantId,
    landmarkId: probe.landmarkId,
  };
}

function summarizeMaterialDiversity(samples: readonly TerrainSurfaceSample[]): TerrainMaterialDiversity {
  const materialCounts = new Map<number, number>();
  for (const sample of samples) {
    materialCounts.set(sample.surfaceMaterial, (materialCounts.get(sample.surfaceMaterial) ?? 0) + 1);
  }
  const total = samples.length;
  const rows = [...materialCounts.entries()]
    .map(([material, count]) => ({
      material,
      hex: materialToHexColor(material),
      count,
      share: roundMetric(ratio(count, total)),
    }))
    .sort((left, right) => right.count - left.count || left.material - right.material);
  const entropy = -sum(rows.map((row) => {
    const share = row.count / total;
    return share <= 0 ? 0 : share * Math.log2(share);
  }));
  const maxEntropy = rows.length <= 1 ? 0 : Math.log2(rows.length);
  return {
    distinctMaterialCount: rows.length,
    entropy: roundMetric(entropy),
    normalizedEntropy: roundMetric(maxEntropy === 0 ? 0 : entropy / maxEntropy),
    dominantMaterial: rows[0] ? { ...rows[0] } : null,
    materialCounts: rows,
  };
}

function summarizeHeightModulo(
  samples: readonly TerrainSurfaceSample[],
  modulo: number,
): TerrainHeightModuloDistribution {
  const counts = new Array<number>(modulo).fill(0);
  for (const sample of samples) {
    counts[positiveModulo(sample.surfaceY, modulo)]! += 1;
  }
  return {
    modulo,
    buckets: counts.map((count, remainder) => ({
      remainder,
      count,
      share: roundMetric(ratio(count, samples.length)),
    })),
  };
}

function summarizeFlatness(
  samples: readonly TerrainSurfaceSample[],
  width: number,
  height: number,
  stepMeters: number,
): TerrainFlatnessDistribution {
  const buckets = FLATNESS_BUCKETS.map((bucket) => ({ ...bucket, count: 0, share: 0 }));
  const slopes: number[] = [];
  for (let z = 0; z < height; z += 1) {
    for (let x = 0; x < width; x += 1) {
      const sample = sampleAt(samples, width, x, z);
      const neighborSlopes: number[] = [];
      if (x + 1 < width) {
        neighborSlopes.push(surfaceSlope(sample, sampleAt(samples, width, x + 1, z), stepMeters));
      }
      if (z + 1 < height) {
        neighborSlopes.push(surfaceSlope(sample, sampleAt(samples, width, x, z + 1), stepMeters));
      }
      if (neighborSlopes.length === 0) {
        continue;
      }
      const slope = Math.max(...neighborSlopes);
      slopes.push(slope);
      const bucket = buckets.find((candidate) =>
        slope >= candidate.minSlope && (candidate.maxSlope === null || slope < candidate.maxSlope)
      ) ?? buckets[buckets.length - 1]!;
      bucket.count += 1;
    }
  }
  for (const bucket of buckets) {
    bucket.share = roundMetric(ratio(bucket.count, slopes.length));
  }
  return {
    averageSlope: roundMetric(average(slopes)),
    maxSlope: roundMetric(Math.max(0, ...slopes)),
    buckets,
  };
}

function summarizeGridLikeness(
  samples: readonly TerrainSurfaceSample[],
  width: number,
  height: number,
): TerrainGridLikeness {
  let axisEdges = 0;
  let axisComparisons = 0;
  let diagonalEdges = 0;
  let diagonalComparisons = 0;
  let heightAxisEdges = 0;
  let materialAxisEdges = 0;
  for (let z = 0; z < height; z += 1) {
    for (let x = 0; x < width; x += 1) {
      const sample = sampleAt(samples, width, x, z);
      for (const [dx, dz] of [[1, 0], [0, 1]] as const) {
        if (x + dx >= width || z + dz >= height) {
          continue;
        }
        axisComparisons += 1;
        const edge = classifySurfaceEdge(sample, sampleAt(samples, width, x + dx, z + dz));
        if (edge.heightEdge) {
          heightAxisEdges += 1;
        }
        if (edge.materialEdge) {
          materialAxisEdges += 1;
        }
        if (edge.heightEdge || edge.materialEdge) {
          axisEdges += 1;
        }
      }
      for (const [dx, dz] of [[1, 1], [-1, 1]] as const) {
        if (x + dx < 0 || x + dx >= width || z + dz >= height) {
          continue;
        }
        diagonalComparisons += 1;
        const edge = classifySurfaceEdge(sample, sampleAt(samples, width, x + dx, z + dz));
        if (edge.heightEdge || edge.materialEdge) {
          diagonalEdges += 1;
        }
      }
    }
  }
  const axisEdgeRate = ratio(axisEdges, axisComparisons);
  const diagonalEdgeRate = ratio(diagonalEdges, diagonalComparisons);
  const axisAlignedEdgeDominance = axisEdgeRate / Math.max(0.001, diagonalEdgeRate);
  const score = clamp01(axisEdgeRate * Math.max(0, axisAlignedEdgeDominance - 1) / 3);
  return {
    score: roundMetric(score),
    axisEdgeRate: roundMetric(axisEdgeRate),
    diagonalEdgeRate: roundMetric(diagonalEdgeRate),
    axisAlignedEdgeDominance: roundMetric(axisAlignedEdgeDominance),
    heightAxisEdgeRate: roundMetric(ratio(heightAxisEdges, axisComparisons)),
    materialAxisEdgeRate: roundMetric(ratio(materialAxisEdges, axisComparisons)),
    axisEdgeCount: axisEdges,
    diagonalEdgeCount: diagonalEdges,
  };
}

function classifySurfaceEdge(left: TerrainSurfaceSample, right: TerrainSurfaceSample): {
  heightEdge: boolean;
  materialEdge: boolean;
} {
  return {
    heightEdge: Math.abs(left.surfaceY - right.surfaceY) >= 2,
    materialEdge: left.surfaceMaterial !== right.surfaceMaterial,
  };
}

function buildPatchWarnings(input: {
  materialDiversity: TerrainMaterialDiversity;
  flatness: TerrainFlatnessDistribution;
  gridLikeness: TerrainGridLikeness;
  directLandmarks: readonly string[];
  focusLandmarkIds: readonly string[];
}): string[] {
  const warnings: string[] = [];
  if (input.materialDiversity.distinctMaterialCount <= 1) {
    warnings.push("single-surface-material");
  }
  if ((input.materialDiversity.dominantMaterial?.share ?? 0) >= 0.92) {
    warnings.push("dominant-surface-material");
  }
  if (input.gridLikeness.score >= 0.20) {
    warnings.push("grid-like-surface");
  }
  if (input.flatness.buckets.find((bucket) => bucket.id === "flat")!.share >= 0.92) {
    warnings.push("mostly-flat-surface");
  }
  const missingFocus = input.focusLandmarkIds.filter((landmarkId) => !input.directLandmarks.includes(landmarkId));
  if (missingFocus.length > 0) {
    warnings.push(`missing-focus-landmark:${missingFocus.join(",")}`);
  }
  return warnings;
}

function summarizeAggregate(patches: readonly TerrainPatchAnalysis[]): TerrainSurfaceLabReport["aggregate"] {
  return {
    patchCount: patches.length,
    sampleCount: sum(patches.map((patch) => patch.sampleCount)),
    distinctBiomes: sortedDistinct(patches.flatMap((patch) => patch.distinctBiomes)),
    distinctLandmarks: sortedDistinct(patches.flatMap((patch) => patch.directLandmarks)),
    averageMaterialCount: roundMetric(average(patches.map((patch) => patch.materialDiversity.distinctMaterialCount))),
    averageGridLikenessScore: roundMetric(average(patches.map((patch) => patch.gridLikeness.score))),
    maxGridLikenessScore: roundMetric(Math.max(0, ...patches.map((patch) => patch.gridLikeness.score))),
    maxSurfaceRangeMeters: roundMetric(Math.max(0, ...patches.map((patch) => patch.surfaceMeters.range))),
    warningCount: sum(patches.map((patch) => patch.warnings.length)),
  };
}

function buildMarkdownSummary(report: TerrainSurfaceLabReport): string {
  const lines = [
    "# Terrain Surface Lab Summary",
    "",
    `Generated: ${report.generatedAt}`,
    `Output: ${report.runDir}`,
    `Seed: ${report.seed}`,
    `Patch radius: ${report.options.radiusMeters} m`,
    `Patch step: ${report.options.stepMeters} m`,
    "",
    "## Aggregate",
    "",
    `- Patches: ${report.aggregate.patchCount}`,
    `- Samples: ${report.aggregate.sampleCount}`,
    `- Biomes: ${report.aggregate.distinctBiomes.length} (${report.aggregate.distinctBiomes.join(", ")})`,
    `- Landmarks: ${report.aggregate.distinctLandmarks.length} (${report.aggregate.distinctLandmarks.join(", ") || "none"})`,
    `- Average material count: ${report.aggregate.averageMaterialCount.toFixed(2)}`,
    `- Average grid-likeness: ${report.aggregate.averageGridLikenessScore.toFixed(3)}`,
    `- Max grid-likeness: ${report.aggregate.maxGridLikenessScore.toFixed(3)}`,
    `- Max surface range: ${report.aggregate.maxSurfaceRangeMeters.toFixed(1)} m`,
    `- Warnings: ${report.aggregate.warningCount}`,
    "",
    "## Patches",
    "",
    "| Patch | Source | Materials | Dominant | Height range | Flat/Gentle/Rolling/Steep/Cliff | Grid | Landmarks | Warnings |",
    "| --- | --- | ---: | ---: | ---: | --- | ---: | --- | --- |",
    ...report.patches.map((patch) => {
      const flatness = patch.flatness.buckets.map((bucket) => formatPercent(bucket.share)).join(" / ");
      const dominant = patch.materialDiversity.dominantMaterial
        ? `${patch.materialDiversity.dominantMaterial.hex} ${formatPercent(patch.materialDiversity.dominantMaterial.share)}`
        : "none";
      return `| ${patch.label} | ${patch.source} | ${patch.materialDiversity.distinctMaterialCount} | ${dominant} | ${patch.surfaceMeters.range.toFixed(1)} m | ${flatness} | ${patch.gridLikeness.score.toFixed(3)} | ${patch.directLandmarks.join(", ") || "none"} | ${patch.warnings.join(", ") || "none"} |`;
    }),
    "",
    "## Height Modulo",
    "",
    "| Patch | Modulo | Buckets |",
    "| --- | ---: | --- |",
    ...report.patches.map((patch) =>
      `| ${patch.label} | ${patch.heightModulo.modulo} | ${patch.heightModulo.buckets.map((bucket) => `${bucket.remainder}:${formatPercent(bucket.share)}`).join(", ")} |`
    ),
    "",
  ];
  return lines.join("\n");
}

function selectPatchSpecs(selectedPatchIds: readonly string[] | null): TerrainPatchSpec[] {
  if (!selectedPatchIds) {
    return [...TERRAIN_SURFACE_PATCHES];
  }
  const byId = new Map(TERRAIN_SURFACE_PATCHES.map((patch) => [patch.id, patch]));
  return selectedPatchIds.map((id) => {
    const patch = byId.get(id);
    if (!patch) {
      throw new Error(`Unknown terrain surface patch '${id}'. Known patches: ${TERRAIN_SURFACE_PATCHES.map((entry) => entry.id).join(", ")}`);
    }
    return patch;
  });
}

function readCliOptions(args: readonly string[]): TerrainSurfaceLabOptions {
  return {
    label: readFlag(args, "--label"),
    outputDir: readFlag(args, "--output-dir") ?? undefined,
    seed: readNumberFlag(args, "--seed"),
    radiusMeters: readNumberFlag(args, "--radius-meters"),
    stepMeters: readNumberFlag(args, "--step-meters"),
    patchIds: readListFlag(args, "--patches") ?? undefined,
  };
}

function readFlag(args: readonly string[], name: string): string | undefined {
  const index = args.indexOf(name);
  if (index < 0) {
    return undefined;
  }
  const value = args[index + 1];
  if (!value || value.startsWith("--")) {
    throw new Error(`${name} requires a value.`);
  }
  return value;
}

function readNumberFlag(args: readonly string[], name: string): number | undefined {
  const value = readFlag(args, name);
  if (value === undefined) {
    return undefined;
  }
  const parsed = Number(value);
  if (!Number.isFinite(parsed)) {
    throw new Error(`${name} must be a number.`);
  }
  return parsed;
}

function readListFlag(args: readonly string[], name: string): string[] | null {
  const value = readFlag(args, name);
  if (value === undefined) {
    return null;
  }
  const entries = value.split(",").map((entry) => entry.trim()).filter(Boolean);
  if (entries.length === 0) {
    throw new Error(`${name} requires at least one id.`);
  }
  return entries;
}

function sampleAt(samples: readonly TerrainSurfaceSample[], width: number, x: number, z: number): TerrainSurfaceSample {
  return samples[z * width + x]!;
}

function surfaceSlope(left: TerrainSurfaceSample, right: TerrainSurfaceSample, stepMeters: number): number {
  return Math.abs(worldUnitsToMeters(left.surfaceY - right.surfaceY)) / stepMeters;
}

function sortedDistinct(values: readonly string[]): string[] {
  return [...new Set(values)].sort();
}

function isString(value: unknown): value is string {
  return typeof value === "string";
}

function sum(values: readonly number[]): number {
  return values.reduce((total, value) => total + value, 0);
}

function average(values: readonly number[]): number {
  return ratio(sum(values), values.length);
}

function ratio(numerator: number, denominator: number): number {
  return denominator === 0 ? 0 : numerator / denominator;
}

function roundMetric(value: number): number {
  return Math.round(value * 1000) / 1000;
}

function positiveModulo(value: number, modulo: number): number {
  return ((value % modulo) + modulo) % modulo;
}

function clamp01(value: number): number {
  return Math.max(0, Math.min(1, value));
}

function validatePositiveNumber(value: number, name: string): void {
  if (!Number.isFinite(value) || value <= 0) {
    throw new Error(`${name} must be greater than zero.`);
  }
}

function formatPercent(value: number): string {
  return `${(value * 100).toFixed(1)}%`;
}
