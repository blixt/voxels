import { execFileSync } from "node:child_process";
import { rustTool } from "./build-wasm.ts";
import { worldServiceCargoArgs } from "./world-service-command.ts";

const ecologySurvey = process.argv.includes("--ecology-survey");
const sourceSmoke = ecologySurvey || process.argv.includes("--source-smoke");
const metal = sourceSmoke || process.argv.includes("--metal");
const cargoArgs = worldServiceCargoArgs({ sourceSmoke, metal });
if (ecologySurvey) cargoArgs.push("--ecology-survey");

execFileSync(rustTool("cargo"), cargoArgs, {
  stdio: "inherit",
});
