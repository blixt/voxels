import { join } from "node:path";
import { readdir, readFile } from "node:fs/promises";

import { resolveAmbientWorldProfile } from "../src/engine/ambient-environment.ts";
import { ProceduralWorldGenerator } from "../src/engine/procedural-generator.ts";
import { metersToWorldUnits, worldUnitsToMeters } from "../src/engine/scale.ts";
import type { WorldRegionId } from "../src/engine/worldgen-region.ts";

interface RouteSpec {
  label: string;
  startMeters: readonly [number, number];
  headingDegrees: number;
  lengthMeters: number;
  stepMeters: number;
  requiredLandmarkIds?: readonly string[];
  expectedRegionId?: WorldRegionId;
  minExpectedRegionCoverage?: number;
  expectedRegionalVariantId?: string;
  minExpectedRegionalVariantCoverage?: number;
}

interface RouteSample {
  distanceMeters: number;
  worldX: number;
  worldZ: number;
  surfaceMeters: number;
  regionId: string;
  biomeId: string;
  undergroundBiomeId: string;
  regionalVariantId: string | null;
  landmarkId: string | null;
  landmarkHeightMeters: number;
  visibleNearbyLandmarkIds: string[];
  visibleNearbyStrongLandmarkIds: string[];
  nearestVisibleNearbyLandmarkId: string | null;
  nearestVisibleNearbyLandmarkDistanceMeters: number | null;
  tallestVisibleNearbyLandmarkHeightMeters: number;
  terrainTokenId: string | null;
  ambientProfileId: string;
  ambientProfileLabel: string;
}

interface RouteSummary {
  label: string;
  sampleCount: number;
  lengthMeters: number;
  distinctBiomes: string[];
  distinctRegions: string[];
  distinctUndergroundBiomes: string[];
  distinctRegionalVariants: string[];
  distinctLandmarks: string[];
  directLandmarks: string[];
  visibleNearbyLandmarks: string[];
  requiredLandmarkIds: string[];
  missingRequiredLandmarkIds: string[];
  expectedRegionId: string | null;
  expectedRegionCoverageRatio: number | null;
  expectedRegionalVariantId: string | null;
  expectedRegionalVariantCoverageRatio: number | null;
  distinctAmbientProfiles: string[];
  directLandmarkHitCount: number;
  visibleNearbyLandmarkHitCount: number;
  landmarkHitCount: number;
  maxNotableGapMeters: number;
  averageNotableGapMeters: number;
  routeStretchCount: number;
  tokenizedRouteStretchCount: number;
  tokenlessRouteStretchCount: number;
  routeStretchCoverageRatio: number;
  maxTokenlessRouteStretchMeters: number;
  tokenlessRouteStretches: RouteStretchSummary[];
  strongSilhouetteStretchCount: number;
  strongSilhouetteTokenizedStretchCount: number;
  strongSilhouettelessStretchCount: number;
  strongSilhouetteStretchCoverageRatio: number;
  maxStrongSilhouettelessStretchMeters: number;
  strongSilhouettelessStretches: RouteStretchSummary[];
  minSurfaceMeters: number;
  maxSurfaceMeters: number;
  samplePreview: RouteSample[];
}

interface RouteAtlasReport {
  generatedAt: string;
  thresholds: typeof thresholds;
  landmarkVistaScan: typeof landmarkVistaScan;
  routeStretchScan: typeof routeStretchScan;
  strongSilhouetteScan: typeof strongSilhouetteScan;
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
  routeStretchCount: number;
  tokenizedRouteStretchCount: number;
  tokenlessRouteStretchCount: number;
  routeStretchCoverageRatio: number;
  maxTokenlessRouteStretchMeters: number;
  strongSilhouetteStretchCount: number;
  strongSilhouetteTokenizedStretchCount: number;
  strongSilhouettelessStretchCount: number;
  strongSilhouetteStretchCoverageRatio: number;
  maxStrongSilhouettelessStretchMeters: number;
  definitionScore: number;
}

interface VisibleNearbyLandmark {
  landmarkId: string;
  distanceMeters: number;
  heightMeters: number;
  worldX: number;
  worldZ: number;
}

interface RouteStretchSummary {
  startMeters: number;
  endMeters: number;
  tokenCount: number;
  silhouetteTokenIds: string[];
  routeTokenIds: string[];
}

