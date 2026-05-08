import { join } from "node:path";
import { readdir, readFile } from "node:fs/promises";

import { resolveAmbientWorldProfile } from "../src/engine/ambient-environment.ts";
import { ProceduralWorldGenerator } from "../src/engine/procedural-generator.ts";
import { metersToWorldUnits, worldUnitsToMeters } from "../src/engine/scale.ts";

interface RouteSpec {
  label: string;
  startMeters: readonly [number, number];
  headingDegrees: number;
  lengthMeters: number;
  stepMeters: number;
  requiredLandmarkIds?: readonly string[];
}

interface RouteSample {
  distanceMeters: number;
  worldX: number;
  worldZ: number;
  surfaceMeters: number;
  biomeId: string;
  undergroundBiomeId: string;
  regionalVariantId: string | null;
  landmarkId: string | null;
  landmarkHeightMeters: number;
  visibleNearbyLandmarkIds: string[];
  nearestVisibleNearbyLandmarkId: string | null;
  nearestVisibleNearbyLandmarkDistanceMeters: number | null;
  tallestVisibleNearbyLandmarkHeightMeters: number;
  ambientProfileId: string;
  ambientProfileLabel: string;
}

interface RouteSummary {
  label: string;
  sampleCount: number;
  lengthMeters: number;
  distinctBiomes: string[];
  distinctUndergroundBiomes: string[];
  distinctRegionalVariants: string[];
  distinctLandmarks: string[];
  directLandmarks: string[];
  visibleNearbyLandmarks: string[];
  requiredLandmarkIds: string[];
  missingRequiredLandmarkIds: string[];
  distinctAmbientProfiles: string[];
  directLandmarkHitCount: number;
  visibleNearbyLandmarkHitCount: number;
  landmarkHitCount: number;
  maxNotableGapMeters: number;
  averageNotableGapMeters: number;
  minSurfaceMeters: number;
  maxSurfaceMeters: number;
  samplePreview: RouteSample[];
}

interface RouteAtlasReport {
  generatedAt: string;
  thresholds: typeof thresholds;
  landmarkVistaScan: typeof landmarkVistaScan;
  enforce: boolean;
  aggregate: RouteAtlasAggregate;
  comparison: RouteAtlasComparison | null;
  routes: RouteSummary[];
  failures: string[];
}

interface RouteAtlasAggregate {
  routeCount: number;
  sampleCount: number;
  distinctBiomes: string[];
  distinctAmbientProfiles: string[];
  distinctRegionalVariants: string[];
  distinctLandmarks: string[];
  directLandmarks: string[];
  visibleNearbyLandmarks: string[];
  directLandmarkHitCount: number;
  visibleNearbyLandmarkHitCount: number;
  landmarkHitCount: number;
  maxNotableGapMeters: number;
  averageNotableGapMeters: number;
  definitionScore: number;
}

interface VisibleNearbyLandmark {
  landmarkId: string;
  distanceMeters: number;
  heightMeters: number;
  worldX: number;
  worldZ: number;
}

interface RouteAtlasComparison {
  baselinePath: string;
  baselineGeneratedAt: string | null;
  metricDeltas: {
    routeCount: number;
    sampleCount: number;
    distinctBiomeCount: number;
    distinctAmbientProfileCount: number;
    distinctRegionalVariantCount: number;
    distinctLandmarkCount: number;
    landmarkHitCount: number;
    maxNotableGapMeters: number;
    averageNotableGapMeters: number;
    definitionScore: number;
  };
  addedLandmarks: string[];
  removedLandmarks: string[];
  addedAmbientProfiles: string[];
  removedAmbientProfiles: string[];
  routeDeltas: Array<{
    label: string;
    landmarkHitCount: number;
    maxNotableGapMeters: number;
    addedLandmarks: string[];
    removedLandmarks: string[];
  }>;
}

