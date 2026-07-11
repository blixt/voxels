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
  fmt: {
    ignorePatterns: [
      "src/**",
      "public/**",
      "docs/20260311-*.md",
      "docs/loop/**",
      "autoresearch*.md",
      "web/generated/**",
    ],
  },
  lint: {
    ignorePatterns: ["src/**", "public/**"],
    jsPlugins: [{ name: "vite-plus", specifier: "vite-plus/oxlint-plugin" }],
    rules: { "vite-plus/prefer-vite-plus-imports": "error" },
    options: { typeAware: true, typeCheck: true },
  },
  run: {
    tasks: {
      "check:rust": { command: "node scripts/check-rust.ts", cache: false },
      verify: {
        command: ["vp check", "vp test", "vp run check:rust", "vp build"],
        cache: false,
      },
    },
  },
}));
