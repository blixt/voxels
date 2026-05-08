import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";

import {
  isProceduralWaterMaterial,
  materialToHexColor,
  ProceduralWorldGenerator,
  type LandmarkId,
  type ProceduralBiomeProbe,
} from "../src/engine/procedural-generator.ts";

export const OBJECT_LAB_LANDMARK_IDS = [
  "oak",
  "canopy_tree",
  "birch",
  "redleaf_tree",
  "willow",
  "blossom_tree",
  "fruit_tree",
  "giant_flower",
  "redwood",
  "dead_tree",
  "thorn_tree",
  "berry_bush",
  "giant_fern",
  "lantern_tree",
  "salt_spire",
  "boulder",
  "standing_stone",
  "shrub",
  "flower_patch",
  "palm",
  "acacia",
  "cactus",
  "dead_snag",
  "hoodoo",
  "fir",
  "tall_fir",
  "ice_spire",
  "frost_shrub",
  "cypress",
  "mangrove",
  "reed_cluster",
  "basalt_spire",
  "crystal_cluster",
  "glowcap",
  "mega_glowcap",
  "root_stump",
  "stone_tor",
  "ancestor_pillar",
  "ash_marker",
  "glass_cairn",
  "silt_shell",
  "velothi_shrine",
  "kwama_mound",
  "pilgrim_cairn",
  "velothi_ziggurat",
  "ash_obelisk",
  "rib_arch",
  "old_road_causeway",
  "pilgrim_lantern",
  "bone_chimes",
  "crystal_reeds",
  "fungal_bridge",
  "rib_remains",
] as const satisfies readonly LandmarkId[];

export const OBJECT_LAB_ROUTE_LANDMARK_IDS = [
  "ash_marker",
  "pilgrim_lantern",
  "bone_chimes",
  "old_road_causeway",
  "velothi_ziggurat",
  "ash_obelisk",
  "rib_arch",
  "crystal_reeds",
  "fungal_bridge",
  "rib_remains",
] as const satisfies readonly LandmarkId[];

export interface LandmarkRoot {
  x: number;
  z: number;
  probe: ProceduralBiomeProbe;
}

export interface ObjectLabOptions {
  landmarkId: LandmarkId;
  seed?: number;
  outputDir?: string;
  label?: string;
  timestamp?: Date;
  scanRadius?: number;
  coarseStep?: number;
  refineRadius?: number;
  sampleRadius?: number;
  heightPadding?: number;
  worldX?: number;
  worldZ?: number;
}

export interface ObjectLabBatchOptions extends Omit<ObjectLabOptions, "landmarkId" | "label" | "worldX" | "worldZ"> {
  landmarkIds?: readonly LandmarkId[];
  label?: string;
}

export interface ObjectLabReport {
  generatedAt: string;
  landmarkId: LandmarkId;
  seed: number;
  runDir: string;
  root: {
    x: number;
    z: number;
    probe: ProceduralBiomeProbe;
  };
  sample: {
    radius: number;
    yMin: number;
    yMax: number;
    solidVoxelCount: number;
    bounds: {
      min: [number, number, number];
      max: [number, number, number];
    } | null;
    materialCounts: Array<{
      material: number;
      hex: string;
      count: number;
    }>;
    diagnostics: ObjectLabDiagnostics;
  };
  artifacts: {
    report: string;
    summary: string;
    contactSheet: string;
    topProjection: string;
    frontProjection: string;
    sideProjection: string;
  };
}

export interface ObjectLabBatchReport {
  generatedAt: string;
  seed: number;
  runDir: string;
  landmarkIds: LandmarkId[];
  reports: ObjectLabReport[];
  comparison: ObjectLabComparisonRow[];
  artifacts: {
    report: string;
    summary: string;
  };
}

export interface ObjectLabComparisonRow {
  landmarkId: LandmarkId;
  root: [number, number];
  biomeId: string;
  solidVoxelCount: number;
  boundsSize: [number, number, number] | null;
  materialVariety: number;
  dominantMaterialShare: number;
  fillRatio: number;
  solidVoxelBudget: ObjectScaleDiagnostics["solidVoxelBudget"];
  topSilhouette: ObjectLabSilhouetteSummary;
  frontSilhouette: ObjectLabSilhouetteSummary;
  warnings: string[];
  contactSheet: string;
}

export interface ObjectLabSilhouetteSummary {
  coverage: number;
  normalizedWidth: number;
  normalizedHeight: number;
  aspectRatio: number | null;
}

export interface ObjectLabDiagnostics {
  materialVariety: number;
  dominantMaterialShare: number;
  scale: ObjectScaleDiagnostics;
  sampleFit: SampleFitDiagnostics;
  warnings: string[];
  silhouette: {
    top: ProjectionDiagnostics;
    front: ProjectionDiagnostics;
    side: ProjectionDiagnostics;
  };
}

export interface ObjectScaleDiagnostics {
  boundsSize: [number, number, number] | null;
  maxHorizontalSpan: number;
  verticalSpan: number;
  occupiedColumnCount: number;
  boundsVolume: number;
  fillRatio: number;
  solidVoxelBudget: "empty" | "tiny" | "small" | "medium" | "large" | "huge";
}

export interface SampleFitDiagnostics {
  xMargin: {
    negative: number;
    positive: number;
  };
  zMargin: {
    negative: number;
    positive: number;
  };
  yHeadroom: number;
  touchesSampleEdge: boolean;
  touchesTop: boolean;
  centerOffset: [number, number, number] | null;
  normalizedCenterOffset: [number, number, number] | null;
}

export interface ProjectionDiagnostics {
  occupiedPixels: number;
  occupiedRows: number;
  occupiedColumns: number;
  coverage: number;
  bounds: {
    min: [number, number];
    max: [number, number];
  } | null;
  edgeTouch: {
    left: boolean;
    right: boolean;
    top: boolean;
    bottom: boolean;
  };
  centerOffset: [number, number] | null;
  normalizedWidth: number;
  normalizedHeight: number;
  aspectRatio: number | null;
}

interface SampledObject {
  radius: number;
  yMin: number;
  yMax: number;
  solidVoxelCount: number;
  bounds: ObjectLabReport["sample"]["bounds"];
  materialCounts: Map<number, number>;
  topProjection: Projection;
  frontProjection: Projection;
  sideProjection: Projection;
}