const ROUTES: RouteSpec[] = [
  { label: "origin-east", startMeters: [0, 0], headingDegrees: 8, lengthMeters: 1800, stepMeters: 12 },
  { label: "origin-northwest", startMeters: [0, 0], headingDegrees: 126, lengthMeters: 1800, stepMeters: 12 },
  { label: "silt-crossing", startMeters: [220, -340], headingDegrees: 54, lengthMeters: 2200, stepMeters: 12 },
  { label: "highland-run", startMeters: [-540, 420], headingDegrees: 315, lengthMeters: 2200, stepMeters: 12 },
  { label: "ash-glass-traverse", startMeters: [960, -780], headingDegrees: 202, lengthMeters: 2600, stepMeters: 12 },
  { label: "ancestor-march", startMeters: [-1152.8, -1992.8], headingDegrees: 0, lengthMeters: 480, stepMeters: 6 },
  { label: "ancestor-pillar-road", startMeters: [66.4, -1997.6], headingDegrees: 0, lengthMeters: 480, stepMeters: 6, requiredLandmarkIds: ["ancestor_pillar"] },
  { label: "ash-marker-road", startMeters: [236, -4604], headingDegrees: 0, lengthMeters: 480, stepMeters: 6, requiredLandmarkIds: ["ash_marker"] },
  { label: "silt-shell-road", startMeters: [-1900, -2323.2], headingDegrees: 0, lengthMeters: 650, stepMeters: 6, requiredLandmarkIds: ["silt_shell"] },
  { label: "velothi-shrine-road", startMeters: [-1427.2, -2348.8], headingDegrees: 0, lengthMeters: 240, stepMeters: 3, requiredLandmarkIds: ["velothi_shrine"] },
  { label: "kwama-mound-road", startMeters: [-1360, -2568], headingDegrees: 0, lengthMeters: 120, stepMeters: 1.6, requiredLandmarkIds: ["kwama_mound"] },
  { label: "pilgrim-cairn-road", startMeters: [-1260, -2593.6], headingDegrees: 0, lengthMeters: 120, stepMeters: 1.6, requiredLandmarkIds: ["pilgrim_cairn"] },
  { label: "ziggurat-vista-road", startMeters: [-1704.8, -2536.9], headingDegrees: 0, lengthMeters: 160, stepMeters: 2, requiredLandmarkIds: ["velothi_ziggurat"] },
  { label: "ash-obelisk-road", startMeters: [-1658.1, -2508.8], headingDegrees: 0, lengthMeters: 120, stepMeters: 2, requiredLandmarkIds: ["ash_obelisk"] },
  { label: "rib-arch-road", startMeters: [-1171.7, -2546.4], headingDegrees: 0, lengthMeters: 120, stepMeters: 2, requiredLandmarkIds: ["rib_arch"] },
  { label: "causeway-road", startMeters: [19.6, -3277.1], headingDegrees: 0, lengthMeters: 120, stepMeters: 2, requiredLandmarkIds: ["old_road_causeway"] },
  { label: "pilgrim-lantern-road", startMeters: [18.6, -3245.3], headingDegrees: 0, lengthMeters: 120, stepMeters: 2, requiredLandmarkIds: ["pilgrim_lantern"] },
  { label: "crystal-reeds-basin", startMeters: [2790.2, -2893.3], headingDegrees: 0, lengthMeters: 120, stepMeters: 2, requiredLandmarkIds: ["crystal_reeds"] },
  { label: "fungal-bridge-basin", startMeters: [844.8, -2899.9], headingDegrees: 0, lengthMeters: 120, stepMeters: 2, requiredLandmarkIds: ["fungal_bridge"] },
  { label: "rib-remains-basin", startMeters: [1727.5, 1753.3], headingDegrees: 0, lengthMeters: 120, stepMeters: 2, requiredLandmarkIds: ["rib_remains"] },
];

const thresholds = {
  minDistinctBiomes: 8,
  minDistinctAmbientProfiles: 5,
  minDistinctRegionalVariants: 3,
  minLandmarkHits: 8,
  maxNotableGapMeters: 540,
} as const;

const landmarkVistaScan = {
  radiusMeters: 64,
  minHeightMeters: 2.5,
  samplesPerRouteSample: 16,
} as const;

const LANDMARK_VISTA_OFFSETS_METERS: ReadonlyArray<readonly [forwardMeters: number, lateralMeters: number]> = [
  [0, -12],
  [0, 12],
  [0, -24],
  [0, 24],
  [0, -48],
  [0, 48],
  [24, -24],
  [24, 24],
  [-24, -24],
  [-24, 24],
  [36, -48],
  [36, 48],
  [-36, -48],
  [-36, 48],
  [48, 0],
  [-48, 0],
] as const;

