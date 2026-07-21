import { type ChildProcess, spawn } from "node:child_process";
import { readFileSync } from "node:fs";
import { createConnection } from "node:net";
import path from "node:path";
import { defineConfig, type Plugin } from "vite-plus";
import {
  buildWasm,
  ensureWasmBuilt,
  RUST_INPUT_FILES,
  RUST_SOURCE_DIRS,
  rustTool,
} from "./scripts/build-wasm.ts";
import type { WasmBuildProfile } from "./scripts/build-wasm.ts";
import { worldServiceBuildCargoArgs } from "./scripts/world-service-command.ts";
import type { WorldServiceCargoProfile } from "./scripts/world-service-command.ts";

interface RustInputWatcher {
  on(event: "add" | "change" | "unlink", listener: (file: string) => void): unknown;
}

const CLIENT_CONFIG_SOURCE = path.resolve(
  process.env.VOXELS_CLIENT_CONFIG_PATH ?? "config/client.toml",
);
const WORLD_SERVICE_CONFIG_SOURCE = path.resolve(
  process.env.VOXELS_WORLD_SERVICE_CONFIG_PATH ?? "config/world-service.toml",
);
const GENERATED_WASM_DIRECTORY = path.resolve("web/generated");
const BROWSER_RUNTIME_DIRECTORY = path.resolve("web");
let sharedBrowserBuildReady = true;

export function worldServiceDevelopmentProfile(
  configured = process.env.VOXELS_WORLD_SERVICE_PROFILE,
): WorldServiceCargoProfile {
  if (configured === undefined || configured === "") return "worldgen-dev";
  if (configured === "worldgen" || configured === "worldgen-dev") return configured;
  throw new Error(
    `invalid VOXELS_WORLD_SERVICE_PROFILE ${configured}; expected worldgen-dev or worldgen`,
  );
}

export function browserWasmProfile(
  command: "build" | "serve",
  configured = process.env.VOXELS_BROWSER_BUILD_PROFILE,
  mode?: string,
): WasmBuildProfile {
  const automationProfile = mode?.match(/^automation-(debug|wasm-dev|release)$/u)?.[1];
  if (
    automationProfile === "debug" ||
    automationProfile === "wasm-dev" ||
    automationProfile === "release"
  ) {
    return automationProfile;
  }
  if (configured === undefined || configured === "") {
    return command === "build" ? "release" : "wasm-dev";
  }
  if (configured === "debug" || configured === "wasm-dev" || configured === "release") {
    return configured;
  }
  throw new Error(
    `invalid VOXELS_BROWSER_BUILD_PROFILE ${configured}; expected debug, wasm-dev, or release`,
  );
}

const NATIVE_WORLD_SERVICE_SOURCE_DIRS = [
  "world-service/src",
  "world-terrain-diffusion/src",
  "core/src",
  "world/src",
].map((source) => path.resolve(source));
const NATIVE_WORLD_SERVICE_INPUT_FILES = [
  "Cargo.toml",
  "Cargo.lock",
  "rust-toolchain.toml",
  "world-service/Cargo.toml",
  "world-terrain-diffusion/Cargo.toml",
  "world-terrain-diffusion/fixtures/pipeline-data.json",
  "core/Cargo.toml",
  "world/Cargo.toml",
  WORLD_SERVICE_CONFIG_SOURCE,
].map((source) => path.resolve(source));

export function pathBelongsTo(file: string, directory: string): boolean {
  const relative = path.relative(directory, path.resolve(file));
  return relative === "" || (!relative.startsWith("..") && !path.isAbsolute(relative));
}

export function isNativeWorldServiceInput(file: string): boolean {
  const resolved = path.resolve(file);
  return (
    NATIVE_WORLD_SERVICE_INPUT_FILES.includes(resolved) ||
    NATIVE_WORLD_SERVICE_SOURCE_DIRS.some((directory) => pathBelongsTo(resolved, directory))
  );
}