interface Projection {
  width: number;
  height: number;
  pixels: number[];
}

interface LandmarkRootCandidate {
  x: number;
  z: number;
  localDensity: number;
}

const DEFAULT_SEED = 1337;
const DEFAULT_OUTPUT_DIR = "artifacts/object-lab";
const DEFAULT_SCAN_RADIUS = 32_768;
const DEFAULT_COARSE_STEP = 64;
const DEFAULT_REFINE_RADIUS = 48;
const DEFAULT_SAMPLE_RADIUS = 32;
const DEFAULT_HEIGHT_PADDING = 8;
const BACKGROUND = 0xf4f4f4;
const LANDMARK_ID_SET = new Set<string>(OBJECT_LAB_LANDMARK_IDS);
const CENTERED_ROOT_LANDMARK_IDS = new Set<LandmarkId>([
  "old_road_causeway",
  "velothi_ziggurat",
  "ash_obelisk",
  "rib_arch",
  "bone_chimes",
  "silt_shell",
  "crystal_reeds",
  "fungal_bridge",
  "rib_remains",
]);

if (import.meta.main) {
  try {
    const options = readCliOptions(Bun.argv.slice(2));
    if (isObjectLabBatchOptions(options)) {
      const report = await runObjectLabBatch(options);
      console.log(`object-lab batch report: ${report.artifacts.report}`);
      console.log(`comparison summary: ${report.artifacts.summary}`);
      console.log(`landmarks: ${report.landmarkIds.join(", ")}`);
      console.log(`runs: ${report.reports.length}`);
    } else {
      const report = await runObjectLab(options);
      console.log(`object-lab report: ${report.artifacts.report}`);
      console.log(`summary: ${report.artifacts.summary}`);
      console.log(`contact sheet: ${report.artifacts.contactSheet}`);
      console.log(`root: ${report.root.x},${report.root.z} (${report.root.probe.biomeId})`);
      console.log(`solid object voxels: ${report.sample.solidVoxelCount}`);
    }
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exit(1);
  }
}

function isObjectLabBatchOptions(options: ObjectLabOptions | ObjectLabBatchOptions): options is ObjectLabBatchOptions {
  return "landmarkIds" in options;
}

export async function runObjectLabBatch(options: ObjectLabBatchOptions = {}): Promise<ObjectLabBatchReport> {
  const seed = options.seed ?? DEFAULT_SEED;
  const timestamp = options.timestamp ?? new Date();
  const landmarkIds = [...(options.landmarkIds ?? OBJECT_LAB_ROUTE_LANDMARK_IDS)];
  if (landmarkIds.length === 0) {
    throw new Error("Object lab batch requires at least one landmark id.");
  }
  const label = sanitizeFileStem(options.label ?? "route-landmark-comparison");
  const runName = `${timestampForFile(timestamp)}-${label}`;
  const runDir = join(options.outputDir ?? DEFAULT_OUTPUT_DIR, runName);
  await mkdir(runDir, { recursive: true });

  const reports: ObjectLabReport[] = [];
  for (const landmarkId of landmarkIds) {
    reports.push(
      await runObjectLab({
        landmarkId,
        seed,
        outputDir: runDir,
        label: landmarkId,
        timestamp,
        scanRadius: options.scanRadius,
        coarseStep: options.coarseStep,
        refineRadius: options.refineRadius,
        sampleRadius: options.sampleRadius,
        heightPadding: options.heightPadding,
      }),
    );
  }

  const comparison = reports.map(buildComparisonRow);
  const reportPath = join(runDir, "batch-report.json");
  const summaryPath = join(runDir, "comparison.md");
  const batchReport: ObjectLabBatchReport = {
    generatedAt: timestamp.toISOString(),
    seed,
    runDir,
    landmarkIds,
    reports,
    comparison,
    artifacts: {
      report: reportPath,
      summary: summaryPath,
    },
  };

  await writeFile(reportPath, `${JSON.stringify(batchReport, null, 2)}\n`);
  await writeFile(summaryPath, buildBatchMarkdownSummary(batchReport));
  return batchReport;
}

