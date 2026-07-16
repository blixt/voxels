import { execFileSync } from "node:child_process";
import { rustTool, wasmCcEnv } from "./build-wasm.ts";

function cargo(args: string[], env = process.env): void {
  execFileSync(rustTool("cargo"), args, {
    stdio: "inherit",
    env,
  });
}

cargo(["fmt", "--all", "--", "--check"]);
cargo(["test", "--workspace"]);
cargo([
  "clippy",
  "--workspace",
  "--exclude",
  "voxels-shell",
  "--all-targets",
  "--",
  "-D",
  "warnings",
]);
cargo(
  [
    "clippy",
    "-p",
    "voxels-shell",
    "--target",
    "wasm32-unknown-unknown",
    "--all-targets",
    "--",
    "-D",
    "warnings",
  ],
  {
    ...process.env,
    ...wasmCcEnv(),
  },
);
if (process.platform === "darwin") {
  cargo(["test", "-p", "voxels-world-terrain-diffusion", "--all-features"]);
  cargo([
    "clippy",
    "-p",
    "voxels-world-terrain-diffusion",
    "--all-features",
    "--all-targets",
    "--",
    "-D",
    "warnings",
  ]);
  cargo([
    "clippy",
    "-p",
    "voxels-world-service",
    "--features",
    "terrain-metal",
    "--all-targets",
    "--",
    "-D",
    "warnings",
  ]);
}