if (LANDMARK_VISTA_OFFSETS_METERS.length !== landmarkVistaScan.samplesPerRouteSample) {
  throw new Error("landmark vista scan sample count does not match offset table");
}

const args = Bun.argv.slice(2);
const label = readFlag(args, "--label");
const outputDir = readFlag(args, "--output-dir") ?? "artifacts/route-atlas";
const enforce = readBooleanFlag(args, "--enforce", true);
const compareTo = readFlag(args, "--compare-to");
const runStamp = timestampForFile(new Date());
const runName = `${runStamp}${label ? `-${sanitizeFileStem(label)}` : ""}`;
const runDir = join(outputDir, runName);
const reportPath = join(runDir, "report.json");
const summaryPath = join(runDir, "summary.md");

await Bun.$`mkdir -p ${runDir}`.quiet();

const generator = new ProceduralWorldGenerator(1337);
const routeSummaries = ROUTES.map((route) => summarizeRoute(generator, route));
const aggregate = summarizeAggregate(routeSummaries);
const failures = enforce ? findFailures(aggregate, routeSummaries) : [];
const baselineReportPath = compareTo ?? await findPreviousReportPath(outputDir, runName);
const comparison = baselineReportPath ? await compareWithBaseline(baselineReportPath, aggregate, routeSummaries) : null;
const report: RouteAtlasReport = {
  generatedAt: new Date().toISOString(),
  thresholds,
  landmarkVistaScan,
  enforce,
  aggregate,
  comparison,
  routes: routeSummaries,
  failures,
};

await Bun.write(reportPath, `${JSON.stringify(report, null, 2)}\n`);
await Bun.write(summaryPath, buildMarkdownSummary(report));

console.log(`route-atlas report: ${reportPath}`);
console.log(`summary: ${summaryPath}`);
console.log(`biomes: ${aggregate.distinctBiomes.length} (${aggregate.distinctBiomes.join(", ")})`);
console.log(`ambient profiles: ${aggregate.distinctAmbientProfiles.length} (${aggregate.distinctAmbientProfiles.join(", ")})`);
console.log(`regional variants: ${aggregate.distinctRegionalVariants.length}`);
console.log(`landmark credited samples: ${aggregate.landmarkHitCount} (${aggregate.directLandmarkHitCount} direct samples, ${aggregate.visibleNearbyLandmarkHitCount} vista samples)`);
console.log(`max notable gap: ${aggregate.maxNotableGapMeters.toFixed(1)} m`);
console.log(`definition score: ${aggregate.definitionScore.toFixed(2)} / 5`);
if (comparison) {
  console.log(`compared to: ${comparison.baselinePath}`);
  console.log(`landmark delta: ${formatSigned(comparison.metricDeltas.landmarkHitCount)} hits, ${formatSigned(comparison.metricDeltas.distinctLandmarkCount)} distinct`);
  console.log(`gap delta: ${formatSigned(comparison.metricDeltas.maxNotableGapMeters, 1)} m max`);
  console.log(`added landmarks: ${comparison.addedLandmarks.length > 0 ? comparison.addedLandmarks.join(", ") : "none"}`);
}
if (failures.length > 0) {
  console.log(`failures: ${failures.join("; ")}`);
  process.exitCode = 1;
} else {
  console.log("failures: none");
}