export async function runObjectLab(options: ObjectLabOptions): Promise<ObjectLabReport> {
  const seed = options.seed ?? DEFAULT_SEED;
  const generator = new ProceduralWorldGenerator(seed);
  const root = options.worldX !== undefined || options.worldZ !== undefined
    ? resolveRequestedRoot(generator, options)
    : findRepresentativeLandmarkRoot(generator, options.landmarkId, {
      scanRadius: options.scanRadius ?? DEFAULT_SCAN_RADIUS,
      coarseStep: options.coarseStep ?? DEFAULT_COARSE_STEP,
      refineRadius: options.refineRadius ?? DEFAULT_REFINE_RADIUS,
    });

  if (!root) {
    throw new Error(
      `Could not find landmark '${options.landmarkId}' within scan radius ${options.scanRadius ?? DEFAULT_SCAN_RADIUS}.`,
    );
  }

  const timestamp = options.timestamp ?? new Date();
  const label = sanitizeFileStem(options.label ?? options.landmarkId);
  const runName = `${timestampForFile(timestamp)}-${label}`;
  const runDir = join(options.outputDir ?? DEFAULT_OUTPUT_DIR, runName);
  await mkdir(runDir, { recursive: true });

  const sample = sampleObject(generator, root, {
    radius: options.sampleRadius ?? DEFAULT_SAMPLE_RADIUS,
    heightPadding: options.heightPadding ?? DEFAULT_HEIGHT_PADDING,
  });
  const reportPath = join(runDir, "report.json");
  const summaryPath = join(runDir, "summary.md");
  const contactSheetPath = join(runDir, "contact-sheet.svg");
  const topPath = join(runDir, "top.ppm");
  const frontPath = join(runDir, "front.ppm");
  const sidePath = join(runDir, "side.ppm");
  const diagnostics = buildDiagnostics(sample, root);
  const report: ObjectLabReport = {
    generatedAt: timestamp.toISOString(),
    landmarkId: options.landmarkId,
    seed,
    runDir,
    root: {
      x: root.x,
      z: root.z,
      probe: root.probe,
    },
    sample: {
      radius: sample.radius,
      yMin: sample.yMin,
      yMax: sample.yMax,
      solidVoxelCount: sample.solidVoxelCount,
      bounds: sample.bounds,
      materialCounts: [...sample.materialCounts.entries()]
        .map(([material, count]) => ({ material, hex: materialToHexColor(material), count }))
        .sort((a, b) => b.count - a.count || a.material - b.material),
      diagnostics,
    },
    artifacts: {
      report: reportPath,
      summary: summaryPath,
      contactSheet: contactSheetPath,
      topProjection: topPath,
      frontProjection: frontPath,
      sideProjection: sidePath,
    },
  };

  await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`);
  await writeFile(summaryPath, buildMarkdownSummary(report));
  await writeFile(contactSheetPath, buildContactSheetSvg(report, sample));
  await writeFile(topPath, encodePpm(sample.topProjection));
  await writeFile(frontPath, encodePpm(sample.frontProjection));
  await writeFile(sidePath, encodePpm(sample.sideProjection));
  return report;
}

export function findRepresentativeLandmarkRoot(
  generator: ProceduralWorldGenerator,
  landmarkId: LandmarkId,
  options: {
    scanRadius?: number;
    coarseStep?: number;
    refineRadius?: number;
  } = {},
): LandmarkRoot | null {
  const scanRadius = options.scanRadius ?? DEFAULT_SCAN_RADIUS;
  const coarseStep = options.coarseStep ?? DEFAULT_COARSE_STEP;
  const refineRadius = options.refineRadius ?? DEFAULT_REFINE_RADIUS;
  for (let coarseZ = -scanRadius; coarseZ <= scanRadius; coarseZ += coarseStep) {
    for (let coarseX = -scanRadius; coarseX <= scanRadius; coarseX += coarseStep) {
      if (generator.sampleBiomeProbe(coarseX, coarseZ).landmarkId !== landmarkId) {
        continue;
      }
      const root = refineLandmarkRoot(generator, landmarkId, coarseX, coarseZ, refineRadius);
      if (root) {
        return root;
      }
    }
  }
  return null;
}

function refineLandmarkRoot(
  generator: ProceduralWorldGenerator,
  landmarkId: LandmarkId,
  coarseX: number,
  coarseZ: number,
  refineRadius: number,
): LandmarkRoot | null {
  const candidates: LandmarkRootCandidate[] = [];
  const candidateKeys = new Set<string>();
  let sumX = 0;
  let sumZ = 0;

  for (let z = coarseZ - refineRadius; z <= coarseZ + refineRadius; z += 1) {
    for (let x = coarseX - refineRadius; x <= coarseX + refineRadius; x += 1) {
      const probe = generator.sampleBiomeProbe(x, z);
      const rootMaterial = generator.sampleMaterial(x, probe.surfaceY + 1, z);
      if (probe.landmarkId !== landmarkId || rootMaterial === 0 || isProceduralWaterMaterial(rootMaterial)) {
        continue;
      }
      candidates.push({ x, z, localDensity: 0 });
      candidateKeys.add(`${x},${z}`);
      sumX += x;
      sumZ += z;
    }
  }

  if (candidates.length === 0) {
    return null;
  }
  if (!CENTERED_ROOT_LANDMARK_IDS.has(landmarkId)) {
    const first = candidates[0]!;
    return { x: first.x, z: first.z, probe: generator.sampleBiomeProbe(first.x, first.z) };
  }

  const centroidX = sumX / candidates.length;
  const centroidZ = sumZ / candidates.length;
  let best = candidates[0]!;
  let bestScore = -Infinity;
  for (const candidate of candidates) {
    candidate.localDensity = countLocalLandmarkRoots(candidateKeys, candidate.x, candidate.z);
    const distanceToCentroid = squaredDistance(candidate.x, candidate.z, centroidX, centroidZ);
    const distanceToCoarseProbe = squaredDistance(candidate.x, candidate.z, coarseX, coarseZ);
    const score = -distanceToCentroid * 100 + candidate.localDensity * 10 - distanceToCoarseProbe * 0.001;
    if (
      score > bestScore
      || (score === bestScore && compareCandidateTieBreak(candidate, best, centroidX, centroidZ) < 0)
    ) {
      best = candidate;
      bestScore = score;
    }
  }

  return { x: best.x, z: best.z, probe: generator.sampleBiomeProbe(best.x, best.z) };
}

function countLocalLandmarkRoots(candidateKeys: Set<string>, x: number, z: number): number {
  let count = 0;
  for (let dz = -2; dz <= 2; dz += 1) {
    for (let dx = -2; dx <= 2; dx += 1) {
      if (candidateKeys.has(`${x + dx},${z + dz}`)) {
        count += 1;
      }
    }
  }
  return count;
}

function squaredDistance(ax: number, az: number, bx: number, bz: number): number {
  const dx = ax - bx;
  const dz = az - bz;
  return dx * dx + dz * dz;
}

function compareCandidateTieBreak(
  candidate: LandmarkRootCandidate,
  incumbent: LandmarkRootCandidate,
  centroidX: number,
  centroidZ: number,
): number {
  const candidateDistance = squaredDistance(candidate.x, candidate.z, centroidX, centroidZ);
  const incumbentDistance = squaredDistance(incumbent.x, incumbent.z, centroidX, centroidZ);
  if (candidateDistance !== incumbentDistance) {
    return candidateDistance - incumbentDistance;
  }
  if (candidate.z !== incumbent.z) {
    return candidate.z - incumbent.z;
  }
  return candidate.x - incumbent.x;
}

function resolveRequestedRoot(generator: ProceduralWorldGenerator, options: ObjectLabOptions): LandmarkRoot {
  if (options.worldX === undefined || options.worldZ === undefined) {
    throw new Error("--world-x and --world-z must be provided together.");
  }
  const probe = generator.sampleBiomeProbe(options.worldX, options.worldZ);
  if (probe.landmarkId !== options.landmarkId) {
    throw new Error(
      `Requested coordinate has landmark '${probe.landmarkId ?? "none"}', not '${options.landmarkId}'.`,
    );
  }
  return { x: options.worldX, z: options.worldZ, probe };
}

function sampleObject(
  generator: ProceduralWorldGenerator,
  root: LandmarkRoot,
  options: {
    radius: number;
    heightPadding: number;
  },
): SampledObject {
  const width = options.radius * 2 + 1;
  const yMin = root.probe.surfaceY + 1;
  const yMax = root.probe.topY + options.heightPadding;
  const height = yMax - yMin + 1;
  const topProjection = createProjection(width, width);
  const frontProjection = createProjection(width, height);
  const sideProjection = createProjection(width, height);
  const materialCounts = new Map<number, number>();
  let solidVoxelCount = 0;
  let minX = Infinity;
  let minY = Infinity;
  let minZ = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  let maxZ = -Infinity;

  for (let dz = -options.radius; dz <= options.radius; dz += 1) {
    for (let dx = -options.radius; dx <= options.radius; dx += 1) {
      const worldX = root.x + dx;
      const worldZ = root.z + dz;
      if (generator.sampleBiomeProbe(worldX, worldZ).landmarkId !== root.probe.landmarkId) {
        continue;
      }
      const localSurfaceY = generator.sampleColumn(worldX, worldZ).surfaceY;
      for (let y = Math.max(yMin, localSurfaceY + 1); y <= yMax; y += 1) {
        const material = generator.sampleMaterial(worldX, y, worldZ);
        if (material === 0 || isProceduralWaterMaterial(material)) {
          continue;
        }
        solidVoxelCount += 1;
        materialCounts.set(material, (materialCounts.get(material) ?? 0) + 1);
        minX = Math.min(minX, worldX);
        minY = Math.min(minY, y);
        minZ = Math.min(minZ, worldZ);
        maxX = Math.max(maxX, worldX + 1);
        maxY = Math.max(maxY, y + 1);
        maxZ = Math.max(maxZ, worldZ + 1);
        const color = rgbIntFromMaterial(material);
        const px = dx + options.radius;
        const pz = dz + options.radius;
        const py = yMax - y;
        setPixel(topProjection, px, pz, color);
        setPixel(frontProjection, px, py, color);
        setPixel(sideProjection, pz, py, color);
      }
    }
  }

  return {
    radius: options.radius,
    yMin,
    yMax,
    solidVoxelCount,
    bounds: solidVoxelCount === 0 ? null : { min: [minX, minY, minZ], max: [maxX, maxY, maxZ] },
    materialCounts,
    topProjection,
    frontProjection,
    sideProjection,
  };
}

function createProjection(width: number, height: number): Projection {
  return {
    width,
    height,
    pixels: Array.from({ length: width * height }, () => BACKGROUND),
  };
}

function setPixel(projection: Projection, x: number, y: number, color: number): void {
  if (x < 0 || y < 0 || x >= projection.width || y >= projection.height) {
    return;
  }
  projection.pixels[x + y * projection.width] = color;
}

function encodePpm(projection: Projection): string {
  const lines = [`P3`, `${projection.width} ${projection.height}`, `255`];
  for (let y = 0; y < projection.height; y += 1) {
    const row: string[] = [];
    for (let x = 0; x < projection.width; x += 1) {
      const color = projection.pixels[x + y * projection.width] ?? BACKGROUND;
      row.push(`${(color >> 16) & 0xff} ${(color >> 8) & 0xff} ${color & 0xff}`);
    }
    lines.push(row.join(" "));
  }
  return `${lines.join("\n")}\n`;
}

function buildDiagnostics(sample: SampledObject, root: LandmarkRoot): ObjectLabDiagnostics {
  const materialCounts = [...sample.materialCounts.values()];
  const dominantCount = materialCounts.length === 0 ? 0 : Math.max(...materialCounts);
  const dominantMaterialShare = sample.solidVoxelCount === 0
    ? 0
    : roundRatio(dominantCount / sample.solidVoxelCount);
  const sampleFit = inspectSampleFit(sample, root);
  const silhouette = {
    top: inspectProjection(sample.topProjection),
    front: inspectProjection(sample.frontProjection),
    side: inspectProjection(sample.sideProjection),
  };
  const scale = inspectObjectScale(sample, silhouette.top);
  return {
    materialVariety: sample.materialCounts.size,
    dominantMaterialShare,
    scale,
    sampleFit,
    warnings: buildDiagnosticWarnings(sampleFit, silhouette, scale, dominantMaterialShare, sample.solidVoxelCount),
    silhouette,
  };
}

function inspectObjectScale(sample: SampledObject, topProjection: ProjectionDiagnostics): ObjectScaleDiagnostics {
  if (!sample.bounds) {
    return {
      boundsSize: null,
      maxHorizontalSpan: 0,
      verticalSpan: 0,
      occupiedColumnCount: 0,
      boundsVolume: 0,
      fillRatio: 0,
      solidVoxelBudget: "empty",
    };
  }
  const xSize = sample.bounds.max[0] - sample.bounds.min[0];
  const ySize = sample.bounds.max[1] - sample.bounds.min[1];
  const zSize = sample.bounds.max[2] - sample.bounds.min[2];
  const boundsVolume = xSize * ySize * zSize;
  return {
    boundsSize: [xSize, ySize, zSize],
    maxHorizontalSpan: Math.max(xSize, zSize),
    verticalSpan: ySize,
    occupiedColumnCount: topProjection.occupiedPixels,
    boundsVolume,
    fillRatio: boundsVolume === 0 ? 0 : roundRatio(sample.solidVoxelCount / boundsVolume),
    solidVoxelBudget: classifySolidVoxelBudget(sample.solidVoxelCount),
  };
}

function classifySolidVoxelBudget(solidVoxelCount: number): ObjectScaleDiagnostics["solidVoxelBudget"] {
  if (solidVoxelCount === 0) {
    return "empty";
  }
  if (solidVoxelCount < 64) {
    return "tiny";
  }
  if (solidVoxelCount < 512) {
    return "small";
  }
  if (solidVoxelCount < 2_048) {
    return "medium";
  }
  if (solidVoxelCount < 8_192) {
    return "large";
  }
  return "huge";
}

function inspectSampleFit(sample: SampledObject, root: LandmarkRoot): SampleFitDiagnostics {
  if (!sample.bounds) {
    return {
      xMargin: { negative: sample.radius, positive: sample.radius },
      zMargin: { negative: sample.radius, positive: sample.radius },
      yHeadroom: sample.yMax - sample.yMin + 1,
      touchesSampleEdge: false,
      touchesTop: false,
      centerOffset: null,
      normalizedCenterOffset: null,
    };
  }
  const minX = sample.bounds.min[0] - root.x;
  const maxX = sample.bounds.max[0] - 1 - root.x;
  const minY = sample.bounds.min[1];
  const maxY = sample.bounds.max[1] - 1;
  const minZ = sample.bounds.min[2] - root.z;
  const maxZ = sample.bounds.max[2] - 1 - root.z;
  const centerOffset: [number, number, number] = [
    roundRatio((minX + maxX) / 2),
    roundRatio((minY + maxY) / 2 - sample.yMin),
    roundRatio((minZ + maxZ) / 2),
  ];
  const sampleHeight = sample.yMax - sample.yMin + 1;
  const normalizedCenterOffset: [number, number, number] = [
    sample.radius === 0 ? 0 : roundRatio(centerOffset[0] / sample.radius),
    sampleHeight <= 1 ? 0 : roundRatio(centerOffset[1] / (sampleHeight - 1)),
    sample.radius === 0 ? 0 : roundRatio(centerOffset[2] / sample.radius),
  ];
  const xMargin = {
    negative: minX + sample.radius,
    positive: sample.radius - maxX,
  };
  const zMargin = {
    negative: minZ + sample.radius,
    positive: sample.radius - maxZ,
  };
  const yHeadroom = sample.yMax - maxY;
  return {
    xMargin,
    zMargin,
    yHeadroom,
    touchesSampleEdge: xMargin.negative <= 0
      || xMargin.positive <= 0
      || zMargin.negative <= 0
      || zMargin.positive <= 0,
    touchesTop: yHeadroom <= 0,
    centerOffset,
    normalizedCenterOffset,
  };
}

function inspectProjection(projection: Projection): ProjectionDiagnostics {
  let occupiedPixels = 0;
  const occupiedRows = new Set<number>();
  const occupiedColumns = new Set<number>();
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (let y = 0; y < projection.height; y += 1) {
    for (let x = 0; x < projection.width; x += 1) {
      if (projection.pixels[x + y * projection.width] === BACKGROUND) {
        continue;
      }
      occupiedPixels += 1;
      occupiedRows.add(y);
      occupiedColumns.add(x);
      minX = Math.min(minX, x);
      minY = Math.min(minY, y);
      maxX = Math.max(maxX, x + 1);
      maxY = Math.max(maxY, y + 1);
    }
  }
  const pixelCount = projection.width * projection.height;
  if (occupiedPixels === 0) {
    return {
      occupiedPixels: 0,
      occupiedRows: 0,
      occupiedColumns: 0,
      coverage: 0,
      bounds: null,
      edgeTouch: { left: false, right: false, top: false, bottom: false },
      centerOffset: null,
      normalizedWidth: 0,
      normalizedHeight: 0,
      aspectRatio: null,
    };
  }
  const width = maxX - minX;
  const height = maxY - minY;
  return {
    occupiedPixels,
    occupiedRows: occupiedRows.size,
    occupiedColumns: occupiedColumns.size,
    coverage: roundRatio(occupiedPixels / pixelCount),
    bounds: { min: [minX, minY], max: [maxX, maxY] },
    edgeTouch: {
      left: minX === 0,
      right: maxX === projection.width,
      top: minY === 0,
      bottom: maxY === projection.height,
    },
    centerOffset: [
      roundRatio(
        ((minX + maxX - 1) / 2 - (projection.width - 1) / 2) / Math.max(1, (projection.width - 1) / 2),
      ),
      roundRatio(
        ((minY + maxY - 1) / 2 - (projection.height - 1) / 2) / Math.max(1, (projection.height - 1) / 2),
      ),
    ],
    normalizedWidth: roundRatio(width / projection.width),
    normalizedHeight: roundRatio(height / projection.height),
    aspectRatio: roundRatio(width / height),
  };
}

function buildDiagnosticWarnings(
  sampleFit: SampleFitDiagnostics,
  silhouette: ObjectLabDiagnostics["silhouette"],
  scale: ObjectScaleDiagnostics,
  dominantMaterialShare: number,
  solidVoxelCount: number,
): string[] {
  if (solidVoxelCount === 0) {
    return ["empty-sample"];
  }
  const warnings: string[] = [];
  if (sampleFit.touchesSampleEdge) {
    warnings.push("sample-touches-horizontal-edge");
  }
  if (sampleFit.touchesTop) {
    warnings.push("sample-touches-top");
  }
  if (
    sampleFit.normalizedCenterOffset
    && Math.max(Math.abs(sampleFit.normalizedCenterOffset[0]), Math.abs(sampleFit.normalizedCenterOffset[2])) > 0.35
  ) {
    warnings.push("root-off-center");
  }
  if (dominantMaterialShare > 0.92) {
    warnings.push("dominant-material");
  }
  if (scale.verticalSpan > 0 && scale.verticalSpan < 3 && scale.maxHorizontalSpan < 4) {
    warnings.push("object-too-small");
  }
  if (scale.solidVoxelBudget === "huge") {
    warnings.push("huge-solid-voxel-budget");
  }
  if (scale.boundsVolume > 0 && scale.fillRatio < 0.08) {
    warnings.push("low-bounds-fill");
  }
  for (const [view, diagnostics] of Object.entries(silhouette)) {
    if (diagnostics.coverage > 0 && diagnostics.coverage < 0.02) {
      warnings.push(`${view}-projection-sparse`);
    }
    const touchesClippingEdge = view === "top"
      ? Object.values(diagnostics.edgeTouch).some(Boolean)
      : diagnostics.edgeTouch.left || diagnostics.edgeTouch.right || diagnostics.edgeTouch.top;
    if (touchesClippingEdge) {
      warnings.push(`${view}-projection-touches-edge`);
    }
  }
  return warnings;
}

function roundRatio(value: number): number {
  return Math.round(value * 1000) / 1000;
}

function rgbIntFromMaterial(material: number): number {
  const hex = materialToHexColor(material);
  const r = Number.parseInt(hex[1]!, 16) * 17;
  const g = Number.parseInt(hex[2]!, 16) * 17;
  const b = Number.parseInt(hex[3]!, 16) * 17;
  return (r << 16) | (g << 8) | b;
}

function buildMarkdownSummary(report: ObjectLabReport): string {
  const bounds = report.sample.bounds
    ? `${report.sample.bounds.min.join(", ")} -> ${report.sample.bounds.max.join(", ")}`
    : "none";
  const scale = report.sample.diagnostics.scale;
  const materialRows = report.sample.materialCounts
    .slice(0, 12)
    .map((entry) => `| ${entry.hex} | ${entry.material} | ${entry.count} |`)
    .join("\n");
  return [
    `# Object Lab: ${report.landmarkId}`,
    ``,
    `- Seed: ${report.seed}`,
    `- Root: ${report.root.x}, ${report.root.z}`,
    `- Biome: ${report.root.probe.biomeId}`,
    `- Regional variant: ${report.root.probe.regionalVariantId ?? "none"}`,
    `- Surface Y: ${report.root.probe.surfaceY}`,
    `- Probe top Y: ${report.root.probe.topY}`,
    `- Sample radius: ${report.sample.radius}`,
    `- Solid object voxels: ${report.sample.solidVoxelCount}`,
    `- Bounds: ${bounds}`,
    `- Bounds size: ${formatNullableTuple(scale.boundsSize)}`,
    `- Material variety: ${report.sample.diagnostics.materialVariety}`,
    `- Dominant material share: ${formatPercent(report.sample.diagnostics.dominantMaterialShare)}`,
    `- Solid voxel budget: ${scale.solidVoxelBudget}`,
    `- Warnings: ${
      report.sample.diagnostics.warnings.length === 0 ? "none" : report.sample.diagnostics.warnings.join(", ")
    }`,
    ``,
    `## Projection Artifacts`,
    ``,
    `- Contact sheet: ${report.artifacts.contactSheet}`,
    `- Top: ${report.artifacts.topProjection}`,
    `- Front: ${report.artifacts.frontProjection}`,
    `- Side: ${report.artifacts.sideProjection}`,
    ``,
    `## Silhouette Diagnostics`,
    ``,
    `| View | Coverage | Width | Height | Aspect | Occupied Pixels | Rows | Columns | Center Offset |`,
    `| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |`,
    silhouetteRow("Top", report.sample.diagnostics.silhouette.top),
    silhouetteRow("Front", report.sample.diagnostics.silhouette.front),
    silhouetteRow("Side", report.sample.diagnostics.silhouette.side),
    ``,
    `## Scale And Cost Diagnostics`,
    ``,
    `| Bounds Size | Max Horizontal Span | Vertical Span | Occupied Columns | Bounds Volume | Fill Ratio | Solid Voxel Budget |`,
    `| ---: | ---: | ---: | ---: | ---: | ---: | --- |`,
    `| ${formatNullableTuple(scale.boundsSize)} | ${scale.maxHorizontalSpan} | ${scale.verticalSpan} | ${scale.occupiedColumnCount} | ${
      scale.boundsVolume
    } | ${formatPercent(scale.fillRatio)} | ${scale.solidVoxelBudget} |`,
    ``,
    `## Sample Fit Diagnostics`,
    ``,
    `| Axis | Negative Margin | Positive Margin |`,
    `| --- | ---: | ---: |`,
    `| X | ${report.sample.diagnostics.sampleFit.xMargin.negative} | ${report.sample.diagnostics.sampleFit.xMargin.positive} |`,
    `| Z | ${report.sample.diagnostics.sampleFit.zMargin.negative} | ${report.sample.diagnostics.sampleFit.zMargin.positive} |`,
    ``,
    `- Y headroom: ${report.sample.diagnostics.sampleFit.yHeadroom}`,
    `- Touches sample edge: ${report.sample.diagnostics.sampleFit.touchesSampleEdge ? "yes" : "no"}`,
    `- Touches top: ${report.sample.diagnostics.sampleFit.touchesTop ? "yes" : "no"}`,
    `- Center offset: ${formatNullableTuple(report.sample.diagnostics.sampleFit.centerOffset)}`,
    `- Normalized center offset: ${formatNullableTuple(report.sample.diagnostics.sampleFit.normalizedCenterOffset)}`,
    ``,
    `## Materials`,
    ``,
    `| Hex | Material | Count |`,
    `| --- | ---: | ---: |`,
    materialRows || `| none | 0 | 0 |`,
    ``,
  ].join("\n");
}

