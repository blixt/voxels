import { expect } from "vite-plus/test";
import { defineScenario } from "./scenario.ts";
import { scenarioTest } from "./vitest.ts";

const inlineScenario = defineScenario({
  id: "inline-vite-test",
  kind: "validation",
  summary: "Runs a complete scenario inside the Vite+ test lifecycle.",
  uses: { metrics: true },
  async run(context) {
    await context.artifacts.writeJson("sample", "sample.json", { value: 42 });
    return { metrics: { answer: 42 } };
  },
});

scenarioTest(inlineScenario, {
  validate(manifest) {
    expect(manifest.status).toBe("passed");
    expect(manifest.result?.metrics?.answer).toBe(42);
    expect(manifest.artifacts.some((artifact) => artifact.path.endsWith("/sample.json"))).toBe(
      true,
    );
  },
});
