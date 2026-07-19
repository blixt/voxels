import { describe, expect, it } from "vite-plus/test";
import { startProcess } from "./process.ts";
import { defineScenario, runScenario } from "./scenario.ts";

describe("managed automation processes", () => {
  it("terminates an owned process when the scenario deadline expires", async () => {
    let childPid: number | undefined;
    const scenario = defineScenario({
      id: "process-timeout",
      kind: "validation",
      summary: "Exercises process cancellation.",
      uses: {},
      timeoutMs: 50,
      async run(context) {
        const managed = startProcess(
          context,
          process.execPath,
          ["-e", "setInterval(() => {}, 10_000)"],
          { label: "long-lived node fixture", stdio: "ignore" },
        );
        childPid = managed.child.pid;
        await managed.completed;
      },
    });

    await expect(
      runScenario(scenario, [], {
        artifacts: { root: "target/automation-tests", runId: "process-timeout" },
        log: () => {},
      }),
    ).rejects.toThrow("scenario timed out after 50ms");
    expect(childPid).toBeDefined();
    expect(() => process.kill(childPid!, 0)).toThrow();
  });
});
