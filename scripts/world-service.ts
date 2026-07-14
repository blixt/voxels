import { execFileSync } from "node:child_process";
import { rustTool } from "./build-wasm.ts";

const sourceSmoke = process.argv.includes("--source-smoke");
const metal = sourceSmoke || process.argv.includes("--metal");
const args = [
  "run",
  "--profile",
  "worldgen",
  "-p",
  "voxels-world-service",
  ...(metal ? ["--features", "terrain-metal"] : []),
  "--bin",
  sourceSmoke ? "voxels-world-source" : "voxels-worldd",
  "--",
  "config/world-service.toml",
];

execFileSync(rustTool("cargo"), args, { stdio: "inherit" });
