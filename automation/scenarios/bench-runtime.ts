import { runCriterionBenchmark } from "../lib/criterion.ts";
import { defineScenario } from "../lib/scenario.ts";

export default defineScenario({
  id: "bench-runtime",
  kind: "benchmark",
  summary: "Criterion streaming-scheduler benchmarks for the portable runtime crate.",
  uses: { metrics: true, rust: true },
  run(context, arguments_) {
    return runCriterionBenchmark(context, arguments_, {
      packageName: "voxels-runtime",
      benchName: "streaming",
    });
  },
});