function summarizeRoute(generator: ProceduralWorldGenerator, route: RouteSpec): RouteSummary {
  const samples = sampleRoute(generator, route);
  const distinctBiomes = sortedDistinct(samples.map((sample) => sample.biomeId));
  const distinctUndergroundBiomes = sortedDistinct(samples.map((sample) => sample.undergroundBiomeId));
  const distinctRegionalVariants = sortedDistinct(samples.map((sample) => sample.regionalVariantId).filter(isString));
  const directLandmarks = sortedDistinct(samples.map((sample) => sample.landmarkId).filter(isString));
  const visibleNearbyLandmarks = sortedDistinct(samples.flatMap((sample) => sample.visibleNearbyLandmarkIds));
  const distinctLandmarks = sortedDistinct([...directLandmarks, ...visibleNearbyLandmarks]);
  const requiredLandmarkIds = [...(route.requiredLandmarkIds ?? [])].sort();
  const missingRequiredLandmarkIds = requiredLandmarkIds.filter((landmarkId) => !distinctLandmarks.includes(landmarkId));
  const distinctAmbientProfiles = sortedDistinct(samples.map((sample) => sample.ambientProfileId));
  const notableGaps = computeNotableGaps(samples);
  const surfaces = samples.map((sample) => sample.surfaceMeters);
  const directLandmarkHitCount = samples.filter((sample) => sample.landmarkId !== null).length;
  const visibleNearbyLandmarkHitCount = samples.filter((sample) => sample.visibleNearbyLandmarkIds.length > 0).length;
  return {
    label: route.label,
    sampleCount: samples.length,
    lengthMeters: route.lengthMeters,
    distinctBiomes,
    distinctUndergroundBiomes,
    distinctRegionalVariants,
    distinctLandmarks,
    directLandmarks,
    visibleNearbyLandmarks,
    requiredLandmarkIds,
    missingRequiredLandmarkIds,
    distinctAmbientProfiles,
    directLandmarkHitCount,
    visibleNearbyLandmarkHitCount,
    landmarkHitCount: samples.filter((sample) => sample.landmarkId !== null || sample.visibleNearbyLandmarkIds.length > 0).length,
    maxNotableGapMeters: Math.max(0, ...notableGaps),
    averageNotableGapMeters: average(notableGaps),
    minSurfaceMeters: Math.min(...surfaces),
    maxSurfaceMeters: Math.max(...surfaces),
    samplePreview: samples.filter((_, index) => index % 12 === 0).slice(0, 24),
  };
}

function sampleRoute(generator: ProceduralWorldGenerator, route: RouteSpec): RouteSample[] {
  const heading = route.headingDegrees * Math.PI / 180;
  const directionX = Math.cos(heading);
  const directionZ = Math.sin(heading);
  const startX = metersToWorldUnits(route.startMeters[0]);
  const startZ = metersToWorldUnits(route.startMeters[1]);
  const samples: RouteSample[] = [];
  for (let distanceMeters = 0; distanceMeters <= route.lengthMeters; distanceMeters += route.stepMeters) {
    const distanceWorldUnits = metersToWorldUnits(distanceMeters);
    const worldX = startX + directionX * distanceWorldUnits;
    const worldZ = startZ + directionZ * distanceWorldUnits;
    const probe = generator.sampleBiomeProbe(worldX, worldZ);
    const visibleNearbyLandmarks = scanVisibleNearbyLandmarks(generator, worldX, worldZ, directionX, directionZ, probe.landmarkId);
    const ambient = resolveAmbientWorldProfile(probe);
    samples.push({
      distanceMeters,
      worldX: Math.round(worldX),
      worldZ: Math.round(worldZ),
      surfaceMeters: worldUnitsToMeters(probe.surfaceY),
      biomeId: probe.biomeId,
      undergroundBiomeId: probe.undergroundBiomeId,
      regionalVariantId: probe.regionalVariantId,
      landmarkId: probe.landmarkId,
      landmarkHeightMeters: landmarkHeightMeters(probe),
      visibleNearbyLandmarkIds: visibleNearbyLandmarks.map((landmark) => landmark.landmarkId),
      nearestVisibleNearbyLandmarkId: visibleNearbyLandmarks[0]?.landmarkId ?? null,
      nearestVisibleNearbyLandmarkDistanceMeters: visibleNearbyLandmarks[0]?.distanceMeters ?? null,
      tallestVisibleNearbyLandmarkHeightMeters: Math.max(0, ...visibleNearbyLandmarks.map((landmark) => landmark.heightMeters)),
      ambientProfileId: ambient.id,
      ambientProfileLabel: ambient.label,
    });
  }
  return samples;
}

