import { spawn } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, unlinkSync, writeFileSync } from "node:fs";
import { createServer as createHttpServer } from "node:http";
import { tmpdir } from "node:os";
import { createServer as createTcpServer } from "node:net";
import path from "node:path";
import { describe, expect, it } from "vite-plus/test";
import {
  browserWasmProfile,
  assertWorldServicePortAvailable,
  isNativeWorldServiceInput,
  NativeRebuildQueue,
  observeBackgroundFailure,
  pathBelongsTo,
  signalOwnedProcessGroup,
  terminateProcessTree,
  WatchedInputContentChanges,
  watchRustInputChanges,
  worldServiceHealthNonce,
  worldServiceDevelopmentProfile,
  worldServiceListenAddress,
} from "./vite.config.ts";
import viteConfiguration from "./vite.config.ts";
import {
  cargoProfileExecutablePath,
  worldServiceBuildCargoArgs,
  worldServiceCargoArgs,
} from "./scripts/world-service-command.ts";

function waitForOutput(
  child: ReturnType<typeof spawn>,
  output: () => string,
  pattern: RegExp,
  timeoutMs = 1_000,
): Promise<void> {
  return new Promise((resolve, reject) => {
    let settled = false;
    let pollTimer: NodeJS.Timeout | undefined;
    const finish = (error?: Error): void => {
      if (settled) return;
      settled = true;
      clearTimeout(deadline);
      if (pollTimer !== undefined) clearTimeout(pollTimer);
      child.off("error", onError);
      child.off("exit", onExit);
      if (error === undefined) resolve();
      else reject(error);
    };
    const onError = (error: Error): void => finish(error);
    const onExit = (): void => {
      if (!pattern.test(output())) {
        finish(new Error(`process exited before reporting ${pattern}: ${output()}`));
      }
    };
    const poll = (): void => {
      if (pattern.test(output())) finish();
      else pollTimer = setTimeout(poll, 5);
    };
    const deadline = setTimeout(
      () => finish(new Error(`process did not report ${pattern}: ${output()}`)),
      timeoutMs,
    );
    child.once("error", onError);
    child.once("exit", onExit);
    poll();
  });
}

describe("development server configuration", () => {
  it("pins the authorized browser origin instead of silently choosing another port", () => {
    const resolved =
      typeof viteConfiguration === "function"
        ? viteConfiguration({
            command: "serve",
            mode: "development",
            isSsrBuild: false,
            isPreview: false,
          })
        : viteConfiguration;

    expect(resolved.server).toMatchObject({ port: 5173, strictPort: true });
  });
});

