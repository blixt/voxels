import { describe, expect, it } from "vite-plus/test";
import { terminateAfterAcknowledgement } from "./hmr-lifecycle.ts";

describe("Vite worker reload lifecycle", () => {
  it("waits for the Rust cleanup acknowledgement before terminating", async () => {
    const calls: string[] = [];
    let acknowledge: (() => void) | undefined;
    const acknowledgement = new Promise<void>((resolve) => {
      acknowledge = () => {
        calls.push("acknowledge");
        resolve();
      };
    });
    const shutdown = terminateAfterAcknowledgement(acknowledgement, () => calls.push("terminate"));
    await Promise.resolve();
    expect(calls).toEqual([]);
    acknowledge?.();
    await shutdown;
    expect(calls).toEqual(["acknowledge", "terminate"]);
  });

  it("terminates a worker whose boot cannot acknowledge within the bound", async () => {
    const calls: string[] = [];
    await terminateAfterAcknowledgement(
      new Promise<void>(() => undefined),
      () => calls.push("terminate"),
      1,
    );
    expect(calls).toEqual(["terminate"]);
  });
});
