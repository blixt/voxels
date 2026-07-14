export interface WorldServiceCommandOptions {
  sourceSmoke?: boolean;
  metal?: boolean;
  configPath?: string;
}

/** One canonical Cargo invocation shared by standalone tasks and the Vite development lifecycle. */
export function worldServiceCargoArgs({
  sourceSmoke = false,
  metal = false,
  configPath = "config/world-service.toml",
}: WorldServiceCommandOptions = {}): string[] {
  return [
    "run",
    "--profile",
    "worldgen",
    "-p",
    "voxels-world-service",
    ...(metal ? ["--features", "terrain-metal"] : []),
    "--bin",
    sourceSmoke ? "voxels-world-source" : "voxels-worldd",
    "--",
    configPath,
  ];
}

/** Build-only form used before Vite launches the daemon binary it can own directly. */
export function worldServiceBuildCargoArgs({
  metal = false,
}: Pick<WorldServiceCommandOptions, "metal"> = {}): string[] {
  return [
    "build",
    "--profile",
    "worldgen",
    "-p",
    "voxels-world-service",
    ...(metal ? ["--features", "terrain-metal"] : []),
    "--bin",
    "voxels-worldd",
  ];
}
