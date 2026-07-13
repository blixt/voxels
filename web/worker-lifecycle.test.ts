import { describe, expect, it } from "vite-plus/test";
import { disposeWorkerEngine, type DisposableWorkerEngine } from "./worker-lifecycle.ts";

describe("worker engine lifecycle", () => {
  it("waits for an in-flight boot and destroys the engine before closing", async () => {
    const calls: string[] = [];
    let finishBoot: ((engine: DisposableWorkerEngine) => void) | undefined;
    const booting = new Promise<DisposableWorkerEngine>((resolve) => {
      finishBoot = resolve;
    });
    const disposing = disposeWorkerEngine(null, booting, () => calls.push("close"));

    expect(calls).toEqual([]);
    finishBoot?.({
      destroy: async () => {
        calls.push("destroy");
      },
    });
    await disposing;

    expect(calls).toEqual(["destroy", "close"]);
  });

  it("still closes after a failed boot", async () => {
    const calls: string[] = [];
    await disposeWorkerEngine(null, Promise.reject(new Error("boot failed")), () =>
      calls.push("close"),
    );

    expect(calls).toEqual(["close"]);
  });
});
