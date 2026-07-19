import { ScenarioArguments } from "../lib/arguments.ts";
import { runProcess } from "../lib/process.ts";
import { defineScenario } from "../lib/scenario.ts";
import { rustTool } from "../../scripts/build-wasm.ts";
import { worldServiceCargoArgs } from "../../scripts/world-service-command.ts";

export default defineScenario({
  id: "world-source",
  kind: "validation",
  summary: "Validates configured world generation or surveys deterministic ecology.",
  uses: { world: true, metrics: true, rust: true },
  timeoutMs: 1_800_000,
  async run(context, arguments_) {
    const argumentsReader = new ScenarioArguments(arguments_);
    const mode = argumentsReader.choice("mode", ["smoke", "ecology-survey"] as const, "smoke");
    argumentsReader.assertEmpty();
    const cargoArguments = worldServiceCargoArgs({ sourceSmoke: true, metal: true });
    if (mode === "ecology-survey") cargoArguments.push("--ecology-survey");
    await runProcess(context, rustTool("cargo"), cargoArguments, {
      label: `world source ${mode}`,
      stdio: "inherit",
    });
    return { summary: `World source ${mode} passed.` };
  },
});