interface RouteTokenEvent {
  distanceMeters: number;
  tokenId: string;
  tokenKind: "silhouette" | "route";
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
    tokenlessRouteStretchCount: number;
    routeStretchCoverageRatio: number;
    strongSilhouettelessStretchCount: number;
    strongSilhouetteStretchCoverageRatio: number;
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
    tokenlessRouteStretchCount: number;
    strongSilhouettelessStretchCount: number;
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
  { label: "ash-marker-road", startMeters: [236, -4604], headingDegrees: 0, lengthMeters: 480, stepMeters: 6, requiredLandmarkIds: ["ash_marker"] },
  { label: "silt-shell-road", startMeters: [-1900, -2323.2], headingDegrees: 0, lengthMeters: 650, stepMeters: 6, requiredLandmarkIds: ["silt_shell"] },
  { label: "ziggurat-vista-road", startMeters: [-1704.8, -2536.9], headingDegrees: 0, lengthMeters: 160, stepMeters: 2, requiredLandmarkIds: ["velothi_ziggurat"] },
  { label: "ash-obelisk-road", startMeters: [-1658.1, -2508.8], headingDegrees: 0, lengthMeters: 120, stepMeters: 2, requiredLandmarkIds: ["ash_obelisk"] },
  { label: "rib-arch-road", startMeters: [-1171.7, -2546.4], headingDegrees: 0, lengthMeters: 120, stepMeters: 2, requiredLandmarkIds: ["rib_arch"] },
  { label: "causeway-road", startMeters: [19.6, -3277.1], headingDegrees: 0, lengthMeters: 120, stepMeters: 2, requiredLandmarkIds: ["old_road_causeway"] },
  { label: "pilgrim-lantern-road", startMeters: [18.6, -3245.3], headingDegrees: 0, lengthMeters: 120, stepMeters: 2, requiredLandmarkIds: ["pilgrim_lantern"] },
  {
    label: "bitter-coast-smuggler-walk",
    startMeters: [-6100, 700],
    headingDegrees: 10,
    lengthMeters: 3000,
    stepMeters: 12,
    requiredLandmarkIds: ["fungal_bridge", "crystal_reeds", "rib_remains"],
    expectedRegionId: "bitter-coast",
    minExpectedRegionCoverage: 0.62,
    expectedRegionalVariantId: "marsh_blackwater",
    minExpectedRegionalVariantCoverage: 0.34,
  },
  {
    label: "bitter-inner-crossing",
    startMeters: [-3000, 700],
    headingDegrees: 352,
    lengthMeters: 2250,
    stepMeters: 12,
    requiredLandmarkIds: ["fungal_bridge", "crystal_reeds", "standing_stone"],
    expectedRegionId: "bitter-coast",
    minExpectedRegionCoverage: 0.40,
    expectedRegionalVariantId: "marsh_blackwater",
    minExpectedRegionalVariantCoverage: 0.22,
  },
  {
    label: "salt-basin-causeway",
    startMeters: [-2100, 3600],
    headingDegrees: 10,
    lengthMeters: 4000,
    stepMeters: 12,
    requiredLandmarkIds: ["old_road_causeway", "salt_spire", "glass_cairn"],
    expectedRegionId: "salt-marsh-basin",
    minExpectedRegionCoverage: 0.60,
    expectedRegionalVariantId: "saltflat_mirror",
    minExpectedRegionalVariantCoverage: 0.42,
  },
  {
    label: "inner-sea-shelf-road",
    startMeters: [-600, -700],
    headingDegrees: 27,
    lengthMeters: 3150,
    stepMeters: 12,
    requiredLandmarkIds: ["standing_stone", "ancestor_pillar", "old_road_causeway"],
    expectedRegionId: "inner-sea",
    minExpectedRegionCoverage: 0.55,
    expectedRegionalVariantId: "moor_shadowglass",
    minExpectedRegionalVariantCoverage: 0.40,
  },
  {
    label: "grazelands-flower-road",
    startMeters: [900, -1800],
    headingDegrees: 333,
    lengthMeters: 3800,
    stepMeters: 12,
    requiredLandmarkIds: ["acacia", "standing_stone", "ancestor_pillar"],
    expectedRegionId: "grazelands",
    minExpectedRegionCoverage: 0.48,
    expectedRegionalVariantId: "savanna_flowersea",
    minExpectedRegionalVariantCoverage: 0.34,
  },
  {
    label: "glass-shard-coast-cairns",
    startMeters: [3000, 900],
    headingDegrees: 32,
    lengthMeters: 3050,
    stepMeters: 12,
    requiredLandmarkIds: ["glass_cairn", "crystal_cluster", "salt_spire"],
    expectedRegionId: "glass-shard-coast",
    minExpectedRegionCoverage: 0.48,
    expectedRegionalVariantId: "dunes_glass",
    minExpectedRegionalVariantCoverage: 0.30,
  },
  {
    label: "west-gash-redleaf-road",
    startMeters: [-4200, -4200],
    headingDegrees: 37,
    lengthMeters: 3150,
    stepMeters: 12,
    requiredLandmarkIds: ["stone_tor", "redleaf_tree", "old_road_causeway"],
    expectedRegionId: "west-gash",
    minExpectedRegionCoverage: 0.54,
    expectedRegionalVariantId: "highland_redleaf",
    minExpectedRegionalVariantCoverage: 0.34,
  },
];

