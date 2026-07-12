import path from "node:path";
import { defineConfig, type Plugin } from "vite-plus";
import {
  buildWasm,
  ensureWasmBuilt,
  RUST_INPUT_FILES,
  RUST_SOURCE_DIRS,
} from "./scripts/build-wasm.ts";

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
      server.watcher.on("change", (file) => {
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
  plugins: mode === "test" ? [] : [rustWasm(command === "build")],
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
      "profile:browser": {
        command: "node scripts/browser-performance.mjs",
        cache: false,
      },
      "profile:sustained": {
        command: "node scripts/browser-performance.mjs --sustained",
        cache: false,
      },
      "profile:edits": {
        command: "node scripts/browser-performance.mjs --edits",
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
