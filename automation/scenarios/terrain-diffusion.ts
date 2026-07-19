import { ScenarioArguments } from "../lib/arguments.ts";
import { runProcess } from "../lib/process.ts";
import { defineScenario } from "../lib/scenario.ts";
import { rustTool } from "../../scripts/build-wasm.ts";

const commandByMode = {
  full: "smoke",
  counterproof: "counterproof",
  base: "base-smoke",
  detail: "detail-smoke",
  survey: "survey-smoke",
} as const;

export default defineScenario({
  id: "terrain-diffusion",
  kind: "validation",
  summary: "Runs native Rust/Metal Terrain Diffusion smoke, counterproof, or survey validation.",
  uses: { metrics: true, rust: true },
  timeoutMs: 1_800_000,
  async run(context, arguments_) {
    const argumentsReader = new ScenarioArguments(arguments_);
    const mode = argumentsReader.choice(
      "mode",
      ["full", "counterproof", "base", "detail", "survey"] as const,
      "full",
    );
    argumentsReader.assertEmpty();
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
        commandByMode[mode],
      ],
      { label: `Terrain Diffusion ${mode}`, stdio: "inherit" },
    );
    return { summary: `Terrain Diffusion ${mode} validation passed.` };
  },
});