function buildComparisonRow(report: ObjectLabReport): ObjectLabComparisonRow {
  return {
    landmarkId: report.landmarkId,
    root: [report.root.x, report.root.z],
    biomeId: report.root.probe.biomeId,
    solidVoxelCount: report.sample.solidVoxelCount,
    boundsSize: report.sample.diagnostics.scale.boundsSize,
    materialVariety: report.sample.diagnostics.materialVariety,
    dominantMaterialShare: report.sample.diagnostics.dominantMaterialShare,
    fillRatio: report.sample.diagnostics.scale.fillRatio,
    solidVoxelBudget: report.sample.diagnostics.scale.solidVoxelBudget,
    topSilhouette: summarizeSilhouette(report.sample.diagnostics.silhouette.top),
    frontSilhouette: summarizeSilhouette(report.sample.diagnostics.silhouette.front),
    warnings: report.sample.diagnostics.warnings,
    contactSheet: report.artifacts.contactSheet,
  };
}

function summarizeSilhouette(diagnostics: ProjectionDiagnostics): ObjectLabSilhouetteSummary {
  return {
    coverage: diagnostics.coverage,
    normalizedWidth: diagnostics.normalizedWidth,
    normalizedHeight: diagnostics.normalizedHeight,
    aspectRatio: diagnostics.aspectRatio,
  };
}

