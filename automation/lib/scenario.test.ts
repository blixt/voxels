import { describe, expect, it } from "vite-plus/test";
import { defineScenario, runScenario } from "./scenario.ts";

describe("automation scenario runner", () => {
  it("records capabilities, results, artifacts, and reverse cleanup", async () => {
    const cleanup: string[] = [];
    const scenario = defineScenario({
      id: "runner-contract",
      kind: "validation",
      summary: "Exercises the scenario lifecycle.",
      uses: { metrics: true },
      async run(context) {
        context.defer("first", () => {
          cleanup.push("first");
        });
        context.defer("second", () => {
          cleanup.push("second");
        });
        await context.artifacts.writeJson("evidence", "evidence.json", { ok: true });
        return { summary: "complete", metrics: { samples: 1 } };
      },
    });

    const manifest = await runScenario(scenario, [], {
      artifacts: { root: "target/automation-tests", runId: "runner-contract" },
      log: () => {},
    });
    expect(manifest.status).toBe("passed");
    expect(manifest.scenario.uses.metrics).toBe(true);
    expect(manifest.result?.metrics).toEqual({ samples: 1 });
    expect(manifest.artifacts.map((artifact) => artifact.label)).toContain("evidence");
    expect(cleanup).toEqual(["second", "first"]);
  });

  it("rejects invalid capability combinations", () => {
    expect(() =>
      defineScenario({
        id: "missing-viewport",
        kind: "capture",
        summary: "Invalid screenshot definition.",
        uses: { screenshots: true },
        async run() {},
      }),
    ).toThrow(/without declaring/u);
    expect(() =>
      defineScenario({
        id: "fake-native",
        kind: "capture",
        summary: "Invalid native renderer clone.",
        uses: { viewport: "native" },
        async run() {},
      }),
    ).toThrow(/parity/u);
  });
});