export function worldServiceListenAddress(contents: string): { host: string; port: number } {
  const value = /^listen\s*=\s*"([^"]+)"\s*(?:#.*)?$/mu.exec(contents)?.[1];
  if (!value) throw new Error("world-service config is missing transport.listen");
  const separator = value.lastIndexOf(":");
  if (separator <= 0) throw new Error(`invalid world-service transport.listen: ${value}`);
  const host = value.slice(0, separator).replace(/^\[|\]$/gu, "");
  const port = Number(value.slice(separator + 1));
  if (!Number.isInteger(port) || port <= 0 || port > 65_535) {
    throw new Error(`invalid world-service transport.listen port: ${value}`);
  }
  return { host, port };
}

function clientConfig(emitBuildAsset: boolean): Plugin {
  return {
    name: "voxels-client-config",
    buildStart() {
      if (!emitBuildAsset) return;
      this.addWatchFile(CLIENT_CONFIG_SOURCE);
      this.emitFile({
        type: "asset",
        fileName: "config/client.toml",
        source: readFileSync(CLIENT_CONFIG_SOURCE, "utf8"),
      });
    },
    configureServer(server) {
      server.middlewares.use("/config/client.toml", (_request, response) => {
        response.statusCode = 200;
        response.setHeader("Content-Type", "text/plain; charset=utf-8");
        response.setHeader("Cache-Control", "no-store");
        response.end(readFileSync(CLIENT_CONFIG_SOURCE, "utf8"));
      });
      server.watcher.add(CLIENT_CONFIG_SOURCE);
      server.watcher.on("change", (file) => {
        if (path.resolve(file) === CLIENT_CONFIG_SOURCE) {
          server.ws.send({ type: "full-reload" });
        }
      });
    },
  };
}

export function watchRustInputChanges(
  watcher: RustInputWatcher,
  listener: (file: string) => void,
): void {
  for (const event of ["add", "change", "unlink"] as const) watcher.on(event, listener);
}

function rustWasm(profile: WasmBuildProfile): Plugin {
  const directories = RUST_SOURCE_DIRS.map((source) => path.resolve(source));
  const files = RUST_INPUT_FILES.map((source) => path.resolve(source));
  let timer: ReturnType<typeof setTimeout> | undefined;
  let nativeReloadWillFollow = false;
  const isInput = (file: string): boolean =>
    files.includes(file) ||
    directories.some((directory) => file.startsWith(`${directory}${path.sep}`));
  return {
    name: "voxels-rust-wasm",
    handleHotUpdate(context) {
      // wasm-bindgen publishes several stable filenames. Suppress Vite's per-file updates and let
      // the completed Rust build below issue exactly one reload after the artifact set is coherent.
      if (pathBelongsTo(context.file, GENERATED_WASM_DIRECTORY)) return [];
    },
    buildStart() {
      ensureWasmBuilt(profile);
    },
    configureServer(server) {
      for (const input of [...directories, ...files]) server.watcher.add(input);
      watchRustInputChanges(server.watcher, (file) => {
        if (!isInput(file)) return;
        nativeReloadWillFollow ||= isNativeWorldServiceInput(file);
        if (timer) clearTimeout(timer);
        timer = setTimeout(() => {
          timer = undefined;
          const delegateReload = nativeReloadWillFollow;
          nativeReloadWillFollow = false;
          try {
            buildWasm(profile);
            sharedBrowserBuildReady = true;
            if (!delegateReload) {
              server.moduleGraph.invalidateAll();
              server.ws.send({ type: "full-reload" });
            }
          } catch (error) {
            if (delegateReload) sharedBrowserBuildReady = false;
            server.config.logger.error(`[voxels-rust-wasm] ${String(error)}`);
          }
        }, 75);
      });
    },
  };
}