function scanVisibleNearbyLandmarks(
  generator: ProceduralWorldGenerator,
  worldX: number,
  worldZ: number,
  directionX: number,
  directionZ: number,
  directLandmarkId: string | null,
): VisibleNearbyLandmark[] {
  const lateralX = -directionZ;
  const lateralZ = directionX;
  const nearestByLandmark = new Map<string, VisibleNearbyLandmark>();
  for (const [forwardMeters, lateralMeters] of LANDMARK_VISTA_OFFSETS_METERS) {
    const distanceMeters = Math.hypot(forwardMeters, lateralMeters);
    if (distanceMeters > landmarkVistaScan.radiusMeters) {
      throw new Error(`landmark vista offset ${forwardMeters},${lateralMeters} exceeds scan radius`);
    }
    const offsetWorldX = metersToWorldUnits(forwardMeters * directionX + lateralMeters * lateralX);
    const offsetWorldZ = metersToWorldUnits(forwardMeters * directionZ + lateralMeters * lateralZ);
    const probeWorldX = worldX + offsetWorldX;
    const probeWorldZ = worldZ + offsetWorldZ;
    const probe = generator.sampleBiomeProbe(probeWorldX, probeWorldZ);
    if (!probe.landmarkId || probe.landmarkId === directLandmarkId) {
      continue;
    }
    const heightMeters = landmarkHeightMeters(probe);
    if (heightMeters < landmarkVistaScan.minHeightMeters) {
      continue;
    }
    const candidate: VisibleNearbyLandmark = {
      landmarkId: probe.landmarkId,
      distanceMeters,
      heightMeters,
      worldX: Math.round(probeWorldX),
      worldZ: Math.round(probeWorldZ),
    };
    const existing = nearestByLandmark.get(probe.landmarkId);
    if (!existing || compareVisibleNearbyLandmarks(candidate, existing) < 0) {
      nearestByLandmark.set(probe.landmarkId, candidate);
    }
  }
  return [...nearestByLandmark.values()].sort(compareVisibleNearbyLandmarks);
}

function compareVisibleNearbyLandmarks(left: VisibleNearbyLandmark, right: VisibleNearbyLandmark): number {
  return left.distanceMeters - right.distanceMeters
    || right.heightMeters - left.heightMeters
    || left.landmarkId.localeCompare(right.landmarkId)
    || left.worldX - right.worldX
    || left.worldZ - right.worldZ;
}

function landmarkHeightMeters(probe: { topY: number; surfaceY: number }): number {
  return worldUnitsToMeters(Math.max(0, probe.topY - probe.surfaceY));
}

function summarizeAggregate(routes: RouteSummary[]): RouteAtlasAggregate {
  const distinctBiomes = sortedDistinct(routes.flatMap((route) => route.distinctBiomes));
  const distinctAmbientProfiles = sortedDistinct(routes.flatMap((route) => route.distinctAmbientProfiles));
  const distinctRegionalVariants = sortedDistinct(routes.flatMap((route) => route.distinctRegionalVariants));
  const distinctLandmarks = sortedDistinct(routes.flatMap((route) => route.distinctLandmarks));
  const directLandmarks = sortedDistinct(routes.flatMap((route) => route.directLandmarks));
  const visibleNearbyLandmarks = sortedDistinct(routes.flatMap((route) => route.visibleNearbyLandmarks));
  const directLandmarkHitCount = sum(routes.map((route) => route.directLandmarkHitCount));
  const visibleNearbyLandmarkHitCount = sum(routes.map((route) => route.visibleNearbyLandmarkHitCount));
  const landmarkHitCount = sum(routes.map((route) => route.landmarkHitCount));
  const maxNotableGapMeters = Math.max(...routes.map((route) => route.maxNotableGapMeters));
  const definitionScore = clampScore(
    distinctBiomes.length / thresholds.minDistinctBiomes
      + distinctAmbientProfiles.length / thresholds.minDistinctAmbientProfiles
      + distinctRegionalVariants.length / thresholds.minDistinctRegionalVariants
      + landmarkHitCount / thresholds.minLandmarkHits
      + thresholds.maxNotableGapMeters / Math.max(thresholds.maxNotableGapMeters, maxNotableGapMeters),
  );
  return {
    routeCount: routes.length,
    sampleCount: sum(routes.map((route) => route.sampleCount)),
    distinctBiomes,
    distinctAmbientProfiles,
    distinctRegionalVariants,
    distinctLandmarks,
    directLandmarks,
    visibleNearbyLandmarks,
    directLandmarkHitCount,
    visibleNearbyLandmarkHitCount,
    landmarkHitCount,
    maxNotableGapMeters,
    averageNotableGapMeters: average(routes.map((route) => route.averageNotableGapMeters)),
    definitionScore,
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
    const previousReportPath = join(outputDir, previousRunName, "report.json");
    try {
      JSON.parse(await readFile(previousReportPath, "utf8")) as Partial<RouteAtlasReport>;
      return previousReportPath;
    } catch {
      // Ignore malformed or incomplete artifacts and keep searching.
    }
  }
  return null;
}

