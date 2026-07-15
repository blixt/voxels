import { execFileSync } from "node:child_process";
import { rustTool } from "./build-wasm.ts";

const command = process.argv[2];
const commands = new Set([
  "fetch",
  "smoke",
  "counterproof",
  "base-smoke",
  "detail-smoke",
  "survey-smoke",
]);
if (!command || !commands.has(command)) {
  throw new Error(
    "expected: fetch, smoke, counterproof, base-smoke, detail-smoke, or survey-smoke",
  );
}

execFileSync(
  rustTool("cargo"),
  [
    "run",
    "--profile",
    "worldgen",
    "-p",
    "voxels-world-terrain-diffusion",
    "--features",
    "metal",
    "--bin",
    "voxels-terrain-diffusion",
    "--",
    command,
  ],
  { stdio: "inherit" },
);