function canvasRuntimeReload(): Plugin {
  return {
    name: "voxels-canvas-runtime-reload",
    handleHotUpdate(context) {
      if (
        pathBelongsTo(context.file, BROWSER_RUNTIME_DIRECTORY) &&
        !pathBelongsTo(context.file, GENERATED_WASM_DIRECTORY) &&
        !context.file.endsWith(".css")
      ) {
        // A transferred OffscreenCanvas is a one-shot browser resource. A single acknowledged page
        // reload is the safe HMR boundary for worker/runtime code; CSS remains true hot replacement.
        context.server.ws.send({ type: "full-reload" });
        return [];
      }
    },
  };
}

function signalProcessTree(child: ChildProcess, signal: NodeJS.Signals): void {
  if (child.exitCode !== null || child.signalCode !== null) return;
  try {
    if (process.platform !== "win32" && child.pid !== undefined) {
      process.kill(-child.pid, signal);
    } else {
      child.kill(signal);
    }
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ESRCH") throw error;
  }
}

async function terminateProcessTree(child: ChildProcess, timeoutMs = 2_000): Promise<void> {
  if (child.exitCode !== null || child.signalCode !== null) return;
  const exited = new Promise<void>((resolve) => child.once("exit", () => resolve()));
  signalProcessTree(child, "SIGTERM");
  await Promise.race([exited, new Promise<void>((resolve) => setTimeout(resolve, timeoutMs))]);
  if (child.exitCode === null && child.signalCode === null) {
    signalProcessTree(child, "SIGKILL");
    await exited;
  }
}

function delay(milliseconds: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

function portAcceptsConnections(host: string, port: number): Promise<boolean> {
  return new Promise((resolve) => {
    const socket = createConnection({ host, port });
    let settled = false;
    const finish = (ready: boolean): void => {
      if (settled) return;
      settled = true;
      socket.destroy();
      resolve(ready);
    };
    socket.setTimeout(250, () => finish(false));
    socket.once("connect", () => finish(true));
    socket.once("error", () => finish(false));
  });
}

async function waitForWorldService(
  child: ChildProcess,
  configPath: string,
  timeoutMs = 300_000,
): Promise<void> {
  const { host, port } = worldServiceListenAddress(readFileSync(configPath, "utf8"));
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (child.exitCode !== null || child.signalCode !== null) {
      throw new Error("native daemon exited before accepting connections");
    }
    if (await portAcceptsConnections(host, port)) return;
    await delay(75);
  }
  throw new Error(`native daemon did not listen on ${host}:${port} within ${timeoutMs}ms`);
}

function childExitReason(code: number | null, signal: NodeJS.Signals | null): string {
  return signal ? `signal ${signal}` : `status ${code ?? "unknown"}`;
}

