import { rustTool } from "../../scripts/build-wasm.ts";
import { runProcess } from "../lib/process.ts";
import { defineScenario } from "../lib/scenario.ts";

export default defineScenario({
  id: "bench-world",
  kind: "benchmark",
  summary: "Criterion generation, codec, and meshing benchmarks for the portable world crate.",
  uses: { metrics: true, rust: true },
  async run(context) {
    await runProcess(context, rustTool("cargo"), ["bench", "-p", "voxels-world"], {
      label: "world benchmarks",
      stdio: "inherit",
    });
  },
});
