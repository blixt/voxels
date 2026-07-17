import { describe, expect, it } from "vite-plus/test";
import {
  browserWasmProfile,
  isNativeWorldServiceInput,
  pathBelongsTo,
  watchRustInputChanges,
  worldServiceDevelopmentProfile,
  worldServiceListenAddress,
} from "./vite.config.ts";
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

  it("builds the incremental Metal-enabled daemon before Vite launches it directly", () => {
    expect(worldServiceBuildCargoArgs({ metal: true, profile: "worldgen-dev" })).toEqual([
      "build",
      "--profile",
      "worldgen-dev",
      "-p",
      "voxels-world-service",
      "--features",
      "terrain-metal",
      "--bin",
      "voxels-worldd",
    ]);
  });

  it("defaults Vite to the fast profile while allowing explicit optimized profiling", () => {
    expect(worldServiceDevelopmentProfile(undefined)).toBe("worldgen-dev");
    expect(worldServiceDevelopmentProfile("worldgen-dev")).toBe("worldgen-dev");
    expect(worldServiceDevelopmentProfile("worldgen")).toBe("worldgen");
    expect(() => worldServiceDevelopmentProfile("release")).toThrow(
      "expected worldgen-dev or worldgen",
    );
  });

  it("parses the supervised loopback listener from server config", () => {
    expect(
      worldServiceListenAddress(`
[transport]
listen = "127.0.0.1:9777"
`),
    ).toEqual({ host: "127.0.0.1", port: 9777 });
    expect(worldServiceListenAddress('listen = "[::1]:4123"')).toEqual({
      host: "::1",
      port: 4123,
    });
    expect(() => worldServiceListenAddress('listen = "127.0.0.1:0"')).toThrow(
      "invalid world-service transport.listen port",
    );
  });

  it("matches watched inputs without confusing sibling path prefixes", () => {
    expect(pathBelongsTo("/repo/world/src/lib.rs", "/repo/world/src")).toBe(true);
    expect(pathBelongsTo("/repo/world/src-old/lib.rs", "/repo/world/src")).toBe(false);
    expect(isNativeWorldServiceInput("world/src/source.rs")).toBe(true);
    expect(isNativeWorldServiceInput("world-terrain-diffusion/fixtures/pipeline-data.json")).toBe(
      true,
    );
    expect(isNativeWorldServiceInput("shell/src/lib.rs")).toBe(false);
  });
});

describe("browser WASM build profile", () => {
  it("uses optimized incremental WASM for play and release WASM for production builds", () => {
    expect(browserWasmProfile("serve", undefined)).toBe("wasm-dev");
    expect(browserWasmProfile("build", undefined)).toBe("release");
  });

  it("keeps explicit profiles available for controlled performance comparisons", () => {
    expect(browserWasmProfile("serve", "debug")).toBe("debug");
    expect(browserWasmProfile("build", "wasm-dev")).toBe("wasm-dev");
    expect(browserWasmProfile("serve", "release")).toBe("release");
    expect(() => browserWasmProfile("serve", "fast")).toThrow(
      "expected debug, wasm-dev, or release",
    );
  });
});
