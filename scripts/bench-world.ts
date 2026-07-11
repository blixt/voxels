import { execFileSync } from "node:child_process";
import { rustTool } from "./build-wasm.ts";

execFileSync(rustTool("rustup"), ["run", "stable", "cargo", "bench", "-p", "voxels-world"], {
  stdio: "inherit",
});