function nativeWorldService(): Plugin {
  const profile = worldServiceDevelopmentProfile();
  let buildChild: ChildProcess | undefined;
  let daemonChild: ChildProcess | undefined;
  let stopping = false;
  let building = false;
  let dirty = false;
  let reloadRequested = false;
  let rebuildTimer: ReturnType<typeof setTimeout> | undefined;
  let crashAttempts = 0;
  let finishInitialBuild: (() => Promise<void>) | undefined;
  return {
    name: "voxels-native-world-service",
    apply: (_config, environment) =>
      environment.command === "serve" && process.env.VOXELS_EXTERNAL_WORLD_SERVICE !== "1",
    async buildStart() {
      await finishInitialBuild?.();
    },
    configureServer(server) {
      let handleSignal: (() => void) | undefined;
      const nativeInputs = [
        ...NATIVE_WORLD_SERVICE_SOURCE_DIRS,
        ...NATIVE_WORLD_SERVICE_INPUT_FILES,
      ];
      const stop = async (): Promise<void> => {
        if (stopping) return;
        stopping = true;
        if (rebuildTimer) clearTimeout(rebuildTimer);
        if (handleSignal) {
          process.off("SIGINT", handleSignal);
          process.off("SIGTERM", handleSignal);
        }
        const build = buildChild;
        const daemon = daemonChild;
        buildChild = undefined;
        daemonChild = undefined;
        await Promise.all([
          build ? terminateProcessTree(build) : Promise.resolve(),
          daemon ? terminateProcessTree(daemon) : Promise.resolve(),
        ]);
      };

      const compile = (): Promise<boolean> => {
        server.config.logger.info(
          `[voxels-world-service] compiling native Terrain Diffusion/Metal daemon (${profile})`,
        );
        return new Promise((resolve) => {
          const child = spawn(
            rustTool("cargo"),
            worldServiceBuildCargoArgs({ metal: true, profile }),
            {
              cwd: process.cwd(),
              env: process.env,
              stdio: "inherit",
              detached: process.platform !== "win32",
            },
          );
          buildChild = child;
          let settled = false;
          const finish = (success: boolean): void => {
            if (settled) return;
            settled = true;
            if (buildChild === child) buildChild = undefined;
            resolve(success);
          };
          child.once("error", (error) => {
            server.config.logger.error(
              `[voxels-world-service] failed to compile: ${error.message}`,
            );
            finish(false);
          });
          child.once("exit", (code, signal) => {
            if (stopping) {
              finish(false);
            } else if (code === 0) {
              finish(true);
            } else {
              server.config.logger.error(
                `[voxels-world-service] build exited with ${childExitReason(code, signal)}`,
              );
              finish(false);
            }
          });
        });
      };

      const launch = async (): Promise<void> => {
        const executable = path.resolve(
          process.env.CARGO_TARGET_DIR ?? "target",
          profile,
          process.platform === "win32" ? "voxels-worldd.exe" : "voxels-worldd",
        );
        server.config.logger.info("[voxels-world-service] starting native daemon");
        const child = spawn(executable, [WORLD_SERVICE_CONFIG_SOURCE], {
          cwd: process.cwd(),
          env: process.env,
          stdio: "inherit",
          detached: process.platform !== "win32",
        });
        daemonChild = child;
        child.once("error", (error) => {
          if (daemonChild !== child || stopping) return;
          daemonChild = undefined;
          server.config.logger.error(`[voxels-world-service] failed to start: ${error.message}`);
        });
        child.once("exit", (code, signal) => {
          if (daemonChild !== child || stopping) return;
          daemonChild = undefined;
          crashAttempts += 1;
          server.config.logger.error(
            `[voxels-world-service] daemon exited with ${childExitReason(code, signal)}`,
          );
          if (crashAttempts <= 5) {
            scheduleRebuild(true, Math.min(250 * 2 ** (crashAttempts - 1), 4_000));
          } else {
            server.config.logger.error(
              "[voxels-world-service] automatic restart limit reached; waiting for a source or config change",
            );
          }
        });
        await waitForWorldService(child, WORLD_SERVICE_CONFIG_SOURCE);
        if (daemonChild !== child) {
          throw new Error("native daemon was replaced before becoming ready");
        }
        server.config.logger.info("[voxels-world-service] native daemon ready");
      };

      const rebuild = async (reload: boolean): Promise<boolean> => {
        if (stopping) return false;
        const compiled = await compile();
        if (!compiled || stopping) return false;
        if (!sharedBrowserBuildReady) {
          const error = new Error(
            "retaining the previous daemon because the matching browser WASM build failed",
          );
          server.config.logger.error(`[voxels-world-service] ${error.message}`);
          return false;
        }
        const previous = daemonChild;
        if (previous) {
          // Let every open browser close its world/presence sockets first so the authoritative
          // service can checkpoint the latest player pose before this development-only swap.
          server.ws.send({ type: "custom", event: "voxels:before-world-restart", data: {} });
          await delay(1_050);
        }
        daemonChild = undefined;
        if (previous) await terminateProcessTree(previous);
        try {
          await launch();
        } catch (error) {
          const failedDaemon = daemonChild;
          daemonChild = undefined;
          if (failedDaemon) await terminateProcessTree(failedDaemon);
          server.config.logger.error(`[voxels-world-service] ${String(error)}`);
          return false;
        }
        if (reload) {
          server.moduleGraph.invalidateAll();
          server.ws.send({ type: "full-reload" });
        }
        return true;
      };

      const drainRebuilds = async (): Promise<void> => {
        if (building || stopping) return;
        building = true;
        try {
          while (dirty && !stopping) {
            dirty = false;
            const reload = reloadRequested;
            reloadRequested = false;
            await rebuild(reload);
          }
        } finally {
          building = false;
          if (dirty && !stopping) scheduleRebuild(reloadRequested, 0);
        }
      };

      function scheduleRebuild(reload: boolean, delayMs = 100): void {
        if (stopping) return;
        dirty = true;
        reloadRequested ||= reload;
        if (rebuildTimer) clearTimeout(rebuildTimer);
        rebuildTimer = setTimeout(() => {
          rebuildTimer = undefined;
          void drainRebuilds();
        }, delayMs);
      }

      handleSignal = (): void => {
        void stop()
          .then(() => server.close())
          .finally(() => process.exit(0));
      };

      server.httpServer?.once("close", () => void stop());
      process.once("SIGINT", handleSignal);
      process.once("SIGTERM", handleSignal);
      for (const input of nativeInputs) server.watcher.add(input);
      watchRustInputChanges(server.watcher, (file) => {
        if (!isNativeWorldServiceInput(file)) return;
        crashAttempts = 0;
        scheduleRebuild(true);
      });
      // Vite runs configureServer before buildStart. Start native compilation immediately, let the
      // preceding Rust/WASM buildStart hook use the other cores, then launch only after both
      // artifacts succeeded. The HTTP server does not listen until every buildStart hook completes.
      const compiled = compile();
      finishInitialBuild = async (): Promise<void> => {
        if (!(await compiled) || stopping) {
          throw new Error("initial native world-service build failed");
        }
        if (!sharedBrowserBuildReady) {
          throw new Error("matching browser WASM build failed");
        }
        await launch();
      };
    },
  };
}