async function compareWithBaseline(
  baselinePath: string,
  aggregate: RouteAtlasAggregate,
  routes: readonly RouteSummary[],
): Promise<RouteAtlasComparison | null> {
  let baseline: Partial<RouteAtlasReport>;
  try {
    baseline = JSON.parse(await readFile(baselinePath, "utf8")) as Partial<RouteAtlasReport>;
  } catch {
    return null;
  }
  if (!baseline.aggregate || !Array.isArray(baseline.routes)) {
    return null;
  }
  const baselineAggregate = baseline.aggregate;
  const baselineRoutes = new Map(baseline.routes.map((route) => [route.label, route]));
  return {
    baselinePath,
    baselineGeneratedAt: typeof baseline.generatedAt === "string" ? baseline.generatedAt : null,
    metricDeltas: {
      routeCount: aggregate.routeCount - readNumber(baselineAggregate.routeCount),
      sampleCount: aggregate.sampleCount - readNumber(baselineAggregate.sampleCount),
      distinctBiomeCount: aggregate.distinctBiomes.length - readArray(baselineAggregate.distinctBiomes).length,
      distinctAmbientProfileCount: aggregate.distinctAmbientProfiles.length - readArray(baselineAggregate.distinctAmbientProfiles).length,
      distinctRegionalVariantCount: aggregate.distinctRegionalVariants.length - readArray(baselineAggregate.distinctRegionalVariants).length,
      distinctLandmarkCount: aggregate.distinctLandmarks.length - readArray(baselineAggregate.distinctLandmarks).length,
      landmarkHitCount: aggregate.landmarkHitCount - readNumber(baselineAggregate.landmarkHitCount),
      maxNotableGapMeters: aggregate.maxNotableGapMeters - readNumber(baselineAggregate.maxNotableGapMeters),
      averageNotableGapMeters: aggregate.averageNotableGapMeters - readNumber(baselineAggregate.averageNotableGapMeters),
      definitionScore: aggregate.definitionScore - readNumber(baselineAggregate.definitionScore),
    },
    addedLandmarks: setDifference(aggregate.distinctLandmarks, readArray(baselineAggregate.distinctLandmarks)),
    removedLandmarks: setDifference(readArray(baselineAggregate.distinctLandmarks), aggregate.distinctLandmarks),
    addedAmbientProfiles: setDifference(aggregate.distinctAmbientProfiles, readArray(baselineAggregate.distinctAmbientProfiles)),
    removedAmbientProfiles: setDifference(readArray(baselineAggregate.distinctAmbientProfiles), aggregate.distinctAmbientProfiles),
    routeDeltas: routes.map((route) => {
      const baselineRoute = baselineRoutes.get(route.label);
      return {
        label: route.label,
        landmarkHitCount: route.landmarkHitCount - readNumber(baselineRoute?.landmarkHitCount),
        maxNotableGapMeters: route.maxNotableGapMeters - readNumber(baselineRoute?.maxNotableGapMeters),
        addedLandmarks: setDifference(route.distinctLandmarks, readArray(baselineRoute?.distinctLandmarks)),
        removedLandmarks: setDifference(readArray(baselineRoute?.distinctLandmarks), route.distinctLandmarks),
      };
    }),
  };
}

