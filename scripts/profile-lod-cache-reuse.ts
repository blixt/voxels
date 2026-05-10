import { ProceduralResidentWorld } from "../src/engine/procedural-resident-world.ts";
import { ProceduralWorldGenerator } from "../src/engine/procedural-generator.ts";
import type { LodResidencyUpdateSummary } from "../src/engine/procedural-resident-world.ts";
import type { Vec3 } from "../src/engine/types.ts";

interface CliOptions {
  label: string;
  seed: number;
  radiusChunks: number;
  farOffset: number;
  maxGenerateLodChunks: number;
  maxPasses: number;
}

interface CompletedLodRun {
  position: { x: number; y: number; z: number };
  complete: boolean;
  passes: number;
  generated: number;
  cacheHits: number;
  emptyCacheHits: number;
  elapsedMs: number;
  downsampleMs: number;
  meshMs: number;
  final: LodResidencyUpdateSummary;
}

const options = parseCli(Bun.argv);
const profile = runProfile(options);
const artifactDir = "artifacts/lod-cache-reuse-profile";
const artifactPath = `${artifactDir}/${timestampSlug()}-${options.label}.json`;
await Bun.$`mkdir -p ${artifactDir}`.quiet();
await Bun.write(artifactPath, `${JSON.stringify(profile, null, 2)}\n`);

console.log(JSON.stringify(profile, null, 2));
console.error(`wrote ${artifactPath}`);

function runProfile(options: CliOptions) {
  const generator = new ProceduralWorldGenerator(options.seed, { chunkSize: 16 });
  const world = new ProceduralResidentWorld(generator, {
    horizontalRadiusChunks: options.radiusChunks,
  });
  const spawn = world.getSpawnPosition();
  const far: Vec3 = [spawn[0] + options.farOffset, spawn[1], spawn[2] + options.farOffset];

  const first = completeLod(world, spawn, options);
  const farRun = completeLod(world, far, options);
  const backNoGenerate = world.updateLodResidencyAround(spawn, { maxGenerateLodChunks: 0 });
  const backComplete = completeLod(world, spawn, options);

  return {
    tool: "profile-lod-cache-reuse",
    label: options.label,
    gitRevision: gitRevision(),
    options: {
      ...options,
      maxGenerateLodChunks: Number.isFinite(options.maxGenerateLodChunks)
        ? options.maxGenerateLodChunks
        : "infinity",
    },
    first,
    far: farRun,
    backNoGenerate,
    backComplete,
    reusedOnReturn: backNoGenerate.cacheHits + backNoGenerate.emptyCacheHits,
    rebuiltOnReturnBeforeReuse: backNoGenerate.generated,
  };
}

function completeLod(
  world: ProceduralResidentWorld,
  position: Vec3,
  options: CliOptions,
): CompletedLodRun {
  const summaries: LodResidencyUpdateSummary[] = [];
  let summary = world.updateLodResidencyAround(position, {
    maxGenerateLodChunks: options.maxGenerateLodChunks,
  });
  summaries.push(summary);
  for (let pass = 1; pass < options.maxPasses && summary.pending > 0; pass += 1) {
    summary = world.updateLodResidencyAround(position, {
      maxGenerateLodChunks: options.maxGenerateLodChunks,
    });
    summaries.push(summary);
  }
  return {
    position: vec3Record(position),
    complete: summary.pending === 0,
    passes: summaries.length,
    generated: sum(summaries.map((entry) => entry.generated)),
    cacheHits: sum(summaries.map((entry) => entry.cacheHits)),
    emptyCacheHits: sum(summaries.map((entry) => entry.emptyCacheHits)),
    elapsedMs: sum(summaries.map((entry) => entry.elapsedMs)),
    downsampleMs: sum(summaries.map((entry) => entry.downsampleMs)),
    meshMs: sum(summaries.map((entry) => entry.meshMs)),
    final: summary,
  };
}

function parseCli(argv: readonly string[]): CliOptions {
  const args = argv.slice(2);
  return {
    label: sanitizeLabel(readFlag(args, "--label") ?? "lod-cache-reuse"),
    seed: readPositiveInt(readFlag(args, "--seed"), 1337),
    radiusChunks: readPositiveInt(readFlag(args, "--radius"), 1),
    farOffset: readPositiveInt(readFlag(args, "--far-offset"), 8192),
    maxGenerateLodChunks: readPositiveInt(readFlag(args, "--max-lod-chunks"), Number.POSITIVE_INFINITY),
    maxPasses: readPositiveInt(readFlag(args, "--max-passes"), 48),
  };
}

function readFlag(args: readonly string[], name: string): string | null {
  const prefix = `${name}=`;
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index]!;
    if (arg === name) {
      return args[index + 1] ?? null;
    }
    if (arg.startsWith(prefix)) {
      return arg.slice(prefix.length);
    }
  }
  return null;
}

function readPositiveInt(raw: string | null, fallback: number): number {
  if (raw === null || raw === "all" || raw === "infinity") {
    return fallback;
  }
  const parsed = Number.parseInt(raw, 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

function sum(values: number[]): number {
  return values.reduce((total, value) => total + value, 0);
}

function vec3Record(vec: Vec3): { x: number; y: number; z: number } {
  return { x: vec[0], y: vec[1], z: vec[2] };
}

function sanitizeLabel(label: string): string {
  return label.trim().replace(/[^a-zA-Z0-9._-]+/g, "-").replace(/^-+|-+$/g, "") || "lod-cache-reuse";
}

function timestampSlug(): string {
  return new Date().toISOString().replace(/[-:]/g, "").replace(/\.\d{3}Z$/, "Z");
}

function gitRevision(): string {
  const result = Bun.spawnSync(["git", "rev-parse", "--short", "HEAD"], {
    stderr: "pipe",
    stdout: "pipe",
  });
  if (!result.success) {
    return "unknown";
  }
  return result.stdout.toString().trim() || "unknown";
}