export default defineConfig(({ command, mode }) => ({
  plugins:
    mode === "test"
      ? []
      : [
          clientConfig(command === "build"),
          rustWasm(browserWasmProfile(command, undefined, mode)),
          canvasRuntimeReload(),
          nativeWorldService(),
        ],
  // A renderer failure belongs in the console. Vite's default HMR overlay appends a shadow-DOM
  // element over the canvas, which violates the engine's canvas-only host contract.
  server: { hmr: { overlay: false } },
  // Current engines support modulepreload. Avoid Vite's compatibility shim because its feature probe
  // constructs detached DOM elements even though the application itself owns only one canvas.
  build: { modulePreload: { polyfill: false } },
  fmt: {
    ignorePatterns: ["docs/20260311-*.md", "docs/loop/**", "web/generated/**"],
  },
  lint: {
    jsPlugins: [{ name: "vite-plus", specifier: "vite-plus/oxlint-plugin" }],
    rules: { "vite-plus/prefer-vite-plus-imports": "error" },
    options: { typeAware: true, typeCheck: true },
  },
  run: {
    tasks: {
      automation: {
        command: "node automation/cli.ts",
        cache: false,
      },
      "check:rust": { command: "node scripts/check-rust.ts", cache: false },
      wrangler: {
        command: "node scripts/wrangler-local.mjs",
        cache: false,
      },
      "build:production": {
        command: "VOXELS_CLIENT_CONFIG_PATH=config/client.production.toml vp build",
        cache: false,
      },
      deploy: {
        command: "node scripts/wrangler-local.mjs deploy",
        dependsOn: ["build:production"],
        cache: false,
      },
      verify: {
        command: ["vp check", "vp test", "vp run check:rust", "vp build"],
        cache: false,
      },
    },
  },
}));