const thresholds = {
  minDistinctBiomes: 8,
  minDistinctAmbientProfiles: 5,
  minDistinctRegionalVariants: 3,
  minLandmarkHits: 8,
  maxNotableGapMeters: 540,
  maxTokenlessRouteStretches: 0,
  minStrongSilhouetteStretchCoverageRatio: 0.80,
  maxStrongSilhouettelessRouteStretches: 35,
} as const;

const landmarkVistaScan = {
  radiusMeters: 96,
  minHeightMeters: 2.5,
  samplesPerRouteSample: 28,
} as const;

const routeStretchScan = {
  windowMeters: 300,
  strideMeters: 50,
  minSilhouetteHeightMeters: landmarkVistaScan.minHeightMeters,
} as const;

const strongSilhouetteScan = {
  windowMeters: 360,
  strideMeters: 60,
  minHeightMeters: 4.0,
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
  [72, 0],
  [-72, 0],
  [72, -24],
  [72, 24],
  [-72, -24],
  [-72, 24],
  [72, -48],
  [72, 48],
  [-72, -48],
  [-72, 48],
  [0, -72],
  [0, 72],
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
  routeStretchScan,
  strongSilhouetteScan,
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
console.log(`route stretch coverage: ${formatPercent(aggregate.routeStretchCoverageRatio)} (${aggregate.tokenizedRouteStretchCount}/${aggregate.routeStretchCount} tokenized, ${aggregate.tokenlessRouteStretchCount} tokenless)`);
console.log(`strong silhouette coverage: ${formatPercent(aggregate.strongSilhouetteStretchCoverageRatio)} (${aggregate.strongSilhouetteTokenizedStretchCount}/${aggregate.strongSilhouetteStretchCount} tokenized, ${aggregate.strongSilhouettelessStretchCount} empty)`);
console.log(`definition score: ${aggregate.definitionScore.toFixed(2)} / 5`);
if (comparison) {
  console.log(`compared to: ${comparison.baselinePath}`);
  console.log(`landmark delta: ${formatSigned(comparison.metricDeltas.landmarkHitCount)} hits, ${formatSigned(comparison.metricDeltas.distinctLandmarkCount)} distinct`);
  console.log(`gap delta: ${formatSigned(comparison.metricDeltas.maxNotableGapMeters, 1)} m max`);
  console.log(`tokenless stretch delta: ${formatSigned(comparison.metricDeltas.tokenlessRouteStretchCount)} stretches`);
  console.log(`strong silhouette stretch delta: ${formatSigned(comparison.metricDeltas.strongSilhouetteStretchCoverageRatio * 100, 1)} pp, ${formatSigned(comparison.metricDeltas.strongSilhouettelessStretchCount)} empty`);
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
  const distinctRegions = sortedDistinct(samples.map((sample) => sample.regionId));
  const distinctUndergroundBiomes = sortedDistinct(samples.map((sample) => sample.undergroundBiomeId));
  const distinctRegionalVariants = sortedDistinct(samples.map((sample) => sample.regionalVariantId).filter(isString));
  const directLandmarks = sortedDistinct(samples.map((sample) => sample.landmarkId).filter(isString));
  const visibleNearbyLandmarks = sortedDistinct(samples.flatMap((sample) => sample.visibleNearbyLandmarkIds));
  const distinctLandmarks = sortedDistinct([...directLandmarks, ...visibleNearbyLandmarks]);
  const requiredLandmarkIds = [...(route.requiredLandmarkIds ?? [])].sort();
  const missingRequiredLandmarkIds = requiredLandmarkIds.filter((landmarkId) => !distinctLandmarks.includes(landmarkId));
  const distinctAmbientProfiles = sortedDistinct(samples.map((sample) => sample.ambientProfileId));
  const notableGaps = computeNotableGaps(samples);
  const routeStretches = summarizeRouteStretches(samples);
  const tokenlessRouteStretches = routeStretches.filter((stretch) => stretch.tokenCount === 0);
  const strongSilhouetteStretches = summarizeStrongSilhouetteStretches(samples);
  const strongSilhouettelessStretches = strongSilhouetteStretches.filter((stretch) => stretch.tokenCount === 0);
  const surfaces = samples.map((sample) => sample.surfaceMeters);
  const directLandmarkHitCount = samples.filter((sample) => sample.landmarkId !== null).length;
  const visibleNearbyLandmarkHitCount = samples.filter((sample) => sample.visibleNearbyLandmarkIds.length > 0).length;
  const expectedRegionCoverageRatio = route.expectedRegionId
    ? ratio(samples.filter((sample) => sample.regionId === route.expectedRegionId).length, samples.length)
    : null;
  const expectedRegionalVariantCoverageRatio = route.expectedRegionalVariantId
    ? ratio(samples.filter((sample) => sample.regionalVariantId === route.expectedRegionalVariantId).length, samples.length)
    : null;
  return {
    label: route.label,
    sampleCount: samples.length,
    lengthMeters: route.lengthMeters,
    distinctBiomes,
    distinctRegions,
    distinctUndergroundBiomes,
    distinctRegionalVariants,
    distinctLandmarks,
    directLandmarks,
    visibleNearbyLandmarks,
    requiredLandmarkIds,
    missingRequiredLandmarkIds,
    expectedRegionId: route.expectedRegionId ?? null,
    expectedRegionCoverageRatio,
    expectedRegionalVariantId: route.expectedRegionalVariantId ?? null,
    expectedRegionalVariantCoverageRatio,
    distinctAmbientProfiles,
    directLandmarkHitCount,
    visibleNearbyLandmarkHitCount,
    landmarkHitCount: samples.filter((sample) => sample.landmarkId !== null || sample.visibleNearbyLandmarkIds.length > 0).length,
    maxNotableGapMeters: Math.max(0, ...notableGaps),
    averageNotableGapMeters: average(notableGaps),
    routeStretchCount: routeStretches.length,
    tokenizedRouteStretchCount: routeStretches.length - tokenlessRouteStretches.length,
    tokenlessRouteStretchCount: tokenlessRouteStretches.length,
    routeStretchCoverageRatio: ratio(routeStretches.length - tokenlessRouteStretches.length, routeStretches.length),
    maxTokenlessRouteStretchMeters: Math.max(0, ...tokenlessRouteStretches.map((stretch) => stretch.endMeters - stretch.startMeters)),
    tokenlessRouteStretches,
    strongSilhouetteStretchCount: strongSilhouetteStretches.length,
    strongSilhouetteTokenizedStretchCount: strongSilhouetteStretches.length - strongSilhouettelessStretches.length,
    strongSilhouettelessStretchCount: strongSilhouettelessStretches.length,
    strongSilhouetteStretchCoverageRatio: ratio(strongSilhouetteStretches.length - strongSilhouettelessStretches.length, strongSilhouetteStretches.length),
    maxStrongSilhouettelessStretchMeters: Math.max(0, ...strongSilhouettelessStretches.map((stretch) => stretch.endMeters - stretch.startMeters)),
    strongSilhouettelessStretches,
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
      regionId: probe.regionId,
      biomeId: probe.biomeId,
      undergroundBiomeId: probe.undergroundBiomeId,
      regionalVariantId: probe.regionalVariantId,
      landmarkId: probe.landmarkId,
      landmarkHeightMeters: landmarkHeightMeters(probe),
      visibleNearbyLandmarkIds: visibleNearbyLandmarks.map((landmark) => landmark.landmarkId),
      visibleNearbyStrongLandmarkIds: visibleNearbyLandmarks
        .filter((landmark) => landmark.heightMeters >= strongSilhouetteScan.minHeightMeters)
        .map((landmark) => landmark.landmarkId),
      nearestVisibleNearbyLandmarkId: visibleNearbyLandmarks[0]?.landmarkId ?? null,
      nearestVisibleNearbyLandmarkDistanceMeters: visibleNearbyLandmarks[0]?.distanceMeters ?? null,
      tallestVisibleNearbyLandmarkHeightMeters: Math.max(0, ...visibleNearbyLandmarks.map((landmark) => landmark.heightMeters)),
      terrainTokenId: resolveTerrainRouteToken(probe),
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
  const routeStretchCount = sum(routes.map((route) => route.routeStretchCount));
  const tokenizedRouteStretchCount = sum(routes.map((route) => route.tokenizedRouteStretchCount));
  const tokenlessRouteStretchCount = sum(routes.map((route) => route.tokenlessRouteStretchCount));
  const strongSilhouetteStretchCount = sum(routes.map((route) => route.strongSilhouetteStretchCount));
  const strongSilhouetteTokenizedStretchCount = sum(routes.map((route) => route.strongSilhouetteTokenizedStretchCount));
  const strongSilhouettelessStretchCount = sum(routes.map((route) => route.strongSilhouettelessStretchCount));
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
    routeStretchCount,
    tokenizedRouteStretchCount,
    tokenlessRouteStretchCount,
    routeStretchCoverageRatio: ratio(tokenizedRouteStretchCount, routeStretchCount),
    maxTokenlessRouteStretchMeters: Math.max(0, ...routes.map((route) => route.maxTokenlessRouteStretchMeters)),
    strongSilhouetteStretchCount,
    strongSilhouetteTokenizedStretchCount,
    strongSilhouettelessStretchCount,
    strongSilhouetteStretchCoverageRatio: ratio(strongSilhouetteTokenizedStretchCount, strongSilhouetteStretchCount),
    maxStrongSilhouettelessStretchMeters: Math.max(0, ...routes.map((route) => route.maxStrongSilhouettelessStretchMeters)),
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
      tokenlessRouteStretchCount: aggregate.tokenlessRouteStretchCount - readNumber(baselineAggregate.tokenlessRouteStretchCount),
      routeStretchCoverageRatio: aggregate.routeStretchCoverageRatio - readNumber(baselineAggregate.routeStretchCoverageRatio),
      strongSilhouettelessStretchCount: aggregate.strongSilhouettelessStretchCount - readNumber(baselineAggregate.strongSilhouettelessStretchCount),
      strongSilhouetteStretchCoverageRatio: aggregate.strongSilhouetteStretchCoverageRatio - readNumber(baselineAggregate.strongSilhouetteStretchCoverageRatio),
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
        tokenlessRouteStretchCount: route.tokenlessRouteStretchCount - readNumber(baselineRoute?.tokenlessRouteStretchCount),
        strongSilhouettelessStretchCount: route.strongSilhouettelessStretchCount - readNumber(baselineRoute?.strongSilhouettelessStretchCount),
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
    `- Route stretch scan: ${report.routeStretchScan.windowMeters} m windows every ${report.routeStretchScan.strideMeters} m`,
    `- Route stretch coverage: ${formatPercent(report.aggregate.routeStretchCoverageRatio)} (${report.aggregate.tokenizedRouteStretchCount}/${report.aggregate.routeStretchCount} tokenized, ${report.aggregate.tokenlessRouteStretchCount} tokenless)`,
    `- Max tokenless route stretch: ${report.aggregate.maxTokenlessRouteStretchMeters.toFixed(1)} m`,
    `- Strong silhouette scan: ${report.strongSilhouetteScan.windowMeters} m windows every ${report.strongSilhouetteScan.strideMeters} m, min height ${report.strongSilhouetteScan.minHeightMeters.toFixed(1)} m`,
    `- Strong silhouette coverage: ${formatPercent(report.aggregate.strongSilhouetteStretchCoverageRatio)} (${report.aggregate.strongSilhouetteTokenizedStretchCount}/${report.aggregate.strongSilhouetteStretchCount} tokenized, ${report.aggregate.strongSilhouettelessStretchCount} empty)`,
    `- Max strong-silhouetteless stretch: ${report.aggregate.maxStrongSilhouettelessStretchMeters.toFixed(1)} m`,
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
      `| Tokenless route stretches | ${formatSigned(report.comparison.metricDeltas.tokenlessRouteStretchCount)} |`,
      `| Route stretch coverage | ${formatSigned(report.comparison.metricDeltas.routeStretchCoverageRatio * 100, 1)} pp |`,
      `| Strong-silhouetteless stretches | ${formatSigned(report.comparison.metricDeltas.strongSilhouettelessStretchCount)} |`,
      `| Strong silhouette coverage | ${formatSigned(report.comparison.metricDeltas.strongSilhouetteStretchCoverageRatio * 100, 1)} pp |`,
      `| Definition score | ${formatSigned(report.comparison.metricDeltas.definitionScore, 2)} |`,
      "",
      `- Added landmarks: ${formatList(report.comparison.addedLandmarks)}`,
      `- Removed landmarks: ${formatList(report.comparison.removedLandmarks)}`,
      `- Added ambient profiles: ${formatList(report.comparison.addedAmbientProfiles)}`,
      `- Removed ambient profiles: ${formatList(report.comparison.removedAmbientProfiles)}`,
      "",
      "### Route Deltas",
      "",
      "| Route | Landmark Hits | Max Gap | Tokenless Stretches | Strong Empty | Added Landmarks | Removed Landmarks |",
      "| --- | ---: | ---: | ---: | ---: | --- | --- |",
      ...report.comparison.routeDeltas.map((route) =>
        `| ${route.label} | ${formatSigned(route.landmarkHitCount)} | ${formatSigned(route.maxNotableGapMeters, 1)} m | ${formatSigned(route.tokenlessRouteStretchCount)} | ${formatSigned(route.strongSilhouettelessStretchCount)} | ${formatList(route.addedLandmarks)} | ${formatList(route.removedLandmarks)} |`,
      ),
      "",
    );
  }
  lines.push(
    "## Route Details",
    "",
    "| Route | Biomes | Ambient | Landmarks | Vista | Required Missing | Hits | Max Gap | Stretch Coverage | Strong Coverage | Tokenless Windows | Strong Empty Windows |",
    "| --- | ---: | ---: | --- | --- | --- | ---: | ---: | ---: | ---: | --- | --- |",
    ...report.routes.map((route) =>
      `| ${route.label} | ${route.distinctBiomes.length} | ${route.distinctAmbientProfiles.length} | ${formatList(route.distinctLandmarks)} | ${formatList(route.visibleNearbyLandmarks)} | ${formatList(route.missingRequiredLandmarkIds)} | ${route.landmarkHitCount} | ${route.maxNotableGapMeters.toFixed(1)} m | ${formatPercent(route.routeStretchCoverageRatio)} | ${formatPercent(route.strongSilhouetteStretchCoverageRatio)} | ${formatTokenlessRouteStretches(route.tokenlessRouteStretches)} | ${formatTokenlessRouteStretches(route.strongSilhouettelessStretches)} |`,
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
  if (aggregate.tokenlessRouteStretchCount > thresholds.maxTokenlessRouteStretches) {
    failures.push(`${aggregate.tokenlessRouteStretchCount} route stretch window(s) lack a silhouette or route token`);
  }
  if (aggregate.strongSilhouetteStretchCoverageRatio < thresholds.minStrongSilhouetteStretchCoverageRatio) {
    failures.push(`strong silhouette stretch coverage ${formatPercent(aggregate.strongSilhouetteStretchCoverageRatio)} is below ${formatPercent(thresholds.minStrongSilhouetteStretchCoverageRatio)}`);
  }
  if (aggregate.strongSilhouettelessStretchCount > thresholds.maxStrongSilhouettelessRouteStretches) {
    failures.push(`${aggregate.strongSilhouettelessStretchCount} route stretch window(s) lack a strong silhouette`);
  }
  for (const route of routes) {
    if (route.missingRequiredLandmarkIds.length > 0) {
      failures.push(`${route.label} missing required landmark(s): ${route.missingRequiredLandmarkIds.join(", ")}`);
    }
    if (
      route.expectedRegionId
      && route.expectedRegionCoverageRatio !== null
      && route.expectedRegionCoverageRatio < (ROUTES.find((spec) => spec.label === route.label)?.minExpectedRegionCoverage ?? 0.5)
    ) {
      failures.push(`${route.label} expected region ${route.expectedRegionId} coverage ${formatPercent(route.expectedRegionCoverageRatio)} is too low`);
    }
    if (
      route.expectedRegionalVariantId
      && route.expectedRegionalVariantCoverageRatio !== null
      && route.expectedRegionalVariantCoverageRatio < (ROUTES.find((spec) => spec.label === route.label)?.minExpectedRegionalVariantCoverage ?? 0.25)
    ) {
      failures.push(`${route.label} expected variant ${route.expectedRegionalVariantId} coverage ${formatPercent(route.expectedRegionalVariantCoverageRatio)} is too low`);
    }
    if (route.tokenlessRouteStretchCount > 0) {
      failures.push(`${route.label} has ${route.tokenlessRouteStretchCount} tokenless route stretch window(s)`);
    }
  }
  return failures;
}

function summarizeRouteStretches(samples: readonly RouteSample[]): RouteStretchSummary[] {
  if (samples.length === 0) {
    return [];
  }
  const finalDistance = samples[samples.length - 1]!.distanceMeters;
  const windowMeters = Math.min(routeStretchScan.windowMeters, finalDistance);
  const starts = routeStretchStarts(finalDistance, windowMeters);
  const tokenEvents = computeRouteTokenEvents(samples);
  return starts.map((startMeters) => {
    const endMeters = Math.min(finalDistance, startMeters + windowMeters);
    const stretchEvents = tokenEvents.filter((event) => event.distanceMeters >= startMeters && event.distanceMeters <= endMeters);
    const silhouetteTokenIds = sortedDistinct(stretchEvents
      .filter((event) => event.tokenKind === "silhouette")
      .map((event) => event.tokenId));
    const routeTokenIds = sortedDistinct(stretchEvents
      .filter((event) => event.tokenKind === "route")
      .map((event) => event.tokenId));
    return {
      startMeters,
      endMeters,
      tokenCount: silhouetteTokenIds.length + routeTokenIds.length,
      silhouetteTokenIds,
      routeTokenIds,
    };
  });
}

function summarizeStrongSilhouetteStretches(samples: readonly RouteSample[]): RouteStretchSummary[] {
  if (samples.length === 0) {
    return [];
  }
  const finalDistance = samples[samples.length - 1]!.distanceMeters;
  const windowMeters = Math.min(strongSilhouetteScan.windowMeters, finalDistance);
  const starts = strongSilhouetteStretchStarts(finalDistance, windowMeters);
  const tokenEvents = computeStrongSilhouetteTokenEvents(samples);
  return starts.map((startMeters) => {
    const endMeters = Math.min(finalDistance, startMeters + windowMeters);
    const stretchEvents = tokenEvents.filter((event) => event.distanceMeters >= startMeters && event.distanceMeters <= endMeters);
    const silhouetteTokenIds = sortedDistinct(stretchEvents.map((event) => event.tokenId));
    return {
      startMeters,
      endMeters,
      tokenCount: silhouetteTokenIds.length,
      silhouetteTokenIds,
      routeTokenIds: [],
    };
  });
}

function routeStretchStarts(finalDistance: number, windowMeters: number): number[] {
  if (finalDistance <= 0) {
    return [0];
  }
  if (windowMeters >= finalDistance) {
    return [0];
  }
  const starts: number[] = [];
  for (let startMeters = 0; startMeters + windowMeters <= finalDistance; startMeters += routeStretchScan.strideMeters) {
    starts.push(roundMeters(startMeters));
  }
  const finalStart = roundMeters(finalDistance - windowMeters);
  if (starts[starts.length - 1] !== finalStart) {
    starts.push(finalStart);
  }
  return starts;
}

function strongSilhouetteStretchStarts(finalDistance: number, windowMeters: number): number[] {
  if (finalDistance <= 0) {
    return [0];
  }
  if (windowMeters >= finalDistance) {
    return [0];
  }
  const starts: number[] = [];
  for (let startMeters = 0; startMeters + windowMeters <= finalDistance; startMeters += strongSilhouetteScan.strideMeters) {
    starts.push(roundMeters(startMeters));
  }
  const finalStart = roundMeters(finalDistance - windowMeters);
  if (starts[starts.length - 1] !== finalStart) {
    starts.push(finalStart);
  }
  return starts;
}

function computeRouteTokenEvents(samples: readonly RouteSample[]): RouteTokenEvent[] {
  const events: RouteTokenEvent[] = [];
  for (const [index, sample] of samples.entries()) {
    if (sample.landmarkId && sample.landmarkHeightMeters >= routeStretchScan.minSilhouetteHeightMeters) {
      events.push({
        distanceMeters: sample.distanceMeters,
        tokenId: `direct:${sample.landmarkId}`,
        tokenKind: "silhouette",
      });
    }
    for (const landmarkId of sample.visibleNearbyLandmarkIds) {
      events.push({
        distanceMeters: sample.distanceMeters,
        tokenId: `vista:${landmarkId}`,
        tokenKind: "silhouette",
      });
    }
    if (sample.regionalVariantId) {
      events.push({
        distanceMeters: sample.distanceMeters,
        tokenId: `region:${sample.regionalVariantId}`,
        tokenKind: "route",
      });
    }
    if (sample.terrainTokenId) {
      events.push({
        distanceMeters: sample.distanceMeters,
        tokenId: `terrain:${sample.terrainTokenId}`,
        tokenKind: "route",
      });
    }
    const previous = samples[index - 1];
    if (!previous) {
      continue;
    }
    if (sample.biomeId !== previous.biomeId) {
      events.push({
        distanceMeters: sample.distanceMeters,
        tokenId: `biome:${previous.biomeId}>${sample.biomeId}`,
        tokenKind: "route",
      });
    }
    if (sample.regionalVariantId !== previous.regionalVariantId) {
      events.push({
        distanceMeters: sample.distanceMeters,
        tokenId: `region-transition:${previous.regionalVariantId ?? "none"}>${sample.regionalVariantId ?? "none"}`,
        tokenKind: "route",
      });
    }
  }
  return events;
}

function computeStrongSilhouetteTokenEvents(samples: readonly RouteSample[]): RouteTokenEvent[] {
  const events: RouteTokenEvent[] = [];
  for (const sample of samples) {
    if (sample.landmarkId && sample.landmarkHeightMeters >= strongSilhouetteScan.minHeightMeters) {
      events.push({
        distanceMeters: sample.distanceMeters,
        tokenId: `direct:${sample.landmarkId}`,
        tokenKind: "silhouette",
      });
    }
    for (const landmarkId of sample.visibleNearbyStrongLandmarkIds) {
      events.push({
        distanceMeters: sample.distanceMeters,
        tokenId: `vista:${landmarkId}`,
        tokenKind: "silhouette",
      });
    }
  }
  return events;
}

function resolveTerrainRouteToken(probe: ReturnType<ProceduralWorldGenerator["sampleBiomeProbe"]>): string | null {
  const fields = probe.fields;
  if (
    probe.biomeId === "steppe"
    && (
      (
        fields.ridge > 0.68
        && fields.desolation > 0.48
        && fields.moisture < 0.52
      )
      || (
        fields.ridge > 0.72
        && fields.uplift > 0.56
        && fields.moisture < 0.44
      )
    )
  ) {
    return "wind-cut-steppe";
  }
  if (
    probe.biomeId === "saltflat"
    && (fields.surfacePatch > 0.54 || fields.strata > 0.62)
  ) {
    return "salt-crust";
  }
  if (
    probe.regionalVariantId === "ash_wastes"
    && (fields.strata > 0.54 || fields.surfaceGrain > 0.58 || fields.scatter > 0.56)
  ) {
    return "ash-crust";
  }
  return null;
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

function formatPercent(value: number): string {
  return `${(value * 100).toFixed(1)}%`;
}

function formatList(values: readonly string[]): string {
  return values.length > 0 ? values.join(", ") : "none";
}

function formatTokenlessRouteStretches(stretches: readonly RouteStretchSummary[]): string {
  if (stretches.length === 0) {
    return "none";
  }
  const preview = stretches.slice(0, 4)
    .map((stretch) => `${stretch.startMeters.toFixed(0)}-${stretch.endMeters.toFixed(0)} m`);
  const remaining = stretches.length - preview.length;
  return remaining > 0 ? `${preview.join(", ")} (+${remaining})` : preview.join(", ");
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

function ratio(numerator: number, denominator: number): number {
  return denominator === 0 ? 1 : numerator / denominator;
}

function roundMeters(value: number): number {
  return Math.round(value * 10) / 10;
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