function buildMarkdownSummary(report: RouteAtlasReport): string {
  const lines: string[] = [
    "# Route Atlas Summary",
    "",
    `Generated: ${report.generatedAt}`,
    "",
    "## Aggregate",
    "",
    `- Definition score: ${report.aggregate.definitionScore.toFixed(2)} / 5`,
    `- Routes: ${report.aggregate.routeCount}`,
    `- Samples: ${report.aggregate.sampleCount}`,
    `- Biomes: ${report.aggregate.distinctBiomes.length} (${report.aggregate.distinctBiomes.join(", ")})`,
    `- Ambient profiles: ${report.aggregate.distinctAmbientProfiles.length} (${report.aggregate.distinctAmbientProfiles.join(", ")})`,
    `- Regional variants: ${report.aggregate.distinctRegionalVariants.length}`,
    `- Landmarks: ${report.aggregate.distinctLandmarks.length} (${report.aggregate.distinctLandmarks.join(", ")})`,
    `- Landmark credited samples: ${report.aggregate.landmarkHitCount} (${report.aggregate.directLandmarkHitCount} direct samples, ${report.aggregate.visibleNearbyLandmarkHitCount} vista samples)`,
    `- Vista scan: ${report.landmarkVistaScan.samplesPerRouteSample} samples within ${report.landmarkVistaScan.radiusMeters} m, min height ${report.landmarkVistaScan.minHeightMeters.toFixed(1)} m`,
    `- Max notable gap: ${report.aggregate.maxNotableGapMeters.toFixed(1)} m`,
    `- Average notable gap: ${report.aggregate.averageNotableGapMeters.toFixed(1)} m`,
    "",
  ];
  if (report.comparison) {
    lines.push(
      "## Comparison",
      "",
      `Baseline: ${report.comparison.baselinePath}`,
      "",
      "| Metric | Delta |",
      "| --- | ---: |",
      `| Routes | ${formatSigned(report.comparison.metricDeltas.routeCount)} |`,
      `| Samples | ${formatSigned(report.comparison.metricDeltas.sampleCount)} |`,
      `| Distinct biomes | ${formatSigned(report.comparison.metricDeltas.distinctBiomeCount)} |`,
      `| Distinct ambient profiles | ${formatSigned(report.comparison.metricDeltas.distinctAmbientProfileCount)} |`,
      `| Distinct regional variants | ${formatSigned(report.comparison.metricDeltas.distinctRegionalVariantCount)} |`,
      `| Distinct landmarks | ${formatSigned(report.comparison.metricDeltas.distinctLandmarkCount)} |`,
      `| Landmark hits | ${formatSigned(report.comparison.metricDeltas.landmarkHitCount)} |`,
      `| Max notable gap | ${formatSigned(report.comparison.metricDeltas.maxNotableGapMeters, 1)} m |`,
      `| Average notable gap | ${formatSigned(report.comparison.metricDeltas.averageNotableGapMeters, 1)} m |`,
      `| Definition score | ${formatSigned(report.comparison.metricDeltas.definitionScore, 2)} |`,
      "",
      `- Added landmarks: ${formatList(report.comparison.addedLandmarks)}`,
      `- Removed landmarks: ${formatList(report.comparison.removedLandmarks)}`,
      `- Added ambient profiles: ${formatList(report.comparison.addedAmbientProfiles)}`,
      `- Removed ambient profiles: ${formatList(report.comparison.removedAmbientProfiles)}`,
      "",
      "### Route Deltas",
      "",
      "| Route | Landmark Hits | Max Gap | Added Landmarks | Removed Landmarks |",
      "| --- | ---: | ---: | --- | --- |",
      ...report.comparison.routeDeltas.map((route) =>
        `| ${route.label} | ${formatSigned(route.landmarkHitCount)} | ${formatSigned(route.maxNotableGapMeters, 1)} m | ${formatList(route.addedLandmarks)} | ${formatList(route.removedLandmarks)} |`,
      ),
      "",
    );
  }
  lines.push(
    "## Route Details",
    "",
    "| Route | Biomes | Ambient | Landmarks | Vista | Required Missing | Hits | Max Gap |",
    "| --- | ---: | ---: | --- | --- | --- | ---: | ---: |",
    ...report.routes.map((route) =>
      `| ${route.label} | ${route.distinctBiomes.length} | ${route.distinctAmbientProfiles.length} | ${formatList(route.distinctLandmarks)} | ${formatList(route.visibleNearbyLandmarks)} | ${formatList(route.missingRequiredLandmarkIds)} | ${route.landmarkHitCount} | ${route.maxNotableGapMeters.toFixed(1)} m |`,
    ),
    "",
    `Failures: ${report.failures.length > 0 ? report.failures.join("; ") : "none"}`,
    "",
  );
  return `${lines.join("\n")}\n`;
}

