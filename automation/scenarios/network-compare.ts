import { readFile } from "node:fs/promises";
import {
  compareNetworkBenchmarks,
  comparisonMarkdown,
  parseNetworkBenchmarkResult,
} from "../lib/network-compare.ts";
import { defineScenario } from "../lib/scenario.ts";

export default defineScenario({
  id: "network-compare",
  kind: "analysis",
  summary: "Validates and compares two compatible network benchmark result files.",
  uses: { metrics: true, network: true },
  async run(context, arguments_) {
    const [baselinePath, candidatePath, ...extra] = arguments_;
    if (baselinePath === undefined || candidatePath === undefined || extra.length > 0) {
      throw new Error("network-compare requires BASELINE.json CANDIDATE.json");
    }
    const [baselineText, candidateText] = await Promise.all([
      readFile(baselinePath, "utf8"),
      readFile(candidatePath, "utf8"),
    ]);
    const baseline = parseNetworkBenchmarkResult(JSON.parse(baselineText) as unknown);
    const candidate = parseNetworkBenchmarkResult(JSON.parse(candidateText) as unknown);
    const comparison = compareNetworkBenchmarks(baseline, candidate);
    const markdown = comparisonMarkdown(baseline, candidate, comparison);
    await context.artifacts.writeText(
      "network comparison",
      "comparison.md",
      markdown,
      "text/markdown",
    );
    return {
      summary: `Compared ${Object.keys(comparison).length} network scenarios.`,
      metrics: comparison,
    };
  },
});
