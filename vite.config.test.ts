import { describe, expect, it } from "vite-plus/test";
import { watchRustInputChanges } from "./vite.config.ts";
import {
  worldServiceBuildCargoArgs,
  worldServiceCargoArgs,
} from "./scripts/world-service-command.ts";

describe("Rust WASM development watcher", () => {
  it("rebuilds for changed, added, and removed Rust inputs", () => {
    const registrations = new Map<string, (file: string) => void>();
    const listener = (): void => undefined;

    watchRustInputChanges(
      {
        on: (event, registered) => registrations.set(event, registered),
      },
      listener,
    );

    expect([...registrations.keys()]).toEqual(["add", "change", "unlink"]);
    expect([...registrations.values()]).toEqual([listener, listener, listener]);
  });
});

describe("native world-service development command", () => {
  it("uses the optimized Metal-enabled daemon and checked-in config when run alone", () => {
    expect(worldServiceCargoArgs({ metal: true })).toEqual([
      "run",
      "--profile",
      "worldgen",
      "-p",
      "voxels-world-service",
      "--features",
      "terrain-metal",
      "--bin",
      "voxels-worldd",
      "--",
      "config/world-service.toml",
    ]);
  });

  it("builds the optimized Metal-enabled daemon before Vite launches it directly", () => {
    expect(worldServiceBuildCargoArgs({ metal: true })).toEqual([
      "build",
      "--profile",
      "worldgen",
      "-p",
      "voxels-world-service",
      "--features",
      "terrain-metal",
      "--bin",
      "voxels-worldd",
    ]);
  });
});
