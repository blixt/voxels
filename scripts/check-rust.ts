import { execFileSync } from "node:child_process";
import { rustTool, wasmCcEnv } from "./build-wasm.ts";

function cargo(args: string[], env = process.env): void {
  execFileSync(rustTool("cargo"), args, {
    stdio: "inherit",
    env,
  });
}

cargo(["fmt", "--all", "--", "--check"]);
cargo(["test", "--workspace", "--exclude", "voxels-shell"]);
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
  ["clippy", "-p", "voxels-shell", "--target", "wasm32-unknown-unknown", "--", "-D", "warnings"],
  {
    ...process.env,
    ...wasmCcEnv(),
  },
);
