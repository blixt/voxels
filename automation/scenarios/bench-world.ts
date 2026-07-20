import { runCriterionBenchmark } from "../lib/criterion.ts";
import { defineScenario } from "../lib/scenario.ts";

export default defineScenario({
  id: "bench-world",
  kind: "benchmark",
  summary: "Criterion generation, codec, and meshing benchmarks for the portable world crate.",
  uses: { metrics: true, rust: true },
  run(context, arguments_) {
    return runCriterionBenchmark(context, arguments_, {
      packageName: "voxels-world",
      benchName: "world",
    });
  },
});
