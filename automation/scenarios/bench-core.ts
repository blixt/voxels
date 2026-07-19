import { rustTool } from "../../scripts/build-wasm.ts";
import { runProcess } from "../lib/process.ts";
import { defineScenario } from "../lib/scenario.ts";

export default defineScenario({
  id: "bench-core",
  kind: "benchmark",
  summary: "Criterion fixed-step simulation benchmarks for the portable core crate.",
  uses: { metrics: true, rust: true },
  async run(context) {
    await runProcess(
      context,
      rustTool("cargo"),
      ["bench", "-p", "voxels-core", "--bench", "simulation"],
      { label: "core benchmarks", stdio: "inherit" },
    );
  },
});
