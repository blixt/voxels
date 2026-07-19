import { rustTool } from "../../scripts/build-wasm.ts";
import { runProcess } from "../lib/process.ts";
import { defineScenario } from "../lib/scenario.ts";

export default defineScenario({
  id: "bench-runtime",
  kind: "benchmark",
  summary: "Criterion streaming-scheduler benchmarks for the portable runtime crate.",
  uses: { metrics: true, rust: true },
  async run(context) {
    await runProcess(
      context,
      rustTool("cargo"),
      ["bench", "-p", "voxels-runtime", "--bench", "streaming"],
      { label: "runtime benchmarks", stdio: "inherit" },
    );
  },
});
