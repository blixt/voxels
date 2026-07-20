import { runCriterionBenchmark } from "../lib/criterion.ts";
import { defineScenario } from "../lib/scenario.ts";

export default defineScenario({
  id: "bench-core",
  kind: "benchmark",
  summary: "Criterion fixed-step simulation benchmarks for the portable core crate.",
  uses: { metrics: true, rust: true },
  run(context, arguments_) {
    return runCriterionBenchmark(context, arguments_, {
      packageName: "voxels-core",
      benchName: "simulation",
    });
  },
});
