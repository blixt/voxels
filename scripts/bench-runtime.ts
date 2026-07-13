import { execFileSync } from "node:child_process";
import { rustTool } from "./build-wasm.ts";

execFileSync(rustTool("cargo"), ["bench", "-p", "voxels-runtime", "--bench", "streaming"], {
  stdio: "inherit",
});
