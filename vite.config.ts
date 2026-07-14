import { readFileSync } from "node:fs";
import path from "node:path";
import { defineConfig, type Plugin } from "vite-plus";
import {
  buildWasm,
  ensureWasmBuilt,
  RUST_INPUT_FILES,
  RUST_SOURCE_DIRS,
} from "./scripts/build-wasm.ts";

interface RustInputWatcher {
  on(event: "add" | "change" | "unlink", listener: (file: string) => void): unknown;
}

const CLIENT_CONFIG_SOURCE = path.resolve("config/client.toml");

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

function rustWasm(release: boolean): Plugin {
  const directories = RUST_SOURCE_DIRS.map((source) => path.resolve(source));
  const files = RUST_INPUT_FILES.map((source) => path.resolve(source));
  let timer: ReturnType<typeof setTimeout> | undefined;
  const isInput = (file: string): boolean =>
    files.includes(file) ||
    directories.some((directory) => file.startsWith(`${directory}${path.sep}`));
  return {
    name: "voxels-rust-wasm",
    buildStart() {
      if (release) buildWasm(true);
      else ensureWasmBuilt();
    },
    configureServer(server) {
      for (const input of [...directories, ...files]) server.watcher.add(input);
      watchRustInputChanges(server.watcher, (file) => {
        if (!isInput(file)) return;
        if (timer) clearTimeout(timer);
        timer = setTimeout(() => {
          timer = undefined;
          try {
            buildWasm(false);
            server.ws.send({ type: "full-reload" });
          } catch (error) {
            server.config.logger.error(`[voxels-rust-wasm] ${String(error)}`);
          }
        }, 75);
      });
    },
  };
}

export default defineConfig(({ command, mode }) => ({
  plugins:
    mode === "test" ? [] : [clientConfig(command === "build"), rustWasm(command === "build")],
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
      "bench:world": { command: "node scripts/bench-world.ts", cache: false },
      "bench:core": {
        command: "node scripts/bench-core.ts",
        cache: false,
      },
      "bench:runtime": {
        command: "node scripts/bench-runtime.ts",
        cache: false,
      },
      "terrain:fetch": {
        command: "node scripts/terrain-diffusion.ts fetch",
        cache: false,
      },
      "terrain:smoke": {
        command: "node scripts/terrain-diffusion.ts smoke",
        cache: false,
      },
      "terrain:counterproof": {
        command: "node scripts/terrain-diffusion.ts counterproof",
        cache: false,
      },
      "terrain:base": {
        command: "node scripts/terrain-diffusion.ts base-smoke",
        cache: false,
      },
      "terrain:detail": {
        command: "node scripts/terrain-diffusion.ts detail-smoke",
        cache: false,
      },
      "world:source-smoke": {
        command: "node scripts/world-service.ts --source-smoke",
        cache: false,
      },
      "world:serve": {
        command: "node scripts/world-service.ts",
        cache: false,
      },
      "world:serve-metal": {
        command: "node scripts/world-service.ts --metal",
        cache: false,
      },
      "profile:browser": {
        command: "node scripts/browser-performance.mjs",
        cache: false,
      },
      "profile:sustained": {
        command: "node scripts/browser-performance.mjs --sustained",
        cache: false,
      },
      "profile:materials": {
        command: "node scripts/browser-performance.mjs --materials",
        cache: false,
      },
      "profile:atmosphere": {
        command: "node scripts/browser-performance.mjs --atmosphere",
        cache: false,
      },
      "profile:gtao": {
        command: "node scripts/browser-performance.mjs --gtao",
        cache: false,
      },
      "profile:heroes": {
        command: "node scripts/browser-performance.mjs --heroes",
        cache: false,
      },
      "profile:caves": {
        command: "node scripts/browser-performance.mjs --caves",
        cache: false,
      },
      "profile:lights": {
        command: "node scripts/browser-performance.mjs --lights",
        cache: false,
      },
      "profile:portals": {
        command: "node scripts/browser-performance.mjs --portals",
        cache: false,
      },
      "test:multiplayer-browser": {
        command: "node scripts/browser-multiplayer.mjs",
        cache: false,
      },
      "profile:portal-edits": {
        command: "node scripts/browser-performance.mjs --portal-edits",
        cache: false,
      },
      "profile:portal-streaming": {
        command: "node scripts/browser-performance.mjs --portal-streaming",
        cache: false,
      },
      "test:persistence-browser": {
        command: "node scripts/browser-persistence-test.mjs",
        cache: false,
      },
      "test:persistence-recovery": {
        command: "node scripts/browser-persistence-recovery-test.mjs",
        cache: false,
      },
      "check:rust": { command: "node scripts/check-rust.ts", cache: false },
      verify: {
        command: ["vp check", "vp test", "vp run check:rust", "vp build"],
        cache: false,
      },
    },
  },
}));
