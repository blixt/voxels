import { execFileSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { homedir } from "node:os";
import { basename, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = process.cwd();
const OUT = join(ROOT, "web/generated");
const TARGET = "wasm32-unknown-unknown";
const CARGO_BIN = join(homedir(), ".cargo/bin");

export const RUST_SOURCE_DIRS = ["core/src", "world/src", "render/src", "shell/src"];
export const RUST_INPUT_FILES = [
  "Cargo.toml",
  "Cargo.lock",
  "core/Cargo.toml",
  "world/Cargo.toml",
  "render/Cargo.toml",
  "shell/Cargo.toml",
  "rust-toolchain.toml",
  "scripts/build-wasm.ts",
];

export function rustTool(name: string): string {
  const installed = join(CARGO_BIN, name);
  return existsSync(installed) ? installed : name;
}

export function wasmCcEnv(): Record<string, string> {
  const override = process.env.CC_wasm32_unknown_unknown;
  if (override) {
    return {
      CC_wasm32_unknown_unknown: override,
      AR_wasm32_unknown_unknown:
        process.env.AR_wasm32_unknown_unknown ?? siblingLlvmAr(override) ?? "llvm-ar",
    };
  }
  for (const compiler of [
    "/opt/homebrew/opt/llvm/bin/clang",
    "/usr/local/opt/llvm/bin/clang",
    "clang",
  ]) {
    if (emitsWasm(compiler)) {
      return {
        CC_wasm32_unknown_unknown: compiler,
        AR_wasm32_unknown_unknown: siblingLlvmAr(compiler) ?? "llvm-ar",
      };
    }
  }
  return {};
}

function siblingLlvmAr(compiler: string): string | null {
  if (!compiler.includes("/")) return null;
  const archiver = join(dirname(compiler), "llvm-ar");
  return existsSync(archiver) ? archiver : null;
}

function emitsWasm(compiler: string): boolean {
  try {
    execFileSync(
      compiler,
      ["--target=wasm32-unknown-unknown", "-x", "c", "-c", "-o", "/dev/null", "-"],
      { input: "int probe(void){return 0;}", stdio: ["pipe", "ignore", "ignore"] },
    );
    return true;
  } catch {
    return false;
  }
}

function run(command: string, args: string[]): void {
  execFileSync(command, args, {
    cwd: ROOT,
    stdio: "inherit",
    env: {
      ...process.env,
      PATH: `${CARGO_BIN}:${process.env.PATH ?? ""}`,
      ...wasmCcEnv(),
    },
  });
}

export function buildWasm(release = false): void {
  const profile = release ? "release" : "debug";
  const cargoArgs = ["build", "--target", TARGET, "-p", "voxels-shell"];
  if (release) cargoArgs.push("--release");
  run(rustTool("cargo"), cargoArgs);

  mkdirSync(join(ROOT, "target"), { recursive: true });
  const staging = mkdtempSync(join(ROOT, "target/wasm-bindgen-"));
  try {
    run(rustTool("wasm-bindgen"), [
      "--target",
      "web",
      "--out-dir",
      staging,
      "--out-name",
      "voxels",
      join(ROOT, "target", TARGET, profile, "voxels.wasm"),
    ]);
    publish(staging);
  } finally {
    rmSync(staging, { recursive: true, force: true });
  }
}

function publish(staging: string): void {
  // wasm-bindgen emits explicit-resource-management syntax in its declaration. Vite+'s current
  // type-aware lint worker does not load that lib even when tsc does, so omit the optional declaration
  // until the toolchains agree. `free()` and our explicit `destroy()` remain typed.
  const declaration = join(staging, "voxels.d.ts");
  if (existsSync(declaration)) {
    const source = readFileSync(declaration, "utf8");
    writeFileSync(declaration, source.replace("    [Symbol.dispose](): void;\n", ""));
  }
  mkdirSync(OUT, { recursive: true });
  const next = readdirSync(staging).sort(
    (left, right) => Number(left.endsWith(".js")) - Number(right.endsWith(".js")),
  );
  for (const name of next) renameSync(join(staging, name), join(OUT, name));
  const keep = new Set(next);
  for (const name of readdirSync(OUT)) {
    if (!keep.has(name)) rmSync(join(OUT, name), { recursive: true, force: true });
  }
}

function newestSource(path: string): number {
  if (!existsSync(path)) return 0;
  const stats = statSync(path);
  if (!stats.isDirectory()) return stats.mtimeMs;
  let newest = 0;
  for (const name of readdirSync(path)) {
    if (basename(name).startsWith(".")) continue;
    newest = Math.max(newest, newestSource(join(path, name)));
  }
  return newest;
}

export function ensureWasmBuilt(): void {
  const artifact = join(OUT, "voxels_bg.wasm");
  const built = existsSync(artifact) ? statSync(artifact).mtimeMs : 0;
  const newest = Math.max(
    ...RUST_SOURCE_DIRS.map((path) => newestSource(join(ROOT, path))),
    ...RUST_INPUT_FILES.map((path) => newestSource(join(ROOT, path))),
  );
  if (built < newest || !existsSync(join(OUT, "voxels.js"))) buildWasm(false);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  buildWasm(process.argv.includes("--release"));
}