function buildBatchMarkdownSummary(report: ObjectLabBatchReport): string {
  const warningRows = report.comparison
    .filter((row) => row.warnings.length > 0)
    .map((row) => `| ${row.landmarkId} | ${row.warnings.join(", ")} | ${row.contactSheet} |`)
    .join("\n");
  const rows = report.comparison.map((row) => [
    `| ${row.landmarkId}`,
    `${row.root[0]}, ${row.root[1]}`,
    row.biomeId,
    row.solidVoxelCount.toString(),
    formatNullableTuple(row.boundsSize),
    row.materialVariety.toString(),
    formatPercent(row.dominantMaterialShare),
    formatPercent(row.fillRatio),
    row.solidVoxelBudget,
    formatSilhouetteSummary(row.topSilhouette),
    formatSilhouetteSummary(row.frontSilhouette),
    row.warnings.length === 0 ? "none" : row.warnings.join(", "),
    row.contactSheet,
  ].join(" | ") + " |").join("\n");

  return [
    `# Object Lab Route Landmark Comparison`,
    ``,
    `- Seed: ${report.seed}`,
    `- Runs: ${report.reports.length}`,
    `- Batch report: ${report.artifacts.report}`,
    ``,
    `## Comparison`,
    ``,
    `| Landmark | Root | Biome | Voxels | Bounds Size | Materials | Dominant | Fill | Budget | Top Silhouette | Front Silhouette | Warnings | Contact Sheet |`,
    `| --- | ---: | --- | ---: | ---: | ---: | ---: | ---: | --- | --- | --- | --- | --- |`,
    rows,
    ``,
    `## Warning Queue`,
    ``,
    `| Landmark | Warnings | Contact Sheet |`,
    `| --- | --- | --- |`,
    warningRows || `| none | none | n/a |`,
    ``,
  ].join("\n");
}