describe("Rust WASM development watcher", () => {
  it("reports detached operation failures instead of leaving them unhandled", async () => {
    let reported: unknown;
    observeBackgroundFailure(Promise.reject(new Error("rebuild failed")), (error) => {
      reported = error;
    });
    await Promise.resolve();
    expect(reported).toEqual(new Error("rebuild failed"));
  });

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
  it("serializes an initial build with source-triggered rebuilds", async () => {
    const rebuilds = new NativeRebuildQueue();
    let releaseInitial = (): void => undefined;
    const initialBlocked = new Promise<void>((resolve) => {
      releaseInitial = resolve;
    });
    let active = 0;
    let maximumActive = 0;
    const reloads: boolean[] = [];
    const rebuild = async (reload: boolean): Promise<boolean> => {
      active += 1;
      maximumActive = Math.max(maximumActive, active);
      reloads.push(reload);
      if (reloads.length === 1) await initialBlocked;
      active -= 1;
      return true;
    };

    rebuilds.request(false);
    const initial = rebuilds.drain(rebuild, () => false);
    await Promise.resolve();
    rebuilds.request(true);
    await expect(rebuilds.drain(rebuild, () => false)).resolves.toBe(false);
    expect(reloads).toEqual([false]);

    releaseInitial();
    await expect(initial).resolves.toBe(true);
    expect(reloads).toEqual([false, true]);
    expect(maximumActive).toBe(1);
  });

  it("reports denied process-group signals without skipping the direct fallback", () => {
    const denied = Object.assign(new Error("denied"), { code: "EPERM" });
    expect(
      signalOwnedProcessGroup(123, "SIGKILL", () => {
        throw denied;
      }),
    ).toBe("forbidden");
  });

  it("accepts readiness only from the daemon instance nonce it launched", async () => {
    const server = createHttpServer((_request, response) => {
      response.statusCode = 200;
      response.end("owned-daemon");
    });
    try {
      await new Promise<void>((resolve, reject) => {
        server.once("error", reject);
        server.listen(0, "127.0.0.1", resolve);
      });
      const address = server.address();
      if (address === null || typeof address === "string") {
        throw new Error("test server did not expose a TCP address");
      }
      await expect(worldServiceHealthNonce("127.0.0.1", address.port)).resolves.toBe(
        "owned-daemon",
      );
      await expect(worldServiceHealthNonce("127.0.0.1", address.port)).resolves.not.toBe(
        "other-daemon",
      );
    } finally {
      await new Promise<void>((resolve) => server.close(() => resolve()));
    }
  });

  it("bounds health responses by bytes and total elapsed time", async () => {
    let connections = 0;
    const server = createHttpServer((_request, response) => {
      response.statusCode = 200;
      response.flushHeaders();
      connections += 1;
      if (connections === 1) response.write("x".repeat(257));
      else response.write("x");
      const drip = setInterval(() => response.write("x"), 25);
      response.once("close", () => clearInterval(drip));
    });
    try {
      await new Promise<void>((resolve, reject) => {
        server.once("error", reject);
        server.listen(0, "127.0.0.1", resolve);
      });
      const address = server.address();
      if (address === null || typeof address === "string") {
        throw new Error("test server did not expose a TCP address");
      }
      const failAfter = (milliseconds: number): Promise<never> =>
        new Promise((_resolve, reject) => {
          setTimeout(
            () => reject(new Error("health probe exceeded its deadline")),
            milliseconds,
          ).unref();
        });

      await expect(
        Promise.race([
          Promise.all([
            worldServiceHealthNonce("127.0.0.1", address.port),
            worldServiceHealthNonce("127.0.0.1", address.port),
          ]),
          failAfter(600),
        ]),
      ).resolves.toEqual([null, null]);
    } finally {
      server.closeAllConnections();
      await new Promise<void>((resolve) => server.close(() => resolve()));
    }
  });

  it("waits for and escalates a stubborn descendant in direct-child fallback", async () => {
    if (process.platform === "win32") return;
    const grandchildSource = [
      'process.on("SIGTERM", () => {});',
      "setInterval(() => {}, 1_000);",
    ].join("");
    const parentSource = [
      'const { spawn } = require("node:child_process");',
      `const child = spawn(process.execPath, ["-e", ${JSON.stringify(grandchildSource)}],`,
      '  { stdio: "ignore" });',
      "process.stdout.write(String(child.pid));",
      'process.on("SIGTERM", () => process.exit(0));',
      "setInterval(() => {}, 1_000);",
    ].join("");
    const parent = spawn(process.execPath, ["-e", parentSource], {
      detached: true,
      stdio: ["ignore", "pipe", "inherit"],
    });
    let grandchildPid: number | undefined;
    try {
      grandchildPid = await new Promise<number>((resolve, reject) => {
        parent.once("error", reject);
        parent.stdout?.once("data", (chunk: Buffer) => resolve(Number(chunk.toString())));
      });
      expect(Number.isSafeInteger(grandchildPid)).toBe(true);

      await terminateProcessTree(parent, 250, true);

      expect(() => process.kill(grandchildPid as number, 0)).toThrow();
    } finally {
      if (parent.exitCode === null && parent.signalCode === null) parent.kill("SIGKILL");
      if (grandchildPid !== undefined) {
        try {
          process.kill(grandchildPid, "SIGKILL");
        } catch {
          // The assertion above verifies the expected ESRCH path; cleanup remains best effort.
        }
      }
    }
  });

  it("reaps a stubborn descendant forked during the termination grace window", async () => {
    if (process.platform === "win32") return;
    const lateChildSource = [
      'process.on("SIGTERM", () => {});',
      "setInterval(() => {}, 1_000);",
    ].join("");
    const parentSource = [
      'const { spawn } = require("node:child_process");',
      'process.stdout.write("ready\\n");',
      'process.once("SIGTERM", () => {',
      `  const child = spawn(process.execPath, ["-e", ${JSON.stringify(lateChildSource)}],`,
      '    { stdio: "ignore" });',
      "  process.stdout.write(`late:${child.pid}\\n`);",
      "  setTimeout(() => process.exit(0), 10);",
      "});",
      "setInterval(() => {}, 1_000);",
    ].join("");
    const parent = spawn(process.execPath, ["-e", parentSource], {
      detached: true,
      stdio: ["ignore", "pipe", "inherit"],
    });
    let output = "";
    parent.stdout?.on("data", (chunk: Buffer) => {
      output += chunk.toString();
    });
    let lateChildPid: number | undefined;
    try {
      await waitForOutput(parent, () => output, /^ready$/mu);

      const lateLine = waitForOutput(parent, () => output, /^late:\d+$/mu);
      await Promise.all([terminateProcessTree(parent, 250), lateLine]);

      lateChildPid = Number(/^late:(\d+)$/mu.exec(output)?.[1]);
      expect(Number.isSafeInteger(lateChildPid)).toBe(true);
      expect(() => process.kill(lateChildPid as number, 0)).toThrow();
    } finally {
      if (parent.exitCode === null && parent.signalCode === null) parent.kill("SIGKILL");
      if (lateChildPid !== undefined) {
        try {
          process.kill(lateChildPid, "SIGKILL");
        } catch {
          // Cleanup is best effort after the ESRCH assertion.
        }
      }
    }
  });

  it("rejects a stale listener instead of accepting the wrong daemon", async () => {
    const directory = mkdtempSync(path.join(tmpdir(), "voxels-world-port-"));
    const config = path.join(directory, "world-service.toml");
    const server = createTcpServer();
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

  it("resolves every Cargo profile binary through the shared target layout", () => {
    expect(cargoProfileExecutablePath("voxels-bots", "worldgen-dev")).toBe(
      path.resolve(
        process.env.CARGO_TARGET_DIR ?? "target",
        "worldgen-dev",
        process.platform === "win32" ? "voxels-bots.exe" : "voxels-bots",
      ),
    );
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
