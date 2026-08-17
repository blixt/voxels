import { describe, expect, it } from "vite-plus/test";
import { startProcess } from "./process.ts";
import { defineScenario, runScenario } from "./scenario.ts";

describe("managed automation processes", () => {
  it("reaps an owned descendant after its launcher exits", async () => {
    if (process.platform === "win32") return;
    let descendantPid: number | undefined;
    const descendantSource = "setInterval(() => {}, 10_000)";
    const launcherSource = [
      'const { spawn } = require("node:child_process");',
      `const child = spawn(process.execPath, ["-e", ${JSON.stringify(descendantSource)}],`,
      '  { stdio: "ignore" });',
      "child.unref();",
      "process.stdout.write(String(child.pid));",
    ].join("");
    const scenario = defineScenario({
      id: "process-exited-launcher",
      kind: "validation",
      summary: "Exercises cleanup after a managed launcher exits.",
      uses: {},
      async run(context) {
        const managed = startProcess(context, process.execPath, ["-e", launcherSource], {
          label: "exited-launcher node fixture",
          stdio: ["ignore", "pipe", "inherit"],
        });
        descendantPid = await new Promise<number>((resolve, reject) => {
          managed.child.once("error", reject);
          managed.child.stdout?.once("data", (chunk: Buffer) => resolve(Number(chunk.toString())));
        });
        await managed.completed;
      },
    });

    try {
      await expect(
        runScenario(scenario, [], {
          artifacts: { root: "target/automation-tests", runId: "process-exited-launcher" },
          log: () => {},
        }),
      ).resolves.toMatchObject({ status: "passed" });
      expect(descendantPid).toBeDefined();
      expect(() => process.kill(descendantPid!, 0)).toThrow();
    } finally {
      if (descendantPid !== undefined) {
        try {
          process.kill(descendantPid, "SIGKILL");
        } catch {
          // Cleanup is best effort after the ESRCH assertion.
        }
      }
    }
  });

  it("shares an in-flight stop between concurrent callers", async () => {
    const scenario = defineScenario({
      id: "process-concurrent-stop",
      kind: "validation",
      summary: "Exercises concurrent process cleanup.",
      uses: {},
      async run(context) {
        const managed = startProcess(
          context,
          process.execPath,
          ["-e", "setInterval(() => {}, 10_000)"],
          { label: "concurrent-stop node fixture", stdio: "ignore" },
        );
        const first = managed.stop();
        expect(managed.stop()).toBe(first);
        await first;
      },
    });

    await expect(
      runScenario(scenario, [], {
        artifacts: { root: "target/automation-tests", runId: "process-concurrent-stop" },
        log: () => {},
      }),
    ).resolves.toMatchObject({ status: "passed" });
  });

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
