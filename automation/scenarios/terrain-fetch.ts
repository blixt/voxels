import { ScenarioArguments } from "../lib/arguments.ts";
import { runProcess } from "../lib/process.ts";
import { defineScenario } from "../lib/scenario.ts";
import { rustTool } from "../../scripts/build-wasm.ts";

export default defineScenario({
  id: "terrain-fetch",
  kind: "setup",
  summary: "Fetches and validates the native Terrain Diffusion model assets.",
  uses: { rust: true },
  timeoutMs: 1_800_000,
  async run(context, arguments_) {
    new ScenarioArguments(arguments_).assertEmpty();
    await runProcess(
      context,
      rustTool("cargo"),
      [
        "run",
        "--profile",
        "worldgen",
        "-p",
        "voxels-world-terrain-diffusion",
        "--features",
        "metal,download",
        "--bin",
        "voxels-terrain-diffusion",
        "--",
        "fetch",
      ],
      { label: "Terrain Diffusion fetch", stdio: "inherit" },
    );
    return { summary: "Terrain Diffusion model assets are ready." };
  },
});
