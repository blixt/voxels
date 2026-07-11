import { execFileSync } from "node:child_process";
import { rustTool, wasmCcEnv } from "./build-wasm.ts";

function cargo(args: string[], env = process.env): void {
  execFileSync(rustTool("cargo"), args, {
    stdio: "inherit",
    env,
  });
}

cargo(["fmt", "--all", "--", "--check"]);
cargo(["test", "-p", "voxels-core", "-p", "voxels-world", "-p", "voxels-render"]);
cargo([
  "clippy",
  "-p",
  "voxels-core",
  "-p",
  "voxels-world",
  "-p",
  "voxels-render",
  "--all-targets",
  "--",
  "-D",
  "warnings",
]);
cargo(["clippy", "--workspace", "--target", "wasm32-unknown-unknown", "--", "-D", "warnings"], {
  ...process.env,
  ...wasmCcEnv(),
});
