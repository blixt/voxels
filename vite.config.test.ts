import { mkdirSync, mkdtempSync, rmSync, unlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { createServer } from "node:net";
import path from "node:path";
import { describe, expect, it } from "vite-plus/test";
import {
  browserWasmProfile,
  assertWorldServicePortAvailable,
  isNativeWorldServiceInput,
  pathBelongsTo,
  WatchedInputContentChanges,
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
    const observed: string[] = [];

    watchRustInputChanges(
      {
        on: (event, registered) => registrations.set(event, registered),
      },
      (file, event) => observed.push(`${event}:${file}`),
    );

    expect([...registrations.keys()]).toEqual(["add", "change", "unlink"]);
    registrations.get("add")?.("added.rs");
    registrations.get("change")?.("changed.rs");
    registrations.get("unlink")?.("removed.rs");
    expect(observed).toEqual(["add:added.rs", "change:changed.rs", "unlink:removed.rs"]);
  });

  it("reacts only to actual file content and existence transitions", () => {
    const directory = mkdtempSync(path.join(tmpdir(), "voxels-watcher-"));
    const file = path.join(directory, "client.toml");
    const changes = new WatchedInputContentChanges();
    try {
      writeFileSync(file, "alpha");
      changes.prime([file]);
      expect(changes.observe(file, "change", true)).toBe(false);
      expect(changes.observe(file, "add", true)).toBe(false);

      writeFileSync(file, "beta");
      expect(changes.observe(file, "change", true)).toBe(true);
      expect(changes.observe(file, "change", true)).toBe(false);

      unlinkSync(file);
      expect(changes.observe(file, "unlink", true)).toBe(true);
      expect(changes.observe(file, "unlink", true)).toBe(false);

      writeFileSync(file, "beta");
      expect(changes.observe(file, "add", true)).toBe(true);
      expect(changes.observe(file, "unlink", true)).toBe(false);
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  });

  it("detects a new file below a primed directory", () => {
    const directory = mkdtempSync(path.join(tmpdir(), "voxels-watcher-directory-"));
    const existing = path.join(directory, "existing.rs");
    const nested = path.join(directory, "nested");
    const added = path.join(nested, "added.rs");
    const changes = new WatchedInputContentChanges();
    try {
      writeFileSync(existing, "existing");
      changes.prime([directory]);
      expect(changes.observe(existing, "add", true)).toBe(false);
      mkdirSync(nested);
      writeFileSync(added, "new");
      expect(changes.observe(added, "add", true)).toBe(true);
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  });
});

describe("native world-service development command", () => {
  it("rejects a stale listener instead of accepting the wrong daemon", async () => {
    const directory = mkdtempSync(path.join(tmpdir(), "voxels-world-port-"));
    const config = path.join(directory, "world-service.toml");
    const server = createServer();
    try {
      await new Promise<void>((resolve, reject) => {
        server.once("error", reject);
        server.listen(0, "127.0.0.1", resolve);
      });
      const address = server.address();
      if (address === null || typeof address === "string") {
        throw new Error("test server did not expose a TCP address");
      }
      writeFileSync(config, `[transport]\nlisten = "127.0.0.1:${address.port}"\n`);
      await expect(assertWorldServicePortAvailable(config)).rejects.toThrow("already in use");
    } finally {
      await new Promise<void>((resolve) => server.close(() => resolve()));
      rmSync(directory, { recursive: true, force: true });
    }
  });

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
    expect(worldServiceListenAddress('listen = "127.0.0.1:4123" # local development')).toEqual({
      host: "127.0.0.1",
      port: 4123,
    });
    expect(() => worldServiceListenAddress('listen = "127.0.0.1:0"')).toThrow(
      "invalid world-service transport.listen port",
    );
  });

  it("matches watched inputs without confusing sibling path prefixes", () => {
    expect(pathBelongsTo("/repo/world/src/lib.rs", "/repo/world/src")).toBe(true);
    expect(pathBelongsTo("/repo/world/src-old/lib.rs", "/repo/world/src")).toBe(false);
    expect(isNativeWorldServiceInput("core/src/lib.rs")).toBe(true);
    expect(isNativeWorldServiceInput("core/Cargo.toml")).toBe(true);
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

  it("selects automation profiles without mutating shared process state", () => {
    expect(browserWasmProfile("build", undefined, "automation-debug")).toBe("debug");
    expect(browserWasmProfile("build", undefined, "automation-wasm-dev")).toBe("wasm-dev");
    expect(browserWasmProfile("build", undefined, "automation-release")).toBe("release");
  });
});