function formatSilhouetteSummary(summary: ObjectLabSilhouetteSummary): string {
  const aspect = summary.aspectRatio === null ? "n/a" : summary.aspectRatio.toFixed(3);
  return `cov ${formatPercent(summary.coverage)}, w ${formatPercent(summary.normalizedWidth)}, h ${formatPercent(
    summary.normalizedHeight,
  )}, ar ${aspect}`;
}

function silhouetteRow(label: string, diagnostics: ProjectionDiagnostics): string {
  return [
    `| ${label}`,
    formatPercent(diagnostics.coverage),
    formatPercent(diagnostics.normalizedWidth),
    formatPercent(diagnostics.normalizedHeight),
    diagnostics.aspectRatio === null ? "n/a" : diagnostics.aspectRatio.toFixed(3),
    diagnostics.occupiedPixels.toString(),
    diagnostics.occupiedRows.toString(),
    diagnostics.occupiedColumns.toString(),
    diagnostics.centerOffset ? diagnostics.centerOffset.join(", ") : "n/a",
  ].join(" | ") + " |";
}

function formatNullableTuple(tuple: readonly number[] | null): string {
  return tuple ? tuple.join(", ") : "n/a";
}

function buildContactSheetSvg(report: ObjectLabReport, sample: SampledObject): string {
  const cellSize = 6;
  const gap = 28;
  const labelHeight = 36;
  const panelPadding = 10;
  const legendWidth = 240;
  const panels = [
    { label: "Top", projection: sample.topProjection, diagnostics: report.sample.diagnostics.silhouette.top },
    { label: "Front", projection: sample.frontProjection, diagnostics: report.sample.diagnostics.silhouette.front },
    { label: "Side", projection: sample.sideProjection, diagnostics: report.sample.diagnostics.silhouette.side },
  ];
  const panelWidth = Math.max(...panels.map((panel) => panel.projection.width * cellSize + panelPadding * 2));
  const panelHeights = panels.map((panel) => panel.projection.height * cellSize + labelHeight + panelPadding * 2);
  const width = panelWidth * panels.length + gap * (panels.length - 1) + legendWidth;
  const height = Math.max(...panelHeights, 260);
  const materialRows = report.sample.materialCounts.slice(0, 12);
  const parts = [
    `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}" role="img" aria-label="Object lab contact sheet for ${escapeXml(report.landmarkId)}">`,
    `<rect width="100%" height="100%" fill="#f8f8f6"/>`,
    `<style>text{font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;fill:#202020}.title{font-size:15px;font-weight:700}.meta{font-size:11px}.legend{font-size:12px}</style>`,
  ];
  let x = 0;
  for (const panel of panels) {
    const panelHeight = panel.projection.height * cellSize + labelHeight + panelPadding * 2;
    parts.push(`<rect x="${x}" y="0" width="${panelWidth}" height="${panelHeight}" fill="#ffffff" stroke="#d6d2c8"/>`);
    parts.push(
      `<text class="title" x="${x + panelPadding}" y="18">${panel.label}</text>`,
      `<text class="meta" x="${x + panelPadding + 58}" y="18">coverage ${formatPercent(panel.diagnostics.coverage)} | aspect ${
        panel.diagnostics.aspectRatio === null ? "n/a" : panel.diagnostics.aspectRatio.toFixed(3)
      }</text>`,
      `<text class="meta" x="${x + panelPadding}" y="34">rows ${panel.diagnostics.occupiedRows} | cols ${panel.diagnostics.occupiedColumns} | offset ${
        panel.diagnostics.centerOffset ? panel.diagnostics.centerOffset.join(",") : "n/a"
      }</text>`,
    );
    parts.push(projectSvg(panel.projection, x + panelPadding, labelHeight + panelPadding + 10, cellSize));
    x += panelWidth + gap;
  }
  parts.push(`<g transform="translate(${x}, 0)">`);
  parts.push(`<text class="title" x="0" y="18">${escapeXml(report.landmarkId)}</text>`);
  parts.push(`<text class="meta" x="0" y="40">seed ${report.seed} | root ${report.root.x}, ${report.root.z}</text>`);
  parts.push(`<text class="meta" x="0" y="58">voxels ${report.sample.solidVoxelCount} | materials ${report.sample.diagnostics.materialVariety}</text>`);
  parts.push(
    `<text class="meta" x="0" y="76">fit x ${formatMargin(report.sample.diagnostics.sampleFit.xMargin)} | z ${
      formatMargin(report.sample.diagnostics.sampleFit.zMargin)
    } | top ${report.sample.diagnostics.sampleFit.yHeadroom}</text>`,
  );
  parts.push(
    `<text class="meta" x="0" y="94">warnings ${
      report.sample.diagnostics.warnings.length === 0 ? "none" : escapeXml(report.sample.diagnostics.warnings.join(", "))
    }</text>`,
  );
  parts.push(
    `<text class="meta" x="0" y="112">size ${formatNullableTuple(report.sample.diagnostics.scale.boundsSize)} | columns ${
      report.sample.diagnostics.scale.occupiedColumnCount
    } | fill ${formatPercent(report.sample.diagnostics.scale.fillRatio)} | budget ${
      report.sample.diagnostics.scale.solidVoxelBudget
    }</text>`,
  );
  parts.push(`<text class="legend" x="0" y="130">Material legend</text>`);
  materialRows.forEach((entry, index) => {
    const y = 142 + index * 18;
    parts.push(`<rect x="0" y="${y - 10}" width="12" height="12" fill="${entry.hex}"/>`);
    parts.push(`<text class="meta" x="20" y="${y}">${entry.hex} material ${entry.material} (${entry.count})</text>`);
  });
  parts.push(`</g>`);
  parts.push(`</svg>`);
  return `${parts.join("\n")}\n`;
}

