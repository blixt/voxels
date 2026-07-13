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
import { homedir, tmpdir } from "node:os";
import { basename, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = process.cwd();
const OUT = join(ROOT, "web/generated");
const TARGET = "wasm32-unknown-unknown";
const CARGO_BIN = join(homedir(), ".cargo/bin");

const RUST_WORKSPACE_CRATES = ["core", "world", "runtime", "render", "shell"] as const;

export const RUST_SOURCE_DIRS = RUST_WORKSPACE_CRATES.map((crate) => `${crate}/src`);
export const RUST_INPUT_FILES = [
  "Cargo.toml",
  "Cargo.lock",
  ...RUST_WORKSPACE_CRATES.map((crate) => `${crate}/Cargo.toml`),
  "rust-toolchain.toml",
  "scripts/build-wasm.ts",
];

export function rustTool(name: string): string {
  const installed = join(CARGO_BIN, name);
  return existsSync(installed) ? installed : name;
}

export function validateWasmBindgenCliVersion(output: string, expected: string): void {
  const installed = /^wasm-bindgen\s+(\S+)$/u.exec(output.trim())?.[1];
  if (installed === expected) return;
  const found = installed ? `found ${installed}` : "could not read the installed version";
  throw new Error(
    `wasm-bindgen-cli ${expected} is required (${found}); install it with ` +
      `cargo install --locked wasm-bindgen-cli --version ${expected}`,
  );
}

function wasmBindgenTool(): string {
  const manifest = readFileSync(join(ROOT, "shell/Cargo.toml"), "utf8");
  const expected = /^wasm-bindgen\s*=\s*"=([^"]+)"$/mu.exec(manifest)?.[1];
  if (!expected) {
    throw new Error("shell/Cargo.toml must pin wasm-bindgen to an exact version");
  }
  const command = rustTool("wasm-bindgen");
  let output: string;
  try {
    output = execFileSync(command, ["--version"], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    });
  } catch {
    output = "";
  }
  validateWasmBindgenCliVersion(output, expected);
  return command;
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
  const probe = mkdtempSync(join(tmpdir(), "voxels-wasm-cc-"));
  try {
    execFileSync(
      compiler,
      ["--target=wasm32-unknown-unknown", "-x", "c", "-c", "-o", join(probe, "probe.o"), "-"],
      { input: "int probe(void){return 0;}", stdio: ["pipe", "ignore", "ignore"] },
    );
    return true;
  } catch {
    return false;
  } finally {
    rmSync(probe, { recursive: true, force: true });
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
  const wasmBindgen = wasmBindgenTool();
  const cargoArgs = ["build", "--target", TARGET, "-p", "voxels-shell"];
  if (release) cargoArgs.push("--release");
  run(rustTool("cargo"), cargoArgs);

  mkdirSync(join(ROOT, "target"), { recursive: true });
  const staging = mkdtempSync(join(ROOT, "target/wasm-bindgen-"));
  try {
    run(wasmBindgen, [
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
  const declaration = join(staging, "voxels.d.ts");
  if (existsSync(declaration)) {
    const source = readFileSync(declaration, "utf8");
    writeFileSync(declaration, normalizeWasmDeclaration(source));
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

export function normalizeWasmDeclaration(source: string): string {
  // wasm-bindgen emits explicit-resource-management syntax that Vite+'s current type-aware lint
  // worker does not load. `free()` and our explicit `destroy()` remain typed.
  const supported = source.replace("    [Symbol.dispose](): void;\n", "");
  // Debug and release builds expose different hashed closure trampolines in InitOutput. They are
  // implementation details rather than callable application exports, so retaining them makes the
  // one tracked declaration alternate between profiles without adding useful type information.
  return supported
    .split("\n")
    .filter(
      (line) =>
        !(line.startsWith("    readonly wasm_bindgen_") && line.includes("___convert__closures")),
    )
    .join("\n");
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