function findFailures(
  aggregate: ReturnType<typeof summarizeAggregate>,
  routes: readonly RouteSummary[],
): string[] {
  const failures: string[] = [];
  if (aggregate.distinctBiomes.length < thresholds.minDistinctBiomes) {
    failures.push(`only ${aggregate.distinctBiomes.length} distinct biomes across atlas`);
  }
  if (aggregate.distinctAmbientProfiles.length < thresholds.minDistinctAmbientProfiles) {
    failures.push(`only ${aggregate.distinctAmbientProfiles.length} distinct ambient profiles across atlas`);
  }
  if (aggregate.distinctRegionalVariants.length < thresholds.minDistinctRegionalVariants) {
    failures.push(`only ${aggregate.distinctRegionalVariants.length} regional variants across atlas`);
  }
  if (aggregate.landmarkHitCount < thresholds.minLandmarkHits) {
    failures.push(`only ${aggregate.landmarkHitCount} landmark route hits`);
  }
  if (aggregate.maxNotableGapMeters > thresholds.maxNotableGapMeters) {
    failures.push(`max notable gap ${aggregate.maxNotableGapMeters.toFixed(1)} m exceeds ${thresholds.maxNotableGapMeters} m`);
  }
  for (const route of routes) {
    if (route.missingRequiredLandmarkIds.length > 0) {
      failures.push(`${route.label} missing required landmark(s): ${route.missingRequiredLandmarkIds.join(", ")}`);
    }
  }
  return failures;
}

function computeNotableGaps(samples: readonly RouteSample[]): number[] {
  if (samples.length < 2) {
    return [];
  }
  const gaps: number[] = [];
  let lastNotableDistance = samples[0]!.distanceMeters;
  let lastKey = notableKey(samples[0]!);
  for (const sample of samples.slice(1)) {
    const key = notableKey(sample);
    if (key === lastKey) {
      continue;
    }
    gaps.push(sample.distanceMeters - lastNotableDistance);
    lastNotableDistance = sample.distanceMeters;
    lastKey = key;
  }
  const finalDistance = samples[samples.length - 1]!.distanceMeters;
  gaps.push(finalDistance - lastNotableDistance);
  return gaps;
}

function notableKey(sample: RouteSample): string {
  return [
    sample.biomeId,
    sample.regionalVariantId ?? "none",
    landmarkKey(sample),
    sample.ambientProfileId,
  ].join("|");
}

function landmarkKey(sample: RouteSample): string {
  return formatList(sortedDistinct([
    ...(sample.landmarkId ? [sample.landmarkId] : []),
    ...sample.visibleNearbyLandmarkIds,
  ]));
}

function sortedDistinct(values: readonly string[]): string[] {
  return [...new Set(values)].sort();
}

function setDifference(left: readonly string[], right: readonly string[]): string[] {
  const rightSet = new Set(right);
  return left.filter((value) => !rightSet.has(value)).sort();
}

function readArray(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((entry): entry is string => typeof entry === "string") : [];
}

function readNumber(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

function formatSigned(value: number, fractionDigits = 0): string {
  const rounded = value.toFixed(fractionDigits);
  return value > 0 ? `+${rounded}` : rounded;
}

function formatList(values: readonly string[]): string {
  return values.length > 0 ? values.join(", ") : "none";
}

function isString(value: string | null): value is string {
  return value !== null;
}

function sum(values: readonly number[]): number {
  return values.reduce((total, value) => total + value, 0);
}

function average(values: readonly number[]): number {
  return values.length === 0 ? 0 : sum(values) / values.length;
}

function clampScore(rawScore: number): number {
  return Math.max(1, Math.min(5, rawScore));
}

function readFlag(args: readonly string[], flag: string): string | null {
  const exact = args.find((arg) => arg.startsWith(`${flag}=`));
  if (exact) return exact.slice(flag.length + 1);
  const index = args.indexOf(flag);
  if (index === -1) return null;
  return args[index + 1] ?? null;
}

function readBooleanFlag(args: readonly string[], flag: string, fallback: boolean): boolean {
  const value = readFlag(args, flag);
  if (value === null) return fallback;
  return value === "1" || value === "true" || value === "yes";
}

function timestampForFile(date: Date): string {
  return date.toISOString().replace(/[-:]/g, "").replace(/\.\d{3}Z$/, "Z");
}

function sanitizeFileStem(value: string): string {
  return value.toLowerCase().replace(/[^a-z0-9._-]+/g, "-").replace(/^-+|-+$/g, "");
}