function projectSvg(projection: Projection, offsetX: number, offsetY: number, cellSize: number): string {
  const parts: string[] = [];
  for (let y = 0; y < projection.height; y += 1) {
    for (let x = 0; x < projection.width; x += 1) {
      const color = projection.pixels[x + y * projection.width] ?? BACKGROUND;
      if (color === BACKGROUND) {
        continue;
      }
      parts.push(
        `<rect x="${offsetX + x * cellSize}" y="${offsetY + y * cellSize}" width="${cellSize}" height="${cellSize}" fill="${hexFromRgbInt(color)}"/>`,
      );
    }
  }
  return parts.join("\n");
}

function hexFromRgbInt(color: number): string {
  return `#${color.toString(16).padStart(6, "0")}`;
}

function formatPercent(value: number): string {
  return `${(value * 100).toFixed(1)}%`;
}

function formatMargin(margin: { negative: number; positive: number }): string {
  return `${margin.negative}/${margin.positive}`;
}

function escapeXml(value: string): string {
  return value.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;").replaceAll('"', "&quot;");
}

function readCliOptions(args: string[]): ObjectLabOptions | ObjectLabBatchOptions {
  const landmarkIds = readLandmarkIds(args);
  if (landmarkIds) {
    if (readFlag(args, "--world-x") !== null || readFlag(args, "--world-z") !== null) {
      throw new Error("--world-x and --world-z are only supported for single-object lab runs.");
    }
    return {
      landmarkIds,
      seed: readPositiveInt(readFlag(args, "--seed"), DEFAULT_SEED),
      outputDir: readFlag(args, "--output-dir") ?? DEFAULT_OUTPUT_DIR,
      label: readFlag(args, "--label") ?? "route-landmark-comparison",
      timestamp: undefined,
      scanRadius: readPositiveInt(readFlag(args, "--scan-radius"), DEFAULT_SCAN_RADIUS),
      coarseStep: readPositiveInt(readFlag(args, "--coarse-step"), DEFAULT_COARSE_STEP),
      refineRadius: readPositiveInt(readFlag(args, "--refine-radius"), DEFAULT_REFINE_RADIUS),
      sampleRadius: readPositiveInt(readFlag(args, "--sample-radius"), DEFAULT_SAMPLE_RADIUS),
      heightPadding: readNonNegativeInt(readFlag(args, "--height-padding"), DEFAULT_HEIGHT_PADDING),
    };
  }
  return readOptions(args);
}

