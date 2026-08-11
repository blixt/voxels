import path from "node:path";

export interface WorldServiceCommandOptions {
  sourceSmoke?: boolean;
  metal?: boolean;
  configPath?: string;
  profile?: WorldServiceCargoProfile;
}

export type WorldServiceCargoProfile = "worldgen" | "worldgen-dev";

/** One canonical Cargo invocation shared by standalone tasks and the Vite development lifecycle. */
export function worldServiceCargoArgs({
  sourceSmoke = false,
  metal = false,
  configPath = "config/world-service.toml",
  profile = "worldgen",
}: WorldServiceCommandOptions = {}): string[] {
  return [
    "run",
    "--profile",
    profile,
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
  profile = "worldgen",
}: Pick<WorldServiceCommandOptions, "metal" | "profile"> = {}): string[] {
  return [
    "build",
    "--profile",
    profile,
    "-p",
    "voxels-world-service",
    ...(metal ? ["--features", "terrain-metal"] : []),
    "--bin",
    "voxels-worldd",
  ];
}

/** Resolves a native Cargo binary built under one of the repository's service profiles. */
export function cargoProfileExecutablePath(
  binary: string,
  profile: WorldServiceCargoProfile = "worldgen",
): string {
  return path.resolve(
    process.env.CARGO_TARGET_DIR ?? "target",
    profile,
    process.platform === "win32" ? `${binary}.exe` : binary,
  );
}

/** Direct daemon path after [`worldServiceBuildCargoArgs`] completes. */
export function worldServiceExecutablePath(profile: WorldServiceCargoProfile = "worldgen"): string {
  return cargoProfileExecutablePath("voxels-worldd", profile);
}
