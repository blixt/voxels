import { rebuildDirtyMeshes } from "../src/engine/mesher.ts";
import { ProceduralWorldGenerator } from "../src/engine/procedural-generator.ts";
import { ProceduralResidentWorld } from "../src/engine/procedural-resident-world.ts";
import type { Vec3 } from "../src/engine/types.ts";

interface CliOptions {
  label: string;
  seed: number;
  radiusChunks: number;
  maxGenerateLodChunks: number;
  maxPasses: number;
}

interface LevelSummary {
  chunks: number;
  renderableChunks: number;
  solidVoxels: number;
  vertices: number;
  indices: number;
  waterIndices: number;
  triangles: number;
}

const options = parseCli(Bun.argv);
const profile = runProfile(options);
const artifactDir = `artifacts/lod-residency-profile`;
const artifactPath = `${artifactDir}/${timestampSlug()}-${options.label}.json`;
await Bun.$`mkdir -p ${artifactDir}`.quiet();
await Bun.write(artifactPath, `${JSON.stringify(profile, null, 2)}\n`);

console.log(JSON.stringify(profile, null, 2));
console.error(`wrote ${artifactPath}`);

function runProfile(options: CliOptions) {
  const generator = new ProceduralWorldGenerator(options.seed);
  const world = new ProceduralResidentWorld(generator, {
    horizontalRadiusChunks: options.radiusChunks,
  });
  const spawn = world.getSpawnPosition();

  const residencyStartedAt = performance.now();
  const residency = world.updateResidencyAround(spawn, {
    maxGenerateChunks: Number.POSITIVE_INFINITY,
  });
  const residencyMs = performance.now() - residencyStartedAt;

  const meshStartedAt = performance.now();
  const mesh = rebuildDirtyMeshes(world, Number.POSITIVE_INFINITY, {
    priorityPosition: spawn,
  });
  const meshMs = performance.now() - meshStartedAt;

  const passes = [];
  let pending = Number.POSITIVE_INFINITY;
  let pass = 0;
  while (pending > 0 && pass < options.maxPasses) {
    const summary = world.updateLodResidencyAround(spawn, {
      maxGenerateLodChunks: options.maxGenerateLodChunks,
    });
    passes.push(summary);
    pending = summary.pending;
    pass += 1;
  }

  const lodComplete = pending === 0;
  const finalPass = passes.at(-1) ?? world.updateLodResidencyAround(spawn, {
    maxGenerateLodChunks: 0,
  });
  const settledPasses = lodComplete
    ? Array.from({ length: 4 }, () => world.updateLodResidencyAround(spawn, {
        maxGenerateLodChunks: options.maxGenerateLodChunks,
      }))
    : [];

  return {
    tool: "profile-lod-residency",
    label: options.label,
    gitRevision: gitRevision(),
    options,
    spawn: vec3Record(spawn),
    residency: {
      elapsedMs: residencyMs,
      generatedChunks: residency.generatedChunks,
      pendingChunks: residency.pendingChunks,
      emptyChunksSkipped: residency.emptyChunksSkipped,
      residentChunks: residency.residentChunks,
      dirtyResidentChunks: residency.dirtyResidentChunks,
      phaseMs: residency.phaseMs,
    },
    mesh: {
      elapsedMs: meshMs,
      meshCount: mesh.meshCount,
      triangleCount: mesh.triangleCount,
    },
    lod: {
      complete: lodComplete,
      passes: passes.length,
      maxPending: Math.max(0, ...passes.map((entry) => entry.pending)),
      generated: sum(passes.map((entry) => entry.generated)),
      elapsedMs: sum(passes.map((entry) => entry.elapsedMs)),
      yRangeMs: sum(passes.map((entry) => entry.yRangeMs)),
      downsampleMs: sum(passes.map((entry) => entry.downsampleMs)),
      meshMs: sum(passes.map((entry) => entry.meshMs)),
      final: finalPass,
      settledPasses,
      settledElapsedMs: sum(settledPasses.map((entry) => entry.elapsedMs)),
      settledDownsampleMs: sum(settledPasses.map((entry) => entry.downsampleMs)),
    },
    levels: summarizeLevels(world),
  };
}

function summarizeLevels(world: ProceduralResidentWorld): Record<string, LevelSummary> {
  const levels: Record<string, LevelSummary> = {};
  for (const chunk of world.iterateResidentChunks()) {
    const key = String(chunk.lodLevel);
    const entry = levels[key] ??= {
      chunks: 0,
      renderableChunks: 0,
      solidVoxels: 0,
      vertices: 0,
      indices: 0,
      waterIndices: 0,
      triangles: 0,
    };
    entry.chunks += 1;
    entry.solidVoxels += chunk.solidCount;
    if (chunk.renderReady && chunk.mesh && (chunk.mesh.indexCount > 0 || chunk.mesh.waterIndexCount > 0)) {
      entry.renderableChunks += 1;
      entry.vertices += chunk.mesh.vertexCount + chunk.mesh.waterVertexCount;
      entry.indices += chunk.mesh.indexCount;
      entry.waterIndices += chunk.mesh.waterIndexCount;
      entry.triangles += chunk.mesh.triangleCount;
    }
  }
  return levels;
}

function parseCli(argv: readonly string[]): CliOptions {
  const args = argv.slice(2);
  return {
    label: sanitizeLabel(readFlag(args, "--label") ?? "lod-residency"),
    seed: readPositiveInt(readFlag(args, "--seed"), 1337),
    radiusChunks: readPositiveInt(readFlag(args, "--radius"), 8),
    maxGenerateLodChunks: readPositiveInt(readFlag(args, "--max-lod-chunks"), Number.POSITIVE_INFINITY),
    maxPasses: readPositiveInt(readFlag(args, "--max-passes"), 128),
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
  return label.trim().replace(/[^a-zA-Z0-9._-]+/g, "-").replace(/^-+|-+$/g, "") || "lod-residency";
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