function readOptions(args: string[]): ObjectLabOptions {
  const rawId = readFlag(args, "--id") ?? args.find((arg) => !arg.startsWith("--"));
  if (!rawId || !isLandmarkId(rawId)) {
    throw new Error(
      `Usage: bun run scripts/object-lab.ts --id <landmark-id> [--seed 1337]\n`
        + `       bun run scripts/object-lab.ts --batch route-landmarks [--seed 1337]\n`
        + `       bun run scripts/object-lab.ts --ids ash_marker,pilgrim_lantern [--seed 1337]`,
    );
  }
  return {
    landmarkId: rawId,
    seed: readPositiveInt(readFlag(args, "--seed"), DEFAULT_SEED),
    outputDir: readFlag(args, "--output-dir") ?? DEFAULT_OUTPUT_DIR,
    label: readFlag(args, "--label") ?? rawId,
    scanRadius: readPositiveInt(readFlag(args, "--scan-radius"), DEFAULT_SCAN_RADIUS),
    coarseStep: readPositiveInt(readFlag(args, "--coarse-step"), DEFAULT_COARSE_STEP),
    refineRadius: readPositiveInt(readFlag(args, "--refine-radius"), DEFAULT_REFINE_RADIUS),
    sampleRadius: readPositiveInt(readFlag(args, "--sample-radius"), DEFAULT_SAMPLE_RADIUS),
    heightPadding: readNonNegativeInt(readFlag(args, "--height-padding"), DEFAULT_HEIGHT_PADDING),
    worldX: readOptionalInt(readFlag(args, "--world-x")),
    worldZ: readOptionalInt(readFlag(args, "--world-z")),
  };
}

function readLandmarkIds(args: string[]): LandmarkId[] | null {
  const batch = readFlag(args, "--batch");
  const rawIds = readFlag(args, "--ids");
  if (batch === null && rawIds === null) {
    return null;
  }
  if (rawIds !== null) {
    const ids = rawIds.split(",").map((id) => id.trim()).filter(Boolean);
    if (ids.length === 0) {
      throw new Error("--ids must include at least one landmark id.");
    }
    const invalid = ids.find((id) => !isLandmarkId(id));
    if (invalid) {
      throw new Error(`Unknown landmark id '${invalid}'.`);
    }
    return ids as LandmarkId[];
  }
  if (batch === "route-landmarks") {
    return [...OBJECT_LAB_ROUTE_LANDMARK_IDS];
  }
  throw new Error(`Unknown object-lab batch '${batch}'. Supported batch: route-landmarks.`);
}

function isLandmarkId(value: string): value is LandmarkId {
  return LANDMARK_ID_SET.has(value);
}

function readFlag(args: string[], flag: string): string | null {
  const exact = args.find((arg) => arg.startsWith(`${flag}=`));
  if (exact) {
    return exact.slice(flag.length + 1);
  }
  const index = args.indexOf(flag);
  if (index === -1) {
    return null;
  }
  return args[index + 1] ?? null;
}

function readPositiveInt(raw: string | null, fallback: number): number {
  const parsed = Number.parseInt(raw ?? "", 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

function readNonNegativeInt(raw: string | null, fallback: number): number {
  const parsed = Number.parseInt(raw ?? "", 10);
  return Number.isFinite(parsed) && parsed >= 0 ? parsed : fallback;
}

function readOptionalInt(raw: string | null): number | undefined {
  if (raw === null) {
    return undefined;
  }
  const parsed = Number.parseInt(raw, 10);
  return Number.isFinite(parsed) ? parsed : undefined;
}

function timestampForFile(date: Date): string {
  return date.toISOString().replace(/[:.]/g, "").replace("T", "-").replace("Z", "Z");
}

function sanitizeFileStem(value: string): string {
  const sanitized = value.toLowerCase().replace(/[^a-z0-9_-]+/g, "-").replace(/^-+|-+$/g, "");
  return sanitized || "object";
}
