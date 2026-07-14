import { execFileSync } from "node:child_process";
import { rustTool } from "./build-wasm.ts";
import { worldServiceCargoArgs } from "./world-service-command.ts";

const sourceSmoke = process.argv.includes("--source-smoke");
const metal = sourceSmoke || process.argv.includes("--metal");

execFileSync(rustTool("cargo"), worldServiceCargoArgs({ sourceSmoke, metal }), {
  stdio: "inherit",
});
