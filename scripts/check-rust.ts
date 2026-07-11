import { execFileSync } from "node:child_process";
import { rustTool } from "./build-wasm.ts";

function cargo(args: string[]): void {
  execFileSync(rustTool("rustup"), ["run", "stable", "cargo", ...args], {
    stdio: "inherit",
  });
}

cargo(["fmt", "--all", "--", "--check"]);
cargo(["test", "-p", "voxels-core", "-p", "voxels-render"]);
cargo([
  "clippy",
  "-p",
  "voxels-core",
  "-p",
  "voxels-render",
  "--all-targets",
  "--",
  "-D",
  "warnings",
]);
cargo(["clippy", "--workspace", "--target", "wasm32-unknown-unknown", "--", "-D", "warnings"]);
