import { spawn } from "node:child_process";
import { EventEmitter } from "node:events";
import { describe, expect, it, vi } from "vite-plus/test";
import { mirrorOwnedChildExit, spawnOwnedChild } from "./child-process-exit.ts";

class TestProcess extends EventEmitter {
  readonly pid = 42;
  exitCode: string | number | null | undefined;
  readonly mirroredSignals: NodeJS.Signals[] = [];

  kill(pid: number, signal: NodeJS.Signals): true {
    expect(pid).toBe(this.pid);
    this.mirroredSignals.push(signal);
    return true;
  }
}

function waitForOutput(child: ReturnType<typeof spawn>, pattern: RegExp): Promise<string> {
  return new Promise((resolve, reject) => {
    let output = "";
    child.once("error", reject);
    child.stdout?.on("data", (chunk: Buffer) => {
      output += chunk.toString();
      if (pattern.test(output)) resolve(output);
    });
  });
}

describe("owned child exit", () => {
  it("forwards wrapper termination and reaps a stubborn process tree", async () => {
    if (process.platform === "win32") return;
    const descendantSource = [
      'process.on("SIGTERM", () => {});',
      "setInterval(() => {}, 1_000);",
    ].join("");
    const childSource = [
      'const { spawn } = require("node:child_process");',
      `const descendant = spawn(process.execPath, ["-e", ${JSON.stringify(descendantSource)}],`,
      '  { stdio: "ignore" });',
      'process.on("SIGTERM", () => {});',
      "process.stdout.write(`ready:${descendant.pid}\\n`);",
      "setInterval(() => {}, 1_000);",
    ].join("");
    const child = spawnOwnedChild(process.execPath, ["-e", childSource], {
      stdio: ["ignore", "pipe", "inherit"],
    });
    const processTarget = new TestProcess();
    let descendantPid: number | undefined;
    try {
      const output = await waitForOutput(child, /^ready:\d+$/mu);
      descendantPid = Number(/^ready:(\d+)$/mu.exec(output)?.[1]);
      expect(Number.isSafeInteger(descendantPid)).toBe(true);
      mirrorOwnedChildExit(child, { processTarget, terminationTimeoutMs: 100 });

      processTarget.emit("SIGTERM");
      await new Promise<void>((resolve) => child.once("close", () => resolve()));
      await vi.waitFor(() => expect(processTarget.mirroredSignals).toEqual(["SIGKILL"]));

      expect(() => process.kill(child.pid as number, 0)).toThrow();
      expect(() => process.kill(descendantPid as number, 0)).toThrow();
    } finally {
      if (child.pid !== undefined) {
        try {
          process.kill(-child.pid, "SIGKILL");
        } catch {
          // The assertions above cover the expected already-reaped path.
        }
      }
      if (descendantPid !== undefined) {
        try {
          process.kill(descendantPid, "SIGKILL");
        } catch {
          // Cleanup remains best effort when a failed assertion interrupts the test.
        }
      }
    }
  });
});
